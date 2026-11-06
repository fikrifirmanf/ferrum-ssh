//! Exercise the real SSH transport against an in-process server, without a Docker daemon.
use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::conn::{self, AppEvent, Bridge, ConnCtxTemplate, PromptReply, Status};
use crate::model::{Host, KnownHost, Vault};
use crate::term::Input;
use parking_lot::Mutex;
use russh::server::{self, Server as _};

#[derive(Clone, Default)]
struct Server {
    commands: Arc<Mutex<Vec<String>>>,
    port_bindings: Arc<Mutex<String>>,
    deny_forward: Arc<std::sync::atomic::AtomicBool>,
    forwards: Arc<Mutex<Vec<(String, u32)>>>,
    terminal_started: Arc<tokio::sync::Notify>,
}
impl server::Server for Server {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}
impl server::Handler for Server {
    type Error = russh::Error;
    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        Ok(if user == "fixture" && password == "fixture" {
            server::Auth::Accept
        } else {
            server::Auth::reject()
        })
    }
    async fn channel_open_session(
        &mut self,
        _: russh::Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: russh::Channel<server::Msg>,
        host: &str,
        port: u32,
        _: &str,
        _: u32,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.forwards.lock().push((host.to_string(), port));
        if self.deny_forward.load(std::sync::atomic::Ordering::Relaxed)
            || !host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        {
            reply
                .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        }
        match tokio::net::TcpStream::connect((host, port as u16)).await {
            Ok(mut socket) => {
                reply.accept().await;
                tokio::spawn(async move {
                    let _ = tokio::io::copy_bidirectional(&mut socket, &mut channel.into_stream())
                        .await;
                });
            }
            Err(_) => reply.reject(russh::ChannelOpenFailure::ConnectFailed).await,
        }
        Ok(())
    }
    async fn pty_request(
        &mut self,
        channel: russh::ChannelId,
        _: &str,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
        _: &[(russh::Pty, u32)],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }
    async fn exec_request(
        &mut self,
        channel: russh::ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).into_owned();
        self.commands.lock().push(command.clone());
        session.channel_success(channel)?;
        if command == "docker exec -it abcdef123456 sh" {
            session.data(channel, b"container$ ".to_vec())?;
            self.terminal_started.notify_one();
            return Ok(());
        }
        if command == "late-status" {
            session.data(channel, b"failed".to_vec())?;
            session.eof(channel)?;
            session.exit_status_request(channel, 9)?;
            session.close(channel)?;
            return Ok(());
        }
        let (text, code) = if command.contains("--format '{{json .NetworkSettings.Ports}}'") {
            let bindings = self.port_bindings.lock().clone();
            (
                if bindings.is_empty() {
                    "null".into()
                } else {
                    bindings
                },
                0,
            )
        } else if command.contains("range .Config.Env") {
            ("\"POSTGRES_USER=fixture\"\n\"POSTGRES_PASSWORD=synthetic=p@ss\"\n\"POSTGRES_DB=events\"\n".into(), 0)
        } else if command.contains(" stats --no-stream ") {
            (r#"{"ID":"abcdef123456","CPUPerc":"125.5%","MemPerc":"12.5%","MemUsage":"32MiB / 256MiB","NetIO":"1kB / 2kB","BlockIO":"0B / 1kB","PIDs":"8"}"#.to_owned(), 0)
        } else if command.contains(" inspect --format ") {
            (r#"{"id":"abcdef123456","image":"nginx:1.27","state":"running","mounts":[],"networks":{},"ports":{}}"#.to_owned(), 0)
        } else if command.contains(" logs --tail ") {
            ("\u{1b}[31mready\u{1b}[0m\n".into(), 0)
        } else if command == "oversize" {
            ("x".repeat(2048), 0)
        } else if command == "failure" {
            ("permission denied".into(), 7)
        } else {
            ("owner alive".into(), 0)
        };
        session.data(channel, text.into_bytes())?;
        session.exit_status_request(channel, code)?;
        session.eof(channel)?;
        session.close(channel)?;
        Ok(())
    }
}

