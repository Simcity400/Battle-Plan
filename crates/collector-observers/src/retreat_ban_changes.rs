//! Commanders' retreat bans (DivisionRetreatBans) carried between hours. The bans are each
//! commander's `cannot_retreat_while_*` values in its two modifier blocks' accumulators (the
//! ordinary and separate-HQ blocks), whose only runtime writers are the naval tree family's
//! commander sites ([`engine_mutations::naval_tree_mutations`]): the leader modifier recalculation tail and
//! HQ scaling each clear both blocks, copy the base accumulator in and scale it. Construction, copy
//! construction and destruction bound the leader's lifetime. Modifier recalculation also assigns
//! the blocks' Display strings  (one per block), which the bans do not read. That family owns the
//! patches and forwards each call here ([`forwarded`]), which notifies this family's watch registry
//! for the leader; it never changes game behaviour.
//!
//! Each hour the Presentation job takes every division's commander from the
//! division census's selection, and reads a commander's blocks only when no held read carries
//! its certificate (no forwarded writer touched it since; same epoch). The
//! held bans live in a [`Store`] the capture takes and returns; a capture
//! that does not complete forgets it.
use crate::watch_registry::{Notification, WatchRegistry};
use engine_native::carmy_leader::RetreatBans;
use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

static WATCHES: WatchRegistry = WatchRegistry::new();
/// Forwarded writer calls.
static CALLS: AtomicU64 = AtomicU64::new(0);
/// Reused and read commanders; refused certificates; failed captures.
static READS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
/// Peak commanders reached by capture or the existing group sizing census.
static LEADERS: AtomicUsize = AtomicUsize::new(0);
/// The held bans, between the Presentation job's captures.
static STORE: Mutex<Option<Box<Store>>> = Mutex::new(None);

/// Notifications running now.
pub fn active() -> usize {
    WATCHES.active()
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    WATCHES.boundary(invalidate);
}

/// Off-thread: the existing group census bounds its distinct commanders before
/// installation, including groups whose commander is not currently demanded.
pub fn prepare(leaders: usize) {
    LEADERS.fetch_max(leaders, Ordering::Relaxed);
    presize();
}

/// Off-thread: include every reached commander, even a refused certificate.
/// The first table installs at once; later growth follows invalidation.
pub fn presize() {
    let leaders = LEADERS.load(Ordering::Relaxed);
    if leaders != 0 {
        WATCHES.offer(leaders);
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.collect();
}

/// The naval tree family's commander patches enabled or disabled; outside
/// attach and detach only. Without them nothing is observed.
pub fn forwarder(enabled: bool) {
    WATCHES.set_enabled(cfg!(feature = "retreat_ban_changes") && enabled);
}

/// Free the watch tables once the forwarding patches were removed and
/// drained after the last capture.
///
/// # Safety
/// No notification may still hold a table.
pub unsafe fn release() {
    unsafe { WATCHES.release() };
}

/// Hook path, allocation-free: a forwarded writer is about to change the
/// blocks of `leader`. Hold the result over the native call.
pub fn forwarded(leader: usize) -> Notification<'static> {
    CALLS.fetch_add(1, Ordering::Relaxed);
    WATCHES.notify(leader)
}

/// What a capture sees of the notifications.
pub trait Changes {
    /// `(epoch, sequence)` of one commander, registering it; `None` when it
    /// cannot be observed.
    fn sequence(&self, leader: usize) -> Option<(u64, u64)>;
}

pub struct Observer;
impl Changes for Observer {
    fn sequence(&self, leader: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(leader)
    }
}

/// One held read: the certificate taken before it and the bans it read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Held {
    pub certificate: (u64, u64),
    pub bans: RetreatBans,
}

/// Held reads by commander (`CUnitLeader` address).
#[derive(Default)]
pub struct Store {
    pub held: HashMap<usize, Held>,
}

/// Worker: the held bans, or an empty store when none is held.
pub fn take_store() -> Box<Store> {
    match STORE.try_lock() {
        Ok(mut slot) => slot.take().unwrap_or_default(),
        Err(_) => Box::default(),
    }
}

/// Worker: hold the bans a complete capture read or carried.
pub fn return_store(store: Box<Store>) {
    if let Ok(mut slot) = STORE.try_lock() {
        *slot = Some(store);
    }
}

