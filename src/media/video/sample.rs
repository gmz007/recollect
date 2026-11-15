//! Sampling frames through a whole video, one GOP at a time.

use super::convert::Converter;
use super::stream::{Stream, chain_is_io_failure, open, packet_time};
use super::{Frame, SamplerConfig, VideoInfo};
use anyhow::{Result, bail};
use ffmpeg_next as ff;
use std::path::Path;

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
        let Some(pkt) = next_packet_or_stop(&mut s, st.emitted, path)? else {
            break;
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
    finish_sampling(&mut st, &mut s, &gop, streaming, on_frame)
}

/// The next packet, or `None` at the end of the stream. A read error after some frames were
/// emitted also ends the stream, unless the cause is an I/O failure.
fn next_packet_or_stop(s: &mut Stream, emitted: usize, path: &Path) -> Result<Option<ff::Packet>> {
    match s.next_packet() {
        Ok(Some(p)) => Ok(Some(p)),
        Ok(None) => Ok(None),
        Err(e) if emitted > 0 && !chain_is_io_failure(&e) => {
            // A damaged region late in a long file shouldn't discard everything before it.
            // A vanished drive is different: fail the file so it is retried in full.
            tracing::warn!("{}: stopping at a read error: {e:#}", path.display());
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Handles what is left once the packets run out: the oversized GOP being streamed, or the
/// last GOP. Fails if nothing was decoded.
fn finish_sampling(
    st: &mut SampleState,
    s: &mut Stream,
    gop: &[ff::Packet],
    streaming: bool,
    on_frame: &mut dyn FnMut(Frame) -> Result<()>,
) -> Result<VideoInfo> {
    if streaming {
        st.finish_stream(s, on_frame)?;
    } else if !gop.is_empty() {
        // The last GOP ends at the known duration; if that is unknown, decode it in order.
        let end = (s.duration > 0.0).then_some(s.duration);
        st.process_gop(s, gop, end, on_frame)?;
    }
    if st.emitted == 0 {
        bail!("no frames could be decoded");
    }
    // Header durations can be missing or too short (crashed recordings); never report less
    // than what was actually decoded.
    st.info.duration = st.info.duration.max(st.next_t);
    Ok(st.info)
}

pub(super) struct SampleState {
    pub(super) conv: Converter,
    pub(super) cfg: SamplerConfig,
    pub(super) next_t: f64,
    pub(super) emitted: usize,
    pub(super) info: VideoInfo,
}

impl SampleState {
    pub(super) fn due(&self, t: f64) -> bool {
        t + 1e-6 >= self.next_t || self.emitted == 0
    }

    pub(super) fn emit(
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
    pub(super) fn decode_due_until(
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

    pub(super) fn decode_due(
        &mut self,
        s: &mut Stream,
        p: &ff::Packet,
        on_frame: &mut dyn FnMut(Frame) -> Result<()>,
    ) -> Result<()> {
        self.decode_due_until(s, Some(p), None, on_frame)
    }

    pub(super) fn finish_stream(
        &mut self,
        s: &mut Stream,
        on_frame: &mut dyn FnMut(Frame) -> Result<()>,
    ) -> Result<()> {
        self.decode_due_until(s, None, None, on_frame)?;
        s.dec.flush();
        Ok(())
    }

    pub(super) fn process_gop(
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
