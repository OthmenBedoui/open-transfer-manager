"""Download engine: segmented multi-connection HTTP(S) downloads with resume, categories,
a queue, a scheduler and a speed limiter. Standard library only.

Threading model: each running download owns one controller thread plus one thread per
connection. Anything that changes the download list from another thread (the local API,
the browser bridge) goes through Manager.call(), which runs it on the thread that calls
Manager.tick() (the UI thread, or the headless loop).
"""

import fnmatch
import json
import os
import queue as queue_mod
import re
import shutil
import ssl
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from collections import deque

from .. import __version__, paths
from . import defaults

USER_AGENT = "Mozilla/5.0 (X11; Linux x86_64) OpenTransferManager/" + __version__
CHUNK = 64 * 1024
PART_EXT = ".otmpart"

# statuses
STOPPED = "Stopped"
QUEUED = "Queued"
CONNECTING = "Connecting"
DOWNLOADING = "Downloading"
COMPLETE = "Complete"
ERROR = "Error"


class CancelledError(Exception):
    """A running call was abandoned (the manager is shutting down)."""


# ---------------------------------------------------------------- formatting

def fmt_size(n):
    if n is None:
        return ""
    n = float(n)
    if n < 1024:
        return "%d B" % n
    for unit in ("KB", "MB", "GB", "TB"):
        n /= 1024
        if n < 1024 or unit == "TB":
            return "%.1f %s" % (n, unit)
    return ""  # pragma: no cover


def fmt_rate(bps):
    if bps is None:
        return ""
    return fmt_size(bps) + "/s"


