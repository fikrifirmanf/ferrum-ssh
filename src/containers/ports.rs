//! Published container bindings and session-scoped loopback tunnels.
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use parking_lot::Mutex;
use serde::Deserialize;

use super::{Item, LIST_TIMEOUT, Runtime, SshClient, shell_quote, sudo_prefix};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PublishedPort {
    pub container_port: u16,
    pub protocol: String,
    pub host: IpAddr,
    pub host_port: u16,
}
impl PublishedPort {
    pub fn key(&self, item_key: &str) -> String {
        format!(
            "{item_key}/{}/{}/{}",
            self.container_port,
            self.protocol,
            SocketAddr::new(self.host, self.host_port)
        )
    }
    pub fn destination(&self) -> IpAddr {
        match self.host {
            IpAddr::V4(ip) if ip.is_unspecified() => Ipv4Addr::LOCALHOST.into(),
            IpAddr::V6(ip) if ip.is_unspecified() => Ipv6Addr::LOCALHOST.into(),
            ip => ip,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Binding {
    host_ip: String,
    host_port: String,
}

pub fn parse(text: &str) -> Result<Vec<PublishedPort>> {
    let bindings: Option<BTreeMap<String, Option<Vec<Binding>>>> = serde_json::from_str(text)?;
    let mut ports = Vec::new();
    for (container, bindings) in bindings.unwrap_or_default() {
        let (port, protocol) = container
            .split_once('/')
            .ok_or_else(|| anyhow!("Invalid published port response"))?;
        let port: u16 = port.parse()?;
        if port == 0 || !matches!(protocol, "tcp" | "udp" | "sctp") {
            bail!("Invalid container port or protocol");
        }
        for binding in bindings.unwrap_or_default() {
            // Routed IPv6 modes have a binding with an empty host port, not a published listener.
            if binding.host_port.is_empty() {
                continue;
            }
            let host_port: u16 = binding.host_port.parse()?;
            if host_port == 0 {
                bail!("Invalid published host port");
            }
            let host = if binding.host_ip.is_empty() {
                IpAddr::V4(Ipv4Addr::UNSPECIFIED)
            } else {
                binding.host_ip.parse()?
            };
            ports.push(PublishedPort {
                container_port: port,
                protocol: protocol.into(),
                host,
                host_port,
            });
            if ports.len() > 1024 {
                bail!(
                    "More than 1,024 published bindings. Select a smaller container configuration."
                );
            }
        }
    }
    ports.sort();
    ports.dedup();
    Ok(ports)
}

pub async fn fetch(client: &SshClient, item: &Item, sudo: bool) -> Result<String> {
    item.valid()
        .ok_or_else(|| anyhow!("Invalid container identity"))?;
    if item.runtime == Runtime::Kubernetes {
        bail!("Published-port tunnels support Docker and Podman containers");
    }
    let command = format!(
        "{}{} inspect --format {} {}",
        sudo_prefix(sudo),
        item.runtime.cli(),
        shell_quote("{{json .NetworkSettings.Ports}}"),
        item.scope
    );
    let (text, code) =
        tokio::time::timeout(LIST_TIMEOUT, client.exec_limited(&command, 256 * 1024))
            .await
            .map_err(|_| anyhow!("Published-port request timed out"))??;
    if code != 0 {
        bail!("{}", super::last_line(&text));
    }
    parse(&text)?;
    Ok(text)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OpenAs {
    #[default]
    Http,
    Https,
    Tcp,
}
impl OpenAs {
    pub fn url(self, address: SocketAddr) -> Option<String> {
        match self {
            Self::Http => Some(format!("http://{address}/")),
            Self::Https => Some(format!("https://{address}/")),
            Self::Tcp => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TunnelStatus {
    Starting,
    Ready(SocketAddr),
    Failed(String),
    Stopped,
}

pub struct Tunnel {
    pub owner: String,
    pub port: PublishedPort,
    pub mode: OpenAs,
    pub browser_error: Option<String>,
    pub connection: super::services::Connection,
    pub credentials_pending: bool,
    credential_result: Arc<Mutex<Option<Result<BTreeMap<String, String>, String>>>>,
    credential_worker: Option<tokio::task::AbortHandle>,
    browser_pending: bool,
    stopped: bool,
    state: Arc<Mutex<TunnelStatus>>,
    connection_error: Arc<Mutex<Option<String>>>,
    worker: tokio::task::AbortHandle,
}
impl Drop for Tunnel {
    fn drop(&mut self) {
        self.worker.abort();
        if let Some(task) = &self.credential_worker {
            task.abort();
        }
    }
}
impl Tunnel {
    pub fn start(
        rt: &tokio::runtime::Handle,
        client: Arc<SshClient>,
        item: Item,
        port: PublishedPort,
        mode: OpenAs,
        sudo: bool,
        ctx: egui::Context,
    ) -> Self {
        use super::services::{Connection, Service};
        let service = match (
            super::services::detect(&item.detail, port.container_port),
            mode,
        ) {
            (Service::Http, OpenAs::Https) => Service::Https,
            (Service::Https, OpenAs::Http) => Service::Http,
            (service, _) => service,
        };
        let mut connection = Connection::new(service, port.container_port);
        if matches!(service, Service::S3 | Service::ClickHouseHttp) && mode == OpenAs::Https {
            connection.tls = true;
        }
        let state = Arc::new(Mutex::new(TunnelStatus::Starting));
        let errors = Arc::new(Mutex::new(None));
        let (task_state, task_errors) = (state.clone(), errors.clone());
        let (owner, task_port) = (item.key(), port.clone());
        let task = rt.spawn(async move {
            let result: Result<()> = async {
                if task_port.protocol != "tcp" {
                    bail!("SSH tunnels support published TCP ports only");
                }
                let current = parse(&fetch(&client, &item, sudo).await?)?;
                if !current.contains(&task_port) {
                    bail!("Published ports changed. Refresh ports and try again.");
                }
                crate::forward::serve_loopback(
                    client,
                    task_port.destination(),
                    task_port.host_port,
                    |address| {
                        *task_state.lock() = TunnelStatus::Ready(address);
                        ctx.request_repaint();
                    },
                    |error| {
                        *task_errors.lock() = Some(error);
                        ctx.request_repaint();
                    },
                )
                .await
            }
            .await;
            *task_state.lock() = match result {
                Ok(()) => TunnelStatus::Stopped,
                Err(error) => TunnelStatus::Failed(error.to_string()),
            };
            ctx.request_repaint();
        });
        Self {
            owner,
            port,
            mode,
            browser_error: None,
            connection,
            credentials_pending: false,
            credential_result: Arc::new(Mutex::new(None)),
            credential_worker: None,
            browser_pending: mode != OpenAs::Tcp,
            stopped: false,
            state,
            connection_error: errors,
            worker: task.abort_handle(),
        }
    }
    /// Synthetic UI QA only: no sockets, server access or browser launch.
    #[cfg(debug_assertions)]
    pub fn qa_demo(rt: &tokio::runtime::Handle, owner: String, port: PublishedPort) -> Self {
        Self {
            owner,
            port,
            mode: OpenAs::Tcp,
            browser_error: None,
            connection: super::services::Connection::new(super::services::Service::Postgres, 5432),
            credentials_pending: false,
            credential_result: Arc::new(Mutex::new(None)),
            credential_worker: None,
            browser_pending: false,
            stopped: false,
            state: Arc::new(Mutex::new(TunnelStatus::Ready(SocketAddr::from((
                [127, 0, 0, 1],
                43210,
            ))))),
            connection_error: Arc::new(Mutex::new(None)),
            worker: rt.spawn(std::future::pending::<()>()).abort_handle(),
        }
    }
    pub fn read_credentials(
        &mut self,
        rt: &tokio::runtime::Handle,
        client: Arc<SshClient>,
        item: Item,
        sudo: bool,
        ctx: egui::Context,
    ) {
        if self.credentials_pending
            || item.key() != self.owner
            || !matches!(self.status(), TunnelStatus::Ready(_))
        {
            return;
        }
        let result = self.credential_result.clone();
        let service = self.connection.service;
        self.credentials_pending = true;
        self.credential_worker = Some(
            rt.spawn(async move {
                *result.lock() = Some(
                    super::services::credentials(&client, &item, service, sudo)
                        .await
                        .map_err(|e| e.to_string()),
                );
                ctx.request_repaint();
            })
            .abort_handle(),
        );
    }
    pub fn poll_credentials(&mut self) {
        let result = self.credential_result.lock().take();
        if let Some(result) = result {
            self.credentials_pending = false;
            match result {
                Ok(env) => self.connection.apply_env(env),
                Err(error) => self.connection.note = error,
            }
        }
    }
    pub fn status(&self) -> TunnelStatus {
        if self.stopped {
            return TunnelStatus::Stopped;
        }
        let state = self.state.lock().clone();
        if self.worker.is_finished()
            && matches!(state, TunnelStatus::Starting | TunnelStatus::Ready(_))
        {
            TunnelStatus::Stopped
        } else {
            state
        }
    }
    pub fn connection_error(&self) -> Option<String> {
        self.connection_error.lock().clone()
    }
    pub fn stop(&mut self) {
        self.stopped = true;
        self.worker.abort();
        *self.state.lock() = TunnelStatus::Stopped;
        self.browser_pending = false;
        if let Some(task) = self.credential_worker.take() {
            task.abort();
        }
        self.credentials_pending = false;
        *self.credential_result.lock() = None;
        self.connection.password.clear();
    }
    pub fn take_browser_url(&mut self) -> Option<String> {
        if !self.browser_pending {
            return None;
        }
        if let TunnelStatus::Ready(address) = self.status() {
            self.browser_pending = false;
            return self.mode.url(address);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_ipv4_ipv6_and_only_published_bindings() {
        let ports = parse(r#"{"80/tcp":[{"HostIp":"0.0.0.0","HostPort":"8080"},{"HostIp":"::","HostPort":"8080"},{"HostIp":"::","HostPort":""}],"53/udp":[{"HostIp":"127.0.0.1","HostPort":"5353"}],"443/tcp":null}"#).unwrap();
        assert_eq!(ports.len(), 3);
        let tcp: Vec<_> = ports.iter().filter(|p| p.protocol == "tcp").collect();
        assert_eq!(tcp[0].destination(), IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(tcp[1].destination(), IpAddr::V6(Ipv6Addr::LOCALHOST));
        assert_eq!(tcp[0].container_port, 80);
        assert_eq!(tcp[0].host_port, 8080);
        assert!(parse("null").unwrap().is_empty());
        assert!(parse(r#"{"80/tcp":null}"#).unwrap().is_empty());
    }
    #[test]
    fn rejects_invalid_ports_and_hostnames_without_turning_them_into_urls() {
        for text in [
            r#"{"80/tcp":[{"HostIp":"evil.example","HostPort":"8080"}]}"#,
            r#"{"80/tcp":[{"HostIp":"127.0.0.1","HostPort":"0"}]}"#,
            r#"{"80/tcp":[{"HostIp":"127.0.0.1","HostPort":"65536"}]}"#,
            r#"{"0/tcp":[]}"#,
            r#"{"80/unknown":[]}"#,
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
        let address: SocketAddr = "127.0.0.1:43210".parse().unwrap();
        assert_eq!(
            OpenAs::Http.url(address).as_deref(),
            Some("http://127.0.0.1:43210/")
        );
        assert_eq!(
            OpenAs::Https.url(address).as_deref(),
            Some("https://127.0.0.1:43210/")
        );
        assert_eq!(OpenAs::Tcp.url(address), None);
    }
}
