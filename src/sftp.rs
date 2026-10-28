//! SFTP / local filesystem backends and the transfer engine.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow};
use parking_lot::Mutex;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::FileAttributes;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::conn::ssh::SshClient;

#[derive(Clone)]
pub enum Backend {
    Local,
    Remote {
        sftp: Arc<SftpSession>,
        client: Arc<SshClient>,
        label: String,
    },
}

impl Backend {
    pub fn label(&self) -> String {
        match self {
            Backend::Local => "Local".into(),
            Backend::Remote { label, .. } => label.clone(),
        }
    }
    pub fn is_local(&self) -> bool {
        matches!(self, Backend::Local)
    }
    pub fn join(&self, dir: &str, name: &str) -> String {
        match self {
            Backend::Local => Path::new(dir).join(name).to_string_lossy().to_string(),
            Backend::Remote { .. } => {
                if dir.ends_with('/') {
                    format!("{dir}{name}")
                } else {
                    format!("{dir}/{name}")
                }
            }
        }
    }
    pub fn parent(&self, path: &str) -> Option<String> {
        match self {
            Backend::Local => Path::new(path)
                .parent()
                .map(|p| p.to_string_lossy().to_string()),
            Backend::Remote { .. } => {
                if path == "/" || path.is_empty() {
                    return None;
                }
                let t = path.trim_end_matches('/');
                match t.rfind('/') {
                    Some(0) => Some("/".into()),
                    Some(i) => Some(t[..i].to_string()),
                    None => Some("/".into()),
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
    pub mtime: Option<i64>,
    pub perms: Option<u32>,
    pub owner: String,
}

pub enum SftpEvent {
    Connected {
        pane: u64,
        backend: Backend,
        home: String,
    },
    ConnectFailed {
        pane: u64,
        error: String,
    },
    Listing {
        pane: u64,
        path: String,
        result: Result<Vec<FileEntry>, String>,
    },
    OpDone {
        pane: u64,
        result: Result<String, String>,
    },
    EditReady {
        local: PathBuf,
        remote: String,
        pane: u64,
    },
}

pub fn perms_string(mode: u32, is_dir: bool) -> String {
    let mut s = String::with_capacity(10);
    s.push(if is_dir { 'd' } else { '-' });
    for shift in [6, 3, 0] {
        let b = (mode >> shift) & 7;
        s.push(if b & 4 != 0 { 'r' } else { '-' });
        s.push(if b & 2 != 0 { 'w' } else { '-' });
        s.push(if b & 1 != 0 { 'x' } else { '-' });
    }
    s
}

pub fn human_size(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

pub async fn home_dir(b: &Backend) -> String {
    match b {
        Backend::Local => directories::UserDirs::new()
            .map(|u| u.home_dir().to_string_lossy().to_string())
            .unwrap_or_else(|| "/".into()),
        Backend::Remote { sftp, .. } => sftp.canonicalize(".").await.unwrap_or_else(|_| "/".into()),
    }
}

pub async fn list(b: &Backend, path: &str) -> Result<Vec<FileEntry>> {
    let mut out = vec![];
    match b {
        Backend::Local => {
            let mut rd = tokio::fs::read_dir(path)
                .await
                .with_context(|| format!("reading {path}"))?;
            while let Some(e) = rd.next_entry().await? {
                let lmeta = tokio::fs::symlink_metadata(e.path()).await.ok();
                let meta = tokio::fs::metadata(e.path()).await.ok().or(lmeta.clone());
                let Some(meta) = meta else { continue };
                #[cfg(unix)]
                let perms = {
                    use std::os::unix::fs::PermissionsExt;
                    Some(meta.permissions().mode() & 0o7777)
                };
                #[cfg(not(unix))]
                let perms = None;
                out.push(FileEntry {
                    name: e.file_name().to_string_lossy().to_string(),
                    path: e.path().to_string_lossy().to_string(),
                    is_dir: meta.is_dir(),
                    is_link: lmeta.map(|m| m.file_type().is_symlink()).unwrap_or(false),
                    size: meta.len(),
                    mtime: meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64),
                    perms,
                    owner: String::new(),
                });
            }
        }
        Backend::Remote { sftp, .. } => {
            for e in sftp.read_dir(path).await.map_err(|e| anyhow!("{e}"))? {
                let name = e.file_name();
                if name == "." || name == ".." {
                    continue;
                }
                let meta: FileAttributes = e.metadata();
                let full = b.join(path, &name);
                let is_link = e.file_type().is_symlink();
                let is_dir = if is_link {
                    sftp.metadata(full.clone())
                        .await
                        .map(|m| m.is_dir())
                        .unwrap_or(false)
                } else {
                    e.file_type().is_dir()
                };
                out.push(FileEntry {
                    name,
                    path: full,
                    is_dir,
                    is_link,
                    size: meta.size.unwrap_or(0),
                    mtime: meta.mtime.map(|t| t as i64),
                    perms: meta.permissions.map(|p| p & 0o7777),
                    owner: meta
                        .user
                        .clone()
                        .or(meta.uid.map(|u| u.to_string()))
                        .unwrap_or_default(),
                });
            }
        }
    }
    Ok(out)
}

pub async fn mkdir(b: &Backend, path: &str) -> Result<()> {
    match b {
        Backend::Local => Ok(tokio::fs::create_dir_all(path).await?),
        Backend::Remote { sftp, .. } => {
            Ok(sftp.create_dir(path).await.map_err(|e| anyhow!("{e}"))?)
        }
    }
}

pub async fn touch(b: &Backend, path: &str) -> Result<()> {
    match b {
        Backend::Local => {
            tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .await?;
            Ok(())
        }
        Backend::Remote { sftp, .. } => {
            let f = sftp.create(path).await.map_err(|e| anyhow!("{e}"))?;
            drop(f);
            Ok(())
        }
    }
}

pub async fn rename(b: &Backend, from: &str, to: &str) -> Result<()> {
    match b {
        Backend::Local => Ok(tokio::fs::rename(from, to).await?),
        Backend::Remote { sftp, .. } => {
            Ok(sftp.rename(from, to).await.map_err(|e| anyhow!("{e}"))?)
        }
    }
}

pub async fn chmod(b: &Backend, path: &str, mode: u32) -> Result<()> {
    match b {
        Backend::Local => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).await?;
            }
            let _ = (path, mode);
            Ok(())
        }
        Backend::Remote { sftp, .. } => {
            let mut attrs = FileAttributes::empty();
            attrs.permissions = Some(mode);
            sftp.set_metadata(path, attrs)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            Ok(())
        }
    }
}

pub fn remove<'a>(
    b: &'a Backend,
    path: &'a str,
    is_dir: bool,
) -> std::pin::Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        match b {
            Backend::Local => {
                if is_dir {
                    tokio::fs::remove_dir_all(path).await?
                } else {
                    tokio::fs::remove_file(path).await?
                }
            }
            Backend::Remote { sftp, .. } => {
                if is_dir {
                    for e in list(b, path).await? {
                        if e.is_dir && !e.is_link {
                            remove(b, &e.path, true).await?;
                        } else {
                            sftp.remove_file(e.path.clone())
                                .await
                                .map_err(|e| anyhow!("{e}"))?;
                        }
                    }
                    sftp.remove_dir(path).await.map_err(|e| anyhow!("{e}"))?;
                } else {
                    sftp.remove_file(path).await.map_err(|e| anyhow!("{e}"))?;
                }
            }
        }
        Ok(())
    })
}

