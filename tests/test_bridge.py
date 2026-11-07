"""Local API, native messaging framing and browser registration."""

import io
import json
import os
import tempfile
import threading
import time
import unittest
import urllib.request

import helpers  # noqa: F401  (sets up sys.path and a temp data dir)

from otm import browsers, native_host, server
from otm.core import engine, media


class ApiTest(unittest.TestCase):
    def setUp(self):
        os.environ["OTM_DATA_DIR"] = tempfile.mkdtemp()
        self.mgr = engine.Manager(persist=False)
        self.api = server.ApiServer(self.mgr)
        self.api.start()
        self.stop = threading.Event()
        self.ticker = threading.Thread(target=self._tick, daemon=True)
        self.ticker.start()

    def _tick(self):
        while not self.stop.is_set():
            self.mgr.tick()
            time.sleep(0.02)

    def tearDown(self):
        self.stop.set()
        self.api.stop()
        self.mgr.stop_all()

    def test_ping_and_runtime_file(self):
        self.assertTrue(server.is_running())
        rt = server.read_runtime()
        self.assertEqual(rt["pid"], os.getpid())
        if os.name != "nt":
            self.assertEqual(os.stat(server.runtime_path()).st_mode & 0o777, 0o600)

    def test_token_required(self):
        rt = server.read_runtime()
        req = urllib.request.Request("http://127.0.0.1:%d/api/downloads" % rt["port"])
        with self.assertRaises(urllib.error.HTTPError) as cm:
            urllib.request.build_opener(urllib.request.ProxyHandler({})).open(req, timeout=3)
        self.assertEqual(cm.exception.code, 401)

    def test_bad_host_rejected(self):
        rt = server.read_runtime()
        req = urllib.request.Request("http://127.0.0.1:%d/api/ping" % rt["port"], headers={"Host": "evil.example"})
        with self.assertRaises(urllib.error.HTTPError) as cm:
            urllib.request.build_opener(urllib.request.ProxyHandler({})).open(req, timeout=3)
        self.assertEqual(cm.exception.code, 403)

    def test_add_list_control(self):
        r = server.request("POST", "/api/add", {"url": "https://example.com/files/a.zip", "start": False,
                                                "referer": "https://example.com/"})
        dl_id = r["download"]["id"]
        self.assertEqual(r["download"]["name"], "a.zip")
        self.assertEqual(r["download"]["category"], "Compressed")
        d = self.mgr.find(dl_id)
        self.assertEqual(d.headers.get("Referer"), "https://example.com/")
        lst = server.request("GET", "/api/downloads")
        self.assertIn(dl_id, [x["id"] for x in lst["downloads"]])
        server.request("POST", "/api/control", {"id": dl_id, "action": "remove"})
        self.assertIsNone(self.mgr.find(dl_id))

    def test_media_link_goes_to_ytdlp(self):
        r = server.request("POST", "/api/add", {"url": "https://www.youtube.com/watch?v=dQw4w9WgXcQ", "start": False})
        self.assertEqual(r["download"]["kind"], "media")

    def test_rejects_non_http(self):
        with self.assertRaises(RuntimeError):
            server.request("POST", "/api/add", {"url": "file:///etc/passwd"})


class NativeMessagingTest(unittest.TestCase):
    def test_roundtrip(self):
        msg = {"id": 7, "cmd": "ping", "text": "héllo"}
        data = native_host.encode_message(msg)
        self.assertEqual(native_host.read_message(io.BytesIO(data)), msg)

    def test_eof(self):
        self.assertIsNone(native_host.read_message(io.BytesIO(b"")))

    def test_too_large(self):
        import struct
        with self.assertRaises(ValueError):
            native_host.read_message(io.BytesIO(struct.pack("=I", 10 ** 8)))


class BrowsersTest(unittest.TestCase):
    def test_manifests(self):
        m = browsers.manifest("chromium", "/x/otm-native-host")
        self.assertTrue(m["allowed_origins"][0].startswith("chrome-extension://"))
        self.assertEqual(m["name"], browsers.HOST_NAME)
        f = browsers.manifest("firefox", "/x/otm-native-host")
        self.assertIn("otm@opentransfermanager.org", f["allowed_extensions"])

    @unittest.skipIf(os.name == "nt", "file based registration is for Linux/macOS")
    def test_register_into_fake_home(self):
        home = tempfile.mkdtemp()
        old = os.environ.get("HOME")
        os.environ["HOME"] = home
        os.environ.pop("XDG_CONFIG_HOME", None)
        try:
            firefox_root = [root for name, _f, root, _d in browsers._targets() if name == "Firefox"][0]
            os.makedirs(firefox_root)  # pretend Firefox is installed (path differs on Linux/macOS)
            done = browsers.register()
            names = [n for n, _p in done]
            self.assertIn("Firefox", names)
            path = dict(done)["Firefox"]
            with open(path) as fh:
                self.assertEqual(json.load(fh)["type"], "stdio")
            self.assertTrue(os.access(browsers.host_launcher(), os.X_OK))
            browsers.unregister()
            self.assertFalse(os.path.exists(path))
        finally:
            if old is not None:
                os.environ["HOME"] = old


class MediaTest(unittest.TestCase):
    def test_detection(self):
        self.assertTrue(media.looks_like_media_page("https://www.youtube.com/watch?v=abc"))
        self.assertTrue(media.looks_like_media_page("https://youtu.be/abc"))
        self.assertTrue(media.looks_like_media_page("https://www.tiktok.com/@a/video/1"))
        self.assertFalse(media.looks_like_media_page("https://example.com/file.zip"))
        self.assertFalse(media.looks_like_media_page("https://x.com/files/archive.zip"))

    def test_format_selection(self):
        self.assertEqual(media.format_for("best", "mp3", True)[0], "bv*+ba/b/b")
        self.assertIn("height<=720", media.format_for("720", "mp3", True)[0])
        self.assertEqual(media.format_for("720", "mp3", False)[0], "b[height<=720]/b")
        fmt, post = media.format_for("audio", "mp3", True)
        self.assertEqual(post[0]["preferredcodec"], "mp3")

    def test_json_roundtrip(self):
        mgr = engine.Manager(persist=False)
        d = mgr.add_media("https://youtu.be/x", start=False, quality="720")
        again = engine.Download.from_json(json.loads(json.dumps(d.to_json())))
        self.assertEqual(again.kind, "media")
        self.assertEqual(again.quality, "720")


if __name__ == "__main__":
    unittest.main()
