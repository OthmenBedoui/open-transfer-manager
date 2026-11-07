//! Video and audio from YouTube and 1000+ sites, through the yt-dlp program.
//!
//! yt-dlp runs as a separate process: if it crashes the app keeps working, and it can be
//! updated on its own (`yt-dlp -U`). MP3 conversion is done by yt-dlp with ffmpeg.

use crate::manager::{DlRef, Manager, lock};
use crate::model::*;
use crate::paths;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;

const MEDIA_HOSTS: &[&str] = &[
    "youtube.com",
    "youtu.be",
    "tiktok.com",
    "instagram.com",
    "facebook.com",
    "fb.watch",
    "twitter.com",
    "x.com",
    "vimeo.com",
    "dailymotion.com",
    "dai.ly",
    "twitch.tv",
    "reddit.com",
    "v.redd.it",
    "soundcloud.com",
    "bandcamp.com",
    "bilibili.com",
    "rumble.com",
    "streamable.com",
    "threads.net",
    "pinterest.com",
    "linkedin.com",
    "kick.com",
    "odysee.com",
    "mixcloud.com",
    "9gag.com",
    "imgur.com",
    "vk.com",
    "ok.ru",
];

/// Fast guess without running yt-dlp: is this a video/audio page rather than a direct file link?
pub fn looks_like_media_page(url: &str) -> bool {
    let host = host_of(url);
    if host.is_empty() {
        return false;
    }
    let path = url
        .split("://")
        .nth(1)
        .unwrap_or("")
        .split(['?', '#'])
        .next()
        .unwrap_or("");
    if file_type(path) != FileType::Other
        && !path.ends_with('/')
        && extension(path).len() <= 5
        && !extension(path).is_empty()
    {
        return false; // direct link to a file
    }
    MEDIA_HOSTS
        .iter()
        .any(|h| host == *h || host.ends_with(&format!(".{h}")))
}

pub fn ytdlp_path() -> Option<PathBuf> {
    paths::find_program("yt-dlp")
}

pub fn ffmpeg_path() -> Option<PathBuf> {
    paths::find_program("ffmpeg")
}

fn command(program: &std::path::Path) -> Command {
    let mut c = paths::background_command(program);
    c.env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

pub fn ytdlp_version() -> Option<String> {
    let p = ytdlp_path()?;
    let out = command(&p).arg("--version").output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

/// Download the official standalone yt-dlp into our tools folder, or update the one we have.
pub fn install_or_update() -> Result<String, String> {
    if let Some(p) = ytdlp_path() {
        let out = command(&p).arg("-U").output().map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr);
        return Ok(text.trim().to_string());
    }
    let asset = if cfg!(windows) {
        "yt-dlp.exe"
    } else if cfg!(target_os = "macos") {
        "yt-dlp_macos"
    } else if cfg!(target_arch = "aarch64") {
        "yt-dlp_linux_aarch64"
    } else {
        "yt-dlp_linux"
    };
    let url = format!("https://github.com/yt-dlp/yt-dlp/releases/latest/download/{asset}");
    let target = paths::tools_dir().join(if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    });
    let resp = ureq::get(&url)
        .call()
        .map_err(|e| format!("Download failed: {e}"))?;
    let mut reader = resp.into_body().into_reader();
    let tmp = target.with_extension("download");
    let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    std::io::copy(&mut reader, &mut f).map_err(|e| e.to_string())?;
    drop(f);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    std::fs::rename(&tmp, &target).map_err(|e| e.to_string())?;
    Ok(format!("yt-dlp installed in {}", target.display()))
}

// ------------------------------------------------------------------ format list

/// One choice shown in the browser button menu and the new download dialog.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Choice {
    pub id: String,
    /// "video" or "audio"
    pub group: String,
    pub ext: String,
    pub label: String,
    pub detail: String,
    pub size: Option<u64>,
    pub recommended: bool,
    pub opts: MediaOpts,
    #[serde(default)]
    pub gold: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MediaInfo {
    pub title: String,
    pub uploader: String,
    pub duration: Option<f64>,
    pub thumbnail: String,
    pub is_playlist: bool,
    pub count: usize,
    pub extractor: String,
    pub choices: Vec<Choice>,
}

