//! One-way folder sync: immutable preview, revalidation, then ordered operations.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::Backend;

#[derive(Clone)]
pub struct Location {
    pub backend: Backend,
    pub path: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Update,
    Mirror,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    directory: bool,
    hash: [u8; 32],
    size: u64,
    permissions: Option<u32>,
}

type Snapshot = BTreeMap<String, Stamp>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    CreateFolder(String),
    CopyFile {
        path: String,
        replace: bool,
        size: u64,
    },
    DeleteFile(String),
    Conflict(String),
}

impl Change {
    pub fn path(&self) -> &str {
        match self {
            Self::CreateFolder(p) | Self::DeleteFile(p) | Self::Conflict(p) => p,
            Self::CopyFile { path, .. } => path,
        }
    }
    pub fn label(&self) -> &str {
        match self {
            Self::CreateFolder(_) => "Create folder",
            Self::CopyFile { replace: false, .. } => "Copy file",
            Self::CopyFile { replace: true, .. } => "Replace file",
            Self::DeleteFile(_) => "Delete file",
            Self::Conflict(_) => "File/folder conflict",
        }
    }
}

#[derive(Clone)]
pub struct Plan {
    pub source: Location,
    pub destination: Location,
    pub changes: Vec<Change>,
    pub mode: Mode,
    exclusions: String,
    source_snapshot: Snapshot,
    destination_snapshot: Snapshot,
}

impl Plan {
    pub fn deletions(&self) -> usize {
        self.changes
            .iter()
            .filter(|c| matches!(c, Change::DeleteFile(_)))
            .count()
    }
    pub fn conflicts(&self) -> bool {
        self.changes
            .iter()
            .any(|c| matches!(c, Change::Conflict(_)))
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        bail!("Sync cancelled. Completed changes remain applied.");
    }
    Ok(())
}

// Patterns match a relative path or any single component. '*' and '?' are supported.
fn exclusions(text: &str) -> Result<Vec<regex::Regex>> {
    if text.len() > 4096 {
        bail!("Use at most 4096 characters for exclusions.");
    }
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|pattern| {
            let mut out = String::from("^");
            for ch in pattern.chars() {
                match ch {
                    '*' => out.push_str(".*"),
                    '?' => out.push('.'),
                    _ => out.push_str(&regex::escape(&ch.to_string())),
                }
            }
            out.push('$');
            Ok(regex::Regex::new(&out)?)
        })
        .collect()
}

fn excluded(path: &str, patterns: &[regex::Regex]) -> bool {
    patterns
        .iter()
        .any(|p| p.is_match(path) || path.split('/').any(|c| p.is_match(c)))
}

fn valid_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0', ':']) {
        bail!("Unsupported file name in sync: {name:?}");
    }
    Ok(())
}

async fn canonical(location: &Location) -> Result<String> {
    Ok(match &location.backend {
        Backend::Local => tokio::fs::canonicalize(&location.path)
            .await?
            .to_str()
            .ok_or_else(|| anyhow!("Sync requires UTF-8 paths."))?
            .to_owned(),
        Backend::Remote { sftp, .. } => sftp.canonicalize(&location.path).await?,
    })
}

async fn validate_locations(source: &Location, destination: &Location) -> Result<()> {
    let same = match (&source.backend, &destination.backend) {
        (Backend::Local, Backend::Local) => true,
        (Backend::Remote { client: a, .. }, Backend::Remote { client: b, .. }) => {
            std::sync::Arc::ptr_eq(a, b) || a.label == b.label
        }
        _ => false,
    };
    if same {
        let a = canonical(source).await?;
        let b = canonical(destination).await?;
        let overlaps = if source.backend.is_local() {
            std::path::Path::new(&a).starts_with(&b) || std::path::Path::new(&b).starts_with(&a)
        } else {
            let a = format!("{}/", a.trim_end_matches('/'));
            let b = format!("{}/", b.trim_end_matches('/'));
            a.starts_with(&b) || b.starts_with(&a)
        };
        if overlaps {
            bail!("Choose separate folders. Sync cannot use the same folder or a nested folder.");
        }
    }
    Ok(())
}

