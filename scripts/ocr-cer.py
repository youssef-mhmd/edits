"""CER of edits-ocr on arabic-ocr's real test sets, scored by arabic-ocr's own code.

Run inside arabic-ocr's environment, from the edits folder, after a release
build of edits-bench:

    uv run --project ../arabic-ocr python -I scripts/ocr-cer.py ../arabic-ocr
    uv run --project ../arabic-ocr python -I scripts/ocr-cer.py ../arabic-ocr --python

For every page of every set in data/real/pages_*:
1. the page is decoded as arabic-ocr decodes it and stored as a grayscale PNG
   (corpus/files/ocr-cer/<set>/), so JPEG decoders cannot make the two differ;
2. `edits-bench ocr` reads the folder and writes arabic-ocr JSON per page;
3. training/eval_pages.py's scoring (score_boxed / score_text) scores each page.

`--python` also runs arabic-ocr itself on the same PNGs and scores it the same
way (its pages are cached in corpus/files/ocr-cer/<set>-python/), so the two
columns differ only by the port.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from collections import defaultdict
from pathlib import Path
from types import SimpleNamespace

import cv2
import numpy as np


def page_from_json(d: dict) -> SimpleNamespace:
    """What score_boxed / score_text use of a pipeline Page: .lines (box, text), .text."""
    lines = [
        SimpleNamespace(box=np.float32(ln["box"]), text=ln["text"])
        for b in d["blocks"]
        for ln in b["lines"]
    ]
    return SimpleNamespace(lines=lines, text=d["text"])


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("arabic_ocr", type=Path, help="arabic-ocr checkout")
    ap.add_argument("--python", action="store_true", help="also score arabic-ocr itself")
    ap.add_argument("--sets", nargs="*", help="only these sets (default: every pages_* set)")
    ap.add_argument("--out", type=Path, help="write the scores as JSON")
    ap.add_argument("--skip-rust", action="store_true", help="reuse the last edits-bench output")
    ap.add_argument("--only", action="append", default=[], metavar="KEY=VALUE",
                    help="only pages whose label has this value, e.g. modern=true (as eval_pages)")
    args = ap.parse_args()
    root = args.arabic_ocr.resolve()
    sys.path[:0] = [str(root / "src"), str(root)]

    from arabic_ocr.pipeline import OCR, load_image
    from arabic_ocr.recognize.recognizer import to_gray
    from training.eval_pages import VARIANTS, Tally, score_boxed, score_text

    edits = Path(__file__).resolve().parents[1]
    bench = edits / "target" / "release" / "edits-bench.exe"
    base = edits / "corpus" / "files" / "ocr-cer"
    sets = args.sets or sorted(p.name for p in (root / "data" / "real").glob("pages_*"))
    ocr = OCR() if args.python else None

    report: dict = {}
    total: dict[str, Tally] = defaultdict(Tally)
    for name in sets:
        folder = root / "data" / "real" / name
        img_dir, rust_dir, py_dir = base / name, base / f"{name}-rust", base / f"{name}-python"
        img_dir.mkdir(parents=True, exist_ok=True)
        only = dict(kv.split("=", 1) for kv in args.only)
        labels = [
            p for p in sorted((folder / "labels").glob("*.json"))
            if not any(str(json.loads(p.read_text(encoding="utf-8")).get(k)).lower() != v.lower()
                       for k, v in only.items())
        ]
        for label in labels:
            png = img_dir / f"{label.stem}.png"
            if not png.exists():
                src = next((folder / "images").glob(label.stem + ".*"))
                ok, data = cv2.imencode(".png", to_gray(load_image(src)))
                assert ok
                png.write_bytes(data.tobytes())

        if not args.skip_rust:
            subprocess.run([str(bench), "ocr", str(img_dir), str(rust_dir)], check=True,
                           stderr=subprocess.DEVNULL, stdout=subprocess.DEVNULL)

        engines = {"rust": rust_dir}
        if ocr:
            py_dir.mkdir(parents=True, exist_ok=True)
            engines["python"] = py_dir
        row = {}
        for engine, out_dir in engines.items():
            t = Tally()
            for label in labels:
                meta = json.loads(label.read_text(encoding="utf-8"))
                out = out_dir / f"{label.stem}.json"
                if engine == "python" and not out.exists():
                    from arabic_ocr.export.to_json import page_to_dict

                    gray = cv2.imread(str(img_dir / f"{label.stem}.png"), cv2.IMREAD_GRAYSCALE)
                    doc = {"pages": [page_to_dict(ocr.read_image(gray))]}
                    out.write_text(json.dumps(doc, ensure_ascii=False), encoding="utf-8")
                page = page_from_json(json.loads(out.read_text(encoding="utf-8"))["pages"][0])
                pt = Tally()
                pt.pages = 1
                if meta.get("lines") is not None:
                    score_boxed(page, meta, pt)
                else:
                    score_text(page, meta, pt)
                t.add(pt)
            total[engine].add(t)
            row[engine] = t.report()
            print(f"{name:18} {engine:6} " + "  ".join(f"{k} {t.rate(k):6.2%}" for k in VARIANTS),
                  flush=True)
        report[name] = row
    for engine, t in total.items():
        print(f"{'all':18} {engine:6} " + "  ".join(f"{k} {t.rate(k):6.2%}" for k in VARIANTS))
        report.setdefault("all", {})[engine] = t.report()
    if args.out:
        args.out.write_text(json.dumps(report, indent=1) + "\n", encoding="utf-8", newline="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
