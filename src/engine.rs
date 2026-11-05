//! Downloads one file over several connections.
//!
//! - One file on disk: each piece is written at its place. A journal lists what is safely
//!   on disk, so a stopped download continues from there. Nothing is joined at the end.
//! - Pieces are sized by time: a request should last a few seconds at the speed measured.
//! - The number of connections follows the results: it grows while the total speed really
//!   grows, and the server is listened to (429, 503, Retry-After).
//! - Every answer is checked against the first (If-Range, ETag, Content-Range). If the
//!   file changed on the server, the download starts over instead of mixing two versions.
//! - A connection far slower than the others is replaced. The last pieces can be asked
//!   for twice, and the first answer wins.
use crate::ranges::RangeSet;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Counters a display can read while a transfer runs.
#[derive(Debug, Default)]
pub struct Progress {
    size: AtomicU64,
    received: AtomicU64,
    sent: AtomicU64,
}

impl Progress {
    pub fn size(&self) -> u64 {
        self.size.load(Ordering::Relaxed)
    }
    pub fn received(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }
    pub fn sent(&self) -> u64 {
        self.sent.load(Ordering::Relaxed)
    }
    pub fn set_received(&self, size: u64, received: u64) {
        self.size.store(size, Ordering::Relaxed);
        self.received.store(received, Ordering::Relaxed);
    }
    pub fn set_sent(&self, sent: u64) {
        self.sent.store(sent, Ordering::Relaxed);
    }
    /// For a server without ranges, where nothing is known until the end.
    pub fn finish(&self, size: u64) {
        self.set_received(size, size);
    }
}

/// Lets another part of the program follow what is safely in the local file.
#[derive(Debug, Default)]
pub struct Watch {
    inner: Mutex<Option<WatchView>>,
    progress: Option<Arc<Progress>>,
}

#[derive(Clone, Debug)]
pub struct WatchView {
    /// Grows each time the download starts on a different version of the file.
    pub round: u32,
    pub size: u64,
    pub etag: String,
    pub modified: String,
    pub done: RangeSet,
}

impl Watch {
    pub fn with_progress(progress: Arc<Progress>) -> Self {
        Watch { inner: Mutex::new(None), progress: Some(progress) }
    }

    pub fn begin(&self, size: u64, etag: &str, modified: &str, done: &RangeSet) {
        if let Some(p) = &self.progress {
            p.set_received(size, done.len());
        }
        let mut v = self.inner.lock().unwrap();
        let round = v.as_ref().map(|x| x.round).unwrap_or(0) + 1;
        *v = Some(WatchView { round, size, etag: etag.to_string(), modified: modified.to_string(), done: done.clone() });
    }

    fn written(&self, start: u64, end: u64) {
        if let Some(v) = self.inner.lock().unwrap().as_mut() {
            v.done.add(start, end);
            if let Some(p) = &self.progress {
                p.set_received(v.size, v.done.len());
            }
        }
    }

    pub fn snapshot(&self) -> Option<WatchView> {
        self.inner.lock().unwrap().clone()
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub max_connections: usize,
    pub start_connections: usize,
    /// How long one request should last.
    pub piece_seconds: f64,
    pub min_piece: u64,
    pub max_piece: u64,
    /// With no data for this long, a connection is given up.
    pub stall_timeout: Duration,
    /// With no progress at all for this long, the download fails.
    pub give_up_after: Duration,
    /// How many times to start over because the file changed.
    pub max_restarts: u32,
    pub cookies: String,
    pub user_agent: String,
    /// Sends the journal to disk this often.
    pub journal_every: Duration,
    /// Told about every range written, for whoever follows this download.
    pub watch: Option<Arc<Watch>>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            max_connections: 16,
            start_connections: 2,
            piece_seconds: 3.0,
            min_piece: 256 * 1024,
            max_piece: 64 * 1024 * 1024,
            stall_timeout: Duration::from_secs(4),
            give_up_after: Duration::from_secs(60),
            max_restarts: 3,
            cookies: String::new(),
            user_agent: "open-transfer-manager/0.1".into(),
            journal_every: Duration::from_secs(1),
            watch: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub size: u64,
    pub seconds: f64,
    pub requests: u64,
    pub peak_connections: usize,
    pub throttled: u64,
    pub retries: u64,
    pub restarts: u32,
    pub replaced_slow: u64,
    pub doubled_pieces: u64,
    pub resumed_bytes: u64,
    /// False when the server gave no ETag or date, so a change couldn't be seen.
    pub checked_for_changes: bool,
    pub used_ranges: bool,
}

#[derive(Debug, PartialEq)]
pub enum Error {
    Stopped,
    Http(u16),
    Failed(String),
    KeepsChanging,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Stopped => write!(f, "stopped; the download can continue later"),
            Error::Http(c) => write!(f, "the server answered HTTP {}", c),
            Error::Failed(m) => write!(f, "{}", m),
            Error::KeepsChanging => write!(f, "the file keeps changing on the server"),
        }
    }
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

