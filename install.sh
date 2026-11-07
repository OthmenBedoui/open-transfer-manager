#!/usr/bin/env bash
# Open Transfer Manager - installer for Linux and macOS.
#
#   curl -fsSL https://raw.githubusercontent.com/OthmenBedoui/open-transfer-manager/main/install.sh | bash
#
# Options (when piping, pass them after `bash -s --`):
#   --yes            answer yes to every question (unattended install)
#   --with-ffmpeg    also install ffmpeg (best video quality, mp3)    [asked by default]
#   --no-ffmpeg      never install ffmpeg
#   --no-browsers    do not connect the browser extension bridge
#   --ref REF        install a tag or branch (default: main)
#   --local DIR      install from a local clone instead of GitHub
#   --uninstall      remove the app (keeps your settings and download list)
#   --purge          with --uninstall: also delete settings and download list
#
# Everything is installed for the current user only: no files outside your home folder,
# except the system packages (Python/Tk/ffmpeg) when they are missing and you agree.

set -euo pipefail

REPO="${OTM_REPO:-OthmenBedoui/open-transfer-manager}"
REF="${OTM_REF:-main}"
APP_NAME="Open Transfer Manager"
APP_ID="open-transfer-manager"

YES=0; FFMPEG="ask"; BROWSERS=1; LOCAL=""; UNINSTALL=0; PURGE=0

while [ $# -gt 0 ]; do
  case "$1" in
    -y|--yes) YES=1 ;;
    --with-ffmpeg) FFMPEG="yes" ;;
    --no-ffmpeg) FFMPEG="no" ;;
    --no-browsers) BROWSERS=0 ;;
    --ref) REF="$2"; shift ;;
    --local) LOCAL="$(cd "$2" && pwd)"; shift ;;
    --uninstall) UNINSTALL=1 ;;
    --purge) PURGE=1 ;;
    -h|--help) sed -n '2,20p' "$0" 2>/dev/null || true; exit 0 ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

# ------------------------------------------------------------------ output
if [ -t 1 ]; then
  B="$(printf '\033[1m')"; G="$(printf '\033[32m')"; Y="$(printf '\033[33m')"; R="$(printf '\033[31m')"
  C="$(printf '\033[36m')"; N="$(printf '\033[0m')"
else
  B=""; G=""; Y=""; R=""; C=""; N=""
fi
step() { printf '%s==>%s %s%s%s\n' "$C" "$N" "$B" "$*" "$N"; }
ok()   { printf '  %s✓%s %s\n' "$G" "$N" "$*"; }
warn() { printf '  %s!%s %s\n' "$Y" "$N" "$*"; }
die()  { printf '%sError:%s %s\n' "$R" "$N" "$*" >&2; exit 1; }

# ask "question" -> 0 for yes. Reads the terminal even when the script is piped.
ask() {
  [ "$YES" = 1 ] && return 0
  if [ -r /dev/tty ] && [ -w /dev/tty ]; then
    printf '  %s?%s %s [Y/n] ' "$Y" "$N" "$1" > /dev/tty
    read -r reply < /dev/tty || reply=""
    case "$reply" in [nN]*) return 1 ;; *) return 0 ;; esac
  fi
  return 1
}

need_sudo() {
  if [ "$(id -u)" = 0 ]; then SUDO=""; return; fi
  command -v sudo >/dev/null 2>&1 || die "sudo is needed to install system packages. Install them yourself, then run this again."
  SUDO="sudo"
}

# ------------------------------------------------------------------ paths
OS="$(uname -s)"
case "$OS" in
  Linux)
    DATA="${XDG_DATA_HOME:-$HOME/.local/share}/$APP_ID"
    APPS="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    ICONS="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"
    ;;
  Darwin)
    DATA="$HOME/Library/Application Support/$APP_NAME"
    MACAPP="$HOME/Applications/$APP_NAME.app"
    ;;
  *) die "This installer supports Linux and macOS (found $OS). On Windows: pip install open-transfer-manager" ;;
esac
BIN="$HOME/.local/bin"
VENV="$DATA/venv"

