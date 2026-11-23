#!/usr/bin/env python3
"""Draw the placeholder app icon (1024x1024 PNG, stdlib only) to assets/app-icon.png.

A deep-marine rounded square with a brass keyhole (the Strongroom palette). Replace assets/app-icon.png with real artwork any
time, then run `npm run tauri icon assets/app-icon.png` to regenerate src-tauri/icons/*.
"""
import pathlib
import struct
import zlib

SIZE = 1024
OUT = pathlib.Path(__file__).resolve().parent.parent / "assets" / "app-icon.png"
BG = (14, 29, 37)       # pane #0E1D25 (UI_DESIGN_SYSTEM.md)
FG = (226, 178, 89)     # brass #E2B259


def inside_rounded_square(x, y, r=224):
    cx = min(max(x, r), SIZE - r)
    cy = min(max(y, r), SIZE - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def inside_keyhole(x, y):
    if (x - 512) ** 2 + (y - 440) ** 2 <= 130 ** 2:          # head
        return True
    if 520 <= y <= 780:                                        # tapered slot
        t = (y - 520) / 260
        half = 46 + 44 * t
        return abs(x - 512) <= half
    return False


def pixel(px, py):
    # 2x2 supersampling for smooth edges
    cov_shape = cov_glyph = 0
    for dx in (0.25, 0.75):
        for dy in (0.25, 0.75):
            x, y = px + dx, py + dy
            if inside_rounded_square(x, y):
                cov_shape += 1
                if inside_keyhole(x, y):
                    cov_glyph += 1
    if cov_shape == 0:
        return (0, 0, 0, 0)
    g = cov_glyph / cov_shape
    r = round(BG[0] * (1 - g) + FG[0] * g)
    gr = round(BG[1] * (1 - g) + FG[1] * g)
    b = round(BG[2] * (1 - g) + FG[2] * g)
    return (r, gr, b, round(255 * cov_shape / 4))


def png(rows):
    raw = b"".join(b"\x00" + bytes(v for p in row for v in p) for row in rows)

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


if __name__ == "__main__":
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_bytes(png([[pixel(x, y) for x in range(SIZE)] for y in range(SIZE)]))
    print(f"wrote {OUT} ({OUT.stat().st_size} bytes)")
