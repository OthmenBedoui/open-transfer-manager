//! Segmented multi-connection HTTP(S) downloads with resume.
//!
//! A download starts with one connection. Once the size is known and the server accepts
//! byte ranges, more connections are opened by splitting the largest remaining part in half.
//! All parts are written into one pre-allocated `.otmpart` file, renamed when complete.

use crate::manager::{DlRef, Manager, lock};
use crate::model::*;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CHUNK: usize = 64 * 1024;
const MIN_SPLIT: u64 = 256 * 1024;
const RETRIES: u32 = 10;
static FINALIZE_LOCK: Mutex<()> = Mutex::new(());

type Resp = ureq::http::Response<ureq::Body>;

pub fn friendly_error(e: &ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(c) => format!(
            "Server answered {c}{}",
            match c {
                401 | 403 => " (access denied — the link may have expired)",
                404 => " (file not found)",
                410 => " (link expired)",
                429 => " (too many requests — try fewer connections)",
                500..=599 => " (server error)",
                _ => "",
            }
        ),
        ureq::Error::Timeout(_) => "Connection timed out".into(),
        ureq::Error::HostNotFound => "Server not found (check the link or your connection)".into(),
        ureq::Error::ConnectionFailed => "Could not connect to the server".into(),
        ureq::Error::Io(io) => format!("Network error: {io}"),
        other => other.to_string(),
    }
}

/// GET with optional Range. Returns the response or a readable error.
pub fn open(
    mgr: &Manager,
    url: &str,
    headers: &[(String, String)],
    range: Option<(u64, Option<u64>)>,
) -> Result<Resp, String> {
    let mut req = mgr
        .agent()
        .get(url)
        .header("Accept", "*/*")
        .header("Accept-Encoding", "identity");
    for (k, v) in headers {
        if !v.is_empty() {
            req = req.header(k.as_str(), v.as_str());
        }
    }
    if let Some((a, b)) = range {
        let r = match b {
            Some(e) => format!("bytes={a}-{e}"),
            None => format!("bytes={a}-"),
        };
        req = req.header("Range", r);
    }
    req.call().map_err(|e| friendly_error(&e))
}

/// (total size, byte ranges supported)
pub fn parse_total(resp: &Resp) -> (Option<u64>, bool) {
    let h = resp.headers();
    if resp.status().as_u16() == 206 {
        if let Some(cr) = h.get("content-range").and_then(|v| v.to_str().ok())
            && let Some(total) = cr
                .rsplit('/')
                .next()
                .and_then(|t| t.trim().parse::<u64>().ok())
        {
            return (Some(total), true);
        }
        return (None, false);
    }
    let len = h
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    (len, false)
}

pub fn name_from_response(resp: &Resp, url: &str) -> String {
    use ureq::ResponseExt;
    if let Some(cd) = resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
    {
        // filename*=UTF-8''name  (RFC 5987) wins over filename="name"
        if let Some(i) = cd.to_lowercase().find("filename*=") {
            let v = &cd[i + 10..];
            let v = v.split(';').next().unwrap_or("").trim().trim_matches('"');
            let v = v.rsplit("''").next().unwrap_or(v);
            let n = safe_name(&percent_decode(v));
            if n != "download" {
                return n;
            }
        }
        if let Some(i) = cd.to_lowercase().find("filename=") {
            let v = &cd[i + 9..];
            let v = v.split(';').next().unwrap_or("").trim().trim_matches('"');
            let n = safe_name(v);
            if n != "download" {
                return n;
            }
        }
    }
    let final_url = resp.get_uri().to_string();
    let n = name_from_url(&final_url);
    if n == "download" {
        name_from_url(url)
    } else {
        n
    }
}

/// Info for the "new download" dialog: (final url, size, resumable, name).
pub fn probe(
    mgr: &Manager,
    url: &str,
    headers: &[(String, String)],
) -> Result<(Option<u64>, bool, String), String> {
    let resp = open(mgr, url, headers, Some((0, None)))?;
    let (size, resumable) = parse_total(&resp);
    let name = name_from_response(&resp, url);
    Ok((size, resumable, name))
}

fn unique_path(path: std::path::PathBuf) -> std::path::PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap()
}

