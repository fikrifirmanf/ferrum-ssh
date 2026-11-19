//! Exercise the real SSH transport against an in-process server, without a Docker daemon.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::conn::{self, AppEvent, Bridge, ConnCtxTemplate, PromptReply, Status};
use crate::model::{Host, KnownHost, Vault};
use crate::term::Input;
use parking_lot::Mutex;
use russh::server::{self, Server as _};

#[derive(Clone, Default)]
struct Server {
    commands: Arc<Mutex<Vec<String>>>,
    responses: Arc<Mutex<std::collections::VecDeque<(String, u32)>>>,
    command_responses: Arc<Mutex<HashMap<String, (String, u32)>>>,
    binary_responses: Arc<Mutex<std::collections::VecDeque<BinaryResponse>>>,
    import_probe: Arc<Mutex<Option<ImportProbe>>>,
    port_bindings: Arc<Mutex<String>>,
    deny_forward: Arc<std::sync::atomic::AtomicBool>,
    forwards: Arc<Mutex<Vec<(String, u32)>>>,
    terminal_started: Arc<tokio::sync::Notify>,
}

struct BinaryResponse {
    stdout: Vec<u8>,
    diagnostics: Vec<u8>,
    status: u32,
}

#[derive(Default)]
struct ImportProbe {
    existing: bool,
    restore_status: u32,
    token: Option<String>,
    uploading: Option<russh::ChannelId>,
    finish_after: Option<usize>,
    received: Vec<u8>,
    completion_output: Vec<u8>,
}
impl server::Server for Server {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}
impl server::Handler for Server {
    type Error = russh::Error;
    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        Ok(if user == "fixture" && password == "fixture" {
            server::Auth::Accept
        } else {
            server::Auth::reject()
        })
    }
    async fn channel_open_session(
        &mut self,
        _: russh::Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: russh::Channel<server::Msg>,
        host: &str,
        port: u32,
        _: &str,
        _: u32,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.forwards.lock().push((host.to_string(), port));
        if self.deny_forward.load(std::sync::atomic::Ordering::Relaxed)
            || !host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        {
            reply
                .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        }
        match tokio::net::TcpStream::connect((host, port as u16)).await {
            Ok(mut socket) => {
                reply.accept().await;
                tokio::spawn(async move {
                    let _ = tokio::io::copy_bidirectional(&mut socket, &mut channel.into_stream())
                        .await;
                });
            }
            Err(_) => reply.reject(russh::ChannelOpenFailure::ConnectFailed).await,
        }
        Ok(())
    }
    async fn pty_request(
        &mut self,
        channel: russh::ChannelId,
        _: &str,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
        _: &[(russh::Pty, u32)],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }

    async fn data(
        &mut self,
        channel: russh::ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if let Some(probe) = self.import_probe.lock().as_mut()
            && probe.uploading == Some(channel)
        {
            probe.received.extend_from_slice(data);
            if probe.finish_after == Some(probe.received.len()) {
                probe.uploading = None;
                session.exit_status_request(channel, 0)?;
                session.eof(channel)?;
                session.close(channel)?;
            }
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: russh::ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let mut state = self.import_probe.lock();
        if let Some(probe) = state.as_mut()
            && probe.uploading == Some(channel)
        {
            probe.uploading = None;
            if probe.restore_status != 0 {
                session.extended_data(channel, 1, b"tar: restore failed\n".to_vec())?;
            }
            if !probe.completion_output.is_empty() {
                session.data(channel, std::mem::take(&mut probe.completion_output))?;
            }
            session.exit_status_request(channel, probe.restore_status)?;
            session.eof(channel)?;
            session.close(channel)?;
        }
        Ok(())
    }
    async fn exec_request(
        &mut self,
        channel: russh::ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).into_owned();
        self.commands.lock().push(command.clone());
        session.channel_success(channel)?;
        if command == "docker exec -it abcdef123456 sh" {
            session.data(channel, b"container$ ".to_vec())?;
            self.terminal_started.notify_one();
            return Ok(());
        }
        if command == "late-status" {
            session.data(channel, b"failed".to_vec())?;
            session.eof(channel)?;
            session.exit_status_request(channel, 9)?;
            session.close(channel)?;
            return Ok(());
        }
        let volume_import_reply = {
            let mut state = self.import_probe.lock();
            if let Some(probe) = state.as_mut() {
                if command.starts_with("docker volume create --driver local --label ") {
                    if !probe.existing {
                        probe.token = command
                            .split_once("dev.ferrum.volume-import=")
                            .and_then(|(_, value)| value.split_once('\''))
                            .map(|(value, _)| value.to_owned());
                    }
                    Some("sample".into())
                } else if command.starts_with("docker volume inspect")
                    && command.contains("dev.ferrum.volume-import")
                {
                    Some(
                        serde_json::json!({
                            "driver":"local",
                            "path":"/var/lib/docker/volumes/sample/_data",
                            "has_options":false,
                            "token":probe.token,
                        })
                        .to_string(),
                    )
                } else if command.contains("exec tar -xkf - --numeric-owner")
                    || command.contains("replacement_hash=$3")
                {
                    probe.uploading = Some(channel);
                    return Ok(());
                } else if command == "docker volume rm -- 'sample'" {
                    probe.token = None;
                    Some("sample".into())
                } else {
                    None
                }
            } else {
                None
            }
        };
        let binary = if command.contains("docker run --rm --pull=missing") {
            self.binary_responses.lock().pop_front()
        } else {
            None
        };
        if let Some(reply) = binary {
            session.data(channel, reply.stdout)?;
            session.extended_data(channel, 1, reply.diagnostics)?;
            session.eof(channel)?;
            session.exit_status_request(channel, reply.status)?;
            session.close(channel)?;
            return Ok(());
        }
        let response = volume_import_reply.map(|output| (output, 0)).or_else(|| {
            self.command_responses
                .lock()
                .remove(&command)
                .or_else(|| self.responses.lock().pop_front())
        });
        let (text, code) = if let Some(response) = response {
            response
        } else if command.contains("--format '{{json .NetworkSettings.Ports}}'") {
            let bindings = self.port_bindings.lock().clone();
            (
                if bindings.is_empty() {
                    "null".into()
                } else {
                    bindings
                },
                0,
            )
        } else if command.contains("range .Config.Env") {
            ("\"POSTGRES_USER=fixture\"\n\"POSTGRES_PASSWORD=synthetic=p@ss\"\n\"POSTGRES_DB=events\"\n".into(), 0)
        } else if command.contains(" stats --no-stream ") {
            (r#"{"ID":"abcdef123456","CPUPerc":"125.5%","MemPerc":"12.5%","MemUsage":"32MiB / 256MiB","NetIO":"1kB / 2kB","BlockIO":"0B / 1kB","PIDs":"8"}"#.to_owned(), 0)
        } else if command.contains(" inspect --format ") {
            (r#"{"id":"abcdef123456","image":"nginx:1.27","state":"running","mounts":[],"networks":{},"ports":{}}"#.to_owned(), 0)
        } else if command.contains(" logs --tail ") {
            ("\u{1b}[31mready\u{1b}[0m\n".into(), 0)
        } else if command == "oversize" {
            ("x".repeat(2048), 0)
        } else if command == "failure" {
            ("permission denied".into(), 7)
        } else {
            ("owner alive".into(), 0)
        };
        session.data(channel, text.into_bytes())?;
        session.exit_status_request(channel, code)?;
        session.eof(channel)?;
        session.close(channel)?;
        Ok(())
    }
}

struct Fixture {
    client: Arc<crate::conn::ssh::SshClient>,
    target: crate::model::ResolvedTarget,
    template: ConnCtxTemplate,
    server: Server,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn fixture() -> Fixture {
    let key = ssh_key::PrivateKey::from(ssh_key::private::Ed25519Keypair::from_seed(&[31; 32]));
    let socket = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = socket.local_addr().unwrap().port();
    let known = KnownHost {
        id: uuid::Uuid::nil(),
        host: "127.0.0.1".into(),
        port,
        key_type: "ssh-ed25519".into(),
        public_key: key.public_key().to_openssh().unwrap(),
        fingerprint: key
            .public_key()
            .fingerprint(russh::keys::HashAlg::Sha256)
            .to_string(),
        added: 0,
        updated: 0,
    };
    let server = Server::default();
    let mut running = server.clone();
    let config = Arc::new(server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let task = tokio::spawn(async move {
        running.run_on_socket(config, &socket).await.unwrap();
    });
    let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    let prompts = tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if let AppEvent::Prompt(prompt) = event {
                let _ = prompt.reply.send(PromptReply::Cancel);
            }
        }
    });
    let template = ConnCtxTemplate {
        bridge: Bridge {
            events: tx,
            ctx: egui::Context::default(),
        },
        known_hosts: vec![known],
        mosh_client: String::new(),
        detect_os: false,
        local_shell: String::new(),
    };
    let mut host = Host {
        address: "127.0.0.1".into(),
        ..Default::default()
    };
    host.settings.port = Some(port);
    host.settings.username = Some("fixture".into());
    host.settings.password = Some("fixture".into());
    host.settings.use_agent = Some(false);
    let target = Vault::default().resolve(&host);
    let client = tokio::time::timeout(
        Duration::from_secs(5),
        crate::conn::ssh::connect_headless(target.clone(), template.headless()),
    )
    .await
    .unwrap()
    .unwrap();
    Fixture {
        client,
        target,
        template,
        server,
        tasks: vec![task, prompts],
    }
}

