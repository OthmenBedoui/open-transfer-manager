"""Small custom widgets that follow the theme."""

import tkinter as tk
from tkinter import ttk

from . import icons


class Tooltip:
    def __init__(self, widget, text, theme):
        self.widget, self.text, self.theme = widget, text, theme
        self.tip = None
        self.after = None
        widget.bind("<Enter>", self._schedule, add="+")
        widget.bind("<Leave>", self._hide, add="+")
        widget.bind("<ButtonPress>", self._hide, add="+")

    def _schedule(self, _e=None):
        self._cancel()
        self.after = self.widget.after(600, self._show)

    def _cancel(self):
        if self.after:
            self.widget.after_cancel(self.after)
            self.after = None

    def _show(self):
        if self.tip or not self.text:
            return
        p = self.theme.p
        x = self.widget.winfo_rootx() + self.widget.winfo_width() // 2
        y = self.widget.winfo_rooty() + self.widget.winfo_height() + 4
        self.tip = tw = tk.Toplevel(self.widget)
        tw.wm_overrideredirect(True)
        tk.Label(tw, text=self.text, bg=p["text"], fg=p["bg"], font=self.theme.font_small,
                 padx=8, pady=4).pack()
        tw.update_idletasks()
        tw.geometry("+%d+%d" % (x - tw.winfo_width() // 2, y))

    def _hide(self, _e=None):
        self._cancel()
        if self.tip:
            self.tip.destroy()
            self.tip = None


class IconButton(tk.Canvas):
    """Flat icon button with a rounded hover background."""

    def __init__(self, parent, theme, icon, command, tooltip="", size=34, bg_key="bg", danger=False):
        s = theme.px(size)
        super().__init__(parent, width=s, height=s, highlightthickness=0, bd=0, cursor="hand2")
        self.theme, self.icon, self.command, self.size = theme, icon, command, s
        self.bg_key = bg_key
        self.danger = danger
        self.hover = False
        self.enabled = True
        self.bind("<Enter>", lambda e: self._set_hover(True))
        self.bind("<Leave>", lambda e: self._set_hover(False))
        self.bind("<ButtonRelease-1>", self._click)
        theme.listeners.append(lambda t: self.redraw())
        if tooltip:
            Tooltip(self, tooltip, theme)
        self.redraw()

    def set_icon(self, icon):
        if icon != self.icon:
            self.icon = icon
            self.redraw()

    def set_enabled(self, on):
        if on != self.enabled:
            self.enabled = on
            self.configure(cursor="hand2" if on else "")
            self.redraw()

    def _set_hover(self, on):
        self.hover = on
        self.redraw()

    def _click(self, e):
        if self.enabled and 0 <= e.x <= self.size and 0 <= e.y <= self.size and self.command:
            self.command()

    def redraw(self):
        p = self.theme.p
        self.delete("all")
        self.configure(bg=p[self.bg_key])
        s = self.size
        self.icon_bg = p[self.bg_key]
        if self.hover and self.enabled:
            _round_rect(self, 1, 1, s - 1, s - 1, s * 0.28, fill=p["hover"])
            self.icon_bg = p["hover"]
        if not self.enabled:
            color = p["border"]
        elif self.danger and self.hover:
            color = p["danger"]
        else:
            color = p["text"] if self.hover else p["muted"]
        pad = s * 0.24
        icons.draw(self, self.icon, pad, pad, s - 2 * pad, color)


class NavItem(ttk.Frame):
    """Sidebar entry: icon, label and a count."""

    def __init__(self, parent, theme, key, icon, text, on_select):
        super().__init__(parent, style="Nav.TFrame", cursor="hand2")
        self.theme, self.key, self.icon_name = theme, key, icon
        self.active = False
        self.hover = False
        s = theme.px(18)
        self.cv = tk.Canvas(self, width=s, height=s, highlightthickness=0, bd=0)
        self.cv.pack(side="left", padx=(theme.px(12), 0))
        self.label = ttk.Label(self, text=text, style="Nav.TLabel")
        self.label.pack(side="left", fill="x", expand=True)
        self.count = ttk.Label(self, text="", style="Nav.Count.TLabel")
        self.count.pack(side="right", padx=(0, theme.px(6)))
        for w in (self, self.cv, self.label, self.count):
            w.bind("<Button-1>", lambda e: on_select(self.key))
            w.bind("<Enter>", lambda e: self._hover(True))
            w.bind("<Leave>", lambda e: self._hover(False))
        theme.listeners.append(lambda t: self._paint())
        self._paint()

    def _hover(self, on):
        self.hover = on
        self._paint()

    def set_active(self, on):
        self.active = on
        self._paint()

    def set_count(self, n):
        self.count.configure(text=str(n) if n else "")

    def _paint(self):
        state = "NavActive" if self.active else ("NavHover" if self.hover else "Nav")
        self.configure(style=state + ".TFrame")
        self.label.configure(style=state + ".TLabel")
        self.count.configure(style=state + ".Count.TLabel")
        p = self.theme.p
        bg = {"NavActive": p["accent_soft"], "NavHover": p["hover"], "Nav": p["sidebar"]}[state]
        self.cv.configure(bg=bg)
        self.cv.delete("all")
        s = int(self.cv["width"])
        color = p["accent"] if self.active else p["muted"]
        icons.draw(self.cv, self.icon_name, 0, 0, s, color)


class SegmentBar(tk.Canvas):
    """Shows which byte ranges are already downloaded (one mark per connection)."""

    def __init__(self, parent, theme, height=10):
        super().__init__(parent, height=theme.px(height), highlightthickness=0, bd=0)
        self.theme = theme
        self.segs = []
        self.size = None
        self.complete = False
        self.bind("<Configure>", lambda e: self.redraw())
        theme.listeners.append(lambda t: self.redraw())

    def set(self, segs, size, complete=False, fraction=None):
        self.segs, self.size, self.complete, self.fraction = segs, size, complete, fraction
        self.redraw()

    def redraw(self):
        p = self.theme.p
        self.delete("all")
        self.configure(bg=p["surface"])
        w = max(10, self.winfo_width())
        h = int(self["height"])
        _round_rect(self, 0, 0, w, h, h / 2, fill=p["track"])
        if self.complete:
            _round_rect(self, 0, 0, w, h, h / 2, fill=p["success"])
            return
        frac = getattr(self, "fraction", None)
        if self.size and self.segs:
            for a, pos, _e in self.segs:
                x0, x1 = a / self.size * w, pos / self.size * w
                if x1 - x0 >= 1:
                    self.create_rectangle(x0, 0, x1, h, fill=p["accent"], outline="")
        elif frac:
            _round_rect(self, 0, 0, max(h, w * frac), h, h / 2, fill=p["accent"])


def _round_rect(cv, x0, y0, x1, y1, r, **kw):
    r = min(r, (x1 - x0) / 2, (y1 - y0) / 2)
    pts = [x0 + r, y0, x1 - r, y0, x1, y0, x1, y0 + r, x1, y1 - r, x1, y1, x1 - r, y1, x0 + r, y1,
           x0, y1, x0, y1 - r, x0, y0 + r, x0, y0]
    return cv.create_polygon(pts, smooth=True, outline="", **kw)
