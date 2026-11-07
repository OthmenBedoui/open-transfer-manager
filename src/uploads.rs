//! Send queue: one job per file × destination, a few running at once, each in its own
//! rclone process. Also owns the list of connectors (saved in `connectors.json`).

use crate::connectors::Connector;
use crate::manager::lock;
use crate::model::now;
use crate::rclone::{self, Remote};
use crate::{paths, secrets};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};

/// Uploads running at the same time (each destination keeps its own connection).
const AT_ONCE: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JobStatus {
    Waiting,
    Running,
    Done,
    Failed,
    Stopped,
}

#[derive(Clone, Debug)]
pub struct Job {
    pub id: u64,
    pub file: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub dest: u64,
    pub dest_name: String,
    pub dest_mono: &'static str,
    pub status: JobStatus,
    pub sent: u64,
    pub total: u64,
    pub speed: f64,
    pub eta: Option<f64>,
    pub error: String,
    pub want_link: bool,
    pub link: Option<String>,
    /// Transfer between destinations: the connector files come from, its name, and the
    /// path on it (`None` for files on this computer).
    pub source: Option<u64>,
    pub source_name: String,
    pub source_path: String,
    pub finished_at: Option<f64>,
    stop: Arc<AtomicBool>,
}

impl Job {
    pub fn progress(&self) -> f32 {
        match self.status {
            JobStatus::Done => 1.0,
            _ if self.total > 0 => (self.sent as f64 / self.total as f64).clamp(0.0, 1.0) as f32,
            _ => 0.0,
        }
    }
    pub fn finished(&self) -> bool {
        matches!(
            self.status,
            JobStatus::Done | JobStatus::Failed | JobStatus::Stopped
        )
    }
}

type JobRef = Arc<Mutex<Job>>;

pub enum UploadEvent {
    Done {
        name: String,
        dest: String,
    },
    Failed {
        name: String,
        dest: String,
        error: String,
    },
}

