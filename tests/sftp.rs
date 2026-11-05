//! The relay and plain uploads over SFTP, on real SSH connections to a server that misbehaves.
use otm_core::destination::Destination;
use otm_core::fake_sftp::{self, FakeSftp, Faults};
use otm_core::lab::{self, bytes_match, Scenario};
use otm_core::relay::{put, relay, RelayError, RelayOptions, RelayReport};
use otm_core::sftp::{HostKeys, Login, SftpDestination, Target};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const MB: u64 = 1 << 20;
const PASSWORD: &str = "test-password";

fn workdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("otm_sftp_test_{}_{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn scenario(name: &str, size: u64) -> Scenario {
    Scenario { size, ..Scenario::named(name).unwrap() }
}

fn target(server: &FakeSftp, path: &str) -> Target {
    Target {
        host: "127.0.0.1".into(),
        port: server.port,
        user: "tester".into(),
        path: path.into(),
        login: Login::Password(PASSWORD.into()),
        host_keys: HostKeys::Pinned(server.fingerprint.clone()),
        verify: true,
        overwrite: false,
    }
}

fn dest(t: Target) -> Arc<dyn Destination> {
    Arc::new(SftpDestination::new(t).unwrap())
}

fn no_stop() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn stored(server: &FakeSftp, path: &str, size: u64) {
    let st = server.files.lock().unwrap();
    let file = st.files.get(path).unwrap_or_else(|| panic!("{} is not on the server; it has {:?}", path, st.files.keys().collect::<Vec<_>>()));
    assert!(bytes_match(file, size, 0) || bytes_match(file, size, 1), "{}: not one whole version of the file", path);
    assert!(!st.files.contains_key(&format!("{}.otmpart", path)), "the temporary file stayed behind");
}

fn run(source: Scenario, faults: Faults, name: &str) -> (RelayReport, FakeSftp) {
    let size = source.size;
    let src = lab::start(source);
    let server = fake_sftp::start(faults, PASSWORD, None);
    let dir = workdir(name);
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let path = "/backup 2026/my file.bin";
    let r = relay(&url, &dir.join("out.bin"), dest(target(&server, path)), &RelayOptions::default(), no_stop()).unwrap_or_else(|e| panic!("{}: {}", name, e));
    stored(&server, path, size);
    assert_eq!(r.stored_size, size);
    let left: Vec<_> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
    assert!(left.is_empty(), "{}: leftovers {:?}", name, left);
    let _ = fs::remove_dir_all(&dir);
    (r, server)
}

#[test]
fn parts_go_up_while_the_file_is_still_arriving() {
    let size = 96 * MB;
    let (r, _s) = run(scenario("per-connection-limit", size), Faults::default(), "overlap");
    let at = r.received_at_first_part.expect("no part was sent");
    assert!(at < size / 2, "the first part was accepted only after {} of {}", at, size);
}

#[test]
fn write_errors_lose_nothing() {
    let (r, _s) = run(scenario("fast", 64 * MB), Faults { error_every: 700, ..Faults::default() }, "errors");
    assert!(r.part_retries > 0, "{:?}", r);
}

#[test]
fn cut_connections_lose_nothing() {
    let (r, s) = run(scenario("fast", 64 * MB), Faults { cut_every: 700, ..Faults::default() }, "cuts");
    assert!(r.part_retries > 0, "{:?}", r);
    assert!(s.counters.connections.load(Ordering::SeqCst) > 1);
}

#[test]
fn silent_damage_is_found_by_reading_back() {
    let (r, _s) = run(scenario("fast", 16 * MB), Faults { corrupt_write: 5, ..Faults::default() }, "corrupt");
    assert!(r.part_retries > 0, "the damage went unnoticed: {:?}", r);
}