async fn open_read(b: &Backend, path: &str) -> Result<Box<dyn AsyncRead + Unpin + Send>> {
    Ok(match b {
        Backend::Local => Box::new(
            tokio::fs::File::open(path)
                .await
                .with_context(|| format!("opening {path}"))?,
        ),
        Backend::Remote { sftp, .. } => Box::new(
            sftp.open(path)
                .await
                .map_err(|e| anyhow!("open {path}: {e}"))?,
        ),
    })
}

async fn open_write(b: &Backend, path: &str) -> Result<Box<dyn AsyncWrite + Unpin + Send>> {
    Ok(match b {
        Backend::Local => Box::new(
            tokio::fs::File::create(path)
                .await
                .with_context(|| format!("creating {path}"))?,
        ),
        Backend::Remote { sftp, .. } => Box::new(
            sftp.create(path)
                .await
                .map_err(|e| anyhow!("create {path}: {e}"))?,
        ),
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TransferStatus {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

pub struct Transfer {
    pub id: u64,
    pub name: String,
    pub from: String,
    pub to: String,
    pub total: AtomicU64,
    pub done: AtomicU64,
    pub cancel: AtomicBool,
    pub status: Mutex<TransferStatus>,
    pub error: Mutex<Option<String>>,
    pub started: std::time::Instant,
}

pub type Transfers = Arc<Mutex<Vec<Arc<Transfer>>>>;

async fn total_size(b: &Backend, path: &str, is_dir: bool) -> u64 {
    if !is_dir {
        return match b {
            Backend::Local => tokio::fs::metadata(path)
                .await
                .map(|m| m.len())
                .unwrap_or(0),
            Backend::Remote { sftp, .. } => sftp
                .metadata(path)
                .await
                .ok()
                .and_then(|m| m.size)
                .unwrap_or(0),
        };
    }
    let mut total = 0;
    let mut stack = vec![path.to_string()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = list(b, &dir).await {
            for e in entries {
                if e.is_dir && !e.is_link {
                    stack.push(e.path);
                } else {
                    total += e.size;
                }
            }
        }
    }
    total
}

fn copy_tree<'a>(
    src: &'a Backend,
    src_path: &'a str,
    is_dir: bool,
    dst: &'a Backend,
    dst_path: &'a str,
    t: &'a Transfer,
) -> std::pin::Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        if t.cancel.load(Ordering::Relaxed) {
            return Err(anyhow!("cancelled"));
        }
        if is_dir {
            let _ = mkdir(dst, dst_path).await;
            for e in list(src, src_path).await? {
                let d = dst.join(dst_path, &e.name);
                copy_tree(src, &e.path, e.is_dir && !e.is_link, dst, &d, t).await?;
            }
            return Ok(());
        }
        let mut r = open_read(src, src_path).await?;
        let mut w = open_write(dst, dst_path).await?;
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            if t.cancel.load(Ordering::Relaxed) {
                return Err(anyhow!("cancelled"));
            }
            let n = r.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            w.write_all(&buf[..n]).await?;
            t.done.fetch_add(n as u64, Ordering::Relaxed);
        }
        w.shutdown().await.ok();
        Ok(())
    })
}