# ------------------------------------------------------------------ uninstall
if [ "$UNINSTALL" = 1 ]; then
  step "Removing $APP_NAME"
  if [ -x "$VENV/bin/otm" ]; then "$VENV/bin/otm" unregister-browsers >/dev/null 2>&1 || true; fi
  rm -rf "${VENV:?}" "${DATA:?}/bin"
  rm -f "$BIN/otm"
  if [ "$OS" = Linux ]; then
    rm -f "$APPS/$APP_ID.desktop" "$ICONS/$APP_ID.png"
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS" >/dev/null 2>&1 || true
  else
    rm -rf "$MACAPP"
  fi
  if [ "$PURGE" = 1 ]; then rm -rf "$DATA"; ok "Settings and download list deleted"; else
    ok "Your settings and download list were kept in: $DATA"; fi
  ok "Done. Your downloaded files were not touched."
  exit 0
fi

printf '\n%s%s installer%s  (%s)\n\n' "$B" "$APP_NAME" "$N" "$OS"

# ------------------------------------------------------------------ python + tk
python_ok() {
  "$1" - <<'EOF' >/dev/null 2>&1
import sys
assert sys.version_info >= (3, 9)
import tkinter, venv, ensurepip  # noqa
tkinter.Tcl()
EOF
}

find_python() {
  local c
  for c in python3.14 python3.13 python3.12 python3.11 python3.10 python3.9 python3 \
           /opt/homebrew/bin/python3 /usr/local/bin/python3 /usr/bin/python3; do
    if command -v "$c" >/dev/null 2>&1 && python_ok "$(command -v "$c")"; then
      PY="$(command -v "$c")"; return 0
    fi
  done
  return 1
}

PKG=""
detect_pkg() {
  for p in apt-get dnf pacman zypper apk xbps-install brew; do
    if command -v "$p" >/dev/null 2>&1; then PKG="$p"; return; fi
  done
}

install_python() {
  detect_pkg
  case "$PKG" in
    apt-get) CMD="apt-get update && apt-get install -y python3 python3-venv python3-tk" ;;
    dnf)     CMD="dnf install -y python3 python3-tkinter" ;;
    pacman)  CMD="pacman -S --needed --noconfirm python tk" ;;
    zypper)  CMD="zypper install -y python3 python3-tk" ;;
    apk)     CMD="apk add python3 py3-tkinter" ;;
    xbps-install) CMD="xbps-install -Sy python3 python3-tkinter" ;;
    brew)    CMD="brew install python python-tk" ;;
    *) die "Python 3.9+ with Tk was not found. Install Python 3 and Tkinter, then run this again." ;;
  esac
  warn "Python 3.9+ with Tkinter is needed."
  if ! ask "Install it now with: $CMD ?"; then
    die "Please run:  $CMD   then run this installer again."
  fi
  if [ "$PKG" = brew ]; then sh -c "$CMD"; else need_sudo; $SUDO sh -c "$CMD"; fi
}

step "Checking Python and Tk"
if ! find_python; then
  if [ "$OS" = Darwin ] && ! command -v brew >/dev/null 2>&1; then
    die "Install Python from https://www.python.org/downloads/macos/ (it includes Tk) or Homebrew (https://brew.sh), then run this again."
  fi
  install_python
  find_python || die "Python with Tk is still missing. Please install it manually."
fi
ok "$("$PY" --version 2>&1) at $PY"

# ------------------------------------------------------------------ ffmpeg (optional)
if ! command -v ffmpeg >/dev/null 2>&1 && [ "$FFMPEG" != no ]; then
  detect_pkg
  case "$PKG" in
    apt-get) FCMD="apt-get install -y ffmpeg" ;;
    dnf)     FCMD="dnf install -y ffmpeg-free" ;;
    pacman)  FCMD="pacman -S --needed --noconfirm ffmpeg" ;;
    zypper)  FCMD="zypper install -y ffmpeg" ;;
    apk)     FCMD="apk add ffmpeg" ;;
    xbps-install) FCMD="xbps-install -Sy ffmpeg" ;;
    brew)    FCMD="brew install ffmpeg" ;;
    *)       FCMD="" ;;
  esac
  step "ffmpeg (optional: best video quality and mp3 conversion)"
  if [ -n "$FCMD" ] && { [ "$FFMPEG" = yes ] || ask "Install ffmpeg with: $FCMD ?"; }; then
    if [ "$PKG" = brew ]; then sh -c "$FCMD" || warn "ffmpeg install failed, continuing";
    else need_sudo; $SUDO sh -c "$FCMD" || warn "ffmpeg install failed, continuing"; fi
  else
    warn "Skipped. You can install ffmpeg later at any time."
  fi
fi

