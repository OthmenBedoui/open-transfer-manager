"""Register the native messaging host with every browser found on this computer.

Chromium-family browsers (Chrome, Chromium, Brave, Edge, Vivaldi, Opera) and Firefox-family
browsers (Firefox, LibreWolf, Waterfox) read a small JSON "host manifest" from a known folder
(or, on Windows, from the registry). Safari works differently: see docs/BROWSER_EXTENSIONS.md.
"""

import json
import os
import stat
import sys

from . import paths

HOST_NAME = "org.opentransfermanager.host"
DESCRIPTION = "Open Transfer Manager browser bridge"

# Extension IDs allowed to talk to the host.
# The first one is fixed by the "key" in extension/manifest.chromium.json (unpacked / self-hosted builds).
# Add the Chrome Web Store / Edge Add-ons / Opera Add-ons IDs here once the extension is published.
CHROMIUM_EXTENSION_IDS = [
    "iaaaiogfbkpiddhonhfnekdffilohjjc",
]
FIREFOX_EXTENSION_IDS = ["otm@opentransfermanager.org"]


def _targets():
    """(browser name, family, profile root that proves the browser is installed, manifest folder)."""
    h = paths.home()
    if paths.IS_MAC:
        sup = os.path.join(h, "Library", "Application Support")
        chromium = [
            ("Google Chrome", "Google/Chrome"),
            ("Chrome Beta", "Google/Chrome Beta"),
            ("Chromium", "Chromium"),
            ("Brave", "BraveSoftware/Brave-Browser"),
            ("Microsoft Edge", "Microsoft Edge"),
            ("Vivaldi", "Vivaldi"),
            ("Opera", "com.operasoftware.Opera"),
            ("Arc", "Arc/User Data"),
        ]
        out = [(n, "chromium", os.path.join(sup, d), os.path.join(sup, d, "NativeMessagingHosts")) for n, d in chromium]
        moz = os.path.join(sup, "Mozilla")
        out.append(("Firefox", "firefox", os.path.join(sup, "Firefox"), os.path.join(moz, "NativeMessagingHosts")))
        out.append(("LibreWolf", "firefox", os.path.join(sup, "librewolf"),
                    os.path.join(sup, "librewolf", "NativeMessagingHosts")))
        return out
    if paths.IS_WIN:
        return []  # registry based, see _register_windows
    cfg = os.environ.get("XDG_CONFIG_HOME") or os.path.join(h, ".config")
    chromium = [
        ("Google Chrome", "google-chrome"),
        ("Chrome Beta", "google-chrome-beta"),
        ("Chromium", "chromium"),
        ("Brave", "BraveSoftware/Brave-Browser"),
        ("Microsoft Edge", "microsoft-edge"),
        ("Vivaldi", "vivaldi"),
        ("Opera", "opera"),
    ]
    out = [(n, "chromium", os.path.join(cfg, d), os.path.join(cfg, d, "NativeMessagingHosts")) for n, d in chromium]
    # Flatpak Chromium-family browsers
    for n, app, d in (("Chromium (Flatpak)", "org.chromium.Chromium", "chromium"),
                      ("Brave (Flatpak)", "com.brave.Browser", "BraveSoftware/Brave-Browser"),
                      ("Chrome (Flatpak)", "com.google.Chrome", "google-chrome")):
        root = os.path.join(h, ".var", "app", app, "config", d)
        out.append((n, "chromium", root, os.path.join(root, "NativeMessagingHosts")))
    out.append(("Firefox", "firefox", os.path.join(h, ".mozilla", "firefox"),
                os.path.join(h, ".mozilla", "native-messaging-hosts")))
    out.append(("Firefox (Snap)", "firefox", os.path.join(h, "snap", "firefox", "common", ".mozilla", "firefox"),
                os.path.join(h, "snap", "firefox", "common", ".mozilla", "native-messaging-hosts")))
    out.append(("LibreWolf", "firefox", os.path.join(h, ".librewolf"),
                os.path.join(h, ".librewolf", "native-messaging-hosts")))
    out.append(("Waterfox", "firefox", os.path.join(h, ".waterfox"),
                os.path.join(h, ".waterfox", "native-messaging-hosts")))
    return out


