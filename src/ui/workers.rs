//! Background helpers for the UI thread: a latest-wins search worker and a thumbnail loader.

use crate::Library;
use crate::media::image::Rgb;
use crate::search::{SearchHit, SearchOptions};
use crate::thumbs::{self, ThumbKey};
use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::{Condvar, Mutex};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, PartialEq)]
pub enum Query {
    Text(String),
    Similar { item_id: i64, label: String },
}

impl Query {
    pub fn label(&self) -> String {
        match self {
            Query::Text(t) => t.clone(),
            Query::Similar { label, .. } => format!("similar to {label}"),
        }
    }
}

pub struct SearchResponse {
    pub id: u64,
    pub query: Query,
    pub hits: Result<Vec<SearchHit>, String>,
}

/// Runs searches on a dedicated thread. Only the newest pending request is executed, so typing
/// quickly never queues up stale searches.
pub struct SearchWorker {
    tx: Sender<(u64, Query, SearchOptions)>,
    rx: Receiver<SearchResponse>,
    next_id: u64,
}

impl SearchWorker {
    pub fn new(lib: Arc<Library>, repaint: impl Fn() + Send + 'static) -> Self {
        let (tx, req_rx) = unbounded::<(u64, Query, SearchOptions)>();
        let (resp_tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("recollect-search".into())
            .spawn(move || {
                while let Ok(mut req) = req_rx.recv() {
                    while let Ok(newer) = req_rx.try_recv() {
                        req = newer;
                    }
                    let (id, query, opts) = req;
                    // A bug in one search must not kill the worker for the rest of the session.
                    let run = std::panic::AssertUnwindSafe(|| match &query {
                        Query::Text(q) => lib.search(q, &opts),
                        Query::Similar { item_id, .. } => lib.similar(*item_id, &opts),
                    });
                    let hits = match std::panic::catch_unwind(run) {
                        Ok(r) => r.map_err(|e| format!("{e:#}")),
                        Err(_) => Err("search failed unexpectedly".to_string()),
                    };
                    if resp_tx.send(SearchResponse { id, query, hits }).is_err() {
                        break;
                    }
                    repaint();
                }
            })
            .expect("spawning search thread");
        Self { tx, rx, next_id: 0 }
    }

    /// Submits a search; returns its id.
    pub fn submit(&mut self, query: Query, opts: SearchOptions) -> u64 {
        self.next_id += 1;
        let _ = self.tx.send((self.next_id, query, opts));
        self.next_id
    }

    /// The newest finished search, if any arrived since the last call.
    pub fn poll(&self) -> Option<SearchResponse> {
        let mut last = None;
        while let Ok(r) = self.rx.try_recv() {
            last = Some(r);
        }
        last
    }
}

enum ThumbState {
    Loading,
    Ready(egui::TextureHandle),
    Failed,
}

struct Shared {
    queue: Mutex<Vec<ThumbKey>>,
    cv: Condvar,
    done: Mutex<Vec<(ThumbKey, Option<Rgb>)>>,
    stop: AtomicBool,
}

/// Most requests are for what is on screen *now*, so the queue is LIFO and bounded.
const QUEUE_CAP: usize = 300;

/// Loads thumbnails on worker threads and keeps a bounded set of GPU textures.
pub struct ThumbCache {
    shared: Arc<Shared>,
    states: HashMap<ThumbKey, (ThumbState, u64)>,
    frame: u64,
    max_textures: usize,
}

impl ThumbCache {
    pub fn new(ctx: &egui::Context, cache_dir: PathBuf, workers: usize) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Vec::new()),
            cv: Condvar::new(),
            done: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
        });
        for i in 0..workers.max(1) {
            let sh = Arc::clone(&shared);
            let ctx = ctx.clone();
            let dir = cache_dir.clone();
            std::thread::Builder::new()
                .name(format!("recollect-thumb-{i}"))
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
                            q.pop()
                        };
                        let Some(key) = key else { continue };
                        let res = thumbs::load(&key, &dir)
                            .map_err(|e| tracing::debug!("thumbnail {}: {e:#}", key.path.display()))
                            .ok();
                        sh.done.lock().push((key, res));
                        ctx.request_repaint();
                    }
                })
                .expect("spawning thumbnail thread");
        }
        Self {
            shared,
            states: HashMap::new(),
            frame: 0,
            max_textures: 600,
        }
    }

    /// Call once per frame before drawing: uploads finished thumbnails and evicts old ones.
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        let done: Vec<_> = std::mem::take(&mut *self.shared.done.lock());
        for (key, rgb) in done {
            let state = match rgb {
                Some(rgb) if rgb.width > 0 => {
                    let img = egui::ColorImage::from_rgb(
                        [rgb.width as usize, rgb.height as usize],
                        &rgb.data,
                    );
                    ThumbState::Ready(ctx.load_texture(
                        format!("thumb:{}:{:?}", key.path.display(), key.t_ms),
                        img,
                        egui::TextureOptions::LINEAR,
                    ))
                }
                Some(_) => ThumbState::Failed,
                None => ThumbState::Failed,
            };
            let used = self.states.get(&key).map(|s| s.1).unwrap_or(self.frame);
            self.states.insert(key, (state, used));
        }
        if self.states.len() > self.max_textures {
            let mut by_age: Vec<(u64, ThumbKey)> = self
                .states
                .iter()
                .filter(|(_, (s, _))| !matches!(s, ThumbState::Loading))
                .map(|(k, (_, f))| (*f, k.clone()))
                .collect();
            by_age.sort_by_key(|x| x.0);
            let excess = self.states.len() - self.max_textures;
            for (f, k) in by_age.into_iter().take(excess) {
                if f + 2 < self.frame {
                    self.states.remove(&k);
                }
            }
        }
    }

    /// The texture for `key` if ready; schedules loading otherwise.
    pub fn get(&mut self, key: &ThumbKey) -> Option<&egui::TextureHandle> {
        let frame = self.frame;
        if !self.states.contains_key(key) {
            self.request(key.clone());
            self.states
                .insert(key.clone(), (ThumbState::Loading, frame));
            return None;
        }
        let entry = self.states.get_mut(key)?;
        entry.1 = frame;
        match &entry.0 {
            ThumbState::Ready(t) => Some(t),
            _ => None,
        }
    }

    pub fn failed(&self, key: &ThumbKey) -> bool {
        matches!(self.states.get(key), Some((ThumbState::Failed, _)))
    }

    fn request(&mut self, key: ThumbKey) {
        let mut q = self.shared.queue.lock();
        q.push(key);
        if q.len() > QUEUE_CAP {
            // Forget the oldest requests; they are re-requested if still visible.
            let excess = q.len() - QUEUE_CAP;
            let dropped: Vec<ThumbKey> = q.drain(..excess).collect();
            for k in dropped {
                self.states.remove(&k);
            }
        }
        self.shared.cv.notify_one();
    }
}

impl Drop for ThumbCache {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.cv.notify_all();
    }
}
