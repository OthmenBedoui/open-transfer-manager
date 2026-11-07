"""Video and audio from YouTube and social networks, through yt-dlp.

yt-dlp is imported lazily so the app starts fast and still works (plain downloads)
when yt-dlp is missing.
"""

import os
import re
import shutil
import subprocess
import sys
import threading
import time
import urllib.parse

from . import defaults, engine

# Hosts we know are video pages: checked first, no yt-dlp import needed.
MEDIA_HOSTS = (
    "youtube.com", "youtu.be", "music.youtube.com", "tiktok.com", "instagram.com", "facebook.com",
    "fb.watch", "twitter.com", "x.com", "vimeo.com", "dailymotion.com", "dai.ly", "twitch.tv",
    "reddit.com", "v.redd.it", "soundcloud.com", "bandcamp.com", "bilibili.com", "rumble.com",
    "streamable.com", "threads.net", "pinterest.com", "linkedin.com", "snapchat.com", "kick.com",
    "odysee.com", "peertube.tv", "mixcloud.com", "9gag.com", "imgur.com", "vk.com", "ok.ru",
)

_yt = None
_yt_lock = threading.Lock()


def yt_dlp():
    """The yt_dlp module, or None when it is not installed."""
    global _yt
    with _yt_lock:
        if _yt is None:
            try:
                import yt_dlp as mod
                _yt = mod
            except ImportError:
                _yt = False
        return _yt or None


def ytdlp_version():
    mod = yt_dlp()
    if not mod:
        return None
    try:
        return mod.version.__version__
    except AttributeError:
        return "unknown"


def ffmpeg_path():
    return shutil.which("ffmpeg")


def update_ytdlp():
    """Upgrade yt-dlp inside the app's own Python. Returns (ok, output)."""
    cmd = [sys.executable, "-m", "pip", "install", "--upgrade", "--disable-pip-version-check", "yt-dlp"]
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=300)
    except (OSError, subprocess.TimeoutExpired) as e:
        return False, str(e)
    global _yt
    with _yt_lock:
        _yt = None  # import again next time (new version is used after restart)
    return out.returncode == 0, (out.stdout + out.stderr)[-2000:]


def _host(url):
    return (urllib.parse.urlparse(url).hostname or "").lower()


def looks_like_media_page(url):
    """Fast guess: is this a video/audio page rather than a direct file link?"""
    host = _host(url)
    if not host:
        return False
    path = urllib.parse.urlparse(url).path.lower()
    # a direct link to a file is a plain download, even on a media host
    ext = os.path.splitext(path)[1].lstrip(".")
    if ext and any(ext in exts.split() for _n, exts in defaults.CATEGORIES):
        return False
    return any(host == h or host.endswith("." + h) for h in MEDIA_HOSTS)


def is_supported(url):
    """Accurate check with yt-dlp's extractors (slower; call from a thread)."""
    if looks_like_media_page(url):
        return True
    mod = yt_dlp()
    if not mod:
        return False
    try:
        for ie in mod.extractor.gen_extractor_classes():
            if ie.ie_key() == "Generic":
                continue
            if ie.suitable(url):
                return ie.working()
    except Exception:  # noqa: BLE001
        return False
    return False


def format_for(quality, audio_format, have_ffmpeg):
    """yt-dlp format selector and post-processors for a quality key."""
    post = []
    if quality == "audio":
        if have_ffmpeg:
            post.append({"key": "FFmpegExtractAudio", "preferredcodec": audio_format, "preferredquality": "0"})
            return "ba/b", post
        return "ba[ext=m4a]/ba/b", post
    cap = "" if quality in ("best", "", None) else "[height<=%s]" % int(quality)
    if have_ffmpeg:
        return "bv*%s+ba/b%s/b" % (cap, cap), post
    # without ffmpeg we cannot merge separate video and audio streams
    return "b%s/b" % cap, post


def probe(url, cookies_browser="", headers=None):
    """Title, duration, available heights. Network call: run in a thread."""
    mod = yt_dlp()
    if not mod:
        raise RuntimeError("yt-dlp is not installed. Run: otm update-ytdlp")
    opts = {"quiet": True, "no_warnings": True, "skip_download": True, "noplaylist": True,
            "extract_flat": "in_playlist", "logger": _QuietLogger()}
    if cookies_browser:
        opts["cookiesfrombrowser"] = (cookies_browser,)
    if headers:
        opts["http_headers"] = {k: v for k, v in headers.items() if k in ("Referer", "User-Agent") and v}
    with mod.YoutubeDL(opts) as ydl:
        info = ydl.extract_info(url, download=False)
    entries = info.get("entries")
    heights = sorted({f.get("height") for f in info.get("formats") or [] if f.get("height")}, reverse=True)
    return {
        "title": info.get("title") or "",
        "uploader": info.get("uploader") or info.get("channel") or "",
        "duration": info.get("duration"),
        "thumbnail": info.get("thumbnail") or "",
        "heights": heights,
        "is_playlist": entries is not None,
        "count": len(list(entries)) if entries is not None else 1,
        "extractor": info.get("extractor_key") or "",
    }


