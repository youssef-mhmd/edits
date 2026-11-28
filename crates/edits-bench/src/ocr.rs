//! OCR checks: parity with arabic-ocr (Python) and batch runs for scoring.
//!
//! - `ocr-parity [TESTS] [IMAGES]`: every tests/ocr/*.json (written by
//!   scripts/ocr-golden.py) against edits-ocr on the same grayscale page:
//!   same deskew angle, the same lines found (boxes within 2 px), the same
//!   symbols read, and the same final text, lines and word boxes.
//! - `ocr IN OUT [--correct-words]`: read a PNG file or a folder of them and
//!   write one arabic-ocr JSON file per image, for scripts/ocr-cer.py.

use std::path::{Path, PathBuf};
use std::time::Instant;

use edits_ocr::{Engine, Gray, Options, Paths};
use serde_json::Value;

use crate::Res;

/// Boxes may differ by this much (PRD 8.5).
const BOX_TOLERANCE: f64 = 2.0;

pub fn load_png(path: &Path) -> Res<Gray> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info()?;
    let mut buf = vec![0; r.output_buffer_size().ok_or("png too large")?];
    let info = r.next_frame(&mut buf)?;
    buf.truncate(info.buffer_size());
    let (w, h) = (info.width as usize, info.height as usize);
    Ok(match info.color_type {
        png::ColorType::Grayscale => Gray::from_vec(w, h, buf),
        png::ColorType::GrayscaleAlpha => Gray::from_vec(w, h, buf.as_chunks::<2>().0.iter().map(|p| p[0]).collect()),
        png::ColorType::Rgb => {
            let bgr: Vec<u8> = buf.as_chunks::<3>().0.iter().flat_map(|p| [p[2], p[1], p[0]]).collect();
            Gray::from_bgr(w, h, &bgr)
        }
        png::ColorType::Rgba => {
            let bgra: Vec<u8> = buf.as_chunks::<4>().0.iter().flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
            Gray::from_bgra(w, h, &bgra)
        }
        png::ColorType::Indexed => return Err("indexed PNG after EXPAND".into()),
    })
}

fn engine() -> Res<Engine> {
    let paths = Paths::beside_exe();
    if !paths.available() {
        return Err(format!(
            "OCR files missing in {} (run scripts/fetch-ocr.ps1, or set EDITS_OCR_DIR)",
            paths.models.display()
        )
        .into());
    }
    Ok(Engine::new(paths))
}

fn quad(v: &Value) -> Vec<(f64, f64)> {
    v.as_array()
        .map(|a| a.iter().map(|p| (p[0].as_f64().unwrap_or(f64::NAN), p[1].as_f64().unwrap_or(f64::NAN))).collect())
        .unwrap_or_default()
}

fn max_dist(a: &[(f64, f64)], b: &[(f64, f64)]) -> f64 {
    if a.len() != b.len() {
        return f64::INFINITY;
    }
    a.iter().zip(b).map(|(p, q)| (p.0 - q.0).abs().max((p.1 - q.1).abs())).fold(0.0, f64::max)
}

#[derive(Default)]
struct Tally {
    pages: usize,
    pages_ok: usize,
    lines: usize,
    lines_text_ok: usize,
    worst_box: f64,
    chars: usize,
    char_errors: usize,
}

/// Levenshtein distance over chars.
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

