//! Video frame sampling with FFmpeg (ADR 0004).
//!
//! The sampler works at the packet level so it does not depend on the decoder honouring
//! `skip_frame`: packets are buffered one GOP (keyframe to keyframe) at a time; when the next
//! keyframe arrives the GOP's duration is known.
//! - short GOP (≤ `max_gap`): decode only its keyframe, and only when a sample is due;
//! - long GOP: decode in order until every sample time due inside the GOP has been produced.

use super::image::Rgb;
use anyhow::{Context, Result, anyhow, bail};
use ffmpeg_next as ff;
use std::path::Path;
use std::sync::Once;

/// Upper bound for one buffered GOP; beyond it we decode as we go.
const MAX_GOP_BYTES: usize = 64 << 20;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplerConfig {
    /// Target spacing between samples, seconds.
    pub interval: f64,
    /// GOPs up to this long are represented by their keyframe alone, seconds.
    pub max_gap: f64,
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self {
            interval: 2.0,
            max_gap: 4.0,
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

struct Stream {
    ictx: ff::format::context::Input,
    index: usize,
    tb: f64,
    start: i64,
    duration: f64,
    dec: ff::decoder::Video,
}

fn open(path: &Path) -> Result<Stream> {
    init();
    let ictx = ff::format::input(path).with_context(|| format!("opening {}", path.display()))?;
    let st = ictx
        .streams()
        .best(ff::media::Type::Video)
        .ok_or_else(|| anyhow!("no video stream"))?;
    let index = st.index();
    let tb = f64::from(st.time_base());
    let start = if st.start_time() == ff::ffi::AV_NOPTS_VALUE {
        0
    } else {
        st.start_time()
    };
    let mut duration = if st.duration() > 0 {
        st.duration() as f64 * tb
    } else {
        0.0
    };
    if duration <= 0.0 && ictx.duration() > 0 {
        duration = ictx.duration() as f64 / f64::from(ff::ffi::AV_TIME_BASE);
    }
    let mut cctx = ff::codec::context::Context::from_parameters(st.parameters())?;
    cctx.set_threading(ff::codec::threading::Config {
        kind: ff::codec::threading::Type::Frame,
        count: 0, // auto
    });
    let dec = cctx
        .decoder()
        .video()
        .context("no decoder for this video codec")?;
    Ok(Stream {
        ictx,
        index,
        tb,
        start,
        duration,
        dec,
    })
}

/// Converts decoded frames to upright RGB at a requested size.
struct Converter {
    min_side: u32,
    scaler: Option<(
        ff::software::scaling::Context,
        (ff::format::Pixel, u32, u32),
    )>,
    out_dims: (u32, u32),
    rotation: u32,
    rotation_known: bool,
}

impl Converter {
    fn new(min_side: u32) -> Self {
        Self {
            min_side,
            scaler: None,
            out_dims: (0, 0),
            rotation: 0,
            rotation_known: false,
        }
    }

    fn convert(&mut self, f: &ff::frame::Video) -> Result<Rgb> {
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
            let (dw, dh) = ((w as f64 * sar).max(1.0), h as f64);
            let scale = (self.min_side as f64 / dw.min(dh)).min(1.0);
            let ow = ((dw * scale).round() as u32).max(2) & !1;
            let oh = ((dh * scale).round() as u32).max(2) & !1;
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
        Ok(rotate(rgb, self.rotation))
    }
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

fn rotation_from_matrix(m: &[f64]) -> u32 {
    let s0 = m[0].hypot(m[3]);
    let s1 = m[1].hypot(m[4]);
    if s0 == 0.0 || s1 == 0.0 {
        return 0;
    }
    // av_display_rotation_get = -atan2(m1/s1, m0/s0); the upright transform is its negation.
    let ccw = -(m[1] / s1).atan2(m[0] / s0).to_degrees();
    let cw = (-ccw).rem_euclid(360.0);
    ((cw / 90.0).round() as u32 % 4) * 90
}

fn rotate(rgb: Rgb, cw: u32) -> Rgb {
    if cw == 0 {
        return rgb;
    }
    let Some(img) = image::RgbImage::from_raw(rgb.width, rgb.height, rgb.data) else {
        return Rgb {
            width: 0,
            height: 0,
            data: Vec::new(),
        };
    };
    let r = match cw {
        90 => image::imageops::rotate90(&img),
        180 => image::imageops::rotate180(&img),
        _ => image::imageops::rotate270(&img),
    };
    Rgb {
        width: r.width(),
        height: r.height(),
        data: r.into_raw(),
    }
}

impl Stream {
    fn time_of(&self, ts: i64) -> f64 {
        (ts - self.start) as f64 * self.tb
    }

    /// Receives all frames currently available; calls `f(time, frame)`; stops early if `f`
    /// returns false.
    fn drain(&mut self, f: &mut dyn FnMut(f64, &ff::frame::Video) -> Result<bool>) -> Result<bool> {
        let mut frame = ff::frame::Video::empty();
        while self.dec.receive_frame(&mut frame).is_ok() {
            let t = self.time_of(frame.timestamp().unwrap_or(self.start));
            if !f(t, &frame)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// Reads the duration and display size without decoding frames (except one for rotation/SAR).
pub fn probe(path: &Path) -> Result<VideoInfo> {
    let mut s = open(path)?;
    let (w, h) = (s.dec.width(), s.dec.height());
    if w == 0 || h == 0 {
        bail!("video has no size");
    }
    Ok(VideoInfo {
        duration: s.duration,
        width: w,
        height: h,
    })
    .map(|mut info| {
        // Display dims from a decoded frame if we can get one quickly.
        if let Ok(f) = first_frame_dims(&mut s) {
            info.width = f.0;
            info.height = f.1;
        }
        info
    })
}

fn first_frame_dims(s: &mut Stream) -> Result<(u32, u32)> {
    let idx = s.index;
    let mut conv = Converter::new(u32::MAX);
    let mut out = None;
    let mut packets = 0;
    let pkts: Vec<ff::Packet> = s
        .ictx
        .packets()
        .filter(|(st, _)| st.index() == idx)
        .map(|(_, p)| p)
        .take(64)
        .collect();
    for p in pkts {
        packets += 1;
        s.dec.send_packet(&p)?;
        s.drain(&mut |_, f| {
            let sar = f.aspect_ratio();
            let sar = if sar.numerator() > 0 && sar.denominator() > 0 {
                f64::from(sar)
            } else {
                1.0
            };
            conv.rotation = rotation_of(f);
            let (w, h) = ((f.width() as f64 * sar).round() as u32, f.height());
            out = Some(if conv.rotation % 180 == 90 {
                (h, w)
            } else {
                (w, h)
            });
            Ok(false)
        })?;
        if out.is_some() || packets > 64 {
            break;
        }
    }
    out.ok_or_else(|| anyhow!("no frame decoded"))
}

/// Samples frames through the whole video. `on_frame` receives frames in time order; return an
/// error from it to abort. `cancel` is polled between packets.
pub fn sample(
    path: &Path,
    cfg: SamplerConfig,
    min_side: u32,
    cancel: &dyn Fn() -> bool,
    on_frame: &mut dyn FnMut(Frame) -> Result<()>,
) -> Result<VideoInfo> {
    let mut s = open(path)?;
    let mut st = SampleState {
        conv: Converter::new(min_side),
        cfg,
        next_t: 0.0,
        emitted: 0,
        info: VideoInfo {
            duration: s.duration,
            ..Default::default()
        },
    };
    let mut gop: Vec<ff::Packet> = Vec::new();
    let mut gop_bytes = 0usize;
    let idx = s.index;
    loop {
        if cancel() {
            bail!("cancelled");
        }
        let mut pkt = ff::Packet::empty();
        match pkt.read(&mut s.ictx) {
            Ok(()) => {}
            Err(ff::Error::Eof) => break,
            Err(e) if is_retryable(&e) => continue,
            Err(e) => return Err(e).context("reading packet"),
        }
        if pkt.stream() != idx {
            continue;
        }
        if pkt.is_key() && !gop.is_empty() {
            let end = pkt.pts().or(pkt.dts()).map(|t| s.time_of(t));
            st.process_gop(&mut s, &gop, end, on_frame)?;
            gop.clear();
            gop_bytes = 0;
        }
        gop_bytes += pkt.size();
        gop.push(pkt);
        if gop_bytes > MAX_GOP_BYTES {
            // Pathological GOP: decode what we have in order, then keep streaming.
            st.process_gop(&mut s, &gop, None, on_frame)?;
            gop.clear();
            gop_bytes = 0;
        }
    }
    if !gop.is_empty() {
        let end = Some(s.duration.max(st.next_t));
        st.process_gop(&mut s, &gop, end, on_frame)?;
    }
    if st.emitted == 0 {
        bail!("no frames could be decoded");
    }
    if st.info.duration <= 0.0 {
        st.info.duration = st.next_t.max(0.0);
    }
    Ok(st.info)
}

fn is_retryable(e: &ff::Error) -> bool {
    matches!(e, ff::Error::Other { errno } if *errno == ff::ffi::EAGAIN)
}

struct SampleState {
    conv: Converter,
    cfg: SamplerConfig,
    next_t: f64,
    emitted: usize,
    info: VideoInfo,
}

impl SampleState {
    fn due(&self, t: f64) -> bool {
        t + 1e-6 >= self.next_t || self.emitted == 0
    }

    fn emit(
        &mut self,
        t: f64,
        f: &ff::frame::Video,
        on_frame: &mut dyn FnMut(Frame) -> Result<()>,
    ) -> Result<()> {
        let rgb = self.conv.convert(f)?;
        if self.emitted == 0 {
            self.info.width = rgb.width;
            self.info.height = rgb.height;
        }
        self.emitted += 1;
        self.next_t = t + self.cfg.interval;
        on_frame(Frame { t: t.max(0.0), rgb })
    }

    fn process_gop(
        &mut self,
        s: &mut Stream,
        gop: &[ff::Packet],
        end: Option<f64>,
        on_frame: &mut dyn FnMut(Frame) -> Result<()>,
    ) -> Result<()> {
        let first = &gop[0];
        let start = first
            .pts()
            .or(first.dts())
            .map(|t| s.time_of(t))
            .unwrap_or(self.next_t);
        let short = first.is_key() && end.is_some_and(|e| e - start <= self.cfg.max_gap);
        if short {
            // Keyframe only, and only if a sample is due before the next GOP starts.
            if end.is_some_and(|e| e <= self.next_t) && self.emitted > 0 {
                return Ok(());
            }
            if s.dec.send_packet(first).is_err() {
                return Ok(()); // corrupt keyframe: skip this GOP
            }
            let _ = s.dec.send_eof();
            let mut got: Option<(f64, ff::frame::Video)> = None;
            s.drain(&mut |t, f| {
                if got.is_none() {
                    got = Some((t, f.clone()));
                }
                Ok(true)
            })?;
            s.dec.flush();
            if let Some((t, f)) = got {
                self.emit(t, &f, on_frame)?;
            }
            return Ok(());
        }
        // Long GOP: decode in order until no further sample is due inside it.
        let mut ready: Vec<(f64, ff::frame::Video)> = Vec::new();
        for p in gop {
            if end.is_some_and(|e| self.next_t >= e) && self.emitted > 0 {
                break;
            }
            if s.dec.send_packet(p).is_err() {
                continue;
            }
            s.drain(&mut |t, f| {
                ready.push((t, f.clone()));
                Ok(true)
            })?;
            for (t, f) in ready.drain(..) {
                if self.due(t) {
                    self.emit(t, &f, on_frame)?;
                }
            }
        }
        let _ = s.dec.send_eof();
        s.drain(&mut |t, f| {
            ready.push((t, f.clone()));
            Ok(true)
        })?;
        for (t, f) in ready.drain(..) {
            if self.due(t) && end.is_none_or(|e| t < e) {
                self.emit(t, &f, on_frame)?;
            }
        }
        s.dec.flush();
        Ok(())
    }
}

/// Decodes the frame shown at time `t` (seconds), scaled so its shorter side is ≈`min_side`.
pub fn frame_at(path: &Path, t: f64, min_side: u32) -> Result<Rgb> {
    let mut s = open(path)?;
    let target = t.max(0.0);
    let ts = (target * f64::from(ff::ffi::AV_TIME_BASE)) as i64;
    // Seek to the keyframe at or before `t`; fall back to decoding from the start.
    let _ = s.ictx.seek(ts, ..ts + 1);
    let mut conv = Converter::new(min_side);
    let idx = s.index;
    let mut best: Option<ff::frame::Video> = None;
    let mut decoded = 0usize;
    loop {
        let mut pkt = ff::Packet::empty();
        match pkt.read(&mut s.ictx) {
            Ok(()) => {}
            Err(ff::Error::Eof) => break,
            Err(e) if is_retryable(&e) => continue,
            Err(e) => return Err(e).context("reading packet"),
        }
        if pkt.stream() != idx || s.dec.send_packet(&pkt).is_err() {
            continue;
        }
        let mut done = false;
        s.drain(&mut |ft, f| {
            decoded += 1;
            best = Some(f.clone());
            done = ft + 0.02 >= target || decoded > 2000;
            Ok(!done)
        })?;
        if done {
            break;
        }
    }
    if best.is_none() {
        let _ = s.dec.send_eof();
        s.drain(&mut |_, f| {
            best = Some(f.clone());
            Ok(true)
        })?;
    }
    let f = best.ok_or_else(|| anyhow!("no frame at {t:.1}s"))?;
    conv.convert(&f)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    /// Generates a test clip with ffmpeg. Left half red / right half blue with a moving bar,
    /// `secs` long at 10 fps, keyframe every `gop` frames. Extra args go before the output.
    pub fn make_clip(dir: &Path, name: &str, secs: u32, gop: u32, extra: &[&str]) -> PathBuf {
        let out = dir.join(name);
        let src = format!(
            "color=c=red:s=160x90:r=10:d={secs}[a];color=c=blue:s=160x90:r=10:d={secs}[b];\
             [a][b]hstack,drawbox=x='mod(t*40,300)':y=0:w=10:h=90:c=white:t=fill"
        );
        let status = Command::new("ffmpeg")
            .args(["-loglevel", "error", "-y", "-filter_complex", &src])
            .args([
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-g",
                &gop.to_string(),
                "-keyint_min",
                &gop.to_string(),
            ])
            .args(["-sc_threshold", "0"])
            .args(extra)
            .arg(&out)
            .status()
            .expect("ffmpeg must be installed to run video tests");
        assert!(status.success());
        out
    }

    fn sample_all(p: &Path, cfg: SamplerConfig) -> (Vec<f64>, VideoInfo) {
        let mut ts = Vec::new();
        let info = sample(p, cfg, 64, &|| false, &mut |f| {
            assert!(f.rgb.width.min(f.rgb.height) <= 90);
            ts.push(f.t);
            Ok(())
        })
        .unwrap();
        (ts, info)
    }

    #[test]
    fn short_gop_uses_keyframes_at_interval() {
        let d = tempfile::tempdir().unwrap();
        let p = make_clip(d.path(), "short.mp4", 20, 10, &[]); // keyframe every 1 s
        let (ts, info) = sample_all(&p, SamplerConfig::default());
        assert!((info.duration - 20.0).abs() < 0.5, "{}", info.duration);
        assert_eq!((info.width, info.height), (228, 64), "{info:?}");
        assert!(ts.len() >= 9 && ts.len() <= 11, "{ts:?}");
        assert!(
            ts.windows(2)
                .all(|w| w[1] - w[0] >= 1.9 && w[1] - w[0] <= 4.01),
            "{ts:?}"
        );
    }

    #[test]
    fn long_gop_still_samples_every_interval() {
        let d = tempfile::tempdir().unwrap();
        let p = make_clip(d.path(), "long.mp4", 20, 200, &[]); // one keyframe only
        let (ts, _) = sample_all(&p, SamplerConfig::default());
        assert!(ts.len() >= 9 && ts.len() <= 11, "{ts:?}");
        assert!(
            ts.windows(2).all(|w| (w[1] - w[0] - 2.0).abs() < 0.15),
            "{ts:?}"
        );
        assert!(ts[0] < 0.2);
    }

    #[test]
    fn frame_at_and_rotation_match_ffmpeg() {
        let d = tempfile::tempdir().unwrap();
        let p = make_clip(d.path(), "rot.mp4", 4, 10, &["-metadata:s:v", "rotate=0"]);
        let rot = d.path().join("rot90.mp4");
        let st = Command::new("ffmpeg")
            .args(["-loglevel", "error", "-y", "-display_rotation", "90", "-i"])
            .arg(&p)
            .args(["-c", "copy"])
            .arg(&rot)
            .status()
            .unwrap();
        assert!(st.success());
        let plain = frame_at(&p, 1.0, 90).unwrap();
        assert_eq!((plain.width, plain.height), (320, 90));
        let r = frame_at(&rot, 1.0, 90).unwrap();
        assert_eq!((r.width, r.height), (90, 320), "rotated display");
        // Compare with ffmpeg's own autorotation: which end of the tall frame is red?
        let png = d.path().join("ref.png");
        let st = Command::new("ffmpeg")
            .args(["-loglevel", "error", "-y", "-ss", "1", "-i"])
            .arg(&rot)
            .args(["-frames:v", "1"])
            .arg(&png)
            .status()
            .unwrap();
        assert!(st.success());
        let reference = image::open(&png).unwrap().to_rgb8();
        assert_eq!(reference.dimensions(), (90, 320));
        let ref_top_red = reference.get_pixel(45, 5)[0] > 128;
        let top_red = r.data[(5 * 90 + 45) * 3] > 128;
        assert_eq!(top_red, ref_top_red);
        let info = probe(&rot).unwrap();
        assert_eq!((info.width, info.height), (90, 320));
    }

    #[test]
    fn rotation_matrix_math() {
        // Identity.
        assert_eq!(
            rotation_from_matrix(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]),
            0
        );
        // 180°.
        assert_eq!(
            rotation_from_matrix(&[-1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0]),
            180
        );
        assert_eq!(rotation_from_matrix(&[0.0; 9]), 0);
    }

    #[test]
    fn garbage_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.mp4");
        std::fs::write(&p, b"definitely not a video").unwrap();
        assert!(sample(&p, SamplerConfig::default(), 64, &|| false, &mut |_| Ok(())).is_err());
        assert!(frame_at(&p, 0.0, 64).is_err());
    }

    #[test]
    fn cancel_stops_sampling() {
        let d = tempfile::tempdir().unwrap();
        let p = make_clip(d.path(), "c.mp4", 10, 10, &[]);
        let err = sample(&p, SamplerConfig::default(), 64, &|| true, &mut |_| Ok(())).unwrap_err();
        assert!(err.to_string().contains("cancelled"));
    }
}
