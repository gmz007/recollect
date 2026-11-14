//! Thumbnails: generated lazily when results are shown, cached on disk as JPEG (see `docs/design.md`).

use crate::media::image::{self as img, Rgb};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Longest side of a thumbnail, in pixels.
pub const THUMB_SIZE: u32 = 320;

/// Identifies one thumbnail: a file version and, for videos, a time position.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ThumbKey {
    pub path: PathBuf,
    pub mtime: i64,
    /// Milliseconds into a video; `None` for images.
    pub t_ms: Option<i64>,
}

impl ThumbKey {
    fn cache_file(&self, dir: &Path) -> PathBuf {
        let mut h = Sha256::new();
        h.update(self.path.to_string_lossy().as_bytes());
        h.update(self.mtime.to_le_bytes());
        h.update(self.t_ms.unwrap_or(-1).to_le_bytes());
        let hex = hex::encode(h.finalize());
        dir.join(&hex[..2]).join(format!("{}.jpg", &hex[2..32]))
    }
}

/// Loads a thumbnail from the disk cache or creates (and caches) it.
pub fn load(key: &ThumbKey, cache_dir: &Path) -> Result<Rgb> {
    let file = key.cache_file(cache_dir);
    if let Ok(d) = img::decode(&file, u32::MAX) {
        // Mark as recently used for pruning (access times are often not maintained).
        if let Ok(f) = std::fs::File::options().write(true).open(&file) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
        return Ok(d.rgb);
    }
    let rgb = generate(key)?;
    if let Err(e) = save_jpeg(&file, &rgb) {
        tracing::debug!("could not cache thumbnail {}: {e:#}", file.display());
    }
    Ok(rgb)
}

fn generate(key: &ThumbKey) -> Result<Rgb> {
    let full = match key.t_ms {
        None => img::decode(&key.path, THUMB_SIZE)?.rgb,
        Some(t) => crate::media::video::frame_at(&key.path, t as f64 / 1000.0, THUMB_SIZE)?,
    };
    img::fit_within(&full, THUMB_SIZE)
}

fn save_jpeg(file: &Path, rgb: &Rgb) -> Result<()> {
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
    }
    crate::config::replace_file(file, |tmp| {
        let out = std::io::BufWriter::new(
            std::fs::File::create(tmp).with_context(|| format!("creating {}", tmp.display()))?,
        );
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(out, 82);
        enc.encode(
            &rgb.data,
            rgb.width,
            rgb.height,
            image::ExtendedColorType::Rgb8,
        )
        .context("encoding thumbnail")
    })
}

/// Deletes the least recently used thumbnails until the cache is below `max_bytes`.
/// Returns the number of files removed.
pub fn prune(cache_dir: &Path, max_bytes: u64) -> usize {
    let mut files = Vec::new();
    let mut total = 0u64;
    let Ok(dirs) = std::fs::read_dir(cache_dir) else {
        return 0;
    };
    for d in dirs.flatten() {
        let Ok(entries) = std::fs::read_dir(d.path()) else {
            continue;
        };
        for e in entries.flatten() {
            if let Ok(m) = e.metadata() {
                total += m.len();
                let t = m.modified().ok();
                files.push((t, m.len(), e.path()));
            }
        }
    }
    if total <= max_bytes {
        return 0;
    }
    files.sort_by_key(|f| f.0);
    let mut removed = 0;
    for (_, len, p) in files {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total = total.saturating_sub(len);
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_caches_and_prunes() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("big.png");
        image::RgbImage::from_fn(1000, 500, |x, _| {
            image::Rgb([u8::try_from(x % 256).unwrap(), 0, 0])
        })
        .save(&src)
        .unwrap();
        let cache = d.path().join("cache");
        let key = ThumbKey {
            path: src.clone(),
            mtime: 1,
            t_ms: None,
        };
        let t = load(&key, &cache).unwrap();
        assert_eq!((t.width, t.height), (THUMB_SIZE, THUMB_SIZE / 2));
        let cached = key.cache_file(&cache);
        assert!(cached.exists());
        // Second load comes from the cache even if the source vanished.
        std::fs::remove_file(&src).unwrap();
        assert_eq!(load(&key, &cache).unwrap().width, THUMB_SIZE);
        // A different mtime is a different thumbnail (and the source is gone now).
        assert!(load(&ThumbKey { mtime: 2, ..key }, &cache).is_err());
        assert_eq!(prune(&cache, u64::MAX), 0);
        assert_eq!(prune(&cache, 0), 1);
        assert!(!cached.exists());
    }
}
