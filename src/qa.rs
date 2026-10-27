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
        })
    }
}

impl App {
    pub fn qa_seed(&mut self) {
        self.loaded = false; // never persist demo data
        self.vault.settings = crate::model::Settings::default();
        self.locked = false;
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
        self.dialog = None;
        self.editor = None;
        self.palette.open = false;
        match step {
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
            "local" => self.open_local(),
            "sftp" => self.open_sftp(None),
            "serial" => {
                self.dialog = Some(crate::app::Dialog::Serial {
                    cfg: Default::default(),
                    ports: crate::conn::serial::list_ports(),
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
            "light" => {
                self.vault.settings.ui_theme = crate::model::UiTheme::Light;
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
