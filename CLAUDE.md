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

- Render workers (`edits-core/src/worker.rs`): a page whose render takes over 40 ms is "heavy"; its later renders go to up to three worker processes (the app's own exe with `--edits-render-worker`, so `main` must call `worker::serve_if_requested()` first). Cancelling a worker render that has run 30 ms ends the worker; a 20 s watchdog does the same. Workers exit after 30 s idle and when the app's pipe closes. `EDITS_WORKERS=0` turns them off. Tile `render_us` excludes page load time; the viewer gives pages with 512 px tiles slower than 40 ms 1024 px tiles.
- Tiles: 512 device px, raw RGBA. Default transport on Windows is WebView2 shared buffers (`src-tauri/src/shm.rs`); the `tile` URL scheme and binary IPC remain for comparison (`EDITS_TRANSPORT=protocol|ipc`). See docs/m0-results.md.
- The viewer (`ui/src/viewer.ts`) is plain DOM, not Svelte: per-page preview canvas plus tile canvases placed in percent of page size. Svelte is only the shell.
- `unsafe` is denied workspace-wide; `shm.rs` (WebView2 COM) is the one allowed exception.
- PDFium keeps every object it has read (image data included) until a document closes. The render thread reopens documents when process memory passes a limit (`Engine::set_memory_limit_mb`, default 128). `edits-bench mem-previews FILE` shows engine memory while rendering every page.
- Previews are rendered unrotated at 300 px and kept for the whole document as WebP blobs; the canvas is turned with CSS. Tiles are rendered rotated by PDFium.
- UI tests: `node scripts/ui-drive.mjs FILE tests/ui/<scenario>.json OUTDIR` drives the release app over the DevTools protocol (WebView2 `--remote-debugging-port`), with the viewer on `window.__viewer`. Steps act first, then wait, then log/screenshot. The driver sets `EDITS_STATE_DIR` (recent files go to a temp folder, never the user's) and `EDITS_NO_SINGLE_INSTANCE` (never hands off to the user's running app); `UI_SINGLE_INSTANCE=1` keeps single-instance on for `{"launch": FILE}` steps.
- Tabs (`ui/src/App.svelte`) hold `Viewer` instances in `$state.raw`; replace tab objects (`patchTab`), never mutate them, or derived state goes stale. A tab is deactivated while still visible and activated only once shown, or it loses its place.
- Text (`edits-core/src/text.rs`, `reading.rs`): PDFium's character order is not reading order for Arabic, so text is rebuilt from glyph positions (marks clustered with their letter, lines by baseline, visual sort, then bidi). Run `edits-bench text-check` after touching it, and add a golden file to `tests/text` for each document fixed. Search normalisation and match mapping are in `text.rs`, with unit tests.
- Unicode escapes written into files by tools or scripts can turn into the characters themselves (an invisible U+FEFF broke lint once); build them in code and check with `od -c`.
- Window close: the front end's close handler saves positions, then Tauri's JS calls `destroy()`, which needs `core:window:allow-destroy` in the capability. The handler must never throw or wait long. Run `tests/close.ps1` after touching start-up, shutdown, capabilities or background threads.
- Text written into PDFs (`crates/edits-text`): rustybuzz shaping, bidi line layout, glyph ink bounds, and subset embedding of the bundled IBM Plex Sans Arabic (OFL) as a Type0/CIDFontType2 font with a ToUnicode map. Glyphs are placed one by one with `Tm`, so the font's widths only matter to other readers' text extraction. Text boxes grow downwards to fit their text (`Annot::fit_text`); stamps fit their word's ink (`fit_label`, `STAMP_PAD`). `edits-bench annot-demo SRC OUT` writes a sample of every annotation kind for checking in other readers.
- Crash recovery: after every change `edit.rs` writes the session state (`Session::recovery`, JSON) to `<data dir>/recovery/<hash of path>.json`; it is removed on save, on closing the tab, and when the reader discards changes. Opening a file offers it only if the file's size and time are unchanged.
- Interface text is in `ui/src/i18n.svelte.ts` (English and Arabic, every key in both; `t()` is reactive). Arabic sets `dir="rtl"` on `<html>`; the page view (`.scroller`) stays `direction: ltr`. Never name a loop variable `t` in Svelte markup. Colours are CSS variables in `app.css` (`data-theme` light/dark); never hard-code colours in components. Theme and language are copied to localStorage so `main.ts` applies them before the first frame (`look.ts`).
- Print (`ui/src/print.ts`): pages are rendered at 150 or 300 dpi into PNG images in `#print-root` and printed with `window.print()`; `app.css` hides everything else in print.
- Updates: `tauri-plugin-updater` against `latest.json` on GitHub releases; JS plugin versions must match the Rust crates' minor version (Rust 1.88 holds those back). Update checks never run in bench mode.
- Editing (`crates/edits-edit`, `src-tauri/src/edit.rs`): a `Session` per document holds the base file (as last saved), annotations as data in PDF user space with a version history per annotation, and the undo history. After every change it rebuilds the whole incremental update (`pdf::write::update`) and the engine reopens the document as file + update (`Engine::set_overlay`, PDFium reads it through `pdf::Layered`), so the screen shows exactly what will be saved, drawn from edits' own appearance streams. Save copies the base beside the target, appends the update, renames over it (Rust opens files share-delete, so this works while PDFium has it open), then `Engine::reload`s. A document with an update is never offloaded to workers. Annotations from other apps keep their own appearance until recoloured (`own_ap`). Encrypted files, and files whose page count edits' reader sees differently from PDFium (damaged), are not editable. `crates/edits-edit/src/pdf` is a small lazy PDF reader (xref tables and streams, object streams, Flate/PNG predictors); check it with `edits-bench pdf-check`. The viewer's annotation layer (`viewer.ts`, `.alayer`) works in unrotated page fractions and is turned with CSS like the preview; geometry crosses the IPC as page fractions and Rust converts with `PageMap`.
- Organising pages (`crates/edits-edit/src/session/pages.rs`, `src-tauri/src/pages.rs`, `ui/src/PageGrid.svelte`): the session holds the page list as slots with stable keys (annotations refer to keys, the interface to indexes; `Session` translates). Every list is kept for undo. The update rewrites the page tree's top node with a flat /Kids list; pages written again get their inherited attributes made their own. Copies of a base page are new page objects (comments copied as entries, links copied as dictionaries). Pages from other files and the whole-file rewrite use `pdf/copy.rs`. A change of pages comes back with the new page list (`Changed::info`) and the front end builds a new `Viewer` for the tab (`replaceViewer`); the engine reopens with `set_overlay_pages`, which drops the text index. Headers and footers (`session/decor.rs`) are content streams added after the page's own (wrapped in q/Q), with the page's resources copied into its dictionary. Check with `edits-bench pages-check` (corpus) and `edits-bench pages-demo SRC OUT` (sample for other readers); UI: `tests/ui/organize.json`, `organize-large.json`.
- Forms (`crates/edits-edit/src/forms.rs` reads the field tree; `session/fields.rs` holds values with history and writes /V, /AS and appearances; `src-tauri/src/forms.rs`; viewer `drawFields`, `.flayer`, created only on pages with fields and loaded 400 ms after open). Flattening is a session flag (`session/flatten.rs`) that writes every page and draws each annotation's normal appearance into it. Signatures are PNG data URLs kept in `<data dir>/signatures.json` and placed as picture stamps (`NewAnnot::image_data`). Check with `edits-bench forms-check`; UI: `tests/ui/forms.json`, `sign.json`.
- Protected files (`pdf/crypt.rs`): `PdfFile::open_with_password` decrypts objects as they are read; `write::update` and `copy::rewrite` encrypt what they write with the file's key (a rewrite renumbers objects, so it uses `encrypt_renumbered`). Sessions keep the password to reopen after saving. Never write a protected file's objects unencrypted.
- Bookmarks and links come from `edits-core/src/nav.rs` as page fractions (top-left origin, page /Rotate applied); the viewer applies view rotation.
- When editing files from scripts on Windows, write with `newline=''` (Python) or the file gains CRLF line endings.

## Conventions

- Plain, short sentences in docs and UI text. British/US spelling as already used in the file.
- Corpus files are generated or added by hand under `corpus/files/` (git-ignored). Passwords for encrypted files go in the name: `...-user-<password>.pdf`.
