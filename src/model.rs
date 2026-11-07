//! Data types shared by the engine, the manager, the API and the UI.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub const GENERAL: &str = "General";

/// Default categories: name and extensions (space separated, `*` wildcard).
pub const CATEGORIES: &[(&str, &str)] = &[
    (
        "Video",
        "mp4 mkv webm avi mov wmv flv m4v mpg mpeg ts 3gp ogv",
    ),
    ("Music", "mp3 flac wav aac m4a ogg opus wma aiff alac"),
    ("Images", "jpg jpeg png gif webp svg bmp tif tiff heic avif"),
    (
        "Documents",
        "pdf doc docx xls xlsx ppt pptx odt ods odp txt rtf epub csv md",
    ),
    (
        "Archives",
        "zip rar 7z tar gz tgz bz2 xz zst lz iso img dmg r0* r1*",
    ),
    (
        "Programs",
        "deb rpm appimage flatpak snap pkg exe msi apk sh run",
    ),
];

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Stopped,
    Connecting,
    Downloading,
    Complete,
    Error,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)] // the browser may send only some fields
pub struct MediaOpts {
    /// "best", "2160", "1440", "1080", "720", "480", "360" or "audio"
    pub quality: String,
    /// mp3, m4a, opus, flac (used when quality == "audio")
    pub audio_format: String,
    /// kbps for lossy audio conversion: 320, 192, 128
    pub audio_kbps: u32,
    pub playlist: bool,
    /// exact yt-dlp format id chosen from the format list (browser button)
    #[serde(default)]
    pub format_id: Option<String>,
    /// True after both lyrics and cover were successfully embedded (legacy field).
    pub music_enriched: bool,
}

