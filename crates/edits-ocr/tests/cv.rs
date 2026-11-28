//! Image operations against OpenCV's own results (tests/ocr/cv, written by
//! scripts/ocr-cv-fixtures.py). Each case must match byte for byte, except the
//! Gaussian-based adaptive threshold and the cubic resize (done by Intel IPP
//! inside OpenCV), which are matched to within a few pixels per image.

use std::path::{Path, PathBuf};

use edits_ocr::image::{self, Gray, Interp};
use serde_json::Value;

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/ocr/cv")
}

fn load(name: &str) -> (usize, usize, usize, Vec<u8>) {
    let file = std::fs::File::open(dir().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut dec = png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(png::Transformations::IDENTITY);
    let mut r = dec.read_info().expect("png");
    let mut buf = vec![0; r.output_buffer_size().expect("size")];
    let info = r.next_frame(&mut buf).expect("frame");
    buf.truncate(info.buffer_size());
    let cn = info.color_type.samples();
    (info.width as usize, info.height as usize, cn, buf)
}

fn gray(name: &str) -> Gray {
    let (w, h, cn, data) = load(name);
    assert_eq!(cn, 1, "{name} is not gray");
    Gray::from_vec(w, h, data)
}

fn matrix(v: &Value) -> image::Affine {
    let m: Vec<f64> = v
        .as_array()
        .expect("m")
        .iter()
        .flat_map(|r| r.as_array().expect("row").iter().map(|x| x.as_f64().expect("f")))
        .collect();
    [m[0], m[1], m[2], m[3], m[4], m[5]]
}

fn diff(a: &Gray, b: &Gray) -> (usize, u8) {
    assert_eq!((a.width, a.height), (b.width, b.height), "size");
    let mut n = 0;
    let mut worst = 0;
    for (x, y) in a.data.iter().zip(&b.data) {
        if x != y {
            n += 1;
            worst = worst.max(x.abs_diff(*y));
        }
    }
    (n, worst)
}

#[test]
fn opencv_fixtures() {
    let cases: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(dir().join("cases.json")).expect("cases.json")).expect("json");
    let mut failures = Vec::new();
    let mut by_op: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    for c in &cases {
        let op = c["op"].as_str().expect("op");
        let input = c["input"].as_str().expect("input");
        let u = |k: &str| c[k].as_u64().expect(k) as usize;
        let entry = by_op.entry(op.to_string()).or_default();
        entry.0 += 1;
        if op == "otsu" {
            let t = image::otsu(&gray(input));
            if t as f64 != c["value"].as_f64().expect("value") {
                failures.push(format!("otsu {input}: {t} vs {}", c["value"]));
                entry.1 += 1;
            }
            continue;
        }
        let want = gray(c["output"].as_str().expect("output"));
        let got = match op {
            "resize" => {
                let src = gray(input);
                match c["interp"].as_str() {
                    Some("area") => image::resize_area(&src, u("width"), u("height")),
                    _ => image::resize_cubic(&src, u("width"), u("height")),
                }
            }
            "resize_fx" => image::resize_area_fx(&gray(input), c["fx"].as_f64().expect("fx")),
            "warp_linear" | "warp_nearest" => {
                let src = gray(input);
                let src = if op == "warp_nearest" {
                    Gray::from_vec(
                        src.width,
                        src.height,
                        src.data.iter().map(|&v| if v < 128 { 255 } else { 0 }).collect(),
                    )
                } else {
                    src
                };
                let interp = if op == "warp_linear" { Interp::Linear } else { Interp::Nearest };
                image::warp_affine(&src, &matrix(&c["m"]), u("width"), u("height"), interp, u("border") as u8)
            }
            "adaptive" => image::adaptive_threshold_inv(&gray(input), 255, u("block"), c["c"].as_f64().expect("c")),
            "open" | "dilate" => {
                let src = gray(input);
                let bw = Gray::from_vec(
                    src.width,
                    src.height,
                    src.data.iter().map(|&v| if v < 140 { 255 } else { 0 }).collect(),
                );
                if op == "open" { image::open(&bw, u("kw"), u("kh")) } else { image::dilate(&bw, u("kw"), u("kh")) }
            }
            "median3" => image::median3(&gray(input)),
            "gray" | "gray_bgra" => {
                let (w, h, cn, data) = load(input);
                // PNG stores RGB(A); OpenCV wrote it from BGR(A).
                let bgr: Vec<u8> = data
                    .chunks_exact(cn)
                    .flat_map(|p| {
                        let mut v = vec![p[2], p[1], p[0]];
                        if cn == 4 {
                            v.push(p[3]);
                        }
                        v
                    })
                    .collect();
                if cn == 4 { Gray::from_bgra(w, h, &bgr) } else { Gray::from_bgr(w, h, &bgr) }
            }
            other => panic!("unknown op {other}"),
        };
        let (n, worst) = diff(&got, &want);
        // OpenCV's float Gaussian filter, and IPP's cubic resize (closed
        // source), are matched to within a few pixels by one level.
        let cubic = op == "resize" && c["interp"].as_str() == Some("cubic");
        let allowed = if op == "adaptive" || cubic { 2 + want.data.len() / 5000 } else { 0 };
        if n > allowed || (cubic && worst > 1) {
            failures.push(format!("{op} {input} {}: {n} px differ (max {worst})", c["output"]));
            entry.1 += 1;
        }
    }
    for (op, (n, bad)) in &by_op {
        eprintln!("{op:12} {n:4} cases, {bad} failing");
    }
    assert!(failures.is_empty(), "{} of {} cases differ:\n{}", failures.len(), cases.len(), failures.join("\n"));
}
