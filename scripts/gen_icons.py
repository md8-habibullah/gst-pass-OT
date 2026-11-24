#!/usr/bin/env python3
"""Render every raster icon of the app from the vector sources in src/assets/.

    python3 scripts/gen_icons.py            # needs rsvg-convert (librsvg) and node_modules (npm ci)

  src/assets/logo.svg             the full squircle icon      -> assets/app-icon.png (1024 px)
  src/assets/logo-mark.svg        the transparent H           -> assets/app-icon-fg.png (Android adaptive foreground)
  src/assets/logo-mark-mono.svg   the one-colour H            -> assets/app-icon-monochrome.png (Android 13 themed icons)
  assets/icon-manifest.json       tells `tauri icon` which file is which (background colour #0B0E14)

then `tauri icon assets/icon-manifest.json` writes src-tauri/icons/* (32x32, 128x128, 128x128@2x, icon.png, icon.icns,
icon.ico, Square*Logo, StoreLogo, ios/*, android/*). With Chromium installed it also renders assets/social-preview.png
(1280x640: the picture GitHub shows when the repository is shared; upload it under Settings, Social preview, and the site
uses it as its Open Graph image). The PNGs are committed: only run this when the artwork changes.
The disguise icons (src-tauri/icons/disguise) come from scripts/gen_disguise_icons.py and are not touched.
"""
import json
import pathlib
import re
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "src" / "assets"
OUT = ROOT / "assets"
BG = "#0B0E14"
# The adaptive-icon safe zone is a circle of 66 dp inside the 108 dp layer; the mark's corners must stay inside it.
FG_MARK_SCALE = 0.60


def render(svg: pathlib.Path, png: pathlib.Path, size: int) -> None:
    subprocess.run(["rsvg-convert", "-w", str(size), "-h", str(size), str(svg), "-o", str(png)], check=True)


def inner(svg_text: str) -> str:
    """The drawing inside the root <svg> of one of our files (they have a single root and no scripts)."""
    m = re.search(r"<svg[^>]*>(.*)</svg>", svg_text, re.S)
    if not m:
        raise SystemExit("not an SVG file")
    return m.group(1)


def padded(src: pathlib.Path, dst_svg: pathlib.Path, scale: float) -> None:
    """The 256-unit mark centred on a 1024 canvas at `scale` of the canvas (the rest stays transparent)."""
    side = 1024 * scale
    off = (1024 - side) / 2
    dst_svg.write_text(
        f'<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">'
        f'<svg x="{off}" y="{off}" width="{side}" height="{side}" viewBox="0 0 256 256">{inner(src.read_text())}</svg></svg>'
    )


SOCIAL_HTML = """<!doctype html><html><head><meta charset="utf-8"><style>
@font-face { font-family: Fraunces; src: url("file://@NM@/@fontsource-variable/fraunces/files/fraunces-latin-wght-normal.woff2"); font-weight: 100 900; }
@font-face { font-family: "Instrument Sans"; src: url("file://@NM@/@fontsource-variable/instrument-sans/files/instrument-sans-latin-wght-normal.woff2"); font-weight: 400 700; }
@font-face { font-family: "IBM Plex Mono"; src: url("file://@NM@/@fontsource/ibm-plex-mono/files/ibm-plex-mono-latin-400-normal.woff2"); }
html,body { margin:0; width:1280px; height:640px; background:#0B0E14; overflow:hidden; }
body { position:relative; color:#F8FAFC; font-family:"Instrument Sans",sans-serif; }
.lattice { position:absolute; inset:0; background-image: radial-gradient(circle at 0 0, rgb(155 178 185 / .30) 1.4px, transparent 2px), radial-gradient(circle at 0 0, rgb(155 178 185 / .30) 1.4px, transparent 2px); background-size:36px 62.35px; background-position:0 0,18px 31.18px; -webkit-mask-image: radial-gradient(120% 110% at 18% 10%, #000 0%, rgb(0 0 0/.55) 55%, rgb(0 0 0/.08) 100%); }
.glow { position:absolute; inset:0; background: radial-gradient(60% 80% at 22% 40%, rgb(129 140 248 / .22), transparent 70%); }
.icon { position:absolute; left:104px; top:120px; filter: drop-shadow(0 30px 50px rgb(0 0 0 / .55)); }
.copy { position:absolute; left:560px; top:150px; right:80px; }
h1 { font-family:Fraunces,serif; font-weight:600; font-size:132px; line-height:1; margin:0; letter-spacing:-.02em; }
p.tag { font-size:38px; line-height:1.25; margin:26px 0 0; color:#CBD5E1; font-weight:450; }
.cmd { position:absolute; left:560px; bottom:78px; font-family:"IBM Plex Mono",monospace; font-size:25px; color:#38BDF8; padding:16px 22px; border:1px solid rgb(248 250 252 / .16); border-radius:12px; background: rgb(248 250 252 / .05); }
.cmd b { color:#818CF8; font-weight:400; }
</style></head><body><div class="lattice"></div><div class="glow"></div>
<div class="icon">@LOGO@</div>
<div class="copy"><h1>Hexis</h1><p class="tag">A password vault that stays on your device, and stays secret from the quantum computers of tomorrow.</p></div>
<div class="cmd">curl -fsS …/install.sh <b>|</b> sh</div>
</body></html>"""