/// Runs on the download's own thread until done, stopped or failed.
pub fn run(dl: &DlRef, mgr: &Manager) {
    let result = run_inner(dl, mgr);
    let mut d = lock(dl);
    let stop = d.rt.stop.load(Ordering::SeqCst);
    match result {
        Err(e) => {
            d.status = if stop { Status::Stopped } else { Status::Error };
            if !stop {
                d.error = e;
            }
        }
        Ok(()) => {
            if stop {
                d.status = Status::Stopped;
            } else if d.segments.iter().all(|s| s.done())
                || (d.size.is_none() && !d.segments.iter().any(|s| s.busy))
            {
                if d.size.is_none() {
                    d.size = Some(d.downloaded());
                }
                // Serialize name selection + rename for simultaneous homonymous files.
                let _finalize = lock(&FINALIZE_LOCK);
                let target = unique_path(d.path());
                match std::fs::rename(d.part_path(), &target) {
                    Ok(()) => {
                        d.file_name = target
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        d.status = Status::Complete;
                        d.error.clear();
                    }
                    Err(e) => {
                        d.status = Status::Error;
                        d.error = format!("Could not save the file: {e}");
                    }
                }
            } else {
                d.status = Status::Error;
                if d.error.is_empty() {
                    d.error = "Connection failed".into();
                }
            }
        }
    }
}

fn run_inner(dl: &DlRef, mgr: &Manager) -> Result<(), String> {
    let (url, headers, stop) = {
        let d = lock(dl);
        (d.url.clone(), d.headers.clone(), d.rt.stop.clone())
    };
    let mut first: Option<Resp> = None;
    {
        let need_fresh = {
            let d = lock(dl);
            (!d.segments.is_empty() && !d.part_path().exists())
                || d.segments.is_empty()
                || (d.size.is_none() && d.resumable != Some(true))
        };
        if need_fresh {
            let resp = open(mgr, &url, &headers, Some((0, None)))?;
            let (total, resumable) = parse_total(&resp);
            let mut d = lock(dl);
            d.size = total;
            d.resumable = Some(resumable);
            if d.file_name.is_empty() || d.auto_name {
                let real = name_from_response(&resp, &url);
                if !real.is_empty() && real != d.file_name {
                    let _ = std::fs::remove_file(d.part_path());
                    d.file_name = real.clone();
                    if d.category == GENERAL {
                        drop(d);
                        let cat = mgr.category_for(&real);
                        let dir = mgr.folder_for(&cat);
                        let mut d2 = lock(dl);
                        d2.category = cat;
                        d2.save_dir = dir;
                        d = d2;
                    }
                }
                d.auto_name = false;
            }
            d.segments = vec![Segment::new(0, total.map(|t| t.saturating_sub(1)))];
            first = Some(resp);
        } else if lock(dl).resumable != Some(true) {
            let mut d = lock(dl);
            let size = d.size;
            d.segments = vec![Segment::new(0, size.map(|t| t.saturating_sub(1)))];
        }
    }
    let (save_dir, part, size, resumable) = {
        let d = lock(dl);
        (
            d.save_dir.clone(),
            d.part_path(),
            d.size,
            d.resumable == Some(true),
        )
    };
    std::fs::create_dir_all(&save_dir)
        .map_err(|e| format!("Cannot create folder {}: {e}", save_dir.display()))?;
    let reuse = part.exists() && first.is_none() && resumable;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(!reuse)
        .open(&part)
        .map_err(|e| format!("Cannot write {}: {e}", part.display()))?;
    if let Some(s) = size {
        file.set_len(s)
            .map_err(|e| format!("Not enough disk space? {e}"))?;
    }
    lock(dl).status = Status::Downloading;
    transfer(
        dl,
        mgr,
        Arc::new(Mutex::new(file)),
        first,
        &url,
        &headers,
        &stop,
        resumable,
    )
}

