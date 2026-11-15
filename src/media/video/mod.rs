//! Video frame sampling with `FFmpeg` (see `docs/design.md`).
//!
//! The sampler (`sample`) works at the packet level so it does not depend on the decoder
//! honouring `skip_frame`: packets are buffered one GOP (keyframe to keyframe) at a time; when
//! the next keyframe arrives the GOP's duration is known.
//! - short GOP (≤ `max_gap`): decode only its keyframe, and only when a sample is due;
//! - long GOP: decode in order until every sample time due inside the GOP has been produced.
//!
//! `stream` opens files and reads packets, `convert` turns decoded frames into upright RGB,
//! and `still` fetches the single frame shown at a given time.

mod convert;
mod sample;
mod still;
mod stream;

pub use sample::sample;
pub use still::frame_at;
pub(crate) use stream::{header_duration, open_input, storage_unreachable};

use super::image::Rgb;
use ffmpeg_next as ff;
use std::sync::Once;

/// Default upper bound for one buffered GOP.
const MAX_GOP_BYTES: usize = 64 << 20;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplerConfig {
    /// Target spacing between samples, seconds.
    pub interval: f64,
    /// GOPs up to this long are represented by their keyframe alone, seconds.
    pub max_gap: f64,
    /// Upper bound for one buffered GOP; beyond it we decode packet by packet.
    pub max_gop_bytes: usize,
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self {
            interval: 2.0,
            max_gap: 4.0,
            max_gop_bytes: MAX_GOP_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct VideoInfo {
    pub duration: f64,
    /// Display size (after sample aspect ratio and rotation).
    pub width: u32,
    pub height: u32,
}

/// A decoded sample: presentation time (seconds from the start) and upright RGB pixels whose
/// shorter side is about `min_side`.
pub struct Frame {
    pub t: f64,
    pub rgb: Rgb,
}

fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = ff::init();
        ff::log::set_level(ff::log::Level::Fatal);
    });
}

/// A float rounded to the nearest whole number in `0..=u32::MAX` (NaN gives 0).
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to the u32 range first"
)]
fn to_u32(x: f64) -> u32 {
    x.round().clamp(0.0, f64::from(u32::MAX)) as u32
}

/// A float as `i64`, saturating at the ends (NaN gives 0).
#[expect(
    clippy::cast_possible_truncation,
    reason = "saturating float-to-integer conversion is the intent"
)]
fn to_i64(x: f64) -> i64 {
    x as i64
}

#[cfg(test)]
pub(crate) mod tests;
