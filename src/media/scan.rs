//! Recursive discovery of media files in library folders.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Image,
    Video,
}

/// A number read from storage that names no known variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownValue {
    pub what: &'static str,
    pub value: i64,
}

impl std::fmt::Display for UnknownValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown {} {}", self.what, self.value)
    }
}

impl std::error::Error for UnknownValue {}

impl From<MediaKind> for i64 {
    fn from(kind: MediaKind) -> i64 {
        match kind {
            MediaKind::Image => 0,
            MediaKind::Video => 1,
        }
    }
}

impl TryFrom<i64> for MediaKind {
    type Error = UnknownValue;

    fn try_from(v: i64) -> Result<Self, UnknownValue> {
        match v {
            0 => Ok(MediaKind::Image),
            1 => Ok(MediaKind::Video),
            value => Err(UnknownValue {
                what: "media kind",
                value,
            }),
        }
    }
}

const IMAGE_EXT: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "webp", "gif", "bmp", "tif", "tiff",
];
const VIDEO_EXT: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "mpg", "mpeg", "mts", "m2ts", "3gp", "ogv",
    "flv",
];

pub fn kind_name(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "image",
        MediaKind::Video => "video",
    }
}

/// Classifies a path by its extension (case-insensitive).
pub fn kind_of(path: &Path) -> Option<MediaKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if IMAGE_EXT.contains(&ext.as_str()) {
        Some(MediaKind::Image)
    } else if VIDEO_EXT.contains(&ext.as_str()) {
        Some(MediaKind::Video)
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFile {
    pub path: PathBuf,
    pub kind: MediaKind,
    pub size: u64,
    /// Modification time, seconds since the Unix epoch.
    pub mtime: i64,
}

/// Outcome of scanning one folder tree.
#[derive(Debug, Default)]
pub struct ScanResult {
    pub files: Vec<ScannedFile>,
    /// Paths that could not be read (permissions, broken links, non-UTF-8 names).
    pub problems: Vec<(PathBuf, String)>,
}

/// Walks `root` recursively. Hidden entries (dot-files) are skipped, symlinks are not followed
/// (avoids cycles and double counting), and non-UTF-8 paths are reported as problems.
pub fn scan(root: &Path, cancel: &dyn Fn() -> bool) -> ScanResult {
    let mut out = ScanResult::default();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancel() {
            break;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                out.problems.push((dir, e.to_string()));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    // Protects the whole directory from being treated as emptied.
                    out.problems.push((dir.clone(), e.to_string()));
                    continue;
                }
            };
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path();
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(e) => {
                    out.problems.push((path, e.to_string()));
                    continue;
                }
            };
            if ft.is_dir() {
                stack.push(path);
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            let Some(kind) = kind_of(&path) else { continue };
            if path.to_str().is_none() {
                out.problems
                    .push((path, "file name is not valid UTF-8".into()));
                continue;
            }
            match entry.metadata() {
                Ok(m) => out.files.push(ScannedFile {
                    kind,
                    size: m.len(),
                    mtime: m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX)),
                    path,
                }),
                Err(e) => out.problems.push((path, e.to_string())),
            }
        }
    }
    out.files.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_extensions() {
        assert_eq!(kind_of(Path::new("a/B.JPG")), Some(MediaKind::Image));
        assert_eq!(kind_of(Path::new("x.webm")), Some(MediaKind::Video));
        assert_eq!(kind_of(Path::new("x.txt")), None);
        assert_eq!(kind_of(Path::new("noext")), None);
    }

    #[test]
    fn scans_recursively_skipping_hidden_and_other_files() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir_all(r.join("sub/été")).unwrap();
        std::fs::create_dir_all(r.join(".hidden")).unwrap();
        for p in [
            "a.jpg",
            "sub/b.PNG",
            "sub/été/c.mp4",
            ".hidden/d.jpg",
            "notes.txt",
            ".e.jpg",
        ] {
            std::fs::write(r.join(p), b"x").unwrap();
        }
        let res = scan(r, &|| false);
        let names: Vec<_> = res
            .files
            .iter()
            .map(|f| {
                f.path
                    .strip_prefix(r)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(names, vec!["a.jpg", "sub/b.PNG", "sub/été/c.mp4"]);
        assert_eq!(res.files[2].kind, MediaKind::Video);
        assert!(res.files.iter().all(|f| f.size == 1 && f.mtime > 0));
    }

    #[test]
    fn missing_root_is_a_problem_not_a_panic() {
        let res = scan(Path::new("/definitely/not/here"), &|| false);
        assert!(res.files.is_empty());
        assert_eq!(res.problems.len(), 1);
    }
}
