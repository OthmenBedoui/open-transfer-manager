<p align="center">
  <img src="assets/cover.webp" alt="Open Transfer Manager — Transfer, Manage, Monitor" width="100%">
</p>

<p align="center">
  A fast, light and open-source download manager for <b>Windows</b>, <b>Linux</b> and <b>macOS</b>.
</p>

## Features

- **Fast downloads** — several connections per file, pause and resume.
- **Video and MP3** — from YouTube and 1000+ sites, choose the quality or MP3 (320 / 192 / 128 kbps).
- **Browser extension** — Chrome, Edge, Brave and Firefox: send downloads to the app, Download button on videos.
- **Organized** — files sorted by type (Video, Music, Documents…), queue, scheduler and speed limit.
- **Light and dark themes** — one small native app, no account, no ads, no tracking.

## Install

### Windows

1. Download `OTM-Setup-x.y.z.exe` from [Releases](https://github.com/OthmenBedoui/open-transfer-manager/releases).
2. Run it and follow the steps. No administrator rights needed.

### Linux

1. Download `otm-linux-x86_64` from [Releases](https://github.com/OthmenBedoui/open-transfer-manager/releases).
2. Rename it, make it runnable and start it:

```bash
mv otm-linux-x86_64 otm
chmod +x otm
./otm
```

### macOS

1. Download `otm-macos-arm64` from [Releases](https://github.com/OthmenBedoui/open-transfer-manager/releases).
2. The app is not signed yet, so allow it once, then start it:

```bash
mv otm-macos-arm64 otm
chmod +x otm
xattr -d com.apple.quarantine otm
./otm
```

### For videos and MP3 (all systems)

Install **yt-dlp** and **ffmpeg** once:

| System | Command |
|---|---|
| Windows | `winget install yt-dlp.yt-dlp` then `winget install Gyan.FFmpeg` |
| Linux (Debian/Ubuntu) | `sudo apt install ffmpeg` then `./otm update-ytdlp` |
| macOS | `brew install yt-dlp ffmpeg` |

## Use

1. Click **New download** (or paste a link with Ctrl+V / Cmd+V).
2. For a video page, choose a quality or **Audio only** for MP3.
3. Click **Download**. Files go to your Downloads folder, sorted by type.

Command line (on Linux and macOS, write `./otm` if `otm` is not in your PATH):

```bash
otm add https://example.com/file.zip           # download a file
otm add https://youtu.be/VIDEO_ID --mp3 192    # video to MP3
otm doctor                                     # check that everything is set up
```

## Browser extension

Install the extension in your browser (see [the guide](docs/BROWSER_EXTENSIONS.md)).
On Windows the setup connects it for you; on Linux and macOS, run once:

```bash
otm register-browsers
```

## Build from source

Needs [Rust](https://rustup.rs) (stable):

```bash
git clone https://github.com/OthmenBedoui/open-transfer-manager
cd open-transfer-manager
cargo build --release
./target/release/otm
```

On Linux, install the window libraries first:
`sudo apt install libxkbcommon-dev libwayland-dev libx11-dev libgl1-mesa-dev`.

## License

[MIT](LICENSE). Only download content you have the right to download.
Documentation en français : [docs/README.fr.md](docs/README.fr.md).
