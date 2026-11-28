//! Line boxes from the detector's core map (arabic-ocr detect/boxes.py and
//! detect/detector.py).
//!
//! The detector paints, for every pixel, how sure it is that the pixel lies in
//! the thin "core" of a text line. Each connected region of core pixels is one
//! line; its smallest rotated rectangle is grown back to the full line.

use crate::image::{self, Gray, py_round};

/// Height of a line's core as a fraction of the line.
pub const KERNEL_FRACTION: f64 = 0.4;
/// Default working size (longer side) of the detector.
pub const MAX_SIDE: usize = 1536;

/// A rotated rectangle: centre, width along the text, height, angle in
/// degrees (0 = horizontal, positive = clockwise in image coordinates).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RotRect {
    pub cx: f64,
    pub cy: f64,
    pub w: f64,
    pub h: f64,
    pub angle: f64,
}

/// Corners as float32, the way numpy stores them.
pub type Quad = [[f32; 2]; 4];

impl RotRect {
    /// Corners: top-right, top-left, bottom-left, bottom-right.
    pub fn quad(&self) -> Quad {
        let a = self.angle.to_radians();
        let (ux, uy) = (a.cos() * self.w / 2.0, a.sin() * self.w / 2.0);
        let (vx, vy) = (-a.sin() * self.h / 2.0, a.cos() * self.h / 2.0);
        let (cx, cy) = (self.cx, self.cy);
        [
            [(cx + ux - vx) as f32, (cy + uy - vy) as f32],
            [(cx - ux - vx) as f32, (cy - uy - vy) as f32],
            [(cx - ux + vx) as f32, (cy - uy + vy) as f32],
            [(cx + ux + vx) as f32, (cy + uy + vy) as f32],
        ]
    }
}

/// The rotated rectangle described by a (TR, TL, BL, BR) quad.
pub fn quad_to_rect(q: &Quad) -> RotRect {
    let p = q.map(|[x, y]| (x as f64, y as f64));
    let (tr, tl, bl, br) = (p[0], p[1], p[2], p[3]);
    let dist = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).hypot(a.1 - b.1);
    let w = (dist(tr, tl) + dist(br, bl)) / 2.0;
    let h = (dist(bl, tl) + dist(br, tr)) / 2.0;
    let cx = (tr.0 + tl.0 + bl.0 + br.0) / 4.0;
    let cy = (tr.1 + tl.1 + bl.1 + br.1) / 4.0;
    let angle = (tr.1 - tl.1).atan2(tr.0 - tl.0).to_degrees();
    RotRect { cx, cy, w, h, angle }
}

/// Grow a detected core back to the full line box.
pub fn core_to_box(r: RotRect) -> RotRect {
    let h = r.h / KERNEL_FRACTION;
    let d = (h - r.h) / 2.0;
    RotRect { w: r.w + 2.0 * d, h, ..r }
}

#[derive(Clone, Debug)]
pub struct Detection {
    /// TR, TL, BL, BR in the coordinates of the image given to the detector.
    pub quad: Quad,
    /// Mean core probability inside the region.
    pub score: f64,
}

/// A page ready for the detector: grayscale, shrunk so the longer side is at
/// most `max_side`, padded with the border colour to multiples of 32.
pub struct PreparedPage {
    pub image: Gray,
    /// prepared = original * scale.
    pub scale: f64,
    pub width: usize,
    pub height: usize,
}

pub fn prepare_page(gray: &Gray, max_side: usize) -> PreparedPage {
    let (w, h) = (gray.width, gray.height);
    let scale = (max_side as f64 / w.max(h) as f64).min(1.0);
    let small = if scale < 1.0 {
        let nw = py_round(w as f64 * scale).max(1) as usize;
        let nh = py_round(h as f64 * scale).max(1) as usize;
        image::resize_area(gray, nw, nh)
    } else {
        gray.clone()
    };
    let (w2, h2) = (small.width, small.height);
    let (pw, ph) = (w2.div_ceil(32) * 32, h2.div_ceil(32) * 32);
    let border = image::border_median(&small);
    PreparedPage { image: image::pad_bottom_right(&small, pw, ph, border), scale, width: w2, height: h2 }
}

/// Core map (row-major, `width` x `height` of the prepared page's valid area,
/// `stride` floats per row) -> line boxes in original image coordinates.
pub fn boxes_from_prob(prob: &[f32], stride: usize, page: &PreparedPage) -> Vec<Detection> {
    let mut dets = map_to_boxes(prob, stride, page.width, page.height, 0.3, 0.5, 4.0);
    let s = page.scale as f32;
    for d in &mut dets {
        for p in &mut d.quad {
            p[0] /= s;
            p[1] /= s;
        }
    }
    dets
}

