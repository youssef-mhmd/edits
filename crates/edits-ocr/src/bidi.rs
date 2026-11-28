//! Which characters of a recognized line are left-to-right (numbers), and the
//! conversion between the recognizer's right-to-left reading order and
//! logical order. A port of arabic-ocr's bidi.py: UAX #9 reduced to what the
//! charset can hold (no Latin letters, no direction controls).

use crate::charset::{self, Bidi};

fn bidi_of(c: char) -> Bidi {
    charset::lookup(c).map(|s| s.bidi).unwrap_or(Bidi::ON)
}

/// For each character, true if it is drawn left to right (part of a number).
pub fn ltr_mask(chars: &[char]) -> Vec<bool> {
    #[derive(Clone, Copy, PartialEq)]
    #[allow(clippy::enum_variant_names)]
    enum T {
        R,
        AL,
        EN,
        AN,
        ES,
        CS,
        ET,
        Other,
    }
    let mut t: Vec<T> = chars
        .iter()
        .map(|&c| match bidi_of(c) {
            Bidi::AL => T::AL,
            Bidi::EN => T::EN,
            Bidi::AN => T::AN,
            Bidi::ES => T::ES,
            Bidi::CS => T::CS,
            Bidi::ET => T::ET,
            Bidi::NSM => T::Other, // resolved below
            _ => T::Other,
        })
        .collect();
    let nsm: Vec<bool> = chars.iter().map(|&c| bidi_of(c) == Bidi::NSM).collect();
    let n = t.len();
    // W1: a mark takes the type of the character before it.
    for i in 0..n {
        if nsm[i] {
            t[i] = if i > 0 { t[i - 1] } else { T::R };
        }
    }
    // W2: European numbers after Arabic letters become Arabic numbers. W3: AL -> R.
    let mut last_strong = T::R;
    for v in t.iter_mut() {
        match *v {
            T::R | T::AL => last_strong = *v,
            T::EN if last_strong == T::AL => *v = T::AN,
            _ => {}
        }
    }
    for v in t.iter_mut() {
        if *v == T::AL {
            *v = T::R;
        }
    }
    // W4: one separator between two numbers of the same kind joins them.
    for i in 1..n.saturating_sub(1) {
        let (l, m, r) = (t[i - 1], t[i], t[i + 1]);
        if m == T::ES && l == T::EN && r == T::EN {
            t[i] = T::EN;
        } else if m == T::CS && l == r && (l == T::EN || l == T::AN) {
            t[i] = l;
        }
    }
    // W5: % signs next to a European number become part of it.
    let mut i = 0;
    while i < n {
        if t[i] == T::ET {
            let mut j = i;
            while j < n && t[j] == T::ET {
                j += 1;
            }
            if (i > 0 && t[i - 1] == T::EN) || (j < n && t[j] == T::EN) {
                t[i..j].iter_mut().for_each(|v| *v = T::EN);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    t.iter().map(|&v| v == T::EN || v == T::AN).collect()
}

/// The characters as met scanning right to left: logical order with every
/// left-to-right number reversed. Its own inverse for real text, so it also
/// turns the recognizer's output into logical order.
pub fn reading_order(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mask = ltr_mask(&chars);
    let mut out = String::with_capacity(text.len());
    let mut run: Vec<char> = Vec::new();
    for (&c, &ltr) in chars.iter().zip(&mask) {
        if ltr {
            run.push(c);
        } else {
            out.extend(run.drain(..).rev());
            out.push(c);
        }
    }
    out.extend(run.drain(..).rev());
    out
}

pub use reading_order as logical_order;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_reversed() {
        let sana: String = ['\u{0633}', '\u{0646}', '\u{0629}'].iter().collect();
        assert_eq!(reading_order(&format!("{sana} 2026")), format!("{sana} 6202"));
        assert_eq!(reading_order(&reading_order(&format!("{sana} 12.5%"))), format!("{sana} 12.5%"));
    }
}
