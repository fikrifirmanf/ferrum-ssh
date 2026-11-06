//! Port forwarding: local (-L), remote (-R) and dynamic SOCKS4/5 (-D).

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::conn::ssh::SshClient;
use crate::conn::{AppEvent, Bridge, ConnCtx};
use crate::model::{ForwardKind, PortForward, ResolvedTarget};

#[derive(Default)]
pub struct ForwardManager {
    running: HashMap<Uuid, tokio::task::AbortHandle>,
}

impl ForwardManager {
    pub fn is_running(&self, id: Uuid) -> bool {
        self.running
            .get(&id)
            .map(|h| !h.is_finished())
            .unwrap_or(false)
    }

    pub fn running_count(&self) -> usize {
        self.running.values().filter(|h| !h.is_finished()).count()
    }

    pub fn stop(&mut self, id: Uuid, bridge: &Bridge) {
        if let Some(h) = self.running.remove(&id) {
            h.abort();
            bridge.send(AppEvent::ForwardState {
                rule: id,
                running: false,
                error: None,
            });
        }
    }

    pub fn start(
        &mut self,
        rt: &tokio::runtime::Handle,
        rule: PortForward,
        target: ResolvedTarget,
        ctx: ConnCtx,
    ) {
        if self.is_running(rule.id) {
            return;
        }
        let bridge = ctx.bridge.clone();
        let id = rule.id;
        let task = rt.spawn(async move {
            let res = async {
                let client = crate::conn::ssh::connect_headless(target, ctx).await?;
                bridge.send(AppEvent::ForwardState {
                    rule: id,
                    running: true,
                    error: None,
                });
                run_rule(&rule, client).await
            }
            .await;
            let error = res.err().map(|e| format!("{e:#}"));
            if let Some(e) = &error {
                bridge.toast(
                    format!("Port forwarding “{}” stopped: {e}", rule.label),
                    true,
                );
            }
            bridge.send(AppEvent::ForwardState {
                rule: id,
                running: false,
                error,
            });
        });
        self.running.insert(id, task.abort_handle());
    }
}

/// Relay through the same direct-tcpip path used by saved local forwarding rules.
pub async fn relay_local(
    mut socket: tokio::net::TcpStream,
    peer: std::net::SocketAddr,
    client: Arc<SshClient>,
    host: String,
    port: u16,
) -> Result<()> {
    let channel = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        client.handle.channel_open_direct_tcpip(
            host,
            port as u32,
            peer.ip().to_string(),
            peer.port() as u32,
        ),
    )
    .await
    .context("Opening SSH port channel timed out")??;
    tokio::io::copy_bidirectional(&mut socket, &mut channel.into_stream()).await?;
    Ok(())
}

/// A temporary loopback-only listener. Dropping this future closes all child connections.
pub async fn serve_loopback(
    client: Arc<SshClient>,
    host: std::net::IpAddr,
    port: u16,
    ready: impl Fn(std::net::SocketAddr),
    failed_connection: impl Fn(String),
) -> Result<()> {
    if port == 0 {
        bail!("Destination port must be greater than zero");
    }
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let address = listener.local_addr()?;
    // Do not announce readiness or launch a browser until SSH accepts the destination.
    let probe = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        client.handle.channel_open_direct_tcpip(
            host.to_string(),
            port as u32,
            "127.0.0.1",
            address.port() as u32,
        ),
    )
    .await
    .context("Checking the published port timed out")??;
    probe.close().await?;
    ready(address);
    let mut connections = tokio::task::JoinSet::new();
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if client.handle.is_closed() { bail!("SSH connection closed. Reconnect and open the port again."); }
            }
            accepted = listener.accept(), if connections.len() < 64 => {
                let (socket, peer) = accepted?;
                let client = client.clone();
                let host = host.to_string();
                connections.spawn(async move { relay_local(socket, peer, client, host, port).await });
            }
            result = connections.join_next(), if !connections.is_empty() => {
                match result {
                    Some(Ok(Err(error))) => failed_connection(format!("Could not forward a connection: {error}")),
                    Some(Err(error)) => failed_connection(format!("Port connection task failed: {error}")),
                    _ => {}
                }
            }
        }
    }
}

