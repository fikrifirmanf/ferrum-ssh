//! Docker, Podman and Kubernetes browser. Everything runs over the existing SSH connection
//! by calling the CLIs already on the server; nothing is installed there.
//!
//! One script lists containers and pods. Docker and Podman emit one JSON object per line
//! (`--format '{{json .}}'`); `kubectl get pods` is read as whitespace-separated columns.
//! Names and ids are validated before they are put into a command line.

use std::time::Duration;

use anyhow::{Result, anyhow, bail};

use crate::conn::ssh::SshClient;

const LIST_TIMEOUT: Duration = Duration::from_secs(15);
const ACTION_TIMEOUT: Duration = Duration::from_secs(60);
const LOG_LINES: u32 = 300;
/// Logs larger than this are cut from the front; the end is what people look for.
const LOG_LIMIT: usize = 256 * 1024;

const SCRIPT: &str = r#"
S=; [ "$1" = sudo ] && S="sudo -n"
for rt in docker podman; do
  command -v $rt >/dev/null 2>&1 || continue
  out=$($S $rt ps -a --format '{{json .}}' 2>&1); rc=$?
  if [ $rc -eq 0 ]; then echo "@$rt"; else echo "@$rt-error"; fi
  printf '%s\n' "$out"
done
if command -v kubectl >/dev/null 2>&1; then
  out=$(kubectl get pods -A --no-headers 2>&1); rc=$?
  if [ $rc -eq 0 ]; then echo "@kubectl"; else echo "@kubectl-error"; fi
  printf '%s\n' "$out"
fi
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runtime {
    Docker,
    Podman,
    Kubernetes,
}

impl Runtime {
    pub fn label(self) -> &'static str {
        match self {
            Runtime::Docker => "Docker",
            Runtime::Podman => "Podman",
            Runtime::Kubernetes => "Kubernetes",
        }
    }

    fn cli(self) -> &'static str {
        match self {
            Runtime::Docker => "docker",
            Runtime::Podman => "podman",
            Runtime::Kubernetes => "kubectl",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub runtime: Runtime,
    /// Container id (Docker, Podman) or namespace (Kubernetes).
    pub scope: String,
    pub name: String,
    /// Image, or the READY column for pods.
    pub detail: String,
    /// `running`, `exited` ... or the pod STATUS column.
    pub state: String,
    pub status: String,
    pub ports: String,
}

impl Item {
    pub fn key(&self) -> String {
        format!("{}/{}", self.scope, self.name)
    }

    pub fn is_running(&self) -> bool {
        match self.runtime {
            Runtime::Kubernetes => self.state == "Running" || self.state == "Completed",
            _ => self.state == "running",
        }
    }

    pub fn is_failing(&self) -> bool {
        match self.runtime {
            Runtime::Kubernetes => !matches!(
                self.state.as_str(),
                "Running" | "Completed" | "Pending" | "ContainerCreating" | "Terminating"
            ),
            _ => matches!(self.state.as_str(), "dead" | "restarting"),
        }
    }

    /// Command that opens a shell inside, typed into the terminal for the user to run.
    pub fn shell_command(&self) -> Option<String> {
        self.valid()?;
        Some(match self.runtime {
            Runtime::Kubernetes => {
                format!("kubectl exec -it -n {} {} -- sh", self.scope, self.name)
            }
            rt => format!("{} exec -it {} sh", rt.cli(), self.scope),
        })
    }

    fn valid(&self) -> Option<()> {
        let scope_ok = match self.runtime {
            Runtime::Kubernetes => is_safe(&self.scope),
            _ => !self.scope.is_empty() && self.scope.chars().all(|c| c.is_ascii_hexdigit()),
        };
        (scope_ok && is_safe(&self.name)).then_some(())
    }
}