def host_launcher():
    """A tiny script the browser can run, pointing at this exact Python and package."""
    folder = os.path.join(paths.data_dir(), "bin")
    os.makedirs(folder, exist_ok=True)
    py = sys.executable
    src = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))  # .../site-packages or src
    if os.name == "nt":
        path = os.path.join(folder, "otm-native-host.bat")
        body = '@echo off\r\nset "PYTHONPATH=%s"\r\n"%s" -m otm.native_host %%*\r\n' % (src, py)
    else:
        path = os.path.join(folder, "otm-native-host")
        body = "#!/bin/sh\nPYTHONPATH=%s exec %s -m otm.native_host \"$@\"\n" % (_sh(src), _sh(py))
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(body)
    if os.name != "nt":
        os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    return path


def _sh(s):
    return "'" + s.replace("'", "'\\''") + "'"


def manifest(family, launcher):
    m = {"name": HOST_NAME, "description": DESCRIPTION, "path": launcher, "type": "stdio"}
    if family == "firefox":
        m["allowed_extensions"] = list(FIREFOX_EXTENSION_IDS)
    else:
        m["allowed_origins"] = ["chrome-extension://%s/" % i for i in CHROMIUM_EXTENSION_IDS]
    return m


def register(force=False):
    """Write host manifests for installed browsers. Returns list of (browser, path)."""
    launcher = host_launcher()
    if paths.IS_WIN:
        return _register_windows(launcher)
    done = []
    for name, family, root, folder in _targets():
        if not force and not os.path.isdir(root):
            continue
        os.makedirs(folder, exist_ok=True)
        path = os.path.join(folder, HOST_NAME + ".json")
        with open(path, "w", encoding="utf-8") as f:
            json.dump(manifest(family, launcher), f, indent=2)
        done.append((name, path))
    return done


def unregister():
    removed = []
    if paths.IS_WIN:
        return _unregister_windows()
    for name, _family, _root, folder in _targets():
        path = os.path.join(folder, HOST_NAME + ".json")
        if os.path.exists(path):
            os.remove(path)
            removed.append((name, path))
    return removed


def status():
    """[(browser, installed, registered)] for the doctor command and Settings."""
    if paths.IS_WIN:
        return [(n, True, ok) for n, ok in _status_windows()]
    out = []
    for name, _family, root, folder in _targets():
        installed = os.path.isdir(root)
        registered = os.path.exists(os.path.join(folder, HOST_NAME + ".json"))
        if installed or registered:
            out.append((name, installed, registered))
    return out


# ---------------------------------------------------------------- Windows

_WIN_KEYS = [
    ("Google Chrome", "chromium", r"Software\Google\Chrome\NativeMessagingHosts"),
    ("Chromium", "chromium", r"Software\Chromium\NativeMessagingHosts"),
    ("Microsoft Edge", "chromium", r"Software\Microsoft\Edge\NativeMessagingHosts"),
    ("Brave", "chromium", r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts"),
    ("Vivaldi", "chromium", r"Software\Vivaldi\NativeMessagingHosts"),
    ("Opera", "chromium", r"Software\Opera Software\NativeMessagingHosts"),
    ("Firefox", "firefox", r"Software\Mozilla\NativeMessagingHosts"),
]


def _register_windows(launcher):
    import winreg
    done = []
    for name, family, key in _WIN_KEYS:
        path = os.path.join(paths.data_dir(), "%s.%s.json" % (HOST_NAME, family))
        with open(path, "w", encoding="utf-8") as f:
            json.dump(manifest(family, launcher), f, indent=2)
        with winreg.CreateKey(winreg.HKEY_CURRENT_USER, key + "\\" + HOST_NAME) as k:
            winreg.SetValueEx(k, "", 0, winreg.REG_SZ, path)
        done.append((name, path))
    return done


def _unregister_windows():
    import winreg
    removed = []
    for name, _family, key in _WIN_KEYS:
        try:
            winreg.DeleteKey(winreg.HKEY_CURRENT_USER, key + "\\" + HOST_NAME)
            removed.append((name, key))
        except OSError:
            pass
    return removed


def _status_windows():
    import winreg
    out = []
    for name, _family, key in _WIN_KEYS:
        try:
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, key + "\\" + HOST_NAME):
                out.append((name, True))
        except OSError:
            out.append((name, False))
    return out
