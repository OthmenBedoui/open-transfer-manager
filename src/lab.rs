//! A small HTTP server that serves a predictable file and misbehaves on demand.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const BLOCK: usize = 1 << 20;
const MB: u64 = 1 << 20;

#[derive(Clone, Debug)]
pub struct Scenario {
    pub name: &'static str,
    pub about: &'static str,
    pub size: u64,
    /// Speed limit of each connection, bytes per second.
    pub per_conn_bps: u64,
    /// Speed limit of all connections together.
    pub total_bps: u64,
    /// Beyond this many requests at once, the answer is 429 with Retry-After.
    pub max_active: u64,
    /// Every Nth connection crawls at `slow_bps`.
    pub slow_every: u64,
    pub slow_bps: u64,
    /// Every Nth connection stops sending after `stall_after` bytes and stays open.
    pub stall_every: u64,
    pub stall_after: u64,
    /// Every answer is cut after this many bytes.
    pub cut_after: u64,
    /// The file changes (same size, new content) after this many bytes have been sent.
    pub change_after: u64,
    pub no_ranges: bool,
    pub no_validators: bool,
}

impl Scenario {
    fn base(name: &'static str, about: &'static str, size: u64) -> Self {
        Scenario {
            name,
            about,
            size,
            per_conn_bps: 0,
            total_bps: 0,
            max_active: 0,
            slow_every: 0,
            slow_bps: 0,
            stall_every: 0,
            stall_after: 0,
            cut_after: 0,
            change_after: 0,
            no_ranges: false,
            no_validators: false,
        }
    }

    pub fn all() -> Vec<Scenario> {
        vec![
            Scenario::base("fast", "No limit: only the engine's own cost counts", 300 * MB),
            Scenario { per_conn_bps: 4 * MB, ..Scenario::base("per-connection-limit", "4 MB/s per connection", 64 * MB) },
            Scenario { total_bps: 16 * MB, ..Scenario::base("saturated-line", "16 MB/s in total: more connections don't help", 64 * MB) },
            Scenario {
                per_conn_bps: 4 * MB,
                slow_every: 4,
                slow_bps: 50 * 1024,
                ..Scenario::base("slow-connections", "1 connection in 4 falls to 50 KB/s", 64 * MB)
            },
            Scenario {
                per_conn_bps: 4 * MB,
                stall_every: 3,
                stall_after: 2 * MB,
                ..Scenario::base("stalled-connections", "1 connection in 3 freezes after 2 MB", 64 * MB)
            },
            Scenario {
                per_conn_bps: 4 * MB,
                max_active: 4,
                ..Scenario::base("server-429", "Above 4 requests: 429 with Retry-After", 64 * MB)
            },
            Scenario { per_conn_bps: 8 * MB, cut_after: 4 * MB, ..Scenario::base("cuts", "Every answer is cut after 4 MB", 64 * MB) },
            Scenario {
                per_conn_bps: 4 * MB,
                change_after: 16 * MB,
                ..Scenario::base("changed-file", "The file changes on the way, same size", 64 * MB)
            },
            Scenario { per_conn_bps: 8 * MB, no_ranges: true, ..Scenario::base("no-range", "The server ignores Range", 32 * MB) },
        ]
    }

    pub fn named(name: &str) -> Option<Scenario> {
        Self::all().into_iter().find(|s| s.name == name)
    }
}

/// One block of content per version of the file; other blocks are rotations of it.
fn table(version: u64) -> Vec<u8> {
    let mut x = 0x9E37_79B9_7F4A_7C15u64 ^ version.wrapping_mul(0xD134_2543_DE82_EF95);
    let mut out = Vec::with_capacity(BLOCK);
    while out.len() < BLOCK {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.truncate(BLOCK);
    out
}

/// Fills `out` with the file's bytes from `offset`.
fn fill(tab: &[u8], offset: u64, out: &mut [u8]) {
    let mut done = 0;
    while done < out.len() {
        let pos = offset + done as u64;
        let block = pos / BLOCK as u64;
        let inner = (pos % BLOCK as u64) as usize;
        let n = (BLOCK - inner).min(out.len() - done);
        let rotation = (block.wrapping_mul(7919) % BLOCK as u64) as usize;
        let start = (inner + rotation) % BLOCK;
        let first = n.min(BLOCK - start);
        out[done..done + first].copy_from_slice(&tab[start..start + first]);
        if first < n {
            out[done + first..done + n].copy_from_slice(&tab[..n - first]);
        }
        done += n;
    }
}

/// Is this file exactly the given version of the server's file?
pub fn file_matches(path: &Path, size: u64, version: u64) -> std::io::Result<bool> {
    if std::fs::metadata(path)?.len() != size {
        return Ok(false);
    }
    let tab = table(version);
    let mut f = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(path)?);
    let mut got = vec![0u8; 1 << 20];
    let mut want = vec![0u8; 1 << 20];
    let mut pos = 0;
    while pos < size {
        let n = ((size - pos) as usize).min(got.len());
        f.read_exact(&mut got[..n])?;
        fill(&tab, pos, &mut want[..n]);
        if got[..n] != want[..n] {
            return Ok(false);
        }
        pos += n as u64;
    }
    Ok(true)
}