pub async fn run_rule(rule: &PortForward, client: Arc<SshClient>) -> Result<()> {
    match rule.kind {
        ForwardKind::Local => {
            let listener = TcpListener::bind((rule.bind_address.as_str(), rule.bind_port))
                .await
                .with_context(|| format!("binding {}:{}", rule.bind_address, rule.bind_port))?;
            loop {
                let (sock, peer) = listener.accept().await?;
                let c = client.clone();
                let (dh, dp) = (rule.dest_host.clone(), rule.dest_port);
                tokio::spawn(async move {
                    if let Err(error) = relay_local(sock, peer, c, dh, dp).await {
                        log::debug!("Local forwarding connection: {error}");
                    }
                });
            }
        }
        ForwardKind::Remote => {
            client.remote_forwards.lock().insert(
                (rule.bind_address.clone(), rule.bind_port as u32),
                (rule.dest_host.clone(), rule.dest_port),
            );
            client
                .handle
                .tcpip_forward(rule.bind_address.clone(), rule.bind_port as u32)
                .await
                .context("server refused remote port forwarding")?;
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                if client.handle.is_closed() {
                    bail!("SSH connection closed");
                }
            }
        }
        ForwardKind::Dynamic => {
            let listener = TcpListener::bind((rule.bind_address.as_str(), rule.bind_port))
                .await
                .with_context(|| format!("binding {}:{}", rule.bind_address, rule.bind_port))?;
            loop {
                let (mut sock, peer) = listener.accept().await?;
                let c = client.clone();
                tokio::spawn(async move {
                    let Ok((host, port, version)) = socks_server_handshake(&mut sock).await else {
                        return;
                    };
                    match c
                        .handle
                        .channel_open_direct_tcpip(
                            host,
                            port as u32,
                            peer.ip().to_string(),
                            peer.port() as u32,
                        )
                        .await
                    {
                        Ok(ch) => {
                            if socks_reply(&mut sock, version, true).await.is_ok() {
                                let mut stream = ch.into_stream();
                                let _ = tokio::io::copy_bidirectional(&mut sock, &mut stream).await;
                            }
                        }
                        Err(_) => {
                            let _ = socks_reply(&mut sock, version, false).await;
                        }
                    }
                });
            }
        }
    }
}

/// Minimal SOCKS4/4a/5 server handshake (CONNECT only). Returns destination and protocol version.
pub async fn socks_server_handshake<S: AsyncRead + AsyncWrite + Unpin>(
    s: &mut S,
) -> Result<(String, u16, u8)> {
    let ver = s.read_u8().await?;
    match ver {
        5 => {
            let n = s.read_u8().await? as usize;
            let mut methods = vec![0u8; n];
            s.read_exact(&mut methods).await?;
            s.write_all(&[5, 0]).await?;
            let mut head = [0u8; 4];
            s.read_exact(&mut head).await?;
            if head[1] != 1 {
                s.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
                bail!("unsupported SOCKS command");
            }
            let host = match head[3] {
                1 => {
                    let mut a = [0u8; 4];
                    s.read_exact(&mut a).await?;
                    std::net::Ipv4Addr::from(a).to_string()
                }
                4 => {
                    let mut a = [0u8; 16];
                    s.read_exact(&mut a).await?;
                    std::net::Ipv6Addr::from(a).to_string()
                }
                3 => {
                    let l = s.read_u8().await? as usize;
                    let mut d = vec![0u8; l];
                    s.read_exact(&mut d).await?;
                    String::from_utf8_lossy(&d).to_string()
                }
                _ => bail!("bad SOCKS address type"),
            };
            let port = s.read_u16().await?;
            Ok((host, port, 5))
        }
        4 => {
            let cmd = s.read_u8().await?;
            let port = s.read_u16().await?;
            let mut ip = [0u8; 4];
            s.read_exact(&mut ip).await?;
            // user id (null terminated)
            loop {
                if s.read_u8().await? == 0 {
                    break;
                }
            }
            if cmd != 1 {
                bail!("unsupported SOCKS4 command");
            }
            let host = if ip[0] == 0 && ip[1] == 0 && ip[2] == 0 && ip[3] != 0 {
                // SOCKS4a: domain follows
                let mut d = vec![];
                loop {
                    let b = s.read_u8().await?;
                    if b == 0 {
                        break;
                    }
                    d.push(b);
                }
                String::from_utf8_lossy(&d).to_string()
            } else {
                std::net::Ipv4Addr::from(ip).to_string()
            };
            Ok((host, port, 4))
        }
        _ => bail!("not a SOCKS request"),
    }
}