# ------------------------------------------------------------------ app
step "Installing $APP_NAME"
mkdir -p "$DATA" "$BIN"
rm -rf "$VENV"
"$PY" -m venv "$VENV"
"$VENV/bin/python" -m pip install --quiet --disable-pip-version-check --upgrade pip
if [ -n "$LOCAL" ]; then
  SRC="$LOCAL"
else
  SRC="https://github.com/$REPO/archive/$REF.tar.gz"
fi
"$VENV/bin/python" -m pip install --quiet --disable-pip-version-check "$SRC" \
  || die "Could not install from $SRC"
VERSION="$("$VENV/bin/otm" --version)"
ok "$VERSION"
ok "yt-dlp $("$VENV/bin/python" -c 'import yt_dlp; print(yt_dlp.version.__version__)' 2>/dev/null || echo 'not installed')"
ln -sf "$VENV/bin/otm" "$BIN/otm"
ok "Command: $BIN/otm"

ICON_SRC="$("$VENV/bin/python" -c 'import os, otm; print(os.path.join(os.path.dirname(otm.__file__), "assets", "otm.png"))')"

if [ "$OS" = Linux ]; then
  mkdir -p "$APPS" "$ICONS"
  cp "$ICON_SRC" "$ICONS/$APP_ID.png"
  cat > "$APPS/$APP_ID.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=$APP_NAME
GenericName=Download Manager
Comment=Fast downloads, videos from 1000+ sites and browser integration
Exec="$VENV/bin/otm" %U
Icon=$APP_ID
Terminal=false
Categories=Network;FileTransfer;
Keywords=download;downloader;accelerator;youtube;video;yt-dlp;
StartupWMClass=Open-transfer-manager
EOF
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS" >/dev/null 2>&1 || true
  command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q "${ICONS%/256x256/apps}" >/dev/null 2>&1 || true
  ok "Added to your applications menu"
else
  rm -rf "$MACAPP"
  mkdir -p "$MACAPP/Contents/MacOS" "$MACAPP/Contents/Resources"
  cat > "$MACAPP/Contents/MacOS/$APP_ID" <<EOF
#!/bin/sh
exec "$VENV/bin/python" -m otm "\$@"
EOF
  chmod +x "$MACAPP/Contents/MacOS/$APP_ID"
  if command -v iconutil >/dev/null 2>&1 && command -v sips >/dev/null 2>&1; then
    SET="$(mktemp -d)/otm.iconset"; mkdir -p "$SET"
    for s in 16 32 128 256; do
      sips -z $s $s "$ICON_SRC" --out "$SET/icon_${s}x${s}.png" >/dev/null 2>&1 || true
      d=$((s * 2)); sips -z $d $d "$ICON_SRC" --out "$SET/icon_${s}x${s}@2x.png" >/dev/null 2>&1 || true
    done
    iconutil -c icns "$SET" -o "$MACAPP/Contents/Resources/otm.icns" >/dev/null 2>&1 || true
  fi
  cat > "$MACAPP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>$APP_NAME</string>
  <key>CFBundleDisplayName</key><string>$APP_NAME</string>
  <key>CFBundleIdentifier</key><string>org.opentransfermanager.app</string>
  <key>CFBundleExecutable</key><string>$APP_ID</string>
  <key>CFBundleIconFile</key><string>otm</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION##* }</string>
  <key>LSMinimumSystemVersion</key><string>10.13</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
EOF
  ok "App: $MACAPP"
fi

# ------------------------------------------------------------------ browsers
if [ "$BROWSERS" = 1 ]; then
  step "Connecting browsers"
  "$VENV/bin/otm" register-browsers | sed 's/^ */  /' || warn "Could not register the browser bridge (run: otm register-browsers)"
fi

# ------------------------------------------------------------------ done
printf '\n%s%s is installed.%s\n\n' "$G$B" "$APP_NAME" "$N"
echo "  Start it from your applications menu, or run:  otm"
echo "  Add a link from the terminal:                 otm add <URL>"
echo "  Check everything:                             otm doctor"
echo "  Browser extension:  https://github.com/$REPO#browser-extension"
case ":$PATH:" in
  *":$BIN:"*) ;;
  *) printf "\n  %sNote:%s %s is not in your PATH. Add this line to your shell profile:\n    export PATH=\"%s:\$PATH\"\n" "$Y" "$N" "$BIN" "$BIN" ;;
esac
echo
