//! Page clean-up before detection (arabic-ocr preprocess.py): deskew,
//! optional denoise, removal of vertical rules, optional binarization.

use crate::image::{self, Affine, Gray, Interp};

/// Deskew searches angles in [-MAX_SKEW, MAX_SKEW] degrees.
pub const MAX_SKEW: f64 = 10.0;
const RULE_V: f64 = 0.10;
const RULE_LETTERS: f64 = 4.0;

pub struct Preprocessed {
    pub image: Gray,
    /// Degrees the page was rotated by to straighten it (0 = untouched).
    pub angle: f64,
    /// Original pixel -> pixel in `image`.
    pub matrix: Affine,
}

/// 0/1 mask of ink, whatever the polarity.
fn dark_text(g: &Gray) -> Gray {
    let t = image::otsu(g);
    let mut mask = image::threshold_inv(g, t, 1);
    let ink: usize = mask.data.iter().map(|&v| v as usize).sum();
    if ink as f64 / mask.data.len() as f64 > 0.5 {
        mask.data.iter_mut().for_each(|v| *v = 1 - *v);
    }
    mask
}

fn profile_sharpness(mask: &Gray, angle: f64) -> f64 {
    let (w, h) = (mask.width, mask.height);
    let m = image::rotation_matrix(w as f64 / 2.0, h as f64 / 2.0, angle);
    let rot = image::warp_affine(mask, &m, w, h, Interp::Nearest, 0);
    let rows: Vec<f64> = (0..h).map(|y| rot.row(y).iter().map(|&v| v as u64).sum::<u64>() as f64).collect();
    rows.windows(2).map(|p| (p[1] - p[0]) * (p[1] - p[0])).sum()
}

/// Angle (degrees) that makes the text lines horizontal: every degree, then
/// every 0.1 degree around the best, on a copy at most 1000 px.
pub fn estimate_skew(gray: &Gray) -> f64 {
    let scale = (1000.0 / gray.width.max(gray.height) as f64).min(1.0);
    let small = if scale < 1.0 { image::resize_area_fx(gray, scale) } else { gray.clone() };
    let mask = dark_text(&small);
    if mask.data.iter().map(|&v| v as u64).sum::<u64>() < 50 {
        return 0.0;
    }
    let best = |angles: &[f64]| -> f64 {
        let mut best = (angles[0], f64::NEG_INFINITY);
        for &a in angles {
            let s = profile_sharpness(&mask, a);
            if s > best.1 {
                best = (a, s);
            }
        }
        best.0
    };
    // np.arange(-10, 10.5, 1.0) and np.arange(c - 1, c + 1.05, 0.1).
    let coarse_angles: Vec<f64> = arange(-MAX_SKEW, MAX_SKEW + 0.5, 1.0);
    let coarse = best(&coarse_angles);
    let fine = best(&arange(coarse - 1.0, coarse + 1.05, 0.1));
    (fine * 10.0).round_ties_even() / 10.0
}

/// numpy.arange(start, stop, step) for floats.
fn arange(start: f64, stop: f64, step: f64) -> Vec<f64> {
    let n = ((stop - start) / step).ceil().max(0.0) as usize;
    (0..n).map(|i| start + i as f64 * step).collect()
}

/// Rotate around the centre, growing the canvas; new areas take the border
/// colour. Returns the image and the matrix applied.
pub fn rotate(gray: &Gray, angle: f64) -> (Gray, Affine) {
    let (w, h) = (gray.width as f64, gray.height as f64);
    let mut m = image::rotation_matrix(w / 2.0, h / 2.0, angle);
    let (cos, sin) = (m[0].abs(), m[1].abs());
    let nw = (h * sin + w * cos + 0.5) as usize;
    let nh = (h * cos + w * sin + 0.5) as usize;
    m[2] += nw as f64 / 2.0 - w / 2.0;
    m[5] += nh as f64 / 2.0 - h / 2.0;
    let border = image::border_median(gray);
    (image::warp_affine(gray, &m, nw, nh, Interp::Linear, border), m)
}

/// Black text on white with Otsu's threshold.
pub fn binarize(gray: &Gray) -> Gray {
    let mask = dark_text(gray);
    Gray::from_vec(gray.width, gray.height, mask.data.iter().map(|&v| if v > 0 { 0 } else { 255 }).collect())
}

/// Paint long, thin vertical lines (frames, column rules) with the paper
/// colour; a frame line next to text was read as an extra letter.
pub fn remove_rules(gray: &Gray) -> Gray {
    let (w, h) = (gray.width, gray.height);
    let mean = gray.data.iter().map(|&v| v as u64).sum::<u64>() as f64 / gray.data.len() as f64;
    let g =
        if mean < 128.0 { Gray::from_vec(w, h, gray.data.iter().map(|&v| 255 - v).collect()) } else { gray.clone() };
    let block = 15.max(((0.02 * w.max(h) as f64) as usize) | 1);
    let mask = image::adaptive_threshold_inv(&g, 1, block, 15.0);
    let thick = 6.max((0.004 * w.max(h) as f64) as usize);
    let mut heights: Vec<usize> = image::component_heights(&mask).into_iter().filter(|&v| v >= 3).collect();
    let letter = if heights.is_empty() {
        10.0
    } else {
        heights.sort_unstable();
        let n = heights.len();
        if n % 2 == 1 { heights[n / 2] as f64 } else { (heights[n / 2 - 1] + heights[n / 2]) as f64 / 2.0 }
    };
    let kh = 25.max((h as f64 * RULE_V) as usize).max((RULE_LETTERS * letter) as usize);
    let vert = image::open(&mask, 1, kh);
    let thick_parts = image::dilate(&image::open(&vert, thick, 1), 3, 3);
    let rules = Gray::from_vec(w, h, vert.data.iter().zip(&thick_parts.data).map(|(&v, &t)| v & !t).collect());
    if rules.data.iter().all(|&v| v == 0) {
        return gray.clone();
    }
    let rules = image::dilate(&rules, 3, 3);
    let mut paper_px: Vec<u8> = gray.data.iter().zip(&mask.data).filter(|(_, m)| **m == 0).map(|(&v, _)| v).collect();
    let paper = if paper_px.is_empty() { 255 } else { image::median_int(&mut paper_px) };
    let mut out = gray.clone();
    for (o, &r) in out.data.iter_mut().zip(&rules.data) {
        if r > 0 {
            *o = paper;
        }
    }
    out
}

pub fn preprocess(gray: &Gray, deskew: bool, denoise: bool, binarize_page: bool) -> Preprocessed {
    let mut img = gray.clone();
    let mut angle = 0.0;
    let mut matrix = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    if deskew {
        angle = estimate_skew(&img);
        if angle.abs() >= 0.2 {
            let (r, m) = rotate(&img, angle);
            img = r;
            matrix = m;
        } else {
            angle = 0.0;
        }
    }
    if denoise {
        img = image::median3(&img);
    }
    img = remove_rules(&img);
    if binarize_page {
        img = binarize(&img);
    }
    Preprocessed { image: img, angle, matrix }
}