fn journal_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part.journal");
    PathBuf::from(s)
}

#[cfg(windows)]
fn write_at(f: &File, mut data: &[u8], mut at: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !data.is_empty() {
        let n = f.seek_write(data, at)?;
        data = &data[n..];
        at += n as u64;
    }
    Ok(())
}

#[cfg(unix)]
fn write_at(f: &File, data: &[u8], at: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    f.write_all_at(data, at)
}

// ------------------------------------------------------------------ Journal

#[derive(Debug, PartialEq)]
struct Journal {
    size: u64,
    etag: String,
    modified: String,
    done: RangeSet,
}

impl Journal {
    fn to_text(&self) -> String {
        let mut t = format!("otm-journal 1\nsize {}\netag {}\nmodified {}\n", self.size, self.etag, self.modified);
        for (a, b) in self.done.items() {
            t.push_str(&format!("done {} {}\n", a, b));
        }
        t.push_str("end\n");
        t
    }

    /// None for anything damaged: a cut-off journal is never trusted.
    fn parse(text: &str) -> Option<Journal> {
        let mut lines = text.lines();
        if lines.next()? != "otm-journal 1" {
            return None;
        }
        let size = lines.next()?.strip_prefix("size ")?.parse().ok()?;
        let etag = lines.next()?.strip_prefix("etag ")?.to_string();
        let modified = lines.next()?.strip_prefix("modified ")?.to_string();
        let mut done = RangeSet::new();
        for line in lines {
            if line == "end" {
                return Some(Journal { size, etag, modified, done });
            }
            let mut p = line.strip_prefix("done ")?.split(' ');
            let (a, b): (u64, u64) = (p.next()?.parse().ok()?, p.next()?.parse().ok()?);
            if a >= b || b > size {
                return None;
            }
            done.add(a, b);
        }
        None
    }

    fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        fs::write(&tmp, self.to_text())?;
        fs::rename(&tmp, path)
    }
}

// ------------------------------------------------------------------ HTTP

struct Http {
    agent: ureq::Agent,
    origin_host: String,
    cookies: String,
    user_agent: String,
}

enum Answer {
    Body(Box<ureq::Response>),
    Status(u16, Option<u64>),
    /// The server said the file holds nothing, so no range of it exists.
    Empty,
    Transport(String),
}

fn host_of(u: &str) -> String {
    url::Url::parse(u).ok().and_then(|p| p.host_str().map(str::to_string)).unwrap_or_default()
}

fn retry_after(resp: &ureq::Response) -> Option<u64> {
    resp.header("Retry-After").and_then(|v| v.trim().parse().ok())
}

impl Http {
    fn new(url: &str, opts: &Options) -> Result<Http, Error> {
        let tls = native_tls::TlsConnector::new().map_err(|e| Error::Failed(e.to_string()))?;
        let agent = ureq::AgentBuilder::new()
            .tls_connector(Arc::new(tls))
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(opts.stall_timeout)
            .timeout_write(Duration::from_secs(15))
            .redirects(0)
            .max_idle_connections(opts.max_connections * 2)
            .max_idle_connections_per_host(opts.max_connections * 2)
            .build();
        Ok(Http { agent, origin_host: host_of(url), cookies: opts.cookies.clone(), user_agent: opts.user_agent.clone() })
    }

    /// Follows redirects itself: cookies stay on the site they belong to, and only there.
    fn get(&self, url: &str, range: &str, if_range: Option<&str>) -> (Answer, String) {
        let mut current = url.to_string();
        for _ in 0..10 {
            let mut r = self.agent.get(&current).set("User-Agent", &self.user_agent).set("Range", range).set("Accept-Encoding", "identity");
            if let Some(v) = if_range {
                r = r.set("If-Range", v);
            }
            if !self.cookies.is_empty() && host_of(&current) == self.origin_host {
                r = r.set("Cookie", &self.cookies);
            }
            match r.call() {
                Ok(resp) if (300..400).contains(&resp.status()) => {
                    let Some(next) = resp.header("Location").and_then(|l| url::Url::parse(&current).ok()?.join(l).ok()) else {
                        return (Answer::Status(resp.status(), None), current);
                    };
                    current = next.to_string();
                }
                Ok(resp) => return (Answer::Body(Box::new(resp)), current),
                Err(ureq::Error::Status(416, resp)) if resp.header("Content-Range").map(|v| v.trim() == "bytes */0").unwrap_or(false) => return (Answer::Empty, current),
                Err(ureq::Error::Status(code, resp)) => return (Answer::Status(code, retry_after(&resp)), current),
                Err(e) => return (Answer::Transport(e.to_string()), current),
            }
        }
        (Answer::Transport("too many redirects".into()), current)
    }
}