/// One capture's commanders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    /// Commanders whose certificate could not be taken, read afresh.
    pub refused: u64,
    /// Commanders reached, carried or read.
    pub leaders: u64,
}

/// One capture's use of the store: which commanders it carries and which it
/// reads under which certificate.
pub struct Leaders<'a> {
    store: &'a mut Store,
    changes: &'a dyn Changes,
    /// Commanders read this capture and their certificate. Ascending once
    /// [`Self::finish`] sorts it.
    pending: Vec<(usize, (u64, u64))>,
    /// Commanders carried this capture.
    carried: Vec<usize>,
    counts: Counts,
}

impl<'a> Leaders<'a> {
    pub fn new(store: &'a mut Store, changes: &'a dyn Changes) -> Self {
        Self {
            store,
            changes,
            pending: Vec::new(),
            carried: Vec::new(),
            counts: Counts::default(),
        }
    }

    /// The held bans of `leader` when its certificate still holds; otherwise
    /// `None`, and the walk reads it.
    pub fn held(&mut self, leader: usize) -> Option<RetreatBans> {
        self.counts.leaders += 1;
        let Some(certificate) = self.changes.sequence(leader) else {
            self.counts.refused += 1;
            return None;
        };
        let held = self
            .store
            .held
            .get(&leader)
            .filter(|held| held.certificate == certificate)
            .map(|held| held.bans);
        if held.is_some() {
            if engine_allocation::reserve(&mut self.carried, 1).is_ok() {
                self.carried.push(leader);
                self.counts.reused += 1;
                return held;
            }
            return None;
        }
        if engine_allocation::reserve(&mut self.pending, 1).is_ok() {
            self.pending.push((leader, certificate));
        }
        None
    }

    /// After a complete walk: hold each fresh read under the certificate
    /// taken before it, and forget every commander the walk did not reach or
    /// could not read.
    pub fn finish(
        mut self,
        results: impl Iterator<Item = Result<Option<RetreatBans>, &'static str>>,
    ) -> Counts {
        self.pending.sort_unstable_by_key(|entry| entry.0);
        self.carried.sort_unstable();
        let mut fresh: Vec<(usize, Held)> = Vec::new();
        for bans in results.filter_map(|result| result.ok().flatten()) {
            let Ok(at) = self
                .pending
                .binary_search_by_key(&bans.leader, |entry| entry.0)
            else {
                continue;
            };
            let (leader, certificate) = self.pending[at];
            self.counts.read += 1;
            if engine_allocation::reserve(&mut fresh, 1).is_ok() {
                fresh.push((leader, Held { certificate, bans }));
            }
        }
        let carried = &self.carried;
        self.store
            .held
            .retain(|leader, _| carried.binary_search(leader).is_ok());
        if engine_allocation::reserve_map(&mut self.store.held, fresh.len()).is_ok() {
            self.store.held.extend(fresh);
        }
        self.counts
    }
}