fn base_args(mgr: &Manager, headers: &[(String, String)]) -> Vec<String> {
    let mut a = vec![
        "--no-warnings".to_string(),
        "--no-colors".to_string(),
        "--encoding".to_string(),
        "utf-8".to_string(),
    ];
    let s = mgr.settings();
    if !s.cookies_browser.is_empty() {
        a.push("--cookies-from-browser".into());
        a.push(s.cookies_browser.clone());
    }
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("referer") && !v.is_empty() {
            a.push("--referer".into());
            a.push(v.clone());
        }
        if k.eq_ignore_ascii_case("user-agent") && !v.is_empty() {
            a.push("--user-agent".into());
            a.push(v.clone());
        }
    }
    if let Some(ff) = ffmpeg_path() {
        a.push("--ffmpeg-location".into());
        a.push(ff.to_string_lossy().into_owned());
    }
    a
}

/// Ask yt-dlp about a page: title, duration, and the formats the user can pick.
pub fn probe(mgr: &Manager, url: &str, headers: &[(String, String)]) -> Result<MediaInfo, String> {
    Ok(info_from_json(
        &probe_json(mgr, url, headers)?,
        ffmpeg_path().is_some(),
    ))
}

fn probe_json(mgr: &Manager, url: &str, headers: &[(String, String)]) -> Result<Value, String> {
    let yt = ytdlp_path().ok_or("yt-dlp is not installed. Settings → Video & audio → Install.")?;
    let out = command(&yt)
        .args(base_args(mgr, headers))
        .args(["-J", "--no-playlist", "--flat-playlist", "--", url])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("Cannot run yt-dlp: {e}"))?;
    if !out.status.success() {
        return Err(clean_error(&String::from_utf8_lossy(&out.stderr)));
    }
    let v: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("Unexpected yt-dlp answer: {e}"))?;
    Ok(v)
}

