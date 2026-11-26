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

Open a file with `edits.exe path\to\file.pdf` or by dropping it on the window.

## Tests and benchmarks

```powershell
cargo test --workspace
./scripts/gen-corpus.ps1 -Large             # test corpus into corpus/files (about 520 MB with -Large)
target/release/edits-bench check            # every corpus file opens and renders without a crash
target/release/edits-bench core FILE...     # engine timings against PRD section 7
target/release/edits-bench app --exe target/release/edits.exe FILE   # real app: start, open, scroll, zoom, memory
target/release/edits-bench lopdf-check      # lopdf load, decrypt, clean and save on every corpus file
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