fn listing() -> Listing {
    parse(
        r#"@docker
{"ID":"abcdef123456","Names":"api","Image":"nginx:1.27","State":"running"}
"#,
    )
}

#[tokio::test]
async fn volume_editor_reads_versions_and_uploads_exact_utf8_bytes() {
    use volume_files::{Edit, EditRequest, EditResult, edit};

    let fixture = fixture().await;
    let metadata =
        r#"{"driver":"local","path":"/var/lib/docker/volumes/sample/_data","has_options":false}"#;
    fixture
        .server
        .responses
        .lock()
        .extend((0..3).map(|_| (metadata.to_owned(), 0)));
    fixture
        .server
        .binary_responses
        .lock()
        .push_back(BinaryResponse {
            stdout: b"20:123:8:time:time:1\nport=80\n".to_vec(),
            diagnostics: b"helper ready".to_vec(),
            status: 0,
        });
    let request = |edit| EditRequest {
        volume: "sample".into(),
        directory: vec![b"conf".to_vec()],
        name: b"app.conf".to_vec(),
        revision: 7,
        edit,
    };
    let opened = edit(&fixture.client, request(Edit::Open), false)
        .await
        .unwrap();
    let EditResult::Opened { version, text } = opened else {
        panic!("Expected the opened file");
    };
    assert_eq!(text, "port=80\n");
    assert!(!version.read_only());

    *fixture.server.import_probe.lock() = Some(ImportProbe {
        completion_output: b"Ferrum saved 20:124:10:time:time:1\n".to_vec(),
        ..Default::default()
    });
    let saved = edit(
        &fixture.client,
        request(Edit::Save {
            original: version.clone(),
            text: "port=8080\n".into(),
        }),
        false,
    )
    .await
    .unwrap();
    let EditResult::Saved(updated) = saved else {
        panic!("Expected a saved revision")
    };
    assert_ne!(updated, version);
    assert_eq!(
        fixture.server.import_probe.lock().take().unwrap().received,
        b"port=8080\n"
    );

    fixture
        .server
        .binary_responses
        .lock()
        .push_back(BinaryResponse {
            stdout: b"20:125:2:time:time:1\n\xff\xfe".to_vec(),
            diagnostics: Vec::new(),
            status: 0,
        });
    let error = edit(&fixture.client, request(Edit::Open), false)
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("not UTF-8 text"));
    let commands = fixture.server.commands.lock();
    assert_eq!(commands.len(), 6);
    assert!(commands[1].contains("readonly,bind-recursive=disabled"));
    assert!(commands[1].ends_with(" ferrum 'YXBwLmNvbmY=' 'Y29uZg=='"));
    assert!(commands[3].contains("20:123:8:time:time:1"));
    assert!(commands[3].contains("mv -fT"));
}

#[tokio::test]
async fn volume_files_replay_docker_desktop_listings_and_preserve_binary_tar() {
    use volume_files::{Kind, browse, display_name, export};

    let fixture = fixture().await;
    let metadata =
        r#"{"driver":"local","path":"/var/lib/docker/volumes/sample/_data","has_options":false}"#;
    fixture
        .server
        .responses
        .lock()
        .extend((0..4).map(|_| (metadata.to_owned(), 0)));
    let mut archive = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_ustar();
    header.set_mode(0o644);
    header.set_size(3);
    archive
        .append_data(&mut header, "blob.bin", [0, 0xff, b'A'].as_slice())
        .unwrap();
    let archive = archive.into_inner().unwrap();
    fixture.server.binary_responses.lock().extend([
        BinaryResponse {
            stdout: b"l\0\0alias\0f\x003\0blob.bin\0d\0\0nested\0f\x006\0.config\0".to_vec(),
            diagnostics: b"helper image available".to_vec(),
            status: 0,
        },
        BinaryResponse {
            stdout: b"f\x005\0line\nbreak\0".to_vec(),
            diagnostics: Vec::new(),
            status: 0,
        },
        BinaryResponse {
            stdout: archive.clone(),
            diagnostics: b"pull diagnostics must not enter the archive".to_vec(),
            status: 0,
        },
        BinaryResponse {
            stdout: archive[..512].to_vec(),
            diagnostics: b"tar: permission denied\n".to_vec(),
            status: 1,
        },
    ]);

    let root = browse(&fixture.client, "sample", &[], false).await.unwrap();
    assert_eq!(
        root.iter()
            .map(|entry| (entry.name.as_slice(), entry.kind))
            .collect::<Vec<_>>(),
        [
            (b"nested".as_slice(), Kind::Directory),
            (b".config".as_slice(), Kind::File(6)),
            (b"alias".as_slice(), Kind::Link),
            (b"blob.bin".as_slice(), Kind::File(3)),
        ],
    );
    let folder = browse(&fixture.client, "sample", &[root[0].name.clone()], false)
        .await
        .unwrap();
    assert_eq!(folder.len(), 1);
    assert_eq!(folder[0].name, b"line\nbreak");
    assert_eq!(display_name(&folder[0].name), r"line\nbreak");
    assert_eq!(display_name(&[0xff]), r"\xff");
    assert_eq!(folder[0].kind, Kind::File(5));

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sample.tar");
    assert_eq!(
        export(&fixture.client, "sample", &path, false)
            .await
            .unwrap(),
        archive.len() as u64
    );
    assert_eq!(std::fs::read(&path).unwrap(), archive);
    let failed = export(&fixture.client, "sample", &path, false)
        .await
        .unwrap_err();
    assert_eq!(failed.to_string(), "tar: permission denied");
    assert_eq!(std::fs::read(&path).unwrap(), archive);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);

    let commands = fixture.server.commands.lock();
    assert_eq!(commands.len(), 8);
    assert!(commands[1].contains("readonly,bind-recursive=disabled"));
    assert!(commands[3].ends_with(" ferrum 'bmVzdGVk'"));
    assert!(commands[5].ends_with(" tar -C /volume -cf - ."));
}

