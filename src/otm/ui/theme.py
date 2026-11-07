"""Light and dark themes for ttk, built on the portable "clam" engine.

All colours live in PALETTES. The accent is the violet at the centre of the logo's
pink -> violet -> cyan gradient.
"""

import os
import subprocess
import tkinter as tk
import tkinter.font as tkfont
from tkinter import ttk

from .. import paths

PALETTES = {
    "light": {
        "bg": "#f5f6f8", "surface": "#ffffff", "surface_alt": "#eef0f4", "sidebar": "#eceff3",
        "hover": "#e3e7ee", "border": "#dde1e8", "text": "#1a1d23", "muted": "#687082",
        "accent": "#6d28d9", "accent_hover": "#5b21b6", "accent_text": "#ffffff", "accent_soft": "#ede9fe",
        "selection": "#ede9fe", "success": "#15803d", "warning": "#b45309", "danger": "#dc2626",
        "field": "#ffffff", "track": "#e4e7ec",
    },
    "dark": {
        "bg": "#121419", "surface": "#181b22", "surface_alt": "#222632", "sidebar": "#0e1014",
        "hover": "#262b37", "border": "#2b303c", "text": "#e7e9ee", "muted": "#8d95a7",
        "accent": "#7c3aed", "accent_hover": "#8b5cf6", "accent_text": "#ffffff", "accent_soft": "#2a2050",
        "selection": "#2c2452", "success": "#22c55e", "warning": "#f59e0b", "danger": "#f87171",
        "field": "#1d2029", "track": "#2a2f3b",
    },
}

CATEGORY_COLORS = {
    "Video": "#ef4444", "Music": "#a855f7", "Images": "#14b8a6", "Documents": "#3b82f6",
    "Compressed": "#f59e0b", "Programs": "#64748b", "General": "#94a3b8",
}


def system_prefers_dark():
    """Best effort: macOS, GNOME/GTK, KDE, Windows. Never raises."""
    try:
        if paths.IS_MAC:
            out = subprocess.run(["defaults", "read", "-g", "AppleInterfaceStyle"],
                                 capture_output=True, text=True, timeout=2)
            return "dark" in out.stdout.lower()
        if paths.IS_WIN:
            import winreg
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER,
                                r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize") as k:
                return winreg.QueryValueEx(k, "AppsUseLightTheme")[0] == 0
        if ":dark" in os.environ.get("GTK_THEME", "").lower():
            return True
        out = subprocess.run(["gsettings", "get", "org.gnome.desktop.interface", "color-scheme"],
                             capture_output=True, text=True, timeout=2)
        if "dark" in out.stdout.lower():
            return True
        out = subprocess.run(["gsettings", "get", "org.gnome.desktop.interface", "gtk-theme"],
                             capture_output=True, text=True, timeout=2)
        if "dark" in out.stdout.lower():
            return True
        kde = os.path.join(paths.home(), ".config", "kdeglobals")
        if os.path.exists(kde):
            with open(kde, encoding="utf-8", errors="ignore") as f:
                return "dark" in f.read().lower()
    except Exception:  # noqa: BLE001
        pass
    return False


