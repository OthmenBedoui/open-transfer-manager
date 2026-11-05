//! The client against OpenSSH's real `sftp-server` program, writing to a real disk.
//! The SSH layer around it is russh's own, so this checks the SFTP side, not OpenSSH's `sshd`.
//! Ignored by default, because it needs that program:
//!
//!   OTM_SFTP_SERVER=/path/to/sftp-server cargo test --release --test openssh_sftp -- --ignored --test-threads=1
//!
//! On Windows, Git for Windows includes it, as `usr/lib/ssh/sftp-server.exe`. Its paths look
//! like `/c/Users/...`, which is why `remote_path` converts them.
use otm_core::destination::Destination;
use otm_core::lab::{self, bytes_match, Scenario};
use otm_core::relay::{put, relay, RelayError, RelayOptions};
use otm_core::sftp::{HostKeys, Login, SftpDestination, Target};
use russh::keys::{Algorithm, HashAlg, PrivateKey};
use russh::server::{Auth, ChannelOpenHandle, Msg, Session};
use russh::{Channel, ChannelId};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const MB: u64 = 1 << 20;

struct Ssh {
    program: String,
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl russh::server::Handler for Ssh {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }

    async fn channel_open_session(&mut self, channel: Channel<Msg>, reply: ChannelOpenHandle, _s: &mut Session) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn subsystem_request(&mut self, id: ChannelId, name: &str, session: &mut Session) -> Result<(), Self::Error> {
        let (true, Some(channel)) = (name == "sftp", self.channels.remove(&id)) else {
            session.channel_failure(id)?;
            return Ok(());
        };
        session.channel_success(id)?;
        let mut child = tokio::process::Command::new(&self.program).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).spawn().expect("sftp-server");
        let (out, input) = (child.stdout.take().unwrap(), child.stdin.take().unwrap());
        tokio::spawn(async move {
            let mut pipes = tokio::io::join(out, input);
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut stream, &mut pipes).await;
            drop(child);
        });
        Ok(())
    }
}

struct Server {
    port: u16,
    fingerprint: String,
    _runtime: tokio::runtime::Runtime,
}

fn start_server() -> Server {
    let program = std::env::var("OTM_SFTP_SERVER").expect("set OTM_SFTP_SERVER to the sftp-server program");
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
    let config = Arc::new(russh::server::Config { keys: vec![key], ..Default::default() });
    let listener = runtime.block_on(tokio::net::TcpListener::bind(("127.0.0.1", 0))).unwrap();
    let port = listener.local_addr().unwrap().port();
    runtime.spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let (config, program) = (config.clone(), program.clone());
            tokio::spawn(async move {
                let handler = Ssh { program, channels: HashMap::new() };
                if let Ok(running) = russh::server::run_stream(config, socket, handler).await {
                    let _ = running.await;
                }
            });
        }
    });
    Server { port, fingerprint, _runtime: runtime }
}

fn workdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("otm_openssh_{}_{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// `C:\Users\me\x` as the server sees it: `/c/Users/me/x`.
fn remote_path(p: &Path) -> String {
    let text = p.to_string_lossy().replace('\\', "/");
    match text.split_once(":/") {
        Some((drive, rest)) => format!("/{}/{}", drive.to_lowercase(), rest),
        None => text,
    }
}

fn dest(server: &Server, local: &Path, overwrite: bool) -> Arc<dyn Destination> {
    Arc::new(
        SftpDestination::new(Target {
            host: "127.0.0.1".into(),
            port: server.port,
            user: "tester".into(),
            path: remote_path(local),
            login: Login::Password("any".into()),
            host_keys: HostKeys::Pinned(server.fingerprint.clone()),
            verify: true,
            overwrite,
        })
        .unwrap(),
    )
}

fn scenario(name: &str, size: u64) -> Scenario {
    Scenario { size, ..Scenario::named(name).unwrap() }
}

fn no_stop() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn on_disk(path: &Path, size: u64) {
    let data = fs::read(path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e));
    assert!(bytes_match(&data, size, 0), "{}: not the file that was sent", path.display());
    let mut temp = path.as_os_str().to_owned();
    temp.push(".otmpart");
    assert!(!Path::new(&temp).exists(), "the temporary file stayed behind");
}