#[tokio::test]
async fn volume_browse_does_not_start_a_helper_for_a_missing_or_plugin_volume() {
    let fixture = fixture().await;
    fixture.server.responses.lock().extend([
        ("No such volume: removed\n".into(), 1),
        (
            r#"{"driver":"local","path":"/var/lib/docker/volumes/nfs/_data","has_options":true}"#
                .into(),
            0,
        ),
    ]);
    for (name, expected) in [
        ("removed", "No such volume: removed"),
        ("nfs", "File access is available for standard local volumes"),
    ] {
        let error = volume_files::browse(&fixture.client, name, &[], false)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), expected);
    }
    let invalid = volume_files::browse(&fixture.client, "sample", &[b"..".to_vec()], false)
        .await
        .unwrap_err();
    assert_eq!(invalid.to_string(), "Invalid volume folder path");
    let commands = fixture.server.commands.lock();
    assert_eq!(commands.len(), 2);
    assert!(
        commands
            .iter()
            .all(|command| command.starts_with("docker volume inspect"))
    );
}

fn volume_archive_header(mode: u32) -> tar::Header {
    let mut header = tar::Header::new_gnu();
    header.set_mode(mode);
    header.set_size(0);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header
}

#[tokio::test]
async fn archive_upload_accepts_success_before_ssh_eof_after_all_bytes() {
    let fixture = fixture().await;
    let bytes = vec![0xa5; 256 * 1024 + 5];
    *fixture.server.import_probe.lock() = Some(ImportProbe {
        finish_after: Some(bytes.len()),
        ..Default::default()
    });
    let (_, status) = fixture
        .client
        .exec_from_reader(
            "exec tar -xkf - --numeric-owner",
            &mut bytes.as_slice(),
            bytes.len() as u64,
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert_eq!(status, 0);
    assert_eq!(
        fixture.server.import_probe.lock().take().unwrap().received,
        bytes
    );
}

fn volume_archive() -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (name, mode) in [(".", 0o750), ("nested", 0o755)] {
        let mut header = volume_archive_header(mode);
        header.set_entry_type(tar::EntryType::Directory);
        tar.append_data(&mut header, name, std::io::empty())
            .unwrap();
    }
    let mut header = volume_archive_header(0o640);
    header.set_size(3);
    header.set_uid(1234);
    header.set_gid(1235);
    tar.append_data(&mut header, "nested/blob.bin", [0, 0xff, b'A'].as_slice())
        .unwrap();
    for (name, kind) in [
        ("alias", tar::EntryType::Symlink),
        ("blob-link", tar::EntryType::Link),
    ] {
        header = volume_archive_header(0o640);
        header.set_entry_type(kind);
        header.set_uid(1234);
        header.set_gid(1235);
        tar.append_link(&mut header, name, "nested/blob.bin")
            .unwrap();
    }
    tar.into_inner().unwrap()
}

#[tokio::test]
async fn volume_import_uploads_tar_and_gzip_with_binary_contents_and_links() {
    use std::io::{Read as _, Write as _};

    let dir = tempfile::tempdir().unwrap();
    let data = volume_archive();
    let plain = dir.path().join("export.tar");
    std::fs::write(&plain, &data).unwrap();
    let compressed = dir.path().join("export.tar.gz");
    let mut gzip = flate2::write::GzEncoder::new(
        std::fs::File::create(&compressed).unwrap(),
        flate2::Compression::fast(),
    );
    gzip.write_all(&data).unwrap();
    gzip.finish().unwrap();
    for path in [plain, compressed] {
        let fixture = fixture().await;
        *fixture.server.import_probe.lock() = Some(ImportProbe::default());
        tokio::time::timeout(
            Duration::from_secs(5),
            volume_import::import(&fixture.client, "sample", &path, false),
        )
        .await
        .unwrap()
        .unwrap();
        let received = fixture.server.import_probe.lock().take().unwrap().received;
        let mut tar = tar::Archive::new(received.as_slice());
        let mut contents = HashMap::new();
        for entry in tar.entries().unwrap() {
            let mut entry = entry.unwrap();
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            contents.insert(
                entry.path_bytes().into_owned(),
                (
                    entry.header().entry_type(),
                    entry.header().mode().unwrap(),
                    entry.header().uid().unwrap(),
                    entry.header().gid().unwrap(),
                    entry.link_name_bytes().map(|target| target.into_owned()),
                    data,
                ),
            );
        }
        let nested = &contents[b"nested/blob.bin".as_slice()];
        let alias = &contents[b"blob-link".as_slice()];
        // Docker may emit either pathname first when recording this inode.
        let (file, hard_link, expected_target): (_, _, &[u8]) =
            if nested.0 == tar::EntryType::Regular {
                (nested, alias, b"nested/blob.bin")
            } else {
                (alias, nested, b"blob-link")
            };
        assert_eq!(
            file,
            &(
                tar::EntryType::Regular,
                0o640,
                1234,
                1235,
                None,
                vec![0, 0xff, b'A']
            )
        );
        assert_eq!(contents[b".".as_slice()].1, 0o750);
        assert_eq!(contents[b"alias".as_slice()].0, tar::EntryType::Symlink);
        assert_eq!(
            contents[b"alias".as_slice()].4.as_deref(),
            Some(b"nested/blob.bin".as_slice())
        );
        assert_eq!(hard_link.0, tar::EntryType::Link);
        assert_eq!(hard_link.4.as_deref(), Some(expected_target));
        let commands = fixture.server.commands.lock();
        assert_eq!(commands.len(), 3);
        assert!(commands[0].starts_with("docker volume create --driver local --label "));
        assert!(commands[1].starts_with("docker volume inspect"));
        assert!(commands[2].contains("exec tar -xkf - --numeric-owner"));
        assert!(commands[2].contains(" -i "));
    }
}