static TRANSFER_ID: AtomicU64 = AtomicU64::new(1);

/// Queue and run a copy between any two backends (local↔remote, remote↔remote).
pub fn start_transfer(
    rt: &tokio::runtime::Handle,
    transfers: &Transfers,
    src: Backend,
    entry: FileEntry,
    dst: Backend,
    dst_dir: String,
    ctx: egui::Context,
    on_done: impl FnOnce(Result<(), String>) + Send + 'static,
) -> u64 {
    let dst_path = dst.join(&dst_dir, &entry.name);
    let t = Arc::new(Transfer {
        id: TRANSFER_ID.fetch_add(1, Ordering::Relaxed),
        name: entry.name.clone(),
        from: format!("{}:{}", src.label(), entry.path),
        to: format!("{}:{}", dst.label(), dst_path),
        total: AtomicU64::new(entry.size),
        done: AtomicU64::new(0),
        cancel: AtomicBool::new(false),
        status: Mutex::new(TransferStatus::Queued),
        error: Mutex::new(None),
        started: std::time::Instant::now(),
    });
    let id = t.id;
    transfers.lock().push(t.clone());
    rt.spawn(async move {
        *t.status.lock() = TransferStatus::Running;
        let total = total_size(&src, &entry.path, entry.is_dir).await;
        t.total.store(total, Ordering::Relaxed);
        let res = copy_tree(
            &src,
            &entry.path,
            entry.is_dir && !entry.is_link,
            &dst,
            &dst_path,
            &t,
        )
        .await;
        let status = match &res {
            Ok(_) => TransferStatus::Done,
            Err(_) if t.cancel.load(Ordering::Relaxed) => TransferStatus::Cancelled,
            Err(_) => TransferStatus::Failed,
        };
        if let Err(e) = &res {
            *t.error.lock() = Some(format!("{e:#}"));
        }
        *t.status.lock() = status;
        on_done(res.map_err(|e| format!("{e:#}")));
        ctx.request_repaint();
    });
    id
}

