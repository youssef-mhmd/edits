//! Grayscale images and the OpenCV operations the arabic-ocr pipeline uses.
//!
//! Each function follows OpenCV 5.0's arithmetic (the version arabic-ocr runs),
//! because the parity tests compare against its output byte for byte: the same
//! fixed-point formulas, float32 steps and round-half-to-even. The fixtures in
//! tests/ocr/cv (scripts/ocr-cv-fixtures.py) check every function here.

/// An 8-bit grayscale image, rows top to bottom.
#[derive(Clone, PartialEq, Eq)]
pub struct Gray {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl std::fmt::Debug for Gray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Gray({}x{})", self.width, self.height)
    }
}

impl Gray {
    pub fn new(width: usize, height: usize, value: u8) -> Gray {
        Gray { width, height, data: vec![value; width * height] }
    }

    pub fn from_vec(width: usize, height: usize, data: Vec<u8>) -> Gray {
        assert_eq!(data.len(), width * height);
        Gray { width, height, data }
    }

    #[inline]
    pub fn at(&self, x: usize, y: usize) -> u8 {
        self.data[y * self.width + x]
    }

    pub fn row(&self, y: usize) -> &[u8] {
        &self.data[y * self.width..(y + 1) * self.width]
    }

    /// cv2.cvtColor(BGR2GRAY): fixed point, 15 bits.
    pub fn from_bgr(width: usize, height: usize, bgr: &[u8]) -> Gray {
        let data = bgr.as_chunks::<3>().0.iter().map(|p| luma(p[0], p[1], p[2])).collect();
        Gray::from_vec(width, height, data)
    }

    /// An opaque RGBA render (PDFium's pages) as BGR2GRAY gives it.
    pub fn from_rgba(width: usize, height: usize, rgba: &[u8]) -> Gray {
        let data = rgba.as_chunks::<4>().0.iter().map(|p| luma(p[2], p[1], p[0])).collect();
        Gray::from_vec(width, height, data)
    }

    /// recognizer.to_gray for BGRA: transparent pixels put on white (float32,
    /// truncated), then BGR2GRAY.
    pub fn from_bgra(width: usize, height: usize, bgra: &[u8]) -> Gray {
        let data = bgra
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                let a = p[3] as f32 / 255.0;
                let on_white = |c: u8| (c as f32 * a + 255.0 * (1.0 - a)) as u8;
                luma(on_white(p[0]), on_white(p[1]), on_white(p[2]))
            })
            .collect();
        Gray::from_vec(width, height, data)
    }
}

#[inline]
fn luma(b: u8, g: u8, r: u8) -> u8 {
    // OpenCV 5: 15-bit coefficients (BY15, GY15, RY15).
    ((b as u32 * 3735 + g as u32 * 19235 + r as u32 * 9798 + (1 << 14)) >> 15) as u8
}

/// cvRound / saturate_cast<int>(double): round half to even.
#[inline]
pub fn round_even(v: f64) -> i64 {
    v.round_ties_even() as i64
}

#[inline]
fn sat_u8_f32(v: f32) -> u8 {
    v.round_ties_even().clamp(0.0, 255.0) as u8
}

/// Python's round() of a float to an int (half to even).
#[inline]
pub fn py_round(v: f64) -> i64 {
    v.round_ties_even() as i64
}

/// numpy.median of bytes, as Python's int() of it (truncated toward zero).
pub fn median_int(values: &mut [u8]) -> u8 {
    if values.is_empty() {
        return 0;
    }
    let mut hist = [0usize; 256];
    for &v in values.iter() {
        hist[v as usize] += 1;
    }
    let n = values.len();
    let nth = |k: usize| -> u8 {
        let mut seen = 0;
        for (v, &c) in hist.iter().enumerate() {
            seen += c;
            if seen > k {
                return v as u8;
            }
        }
        255
    };
    if n % 2 == 1 { nth(n / 2) } else { ((nth(n / 2 - 1) as u32 + nth(n / 2) as u32) / 2) as u8 }
}