/// "bytes 10-19/100" gives (10, 20, Some(100)).
fn parse_content_range(v: &str) -> Option<(u64, u64, Option<u64>)> {
    let rest = v.trim().strip_prefix("bytes ")?;
    let (span, total) = rest.split_once('/')?;
    let (a, b) = span.split_once('-')?;
    Some((a.parse().ok()?, b.parse::<u64>().ok()? + 1, total.parse().ok()))
}

fn strong_etag(resp: &ureq::Response) -> String {
    resp.header("ETag").filter(|e| !e.trim_start().starts_with("W/")).unwrap_or("").trim().to_string()
}

// ------------------------------------------------------------------ Shared state

struct Flight {
    start: u64,
    pos: u64,
    end: u64,
    began: Instant,
    last_data: Instant,
    doubled: bool,
    cancel: Arc<AtomicBool>,
}

impl Flight {
    fn speed(&self) -> f64 {
        (self.pos - self.start) as f64 / self.began.elapsed().as_secs_f64().max(0.01)
    }
}

#[derive(PartialEq)]
enum Halt {
    Running,
    Changed,
    Fatal,
}

struct State {
    total: u64,
    /// Handed to the writer. What a running request still has to bring is in `flights`.
    claimed: RangeSet,
    /// Written to the file.
    done: RangeSet,
    flights: HashMap<u64, Flight>,
    next_flight: u64,
    target: usize,
    workers: usize,
    not_before: Instant,
    last_progress: Instant,
    halt: Halt,
    fatal: Option<Error>,
    report: Report,
}

enum Job {
    Piece { id: u64, start: u64, end: u64, cancel: Arc<AtomicBool> },
    Wait,
    Leave,
}

impl State {
    /// The next piece for a connection running at `speed`.
    fn next_job(&mut self, speed: f64, opts: &Options) -> Job {
        if self.halt != Halt::Running || self.workers > self.target || self.claimed.first_gap(self.total).is_none() {
            return Job::Leave;
        }
        let mut busy = self.claimed.clone();
        for f in self.flights.values() {
            busy.add(f.pos, f.end);
        }
        if let Some((a, b)) = busy.first_gap(self.total) {
            let wanted = if speed > 0.0 { (speed * opts.piece_seconds) as u64 } else { opts.min_piece * 4 };
            let wanted = wanted.clamp(opts.min_piece, opts.max_piece);
            let mut end = a.saturating_add(wanted).min(b);
            // No tiny leftover that would need a request of its own.
            if b - end < opts.min_piece {
                end = b;
            }
            return self.open_flight(a, end, false);
        }
        // Everything is reserved. Take over the second half of the biggest piece still to
        // come: without this, a few large pieces would leave new connections nothing to do.
        let mut biggest: Option<(u64, u64)> = None;
        for (id, f) in &self.flights {
            let left = f.end.saturating_sub(f.pos);
            if left >= 2 * opts.min_piece && biggest.map(|b| left > b.1).unwrap_or(true) {
                biggest = Some((*id, left));
            }
        }
        if let Some((id, left)) = biggest {
            let f = self.flights.get_mut(&id).unwrap();
            let (middle, end) = (f.pos + left / 2, f.end);
            f.end = middle;
            return self.open_flight(middle, end, false);
        }
        // Only small rests are left. Help the one that will finish last, if it's worth it.
        let mut worst: Option<(u64, f64)> = None;
        for (id, f) in &self.flights {
            let left = f.end.saturating_sub(f.pos);
            if f.doubled || left == 0 {
                continue;
            }
            let eta = left as f64 / f.speed().max(1.0);
            if eta > 1.5 && worst.map(|w| eta > w.1).unwrap_or(true) {
                worst = Some((*id, eta));
            }
        }
        if let Some((id, _)) = worst {
            let f = self.flights.get_mut(&id).unwrap();
            f.doubled = true;
            let (a, b) = (f.pos, f.end);
            self.report.doubled_pieces += 1;
            return self.open_flight(a, b, true);
        }
        Job::Wait
    }

    fn open_flight(&mut self, start: u64, end: u64, doubled: bool) -> Job {
        let id = self.next_flight;
        self.next_flight += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        let now = Instant::now();
        self.flights.insert(id, Flight { start, pos: start, end, began: now, last_data: now, doubled, cancel: cancel.clone() });
        Job::Piece { id, start, end, cancel }
    }

    fn halt_fatal(&mut self, e: Error) {
        if self.fatal.is_none() {
            self.fatal = Some(e);
        }
        self.halt = Halt::Fatal;
    }
}

struct Session {
    url: String,
    http: Http,
    opts: Options,
    if_range: Option<String>,
    etag: String,
    state: Mutex<State>,
    writer: SyncSender<(u64, Vec<u8>)>,
    stop: Arc<AtomicBool>,
}