struct Fixture {
    client: Arc<crate::conn::ssh::SshClient>,
    target: crate::model::ResolvedTarget,
    template: ConnCtxTemplate,
    server: Server,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn fixture() -> Fixture {
    let key = ssh_key::PrivateKey::from(ssh_key::private::Ed25519Keypair::from_seed(&[31; 32]));
    let socket = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = socket.local_addr().unwrap().port();
    let known = KnownHost {
        id: uuid::Uuid::nil(),
        host: "127.0.0.1".into(),
        port,
        key_type: "ssh-ed25519".into(),
        public_key: key.public_key().to_openssh().unwrap(),
        fingerprint: key
            .public_key()
            .fingerprint(russh::keys::HashAlg::Sha256)
            .to_string(),
        added: 0,
        updated: 0,
    };
    let server = Server::default();
    let mut running = server.clone();
    let config = Arc::new(server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let task = tokio::spawn(async move {
        running.run_on_socket(config, &socket).await.unwrap();
    });
    let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    let prompts = tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if let AppEvent::Prompt(prompt) = event {
                let _ = prompt.reply.send(PromptReply::Cancel);
            }
        }
    });
    let template = ConnCtxTemplate {
        bridge: Bridge {
            events: tx,
            ctx: egui::Context::default(),
        },
        known_hosts: vec![known],
        mosh_client: String::new(),
        detect_os: false,
        local_shell: String::new(),
    };
    let mut host = Host {
        address: "127.0.0.1".into(),
        ..Default::default()
    };
    host.settings.port = Some(port);
    host.settings.username = Some("fixture".into());
    host.settings.password = Some("fixture".into());
    host.settings.use_agent = Some(false);
    let target = Vault::default().resolve(&host);
    let client = tokio::time::timeout(
        Duration::from_secs(5),
        crate::conn::ssh::connect_headless(target.clone(), template.headless()),
    )
    .await
    .unwrap()
    .unwrap();
    Fixture {
        client,
        target,
        template,
        server,
        tasks: vec![task, prompts],
    }
}

fn listing() -> Listing {
    parse(
        r#"@docker
{"ID":"abcdef123456","Names":"api","Image":"nginx:1.27","State":"running"}
"#,
    )
}

#[tokio::test]
async fn container_queries_parse_real_ssh_responses_and_preserve_exit_codes() {
    let fixture = fixture().await;
    let listing = listing();
    let item = &listing.items[0];
    let samples = manager::stats(&fixture.client, &listing, false)
        .await
        .unwrap();
    assert_eq!(samples[&item.key()].cpu, Some(125.5));
    assert_eq!(samples[&item.key()].memory, "32MiB / 256MiB");
    let inspected: serde_json::Value = serde_json::from_str(
        &manager::inspect(&fixture.client, item, false)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(inspected["image"], "nginx:1.27");
    assert_eq!(logs(&fixture.client, item, false).await.unwrap(), "ready\n");
    assert_eq!(
        fixture.client.exec_limited("failure", 1024).await.unwrap(),
        ("permission denied".into(), 7)
    );
    assert_eq!(
        fixture
            .client
            .exec_limited("late-status", 1024)
            .await
            .unwrap(),
        ("failed".into(), 9)
    );
    let commands = fixture.server.commands.lock();
    assert_eq!(
        commands.iter().filter(|c| c.contains(" stats ")).count(),
        1,
        "one batch, no request per container"
    );
}

#[tokio::test]
async fn oversized_responses_fail_without_closing_the_shared_connection() {
    let fixture = fixture().await;
    let error = fixture
        .client
        .exec_limited("oversize", 64)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("exceeded"), "{error}");
    assert_eq!(
        fixture.client.exec("probe-owner").await.unwrap(),
        ("owner alive".into(), 0)
    );
}

