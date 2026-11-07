#!/usr/bin/env bash
# Open Transfer Manager — one-command installer for Linux and macOS.
#
#   curl -fsSL https://raw.githubusercontent.com/OthmenBedoui/open-transfer-manager/main/install.sh | bash
#
# Options (when piping, put them after `bash -s --`):
#   --yes            answer yes to every question
#   --no-ffmpeg      do not offer to install ffmpeg
#   --no-browsers    do not connect the browser extension
#   --version TAG    install a given release (e.g. v0.2.6) instead of the latest
#   --from-source    build with cargo (Intel Mac, ARM, older Linux)
#   --uninstall      remove the app (keeps your settings and downloads)
#   --purge          with --uninstall: also delete settings and download list
#
# Installs for the current user only (no sudo): ~/.local/bin/otm, plus a menu entry
# (Linux) or ~/Applications/Open Transfer Manager.app (macOS), then yt-dlp, rclone and
# ffmpeg (each checked against its official checksum). sudo is only used to install
# ffmpeg / secret-tool with your package manager, and only if you agree.
#
# Testing hook: OTM_BINARY=/path/to/otm installs that file instead of downloading.

set -euo pipefail

REPO="${OTM_REPO:-OthmenBedoui/open-transfer-manager}"
APP_NAME="Open Transfer Manager"
APP_ID="open-transfer-manager"
MIN_GLIBC="2.35"

YES=0; FFMPEG=1; BROWSERS=1; TAG=""; FROM_SOURCE=0; UNINSTALL=0; PURGE=0
while [ $# -gt 0 ]; do
  case "$1" in
    -y|--yes) YES=1 ;;
    --no-ffmpeg) FFMPEG=0 ;;
    --no-browsers) BROWSERS=0 ;;
    --version) TAG="${2:?--version needs a tag, e.g. v0.2.6}"; shift ;;
    --from-source) FROM_SOURCE=1 ;;
    --uninstall) UNINSTALL=1 ;;
    --purge) PURGE=1 ;;
    -h|--help) sed -n '2,20p' "$0" 2>/dev/null || true; exit 0 ;;
    *) echo "Unknown option: $1 (see --help)" >&2; exit 2 ;;
  esac
  shift
done

# ------------------------------------------------------------------ output helpers
if [ -t 1 ]; then
  B=$'\033[1m'; G=$'\033[32m'; Y=$'\033[33m'; R=$'\033[31m'; C=$'\033[36m'; N=$'\033[0m'
else
  B=""; G=""; Y=""; R=""; C=""; N=""
fi
step() { printf '%s==>%s %s%s%s\n' "$C" "$N" "$B" "$*" "$N"; }
ok()   { printf '  %s✓%s %s\n' "$G" "$N" "$*"; }
warn() { printf '  %s!%s %s\n' "$Y" "$N" "$*"; }
die()  { printf '%sError:%s %s\n' "$R" "$N" "$*" >&2; exit 1; }

# ask "question" -> 0 = yes. Reads the keyboard even when the script is piped into bash.
ask() {
  [ "$YES" = 1 ] && return 0
  if [ -r /dev/tty ] && [ -w /dev/tty ]; then
    printf '  %s?%s %s [Y/n] ' "$Y" "$N" "$1" > /dev/tty
    local reply=""
    read -r reply < /dev/tty || reply=""
    case "$reply" in [nN]*) return 1 ;; *) return 0 ;; esac
  fi
  return 1
}