def social() -> None:
    """assets/social-preview.png, rendered by headless Chromium from the logo and the app's own fonts."""
    chrome = shutil.which("chromium") or shutil.which("chromium-browser") or shutil.which("google-chrome")
    if not chrome:
        print("Chromium not found: assets/social-preview.png was left as it is")
        return
    logo = re.sub(r"<svg[^>]*>", '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="400" height="400">',
                  (SRC / "logo.svg").read_text(), count=1)
    html = SOCIAL_HTML.replace("@NM@", str(ROOT / "node_modules")).replace("@LOGO@", logo)
    tmp = ROOT / "assets" / ".tmp-social.html"
    tmp.write_text(html)
    try:
        subprocess.run([chrome, "--headless=new", "--disable-gpu", "--no-sandbox", "--hide-scrollbars", "--allow-file-access-from-files",
                        "--force-device-scale-factor=1", "--window-size=1280,640", "--virtual-time-budget=3000",
                        f"--screenshot={OUT / 'social-preview.png'}", f"file://{tmp}"], check=True, timeout=120,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    finally:
        tmp.unlink(missing_ok=True)


def main() -> int:
    if not shutil.which("rsvg-convert"):
        print("rsvg-convert is needed (package librsvg / librsvg2-bin)", file=sys.stderr)
        return 2
    tmp = ROOT / "assets" / ".tmp-icons"
    tmp.mkdir(exist_ok=True)
    try:
        render(SRC / "logo.svg", OUT / "app-icon.png", 1024)
        padded(SRC / "logo-mark.svg", tmp / "fg.svg", FG_MARK_SCALE)
        subprocess.run(["rsvg-convert", "-w", "1024", "-h", "1024", str(tmp / "fg.svg"), "-o", str(OUT / "app-icon-fg.png")], check=True)
        padded(SRC / "logo-mark-mono.svg", tmp / "mono.svg", FG_MARK_SCALE)
        subprocess.run(["rsvg-convert", "-w", "1024", "-h", "1024", str(tmp / "mono.svg"), "-o", str(OUT / "app-icon-monochrome.png")], check=True)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    manifest = {
        "default": "app-icon.png",
        "bg_color": BG,
        "android_fg": "app-icon-fg.png",
        "android_fg_scale": 100,
        "android_monochrome": "app-icon-monochrome.png",
    }
    (OUT / "icon-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    # With a generated Android project present, `tauri icon` writes the Android icons into that (untracked) project and
    # leaves src-tauri/icons/android alone. Move the project aside so the tracked folder is the one that is written.
    gen = ROOT / "src-tauri" / "gen" / "android"
    aside = gen.with_name("android.aside")
    moved = gen.exists() and not aside.exists()
    if moved:
        gen.rename(aside)
    try:
        subprocess.run([str(ROOT / "node_modules" / ".bin" / "tauri"), "icon", str(OUT / "icon-manifest.json")], cwd=ROOT, check=True)
    finally:
        if moved:
            aside.rename(gen)
    # keep a local generated project in step with the tracked icons (CI copies them after `tauri android init`)
    tracked = ROOT / "src-tauri" / "icons" / "android"
    res = gen / "app" / "src" / "main" / "res"
    if res.is_dir():
        for f in tracked.rglob("*"):
            if f.is_file():
                dst = res / f.relative_to(tracked)
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(f, dst)
    social()
    print("icons written to src-tauri/icons; review them, then stage the changes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
