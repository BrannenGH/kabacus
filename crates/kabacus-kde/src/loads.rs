// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The bookkeeping behind `Ledger.load`: which screen loads are
//! cached for which revision, which are under way, and which replies reach
//! the pages. No Qt here, so it's tested on its own; the bridge only spawns
//! the fetches and emits what this decides.

use std::collections::HashMap;

/// Replies by key (`view|args`), as text, with the revision each is for.
#[derive(Default)]
pub struct Loads {
    /// Only successes are kept: a failed load is tried again next time,
    /// and the last good reply stays readable (offline).
    cache: HashMap<String, (i32, String)>,
    /// The loads under way, by key, with the revision each is for.
    loading: HashMap<String, i32>,
    /// Bumped by `forget`: a load from before it is for a session or
    /// server that's gone, and is dropped.
    epoch: u64,
}

/// What `start` decided.
#[derive(Debug, PartialEq)]
pub struct Start {
    /// The last good reply, possibly for an older revision.
    pub cached: Option<String>,
    /// The cached reply isn't for this revision: one is on its way.
    pub pending: bool,
    /// Begin a fetch tagged with this epoch (none is under way yet).
    pub fetch: Option<u64>,
}

impl Loads {
    /// A page asks for `key` at `revision`.
    pub fn start(&mut self, key: &str, revision: i32) -> Start {
        let cached = self.cache.get(key);
        let pending = !matches!(cached, Some((r, _)) if *r == revision);
        let fetch = (pending && self.loading.get(key) != Some(&revision)).then(|| {
            self.loading.insert(key.to_string(), revision);
            self.epoch
        });
        Start {
            cached: cached.map(|(_, reply)| reply.clone()),
            pending,
            fetch,
        }
    }

    /// A fetch for `key` at `revision` (started in `epoch`) came back; the
    /// app is now at `current`. Returns the reply to hand the pages, if
    /// they should see it.
    pub fn finish(
        &mut self,
        key: &str,
        revision: i32,
        epoch: u64,
        current: i32,
        ok: bool,
        reply: String,
    ) -> Option<String> {
        if epoch != self.epoch {
            return None;
        }
        if self.loading.get(key) == Some(&revision) {
            self.loading.remove(key);
        }
        if matches!(self.cache.get(key), Some((r, _)) if *r > revision) {
            return None;
        }
        if ok {
            self.cache
                .insert(key.to_string(), (revision, reply.clone()));
        }
        // The books changed since: pages asked again, and that answer is
        // the one to show.
        (revision == current).then_some(reply)
    }

    /// `refresh` brought `reply` for `key`, for `revision`.
    pub fn preloaded(&mut self, key: String, revision: i32, reply: String) {
        self.cache.insert(key, (revision, reply));
    }

    /// Forget every reply and drop the loads under way (signed out, or
    /// another server).
    pub fn forget(&mut self) {
        self.cache.clear();
        self.loading.clear();
        self.epoch += 1;
    }

    /// Drop the loads under way but keep the replies (signed in again: a
    /// load from the old session would only report it expired).
    pub fn restart(&mut self) {
        self.loading.clear();
        self.epoch += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fetched(loads: &mut Loads, key: &str, revision: i32) -> u64 {
        loads.start(key, revision).fetch.expect("a fetch starts")
    }
    #[test]
    fn a_cached_reply_comes_at_once_and_a_fresh_one_is_fetched_once() {
        let mut loads = Loads::default();
        let first = loads.start("overview|{}", 1);
        assert_eq!(
            first,
            Start {
                cached: None,
                pending: true,
                fetch: Some(0)
            }
        );
        // Asked again while it's on its way: no second fetch.
        assert_eq!(loads.start("overview|{}", 1).fetch, None);
        let shown = loads.finish("overview|{}", 1, 0, 1, true, "A".into());
        assert_eq!(shown.as_deref(), Some("A"));
        let again = loads.start("overview|{}", 1);
        assert_eq!(
            again,
            Start {
                cached: Some("A".into()),
                pending: false,
                fetch: None
            }
        );
        // The books changed: the old reply at once, and a new fetch.
        let next = loads.start("overview|{}", 2);
        assert_eq!(
            next,
            Start {
                cached: Some("A".into()),
                pending: true,
                fetch: Some(0)
            }
        );
    }
    #[test]
    fn a_failed_load_is_shown_but_the_last_good_reply_stays_cached() {
        let mut loads = Loads::default();
        let epoch = fetched(&mut loads, "funds|{}", 1);
        loads.finish("funds|{}", 1, epoch, 1, true, "good".into());
        let epoch = fetched(&mut loads, "funds|{}", 2);
        let shown = loads.finish("funds|{}", 2, epoch, 2, false, "offline".into());
        assert_eq!(shown.as_deref(), Some("offline"));
        let start = loads.start("funds|{}", 2);
        assert_eq!(start.cached.as_deref(), Some("good"));
        assert!(
            start.pending && start.fetch.is_some(),
            "a failure is tried again"
        );
    }
    #[test]
    fn a_reply_older_than_the_books_is_cached_but_not_shown() {
        let mut loads = Loads::default();
        let old = fetched(&mut loads, "register|{}", 1);
        // A write bumped the revision and the page asked again.
        let new = fetched(&mut loads, "register|{}", 2);
        assert_eq!(
            loads.finish("register|{}", 1, old, 2, true, "before".into()),
            None
        );
        assert_eq!(
            loads
                .finish("register|{}", 2, new, 2, true, "after".into())
                .as_deref(),
            Some("after")
        );
    }
    #[test]
    fn a_reply_older_than_a_preloaded_one_is_dropped() {
        let mut loads = Loads::default();
        let old = fetched(&mut loads, "cards|{}", 1);
        loads.preloaded("cards|{}".into(), 2, "preloaded".into());
        assert_eq!(
            loads.finish("cards|{}", 1, old, 2, true, "old".into()),
            None
        );
        assert_eq!(
            loads.start("cards|{}", 2).cached.as_deref(),
            Some("preloaded")
        );
    }
    #[test]
    fn nothing_from_before_a_sign_out_or_server_change_comes_back() {
        let mut loads = Loads::default();
        let epoch = fetched(&mut loads, "people|{}", 1);
        loads.preloaded("overview|{}".into(), 1, "theirs".into());
        loads.forget();
        assert_eq!(
            loads.finish("people|{}", 1, epoch, 2, true, "theirs".into()),
            None
        );
        assert_eq!(loads.start("overview|{}", 2).cached, None);
        assert_eq!(loads.start("people|{}", 2).cached, None);
    }
    #[test]
    fn signing_in_again_drops_old_loads_but_keeps_what_was_shown() {
        let mut loads = Loads::default();
        let epoch = fetched(&mut loads, "accounts|{}", 1);
        loads.preloaded("funds|{}".into(), 1, "kept".into());
        loads.restart();
        assert_eq!(
            loads.finish("accounts|{}", 1, epoch, 1, false, "expired".into()),
            None
        );
        assert_eq!(loads.start("funds|{}", 1).cached.as_deref(), Some("kept"));
        assert!(loads.start("accounts|{}", 1).fetch.is_some(), "asked again");
    }
}
