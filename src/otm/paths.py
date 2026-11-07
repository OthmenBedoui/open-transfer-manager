"""Where things live on each operating system, and how to open files there."""

import os
import re
import subprocess
import sys

from . import APP_ID, APP_NAME

IS_MAC = sys.platform == "darwin"
IS_WIN = os.name == "nt"
IS_LINUX = sys.platform.startswith("linux")


def home():
    return os.path.expanduser("~")


def data_dir():
    """Settings, download list and runtime files."""
    override = os.environ.get("OTM_DATA_DIR")
    if override:
        path = override
    elif IS_MAC:
        path = os.path.join(home(), "Library", "Application Support", APP_NAME)
    elif IS_WIN:
        path = os.path.join(os.environ.get("APPDATA") or home(), "OpenTransferManager")
    else:
        base = os.environ.get("XDG_DATA_HOME") or os.path.join(home(), ".local", "share")
        path = os.path.join(base, APP_ID)
    os.makedirs(path, exist_ok=True)
    return path


def downloads_dir():
    """The user's Downloads folder (honours XDG user-dirs on Linux)."""
    if IS_LINUX:
        cfg = os.path.join(os.environ.get("XDG_CONFIG_HOME") or os.path.join(home(), ".config"), "user-dirs.dirs")
        try:
            with open(cfg, encoding="utf-8") as f:
                for line in f:
                    m = re.match(r'\s*XDG_DOWNLOAD_DIR\s*=\s*"(.*)"', line)
                    if m:
                        return m.group(1).replace("$HOME", home())
        except OSError:
            pass
    return os.path.join(home(), "Downloads")


def open_path(path):
    """Open a file or folder with the default application."""
    try:
        if IS_WIN:
            os.startfile(path)  # type: ignore[attr-defined]
        elif IS_MAC:
            subprocess.Popen(["open", path])
        else:
            subprocess.Popen(["xdg-open", path], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except OSError:
        pass


def reveal(path):
    """Show a file selected in the file manager (or open its folder)."""
    try:
        if IS_MAC and os.path.exists(path):
            subprocess.Popen(["open", "-R", path])
            return
        if IS_WIN and os.path.exists(path):
            subprocess.Popen(["explorer", "/select,", os.path.normpath(path)])
            return
    except OSError:
        pass
    folder = path if os.path.isdir(path) else os.path.dirname(path)
    open_path(folder)


def notify(title, message):
    """Desktop notification without extra dependencies. Never raises."""
    try:
        if IS_MAC:
            script = 'display notification %s with title %s' % (_applescript_str(message), _applescript_str(title))
            subprocess.Popen(["osascript", "-e", script], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        elif IS_LINUX:
            subprocess.Popen(["notify-send", "-a", APP_NAME, title, message],
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except OSError:
        pass


def _applescript_str(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'
