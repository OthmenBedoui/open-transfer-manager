"""Command line entry point: `otm`.

    otm                         open the app (or bring the running one to the front)
    otm URL [URL...]            add links to the running app (starts it if needed)
    otm add URL [--media] [--quality 720|audio|best] [--later]
    otm list                    show downloads of the running app
    otm headless                run without a window (server / NAS), controlled by the extension
    otm doctor                  diagnostics to paste in bug reports
    otm register-browsers [--force]
    otm unregister-browsers
    otm update-ytdlp
    otm --version
"""

import argparse
import os
import platform
import sys
import time

from . import APP_NAME, HOMEPAGE, __version__, browsers, paths, server

COMMANDS = ("add", "list", "headless", "doctor", "register-browsers", "unregister-browsers", "update-ytdlp")


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    if argv and argv[0] in ("-V", "--version"):
        print("%s %s" % (APP_NAME, __version__))
        return 0
    if argv and argv[0] in ("-h", "--help", "help"):
        print(__doc__)
        return 0
    if argv and argv[0] in COMMANDS:
        return _command(argv[0], argv[1:])
    return _gui(argv)


# ---------------------------------------------------------------- GUI

def _gui(argv):
    p = argparse.ArgumentParser(prog="otm", add_help=False)
    p.add_argument("urls", nargs="*")
    p.add_argument("--from-browser", action="store_true")
    p.add_argument("--minimized", action="store_true")
    p.add_argument("--screenshot")
    p.add_argument("--theme", choices=["light", "dark", "system"])
    a = p.parse_args(argv)
    urls = [u for u in a.urls if u.lower().startswith(("http://", "https://"))]

    if not a.screenshot and server.is_running():
        for u in urls:
            server.request("POST", "/api/add", {"url": u})
        if not a.from_browser:
            server.request("POST", "/api/show", {})
        return 0

    try:
        import tkinter  # noqa: F401
    except ImportError:
        print("Tkinter is missing. Install it, for example:\n"
              "  Debian/Ubuntu: sudo apt install python3-tk\n"
              "  Fedora:        sudo dnf install python3-tkinter\n"
              "  Arch:          sudo pacman -S tk\n"
              "  macOS (brew):  brew install python-tk\n"
              "Or run without a window: otm headless", file=sys.stderr)
        return 1

    from .ui.app import run
    return run(urls=urls, screenshot=a.screenshot, minimized=a.minimized or a.from_browser, theme=a.theme)


# ---------------------------------------------------------------- commands

def _command(name, args):
    if name == "add":
        p = argparse.ArgumentParser(prog="otm add")
        p.add_argument("url")
        p.add_argument("--media", action="store_true", default=None, help="force video/audio mode (yt-dlp)")
        p.add_argument("--file", action="store_true", help="force plain file mode")
        p.add_argument("--quality", choices=["best", "2160", "1440", "1080", "720", "480", "360", "audio"])
        p.add_argument("--later", action="store_true", help="add to the queue without starting")
        a = p.parse_args(args)
        body = {"url": a.url, "start": not a.later}
        if a.file:
            body["media"] = False
        elif a.media:
            body["media"] = True
        if a.quality:
            body["quality"] = a.quality
        if not server.is_running():
            from .native_host import launch_app
            if not launch_app():
                print("Could not start %s" % APP_NAME, file=sys.stderr)
                return 1
        r = server.request("POST", "/api/add", body, timeout=15)
        d = r["download"]
        print("Added #%d %s" % (d["id"], d["name"]))
        return 0

    if name == "list":
        if not server.is_running():
            print("%s is not running." % APP_NAME)
            return 1
        r = server.request("GET", "/api/downloads")
        from .core.engine import fmt_rate, fmt_size
        for d in r["downloads"]:
            print("#%-4d %-14s %9s  %10s  %s" % (d["id"], d["status_text"][:14], fmt_size(d["size"]),
                                                 fmt_rate(d["rate"]) if d["running"] else "", d["name"]))
        return 0

    if name == "headless":
        return _headless()

    if name == "doctor":
        _doctor()
        return 0

    if name == "register-browsers":
        done = browsers.register(force="--force" in args)
        if not done:
            print("No supported browser found. Use --force to write the files anyway.")
        for b, path in done:
            print("  registered for %-20s %s" % (b, path))
        return 0

    if name == "unregister-browsers":
        for b, path in browsers.unregister():
            print("  removed for %-20s %s" % (b, path))
        return 0

    if name == "update-ytdlp":
        from .core import media
        print("Updating yt-dlp (current: %s)..." % (media.ytdlp_version() or "not installed"))
        ok, out = media.update_ytdlp()
        print(out.strip())
        return 0 if ok else 1
    return 2


def _headless():
    from .core import engine
    if server.is_running():
        print("%s is already running." % APP_NAME)
        return 1
    mgr = engine.Manager()
    api = server.ApiServer(mgr)
    port = api.start()
    print("%s %s running headless on 127.0.0.1:%d (Ctrl+C to stop)" % (APP_NAME, __version__, port), flush=True)
    import signal

    def _term(*_a):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, _term)  # clean shutdown under systemd / kill
    try:
        while True:
            mgr.tick()
            time.sleep(0.5)
    except KeyboardInterrupt:
        pass
    finally:
        api.stop()
        mgr.shutdown()
    return 0


def _doctor():
    from .core import media
    try:
        import tkinter
        tk_ver = str(tkinter.TkVersion)
    except ImportError:
        tk_ver = "MISSING"
    rows = [
        ("Open Transfer Manager", __version__),
        ("Python", "%s (%s)" % (platform.python_version(), sys.executable)),
        ("OS", "%s %s %s" % (platform.system(), platform.release(), platform.machine())),
        ("Tk", tk_ver),
        ("yt-dlp", media.ytdlp_version() or "not installed"),
        ("ffmpeg", media.ffmpeg_path() or "not found (needed for best video quality and mp3)"),
        ("Data folder", paths.data_dir()),
        ("Downloads folder", paths.downloads_dir()),
        ("App running", "yes" if server.is_running() else "no"),
        ("Homepage", HOMEPAGE),
    ]
    for k, v in rows:
        print("%-22s %s" % (k, v))
    print("Browsers:")
    st = browsers.status()
    if not st:
        print("  (none detected)")
    for b, installed, registered in st:
        print("  %-22s %s, %s" % (b, "installed" if installed else "not found",
                                  "bridge registered" if registered else "bridge NOT registered"))
    if os.environ.get("WAYLAND_DISPLAY") and not os.environ.get("DISPLAY"):
        print("Note: Wayland without XWayland - Tk needs XWayland to show the window.")