#[tokio::test]
async fn volume_import_preserves_existing_volumes_and_removes_its_failed_restore() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("export.tar");
    std::fs::write(&path, volume_archive()).unwrap();
    for (probe, expected, commands, deleted) in [
        (
            ImportProbe {
                existing: true,
                token: Some("original".into()),
                ..Default::default()
            },
            "already exists",
            2,
            false,
        ),
        (
            ImportProbe {
                restore_status: 1,
                ..Default::default()
            },
            "partial volume was removed",
            5,
            true,
        ),
    ] {
        let fixture = fixture().await;
        *fixture.server.import_probe.lock() = Some(probe);
        let error = tokio::time::timeout(
            Duration::from_secs(5),
            volume_import::import(&fixture.client, "sample", &path, false),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(error.to_string().contains(expected), "{error:#}");
        let probe = fixture.server.import_probe.lock().take().unwrap();
        assert_eq!(probe.token.as_deref(), (!deleted).then_some("original"));
        assert_eq!(probe.received.is_empty(), !deleted);
        assert_eq!(fixture.server.commands.lock().len(), commands);
    }
}

#[tokio::test]
async fn volume_import_rejects_escaping_paths_and_writes_through_links_before_docker() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid.tar");
    let fixture = fixture().await;
    let mut tar = tar::Builder::new(Vec::new());
    let mut header = volume_archive_header(0o644);
    header.as_gnu_mut().unwrap().name[..10].copy_from_slice(b"../outside");
    header.set_cksum();
    tar.append(&header, std::io::empty()).unwrap();
    std::fs::write(&path, tar.into_inner().unwrap()).unwrap();
    let error = volume_import::import(&fixture.client, "sample", &path, false)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Archive paths cannot leave the destination folder"
    );

    let mut tar = tar::Builder::new(Vec::new());
    let mut header = volume_archive_header(0o777);
    header.set_entry_type(tar::EntryType::Symlink);
    tar.append_link(&mut header, "link", "target").unwrap();
    header = volume_archive_header(0o644);
    tar.append_data(&mut header, "link/content", std::io::empty())
        .unwrap();
    std::fs::write(&path, tar.into_inner().unwrap()).unwrap();
    let error = volume_import::import(&fixture.client, "sample", &path, false)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "The archive places an entry inside a file or link"
    );
    assert!(fixture.server.commands.lock().is_empty());
}

#[tokio::test]
async fn unified_logs_merges_docker_desktop_inputs_and_preserves_source_errors() {
    use unified_logs::{Source, load};

    // Stdout and stderr from a disposable Redis-image container, plus a
    // Buildx raw JSON event from a scratch build with a missing source file.
    let container =
        "2026-11-18T18:12:37.043449000Z ready\n2026-11-18T18:12:37.043507250Z warning on stderr\n";
    let build = r#"{"vertexes":[{"name":"[1/1] COPY missing-file /missing-file","completed":"2026-11-18T18:13:27.95679526Z","error":"failed to calculate checksum of ref gs80qpl5em9zqmq2bcuwtegv6::newobunmrw6p9kqlq0kiweao7: \"/missing-file\": not found"}]}
{"logs":[{"data":"cHJlcGFyaW5nIGJ1aWxkCg==","timestamp":"2026-11-18T18:12:37.043500Z"}]}
"#;
    let fixture = fixture().await;
    fixture.server.command_responses.lock().extend([
        (
            "docker logs --tail 300 --timestamps abcdef123456 2>&1".into(),
            (container.into(), 0),
        ),
        (
            "docker buildx history logs --builder 'desktop-linux' --progress rawjson -- 'build123'"
                .into(),
            (build.into(), 0),
        ),
        (
            "podman logs --tail 300 --timestamps 123456abcdef 2>&1".into(),
            ("permission denied\n".into(), 1),
        ),
    ]);
    let item = listing().items.remove(0);
    let mut podman = item.clone();
    podman.runtime = Runtime::Podman;
    podman.scope = "123456abcdef".into();
    let snapshot = load(
        &fixture.client,
        vec![
            Source::Build {
                reference: "desktop-linux/desktop-linux/build123".into(),
                name: "scratch".into(),
            },
            Source::Container(podman),
            Source::Container(item),
        ],
        false,
    )
    .await;
    assert_eq!(
        snapshot.errors,
        [("Podman / api".into(), "permission denied".into())]
    );
    let messages: Vec<_> = snapshot
        .rows
        .iter()
        .map(|row| row.message.as_str())
        .collect();
    assert_eq!(
        messages,
        [
            "ready",
            "preparing build",
            "warning on stderr",
            "[1/1] COPY missing-file /missing-file: failed to calculate checksum of ref gs80qpl5em9zqmq2bcuwtegv6::newobunmrw6p9kqlq0kiweao7: \"/missing-file\": not found",
        ]
    );
    assert_eq!(&*snapshot.rows[0].source, "Docker / api");
    assert_eq!(&*snapshot.rows[3].source, "Build / scratch (build123)");
    assert_eq!(
        snapshot.rows[0].time.unwrap().timestamp_subsec_nanos(),
        43_449_000
    );
    assert!(fixture.server.command_responses.lock().is_empty());
}

#[test]
fn changing_log_sources_drops_an_in_flight_snapshot() {
    let mut view = unified_logs::View::default();
    view.change_source("first".into(), true);
    let old_revision = view.revision;
    view.pending = true;
    view.change_source("second".into(), true);
    view.apply(old_revision, unified_logs::Snapshot::default());
    assert!(view.snapshot.is_none());
    assert!(!view.pending);
    view.apply(view.revision, unified_logs::Snapshot::default());
    assert!(view.snapshot.is_some());
}

