//! One transfer of any kind, and a queue that runs many of them.
use crate::destination::Destination;
use crate::engine::{download, Options, Progress, Watch};
use crate::relay::{put, relay, sha256_of_file, RelayOptions, RelayReport};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Get { url: String, file: PathBuf, sha256: Option<String> },
    Relay { url: String, target: String, work: Option<PathBuf>, keep_local: bool, sha256: Option<String> },
    Put { file: PathBuf, target: String, sha256: Option<String> },
}

/// What a finished transfer reports. Holds nothing secret.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub size: u64,
    pub seconds: f64,
    pub sha256: String,
    /// How the stored copy was checked: `local`, `parts-read-back`, `parts-signed` or `parts-not-checked`.
    pub verified: String,
    pub connections: u64,
    pub parts: u64,
    pub parts_resumed: u64,
    pub resumed_bytes: u64,
    pub retries: u64,
    pub restarts: u64,
}

impl Outcome {
    pub fn to_json(&self) -> Value {
        json!({
            "size": self.size,
            "seconds": (self.seconds * 1000.0).round() / 1000.0,
            "sha256": self.sha256,
            "verified": self.verified,
            "connections": self.connections,
            "parts": self.parts,
            "parts_resumed": self.parts_resumed,
            "resumed_bytes": self.resumed_bytes,
            "retries": self.retries,
            "restarts": self.restarts,
        })
    }
}

/// A password inside an address never reaches the screen, logs or JSON output.
pub fn safe_address(address: &str) -> String {
    match url::Url::parse(address) {
        Ok(mut u) if u.password().is_some() => {
            let _ = u.set_password(None);
            u.to_string()
        }
        _ => address.to_string(),
    }
}

impl Item {
    pub fn kind(&self) -> &'static str {
        match self {
            Item::Get { .. } => "get",
            Item::Relay { .. } => "relay",
            Item::Put { .. } => "put",
        }
    }

    pub fn source(&self) -> String {
        match self {
            Item::Get { url, .. } | Item::Relay { url, .. } => safe_address(url),
            Item::Put { file, .. } => file.display().to_string(),
        }
    }

    pub fn target(&self) -> String {
        match self {
            Item::Get { file, .. } => file.display().to_string(),
            Item::Relay { target, .. } | Item::Put { target, .. } => safe_address(target),
        }
    }

    /// The file this transfer works on, for relays the one used while passing through.
    fn local_file(&self) -> PathBuf {
        match self {
            Item::Get { file, .. } | Item::Put { file, .. } => file.clone(),
            Item::Relay { target, work, .. } => work.clone().unwrap_or_else(|| PathBuf::from(target.rsplit('/').next().unwrap_or("relay.bin"))),
        }
    }

    /// Identifies this exact transfer, so a queue can tell what it already finished.
    /// A local file that changed counts as new work.
    fn key(&self) -> String {
        let local = match self {
            Item::Put { file, .. } => fs::metadata(file).ok().map(|m| format!("{} {:?}", m.len(), m.modified().ok())).unwrap_or_default(),
            _ => String::new(),
        };
        crate::s3::sha256_hex(format!("{:?} {}", self, local).as_bytes())
    }

    /// Reads one queue line: a JSON object.
    pub fn from_json(v: &Value) -> Result<Item, String> {
        let text = |name: &str| v.get(name).and_then(Value::as_str).map(str::to_string);
        let need = |name: &str| text(name).ok_or_else(|| format!("missing \"{}\"", name));
        let sha256 = match text("sha256") {
            Some(s) if s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) => Some(s.to_lowercase()),
            Some(_) => return Err("\"sha256\" must be 64 hexadecimal characters".into()),
            None => None,
        };
        match need("op")?.as_str() {
            "get" => Ok(Item::Get { url: need("url")?, file: PathBuf::from(need("file")?), sha256 }),
            "relay" => Ok(Item::Relay {
                url: need("url")?,
                target: need("target")?,
                work: text("work").map(PathBuf::from),
                keep_local: v.get("keep_local").and_then(Value::as_bool).unwrap_or(false),
                sha256,
            }),
            "put" => Ok(Item::Put { file: PathBuf::from(need("file")?), target: need("target")?, sha256 }),
            other => Err(format!("unknown op \"{}\"", other)),
        }
    }
}

