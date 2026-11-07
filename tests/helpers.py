import http.server
import os
import re
import sys
import tempfile
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "src"))

# keep tests away from the real settings
os.environ.setdefault("OTM_DATA_DIR", tempfile.mkdtemp(prefix="otm-test-data-"))

DATA = os.urandom(6 * 1024 * 1024 + 123)
SLOW = {"on": False}


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def do_GET(self):  # noqa: N802
        if self.path.startswith("/missing"):
            self.send_response(404)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        rng = self.headers.get("Range")
        ranges = not self.path.startswith("/norange")
        start, end = 0, len(DATA) - 1
        if rng and ranges:
            m = re.match(r"bytes=(\d+)-(\d*)", rng)
            start = int(m.group(1))
            if m.group(2):
                end = min(end, int(m.group(2)))
            self.send_response(206)
            self.send_header("Content-Range", "bytes %d-%d/%d" % (start, end, len(DATA)))
        else:
            self.send_response(200)
        if self.path.startswith("/named"):
            self.send_header("Content-Disposition", 'attachment; filename="report final.pdf"')
        self.send_header("Content-Length", str(end - start + 1))
        self.send_header("Content-Type", "application/octet-stream")
        self.end_headers()
        pos = start
        try:
            while pos <= end:
                n = min(65536, end - pos + 1)
                self.wfile.write(DATA[pos:pos + n])
                pos += n
                if SLOW["on"]:
                    time.sleep(0.2)
        except (ConnectionError, OSError):
            pass


def serve():
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    srv.daemon_threads = True
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv, "http://127.0.0.1:%d" % srv.server_address[1]


def wait(dl, secs=60):
    t0 = time.time()
    while dl.running() and time.time() - t0 < secs:
        time.sleep(0.05)