#[tokio::test]
async fn docker_desktop_log_search_presets_and_jsonl_export_round_trip() {
    use crate::model::{LogFilterPreset, LogSearch};
    use unified_logs::{PresetEdit, Source, View, edit_presets, export, load};

    let fixture = fixture().await;
    fixture.server.command_responses.lock().insert(
        "docker logs --tail 300 --timestamps abcdef123456 2>&1".into(),
        (
            "2026-11-18T18:28:03.378941416Z ready\n2026-11-18T18:28:03.379010250Z WARN retry\n2026-11-18T18:28:03.379014125Z ERROR request failed\n".into(),
            0,
        ),
    );
    let source = Source::Container(listing().items.remove(0));
    let preset = LogFilterPreset {
        name: "Warnings and errors".into(),
        search: LogSearch {
            query: r"^(warn|error)\b".into(),
            regex: true,
            case_sensitive: false,
        },
        sources: [source.key()].into(),
    };
    let snapshot = load(&fixture.client, vec![source], false).await;
    assert!(snapshot.errors.is_empty());
    let dir = tempfile::tempdir().unwrap();
    let store = crate::store::Store {
        path: dir.path().join("vault.ferrum"),
        master_password: Some("synthetic test vault".into()),
    };
    let mut vault = Vault::default();
    let host = uuid::Uuid::from_u128(19);
    let other_host = uuid::Uuid::from_u128(20);
    assert!(edit_presets(
        &mut vault.settings,
        host,
        PresetEdit::Save(preset.clone())
    ));
    store.save(&vault).unwrap();
    let crate::store::LoadOutcome::Loaded(mut saved) = store.probe().unwrap() else {
        panic!("Expected the saved encrypted vault");
    };
    assert!(!saved.settings.log_filter_presets.contains_key(&other_host));
    let loaded_preset = &saved.settings.log_filter_presets[&host][0];
    assert_eq!(loaded_preset, &preset);

    let mut view = View::default();
    view.apply_preset(loaded_preset);
    assert_eq!(view.selected, preset.sources);
    assert!(view.refresh_requested);
    let matches: Vec<_> = snapshot
        .rows
        .iter()
        .filter(|row| view.matcher().matches(row))
        .cloned()
        .collect();
    assert_eq!(
        matches
            .iter()
            .map(|row| row.message.as_str())
            .collect::<Vec<_>>(),
        ["WARN retry", "ERROR request failed"],
    );
    let path = dir.path().join("matches.jsonl");
    export(&path, &snapshot.rows).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 3);
    export(&path, &matches).unwrap();
    let file = std::fs::read_to_string(&path).unwrap();
    let records: Vec<serde_json::Value> = file
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        records,
        [
            serde_json::json!({"timestamp":"2026-11-18T18:28:03.379010250Z","source":"Docker / api","message":"WARN retry"}),
            serde_json::json!({"timestamp":"2026-11-18T18:28:03.379014125Z","source":"Docker / api","message":"ERROR request failed"}),
        ]
    );

    view.search.case_sensitive = true;
    view.update_search();
    assert!(!snapshot.rows.iter().any(|row| view.matcher().matches(row)));
    view.search.query = "[".into();
    view.update_search();
    assert!(view.matcher().error().unwrap().contains("unclosed"));
    assert!(!snapshot.rows.iter().any(|row| view.matcher().matches(row)));
    assert!(edit_presets(
        &mut saved.settings,
        host,
        PresetEdit::Remove(preset.name)
    ));
    assert!(!saved.settings.log_filter_presets.contains_key(&host));
}

#[tokio::test]
async fn docker_desktop_build_history_replays_completed_failed_and_log_inputs() {
    use inventory::{Detail, Entry, Kind, Reply, Request, run};

    let fixture = fixture().await;
    // Records and failure output from two scratch builds on Docker Desktop 4.99.0.
    let records = r#"{"cached_steps":0,"completed_at":"2026-11-18T17:56:33.081930255Z","completed_steps":4,"created_at":"2026-11-18T17:56:33.07147788Z","name":"context","ref":"desktop-linux/desktop-linux/pmb2k96xaq5c29pr8kedmabvx","status":"Error","total_steps":4}
{"cached_steps":0,"completed_at":"2026-11-18T17:56:32.968074213Z","completed_steps":5,"created_at":"2026-11-18T17:56:32.933179546Z","name":"context","ref":"desktop-linux/desktop-linux/gndo3i3l3gywy7dbymwea920p","status":"Completed","total_steps":5}
"#;
    let log = "#4 [1/1] COPY missing-file /missing-file\n#4 ERROR: failed to calculate checksum of ref gs80qpl5em9zqmq2bcuwtegv6::ysmjki4zc03ao35yhploet1ju: \"/missing-file\": not found\n------\n > [1/1] COPY missing-file /missing-file:\n------\n";
    fixture
        .server
        .responses
        .lock()
        .extend([(records.into(), 0), (log.into(), 0)]);

    let Reply::Listing(rows) = run(&fixture.client, &Request::List(Kind::Builds), false)
        .await
        .unwrap()
    else {
        panic!("Expected Buildx records");
    };
    let [Entry::Build(failed), Entry::Build(completed)] = rows.as_slice() else {
        panic!("Expected the captured failed and completed builds");
    };
    assert_eq!(completed.status, "Completed");
    assert_eq!(
        (completed.completed_steps, completed.total_steps),
        (Some(5), Some(5))
    );
    assert_eq!(failed.status, "Error");
    assert!(failed.created_at > completed.created_at);
    let inspect = Request::Inspect(Kind::Builds, failed.reference.clone());
    let Reply::Detail(Detail::BuildLog(output)) =
        run(&fixture.client, &inspect, false).await.unwrap()
    else {
        panic!("Expected the failed build's log");
    };
    assert_eq!(output, log);

    let unsafe_ref = Request::Inspect(Kind::Builds, "builder/node/id; echo".into());
    let error = run(&fixture.client, &unsafe_ref, false)
        .await
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "Invalid Buildx record reference");
    assert_eq!(
        *fixture.server.commands.lock(),
        [
            "docker buildx history ls --no-trunc --format '{{json .}}'",
            "docker buildx history logs --builder 'desktop-linux' --progress plain -- 'pmb2k96xaq5c29pr8kedmabvx'",
        ]
    );
}