fn from_relay(r: RelayReport) -> Outcome {
    Outcome {
        size: r.stored_size,
        seconds: r.seconds,
        sha256: r.sha256,
        verified: r.verified,
        connections: r.download.peak_connections as u64,
        parts: r.parts as u64,
        parts_resumed: r.parts_resumed as u64,
        resumed_bytes: r.download.resumed_bytes,
        retries: r.download.retries + r.part_retries as u64,
        restarts: r.download.restarts as u64 + r.uploads_restarted as u64,
    }
}

/// Makes the destination a target names.
pub type Destinations<'a> = dyn Fn(&str) -> Result<Arc<dyn Destination>, String> + Sync + 'a;

/// Runs one transfer to its end.
pub fn run_item(item: &Item, destinations: &Destinations, progress: Arc<Progress>, stop: Arc<AtomicBool>) -> Result<Outcome, String> {
    let relay_opts = |keep_local, sha256: &Option<String>| RelayOptions { keep_local, expected_sha256: sha256.clone(), progress: Some(progress.clone()), ..RelayOptions::default() };
    match item {
        Item::Get { url, file, sha256 } => {
            let opts = Options { watch: Some(Arc::new(Watch::with_progress(progress.clone()))), ..Options::default() };
            let r = download(url, file, &opts, stop).map_err(|e| e.to_string())?;
            let digest = sha256_of_file(file).map_err(|e| format!("cannot read the finished file: {}", e))?;
            progress.finish(r.size);
            if let Some(expected) = sha256 {
                if !digest.eq_ignore_ascii_case(expected) {
                    // Never leave a wrong file under the name that was asked for.
                    let mut bad = file.as_os_str().to_owned();
                    bad.push(".sha256-mismatch");
                    let _ = fs::rename(file, &bad);
                    return Err(format!("the file's SHA-256 is {}, not the expected {}. It was kept as {}", digest, expected, Path::new(&bad).display()));
                }
            }
            Ok(Outcome {
                size: r.size,
                seconds: r.seconds,
                sha256: digest,
                verified: "local".into(),
                connections: r.peak_connections as u64,
                parts: 0,
                parts_resumed: 0,
                resumed_bytes: r.resumed_bytes,
                retries: r.retries,
                restarts: r.restarts as u64,
            })
        }
        Item::Relay { url, target, keep_local, sha256, .. } => {
            let dest = destinations(target)?;
            relay(url, &item.local_file(), dest, &relay_opts(*keep_local, sha256), stop).map(from_relay).map_err(|e| e.to_string())
        }
        Item::Put { file, target, sha256 } => {
            let dest = destinations(target)?;
            put(file, dest, &relay_opts(true, sha256), stop).map(from_relay).map_err(|e| e.to_string())
        }
    }
}

// ------------------------------------------------------------------ Queue

/// Reads a queue file: one JSON object per line. Blank lines and lines starting with `#` are skipped.
pub fn load_queue(text: &str) -> Result<Vec<Item>, String> {
    let mut items = vec![];
    let mut files = HashSet::new();
    let mut targets = HashSet::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let number = n + 1;
        let v: Value = serde_json::from_str(line).map_err(|e| format!("line {}: not valid JSON: {}", number, e))?;
        let item = Item::from_json(&v).map_err(|e| format!("line {}: {}", number, e))?;
        // Two transfers on one file would damage each other. A file may be sent twice, one at a time.
        if !matches!(item, Item::Put { .. }) && !files.insert(item.local_file()) {
            return Err(format!("line {}: another transfer already uses {}", number, item.local_file().display()));
        }
        if !matches!(item, Item::Get { .. }) && !targets.insert(item.target()) {
            return Err(format!("line {}: another transfer already goes to {}", number, item.target()));
        }
        items.push(item);
    }
    Ok(items)
}

