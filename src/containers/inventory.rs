//! Docker images, volumes and build history scoped to an existing SSH session.
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::conn::ssh::SshClient;

use super::{LIST_TIMEOUT, last_line, shell_quote, strip_ansi, sudo_prefix};

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum Kind {
    Images,
    Volumes,
    Builds,
}

const IMAGE_LIST: &str = r#"{"ID":{{json .ID}},"Repository":{{json .Repository}},"Tag":{{json .Tag}},"Size":{{json .Size}}}"#;
const VOLUME_LIST: &str =
    r#"{"Name":{{json .Name}},"Driver":{{json .Driver}},"Scope":{{json .Scope}}}"#;
const IMAGE_DETAIL: &str = r#"{"id":{{json .Id}},"tags":{{json .RepoTags}},"digests":{{json .RepoDigests}},"os":{{json .Os}},"architecture":{{json .Architecture}},"created":{{json .Created}},"size_bytes":{{json .Size}},"layers":{{json .RootFS.Layers}}}"#;
const VOLUME_DETAIL: &str = r#"{"name":{{json .Name}},"driver":{{json .Driver}},"scope":{{json .Scope}},"created":{{json .CreatedAt}},"mountpoint":{{json .Mountpoint}}}"#;

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Image {
    #[serde(rename = "ID")]
    pub id: String,
    pub repository: String,
    pub tag: String,
    pub size: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Volume {
    pub name: String,
    pub driver: String,
    pub scope: String,
}

#[derive(Deserialize)]
pub struct Build {
    #[serde(rename = "ref")]
    pub reference: String,
    pub name: String,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_steps: Option<u32>,
    pub total_steps: Option<u32>,
    pub cached_steps: Option<u32>,
}

pub enum Entry {
    Image(Image),
    Volume(Volume),
    Build(Build),
}

impl Entry {
    pub fn id(&self) -> &str {
        match self {
            Self::Image(image) => &image.id,
            Self::Volume(volume) => &volume.name,
            Self::Build(build) => &build.reference,
        }
    }
}

impl Image {
    pub fn reference(&self) -> Option<String> {
        if self.repository.is_empty() || self.repository == "<none>" {
            return None;
        }
        if self.tag.is_empty() || self.tag == "<none>" {
            return Some(self.repository.clone());
        }
        Some(format!("{}:{}", self.repository, self.tag))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    List(Kind),
    Inspect(Kind, String),
    Pull(String),
    CreateVolume(String),
}

impl Request {
    pub fn kind(&self) -> Kind {
        match self {
            Self::List(kind) | Self::Inspect(kind, _) => *kind,
            Self::Pull(_) => Kind::Images,
            Self::CreateVolume(_) => Kind::Volumes,
        }
    }

    fn command(&self, sudo: bool) -> Result<String> {
        let arguments = match self {
            Self::List(Kind::Images) => {
                format!("image ls --no-trunc --format {}", shell_quote(IMAGE_LIST))
            }
            Self::List(Kind::Volumes) => format!("volume ls --format {}", shell_quote(VOLUME_LIST)),
            Self::List(Kind::Builds) => "buildx history ls --no-trunc --format '{{json .}}'".into(),
            Self::Inspect(Kind::Images, id) => {
                if !valid_name(id, 512, b":") {
                    bail!("Invalid Docker image ID");
                }
                format!(
                    "image inspect --format {} -- {}",
                    shell_quote(IMAGE_DETAIL),
                    shell_quote(id)
                )
            }
            Self::Inspect(Kind::Volumes, name) => {
                check_volume(name)?;
                format!(
                    "volume inspect --format {} -- {}",
                    shell_quote(VOLUME_DETAIL),
                    shell_quote(name)
                )
            }
            Self::Inspect(Kind::Builds, reference) => build_logs_command(reference, false)?,
            Self::Pull(reference) => {
                if !valid_name(reference, 512, b":/@") {
                    bail!("Enter a Docker image reference, such as redis:7");
                }
                format!("image pull -- {}", shell_quote(reference))
            }
            Self::CreateVolume(name) => {
                check_volume(name)?;
                format!("volume create -- {}", shell_quote(name))
            }
        };
        Ok(format!("{}docker {arguments}", sudo_prefix(sudo)))
    }
}

pub(super) fn build_logs_command(reference: &str, raw_json: bool) -> Result<String> {
    let mut parts = reference.splitn(4, '/');
    let (builder, id) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(id), None, None, None) if valid_name(id, 128, b"") => (None, id),
        (Some(builder), Some(node), Some(id), None)
            if [builder, node, id]
                .into_iter()
                .all(|part| valid_name(part, 128, b"")) =>
        {
            (Some(builder), id)
        }
        _ => bail!("Invalid Buildx record reference"),
    };
    // Buildx lists qualified references but history logs takes a record ID on a builder.
    let builder = builder
        .map(|name| format!(" --builder {}", shell_quote(name)))
        .unwrap_or_default();
    let progress = if raw_json { "rawjson" } else { "plain" };
    Ok(format!(
        "buildx history logs{builder} --progress {progress} -- {}",
        shell_quote(id)
    ))
}

