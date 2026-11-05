//! Whole-file checks, the queue, and the command line as a script would use it.
use otm_core::destination::{Destination, S3Destination};
use otm_core::engine::Progress;
use otm_core::fake_s3::{self, FakeS3, Faults};
use otm_core::fake_sftp;
use otm_core::lab::{self, Scenario};
use otm_core::relay::sha256_of_file;
use otm_core::s3::{sha256_hex, Client, Config};
use otm_core::sftp::{HostKeys, Login, SftpDestination, Target};
use otm_core::transfer::{run_item, run_queue, Event, Item};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const MB: u64 = 1 << 20;
const OTM: &str = env!("CARGO_BIN_EXE_otm");
const WRONG: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn workdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("otm_transfer_test_{}_{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn scenario(name: &str, size: u64) -> Scenario {
    Scenario { size, ..Scenario::named(name).unwrap() }
}

fn no_stop() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn s3_dest(s3: &FakeS3, key: &str) -> Arc<dyn Destination> {
    let client = Client::new(Config { endpoint: format!("http://127.0.0.1:{}", s3.port), region: "us-east-1".into(), access_key: "a".into(), secret_key: "b".into() }).unwrap();
    Arc::new(S3Destination::new(Arc::new(client), "bucket", key))
}

fn fresh() -> Arc<Progress> {
    Arc::new(Progress::default())
}

/// The real SHA-256 of a test server's file, fetched once without any option.
fn real_sha(src: &lab::Server, dir: &Path) -> String {
    let file = dir.join("reference.bin");
    otm_core::engine::download(&format!("http://127.0.0.1:{}/file", src.port), &file, &Default::default(), no_stop()).unwrap();
    let digest = sha256_of_file(&file).unwrap();
    fs::remove_file(&file).unwrap();
    digest
}

