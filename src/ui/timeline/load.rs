//! Keeps the list on screen up to date: reloads when the view, a filter or the library
//! changes, and while a scan is running.

use super::View;
use crate::library::Activity;
#[cfg(test)]
use crate::store::db::TimelineItem;
use crate::ui::App;
use std::time::{Duration, Instant};

/// Reload interval while a scan or indexing run keeps adding files.
pub(super) const LIVE_RELOAD: Duration = Duration::from_secs(10);
/// Wait before loading the same view again after a failed load.
pub(super) const RETRY_AFTER: Duration = Duration::from_secs(5);

impl App {
    /// Per-frame: starts reloads when what is shown or the library changed, applies results.
    pub(in crate::ui) fn update_timeline(&mut self) {
        if let Some(r) = self.tl.worker.poll()
            && self.tl.awaiting.as_ref().map(|a| a.0) == Some(r.id)
        {
            self.tl.awaiting = None;
            self.tl.requested_at = Instant::now();
            let same_view = self.tl.loaded_filter.as_ref() == Some(&r.filter);
            match r.items {
                Ok(mut items) => {
                    if r.filter.favorites_only {
                        // A list read before an unfavorite was saved still has the item.
                        items.retain(|it| self.fav_marks.get(&it.file_id).is_none_or(|m| m.value));
                    }
                    if same_view {
                        // A reload of what is shown: stay on the same photo.
                        self.tl.anchor_scroll(&items);
                    } else {
                        self.tl.grid.scroll_to = Some(0.0);
                        self.tl.grid.selected = None;
                        self.tl.pending = None;
                    }
                    self.tl.loaded_filter = Some(r.filter);
                    self.tl.error = None;
                    self.tl.failed = None;
                    self.tl.set_items(items);
                }
                Err(e) => {
                    if !same_view {
                        // Never keep showing another view's items.
                        self.tl.grid.selected = None;
                        self.tl.pending = None;
                        self.tl.grid.scroll_to = Some(0.0);
                        self.tl.loaded_filter = Some(r.filter.clone());
                        self.tl.set_items(Vec::new());
                    }
                    tracing::warn!("could not load the photos: {e}");
                    self.tl.error = Some(e);
                    self.tl.loaded = true;
                    self.tl.failed = Some((r.filter, Instant::now()));
                    self.tl.requested = None; // allow a retry
                }
            }
        }
        if let View::Folder(id) = self.tl.view
            && !self.snapshot.folders.is_empty()
            && !self.snapshot.folders.iter().any(|f| f.id == id)
        {
            self.set_view(View::Timeline);
        }
        let filter = self.tl.filter(self.kind);
        let jobs = self.lib.jobs_finished();
        // Files are being added: reload now and then. Embedding adds no files, so it does not
        // trigger reloads; the reload when the job finishes picks up its results.
        let growing = matches!(
            self.status.activity,
            Activity::Scanning { .. } | Activity::ReadingDates { .. }
        );
        let backing_off = self
            .tl
            .failed
            .as_ref()
            .is_some_and(|(f, at)| *f == filter && at.elapsed() < RETRY_AFTER);
        let stale = self.tl.requested.as_ref() != Some(&(filter.clone(), jobs)) && !backing_off;
        let live = growing && self.tl.requested_at.elapsed() > LIVE_RELOAD;
        if self.tl.awaiting.is_none() {
            if stale || live {
                let id = self.tl.worker.submit(filter.clone());
                self.tl.awaiting = Some((id, filter.clone()));
                self.tl.requested = Some((filter, jobs));
                self.tl.requested_at = Instant::now();
            }
        } else if stale {
            // The view or library changed while a load is in flight: load again once it is
            // done. (A live tick during a load does nothing.)
            self.tl.requested = None;
        }
        if backing_off {
            self.ctx.request_repaint_after(RETRY_AFTER);
        }
        if growing {
            self.ctx.request_repaint_after(LIVE_RELOAD);
        }
    }

    #[cfg(test)]
    pub(in crate::ui) fn inject_timeline(&mut self, items: Vec<TimelineItem>) {
        let filter = self.tl.filter(self.kind);
        self.tl.requested = Some((filter.clone(), self.lib.jobs_finished()));
        self.tl.loaded_filter = Some(filter);
        self.tl.set_items(items);
    }
}
