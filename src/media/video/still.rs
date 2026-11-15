//! The single frame shown at a given time (thumbnails and previews).

use super::convert::Converter;
use super::stream::open;
use super::to_i64;
use crate::media::image::Rgb;
use anyhow::{Result, anyhow, bail};
use ffmpeg_next as ff;
use std::path::Path;

/// Decodes the frame shown at time `t` (seconds from the start), scaled so its shorter side is
/// ≈`min_side`. A time past the last frame yields the last frame.
pub fn frame_at(path: &Path, t: f64, min_side: u32) -> Result<Rgb> {
    match find_frame(path, t, min_side)? {
        Found::Frame(rgb) => Ok(rgb),
        // `t` lies past the last frame: one more attempt at the last frame's own time.
        Found::PastEnd(last) => match find_frame(path, last, min_side)? {
            Found::Frame(rgb) => Ok(rgb),
            _ => Err(anyhow!("no frame at {t:.1}s")),
        },
        Found::Missing => Err(anyhow!("no frame at {t:.1}s")),
    }
}

enum Found {
    Frame(Rgb),
    /// No frame at the requested time; the last frame seen was earlier, at this time.
    PastEnd(f64),
    Missing,
}

fn find_frame(path: &Path, t: f64, min_side: u32) -> Result<Found> {
    let mut s = open(path, 2)?;
    let target = if t.is_finite() { t.max(0.0) } else { 0.0 };
    // Seek to the keyframe at or before `t`, in absolute stream time (streams may not start
    // at zero, e.g. MPEG-TS); fall back to decoding from the start if seeking fails.
    let abs = s.start as f64 * s.tb + target;
    let ts = to_i64(abs * f64::from(ff::ffi::AV_TIME_BASE));
    let _ = s.ictx.seek(ts, ..ts.saturating_add(1));
    let mut conv = Converter::new(min_side);
    let mut found: Option<Rgb> = None;
    let mut decoded = 0usize;
    while let Some(pkt) = s.next_packet()? {
        if s.dec.send_packet(&pkt).is_err() {
            continue;
        }
        s.drain(&mut |ft, f| {
            decoded += 1;
            if ft + 0.02 >= target {
                found = Some(conv.convert(f)?);
                return Ok(false);
            }
            if decoded > 3000 {
                bail!("could not reach {t:.1}s (seeking failed)");
            }
            Ok(true)
        })?;
        if found.is_some() {
            break;
        }
    }
    let last_t = s.last_t;
    if found.is_none() {
        // Past the last frame (or decoder delay at EOF): take the last frame available.
        let _ = s.dec.send_eof();
        let mut last: Option<ff::frame::Video> = None;
        s.drain(&mut |_, f| {
            last = Some(f.clone());
            Ok(true)
        })?;
        if let Some(f) = last {
            found = Some(conv.convert(&f)?);
        } else if let Some(lt) = last_t.filter(|lt| *lt < target) {
            return Ok(Found::PastEnd(lt));
        }
    }
    Ok(found.map_or(Found::Missing, Found::Frame))
}
