//! Application state, tab management and the top-level frame layout.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use egui::{Color32, Key, RichText};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use uuid::Uuid;

use crate::conn::{self, AppEvent, Bridge, ConnCtxTemplate, PromptRequest, Session, Status};
use crate::forward::ForwardManager;
use crate::model::*;
use crate::sftp::{Backend, FileEntry, SftpEvent, Transfers};
use crate::store::{LoadOutcome, Store};
use crate::themes::{self, Palette};
use crate::ui::term_view::ViewState;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Hosts,
    Keychain,
    Forwarding,
    Snippets,
    Workspaces,
    KnownHosts,
    History,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SplitLayout {
    Horizontal,
    Vertical,
    Grid,
}

pub struct Pane {
    pub session: Session,
    pub view: ViewState,
}

pub struct TermTab {
    pub id: u64,
    pub panes: Vec<Pane>,
    pub layout: SplitLayout,
    pub active: usize,
    pub broadcast: bool,
    pub side_panel: Option<SidePanel>,
    pub ai_query: String,
    pub ai_result: Option<Result<String, String>>,
    pub ai_busy: bool,
    pub snippet_filter: String,
    pub custom_title: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SidePanel {
    Snippets,
    History,
    Ai,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Name,
    Size,
    Modified,
}

pub struct FsPane {
    pub id: u64,
    pub backend: Option<Backend>,
    pub host_id: Option<Uuid>,
    pub connecting: bool,
    pub path: String,
    pub path_edit: String,
    pub entries: Vec<FileEntry>,
    pub selected: HashSet<String>,
    pub filter: String,
    pub loading: bool,
    pub error: Option<String>,
    pub sort: SortKey,
    pub sort_asc: bool,
    pub back: Vec<String>,
    pub forward: Vec<String>,
    pub rename: Option<(String, String)>,
    pub new_folder: Option<String>,
    pub new_file: Option<String>,
    pub chmod: Option<(String, u32)>,
    pub confirm_delete: Option<Vec<FileEntry>>,
    pub host_picker: bool,
}

impl FsPane {
    pub fn new() -> Self {
        Self {
            id: conn::next_id(),
            backend: None,
            host_id: None,
            connecting: false,
            path: String::new(),
            path_edit: String::new(),
            entries: vec![],
            selected: HashSet::new(),
            filter: String::new(),
            loading: false,
            error: None,
            sort: SortKey::Name,
            sort_asc: true,
            back: vec![],
            forward: vec![],
            rename: None,
            new_folder: None,
            new_file: None,
            chmod: None,
            confirm_delete: None,
            host_picker: false,
        }
    }
}

pub struct SftpTab {
    pub id: u64,
    pub title: String,
    pub left: FsPane,
    pub right: FsPane,
}

pub enum Tab {
    Terminal(TermTab),
    Sftp(SftpTab),
}

impl Tab {
    pub fn id(&self) -> u64 {
        match self {
            Tab::Terminal(t) => t.id,
            Tab::Sftp(t) => t.id,
        }
    }
}

pub enum Editor {
    Host(Box<Host>, bool),
    Group(Group, bool),
    Identity(Identity, bool),
    Key(SshKey, bool),
    Forward(PortForward, bool),
    Snippet(Snippet, bool),
}

pub struct PromptState {
    pub req: Option<PromptRequest>,
    pub values: Vec<String>,
    pub save_host_key: bool,
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub at: f64,
}

pub enum Dialog {
    KeyGen {
        kind: crate::keys::KeyKind,
        label: String,
        passphrase: String,
        busy: bool,
    },
    KeyImport {
        label: String,
        private: String,
        passphrase: String,
        error: Option<String>,
    },
    Import {
        error: Option<String>,
    },
    Serial {
        cfg: SerialConfig,
        ports: Vec<String>,
    },
    InstallKey {
        key_id: Uuid,
        host_id: Option<Uuid>,
        busy: bool,
    },
    MasterPassword {
        current: String,
        new1: String,
        new2: String,
        error: Option<String>,
    },
    ExportVault {
        passphrase: String,
        error: Option<String>,
    },
    ImportVault {
        passphrase: String,
        error: Option<String>,
    },
    ConfirmDelete {
        ids: Vec<Uuid>,
        what: String,
    },
    RunSnippet {
        snippet: Snippet,
        values: Vec<(String, String)>,
        targets: Vec<(u64, bool)>,
    },
    SaveWorkspace {
        name: String,
    },
    RenameTab {
        tab: u64,
        title: String,
    },
    About,
    Updates,
}

pub struct PaletteState {
    pub open: bool,
    pub query: String,
    pub selected: usize,
}

pub struct EditWatch {
    pub local: std::path::PathBuf,
    pub remote: String,
    pub pane: u64,
    pub mtime: Option<std::time::SystemTime>,
}

pub struct App {
    pub rt: tokio::runtime::Runtime,
    pub updater: crate::updates::Updater,
    pub update_check_started: bool,
    pub menu_actions: Vec<String>,
    pub clipboard_actions: Vec<String>,
    pub edit_focus: Option<egui::Id>,
    #[cfg(target_os = "macos")]
    pub _native_menu: Option<muda::Menu>,
    pub store: Store,
    pub vault: Vault,
    pub loaded: bool,
    pub locked: bool,
    pub unlock_input: String,
    pub unlock_error: Option<String>,
    pub dirty: bool,
    pub last_save: Instant,
    pub bridge: Bridge,
    pub events: UnboundedReceiver<AppEvent>,
    pub tabs: Vec<Tab>,
    /// `None` = Vault (home) tab.
    pub active_tab: Option<u64>,
    pub page: Page,
    pub search: String,
    pub current_group: Option<Uuid>,
    pub tag_filter: Option<String>,
    pub selected_hosts: HashSet<Uuid>,
    pub editor: Option<Editor>,
    pub host_test: Option<crate::ui::editors::HostTest>,
    pub prompts: VecDeque<PromptState>,
    pub toasts: Vec<Toast>,
    pub dialog: Option<Dialog>,
    pub palette: PaletteState,
    pub forwards: ForwardManager,
    pub forward_status: HashMap<Uuid, (bool, Option<String>)>,
    pub transfers: Transfers,
    pub show_transfers: bool,
    pub edit_watches: Vec<EditWatch>,
    pub keyword_cache: (Vec<KeywordRule>, Vec<(regex::Regex, Color32)>),
    pub sync_passphrase: String,
    pub sync_status: String,
    pub sync_busy: bool,
    pub cloud_busy: bool,
    pub last_activity: Instant,
    pub style_key: (UiTheme, [u8; 3]),
    pub font_key: String,
    pub history_filter: String,
    pub key_search: String,
    pub snippet_search: String,
    pub settings_section: u8,
    pub drag_host: Option<Uuid>,
    pub qa: Option<crate::qa::Qa>,
    /// Deep links handed over by later launches; `None` entries are plain focus requests.
    pub handoffs: Option<std::sync::mpsc::Receiver<Option<String>>>,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        deep_link: Option<String>,
        instance: Option<crate::instance::Guard>,
    ) -> Self {
        #[cfg(target_os = "macos")]
        if let Err(error) = crate::native::install_glass(cc) {
            log::warn!("Native glass unavailable: {error}");
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(4)
            .build()
            .expect("tokio runtime");
        let (tx, rx) = unbounded_channel();
        let bridge = Bridge {
            events: tx,
            ctx: cc.egui_ctx.clone(),
        };
        let store = Store::new();
        let (vault, loaded, locked) = match store.probe() {
            Ok(LoadOutcome::Loaded(v)) => (v, true, false),
            Ok(LoadOutcome::Fresh) => (Self::starter_vault(), true, false),
            Ok(LoadOutcome::NeedsPassword) => (Vault::default(), false, true),
            Err(e) => {
                log::error!("vault load failed: {e:#}");
                (Vault::default(), false, true)
            }
        };
        let mut app = Self {
            rt,
            updater: crate::updates::Updater::default(),
            update_check_started: false,
            menu_actions: vec![],
            clipboard_actions: vec![],
            edit_focus: None,
            #[cfg(target_os = "macos")]
            _native_menu: crate::ui::menus::native_menu()
                .map_err(|e| log::warn!("Menu setup failed: {e}"))
                .ok(),
            store,
            vault,
            loaded,
            locked,
            unlock_input: String::new(),
            unlock_error: None,
            dirty: false,
            last_save: Instant::now(),
            bridge,
            events: rx,
            tabs: vec![],
            active_tab: None,
            page: Page::Hosts,
            search: String::new(),
            current_group: None,
            tag_filter: None,
            selected_hosts: HashSet::new(),
            editor: None,
            host_test: None,
            prompts: VecDeque::new(),
            toasts: vec![],
            dialog: None,
            palette: PaletteState {
                open: false,
                query: String::new(),
                selected: 0,
            },
            forwards: ForwardManager::default(),
            forward_status: HashMap::new(),
            transfers: Default::default(),
            show_transfers: false,
            edit_watches: vec![],
            keyword_cache: (vec![], vec![]),
            sync_passphrase: String::new(),
            sync_status: String::new(),
            sync_busy: false,
            cloud_busy: false,
            last_activity: Instant::now(),
            style_key: (UiTheme::Dark, [0, 0, 0]),
            font_key: String::from("\u{0}"),
            history_filter: String::new(),
            key_search: String::new(),
            snippet_search: String::new(),
            settings_section: 0,
            drag_host: None,
            qa: crate::qa::Qa::from_env(),
            handoffs: instance.map(|guard| guard.serve(cc.egui_ctx.clone())),
        };
        if app.qa.is_some() {
            app.qa_seed();
        }
        if let Some(link) = deep_link {
            app.open_quick_connect(&link);
        }
        app
    }

    fn starter_vault() -> Vault {
        let mut v = Vault::default();
        v.snippets.push(Snippet {
            name: "System overview".into(),
            script: "uname -a && uptime && df -h / && free -h 2>/dev/null".into(),
            tags: vec!["linux".into()],
            description: "Kernel, load, disk and memory".into(),
            ..Default::default()
        });
        v.snippets.push(Snippet {
            name: "Tail a log".into(),
            script: "sudo tail -n {{lines}} -f {{file}}".into(),
            tags: vec!["logs".into()],
            description: "Variables in {{braces}} are asked for when you run it".into(),
            ..Default::default()
        });
        v.snippets.push(Snippet {
            name: "Docker containers".into(),
            script: "docker ps --format 'table {{.Names}}\\t{{.Status}}\\t{{.Ports}}'".into(),
            tags: vec!["docker".into()],
            ..Default::default()
        });
        v
    }

    pub fn pal(&self) -> Palette {
        themes::palette(self.vault.settings.ui_theme, self.vault.settings.accent)
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        self.toasts.push(Toast {
            text: text.into(),
            error,
            at: self.bridge.ctx.input(|i| i.time),
        });
    }

    pub fn conn_template(&self) -> ConnCtxTemplate {
        ConnCtxTemplate {
            bridge: self.bridge.clone(),
            known_hosts: self.vault.known_hosts.clone(),
            mosh_client: self.vault.settings.mosh_client_path.clone(),
            detect_os: self.vault.settings.detect_os,
            local_shell: self.vault.settings.local_shell.clone(),
        }
    }

    fn spawn_opts(&self) -> conn::SpawnOptions {
        let s = &self.vault.settings;
        conn::SpawnOptions {
            scrollback: s.scrollback,
            log_dir: s.session_logging.then(|| {
                if s.log_dir.trim().is_empty() {
                    crate::store::default_log_dir()
                } else {
                    std::path::PathBuf::from(&s.log_dir)
                }
            }),
        }
    }

    pub fn new_session(&mut self, target: ResolvedTarget) -> Session {
        let s = conn::spawn_session(
            self.rt.handle(),
            target.clone(),
            self.conn_template(),
            self.spawn_opts(),
        );
        self.vault.connection_logs.push(ConnectionLog {
            id: s.log_id,
            host_label: target.label.clone(),
            address: target.address.clone(),
            protocol: target.protocol,
            started: now_ts(),
            ended: None,
            status: "Connecting".into(),
            log_file: s.term.lock().log_path.clone(),
        });
        if self.vault.connection_logs.len() > 1000 {
            self.vault.connection_logs.remove(0);
        }
        // Auto-start port forwarding rules bound to this host.
        if let Some(hid) = target.host_id {
            let rules: Vec<PortForward> = self
                .vault
                .forwards
                .iter()
                .filter(|f| f.auto_start && f.host_id == Some(hid))
                .cloned()
                .collect();
            for r in rules {
                self.start_forward(r);
            }
            if let Some(h) = self.vault.hosts.iter_mut().find(|h| h.id == hid) {
                h.last_connected = Some(now_ts());
            }
        }
        self.mark_dirty();
        s
    }

    pub fn open_target(&mut self, target: ResolvedTarget) {
        let session = self.new_session(target);
        let id = conn::next_id();
        self.tabs.push(Tab::Terminal(TermTab {
            id,
            panes: vec![Pane {
                session,
                view: ViewState::default(),
            }],
            layout: SplitLayout::Horizontal,
            active: 0,
            broadcast: false,
            side_panel: None,
            ai_query: String::new(),
            ai_result: None,
            ai_busy: false,
            snippet_filter: String::new(),
            custom_title: None,
        }));
        self.active_tab = Some(id);
    }

    pub fn open_host(&mut self, host_id: Uuid) {
        if let Some(h) = self.vault.host(host_id).cloned() {
            let t = self.vault.resolve(&h);
            self.open_target(t);
        }
    }

    pub fn local_target(&self) -> ResolvedTarget {
        let mut h = Host {
            label: "Local terminal".into(),
            protocol: Protocol::Local,
            ..Default::default()
        };
        h.id = Uuid::nil();
        let mut t = self.vault.resolve(&h);
        t.host_id = None;
        t
    }

    pub fn open_local(&mut self) {
        let t = self.local_target();
        self.open_target(t);
    }

    pub fn open_quick_connect(&mut self, input: &str) {
        let Some((user, host, port)) = parse_quick_connect(input) else {
            self.toast("Use the form user@host:port", true);
            return;
        };
        let mut h = Host {
            label: host.clone(),
            address: host,
            ..Default::default()
        };
        h.settings.username = user;
        h.settings.port = port;
        if self.vault.settings.save_quick_connect {
            self.vault.hosts.push(h.clone());
            self.mark_dirty();
        }
        let mut t = self.vault.resolve(&h);
        if !self.vault.settings.save_quick_connect {
            t.host_id = None;
        }
        self.open_target(t);
    }

    pub fn open_sftp(&mut self, host_id: Option<Uuid>) {
        let id = conn::next_id();
        let mut left = FsPane::new();
        left.backend = Some(Backend::Local);
        let home = directories::UserDirs::new()
            .map(|u| u.home_dir().to_string_lossy().to_string())
            .unwrap_or("/".into());
        let mut right = FsPane::new();
        let title = host_id
            .and_then(|h| self.vault.host(h))
            .map(|h| format!("SFTP · {}", h.display_name()))
            .unwrap_or_else(|| "SFTP".into());
        let left_id = left.id;
        if host_id.is_none() {
            right.host_picker = true;
        }
        self.tabs.push(Tab::Sftp(SftpTab {
            id,
            title,
            left,
            right,
        }));
        self.active_tab = Some(id);
        self.fs_navigate(left_id, home);
        if let Some(h) = host_id {
            let rid = match self.tabs.last() {
                Some(Tab::Sftp(t)) => t.right.id,
                _ => return,
            };
            self.fs_connect(rid, h);
        }
    }

    pub fn start_forward(&mut self, rule: PortForward) {
        let Some(host) = rule.host_id.and_then(|h| self.vault.host(h)).cloned() else {
            self.toast("Choose a host for this rule first", true);
            return;
        };
        let target = self.vault.resolve(&host);
        let ctx = self.conn_template().headless();
        self.forward_status.insert(rule.id, (false, None));
        self.forwards.start(self.rt.handle(), rule, target, ctx);
    }

    pub fn find_pane_mut(&mut self, session_id: u64) -> Option<&mut Pane> {
        self.tabs.iter_mut().find_map(|t| match t {
            Tab::Terminal(tt) => tt.panes.iter_mut().find(|p| p.session.id == session_id),
            _ => None,
        })
    }

    pub fn fs_pane_mut(&mut self, pane: u64) -> Option<&mut FsPane> {
        self.tabs.iter_mut().find_map(|t| match t {
            Tab::Sftp(s) => {
                if s.left.id == pane {
                    Some(&mut s.left)
                } else if s.right.id == pane {
                    Some(&mut s.right)
                } else {
                    None
                }
            }
            _ => None,
        })
    }

    pub fn fs_navigate(&mut self, pane: u64, path: String) {
        let show_hidden = self.vault.settings.sftp_show_hidden;
        let _ = show_hidden;
        let Some(p) = self.fs_pane_mut(pane) else {
            return;
        };
        let Some(backend) = p.backend.clone() else {
            return;
        };
        p.loading = true;
        p.error = None;
        let bridge = self.bridge.clone();
        self.rt.spawn(async move {
            let result = crate::sftp::list(&backend, &path)
                .await
                .map_err(|e| format!("{e:#}"));
            bridge.send(AppEvent::Sftp(SftpEvent::Listing { pane, path, result }));
        });
    }

    pub fn fs_connect(&mut self, pane: u64, host_id: Uuid) {
        let Some(host) = self.vault.host(host_id).cloned() else {
            return;
        };
        let target = self.vault.resolve(&host);
        let ctx = self.conn_template().headless();
        let bridge = self.bridge.clone();
        if let Some(p) = self.fs_pane_mut(pane) {
            p.connecting = true;
            p.host_picker = false;
            p.host_id = Some(host_id);
            p.error = None;
        }
        let label = host.display_name();
        self.rt.spawn(async move {
            let res = async {
                let client = crate::conn::ssh::connect_headless(target, ctx).await?;
                let sftp = client.sftp().await?;
                anyhow::Ok(Backend::Remote {
                    sftp: std::sync::Arc::new(sftp),
                    client,
                    label,
                })
            }
            .await;
            match res {
                Ok(backend) => {
                    let home = crate::sftp::home_dir(&backend).await;
                    bridge.send(AppEvent::Sftp(SftpEvent::Connected {
                        pane,
                        backend,
                        home,
                    }));
                }
                Err(e) => bridge.send(AppEvent::Sftp(SftpEvent::ConnectFailed {
                    pane,
                    error: format!("{e:#}"),
                })),
            }
        });
    }

    /// Drain background events.
    fn process_events(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                AppEvent::Prompt(req) => {
                    if req.reply.is_closed() {
                        continue;
                    }
                    let values = match &req.kind {
                        conn::PromptKind::Fields { fields, .. } => {
                            fields.iter().map(|f| f.value.clone()).collect()
                        }
                        _ => vec![],
                    };
                    self.prompts.push_back(PromptState {
                        req: Some(req),
                        values,
                        save_host_key: true,
                    });
                }
                AppEvent::KnownHostAdd(k) => {
                    self.vault.known_hosts.push(k);
                    self.mark_dirty();
                }
                AppEvent::KnownHostReplace(k) => {
                    self.vault
                        .known_hosts
                        .retain(|x| !(x.host == k.host && x.port == k.port));
                    self.vault.known_hosts.push(k);
                    self.mark_dirty();
                }
                AppEvent::HostOs { host_id, os } => {
                    if let Some(h) = self.vault.hosts.iter_mut().find(|h| h.id == host_id) {
                        if h.os.as_deref() != Some(os.as_str()) {
                            h.os = Some(os);
                            h.updated = now_ts();
                            self.dirty = true;
                        }
                    }
                }
                AppEvent::Toast { text, error } => self.toast(text, error),
                AppEvent::ForwardState {
                    rule,
                    running,
                    error,
                } => {
                    self.forward_status.insert(rule, (running, error));
                }
                AppEvent::Sftp(ev) => self.handle_sftp_event(ev),
                AppEvent::AiResult { tab, result } => {
                    for t in self.tabs.iter_mut() {
                        if let Tab::Terminal(tt) = t {
                            if tt.id == tab {
                                tt.ai_busy = false;
                                tt.ai_result = Some(result.clone());
                            }
                        }
                    }
                }
                AppEvent::CloudImport(res) => {
                    self.cloud_busy = false;
                    match res {
                        Ok(hosts) => {
                            let mut added = 0;
                            for h in hosts {
                                if !self.vault.hosts.iter().any(|x| x.address == h.address) {
                                    self.vault.hosts.push(h);
                                    added += 1;
                                }
                            }
                            self.toast(format!("Imported {added} new hosts"), false);
                            self.mark_dirty();
                        }
                        Err(e) => self.toast(e, true),
                    }
                }
                AppEvent::SyncDone(res) => {
                    self.sync_busy = false;
                    match res {
                        Ok((msg, merged)) => {
                            self.vault = merged;
                            self.sync_status = msg;
                            self.mark_dirty();
                        }
                        Err(e) => {
                            self.sync_status = format!("Sync failed: {e}");
                            self.toast(self.sync_status.clone(), true);
                        }
                    }
                }
            }
        }
        // Session bookkeeping: titles, bells, clipboard, status → logs, auto-reconnect.
        let now = ctx.input(|i| i.time);
        let auto_reconnect = self.vault.settings.auto_reconnect;
        let bell_sound = self.vault.settings.bell_sound;
        let mut reconnect: Vec<u64> = vec![];
        let mut log_updates: Vec<(Uuid, String, bool)> = vec![];
        for t in self.tabs.iter_mut() {
            if let Tab::Terminal(tt) = t {
                for p in tt.panes.iter_mut() {
                    let s = &mut p.session;
                    let mut ev = s.events.lock();
                    if let Some(title) = ev.title.take() {
                        if !title.trim().is_empty() {
                            s.title = title;
                        }
                    }
                    if std::mem::take(&mut ev.bell) && bell_sound {
                        ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                            egui::UserAttentionType::Informational,
                        ));
                    }
                    if let Some(text) = ev.clipboard.take() {
                        ctx.copy_text(text);
                    }
                    drop(ev);
                    let status = s.status.lock().clone();
                    match &status {
                        Status::Connected => {
                            s.reconnect_attempts = 0;
                            log_updates.push((s.log_id, "Connected".into(), false));
                        }
                        Status::Closed(r) | Status::Failed(r) => {
                            log_updates.push((s.log_id, r.clone(), true));
                            let is_failure = matches!(status, Status::Failed(_))
                                || r.contains("Connection closed");
                            if auto_reconnect
                                && !s.user_closed
                                && is_failure
                                && s.target.protocol != Protocol::Local
                                && s.reconnect_attempts < 5
                                && !r.contains("cancelled")
                                && !r.contains("authentication failed")
                            {
                                match s.reconnect_at {
                                    None => {
                                        s.reconnect_at =
                                            Some(now + 2.0 + s.reconnect_attempts as f64 * 3.0)
                                    }
                                    Some(at) if now >= at => {
                                        s.reconnect_at = None;
                                        s.reconnect_attempts += 1;
                                        reconnect.push(s.id);
                                    }
                                    Some(_) => ctx.request_repaint_after(
                                        std::time::Duration::from_millis(500),
                                    ),
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        for (id, status, ended) in log_updates {
            if let Some(l) = self.vault.connection_logs.iter_mut().find(|l| l.id == id) {
                if l.status != status {
                    l.status = status;
                    if ended && l.ended.is_none() {
                        l.ended = Some(now_ts());
                    }
                    self.dirty = true;
                }
            }
        }
        for sid in reconnect {
            self.reconnect_session(sid);
        }
    }

    pub fn reconnect_session(&mut self, sid: u64) {
        let tpl = self.conn_template();
        let handle = self.rt.handle().clone();
        if let Some(p) = self.find_pane_mut(sid) {
            p.session.user_closed = false;
            p.session.term.lock().status_line("\u{E036} Reconnecting…");
            conn::restart_session(&handle, &mut p.session, tpl);
        }
    }

    fn handle_sftp_event(&mut self, ev: SftpEvent) {
        match ev {
            SftpEvent::Connected {
                pane,
                backend,
                home,
            } => {
                if let Some(p) = self.fs_pane_mut(pane) {
                    p.backend = Some(backend);
                    p.connecting = false;
                }
                self.fs_navigate(pane, home);
            }
            SftpEvent::ConnectFailed { pane, error } => {
                if let Some(p) = self.fs_pane_mut(pane) {
                    p.connecting = false;
                    p.error = Some(error);
                    p.host_picker = true;
                }
            }
            SftpEvent::Listing { pane, path, result } => {
                if let Some(p) = self.fs_pane_mut(pane) {
                    p.loading = false;
                    match result {
                        Ok(entries) => {
                            if !p.path.is_empty() && p.path != path {
                                p.back.push(std::mem::take(&mut p.path));
                            }
                            p.path = path.clone();
                            p.path_edit = path;
                            p.entries = entries;
                            p.selected.clear();
                        }
                        Err(e) => p.error = Some(e),
                    }
                }
            }
            SftpEvent::OpDone { pane, result } => {
                match result {
                    Ok(msg) => {
                        if !msg.is_empty() {
                            self.toast(msg, false);
                        }
                    }
                    Err(e) => self.toast(e, true),
                }
                let path = self.fs_pane_mut(pane).map(|p| p.path.clone());
                if let Some(path) = path {
                    self.fs_refresh_keep_history(pane, path);
                }
            }
            SftpEvent::EditReady {
                local,
                remote,
                pane,
            } => {
                let editor = self.vault.settings.editor_for(&remote);
                let res = if editor.trim().is_empty() {
                    open::that_detached(&local).map_err(|e| e.to_string())
                } else {
                    let mut parts = shell_words::split(&editor).unwrap_or_default();
                    if parts.is_empty() {
                        Err("bad editor command".into())
                    } else {
                        let prog = parts.remove(0);
                        std::process::Command::new(prog)
                            .args(parts)
                            .arg(&local)
                            .spawn()
                            .map(|_| ())
                            .map_err(|e| e.to_string())
                    }
                };
                if let Err(e) = res {
                    self.toast(format!("Could not open editor: {e}"), true);
                }
                let mtime = std::fs::metadata(&local).and_then(|m| m.modified()).ok();
                self.edit_watches.push(EditWatch {
                    local,
                    remote,
                    pane,
                    mtime,
                });
            }
        }
    }

    pub fn fs_refresh_keep_history(&mut self, pane: u64, path: String) {
        if let Some(p) = self.fs_pane_mut(pane) {
            p.path.clear();
        }
        self.fs_navigate(pane, path);
    }

    /// Upload locally edited files when they change on disk.
    fn poll_edit_watches(&mut self) {
        let mut uploads = vec![];
        for w in self.edit_watches.iter_mut() {
            let m = std::fs::metadata(&w.local).and_then(|m| m.modified()).ok();
            if m.is_some() && m != w.mtime {
                w.mtime = m;
                uploads.push((w.local.clone(), w.remote.clone(), w.pane));
            }
        }
        for (local, remote, pane) in uploads {
            let backend = self.fs_pane_mut(pane).and_then(|p| p.backend.clone());
            if let Some(b) = backend {
                let bridge = self.bridge.clone();
                self.rt.spawn(async move {
                    let res = crate::sftp::upload_file(&b, &local, &remote)
                        .await
                        .map(|_| format!("Saved {remote}"))
                        .map_err(|e| format!("Upload failed: {e:#}"));
                    bridge.send(AppEvent::Sftp(SftpEvent::OpDone { pane, result: res }));
                });
            }
        }
    }

    fn save_if_needed(&mut self) {
        if self.dirty && self.loaded && self.last_save.elapsed().as_millis() > 800 {
            if let Err(e) = self.store.save(&self.vault) {
                self.toast(format!("Saving vault failed: {e:#}"), true);
            }
            self.dirty = false;
            self.last_save = Instant::now();
            if self.vault.settings.auto_sync && !self.sync_passphrase.is_empty() && !self.sync_busy
            {
                // Debounced background sync after local changes.
                self.start_sync();
            }
        }
    }

    pub fn start_sync(&mut self) {
        if self.sync_busy {
            return;
        }
        self.sync_busy = true;
        let mut v = self.vault.clone();
        let pass = self.sync_passphrase.clone();
        let bridge = self.bridge.clone();
        self.rt.spawn_blocking(move || {
            let res = crate::store::sync_with_folder(&mut v, &pass)
                .map(|m| (m, v))
                .map_err(|e| format!("{e:#}"));
            bridge.send(AppEvent::SyncDone(res));
        });
    }

    fn ensure_style(&mut self, ctx: &egui::Context) {
        let key = (self.vault.settings.ui_theme, self.vault.settings.accent);
        if key != self.style_key {
            themes::apply_style(ctx, key.0, key.1);
            self.style_key = key;
        }
        if self.font_key != self.vault.settings.custom_font_path {
            self.font_key = self.vault.settings.custom_font_path.clone();
            crate::fonts::install(ctx, &self.font_key);
        }
        if self.keyword_cache.0 != self.vault.settings.keyword_rules {
            let rules = self.vault.settings.keyword_rules.clone();
            let compiled = rules
                .iter()
                .filter(|r| r.enabled)
                .filter_map(|r| {
                    regex::Regex::new(&r.pattern)
                        .ok()
                        .map(|re| (re, Color32::from_rgb(r.color[0], r.color[1], r.color[2])))
                })
                .collect();
            self.keyword_cache = (rules, compiled);
        }
    }

    pub fn close_tab(&mut self, id: u64) {
        if let Some(idx) = self.tabs.iter().position(|t| t.id() == id) {
            let tab = self.tabs.remove(idx);
            if let Tab::Terminal(t) = tab {
                for mut p in t.panes {
                    p.session.user_closed = true;
                    p.session.send_input(crate::term::Input::Close);
                }
            }
            if self.active_tab == Some(id) {
                self.active_tab = if self.tabs.is_empty() {
                    None
                } else {
                    Some(self.tabs[idx.min(self.tabs.len() - 1)].id())
                };
            }
        }
    }

    pub fn split_active(&mut self, layout: SplitLayout) {
        let Some(id) = self.active_tab else { return };
        let target = self.tabs.iter().find_map(|t| match t {
            Tab::Terminal(tt) if tt.id == id => {
                tt.panes.get(tt.active).map(|p| p.session.target.clone())
            }
            _ => None,
        });
        if let Some(target) = target {
            let session = self.new_session(target);
            if let Some(Tab::Terminal(tt)) = self.tabs.iter_mut().find(|t| t.id() == id) {
                tt.panes.push(Pane {
                    session,
                    view: ViewState::default(),
                });
                tt.active = tt.panes.len() - 1;
                tt.layout = if tt.panes.len() > 2 && layout != tt.layout {
                    SplitLayout::Grid
                } else {
                    layout
                };
            }
        }
    }

    /// Run any snippet whose hotkey was pressed in the active terminal pane.
    fn snippet_hotkeys(&mut self, ctx: &egui::Context) {
        if self.editor.is_some() || self.dialog.is_some() || !self.prompts.is_empty() {
            return;
        }
        let mut hits: Vec<Snippet> = vec![];
        for sn in &self.vault.snippets {
            if let Some((mods, key)) = crate::hotkeys::parse(&sn.hotkey) {
                if ctx.input_mut(|i| i.consume_key(mods, key)) {
                    hits.push(sn.clone());
                }
            }
        }
        for sn in hits {
            let active = self.active_tab.and_then(|id| {
                self.tabs.iter().find_map(|t| match t {
                    Tab::Terminal(tt) if tt.id == id => tt
                        .panes
                        .get(tt.active)
                        .filter(|p| p.session.is_live())
                        .map(|p| p.session.id),
                    _ => None,
                })
            });
            match active {
                None => self.toast("Open a terminal to run this snippet", true),
                Some(_) if sn.script.contains("{{") => self.begin_run_snippet(sn),
                Some(sid) => self.run_snippet_now(&sn.script, &[sid]),
            }
        }
    }

    fn global_shortcuts(&mut self, ctx: &egui::Context) {
        let cmd = |k: Key| ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, k));
        let cmd_shift = |k: Key| {
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, k))
        };
        if cmd(Key::K) || cmd_shift(Key::P) {
            self.palette.open = !self.palette.open;
            self.palette.query.clear();
            self.palette.selected = 0;
        }
        if cmd(Key::T) {
            self.open_local();
        }
        if cmd(Key::W) {
            if let Some(id) = self.active_tab {
                self.close_tab(id);
            }
        }
        if cmd(Key::Comma) {
            self.active_tab = None;
            self.page = Page::Settings;
        }
        if cmd_shift(Key::D) {
            self.split_active(SplitLayout::Vertical);
        } else if cmd(Key::D) {
            self.split_active(SplitLayout::Horizontal);
        }
        if cmd(Key::Equals) || cmd(Key::Plus) {
            self.vault.settings.font_size = (self.vault.settings.font_size + 1.0).min(36.0);
            self.mark_dirty();
        }
        if cmd(Key::Minus) {
            self.vault.settings.font_size = (self.vault.settings.font_size - 1.0).max(8.0);
            self.mark_dirty();
        }
        if cmd(Key::Num0) {
            self.vault.settings.font_size = 14.0;
            self.mark_dirty();
        }
        let nums = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ];
        for (i, k) in nums.iter().enumerate() {
            if cmd(*k) {
                self.active_tab = if i == 0 {
                    None
                } else {
                    self.tabs.get(i - 1).map(|t| t.id())
                };
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, Key::Tab))
            && !self.tabs.is_empty()
        {
            let pos = self
                .active_tab
                .and_then(|id| self.tabs.iter().position(|t| t.id() == id));
            self.active_tab = match pos {
                None => Some(self.tabs[0].id()),
                Some(p) if p + 1 < self.tabs.len() => Some(self.tabs[p + 1].id()),
                Some(_) => None,
            };
        }
    }

    fn lock_screen(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(pal.bg))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.25);
                    ui.label(RichText::new("\u{E308}").size(48.0));
                    ui.add_space(8.0);
                    ui.label(RichText::new("Ferrum is locked").size(24.0).strong());
                    ui.label(
                        RichText::new("Enter your master password to unlock the vault")
                            .color(pal.text_dim),
                    );
                    ui.add_space(16.0);
                    let te = crate::ui::password_input(
                        ui,
                        &mut self.unlock_input,
                        "Master password",
                        280.0,
                    );
                    te.request_focus();
                    if let Some(e) = &self.unlock_error {
                        ui.colored_label(pal.danger, e);
                    }
                    ui.add_space(8.0);
                    let go = ui.add(crate::ui::primary_button("Unlock", pal)).clicked()
                        || (te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)));
                    if go {
                        self.try_unlock();
                    }
                });
            });
    }

    fn try_unlock(&mut self) {
        let pw = std::mem::take(&mut self.unlock_input);
        if self.loaded {
            if self.store.master_password.as_deref() == Some(pw.as_str()) {
                self.locked = false;
                self.unlock_error = None;
            } else {
                self.unlock_error = Some("Wrong password".into());
            }
            return;
        }
        self.store.master_password = Some(pw);
        match self.store.probe() {
            Ok(LoadOutcome::Loaded(v)) => {
                self.vault = v;
                self.loaded = true;
                self.locked = false;
                self.unlock_error = None;
            }
            Ok(_) => self.unlock_error = Some("Could not open vault".into()),
            Err(e) => {
                self.store.master_password = None;
                self.unlock_error = Some(format!("{e:#}"));
            }
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        egui::Panel::bottom("status")
            .exact_size(26.0)
            .frame(
                egui::Frame::new()
                    .fill(crate::ui::chrome_fill(pal.sidebar))
                    .inner_margin(egui::Margin::symmetric(12, 4)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let live = self
                        .tabs
                        .iter()
                        .map(|t| match t {
                            Tab::Terminal(tt) => {
                                tt.panes.iter().filter(|p| p.session.is_live()).count()
                            }
                            _ => 0,
                        })
                        .sum::<usize>();
                    let dot = if live > 0 { pal.success } else { pal.text_dim };
                    ui.label(RichText::new("●").color(dot).small());
                    ui.label(
                        RichText::new(format!("{live} active sessions"))
                            .small()
                            .color(pal.text_dim),
                    );
                    ui.separator();
                    let fw = self.forwards.running_count();
                    ui.label(
                        RichText::new(format!("\u{E0A0} {fw} tunnels"))
                            .small()
                            .color(pal.text_dim),
                    );
                    ui.separator();
                    let running = self
                        .transfers
                        .lock()
                        .iter()
                        .filter(|t| *t.status.lock() == crate::sftp::TransferStatus::Running)
                        .count();
                    if ui
                        .add(
                            egui::Label::new(
                                RichText::new(format!("\u{E098} {running} transfers"))
                                    .small()
                                    .color(pal.text_dim),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_text("Show transfer queue")
                        .clicked()
                    {
                        self.show_transfers = !self.show_transfers;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("⌘K  Command palette")
                                .small()
                                .color(pal.text_dim),
                        );
                        if !self.sync_status.is_empty() {
                            ui.separator();
                            ui.label(
                                RichText::new(format!("\u{E1AA} {}", self.sync_status))
                                    .small()
                                    .color(pal.text_dim),
                            );
                        }
                        if self.store.master_password.is_some() {
                            ui.separator();
                            ui.label(
                                RichText::new("\u{E308} Vault encrypted with master password")
                                    .small()
                                    .color(pal.text_dim),
                            );
                        }
                    });
                });
            });
    }

    pub fn term_view_config_parts(
        &self,
        target: &ResolvedTarget,
    ) -> (themes::TermScheme, egui::FontId) {
        let s = &self.vault.settings;
        let scheme =
            themes::scheme_by_name(target.color_scheme.as_deref().unwrap_or(&s.terminal_scheme));
        let size = target.font_size.unwrap_or(s.font_size);
        (
            scheme,
            egui::FontId::new(size, crate::fonts::terminal_family()),
        )
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some(qa) = self.qa.as_mut() {
            raw.events.append(&mut qa.keys);
        }
        #[cfg(target_os = "macos")]
        while let Ok(event) = muda::MenuEvent::receiver().try_recv() {
            let action = event.id.0;
            if matches!(action.as_str(), "copy" | "cut" | "paste" | "select-all") {
                self.clipboard_actions.push(action);
            } else {
                self.menu_actions.push(action);
            }
        }
        for action in std::mem::take(&mut self.clipboard_actions) {
            if let Some(id) = self.edit_focus {
                ctx.memory_mut(|m| m.request_focus(id));
            }
            let paste = if action == "paste" {
                arboard::Clipboard::new()
                    .and_then(|mut c| c.get_text())
                    .ok()
            } else {
                None
            };
            if let Some(event) = crate::ui::menus::clipboard_event(&action, paste) {
                if !raw.events.contains(&event) {
                    raw.events.push(event);
                }
            }
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.ensure_style(&ctx);
        self.updater.poll();
        self.application_menus(ui);
        frame.set_traffic_lights_position(&ctx, egui::Rangef::new(0.0, 52.0), 16.0);
        if self.host_test.as_ref().is_some_and(|test| {
            self.locked || !matches!(&self.editor, Some(Editor::Host(h, _)) if **h == test.host)
        }) {
            self.cancel_host_test();
        }
        self.qa_tick(&ctx);
        self.process_events(&ctx);
        while let Some(link) = self.handoffs.as_ref().and_then(|rx| rx.try_recv().ok()) {
            if let Some(link) = link {
                self.open_quick_connect(&link);
            }
        }
        if ctx.input(|i| !i.events.is_empty() || i.pointer.is_moving()) {
            self.last_activity = Instant::now();
        }
        let lock_after = self.vault.settings.lock_after_minutes;
        if lock_after > 0
            && self.store.master_password.is_some()
            && self.last_activity.elapsed().as_secs() > lock_after as u64 * 60
        {
            self.locked = true;
        }
        if self.locked {
            self.lock_screen(ui);
            return;
        }
        if !self.update_check_started && self.qa.is_none() && self.loaded {
            self.update_check_started = true;
            if self.vault.settings.check_updates_on_startup {
                self.updater.check_automatically(self.rt.handle(), &ctx);
            }
        }
        self.global_shortcuts(&ctx);
        self.snippet_hotkeys(&ctx);
        self.poll_edit_watches();

        self.tab_bar(ui);
        self.status_bar(ui);
        if self.show_transfers {
            self.transfers_drawer(ui);
        }
        match self.active_tab {
            None => self.vault_home(ui),
            Some(id) => {
                if let Some(idx) = self.tabs.iter().position(|t| t.id() == id) {
                    let is_term = matches!(self.tabs[idx], Tab::Terminal(_));
                    if is_term {
                        self.terminal_tab(ui, idx);
                    } else {
                        self.sftp_tab(ui, idx);
                    }
                } else {
                    self.active_tab = None;
                }
            }
        }
        self.draw_update_notice(&ctx);
        self.draw_prompts(&ctx);
        self.draw_dialog(&ctx);
        self.draw_palette(&ctx);
        self.draw_toasts(&ctx);
        if ctx.egui_wants_keyboard_input() {
            self.edit_focus = ctx.memory(|m| m.focused());
        }
        self.save_if_needed();
        // Keep polling background work.
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.loaded {
            let _ = self.store.save(&self.vault);
        }
        for t in &self.tabs {
            if let Tab::Terminal(tt) = t {
                for p in &tt.panes {
                    p.session.send_input(crate::term::Input::Close);
                }
            }
        }
    }
}
