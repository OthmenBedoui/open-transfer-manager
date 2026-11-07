"""Main window."""

import os
import queue as queue_mod
import sys
import time
import tkinter as tk
from tkinter import ttk

from .. import APP_NAME, __version__, paths, server
from ..core import defaults, engine
from . import dialogs, icons, widgets
from .theme import CATEGORY_COLORS, Theme

MOD = "Command" if paths.IS_MAC else "Control"
MOD_LABEL = "Cmd" if paths.IS_MAC else "Ctrl"

VIEWS = [
    ("all", "all", "All downloads"),
    ("active", "active", "Active"),
    ("unfinished", "queue", "Unfinished"),
    ("finished", "check", "Completed"),
    ("failed", "alert", "Failed"),
]

COLUMNS = [
    # key, title, width, anchor, stretch
    ("size", "Size", 85, "e", False),
    ("progress", "Progress", 190, "w", False),
    ("speed", "Speed", 90, "e", False),
    ("eta", "Time left", 80, "e", False),
    ("status", "Status", 190, "w", True),
    ("added", "Added", 125, "w", False),
]


class App:
    def __init__(self, root, mgr, theme_override=None):
        self.root = root
        self.mgr = mgr
        self.events = queue_mod.Queue()
        self.view = "all"
        self.search = tk.StringVar()
        self.sort_key = None
        self.sort_rev = False
        self.theme = Theme(root, theme_override or mgr.theme)
        self.theme.apply()
        t = self.theme
        root.title(APP_NAME)
        root.geometry("%dx%d" % (t.px(1120), t.px(680)))
        root.minsize(t.px(760), t.px(440))
        self._window_icon()
        self._build_menu()
        self._build_layout()
        self.theme.listeners.append(lambda th: self._on_theme())
        self._on_theme()
        mgr.on_added = lambda d: self._attach(d)
        mgr.on_show = self.show_window
        for d in mgr.downloads:
            self._attach(d)
        self._bind_keys()
        self.select_view("all")
        root.protocol("WM_DELETE_WINDOW", self.quit)
        if paths.IS_MAC:
            root.createcommand("tk::mac::ShowPreferences", lambda: dialogs.SettingsDialog(self))
            root.createcommand("tk::mac::Quit", self.quit)
        root.after(500, self._tick)

    # ------------------------------------------------------------ build
    def _window_icon(self):
        here = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "assets")
        png = os.path.join(here, "otm.png")
        if os.path.exists(png):
            try:
                self._icon = tk.PhotoImage(file=png)
                self.root.iconphoto(True, self._icon)
            except tk.TclError:
                pass

    def _build_menu(self):
        m = tk.Menu(self.root)
        fm = tk.Menu(m, tearoff=0)
        fm.add_command(label="New download...", accelerator=MOD_LABEL + "+N", command=self.add_url)
        fm.add_command(label="Paste links from clipboard", accelerator=MOD_LABEL + "+V", command=self.paste)
        fm.add_separator()
        fm.add_command(label="Open download folder", command=lambda: paths.open_path(self.mgr.base_folder))
        if not paths.IS_MAC:
            fm.add_separator()
            fm.add_command(label="Quit", accelerator=MOD_LABEL + "+Q", command=self.quit)
        m.add_cascade(label="File", menu=fm)

        dm = tk.Menu(m, tearoff=0)
        dm.add_command(label="Resume", command=self.resume)
        dm.add_command(label="Pause", command=self.pause)
        dm.add_command(label="Pause all", command=self.pause_all)
        dm.add_command(label="Download again", command=self.redownload)
        dm.add_separator()
        dm.add_command(label="Start queue", command=self.start_queue)
        dm.add_command(label="Stop queue", command=self.stop_queue)
        dm.add_separator()
        self.lim_var = tk.BooleanVar(value=self.mgr.limiter_on)
        dm.add_checkbutton(label="Speed limit", variable=self.lim_var, command=self.toggle_limiter)
        dm.add_separator()
        dm.add_command(label="Remove...", accelerator="Del", command=self.delete)
        dm.add_command(label="Clear completed...", command=self.delete_completed)
        m.add_cascade(label="Downloads", menu=dm)

        vm = tk.Menu(m, tearoff=0)
        self.theme_var = tk.StringVar(value=self.theme.mode)
        for mode, label in (("system", "Follow system"), ("light", "Light"), ("dark", "Dark")):
            vm.add_radiobutton(label=label, value=mode, variable=self.theme_var, command=self._theme_from_menu)
        vm.add_separator()
        self.details_var = tk.BooleanVar(value=self.mgr.show_details)
        vm.add_checkbutton(label="Details panel", variable=self.details_var, command=self._toggle_details)
        m.add_cascade(label="View", menu=vm)

        tm = tk.Menu(m, tearoff=0)
        tm.add_command(label="Settings...", accelerator=MOD_LABEL + "+,", command=lambda: dialogs.SettingsDialog(self))
        tm.add_command(label="Browser integration...", command=lambda: dialogs.SettingsDialog(self, tab="browser"))
        tm.add_command(label="Update yt-dlp", command=lambda: dialogs.SettingsDialog(self, tab="media"))
        m.add_cascade(label="Tools", menu=tm)

        hm = tk.Menu(m, tearoff=0)
        hm.add_command(label="Website", command=lambda: dialogs.open_url(dialogs.HOMEPAGE))
        hm.add_command(label="Report a problem", command=lambda: dialogs.open_url(dialogs.HOMEPAGE + "/issues/new/choose"))
        hm.add_command(label="Diagnostics", command=lambda: dialogs.DiagnosticsDialog(self))
        hm.add_separator()
        hm.add_command(label="About " + APP_NAME, command=lambda: dialogs.AboutDialog(self))
        m.add_cascade(label="Help", menu=hm)
        self.root.config(menu=m)
        self.menubar = m

    def _build_layout(self):
        t = self.theme
        px = t.px
        # ---- sidebar
        self.sidebar = ttk.Frame(self.root, style="Sidebar.TFrame", width=px(220))
        self.sidebar.pack(side="left", fill="y")
        self.sidebar.pack_propagate(False)
        brand = ttk.Frame(self.sidebar, style="Sidebar.TFrame")
        brand.pack(fill="x", padx=px(16), pady=(px(18), px(14)))
        self.logo_img = icons.logo_image(self.root, px(32))
        ttk.Label(brand, image=self.logo_img, style="Sidebar.TLabel").pack(side="left")
        bt = ttk.Frame(brand, style="Sidebar.TFrame")
        bt.pack(side="left", padx=(px(10), 0))
        ttk.Label(bt, text="Open Transfer", style="Brand.TLabel").pack(anchor="w")
        ttk.Label(bt, text="Manager", style="Sidebar.Muted.TLabel").pack(anchor="w")

        self.nav = {}
        self._nav_header("LIBRARY")
        for key, icon, text in VIEWS:
            self._nav_item(key, icon, text)
        self._nav_header("CATEGORIES")
        for c in self.mgr.categories:
            self._nav_item("cat/" + c.name, icons.CATEGORY_ICONS.get(c.name, "file"), c.name)
        self._nav_item("cat/" + defaults.GENERAL, "file", defaults.GENERAL)

        bottom = ttk.Frame(self.sidebar, style="Sidebar.TFrame")
        bottom.pack(side="bottom", fill="x", padx=px(10), pady=px(10))
        self.theme_btn = widgets.IconButton(bottom, t, "moon", self.toggle_theme, "Light / dark theme", bg_key="sidebar")
        self.theme_btn.pack(side="left")
        widgets.IconButton(bottom, t, "gear", lambda: dialogs.SettingsDialog(self), "Settings",
                           bg_key="sidebar").pack(side="left", padx=px(2))
        widgets.IconButton(bottom, t, "folder", lambda: paths.open_path(self.mgr.base_folder), "Open download folder",
                           bg_key="sidebar").pack(side="left")
        self.sidebar_sep = tk.Frame(self.root, width=1)
        self.sidebar_sep.pack(side="left", fill="y")

        # ---- main area
        main = ttk.Frame(self.root)
        main.pack(side="left", fill="both", expand=True)
        top = ttk.Frame(main, style="Toolbar.TFrame")
        top.pack(fill="x", padx=px(18), pady=(px(14), px(10)))
        self.view_title = ttk.Label(top, text="All downloads", style="Title.TLabel")
        self.view_title.pack(side="left")
        ttk.Button(top, text="+  New download", style="Accent.TButton", command=self.add_url).pack(side="right")
        sf = ttk.Frame(top)
        sf.pack(side="right", padx=(px(12), px(10)))
        self.search_entry = ttk.Entry(sf, textvariable=self.search, width=26, style="Search.TEntry")
        self.search_entry.pack(side="left")
        self._placeholder(self.search_entry, self.search, "Search")
        self.search.trace_add("write", lambda *a: self.refresh(full=True))

        tools = ttk.Frame(main, style="Toolbar.TFrame")
        tools.pack(fill="x", padx=px(14), pady=(0, px(8)))
        self.btn = {}
        for key, icon, tip, fn in (
                ("resume", "play", "Resume (Space)", self.resume),
                ("pause", "pause", "Pause (Space)", self.pause),
                ("delete", "trash", "Remove (Del)", self.delete),
                (None, None, None, None),
                ("pauseall", "pause_all", "Pause all", self.pause_all),
                ("queue", "queue", "Start queue", self.toggle_queue),
                ("clear", "broom", "Clear completed", self.delete_completed),
                (None, None, None, None),
                ("open", "folder", "Show in folder", self.reveal_selected)):
            if key is None:
                ttk.Separator(tools, orient="vertical").pack(side="left", fill="y", padx=px(6), pady=px(6))
                continue
            b = widgets.IconButton(tools, t, icon, fn, tip, danger=(key == "delete"))
            b.pack(side="left", padx=px(1))
            self.btn[key] = b
        self.limit_lbl = ttk.Label(tools, text="", style="Muted.TLabel", cursor="hand2")
        self.limit_lbl.pack(side="right", padx=px(6))
        self.limit_lbl.bind("<Button-1>", lambda e: dialogs.SettingsDialog(self, tab="speed"))

        # list
        card = ttk.Frame(main, style="Card.TFrame")
        card.pack(fill="both", expand=True, padx=px(18), pady=(0, px(10)))
        self.card = card
        cols = [c[0] for c in COLUMNS]
        self.list = ttk.Treeview(card, columns=cols, show="tree headings", selectmode="extended")
        self.list.heading("#0", text="  Name", anchor="w", command=lambda: self._sort("name"))
        self.list.column("#0", width=px(300), minwidth=px(160), stretch=True, anchor="w")
        for key, title, w, anchor, stretch in COLUMNS:
            self.list.heading(key, text=title, anchor=anchor, command=lambda k=key: self._sort(k))
            self.list.column(key, width=px(w), minwidth=px(50), stretch=stretch, anchor=anchor)
        ys = ttk.Scrollbar(card, orient="vertical", command=self.list.yview)
        self.list.configure(yscrollcommand=lambda *a: (ys.set(*a), self._draw_bars()))
        self.list.pack(side="left", fill="both", expand=True, padx=(px(4), 0), pady=px(4))
        ys.pack(side="right", fill="y", pady=px(4))
        self.bars = {}
        self.list.bind("<Configure>", lambda e: self.root.after_idle(self._draw_bars), add="+")
        self.list.bind("<<TreeviewSelect>>", lambda e: (self._update_details(), self._draw_bars()))
        self.list.bind("<Double-1>", lambda e: self._activate())
        self.list.bind("<Return>", lambda e: self._activate())
        self.list.bind("<Button-3>", self._context)
        if paths.IS_MAC:
            self.list.bind("<Button-2>", self._context)
        self.empty = ttk.Label(card, style="Surface.Muted.TLabel", justify="center",
                               text="No downloads here yet.\n\nClick “New download”, paste a link with %s+V,\n"
                                    "or send links from your browser with the extension." % MOD_LABEL)
        self.ctx = tk.Menu(self.root, tearoff=0)
        for label, fn in (("Open", self.open_selected), ("Show in folder", self.reveal_selected), (None, None),
                          ("Resume", self.resume), ("Pause", self.pause), ("Download again", self.redownload),
                          (None, None), ("Add to queue", lambda: self._set_queue(defaults.MAIN_QUEUE)),
                          ("Remove from queue", lambda: self._set_queue(None)), (None, None),
                          ("Copy link", self.copy_link), ("Remove...", self.delete)):
            if label is None:
                self.ctx.add_separator()
            else:
                self.ctx.add_command(label=label, command=fn)

        # details panel
        self.details = dialogs.DetailsPanel(main, self)
        if self.mgr.show_details:
            self.details.pack(fill="x", padx=px(18), pady=(0, px(10)))

        # status bar
        sb = ttk.Frame(main)
        sb.pack(fill="x", padx=px(20), pady=(0, px(10)))
        self.status_left = ttk.Label(sb, text="", style="Muted.TLabel")
        self.status_left.pack(side="left")
        self.status_right = ttk.Label(sb, text="", style="Muted.TLabel")
        self.status_right.pack(side="right")

    def _nav_header(self, text):
        ttk.Label(self.sidebar, text=text, style="SidebarHeader.TLabel").pack(
            fill="x", padx=self.theme.px(22), pady=(self.theme.px(14), self.theme.px(4)))

    def _nav_item(self, key, icon, text):
        item = widgets.NavItem(self.sidebar, self.theme, key, icon, text, self.select_view)
        item.pack(fill="x", padx=self.theme.px(10), pady=1)
        self.nav[key] = item

    def _placeholder(self, entry, var, text):
        # ttk has no placeholder: show a hint while empty and unfocused
        def show(_e=None):
            if not var.get():
                entry.configure(foreground=self.theme.p["muted"])
                entry.insert(0, text)
                entry._ph = True

        def hide(_e=None):
            if getattr(entry, "_ph", False):
                entry.delete(0, "end")
                entry.configure(foreground=self.theme.p["text"])
                entry._ph = False
        entry.bind("<FocusIn>", hide)
        entry.bind("<FocusOut>", show)
        self._ph_show = show
        show()

    def search_text(self):
        if getattr(self.search_entry, "_ph", False):
            return ""
        return self.search.get().strip().lower()

    def _bind_keys(self):
        r = self.root
        r.bind_all("<%s-n>" % MOD, lambda e: self.add_url())
        r.bind_all("<%s-q>" % MOD, lambda e: self.quit())
        r.bind_all("<%s-comma>" % MOD, lambda e: dialogs.SettingsDialog(self))
        r.bind_all("<%s-f>" % MOD, lambda e: self.search_entry.focus_set())
        self.list.bind("<%s-v>" % MOD, lambda e: self.paste())
        r.bind("<%s-v>" % MOD, lambda e: self._paste_if_not_entry(e))
        self.list.bind("<Delete>", lambda e: self.delete())
        self.list.bind("<BackSpace>", lambda e: self.delete() if paths.IS_MAC else None)
        self.list.bind("<space>", lambda e: self.toggle_selected())
        self.list.bind("<%s-a>" % MOD, lambda e: self.list.selection_set(self.list.get_children()))

    def _paste_if_not_entry(self, e):
        if not isinstance(e.widget, (tk.Entry, ttk.Entry, tk.Text)):
            self.paste()

    def _attach(self, d):
        d.on_finish = lambda dl: self.events.put(dl)

    # ------------------------------------------------------------ theme
    def _on_theme(self):
        p = self.theme.p
        self.sidebar_sep.configure(bg=p["border"])
        self.theme_btn.set_icon("sun" if self.theme.name == "dark" else "moon")
        self.list.tag_configure("error", foreground=p["danger"])
        self.list.tag_configure("done", foreground=p["text"])
        self.list.tag_configure("paused", foreground=p["muted"])
        self.list.tag_configure("active", foreground=p["text"])
        s = self.theme.px(12)
        self.badges = {name: icons.badge(self.root, color, s) for name, color in CATEGORY_COLORS.items()}
        if getattr(self.search_entry, "_ph", False):
            self.search_entry.configure(foreground=p["muted"])
        self.refresh(full=True)

    def toggle_theme(self):
        mode = "light" if self.theme.name == "dark" else "dark"
        self._set_theme(mode)

    def _theme_from_menu(self):
        self._set_theme(self.theme_var.get())

    def _set_theme(self, mode):
        self.mgr.theme = mode
        self.theme_var.set(mode)
        self.theme.set_mode(mode)
        self.mgr.save()

    # ------------------------------------------------------------ list
    def select_view(self, key):
        self.view = key
        for k, item in self.nav.items():
            item.set_active(k == key)
        title = dict((k, t) for k, _i, t in VIEWS).get(key) or key.split("/", 1)[-1]
        self.view_title.configure(text=title)
        self.refresh(full=True)

    def _visible(self, d):
        v = self.view
        if v == "active" and not (d.running() or d.status in (engine.CONNECTING, engine.DOWNLOADING)):
            return False
        if v == "unfinished" and d.status == engine.COMPLETE:
            return False
        if v == "finished" and d.status != engine.COMPLETE:
            return False
        if v == "failed" and d.status != engine.ERROR:
            return False
        if v.startswith("cat/") and d.category != v[4:]:
            return False
        q = self.search_text()
        if q and q not in d.file_name.lower() and q not in d.url.lower():
            return False
        return True

    def _draw_bars(self):
        """Real progress bars: small canvases placed over the Progress cells of visible rows."""
        if not hasattr(self, "bars"):
            return
        p = self.theme.p
        px = self.theme.px
        sel = set(self.list.selection())
        seen = set()
        for iid in self.list.get_children():
            try:
                bb = self.list.bbox(iid, "progress")
            except tk.TclError:
                bb = None
            d = self.mgr.find(int(iid))
            if not bb or d is None:
                continue
            x, y, w, h = bb
            cv = self.bars.get(iid)
            if cv is None:
                cv = tk.Canvas(self.list, highlightthickness=0, bd=0, cursor="arrow")
                cv.bind("<Button-1>", lambda e, i=iid: (self.list.selection_set(i), self.list.focus(i)))
                cv.bind("<Double-1>", lambda e: self._activate())
                cv.bind("<Button-3>", lambda e, i=iid: (self.list.selection_set(i),
                                                        self.ctx.tk_popup(e.x_root, e.y_root)))
                self.bars[iid] = cv
            cv.place(x=x + 1, y=y + 1, width=max(1, w - 2), height=max(1, h - 2))
            cv.configure(bg=p["selection"] if iid in sel else p["surface"])
            cv.delete("all")
            frac = d.progress()
            pct_w = px(42)
            bw = max(10, w - pct_w - px(10))
            bh = px(6)
            by = (h - 2 - bh) / 2
            x0 = px(4)
            widgets._round_rect(cv, x0, by, x0 + bw, by + bh, bh / 2, fill=p["track"])
            if d.status == engine.COMPLETE:
                color, frac = p["success"], 1.0
            elif d.status == engine.ERROR:
                color = p["danger"]
            elif d.running():
                color = p["accent"]
            else:
                color = p["muted"]
            if frac:
                widgets._round_rect(cv, x0, by, x0 + max(bh, bw * frac), by + bh, bh / 2, fill=color)
            label = "%d%%" % int((frac or 0) * 100) if frac is not None else ("…" if d.running() else "")
            cv.create_text(w - px(6), (h - 2) / 2, text=label, anchor="e", fill=p["muted"], font=self.theme.font_small)
            seen.add(iid)
        for iid in list(self.bars):
            if iid not in seen:
                self.bars.pop(iid).destroy()

    def _row(self, d):
        running = d.running()
        st = d.status_text()
        if d.status == engine.DOWNLOADING:
            phase = getattr(d, "phase", "")
            st = "Downloading" + (" · " + phase if phase else "")
        elif st.startswith("Paused"):
            st = "Paused"
        if d.status == engine.ERROR and d.error:
            st = "Error: " + d.error
        return {
            "size": engine.fmt_size(d.size) if d.size else "",
            "progress": "",  # drawn by _draw_bars
            "speed": engine.fmt_rate(d.rate) if running and d.rate else "",
            "eta": engine.fmt_eta(d.time_left()) if running else "",
            "status": st,
            "added": engine.fmt_date(d.date_added),
        }

    def _tag(self, d):
        if d.status == engine.ERROR:
            return "error"
        if d.status == engine.COMPLETE:
            return "done"
        if d.running():
            return "active"
        return "paused"

    def _sorted(self, items):
        k = self.sort_key
        if not k:
            return list(reversed(items))  # newest first
        keyf = {
            "name": lambda d: d.file_name.lower(),
            "size": lambda d: d.size or 0,
            "added": lambda d: d.date_added or 0,
            "progress": lambda d: d.progress() or 0,
            "speed": lambda d: d.rate or 0,
            "eta": lambda d: d.time_left() or 0,
            "status": lambda d: d.status,
        }.get(k, lambda d: 0)
        return sorted(items, key=keyf, reverse=self.sort_rev)

    def _sort(self, key):
        if self.sort_key == key:
            self.sort_rev = not self.sort_rev
        else:
            self.sort_key, self.sort_rev = key, False
        self.refresh(full=True)

    def refresh(self, full=False):
        if not hasattr(self, "list"):
            return
        lst = self.list
        items = self._sorted([d for d in self.mgr.downloads if self._visible(d)])
        want = [str(d.id) for d in items]
        have = list(lst.get_children())
        cols = [c[0] for c in COLUMNS]
        if full or want != have:
            sel = set(lst.selection())
            lst.delete(*have)
            for d in items:
                row = self._row(d)
                badge = self.badges.get(d.category, self.badges["General"])
                lst.insert("", "end", iid=str(d.id), text="  " + d.file_name, image=badge,
                           values=[row[k] for k in cols], tags=(self._tag(d),))
            keep = [i for i in want if i in sel]
            if keep:
                lst.selection_set(keep)
        else:
            for d in items:
                row = self._row(d)
                lst.item(str(d.id), text="  " + d.file_name, values=[row[k] for k in cols], tags=(self._tag(d),))
        if items:
            self.empty.place_forget()
        else:
            self.empty.place(relx=0.5, rely=0.45, anchor="center")
        self._draw_bars()
        self._update_counts()
        self._update_buttons()

    def _update_counts(self):
        dl = self.mgr.downloads
        counts = {
            "all": len(dl),
            "active": sum(1 for d in dl if d.running()),
            "unfinished": sum(1 for d in dl if d.status != engine.COMPLETE),
            "finished": sum(1 for d in dl if d.status == engine.COMPLETE),
            "failed": sum(1 for d in dl if d.status == engine.ERROR),
        }
        for key, item in self.nav.items():
            if key.startswith("cat/"):
                item.set_count(sum(1 for d in dl if d.category == key[4:]))
            else:
                item.set_count(counts.get(key, 0))

    def _update_buttons(self):
        sel = self.selected()
        self.btn["resume"].set_enabled(any(not d.running() and d.status != engine.COMPLETE for d in sel))
        self.btn["pause"].set_enabled(any(d.running() for d in sel))
        self.btn["delete"].set_enabled(bool(sel))
        self.btn["open"].set_enabled(bool(sel))
        self.btn["clear"].set_enabled(any(d.status == engine.COMPLETE for d in self.mgr.downloads))
        self.btn["pauseall"].set_enabled(any(d.running() for d in self.mgr.downloads))
        running = self.mgr.queue_running.get(defaults.MAIN_QUEUE)
        self.btn["queue"].set_icon("stop" if running else "queue")

    def selected(self):
        ids = {int(i) for i in self.list.selection()}
        return [d for d in self.mgr.downloads if d.id in ids]

    def _update_details(self):
        sel = self.selected()
        self.details.show(sel[0] if len(sel) == 1 else None)
        self._update_buttons()

    def _toggle_details(self):
        self.mgr.show_details = self.details_var.get()
        if self.mgr.show_details:
            self.details.pack(fill="x", padx=self.theme.px(18), pady=(0, self.theme.px(10)), before=self.status_left.master)
        else:
            self.details.pack_forget()
        self.mgr.save()

    # ------------------------------------------------------------ actions
    def add_url(self, url="", headers=None):
        dialogs.AddDialog(self, url, headers)

    def paste(self):
        try:
            text = self.root.clipboard_get()
        except tk.TclError:
            text = ""
        urls = [u.strip() for u in text.split() if u.strip().lower().startswith(("http://", "https://"))]
        if len(urls) == 1:
            self.add_url(urls[0])
        elif urls:
            for u in urls:
                self.mgr.add_smart(u, start=False, queue=defaults.MAIN_QUEUE)
            self.refresh(full=True)
            self.toast("%d links added to the queue" % len(urls))
        else:
            self.add_url()

    def resume(self):
        for d in self.selected():
            d.start(self.mgr)
        self.refresh()

    def pause(self):
        for d in self.selected():
            d.stop()

    def toggle_selected(self):
        sel = self.selected()
        if any(d.running() for d in sel):
            self.pause()
        else:
            self.resume()

    def pause_all(self):
        self.mgr.stop_all()
        self.refresh()

    def redownload(self):
        for d in self.selected():
            if d.running():
                continue
            d.reset()
            d.start(self.mgr)
        self.refresh(full=True)

    def delete(self):
        sel = self.selected()
        if sel:
            dialogs.DeleteDialog(self, sel)

    def delete_completed(self):
        done = [d for d in self.mgr.downloads if d.status == engine.COMPLETE]
        if done:
            dialogs.DeleteDialog(self, done, "Clear completed downloads")

    def start_queue(self):
        self.mgr.start_queue()
        self.refresh()

    def stop_queue(self):
        self.mgr.stop_queue()
        self.refresh()

    def toggle_queue(self):
        if self.mgr.queue_running.get(defaults.MAIN_QUEUE):
            self.stop_queue()
        else:
            if not any(d.queue for d in self.mgr.downloads):
                self.toast("The queue is empty: right-click a download → Add to queue")
            self.start_queue()

    def _set_queue(self, name):
        for d in self.selected():
            d.queue = name
        self.mgr.save()
        self.refresh(full=True)

    def toggle_limiter(self):
        self.mgr.set_limiter(self.lim_var.get())
        self.mgr.save()
        self._update_status()

    def open_selected(self):
        for d in self.selected()[:1]:
            if d.status == engine.COMPLETE:
                paths.open_path(d.path)

    def reveal_selected(self):
        for d in self.selected()[:1]:
            paths.reveal(d.path if d.status == engine.COMPLETE else d.save_dir)

    def copy_link(self):
        sel = self.selected()
        if sel:
            self.root.clipboard_clear()
            self.root.clipboard_append("\n".join(d.url for d in sel))
            self.toast("Link copied")

    def _activate(self):
        sel = self.selected()
        if not sel:
            return
        d = sel[0]
        if d.status == engine.COMPLETE:
            paths.open_path(d.path)
        elif not d.running():
            d.start(self.mgr)

    def _context(self, event):
        row = self.list.identify_row(event.y)
        if row and row not in self.list.selection():
            self.list.selection_set(row)
        if self.list.selection():
            self.ctx.tk_popup(event.x_root, event.y_root)

    def toast(self, text):
        self.status_left.configure(text=text)
        self._toast_until = time.time() + 4

    def show_window(self):
        self.root.deiconify()
        self.root.lift()
        self.root.attributes("-topmost", True)
        self.root.after(300, lambda: self.root.attributes("-topmost", False))
        self.root.focus_force()

    # ------------------------------------------------------------ loop
    def _update_status(self):
        if time.time() < getattr(self, "_toast_until", 0):
            pass
        else:
            active = sum(1 for d in self.mgr.downloads if d.running())
            rate = self.mgr.total_rate()
            txt = "%d active · ↓ %s" % (active, engine.fmt_rate(rate) or "0 B/s") if active else "Ready"
            if self.mgr.queue_running.get(defaults.MAIN_QUEUE):
                txt += " · queue running"
            self.status_left.configure(text=txt)
        lim = "Speed limit %s/s" % engine.fmt_size(self.mgr.limit_kb * 1024) if self.mgr.limiter_on else ""
        self.limit_lbl.configure(text=lim)
        self.status_right.configure(text="v" + __version__)

    def _tick(self):
        try:
            self.mgr.tick()
            while True:
                try:
                    dl = self.events.get_nowait()
                except queue_mod.Empty:
                    break
                if dl.status == engine.COMPLETE and self.mgr.notify_on_complete:
                    paths.notify("Download complete", dl.file_name)
                elif dl.status == engine.ERROR and dl.error:
                    self.toast("%s: %s" % (dl.file_name, dl.error))
            self.refresh()
            self.details.update_view()
            self._update_status()
        finally:
            self.root.after(500, self._tick)

    def quit(self):
        self.mgr.shutdown()
        self.root.destroy()


