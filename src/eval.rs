//! Search-quality evaluation against a query set (format: workspace `testdata/eval/README.md`).
//!
//! - caption queries (one expected image): Recall@1/5/10 and MRR
//! - category queries (many expected images): precision@10
//! - video queries (expected time ranges): Hit@1/5/10 — a result counts if it is the right
//!   video and its best moment overlaps an expected range (±2 s tolerance)

use crate::library::Library;
use crate::media::MediaKind;
use crate::search::{SearchHit, SearchOptions};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const TOLERANCE_S: f64 = 2.0;

#[derive(Debug, Deserialize)]
struct Query {
    id: String,
    query: String,
    kind: String,
    #[serde(default)]
    source: String,
    expected: Vec<serde_json::Value>,
}

#[derive(Debug, Clone)]
enum Expected {
    Image(PathBuf),
    Range { path: PathBuf, start: f64, end: f64 },
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Report {
    pub captions: usize,
    pub caption_r1: f64,
    pub caption_r5: f64,
    pub caption_r10: f64,
    pub caption_mrr: f64,
    pub categories: usize,
    pub category_p10: f64,
    pub videos: usize,
    pub video_hit1: f64,
    pub video_hit5: f64,
    pub video_hit10: f64,
    /// Queries whose expected files are not in the library (skipped).
    pub skipped: usize,
    /// Per-query rank of the first correct answer (None = not in top 10).
    pub ranks: Vec<(String, Option<usize>)>,
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "captions   n={:<4} R@1={:.3} R@5={:.3} R@10={:.3} MRR={:.3}",
            self.captions, self.caption_r1, self.caption_r5, self.caption_r10, self.caption_mrr
        )?;
        writeln!(
            f,
            "categories n={:<4} P@10={:.3}",
            self.categories, self.category_p10
        )?;
        writeln!(
            f,
            "videos     n={:<4} Hit@1={:.3} Hit@5={:.3} Hit@10={:.3}",
            self.videos, self.video_hit1, self.video_hit5, self.video_hit10
        )?;
        write!(
            f,
            "skipped    {} (expected files not indexed)",
            self.skipped
        )
    }
}

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn parse_expected(root: &Path, q: &Query) -> Vec<Expected> {
    q.expected
        .iter()
        .filter_map(|e| match e {
            serde_json::Value::String(s) => Some(Expected::Image(canon(&root.join(s)))),
            serde_json::Value::Object(o) => Some(Expected::Range {
                path: canon(&root.join(o.get("path")?.as_str()?)),
                start: o.get("start")?.as_f64()?,
                end: o.get("end")?.as_f64()?,
            }),
            _ => None,
        })
        .collect()
}

fn matches(hit: &SearchHit, exp: &[Expected]) -> bool {
    exp.iter().any(|e| match e {
        Expected::Image(p) => &hit.path == p,
        Expected::Range { path, start, end } => {
            let (s, t) = (
                hit.t_start.unwrap_or(0.0),
                hit.t_end.or(hit.t_start).unwrap_or(0.0),
            );
            &hit.path == path && s <= end + TOLERANCE_S && t >= start - TOLERANCE_S
        }
    })
}

pub fn run(lib: &Library, queries: &Path, root: &Path) -> Result<Report> {
    let text = std::fs::read_to_string(queries)
        .with_context(|| format!("reading {}", queries.display()))?;
    let qs: Vec<Query> = serde_json::from_str(&text).context("parsing queries")?;
    let mut rep = Report::default();
    let (mut r1, mut r5, mut r10, mut mrr, mut p10, mut h1, mut h5, mut h10) =
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for q in &qs {
        let exp = parse_expected(root, q);
        let kind = if q.kind == "video" {
            MediaKind::Video
        } else {
            MediaKind::Image
        };
        let present = exp.iter().any(|e| match e {
            Expected::Image(p) | Expected::Range { path: p, .. } => p.exists(),
        });
        if exp.is_empty() || !present {
            rep.skipped += 1;
            continue;
        }
        let hits = lib.search(
            &q.query,
            &SearchOptions {
                limit: 10,
                kind: Some(kind),
            },
        )?;
        let rank = hits.iter().position(|h| matches(h, &exp));
        rep.ranks.push((q.id.clone(), rank));
        let at = |k: usize| f64::from(u8::from(rank.is_some_and(|r| r < k)));
        match (kind, q.source.as_str()) {
            (MediaKind::Video, _) => {
                rep.videos += 1;
                h1 += at(1);
                h5 += at(5);
                h10 += at(10);
            }
            (_, "coco-category") => {
                rep.categories += 1;
                let good = hits.iter().filter(|h| matches(h, &exp)).count() as f64;
                p10 += good / (exp.len().min(10) as f64);
            }
            _ => {
                rep.captions += 1;
                r1 += at(1);
                r5 += at(5);
                r10 += at(10);
                mrr += rank.map(|r| 1.0 / (r as f64 + 1.0)).unwrap_or(0.0);
            }
        }
    }
    let div = |a: f64, n: usize| if n == 0 { 0.0 } else { a / n as f64 };
    rep.caption_r1 = div(r1, rep.captions);
    rep.caption_r5 = div(r5, rep.captions);
    rep.caption_r10 = div(r10, rep.captions);
    rep.caption_mrr = div(mrr, rep.captions);
    rep.category_p10 = div(p10, rep.categories);
    rep.video_hit1 = div(h1, rep.videos);
    rep.video_hit5 = div(h5, rep.videos);
    rep.video_hit10 = div(h10, rep.videos);
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(path: &str, t: Option<(f64, f64)>) -> SearchHit {
        SearchHit {
            item_id: 1,
            file_id: 1,
            path: PathBuf::from(path),
            kind: if t.is_some() {
                MediaKind::Video
            } else {
                MediaKind::Image
            },
            mtime: 0,
            score: 0.5,
            t_start: t.map(|x| x.0),
            t_end: t.map(|x| x.1),
            width: None,
            height: None,
            duration: None,
            moments: vec![],
        }
    }

    #[test]
    fn range_matching_with_tolerance() {
        let exp = vec![Expected::Range {
            path: PathBuf::from("/v.mp4"),
            start: 30.0,
            end: 46.0,
        }];
        assert!(matches(&hit("/v.mp4", Some((40.0, 42.0))), &exp));
        assert!(matches(&hit("/v.mp4", Some((47.0, 50.0))), &exp));
        assert!(!matches(&hit("/v.mp4", Some((49.0, 52.0))), &exp));
        assert!(!matches(&hit("/w.mp4", Some((40.0, 42.0))), &exp));
        assert!(matches(
            &hit("/a.jpg", None),
            &[Expected::Image(PathBuf::from("/a.jpg"))]
        ));
    }

    #[test]
    fn parses_both_expected_forms() {
        let q: Query = serde_json::from_str(
            r#"{"id":"x","query":"q","kind":"video","expected":[{"path":"v.mp4","start":1,"end":2}, "a.jpg"]}"#,
        )
        .unwrap();
        let e = parse_expected(Path::new("/root"), &q);
        assert_eq!(e.len(), 2);
        assert!(matches!(&e[0], Expected::Range { start, .. } if *start == 1.0));
    }
}
