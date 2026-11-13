//! Large previews for the viewer, decoded on worker threads.
//!
//! Previews are decoded at (about) the window size, never larger than [`MAX_FIT_SIDE`] pixels
//! on the longest side, using the same DCT-scaled decoder as the indexer. A handful of textures
//! is kept in memory (least recently used first) and nothing is cached on disk. The viewer asks
//! for the neighbouring items ahead of time so stepping through a list feels instant.

use crate::Library;
use crate::media::image::{self as img, Rgb};
use crate::media::metadata::DateSource;
use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::{Condvar, Mutex};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Longest side of a preview that fits the window.
pub const MAX_FIT_SIDE: u32 = 2048;
/// Longest side of the sharper preview used at 100 % zoom.
pub const MAX_ZOOM_SIDE: u32 = 4096;
/// Textures kept in memory.
const MAX_TEXTURES: usize = 8;

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

/// What the viewer knows about a preview right now.
pub enum Preview<'a> {
    Loading,
    Ready(&'a egui::TextureHandle),
    Failed,
}

enum State {
    Loading,
    Ready(egui::TextureHandle),
    Failed,
}

struct Shared {
    queue: Mutex<Vec<PreviewKey>>,
    cv: Condvar,
    done: Mutex<Vec<(PreviewKey, Option<Rgb>)>>,
    stop: AtomicBool,
}

/// Facts about a file that are read on demand (file size, and what list views do not carry).
#[derive(Debug, Clone, PartialEq)]
pub struct Details {
    pub taken_at: i64,
    pub date_source: DateSource,
    pub camera: Option<String>,
    pub size: Option<u64>,
}

pub struct Previews {
    shared: Arc<Shared>,
    states: HashMap<PreviewKey, (State, u64)>,
    frame: u64,
    details: HashMap<i64, Option<Details>>,
    details_tx: Sender<i64>,
    details_rx: Receiver<(i64, Option<Details>)>,
}

