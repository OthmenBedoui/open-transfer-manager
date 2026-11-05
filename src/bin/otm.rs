//! The command line.
use otm_core::destination::{Destination, S3Destination};
use otm_core::engine::Progress;
use otm_core::s3::{Client, Config};
use otm_core::sftp::{parse_address, HostKeys, Login, SftpDestination, Target};
use otm_core::transfer::{load_queue, run_item, run_queue, safe_address, Event, Item, Outcome};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const USAGE: &str = "usage:
  otm get URL FILE [options]
  otm relay URL TARGET [options]
  otm put FILE TARGET [options]
  otm queue FILE [options]

TARGET is s3://BUCKET/KEY or sftp://USER@HOST[:PORT]/PATH

Options:
  --sha256 HEX        fail unless the file has this SHA-256
  --json              print one JSON object per event on standard output
  --work FILE         relay: the local file used on the way
  --keep-local        relay: keep that file at the end
  --jobs N            queue: how many transfers run at once (default 2)
  --restart           queue: forget what was already finished

S3 options:
  --endpoint URL      required
  --region NAME       default us-east-1
  Keys come from AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY.

SFTP options:
  --identity FILE     private key, default ~/.ssh/id_ed25519
  --host-key SHA256:  trust only this server key
  --accept-new-host-key  trust a first-seen key and remember it in ~/.ssh/known_hosts
  --no-verify         don't read pieces back to compare them
  --overwrite         replace a file that already exists
  The password comes from OTM_SFTP_PASSWORD, a key's passphrase from OTM_SFTP_KEY_PASSPHRASE.
  Without these, the server must already be in ~/.ssh/known_hosts.

