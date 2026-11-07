<p align="center">
  <img src="assets/cover.webp" alt="Open Transfer Manager — Transfer, Manage, Monitor" width="100%">
</p>

<p align="center">
  A fast, light and open-source download manager for <b>Windows</b>, <b>Linux</b> and <b>macOS</b>.
</p>

## Features

- **Fast downloads** — several connections per file, pause and resume.
- **Video and MP3** — from YouTube and 1000+ sites, choose the quality or MP3 (320 / 192 / 128 kbps).
- **Send anywhere** — drop files and upload them to several SFTP, FTP, WebDAV (Nextcloud) or S3 servers at once.
- **Browser extension** — Chrome, Edge, Brave and Firefox: send downloads to the app, Download button on videos.
- **Organized** — files sorted by type (Video, Music, Documents…), queue, scheduler and speed limit.
- **Light and dark themes** — one small native app, no account, no ads, no tracking.

## Install

All files are on the [Releases](https://github.com/OthmenBedoui/open-transfer-manager/releases) page.
For videos and MP3, OTM also needs **ffmpeg** and **yt-dlp**: each section below shows how to get them.

### Windows 10 / 11

1. Download `OTM-Setup-x.y.z.exe` and run it. No administrator rights needed.
2. Open **Open Transfer Manager** from the Start menu.
3. Videos and MP3 — in a terminal:

```powershell
winget install yt-dlp.yt-dlp
winget install Gyan.FFmpeg
```

Then close and reopen OTM so it finds them.

### Linux and macOS — one command

Open a terminal and paste:

```bash
curl -fsSL https://raw.githubusercontent.com/OthmenBedoui/open-transfer-manager/main/install.sh | bash
```

It downloads the app (and checks its SHA-256), adds it to your applications menu
(Linux) or `~/Applications` (macOS), offers to install ffmpeg, sets up yt-dlp and
connects your browsers. No `sudo` for the app itself. Then open
**Open Transfer Manager** from your menu, or type `otm`.

Works on **Ubuntu** 22.04+, **Linux Mint** 21+, **Pop!_OS**, **Zorin**, **elementary OS**,
**Debian** 12+, **Fedora**, **Arch**, **Manjaro**, **EndeavourOS**, **openSUSE** Tumbleweed,
and **macOS** on Apple Silicon (M1–M4). On other systems (Intel Mac, ARM, older
distributions) the installer offers to build the app from source.

Uninstall (keeps your files and settings; add `--purge` to remove settings too):

```bash
curl -fsSL https://raw.githubusercontent.com/OthmenBedoui/open-transfer-manager/main/install.sh | bash -s -- --uninstall
```

<details>
<summary>Manual install (without the script)</summary>

1. Download `otm-linux-x86_64` (Linux) or `otm-macos-arm64` (macOS) from Releases, then:

```bash
cd ~/Downloads
mv otm-linux-x86_64 otm                 # or: mv otm-macos-arm64 otm
chmod +x otm
xattr -d com.apple.quarantine otm       # macOS only: the app is not signed yet
sudo mv otm /usr/local/bin/
otm                                     # start the app
```

2. macOS: get videos and MP3 with [Homebrew](https://brew.sh): `brew install yt-dlp ffmpeg`.
3. Linux: get ffmpeg for your distribution, and the latest yt-dlp:

| Distribution | ffmpeg | yt-dlp |
|---|---|---|
| **Ubuntu** 22.04+, **Linux Mint** 21+, **Pop!_OS**, **Zorin**, **elementary OS** | `sudo apt install ffmpeg` | `otm update-ytdlp` |
| **Debian** 12+ | `sudo apt install ffmpeg` | `otm update-ytdlp` |
| **Fedora** | `sudo dnf install ffmpeg-free` | `otm update-ytdlp` |
| **Arch**, **Manjaro**, **EndeavourOS** | `sudo pacman -S ffmpeg` | `otm update-ytdlp` |
| **openSUSE** Tumbleweed | `sudo zypper install ffmpeg` | `otm update-ytdlp` |

`otm update-ytdlp` downloads the official yt-dlp for OTM. It is better than the
distribution package, which is often too old for YouTube.

The ready-made Linux file needs a 64-bit PC with glibc 2.35 or newer. Older distributions
(Ubuntu 20.04, Debian 11, RHEL / Rocky / Alma 8–9, openSUSE Leap 15), Alpine and ARM
computers: see [Build from source](#build-from-source), or use the script, which builds for you.

</details>

## Use

1. Click **New download** (or paste a link with Ctrl+V / Cmd+V).
2. For a video page, choose a quality or **Audio only** for MP3.
3. Click **Download**. Files go to your Downloads folder, sorted by type.

Command line:

```bash
otm add https://example.com/file.zip           # download a file
otm add https://youtu.be/VIDEO_ID --mp3 192    # video to MP3
otm send build.zip --to prod-server,backup-s3  # upload to saved destinations
otm doctor                                     # check that everything is set up
```

### Send files

Open **Connectors**, add a server (SFTP, FTP/FTPS, WebDAV or S3-compatible) and click
**Test connection**. Then drop files on **Send**, tick one or more destinations and click
**Send**. Uploads run through [rclone](https://rclone.org); OTM installs it for you the
first time (checked against its published SHA-256). Passwords and keys stay in Windows
Credential Manager, the macOS Keychain or the Linux Secret Service.

## Browser extension

Install the extension in your browser (see [the guide](docs/BROWSER_EXTENSIONS.md)).
The Windows setup and the Linux/macOS script connect it for you. If you install a new
browser later, run once:

```bash
otm register-browsers
```

## Build from source

For systems without a ready-made download (Intel Mac, older Linux, ARM).
Needs [Rust](https://rustup.rs) (stable) and Git.

On Linux, install the build tools first:

| Distribution | Command |
|---|---|
| Ubuntu / Debian / Mint | `sudo apt install build-essential pkg-config libxkbcommon-dev libwayland-dev libx11-dev libgl1-mesa-dev` |
| Fedora / RHEL / Rocky | `sudo dnf install gcc pkgconf libxkbcommon-devel wayland-devel libX11-devel mesa-libGL-devel` |
| Arch / Manjaro | `sudo pacman -S --needed base-devel libxkbcommon wayland libx11 mesa` |
| openSUSE | `sudo zypper install gcc pkgconf libxkbcommon-devel wayland-devel libX11-devel Mesa-libGL-devel` |

On macOS: `xcode-select --install`. Then, on any system:

```bash
git clone https://github.com/OthmenBedoui/open-transfer-manager
cd open-transfer-manager
cargo build --release
./target/release/otm
```

On Windows, `scripts/install-windows.ps1` builds the setup (see [docs/WINDOWS.md](docs/WINDOWS.md)).

## License

[MIT](LICENSE). Only download content you have the right to download.
Documentation en français : [docs/README.fr.md](docs/README.fr.md).
