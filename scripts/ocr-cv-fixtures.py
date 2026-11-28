"""OpenCV reference results for the image operations edits-ocr ports.

Run inside arabic-ocr's environment, from the edits folder:

    uv run --project ../arabic-ocr python -I scripts/ocr-cv-fixtures.py

Writes tests/ocr/cv/: one input PNG per case, the result PNG, and cases.json
with the parameters. The Rust tests (edits-ocr imgproc) must give the same bytes.
Inputs are line crops cut from the parity pages (corpus/files/ocr) and small
synthetic images, so the fixtures stay small.
"""

from __future__ import annotations

import json
from pathlib import Path

import cv2
import numpy as np

EDITS = Path(__file__).resolve().parents[1]
OUT = EDITS / "tests" / "ocr" / "cv"


def save(name: str, img: np.ndarray) -> str:
    ok, data = cv2.imencode(".png", img)
    assert ok
    (OUT / name).write_bytes(data.tobytes())
    return name


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for old in OUT.glob("*.png"):
        old.unlink()
    rng = np.random.default_rng(7)
    cases: list[dict] = []

    # Inputs: strips of real pages (text lines at several sizes) and noise.
    pages = sorted((EDITS / "corpus" / "files" / "ocr").glob("*.png"))
    inputs = []
    for i, p in enumerate(pages):
        g = cv2.imread(str(p), cv2.IMREAD_GRAYSCALE)
        h, w = g.shape
        y = int(rng.integers(0, max(1, h - 120)))
        x = int(rng.integers(0, max(1, w - 400)))
        hh = int(rng.integers(14, 110))
        ww = int(rng.integers(60, 400))
        inputs.append(save(f"in{i:02}.png", np.ascontiguousarray(g[y : y + hh, x : x + ww])))
    noise = rng.integers(0, 256, (37, 53), dtype=np.uint8)
    inputs.append(save("noise.png", noise))
    color = rng.integers(0, 256, (21, 34, 3), dtype=np.uint8)
    save("color.png", color)
    bgra = rng.integers(0, 256, (17, 29, 4), dtype=np.uint8)
    save("bgra.png", bgra)

    def load(name: str, flags: int = cv2.IMREAD_GRAYSCALE) -> np.ndarray:
        return cv2.imread(str(OUT / name), flags)

    n = 0

    def add(op: str, src: str, out: np.ndarray, **params) -> None:
        nonlocal n
        cases.append({"op": op, "input": src, "output": save(f"out{n:03}.png", out), **params})
        n += 1

    for src in inputs:
        g = load(src)
        h, w = g.shape
        # Line images to the recognizer height: shrink (area) or grow (cubic).
        for height in (64, 32):
            nw = max(1, round(w * height / h))
            interp = cv2.INTER_AREA if h > height else cv2.INTER_CUBIC
            out = cv2.resize(g, (nw, height), interpolation=interp)
            add("resize", src, out, width=nw, height=height,
                interp="area" if interp == cv2.INTER_AREA else "cubic")  # fmt: skip
        for f in (1.7, 3.1):  # always cubic
            nh = max(1, round(h * f))
            nw = max(1, round(w * f))
            add("resize", src, cv2.resize(g, (nw, nh), interpolation=cv2.INTER_CUBIC),
                width=nw, height=nh, interp="cubic")  # fmt: skip
        for s in (0.5, 1 / 3, 0.37, 0.61):  # area shrink by dsize, and by factor
            nw, nh = max(1, round(w * s)), max(1, round(h * s))
            add("resize", src, cv2.resize(g, (nw, nh), interpolation=cv2.INTER_AREA),
                width=nw, height=nh, interp="area")  # fmt: skip
            out = cv2.resize(g, None, fx=s, fy=s, interpolation=cv2.INTER_AREA)
            add("resize_fx", src, out, fx=s, interp="area")
        # Affine warps: crop-like rotations with a constant border, and nearest.
        for k in range(3):
            ang = float(rng.uniform(-8, 8))
            m = cv2.getRotationMatrix2D((w / 2 + rng.uniform(-3, 3), h / 2), ang, 1.0)
            m[:, 2] += rng.uniform(-5, 5, 2)
            ow, oh = w + int(rng.integers(-10, 10)), h + int(rng.integers(-5, 5))
            bv = int(rng.integers(0, 256))
            out = cv2.warpAffine(g, m, (ow, oh), flags=cv2.INTER_LINEAR,
                                 borderMode=cv2.BORDER_CONSTANT, borderValue=bv)  # fmt: skip
            add("warp_linear", src, out, m=m.tolist(), width=ow, height=oh, border=bv)
        m = cv2.getRotationMatrix2D((w / 2, h / 2), float(rng.uniform(-10, 10)), 1.0)
        mask = (g < 128).astype(np.uint8) * 255
        add("warp_nearest", src, cv2.warpAffine(mask, m, (w, h), flags=cv2.INTER_NEAREST),
            m=m.tolist(), width=w, height=h, border=0)  # fmt: skip
        # Thresholds and morphology (preprocess.remove_rules, _dark_text).
        t, _ = cv2.threshold(g, 0, 1, cv2.THRESH_BINARY_INV + cv2.THRESH_OTSU)
        cases.append({"op": "otsu", "input": src, "value": float(t)})
        for block in (15, 31, 71):
            out = cv2.adaptiveThreshold(g, 255, cv2.ADAPTIVE_THRESH_GAUSSIAN_C,
                                        cv2.THRESH_BINARY_INV, block, 15)  # fmt: skip
            add("adaptive", src, out, block=block, c=15)
        bw = (g < 140).astype(np.uint8) * 255
        for kw, kh in ((1, 25), (25, 1), (6, 1), (1, 6), (3, 3)):
            k = cv2.getStructuringElement(cv2.MORPH_RECT, (kw, kh))
            add("open", src, cv2.morphologyEx(bw, cv2.MORPH_OPEN, k), kw=kw, kh=kh)
        add("dilate", src, cv2.dilate(bw, np.ones((3, 3), np.uint8)), kw=3, kh=3)
        add("median3", src, cv2.medianBlur(g, 3))

    add("gray", "color.png", cv2.cvtColor(color, cv2.COLOR_BGR2GRAY))
    alpha = bgra[:, :, 3:4].astype(np.float32) / 255.0
    flat = (bgra[:, :, :3].astype(np.float32) * alpha + 255.0 * (1.0 - alpha)).astype(np.uint8)
    add("gray_bgra", "bgra.png", cv2.cvtColor(flat, cv2.COLOR_BGR2GRAY))

    (OUT / "cases.json").write_text(json.dumps(cases, indent=1) + "\n", encoding="utf-8", newline="")
    size = sum(p.stat().st_size for p in OUT.iterdir()) / 1e6
    print(f"{len(cases)} cases, {size:.1f} MB in {OUT}")


if __name__ == "__main__":
    main()