/// Docker, Podman and Kubernetes names are made of these characters; refusing anything else
/// keeps a hostile name from becoming shell syntax.
fn is_safe(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 253
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
    Restart,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Start => "Start",
            Action::Stop => "Stop",
            Action::Restart => "Restart",
        }
    }

    fn verb(self) -> &'static str {
        match self {
            Action::Start => "start",
            Action::Stop => "stop",
            Action::Restart => "restart",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listing {
    pub items: Vec<Item>,
    /// Runtimes whose CLI is installed but failed (daemon down, permission denied ...).
    pub errors: Vec<(Runtime, String)>,
}

/// With `sudo`, Docker and Podman run as `sudo -n` (passwordless sudo only), for users that
/// are not in the docker group.
pub async fn list(client: &SshClient, sudo: bool) -> Result<Listing> {
    let command = format!(
        "sh -c {} sh{}",
        shell_quote(SCRIPT),
        if sudo { " sudo" } else { "" }
    );
    let (output, _) = tokio::time::timeout(LIST_TIMEOUT, client.exec(&command))
        .await
        .map_err(|_| anyhow!("The server did not answer in {}s", LIST_TIMEOUT.as_secs()))??;
    Ok(parse(&output))
}

pub async fn act(client: &SshClient, item: &Item, action: Action, sudo: bool) -> Result<()> {
    if item.runtime == Runtime::Kubernetes {
        bail!("Kubernetes pods can't be {}ed from here", action.verb());
    }
    item.valid()
        .ok_or_else(|| anyhow!("unsafe container name"))?;
    let command = format!(
        "{}{} {} {} 2>&1",
        sudo_prefix(sudo),
        item.runtime.cli(),
        action.verb(),
        item.scope
    );
    let (output, code) = tokio::time::timeout(ACTION_TIMEOUT, client.exec(&command))
        .await
        .map_err(|_| anyhow!("{} timed out", action.label()))??;
    if code != 0 {
        bail!("{}", last_line(&output));
    }
    Ok(())
}

pub async fn logs(client: &SshClient, item: &Item, sudo: bool) -> Result<String> {
    item.valid()
        .ok_or_else(|| anyhow!("unsafe container name"))?;
    let command = match item.runtime {
        Runtime::Kubernetes => format!(
            "kubectl logs --tail={LOG_LINES} -n {} {} 2>&1",
            item.scope, item.name
        ),
        rt => format!(
            "{}{} logs --tail {LOG_LINES} {} 2>&1",
            sudo_prefix(sudo),
            rt.cli(),
            item.scope
        ),
    };
    let (output, code) = tokio::time::timeout(LIST_TIMEOUT, client.exec(&command))
        .await
        .map_err(|_| anyhow!("The server did not answer in {}s", LIST_TIMEOUT.as_secs()))??;
    if code != 0 {
        bail!("{}", last_line(&output));
    }
    Ok(clip_start(strip_ansi(&output), LOG_LIMIT))
}

fn sudo_prefix(sudo: bool) -> &'static str {
    if sudo { "sudo -n " } else { "" }
}

/// True for the error Docker and Podman print when the user may not use the socket.
pub fn is_permission_error(message: &str) -> bool {
    message.to_lowercase().contains("permission denied")
}

fn last_line(output: &str) -> String {
    output
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("the command failed")
        .trim()
        .to_owned()
}

fn clip_start(text: String, limit: usize) -> String {
    if text.len() <= limit {
        return text;
    }
    let mut start = text.len() - limit;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("… earlier output cut …\n{}", &text[start..])
}

/// Removes escape sequences so colour codes in logs do not show as garbage.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if chars.next_if_eq(&'[').is_some() {
            while let Some(n) = chars.next() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        }
    }
    out
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

pub fn parse(output: &str) -> Listing {
    let mut listing = Listing::default();
    let mut section: Option<(String, Vec<&str>)> = None;
    let mut sections = vec![];
    for line in output.lines() {
        if let Some(name) = line.strip_prefix('@') {
            sections.extend(section.take());
            section = Some((name.trim().to_owned(), vec![]));
        } else if let Some((_, lines)) = section.as_mut() {
            if !line.trim().is_empty() {
                lines.push(line);
            }
        }
    }
    sections.extend(section);
    for (name, lines) in sections {
        let (base, failed) = match name.strip_suffix("-error") {
            Some(base) => (base, true),
            None => (name.as_str(), false),
        };
        let runtime = match base {
            "docker" => Runtime::Docker,
            "podman" => Runtime::Podman,
            "kubectl" => Runtime::Kubernetes,
            _ => continue,
        };
        if failed {
            let message = lines.last().copied().unwrap_or("the command failed");
            listing.errors.push((runtime, message.trim().to_owned()));
            continue;
        }
        for line in lines {
            let item = match runtime {
                Runtime::Kubernetes => parse_pod(line),
                _ => parse_container(runtime, line),
            };
            listing.items.extend(item);
        }
    }
    listing
}