fn valid_name(name: &str, max_len: usize, extra: &[u8]) -> bool {
    name.len() <= max_len
        && name
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b) || extra.contains(&b))
}

pub(super) fn check_volume(name: &str) -> Result<()> {
    if !valid_name(name, 255, b"") {
        bail!("Enter a volume name using letters, numbers, dots, underscores or hyphens");
    }
    Ok(())
}

pub enum Reply {
    Listing(Vec<Entry>),
    Detail(Detail),
    Saved(String),
}

pub enum Detail {
    Metadata(Value),
    BuildLog(String),
}

pub async fn run(client: &SshClient, request: &Request, sudo: bool) -> Result<Reply> {
    let command = request.command(sudo)?;
    let timeout = if matches!(request, Request::Pull(_)) {
        Duration::from_secs(180)
    } else {
        LIST_TIMEOUT
    };
    let (output, code) =
        tokio::time::timeout(timeout, client.exec_limited(&command, 2 * 1024 * 1024))
            .await
            .map_err(|_| anyhow!("Docker did not answer within {} seconds", timeout.as_secs()))??;
    if code != 0 {
        if request.kind() == Kind::Builds {
            bail!("Buildx history failed. {}", last_line(&output));
        }
        bail!("{}", last_line(&output));
    }
    match request {
        Request::List(kind) => {
            let entries = output
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| {
                    match kind {
                        Kind::Images => serde_json::from_str(line).map(Entry::Image),
                        Kind::Volumes => serde_json::from_str(line).map(Entry::Volume),
                        Kind::Builds => serde_json::from_str(line).map(Entry::Build),
                    }
                    .context("Docker returned an unreadable inventory row")
                })
                .collect::<Result<_>>()?;
            Ok(Reply::Listing(entries))
        }
        Request::Inspect(Kind::Builds, _) => {
            Ok(Reply::Detail(Detail::BuildLog(strip_ansi(&output))))
        }
        Request::Inspect(..) => Ok(Reply::Detail(Detail::Metadata(
            serde_json::from_str(&output).context("Docker returned unreadable metadata")?,
        ))),
        Request::Pull(reference) | Request::CreateVolume(reference) => {
            Ok(Reply::Saved(reference.clone()))
        }
    }
}

#[derive(Default)]
pub struct Page {
    pub listing: Option<Result<Vec<Entry>, String>>,
    pub selected: Option<String>,
    pub detail: Option<Result<Detail, String>>,
    pub pending: bool,
    pub notice: Option<Result<String, String>>,
    pub filter: String,
    pub log_filter: String,
    pub input: String,
    pub files: super::volume_files::View,
}

impl Page {
    pub fn select(&mut self, id: &str) {
        if self.selected.as_deref() != Some(id) {
            self.selected = Some(id.to_owned());
            self.detail = None;
            self.files.close();
        }
    }

    pub fn clear_selection(&mut self) {
        self.selected = None;
        self.detail = None;
        self.files.close();
    }

    pub fn apply(&mut self, request: Request, result: Result<Reply, String>) {
        self.pending = false;
        match result {
            Ok(Reply::Listing(entries)) => {
                if !entries
                    .iter()
                    .any(|entry| Some(entry.id()) == self.selected.as_deref())
                {
                    self.clear_selection();
                }
                self.detail = None;
                self.listing = Some(Ok(entries));
            }
            Ok(Reply::Detail(value)) => {
                if let Request::Inspect(_, id) = request
                    && Some(id.as_str()) == self.selected.as_deref()
                {
                    self.detail = Some(Ok(value));
                }
            }
            Ok(Reply::Saved(target)) => {
                self.notice = Some(Ok(match request {
                    Request::Pull(_) => format!("Image {target} is available on this host."),
                    _ => format!("Volume {target} is available on this host."),
                }));
                self.input.clear();
                self.listing = None;
            }
            Err(error) => match request {
                Request::List(_) => self.listing = Some(Err(error)),
                Request::Inspect(_, id) if Some(id.as_str()) == self.selected.as_deref() => {
                    self.detail = Some(Err(error));
                }
                Request::Inspect(..) => {}
                _ => self.notice = Some(Err(error)),
            },
        }
    }
}

#[derive(Default)]
pub struct Inventory {
    images: Page,
    volumes: Page,
    builds: Page,
}

impl Inventory {
    pub fn page(&mut self, kind: Kind) -> &mut Page {
        match kind {
            Kind::Images => &mut self.images,
            Kind::Volumes => &mut self.volumes,
            Kind::Builds => &mut self.builds,
        }
    }
}
