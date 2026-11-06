//! Read-only container details and bounded resource samples over SSH.
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

static DETAIL_SEQUENCE: AtomicU64 = AtomicU64::new(1);

use anyhow::{Result, anyhow, bail};
use serde_json::Value;

use super::{Item, LIST_TIMEOUT, Listing, Runtime, SshClient, shell_quote, sudo_prefix};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DetailTab {
    #[default]
    Logs,
    Stats,
    Inspect,
    Terminal,
    Ports,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stats {
    pub cpu: Option<f64>,
    pub memory_percent: Option<f64>,
    pub memory: String,
    pub network: String,
    pub disk: String,
    pub pids: String,
}

#[derive(Clone)]
pub struct Sample {
    pub at: Instant,
    pub stats: Stats,
}

pub struct Manager {
    pub open: bool,
    pub selected: Option<String>,
    pub focus_details: bool,
    pub tab: DetailTab,
    pub logs: Option<Result<String, String>>,
    pub inspect: Option<Result<String, String>>,
    pub ports: Option<Result<Vec<super::ports::PublishedPort>, String>>,
    pub port_mode: super::ports::OpenAs,
    pub tunnels: BTreeMap<String, super::ports::Tunnel>,
    pub detail_pending: Option<(u64, DetailTab)>,
    pub detail_sequence: u64,
    pub last_detail_poll: f64,
    pub logs_updated: Option<Instant>,
    pub pause_logs: bool,
    pub log_filter: String,
    pub stats: BTreeMap<String, VecDeque<Sample>>,
    pub stats_pending: bool,
    pub last_stats_poll: f64,
    pub stats_error: Option<String>,
    pub stats_updated: Option<Instant>,
}

impl Default for Manager {
    fn default() -> Self {
        Self {
            open: false,
            selected: None,
            focus_details: false,
            tab: DetailTab::Logs,
            logs: None,
            inspect: None,
            ports: None,
            port_mode: Default::default(),
            tunnels: BTreeMap::new(),
            detail_pending: None,
            detail_sequence: 0,
            last_detail_poll: f64::NEG_INFINITY,
            logs_updated: None,
            pause_logs: false,
            log_filter: String::new(),
            stats: BTreeMap::new(),
            stats_pending: false,
            last_stats_poll: f64::NEG_INFINITY,
            stats_error: None,
            stats_updated: None,
        }
    }
}

impl Manager {
    pub fn select(&mut self, key: String) {
        self.focus_details = true;
        if self.selected.as_ref() == Some(&key) {
            return;
        }
        self.selected = Some(key);
        self.logs = None;
        self.inspect = None;
        self.ports = None;
        self.logs_updated = None;
        self.pause_logs = false;
        self.log_filter.clear();
        self.invalidate_detail();
    }
    pub fn invalidate_detail(&mut self) {
        self.detail_sequence = DETAIL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        self.detail_pending = None;
        self.last_detail_poll = f64::NEG_INFINITY;
    }
    pub fn choose_tab(&mut self, tab: DetailTab) {
        if self.tab != tab {
            self.tab = tab;
            self.invalidate_detail();
        }
    }
    pub fn begin_detail(&mut self, now: f64) -> u64 {
        self.detail_sequence = DETAIL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        self.detail_pending = Some((self.detail_sequence, self.tab));
        self.last_detail_poll = now;
        self.detail_sequence
    }
    pub fn apply_detail(
        &mut self,
        key: &str,
        sequence: u64,
        tab: DetailTab,
        result: Result<String, String>,
    ) {
        if self.selected.as_deref() != Some(key) || self.detail_pending != Some((sequence, tab)) {
            return;
        }
        self.detail_pending = None;
        match tab {
            DetailTab::Logs => {
                if result.is_ok() {
                    self.logs_updated = Some(Instant::now());
                }
                self.logs = Some(result);
            }
            DetailTab::Inspect => self.inspect = Some(result),
            DetailTab::Ports => {
                self.ports =
                    Some(result.and_then(|text| {
                        super::ports::parse(&text).map_err(|error| error.to_string())
                    }))
            }
            _ => {}
        }
    }
    pub fn reconcile(&mut self, listing: &Listing) {
        self.tunnels.retain(|_, tunnel| {
            listing
                .items
                .iter()
                .any(|item| item.key() == tunnel.owner && item.is_running())
        });
        self.stats.retain(|key, _| {
            listing
                .items
                .iter()
                .any(|item| item.key() == *key && item.is_running())
        });
        if self
            .selected
            .as_ref()
            .is_some_and(|key| !listing.items.iter().any(|item| item.key() == *key))
        {
            self.selected = None;
            self.logs = None;
            self.inspect = None;
            self.invalidate_detail();
        }
    }
    pub fn apply_stats(&mut self, result: Result<BTreeMap<String, Stats>, String>) {
        self.stats_pending = false;
        match result {
            Err(error) => self.stats_error = Some(error),
            Ok(samples) => {
                let now = Instant::now();
                self.stats.retain(|key, _| samples.contains_key(key));
                for (key, stats) in samples {
                    let history = self.stats.entry(key).or_default();
                    if history.len() >= 60 {
                        history.pop_front();
                    }
                    history.push_back(Sample { at: now, stats });
                }
                self.stats_error = None;
                self.stats_updated = Some(now);
            }
        }
    }
}

pub fn stats_key(runtime: Runtime, id: &str) -> String {
    format!("{}/{id}", runtime.label())
}

fn percent(value: &Value, field: &str) -> Result<Option<f64>> {
    let text = value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Missing {field} in resource sample"))?
        .trim();
    if matches!(text, "--" | "N/A" | "") {
        return Ok(None);
    }
    let n: f64 = text.trim_end_matches('%').parse()?;
    if !n.is_finite() || n < 0.0 {
        bail!("Invalid {field} in resource sample");
    }
    Ok(Some(n))
}

pub fn parse_stats(runtime: Runtime, text: &str) -> Result<BTreeMap<String, Stats>> {
    let mut samples = BTreeMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        if samples.len() >= 2000 {
            bail!("Resource response exceeds 2,000 containers");
        }
        let value: Value = serde_json::from_str(line)?;
        let id = ["ID", "Container", "ContainerID"]
            .iter()
            .filter_map(|key| value.get(key).and_then(Value::as_str))
            .find(|id| !id.is_empty())
            .ok_or_else(|| anyhow!("Resource sample has no container ID"))?;
        if !id.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("Invalid container ID in resource sample");
        }
        let text = |field| match value.get(field) {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Number(number)) => number.to_string(),
            _ => "Unavailable".to_owned(),
        };
        samples.insert(
            stats_key(runtime, id),
            Stats {
                cpu: percent(&value, "CPUPerc")?,
                memory_percent: percent(&value, "MemPerc")?,
                memory: text("MemUsage"),
                network: text("NetIO"),
                disk: text("BlockIO"),
                pids: text("PIDs"),
            },
        );
    }
    Ok(samples)
}