#[tokio::test]
async fn closing_a_container_terminal_keeps_the_host_transport_alive() {
    let fixture = fixture().await;
    let mut target = fixture.target.clone();
    target.exec_command = Some("docker exec -it abcdef123456 sh".into());
    target.startup_script = Some("must never execute".into());
    let terminal = conn::spawn_ssh_channel(
        &tokio::runtime::Handle::current(),
        target,
        fixture.template.clone(),
        conn::SpawnOptions {
            scrollback: 100,
            log_dir: None,
        },
        fixture.client.clone(),
    );
    terminal.send_input(Input::Resize {
        cols: 80,
        rows: 24,
        px_w: 0,
        px_h: 0,
    });
    tokio::time::timeout(
        Duration::from_secs(5),
        fixture.server.terminal_started.notified(),
    )
    .await
    .unwrap();
    terminal.send_input(Input::Close);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(*terminal.status.lock(), Status::Closed(_)) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        fixture.client.exec("probe-owner").await.unwrap(),
        ("owner alive".into(), 0)
    );
    assert_eq!(
        fixture.server.commands.lock().as_slice(),
        ["docker exec -it abcdef123456 sh", "probe-owner"]
    );
}

async fn ready_address(tunnel: &crate::containers::ports::Tunnel) -> std::net::SocketAddr {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match tunnel.status() {
                ports::TunnelStatus::Ready(address) => return address,
                ports::TunnelStatus::Failed(error) => panic!("Tunnel failed: {error}"),
                _ => tokio::task::yield_now().await,
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn published_port_tunnel_serves_http_and_stop_releases_the_listener() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture = fixture().await;
    let remote = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = remote.local_addr().unwrap().port();
    *fixture.server.port_bindings.lock() =
        serde_json::json!({"80/tcp":[{"HostIp":"0.0.0.0", "HostPort":port.to_string()}]})
            .to_string();
    let http = tokio::spawn(async move {
        // The first connection is the readiness probe; the next carries the HTTP request.
        for _ in 0..2 {
            let (mut socket, _) = remote.accept().await.unwrap();
            let mut request = [0; 1024];
            let read = socket.read(&mut request).await.unwrap();
            if read == 0 {
                continue;
            }
            assert!(request[..read].starts_with(b"GET / HTTP/1.1"));
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 17\r\nConnection: close\r\n\r\nferrum-port-ready").await.unwrap();
        }
    });
    let binding = ports::parse(&fixture.server.port_bindings.lock())
        .unwrap()
        .remove(0);
    let mut tunnel = ports::Tunnel::start(
        &tokio::runtime::Handle::current(),
        fixture.client.clone(),
        listing().items.remove(0),
        binding,
        ports::OpenAs::Http,
        false,
        egui::Context::default(),
    );
    assert_eq!(
        tunnel.take_browser_url(),
        None,
        "a starting tunnel must not launch a browser"
    );
    let address = ready_address(&tunnel).await;
    assert_eq!(
        address.ip(),
        std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
    );
    assert_ne!(address.port(), 0);
    assert_eq!(
        tunnel.take_browser_url(),
        Some(format!("http://{address}/"))
    );
    assert_eq!(
        tunnel.take_browser_url(),
        None,
        "automatic browser launch occurs once"
    );
    let response = tokio::time::timeout(Duration::from_secs(5), async {
        let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
        socket
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut out = String::new();
        socket.read_to_string(&mut out).await.unwrap();
        out
    })
    .await
    .unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"));
    assert!(response.ends_with("ferrum-port-ready"));
    tokio::time::timeout(Duration::from_secs(5), http)
        .await
        .unwrap()
        .unwrap();
    tunnel.stop();
    assert_eq!(tunnel.status(), ports::TunnelStatus::Stopped);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if tokio::net::TcpStream::connect(address).await.is_err() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        fixture.client.exec("probe-owner").await.unwrap().0,
        "owner alive"
    );
    assert!(
        fixture
            .server
            .forwards
            .lock()
            .iter()
            .all(|(host, forwarded)| host == "127.0.0.1" && *forwarded == port as u32)
    );
}