impl Default for MediaOpts {
    fn default() -> Self {
        Self {
            quality: "best".into(),
            audio_format: "mp3".into(),
            audio_kbps: 192,
            playlist: false,
            format_id: None,
            music_enriched: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Kind {
    Http,
    Media(MediaOpts),
}

/// An external link awaiting the user's decision; never saved as a download.
#[derive(Clone, Debug)]
pub struct DownloadRequest {
    pub url: String,
    pub filename: String,
    pub headers: Vec<(String, String)>,
    pub media: Option<bool>,
    pub opts: Option<MediaOpts>,
}

impl DownloadRequest {
    pub fn from_url(url: String) -> Self {
        Self {
            url,
            filename: String::new(),
            headers: vec![],
            media: None,
            opts: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct Segment {
    pub start: u64,
    /// inclusive end; None while the size is unknown
    pub end: Option<u64>,
    pub pos: u64,
    #[serde(skip)]
    pub busy: bool,
}

impl Segment {
    pub fn new(start: u64, end: Option<u64>) -> Self {
        Self {
            start,
            end,
            pos: start,
            busy: false,
        }
    }
    pub fn done(&self) -> bool {
        matches!(self.end, Some(e) if self.pos > e)
    }
    pub fn remaining(&self) -> u64 {
        match self.end {
            Some(e) => (e + 1).saturating_sub(self.pos),
            None => u64::MAX / 4,
        }
    }
}

/// Runtime-only state (never saved).
#[derive(Default, Debug)]
pub struct Runtime {
    pub running: bool,
    pub stop: Arc<AtomicBool>,
    pub rate: Option<f64>,
    pub samples: VecDeque<(Instant, u64)>,
    pub connections: usize,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Download {
    pub id: u64,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    pub file_name: String,
    pub save_dir: PathBuf,
    pub category: String,
    pub kind: Kind,
    pub size: Option<u64>,
    pub resumable: Option<bool>,
    pub status: Status,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub segments: Vec<Segment>,
    pub date_added: f64,
    #[serde(default)]
    pub last_try: Option<f64>,
    #[serde(default)]
    pub queued: bool,
    #[serde(default)]
    pub auto_name: bool,
    // media (yt-dlp)
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub final_path: Option<PathBuf>,
    #[serde(default)]
    pub media_done: u64,
    #[serde(default)]
    pub media_total: Option<u64>,
    #[serde(skip)]
    pub media_speed: Option<f64>,
    #[serde(skip)]
    pub media_eta: Option<f64>,
    #[serde(skip)]
    pub phase: String,
    #[serde(default)]
    pub tmp_files: Vec<PathBuf>,
    #[serde(skip)]
    pub rt: Runtime,
    #[serde(default)]
    pub retries_left: u32,
}

impl Download {
    pub fn new(
        id: u64,
        url: &str,
        file_name: &str,
        save_dir: PathBuf,
        category: &str,
        kind: Kind,
    ) -> Self {
        Self {
            id,
            url: url.to_string(),
            headers: Vec::new(),
            file_name: file_name.to_string(),
            save_dir,
            category: category.to_string(),
            kind,
            size: None,
            resumable: None,
            status: Status::Stopped,
            error: String::new(),
            segments: Vec::new(),
            date_added: now(),
            last_try: None,
            queued: false,
            auto_name: false,
            title: String::new(),
            final_path: None,
            media_done: 0,
            media_total: None,
            media_speed: None,
            media_eta: None,
            phase: String::new(),
            tmp_files: Vec::new(),
            rt: Runtime::default(),
            retries_left: 10,
        }
    }

    pub fn is_media(&self) -> bool {
        matches!(self.kind, Kind::Media(_))
    }

    pub fn path(&self) -> PathBuf {
        if let Some(p) = &self.final_path {
            return p.clone();
        }
        self.save_dir.join(&self.file_name)
    }

    pub fn part_path(&self) -> PathBuf {
        self.save_dir
            .join(format!("{}.{}.otmpart", self.file_name, self.id))
    }

    pub fn downloaded(&self) -> u64 {
        if self.is_media() {
            return if self.status == Status::Complete {
                self.size.unwrap_or(self.media_done)
            } else {
                self.media_done
            };
        }
        self.segments.iter().map(|s| s.pos - s.start).sum()
    }

    pub fn progress(&self) -> Option<f64> {
        if self.status == Status::Complete {
            return Some(1.0);
        }
        let total = if self.is_media() {
            self.media_total
        } else {
            self.size
        };
        match total {
            Some(t) if t > 0 => Some((self.downloaded() as f64 / t as f64).min(1.0)),
            _ => None,
        }
    }

    pub fn running(&self) -> bool {
        self.rt.running
    }

    pub fn time_left(&self) -> Option<f64> {
        if !self.running() {
            return None;
        }
        if self.is_media() {
            return self.media_eta;
        }
        let rate = self.rt.rate?;
        let size = self.size?;
        if rate <= 0.0 {
            return None;
        }
        Some((size.saturating_sub(self.downloaded())) as f64 / rate)
    }

    pub fn status_text(&self) -> String {
        match self.status {
            Status::Downloading => {
                if self.phase.is_empty() {
                    "Downloading".into()
                } else {
                    format!("Downloading · {}", self.phase)
                }
            }
            Status::Connecting => {
                if self.is_media() {
                    "Fetching info".into()
                } else {
                    "Connecting".into()
                }
            }
            Status::Complete => "Completed".into(),
            Status::Error => "Failed".into(),
            Status::Stopped if self.queued => "Queued".into(),
            Status::Stopped => "Paused".into(),
        }
    }

    pub fn display_name(&self) -> &str {
        if self.file_name.is_empty() || self.file_name == self.url {
            if self.title.is_empty() {
                &self.url
            } else {
                &self.title
            }
        } else {
            &self.file_name
        }
    }

    /// Final file size when complete (after MP3 conversion), else the expected size.
    pub fn display_size(&self) -> Option<u64> {
        if self.status == Status::Complete && self.size.is_some() {
            return self.size;
        }
        if self.is_media() {
            self.media_total.or(self.size)
        } else {
            self.size
        }
    }

    pub fn host(&self) -> String {
        host_of(&self.url)
    }

    /// Temporary files to delete when the download is removed.
    pub fn leftover_files(&self) -> Vec<PathBuf> {
        if self.is_media() {
            self.tmp_files
                .iter()
                .filter(|p| {
                    let s = p.to_string_lossy();
                    s.ends_with(".part") || s.contains(".part-Frag") || s.ends_with(".ytdl")
                })
                .cloned()
                .collect()
        } else {
            vec![self.part_path()]
        }
    }

    pub fn reset(&mut self) {
        for p in self.leftover_files() {
            let _ = std::fs::remove_file(p);
        }
        self.status = Status::Stopped;
        self.segments.clear();
        self.size = None;
        self.error.clear();
        self.final_path = None;
        self.media_done = 0;
        self.media_total = None;
        self.tmp_files.clear();
    }
}

/// One row for the API / browser extension.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Summary {
    pub id: u64,
    pub kind: String,
    pub name: String,
    pub url: String,
    pub status: Status,
    pub status_text: String,
    pub error: String,
    pub progress: Option<f64>,
    pub size: Option<u64>,
    pub downloaded: u64,
    pub rate: Option<f64>,
    pub eta: Option<f64>,
    pub category: String,
    pub path: String,
    pub running: bool,
    #[serde(default)]
    pub gold: bool,
}

impl From<&Download> for Summary {
    fn from(d: &Download) -> Self {
        Summary {
            gold: matches!(&d.kind, Kind::Media(o) if o.music_enriched),
            id: d.id,
            kind: if d.is_media() {
                "media".into()
            } else {
                "http".into()
            },
            name: d.display_name().to_string(),
            url: d.url.clone(),
            status: d.status,
            status_text: d.status_text(),
            error: d.error.clone(),
            progress: d.progress(),
            size: d.display_size(),
            downloaded: d.downloaded(),
            rate: if d.running() { d.rt.rate } else { None },
            eta: d.time_left(),
            category: d.category.clone(),
            path: d.path().to_string_lossy().into_owned(),
            running: d.running(),
        }
    }
}

// ------------------------------------------------------------------ settings

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Category {
    pub name: String,
    pub exts: String,
    pub folder: PathBuf,
}

impl Category {
    pub fn matches(&self, file_name: &str) -> bool {
        let ext = extension(file_name);
        if ext.is_empty() {
            return false;
        }
        self.exts
            .split_whitespace()
            .any(|pat| glob_match(&pat.to_lowercase(), &ext))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Schedule {
    pub start_on: bool,
    pub start_at: String,
    pub stop_on: bool,
    pub stop_at: String,
    #[serde(default)]
    pub last_start: String,
    #[serde(default)]
    pub last_stop: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub base_folder: PathBuf,
    pub categories: Vec<Category>,
    pub max_connections: usize,
    pub server_connections: Vec<(String, usize)>,
    pub limit_kb: u64,
    pub limiter_on: bool,
    pub limiter_on_startup: bool,
    pub queue_at_once: usize,
    pub retries: u32,
    pub schedule: Schedule,
    pub theme: ThemeMode,
    pub notify_on_complete: bool,
    pub show_details: bool,
    pub media: MediaOpts,
    pub cookies_browser: String,
    pub start_minimized: bool,
    pub clipboard_watch: bool,
    /// Add cover art and lyrics to MP3 files (asks LRCLIB, MusicBrainz and Cover Art Archive).
    /// Off by default: it contacts third-party services, so the user turns it on.
    pub music_tags: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let base = crate::paths::downloads_dir();
        Settings {
            categories: default_categories(&base),
            base_folder: base,
            max_connections: 8,
            server_connections: Vec::new(),
            limit_kb: 1024,
            limiter_on: false,
            limiter_on_startup: false,
            queue_at_once: 3,
            retries: 10,
            schedule: Schedule {
                start_on: false,
                start_at: "02:00".into(),
                stop_on: false,
                stop_at: "07:00".into(),
                last_start: String::new(),
                last_stop: String::new(),
            },
            theme: ThemeMode::System,
            notify_on_complete: true,
            show_details: true,
            media: MediaOpts::default(),
            cookies_browser: String::new(),
            start_minimized: false,
            clipboard_watch: false,
            music_tags: false,
        }
    }
}

pub fn default_categories(base: &std::path::Path) -> Vec<Category> {
    CATEGORIES
        .iter()
        .map(|(n, e)| Category {
            name: n.to_string(),
            exts: e.to_string(),
            folder: base.join(n),
        })
        .collect()
}

// ------------------------------------------------------------------ file types (icons in the list)

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileType {
    Video,
    Audio,
    Disk,
    Archive,
    Document,
    Image,
    Program,
    Other,
}

pub fn file_type(name: &str) -> FileType {
    let ext = extension(name);
    let has = |list: &str| list.split(' ').any(|e| e == ext);
    if has("mp4 mkv webm avi mov m4v wmv flv mpg mpeg ts 3gp ogv") {
        FileType::Video
    } else if has("mp3 flac wav m4a ogg opus aac wma aiff alac") {
        FileType::Audio
    } else if has("iso img dmg") {
        FileType::Disk
    } else if has("zip 7z rar gz xz zst bz2 tar tgz lz") {
        FileType::Archive
    } else if has("pdf doc docx xls xlsx ppt pptx odt ods odp txt rtf epub csv md") {
        FileType::Document
    } else if has("png jpg jpeg gif webp svg avif heic bmp tif tiff") {
        FileType::Image
    } else if has("deb rpm appimage flatpak snap pkg exe msi apk sh run") {
        FileType::Program
    } else {
        FileType::Other
    }
}

// ------------------------------------------------------------------ helpers

pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn extension(name: &str) -> String {
    let clean = name.split(['?', '#']).next().unwrap_or("");
    let base = clean.rsplit(['/', '\\']).next().unwrap_or("");
    match base.rfind('.') {
        Some(i) if i > 0 => base[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

fn glob_match(pat: &str, text: &str) -> bool {
    match pat.find('*') {
        None => pat == text,
        Some(i) => {
            text.starts_with(&pat[..i])
                && text.ends_with(&pat[i + 1..])
                && text.len() >= pat.len() - 1
        }
    }
}

pub fn host_of(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    host.trim_start_matches("www.").to_lowercase()
}

pub fn safe_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if "\\/:*?\"<>|".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    out = out.trim_matches(|c| c == ' ' || c == '.').to_string();
    if out.len() > 240 {
        let mut cut = 240;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
    }
    if out.is_empty() {
        "download".into()
    } else {
        out
    }
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn name_from_url(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    // drop the host: only the path after the first "/" can hold a file name
    let path = rest.split_once('/').map(|(_, p)| p).unwrap_or("");
    let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    safe_name(&percent_decode(last))
}

pub fn fmt_size(n: Option<u64>) -> String {
    let Some(n) = n else { return String::new() };
    let mut v = n as f64;
    if v < 1024.0 {
        return format!("{n} B");
    }
    for unit in ["KB", "MB", "GB", "TB"] {
        v /= 1024.0;
        if v < 1024.0 || unit == "TB" {
            return format!("{v:.1} {unit}");
        }
    }
    String::new()
}

pub fn fmt_rate(bps: Option<f64>) -> String {
    match bps {
        Some(b) if b > 0.0 => format!("{}/s", fmt_size(Some(b as u64))),
        _ => String::new(),
    }
}

pub fn fmt_eta(sec: Option<f64>) -> String {
    let Some(s) = sec else { return String::new() };
    let s = s.max(0.0) as u64;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else if s < 86400 {
        format!("{}h {:02}m", s / 3600, (s % 3600) / 60)
    } else {
        format!("{}d {}h", s / 86400, (s % 86400) / 3600)
    }
}

pub fn fmt_duration(sec: f64) -> String {
    let s = sec.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_types() {
        assert_eq!(
            name_from_url("https://x.org/a/My%20File.zip?x=1"),
            "My File.zip"
        );
        assert_eq!(name_from_url("https://x.org/"), "download");
        assert_eq!(safe_name("a/b:c*?.txt"), "a_b_c__.txt");
        assert_eq!(extension("Ubuntu.ISO"), "iso");
        assert_eq!(file_type("movie.mkv"), FileType::Video);
        assert_eq!(file_type("x.tar.xz"), FileType::Archive);
        assert_eq!(file_type("song.MP3"), FileType::Audio);
        assert_eq!(file_type("ubuntu.iso"), FileType::Disk);
        assert_eq!(host_of("https://www.YouTube.com/watch?v=1"), "youtube.com");
    }

    #[test]
    fn partial_media_opts_from_browser() {
        let o: MediaOpts =
            serde_json::from_str(r#"{"quality":"audio","audio_format":"mp3","audio_kbps":320}"#)
                .unwrap();
        assert_eq!(o.audio_kbps, 320);
        assert!(!o.playlist);
    }

    #[test]
    fn formatting() {
        assert_eq!(fmt_size(Some(512)), "512 B");
        assert_eq!(fmt_size(Some(1536)), "1.5 KB");
        assert_eq!(fmt_size(Some(5 * 1024 * 1024 * 1024)), "5.0 GB");
        assert_eq!(fmt_eta(Some(75.0)), "1m 15s");
        assert_eq!(fmt_duration(634.0), "10:34");
    }

    #[test]
    fn categories_match() {
        let cats = default_categories(std::path::Path::new("/d"));
        let find = |n: &str| cats.iter().find(|c| c.matches(n)).map(|c| c.name.clone());
        assert_eq!(find("a.ZIP").as_deref(), Some("Archives"));
        assert_eq!(find("b.r01").as_deref(), Some("Archives"));
        assert_eq!(find("c.docx").as_deref(), Some("Documents"));
        assert_eq!(find("e.deb").as_deref(), Some("Programs"));
        assert_eq!(find("g.xyz"), None);
    }
}
