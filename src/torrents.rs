//! What the window knows about each torrent besides its row: its state, when it was added,
//! whether the download queue holds it, and what the database last received for it. The
//! queue is decided here, from this alone.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::engine::TorrentUiState;

/// What the database last received for a torrent: state, downloaded, total, total
/// pieces, downloaded pieces.
pub type PersistedSnapshot = (&'static str, u64, u64, u64, u64);

/// How often the progress of a running torrent is saved. Every write is synced to disk, and
/// a download changes its figures every second; the state and the size are saved at once.
pub const PROGRESS_SAVE_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug)]
struct Entry {
    state: TorrentUiState,
    /// Paused by the queue, not by the user: it starts when a slot frees up, and it is
    /// stored as downloading, so after a restart too.
    queued: bool,
    /// When it was added, then the order this session learnt of it in: the queue starts
    /// the oldest first and pauses the newest first.
    added: (i64, u64),
    /// What was last sent to the database, and when.
    persisted: Option<(PersistedSnapshot, Instant)>,
}

/// The torrents of the window.
#[derive(Debug, Default)]
pub struct Torrents {
    entries: HashMap<String, Entry>,
    /// Deleted torrents, whose late updates are dropped.
    deleted: HashSet<String>,
    next_seq: u64,
}

/// What the queue does: the torrents to pause and the ones to start.
#[derive(Debug, Default, PartialEq)]
pub struct QueuePlan {
    pub pause: Vec<String>,
    pub start: Vec<String>,
}

impl Torrents {
    /// Adds a torrent from the database and returns the state to show. One stored as
    /// downloading was running, or waiting for a slot, when Rill last stopped: it waits
    /// for the queue.
    pub fn restore(&mut self, hash: &str, stored_state: &str, added_at: i64) -> TorrentUiState {
        let state = match stored_state {
            "completed" => TorrentUiState::Completed,
            "error" => TorrentUiState::Error,
            _ => TorrentUiState::Paused,
        };
        self.insert(hash, state, added_at, stored_state == "downloading");
        state
    }

    /// Adds a torrent seen for the first time.
    pub fn add(&mut self, hash: &str, state: TorrentUiState, added_at: i64) {
        self.insert(hash, state, added_at, false);
    }

    fn insert(&mut self, hash: &str, state: TorrentUiState, added_at: i64, queued: bool) {
        self.deleted.remove(hash);
        self.next_seq += 1;
        self.entries.insert(
            hash.to_string(),
            Entry {
                state,
                queued,
                added: (added_at, self.next_seq),
                persisted: None,
            },
        );
    }

    /// When the torrent was added, and where it came in this session, which is what the
    /// list falls back to ordering by.
    pub fn added(&self, hash: &str) -> (i64, u64) {
        self.entries.get(hash).map_or((0, 0), |entry| entry.added)
    }

    pub fn set_state(&mut self, hash: &str, state: TorrentUiState) {
        if let Some(entry) = self.entries.get_mut(hash) {
            entry.state = state;
        }
    }

    /// The user paused or resumed the torrent: the queue no longer holds it.
    pub fn leave_queue(&mut self, hash: &str) {
        if let Some(entry) = self.entries.get_mut(hash) {
            entry.queued = false;
        }
    }

    pub fn delete(&mut self, hash: &str) {
        self.entries.remove(hash);
        self.deleted.insert(hash.to_string());
    }

    /// Lets a torrent deleted earlier in this session come back.
    pub fn undelete(&mut self, hash: &str) {
        self.deleted.remove(hash);
    }

    pub fn is_deleted(&self, hash: &str) -> bool {
        self.deleted.contains(hash)
    }