struct Inner {
    jobs: Mutex<Vec<JobRef>>,
    connectors: Mutex<Vec<Connector>>,
    next_job: AtomicU64,
    persist: bool,
    events: Mutex<Vec<UploadEvent>>,
    frozen: AtomicBool,
    tick_lock: Mutex<()>,
    repaint: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

#[derive(Clone)]
pub struct Uploads {
    inner: Arc<Inner>,
}

fn connectors_path() -> PathBuf {
    paths::data_dir().join("connectors.json")
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct SavedConnectors {
    connectors: Vec<Connector>,
}

impl Uploads {
    pub fn new(persist: bool) -> Self {
        if persist {
            rclone::clean_run_dir();
        }
        let connectors = if persist {
            std::fs::read_to_string(connectors_path())
                .ok()
                .and_then(|t| serde_json::from_str::<SavedConnectors>(&t).ok())
                .map(|s| s.connectors)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Uploads {
            inner: Arc::new(Inner {
                jobs: Mutex::new(Vec::new()),
                connectors: Mutex::new(connectors),
                next_job: AtomicU64::new(1),
                persist,
                events: Mutex::new(Vec::new()),
                frozen: AtomicBool::new(false),
                tick_lock: Mutex::new(()),
                repaint: Mutex::new(None),
            }),
        }
    }

    pub fn set_repaint(&self, f: impl Fn() + Send + Sync + 'static) {
        *lock(&self.inner.repaint) = Some(Box::new(f));
    }

    fn repaint(&self) {
        if let Some(f) = lock(&self.inner.repaint).as_ref() {
            f();
        }
    }

    pub fn take_events(&self) -> Vec<UploadEvent> {
        std::mem::take(&mut *lock(&self.inner.events))
    }

    // ---------------------------------------------------------------- connectors

    pub fn connectors(&self) -> Vec<Connector> {
        lock(&self.inner.connectors).clone()
    }

    pub fn connector(&self, id: u64) -> Option<Connector> {
        lock(&self.inner.connectors)
            .iter()
            .find(|c| c.id == id)
            .cloned()
    }

    pub fn find_connector(&self, name: &str) -> Option<Connector> {
        let all = self.connectors();
        let n = name.trim().to_lowercase();
        all.iter()
            .find(|c| c.display_name().to_lowercase() == n)
            .or_else(|| all.iter().find(|c| c.id.to_string() == n))
            .cloned()
    }

    pub fn has_secret(&self, c: &Connector) -> bool {
        c.id != 0 && secrets::get(&c.secret_key()).is_some()
    }

    /// Add or update a connection. `secret`: `None` keeps the saved one, `Some("")`
    /// removes it, anything else replaces it. Returns the connection's id.
    pub fn save_connector(&self, mut c: Connector, secret: Option<&str>) -> Result<u64, String> {
        let has_secret = match secret {
            Some(s) => !s.is_empty(),
            None => self.has_secret(&c),
        };
        c.validate(has_secret)?;
        {
            let mut all = lock(&self.inner.connectors);
            if c.id == 0 {
                c.id = all.iter().map(|x| x.id).max().unwrap_or(0) + 1;
            }
            if c.name.trim().is_empty() {
                c.name = c.address();
            }
            match all.iter_mut().find(|x| x.id == c.id) {
                Some(x) => *x = c.clone(),
                None => all.push(c.clone()),
            }
        }
        match secret {
            Some("") => secrets::delete(&c.secret_key()),
            Some(s) => secrets::set(&c.secret_key(), s)?,
            None => {}
        }
        self.save_connectors();
        Ok(c.id)
    }

    pub fn remove_connector(&self, id: u64) {
        let removed = {
            let mut all = lock(&self.inner.connectors);
            let before = all.len();
            all.retain(|c| c.id != id);
            before != all.len()
        };
        if removed {
            secrets::delete(&format!("connector-{id}"));
            self.save_connectors();
        }
    }

    fn save_connectors(&self) {
        if !self.inner.persist {
            return;
        }
        let saved = SavedConnectors {
            connectors: self.connectors(),
        };
        if let Ok(text) = serde_json::to_string_pretty(&saved) {
            let path = connectors_path();
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    /// Check a connection in the background (it may take a few seconds).
    pub fn test(
        c: Connector,
        typed_secret: Option<String>,
    ) -> mpsc::Receiver<Result<String, String>> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let r = Remote::new(&c, typed_secret.as_deref()).and_then(|r| r.test(&c));
            let _ = tx.send(r);
        });
        rx
    }

    // ---------------------------------------------------------------- jobs

    pub fn jobs(&self) -> Vec<Job> {
        lock(&self.inner.jobs)
            .iter()
            .map(|j| lock(j).clone())
            .collect()
    }

    /// Queue every file for every destination. Returns how many jobs were added.
    pub fn send(&self, files: &[PathBuf], dests: &[u64], want_link: bool) -> usize {
        let mut added = 0;
        for d in dests {
            let Some(c) = self.connector(*d) else {
                continue;
            };
            for f in files {
                let name = f
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if name.is_empty() {
                    continue;
                }
                let total = if f.is_file() {
                    std::fs::metadata(f).map(|m| m.len()).unwrap_or(0)
                } else {
                    0
                };
                let job = Job {
                    id: self.inner.next_job.fetch_add(1, Ordering::SeqCst),
                    file: f.clone(),
                    name,
                    is_dir: f.is_dir(),
                    dest: c.id,
                    dest_name: c.display_name(),
                    dest_mono: c.kind.mono(),
                    status: JobStatus::Waiting,
                    sent: 0,
                    total,
                    speed: 0.0,
                    eta: None,
                    error: String::new(),
                    want_link,
                    link: None,
                    finished_at: None,
                    stop: Arc::new(AtomicBool::new(false)),
                    source: None,
                    source_name: String::new(),
                    source_path: String::new(),
                };
                lock(&self.inner.jobs).push(Arc::new(Mutex::new(job)));
                added += 1;
            }
        }
        self.tick();
        added
    }

    /// Copy items of one destination (`src`, paths relative to its root) to others.
    /// Returns how many jobs were added.
    pub fn send_remote(
        &self,
        src: u64,
        items: &[(String, bool, Option<u64>)],
        dests: &[u64],
        want_link: bool,
    ) -> usize {
        let Some(from) = self.connector(src) else {
            return 0;
        };
        let mut added = 0;
        for d in dests.iter().filter(|d| **d != src) {
            let Some(c) = self.connector(*d) else {
                continue;
            };
            for (path, is_dir, size) in items {
                let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
                if name.is_empty() {
                    continue;
                }
                let job = Job {
                    id: self.inner.next_job.fetch_add(1, Ordering::SeqCst),
                    file: PathBuf::from(name),
                    name: name.to_string(),
                    is_dir: *is_dir,
                    dest: c.id,
                    dest_name: c.display_name(),
                    dest_mono: c.kind.mono(),
                    status: JobStatus::Waiting,
                    sent: 0,
                    total: size.unwrap_or(0),
                    speed: 0.0,
                    eta: None,
                    error: String::new(),
                    want_link,
                    link: None,
                    finished_at: None,
                    stop: Arc::new(AtomicBool::new(false)),
                    source: Some(from.id),
                    source_name: from.display_name(),
                    source_path: path.clone(),
                };
                lock(&self.inner.jobs).push(Arc::new(Mutex::new(job)));
                added += 1;
            }
        }
        self.tick();
        added
    }

    /// List a folder of a destination in the background (Transfer page).
    pub fn browse(
        c: Connector,
        path: String,
    ) -> mpsc::Receiver<Result<Vec<rclone::Entry>, String>> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(Remote::new(&c, None).and_then(|r| r.list(&path)));
        });
        rx
    }

    fn find(&self, id: u64) -> Option<JobRef> {
        lock(&self.inner.jobs)
            .iter()
            .find(|j| lock(j).id == id)
            .cloned()
    }

    pub fn stop(&self, id: u64) {
        if let Some(j) = self.find(id) {
            let mut j = lock(&j);
            j.stop.store(true, Ordering::SeqCst);
            if j.status == JobStatus::Waiting {
                j.status = JobStatus::Stopped;
                j.finished_at = Some(now());
            }
        }
    }

    pub fn retry(&self, id: u64) {
        if let Some(j) = self.find(id) {
            let mut j = lock(&j);
            if j.finished() && j.status != JobStatus::Done {
                j.status = JobStatus::Waiting;
                j.stop = Arc::new(AtomicBool::new(false));
                j.error.clear();
                j.sent = 0;
                j.finished_at = None;
            }
        }
        self.tick();
    }

    pub fn clear_finished(&self) {
        lock(&self.inner.jobs).retain(|j| !lock(j).finished());
    }

    pub fn active_count(&self) -> usize {
        lock(&self.inner.jobs)
            .iter()
            .filter(|j| matches!(lock(j).status, JobStatus::Running | JobStatus::Waiting))
            .count()
    }

    pub fn total_rate(&self) -> f64 {
        lock(&self.inner.jobs)
            .iter()
            .map(|j| {
                let j = lock(j);
                if j.status == JobStatus::Running {
                    j.speed
                } else {
                    0.0
                }
            })
            .sum()
    }

    pub fn stop_all(&self) {
        for j in lock(&self.inner.jobs).iter() {
            lock(j).stop.store(true, Ordering::SeqCst);
        }
    }

    /// Start waiting jobs while fewer than `AT_ONCE` run.
    pub fn tick(&self) {
        if self.inner.frozen.load(Ordering::SeqCst) {
            return;
        }
        // the UI and finishing jobs both call this: one at a time, or a job could start twice
        let _guard = lock(&self.inner.tick_lock);
        let jobs = lock(&self.inner.jobs).clone();
        let mut running = 0;
        // Drives signed in through the browser upload one file at a time: some (Box)
        // replace the sign-in token on every renewal, so two uploads must not race for it.
        let mut busy_drives = std::collections::HashSet::new();
        for j in &jobs {
            let j = lock(j);
            if j.status == JobStatus::Running {
                running += 1;
                busy_drives.insert(j.dest);
            }
        }
        for j in jobs {
            if running >= AT_ONCE {
                break;
            }
            let dest = {
                let g = lock(&j);
                if g.status != JobStatus::Waiting {
                    continue;
                }
                g.dest
            };
            let one_at_a_time = self
                .connector(dest)
                .is_some_and(|c| c.kind.uses_browser_sign_in());
            if one_at_a_time && !busy_drives.insert(dest) {
                continue;
            }
            lock(&j).status = JobStatus::Running;
            running += 1;
            let me = self.clone();
            std::thread::spawn(move || me.run(j));
        }
    }

    fn run(&self, job: JobRef) {
        let (file, name, dest, stop, want_link, source, source_path, is_dir) = {
            let j = lock(&job);
            (
                j.file.clone(),
                j.name.clone(),
                j.dest,
                j.stop.clone(),
                j.want_link,
                j.source,
                j.source_path.clone(),
                j.is_dir,
            )
        };
        let result = (|| {
            let c = self
                .connector(dest)
                .ok_or("This destination was removed.")?;
            let repaint = self.clone();
            let (remote, target) = match source {
                Some(src) => {
                    let from = self.connector(src).ok_or("The source was removed.")?;
                    (Remote::pair(&from, &c)?, c.path_on("dst", &name))
                }
                None => {
                    if !file.exists() {
                        return Err(format!("File not found: {}", file.display()));
                    }
                    (Remote::new(&c, None)?, c.remote_path(&name))
                }
            };
            let from: std::ffi::OsString = match source {
                Some(_) => format!("src:{source_path}").into(),
                None => file.clone().into_os_string(),
            };
            let is_dir = if source.is_some() {
                is_dir
            } else {
                file.is_dir()
            };
            remote.copy(&from, is_dir, &target, &stop, |s| {
                {
                    let mut j = lock(&job);
                    j.sent = s.bytes;
                    if s.total > 0 {
                        j.total = s.total;
                    }
                    j.speed = s.speed;
                    j.eta = s.eta;
                }
                repaint.repaint();
            })?;
            Ok(if want_link {
                remote.link(&target)
            } else {
                None
            })
        })();
        let event = {
            let mut j = lock(&job);
            j.speed = 0.0;
            j.eta = None;
            j.finished_at = Some(now());
            match result {
                Ok(link) => {
                    j.status = JobStatus::Done;
                    j.sent = j.total;
                    j.link = link;
                    UploadEvent::Done {
                        name: j.name.clone(),
                        dest: j.dest_name.clone(),
                    }
                }
                Err(e) if stop.load(Ordering::SeqCst) => {
                    j.status = JobStatus::Stopped;
                    j.error = e;
                    self.tick();
                    self.repaint();
                    return;
                }
                Err(e) => {
                    j.status = JobStatus::Failed;
                    j.error = e.clone();
                    UploadEvent::Failed {
                        name: j.name.clone(),
                        dest: j.dest_name.clone(),
                        error: e,
                    }
                }
            }
        };
        lock(&self.inner.events).push(event);
        self.tick();
        self.repaint();
    }
}