pub fn info_from_json(v: &Value, have_ffmpeg: bool) -> MediaInfo {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let entries = v.get("entries").and_then(Value::as_array);
    let duration = v.get("duration").and_then(Value::as_f64);
    let formats = v["formats"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let choices = build_choices(formats, duration, have_ffmpeg);
    MediaInfo {
        title: s("title"),
        uploader: {
            let u = s("uploader");
            if u.is_empty() { s("channel") } else { u }
        },
        duration,
        thumbnail: s("thumbnail"),
        is_playlist: entries.is_some(),
        count: entries.map(|e| e.len()).unwrap_or(1),
        extractor: s("extractor_key"),
        choices,
    }
}

fn fmt_size_of(f: &Value, duration: Option<f64>) -> Option<u64> {
    f.get("filesize")
        .and_then(Value::as_u64)
        .or_else(|| f.get("filesize_approx").and_then(Value::as_u64))
        .or_else(|| match (f.get("tbr").and_then(Value::as_f64), duration) {
            (Some(tbr), Some(d)) => Some((tbr * 1000.0 / 8.0 * d) as u64),
            _ => None,
        })
}

fn codec_name(c: &str) -> &str {
    let c = c.to_lowercase();
    if c.starts_with("avc") || c.starts_with("h264") {
        "H.264"
    } else if c.starts_with("vp9") || c.starts_with("vp09") {
        "VP9"
    } else if c.starts_with("av01") || c.starts_with("av1") {
        "AV1"
    } else if c.starts_with("hev") || c.starts_with("hvc") || c.starts_with("h265") {
        "H.265"
    } else {
        ""
    }
}

/// Turn yt-dlp's raw format list into a short, readable menu.
pub fn build_choices(formats: &[Value], duration: Option<f64>, have_ffmpeg: bool) -> Vec<Choice> {
    let mut out = Vec::new();
    // best audio size, added to video-only streams
    let best_audio = formats
        .iter()
        .filter(|f| {
            f.get("vcodec").and_then(Value::as_str) == Some("none")
                && f.get("acodec")
                    .and_then(Value::as_str)
                    .is_some_and(|a| a != "none")
        })
        .max_by(|a, b| {
            a.get("abr")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .total_cmp(&b.get("abr").and_then(Value::as_f64).unwrap_or(0.0))
        });
    let audio_size = best_audio.and_then(|f| fmt_size_of(f, duration));

    let mut heights: Vec<u64> = formats
        .iter()
        .filter_map(|f| f.get("height").and_then(Value::as_u64))
        .filter(|h| *h >= 144)
        .collect();
    heights.sort_unstable();
    heights.dedup();
    heights.reverse();
    let wanted: Vec<u64> = heights
        .iter()
        .copied()
        .filter(|h| [4320, 2160, 1440, 1080, 720, 480, 360, 240].contains(h))
        .collect();
    let wanted = if wanted.is_empty() {
        heights.iter().copied().take(4).collect()
    } else {
        wanted
    };
    let recommended_h = wanted.iter().copied().find(|h| *h <= 1080);
    for h in wanted.iter().copied().take(5) {
        let at_h: Vec<&Value> = formats
            .iter()
            .filter(|f| f.get("height").and_then(Value::as_u64) == Some(h))
            .collect();
        let combined: Vec<&&Value> = at_h
            .iter()
            .filter(|f| {
                f.get("acodec")
                    .and_then(Value::as_str)
                    .is_some_and(|a| a != "none")
            })
            .collect();
        if !have_ffmpeg && combined.is_empty() {
            continue; // cannot merge video + audio without ffmpeg
        }
        let best = at_h.iter().max_by(|a, b| {
            a.get("tbr")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .total_cmp(&b.get("tbr").and_then(Value::as_f64).unwrap_or(0.0))
        });
        let Some(best) = best else { continue };
        let video_only = best.get("acodec").and_then(Value::as_str) == Some("none");
        let size = fmt_size_of(best, duration).map(|s| {
            s + if video_only {
                audio_size.unwrap_or(0)
            } else {
                0
            }
        });
        let fps = best
            .get("fps")
            .and_then(Value::as_f64)
            .map(|f| f.round() as u64);
        let codec = codec_name(best.get("vcodec").and_then(Value::as_str).unwrap_or(""));
        let ext = if have_ffmpeg && video_only {
            (if codec == "H.264" { "MP4" } else { "MKV" }).to_string()
        } else {
            best.get("ext")
                .and_then(Value::as_str)
                .unwrap_or("mp4")
                .to_uppercase()
        };
        let mut detail = Vec::new();
        if !codec.is_empty() {
            detail.push(codec.to_string());
        }
        if let Some(f) = fps
            && f > 0
        {
            detail.push(format!("{f} fps"));
        }
        let rec = Some(h) == recommended_h;
        if rec {
            detail.push("recommended".into());
        }
        let label = match h {
            4320 => "4320p 8K".into(),
            2160 => "2160p 4K".into(),
            1440 => "1440p 2K".into(),
            _ => format!("{h}p"),
        };
        out.push(Choice {
            gold: false,
            id: format!("v{h}"),
            group: "video".into(),
            ext,
            label,
            detail: detail.join(" · "),
            size,
            recommended: rec,
            opts: MediaOpts {
                quality: h.to_string(),
                ..MediaOpts::default()
            },
        });
    }
    // audio: MP3 at three qualities (size = bitrate x duration), then the original stream
    if have_ffmpeg {
        for (kbps, label, note) in [
            (320u32, "MP3 · High", "320 kbps · best sound"),
            (192, "MP3 · Standard", "192 kbps · recommended"),
            (128, "MP3 · Small", "128 kbps · smallest file"),
        ] {
            out.push(Choice {
                gold: false,
                id: format!("mp3-{kbps}"),
                group: "audio".into(),
                ext: "MP3".into(),
                label: label.into(),
                detail: note.into(),
                size: duration.map(|d| (kbps as f64 * 1000.0 / 8.0 * d) as u64),
                recommended: kbps == 192,
                opts: MediaOpts {
                    quality: "audio".into(),
                    audio_format: "mp3".into(),
                    audio_kbps: kbps,
                    ..MediaOpts::default()
                },
            });
        }
    }
    if let Some(a) = best_audio {
        let ext = a
            .get("ext")
            .and_then(Value::as_str)
            .unwrap_or("m4a")
            .to_uppercase();
        let abr = a
            .get("abr")
            .and_then(Value::as_f64)
            .map(|v| format!("{} kbps · ", v.round()))
            .unwrap_or_default();
        out.push(Choice {
            gold: false,
            id: "audio-original".into(),
            group: "audio".into(),
            ext: ext.clone(),
            label: "Original audio".into(),
            detail: format!("{abr}no conversion"),
            size: audio_size,
            recommended: !have_ffmpeg,
            opts: MediaOpts {
                quality: "audio".into(),
                audio_format: "original".into(),
                audio_kbps: 0,
                ..MediaOpts::default()
            },
        });
    }
    out
}

/// yt-dlp -f selector and extra arguments for the options.
pub fn format_args(o: &MediaOpts, have_ffmpeg: bool) -> Vec<String> {
    let mut a = Vec::new();
    if let Some(id) = &o.format_id {
        a.extend(["-f".to_string(), id.clone()]);
        return a;
    }
    if o.quality == "audio" {
        if o.audio_format == "original" || !have_ffmpeg {
            a.extend(["-f".into(), "ba[ext=m4a]/ba/b".into()]);
            return a;
        }
        a.extend([
            "-f".into(),
            "ba/b".into(),
            "-x".into(),
            "--audio-format".into(),
            o.audio_format.clone(),
        ]);
        if o.audio_format == "mp3" || o.audio_format == "opus" || o.audio_format == "m4a" {
            a.extend([
                "--audio-quality".into(),
                format!("{}K", o.audio_kbps.max(64)),
            ]);
        }
        a.push("--embed-metadata".into());
        return a;
    }
    let cap = if o.quality == "best" || o.quality.is_empty() {
        String::new()
    } else {
        format!("[height<={}]", o.quality)
    };
    if have_ffmpeg {
        a.extend([
            "-f".into(),
            format!("bv*{cap}+ba/b{cap}/b"),
            "--merge-output-format".into(),
            "mp4/mkv".into(),
        ]);
    } else {
        a.extend(["-f".into(), format!("b{cap}/b")]);
    }
    a
}

/// Suffix that keeps different qualities of the same video apart (" 128k", " 720p"); empty for the defaults.
pub fn quality_tag(o: &MediaOpts, have_ffmpeg: bool) -> String {
    if o.quality == "audio" {
        if have_ffmpeg
            && o.audio_format != "original"
            && o.audio_format != "flac"
            && o.audio_kbps != 192
        {
            return format!(" {}k", o.audio_kbps);
        }
        return String::new();
    }
    if o.quality == "best" || o.quality.is_empty() {
        String::new()
    } else {
        format!(" {}p", o.quality)
    }
}

/// Cover art + lyrics are added only when the user enabled them in Settings,
/// for a single MP3 at any quality (not playlists, not exact format ids).
pub fn wants_music_tags(o: &MediaOpts, enabled: bool) -> bool {
    enabled
        && o.quality == "audio"
        && o.audio_format == "mp3"
        && !o.playlist
        && o.format_id.is_none()
}

pub fn clean_error(stderr: &str) -> String {
    let line = stderr
        .lines()
        .rev()
        .find(|l| l.starts_with("ERROR:"))
        .or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
        .unwrap_or("yt-dlp failed")
        .trim_start_matches("ERROR:")
        .trim();
    // drop "[youtube] abc123: "
    let line = if line.starts_with('[') {
        line.split_once(": ").map(|x| x.1).unwrap_or(line)
    } else {
        line
    };
    if line.contains("Requested format is not available") {
        return if ffmpeg_path().is_none() {
            "This video has no single-file version at this quality. Install ffmpeg (it joins video and audio) or choose another quality.".into()
        } else {
            "This quality is not available for this video. Choose another quality.".into()
        };
    }
    if line.contains("Sign in to confirm")
        || (line.to_lowercase().contains("age") && line.to_lowercase().contains("restrict"))
    {
        return format!("{line} Tip: Settings → Video & audio → Use cookies from your browser.");
    }
    if line.to_lowercase().contains("ffmpeg") && line.to_lowercase().contains("not") {
        return "ffmpeg is needed for this download (MP3 conversion or high quality). Install ffmpeg or pick “Original audio”.".into();
    }
    line.chars().take(300).collect()
}

/// Download with yt-dlp on the download's thread, reading its progress lines.
pub fn run(dl: &DlRef, mgr: &Manager) {
    let Some(yt) = ytdlp_path() else {
        let mut d = lock(dl);
        d.status = Status::Error;
        d.error = "yt-dlp is not installed. Settings → Video & audio → Install.".into();
        return;
    };
    let (url, headers, opts, dir, stop) = {
        let d = lock(dl);
        let Kind::Media(o) = &d.kind else { return };
        (
            d.url.clone(),
            d.headers.clone(),
            o.clone(),
            d.save_dir.clone(),
            d.rt.stop.clone(),
        )
    };
    let _ = std::fs::create_dir_all(&dir);
    let have_ffmpeg = ffmpeg_path().is_some();
    if stop.load(Ordering::SeqCst) {
        lock(dl).status = Status::Stopped;
        return;
    }
    let mut args = base_args(mgr, &headers);
    args.extend(format_args(&opts, have_ffmpeg));
    args.extend([
        "--newline".into(), "--progress".into(), "--no-simulate".into(), "--windows-filenames".into(),
        "--continue".into(), "--retries".into(), "10".into(), "--fragment-retries".into(), "10".into(),
        "--socket-timeout".into(), "20".into(), "-N".into(), "4".into(),
        "-P".into(), dir.to_string_lossy().into_owned(),
        "--progress-template".into(),
        "download:OTMDL\t%(progress.downloaded_bytes)s\t%(progress.total_bytes)s\t%(progress.total_bytes_estimate)s\t%(progress.speed)s\t%(progress.eta)s\t%(info.vcodec)s\t%(info.acodec)s\t%(info.playlist_index)s\t%(info.n_entries)s\t%(progress.tmpfilename)s".into(),
        "--progress-template".into(),
        "postprocess:OTMPP\t%(progress.postprocessor)s\t%(progress.status)s".into(),
        "--print".into(), "video:OTMTITLE\t%(title)s".into(),
        "--print".into(), "after_move:OTMFILE\t%(filepath)s".into(),
    ]);
    let tag = quality_tag(&opts, have_ffmpeg);
    if opts.playlist {
        args.extend(["--yes-playlist".into(), "-o".into(), format!("%(playlist_title|Playlist).80B/%(playlist_index|0)03d - %(title).150B [%(id)s]{tag}.%(ext)s")]);
    } else {
        args.extend([
            "--no-playlist".into(),
            "-o".into(),
            format!("%(title).150B [%(id)s]{tag}.%(ext)s"),
        ]);
    }
    let limit = mgr.global_limit_bytes();
    if limit > 0 {
        args.extend(["-r".into(), format!("{}K", (limit / 1024).max(16))]);
    }
    args.push("--".into());
    args.push(url.clone());

    let child = command(&yt)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            let mut d = lock(dl);
            d.status = Status::Error;
            d.error = format!("Cannot run yt-dlp: {e}");
            return;
        }
    };
    let (tx, rx) = mpsc::channel::<(bool, String)>();
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let tx2 = tx.clone();
    std::thread::spawn(move || {
        for l in BufReader::new(out).lines().map_while(Result::ok) {
            let _ = tx.send((false, l));
        }
    });
    std::thread::spawn(move || {
        for l in BufReader::new(err).lines().map_while(Result::ok) {
            let _ = tx2.send((true, l));
        }
    });

    let mut stderr_text = String::new();
    let mut finals: Vec<PathBuf> = Vec::new();
    loop {
        if stop.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            lock(dl).status = Status::Stopped;
            return;
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok((is_err, line)) => {
                if is_err {
                    stderr_text.push_str(&line);
                    stderr_text.push('\n');
                    continue;
                }
                handle_line(dl, &line, &mut finals);
                mgr.repaint();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(Some(_)) = child.try_wait() {
                    // drain what is left
                    while let Ok((is_err, line)) = rx.recv_timeout(Duration::from_millis(100)) {
                        if is_err {
                            stderr_text.push_str(&line);
                            stderr_text.push('\n');
                        } else {
                            handle_line(dl, &line, &mut finals);
                        }
                    }
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait().ok();
    if stop.load(Ordering::SeqCst) {
        lock(dl).status = Status::Stopped;
        return;
    }
    let mut enrichment_note = String::new();
    let mut enriched = false;
    if status.as_ref().is_some_and(|s| s.success())
        && have_ffmpeg
        && wants_music_tags(&opts, mgr.settings().music_tags)
    {
        lock(dl).phase = "Recherche de la pochette et des paroles".into();
        mgr.repaint();
        enrichment_note = "Pochette / paroles introuvables".into();
        if let Ok(metadata) = probe_json(mgr, &url, &headers)
            && let Some(song) = crate::music::resolve(&metadata)
            && !stop.load(Ordering::SeqCst)
        {
            let (mime, bytes) = crate::music::cover(mgr, &song)
                .or_else(|error| {
                    let Some(thumbnail) = metadata["thumbnail"]
                        .as_str()
                        .filter(|u| u.starts_with("https://"))
                    else {
                        return Err(error);
                    };
                    let mut fallback = song.clone();
                    fallback.cover_url = thumbnail.into();
                    crate::music::cover(mgr, &fallback)
                })
                .unwrap_or_default();
            if !stop.load(Ordering::SeqCst)
                && (!song.lyrics.is_empty() || !bytes.is_empty())
                && let Some(file) = finals.last()
            {
                match crate::music::embed(file, &song, &mime, &bytes) {
                    Ok(()) => {
                        enriched = !song.lyrics.is_empty() && !bytes.is_empty();
                        enrichment_note = match (song.lyrics.is_empty(), bytes.is_empty()) {
                            (false, false) => "Pochette + paroles intégrées",
                            (false, true) => "Paroles intégrées · pochette indisponible",
                            _ => "Pochette intégrée · paroles indisponibles",
                        }
                        .into();
                    }
                    Err(_) => {
                        enrichment_note =
                            "Audio téléchargé · intégration des tags impossible".into()
                    }
                }
            }
        }
    }
    let mut d = lock(dl);
    if let Kind::Media(o) = &mut d.kind {
        o.music_enriched = enriched;
    }
    if stop.load(Ordering::SeqCst) {
        d.status = Status::Stopped;
        return;
    }
    if status.is_some_and(|s| s.success()) {
        if let Some(fp) = finals.last() {
            d.size = std::fs::metadata(fp)
                .ok()
                .map(|m| m.len())
                .or(d.media_total);
            if finals.len() == 1 {
                d.file_name = fp
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                d.final_path = Some(fp.clone());
            } else {
                d.final_path = fp.parent().map(|p| p.to_path_buf());
                d.file_name = format!(
                    "{} ({} files)",
                    if d.title.is_empty() {
                        "Playlist"
                    } else {
                        &d.title
                    },
                    finals.len()
                );
            }
        }
        d.status = Status::Complete;
        d.phase = enrichment_note;
        d.tmp_files.clear();
        d.error.clear();
    } else {
        d.status = Status::Error;
        d.error = clean_error(&stderr_text);
    }
}

fn opt_num(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

fn handle_line(dl: &DlRef, line: &str, finals: &mut Vec<PathBuf>) {
    let parts: Vec<&str> = line.split('\t').collect();
    let mut d = lock(dl);
    match parts.first().copied() {
        Some("OTMTITLE") if parts.len() >= 2 => {
            d.title = parts[1].to_string();
            if d.file_name.is_empty() || d.file_name == d.url {
                d.file_name = parts[1].to_string();
            }
        }
        Some("OTMDL") if parts.len() >= 11 => {
            d.status = Status::Downloading;
            d.media_done = opt_num(parts[1]).unwrap_or(0.0) as u64;
            d.media_total = opt_num(parts[2])
                .or_else(|| opt_num(parts[3]))
                .map(|v| v as u64);
            d.media_speed = opt_num(parts[4]);
            d.media_eta = opt_num(parts[5]);
            let (v, a) = (parts[6], parts[7]);
            let mut phase = Vec::new();
            if let (Some(i), Some(n)) = (opt_num(parts[8]), opt_num(parts[9])) {
                phase.push(format!("item {}/{}", i as u64, n as u64));
            }
            if v == "none" && a != "none" && a != "NA" {
                phase.push("audio".to_string());
            } else if a == "none" && v != "none" && v != "NA" {
                phase.push("video".to_string());
            }
            d.phase = phase.join(" ");
            let tmp = PathBuf::from(parts[10..].join("\t"));
            if !tmp.as_os_str().is_empty() && !d.tmp_files.contains(&tmp) {
                d.tmp_files.push(tmp.clone());
                d.tmp_files
                    .push(PathBuf::from(format!("{}.ytdl", tmp.display())));
            }
        }
        Some("OTMPP") if parts.len() >= 3 => {
            if parts[2] == "started" {
                let p = parts[1];
                d.phase = if p.contains("Merger") {
                    "merging".into()
                } else if p.contains("ExtractAudio") {
                    "converting to audio".into()
                } else {
                    "finishing".into()
                };
                d.media_speed = None;
                d.media_eta = None;
            }
        }
        Some("OTMFILE") if parts.len() >= 2 => {
            finals.push(PathBuf::from(parts[1..].join("\t")));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn music_tags_are_opt_in_and_work_for_every_mp3_quality() {
        let mut opts = MediaOpts {
            quality: "audio".into(),
            audio_kbps: 320,
            ..MediaOpts::default()
        };
        // off by default: never contacts third-party services
        assert!(!crate::model::Settings::default().music_tags);
        assert!(!wants_music_tags(&opts, false));
        for kbps in [320, 192, 128] {
            opts.audio_kbps = kbps;
            assert!(wants_music_tags(&opts, true), "{kbps} kbps");
        }
        opts.quality = "best".into();
        assert!(!wants_music_tags(&opts, true));
        opts.quality = "audio".into();
        opts.audio_format = "original".into();
        assert!(!wants_music_tags(&opts, true));
        opts.audio_format = "mp3".into();
        opts.playlist = true;
        assert!(!wants_music_tags(&opts, true));
        opts.playlist = false;
        opts.format_id = Some("140".into());
        assert!(!wants_music_tags(&opts, true));
    }

    use serde_json::json;

    #[test]
    fn detection() {
        assert!(looks_like_media_page("https://www.youtube.com/watch?v=abc"));
        assert!(looks_like_media_page("https://youtu.be/abc"));
        assert!(looks_like_media_page("https://www.tiktok.com/@a/video/1"));
        assert!(!looks_like_media_page("https://example.com/file.zip"));
        assert!(!looks_like_media_page("https://x.com/files/archive.zip"));
    }

    #[test]
    fn format_selectors() {
        let o = MediaOpts {
            quality: "720".into(),
            ..MediaOpts::default()
        };
        assert_eq!(
            format_args(&o, true)[1],
            "bv*[height<=720]+ba/b[height<=720]/b"
        );
        assert_eq!(format_args(&o, false)[1], "b[height<=720]/b");
        let mp3 = MediaOpts {
            quality: "audio".into(),
            audio_format: "mp3".into(),
            audio_kbps: 320,
            ..MediaOpts::default()
        };
        let a = format_args(&mp3, true);
        assert!(
            a.contains(&"-x".to_string())
                && a.contains(&"mp3".to_string())
                && a.contains(&"320K".to_string())
        );
    }

    #[test]
    fn choices_from_formats() {
        let formats = vec![
            json!({"format_id":"18","height":360,"vcodec":"avc1","acodec":"mp4a","ext":"mp4","tbr":500.0}),
            json!({"format_id":"137","height":1080,"vcodec":"avc1.640028","acodec":"none","ext":"mp4","fps":30,"filesize":400_000_000u64}),
            json!({"format_id":"313","height":2160,"vcodec":"vp9","acodec":"none","ext":"webm","fps":60,"filesize":1_400_000_000u64}),
            json!({"format_id":"140","vcodec":"none","acodec":"mp4a","ext":"m4a","abr":128.0,"filesize":10_000_000u64}),
        ];
        let c = build_choices(&formats, Some(634.0), true);
        let ids: Vec<&str> = c.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "v2160",
                "v1080",
                "v360",
                "mp3-320",
                "mp3-192",
                "mp3-128",
                "audio-original"
            ]
        );
        assert_eq!(c[1].size, Some(410_000_000));
        assert!(c[1].recommended);
        assert_eq!(c[4].size, Some(15_216_000)); // 192 kbps x 634 s
        // without ffmpeg: only formats that already contain audio, no MP3
        let c = build_choices(&formats, Some(634.0), false);
        let ids: Vec<&str> = c.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["v360", "audio-original"]);
    }

    #[test]
    fn quality_tags_keep_files_apart() {
        let mut o = MediaOpts {
            quality: "audio".into(),
            audio_format: "mp3".into(),
            audio_kbps: 128,
            ..MediaOpts::default()
        };
        assert_eq!(quality_tag(&o, true), " 128k");
        o.audio_kbps = 192;
        assert_eq!(quality_tag(&o, true), "");
        let v = MediaOpts {
            quality: "720".into(),
            ..MediaOpts::default()
        };
        assert_eq!(quality_tag(&v, true), " 720p");
        assert_eq!(quality_tag(&MediaOpts::default(), true), "");
    }

    #[test]
    fn errors_are_readable() {
        assert_eq!(
            clean_error("ERROR: [youtube] abc: Video unavailable"),
            "Video unavailable"
        );
    }
}
