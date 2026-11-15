//! Video indexing: sample frames, embed them, merge similar consecutive frames into
//! segments, store one vector per segment.

use crate::indexer::{Embedded, Outcome, Progress, contain, fail_file, fit_for, write_items};
use crate::library::Library;
use crate::media::image as img;
use crate::media::video::{self, SamplerConfig, VideoInfo};
use crate::model::embedder::normalize;
use crate::model::{Device, VisionEncoder};
use crate::store::db::{FileMeta, FileRow, NewItem};
use anyhow::{Context, Result, anyhow};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Consecutive frames whose embedding has at least this cosine similarity with the first frame
/// of the current segment are merged into it (0.90 halved the number of vectors without
/// losing retrieval quality in our measurements).
pub const DEFAULT_SEGMENT_TAU: f32 = 0.90;
/// Longest segment, seconds; keeps moments in long static shots findable.
pub const MAX_SEGMENT_S: f64 = 60.0;

fn segment_tau() -> f32 {
    parse_segment_tau(std::env::var("RECOLLECT_SEGMENT_TAU").ok().as_deref())
}

/// The override when it is a finite similarity in `0.0..=1.0`, else the default.
fn parse_segment_tau(value: Option<&str>) -> f32 {
    value
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|t| t.is_finite() && (0.0..=1.0).contains(t))
        .unwrap_or(DEFAULT_SEGMENT_TAU)
}

/// Videos decoded at the same time. Their frames share embedding batches, which matters for
/// libraries of short clips (a 5 s clip gives ~3 frames; alone it would waste most of a GPU
/// batch and leave the GPU idle while the next file opens). On a CPU the encoder already uses
/// every core, so fewer decoders compete with it.
fn parallel_videos(device: Device) -> usize {
    match device {
        Device::Gpu => 3,
        Device::Cpu => 2,
    }
}

enum Msg {
    Frame(usize, f64, Vec<u8>),
    End(usize, Result<VideoInfo>),
}

struct FileState {
    merger: Merger,
    /// Frames sent by the sampler but not yet embedded.
    outstanding: usize,
    ended: Option<Result<VideoInfo>>,
}

impl FileState {
    fn new(interval: f64) -> Self {
        Self {
            merger: Merger::new(segment_tau(), interval),
            outstanding: 0,
            ended: None,
        }
    }
}

/// Records a sampler message: a frame waits in `pending` for the next embedding batch, an end
/// marks its file as fully sampled.
fn note_message(
    msg: Msg,
    states: &mut HashMap<usize, FileState>,
    pending: &mut Vec<(usize, f64, Vec<u8>)>,
    interval: f64,
) {
    let new_state = || FileState::new(interval);
    match msg {
        Msg::Frame(i, t, px) => {
            states.entry(i).or_insert_with(new_state).outstanding += 1;
            pending.push((i, t, px));
        }
        Msg::End(i, res) => states.entry(i).or_insert_with(new_state).ended = Some(res),
    }
}

pub(crate) fn index_videos(
    lib: &Library,
    enc: &mut VisionEncoder,
    files: &[FileRow],
    prog: &mut Progress,
) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let cfg = SamplerConfig::default();
    let batch = enc.batch_size();
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    std::thread::scope(|s| -> Result<()> {
        let (tx, rx) = crossbeam_channel::bounded::<Msg>(batch * 4);
        for n in 0..parallel_videos(enc.device()).min(files.len()) {
            let tx = tx.clone();
            let (next, stop) = (&next, &stop);
            std::thread::Builder::new()
                .name(format!("recollect-video-{n}"))
                .spawn_scoped(s, move || run_sampler(lib, files, cfg, next, stop, &tx))
                .context("spawning a video sampler thread")?;
        }
        drop(tx);
        let mut states: HashMap<usize, FileState> = HashMap::new();
        let mut pending: Vec<(usize, f64, Vec<u8>)> = Vec::with_capacity(batch);
        let mut result: Result<()> = Ok(());
        for msg in &rx {
            if lib.control.is_cancelled() {
                break;
            }
            note_message(msg, &mut states, &mut pending, cfg.interval);
            if pending.len() >= batch {
                result = embed_pending(lib, enc, &mut pending, &mut states, prog);
            }
            if result.is_ok() {
                result = finalize_ready(lib, files, &mut states, prog);
            }
            if result.is_err() {
                break;
            }
        }
        // Make the samplers stop (their sends fail) before the scope joins them.
        stop.store(true, Ordering::Relaxed);
        drop(rx);
        if result.is_ok() && !lib.control.is_cancelled() {
            result = embed_pending(lib, enc, &mut pending, &mut states, prog)
                .and_then(|()| finalize_ready(lib, files, &mut states, prog));
        }
        result
    })
}

