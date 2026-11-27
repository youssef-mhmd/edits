# M1.4 results: speed and memory

Measured 2026-11-27, release build, same machine as M0 (AMD Ryzen 7 250, 8 cores / 16 threads, 31 GB, RTX 5060 Laptop, 144 Hz at 125%). "4 cores" runs pin `edits-bench` and everything it starts (the app, its WebView2 processes and render workers) to 4 logical processors with a CPU affinity mask. That stands in for the PRD reference machine's CPU only: it keeps this machine's memory, disk, GPU and clock speed, and 4 logical processors are 2 physical cores here. A real 4-core / 8 GB laptop is still to be measured.

Reproduce: `edits-bench core FILE...`, `edits-bench app --exe target/release/edits.exe --runs 5 FILE`, `edits-bench startup --exe target/release/edits.exe --runs 10`.

## What changed

- **Render workers.** A page whose render takes over 40 ms is heavy. Its later renders go to up to three worker processes (the app's own executable, started on demand), in parallel. The render thread keeps everything else. A worker that crashes or hangs on a page loses only that tile; a 20 s watchdog ends a stuck render.
- **Cancelling a render in progress.** PDFium's progressive rendering needs `unsafe` FFI calls the high-level binding does not expose, so a render that is no longer wanted (scrolled past) is stopped by ending its worker once it has run 30 ms; a new worker starts for the next job.
- **Larger tiles for slow pages.** A page whose 512 px tiles take over 40 ms to draw gets 1024 px tiles. Tile times now exclude page loading, so the first tile on a fresh worker does not trigger this.
- **Tiles out of view** are detached from the page and can be evicted from the tile cache (before, every tile of every page within a screen of the view stayed attached and could not be evicted).
- **Targets decided** (PRD open questions 5 and 6): cold start ≤ 500 ms; memory ≤ 200 MB above the app with no document open, all processes counted.

## Against PRD section 7

App, 5 runs each, medians and p95 as the bench reports them.

| Measure | Target | images-10mb | large-1000p-500mb | real Arabic textbook (107 MB, 162 pages) | vector-heavy-5p |
|---|---|---|---|---|---|
| Cold start (separate run, 10×) | ≤ 500 ms | 436 ms | | | |
| Start with file to first page | ≤ 600 / 1000 ms | 522 | 527 | 728 | **730** |
| Scroll frame interval p95 | 60 fps | 7.1 ms | 7.1 | 7.1 | 7.1 |
| Blank frames | 0 | 0 | 0 | 0 | 0 |
| Page sharp after scrolling, p95 | ≤ 100 ms | 23 | 22 | **189** | **132** |
| Zoom step to sharp, p95 | ≤ 100 ms | 62 | 70 | **133** | 83 |
| Memory above no-file start | ≤ 200 MB | **236** | **332** | **568** | **349** |

Same, pinned to 4 logical processors:

| Measure | Target | images-10mb | large-1000p-500mb | real Arabic textbook | vector-heavy-5p |
|---|---|---|---|---|---|
| Cold start (separate run, 10×) | ≤ 500 ms | **557 ms** | | | |
| Start with file to first page | ≤ 600 / 1000 ms | **666** | 678 | 984 | **1124** |
| Scroll frame interval p95 | 60 fps | 7.1 ms | 7.1 | 7.1 | 7.1 |
| Blank frames | 0 | 0 | 0 | 0 | 0 |
| Page sharp after scrolling, p95 | ≤ 100 ms | 36 | 40 | **271** | **261** |
| Zoom step to sharp, p95 | ≤ 100 ms | 80 | 100 | **232** | **188** |
| Memory above no-file start | ≤ 200 MB | **232** | **311** | **497** | **360** |

Engine alone (`edits-bench core`), vector-heavy-5p, p95: scrolled page to sharp 206 → 87 ms with workers; zoom step 190 → 128 ms. Other synthetic files are unchanged (all pass with large margins).

## Findings

1. **Heavy pages are better but not fixed.** Workers halve the time for pages with tens of thousands of paths, but each worker must parse the page itself first (100 ms and more), so the first sharp view of a heavy page still takes 130–260 ms.
2. **The real Arabic textbook is the hardest file so far.** It is a designed book: 336 images, 79 of them with soft masks (transparency). Tiles take up to 60 ms (p95) and previews 55–100 ms, and background previews compete with visible tiles on the render thread. Two attempts did not help and were not kept: keeping PDFium's decoded-image cache for tiles, and sending every page of a document to the workers once three of its pages are heavy.
3. **Memory is still over the target, by 30–370 MB.** With a document open and idle, the app is about 105 MB above the no-file start (266 MB in total), within the target. Scrolling and zooming grow the WebView2 GPU process by about 100 MB more, and it does not shrink. It is not our canvases: detaching off-screen tiles, a smaller tile cache, reusing canvases (worse) and Chromium memory flags made no difference. `--disable-gpu` brings the large file to 285 MB in total, but zoom-to-sharp rises to 98 ms on this machine, so it would fail on slower ones. The rest is PDFium on large image files (up to 100 MB in edits.exe, bounded by the 128 MB reopen limit).
4. **On 4 cores, cold start is 557 ms** (over 500) and opening a 10 MB file is 666 ms (over 600). Both are mostly WebView2 start-up, which slows with fewer cores.
5. **Real-world corpus:** 28 files from the owner's folders (Arabic textbooks and summaries, slides, scans, forms) and 104 damaged copies of them (`edits-bench fuzz-corpus`). With the synthetic files, 149 files: none crashes or hangs.

## Open

- Memory target: either accept about 350 MB above idle for large image files, or move the page area off the WebView's GPU compositor (PRD 8.2 fallback), which is a large change.
- Heavy pages: keep a worker warm with the document open, so the first heavy page does not pay for starting it; render a heavy page's preview and its first tiles in different workers at once.
- Repeat on a real 4-core / 8 GB laptop.
