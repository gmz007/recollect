//! Opening a video file and reading its packets with sane timestamps.

use super::init;
use anyhow::{Context, Result, anyhow, bail};
use ffmpeg_next as ff;
use std::path::Path;

/// Seconds for a stream timestamp: `(ts - start) * tb + offset`. The subtraction saturates, so
/// a crafted start time cannot overflow.
pub(super) fn stream_time(ts: i64, start: i64, tb: f64, offset: f64) -> f64 {
    ts.saturating_sub(start) as f64 * tb + offset
}

pub(super) struct Stream {
    pub(super) ictx: ff::format::context::Input,
    pub(super) index: usize,
    pub(super) tb: f64,
    pub(super) start: i64,
    pub(super) duration: f64,
    /// Nominal frame rate, used to time frames that carry no timestamp.
    pub(super) fps: f64,
    pub(super) last_t: Option<f64>,
    /// Added to all times after a backwards timestamp jump (spliced/concatenated streams) so
    /// sample times keep increasing.
    pub(super) offset: f64,
    pub(super) max_packet_t: f64,
    pub(super) dec: ff::decoder::Video,
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
pub(super) fn checked_time_base(tb: f64) -> Result<f64> {
    if tb.is_finite() && tb > 0.0 {
        Ok(tb)
    } else {
        bail!("invalid stream time base")
    }
}

/// Opens a local video file and picks its best video stream.
pub(super) fn open(path: &Path, threads: usize) -> Result<Stream> {
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

/// Tells the demuxer to drop every stream except `keep`, so no time is spent on audio or
/// subtitle packets that would be thrown away anyway.
pub(super) fn discard_other_streams(ictx: &mut ff::format::context::Input, keep: usize) {
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

impl Stream {
    pub(super) fn time_of(&self, ts: i64) -> f64 {
        stream_time(ts, self.start, self.tb, self.offset)
    }

    /// Records a packet's time, detecting backwards jumps of more than a few seconds.
    pub(super) fn note_packet_time(&mut self, p: &ff::Packet) {
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
    pub(super) fn next_packet(&mut self) -> Result<Option<ff::Packet>> {
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
    pub(super) fn send(
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
    pub(super) fn drain(
        &mut self,
        f: &mut dyn FnMut(f64, &ff::frame::Video) -> Result<bool>,
    ) -> Result<bool> {
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

pub(super) fn packet_time(s: &Stream, p: &ff::Packet) -> Option<f64> {
    p.pts().or(p.dts()).map(|t| s.time_of(t))
}

pub(super) fn is_retryable(e: ff::Error) -> bool {
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

/// True when the file cannot be opened or its last bytes cannot be read right now. `FFmpeg`
/// demuxers report `EIO` for some truncated or garbled files too, so the errno alone cannot tell
/// a vanished drive from damaged data; reading the end of the file can.
pub(crate) fn storage_unreachable(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    const TAIL_PROBE_BYTES: i64 = 4096;
    let probe = || -> std::io::Result<()> {
        let mut file = std::fs::File::open(path)?;
        if file.seek(SeekFrom::End(-TAIL_PROBE_BYTES)).is_err() {
            // Shorter than the probe: read from the start instead.
            file.rewind()?;
        }
        std::io::copy(
            &mut file.by_ref().take(2 * TAIL_PROBE_BYTES as u64),
            &mut std::io::sink(),
        )?;
        Ok(())
    };
    probe().is_err()
}

/// Like [`is_io_failure`], looking through the context layers of an `anyhow` error.
pub(crate) fn chain_is_io_failure(e: &anyhow::Error) -> bool {
    e.chain()
        .filter_map(|c| c.downcast_ref::<ff::Error>())
        .any(|e| is_io_failure(*e))
}
