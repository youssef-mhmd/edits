//! Small statistics helpers.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct Summary {
    pub n: usize,
    pub min: f64,
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
    pub mean: f64,
}

pub fn summarize(values: &[f64]) -> Summary {
    if values.is_empty() {
        return Summary::default();
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let pct = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
    Summary {
        n: v.len(),
        min: v[0],
        p50: pct(0.5),
        p95: pct(0.95),
        max: v[v.len() - 1],
        mean: v.iter().sum::<f64>() / v.len() as f64,
    }
}

pub fn median(values: &[f64]) -> f64 {
    summarize(values).p50
}
