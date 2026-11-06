//! Debug-only visual QA: `FERRUM_QA=home,sftp,... cargo run` walks the listed screens,
//! saves a PNG of each to `$FERRUM_QA_DIR` (default /tmp/ferrum-qa) and exits.
//! Demo data is injected and the vault is never saved in this mode.

use std::sync::Arc;

use crate::app::{App, Page, Tab};
use crate::model::{Group, Host, PortForward, Protocol, SshKey};

pub struct Qa {
    steps: Vec<String>,
    step: usize,
    frames: u32,
    waiting: bool,
    shot: Arc<parking_lot::Mutex<Option<Arc<egui::ColorImage>>>>,
    dir: std::path::PathBuf,
    /// Keyboard events to feed through egui's input on the next frame.
    pub keys: Vec<egui::Event>,
}

impl Qa {
    pub fn from_env() -> Option<Self> {
        let steps: Vec<String> = std::env::var("FERRUM_QA")
            .ok()?
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let dir = std::env::var("FERRUM_QA_DIR")
            .map(Into::into)
            .unwrap_or_else(|_| std::env::temp_dir().join("ferrum-qa"));
        std::fs::create_dir_all(&dir).ok()?;
        Some(Self {
            steps,
            step: 0,
            frames: 0,
            waiting: false,
            shot: Default::default(),
            dir,
            keys: vec![],
        })
    }
}

impl App {
    pub fn qa_seed(&mut self) {
        self.loaded = false; // never persist demo data
        self.vault.settings = crate::model::Settings::default();
        #[cfg(unix)]
        {
            self.vault.settings.local_shell = "/bin/sh".into();
        }
        self.locked = false;
        // Screenshots must never show the real vault's records.
        self.vault.known_hosts.clear();
        self.vault.identities.clear();
        self.vault.connection_logs.clear();
        self.vault.history = ["docker compose ps", "uptime", "systemctl status nginx"]
            .into_iter()
            .map(|command| crate::model::HistoryEntry {
                id: uuid::Uuid::new_v4(),
                command: command.into(),
                host_label: "api-gateway".into(),
                timestamp: 0,
            })
            .collect();
        let prod = Group {
            name: "Production".into(),
            ..Default::default()
        };
        let stage = Group {
            name: "Staging".into(),
            ..Default::default()
        };
        let mk = |label: &str,
                  addr: &str,
                  user: &str,
                  g: Option<uuid::Uuid>,
                  tags: &[&str],
                  os: Option<&str>| {
            let mut h = Host {
                label: label.into(),
                address: addr.into(),
                group_id: g,
                tags: tags.iter().map(|t| t.to_string()).collect(),
                os: os.map(Into::into),
                ..Default::default()
            };
            h.settings.username = Some(user.into());
            h
        };
        self.vault.hosts = vec![
            mk(
                "api-gateway",
                "10.0.1.12",
                "deploy",
                Some(prod.id),
                &["web", "k8s"],
                Some("ubuntu"),
            ),
            mk(
                "postgres-primary",
                "10.0.2.5",
                "postgres",
                Some(prod.id),
                &["db"],
                Some("debian"),
            ),
            mk(
                "redis-cache",
                "10.0.2.9",
                "admin",
                Some(prod.id),
                &["cache"],
                Some("alpine"),
            ),
            mk(
                "staging-web",
                "staging.example.com",
                "ubuntu",
                Some(stage.id),
                &["web"],
                Some("ubuntu"),
            ),
            mk(
                "home-nas",
                "192.168.1.20",
                "fikri",
                None,
                &["home"],
                Some("freebsd"),
            ),
            mk(
                "raspberry-pi",
                "pi.local",
                "pi",
                None,
                &["home", "iot"],
                Some("raspbian"),
            ),
        ];
        let mut serial = Host {
            label: "Router console".into(),
            address: "/dev/tty.usbserial".into(),
            protocol: Protocol::Serial,
            ..Default::default()
        };
        serial.serial.device = "/dev/tty.usbserial".into();
        self.vault.hosts.push(serial);
        self.vault.groups = vec![prod, stage];
        // Visual QA uses explicit fixtures and never probes demo addresses.
        for (index, latency) in [Some(24), Some(31), None, Some(28), Some(4), Some(6)]
            .into_iter()
            .enumerate()
        {
            self.host_status.insert(self.vault.hosts[index].id, latency);
        }
        self.vault.keys = vec![SshKey {
            label: "work-laptop".into(),
            key_type: "ssh-ed25519".into(),
            public_key:
                "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDemoDemoDemoDemoDemoDemoDemoDemoDemoDemo work"
                    .into(),
            ..Default::default()
        }];
        for (i, sn) in self.vault.snippets.iter_mut().enumerate() {
            sn.package = if i == 2 {
                "Docker".into()
            } else {
                "Server health".into()
            };
            if i == 0 {
                sn.hotkey = "Cmd+Alt+1".into();
            }
        }
        self.vault.workspaces = vec![crate::model::Workspace {
            id: uuid::Uuid::new_v4(),
            name: "Prod incident".into(),
            tabs: vec![
                crate::model::WorkspaceTab {
                    hosts: vec![Some(self.vault.hosts[0].id), Some(self.vault.hosts[1].id)],
                    layout: 0,
                    sftp: false,
                    title: None,
                },
                crate::model::WorkspaceTab {
                    hosts: vec![None],
                    layout: 0,
                    sftp: false,
                    title: None,
                },
                crate::model::WorkspaceTab {
                    hosts: vec![Some(self.vault.hosts[2].id)],
                    layout: 0,
                    sftp: true,
                    title: None,
                },
            ],
            updated: 0,
        }];
        let pg = self.vault.hosts[1].id;
        self.vault.forwards = vec![PortForward {
            label: "Postgres tunnel".into(),
            host_id: Some(pg),
            ..Default::default()
        }];
    }