/// Are these bytes exactly the given version of the server's file?
pub fn bytes_match(data: &[u8], size: u64, version: u64) -> bool {
    if data.len() as u64 != size {
        return false;
    }
    let tab = table(version);
    let mut want = vec![0u8; 1 << 20];
    let mut pos = 0usize;
    while pos < data.len() {
        let n = (data.len() - pos).min(want.len());
        fill(&tab, pos as u64, &mut want[..n]);
        if data[pos..pos + n] != want[..n] {
            return false;
        }
        pos += n;
    }
    true
}

#[derive(Default)]
pub struct Metrics {
    pub connections: AtomicU64,
    pub requests: AtomicU64,
    pub bytes_sent: AtomicU64,
    pub rejected_429: AtomicU64,
    pub active: AtomicU64,
    pub peak_active: AtomicU64,
    pub version: AtomicU64,
}

struct Shared {
    scenario: Scenario,
    metrics: Arc<Metrics>,
    tables: Vec<Vec<u8>>,
    line: Mutex<Instant>,
    stop: AtomicBool,
}

pub struct Server {
    pub port: u16,
    pub metrics: Arc<Metrics>,
    shared: Arc<Shared>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
    }
}

pub fn start(scenario: Scenario) -> Server {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let port = listener.local_addr().expect("address").port();
    let metrics = Arc::new(Metrics::default());
    let shared = Arc::new(Shared {
        scenario,
        metrics: metrics.clone(),
        tables: vec![table(0), table(1)],
        line: Mutex::new(Instant::now()),
        stop: AtomicBool::new(false),
    });
    let s2 = shared.clone();
    std::thread::spawn(move || {
        for (n, stream) in listener.incoming().flatten().enumerate() {
            if s2.stop.load(Ordering::SeqCst) {
                return;
            }
            s2.metrics.connections.fetch_add(1, Ordering::Relaxed);
            let s3 = s2.clone();
            std::thread::spawn(move || {
                let _ = serve(stream, n as u64 + 1, &s3);
            });
        }
    });
    Server { port, metrics, shared }
}

fn etag(version: u64) -> String {
    format!("\"lab-v{}\"", version)
}

fn serve(stream: TcpStream, conn: u64, sh: &Shared) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut out = stream;
    loop {
        let mut request = String::new();
        if reader.read_line(&mut request)? == 0 {
            return Ok(());
        }
        let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
        let (mut range, mut if_range) = (None, None);
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
                    "range" => range = Some(v.trim().to_string()),
                    "if-range" => if_range = Some(v.trim().to_string()),
                    _ => {}
                }
            }
        }
        if path == "/__metrics" {
            let m = &sh.metrics;
            let body = format!(
                "{} {} {} {} {} {}",
                m.connections.load(Ordering::Relaxed),
                m.requests.load(Ordering::Relaxed).saturating_sub(1),
                m.bytes_sent.load(Ordering::Relaxed),
                m.rejected_429.load(Ordering::Relaxed),
                m.peak_active.load(Ordering::Relaxed),
                m.version.load(Ordering::Relaxed),
            );
            sh.metrics.requests.fetch_add(1, Ordering::Relaxed);
            write!(out, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body)?;
            return Ok(());
        }
        sh.metrics.requests.fetch_add(1, Ordering::Relaxed);
        let scn = &sh.scenario;
        let active = sh.metrics.active.fetch_add(1, Ordering::SeqCst) + 1;
        sh.metrics.peak_active.fetch_max(active, Ordering::SeqCst);
        let result = (|| -> std::io::Result<bool> {
            if scn.max_active > 0 && active > scn.max_active {
                sh.metrics.rejected_429.fetch_add(1, Ordering::Relaxed);
                write!(out, "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Length: 0\r\n\r\n")?;
                return Ok(true);
            }
            answer(&mut out, conn, range.as_deref(), if_range.as_deref(), sh)
        })();
        sh.metrics.active.fetch_sub(1, Ordering::SeqCst);
        if !result? {
            return Ok(());
        }
    }
}

