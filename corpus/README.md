# Test corpus

Files live in `corpus/files/` and are not committed. `scripts/gen-corpus.ps1 [-Large]` rebuilds the reproducible part.

| Folder | Contents | How |
|---|---|---|
| `synthetic/` | Latin text (20 and 2,000 pages), 10 MB of images, 1,000 pages / 506 MB (`-Large`), heavy vector drawing, mixed and rotated page sizes, AcroForm, AES-256 encrypted (user password and owner-only), four damaged files | `edits-bench gen-corpus` |
| `generated/` | Arabic, Persian and Urdu text with diacritics, tatweel, mixed RTL/LTR, tables (`arabic-mixed.pdf`, printed by Edge from `sources/`), and an image-only noisy, skewed scan of it | `gen-corpus.ps1` |
| `real/` | Real-world files, added by hand (28 files, 170 MB, 2026-11-27) | copied from the owner's own folders; not committed |
| `fuzzed/` | Damaged copies of the `real/` files: truncated, bytes flipped, a block zeroed, a block repeated | `edits-bench fuzz-corpus corpus/files/real corpus/files/fuzzed` |

Encrypted files carry their password in the name: `...-user-<password>.pdf`.

## Real-world files so far

Arabic school textbooks (a 106 MB scanned book, a 32 MB one), Arabic summaries made in Word, Arabic and English lecture slides exported from PowerPoint, whiteboard scans, scans from arabic-ocr (plain and with its searchable text layer), notebook and report exports, a survey form. None of them, nor any of their damaged copies, crashes or hangs the app (`edits-bench check`, M1.4).

## Still needed (real-world files)

Synthetic files cannot stand in for these. Add them to `corpus/files/real/` and note their source here (not the files themselves, unless the licence allows):

- Scanned Arabic books and documents (real scanner output, JPEG and JBIG2 compressed).
- Arabic PDFs from Word, InDesign and common Arabic publishing tools.
- Government and bank forms (AcroForm and XFA), filled and empty.
- Large real documents (1,000+ pages; textbooks, manuals).
- PDFs known to break other readers.
- Map/CAD-style pages with very many vector paths.
