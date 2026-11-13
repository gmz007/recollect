//! Video indexing: sample frames (ADR 0004), embed them, merge similar consecutive frames into
//! segments, store one vector per segment.

use crate::indexer::{Progress, fit_for, write_items};
use crate::library::Library;
use crate::media::image as img;
use crate::media::video::{self, SamplerConfig};
use crate::model::VisionEncoder;
use crate::model::embedder::normalize;
use crate::store::db::{FileMeta, FileRow, NewItem};
use anyhow::Result;

/// Consecutive frames whose embedding has at least this cosine similarity with the first frame
/// of the current segment are merged into it (tuned on the workspace eval set, see M2 report).
pub const DEFAULT_SEGMENT_TAU: f32 = 0.90;
/// Longest segment, seconds; keeps moments in long static shots findable.
pub const MAX_SEGMENT_S: f64 = 60.0;

fn segment_tau() -> f32 {
    std::env::var("RECOLLECT_SEGMENT_TAU")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_SEGMENT_TAU)
}

pub(crate) fn index_videos(
    lib: &Library,
    enc: &mut VisionEncoder,
    files: &[FileRow],
    prog: &mut Progress,
) -> Result<()> {
    for f in files {
        if lib.control.checkpoint() {
            return Ok(());
        }
        match index_one(lib, enc, f, prog) {
            Ok(Some((meta, items))) => {
                write_items(lib, &mut vec![(f.clone(), meta, items)])?;
                prog.file_done(lib, f, 0, false); // frames were counted as they were embedded
            }
            Ok(None) => return Ok(()), // cancelled: file stays pending
            Err(e) if lib.control.is_cancelled() => {
                tracing::debug!("cancelled during {}: {e:#}", f.path.display());
                return Ok(());
            }
            Err(e) => {
                if is_fatal(&e) {
                    return Err(e);
                }
                let failed = crate::indexer::file_failed(lib, f, &e)?;
                prog.file_done(lib, f, 0, failed);
            }
        }
    }
    Ok(())
}

/// Errors from the model itself (not the file) should stop the run instead of marking every
/// remaining video as broken.
fn is_fatal(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.downcast_ref::<ModelError>().is_some())
}

#[derive(Debug)]
struct ModelError(String);

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "inference failed: {}", self.0)
    }
}

impl std::error::Error for ModelError {}

type Sampled = (f64, Vec<u8>);