A queue file holds one JSON object per line:
  {\"op\":\"get\",\"url\":\"https://example.com/a.zip\",\"file\":\"a.zip\"}
  {\"op\":\"relay\",\"url\":\"https://example.com/b.zip\",\"target\":\"s3://bucket/b.zip\"}
  {\"op\":\"put\",\"file\":\"c.zip\",\"target\":\"sftp://me@example.org/c.zip\",\"sha256\":\"...\"}

Exit codes: 0 all done, 1 a transfer failed, 2 wrong usage.";

fn stop_with(message: &str) -> ! {
    eprintln!("{}", message);
    std::process::exit(2);
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default()
}

#[derive(Default)]
struct Flags {
    endpoint: Option<String>,
    region: Option<String>,
    work: Option<PathBuf>,
    keep_local: bool,
    identity: Option<PathBuf>,
    host_key: Option<String>,
    accept_new: bool,
    no_verify: bool,
    overwrite: bool,
    sha256: Option<String>,
    json: bool,
    jobs: Option<usize>,
    restart: bool,
}

fn read_flags(args: &[String]) -> Flags {
    let mut f = Flags::default();
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let mut value = || rest.next().cloned().unwrap_or_else(|| stop_with(USAGE));
        match flag.as_str() {
            "--endpoint" => f.endpoint = Some(value()),
            "--region" => f.region = Some(value()),
            "--work" => f.work = Some(PathBuf::from(value())),
            "--identity" => f.identity = Some(PathBuf::from(value())),
            "--host-key" => f.host_key = Some(value()),
            "--sha256" => {
                let v = value();
                if v.len() != 64 || !v.bytes().all(|b| b.is_ascii_hexdigit()) {
                    stop_with("--sha256 needs 64 hexadecimal characters");
                }
                f.sha256 = Some(v.to_lowercase());
            }
            "--jobs" => f.jobs = Some(value().parse().ok().filter(|n| *n > 0).unwrap_or_else(|| stop_with("--jobs needs a number above 0"))),
            "--keep-local" => f.keep_local = true,
            "--accept-new-host-key" => f.accept_new = true,
            "--no-verify" => f.no_verify = true,
            "--overwrite" => f.overwrite = true,
            "--json" => f.json = true,
            "--restart" => f.restart = true,
            _ => stop_with(USAGE),
        }
    }
    f
}

/// Secrets only ever come from the environment, where other programs can't read them in a process list.
fn destination(target: &str, f: &Flags) -> Result<Arc<dyn Destination>, String> {
    if let Some(rest) = target.strip_prefix("s3://") {
        let Some((bucket, key)) = rest.split_once('/').filter(|(b, k)| !b.is_empty() && !k.is_empty()) else {
            return Err(format!("not a valid target: {}", target));
        };
        let endpoint = f.endpoint.clone().ok_or("s3 needs --endpoint")?;
        let (Ok(access_key), Ok(secret_key)) = (std::env::var("AWS_ACCESS_KEY_ID"), std::env::var("AWS_SECRET_ACCESS_KEY")) else {
            return Err("set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY first".into());
        };
        let region = f.region.clone().unwrap_or_else(|| "us-east-1".into());
        let client = Client::new(Config { endpoint, region, access_key, secret_key }).map_err(|e| e.to_string())?;
        return Ok(Arc::new(S3Destination::new(Arc::new(client), bucket, key)));
    }
    let Some((host, port, user, path)) = parse_address(target) else {
        return Err(format!("not a valid target: {}", safe_address(target)));
    };
    let known = home().join(".ssh").join("known_hosts");
    let host_keys = match (&f.host_key, f.accept_new) {
        (Some(k), _) => HostKeys::Pinned(k.clone()),
        (None, true) => HostKeys::AcceptNew(known),
        (None, false) => HostKeys::KnownHosts(known),
    };
    let login = match std::env::var("OTM_SFTP_PASSWORD") {
        Ok(p) => Login::Password(p),
        Err(_) => Login::KeyFile {
            path: f.identity.clone().unwrap_or_else(|| home().join(".ssh").join("id_ed25519")),
            passphrase: std::env::var("OTM_SFTP_KEY_PASSPHRASE").ok(),
        },
    };
    let t = Target { host, port, user, path, login, host_keys, verify: !f.no_verify, overwrite: f.overwrite };
    Ok(Arc::new(SftpDestination::new(t).map_err(|e| e.to_string())?))
}

fn print_json(v: Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", v);
    let _ = out.flush();
}

fn megabytes(n: u64) -> String {
    format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
}

type Active = Arc<Mutex<HashMap<usize, Arc<Progress>>>>;

/// Reports the transfers in progress once a second until told to stop.
fn start_reporter(json_mode: bool, active: Active, finished: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let live_line = !json_mode && std::io::stderr().is_terminal();
        let mut last: HashMap<usize, (u64, u64, Instant)> = HashMap::new();
        let mut next_report = Instant::now();
        let mut drew = false;
        while !finished.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
            let now = Instant::now();
            if now < next_report {
                continue;
            }
            next_report = now + Duration::from_secs(1);
            let snapshot: Vec<(usize, Arc<Progress>)> = active.lock().unwrap().iter().map(|(k, v)| (*k, v.clone())).collect();
            let (mut received_speed, mut sent_speed, mut received, mut sent, mut size) = (0.0, 0.0, 0, 0, 0);
            for (id, p) in &snapshot {
                let (r, s, t) = (p.received(), p.sent(), p.size());
                let (rs, ss) = match last.insert(*id, (r, s, now)) {
                    Some((br, bs, at)) => {
                        let dt = now.duration_since(at).as_secs_f64().max(0.001);
                        (r.saturating_sub(br) as f64 / dt, s.saturating_sub(bs) as f64 / dt)
                    }
                    None => (0.0, 0.0),
                };
                received_speed += rs;
                sent_speed += ss;
                received += r;
                sent += s;
                size += t;
                if json_mode {
                    print_json(json!({ "event": "progress", "id": id, "size": t, "received": r, "sent": s, "received_per_second": rs as u64, "sent_per_second": ss as u64 }));
                }
            }
            if live_line && !snapshot.is_empty() {
                let percent = if size > 0 { received * 100 / size } else { 0 };
                let up = if sent > 0 || sent_speed > 0.0 { format!(", sent {} at {}/s", megabytes(sent), megabytes(sent_speed as u64)) } else { String::new() };
                eprint!("\r\x1b[K{}% of {}, received at {}/s{}", percent, megabytes(size), megabytes(received_speed as u64), up);
                drew = true;
            }
        }
        if drew {
            eprint!("\r\x1b[K");
        }
    })
}

fn done_text(item: &Item, o: &Outcome) -> String {
    let check = match o.verified.as_str() {
        "parts-read-back" => " (each part read back and compared)",
        "parts-signed" => " (each part checked by the storage)",
        "parts-not-checked" => " (parts not checked)",
        _ => "",
    };
    format!("done: {} -> {}: {} in {:.1} s\nsha256: {}{}", item.source(), item.target(), megabytes(o.size), o.seconds, o.sha256, check)
}

