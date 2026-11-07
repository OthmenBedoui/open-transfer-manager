"""Native messaging host: the bridge between the browser extension and the app.

The browser starts this program and talks to it over stdin/stdout with
length-prefixed JSON messages. Each request carries an "id"; each answer repeats it.

Requests:
  {"id": 1, "cmd": "ping"}
  {"id": 2, "cmd": "add", "url": "...", "filename": "...", "referer": "...", "cookies": "...", "media": true}
  {"id": 3, "cmd": "list"}
  {"id": 4, "cmd": "control", "dl": 12, "action": "pause|resume|remove|open|reveal"}
  {"id": 5, "cmd": "show"}
  {"id": 6, "cmd": "subscribe"}       -> pushes {"type": "downloads", ...} every second
"""

import json
import os
import struct
import subprocess
import sys
import threading
import time

from . import __version__, server

MAX_MESSAGE = 1024 * 1024
_out_lock = threading.Lock()


def read_message(stream):
    raw = stream.read(4)
    if len(raw) < 4:
        return None
    (n,) = struct.unpack("=I", raw)
    if n > MAX_MESSAGE:
        raise ValueError("message too large")
    data = stream.read(n)
    return json.loads(data.decode("utf-8"))


def encode_message(obj):
    data = json.dumps(obj, separators=(",", ":")).encode("utf-8")
    return struct.pack("=I", len(data)) + data


def send(obj, stream=None):
    stream = stream or sys.stdout.buffer
    with _out_lock:
        stream.write(encode_message(obj))
        stream.flush()


def launch_app(wait=15.0):
    """Start the app in the background if it is not running. Returns True when reachable."""
    if server.is_running():
        return True
    kwargs = {"stdin": subprocess.DEVNULL, "stdout": subprocess.DEVNULL, "stderr": subprocess.DEVNULL,
              "close_fds": True}
    if os.name == "nt":
        kwargs["creationflags"] = 0x00000008 | 0x00000200  # DETACHED_PROCESS | NEW_PROCESS_GROUP
    else:
        kwargs["start_new_session"] = True
    subprocess.Popen([sys.executable, "-m", "otm", "--from-browser"], **kwargs)
    end = time.time() + wait
    while time.time() < end:
        if server.is_running(timeout=0.5):
            return True
        time.sleep(0.3)
    return False


def handle(msg):
    cmd = msg.get("cmd")
    if cmd == "ping":
        return {"host": __version__, "app_running": server.is_running()}
    if cmd == "add":
        if not launch_app():
            raise RuntimeError("Could not start Open Transfer Manager")
        body = {k: msg[k] for k in ("url", "filename", "referer", "cookies", "user_agent", "media", "quality",
                                    "start", "focus") if k in msg}
        return server.request("POST", "/api/add", body, timeout=15)
    if cmd == "list":
        if not server.is_running():
            return {"app_running": False, "downloads": []}
        r = server.request("GET", "/api/downloads")
        r["app_running"] = True
        return r
    if cmd == "control":
        return server.request("POST", "/api/control", {"id": msg["dl"], "action": msg["action"]})
    if cmd == "show":
        if not launch_app():
            raise RuntimeError("Could not start Open Transfer Manager")
        return server.request("POST", "/api/show", {})
    raise ValueError("unknown command %r" % cmd)


def _pusher(stop):
    while not stop.is_set():
        try:
            r = handle({"cmd": "list"})
            r["type"] = "downloads"
            send(r)
        except Exception as e:  # noqa: BLE001
            send({"type": "downloads", "app_running": False, "downloads": [], "error": str(e)})
        stop.wait(1.0)


def main():
    if os.name == "nt":
        import msvcrt
        msvcrt.setmode(sys.stdin.fileno(), os.O_BINARY)
        msvcrt.setmode(sys.stdout.fileno(), os.O_BINARY)
    stop = threading.Event()
    pusher = None
    workers = []
    stdin = sys.stdin.buffer
    while True:
        try:
            msg = read_message(stdin)
        except (ValueError, OSError):
            break
        if msg is None:
            break
        mid = msg.get("id")
        if msg.get("cmd") == "subscribe":
            if pusher is None:
                pusher = threading.Thread(target=_pusher, args=(stop,), daemon=True)
                pusher.start()
            send({"id": mid, "ok": True})
            continue

        def work(m=msg, i=mid):
            try:
                send({"id": i, "ok": True, "data": handle(m)})
            except Exception as e:  # noqa: BLE001
                send({"id": i, "ok": False, "error": str(e)})
        t = threading.Thread(target=work, daemon=True)
        t.start()
        workers.append(t)
        workers = [w for w in workers if w.is_alive()]
    stop.set()
    for t in workers:  # let pending answers go out (one-shot sendNativeMessage calls)
        t.join(timeout=20)
    return 0


if __name__ == "__main__":
    sys.exit(main())
