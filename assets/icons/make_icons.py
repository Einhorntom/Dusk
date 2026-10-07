"""Generates Dusk's icons ("Screen at dusk": a monitor whose screen shows the
sun going down) with the Python standard library only.

Small sizes (16-32 px) use geometry snapped to whole pixels so the frame,
sun and stand stay crisp in the tray; larger sizes use the smooth design.

Outputs (next to this script):
  dusk.ico                 colour app icon, 16-256 px (duskd.exe, dusk.exe)
  tray-light.ico           white glyph for dark taskbars
  tray-dark.ico            black glyph for light taskbars
  dusk-256.png             preview (README)
the PowerToys Run and Command Palette images in integrations/, and the
MSIX package images in packaging/msix/Assets, and the Store listing logos
in packaging/store.

Usage: python assets/icons/make_icons.py
"""

import math
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
SUPERSAMPLE = 4  # 4 x 4 samples per pixel for anti-aliasing

# Palette.
BODY = (0x1E, 0x1C, 0x33)
BODY_EDGE = (0x4A, 0x45, 0x78)
SKY = [(0.0, (0x3A, 0x34, 0x82)), (0.6, (0x9A, 0x5A, 0x9C)), (1.0, (0xF1, 0x9A, 0x4E))]
SUN = (0xFF, 0xD2, 0x7A)


# --------------------------------------------------------------- geometry

class Rect:
    def __init__(self, x0, y0, x1, y1, radius=0.0):
        self.x0, self.y0, self.x1, self.y1, self.r = x0, y0, x1, y1, radius

    def contains(self, x, y):
        if not (self.x0 <= x < self.x1 and self.y0 <= y < self.y1):
            return False
        r = self.r
        if r <= 0:
            return True
        cx = min(max(x, self.x0 + r), self.x1 - r)
        cy = min(max(y, self.y0 + r), self.y1 - r)
        return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


class Disc:
    def __init__(self, cx, cy, r):
        self.cx, self.cy, self.r = cx, cy, r

    def contains(self, x, y):
        return (x - self.cx) ** 2 + (y - self.cy) ** 2 <= self.r * self.r


def design(size):
    """Shapes for one icon size, in pixels."""
    if size <= 32:
        # Pixel-snapped geometry on a 16-unit grid.
        unit = size / 16
        t = max(1, round(unit))                      # frame thickness
        margin = max(1, round(unit))
        top, bottom = round(2 * unit), round(12 * unit)
        frame = Rect(margin, top, size - margin, bottom, radius=0 if size < 24 else t)
        screen = Rect(margin + t, top + t, size - margin - t, bottom - t)
        # Off-centre so the sun does not line up with the stand (which made
        # the glyph read as a lamp), and small enough to stay inside the screen.
        sun_r = max(3, round(3.2 * unit))
        sun_x = round(screen.x0 + 0.62 * (screen.x1 - screen.x0))
        sun = Disc(sun_x, screen.y1, sun_r)
        neck_w = max(2, 2 * round(unit))
        neck_h = max(1, round(unit))
        neck = Rect(size / 2 - neck_w / 2, bottom, size / 2 + neck_w / 2, bottom + neck_h)
        base_w = 2 * round(4 * unit)
        base = Rect(size / 2 - base_w / 2, bottom + neck_h, size / 2 + base_w / 2,
                    min(size - max(1, round(unit) - 1), bottom + neck_h + t), radius=0)
        return frame, screen, sun, neck, base
    # Smooth design on a 64-unit grid.
    s = size / 64
    frame = Rect(6 * s, 8 * s, 58 * s, 46 * s, radius=7 * s)
    screen = Rect(10 * s, 12 * s, 54 * s, 42 * s, radius=3 * s)
    sun = Disc(37 * s, 42 * s, 12 * s)
    neck = Rect(27 * s, 46 * s, 37 * s, 53 * s)
    base = Rect(18 * s, 52 * s, 46 * s, 57 * s, radius=2.5 * s)
    return frame, screen, sun, neck, base


# --------------------------------------------------------------- rendering

def sky_colour(t):
    for (p0, c0), (p1, c1) in zip(SKY, SKY[1:]):
        if t <= p1:
            f = (t - p0) / (p1 - p0)
            return tuple(round(a + (b - a) * f) for a, b in zip(c0, c1))
    return SKY[-1][1]


def render(size, paint):
    """`paint(x, y)` returns (r, g, b, a) or None for a sample point; the
    pixel is the average of SUPERSAMPLE^2 samples."""
    pixels = bytearray()
    n = SUPERSAMPLE
    for py in range(size):
        pixels.append(0)  # PNG filter type: none
        for px in range(size):
            r = g = b = a = 0.0
            for sy in range(n):
                for sx in range(n):
                    sample = paint(px + (sx + 0.5) / n, py + (sy + 0.5) / n)
                    if sample:
                        sr, sg, sb, sa = sample
                        r += sr * sa
                        g += sg * sa
                        b += sb * sa
                        a += sa
            if a > 0:
                pixels += bytes((round(r / a), round(g / a), round(b / a), round(a / (n * n))))
            else:
                pixels += b"\0\0\0\0"
    return png(size, size, bytes(pixels))


