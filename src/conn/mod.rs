//! Connection backends (SSH, Mosh, Telnet, Serial, local shell) and the
//! session object the UI talks to.

pub mod local;
pub mod mosh;
pub mod serial;
pub mod ssh;
pub mod telnet;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::model::{KnownHost, Protocol, ResolvedTarget};
use crate::term::{Input, Listener, TermEvents, TermShared};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Connecting(String),
    Connected,
    Closed(String),
    Failed(String),
}

impl Status {
    pub fn is_live(&self) -> bool {
        matches!(self, Status::Connecting(_) | Status::Connected)
    }
}

#[derive(Clone, Debug)]
pub struct PromptField {
    pub label: String,
    pub secret: bool,
    pub value: String,
}

#[derive(Debug)]
pub enum PromptKind {
    HostKeyUnknown {
        host: String,
        port: u16,
        key_type: String,
        fingerprint: String,
    },
    HostKeyChanged {
        host: String,
        port: u16,
        key_type: String,
        fingerprint: String,
        old_fingerprint: String,
    },
    Fields {
        title: String,
        message: String,
        fields: Vec<PromptField>,
    },
}

#[derive(Debug)]
pub enum PromptReply {
    Accept { save: bool },
    Values(Vec<String>),
    Cancel,
}

pub struct PromptRequest {
    pub session: u64,
    pub kind: PromptKind,
    pub reply: oneshot::Sender<PromptReply>,
}

/// Events flowing from background tasks into the UI thread.
pub enum AppEvent {
    Prompt(PromptRequest),
    KnownHostAdd(KnownHost),
    KnownHostReplace(KnownHost),
    HostOs {
        host_id: Uuid,
        os: String,
    },
    Toast {
        text: String,
        error: bool,
    },
    ForwardState {
        rule: Uuid,
        running: bool,
        error: Option<String>,
    },
    Sftp(crate::sftp::SftpEvent),
    AiResult {
        tab: u64,
        result: Result<String, String>,
    },
    /// Container/pod listing for the panel of `tab`.
    Containers {
        tab: u64,
        session: u64,
        result: Result<crate::containers::Listing, String>,
    },
    ContainerAction {
        tab: u64,
        session: u64,
        result: Result<String, String>,
    },
    ContainerLogs {
        tab: u64,
        session: u64,
        key: String,
        result: Result<String, String>,
    },
    CloudImport(Result<Vec<crate::model::Host>, String>),
    SyncDone(Result<(String, crate::model::Vault), String>),
}

#[derive(Clone)]
pub struct Bridge {
    pub events: UnboundedSender<AppEvent>,
    pub ctx: egui::Context,
}

impl Bridge {
    pub fn send(&self, ev: AppEvent) {
        let _ = self.events.send(ev);
        self.ctx.request_repaint();
    }

    pub fn toast(&self, text: impl Into<String>, error: bool) {
        self.send(AppEvent::Toast {
            text: text.into(),
            error,
        });
    }

    pub async fn prompt(&self, session: u64, kind: PromptKind) -> PromptReply {
        let (tx, rx) = oneshot::channel();
        self.send(AppEvent::Prompt(PromptRequest {
            session,
            kind,
            reply: tx,
        }));
        rx.await.unwrap_or(PromptReply::Cancel)
    }

    pub async fn ask_fields(
        &self,
        session: u64,
        title: &str,
        message: &str,
        fields: &[(&str, bool)],
    ) -> Option<Vec<String>> {
        let kind = PromptKind::Fields {
            title: title.into(),
            message: message.into(),
            fields: fields
                .iter()
                .map(|(l, s)| PromptField {
                    label: l.to_string(),
                    secret: *s,
                    value: String::new(),
                })
                .collect(),
        };
        match self.prompt(session, kind).await {
            PromptReply::Values(v) => Some(v),
            _ => None,
        }
    }
}

/// Context handed to connection tasks.
#[derive(Clone)]
pub struct ConnCtx {
    pub session: u64,
    pub bridge: Bridge,
    pub known_hosts: Vec<KnownHost>,
    pub status: Arc<Mutex<Status>>,
    pub term: Arc<Mutex<TermShared>>,
    pub mosh_client: String,
    pub detect_os: bool,
    pub local_shell: String,
}

