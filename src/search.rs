//! Turning a query vector into ranked, per-file search hits.

use crate::library::Library;
use crate::media::MediaKind;
use crate::store::db::ItemInfo;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct SearchOptions {
    /// Maximum number of files returned.
    pub limit: usize,
    pub kind: Option<MediaKind>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            limit: 200,
            kind: None,
        }
    }
}

/// A matching moment inside a video.
#[derive(Debug, Clone, PartialEq)]
pub struct Moment {
    pub item_id: i64,
    pub t_start: f64,
    pub t_end: f64,
    pub score: f32,
}

/// One result: a file with its best-matching item (and, for videos, other good moments).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub item_id: i64,
    pub file_id: i64,
    pub path: PathBuf,
    pub kind: MediaKind,
    pub mtime: i64,
    pub score: f32,
    pub t_start: Option<f64>,
    pub t_end: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration: Option<f64>,
    /// For videos: further matching moments, best first (excluding the main one).
    pub moments: Vec<Moment>,
}

const MAX_MOMENTS: usize = 10;
/// Upper bound for candidates scanned when widening a search.
const MAX_CANDIDATES: usize = 50_000;

pub(crate) fn by_vector(
    lib: &Library,
    q: &[f32],
    opts: &SearchOptions,
    exclude_file: Option<i64>,
) -> Result<Vec<SearchHit>> {
    // Over-fetch: videos contribute many items per file, and re-scoring may reorder. If long
    // videos crowd the candidates (few distinct files), widen the scan.
    let mut k = (opts.limit * 4 + 100).min(5000);
    let cands = loop {
        let cands = {
            let index = lib.index.read();
            if q.len() != index.dim() {
                anyhow::bail!("the query was made with a different model; please search again");
            }
            index.search_excluding(q, k, opts.kind, exclude_file)
        };
        let files: HashSet<i64> = cands.iter().map(|c| c.file_id).collect();
        if files.len() >= opts.limit || cands.len() < k || k >= MAX_CANDIDATES {
            break cands;
        }
        k = (k * 4).min(MAX_CANDIDATES);
    };
    if cands.is_empty() {
        return Ok(Vec::new());
    }
    // Re-score only what can appear: the best few items of the first `limit` + margin files.
    let mut per_file: HashMap<i64, usize> = HashMap::new();
    let cands: Vec<_> = cands
        .into_iter()
        .filter(|c| {
            let n = per_file.len();
            let e = per_file.entry(c.file_id).or_insert(0);
            if *e == 0 && n >= opts.limit + 20 {
                return false;
            }
            *e += 1;
            *e <= MAX_MOMENTS + 1
        })
        .collect();
    let ids: Vec<i64> = cands.iter().map(|c| c.item_id).collect();
    let db = lib.db.lock();
    // Exact re-scoring with the stored f32 vectors.
    let embs = db.embeddings(&ids)?;
    let mut scored: Vec<(i64, i64, f32)> = cands
        .iter()
        .map(|c| {
            let s = embs.get(&c.item_id).map(|e| dot(e, q)).unwrap_or(c.score);
            (c.item_id, c.file_id, s)
        })
        .collect();
    scored.sort_by(|a, b| b.2.total_cmp(&a.2));
    let grouped = group_by_file(&scored, opts.limit);
    let all_ids: Vec<i64> = grouped
        .iter()
        .flat_map(|g| std::iter::once(g.0).chain(g.2.iter().map(|m| m.0)))
        .collect();
    let infos: HashMap<i64, ItemInfo> = db
        .item_info(&all_ids)?
        .into_iter()
        .map(|i| (i.item_id, i))
        .collect();
    drop(db);
    let moment_infos = &infos;
    Ok(grouped
        .into_iter()
        .filter_map(|(id, score, moments)| {
            let i = infos.get(&id)?;
            Some(SearchHit {
                item_id: id,
                file_id: i.file_id,
                path: i.path.clone(),
                kind: i.kind,
                mtime: i.mtime,
                score,
                t_start: i.t_start,
                t_end: i.t_end,
                width: i.width,
                height: i.height,
                duration: i.duration,
                moments: moments
                    .iter()
                    .filter_map(|&(mid, s)| {
                        let m = moment_infos.get(&mid)?;
                        Some(Moment {
                            item_id: mid,
                            t_start: m.t_start?,
                            t_end: m.t_end.or(m.t_start)?,
                            score: s,
                        })
                    })
                    .collect(),
            })
        })
        .collect())
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// (best item id, best score, further (item id, score) moments)
type Group = (i64, f32, Vec<(i64, f32)>);

/// Groups score-sorted `(item, file, score)` by file, preserving order of each file's best item.
/// Returns `(best item, best score, other moments)` for at most `limit` files.
fn group_by_file(sorted: &[(i64, i64, f32)], limit: usize) -> Vec<Group> {
    let mut order: Vec<Group> = Vec::new();
    let mut pos: HashMap<i64, usize> = HashMap::new();
    for &(item, file, score) in sorted {
        match pos.get(&file) {
            Some(&p) => {
                if order[p].2.len() < MAX_MOMENTS {
                    order[p].2.push((item, score));
                }
            }
            None => {
                if order.len() < limit {
                    pos.insert(file, order.len());
                    order.push((item, score, Vec::new()));
                }
            }
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouping_keeps_best_per_file_and_limit() {
        let sorted = vec![
            (1, 10, 0.9),
            (2, 20, 0.8),
            (3, 10, 0.7),
            (4, 30, 0.6),
            (5, 20, 0.5),
        ];
        let g = group_by_file(&sorted, 2);
        assert_eq!(g.len(), 2);
        assert_eq!((g[0].0, g[0].1), (1, 0.9));
        assert_eq!(g[0].2, vec![(3, 0.7)]);
        assert_eq!(g[1].0, 2);
        assert_eq!(g[1].2, vec![(5, 0.5)]);
    }
}