    pub fn qa_tick(&mut self, ctx: &egui::Context) {
        let Some(qa) = self.qa.as_mut() else { return };
        ctx.request_repaint();
        if qa.waiting {
            // Wait for the screenshot event from the previous request.
            let shot = qa.shot.lock().take();
            if let Some(img) = shot {
                let name = qa.steps.get(qa.step).cloned().unwrap_or_default();
                save_png(&qa.dir.join(format!("{:02}-{name}.png", qa.step)), &img);
                qa.waiting = false;
                qa.step += 1;
                qa.frames = 0;
            }
            return;
        }
        if qa.step >= qa.steps.len() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if qa.frames == 0 {
            let step = qa.steps[qa.step].clone();
            self.qa_apply(&step);
        }
        let ready = self.qa.as_ref().is_some_and(|qa| qa.frames == 69);
        if ready {
            let step = self
                .qa
                .as_ref()
                .map(|qa| qa.steps[qa.step].as_str())
                .unwrap_or("");
            match step {
                "hostkeys" => assert_eq!(
                    self.inspected_host,
                    crate::ui::hosts::library_order(&self.vault, &self.filtered_hosts())
                        .get(1)
                        .copied(),
                    "Native ArrowDown must select the next host on screen"
                ),
                "filtered" => assert_eq!(
                    self.filtered_hosts()
                        .iter()
                        .map(|host| host.label.as_str())
                        .collect::<Vec<_>>(),
                    vec!["api-gateway"],
                    "Native search, group and tag filters must compose"
                ),
                "retrytransfer" => {
                    let jobs = self.transfers.lock();
                    let last = jobs.last().expect("Retried transfer job");
                    assert_eq!(*last.status.lock(), crate::sftp::TransferStatus::Done);
                    assert_eq!(last.done.load(std::sync::atomic::Ordering::Relaxed), 5);
                }
                _ => {}
            }
        }
        let qa = self.qa.as_mut().unwrap();
        qa.frames += 1;
        if qa.steps[qa.step] == "interactive" {
            return;
        }
        if qa.frames == 70 {
            qa.waiting = true;
            let slot = qa.shot.clone();
            ctx.request_screenshot(move |img| *slot.lock() = Some(img));
        }
    }

