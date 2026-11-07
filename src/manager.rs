//! The download list, settings, queue, scheduler and speed limiter.
//! `Manager` is cheap to clone and safe to use from any thread (UI, local API, workers).

use crate::model::*;
use crate::{http, media, paths};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub type DlRef = Arc<Mutex<Download>>;

/// Lock that survives a panicked worker instead of poisoning the whole app.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ------------------------------------------------------------------ limiter

pub struct RateLimiter {
    state: Mutex<(u64, f64, Instant)>, // rate (bytes/s, 0 = off), tokens, last refill
}

impl RateLimiter {
    pub fn new(rate: u64) -> Self {
        Self {
            state: Mutex::new((rate, 0.0, Instant::now())),
        }
    }
    pub fn set_rate(&self, rate: u64) {
        *lock(&self.state) = (rate, 0.0, Instant::now());
    }
    /// Wait until `n` bytes may pass. Returns early when `stop` is set.
    pub fn consume(&self, n: usize, stop: &AtomicBool) {
        let mut wait = {
            let mut s = lock(&self.state);
            let rate = s.0;
            if rate == 0 {
                return;
            }
            let now = Instant::now();
            s.1 = (s.1 + now.duration_since(s.2).as_secs_f64() * rate as f64).min(rate as f64);
            s.2 = now;
            s.1 -= n as f64;
            if s.1 < 0.0 { -s.1 / rate as f64 } else { 0.0 }
        };
        while wait > 0.0 && !stop.load(Ordering::Relaxed) {
            let step = wait.min(0.2);
            std::thread::sleep(Duration::from_secs_f64(step));
            wait -= step;
        }
    }
}

// ------------------------------------------------------------------ events for the UI

#[derive(Clone, Debug)]
pub enum Event {
    Finished { name: String, path: PathBuf },
    Failed { name: String, error: String },
    Added,
    ShowWindow,
    ShowBrowserSetup,
    ConfirmDownload(DownloadRequest),
}

// ------------------------------------------------------------------ manager

struct Inner {
    downloads: Mutex<Vec<DlRef>>,
    settings: Mutex<Settings>,
    next_id: AtomicU64,
    limiter: RateLimiter,
    persist: bool,
    save_lock: Mutex<()>,
    queue_running: AtomicBool,
    confirmation_ui: AtomicBool,
    events: Mutex<Vec<Event>>,
    repaint: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    agent: ureq::Agent,
}

#[derive(Clone)]
pub struct Manager {
    inner: Arc<Inner>,
}

#[derive(Serialize, Deserialize, Default)]
struct SavedList {
    downloads: Vec<Download>,
}