impl ConnCtx {
    pub fn set_status(&self, s: Status) {
        *self.status.lock() = s;
        self.bridge.ctx.request_repaint();
    }
    pub fn info(&self, msg: &str) {
        self.term.lock().status_line(msg);
        self.bridge.ctx.request_repaint();
    }
    pub fn feed(&self, bytes: &[u8]) {
        self.term.lock().feed(bytes);
        self.bridge.ctx.request_repaint();
    }
}

/// A live terminal session.
pub struct Session {
    pub id: u64,
    pub title: String,
    pub target: ResolvedTarget,
    pub term: Arc<Mutex<TermShared>>,
    pub events: Arc<Mutex<TermEvents>>,
    pub input: Arc<Mutex<UnboundedSender<Input>>>,
    pub status: Arc<Mutex<Status>>,
    pub ssh: Arc<Mutex<Option<Arc<ssh::SshClient>>>>,
    pub started: i64,
    pub log_id: Uuid,
    /// Characters typed since the last Enter, used for history and autocomplete.
    pub line_buf: String,
    pub reconnect_attempts: u32,
    pub reconnect_at: Option<f64>,
    pub user_closed: bool,
    pub last_size: (u16, u16),
}

impl Session {
    pub fn is_live(&self) -> bool {
        self.status.lock().is_live()
    }

    pub fn send(&self, bytes: Vec<u8>) {
        let _ = self.input.lock().send(Input::Data(bytes));
    }

    pub fn send_input(&self, input: Input) {
        let _ = self.input.lock().send(input);
    }

    pub fn send_text(&self, text: &str) {
        self.send(crate::term::encode_input(&self.target.charset, text));
    }
}

pub struct SpawnOptions {
    pub scrollback: usize,
    pub log_dir: Option<std::path::PathBuf>,
}