async fn file_hash(b: &Backend, path: &str, cancel: &AtomicBool) -> Result<([u8; 32], u64)> {
    let mut reader = super::open_read(b, path).await?;
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut buffer = vec![0; 256 * 1024];
    loop {
        check_cancel(cancel)?;
        let n = tokio::time::timeout(Duration::from_secs(30), reader.read(&mut buffer))
            .await
            .map_err(|_| anyhow!("File read timed out."))??;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        size += n as u64;
    }
    Ok((hash.finalize().into(), size))
}

async fn snapshot(
    location: &Location,
    patterns: &[regex::Regex],
    cancel: &AtomicBool,
) -> Result<Snapshot> {
    let mut out = BTreeMap::new();
    let mut stack = vec![(String::new(), location.path.clone(), 0)];
    let mut visited = 0;
    while let Some((prefix, directory, depth)) = stack.pop() {
        check_cancel(cancel)?;
        if depth > 64 {
            bail!("Folder nesting exceeds 64 levels.");
        }
        // The general browser tolerates unreadable entries; sync must fail closed.
        let mut entries = Vec::new();
        match &location.backend {
            Backend::Local => {
                let mut rd = tokio::fs::read_dir(&directory).await?;
                while let Some(entry) = rd.next_entry().await? {
                    let name = entry
                        .file_name()
                        .into_string()
                        .map_err(|_| anyhow!("Sync requires UTF-8 file names."))?;
                    let meta = tokio::fs::symlink_metadata(entry.path()).await?;
                    entries.push((
                        name,
                        meta.is_dir(),
                        meta.is_file(),
                        meta.file_type().is_symlink(),
                    ));
                }
            }
            Backend::Remote { sftp, .. } => {
                for entry in sftp.read_dir(&directory).await? {
                    let name = entry.file_name();
                    if name == "." || name == ".." {
                        continue;
                    }
                    let kind = entry.file_type();
                    entries.push((name, kind.is_dir(), kind.is_file(), kind.is_symlink()));
                }
            }
        }
        for (name, is_dir, is_file, is_link) in entries {
            check_cancel(cancel)?;
            visited += 1;
            if visited > 20_000 {
                bail!(
                    "Comparison exceeds 20,000 entries. Choose a smaller folder or add exclusions."
                );
            }
            valid_name(&name)?;
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if excluded(&relative, patterns) {
                continue;
            }
            if is_link || (!is_dir && !is_file) {
                bail!("Cannot sync link or special file: {relative}. Add it to exclusions.");
            }
            let path = location.backend.join(&directory, &name);
            let (hash, size) = if is_dir {
                ([0; 32], 0)
            } else {
                file_hash(&location.backend, &path, cancel).await?
            };
            let permissions = match &location.backend {
                #[cfg(unix)]
                Backend::Local => {
                    use std::os::unix::fs::PermissionsExt;
                    Some(
                        tokio::fs::symlink_metadata(&path)
                            .await?
                            .permissions()
                            .mode()
                            & 0o777,
                    )
                }
                #[cfg(not(unix))]
                Backend::Local => None,
                Backend::Remote { sftp, .. } => sftp
                    .symlink_metadata(&path)
                    .await?
                    .permissions
                    .map(|m| m & 0o777),
            };
            out.insert(
                relative.clone(),
                Stamp {
                    directory: is_dir,
                    hash,
                    size,
                    permissions,
                },
            );
            if is_dir {
                stack.push((relative, path, depth + 1));
            }
        }
    }
    Ok(out)
}