fetch() { # fetch URL FILE
  if command -v curl >/dev/null 2>&1; then curl -fsSL --retry 3 -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then wget -q -O "$2" "$1"
  else die "curl or wget is needed."; fi
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

version_ge() { # version_ge A B -> A >= B
  [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" = "$2" ]
}

# ------------------------------------------------------------------ where things go
OS="$(uname -s)"; ARCH="$(uname -m)"
BIN_DIR="$HOME/.local/bin"
BIN="$BIN_DIR/otm"
case "$OS" in
  Linux)
    SHARE="${XDG_DATA_HOME:-$HOME/.local/share}"
    DATA="$SHARE/$APP_ID"
    DESKTOP="$SHARE/applications/$APP_ID.desktop"
    ICON="$SHARE/icons/hicolor/256x256/apps/$APP_ID.png"
    ;;
  Darwin)
    DATA="$HOME/Library/Application Support/$APP_NAME"
    MACAPP="$HOME/Applications/$APP_NAME.app"
    ;;
  *) die "This installer is for Linux and macOS. On Windows, use OTM-Setup from the Releases page." ;;
esac

# ------------------------------------------------------------------ uninstall
if [ "$UNINSTALL" = 1 ]; then
  step "Removing $APP_NAME"
  if [ -x "$BIN" ]; then "$BIN" unregister-browsers >/dev/null 2>&1 || true; fi
  rm -f "$BIN"
  if [ "$OS" = Linux ]; then
    rm -f "$DESKTOP" "$ICON"
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$(dirname "$DESKTOP")" >/dev/null 2>&1 || true
  else
    rm -rf "$MACAPP"
  fi
  if [ "$PURGE" = 1 ]; then rm -rf "$DATA"; ok "Settings and download list deleted"
  else ok "Settings and download list kept in: $DATA"; fi
  ok "Done. Your downloaded files were not touched."
  exit 0
fi

printf '\n%s%s installer%s (%s %s)\n\n' "$B" "$APP_NAME" "$N" "$OS" "$ARCH"

# ------------------------------------------------------------------ which file?
ASSET=""
if [ "$FROM_SOURCE" = 0 ] && [ -z "${OTM_BINARY:-}" ]; then
  case "$OS-$ARCH" in
    Linux-x86_64|Linux-amd64) ASSET="otm-linux-x86_64" ;;
    Darwin-arm64|Darwin-aarch64) ASSET="otm-macos-arm64" ;;
  esac
  if [ -z "$ASSET" ]; then
    warn "No ready-made download for $OS $ARCH yet."
    ask "Build it from source with Rust instead (takes a few minutes)?" || die "Nothing installed."
    FROM_SOURCE=1
  fi
  if [ "$OS" = Linux ] && [ "$FROM_SOURCE" = 0 ]; then
    glibc="$(ldd --version 2>/dev/null | head -n1 | grep -oE '[0-9]+\.[0-9]+$' || true)"
    if [ -z "$glibc" ]; then
      warn "This system does not use glibc (Alpine?): the download will not run."
      ask "Build it from source with Rust instead?" || die "Nothing installed."
      FROM_SOURCE=1
    elif ! version_ge "$glibc" "$MIN_GLIBC"; then
      warn "glibc $glibc is older than $MIN_GLIBC: the download will not run on this system."
      ask "Build it from source with Rust instead?" || die "Nothing installed."
      FROM_SOURCE=1
    fi
  fi
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$BIN_DIR"

# ------------------------------------------------------------------ get the app
if [ -n "${OTM_BINARY:-}" ]; then
  step "Installing local file $OTM_BINARY"
  cp "$OTM_BINARY" "$TMP/otm"
elif [ "$FROM_SOURCE" = 1 ]; then
  step "Building from source"
  command -v git >/dev/null 2>&1 || die "git is needed (install it with your package manager)."
  if ! command -v cargo >/dev/null 2>&1; then
    [ -x "$HOME/.cargo/bin/cargo" ] && export PATH="$HOME/.cargo/bin:$PATH"
  fi
  command -v cargo >/dev/null 2>&1 || die "Rust is needed: https://rustup.rs (then run this installer again)."
  ref="${TAG:-main}"
  git clone --depth 1 --branch "$ref" "https://github.com/$REPO" "$TMP/src" >/dev/null 2>&1 \
    || die "Could not download the source code ($ref)."
  ( cd "$TMP/src" && cargo build --release --locked ) || die "Build failed. Linux build tools are listed in the README."
  cp "$TMP/src/target/release/otm" "$TMP/otm"