fn index_one(
    lib: &Library,
    enc: &mut VisionEncoder,
    f: &FileRow,
    prog: &mut Progress,
) -> Result<Option<(FileMeta, Vec<NewItem>)>> {
    let spec = lib.model();
    let fit = fit_for(spec);
    let cfg = SamplerConfig::default();
    let batch = enc.batch_size();
    let (tx, rx) = crossbeam_channel::bounded::<Sampled>(batch * 2);
    // Segments are merged as embeddings arrive: memory stays flat for very long videos.
    let mut frames = Merger::new(segment_tau(), cfg.interval);
    let path = f.path.clone();
    let (info, embed_result) = std::thread::scope(|s| {
        let sampler = s.spawn(move || {
            video::sample(
                &path,
                cfg,
                spec.image_size,
                &|| lib.control.checkpoint(),
                &mut |fr| {
                    let px = img::to_square(&fr.rgb, spec.image_size, fit)?;
                    tx.send((fr.t, px))
                        .map_err(|_| anyhow::anyhow!("embedding stopped"))
                },
            )
        });
        let mut pending: Vec<Sampled> = Vec::with_capacity(batch);
        let mut embed = |pending: &mut Vec<Sampled>, frames: &mut Merger| -> Result<()> {
            if pending.is_empty() {
                return Ok(());
            }
            let px: Vec<&[u8]> = pending.iter().map(|p| p.1.as_slice()).collect();
            let embs = enc.embed(&px).map_err(|e| ModelError(format!("{e:#}")))?;
            for (p, e) in pending.drain(..).zip(embs) {
                frames.push(p.0, e);
            }
            Ok(())
        };
        let mut result = Ok(());
        for item in rx.iter() {
            pending.push(item);
            if pending.len() >= batch {
                let n = pending.len();
                result = embed(&mut pending, &mut frames);
                prog.items_done(lib, n);
                if result.is_err() {
                    break;
                }
            }
        }
        // Dropping the receiver makes the sampler's next send fail, so it stops decoding.
        drop(rx);
        if result.is_ok() {
            let n = pending.len();
            result = embed(&mut pending, &mut frames);
            prog.items_done(lib, n);
        }
        (
            sampler
                .join()
                .unwrap_or_else(|_| Err(anyhow::anyhow!("video decoder panicked"))),
            result,
        )
    });
    embed_result?;
    if lib.control.is_cancelled() {
        return Ok(None);
    }
    let info = info?;
    let items = frames.finish(info.duration);
    let meta = FileMeta {
        width: Some(info.width),
        height: Some(info.height),
        duration: Some(info.duration),
    };
    Ok(Some((meta, items)))
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Online segment merger: a frame joins the current segment when it is similar enough to the
/// segment's *first* frame (so slow drift still splits) and the segment is not too long. Each
/// segment stores the normalized mean of its frames. Only the open segment is kept in memory.
pub(crate) struct Merger {
    tau: f32,
    interval: f64,
    open: Option<(f64, f64, Vec<f32>, Vec<f32>)>, // (start, last frame t, first emb, sum)
    done: Vec<NewItem>,
}

impl Merger {
    pub fn new(tau: f32, interval: f64) -> Self {
        Self {
            tau,
            interval,
            open: None,
            done: Vec::new(),
        }
    }

    pub fn push(&mut self, t: f64, emb: Vec<f32>) {
        if let Some((t0, last, first, sum)) = &mut self.open
            && t - *t0 <= MAX_SEGMENT_S
            && dot(&emb, first) >= self.tau
        {
            sum.iter_mut().zip(&emb).for_each(|(s, x)| *s += x);
            *last = t;
            return;
        }
        self.close(Some(t));
        self.open = Some((t, t, emb.clone(), emb));
    }

    fn close(&mut self, next_start: Option<f64>) {
        if let Some((t0, last, _, mut sum)) = self.open.take() {
            normalize(&mut sum);
            let end = next_start.unwrap_or(last + self.interval).max(last).max(t0);
            self.done.push(NewItem {
                t_start: Some(t0),
                t_end: Some(end),
                embedding: sum,
            });
        }
    }

    /// Closes the last segment; `duration` (if known and plausible) caps its end.
    pub fn finish(mut self, duration: f64) -> Vec<NewItem> {
        self.close(None);
        if let Some(last) = self.done.last_mut()
            && duration > 0.0
            && let (Some(s), Some(e)) = (last.t_start, last.t_end)
        {
            last.t_end = Some(e.min(duration).max(s));
        }
        self.done
    }
}

/// Merges a complete list of frames.
#[cfg(test)]
pub(crate) fn merge_segments(
    frames: &[(f64, Vec<f32>)],
    duration: f64,
    interval: f64,
    tau: f32,
) -> Vec<NewItem> {
    let mut m = Merger::new(tau, interval);
    for (t, e) in frames {
        m.push(*t, e.clone());
    }
    m.finish(duration)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: f32) -> Vec<f32> {
        let mut x = vec![a.cos(), a.sin()];
        normalize(&mut x);
        x
    }

    #[test]
    fn merges_similar_runs_and_splits_on_change() {
        // Angles: three near 0, then two near 1.5 rad (a cut), then one far.
        let frames: Vec<(f64, Vec<f32>)> = [0.0, 0.05, 0.1, 1.5, 1.52, 3.0]
            .iter()
            .enumerate()
            .map(|(i, a)| (i as f64 * 2.0, v(*a)))
            .collect();
        let segs = merge_segments(&frames, 11.0, 2.0, 0.95);
        let spans: Vec<(f64, f64)> = segs
            .iter()
            .map(|s| (s.t_start.unwrap(), s.t_end.unwrap()))
            .collect();
        assert_eq!(spans, vec![(0.0, 6.0), (6.0, 10.0), (10.0, 11.0)]);
        // Mean embedding is unit length.
        let n: f32 = segs[0].embedding.iter().map(|x| x * x).sum();
        assert!((n - 1.0).abs() < 1e-5);
    }

    #[test]
    fn too_short_header_duration_never_makes_empty_segments() {
        let frames: Vec<(f64, Vec<f32>)> = (0..4).map(|i| (i as f64 * 2.0, v(i as f32))).collect();
        let segs = merge_segments(&frames, 3.0, 2.0, 0.99);
        assert_eq!(segs.len(), 4);
        for s in &segs {
            assert!(s.t_end.unwrap() >= s.t_start.unwrap());
        }
        assert_eq!(segs[1].t_end, Some(4.0));
    }

    #[test]
    fn caps_segment_length_and_handles_edges() {
        let frames: Vec<(f64, Vec<f32>)> = (0..100).map(|i| (i as f64 * 2.0, v(0.0))).collect();
        let segs = merge_segments(&frames, 200.0, 2.0, 0.9);
        assert!(segs.len() >= 3, "{}", segs.len());
        assert!(
            segs.iter()
                .all(|s| s.t_end.unwrap() - s.t_start.unwrap() <= MAX_SEGMENT_S + 2.0)
        );
        assert!(merge_segments(&[], 10.0, 2.0, 0.9).is_empty());
        let one = merge_segments(&[(3.0, v(0.0))], 0.0, 2.0, 0.9);
        assert_eq!((one[0].t_start, one[0].t_end), (Some(3.0), Some(5.0)));
    }
}
