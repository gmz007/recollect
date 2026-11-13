//! In-memory vector index: exact brute-force scan over i8-quantized unit vectors (ADR 0005).

use crate::media::MediaKind;
use rayon::prelude::*;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};

/// Rows per parallel work unit.
const BLOCK: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub item_id: i64,
    pub file_id: i64,
    pub kind: MediaKind,
    /// Approximate cosine similarity (from quantized vectors).
    pub score: f32,
}

pub struct VectorIndex {
    dim: usize,
    scale: f32,
    data: Vec<i8>,
    item_ids: Vec<i64>,
    file_ids: Vec<i64>,
    kinds: Vec<MediaKind>,
}

/// Min-heap entry so the heap root is the weakest of the current top-k.
#[derive(PartialEq, Eq)]
struct Entry(i32, usize);

impl Ord for Entry {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.cmp(&self.0).then(o.1.cmp(&self.1))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

fn quantize(v: &[f32], scale: f32) -> impl Iterator<Item = i8> + '_ {
    v.iter()
        .map(move |x| (x * scale).round().clamp(-127.0, 127.0) as i8)
}

#[inline]
fn dot_i8(a: &[i8], b: &[i8]) -> i32 {
    // Written so LLVM auto-vectorizes it (i8×i8 → i16 → i32 accumulate).
    a.iter()
        .zip(b)
        .map(|(&x, &y)| i32::from(x) * i32::from(y))
        .sum()
}

