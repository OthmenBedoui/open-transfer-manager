import hashlib
import json
import os
import tempfile
import time
import unittest

from helpers import DATA, SLOW, serve, wait

from otm.core import defaults, engine


def sha(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


WANT = hashlib.sha256(DATA).hexdigest()


class EngineTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.srv, cls.base = serve()

    @classmethod
    def tearDownClass(cls):
        cls.srv.shutdown()

    def setUp(self):
        SLOW["on"] = False
        self.tmp = tempfile.mkdtemp()
        self.mgr = engine.Manager(persist=False)

    def test_probe(self):
        info = engine.probe(self.base + "/files/archive.zip")
        self.assertTrue(info["resumable"])
        self.assertEqual(info["size"], len(DATA))
        self.assertEqual(engine.probe(self.base + "/named/x")["name"], "report final.pdf")

    def test_multi_connection(self):
        SLOW["on"] = True  # a local server is too fast to see the parallel connections
        dl = self.mgr.add(self.base + "/files/archive.zip", "archive.zip", self.tmp, "Compressed")
        peak = 0
        while dl.running():
            peak = max(peak, dl.active_connections())
            time.sleep(0.01)
        self.assertEqual(dl.status, engine.COMPLETE, dl.error)
        self.assertEqual(sha(dl.path), WANT)
        self.assertGreater(peak, 1)
        self.assertFalse(os.path.exists(dl.part_path))

    def test_pause_resume(self):
        SLOW["on"] = True
        dl = self.mgr.add(self.base + "/files/video.mp4", "video.mp4", self.tmp, "Video")
        time.sleep(1.0)
        dl.stop()
        wait(dl)
        mid = dl.downloaded()
        self.assertEqual(dl.status, engine.STOPPED)
        self.assertTrue(0 < mid < len(DATA))
        SLOW["on"] = False
        dl.start(self.mgr)
        wait(dl)
        self.assertEqual(dl.status, engine.COMPLETE, dl.error)
        self.assertEqual(sha(dl.path), WANT)

    def test_resume_after_restart(self):
        """Progress survives saving the list and loading it in a new manager."""
        SLOW["on"] = True
        dl = self.mgr.add(self.base + "/files/big.bin", "big.bin", self.tmp, "General")
        time.sleep(1.0)
        dl.stop()
        wait(dl)
        saved = json.loads(json.dumps(dl.to_json()))
        SLOW["on"] = False
        again = engine.Download.from_json(saved)
        self.assertEqual(again.downloaded(), dl.downloaded())
        again.start(self.mgr)
        wait(again)
        self.assertEqual(again.status, engine.COMPLETE, again.error)
        self.assertEqual(sha(again.path), WANT)

    def test_no_ranges_single_connection(self):
        dl = self.mgr.add(self.base + "/norange/doc.pdf", "doc.pdf", self.tmp, "Documents")
        wait(dl)
        self.assertEqual(dl.status, engine.COMPLETE, dl.error)
        self.assertFalse(dl.resumable)
        self.assertEqual(len(dl.segments), 1)
        self.assertEqual(sha(dl.path), WANT)

    def test_http_error(self):
        dl = self.mgr.add(self.base + "/missing/file.zip", "file.zip", self.tmp, "Compressed")
        wait(dl)
        self.assertEqual(dl.status, engine.ERROR)
        self.assertIn("404", dl.error)

    def test_server_file_name_replaces_guess(self):
        self.mgr.base_folder = self.tmp
        self.mgr.categories = [engine.Category(n, e, os.path.join(self.tmp, n)) for n, e in defaults.CATEGORIES]
        dl = self.mgr.add_smart(self.base + "/named/download?id=1", media=False)
        wait(dl)
        self.assertEqual(dl.status, engine.COMPLETE, dl.error)
        self.assertEqual(dl.file_name, "report final.pdf")
        self.assertEqual(dl.category, "Documents")
        self.assertTrue(os.path.exists(os.path.join(self.tmp, "Documents", "report final.pdf")))

    def test_name_clash(self):
        open(os.path.join(self.tmp, "same.bin"), "wb").close()
        dl = self.mgr.add(self.base + "/files/same.bin", "same.bin", self.tmp, "General")
        wait(dl)
        self.assertEqual(dl.file_name, "same (2).bin")

    def test_speed_limit(self):
        self.mgr.limit_kb = 2048
        self.mgr.set_limiter(True)
        t0 = time.time()
        dl = self.mgr.add(self.base + "/files/limited.zip", "limited.zip", self.tmp, "Compressed")
        wait(dl, 30)
        self.assertEqual(dl.status, engine.COMPLETE)
        self.assertGreater(time.time() - t0, 2.0)

    def test_categories(self):
        for name, cat in (("a.ZIP", "Compressed"), ("b.r01", "Compressed"), ("c.docx", "Documents"),
                          ("d.mkv", "Video"), ("e.deb", "Programs"), ("f.flac", "Music"), ("g.webp", "Images"),
                          ("h.xyz", defaults.GENERAL)):
            self.assertEqual(self.mgr.category_name_for(name), cat, name)

    def test_queue_limit(self):
        SLOW["on"] = True
        self.mgr.queue_at_once = 2
        q = [self.mgr.add(self.base + "/files/q%d.zip" % i, "q%d.zip" % i, self.tmp, "Compressed",
                          start=False, queue=defaults.MAIN_QUEUE) for i in range(3)]
        self.mgr.start_queue()
        peak = 0
        t0 = time.time()
        while time.time() - t0 < 4:
            self.mgr.tick()
            peak = max(peak, sum(1 for d in q if d.running()))
            time.sleep(0.1)
        self.mgr.stop_all()
        for d in q:
            wait(d)
        self.assertEqual(peak, 2)

    def test_formatting(self):
        self.assertEqual(engine.fmt_size(512), "512 B")
        self.assertEqual(engine.fmt_size(1536), "1.5 KB")
        self.assertEqual(engine.fmt_size(5 * 1024 ** 3), "5.0 GB")
        self.assertEqual(engine.fmt_eta(75), "1m 15s")
        self.assertEqual(engine.safe_name('a/b:c*?.txt'), "a_b_c__.txt")

    def test_call_runs_on_tick_thread(self):
        import threading
        out = {}
        t = threading.Thread(target=lambda: out.setdefault("v", self.mgr.call(lambda: 42)))
        t.start()
        for _ in range(50):
            self.mgr.tick()
            if not t.is_alive():
                break
            time.sleep(0.02)
        t.join(2)
        self.assertEqual(out.get("v"), 42)


class PersistenceTest(unittest.TestCase):
    def test_save_and_load(self):
        d = tempfile.mkdtemp()
        os.environ["OTM_DATA_DIR"] = d
        mgr = engine.Manager()
        mgr.theme = "dark"
        mgr.add("https://example.com/a.zip", "a.zip", d, "Compressed", start=False)
        mgr.save()
        again = engine.Manager()
        self.assertEqual(again.theme, "dark")
        self.assertEqual([x.file_name for x in again.downloads], ["a.zip"])

    def test_corrupt_list_falls_back_to_backup(self):
        d = tempfile.mkdtemp()
        os.environ["OTM_DATA_DIR"] = d
        mgr = engine.Manager()
        mgr.add("https://example.com/a.zip", "a.zip", d, "Compressed", start=False)
        mgr.save()
        mgr.save()  # creates .bak
        with open(mgr.list_path(), "w") as f:
            f.write("{broken")
        again = engine.Manager()
        self.assertEqual(len(again.downloads), 1)


if __name__ == "__main__":
    unittest.main()
