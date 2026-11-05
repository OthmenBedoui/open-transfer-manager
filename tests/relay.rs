//! Receiving and sending the same file at once, against difficult sources and destinations.
//! The result must be exactly one whole version of the source file.
use otm_core::fake_s3::{self, FakeS3, Faults};
use otm_core::lab::{self, bytes_match, Scenario};
use otm_core::relay::{relay, RelayError, RelayOptions, RelayReport};
use otm_core::s3::{Client, Config};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const MB: u64 = 1 << 20;

fn workdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("otm_relay_test_{}_{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn scenario(name: &str, size: u64) -> Scenario {
    Scenario { size, ..Scenario::named(name).unwrap() }
}

fn client(s3: &FakeS3) -> Arc<Client> {
    Arc::new(
        Client::new(Config {
            endpoint: format!("http://127.0.0.1:{}", s3.port),
            region: "us-east-1".into(),
            access_key: "test-access".into(),
            secret_key: "test-secret".into(),
        })
        .unwrap(),
    )
}

fn dest(s3: &FakeS3) -> Arc<dyn otm_core::destination::Destination> {
    Arc::new(otm_core::destination::S3Destination::new(client(s3), "bucket", "dir/my file.bin"))
}

fn no_stop() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn stored_whole(s3: &FakeS3, size: u64) {
    let st = s3.store.lock().unwrap();
    // The fake stores the encoded path it was asked for.
    let object = st.objects.get("/bucket/dir/my%20file.bin").unwrap_or_else(|| panic!("no object; has {:?}", st.objects.keys().collect::<Vec<_>>()));
    assert!(bytes_match(object, size, 0) || bytes_match(object, size, 1), "the stored object is not one whole version");
}

fn run(source: Scenario, faults: Faults, name: &str) -> (RelayReport, FakeS3) {
    let size = source.size;
    let src = lab::start(source);
    let s3 = fake_s3::start(faults);
    let dir = workdir(name);
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let r = relay(&url, &dir.join("out.bin"), dest(&s3), &RelayOptions::default(), no_stop()).unwrap_or_else(|e| panic!("{}: {}", name, e));
    stored_whole(&s3, size);
    assert_eq!(r.stored_size, size);
    let left: Vec<_> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
    assert!(left.is_empty(), "{}: leftovers {:?}", name, left);
    let _ = fs::remove_dir_all(&dir);
    (r, s3)
}

#[test]
fn parts_go_up_while_the_file_is_still_arriving() {
    let size = 96 * MB;
    let (r, _s3) = run(scenario("per-connection-limit", size), Faults::default(), "overlap");
    let at = r.received_at_first_part.expect("no part was sent");
    assert!(at < size / 2, "the first part was accepted only after {} of {} bytes arrived", at, size);
}

#[test]
fn a_slow_destination_lets_the_download_run_ahead() {
    let size = 48 * MB;
    let (r, _s3) = run(scenario("fast", size), Faults { part_bps: 8 * MB, ..Faults::default() }, "slow_destination");
    // The download doesn't wait for the slow side: most parts were still to send when it ended.
    assert!(r.download.seconds < r.seconds / 2.0, "download {:.1} s of {:.1} s", r.download.seconds, r.seconds);
}

#[test]
fn a_slow_source_makes_uploads_wait_without_failing() {
    let size = 32 * MB;
    let source = Scenario { per_conn_bps: 2 * MB, ..scenario("per-connection-limit", size) };
    let (r, _s3) = run(source, Faults::default(), "slow_source");
    assert_eq!(r.part_retries, 0);
}

#[test]
fn errors_from_storage_lose_nothing() {
    let (r, s3) = run(scenario("fast", 64 * MB), Faults { error_every: 3, ..Faults::default() }, "errors");
    assert!(r.part_retries > 0);
    assert_eq!(s3.counters.completed.load(Ordering::SeqCst), 1);
}

#[test]
fn a_storage_asking_to_slow_down_is_listened_to() {
    let (r, _s3) = run(scenario("fast", 48 * MB), Faults { slow_down_every: 3, ..Faults::default() }, "slow_down");
    assert!(r.throttled > 0, "{:?}", r);
}

#[test]
fn cut_upload_connections_lose_nothing() {
    let (r, _s3) = run(scenario("fast", 48 * MB), Faults { cut_every: 3, ..Faults::default() }, "cuts");
    assert!(r.part_retries > 0, "{:?}", r);
}

#[test]
fn a_failure_hidden_in_a_success_answer_is_seen() {
    let (_r, s3) = run(scenario("fast", 16 * MB), Faults { complete_error_once: true, ..Faults::default() }, "hidden_failure");
    assert_eq!(s3.counters.completed.load(Ordering::SeqCst), 1);
}

