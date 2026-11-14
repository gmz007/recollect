//! Large previews for the viewer, decoded on worker threads.
//!
//! Previews are decoded at (about) the window size, never larger than [`MAX_FIT_SIDE`] pixels
//! on the longest side, using the same DCT-scaled decoder as the indexer. A handful of textures
//! is kept in memory (least recently used first) and nothing is cached on disk. The viewer asks
//! for the neighbouring items ahead of time so stepping through a list feels instant; what is
//! on screen is always decoded before those prefetches ([`Previews::get_urgent`]).

use super::texture_loader::{Slot, TextureLoader};
use crate::Library;
use crate::media::image::{self as img, Rgb};
use crate::store::db::FileDetails;
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

/// Longest side of a preview that fits the window.
pub const MAX_FIT_SIDE: u32 = 2048;
/// Longest side of the sharper preview used at 100 % zoom.
pub const MAX_ZOOM_SIDE: u32 = 4096;
/// Textures kept in memory.
const MAX_TEXTURES: usize = 8;
/// Of those, at most this many are 100 % zoom textures (up to 4096 px a side, about 64 MB each).
const MAX_ZOOM_TEXTURES: usize = 2;

/// One decoded preview: a file version, a video position and the size it is decoded at.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PreviewKey {
    pub path: PathBuf,
    pub mtime: i64,
    /// Milliseconds into a video; `None` for images.
    pub t_ms: Option<i64>,
    /// The result is scaled down to fit this many pixels on its longest side.
    pub side: u32,
    /// The decoder may skip detail as long as the shorter side stays at least this big.
    pub min_side: u32,
    /// The sharper preview for 100 % zoom (as opposed to one that fits the window).
    pub zoom: bool,
}

/// Longest side to decode for a window of `window` points at `ppp` pixels per point: rounded
/// up to a multiple of 256 so small resizes keep using the same previews.
pub fn fit_side(window: egui::Vec2, ppp: f32) -> u32 {
    let px = (window.max_elem() * ppp).ceil().max(1.0) as u32;
    (px.div_ceil(256) * 256).clamp(1024, MAX_FIT_SIDE)
}

/// Shorter side needed so that the longest side reaches `side`, from the known dimensions.
pub fn min_side_for(dims: Option<(u32, u32)>, side: u32) -> u32 {
    match dims {
        Some((w, h)) if w > 0 && h > 0 => {
            let (long, short) = (w.max(h) as f32, w.min(h) as f32);
            ((side as f32 * short / long).ceil() as u32).clamp(64, side)
        }
        _ => side,
    }
}

fn decode_preview(key: &PreviewKey) -> anyhow::Result<Rgb> {
    let full = match key.t_ms {
        None => img::decode(&key.path, key.min_side)?.rgb,
        Some(t) => crate::media::video::frame_at(&key.path, t as f64 / 1000.0, key.min_side)?,
    };
    img::fit_within(&full, key.side)
}

/// Moves `key` to the back of the queue, which is where workers take their next request from.
fn promote(queue: &mut Vec<PreviewKey>, key: &PreviewKey) {
    if let Some(pos) = queue.iter().position(|k| k == key) {
        let k = queue.remove(pos);
        queue.push(k);
    }
}

/// What the viewer knows about a preview right now.
pub enum Preview<'a> {
    Loading,
    Ready(&'a egui::TextureHandle),
    Failed,
}

pub struct Previews {
    loader: TextureLoader<PreviewKey>,
    details: HashMap<i64, Option<FileDetails>>,
    /// Bumped whenever the details cache is emptied; answers to older requests are ignored.
    details_epoch: u64,
    /// What the cached details were read under (see [`Self::sync_details`]).
    details_stamp: Option<(u64, u64)>,
    details_tx: Sender<(i64, u64)>,
    details_rx: Receiver<(i64, u64, Option<FileDetails>)>,
}

impl Previews {
    pub fn new(ctx: &egui::Context, lib: Arc<Library>, workers: usize) -> Self {
        let decode = |key: &PreviewKey| {
            decode_preview(key)
                .map_err(|e| tracing::debug!("preview {}: {e:#}", key.path.display()))
                .ok()
        };
        let name = |key: &PreviewKey| {
            format!("preview:{}:{:?}:{}", key.path.display(), key.t_ms, key.side)
        };
        let loader = TextureLoader::new(ctx, "preview", workers, decode, name);
        // Reading details touches the database and the file system, so it happens off the UI
        // thread too.
        let (details_tx, req_rx) = unbounded::<(i64, u64)>();
        let (resp_tx, details_rx) = unbounded();
        let c = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("recollect-details".into())
            .spawn(move || {
                while let Ok((id, epoch)) = req_rx.recv() {
                    let d = lib.file_details(id).ok().flatten();
                    if resp_tx.send((id, epoch, d)).is_err() {
                        break;
                    }
                    c.request_repaint();
                }
            });
        if let Err(e) = spawned {
            // Details then never arrive; the viewer still shows the image.
            tracing::error!("could not start the details thread: {e}");
        }
        Self {
            loader,
            details: HashMap::new(),
            details_epoch: 0,
            details_stamp: None,
            details_tx,
            details_rx,
        }
    }