class Theme:
    def __init__(self, root, mode="system"):
        self.root = root
        self.style = ttk.Style(root)
        self.mode = mode
        self.name = "light"
        self.p = PALETTES["light"]
        self.listeners = []
        dpi = root.winfo_fpixels("1i")
        self.scale = max(1.0, dpi / 96.0)
        base = tkfont.nametofont("TkDefaultFont").actual()
        self.family = base["family"]
        size = abs(int(base["size"])) or 10
        if paths.IS_MAC:
            size = max(size, 13)
        self.size = size
        self.font = tkfont.Font(root=root, family=self.family, size=size)
        self.font_small = tkfont.Font(root=root, family=self.family, size=size - 1)
        self.font_bold = tkfont.Font(root=root, family=self.family, size=size, weight="bold")
        self.font_title = tkfont.Font(root=root, family=self.family, size=size + 5, weight="bold")
        self.font_brand = tkfont.Font(root=root, family=self.family, size=size + 2, weight="bold")

    def px(self, n):
        return int(round(n * self.scale))

    def set_mode(self, mode):
        self.mode = mode
        self.apply()

    def apply(self):
        name = self.mode
        if name == "system":
            name = "dark" if system_prefers_dark() else "light"
        self.name = name
        self.p = p = PALETTES[name]
        st = self.style
        st.theme_use("clam")
        px = self.px
        root = self.root
        root.configure(bg=p["bg"])

        st.configure(".", background=p["bg"], foreground=p["text"], fieldbackground=p["field"],
                     bordercolor=p["border"], lightcolor=p["bg"], darkcolor=p["bg"], troughcolor=p["track"],
                     focuscolor=p["accent"], selectbackground=p["accent"], selectforeground=p["accent_text"],
                     insertcolor=p["text"], font=self.font, borderwidth=1)
        st.map(".", foreground=[("disabled", p["muted"])])

        for name_, bg in (("TFrame", p["bg"]), ("Surface.TFrame", p["surface"]), ("Sidebar.TFrame", p["sidebar"]),
                          ("Card.TFrame", p["surface"]), ("Toolbar.TFrame", p["bg"])):
            st.configure(name_, background=bg)
        st.configure("Card.TFrame", bordercolor=p["border"], relief="solid", borderwidth=1)

        st.configure("TLabel", background=p["bg"], foreground=p["text"])
        st.configure("Muted.TLabel", foreground=p["muted"], font=self.font_small)
        st.configure("Title.TLabel", font=self.font_title)
        st.configure("Bold.TLabel", font=self.font_bold)
        st.configure("Danger.TLabel", foreground=p["danger"])
        st.configure("Success.TLabel", foreground=p["success"])
        for prefix, bg in (("Surface", p["surface"]), ("Sidebar", p["sidebar"])):
            st.configure(prefix + ".TLabel", background=bg, foreground=p["text"])
            st.configure(prefix + ".Muted.TLabel", background=bg, foreground=p["muted"], font=self.font_small)
            st.configure(prefix + ".Bold.TLabel", background=bg, foreground=p["text"], font=self.font_bold)
        st.configure("Brand.TLabel", background=p["sidebar"], foreground=p["text"], font=self.font_brand)
        st.configure("SidebarHeader.TLabel", background=p["sidebar"], foreground=p["muted"], font=self.font_small)

        # navigation items in the sidebar
        for state, bg, fg in (("Nav", p["sidebar"], p["text"]), ("NavHover", p["hover"], p["text"]),
                              ("NavActive", p["accent_soft"], p["accent"] if self.name == "light" else p["text"])):
            st.configure(state + ".TFrame", background=bg)
            st.configure(state + ".TLabel", background=bg, foreground=fg, padding=(px(10), px(6)))
            st.configure(state + ".Count.TLabel", background=bg, foreground=p["muted"], font=self.font_small,
                         padding=(px(6), px(6)))

        btn_pad = (px(14), px(7))
        st.configure("TButton", background=p["surface_alt"], foreground=p["text"], bordercolor=p["border"],
                     lightcolor=p["surface_alt"], darkcolor=p["surface_alt"], padding=btn_pad, relief="flat",
                     focusthickness=0, focuscolor=p["surface_alt"])
        st.map("TButton", background=[("pressed", p["border"]), ("active", p["hover"])],
               lightcolor=[("active", p["hover"])], darkcolor=[("active", p["hover"])])
        st.configure("Accent.TButton", background=p["accent"], foreground=p["accent_text"], bordercolor=p["accent"],
                     lightcolor=p["accent"], darkcolor=p["accent"], focuscolor=p["accent"], font=self.font_bold)
        st.map("Accent.TButton", background=[("pressed", p["accent_hover"]), ("active", p["accent_hover"])],
               lightcolor=[("active", p["accent_hover"])], darkcolor=[("active", p["accent_hover"])],
               bordercolor=[("active", p["accent_hover"])])
        st.configure("Danger.TButton", background=p["danger"], foreground="#ffffff", bordercolor=p["danger"],
                     lightcolor=p["danger"], darkcolor=p["danger"], focuscolor=p["danger"])
        st.map("Danger.TButton", background=[("active", p["danger"])])

        for w in ("TEntry", "TCombobox", "TSpinbox"):
            st.configure(w, fieldbackground=p["field"], foreground=p["text"], bordercolor=p["border"],
                         lightcolor=p["field"], darkcolor=p["field"], padding=(px(8), px(5)), arrowcolor=p["muted"],
                         background=p["surface_alt"], insertcolor=p["text"])
            st.map(w, bordercolor=[("focus", p["accent"])], lightcolor=[("focus", p["accent"])],
                   fieldbackground=[("readonly", p["field"]), ("disabled", p["surface_alt"])],
                   foreground=[("readonly", p["text"])], selectbackground=[("readonly", p["field"])],
                   selectforeground=[("readonly", p["text"])])
        st.configure("Search.TEntry", padding=(px(10), px(6)))

        for w in ("TCheckbutton", "TRadiobutton"):
            st.configure(w, background=p["bg"], foreground=p["text"], indicatorbackground=p["field"],
                         indicatorforeground=p["accent_text"], indicatormargin=(0, 0, px(8), 0), focuscolor=p["bg"],
                         upperbordercolor=p["border"], lowerbordercolor=p["border"])
            st.map(w, indicatorbackground=[("selected", p["accent"]), ("active", p["hover"])],
                   background=[("active", p["bg"])])
        for w in ("Surface.TCheckbutton", "Surface.TRadiobutton"):
            st.configure(w, background=p["surface"])
            st.map(w, background=[("active", p["surface"])])

        st.configure("Treeview", background=p["surface"], fieldbackground=p["surface"], foreground=p["text"],
                     bordercolor=p["surface"], lightcolor=p["surface"], darkcolor=p["surface"],
                     rowheight=px(30), borderwidth=0, font=self.font)
        st.map("Treeview", background=[("selected", p["selection"])], foreground=[("selected", p["text"])])
        st.configure("Treeview.Heading", background=p["surface"], foreground=p["muted"], relief="flat",
                     bordercolor=p["border"], lightcolor=p["surface"], darkcolor=p["surface"], font=self.font_small,
                     padding=(px(6), px(6)))
        st.map("Treeview.Heading", background=[("active", p["surface_alt"])])
        st.layout("Treeview", [("Treeview.treearea", {"sticky": "nswe"})])

        for orient in ("Vertical", "Horizontal"):
            st.configure(orient + ".TScrollbar", troughcolor=p["surface"], background=p["track"],
                         bordercolor=p["surface"], lightcolor=p["track"], darkcolor=p["track"],
                         arrowcolor=p["muted"], gripcount=0, arrowsize=px(12))
            st.map(orient + ".TScrollbar", background=[("active", p["border"])])

        st.configure("Horizontal.TProgressbar", troughcolor=p["track"], background=p["accent"],
                     bordercolor=p["track"], lightcolor=p["accent"], darkcolor=p["accent"], thickness=px(6))

        st.configure("TNotebook", background=p["bg"], bordercolor=p["border"], tabmargins=(0, 0, 0, 0))
        st.configure("TNotebook.Tab", background=p["bg"], foreground=p["muted"], bordercolor=p["border"],
                     lightcolor=p["bg"], darkcolor=p["bg"], padding=(px(14), px(7)))
        st.map("TNotebook.Tab", background=[("selected", p["surface"])], foreground=[("selected", p["text"])],
               lightcolor=[("selected", p["surface"])])

        st.configure("TSeparator", background=p["border"])
        st.configure("TPanedwindow", background=p["border"])
        st.configure("Sash", sashthickness=px(1), gripcount=0, background=p["border"])
        st.configure("TLabelframe", background=p["bg"], bordercolor=p["border"])
        st.configure("TLabelframe.Label", background=p["bg"], foreground=p["muted"])

        # classic Tk widgets created after this point
        root.option_add("*TCombobox*Listbox.background", p["field"])
        root.option_add("*TCombobox*Listbox.foreground", p["text"])
        root.option_add("*TCombobox*Listbox.selectBackground", p["accent"])
        root.option_add("*TCombobox*Listbox.selectForeground", p["accent_text"])
        root.option_add("*TCombobox*Listbox.font", self.font)
        root.option_add("*Menu.background", p["surface"])
        root.option_add("*Menu.foreground", p["text"])
        root.option_add("*Menu.activeBackground", p["selection"])
        root.option_add("*Menu.activeForeground", p["text"])
        root.option_add("*Menu.relief", "flat")
        root.option_add("*Toplevel.background", p["bg"])
        self._recolor_menus(root)
        for fn in list(self.listeners):
            try:
                fn(self)
            except tk.TclError:
                self.listeners.remove(fn)

    def _recolor_menus(self, widget):
        if paths.IS_MAC:
            return  # native menu bar
        p = self.p
        for w in widget.winfo_children():
            if isinstance(w, tk.Menu):
                try:
                    w.configure(background=p["surface"], foreground=p["text"], activebackground=p["selection"],
                                activeforeground=p["text"], relief="flat", borderwidth=1)
                except tk.TclError:
                    pass
            if isinstance(w, tk.Toplevel):
                w.configure(bg=p["bg"])
            self._recolor_menus(w)