#[tokio::test]
async fn docker_desktop_image_and_volume_inputs_round_trip_over_ssh() {
    use inventory::{Detail, Entry, Kind, Page, Reply, Request, run};

    const IMAGE_ID: &str =
        "sha256:2e3eb11462c4e3759e6972678dd051d150ff54b1e60714b3c33b701260172a90";
    const VOLUME: &str = "ferrum-check-e4f5b822";
    let fixture = fixture().await;
    // Projected responses from Docker Desktop 4.99.0, Engine 29.9.3.
    fixture.server.responses.lock().extend([
        (format!(r#"{{"ID":"{IMAGE_ID}","Repository":"redis","Tag":"7","Size":"192MB"}}"#), 0),
        (format!(r#"{{"id":"{IMAGE_ID}","tags":["redis:7"],"digests":["redis@{IMAGE_ID}"],"os":"linux","architecture":"arm64","created":"2026-11-05T07:35:21.030091384Z","size_bytes":189938399,"layers":["sha256:c96c411090da09187c55ab267826b3903b542b5c03f1cafa912d60e060e7850c","sha256:e68e60cd6f9ab41d965d6993d102557781540cad69833d5839d8029fa9393082","sha256:c680cbb7dbb46b58fbc55948079b91056e58403ef70156f405b5e7cca7b589df","sha256:7142801704b3e0a031811afa0e38241417ea4c28d22f55642615f6a775ea7236","sha256:9b6bac1bdefde72f8c140a4b8137e68ba5a494f72502f1d73be768a49d5ea411","sha256:5f70bf18a086007016e948b04aed3b82103a36bea41755b6cddfaf10ace3c6ef","sha256:a3ed94792f843221cb9e57855a154feca3f5f8f57762130e1117ace4d8881ae8"]}}"#), 0),
        (format!("{VOLUME}\n"), 0),
        (format!(r#"{{"Name":"{VOLUME}","Driver":"local","Scope":"local"}}"#), 0),
        (format!(r#"{{"name":"{VOLUME}","driver":"local","scope":"local","created":"2026-11-18T17:48:08Z","mountpoint":"/var/lib/docker/volumes/{VOLUME}/_data"}}"#), 0),
    ]);

    let Reply::Listing(images) = run(&fixture.client, &Request::List(Kind::Images), false)
        .await
        .unwrap()
    else {
        panic!("Expected an image listing");
    };
    let [Entry::Image(image)] = images.as_slice() else {
        panic!("Expected the captured Redis image");
    };
    assert_eq!(image.reference().as_deref(), Some("redis:7"));
    assert_eq!(image.id, IMAGE_ID);
    assert_eq!(image.size, "192MB");

    let inspect = Request::Inspect(Kind::Images, image.id.clone());
    let Reply::Detail(Detail::Metadata(image)) =
        run(&fixture.client, &inspect, false).await.unwrap()
    else {
        panic!("Expected image metadata");
    };
    assert_eq!(image["architecture"], "arm64");
    assert_eq!(image["layers"].as_array().unwrap().len(), 7);
    assert_eq!(image["size_bytes"], 189938399);

    let mut page = Page {
        listing: Some(Ok(Vec::new())),
        input: VOLUME.into(),
        pending: true,
        ..Default::default()
    };
    let create = Request::CreateVolume(VOLUME.into());
    let reply = run(&fixture.client, &create, true).await.unwrap();
    page.apply(create, Ok(reply));
    assert_eq!(
        page.notice.unwrap().unwrap(),
        format!("Volume {VOLUME} is available on this host.")
    );
    assert!(page.listing.is_none());
    assert!(page.input.is_empty());
    let volumes = Request::List(Kind::Volumes);
    let Reply::Listing(volumes) = run(&fixture.client, &volumes, true).await.unwrap() else {
        panic!("Expected a volume listing");
    };
    let [Entry::Volume(volume)] = volumes.as_slice() else {
        panic!("Expected the captured volume");
    };
    assert_eq!(volume.driver, "local");
    assert_eq!(volume.name, VOLUME);
    let inspect = Request::Inspect(Kind::Volumes, volume.name.clone());
    let Reply::Detail(Detail::Metadata(volume)) =
        run(&fixture.client, &inspect, true).await.unwrap()
    else {
        panic!("Expected volume metadata");
    };
    assert_eq!(
        volume["mountpoint"],
        format!("/var/lib/docker/volumes/{VOLUME}/_data")
    );

    let commands = fixture.server.commands.lock();
    assert_eq!(commands.len(), 5);
    assert!(commands[0].starts_with("docker image ls --no-trunc --format "));
    assert!(commands[1].ends_with(&format!(" -- '{IMAGE_ID}'")));
    assert_eq!(
        commands[2],
        format!("sudo -n docker volume create -- '{VOLUME}'")
    );
    assert!(commands[3].starts_with("sudo -n docker volume ls --format "));
    assert!(commands[4].ends_with(&format!(" -- '{VOLUME}'")));
    assert!(!commands[1].contains(".Config"));
    assert!(!commands[4].contains(".Options"));
}

#[tokio::test]
async fn image_pull_validates_the_reference_and_preserves_docker_errors() {
    use inventory::{Request, run};

    let fixture = fixture().await;
    let result = run(&fixture.client, &Request::Pull("redis:7; id".into()), false).await;
    assert_eq!(
        result.err().unwrap().to_string(),
        "Enter a Docker image reference, such as redis:7"
    );
    assert!(fixture.server.commands.lock().is_empty());

    fixture
        .server
        .responses
        .lock()
        .push_back(("manifest unknown\n".into(), 1));
    let result = run(
        &fixture.client,
        &Request::Pull("registry.example:5000/redis:7".into()),
        false,
    )
    .await;
    assert_eq!(result.err().unwrap().to_string(), "manifest unknown");
    assert_eq!(
        *fixture.server.commands.lock(),
        ["docker image pull -- 'registry.example:5000/redis:7'"]
    );
}

#[tokio::test]
async fn docker_desktop_lifecycle_inputs_select_the_correct_filter_and_ssh_action() {
    // Captured with Docker Desktop 4.99.0, Engine 29.9.3, using three disposable
    // containers. The reference engine transitioned to paused, running, and running.
    let listing = parse(
        r#"@docker
{"ID":"373ed0cab93b963acda242786b469e6b8d585199ee13be7566807071435a7b76","Names":"ferrum-rea-f0c7f93f-running","Image":"redis:7","State":"running","Status":"Up Less than a second","Ports":"","Labels":"desktop.docker.io/ports.scheme=v2"}
{"ID":"45bb980548d7f4256530b2ddf17b410b404cf667a78e3c936ef8364873a9c90f","Names":"ferrum-rea-f0c7f93f-paused","Image":"redis:7","State":"paused","Status":"Up Less than a second (Paused)","Ports":"","Labels":"desktop.docker.io/ports.scheme=v2"}
{"ID":"e3871a4e1e159b231293b48e63c3430f658239ad2616142adf52279256e9a28f","Names":"ferrum-rea-f0c7f93f-exited","Image":"redis:7","State":"exited","Status":"Exited (137) Less than a second ago","Ports":"","Labels":"desktop.docker.io/ports.scheme=v2"}
"#,
    );
    assert_eq!(listing.items.len(), 3);
    let fixture = fixture().await;
    let cases: [(StateFilter, &[Action], Action); 3] = [
        (
            StateFilter::Running,
            &[Action::Stop, Action::Restart, Action::Pause],
            Action::Pause,
        ),
        (StateFilter::Paused, &[Action::Resume], Action::Resume),
        (StateFilter::Stopped, &[Action::Start], Action::Start),
    ];
    for (item, (expected_filter, expected_actions, action)) in listing.items.iter().zip(cases) {
        let matching: Vec<_> = [
            StateFilter::Running,
            StateFilter::Paused,
            StateFilter::Stopped,
        ]
        .into_iter()
        .filter(|filter| filter.matches(item))
        .collect();
        assert_eq!(matching, [expected_filter]);
        assert_eq!(item.actions(), expected_actions);
        act(&fixture.client, item, action, false).await.unwrap();
    }
    let error = act(&fixture.client, &listing.items[1], Action::Start, false)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("is unavailable"));
    assert_eq!(
        *fixture.server.commands.lock(),
        [
            "docker pause 373ed0cab93b963acda242786b469e6b8d585199ee13be7566807071435a7b76 2>&1",
            "docker unpause 45bb980548d7f4256530b2ddf17b410b404cf667a78e3c936ef8364873a9c90f 2>&1",
            "docker start e3871a4e1e159b231293b48e63c3430f658239ad2616142adf52279256e9a28f 2>&1",
        ]
    );
}

#[test]
fn lifecycle_actions_wait_for_a_successful_refresh() {
    let mut browser = Browser::default();
    browser.apply_list(Ok(listing()));
    assert!(browser.can_act());

    browser.apply_action(Ok("Paused api".into()));
    assert!(!browser.can_act());
    browser.apply_list(Err("SSH request timed out".into()));
    assert!(!browser.can_act());
    browser.apply_list(Ok(listing()));
    assert!(browser.can_act());
}

#[tokio::test]
async fn container_queries_parse_real_ssh_responses_and_preserve_exit_codes() {
    let fixture = fixture().await;
    let listing = listing();
    let item = &listing.items[0];
    let samples = manager::stats(&fixture.client, &listing, false)
        .await
        .unwrap();
    assert_eq!(samples[&item.key()].cpu, Some(125.5));
    assert_eq!(samples[&item.key()].memory, "32MiB / 256MiB");
    let inspected: serde_json::Value = serde_json::from_str(
        &manager::inspect(&fixture.client, item, false)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(inspected["image"], "nginx:1.27");
    assert_eq!(logs(&fixture.client, item, false).await.unwrap(), "ready\n");
    assert_eq!(
        fixture.client.exec_limited("failure", 1024).await.unwrap(),
        ("permission denied".into(), 7)
    );
    assert_eq!(
        fixture
            .client
            .exec_limited("late-status", 1024)
            .await
            .unwrap(),
        ("failed".into(), 9)
    );
    let commands = fixture.server.commands.lock();
    assert_eq!(
        commands.iter().filter(|c| c.contains(" stats ")).count(),
        1,
        "one batch, no request per container"
    );
}

#[tokio::test]
async fn oversized_responses_fail_without_closing_the_shared_connection() {
    let fixture = fixture().await;
    let error = fixture
        .client
        .exec_limited("oversize", 64)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("exceeded"), "{error}");
    assert_eq!(
        fixture.client.exec("probe-owner").await.unwrap(),
        ("owner alive".into(), 0)
    );
}

#[tokio::test]
async fn closing_a_container_terminal_keeps_the_host_transport_alive() {
    let fixture = fixture().await;
    let mut target = fixture.target.clone();
    target.exec_command = Some("docker exec -it abcdef123456 sh".into());
    target.startup_script = Some("must never execute".into());
    let terminal = conn::spawn_ssh_channel(
        &tokio::runtime::Handle::current(),
        target,
        fixture.template.clone(),
        conn::SpawnOptions {
            scrollback: 100,
            log_dir: None,
        },
        fixture.client.clone(),
    );
    terminal.send_input(Input::Resize {
        cols: 80,
        rows: 24,
        px_w: 0,
        px_h: 0,
    });
    tokio::time::timeout(
        Duration::from_secs(5),
        fixture.server.terminal_started.notified(),
    )
    .await
    .unwrap();
    terminal.send_input(Input::Close);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(*terminal.status.lock(), Status::Closed(_)) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        fixture.client.exec("probe-owner").await.unwrap(),
        ("owner alive".into(), 0)
    );
    assert_eq!(
        fixture.server.commands.lock().as_slice(),
        ["docker exec -it abcdef123456 sh", "probe-owner"]
    );
}

async fn ready_address(tunnel: &crate::containers::ports::Tunnel) -> std::net::SocketAddr {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match tunnel.status() {
                ports::TunnelStatus::Ready(address) => return address,
                ports::TunnelStatus::Failed(error) => panic!("Tunnel failed: {error}"),
                _ => tokio::task::yield_now().await,
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn published_port_tunnel_serves_http_and_stop_releases_the_listener() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture = fixture().await;
    let remote = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = remote.local_addr().unwrap().port();
    *fixture.server.port_bindings.lock() =
        serde_json::json!({"80/tcp":[{"HostIp":"0.0.0.0", "HostPort":port.to_string()}]})
            .to_string();
    let http = tokio::spawn(async move {
        // The first connection is the readiness probe; the next carries the HTTP request.
        for _ in 0..2 {
            let (mut socket, _) = remote.accept().await.unwrap();
            let mut request = [0; 1024];
            let read = socket.read(&mut request).await.unwrap();
            if read == 0 {
                continue;
            }
            assert!(request[..read].starts_with(b"GET / HTTP/1.1"));
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 17\r\nConnection: close\r\n\r\nferrum-port-ready").await.unwrap();
        }
    });
    let binding = ports::parse(&fixture.server.port_bindings.lock())
        .unwrap()
        .remove(0);
    let mut tunnel = ports::Tunnel::start(
        &tokio::runtime::Handle::current(),
        fixture.client.clone(),
        listing().items.remove(0),
        binding,
        ports::OpenAs::Http,
        false,
        egui::Context::default(),
    );
    assert_eq!(
        tunnel.take_browser_url(),
        None,
        "a starting tunnel must not launch a browser"
    );
    let address = ready_address(&tunnel).await;
    assert_eq!(
        address.ip(),
        std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
    );
    assert_ne!(address.port(), 0);
    assert_eq!(
        tunnel.take_browser_url(),
        Some(format!("http://{address}/"))
    );
    assert_eq!(
        tunnel.take_browser_url(),
        None,
        "automatic browser launch occurs once"
    );
    let response = tokio::time::timeout(Duration::from_secs(5), async {
        let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
        socket
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut out = String::new();
        socket.read_to_string(&mut out).await.unwrap();
        out
    })
    .await
    .unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"));
    assert!(response.ends_with("ferrum-port-ready"));
    tokio::time::timeout(Duration::from_secs(5), http)
        .await
        .unwrap()
        .unwrap();
    tunnel.stop();
    assert_eq!(tunnel.status(), ports::TunnelStatus::Stopped);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if tokio::net::TcpStream::connect(address).await.is_err() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        fixture.client.exec("probe-owner").await.unwrap().0,
        "owner alive"
    );
    assert!(
        fixture
            .server
            .forwards
            .lock()
            .iter()
            .all(|(host, forwarded)| host == "127.0.0.1" && *forwarded == port as u32)
    );
}

#[tokio::test]
async fn denied_or_changed_bindings_fail_without_a_browser_request() {
    for changed in [true, false] {
        let fixture = fixture().await;
        *fixture.server.port_bindings.lock() =
            r#"{"80/tcp":[{"HostIp":"127.0.0.1","HostPort":"8080"}]}"#.into();
        let mut port = ports::parse(&fixture.server.port_bindings.lock())
            .unwrap()
            .remove(0);
        if changed {
            port.host_port = 8081;
        }
        fixture
            .server
            .deny_forward
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let mut tunnel = ports::Tunnel::start(
            &tokio::runtime::Handle::current(),
            fixture.client.clone(),
            listing().items.remove(0),
            port,
            ports::OpenAs::Http,
            false,
            egui::Context::default(),
        );
        let error = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let ports::TunnelStatus::Failed(error) = tunnel.status() {
                    break error;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!error.is_empty());
        if changed {
            assert!(error.contains("Published ports changed"));
            assert!(fixture.server.forwards.lock().is_empty());
        }
        assert_eq!(tunnel.take_browser_url(), None);
    }
}

#[tokio::test]
async fn removing_a_container_closes_its_tunnel_and_active_connections() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture = fixture().await;
    let remote = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = remote.local_addr().unwrap().port();
    *fixture.server.port_bindings.lock() =
        serde_json::json!({"80/tcp":[{"HostIp":"127.0.0.1", "HostPort":port.to_string()}]})
            .to_string();
    let echo = tokio::spawn(async move {
        loop {
            let (mut socket, _) = remote.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buffer = [0; 256];
                while let Ok(n) = socket.read(&mut buffer).await {
                    if n == 0 {
                        break;
                    }
                    if socket.write_all(&buffer[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    let item = listing().items.remove(0);
    let binding = ports::parse(&fixture.server.port_bindings.lock())
        .unwrap()
        .remove(0);
    let mut tunnel = ports::Tunnel::start(
        &tokio::runtime::Handle::current(),
        fixture.client.clone(),
        item.clone(),
        binding.clone(),
        ports::OpenAs::Tcp,
        false,
        egui::Context::default(),
    );
    let address = ready_address(&tunnel).await;
    assert_eq!(
        tunnel.take_browser_url(),
        None,
        "TCP-only mode never requests a browser"
    );
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    socket.write_all(b"held").await.unwrap();
    let mut bytes = [0; 4];
    tokio::time::timeout(Duration::from_secs(5), socket.read_exact(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes, b"held");
    let mut manager = manager::Manager::default();
    manager.tunnels.insert(binding.key(&item.key()), tunnel);
    manager.reconcile(&Listing::default());
    assert!(manager.tunnels.is_empty());
    let read = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut bytes))
        .await
        .unwrap();
    assert!(
        matches!(read, Ok(0) | Err(_)),
        "dropping a tunnel must close existing connections"
    );
    assert_eq!(
        fixture.client.exec("probe-owner").await.unwrap().0,
        "owner alive"
    );
    echo.abort();
}

#[tokio::test]
async fn published_port_tunnel_preserves_concurrent_html_css_and_script_responses() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture = fixture().await;
    let remote = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = remote.local_addr().unwrap().port();
    *fixture.server.port_bindings.lock() =
        serde_json::json!({"80/tcp":[{"HostIp":"127.0.0.1", "HostPort":port.to_string()}]})
            .to_string();
    let html = b"<!doctype html><link rel=stylesheet href=/assets/app.css><script src=/assets/app.js></script>".to_vec();
    let css = "body{color:#102030;background:#ffffff;}\n"
        .repeat(4096)
        .into_bytes();
    let js = "document.documentElement.dataset.ready='yes';\n"
        .repeat(1024)
        .into_bytes();
    let assets = Arc::new(std::collections::BTreeMap::from([
        ("/", ("text/html", html)),
        ("/assets/app.css", ("text/css", css)),
        ("/assets/app.js", ("application/javascript", js)),
    ]));
    let responses = assets.clone();
    let http = tokio::spawn(async move {
        let mut clients = tokio::task::JoinSet::new();
        // One readiness probe plus three independent browser asset requests.
        for _ in 0..4 {
            let (mut socket, _) = remote.accept().await.unwrap();
            let assets = responses.clone();
            clients.spawn(async move {
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    if socket.read(&mut byte).await.unwrap() == 0 { return; }
                    request.extend_from_slice(&byte);
                    assert!(request.len() < 2048);
                }
                let request = String::from_utf8(request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap();
                let (mime, body) = &assets[path];
                let header = format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                socket.write_all(header.as_bytes()).await.unwrap();
                socket.write_all(body).await.unwrap();
            });
        }
        while let Some(result) = clients.join_next().await {
            result.unwrap();
        }
    });
    let binding = ports::parse(&fixture.server.port_bindings.lock())
        .unwrap()
        .remove(0);
    let tunnel = ports::Tunnel::start(
        &tokio::runtime::Handle::current(),
        fixture.client.clone(),
        listing().items.remove(0),
        binding,
        ports::OpenAs::Tcp,
        false,
        egui::Context::default(),
    );
    let address = ready_address(&tunnel).await;
    let fetch = |path: &'static str| async move {
        let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
        socket
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        socket.read_to_end(&mut response).await.unwrap();
        (path, response)
    };
    let results = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(
            fetch("/"),
            fetch("/assets/app.css"),
            fetch("/assets/app.js")
        )
    })
    .await
    .unwrap();
    for (path, response) in [results.0, results.1, results.2] {
        let separator = response
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .unwrap();
        let header = std::str::from_utf8(&response[..separator]).unwrap();
        assert!(header.starts_with("HTTP/1.1 200 OK"));
        assert!(header.contains(&format!("Content-Type: {}", assets[path].0)));
        assert_eq!(
            &response[separator + 4..],
            assets[path].1.as_slice(),
            "asset bytes changed: {path}"
        );
    }
    tokio::time::timeout(Duration::from_secs(5), http)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn credentials_are_requested_on_demand_and_stopped_tunnels_clear_passwords() {
    let fixture = fixture().await;
    let remote = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let host_port = remote.local_addr().unwrap().port();
    *fixture.server.port_bindings.lock() =
        serde_json::json!({"5432/tcp":[{"HostIp":"127.0.0.1","HostPort":host_port.to_string()}]})
            .to_string();
    let mut item = listing().items.remove(0);
    item.detail = "postgres:17".into();
    let port = ports::parse(&fixture.server.port_bindings.lock())
        .unwrap()
        .remove(0);
    let mut tunnel = ports::Tunnel::start(
        &tokio::runtime::Handle::current(),
        fixture.client.clone(),
        item.clone(),
        port,
        ports::OpenAs::Tcp,
        false,
        egui::Context::default(),
    );
    let address = ready_address(&tunnel).await;
    assert!(tunnel.connection.password.is_empty());
    assert!(
        !fixture
            .server
            .commands
            .lock()
            .iter()
            .any(|c| c.contains(".Config.Env"))
    );
    tunnel.read_credentials(
        &tokio::runtime::Handle::current(),
        fixture.client.clone(),
        item,
        false,
        egui::Context::default(),
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while tunnel.credentials_pending {
            tunnel.poll_credentials();
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        tunnel.connection.uri(address, true),
        format!("postgresql://fixture:synthetic%3Dp%40ss@{address}/events")
    );
    assert_eq!(tunnel.take_browser_url(), None);
    tunnel.stop();
    assert_eq!(tunnel.connection.password, "");
    assert!(!tunnel.credentials_pending);
    assert_eq!(fixture.client.exec("owner-check").await.unwrap().1, 0);
}