/// One 8-connected region of core pixels: its pixels' rows, with the leftmost
/// and rightmost pixel of each row (enough for the convex hull), and its
/// bounding box.
struct Region {
    rows: Vec<(usize, usize, usize)>, // (y, x_min, x_max)
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    pixels: Vec<usize>,
}

pub fn map_to_boxes(
    prob: &[f32],
    stride: usize,
    width: usize,
    height: usize,
    threshold: f32,
    min_score: f64,
    min_height: f64,
) -> Vec<Detection> {
    let at = |x: usize, y: usize| prob[y * stride + x];
    let mut mask = vec![false; width * height];
    for y in 0..height {
        for x in 0..width {
            mask[y * width + x] = at(x, y) > threshold;
        }
    }
    let regions = regions(&mask, width, height);
    let mut out = Vec::new();
    // OpenCV returns external contours last found first.
    for r in regions.iter().rev() {
        if is_degenerate(r) {
            continue; // fewer than 3 contour points (a dot or a straight line)
        }
        let score = region_score(r, width, &at);
        if score < min_score {
            continue;
        }
        let rect = core_to_box(horizontal_rect(r));
        if rect.h < min_height {
            continue;
        }
        out.push(Detection { quad: rect.quad(), score });
    }
    out
}

fn regions(mask: &[bool], w: usize, h: usize) -> Vec<Region> {
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !mask[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let mut pixels = Vec::new();
        while let Some(p) = stack.pop() {
            pixels.push(p);
            let (x, y) = ((p % w) as i64, (p / w) as i64);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (xx, yy) = (x + dx, y + dy);
                    if xx < 0 || yy < 0 || xx >= w as i64 || yy >= h as i64 {
                        continue;
                    }
                    let q = yy as usize * w + xx as usize;
                    if mask[q] && !seen[q] {
                        seen[q] = true;
                        stack.push(q);
                    }
                }
            }
        }
        pixels.sort_unstable();
        let mut rows: Vec<(usize, usize, usize)> = Vec::new();
        for &p in &pixels {
            let (x, y) = (p % w, p / w);
            match rows.last_mut() {
                Some(r) if r.0 == y => {
                    r.1 = r.1.min(x);
                    r.2 = r.2.max(x);
                }
                _ => rows.push((y, x, x)),
            }
        }
        let x0 = rows.iter().map(|r| r.1).min().unwrap_or(0);
        let x1 = rows.iter().map(|r| r.2).max().unwrap_or(0);
        let (y0, y1) = (rows[0].0, rows[rows.len() - 1].0);
        out.push(Region { rows, x0, y0, x1, y1, pixels });
    }
    out
}

/// True when OpenCV's simplified contour has fewer than 3 points: a single
/// pixel, or pixels on one horizontal, vertical or 45-degree line.
fn is_degenerate(r: &Region) -> bool {
    if r.y0 == r.y1 || r.x0 == r.x1 {
        return true;
    }
    if r.rows.iter().any(|&(_, a, b)| a != b) || r.rows.len() != r.y1 - r.y0 + 1 {
        return false;
    }
    let step = r.rows[1].1 as i64 - r.rows[0].1 as i64;
    step.abs() == 1 && r.rows.windows(2).all(|p| p[1].1 as i64 - p[0].1 as i64 == step)
}

/// Mean probability over the region filled in (holes included), as
/// cv2.drawContours(..., FILLED) of its outer contour.
fn region_score(r: &Region, w: usize, at: &impl Fn(usize, usize) -> f32) -> f64 {
    let (bw, bh) = (r.x1 - r.x0 + 1, r.y1 - r.y0 + 1);
    let mut mine = vec![false; bw * bh];
    for &p in &r.pixels {
        mine[(p / w - r.y0) * bw + (p % w - r.x0)] = true;
    }
    // Outside = reachable from the box edge without crossing this region
    // (4-connected, the dual of the region's 8-connectivity).
    let mut outside = vec![false; bw * bh];
    let mut stack = Vec::new();
    for x in 0..bw {
        for y in [0, bh - 1] {
            stack.push(y * bw + x);
        }
    }
    for y in 0..bh {
        for x in [0, bw - 1] {
            stack.push(y * bw + x);
        }
    }
    while let Some(p) = stack.pop() {
        if mine[p] || outside[p] {
            continue;
        }
        outside[p] = true;
        let (x, y) = (p % bw, p / bw);
        if x > 0 {
            stack.push(p - 1);
        }
        if x + 1 < bw {
            stack.push(p + 1);
        }
        if y > 0 {
            stack.push(p - bw);
        }
        if y + 1 < bh {
            stack.push(p + bw);
        }
    }
    let (mut sum, mut n) = (0f64, 0usize);
    for y in 0..bh {
        for x in 0..bw {
            if !outside[y * bw + x] {
                sum += at(r.x0 + x, r.y0 + y) as f64;
                n += 1;
            }
        }
    }
    if n == 0 { 0.0 } else { sum / n as f64 }
}

