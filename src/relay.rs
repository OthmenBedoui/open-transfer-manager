//! Sends a file to storage while it is still arriving.
//!
//! The download engine writes to its local file and reports each finished range. As soon as a
//! whole part is on disk it goes up, in whatever order parts complete. A slow destination only
//! lets the local file wait; a slow source only makes the uploads wait. A journal records the
//! parts that were accepted, so a stopped relay continues without sending them again. The object
//! becomes visible only at the very end, once the whole file arrived and every part was accepted.
use crate::destination::{Destination, SendError};
use crate::engine::{download, Error, Options, Report, Watch};
use crate::ranges::RangeSet;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_PART_TRIES: u32 = 8;

#[derive(Clone, Debug)]
pub struct RelayOptions {
    pub download: Options,
    pub upload_connections: usize,
    pub keep_local: bool,
}

impl Default for RelayOptions {
    fn default() -> Self {
        RelayOptions { download: Options::default(), upload_connections: 4, keep_local: false }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RelayReport {
    pub download: Report,
    pub seconds: f64,
    pub parts: u32,
    pub parts_sent: u32,
    pub parts_resumed: u32,
    pub part_retries: u32,
    pub throttled: u32,
    /// Bytes received when the first part was accepted: below the size means it really overlapped.
    pub received_at_first_part: Option<u64>,
    pub uploads_restarted: u32,
    pub stored_size: u64,
}

#[derive(Debug, PartialEq)]
pub enum RelayError {
    Stopped,
    Download(String),
    Upload(String),
}

impl std::fmt::Display for RelayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelayError::Stopped => write!(f, "stopped; the relay can continue later"),
            RelayError::Download(m) => write!(f, "download failed: {}", m),
            RelayError::Upload(m) => write!(f, "upload failed: {}", m),
        }
    }
}

fn journal_path(work: &Path) -> PathBuf {
    let mut s = work.as_os_str().to_owned();
    s.push(".upload.journal");
    PathBuf::from(s)
}

