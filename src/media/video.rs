//! Video frame sampling with `FFmpeg` (see `docs/design.md`).
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

/// Seconds for a stream timestamp: `(ts - start) * tb + offset`. The subtraction saturates, so
/// a crafted start time cannot overflow.
fn stream_time(ts: i64, start: i64, tb: f64, offset: f64) -> f64 {
    ts.saturating_sub(start) as f64 * tb + offset
}

struct Stream {
    ictx: ff::format::context::Input,
    index: usize,
    tb: f64,
    start: i64,
    duration: f64,
    /// Nominal frame rate, used to time frames that carry no timestamp.
    fps: f64,
    last_t: Option<f64>,
    /// Added to all times after a backwards timestamp jump (spliced/concatenated streams) so
    /// sample times keep increasing.
    offset: f64,
    max_packet_t: f64,
    dec: ff::decoder::Video,
}

/// Opens a local media file for demuxing. Only the `file` protocol is allowed, so a crafted
/// file (e.g. a playlist disguised as .mp4) can never make `FFmpeg` touch the network.
pub(crate) fn open_input(path: &Path) -> Result<ff::format::context::Input> {
    init();
    let mut opts = ff::Dictionary::new();
    opts.set("protocol_whitelist", "file");
    ff::format::input_with_dictionary(path, opts)
        .with_context(|| format!("opening {}", path.display()))
}

/// Duration in seconds from the container header: the video stream's if it has one, else the
/// container's; 0 if unknown.
pub(crate) fn header_duration(
    ictx: &ff::format::context::Input,
    st: &ff::format::stream::Stream<'_>,
) -> f64 {
    let tb = f64::from(st.time_base());
    let mut duration = if st.duration() > 0 {
        st.duration() as f64 * tb
    } else {
        0.0
    };
    if !(duration.is_finite() && duration > 0.0) && ictx.duration() > 0 {
        duration = ictx.duration() as f64 / f64::from(ff::ffi::AV_TIME_BASE);
    }
    finite_positive(duration)
}

/// `x` if it is a finite number above zero, else 0 (a crafted header can yield NaN or
/// infinity, which would poison every later time computation).
pub(crate) fn finite_positive(x: f64) -> f64 {
    if x.is_finite() && x > 0.0 { x } else { 0.0 }
}

/// Validates a stream time base (seconds per tick): it must be a finite positive number.
fn checked_time_base(tb: f64) -> Result<f64> {
    if tb.is_finite() && tb > 0.0 {
        Ok(tb)
    } else {
        bail!("invalid stream time base")
    }
}

