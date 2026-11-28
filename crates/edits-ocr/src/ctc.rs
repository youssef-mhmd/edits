//! Greedy CTC decoding of the recognizer's per-slice scores (arabic-ocr ctc.py).

use crate::bidi::logical_order;
use crate::charset;

#[derive(Clone, Debug, Default)]
pub struct Decoded {
    /// Logical Unicode order.
    pub text: String,
    /// 0-1: mean probability of the emitted symbols.
    pub confidence: f64,
    /// Symbol ids in reading order (right to left).
    pub ids: Vec<usize>,
    pub probs: Vec<f64>,
    /// Slice where each id was emitted (slice 0 is the right edge).
    pub frames: Vec<usize>,
}

/// Decode one line: `scores` holds `steps` rows of `classes` log-probabilities;
/// row t of the line is at `scores[t * stride ..][..classes]`.
pub fn greedy_decode(scores: &[f32], steps: usize, stride: usize, classes: usize) -> Decoded {
    let mut d = Decoded::default();
    let mut prev = usize::MAX;
    for t in 0..steps {
        let row = &scores[t * stride..t * stride + classes];
        // argmax: first maximum, as numpy.
        let mut best = 0;
        for (i, &v) in row.iter().enumerate() {
            if v > row[best] {
                best = i;
            }
        }
        if best != prev && best != charset::BLANK_ID {
            d.ids.push(best);
            d.probs.push(row[best].exp() as f64); // np.exp of a float32
            d.frames.push(t);
        }
        prev = best;
    }
    d.text = logical_order(&charset::decode(&d.ids));
    d.confidence = mean(&d.probs);
    d
}

pub fn mean(v: &[f64]) -> f64 {
    // numpy's pairwise summation differs from a plain sum only in the last
    // bits; confidences are compared with a tolerance.
    if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 }
}
