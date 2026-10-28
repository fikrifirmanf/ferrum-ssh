//! Vault data model: hosts, groups, identities, keys, port forwarding rules,
//! snippets, known hosts, history and settings.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub fn now_ts() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Protocol {
    #[default]
    Ssh,
    Mosh,
    Telnet,
    Serial,
    Local,
}

impl Protocol {
    pub fn label(&self) -> &'static str {
        match self {
            Protocol::Ssh => "SSH",
            Protocol::Mosh => "Mosh",
            Protocol::Telnet => "Telnet",
            Protocol::Serial => "Serial",
            Protocol::Local => "Local",
        }
    }
    pub fn default_port(&self) -> u16 {
        match self {
            Protocol::Telnet => 23,
            _ => 22,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BackspaceMode {
    #[default]
    Del,
    CtrlH,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ProxyKind {
    #[default]
    Socks5,
    Http,
    Command,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ProxyConfig {
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// For `ProxyKind::Command` (OpenSSH ProxyCommand style, `%h` and `%p` are substituted).
    pub command: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Parity {
    #[default]
    None,
    Odd,
    Even,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FlowControl {
    #[default]
    None,
    Software,
    Hardware,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SerialConfig {
    pub device: String,
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: Parity,
    pub flow_control: FlowControl,
    pub local_echo: bool,
    pub line_ending: String,
}

impl Default for SerialConfig {
    fn default() -> Self {
        Self {
            device: String::new(),
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: Parity::None,
            flow_control: FlowControl::None,
            local_echo: false,
            line_ending: "\r".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoshConfig {
    pub server_command: String,
}

impl Default for MoshConfig {
    fn default() -> Self {
        Self {
            server_command: "mosh-server new -s -c 256 -l LANG=en_US.UTF-8".into(),
        }
    }
}

/// Settings shared by hosts and groups. Hosts inherit unset values from their group chain.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ConnSettings {
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub key_id: Option<Uuid>,
    pub identity_id: Option<Uuid>,
    pub use_agent: Option<bool>,
    pub agent_forwarding: Option<bool>,
    pub startup_snippet_id: Option<Uuid>,
    pub env: Vec<(String, String)>,
    pub keepalive_secs: Option<u32>,
    /// Refuse hosts whose key isn't already trusted instead of asking.
    pub strict_host_key_check: Option<bool>,
    pub connect_timeout_secs: Option<u32>,
    pub charset: Option<String>,
    pub backspace: Option<BackspaceMode>,
    /// Ordered jump hosts (ProxyJump). Each is a host id from the vault.
    pub host_chain: Vec<Uuid>,
    pub proxy: Option<ProxyConfig>,
    pub color_scheme: Option<String>,
    pub font_size: Option<f32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Host {
    pub id: Uuid,
    pub label: String,
    pub address: String,
    pub protocol: Protocol,
    pub group_id: Option<Uuid>,
    pub tags: Vec<String>,
    pub settings: ConnSettings,
    pub mosh: MoshConfig,
    pub serial: SerialConfig,
    pub notes: String,
    pub os: Option<String>,
    pub pinned: bool,
    pub last_connected: Option<i64>,
    pub updated: i64,
}

impl Default for Host {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            label: String::new(),
            address: String::new(),
            protocol: Protocol::Ssh,
            group_id: None,
            tags: vec![],
            settings: ConnSettings::default(),
            mosh: MoshConfig::default(),
            serial: SerialConfig::default(),
            notes: String::new(),
            os: None,
            pinned: false,
            last_connected: None,
            updated: now_ts(),
        }
    }
}

impl Host {
    pub fn display_name(&self) -> String {
        if self.label.trim().is_empty() {
            self.address.clone()
        } else {
            self.label.clone()
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Group {
    pub id: Uuid,
    pub name: String,
    pub parent_id: Option<Uuid>,
    pub settings: ConnSettings,
    pub updated: i64,
}

impl Default for Group {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::new(),
            parent_id: None,
            settings: ConnSettings::default(),
            updated: now_ts(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Identity {
    pub id: Uuid,
    pub label: String,
    pub username: String,
    pub password: String,
    pub key_id: Option<Uuid>,
    pub updated: i64,
}

impl Default for Identity {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            label: String::new(),
            username: String::new(),
            password: String::new(),
            key_id: None,
            updated: now_ts(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct SshKey {
    pub id: Uuid,
    pub label: String,
    /// OpenSSH private key (may itself be passphrase-encrypted).
    pub private_key: String,
    pub public_key: String,
    pub passphrase: String,
    /// Optional OpenSSH certificate (`*-cert.pub` contents).
    pub certificate: String,
    pub key_type: String,
    pub updated: i64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ForwardKind {
    #[default]
    Local,
    Remote,
    Dynamic,
}

impl ForwardKind {
    pub fn label(&self) -> &'static str {
        match self {
            ForwardKind::Local => "Local",
            ForwardKind::Remote => "Remote",
            ForwardKind::Dynamic => "Dynamic (SOCKS)",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PortForward {
    pub id: Uuid,
    pub label: String,
    pub kind: ForwardKind,
    pub host_id: Option<Uuid>,
    pub bind_address: String,
    pub bind_port: u16,
    pub dest_host: String,
    pub dest_port: u16,
    /// Start automatically whenever a terminal to `host_id` is opened.
    pub auto_start: bool,
    pub updated: i64,
}

impl Default for PortForward {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            label: String::new(),
            kind: ForwardKind::Local,
            host_id: None,
            bind_address: "127.0.0.1".into(),
            bind_port: 8080,
            dest_host: "127.0.0.1".into(),
            dest_port: 80,
            auto_start: false,
            updated: now_ts(),
        }
    }
}

/// One tab of a saved workspace.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WorkspaceTab {
    /// One entry per pane; `None` is a local shell.
    pub hosts: Vec<Option<Uuid>>,
    /// 0 = side by side, 1 = stacked, 2 = grid.
    pub layout: u8,
    pub sftp: bool,
    pub title: Option<String>,
}

/// A named set of tabs that can be reopened together.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Workspace {
    pub id: Uuid,
    pub name: String,
    pub tabs: Vec<WorkspaceTab>,
    pub updated: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Snippet {
    pub id: Uuid,
    pub name: String,
    pub script: String,
    pub tags: Vec<String>,
    pub description: String,
    pub updated: i64,
    /// Folder the snippet is filed under ("" = top level).
    #[serde(default)]
    pub package: String,
    /// Global shortcut that runs it in the active terminal, e.g. "Cmd+Alt+1".
    #[serde(default)]
    pub hotkey: String,
}

impl Default for Snippet {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::new(),
            script: String::new(),
            tags: vec![],
            description: String::new(),
            updated: now_ts(),
            package: String::new(),
            hotkey: String::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KnownHost {
    pub id: Uuid,
    pub host: String,
    pub port: u16,
    pub key_type: String,
    /// Public key in OpenSSH single-line format.
    pub public_key: String,
    pub fingerprint: String,
    pub added: i64,
    pub updated: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HistoryEntry {
    pub id: Uuid,
    pub command: String,
    pub host_label: String,
    pub timestamp: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ConnectionLog {
    pub id: Uuid,
    pub host_label: String,
    pub address: String,
    pub protocol: Protocol,
    pub started: i64,
    pub ended: Option<i64>,
    pub status: String,
    pub log_file: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KeywordRule {
    pub name: String,
    pub pattern: String,
    pub color: [u8; 3],
    pub enabled: bool,
}

pub fn default_keyword_rules() -> Vec<KeywordRule> {
    let r = |name: &str, pattern: &str, color: [u8; 3]| KeywordRule {
        name: name.into(),
        pattern: pattern.into(),
        color,
        enabled: true,
    };
    vec![
        r(
            "Error",
            r"(?i)\b(error|errors|err|fail|failed|failure|fatal|denied|refused|critical|panic|exception|unreachable|timeout)\b",
            [255, 92, 87],
        ),
        r(
            "Warning",
            r"(?i)\b(warn|warning|warnings|deprecated|caution)\b",
            [243, 199, 72],
        ),
        r(
            "Success",
            r"(?i)\b(ok|success|successful|succeeded|done|passed|completed|active|running|enabled)\b",
            [80, 220, 120],
        ),
        r("Info", r"(?i)\b(info|notice|note)\b", [88, 166, 255]),
        r("Debug", r"(?i)\b(debug|trace|verbose)\b", [150, 150, 170]),
        r(
            "IP address",
            r"\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)(?::\d{1,5})?\b",
            [198, 120, 221],
        ),
        r(
            "URL",
            r#"\b(?:https?|ftp|ssh)://[^\s<>"']+"#,
            [86, 214, 214],
        ),
    ]
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum UiTheme {
    #[default]
    Dark,
    Light,
    Midnight,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CursorShape {
    #[default]
    Block,
    Beam,
    Underline,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AiProvider {
    #[default]
    Anthropic,
    OpenAiCompatible,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub check_updates_on_startup: bool,
    pub ui_theme: UiTheme,
    pub accent: [u8; 3],
    pub terminal_scheme: String,
    pub font_size: f32,
    pub custom_font_path: String,
    pub line_height: f32,
    pub cursor_shape: CursorShape,
    pub cursor_blink: bool,
    pub scrollback: usize,
    pub bell_sound: bool,
    pub copy_on_select: bool,
    pub right_click_paste: bool,
    pub option_is_meta: bool,
    pub brighten_bold: bool,
    pub keyword_highlighting: bool,
    pub keyword_rules: Vec<KeywordRule>,
    pub autocomplete: bool,
    pub auto_reconnect: bool,
    pub detect_os: bool,
    pub save_quick_connect: bool,
    pub confirm_close_tab: bool,
    pub sftp_show_hidden: bool,
    pub sftp_confirm_overwrite: bool,
    pub local_shell: String,
    pub session_logging: bool,
    pub log_dir: String,
    pub default_keepalive_secs: u32,
    pub term_type: String,
    pub sync_folder: String,
    pub auto_sync: bool,
    pub lock_after_minutes: u32,
    pub ai_provider: AiProvider,
    pub ai_endpoint: String,
    pub ai_api_key: String,
    pub ai_model: String,
    pub mosh_client_path: String,
    pub external_editor: String,
    pub do_token: String,
    pub aws_access_key: String,
    pub aws_secret_key: String,
    pub aws_regions: String,
    pub azure_tenant: String,
    pub azure_client_id: String,
    pub azure_client_secret: String,
    pub azure_subscription: String,
    pub hosts_grid_view: bool,
    pub hosts_compact: bool,
    /// Probe saved hosts so the vault shows whether they answer and how fast.
    pub show_host_status: bool,
    pub sidebar_collapsed: bool,
    /// File extension (no dot) -> command used to open remote files, e.g. ("json", "code").
    pub file_associations: Vec<(String, String)>,
}

impl Settings {
    /// Command used to open `filename`: a matching file-type association, else the
    /// general external editor ("" means the system default).
    pub fn editor_for(&self, filename: &str) -> String {
        let name = filename.rsplit('/').next().unwrap_or(filename);
        if let Some((_, ext)) = name.rsplit_once('.') {
            let ext = ext.to_ascii_lowercase();
            if let Some((_, cmd)) = self.file_associations.iter().find(|(e, c)| {
                e.trim().trim_start_matches('.').to_ascii_lowercase() == ext && !c.trim().is_empty()
            }) {
                return cmd.clone();
            }
        }
        self.external_editor.clone()
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            check_updates_on_startup: true,
            ui_theme: UiTheme::Dark,
            accent: crate::themes::DEFAULT_ACCENT,
            terminal_scheme: "Ferrum Night".into(),
            font_size: 14.0,
            custom_font_path: String::new(),
            line_height: 1.2,
            cursor_shape: CursorShape::Block,
            cursor_blink: true,
            scrollback: 10_000,
            bell_sound: false,
            copy_on_select: false,
            right_click_paste: true,
            option_is_meta: true,
            brighten_bold: true,
            keyword_highlighting: true,
            keyword_rules: default_keyword_rules(),
            autocomplete: true,
            auto_reconnect: true,
            detect_os: true,
            save_quick_connect: false,
            confirm_close_tab: true,
            sftp_show_hidden: false,
            sftp_confirm_overwrite: true,
            local_shell: String::new(),
            session_logging: false,
            log_dir: String::new(),
            default_keepalive_secs: 30,
            term_type: "xterm-256color".into(),
            sync_folder: String::new(),
            auto_sync: false,
            lock_after_minutes: 0,
            ai_provider: AiProvider::Anthropic,
            ai_endpoint: "https://api.anthropic.com/v1/messages".into(),
            ai_api_key: String::new(),
            ai_model: "claude-sonnet-5-5".into(),
            mosh_client_path: "mosh-client".into(),
            external_editor: String::new(),
            do_token: String::new(),
            aws_access_key: String::new(),
            aws_secret_key: String::new(),
            aws_regions: "us-east-1".into(),
            azure_tenant: String::new(),
            azure_client_id: String::new(),
            azure_client_secret: String::new(),
            azure_subscription: String::new(),
            hosts_grid_view: false,
            hosts_compact: false,
            show_host_status: true,
            sidebar_collapsed: false,
            file_associations: vec![],
        }
    }
}

/// Everything stored (encrypted) on disk.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Vault {
    pub hosts: Vec<Host>,
    pub groups: Vec<Group>,
    pub identities: Vec<Identity>,
    pub keys: Vec<SshKey>,
    pub forwards: Vec<PortForward>,
    pub snippets: Vec<Snippet>,
    pub workspaces: Vec<Workspace>,
    pub known_hosts: Vec<KnownHost>,
    pub history: Vec<HistoryEntry>,
    pub connection_logs: Vec<ConnectionLog>,
    pub settings: Settings,
    /// Deleted entity ids with deletion timestamps (for sync merges).
    pub tombstones: HashMap<Uuid, i64>,
    pub settings_updated: i64,
}

/// Fully resolved connection parameters after applying group inheritance and identities.
#[derive(Clone, Debug)]
pub struct ResolvedTarget {
    pub label: String,
    pub host_id: Option<Uuid>,
    pub address: String,
    pub port: u16,
    pub protocol: Protocol,
    pub username: String,
    pub password: Option<String>,
    pub key: Option<SshKey>,
    pub use_agent: bool,
    pub agent_forwarding: bool,
    pub startup_script: Option<String>,
    pub env: Vec<(String, String)>,
    pub keepalive_secs: u32,
    pub strict_host_key_check: bool,
    pub connect_timeout_secs: u32,
    pub charset: String,
    pub backspace: BackspaceMode,
    pub chain: Vec<ResolvedTarget>,
    pub proxy: Option<ProxyConfig>,
    pub mosh: MoshConfig,
    pub serial: SerialConfig,
    pub color_scheme: Option<String>,
    pub font_size: Option<f32>,
    pub term_type: String,
}

impl Vault {
    pub fn host(&self, id: Uuid) -> Option<&Host> {
        self.hosts.iter().find(|h| h.id == id)
    }
    pub fn group(&self, id: Uuid) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }
    pub fn key(&self, id: Uuid) -> Option<&SshKey> {
        self.keys.iter().find(|k| k.id == id)
    }
    pub fn identity(&self, id: Uuid) -> Option<&Identity> {
        self.identities.iter().find(|k| k.id == id)
    }
    pub fn snippet(&self, id: Uuid) -> Option<&Snippet> {
        self.snippets.iter().find(|k| k.id == id)
    }

    /// Group ancestry for `group_id`, nearest first.
    pub fn group_chain(&self, group_id: Option<Uuid>) -> Vec<&Group> {
        let mut out = vec![];
        let mut cur = group_id;
        while let Some(id) = cur {
            if out.len() > 32 {
                break;
            }
            match self.group(id) {
                Some(g) => {
                    out.push(g);
                    cur = g.parent_id;
                }
                None => break,
            }
        }
        out
    }

    pub fn group_path(&self, group_id: Option<Uuid>) -> String {
        let mut names: Vec<String> = self
            .group_chain(group_id)
            .iter()
            .map(|g| g.name.clone())
            .collect();
        names.reverse();
        names.join(" / ")
    }

    pub fn resolve(&self, host: &Host) -> ResolvedTarget {
        self.resolve_depth(host, 0)
    }

    fn resolve_depth(&self, host: &Host, depth: usize) -> ResolvedTarget {
        // Merge settings: host first, then group chain.
        let mut layers: Vec<&ConnSettings> = vec![&host.settings];
        for g in self.group_chain(host.group_id) {
            layers.push(&g.settings);
        }
        macro_rules! pick {
            ($f:ident) => {
                layers.iter().find_map(|l| l.$f.clone())
            };
        }
        let identity = pick!(identity_id).and_then(|id| self.identity(id).cloned());
        let mut username = pick!(username).filter(|s| !s.is_empty());
        let mut password = pick!(password).filter(|s| !s.is_empty());
        let mut key_id = pick!(key_id);
        if let Some(ident) = &identity {
            if username.is_none() && !ident.username.is_empty() {
                username = Some(ident.username.clone());
            }
            if password.is_none() && !ident.password.is_empty() {
                password = Some(ident.password.clone());
            }
            if key_id.is_none() {
                key_id = ident.key_id;
            }
        }
        let env = layers
            .iter()
            .find(|l| !l.env.is_empty())
            .map(|l| l.env.clone())
            .unwrap_or_default();
        let chain_ids = layers
            .iter()
            .find(|l| !l.host_chain.is_empty())
            .map(|l| l.host_chain.clone())
            .unwrap_or_default();
        let chain = if depth < 8 {
            chain_ids
                .iter()
                .filter_map(|id| self.host(*id))
                .filter(|h| h.id != host.id)
                .map(|h| {
                    let mut r = self.resolve_depth(h, depth + 1);
                    r.chain.clear();
                    r
                })
                .collect()
        } else {
            vec![]
        };
        let startup_script = pick!(startup_snippet_id)
            .and_then(|id| self.snippet(id))
            .map(|s| s.script.clone());
        ResolvedTarget {
            label: host.display_name(),
            host_id: Some(host.id),
            address: host.address.clone(),
            port: pick!(port).unwrap_or(host.protocol.default_port()),
            protocol: host.protocol,
            username: username.unwrap_or_default(),
            password,
            key: key_id.and_then(|id| self.key(id).cloned()),
            use_agent: pick!(use_agent).unwrap_or(true),
            agent_forwarding: pick!(agent_forwarding).unwrap_or(false),
            startup_script,
            env,
            keepalive_secs: pick!(keepalive_secs).unwrap_or(self.settings.default_keepalive_secs),
            strict_host_key_check: pick!(strict_host_key_check).unwrap_or(false),
            connect_timeout_secs: pick!(connect_timeout_secs).unwrap_or(20),
            charset: pick!(charset).unwrap_or_else(|| "UTF-8".into()),
            backspace: pick!(backspace).unwrap_or_default(),
            chain,
            proxy: pick!(proxy),
            mosh: host.mosh.clone(),
            serial: host.serial.clone(),
            color_scheme: pick!(color_scheme),
            font_size: pick!(font_size),
            term_type: self.settings.term_type.clone(),
        }
    }

    pub fn all_tags(&self) -> Vec<String> {
        let mut t: Vec<String> = self.hosts.iter().flat_map(|h| h.tags.clone()).collect();
        t.sort();
        t.dedup();
        t
    }

    pub fn delete(&mut self, id: Uuid) {
        self.tombstones.insert(id, now_ts());
        self.hosts.retain(|x| x.id != id);
        self.groups.retain(|x| x.id != id);
        self.identities.retain(|x| x.id != id);
        self.keys.retain(|x| x.id != id);
        self.forwards.retain(|x| x.id != id);
        self.snippets.retain(|x| x.id != id);
        self.workspaces.retain(|x| x.id != id);
        self.known_hosts.retain(|x| x.id != id);
        // Ungroup children of a deleted group.
        for h in self.hosts.iter_mut() {
            if h.group_id == Some(id) {
                h.group_id = None;
                h.updated = now_ts();
            }
        }
        for g in self.groups.iter_mut() {
            if g.parent_id == Some(id) {
                g.parent_id = None;
                g.updated = now_ts();
            }
        }
    }

    pub fn record_history(&mut self, command: &str, host_label: &str) {
        let command = command.trim();
        if command.is_empty() || command.len() > 2000 {
            return;
        }
        self.history.retain(|h| h.command != command);
        self.history.push(HistoryEntry {
            id: Uuid::new_v4(),
            command: command.to_string(),
            host_label: host_label.to_string(),
            timestamp: now_ts(),
        });
        if self.history.len() > 5000 {
            let excess = self.history.len() - 5000;
            self.history.drain(0..excess);
        }
    }
}

/// Parse a quick-connect string such as `user@host:2222` or `ssh://user@host:22`.
pub fn parse_quick_connect(s: &str) -> Option<(Option<String>, String, Option<u16>)> {
    let mut s = s.trim();
    for prefix in ["ssh://", "sftp://"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest;
        }
    }
    let s = s.trim_end_matches('/');
    if s.is_empty() || s.contains(char::is_whitespace) {
        return None;
    }
    let (user, hostport) = match s.rsplit_once('@') {
        Some((u, h)) => (Some(u.to_string()), h),
        None => (None, s),
    };
    // IPv6 in brackets
    if let Some(rest) = hostport.strip_prefix('[') {
        let (h, tail) = rest.split_once(']')?;
        let port = tail.strip_prefix(':').and_then(|p| p.parse().ok());
        return Some((user, h.to_string(), port));
    }
    match hostport.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => Some((user, h.to_string(), p.parse().ok())),
        _ => Some((user, hostport.to_string(), None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workbench_defaults_preserve_existing_appearance_preferences() {
        let settings:Settings=serde_json::from_str(r#"{"accent":[9,20,40],"hosts_grid_view":true,"terminal_scheme":"Nord","custom_font_path":"/tmp/mono.ttf"}"#).unwrap();
        assert_eq!(settings.accent, [9, 20, 40]);
        assert!(settings.hosts_grid_view);
        assert!(!settings.hosts_compact);
        assert_eq!(settings.terminal_scheme, "Nord");
        assert_eq!(settings.custom_font_path, "/tmp/mono.ttf");
        let defaults: Settings = serde_json::from_str("{}").unwrap();
        assert!(!defaults.hosts_grid_view);
        assert_eq!(defaults.accent, crate::themes::DEFAULT_ACCENT);
    }

    #[test]
    fn quick_connect_parsing() {
        assert_eq!(
            parse_quick_connect("root@10.0.0.1:2222"),
            Some((Some("root".into()), "10.0.0.1".into(), Some(2222)))
        );
        assert_eq!(
            parse_quick_connect("example.com"),
            Some((None, "example.com".into(), None))
        );
        assert_eq!(
            parse_quick_connect("ssh://a@[::1]:22"),
            Some((Some("a".into()), "::1".into(), Some(22)))
        );
        assert_eq!(parse_quick_connect("two words"), None);
    }

    #[test]
    fn group_inheritance_resolves_username_and_port() {
        let mut v = Vault::default();
        let mut g = Group::default();
        g.name = "prod".into();
        g.settings.username = Some("deploy".into());
        g.settings.port = Some(2200);
        let mut h = Host::default();
        h.address = "1.2.3.4".into();
        h.group_id = Some(g.id);
        v.groups.push(g);
        v.hosts.push(h.clone());
        let r = v.resolve(&h);
        assert_eq!(r.username, "deploy");
        assert_eq!(r.port, 2200);
        h.settings.username = Some("me".into());
        assert_eq!(v.resolve(&h).username, "me");
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn file_association_beats_general_editor() {
        let mut s = Settings::default();
        s.external_editor = "nano".into();
        s.file_associations = vec![
            (".JSON".into(), "code -w".into()),
            ("png".into(), "".into()),
        ];
        assert_eq!(s.editor_for("/etc/app/config.json"), "code -w");
        assert_eq!(s.editor_for("notes.txt"), "nano");
        assert_eq!(s.editor_for("pic.png"), "nano", "blank command falls back");
        assert_eq!(s.editor_for("/srv/Makefile"), "nano", "no extension");
    }
}

#[cfg(test)]
mod compat_tests {
    use super::*;

    /// Vaults saved before snippets had packages/hotkeys and before workspaces existed.
    #[test]
    fn old_vault_json_still_loads() {
        let json = r#"{
            "snippets": [{"id":"6c1f4a2e-3b5d-4e7f-8a9b-0c1d2e3f4a5b","name":"uptime","script":"uptime","tags":[],"description":"","updated":1}],
            "hosts": []
        }"#;
        let v: Vault = serde_json::from_str(json).expect("old vault must load");
        assert_eq!(v.snippets[0].package, "");
        assert_eq!(v.snippets[0].hotkey, "");
        assert!(v.workspaces.is_empty());
        assert!(v.settings.file_associations.is_empty());
    }

    #[test]
    fn host_settings_default_to_lenient_connect() {
        let v = Vault::default();
        let t = v.resolve(&Host {
            address: "h".into(),
            ..Default::default()
        });
        assert!(!t.strict_host_key_check);
        assert_eq!(t.connect_timeout_secs, 20);
    }

    #[test]
    fn group_strict_host_key_check_is_inherited() {
        let mut v = Vault::default();
        let mut g = Group {
            name: "prod".into(),
            ..Default::default()
        };
        g.settings.strict_host_key_check = Some(true);
        g.settings.connect_timeout_secs = Some(5);
        let h = Host {
            address: "h".into(),
            group_id: Some(g.id),
            ..Default::default()
        };
        v.groups.push(g);
        let t = v.resolve(&h);
        assert!(t.strict_host_key_check);
        assert_eq!(t.connect_timeout_secs, 5);
    }
}
