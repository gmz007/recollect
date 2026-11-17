//! Toasts: short messages about what just happened, and what the library reports.

use super::{App, guide};
use crate::library::AlbumOp;
use std::time::{Duration, Instant};

/// Short messages shown as toasts. An error stays visible until the next error or
/// [`Notices::ERROR_FOR`]; later plain messages do not push it out (they show once it is gone).
#[derive(Default)]
pub(super) struct Notices {
    pub(super) items: Vec<Notice>,
    /// While the pointer rests on the toast: since when. Timers stand still meanwhile.
    held_since: Option<Instant>,
}

/// What the "Undo" button of a confirmation puts back.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Undo {
    /// `files` were just put into `album`: take them out again.
    TakeOutOfAlbum { album: i64, files: Vec<i64> },
    /// `files` were just taken out of `album`: put them back.
    PutBackInAlbum { album: i64, files: Vec<i64> },
    /// Favorite marks that were changed: each file's mark as it was.
    Favorites(Vec<(i64, bool)>),
}

pub(super) struct Notice {
    pub(super) text: String,
    pub(super) at: Instant,
    pub(super) error: bool,
    /// Technical text, shown only when the user unfolds "Details".
    pub(super) details: Option<String>,
    /// A button that tries the failed thing again.
    pub(super) action: Option<NoticeAction>,
    /// A confirmation of something the user did, with the way back.
    pub(super) undo: Option<Undo>,
}

/// What the "Try again" button of an error does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NoticeAction {
    Download,
    Scan,
}

impl Notices {
    const INFO_FOR: Duration = Duration::from_secs(5);
    const ERROR_FOR: Duration = Duration::from_secs(10);
    /// More than this many pending messages are never useful.
    pub(super) const MAX: usize = 6;

    fn ttl(n: &Notice) -> Duration {
        if n.error {
            Self::ERROR_FOR
        } else {
            Self::INFO_FOR
        }
    }

    pub(super) fn push(&mut self, text: String, error: bool, now: Instant) {
        self.push_with(text, error, now, None, None);
    }

    fn push_with(
        &mut self,
        text: String,
        error: bool,
        now: Instant,
        details: Option<String>,
        action: Option<NoticeAction>,
    ) {
        self.expire(now);
        if error {
            // A newer error replaces the older ones.
            self.items.retain(|n| !n.error);
        }
        self.items.push(Notice {
            text,
            at: now,
            error,
            details,
            action,
            undo: None,
        });
        while self.items.len() > Self::MAX {
            self.items.remove(0);
        }
    }

    /// Pauses every timer while `hovered` (the pointer is on the toast) and, once it leaves,
    /// gives each message the time it was held.
    pub(super) fn set_hovered(&mut self, hovered: bool, now: Instant) {
        match (hovered, self.held_since) {
            (true, None) => self.held_since = Some(now),
            (false, Some(since)) => {
                self.held_since = None;
                let held = now.saturating_duration_since(since);
                for n in &mut self.items {
                    n.at += held;
                }
            }
            _ => {}
        }
    }

    fn expire(&mut self, now: Instant) {
        let now = self.held_since.unwrap_or(now);
        self.items
            .retain(|n| now.saturating_duration_since(n.at) < Self::ttl(n));
    }

    /// What to show: the error if there is one, else the latest message.
    #[cfg(test)]
    pub(super) fn current(&self) -> Option<&str> {
        self.items
            .iter()
            .find(|n| n.error)
            .or(self.items.last())
            .map(|n| n.text.as_str())
    }

    /// The notice [`Notices::current`] shows.
    pub(super) fn current_notice(&self) -> Option<&Notice> {
        self.items.iter().find(|n| n.error).or(self.items.last())
    }

    /// Time until the next message disappears (to schedule a repaint).
    fn next_expiry(&self, now: Instant) -> Option<Duration> {
        if self.held_since.is_some() {
            return None;
        }
        self.items
            .iter()
            .map(|n| Self::ttl(n).saturating_sub(now.saturating_duration_since(n.at)))
            .min()
    }
}

impl App {
    pub(super) fn notify(&mut self, msg: impl Into<String>) {
        self.notices.push(msg.into(), false, Instant::now());
    }

    /// A confirmation with an "Undo" button; it replaces the selection bar while it shows.
    pub(super) fn notify_undoable(&mut self, msg: impl Into<String>, undo: Undo) {
        self.notices.push(msg.into(), false, Instant::now());
        if let Some(n) = self.notices.items.last_mut() {
            n.undo = Some(undo);
        }
    }

    /// Whether a confirmation with "Undo" is showing (the selection bar gives way to it).
    pub(super) fn confirmation_showing(&self) -> bool {
        self.notices
            .current_notice()
            .is_some_and(|n| n.undo.is_some())
    }

