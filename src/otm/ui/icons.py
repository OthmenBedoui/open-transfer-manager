"""Line icons drawn on a Canvas (no image files, crisp at any size, follow the theme)."""

import math
import os
import tkinter as tk


def draw(cv, name, x, y, s, color, width=None):
    """Draw icon `name` in the square (x, y, s) with stroke `color`."""
    w = width or max(1.6, s / 11.0)
    L = dict(fill=color, width=w, capstyle="round", joinstyle="round")

    def P(fx, fy):
        return x + fx * s, y + fy * s

    def line(*pts, **kw):
        flat = []
        for fx, fy in pts:
            flat += P(fx, fy)
        opts = dict(L)
        opts.update(kw)
        cv.create_line(*flat, **opts)

    def oval(x0, y0, x1, y1, fill=""):
        cv.create_oval(*P(x0, y0), *P(x1, y1), outline=color, width=w, fill=fill)

    if name == "plus":
        line((0.5, 0.2), (0.5, 0.8))
        line((0.2, 0.5), (0.8, 0.5))
    elif name == "play":
        cv.create_polygon(*P(0.32, 0.2), *P(0.32, 0.8), *P(0.8, 0.5), fill=color, outline=color, width=w,
                          joinstyle="round")
    elif name == "pause":
        line((0.36, 0.22), (0.36, 0.78), width=w * 1.6)
        line((0.64, 0.22), (0.64, 0.78), width=w * 1.6)
    elif name == "stop":
        cv.create_rectangle(*P(0.27, 0.27), *P(0.73, 0.73), fill=color, outline=color, width=w)
    elif name == "pause_all":
        oval(0.12, 0.12, 0.88, 0.88)
        line((0.41, 0.35), (0.41, 0.65), width=w * 1.3)
        line((0.59, 0.35), (0.59, 0.65), width=w * 1.3)
    elif name == "trash":
        line((0.2, 0.28), (0.8, 0.28))
        line((0.4, 0.28), (0.42, 0.17), (0.58, 0.17), (0.6, 0.28))
        line((0.27, 0.28), (0.31, 0.83), (0.69, 0.83), (0.73, 0.28))
        line((0.44, 0.42), (0.45, 0.7))
        line((0.56, 0.42), (0.55, 0.7))
    elif name == "broom":
        line((0.78, 0.15), (0.5, 0.5))
        cv.create_polygon(*P(0.42, 0.42), *P(0.6, 0.6), *P(0.42, 0.86), *P(0.14, 0.86), *P(0.2, 0.62),
                          outline=color, fill="", width=w, joinstyle="round")
        line((0.3, 0.86), (0.36, 0.68))
    elif name == "gear":
        cx, cy = P(0.5, 0.5)
        r1, r2 = s * 0.36, s * 0.27
        pts = []
        for i in range(32):
            a = 2 * math.pi * i / 32
            r = r1 if (i // 2) % 2 == 0 else r2
            pts += [cx + r * math.cos(a), cy + r * math.sin(a)]
        cv.create_polygon(pts, outline=color, fill="", width=w, joinstyle="round")
        oval(0.4, 0.4, 0.6, 0.6)
    elif name == "clock":
        oval(0.14, 0.14, 0.86, 0.86)
        line((0.5, 0.3), (0.5, 0.5), (0.64, 0.6))
    elif name == "sun":
        oval(0.34, 0.34, 0.66, 0.66)
        for i in range(8):
            a = math.pi * i / 4
            line((0.5 + 0.26 * math.cos(a), 0.5 + 0.26 * math.sin(a)),
                 (0.5 + 0.38 * math.cos(a), 0.5 + 0.38 * math.sin(a)))
    elif name == "moon":
        # filled crescent: a disc minus an offset disc painted with the canvas background
        bg = getattr(cv, "icon_bg", None) or cv.cget("bg")
        cv.create_oval(*P(0.16, 0.16), *P(0.84, 0.84), fill=color, outline="")
        cv.create_oval(*P(0.36, 0.04), *P(0.98, 0.66), fill=bg, outline="")
    elif name == "folder":
        line((0.14, 0.3), (0.14, 0.78), (0.86, 0.78), (0.86, 0.38), (0.5, 0.38), (0.42, 0.28), (0.14, 0.28),
             (0.14, 0.3))
    elif name == "search":
        oval(0.18, 0.18, 0.62, 0.62)
        line((0.56, 0.56), (0.82, 0.82))
    elif name == "link":
        line((0.42, 0.58), (0.58, 0.42))
        line((0.36, 0.48), (0.24, 0.6), (0.24, 0.72), (0.28, 0.76), (0.4, 0.76), (0.52, 0.64))
        line((0.48, 0.36), (0.6, 0.24), (0.72, 0.24), (0.76, 0.28), (0.76, 0.4), (0.64, 0.52))
    elif name == "download":
        line((0.5, 0.16), (0.5, 0.64))
        line((0.3, 0.46), (0.5, 0.66), (0.7, 0.46))
        line((0.2, 0.8), (0.8, 0.8))
    elif name == "video":
        cv.create_rectangle(*P(0.14, 0.28), *P(0.66, 0.72), outline=color, width=w)
        line((0.66, 0.44), (0.86, 0.32), (0.86, 0.68), (0.66, 0.56))
    elif name == "music":
        line((0.38, 0.72), (0.38, 0.22), (0.78, 0.16), (0.78, 0.64))
        oval(0.2, 0.64, 0.38, 0.8)
        oval(0.6, 0.56, 0.78, 0.72)
    elif name == "image":
        cv.create_rectangle(*P(0.14, 0.2), *P(0.86, 0.8), outline=color, width=w)
        line((0.14, 0.7), (0.4, 0.46), (0.58, 0.62), (0.68, 0.54), (0.86, 0.7))
        oval(0.6, 0.3, 0.7, 0.4)
    elif name == "document":
        line((0.24, 0.14), (0.24, 0.86), (0.76, 0.86), (0.76, 0.34), (0.56, 0.14), (0.24, 0.14))
        line((0.56, 0.14), (0.56, 0.34), (0.76, 0.34))
        line((0.36, 0.54), (0.64, 0.54))
        line((0.36, 0.68), (0.64, 0.68))
    elif name == "archive":
        cv.create_rectangle(*P(0.16, 0.2), *P(0.84, 0.36), outline=color, width=w)
        line((0.22, 0.36), (0.22, 0.82), (0.78, 0.82), (0.78, 0.36))
        line((0.42, 0.5), (0.58, 0.5))
    elif name == "app":
        cv.create_rectangle(*P(0.16, 0.16), *P(0.44, 0.44), outline=color, width=w)
        cv.create_rectangle(*P(0.56, 0.16), *P(0.84, 0.44), outline=color, width=w)
        cv.create_rectangle(*P(0.16, 0.56), *P(0.44, 0.84), outline=color, width=w)
        cv.create_rectangle(*P(0.56, 0.56), *P(0.84, 0.84), outline=color, width=w)
    elif name == "file":
        line((0.26, 0.14), (0.26, 0.86), (0.74, 0.86), (0.74, 0.34), (0.54, 0.14), (0.26, 0.14))
    elif name == "all":
        line((0.2, 0.3), (0.8, 0.3))
        line((0.2, 0.5), (0.8, 0.5))
        line((0.2, 0.7), (0.8, 0.7))
    elif name == "active":
        line((0.5, 0.16), (0.5, 0.64))
        line((0.3, 0.46), (0.5, 0.66), (0.7, 0.46))
        line((0.2, 0.84), (0.8, 0.84))
    elif name == "check":
        line((0.2, 0.52), (0.42, 0.72), (0.8, 0.3))
    elif name == "queue":
        line((0.18, 0.28), (0.62, 0.28))
        line((0.18, 0.5), (0.62, 0.5))
        line((0.18, 0.72), (0.5, 0.72))
        cv.create_polygon(*P(0.7, 0.58), *P(0.7, 0.86), *P(0.88, 0.72), fill=color, outline=color)
    elif name == "alert":
        line((0.5, 0.14), (0.88, 0.82), (0.12, 0.82), (0.5, 0.14))
        line((0.5, 0.4), (0.5, 0.6))
        line((0.5, 0.71), (0.5, 0.72))
    elif name == "close":
        line((0.26, 0.26), (0.74, 0.74))
        line((0.74, 0.26), (0.26, 0.74))


ASSETS = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "assets")
LOGO_SIZES = (32, 48, 64, 96, 128)