/// Median of the four borders (top row, bottom row, left and right columns),
/// as np.median(np.concatenate([g[0], g[-1], g[:, 0], g[:, -1]])).
pub fn border_median(g: &Gray) -> u8 {
    let mut v = Vec::with_capacity(2 * (g.width + g.height));
    v.extend_from_slice(g.row(0));
    v.extend_from_slice(g.row(g.height - 1));
    for y in 0..g.height {
        v.push(g.at(0, y));
    }
    for y in 0..g.height {
        v.push(g.at(g.width - 1, y));
    }
    median_int(&mut v)
}

// ---------------------------------------------------------------------------
// Resize
// ---------------------------------------------------------------------------

/// cv2.resize(src, (width, height), interpolation=INTER_AREA).
pub fn resize_area(src: &Gray, width: usize, height: usize) -> Gray {
    let inv_x = width as f64 / src.width as f64;
    let inv_y = height as f64 / src.height as f64;
    resize_area_scaled(src, width, height, inv_x, inv_y)
}

/// cv2.resize(src, None, fx=f, fy=f, interpolation=INTER_AREA): the size is
/// rounded, but the scale used is exactly f.
pub fn resize_area_fx(src: &Gray, f: f64) -> Gray {
    let width = round_even(src.width as f64 * f).max(1) as usize;
    let height = round_even(src.height as f64 * f).max(1) as usize;
    resize_area_scaled(src, width, height, f, f)
}

fn resize_area_scaled(src: &Gray, width: usize, height: usize, inv_x: f64, inv_y: f64) -> Gray {
    let scale_x = 1.0 / inv_x;
    let scale_y = 1.0 / inv_y;
    assert!(scale_x >= 1.0 && scale_y >= 1.0, "INTER_AREA only shrinks here");
    let ix = round_even(scale_x);
    let iy = round_even(scale_y);
    let fast = (scale_x - ix as f64).abs() < f64::EPSILON && (scale_y - iy as f64).abs() < f64::EPSILON;
    if fast {
        return resize_area_fast(src, width, height, ix as usize, iy as usize);
    }
    let xtab = area_tab(src.width, width, scale_x);
    let ytab = area_tab(src.height, height, scale_y);
    let mut out = Gray::new(width, height, 0);
    let mut buf = vec![0f32; width];
    let mut sum = vec![0f32; width];
    let mut prev_dy = ytab[0].0;
    let store = |sum: &[f32], out: &mut Gray, dy: usize| {
        for (o, &s) in out.data[dy * width..(dy + 1) * width].iter_mut().zip(sum) {
            *o = sat_u8_f32(s);
        }
    };
    for &(dy, sy, beta) in &ytab {
        buf.iter_mut().for_each(|b| *b = 0.0);
        let row = src.row(sy);
        for &(dx, sx, alpha) in &xtab {
            buf[dx] += row[sx] as f32 * alpha;
        }
        if dy != prev_dy {
            store(&sum, &mut out, prev_dy);
            for (s, &b) in sum.iter_mut().zip(&buf) {
                *s = beta * b;
            }
            prev_dy = dy;
        } else {
            for (s, &b) in sum.iter_mut().zip(&buf) {
                *s += beta * b;
            }
        }
    }
    store(&sum, &mut out, prev_dy);
    out
}

/// computeResizeAreaTab: (destination index, source index, weight).
fn area_tab(ssize: usize, dsize: usize, scale: f64) -> Vec<(usize, usize, f32)> {
    let mut tab = Vec::with_capacity(ssize * 2);
    for dx in 0..dsize {
        let fsx1 = dx as f64 * scale;
        let fsx2 = fsx1 + scale;
        let cell = scale.min(ssize as f64 - fsx1);
        let mut sx1 = fsx1.ceil() as i64;
        let mut sx2 = fsx2.floor() as i64;
        sx2 = sx2.min(ssize as i64 - 1);
        sx1 = sx1.min(sx2);
        if sx1 as f64 - fsx1 > 1e-3 {
            tab.push((dx, (sx1 - 1) as usize, ((sx1 as f64 - fsx1) / cell) as f32));
        }
        for sx in sx1..sx2 {
            tab.push((dx, sx as usize, (1.0 / cell) as f32));
        }
        if fsx2 - sx2 as f64 > 1e-3 {
            let w = (fsx2 - sx2 as f64).min(1.0).min(cell) / cell;
            tab.push((dx, sx2 as usize, w as f32));
        }
    }
    tab
}