fn describe(item: &Item) -> Value {
    json!({ "op": item.kind(), "source": item.source(), "target": item.target() })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("");
    let positional = if command == "queue" { 1 } else { 2 };
    if !matches!(command, "get" | "relay" | "put" | "queue") || args.len() < 1 + positional {
        stop_with(USAGE);
    }
    let flags = read_flags(&args[1 + positional..]);
    let finished = Arc::new(AtomicBool::new(false));
    let active: Active = Arc::new(Mutex::new(HashMap::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let destinations = |target: &str| destination(target, &flags);

    if command == "queue" {
        let path = PathBuf::from(&args[1]);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| stop_with(&format!("cannot read {}: {}", path.display(), e)));
        let items = load_queue(&text).unwrap_or_else(|e| stop_with(&format!("{}: {}", path.display(), e)));
        let mut state = path.as_os_str().to_owned();
        state.push(".state");
        let state = PathBuf::from(state);
        if flags.restart {
            let _ = std::fs::remove_file(&state);
        }
        let reporter = start_reporter(flags.json, active.clone(), finished.clone());
        let (json_mode, active2) = (flags.json, active.clone());
        let on_event = move |e: Event| match e {
            Event::Started { id, item, progress } => {
                active2.lock().unwrap().insert(id, progress);
                if json_mode {
                    print_json(json!({ "event": "started", "id": id, "transfer": describe(&item) }));
                }
            }
            Event::Skipped { id, item } => {
                if json_mode {
                    print_json(json!({ "event": "skipped", "id": id, "transfer": describe(&item) }));
                } else {
                    println!("skipped, already done: {} -> {}", item.source(), item.target());
                }
            }
            Event::Done { id, item, outcome } => {
                active2.lock().unwrap().remove(&id);
                if json_mode {
                    print_json(json!({ "event": "done", "id": id, "transfer": describe(&item), "result": outcome.to_json() }));
                } else {
                    println!("{}", done_text(&item, &outcome));
                }
            }
            Event::Failed { id, item, error } => {
                active2.lock().unwrap().remove(&id);
                if json_mode {
                    print_json(json!({ "event": "failed", "id": id, "transfer": describe(&item), "error": error }));
                } else {
                    eprintln!("failed: {} -> {}: {}", item.source(), item.target(), error);
                }
            }
        };
        let totals = run_queue(&items, flags.jobs.unwrap_or(2), Some(&state), &destinations, stop, &on_event);
        finished.store(true, Ordering::Relaxed);
        let _ = reporter.join();
        if flags.json {
            print_json(json!({ "event": "summary", "done": totals.done, "failed": totals.failed, "skipped": totals.skipped }));
        } else {
            println!("{} done, {} failed, {} skipped", totals.done, totals.failed, totals.skipped);
        }
        std::process::exit(if totals.failed > 0 { 1 } else { 0 });
    }

    let item = match command {
        "get" => Item::Get { url: args[1].clone(), file: PathBuf::from(&args[2]), sha256: flags.sha256.clone() },
        "relay" => Item::Relay { url: args[1].clone(), target: args[2].clone(), work: flags.work.clone(), keep_local: flags.keep_local, sha256: flags.sha256.clone() },
        _ => Item::Put { file: PathBuf::from(&args[1]), target: args[2].clone(), sha256: flags.sha256.clone() },
    };
    let progress = Arc::new(Progress::default());
    active.lock().unwrap().insert(0, progress.clone());
    if flags.json {
        print_json(json!({ "event": "started", "id": 0, "transfer": describe(&item) }));
    }
    let reporter = start_reporter(flags.json, active, finished.clone());
    let result = run_item(&item, &destinations, progress, stop);
    finished.store(true, Ordering::Relaxed);
    let _ = reporter.join();
    match result {
        Ok(outcome) => {
            if flags.json {
                print_json(json!({ "event": "done", "id": 0, "transfer": describe(&item), "result": outcome.to_json() }));
            } else {
                println!("{}", done_text(&item, &outcome));
            }
        }
        Err(error) => {
            if flags.json {
                print_json(json!({ "event": "failed", "id": 0, "transfer": describe(&item), "error": error }));
            } else {
                eprintln!("failed: {}", error);
            }
            std::process::exit(1);
        }
    }
}