/// Opens a local video file and picks its best video stream.
fn open(path: &Path, threads: usize) -> Result<Stream> {
    let mut ictx = open_input(path)?;
    let st = ictx
        .streams()
        .best(ff::media::Type::Video)
        .ok_or_else(|| anyhow!("no video stream"))?;
    let index = st.index();
    let tb = checked_time_base(f64::from(st.time_base()))?;
    let start = if st.start_time() == ff::ffi::AV_NOPTS_VALUE {
        0
    } else {
        st.start_time()
    };
    let duration = header_duration(&ictx, &st);
    let rate = st.avg_frame_rate();
    let fps = if rate.numerator() > 0 && rate.denominator() > 0 {
        f64::from(rate)
    } else {
        25.0
    };
    let mut cctx = ff::codec::context::Context::from_parameters(st.parameters())?;
    cctx.set_threading(ff::codec::threading::Config {
        kind: ff::codec::threading::Type::Frame,
        count: threads, // 0 = auto
    });
    let dec = cctx
        .decoder()
        .video()
        .context("no decoder for this video codec")?;
    discard_other_streams(&mut ictx, index);
    Ok(Stream {
        ictx,
        index,
        tb,
        start,
        duration,
        fps,
        last_t: None,
        offset: 0.0,
        max_packet_t: 0.0,
        dec,
    })
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

/// Tells the demuxer to drop every stream except `keep`, so no time is spent on audio or
/// subtitle packets that would be thrown away anyway.
fn discard_other_streams(ictx: &mut ff::format::context::Input, keep: usize) {
    for i in 0..ictx.nb_streams() as usize {
        if i == keep {
            continue;
        }
        // `stream_mut` checks `i` against the stream count.
        let Some(mut stream) = ictx.stream_mut(i) else {
            continue;
        };
        // SAFETY: `stream` wraps an index that is below `nb_streams` of the open input, so the
        // pointer it returns is the valid, non-null `AVStream` at that index.
        let raw = unsafe { stream.as_mut_ptr() };
        // SAFETY: `raw` is valid (above) and nothing else can touch the stream: we hold
        // `&mut Input` through `stream`. Only the plain `discard` field is written.
        unsafe { (*raw).discard = ff::ffi::AVDiscard::AVDISCARD_ALL };
    }
}

/// Converts decoded frames to upright RGB at a requested size.
struct Converter {
    min_side: u32,
    scaler: Option<(
        ff::software::scaling::Context,
        (ff::format::Pixel, u32, u32),
    )>,
    out_dims: (u32, u32),
    /// Original display size (sample aspect ratio applied, before rotation).
    display: (u32, u32),
    rotation: u32,
    rotation_known: bool,
}

impl Converter {
    fn new(min_side: u32) -> Self {
        Self {
            min_side,
            scaler: None,
            out_dims: (0, 0),
            display: (0, 0),
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
fn frame_sizes(w: u32, h: u32, sar: f64, min_side: u32) -> ((u32, u32), (u32, u32)) {
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

fn rotation_from_matrix(m: &[f64]) -> u32 {
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

fn rotate(rgb: Rgb, cw: u32) -> Result<Rgb> {
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

impl Stream {
    fn time_of(&self, ts: i64) -> f64 {
        stream_time(ts, self.start, self.tb, self.offset)
    }

    /// Records a packet's time, detecting backwards jumps of more than a few seconds.
    fn note_packet_time(&mut self, p: &ff::Packet) {
        let Some(ts) = p.pts().or(p.dts()) else {
            return;
        };
        let t = self.time_of(ts);
        if t + 5.0 < self.max_packet_t {
            tracing::debug!(
                "timestamp discontinuity: {t:.1}s after {:.1}s",
                self.max_packet_t
            );
            self.offset += self.max_packet_t - t;
        } else {
            self.max_packet_t = self.max_packet_t.max(t);
        }
    }

    /// Reads the next packet of the video stream; `None` at end of file.
    fn next_packet(&mut self) -> Result<Option<ff::Packet>> {
        let mut again = 0;
        loop {
            let mut pkt = ff::Packet::empty();
            match pkt.read(&mut self.ictx) {
                Ok(()) if pkt.stream() == self.index => {
                    self.note_packet_time(&pkt);
                    return Ok(Some(pkt));
                }
                Ok(()) => {}
                Err(ff::Error::Eof) => return Ok(None),
                Err(e) if is_retryable(e) && again < 1000 => again += 1,
                Err(e) => return Err(e).context("reading packet"),
            }
        }
    }

    /// Sends a packet, draining pending frames first if the decoder is full.
    /// Returns false if the packet was rejected (corrupt data).
    fn send(
        &mut self,
        p: &ff::Packet,
        f: &mut dyn FnMut(f64, &ff::frame::Video) -> Result<bool>,
    ) -> Result<bool> {
        for _ in 0..3 {
            match self.dec.send_packet(p) {
                Ok(()) => return Ok(true),
                Err(e) if is_retryable(e) => {
                    self.drain(f)?;
                }
                Err(_) => return Ok(false),
            }
        }
        Ok(false)
    }

    /// Receives all frames currently available and calls `f(time, frame)` for each; stops
    /// early (returning false) when `f` returns false. Frames are lent, never copied.
    fn drain(&mut self, f: &mut dyn FnMut(f64, &ff::frame::Video) -> Result<bool>) -> Result<bool> {
        let mut frame = ff::frame::Video::empty();
        while self.dec.receive_frame(&mut frame).is_ok() {
            let t = match frame.timestamp() {
                Some(ts) => self.time_of(ts),
                // No timestamp: assume constant frame rate after the previous frame.
                None => self.last_t.map_or(0.0, |l| l + 1.0 / self.fps),
            };
            self.last_t = Some(t);
            if !f(t, &frame)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn packet_time(s: &Stream, p: &ff::Packet) -> Option<f64> {
    p.pts().or(p.dts()).map(|t| s.time_of(t))
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
    let mut s = open(path, 0)?;
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
    // True while decoding an oversized GOP packet by packet (decoder state must be kept).
    let mut streaming = false;
    loop {
        let pkt = match s.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(e) if st.emitted > 0 && !chain_is_io_failure(&e) => {
                // A damaged region late in a long file shouldn't discard everything before it.
                // A vanished drive is different: fail the file so it is retried in full.
                tracing::warn!("{}: stopping at a read error: {e:#}", path.display());
                break;
            }
            Err(e) => return Err(e),
        };
        if cancel() {
            bail!("cancelled");
        }
        if pkt.is_key() && streaming {
            st.finish_stream(&mut s, on_frame)?;
            streaming = false;
        }
        if streaming {
            st.decode_due(&mut s, &pkt, on_frame)?;
            continue;
        }
        if pkt.is_key() && !gop.is_empty() {
            let end = packet_time(&s, &pkt);
            st.process_gop(&mut s, &gop, end, on_frame)?;
            gop.clear();
            gop_bytes = 0;
        }
        gop_bytes += pkt.size();
        gop.push(pkt);
        if gop_bytes > st.cfg.max_gop_bytes {
            // Pathological GOP: decode what we have in order and keep going packet by packet.
            for p in &gop {
                st.decode_due(&mut s, p, on_frame)?;
            }
            gop.clear();
            gop_bytes = 0;
            streaming = true;
        }
    }
    if streaming {
        st.finish_stream(&mut s, on_frame)?;
    } else if !gop.is_empty() {
        // The last GOP ends at the known duration; if that is unknown, decode it in order.
        let end = (s.duration > 0.0).then_some(s.duration);
        st.process_gop(&mut s, &gop, end, on_frame)?;
    }
    if st.emitted == 0 {
        bail!("no frames could be decoded");
    }
    // Header durations can be missing or too short (crashed recordings); never report less
    // than what was actually decoded.
    st.info.duration = st.info.duration.max(st.next_t);
    Ok(st.info)
}

fn is_retryable(e: ff::Error) -> bool {
    matches!(e, ff::Error::Other { errno } if errno == ff::ffi::EAGAIN)
}

/// True when the error means the storage or network went away (as opposed to damaged data),
/// so the file should be retried later instead of being kept as it was read so far.
pub(crate) fn is_io_failure(e: ff::Error) -> bool {
    use ff::error::{
        ECONNABORTED, ECONNRESET, EHOSTUNREACH, EIO, ENETDOWN, ENETUNREACH, ENODEV, ENOENT,
        ENOTCONN, ENXIO, EPIPE, ETIMEDOUT,
    };
    const GONE: [i32; 12] = [
        EIO,
        ENOENT,
        ENODEV,
        ENXIO,
        ETIMEDOUT,
        ECONNRESET,
        ECONNABORTED,
        ENETDOWN,
        ENETUNREACH,
        EHOSTUNREACH,
        ENOTCONN,
        EPIPE,
    ];
    matches!(e, ff::Error::Other { errno } if GONE.contains(&errno))
}

/// Like [`is_io_failure`], looking through the context layers of an `anyhow` error.
pub(crate) fn chain_is_io_failure(e: &anyhow::Error) -> bool {
    e.chain()
        .filter_map(|c| c.downcast_ref::<ff::Error>())
        .any(|e| is_io_failure(*e))
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
            let (w, h) = self.conv.display;
            (self.info.width, self.info.height) = if self.conv.rotation % 180 == 90 {
                (h, w)
            } else {
                (w, h)
            };
        }
        self.emitted += 1;
        self.next_t = t + self.cfg.interval;
        on_frame(Frame { t: t.max(0.0), rgb })
    }

    /// Sends one packet and emits whichever decoded frames are due (and before `end`).
    fn decode_due_until(
        &mut self,
        s: &mut Stream,
        p: Option<&ff::Packet>,
        end: Option<f64>,
        on_frame: &mut dyn FnMut(Frame) -> Result<()>,
    ) -> Result<()> {
        let mut handle = |t: f64, f: &ff::frame::Video| -> Result<bool> {
            if self.due(t) && end.is_none_or(|e| t < e) {
                self.emit(t, f, on_frame)?;
            }
            Ok(true)
        };
        match p {
            Some(p) => {
                if !s.send(p, &mut handle)? {
                    return Ok(()); // corrupt packet: skip
                }
            }
            None => {
                let _ = s.dec.send_eof();
            }
        }
        s.drain(&mut handle)?;
        Ok(())
    }

    fn decode_due(
        &mut self,
        s: &mut Stream,
        p: &ff::Packet,
        on_frame: &mut dyn FnMut(Frame) -> Result<()>,
    ) -> Result<()> {
        self.decode_due_until(s, Some(p), None, on_frame)
    }

    fn finish_stream(
        &mut self,
        s: &mut Stream,
        on_frame: &mut dyn FnMut(Frame) -> Result<()>,
    ) -> Result<()> {
        self.decode_due_until(s, None, None, on_frame)?;
        s.dec.flush();
        Ok(())
    }

    fn process_gop(
        &mut self,
        s: &mut Stream,
        gop: &[ff::Packet],
        end: Option<f64>,
        on_frame: &mut dyn FnMut(Frame) -> Result<()>,
    ) -> Result<()> {
        let [first, ..] = gop else {
            return Ok(());
        };
        let start = packet_time(s, first).unwrap_or(self.next_t);
        let short = first.is_key() && end.is_some_and(|e| e - start <= self.cfg.max_gap);
        if short {
            // Keyframe only, and only if a sample is due before the next GOP starts.
            // ... and not right after the previous sample (e.g. after a long GOP).
            if self.emitted > 0
                && (end.is_some_and(|e| e <= self.next_t)
                    || start < self.next_t - self.cfg.interval / 2.0)
            {
                return Ok(());
            }
            let mut got = false;
            if s.dec.send_packet(first).is_ok() {
                let _ = s.dec.send_eof();
                let mut res = Ok(());
                s.drain(&mut |t, f| {
                    got = true;
                    res = self.emit(t, f, on_frame);
                    Ok(false) // the first frame is all we want
                })?;
                s.dec.flush();
                res?;
            }
            if got {
                return Ok(());
            }
            // The keyframe alone decoded to nothing (e.g. needs following packets): decode the
            // GOP in order instead.
        }
        // Long GOP: decode in order until no further sample is due inside it.
        for p in gop {
            if end.is_some_and(|e| self.next_t >= e) && self.emitted > 0 {
                break;
            }
            self.decode_due_until(s, Some(p), end, on_frame)?;
        }
        self.decode_due_until(s, None, end, on_frame)?;
        s.dec.flush();
        Ok(())
    }
}

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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    #[test]
    fn square_pixels_scale_the_shorter_side_down_to_the_limit() {
        let (display, out) = frame_sizes(1920, 1080, 1.0, 270);
        assert_eq!(display, (1920, 1080));
        assert_eq!(out, (480, 270));
    }

    #[test]
    fn anamorphic_pixels_widen_the_display_size() {
        let (display, out) = frame_sizes(720, 576, 64.0 / 45.0, 288);
        assert_eq!(display, (1024, 576));
        assert_eq!(out, (512, 288));
    }

    #[test]
    fn an_absurd_pixel_aspect_ratio_falls_back_to_square_pixels() {
        for sar in [100_000.0, 1.0 / 100_000.0, f64::NAN, f64::INFINITY] {
            let (display, out) = frame_sizes(1920, 1080, sar, 270);
            assert_eq!(display, (1920, 1080), "sar {sar}");
            assert_eq!(out, (480, 270), "sar {sar}");
        }
    }

    #[test]
    fn a_vanished_drive_counts_as_an_io_failure_but_damaged_data_does_not() {
        let io = ff::Error::Other {
            errno: ff::error::EIO,
        };
        let wrapped = anyhow::Error::new(io).context("reading packet");
        assert!(is_io_failure(io));
        assert!(chain_is_io_failure(&wrapped));
        assert!(!is_io_failure(ff::Error::InvalidData));
        assert!(!chain_is_io_failure(&anyhow::Error::new(
            ff::Error::InvalidData
        )));
        assert!(!chain_is_io_failure(&anyhow!("plain")));
    }

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
    fn hostile_header_values_are_contained() {
        // Time bases must be finite and positive.
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(checked_time_base(bad).is_err(), "{bad}");
        }
        assert_eq!(checked_time_base(1.0 / 90_000.0).unwrap(), 1.0 / 90_000.0);
        // Durations that are not finite and positive count as unknown.
        for bad in [0.0, -3.0, f64::NAN, f64::INFINITY] {
            assert_eq!(finite_positive(bad), 0.0, "{bad}");
        }
        assert_eq!(finite_positive(12.5), 12.5);
        // Extreme timestamps and start times do not overflow.
        assert!(stream_time(i64::MIN, i64::MAX, 1e-5, 0.0).is_finite());
        assert!(stream_time(i64::MAX, i64::MIN, 1e-5, 0.0).is_finite());
        assert_eq!(stream_time(100, 40, 0.5, 1.0), 31.0);
    }

    #[test]
    fn a_frame_buffer_that_does_not_match_its_size_is_an_error() {
        let ok = Rgb {
            width: 2,
            height: 1,
            data: vec![0; 6],
        };
        let turned = rotate(ok, 90).unwrap();
        assert_eq!((turned.width, turned.height), (1, 2));
        let short = Rgb {
            width: 2,
            height: 2,
            data: vec![0; 5],
        };
        assert!(rotate(short, 90).is_err());
    }

    #[test]
    fn short_gop_uses_keyframes_at_interval() {
        let d = tempfile::tempdir().unwrap();
        let p = make_clip(d.path(), "short.mp4", 20, 10, &[]); // keyframe every 1 s
        let (ts, info) = sample_all(&p, SamplerConfig::default());
        assert!((info.duration - 20.0).abs() < 0.5, "{}", info.duration);
        assert_eq!((info.width, info.height), (320, 90), "{info:?}");
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
        let (_, info) = sample_all(&rot, SamplerConfig::default());
        assert_eq!((info.width, info.height), (90, 320));
    }

    #[test]
    fn oversized_gop_streams_without_losing_frames() {
        let d = tempfile::tempdir().unwrap();
        let p = make_clip(d.path(), "big.mp4", 20, 200, &[]);
        let cfg = SamplerConfig {
            max_gop_bytes: 2_000,
            ..Default::default()
        };
        let (ts, _) = sample_all(&p, cfg);
        assert!(ts.len() >= 9 && ts.len() <= 11, "{ts:?}");
        assert!(
            ts.windows(2).all(|w| (w[1] - w[0] - 2.0).abs() < 0.15),
            "{ts:?}"
        );
    }

    #[test]
    fn single_gop_without_known_duration() {
        let d = tempfile::tempdir().unwrap();
        let p = make_clip(d.path(), "src.mp4", 20, 200, &[]);
        // Matroska written to a pipe has no duration and no cues.
        let out = d.path().join("nodur.mkv");
        let st = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "ffmpeg -loglevel error -i '{}' -c copy -f matroska - > '{}'",
                p.display(),
                out.display()
            ))
            .status()
            .unwrap();
        assert!(st.success());
        let (ts, info) = sample_all(&out, SamplerConfig::default());
        assert!(ts.len() >= 9, "{ts:?}");
        assert!(info.duration >= 18.0, "{info:?}");
    }

    #[test]
    fn frame_past_the_end_returns_the_last_frame() {
        let d = tempfile::tempdir().unwrap();
        let p = make_clip(d.path(), "e.mp4", 4, 10, &[]);
        let f = frame_at(&p, 100.0, 90).unwrap();
        assert_eq!((f.width, f.height), (320, 90));
    }

    #[test]
    fn concatenated_timestamps_keep_increasing() {
        let d = tempfile::tempdir().unwrap();
        let a = make_clip(d.path(), "a.ts", 10, 10, &["-f", "mpegts"]);
        // Byte-level concatenation: timestamps restart in the middle of the file.
        let joined = d.path().join("joined.ts");
        let mut bytes = std::fs::read(&a).unwrap();
        bytes.extend(std::fs::read(&a).unwrap());
        std::fs::write(&joined, bytes).unwrap();
        let (ts, _) = sample_all(&joined, SamplerConfig::default());
        assert!(ts.len() >= 8, "only {} samples: {ts:?}", ts.len());
        assert!(ts.windows(2).all(|w| w[1] > w[0]), "{ts:?}");
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
