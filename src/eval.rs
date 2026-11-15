//! Search-quality evaluation against a query set.
//!
//! The query file is a JSON array of `{id, query, kind: "image"|"video", source, expected}`;
//! `expected` lists image paths, or `{path, start, end}` time ranges for videos, relative to
//! `--root`. `source: "coco-category"` marks queries with many correct images.
//!
//! - caption queries (one expected image): Recall@1/5/10 and MRR
//! - category queries (many expected images): "P@10", the correct images among the top 10
//!   divided by `min(expected, 10)`. With fewer than 10 expected images this measures against
//!   the best achievable result, not against 10 slots; it is kept as is so earlier
//!   measurements stay comparable
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
    #[serde(rename = "query")]
    text: String,
    kind: Kind,
    #[serde(default)]
    source: String,
    expected: Vec<ExpectedSpec>,
}

/// Which kind of media a query looks for.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Image,
    Video,
}

impl From<Kind> for MediaKind {
    fn from(k: Kind) -> Self {
        match k {
            Kind::Image => Self::Image,
            Kind::Video => Self::Video,
        }
    }
}

/// An expected answer as written in the query file: a path, or a path with a time range.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ExpectedSpec {
    Image(String),
    Range { path: String, start: f64, end: f64 },
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
    /// Mean over category queries of (correct results in the top 10) / `min(expected, 10)`:
    /// precision at 10 measured against the best achievable when fewer than 10 images are
    /// expected.
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
        .map(|e| match e {
            ExpectedSpec::Image(s) => Expected::Image(canon(&root.join(s))),
            ExpectedSpec::Range { path, start, end } => Expected::Range {
                path: canon(&root.join(path)),
                start: *start,
                end: *end,
            },
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

/// Running totals of the per-query scores, averaged once all queries are done.
#[derive(Default)]
struct Sums {
    r1: f64,
    r5: f64,
    r10: f64,
    mrr: f64,
    p10: f64,
    h1: f64,
    h5: f64,
    h10: f64,
}

pub fn run(lib: &Library, queries: &Path, root: &Path) -> Result<Report> {
    let text = std::fs::read_to_string(queries)
        .with_context(|| format!("reading {}", queries.display()))?;
    let qs: Vec<Query> =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", queries.display()))?;
    let mut rep = Report::default();
    let mut sums = Sums::default();
    for q in &qs {
        let exp = parse_expected(root, q);
        let kind = MediaKind::from(q.kind);
        let present = exp.iter().any(|e| match e {
            Expected::Image(p) | Expected::Range { path: p, .. } => p.exists(),
        });
        if exp.is_empty() || !present {
            rep.skipped += 1;
            continue;
        }
        let hits = lib.search(
            &q.text,
            &SearchOptions {
                limit: 10,
                kind: Some(kind),
                ..Default::default()
            },
        )?;
        let rank = hits.iter().position(|h| matches(h, &exp));
        rep.ranks.push((q.id.clone(), rank));
        tally(&mut rep, &mut sums, (kind, q.source.as_str()), &hits, &exp);
    }
    fill_averages(&mut rep, &sums);
    Ok(rep)
}

/// Adds one answered query to the report counts and the running totals. Videos count towards
/// Hit@k, category queries towards precision, everything else towards recall and MRR.
fn tally(
    rep: &mut Report,
    sums: &mut Sums,
    (kind, source): (MediaKind, &str),
    hits: &[SearchHit],
    exp: &[Expected],
) {
    let rank = hits.iter().position(|h| matches(h, exp));
    let at = |k: usize| f64::from(u8::from(rank.is_some_and(|r| r < k)));
    match (kind, source) {
        (MediaKind::Video, _) => {
            rep.videos += 1;
            sums.h1 += at(1);
            sums.h5 += at(5);
            sums.h10 += at(10);
        }
        (_, "coco-category") => {
            rep.categories += 1;
            let good = hits.iter().filter(|h| matches(h, exp)).count() as f64;
            sums.p10 += good / (exp.len().min(10) as f64);
        }
        _ => {
            rep.captions += 1;
            sums.r1 += at(1);
            sums.r5 += at(5);
            sums.r10 += at(10);
            sums.mrr += rank.map_or(0.0, |r| 1.0 / (r as f64 + 1.0));
        }
    }
}

/// Turns the running totals into averages over the queries of each group.
fn fill_averages(rep: &mut Report, sums: &Sums) {
    let div = |a: f64, n: usize| if n == 0 { 0.0 } else { a / n as f64 };
    rep.caption_r1 = div(sums.r1, rep.captions);
    rep.caption_r5 = div(sums.r5, rep.captions);
    rep.caption_r10 = div(sums.r10, rep.captions);
    rep.caption_mrr = div(sums.mrr, rep.captions);
    rep.category_p10 = div(sums.p10, rep.categories);
    rep.video_hit1 = div(sums.h1, rep.videos);
    rep.video_hit5 = div(sums.h5, rep.videos);
    rep.video_hit10 = div(sums.h10, rep.videos);
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
            taken_at: 0,
            favorite: false,
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
        assert!(matches!(&e[0], Expected::Range { start, .. } if (*start - 1.0).abs() < 1e-9));
    }

    #[test]
    fn malformed_queries_are_errors() {
        let parse = |json: &str| serde_json::from_str::<Query>(json);
        let ok = r#"{"id":"x","query":"q","kind":"image","expected":["a.jpg"]}"#;
        assert!(parse(ok).is_ok());
        // Unknown kind, a range without an end, a number where a path belongs.
        assert!(parse(&ok.replace("image", "audio")).is_err());
        let range =
            r#"{"id":"x","query":"q","kind":"video","expected":[{"path":"v.mp4","start":1}]}"#;
        assert!(parse(range).is_err());
        assert!(parse(&ok.replace(r#""a.jpg""#, "7")).is_err());
    }
}