def app_painter(size):
    frame, screen, sun, neck, base = design(size)
    edge = 1 if size >= 48 else 0

    def paint(x, y):
        if screen.contains(x, y):
            if sun.contains(x, y):
                return (*SUN, 255)
            t = (y - screen.y0) / max(1e-6, screen.y1 - screen.y0)
            return (*sky_colour(t), 255)
        if frame.contains(x, y) or neck.contains(x, y) or base.contains(x, y):
            # A slightly lighter rim keeps the dark body visible on dark backgrounds.
            if edge and not Rect(frame.x0 + 1.2, frame.y0 + 1.2, frame.x1 - 1.2, frame.y1 - 1.2,
                                 max(0, frame.r - 1.2)).contains(x, y) and frame.contains(x, y):
                return (*BODY_EDGE, 255)
            return (*BODY, 255)
        return None

    return paint


def glyph_painter(size, rgb):
    frame, screen, sun, neck, base = design(size)
    # In one colour the sun would merge with the frame and the stand below it
    # (reading as a lamp), so it sits on a gap above the frame's bottom edge.
    gap = max(1.0, round(size / 16))
    sun = Disc(sun.cx, screen.y1 - gap, sun.r)

    def paint(x, y):
        inside_screen = screen.contains(x, y)
        on_sun = inside_screen and y < sun.cy and sun.contains(x, y)
        if (frame.contains(x, y) and not inside_screen) or neck.contains(x, y) \
                or base.contains(x, y) or on_sun:
            return (*rgb, 255)
        return None

    return paint


# --------------------------------------------------------------- file formats

def png(width, height, raw_rows):
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)  # 8-bit RGBA
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
            + chunk(b"IDAT", zlib.compress(raw_rows, 9)) + chunk(b"IEND", b""))


def ico(images):
    """`images`: list of (size, png_bytes). PNG-compressed entries (Windows Vista+)."""
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries, data = b"", b""
    for size, blob in images:
        dim = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset + len(data))
        data += blob
    return header + entries + data


def write(path, blob):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as file:
        file.write(blob)
    print(f"wrote {os.path.relpath(path, HERE)} ({len(blob)} bytes)")


def main():
    app_sizes = [16, 20, 24, 32, 48, 64, 256]
    write(os.path.join(HERE, "dusk.ico"),
          ico([(s, render(s, app_painter(s))) for s in app_sizes]))

    tray_sizes = [16, 20, 24, 32, 48]
    write(os.path.join(HERE, "tray-light.ico"),
          ico([(s, render(s, glyph_painter(s, (0xFF, 0xFF, 0xFF)))) for s in tray_sizes]))
    write(os.path.join(HERE, "tray-dark.ico"),
          ico([(s, render(s, glyph_painter(s, (0x1B, 0x1B, 0x1F)))) for s in tray_sizes]))

    root = os.path.normpath(os.path.join(HERE, "..", ".."))
    run_images = os.path.join(root, "integrations", "PowerToysRun", "Images")
    palette_assets = os.path.join(root, "integrations", "CommandPalette", "Assets")
    msix_assets = os.path.join(root, "packaging", "msix", "Assets")
    store_listing = os.path.join(root, "packaging", "store")
    pngs = {
        # PowerToys Run: light glyph on dark themes, dark glyph on light themes.
        os.path.join(run_images, "dusk.dark.png"): render(64, glyph_painter(64, (0xF0, 0xF0, 0xF0))),
        os.path.join(run_images, "dusk.light.png"): render(64, glyph_painter(64, (0x1E, 0x1E, 0x1E))),
        # Command Palette package assets.
        os.path.join(palette_assets, "Square44x44Logo.png"): render(44, app_painter(44)),
        os.path.join(palette_assets, "Square150x150Logo.png"): render(150, app_painter(150)),
        os.path.join(palette_assets, "StoreLogo.png"): render(50, app_painter(50)),
        # MSIX package (Microsoft Store): tiles, app list and Store logo.
        os.path.join(msix_assets, "Square44x44Logo.png"): render(44, app_painter(44)),
        os.path.join(msix_assets, "Square150x150Logo.png"): render(150, app_painter(150)),
        os.path.join(msix_assets, "StoreLogo.png"): render(50, app_painter(50)),
        # Microsoft Store listing (Partner Center > Store listing > Store logos).
        os.path.join(store_listing, "StoreLogo-300x300.png"): render(300, app_painter(300)),
        os.path.join(store_listing, "StoreLogo-71x71.png"): render(71, app_painter(71)),
        # Preview for the README.
        os.path.join(HERE, "dusk-256.png"): render(256, app_painter(256)),
    }
    for path, blob in pngs.items():
        write(path, blob)


if __name__ == "__main__":
    main()