impl Manager {
    pub fn new(persist: bool) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_per_read(Some(Duration::from_secs(30)))
            .user_agent(format!(
                "Mozilla/5.0 OpenTransferManager/{}",
                env!("CARGO_PKG_VERSION")
            ))
            .max_redirects(10)
            .build()
            .into();
        let m = Manager {
            inner: Arc::new(Inner {
                downloads: Mutex::new(Vec::new()),
                settings: Mutex::new(Settings::default()),
                next_id: AtomicU64::new(1),
                limiter: RateLimiter::new(0),
                persist,
                save_lock: Mutex::new(()),
                queue_running: AtomicBool::new(false),
                confirmation_ui: AtomicBool::new(false),
                events: Mutex::new(Vec::new()),
                repaint: Mutex::new(None),
                agent,
            }),
        };
        if persist {
            m.load();
        }
        let s = m.settings();
        m.set_limiter(s.limiter_on_startup, None);
        m
    }

    pub fn agent(&self) -> &ureq::Agent {
        &self.inner.agent
    }

    pub fn enable_confirmation_ui(&self) {
        self.inner.confirmation_ui.store(true, Ordering::SeqCst);
    }
    pub fn has_confirmation_ui(&self) -> bool {
        self.inner.confirmation_ui.load(Ordering::SeqCst)
    }

    // ---------------------------------------------------------------- persistence
    fn settings_path() -> PathBuf {
        paths::data_dir().join("settings.json")
    }
    fn list_path() -> PathBuf {
        paths::data_dir().join("downloads.json")
    }

    fn load(&self) {
        if let Some(s) = read_json::<Settings>(&Self::settings_path()) {
            *lock(&self.inner.settings) = s;
        }
        if let Some(list) = read_json::<SavedList>(&Self::list_path()) {
            let mut legacy_counts = std::collections::HashMap::new();
            for d in &list.downloads {
                if !d.is_media() && d.status != Status::Complete {
                    *legacy_counts
                        .entry(d.save_dir.join(format!("{}.otmpart", d.file_name)))
                        .or_insert(0usize) += 1;
                }
            }
            let mut max = 0;
            let items: Vec<DlRef> = list
                .downloads
                .into_iter()
                .map(|mut d| {
                    // Migrate only unambiguous old partials. Ambiguous legacy files
                    // are preserved; absent new partials make the engine start fresh.
                    if !d.is_media() && d.status != Status::Complete && !d.part_path().exists() {
                        let legacy = d.save_dir.join(format!("{}.otmpart", d.file_name));
                        if legacy_counts.get(&legacy) == Some(&1) && legacy.is_file() {
                            let _ = std::fs::rename(legacy, d.part_path());
                        }
                    }
                    max = max.max(d.id);
                    if matches!(d.status, Status::Connecting | Status::Downloading) {
                        d.status = Status::Stopped;
                    }
                    Arc::new(Mutex::new(d))
                })
                .collect();
            self.inner.next_id.store(max + 1, Ordering::SeqCst);
            *lock(&self.inner.downloads) = items;
        }
    }

    pub fn save(&self) {
        if !self.inner.persist {
            return;
        }
        let _guard = lock(&self.inner.save_lock);
        let settings = self.settings();
        let list: Vec<serde_json::Value> = self
            .list()
            .iter()
            .filter_map(|d| serde_json::to_value(&*lock(d)).ok())
            .collect();
        write_json(&Self::settings_path(), &settings);
        write_json(
            &Self::list_path(),
            &serde_json::json!({ "downloads": list }),
        );
    }

    // ---------------------------------------------------------------- settings
    pub fn settings(&self) -> Settings {
        lock(&self.inner.settings).clone()
    }

    pub fn update_settings(&self, f: impl FnOnce(&mut Settings)) {
        {
            let mut s = lock(&self.inner.settings);
            f(&mut s);
        }
        let s = self.settings();
        self.inner
            .limiter
            .set_rate(if s.limiter_on { s.limit_kb * 1024 } else { 0 });
        self.save();
    }

    pub fn set_limiter(&self, on: bool, kb: Option<u64>) {
        let mut s = lock(&self.inner.settings);
        s.limiter_on = on;
        if let Some(k) = kb {
            s.limit_kb = k.max(16);
        }
        self.inner
            .limiter
            .set_rate(if s.limiter_on { s.limit_kb * 1024 } else { 0 });
    }

    pub fn limiter(&self) -> &RateLimiter {
        &self.inner.limiter
    }

    pub fn global_limit_bytes(&self) -> u64 {
        let s = lock(&self.inner.settings);
        if s.limiter_on { s.limit_kb * 1024 } else { 0 }
    }

    pub fn category_for(&self, name: &str) -> String {
        let s = lock(&self.inner.settings);
        s.categories
            .iter()
            .find(|c| c.matches(name))
            .map(|c| c.name.clone())
            .unwrap_or_else(|| GENERAL.to_string())
    }

    pub fn folder_for(&self, category: &str) -> PathBuf {
        let s = lock(&self.inner.settings);
        s.categories
            .iter()
            .find(|c| c.name == category)
            .map(|c| c.folder.clone())
            .unwrap_or_else(|| s.base_folder.clone())
    }

    pub fn max_connections_for(&self, url: &str) -> usize {
        let host = host_of(url);
        let s = lock(&self.inner.settings);
        for (h, n) in &s.server_connections {
            let h = h.to_lowercase();
            if host == h || host.ends_with(&format!(".{h}")) {
                return (*n).max(1);
            }
        }
        s.max_connections.max(1)
    }

    // ---------------------------------------------------------------- events
    pub fn set_repaint(&self, f: impl Fn() + Send + Sync + 'static) {
        *lock(&self.inner.repaint) = Some(Box::new(f));
    }
    pub fn repaint(&self) {
        if let Some(f) = &*lock(&self.inner.repaint) {
            f();
        }
    }
    pub fn push_event(&self, e: Event) {
        lock(&self.inner.events).push(e);
        self.repaint();
    }
    pub fn take_events(&self) -> Vec<Event> {
        std::mem::take(&mut *lock(&self.inner.events))
    }

    // ---------------------------------------------------------------- list
    pub fn list(&self) -> Vec<DlRef> {
        lock(&self.inner.downloads).clone()
    }

    pub fn find(&self, id: u64) -> Option<DlRef> {
        self.list().into_iter().find(|d| lock(d).id == id)
    }

    fn next_id(&self) -> u64 {
        self.inner.next_id.fetch_add(1, Ordering::SeqCst)
    }

    fn register(&self, d: Download, start: bool) -> DlRef {
        let r = Arc::new(Mutex::new(d));
        lock(&self.inner.downloads).push(r.clone());
        if start {
            self.start(&r);
        }
        self.save();
        self.push_event(Event::Added);
        r
    }

    #[allow(clippy::too_many_arguments)] // Explicit transfer parameters shared by UI/API.
    pub fn add_http(
        &self,
        url: &str,
        file_name: &str,
        save_dir: &Path,
        category: &str,
        start: bool,
        queued: bool,
        headers: Vec<(String, String)>,
    ) -> DlRef {
        let mut d = Download::new(
            self.next_id(),
            url,
            &safe_name(file_name),
            save_dir.to_path_buf(),
            category,
            Kind::Http,
        );
        d.headers = headers;
        d.queued = queued;
        d.retries_left = self.settings().retries;
        self.register(d, start)
    }

    #[allow(clippy::too_many_arguments)] // Same transfer boundary as add_http.
    pub fn add_media(
        &self,
        url: &str,
        opts: MediaOpts,
        save_dir: Option<PathBuf>,
        title: &str,
        start: bool,
        queued: bool,
        headers: Vec<(String, String)>,
    ) -> DlRef {
        let cat = if opts.quality == "audio" {
            "Music"
        } else {
            "Video"
        };
        let dir = save_dir.unwrap_or_else(|| self.folder_for(cat));
        let mut d = Download::new(self.next_id(), url, title, dir, cat, Kind::Media(opts));
        d.title = title.to_string();
        d.headers = headers;
        d.queued = queued;
        d.retries_left = self.settings().retries;
        self.register(d, start)
    }

    /// Add a link from the browser or the command line: video page -> yt-dlp, else a plain file.
    pub fn add_smart(
        &self,
        url: &str,
        file_name: &str,
        media: Option<bool>,
        start: bool,
        headers: Vec<(String, String)>,
        opts: Option<MediaOpts>,
    ) -> DlRef {
        let is_media = media.unwrap_or_else(|| media::looks_like_media_page(url));
        if is_media {
            let opts = opts.unwrap_or_else(|| self.settings().media);
            return self.add_media(url, opts, None, "", start, !start, headers);
        }
        let name = if file_name.is_empty() {
            name_from_url(url)
        } else {
            safe_name(file_name)
        };
        let cat = self.category_for(&name);
        let dir = self.folder_for(&cat);
        let r = self.add_http(url, &name, &dir, &cat, false, !start, headers);
        lock(&r).auto_name = file_name.is_empty();
        if start {
            self.start(&r);
        }
        r
    }

    // ---------------------------------------------------------------- control
    pub fn start(&self, dl: &DlRef) {
        {
            let mut d = lock(dl);
            if d.rt.running || d.status == Status::Complete {
                return;
            }
            d.rt.running = true;
            d.rt.stop = Arc::new(AtomicBool::new(false));
            d.rt.samples.clear();
            d.status = Status::Connecting;
            d.error.clear();
            d.last_try = Some(now());
        }
        let mgr = self.clone();
        let dl = dl.clone();
        std::thread::Builder::new()
            .name("otm-download".into())
            .spawn(move || {
                let is_media = lock(&dl).is_media();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if is_media {
                        media::run(&dl, &mgr)
                    } else {
                        http::run(&dl, &mgr)
                    }
                }));
                let event = {
                    let mut d = lock(&dl);
                    if result.is_err() {
                        d.status = Status::Error;
                        d.error = "Internal error (please report it)".into();
                    }
                    d.rt.running = false;
                    d.rt.connections = 0;
                    d.rt.rate = None;
                    d.media_speed = None;
                    d.media_eta = None;
                    for s in d.segments.iter_mut() {
                        s.busy = false;
                    }
                    match d.status {
                        Status::Complete => Some(Event::Finished {
                            name: d.display_name().to_string(),
                            path: d.path(),
                        }),
                        Status::Error => Some(Event::Failed {
                            name: d.display_name().to_string(),
                            error: d.error.clone(),
                        }),
                        _ => None,
                    }
                };
                mgr.save();
                if let Some(e) = event {
                    mgr.push_event(e);
                } else {
                    mgr.repaint();
                }
            })
            .expect("spawn download thread");
    }

    pub fn stop(&self, dl: &DlRef) {
        lock(dl).rt.stop.store(true, Ordering::SeqCst);
    }

    pub fn stop_all(&self) {
        self.inner.queue_running.store(false, Ordering::SeqCst);
        for d in self.list() {
            self.stop(&d);
        }
    }

    pub fn remove(&self, id: u64, delete_file: bool) {
        let Some(dl) = self.find(id) else { return };
        self.stop(&dl);
        lock(&self.inner.downloads).retain(|d| !Arc::ptr_eq(d, &dl));
        let mgr = self.clone();
        std::thread::spawn(move || {
            for _ in 0..300 {
                if !lock(&dl).rt.running {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let d = lock(&dl);
            let mut files = d.leftover_files();
            // Never delete a file that another entry of the list still points to
            // (the same video downloaded twice gives two entries for one file).
            let shared = mgr.list().iter().any(|o| lock(o).path() == d.path());
            if delete_file && d.status == Status::Complete && !shared {
                files.push(d.path());
            }
            for f in files {
                if f.is_file() {
                    let _ = std::fs::remove_file(f);
                }
            }
        });
        self.save();
        self.repaint();
    }

    pub fn redownload(&self, dl: &DlRef) {
        if lock(dl).rt.running {
            return;
        }
        lock(dl).reset();
        self.start(dl);
    }

    pub fn queue_running(&self) -> bool {
        self.inner.queue_running.load(Ordering::SeqCst)
    }

    pub fn start_queue(&self) {
        let retries = self.settings().retries;
        for d in self.list() {
            let mut d = lock(&d);
            if d.queued && d.status == Status::Error {
                d.retries_left = retries;
                d.status = Status::Stopped;
            }
        }
        self.inner.queue_running.store(true, Ordering::SeqCst);
        self.repaint();
    }

    pub fn stop_queue(&self) {
        self.inner.queue_running.store(false, Ordering::SeqCst);
        for d in self.list() {
            if lock(&d).queued {
                self.stop(&d);
            }
        }
    }

    pub fn total_rate(&self) -> f64 {
        self.list()
            .iter()
            .map(|d| lock(d).rt.rate.unwrap_or(0.0))
            .sum()
    }

    pub fn active_count(&self) -> usize {
        self.list().iter().filter(|d| lock(d).rt.running).count()
    }

    /// Call about twice a second: speeds, queue, scheduler.
    pub fn tick(&self) {
        let now_i = Instant::now();
        for d in self.list() {
            let mut d = lock(&d);
            if d.is_media() {
                d.rt.rate = if d.rt.running { d.media_speed } else { None };
                continue;
            }
            let got = d.downloaded();
            d.rt.samples.push_back((now_i, got));
            while let Some((t, _)) = d.rt.samples.front() {
                if now_i.duration_since(*t) > Duration::from_secs(3) {
                    d.rt.samples.pop_front();
                } else {
                    break;
                }
            }
            if !d.rt.running {
                d.rt.rate = None;
            } else if let (Some(a), Some(b)) = (d.rt.samples.front(), d.rt.samples.back()) {
                let dt = b.0.duration_since(a.0).as_secs_f64();
                d.rt.rate = if dt > 0.2 {
                    Some((b.1.saturating_sub(a.1)) as f64 / dt)
                } else {
                    d.rt.rate
                };
            }
        }
        if self.queue_running() {
            let at_once = self.settings().queue_at_once.max(1);
            let members: Vec<DlRef> = self.list().into_iter().filter(|d| lock(d).queued).collect();
            let running = members.iter().filter(|d| lock(d).rt.running).count();
            let waiting: Vec<DlRef> = members
                .into_iter()
                .filter(|d| {
                    let d = lock(d);
                    !d.rt.running
                        && (d.status == Status::Stopped
                            || (d.status == Status::Error && d.retries_left > 0))
                })
                .collect();
            if running == 0 && waiting.is_empty() {
                self.inner.queue_running.store(false, Ordering::SeqCst);
            }
            for d in waiting.into_iter().take(at_once.saturating_sub(running)) {
                {
                    let mut g = lock(&d);
                    if g.status == Status::Error {
                        g.retries_left = g.retries_left.saturating_sub(1);
                    }
                }
                self.start(&d);
            }
        }
        self.scheduler();
    }

    fn scheduler(&self) {
        let now = chrono::Local::now();
        let hm = now.format("%H:%M").to_string();
        let today = now.format("%Y-%m-%d").to_string();
        let mut start = false;
        let mut stop = false;
        {
            let mut s = lock(&self.inner.settings);
            let sc = &mut s.schedule;
            if sc.start_on && sc.start_at == hm && sc.last_start != today {
                sc.last_start = today.clone();
                start = true;
            }
            if sc.stop_on && sc.stop_at == hm && sc.last_stop != today {
                sc.last_stop = today;
                stop = true;
            }
        }
        if start {
            self.start_queue();
        }
        if stop {
            self.stop_queue();
        }
    }

    pub fn shutdown(&self) {
        self.stop_all();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && self.active_count() > 0 {
            std::thread::sleep(Duration::from_millis(50));
        }
        self.save();
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    for p in [path.to_path_buf(), path.with_extension("json.bak")] {
        if let Ok(text) = std::fs::read_to_string(&p)
            && let Ok(v) = serde_json::from_str(&text)
        {
            return Some(v);
        }
    }
    None
}

fn write_json<T: Serialize>(path: &Path, value: &T) {
    let Ok(text) = serde_json::to_string_pretty(value) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        if path.exists() {
            let _ = std::fs::copy(path, path.with_extension("json.bak"));
        }
        let _ = std::fs::rename(&tmp, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_a_duplicate_keeps_the_file_the_other_entry_uses() {
        let dir = std::env::temp_dir().join(format!("otm-dup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("AURORA - Runaway [d_HlPboLRL8] 320k.mp3");
        std::fs::write(&file, b"audio").unwrap();
        let mgr = Manager::new(false);
        let add = || {
            let d = mgr.add_http(
                "https://example.com/a.mp3",
                "AURORA - Runaway [d_HlPboLRL8] 320k.mp3",
                &dir,
                "Music",
                false,
                false,
                vec![],
            );
            lock(&d).status = Status::Complete;
            lock(&d).id
        };
        let (first, second) = (add(), add());
        let wait = || std::thread::sleep(Duration::from_millis(400));
        mgr.remove(first, true);
        wait();
        assert!(file.exists(), "the other entry still uses this file");
        mgr.remove(second, true);
        wait();
        assert!(
            !file.exists(),
            "last entry removed with delete_file: file deleted"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
