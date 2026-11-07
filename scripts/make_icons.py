"""Generate every icon from the official logo.

    pip install pillow
    python scripts/make_icons.py

Input:  assets/logo/logo-source.(webp|png|jpg)   (logo on a white background is fine)
Output: assets/logo/logo-1024.png                 transparent master
        src/otm/assets/otm.png + otm-{32,48,64,96,128}.png   app window / sidebar / installer icons
        extension/src/icons/{16,32,48,128}.png    browser extension
        docs/social-preview.png                   1280x640 image for GitHub "Social preview"
"""

import glob
import os
import sys

try:
    from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont
except ImportError:
    sys.exit("Pillow is needed: pip install pillow")

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def find_source():
    for ext in ("png", "webp", "jpg", "jpeg"):
        hits = glob.glob(os.path.join(ROOT, "assets", "logo", "logo-source.%s" % ext))
        if hits:
            return hits[0]
    sys.exit("Put the logo in assets/logo/logo-source.png (or .webp/.jpg)")


def white_to_alpha(img):
    """Remove a white background. Only pixels next to the background get soft ('colour to alpha')
    transparency, so light colours inside the logo stay fully opaque."""
    img = img.convert("RGBA")
    rgb = img.convert("RGB")
    bg = rgb.point(lambda v: 255 if v > 232 else 0).split()
    bg_mask = ImageChops.multiply(ImageChops.multiply(bg[0], bg[1]), bg[2])  # near-white in all channels
    edge = bg_mask.filter(ImageFilter.MaxFilter(7))  # background grown by 3 px = soft edge zone
    px = img.load()
    bgp, edp = bg_mask.load(), edge.load()
    w, h = img.size
    for y in range(h):
        for x in range(w):
            r, g, b, a0 = px[x, y]
            if bgp[x, y]:
                px[x, y] = (0, 0, 0, 0)
                continue
            if not edp[x, y]:
                continue  # inside the logo: keep as is
            a = max(255 - r, 255 - g, 255 - b) / 255.0
            if a < 0.08:
                px[x, y] = (0, 0, 0, 0)
                continue
            a = min(1.0, a * 1.04)
            c = [int(max(0, min(255, (v - (1 - a) * 255) / a))) for v in (r, g, b)]
            px[x, y] = (c[0], c[1], c[2], int(a * a0))
    return img


def square(img, margin=0.03):
    bbox = img.getchannel("A").point(lambda v: 255 if v > 25 else 0).getbbox()
    img = img.crop(bbox)
    side = int(max(img.size) * (1 + 2 * margin))
    out = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    out.paste(img, ((side - img.width) // 2, (side - img.height) // 2), img)
    return out


def resized(img, size):
    return img.convert("RGBa").resize((size, size), Image.LANCZOS).convert("RGBA")


def save(img, *parts):
    path = os.path.join(ROOT, *parts)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    img.save(path, optimize=True)
    print("wrote", os.path.relpath(path, ROOT))


def social_preview(master):
    w, h = 1280, 640
    img = Image.new("RGBA", (w, h), (18, 20, 25, 255))
    logo = resized(master, 300)
    img.paste(logo, (110, (h - 300) // 2), logo)
    d = ImageDraw.Draw(img)

    def font(size, bold=False):
        names = (["segoeuib.ttf", "DejaVuSans-Bold.ttf", "Arial Bold.ttf"] if bold
                 else ["segoeui.ttf", "DejaVuSans.ttf", "Arial.ttf"])
        for n in names:
            try:
                return ImageFont.truetype(n, size)
            except OSError:
                continue
        return ImageFont.load_default()
    d.text((470, 210), "Open Transfer", font=font(76, True), fill=(231, 233, 238))
    d.text((470, 295), "Manager", font=font(76, True), fill=(231, 233, 238))
    d.text((472, 400), "Fast, light, open-source downloads for Linux & macOS", font=font(30), fill=(141, 149, 167))
    return img.convert("RGB")


def main():
    src = find_source()
    master = square(white_to_alpha(Image.open(src)))
    master = resized(master, 1024)
    save(master, "assets", "logo", "logo-1024.png")
    save(resized(master, 256), "src", "otm", "assets", "otm.png")
    for s in (32, 48, 64, 96, 128):
        save(resized(master, s), "src", "otm", "assets", "otm-%d.png" % s)
    for s in (16, 32, 48, 128):
        save(resized(master, s), "extension", "src", "icons", "%d.png" % s)
    save(social_preview(master), "docs", "social-preview.png")


if __name__ == "__main__":
    main()
