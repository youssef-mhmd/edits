//! Corpus checks: every file opens and renders without a crash or hang, and
//! lopdf can load, decrypt, clean and re-save it.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use edits_core::{Engine, EngineError, RenderSpec};

use crate::Res;

const TIMEOUT: Duration = Duration::from_secs(120);
const THUMB_W: u32 = 200;
/// Pages rendered per file in the crash check; very long files are sampled.
const MAX_PAGES: u32 = 400;

/// Password convention for corpus files: `...-user-<password>.pdf`.
pub fn password_for(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    stem.rsplit_once("-user-").map(|(_, pw)| pw.to_string())
}

pub fn pdf_files(dir: &Path) -> Res<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))? {
            let p = entry?.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Child process: open the file, render a thumbnail of each page, print one line.
/// Expected failures (damaged file) are reported, not crashed on.
pub fn render_file(path: &Path) -> Res<bool> {
    let engine = Engine::start()?;
    let t = Instant::now();
    let info = match engine.open(path, password_for(path)) {
        Ok(i) => i,
        Err(e @ (EngineError::Open(_) | EngineError::PasswordRequired)) => {
            println!("REFUSED open: {}", e.to_string().replace(['\n', '\r'], " "));
            return Ok(true);
        }
        Err(e) => return Err(e.into()),
    };
    let step = info.page_count.div_ceil(MAX_PAGES).max(1);
    let mut errors = 0;
    let mut rendered = 0;
    for page in (0..info.page_count).step_by(step as usize) {
        let p = info.pages[page as usize];
        let h = ((THUMB_W as f32 * p.h / p.w).round() as u32).clamp(1, 4096);
        let spec =
            RenderSpec { doc: info.doc, page, page_w: THUMB_W, page_h: h, x: 0, y: 0, w: THUMB_W, h, rotation: 0 };
        match engine.render(spec, 0) {
            Ok(_) => rendered += 1,
            Err(_) => errors += 1,
        }
    }
    println!(
        "OK pages={} rendered={rendered} page_errors={errors} ms={:.0}",
        info.page_count,
        t.elapsed().as_secs_f64() * 1000.0
    );
    Ok(true)
}

fn run_child(args: &[&str]) -> (String, String) {
    let exe = std::env::current_exe().expect("current exe");
    let mut child = match Command::new(exe).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => return ("ERROR".into(), e.to_string()),
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                if let Some(mut s) = child.stdout.take() {
                    let _ = s.read_to_string(&mut out);
                }
                let mut err = String::new();
                if let Some(mut s) = child.stderr.take() {
                    let _ = s.read_to_string(&mut err);
                }
                let line = out.lines().last().unwrap_or("").to_string();
                if status.success() {
                    let verdict = if line.starts_with("REFUSED") { "REFUSED" } else { "OK" };
                    return (verdict.into(), line);
                }
                return ("CRASH".into(), format!("{status} {}", err.trim()));
            }
            Ok(None) if start.elapsed() > TIMEOUT => {
                let _ = child.kill();
                return ("HANG".into(), format!("killed after {}s", TIMEOUT.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return ("ERROR".into(), e.to_string()),
        }
    }
}

pub fn run_all(dir: &Path) -> Res<bool> {
    let files = pdf_files(dir)?;
    if files.is_empty() {
        return Err(format!("no PDF files under {}", dir.display()).into());
    }
    let mut bad = 0;
    for f in &files {
        let (verdict, detail) = run_child(&["render-file", f.to_str().unwrap_or_default()]);
        if verdict == "CRASH" || verdict == "HANG" || verdict == "ERROR" {
            bad += 1;
        }
        println!("{verdict:<8} {}  {detail}", f.display());
    }
    println!("\n{} files, {} crashed or hung", files.len(), bad);
    Ok(bad == 0)
}

/// lopdf evaluation for the PRD 8.1 decision: can it load, decrypt, prune,
/// compress and save each file, and does PDFium still open the result?
pub fn lopdf_all(dir: &Path) -> Res<bool> {
    let files = pdf_files(dir)?;
    let out_dir = std::env::temp_dir().join("edits-lopdf-check");
    std::fs::create_dir_all(&out_dir)?;
    let engine = Engine::start()?;
    let mut ok = 0;
    println!("{:<44} {:>8} {:>8} {:>8} {:>8}  result", "file", "load ms", "save ms", "MB in", "MB out");
    for f in &files {
        let name = f.file_name().unwrap_or_default().to_string_lossy().to_string();
        let size_in = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0) as f64 / 1e6;
        let t = Instant::now();
        let loaded = match password_for(f) {
            Some(pw) => lopdf::Document::load_with_password(f, &pw),
            None => lopdf::Document::load(f),
        };
        let load_ms = t.elapsed().as_secs_f64() * 1000.0;
        let mut doc = match loaded {
            Ok(d) => d,
            Err(e) => {
                println!("{name:<44} {load_ms:>8.0} {:>8} {size_in:>8.1} {:>8}  load failed: {e}", "-", "-");
                continue;
            }
        };
        let pages_lopdf = doc.get_pages().len() as u32;
        let was_encrypted = doc.is_encrypted();
        if was_encrypted && let Err(e) = doc.decrypt(&password_for(f).unwrap_or_default()) {
            println!("{name:<44} {load_ms:>8.0} {:>8} {size_in:>8.1} {:>8}  decrypt failed: {e}", "-", "-");
            continue;
        }
        let t = Instant::now();
        doc.prune_objects();
        doc.delete_zero_length_streams();
        doc.compress();
        let out = out_dir.join(&name);
        if let Err(e) = doc.save(&out) {
            println!("{name:<44} {load_ms:>8.0} {:>8} {size_in:>8.1} {:>8}  save failed: {e}", "-", "-");
            continue;
        }
        let save_ms = t.elapsed().as_secs_f64() * 1000.0;
        let size_out = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0) as f64 / 1e6;
        let verdict = match engine.open(&out, None) {
            Ok(info) if info.page_count == pages_lopdf => {
                ok += 1;
                engine.close(info.doc);
                format!("ok ({pages_lopdf} pages{})", if was_encrypted { ", decrypted" } else { "" })
            }
            Ok(info) => format!("page count changed: lopdf {pages_lopdf}, PDFium {}", info.page_count),
            Err(e) => format!("PDFium cannot open result: {e}"),
        };
        println!("{name:<44} {load_ms:>8.0} {save_ms:>8.0} {size_in:>8.1} {size_out:>8.1}  {verdict}");
        let _ = std::fs::remove_file(&out);
    }
    println!("\n{ok} of {} files round-tripped", files.len());
    Ok(true)
}