pub async fn stats(
    client: &SshClient,
    listing: &Listing,
    sudo: bool,
) -> Result<BTreeMap<String, Stats>> {
    let mut out = BTreeMap::new();
    // At most two batched CLI calls, independent of the number of containers.
    // unslop-ignore N_PLUS_ONE_IN_LOOP -- Exactly two runtime batches; each request contains every running container ID.
    for runtime in [Runtime::Docker, Runtime::Podman] {
        let ids: Vec<&str> = listing
            .items
            .iter()
            .filter(|item| item.runtime == runtime && item.is_running() && item.valid().is_some())
            .map(|item| item.scope.as_str())
            .collect();
        if ids.is_empty() {
            continue;
        }
        if ids.len() > 2000 {
            bail!("Resource polling supports at most 2,000 containers per runtime");
        }
        let command = format!(
            "{}{} stats --no-stream --format {} {}",
            sudo_prefix(sudo),
            runtime.cli(),
            shell_quote(
                r#"{"ID":{{json .ID}},"CPUPerc":{{json .CPUPerc}},"MemPerc":{{json .MemPerc}},"MemUsage":{{json .MemUsage}},"NetIO":{{json .NetIO}},"BlockIO":{{json .BlockIO}},"PIDs":{{json .PIDs}}}"#
            ),
            ids.join(" ")
        );
        let (text, code) =
            tokio::time::timeout(LIST_TIMEOUT, client.exec_limited(&command, 2 * 1024 * 1024))
                .await
                .map_err(|_| anyhow!("Container resource request timed out"))??;
        if code != 0 {
            bail!("{}: {}", runtime.label(), super::last_line(&text));
        }
        let samples = parse_stats(runtime, &text)?;
        // Match returned full or abbreviated IDs back to the requested immutable IDs.
        for item in listing
            .items
            .iter()
            .filter(|item| item.runtime == runtime && item.is_running())
        {
            let prefix = stats_key(runtime, &item.scope);
            if let Some((_, sample)) = samples
                .iter()
                .find(|(key, _)| key.starts_with(&prefix) || prefix.starts_with(key.as_str()))
            {
                out.insert(item.key(), sample.clone());
            }
        }
    }
    Ok(out)
}

