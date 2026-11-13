//! Model download. **This is the only module in Recollect that uses the network**, and it is
//! only invoked after the user explicitly agreed to download a model.

use super::registry::{ModelFile, ModelSpec};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Directory where a model's files live.
pub fn model_dir(models_root: &Path, spec: &ModelSpec) -> PathBuf {
    models_root.join(spec.id)
}

/// True if every file of the model is present with the expected size.
/// (Checksums are verified once, at download time.)
pub fn is_installed(models_root: &Path, spec: &ModelSpec) -> bool {
    let dir = model_dir(models_root, spec);
    spec.files.iter().all(|f| {
        std::fs::metadata(dir.join(f.local))
            .map(|m| m.len() == f.size)
            .unwrap_or(false)
    })
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
        if std::fs::metadata(&dest)
            .map(|m| m.len() == f.size)
            .unwrap_or(false)
        {
            done += f.size;
            progress(done, total);
            continue;
        }
        fetch_file(&spec.url(f), f, &dest, cancel, &mut |n| {
            progress(done + n, total)
        })?;
        done += f.size;
    }
    Ok(())
}

fn fetch_file(
    url: &str,
    f: &ModelFile,
    dest: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<()> {
    let part = dest.with_extension("part");
    let resp = ureq::get(url)
        .header(
            "User-Agent",
            concat!("recollect/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .with_context(|| format!("downloading {url}"))?;
    let mut reader = resp.into_body().into_reader();
    let mut out = std::fs::File::create(&part)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut n = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(out);
            let _ = std::fs::remove_file(&part);
            bail!("download cancelled");
        }
        let k = reader.read(&mut buf)?;
        if k == 0 {
            break;
        }
        hasher.update(&buf[..k]);
        out.write_all(&buf[..k])?;
        n += k as u64;
        progress(n);
    }
    out.sync_all()?;
    drop(out);
    verify(&part, f, n, &hex::encode(hasher.finalize()))?;
    std::fs::rename(&part, dest)?;
    Ok(())
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
    fn verify_rejects_and_removes_bad_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.part");
        std::fs::write(&p, b"abc").unwrap();
        let f = CLIP_B32.files[0];
        assert!(verify(&p, &f, 3, "00").is_err());
        assert!(!p.exists());
    }
}