#[test]
fn the_digest_is_the_real_sha256_of_the_file() {
    let dir = workdir("digest");
    let file = dir.join("f");
    fs::write(&file, b"abc").unwrap();
    assert_eq!(sha256_of_file(&file).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn get_checks_the_whole_file() {
    let src = lab::start(scenario("fast", 8 * MB));
    let dir = workdir("get");
    let digest = real_sha(&src, &dir);
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let none = |_: &str| -> Result<Arc<dyn Destination>, String> { Err("unused".into()) };

    let good = Item::Get { url: url.clone(), file: dir.join("good.bin"), sha256: Some(digest.clone()) };
    let progress = fresh();
    let o = run_item(&good, &none, progress.clone(), no_stop()).unwrap();
    assert_eq!(o.sha256, digest);
    assert_eq!(o.verified, "local");
    assert_eq!((progress.received(), progress.size()), (8 * MB, 8 * MB));

    let bad = Item::Get { url, file: dir.join("bad.bin"), sha256: Some(WRONG.into()) };
    let e = run_item(&bad, &none, fresh(), no_stop()).unwrap_err();
    assert!(e.contains(&digest) && e.contains(WRONG), "{}", e);
    assert!(!dir.join("bad.bin").exists(), "a wrong file stayed under the requested name");
    assert!(dir.join("bad.bin.sha256-mismatch").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_wrong_digest_never_reaches_the_final_name_on_s3() {
    let src = lab::start(scenario("fast", 24 * MB));
    let s3 = fake_s3::start(Faults::default());
    let dir = workdir("relay_s3");
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let digest = real_sha(&src, &dir);
    let dests = |_: &str| Ok(s3_dest(&s3, "f.bin"));
    let bad = Item::Relay { url: url.clone(), target: "s3://bucket/f.bin".into(), work: Some(dir.join("w")), keep_local: false, sha256: Some(WRONG.into()) };
    let e = run_item(&bad, &dests, fresh(), no_stop()).unwrap_err();
    assert!(e.contains("SHA-256"), "{}", e);
    assert!(s3.store.lock().unwrap().objects.is_empty(), "the object appeared");
    assert_eq!(s3.counters.aborted.load(Ordering::SeqCst), 1, "the upload should have been cancelled");
    let leftovers: Vec<_> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).filter(|n| n.to_string_lossy().contains("journal")).collect();
    assert!(leftovers.is_empty(), "{:?}", leftovers);

    let progress = fresh();
    let good = Item::Relay { url, target: "s3://bucket/f.bin".into(), work: Some(dir.join("w2")), keep_local: false, sha256: Some(digest.clone()) };
    let o = run_item(&good, &dests, progress.clone(), no_stop()).unwrap();
    assert_eq!(o.sha256, digest);
    assert_eq!(o.verified, "parts-signed");
    assert_eq!((progress.received(), progress.sent()), (24 * MB, 24 * MB));
    assert_eq!(sha256_hex(s3.store.lock().unwrap().objects.values().next().unwrap()), digest);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_wrong_digest_never_reaches_the_final_name_on_sftp() {
    let dir = workdir("put_sftp");
    let local = dir.join("local.bin");
    fs::write(&local, vec![7u8; (9 * MB) as usize]).unwrap();
    let digest = sha256_of_file(&local).unwrap();
    let server = fake_sftp::start(fake_sftp::Faults::default(), "pw", None);
    let target = |verify| Target {
        host: "127.0.0.1".into(),
        port: server.port,
        user: "t".into(),
        path: "/f.bin".into(),
        login: Login::Password("pw".into()),
        host_keys: HostKeys::Pinned(server.fingerprint.clone()),
        verify,
        overwrite: false,
    };
    let dests = |_: &str| Ok(Arc::new(SftpDestination::new(target(true)).unwrap()) as Arc<dyn Destination>);
    let item = |sha: &str| Item::Put { file: local.clone(), target: "sftp://t@h/f.bin".into(), sha256: Some(sha.into()) };
    let e = run_item(&item(WRONG), &dests, fresh(), no_stop()).unwrap_err();
    assert!(e.contains("SHA-256"), "{}", e);
    assert!(server.files.lock().unwrap().files.is_empty(), "something stayed on the server");
    let o = run_item(&item(&digest), &dests, fresh(), no_stop()).unwrap();
    assert_eq!(o.verified, "parts-read-back");
    assert_eq!(sha256_hex(&server.files.lock().unwrap().files["/f.bin"]), digest);
    // Saying plainly when nothing was compared.
    let unchecked = |_: &str| Ok(Arc::new(SftpDestination::new(Target { path: "/g.bin".into(), ..target(false) }).unwrap()) as Arc<dyn Destination>);
    let plain = Item::Put { file: local.clone(), target: "sftp://t@h/g.bin".into(), sha256: None };
    assert_eq!(run_item(&plain, &unchecked, fresh(), no_stop()).unwrap().verified, "parts-not-checked");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_local_file_edited_during_the_send_is_not_stored() {
    let dir = workdir("edited");
    let local = dir.join("f.bin");
    fs::write(&local, vec![1u8; (48 * MB) as usize]).unwrap();
    let server = fake_sftp::start(fake_sftp::Faults { write_delay_ms: 3, ..fake_sftp::Faults::default() }, "pw", None);
    let target = Target {
        host: "127.0.0.1".into(),
        port: server.port,
        user: "t".into(),
        path: "/f.bin".into(),
        login: Login::Password("pw".into()),
        host_keys: HostKeys::Pinned(server.fingerprint.clone()),
        verify: false,
        overwrite: false,
    };
    let dests = |_: &str| Ok(Arc::new(SftpDestination::new(target.clone()).unwrap()) as Arc<dyn Destination>);
    let edit = local.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(700));
        fs::write(&edit, vec![2u8; (48 * MB) as usize]).unwrap();
    });
    let item = Item::Put { file: local.clone(), target: "sftp://t@h/f.bin".into(), sha256: None };
    let e = run_item(&item, &dests, fresh(), no_stop()).unwrap_err();
    assert!(e.contains("changed while"), "{}", e);
    assert!(server.files.lock().unwrap().files.is_empty(), "something stayed on the server");
    let _ = fs::remove_dir_all(&dir);
}

fn events(items: &[Item], jobs: usize, state: Option<&Path>, s3: &FakeS3) -> (otm_core::transfer::Totals, Vec<String>) {
    let log = Mutex::new(vec![]);
    let dests = |target: &str| Ok(s3_dest(s3, target.rsplit('/').next().unwrap()));
    let totals = run_queue(items, jobs, state, &dests, no_stop(), &|e| {
        log.lock().unwrap().push(match e {
            Event::Started { id, .. } => format!("started {}", id),
            Event::Skipped { id, .. } => format!("skipped {}", id),
            Event::Done { id, .. } => format!("done {}", id),
            Event::Failed { id, .. } => format!("failed {}", id),
        });
    });
    let v = log.into_inner().unwrap();
    (totals, v)
}

#[test]
fn a_queue_runs_everything_and_a_failure_stops_only_itself() {
    let src = lab::start(scenario("fast", 4 * MB));
    let s3 = fake_s3::start(Faults::default());
    let dir = workdir("queue");
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let items = vec![
        Item::Get { url: url.clone(), file: dir.join("a.bin"), sha256: None },
        Item::Put { file: dir.join("also-missing.bin"), target: "s3://bucket/b.bin".into(), sha256: None },
        Item::Get { url: url.clone(), file: dir.join("c.bin"), sha256: None },
        Item::Relay { url: url.clone(), target: "s3://bucket/d.bin".into(), work: Some(dir.join("d.work")), keep_local: false, sha256: None },
        Item::Put { file: dir.join("missing.bin"), target: "s3://bucket/e.bin".into(), sha256: None },
    ];
    let state = dir.join("q.state");
    let (totals, log) = events(&items, 2, Some(&state), &s3);
    assert_eq!((totals.done, totals.failed, totals.skipped), (3, 2, 0), "{:?}", log);
    assert!(dir.join("a.bin").exists() && dir.join("c.bin").exists());
    assert_eq!(s3.store.lock().unwrap().objects.len(), 1);

    // Run again: what's done is skipped, what failed is tried again.
    let (again, _) = events(&items, 2, Some(&state), &s3);
    assert_eq!((again.done, again.failed, again.skipped), (0, 2, 3));

    // A deleted download is real work again.
    fs::remove_file(dir.join("a.bin")).unwrap();
    let (third, _) = events(&items, 2, Some(&state), &s3);
    assert_eq!((third.done, third.failed, third.skipped), (1, 2, 2));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_queue_never_runs_more_than_it_was_told() {
    let src = lab::start(scenario("per-connection-limit", 8 * MB));
    let s3 = fake_s3::start(Faults::default());
    let dir = workdir("jobs");
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let items: Vec<Item> = (0..6).map(|i| Item::Get { url: url.clone(), file: dir.join(format!("{}.bin", i)), sha256: None }).collect();
    let (totals, log) = events(&items, 2, None, &s3);
    assert_eq!(totals.done, 6);
    let mut running = 0i32;
    let mut peak = 0;
    for line in &log {
        if line.starts_with("started") {
            running += 1;
            peak = peak.max(running);
        } else {
            running -= 1;
        }
    }
    assert_eq!(peak, 2, "{:?}", log);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_changed_local_file_is_sent_again_by_a_queue() {
    let s3 = fake_s3::start(Faults::default());
    let dir = workdir("put_again");
    let local = dir.join("f.bin");
    fs::write(&local, vec![1u8; MB as usize]).unwrap();
    let state = dir.join("q.state");
    let items = vec![Item::Put { file: local.clone(), target: "s3://bucket/f.bin".into(), sha256: None }];
    assert_eq!(events(&items, 1, Some(&state), &s3).0.done, 1);
    assert_eq!(events(&items, 1, Some(&state), &s3).0.skipped, 1);
    std::thread::sleep(std::time::Duration::from_millis(50));
    fs::write(&local, vec![2u8; 2 * MB as usize]).unwrap();
    assert_eq!(events(&items, 1, Some(&state), &s3).0.done, 1);
    assert_eq!(s3.store.lock().unwrap().objects.values().next().unwrap().len() as u64, 2 * MB);
    let _ = fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------ The command

fn json_lines(text: &str) -> Vec<Value> {
    text.lines().map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON: {}: {}", l, e))).collect()
}

#[test]
fn the_command_speaks_json_to_scripts() {
    let src = lab::start(scenario("per-connection-limit", 16 * MB));
    let dir = workdir("cli");
    let digest = real_sha(&src, &dir);
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let out = Command::new(OTM).args(["get", &url]).arg(dir.join("a.bin")).args(["--json", "--sha256", &digest]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let lines = json_lines(&String::from_utf8_lossy(&out.stdout));
    assert_eq!(lines[0]["event"], "started");
    assert!(lines.iter().any(|l| l["event"] == "progress"), "no progress events: {:?}", lines);
    let last = lines.last().unwrap();
    assert_eq!(last["event"], "done");
    assert_eq!(last["result"]["sha256"], digest.as_str());
    assert_eq!(last["result"]["size"], 16 * MB);
    assert_eq!(last["transfer"]["op"], "get");

    let bad = Command::new(OTM).args(["get", &url]).arg(dir.join("b.bin")).args(["--json", "--sha256", WRONG]).output().unwrap();
    assert_eq!(bad.status.code(), Some(1));
    let lines = json_lines(&String::from_utf8_lossy(&bad.stdout));
    assert_eq!(lines.last().unwrap()["event"], "failed");
    assert!(lines.last().unwrap()["error"].as_str().unwrap().contains("SHA-256"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_command_runs_a_queue_file() {
    let src = lab::start(scenario("fast", 2 * MB));
    let dir = workdir("cli_queue");
    let url = format!("http://127.0.0.1:{}/file", src.port);
    let path_a = dir.join("a.bin").display().to_string().replace('\\', "/");
    let path_b = dir.join("b.bin").display().to_string().replace('\\', "/");
    let queue = dir.join("q.jsonl");
    fs::write(&queue, format!("# two files\n{{\"op\":\"get\",\"url\":\"{u}\",\"file\":\"{a}\"}}\n{{\"op\":\"get\",\"url\":\"{u}\",\"file\":\"{b}\"}}\n", u = url, a = path_a, b = path_b)).unwrap();
    let run = |extra: &[&str]| Command::new(OTM).arg("queue").arg(&queue).args(["--json", "--jobs", "2"]).args(extra).output().unwrap();
    let summary = |o: &std::process::Output| json_lines(&String::from_utf8_lossy(&o.stdout)).pop().unwrap();

    let first = run(&[]);
    assert_eq!(first.status.code(), Some(0));
    assert_eq!(summary(&first)["done"], 2);
    let second = run(&[]);
    assert_eq!(summary(&second)["skipped"], 2);
    let third = run(&["--restart"]);
    assert_eq!(summary(&third)["done"], 2);

    fs::write(&queue, "{\"op\":\"nope\"}\n").unwrap();
    let bad = run(&[]);
    assert_eq!(bad.status.code(), Some(2), "a wrong queue file is a usage error");
    assert!(String::from_utf8_lossy(&bad.stderr).contains("line 1"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn secrets_stay_out_of_the_output() {
    let dir = workdir("secrets");
    let local = dir.join("f.bin");
    fs::write(&local, b"x").unwrap();
    let free = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let target = format!("sftp://me:in-address-secret@127.0.0.1:{}/f.bin", free);
    let out = Command::new(OTM)
        .args(["put"])
        .arg(&local)
        .arg(&target)
        .args(["--json", "--host-key", "SHA256:x"])
        .env("OTM_SFTP_PASSWORD", "env-secret")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let all = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!all.contains("in-address-secret") && !all.contains("env-secret"), "{}", all);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn wrong_usage_is_exit_code_two() {
    for args in [vec!["nothing"], vec!["get", "only-one"], vec!["get", "u", "f", "--sha256", "short"], vec!["queue", "does-not-exist.jsonl"], vec!["queue", "x", "--jobs", "0"]] {
        let out = Command::new(OTM).args(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{:?}", args);
    }
}
