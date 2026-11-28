"""Brand assets for edits: the Kashida e symbol, the app icon, the PDF file
icon and the name set beside the symbol. Writes SVG masters to brand/.

The letters of "edits" and "PDF" are IBM Plex Sans Arabic Bold (OFL, bundled
in crates/edits-text/fonts) turned into outlines, so no font is needed to show
them.

    uv run --no-project --with fonttools python -I scripts/gen-brand.py

Then scripts/gen-brand-icons.py renders the PNG and ICO files.
"""

import math
import os

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "brand")
FONT = os.path.join(ROOT, "crates", "edits-text", "fonts", "IBMPlexSansArabic-Bold.ttf")

# Colours (docs/brand.md).
COBALT = "#2B4EE6"
COBALT_LIGHT = "#4466F5"
COBALT_DARK = "#1F37C2"
CORAL = "#FF6A3D"
INK = "#141A33"
PAPER_EDGE = "#C5CBDB"
FOLD = "#D3DCFB"


def svg(body, w=256, h=256):
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}">'
        f"{body}</svg>\n"
    )


def write(name, text):
    with open(os.path.join(OUT, name), "w", encoding="utf-8", newline="") as f:
        f.write(text)


# --- The symbol -----------------------------------------------------------
# A lowercase e drawn as a ring (centre cx, cy; outer R, inner r) open at the
# lower right, with its bar running on past the bowl like an Arabic kashida
# and ending round, and a text cursor (the accent colour) after it. `small` is the cut for 16-32 px: a heavier
# ring and bar so the counter and the opening stay open.


def e_parts(small=False):
    cx, cy = 104, 128
    R, r = (88, 48) if small else (88, 56)
    half = 20 if small else 16  # half the bar's height
    open_deg = 40 if small else 38  # where the bowl ends, below the bar
    top_deg = 350 if small else 352  # where the bowl meets the bar, on the right
    tail_x = 212 if small else 206  # centre of the round end

    def pt(rad, deg):
        a = math.radians(deg)
        return cx + rad * math.cos(a), cy + rad * math.sin(a)

    o0, o1 = pt(R, open_deg), pt(R, top_deg)
    i1, i0 = pt(r, top_deg), pt(r, open_deg)
    bowl = (
        f"M{o0[0]:.2f} {o0[1]:.2f}A{R} {R} 0 1 1 {o1[0]:.2f} {o1[1]:.2f}"
        f"L{i1[0]:.2f} {i1[1]:.2f}A{r} {r} 0 1 0 {i0[0]:.2f} {i0[1]:.2f}Z"
    )
    x0 = cx - r - 2
    # The bar runs on past the bowl like a kashida and ends round.
    bar = (
        f"M{x0} {cy - half}H{tail_x}A{half} {half} 0 0 1 {tail_x} {cy + half}H{x0}Z"
    )
    # A text cursor stands after it (not in the small cut: it would vanish).
    tail = "" if small else f"M{tail_x + half + 8} {cy - 28}h12v56h-12Z"
    return bowl, bar, tail


def symbol(fill_e, fill_tail=None, small=False, transform=""):
    bowl, bar, tail = e_parts(small)
    t = f' transform="{transform}"' if transform else ""
    return (
        f'<g{t}><path d="{bowl}" fill="{fill_e}"/><path d="{bar}" fill="{fill_e}"/>'
        + (f'<path d="{tail}" fill="{fill_tail or fill_e}"/>' if tail else "")
        + "</g>"
    )


# --- Letters as outlines ----------------------------------------------------

font = TTFont(FONT)
glyphs = font.getGlyphSet()
cmap = font.getBestCmap()
UPM = font["head"].unitsPerEm


def text_paths(text, size, x, baseline, fill, tracking=0.0):
    """`text` as outlined paths, `size` px per em, starting at x on `baseline`."""
    k = size / UPM
    parts = []
    for ch in text:
        name = cmap[ord(ch)]
        pen = SVGPathPen(glyphs)
        glyphs[name].draw(TransformPen(pen, (k, 0, 0, -k, x, baseline)))
        parts.append(f'<path d="{pen.getCommands()}" fill="{fill}"/>')
        x += glyphs[name].width * k + tracking * size
    return "".join(parts), x


def text_width(text, size, tracking=0.0):
    k = size / UPM
    w = sum(glyphs[cmap[ord(c)]].width * k for c in text)
    return w + tracking * size * (len(text) - 1)


# --- Files ------------------------------------------------------------------

os.makedirs(OUT, exist_ok=True)

