//! SSH client: jump-host chains, proxies, every auth method, host key
//! verification, agent forwarding, remote port-forward dispatch and the
//! interactive shell loop.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use parking_lot::Mutex;
use russh::client::{self, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{ChannelMsg, Disconnect};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc::UnboundedReceiver;

use super::{AppEvent, Bridge, ConnCtx, PromptKind, PromptReply, Status};
use crate::model::{KnownHost, ProxyConfig, ProxyKind, ResolvedTarget, now_ts};
use crate::term::Input;

pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// Remote (server → client) forwards: (bind address, bind port) → local destination.
pub type RemoteForwardMap = Arc<Mutex<HashMap<(String, u32), (String, u16)>>>;

pub struct ClientHandler {
    host: String,
    port: u16,
    session: u64,
    bridge: Bridge,
    known: Vec<KnownHost>,
    remote_forwards: RemoteForwardMap,
    agent_forwarding: bool,
    strict_host_key_check: bool,
    pub banner: Arc<Mutex<Option<String>>>,
}

fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

impl client::Handler for ClientHandler {
    type Error = anyhow::Error;

    async fn auth_banner(
        &mut self,
        banner: &str,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        *self.banner.lock() = Some(banner.to_string());
        Ok(())
    }

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key: PublicKey = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(cert) => PublicKey::from(cert.public_key().clone()),
        };
        let fp = fingerprint(&key);
        let key_type = key.algorithm().as_str().to_string();
        let openssh = key.to_openssh().unwrap_or_default();
        let known: Vec<&KnownHost> = self
            .known
            .iter()
            .filter(|k| k.host.eq_ignore_ascii_case(&self.host) && k.port == self.port)
            .collect();
        if known.iter().any(|k| k.fingerprint == fp) {
            return Ok(true);
        }
        // Fall back to the user's OpenSSH known_hosts (read-only).
        if known.is_empty() {
            if let Ok(true) = russh::keys::check_known_hosts(&self.host, self.port, &key) {
                return Ok(true);
            }
        }
        if self.strict_host_key_check {
            return Err(anyhow!(
                "host key for {}:{} is {} and strict host key checking is on ({fp})",
                self.host,
                self.port,
                if known.is_empty() {
                    "not trusted yet"
                } else {
                    "different from the saved one"
                },
            ));
        }
        let new_entry = KnownHost {
            id: known
                .first()
                .map(|k| k.id)
                .unwrap_or_else(uuid::Uuid::new_v4),
            host: self.host.clone(),
            port: self.port,
            key_type: key_type.clone(),
            public_key: openssh,
            fingerprint: fp.clone(),
            added: now_ts(),
            updated: now_ts(),
        };
        let kind = if let Some(old) = known.first() {
            PromptKind::HostKeyChanged {
                host: self.host.clone(),
                port: self.port,
                key_type,
                fingerprint: fp,
                old_fingerprint: old.fingerprint.clone(),
            }
        } else {
            PromptKind::HostKeyUnknown {
                host: self.host.clone(),
                port: self.port,
                key_type,
                fingerprint: fp,
            }
        };
        let changed = !known.is_empty();
        match self.bridge.prompt(self.session, kind).await {
            PromptReply::Accept { save } => {
                if save {
                    self.bridge.send(if changed {
                        AppEvent::KnownHostReplace(new_entry)
                    } else {
                        AppEvent::KnownHostAdd(new_entry)
                    });
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let dest = {
            let map = self.remote_forwards.lock();
            map.get(&(connected_address.to_string(), connected_port))
                .or_else(|| {
                    map.iter()
                        .find(|((_, p), _)| *p == connected_port)
                        .map(|(_, v)| v)
                })
                .cloned()
        };
        let Some((dhost, dport)) = dest else {
            drop(reply);
            return Ok(());
        };
        reply.accept().await;
        tokio::spawn(async move {
            if let Ok(mut sock) = tokio::net::TcpStream::connect((dhost.as_str(), dport)).await {
                let mut stream = channel.into_stream();
                let _ = tokio::io::copy_bidirectional(&mut sock, &mut stream).await;
            }
        });
        Ok(())
    }

    async fn server_channel_open_agent_forward(
        &mut self,
        channel: russh::Channel<client::Msg>,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        if !self.agent_forwarding {
            drop(reply);
            return Ok(());
        }
        reply.accept().await;
        tokio::spawn(async move {
            let mut stream = channel.into_stream();
            if let Ok(mut agent) = connect_local_agent().await {
                let _ = tokio::io::copy_bidirectional(&mut agent, &mut stream).await;
            }
        });
        Ok(())
    }
}

#[cfg(unix)]
async fn connect_local_agent() -> Result<Box<dyn Io>> {
    let path = std::env::var("SSH_AUTH_SOCK").context("SSH_AUTH_SOCK is not set")?;
    Ok(Box::new(tokio::net::UnixStream::connect(path).await?))
}

#[cfg(windows)]
async fn connect_local_agent() -> Result<Box<dyn Io>> {
    let pipe = tokio::net::windows::named_pipe::ClientOptions::new()
        .open(r"\\.\pipe\openssh-ssh-agent")?;
    Ok(Box::new(pipe))
}

/// An authenticated SSH connection (plus any jump-host connections it rides on).
pub struct SshClient {
    pub handle: Handle<ClientHandler>,
    pub remote_forwards: RemoteForwardMap,
    pub label: String,
    _jumps: Vec<Handle<ClientHandler>>,
}

impl SshClient {
    pub async fn exec(&self, cmd: &str) -> Result<(String, u32)> {
        self.exec_limited(cmd, usize::MAX).await
    }

    /// Close the command channel if the caller's byte budget is exceeded.
    pub async fn exec_limited(&self, cmd: &str, limit: usize) -> Result<(String, u32)> {
        let mut ch = self.handle.channel_open_session().await?;
        ch.exec(true, cmd).await?;
        let mut out = Vec::new();
        let mut code = None;
        while let Some(msg) = ch.wait().await {
            match msg {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                    if data.len() > limit.saturating_sub(out.len()) {
                        ch.close().await?;
                        bail!(
                            "Remote response exceeded {} KiB. Narrow the request and retry.",
                            limit / 1024
                        );
                    }
                    out.extend_from_slice(&data);
                }
                ChannelMsg::Failure => bail!("The server refused the command"),
                ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        Ok((
            String::from_utf8_lossy(&out).to_string(),
            code.ok_or_else(|| {
                anyhow!("The remote command closed without reporting its exit status")
            })?,
        ))
    }

    pub async fn sftp(&self) -> Result<russh_sftp::client::SftpSession> {
        let ch = self.handle.channel_open_session().await?;
        ch.request_subsystem(true, "sftp").await?;
        Ok(russh_sftp::client::SftpSession::new(ch.into_stream()).await?)
    }

    pub async fn disconnect(&self) {
        let _ = self
            .handle
            .disconnect(Disconnect::ByApplication, "", "en")
            .await;
    }
}

fn client_config(keepalive: u32) -> Arc<client::Config> {
    Arc::new(client::Config {
        keepalive_interval: if keepalive > 0 {
            Some(Duration::from_secs(keepalive as u64))
        } else {
            None
        },
        keepalive_max: 4,
        nodelay: true,
        ..Default::default()
    })
}

async fn open_proxy_stream(proxy: &ProxyConfig, host: &str, port: u16) -> Result<Box<dyn Io>> {
    match proxy.kind {
        ProxyKind::Command => {
            let cmd = proxy
                .command
                .replace("%h", host)
                .replace("%p", &port.to_string());
            let mut parts = shell_words::split(&cmd).context("parsing proxy command")?;
            if parts.is_empty() {
                bail!("empty proxy command");
            }
            let prog = parts.remove(0);
            let mut child = tokio::process::Command::new(prog)
                .args(parts)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .context("starting proxy command")?;
            let stdin = child.stdin.take().unwrap();
            let stdout = child.stdout.take().unwrap();
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
            Ok(Box::new(tokio::io::join(stdout, stdin)))
        }
        ProxyKind::Socks5 => {
            let mut s = tokio::net::TcpStream::connect((proxy.host.as_str(), proxy.port))
                .await
                .context("connecting to SOCKS proxy")?;
            socks5_connect(&mut s, host, port, &proxy.username, &proxy.password).await?;
            Ok(Box::new(s))
        }
        ProxyKind::Http => {
            let mut s = tokio::net::TcpStream::connect((proxy.host.as_str(), proxy.port))
                .await
                .context("connecting to HTTP proxy")?;
            let mut req = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n");
            if !proxy.username.is_empty() {
                use base64::Engine;
                let cred = base64::engine::general_purpose::STANDARD
                    .encode(format!("{}:{}", proxy.username, proxy.password));
                req.push_str(&format!("Proxy-Authorization: Basic {cred}\r\n"));
            }
            req.push_str("\r\n");
            s.write_all(req.as_bytes()).await?;
            let mut buf = Vec::new();
            let mut byte = [0u8; 1];
            while !buf.ends_with(b"\r\n\r\n") {
                if s.read(&mut byte).await? == 0 || buf.len() > 8192 {
                    bail!("HTTP proxy closed connection");
                }
                buf.push(byte[0]);
            }
            let head = String::from_utf8_lossy(&buf);
            let status = head.split_whitespace().nth(1).unwrap_or("");
            if status != "200" {
                bail!(
                    "HTTP proxy refused CONNECT: {}",
                    head.lines().next().unwrap_or("")
                );
            }
            Ok(Box::new(s))
        }
    }
}

pub async fn socks5_connect<S: AsyncRead + AsyncWrite + Unpin>(
    s: &mut S,
    host: &str,
    port: u16,
    user: &str,
    pass: &str,
) -> Result<()> {
    let methods: &[u8] = if user.is_empty() {
        &[0x00]
    } else {
        &[0x00, 0x02]
    };
    let mut hello = vec![0x05, methods.len() as u8];
    hello.extend_from_slice(methods);
    s.write_all(&hello).await?;
    let mut resp = [0u8; 2];
    s.read_exact(&mut resp).await?;
    match resp[1] {
        0x00 => {}
        0x02 => {
            let mut a = vec![0x01, user.len() as u8];
            a.extend_from_slice(user.as_bytes());
            a.push(pass.len() as u8);
            a.extend_from_slice(pass.as_bytes());
            s.write_all(&a).await?;
            s.read_exact(&mut resp).await?;
            if resp[1] != 0 {
                bail!("SOCKS5 authentication failed");
            }
        }
        _ => bail!("SOCKS5 proxy: no acceptable auth method"),
    }
    let mut req = vec![0x05, 0x01, 0x00, 0x03, host.len() as u8];
    req.extend_from_slice(host.as_bytes());
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req).await?;
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await?;
    if head[1] != 0 {
        bail!("SOCKS5 connect failed (code {})", head[1]);
    }
    let skip = match head[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => {
            let mut l = [0u8; 1];
            s.read_exact(&mut l).await?;
            l[0] as usize
        }
        _ => 0,
    };
    let mut rest = vec![0u8; skip + 2];
    s.read_exact(&mut rest).await?;
    Ok(())
}

async fn handshake(
    stream: Box<dyn Io>,
    t: &ResolvedTarget,
    ctx: &ConnCtx,
    remote_forwards: RemoteForwardMap,
) -> Result<Handle<ClientHandler>> {
    let handler = ClientHandler {
        host: t.address.clone(),
        port: t.port,
        session: ctx.session,
        bridge: ctx.bridge.clone(),
        known: ctx.known_hosts.clone(),
        remote_forwards,
        agent_forwarding: t.agent_forwarding,
        strict_host_key_check: t.strict_host_key_check,
        banner: Arc::new(Mutex::new(None)),
    };
    let banner = handler.banner.clone();
    let mut handle = tokio::time::timeout(
        Duration::from_secs(t.connect_timeout_secs.max(1) as u64 + 10),
        client::connect_stream(client_config(t.keepalive_secs), stream, handler),
    )
    .await
    .map_err(|_| anyhow!("SSH handshake with {}:{} timed out", t.address, t.port))?
    .map_err(|e| anyhow!("{e:#}"))
    .with_context(|| format!("SSH handshake with {}:{}", t.address, t.port))?;
    authenticate(&mut handle, t, ctx).await?;
    if let Some(b) = banner.lock().take() {
        if !b.trim().is_empty() {
            ctx.feed(b.replace('\n', "\r\n").as_bytes());
        }
    }
    Ok(handle)
}

/// Connect (through jump hosts / proxy as configured) and authenticate.
pub async fn connect(t: &ResolvedTarget, ctx: &ConnCtx) -> Result<Arc<SshClient>> {
    let remote_forwards: RemoteForwardMap = Arc::new(Mutex::new(HashMap::new()));
    let mut jumps: Vec<Handle<ClientHandler>> = Vec::new();
    let hops: Vec<&ResolvedTarget> = t.chain.iter().chain(std::iter::once(t)).collect();
    for (i, hop) in hops.iter().enumerate() {
        let is_last = i == hops.len() - 1;
        ctx.set_status(Status::Connecting(format!(
            "Connecting to {}…",
            hop.address
        )));
        ctx.info(&format!(
            "→ Connecting to {}:{}{}",
            hop.address,
            hop.port,
            if i > 0 { " (via jump host)" } else { "" }
        ));
        let stream: Box<dyn Io> = if let Some(prev) = jumps.last() {
            let ch = prev
                .channel_open_direct_tcpip(hop.address.clone(), hop.port as u32, "127.0.0.1", 0)
                .await
                .with_context(|| {
                    format!("jump host could not reach {}:{}", hop.address, hop.port)
                })?;
            Box::new(ch.into_stream())
        } else if let Some(proxy) = t
            .proxy
            .as_ref()
            .filter(|p| !p.host.is_empty() || !p.command.is_empty())
        {
            open_proxy_stream(proxy, &hop.address, hop.port).await?
        } else {
            let addr = (hop.address.as_str(), hop.port);
            let sock = tokio::time::timeout(
                Duration::from_secs(hop.connect_timeout_secs.max(1) as u64),
                tokio::net::TcpStream::connect(addr),
            )
            .await
            .map_err(|_| anyhow!("connection to {}:{} timed out", hop.address, hop.port))?
            .with_context(|| format!("cannot reach {}:{}", hop.address, hop.port))?;
            sock.set_nodelay(true).ok();
            Box::new(sock)
        };
        let rf = if is_last {
            remote_forwards.clone()
        } else {
            Arc::new(Mutex::new(HashMap::new()))
        };
        let handle = handshake(stream, hop, ctx, rf).await?;
        jumps.push(handle);
    }
    let handle = jumps.pop().ok_or_else(|| anyhow!("no hops"))?;
    Ok(Arc::new(SshClient {
        handle,
        remote_forwards,
        label: t.label.clone(),
        _jumps: jumps,
    }))
}

async fn load_key(t: &ResolvedTarget, ctx: &ConnCtx) -> Result<Option<russh::keys::PrivateKey>> {
    let Some(k) = &t.key else { return Ok(None) };
    if k.private_key.trim().is_empty() {
        return Ok(None);
    }
    let pass = (!k.passphrase.is_empty()).then(|| k.passphrase.clone());
    match russh::keys::decode_secret_key(&k.private_key, pass.as_deref()) {
        Ok(key) => Ok(Some(key)),
        Err(_) if pass.is_none() => {
            let v = ctx
                .bridge
                .ask_fields(
                    ctx.session,
                    "Key passphrase",
                    &format!("Enter the passphrase for key “{}”", k.label),
                    &[("Passphrase", true)],
                )
                .await
                .ok_or_else(|| anyhow!("passphrase entry cancelled"))?;
            Ok(Some(
                russh::keys::decode_secret_key(&k.private_key, Some(&v[0]))
                    .context("wrong key passphrase")?,
            ))
        }
        Err(e) => Err(anyhow!("cannot read private key “{}”: {e}", k.label)),
    }
}

async fn authenticate(
    h: &mut Handle<ClientHandler>,
    t: &ResolvedTarget,
    ctx: &ConnCtx,
) -> Result<()> {
    let mut user = t.username.clone();
    if user.is_empty() {
        let v = ctx
            .bridge
            .ask_fields(
                ctx.session,
                "Username",
                &format!("Log in to {}", t.address),
                &[("Username", false)],
            )
            .await
            .ok_or_else(|| anyhow!("login cancelled"))?;
        user = v[0].clone();
    }
    ctx.set_status(Status::Connecting("Authenticating…".into()));

    // 1. Private key / certificate from the vault.
    if let Some(key) = load_key(t, ctx).await? {
        let key = Arc::new(key);
        let cert = t
            .key
            .as_ref()
            .map(|k| k.certificate.trim().to_string())
            .filter(|c| !c.is_empty())
            .and_then(|c| russh::keys::Certificate::from_openssh(&c).ok());
        let res = if let Some(cert) = cert {
            h.authenticate_openssh_cert(user.clone(), key, cert).await?
        } else {
            let hash = h.best_supported_rsa_hash().await?.flatten();
            h.authenticate_publickey(user.clone(), PrivateKeyWithHashAlg::new(key, hash))
                .await?
        };
        if res.success() {
            ctx.info("\u{E182} Authenticated with key");
            return Ok(());
        }
    }

    // 2. SSH agent (includes FIDO2 / hardware-backed keys held by the agent).
    if t.use_agent {
        if let Ok(ok) = try_agent(h, &user).await {
            if ok {
                ctx.info("\u{E182} Authenticated with SSH agent");
                return Ok(());
            }
        }
    }

    // 3. Stored password.
    if let Some(pw) = &t.password {
        if h.authenticate_password(user.clone(), pw.clone())
            .await?
            .success()
        {
            return Ok(());
        }
    }

    // 4. Keyboard-interactive (OTP / 2FA prompts).
    let mut used_stored_pw = false;
    if let Ok(mut resp) = h
        .authenticate_keyboard_interactive_start(user.clone(), None)
        .await
    {
        for _ in 0..8 {
            match resp {
                KeyboardInteractiveAuthResponse::Success => return Ok(()),
                KeyboardInteractiveAuthResponse::Failure { .. } => break,
                KeyboardInteractiveAuthResponse::InfoRequest {
                    name,
                    instructions,
                    prompts,
                } => {
                    let answers = if prompts.is_empty() {
                        vec![]
                    } else if prompts.len() == 1
                        && !prompts[0].echo
                        && t.password.is_some()
                        && !used_stored_pw
                    {
                        used_stored_pw = true;
                        vec![t.password.clone().unwrap_or_default()]
                    } else {
                        let fields: Vec<(&str, bool)> = prompts
                            .iter()
                            .map(|p| (p.prompt.as_str(), !p.echo))
                            .collect();
                        let title = if name.is_empty() {
                            "Authentication".to_string()
                        } else {
                            name.clone()
                        };
                        ctx.bridge
                            .ask_fields(ctx.session, &title, &instructions, &fields)
                            .await
                            .ok_or_else(|| anyhow!("login cancelled"))?
                    };
                    resp = h.authenticate_keyboard_interactive_respond(answers).await?;
                }
            }
        }
    }

    // 5. Ask for a password.
    for attempt in 0..3 {
        let msg = if attempt == 0 {
            format!("{}@{}", user, t.address)
        } else {
            "Permission denied, please try again.".to_string()
        };
        let v = ctx
            .bridge
            .ask_fields(ctx.session, "Password", &msg, &[("Password", true)])
            .await
            .ok_or_else(|| anyhow!("login cancelled"))?;
        if h.authenticate_password(user.clone(), v[0].clone())
            .await?
            .success()
        {
            return Ok(());
        }
    }
    bail!("authentication failed for {user}@{}", t.address)
}

#[cfg(unix)]
async fn try_agent(h: &mut Handle<ClientHandler>, user: &str) -> Result<bool> {
    let mut agent = russh::keys::agent::client::AgentClient::connect_env().await?;
    agent_auth(h, user, &mut agent).await
}

#[cfg(windows)]
async fn try_agent(h: &mut Handle<ClientHandler>, user: &str) -> Result<bool> {
    if let Ok(mut agent) =
        russh::keys::agent::client::AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent")
            .await
    {
        if agent_auth(h, user, &mut agent).await? {
            return Ok(true);
        }
    }
    let mut agent = russh::keys::agent::client::AgentClient::connect_pageant().await?;
    agent_auth(h, user, &mut agent).await
}

async fn agent_auth<S>(
    h: &mut Handle<ClientHandler>,
    user: &str,
    agent: &mut russh::keys::agent::client::AgentClient<S>,
) -> Result<bool>
where
    S: russh::keys::agent::client::AgentStream + Unpin + Send + 'static,
{
    let ids = agent.request_identities().await?;
    let hash = h.best_supported_rsa_hash().await?.flatten();
    for id in ids {
        let key = match id {
            russh::keys::agent::AgentIdentity::PublicKey { key, .. } => key,
            _ => continue,
        };
        if let Ok(res) = h
            .authenticate_publickey_with(user.to_string(), key, hash, agent)
            .await
        {
            if res.success() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn detect_os_from(out: &str) -> Option<String> {
    for line in out.lines() {
        if let Some(v) = line.strip_prefix("PRETTY_NAME=") {
            return Some(v.trim_matches('"').to_string());
        }
    }
    let first = out.lines().next()?.trim();
    (!first.is_empty()).then(|| first.to_string())
}

/// Authenticate with the normal trust/proxy/jump-host path, without opening a shell.
/// The deadline also bounds interactive prompts and a stalled OS detection command.
pub async fn test_connection(t: ResolvedTarget, ctx: ConnCtx) -> Result<Option<String>, String> {
    let deadline = Duration::from_secs(u64::from(t.connect_timeout_secs.clamp(5, 120)) + 30);
    tokio::time::timeout(deadline, async {
        let client = connect(&t, &ctx).await.map_err(|e| format!("{e:#}"))?;
        let os = tokio::time::timeout(
            Duration::from_secs(5),
            client.exec("cat /etc/os-release 2>/dev/null || uname -sr"),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .and_then(|(out, code)| (code == 0).then(|| detect_os_from(&out)).flatten());
        client.disconnect().await;
        Ok(os)
    })
    .await
    .unwrap_or_else(|_| {
        Err("Connection test timed out. Check the address, network, and login details.".into())
    })
}

/// SSH terminal session task.
pub async fn run_terminal(
    t: ResolvedTarget,
    ctx: ConnCtx,
    rx: UnboundedReceiver<Input>,
    slot: Arc<Mutex<Option<Arc<SshClient>>>>,
) -> Result<()> {
    let client = connect(&t, &ctx).await?;
    run_terminal_on_client(t, ctx, rx, slot, client, true).await
}

pub async fn run_terminal_on_client(
    t: ResolvedTarget,
    ctx: ConnCtx,
    mut rx: UnboundedReceiver<Input>,
    slot: Arc<Mutex<Option<Arc<SshClient>>>>,
    client: Arc<SshClient>,
    owns_connection: bool,
) -> Result<()> {
    *slot.lock() = Some(client.clone());

    let mut channel = client.handle.channel_open_session().await?;
    if t.agent_forwarding {
        let _ = channel.agent_forward(false).await;
    }
    for (k, v) in &t.env {
        let _ = channel.set_env(false, k.clone(), v.clone()).await;
    }
    // Wait for the first resize so the PTY starts with the real geometry.
    let (mut cols, mut rows) = (100u32, 30u32);
    let mut pending: Vec<Input> = Vec::new();
    if let Ok(Some(first)) = tokio::time::timeout(Duration::from_millis(400), rx.recv()).await {
        match first {
            Input::Resize {
                cols: c, rows: r, ..
            } => {
                cols = c as u32;
                rows = r as u32;
            }
            other => pending.push(other),
        }
    }
    channel
        .request_pty(false, &t.term_type, cols, rows, 0, 0, &[])
        .await?;
    if let Some(command) = &t.exec_command {
        channel.exec(true, command.clone()).await?;
    } else {
        channel.request_shell(true).await?;
    }
    ctx.set_status(Status::Connected);

    if ctx.detect_os {
        if let Some(host_id) = t.host_id {
            let c = client.clone();
            let bridge = ctx.bridge.clone();
            tokio::spawn(async move {
                if let Ok((out, _)) = c.exec("cat /etc/os-release 2>/dev/null || uname -sr").await {
                    if let Some(os) = detect_os_from(&out) {
                        bridge.send(AppEvent::HostOs { host_id, os });
                    }
                }
            });
        }
    }

    if let Some(script) = t
        .startup_script
        .clone()
        .filter(|s| t.exec_command.is_none() && !s.trim().is_empty())
    {
        let ch_w = channel.make_writer();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(600)).await;
            let mut w = ch_w;
            let mut s = script.replace("\r\n", "\n").replace('\n', "\r");
            if !s.ends_with('\r') {
                s.push('\r');
            }
            let _ = w.write_all(s.as_bytes()).await;
        });
    }

    for p in pending {
        if let Input::Data(d) = p {
            channel.data_bytes(d).await?;
        }
    }

    let reason = loop {
        tokio::select! {
            input = rx.recv() => match input {
                Some(Input::Data(d)) => { channel.data_bytes(d).await?; }
                Some(Input::Resize { cols, rows, .. }) => {
                    let _ = channel.window_change(cols as u32, rows as u32, 0, 0).await;
                }
                Some(Input::Close) | None => {
                    let _ = channel.eof().await;
                    break "Session closed".to_string();
                }
            },
            msg = channel.wait() => match msg {
                Some(ChannelMsg::Data { data }) => ctx.feed(&data),
                Some(ChannelMsg::ExtendedData { data, .. }) => ctx.feed(&data),
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    break format!("Process exited with code {exit_status}");
                }
                Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                    break format!("Process killed by signal {signal_name:?}");
                }
                Some(ChannelMsg::Failure) => break "The server refused the terminal request".to_string(),
                Some(ChannelMsg::Close) | None => break "Connection closed".to_string(),
                _ => {}
            }
        }
    };
    *slot.lock() = None;
    if let Err(error) = channel.close().await {
        log::debug!("Closing terminal channel: {error}");
    }
    if owns_connection {
        client.disconnect().await;
    }
    ctx.info(&format!("\r\n● {reason}"));
    ctx.set_status(Status::Closed(reason));
    Ok(())
}

/// Connect a standalone SSH client (used by SFTP and port forwarding when no
/// terminal for the host is open).
pub async fn connect_headless(t: ResolvedTarget, ctx: ConnCtx) -> Result<Arc<SshClient>> {
    connect(&t, &ctx).await
}

/// Append a public key to `~/.ssh/authorized_keys` on the remote host.
pub async fn install_public_key(client: &SshClient, public_key: &str) -> Result<()> {
    let key = public_key.trim().replace('\'', "");
    let cmd = format!(
        "umask 077; mkdir -p ~/.ssh && touch ~/.ssh/authorized_keys && (grep -qxF '{key}' ~/.ssh/authorized_keys || echo '{key}' >> ~/.ssh/authorized_keys) && chmod 600 ~/.ssh/authorized_keys && echo FERRUM_OK"
    );
    let (out, _) = client.exec(&cmd).await?;
    if out.contains("FERRUM_OK") {
        Ok(())
    } else {
        bail!("remote command failed: {}", out.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_detection_prefers_pretty_name() {
        let out = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04 LTS\"\n";
        assert_eq!(detect_os_from(out).as_deref(), Some("Ubuntu 24.04 LTS"));
        assert_eq!(
            detect_os_from("FreeBSD 14.1-RELEASE\n").as_deref(),
            Some("FreeBSD 14.1-RELEASE")
        );
    }

    #[tokio::test]
    async fn socks5_handshake_against_fake_server() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let srv = tokio::spawn(async move {
            let mut buf = [0u8; 3];
            server.read_exact(&mut buf).await.unwrap();
            assert_eq!(buf, [5, 1, 0]);
            server.write_all(&[5, 0]).await.unwrap();
            let mut head = [0u8; 5];
            server.read_exact(&mut head).await.unwrap();
            let mut rest = vec![0u8; head[4] as usize + 2];
            server.read_exact(&mut rest).await.unwrap();
            assert_eq!(&rest[..head[4] as usize], b"example.com");
            server
                .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                .await
                .unwrap();
        });
        socks5_connect(&mut client, "example.com", 22, "", "")
            .await
            .unwrap();
        srv.await.unwrap();
    }
}

#[cfg(test)]
mod probe_tests;