pub async fn compare(
    source: Location,
    destination: Location,
    mode: Mode,
    exclude: String,
    cancel: &AtomicBool,
) -> Result<Plan> {
    tokio::time::timeout(Duration::from_secs(120), async {
        let source = Location {
            path: canonical(&source).await?,
            ..source
        };
        let destination = Location {
            path: canonical(&destination).await?,
            ..destination
        };
        validate_locations(&source, &destination).await?;
        let patterns = exclusions(&exclude)?;
        let a = snapshot(&source, &patterns, cancel).await?;
        let b = snapshot(&destination, &patterns, cancel).await?;
        // Refuse case aliases before writing across filesystems with different case rules.
        let mut folded = BTreeMap::new();
        for path in a.keys().chain(b.keys()) {
            if let Some(previous) = folded.insert(path.to_lowercase(), path) {
                if previous != path { bail!("File names differ only by case: {previous} and {path}. Rename or exclude one before syncing."); }
            }
        }
        let mut changes = Vec::new();
        for (path, entry) in &a {
            match b.get(path) {
                Some(other) if entry.directory != other.directory => {
                    changes.push(Change::Conflict(path.clone()))
                }
                None if entry.directory => changes.push(Change::CreateFolder(path.clone())),
                other if !entry.directory && other.is_none_or(|other| other.hash != entry.hash || other.size != entry.size) => {
                    changes.push(Change::CopyFile {
                        path: path.clone(),
                        replace: other.is_some(),
                        size: entry.size,
                    })
                }
                _ => {}
            }
        }
        if mode == Mode::Mirror {
            for (path, entry) in &b {
                if !entry.directory && !a.contains_key(path) {
                    changes.push(Change::DeleteFile(path.clone()));
                }
            }
        }
        Ok(Plan {
            source,
            destination,
            changes,
            mode,
            exclusions: exclude,
            source_snapshot: a,
            destination_snapshot: b,
        })
    })
    .await
    .map_err(|_| anyhow!("Comparison timed out after two minutes. Choose a smaller folder."))?
}

fn full_path(location: &Location, relative: &str) -> String {
    relative
        .split('/')
        .fold(location.path.clone(), |dir, name| {
            location.backend.join(&dir, name)
        })
}

/// Files are staged beside the destination. An old file is kept until the new file is complete.
async fn copy_file(plan: &Plan, path: &str, replace: bool, cancel: &AtomicBool) -> Result<()> {
    let dst = &plan.destination.backend;
    let target = full_path(&plan.destination, path);
    let parent = dst
        .parent(&target)
        .ok_or_else(|| anyhow!("Destination has no parent folder."))?;
    let temporary = dst.join(&parent, &format!(".ferrum-sync-{}", uuid::Uuid::new_v4()));
    let backup = format!("{temporary}.backup");
    let copy_result: Result<()> = async {
        let mut reader =
            super::open_read(&plan.source.backend, &full_path(&plan.source, path)).await?;
        let mut writer = super::open_write(dst, &temporary).await?;
        let mut hash = Sha256::new();
        let mut buffer = vec![0; 256 * 1024];
        loop {
            check_cancel(cancel)?;
            let n = tokio::time::timeout(Duration::from_secs(30), reader.read(&mut buffer))
                .await
                .map_err(|_| anyhow!("File read timed out."))??;
            if n == 0 {
                break;
            }
            tokio::time::timeout(Duration::from_secs(30), writer.write_all(&buffer[..n]))
                .await
                .map_err(|_| anyhow!("File write timed out."))??;
            hash.update(&buffer[..n]);
        }
        writer.flush().await?;
        writer.shutdown().await?;
        drop(writer);
        let actual: [u8; 32] = hash.finalize().into();
        if actual != plan.source_snapshot[path].hash {
            bail!("Source changed while copying {path}. Compare again.");
        }
        if let Some(mode) = plan.source_snapshot[path].permissions {
            super::chmod(dst, &temporary, mode).await?;
        }
        check_cancel(cancel)?;
        // No cancellation between moving the old file aside and committing its replacement.
        if replace {
            super::rename(dst, &target, &backup).await?;
        }
        if let Err(error) = super::rename(dst, &temporary, &target).await {
            if replace {
                super::rename(dst, &backup, &target)
                    .await
                    .map_err(|restore| {
                        anyhow!("{error}; restore failed: {restore}. Original file is at {backup}")
                    })?;
            }
            return Err(error);
        }
        if replace {
            super::remove(dst, &backup, false).await?;
        }
        Ok(())
    }
    .await;
    if copy_result.is_err() {
        // A rename may have committed already. Never remove the destination on failure.
        if let Err(cleanup) = super::remove(dst, &temporary, false).await {
            log::debug!("Sync temporary file cleanup: {cleanup}");
        }
    }
    copy_result
}

