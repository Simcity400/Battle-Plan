//! Exact native query demand; values are acquired fresh at the hourly boundary.
//! No predicate value, CRelation pointer or absence certificate is retained.

use crate::hook::{Hook, Registers, Tap};
use engine::{Memory, allocation, directed_war_queries as native};
use std::{
    collections::HashSet,
    ops::Range,
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

static BASE: AtomicUsize = AtomicUsize::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
pub(crate) static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static QUERIES: LazyLock<Registry> = LazyLock::new(Registry::new);

const NAMES: [&str; 8] = [
    "directed war query identity refused",
    "directed war query store overlap",
    "directed war query capacity exceeded",
    "directed war query drain storage short",
    "directed war query world changed",
    "directed war query preparation failed",
    "directed war query boundary admission refused",
    native::UNSUPPORTED_RECEIVER,
];
static REFUSALS: [AtomicU64; NAMES.len()] = [const { AtomicU64::new(0) }; NAMES.len()];

#[derive(Default)]
struct Store {
    world: u64,
    entries: HashSet<native::Demand>,
}

struct Registry {
    store: Mutex<Store>,
    spare: Mutex<Option<HashSet<native::Demand>>>,
    retired: Mutex<Option<HashSet<native::Demand>>>,
    capacity: AtomicUsize,
    required: AtomicUsize,
    lost: AtomicBool,
}

#[derive(Default)]
pub(crate) struct Drain {
    pub(crate) len: usize,
    pub(crate) required: usize,
    pub(crate) reason: Option<&'static str>,
}

impl Registry {
    fn new() -> Self {
        Self {
            store: Mutex::new(Store::default()),
            spare: Mutex::new(None),
            retired: Mutex::new(None),
            capacity: AtomicUsize::new(0),
            required: AtomicUsize::new(0),
            lost: AtomicBool::new(false),
        }
    }

    // Worker only. Replacement maps are installed only at an empty drain;
    // their old allocations are parked until this next worker preparation.
    fn prepare(&self, slots: usize) -> std::io::Result<()> {
        drop(
            self.retired
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take(),
        );
        self.required.fetch_max(slots, Ordering::Release);
        let demand = self.required.load(Ordering::Acquire);
        if demand <= self.capacity.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut entries = HashSet::new();
        allocation::reserve_set(&mut entries, demand)?;
        let mut spare = self.spare.lock().unwrap_or_else(|e| e.into_inner());
        if spare
            .as_ref()
            .is_none_or(|old| old.capacity() < entries.capacity())
        {
            self.capacity.store(entries.capacity(), Ordering::Release);
            *spare = Some(entries);
        }
        Ok(())
    }

    fn insert(&self, world: u64, demand: native::Demand) {
        let Ok(mut store) = self.store.try_lock() else {
            self.lost.store(true, Ordering::SeqCst);
            REFUSALS[1].fetch_add(1, Ordering::Relaxed);
            return;
        };
        if store.world != world {
            store.entries.clear();
            store.world = world;
        }
        if store.entries.contains(&demand) {
            return;
        }
        if store.entries.len() == store.entries.capacity() {
            self.required
                .fetch_max(store.entries.len().saturating_add(1), Ordering::Release);
            self.lost.store(true, Ordering::SeqCst);
            REFUSALS[2].fetch_add(1, Ordering::Relaxed);
            return;
        }
        store.entries.insert(demand);
    }

    fn drain(&self, world: Option<u64>, output: &mut [native::Demand]) -> Drain {
        let Ok(mut store) = self.store.try_lock() else {
            self.lost.store(true, Ordering::SeqCst);
            REFUSALS[1].fetch_add(1, Ordering::Relaxed);
            return Drain {
                reason: Some(NAMES[1]),
                ..Default::default()
            };
        };
        let required = store
            .entries
            .len()
            .max(self.required.load(Ordering::Acquire));
        let lost = self.lost.swap(false, Ordering::SeqCst);
        let mut reason = if world.is_none() || world != Some(store.world) {
            if !store.entries.is_empty() {
                REFUSALS[4].fetch_add(1, Ordering::Relaxed);
            }
            Some(NAMES[4])
        } else if lost {
            Some("directed war query interval lost")
        } else if store.entries.len() > output.len() {
            REFUSALS[3].fetch_add(1, Ordering::Relaxed);
            Some(NAMES[3])
        } else {
            None
        };
        let mut len = if reason.is_none() {
            for (target, demand) in output.iter_mut().zip(&store.entries) {
                *target = *demand;
            }
            store.entries.len()
        } else {
            0
        };
        store.entries.clear();
        // Every next interval is partial anyway: recovery certifies only its
        // successfully observed demands, never missing matrix cells.
        if self.lost.swap(false, Ordering::SeqCst) {
            len = 0;
            reason = Some("directed war query interval lost");
        }
        if let (Ok(mut spare), Ok(mut retired)) = (self.spare.try_lock(), self.retired.try_lock())
            && retired.is_none()
            && spare
                .as_ref()
                .is_some_and(|next| next.capacity() > store.entries.capacity())
        {
            *retired = Some(std::mem::replace(&mut store.entries, spare.take().unwrap()));
        }
        output[..len].sort_unstable_by_key(|demand| demand.receiver_raw);
        Drain {
            len,
            required,
            reason,
        }
    }
}

pub(crate) fn prepare(slots: usize) {
    if engine::allocation::advisory(|| QUERIES.prepare(slots)).is_err() {
        REFUSALS[5].fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn drain(output: &mut [native::Demand]) -> Drain {
    QUERIES.drain(crate::world_lifetime::token(), output)
}

pub(crate) fn capacity() -> usize {
    QUERIES.capacity.load(Ordering::Acquire)
}

pub(crate) fn demand() -> usize {
    QUERIES.required.load(Ordering::Acquire)
}

pub(crate) fn refuse_admission() {
    REFUSALS[6].fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn status() -> impl serde::Serialize {
    crate::status::status!({
        "enabled": ENABLED.load(Ordering::Acquire),
        "capacity": capacity(),
        "required": QUERIES.required.load(Ordering::Acquire),
        "refusals": crate::status::map(NAMES.into_iter().zip(REFUSALS.iter().map(|count| count.load(Ordering::Relaxed))).filter(|(_, count)| *count != 0)),
        "scope": "exact native requested cells, fresh at admitted hourly boundary; missing cells unknown; complete_active_domain false",
    })
}

unsafe extern "system-unwind" fn observe(registers: &Registers, site: usize) {
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    struct Flight;
    impl Drop for Flight {
        fn drop(&mut self) {
            ACTIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let _flight = Flight;
    if !ENABLED.load(Ordering::Acquire) {
        return;
    }
    let world = crate::world_lifetime::token();
    let Some(world) = world else {
        return;
    };
    // Before the native seven-byte MOV at1406F3B28 /1406F3A52. The
    // observer stores only authenticated demand, never +2E8 or pointed+49.
    let frame = match site {
        0 => native::Frame {
            receiver: registers.rsi,
            target: registers.rdi,
            status: registers.rax,
        },
        1 => native::Frame {
            receiver: registers.rdi,
            target: registers.rsi,
            status: registers.rbx,
        },
        _ => return,
    };
    let memory = unsafe { engine::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Acquire);
    let state = base
        .checked_add(engine::cgame_state::SINGLETON_RVA)
        .and_then(|at| engine::read::<u64>(&memory, at))
        .and_then(|state| usize::try_from(state).ok());
    let demand = state
        .ok_or("directed war query world unavailable")
        .and_then(|state| native::read_demand(&memory, base, state, frame));
    match demand {
        Ok(demand) if crate::world_lifetime::current(Some(world)) => QUERIES.insert(world, demand),
        Err(native::UNSUPPORTED_RECEIVER) => {
            REFUSALS[7].fetch_add(1, Ordering::Relaxed);
        }
        _ => {
            REFUSALS[0].fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub(crate) struct Hooks {
    taps: [Tap; 2],
}
impl Hooks {
    pub(crate) unsafe fn new(
        memory: &impl Memory,
        base: usize,
        slots: usize,
    ) -> Result<Self, &'static str> {
        prepare(slots);
        // Install the bootstrap map before any native callback exists.
        QUERIES.drain(None, &mut []);
        BASE.store(base, Ordering::Release);
        for site in native::SITES {
            if engine::read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel) {
                return Err("directed war query sentinel differs");
            }
        }
        let tap = |index: usize| unsafe {
            let site = &native::SITES[index];
            Tap::new(base + site.rva, observe, index, site.sentinel)
                .map_err(|_| "directed war query tap construction failed")
        };
        Ok(Self {
            taps: [tap(0)?, tap(1)?],
        })
    }
    pub(crate) fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
        }
        let [first, second] = &mut self.taps;
        let ready = Hook::set_enabled_batch(
            &mut [first.publication_hook(), second.publication_hook()],
            enabled,
        )
        .map_err(|_| "directed war query hook state change failed")?;
        if ready {
            ENABLED.store(enabled, Ordering::SeqCst);
        }
        Ok(ready)
    }
    pub(crate) fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.taps.iter().flat_map(Tap::executable_ranges)
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        if ENABLED.load(Ordering::SeqCst) || ACTIVE.load(Ordering::SeqCst) != 0 {
            return;
        }
        // Runtime drops hook owners only after restoration and executable
        // quiescence. No native pointer/value survives this worker cleanup.
        if let Ok(mut store) = QUERIES.store.try_lock() {
            *store = Store::default();
        }
        if let Ok(mut spare) = QUERIES.spare.try_lock() {
            drop(spare.take());
        }
        if let Ok(mut retired) = QUERIES.retired.try_lock() {
            drop(retired.take());
        }
        QUERIES.capacity.store(0, Ordering::Release);
        QUERIES.required.store(0, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn demand(index: u32) -> native::Demand {
        native::Demand {
            receiver_raw: index,
            target_slot: index + 1,
            status: index as usize + 1,
            ..Default::default()
        }
    }
    fn registry(count: usize) -> Registry {
        let result = Registry::new();
        result.prepare(count).unwrap();
        result.drain(None, &mut []);
        result
    }
    #[test]
    fn five_thousand_native_requests_deduplicate_and_sort_without_growth() {
        let registry = registry(5000);
        let capacity = registry.store.lock().unwrap().entries.capacity();
        for index in (0..5000).rev() {
            registry.insert(7, demand(index));
            registry.insert(7, demand(index));
        }
        assert_eq!(registry.store.lock().unwrap().entries.capacity(), capacity);
        let mut rows = vec![native::Demand::default(); 5000];
        let drain = registry.drain(Some(7), &mut rows);
        assert_eq!(drain.len, 5000);
        assert!(drain.reason.is_none());
        assert!(
            rows.windows(2)
                .all(|pair| pair[0].receiver_raw < pair[1].receiver_raw)
        );
        assert_eq!(registry.drain(Some(7), &mut rows).len, 0);
    }
    #[test]
    fn world_replacement_and_short_drain_never_publish_old_demand() {
        let registry = registry(4);
        registry.insert(7, demand(1));
        let mut rows = [native::Demand::default(); 4];
        assert_eq!(registry.drain(Some(8), &mut rows).len, 0);
        registry.insert(8, demand(2));
        assert_eq!(registry.drain(Some(8), &mut []).len, 0);
        assert_eq!(registry.drain(Some(8), &mut rows).len, 0);
    }
    #[test]
    fn overflow_refuses_interval_and_installs_growth_only_after_empty_drain() {
        let registry = registry(1);
        let capacity = registry.store.lock().unwrap().entries.capacity();
        for index in 0..=capacity as u32 {
            registry.insert(7, demand(index));
        }
        assert_eq!(registry.store.lock().unwrap().entries.capacity(), capacity);
        registry.prepare(0).unwrap();
        let mut rows = vec![native::Demand::default(); capacity + 1];
        assert_eq!(registry.drain(Some(7), &mut rows).len, 0);
        assert!(registry.store.lock().unwrap().entries.capacity() > capacity);
        registry.insert(7, demand(1));
        assert_eq!(registry.drain(Some(7), &mut rows).len, 1);
    }
    #[test]
    fn overlapping_access_refuses_demand_without_waiting() {
        let registry = registry(4);
        registry.insert(7, demand(1));
        let held = registry.store.lock().unwrap();
        registry.insert(7, demand(2));
        drop(held);
        let mut rows = [native::Demand::default(); 4];
        assert_eq!(registry.drain(Some(7), &mut rows).len, 0);
    }
}