enum PieceEnd {
    Finished,
    Failed,
    Throttled,
}

fn backoff(failures: u32) -> Duration {
    // Waits grow, and a varying share keeps connections from retrying all together.
    let base = 0.25 * 2f64.powi(failures.min(6) as i32);
    let spread = (Instant::now().elapsed().subsec_nanos() as f64 + std::process::id() as f64 * 7919.0 + failures as f64 * 104_729.0) % 1000.0 / 1000.0;
    Duration::from_secs_f64(base.min(15.0) * (0.5 + 0.5 * spread))
}

fn run_piece(s: &Session, id: u64, start: u64, end: u64, cancel: &AtomicBool) -> PieceEnd {
    let range = format!("bytes={}-{}", start, end - 1);
    s.state.lock().unwrap().report.requests += 1;
    let (answer, _) = s.http.get(&s.url, &range, s.if_range.as_deref());
    let resp = match answer {
        Answer::Body(r) => r,
        Answer::Status(429 | 503, wait) => {
            let mut st = s.state.lock().unwrap();
            st.report.throttled += 1;
            let until = Instant::now() + Duration::from_secs(wait.unwrap_or(2).min(120));
            if until > st.not_before {
                st.not_before = until;
                st.target = (st.target / 2).max(1);
            }
            return PieceEnd::Throttled;
        }
        Answer::Status(code, _) if code >= 500 => return PieceEnd::Failed,
        Answer::Status(code, _) => {
            s.state.lock().unwrap().halt_fatal(Error::Http(code));
            return PieceEnd::Failed;
        }
        Answer::Empty | Answer::Transport(_) => return PieceEnd::Failed,
    };
    // A full answer to If-Range means another version of the file.
    if resp.status() != 206 {
        let mut st = s.state.lock().unwrap();
        if s.if_range.is_some() {
            st.halt = Halt::Changed;
        } else {
            st.halt_fatal(Error::Failed("the server stopped honouring ranges".into()));
        }
        return PieceEnd::Failed;
    }
    let content = resp.header("Content-Range").and_then(parse_content_range);
    let total = s.state.lock().unwrap().total;
    let tag = strong_etag(&resp);
    let same_file = content.map(|c| c.2.map(|t| t == total).unwrap_or(true)).unwrap_or(false) && (s.etag.is_empty() || tag.is_empty() || tag == s.etag);
    if !same_file {
        s.state.lock().unwrap().halt = Halt::Changed;
        return PieceEnd::Failed;
    }
    let (from, until, _) = content.unwrap();
    if from != start || until > end {
        return PieceEnd::Failed;
    }
    let mut body = resp.into_reader();
    let mut pos = start;
    // Another connection may take over the end of this piece while it runs.
    let mut end = end;
    loop {
        if cancel.load(Ordering::Relaxed) || s.stop.load(Ordering::Relaxed) {
            return PieceEnd::Failed;
        }
        let mut buf = vec![0u8; 64 * 1024];
        let n = match body.read(&mut buf) {
            Ok(n) => n,
            Err(_) => return PieceEnd::Failed,
        };
        if n == 0 {
            return if pos >= end { PieceEnd::Finished } else { PieceEnd::Failed };
        }
        {
            let mut st = s.state.lock().unwrap();
            if st.halt != Halt::Running {
                return PieceEnd::Failed;
            }
            if let Some(f) = st.flights.get(&id) {
                end = f.end;
            }
            if pos >= end {
                return PieceEnd::Finished;
            }
            let n = n.min((end - pos) as usize);
            buf.truncate(n);
            st.claimed.add(pos, pos + n as u64);
            st.last_progress = Instant::now();
            if let Some(f) = st.flights.get_mut(&id) {
                f.pos = pos + n as u64;
                f.last_data = Instant::now();
            }
            // The other copy of a doubled piece already brought the rest.
            if st.claimed.covers(pos + n as u64, end) && pos + n as u64 != end {
                drop(st);
                let _ = s.writer.send((pos, buf));
                return PieceEnd::Finished;
            }
        }
        let n = buf.len();
        // Waits here when the disk can't keep up: the connections slow down, nothing piles up.
        if s.writer.send((pos, buf)).is_err() {
            return PieceEnd::Failed;
        }
        pos += n as u64;
    }
}