else
  if [ -n "$TAG" ]; then base="https://github.com/$REPO/releases/download/$TAG"
  else base="https://github.com/$REPO/releases/latest/download"; fi
  step "Downloading $ASSET"
  fetch "$base/$ASSET" "$TMP/otm" || die "Download failed: $base/$ASSET"
  if fetch "$base/SHA256SUMS.txt" "$TMP/SHA256SUMS.txt" 2>/dev/null; then
    want="$(grep " $ASSET\$" "$TMP/SHA256SUMS.txt" | cut -d' ' -f1 || true)"
    [ -n "$want" ] || die "No checksum for $ASSET in SHA256SUMS.txt."
    [ "$(sha256 "$TMP/otm")" = "$want" ] || die "Checksum mismatch: the download is damaged or was changed."
    ok "Checksum verified"
  else
    warn "SHA256SUMS.txt not found: checksum not verified."
  fi
fi

chmod +x "$TMP/otm"
[ "$OS" = Darwin ] && xattr -d com.apple.quarantine "$TMP/otm" >/dev/null 2>&1 || true
"$TMP/otm" --version >/dev/null 2>&1 || die "The app does not start on this system. Try: --from-source"
mv -f "$TMP/otm" "$BIN"
ok "$("$BIN" --version)"
ok "Command: $BIN"

# ------------------------------------------------------------------ menu entry / app bundle
ICON_URL="https://raw.githubusercontent.com/$REPO/${TAG:-main}/assets/otm.png"
if [ "$OS" = Linux ]; then
  mkdir -p "$(dirname "$DESKTOP")" "$(dirname "$ICON")"
  fetch "$ICON_URL" "$ICON" 2>/dev/null || warn "Icon not downloaded (menu entry without icon)."
  cat > "$DESKTOP" <<EOF
[Desktop Entry]
Type=Application
Name=$APP_NAME
GenericName=Download Manager
Comment=Fast downloads, video and MP3 from 1000+ sites
Exec="$BIN" %U
Icon=$APP_ID
Terminal=false
Categories=Network;FileTransfer;
Keywords=download;downloader;youtube;mp3;video;
StartupWMClass=$APP_ID
EOF
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$(dirname "$DESKTOP")" >/dev/null 2>&1 || true
  ok "Added to your applications menu"
else
  rm -rf "$MACAPP"
  mkdir -p "$MACAPP/Contents/MacOS" "$MACAPP/Contents/Resources"
  printf '#!/bin/sh\nexec "%s" "$@"\n' "$BIN" > "$MACAPP/Contents/MacOS/$APP_ID"
  chmod +x "$MACAPP/Contents/MacOS/$APP_ID"
  if fetch "$ICON_URL" "$TMP/icon.png" 2>/dev/null && command -v sips >/dev/null 2>&1 && command -v iconutil >/dev/null 2>&1; then
    set_dir="$TMP/otm.iconset"; mkdir -p "$set_dir"
    for s in 16 32 128 256; do
      sips -z "$s" "$s" "$TMP/icon.png" --out "$set_dir/icon_${s}x${s}.png" >/dev/null 2>&1 || true
      d=$((s * 2)); sips -z "$d" "$d" "$TMP/icon.png" --out "$set_dir/icon_${s}x${s}@2x.png" >/dev/null 2>&1 || true
    done
    iconutil -c icns "$set_dir" -o "$MACAPP/Contents/Resources/otm.icns" >/dev/null 2>&1 || true
  fi
  version="$("$BIN" --version | awk '{print $NF}')"
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
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
EOF
  ok "App: $MACAPP"
fi