fn take_segment(dl: &DlRef) -> Option<usize> {
    let mut d = lock(dl);
    if let Some(i) = d.segments.iter().position(|s| !s.busy && !s.done()) {
        d.segments[i].busy = true;
        return Some(i);
    }
    if d.resumable != Some(true) || d.size.is_none() {
        return None;
    }
    let (i, rem) = d
        .segments
        .iter()
        .enumerate()
        .filter(|(_, s)| s.busy && !s.done())
        .map(|(i, s)| (i, s.remaining()))
        .max_by_key(|(_, r)| *r)?;
    if rem < 2 * MIN_SPLIT {
        return None;
    }
    let big = d.segments[i];
    let mid = big.pos + rem / 2;
    d.segments[i].end = Some(mid - 1);
    let mut new = Segment::new(mid, big.end);
    new.busy = true;
    d.segments.push(new);
    Some(d.segments.len() - 1)
}

#[allow(clippy::too_many_arguments)]
fn transfer(
    dl: &DlRef,
    mgr: &Manager,
    file: Arc<Mutex<File>>,
    first: Option<Resp>,
    url: &str,
    headers: &[(String, String)],
    stop: &Arc<AtomicBool>,
    resumable: bool,
) -> Result<(), String> {
    let mut workers: Vec<std::thread::JoinHandle<Result<(), String>>> = Vec::new();
    let last_error: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let abort = Arc::new(AtomicBool::new(false));
    let mut fatal_error = None;

    let launch = |idx: usize,
                  resp: Option<Resp>,
                  workers: &mut Vec<std::thread::JoinHandle<Result<(), String>>>| {
        let dl = dl.clone();
        let mgr = mgr.clone();
        let file = file.clone();
        let stop = stop.clone();
        let url = url.to_string();
        let headers = headers.to_vec();
        let last_error = last_error.clone();
        let abort = abort.clone();
        workers.push(std::thread::spawn(move || {
            worker(
                &dl,
                &mgr,
                &file,
                idx,
                resp,
                &url,
                &headers,
                &stop,
                &abort,
                resumable,
                &last_error,
            )
        }));
    };

    if let Some(resp) = first {
        lock(dl).segments[0].busy = true;
        launch(0, Some(resp), &mut workers);
    }
    let max_conn = if resumable && lock(dl).size.is_some() {
        mgr.max_connections_for(url)
    } else {
        1
    };
    let mut last_flush = Instant::now();
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let mut i = 0;
        while i < workers.len() {
            if workers[i].is_finished() {
                match workers.swap_remove(i).join() {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        fatal_error.get_or_insert(e);
                    }
                    Err(_) => {
                        fatal_error.get_or_insert("Internal transfer worker error".into());
                    }
                }
            } else {
                i += 1;
            }
        }
        if fatal_error.is_some() {
            abort.store(true, Ordering::SeqCst);
            break;
        }
        let alive = workers.len();
        let all_done = lock(dl).segments.iter().all(|s| s.done());
        if all_done && alive == 0 {
            break;
        }
        let mut alive_now = alive;
        while alive_now < max_conn {
            match take_segment(dl) {
                Some(idx) => {
                    launch(idx, None, &mut workers);
                    alive_now += 1;
                }
                None => break,
            }
        }
        lock(dl).rt.connections = alive_now;
        if alive_now == 0 {
            let d = lock(dl);
            if d.size.is_some() && d.segments.iter().any(|s| !s.done()) {
                let e = lock(&last_error).clone();
                return Err(if e.is_empty() {
                    "Connection failed".into()
                } else {
                    e
                });
            }
            break;
        }
        if last_flush.elapsed() > Duration::from_secs(5) {
            let _ = lock(&file).flush();
            mgr.save();
            last_flush = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    for w in workers {
        match w.join() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                fatal_error.get_or_insert(e);
            }
            Err(_) => {
                fatal_error.get_or_insert("Internal transfer worker error".into());
            }
        }
    }
    let _ = lock(&file).flush();
    match fatal_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