/// resizeAreaFast_ (integer factors).
fn resize_area_fast(src: &Gray, width: usize, height: usize, sx: usize, sy: usize) -> Gray {
    let mut out = Gray::new(width, height, 0);
    let area = sx * sy;
    let scale = 1.0f32 / area as f32;
    let full_w = src.width / sx;
    for dy in 0..height {
        let y0 = dy * sy;
        if y0 >= src.height {
            continue;
        }
        let w = if y0 + sy <= src.height { full_w } else { 0 };
        let mut dx = 0;
        if sx == 2 && sy == 2 && w > 0 {
            let (r0, r1) = (src.row(y0), src.row(y0 + 1));
            while dx < w {
                let i = dx * 2;
                let s = r0[i] as u32 + r0[i + 1] as u32 + r1[i] as u32 + r1[i + 1] as u32;
                out.data[dy * width + dx] = ((s + 2) >> 2) as u8;
                dx += 1;
            }
        }
        while dx < w {
            let mut s = 0i32;
            for k in 0..sy {
                let r = src.row(y0 + k);
                for j in 0..sx {
                    s += r[dx * sx + j] as i32;
                }
            }
            out.data[dy * width + dx] = sat_u8_f32(s as f32 * scale);
            dx += 1;
        }
        while dx < width {
            let x0 = dx * sx;
            let (mut s, mut count) = (0i32, 0i32);
            for k in 0..sy {
                if y0 + k >= src.height {
                    break;
                }
                for j in 0..sx {
                    if x0 + j >= src.width {
                        break;
                    }
                    s += src.at(x0 + j, y0 + k) as i32;
                    count += 1;
                }
            }
            out.data[dy * width + dx] = if count > 0 { sat_u8_f32(s as f32 / count as f32) } else { 0 };
            dx += 1;
        }
    }
    out
}

/// Keys cubic kernel, a = -0.75 (Mitchell-Netravali with B = 0, C = 0.75).
fn keys(x: f64) -> f64 {
    let x = x.abs();
    let a = -0.75;
    if x < 1.0 {
        (a + 2.0) * x * x * x - (a + 3.0) * x * x + 1.0
    } else if x < 2.0 {
        a * x * x * x - 5.0 * a * x * x + 8.0 * a * x - 4.0 * a
    } else {
        0.0
    }
}

/// Source taps and weights for each destination index.
fn cubic_taps(n_src: usize, n_dst: usize) -> Vec<([usize; 4], [f64; 4])> {
    let scale = n_src as f64 / n_dst as f64;
    (0..n_dst)
        .map(|d| {
            let f = (d as f64 + 0.5) * scale - 0.5;
            let s = f.floor();
            let t = f - s;
            let s = s as i64;
            let idx = [-1i64, 0, 1, 2].map(|k| (s + k).clamp(0, n_src as i64 - 1) as usize);
            (idx, [keys(t + 1.0), keys(t), keys(1.0 - t), keys(2.0 - t)])
        })
        .collect()
}