/// cv2.minAreaRect of the region's contour, expressed with the side nearest
/// horizontal as the width (boxes._horizontal_rect).
fn horizontal_rect(r: &Region) -> RotRect {
    let hull = opencv_hull(r);
    let [cx, cy, w, h, angle] = min_area_rect(&hull);
    let (mut w, mut h, mut angle) = (w as f64, h as f64, angle as f64);
    while angle > 45.0 {
        angle -= 90.0;
        std::mem::swap(&mut w, &mut h);
    }
    while angle < -45.0 {
        angle += 90.0;
        std::mem::swap(&mut w, &mut h);
    }
    RotRect { cx: cx as f64, cy: cy as f64, w, h, angle }
}

/// The convex hull as cv2.convexHull(contour, clockwise=False) returns it:
/// clockwise on screen (y down), starting at the vertex after the contour's
/// first point (the region's top-left pixel) and ending at that point.
fn opencv_hull(r: &Region) -> Vec<[f32; 2]> {
    let mut pts: Vec<(i64, i64)> = Vec::with_capacity(r.rows.len() * 2);
    for &(y, a, b) in &r.rows {
        pts.push((a as i64, y as i64));
        if b != a {
            pts.push((b as i64, y as i64));
        }
    }
    let mut hull = convex_hull(pts);
    // Shoelace in y-down coordinates: positive = clockwise on screen.
    let area2: i64 = (0..hull.len())
        .map(|i| {
            let (p, q) = (hull[i], hull[(i + 1) % hull.len()]);
            p.0 * q.1 - q.0 * p.1
        })
        .sum();
    if area2 < 0 {
        hull.reverse();
    }
    let start = (r.rows[0].1 as i64, r.rows[0].0 as i64);
    if let Some(k) = hull.iter().position(|&p| p == start) {
        let n = hull.len();
        hull.rotate_left((k + 1) % n);
    }
    hull.into_iter().map(|(x, y)| [x as f32, y as f32]).collect()
}