/// A worker publishes one capture's counts.
pub fn record(counts: &Counts) {
    for (target, value) in READS
        .iter()
        .zip([counts.reused, counts.read, counts.refused])
    {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if counts.leaders != 0 {
        LEADERS.fetch_max(counts.leaders as usize, Ordering::Relaxed);
    }
}

/// A capture that did not complete: its store was forgotten.
pub fn failed() {
    READS[3].fetch_add(1, Ordering::Relaxed);
}

pub fn status() -> impl serde::Serialize {
    let reads = READS.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "retreat_ban_changes"),
        "poisoned": WATCHES.poisoned(),
        "forwarded_writes": CALLS.load(Ordering::Relaxed),
        "reused_commanders": reads[0],
        "read_commanders": reads[1],
        "refused_commanders": reads[2],
        "failed_captures": reads[3],
        "watches": WATCHES.status(),
        "scope": "cumulative since attach; each hour DivisionRetreatBans takes every division's commander from the division census's selection, and carries a commander's bans while its certificate holds (no forwarded naval-tree commander writer touched it since the held read, same epoch); every other commander's two blocks are read"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Certificates from a test registry.
    struct Registry(WatchRegistry);
    impl Changes for Registry {
        fn sequence(&self, leader: usize) -> Option<(u64, u64)> {
            self.0.sequence(leader)
        }
    }

    fn registry() -> Registry {
        let registry = WatchRegistry::new();
        registry.offer(16);
        registry.boundary(false);
        registry.set_enabled(true);
        Registry(registry)
    }

    fn bans(leader: usize, ordinary: u8) -> RetreatBans {
        RetreatBans {
            leader,
            ordinary,
            alternate: 0,
            movement: [[None; 6]; 2],
        }
    }

    /// One capture: `walk` answers each commander the store does not carry.
    fn capture(
        store: &mut Store,
        changes: &dyn Changes,
        reached: &[usize],
        walk: impl Fn(usize) -> RetreatBans,
    ) -> (Counts, Vec<RetreatBans>) {
        let mut leaders = Leaders::new(store, changes);
        let mut answers = Vec::new();
        let mut results = Vec::new();
        for &leader in reached {
            match leaders.held(leader) {
                Some(held) => answers.push(held),
                None => {
                    answers.push(walk(leader));
                    results.push(Ok(Some(walk(leader))));
                }
            }
        }
        (leaders.finish(results.into_iter()), answers)
    }

    #[test]
    fn refused_commanders_bootstrap_watch_demand_and_then_carry_exact_bans() {
        let mut store = Store::default();
        WATCHES.set_enabled(true);
        let reached = [0x100, 0x200, 0x300];
        let (counts, answers) = capture(&mut store, &Observer, &reached, |l| bans(l, 4));
        assert_eq!((counts.leaders, counts.refused), (3, 3));
        assert_eq!(answers, reached.map(|l| bans(l, 4)));
        assert!(store.held.is_empty());
        record(&counts);
        presize();
        boundary(false);
        let (counts, _) = capture(&mut store, &Observer, &reached, |l| bans(l, 4));
        assert_eq!((counts.leaders, counts.read, counts.refused), (3, 3, 0));
        let (counts, answers) = capture(&mut store, &Observer, &reached, |_| {
            panic!("quiet commanders must carry")
        });
        assert_eq!((counts.leaders, counts.reused), (3, 3));
        assert_eq!(answers, reached.map(|l| bans(l, 4)));
        drop(forwarded(0x200));
        let (counts, answers) = capture(&mut store, &Observer, &reached, |l| bans(l, 8));
        assert_eq!((counts.read, counts.reused), (1, 2));
        assert_eq!(answers, [bans(0x100, 4), bans(0x200, 8), bans(0x300, 4)]);
    }

    /// A commander no forwarded writer touched is carried; one a writer
    /// touched, or one new to the store, is read; one the walk no longer
    /// reaches is forgotten.
    #[test]
    fn untouched_commanders_are_carried_and_touched_ones_read() {
        let changes = registry();
        let mut store = Store::default();
        let (counts, _) = capture(&mut store, &changes, &[0x100, 0x200], |l| bans(l, 4));
        assert_eq!((counts.read, counts.reused), (2, 0));
        drop(changes.0.notify(0x200));
        let (counts, answers) = capture(&mut store, &changes, &[0x100, 0x200], |l| bans(l, 8));
        assert_eq!((counts.read, counts.reused), (1, 1));
        assert_eq!(answers, [bans(0x100, 4), bans(0x200, 8)]);
        capture(&mut store, &changes, &[0x200], |l| bans(l, 8));
        assert!(!store.held.contains_key(&0x100));
        // An invalidating hour certifies nothing held before it.
        changes.0.boundary(true);
        let (counts, _) = capture(&mut store, &changes, &[0x200], |l| bans(l, 8));
        assert_eq!((counts.read, counts.reused), (1, 0));
    }

    /// An unobserved commander is read every hour and never held.
    #[test]
    fn unobserved_commanders_are_read_and_never_held() {
        let changes = registry();
        let mut store = Store::default();
        capture(&mut store, &changes, &[0x100], |l| bans(l, 4));
        changes.0.poison();
        let (counts, _) = capture(&mut store, &changes, &[0x100], |l| bans(l, 8));
        assert_eq!((counts.refused, counts.read, counts.reused), (1, 0, 0));
        assert!(store.held.is_empty());
    }
}
