//! Bounded, timestamped snapshots from a selection of sources on one SSH host.
use std::{
    collections::BTreeSet,
    io::{BufWriter, Write},
    path::Path,
    sync::Arc,
};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;

use crate::{
    conn::ssh::SshClient,
    model::{LogFilterPreset, LogSearch, Settings},
};

use super::{
    Item, LIST_TIMEOUT, LOG_LINES, inventory::build_logs_command, last_line, read_logs, strip_ansi,
    sudo_prefix,
};

pub const MAX_SOURCES: usize = 8;

#[derive(Clone)]
pub enum Source {
    Container(Item),
    Build { reference: String, name: String },
}

impl Source {
    pub fn key(&self) -> String {
        match self {
            Self::Container(item) => item.key(),
            Self::Build { reference, .. } => format!("Build/{reference}"),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Container(item) if item.runtime == super::Runtime::Kubernetes => {
                format!("{} / {}/{}", item.runtime.label(), item.scope, item.name)
            }
            Self::Container(item) => format!("{} / {}", item.runtime.label(), item.name),
            Self::Build { reference, name } => {
                let id = reference.rsplit('/').next().unwrap_or(reference);
                if name.trim().is_empty() {
                    format!("Build / {id}")
                } else {
                    let short_id: String = id.chars().take(8).collect();
                    format!("Build / {name} ({short_id})")
                }
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Row {
    pub time: Option<DateTime<Utc>>,
    pub source: Arc<str>,
    pub message: String,
}

#[derive(Default)]
pub enum SearchMatcher {
    #[default]
    All,
    Pattern(Regex),
    Invalid(String),
}

impl SearchMatcher {
    fn compile(search: &LogSearch) -> Self {
        let query = if search.regex {
            search.query.as_str()
        } else {
            search.query.trim()
        };
        if query.is_empty() {
            return Self::All;
        }
        let pattern = if search.regex {
            query.to_owned()
        } else {
            regex::escape(query)
        };
        match RegexBuilder::new(&pattern)
            .case_insensitive(!search.case_sensitive)
            .size_limit(1024 * 1024)
            .build()
        {
            Ok(regex) => Self::Pattern(regex),
            Err(error) => {
                let detail = error.to_string();
                Self::Invalid(
                    detail
                        .lines()
                        .last()
                        .unwrap_or(&detail)
                        .trim_start_matches("error: ")
                        .to_owned(),
                )
            }
        }
    }

    pub fn matches(&self, row: &Row) -> bool {
        match self {
            Self::All => true,
            Self::Pattern(regex) => regex.is_match(&row.source) || regex.is_match(&row.message),
            Self::Invalid(_) => false,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Invalid(message) => Some(message),
            _ => None,
        }
    }
}

pub enum PresetEdit {
    Save(LogFilterPreset),
    Remove(String),
}

pub fn edit_presets(settings: &mut Settings, host: uuid::Uuid, edit: PresetEdit) -> bool {
    match edit {
        PresetEdit::Save(preset) => {
            let presets = settings.log_filter_presets.entry(host).or_default();
            if let Some(existing) = presets.iter_mut().find(|saved| saved.name == preset.name) {
                if *existing == preset {
                    return false;
                }
                *existing = preset;
            } else {
                presets.push(preset);
                presets.sort_by(|left, right| left.name.cmp(&right.name));
            }
            true
        }
        PresetEdit::Remove(name) => {
            let Some(presets) = settings.log_filter_presets.get_mut(&host) else {
                return false;
            };
            let previous = presets.len();
            presets.retain(|preset| preset.name != name);
            let changed = previous != presets.len();
            if presets.is_empty() {
                settings.log_filter_presets.remove(&host);
            }
            changed
        }
    }
}

/// Replace the file chosen in the save dialog only after writing a complete JSONL snapshot.
pub fn export(path: &Path, rows: &[Row]) -> Result<()> {
    #[derive(Serialize)]
    struct Record<'a> {
        timestamp: Option<DateTime<Utc>>,
        source: &'a str,
        message: &'a str,
    }

    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut staged =
        tempfile::NamedTempFile::new_in(parent).context("Could not create the export file")?;
    {
        let mut writer = BufWriter::new(staged.as_file_mut());
        for row in rows {
            serde_json::to_writer(
                &mut writer,
                &Record {
                    timestamp: row.time,
                    source: &row.source,
                    message: &row.message,
                },
            )?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
    }
    staged
        .persist(path)
        .map_err(|error| error.error)
        .context("Could not save the export file")?;
    Ok(())
}

#[derive(Default)]
pub struct Snapshot {
    pub rows: Vec<Row>,
    pub errors: Vec<(String, String)>,
}

#[derive(Default)]
pub struct View {
    pub selected: BTreeSet<String>,
    pub search: LogSearch,
    search_at_compile: LogSearch,
    matcher: SearchMatcher,
    pub preset_name: String,
    pub refresh_requested: bool,
    pub source_query: String,
    pub snapshot: Option<Snapshot>,
    pub pending: bool,
    pub auto_refresh: bool,
    pub last_request: Option<f64>,
    pub revision: u64,
    pub selected_row: Option<usize>,
}

impl View {
    pub fn update_search(&mut self) {
        if self.search_at_compile != self.search {
            self.matcher = SearchMatcher::compile(&self.search);
            self.search_at_compile = self.search.clone();
        }
    }

    pub fn matcher(&self) -> &SearchMatcher {
        &self.matcher
    }

    pub fn apply_preset(&mut self, preset: &LogFilterPreset) {
        self.preset_name.clone_from(&preset.name);
        self.search.clone_from(&preset.search);
        self.update_search();
        let sources = preset.sources.iter().take(MAX_SOURCES).cloned().collect();
        if self.selected != sources {
            self.selected = sources;
            self.invalidate_snapshot();
        }
        self.refresh_requested = self.snapshot.is_none() && !self.selected.is_empty();
    }

    pub fn change_source(&mut self, key: String, selected: bool) {
        let changed = if selected {
            self.selected.len() < MAX_SOURCES && self.selected.insert(key)
        } else {
            self.selected.remove(&key)
        };
        if changed {
            self.invalidate_snapshot();
        }
    }

    pub fn clear_sources(&mut self) {
        self.selected.clear();
        self.invalidate_snapshot();
    }

    fn invalidate_snapshot(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.snapshot = None;
        self.selected_row = None;
        self.last_request = None;
        self.refresh_requested = false;
    }

    pub fn apply(&mut self, revision: u64, snapshot: Snapshot) {
        self.pending = false;
        if self.revision == revision {
            self.selected_row = self
                .snapshot
                .as_ref()
                .and_then(|previous| previous.rows.get(self.selected_row?))
                .and_then(|selected| snapshot.rows.iter().position(|row| row == selected));
            self.snapshot = Some(snapshot);
        }
    }
}

type Line = (Option<DateTime<Utc>>, String);

fn parse_container(text: &str) -> Vec<Line> {
    text.lines()
        .map(|line| {
            if let Some((prefix, message)) = line.split_once(' ')
                && let Ok(time) = DateTime::parse_from_rfc3339(prefix)
            {
                return (Some(time.to_utc()), message.to_owned());
            }
            (None, line.to_owned())
        })
        .collect()
}

#[derive(Deserialize)]
struct StatusUpdate {
    logs: Option<Vec<BuildChunk>>,
    vertexes: Option<Vec<BuildVertex>>,
}

#[derive(Deserialize)]
struct BuildChunk {
    timestamp: Option<DateTime<Utc>>,
    #[serde(default)]
    data: String,
}

#[derive(Deserialize)]
struct BuildVertex {
    #[serde(default)]
    name: String,
    completed: Option<DateTime<Utc>>,
    error: Option<String>,
}

fn parse_build(text: &str) -> Result<Vec<Line>> {
    let mut lines = Vec::new();
    let mut errors = BTreeSet::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        if !line.trim_start().starts_with('{') {
            // Buildx can emit a plain-text warning on its diagnostic stream.
            lines.push((None, strip_ansi(line)));
            continue;
        }
        let update: StatusUpdate =
            serde_json::from_str(line).context("Buildx returned an unreadable log event")?;
        for chunk in update.logs.into_iter().flatten() {
            let data = STANDARD
                .decode(chunk.data)
                .context("Buildx returned invalid log data")?;
            let text = strip_ansi(&String::from_utf8_lossy(&data));
            lines.extend(text.lines().map(|line| (chunk.timestamp, line.to_owned())));
        }
        for vertex in update.vertexes.into_iter().flatten() {
            if let Some(error) = vertex.error {
                let message = if vertex.name.is_empty() {
                    strip_ansi(&error)
                } else {
                    strip_ansi(&format!("{}: {error}", vertex.name))
                };
                if errors.insert((vertex.completed, message.clone())) {
                    lines.push((vertex.completed, message));
                }
            }
        }
    }
    Ok(lines)
}

async fn read_source(client: &SshClient, source: &Source, sudo: bool) -> Result<Vec<Line>> {
    match source {
        Source::Container(item) => Ok(parse_container(&read_logs(client, item, sudo, true).await?)),
        Source::Build { reference, .. } => {
            let action = build_logs_command(reference, true)?;
            let command = format!("{}docker {action}", sudo_prefix(sudo));
            let (text, code) =
                tokio::time::timeout(LIST_TIMEOUT, client.exec_limited(&command, 2 * 1024 * 1024))
                    .await
                    .context("Buildx log request timed out")??;
            if code != 0 {
                bail!("{}", last_line(&text));
            }
            parse_build(&text)
        }
    }
}

pub async fn load(client: &Arc<SshClient>, sources: Vec<Source>, sudo: bool) -> Snapshot {
    let mut tasks = JoinSet::new();
    for (index, source) in sources.into_iter().take(MAX_SOURCES).enumerate() {
        let client = Arc::clone(client);
        tasks.spawn(async move {
            let label = source.label();
            (index, label, read_source(&client, &source, sudo).await)
        });
    }
    let mut snapshot = Snapshot::default();
    let mut results = Vec::new();
    while let Some(task) = tasks.join_next().await {
        match task {
            Ok(batch) => results.push(batch),
            Err(error) => snapshot
                .errors
                .push(("Log source".into(), error.to_string())),
        }
    }
    results.sort_by_key(|(index, _, _)| *index);
    for (_, label, result) in results {
        match result {
            Ok(mut lines) => {
                lines.sort_by_key(|(time, _)| (time.is_none(), *time));
                // Prefer the latest timestamped entries and keep untimed diagnostics visible.
                let start = lines.len().saturating_sub(LOG_LINES as usize);
                let source: Arc<str> = label.into();
                snapshot
                    .rows
                    .extend(lines.into_iter().skip(start).map(|(time, message)| Row {
                        time,
                        source: Arc::clone(&source),
                        message,
                    }));
            }
            Err(error) => snapshot.errors.push((label, error.to_string())),
        }
    }
    snapshot
        .rows
        .sort_by_key(|row| (row.time.is_none(), row.time));
    snapshot
}
