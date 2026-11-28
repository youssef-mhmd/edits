//! Reading order (arabic-ocr layout.py): right to left, top to bottom; columns
//! right to left; lines grouped into blocks (paragraphs).
//!
//! Works on axis-aligned boxes (x0, y0, x1, y1) of the deskewed page. Boxes
//! come from float32 quads, as in Python, and all arithmetic is in f64.

pub type Box4 = [f64; 4];

const MAX_CROSSING: f64 = 0.15;
const MIN_GUTTER: f64 = 0.5;
const MIN_BESIDE: f64 = 0.3;
const SHORT_LINE: f64 = 2.0;
const INDENT: f64 = 1.0;
const VERSE_MAX_OFFSET: f64 = 0.07;
const VERSE_MAX_WIDTH: f64 = 10.0;

fn mid(b: &Box4) -> f64 {
    (b[1] + b[3]) / 2.0
}

/// numpy.median.
fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

/// Top to bottom; lines sharing a row right to left.
pub fn row_order(boxes: &[Box4], idx: &[usize]) -> Vec<usize> {
    let mut idx = idx.to_vec();
    idx.sort_by(|&a, &b| mid(&boxes[a]).total_cmp(&mid(&boxes[b]))); // stable, as sorted()
    let mut rows: Vec<Vec<usize>> = Vec::new();
    for i in idx {
        let (y0, y1) = (boxes[i][1], boxes[i][3]);
        if let Some(last) = rows.last_mut() {
            let ly0 = last.iter().map(|&j| boxes[j][1]).fold(f64::INFINITY, f64::min);
            let ly1 = last.iter().map(|&j| boxes[j][3]).fold(f64::NEG_INFINITY, f64::max);
            if y1.min(ly1) - y0.max(ly0) > 0.5 * (y1 - y0).min(ly1 - ly0) {
                last.push(i);
                continue;
            }
        }
        rows.push(vec![i]);
    }
    rows.into_iter()
        .flat_map(|mut row| {
            row.sort_by(|&a, &b| (-boxes[a][2]).total_cmp(&-boxes[b][2]));
            row
        })
        .collect()
}

fn split_blocks(boxes: &[Box4], ordered: &[usize], h: f64) -> Vec<Vec<usize>> {
    if ordered.is_empty() {
        return Vec::new();
    }
    let left = ordered.iter().map(|&i| boxes[i][0]).fold(f64::INFINITY, f64::min);
    let right = ordered.iter().map(|&i| boxes[i][2]).fold(f64::NEG_INFINITY, f64::max);
    let short = (SHORT_LINE * h).max(0.15 * (right - left));
    let full = ordered.iter().filter(|&&i| boxes[i][0] - left < 1.0 * h).count();
    let justified = ordered.len() >= 3 && full as f64 >= 0.5 * ordered.len() as f64;
    let mut blocks: Vec<Vec<usize>> = Vec::new();
    for &i in ordered {
        let Some(last) = blocks.last_mut() else {
            blocks.push(vec![i]);
            continue;
        };
        let p = *last.last().unwrap_or(&i);
        let below = boxes[i][1] >= boxes[p][3] - 0.3 * h;
        let gap = boxes[i][1] - boxes[p][3] >= 1.0 * h;
        let ends_short = justified && below && boxes[p][0] - left > short;
        let indented = below && right - boxes[i][2] > INDENT * h && right - boxes[p][2] < 0.5 * h;
        if gap || ends_short || indented {
            blocks.push(vec![i]);
        } else {
            last.push(i);
        }
    }
    blocks
}

fn find_gutter(boxes: &[Box4], idx: &[usize], h: f64) -> Option<f64> {
    let idx: Vec<usize> = idx.iter().copied().filter(|&i| boxes[i][2] - boxes[i][0] >= 2.0 * h).collect();
    if idx.len() < 2 {
        return None;
    }
    let x0: Vec<f64> = idx.iter().map(|&i| boxes[i][0]).collect();
    let x1: Vec<f64> = idx.iter().map(|&i| boxes[i][2]).collect();
    let lo = x0.iter().copied().fold(f64::INFINITY, f64::min).floor() as i64;
    let hi = x1.iter().copied().fold(f64::NEG_INFINITY, f64::max).ceil() as i64;
    if hi - lo < 3 {
        return None;
    }
    let n = (hi - lo) as usize;
    let mut diff = vec![0i64; n + 2];
    for (&a, &b) in x0.iter().zip(&x1) {
        diff[(a.floor() as i64 - lo) as usize] += 1;
        diff[(b.ceil() as i64 - lo) as usize] -= 1;
    }
    let mut crossing = Vec::with_capacity(n);
    let mut acc = 0;
    for d in diff.iter().take(n) {
        acc += d;
        crossing.push(acc);
    }
    // Only positions with text entirely on both sides can be gutters.
    let mut inside = vec![false; n];
    let max_x0 = x0.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let min_x1 = x1.iter().copied().fold(f64::INFINITY, f64::min);
    for (k, v) in inside.iter_mut().enumerate() {
        let x = (lo + k as i64) as f64;
        *v = max_x0 >= x && min_x1 <= x;
    }
    let best = inside.iter().zip(&crossing).filter(|(v, _)| **v).map(|(_, &c)| c).min()?;
    if best as f64 > MAX_CROSSING * idx.len() as f64 {
        return None;
    }
    // Widest run of positions at that lowest count (first widest wins).
    let ok: Vec<bool> = inside.iter().zip(&crossing).map(|(&v, &c)| v && c == best).collect();
    let (mut run, mut start) = ((0usize, 0usize), None);
    for (k, &v) in ok.iter().chain([&false]).enumerate() {
        match (v, start) {
            (true, None) => start = Some(k),
            (false, Some(s)) => {
                if k - s > run.1 - run.0 {
                    run = (s, k);
                }
                start = None;
            }
            _ => {}
        }
    }
    let (a, b) = run;
    if ((b - a) as f64) < MIN_GUTTER * h {
        return None;
    }
    let x = lo as f64 + (a + b) as f64 / 2.0;
    let right: Vec<usize> = idx.iter().copied().filter(|&i| boxes[i][0] >= x).collect();
    let left: Vec<usize> = idx.iter().copied().filter(|&i| boxes[i][2] <= x).collect();
    let (small, big) = if left.len() < right.len() { (&left, &right) } else { (&right, &left) };
    let beside = small
        .iter()
        .filter(|&&i| big.iter().any(|&j| boxes[i][3].min(boxes[j][3]) > boxes[i][1].max(boxes[j][1])))
        .count();
    if (beside as f64) < MIN_BESIDE * small.len() as f64 {
        return None;
    }
    if is_verse(boxes, &right, &left) {
        return None;
    }
    Some(x)
}