#[tokio::test]
async fn denied_or_changed_bindings_fail_without_a_browser_request() {
    for changed in [true, false] {
        let fixture = fixture().await;
        *fixture.server.port_bindings.lock() =
            r#"{"80/tcp":[{"HostIp":"127.0.0.1","HostPort":"8080"}]}"#.into();
        let mut port = ports::parse(&fixture.server.port_bindings.lock())
            .unwrap()
            .remove(0);
        if changed {
            port.host_port = 8081;
        }
        fixture
            .server
            .deny_forward
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let mut tunnel = ports::Tunnel::start(
            &tokio::runtime::Handle::current(),
            fixture.client.clone(),
            listing().items.remove(0),
            port,
            ports::OpenAs::Http,
            false,
            egui::Context::default(),
        );
        let error = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let ports::TunnelStatus::Failed(error) = tunnel.status() {
                    break error;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!error.is_empty());
        if changed {
            assert!(error.contains("Published ports changed"));
            assert!(fixture.server.forwards.lock().is_empty());
        }
        assert_eq!(tunnel.take_browser_url(), None);
    }
}

#[tokio::test]
async fn removing_a_container_closes_its_tunnel_and_active_connections() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture = fixture().await;
    let remote = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = remote.local_addr().unwrap().port();
    *fixture.server.port_bindings.lock() =
        serde_json::json!({"80/tcp":[{"HostIp":"127.0.0.1", "HostPort":port.to_string()}]})
            .to_string();
    let echo = tokio::spawn(async move {
        loop {
            let (mut socket, _) = remote.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buffer = [0; 256];
                while let Ok(n) = socket.read(&mut buffer).await {
                    if n == 0 {
                        break;
                    }
                    if socket.write_all(&buffer[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    let item = listing().items.remove(0);
    let binding = ports::parse(&fixture.server.port_bindings.lock())
        .unwrap()
        .remove(0);
    let mut tunnel = ports::Tunnel::start(
        &tokio::runtime::Handle::current(),
        fixture.client.clone(),
        item.clone(),
        binding.clone(),
        ports::OpenAs::Tcp,
        false,
        egui::Context::default(),
    );
    let address = ready_address(&tunnel).await;
    assert_eq!(
        tunnel.take_browser_url(),
        None,
        "TCP-only mode never requests a browser"
    );
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    socket.write_all(b"held").await.unwrap();
    let mut bytes = [0; 4];
    tokio::time::timeout(Duration::from_secs(5), socket.read_exact(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes, b"held");
    let mut manager = manager::Manager::default();
    manager.tunnels.insert(binding.key(&item.key()), tunnel);
    manager.reconcile(&Listing::default());
    assert!(manager.tunnels.is_empty());
    let read = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut bytes))
        .await
        .unwrap();
    assert!(
        matches!(read, Ok(0) | Err(_)),
        "dropping a tunnel must close existing connections"
    );
    assert_eq!(
        fixture.client.exec("probe-owner").await.unwrap().0,
        "owner alive"
    );
    echo.abort();
}

#[tokio::test]
async fn published_port_tunnel_preserves_concurrent_html_css_and_script_responses() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture = fixture().await;
    let remote = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = remote.local_addr().unwrap().port();
    *fixture.server.port_bindings.lock() =
        serde_json::json!({"80/tcp":[{"HostIp":"127.0.0.1", "HostPort":port.to_string()}]})
            .to_string();
    let html = b"<!doctype html><link rel=stylesheet href=/assets/app.css><script src=/assets/app.js></script>".to_vec();
    let css = "body{color:#102030;background:#ffffff;}\n"
        .repeat(4096)
        .into_bytes();
    let js = "document.documentElement.dataset.ready='yes';\n"
        .repeat(1024)
        .into_bytes();
    let assets = Arc::new(std::collections::BTreeMap::from([
        ("/", ("text/html", html)),
        ("/assets/app.css", ("text/css", css)),
        ("/assets/app.js", ("application/javascript", js)),
    ]));
    let responses = assets.clone();
    let http = tokio::spawn(async move {
        let mut clients = tokio::task::JoinSet::new();
        // One readiness probe plus three independent browser asset requests.
        for _ in 0..4 {
            let (mut socket, _) = remote.accept().await.unwrap();
            let assets = responses.clone();
            clients.spawn(async move {
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    if socket.read(&mut byte).await.unwrap() == 0 { return; }
                    request.extend_from_slice(&byte);
                    assert!(request.len() < 2048);
                }
                let request = String::from_utf8(request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap();
                let (mime, body) = &assets[path];
                let header = format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                socket.write_all(header.as_bytes()).await.unwrap();
                socket.write_all(body).await.unwrap();
            });
        }
        while let Some(result) = clients.join_next().await {
            result.unwrap();
        }
    });
    let binding = ports::parse(&fixture.server.port_bindings.lock())
        .unwrap()
        .remove(0);
    let tunnel = ports::Tunnel::start(
        &tokio::runtime::Handle::current(),
        fixture.client.clone(),
        listing().items.remove(0),
        binding,
        ports::OpenAs::Tcp,
        false,
        egui::Context::default(),
    );
    let address = ready_address(&tunnel).await;
    let fetch = |path: &'static str| async move {
        let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
        socket
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        socket.read_to_end(&mut response).await.unwrap();
        (path, response)
    };
    let results = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(
            fetch("/"),
            fetch("/assets/app.css"),
            fetch("/assets/app.js")
        )
    })
    .await
    .unwrap();
    for (path, response) in [results.0, results.1, results.2] {
        let separator = response
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .unwrap();
        let header = std::str::from_utf8(&response[..separator]).unwrap();
        assert!(header.starts_with("HTTP/1.1 200 OK"));
        assert!(header.contains(&format!("Content-Type: {}", assets[path].0)));
        assert_eq!(
            &response[separator + 4..],
            assets[path].1.as_slice(),
            "asset bytes changed: {path}"
        );
    }
    tokio::time::timeout(Duration::from_secs(5), http)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn credentials_are_requested_on_demand_and_stopped_tunnels_clear_passwords() {
    let fixture = fixture().await;
    let remote = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let host_port = remote.local_addr().unwrap().port();
    *fixture.server.port_bindings.lock() =
        serde_json::json!({"5432/tcp":[{"HostIp":"127.0.0.1","HostPort":host_port.to_string()}]})
            .to_string();
    let mut item = listing().items.remove(0);
    item.detail = "postgres:17".into();
    let port = ports::parse(&fixture.server.port_bindings.lock())
        .unwrap()
        .remove(0);
    let mut tunnel = ports::Tunnel::start(
        &tokio::runtime::Handle::current(),
        fixture.client.clone(),
        item.clone(),
        port,
        ports::OpenAs::Tcp,
        false,
        egui::Context::default(),
    );
    let address = ready_address(&tunnel).await;
    assert!(tunnel.connection.password.is_empty());
    assert!(
        !fixture
            .server
            .commands
            .lock()
            .iter()
            .any(|c| c.contains(".Config.Env"))
    );
    tunnel.read_credentials(
        &tokio::runtime::Handle::current(),
        fixture.client.clone(),
        item,
        false,
        egui::Context::default(),
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while tunnel.credentials_pending {
            tunnel.poll_credentials();
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        tunnel.connection.uri(address, true),
        format!("postgresql://fixture:synthetic%3Dp%40ss@{address}/events")
    );
    assert_eq!(tunnel.take_browser_url(), None);
    tunnel.stop();
    assert_eq!(tunnel.connection.password, "");
    assert!(!tunnel.credentials_pending);
    assert_eq!(fixture.client.exec("owner-check").await.unwrap().1, 0);
}