pub async fn apply(
    plan: &Plan,
    cancel: &AtomicBool,
    progress: impl Fn(usize, usize),
) -> Result<usize> {
    if plan.conflicts() {
        bail!("Resolve file/folder conflicts before syncing.");
    }
    // Re-read both trees before changing anything: a preview is not permission to apply stale data.
    let fresh = compare(
        plan.source.clone(),
        plan.destination.clone(),
        plan.mode,
        plan.exclusions.clone(),
        cancel,
    )
    .await?;
    if fresh.source_snapshot != plan.source_snapshot
        || fresh.destination_snapshot != plan.destination_snapshot
    {
        bail!("Folders changed since the preview. Compare again before syncing.");
    }
    let mut completed = 0;
    for change in &plan.changes {
        check_cancel(cancel)?;
        match change {
            Change::CreateFolder(path) => {
                super::mkdir(
                    &plan.destination.backend,
                    &full_path(&plan.destination, path),
                )
                .await?
            }
            Change::CopyFile { path, replace, .. } => {
                copy_file(plan, path, *replace, cancel).await?
            }
            Change::DeleteFile(path) => {
                super::remove(
                    &plan.destination.backend,
                    &full_path(&plan.destination, path),
                    false,
                )
                .await?
            }
            Change::Conflict(_) => bail!("Resolve file/folder conflicts before syncing."),
        }
        completed += 1;
        progress(completed, plan.changes.len());
    }
    Ok(completed)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Folders {
        _root: tempfile::TempDir,
        source: Location,
        destination: Location,
    }
    impl Folders {
        async fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source");
            let destination = root.path().join("destination");
            tokio::fs::create_dir(&source).await.unwrap();
            tokio::fs::create_dir(&destination).await.unwrap();
            Self {
                _root: root,
                source: Location {
                    backend: Backend::Local,
                    path: source.to_str().unwrap().into(),
                },
                destination: Location {
                    backend: Backend::Local,
                    path: destination.to_str().unwrap().into(),
                },
            }
        }
        async fn put(&self, source: bool, path: &str, content: &[u8]) {
            let location = if source {
                &self.source
            } else {
                &self.destination
            };
            let full = std::path::PathBuf::from(full_path(location, path));
            tokio::fs::create_dir_all(full.parent().unwrap())
                .await
                .unwrap();
            tokio::fs::write(full, content).await.unwrap();
        }
        async fn plan(&self, mode: Mode, exclude: &str) -> Plan {
            compare(
                self.source.clone(),
                self.destination.clone(),
                mode,
                exclude.into(),
                &AtomicBool::new(false),
            )
            .await
            .unwrap()
        }
        async fn contents(&self, path: &str) -> Vec<u8> {
            tokio::fs::read(full_path(&self.destination, path))
                .await
                .unwrap()
        }
    }

    #[tokio::test]
    async fn update_previews_and_applies_nested_files_and_same_size_changes() {
        let f = Folders::new().await;
        f.put(true, "nested/deploy.sh", b"deploy").await;
        f.put(true, "changed.txt", b"new").await;
        f.put(false, "changed.txt", b"old").await;
        f.put(true, "unchanged.txt", b"same").await;
        f.put(false, "unchanged.txt", b"same").await;
        f.put(false, "keep.txt", b"keep").await;
        let plan = f.plan(Mode::Update, "").await;
        assert_eq!(
            plan.changes,
            vec![
                Change::CopyFile {
                    path: "changed.txt".into(),
                    replace: true,
                    size: 3
                },
                Change::CreateFolder("nested".into()),
                Change::CopyFile {
                    path: "nested/deploy.sh".into(),
                    replace: false,
                    size: 6
                },
            ]
        );
        assert_eq!(
            f.contents("changed.txt").await,
            b"old",
            "preview must not write"
        );
        let progress = std::sync::Mutex::new(Vec::new());
        assert_eq!(
            apply(&plan, &AtomicBool::new(false), |done, total| progress
                .lock()
                .unwrap()
                .push((done, total)))
            .await
            .unwrap(),
            3
        );
        assert_eq!(*progress.lock().unwrap(), vec![(1, 3), (2, 3), (3, 3)]);
        assert_eq!(f.contents("changed.txt").await, b"new");
        assert_eq!(f.contents("nested/deploy.sh").await, b"deploy");
        assert_eq!(f.contents("keep.txt").await, b"keep");
        assert!(f.plan(Mode::Update, "").await.changes.is_empty());
        let names: Vec<_> = super::super::list(&Backend::Local, &f.destination.path)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert!(!names.iter().any(|name| name.starts_with(".ferrum-sync-")));
    }

    #[tokio::test]
    async fn mirror_deletes_only_unexcluded_extra_files_and_keeps_directories() {
        let f = Folders::new().await;
        f.put(true, "release.txt", b"release").await;
        f.put(false, "old.txt", b"old").await;
        f.put(false, "logs/server.log", b"log").await;
        f.put(false, ".git/config", b"private").await;
        tokio::fs::create_dir(full_path(&f.destination, "empty"))
            .await
            .unwrap();
        let plan = f.plan(Mode::Mirror, ".git, *.log").await;
        assert_eq!(plan.deletions(), 1);
        assert_eq!(
            plan.changes.last(),
            Some(&Change::DeleteFile("old.txt".into()))
        );
        apply(&plan, &AtomicBool::new(false), |_, _| {})
            .await
            .unwrap();
        assert!(
            !tokio::fs::try_exists(full_path(&f.destination, "old.txt"))
                .await
                .unwrap()
        );
        assert_eq!(f.contents("logs/server.log").await, b"log");
        assert_eq!(f.contents(".git/config").await, b"private");
        assert!(
            tokio::fs::metadata(full_path(&f.destination, "empty"))
                .await
                .unwrap()
                .is_dir()
        );
    }

    #[tokio::test]
    async fn stale_previews_fail_before_any_write_for_changes_on_either_side() {
        for changed_source in [true, false] {
            let f = Folders::new().await;
            f.put(true, "a.txt", b"new").await;
            f.put(false, "a.txt", b"old").await;
            f.put(true, "b.txt", b"new").await;
            let plan = f.plan(Mode::Update, "").await;
            f.put(changed_source, "a.txt", b"edited").await;
            let error = apply(&plan, &AtomicBool::new(false), |_, _| {})
                .await
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("Folders changed since the preview"),
                "{error}"
            );
            assert!(
                !tokio::fs::try_exists(full_path(&f.destination, "b.txt"))
                    .await
                    .unwrap()
            );
            assert_eq!(
                f.contents("a.txt").await,
                if changed_source {
                    b"old".as_slice()
                } else {
                    b"edited".as_slice()
                }
            );
        }
    }

    #[tokio::test]
    async fn conflicts_are_visible_and_block_all_changes() {
        let f = Folders::new().await;
        f.put(true, "a.txt", b"new").await;
        f.put(true, "conflict/child", b"child").await;
        f.put(false, "conflict", b"file").await;
        let plan = f.plan(Mode::Mirror, "").await;
        assert!(plan.changes.contains(&Change::Conflict("conflict".into())));
        assert!(
            apply(&plan, &AtomicBool::new(false), |_, _| {})
                .await
                .unwrap_err()
                .to_string()
                .contains("conflicts")
        );
        assert!(
            !tokio::fs::try_exists(full_path(&f.destination, "a.txt"))
                .await
                .unwrap()
        );
        assert_eq!(f.contents("conflict").await, b"file");
    }

    #[tokio::test]
    async fn rejects_equal_and_nested_locations_in_both_directions() {
        let f = Folders::new().await;
        tokio::fs::create_dir(full_path(&f.source, "nested"))
            .await
            .unwrap();
        for (a, b) in [
            (f.source.clone(), f.source.clone()),
            (
                f.source.clone(),
                Location {
                    backend: Backend::Local,
                    path: full_path(&f.source, "nested"),
                },
            ),
            (
                Location {
                    backend: Backend::Local,
                    path: full_path(&f.source, "nested"),
                },
                f.source.clone(),
            ),
        ] {
            let error = compare(a, b, Mode::Update, "".into(), &AtomicBool::new(false))
                .await
                .err()
                .unwrap();
            assert!(
                error.to_string().contains("Choose separate folders"),
                "{error}"
            );
        }
    }

    #[tokio::test]
    async fn cancellation_stops_between_files_and_leaves_completed_files() {
        let f = Folders::new().await;
        f.put(true, "a.txt", b"first").await;
        f.put(true, "b.txt", b"second").await;
        let plan = f.plan(Mode::Update, "").await;
        let cancel = AtomicBool::new(false);
        let result = apply(&plan, &cancel, |_, _| cancel.store(true, Ordering::Relaxed)).await;
        assert!(result.unwrap_err().to_string().contains("cancelled"));
        assert_eq!(f.contents("a.txt").await, b"first");
        assert!(
            !tokio::fs::try_exists(full_path(&f.destination, "b.txt"))
                .await
                .unwrap()
        );
        assert!(
            compare(f.source, f.destination, Mode::Update, "".into(), &cancel)
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("cancelled")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn links_are_rejected_unless_excluded_and_never_followed() {
        let f = Folders::new().await;
        f.put(true, "a.txt", b"safe").await;
        std::os::unix::fs::symlink(&f.source.path, full_path(&f.source, "loop")).unwrap();
        let error = compare(
            f.source.clone(),
            f.destination.clone(),
            Mode::Update,
            "".into(),
            &AtomicBool::new(false),
        )
        .await
        .err()
        .unwrap();
        assert!(error.to_string().contains("Cannot sync link"), "{error}");
        let plan = f.plan(Mode::Update, "loop").await;
        assert_eq!(plan.changes.len(), 1);
        apply(&plan, &AtomicBool::new(false), |_, _| {})
            .await
            .unwrap();
        assert_eq!(f.contents("a.txt").await, b"safe");
        assert!(
            !tokio::fs::try_exists(full_path(&f.destination, "loop"))
                .await
                .unwrap()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn copied_executable_keeps_its_permission_bits() {
        use std::os::unix::fs::PermissionsExt;
        let f = Folders::new().await;
        f.put(true, "deploy.sh", b"echo ready").await;
        tokio::fs::set_permissions(
            full_path(&f.source, "deploy.sh"),
            std::fs::Permissions::from_mode(0o750),
        )
        .await
        .unwrap();
        let plan = f.plan(Mode::Update, "").await;
        apply(&plan, &AtomicBool::new(false), |_, _| {})
            .await
            .unwrap();
        let metadata = tokio::fs::metadata(full_path(&f.destination, "deploy.sh"))
            .await
            .unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o750);
    }

    #[tokio::test]
    async fn rejects_case_aliases_across_source_and_destination() {
        let f = Folders::new().await;
        f.put(true, "README", b"new").await;
        f.put(false, "readme", b"old").await;
        let error = compare(
            f.source.clone(),
            f.destination.clone(),
            Mode::Update,
            "".into(),
            &AtomicBool::new(false),
        )
        .await
        .err()
        .unwrap();
        assert!(error.to_string().contains("differ only by case"), "{error}");
        assert_eq!(f.contents("readme").await, b"old");
    }

    #[tokio::test]
    async fn missing_folder_fails_comparison_and_matching_folders_need_no_changes() {
        let f = Folders::new().await;
        let plan = f.plan(Mode::Mirror, "").await;
        assert!(plan.changes.is_empty());
        assert_eq!(
            apply(&plan, &AtomicBool::new(false), |_, _| {})
                .await
                .unwrap(),
            0
        );
        tokio::fs::remove_dir(&f.destination.path).await.unwrap();
        assert!(
            compare(
                f.source,
                f.destination,
                Mode::Mirror,
                "".into(),
                &AtomicBool::new(false)
            )
            .await
            .is_err()
        );
    }

    #[test]
    fn unsafe_names_and_exclusion_patterns() {
        for name in ["..", "../escape", "a/b", "a\\b", "C:drive", ""] {
            assert!(valid_name(name).is_err());
        }
        let patterns = exclusions(".git, build/*, *.log, secret?.txt").unwrap();
        for path in [
            ".git/config",
            "build/release/app",
            "logs/app.log",
            "secrets.txt",
        ] {
            assert!(excluded(path, &patterns), "{path}");
        }
        assert!(!excluded("catalog.rs", &patterns));
    }
}
