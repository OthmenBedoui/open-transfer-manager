//! Open Transfer Manager — a light, open-source download manager.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod api;
mod browsers;
mod connectors;
mod http;
mod manager;
mod media;
mod model;
mod music;
mod native_host;
mod paths;
mod rclone;
mod secrets;
mod ui;
mod uploads;

use manager::Manager;
use serde_json::json;
use std::time::Duration;

const HELP: &str = "Open Transfer Manager

USAGE
  otm                         open the app (or bring the running one to the front)
  otm URL [URL...]            add links to the running app (starts it if needed)
  otm add URL [--audio|--video] [--quality 1080|720|...] [--mp3 320|192|128] [--later]
  otm formats URL             list the formats yt-dlp offers for a page
  otm list                    show downloads of the running app
  otm headless                run without a window (server / NAS)
  otm doctor                  diagnostics for bug reports
  otm register-browsers [--force] | unregister-browsers
  otm update-ytdlp            install or update the video engine
  otm update-rclone           install or update the upload engine
  otm connectors              list your upload destinations (set them up in the app)
  otm send FILE... --to NAME[,NAME...] [--link]
                              upload files to one or more destinations
  otm --version";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("-V" | "--version") => {
            println!("{} {}", paths::APP_NAME, env!("CARGO_PKG_VERSION"));
            0
        }
        Some("-h" | "--help" | "help") => {
            println!("{HELP}");
            0
        }
        Some("native-host") => native_host::main(),
        // Chrome/Firefox may also start the binary directly with the extension origin / manifest path
        Some(a) if a.starts_with("chrome-extension://") || a.ends_with(".json") => {
            native_host::main()
        }
        Some("add") => cmd_add(&args[1..]),
        Some("formats") => cmd_formats(&args[1..]),
        Some("list") => cmd_list(),
        Some("headless") => cmd_headless(),
        Some("doctor") => {
            print!("{}", doctor());
            0
        }
        Some("register-browsers") => {
            let registration = if args.iter().any(|a| a == "--machine") {
                browsers::register_machine()
            } else {
                browsers::register(args.iter().any(|a| a == "--force"))
            };
            match registration {
                Ok(done) if done.is_empty() => {
                    println!("No supported browser found. Use --force to write the files anyway.");
                    0
                }
                Ok(done) => {
                    for (b, p) in done {
                        println!("  registered for {b:<18} {}", p.display());
                    }
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            }
        }
        Some("unregister-browsers") => {
            if args.iter().any(|a| a == "--machine") {
                if let Err(e) = browsers::unregister_machine() {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
                return;
            }
            for b in browsers::unregister() {
                println!("  removed for {b}");
            }
            0
        }
        Some("update-rclone") => match rclone::install_or_update() {
            Ok(t) => {
                println!("{t}");
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
        Some("connectors") => cmd_connectors(),
        Some("send") => cmd_send(&args[1..]),
        Some("update-ytdlp") => match media::install_or_update() {
            Ok(t) => {
                println!("{t}");
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
        _ => gui(&args),
    };
    std::process::exit(code);
}

fn gui(args: &[String]) -> i32 {
    let urls: Vec<String> = args
        .iter()
        .filter(|a| a.starts_with("http://") || a.starts_with("https://"))
        .cloned()
        .collect();
    let from_browser = args.iter().any(|a| a == "--from-browser");
    let screenshot = args
        .iter()
        .position(|a| a == "--screenshot")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let setup_browsers = args
        .iter()
        .any(|a| a == "--setup-browsers" || a == "--install-chrome-extension");
    if screenshot.is_none()
        && args.iter().any(|a| a == "--install-chrome-extension")
        && let Err(e) = browsers::open_chrome_setup()
    {
        eprintln!("Chrome setup: {e}");
    }
    if screenshot.is_none() && api::is_running() {
        for u in &urls {
            let _ = api::request(
                "POST",
                "/api/add",
                Some(json!({"url": u, "confirm": true})),
                Duration::from_secs(10),
            );
        }
        if !from_browser {
            let _ = api::request(
                "POST",
                "/api/show",
                Some(json!({"browser_settings": setup_browsers})),
                Duration::from_secs(5),
            );
        }
        return 0;
    }
    if screenshot.is_none() {
        // Registry repair is a one-off background task, never a prerequisite
        // for showing the desktop window.
        std::thread::spawn(|| {
            if let Err(e) = browsers::ensure_registered() {
                eprintln!("Browser bridge: {e}");
            }
        });
    }
    ui::run(urls, from_browser, screenshot, setup_browsers)
}

fn cmd_add(args: &[String]) -> i32 {
    let Some(url) = args
        .iter()
        .find(|a| !a.starts_with("--") && a.contains("://"))
    else {
        eprintln!("usage: otm add URL [--audio|--video] [--quality Q] [--mp3 KBPS] [--later]");
        return 2;
    };
    let val = |k: &str| {
        args.iter()
            .position(|a| a == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let mut body = json!({"url": url, "start": !args.iter().any(|a| a == "--later")});
    if args.iter().any(|a| a == "--file") {
        body["media"] = json!(false);
    }
    let mut opts = None;
    if args.iter().any(|a| a == "--audio") || val("--mp3").is_some() {
        opts = Some(
            json!({"quality": "audio", "audio_format": "mp3", "audio_kbps": val("--mp3").and_then(|v| v.parse::<u32>().ok()).unwrap_or(192), "playlist": false}),
        );
    } else if let Some(q) = val("--quality") {
        opts = Some(
            json!({"quality": q, "audio_format": "mp3", "audio_kbps": 192, "playlist": false}),
        );
    } else if args.iter().any(|a| a == "--video") {
        body["media"] = json!(true);
    }
    if let Some(o) = opts {
        body["opts"] = o;
    }
    if !native_host::launch_app() {
        eprintln!("Could not start {}", paths::APP_NAME);
        return 1;
    }
    match api::request("POST", "/api/add", Some(body), Duration::from_secs(15)) {
        Ok(v) => {
            println!(
                "Added #{} {}",
                v["download"]["id"],
                v["download"]["name"].as_str().unwrap_or("")
            );
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn cmd_formats(args: &[String]) -> i32 {
    let Some(url) = args.first() else {
        eprintln!("usage: otm formats URL");
        return 2;
    };
    let mgr = Manager::new(false);
    match media::probe(&mgr, url, &[]) {
        Ok(info) => {
            println!(
                "{}  ({}{})",
                info.title,
                info.uploader,
                info.duration
                    .map(|d| format!(", {}", model::fmt_duration(d)))
                    .unwrap_or_default()
            );
            for c in info.choices {
                println!(
                    "  {:<15} {:<5} {:<16} {:<34} {}",
                    c.id,
                    c.ext,
                    c.label,
                    c.detail,
                    model::fmt_size(c.size)
                );
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn cmd_connectors() -> i32 {
    let up = uploads::Uploads::new(true);
    let all = up.connectors();
    if all.is_empty() {
        println!("No destinations yet. Add one in the app: Connectors.");
    }
    for c in all {
        println!(
            "  {:<24} {:<14} {}",
            c.display_name(),
            c.kind.label(),
            c.address()
        );
    }
    0
}

fn cmd_send(args: &[String]) -> i32 {
    let usage = "usage: otm send FILE... --to NAME[,NAME...] [--link]";
    let to = args
        .iter()
        .position(|a| a == "--to")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let files: Vec<std::path::PathBuf> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--to"))
        .map(|(_, a)| std::path::PathBuf::from(a))
        .collect();
    let (Some(to), false) = (to, files.is_empty()) else {
        eprintln!("{usage}");
        return 2;
    };
    for f in &files {
        if !f.exists() {
            eprintln!("error: not found: {}", f.display());
            return 1;
        }
    }
    let up = uploads::Uploads::new(true);
    let mut dests = Vec::new();
    for name in to.split(',').filter(|n| !n.trim().is_empty()) {
        match up.find_connector(name) {
            Some(c) => dests.push(c.id),
            None => {
                eprintln!(
                    "error: no destination named \"{}\" (see: otm connectors)",
                    name.trim()
                );
                return 1;
            }
        }
    }
    if !uploads::engine_ready() {
        eprintln!("The upload engine is missing: run  otm update-rclone");
        return 1;
    }
    let files: Vec<_> = files
        .iter()
        .map(|f| std::fs::canonicalize(f).unwrap_or_else(|_| f.clone()))
        .collect();
    up.send(&files, &dests, args.iter().any(|a| a == "--link"));
    let mut shown = std::collections::HashMap::new();
    loop {
        up.tick();
        for j in up.jobs() {
            let line = match j.status {
                uploads::JobStatus::Running => format!(
                    "{:>5.1}%  {}",
                    j.progress() * 100.0,
                    model::fmt_rate(Some(j.speed))
                ),
                uploads::JobStatus::Done => match &j.link {
                    Some(l) => format!("done   {l}"),
                    None => "done".into(),
                },
                uploads::JobStatus::Failed => format!("failed: {}", j.error),
                uploads::JobStatus::Stopped => "stopped".into(),
                uploads::JobStatus::Waiting => "waiting".into(),
            };
            if shown.get(&j.id) != Some(&line) {
                println!("{} → {}  {line}", j.name, j.dest_name);
                shown.insert(j.id, line);
            }
        }
        if up.active_count() == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(1000));
    }
    let failed = up
        .jobs()
        .iter()
        .filter(|j| j.status != uploads::JobStatus::Done)
        .count();
    if failed > 0 { 1 } else { 0 }
}

fn cmd_list() -> i32 {
    match api::request("GET", "/api/downloads", None, Duration::from_secs(5)) {
        Ok(v) => {
            for d in v["downloads"].as_array().cloned().unwrap_or_default() {
                println!(
                    "#{:<4} {:<14} {:>10}  {:>10}  {}",
                    d["id"],
                    d["status_text"].as_str().unwrap_or(""),
                    model::fmt_size(d["size"].as_u64()),
                    model::fmt_rate(d["rate"].as_f64()),
                    d["name"].as_str().unwrap_or("")
                );
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cmd_headless() -> i32 {
    if api::is_running() {
        eprintln!("{} is already running.", paths::APP_NAME);
        return 1;
    }
    let mgr = Manager::new(true);
    let srv = match api::ApiServer::start(mgr.clone()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    println!(
        "{} {} running headless on 127.0.0.1:{} (Ctrl+C to stop)",
        paths::APP_NAME,
        env!("CARGO_PKG_VERSION"),
        srv.port
    );
    loop {
        mgr.tick();
        for e in mgr.take_events() {
            if let manager::Event::Finished { name, .. } = e {
                println!("complete: {name}");
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

pub fn doctor() -> String {
    let mut s = String::new();
    let mut row = |k: &str, v: String| s.push_str(&format!("{k:<20} {v}\n"));
    row(paths::APP_NAME, env!("CARGO_PKG_VERSION").into());
    row(
        "OS",
        format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
    );
    row(
        "yt-dlp",
        media::ytdlp_version()
            .map(|v| format!("{v} ({})", media::ytdlp_path().unwrap().display()))
            .unwrap_or("not installed (otm update-ytdlp)".into()),
    );
    row(
        "ffmpeg",
        media::ffmpeg_path()
            .map(|p| p.display().to_string())
            .unwrap_or("not found (needed for MP3 and best quality)".into()),
    );
    row(
        "rclone",
        rclone::version()
            .map(|v| format!("{v} ({})", rclone::path().unwrap().display()))
            .unwrap_or("not installed (otm update-rclone)".into()),
    );
    row("Secrets stored in", secrets::backend().into());
    row("Data folder", paths::data_dir().display().to_string());
    row(
        "Downloads folder",
        paths::downloads_dir().display().to_string(),
    );
    row(
        "App running",
        if api::is_running() {
            "yes".into()
        } else {
            "no".into()
        },
    );
    s.push_str("Browsers:\n");
    let st = browsers::status();
    if st.is_empty() {
        s.push_str("  (none detected)\n");
    }
    for (b, installed, reg) in st {
        s.push_str(&format!(
            "  {b:<20} {}, {}\n",
            if installed { "installed" } else { "not found" },
            if reg {
                "bridge registered"
            } else {
                "bridge NOT registered"
            }
        ));
    }
    s
}
