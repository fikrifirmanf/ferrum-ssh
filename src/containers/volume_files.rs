//! Browse, edit, and export Docker's standard local volumes over SSH.
use std::{fmt::Write as _, path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::conn::ssh::SshClient;

use super::{LIST_TIMEOUT, inventory::check_volume, last_line, shell_quote, sudo_prefix};

const HELPER: &str = "docker.io/library/busybox:1.37@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec";
pub(super) const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const MOUNT_INFO: &str = r#"{"driver":{{json .Driver}},"path":{{json .Mountpoint}},"has_options":{{if .Options}}true{{else}}false{{end}}}"#;
const WALK: &str = r#"
cd /volume || exit 1
for encoded do
    component=$(printf '%s' "$encoded" | base64 -d && printf '.') || exit 1
    component=${component%.}
    if [ -L "./$component" ] || ! cd -P "./$component"; then
        printf '%s\n' 'Folder is unavailable or is a symbolic link' >&2
        exit 1
    fi
    case "$(pwd -P)" in /volume|/volume/*) ;; *) exit 1 ;; esac
done
"#;
const BROWSE: &str = r#"
for entry in ./* ./.[!.]* ./..?*; do
    size=
    if [ -L "$entry" ]; then kind=l
    elif [ -d "$entry" ]; then kind=d
    elif [ -f "$entry" ]; then
        kind=f
        size=$(stat -c '%s' "$entry") || exit 1
    elif [ -e "$entry" ]; then kind=o
    else continue
    fi
    printf '%s\000%s\000%s\000' "$kind" "$size" "${entry#./}"
done
"#;

pub const EDIT_LIMIT: u64 = 512 * 1024;
const FILE_PATH: &str = r#"
filename=$(printf '%s' "$1" | base64 -d && printf '.') || exit 1
target="./${filename%.}"
shift
"#;
const FILE_CHECK: &str = r#"
identity() {
    if [ -L "$target" ] || [ ! -f "$target" ]; then
        printf '%s\n' 'The file is unavailable or no longer a regular file' >&2
        return 1
    fi
    stat -c '%d:%i:%s:%y:%z:%h' "$target"
}
"#;
const READ_TEXT: &str = r#"
revision=$(identity) || exit 1
printf '%s\n' "$revision"
head -c 524289 "$target" || exit 1
[ "$(identity)" = "$revision" ] || {
    printf '%s\n' 'The file changed while it was being read. Reopen it to try again' >&2
    exit 1
}
"#;
const SAVE_TEXT: &str = r#"
exec 9< . || exit 1
if ! flock -n 9; then
    printf '%s\n' 'Another save is in progress in this folder. Try again' >&2
    exit 1
fi
check_original() {
    current=$(identity) || return 1
    if [ "${current##*:}" != 1 ]; then
        printf '%s\n' 'Files with multiple hard links are read-only' >&2
        return 1
    fi
    if [ "$current" = "$expected_revision" ]; then
        hash=$(sha256sum < "$target") || return 1
        if [ "${hash%% *}" = "$expected_hash" ]; then return 0; fi
    fi
    printf '%s\n' 'The file changed on the host. Your draft is still open' >&2
    return 1
}
check_original || exit 1
temporary=$(mktemp ./.ferrum-edit.XXXXXX) || exit 1
trap 'rm -f "$temporary"' EXIT
trap 'exit 1' HUP INT TERM
cat > "$temporary" || exit 1
hash=$(sha256sum < "$temporary") || exit 1
if [ "${hash%% *}" != "$replacement_hash" ]; then
    printf '%s\n' 'The upload was incomplete. The file was not saved' >&2
    exit 1
fi
metadata=$(stat -c '%a %u %g' "$target") || exit 1
set -- $metadata
chown "$2:$3" "$temporary" && chmod "$1" "$temporary" || exit 1
if [ "$(stat -c '%a %u %g' "$temporary")" != "$metadata" ]; then
    printf '%s\n' 'Could not preserve file ownership and permissions' >&2
    exit 1
fi
check_original || exit 1
mv -fT "$temporary" "$target" || exit 1
printf 'Ferrum saved %s\n' "$(identity)"
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Directory,
    File(u64),
    Link,
    Other,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: Vec<u8>,
    pub kind: Kind,
}

pub fn display_name(name: &[u8]) -> String {
    let mut display = String::new();
    for chunk in name.utf8_chunks() {
        for character in chunk.valid().chars() {
            if character.is_control() || character == '\\' {
                display.extend(character.escape_default());
            } else {
                display.push(character);
            }
        }
        for byte in chunk.invalid() {
            let _ = write!(display, "\\x{byte:02x}");
        }
    }
    display
}

#[derive(Default)]
pub struct View {
    pub open: bool,
    pub directory: Vec<Vec<u8>>,
    pub result: Option<Result<Vec<Entry>, String>>,
    pub pending: bool,
    pub revision: u64,
    pub editor: Option<Editor>,
}

pub struct Editor {
    pub name: Vec<u8>,
    pub text: String,
    pub original: Option<Version>,
    pub pending: bool,
    pub error: Option<String>,
    pub dirty: bool,
    pub saved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    revision: String,
    digest: String,
}

impl Version {
    pub fn read_only(&self) -> bool {
        !self.revision.ends_with(":1")
    }
}

pub enum Edit {
    Open,
    Save { original: Version, text: String },
}

pub struct EditRequest {
    pub volume: String,
    pub directory: Vec<Vec<u8>>,
    pub name: Vec<u8>,
    pub revision: u64,
    pub edit: Edit,
}

pub enum EditResult {
    Opened { version: Version, text: String },
    Saved(Version),
}

impl View {
    pub fn navigate(&mut self, directory: Vec<Vec<u8>>) {
        self.directory = directory;
        self.result = None;
        self.pending = false;
        self.editor = None;
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn open_editor(&mut self, name: Vec<u8>) {
        self.revision = self.revision.wrapping_add(1);
        self.editor = Some(Editor {
            name,
            text: String::new(),
            original: None,
            pending: false,
            error: None,
            dirty: false,
            saved: false,
        });
    }

    pub fn close_editor(&mut self) {
        self.navigate(self.directory.clone());
    }

    pub fn apply_edit(&mut self, revision: u64, result: Result<EditResult, String>) {
        if self.revision != revision {
            return;
        }
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        editor.pending = false;
        match result {
            Ok(EditResult::Opened { version, text }) => {
                editor.original = Some(version);
                editor.text = text;
            }
            Ok(EditResult::Saved(version)) => {
                editor.original = Some(version);
                editor.dirty = false;
                editor.saved = true;
            }
            Err(error) => editor.error = Some(error),
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.navigate(Vec::new());
    }

    pub fn apply(&mut self, revision: u64, result: Result<Vec<Entry>, String>) {
        if self.open && self.revision == revision {
            self.pending = false;
            self.result = Some(result);
        }
    }
}

#[derive(Deserialize)]
struct Mount {
    driver: String,
    path: String,
    has_options: bool,
}

async fn mount_source(client: &SshClient, volume: &str, sudo: bool) -> Result<String> {
    check_volume(volume)?;
    let command = format!(
        "{}docker volume inspect --format {} -- {}",
        sudo_prefix(sudo),
        shell_quote(MOUNT_INFO),
        shell_quote(volume),
    );
    let (output, code) =
        tokio::time::timeout(LIST_TIMEOUT, client.exec_limited(&command, 64 * 1024))
            .await
            .context("Docker volume lookup timed out")??;
    if code != 0 {
        bail!("{}", last_line(&output));
    }
    let mount: Mount =
        serde_json::from_str(&output).context("Docker returned unreadable volume metadata")?;
    check_local_mount(&mount.driver, mount.has_options, &mount.path)?;
    Ok(mount.path)
}

pub(super) fn check_local_mount(driver: &str, has_options: bool, path: &str) -> Result<()> {
    if driver != "local" || has_options {
        bail!("File access is available for standard local volumes");
    }
    if path.len() < 2
        || !path.starts_with('/')
        || path.contains('\0')
        || path.split('/').any(|part| matches!(part, "." | ".."))
    {
        bail!("Docker returned an invalid volume path");
    }
    Ok(())
}

pub(super) fn helper_command(mount: &str, sudo: bool, writable: bool, command: &str) -> String {
    // Docker parses --mount as CSV, including quotes inside the inspected host path.
    let mount = format!(
        "type=bind,\"source={}\",target=/volume,{}bind-recursive=disabled",
        mount.replace('"', "\"\""),
        if writable { "" } else { "readonly," },
    );
    let writer = if writable {
        " -i --cap-add CHOWN --cap-add FOWNER"
    } else {
        ""
    };
    format!(
        "{}docker run --rm --pull=missing --network none --read-only --security-opt no-new-privileges --cap-drop ALL --cap-add DAC_OVERRIDE{writer} --mount {} {HELPER} {command}",
        sudo_prefix(sudo),
        shell_quote(&mount)
    )
}

fn valid_component(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && bytes != b"."
        && bytes != b".."
        && !bytes.contains(&b'/')
        && !bytes.contains(&0)
}

fn parse_listing(data: &[u8]) -> Result<Vec<Entry>> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let mut fields = data
        .strip_suffix(&[0])
        .context("Docker returned an incomplete file list")?
        .split(|byte| *byte == 0);
    let mut entries = Vec::new();
    while let Some(kind) = fields.next() {
        let size = fields.next().context("Missing volume file size")?;
        let name = fields.next().context("Missing volume file name")?;
        if !valid_component(name) {
            bail!("Docker returned an invalid volume file name");
        }
        let kind = match (kind, size) {
            (b"d", b"") => Kind::Directory,
            (b"l", b"") => Kind::Link,
            (b"o", b"") => Kind::Other,
            (b"f", size) => Kind::File(
                std::str::from_utf8(size)?
                    .parse()
                    .context("Invalid volume file size")?,
            ),
            _ => bail!("Docker returned an unknown file type"),
        };
        entries.push(Entry {
            name: name.to_vec(),
            kind,
        });
    }
    entries.sort_by(|left, right| {
        (left.kind != Kind::Directory, &left.name)
            .cmp(&(right.kind != Kind::Directory, &right.name))
    });
    Ok(entries)
}

pub async fn browse(
    client: &SshClient,
    volume: &str,
    directory: &[Vec<u8>],
    sudo: bool,
) -> Result<Vec<Entry>> {
    if directory.iter().any(|name| !valid_component(name)) {
        bail!("Invalid volume folder path");
    }
    let path = mount_source(client, volume, sudo).await?;
    let mut command = format!("sh -c {} ferrum", shell_quote(&format!("{WALK}{BROWSE}")));
    for component in directory {
        command.push(' ');
        command.push_str(&shell_quote(&STANDARD.encode(component)));
    }
    let command = helper_command(&path, sudo, false, &command);
    let mut listing = Vec::new();
    let (_, diagnostics, code) = client
        .exec_to_writer(&command, &mut listing, Some(2 * 1024 * 1024), IDLE_TIMEOUT)
        .await?;
    if code != 0 {
        bail!("{}", last_line(&diagnostics));
    }
    parse_listing(&listing)
}

pub async fn edit(client: &SshClient, request: EditRequest, sudo: bool) -> Result<EditResult> {
    if !valid_component(&request.name)
        || request.directory.iter().any(|name| !valid_component(name))
    {
        bail!("Invalid volume file path");
    }
    let mount = mount_source(client, &request.volume, sudo).await?;
    let mut arguments = vec![shell_quote(&STANDARD.encode(&request.name))];
    let writable = match &request.edit {
        Edit::Open => false,
        Edit::Save { original, text } => {
            if text.len() as u64 > EDIT_LIMIT || text.contains('\0') {
                bail!("The editor supports UTF-8 text files up to 512 KiB");
            }
            if original.read_only() {
                bail!("Files with multiple hard links are read-only");
            }
            arguments.push(shell_quote(&original.revision));
            arguments.push(shell_quote(&original.digest));
            arguments.push(shell_quote(&digest(text.as_bytes())));
            true
        }
    };
    arguments.extend(
        request
            .directory
            .iter()
            .map(|name| shell_quote(&STANDARD.encode(name))),
    );
    let script = if writable {
        format!(
            "{FILE_PATH}\nexpected_revision=$1; expected_hash=$2; replacement_hash=$3; shift 3\n{WALK}{FILE_CHECK}{SAVE_TEXT}"
        )
    } else {
        format!("{FILE_PATH}{WALK}{FILE_CHECK}{READ_TEXT}")
    };
    let action = format!(
        "sh -c {} ferrum {}",
        shell_quote(&script),
        arguments.join(" ")
    );
    let command = helper_command(&mount, sudo, writable, &action);
    match request.edit {
        Edit::Open => {
            let mut output = Vec::new();
            let (_, diagnostics, status) = client
                .exec_to_writer(&command, &mut output, Some(EDIT_LIMIT + 1024), IDLE_TIMEOUT)
                .await?;
            if status != 0 {
                bail!("{}", last_line(&diagnostics));
            }
            let split = output
                .iter()
                .position(|byte| *byte == b'\n')
                .context("Docker did not return a file revision")?;
            let (revision, data) = output.split_at(split);
            let text = std::str::from_utf8(&data[1..]).context("This file is not UTF-8 text")?;
            if text.len() as u64 > EDIT_LIMIT || text.contains('\0') {
                bail!("The editor supports UTF-8 text files up to 512 KiB");
            }
            let version = version(std::str::from_utf8(revision)?, text)?;
            Ok(EditResult::Opened {
                version,
                text: text.to_owned(),
            })
        }
        Edit::Save { text, .. } => {
            let (output, status) = client
                .exec_from_reader(
                    &command,
                    &mut text.as_bytes(),
                    text.len() as u64,
                    IDLE_TIMEOUT,
                )
                .await?;
            if status != 0 {
                bail!("{}", last_line(&output));
            }
            let revision = output
                .lines()
                .rev()
                .find_map(|line| line.strip_prefix("Ferrum saved "))
                .context("Docker did not confirm the saved revision. Reopen the file to check")?;
            Ok(EditResult::Saved(version(revision, &text)?))
        }
    }
}

pub(crate) fn version(revision: &str, text: &str) -> Result<Version> {
    if revision.is_empty()
        || revision.len() > 512
        || !revision
            .rsplit_once(':')
            .is_some_and(|(_, links)| links.parse::<u64>().is_ok())
    {
        bail!("Docker did not return a valid file revision");
    }
    Ok(Version {
        revision: revision.to_owned(),
        digest: digest(text.as_bytes()),
    })
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub async fn export(
    client: &SshClient,
    volume: &str,
    destination: &Path,
    sudo: bool,
) -> Result<u64> {
    let path = mount_source(client, volume, sudo).await?;
    let command = helper_command(&path, sudo, false, "tar -C /volume -cf - .");
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let staged =
        tempfile::NamedTempFile::new_in(parent).context("Could not create the archive file")?;
    let mut file = tokio::fs::File::from_std(staged.reopen()?);
    let (written, diagnostics, code) = client
        .exec_to_writer(&command, &mut file, None, IDLE_TIMEOUT)
        .await?;
    if code != 0 {
        bail!("{}", last_line(&diagnostics));
    }
    if written < 1024 || written % 512 != 0 {
        bail!("Docker returned an incomplete tar archive");
    }
    file.sync_all().await?;
    drop(file);
    staged
        .persist(destination)
        .map_err(|error| error.error)
        .context("Could not save the archive")?;
    Ok(written)
}