def run(urls=(), screenshot=None, minimized=False, theme=None):
    if paths.IS_WIN:
        try:
            import ctypes
            ctypes.windll.shcore.SetProcessDpiAwareness(1)
        except (AttributeError, OSError):
            pass
    root = tk.Tk(className="open-transfer-manager")
    mgr = engine.Manager(persist=screenshot is None)
    if screenshot and not mgr.downloads:
        mgr.downloads = demo_downloads(mgr)
    app = App(root, mgr, theme)
    api = None
    if screenshot is None:
        api = server.ApiServer(mgr)
        try:
            api.start()
        except OSError as e:
            app.toast("Browser bridge unavailable: %s" % e)
            api = None
    for u in urls:
        mgr.add_smart(u)
    if minimized and mgr.start_minimized:
        root.iconify()
    if screenshot:
        _screenshot_loop(root, app, screenshot)
    try:
        root.mainloop()
    finally:
        if api:
            api.stop()
    return 0


def _screenshot_loop(root, app, out):
    from . import shot
    root.attributes("-topmost", True)
    frames = [0]

    def frame():
        frames[0] += 1
        if frames[0] == 10 and app.list.get_children():
            app.list.selection_set(app.list.get_children()[1])
        if frames[0] < 40:
            root.after(50, frame)
            return
        root.update()
        try:
            w, h = shot.capture(root, out)
            print("saved %s (%dx%d)" % (out, w, h))
        except Exception as e:  # noqa: BLE001
            print("screenshot failed: %s" % e, file=sys.stderr)
        root.destroy()
    root.after(200, frame)