/// cv2.resize(src, (width, height), interpolation=INTER_CUBIC) as arabic-ocr
/// runs it: OpenCV hands 8-bit cubic resizes to Intel IPP (B = 0, C = 0.75,
/// edges replicated). IPP is closed; this float64 version, rows then
/// columns, rounded half to even, was found to match it on all but about 5 in
/// a million pixels (by one level).
pub fn resize_cubic(src: &Gray, width: usize, height: usize) -> Gray {
    let xs = cubic_taps(src.width, width);
    let ys = cubic_taps(src.height, height);
    let mut rows = vec![0f64; src.height * width];
    for y in 0..src.height {
        let s = src.row(y);
        for (x, (idx, w)) in xs.iter().enumerate() {
            let v = s[idx[0]] as f64 * w[0] + s[idx[1]] as f64 * w[1];
            rows[y * width + x] = v + s[idx[2]] as f64 * w[2] + s[idx[3]] as f64 * w[3];
        }
    }
    let mut out = Gray::new(width, height, 0);
    for (y, (idx, w)) in ys.iter().enumerate() {
        let r = |k: usize| &rows[idx[k] * width..(idx[k] + 1) * width];
        let (r0, r1, r2, r3) = (r(0), r(1), r(2), r(3));
        let d = &mut out.data[y * width..(y + 1) * width];
        for x in 0..width {
            let v = r0[x] * w[0] + r1[x] * w[1] + r2[x] * w[2] + r3[x] * w[3];
            d[x] = v.round_ties_even().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Affine warps
// ---------------------------------------------------------------------------

/// A 2x3 affine matrix: x' = m[0] x + m[1] y + m[2], y' = m[3] x + m[4] y + m[5].
pub type Affine = [f64; 6];

/// cv2.invertAffineTransform.
pub fn invert_affine(m: &Affine) -> Affine {
    let d = m[0] * m[4] - m[1] * m[3];
    let d = if d != 0.0 { 1.0 / d } else { 0.0 };
    let a11 = m[4] * d;
    let a22 = m[0] * d;
    let a12 = -m[1] * d;
    let a21 = -m[3] * d;
    let b1 = -a11 * m[2] - a12 * m[5];
    let b2 = -a21 * m[2] - a22 * m[5];
    [a11, a12, b1, a21, a22, b2]
}

/// `outer` after `inner`.
pub fn compose(outer: &Affine, inner: &Affine) -> Affine {
    let [a, b, c, d, e, f] = *outer;
    let [g, h, i, j, k, l] = *inner;
    [a * g + b * j, a * h + b * k, a * i + b * l + c, d * g + e * j, d * h + e * k, d * i + e * l + f]
}

pub fn apply(m: &Affine, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5])
}

/// cv2.getRotationMatrix2D.
pub fn rotation_matrix(cx: f64, cy: f64, angle: f64) -> Affine {
    let a = angle.to_radians();
    let alpha = a.cos();
    let beta = a.sin();
    [alpha, beta, (1.0 - alpha) * cx - beta * cy, -beta, alpha, beta * cx + (1.0 - alpha) * cy]
}

/// Lanes of OpenCV's vector loop (AVX2: 8 floats, two vectors per step).
const WARP_UF: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Interp {
    Nearest,
    Linear,
}

/// cv2.warpAffine(src, m, (width, height), flags, BORDER_CONSTANT, border).
/// `m` maps source to destination, as in OpenCV.
pub fn warp_affine(src: &Gray, m: &Affine, width: usize, height: usize, interp: Interp, border: u8) -> Gray {
    let inv = invert_affine(m);
    let mf = inv.map(|v| v as f32);
    let (sw, sh) = (src.width as i64, src.height as i64);
    let mut out = Gray::new(width, height, border);
    let vec_end = width / WARP_UF * WARP_UF;
    let fetch = |ix: i64, iy: i64| -> f32 {
        if ix >= 0 && ix < sw && iy >= 0 && iy < sh { src.data[(iy * sw + ix) as usize] as f32 } else { border as f32 }
    };
    for y in 0..height {
        let yf = y as f32;
        let mx = yf * mf[1] + mf[2];
        let my = yf * mf[4] + mf[5];
        let d = &mut out.data[y * width..(y + 1) * width];
        for (x, px) in d.iter_mut().enumerate() {
            let xf = x as f32;
            let (sx, sy) = if x < vec_end {
                (fma32(mf[0], xf, mx), fma32(mf[3], xf, my))
            } else {
                (xf * mf[0] + yf * mf[1] + mf[2], xf * mf[3] + yf * mf[4] + mf[5])
            };
            match interp {
                Interp::Nearest => {
                    let (ix, iy) = (sx.round_ties_even() as i64, sy.round_ties_even() as i64);
                    *px = if ix >= 0 && ix < sw && iy >= 0 && iy < sh {
                        src.data[(iy * sw + ix) as usize]
                    } else {
                        border
                    };
                }
                Interp::Linear => {
                    let (fx, fy) = (sx.floor(), sy.floor());
                    let (ix, iy) = (fx as i64, fy as i64);
                    if ix < -1 || ix >= sw || iy < -1 || iy >= sh {
                        *px = border;
                        continue;
                    }
                    let (ax, ay) = (sx - fx, sy - fy);
                    let p00 = fetch(ix, iy);
                    let p01 = fetch(ix + 1, iy);
                    let p10 = fetch(ix, iy + 1);
                    let p11 = fetch(ix + 1, iy + 1);
                    let v = if x < vec_end {
                        let v0 = fma32(ax, p01 - p00, p00);
                        let v1 = fma32(ax, p11 - p10, p10);
                        fma32(ay, v1 - v0, v0)
                    } else {
                        let v0 = p00 + ax * (p01 - p00);
                        let v1 = p10 + ax * (p11 - p10);
                        v0 + ay * (v1 - v0)
                    };
                    *px = sat_u8_f32(v);
                }
            }
        }
    }
    out
}

/// cv2.getAffineTransform for three point pairs (solved in double).
pub fn affine_from_points(src: [[f32; 2]; 3], dst: [[f32; 2]; 3]) -> Affine {
    // Unknowns a..f: x' = a x + b y + c, y' = d x + e y + f. Same 6x6 system as
    // OpenCV, solved with Gaussian elimination (partial pivoting).
    let mut a = [[0f64; 7]; 6];
    for i in 0..3 {
        let (x, y) = (src[i][0] as f64, src[i][1] as f64);
        a[i * 2] = [x, y, 1.0, 0.0, 0.0, 0.0, dst[i][0] as f64];
        a[i * 2 + 1] = [0.0, 0.0, 0.0, x, y, 1.0, dst[i][1] as f64];
    }
    for col in 0..6 {
        let p = (col..6).max_by(|&r, &s| a[r][col].abs().total_cmp(&a[s][col].abs())).unwrap_or(col);
        a.swap(col, p);
        let piv = a[col][col];
        if piv == 0.0 {
            continue;
        }
        for r in 0..6 {
            if r != col {
                let f = a[r][col] / piv;
                for c in col..7 {
                    a[r][c] -= f * a[col][c];
                }
            }
        }
    }
    let mut m = [0f64; 6];
    for (i, v) in m.iter_mut().enumerate() {
        *v = if a[i][i] != 0.0 { a[i][6] / a[i][i] } else { 0.0 };
    }
    m
}

// ---------------------------------------------------------------------------
// Thresholds
// ---------------------------------------------------------------------------

/// Otsu's threshold (cv2.threshold with THRESH_OTSU), as an integer level.
pub fn otsu(g: &Gray) -> u8 {
    let mut h = [0u64; 256];
    for &v in &g.data {
        h[v as usize] += 1;
    }
    let scale = 1.0 / g.data.len() as f64;
    let mut mu = 0.0;
    for (i, &c) in h.iter().enumerate() {
        mu += i as f64 * c as f64;
    }
    mu *= scale;
    let (mut mu1, mut q1) = (0.0f64, 0.0f64);
    let (mut max_sigma, mut max_val) = (0.0f64, 0u8);
    for (i, &c) in h.iter().enumerate() {
        let p_i = c as f64 * scale;
        mu1 *= q1;
        q1 += p_i;
        let q2 = 1.0 - q1;
        let eps = f32::EPSILON as f64;
        if q1.min(q2) < eps || q1.max(q2) > 1.0 - eps {
            continue;
        }
        mu1 = (mu1 + i as f64 * p_i) / q1;
        let mu2 = (mu - q1 * mu1) / q2;
        let sigma = q1 * q2 * (mu1 - mu2) * (mu1 - mu2);
        if sigma > max_sigma {
            max_sigma = sigma;
            max_val = i as u8;
        }
    }
    max_val
}

/// Pixels at or below `t` become `on`, others 0 (THRESH_BINARY_INV).
pub fn threshold_inv(g: &Gray, t: u8, on: u8) -> Gray {
    Gray::from_vec(g.width, g.height, g.data.iter().map(|&v| if v > t { 0 } else { on }).collect())
}

/// Gaussian kernel as OpenCV's getGaussianKernel(n, 0, CV_32F) (n > 9).
fn gaussian_kernel(n: usize) -> Vec<f32> {
    let sigma = (n as f64).mul_add(0.15, 0.35);
    let scale2 = -0.125 / (sigma * sigma);
    let half = (n - 1) / 2;
    let mut values = Vec::with_capacity(half);
    let mut sum = 0.0f64;
    let mut x = 1 - n as i64;
    for _ in 0..half {
        let t = ((x * x) as f64 * scale2).exp();
        values.push(t);
        sum += t;
        x += 2;
    }
    sum *= 2.0;
    sum += 1.0;
    let mul = 1.0 / sum;
    let mut k = vec![0f32; n];
    for (i, v) in values.iter().enumerate() {
        let t = (v * mul) as f32;
        k[i] = t;
        k[n - 1 - i] = t;
    }
    k[half] = mul as f32;
    k
}

/// cv2.adaptiveThreshold(g, on, GAUSSIAN_C, THRESH_BINARY_INV, block, c):
/// pixels darker than their Gaussian-weighted neighbourhood by more than `c`.
pub fn adaptive_threshold_inv(g: &Gray, on: u8, block: usize, c: f64) -> Gray {
    let mean = gaussian_blur_replicate(g, block);
    let idelta = c.floor() as i32;
    let data = g.data.iter().zip(&mean).map(|(&s, &m)| if s as i32 - m as i32 <= -idelta { on } else { 0 }).collect();
    Gray::from_vec(g.width, g.height, data)
}

/// Fused multiply-add of float32 values: the product is exact in f64, so one
/// rounding to f32 gives the fused result (a double rounding could differ in
/// the last bit, in practice never). Much faster than f32::mul_add without
/// compiling for FMA.
#[inline(always)]
pub fn fma32(a: f32, b: f32, c: f32) -> f32 {
    (a as f64 * b as f64 + c as f64) as f32
}

/// Run `f(first_row, rows)` over bands of `out` (`row_len` values per row) on
/// up to 8 threads. Each output value depends only on the input, so results
/// are the same for any number of threads.
fn par_rows<T: Send>(out: &mut [T], row_len: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    let rows = out.len() / row_len.max(1);
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(8);
    if threads <= 1 || rows < 64 {
        f(0, out);
        return;
    }
    let band = rows.div_ceil(threads);
    std::thread::scope(|s| {
        for (i, chunk) in out.chunks_mut(band * row_len).enumerate() {
            let f = &f;
            s.spawn(move || f(i * band, chunk));
        }
    });
}

/// GaussianBlur of the image as float32 (BORDER_REPLICATE), rounded back to
/// bytes: rows first, then columns, with fused multiply-adds as in OpenCV's
/// AVX2 filters.
fn gaussian_blur_replicate(g: &Gray, n: usize) -> Vec<u8> {
    let (w, h) = (g.width, g.height);
    let k = gaussian_kernel(n);
    let kx = if w > 1 { &k[..] } else { &[1.0][..] };
    let ky = if h > 1 { &k[..] } else { &[1.0][..] };
    let (rx, ry) = (kx.len() / 2, ky.len() / 2);
    let mut rows = vec![0f32; w * h];
    par_rows(&mut rows, w, |y0, band| {
        let mut line = vec![0f32; w + 2 * rx];
        for (dy, d) in band.chunks_mut(w).enumerate() {
            let s = g.row(y0 + dy);
            for (i, v) in line.iter_mut().enumerate() {
                let x = (i as i64 - rx as i64).clamp(0, w as i64 - 1) as usize;
                *v = s[x] as f32;
            }
            for (x, out) in d.iter_mut().enumerate() {
                let mut acc = 0f32;
                for (j, &kj) in kx.iter().enumerate() {
                    acc = fma32(line[x + j], kj, acc);
                }
                *out = acc;
            }
        }
    });
    let mut out = vec![0u8; w * h];
    let rows = &rows;
    let row_of = |y: i64| -> &[f32] {
        let y = y.clamp(0, h as i64 - 1) as usize;
        &rows[y * w..(y + 1) * w]
    };
    par_rows(&mut out, w, |y0, band| {
        let mut acc = vec![0f32; w];
        for (dy, d) in band.chunks_mut(w).enumerate() {
            let y = (y0 + dy) as i64;
            for (a, &v) in acc.iter_mut().zip(row_of(y)) {
                *a = fma32(v, ky[ry], 0.0);
            }
            for kk in 1..=ry {
                let (a, b) = (row_of(y + kk as i64), row_of(y - kk as i64));
                let f = ky[ry + kk];
                for x in 0..w {
                    acc[x] = fma32(a[x] + b[x], f, acc[x]);
                }
            }
            for (o, &v) in d.iter_mut().zip(&acc) {
                *o = sat_u8_f32(v);
            }
        }
    });
    out
}
// ---------------------------------------------------------------------------
// Morphology (binary masks: 0 or any non-zero value)
// ---------------------------------------------------------------------------

/// Min (erode) or max (dilate) over every window of `k` values of `src`,
/// the window for output i starting at i - `anchor`; values outside count as
/// `neutral`. Van Herk / Gil-Werman: three operations per value, any `k`.
#[allow(clippy::too_many_arguments)]
fn window_op(
    src: &[u8],
    k: usize,
    anchor: usize,
    dilate: bool,
    neutral: u8,
    out: &mut [u8],
    pad: &mut Vec<u8>,
    g: &mut Vec<u8>,
    hh: &mut Vec<u8>,
) {
    let n = src.len();
    let pick = |a: u8, b: u8| if dilate { a.max(b) } else { a.min(b) };
    let len = (n + k - 1).div_ceil(k) * k;
    pad.clear();
    pad.resize(anchor, neutral);
    pad.extend_from_slice(src);
    pad.resize(len, neutral);
    g.resize(len, 0);
    hh.resize(len, 0);
    for b in (0..len).step_by(k) {
        g[b] = pad[b];
        for j in b + 1..b + k {
            g[j] = pick(g[j - 1], pad[j]);
        }
        hh[b + k - 1] = pad[b + k - 1];
        for j in (b..b + k - 1).rev() {
            hh[j] = pick(hh[j + 1], pad[j]);
        }
    }
    for (i, o) in out.iter_mut().enumerate().take(n) {
        *o = pick(hh[i], g[i + k - 1]);
    }
}

/// Erode (min) or dilate (max) with a kw x kh rectangle anchored at its
/// centre; pixels outside the image never win (OpenCV's default border).
fn morph(src: &Gray, kw: usize, kh: usize, dilate: bool) -> Gray {
    let (w, h) = (src.width, src.height);
    let neutral = if dilate { 0 } else { 255 };
    let (mut pad, mut g, mut hh) = (Vec::new(), Vec::new(), Vec::new());
    // Horizontal pass, then vertical (a rectangle is separable).
    let mut tmp = Gray::new(w, h, 0);
    if kw > 1 {
        for y in 0..h {
            window_op(
                src.row(y),
                kw,
                kw / 2,
                dilate,
                neutral,
                &mut tmp.data[y * w..(y + 1) * w],
                &mut pad,
                &mut g,
                &mut hh,
            );
        }
    } else {
        tmp.data.copy_from_slice(&src.data);
    }
    if kh <= 1 {
        return tmp;
    }
    let mut out = Gray::new(w, h, 0);
    let mut col = vec![0u8; h];
    let mut res = vec![0u8; h];
    for x in 0..w {
        for (y, c) in col.iter_mut().enumerate() {
            *c = tmp.data[y * w + x];
        }
        window_op(&col, kh, kh / 2, dilate, neutral, &mut res, &mut pad, &mut g, &mut hh);
        for (y, &r) in res.iter().enumerate() {
            out.data[y * w + x] = r;
        }
    }
    out
}
pub fn erode(src: &Gray, kw: usize, kh: usize) -> Gray {
    morph(src, kw, kh, false)
}

pub fn dilate(src: &Gray, kw: usize, kh: usize) -> Gray {
    morph(src, kw, kh, true)
}

/// cv2.morphologyEx(MORPH_OPEN) with a kw x kh rectangle.
pub fn open(src: &Gray, kw: usize, kh: usize) -> Gray {
    dilate(&erode(src, kw, kh), kw, kh)
}

/// cv2.medianBlur(g, 3) (edges replicated).
pub fn median3(g: &Gray) -> Gray {
    let (w, h) = (g.width as i64, g.height as i64);
    let mut out = Gray::new(g.width, g.height, 0);
    for y in 0..h {
        for x in 0..w {
            let mut v = [0u8; 9];
            let mut k = 0;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let xx = (x + dx).clamp(0, w - 1);
                    let yy = (y + dy).clamp(0, h - 1);
                    v[k] = g.data[(yy * w + xx) as usize];
                    k += 1;
                }
            }
            v.sort_unstable();
            out.data[(y * w + x) as usize] = v[4];
        }
    }
    out
}