def logo_image(master, size):
    """The app logo as a PhotoImage, from the pre-rendered PNG closest to `size` pixels."""
    best = min(LOGO_SIZES, key=lambda s: (s < size, abs(s - size)))
    path = os.path.join(ASSETS, "otm-%d.png" % best)
    if not os.path.exists(path):
        path = os.path.join(ASSETS, "otm.png")
    return tk.PhotoImage(master=master, file=path)


CATEGORY_ICONS = {"Video": "video", "Music": "music", "Images": "image", "Documents": "document",
                  "Compressed": "archive", "Programs": "app", "General": "file"}


def badge(master, color, size, letter_rows=None):
    """A small rounded square in `color` (PhotoImage) used as the file-type mark in the list."""
    img = tk.PhotoImage(master=master, width=size, height=size)
    r = max(2, size // 4)
    rows = []
    for yy in range(size):
        row = []
        for xx in range(size):
            dx = max(r - xx, 0, xx - (size - 1 - r))
            dy = max(r - yy, 0, yy - (size - 1 - r))
            inside = dx * dx + dy * dy <= r * r
            row.append(color if inside else "")
        rows.append(row)
    for yy, row in enumerate(rows):
        for xx, c in enumerate(row):
            if c:
                img.put(c, (xx, yy))
    return img


def dot(master, color, size):
    """A filled circle PhotoImage (status dots)."""
    img = tk.PhotoImage(master=master, width=size, height=size)
    c = (size - 1) / 2.0
    for yy in range(size):
        for xx in range(size):
            if (xx - c) ** 2 + (yy - c) ** 2 <= (size / 2.0) ** 2:
                img.put(color, (xx, yy))
    return img