    /// How the database spells the state of a torrent: one the queue holds is stored as
    /// downloading.
    pub fn stored_state(&self, hash: &str, state: TorrentUiState) -> &'static str {
        let queued = self.entries.get(hash).is_some_and(|entry| entry.queued);
        if state == TorrentUiState::Paused && queued {
            "downloading"
        } else {
            state_key(state)
        }
    }

    /// Records `snapshot`, taken at `now`, as sent to the database, and returns whether it is
    /// worth sending: it has a new state or size, or new progress that has not been saved for
    /// [`PROGRESS_SAVE_INTERVAL`].
    pub fn record_snapshot(
        &mut self,
        hash: &str,
        snapshot: PersistedSnapshot,
        now: Instant,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(hash) else {
            return false;
        };
        if let Some((saved, at)) = entry.persisted {
            let (state, _, total, total_pieces, _) = snapshot;
            let same_kind = (saved.0, saved.2, saved.3) == (state, total, total_pieces);
            if saved == snapshot
                || (same_kind && now.saturating_duration_since(at) < PROGRESS_SAVE_INTERVAL)
            {
                return false;
            }
        }
        entry.persisted = Some((snapshot, now));
        true
    }

    /// The database did not take `snapshot`: forget it, so that the next identical one is
    /// sent again. A newer one recorded in the meantime stays.
    pub fn snapshot_failed(&mut self, hash: &str, snapshot: PersistedSnapshot) {
        if let Some(entry) = self.entries.get_mut(hash)
            && entry.persisted.is_some_and(|(saved, _)| saved == snapshot)
        {
            entry.persisted = None;
        }
    }

    /// Decides the queue, when at most `limit` downloads may run and `is_running` says
    /// which torrents the engine runs: the newest downloads over the limit are paused and
    /// held, or the oldest held ones start while there is room.
    pub fn plan_queue(&mut self, limit: usize, is_running: impl Fn(&str) -> bool) -> QueuePlan {
        let mut running = Vec::new();
        let mut waiting = Vec::new();
        for (hash, entry) in &self.entries {
            if entry.state == TorrentUiState::Completed {
                continue;
            }
            if is_running(hash) {
                running.push((hash, entry.added));
            } else if entry.queued {
                waiting.push((hash, entry.added));
            }
        }

        let mut plan = QueuePlan::default();
        if running.len() > limit {
            running.sort_by_key(|&(_, added)| std::cmp::Reverse(added));
            let excess = running.len() - limit;
            plan.pause = running[..excess]
                .iter()
                .map(|(h, _)| h.to_string())
                .collect();
        } else {
            waiting.sort_by_key(|&(_, added)| added);
            let room = limit - running.len();
            plan.start = waiting
                .iter()
                .take(room)
                .map(|(h, _)| h.to_string())
                .collect();
        }
        for hash in &plan.pause {
            if let Some(entry) = self.entries.get_mut(hash) {
                entry.queued = true;
            }
        }
        for hash in &plan.start {
            self.leave_queue(hash);
        }
        plan
    }
}