    fn qa_apply(&mut self, step: &str) {
        self.bridge
            .ctx
            .send_viewport_cmd(egui::ViewportCommand::Focus);
        self.dialog = None;
        self.editor = None;
        self.palette.open = false;
        self.host_inspector_open = false;
        match step {
            "wide" => self
                .bridge
                .ctx
                .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1440.0, 900.0))),
            "narrow" => self
                .bridge
                .ctx
                .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1024.0, 768.0))),
            "minimum" => self
                .bridge
                .ctx
                .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(760.0, 480.0))),
            "compact" => {
                self.active_tab = None;
                self.page = Page::Hosts;
                self.vault.settings.hosts_compact = true;
            }
            "comfortable" => {
                self.vault.settings.hosts_compact = false;
            }
            "inspector" => {
                self.active_tab = None;
                self.page = Page::Hosts;
                self.inspected_host = self.vault.hosts.first().map(|h| h.id);
                self.host_inspector_open = true;
            }
            "quickconnect" => {
                self.dialog = Some(crate::app::Dialog::QuickConnect {
                    input: String::new(),
                    error: None,
                });
            }
            "emptyhosts" => {
                self.active_tab = None;
                self.page = Page::Hosts;
                self.vault.hosts.clear();
                self.vault.groups.clear();
                self.inspected_host = None;
            }

            "interactive" => {}
            "about" => {
                self.dialog = Some(crate::app::Dialog::About);
            }
            "updatesettings" => {
                self.active_tab = None;
                self.page = Page::Settings;
                self.settings_section = 8;
            }
            "updatenotice" => {
                self.active_tab = None;
                self.page = Page::Hosts;
                #[cfg(debug_assertions)]
                self.updater.preview_notice();
            }
            "home" | "hosts" => {
                self.search.clear();
                self.tag_filter = None;
                self.current_group = None;
                self.active_tab = None;
                self.page = Page::Hosts;
            }
            "keychain" => {
                self.active_tab = None;
                self.page = Page::Keychain;
            }
            "forwarding" => {
                self.active_tab = None;
                self.page = Page::Forwarding;
            }
            "snippets" => {
                self.active_tab = None;
                self.page = Page::Snippets;
            }
            "knownhosts" => {
                self.active_tab = None;
                self.page = Page::KnownHosts;
            }
            "history" => {
                self.active_tab = None;
                self.page = Page::History;
            }
            "settings" => {
                self.active_tab = None;
                self.page = Page::Settings;
            }
            "edithost" => {
                self.active_tab = None;
                self.page = Page::Hosts;
                if let Some(h) = self.vault.hosts.first().cloned() {
                    self.editor = Some(crate::app::Editor::Host(Box::new(h), false));
                }
            }
            // Focuses the address line so a capture shows the focus treatment.
            "editfocus" => {
                self.active_tab = None;
                self.page = Page::Hosts;
                if let Some(h) = self.vault.hosts.first().cloned() {
                    self.editor = Some(crate::app::Editor::Host(Box::new(h), false));
                }
                self.bridge
                    .ctx
                    .memory_mut(|memory| memory.request_focus(crate::ui::forms::address_host_id()));
            }
            "workspaces" => {
                self.active_tab = None;
                self.page = Page::Workspaces;
            }
            "snippetedit" => {
                self.active_tab = None;
                self.page = Page::Snippets;
                if let Some(sn) = self.vault.snippets.first().cloned() {
                    self.editor = Some(crate::app::Editor::Snippet(sn, false));
                }
            }
            "saveworkspace" => {
                self.save_workspace("QA workspace");
            }
            "duplicate" => {
                if let Some(id) = self.active_tab {
                    self.duplicate_tab(id);
                }
            }
            "openws" => {
                if let Some(id) = self.vault.workspaces.last().map(|w| w.id) {
                    self.open_workspace(id);
                }
            }
            "palette" => {
                self.palette.open = true;
            }
            "local" => {
                self.open_local();
                if let Some(Tab::Terminal(tab)) = self.tabs.last() {
                    if let Some(pane) = tab.panes.first() {
                        pane.session.term.lock().feed(b"Ferrum visual QA session\r\nThese lines are fixture output for layout review.\r\n\r\n$ systemctl is-active nginx\r\n\x1b[32mactive\x1b[0m\r\n\r\n$ ");
                    }
                }
            }
            "sync" => {
                let root = std::env::temp_dir().join("ferrum-qa-sync");
                let source = root.join("source");
                let destination = root.join("destination");
                let setup = (|| -> std::io::Result<()> {
                    std::fs::create_dir_all(source.join("releases"))?;
                    std::fs::create_dir_all(&destination)?;
                    std::fs::write(source.join("deploy.sh"), "echo ready\n")?;
                    std::fs::write(source.join("releases/version.txt"), "0.4.0\n")?;
                    std::fs::write(destination.join("deploy.sh"), "echo old\n")?;
                    std::fs::write(destination.join("obsolete.txt"), "old\n")?;
                    Ok(())
                })();
                if let Err(error) = setup {
                    log::warn!("Sync QA fixtures: {error}");
                    return;
                }
                let mut left = crate::app::FsPane::new();
                let mut right = crate::app::FsPane::new();
                left.backend = Some(crate::sftp::Backend::Local);
                right.backend = Some(crate::sftp::Backend::Local);
                let (left_id, right_id) = (left.id, right.id);
                let id = crate::conn::next_id();
                self.tabs.push(Tab::Sftp(crate::app::SftpTab {
                    sync: None,
                    id,
                    title: "File transfer".into(),
                    left,
                    right,
                }));
                self.active_tab = Some(id);
                self.fs_navigate(left_id, source.to_string_lossy().into());
                self.fs_navigate(right_id, destination.to_string_lossy().into());
            }
            "syncpreview" => {
                let Some(Tab::Sftp(tab)) = self.tabs.last_mut() else {
                    return;
                };
                let (Some(left), Some(right)) = (&tab.left.backend, &tab.right.backend) else {
                    return;
                };
                let mut dialog = crate::ui::sync_view::SyncDialog::new(
                    crate::sftp::sync::Location {
                        backend: left.clone(),
                        path: tab.left.path.clone(),
                    },
                    crate::sftp::sync::Location {
                        backend: right.clone(),
                        path: tab.right.path.clone(),
                    },
                );
                dialog.compare(self.rt.handle(), &self.bridge.ctx);
                tab.sync = Some(dialog);
            }
            "sftp" => {
                let directory = std::env::temp_dir().join("ferrum-qa-files");
                if let Err(error) = std::fs::create_dir_all(directory.join("releases")) {
                    log::warn!("QA directory: {error}");
                }
                for (name, content) in [
                    ("deploy.sh", "#!/bin/sh\necho ready\n"),
                    ("nginx.conf", "server { listen 8080; }\n"),
                    ("README.md", "Ferrum visual QA files.\n"),
                ] {
                    if let Err(error) = std::fs::write(directory.join(name), content) {
                        log::warn!("QA fixture: {error}");
                    }
                }
                let mut left = crate::app::FsPane::new();
                left.backend = Some(crate::sftp::Backend::Local);
                let pane = left.id;
                let id = crate::conn::next_id();
                let mut right = crate::app::FsPane::new();
                right.host_picker = true;
                self.tabs.push(Tab::Sftp(crate::app::SftpTab {
                    sync: None,
                    id,
                    title: "File transfer".into(),
                    left,
                    right,
                }));
                self.active_tab = Some(id);
                self.fs_navigate(pane, directory.to_string_lossy().to_string());
            }
            "transferfailed" => {
                let root = std::env::temp_dir()
                    .join(format!("ferrum-qa-transfer-{}", uuid::Uuid::new_v4()));
                let destination = root.join("uploads");
                if let Err(error) = std::fs::create_dir_all(&destination) {
                    log::warn!("QA transfer directory: {error}");
                }
                self.queue_file_transfer(crate::ui::sftp_view::TransferRetry {
                    source: crate::sftp::Backend::Local,
                    entry: crate::sftp::FileEntry {
                        name: "release.tar.gz".into(),
                        path: root.join("release.tar.gz").to_string_lossy().into(),
                        is_dir: false,
                        is_link: false,
                        size: 0,
                        mtime: None,
                        perms: None,
                        owner: String::new(),
                    },
                    destination: crate::sftp::Backend::Local,
                    directory: destination.to_string_lossy().into(),
                    pane: u64::MAX,
                });
            }
            "hostkeys" => {
                self.active_tab = None;
                self.page = Page::Hosts;
                self.search.clear();
                self.current_group = None;
                self.tag_filter = None;
                let order = crate::ui::hosts::library_order(&self.vault, &self.filtered_hosts());
                if let Some(id) = order.first().copied() {
                    self.inspected_host = Some(id);
                    self.bridge.ctx.memory_mut(|memory| {
                        memory.request_focus(egui::Id::unique(("host-row", id)))
                    });
                }
                if let Some(qa) = self.qa.as_mut() {
                    for pressed in [true, false] {
                        qa.keys.push(egui::Event::Key {
                            key: egui::Key::ArrowDown,
                            physical_key: Some(egui::Key::ArrowDown),
                            pressed,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                }
            }
            "filtered" => {
                self.active_tab = None;
                self.page = Page::Hosts;
                self.current_group = self.vault.groups.first().map(|group| group.id);
                self.search = "deploy".into();
                self.tag_filter = Some("web".into());
            }
            "retrytransfer" => {
                let id = self
                    .transfers
                    .lock()
                    .iter()
                    .find(|transfer| *transfer.status.lock() == crate::sftp::TransferStatus::Failed)
                    .map(|transfer| transfer.id);
                if let Some(id) = id {
                    if let Some(request) = self.transfer_retries.get(&id) {
                        if let Err(error) = std::fs::write(&request.entry.path, b"ready") {
                            log::warn!("QA source file: {error}");
                        }
                    }
                    self.retry_file_transfer(id);
                }
            }
            "serial" => {
                self.dialog = Some(crate::app::Dialog::Serial {
                    cfg: Default::default(),
                });
            }
            "keygen" => {
                self.dialog = Some(crate::app::Dialog::KeyGen {
                    kind: crate::keys::KeyKind::Ed25519,
                    label: "my-key".into(),
                    passphrase: String::new(),
                    busy: false,
                });
            }
            "ssh" | "sftpremote" => {
                let host_id = self.qa_live_host();
                match (step, host_id) {
                    ("ssh", Some(h)) => self.open_host(h),
                    (_, Some(h)) => self.open_sftp(Some(h)),
                    _ => log::warn!("QA {step}: set FERRUM_IT_SSH and FERRUM_IT_KEY"),
                }
            }
            "accept" => {
                if let Some(mut p) = self.prompts.pop_front() {
                    if let Some(req) = p.req.take() {
                        let _ = req
                            .reply
                            .send(crate::conn::PromptReply::Accept { save: false });
                    }
                }
            }
            "type" => {
                let cmd = std::env::var("FERRUM_QA_TYPE")
                    .unwrap_or_else(|_| "uname -a; ls -la --color=auto /tmp | head -15\r".into());
                if let Some(Tab::Terminal(t)) = self
                    .active_tab
                    .and_then(|id| self.tabs.iter().find(|t| t.id() == id))
                {
                    if let Some(p) = t.panes.get(t.active) {
                        p.session.send_text(&cmd.replace("\\r", "\r"));
                    }
                }
            }
            "health" => {
                if let Some(Tab::Terminal(t)) = self
                    .active_tab
                    .and_then(|id| self.tabs.iter_mut().find(|t| t.id() == id))
                {
                    t.side_panel = Some(crate::app::SidePanel::Health);
                }
            }
            "containermanager" => {
                self.open_target(self.local_target());
                let Some(Tab::Terminal(tab)) = self.tabs.last_mut() else {
                    return;
                };
                let pane = &mut tab.panes[0];
                pane.session.target.protocol = Protocol::Ssh;
                pane.session.target.label = "Demo / build-node".into();
                *pane.session.status.lock() = crate::conn::Status::Connected;
                let mut browser = crate::containers::Browser::default();
                browser.session = pane.session.id;
                browser.last_poll = f64::MAX;
                browser.listing = Some(crate::containers::parse(
                    r#"@docker
{"ID":"abcdef123456","Names":"shop-api-1","Image":"ghcr.io/example/api:2026.11","State":"running","Status":"Up 2 hours (healthy)","Ports":"127.0.0.1:8080->8080/tcp","Labels":"com.docker.compose.project=shop,com.docker.compose.service=api"}
{"ID":"112233445566","Names":"shop-worker-1","Image":"ghcr.io/example/worker:2026.11","State":"running","Status":"Up 2 hours (healthy)","Labels":"com.docker.compose.project=shop,com.docker.compose.service=worker"}
{"ID":"aabbccddeeff","Names":"shop-db-1","Image":"postgres:17","State":"running","Status":"Up 2 days (healthy)","Ports":"5432/tcp","Labels":"com.docker.compose.project=shop,com.docker.compose.service=db"}
{"ID":"8899aabbccdd","Names":"old-migration","Image":"alpine:3.21","State":"exited","Status":"Exited (0) 3 days ago"}
"#,
                ));
                browser.manager.open = true;
                let items = browser
                    .listing
                    .as_ref()
                    .map(|l| l.items.clone())
                    .unwrap_or_default();
                for n in 0..18 {
                    let samples = items
                        .iter()
                        .filter(|i| i.is_running())
                        .enumerate()
                        .map(|(i, item)| {
                            (
                                item.key(),
                                crate::containers::manager::Stats {
                                    cpu: Some((n % 7) as f64 * 2.5 + i as f64 * 3.0 + 4.0),
                                    memory_percent: Some(20.0 + n as f64 * 0.4 + i as f64 * 8.0),
                                    memory: format!("{} MiB / 1 GiB", 200 + n * 4 + i * 80),
                                    network: "24 MB / 8 MB".into(),
                                    disk: "12 MB / 4 MB".into(),
                                    pids: "12".into(),
                                },
                            )
                        })
                        .collect();
                    browser.manager.apply_stats(Ok(samples));
                }
                if let Some(item) = items.first() {
                    browser.manager.select(item.key());
                }
                browser.manager.logs = Some(Ok("2026-11-06T08:40:01Z INFO  Listening on :8080\n2026-11-06T08:40:02Z INFO  Database connection ready\n2026-11-06T08:40:12Z INFO  GET /health 200 1ms\n2026-11-06T08:40:14Z INFO  GET /api/orders 200 12ms\n2026-11-06T08:40:18Z WARN  Queue latency above 250ms\n2026-11-06T08:40:21Z INFO  Worker recovered\n".into()));
                browser.manager.logs_updated = Some(std::time::Instant::now());
                browser.manager.inspect = Some(Ok(serde_json::to_string_pretty(&serde_json::json!({"image":"ghcr.io/example/api:2026.11", "state":"running", "ports":{"8080/tcp":[{"HostIp":"127.0.0.1", "HostPort":"8080"}]}})).unwrap_or_default()));
                browser.manager.last_stats_poll = f64::MAX;
                browser.manager.last_detail_poll = f64::MAX;
                tab.containers = browser;
            }
            "connectionscroll" => {
                if let Some(qa) = &mut self.qa {
                    qa.keys
                        .push(egui::Event::PointerMoved(egui::pos2(500.0, 350.0)));
                    qa.keys.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -3000.0),
                        phase: egui::TouchPhase::Move,
                        source: egui::MouseWheelSource::Wheel,
                        modifiers: Default::default(),
                    });
                }
            }
            #[cfg(debug_assertions)]
            "containerconnection" => {
                if let Some(Tab::Terminal(tab)) = self.tabs.last_mut() {
                    if let Some(item) = tab.containers.listing.as_ref().and_then(|l| {
                        l.items
                            .iter()
                            .find(|item| item.detail.starts_with("postgres:"))
                    }) {
                        let key = item.key();
                        tab.containers.manager.select(key.clone());
                        let port = crate::containers::ports::PublishedPort {
                            container_port: 5432,
                            protocol: "tcp".into(),
                            host: std::net::Ipv4Addr::LOCALHOST.into(),
                            host_port: 5432,
                        };
                        let mut tunnel = crate::containers::ports::Tunnel::qa_demo(
                            self.rt.handle(),
                            key.clone(),
                            port.clone(),
                        );
                        tunnel.connection.user = "app_reader".into();
                        tunnel.connection.password = "demo-password-only".into();
                        tunnel.connection.database = "analytics".into();
                        tab.containers
                            .manager
                            .tunnels
                            .insert(port.key(&key), tunnel);
                        tab.containers.manager.ports = Some(Ok(vec![port]));
                        tab.containers
                            .manager
                            .choose_tab(crate::containers::manager::DetailTab::Ports);
                        tab.containers.manager.focus_details = true;
                        tab.containers.manager.last_detail_poll = f64::MAX;
                    }
                }
            }
            "containerports" => {
                if let Some(Tab::Terminal(tab)) = self.tabs.last_mut() {
                    tab.containers
                        .manager
                        .choose_tab(crate::containers::manager::DetailTab::Ports);
                    tab.containers.manager.focus_details = true;
                    tab.containers.manager.ports = Some(crate::containers::ports::parse(r#"{"8080/tcp":[{"HostIp":"127.0.0.1","HostPort":"8080"},{"HostIp":"::","HostPort":"8080"}],"5353/udp":[{"HostIp":"0.0.0.0","HostPort":"5353"}]}"#).map_err(|error| error.to_string()));
                    tab.containers.manager.last_detail_poll = f64::MAX;
                }
            }
            "containerstats" | "containerinspect" | "containerlist" => {
                if let Some(Tab::Terminal(tab)) = self.tabs.last_mut() {
                    if step == "containerlist" {
                        tab.containers.manager.focus_details = false;
                    } else {
                        tab.containers.manager.focus_details = true;
                        tab.containers
                            .manager
                            .choose_tab(if step == "containerstats" {
                                crate::containers::manager::DetailTab::Stats
                            } else {
                                crate::containers::manager::DetailTab::Inspect
                            });
                    }
                }
            }
            "containers" => {
                if let Some(Tab::Terminal(t)) = self
                    .active_tab
                    .and_then(|id| self.tabs.iter_mut().find(|t| t.id() == id))
                {
                    t.side_panel = Some(crate::app::SidePanel::Containers);
                }
            }
            "dark" => self.vault.settings.ui_theme = crate::model::UiTheme::Dark,
            "midnight" => self.vault.settings.ui_theme = crate::model::UiTheme::Midnight,
            "light" => {
                self.vault.settings.ui_theme = crate::model::UiTheme::Light;
            }
            // Real keyboard input, so suggestions and line tracking run as for a user.
            "keys" | "right" | "enter" => {
                let key = |key| egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                };
                if let Some(qa) = self.qa.as_mut() {
                    qa.keys.push(match step {
                        "keys" => egui::Event::Text(
                            std::env::var("FERRUM_QA_KEYS").unwrap_or_else(|_| "git sta".into()),
                        ),
                        "right" => key(egui::Key::ArrowRight),
                        _ => key(egui::Key::Enter),
                    });
                }
            }
            _ => log::warn!("unknown QA step {step}"),
        }
    }
}