impl Previews {
    pub fn new(ctx: &egui::Context, lib: Arc<Library>, workers: usize) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Vec::new()),
            cv: Condvar::new(),
            done: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
        });
        for i in 0..workers.max(1) {
            let sh = Arc::clone(&shared);
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name(format!("recollect-preview-{i}"))
                .spawn(move || {
                    loop {
                        let key = {
                            let mut q = sh.queue.lock();
                            while q.is_empty() && !sh.stop.load(Ordering::Relaxed) {
                                sh.cv.wait(&mut q);
                            }
                            if sh.stop.load(Ordering::Relaxed) {
                                return;
                            }
                            // Newest request first: it is what the user is looking at.
                            q.pop()
                        };
                        let Some(key) = key else { continue };
                        let rgb = decode_preview(&key)
                            .map_err(|e| tracing::debug!("preview {}: {e:#}", key.path.display()))
                            .ok();
                        sh.done.lock().push((key, rgb));
                        ctx.request_repaint();
                    }
                })
                .expect("spawning preview thread");
        }
        // Reading details touches the database and the file system, so it happens off the UI
        // thread too.
        let (details_tx, req_rx) = unbounded::<i64>();
        let (resp_tx, details_rx) = unbounded();
        let c = ctx.clone();
        std::thread::Builder::new()
            .name("recollect-details".into())
            .spawn(move || {
                while let Ok(id) = req_rx.recv() {
                    let d = lib
                        .file_details(id)
                        .ok()
                        .flatten()
                        .map(|(d, size)| Details {
                            taken_at: d.taken_at,
                            date_source: d.date_source,
                            camera: d.camera,
                            size,
                        });
                    if resp_tx.send((id, d)).is_err() {
                        break;
                    }
                    c.request_repaint();
                }
            })
            .expect("spawning details thread");
        Self {
            shared,
            states: HashMap::new(),
            frame: 0,
            details: HashMap::new(),
            details_tx,
            details_rx,
        }
    }

    /// Call once per frame before drawing: uploads finished previews and evicts old ones.
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        let done: Vec<_> = std::mem::take(&mut *self.shared.done.lock());
        for (key, rgb) in done {
            if !self.states.contains_key(&key) {
                continue; // no longer wanted
            }
            let state = match rgb {
                Some(rgb) if rgb.width > 0 && rgb.height > 0 => {
                    let img = egui::ColorImage::from_rgb(
                        [rgb.width as usize, rgb.height as usize],
                        &rgb.data,
                    );
                    State::Ready(ctx.load_texture(
                        format!("preview:{}:{:?}:{}", key.path.display(), key.t_ms, key.side),
                        img,
                        egui::TextureOptions::LINEAR,
                    ))
                }
                _ => State::Failed,
            };
            let used = self.states.get(&key).map_or(self.frame, |s| s.1);
            self.states.insert(key, (state, used));
        }
        while let Ok((id, d)) = self.details_rx.try_recv() {
            self.details.insert(id, d);
        }
        let finished = self
            .states
            .values()
            .filter(|(s, _)| !matches!(s, State::Loading))
            .count();
        if finished > MAX_TEXTURES {
            let mut by_age: Vec<(u64, PreviewKey)> = self
                .states
                .iter()
                .filter(|(_, (s, f))| !matches!(s, State::Loading) && *f + 1 < self.frame)
                .map(|(k, (_, f))| (*f, k.clone()))
                .collect();
            by_age.sort_by_key(|x| x.0);
            for (_, k) in by_age.into_iter().take(finished - MAX_TEXTURES) {
                self.states.remove(&k);
            }
        }
    }

    /// The preview for `key`; schedules decoding if it is not known yet. The newest request is
    /// decoded first, so ask for neighbours before the current item.
    pub fn get(&mut self, key: &PreviewKey) -> Preview<'_> {
        let frame = self.frame;
        if !self.states.contains_key(key) {
            self.shared.queue.lock().push(key.clone());
            self.shared.cv.notify_one();
            self.states.insert(key.clone(), (State::Loading, frame));
        }
        let entry = self.states.get_mut(key).expect("state was just inserted");
        entry.1 = frame;
        match &entry.0 {
            State::Ready(t) => Preview::Ready(t),
            State::Loading => Preview::Loading,
            State::Failed => Preview::Failed,
        }
    }

    /// Forgets queued (not yet started) requests that are not in `wanted`, so quickly stepping
    /// through a list does not decode everything that was passed over.
    pub fn retain_queued(&mut self, wanted: &[PreviewKey]) {
        let keep: HashSet<&PreviewKey> = wanted.iter().collect();
        let dropped: Vec<PreviewKey> = {
            let mut q = self.shared.queue.lock();
            let (kept, dropped) = q.drain(..).partition(|k| keep.contains(k));
            *q = kept;
            dropped
        };
        for k in dropped {
            self.states.remove(&k);
        }
    }

    /// Details of a file once they were read (requested on first use).
    pub fn details(&mut self, file_id: i64) -> Option<&Details> {
        match self.details.entry(file_id) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut().as_ref(),
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(None);
                let _ = self.details_tx.send(file_id);
                None
            }
        }
    }
}

impl Drop for Previews {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.cv.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn previews_are_scaled_to_the_requested_side() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("wide.png");
        image::RgbImage::from_fn(3000, 1500, |x, _| image::Rgb([(x % 256) as u8, 7, 9]))
            .save(&p)
            .unwrap();
        let key = PreviewKey {
            path: p,
            mtime: 1,
            t_ms: None,
            side: 1024,
            min_side: min_side_for(Some((3000, 1500)), 1024),
        };
        let rgb = decode_preview(&key).unwrap();
        assert_eq!((rgb.width, rgb.height), (1024, 512));
        // Never upscaled.
        let small = PreviewKey { side: 4096, ..key };
        assert_eq!(decode_preview(&small).unwrap().width, 3000);
    }
}
