//! `otm-lab get URL FILE`        downloads with this engine (used by the bench)
//! `otm-lab serve SCENARIO`          runs the misbehaving server
//! `otm-lab bench [--old EXE] [--repeat N] [--only NAME] [--work DIR]`  compares the engines
use otm_core::engine::{download, Options};
use otm_core::lab::{self, file_matches, Scenario};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const TIME_LIMIT: Duration = Duration::from_secs(180);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("get") if args.len() == 3 => get(&args[1], Path::new(&args[2])),
        Some("serve") if args.len() == 2 => serve(&args[1]),
        Some("bench") => bench(&args[1..]),
        _ => {
            eprintln!("usage: otm-lab get URL FILE | otm-lab serve SCENARIO | otm-lab bench [--old EXE] [--repeat N] [--only NAME] [--work DIR]");
            std::process::exit(2);
        }
    }
}

fn get(url: &str, dest: &Path) {
    match download(url, dest, &Options::default(), Arc::new(AtomicBool::new(false))) {
        Ok(r) => println!("LAB_RESULT ok peak={} doubled={} replaced={} throttled={} retries={} restarts={}", r.peak_connections, r.doubled_pieces, r.replaced_slow, r.throttled, r.retries, r.restarts),
        Err(e) => {
            println!("LAB_RESULT failed {}", e);
            std::process::exit(1);
        }
    }
}

fn serve(name: &str) {
    let Some(s) = Scenario::named(name) else {
        eprintln!("unknown scenario");
        std::process::exit(2);
    };
    let server = lab::start(s);
    println!("PORT={}", server.port);
    let _ = std::io::stdout().flush();
    // Runs until the bench ends it.
    let _ = std::io::stdin().read(&mut [0u8; 1]);
}

#[derive(Clone, Debug, Default)]
struct Run {
    finished: bool,
    correct: bool,
    seconds: f64,
    connections: u64,
    requests: u64,
    sent: u64,
    rejected: u64,
    peak_requests: u64,
    peak_disk: u64,
    note: String,
}

fn metrics(port: u16) -> Option<(u64, u64, u64, u64, u64, u64)> {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
    write!(s, "GET /__metrics HTTP/1.1\r\nHost: x\r\n\r\n").ok()?;
    let mut text = String::new();
    s.read_to_string(&mut text).ok()?;
    let body = text.split("\r\n\r\n").nth(1)?;
    let v: Vec<u64> = body.split_whitespace().filter_map(|x| x.parse().ok()).collect();
    Some((*v.first()?, *v.get(1)?, *v.get(2)?, *v.get(3)?, *v.get(4)?, *v.get(5)?))
}

fn disk_use(dir: &Path) -> u64 {
    let mut total = 0;
    if let Ok(list) = std::fs::read_dir(dir) {
        for e in list.flatten() {
            let p = e.path();
            total += if p.is_dir() { disk_use(&p) } else { e.metadata().map(|m| m.len()).unwrap_or(0) };
        }
    }
    total
}