/// Andrew's monotone chain on integer points; collinear points dropped.
fn convex_hull(mut pts: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    pts.sort_unstable();
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: (i64, i64), a: (i64, i64), b: (i64, i64)| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let mut lower: Vec<(i64, i64)> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<(i64, i64)> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// OpenCV's minAreaRect (rotating calipers, float32): centre x, y, width,
/// height, angle in degrees in [-90, 0).
fn min_area_rect(points: &[[f32; 2]]) -> [f32; 5] {
    let n = points.len();
    if n == 0 {
        return [0.0, 0.0, 0.0, 0.0, -90.0];
    }
    if n == 1 {
        return [points[0][0], points[0][1], 0.0, 0.0, -90.0];
    }
    if n == 2 {
        let (p, q) = (points[0], points[1]);
        let (cx, cy) = ((p[0] + q[0]) * 0.5, (p[1] + q[1]) * 0.5);
        let (dx, dy) = ((p[0] - q[0]) as f64, (p[1] - q[1]) as f64);
        let len = (dx * dx + dy * dy).sqrt() as f32;
        let (mut w, mut h, mut angle) = (0.0f32, len, -std::f64::consts::FRAC_PI_2);
        if dx == 0.0 {
            std::mem::swap(&mut w, &mut h);
        } else if dy < 0.0 {
            angle = dy.atan2(dx);
            std::mem::swap(&mut w, &mut h);
        } else if dy > 0.0 {
            angle = -dx.atan2(dy);
        }
        return [cx, cy, w, h, (angle * 180.0 / std::f64::consts::PI) as f32];
    }
    let out = rotating_calipers(points);
    let cx = out[0] + (out[2] + out[4]) * 0.5;
    let cy = out[1] + (out[3] + out[5]) * 0.5;
    let mut w = ((out[4] as f64).powi(2) + (out[5] as f64).powi(2)).sqrt() as f32;
    let mut h = ((out[2] as f64).powi(2) + (out[3] as f64).powi(2)).sqrt() as f32;
    let mut angle = -std::f64::consts::FRAC_PI_2;
    if out[2] == 0.0 && out[3] > 0.0 {
        std::mem::swap(&mut w, &mut h);
    } else {
        angle = -(out[2] as f64).atan2(out[3] as f64);
    }
    [cx, cy, w, h, (angle * 180.0 / std::f64::consts::PI) as f32]
}

/// rotatingCalipers(..., CALIPERS_MINAREARECT) from OpenCV's rotcalipers.cpp,
/// for a hull in OpenCV's order (orientation 1). Returns the corner point and
/// the two side vectors: [px, py, side1x, side1y, side2x, side2y].
fn rotating_calipers(points: &[[f32; 2]]) -> [f32; 6] {
    let n = points.len();
    let mut vect = vec![[0f32; 2]; n];
    let mut inv_len = vec![0f32; n];
    let (mut left, mut bottom, mut right, mut top) = (0usize, 0usize, 0usize, 0usize);
    let mut pt0 = points[0];
    let (mut left_x, mut right_x, mut top_y, mut bottom_y) = (pt0[0], pt0[0], pt0[1], pt0[1]);
    for i in 0..n {
        if pt0[0] < left_x {
            left_x = pt0[0];
            left = i;
        }
        if pt0[0] > right_x {
            right_x = pt0[0];
            right = i;
        }
        if pt0[1] > top_y {
            top_y = pt0[1];
            top = i;
        }
        if pt0[1] < bottom_y {
            bottom_y = pt0[1];
            bottom = i;
        }
        let pt = points[if i + 1 < n { i + 1 } else { 0 }];
        let (dx, dy) = ((pt[0] - pt0[0]) as f64, (pt[1] - pt0[1]) as f64);
        vect[i] = [dx as f32, dy as f32];
        inv_len[i] = (1.0 / (dx * dx + dy * dy).sqrt()) as f32;
        pt0 = pt;
    }
    let (mut base_a, mut base_b);
    let mut seq = [bottom, right, top, left];
    let mut minarea = f32::MAX;
    let mut best_left = 0usize;
    let mut best_bottom = 0usize;
    let mut buf = [0f32; 4]; // base_a, width, base_b, height
    let rot90cw = |v: [f32; 2]| [v[1], -v[0]];
    let rot90ccw = |v: [f32; 2]| [-v[1], v[0]];
    let rot180 = |v: [f32; 2]| [-v[0], -v[1]];
    let first_is_right = |a: [f32; 2], b: [f32; 2]| {
        let t = rot90cw(a);
        t[0] * b[0] + t[1] * b[1] < 0.0
    };
    for _ in 0..n {
        let rot = [vect[seq[0]], rot90cw(vect[seq[1]]), rot180(vect[seq[2]]), rot90ccw(vect[seq[3]])];
        let mut main = 0;
        for i in 1..4 {
            if first_is_right(rot[i], rot[main]) {
                main = i;
            }
        }
        let p = seq[main];
        let (lx, ly) = (vect[p][0] * inv_len[p], vect[p][1] * inv_len[p]);
        (base_a, base_b) = match main {
            0 => (lx, ly),
            1 => (ly, -lx),
            2 => (-lx, -ly),
            _ => (-ly, lx),
        };
        seq[main] = if seq[main] + 1 == n { 0 } else { seq[main] + 1 };
        let dx = points[seq[1]][0] - points[seq[3]][0];
        let dy = points[seq[1]][1] - points[seq[3]][1];
        let width = dx * base_a + dy * base_b;
        let dx = points[seq[2]][0] - points[seq[0]][0];
        let dy = points[seq[2]][1] - points[seq[0]][1];
        let height = -dx * base_b + dy * base_a;
        let area = width * height;
        if area <= minarea {
            minarea = area;
            best_left = seq[3];
            buf = [base_a, width, base_b, height];
            best_bottom = seq[0];
        }
    }
    let (a1, b1) = (buf[0], buf[2]);
    let (a2, b2) = (-buf[2], buf[0]);
    let c1 = a1 * points[best_left][0] + points[best_left][1] * b1;
    let c2 = a2 * points[best_bottom][0] + points[best_bottom][1] * b2;
    let idet = 1.0f32 / (a1 * b2 - a2 * b1);
    let px = (c1 * b2 - c2 * b1) * idet;
    let py = (a1 * c2 - a2 * c1) * idet;
    [px, py, a1 * buf[1], b1 * buf[1], a2 * buf[3], b2 * buf[3]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_aligned_block() {
        // A 10 x 3 block of core pixels: rect 9 x 2 between pixel centres.
        let (w, h) = (20, 8);
        let mut prob = vec![0f32; w * h];
        for y in 2..5 {
            for x in 4..14 {
                prob[y * w + x] = 0.9;
            }
        }
        let d = map_to_boxes(&prob, w, w, h, 0.3, 0.5, 1.0);
        assert_eq!(d.len(), 1);
        let r = quad_to_rect(&d[0].quad);
        assert!((r.cx - 8.5).abs() < 1e-4 && (r.cy - 3.0).abs() < 1e-4, "{r:?}");
        assert!((r.h - 5.0).abs() < 1e-4 && (r.w - 12.0).abs() < 1e-4, "{r:?}");
        assert!(r.angle.abs() < 1e-6);
    }
}