fn worker(
    dl: &DlRef,
    mgr: &Manager,
    file: &Mutex<File>,
    idx: usize,
    mut resp: Option<Resp>,
    url: &str,
    headers: &[(String, String)],
    stop: &AtomicBool,
    abort: &AtomicBool,
    resumable: bool,
    last_error: &Mutex<String>,
) -> Result<(), String> {
    let mut failures = 0u32;
    let mut terminal_error = None;
    let mut buf = vec![0u8; CHUNK];
    loop {
        if stop.load(Ordering::SeqCst)
            || abort.load(Ordering::SeqCst)
            || lock(dl).segments[idx].done()
        {
            break;
        }
        let res: Result<(), String> = (|| {
            let r = match resp.take() {
                Some(r) => r,
                None => {
                    let (pos, end) = {
                        let d = lock(dl);
                        (d.segments[idx].pos, d.segments[idx].end)
                    };
                    let r = open(
                        mgr,
                        url,
                        headers,
                        if resumable { Some((pos, end)) } else { None },
                    )?;
                    if resumable && r.status().as_u16() != 206 {
                        return Err("The server stopped supporting resume".into());
                    }
                    r
                }
            };
            let mut reader = r.into_body().into_reader();
            loop {
                if stop.load(Ordering::SeqCst) || abort.load(Ordering::SeqCst) {
                    return Ok(());
                }
                let want = {
                    let d = lock(dl);
                    let s = d.segments[idx];
                    match s.end {
                        Some(e) => (e + 1).saturating_sub(s.pos).min(CHUNK as u64) as usize,
                        None => CHUNK,
                    }
                };
                if want == 0 {
                    return Ok(());
                }
                let n = reader
                    .read(&mut buf[..want])
                    .map_err(|e| format!("Network error: {e}"))?;
                if n == 0 {
                    let mut d = lock(dl);
                    let s = &mut d.segments[idx];
                    if s.end.is_none() {
                        // unknown size: the end of the stream is the end of the file
                        if s.pos == s.start {
                            s.end = Some(s.start);
                            s.pos = s.start + 1; // empty file: mark done
                        } else {
                            s.end = Some(s.pos - 1);
                        }
                        return Ok(());
                    }
                    if !s.done() {
                        return Err("Connection closed by the server".into());
                    }
                    return Ok(());
                }
                mgr.limiter().consume(n, stop);
                // Keep splitting/saving from observing a position not yet written.
                // Lock order is file -> download; no other path takes both in reverse.
                let mut f = lock(file);
                let mut d = lock(dl);
                let s = &mut d.segments[idx];
                let room = match s.end {
                    Some(e) => (e + 1).saturating_sub(s.pos) as usize,
                    None => n,
                };
                let written = n.min(room);
                if written > 0 {
                    f.seek(SeekFrom::Start(s.pos))
                        .map_err(|e| format!("Disk error: {e}"))?;
                    f.write_all(&buf[..written])
                        .map_err(|e| format!("Disk error: {e}"))?;
                    s.pos += written as u64;
                }
                drop(d);
                drop(f);
                failures = 0;
            }
        })();
        match res {
            Ok(()) => {
                if stop.load(Ordering::SeqCst) || lock(dl).segments[idx].done() {
                    break;
                }
                if lock(dl).segments[idx].end.is_some() && !resumable {
                    break;
                }
            }
            Err(e) => {
                *lock(last_error) = e.clone();
                lock(dl).error = e.clone();
                failures += 1;
                if failures > RETRIES || !resumable {
                    terminal_error = Some(e);
                    break;
                }
                let delay = (0.5 * 2f64.powi(failures as i32 - 1)).min(15.0);
                let end = Instant::now() + Duration::from_secs_f64(delay);
                while Instant::now() < end
                    && !stop.load(Ordering::SeqCst)
                    && !abort.load(Ordering::SeqCst)
                {
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
    lock(dl).segments[idx].busy = false;
    match terminal_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;

    /// Minimal HTTP/1.1 server with optional byte ranges, enough for the engine tests.
    fn serve(
        data: Arc<Vec<u8>>,
        ranges: bool,
        slow: Arc<AtomicBool>,
        hits: Arc<AtomicUsize>,
    ) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in l.incoming().flatten() {
                let data = data.clone();
                let slow = slow.clone();
                hits.fetch_add(1, Ordering::SeqCst);
                std::thread::spawn(move || {
                    let mut s = stream;
                    let mut reader = BufReader::new(s.try_clone().unwrap());
                    let mut range: Option<(u64, Option<u64>)> = None;
                    let mut line = String::new();
                    let mut first = true;
                    let mut path = String::new();
                    loop {
                        line.clear();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        if first {
                            path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                            first = false;
                        }
                        let l = line.trim();
                        if l.is_empty() {
                            break;
                        }
                        if let Some(v) = l.to_lowercase().strip_prefix("range: bytes=") {
                            let mut it = v.split('-');
                            let a = it.next().unwrap().parse().unwrap();
                            let b = it.next().and_then(|b| b.parse().ok());
                            range = Some((a, b));
                        }
                    }
                    if path.starts_with("/missing") {
                        let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        return;
                    }
                    let len = data.len() as u64;
                    let (start, end, head) = match (range, ranges) {
                        (Some((a, b)), true) => {
                            let e = b.unwrap_or(len - 1).min(len - 1);
                            (
                                a,
                                e,
                                format!(
                                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {a}-{e}/{len}\r\n"
                                ),
                            )
                        }
                        _ => (0, len - 1, "HTTP/1.1 200 OK\r\n".to_string()),
                    };
                    let mut head = head;
                    if path.starts_with("/named") {
                        head.push_str(
                            "Content-Disposition: attachment; filename=\"report final.pdf\"\r\n",
                        );
                    }
                    head.push_str(&format!(
                        "Content-Length: {}\r\nConnection: close\r\n\r\n",
                        end - start + 1
                    ));
                    if s.write_all(head.as_bytes()).is_err() {
                        return;
                    }
                    let mut pos = start;
                    while pos <= end {
                        if path.starts_with("/short") && pos >= len / 2 {
                            return;
                        }
                        let n = ((end - pos + 1) as usize).min(65536);
                        if s.write_all(&data[pos as usize..pos as usize + n]).is_err() {
                            return;
                        }
                        pos += n as u64;
                        if slow.load(Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_millis(150));
                        }
                    }
                });
            }
        });
        format!("http://{addr}")
    }

    fn data() -> Arc<Vec<u8>> {
        Arc::new(
            (0..6 * 1024 * 1024 + 123)
                .map(|i: usize| (i.wrapping_mul(2654435761) >> 13) as u8)
                .collect(),
        )
    }

    fn wait(dl: &DlRef, secs: u64) {
        let end = Instant::now() + Duration::from_secs(secs);
        std::thread::sleep(Duration::from_millis(50));
        while lock(dl).rt.running && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    fn tmp() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("otm-test-{}-{}", std::process::id(), now()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn multi_connection_pause_resume_and_errors() {
        let data = data();
        let slow = Arc::new(AtomicBool::new(true));
        let base = serve(
            data.clone(),
            true,
            slow.clone(),
            Arc::new(AtomicUsize::new(0)),
        );
        let mgr = Manager::new(false);
        let dir = tmp();
        // never touch the user's real Downloads folder
        mgr.update_settings(|s| {
            s.base_folder = dir.clone();
            s.categories = default_categories(&dir);
        });

        // multi-connection + pause/resume
        let dl = mgr.add_http(
            &format!("{base}/files/a.zip"),
            "a.zip",
            &dir,
            "Archives",
            true,
            false,
            vec![],
        );
        let mut peak = 0;
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_millis(1200) {
            peak = peak.max(lock(&dl).segments.iter().filter(|s| s.busy).count());
            std::thread::sleep(Duration::from_millis(20));
        }
        mgr.stop(&dl);
        wait(&dl, 30);
        let mid = lock(&dl).downloaded();
        assert_eq!(lock(&dl).status, Status::Stopped);
        assert!(mid > 0 && mid < data.len() as u64, "mid={mid}");
        assert!(peak > 1, "peak connections {peak}");
        slow.store(false, Ordering::SeqCst);
        mgr.start(&dl);
        wait(&dl, 60);
        let d = lock(&dl);
        assert_eq!(d.status, Status::Complete, "{}", d.error);
        assert_eq!(std::fs::read(d.path()).unwrap(), *data);
        assert!(!d.part_path().exists());
        drop(d);

        // name from Content-Disposition replaces the guess
        let r = mgr.add_smart(
            &format!("{base}/named/download?id=1"),
            "",
            Some(false),
            true,
            vec![],
            None,
        );
        wait(&r, 30);
        assert_eq!(lock(&r).file_name, "report final.pdf");
        assert_eq!(lock(&r).category, "Documents");
        assert!(dir.join("Documents").join("report final.pdf").exists());

        // HTTP error
        let e = mgr.add_http(
            &format!("{base}/missing/x.zip"),
            "x.zip",
            &dir,
            "Archives",
            true,
            false,
            vec![],
        );
        wait(&e, 30);
        let e = lock(&e);
        assert_eq!(e.status, Status::Error);
        assert!(e.error.contains("404"), "{}", e.error);
    }

    #[test]
    fn server_without_ranges_uses_one_connection() {
        let data = data();
        let base = serve(
            data.clone(),
            false,
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicUsize::new(0)),
        );
        let mgr = Manager::new(false);
        let dir = tmp();
        let dl = mgr.add_http(
            &format!("{base}/norange/doc.pdf"),
            "doc.pdf",
            &dir,
            "Documents",
            true,
            false,
            vec![],
        );
        wait(&dl, 60);
        let d = lock(&dl);
        assert_eq!(d.status, Status::Complete, "{}", d.error);
        assert_eq!(d.resumable, Some(false));
        assert_eq!(d.segments.len(), 1);
        assert_eq!(std::fs::read(d.path()).unwrap(), *data);
    }

    #[test]
    fn interrupted_nonresumable_download_fails_instead_of_restarting_forever() {
        let base = serve(
            data(),
            false,
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicUsize::new(0)),
        );
        let mgr = Manager::new(false);
        let dl = mgr.add_http(
            &format!("{base}/short/a.zip"),
            "a.zip",
            &tmp(),
            "Archives",
            true,
            false,
            vec![],
        );
        wait(&dl, 5);
        assert!(
            !lock(&dl).running(),
            "worker should stop after non-resumable failure"
        );
        assert_eq!(lock(&dl).status, Status::Error);
    }

    #[test]
    fn parallel_same_names_have_independent_partial_files() {
        let contents = data();
        let base = serve(
            contents.clone(),
            true,
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicUsize::new(0)),
        );
        let mgr = Manager::new(false);
        let dir = tmp();
        let a = mgr.add_http(
            &format!("{base}/a.zip"),
            "same.zip",
            &dir,
            "Archives",
            false,
            false,
            vec![],
        );
        let b = mgr.add_http(
            &format!("{base}/b.zip"),
            "same.zip",
            &dir,
            "Archives",
            false,
            false,
            vec![],
        );
        assert_ne!(lock(&a).part_path(), lock(&b).part_path());
        mgr.start(&a);
        mgr.start(&b);
        wait(&a, 15);
        wait(&b, 15);
        assert_eq!(lock(&a).status, Status::Complete);
        assert_eq!(lock(&b).status, Status::Complete);
        assert_ne!(lock(&a).path(), lock(&b).path());
        assert_eq!(std::fs::read(lock(&a).path()).unwrap(), *contents);
        assert_eq!(std::fs::read(lock(&b).path()).unwrap(), *contents);
    }

    #[test]
    fn arabic_filename_is_saved_without_reordering_or_renaming() {
        let contents = data();
        let base = serve(
            contents.clone(),
            true,
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicUsize::new(0)),
        );
        let mgr = Manager::new(false);
        let dir = tmp();
        let name = "تقرير العمل 2026.pdf";
        let dl = mgr.add_http(
            &format!("{base}/files/report.pdf"),
            name,
            &dir,
            "Documents",
            true,
            false,
            vec![],
        );
        wait(&dl, 15);
        assert_eq!(lock(&dl).status, Status::Complete);
        assert_eq!(lock(&dl).file_name, name);
        assert_eq!(std::fs::read(dir.join(name)).unwrap(), *contents);
    }

    #[test]
    fn speed_limit_slows_down() {
        let data = data();
        let base = serve(
            data.clone(),
            true,
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicUsize::new(0)),
        );
        let mgr = Manager::new(false);
        mgr.set_limiter(true, Some(2048));
        let t0 = Instant::now();
        let dl = mgr.add_http(
            &format!("{base}/files/l.zip"),
            "l.zip",
            &tmp(),
            "Archives",
            true,
            false,
            vec![],
        );
        wait(&dl, 60);
        assert_eq!(lock(&dl).status, Status::Complete);
        assert!(t0.elapsed() > Duration::from_secs(2), "{:?}", t0.elapsed());
    }
}
