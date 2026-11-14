//! What the viewer shows: one entry of the list it was opened on.

use crate::media::MediaKind;
use crate::media::metadata::DateSource;
use crate::search::SearchHit;
use crate::store::db::TimelineItem;
use crate::thumbs::ThumbKey;
use crate::ui::preview::{self, PreviewKey};
use std::path::PathBuf;
use std::sync::Arc;

/// A matching moment of a video (from a search hit).
#[derive(Debug, Clone, PartialEq)]
pub(in crate::ui) struct MomentChip {
    pub item_id: i64,
    pub t_start: f64,
    pub t_end: f64,
    pub score: f32,
}

/// One item of the list being viewed, from a timeline row or a search hit.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::ui) struct ViewerEntry {
    pub file_id: i64,
    /// A searchable item of the file, if it is indexed (what "Find similar" starts from).
    pub item_id: Option<i64>,
    pub path: PathBuf,
    pub kind: MediaKind,
    pub mtime: i64,
    pub taken_at: i64,
    /// Where `taken_at` comes from, when the list knew it (search hits do not).
    pub date_source: Option<DateSource>,
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub favorite: bool,
    pub camera: Option<String>,
    /// Video position shown, in seconds: the matching moment of a search hit.
    pub t: Option<f64>,
    /// All matching moments (best first) when opened from a search hit.
    pub moments: Vec<MomentChip>,
}

impl ViewerEntry {
    pub fn from_timeline(it: &TimelineItem) -> Self {
        Self {
            file_id: it.file_id,
            item_id: it.item_id,
            path: it.path.clone(),
            kind: it.kind,
            mtime: it.mtime,
            taken_at: it.taken_at,
            date_source: Some(it.date_source),
            duration: it.duration,
            width: it.width,
            height: it.height,
            favorite: it.favorite,
            camera: it.camera.clone(),
            t: None,
            moments: Vec::new(),
        }
    }

    pub fn from_hit(h: &SearchHit) -> Self {
        let mut moments = Vec::new();
        if h.kind == MediaKind::Video {
            moments.push(MomentChip {
                item_id: h.item_id,
                t_start: h.t_start.unwrap_or(0.0),
                t_end: h.t_end.or(h.t_start).unwrap_or(0.0),
                score: h.score,
            });
            moments.extend(h.moments.iter().map(|m| MomentChip {
                item_id: m.item_id,
                t_start: m.t_start,
                t_end: m.t_end,
                score: m.score,
            }));
        }
        Self {
            file_id: h.file_id,
            item_id: Some(h.item_id),
            path: h.path.clone(),
            kind: h.kind,
            mtime: h.mtime,
            taken_at: h.taken_at,
            date_source: None,
            duration: h.duration,
            width: h.width,
            height: h.height,
            favorite: h.favorite,
            camera: None,
            t: h.t_start,
            moments,
        }
    }

    pub(super) fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Position of the frame shown for a video: the chosen moment, else 10 % of the length
    /// (at most 5 s), like the timeline thumbnail.
    pub(super) fn frame_time(&self) -> Option<f64> {
        (self.kind == MediaKind::Video).then(|| {
            self.t
                .unwrap_or_else(|| (self.duration.unwrap_or(0.0) * 0.1).clamp(0.0, 5.0))
        })
    }

    pub(super) fn thumb_key(&self) -> ThumbKey {
        ThumbKey {
            path: self.path.clone(),
            mtime: self.mtime,
            t_ms: self.frame_time().map(crate::ui::to_millis),
        }
    }

    pub(super) fn dims(&self) -> Option<(u32, u32)> {
        self.width.zip(self.height)
    }

    pub(super) fn preview_key(&self, side: u32, zoom: bool) -> PreviewKey {
        PreviewKey {
            path: self.path.clone(),
            mtime: self.mtime,
            t_ms: self.frame_time().map(crate::ui::to_millis),
            side,
            min_side: preview::min_side_for(self.dims(), side),
            zoom,
        }
    }

    /// The item "Find similar" should start from: the chosen moment's, for videos.
    pub(super) fn similar_item(&self) -> Option<i64> {
        self.moments
            .iter()
            .find(|m| Some(m.t_start) == self.t)
            .map(|m| m.item_id)
            .or(self.item_id)
    }
}

/// The list a viewer was opened on.
#[derive(Clone)]
pub(in crate::ui) enum Source {
    /// The album (shared with the timeline).
    Timeline(Arc<Vec<TimelineItem>>),
    /// Search results (shared with the results grid).
    Results(Arc<Vec<SearchHit>>),
    /// A short list built for the occasion (the "On this day" strip).
    Entries(Vec<ViewerEntry>),
}

impl Source {
    pub(super) fn len(&self) -> usize {
        match self {
            Source::Timeline(v) => v.len(),
            Source::Results(v) => v.len(),
            Source::Entries(v) => v.len(),
        }
    }

    /// The entry at `i` (`i < len`), built on demand.
    pub(super) fn entry(&self, i: usize) -> ViewerEntry {
        match self {
            Source::Timeline(v) => ViewerEntry::from_timeline(&v[i]),
            Source::Results(v) => ViewerEntry::from_hit(&v[i]),
            Source::Entries(v) => v[i].clone(),
        }
    }
}
