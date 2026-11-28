//! Optional word-list correction of dot confusions (arabic-ocr correct.py).
//!
//! When the recognizer is unsure of a word that is not in the word list, and
//! changing one letter to a letter of the same shape (ب ت ث ن ي ...) gives a
//! common word, that word is probably what was printed.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::charset;
use crate::{Error, Result};

/// Only words read with less confidence than this are candidates.
pub const MAX_CONFIDENCE: f64 = 0.97;
/// A replacement must be at least this frequent in the word list.
pub const MIN_COUNT: u64 = 20;

pub struct Lexicon {
    counts: HashMap<String, u64>,
}

impl Lexicon {
    /// Load lexicon.tsv.gz: one "word<TAB>count" per line.
    pub fn load(path: &Path) -> Result<Lexicon> {
        let file = std::fs::File::open(path).map_err(|e| Error::Model(format!("{}: {e}", path.display())))?;
        let reader = BufReader::new(flate2::read::GzDecoder::new(file));
        let mut counts = HashMap::new();
        for line in reader.lines() {
            let line = line.map_err(|e| Error::Model(format!("{}: {e}", path.display())))?;
            let (word, n) = line.split_once('\t').unwrap_or((&line, ""));
            counts.insert(word.to_string(), n.trim().parse().unwrap_or(0));
        }
        Ok(Lexicon { counts })
    }

    pub fn from_counts(counts: HashMap<String, u64>) -> Lexicon {
        Lexicon { counts }
    }

    fn count(&self, w: &str) -> u64 {
        self.counts.get(w).copied().unwrap_or(0)
    }

    /// The corrected word, or the word unchanged.
    pub fn correct(&self, word: &str, confidence: f64) -> String {
        if confidence >= MAX_CONFIDENCE {
            return word.to_string();
        }
        let chars: Vec<char> = word.chars().collect();
        // Keep punctuation, digits, diacritics: look the bare letters up.
        let mut start = 0;
        while start < chars.len() && !charset::has_twins(chars[start]) && !charset::is_alpha(chars[start]) {
            start += 1;
        }
        let mut end = chars.len();
        while end > start && !charset::is_alpha(chars[end - 1]) {
            end -= 1;
        }
        let core: String = chars[start..end].iter().collect();
        let bare = charset::strip_tatweel(&charset::strip_diacritics(&core));
        if bare.chars().count() < 2 || bare != core || self.count(&bare) >= MIN_COUNT {
            return word.to_string();
        }
        let letters: Vec<char> = bare.chars().collect();
        let mut best = bare.clone();
        let mut best_count = (self.count(&bare) * 10).max(MIN_COUNT - 1);
        for (i, &c) in letters.iter().enumerate() {
            for twin in charset::twins(c) {
                let mut cand = letters.clone();
                cand[i] = twin;
                let cand: String = cand.into_iter().collect();
                let n = self.count(&cand);
                if n > best_count {
                    best = cand;
                    best_count = n;
                }
            }
        }
        let head: String = chars[..start].iter().collect();
        let tail: String = chars[end..].iter().collect();
        format!("{head}{best}{tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_dot_fix() {
        // الكتاث -> الكتاب (ث and ب share a shape).
        let word: String = ['\u{0627}', '\u{0644}', '\u{0643}', '\u{062A}', '\u{0627}', '\u{062B}'].iter().collect();
        let fixed: String = ['\u{0627}', '\u{0644}', '\u{0643}', '\u{062A}', '\u{0627}', '\u{0628}'].iter().collect();
        let lex = Lexicon::from_counts(HashMap::from([(fixed.clone(), 500)]));
        assert_eq!(lex.correct(&word, 0.6), fixed);
        assert_eq!(lex.correct(&word, 0.99), word);
    }
}
