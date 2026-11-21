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
static CALLS: AtomicU64 = AtomicU64::new(0);
static READS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static LEADERS: AtomicUsize = AtomicUsize::new(0);
static STORE: Mutex<Option<Box<Store>>> = Mutex::new(None);

pub fn active() -> usize {
    WATCHES.active()
}

pub fn boundary(invalidate: bool) {
    WATCHES.boundary(invalidate);
}

pub fn prepare(leaders: usize) {
    LEADERS.fetch_max(leaders, Ordering::Relaxed);
    presize();
}

pub fn presize() {
    let leaders = LEADERS.load(Ordering::Relaxed);
    if leaders != 0 {
        WATCHES.offer(leaders);
    }
}

pub fn collect() {
    WATCHES.collect();
}

pub fn forwarder(enabled: bool) {
    WATCHES.set_enabled(cfg!(feature = "retreat_ban_changes") && enabled);
}

#[doc = " Free the watch tables once the forwarding patches were removed and"]
#[doc = " drained after the last capture."]
#[doc = ""]
#[doc = " # Safety"]
#[doc = " No notification may still hold a table."]
pub unsafe fn release() {
    unsafe { WATCHES.release() };
}

pub fn forwarded(leader: usize) -> Notification<'static> {
    CALLS.fetch_add(1, Ordering::Relaxed);
    WATCHES.notify(leader)
}

pub trait Changes {
    fn sequence(&self, leader: usize) -> Option<(u64, u64)>;
}

pub struct Observer;
impl Changes for Observer {
    fn sequence(&self, leader: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(leader)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Held {
    pub certificate: (u64, u64),
    pub bans: RetreatBans,
}

#[derive(Default)]
pub struct Store {
    pub held: HashMap<usize, Held>,
}

pub fn take_store() -> Box<Store> {
    match STORE.try_lock() {
        Ok(mut slot) => slot.take().unwrap_or_default(),
        Err(_) => Box::default(),
    }
}

pub fn return_store(store: Box<Store>) {
    if let Ok(mut slot) = STORE.try_lock() {
        *slot = Some(store);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub refused: u64,
    pub leaders: u64,
}

pub struct Leaders<'a> {
    store: &'a mut Store,
    changes: &'a dyn Changes,
    pending: Vec<(usize, (u64, u64))>,
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

pub fn failed() {
    READS[3].fetch_add(1, Ordering::Relaxed);
}


#[cfg(test)]
mod tests {
    use super::*;

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
        changes.0.boundary(true);
        let (counts, _) = capture(&mut store, &changes, &[0x200], |l| bans(l, 8));
        assert_eq!((counts.read, counts.reused), (1, 0));
    }

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