/// Heights of the 8-connected components of non-zero pixels.
pub fn component_heights(mask: &Gray) -> Vec<usize> {
    let (w, h) = (mask.width, mask.height);
    let mut label = vec![0u32; w * h];
    let mut heights = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if mask.data[start] == 0 || label[start] != 0 {
            continue;
        }
        let id = heights.len() as u32 + 1;
        label[start] = id;
        stack.push(start);
        let (mut y0, mut y1) = (start / w, start / w);
        while let Some(p) = stack.pop() {
            let (x, y) = (p % w, p / w);
            y0 = y0.min(y);
            y1 = y1.max(y);
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (xx, yy) = (x as i64 + dx, y as i64 + dy);
                    if xx < 0 || yy < 0 || xx >= w as i64 || yy >= h as i64 {
                        continue;
                    }
                    let q = yy as usize * w + xx as usize;
                    if mask.data[q] != 0 && label[q] == 0 {
                        label[q] = id;
                        stack.push(q);
                    }
                }
            }
        }
        heights.push(y1 - y0 + 1);
    }
    heights
}

/// cv2.copyMakeBorder(BORDER_CONSTANT): pad at the bottom and right.
pub fn pad_bottom_right(g: &Gray, width: usize, height: usize, value: u8) -> Gray {
    let mut out = Gray::new(width, height, value);
    for y in 0..g.height {
        out.data[y * width..y * width + g.width].copy_from_slice(g.row(y));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_like_numpy() {
        assert_eq!(median_int(&mut [3, 1, 2]), 2);
        assert_eq!(median_int(&mut [1, 2, 4, 9]), 3); // (2 + 4) / 2
        assert_eq!(median_int(&mut [1, 2]), 1); // 1.5 -> int 1
    }

    #[test]
    fn affine_inverse_round_trip() {
        let m = rotation_matrix(10.0, 20.0, 7.0);
        let i = invert_affine(&m);
        let (x, y) = apply(&m, 3.0, 4.0);
        let (bx, by) = apply(&i, x, y);
        assert!((bx - 3.0).abs() < 1e-9 && (by - 4.0).abs() < 1e-9);
    }
}