#[test]
#[ignore]
fn relay_to_a_real_disk() {
    let server = start_server();
    let size = 96 * MB;
    let src = lab::start(scenario("per-connection-limit", size));
    let dir = workdir("relay");
    let final_path = dir.join("remote").join("sub folder").join("été فيلم (1).bin");
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let r = relay(&url, &dir.join("work.bin"), dest(&server, &final_path, false), &RelayOptions::default(), no_stop()).unwrap();
    on_disk(&final_path, size);
    let at = r.received_at_first_part.unwrap();
    assert!(at < size / 2, "first part only after {} of {}", at, size);
    println!("REAL relay: {:.1} s, first part accepted at {} of {} MB", r.seconds, at / MB, size / MB);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn small_and_empty_files_on_a_real_disk() {
    let server = start_server();
    let dir = workdir("sizes");
    for size in [0, 1, 1000, 8 * MB, 8 * MB + 1] {
        let src = lab::start(scenario("fast", size));
        let final_path = dir.join(format!("f{}.bin", size));
        let url = format!("http://127.0.0.1:{}/file", src.port);
        relay(&url, &dir.join("work"), dest(&server, &final_path, false), &RelayOptions::default(), no_stop()).unwrap_or_else(|e| panic!("{} bytes: {}", size, e));
        on_disk(&final_path, size);
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn stopped_and_continued_on_a_real_disk() {
    let server = start_server();
    let size = 256 * MB;
    let src = lab::start(scenario("per-connection-limit", size));
    let dir = workdir("resume");
    let final_path = dir.join("f.bin");
    let work = dir.join("work.bin");
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let stop = no_stop();
    let st2 = stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        st2.store(true, Ordering::SeqCst);
    });
    let first = relay(&url, &work, dest(&server, &final_path, false), &RelayOptions::default(), stop);
    assert!(matches!(first, Err(RelayError::Stopped)), "{:?}", first);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!final_path.exists(), "the file must not appear before it is complete");
    let r = relay(&url, &work, dest(&server, &final_path, false), &RelayOptions::default(), no_stop()).unwrap();
    assert!(r.parts_resumed > 0, "{:?}", r);
    assert_eq!(r.parts_sent + r.parts_resumed, r.parts, "{:?}", r);
    on_disk(&final_path, size);
    println!("REAL resume: {} parts kept, {} sent after the stop", r.parts_resumed, r.parts_sent);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn an_existing_file_is_protected_then_replaced_on_request() {
    let server = start_server();
    let size = 2 * MB;
    let src = lab::start(scenario("fast", size));
    let dir = workdir("exists");
    let final_path = dir.join("f.bin");
    fs::write(&final_path, b"precious").unwrap();
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let r = relay(&url, &dir.join("a"), dest(&server, &final_path, false), &RelayOptions::default(), no_stop());
    assert!(matches!(&r, Err(RelayError::Upload(m)) if m.contains("already exists")), "{:?}", r);
    assert_eq!(fs::read(&final_path).unwrap(), b"precious");
    relay(&url, &dir.join("b"), dest(&server, &final_path, true), &RelayOptions::default(), no_stop()).unwrap();
    on_disk(&final_path, size);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn a_local_file_is_sent_and_stays() {
    let server = start_server();
    let size = 40 * MB;
    let src = lab::start(scenario("fast", size));
    let dir = workdir("put");
    let local = dir.join("local.bin");
    otm_core::engine::download(&format!("http://127.0.0.1:{}/file", src.port), &local, &Default::default(), no_stop()).unwrap();
    let final_path = dir.join("up").join("f.bin");
    put(&local, dest(&server, &final_path, false), &RelayOptions::default(), no_stop()).unwrap();
    on_disk(&final_path, size);
    on_disk(&local, size);
    let _ = fs::remove_dir_all(&dir);
}
