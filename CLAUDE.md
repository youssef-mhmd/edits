# edits

Desktop PDF reader/editor. Read [docs/PRD.md](docs/PRD.md) and [docs/milestones.md](docs/milestones.md) first. Work on the current milestone only and tick its boxes as items land.

## Rules

- **Speed first.** PRD section 7 targets are checked by `edits-bench`. Run `edits-bench core` (engine) and `edits-bench app` (real window) before and after any change to rendering, transfer or the viewer, and report the numbers. A regression blocks the milestone.
- The interface never waits on PDFium. All PDFium calls run on the render thread in `edits-core` (`Engine`); everything else submits jobs with a priority and can cancel them.
- No GPL/AGPL dependencies (PRD 10.1). `cargo deny check licenses` must pass. Never MuPDF.
- Arabic text is never written without shaping (rustybuzz). Text is kept in logical order.
- Malformed PDFs must never crash or hang the app. Add any file that does to the corpus.

## Build and check

```powershell
./scripts/fetch-pdfium.ps1        # once; PDFium is pinned to the version pdfium-render binds (pdfium_8125)
npm ci
cargo fmt --all; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace
npm run lint; npm run check
npx tauri build --no-bundle       # release exe for benchmarks: target/release/edits.exe
```

Benchmarks need a release build. `edits.exe --bench-out OUT.json [FILE]` runs the in-app script (`ui/src/bench.ts`) and exits.

## Architecture notes

- Tiles: 512 device px, raw RGBA. Default transport on Windows is WebView2 shared buffers (`src-tauri/src/shm.rs`); the `tile` URL scheme and binary IPC remain for comparison (`EDITS_TRANSPORT=protocol|ipc`). See docs/m0-results.md.
- The viewer (`ui/src/viewer.ts`) is plain DOM, not Svelte: per-page preview canvas plus tile canvases placed in percent of page size. Svelte is only the shell.
- `unsafe` is denied workspace-wide; `shm.rs` (WebView2 COM) is the one allowed exception.

## Conventions

- Plain, short sentences in docs and UI text. British/US spelling as already used in the file.
- Corpus files are generated or added by hand under `corpus/files/` (git-ignored). Passwords for encrypted files go in the name: `...-user-<password>.pdf`.
