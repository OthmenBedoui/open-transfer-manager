"""Build the browser extension for each browser family.

    python scripts/build_extension.py            -> dist/extension/{chromium,firefox}/ + .zip files

The Chromium build works in Chrome, Brave, Edge, Opera and Vivaldi.
The Firefox build works in Firefox, LibreWolf and Waterfox.
Safari: see docs/BROWSER_EXTENSIONS.md (needs Xcode on a Mac).
"""

import json
import os
import re
import shutil
import sys
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXT = os.path.join(ROOT, "extension")
OUT = os.path.join(ROOT, "dist", "extension")
sys.path.insert(0, os.path.join(ROOT, "src"))


def merge(a, b):
    out = dict(a)
    for k, v in b.items():
        out[k] = merge(out[k], v) if isinstance(v, dict) and isinstance(out.get(k), dict) else v
    return out


def app_version():
    with open(os.path.join(ROOT, "src", "otm", "__init__.py"), encoding="utf-8") as f:
        return re.search(r'__version__ = "([^"]+)"', f.read()).group(1)


def build(family):
    with open(os.path.join(EXT, "manifest.base.json"), encoding="utf-8") as f:
        base = json.load(f)
    with open(os.path.join(EXT, "manifest.%s.json" % family), encoding="utf-8") as f:
        extra = json.load(f)
    manifest = merge(base, extra)
    manifest["version"] = app_version()
    target = os.path.join(OUT, family)
    shutil.rmtree(target, ignore_errors=True)
    shutil.copytree(os.path.join(EXT, "src"), target)
    with open(os.path.join(target, "manifest.json"), "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2)
    zpath = os.path.join(OUT, "open-transfer-manager-%s-%s.zip" % (family, manifest["version"]))
    with zipfile.ZipFile(zpath, "w", zipfile.ZIP_DEFLATED) as z:
        for dirpath, _dirs, files in os.walk(target):
            for name in sorted(files):
                full = os.path.join(dirpath, name)
                z.write(full, os.path.relpath(full, target).replace(os.sep, "/"))
    return target, zpath


def main():
    os.makedirs(OUT, exist_ok=True)
    for family in ("chromium", "firefox"):
        folder, z = build(family)
        print("%-9s %s\n          %s" % (family, folder, z))


if __name__ == "__main__":
    main()
