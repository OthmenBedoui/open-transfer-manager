"""Local control API on 127.0.0.1, used by the browser bridge and by a second `otm` launch.

Security:
* listens on the loopback interface only;
* every call except /api/ping needs the secret token stored in runtime.json (file mode 600);
* the Host header must be 127.0.0.1/localhost (blocks DNS-rebinding);
* no CORS headers are ever sent, so web pages cannot read answers or send the token header.
"""

import hmac
import http.server
import json
import os
import secrets
import socket
import threading
import urllib.error
import urllib.request

from . import __version__, paths
from .core import engine

DEFAULT_PORT = 51737
PORT_TRIES = 10
MAX_BODY = 256 * 1024


def runtime_path():
    return os.path.join(paths.data_dir(), "runtime.json")


def read_runtime():
    try:
        with open(runtime_path(), encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


# ---------------------------------------------------------------- client side

def request(method, path, body=None, timeout=5, rt=None):
    """Call the running app. Raises OSError when it is not running."""
    rt = rt or read_runtime()
    if not rt:
        raise ConnectionRefusedError("Open Transfer Manager is not running")
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request("http://127.0.0.1:%d%s" % (rt["port"], path), data=data, method=method)
    req.add_header("X-OTM-Token", rt.get("token", ""))
    if data is not None:
        req.add_header("Content-Type", "application/json")
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))  # never via a proxy
    try:
        with opener.open(req, timeout=timeout) as resp:
            return json.loads(resp.read() or b"{}")
    except urllib.error.HTTPError as e:
        try:
            msg = json.loads(e.read()).get("error", str(e))
        except ValueError:
            msg = str(e)
        raise RuntimeError(msg) from None


def is_running(timeout=1.5):
    try:
        return request("GET", "/api/ping", timeout=timeout).get("app") == "otm"
    except (OSError, RuntimeError, ValueError):
        return False


# ---------------------------------------------------------------- server side

class _Handler(http.server.BaseHTTPRequestHandler):
    server_version = "OTM/" + __version__
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def _send(self, code, obj):
        data = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(data)

    def _guard(self, need_token=True):
        host = (self.headers.get("Host") or "").rsplit(":", 1)[0].strip("[]")
        if host not in ("127.0.0.1", "localhost", "::1"):
            self._send(403, {"error": "bad host"})
            return False
        if need_token:
            token = self.headers.get("X-OTM-Token") or ""
            if not hmac.compare_digest(token, self.server.token):
                self._send(401, {"error": "bad token"})
                return False
        return True

    def _body(self):
        n = int(self.headers.get("Content-Length") or 0)
        if n > MAX_BODY:
            raise ValueError("request too large")
        raw = self.rfile.read(n) if n else b"{}"
        obj = json.loads(raw or b"{}")
        if not isinstance(obj, dict):
            raise ValueError("JSON object expected")
        return obj

    def do_OPTIONS(self):  # noqa: N802 - no CORS, ever
        self._send(405, {"error": "not allowed"})

    def do_GET(self):  # noqa: N802
        if self.path == "/api/ping":
            if self._guard(need_token=False):
                self._send(200, {"app": "otm", "version": __version__})
            return
        if not self._guard():
            return
        mgr = self.server.mgr
        if self.path == "/api/downloads":
            items = [d.summary() for d in list(mgr.downloads) if not d.demo]
            self._send(200, {"downloads": items, "total_rate": mgr.total_rate()})
        else:
            self._send(404, {"error": "not found"})

    def do_POST(self):  # noqa: N802
        if not self._guard():
            return
        try:
            body = self._body()
            result = self.server.api.handle(self.path, body)
        except KeyError as e:
            self._send(400, {"error": "missing field %s" % e})
            return
        except (ValueError, TypeError) as e:
            self._send(400, {"error": str(e)})
            return
        except engine.CancelledError as e:
            self._send(503, {"error": str(e)})
            return
        except Exception as e:  # noqa: BLE001
            self._send(500, {"error": str(e)})
            return
        if result is None:
            self._send(404, {"error": "not found"})
        else:
            self._send(200, result)


class _HttpServer(http.server.ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = False


class ApiServer:
    def __init__(self, mgr):
        self.mgr = mgr
        self.httpd = None
        self.thread = None

    # --- commands (run on the manager's tick thread through mgr.call)
    def handle(self, path, body):
        mgr = self.mgr
        if path == "/api/add":
            url = str(body["url"]).strip()
            if not url.lower().startswith(("http://", "https://")):
                raise ValueError("only http(s) links can be downloaded")
            headers = {}
            for key, hk in (("referer", "Referer"), ("cookies", "Cookie"), ("user_agent", "User-Agent")):
                if body.get(key):
                    headers[hk] = str(body[key])[:8192]
            media = body.get("media")
            opts = {}
            if body.get("quality"):
                opts["quality"] = str(body["quality"])
            dl = mgr.call(lambda: mgr.add_smart(url, body.get("filename") or "", start=body.get("start", True),
                                                headers=headers, media=media, **opts))
            if mgr.on_show and body.get("focus"):
                mgr.call(mgr.on_show)
            return {"ok": True, "download": dl.summary()}
        if path == "/api/control":
            dl_id = int(body["id"])
            action = body["action"]

            def run():
                d = mgr.find(dl_id)
                if d is None:
                    raise ValueError("no such download")
                if action in ("pause", "stop"):
                    d.stop()
                elif action in ("resume", "start"):
                    d.start(mgr)
                elif action == "remove":
                    mgr.remove(d, delete_file=False)
                elif action == "open":
                    paths.open_path(d.path)
                elif action == "reveal":
                    paths.reveal(d.path)
                else:
                    raise ValueError("unknown action")
                return {"ok": True}
            return mgr.call(run)
        if path == "/api/show":
            if mgr.on_show:
                mgr.call(mgr.on_show)
            return {"ok": True}
        if path == "/api/pause-all":
            mgr.call(mgr.stop_all)
            return {"ok": True}
        return None

    def start(self):
        token = secrets.token_urlsafe(32)
        last = None
        for port in [DEFAULT_PORT + i for i in range(PORT_TRIES)] + [0]:
            try:
                self.httpd = _HttpServer(("127.0.0.1", port), _Handler)
                break
            except OSError as e:
                last = e
        if self.httpd is None:
            raise last or OSError("cannot listen")
        self.httpd.token = token
        self.httpd.mgr = self.mgr
        self.httpd.api = self
        port = self.httpd.server_address[1]
        _write_private(runtime_path(), {"port": port, "token": token, "pid": os.getpid(), "version": __version__})
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True, name="otm-api")
        self.thread.start()
        return port

    def stop(self):
        if self.httpd:
            self.httpd.shutdown()
            self.httpd.server_close()
            rt = read_runtime()
            if rt and rt.get("pid") == os.getpid():
                try:
                    os.remove(runtime_path())
                except OSError:
                    pass
            self.httpd = None


def _write_private(path, obj):
    tmp = path + ".tmp"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        json.dump(obj, f)
    os.replace(tmp, path)


def port_free(port):
    with socket.socket() as s:
        try:
            s.bind(("127.0.0.1", port))
            return True
        except OSError:
            return False