fn worker(s: Arc<Session>) {
    let mut failures = 0u32;
    let mut speed = 0.0f64;
    loop {
        if s.stop.load(Ordering::Relaxed) {
            break;
        }
        let wait = s.state.lock().unwrap().not_before.saturating_duration_since(Instant::now());
        if !wait.is_zero() {
            std::thread::sleep(wait.min(Duration::from_millis(200)));
            continue;
        }
        let job = s.state.lock().unwrap().next_job(speed, &s.opts);
        match job {
            Job::Leave => break,
            Job::Wait => std::thread::sleep(Duration::from_millis(50)),
            Job::Piece { id, start, end, cancel } => {
                let began = Instant::now();
                let result = run_piece(&s, id, start, end, &cancel);
                let (got, mut st) = {
                    let mut st = s.state.lock().unwrap();
                    let got = st.flights.remove(&id).map(|f| f.pos - f.start).unwrap_or(0);
                    (got, st)
                };
                match result {
                    PieceEnd::Finished => {
                        failures = 0;
                        let now = got as f64 / began.elapsed().as_secs_f64().max(0.01);
                        speed = if speed == 0.0 { now } else { speed * 0.5 + now * 0.5 };
                    }
                    PieceEnd::Throttled => {}
                    PieceEnd::Failed => {
                        if got > 0 {
                            failures = 0;
                        } else {
                            failures += 1;
                        }
                        if !cancel.load(Ordering::Relaxed) {
                            st.report.retries += 1;
                            drop(st);
                            std::thread::sleep(backoff(failures));
                        }
                    }
                }
            }
        }
    }
    s.state.lock().unwrap().workers -= 1;
}

#[allow(clippy::too_many_arguments)]
fn writer_thread(file: File, rx: Receiver<(u64, Vec<u8>)>, session_state: Arc<Mutex<Option<Arc<Session>>>>, journal: PathBuf, meta: (u64, String, String), every: Duration, seed: RangeSet, watch: Option<Arc<Watch>>) -> Result<(), String> {
    let mut done_since_save = false;
    let mut last = Instant::now();
    let mut done = seed;
    let save = |done: &RangeSet| -> Result<(), String> {
        // The data first: the journal must never promise what the disk doesn't hold.
        file.sync_data().map_err(|e| e.to_string())?;
        Journal { size: meta.0, etag: meta.1.clone(), modified: meta.2.clone(), done: done.clone() }.save(&journal).map_err(|e| e.to_string())
    };
    while let Ok((at, data)) = rx.recv() {
        write_at(&file, &data, at).map_err(|e| format!("cannot write the file: {}", e))?;
        done.add(at, at + data.len() as u64);
        if let Some(w) = &watch {
            w.written(at, at + data.len() as u64);
        }
        done_since_save = true;
        if let Some(s) = session_state.lock().unwrap().as_ref() {
            s.state.lock().unwrap().done = done.clone();
        }
        if last.elapsed() >= every {
            save(&done)?;
            done_since_save = false;
            last = Instant::now();
        }
    }
    if done_since_save {
        save(&done)?;
    }
    if let Some(s) = session_state.lock().unwrap().as_ref() {
        s.state.lock().unwrap().done = done;
    }
    Ok(())
}

// ------------------------------------------------------------------ Control

/// Grows the connections while the total speed really grows, and goes back when it doesn't.
struct Climber {
    window_start: Instant,
    window_bytes: u64,
    best: f64,
    before_growth: Option<usize>,
    hold_until: Instant,
}

impl Climber {
    fn new() -> Self {
        let now = Instant::now();
        Climber { window_start: now, window_bytes: 0, best: 0.0, before_growth: None, hold_until: now }
    }

    fn step(&mut self, st: &mut State, max: usize) {
        if self.window_start.elapsed() < Duration::from_millis(1000) {
            return;
        }
        let bytes = st.claimed.len();
        let speed = bytes.saturating_sub(self.window_bytes) as f64 / self.window_start.elapsed().as_secs_f64();
        self.window_start = Instant::now();
        self.window_bytes = bytes;
        if Instant::now() < st.not_before || Instant::now() < self.hold_until {
            return;
        }
        // Only worth judging once the extra connections have had time to work.
        if speed > self.best * 1.10 {
            self.best = speed;
            self.before_growth = Some(st.target);
            st.target = (st.target + (st.target / 2).max(1)).min(max);
        } else {
            if let Some(previous) = self.before_growth.take() {
                st.target = previous.max(1);
            }
            // Try one more now and then: conditions change.
            self.hold_until = Instant::now() + Duration::from_secs(10);
            self.best = speed.max(self.best * 0.8);
        }
    }
}

/// Gives up connections that stopped or fell far behind the others.
fn replace_slow(st: &mut State, stall: Duration) {
    let mut speeds: Vec<f64> = st.flights.values().filter(|f| f.began.elapsed() > Duration::from_secs(1)).map(Flight::speed).collect();
    speeds.sort_by(|a, b| a.total_cmp(b));
    let median = speeds.get(speeds.len() / 2).copied().unwrap_or(0.0);
    let mut replaced = 0;
    for f in st.flights.values() {
        if f.cancel.load(Ordering::Relaxed) {
            continue;
        }
        let silent = f.last_data.elapsed() > stall;
        let behind = speeds.len() >= 3 && f.began.elapsed() > Duration::from_secs(2) && f.speed() < median * 0.2 && f.end - f.pos > 256 * 1024;
        if silent || behind {
            f.cancel.store(true, Ordering::Relaxed);
            replaced += 1;
        }
    }
    st.report.replaced_slow += replaced;
}

