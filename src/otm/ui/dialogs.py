"""Dialogs and the details panel."""

import contextlib
import io
import os
import re
import threading
import tkinter as tk
import webbrowser
from tkinter import filedialog, messagebox, ttk

from .. import APP_NAME, HOMEPAGE, __version__, browsers, paths
from ..core import defaults, engine, media
from . import icons, widgets


def open_url(url):
    try:
        webbrowser.open(url)
    except webbrowser.Error:
        pass


def _center(win, parent):
    win.update_idletasks()
    try:
        px, py = parent.winfo_rootx(), parent.winfo_rooty()
        pw, ph = parent.winfo_width(), parent.winfo_height()
        w, h = win.winfo_reqwidth(), win.winfo_reqheight()
        win.geometry("+%d+%d" % (px + max(0, (pw - w) // 2), py + max(0, (ph - h) // 3)))
    except tk.TclError:
        pass


class Dialog(tk.Toplevel):
    def __init__(self, app, title, resizable=False):
        super().__init__(app.root)
        self.withdraw()
        self.app = app
        self.t = app.theme
        self.title(title)
        self.transient(app.root)
        self.resizable(resizable, resizable)
        self.configure(bg=self.t.p["bg"])
        px = self.t.px
        self.body = ttk.Frame(self, padding=(px(22), px(18), px(22), px(10)))
        self.body.pack(fill="both", expand=True)
        self.footer = ttk.Frame(self, padding=(px(22), px(6), px(22), px(18)))
        self.footer.pack(fill="x")
        self.bind("<Escape>", lambda e: self.destroy())

    def button(self, text, command, accent=False, side="right"):
        b = ttk.Button(self.footer, text=text, command=command, style="Accent.TButton" if accent else "TButton")
        b.pack(side=side, padx=(self.t.px(8), 0) if side == "right" else (0, self.t.px(8)))
        return b

    def show(self, modal=True):
        _center(self, self.app.root)
        self.deiconify()
        self.lift()
        if modal:
            try:
                self.grab_set()
            except tk.TclError:
                pass
        self.focus_set()


def _row(parent, r, label, widget, hint=None, pad=6):
    ttk.Label(parent, text=label).grid(row=r, column=0, sticky="w", pady=pad, padx=(0, 14))
    narrow = isinstance(widget, (ttk.Combobox, ttk.Spinbox))  # small pickers keep their own width
    widget.grid(row=r, column=1, sticky="w" if narrow else "we", pady=pad)
    if hint:
        ttk.Label(parent, text=hint, style="Muted.TLabel").grid(row=r + 1, column=1, sticky="w")


# ===================================================================== add

class AddDialog(Dialog):
    def __init__(self, app, url="", headers=None):
        super().__init__(app, "New download")
        self.mgr = app.mgr
        self.headers = headers or {}
        self.info = None
        self.probe_id = 0
        if not url:
            try:
                clip = app.root.clipboard_get().strip()
                if clip.lower().startswith(("http://", "https://")) and "\n" not in clip:
                    url = clip
            except tk.TclError:
                pass
        px = self.t.px
        b = self.body
        b.columnconfigure(1, weight=1)
        ttk.Label(b, text="New download", style="Title.TLabel").grid(row=0, column=0, columnspan=3, sticky="w",
                                                                    pady=(0, px(12)))
        self.url = tk.StringVar(value=url)
        e = ttk.Entry(b, textvariable=self.url, width=64)
        _row(b, 1, "Link", e)
        e.focus_set()
        e.select_range(0, "end")

        self.mode = tk.StringVar(value="media" if media.looks_like_media_page(url) else "file")
        mf = ttk.Frame(b)
        ttk.Radiobutton(mf, text="File", value="file", variable=self.mode, command=self._mode_changed).pack(side="left")
        ttk.Radiobutton(mf, text="Video / audio (yt-dlp)", value="media", variable=self.mode,
                        command=self._mode_changed).pack(side="left", padx=(px(18), 0))
        _row(b, 2, "Type", mf)

        # file options
        self.file_box = ttk.Frame(b)
        self.file_box.columnconfigure(1, weight=1)
        name = engine.name_from_url(url) if url else ""
        cat = self.mgr.category_name_for(name) if name else defaults.GENERAL
        self.cat = tk.StringVar(value=cat)
        cb = ttk.Combobox(self.file_box, textvariable=self.cat, state="readonly", width=18,
                          values=[c.name for c in self.mgr.categories] + [defaults.GENERAL])
        cb.bind("<<ComboboxSelected>>", lambda e: self.folder.set(self.mgr.folder_for(self.cat.get())))
        _row(self.file_box, 0, "Category", cb)
        self.name = tk.StringVar(value=name)
        _row(self.file_box, 1, "File name", ttk.Entry(self.file_box, textvariable=self.name))

        # media options
        self.media_box = ttk.Frame(b)
        self.media_box.columnconfigure(1, weight=1)
        self.quality_keys = [k for k, _l in defaults.MEDIA_QUALITIES]
        self.quality = tk.StringVar(value=dict(defaults.MEDIA_QUALITIES).get(self.mgr.media_quality, "Best available"))
        qcb = ttk.Combobox(self.media_box, textvariable=self.quality, state="readonly", width=20,
                           values=[label for _k, label in defaults.MEDIA_QUALITIES])
        qcb.bind("<<ComboboxSelected>>", lambda e: self._quality_changed())
        _row(self.media_box, 0, "Quality", qcb)
        self.audio_fmt = tk.StringVar(value=self.mgr.media_audio_format)
        self.audio_row = ttk.Combobox(self.media_box, textvariable=self.audio_fmt, state="readonly", width=8,
                                      values=defaults.AUDIO_FORMATS)
        _row(self.media_box, 1, "Audio format", self.audio_row)
        self.playlist = tk.BooleanVar(value=self.mgr.media_playlist)
        ttk.Checkbutton(self.media_box, text="Download the whole playlist when the link is one",
                        variable=self.playlist).grid(row=2, column=1, sticky="w", pady=4)
        if not media.ffmpeg_path():
            ttk.Label(self.media_box, style="Muted.TLabel", wraplength=px(420),
                      text="ffmpeg is not installed: the best quality and mp3 conversion need it. "
                           "Videos will use the best single-file format.").grid(row=3, column=1, sticky="w")

        # common
        self.folder = tk.StringVar(value=self.mgr.folder_for(cat))
        ff = ttk.Frame(b)
        ff.columnconfigure(0, weight=1)
        ttk.Entry(ff, textvariable=self.folder).grid(row=0, column=0, sticky="we")
        ttk.Button(ff, text="Browse…", command=self._browse).grid(row=0, column=1, padx=(px(6), 0))
        self.folder_row = ff
        self.info_lbl = ttk.Label(b, text="", style="Muted.TLabel", wraplength=px(520))
        self.start_now = tk.BooleanVar(value=True)

        self.button("Download", self.ok, accent=True)
        self.button("Add to queue", self.later)
        self.button("Cancel", self.destroy, side="left")
        self.bind("<Return>", lambda e: self.ok())
        self.url.trace_add("write", lambda *a: self._url_changed())
        self._layout()
        self._quality_changed()
        self.show()
        if url:
            self._probe()

    def _layout(self):
        px = self.t.px
        for w in (self.file_box, self.media_box):
            w.grid_forget()
        box = self.media_box if self.mode.get() == "media" else self.file_box
        box.grid(row=3, column=0, columnspan=2, sticky="we")
        box.columnconfigure(0, minsize=0)
        ttk.Label(self.body, text="Save to").grid(row=4, column=0, sticky="w", pady=6, padx=(0, 14))
        self.folder_row.grid(row=4, column=1, sticky="we", pady=6)
        self.info_lbl.grid(row=5, column=0, columnspan=2, sticky="w", pady=(px(8), 0))

    def _mode_changed(self):
        if self.mode.get() == "media":
            self.folder.set(self.mgr.folder_for("Music" if self._quality_key() == "audio" else "Video"))
        else:
            self.folder.set(self.mgr.folder_for(self.cat.get()))
        self._layout()
        self._probe()

    def _quality_key(self):
        label = self.quality.get()
        for k, lab in defaults.MEDIA_QUALITIES:
            if lab == label:
                return k
        return "best"

    def _quality_changed(self):
        self.audio_row.configure(state="readonly" if self._quality_key() == "audio" else "disabled")
        if self.mode.get() == "media":
            self.folder.set(self.mgr.folder_for("Music" if self._quality_key() == "audio" else "Video"))

    def _url_changed(self):
        url = self.url.get().strip()
        if self.mode.get() == "file" and media.looks_like_media_page(url):
            self.mode.set("media")
            self._mode_changed()
            return
        if self.mode.get() == "file":
            name = engine.name_from_url(url)
            self.name.set(name)
            self.cat.set(self.mgr.category_name_for(name))
            self.folder.set(self.mgr.folder_for(self.cat.get()))
        if hasattr(self, "_probe_after"):
            self.after_cancel(self._probe_after)
        self._probe_after = self.after(700, self._probe)

    def _probe(self):
        url = self.url.get().strip()
        if not url.lower().startswith(("http://", "https://")):
            self.info_lbl.configure(text="")
            return
        self.probe_id += 1
        pid = self.probe_id
        mode = self.mode.get()
        self.info_lbl.configure(text="Getting information…", style="Muted.TLabel")

        def work():
            try:
                if mode == "media":
                    info = media.probe(url, self.mgr.media_cookies_browser, self.headers)
                else:
                    info = engine.probe(url, self.headers)
            except Exception as e:  # noqa: BLE001
                info = {"error": str(e).replace("ERROR: ", "")[:200]}
            try:
                self.after(0, self._got, pid, mode, info)
            except (tk.TclError, RuntimeError):
                pass
        threading.Thread(target=work, daemon=True).start()

    def _got(self, pid, mode, info):
        if pid != self.probe_id or not self.winfo_exists():
            return
        self.info = info
        if "error" in info:
            self.info_lbl.configure(text="Could not get information: " + info["error"], style="Danger.TLabel")
            return
        if mode == "media":
            parts = [info.get("title") or ""]
            if info.get("uploader"):
                parts.append(info["uploader"])
            if info.get("duration"):
                parts.append(engine.fmt_eta(info["duration"]))
            if info.get("is_playlist"):
                parts.append("playlist of %d" % info["count"])
            elif info.get("heights"):
                parts.append("up to %dp" % info["heights"][0])
            self.info_lbl.configure(text="  ·  ".join(p for p in parts if p), style="Muted.TLabel")
        else:
            size = engine.fmt_size(info["size"]) if info.get("size") else "unknown size"
            resume = "can resume" if info.get("resumable") else "no resume"
            self.info_lbl.configure(text="%s  ·  %s  ·  %s" % (info.get("name", ""), size, resume), style="Muted.TLabel")
            cur = self.name.get()
            if info.get("name") and cur == engine.name_from_url(self.url.get().strip()):
                self.name.set(info["name"])
                self.cat.set(self.mgr.category_name_for(info["name"]))
                self.folder.set(self.mgr.folder_for(self.cat.get()))

    def _browse(self):
        p = filedialog.askdirectory(parent=self, initialdir=self.folder.get() or None)
        if p:
            self.folder.set(os.path.normpath(p))

    def _add(self, start):
        url = self.url.get().strip()
        if not url.lower().startswith(("http://", "https://")):
            messagebox.showerror(APP_NAME, "Please enter a link that starts with http:// or https://", parent=self)
            return
        folder = self.folder.get().strip() or self.mgr.base_folder
        queue = None if start else defaults.MAIN_QUEUE
        if self.mode.get() == "media":
            q = self._quality_key()
            title = (self.info or {}).get("title", "") if self.info and "error" not in self.info else ""
            self.mgr.add_media(url, folder, "Music" if q == "audio" else "Video", start=start, queue=queue,
                               headers=self.headers, quality=q, audio_format=self.audio_fmt.get(),
                               playlist=self.playlist.get(), title=title)
        else:
            name = engine.safe_name(self.name.get().strip() or engine.name_from_url(url))
            dl = self.mgr.add(url, name, folder, self.cat.get(), start=start, queue=queue, headers=self.headers)
            if self.info and "error" not in self.info:
                dl.size = self.info.get("size")
        self.destroy()
        self.app.refresh(full=True)

    def ok(self):
        self._add(True)

    def later(self):
        self._add(False)


# ===================================================================== details

class DetailsPanel(ttk.Frame):
    def __init__(self, parent, app):
        super().__init__(parent, style="Card.TFrame", padding=app.theme.px(14))
        self.app = app
        t = app.theme
        px = t.px
        self.dl = None
        self.columnconfigure(0, weight=1)
        head = ttk.Frame(self, style="Surface.TFrame")
        head.grid(row=0, column=0, sticky="we")
        head.columnconfigure(0, weight=1)
        self.title = ttk.Label(head, text="", style="Surface.Bold.TLabel")
        self.title.grid(row=0, column=0, sticky="w")
        btns = ttk.Frame(head, style="Surface.TFrame")
        btns.grid(row=0, column=1, rowspan=2, sticky="e")
        self.b_toggle = widgets.IconButton(btns, t, "pause", self._toggle, "Pause / resume", size=30, bg_key="surface")
        self.b_toggle.pack(side="left")
        widgets.IconButton(btns, t, "folder", lambda: self.dl and paths.reveal(
            self.dl.path if self.dl.status == engine.COMPLETE else self.dl.save_dir), "Show in folder", size=30,
            bg_key="surface").pack(side="left")
        widgets.IconButton(btns, t, "link", self._copy, "Copy link", size=30, bg_key="surface").pack(side="left")
        self.url = ttk.Label(head, text="", style="Surface.Muted.TLabel")
        self.url.grid(row=1, column=0, sticky="w")
        self.bar = widgets.SegmentBar(self, t, height=8)
        self.bar.grid(row=1, column=0, sticky="we", pady=(px(10), px(8)))
        self.stats = ttk.Label(self, text="", style="Surface.TLabel")
        self.stats.grid(row=2, column=0, sticky="w")
        self.where = ttk.Label(self, text="", style="Surface.Muted.TLabel")
        self.where.grid(row=3, column=0, sticky="w", pady=(px(2), 0))
        self.err = ttk.Label(self, text="", style="Danger.TLabel")
        self.placeholder = ttk.Label(self, text="Select a download to see its details.", style="Surface.Muted.TLabel")
        t.listeners.append(lambda th: self.err.configure(background=th.p["surface"]))
        self.err.configure(background=t.p["surface"])
        self.parts = [head, self.bar, self.stats, self.where]
        self.show(None)

    def show(self, dl):
        self.dl = dl
        kids = self.parts
        if dl is None:
            self.err.grid_remove()
            for w in kids:
                w.grid_remove()
            self.placeholder.grid(row=0, column=0, sticky="w")
            return
        self.placeholder.grid_remove()
        for w in kids:
            w.grid()
        self.update_view()

    def update_view(self):
        d = self.dl
        if d is None or not self.winfo_ismapped():
            return
        if d not in self.app.mgr.downloads:
            self.show(None)
            return
        self.title.configure(text=d.file_name)
        self.url.configure(text=d.url if len(d.url) < 110 else d.url[:107] + "…")
        with d.lock:
            segs = [(s.start, s.pos, s.end) for s in d.segments]
        self.bar.set(segs, d.size, d.status == engine.COMPLETE, d.progress())
        parts = []
        got = d.downloaded()
        if d.size:
            parts.append("%s of %s" % (engine.fmt_size(got), engine.fmt_size(d.size)))
        elif got:
            parts.append(engine.fmt_size(got))
        if d.running():
            if d.rate:
                parts.append(engine.fmt_rate(d.rate))
            eta = d.time_left()
            if eta is not None:
                parts.append(engine.fmt_eta(eta) + " left")
            n = d.active_connections()
            if d.kind == "http" and n:
                parts.append("%d connection%s" % (n, "s" if n > 1 else ""))
        parts.append(d.status_text())
        if d.kind == "http" and d.resumable is not None:
            parts.append("resumable" if d.resumable else "cannot resume")
        if d.kind == "media":
            parts.append("yt-dlp · " + dict(defaults.MEDIA_QUALITIES).get(d.quality, d.quality))
        self.stats.configure(text="   ·   ".join(parts))
        self.where.configure(text="Saved to  " + (d.path if d.status == engine.COMPLETE else d.save_dir))
        if d.status == engine.ERROR and d.error:
            self.err.configure(text=d.error)
            self.err.grid(row=4, column=0, sticky="w", pady=(self.app.theme.px(4), 0))
        else:
            self.err.grid_remove()
        self.b_toggle.set_icon("pause" if d.running() else "play")
        self.b_toggle.set_enabled(d.status != engine.COMPLETE)

    def _toggle(self):
        d = self.dl
        if not d:
            return
        if d.running():
            d.stop()
        else:
            d.start(self.app.mgr)

    def _copy(self):
        if self.dl:
            self.app.root.clipboard_clear()
            self.app.root.clipboard_append(self.dl.url)
            self.app.toast("Link copied")


# ===================================================================== delete

class DeleteDialog(Dialog):
    def __init__(self, app, items, title="Remove downloads"):
        super().__init__(app, title)
        self.items = items
        b = self.body
        msg = ('Remove "%s" from the list?' % items[0].file_name) if len(items) == 1 else \
            "Remove %d downloads from the list?" % len(items)
        ttk.Label(b, text=msg, wraplength=self.t.px(420), style="Bold.TLabel").pack(anchor="w")
        self.disk = tk.BooleanVar(value=False)
        if any(d.status == engine.COMPLETE for d in items):
            ttk.Checkbutton(b, text="Also delete the downloaded files from disk", variable=self.disk).pack(
                anchor="w", pady=(self.t.px(12), 0))
        ttk.Label(b, text="Unfinished parts are always cleaned up.", style="Muted.TLabel").pack(anchor="w", pady=(6, 0))
        self.button("Remove", self.ok, accent=True)
        self.button("Cancel", self.destroy)
        self.bind("<Return>", lambda e: self.ok())
        self.show()

    def ok(self):
        for d in self.items:
            self.app.mgr.remove(d, self.disk.get())
        self.destroy()
        self.app.refresh(full=True)
        self.app.details.show(None)


# ===================================================================== settings

class SettingsDialog(Dialog):
    TABS = ["general", "connections", "speed", "categories", "media", "browser"]

    def __init__(self, app, tab="general"):
        super().__init__(app, "Settings")
        mgr = self.mgr = app.mgr
        px = self.t.px
        nb = ttk.Notebook(self.body)
        nb.pack(fill="both", expand=True)
        self.frames = {}
        for key, label in (("general", "General"), ("connections", "Connections"), ("speed", "Speed & queue"),
                           ("categories", "Categories"), ("media", "Video & audio"), ("browser", "Browsers")):
            f = ttk.Frame(nb, padding=px(16))
            f.columnconfigure(1, weight=1)
            nb.add(f, text=label)
            self.frames[key] = f

        # general
        g = self.frames["general"]
        self.theme = tk.StringVar(value=mgr.theme)
        tf = ttk.Frame(g)
        for mode, label in (("system", "Follow system"), ("light", "Light"), ("dark", "Dark")):
            ttk.Radiobutton(tf, text=label, value=mode, variable=self.theme).pack(side="left", padx=(0, px(14)))
        _row(g, 0, "Theme", tf)
        self.base = tk.StringVar(value=mgr.base_folder)
        bf = ttk.Frame(g)
        bf.columnconfigure(0, weight=1)
        ttk.Entry(bf, textvariable=self.base, width=46).grid(row=0, column=0, sticky="we")
        ttk.Button(bf, text="Browse…", command=lambda: self._pick(self.base)).grid(row=0, column=1, padx=(px(6), 0))
        _row(g, 1, "Download folder", bf, "Each category saves into a subfolder (see Categories).")
        self.notify = tk.BooleanVar(value=mgr.notify_on_complete)
        ttk.Checkbutton(g, text="Show a notification when a download finishes", variable=self.notify).grid(
            row=3, column=1, sticky="w", pady=(px(10), 2))
        self.minimized = tk.BooleanVar(value=mgr.start_minimized)
        ttk.Checkbutton(g, text="Start minimized when opened by the browser extension", variable=self.minimized).grid(
            row=4, column=1, sticky="w", pady=2)

        # connections
        c = self.frames["connections"]
        self.maxconn = tk.StringVar(value=str(mgr.max_connections))
        _row(c, 0, "Connections per download", ttk.Combobox(c, textvariable=self.maxconn, width=6, state="readonly",
                                                            values=[str(n) for n in defaults.CONNECTION_CHOICES]),
             "More connections are faster on most servers. Some servers limit them.")
        ttk.Label(c, text="Per-server limits (one per line, e.g. example.com = 2)").grid(
            row=2, column=0, columnspan=2, sticky="w", pady=(px(14), px(4)))
        self.exc = tk.Text(c, height=6, width=50, relief="flat", font=self.t.font, padx=8, pady=6,
                           highlightthickness=1)
        self._style_text(self.exc)
        self.exc.insert("1.0", "\n".join("%s = %s" % (h, n) for h, n in mgr.server_connections.items()))
        self.exc.grid(row=3, column=0, columnspan=2, sticky="we")

        # speed & queue
        s = self.frames["speed"]
        self.lim_on = tk.BooleanVar(value=mgr.limiter_on)
        ttk.Checkbutton(s, text="Limit total download speed", variable=self.lim_on).grid(row=0, column=0,
                                                                                        columnspan=2, sticky="w")
        lf = ttk.Frame(s)
        self.lim_kb = tk.StringVar(value=str(mgr.limit_kb))
        ttk.Spinbox(lf, from_=16, to=10_000_000, increment=128, textvariable=self.lim_kb, width=10).pack(side="left")
        ttk.Label(lf, text="KB/s").pack(side="left", padx=px(6))
        _row(s, 1, "Maximum", lf)
        self.lim_start = tk.BooleanVar(value=mgr.limiter_on_startup)
        ttk.Checkbutton(s, text="Turn the limit on at startup", variable=self.lim_start).grid(row=2, column=1, sticky="w")
        ttk.Separator(s).grid(row=3, column=0, columnspan=2, sticky="we", pady=px(14))
        self.at_once = tk.StringVar(value=str(mgr.queue_at_once))
        _row(s, 4, "Queue: files at once", ttk.Spinbox(s, from_=1, to=defaults.QUEUE_MAX, textvariable=self.at_once, width=6))
        self.retries = tk.StringVar(value=str(mgr.queue_retries))
        _row(s, 5, "Retries per file", ttk.Spinbox(s, from_=0, to=100, textvariable=self.retries, width=6))
        sc = mgr.schedule
        self.start_on = tk.BooleanVar(value=sc["start_on"])
        self.start_at = tk.StringVar(value=sc["start_at"])
        self.stop_on = tk.BooleanVar(value=sc["stop_on"])
        self.stop_at = tk.StringVar(value=sc["stop_at"])
        f1 = ttk.Frame(s)
        ttk.Checkbutton(f1, text="Start the queue every day at", variable=self.start_on).pack(side="left")
        ttk.Entry(f1, textvariable=self.start_at, width=6).pack(side="left", padx=px(6))
        _row(s, 6, "Scheduler", f1)
        f2 = ttk.Frame(s)
        ttk.Checkbutton(f2, text="Stop the queue every day at", variable=self.stop_on).pack(side="left")
        ttk.Entry(f2, textvariable=self.stop_at, width=6).pack(side="left", padx=px(6))
        f2.grid(row=7, column=1, sticky="w")
        ttk.Label(s, text="Times as HH:MM (24-hour clock).", style="Muted.TLabel").grid(row=8, column=1, sticky="w")

        # categories
        k = self.frames["categories"]
        self.cat_edits = {cat.name: [cat.exts, cat.folder] for cat in mgr.categories}
        self.cat_name = tk.StringVar()
        ccb = ttk.Combobox(k, textvariable=self.cat_name, values=list(self.cat_edits), state="readonly", width=18)
        ccb.bind("<<ComboboxSelected>>", lambda e: self._load_cat())
        _row(k, 0, "Category", ccb)
        self.cat_exts = tk.StringVar()
        _row(k, 1, "File types", ttk.Entry(k, textvariable=self.cat_exts, width=46),
             "Extensions separated by spaces; * is a wildcard.")
        self.cat_folder = tk.StringVar()
        cf = ttk.Frame(k)
        cf.columnconfigure(0, weight=1)
        ttk.Entry(cf, textvariable=self.cat_folder).grid(row=0, column=0, sticky="we")
        ttk.Button(cf, text="Browse…", command=lambda: self._pick(self.cat_folder)).grid(row=0, column=1, padx=(px(6), 0))
        _row(k, 3, "Save to", cf)
        self.cat_exts.trace_add("write", lambda *a: self._store_cat())
        self.cat_folder.trace_add("write", lambda *a: self._store_cat())
        if self.cat_edits:
            self.cat_name.set(next(iter(self.cat_edits)))
            self._load_cat()

        # media
        m = self.frames["media"]
        ver = media.ytdlp_version()
        yf = ttk.Frame(m)
        self.yt_lbl = ttk.Label(yf, text=("yt-dlp %s" % ver) if ver else "yt-dlp is not installed",
                                style="TLabel" if ver else "Danger.TLabel")
        self.yt_lbl.pack(side="left")
        self.yt_btn = ttk.Button(yf, text="Update now", command=self._update_ytdlp)
        self.yt_btn.pack(side="left", padx=px(10))
        _row(m, 0, "Engine", yf, "Sites change often: update yt-dlp when a video stops working.")
        ff = media.ffmpeg_path()
        _row(m, 2, "ffmpeg", ttk.Label(m, text=ff or "not found (install it for best quality and mp3)",
                                       style="TLabel" if ff else "Muted.TLabel"))
        self.mq = tk.StringVar(value=dict(defaults.MEDIA_QUALITIES).get(mgr.media_quality, "Best available"))
        _row(m, 3, "Default quality", ttk.Combobox(m, textvariable=self.mq, state="readonly", width=20,
                                                  values=[lab for _k, lab in defaults.MEDIA_QUALITIES]))
        self.maf = tk.StringVar(value=mgr.media_audio_format)
        _row(m, 4, "Audio format", ttk.Combobox(m, textvariable=self.maf, state="readonly", width=8,
                                               values=defaults.AUDIO_FORMATS))
        self.mcb = tk.StringVar(value=mgr.media_cookies_browser or "none")
        _row(m, 5, "Use cookies from", ttk.Combobox(m, textvariable=self.mcb, state="readonly", width=12,
                                                   values=["none"] + defaults.COOKIE_BROWSERS[1:]),
             "Lets yt-dlp see videos you can watch while signed in (age-restricted, private).")
        self.mpl = tk.BooleanVar(value=mgr.media_playlist)
        ttk.Checkbutton(m, text="Download whole playlists by default", variable=self.mpl).grid(row=7, column=1, sticky="w",
                                                                                            pady=(px(8), 0))

        # browser
        br = self.frames["browser"]
        ttk.Label(br, text="The browser extension sends links and videos to %s." % APP_NAME, wraplength=px(480)).grid(
            row=0, column=0, columnspan=2, sticky="w")
        self.br_list = ttk.Frame(br)
        self.br_list.grid(row=1, column=0, columnspan=2, sticky="we", pady=px(12))
        self._fill_browsers()
        bb = ttk.Frame(br)
        bb.grid(row=2, column=0, columnspan=2, sticky="w")
        ttk.Button(bb, text="Connect browsers", command=self._connect_browsers).pack(side="left")
        ttk.Button(bb, text="How to install the extension", command=lambda: open_url(
            HOMEPAGE + "/blob/main/docs/BROWSER_EXTENSIONS.md")).pack(side="left", padx=px(8))

        nb.select(self.TABS.index(tab) if tab in self.TABS else 0)
        self.button("Save", self.ok, accent=True)
        self.button("Cancel", self.destroy)
        self.show()

    def _style_text(self, w):
        p = self.t.p
        w.configure(bg=p["field"], fg=p["text"], insertbackground=p["text"], highlightbackground=p["border"],
                    highlightcolor=p["accent"], selectbackground=p["accent"])

    def _fill_browsers(self):
        for w in self.br_list.winfo_children():
            w.destroy()
        rows = browsers.status()
        if not rows:
            ttk.Label(self.br_list, text="No supported browser found.", style="Muted.TLabel").grid(row=0, column=0)
        for i, (name, _installed, registered) in enumerate(rows):
            ttk.Label(self.br_list, text=name).grid(row=i, column=0, sticky="w", padx=(0, 30), pady=2)
            txt, st = ("connected", "Success.TLabel") if registered else ("not connected", "Muted.TLabel")
            ttk.Label(self.br_list, text=txt, style=st).grid(row=i, column=1, sticky="w")

    def _connect_browsers(self):
        try:
            done = browsers.register()
            self.app.toast("Bridge registered for %d browser(s)" % len(done))
        except OSError as e:
            messagebox.showerror(APP_NAME, str(e), parent=self)
        self._fill_browsers()

    def _update_ytdlp(self):
        self.yt_btn.configure(state="disabled", text="Updating…")

        def work():
            ok, out = media.update_ytdlp()
            self.after(0, done, ok, out)

        def done(ok, out):
            if not self.winfo_exists():
                return
            self.yt_btn.configure(state="normal", text="Update now")
            if ok:
                self.yt_lbl.configure(text="yt-dlp updated (restart to use the new version)", style="Success.TLabel")
            else:
                messagebox.showerror(APP_NAME, "Update failed:\n\n" + out[-800:], parent=self)
        threading.Thread(target=work, daemon=True).start()

    def _pick(self, var):
        p = filedialog.askdirectory(parent=self, initialdir=var.get() or None)
        if p:
            var.set(os.path.normpath(p))

    def _load_cat(self):
        self._loading = True
        exts, folder = self.cat_edits[self.cat_name.get()]
        self.cat_exts.set(exts)
        self.cat_folder.set(folder)
        self._loading = False

    def _store_cat(self):
        if getattr(self, "_loading", False) or not self.cat_name.get():
            return
        self.cat_edits[self.cat_name.get()] = [self.cat_exts.get().strip(), self.cat_folder.get().strip()]

    def ok(self):
        mgr = self.mgr
        for var in (self.start_at, self.stop_at):
            if not re.fullmatch(r"([01]\d|2[0-3]):[0-5]\d", var.get().strip()):
                messagebox.showerror(APP_NAME, "Please enter scheduler times as HH:MM, for example 02:30.", parent=self)
                return
        try:
            limit = max(16, int(float(self.lim_kb.get())))
            at_once = max(1, min(defaults.QUEUE_MAX, int(self.at_once.get())))
            retries = max(0, int(self.retries.get()))
        except ValueError:
            messagebox.showerror(APP_NAME, "Please enter whole numbers.", parent=self)
            return
        exc = {}
        for line in self.exc.get("1.0", "end").splitlines():
            if "=" in line:
                h, n = line.split("=", 1)
                try:
                    if h.strip():
                        exc[h.strip()] = max(1, int(n.strip()))
                except ValueError:
                    pass
        mgr.server_connections = exc
        mgr.max_connections = int(self.maxconn.get())
        old_base = mgr.base_folder
        mgr.base_folder = self.base.get().strip() or old_base
        for cat in mgr.categories:
            exts, folder = self.cat_edits.get(cat.name, [cat.exts, cat.folder])
            if folder == os.path.join(old_base, cat.name) and mgr.base_folder != old_base:
                folder = os.path.join(mgr.base_folder, cat.name)  # follow the new base folder
            cat.exts, cat.folder = exts, folder
        mgr.limit_kb = limit
        mgr.limiter_on_startup = self.lim_start.get()
        mgr.set_limiter(self.lim_on.get())
        self.app.lim_var.set(mgr.limiter_on)
        mgr.queue_at_once = at_once
        mgr.queue_retries = retries
        mgr.schedule.update(start_on=self.start_on.get(), start_at=self.start_at.get().strip(),
                            stop_on=self.stop_on.get(), stop_at=self.stop_at.get().strip())
        mgr.notify_on_complete = self.notify.get()
        mgr.start_minimized = self.minimized.get()
        labels = {lab: k for k, lab in defaults.MEDIA_QUALITIES}
        mgr.media_quality = labels.get(self.mq.get(), "best")
        mgr.media_audio_format = self.maf.get()
        mgr.media_cookies_browser = "" if self.mcb.get() == "none" else self.mcb.get()
        mgr.media_playlist = self.mpl.get()
        if self.theme.get() != mgr.theme:
            self.app._set_theme(self.theme.get())
        mgr.save()
        self.destroy()


# ===================================================================== about / diagnostics

class AboutDialog(Dialog):
    def __init__(self, app):
        super().__init__(app, "About")
        px = self.t.px
        b = self.body
        self.logo = icons.logo_image(self, px(96))
        ttk.Label(b, image=self.logo).pack(pady=(0, px(10)))
        ttk.Label(b, text=APP_NAME, style="Title.TLabel").pack()
        ttk.Label(b, text="Version %s" % __version__, style="Muted.TLabel").pack()
        ttk.Label(b, text="A light, open-source download manager for Linux and macOS.\n"
                          "Multi-connection downloads, video from 1000+ sites (yt-dlp),\n"
                          "browser integration, queue, scheduler and speed limit.",
                  justify="center").pack(pady=px(12))
        ttk.Label(b, text="Free software under the MIT License.", style="Muted.TLabel").pack()
        self.button("Close", self.destroy, accent=True)
        self.button("GitHub", lambda: open_url(HOMEPAGE))
        self.show()


def doctor_text():
    from .. import cli
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        cli._doctor()
    return buf.getvalue()


class DiagnosticsDialog(Dialog):
    def __init__(self, app):
        super().__init__(app, "Diagnostics", resizable=True)
        text = doctor_text()
        ttk.Label(self.body, text="Paste this in bug reports:", style="Muted.TLabel").pack(anchor="w")
        t = tk.Text(self.body, width=92, height=18, relief="flat", padx=10, pady=8, highlightthickness=1,
                    font=("TkFixedFont",))
        p = self.t.p
        t.configure(bg=p["field"], fg=p["text"], highlightbackground=p["border"], insertbackground=p["text"])
        t.insert("1.0", text)
        t.configure(state="disabled")
        t.pack(fill="both", expand=True, pady=(self.t.px(6), 0))

        def copy():
            self.clipboard_clear()
            self.clipboard_append(text)
        self.button("Close", self.destroy)
        self.button("Copy", copy, accent=True)
        self.show(modal=False)