/// Checks edits' own PDF reader (edits-edit `pdf`) on every corpus file:
/// page count and annotations against PDFium, and that a file with an
/// update appended by edits opens in PDFium with the same pages.
/// `edits-bench pdf-check [DIR]`
pub fn pdf_all(dir: &Path) -> Res<bool> {
    use edits_edit::pdf::{FileSource, Obj, PdfFile, write};
    let engine = Engine::start()?;
    let tmp = std::env::temp_dir().join(format!("edits-pdf-check-{}.pdf", std::process::id()));
    let mut ok = true;
    for path in pdf_files(dir)? {
        let name = path.strip_prefix(dir).unwrap_or(&path).display().to_string();
        let t = Instant::now();
        let ours = FileSource::open(&path)
            .map_err(|e| e.to_string())
            .and_then(|s| PdfFile::open(s).map_err(|e| e.to_string()));
        let theirs = engine.open(&path, password_for(&path));
        match (ours, theirs) {
            (Err(e), Err(_)) => println!("BOTH-REFUSE {name}: {e}"),
            (Err(e), Ok(info)) => {
                println!("OURS-FAIL  {name}: {e} (PDFium: {} pages)", info.page_count);
                engine.close(info.doc);
            }
            (Ok(_), Err(e)) => println!("PDFIUM-FAIL {name}: {e}"),
            (Ok(f), Ok(info)) => {
                let open_ms = t.elapsed().as_secs_f64() * 1000.0;
                let pages = f.pages().map(|p| p.len() as u32).unwrap_or(0);
                let mut annots = 0;
                let mut problems = Vec::new();
                for i in 0..pages {
                    match f.page_annots(i) {
                        Ok((_, _, a)) => {
                            for o in &a {
                                if let Err(e) = f.resolve(o) {
                                    problems.push(format!("page {i}: {e}"));
                                }
                            }
                            annots += a.len();
                        }
                        Err(e) => problems.push(format!("page {i}: {e}")),
                    }
                }
                engine.close(info.doc);
                let mut status = if pages == info.page_count { "OK" } else { "PAGES-DIFFER" };
                // Round trip: rewrite page 1 unchanged as an update; PDFium must read it.
                if f.is_encrypted() {
                    status = if status == "OK" { "OK-ENCRYPTED" } else { status };
                } else if pages > 0 {
                    let (r, d, _) = f.page_annots(0).map_err(|e| e.to_string())?;
                    let tail = write::update(&f, &[(r, Obj::Dict(d))]);
                    std::fs::copy(&path, &tmp)?;
                    std::fs::OpenOptions::new().append(true).open(&tmp)?.write_all(&tail)?;
                    match engine.open(&tmp, None) {
                        Ok(i2) => {
                            if i2.page_count != info.page_count {
                                status = "UPDATE-PAGES-DIFFER";
                            }
                            engine.close(i2.doc);
                        }
                        Err(e) => {
                            status = "UPDATE-BREAKS";
                            problems.push(e.to_string());
                        }
                    }
                }
                if status != "OK" && status != "OK-ENCRYPTED" {
                    ok = false;
                }
                println!(
                    "{status:<11} {name}: {pages} pages (PDFium {}), {annots} annotations, open {open_ms:.1} ms{}",
                    info.page_count,
                    if problems.is_empty() {
                        String::new()
                    } else {
                        format!(", {} unreadable: {}", problems.len(), problems[0])
                    }
                );
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Ok(ok)
}

/// Page operations on every editable corpus file: last page to the front,
/// first page turned, copied, page 3 deleted, a blank page added. Checked as
/// shown (file plus update) and as saved (a copy of the file, written anew
/// since a page was deleted): page count, sizes and text in the expected
/// order, and every page renders. Also writes page 1 alone as a new file.
pub fn pages_all(dir: &Path) -> Res<bool> {
    use edits_core::{DocInfo, PageSize};
    use edits_edit::Session;
    let engine = Engine::start()?;
    let tmp = std::env::temp_dir().join(format!("edits-pages-check-{}", std::process::id()));
    std::fs::create_dir_all(&tmp)?;
    let mut ok = true;
    let mut checked = 0;
    // Expected pages: index into the original (None: blank) and turned or not.
    type Expect = Vec<(Option<u32>, bool)>;
    fn steps(s: &mut Session, n: u32) -> Result<Expect, String> {
        let mut expect: Expect = (0..n).map(|i| (Some(i), false)).collect();
        let e = |e: edits_edit::EditError| e.to_string();
        if n > 1 {
            s.move_pages(&[n - 1], 0).map_err(e)?;
            let last = expect.pop().unwrap();
            expect.insert(0, last);
        }
        s.rotate_pages(&[0], 1).map_err(e)?;
        expect[0].1 = true;
        s.duplicate_pages(&[0]).map_err(e)?;
        expect.insert(1, expect[0]);
        if expect.len() >= 4 {
            s.delete_pages(&[3]).map_err(e)?;
            expect.remove(3);
        }
        let at = expect.len() as u32;
        s.insert_blank(at, 300.0, 200.0, 1).map_err(e)?;
        expect.push((None, false));
        Ok(expect)
    }
    for path in pdf_files(dir)? {
        let name = path.strip_prefix(dir).unwrap_or(&path).display().to_string();
        let Ok(info) = engine.open(&path, password_for(&path)) else { continue };
        let n = info.page_count;
        let session = Session::open(&path).ok().filter(|s| s.page_count().ok() == Some(n));
        let Some(mut s) = session.filter(|_| n > 0) else {
            engine.close(info.doc);
            continue;
        };
        let fingerprint = |doc, i: u32| -> String {
            engine
                .page_text(doc, i, 0)
                .map(|t| {
                    let mut c: Vec<char> = t.text.chars().filter(|c| !c.is_whitespace()).collect();
                    c.sort_unstable();
                    c.into_iter().collect()
                })
                .unwrap_or_else(|_| "?".into())
        };
        let original: Vec<(PageSize, String)> =
            (0..n).map(|i| (info.pages[i as usize], fingerprint(info.doc, i))).collect();
        let verify = |expect: &Expect, got: &DocInfo| -> Result<(), String> {
            if got.page_count as usize != expect.len() {
                return Err(format!("{} pages, expected {}", got.page_count, expect.len()));
            }
            for (i, (src, turned)) in expect.iter().enumerate() {
                let size = got.pages[i];
                let (want, text) = match src {
                    Some(k) => original[*k as usize].clone(),
                    None => (PageSize { w: 300.0, h: 200.0 }, String::new()),
                };
                let want = if *turned { PageSize { w: want.h, h: want.w } } else { want };
                if (size.w - want.w).abs() > 1.0 || (size.h - want.h).abs() > 1.0 {
                    return Err(format!("page {i} is {}x{}, expected {}x{}", size.w, size.h, want.w, want.h));
                }
                // A turned page's text is read in another order (and Arabic
                // letters are joined differently): its size is checked instead.
                if !turned && fingerprint(got.doc, i as u32) != text {
                    return Err(format!("page {i} has other text"));
                }
                let spec = RenderSpec {
                    doc: got.doc,
                    page: i as u32,
                    page_w: 60,
                    page_h: 60,
                    x: 0,
                    y: 0,
                    w: 60,
                    h: 60,
                    rotation: 0,
                };
                engine.render(spec, 0).map_err(|e| format!("page {i} does not render: {e}"))?;
            }
            Ok(())
        };
        let t = Instant::now();
        let expect = steps(&mut s, n);
        let ops_ms = t.elapsed().as_secs_f64() * 1000.0;
        let shown = expect.clone().and_then(|ex| {
            let got = engine.set_overlay_pages(info.doc, s.tail()).map_err(|e| e.to_string())?;
            verify(&ex, &got)
        });
        engine.close(info.doc);
        // The same on a copy, saved.
        let out = tmp.join("saved.pdf");
        let t2 = Instant::now();
        let saved = shown.clone().and_then(|()| {
            std::fs::copy(&path, &out).map_err(|e| e.to_string())?;
            let mut s2 = Session::open(&out).map_err(|e| e.to_string())?;
            let ex = steps(&mut s2, n)?;
            s2.save(&out).map_err(|e| e.to_string())?;
            let ms = t2.elapsed().as_secs_f64() * 1000.0;
            let got = engine.open(&out, None).map_err(|e| e.to_string())?;
            let r = verify(&ex, &got);
            engine.close(got.doc);
            r.map(|()| ms)
        });
        let part = tmp.join("part.pdf");
        let exported = shown.clone().and_then(|()| {
            s.export(&[0], &part).map_err(|e| e.to_string())?;
            let got = engine.open(&part, None).map_err(|e| e.to_string())?;
            let pages = got.page_count;
            engine.close(got.doc);
            if pages == 1 { Ok(()) } else { Err(format!("export has {pages} pages")) }
        });
        checked += 1;
        let before = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let after = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        // Writing some pages of a damaged file anew is refused, not done badly.
        let refused = matches!(&exported, Err(e) if e.contains("damaged"));
        match (&shown, &saved, &exported) {
            (Ok(()), Ok(_), Err(_)) if refused => println!("OK-DAMAGED  {name}: export refused (damaged file)"),
            (Ok(()), Ok(save_ms), Ok(())) => println!(
                "OK          {name}: {n} pages, {:.1} MB; steps {ops_ms:.0} ms, save {save_ms:.0} ms ({:.0}% of the size)",
                before as f64 / 1e6,
                after as f64 * 100.0 / before.max(1) as f64
            ),
            _ => {
                ok = false;
                let what = [("shown", shown.err()), ("saved", saved.err()), ("export", exported.err())]
                    .into_iter()
                    .filter_map(|(k, e)| e.map(|e| format!("{k}: {e}")))
                    .collect::<Vec<_>>()
                    .join("; ");
                println!("FAIL        {name}: {what}");
            }
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    println!("{checked} files checked");
    Ok(ok)
}
