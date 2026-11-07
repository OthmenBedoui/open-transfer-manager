# Architecture

```
src/otm/
  cli.py            `otm` command: GUI start, single instance, add/list/headless/doctor...
  paths.py          per-OS folders, open/reveal files, notifications
  server.py         local control API (127.0.0.1 + token) used by the bridge and a second `otm` launch
  native_host.py    native messaging host started by the browser
  browsers.py       registers the native host with every installed browser
  core/
    engine.py       HTTP engine: segments, resume, retries, limiter, queue, scheduler, persistence
    media.py        yt-dlp downloads (same interface as a plain download)
    defaults.py     default values
  ui/
    app.py          main window
    dialogs.py      new download, settings, details panel, about, diagnostics
    theme.py        light/dark palettes applied to ttk
    widgets.py      icon buttons, sidebar items, segment bar
    icons.py        vector icons drawn on Canvas
extension/          browser extension (one source, per-browser manifests)
install.sh          one-command installer for Linux and macOS
```

## Threads

* **UI thread** – Tk main loop; calls `Manager.tick()` every 500 ms.
* **Download threads** – one controller per running download, one thread per connection.
* **API threads** – one per local request. Anything that changes the list goes through `Manager.call()`,
  which runs it on the tick thread, so the list is only changed from one thread.

## Segmented downloads

A download starts with one connection. Once the size is known and the server accepts byte ranges, more connections
are opened by splitting the largest remaining part in half (never below 256 KB). Parts are written into one
pre-allocated `.otmpart` file, renamed when complete. Progress (`[start, end, position]` per part) is saved every
few seconds, so downloads resume after a restart.

## Files

| | Linux | macOS |
|---|---|---|
| Settings, list | `~/.local/share/open-transfer-manager/` | `~/Library/Application Support/Open Transfer Manager/` |
| App (installer) | `.../venv/` in the same folder | same |
| Command | `~/.local/bin/otm` | `~/.local/bin/otm` |
| Launcher | `~/.local/share/applications/open-transfer-manager.desktop` | `~/Applications/Open Transfer Manager.app` |

`settings.json` and `downloads.json` are written atomically, with a `.bak` copy used if a file is damaged.
`runtime.json` (mode 600) holds the API port and token while the app runs.