# ------------------------------------------------------------------ system packages
# ffmpeg (MP3, best video quality) and, on Linux, secret-tool (keeps upload passwords in
# the desktop keyring). Both come from your package manager, only if you agree; without
# them OTM still works: install-tools below fetches a checked static ffmpeg on Linux, and
# passwords then go to a private file.
pkgs=()
if [ "$FFMPEG" = 1 ] && ! command -v ffmpeg >/dev/null 2>&1; then pkgs+=(ffmpeg); fi
need_secret=0
if [ "$OS" = Linux ] && ! command -v secret-tool >/dev/null 2>&1; then need_secret=1; fi
# The window needs libxkbcommon-x11 (present on desktops, missing on minimal installs).
need_xkb=0
if [ "$OS" = Linux ] && command -v ldconfig >/dev/null 2>&1    && ! ldconfig -p 2>/dev/null | grep -q 'libxkbcommon-x11\.so'; then need_xkb=1; fi
cmd=""
if [ "$OS" = Darwin ]; then
  if [ "${#pkgs[@]}" -gt 0 ] && command -v brew >/dev/null 2>&1; then cmd="brew install ffmpeg"; fi
elif command -v apt-get >/dev/null 2>&1; then
  if [ "$need_secret" = 1 ]; then pkgs+=(libsecret-tools); fi
  if [ "$need_xkb" = 1 ]; then pkgs+=(libxkbcommon-x11-0); fi
  if [ "${#pkgs[@]}" -gt 0 ]; then cmd="sudo apt-get install -y ${pkgs[*]}"; fi
elif command -v dnf >/dev/null 2>&1; then
  if [ "${#pkgs[@]}" -gt 0 ]; then pkgs=(ffmpeg-free); fi
  if [ "$need_secret" = 1 ]; then pkgs+=(libsecret); fi
  if [ "$need_xkb" = 1 ]; then pkgs+=(libxkbcommon-x11); fi
  if [ "${#pkgs[@]}" -gt 0 ]; then cmd="sudo dnf install -y ${pkgs[*]}"; fi
elif command -v pacman >/dev/null 2>&1; then
  if [ "$need_secret" = 1 ]; then pkgs+=(libsecret); fi
  if [ "$need_xkb" = 1 ]; then pkgs+=(libxkbcommon-x11); fi
  if [ "${#pkgs[@]}" -gt 0 ]; then cmd="sudo pacman -S --needed --noconfirm ${pkgs[*]}"; fi
elif command -v zypper >/dev/null 2>&1; then
  if [ "$need_xkb" = 1 ]; then pkgs+=(libxkbcommon-x11-0); fi
  if [ "${#pkgs[@]}" -gt 0 ]; then cmd="sudo zypper --non-interactive install ${pkgs[*]}"; fi
fi
if [ -n "$cmd" ]; then
  step "System packages ($cmd)"
  if ask "Install them now?"; then
    sh -c "$cmd" || warn "Not installed; OTM uses its own copies where it can."
  else
    warn "Skipped. Install later with: $cmd"
  fi
fi

# ------------------------------------------------------------------ yt-dlp, rclone, ffmpeg
step "Helper programs: yt-dlp (videos, MP3), rclone (Send, Transfer), ffmpeg"
tool_arg="--with-ffmpeg"
if [ "$FFMPEG" = 0 ]; then tool_arg="--no-ffmpeg"; fi
if "$BIN" install-tools "$tool_arg" | sed 's/^/  /'; then
  ok "All helper programs are ready"
else
  warn "Some helper programs are missing; run later: otm install-tools"
fi

if [ "$BROWSERS" = 1 ]; then
  step "Browser extension bridge"
  "$BIN" register-browsers | sed 's/^ */  /' || warn "Run later: otm register-browsers"
fi

# ------------------------------------------------------------------ done
printf '\n%s%s is installed.%s\n\n' "$G$B" "$APP_NAME" "$N"
echo "  Open it from your applications menu, or run:  otm"
echo "  Check everything:                            otm doctor"
echo "  Uninstall:  curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | bash -s -- --uninstall"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) printf "\n  %sNote:%s add %s to your PATH to use the otm command:\n    echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.%src\n" \
       "$Y" "$N" "$BIN_DIR" "$(basename "${SHELL:-bash}")" ;;
esac
echo