#[test]
fn without_reading_back_damage_goes_unseen() {
    // Documents what turning the check off costs.
    let size = 16 * MB;
    let src = lab::start(scenario("fast", size));
    let server = fake_sftp::start(Faults { corrupt_write: 5, ..Faults::default() }, PASSWORD, None);
    let dir = workdir("corrupt_unverified");
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let t = Target { verify: false, ..target(&server, "/f.bin") };
    relay(&url, &dir.join("out.bin"), dest(t), &RelayOptions::default(), no_stop()).unwrap();
    assert!(!bytes_match(&server.files.lock().unwrap().files["/f.bin"], size, 0));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_slow_destination_lets_the_download_run_ahead() {
    let (r, _s) = run(scenario("fast", 32 * MB), Faults { write_delay_ms: 40, ..Faults::default() }, "slow");
    assert!(r.download.seconds < r.seconds / 2.0, "download {:.1} s of {:.1} s", r.download.seconds, r.seconds);
}

#[test]
fn a_changed_file_never_mixes_two_versions() {
    let source = Scenario { change_after: 40 * MB, ..scenario("changed-file", 160 * MB) };
    let (r, s) = run(source, Faults::default(), "changed");
    assert_eq!(r.uploads_restarted, 1, "{:?}", r);
    assert!(s.counters.removes.load(Ordering::SeqCst) >= 1, "the first attempt was not cleaned up");
}

#[test]
fn small_and_empty_files() {
    run(scenario("fast", 1000), Faults::default(), "small");
    run(scenario("fast", 0), Faults::default(), "empty");
    run(scenario("fast", 8 * MB), Faults::default(), "one_part");
    run(scenario("fast", 8 * MB + 1), Faults::default(), "over_one_part");
}

#[test]
fn a_stopped_relay_continues_without_sending_parts_again() {
    let size = 256 * MB;
    let src = lab::start(scenario("per-connection-limit", size));
    let server = fake_sftp::start(Faults::default(), PASSWORD, None);
    let dir = workdir("resume");
    let work = dir.join("out.bin");
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let stop = no_stop();
    let st2 = stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(4));
        st2.store(true, Ordering::SeqCst);
    });
    let first = relay(&url, &work, dest(target(&server, "/f.bin")), &RelayOptions::default(), stop);
    assert!(matches!(first, Err(RelayError::Stopped)), "{:?}", first);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(server.counters.removes.load(Ordering::SeqCst), 0, "stopping must keep what was sent");
    let r = relay(&url, &work, dest(target(&server, "/f.bin")), &RelayOptions::default(), no_stop()).unwrap();
    stored(&server, "/f.bin", size);
    assert!(r.parts_resumed > 0, "{:?}", r);
    assert_eq!(r.parts_sent + r.parts_resumed, r.parts, "{:?}", r);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_deleted_temporary_file_means_starting_the_upload_again() {
    let size = 128 * MB;
    let src = lab::start(scenario("per-connection-limit", size));
    let server = fake_sftp::start(Faults::default(), PASSWORD, None);
    let dir = workdir("temp_gone");
    let work = dir.join("out.bin");
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let stop = no_stop();
    let st2 = stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        st2.store(true, Ordering::SeqCst);
    });
    let first = relay(&url, &work, dest(target(&server, "/f.bin")), &RelayOptions::default(), stop);
    assert!(matches!(first, Err(RelayError::Stopped)), "{:?}", first);
    std::thread::sleep(Duration::from_millis(1500));
    server.files.lock().unwrap().files.remove("/f.bin.otmpart");
    let r = relay(&url, &work, dest(target(&server, "/f.bin")), &RelayOptions::default(), no_stop()).unwrap();
    assert_eq!(r.parts_resumed, 0, "{:?}", r);
    stored(&server, "/f.bin", size);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_existing_file_is_kept_unless_asked() {
    let size = MB;
    let src = lab::start(scenario("fast", size));
    let server = fake_sftp::start(Faults::default(), PASSWORD, None);
    server.files.lock().unwrap().files.insert("/f.bin".into(), b"precious".to_vec());
    let dir = workdir("exists");
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let r = relay(&url, &dir.join("a"), dest(target(&server, "/f.bin")), &RelayOptions::default(), no_stop());
    assert!(matches!(&r, Err(RelayError::Upload(m)) if m.contains("already exists")), "{:?}", r);
    assert_eq!(server.files.lock().unwrap().files["/f.bin"], b"precious");
    let t = Target { overwrite: true, ..target(&server, "/f.bin") };
    relay(&url, &dir.join("b"), dest(t), &RelayOptions::default(), no_stop()).unwrap();
    stored(&server, "/f.bin", size);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn nothing_goes_to_a_server_with_an_unexpected_key() {
    let src = lab::start(scenario("fast", MB));
    let server = fake_sftp::start(Faults::default(), PASSWORD, None);
    let dir = workdir("wrong_key");
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let t = Target { host_keys: HostKeys::Pinned("SHA256:not-this-one".into()), ..target(&server, "/f.bin") };
    let r = relay(&url, &dir.join("a"), dest(t), &RelayOptions::default(), no_stop());
    assert!(matches!(&r, Err(RelayError::Upload(m)) if m.contains("not the expected")), "{:?}", r);
    assert!(server.files.lock().unwrap().files.is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn known_hosts_decides_who_is_trusted() {
    let src = lab::start(scenario("fast", MB));
    let server = fake_sftp::start(Faults::default(), PASSWORD, None);
    let dir = workdir("known_hosts");
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let file = dir.join("known_hosts");
    // Unknown: refused, and the message gives the fingerprint to compare.
    let t = Target { host_keys: HostKeys::KnownHosts(file.clone()), ..target(&server, "/a.bin") };
    let r = relay(&url, &dir.join("a"), dest(t.clone()), &RelayOptions::default(), no_stop());
    assert!(matches!(&r, Err(RelayError::Upload(m)) if m.contains(&server.fingerprint)), "{:?}", r);
    // Accepted once on purpose, then trusted.
    let first = Target { host_keys: HostKeys::AcceptNew(file.clone()), ..t.clone() };
    relay(&url, &dir.join("b"), dest(first), &RelayOptions::default(), no_stop()).unwrap();
    let again = Target { path: "/c.bin".into(), ..t.clone() };
    relay(&url, &dir.join("c"), dest(again), &RelayOptions::default(), no_stop()).unwrap();
    // Another server on the same address: refused loudly, even in accept-new mode.
    let port = server.port;
    let other = fake_sftp::start(Faults::default(), PASSWORD, None);
    let text = fs::read_to_string(&file).unwrap().replace(&port.to_string(), &other.port.to_string());
    fs::write(&file, text).unwrap();
    let changed = Target { port: other.port, path: "/d.bin".into(), host_keys: HostKeys::AcceptNew(file.clone()), ..t };
    let r = relay(&url, &dir.join("d"), dest(changed), &RelayOptions::default(), no_stop());
    assert!(matches!(&r, Err(RelayError::Upload(m)) if m.contains("CHANGED")), "{:?}", r);
    assert!(other.files.lock().unwrap().files.is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn logins() {
    let src = lab::start(scenario("fast", MB));
    let key = russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519).unwrap();
    let server = fake_sftp::start(Faults::default(), PASSWORD, Some(key.public_key().clone()));
    let dir = workdir("logins");
    let url = format!("http://127.0.0.1:{}/file", src.port);
    // A wrong password stops at once, with no pointless retries.
    let bad = Target { login: Login::Password("wrong".into()), ..target(&server, "/a.bin") };
    let began = std::time::Instant::now();
    let r = relay(&url, &dir.join("a"), dest(bad), &RelayOptions::default(), no_stop());
    assert!(matches!(&r, Err(RelayError::Upload(m)) if m.contains("refused the login")), "{:?}", r);
    assert!(!format!("{:?}", r).contains("wrong"), "the password appeared in an error");
    assert!(began.elapsed() < Duration::from_secs(10));
    // A key file.
    let key_file = dir.join("id_ed25519");
    fs::write(&key_file, key.to_openssh(russh::keys::ssh_key::LineEnding::LF).unwrap().as_bytes()).unwrap();
    let t = Target { login: Login::KeyFile { path: key_file, passphrase: None }, ..target(&server, "/b.bin") };
    relay(&url, &dir.join("b"), dest(t), &RelayOptions::default(), no_stop()).unwrap();
    stored(&server, "/b.bin", MB);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_unreachable_server_is_reported_soon() {
    let src = lab::start(scenario("fast", MB));
    let server = fake_sftp::start(Faults::default(), PASSWORD, None);
    let free = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let dir = workdir("unreachable");
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let t = Target { port: free, ..target(&server, "/f.bin") };
    let began = std::time::Instant::now();
    let r = relay(&url, &dir.join("a"), dest(t), &RelayOptions::default(), no_stop());
    assert!(matches!(&r, Err(RelayError::Upload(m)) if m.contains("cannot connect")), "{:?}", r);
    assert!(began.elapsed() < Duration::from_secs(40), "took {:?}", began.elapsed());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_folder_without_permission_is_refused_clearly() {
    let src = lab::start(scenario("fast", MB));
    let server = fake_sftp::start(Faults::default(), PASSWORD, None);
    let dir = workdir("denied");
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let began = std::time::Instant::now();
    let r = relay(&url, &dir.join("a"), dest(target(&server, "/readonly/f.bin")), &RelayOptions::default(), no_stop());
    assert!(matches!(r, Err(RelayError::Upload(_))), "{:?}", r);
    assert!(began.elapsed() < Duration::from_secs(10));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_local_file_can_be_sent_and_stays_in_place() {
    let dir = workdir("put");
    let size = 48 * MB;
    // The same predictable content the checks know: fetched once from the test server.
    let src = lab::start(scenario("fast", size));
    let local = dir.join("local file.bin");
    otm_core::engine::download(&format!("http://127.0.0.1:{}/file", src.port), &local, &Default::default(), no_stop()).unwrap();
    let server = fake_sftp::start(Faults { cut_every: 700, ..Faults::default() }, PASSWORD, None);
    let r = put(&local, dest(target(&server, "/up/f.bin")), &RelayOptions::default(), no_stop()).unwrap();
    assert!(r.parts_sent > 0);
    stored(&server, "/up/f.bin", size);
    assert_eq!(fs::metadata(&local).unwrap().len(), size, "the local file must stay");
    assert!(!fs::read_dir(&dir).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("journal")));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_changed_local_file_is_not_continued() {
    let dir = workdir("put_changed");
    let local = dir.join("f.bin");
    let src = lab::start(scenario("fast", 64 * MB));
    otm_core::engine::download(&format!("http://127.0.0.1:{}/file", src.port), &local, &Default::default(), no_stop()).unwrap();
    let server = fake_sftp::start(Faults { write_delay_ms: 15, ..Faults::default() }, PASSWORD, None);
    let stop = no_stop();
    let st2 = stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        st2.store(true, Ordering::SeqCst);
    });
    let first = put(&local, dest(target(&server, "/f.bin")), &RelayOptions::default(), stop);
    assert!(matches!(first, Err(RelayError::Stopped)), "{:?}", first);
    std::thread::sleep(Duration::from_millis(1100));
    // Same size, new content and date.
    let mut data = fs::read(&local).unwrap();
    data[0] ^= 0xFF;
    std::thread::sleep(Duration::from_millis(1100));
    fs::write(&local, &data).unwrap();
    let r = put(&local, dest(target(&server, "/f.bin")), &RelayOptions::default(), no_stop()).unwrap();
    assert_eq!(r.parts_resumed, 0, "{:?}", r);
    let stored_bytes = server.files.lock().unwrap().files["/f.bin"].clone();
    assert!(stored_bytes == data, "the server's file differs from the new local file");
    let _ = fs::remove_dir_all(&dir);
}