fn parse_container(runtime: Runtime, line: &str) -> Option<Item> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    // Podman prints lists where Docker prints strings.
    let text = |key: &str| match value.get(key) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    };
    let id = ["ID", "Id"]
        .iter()
        .map(|k| text(k))
        .find(|s| !s.is_empty())?;
    let name = text("Names").split(',').next()?.trim().to_owned();
    Some(Item {
        runtime,
        // Podman prints the full id; the short one is enough for commands.
        scope: id.chars().take(12).collect(),
        name,
        detail: text("Image"),
        state: text("State").to_lowercase(),
        status: text("Status"),
        ports: text("Ports"),
    })
}

fn parse_pod(line: &str) -> Option<Item> {
    let mut cols = line.split_whitespace();
    let namespace = cols.next()?;
    let name = cols.next()?;
    let ready = cols.next()?;
    let status = cols.next()?;
    Some(Item {
        runtime: Runtime::Kubernetes,
        scope: namespace.to_owned(),
        name: name.to_owned(),
        detail: ready.to_owned(),
        state: status.to_owned(),
        status: line
            .split_whitespace()
            .skip(4)
            .collect::<Vec<_>>()
            .join(" "),
        ports: String::new(),
    })
}

/// Log text shown in place of the list.
pub struct LogView {
    pub item: Item,
    /// `None` while loading.
    pub text: Option<Result<String, String>>,
}

/// Panel state for one terminal tab.
#[derive(Default)]
pub struct Browser {
    /// Session this state belongs to; switching panes starts over.
    pub session: u64,
    pub listing: Option<Listing>,
    pub error: Option<String>,
    pub in_flight: bool,
    /// UI time of the last list request.
    pub last_poll: f64,
    /// Label of the action running right now (`Stop web`).
    pub busy: Option<String>,
    /// Outcome of the last action, shown until the next one.
    pub notice: Option<Result<String, String>>,
    pub logs: Option<LogView>,
    pub filter: String,
    /// Run Docker and Podman through `sudo -n`; chosen by the user for this pane.
    pub use_sudo: bool,
}

impl Browser {
    pub fn reset(&mut self, session: u64) {
        *self = Self {
            session,
            ..Default::default()
        };
    }