/// Create a session and start its backend on the tokio runtime.
pub fn spawn_session(
    rt: &tokio::runtime::Handle,
    target: ResolvedTarget,
    ctx_template: ConnCtxTemplate,
    opts: SpawnOptions,
) -> Session {
    let id = next_id();
    let (tx, rx) = unbounded_channel::<Input>();
    let events = Arc::new(Mutex::new(TermEvents::default()));
    let input = Arc::new(Mutex::new(tx));
    let listener = Listener {
        input: input.clone(),
        events: events.clone(),
        ctx: ctx_template.bridge.ctx.clone(),
    };
    let term = Arc::new(Mutex::new(TermShared::new(
        listener,
        opts.scrollback,
        &target.charset,
    )));
    let log_id = Uuid::new_v4();
    if let Some(dir) = opts.log_dir {
        let _ = std::fs::create_dir_all(&dir);
        let safe: String = target
            .label
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let name = format!(
            "{}-{}.log",
            safe,
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        term.lock().start_log(dir.join(name));
    }
    let status = Arc::new(Mutex::new(Status::Connecting("Starting…".into())));
    let ssh_slot = Arc::new(Mutex::new(None));
    let ctx = ConnCtx {
        session: id,
        bridge: ctx_template.bridge,
        known_hosts: ctx_template.known_hosts,
        status: status.clone(),
        term: term.clone(),
        mosh_client: ctx_template.mosh_client,
        detect_os: ctx_template.detect_os,
        local_shell: ctx_template.local_shell,
    };
    start_backend(rt, target.clone(), ctx, rx, ssh_slot.clone());
    Session {
        id,
        title: target.label.clone(),
        target,
        term,
        events,
        input,
        status,
        ssh: ssh_slot,
        started: crate::model::now_ts(),
        log_id,
        line_buf: String::new(),
        reconnect_attempts: 0,
        reconnect_at: None,
        user_closed: false,
        last_size: (0, 0),
    }
}

/// Restart the backend of an existing session (reconnect) keeping its scrollback.
pub fn restart_session(rt: &tokio::runtime::Handle, s: &mut Session, tpl: ConnCtxTemplate) {
    let (tx, rx) = unbounded_channel::<Input>();
    // The terminal listener shares this sender, so replies follow the new backend.
    *s.input.lock() = tx;
    *s.status.lock() = Status::Connecting("Reconnecting…".into());
    let ctx = ConnCtx {
        session: s.id,
        bridge: tpl.bridge,
        known_hosts: tpl.known_hosts,
        status: s.status.clone(),
        term: s.term.clone(),
        mosh_client: tpl.mosh_client,
        detect_os: false,
        local_shell: tpl.local_shell,
    };
    start_backend(rt, s.target.clone(), ctx, rx, s.ssh.clone());
    if s.last_size.0 > 0 {
        s.send_input(Input::Resize {
            cols: s.last_size.0,
            rows: s.last_size.1,
            px_w: 0,
            px_h: 0,
        });
    }
}

#[derive(Clone)]
pub struct ConnCtxTemplate {
    pub bridge: Bridge,
    pub known_hosts: Vec<KnownHost>,
    pub mosh_client: String,
    pub detect_os: bool,
    pub local_shell: String,
}

impl ConnCtxTemplate {
    /// Context for connections without a terminal (SFTP, port forwarding, key install).
    pub fn headless(&self) -> ConnCtx {
        let (tx, _rx) = unbounded_channel::<Input>();
        let listener = Listener {
            input: Arc::new(Mutex::new(tx)),
            events: Arc::new(Mutex::new(TermEvents::default())),
            ctx: self.bridge.ctx.clone(),
        };
        ConnCtx {
            session: 0,
            bridge: self.bridge.clone(),
            known_hosts: self.known_hosts.clone(),
            status: Arc::new(Mutex::new(Status::Connecting(String::new()))),
            term: Arc::new(Mutex::new(TermShared::new(listener, 100, "UTF-8"))),
            mosh_client: self.mosh_client.clone(),
            detect_os: false,
            local_shell: self.local_shell.clone(),
        }
    }
}

fn start_backend(
    rt: &tokio::runtime::Handle,
    target: ResolvedTarget,
    ctx: ConnCtx,
    rx: UnboundedReceiver<Input>,
    ssh_slot: Arc<Mutex<Option<Arc<ssh::SshClient>>>>,
) {
    match target.protocol {
        Protocol::Ssh => {
            rt.spawn(async move {
                let c2 = ctx.clone();
                if let Err(e) = ssh::run_terminal(target, ctx, rx, ssh_slot).await {
                    c2.info(&format!("\u{E4F6} {e:#}"));
                    c2.set_status(Status::Failed(format!("{e:#}")));
                }
            });
        }
        Protocol::Mosh => {
            let rt2 = rt.clone();
            rt.spawn(async move {
                let c2 = ctx.clone();
                if let Err(e) = mosh::run(target, ctx, rx, rt2).await {
                    c2.info(&format!("\u{E4F6} {e:#}"));
                    c2.set_status(Status::Failed(format!("{e:#}")));
                }
            });
        }
        Protocol::Telnet => {
            rt.spawn(async move {
                let c2 = ctx.clone();
                if let Err(e) = telnet::run(target, ctx, rx).await {
                    c2.info(&format!("\u{E4F6} {e:#}"));
                    c2.set_status(Status::Failed(format!("{e:#}")));
                }
            });
        }
        Protocol::Serial => {
            std::thread::spawn(move || {
                let c2 = ctx.clone();
                if let Err(e) = serial::run(target, ctx, rx) {
                    c2.info(&format!("\u{E4F6} {e:#}"));
                    c2.set_status(Status::Failed(format!("{e:#}")));
                }
            });
        }
        Protocol::Local => {
            std::thread::spawn(move || {
                let c2 = ctx.clone();
                if let Err(e) = local::run(None, ctx, rx) {
                    c2.info(&format!("\u{E4F6} {e:#}"));
                    c2.set_status(Status::Failed(format!("{e:#}")));
                }
            });
        }
    }
}
