//! In-memory vector index: exact brute-force scan over i8-quantized unit vectors (see `docs/design.md`).

use crate::media::MediaKind;
use rayon::prelude::*;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

const CACHE_MAGIC: &[u8; 8] = b"RCLXIDX3";

/// Longest model name / library id accepted in a cache header.
const MAX_NAME_LEN: usize = 256;

/// Rows per parallel work unit.
const BLOCK: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub item_id: i64,
    pub file_id: i64,
    pub kind: MediaKind,
    /// Approximate cosine similarity (from quantized vectors).
    pub score: f32,
    /// Capture time (local wall-clock seconds, see [`VectorIndex::add`]).
    pub taken_at: i64,
}

/// What a scan skips. All conditions must hold for a row to be considered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanFilter {
    pub kind: Option<MediaKind>,
    /// Skip every item of this file.
    pub exclude_file: Option<i64>,
    /// Keep only items captured within this range (inclusive, wall-clock seconds).
    pub taken_range: Option<(i64, i64)>,
}

pub struct VectorIndex {
    dim: usize,
    scale: f32,
    data: Vec<i8>,
    item_ids: Vec<i64>,
    file_ids: Vec<i64>,
    kinds: Vec<MediaKind>,
    /// Capture time of the item's file (local wall-clock seconds), parallel to `item_ids`.
    taken: Vec<i64>,
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

#[inline(always)]
fn dot_i8_portable(a: &[i8], b: &[i8]) -> i32 {
    // Written so LLVM auto-vectorizes it (i8×i8 → i16 → i32 accumulate).
    a.iter()
        .zip(b)
        .map(|(&x, &y)| i32::from(x) * i32::from(y))
        .sum()
}

fn dot_i8(a: &[i8], b: &[i8]) -> i32 {
    dot_i8_portable(a, b)
}

/// The same loop compiled for AVX2 (release builds target baseline x86-64, i.e. SSE2).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
fn dot_i8_avx2(a: &[i8], b: &[i8]) -> i32 {
    dot_i8_portable(a, b)
}

type DotFn = fn(&[i8], &[i8]) -> i32;

/// Picks the fastest dot product this CPU supports (checked once per search).
fn pick_dot() -> DotFn {
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx2") {
        // SAFETY: only called after confirming the CPU supports AVX2.
        return |a, b| unsafe { dot_i8_avx2(a, b) };
    }
    dot_i8
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
            taken: Vec::new(),
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
        self.data.len() + self.item_ids.len() * (8 + 8 + 1 + 8)
    }

    pub fn reserve(&mut self, n: usize) {
        self.data.reserve(n * self.dim);
        self.item_ids.reserve(n);
        self.file_ids.reserve(n);
        self.kinds.reserve(n);
        self.taken.reserve(n);
    }

    /// Appends a vector. `taken_at` is the file's capture time as the timeline shows it: local
    /// wall-clock seconds, or the file's modification time converted the same way when no date
    /// is known.
    pub fn add(&mut self, item_id: i64, file_id: i64, kind: MediaKind, taken_at: i64, v: &[f32]) {
        assert_eq!(v.len(), self.dim, "vector dimension mismatch");
        self.data.extend(quantize(v, self.scale));
        self.item_ids.push(item_id);
        self.file_ids.push(file_id);
        self.kinds.push(kind);
        self.taken.push(taken_at);
    }

    /// Sets the capture time of every item of the given files (`file id -> time`). Used when the
    /// date pass learns a date for a file that was already embedded: the in-memory column is
    /// patched in place (one pass over the rows), and the database change that accompanies it
    /// moves the epoch so the on-disk cache is rewritten rather than reused with old dates.
    pub fn set_taken(&mut self, times: &std::collections::HashMap<i64, i64>) {
        if times.is_empty() {
            return;
        }
        for (f, t) in self.file_ids.iter().zip(self.taken.iter_mut()) {
            if let Some(&new) = times.get(f) {
                *t = new;
            }
        }
    }

    /// Saves the index to `path` (atomically), stamped with `model`, `library_id` and `epoch`.
    ///
    /// The bytes go to a uniquely named temp file next to `path` (opened with `create_new`) and
    /// are renamed into place, so concurrent savers can never interleave writes into one file
    /// and a crash never leaves a half-written cache under the real name.
    pub fn save(
        &self,
        path: &std::path::Path,
        model: &str,
        library_id: &str,
        epoch: u64,
    ) -> std::io::Result<()> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tmp = path.with_file_name(format!(
            "{name}.{}.{}.tmp",
            std::process::id(),
            COUNTER.fetch_add(1, AtomicOrdering::Relaxed)
        ));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        let res = self
            .write_to(file, model, library_id, epoch)
            .and_then(|()| std::fs::rename(&tmp, path));
        if res.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        res
    }

    fn write_to(
        &self,
        file: std::fs::File,
        model: &str,
        library_id: &str,
        epoch: u64,
    ) -> std::io::Result<()> {
        use std::io::Write;
        let mut w = std::io::BufWriter::with_capacity(1 << 20, file);
        w.write_all(CACHE_MAGIC)?;
        w.write_all(&(model.len() as u32).to_le_bytes())?;
        w.write_all(model.as_bytes())?;
        w.write_all(&(library_id.len() as u32).to_le_bytes())?;
        w.write_all(library_id.as_bytes())?;
        w.write_all(&(self.dim as u32).to_le_bytes())?;
        w.write_all(&self.scale.to_le_bytes())?;
        w.write_all(&epoch.to_le_bytes())?;
        w.write_all(&(self.len() as u64).to_le_bytes())?;
        for v in &self.item_ids {
            w.write_all(&v.to_le_bytes())?;
        }
        for v in &self.file_ids {
            w.write_all(&v.to_le_bytes())?;
        }
        for v in &self.taken {
            w.write_all(&v.to_le_bytes())?;
        }
        w.write_all(
            &self
                .kinds
                .iter()
                .map(|k| k.as_i64() as u8)
                .collect::<Vec<_>>(),
        )?;
        // SAFETY: i8 and u8 have the same size and layout.
        let bytes =
            unsafe { std::slice::from_raw_parts(self.data.as_ptr().cast::<u8>(), self.data.len()) };
        w.write_all(bytes)?;
        w.into_inner().map_err(|e| e.into_error())?.sync_all()
    }

    /// Loads a cache written by [`Self::save`] if it matches `model`, `library_id`, `dim`,
    /// `scale` and `epoch`. The header is untrusted: the file length must equal exactly what the
    /// header promises before anything is allocated.
    pub fn load(
        path: &std::path::Path,
        model: &str,
        library_id: &str,
        dim: usize,
        scale: f32,
        epoch: u64,
    ) -> Option<Self> {
        use std::io::Read;
        let file = std::fs::File::open(path).ok()?;
        let file_len = file.metadata().ok()?.len();
        let mut r = std::io::BufReader::with_capacity(1 << 20, file);
        let mut magic = [0u8; 8];
        r.read_exact(&mut magic).ok()?;
        if &magic != CACHE_MAGIC {
            return None;
        }
        let mut u32b = [0u8; 4];
        let mut u64b = [0u8; 8];
        r.read_exact(&mut u32b).ok()?;
        let name_len = u32::from_le_bytes(u32b) as usize;
        if name_len > MAX_NAME_LEN {
            return None;
        }
        let mut name = vec![0u8; name_len];
        r.read_exact(&mut name).ok()?;
        r.read_exact(&mut u32b).ok()?;
        let id_len = u32::from_le_bytes(u32b) as usize;
        if id_len > MAX_NAME_LEN {
            return None;
        }
        let mut id = vec![0u8; id_len];
        r.read_exact(&mut id).ok()?;
        r.read_exact(&mut u32b).ok()?;
        let d = u32::from_le_bytes(u32b) as usize;
        r.read_exact(&mut u32b).ok()?;
        let sc = f32::from_le_bytes(u32b);
        r.read_exact(&mut u64b).ok()?;
        let ep = u64::from_le_bytes(u64b);
        r.read_exact(&mut u64b).ok()?;
        let n = usize::try_from(u64::from_le_bytes(u64b)).ok()?;
        if name != model.as_bytes()
            || id != library_id.as_bytes()
            || d != dim
            || sc != scale
            || ep != epoch
        {
            return None;
        }
        // magic + name_len + name + id_len + id + dim + scale + epoch + n
        let header = (8 + 4 + 4 + 4 + 4 + 8 + 8usize)
            .checked_add(name_len)?
            .checked_add(id_len)?;
        let expected = n
            .checked_mul(8 + 8 + 8 + 1usize)?
            .checked_add(n.checked_mul(dim)?)?
            .checked_add(header)?;
        if expected as u64 != file_len {
            return None;
        }
        let mut read_i64s = |n: usize| -> Option<Vec<i64>> {
            let mut buf = vec![0u8; n * 8];
            r.read_exact(&mut buf).ok()?;
            Some(
                buf.as_chunks::<8>()
                    .0
                    .iter()
                    .map(|c| i64::from_le_bytes(*c))
                    .collect(),
            )
        };
        let item_ids = read_i64s(n)?;
        let file_ids = read_i64s(n)?;
        let taken = read_i64s(n)?;
        let mut kinds = vec![0u8; n];
        r.read_exact(&mut kinds).ok()?;
        let mut raw = vec![0u8; n * dim];
        r.read_exact(&mut raw).ok()?;
        // Reinterpret the bytes as i8 without copying.
        let mut raw = std::mem::ManuallyDrop::new(raw);
        // SAFETY: same allocation, element size, alignment and length; ownership moves over.
        let data = unsafe {
            Vec::from_raw_parts(raw.as_mut_ptr().cast::<i8>(), raw.len(), raw.capacity())
        };
        Some(Self {
            dim,
            scale,
            data,
            item_ids,
            file_ids,
            taken,
            kinds: kinds
                .into_iter()
                .map(|k| MediaKind::from_i64(i64::from(k)))
                .collect(),
        })
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
                self.taken[w] = self.taken[r];
            }
            w += 1;
        }
        self.data.truncate(w * dim);
        self.item_ids.truncate(w);
        self.file_ids.truncate(w);
        self.kinds.truncate(w);
        self.taken.truncate(w);
    }

    /// Top `k` items by approximate cosine similarity to unit vector `q`, best first.
    pub fn search(&self, q: &[f32], k: usize, kind: Option<MediaKind>) -> Vec<Candidate> {
        self.search_filtered(
            q,
            k,
            &ScanFilter {
                kind,
                ..Default::default()
            },
        )
    }

    /// Like [`Self::search`] with a [`ScanFilter`]; the filter is applied inside the scan.
    pub fn search_filtered(&self, q: &[f32], k: usize, filter: &ScanFilter) -> Vec<Candidate> {
        let ScanFilter {
            kind,
            exclude_file,
            taken_range,
        } = *filter;
        if k == 0 || self.is_empty() || q.len() != self.dim {
            return Vec::new();
        }
        // Quantize the query with its own scale for full i8 precision.
        let qmax = q.iter().fold(0f32, |m, x| m.max(x.abs())).max(1e-12);
        let qscale = 127.0 / qmax;
        let qq: Vec<i8> = quantize(q, qscale).collect();
        let dim = self.dim;
        let n = self.len();
        let dot = pick_dot();
        let blocks: Vec<usize> = (0..n.div_ceil(BLOCK)).collect();
        let heaps: Vec<BinaryHeap<Entry>> = blocks
            .par_iter()
            .map(|&b| {
                let mut heap = BinaryHeap::with_capacity(k + 1);
                let end = ((b + 1) * BLOCK).min(n);
                for r in b * BLOCK..end {
                    if kind.is_some_and(|kd| self.kinds[r] != kd)
                        || exclude_file == Some(self.file_ids[r])
                        || taken_range.is_some_and(|(a, b)| self.taken[r] < a || self.taken[r] > b)
                    {
                        continue;
                    }
                    let s = dot(&self.data[r * dim..(r + 1) * dim], &qq);
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
                taken_at: self.taken[r],
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
                0,
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
            idx.add(i as i64, 0, MediaKind::Image, 0, v);
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
    fn simd_and_portable_dot_agree() {
        let a: Vec<i8> = (0..768).map(|i| ((i * 37) % 255) as i8).collect();
        let b: Vec<i8> = (0..768).map(|i| ((i * 91 + 7) % 255) as i8).collect();
        assert_eq!(pick_dot()(&a, &b), dot_i8(&a, &b));
    }

    #[test]
    fn cache_round_trip_and_staleness() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("index-m.bin");
        let vs = rand_vecs(300, 16, 5);
        let mut idx = VectorIndex::new(16, 300.0);
        for (i, v) in vs.iter().enumerate() {
            idx.add(
                i as i64,
                i as i64 / 2,
                if i % 3 == 0 {
                    MediaKind::Video
                } else {
                    MediaKind::Image
                },
                1000 + i as i64,
                v,
            );
        }
        idx.save(&p, "m", "lib", 7).unwrap();
        let back = VectorIndex::load(&p, "m", "lib", 16, 300.0, 7).unwrap();
        assert_eq!(
            (
                back.data.clone(),
                back.item_ids.clone(),
                back.file_ids.clone(),
                back.kinds.clone(),
                back.taken.clone()
            ),
            (
                idx.data.clone(),
                idx.item_ids.clone(),
                idx.file_ids.clone(),
                idx.kinds.clone(),
                idx.taken.clone()
            )
        );
        assert_eq!(back.search(&vs[10], 1, None)[0].item_id, 10);
        assert_eq!(back.search(&vs[10], 1, None)[0].taken_at, 1010);
        assert!(
            VectorIndex::load(&p, "m", "lib", 16, 300.0, 8).is_none(),
            "stale epoch"
        );
        assert!(
            VectorIndex::load(&p, "other", "lib", 16, 300.0, 7).is_none(),
            "other model"
        );
        assert!(
            VectorIndex::load(&p, "m", "other-lib", 16, 300.0, 7).is_none(),
            "other library"
        );
        // Truncated file.
        let bytes = std::fs::read(&p).unwrap();
        std::fs::write(&p, &bytes[..bytes.len() - 1]).unwrap();
        assert!(VectorIndex::load(&p, "m", "lib", 16, 300.0, 7).is_none());
        // Header claiming a huge item count must not allocate or panic.
        let mut forged = bytes.clone();
        let n_at = 8 + 4 + 1 + 4 + 3 + 4 + 4 + 8;
        forged[n_at..n_at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        std::fs::write(&p, &forged).unwrap();
        assert!(VectorIndex::load(&p, "m", "lib", 16, 300.0, 7).is_none());
        // Oversized model name.
        let mut forged = bytes.clone();
        forged[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&p, &forged).unwrap();
        assert!(VectorIndex::load(&p, "m", "lib", 16, 300.0, 7).is_none());
        std::fs::write(&p, b"garbage").unwrap();
        assert!(VectorIndex::load(&p, "m", "lib", 16, 300.0, 7).is_none());
    }

    #[test]
    fn taken_filter_is_applied_in_the_scan_and_can_be_patched() {
        let mut idx = VectorIndex::new(2, 100.0);
        // All vectors point the same way, so only the filter decides what is returned.
        for (i, t) in [100, 200, 300, 400].into_iter().enumerate() {
            idx.add(i as i64, i as i64, MediaKind::Image, t, &[1.0, 0.0]);
        }
        let ids = |idx: &VectorIndex, r: Option<(i64, i64)>| {
            let f = ScanFilter {
                taken_range: r,
                ..Default::default()
            };
            let mut v: Vec<i64> = idx
                .search_filtered(&[1.0, 0.0], 10, &f)
                .iter()
                .map(|c| c.item_id)
                .collect();
            v.sort();
            v
        };
        assert_eq!(ids(&idx, None), [0, 1, 2, 3]);
        assert_eq!(ids(&idx, Some((200, 300))), [1, 2]); // inclusive on both ends
        assert_eq!(ids(&idx, Some((500, 600))), Vec::<i64>::new());
        idx.set_taken(&std::collections::HashMap::from([(0, 250)]));
        assert_eq!(ids(&idx, Some((200, 300))), [0, 1, 2]);
        // Removing files keeps the column aligned.
        idx.remove_files(&HashSet::from([1]));
        assert_eq!(ids(&idx, Some((300, 400))), [2, 3]);
    }

    #[test]
    fn remove_and_edge_cases() {
        let mut idx = VectorIndex::new(2, 100.0);
        assert!(idx.search(&[1.0, 0.0], 3, None).is_empty());
        idx.add(1, 10, MediaKind::Image, 0, &[1.0, 0.0]);
        idx.add(2, 20, MediaKind::Image, 0, &[0.0, 1.0]);
        idx.add(3, 10, MediaKind::Video, 0, &[0.6, 0.8]);
        idx.remove_files(&HashSet::from([10]));
        assert_eq!(idx.len(), 1);
        let r = idx.search(&[0.0, 1.0], 5, None);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].item_id, 2);
        assert!(idx.search(&[0.0, 1.0], 0, None).is_empty());
    }
}
