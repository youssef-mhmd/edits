//! The full pipeline (arabic-ocr pipeline.py): page image -> blocks, lines and
//! words with boxes, in reading order.
//!
//! 1. Preprocess: deskew, remove vertical rules.
//! 2. Detect a box around every line.
//! 3. Cut each line out, turned horizontal, with a small margin.
//! 4. Recognize every line (CTC).
//! 5. Reading order: columns, blocks.
//! 6. Options: diacritics, digits, tatweel; noise lines dropped.
//!
//! Boxes are in pixels of the image given, corners TL, TR, BR, BL.

use crate::Result;
use crate::bidi::logical_order;
use crate::charset::{self, Digits};
use crate::correct::Lexicon;
use crate::ctc::{self, Decoded};
use crate::detect::{RotRect, quad_to_rect};
use crate::image::{self, Affine, Gray, Interp, py_round};
use crate::layout;
use crate::models::{Detector, Recognizer};
use crate::preprocess::preprocess;

/// Margin around a detected line before reading it, as a fraction of its
/// height: above and below, and at both ends.
pub const PAD_Y: f64 = 0.05;
pub const PAD_X: f64 = 0.2;

/// Corners TL, TR, BR, BL (float32, as numpy keeps them).
pub type Corners = [[f32; 2]; 4];

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Options {
    pub diacritics: bool,
    pub digits: Digits,
    pub tatweel: bool,
    pub deskew: bool,
    pub denoise: bool,
    pub binarize: bool,
    pub drop_noise: bool,
    pub correct_words: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            diacritics: true,
            digits: Digits::Keep,
            tatweel: true,
            deskew: true,
            denoise: false,
            binarize: false,
            drop_noise: true,
            correct_words: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Word {
    /// Logical order.
    pub text: String,
    pub confidence: f64,
    pub corners: Corners,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub text: String,
    pub confidence: f64,
    pub corners: Corners,
    /// Reading order (right to left).
    pub words: Vec<Word>,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub lines: Vec<Line>,
}

impl Block {
    pub fn text(&self) -> String {
        self.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n")
    }

    pub fn corners(&self) -> Corners {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
        for l in &self.lines {
            for p in &l.corners {
                x0 = x0.min(p[0]);
                y0 = y0.min(p[1]);
                x1 = x1.max(p[0]);
                y1 = y1.max(p[1]);
            }
        }
        [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }
}

#[derive(Clone, Debug, Default)]
pub struct Timings {
    pub preprocess: f64,
    pub detect: f64,
    pub recognize: f64,
    pub layout: f64,
}

#[derive(Clone, Debug)]
pub struct Page {
    pub width: usize,
    pub height: usize,
    /// Deskew rotation applied, degrees.
    pub angle: f64,
    pub blocks: Vec<Block>,
    pub timings: Timings,
}

impl Page {
    pub fn lines(&self) -> impl Iterator<Item = &Line> {
        self.blocks.iter().flat_map(|b| &b.lines)
    }

    /// Blocks separated by an empty line, lines by a newline.
    pub fn text(&self) -> String {
        self.blocks.iter().map(Block::text).collect::<Vec<_>>().join("\n\n")
    }

    /// The words for an invisible text layer, as arabic-ocr's searchable
    /// PDF writes them: corners in page pixels (top-left, top-right,
    /// bottom-right, bottom-left) and the characters in display order, left
    /// to right as drawn, with a space after each word (it separates words
    /// for copy and search, and PDFium drops a text piece of one character).
    /// Brackets are stored as drawn, mirrored: readers mirror them back
    /// when they rebuild right-to-left text (PDFium does).
    pub fn layer_words(&self) -> Vec<(Corners, String)> {
        let mirror = |c: char| match c {
            '(' => ')',
            ')' => '(',
            '[' => ']',
            ']' => '[',
            '{' => '}',
            '}' => '{',
            '<' => '>',
            '>' => '<',
            '«' => '»',
            '»' => '«',
            c => c,
        };
        self.lines()
            .flat_map(|l| &l.words)
            .filter(|w| !w.text.is_empty())
            .map(|w| {
                (w.corners, crate::bidi::reading_order(&format!("{} ", w.text)).chars().rev().map(mirror).collect())
            })
            .collect()
    }
}

/// Cut out a (possibly rotated) line with margins as a horizontal image, and
/// the matrix mapping crop pixels back to page pixels.
pub fn crop_line(gray: &Gray, rect: &RotRect) -> (Gray, Affine) {
    let h = rect.h * (1.0 + 2.0 * PAD_Y);
    let w = rect.w + 2.0 * PAD_X * rect.h;
    let big = RotRect { w, h, ..*rect };
    let [tr, tl, bl, _] = big.quad();
    let (cw, ch) = (py_round(w).max(1) as usize, py_round(h).max(1) as usize);
    let dst = [[cw as f32, 0.0], [0.0, 0.0], [0.0, ch as f32]];
    let m = image::affine_from_points([tr, tl, bl], dst);
    let (sy, sx) = ((gray.height / 64).max(1), (gray.width / 64).max(1));
    let mut sample = Vec::new();
    for y in (0..gray.height).step_by(sy) {
        for x in (0..gray.width).step_by(sx) {
            sample.push(gray.at(x, y));
        }
    }
    let border = image::median_int(&mut sample);
    let crop = image::warp_affine(gray, &m, cw, ch, Interp::Linear, border);
    (crop, image::invert_affine(&m))
}

/// Corners of x0..x1, y0..y1 (as float32) mapped by `m`, kept as float32.
fn map_rect(m: &Affine, x0: f64, x1: f64, y0: f64, y1: f64) -> Corners {
    let (x0, x1, y0, y1) = (x0 as f32 as f64, x1 as f32 as f64, y0 as f32 as f64, y1 as f32 as f64);
    [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| {
        let (a, b) = image::apply(m, x, y);
        [a as f32, b as f32]
    })
}

/// Split a decoded line at its spaces: (text, confidence, x0, x1) in crop
/// pixels, right to left. Each word reaches from the space before it to the
/// space after it, so word boxes tile the line.
pub fn split_words(
    dec: &Decoded,
    crop_w: usize,
    crop_h: usize,
    model_h: usize,
    stride: usize,
) -> Vec<(String, f64, f64, f64)> {
    let px = stride as f64 * crop_h as f64 / model_h as f64;
    let cw = crop_w as f64;
    let x_of = |frame: f64| (cw - frame * px).clamp(0.0, cw);
    let mut ids = dec.ids.clone();
    ids.push(charset::SPACE_ID);
    let mut frames: Vec<f64> = dec.frames.iter().map(|&f| f as f64).collect();
    frames.push(cw / px);
    let mut words = Vec::new();
    let (mut start, mut edge) = (0usize, 0.0f64);
    for (k, &i) in ids.iter().enumerate() {
        if i != charset::SPACE_ID {
            continue;
        }
        if k > start {
            let text = logical_order(&charset::decode(&ids[start..k]));
            let conf = if dec.probs.is_empty() { dec.confidence } else { ctc::mean(&dec.probs[start..k]) };
            words.push((text, conf, x_of(frames[k]), x_of(edge)));
        }
        start = k + 1;
        edge = frames[k];
    }
    words
}

/// Lines that are page noise, not text: borders and rules read as letters,
/// specks, ornament rows.
pub fn is_noise(line: &Line) -> bool {
    let text: Vec<char> = charset::strip_tatweel(&line.text).chars().filter(|&c| c != ' ').collect();
    if text.is_empty() {
        return true;
    }
    let n = text.len() as f64;
    let letters = text.iter().filter(|&&c| charset::category(c) == b'L').count() as f64 / n;
    let conf = line.confidence;
    if conf < 0.5 {
        return true;
    }
    if conf < 0.65 && (text.len() <= 3 || letters < 0.5) {
        return true;
    }
    let count = |c: char| text.iter().filter(|&&t| t == c).count();
    let mut common = text[0];
    for &c in &text {
        if count(c) > count(common) {
            common = c;
        }
    }
    let symbol = !matches!(charset::category(common), b'L' | b'N');
    symbol && text.len() >= 6 && count(common) as f64 / n > 0.4 && conf < 0.9
}

pub fn postprocess(text: &str, o: &Options) -> String {
    let mut t = text.to_string();
    if !o.diacritics {
        t = charset::strip_diacritics(&t);
    }
    if !o.tatweel {
        t = charset::strip_tatweel(&t);
    }
    charset::convert_digits(&t, o.digits)
}

/// The OCR engine: models loaded once, then any number of pages.
pub struct Ocr {
    pub detector: Detector,
    pub recognizer: Recognizer,
    pub lexicon: Option<Lexicon>,
    /// Lines read at the same time.
    pub workers: usize,
}

impl Ocr {
    pub fn read(&mut self, gray: &Gray, o: &Options) -> Result<Page> {
        let clock = std::time::Instant::now();
        let mut timings = Timings::default();
        let mut lap = {
            let mut last = clock.elapsed().as_secs_f64();
            move || {
                let now = clock.elapsed().as_secs_f64();
                let d = now - last;
                last = now;
                d
            }
        };
        let prep = preprocess(gray, o.deskew, o.denoise, o.binarize);
        let to_original = image::invert_affine(&prep.matrix);
        timings.preprocess = lap();

        let mut crops = Vec::new();
        let mut geometry = Vec::new(); // (crop -> original, quad in the straightened page)
        for d in self.detector.detect(&prep.image)? {
            let rect = quad_to_rect(&d.quad);
            let (crop, m) = crop_line(&prep.image, &rect);
            geometry.push((image::compose(&to_original, &m), rect.quad()));
            crops.push(crop);
        }
        timings.detect = lap();
        let decoded = self.recognizer.recognize_batch(&crops, self.workers, 16)?;
        timings.recognize = lap();

        let (model_h, stride) = (self.recognizer.height, self.recognizer.stride);
        let mut lines: Vec<Option<Line>> = Vec::with_capacity(crops.len());
        for ((crop, (to_img, _)), dec) in crops.iter().zip(&geometry).zip(&decoded) {
            let text = postprocess(&dec.text, o);
            if text.trim().is_empty() {
                lines.push(None);
                continue;
            }
            let (cw, ch) = (crop.width, crop.height);
            let pad = PAD_Y * ch as f64 / (1.0 + 2.0 * PAD_Y);
            let words = split_words(dec, cw, ch, model_h, stride)
                .into_iter()
                .map(|(t, c, x0, x1)| Word {
                    text: postprocess(&t, o),
                    confidence: c,
                    corners: map_rect(to_img, x0, x1, pad, ch as f64 - pad),
                })
                .collect();
            let corners = map_rect(to_img, 0.0, cw as f64, pad, ch as f64 - pad);
            lines.push(Some(Line { text, confidence: dec.confidence, corners, words }));
        }

        if o.correct_words
            && let Some(lex) = &self.lexicon
        {
            for ln in lines.iter_mut().flatten() {
                let mut tokens: Vec<String> = ln.text.split(' ').map(str::to_string).collect();
                for w in &mut ln.words {
                    let fixed = lex.correct(&w.text, w.confidence);
                    if fixed != w.text
                        && let Some(k) = tokens.iter().position(|t| *t == w.text)
                    {
                        tokens[k] = fixed.clone();
                        w.text = fixed;
                    }
                }
                ln.text = tokens.join(" ");
            }
        }
        if o.drop_noise {
            for ln in &mut lines {
                if ln.as_ref().is_some_and(is_noise) {
                    *ln = None;
                }
            }
        }
        let kept: Vec<(usize, Line)> = lines.into_iter().enumerate().filter_map(|(i, l)| l.map(|l| (i, l))).collect();
        let aligned: Vec<layout::Box4> = kept
            .iter()
            .map(|(i, _)| {
                let q = &geometry[*i].1;
                let xs = q.map(|p| p[0]);
                let ys = q.map(|p| p[1]);
                let min = |v: [f32; 4]| v.into_iter().fold(f32::INFINITY, f32::min) as f64;
                let max = |v: [f32; 4]| v.into_iter().fold(f32::NEG_INFINITY, f32::max) as f64;
                [min(xs), min(ys), max(xs), max(ys)]
            })
            .collect();
        let mut slots: Vec<Option<Line>> = kept.into_iter().map(|(_, l)| Some(l)).collect();
        let blocks = layout::reading_order(&aligned)
            .into_iter()
            .map(|blk| Block { lines: blk.into_iter().filter_map(|j| slots[j].take()).collect() })
            .collect();
        timings.layout = lap();
        Ok(Page { width: gray.width, height: gray.height, angle: prep.angle, blocks, timings })
    }
}
