//! Favorite marks and album changes. The GUI queues them and one writer thread applies them
//! in order, so a slow database never blocks a frame and nothing is reordered.

use super::Library;
use crate::store::db::{AlbumError, AlbumInfo};
use anyhow::Result;
#[cfg(test)]
use anyhow::anyhow;
use crossbeam_channel::{Receiver, Sender};
use std::collections::HashMap;
use std::sync::atomic::Ordering;

/// Work for the library writer thread.
pub(super) enum WriteMsg {
    Favorite(i64, bool),
    Album(u64, AlbumOp),
    /// Answered once everything queued before it has been written.
    Flush(Sender<()>),
}

/// A change to the albums, applied by the writer in the order it was queued.
#[derive(Debug, Clone, PartialEq)]
pub enum AlbumOp {
    /// Creates an album and puts `add` in it, all in one transaction: if anything fails,
    /// nothing is created.
    Create {
        name: String,
        add: Vec<i64>,
    },
    Rename {
        id: i64,
        name: String,
    },
    Delete {
        id: i64,
    },
    Add {
        album: i64,
        files: Vec<i64>,
    },
    Remove {
        album: i64,
        files: Vec<i64>,
    },
}

/// The outcome of one favorites write, as reported by the writer thread.
#[derive(Debug, Clone, PartialEq)]
pub struct FavoriteWrite {
    pub file_id: i64,
    /// The value that was written (or that failed to be written).
    pub fav: bool,
    /// Why the write failed; `None` once it is committed.
    pub error: Option<String>,
}

/// The outcome of one album write.
#[derive(Debug, Clone, PartialEq)]
pub struct AlbumWrite {
    /// What [`Library::queue_album`] returned for this write.
    pub ticket: u64,
    pub op: AlbumOp,
    /// The album that was created, or the one the write was about.
    pub album: Option<i64>,
    /// How many items were added or removed (for `Create`: put into the new album).
    pub changed: usize,
    /// Why the write failed; `None` once it is committed.
    pub error: Option<String>,
    /// The reason when it is one the library refuses on purpose (a bad name, a removed
    /// album); `None` for any other failure.
    pub error_kind: Option<AlbumError>,
}

/// What an applied album change did.
struct Applied {
    album: Option<i64>,
    changed: usize,
    /// Whether anything in the database is different now.
    modified: bool,
}

/// The library writer: applies queued writes in order on one thread and reports each outcome.
/// Consecutive favorite marks collapse to the latest value per file; an album change ends the
/// run, so nothing is ever reordered. Exits when the library is gone.
pub(super) fn library_writer(
    lib: &std::sync::Weak<Library>,
    rx: &Receiver<WriteMsg>,
    favorites: &Sender<FavoriteWrite>,
    albums: &Sender<AlbumWrite>,
) {
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        while let Ok(m) = rx.try_recv() {
            batch.push(m);
        }
        let Some(lib) = lib.upgrade() else {
            return;
        };
        let mut run: HashMap<i64, bool> = HashMap::new();
        let mut flushes = Vec::new();
        for m in batch {
            match m {
                WriteMsg::Favorite(id, fav) => {
                    run.insert(id, fav);
                }
                WriteMsg::Album(ticket, op) => {
                    write_favorites(&lib, &mut run, favorites);
                    let result = write_album(&lib, ticket, op);
                    let _ = albums.send(result);
                }
                WriteMsg::Flush(done) => flushes.push(done),
            }
        }
        write_favorites(&lib, &mut run, favorites);
        lib.notify();
        for done in flushes {
            let _ = done.send(());
        }
    }
}

fn write_favorites(lib: &Library, run: &mut HashMap<i64, bool>, results: &Sender<FavoriteWrite>) {
    let latest = std::mem::take(run);
    for fav in [true, false] {
        let ids: Vec<i64> = latest
            .iter()
            .filter(|(_, v)| **v == fav)
            .map(|(id, _)| *id)
            .collect();
        if ids.is_empty() {
            continue;
        }
        #[cfg(test)]
        let outcome = if lib.fail_favorites.load(Ordering::Acquire) {
            Err(anyhow!("simulated failure"))
        } else {
            lib.db.lock().set_favorite(&ids, fav)
        };
        #[cfg(not(test))]
        let outcome = lib.db.lock().set_favorite(&ids, fav);
        let error = outcome.err().map(|e| format!("{e:#}"));
        if let Some(e) = &error {
            tracing::warn!("could not save the favorite mark: {e}");
        } else {
            lib.favorites_version.fetch_add(1, Ordering::AcqRel);
        }
        for file_id in ids {
            let _ = results.send(FavoriteWrite {
                file_id,
                fav,
                error: error.clone(),
            });
        }
    }
}

fn write_album(lib: &Library, ticket: u64, op: AlbumOp) -> AlbumWrite {
    #[cfg(test)]
    let outcome = if lib.fail_albums.load(Ordering::Acquire) {
        Err(anyhow!("simulated failure"))
    } else {
        lib.apply_album_op(&op)
    };
    #[cfg(not(test))]
    let outcome = lib.apply_album_op(&op);
    let (album, changed, error, error_kind) = match outcome {
        Ok(done) => {
            if done.modified {
                lib.albums_version.fetch_add(1, Ordering::AcqRel);
            }
            (done.album, done.changed, None, None)
        }
        Err(e) => {
            tracing::warn!("could not save the album change: {e:#}");
            let kind = e.downcast_ref::<AlbumError>().cloned();
            (None, 0, Some(format!("{e:#}")), kind)
        }
    };
    AlbumWrite {
        ticket,
        op,
        album,
        changed,
        error,
        error_kind,
    }
}