impl VectorIndex {
    pub fn new(dim: usize, scale: f32) -> Self {
        Self {
            dim,
            scale,
            data: Vec::new(),
            item_ids: Vec::new(),
            file_ids: Vec::new(),
            kinds: Vec::new(),
        }
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn len(&self) -> usize {
        self.item_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.item_ids.is_empty()
    }

    /// Bytes used by the vectors.
    pub fn memory_bytes(&self) -> usize {
        self.data.len() + self.item_ids.len() * (8 + 8 + 1)
    }

    pub fn reserve(&mut self, n: usize) {
        self.data.reserve(n * self.dim);
        self.item_ids.reserve(n);
        self.file_ids.reserve(n);
        self.kinds.reserve(n);
    }

    pub fn add(&mut self, item_id: i64, file_id: i64, kind: MediaKind, v: &[f32]) {
        assert_eq!(v.len(), self.dim, "vector dimension mismatch");
        self.data.extend(quantize(v, self.scale));
        self.item_ids.push(item_id);
        self.file_ids.push(file_id);
        self.kinds.push(kind);
    }

    /// Removes all items belonging to the given files.
    pub fn remove_files(&mut self, files: &HashSet<i64>) {
        if files.is_empty() {
            return;
        }
        let dim = self.dim;
        let mut w = 0;
        for r in 0..self.item_ids.len() {
            if files.contains(&self.file_ids[r]) {
                continue;
            }
            if w != r {
                self.data.copy_within(r * dim..(r + 1) * dim, w * dim);
                self.item_ids[w] = self.item_ids[r];
                self.file_ids[w] = self.file_ids[r];
                self.kinds[w] = self.kinds[r];
            }
            w += 1;
        }
        self.data.truncate(w * dim);
        self.item_ids.truncate(w);
        self.file_ids.truncate(w);
        self.kinds.truncate(w);
    }

    /// Top `k` items by approximate cosine similarity to unit vector `q`, best first.
    pub fn search(&self, q: &[f32], k: usize, kind: Option<MediaKind>) -> Vec<Candidate> {
        self.search_excluding(q, k, kind, None)
    }

    /// Like [`Self::search`], skipping all items of `exclude_file`.
    pub fn search_excluding(
        &self,
        q: &[f32],
        k: usize,
        kind: Option<MediaKind>,
        exclude_file: Option<i64>,
    ) -> Vec<Candidate> {
        if k == 0 || self.is_empty() || q.len() != self.dim {
            return Vec::new();
        }
        // Quantize the query with its own scale for full i8 precision.
        let qmax = q.iter().fold(0f32, |m, x| m.max(x.abs())).max(1e-12);
        let qscale = 127.0 / qmax;
        let qq: Vec<i8> = quantize(q, qscale).collect();
        let dim = self.dim;
        let n = self.len();
        let blocks: Vec<usize> = (0..n.div_ceil(BLOCK)).collect();
        let heaps: Vec<BinaryHeap<Entry>> = blocks
            .par_iter()
            .map(|&b| {
                let mut heap = BinaryHeap::with_capacity(k + 1);
                let end = ((b + 1) * BLOCK).min(n);
                for r in b * BLOCK..end {
                    if kind.is_some_and(|kd| self.kinds[r] != kd)
                        || exclude_file == Some(self.file_ids[r])
                    {
                        continue;
                    }
                    let s = dot_i8(&self.data[r * dim..(r + 1) * dim], &qq);
                    if heap.len() < k {
                        heap.push(Entry(s, r));
                    } else if heap.peek().is_some_and(|e| s > e.0) {
                        heap.pop();
                        heap.push(Entry(s, r));
                    }
                }
                heap
            })
            .collect();
        let mut all: Vec<Entry> = heaps.into_iter().flat_map(|h| h.into_vec()).collect();
        all.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        all.truncate(k);
        let denom = self.scale * qscale;
        all.into_iter()
            .map(|Entry(s, r)| Candidate {
                item_id: self.item_ids[r],
                file_id: self.file_ids[r],
                kind: self.kinds[r],
                score: s as f32 / denom,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: &[f32]) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / n).collect()
    }

    fn rand_vecs(n: usize, d: usize, seed: u64) -> Vec<Vec<f32>> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                unit(
                    &(0..d)
                        .map(|_| {
                            s ^= s << 13;
                            s ^= s >> 7;
                            s ^= s << 17;
                            (s >> 40) as f32 / (1u64 << 24) as f32 - 0.5
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect()
    }

    #[test]
    fn finds_exact_neighbours_across_blocks() {
        let d = 64;
        let vs = rand_vecs(10_000, d, 7);
        let mut idx = VectorIndex::new(d, 300.0);
        for (i, v) in vs.iter().enumerate() {
            idx.add(
                i as i64 + 1,
                i as i64 / 3,
                if i % 2 == 0 {
                    MediaKind::Image
                } else {
                    MediaKind::Video
                },
                v,
            );
        }
        for probe in [0usize, 4097, 9999] {
            let res = idx.search(&vs[probe], 5, None);
            assert_eq!(res[0].item_id, probe as i64 + 1);
            assert!((res[0].score - 1.0).abs() < 0.05, "{}", res[0].score);
            assert!(res.windows(2).all(|w| w[0].score >= w[1].score));
        }
        // Kind filter.
        let res = idx.search(&vs[1], 10, Some(MediaKind::Image));
        assert!(res.iter().all(|c| c.kind == MediaKind::Image));
        assert_eq!(res.len(), 10);
    }

    #[test]
    fn ranking_matches_float_ranking() {
        let d = 128;
        let vs = rand_vecs(2000, d, 3);
        let mut idx = VectorIndex::new(d, 300.0);
        for (i, v) in vs.iter().enumerate() {
            idx.add(i as i64, 0, MediaKind::Image, v);
        }
        let q = &rand_vecs(1, d, 99)[0];
        let mut exact: Vec<(f32, i64)> = vs
            .iter()
            .enumerate()
            .map(|(i, v)| (v.iter().zip(q).map(|(a, b)| a * b).sum(), i as i64))
            .collect();
        exact.sort_by(|a, b| b.0.total_cmp(&a.0));
        let top: HashSet<i64> = exact.iter().take(20).map(|x| x.1).collect();
        let got = idx.search(q, 20, None);
        let overlap = got.iter().filter(|c| top.contains(&c.item_id)).count();
        assert!(overlap >= 17, "overlap {overlap}/20");
    }

    #[test]
    fn remove_and_edge_cases() {
        let mut idx = VectorIndex::new(2, 100.0);
        assert!(idx.search(&[1.0, 0.0], 3, None).is_empty());
        idx.add(1, 10, MediaKind::Image, &[1.0, 0.0]);
        idx.add(2, 20, MediaKind::Image, &[0.0, 1.0]);
        idx.add(3, 10, MediaKind::Video, &[0.6, 0.8]);
        idx.remove_files(&HashSet::from([10]));
        assert_eq!(idx.len(), 1);
        let r = idx.search(&[0.0, 1.0], 5, None);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].item_id, 2);
        assert!(idx.search(&[0.0, 1.0], 0, None).is_empty());
    }
}
