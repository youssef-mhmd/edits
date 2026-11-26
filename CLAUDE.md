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
- PDFium keeps every object it has read (image data included) until a document closes. The render thread reopens documents when process memory passes a limit (`Engine::set_memory_limit_mb`, default 128). `edits-bench mem-previews FILE` shows engine memory while rendering every page.
- Previews are rendered unrotated at 300 px and kept for the whole document as WebP blobs; the canvas is turned with CSS. Tiles are rendered rotated by PDFium.
- UI tests: `node scripts/ui-drive.mjs FILE tests/ui/<scenario>.json OUTDIR` drives the release app over the DevTools protocol (WebView2 `--remote-debugging-port`), with the viewer on `window.__viewer`. Steps act first, then wait, then log/screenshot. The driver sets `EDITS_STATE_DIR` (recent files go to a temp folder, never the user's) and `EDITS_NO_SINGLE_INSTANCE` (never hands off to the user's running app); `UI_SINGLE_INSTANCE=1` keeps single-instance on for `{"launch": FILE}` steps.
- Tabs (`ui/src/App.svelte`) hold `Viewer` instances in `$state.raw`; replace tab objects (`patchTab`), never mutate them, or derived state goes stale. A tab is deactivated while still visible and activated only once shown, or it loses its place.
- Text (`edits-core/src/text.rs`, `reading.rs`): PDFium's character order is not reading order for Arabic, so text is rebuilt from glyph positions (marks clustered with their letter, lines by baseline, visual sort, then bidi). Run `edits-bench text-check` after touching it, and add a golden file to `tests/text` for each document fixed. Search normalisation and match mapping are in `text.rs`, with unit tests.
- Unicode escapes written into files by tools or scripts can turn into the characters themselves (an invisible U+FEFF broke lint once); build them in code and check with `od -c`.
- Bookmarks and links come from `edits-core/src/nav.rs` as page fractions (top-left origin, page /Rotate applied); the viewer applies view rotation.
- When editing files from scripts on Windows, write with `newline=''` (Python) or the file gains CRLF line endings.

## Conventions

- Plain, short sentences in docs and UI text. British/US spelling as already used in the file.
- Corpus files are generated or added by hand under `corpus/files/` (git-ignored). Passwords for encrypted files go in the name: `...-user-<password>.pdf`.