# Symbol masters: black, white, colour, and the small cut.
write("symbol-black.svg", svg(symbol("#000")))
write("symbol-white.svg", svg(symbol("#fff")))
write("symbol.svg", svg(symbol(COBALT, CORAL)))
write("symbol-small.svg", svg(symbol(COBALT, CORAL, small=True)))
write("symbol-small-black.svg", svg(symbol("#000", small=True)))


def app_icon(small=False):
    """1024 px app icon: the symbol in white on a cobalt tile."""
    k = 3.0 if not small else 3.3
    # Centre the mark between the bowl's centre and the whole mark's centre.
    tx = 512 - k * 116
    ty = 512 - k * 128
    return svg(
        "<defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'>"
        f"<stop offset='0' stop-color='{COBALT_LIGHT}'/><stop offset='1' stop-color='{COBALT_DARK}'/>"
        "</linearGradient></defs>"
        "<rect x='64' y='64' width='896' height='896' rx='216' fill='url(#g)'/>"
        + symbol("#fff", CORAL, small=small, transform=f"translate({tx:.1f} {ty:.1f}) scale({k})"),
        1024,
        1024,
    )


write("app-icon.svg", app_icon())
write("app-icon-small.svg", app_icon(small=True))

# PDF file icon: a white page with a folded corner, the symbol, and a cobalt
# label with "PDF". The small version (16-32 px) has no label.
PAGE = "M58 16H166L218 68V226Q218 240 204 240H58Q44 240 44 226V30Q44 16 58 16Z"
FOLD_PATH = "M166 16V56Q166 68 178 68H218Z"


def file_icon(small=False):
    body = f'<path d="{PAGE}" fill="#fff" stroke="{PAPER_EDGE}" stroke-width="{8 if small else 4}"/>'
    body += f'<path d="{FOLD_PATH}" fill="{FOLD}"/>'
    if small:
        body += symbol(COBALT, CORAL, small=True, transform="translate(31 66) scale(0.72)")
    else:
        body += symbol(COBALT, CORAL, transform="translate(62 34) scale(0.54)")
        body += '<path d="M44 168H218V226Q218 240 204 240H58Q44 240 44 226Z" fill="' + COBALT + '"/>'
        size = 50
        w = text_width("PDF", size, 0.04)
        letters, _ = text_paths("PDF", size, 131 - w / 2, 222, "#fff", 0.04)
        body += letters
    return svg(body)


write("pdf-file.svg", file_icon())
write("pdf-file-small.svg", file_icon(small=True))

# The name beside the symbol (horizontal) and under it (stacked). The bowl's
# height (176) matches the d's ascender; baselines meet at the bowl's bottom.
d_box = glyphs[cmap[ord("d")]]
from fontTools.pens.boundsPen import BoundsPen  # noqa: E402

bp = BoundsPen(glyphs)
d_box.draw(bp)
d_height = bp.bounds[3]  # ascender of d above the baseline, font units
NAME_SIZE = 150 * UPM / d_height  # the d stands 150 px tall


def lockup(ink, mark_e, mark_tail):
    gap = 64
    x = 256 + gap - 30
    letters, end = text_paths("edits", NAME_SIZE, x, 214, ink, -0.01)
    w = math.ceil(end + 12)
    return svg(symbol(mark_e, mark_tail) + letters, w, 256)


write("lockup.svg", lockup(INK, COBALT, CORAL))
write("lockup-white.svg", lockup("#fff", "#fff", CORAL))
write("lockup-black.svg", lockup("#000", "#000", "#000"))


def stacked(ink):
    size = NAME_SIZE * 0.9
    w = text_width("edits", size, -0.01)
    width = max(256, math.ceil(w + 24))
    letters, _ = text_paths("edits", size, width / 2 - w / 2, 380, ink, -0.01)
    return svg(symbol(COBALT, CORAL, transform=f"translate({width / 2 - 133:.1f} 0)") + letters, width, 410)


write("lockup-stacked.svg", stacked(INK))
print("wrote", sorted(os.listdir(OUT)))

# The title bar's logo (ui/src/logo.ts): the small cut in the text colour.
bowl, bar, _ = e_parts(small=True)
logo = (
    '<svg viewBox="16 24 232 208" width="16" height="16" fill="currentColor">'
    f'<path d="{bowl}"/><path d="{bar}"/></svg>'
)
with open(os.path.join(ROOT, "ui", "src", "logo.ts"), "w", encoding="utf-8", newline="") as f:
    f.write(
        "// The edits symbol (small cut) for the title bar. Written by scripts/gen-brand.py.\n"
        f"export const LOGO_SVG =\n  '{logo}';\n"
    )
