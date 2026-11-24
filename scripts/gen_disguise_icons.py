#!/usr/bin/env python3
"""Generate the 11 disguise icons and their single source of truth, assets/disguise/presets.json.

Outputs (all derived, safe to regenerate):
  assets/disguise/<id>.svg       full tile (rounded square), for previews and Linux
  assets/disguise/<id>-fg.svg    glyph only on transparent ground, for Android adaptive icons
  assets/disguise/presets.json   id, name, description, tile colour; read by the UI, the Rust core and the Android patch script
  src-tauri/icons/disguise/<id>.png (512) and <id>-128.png   rendered with rsvg-convert

Run from anywhere: python3 scripts/gen_disguise_icons.py
"""
import json
import math
import pathlib
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SVG_DIR = ROOT / "assets" / "disguise"
PNG_DIR = ROOT / "src-tauri" / "icons" / "disguise"
C = 512  # canvas centre (viewBox is 1024)


def pt(cx, cy, r, deg):
    a = math.radians(deg)
    return (cx + r * math.cos(a), cy + r * math.sin(a))


def poly(points, close=True):
    d = " ".join(f"{x:.1f},{y:.1f}" for x, y in points)
    return d


def line(a, b, stroke, w):
    return f'<line x1="{a[0]:.1f}" y1="{a[1]:.1f}" x2="{b[0]:.1f}" y2="{b[1]:.1f}" stroke="{stroke}" stroke-width="{w}" stroke-linecap="round"/>'


def hexagon(cx, cy, r, start=-90):
    return [pt(cx, cy, r, start + 60 * k) for k in range(6)]


# ---- glyphs (1024 viewBox, kept inside the central 60 % so Android's adaptive mask never clips them) ----

def tally():
    x0, y0, size = 292, 292, 440
    cell = size / 3
    ink, accent = "#C9D3DA", "#E0B35A"
    out = [f'<rect x="{x0}" y="{y0}" width="{size}" height="{size}" rx="34" fill="none" stroke="{ink}" stroke-width="16"/>']
    for k in (1, 2):
        out.append(line((x0 + cell * k, y0), (x0 + cell * k, y0 + size), ink, 12))
        out.append(line((x0, y0 + cell * k), (x0 + size, y0 + cell * k), ink, 12))
    m = 24
    out.append(f'<rect x="{x0 + cell * 2 + m:.1f}" y="{y0 + cell + m:.1f}" width="{cell - 2 * m:.1f}" height="{cell - 2 * m:.1f}" rx="12" fill="{accent}"/>')
    return "".join(out)


def vellum():
    out = []
    sheets = [(-38, -34, "#BFB19A"), (-19, -17, "#D6CBB8"), (0, 0, "#FBF7EE")]
    for dx, dy, fill in sheets:
        out.append(f'<rect x="{332 + dx}" y="{297 + dy}" width="370" height="450" rx="34" fill="{fill}" stroke="#8C7B62" stroke-width="8"/>')
    for i, w in enumerate((250, 250, 160)):
        y = 400 + i * 70
        out.append(line((372, y), (372 + w, y), "#8C7B62", 14))
    return "".join(out)


def drizzle():
    ink = "#35566B"
    cloud = (
        f'<g fill="{ink}">'
        '<rect x="298" y="498" width="430" height="132" rx="66"/>'
        '<circle cx="428" cy="494" r="104"/>'
        '<circle cx="570" cy="452" r="132"/>'
        '<circle cx="668" cy="540" r="84"/>'
        '</g>'
    )
    drops = "".join(line((x, 690), (x - 26, 760), "#5B86A0", 26) for x in (410, 526, 642))
    return cloud + drops


def trim():
    track, fill, ink = "#3A4A43", "#7FB89A", "#5E7068"
    return (
        f'<rect x="262" y="410" width="500" height="150" rx="75" fill="{track}"/>'
        f'<rect x="262" y="410" width="312" height="150" rx="75" fill="{fill}"/>'
        + line((290, 646), (590, 646), ink, 20)
        + line((290, 712), (470, 712), ink, 20)
    )


def plinth():
    outer, inner = hexagon(C, C, 262), hexagon(C, C, 124)
    ink, soft = "#8FA3C7", "#5C6F96"
    out = [f'<polygon points="{poly(outer)}" fill="none" stroke="{ink}" stroke-width="10" stroke-linejoin="round"/>']
    out.append(f'<polygon points="{poly(inner)}" fill="none" stroke="{ink}" stroke-width="10" stroke-linejoin="round"/>')
    for a, b in zip(outer, inner):
        out.append(line(a, b, soft, 8))
    for k in (1, 3, 5):  # the three inner edges that give each hexagon its cube reading
        out.append(line((C, C), outer[k], soft, 8))
        out.append(line((C, C), inner[k], ink, 8))
    return "".join(out)


def kerf():
    return (
        '<rect x="372" y="322" width="58" height="380" rx="14" fill="#ECECEC"/>'
        '<circle cx="598" cy="656" r="48" fill="#E0A24A"/>'
    )