/// Returns whether the connection can serve another request.
fn answer(out: &mut TcpStream, conn: u64, range: Option<&str>, if_range: Option<&str>, sh: &Shared) -> std::io::Result<bool> {
    let scn = &sh.scenario;
    let version = sh.metrics.version.load(Ordering::SeqCst);
    let current = etag(version);
    let range_applies = !scn.no_ranges && if_range.map(|v| v == current).unwrap_or(true);
    let wanted = range.filter(|_| range_applies).and_then(|r| parse_range(r, scn.size));
    let (start, end, partial) = match wanted {
        Some(Some((a, b))) => (a, b, true),
        Some(None) => {
            write!(out, "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\n\r\n", scn.size)?;
            return Ok(true);
        }
        None => (0, scn.size, false),
    };
    let mut head = format!(
        "HTTP/1.1 {}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n",
        if partial { "206 Partial Content" } else { "200 OK" },
        end - start
    );
    if partial {
        head.push_str(&format!("Content-Range: bytes {}-{}/{}\r\n", start, end - 1, scn.size));
    }
    if !scn.no_ranges {
        head.push_str("Accept-Ranges: bytes\r\n");
    }
    if !scn.no_validators {
        head.push_str(&format!("ETag: {}\r\nLast-Modified: Mon, 0{} Jan 2024 00:00:00 GMT\r\n", current, version + 1));
    }
    head.push_str("\r\n");
    out.write_all(head.as_bytes())?;

    let stalls = scn.stall_every > 0 && conn % scn.stall_every == 0;
    let slow = scn.slow_every > 0 && conn % scn.slow_every == 0;
    let conn_bps = if slow { scn.slow_bps } else { scn.per_conn_bps };
    let tab = &sh.tables[version.min(1) as usize];
    let mut buf = vec![0u8; 64 * 1024];
    let mut pos = start;
    let mut sent = 0u64;
    let began = Instant::now();
    while pos < end {
        if sh.stop.load(Ordering::SeqCst) {
            return Ok(false);
        }
        if stalls && sent >= scn.stall_after {
            // Silent but open, as a stuck proxy would be, until the client gives up.
            let _ = out.set_read_timeout(Some(Duration::from_secs(120)));
            let mut one = [0u8; 1];
            let _ = out.read(&mut one);
            return Ok(false);
        }
        if scn.cut_after > 0 && sent >= scn.cut_after {
            return Ok(false);
        }
        let mut n = ((end - pos) as usize).min(buf.len());
        if conn_bps > 0 {
            // Short pieces at low speeds, so the pace stays even.
            n = n.min(((conn_bps / 20).max(1024)) as usize);
        }
        fill(tab, pos, &mut buf[..n]);
        out.write_all(&buf[..n])?;
        pos += n as u64;
        sent += n as u64;
        let total_sent = sh.metrics.bytes_sent.fetch_add(n as u64, Ordering::Relaxed) + n as u64;
        if scn.change_after > 0 && total_sent >= scn.change_after {
            sh.metrics.version.store(1, Ordering::SeqCst);
        }
        if conn_bps > 0 {
            let due = Duration::from_secs_f64(sent as f64 / conn_bps as f64);
            let spent = began.elapsed();
            if due > spent {
                std::thread::sleep(due - spent);
            }
        }
        if scn.total_bps > 0 {
            let wait = {
                let mut next = sh.line.lock().unwrap();
                let now = Instant::now();
                if *next < now {
                    *next = now;
                }
                let w = *next - now;
                *next += Duration::from_secs_f64(n as f64 / scn.total_bps as f64);
                w
            };
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
        }
    }
    Ok(true)
}

/// Outer None: not a range this server reads. Inner None: outside the file.
fn parse_range(value: &str, size: u64) -> Option<Option<(u64, u64)>> {
    let spec = value.strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        return Some((n > 0 && size > 0).then(|| (size.saturating_sub(n), size)));
    }
    let start: u64 = a.parse().ok()?;
    if start >= size {
        return Some(None);
    }
    let end = if b.is_empty() { size } else { (b.parse::<u64>().ok()? + 1).min(size) };
    if end <= start {
        return None;
    }
    Some(Some((start, end)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_read_like_the_standard_says() {
        assert_eq!(parse_range("bytes=0-0", 10), Some(Some((0, 1))));
        assert_eq!(parse_range("bytes=2-", 10), Some(Some((2, 10))));
        assert_eq!(parse_range("bytes=2-99", 10), Some(Some((2, 10))));
        assert_eq!(parse_range("bytes=-3", 10), Some(Some((7, 10))));
        assert_eq!(parse_range("bytes=10-", 10), Some(None));
        assert_eq!(parse_range("items=1-2", 10), None);
    }

    #[test]
    fn content_depends_on_position_and_version() {
        let (t0, t1) = (table(0), table(1));
        let mut a = vec![0u8; 3 * BLOCK];
        fill(&t0, 0, &mut a);
        let mut part = vec![0u8; 1000];
        fill(&t0, BLOCK as u64 - 500, &mut part);
        assert_eq!(part[..], a[BLOCK - 500..BLOCK + 500]);
        assert_ne!(a[..BLOCK], a[BLOCK..2 * BLOCK]);
        let mut b = vec![0u8; 1000];
        fill(&t1, 0, &mut b);
        assert_ne!(a[..1000], b[..]);
    }
}
