//! Model download. **This is the only module in Recollect that uses the network**, and it is
//! only invoked after the user explicitly agreed to download a model.

use super::registry::{ModelFile, ModelSpec};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Directory where a model's files live.
pub fn model_dir(models_root: &Path, spec: &ModelSpec) -> PathBuf {
    models_root.join(spec.id)
}

/// True if every file of the model is present with the expected size.
/// (Checksums are verified once, at download time.)
pub fn is_installed(models_root: &Path, spec: &ModelSpec) -> bool {
    let dir = model_dir(models_root, spec);
    spec.files
        .iter()
        .all(|f| is_complete(&dir.join(f.local), f))
}

/// True if `path` holds all of `f` (its expected size).
fn is_complete(path: &Path, f: &ModelFile) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.len() == f.size || placeholders_count())
}

/// Lets tests stand in empty placeholder files for a model. Full-size placeholders are free
/// where files can be sparse, but on Windows they take the model's whole size on disk, and
/// tests running side by side would fill it.
#[cfg(test)]
pub static PLACEHOLDERS_COUNT: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
fn placeholders_count() -> bool {
    PLACEHOLDERS_COUNT.load(Ordering::Relaxed)
}

#[cfg(not(test))]
const fn placeholders_count() -> bool {
    false
}

/// Progress callback: (bytes done across all files, total bytes).
pub type ProgressFn<'a> = dyn FnMut(u64, u64) + 'a;

/// Downloads all missing files of `spec`, verifying SHA-256. Safe to re-run; partial
/// downloads are discarded and restarted. Returns early with an error if `cancel` is set.
pub fn download(
    models_root: &Path,
    spec: &ModelSpec,
    cancel: &AtomicBool,
    progress: &mut ProgressFn<'_>,
) -> Result<()> {
    let dir = model_dir(models_root, spec);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let total = spec.total_size();
    let mut done = 0u64;
    for f in spec.files {
        let dest = dir.join(f.local);
        if is_complete(&dest, f) {
            done += f.size;
            progress(done, total);
            continue;
        }
        fetch_file(&spec.url(f), f, &dest, cancel, &mut |n| {
            progress(done + n, total);
        })?;
        done += f.size;
    }
    remove_stale_files(&dir, spec);
    Ok(())
}

/// Deletes regular files in the model directory that the current spec doesn't list (e.g. the
/// 441 MB fp32 `text_model.onnx` left by an older `SigLIP` install). Runs only after a complete,
/// verified download, so a failed update never removes anything. Directories and symlinks are
/// left alone.
fn remove_stale_files(dir: &Path, spec: &ModelSpec) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let known = spec.files.iter().any(|f| e.file_name() == f.local);
        if !known && e.file_type().is_ok_and(|t| t.is_file()) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Downloads one file to `dest` through a `.part` file, which is removed again on every
/// failure (cancel, network or disk error, wrong content).
fn fetch_file(
    url: &str,
    f: &ModelFile,
    dest: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<()> {
    let part = dest.with_extension("part");
    let result = fetch_to_part(url, f, &part, cancel, progress).and_then(|()| {
        std::fs::rename(&part, dest).with_context(|| format!("installing {}", dest.display()))
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

/// Downloads and verifies `url` into `part`; the caller removes `part` when this fails.
fn fetch_to_part(
    url: &str,
    f: &ModelFile,
    part: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<()> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        // Generous total budget for ~400 MB on a slow line; cancel works between reads.
        .timeout_recv_body(Some(Duration::from_secs(4 * 3600)))
        .build()
        .into();
    let resp = agent
        .get(url)
        .header(
            "User-Agent",
            concat!("recollect/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .with_context(|| format!("downloading {url}"))?;
    let mut reader = resp.into_body().into_reader();
    let mut out =
        std::fs::File::create(part).with_context(|| format!("creating {}", part.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut n = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("download cancelled");
        }
        let k = reader
            .read(&mut buf)
            .with_context(|| format!("downloading {url}"))?;
        if k == 0 {
            break;
        }
        hasher.update(&buf[..k]);
        out.write_all(&buf[..k])
            .with_context(|| format!("writing {}", part.display()))?;
        n += k as u64;
        if n > f.size {
            bail!("{}: server sent more data than expected", f.remote);
        }
        progress(n);
    }
    out.sync_all()
        .with_context(|| format!("writing {}", part.display()))?;
    drop(out);
    verify(part, f, n, &hex::encode(hasher.finalize()))
}

fn verify(part: &Path, f: &ModelFile, size: u64, sha: &str) -> Result<()> {
    if size != f.size || sha != f.sha256 {
        let _ = std::fs::remove_file(part);
        bail!(
            "{}: integrity check failed (got {size} bytes, sha256 {sha}; expected {} bytes, {})",
            f.remote,
            f.size,
            f.sha256
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::registry::CLIP_B32;

    #[test]
    fn installed_requires_all_files_with_right_size() {
        let root = tempfile::tempdir().unwrap();
        assert!(!is_installed(root.path(), &CLIP_B32));
        let dir = model_dir(root.path(), &CLIP_B32);
        std::fs::create_dir_all(&dir).unwrap();
        for f in CLIP_B32.files {
            std::fs::write(dir.join(f.local), b"short").unwrap();
        }
        assert!(!is_installed(root.path(), &CLIP_B32));
    }

    #[test]
    fn cleanup_removes_only_unknown_regular_files() {
        let d = tempfile::tempdir().unwrap();
        for f in CLIP_B32.files {
            std::fs::write(d.path().join(f.local), b"x").unwrap();
        }
        std::fs::write(d.path().join("old_fp32.onnx"), b"x").unwrap();
        std::fs::create_dir(d.path().join("subdir")).unwrap();
        std::fs::write(d.path().join("subdir/keep.txt"), b"x").unwrap();
        remove_stale_files(d.path(), &CLIP_B32);
        assert!(!d.path().join("old_fp32.onnx").exists());
        assert!(d.path().join("subdir/keep.txt").exists());
        for f in CLIP_B32.files {
            assert!(d.path().join(f.local).exists());
        }
    }

    #[test]
    fn verify_rejects_and_removes_bad_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.part");
        std::fs::write(&p, b"abc").unwrap();
        let f = CLIP_B32.files[0];
        assert!(verify(&p, &f, 3, "00").is_err());
        assert!(!p.exists());
    }
}