def basalt():
    outer = hexagon(C, C, 270)
    inner = hexagon(C, C, 128, start=-60)
    ink = "#9BA3AD"
    out = [f'<polygon points="{poly(outer)}" fill="none" stroke="{ink}" stroke-width="9" stroke-linejoin="round"/>']
    out.append(f'<polygon points="{poly(inner)}" fill="none" stroke="{ink}" stroke-width="9" stroke-linejoin="round"/>')
    for k in range(6):
        out.append(line(inner[k], outer[k], ink, 9))
        out.append(line(inner[k], outer[(k + 1) % 6], ink, 9))
    for p in outer:
        out.append(f'<circle cx="{p[0]:.1f}" cy="{p[1]:.1f}" r="15" fill="#D8DDE3"/>')
    out.append(f'<circle cx="{C}" cy="{C}" r="15" fill="#D8DDE3"/>')
    return "".join(out)


def cairn():
    ink = "#6FB7A8"
    hx = hexagon(C, C, 150)
    out = [f'<polygon points="{poly(hx)}" fill="none" stroke="{ink}" stroke-width="18" stroke-linejoin="round"/>']
    out.append(f'<circle cx="{C}" cy="{C}" r="44" fill="{ink}"/>')
    for k in (0, 2, 4):  # circuit traces leave three vertices, bend once, end in a pad
        a = hx[k]
        dirn = -90 + 60 * k
        mid = pt(C, C, 262, dirn)
        side = 1 if math.cos(math.radians(dirn)) >= 0 else -1
        end = (mid[0] + side * 78, mid[1])
        out.append(f'<polyline points="{poly([a, mid, end])}" fill="none" stroke="{ink}" stroke-width="14" stroke-linejoin="round" stroke-linecap="round"/>')
        out.append(f'<circle cx="{end[0]:.1f}" cy="{end[1]:.1f}" r="24" fill="#1D2B2B" stroke="{ink}" stroke-width="12"/>')
    return "".join(out)


def kovren():
    back = '<rect x="-190" y="-120" width="380" height="240" rx="36" fill="#7E6B8A" transform="translate(470 470) rotate(-14)"/>'
    front = (
        '<g transform="translate(560 568) rotate(10)">'
        '<rect x="-190" y="-120" width="380" height="240" rx="36" fill="#B8A1A9"/>'
        '<rect x="-190" y="-52" width="380" height="42" fill="#9B8590"/>'
        '<rect x="-150" y="52" width="150" height="20" rx="10" fill="#9B8590"/>'
        '</g>'
    )
    return back + front


def tivlo():
    out = []
    for r in range(3):
        for c in range(3):
            op = 0.32 + 0.085 * (r + c) * 1.0
            out.append(f'<circle cx="{C + (c - 1) * 142}" cy="{C + (r - 1) * 142}" r="36" fill="#9A9AA8" fill-opacity="{op:.2f}"/>')
    return "".join(out)


def orvyn():
    return (
        '<circle cx="446" cy="478" r="196" fill="#E0836B"/>'
        '<circle cx="590" cy="578" r="196" fill="#7C6BC4" fill-opacity="0.88"/>'
    )


PRESETS = [
    ("tally", "Tally", "Minimal grid", "#1F2A33", tally),
    ("vellum", "Vellum", "Stacked paper sheets", "#EDE6D8", vellum),
    ("drizzle", "Drizzle", "Cloud and drops", "#CFE3EE", drizzle),
    ("trim", "Trim", "Storage bar", "#26332E", trim),
    ("plinth", "Plinth", "Nested wireframe prism", "#14171F", plinth),
    ("kerf", "Kerf", "Cursor and dot", "#0F1013", kerf),
    ("basalt", "Basalt", "Wireframe mesh", "#23262B", basalt),
    ("cairn", "Cairn", "Hexagon circuit node", "#1D2B2B", cairn),
    ("kovren", "Kovren", "Two overlapping cards", "#3A3340", kovren),
    ("tivlo", "Tivlo", "Muted dot matrix", "#2B2B33", tivlo),
    ("orvyn", "Orvyn", "Two dusk circles", "#2A2440", orvyn),
]


def svg(body, bg=None):
    ground = f'<rect width="1024" height="1024" rx="224" fill="{bg}"/>' if bg else ""
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">{ground}{body}</svg>\n'


def main():
    if not shutil.which("rsvg-convert"):
        sys.exit("rsvg-convert is required to render the PNGs (librsvg).")
    SVG_DIR.mkdir(parents=True, exist_ok=True)
    PNG_DIR.mkdir(parents=True, exist_ok=True)
    meta = []
    for pid, name, desc, bg, glyph in PRESETS:
        body = glyph()
        (SVG_DIR / f"{pid}.svg").write_text(svg(body, bg))
        (SVG_DIR / f"{pid}-fg.svg").write_text(svg(body))
        for size, suffix in ((512, ""), (128, "-128")):
            subprocess.run(["rsvg-convert", "-w", str(size), "-h", str(size), str(SVG_DIR / f"{pid}.svg"), "-o", str(PNG_DIR / f"{pid}{suffix}.png")], check=True)
        meta.append({"id": pid, "name": name, "description": desc, "background": bg})
    (SVG_DIR / "presets.json").write_text(json.dumps({"presets": meta}, indent=2) + "\n")
    print(f"wrote {len(meta)} presets to {SVG_DIR} and {PNG_DIR}")


if __name__ == "__main__":
    main()