impl Uploads {
    /// Sample destinations and transfers for screenshots (never saved).
    pub fn demo(&self) {
        use crate::connectors::{Kind, SftpAuth};
        let mut ids = Vec::new();
        for (name, kind, f) in [
            (
                "prod-server",
                Kind::Sftp,
                "deploy@203.0.113.10:/var/www/releases",
            ),
            ("Backblaze B2", Kind::S3, "otm-releases/builds"),
            ("Nextcloud", Kind::Webdav, "Uploads"),
            ("NAS", Kind::Ftp, "/public"),
            ("Google Drive", Kind::Gdrive, "OTM"),
            ("OneDrive", Kind::Onedrive, "Backups"),
            ("Dropbox", Kind::Dropbox, "Shared"),
        ] {
            let mut c = Connector::new(kind);
            c.name = name.into();
            c.folder = f.into();
            c.host = "203.0.113.10".into();
            c.user = "deploy".into();
            c.auth = SftpAuth::Agent;
            c.endpoint = "https://s3.eu-central-003.backblazeb2.com".into();
            c.url = "https://cloud.example.com/remote.php/dav/files/me".into();
            c.id = ids.len() as u64 + 1;
            if kind == Kind::Sftp {
                c.folder = "/var/www/releases".into();
            }
            lock(&self.inner.connectors).push(c.clone());
            ids.push(c);
        }
        let rows = [
            (
                "demo-video-final.mp4",
                0,
                JobStatus::Running,
                0.62,
                8.1e6,
                "",
            ),
            (
                "demo-video-final.mp4",
                1,
                JobStatus::Running,
                0.41,
                6.5e6,
                "",
            ),
            (
                "demo-video-final.mp4",
                2,
                JobStatus::Failed,
                0.18,
                0.0,
                "Not enough space on the destination.",
            ),
            (
                "open-transfer-manager-0.2.7.zip",
                1,
                JobStatus::Done,
                1.0,
                0.0,
                "",
            ),
            (
                "open-transfer-manager-0.2.7.zip",
                0,
                JobStatus::Done,
                1.0,
                0.0,
                "",
            ),
            ("Releases", 5, JobStatus::Running, 0.34, 11.2e6, ""),
        ];
        for (name, d, status, frac, speed, err) in rows {
            let c = &ids[d];
            let total: u64 = if name.ends_with("mp4") {
                248_000_000
            } else {
                12_400_000
            };
            let job = Job {
                id: self.inner.next_job.fetch_add(1, Ordering::SeqCst),
                file: PathBuf::from(name),
                name: name.into(),
                is_dir: false,
                dest: c.id,
                dest_name: c.display_name(),
                dest_mono: c.kind.mono(),
                status,
                sent: (total as f64 * frac) as u64,
                total,
                speed,
                eta: (speed > 0.0).then(|| total as f64 * (1.0 - frac) / speed),
                error: err.into(),
                want_link: true,
                link: (status == JobStatus::Done && c.kind == Kind::S3).then(|| {
                    "https://s3.example.com/otm-releases/builds/open-transfer-manager-0.2.7.zip"
                        .into()
                }),
                finished_at: None,
                stop: Arc::new(AtomicBool::new(false)),
                source: (d == 5).then_some(5),
                source_name: if d == 5 {
                    "Google Drive".into()
                } else {
                    String::new()
                },
                source_path: String::new(),
            };
            lock(&self.inner.jobs).push(Arc::new(Mutex::new(job)));
        }
    }