pub fn parity(tests: &Path, images: &Path) -> Res<bool> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(tests)
        .map_err(|e| format!("{}: {e}", tests.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("no parity files in {}", tests.display()).into());
    }
    let mut ocr = engine()?;
    let mut t = Tally::default();
    let mut seconds = 0.0;
    for f in &files {
        let want: Value = serde_json::from_str(&std::fs::read_to_string(f)?)?;
        let img = images.join(want["image"].as_str().unwrap_or_default());
        if !img.is_file() {
            println!("{}: image missing ({}); run scripts/ocr-golden.py", f.display(), img.display());
            continue;
        }
        let gray = load_png(&img)?;
        let name = f.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let mut problems: Vec<String> = Vec::new();

        // Steps: deskew, then the lines found and what each reads.
        let prep = edits_ocr::preprocess::preprocess(&gray, true, false, false);
        let want_angle = want["angle"].as_f64().unwrap_or(f64::NAN);
        if (prep.angle - want_angle).abs() > 1e-9 {
            problems.push(format!("angle {} vs {}", prep.angle, want_angle));
        }
        let engine = ocr.load()?;
        let dets = engine.detector.detect(&prep.image)?;
        let want_dets = want["detections"].as_array().cloned().unwrap_or_default();
        if dets.len() != want_dets.len() {
            problems.push(format!("{} lines found vs {}", dets.len(), want_dets.len()));
        }
        let mut crops = Vec::new();
        let mut matched = Vec::new(); // (index in want, our detection)
        for d in &dets {
            let q: Vec<(f64, f64)> = d.quad.iter().map(|p| (p[0] as f64, p[1] as f64)).collect();
            let best = want_dets
                .iter()
                .enumerate()
                .map(|(i, w)| (i, max_dist(&q, &quad(&w["quad"]))))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((i, dist)) = best {
                t.worst_box = t.worst_box.max(dist);
                if dist > BOX_TOLERANCE {
                    problems.push(format!("line box off by {dist:.2} px"));
                }
                matched.push(i);
            }
            let rect = edits_ocr::detect::quad_to_rect(&d.quad);
            crops.push(edits_ocr::pipeline::crop_line(&prep.image, &rect).0);
        }
        // EDITS_OCR_DEBUG=DIR: compare with the crops ocr-golden.py --debug saved.
        if let Some(dbg) = std::env::var_os("EDITS_OCR_DEBUG") {
            let dbg = Path::new(&dbg).join(&name);
            let mut report = Vec::new();
            for (k, (c, &i)) in crops.iter().zip(&matched).enumerate() {
                let p = dbg.join(format!("crop{i:03}.png"));
                if let Ok(py) = load_png(&p) {
                    if (py.width, py.height) != (c.width, c.height) {
                        report.push(format!(
                            "crop {k}->{i}: size {}x{} vs {}x{}",
                            c.width, c.height, py.width, py.height
                        ));
                    } else {
                        let n = c.data.iter().zip(&py.data).filter(|(a, b)| a != b).count();
                        if n > 0 {
                            report.push(format!("crop {k}->{i}: {n} px differ"));
                        }
                    }
                }
            }
            let mut qd: Vec<f64> = dets
                .iter()
                .zip(&matched)
                .map(|(d, &i)| {
                    let q: Vec<(f64, f64)> = d.quad.iter().map(|p| (p[0] as f64, p[1] as f64)).collect();
                    max_dist(&q, &quad(&want_dets[i]["quad"]))
                })
                .collect();
            qd.sort_by(f64::total_cmp);
            report.push(format!("quad diff median {:.4} max {:.4}", qd[qd.len() / 2], qd[qd.len() - 1]));
            let sd: f64 = dets
                .iter()
                .zip(&matched)
                .map(|(d, &i)| (d.score - want_dets[i]["score"].as_f64().unwrap_or(0.0)).abs())
                .fold(0.0, f64::max);
            report.push(format!("score diff max {sd:.6}"));
            if let Ok(py) = load_png(&dbg.join("prep.png")) {
                let n = py.data.iter().zip(&prep.image.data).filter(|(a, b)| a != b).count();
                report.push(format!("prep: {n} px differ"));
            }
            println!("  debug: {}", if report.is_empty() { "crops identical".into() } else { report.join(", ") });
        }
        let workers = engine.workers;
        let decoded = engine.recognizer.recognize_batch(&crops, workers, 16)?;
        for (dec, &i) in decoded.iter().zip(&matched) {
            let ids: Vec<usize> = want_dets[i]["ids"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_u64().map(|v| v as usize)).collect())
                .unwrap_or_default();
            t.lines += 1;
            if dec.ids == ids {
                t.lines_text_ok += 1;
            } else {
                let a = edits_ocr::charset::decode(&dec.ids);
                let b = edits_ocr::charset::decode(&ids);
                problems.push(format!("line {i} reads differently ({} edits)", edit_distance(&a, &b)));
            }
        }

        // The whole page, with and without word correction.
        for (key, correct) in [("page", false), ("page_corrected", true)] {
            let clock = Instant::now();
            let page = ocr.read(&gray, &Options { correct_words: correct, ..Options::default() })?;
            if !correct {
                seconds += clock.elapsed().as_secs_f64();
            }
            let got = edits_ocr::json::page(&page, 0);
            let (gt, wt) = (got["text"].as_str().unwrap_or(""), want[key]["text"].as_str().unwrap_or(""));
            if !correct {
                t.chars += wt.chars().count();
                t.char_errors += edit_distance(gt, wt);
            }
            if gt != wt {
                problems.push(format!("{key}: text differs ({} edits)", edit_distance(gt, wt)));
            }
            let lines = |v: &Value| -> Vec<Value> {
                v["blocks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|b| b["lines"].as_array().cloned().unwrap_or_default())
                    .collect()
            };
            let (gl, wl) = (lines(&got), lines(&want[key]));
            if gl.len() != wl.len() {
                problems.push(format!("{key}: {} lines vs {}", gl.len(), wl.len()));
            }
            for (a, b) in gl.iter().zip(&wl) {
                let d = max_dist(&quad(&a["box"]), &quad(&b["box"]));
                if d > BOX_TOLERANCE {
                    problems.push(format!("{key}: line box off by {d:.2} px"));
                }
                let (aw, bw) = (
                    a["words"].as_array().cloned().unwrap_or_default(),
                    b["words"].as_array().cloned().unwrap_or_default(),
                );
                if aw.len() == bw.len() {
                    for (x, y) in aw.iter().zip(&bw) {
                        let d = max_dist(&quad(&x["box"]), &quad(&y["box"]));
                        t.worst_box = t.worst_box.max(d);
                        if d > BOX_TOLERANCE {
                            problems.push(format!("{key}: word box off by {d:.2} px"));
                        }
                    }
                }
            }
        }
        t.pages += 1;
        problems.dedup();
        if problems.is_empty() {
            t.pages_ok += 1;
            println!("{name}: ok ({} lines)", dets.len());
        } else {
            println!("{name}: {}", problems.join("; "));
        }
    }
    let cer = 100.0 * t.char_errors as f64 / t.chars.max(1) as f64;
    println!(
        "\n{}/{} pages identical; {}/{} lines read the same; worst box difference {:.2} px; \
         page text differs from Python by {cer:.3}% of characters; {:.2} s per page",
        t.pages_ok,
        t.pages,
        t.lines_text_ok,
        t.lines,
        t.worst_box,
        seconds / t.pages.max(1) as f64
    );
    Ok(t.pages_ok == t.pages && t.pages > 0)
}