impl App {
    /// Adds (once) the live test server described by FERRUM_IT_SSH / FERRUM_IT_KEY.
    fn qa_live_host(&mut self) -> Option<uuid::Uuid> {
        if let Some(h) = self.vault.hosts.iter().find(|h| h.label == "qa-live") {
            return Some(h.id);
        }
        let spec = std::env::var("FERRUM_IT_SSH").ok()?;
        let key_text = std::fs::read_to_string(std::env::var("FERRUM_IT_KEY").ok()?).ok()?;
        let (user, host, port) = crate::model::parse_quick_connect(&spec)?;
        let key = crate::keys::import("qa-live-key", &key_text, "").ok()?;
        let mut h = Host {
            label: "qa-live".into(),
            address: host,
            ..Default::default()
        };
        h.settings.username = user;
        h.settings.port = port;
        h.settings.key_id = Some(key.id);
        h.settings.use_agent = Some(false);
        let id = h.id;
        self.vault.keys.push(key);
        self.vault.hosts.push(h);
        Some(id)
    }
}

fn save_png(path: &std::path::Path, img: &Arc<egui::ColorImage>) {
    let [w, h] = img.size;
    let bytes: Vec<u8> = img
        .pixels
        .iter()
        .flat_map(|c| c.to_srgba_unmultiplied())
        .collect();
    if let Err(e) = image::save_buffer(
        path,
        &bytes,
        w as u32,
        h as u32,
        image::ExtendedColorType::Rgba8,
    ) {
        log::error!("QA screenshot {}: {e}", path.display());
    }
}