/// A sampler thread: takes the next unclaimed video, samples its frames and sends them (then
/// an end message) to the embedder, until the videos run out or a stop is requested.
fn run_sampler(
    lib: &Library,
    files: &[FileRow],
    cfg: SamplerConfig,
    next: &AtomicUsize,
    stop: &AtomicBool,
    tx: &crossbeam_channel::Sender<Msg>,
) {
    let spec = lib.model();
    let fit = fit_for(spec);
    let halt = || stop.load(Ordering::Relaxed) || lib.control.checkpoint();
    loop {
        if halt() {
            return;
        }
        let i = next.fetch_add(1, Ordering::Relaxed);
        let Some(f) = files.get(i) else { return };
        // A decoder bug on one odd file must fail that file, not the whole run.
        let res = contain("the video decoder", || {
            video::sample(&f.path, cfg, spec.image_size, &halt, &mut |fr| {
                let px = img::to_square(&fr.rgb, spec.image_size, fit)?;
                tx.send(Msg::Frame(i, fr.t, px))
                    .map_err(|_| anyhow!("embedding stopped"))
            })
        });
        if tx.send(Msg::End(i, res)).is_err() {
            return;
        }
    }
}

fn embed_pending(
    lib: &Library,
    enc: &mut VisionEncoder,
    pending: &mut Vec<(usize, f64, Vec<u8>)>,
    states: &mut HashMap<usize, FileState>,
    prog: &mut Progress,
) -> Result<()> {
    if pending.is_empty() {
        return Ok(());
    }
    let px: Vec<&[u8]> = pending.iter().map(|p| p.2.as_slice()).collect();
    let embs = enc.embed(&px).context("embedding frames")?;
    crate::indexer::check_count("frames", embs.len(), pending.len())?;
    let n = pending.len();
    for ((i, t, _), e) in pending.drain(..).zip(embs) {
        if let Some(st) = states.get_mut(&i) {
            st.merger.push(t, e);
            st.outstanding -= 1;
        }
    }
    prog.items_done(lib, n);
    Ok(())
}

/// Stores every video whose sampler has finished and whose frames are all embedded.
fn finalize_ready(
    lib: &Library,
    files: &[FileRow],
    states: &mut HashMap<usize, FileState>,
    prog: &mut Progress,
) -> Result<()> {
    let ready: Vec<usize> = states
        .iter()
        .filter(|(_, st)| st.ended.is_some() && st.outstanding == 0)
        .map(|(i, _)| *i)
        .collect();
    for i in ready {
        let Some(st) = states.remove(&i) else {
            continue;
        };
        let (Some(f), Some(ended)) = (files.get(i), st.ended) else {
            continue;
        };
        match ended {
            Ok(info) => {
                let items = st.merger.finish(info.duration);
                let meta = FileMeta {
                    width: Some(info.width),
                    height: Some(info.height),
                    duration: Some(info.duration),
                };
                write_items(
                    lib,
                    vec![Embedded {
                        file: f.clone(),
                        meta,
                        items,
                    }],
                )?;
                prog.file_done(lib, f, 0, Outcome::Indexed); // frames were counted as they were embedded
            }
            // Cancelled: the file stays pending for the next run.
            Err(_) if lib.control.is_cancelled() => {}
            Err(e) => fail_file(lib, prog, f, &e),
        }
    }
    Ok(())
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Online segment merger: a frame joins the current segment when it is similar enough to the
/// segment's *first* frame (so slow drift still splits) and the segment is not too long. Each
/// segment stores the normalized mean of its frames. Only the open segment is kept in memory.
struct OpenSegment {
    start: f64,
    /// Time of the latest frame in the segment.
    last: f64,
    /// Embedding of the first frame (what later frames are compared with).
    first: Vec<f32>,
    /// Sum of the embeddings of all frames so far.
    sum: Vec<f32>,
}

pub(crate) struct Merger {
    tau: f32,
    interval: f64,
    open: Option<OpenSegment>,
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
        if let Some(seg) = &mut self.open
            && t - seg.start <= MAX_SEGMENT_S
            && dot(&emb, &seg.first) >= self.tau
        {
            seg.sum.iter_mut().zip(&emb).for_each(|(s, x)| *s += x);
            seg.last = t;
            return;
        }
        self.close(Some(t));
        self.open = Some(OpenSegment {
            start: t,
            last: t,
            first: emb.clone(),
            sum: emb,
        });
    }

    fn close(&mut self, next_start: Option<f64>) {
        if let Some(OpenSegment {
            start: t0,
            last,
            mut sum,
            ..
        }) = self.open.take()
        {
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
    fn the_segment_threshold_override_must_be_a_similarity() {
        assert_eq!(parse_segment_tau(Some("0.8")), 0.8);
        assert_eq!(parse_segment_tau(Some("0")), 0.0);
        for bad in ["NaN", "2", "-0.1", "abc", "inf", ""] {
            assert_eq!(parse_segment_tau(Some(bad)), DEFAULT_SEGMENT_TAU, "{bad}");
        }
        assert_eq!(parse_segment_tau(None), DEFAULT_SEGMENT_TAU);
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
        let frames: Vec<(f64, Vec<f32>)> =
            (0..4).map(|i| (f64::from(i) * 2.0, v(i as f32))).collect();
        let segs = merge_segments(&frames, 3.0, 2.0, 0.99);
        assert_eq!(segs.len(), 4);
        for s in &segs {
            assert!(s.t_end.unwrap() >= s.t_start.unwrap());
        }
        assert_eq!(segs[1].t_end, Some(4.0));
    }

    #[test]
    fn caps_segment_length_and_handles_edges() {
        let frames: Vec<(f64, Vec<f32>)> = (0..100).map(|i| (f64::from(i) * 2.0, v(0.0))).collect();
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
