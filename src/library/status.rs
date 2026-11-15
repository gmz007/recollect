//! What the library reports about its background work.

use crate::model::Device;
use std::path::PathBuf;

/// What the background worker is doing right now.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Activity {
    #[default]
    Idle,
    Downloading {
        model: String,
        done: u64,
        total: u64,
    },
    Scanning {
        folder: PathBuf,
    },
    /// Reading capture dates (EXIF / video metadata) of new files.
    ReadingDates {
        done: usize,
        total: usize,
    },
    LoadingModel,
    Indexing(IndexProgress),
    SwitchingModel,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct IndexProgress {
    pub done: usize,
    pub total: usize,
    pub errors: usize,
    pub current: Option<PathBuf>,
    /// Items (images or video frames) embedded per second.
    pub rate: f32,
    pub device: Option<Device>,
}

/// Summary of the last finished indexing run.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunSummary {
    pub files: usize,
    pub errors: usize,
    /// Files left for the next run because their drive or file was unreachable.
    pub deferred: usize,
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
    /// Files whose capture date was read during this run.
    pub dates_read: usize,
    pub seconds: f32,
    pub cancelled: bool,
    pub device: Option<Device>,
    /// Folders that were unreachable or looked unmounted (kept as they were).
    pub skipped_folders: Vec<PathBuf>,
}

/// What a recorded failure was doing, so the interface can explain it in plain words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrorKind {
    #[default]
    Other,
    Download,
    AddFolder,
    RemoveFolder,
    Scan,
    SwitchModel,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    pub activity: Activity,
    pub paused: bool,
    /// The technical text of the last failure (shown only under "Details").
    pub last_error: Option<String>,
    /// What the last failure was doing.
    pub last_error_kind: ErrorKind,
    /// Counts every time an error was recorded, so a repeat of the same message is still
    /// noticed as a new failure.
    pub error_seq: u64,
    pub last_run: Option<RunSummary>,
    /// Counts every finished run (each time `last_run` is set), so a run that starts and ends
    /// between two looks at the status is still noticed.
    pub run_seq: u64,
}

impl Status {
    /// Records a failure (and counts it, see [`Status::error_seq`]).
    pub fn set_error(&mut self, message: impl Into<String>) {
        self.set_error_of(ErrorKind::Other, message);
    }

    /// Like [`Status::set_error`], saying what was being done.
    pub fn set_error_of(&mut self, kind: ErrorKind, message: impl Into<String>) {
        self.last_error = Some(message.into());
        self.last_error_kind = kind;
        self.error_seq += 1;
    }
}