def fmt_eta(sec):
    if sec is None:
        return ""
    sec = int(sec)
    if sec < 60:
        return "%ds" % sec
    if sec < 3600:
        return "%dm %02ds" % (sec // 60, sec % 60)
    if sec < 86400:
        return "%dh %02dm" % (sec // 3600, (sec % 3600) // 60)
    return "%dd %dh" % (sec // 86400, (sec % 86400) // 3600)


def fmt_date(ts):
    if not ts:
        return ""
    return time.strftime("%Y-%m-%d %H:%M", time.localtime(ts))


# ---------------------------------------------------------------- limiter

class RateLimiter:
    """Token bucket. rate is in bytes per second; 0 means unlimited."""

    def __init__(self, rate=0):
        self.rate = rate
        self.tokens = 0.0
        self.last = time.monotonic()
        self.lock = threading.Lock()

    def set_rate(self, rate):
        with self.lock:
            self.rate = max(0, int(rate))
            self.tokens = 0.0
            self.last = time.monotonic()

    def consume(self, n, stop=None):
        with self.lock:
            rate = self.rate
            if rate <= 0:
                return
            now = time.monotonic()
            self.tokens = min(float(rate), self.tokens + (now - self.last) * rate)
            self.last = now
            self.tokens -= n
            wait = -self.tokens / rate if self.tokens < 0 else 0.0
        while wait > 0:
            if stop is not None and stop():
                return
            step = min(wait, 0.2)
            time.sleep(step)
            wait -= step


# ---------------------------------------------------------------- categories

class Category:
    def __init__(self, name, exts, folder):
        self.name = name
        self.exts = exts
        self.folder = folder

    def matches(self, filename):
        ext = os.path.splitext(filename)[1].lstrip(".").lower()
        if not ext:
            return False
        return any(fnmatch.fnmatchcase(ext, pat) for pat in self.exts.lower().split())

    def to_json(self):
        return {"name": self.name, "exts": self.exts, "folder": self.folder}


# ---------------------------------------------------------------- http helpers

_ssl_ctx = ssl.create_default_context()


def open_url(url, start=None, end=None, timeout=defaults.CONNECTION_TIMEOUT, headers=None):
    h = {"User-Agent": USER_AGENT, "Accept": "*/*", "Accept-Encoding": "identity"}
    for k, v in (headers or {}).items():
        if v:
            h[k] = v
    if start is not None:
        h["Range"] = "bytes=%d-%s" % (start, "" if end is None else str(end))
    req = urllib.request.Request(url, headers=h)
    return urllib.request.urlopen(req, timeout=timeout, context=_ssl_ctx)


def name_from_response(resp, url):
    cd = resp.headers.get("Content-Disposition") or ""
    m = re.search(r"filename\*\s*=\s*[^']*'[^']*'([^;]+)", cd)
    if m:
        return safe_name(urllib.parse.unquote(m.group(1).strip().strip('"')))
    m = re.search(r'filename\s*=\s*"?([^";]+)"?', cd)
    if m:
        return safe_name(m.group(1).strip())
    return name_from_url(resp.geturl() or url)


def name_from_url(url):
    path = urllib.parse.urlparse(url).path
    name = urllib.parse.unquote(path.rstrip("/").split("/")[-1]) if path else ""
    return safe_name(name or "download")


def safe_name(name):
    name = re.sub(r'[\\/:*?"<>|\x00-\x1f]', "_", name).strip(" .")
    return name[:240] or "download"


def parse_total(resp):
    """Returns (total size or None, byte ranges supported)."""
    if resp.status == 206:
        m = re.search(r"/(\d+)", resp.headers.get("Content-Range") or "")
        if m:
            return int(m.group(1)), True
        return None, False
    cl = resp.headers.get("Content-Length")
    return (int(cl) if cl and cl.isdigit() else None), False


def probe(url, headers=None):
    """Ask the server about a file without downloading it."""
    resp = open_url(url, 0, None, headers=headers)
    try:
        total, resumable = parse_total(resp)
        return {"final_url": resp.geturl(), "size": total, "resumable": resumable,
                "name": name_from_response(resp, url), "type": resp.headers.get("Content-Type", "")}
    finally:
        resp.close()


def friendly_error(e):
    text = str(e) or e.__class__.__name__
    if isinstance(e, urllib.error.HTTPError):
        return "Server answered %d %s" % (e.code, e.reason)
    if isinstance(e, urllib.error.URLError):
        return "Cannot reach server: %s" % (e.reason,)
    if isinstance(e, TimeoutError) or "timed out" in text:
        return "Connection timed out"
    return text


# ---------------------------------------------------------------- download

class Segment:
    __slots__ = ("start", "end", "pos", "busy")

    def __init__(self, start, end, pos=None):
        self.start = start
        self.end = end  # inclusive; None when the size is unknown
        self.pos = start if pos is None else pos
        self.busy = False

    def remaining(self):
        if self.end is None:
            return 1 << 62
        return self.end - self.pos + 1

    def done(self):
        return self.end is not None and self.pos > self.end


_next_id = [1]
_ids_lock = threading.Lock()


def _new_id():
    with _ids_lock:
        n = _next_id[0]
        _next_id[0] += 1
        return n


def _reserve_id(n):
    with _ids_lock:
        _next_id[0] = max(_next_id[0], n + 1)


class Download:
    kind = "http"

    def __init__(self, url, file_name, save_dir, category, description="", headers=None):
        self.id = _new_id()
        self.url = url
        self.headers = dict(headers or {})   # Referer, Cookie, User-Agent from the browser
        self.file_name = file_name
        self.save_dir = save_dir
        self.category = category
        self.description = description
        self.size = None
        self.resumable = None
        self.status = STOPPED
        self.error = ""
        self.segments = []
        self.date_added = time.time()
        self.last_try = None
        self.queue = None
        self.use_limit = False
        self.limit_kb = defaults.SPEED_LIMIT_KB
        self.max_conn = None
        self.retries_left = defaults.RETRIES
        self.auto_name = False   # name guessed from the URL: replace it with the server's name
        # runtime only
        self.limiter = RateLimiter(0)
        self.lock = threading.Lock()
        self.thread = None
        self.stop_flag = False
        self.conn_info = {}
        self.samples = deque()
        self.rate = None
        self.on_finish = None
        self.demo = False

    # --- persistence
    def to_json(self):
        st = self.status if self.status in (COMPLETE, ERROR) else STOPPED
        return {
            "kind": self.kind, "id": self.id, "url": self.url, "headers": self.headers,
            "file_name": self.file_name, "save_dir": self.save_dir, "category": self.category,
            "description": self.description, "size": self.size, "resumable": self.resumable,
            "status": st, "error": self.error,
            "segments": [[s.start, s.end, s.pos] for s in self.segments],
            "date_added": self.date_added, "last_try": self.last_try, "queue": self.queue,
            "use_limit": self.use_limit, "limit_kb": self.limit_kb, "max_conn": self.max_conn,
            "auto_name": self.auto_name,
        }

    def _load_common(self, d):
        self.auto_name = d.get("auto_name", False)
        if d.get("id"):
            self.id = int(d["id"])
            _reserve_id(self.id)
        self.size = d.get("size")
        self.resumable = d.get("resumable")
        self.status = d.get("status", STOPPED)
        self.error = d.get("error", "")
        self.date_added = d.get("date_added", time.time())
        self.last_try = d.get("last_try")
        self.queue = d.get("queue")
        self.use_limit = d.get("use_limit", False)
        self.limit_kb = d.get("limit_kb", defaults.SPEED_LIMIT_KB)
        self.max_conn = d.get("max_conn")

    @staticmethod
    def from_json(d):
        if d.get("kind") == "media":
            from .media import MediaDownload
            return MediaDownload.from_json(d)
        dl = Download(d["url"], d["file_name"], d["save_dir"], d.get("category", defaults.GENERAL),
                      d.get("description", ""), d.get("headers") or {})
        if d.get("referer") and "Referer" not in dl.headers:  # older lists
            dl.headers["Referer"] = d["referer"]
        dl._load_common(d)
        dl.segments = [Segment(a, b, p) for a, b, p in d.get("segments", [])]
        return dl

    # --- facts
    @property
    def referer(self):
        return self.headers.get("Referer", "")

    @property
    def path(self):
        return os.path.join(self.save_dir, self.file_name)

    @property
    def part_path(self):
        return self.path + PART_EXT

    def leftover_files(self):
        """Temporary files to delete when the download is removed."""
        return [self.part_path]

    def downloaded(self):
        with self.lock:
            return sum(s.pos - s.start for s in self.segments)

    def running(self):
        return self.thread is not None and self.thread.is_alive()

    def progress(self):
        if self.status == COMPLETE:
            return 1.0
        if not self.size:
            return None
        return min(1.0, self.downloaded() / self.size)

    def active_connections(self):
        with self.lock:
            return sum(1 for s in self.segments if s.busy)

    def sample_rate(self):
        now = time.monotonic()
        self.samples.append((now, self.downloaded()))
        while self.samples and now - self.samples[0][0] > 3.0:
            self.samples.popleft()
        if not self.running():
            self.rate = None
            return
        if len(self.samples) >= 2 and self.samples[-1][0] > self.samples[0][0]:
            self.rate = (self.samples[-1][1] - self.samples[0][1]) / (self.samples[-1][0] - self.samples[0][0])
        else:
            self.rate = 0.0

    def time_left(self):
        if not self.running() or not self.size or not self.rate:
            return None
        return max(0, (self.size - self.downloaded()) / self.rate)

    def status_text(self):
        if self.status == DOWNLOADING:
            p = self.progress()
            return "%.1f%%" % (p * 100) if p is not None else "Receiving data"
        if self.status == STOPPED and self.size and self.downloaded() > 0:
            return "Paused %.1f%%" % (self.progress() * 100)
        if self.status == STOPPED and self.queue:
            return QUEUED
        if self.status == STOPPED:
            return "Paused"
        return self.status

    def summary(self):
        """Small JSON-friendly view, used by the local API and the browser extension."""
        p = self.progress()
        return {
            "id": self.id, "kind": self.kind, "name": self.file_name, "url": self.url,
            "status": self.status, "status_text": self.status_text(), "error": self.error,
            "progress": p, "size": self.size, "downloaded": self.downloaded(),
            "rate": self.rate, "eta": self.time_left(), "category": self.category,
            "path": self.path, "running": self.running(), "queue": self.queue,
        }

    # --- control
    def start(self, manager):
        if self.running() or self.status == COMPLETE:
            return
        self.stop_flag = False
        self.error = ""
        self.status = CONNECTING
        self.last_try = time.time()
        self.samples.clear()
        self.thread = threading.Thread(target=self._run_wrapper, args=(manager,), daemon=True,
                                       name="otm-dl-%d" % self.id)
        self.thread.start()

    def stop(self):
        self.stop_flag = True

    def update_limit(self):
        self.limiter.set_rate(int(self.limit_kb * 1024) if self.use_limit else 0)

    def reset(self):
        """Forget progress so the next start downloads from scratch."""
        for p in self.leftover_files():
            try:
                os.remove(p)
            except OSError:
                pass
        self.status = STOPPED
        self.segments = []
        self.size = None
        self.error = ""

    # --- worker side
    def _run_wrapper(self, manager):
        try:
            self._run(manager)
        except Exception as e:  # noqa: BLE001 - last line of defence, shown to the user
            self.error = friendly_error(e)
            self.status = ERROR
        for s in self.segments:
            s.busy = False
        self.conn_info.clear()
        manager.save()
        if self.on_finish:
            try:
                self.on_finish(self)
            except Exception:  # noqa: BLE001
                pass

    def _run(self, manager):
        self.update_limit()
        fh = None
        try:
            first = None
            if self.segments and not os.path.exists(self.part_path):
                self.segments = []  # the partial file is gone: start again
            if not self.segments or (self.size is None and not self.resumable):
                first = open_url(self.url, 0, None, headers=self.headers)
                total, resumable = parse_total(first)
                self.size = total
                self.resumable = resumable
                if not self.file_name or self.auto_name:
                    real = name_from_response(first, self.url)
                    if real and real != self.file_name:
                        if os.path.exists(self.part_path):
                            os.remove(self.part_path)
                        self.file_name = real
                        if callable(getattr(manager, "category_name_for", None)) and \
                                self.category == defaults.GENERAL:
                            self.category = manager.category_name_for(real)
                            self.save_dir = manager.folder_for(self.category)
                    self.auto_name = False
                with self.lock:
                    self.segments = [Segment(0, total - 1 if total else None)]
            elif not self.resumable:
                with self.lock:  # the server cannot continue where we stopped
                    self.segments = [Segment(0, self.size - 1 if self.size else None)]
            os.makedirs(self.save_dir, exist_ok=True)
            if self.size:
                free = shutil.disk_usage(self.save_dir).free
                have = os.path.getsize(self.part_path) if os.path.exists(self.part_path) else 0
                if self.size - have > free:
                    raise IOError("Not enough disk space (%s needed, %s free)"
                                  % (fmt_size(self.size - have), fmt_size(free)))
            mode = "r+b" if (os.path.exists(self.part_path) and first is None and self.resumable) else "w+b"
            fh = open(self.part_path, mode)
            if self.size:
                fh.truncate(self.size)
            self.status = DOWNLOADING
            self._transfer(manager, fh, first)
        except Exception as e:  # noqa: BLE001 - shown to the person
            self.error = friendly_error(e)
            self.status = ERROR
        finally:
            if fh:
                fh.close()
        if self.status == DOWNLOADING:
            if self.stop_flag:
                self.status = STOPPED
            elif all(s.done() for s in self.segments) or (
                    self.size is None and not any(s.busy for s in self.segments) and not self.error):
                self._finish()
            else:
                self.status = ERROR
                self.error = self.error or "Connection failed"

    def _finish(self):
        target = self.path
        base, ext = os.path.splitext(target)
        n = 2
        while os.path.exists(target):
            target = "%s (%d)%s" % (base, n, ext)
            n += 1
        if self.size is None:
            with self.lock:
                self.size = sum(s.pos - s.start for s in self.segments)
        os.replace(self.part_path, target)
        self.file_name = os.path.basename(target)
        self.status = COMPLETE

    def _max_conn(self, manager):
        if not self.resumable or not self.size:
            return 1
        return max(1, int(self.max_conn or manager.max_connections_for(self.url)))

    def _take_segment(self):
        """Work for a new connection: an idle unfinished part, else half of the largest busy part."""
        with self.lock:
            for s in self.segments:
                if not s.busy and not s.done():
                    s.busy = True
                    return s
            if not self.resumable or not self.size:
                return None
            busy = [s for s in self.segments if s.busy and not s.done()]
            if not busy:
                return None
            big = max(busy, key=lambda s: s.remaining())
            rem = big.remaining()
            if rem < 2 * defaults.MIN_SPLIT_BYTES:
                return None
            mid = big.pos + rem // 2
            new = Segment(mid, big.end)
            big.end = mid - 1
            new.busy = True
            self.segments.append(new)
            self.segments.sort(key=lambda s: s.start)
            return new

    def _transfer(self, manager, fh, first):
        file_lock = threading.Lock()
        workers = []
        counter = [0]
        stopped = lambda: self.stop_flag  # noqa: E731

        def worker(seg, resp, num):
            failures = 0
            while not self.stop_flag and not seg.done():
                try:
                    if resp is None:
                        self.conn_info[num] = "Connecting"
                        resp = open_url(self.url, seg.pos if self.resumable else None,
                                        seg.end if self.resumable else None, headers=self.headers)
                        if self.resumable and resp.status != 206:
                            raise IOError("Server stopped supporting resume")
                    self.conn_info[num] = "Receiving"
                    while not self.stop_flag:
                        with self.lock:
                            want = CHUNK if seg.end is None else min(CHUNK, seg.end - seg.pos + 1)
                        if want <= 0:
                            break
                        data = resp.read(want)
                        if not data:
                            if seg.end is None:
                                seg.end = seg.pos - 1  # unknown size: done at end of stream
                            break
                        manager.limiter.consume(len(data), stopped)
                        self.limiter.consume(len(data), stopped)
                        with self.lock:
                            room = len(data) if seg.end is None else max(0, seg.end - seg.pos + 1)
                            data = data[:room]
                            at = seg.pos
                            seg.pos += len(data)
                        if data:
                            with file_lock:
                                fh.seek(at)
                                fh.write(data)
                        failures = 0
                    resp.close()
                    resp = None
                    if seg.done() or self.stop_flag:
                        break
                    if seg.end is not None and not seg.done():
                        raise IOError("Connection closed by the server")
                except Exception as e:  # noqa: BLE001
                    if resp is not None:
                        try:
                            resp.close()
                        except Exception:  # noqa: BLE001
                            pass
                        resp = None
                    failures += 1
                    self.error = friendly_error(e)
                    if failures > defaults.RETRIES or not self.resumable:
                        self.conn_info[num] = "Failed"
                        break
                    self.conn_info[num] = "Retrying (%d)" % failures
                    delay = min(15.0, 0.5 * (2 ** (failures - 1)))   # exponential backoff
                    end = time.monotonic() + delay
                    while not self.stop_flag and time.monotonic() < end:
                        time.sleep(0.1)
            self.conn_info[num] = "Done" if seg.done() else "Stopped"
            with self.lock:
                seg.busy = False

        def launch(seg, resp=None):
            counter[0] += 1
            t = threading.Thread(target=worker, args=(seg, resp, counter[0]), daemon=True)
            workers.append(t)
            t.start()

        if first is not None:
            with self.lock:
                seg0 = self.segments[0]
                seg0.busy = True
            launch(seg0, first)

        last_save = time.monotonic()
        while not self.stop_flag:
            alive = sum(1 for t in workers if t.is_alive())
            if all(s.done() for s in self.segments) and alive == 0:
                break
            while alive < self._max_conn(manager):
                seg = self._take_segment()
                if seg is None:
                    break
                launch(seg)
                alive += 1
            if alive == 0:
                if self.size is None or not any(not s.done() for s in self.segments):
                    break
                self.error = self.error or "Connection failed"  # every connection gave up
                break
            if time.monotonic() - last_save > 5:
                with file_lock:
                    fh.flush()
                manager.save()
                last_save = time.monotonic()
            time.sleep(0.15)
        for t in workers:
            t.join(timeout=defaults.CONNECTION_TIMEOUT + 2)


# ---------------------------------------------------------------- manager

SETTINGS = {
    # key: default
    "base_folder": None,
    "max_connections": defaults.MAX_CONNECTIONS,
    "server_connections": {},
    "limit_kb": defaults.SPEED_LIMIT_KB,
    "limiter_on_startup": False,
    "queue_at_once": defaults.QUEUE_AT_ONCE,
    "queue_retries": defaults.RETRIES,
    "schedule": {"start_on": False, "start_at": "02:00", "stop_on": False, "stop_at": "07:00",
                 "last_start": "", "last_stop": ""},
    "theme": "system",                 # system | light | dark
    "notify_on_complete": True,
    "show_details": True,
    "media_quality": "best",
    "media_audio_format": "mp3",
    "media_cookies_browser": "",
    "media_playlist": False,
    "start_minimized": False,
}


class Manager:
    def __init__(self, persist=True):
        self.persist = persist
        self.downloads = []
        self.categories = []
        self.limiter = RateLimiter(0)
        self.limiter_on = False
        self.queue_running = {defaults.MAIN_QUEUE: False}
        self.save_lock = threading.Lock()
        self.commands = queue_mod.Queue()
        self.on_added = None        # callback(download) on the tick thread
        self.on_show = None         # callback() when another instance asks to show the window
        for k, v in SETTINGS.items():
            setattr(self, k, json.loads(json.dumps(v)))
        self.base_folder = paths.downloads_dir()
        self.load()

    # --- files
    def settings_path(self):
        return os.path.join(paths.data_dir(), "settings.json")

    def list_path(self):
        return os.path.join(paths.data_dir(), "downloads.json")

    def default_categories(self):
        return [Category(n, e, os.path.join(self.base_folder, n)) for n, e in defaults.CATEGORIES]

    def load(self):
        s = {}
        if self.persist:
            s = _read_json(self.settings_path(), {})
        for k in SETTINGS:
            if k in s:
                if isinstance(SETTINGS[k], dict) and isinstance(s[k], dict):
                    getattr(self, k).update(s[k])
                else:
                    setattr(self, k, s[k])
        if not self.base_folder:
            self.base_folder = paths.downloads_dir()
        cats = s.get("categories")
        if cats:
            self.categories = [Category(c["name"], c["exts"], c["folder"]) for c in cats if "name" in c]
        else:
            self.categories = self.default_categories()
        self.set_limiter(self.limiter_on_startup)
        if self.persist:
            items = _read_json(self.list_path(), [])
            out = []
            for d in items if isinstance(items, list) else []:
                try:
                    out.append(Download.from_json(d))
                except (KeyError, TypeError, ValueError):
                    continue  # skip one broken entry instead of losing the whole list
            self.downloads = out

    def save(self):
        if not self.persist:
            return
        with self.save_lock:
            s = {k: getattr(self, k) for k in SETTINGS}
            s["categories"] = [c.to_json() for c in self.categories]
            items = [d.to_json() for d in list(self.downloads) if not d.demo]
            for path, obj in ((self.settings_path(), s), (self.list_path(), items)):
                tmp = path + ".tmp"
                try:
                    with open(tmp, "w", encoding="utf-8") as f:
                        json.dump(obj, f, indent=1)
                        f.flush()
                        os.fsync(f.fileno())
                    if os.path.exists(path):
                        shutil.copyfile(path, path + ".bak")
                    os.replace(tmp, path)
                except OSError:
                    pass

    # --- cross-thread calls
    def call(self, fn, timeout=10):
        """Run fn() on the tick thread and return its result (used by the local API)."""
        if threading.current_thread() is getattr(self, "_tick_thread", None):
            return fn()
        box = {}
        done = threading.Event()

        def run():
            try:
                box["value"] = fn()
            except Exception as e:  # noqa: BLE001
                box["error"] = e
            done.set()

        self.commands.put(run)
        if not done.wait(timeout):
            raise CancelledError("The app did not answer in time")
        if "error" in box:
            raise box["error"]
        return box.get("value")

    # --- rules
    def max_connections_for(self, url):
        host = (urllib.parse.urlparse(url).hostname or "").lower()
        for h, n in self.server_connections.items():
            if host == h.lower() or host.endswith("." + h.lower()):
                return n
        return self.max_connections

    def category_for(self, filename):
        for c in self.categories:
            if c.matches(filename):
                return c
        return None

    def category_name_for(self, filename):
        c = self.category_for(filename)
        return c.name if c else defaults.GENERAL

    def folder_for(self, category_name):
        for c in self.categories:
            if c.name == category_name:
                return c.folder
        return self.base_folder

    def set_limiter(self, on):
        self.limiter_on = bool(on)
        self.limiter.set_rate(int(self.limit_kb * 1024) if self.limiter_on else 0)

    def global_limit_bytes(self):
        return int(self.limit_kb * 1024) if self.limiter_on else 0

    # --- list
    def find(self, dl_id):
        for d in self.downloads:
            if d.id == dl_id:
                return d
        return None

    def add(self, url, file_name, save_dir, category, description="", start=True, queue=None, headers=None):
        dl = Download(url, file_name, save_dir, category, description, headers)
        return self._register(dl, start, queue)

    def add_media(self, url, save_dir=None, category="Video", start=True, queue=None, headers=None,
                  quality=None, audio_format=None, playlist=None, title=""):
        from .media import MediaDownload
        quality = quality or self.media_quality
        if quality == "audio" and category == "Video":
            category = "Music"
        dl = MediaDownload(url, save_dir or self.folder_for(category), category, headers=headers,
                           quality=quality, audio_format=audio_format or self.media_audio_format,
                           playlist=self.media_playlist if playlist is None else playlist, title=title)
        return self._register(dl, start, queue)

    def _register(self, dl, start, queue):
        dl.queue = queue
        self.downloads.append(dl)
        if start:
            dl.start(self)
        self.save()
        if self.on_added:
            self.on_added(dl)
        return dl

    def add_smart(self, url, file_name="", start=True, headers=None, media=None, queue=None, **media_opts):
        """Add a URL from the browser or the command line, choosing media or plain download."""
        from . import media as media_mod
        if media is None:
            media = media_mod.looks_like_media_page(url)
        if media:
            return self.add_media(url, start=start, headers=headers, queue=queue, **media_opts)
        name = safe_name(file_name) if file_name else name_from_url(url)
        cat = self.category_name_for(name)
        dl = Download(url, name, self.folder_for(cat), cat, headers=headers)
        dl.auto_name = not file_name
        return self._register(dl, start, queue)

    def remove(self, dl, delete_file=False):
        dl.stop()
        if dl in self.downloads:
            self.downloads.remove(dl)

        def cleanup():
            if dl.thread:
                dl.thread.join(timeout=30)
            files = list(dl.leftover_files())
            if delete_file and dl.status == COMPLETE:
                files.append(dl.path)
            for p in files:
                try:
                    if p and os.path.isfile(p):
                        os.remove(p)
                except OSError:
                    pass
        threading.Thread(target=cleanup, daemon=True).start()
        self.save()

    def stop_all(self):
        for d in self.downloads:
            d.stop()
        for q in self.queue_running:
            self.queue_running[q] = False

    def start_queue(self, name=defaults.MAIN_QUEUE):
        self.queue_running[name] = True
        for d in self.downloads:
            if d.queue == name and d.status == ERROR:
                d.retries_left = self.queue_retries
                d.status = STOPPED

    def stop_queue(self, name=defaults.MAIN_QUEUE):
        self.queue_running[name] = False
        for d in self.downloads:
            if d.queue == name:
                d.stop()

    def total_rate(self):
        return sum(d.rate or 0 for d in self.downloads if d.running())

    def tick(self):
        """Call about twice a second from one thread: commands, speeds, queue and scheduler."""
        self._tick_thread = threading.current_thread()
        while True:
            try:
                fn = self.commands.get_nowait()
            except queue_mod.Empty:
                break
            fn()
        for d in self.downloads:
            d.sample_rate()
        for name, on in self.queue_running.items():
            if not on:
                continue
            members = [d for d in self.downloads if d.queue == name]
            running = [d for d in members if d.running()]
            waiting = [d for d in members if not d.running() and d.status in (STOPPED, ERROR)
                       and (d.status != ERROR or d.retries_left > 0)]
            for d in waiting[: max(0, self.queue_at_once - len(running))]:
                if d.status == ERROR:
                    d.retries_left -= 1
                d.start(self)
            if not running and not waiting:
                self.queue_running[name] = False
        self._scheduler()

    def _scheduler(self):
        now = time.strftime("%H:%M")
        today = time.strftime("%Y-%m-%d")
        sc = self.schedule
        if sc.get("start_on") and now == sc.get("start_at") and sc.get("last_start") != today:
            sc["last_start"] = today
            self.start_queue(defaults.MAIN_QUEUE)
        if sc.get("stop_on") and now == sc.get("stop_at") and sc.get("last_stop") != today:
            sc["last_stop"] = today
            self.stop_queue(defaults.MAIN_QUEUE)

    def shutdown(self, wait=3.0):
        self.stop_all()
        deadline = time.time() + wait
        for d in self.downloads:
            if d.thread:
                d.thread.join(timeout=max(0.0, deadline - time.time()))
        self.save()


def _read_json(path, fallback):
    for p in (path, path + ".bak"):
        try:
            with open(p, encoding="utf-8") as f:
                return json.load(f)
        except (OSError, ValueError):
            continue
    return fallback
