//! Application directories and persisted user settings.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Environment variable that relocates all application data under one directory.
/// Used by tests, benchmarks and portable installs.
pub const HOME_ENV: &str = "RECOLLECT_HOME";

/// A sibling of `path` that is unique to this process and call, for writing a file that is
/// then renamed into place (two writers never share a temporary file).
pub(crate) fn temp_sibling(path: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(
        "{name}.{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Replaces `path` atomically: `write` fills a unique temporary file next to it, which is
/// then renamed over `path`. The temporary file is removed if anything fails.
pub(crate) fn replace_file(path: &Path, write: impl FnOnce(&Path) -> Result<()>) -> Result<()> {
    let tmp = temp_sibling(path);
    let res = write(&tmp).and_then(|()| {
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
    });
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

/// Where Recollect keeps its files. Nothing is ever written inside library folders.
#[derive(Debug, Clone)]
pub struct Paths {
    /// Durable data: the library database and downloaded models.
    pub data: PathBuf,
    /// Re-creatable data: thumbnails.
    pub cache: PathBuf,
    /// User settings.
    pub config: PathBuf,
}

impl Paths {
    /// Platform directories (`~/.local/share/recollect`, `%APPDATA%\recollect`,
    /// `~/Library/Application Support/recollect`, ...) unless [`HOME_ENV`] is set.
    pub fn from_env() -> Result<Self> {
        if let Some(home) = std::env::var_os(HOME_ENV) {
            return Ok(Self::under(Path::new(&home)));
        }
        let dirs = directories::ProjectDirs::from("", "", "recollect")
            .context("could not determine the user's home directory")?;
        Ok(Self {
            data: dirs.data_dir().to_path_buf(),
            cache: dirs.cache_dir().to_path_buf(),
            config: dirs.config_dir().to_path_buf(),
        })
    }

    /// All directories below a single root (portable mode, tests).
    pub fn under(root: &Path) -> Self {
        Self {
            data: root.join("data"),
            cache: root.join("cache"),
            config: root.join("config"),
        }
    }

    pub fn models_dir(&self) -> PathBuf {
        self.data.join("models")
    }

    pub fn db_path(&self) -> PathBuf {
        self.data.join("library.db")
    }

    pub fn thumbs_dir(&self) -> PathBuf {
        self.cache.join("thumbs")
    }

    pub fn settings_path(&self) -> PathBuf {
        self.config.join("settings.toml")
    }

    pub fn create_all(&self) -> Result<()> {
        for d in [&self.data, &self.cache, &self.config] {
            std::fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
        }
        Ok(())
    }
}

/// Which colour scheme the app uses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    /// Follow the operating system's light or dark setting.
    #[default]
    System,
    Light,
    Dark,
}

/// How large text is drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextSize {
    #[default]
    Normal,
    /// 15 % larger.
    Large,
}

impl TextSize {
    /// The interface zoom factor for this size.
    pub fn zoom(self) -> f32 {
        match self {
            Self::Normal => 1.0,
            Self::Large => 1.15,
        }
    }
}

/// User-adjustable settings, stored as TOML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Embedding model id (see [`crate::model::registry`]). `None` = pick by hardware.
    pub model: Option<String>,
    /// Try the GPU (WebGPU) for embedding; falls back to CPU automatically.
    pub use_gpu: bool,
    /// Worker threads for decoding during indexing. `None` = all cores but one.
    pub index_threads: Option<usize>,
    /// Upper bound for the thumbnail disk cache, in megabytes.
    pub thumb_cache_mb: u64,
    /// Light, dark or follow the system.
    pub theme: Theme,
    /// Normal or large text.
    pub text_size: TextSize,
    /// The user chose "Not now" on the invitation to turn on smart search.
    pub search_prompt_dismissed: bool,
    /// The first-run welcome flow was completed or skipped.
    pub welcome_done: bool,
    /// Searches the user ran, newest first (kept on this computer only).
    pub recent_searches: Vec<String>,
    /// Ids of the one-time tips the user has already seen or closed.
    pub tips_dismissed: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model: None,
            use_gpu: true,
            index_threads: None,
            thumb_cache_mb: 1024,
            theme: Theme::System,
            text_size: TextSize::Normal,
            search_prompt_dismissed: false,
            welcome_done: false,
            recent_searches: Vec::new(),
            tips_dismissed: Vec::new(),
        }
    }
}

impl Settings {
    /// Loads settings; a missing file gives defaults, a corrupt file is reported.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("parsing {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let text = toml::to_string_pretty(self).context("serializing settings")?;
        replace_file(path, |tmp| {
            std::fs::write(tmp, text).with_context(|| format!("writing {}", tmp.display()))
        })
    }

    pub fn decode_threads(&self) -> usize {
        self.index_threads
            .unwrap_or_else(|| available_threads().map_or(1, |n| n.saturating_sub(1)))
            .max(1)
    }
}

/// How many threads the machine can run at once, if it says. Asked once: the answer is read
/// again and again by screens that redraw every frame.
pub fn available_threads() -> Option<usize> {
    static THREADS: OnceLock<Option<usize>> = OnceLock::new();
    *THREADS.get_or_init(|| {
        std::thread::available_parallelism()
            .ok()
            .map(std::num::NonZero::get)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cfg/settings.toml");
        assert_eq!(Settings::load(&p).unwrap(), Settings::default());
        let s = Settings {
            model: Some("clip-vit-b32".into()),
            use_gpu: false,
            index_threads: Some(3),
            thumb_cache_mb: 10,
            theme: Theme::Dark,
            text_size: TextSize::Large,
            search_prompt_dismissed: true,
            welcome_done: true,
            recent_searches: vec!["snow".into(), "dog".into()],
            tips_dismissed: vec!["search".into()],
        };
        s.save(&p).unwrap();
        assert_eq!(Settings::load(&p).unwrap(), s);
        // Unknown/missing keys fall back to defaults.
        std::fs::write(&p, "use_gpu = false\n").unwrap();
        let l = Settings::load(&p).unwrap();
        assert!(!l.use_gpu);
        assert_eq!(l.thumb_cache_mb, 1024);
        // Settings files from before the appearance options still load.
        assert_eq!(l.theme, Theme::System);
        assert_eq!(l.text_size, TextSize::Normal);
        std::fs::write(&p, "theme = \"Light\"\ntext_size = \"Large\"\n").unwrap();
        let l = Settings::load(&p).unwrap();
        assert_eq!((l.theme, l.text_size), (Theme::Light, TextSize::Large));
    }

    #[test]
    fn replace_file_leaves_no_temp_files() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.toml");
        replace_file(&p, |t| Ok(std::fs::write(t, "one")?)).unwrap();
        replace_file(&p, |t| Ok(std::fs::write(t, "two")?)).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "two");
        // A failing writer keeps the old content and removes its temp file.
        let failed = replace_file(&p, |t| {
            std::fs::write(t, "partial")?;
            anyhow::bail!("disk full")
        });
        assert!(failed.is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "two");
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
        assert_ne!(temp_sibling(&p), temp_sibling(&p));
    }

    #[test]
    fn paths_under_root() {
        let p = Paths::under(Path::new("/x"));
        assert_eq!(p.db_path(), Path::new("/x/data/library.db"));
        assert_eq!(p.thumbs_dir(), Path::new("/x/cache/thumbs"));
    }
}