enum Outcome {
    Done(Report),
    Changed(Report),
}

fn prepare_part(part: &Path, journal: &Path, size: u64, etag: &str, modified: &str) -> Result<(File, RangeSet), Error> {
    let io = |e: std::io::Error| Error::Failed(format!("cannot prepare the file: {}", e));
    let saved = fs::read_to_string(journal).ok().and_then(|t| Journal::parse(&t));
    let usable = saved.filter(|j| {
        // Without an ETag or a date, there's no way to know it's still the same file.
        j.size == size && j.etag == etag && j.modified == modified && (!etag.is_empty() || !modified.is_empty()) && fs::metadata(part).map(|m| m.len() == size).unwrap_or(false)
    });
    let done = usable.map(|j| j.done).unwrap_or_default();
    let mut o = OpenOptions::new();
    o.write(true).read(true).create(true);
    if done.is_empty() {
        o.truncate(true);
    }
    let file = o.open(part).map_err(io)?;
    // Taking the whole size now means a full disk shows at the start, not at 90%.
    file.set_len(size).map_err(io)?;
    Ok((file, done))
}

fn run_ranged(http: Http, url: String, dest: &Path, size: u64, etag: String, modified: String, opts: &Options, stop: &Arc<AtomicBool>, mut report: Report) -> Result<Outcome, Error> {
    let part = part_path(dest);
    let journal = journal_path(dest);
    let (file, seed) = prepare_part(&part, &journal, size, &etag, &modified)?;
    report.resumed_bytes = seed.len();
    if let Some(w) = &opts.watch {
        w.begin(size, &etag, &modified, &seed);
    }
    let if_range = if !etag.is_empty() {
        Some(etag.clone())
    } else if !modified.is_empty() {
        Some(modified.clone())
    } else {
        None
    };
    report.checked_for_changes = if_range.is_some();
    let (tx, rx) = sync_channel(64);
    let now = Instant::now();
    let session = Arc::new(Session {
        http,
        url,
        opts: opts.clone(),
        if_range,
        etag: etag.clone(),
        state: Mutex::new(State {
            total: size,
            claimed: seed.clone(),
            done: seed.clone(),
            flights: HashMap::new(),
            next_flight: 0,
            target: opts.start_connections.clamp(1, opts.max_connections),
            workers: 0,
            not_before: now,
            last_progress: now,
            halt: Halt::Running,
            fatal: None,
            report,
        }),
        writer: tx,
        stop: stop.clone(),
    });
    let link = Arc::new(Mutex::new(Some(session.clone())));
    let (link2, jp, meta, every, seed2, watch) = (link.clone(), journal.clone(), (size, etag, modified), opts.journal_every, seed, opts.watch.clone());
    let writer = std::thread::spawn(move || writer_thread(file, rx, link2, jp, meta, every, seed2, watch));

    let mut climber = Climber::new();
    let mut handles = vec![];
    let result = loop {
        {
            let mut st = session.state.lock().unwrap();
            let all_claimed = st.claimed.first_gap(size).is_none();
            if st.halt == Halt::Running && all_claimed && st.workers == 0 {
                break None;
            }
            if st.halt != Halt::Running || stop.load(Ordering::Relaxed) {
                for f in st.flights.values() {
                    f.cancel.store(true, Ordering::Relaxed);
                }
                if st.workers == 0 {
                    break Some(());
                }
            } else {
                if st.last_progress.elapsed() > opts.give_up_after {
                    st.halt_fatal(Error::Failed("no data has arrived for too long".into()));
                }
                climber.step(&mut st, opts.max_connections);
                replace_slow(&mut st, opts.stall_timeout);
                st.report.peak_connections = st.report.peak_connections.max(st.workers);
                let blocked = Instant::now() < st.not_before;
                while st.workers < st.target && !blocked && !all_claimed {
                    st.workers += 1;
                    let s = session.clone();
                    handles.push(std::thread::spawn(move || worker(s)));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let _ = result;
    for h in handles {
        let _ = h.join();
    }
    *link.lock().unwrap() = None;
    let Ok(session) = Arc::try_unwrap(session) else {
        return Err(Error::Failed("a connection did not finish".into()));
    };
    let Session { state, writer: sender, .. } = session;
    drop(sender);
    let written = writer.join().map_err(|_| Error::Failed("the writer stopped unexpectedly".to_string()))?;
    let mut st = state.into_inner().unwrap();
    if st.halt == Halt::Fatal {
        return Err(st.fatal.take().unwrap_or(Error::Failed("failed".into())));
    }
    written.map_err(Error::Failed)?;
    if st.halt == Halt::Changed {
        return Ok(Outcome::Changed(st.report));
    }
    if stop.load(Ordering::Relaxed) && st.claimed.first_gap(size).is_some() {
        return Err(Error::Stopped);
    }
    st.report.size = size;
    finish(dest, size)?;
    Ok(Outcome::Done(st.report))
}

fn finish(dest: &Path, size: u64) -> Result<(), Error> {
    let part = part_path(dest);
    let real = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if real != size {
        return Err(Error::Failed(format!("the file has {} bytes instead of {}", real, size)));
    }
    let _ = fs::remove_file(dest);
    fs::rename(&part, dest).map_err(|e| Error::Failed(format!("cannot save the file: {}", e)))?;
    let _ = fs::remove_file(journal_path(dest));
    Ok(())
}

/// For a server without ranges: one stream, and a cut means starting over.
fn run_single(mut body: Box<dyn Read + Send>, expected: Option<u64>, dest: &Path, stop: &AtomicBool) -> Result<u64, Option<Error>> {
    use std::io::Write;
    let part = part_path(dest);
    let mut f = std::io::BufWriter::with_capacity(1 << 20, File::create(&part).map_err(|e| Some(Error::Failed(e.to_string())))?);
    let mut got = 0u64;
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err(Some(Error::Stopped));
        }
        match body.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                f.write_all(&buf[..n]).map_err(|e| Some(Error::Failed(e.to_string())))?;
                got += n as u64;
            }
            Err(_) => return Err(None),
        }
    }
    f.flush().map_err(|e| Some(Error::Failed(e.to_string())))?;
    if expected.map(|e| e != got).unwrap_or(false) {
        return Err(None);
    }
    Ok(got)
}

