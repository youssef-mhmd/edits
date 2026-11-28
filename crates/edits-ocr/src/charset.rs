//! The recognizer's symbols (generated from arabic-ocr's charset.py) and the
//! text options applied after recognition.

#[allow(clippy::upper_case_acronyms)]
#[rustfmt::skip]
mod data {
    include!("charset_data.rs");
}

use data::{ARABIC_INDIC_DIGITS, DIACRITICS, EASTERN_DIGITS, TATWEEL, TWIN_GROUPS};
pub use data::{Bidi, FINGERPRINT, NUM_CLASSES, SYMBOLS, Symbol};

pub const BLANK_ID: usize = 0;
/// Id of the space symbol (the first symbol).
pub const SPACE_ID: usize = 1;

/// The symbol for an id (None for the blank or an id out of range).
pub fn symbol(id: usize) -> Option<&'static Symbol> {
    id.checked_sub(1).and_then(|i| SYMBOLS.get(i))
}

/// The symbol for a character, if it is in the charset.
pub fn lookup(ch: char) -> Option<&'static Symbol> {
    SYMBOLS.iter().find(|s| s.ch == ch)
}

/// Text of symbol ids (blanks and unknown ids skipped).
pub fn decode(ids: &[usize]) -> String {
    ids.iter().filter_map(|&i| symbol(i)).map(|s| s.ch).collect()
}

/// Remove tashkeel; the combining hamza marks are kept (part of the letter).
pub fn strip_diacritics(text: &str) -> String {
    const HAMZA: [char; 2] = ['\u{0654}', '\u{0655}'];
    text.chars().filter(|c| !DIACRITICS.contains(c) || HAMZA.contains(c)).collect()
}

pub fn strip_tatweel(text: &str) -> String {
    text.chars().filter(|&c| c != TATWEEL).collect()
}

/// How digits come out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Digits {
    /// As printed.
    #[default]
    Keep,
    /// 0-9.
    Western,
    /// Arabic-Indic (U+0660..U+0669).
    Arabic,
}

pub fn convert_digits(text: &str, style: Digits) -> String {
    let digit = |c: char| -> Option<usize> {
        if c.is_ascii_digit() {
            return Some(c as usize - '0' as usize);
        }
        ARABIC_INDIC_DIGITS.iter().position(|&d| d == c).or_else(|| EASTERN_DIGITS.iter().position(|&d| d == c))
    };
    match style {
        Digits::Keep => text.to_string(),
        Digits::Western => text
            .chars()
            .map(|c| match digit(c) {
                Some(v) if !c.is_ascii_digit() => char::from(b'0' + v as u8),
                _ => c,
            })
            .collect(),
        Digits::Arabic => text
            .chars()
            .map(|c| match digit(c) {
                Some(v) if !ARABIC_INDIC_DIGITS.contains(&c) => ARABIC_INDIC_DIGITS[v],
                _ => c,
            })
            .collect(),
    }
}

/// Letters of the same shape as `c` (told apart only by dots).
pub fn twins(c: char) -> impl Iterator<Item = char> {
    TWIN_GROUPS
        .iter()
        .find(|g| g.contains(&c))
        .into_iter()
        .flat_map(move |g| g.iter().copied().filter(move |&t| t != c))
}

pub fn has_twins(c: char) -> bool {
    TWIN_GROUPS.iter().any(|g| g.contains(&c))
}

/// Unicode general category class (first letter) of a charset character;
/// b'?' for characters outside the charset.
pub fn category(c: char) -> u8 {
    lookup(c).map(|s| s.category).unwrap_or(b'?')
}

/// Python's str.isalpha() for charset characters.
pub fn is_alpha(c: char) -> bool {
    lookup(c).map(|s| s.alpha).unwrap_or_else(|| c.is_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_shape() {
        assert_eq!(SYMBOLS.len() + 1, NUM_CLASSES);
        assert_eq!(symbol(SPACE_ID).map(|s| s.ch), Some(' '));
        assert!(symbol(BLANK_ID).is_none());
    }

    #[test]
    fn digits() {
        let ar: String = ['\u{0661}', '\u{0662}'].iter().collect();
        let fa: String = ['\u{06F3}'].iter().collect();
        assert_eq!(convert_digits(&format!("{ar}{fa}7"), Digits::Western), "1237");
        let want: String = ['\u{0661}', '\u{0662}', '\u{0663}', '\u{0667}'].iter().collect();
        assert_eq!(convert_digits(&format!("{ar}{fa}7"), Digits::Arabic), want);
    }

    #[test]
    fn diacritics_keep_hamza() {
        let text: String = ['\u{0628}', '\u{064E}', '\u{0654}'].iter().collect();
        let want: String = ['\u{0628}', '\u{0654}'].iter().collect();
        assert_eq!(strip_diacritics(&text), want);
    }
}