fn part_file(work: &Path) -> PathBuf {
    let mut s = work.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

#[derive(Clone, Debug, Default, PartialEq)]
struct UploadJournal {
    size: u64,
    etag: String,
    modified: String,
    target: String,
    upload_id: String,
    part_size: u64,
    downloaded: bool,
    parts: Vec<(u32, String)>,
}

impl UploadJournal {
    fn to_text(&self) -> String {
        let mut t = format!(
            "otm-upload 1\nsize {}\netag {}\nmodified {}\ntarget {}\nupload {}\npart-size {}\ndownloaded {}\n",
            self.size, self.etag, self.modified, self.target, self.upload_id, self.part_size, self.downloaded
        );
        for (n, tag) in &self.parts {
            t.push_str(&format!("part {} {}\n", n, tag));
        }
        t.push_str("end\n");
        t
    }

    /// None for anything damaged: a cut-off journal is never trusted.
    fn parse(text: &str) -> Option<UploadJournal> {
        let mut lines = text.lines();
        if lines.next()? != "otm-upload 1" {
            return None;
        }
        let mut field = |name: &str| lines.next()?.strip_prefix(name)?.strip_prefix(' ').map(str::to_string);
        let size = field("size")?.parse().ok()?;
        let etag = field("etag")?;
        let modified = field("modified")?;
        let target = field("target")?;
        let upload_id = field("upload")?;
        let part_size = field("part-size")?.parse().ok()?;
        let downloaded = field("downloaded")? == "true";
        let mut j = UploadJournal { size, etag, modified, target, upload_id, part_size, downloaded, parts: vec![] };
        for line in lines {
            if line == "end" {
                return Some(j);
            }
            let (n, tag) = line.strip_prefix("part ")?.split_once(' ')?;
            j.parts.push((n.parse().ok()?, tag.to_string()));
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

/// The local file is `<work>.part` until the download ends, then `<work>`.
fn read_part(work: &Path, start: u64, len: usize) -> std::io::Result<Vec<u8>> {
    let mut f = match File::open(part_file(work)) {
        Ok(f) => f,
        Err(_) => File::open(work)?,
    };
    f.seek(SeekFrom::Start(start))?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

struct Done {
    upload_round: u32,
    number: u32,
    result: Result<String, SendError>,
}

fn pause(stop: &AtomicBool, wait: Duration) {
    let end = Instant::now() + wait;
    while Instant::now() < end && !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Sends `url` to `dest`, using `work` as the local file while it passes through.
pub fn relay(url: &str, work: &Path, dest: Arc<dyn Destination>, opts: &RelayOptions, stop: Arc<AtomicBool>) -> Result<RelayReport, RelayError> {
    run(Source::Url(url), work, dest, opts, stop)
}

/// Sends a file already on this computer. The file stays where it is.
pub fn put(file: &Path, dest: Arc<dyn Destination>, opts: &RelayOptions, stop: Arc<AtomicBool>) -> Result<RelayReport, RelayError> {
    let opts = RelayOptions { keep_local: true, ..opts.clone() };
    run(Source::Local, file, dest, &opts, stop)
}

enum Source<'a> {
    Url(&'a str),
    Local,
}

/// What identifies this version of a local file: a resume must not mix it with another.
fn local_version(file: &Path) -> Result<(u64, String), RelayError> {
    let m = fs::metadata(file).map_err(|e| RelayError::Download(format!("cannot read {}: {}", file.display(), e)))?;
    let when = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
    Ok((m.len(), format!("local-{}", when)))
}

fn run(source: Source, work: &Path, client: Arc<dyn Destination>, opts: &RelayOptions, stop: Arc<AtomicBool>) -> Result<RelayReport, RelayError> {
    let began = Instant::now();
    let journal_file = journal_path(work);
    let target = client.describe();
    let saved = fs::read_to_string(&journal_file).ok().and_then(|t| UploadJournal::parse(&t)).filter(|j| j.target == target && client.still_valid(&j.upload_id));

    let watch = Arc::new(Watch::default());
    let mut dl_opts = opts.download.clone();
    dl_opts.watch = Some(watch.clone());

    // A file fully received in an earlier run needs no download: its journal says which version it is.
    let already_here = saved.as_ref().map(|j| j.downloaded && fs::metadata(work).map(|m| m.len() == j.size).unwrap_or(false)).unwrap_or(false);
    let (result_tx, result_rx) = channel();
    if let Source::Local = source {
        let (size, version) = local_version(work)?;
        watch.begin(size, "", &version, &RangeSet::from_items(&[(0, size)]));
        let _ = result_tx.send(Ok(Report { size, ..Report::default() }));
    } else if already_here {
        let j = saved.as_ref().unwrap();
        watch.begin(j.size, &j.etag, &j.modified, &RangeSet::from_items(&[(0, j.size)]));
        let _ = result_tx.send(Ok(Report { size: j.size, ..Report::default() }));
    } else {
        let Source::Url(url) = source else { unreachable!() };
        let (u, w, o, s) = (url.to_string(), work.to_path_buf(), dl_opts, stop.clone());
        std::thread::spawn(move || {
            let _ = result_tx.send(download(&u, &w, &o, s));
        });
    }

    let mut report = RelayReport::default();
    let mut journal: Option<UploadJournal> = saved;
    let mut seen_round = 0u32;
    let mut upload_round = 0u32;
    let mut accepted: HashMap<u32, String> = HashMap::new();
    let mut in_flight: HashMap<u32, Instant> = HashMap::new();
    let mut tries: HashMap<u32, u32> = HashMap::new();
    let mut wait_until: HashMap<u32, Instant> = HashMap::new();
    let mut not_before = Instant::now();
    let mut download_report: Option<Report> = None;
    let (done_tx, done_rx) = channel::<Done>();

    let fail = |stop: &AtomicBool, e: RelayError| {
        stop.store(true, Ordering::SeqCst);
        Err(e)
    };

    loop {
        // What the download is doing.
        if download_report.is_none() {
            if let Ok(r) = result_rx.try_recv() {
                match r {
                    Ok(rep) => {
                        download_report = Some(rep);
                        if let Some(j) = journal.as_mut() {
                            j.downloaded = true;
                            let _ = j.save(&journal_file);
                        }
                    }
                    Err(Error::Stopped) => return Err(RelayError::Stopped),
                    Err(e) => return fail(&stop, RelayError::Download(e.to_string())),
                }
            }
        }
        if stop.load(Ordering::Relaxed) {
            return Err(RelayError::Stopped);
        }

        let snapshot = watch.snapshot();
        if let Some(view) = &snapshot {
            // A new round: the first view of this file, or the file changed on the server.
            if view.round != seen_round {
                let first = seen_round == 0;
                seen_round = view.round;
                upload_round += 1;
                in_flight.clear();
                tries.clear();
                wait_until.clear();
                accepted.clear();
                let usable = journal.as_ref().filter(|j| j.size == view.size && j.etag == view.etag && j.modified == view.modified && (!j.etag.is_empty() || !j.modified.is_empty()) && j.part_size == client.part_size(view.size));
                if let (true, Some(j)) = (first, usable) {
                    report.parts_resumed = j.parts.len() as u32;
                    accepted.extend(j.parts.iter().cloned());
                } else {
                    if let Some(old) = journal.take() {
                        // Parts of another version must never end up in the object.
                        client.cancel(&old.upload_id);
                        if !first {
                            report.uploads_restarted += 1;
                        }
                    }
                    if view.size > 0 {
                        let id = loop {
                            match client.start(view.size) {
                                Ok(id) => break id,
                                Err(SendError::Fatal(m)) => return fail(&stop, RelayError::Upload(m)),
                                Err(SendError::Retry(m, wait)) => {
                                    let t = tries.entry(0).or_insert(0);
                                    *t += 1;
                                    if *t > MAX_PART_TRIES {
                                        return fail(&stop, RelayError::Upload(m));
                                    }
                                    let secs = wait.unwrap_or(1u64 << (*t).min(5));
                                    pause(&stop, Duration::from_secs(secs));
                                    if stop.load(Ordering::Relaxed) {
                                        return Err(RelayError::Stopped);
                                    }
                                }
                            }
                        };
                        tries.clear();
                        let j = UploadJournal {
                            size: view.size,
                            etag: view.etag.clone(),
                            modified: view.modified.clone(),
                            target: target.clone(),
                            upload_id: id,
                            part_size: client.part_size(view.size),
                            downloaded: false,
                            parts: vec![],
                        };
                        let _ = j.save(&journal_file);
                        journal = Some(j);
                    }
                }
            }
        }

        // Answers from parts sent.
        while let Ok(d) = done_rx.try_recv() {
            if d.upload_round != upload_round {
                continue; // From a version of the file that no longer counts.
            }
            in_flight.remove(&d.number);
            match d.result {
                Ok(tag) => {
                    if report.received_at_first_part.is_none() {
                        report.received_at_first_part = snapshot.as_ref().map(|v| v.done.len());
                    }
                    accepted.insert(d.number, tag.clone());
                    report.parts_sent += 1;
                    if let Some(j) = journal.as_mut() {
                        j.parts.push((d.number, tag));
                        let _ = j.save(&journal_file);
                    }
                }
                Err(SendError::Fatal(m)) => return fail(&stop, RelayError::Upload(m)),
                Err(SendError::Retry(m, wait)) => {
                    report.part_retries += 1;
                    if wait.is_some() {
                        report.throttled += 1;
                    }
                    let t = tries.entry(d.number).or_insert(0);
                    *t += 1;
                    if *t >= MAX_PART_TRIES {
                        return fail(&stop, RelayError::Upload(format!("part {} kept failing: {}", d.number, m)));
                    }
                    let backoff = Duration::from_millis(250u64 << (*t).min(6));
                    wait_until.insert(d.number, Instant::now() + backoff);
                    if let Some(secs) = wait {
                        // The storage asked everyone to wait, not only this part.
                        not_before = not_before.max(Instant::now() + Duration::from_secs(secs.min(120)));
                    }
                }
            }
        }

        // Parts that are complete on disk and not sent yet.
        if let (Some(view), Some(j)) = (&snapshot, &journal) {
            let parts_total = view.size.div_ceil(j.part_size) as u32;
            report.parts = parts_total;
            if Instant::now() >= not_before {
                for number in 1..=parts_total {
                    if in_flight.len() >= opts.upload_connections.max(1) {
                        break;
                    }
                    if accepted.contains_key(&number) || in_flight.contains_key(&number) {
                        continue;
                    }
                    if wait_until.get(&number).map(|t| Instant::now() < *t).unwrap_or(false) {
                        continue;
                    }
                    let start = (number as u64 - 1) * j.part_size;
                    let end = (start + j.part_size).min(view.size);
                    if !view.done.covers(start, end) {
                        continue;
                    }
                    in_flight.insert(number, Instant::now());
                    let (c, id) = (client.clone(), j.upload_id.clone());
                    let (w, tx, round) = (work.to_path_buf(), done_tx.clone(), upload_round);
                    std::thread::spawn(move || {
                        let result = match read_part(&w, start, (end - start) as usize) {
                            Ok(data) => c.send_part(&id, number, start, &data),
                            Err(e) => Err(SendError::Retry(e.to_string(), None)),
                        };
                        let _ = tx.send(Done { upload_round: round, number, result });
                    });
                }
            }

            // Everything arrived and everything was accepted: only now does the object appear.
            if download_report.is_some() && in_flight.is_empty() && accepted.len() as u32 == parts_total {
                let mut parts: Vec<(u32, String)> = accepted.iter().map(|(n, t)| (*n, t.clone())).collect();
                parts.sort_by_key(|p| p.0);
                let mut attempt = 0;
                loop {
                    match client.finish(&j.upload_id, &parts, view.size) {
                        Ok(()) => break,
                        Err(SendError::Fatal(m)) => return fail(&stop, RelayError::Upload(m)),
                        Err(SendError::Retry(m, wait)) => {
                            attempt += 1;
                            if attempt >= MAX_PART_TRIES {
                                return fail(&stop, RelayError::Upload(m));
                            }
                            pause(&stop, Duration::from_secs(wait.unwrap_or(1u64 << attempt.min(5))));
                        }
                    }
                }
                return finish(client.as_ref(), work, view.size, &journal_file, opts, report, download_report, began);
            }
        }
        // An empty file has no parts to send.
        if let (Some(view), true, None) = (&snapshot, download_report.is_some(), &journal) {
            if view.size == 0 {
                client.store_empty().map_err(|e| RelayError::Upload(e.to_string()))?;
                return finish(client.as_ref(), work, 0, &journal_file, opts, report, download_report, began);
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[allow(clippy::too_many_arguments)]
fn finish(client: &dyn Destination, work: &Path, size: u64, journal_file: &Path, opts: &RelayOptions, mut report: RelayReport, download_report: Option<Report>, began: Instant) -> Result<RelayReport, RelayError> {
    // What the destination now holds, checked rather than assumed.
    let stored = client.stored_size().map_err(|e| RelayError::Upload(format!("cannot check the stored file: {}", e)))?;
    if stored != size {
        return Err(RelayError::Upload(format!("the storage holds {} bytes instead of {}", stored, size)));
    }
    report.stored_size = stored;
    let _ = fs::remove_file(journal_file);
    if !opts.keep_local {
        let _ = fs::remove_file(work);
    }
    report.download = download_report.unwrap_or_default();
    report.seconds = began.elapsed().as_secs_f64();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_journal_survives_only_when_whole() {
        let j = UploadJournal {
            size: 100,
            etag: "\"a b\"".into(),
            modified: "Mon, 01 Jan".into(),
            target: "s3://b/dir/my file.mp4".into(),
            upload_id: "id1".into(),
            part_size: 10,
            downloaded: true,
            parts: vec![(1, "\"x\"".into()), (3, "\"y\"".into())],
        };
        let text = j.to_text();
        assert_eq!(UploadJournal::parse(&text), Some(j));
        assert_eq!(UploadJournal::parse(text.trim_end_matches("end\n")), None);
        assert_eq!(UploadJournal::parse("otm-upload 2\n"), None);
    }
}