pub fn batch(input: &Path, out: &Path, correct_words: bool) -> Res<bool> {
    let mut files: Vec<PathBuf> = if input.is_dir() {
        std::fs::read_dir(input)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
            .collect()
    } else {
        vec![input.to_owned()]
    };
    files.sort();
    std::fs::create_dir_all(out)?;
    let mut ocr = engine()?;
    let options = Options { correct_words, ..Options::default() };
    let clock = Instant::now();
    for f in &files {
        let gray = load_png(f)?;
        let page = ocr.read(&gray, &options)?;
        let tm = &page.timings;
        eprintln!(
            "{}: preprocess {:.2} detect {:.2} recognize {:.2} layout {:.3}",
            f.display(),
            tm.preprocess,
            tm.detect,
            tm.recognize,
            tm.layout
        );
        let name = f.file_name().map(|n| n.to_string_lossy().to_string());
        let doc = edits_ocr::json::document(std::slice::from_ref(&page), name.as_deref());
        let stem = f.file_stem().unwrap_or_default().to_string_lossy();
        std::fs::write(out.join(format!("{stem}.json")), serde_json::to_string_pretty(&doc)?)?;
    }
    println!("{} pages, {:.2} s per page", files.len(), clock.elapsed().as_secs_f64() / files.len().max(1) as f64);
    Ok(true)
}

/// Pixels per inch pages are read at (arabic-ocr's PDF_DPI).
pub const PAGE_DPI: f32 = 300.0;

/// The size a page is rendered at for OCR: 300 dpi, smaller for pages so
/// large that a side would pass the engine's largest tile.
pub fn ocr_size(w_pt: f32, h_pt: f32) -> (u32, u32) {
    let scale = (PAGE_DPI / 72.0).min(edits_core::MAX_TILE_SIDE as f32 / w_pt.max(h_pt).max(1.0));
    (((w_pt * scale).round() as u32).max(1), ((h_pt * scale).round() as u32).max(1))
}