/// Two "columns" that are really the half-lines of verse.
fn is_verse(boxes: &[Box4], right: &[usize], left: &[usize]) -> bool {
    let (small, big) = if left.len() < right.len() { (left, right) } else { (right, left) };
    if small.len() < 4 || big.len() as f64 > 1.2 * small.len() as f64 {
        return false;
    }
    let partnered = |i: usize| {
        let (y0, y1) = (boxes[i][1], boxes[i][3]);
        big.iter().any(|&j| y1.min(boxes[j][3]) - y0.max(boxes[j][1]) > 0.6 * (y1 - y0).min(boxes[j][3] - boxes[j][1]))
    };
    if (small.iter().filter(|&&i| partnered(i)).count() as f64) < 0.9 * small.len() as f64 {
        return false;
    }
    let all: Vec<usize> = right.iter().chain(left).copied().collect();
    let h = median(all.iter().map(|&i| boxes[i][3] - boxes[i][1]).collect());
    let offset = median(
        small
            .iter()
            .map(|&i| big.iter().map(|&j| (mid(&boxes[i]) - mid(&boxes[j])).abs()).fold(f64::INFINITY, f64::min))
            .collect(),
    ) / h;
    let width = median(all.iter().map(|&i| boxes[i][2] - boxes[i][0]).collect()) / h;
    if offset > VERSE_MAX_OFFSET || width > VERSE_MAX_WIDTH {
        return false;
    }
    for side in [right, left] {
        let widths: Vec<f64> = side.iter().map(|&i| boxes[i][2] - boxes[i][0]).collect();
        let full = widths.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if widths.iter().filter(|&&w| w < 0.7 * full).count() as f64 > 0.1 * side.len() as f64 {
            return false;
        }
    }
    true
}

fn order(boxes: &[Box4], idx: &[usize], h: f64) -> Vec<Vec<usize>> {
    if idx.is_empty() {
        return Vec::new();
    }
    let Some(x) = find_gutter(boxes, idx, h) else {
        return split_blocks(boxes, &row_order(boxes, idx), h);
    };
    let crosses = |i: usize| boxes[i][0] < x && x < boxes[i][2];
    let mut spanning: Vec<usize> = idx.iter().copied().filter(|&i| crosses(i)).collect();
    spanning.sort_by(|&a, &b| mid(&boxes[a]).total_cmp(&mid(&boxes[b])));
    let mut out = Vec::new();
    let mut pending: Vec<usize> = Vec::new();
    let mut top = f64::NEG_INFINITY;
    let cuts: Vec<Option<usize>> = spanning.iter().map(|&c| Some(c)).chain([None]).collect();
    for cut in cuts {
        let bottom = cut.map(|c| mid(&boxes[c])).unwrap_or(f64::INFINITY);
        let section: Vec<usize> =
            idx.iter().copied().filter(|&i| !crosses(i) && top <= mid(&boxes[i]) && mid(&boxes[i]) < bottom).collect();
        if !section.is_empty() {
            if !pending.is_empty() {
                out.extend(split_blocks(boxes, &pending, h));
                pending.clear();
            }
            let right: Vec<usize> = section.iter().copied().filter(|&i| boxes[i][0] >= x).collect();
            let left: Vec<usize> = section.iter().copied().filter(|&i| boxes[i][0] < x).collect();
            out.extend(order(boxes, &right, h));
            out.extend(order(boxes, &left, h));
        }
        if let Some(c) = cut {
            pending.push(c);
            top = bottom;
        }
    }
    if !pending.is_empty() {
        out.extend(split_blocks(boxes, &pending, h));
    }
    out
}

/// Blocks of indices into `boxes`, in reading order; every index once.
pub fn reading_order(boxes: &[Box4]) -> Vec<Vec<usize>> {
    if boxes.is_empty() {
        return Vec::new();
    }
    let h = median(boxes.iter().map(|b| b[3] - b[1]).collect());
    let idx: Vec<usize> = (0..boxes.len()).collect();
    order(boxes, &idx, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_columns_right_first() {
        let mut boxes = Vec::new();
        for r in 0..6 {
            let y = r as f64 * 30.0;
            boxes.push([0.0, y, 400.0, y + 20.0]); // left column
            boxes.push([500.0, y, 900.0, y + 20.0]); // right column
        }
        let order: Vec<usize> = reading_order(&boxes).concat();
        assert_eq!(&order[..6], &[1, 3, 5, 7, 9, 11]);
        assert_eq!(&order[6..], &[0, 2, 4, 6, 8, 10]);
    }
}