def _clean_error(e):
    text = str(e)
    if text.startswith("ERROR: "):
        text = text[7:]
    text = re.sub(r"^\[[\w:]+\] [\w-]+: ", "", text.split("\n")[0])  # drop "[youtube] abc123: "
    if "Requested format is not available" in text:
        if not ffmpeg_path():
            return ("This video has no single-file version at this quality. Install ffmpeg "
                    "(needed to join video and audio), or choose another quality.")
        return "This quality is not available for this video. Choose another quality."
    if "Sign in to confirm" in text or ("age" in text.lower() and "restrict" in text.lower()):
        return text[:200] + " Tip: Settings → Video & audio → Use cookies from your browser."
    if "ffmpeg" in text.lower() and "not" in text.lower():
        return "ffmpeg is needed for this download. Install ffmpeg, or choose another quality."
    return text[:300]


class _QuietLogger:
    """Keeps yt-dlp from printing to the terminal; errors are shown in the app instead."""

    def debug(self, msg):
        pass

    def info(self, msg):
        pass

    def warning(self, msg):
        pass

    def error(self, msg):
        pass


class MediaDownload(engine.Download):
    kind = "media"

    def __init__(self, url, save_dir, category="Video", description="", headers=None,
                 quality="best", audio_format="mp3", playlist=False, title=""):
        super().__init__(url, title or url, save_dir, category, description, headers)
        self.quality = quality
        self.audio_format = audio_format
        self.playlist = playlist
        self.title = title
        self.resumable = True
        self.final_path = ""
        self.media_done = 0
        self.media_total = None
        self.media_speed = None
        self.media_eta = None
        self.phase = ""
        self.tmp_files = set()

    # --- persistence
    def to_json(self):
        d = super().to_json()
        d.update(quality=self.quality, audio_format=self.audio_format, playlist=self.playlist,
                 title=self.title, final_path=self.final_path, media_done=self.media_done,
                 media_total=self.media_total, tmp_files=sorted(self.tmp_files))
        d["segments"] = []
        return d

    @classmethod
    def from_json(cls, d):
        dl = cls(d["url"], d["save_dir"], d.get("category", "Video"), d.get("description", ""),
                 d.get("headers") or {}, d.get("quality", "best"), d.get("audio_format", "mp3"),
                 d.get("playlist", False), d.get("title", ""))
        dl._load_common(d)
        dl.file_name = d.get("file_name") or dl.file_name
        dl.final_path = d.get("final_path", "")
        dl.media_done = d.get("media_done", 0)
        dl.media_total = d.get("media_total")
        dl.tmp_files = set(d.get("tmp_files", []))
        return dl

    # --- facts
    @property
    def path(self):
        return self.final_path or os.path.join(self.save_dir, self.file_name)

    @property
    def part_path(self):
        return ""

    def leftover_files(self):
        return [p for p in self.tmp_files if p and (p.endswith(".part") or ".part-Frag" in p or p.endswith(".ytdl"))]

    def downloaded(self):
        if self.status == engine.COMPLETE:
            return self.size or self.media_done
        return self.media_done

    def progress(self):
        if self.status == engine.COMPLETE:
            return 1.0
        if not self.media_total:
            return None
        return min(1.0, self.media_done / self.media_total)

    def active_connections(self):
        return 1 if self.running() else 0

    def sample_rate(self):
        self.rate = self.media_speed if self.running() else None

    def time_left(self):
        return self.media_eta if self.running() else None

    def status_text(self):
        if self.status == engine.DOWNLOADING:
            p = self.progress()
            base = ("%.1f%%" % (p * 100)) if p is not None else "Downloading"
            return (base + " · " + self.phase) if self.phase else base
        if self.status == engine.CONNECTING:
            return "Fetching info"
        return super().status_text()

    def reset(self):
        super().reset()
        self.final_path = ""
        self.media_done = 0
        self.media_total = None
        self.tmp_files.clear()

    # --- worker
    def _opts(self, manager):
        have_ffmpeg = bool(ffmpeg_path())
        fmt, post = format_for(self.quality, self.audio_format, have_ffmpeg)
        opts = {
            "format": fmt,
            "postprocessors": post,
            "paths": {"home": self.save_dir},
            "outtmpl": "%(title).150B [%(id)s].%(ext)s",
            "noplaylist": not self.playlist,
            "continuedl": True,
            "quiet": True,
            "no_warnings": True,
            "noprogress": True,
            "logger": _QuietLogger(),
            "retries": defaults.RETRIES,
            "fragment_retries": defaults.RETRIES,
            "socket_timeout": defaults.CONNECTION_TIMEOUT,
            "concurrent_fragment_downloads": 4,
            "progress_hooks": [self._hook],
            "postprocessor_hooks": [self._post_hook],
            "windowsfilenames": True,
        }
        if self.playlist:
            opts["outtmpl"] = "%(playlist_title|Playlist).80B/%(playlist_index|0)03d - %(title).150B [%(id)s].%(ext)s"
        limits = [x for x in (manager.global_limit_bytes(), int(self.limit_kb * 1024) if self.use_limit else 0) if x]
        if limits:
            opts["ratelimit"] = min(limits)
        if manager.media_cookies_browser:
            opts["cookiesfrombrowser"] = (manager.media_cookies_browser,)
        hdr = {k: v for k, v in self.headers.items() if k in ("Referer", "User-Agent") and v}
        if hdr:
            opts["http_headers"] = hdr
        if have_ffmpeg:
            opts["ffmpeg_location"] = ffmpeg_path()
        return opts

    def _hook(self, d):
        if self.stop_flag:
            mod = yt_dlp()
            raise mod.utils.DownloadCancelled("Stopped")
        st = d.get("status")
        tmp = d.get("tmpfilename")
        if tmp:
            self.tmp_files.add(tmp)
            self.tmp_files.add(tmp + ".ytdl")
        info = d.get("info_dict") or {}
        if info.get("title") and (not self.title or self.file_name == self.url):
            self.title = info["title"]
            self.file_name = os.path.basename(d.get("filename") or self.title)
        if st == "downloading":
            self.status = engine.DOWNLOADING
            self.media_done = d.get("downloaded_bytes") or 0
            self.media_total = d.get("total_bytes") or d.get("total_bytes_estimate")
            self.size = self.media_total
            self.media_speed = d.get("speed")
            self.media_eta = d.get("eta")
            parts = []
            if info.get("playlist_index") and info.get("n_entries"):
                parts.append("item %s/%s" % (info["playlist_index"], info["n_entries"]))
            vcodec, acodec = info.get("vcodec"), info.get("acodec")
            if vcodec == "none" and acodec not in (None, "none"):
                parts.append("audio")
            elif acodec == "none" and vcodec not in (None, "none"):
                parts.append("video")
            self.phase = " ".join(parts)
        elif st == "finished":
            self.media_done = self.media_total or self.media_done
            if d.get("filename"):
                self.file_name = os.path.basename(d["filename"])

    def _post_hook(self, d):
        if d.get("status") == "started":
            name = d.get("postprocessor") or ""
            self.phase = "merging" if "Merger" in name else ("converting" if "Extract" in name else "processing")
        elif d.get("status") == "finished":
            fp = (d.get("info_dict") or {}).get("filepath")
            if fp:
                self.final_path = fp
                self.file_name = os.path.basename(fp)

    def _run(self, manager):
        mod = yt_dlp()
        if not mod:
            self.error = "yt-dlp is not installed. Run: otm update-ytdlp"
            self.status = engine.ERROR
            return
        os.makedirs(self.save_dir, exist_ok=True)
        self.phase = ""
        try:
            with mod.YoutubeDL(self._opts(manager)) as ydl:
                # extract first (cheap, no format choice) so the list shows the real title early
                raw = ydl.extract_info(self.url, download=False, process=False)
                if raw and raw.get("title") and (not self.title or self.file_name == self.url):
                    self.title = raw["title"]
                    self.file_name = raw["title"]
                if self.stop_flag:
                    self.status = engine.STOPPED
                    return
                info = ydl.process_ie_result(raw, download=True)
            if self.stop_flag:
                self.status = engine.STOPPED
                return
            info = info or {}
            req = (info.get("requested_downloads") or [{}])[0]
            fp = req.get("filepath") or self.final_path
            if fp and os.path.exists(fp):
                self.final_path = fp
                self.file_name = os.path.basename(fp)
                self.size = os.path.getsize(fp)
            elif self.playlist:
                self.file_name = info.get("title") or self.file_name
                self.final_path = os.path.join(self.save_dir, info.get("title") or "")
            self.title = info.get("title") or self.title
            self.status = engine.COMPLETE
            self.phase = ""
            self.tmp_files.clear()
        except mod.utils.DownloadCancelled:
            self.status = engine.STOPPED
        except Exception as e:  # noqa: BLE001 - yt-dlp raises many kinds
            if self.stop_flag:
                self.status = engine.STOPPED
            else:
                self.error = _clean_error(e)
                self.status = engine.ERROR
        finally:
            self.media_speed = None
            self.media_eta = None
            self.last_try = time.time()