impl Library {
    /// Marks files as favorites (or removes the mark), waiting for the write. The GUI uses
    /// [`Self::queue_favorite`] instead.
    pub fn set_favorite(&self, file_ids: &[i64], fav: bool) -> Result<()> {
        self.db.lock().set_favorite(file_ids, fav)?;
        self.favorites_version.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    /// Queues a favorite mark for the writer thread and returns at once. Writes are applied
    /// in order; consecutive writes for one file collapse into the latest value. The outcome
    /// arrives through [`Self::take_favorite_results`].
    pub fn queue_favorite(&self, file_id: i64, fav: bool) {
        let _ = self.write_tx.send(WriteMsg::Favorite(file_id, fav));
    }

    /// Blocks until every write queued so far (favorites and album changes) has been written
    /// (or has failed).
    pub fn wait_writes(&self) {
        self.wait_writes_for(None);
    }

    /// Like [`Self::wait_writes`] but gives up after `limit` (e.g. at exit, when the database
    /// may be busy). Returns true when everything queued has been written.
    pub fn wait_writes_for(&self, limit: impl Into<Option<std::time::Duration>>) -> bool {
        let (tx, rx) = crossbeam_channel::bounded(1);
        if self.write_tx.send(WriteMsg::Flush(tx)).is_err() {
            return false;
        }
        match limit.into() {
            Some(l) => rx.recv_timeout(l).is_ok(),
            None => rx.recv().is_ok(),
        }
    }

    /// Queues an album change for the writer thread and returns its ticket at once. Changes
    /// are applied in the order queued, also relative to favorite marks; the outcome (with the
    /// ticket, and the new album's id for `Create`) arrives through
    /// [`Self::take_album_results`].
    pub fn queue_album(&self, op: AlbumOp) -> u64 {
        let ticket = self.next_ticket.fetch_add(1, Ordering::AcqRel);
        let _ = self.write_tx.send(WriteMsg::Album(ticket, op));
        ticket
    }

    /// Outcomes of the album writes finished since the last call, oldest first.
    pub fn take_album_results(&self) -> Vec<AlbumWrite> {
        self.album_results.try_iter().collect()
    }

    /// Changes after every committed album write: album lists and album views are stale once
    /// it moves.
    pub fn albums_version(&self) -> u64 {
        self.albums_version.load(Ordering::Acquire)
    }

    /// Applies one album change.
    fn apply_album_op(&self, op: &AlbumOp) -> Result<Applied> {
        let mut db = self.db.lock();
        Ok(match op {
            AlbumOp::Create { name, add } => {
                let (id, n) = db.create_album_with(name, add)?;
                Applied {
                    album: Some(id),
                    changed: n,
                    modified: true,
                }
            }
            AlbumOp::Rename { id, name } => {
                db.rename_album(*id, name)?;
                Applied {
                    album: Some(*id),
                    changed: 0,
                    modified: true,
                }
            }
            AlbumOp::Delete { id } => Applied {
                album: Some(*id),
                changed: 0,
                modified: db.delete_album(*id)?,
            },
            AlbumOp::Add { album, files } => {
                let n = db.add_to_album(*album, files)?;
                Applied {
                    album: Some(*album),
                    changed: n,
                    modified: n > 0,
                }
            }
            AlbumOp::Remove { album, files } => {
                let n = db.remove_from_album(*album, files)?;
                Applied {
                    album: Some(*album),
                    changed: n,
                    modified: n > 0,
                }
            }
        })
    }

    /// All albums with counts and covers.
    pub fn albums(&self) -> Result<Vec<AlbumInfo>> {
        self.db.lock().albums()
    }

    /// Creates an album straight away, bypassing the write queue (tests set up state with it).
    #[cfg(test)]
    pub(crate) fn create_album(&self, name: &str, add: &[i64]) -> Result<i64> {
        let done = self.apply_album_op(&AlbumOp::Create {
            name: name.to_string(),
            add: add.to_vec(),
        })?;
        self.albums_version.fetch_add(1, Ordering::AcqRel);
        Ok(done.album.expect("create returns an id"))
    }

    /// Adds files to an album straight away, bypassing the write queue (tests only).
    #[cfg(test)]
    pub(crate) fn add_to_album(&self, album: i64, files: &[i64]) -> Result<usize> {
        let done = self.apply_album_op(&AlbumOp::Add {
            album,
            files: files.to_vec(),
        })?;
        self.albums_version.fetch_add(1, Ordering::AcqRel);
        Ok(done.changed)
    }

    /// Outcomes of the favorites writes finished since the last call, oldest first.
    pub fn take_favorite_results(&self) -> Vec<FavoriteWrite> {
        self.favorite_results.try_iter().collect()
    }

    /// Changes after every committed favorites write: lists filtered by favorites are stale
    /// once it moves.
    pub fn favorites_version(&self) -> u64 {
        self.favorites_version.load(Ordering::Acquire)
    }
}
