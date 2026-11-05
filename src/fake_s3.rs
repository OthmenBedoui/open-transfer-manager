//! A stand-in for S3-compatible storage that misbehaves on demand. It checks what a real service
//! checks about the data (SHA-256, part ETags, part sizes), but it is not a real service.
use crate::s3::sha256_hex;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default)]
pub struct Faults {
    /// Speed limit of each part upload, bytes per second.
    pub part_bps: u64,
    /// Every Nth part upload is answered 500.
    pub error_every: u64,
    /// Every Nth part upload is answered 503 with Retry-After.
    pub slow_down_every: u64,
    /// Every Nth part upload loses its connection halfway through.
    pub cut_every: u64,
    /// The first completion answers 200 with an error in its body, as S3 can.
    pub complete_error_once: bool,
}

#[derive(Default)]
pub struct Store {
    pub objects: HashMap<String, Vec<u8>>,
    uploads: HashMap<String, (String, HashMap<u32, Vec<u8>>)>,
    next_upload: u64,
}

#[derive(Default)]
pub struct Counters {
    pub part_requests: AtomicU64,
    pub parts_accepted: AtomicU64,
    pub bytes_accepted: AtomicU64,
    pub created: AtomicU64,
    pub aborted: AtomicU64,
    pub completed: AtomicU64,
    pub unsigned_refused: AtomicU64,
    pub bad_data_refused: AtomicU64,
}

pub struct FakeS3 {
    pub port: u16,
    pub store: Arc<Mutex<Store>>,
    pub counters: Arc<Counters>,
    stop: Arc<AtomicBool>,
}

impl Drop for FakeS3 {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

struct Shared {
    faults: Faults,
    store: Arc<Mutex<Store>>,
    counters: Arc<Counters>,
    complete_failed: AtomicBool,
}

pub fn start(faults: Faults) -> FakeS3 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let port = listener.local_addr().expect("address").port();
    let store = Arc::new(Mutex::new(Store::default()));
    let counters = Arc::new(Counters::default());
    let stop = Arc::new(AtomicBool::new(false));
    let shared = Arc::new(Shared { faults, store: store.clone(), counters: counters.clone(), complete_failed: AtomicBool::new(false) });
    let s2 = stop.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if s2.load(Ordering::SeqCst) {
                return;
            }
            let sh = shared.clone();
            std::thread::spawn(move || {
                let _ = serve(stream, &sh);
            });
        }
    });
    FakeS3 { port, store, counters, stop }
}

fn reply(out: &mut TcpStream, status: &str, extra: &str, body: &str) -> std::io::Result<()> {
    write!(out, "HTTP/1.1 {}\r\n{}Content-Length: {}\r\n\r\n{}", status, extra, body.len(), body)
}

fn query_value<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|p| p.split_once('=').filter(|(k, _)| *k == name).map(|(_, v)| v))
}

fn serve(stream: TcpStream, sh: &Shared) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut out = stream;
    loop {
        let mut first = String::new();
        if reader.read_line(&mut first)? == 0 {
            return Ok(());
        }
        let mut words = first.split_whitespace();
        let method = words.next().unwrap_or("").to_string();
        let target = words.next().unwrap_or("/").to_string();
        let (path, query) = target.split_once('?').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((target.clone(), String::new()));
        let (mut length, mut sha, mut auth) = (0usize, String::new(), String::new());
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                return Ok(());
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                match k.to_ascii_lowercase().as_str() {
                    "content-length" => length = v.trim().parse().unwrap_or(0),
                    "x-amz-content-sha256" => sha = v.trim().to_string(),
                    "authorization" => auth = v.trim().to_string(),
                    _ => {}
                }
            }
        }
        let is_part = method == "PUT" && query.contains("partNumber=");
        let n = if is_part { sh.counters.part_requests.fetch_add(1, Ordering::SeqCst) + 1 } else { 0 };
        let f = &sh.faults;
        if is_part && f.cut_every > 0 && n % f.cut_every == 0 {
            // Reads part of the data, then drops the connection.
            let mut some = vec![0u8; length / 2];
            let _ = reader.read_exact(&mut some);
            return Ok(());
        }
        // The body, at the pace this part upload is allowed.
        let mut body = vec![0u8; length];
        if is_part && f.part_bps > 0 {
            let began = Instant::now();
            let mut got = 0;
            while got < length {
                let piece = (length - got).min(((f.part_bps / 20).max(1024)) as usize);
                reader.read_exact(&mut body[got..got + piece])?;
                got += piece;
                let due = Duration::from_secs_f64(got as f64 / f.part_bps as f64);
                if due > began.elapsed() {
                    std::thread::sleep(due - began.elapsed());
                }
            }
        } else {
            reader.read_exact(&mut body)?;
        }
        if !auth.starts_with("AWS4-HMAC-SHA256 Credential=") {
            sh.counters.unsigned_refused.fetch_add(1, Ordering::SeqCst);
            reply(&mut out, "403 Forbidden", "", "<Error><Code>AccessDenied</Code></Error>")?;
            continue;
        }
        if method != "HEAD" && sha256_hex(&body) != sha {
            sh.counters.bad_data_refused.fetch_add(1, Ordering::SeqCst);
            reply(&mut out, "400 Bad Request", "", "<Error><Code>XAmzContentSHA256Mismatch</Code></Error>")?;
            continue;
        }
        if is_part && f.error_every > 0 && n % f.error_every == 0 {
            reply(&mut out, "500 Internal Server Error", "", "<Error><Code>InternalError</Code></Error>")?;
            continue;
        }
        if is_part && f.slow_down_every > 0 && n % f.slow_down_every == 0 {
            reply(&mut out, "503 Slow Down", "Retry-After: 1\r\n", "<Error><Code>SlowDown</Code></Error>")?;
            continue;
        }
        handle(&mut out, sh, &method, &path, &query, body)?;
    }
}

