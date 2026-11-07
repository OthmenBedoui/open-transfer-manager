"""Save a picture of the main window (for README screenshots and visual checks)."""

import os
import shutil
import struct
import subprocess
import sys
import zlib


def capture(root, path):
    root.update()
    os.makedirs(os.path.dirname(os.path.abspath(path)) or ".", exist_ok=True)
    if os.name == "nt":
        return _capture_windows(root.winfo_id(), path)
    x, y = root.winfo_rootx(), root.winfo_rooty()
    w, h = root.winfo_width(), root.winfo_height()
    if sys.platform == "darwin":
        subprocess.run(["screencapture", "-x", "-R%d,%d,%d,%d" % (x, y, w, h), path], check=True)
        return w, h
    if shutil.which("import"):  # ImageMagick
        subprocess.run(["import", "-window", hex(root.winfo_id()), path], check=True)
        return w, h
    raise RuntimeError("install ImageMagick (`import`) to take screenshots on Linux")


def _png(width, height, rgb):
    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    stride = width * 3
    raw = bytearray()
    for y in range(height):
        raw.append(0)
        raw += rgb[y * stride:(y + 1) * stride]
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(bytes(raw), 6)) + chunk(b"IEND", b""))


def _capture_windows(tk_id, path):
    import ctypes

    class RECT(ctypes.Structure):
        _fields_ = [("l", ctypes.c_long), ("t", ctypes.c_long), ("r", ctypes.c_long), ("b", ctypes.c_long)]

    class BMI(ctypes.Structure):
        _fields_ = [("biSize", ctypes.c_uint32), ("biWidth", ctypes.c_int32), ("biHeight", ctypes.c_int32),
                    ("biPlanes", ctypes.c_uint16), ("biBitCount", ctypes.c_uint16), ("biCompression", ctypes.c_uint32),
                    ("biSizeImage", ctypes.c_uint32), ("x", ctypes.c_int32), ("y", ctypes.c_int32),
                    ("u", ctypes.c_uint32), ("i", ctypes.c_uint32)]
    u, g = ctypes.windll.user32, ctypes.windll.gdi32
    vp = ctypes.c_void_p
    u.GetAncestor.restype = vp
    u.GetAncestor.argtypes = [vp, ctypes.c_uint]
    u.GetWindowRect.argtypes = [vp, ctypes.POINTER(RECT)]
    u.GetWindowDC.restype = vp
    u.GetWindowDC.argtypes = [vp]
    u.ReleaseDC.argtypes = [vp, vp]
    u.PrintWindow.argtypes = [vp, vp, ctypes.c_uint]
    g.CreateCompatibleDC.restype = vp
    g.CreateCompatibleDC.argtypes = [vp]
    g.CreateCompatibleBitmap.restype = vp
    g.CreateCompatibleBitmap.argtypes = [vp, ctypes.c_int, ctypes.c_int]
    g.SelectObject.restype = vp
    g.SelectObject.argtypes = [vp, vp]
    g.GetDIBits.argtypes = [vp, vp, ctypes.c_uint, ctypes.c_uint, vp, ctypes.c_void_p, ctypes.c_uint]
    g.DeleteObject.argtypes = [vp]
    g.DeleteDC.argtypes = [vp]
    hwnd = u.GetAncestor(tk_id, 2) or tk_id
    r = RECT()
    u.GetWindowRect(hwnd, ctypes.byref(r))
    w, h = r.r - r.l, r.b - r.t
    hdc = u.GetWindowDC(hwnd)
    mdc = g.CreateCompatibleDC(hdc)
    bmp = g.CreateCompatibleBitmap(hdc, w, h)
    old = g.SelectObject(mdc, bmp)
    u.PrintWindow(hwnd, mdc, 2)
    bi = BMI()
    bi.biSize = ctypes.sizeof(BMI)
    bi.biWidth, bi.biHeight, bi.biPlanes, bi.biBitCount = w, -h, 1, 32
    buf = ctypes.create_string_buffer(w * h * 4)
    g.GetDIBits(mdc, bmp, 0, h, buf, ctypes.byref(bi), 0)
    g.SelectObject(mdc, old)
    g.DeleteObject(bmp)
    g.DeleteDC(mdc)
    u.ReleaseDC(hwnd, hdc)
    bgra = buf.raw
    rgb = bytearray(w * h * 3)
    rgb[0::3], rgb[1::3], rgb[2::3] = bgra[2::4], bgra[1::4], bgra[0::4]
    with open(path, "wb") as f:
        f.write(_png(w, h, rgb))
    return w, h