    pub fn apply_list(&mut self, result: Result<Listing, String>) {
        self.in_flight = false;
        match result {
            Ok(listing) => {
                self.listing = Some(listing);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub fn apply_action(&mut self, result: Result<String, String>) {
        self.busy = None;
        self.notice = Some(result);
        // Show the new state right away instead of waiting for the next poll.
        self.last_poll = f64::NEG_INFINITY;
    }

    /// `key` identifies the item the logs belong to, so a slow reply for an item the user
    /// already left cannot show under another one.
    pub fn apply_logs(&mut self, key: &str, result: Result<String, String>) {
        if let Some(view) = self.logs.as_mut().filter(|v| v.item.key() == key) {
            view.text = Some(result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCKER: &str = r#"@docker
{"Command":"\"/docker-entrypoint.…\"","CreatedAt":"2026-10-01","ID":"3f2a9c1b7d44","Image":"nginx:1.27","Names":"web","Ports":"0.0.0.0:8080->80/tcp","State":"running","Status":"Up 2 hours"}
{"ID":"aa11bb22cc33","Image":"redis:7","Names":"cache","Ports":"","State":"exited","Status":"Exited (0) 3 days ago"}
"#;

    #[test]
    fn parses_docker_json_lines() {
        let listing = parse(DOCKER);
        assert_eq!(listing.items.len(), 2);
        let web = &listing.items[0];
        assert_eq!(web.runtime, Runtime::Docker);
        assert_eq!(web.scope, "3f2a9c1b7d44");
        assert_eq!(web.name, "web");
        assert_eq!(web.detail, "nginx:1.27");
        assert!(web.is_running());
        assert_eq!(web.ports, "0.0.0.0:8080->80/tcp");
        assert!(!listing.items[1].is_running());
    }

    #[test]
    fn parses_podman_list_fields_and_long_ids() {
        let line = r#"{"Id":"0123456789abcdef0123","Image":"docker.io/library/alpine","Names":["job","alias"],"Ports":null,"State":"running","Status":"Up 1 minute"}"#;
        let listing = parse(&format!("@podman\n{line}\n"));
        let item = &listing.items[0];
        assert_eq!(item.runtime, Runtime::Podman);
        assert_eq!(item.scope, "0123456789ab");
        assert_eq!(item.name, "job");
        assert_eq!(item.ports, "");
    }

    #[test]
    fn parses_kubectl_columns_including_restart_age() {
        let out = "@kubectl\n\
            default   api-7d9f   1/1   Running   2 (5m ago)   3d\n\
            kube-system   dns-1   0/1   CrashLoopBackOff   9   1h\n";
        let listing = parse(out);
        assert_eq!(listing.items.len(), 2);
        let api = &listing.items[0];
        assert_eq!(
            (api.scope.as_str(), api.name.as_str()),
            ("default", "api-7d9f")
        );
        assert_eq!(api.detail, "1/1");
        assert!(api.is_running() && !api.is_failing());
        assert!(listing.items[1].is_failing());
    }

    #[test]
    fn reports_a_failing_runtime_without_hiding_the_others() {
        let out = format!(
            "@docker-error\npermission denied while trying to connect to the Docker daemon\n{}",
            "@kubectl\ndefault a 1/1 Running 0 1m\n"
        );
        let listing = parse(&out);
        assert_eq!(listing.errors.len(), 1);
        assert_eq!(listing.errors[0].0, Runtime::Docker);
        assert!(listing.errors[0].1.contains("permission denied"));
        assert_eq!(listing.items.len(), 1);
    }

    #[test]
    fn permission_errors_are_recognised() {
        assert!(is_permission_error(
            "permission denied while trying to connect to the docker API at unix:///var/run/docker.sock"
        ));
        assert!(!is_permission_error("Cannot connect to the Docker daemon"));
    }

    #[test]
    fn empty_output_means_no_runtime() {
        assert_eq!(parse(""), Listing::default());
    }

    #[test]
    fn shell_commands_refuse_hostile_names() {
        let mut item = parse(DOCKER).items.remove(0);
        assert_eq!(
            item.shell_command().as_deref(),
            Some("docker exec -it 3f2a9c1b7d44 sh")
        );
        item.name = "web; rm -rf ~".into();
        assert_eq!(item.shell_command(), None);
        item.name = "web".into();
        item.scope = "abc$(id)".into();
        assert_eq!(item.shell_command(), None);

        let pod = parse("@kubectl\nprod api-1 1/1 Running 0 1m\n")
            .items
            .remove(0);
        assert_eq!(
            pod.shell_command().as_deref(),
            Some("kubectl exec -it -n prod api-1 -- sh")
        );
    }

    #[test]
    fn an_action_result_clears_busy_and_forces_a_refresh() {
        let mut browser = Browser {
            busy: Some("Stop web".into()),
            last_poll: 100.0,
            ..Default::default()
        };
        browser.apply_action(Err("no such container".into()));
        assert_eq!(browser.busy, None);
        assert_eq!(browser.notice, Some(Err("no such container".into())));
        assert!(browser.last_poll < 0.0);
    }

    #[test]
    fn a_failed_poll_keeps_the_last_listing() {
        let mut browser = Browser::default();
        browser.apply_list(Ok(parse(DOCKER)));
        browser.apply_list(Err("timeout".into()));
        assert_eq!(browser.error.as_deref(), Some("timeout"));
        assert_eq!(browser.listing.as_ref().map(|l| l.items.len()), Some(2));
    }

    #[test]
    fn logs_lose_colour_codes_and_keep_the_tail() {
        assert_eq!(strip_ansi("\u{1b}[31merror\u{1b}[0m ok"), "error ok");
        let clipped = clip_start("a".repeat(10) + "tail", 4);
        assert!(clipped.ends_with("tail") && clipped.starts_with('…'));
        assert_eq!(clip_start("short".into(), 100), "short");
    }
}