#[derive(Clone, Debug)]
pub enum Event {
    Started { id: usize, item: Item, progress: Arc<Progress> },
    Skipped { id: usize, item: Item },
    Done { id: usize, item: Item, outcome: Outcome },
    Failed { id: usize, item: Item, error: String },
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Totals {
    pub done: usize,
    pub failed: usize,
    pub skipped: usize,
}

/// Runs the items, `jobs` at a time. A failure doesn't stop the others. If `state` is given,
/// finished transfers are recorded there and skipped when the queue runs again.
pub fn run_queue(items: &[Item], jobs: usize, state: Option<&Path>, destinations: &Destinations, stop: Arc<AtomicBool>, on_event: &(dyn Fn(Event) + Sync)) -> Totals {
    let finished: HashSet<String> = state.and_then(|p| fs::read_to_string(p).ok()).map(|t| t.lines().map(str::to_string).collect()).unwrap_or_default();
    let next = AtomicUsize::new(0);
    let (done, failed, skipped) = (AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0));
    let record = Mutex::new(());
    std::thread::scope(|scope| {
        for _ in 0..jobs.max(1).min(items.len().max(1)) {
            scope.spawn(|| loop {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let id = next.fetch_add(1, Ordering::SeqCst);
                let Some(item) = items.get(id) else { return };
                let key = item.key();
                let file_gone = matches!(item, Item::Get { file, .. } if !file.exists());
                if finished.contains(&key) && !file_gone {
                    skipped.fetch_add(1, Ordering::SeqCst);
                    on_event(Event::Skipped { id, item: item.clone() });
                    continue;
                }
                let progress = Arc::new(Progress::default());
                on_event(Event::Started { id, item: item.clone(), progress: progress.clone() });
                match run_item(item, destinations, progress, stop.clone()) {
                    Ok(outcome) => {
                        if let Some(path) = state {
                            let _guard = record.lock().unwrap();
                            if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
                                let _ = writeln!(f, "{}", key);
                            }
                        }
                        done.fetch_add(1, Ordering::SeqCst);
                        on_event(Event::Done { id, item: item.clone(), outcome });
                    }
                    Err(error) => {
                        failed.fetch_add(1, Ordering::SeqCst);
                        on_event(Event::Failed { id, item: item.clone(), error });
                    }
                }
            });
        }
    });
    Totals { done: done.into_inner() as usize, failed: failed.into_inner() as usize, skipped: skipped.into_inner() as usize }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_lines_are_read() {
        let q = "# my transfers\n\n{\"op\":\"get\",\"url\":\"http://a/x\",\"file\":\"x\"}\n{\"op\":\"put\",\"file\":\"x\",\"target\":\"s3://b/x\"}\n";
        let items = load_queue(q).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], Item::Get { url: "http://a/x".into(), file: "x".into(), sha256: None });
    }

    #[test]
    fn mistakes_name_their_line() {
        assert!(load_queue("\n{oops").unwrap_err().starts_with("line 2:"));
        assert!(load_queue("{\"op\":\"copy\"}").unwrap_err().contains("unknown op"));
        assert!(load_queue("{\"op\":\"get\",\"url\":\"u\"}").unwrap_err().contains("\"file\""));
        assert!(load_queue("{\"op\":\"get\",\"url\":\"u\",\"file\":\"f\",\"sha256\":\"abc\"}").unwrap_err().contains("64"));
    }

    #[test]
    fn two_transfers_cannot_share_a_file_or_target() {
        let same_file = "{\"op\":\"get\",\"url\":\"a\",\"file\":\"f\"}\n{\"op\":\"get\",\"url\":\"b\",\"file\":\"f\"}";
        assert!(load_queue(same_file).unwrap_err().starts_with("line 2:"));
        let same_target = "{\"op\":\"put\",\"file\":\"a\",\"target\":\"s3://b/k\"}\n{\"op\":\"put\",\"file\":\"b\",\"target\":\"s3://b/k\"}";
        assert!(load_queue(same_target).unwrap_err().starts_with("line 2:"));
        // One file may go to two places.
        assert!(load_queue("{\"op\":\"put\",\"file\":\"a\",\"target\":\"s3://b/1\"}\n{\"op\":\"put\",\"file\":\"a\",\"target\":\"s3://b/2\"}").is_ok());
    }

    #[test]
    fn passwords_in_addresses_are_hidden() {
        assert_eq!(safe_address("sftp://me:secret@host/f"), "sftp://me@host/f");
        assert_eq!(safe_address("https://me:secret@host/f"), "https://me@host/f");
        assert_eq!(safe_address("https://host/f"), "https://host/f");
        let item = Item::Get { url: "https://me:secret@host/f".into(), file: "f".into(), sha256: None };
        assert!(!item.source().contains("secret"));
    }
}
