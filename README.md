# edits

A fast desktop PDF reader and editor for Windows, with first-class Arabic support. Offline and open source (Apache-2.0).

Status: **M0, foundation and speed spike.** Not usable yet. See [docs/PRD.md](docs/PRD.md) for the plan and [docs/milestones.md](docs/milestones.md) for progress.

## Build

Requirements: Windows 10/11 x64, Rust (stable, MSVC), Node.js 22+, WebView2 runtime (included in Windows 11).

```powershell
./scripts/fetch-pdfium.ps1     # downloads the pinned PDFium build into vendor/
npm ci
npx tauri dev                  # run in development
npx tauri build                # installer in target/release/bundle/nsis/
```

Open a file with the Open button (Ctrl O), by dropping it on the window, with `edits.exe path\to\file.pdf`, or after installing, from "Open with" in Explorer.

Keys: Ctrl + / Ctrl − / Ctrl + wheel zoom; Ctrl 0 fit width; Ctrl 2 fit page; Ctrl 1 actual size; Ctrl Shift + / Ctrl Shift − rotate; Ctrl G go to page; Page Up/Down, Space, Home, End; ← → turn pages in single and two-page views; Alt ← / Alt → back and forward after links and bookmarks; F4 pages and bookmarks panel; Ctrl F find, F3 / Shift F3 next and previous; drag to select text, double-click a word, triple-click a line, Ctrl A select page, Ctrl C copy; Ctrl Tab / Ctrl Shift Tab switch tabs; Ctrl W close tab.

## Tests and benchmarks

```powershell
cargo test --workspace
./scripts/gen-corpus.ps1 -Large             # test corpus into corpus/files (about 520 MB with -Large)
target/release/edits-bench check            # every corpus file opens and renders without a crash
target/release/edits-bench core FILE...     # engine timings against PRD section 7
target/release/edits-bench app --exe target/release/edits.exe FILE   # real app: start, open, scroll, zoom, memory
target/release/edits-bench lopdf-check      # lopdf load, decrypt, clean and save on every corpus file
target/release/edits-bench mem-previews FILE # engine memory while rendering every page
target/release/edits-bench text FILE [PAGE]  # a page's text in reading order, as copy and search see it
target/release/edits-bench text-check        # page text against tests/text (reading-order regressions)
target/release/edits-bench search FILE QUERY # search timing, before and after indexing
node scripts/ui-drive.mjs FILE tests/ui/navigation.json OUTDIR   # drive the real app, log state, take screenshots
```

## Layout

```
crates/edits-core/   PDFium wrapper, render thread, priority queue
crates/edits-bench/  benchmarks, corpus generator, corpus checks
src-tauri/           Tauri app: commands, tile transports
ui/                  Svelte 5 front end; ui/src/viewer.ts is the page view
corpus/              test corpus sources and notes (files are generated, not committed)
docs/                PRD, milestones, M0 results
```

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE). PDFium is BSD-3-Clause / Apache-2.0.