fn handle(out: &mut TcpStream, sh: &Shared, method: &str, path: &str, query: &str, body: Vec<u8>) -> std::io::Result<()> {
    let mut st = sh.store.lock().unwrap();
    let upload_id = query_value(query, "uploadId").map(str::to_string);
    match (method, upload_id) {
        ("POST", None) => {
            st.next_upload += 1;
            let id = format!("upload-{}", st.next_upload);
            st.uploads.insert(id.clone(), (path.to_string(), HashMap::new()));
            sh.counters.created.fetch_add(1, Ordering::SeqCst);
            reply(out, "200 OK", "", &format!("<InitiateMultipartUploadResult><UploadId>{}</UploadId></InitiateMultipartUploadResult>", id))
        }
        ("PUT", Some(id)) => {
            let number: u32 = query_value(query, "partNumber").and_then(|v| v.parse().ok()).unwrap_or(0);
            let Some((_, parts)) = st.uploads.get_mut(&id) else {
                return reply(out, "404 Not Found", "", "<Error><Code>NoSuchUpload</Code></Error>");
            };
            let tag = format!("\"{}\"", &sha256_hex(&body)[..32]);
            sh.counters.parts_accepted.fetch_add(1, Ordering::SeqCst);
            sh.counters.bytes_accepted.fetch_add(body.len() as u64, Ordering::SeqCst);
            parts.insert(number, body);
            reply(out, "200 OK", &format!("ETag: {}\r\n", tag), "")
        }
        ("POST", Some(id)) => {
            if sh.faults.complete_error_once && !sh.complete_failed.swap(true, Ordering::SeqCst) {
                return reply(out, "200 OK", "", "<Error><Code>InternalError</Code></Error>");
            }
            let Some((target, parts)) = st.uploads.get(&id) else {
                return reply(out, "404 Not Found", "", "<Error><Code>NoSuchUpload</Code></Error>");
            };
            let list = String::from_utf8_lossy(&body).to_string();
            let mut object = vec![];
            let mut last = 0u32;
            let mut entries = list.split("<Part>").skip(1).peekable();
            while let Some(entry) = entries.next() {
                let n: u32 = entry.split("<PartNumber>").nth(1).and_then(|x| x.split('<').next()).and_then(|x| x.parse().ok()).unwrap_or(0);
                let tag = entry.split("<ETag>").nth(1).and_then(|x| x.split('<').next()).unwrap_or("").replace("&quot;", "\"");
                let Some(data) = parts.get(&n) else {
                    return reply(out, "400 Bad Request", "", "<Error><Code>InvalidPart</Code></Error>");
                };
                if n <= last || tag != format!("\"{}\"", &sha256_hex(data)[..32]) {
                    return reply(out, "400 Bad Request", "", "<Error><Code>InvalidPartOrder</Code></Error>");
                }
                // Only the last part may be small, as on real storage.
                if entries.peek().is_some() && (data.len() as u64) < crate::s3::MIN_PART {
                    return reply(out, "400 Bad Request", "", "<Error><Code>EntityTooSmall</Code></Error>");
                }
                last = n;
                object.extend_from_slice(data);
            }
            let target = target.clone();
            st.uploads.remove(&id);
            st.objects.insert(target, object);
            sh.counters.completed.fetch_add(1, Ordering::SeqCst);
            reply(out, "200 OK", "", "<CompleteMultipartUploadResult></CompleteMultipartUploadResult>")
        }
        ("DELETE", Some(id)) => {
            st.uploads.remove(&id);
            sh.counters.aborted.fetch_add(1, Ordering::SeqCst);
            reply(out, "204 No Content", "", "")
        }
        ("PUT", None) => {
            st.objects.insert(path.to_string(), body);
            reply(out, "200 OK", "", "")
        }
        ("HEAD", None) => match st.objects.get(path) {
            // A HEAD answer states the size without sending it.
            Some(o) => write!(out, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", o.len()),
            None => write!(out, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n"),
        },
        _ => reply(out, "400 Bad Request", "", "<Error><Code>BadRequest</Code></Error>"),
    }
}