def demo_downloads(mgr):
    """Sample rows for screenshots (never saved)."""
    now = time.time()
    rows = [
        ("Ubuntu 26.04 Desktop amd64.iso", "Compressed", 6_012_000_000, 0.62, engine.DOWNLOADING, 9_400_000),
        ("Big Buck Bunny 4K [aqz-KE-bpKQ].mp4", "Video", 734_003_200, 0.31, engine.DOWNLOADING, 4_100_000),
        ("lofi beats - live mix.mp3", "Music", 58_720_256, 1.0, engine.COMPLETE, None),
        ("annual-report-2026.pdf", "Documents", 2_841_002, 1.0, engine.COMPLETE, None),
        ("blender-5.1-linux-x64.tar.xz", "Compressed", 412_517_120, 0.4532, engine.STOPPED, None),
        ("dataset-archive.7z", "Compressed", 1_288_490_188, 0.0, engine.ERROR, None),
        ("vacation-photos.zip", "Compressed", 48_312_445, 1.0, engine.COMPLETE, None),
    ]
    out = []
    for i, (name, cat, size, frac, status, rate) in enumerate(rows):
        d = engine.Download("https://example.com/files/" + name, name, mgr.folder_for(cat), cat)
        d.demo = True
        d.size = size
        d.resumable = True
        d.status = status
        d.date_added = now - 3600 * (i + 1) * 5
        d.last_try = now - 600 * (i + 2)
        d.segments = [engine.Segment(0, size // 2 - 1, int(size // 2 * frac)),
                      engine.Segment(size // 2, size - 1, size // 2 + int(size // 2 * frac))]
        if status == engine.ERROR:
            d.error = "Server answered 403 Forbidden"
        if rate:
            d.rate = rate
            d.running = (lambda: True)
            d.time_left = (lambda d=d, r=rate: (d.size - d.downloaded()) / r)
            d.sample_rate = (lambda: None)
        out.append(d)
    return out
