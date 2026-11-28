"""Renders the brand SVGs (scripts/gen-brand.py) into the icon files the app
and installer use:

    src-tauri/icons/icon.ico, 32x32.png, 128x128.png, 128x128@2x.png, icon.png
    src-tauri/icons/pdf.ico          the icon Windows shows for PDF files

Sizes up to 32 px are drawn from the small cuts (*-small.svg). Rendering uses
headless Chrome or Edge. icon.icns comes from `npx tauri icon brand/app-icon.png`.

    python -I scripts/gen-brand-icons.py
"""

import os
import shutil
import struct
import subprocess
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BRAND = os.path.join(ROOT, "brand")
ICONS = os.path.join(ROOT, "src-tauri", "icons")

ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]
SMALL_MAX = 32


def browser():
    for p in (
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    ):
        if os.path.exists(p):
            return p
    raise SystemExit("needs Chrome or Edge to render")


def render(svg_path, size, out_png, tmp):
    """Renders an SVG to a transparent PNG of size × size."""
    page = os.path.join(tmp, "page.html")
    with open(svg_path, encoding="utf-8") as f:
        svg = f.read()
    with open(page, "w", encoding="utf-8", newline="") as f:
        f.write(
            "<!doctype html><style>html,body{margin:0;background:transparent}"
            f"svg{{display:block;width:{size}px;height:{size}px}}</style>{svg}"
        )
    subprocess.run(
        [
            browser(),
            "--headless=new",
            "--disable-gpu",
            "--hide-scrollbars",
            "--force-device-scale-factor=1",
            "--default-background-color=00000000",
            f"--window-size={size},{size}",
            f"--screenshot={out_png}",
            "file:///" + page.replace("\\", "/"),
        ],
        check=True,
        capture_output=True,
    )


def ico(pngs, out):
    """An ICO holding PNG images; `pngs` maps size to PNG bytes."""
    sizes = sorted(pngs)
    head = struct.pack("<HHH", 0, 1, len(sizes))
    offset = 6 + 16 * len(sizes)
    entries, data = b"", b""
    for s in sizes:
        b = pngs[s]
        d = 0 if s >= 256 else s
        entries += struct.pack("<BBBBHHII", d, d, 0, 0, 1, 32, len(b), offset + len(data))
        data += b
    with open(out, "wb") as f:
        f.write(head + entries + data)


def build(big, small, out_ico, tmp, keep=()):
    pngs = {}
    for s in ICO_SIZES:
        src = small if s <= SMALL_MAX else big
        p = os.path.join(tmp, f"{os.path.basename(out_ico)}-{s}.png")
        render(os.path.join(BRAND, src), s, p, tmp)
        with open(p, "rb") as f:
            pngs[s] = f.read()
    ico(pngs, out_ico)
    return {s: os.path.join(tmp, f"{os.path.basename(out_ico)}-{s}.png") for s in ICO_SIZES}


with tempfile.TemporaryDirectory() as tmp:
    app = build("app-icon.svg", "app-icon-small.svg", os.path.join(ICONS, "icon.ico"), tmp)
    build("pdf-file.svg", "pdf-file-small.svg", os.path.join(ICONS, "pdf.ico"), tmp)
    shutil.copy(app[32], os.path.join(ICONS, "32x32.png"))
    shutil.copy(app[128], os.path.join(ICONS, "128x128.png"))
    for s, name in ((256, "128x128@2x.png"), (512, "icon.png"), (1024, os.path.join("..", "..", "brand", "app-icon.png"))):
        render(os.path.join(BRAND, "app-icon.svg"), s, os.path.join(ICONS, name), tmp)
    render(os.path.join(BRAND, "pdf-file.svg"), 256, os.path.join(BRAND, "pdf-file.png"), tmp)
    render(os.path.join(BRAND, "symbol.svg"), 512, os.path.join(BRAND, "symbol.png"), tmp)
print("icons written to", ICONS)
