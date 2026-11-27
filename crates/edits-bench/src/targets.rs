//! PRD section 7 targets and pass/fail reporting.

use serde::Serialize;

/// One measured value against its target. Lower is better for every target.
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: String,
    pub value: f64,
    pub target: f64,
    pub unit: &'static str,
    pub pass: bool,
}

impl Check {
    pub fn new(name: impl Into<String>, value: f64, target: f64, unit: &'static str) -> Self {
        Self { name: name.into(), value, target, unit, pass: value <= target }
    }
}

pub fn print(checks: &[Check]) {
    for c in checks {
        println!(
            "  {} {:<52} {:>9.1} {:<3} (target <= {} {})",
            if c.pass { "PASS" } else { "FAIL" },
            c.name,
            c.value,
            c.unit,
            c.target,
            c.unit
        );
    }
}

// Section 7 values.
/// Relaxed from 400 ms (PRD open question 5): WebView2 start-up alone is about 400 ms.
pub const COLD_START_MS: f64 = 500.0;
pub const OPEN_10MB_FIRST_PAGE_MS: f64 = 600.0;
pub const OPEN_LARGE_FIRST_PAGE_MS: f64 = 1000.0;
pub const SHARP_AFTER_SCROLL_MS: f64 = 100.0;
pub const ZOOM_SHARP_MS: f64 = 100.0;
/// 60 fps: a frame every 16.7 ms. The p95 frame interval is held to this,
/// with a small allowance for timer jitter.
pub const FRAME_P95_MS: f64 = 1000.0 / 60.0 + 2.0;
/// Growth of all processes (WebView2 included) over the app with no document
/// open (PRD open question 6).
pub const MEMORY_ABOVE_IDLE_MB: f64 = 200.0;
pub const SEARCH_FIRST_MS: f64 = 200.0;
pub const SEARCH_ALL_MS: f64 = 2000.0;