/// How the database spells a state.
pub fn state_key(state: TorrentUiState) -> &'static str {
    match state {
        TorrentUiState::Downloading => "downloading",
        TorrentUiState::Paused => "paused",
        TorrentUiState::Completed => "completed",
        TorrentUiState::Error => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TorrentUiState::*;

    fn running(hashes: &'static [&'static str]) -> impl Fn(&str) -> bool {
        move |hash| hashes.contains(&hash)
    }

    fn plan(pause: &[&str], start: &[&str]) -> QueuePlan {
        QueuePlan {
            pause: pause.iter().map(|h| h.to_string()).collect(),
            start: start.iter().map(|h| h.to_string()).collect(),
        }
    }

    #[test]
    fn restored_torrents_show_their_stored_state() {
        let mut torrents = Torrents::default();
        assert_eq!(torrents.restore("a", "completed", 1), Completed);
        assert_eq!(torrents.restore("b", "error", 1), Error);
        assert_eq!(torrents.restore("c", "paused", 1), Paused);
        assert_eq!(torrents.restore("d", "downloading", 1), Paused);
    }

    #[test]
    fn torrents_left_downloading_start_as_far_as_the_limit_allows() {
        let mut torrents = Torrents::default();
        torrents.restore("new", "downloading", 3);
        torrents.restore("old", "downloading", 1);
        torrents.restore("mid", "downloading", 2);
        torrents.restore("paused", "paused", 0);
        torrents.restore("failed", "error", 0);

        assert_eq!(
            torrents.plan_queue(2, running(&[])),
            plan(&[], &["old", "mid"])
        );
        // Started once, a torrent is off the queue.
        assert_eq!(
            torrents.plan_queue(2, running(&["old"])),
            plan(&[], &["new"])
        );
        assert_eq!(
            torrents.plan_queue(2, running(&["old", "mid"])),
            plan(&[], &[])
        );
    }

    #[test]
    fn the_newest_downloads_over_the_limit_are_paused_and_resume_later() {
        let mut torrents = Torrents::default();
        for (hash, added) in [("a", 1), ("b", 2), ("c", 3)] {
            torrents.add(hash, Downloading, added);
        }
        let all = running(&["a", "b", "c"]);
        assert_eq!(torrents.plan_queue(1, all), plan(&["c", "b"], &[]));
        torrents.set_state("b", Paused);
        torrents.set_state("c", Paused);
        // Stored as downloading, so that they wait for the queue after a restart too.
        assert_eq!(torrents.stored_state("b", Paused), "downloading");

        torrents.set_state("a", Completed);
        assert_eq!(torrents.plan_queue(1, running(&["a"])), plan(&[], &["b"]));
        assert_eq!(torrents.stored_state("b", Paused), "paused");
    }

    #[test]
    fn a_torrent_the_user_paused_is_never_started_by_the_queue() {
        let mut torrents = Torrents::default();
        torrents.restore("a", "downloading", 1);
        torrents.set_state("a", Paused);
        torrents.leave_queue("a");
        assert_eq!(torrents.stored_state("a", Paused), "paused");
        assert_eq!(torrents.plan_queue(3, running(&[])), plan(&[], &[]));
    }

    #[test]
    fn completed_torrents_take_no_slot() {
        let mut torrents = Torrents::default();
        torrents.add("done", Completed, 1);
        torrents.restore("waiting", "downloading", 2);
        // The engine still lists a completed torrent as running.
        assert_eq!(
            torrents.plan_queue(1, running(&["done"])),
            plan(&[], &["waiting"])
        );
    }

    #[test]
    fn torrents_added_in_the_same_second_keep_their_order() {
        let mut torrents = Torrents::default();
        for hash in ["first", "second", "third"] {
            torrents.add(hash, Downloading, 7);
        }
        let all = running(&["first", "second", "third"]);
        assert_eq!(torrents.plan_queue(1, all), plan(&["third", "second"], &[]));
    }

    #[test]
    fn deleted_torrents_leave_the_queue_until_added_again() {
        let mut torrents = Torrents::default();
        torrents.restore("a", "downloading", 1);
        torrents.delete("a");
        assert!(torrents.is_deleted("a"));
        assert_eq!(torrents.plan_queue(1, running(&[])), plan(&[], &[]));

        torrents.undelete("a");
        assert!(!torrents.is_deleted("a"));
        torrents.add("b", Downloading, 2);
        torrents.delete("b");
        torrents.add("b", Downloading, 3);
        assert!(!torrents.is_deleted("b"));
    }

    #[test]
    fn a_snapshot_is_saved_once_and_again_after_a_failure() {
        let mut torrents = Torrents::default();
        torrents.add("a", Downloading, 1);
        let first = ("downloading", 1, 10, 1, 0);
        let second = ("downloading", 5, 10, 1, 0);
        let now = Instant::now();
        let later = now + PROGRESS_SAVE_INTERVAL;

        assert!(torrents.record_snapshot("a", first, now));
        assert!(!torrents.record_snapshot("a", first, later));
        torrents.snapshot_failed("a", first);
        assert!(torrents.record_snapshot("a", first, now));

        // A failure of an older write does not forget a newer snapshot.
        assert!(torrents.record_snapshot("a", second, later));
        torrents.snapshot_failed("a", first);
        assert!(!torrents.record_snapshot("a", second, later));

        assert!(!torrents.record_snapshot("unknown", first, now));
    }

    #[test]
    fn progress_is_saved_now_and_then_but_a_new_state_or_size_at_once() {
        let mut torrents = Torrents::default();
        torrents.add("a", Downloading, 1);
        let now = Instant::now();
        let soon = now + Duration::from_secs(1);

        assert!(torrents.record_snapshot("a", ("downloading", 0, 0, 0, 0), now));
        // The metadata arrived: the size is new.
        assert!(torrents.record_snapshot("a", ("downloading", 0, 10, 2, 0), now));
        assert!(!torrents.record_snapshot("a", ("downloading", 4, 10, 2, 1), soon));
        assert!(torrents.record_snapshot("a", ("paused", 4, 10, 2, 1), soon));
        assert!(!torrents.record_snapshot("a", ("paused", 4, 10, 2, 1), soon));
        assert!(torrents.record_snapshot("a", ("downloading", 4, 10, 2, 1), soon));
        assert!(!torrents.record_snapshot("a", ("downloading", 6, 10, 2, 1), soon));
        assert!(torrents.record_snapshot(
            "a",
            ("downloading", 6, 10, 2, 1),
            soon + PROGRESS_SAVE_INTERVAL
        ));
    }
}
