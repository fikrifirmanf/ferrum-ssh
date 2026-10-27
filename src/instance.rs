//! Single-instance guard. The first launch holds an OS file lock and listens on a loopback
//! port; later launches hand their deep link to it, ask it to focus, and exit.
//!
//! The lock dies with the process, so a crash never leaves a stale lock. Requests must carry
//! a random token from a file in the user's data directory, so other local users cannot
//! open tabs in someone else's Ferrum.

use std::fs::{self, File, TryLockError};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};

const LOCK_FILE: &str = "instance.lock";
const ENDPOINT_FILE: &str = "instance.endpoint";
const MAX_REQUEST: u64 = 4096;
const IO_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a second launch waits for a starting first instance to publish its endpoint.
const HANDOFF_WAIT: Duration = Duration::from_secs(3);

pub enum Claim {
    /// This process is the only instance; keep the guard alive for the whole run.
    Primary(Guard),
    /// Another instance is running and received the request; this process should exit.
    Handed,
    /// Locking is unsupported here (e.g. some network filesystems); run without a guard.
    Unguarded,
}

pub struct Guard {
    _lock: File,
    listener: TcpListener,
    token: String,
}

pub fn claim(dir: &Path, deep_link: Option<&str>) -> Claim {
    let lock = match File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(LOCK_FILE))
    {
        Ok(file) => file,
        Err(error) => {
            log::warn!("Single-instance lock unavailable: {error}");
            return Claim::Unguarded;
        }
    };
    match lock.try_lock() {
        Ok(()) => match Guard::start(lock, dir) {
            Ok(guard) => Claim::Primary(guard),
            Err(error) => {
                log::warn!("Single-instance listener unavailable: {error:#}");
                Claim::Unguarded
            }
        },
        Err(TryLockError::WouldBlock) => {
            if let Err(error) = hand_off(dir, deep_link) {
                log::warn!("Ferrum is already running but did not answer: {error:#}");
            }
            Claim::Handed
        }
        Err(TryLockError::Error(error)) => {
            log::warn!("Single-instance lock unavailable: {error}");
            Claim::Unguarded
        }
    }
}

impl Guard {
    fn start(lock: File, dir: &Path) -> Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let mut bytes = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rng(), &mut bytes);
        let token = hex::encode(bytes);
        let port = listener.local_addr()?.port();
        write_private(&dir.join(ENDPOINT_FILE), &format!("{port} {token}\n"))?;
        Ok(Self {
            _lock: lock,
            listener,
            token,
        })
    }

    /// Accepts handoffs on a background thread. Each one focuses the window and yields
    /// the forwarded deep link, if any, on the returned channel.
    pub fn serve(self, ctx: egui::Context) -> mpsc::Receiver<Option<String>> {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("ferrum-instance".into())
            .spawn(move || {
                // Bind the whole guard: closures capture only the fields they use, and the
                // lock must stay held for the process lifetime.
                let guard = self;
                for stream in guard.listener.incoming().flatten() {
                    let Some(link) = read_request(stream, &guard.token) else {
                        continue;
                    };
                    if tx.send(link).is_err() {
                        break;
                    }
                    // Viewport commands are processed even while the window is minimized.
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    ctx.request_repaint();
                }
            })
            .expect("spawn single-instance listener");
        rx
    }
}

/// Returns the forwarded link (`Some(None)` for a plain focus request), or `None` if the
/// request is malformed or unauthenticated.
fn read_request(mut stream: TcpStream, token: &str) -> Option<Option<String>> {
    stream.set_read_timeout(Some(IO_TIMEOUT)).ok()?;
    let mut request = String::new();
    (&mut stream)
        .take(MAX_REQUEST)
        .read_to_string(&mut request)
        .ok()?;
    let (sent, link) = request.split_once('\n')?;
    if sent != token {
        return None;
    }
    let link = link.trim_end_matches('\n');
    Some((!link.is_empty()).then(|| link.to_owned()))
}

fn hand_off(dir: &Path, deep_link: Option<&str>) -> Result<()> {
    let link = deep_link.unwrap_or_default();
    if link.contains('\n') {
        return Err(anyhow!("deep links cannot contain line breaks"));
    }
    let deadline = Instant::now() + HANDOFF_WAIT;
    loop {
        let attempt = endpoint(dir).and_then(|(addr, token)| {
            let mut stream = TcpStream::connect_timeout(&addr, IO_TIMEOUT)?;
            stream.set_write_timeout(Some(IO_TIMEOUT))?;
            stream.write_all(format!("{token}\n{link}\n").as_bytes())?;
            Ok(())
        });
        // The running instance may still be between taking the lock and writing its endpoint.
        match attempt {
            Ok(()) => return Ok(()),
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn endpoint(dir: &Path) -> Result<(SocketAddr, String)> {
    let text = fs::read_to_string(dir.join(ENDPOINT_FILE))?;
    let (port, token) = text
        .trim()
        .split_once(' ')
        .context("malformed instance endpoint")?;
    Ok((
        SocketAddr::from((Ipv4Addr::LOCALHOST, port.parse::<u16>()?)),
        token.to_owned(),
    ))
}

/// Replaces `path` atomically, readable only by the current user on Unix.
fn write_private(path: &Path, contents: &str) -> Result<()> {
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    let mut options = File::options();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(&tmp)?.write_all(contents.as_bytes())?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn primary(dir: &Path) -> Guard {
        match claim(dir, None) {
            Claim::Primary(guard) => guard,
            _ => panic!("first launch must become the primary instance"),
        }
    }

    #[test]
    fn second_launch_forwards_its_deep_link_to_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let rx = primary(dir.path()).serve(egui::Context::default());
        assert!(matches!(
            claim(dir.path(), Some("ssh://root@example.test:2222")),
            Claim::Handed
        ));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap().as_deref(),
            Some("ssh://root@example.test:2222")
        );
        assert!(matches!(claim(dir.path(), None), Claim::Handed));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), None);
    }

    #[test]
    fn request_with_wrong_token_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let guard = primary(dir.path());
        let addr = guard.listener.local_addr().unwrap();
        let rx = guard.serve(egui::Context::default());
        TcpStream::connect(addr)
            .unwrap()
            .write_all(b"wrong-token\nssh://attacker@example.test\n")
            .unwrap();
        assert!(matches!(claim(dir.path(), None), Claim::Handed));
        // Only the authenticated focus request arrives.
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), None);
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }

    #[test]
    fn lock_is_released_when_the_primary_exits() {
        let dir = tempfile::tempdir().unwrap();
        drop(primary(dir.path()));
        primary(dir.path());
    }

    #[cfg(unix)]
    #[test]
    fn endpoint_file_is_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let _guard = primary(dir.path());
        let mode = fs::metadata(dir.path().join(ENDPOINT_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
