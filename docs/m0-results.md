# M0 results: speed spike

Measured 2026-11-26. Release build installed from the NSIS installer. 5 runs per file, medians unless stated.

**Machine:** AMD Ryzen 7 250 (8 cores / 16 threads), 31 GB RAM, RTX 5060 Laptop GPU, 144 Hz display at 125% scaling, WebView2 158.0. This is faster than the PRD reference machine (4 cores, 8 GB). The numbers still need checking on a reference-class machine.

Reproduce: `edits-bench app --exe <installed edits.exe> --runs 5 FILE`, `edits-bench core FILE...`, `edits-bench startup --exe EXE`.

## Against PRD section 7

| Measure | Target | Result | |
|---|---|---|---|
| Cold start to usable window | ≤ 400 ms | 470–480 ms | **FAIL** |
| Start with 10 MB PDF to first page sharp | ≤ 600 ms | 519 ms | pass |
| Start with 1,000-page / 506 MB PDF to first page sharp | ≤ 1 s | 520 ms | pass |
| Scrolling, frame interval p95 (3,000 px/s) | 60 fps | 7.1 ms (144 Hz, no dropped frames) | pass |
| Frames with a blank page in view | 0 | 0 | pass |
| Page sharp after scrolling into view, p95 | ≤ 100 ms | 20 ms | pass |
| Zoom step to sharp, p95 | ≤ 100 ms | 60 ms (10 MB), 79 ms (506 MB) | pass |
| Memory, document open (private bytes, app + all WebView2 processes) | ≤ 200 MB | 350–380 MB; 160 MB with no file | **FAIL** |
| Search, save, OCR, installer size | | not in M0 scope; installer is 4.7 MB without OCR models | |

## Engine alone (`edits-bench core`)

| File | Open | First page sharp | Tile p50 | Scrolled page sharp p95 |
|---|---|---|---|---|
| images-10mb (10 pages) | 0.5 ms | 22 ms | 2.0 ms | 16 ms |
| latin-text-2000p | 32 ms | 45 ms | 1.5 ms | 11 ms |
| large-1000p-500mb | 18 ms | 40 ms | 2.1 ms | 14 ms |
| arabic-mixed (Edge print) | 8 ms | 21 ms | 0.8 ms | 7 ms |
| arabic-mixed-scan | 2 ms | 41 ms | 3.4 ms | 20 ms |
| vector-heavy-5p (20,000 paths/page) | 0.5 ms | 270 ms | 17 ms | **143 ms** |

The engine is never the bottleneck except on pages with very many vector paths, where PDFium itself needs 140 ms for even a small preview. Every tile re-runs the whole page, so tiling multiplies that cost.

## Tile transfer

Same app, same file, only the transport changed (`EDITS_TRANSPORT`):

| Transport | 1 MB tile, total (of which render) | 12 × 1 MB in parallel | Zoom to sharp p95 |
|---|---|---|---|
| `tile` URL scheme (custom protocol) | 12.6 ms (2.6) | 78 ms | 146 ms — fails |
| Tauri binary IPC | about the same as the URL scheme (it uses one on Windows) | | 152 ms — fails |
| **WebView2 shared buffer** | 4.2 ms (2.6) | 30 ms | 60 ms — passes |

The shared buffer is copied once into memory the WebView's renderer maps directly. Script draws straight from it with `putImageData` and releases it at once, so no garbage is left in the JS heap.

## Where time and memory go

- **Cold start:** spawn → `main` 20 ms; `main` → page navigation starts ≈ 400 ms (window creation and WebView2 environment and controller start-up); navigation → DOM ready 50 ms; → first frame 10 ms. Our own code adds about 60 ms in total. WebView2 browser flags made no difference (tested: GPU off, background features off).
- **Memory:** with no file, WebView2's processes take 160 MB private (GPU process 80, browser 30, renderer 24, utilities 20), and edits.exe 7 MB. With a document, after scrolling and zooming, the GPU process grows to 210–250 MB. That is Chromium's texture and raster caches for the tile canvases: shrinking our canvas cache from 64 to 32 MB barely changed it, and CPU-backed canvases made it worse. The Rust side stays at about 30 MB.

## Decision

**WebView tiles, delivered through WebView2 shared buffers.** Recorded in PRD 8.2.

The native GPU surface fallback is not needed for speed: every target that depends on getting pixels to the screen passes with room to spare. It would also not fix the two failing targets, because both come from WebView2 itself, which the fallback keeps for the rest of the interface.

## Open issues

1. **Cold start target (400 ms).** About 400 ms of it is WebView2 start-up that we do not control. Needs a decision: relax the target, or keep investigating (for example a native splash window that is interactive before the WebView is ready).
2. **Memory target (200 MB).** WebView2 alone is 160 MB. Needs a decision on how the target is defined, for example ≤ 200 MB above the no-file baseline, or ≤ 400 MB in total. GPU-side caching can be reduced further in M1.
3. **Heavy vector pages.** PDFium needs about 140 ms per page for 20,000 paths. Planned for M1: render worker processes in parallel (PRD 8.2), progressive rendering so a slow page can be cancelled, and larger tiles for slow pages.
4. **Reference machine.** All numbers come from a fast development machine. They need repeating on a 4-core / 8 GB laptop.
5. **Real-world corpus.** Only synthetic and generated files so far (corpus/README.md).

## lopdf (PRD 8.1)

`edits-bench lopdf-check` on the 15 corpus files: 12 load, decrypt (AES-256, user and owner passwords), prune, compress and save, and PDFium reopens each result with the same page count. lopdf rejects the 3 damaged files cleanly; PDFium rejects the same 3. lopdf reads the whole file into memory (316 ms for 506 MB) and recompressing 500 MB of images took 17 s. **Verdict: suitable for encryption, cleanup and object-level edits; never on the open or render path.** Confirmed for PRD 8.1.
