//! Shared machinery of the thumbnail and preview loaders: decode requests are served by a
//! small pool of worker threads (newest request first), finished images are uploaded as
//! textures on the UI thread, and old textures are evicted. Each loader adds its own policy
//! on top (queue limits, urgency, which textures to keep).

use crate::media::image::Rgb;
use parking_lot::{Condvar, Mutex};
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) enum Slot {
    Loading,
    Ready(egui::TextureHandle),
    Failed,
}

impl Slot {
    pub(super) fn is_loading(&self) -> bool {
        matches!(self, Slot::Loading)
    }
}

pub(super) struct Shared<K> {
    pub(super) queue: Mutex<Vec<K>>,
    cv: Condvar,
    done: Mutex<Vec<(K, Option<Rgb>)>>,
    stop: AtomicBool,
}

pub(super) struct TextureLoader<K> {
    pub(super) shared: Arc<Shared<K>>,
    /// Each known key with its state and the frame it was last asked for.
    pub(super) states: HashMap<K, (Slot, u64)>,
    pub(super) frame: u64,
    texture_name: fn(&K) -> String,
}

impl<K> Shared<K> {
    pub(super) fn notify_one(&self) {
        self.cv.notify_one();
    }

    /// Queues `key` for decoding and wakes a worker.
    pub(super) fn push(&self, key: K) {
        self.queue.lock().push(key);
        self.cv.notify_one();
    }
}

impl<K: Clone + Eq + Hash + Send + 'static> TextureLoader<K> {
    /// Starts `workers` threads (at least one is attempted) that decode with `decode`;
    /// `None` marks a failure. `texture_name` labels the uploaded textures.
    pub(super) fn new(
        ctx: &egui::Context,
        name: &str,
        workers: usize,
        decode: impl Fn(&K) -> Option<Rgb> + Send + Sync + 'static,
        texture_name: fn(&K) -> String,
    ) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Vec::new()),
            cv: Condvar::new(),
            done: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
        });
        let decode = Arc::new(decode);
        for i in 0..workers.max(1) {
            let sh = Arc::clone(&shared);
            let ctx = ctx.clone();
            let decode = Arc::clone(&decode);
            let spawned = std::thread::Builder::new()
                .name(format!("recollect-{name}-{i}"))
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
                        let rgb = decode(&key);
                        sh.done.lock().push((key, rgb));
                        ctx.request_repaint();
                    }
                });
            if let Err(e) = spawned {
                // Carry on with the workers that did start.
                tracing::error!("could not start a {name} thread: {e}");
            }
        }
        Self {
            shared,
            states: HashMap::new(),
            frame: 0,
            texture_name,
        }
    }

    /// Starts a frame: counts it and uploads the images that finished. With `keep_unknown`
    /// an image nobody waits for any more still gets a slot; otherwise it is dropped.
    pub(super) fn upload_finished(&mut self, ctx: &egui::Context, keep_unknown: bool) {
        self.frame += 1;
        let done: Vec<_> = std::mem::take(&mut *self.shared.done.lock());
        for (key, rgb) in done {
            if !keep_unknown && !self.states.contains_key(&key) {
                continue;
            }
            let slot = match rgb {
                Some(rgb) if rgb.width > 0 && rgb.height > 0 => {
                    let img = egui::ColorImage::from_rgb(
                        [rgb.width as usize, rgb.height as usize],
                        &rgb.data,
                    );
                    Slot::Ready(ctx.load_texture(
                        (self.texture_name)(&key),
                        img,
                        egui::TextureOptions::LINEAR,
                    ))
                }
                _ => Slot::Failed,
            };
            let used = self.states.get(&key).map_or(self.frame, |s| s.1);
            self.states.insert(key, (slot, used));
        }
    }

    /// Drops up to `count` finished textures matching `wanted`, least recently used first,
    /// sparing those asked for within the last `idle_frames` frames.
    pub(super) fn evict(&mut self, count: usize, idle_frames: u64, wanted: impl Fn(&K) -> bool) {
        let mut by_age: Vec<(u64, K)> = self
            .states
            .iter()
            .filter(|(k, (s, f))| !s.is_loading() && f + idle_frames < self.frame && wanted(k))
            .map(|(k, (_, f))| (*f, k.clone()))
            .collect();
        by_age.sort_by_key(|x| x.0);
        for (_, k) in by_age.into_iter().take(count) {
            self.states.remove(&k);
        }
    }

    /// Number of slots that are not waiting for a decode.
    pub(super) fn finished(&self) -> usize {
        self.states
            .values()
            .filter(|(s, _)| !s.is_loading())
            .count()
    }
}

impl<K> Drop for TextureLoader<K> {
    fn drop(&mut self) {
        // Under the queue lock: a worker that just checked `stop` is then either still before
        // its wait (and sees the flag when it rechecks) or already waiting (and is woken).
        let _queue = self.shared.queue.lock();
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.cv.notify_all();
    }
}