async fn socks_reply<S: AsyncWrite + Unpin>(s: &mut S, version: u8, ok: bool) -> Result<()> {
    if version == 5 {
        s.write_all(&[5, if ok { 0 } else { 5 }, 0, 1, 0, 0, 0, 0, 0, 0])
            .await?;
    } else {
        s.write_all(&[0, if ok { 0x5a } else { 0x5b }, 0, 0, 0, 0, 0, 0])
            .await?;
    }
    Ok(())
}

/// Parse an OpenSSH-style forward spec: `[bind_address:]port:host:hostport`.
pub fn parse_forward_spec(spec: &str, kind: ForwardKind) -> Option<PortForward> {
    let parts: Vec<&str> = spec.split(':').collect();
    let mut rule = PortForward {
        kind,
        ..Default::default()
    };
    match (kind, parts.as_slice()) {
        (ForwardKind::Dynamic, [p]) => rule.bind_port = p.parse().ok()?,
        (ForwardKind::Dynamic, [b, p]) => {
            rule.bind_address = b.to_string();
            rule.bind_port = p.parse().ok()?;
        }
        (_, [p, h, hp]) => {
            rule.bind_port = p.parse().ok()?;
            rule.dest_host = h.to_string();
            rule.dest_port = hp.parse().ok()?;
        }
        (_, [b, p, h, hp]) => {
            rule.bind_address = b.to_string();
            rule.bind_port = p.parse().ok()?;
            rule.dest_host = h.to_string();
            rule.dest_port = hp.parse().ok()?;
        }
        _ => return None,
    }
    Some(rule)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openssh_forward_specs() {
        let r = parse_forward_spec("8080:localhost:80", ForwardKind::Local).unwrap();
        assert_eq!(
            (r.bind_port, r.dest_host.as_str(), r.dest_port),
            (8080, "localhost", 80)
        );
        let r = parse_forward_spec("0.0.0.0:1080", ForwardKind::Dynamic).unwrap();
        assert_eq!((r.bind_address.as_str(), r.bind_port), ("0.0.0.0", 1080));
        assert!(parse_forward_spec("nonsense", ForwardKind::Local).is_none());
    }

    #[tokio::test]
    async fn socks5_server_reads_domain_target() {
        let (mut client, mut server) = tokio::io::duplex(256);
        let t = tokio::spawn(async move { socks_server_handshake(&mut server).await.unwrap() });
        client.write_all(&[5, 1, 0]).await.unwrap();
        let mut r = [0u8; 2];
        client.read_exact(&mut r).await.unwrap();
        assert_eq!(r, [5, 0]);
        let mut req = vec![5, 1, 0, 3, 11];
        req.extend_from_slice(b"example.org");
        req.extend_from_slice(&443u16.to_be_bytes());
        client.write_all(&req).await.unwrap();
        assert_eq!(t.await.unwrap(), ("example.org".to_string(), 443, 5));
    }
}