    /// Jobs are only started by `tick`; demo jobs must never run.
    pub fn freeze(&self) {
        self.inner.frozen.store(true, Ordering::SeqCst);
    }
}

/// Size of a file, or of everything inside a folder (for the "Send" file list).
pub fn size_of(path: &Path) -> u64 {
    if path.is_file() {
        return std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    }
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];
    let mut seen = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            seen += 1;
            if seen > 50_000 {
                return total; // enough for a display estimate
            }
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => total += e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => {}
            }
        }
    }
    total
}

/// rclone must be installed before anything can be sent.
pub fn engine_ready() -> bool {
    rclone::path().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::{Kind, SftpAuth};

    #[test]
    fn connectors_keep_ids_and_secrets_follow_them() {
        let up = Uploads::new(false);
        let mut c = Connector::new(Kind::Sftp);
        c.host = "example.com".into();
        c.user = "me".into();
        c.auth = SftpAuth::Password;
        assert!(
            up.save_connector(c.clone(), None).is_err(),
            "password needed"
        );
        let id = up.save_connector(c.clone(), Some("pw")).unwrap();
        assert_eq!(id, 1);
        let saved = up.connector(id).unwrap();
        assert_eq!(
            saved.name, "me@example.com:~",
            "name defaults to the address"
        );
        assert!(up.has_secret(&saved));
        // editing without retyping the password keeps it
        let mut edited = saved.clone();
        edited.folder = "/srv".into();
        up.save_connector(edited, None).unwrap();
        assert_eq!(up.connectors().len(), 1);
        assert_eq!(
            up.find_connector("ME@example.com:~").unwrap().folder,
            "/srv"
        );
        up.remove_connector(id);
        assert!(up.connectors().is_empty());
        assert!(
            secrets::get("connector-1").is_none(),
            "secret removed with it"
        );
    }

    #[test]
    fn jobs_for_each_file_and_destination() {
        let up = Uploads::new(false);
        let mut c = Connector::new(Kind::Webdav);
        c.url = "https://dav.invalid/remote".into();
        let a = up.save_connector(c.clone(), None).unwrap();
        let b = up.save_connector(c, None).unwrap();
        // files that don't exist fail fast without running anything
        let files = [
            PathBuf::from("/nope/one.zip"),
            PathBuf::from("/nope/two.mp4"),
        ];
        assert_eq!(up.send(&files, &[a, b, 99], false), 4);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while up.active_count() > 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let jobs = up.jobs();
        assert!(jobs.iter().all(|j| j.status == JobStatus::Failed));
        assert!(jobs[0].error.starts_with("File not found"));
        assert_eq!(up.take_events().len(), 4);
        up.retry(jobs[0].id);
        up.clear_finished();
        assert!(up.jobs().len() <= 1);
    }

    /// Real transfers against local test servers (`rclone serve sftp|webdav|ftp|s3`).
    /// Run with: OTM_LIVE_DIR=<served root> cargo test live_servers -- --ignored
    #[test]
    #[ignore]
    fn live_servers() {
        let root = PathBuf::from(std::env::var("OTM_LIVE_DIR").expect("OTM_LIVE_DIR"));
        // written before the server first lists the folder (rclone serve caches listings)
        std::fs::create_dir_all(root.join("sftp/in/box")).unwrap();
        std::fs::write(root.join("sftp/in/box/only-remote.txt"), "from sftp").unwrap();
        let up = Uploads::new(false);
        let pw = "s3cret pw";
        let mut sftp = Connector::new(Kind::Sftp);
        (sftp.host, sftp.port, sftp.user, sftp.auth) = (
            "127.0.0.1".into(),
            Some(2022),
            "tester".into(),
            SftpAuth::Password,
        );
        sftp.folder = "in/box".into();
        let mut ftp = Connector::new(Kind::Ftp);
        (ftp.host, ftp.port, ftp.user) = ("127.0.0.1".into(), Some(2121), "tester".into());
        let mut dav = Connector::new(Kind::Webdav);
        (dav.url, dav.user) = ("http://127.0.0.1:8081".into(), "tester".into());
        dav.folder = "shared".into();
        let mut s3 = Connector::new(Kind::S3);
        (s3.endpoint, s3.access_key, s3.folder) = (
            "http://127.0.0.1:8082".into(),
            "AKTEST".into(),
            "bucket1/builds".into(),
        );
        let ids: Vec<u64> = [(sftp, pw), (ftp, pw), (dav, pw), (s3, "SKTEST123")]
            .into_iter()
            .map(|(c, s)| {
                let t = Uploads::test(c.clone(), Some(s.into())).recv().unwrap();
                println!("{}: {t:?}", c.kind.label());
                assert!(t.is_ok(), "{t:?}");
                up.save_connector(c, Some(s)).unwrap()
            })
            .collect();
        // wrong password: a readable error
        let mut bad = up.connector(ids[0]).unwrap();
        bad.id = 0;
        let t = Uploads::test(bad, Some("nope".into())).recv().unwrap();
        println!("bad password: {t:?}");
        assert!(t.unwrap_err().starts_with("Sign-in refused"));

        let src = std::env::temp_dir().join(format!("otm-live-{}", std::process::id()));
        std::fs::create_dir_all(src.join("folder/sub")).unwrap();
        let big = src.join("big file.bin");
        std::fs::write(&big, vec![7u8; 24 << 20]).unwrap();
        std::fs::write(src.join("folder/sub/a.txt"), "hello").unwrap();
        let n = up.send(&[big.clone(), src.join("folder")], &ids, true);
        assert_eq!(n, 8);
        let t0 = std::time::Instant::now();
        while up.active_count() > 0 && t0.elapsed().as_secs() < 120 {
            up.tick();
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        for j in up.jobs() {
            println!(
                "{} -> {}: {:?} {} {:?}",
                j.name, j.dest_name, j.status, j.error, j.link
            );
            assert_eq!(j.status, JobStatus::Done, "{}", j.error);
        }
        for dir in ["sftp/in/box", "ftp", "dav/shared", "s3/bucket1/builds"] {
            let d = root.join(dir);
            assert_eq!(
                std::fs::metadata(d.join("big file.bin")).unwrap().len(),
                24 << 20,
                "{dir}"
            );
            assert_eq!(
                std::fs::read_to_string(d.join("folder/sub/a.txt")).unwrap(),
                "hello",
                "{dir}"
            );
        }
        // transfer between two destinations: sftp -> ftp and s3, nothing on local disk
        let listed = Remote::new(&up.connector(ids[0]).unwrap(), None)
            .unwrap()
            .list("in/box")
            .unwrap();
        assert!(listed.iter().any(|e| e.name == "folder" && e.is_dir));
        assert!(
            listed
                .iter()
                .any(|e| e.name == "only-remote.txt" && e.size == Some(9))
        );
        let n = up.send_remote(
            ids[0],
            &[
                ("in/box/only-remote.txt".into(), false, Some(9)),
                ("in/box/folder".into(), true, None),
            ],
            &[ids[1], ids[3], ids[0]],
            false,
        );
        assert_eq!(n, 4, "the source itself is skipped");
        let t0 = std::time::Instant::now();
        while up.active_count() > 0 && t0.elapsed().as_secs() < 60 {
            up.tick();
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        for j in up.jobs().iter().filter(|j| j.source.is_some()) {
            println!(
                "{} {} -> {}: {:?} {}",
                j.source_name, j.name, j.dest_name, j.status, j.error
            );
            assert_eq!(j.status, JobStatus::Done, "{}", j.error);
        }
        for dir in ["ftp", "s3/bucket1/builds"] {
            let d = root.join(dir);
            assert_eq!(
                std::fs::read_to_string(d.join("only-remote.txt")).unwrap(),
                "from sftp"
            );
            assert_eq!(
                std::fs::read_to_string(d.join("folder/sub/a.txt")).unwrap(),
                "hello"
            );
        }
        for id in ids {
            up.remove_connector(id);
        }
        let _ = std::fs::remove_dir_all(src);
    }
}