/// Inspect configuration without exposing environment values or arbitrary secret-bearing labels.
pub fn inspect_command(item: &Item, sudo: bool) -> Result<String> {
    item.valid()
        .ok_or_else(|| anyhow!("Invalid container identity"))?;
    if item.runtime == Runtime::Kubernetes {
        bail!("Inspect is available for Docker and Podman containers");
    }
    // Project only known fields; environment, command arguments and arbitrary labels stay remote.
    let template = r#"{"id":{{json .Id}},"name":{{json .Name}},"image":{{json .Config.Image}},"state":{{json .State.Status}},"created":{{json .Created}},"mounts":{{json .Mounts}},"networks":{{json .NetworkSettings.Networks}},"ports":{{json .NetworkSettings.Ports}},"restartPolicy":{{json .HostConfig.RestartPolicy}}}"#;
    Ok(format!(
        "{}{} inspect --format {} {}",
        sudo_prefix(sudo),
        item.runtime.cli(),
        shell_quote(template),
        item.scope
    ))
}

pub async fn inspect(client: &SshClient, item: &Item, sudo: bool) -> Result<String> {
    let command = inspect_command(item, sudo)?;
    let (text, code) =
        tokio::time::timeout(LIST_TIMEOUT, client.exec_limited(&command, 512 * 1024))
            .await
            .map_err(|_| anyhow!("Container inspect request timed out"))??;
    if code != 0 {
        bail!("{}", super::last_line(&text));
    }
    let value: Value = serde_json::from_str(&text)?;
    Ok(serde_json::to_string_pretty(&value)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> Item {
        super::super::parse(r#"@docker
{"ID":"abcdef123456","Names":"api","Image":"nginx:1.27","State":"running","Labels":"com.docker.compose.project=shop,com.docker.compose.service=api"}
"#).items.remove(0)
    }

    fn sample() -> Stats {
        Stats {
            cpu: Some(125.5),
            memory_percent: Some(12.5),
            memory: "32MiB / 256MiB".into(),
            network: "1kB / 2kB".into(),
            disk: "0B / 1kB".into(),
            pids: "8".into(),
        }
    }

    #[test]
    fn parses_compose_projects_from_docker_and_podman_labels() {
        let docker = item();
        assert_eq!(docker.project.as_deref(), Some("shop"));
        assert_eq!(docker.service.as_deref(), Some("api"));
        let podman = super::super::parse(r#"@podman
{"Id":"abcdef123456","Names":["api"],"State":"running","Labels":{"com.docker.compose.project":"store","com.docker.compose.service":"web"}}
"#).items.remove(0);
        assert_eq!(podman.project.as_deref(), Some("store"));
        assert_eq!(podman.service.as_deref(), Some("web"));
        assert_ne!(
            docker.key(),
            podman.key(),
            "runtimes must have separate identities"
        );
    }

    #[test]
    fn resource_samples_preserve_multicore_cpu_and_io() {
        let samples = parse_stats(Runtime::Docker, r#"{"ID":"abcdef123456","CPUPerc":"125.50%","MemPerc":"12.50%","MemUsage":"32MiB / 256MiB","NetIO":"1kB / 2kB","BlockIO":"0B / 1kB","PIDs":"8"}"#).unwrap();
        assert_eq!(samples.get("Docker/abcdef123456"), Some(&sample()));
        for cpu in ["NaN%", "-1%", "inf%", "unknown"] {
            let text = serde_json::json!({"ID":"abcdef123456", "CPUPerc": cpu, "MemPerc":"1%"})
                .to_string();
            assert!(parse_stats(Runtime::Docker, &text).is_err(), "{cpu}");
        }
        assert!(parse_stats(Runtime::Docker, "not json").is_err());
    }

    #[test]
    fn unavailable_metrics_stay_unknown_and_numeric_process_counts_are_preserved() {
        let samples = parse_stats(Runtime::Podman, r#"{"ID":"abcdef123456","CPUPerc":"--","MemPerc":"12.5%","MemUsage":"32MiB / 256MiB","PIDs":8}"#).unwrap();
        let sample = &samples["Podman/abcdef123456"];
        assert_eq!(sample.cpu, None);
        assert_eq!(sample.memory_percent, Some(12.5));
        assert_eq!(sample.pids, "8");
    }

    #[test]
    fn stale_replies_cannot_cross_selections_or_tabs_even_after_selecting_back() {
        let mut manager = Manager::default();
        manager.select("A".into());
        let old = manager.begin_detail(1.0);
        manager.select("B".into());
        manager.select("A".into());
        let current = manager.begin_detail(2.0);
        manager.apply_detail("A", old, DetailTab::Logs, Ok("old".into()));
        assert_eq!(manager.logs, None);
        assert_eq!(manager.detail_pending, Some((current, DetailTab::Logs)));
        manager.apply_detail("A", current, DetailTab::Logs, Ok("current".into()));
        assert_eq!(manager.logs, Some(Ok("current".into())));
        manager.choose_tab(DetailTab::Inspect);
        let inspect = manager.begin_detail(3.0);
        manager.choose_tab(DetailTab::Stats);
        manager.apply_detail("A", inspect, DetailTab::Inspect, Ok("wrong tab".into()));
        assert_eq!(manager.inspect, None);
    }

    #[test]
    fn history_is_bounded_and_removed_containers_release_their_state() {
        let item = item();
        let mut manager = Manager::default();
        manager.select(item.key());
        for _ in 0..100 {
            manager.apply_stats(Ok(BTreeMap::from([(item.key(), sample())])));
        }
        assert_eq!(manager.stats[&item.key()].len(), 60);
        manager.apply_stats(Err("timeout".into()));
        assert_eq!(
            manager.stats[&item.key()].len(),
            60,
            "failed poll keeps stale samples"
        );
        assert_eq!(manager.stats_error.as_deref(), Some("timeout"));
        let sequence = manager.begin_detail(1.0);
        manager.reconcile(&Listing::default());
        manager.apply_detail(&item.key(), sequence, DetailTab::Logs, Ok("removed".into()));
        assert_eq!(manager.selected, None);
        assert_eq!(manager.logs, None);
        assert!(manager.stats.is_empty());
    }

    #[test]
    fn inspect_projects_safe_fields_and_rejects_hostile_ids() {
        let mut item = item();
        let command = inspect_command(&item, true).unwrap();
        assert!(command.starts_with("sudo -n docker inspect --format "));
        assert!(command.ends_with(" abcdef123456"));
        assert!(command.contains(".Mounts") && command.contains(".NetworkSettings.Networks"));
        assert!(
            !command.contains(".Env")
                && !command.contains(".Config.Cmd")
                && !command.contains(".Config.Labels")
        );
        assert_eq!(
            item.terminal_command(true).as_deref(),
            Some("sudo -n docker exec -it abcdef123456 sh")
        );
        item.scope = "abcdef; touch /tmp/injected".into();
        assert!(inspect_command(&item, false).is_err());
        assert_eq!(item.terminal_command(false), None);
    }
}
