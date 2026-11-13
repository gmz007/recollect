//! Optional frame-time statistics, enabled with the `RECOLLECT_FRAME_STATS` environment
//! variable. Used to check that the UI stays responsive while background work runs.

use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(10);

pub struct FrameStats {
    last_frame: Option<Instant>,
    window_start: Instant,
    /// Frame intervals in milliseconds for the current window.
    samples: Vec<f32>,
    busy_seen: bool,
}

impl FrameStats {
    /// Returns a collector when the environment variable is set (to any value).
    pub fn from_env() -> Option<Self> {
        std::env::var_os("RECOLLECT_FRAME_STATS").map(|_| Self::new())
    }

    fn new() -> Self {
        Self {
            last_frame: None,
            window_start: Instant::now(),
            samples: Vec::new(),
            busy_seen: false,
        }
    }

    /// Call once per frame. Logs a summary every ten seconds.
    pub fn frame(&mut self, busy: bool) {
        let now = Instant::now();
        if let Some(prev) = self.last_frame.replace(now) {
            self.samples.push((now - prev).as_secs_f32() * 1000.0);
        }
        self.busy_seen |= busy;
        if now - self.window_start >= WINDOW {
            if let Some((p50, p95, max)) = summarize(&mut self.samples) {
                tracing::info!(
                    frames = self.samples.len(),
                    p50_ms = format!("{p50:.1}"),
                    p95_ms = format!("{p95:.1}"),
                    max_ms = format!("{max:.1}"),
                    background_job = self.busy_seen,
                    "frame times over the last 10 s"
                );
            }
            self.samples.clear();
            self.busy_seen = busy;
            self.window_start = now;
        }
    }
}

/// Returns (p50, p95, max) of the samples, sorting them in place.
fn summarize(samples: &mut [f32]) -> Option<(f32, f32, f32)> {
    if samples.is_empty() {
        return None;
    }
    samples.sort_by(f32::total_cmp);
    Some((
        percentile(samples, 50.0),
        percentile(samples, 95.0),
        samples[samples.len() - 1],
    ))
}

/// Nearest-rank percentile of an ascending, non-empty slice.
fn percentile(sorted: &[f32], p: f32) -> f32 {
    let rank = (p / 100.0 * sorted.len() as f32).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_use_nearest_rank() {
        let mut v: Vec<f32> = (1..=100).rev().map(|x| x as f32).collect();
        assert_eq!(summarize(&mut v), Some((50.0, 95.0, 100.0)));
        let mut one = [7.0];
        assert_eq!(summarize(&mut one), Some((7.0, 7.0, 7.0)));
        assert_eq!(summarize(&mut []), None);
    }
}
