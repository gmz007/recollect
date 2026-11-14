//! Background helpers for the UI thread: a latest-wins search worker and a thumbnail loader.

use super::bg::LatestWorker;
use super::texture_loader::{Slot, TextureLoader};
use crate::Library;
use crate::library::SearchError;
use crate::search::{SearchHit, SearchOptions};
use crate::thumbs::{self, ThumbKey};
use std::path::Path;
use std::sync::Arc;

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

/// A search that could not run: what kind of failure it was, and the technical text (logged).
#[derive(Debug, Clone)]
pub struct SearchFailure {
    pub kind: SearchError,
    pub detail: String,
}

pub struct SearchResponse {
    pub id: u64,
    pub query: Query,
    pub hits: Result<Vec<SearchHit>, SearchFailure>,
}

/// Runs searches on a dedicated thread. Only the newest pending request is executed, so typing
/// quickly never queues up stale searches.
pub struct SearchWorker(LatestWorker<(Query, SearchOptions), SearchResponse>);

impl SearchWorker {
    pub fn new(lib: Arc<Library>, repaint: impl Fn() + Send + 'static) -> Self {
        Self(LatestWorker::spawn(
            "search",
            repaint,
            move |(query, opts)| match query {
                Query::Text(q) => lib.search(q, opts),
                Query::Similar { item_id, .. } => lib.similar(*item_id, opts),
            },
            |id, (query, _), outcome| SearchResponse {
                id,
                query,
                hits: match outcome {
                    Some(r) => r.map_err(|e| SearchFailure {
                        kind: SearchError::of(&e),
                        detail: format!("{e:#}"),
                    }),
                    None => Err(SearchFailure {
                        kind: SearchError::Other,
                        detail: "search failed unexpectedly".to_string(),
                    }),
                },
            },
        ))
    }

    /// Submits a search; returns its id.
    pub fn submit(&mut self, query: Query, opts: SearchOptions) -> u64 {
        self.0.submit((query, opts))
    }

    /// The newest finished search, if any arrived since the last call.
    pub fn poll(&self) -> Option<SearchResponse> {
        self.0.poll()
    }
}

/// Most requests are for what is on screen *now*, so the queue is LIFO and bounded.
const QUEUE_CAP: usize = 300;
/// Textures kept before the least recently used are evicted.
const MAX_TEXTURES: usize = 600;

/// Loads thumbnails on worker threads and keeps a bounded set of GPU textures.
pub struct ThumbCache {
    loader: TextureLoader<ThumbKey>,
}

impl ThumbCache {
    pub fn new(ctx: &egui::Context, cache_dir: &Path, workers: usize) -> Self {
        let dir = cache_dir.to_path_buf();
        let decode = move |key: &ThumbKey| {
            thumbs::load(key, &dir)
                .map_err(|e| tracing::debug!("thumbnail {}: {e:#}", key.path.display()))
                .ok()
        };
        let name = |key: &ThumbKey| format!("thumb:{}:{:?}", key.path.display(), key.t_ms);
        Self {
            loader: TextureLoader::new(ctx, "thumb", workers, decode, name),
        }
    }

    /// Call once per frame before drawing: uploads finished thumbnails and evicts old ones.
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        self.loader.upload_finished(ctx, true);
        let len = self.loader.states.len();
        if len > MAX_TEXTURES {
            self.loader.evict(len - MAX_TEXTURES, 2, |_| true);
        }
    }

    /// The texture for `key` if ready; schedules loading otherwise.
    pub fn get(&mut self, key: &ThumbKey) -> Option<&egui::TextureHandle> {
        let frame = self.loader.frame;
        if let Some(entry) = self.loader.states.get_mut(key) {
            entry.1 = frame;
        } else {
            self.request(key.clone());
            self.loader
                .states
                .insert(key.clone(), (Slot::Loading, frame));
            return None;
        }
        match self.loader.states.get(key) {
            Some((Slot::Ready(t), _)) => Some(t),
            _ => None,
        }
    }

    pub fn failed(&self, key: &ThumbKey) -> bool {
        matches!(self.loader.states.get(key), Some((Slot::Failed, _)))
    }

    fn request(&mut self, key: ThumbKey) {
        let dropped: Vec<ThumbKey> = {
            let mut q = self.loader.shared.queue.lock();
            q.push(key);
            // Forget the oldest requests; they are re-requested if still visible.
            let excess = q.len().saturating_sub(QUEUE_CAP);
            q.drain(..excess).collect()
        };
        for k in dropped {
            self.loader.states.remove(&k);
        }
        self.loader.shared.notify_one();
    }
}
