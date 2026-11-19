//! Validate an archive locally, then restore it into a newly owned Docker volume.
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, BufRead, BufReader, BufWriter, Cursor, Read, Seek},
    path::Path,
};

use anyhow::{Context, Result, bail};
use flate2::read::MultiGzDecoder;
use serde::Deserialize;
use tar::{Archive, Builder, EntryType, Header};

use crate::conn::ssh::SshClient;

use super::{
    LIST_TIMEOUT,
    inventory::check_volume,
    last_line, shell_quote, sudo_prefix,
    volume_files::{IDLE_TIMEOUT, check_local_mount, display_name, helper_command},
};

const IMPORT_LABEL: &str = "dev.ferrum.volume-import";
const IMPORT_INFO: &str = r#"{"driver":{{json .Driver}},"path":{{json .Mountpoint}},"has_options":{{if .Options}}true{{else}}false{{end}},"token":{{json (index .Labels "dev.ferrum.volume-import")}}}"#;
const RESTORE: &str = r#"
cd /volume || exit 1
for entry in ./* ./.[!.]* ./..?*; do
    if [ -e "$entry" ] || [ -L "$entry" ]; then
        printf '%s\n' 'The destination volume already contains files' >&2
        exit 1
    fi
done
exec tar -xkf - --numeric-owner
"#;

struct Link {
    path: Vec<u8>,
    target: Vec<u8>,
    header: Header,
}

fn normalize_path(path: &[u8]) -> Result<Vec<u8>> {
    if path.starts_with(b"/") || path.contains(&0) {
        bail!("Archive contains an absolute or invalid path");
    }
    let mut normalized = Vec::new();
    for part in path.split(|byte| *byte == b'/') {
        match part {
            b"" | b"." => continue,
            b".." => bail!("Archive paths cannot leave the destination folder"),
            _ => {}
        }
        if !normalized.is_empty() {
            normalized.push(b'/');
        }
        normalized.extend_from_slice(part);
    }
    Ok(normalized)
}

fn check_symbolic_target(path: &[u8], target: &[u8]) -> Result<()> {
    if target.is_empty() || target.starts_with(b"/") || target.contains(&0) {
        bail!("A symbolic link points outside the volume");
    }
    let mut depth = path.iter().filter(|byte| **byte == b'/').count();
    for part in target.split(|byte| *byte == b'/') {
        match part {
            b"" | b"." => {}
            b".." if depth == 0 => bail!("A symbolic link points outside the volume"),
            b".." => depth -= 1,
            _ => depth += 1,
        }
    }
    Ok(())
}

/// A new header with raw Unix names keeps both parsers on the same path semantics.
fn write_name(
    output: &mut Builder<BufWriter<File>>,
    field: &mut [u8],
    name: &[u8],
    extension_type: EntryType,
) -> Result<()> {
    if name.len() >= field.len() {
        let mut extension = Header::new_gnu();
        extension.set_path("././@LongLink")?;
        extension.set_mode(0o644);
        extension.set_uid(0);
        extension.set_gid(0);
        extension.set_mtime(0);
        extension.set_entry_type(extension_type);
        extension.set_size((name.len() + 1) as u64);
        extension.set_cksum();
        output.append(&extension, name.chain(&[0][..]))?;
    }
    let length = name.len().min(field.len() - 1);
    field[..length].copy_from_slice(&name[..length]);
    Ok(())
}

fn clean_header(original: &Header, kind: EntryType, size: u64) -> Result<Header> {
    let mut header = Header::new_gnu();
    header.set_entry_type(kind);
    header.set_size(size);
    header.set_mode(original.mode()? & 0o1777);
    header.set_uid(original.uid()?);
    header.set_gid(original.gid()?);
    header.set_mtime(original.mtime()?);
    Ok(header)
}

fn append(
    output: &mut Builder<BufWriter<File>>,
    mut header: Header,
    path: &[u8],
    target: Option<&[u8]>,
    data: impl Read,
) -> Result<()> {
    let fields = header
        .as_gnu_mut()
        .expect("restore headers are GNU tar headers");
    write_name(output, &mut fields.name, path, EntryType::GNULongName)?;
    if let Some(target) = target {
        write_name(output, &mut fields.linkname, target, EntryType::GNULongLink)?;
    }
    header.set_cksum();
    output.append(&header, data)?;
    Ok(())
}

pub(super) fn prepare(path: &Path) -> Result<File> {
    let mut file = BufReader::new(File::open(path).context("Could not open the archive")?);
    let mut input: Box<dyn Read> = if file.fill_buf()?.starts_with(&[0x1f, 0x8b]) {
        Box::new(MultiGzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut first = [0; 512];
    input
        .read_exact(&mut first)
        .context("The file is not a complete tar archive")?;
    let mut archive = Archive::new(Cursor::new(first).chain(input));
    let staged = tempfile::tempfile().context("Could not stage the archive locally")?;
    let mut output = Builder::new(BufWriter::new(staged));
    let mut entries = BTreeMap::new();
    let mut links = Vec::new();
    let mut root = None;
    for entry in archive.entries()? {
        let entry = entry.context("Could not read a tar entry")?;
        let path = normalize_path(&entry.path_bytes())?;
        let kind = entry.header().entry_type();
        if path.is_empty() {
            if !kind.is_dir() || root.is_some() || entry.size() != 0 {
                bail!("The archive has an invalid root entry");
            }
            root = Some(clean_header(entry.header(), kind, 0)?);
            continue;
        }
        if entries.insert(path.clone(), kind).is_some() {
            bail!("The archive repeats {}", display_name(&path));
        }
        let size = entry.size();
        if kind.is_file() {
            let header = clean_header(entry.header(), kind, size)?;
            let mut data = entry.take(size);
            append(&mut output, header, &path, None, &mut data)?;
            if data.limit() != 0 {
                bail!("The archive ends inside {}", display_name(&path));
            }
        } else if kind.is_dir() && size == 0 {
            append(
                &mut output,
                clean_header(entry.header(), kind, 0)?,
                &path,
                None,
                io::empty(),
            )?;
        } else if (kind.is_symlink() || kind.is_hard_link()) && size == 0 {
            let target = entry
                .link_name_bytes()
                .context("An archive link has no target")?;
            let target = if kind.is_symlink() {
                check_symbolic_target(&path, &target)?;
                target.into_owned()
            } else {
                normalize_path(&target)?
            };
            links.push(Link {
                path,
                target,
                header: clean_header(entry.header(), kind, 0)?,
            });
        } else {
            bail!("Unsupported archive entry {}", display_name(&path));
        }
    }
    // Consume terminal padding and validate the gzip checksum before Docker is contacted.
    let mut rest = archive.into_inner();
    let mut padding = [0; 8192];
    loop {
        let count = rest
            .read(&mut padding)
            .context("Could not finish reading the archive")?;
        if count == 0 {
            break;
        }
        if padding[..count].iter().any(|byte| *byte != 0) {
            bail!("The archive contains data after its end marker");
        }
    }
    for path in entries.keys() {
        for (index, _) in path.iter().enumerate().filter(|(_, byte)| **byte == b'/') {
            if entries
                .get(&path[..index])
                .is_some_and(|kind| !kind.is_dir())
            {
                bail!("The archive places an entry inside a file or link");
            }
        }
    }
    // Create links after the file data. No later archive entry can write through a link.
    for link in links {
        if link.header.entry_type().is_hard_link()
            && !entries.get(&link.target).is_some_and(EntryType::is_file)
        {
            bail!("A hard link has no regular file target in this archive");
        }
        append(
            &mut output,
            link.header,
            &link.path,
            Some(&link.target),
            io::empty(),
        )?;
    }
    if let Some(header) = root {
        append(&mut output, header, b".", None, io::empty())?;
    }
    let mut file = output.into_inner()?.into_inner()?;
    file.rewind()?;
    Ok(file)
}

#[derive(Deserialize)]
struct OwnedVolume {
    driver: String,
    path: String,
    has_options: bool,
    token: Option<String>,
}

async fn docker(client: &SshClient, sudo: bool, arguments: &str) -> Result<String> {
    let command = format!("{}docker volume {arguments}", sudo_prefix(sudo));
    let (output, status) =
        tokio::time::timeout(LIST_TIMEOUT, client.exec_limited(&command, 64 * 1024))
            .await
            .context("Docker volume command timed out")??;
    if status != 0 {
        bail!("{}", last_line(&output));
    }
    Ok(output)
}

async fn describe(client: &SshClient, name: &str, sudo: bool) -> Result<OwnedVolume> {
    let arguments = format!(
        "inspect --format {} -- {}",
        shell_quote(IMPORT_INFO),
        shell_quote(name)
    );
    let output = docker(client, sudo, &arguments).await?;
    serde_json::from_str(&output).context("Docker returned unreadable import metadata")
}

async fn remove_owned(client: &SshClient, name: &str, token: &str, sudo: bool) -> Result<()> {
    let current = describe(client, name, sudo).await?;
    if current.token.as_deref() != Some(token) {
        bail!("The volume no longer belongs to this import");
    }
    docker(client, sudo, &format!("rm -- {}", shell_quote(name))).await?;
    Ok(())
}

async fn restore(client: &SshClient, volume: &OwnedVolume, staged: File, sudo: bool) -> Result<()> {
    check_local_mount(&volume.driver, volume.has_options, &volume.path)?;
    let command = helper_command(
        &volume.path,
        sudo,
        true,
        &format!("sh -c {}", shell_quote(RESTORE)),
    );
    let size = staged.metadata()?.len();
    let mut input = tokio::fs::File::from_std(staged);
    let (output, status) = client
        .exec_from_reader(&command, &mut input, size, IDLE_TIMEOUT)
        .await?;
    if status != 0 {
        bail!("{}", last_line(&output));
    }
    Ok(())
}

pub async fn import(client: &SshClient, name: &str, source: &Path, sudo: bool) -> Result<()> {
    check_volume(name)?;
    let source = source.to_owned();
    let staged = tokio::task::spawn_blocking(move || prepare(&source))
        .await
        .context("Archive validation was interrupted")??;
    let token = uuid::Uuid::new_v4().to_string();
    let label = shell_quote(&format!("{IMPORT_LABEL}={token}"));
    docker(
        client,
        sudo,
        &format!(
            "create --driver local --label {label} -- {}",
            shell_quote(name)
        ),
    )
    .await?;
    let volume = describe(client, name, sudo)
        .await
        .context("Could not verify the destination volume. No archive bytes were sent")?;
    if volume.token.as_deref() != Some(token.as_str()) {
        bail!("The volume {name} already exists. Enter a new volume name");
    }
    if let Err(error) = restore(client, &volume, staged, sudo).await {
        if let Err(cleanup) = remove_owned(client, name, &token, sudo).await {
            bail!(
                "Import failed. {error:#}. Could not remove the partial volume {name}. {cleanup:#}"
            );
        }
        return Err(error.context("Import failed and the partial volume was removed"));
    }
    Ok(())
}