#[test]
fn trouble_on_the_source_side_lose_nothing() {
    run(scenario("cuts", 24 * MB), Faults::default(), "source_cuts");
    run(scenario("stalled-connections", 24 * MB), Faults::default(), "source_stalls");
}

#[test]
fn a_changed_file_never_mixes_two_versions() {
    // Late enough that some parts of the first version are already on the storage.
    let source = Scenario { change_after: 40 * MB, ..scenario("changed-file", 160 * MB) };
    let (r, s3) = run(source, Faults::default(), "changed");
    assert!(s3.counters.parts_accepted.load(Ordering::SeqCst) > r.parts as u64, "no part of the first version was sent: {:?}", r);
    assert_eq!(r.download.restarts, 1, "{:?}", r);
    assert_eq!(r.uploads_restarted, 1, "{:?}", r);
    assert_eq!(s3.counters.aborted.load(Ordering::SeqCst), 1, "the first upload should have been cancelled");
    assert_eq!(s3.counters.created.load(Ordering::SeqCst), 2);
}

#[test]
fn a_server_without_ranges_still_relays() {
    let (r, _s3) = run(scenario("no-range", 24 * MB), Faults::default(), "no_range");
    assert!(!r.download.used_ranges);
}

#[test]
fn small_and_empty_files() {
    run(scenario("fast", 1000), Faults::default(), "small");
    run(scenario("fast", 0), Faults::default(), "empty");
}

#[test]
fn a_stopped_relay_continues_without_sending_parts_again() {
    let size = 256 * MB;
    let src = lab::start(scenario("per-connection-limit", size));
    let s3 = fake_s3::start(Faults::default());
    let dir = workdir("resume");
    let work = dir.join("out.bin");
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let stop = no_stop();
    let st2 = stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(4));
        st2.store(true, Ordering::SeqCst);
    });
    let first = relay(&url, &work, dest(&s3), &RelayOptions::default(), stop);
    assert!(matches!(first, Err(RelayError::Stopped)), "{:?}", first);
    // In-flight parts may land just after the stop: only count what can no longer change.
    std::thread::sleep(Duration::from_millis(1500));
    let accepted_before = s3.counters.parts_accepted.load(Ordering::SeqCst);
    assert!(accepted_before > 0, "nothing was sent before the stop");
    assert_eq!(s3.counters.aborted.load(Ordering::SeqCst), 0, "stopping must keep the upload");

    let r = relay(&url, &work, dest(&s3), &RelayOptions::default(), no_stop()).unwrap();
    stored_whole(&s3, size);
    assert!(r.parts_resumed > 0, "{:?}", r);
    assert_eq!(s3.counters.created.load(Ordering::SeqCst), 1, "the same upload should have continued");
    assert_eq!(r.parts_sent + r.parts_resumed, r.parts, "{:?}", r);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_finished_download_is_not_fetched_again_on_resume() {
    let size = 32 * MB;
    let src = lab::start(scenario("fast", size));
    let s3 = fake_s3::start(Faults { part_bps: 2 * MB, ..Faults::default() });
    let dir = workdir("resume_upload_only");
    let work = dir.join("out.bin");
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let stop = no_stop();
    let st2 = stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        st2.store(true, Ordering::SeqCst);
    });
    let first = relay(&url, &work, dest(&s3), &RelayOptions::default(), stop);
    assert!(matches!(first, Err(RelayError::Stopped)), "{:?}", first);
    assert!(work.exists(), "the download should have finished before the stop");
    let sent = src.metrics.bytes_sent.load(Ordering::SeqCst);
    let r = relay(&url, &work, dest(&s3), &RelayOptions::default(), no_stop()).unwrap();
    stored_whole(&s3, size);
    assert_eq!(src.metrics.bytes_sent.load(Ordering::SeqCst), sent, "the file was fetched again");
    assert!(r.stored_size == size);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_local_copy_is_kept_only_when_asked() {
    let size = 16 * MB;
    let src = lab::start(scenario("fast", size));
    let s3 = fake_s3::start(Faults::default());
    let dir = workdir("keep");
    let work = dir.join("out.bin");
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let opts = RelayOptions { keep_local: true, ..RelayOptions::default() };
    relay(&url, &work, dest(&s3), &opts, no_stop()).unwrap();
    assert_eq!(fs::metadata(&work).unwrap().len(), size);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_refusal_from_the_source_stops_everything() {
    let s3 = fake_s3::start(Faults::default());
    let dir = workdir("refused");
    let free = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{}/file", free);
    let r = relay(&url, &dir.join("out.bin"), dest(&s3), &RelayOptions::default(), no_stop());
    assert!(matches!(r, Err(RelayError::Download(_))), "{:?}", r);
    assert!(s3.store.lock().unwrap().objects.is_empty());
    let _ = fs::remove_dir_all(&dir);
}