/// Downloads `url` to `dest`. If a previous run was stopped, continues it.
pub fn download(url: &str, dest: &Path, opts: &Options, stop: Arc<AtomicBool>) -> Result<Report, Error> {
    let began = Instant::now();
    let mut restarts = 0;
    let mut carried = Report::default();
    let mut failures = 0u32;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err(Error::Stopped);
        }
        let http = Http::new(url, opts)?;
        // One byte tells everything needed: size, range support, and how to recognize the file.
        let (answer, final_url) = http.get(url, "bytes=0-0", None);
        let resp = match answer {
            Answer::Body(r) => r,
            Answer::Status(429 | 503, wait) => {
                carried.throttled += 1;
                std::thread::sleep(Duration::from_secs(wait.unwrap_or(2).min(120)));
                continue;
            }
            Answer::Status(code, _) if code >= 500 => {
                failures += 1;
                if failures > 6 {
                    return Err(Error::Http(code));
                }
                std::thread::sleep(backoff(failures));
                continue;
            }
            Answer::Status(code, _) => return Err(Error::Http(code)),
            Answer::Empty => {
                File::create(part_path(dest)).map_err(|e| Error::Failed(e.to_string()))?;
                finish(dest, 0)?;
                if let Some(w) = &opts.watch {
                    w.begin(0, "", "", &RangeSet::new());
                }
                carried.seconds = began.elapsed().as_secs_f64();
                return Ok(carried);
            }
            Answer::Transport(m) => {
                failures += 1;
                if failures > 6 {
                    return Err(Error::Failed(m));
                }
                std::thread::sleep(backoff(failures));
                continue;
            }
        };
        carried.requests += 1;
        if resp.status() != 206 {
            // No ranges: this very answer is the whole file.
            let expected = resp.header("Content-Length").and_then(|v| v.parse().ok());
            match run_single(Box::new(resp.into_reader()), expected, dest, &stop) {
                Ok(size) => {
                    finish(dest, size)?;
                    if let Some(w) = &opts.watch {
                        w.begin(size, "", "", &RangeSet::from_items(&[(0, size)]));
                    }
                    carried.size = size;
                    carried.used_ranges = false;
                    carried.peak_connections = 1;
                    carried.seconds = began.elapsed().as_secs_f64();
                    return Ok(carried);
                }
                Err(Some(e)) => return Err(e),
                Err(None) => {
                    failures += 1;
                    carried.retries += 1;
                    if failures > 6 {
                        return Err(Error::Failed("the connection keeps dropping and the server can't resume".into()));
                    }
                    std::thread::sleep(backoff(failures));
                    continue;
                }
            }
        }
        let size = resp.header("Content-Range").and_then(parse_content_range).and_then(|c| c.2);
        let Some(size) = size else {
            return Err(Error::Failed("the server gave no file size".into()));
        };
        let etag = strong_etag(&resp);
        let modified = resp.header("Last-Modified").unwrap_or("").to_string();
        // Read to the end, so this connection can serve the next request.
        let mut rest = Vec::new();
        let _ = resp.into_reader().read_to_end(&mut rest);
        let mut report = carried.clone();
        report.used_ranges = true;
        report.restarts = restarts;
        match run_ranged(http, final_url, dest, size, etag, modified, opts, &stop, report)? {
            Outcome::Done(mut r) => {
                r.seconds = began.elapsed().as_secs_f64();
                return Ok(r);
            }
            Outcome::Changed(r) => {
                restarts += 1;
                carried = r;
                carried.resumed_bytes = 0;
                let _ = fs::remove_file(part_path(dest));
                let _ = fs::remove_file(journal_path(dest));
                if restarts > opts.max_restarts {
                    return Err(Error::KeepsChanging);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range_is_read() {
        assert_eq!(parse_content_range("bytes 10-19/100"), Some((10, 20, Some(100))));
        assert_eq!(parse_content_range("bytes 0-0/*"), Some((0, 1, None)));
        assert_eq!(parse_content_range("items 1-2/3"), None);
    }

    #[test]
    fn a_journal_survives_only_when_whole() {
        let j = Journal { size: 100, etag: "\"a\"".into(), modified: "d".into(), done: RangeSet::from_items(&[(0, 10), (20, 30)]) };
        let text = j.to_text();
        assert_eq!(Journal::parse(&text), Some(j));
        assert_eq!(Journal::parse(text.trim_end_matches("end\n")), None);
        assert_eq!(Journal::parse(&text.replace("done 20 30", "done 20 300")), None);
        assert_eq!(Journal::parse("something else"), None);
    }

    fn state(total: u64) -> State {
        let now = Instant::now();
        State {
            total,
            claimed: RangeSet::new(),
            done: RangeSet::new(),
            flights: HashMap::new(),
            next_flight: 0,
            target: 4,
            workers: 1,
            not_before: now,
            last_progress: now,
            halt: Halt::Running,
            fatal: None,
            report: Report::default(),
        }
    }

    fn opts() -> Options {
        Options { min_piece: 100, max_piece: 10_000, piece_seconds: 2.0, ..Options::default() }
    }

    #[test]
    fn pieces_follow_the_speed_and_leave_no_tiny_rest() {
        let mut st = state(100_000);
        let Job::Piece { start, end, .. } = st.next_job(1000.0, &opts()) else { panic!() };
        assert_eq!((start, end), (0, 2000));
        let Job::Piece { start, end, .. } = st.next_job(1_000_000.0, &opts()) else { panic!() };
        assert_eq!((start, end), (2000, 12_000));
        let mut small = state(2050);
        let Job::Piece { end, .. } = small.next_job(1000.0, &opts()) else { panic!() };
        assert_eq!(end, 2050);
    }

    #[test]
    fn a_failed_piece_comes_back_from_where_it_stopped() {
        let mut st = state(1000);
        let Job::Piece { id, .. } = st.next_job(1000.0, &Options { min_piece: 100, max_piece: 400, piece_seconds: 1.0, ..Options::default() }) else { panic!() };
        st.claimed.add(0, 150);
        st.flights.get_mut(&id).unwrap().pos = 150;
        st.flights.remove(&id);
        let Job::Piece { start, .. } = st.next_job(1000.0, &opts()) else { panic!() };
        assert_eq!(start, 150);
    }

    #[test]
    fn a_new_connection_takes_half_of_the_biggest_piece() {
        let mut st = state(1000);
        let o = Options { min_piece: 100, max_piece: 2000, piece_seconds: 10.0, ..Options::default() };
        let Job::Piece { id, .. } = st.next_job(1000.0, &o) else { panic!() };
        st.flights.get_mut(&id).unwrap().pos = 200;
        st.claimed.add(0, 200);
        let Job::Piece { start, end, .. } = st.next_job(1000.0, &o) else { panic!("expected half of the piece") };
        assert_eq!((start, end), (600, 1000));
        assert_eq!(st.flights[&id].end, 600);
    }

    #[test]
    fn the_last_piece_is_doubled_only_if_it_is_worth_it() {
        let mut st = state(1000);
        // Too small to cut in two.
        let o = Options { min_piece: 1000, max_piece: 2000, piece_seconds: 10.0, ..Options::default() };
        let Job::Piece { id, .. } = st.next_job(1000.0, &o) else { panic!() };
        // It runs, slowly: finishing would take long enough to justify a second copy.
        {
            let f = st.flights.get_mut(&id).unwrap();
            f.began = Instant::now() - Duration::from_secs(10);
            f.pos = 10;
        }
        st.claimed.add(0, 10);
        let Job::Piece { start, end, .. } = st.next_job(1000.0, &o) else { panic!("expected a doubled piece") };
        assert_eq!((start, end), (10, 1000));
        assert_eq!(st.report.doubled_pieces, 1);
        // Not a third time.
        assert!(matches!(st.next_job(1000.0, &o), Job::Wait));
    }

    #[test]
    fn extra_connections_leave() {
        let mut st = state(1000);
        st.workers = 5;
        assert!(matches!(st.next_job(1000.0, &opts()), Job::Leave));
    }
}
