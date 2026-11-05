//! Every difficult server: the result must be exactly the file the server holds now.
use otm_core::engine::{download, Error, Options};
use otm_core::lab::{self, file_matches, Scenario};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MB: u64 = 1 << 20;

fn workdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("otm_test_{}_{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// The bench's scenario, on a smaller file so tests stay quick.
fn smaller(name: &str, size: u64) -> Scenario {
    Scenario { size, ..Scenario::named(name).unwrap() }
}

fn check(s: Scenario) -> otm_core::engine::Report {
    let name = s.name;
    let size = s.size;
    let server = lab::start(s);
    let dir = workdir(&format!("{}_{}", name, size));
    let dest = dir.join("out.bin");
    let url = format!("http://127.0.0.1:{}/file", server.port);
    let r = download(&url, &dest, &Options::default(), Arc::new(AtomicBool::new(false))).unwrap_or_else(|e| panic!("{}: {}", name, e));
    // One whole version of the file, never a mix of two.
    let whole = file_matches(&dest, size, 0).unwrap() || file_matches(&dest, size, 1).unwrap();
    assert!(whole, "{}: the file is not one whole version: {:?}", name, r);
    assert!(!dir.join("out.bin.part").exists() && !dir.join("out.bin.part.journal").exists(), "{}: leftovers", name);
    let _ = fs::remove_dir_all(&dir);
    r
}

#[test]
fn fast() {
    let r = check(smaller("fast", 64 * MB));
    assert!(r.used_ranges && r.checked_for_changes);
}

#[test]
fn limit_per_connection_brings_more_connections() {
    let r = check(smaller("per-connection-limit", 24 * MB));
    assert!(r.peak_connections > 2, "stayed at {} connections", r.peak_connections);
}

#[test]
fn saturated_line() {
    check(smaller("saturated-line", 24 * MB));
}

#[test]
fn slow_connections_are_replaced() {
    let r = check(smaller("slow-connections", 24 * MB));
    assert!(r.replaced_slow > 0 || r.doubled_pieces > 0, "{:?}", r);
}

#[test]
fn stalled_connections_are_replaced() {
    let r = check(smaller("stalled-connections", 24 * MB));
    // Given up by the monitor or by the read timeout: either way, the rest was fetched again.
    assert!(r.replaced_slow + r.retries > 0, "{:?}", r);
}

#[test]
fn a_server_asking_to_slow_down_is_listened_to() {
    // A lower limit, so that even a small file reaches it.
    let s = Scenario { max_active: 2, ..smaller("server-429", 24 * MB) };
    let r = check(s);
    assert!(r.throttled > 0, "the test never hit the limit: {:?}", r);
}

#[test]
fn cuts_lose_nothing() {
    let r = check(smaller("cuts", 24 * MB));
    assert!(r.retries > 0, "{:?}", r);
}

#[test]
fn a_changed_file_starts_over_instead_of_mixing() {
    let s = Scenario { change_after: 6 * MB, ..smaller("changed-file", 48 * MB) };
    let r = check(s);
    assert_eq!(r.restarts, 1, "{:?}", r);
}

#[test]
fn a_server_without_ranges_still_works() {
    let r = check(smaller("no-range", 8 * MB));
    assert!(!r.used_ranges);
}

#[test]
fn small_files_and_empty_ones() {
    check(smaller("fast", 0));
    check(smaller("fast", 1));
    check(smaller("fast", 1000));
}

#[test]
fn a_stopped_download_continues_without_fetching_again() {
    // Big enough that the engine, however quick it gets, is still far from done at the stop.
    let size = 256 * MB;
    let s = smaller("per-connection-limit", size);
    let server = lab::start(s);
    let dir = workdir("resume");
    let dest = dir.join("out.bin");
    let url = format!("http://127.0.0.1:{}/file", server.port);
    let stop = Arc::new(AtomicBool::new(false));
    let st2 = stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        st2.store(true, Ordering::SeqCst);
    });
    let first = download(&url, &dest, &Options::default(), stop);
    assert!(matches!(first, Err(Error::Stopped)), "{:?}", first);
    assert!(dir.join("out.bin.part.journal").exists(), "no journal was kept");
    let sent_before = server.metrics.bytes_sent.load(Ordering::SeqCst);
    assert!(sent_before > 2 * MB, "too little to judge: {}", sent_before);
    let began = Instant::now();
    let r = download(&url, &dest, &Options::default(), Arc::new(AtomicBool::new(false))).unwrap();
    assert!(r.resumed_bytes > MB, "{:?}", r);
    assert!(file_matches(&dest, size, 0).unwrap());
    let second_run = server.metrics.bytes_sent.load(Ordering::SeqCst) - sent_before;
    // What was safely on disk isn't asked for again. The margin is for pieces fetched twice.
    let needed = size - r.resumed_bytes;
    assert!(second_run < needed + 16 * MB, "needed {}, sent {}, took {:?}", needed, second_run, began.elapsed());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_changed_file_while_stopped_is_not_continued() {
    let server = lab::start(smaller("fast", 4 * MB));
    let dir = workdir("journal");
    let dest = dir.join("out.bin");
    fs::write(dir.join("out.bin.part"), vec![9u8; (4 * MB) as usize]).unwrap();
    fs::write(dir.join("out.bin.part.journal"), format!("otm-journal 1\nsize {}\netag \"another\"\nmodified x\ndone 0 {}\nend\n", 4 * MB, 2 * MB)).unwrap();
    let url = format!("http://127.0.0.1:{}/file", server.port);
    let r = download(&url, &dest, &Options::default(), Arc::new(AtomicBool::new(false))).unwrap();
    assert_eq!(r.resumed_bytes, 0);
    assert!(file_matches(&dest, 4 * MB, 0).unwrap());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn nothing_listening_is_an_error() {
    let dir = workdir("refused");
    let dest = dir.join("out.bin");
    // Nothing is listening on this port.
    let free = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{}/file", free);
    let began = Instant::now();
    assert!(download(&url, &dest, &Options::default(), Arc::new(AtomicBool::new(false))).is_err());
    assert!(began.elapsed() < Duration::from_secs(60));
    let _ = fs::remove_dir_all(&dir);
}