fn find_result(dir: &Path) -> Option<PathBuf> {
    for sub in ["out.bin", "out/lab.bin"] {
        let p = dir.join(sub);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn run_once(scn: &Scenario, engine: &str, old_exe: Option<&str>, me: &Path, work: &Path) -> Run {
    let dir = work.join(format!("{}_{}", scn.name, engine));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("work folder");
    let mut server = Command::new(me).args(["serve", scn.name]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("server");
    let mut line = String::new();
    BufReader::new(server.stdout.as_mut().unwrap()).read_line(&mut line).expect("port");
    let port: u16 = line.trim().trim_start_matches("PORT=").parse().expect("port number");
    let url = format!("http://127.0.0.1:{}/file", port);
    let mut child = match engine {
        "otm" => Command::new(me).args(["get", &url]).arg(dir.join("out.bin")).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("new engine"),
        _ => Command::new(old_exe.expect("old engine"))
            .args(["e2e_tests::lab_old_engine", "--ignored", "--exact", "--nocapture"])
            .env("LAB_URL", &url)
            .env("LAB_DIR", &dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("old engine"),
    };
    let peak = Arc::new(AtomicU64::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let (p2, d2, dir2) = (peak.clone(), done.clone(), dir.clone());
    let sampler = std::thread::spawn(move || {
        while !d2.load(Ordering::Relaxed) {
            p2.fetch_max(disk_use(&dir2), Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    let began = Instant::now();
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if began.elapsed() > TIME_LIMIT => {
                let _ = child.kill();
                let _ = child.wait();
                timed_out = true;
                break;
            }
            _ => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let seconds = began.elapsed().as_secs_f64();
    let output = reader.join().unwrap_or_default();
    done.store(true, Ordering::Relaxed);
    let _ = sampler.join();
    let result_line = output.lines().find(|l| l.starts_with("LAB_RESULT")).unwrap_or("").to_string();
    let finished = !timed_out && result_line.contains("ok");
    let (connections, requests, sent, rejected, peak_requests, version) = metrics(port).unwrap_or_default();
    // Correct means one whole version of the file. Which one depends on timing.
    let _ = version;
    let correct = finished
        && find_result(&dir)
            .map(|p| file_matches(&p, scn.size, 0).unwrap_or(false) || file_matches(&p, scn.size, 1).unwrap_or(false))
            .unwrap_or(false);
    let _ = server.kill();
    let _ = server.wait();
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        finished,
        correct,
        seconds,
        connections,
        requests,
        sent,
        rejected,
        peak_requests,
        peak_disk: peak.load(Ordering::Relaxed),
        note: if timed_out { "time limit".into() } else { result_line.trim_start_matches("LAB_RESULT ").to_string() },
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

fn bench(args: &[String]) {
    let (mut old, mut repeat, mut only, mut work) = (None, 3usize, None, std::env::temp_dir().join("otm-bench"));
    let mut i = 0;
    while i < args.len() {
        let v = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--old" => old = Some(v),
            "--repeat" => repeat = v.parse().unwrap_or(3),
            "--only" => only = Some(v),
            "--work" => work = PathBuf::from(v),
            _ => {}
        }
        i += 2;
    }
    let me = std::env::current_exe().expect("own path");
    std::fs::create_dir_all(&work).expect("work folder");
    let engines: Vec<&str> = if old.is_some() { vec!["baseline", "otm"] } else { vec!["otm"] };
    println!("| Scenario | Engine | Correct | Time (s) | Connections | Requests | Sent (MB) | 429 received | Peak disk (MB) |");
    println!("|---|---|---|---|---|---|---|---|---|");
    for scn in Scenario::all() {
        if only.as_deref().map(|o| o != scn.name).unwrap_or(false) {
            continue;
        }
        eprintln!("== {} : {}", scn.name, scn.about);
        for engine in &engines {
            let mut runs = vec![];
            for _ in 0..repeat {
                let r = run_once(&scn, engine, old.as_deref(), &me, &work);
                eprintln!("   {} {:.1}s correct={} finished={} {}", engine, r.seconds, r.correct, r.finished, r.note);
                runs.push(r);
            }
            let correct = runs.iter().filter(|r| r.correct).count();
            let m = |f: &dyn Fn(&Run) -> f64| median(runs.iter().map(f).collect());
            println!(
                "| {} | {} | {}/{} | {:.1} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} |",
                scn.name,
                engine,
                correct,
                runs.len(),
                m(&|r| r.seconds),
                m(&|r| r.connections as f64),
                m(&|r| r.requests as f64),
                m(&|r| r.sent as f64 / (1 << 20) as f64),
                m(&|r| r.rejected as f64),
                m(&|r| r.peak_disk as f64 / (1 << 20) as f64),
            );
            let _ = std::io::stdout().flush();
            let _ = runs.iter().map(|r| r.peak_requests).max();
        }
    }
}