    /// Undoes what a confirmation stands for. The writes are silent: no confirmation of their
    /// own, so Undo is not offered for Undo.
    pub(super) fn undo(&mut self, undo: Undo) {
        match undo {
            Undo::TakeOutOfAlbum { album, files } => {
                let t = self.lib.queue_album(AlbumOp::Remove { album, files });
                self.silent_album_writes.insert(t);
            }
            Undo::PutBackInAlbum { album, files } => {
                let t = self.lib.queue_album(AlbumOp::Add { album, files });
                self.silent_album_writes.insert(t);
            }
            Undo::Favorites(previous) => {
                for fav in [true, false] {
                    let ids: Vec<i64> = (previous.iter())
                        .filter(|(_, was)| *was == fav)
                        .map(|(id, _)| *id)
                        .collect();
                    if !ids.is_empty() {
                        self.set_favorites(&ids, fav);
                    }
                }
            }
        }
        self.notify("Undone");
    }

    /// Like [`Self::notify`] for failures: stays visible longer and is not pushed out by the
    /// messages that follow.
    pub(super) fn notify_error(&mut self, msg: impl Into<String>) {
        self.notices.push(msg.into(), true, Instant::now());
    }

    /// An error in two parts: what happened and what to do (`msg`), and the technical text
    /// that goes under "Details". `action` adds a "Try again" button.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "callers pass temporaries such as format!() output or an error"
    )]
    pub(super) fn notify_problem(
        &mut self,
        msg: impl Into<String>,
        details: impl ToString,
        action: Option<NoticeAction>,
    ) {
        let details = details.to_string();
        self.notices.push_with(
            msg.into(),
            true,
            Instant::now(),
            (!details.is_empty()).then_some(details),
            action,
        );
    }

    /// Runs what a toast's "Try again" button stands for.
    pub(super) fn retry(&mut self, action: NoticeAction) {
        match action {
            NoticeAction::Download => self.turn_on_search(),
            NoticeAction::Scan => self.start_indexing(),
        }
    }

    /// Toasts for what the status says, and expiry of old ones.
    pub(super) fn report_status_and_notices(&mut self, ctx: &egui::Context) {
        self.report_status();
        let now = Instant::now();
        self.notices.expire(now);
        if let Some(left) = self.notices.next_expiry(now) {
            ctx.request_repaint_after(left);
        }
    }

    /// Turns what the library reports into toasts: each failure once (also when the same
    /// message comes again), and folders that could not be reached once per finished run (by
    /// its counter, so a run too short to be seen running is reported too).
    fn report_status(&mut self) {
        if self.status.error_seq != self.error_seq_seen {
            self.error_seq_seen = self.status.error_seq;
            if let Some(e) = self.status.last_error.clone() {
                let (text, action) = guide::describe_failure(self.status.last_error_kind);
                self.notify_problem(text, e, action);
            }
        }
        if self.status.run_seq == self.run_seq_seen {
            return;
        }
        self.run_seq_seen = self.status.run_seq;
        let skipped = |r: &crate::library::RunSummary| !r.skipped_folders.is_empty();
        if self.status.last_run.as_ref().is_some_and(skipped) {
            self.notify("Some folders couldn't be reached, so they were left as they were.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Notices;
    use std::time::{Duration, Instant};

    #[test]
    fn an_error_is_not_swallowed_by_later_messages() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut n = Notices::default();
        n.push("could not save".into(), true, t0);
        n.push("Added 2 items".into(), false, at(1));
        assert_eq!(n.current(), Some("could not save"));
        // A newer error replaces the older one.
        n.push("disk full".into(), true, at(2));
        assert_eq!(n.current(), Some("disk full"));
        // Plain messages expire sooner than errors; the error outlives them.
        n.expire(at(7));
        assert_eq!(n.current(), Some("disk full"));
        assert_eq!(n.next_expiry(at(7)), Some(Duration::from_secs(5)));
        // After the error's time the message is gone.
        n.expire(at(13));
        assert_eq!(n.current(), None);
        // With no error, the latest message shows.
        n.push("one".into(), false, at(20));
        n.push("two".into(), false, at(21));
        assert_eq!(n.current(), Some("two"));
    }

    #[test]
    fn a_message_stays_while_the_pointer_is_on_it() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut n = Notices::default();
        n.push("Added 2 items".into(), false, t0);
        n.set_hovered(true, at(3));
        n.expire(at(30));
        assert_eq!(n.current(), Some("Added 2 items"));
        assert_eq!(n.next_expiry(at(30)), None);
        // Released after 7 s: the 2 s it had left are still ahead.
        n.set_hovered(false, at(10));
        n.expire(at(11));
        assert_eq!(n.current(), Some("Added 2 items"));
        n.expire(at(13));
        assert_eq!(n.current(), None);
    }
}