    /// Call once per frame before drawing: uploads finished previews and evicts old ones.
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        self.loader.upload_finished(ctx, false);
        while let Ok((id, epoch, d)) = self.details_rx.try_recv() {
            if epoch == self.details_epoch {
                self.details.insert(id, d);
            }
        }
        let finished = self.loader.finished();
        if finished > MAX_TEXTURES {
            self.loader.evict(finished - MAX_TEXTURES, 1, |_| true);
        }
        self.evict_zoom_textures();
    }

    /// Keeps at most [`MAX_ZOOM_TEXTURES`] zoom textures, dropping the least recently used.
    fn evict_zoom_textures(&mut self) {
        let total = self
            .loader
            .states
            .iter()
            .filter(|(k, (s, _))| k.zoom && !s.is_loading())
            .count();
        if total > MAX_ZOOM_TEXTURES {
            self.loader.evict(total - MAX_ZOOM_TEXTURES, 1, |k| k.zoom);
        }
    }

    /// The preview for `key`; schedules decoding if it is not known yet. Meant for prefetching:
    /// it goes behind everything asked for with [`Self::get_urgent`].
    pub fn get(&mut self, key: &PreviewKey) -> Preview<'_> {
        self.request(key, false)
    }

    /// Like [`Self::get`], for what is on screen: while the preview is not ready its request
    /// is moved to the front of the decode queue, in front of all prefetches (call it every
    /// frame; of several urgent keys, the one asked for last is decoded first).
    pub fn get_urgent(&mut self, key: &PreviewKey) -> Preview<'_> {
        self.request(key, true)
    }

    fn request(&mut self, key: &PreviewKey, urgent: bool) -> Preview<'_> {
        let frame = self.loader.frame;
        self.loader.requeue_if_due(key);
        let shared = &self.loader.shared;
        let entry = match self.loader.states.entry(key.clone()) {
            Entry::Vacant(v) => {
                shared.push(key.clone());
                v.insert((Slot::Loading, frame))
            }
            Entry::Occupied(o) => {
                let entry = o.into_mut();
                if urgent && entry.0.is_loading() {
                    promote(&mut shared.queue.lock(), key);
                }
                entry
            }
        };
        entry.1 = frame;
        match &entry.0 {
            Slot::Ready(t) => Preview::Ready(t),
            Slot::Loading => Preview::Loading,
            Slot::Failed(_) => Preview::Failed,
        }
    }

    /// Forgets queued (not yet started) requests that are not in `wanted`, so quickly stepping
    /// through a list does not decode everything that was passed over.
    pub fn retain_queued(&mut self, wanted: &[PreviewKey]) {
        let keep: HashSet<&PreviewKey> = wanted.iter().collect();
        let dropped: Vec<PreviewKey> = {
            let mut q = self.loader.shared.queue.lock();
            let (kept, dropped) = q.drain(..).partition(|k| keep.contains(k));
            *q = kept;
            dropped
        };
        for k in dropped {
            self.loader.states.remove(&k);
        }
    }

    /// Empties the decode queue and drops every texture and cached detail: for when the viewer
    /// goes away. Decodes already running finish, but their results are discarded.
    pub fn clear(&mut self) {
        self.loader.shared.queue.lock().clear();
        self.loader.states.clear();
        self.details.clear();
        self.details_epoch += 1;
        self.details_stamp = None;
    }

    /// Details of a file once they were read (requested on first use).
    pub fn details(&mut self, file_id: i64) -> Option<&FileDetails> {
        match self.details.entry(file_id) {
            Entry::Occupied(e) => e.into_mut().as_ref(),
            Entry::Vacant(e) => {
                e.insert(None);
                let _ = self.details_tx.send((file_id, self.details_epoch));
                None
            }
        }
    }

    /// Number of previews and details held (tests).
    #[cfg(test)]
    pub(super) fn cached(&self) -> usize {
        self.loader.states.len() + self.details.len() + self.loader.shared.queue.lock().len()
    }

    /// Drops the cached details when `stamp` (what the library looked like when they were
    /// read: the album generation and the number of finished jobs) is not the one they were
    /// read under. Call every frame while the viewer is open.
    pub fn sync_details(&mut self, stamp: (u64, u64)) {
        if self.details_stamp != Some(stamp) {
            if self.details_stamp.is_some() {
                self.details.clear();
                self.details_epoch += 1;
            }
            self.details_stamp = Some(stamp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn fit_side_follows_the_window_in_steps() {
        let v = |w, h| egui::vec2(w, h);
        assert_eq!(fit_side(v(1280.0, 800.0), 1.0), 1280);
        assert_eq!(fit_side(v(1290.0, 800.0), 1.0), 1536);
        assert_eq!(fit_side(v(300.0, 200.0), 1.0), 1024);
        assert_eq!(fit_side(v(3840.0, 2160.0), 1.0), MAX_FIT_SIDE);
        assert_eq!(fit_side(v(1600.0, 900.0), 2.0), MAX_FIT_SIDE);
    }

    #[test]
    fn min_side_keeps_the_longest_side_reachable() {
        assert_eq!(min_side_for(Some((4000, 3000)), 2000), 1500);
        assert_eq!(min_side_for(Some((3000, 4000)), 2000), 1500);
        assert_eq!(min_side_for(Some((8000, 1000)), 2048), 256);
        assert_eq!(min_side_for(Some((10000, 10)), 2048), 64);
        assert_eq!(min_side_for(None, 2048), 2048);
        assert_eq!(min_side_for(Some((0, 5)), 2048), 2048);
    }

    fn key(name: &str, zoom: bool) -> PreviewKey {
        PreviewKey {
            path: format!("/nowhere/{name}.jpg").into(),
            mtime: 1,
            t_ms: None,
            side: 1024,
            min_side: 1024,
            zoom,
        }
    }

    fn previews() -> (tempfile::TempDir, Previews) {
        let d = tempfile::tempdir().unwrap();
        crate::gpu::force_cpu(true);
        let lib = Library::open(
            crate::config::Paths::under(d.path()),
            Some(crate::model::registry::CLIP_B32.id),
        )
        .unwrap();
        let p = Previews::new(&egui::Context::default(), lib, 1);
        (d, p)
    }

    #[test]
    fn the_item_on_screen_goes_before_prefetches() {
        let mut q = vec![
            key("current", false),
            key("next", false),
            key("prev", false),
        ];
        // Workers pop from the back: the prefetches would come first.
        assert_eq!(q.last().unwrap(), &key("prev", false));
        promote(&mut q, &key("current", false));
        assert_eq!(q.pop().unwrap(), key("current", false));
        assert_eq!(q.pop().unwrap(), key("prev", false));
        // A key that is not queued (already being decoded) changes nothing.
        promote(&mut q, &key("gone", false));
        assert_eq!(q, [key("next", false)]);
    }

    #[test]
    fn clearing_forgets_queue_textures_and_details() {
        let (_d, mut p) = previews();
        p.loader.shared.queue.lock().push(key("queued", false));
        p.loader
            .states
            .insert(key("failed", false), (Slot::Failed(Instant::now()), 1));
        p.details.insert(7, None);
        p.clear();
        assert!(p.loader.shared.queue.lock().is_empty());
        assert!(p.loader.states.is_empty());
        assert!(p.details.is_empty());
    }

    #[test]
    fn only_two_zoom_textures_are_kept() {
        let (_d, mut p) = previews();
        p.loader.frame = 100;
        for (i, n) in ["a", "b", "c", "d"].iter().enumerate() {
            p.loader
                .states
                .insert(key(n, true), (Slot::Failed(Instant::now()), 10 + i as u64));
        }
        p.loader
            .states
            .insert(key("fit", false), (Slot::Failed(Instant::now()), 1));
        p.evict_zoom_textures();
        let mut left: Vec<_> = p
            .loader
            .states
            .keys()
            .filter(|k| k.zoom)
            .map(|k| k.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["c.jpg", "d.jpg"], "the most recently used stay");
        assert!(p.loader.states.contains_key(&key("fit", false)));
    }

    #[test]
    fn details_are_dropped_when_the_library_changes() {
        let (_d, mut p) = previews();
        p.sync_details((1, 1));
        p.details.insert(7, None);
        p.sync_details((1, 1));
        assert!(p.details.contains_key(&7));
        p.sync_details((2, 1));
        assert!(p.details.is_empty());
        p.details.insert(7, None);
        p.sync_details((2, 2));
        assert!(p.details.is_empty());
    }

    #[test]
    fn previews_are_scaled_to_the_requested_side() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("wide.png");
        image::RgbImage::from_fn(3000, 1500, |x, _| {
            image::Rgb([u8::try_from(x % 256).unwrap(), 7, 9])
        })
        .save(&p)
        .unwrap();
        let key = PreviewKey {
            path: p,
            mtime: 1,
            t_ms: None,
            side: 1024,
            min_side: min_side_for(Some((3000, 1500)), 1024),
            zoom: false,
        };
        let rgb = decode_preview(&key).unwrap();
        assert_eq!((rgb.width, rgb.height), (1024, 512));
        // Never upscaled.
        let small = PreviewKey { side: 4096, ..key };
        assert_eq!(decode_preview(&small).unwrap().width, 3000);
    }
}
