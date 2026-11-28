"""Reference output of arabic-ocr (Python) for the edits-ocr parity check.

Run inside arabic-ocr's own environment, from the edits folder:

    uv run --project ../arabic-ocr python -I scripts/ocr-golden.py ../arabic-ocr

For a fixed set of real test pages it writes:
- corpus/files/ocr/<set>-<page>.png: the page as grayscale, lossless. The Rust
  port reads this, so JPEG decoders cannot make the two differ.
- tests/ocr/<set>-<page>.json: what arabic-ocr makes of it, step by step:
  deskew angle, detected boxes, each line's decoded symbols, and the final page
  (arabic-ocr's JSON schema), also with word correction on.

`--debug DIR` also writes the preprocessed page, the detector's map and every
line crop as PNG files, for finding where the two first differ.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import cv2
import numpy as np

#: (test set, number of pages taken from the start of its sorted image list)
PAGES = [
    ("pages_printed", 3),
    ("pages_printed_fa", 2),
    ("pages_printed_ur", 1),
    ("pages_screens", 2),
    ("pages_screens_ui", 2),
    ("pages_screens_v2", 2),
    ("pages_digital_fa", 1),
    ("pages_digital_ur", 1),
]


def r(v: float, n: int = 3) -> float:
    return round(float(v), n)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("arabic_ocr", type=Path, help="arabic-ocr checkout")
    ap.add_argument("--debug", type=Path)
    args = ap.parse_args()
    sys.path.insert(0, str(args.arabic_ocr / "src"))

    from arabic_ocr.detect.boxes import quad_to_rect
    from arabic_ocr.detect.detector import prepare_page
    from arabic_ocr.export.to_json import page_to_dict
    from arabic_ocr.pipeline import OCR, crop_line, load_image
    from arabic_ocr.preprocess import preprocess
    from arabic_ocr.recognize.recognizer import to_gray

    edits = Path(__file__).resolve().parents[1]
    img_dir = edits / "corpus" / "files" / "ocr"
    out_dir = edits / "tests" / "ocr"
    img_dir.mkdir(parents=True, exist_ok=True)
    out_dir.mkdir(parents=True, exist_ok=True)
    ocr = OCR()

    for set_name, count in PAGES:
        images = sorted((args.arabic_ocr / "data" / "real" / set_name / "images").iterdir())
        for path in images[:count]:
            key = f"{set_name}-{path.stem}"
            gray = to_gray(load_image(path))
            ok, png = cv2.imencode(".png", gray)
            assert ok
            (img_dir / f"{key}.png").write_bytes(png.tobytes())

            prep = preprocess(gray)
            dets = ocr.detector.detect(prep.image)
            raw = []
            crops = []
            for d in dets:
                crop, _ = crop_line(prep.image, quad_to_rect(d.quad))
                crops.append(crop)
            decoded = ocr.recognizer.recognize_batch(crops)
            for d, crop, dec in zip(dets, crops, decoded, strict=True):
                raw.append(
                    {
                        "quad": [[r(x), r(y)] for x, y in d.quad],
                        "score": r(d.score, 5),
                        "crop": [int(crop.shape[1]), int(crop.shape[0])],
                        "ids": [int(i) for i in dec.ids],
                        "frames": [int(f) for f in dec.frames],
                        "confidence": r(dec.confidence, 5),
                    }
                )
            page = ocr.read_image(gray)
            corrected = ocr.read_image(gray, correct_words=True)
            out = {
                "image": f"{key}.png",
                "width": int(gray.shape[1]),
                "height": int(gray.shape[0]),
                "angle": float(prep.angle),
                "detections": raw,
                "page": page_to_dict(page),
                "page_corrected": page_to_dict(corrected),
            }
            for p in (out["page"], out["page_corrected"]):
                p.pop("seconds", None)
                p.pop("timings", None)
            (out_dir / f"{key}.json").write_text(
                json.dumps(out, ensure_ascii=False, indent=1) + "\n", encoding="utf-8", newline=""
            )
            print(f"{key}: angle {prep.angle}, {len(dets)} lines, {page.seconds:.2f} s")

            if args.debug:
                dbg = args.debug / key
                dbg.mkdir(parents=True, exist_ok=True)
                cv2.imwrite(str(dbg / "prep.png"), prep.image)
                pp = prepare_page(prep.image, ocr.detector.max_side)
                prob = ocr.detector.probability_map(pp)
                cv2.imwrite(str(dbg / "prob.png"), np.round(prob * 65535).astype(np.uint16))
                for i, c in enumerate(crops):
                    cv2.imwrite(str(dbg / f"crop{i:03}.png"), c)


if __name__ == "__main__":
    main()