/// `ocr-pdf SRC OUT [FIRST LAST]`: reads pages of SRC (all, or FIRST..=LAST,
/// 1-based), writes OUT with an invisible text layer, then reads OUT's text
/// back with PDFium and compares it with what was recognised (whitespace
/// ignored), and searches for a word of each page.
pub fn to_pdf(src: &Path, out: &Path, range: Option<(u32, u32)>) -> Res<bool> {
    use edits_core::{Engine as Pdf, RenderSpec, SearchOptions};
    std::fs::copy(src, out)?;
    let pdf = Pdf::start()?;
    let info = pdf.open(out, None)?;
    let (first, last) = range.unwrap_or((1, info.page_count));
    let pages: Vec<u32> = (first.max(1) - 1..last.min(info.page_count)).collect();
    let mut ocr = engine()?;
    let mut session = edits_edit::Session::open(out)?;
    let mut layers = Vec::new();
    let mut texts = Vec::new();
    let clock = Instant::now();
    for &p in &pages {
        let size = &info.pages[p as usize];
        let (w, h) = ocr_size(size.w, size.h);
        let t = Instant::now();
        let spec = RenderSpec { doc: info.doc, page: p, page_w: w, page_h: h, x: 0, y: 0, w, h, rotation: 0 };
        let tile = pdf.render(spec, 0)?;
        let render = t.elapsed().as_secs_f64();
        let gray = Gray::from_rgba(tile.w as usize, tile.h as usize, &tile.rgba);
        let page = ocr.read(&gray, &Options::default())?;
        let words = page.layer_words();
        let points: Vec<[f32; 2]> =
            words.iter().flat_map(|(q, _)| q.iter().map(|c| [c[0] / tile.w as f32, c[1] / tile.h as f32])).collect();
        let user = pdf.to_user_space(info.doc, p, points)?;
        let layer = edits_edit::OcrLayer {
            words: words
                .into_iter()
                .zip(user.as_chunks::<4>().0)
                .map(|((_, shown), q)| edits_edit::OcrWord { quad: *q, shown })
                .collect(),
        };
        println!(
            "page {}: {}x{} px, render {render:.2} s, OCR {:.2} s, {} words",
            p + 1,
            tile.w,
            tile.h,
            t.elapsed().as_secs_f64() - render,
            layer.words.len()
        );
        layers.push((p, layer));
        texts.push((p, page.text()));
    }
    session.set_page_text(1, layers)?;
    let len = session.save(out)?;
    pdf.close(info.doc);
    println!(
        "{}: {len} bytes, {:.2} s per page",
        out.display(),
        clock.elapsed().as_secs_f64() / pages.len().max(1) as f64
    );

    // Read back.
    let back = pdf.open(out, None)?;
    let squash = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let (mut errors, mut chars, mut found, mut searched) = (0, 0, 0, 0);
    for (p, want) in &texts {
        let got = pdf.page_text(back.doc, *p, 0)?.text;
        let (a, b) = (squash(want), squash(&got));
        let e = edit_distance(&a, &b);
        errors += e;
        chars += a.chars().count();
        if e > 0 {
            println!("page {}: read back with {e} differences in {} characters", p + 1, a.chars().count());
            if std::env::var_os("OCR_DUMP").is_some() {
                std::fs::write(out.with_extension(format!("{}.want.txt", p + 1)), want)?;
                std::fs::write(out.with_extension(format!("{}.got.txt", p + 1)), &got)?;
            }
        }
        // The longest word of the page should be found on it.
        if let Some(word) = want.split_whitespace().max_by_key(|w| w.chars().count()) {
            searched += 1;
            let hits = pdf.search_pages(back.doc, vec![*p], word.to_string(), SearchOptions::default(), 0)?;
            if hits.iter().any(|(_, m)| !m.is_empty()) {
                found += 1;
            } else {
                println!("page {}: \"{word}\" not found", p + 1);
            }
        }
    }
    let rate = errors as f64 / chars.max(1) as f64;
    println!(
        "read back: {errors} differences in {chars} characters ({:.3}%), search found {found} of {searched}",
        rate * 100.0
    );
    Ok(rate < 0.01 && found == searched)
}
