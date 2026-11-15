//! Decoded frames to upright RGB at the size the caller asked for.

use super::to_u32;
use crate::media::image::Rgb;
use anyhow::{Result, bail};
use ffmpeg_next as ff;

/// Converts decoded frames to upright RGB at a requested size.
pub(super) struct Converter {
    pub(super) min_side: u32,
    pub(super) scaler: Option<(
        ff::software::scaling::Context,
        (ff::format::Pixel, u32, u32),
    )>,
    pub(super) out_dims: (u32, u32),
    /// Original display size (sample aspect ratio applied, before rotation).
    pub(super) display: (u32, u32),
    pub(super) rotation: u32,
    pub(super) rotation_known: bool,
}

impl Converter {
    pub(super) fn new(min_side: u32) -> Self {
        Self {
            min_side,
            scaler: None,
            out_dims: (0, 0),
            display: (0, 0),
            rotation: 0,
            rotation_known: false,
        }
    }

    pub(super) fn convert(&mut self, f: &ff::frame::Video) -> Result<Rgb> {
        let (w, h, fmt) = (f.width(), f.height(), f.format());
        if w == 0 || h == 0 {
            bail!("empty frame");
        }
        if !self.rotation_known {
            self.rotation = rotation_of(f);
            self.rotation_known = true;
        }
        if self.scaler.as_ref().is_none_or(|s| s.1 != (fmt, w, h)) {
            let sar = f.aspect_ratio();
            let sar = if sar.numerator() > 0 && sar.denominator() > 0 {
                f64::from(sar)
            } else {
                1.0
            };
            let (display, (ow, oh)) = frame_sizes(w, h, sar, self.min_side);
            self.display = display;
            let ctx = ff::software::scaling::Context::get(
                fmt,
                w,
                h,
                ff::format::Pixel::RGB24,
                ow,
                oh,
                ff::software::scaling::Flags::AREA,
            )?;
            self.scaler = Some((ctx, (fmt, w, h)));
            self.out_dims = (ow, oh);
        }
        let (ow, oh) = self.out_dims;
        let mut out = ff::frame::Video::empty();
        self.scaler.as_mut().expect("scaler").0.run(f, &mut out)?;
        let stride = out.stride(0);
        let data = out.data(0);
        let row = ow as usize * 3;
        let mut px = Vec::with_capacity(row * oh as usize);
        for y in 0..oh as usize {
            px.extend_from_slice(&data[y * stride..y * stride + row]);
        }
        let rgb = Rgb {
            width: ow,
            height: oh,
            data: px,
        };
        rotate(rgb, self.rotation)
    }
}

/// Largest sample aspect ratio (and its inverse) taken at face value. Real anamorphic video
/// stays within about 0.5 to 2.7; anything beyond is a damaged or hostile header and would
/// ask the scaler for an enormous frame.
const MAX_SAMPLE_ASPECT: f64 = 4.0;

/// Display size (sample aspect ratio applied) and even output size whose shorter side is at
/// most `min_side`, for a `w`x`h` frame. An implausible ratio is treated as square pixels.
pub(super) fn frame_sizes(w: u32, h: u32, sar: f64, min_side: u32) -> ((u32, u32), (u32, u32)) {
    let sar = if sar.is_finite() && (1.0 / MAX_SAMPLE_ASPECT..=MAX_SAMPLE_ASPECT).contains(&sar) {
        sar
    } else {
        1.0
    };
    let (dw, dh) = ((f64::from(w) * sar).max(1.0), f64::from(h));
    let scale = (f64::from(min_side) / dw.min(dh)).min(1.0);
    let ow = to_u32(dw * scale).max(2) & !1;
    let oh = to_u32(dh * scale).max(2) & !1;
    ((to_u32(dw), h), (ow, oh))
}

/// Clockwise rotation (0/90/180/270) needed to display the frame upright, from the
/// display matrix (same math as libavutil's `av_display_rotation_get`).
fn rotation_of(f: &ff::frame::Video) -> u32 {
    let Some(sd) = f.side_data(ff::frame::side_data::Type::DisplayMatrix) else {
        return 0;
    };
    let d = sd.data();
    if d.len() < 36 {
        return 0;
    }
    let m: Vec<f64> = d
        .as_chunks::<4>()
        .0
        .iter()
        .take(9)
        .map(|c| f64::from(i32::from_ne_bytes(*c)) / 65536.0)
        .collect();
    rotation_from_matrix(&m)
}

pub(super) fn rotation_from_matrix(m: &[f64]) -> u32 {
    let s0 = m[0].hypot(m[3]);
    let s1 = m[1].hypot(m[4]);
    if s0 == 0.0 || s1 == 0.0 {
        return 0;
    }
    // av_display_rotation_get = -atan2(m1/s1, m0/s0); the upright transform is its negation.
    let ccw = -(m[1] / s1).atan2(m[0] / s0).to_degrees();
    let cw = (-ccw).rem_euclid(360.0);
    (to_u32(cw / 90.0) % 4) * 90
}

pub(super) fn rotate(rgb: Rgb, cw: u32) -> Result<Rgb> {
    if cw == 0 {
        return Ok(rgb);
    }
    let (width, height) = (rgb.width, rgb.height);
    let Some(img) = image::RgbImage::from_raw(width, height, rgb.data) else {
        bail!("frame buffer does not match its {width}x{height} size");
    };
    let r = match cw {
        90 => image::imageops::rotate90(&img),
        180 => image::imageops::rotate180(&img),
        _ => image::imageops::rotate270(&img),
    };
    Ok(Rgb {
        width: r.width(),
        height: r.height(),
        data: r.into_raw(),
    })
}