/// Download a remote file to a temp dir for editing.
pub async fn download_for_edit(b: &Backend, remote: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir()
        .join("ferrum-edit")
        .join(uuid::Uuid::new_v4().to_string());
    tokio::fs::create_dir_all(&dir).await?;
    let name = remote.rsplit('/').next().unwrap_or("file");
    let local = dir.join(name);
    let mut r = open_read(b, remote).await?;
    let mut w = tokio::fs::File::create(&local).await?;
    tokio::io::copy(&mut r, &mut w).await?;
    w.flush().await?;
    Ok(local)
}

pub async fn upload_file(b: &Backend, local: &Path, remote: &str) -> Result<()> {
    let mut r = tokio::fs::File::open(local).await?;
    let mut w = open_write(b, remote).await?;
    tokio::io::copy(&mut r, &mut w).await?;
    w.shutdown().await.ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_permissions_and_sizes() {
        assert_eq!(perms_string(0o755, true), "drwxr-xr-x");
        assert_eq!(perms_string(0o640, false), "-rw-r-----");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 KB");
    }

    #[test]
    fn remote_path_helpers() {
        let b = Backend::Local;
        assert!(b.join("/tmp", "x").ends_with("x"));
    }

    #[tokio::test]
    async fn retrying_a_failed_transfer_uses_a_new_job_and_copies_the_original_file() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("release.txt");
        let destination = directory.path().join("destination");
        tokio::fs::create_dir(&destination).await.unwrap();
        let entry = FileEntry {
            name: "release.txt".into(),
            path: source.to_string_lossy().into(),
            is_dir: false,
            is_link: false,
            size: 5,
            mtime: None,
            perms: None,
            owner: String::new(),
        };
        let transfers: Transfers = Default::default();
        let (ctx, handle) = (egui::Context::default(), tokio::runtime::Handle::current());
        let (tx, rx) = tokio::sync::oneshot::channel();
        let failed_id = start_transfer(
            &handle,
            &transfers,
            Backend::Local,
            entry.clone(),
            Backend::Local,
            destination.to_string_lossy().into(),
            ctx.clone(),
            move |outcome| {
                tx.send(outcome).unwrap();
            },
        );
        let failed = tokio::time::timeout(std::time::Duration::from_secs(5), rx)
            .await
            .unwrap()
            .unwrap();
        assert!(failed.is_err());
        assert_eq!(*transfers.lock()[0].status.lock(), TransferStatus::Failed);
        tokio::fs::write(&source, b"ready").await.unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let retry_id = start_transfer(
            &handle,
            &transfers,
            Backend::Local,
            entry,
            Backend::Local,
            destination.to_string_lossy().into(),
            ctx,
            move |outcome| {
                tx.send(outcome).unwrap();
            },
        );
        tokio::time::timeout(std::time::Duration::from_secs(5), rx)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_ne!(failed_id, retry_id);
        assert_eq!(
            tokio::fs::read(destination.join("release.txt"))
                .await
                .unwrap(),
            b"ready"
        );
        let jobs = transfers.lock();
        assert_eq!(jobs.len(), 2);
        assert_eq!(*jobs[1].status.lock(), TransferStatus::Done);
        assert_eq!(jobs[1].done.load(Ordering::Relaxed), 5);
    }

    #[tokio::test]
    async fn local_copy_tree_copies_nested_files() {
        let base = std::env::temp_dir().join(format!("ferrum-test-{}", uuid::Uuid::new_v4()));
        let src = base.join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), b"hello").unwrap();
        std::fs::write(src.join("sub/b.txt"), b"world!").unwrap();
        let dst = base.join("dst");
        let t = Transfer {
            id: 0,
            name: "src".into(),
            from: String::new(),
            to: String::new(),
            total: AtomicU64::new(0),
            done: AtomicU64::new(0),
            cancel: AtomicBool::new(false),
            status: Mutex::new(TransferStatus::Running),
            error: Mutex::new(None),
            started: std::time::Instant::now(),
        };
        copy_tree(
            &Backend::Local,
            src.to_str().unwrap(),
            true,
            &Backend::Local,
            dst.to_str().unwrap(),
            &t,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(dst.join("sub/b.txt")).unwrap(), b"world!");
        assert_eq!(t.done.load(Ordering::Relaxed), 11);
        std::fs::remove_dir_all(base).ok();
    }
}
