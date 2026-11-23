
pub use crate::event_feed_history::Fact;
pub mod decisions;

use crate::{
    event_feed_history::{DetachLosses, ScriptFact, Snapshot, Store, Ticket},
    hook::{Hook, Tap, TapSafety},
};
use engine_events::event_feed_sites as native;
use engine_native::Memory;
use std::{
    cell::UnsafeCell,
    mem::{ManuallyDrop, MaybeUninit, size_of},
    ops::Range,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
};
use windows_sys::Win32::System::Threading::{WaitOnAddress, WakeByAddressAll};

const RETAIN: usize = 1 << 63;
const BEFORE_FAULT: u32 = 1;
const RETURNED_FAULT: u32 = 2;
const NATIVE_ESCAPE: u32 = 4;
const OWNERSHIP_FAULT: u32 = 8;

static FLIGHT: AtomicUsize = AtomicUsize::new(0);
static FAULT: AtomicU32 = AtomicU32::new(0);
static SHARED: AtomicPtr<Owner> = AtomicPtr::new(ptr::null_mut());
static TARGETS: [AtomicUsize; 2] = [AtomicUsize::new(0), AtomicUsize::new(0)];
struct Ledger {
    owner: u64,
    cells: Vec<crate::event_feed_history::Receipt>,
}
static LEDGER: Mutex<Ledger> = Mutex::new(Ledger {
    owner: 0,
    cells: Vec::new(),
});
static OWNERS: AtomicU64 = AtomicU64::new(1);
fn ledger(owner: &Owner) -> std::sync::MutexGuard<'static, Ledger> {
    let mut ledger = LEDGER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if ledger.owner != owner.serial {
        ledger.cells.clear();
        ledger.owner = owner.serial;
    }
    ledger
}
static DEMAND: AtomicUsize = AtomicUsize::new(0);
static CAPACITY: AtomicUsize = AtomicUsize::new(0);
static WITNESS_CAPACITY: AtomicUsize = AtomicUsize::new(0);
static SCRIPT_TOKEN_DEMAND: AtomicUsize = AtomicUsize::new(0);
static SCRIPT_SCOPE_DEMAND: AtomicUsize = AtomicUsize::new(0);
static SCRIPT_GRAPH_DEMAND: AtomicUsize = AtomicUsize::new(0);
static SCRIPT_CAPACITY: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];
static SPARE: Mutex<Option<Store>> = Mutex::new(None);
static RETIRED: Mutex<Option<Store>> = Mutex::new(None);

fn script_demands(store: &Store) {
    DEMAND.fetch_max(store.capacity_demand(), Ordering::Relaxed);
    SCRIPT_TOKEN_DEMAND.fetch_max(store.token_demand, Ordering::Relaxed);
    SCRIPT_SCOPE_DEMAND.fetch_max(store.scope_demand, Ordering::Relaxed);
    SCRIPT_GRAPH_DEMAND.fetch_max(store.graph_demand, Ordering::Relaxed);
}
fn script_capacities(store: &Store) {
    CAPACITY.store(store.capacity(), Ordering::Release);
    WITNESS_CAPACITY.store(store.witness_capacity(), Ordering::Release);
    for (capacity, value) in SCRIPT_CAPACITY.iter().zip(store.script_capacity()) {
        capacity.store(value, Ordering::Release);
    }
}

#[derive(Clone, Copy)]
pub struct DelayedCensus {
    base: usize,
    state: usize,
    hour: i32,
    generation: u64,
    epoch: u64,
}

pub fn recovery_pending() -> bool {
    crate::event_feed_history::RECOVERY.load(Ordering::Acquire)
        > crate::event_feed_history::RECOVERED.load(Ordering::Acquire)
}

pub fn begin_delayed_census(
    base: usize,
    state: usize,
    hour: i32,
    full: bool,
) -> Option<DelayedCensus> {
    if full {
        crate::event_feed_history::recovery_needed();
    }
    let owner = unsafe { SHARED.load(Ordering::SeqCst).as_ref() }?;
    owner
        .worker(Role::Census, |store| {
            if retained() || !owner.armed.load(Ordering::SeqCst) {
                return None;
            }
            let generation = crate::world_lifetime::token()?;
            if owner.base != base {
                store.reject("delayed-event census world certificate unavailable");
                return None;
            }
            let epoch = store.begin_recovery(generation, state)?;
            Some(DelayedCensus {
                base,
                state,
                hour,
                generation,
                epoch,
            })
        })
        .ok()
        .flatten()
}
pub fn baseline_delayed_country(
    memory: &impl Memory,
    context: DelayedCensus,
    country: engine_native::ccountry::Country,
) {
    let Some(owner) = (unsafe { SHARED.load(Ordering::SeqCst).as_ref() }) else {
        return;
    };
    let work = owner.worker(Role::Census, |store| {
        if retained()
            || !owner.armed.load(Ordering::SeqCst)
            || owner.base != context.base
            || crate::world_lifetime::token() != Some(context.generation)
        {
            return [false; 2];
        }
        let work = store.owner_work(
            memory,
            context.base,
            context.generation,
            context.state,
            context.hour,
            country,
        );
        script_demands(store);
        script_capacities(store);
        work
    });
    let Ok(work) = work else {
        return;
    };
    if !work.into_iter().any(|work| work) {
        return;
    }
    for registry in [
        crate::event_feed_history::OwnerRegistry::Decisions,
        crate::event_feed_history::OwnerRegistry::Delayed,
    ] {
        if !work[registry as usize] {
            continue;
        }
        let mut selected = [false; 2];
        selected[registry as usize] = true;
        let prepared = crate::event_feed_history::recovery::PreparedOwner::read(
            memory,
            context.base,
            context.generation,
            context.state,
            context.hour,
            country,
            selected,
        );
        let Ok(mut prepared) = prepared else {
            let _ = owner.worker(Role::Census, |store| {
                if retained()
                    || !owner.armed.load(Ordering::SeqCst)
                    || crate::world_lifetime::token() != Some(context.generation)
                {
                    return;
                }
                store.refuse_owner_preparation(
                    memory,
                    context.base,
                    context.generation,
                    context.hour,
                    country,
                    selected,
                );
                script_demands(store);
            });
            continue;
        };
        let _ = owner.worker(Role::Census, |store| {
            store.prepare_owner_demand(&prepared);
            script_demands(store);
        });
        presize(0);
        let _ = owner.worker(Role::Census, |store| {
            if retained()
                || !owner.armed.load(Ordering::SeqCst)
                || crate::world_lifetime::token() != Some(context.generation)
            {
                return;
            }
            install_offered(store);
            if let Err(reason) = store.publish_owner(
                memory,
                context.base,
                context.generation,
                context.hour,
                &mut prepared,
            ) {
                store.reject(reason);
            }
            script_demands(store);
            script_capacities(store);
        });
    }
}
pub fn finish_delayed_census(context: DelayedCensus, complete: bool) {
    let Some(owner) = (unsafe { SHARED.load(Ordering::SeqCst).as_ref() }) else {
        return;
    };
    let _ = owner.worker(Role::Census, |store| {
        if !retained()
            && owner.armed.load(Ordering::SeqCst)
            && crate::world_lifetime::token() == Some(context.generation)
        {
            store.finish_recovery(context.generation, context.epoch, complete);
        }
    });
}
pub fn scheduled_delayed(country: usize) {
    let Some(owner) = (unsafe { SHARED.load(Ordering::SeqCst).as_ref() }) else {
        return;
    };
    let _ = owner.observe(Role::DelayedWriter, |store| {
        let Some(generation) = observed(owner, store) else {
            return;
        };
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        let result = (|| {
            let world = native::provenance::World::new(&memory, owner.base)?;
            let identity = world.country_object(country)?;
            let array = engine_events::script_event_registry::delayed_array(&memory, country)?;
            let ordinal = array
                .count
                .checked_sub(1)
                .ok_or("delayed-event native append is empty")?;
            let row = engine_events::script_event_registry::delayed_at(
                &memory, owner.base, array, ordinal,
            )?;
            store.delayed_stored(
                &world,
                &memory,
                owner.base,
                generation,
                world.hour()?,
                identity,
                row,
            )
        })();
        if let Err(reason) = result {
            store.reject(reason);
        }
        script_demands(store);
    });
}
pub fn retired_delayed(pointer: usize, pass: Option<(usize, u64)>, visited: bool) {
    let Some(owner) = (unsafe { SHARED.load(Ordering::SeqCst).as_ref() }) else {
        return;
    };
    let role = if pass.is_some() {
        Role::DelayedPass
    } else {
        Role::DelayedWriter
    };
    let _ = owner.observe(role, |store| {
        let Some(generation) = observed(owner, store) else {
            return;
        };
        if pass.is_none() && !store.has_delayed(pointer) {
            return;
        }
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        let result = native::provenance::World::new(&memory, owner.base).and_then(|world| {
            store.delayed_removed(
                &world, &memory, owner.base, generation, pointer, pass, visited,
            )
        });
        if let Err(reason) = result {
            store.reject(reason);
        }
        script_demands(store);
    });
}
pub fn reset_delayed_country(pointer: usize) {
    let Some(owner) = (unsafe { SHARED.load(Ordering::SeqCst).as_ref() }) else {
        return;
    };
    let _ = owner.observe(Role::DelayedWriter, |store| {
        store.invalidate_delayed_country(pointer);
        let Some(generation) = observed(owner, store) else {
            return;
        };
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        let result = (|| {
            let world = native::provenance::World::new(&memory, owner.base)?;
            let mut fact = crate::event_feed_history::registry::marker(
                &world,
                &memory,
                replay_data::event_feed_rows::EventFeedScriptChange::DelayedBaseline,
                None,
            )?;
            fact.target = world.country_object(pointer)?;
            store.script_known(&memory, owner.base, generation, world.hour()?, fact)
        })();
        if let Err(reason) = result {
            store.reject(reason);
        }
        script_demands(store);
    });
}

#[doc = " Snapshot offsets are duplicated only in the matching ASM/header."]
#[doc = " The ticket bytes are opaque to C++; only Rust creates/consumes the ticket."]
#[repr(C)]
struct Frame {
    rsp_before_call: usize,
    rcx: usize,
    rdx: usize,
    r8: usize,
    r9: usize,
    rdi: usize,
    rbp: usize,
    rax: usize,
    r10: usize,
    r11: usize,
    xmm: [[u8; 16]; 6],
    site: u32,
    owned_flight: u32,
    terminal: u32,
    reserved: u32,
    ticket: MaybeUninit<Ticket>,
    generation: u64,
    target: usize,
}
const _: () = {
    assert!(size_of::<usize>() == 8);
    assert!(size_of::<Ticket>() == 16);
    assert!(size_of::<Frame>() == 224);
    assert!(std::mem::offset_of!(Frame, xmm) == 80);
    assert!(std::mem::offset_of!(Frame, ticket) == 192);
    assert!(std::mem::offset_of!(Frame, target) == 216);
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Role {
    WarCall,
    Settlement,
    Provenance,
    Script,
    ScriptInstance,
    PendingRegistration,
    DelayedWriter,
    DelayedPass,
    DecisionWriter,
    PoliticalBalance,
    Refusal,
    BoundaryPrepare,
    BoundarySeal,
    Census,
    Detach,
    Staging,
}
const ROLES: usize = 16;
impl Role {
    fn held(self) -> u8 {
        self as u8 + 1
    }
    fn holder(busy: u64) -> usize {
        ((busy & HOLDER) as usize).saturating_sub(1).min(ROLES - 1)
    }
}
static DECLINED: [[AtomicU64; ROLES]; ROLES] =
    [const { [const { AtomicU64::new(0) }; ROLES] }; ROLES];
static DECLINED_FOR_WAITING_WORKER: [AtomicU64; ROLES] = [const { AtomicU64::new(0) }; ROLES];
static WORKER_WAITS: [[AtomicU64; ROLES]; ROLES] =
    [const { [const { AtomicU64::new(0) }; ROLES] }; ROLES];
static WORKER_LOST_RACES: [AtomicU64; ROLES] = [const { AtomicU64::new(0) }; ROLES];
static WORKER_REFUSALS: [AtomicU64; ROLES] = [const { AtomicU64::new(0) }; ROLES];
const WORKER_SPINS: u32 = 64;
const WORKER_RETAINED: &str = "Event feed owner retained while the worker waited";

const HOLDER: u64 = 0xff;
const WORKER_WAITING: u64 = 1 << 8;
const CANDIDATE: u64 = 1 << 20;
const CANDIDATE_WAITING: u64 = 1 << 21;

fn candidate_held(word: u64) -> bool {
    word & (HOLDER | CANDIDATE | WORKER_WAITING)
        == (u64::from(Role::DecisionWriter.held()) | CANDIDATE)
}

static WRITER_EPOCH: AtomicU64 = AtomicU64::new(0);
pub(crate) fn writer_moved() {
    WRITER_EPOCH.fetch_add(1, Ordering::SeqCst);
}
pub(crate) fn writer_cut() -> Option<u64> {
    let epoch = WRITER_EPOCH.load(Ordering::SeqCst);
    (active() & !RETAIN == 0).then_some(epoch)
}
pub(crate) fn writer_cut_holds(cut: u64) -> bool {
    active() & !RETAIN == 0 && WRITER_EPOCH.load(Ordering::SeqCst) == cut
}

struct Owner {
    base: usize,
    serial: u64,
    armed: AtomicBool,
    busy: AtomicU64,
    releases: AtomicU64,
    phase: AtomicU64,
    entering: [AtomicU64; 2],
    declines: [AtomicU64; 2],
    taken: [AtomicU64; 2],
    seal_loss_count: AtomicU64,
    seal_loss_publication: AtomicU64,
    store: UnsafeCell<Store>,
}
const NO_SEAL: u64 = u64::MAX;
unsafe impl Send for Owner {}
unsafe impl Sync for Owner {}

impl Owner {
    fn new(base: usize, store: Store) -> Arc<Self> {
        Arc::new(Self {
            base,
            serial: OWNERS.fetch_add(1, Ordering::Relaxed),
            armed: AtomicBool::new(false),
            busy: AtomicU64::new(0),
            releases: AtomicU64::new(0),
            phase: AtomicU64::new(0),
            entering: [const { AtomicU64::new(0) }; 2],
            declines: [const { AtomicU64::new(0) }; 2],
            taken: [const { AtomicU64::new(0) }; 2],
            seal_loss_count: AtomicU64::new(0),
            seal_loss_publication: AtomicU64::new(0),
            store: UnsafeCell::new(store),
        })
    }

    fn access<T>(
        &self,
        role: Role,
        apply: impl FnOnce(&mut Store) -> T,
    ) -> Result<T, &'static str> {
        self.access_mode(role, apply, true, false)
    }

    fn observe<T>(
        &self,
        role: Role,
        apply: impl FnOnce(&mut Store) -> T,
    ) -> Result<T, &'static str> {
        self.access_mode(role, apply, false, false)
    }

    fn observe_candidate<T>(&self, apply: impl FnOnce(&mut Store) -> T) -> Result<T, &'static str> {
        self.access_mode(Role::DecisionWriter, apply, false, true)
    }

    fn wait_candidate(&self, mut word: u64) {
        for _ in 0..WORKER_SPINS {
            if !candidate_held(word) {
                return;
            }
            std::hint::spin_loop();
            word = self.busy.load(Ordering::SeqCst);
        }
        if !candidate_held(word) || retained() {
            return;
        }
        let parked = word | CANDIDATE_WAITING;
        if self
            .busy
            .compare_exchange(word, parked, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        #[cfg(test)]
        BEFORE_CANDIDATE_WAIT.with(|hook| {
            if let Some(hook) = hook.take() {
                hook();
            }
        });
        unsafe {
            WaitOnAddress(
                self.busy.as_ptr().cast(),
                (&parked as *const u64).cast(),
                size_of::<u64>(),
                1,
            )
        };
    }

    fn join_phase(&self, mut phase: u64) -> u64 {
        loop {
            let slot = (phase & 1) as usize;
            self.entering[slot].fetch_add(1, Ordering::SeqCst);
            let now = self.phase.load(Ordering::SeqCst);
            if now == phase {
                return phase;
            }
            self.entering[slot].fetch_sub(1, Ordering::SeqCst);
            phase = now;
        }
    }
    #[cfg(test)]
    fn enter_phase(&self) -> usize {
        self.validate_phase(self.phase.load(Ordering::SeqCst))
    }
    #[cfg(test)]
    fn validate_phase(&self, phase: u64) -> usize {
        (self.join_phase(phase) & 1) as usize
    }
    fn leave_phase(&self, slot: usize) {
        self.entering[slot].fetch_sub(1, Ordering::SeqCst);
    }

    fn access_mode<T>(
        &self,
        role: Role,
        apply: impl FnOnce(&mut Store) -> T,
        retain_overlap: bool,
        candidate: bool,
    ) -> Result<T, &'static str> {
        let (slot, word) = loop {
            writer_moved();
            let phase = self.join_phase(self.phase.load(Ordering::SeqCst));
            let slot = (phase & 1) as usize;
            let entered = self
                .busy
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |word| {
                    (word & HOLDER == 0 && (retain_overlap || word & WORKER_WAITING == 0)).then(
                        || word | u64::from(role.held()) | if candidate { CANDIDATE } else { 0 },
                    )
                });
            let word = match entered {
                Ok(_) => {
                    self.leave_phase(slot);
                    return self.held(apply, !retain_overlap, Some(phase));
                }
                Err(word) => word,
            };
            if !candidate || !candidate_held(word) || retained() {
                break (slot, word);
            }
            self.leave_phase(slot);
            self.wait_candidate(word);
        };
        if word & HOLDER == 0 {
            DECLINED_FOR_WAITING_WORKER[role as usize].fetch_add(1, Ordering::Relaxed);
        } else {
            DECLINED[role as usize][Role::holder(word)].fetch_add(1, Ordering::Relaxed);
        }
        if retain_overlap {
            self.leave_phase(slot);
            fault(OWNERSHIP_FAULT);
            return Err("Event feed store overlap; owner retained");
        }
        let counted =
            self.declines[slot].fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                count.checked_add(1)
            });
        self.leave_phase(slot);
        if counted.is_err() {
            fault(OWNERSHIP_FAULT);
            return Err("Event feed declined observation count exhausted; owner retained");
        }
        Err(OBSERVATION_OVERLAP)
    }

    fn worker<T>(
        &self,
        role: Role,
        apply: impl FnOnce(&mut Store) -> T,
    ) -> Result<T, &'static str> {
        self.enter_worker(role)?;
        self.held(apply, true, None)
    }

    fn sealing<T>(
        &self,
        role: Role,
        apply: impl FnOnce(&mut Store) -> (T, Option<u64>),
    ) -> Result<(T, u64), &'static str> {
        self.enter_worker(role)?;
        let (value, publication, old) = self.held(
            |store| {
                let (value, publication) = apply(store);
                let old = (self.phase.fetch_add(1, Ordering::SeqCst) & 1) as usize;
                (value, publication, old)
            },
            true,
            None,
        )?;
        #[cfg(test)]
        after_seal_release();
        let mut spins = 0;
        while self.entering[old].load(Ordering::SeqCst) != 0 {
            if spins < WORKER_SPINS {
                spins += 1;
                std::hint::spin_loop();
            } else {
                std::thread::yield_now();
            }
        }
        let owned = self.take_declines(old);
        if owned != 0 {
            self.seal_loss_count.store(owned, Ordering::SeqCst);
            self.seal_loss_publication
                .store(publication.unwrap_or(NO_SEAL), Ordering::Release);
        }
        Ok((value, if publication.is_some() { owned } else { 0 }))
    }

    fn enter_worker(&self, role: Role) -> Result<(), &'static str> {
        let mut spins = 0;
        let mut found: Option<u64> = None;
        let mut owed = false;
        loop {
            let releases = self.releases.load(Ordering::SeqCst);
            let word = match self
                .busy
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |word| {
                    (word & HOLDER == 0).then(|| (word & !WORKER_WAITING) | u64::from(role.held()))
                }) {
                Ok(_) => return Ok(()),
                Err(word) => word,
            };
            #[cfg(test)]
            AFTER_FAILED_CLAIM.with(|hook| {
                if let Some(hook) = hook.take() {
                    hook();
                }
            });
            match found {
                None => {
                    WORKER_WAITS[role as usize][Role::holder(word)].fetch_add(1, Ordering::Relaxed);
                    found = Some(releases);
                }
                Some(seen) if !owed => {
                    if releases != seen {
                        owed = true;
                        self.busy.fetch_or(WORKER_WAITING, Ordering::SeqCst);
                        WORKER_LOST_RACES[role as usize].fetch_add(1, Ordering::Relaxed);
                    }
                }
                Some(_) if word & WORKER_WAITING == 0 => {
                    self.busy.fetch_or(WORKER_WAITING, Ordering::SeqCst);
                }
                Some(_) => {}
            }
            if retained() {
                if owed {
                    self.busy.fetch_and(!WORKER_WAITING, Ordering::SeqCst);
                }
                WORKER_REFUSALS[role as usize].fetch_add(1, Ordering::Relaxed);
                return Err(WORKER_RETAINED);
            }
            if spins < WORKER_SPINS {
                spins += 1;
                std::hint::spin_loop();
            } else {
                std::thread::yield_now();
            }
            #[cfg(test)]
            BEFORE_WORKER_RETRY.with(|hook| {
                if let Some(hook) = hook.take() {
                    hook();
                }
            });
        }
    }

    fn held<T>(
        &self,
        apply: impl FnOnce(&mut Store) -> T,
        take_missed: bool,
        phase: Option<u64>,
    ) -> Result<T, &'static str> {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let store = unsafe { &mut *self.store.get() };
            let publication = self.seal_loss_publication.swap(0, Ordering::Acquire);
            if publication != 0 {
                let count = self.seal_loss_count.swap(0, Ordering::SeqCst);
                if publication == NO_SEAL {
                    store.invalidate(OBSERVATION_OVERLAP);
                    store.reject_many(OBSERVATION_OVERLAP, count);
                } else {
                    store.seal_declined(publication, count);
                }
            }
            if take_missed {
                self.take_missed(store, phase);
            }
            apply(store)
        }));
        match result {
            Ok(value) => {
                let held = self
                    .busy
                    .fetch_and(!(HOLDER | CANDIDATE | CANDIDATE_WAITING), Ordering::SeqCst);
                self.releases.fetch_add(1, Ordering::SeqCst);
                if held & CANDIDATE_WAITING != 0 {
                    // SAFETY: the ownership qword stays live through every
                    // synchronous callback and any enclosing native Flight.
                    unsafe { WakeByAddressAll(self.busy.as_ptr().cast()) };
                }
                Ok(value)
            }
            Err(payload) => {
                fault(OWNERSHIP_FAULT);
                std::mem::forget(payload);
                Err("Event feed callback panic; owner retained")
            }
        }
    }

    fn take_declines(&self, slot: usize) -> u64 {
        let declines = self.declines[slot].load(Ordering::SeqCst);
        declines.saturating_sub(self.taken[slot].fetch_max(declines, Ordering::SeqCst))
    }
    fn current(&self) -> usize {
        (self.phase.load(Ordering::SeqCst) & 1) as usize
    }
    fn untaken(&self) -> u64 {
        let slot = self.current();
        self.declines[slot]
            .load(Ordering::SeqCst)
            .saturating_sub(self.taken[slot].load(Ordering::SeqCst))
    }

    fn take_missed(&self, store: &mut Store, phase: Option<u64>) -> bool {
        let current = self.phase.load(Ordering::SeqCst);
        if phase.is_some_and(|phase| phase != current) {
            return false;
        }
        let missed = self.take_declines((current & 1) as usize);
        if missed == 0 {
            return false;
        }
        store.invalidate(OBSERVATION_OVERLAP);
        store.reject_many(OBSERVATION_OVERLAP, missed);
        true
    }

    fn revoke_world(&self) {
        self.armed.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
thread_local! {
    static BEFORE_CANDIDATE_WAIT: std::cell::Cell<Option<fn()>> = const { std::cell::Cell::new(None) };
    static BEFORE_WORKER_RETRY: std::cell::Cell<Option<fn()>> = const { std::cell::Cell::new(None) };
    static AFTER_FAILED_CLAIM: std::cell::Cell<Option<fn()>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn untaken_declines() -> u64 {
    let lost = unsafe { SHARED.load(Ordering::SeqCst).as_ref() }.map_or(0, Owner::untaken);
    #[cfg(test)]
    let lost = lost + TEST_UNTAKEN_DECLINES.load(Ordering::SeqCst);
    lost
}
#[cfg(test)]
pub(crate) static TEST_UNTAKEN_DECLINES: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
static SEAL_RELEASED: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
static SEAL_RESUME: AtomicBool = AtomicBool::new(true);
#[cfg(test)]
fn after_seal_release() {
    SEAL_RELEASED.store(true, Ordering::SeqCst);
    while !SEAL_RESUME.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }
}

pub(crate) const OBSERVATION_OVERLAP: &str =
    "Event feed observation overlapped another acquisition";

fn retained() -> bool {
    FAULT.load(Ordering::SeqCst) != 0 || active() & RETAIN != 0
}

fn fault(code: u32) {
    FAULT.fetch_or(code, Ordering::SeqCst);
    FLIGHT.fetch_or(RETAIN, Ordering::SeqCst);
}

/// # Safety
/// Only the assembly entry stub calls this, through `hoi4_event_before`, on the native thread
/// entering the hooked call, with the frame it owns for that call.
unsafe fn before(frame: &mut Frame) -> Result<bool, &'static str> {
    let Some(owner) = (unsafe { SHARED.load(Ordering::SeqCst).as_ref() }) else {
        fault(BEFORE_FAULT);
        return Err("Event feed owner unavailable");
    };
    owner.observe(Role::WarCall, |store| {
        if frame.owned_flight != 1 || FAULT.load(Ordering::SeqCst) != 0 {
            return Err(store.reject("Event feed native flight unavailable"));
        }
        let generation = crate::world_lifetime::token();
        if !owner.armed.load(Ordering::SeqCst) || generation.is_none() {
            store.invalidate("Event feed world observation unavailable");
            return Err(store.reject("Event feed world observation unavailable"));
        }
        let generation = generation.unwrap();
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        let ticket = match frame.site {
            0 => store.war_before(
                &memory,
                owner.base,
                generation,
                native::WarFrame {
                    rsp_before_call: frame.rsp_before_call,
                    rcx: frame.rcx,
                    rdx: frame.rdx,
                    rdi: frame.rdi,
                },
                crate::war_provenance::inside(),
            ),
            1 => store.annex_before(
                &memory,
                owner.base,
                generation,
                native::AnnexFrame {
                    rbp: frame.rbp,
                    r8: frame.r8,
                    r9: frame.r9 as u64,
                },
                crate::war_provenance::inside(),
            ),
            _ => Err(store.reject("Event feed CALL owner differs")),
        };
        DEMAND.fetch_max(store.capacity_demand(), Ordering::Relaxed);
        let ticket = ticket?;
        frame.ticket.write(ticket);
        frame.generation = generation;
        frame.terminal = 1;
        Ok(true)
    })?
}

pub fn record(
    observe: impl FnOnce(
        &native::provenance::World<'_, engine_native::memory::LiveMemory>,
        &engine_native::memory::LiveMemory,
        usize,
    ) -> Result<Option<Fact>, &'static str>,
) {
    let Some(owner) = (unsafe { SHARED.load(Ordering::SeqCst).as_ref() }) else {
        return;
    };
    let _ = owner.observe(Role::Provenance, |store| {
        let Some(generation) = observed(owner, store) else {
            return;
        };
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        let fact = native::provenance::World::new(&memory, owner.base).and_then(|world| {
            let hour = world.hour()?;
            Ok(observe(&world, &memory, owner.base)?.map(|fact| (hour, fact)))
        });
        match fact {
            Ok(Some((hour, fact))) => {
                let _ = store.known(generation, hour, fact);
            }
            Ok(None) => {}
            Err(reason) => {
                store.reject(reason);
            }
        }
        DEMAND.fetch_max(store.capacity_demand(), Ordering::Relaxed);
    });
}

pub fn record_script(
    observe: impl FnOnce(
        &native::provenance::World<'_, engine_native::memory::LiveMemory>,
        &engine_native::memory::LiveMemory,
        usize,
    ) -> Result<Option<ScriptFact>, &'static str>,
) {
    let Some(owner) = (unsafe { SHARED.load(Ordering::SeqCst).as_ref() }) else {
        return;
    };
    let _ = owner.observe(Role::Script, |store| {
        let Some(generation) = observed(owner, store) else {
            return;
        };
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        let fact = native::provenance::World::new(&memory, owner.base).and_then(|world| {
            let hour = world.hour()?;
            Ok(observe(&world, &memory, owner.base)?.map(|fact| (hour, fact)))
        });
        match fact {
            Ok(Some((hour, fact))) => {
                let _ = store.script_known(&memory, owner.base, generation, hour, fact);
            }
            Ok(None) => {}
            Err(reason) => {
                store.reject(reason);
            }
        }
        DEMAND.fetch_max(store.capacity_demand(), Ordering::Relaxed);
        SCRIPT_TOKEN_DEMAND.fetch_max(store.token_demand, Ordering::Relaxed);
        SCRIPT_SCOPE_DEMAND.fetch_max(store.scope_demand, Ordering::Relaxed);
        SCRIPT_GRAPH_DEMAND.fetch_max(store.graph_demand, Ordering::Relaxed);
    });
}

pub fn script_instance(frame: usize, instance: u32) {
    if let Some(owner) = unsafe { SHARED.load(Ordering::SeqCst).as_ref() } {
        let _ = owner.observe(Role::ScriptInstance, |store| {
            if let Some(generation) = observed(owner, store) {
                store.script_instance(generation, frame, instance);
            }
        });
    }
}

pub fn registered_script_instance(state: usize, event: usize, scope: usize, instance: u32) {
    if let Some(owner) = unsafe { SHARED.load(Ordering::SeqCst).as_ref() } {
        let _ = owner.observe(Role::PendingRegistration, |store| {
            let Some(generation) = observed(owner, store) else {
                return;
            };
            store.registered_instance(generation, scope, event, instance);
            let memory = unsafe { engine_native::memory::LiveMemory::new() };
            let result = (|| {
                let world = native::provenance::World::new(&memory, owner.base)?;
                if world.state_pointer() != state {
                    return Err("pending event registration world differs");
                }
                let row =
                    engine_events::script_event_registry::pending_last(&memory, state, instance)?;
                if row.definition != event {
                    return Err("pending event registered definition differs");
                }
                let fact =
                    crate::event_feed_history::registry::pending(&world, &memory, owner.base, row)?;
                store.script_known(&memory, owner.base, generation, world.hour()?, fact)
            })();
            if let Err(reason) = result {
                store.reject(reason);
            }
            DEMAND.fetch_max(store.capacity_demand(), Ordering::Relaxed);
            SCRIPT_TOKEN_DEMAND.fetch_max(store.token_demand, Ordering::Relaxed);
            SCRIPT_SCOPE_DEMAND.fetch_max(store.scope_demand, Ordering::Relaxed);
            SCRIPT_GRAPH_DEMAND.fetch_max(store.graph_demand, Ordering::Relaxed);
        });
    }
}

pub struct WriterFlight(crate::script_event_observer::Flight);
impl WriterFlight {
    pub fn enter() -> Self {
        Self(crate::script_event_observer::Flight::enter())
    }
    pub fn returned(&mut self) {
        self.0.returned();
    }
}

pub fn refuse(reason: &'static str) {
    if let Some(owner) = unsafe { SHARED.load(Ordering::SeqCst).as_ref() } {
        let _ = owner.observe(Role::Refusal, |store| store.reject(reason));
    }
}

pub(crate) fn stage_witness(token: u64, witness: crate::event_feed_history::Witness) {
    if let Some(owner) = unsafe { SHARED.load(Ordering::SeqCst).as_ref() } {
        let _ = owner.observe(Role::Staging, |store| {
            if let Err(reason) = store.stage_witness(token, witness) {
                store.reject(reason);
            }
            script_demands(store);
        });
    }
}
pub(crate) fn staging_epoch() -> Option<u64> {
    let owner = unsafe { SHARED.load(Ordering::SeqCst).as_ref() }?;
    owner
        .observe(Role::Staging, |store| store.staged_epoch())
        .ok()
}
pub(crate) fn take_staged(
    token: u64,
    epoch: Option<u64>,
) -> Result<Option<crate::event_feed_history::Witness>, &'static str> {
    let Some(owner) = (unsafe { SHARED.load(Ordering::SeqCst).as_ref() }) else {
        return Ok(None);
    };
    owner
        .observe(Role::Staging, |store| store.take_staged(token, epoch))
        .unwrap_or(Ok(None))
}

fn observed(owner: &Owner, store: &mut Store) -> Option<u64> {
    if FAULT.load(Ordering::SeqCst) != 0 {
        store.reject("Event feed native flight unavailable");
        return None;
    }
    match crate::world_lifetime::token() {
        Some(generation) if owner.armed.load(Ordering::SeqCst) => Some(generation),
        _ => {
            store.invalidate("Event feed world observation unavailable");
            store.reject("Event feed world observation unavailable");
            None
        }
    }
}

/// # Safety
/// C++ invokes only on this exact original's normal return. It never repeats
/// this callback after Err/panic/SEH, and never guesses an incomplete settlement.
unsafe fn returned(frame: &mut Frame) -> Result<(), &'static str> {
    let Some(owner) = (unsafe { SHARED.load(Ordering::SeqCst).as_ref() }) else {
        fault(RETURNED_FAULT);
        return Err("Event feed terminal owner unavailable");
    };
    if frame.terminal != 1 {
        fault(RETURNED_FAULT);
        return Err("Event feed terminal ownership differs");
    }
    frame.terminal = 2;
    let ticket = unsafe { frame.ticket.assume_init_read() };
    let result = owner.access(Role::Settlement, |store| {
        if FAULT.load(Ordering::SeqCst) != 0 {
            store.uncertain(&ticket, "Event feed world or native lifetime invalidated");
            return Err(crate::event_feed_history::Retained {
                ticket,
                reason: "Event feed world or native lifetime invalidated",
            });
        }
        if !crate::world_lifetime::current(Some(frame.generation)) {
            return store.world_returned(ticket);
        }
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        unsafe { store.normal_returned(ticket, frame.generation, &memory, owner.base) }
    })?;
    match result {
        Ok(()) => {
            frame.terminal = 3;
            Ok(())
        }
        Err(retained) => {
            frame.ticket.write(retained.ticket);
            frame.terminal = 4;
            fault(RETURNED_FAULT);
            Err(retained.reason)
        }
    }
}

#[unsafe(no_mangle)]
unsafe extern "system-unwind" fn hoi4_event_before(frame: *mut Frame) -> u32 {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeed);
    u32::from(unsafe { before(&mut *frame) }.unwrap_or(false))
}
#[unsafe(no_mangle)]
unsafe extern "system-unwind" fn hoi4_event_returned(frame: *mut Frame) {
    let _timing = crate::hook_timing::enter_return(crate::hook_timing::Family::EventFeed);
    let _ = unsafe { returned(&mut *frame) };
}
#[unsafe(no_mangle)]
extern "system" fn hoi4_event_observer_fault() {
    fault(BEFORE_FAULT | RETURNED_FAULT);
}
#[unsafe(no_mangle)]
extern "system" fn hoi4_event_native_escape() {
    fault(NATIVE_ESCAPE);
}

core::arch::global_asm!(
    include_str!("event_feed_entry.asm"),
    flight = sym FLIGHT,
    targets = sym TARGETS,
    guard = sym hoi4_event_native_guard,
);
unsafe extern "system-unwind" {
    fn hoi4_event_native_guard(frame: *mut Frame);
    fn event_war_entry();
    fn event_annex_entry();
    static event_stub_start: u8;
    static event_stub_end: u8;
}

struct Held {
    hooks: Vec<Hook>,
    taps: Vec<Tap>,
    owner: Arc<Owner>,
    restored: bool,
}
pub struct Hooks {
    held: Option<ManuallyDrop<Held>>,
}

impl Hooks {
    /// Build verified, unpublished CALL hooks using the shared patch owner.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(
        memory: &impl Memory,
        base: usize,
        last_sequence: Option<u64>,
        counts: replay_data::event_feed_rows::EventFeedCounts,
        slots: usize,
        enabled: bool,
    ) -> Result<Self, String> {
        if !enabled {
            return Ok(Self { held: None });
        }
        let mut store = Store::new(last_sequence)?;
        store.restore_counts(counts);
        let capacity = slots.saturating_mul(2).max(1024);
        store
            .prepare_capacity(capacity)
            .map_err(|_| "Event feed staging allocation failed")?;
        store
            .prepare_scripts(capacity.saturating_mul(32), capacity, slots.max(1))
            .map_err(|_| "Event feed script staging allocation failed")?;
        store
            .prepare_delayed(capacity)
            .map_err(|_| "Event feed delayed-event certificate allocation failed")?;
        store
            .prepare_decisions(capacity)
            .map_err(|_| "Event feed decision certificate allocation failed")?;
        script_capacities(&store);
        let owner = Owner::new(base, store);
        let range = unsafe { &event_stub_start } as *const u8 as usize
            ..unsafe { &event_stub_end } as *const u8 as usize;
        let make = |site: native::CallSite, callback| {
            let code = engine_sites::hook(base, site.rva)?;
            unsafe {
                Hook::new_call_with_safety(
                    code.address,
                    callback,
                    code.sentinel::<16>()?,
                    TapSafety {
                        in_flight: &FLIGHT,
                        stub: range.clone(),
                    },
                )
            }
            .map_err(|_| "Event feed CALL construction failed")
        };
        native::war_source_sites(base)?;
        let targets = [
            engine_sites::hook(base, native::WAR.target_rva)?.address,
            engine_sites::hook(base, native::ANNEX.target_rva)?.address,
        ];
        let mut hooks = vec![
            make(native::WAR, event_war_entry as *const () as usize)?,
            make(native::ANNEX, event_annex_entry as *const () as usize)?,
        ];
        hooks.extend(unsafe { crate::war_provenance::hooks(memory, base) }?);
        let (script_hooks, taps) = unsafe { crate::script_event_observer::hooks(memory, base) }?;
        hooks.extend(script_hooks);
        if FLIGHT.load(Ordering::SeqCst) != 0
            || FAULT.load(Ordering::SeqCst) != 0
            || !SHARED.load(Ordering::SeqCst).is_null()
        {
            return Err("Event feed previous owner retained".into());
        }
        TARGETS[0].store(targets[0], Ordering::SeqCst);
        TARGETS[1].store(targets[1], Ordering::SeqCst);
        SHARED.store(Arc::as_ptr(&owner).cast_mut(), Ordering::SeqCst);
        Ok(Self {
            held: Some(ManuallyDrop::new(Held {
                hooks,
                taps,
                owner,
                restored: true,
            })),
        })
    }

    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        let Some(held) = self.held.as_mut() else {
            return Ok(true);
        };
        if !enabled && active() != 0 {
            return Ok(false);
        }
        if !enabled {
            held.owner.revoke_world();
        }
        let Held { hooks, taps, .. } = &mut **held;
        let ready = Hook::set_enabled_iter(
            hooks
                .iter_mut()
                .chain(taps.iter_mut().map(Tap::publication_hook)),
            enabled,
        )
        .map_err(|cause| {
            let label = hooks
                .iter_mut()
                .chain(taps.iter_mut().map(Tap::publication_hook))
                .find_map(|hook| hook.snapshot_failure())
                .unwrap_or("Event feed / action, registry, decision store and pass family");
            crate::script_event_observer::terminal_hook_failure(label, "publication", cause)
        })?;
        if ready {
            held.restored = !enabled;
            held.owner.armed.store(enabled, Ordering::SeqCst);
        }
        Ok(ready)
    }

    pub fn executable_ranges(&self) -> Vec<Range<usize>> {
        let Some(held) = self.held.as_ref() else {
            return Vec::new();
        };
        held.hooks
            .iter()
            .flat_map(Hook::executable_ranges)
            .chain(held.taps.iter().flat_map(Tap::executable_ranges))
            .chain(std::iter::once(
                unsafe { &event_stub_start } as *const u8 as usize
                    ..unsafe { &event_stub_end } as *const u8 as usize,
            ))
            .collect()
    }

    pub fn release_after_quiet(&mut self) -> Result<DetachLosses, &'static str> {
        let Some(held) = self.held.as_ref() else {
            return Ok(DetachLosses::default());
        };
        let losses = held.detach_losses(active())?;
        *SPARE
            .lock()
            .map_err(|_| "Event feed spare storage retained")? = None;
        *RETIRED
            .lock()
            .map_err(|_| "Event feed retired storage retained")? = None;
        SHARED.store(ptr::null_mut(), Ordering::SeqCst);
        TARGETS[0].store(0, Ordering::SeqCst);
        TARGETS[1].store(0, Ordering::SeqCst);
        let mut held = self.held.take().unwrap();
        drop(unsafe { ManuallyDrop::take(&mut held) });
        Ok(losses)
    }
}

impl Held {
    fn detach_losses(&self, native_flights: usize) -> Result<DetachLosses, &'static str> {
        if !self.restored
            || self.hooks.iter().any(Hook::is_enabled)
            || native_flights != 0
            || self.owner.busy.load(Ordering::SeqCst) & HOLDER != 0
        {
            return Err("Event feed owner has unsettled native ownership");
        }
        self.owner.access(Role::Detach, |store| {
            take_receipts(&self.owner, store);
            store.detach_losses()
        })?
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        let _ = self.release_after_quiet();
    }
}

fn take_receipts(owner: &Owner, store: &mut Store) {
    let mut ledger = ledger(owner);
    store.receipts(&ledger.cells);
    ledger.cells.retain(|(_, outcome)| outcome.is_none());
}

pub fn active() -> usize {
    FLIGHT
        .load(Ordering::SeqCst)
        .saturating_add(crate::war_provenance::active())
        .saturating_add(crate::script_event_observer::active())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forwarder {
    Politics,
    Territory,
    DelayedPass,
}

static FORWARDERS: [AtomicBool; 3] = [const { AtomicBool::new(false) }; 3];

pub fn forwarder(forwarder: Forwarder, enabled: bool) {
    if matches!(forwarder, Forwarder::Politics) && !enabled {
        decisions::political_balance_lost();
    }
    FORWARDERS[forwarder as usize].store(enabled, Ordering::SeqCst);
}

fn coverage(partial: bool) -> replay_data::event_feed_rows::EventFeedCoverage {
    use replay_data::event_feed_rows::EventFeedCoverage;
    if partial
        || !FORWARDERS
            .iter()
            .all(|enabled| enabled.load(Ordering::SeqCst))
    {
        EventFeedCoverage::Partial
    } else {
        EventFeedCoverage::Complete
    }
}

static WARS: AtomicUsize = AtomicUsize::new(0);

pub fn note_wars(count: usize) {
    WARS.store(count, Ordering::Relaxed);
}

/// # Safety
/// Caller must own the active session's capture, recovery, or publication failure path (or an
/// isolated test fixture) when revoking live event-feed world observation.
pub unsafe fn lost_drain() {
    if let Some(owner) = unsafe { SHARED.load(Ordering::SeqCst).as_ref() } {
        owner.revoke_world();
    }
}

fn input_failure(owner: &Owner) -> Option<&'static str> {
    if retained() {
        Some("Event feed native or callback ownership retained")
    } else if !owner.armed.load(Ordering::SeqCst) {
        Some("Event feed publication lost; interval unavailable")
    } else {
        crate::world_lifetime::unavailable_reason()
    }
}

/// # Safety
/// Caller must run on the admitted hourly observer boundary as the sole consumer preparing the
/// event-feed interval with teardown excluded; a stable absent owner (hooks disabled) is permitted
/// and returns unavailable.
pub unsafe fn prepare_growing(hour: i32, partial: bool) -> Result<(), &'static str> {
    let generation = crate::world_lifetime::generation();
    let owner =
        unsafe { SHARED.load(Ordering::SeqCst).as_ref() }.ok_or("Event feed hooks unavailable")?;
    owner
        .worker(Role::BoundaryPrepare, |store| {
            take_receipts(owner, store);
            install_offered(store);
            let failure = input_failure(owner);
            let boundary = crate::event_feed_history::OwnerBoundary {
                generation,
                read_hour: hour,
                coverage: coverage(partial),
                failure,
            };
            let result = if failure.is_none() {
                let memory = unsafe { engine_native::memory::LiveMemory::new() };
                store.prepare_boundary(boundary, Some((&memory, owner.base)))
            } else {
                store.prepare_boundary::<engine_native::memory::LiveMemory>(boundary, None)
            };
            if let Err(reason) = result {
                store.invalidate(reason);
            }
            script_capacities(store);
            script_demands(store);
            result
        })
        .and_then(|result| result)
}

fn install_offered(store: &mut Store) {
    if active() == 0
        && let (Ok(mut spare), Ok(mut retired)) = (SPARE.try_lock(), RETIRED.try_lock())
        && retired.is_none()
        && let Some(mut offered) = spare.take()
    {
        store.install_capacity(&mut offered);
        script_capacities(store);
        *retired = Some(offered);
    }
}

#[cfg(test)]
pub(crate) fn presize_owner_for_test(
    store: &mut Store,
    plan: &crate::event_feed_history::recovery::PreparedOwner,
) {
    store.prepare_owner_demand(plan);
    script_capacities(store);
    script_demands(store);
    presize(0);
    crate::test_support::with_allocations_denied(|| install_offered(store));
}

/// # Safety
/// Caller must hold hourly capture ownership as the sole consumer sealing the prepared event-feed
/// interval while event-feed storage stays live and teardown is excluded.
pub unsafe fn seal(output: &mut Snapshot, hour: i32, partial: bool) {
    seal_hour(output, hour, partial, || {});
}

fn seal_hour(output: &mut Snapshot, hour: i32, partial: bool, during: impl FnOnce()) {
    let generation = crate::world_lifetime::generation();
    let Some(owner) = (unsafe { SHARED.load(Ordering::SeqCst).as_ref() }) else {
        output.refuse(generation, hour, "Event feed hooks unavailable");
        return;
    };
    let room = match owner.worker(Role::BoundarySeal, |store| {
        (store.seal_demand(), store.receipt_room())
    }) {
        Ok((demand, room)) => {
            if let Ok(demand) = demand {
                let _ = output.reserve_with_headroom(demand);
            }
            room
        }
        Err(reason) => {
            owner.revoke_world();
            output.refuse(generation, hour, reason);
            return;
        }
    };
    let mut spare = room
        .and_then(|room| engine_allocation::vec(room).ok())
        .unwrap_or_default();
    let cell = engine_allocation::reserve(&mut ledger(owner).cells, 1).is_ok();
    let sealed = owner.sealing(Role::BoundarySeal, |store| {
        store.install_receipts(&mut spare);
        let boundary = crate::event_feed_history::OwnerBoundary {
            generation,
            read_hour: hour,
            coverage: coverage(partial),
            failure: input_failure(owner),
        };
        let result = if cell {
            store.seal(output, boundary)
        } else {
            Err(crate::event_feed_history::RECEIPT_STORAGE)
        };
        if let Err(reason) = result {
            store.invalidate(reason);
        }
        during();
        script_capacities(store);
        script_demands(store);
        owner.armed.store(true, Ordering::SeqCst);
        let publication = result.is_ok().then(|| output.publication());
        (result, publication)
    });
    drop(spare);
    match sealed {
        Ok((Ok(()), declined)) => {
            let mut ledger = ledger(owner);
            if ledger.cells.len() < ledger.cells.capacity() {
                ledger.cells.push((output.publication(), None));
            } else {
                fault(OWNERSHIP_FAULT);
            }
            drop(ledger);
            if declined != 0 {
                output.declined_during_seal(declined);
            }
        }
        Ok((Err(reason), _)) => output.refuse(generation, hour, reason),
        Err(reason) => {
            owner.revoke_world();
            output.refuse(generation, hour, reason);
        }
    }
}

/// # Safety
/// Caller must be the originating spool resolving a confirmed publication receipt for this session.
pub unsafe fn accepted(pack: &crate::event_feed_history::Prepared) -> std::io::Result<()> {
    resolve(pack.publication(), true);
    Ok(())
}

/// # Safety
/// Caller must be the originating spool or capture recovery path discarding an uncommitted
/// publication receipt for this session.
pub unsafe fn discarded(publication: u64) {
    resolve(publication, false);
}

fn resolve(publication: u64, accepted: bool) {
    if publication == 0 {
        return;
    }
    let mut ledger = LEDGER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Ok(index) = ledger
        .cells
        .binary_search_by_key(&publication, |cell| cell.0)
    {
        ledger.cells[index].1.get_or_insert(accepted);
    }
}

pub fn presize(slots: usize) {
    engine_allocation::advisory(|| presize_storage(slots));
}

fn staging_capacity(current: usize, required: usize) -> usize {
    let required = current.max(required);
    required.checked_next_power_of_two().unwrap_or(required)
}

fn presize_storage(slots: usize) {
    if active() & RETAIN != 0 {
        return;
    }
    if let Ok(mut retired) = RETIRED.lock() {
        drop(retired.take());
    }
    let capacity = staging_capacity(
        CAPACITY.load(Ordering::Acquire),
        slots
            .saturating_mul(2)
            .saturating_add(crate::territory_changes::state_slots())
            .saturating_add(WARS.load(Ordering::Relaxed))
            .max(1024)
            .max(DEMAND.load(Ordering::Relaxed)),
    );
    let script_demand = [
        capacity
            .saturating_mul(32)
            .max(SCRIPT_CAPACITY[0].load(Ordering::Acquire))
            .max(SCRIPT_TOKEN_DEMAND.load(Ordering::Relaxed)),
        capacity
            .max(SCRIPT_CAPACITY[1].load(Ordering::Acquire))
            .max(SCRIPT_SCOPE_DEMAND.load(Ordering::Relaxed)),
        slots
            .max(1)
            .max(SCRIPT_CAPACITY[2].load(Ordering::Acquire))
            .max(SCRIPT_GRAPH_DEMAND.load(Ordering::Relaxed)),
    ];
    let witness_demand = DEMAND.load(Ordering::Relaxed);
    if capacity <= CAPACITY.load(Ordering::Acquire)
        && witness_demand <= WITNESS_CAPACITY.load(Ordering::Acquire)
        && script_demand
            .iter()
            .zip(&SCRIPT_CAPACITY)
            .all(|(demand, capacity)| *demand <= capacity.load(Ordering::Acquire))
    {
        return;
    }
    if let Ok(mut spare) = SPARE.lock() {
        if spare.as_ref().is_some_and(|store| {
            store.capacity() >= capacity
                && store.witness_capacity() >= witness_demand
                && store
                    .script_capacity()
                    .into_iter()
                    .zip(script_demand)
                    .all(|(have, need)| have >= need)
        }) {
            return;
        }
        if let Ok(mut store) = Store::new(None)
            && store.prepare_capacity(capacity).is_ok()
            && store
                .prepare_scripts(script_demand[0], script_demand[1], script_demand[2])
                .is_ok()
            && store.prepare_delayed(capacity).is_ok()
            && store.prepare_decisions(capacity).is_ok()
        {
            *spare = Some(store);
        }
    }
}

#[doc = " Test only: the event feed's process-wide state for one test at a time,"]
#[doc = " from its initial values. The owner, target, fault and flight are restored"]
#[doc = " on drop; a failed test never poisons the lock."]
#[cfg(test)]
pub(crate) struct TestGlobals {
    shared: *mut Owner,
    target: usize,
    faults: u32,
    flights: usize,
    _guard: std::sync::MutexGuard<'static, ()>,
    _slots: std::sync::MutexGuard<'static, ()>,
}
#[cfg(test)]
pub(crate) fn test_globals() -> TestGlobals {
    use crate::event_feed_history as history;
    let guard = crate::hook::FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let globals = TestGlobals {
        shared: SHARED.swap(ptr::null_mut(), Ordering::SeqCst),
        target: TARGETS[0].swap(0, Ordering::SeqCst),
        faults: FAULT.swap(0, Ordering::SeqCst),
        flights: FLIGHT.swap(0, Ordering::SeqCst),
        _guard: guard,
        _slots: collector_primitives::country_slots::hint_guard(),
    };
    for value in [
        &DEMAND,
        &CAPACITY,
        &WITNESS_CAPACITY,
        &SCRIPT_TOKEN_DEMAND,
        &SCRIPT_SCOPE_DEMAND,
        &SCRIPT_GRAPH_DEMAND,
    ]
    .into_iter()
    .chain(&SCRIPT_CAPACITY)
    {
        value.store(0, Ordering::SeqCst);
    }
    for store in [&SPARE, &RETIRED] {
        *store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
    TEST_UNTAKEN_DECLINES.store(0, Ordering::SeqCst);
    history::RECOVERY.store(1, Ordering::SeqCst);
    history::RECOVERED.store(0, Ordering::SeqCst);
    for count in history::REPEATED_OWNER_REFUSALS
        .iter()
        .chain(&history::CROSSED_SAMPLES)
    {
        count.store(0, Ordering::SeqCst);
    }
    globals
}
#[cfg(test)]
impl TestGlobals {
    fn restore(&self) {
        SHARED.store(self.shared, Ordering::SeqCst);
        TARGETS[0].store(self.target, Ordering::SeqCst);
        FAULT.store(self.faults, Ordering::SeqCst);
        FLIGHT.store(self.flights, Ordering::SeqCst);
    }
}
#[cfg(test)]
impl Drop for TestGlobals {
    fn drop(&mut self) {
        self.restore();
    }
}

#[cfg(test)]
pub(crate) struct StagingTest {
    _owner: Arc<Owner>,
    globals: TestGlobals,
}
#[cfg(test)]
pub(crate) fn staging_test(capacity: usize) -> StagingTest {
    let globals = test_globals();
    let test = StagingTest {
        _owner: {
            let mut store = Store::new(None).unwrap();
            store.prepare_decisions(capacity).unwrap();
            Owner::new(0, store)
        },
        globals,
    };
    test._owner.armed.store(true, Ordering::SeqCst);
    SHARED.store(Arc::as_ptr(&test._owner).cast_mut(), Ordering::SeqCst);
    test
}
#[cfg(test)]
impl StagingTest {
    pub(crate) fn staged_live(&self) -> usize {
        unsafe { (*self._owner.store.get()).staged_live() }
    }
    pub(crate) fn prepare(&self, hour: i32) {
        self._owner
            .worker(Role::BoundaryPrepare, |store| {
                store.prepare_boundary::<engine_native::memory::LiveMemory>(
                    crate::event_feed_history::OwnerBoundary {
                        generation: 1,
                        read_hour: hour,
                        coverage: replay_data::event_feed_rows::EventFeedCoverage::Complete,
                        failure: None,
                    },
                    None,
                )
            })
            .unwrap()
            .unwrap();
    }
    pub(crate) fn failure(&self) -> (Option<&'static str>, u64) {
        let store = unsafe { &*self._owner.store.get() };
        (store.failure_for_test(), store.rejected_for_test())
    }
}
#[cfg(test)]
impl Drop for StagingTest {
    fn drop(&mut self) {
        self.globals.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        CONTEXT, RtlLookupFunctionEntry, RtlVirtualUnwind,
    };

    #[test]
    fn uneven_staging_and_rounded_indexes_converge_without_new_demand() {
        let mut store = Store::new(None).unwrap();
        store.prepare_capacity(1030).unwrap();
        store.prepare_delayed(1030).unwrap();
        store.prepare_decisions(1030).unwrap();
        let capacity = staging_capacity(store.capacity(), store.capacity_demand());
        assert_eq!(capacity, 2048);
        let mut spare = Store::new(None).unwrap();
        spare.prepare_capacity(capacity).unwrap();
        spare.prepare_delayed(capacity).unwrap();
        spare.prepare_decisions(capacity).unwrap();
        store.install_capacity(&mut spare);
        for _ in 0..16 {
            assert_eq!(
                staging_capacity(store.capacity(), store.capacity_demand()),
                capacity
            );
        }
        assert_eq!(staging_capacity(capacity, capacity + 1), 4096);
    }

    #[test]
    fn excluded_history_constructs_no_hooks_or_storage() {
        struct NoReads;
        impl Memory for NoReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("excluded history must not read game memory")
            }
        }
        let mut hooks =
            unsafe { Hooks::new(&NoReads, 0, None, Default::default(), 1500, false) }.unwrap();
        assert!(hooks.held.is_none());
        assert!(hooks.executable_ranges().is_empty());
        assert_eq!(hooks.set_enabled(true), Ok(true));
        assert_eq!(hooks.set_enabled(false), Ok(true));
    }

    fn untaken(owner: &Owner) -> u64 {
        (0..2)
            .map(|slot| {
                owner.declines[slot].load(Ordering::SeqCst)
                    - owner.taken[slot].load(Ordering::SeqCst)
            })
            .sum()
    }
    fn total_declines(owner: &Owner) -> u64 {
        owner
            .declines
            .iter()
            .map(|count| count.load(Ordering::SeqCst))
            .sum()
    }

    #[test]
    fn declined_call_observation_acquires_no_ticket_and_original_releases_its_flight() {
        static ORIGINAL_CALLS: AtomicUsize = AtomicUsize::new(0);
        extern "system" fn original() {
            ORIGINAL_CALLS.fetch_add(1, Ordering::SeqCst);
        }
        let _globals = test_globals();
        assert_eq!(FAULT.load(Ordering::SeqCst), 0);
        assert_eq!(FLIGHT.load(Ordering::SeqCst), 0);
        assert!(SHARED.load(Ordering::SeqCst).is_null());
        let owner = Owner::new(0, Store::new(None).unwrap());
        owner.armed.store(true, Ordering::SeqCst);
        SHARED.store(Arc::as_ptr(&owner).cast_mut(), Ordering::SeqCst);
        TARGETS[0].store(original as *const () as usize, Ordering::SeqCst);
        let before = ORIGINAL_CALLS.load(Ordering::SeqCst);
        let declined = &DECLINED[Role::WarCall as usize][Role::DelayedPass as usize];
        let declined_before = declined.load(Ordering::SeqCst);
        owner
            .observe(Role::DelayedPass, |_| unsafe { event_war_entry() })
            .unwrap();
        assert_eq!(ORIGINAL_CALLS.load(Ordering::SeqCst), before + 1);
        assert_eq!(FLIGHT.load(Ordering::SeqCst), 0);
        assert_eq!(FAULT.load(Ordering::SeqCst), 0);
        assert_eq!(untaken(&owner), 1);
        assert_eq!(declined.load(Ordering::SeqCst), declined_before + 1);
        assert!(owner.armed.load(Ordering::SeqCst));
        let mut snapshot = Snapshot::new(0);
        owner
            .observe(Role::BoundarySeal, |store| {
                assert_eq!(store.outstanding(), 0);
                store.snapshot(
                    &mut snapshot,
                    crate::event_feed_history::OwnerBoundary {
                        generation: 7,
                        read_hour: 1,
                        coverage: replay_data::event_feed_rows::EventFeedCoverage::Complete,
                        failure: None,
                    },
                )
            })
            .unwrap()
            .unwrap();
        let row = snapshot.prepare_pack().unwrap();
        assert_eq!(row.row().counts.unwrap().rejected, 1);
        assert_eq!(row.row().counts.unwrap().acquired, 0);
        assert!(row.row().events.is_empty());
        assert_eq!(
            row.row().failure.as_deref(),
            Some(OBSERVATION_OVERLAP.as_bytes())
        );
        assert_eq!(
            row.row().coverage,
            replay_data::event_feed_rows::EventFeedCoverage::Unavailable
        );
    }

    #[test]
    fn observation_gap_does_not_clear_concurrent_terminal_ownership_retention() {
        let _globals = test_globals();
        assert_eq!(FAULT.load(Ordering::SeqCst), 0);
        assert_eq!(FLIGHT.load(Ordering::SeqCst), 0);
        let owner = Owner::new(0, Store::new(None).unwrap());
        owner
            .observe(Role::Script, |_| {
                assert_eq!(
                    owner.access(Role::Settlement, |_| panic!(
                        "overlap must never enter settlement"
                    )),
                    Err::<(), _>("Event feed store overlap; owner retained")
                );
                assert_eq!(
                    owner.observe(Role::ScriptInstance, |_| panic!(
                        "overlap must acquire no ticket"
                    )),
                    Err::<(), _>(OBSERVATION_OVERLAP)
                );
            })
            .unwrap();
        owner.observe(Role::Script, |_| ()).unwrap();
        assert_eq!(untaken(&owner), 0);
        assert_eq!(FAULT.load(Ordering::SeqCst), OWNERSHIP_FAULT);
        assert_eq!(FLIGHT.load(Ordering::SeqCst), RETAIN);
    }

    fn total(counters: &[AtomicU64; ROLES]) -> u64 {
        counters
            .iter()
            .map(|count| count.load(Ordering::SeqCst))
            .sum()
    }

    fn append_candidate(store: &mut Store, ordinal: u32) {
        use replay_data::event_feed_rows::{EventFeedScriptChange, decisions::*};
        struct NoReads;
        impl Memory for NoReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("Candidate admission must not defer native reads")
            }
        }
        let mut fact = crate::event_feed_history::registry::marker_for(
            &NoReads,
            EventFeedScriptChange::Decision(DecisionChange {
                ordinal,
                ..DecisionChange::new(Operation::Candidate, 1)
            }),
            None,
        );
        let mut header = [0; 32];
        header[..9].copy_from_slice(b"candidate");
        header[16..24].copy_from_slice(&9_u64.to_le_bytes());
        header[24..].copy_from_slice(&15_u64.to_le_bytes());
        fact.token =
            engine_native::msvc_string::Descriptor::from_header(&NoReads, 0, header).unwrap();
        store
            .script_known(&NoReads, 0, crate::world_lifetime::generation(), 2, fact)
            .unwrap();
    }

    fn concurrent_candidate_publications(seal_between: bool) {
        use replay_data::event_feed_rows::{
            EventFeedCoverage, EventFeedScriptChange, decisions::Operation,
        };
        static REGISTERED: Barrier = Barrier::new(2);
        static RESUME: Barrier = Barrier::new(2);
        fn before_wait() {
            REGISTERED.wait();
            RESUME.wait();
        }
        let _globals = test_globals();
        let owner = sealing_owner();
        owner
            .worker(Role::Census, |store| store.prepare_scripts(64, 0, 0))
            .unwrap()
            .unwrap();
        let mut output = Snapshot::new(0);
        assert_eq!(
            publish_hour(&owner, &mut output, 1, &|| {}).row().coverage,
            EventFeedCoverage::Partial
        );
        let held = Barrier::new(2);
        let release = Barrier::new(2);
        let declined = DECLINED[Role::DecisionWriter as usize][Role::DecisionWriter as usize]
            .load(Ordering::SeqCst);
        let earlier = std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                owner.observe_candidate(|store| {
                    crate::test_support::with_allocations_denied(|| append_candidate(store, 1));
                    held.wait();
                    release.wait();
                })
            });
            held.wait();
            let second = scope.spawn(|| {
                BEFORE_CANDIDATE_WAIT.with(|hook| hook.set(Some(before_wait)));
                crate::test_support::with_allocations_denied(|| {
                    owner.observe_candidate(|store| append_candidate(store, 2))
                })
            });
            REGISTERED.wait();
            assert_ne!(owner.busy.load(Ordering::SeqCst) & CANDIDATE_WAITING, 0);
            assert!(owner.entering.iter().all(|n| n.load(Ordering::SeqCst) == 0));
            assert_eq!(total_declines(&owner), 0);
            release.wait();
            assert_eq!(first.join().unwrap(), Ok(()));
            let earlier = seal_between.then(|| publish_hour(&owner, &mut output, 2, &|| {}));
            let cut = writer_cut().unwrap();
            RESUME.wait();
            assert_eq!(second.join().unwrap(), Ok(()));
            assert!(!writer_cut_holds(cut));
            earlier
        });
        let last = publish_hour(
            &owner,
            &mut output,
            if seal_between { 3 } else { 2 },
            &|| {},
        );
        let events = earlier
            .iter()
            .flat_map(|pack| &pack.row().events)
            .chain(&last.row().events)
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 2);
        assert!(events[0].sequence < events[1].sequence);
        for (event, ordinal) in events.into_iter().zip([1, 2]) {
            let Some(EventFeedScriptChange::Decision(change)) = event.script.unwrap().change else {
                panic!("Candidate occurrence missing")
            };
            assert_eq!(change.operation, Operation::Candidate);
            assert_eq!(change.ordinal, ordinal);
        }
        if let Some(earlier) = earlier {
            assert_eq!(earlier.row().events.len(), 1);
            assert_eq!(earlier.row().coverage, EventFeedCoverage::Complete);
        }
        assert_eq!(last.row().coverage, EventFeedCoverage::Complete);
        assert_eq!(last.row().counts.unwrap().acquired, 2);
        assert_eq!(last.row().counts.unwrap().rejected, 0);
        assert_eq!(total_declines(&owner), 0);
        assert_eq!(owner.busy.load(Ordering::SeqCst), 0);
        assert_eq!(
            DECLINED[Role::DecisionWriter as usize][Role::DecisionWriter as usize]
                .load(Ordering::SeqCst),
            declined
        );
        assert_eq!(FAULT.load(Ordering::SeqCst), 0);
        forwarders_off();
    }

    #[test]
    fn concurrent_candidates_keep_both_occurrences_in_lease_order_without_allocation() {
        concurrent_candidate_publications(false);
    }

    #[test]
    fn a_parked_candidate_does_not_strand_the_seals_phase_drain() {
        concurrent_candidate_publications(true);
    }

    #[test]
    fn a_candidate_still_declines_immediately_while_a_worker_holds_the_store() {
        let _globals = test_globals();
        let owner = Owner::new(0, Store::new(None).unwrap());
        let declined = &DECLINED[Role::DecisionWriter as usize][Role::Census as usize];
        let before = declined.load(Ordering::SeqCst);
        owner
            .worker(Role::Census, |_| {
                crate::test_support::with_allocations_denied(|| {
                    assert_eq!(
                        owner.observe_candidate(|_| panic!("worker-held Candidate entered")),
                        Err::<(), _>(OBSERVATION_OVERLAP)
                    );
                });
                assert_eq!(
                    owner.busy.load(Ordering::SeqCst),
                    u64::from(Role::Census.held())
                );
            })
            .unwrap();
        assert_eq!(declined.load(Ordering::SeqCst), before + 1);
        assert_eq!(total_declines(&owner), 1);
        assert_eq!(FAULT.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn candidate_waiting_does_not_extend_to_other_decision_sites_or_an_owed_worker() {
        let _globals = test_globals();
        let owner = Owner::new(0, Store::new(None).unwrap());
        owner
            .observe(Role::DecisionWriter, |_| {
                assert_eq!(owner.observe_candidate(|_| ()), Err(OBSERVATION_OVERLAP));
            })
            .unwrap();
        owner
            .observe_candidate(|_| {
                assert_eq!(
                    owner.observe(Role::DecisionWriter, |_| ()),
                    Err(OBSERVATION_OVERLAP)
                );
                owner.busy.fetch_or(WORKER_WAITING, Ordering::SeqCst);
                assert_eq!(owner.observe_candidate(|_| ()), Err(OBSERVATION_OVERLAP));
            })
            .unwrap();
        assert_eq!(owner.busy.load(Ordering::SeqCst), WORKER_WAITING);
        assert_eq!(total_declines(&owner), 3);
        assert_eq!(owner.worker(Role::Census, |_| ()), Ok(()));
        assert_eq!(owner.busy.load(Ordering::SeqCst), 0);
        assert_eq!(FAULT.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn terminal_retention_ends_a_candidate_wait_without_releasing_the_owner() {
        static REGISTERED: Barrier = Barrier::new(2);
        fn before_wait() {
            REGISTERED.wait();
        }
        let _globals = test_globals();
        let owner = Owner::new(0, Store::new(None).unwrap());
        let holder = u64::from(Role::DecisionWriter.held()) | CANDIDATE;
        owner.busy.store(holder, Ordering::SeqCst);
        std::thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                BEFORE_CANDIDATE_WAIT.with(|hook| hook.set(Some(before_wait)));
                owner.observe_candidate(|_| panic!("a retained Candidate store was entered"))
            });
            REGISTERED.wait();
            fault(OWNERSHIP_FAULT);
            assert_eq!(waiter.join().unwrap(), Err::<(), _>(OBSERVATION_OVERLAP));
        });
        assert_eq!(
            owner.busy.load(Ordering::SeqCst) & (HOLDER | CANDIDATE),
            holder
        );
        assert_eq!(owner.releases.load(Ordering::SeqCst), 0);
        assert_eq!(total_declines(&owner), 1);
        assert!(retained());
    }

    #[test]
    fn witness_pressure_replaces_an_entries_large_witnesses_small_spare() {
        use crate::event_feed_history::{WITNESS_ALLOCATION, Witness};
        let _globals = test_globals();
        let prepared = || {
            let mut store = Store::new(None).unwrap();
            store.prepare_capacity(8192).unwrap();
            store.prepare_scripts(8192 * 32, 8192, 1).unwrap();
            store.prepare_delayed(4096).unwrap();
            store.prepare_decisions(4096).unwrap();
            store
        };
        let mut store = prepared();
        *SPARE.lock().unwrap() = Some(prepared());
        DEMAND.store(0, Ordering::SeqCst);
        script_capacities(&store);
        for n in 0..4096 {
            crate::test_support::with_allocations_denied(|| {
                store
                    .stage_witness(1, Witness::Selected { instance: n })
                    .unwrap();
            });
        }
        script_demands(&store);
        presize(0);
        assert_eq!(
            SPARE.lock().unwrap().as_ref().unwrap().witness_capacity(),
            4096
        );
        crate::test_support::with_allocations_denied(|| {
            assert_eq!(
                store.stage_witness(1, Witness::Selected { instance: 4096 }),
                Err(WITNESS_ALLOCATION)
            );
        });
        assert_eq!(store.capacity_demand(), 4097);
        script_demands(&store);
        presize(0);
        assert!(SPARE.lock().unwrap().as_ref().unwrap().witness_capacity() >= 4097);
        crate::test_support::with_allocations_denied(|| {
            install_offered(&mut store);
            store
                .stage_witness(1, Witness::Selected { instance: 4096 })
                .unwrap();
        });
        assert!(store.witness_capacity() >= 4097);
        assert_eq!(store.staged_live(), 4097);
    }

    #[test]
    fn a_worker_that_wins_the_first_race_waits_for_a_live_holder_without_any_decline() {
        static SECOND_CLAIM_FAILED: Barrier = Barrier::new(2);
        static RELEASED: AtomicBool = AtomicBool::new(false);
        fn release_after_the_claim() {
            SECOND_CLAIM_FAILED.wait();
            while !RELEASED.load(Ordering::SeqCst) {
                std::thread::yield_now();
            }
        }
        fn on_the_second_failure() {
            AFTER_FAILED_CLAIM.with(|hook| hook.set(Some(release_after_the_claim)));
        }
        let _globals = test_globals();
        RELEASED.store(false, Ordering::SeqCst);
        assert_eq!(FAULT.load(Ordering::SeqCst), 0);
        assert_eq!(FLIGHT.load(Ordering::SeqCst), 0);
        let owner = Owner::new(0, Store::new(None).unwrap());
        owner.armed.store(true, Ordering::SeqCst);
        let lost = WORKER_LOST_RACES[Role::Census as usize].load(Ordering::SeqCst);
        let for_waiting = total(&DECLINED_FOR_WAITING_WORKER);
        let waits = &WORKER_WAITS[Role::Census as usize][Role::DelayedPass as usize];
        let waited = waits.load(Ordering::SeqCst);
        let declined = DECLINED[Role::Census as usize]
            .iter()
            .map(|count| count.load(Ordering::SeqCst))
            .sum::<u64>();
        let entered = Arc::new(AtomicBool::new(false));
        let holder = {
            let owner = Arc::clone(&owner);
            let entered = Arc::clone(&entered);
            std::thread::spawn(move || {
                owner
                    .observe(Role::DelayedPass, |_| {
                        entered.store(true, Ordering::SeqCst);
                        SECOND_CLAIM_FAILED.wait();
                    })
                    .unwrap();
                RELEASED.store(true, Ordering::SeqCst);
            })
        };
        while !entered.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        AFTER_FAILED_CLAIM.with(|hook| hook.set(Some(on_the_second_failure)));
        assert_eq!(owner.worker(Role::Census, |_| 7), Ok(7));
        holder.join().unwrap();
        assert_eq!(waits.load(Ordering::SeqCst), waited + 1);
        assert_eq!(untaken(&owner), 0);
        assert_eq!(
            DECLINED[Role::Census as usize]
                .iter()
                .map(|count| count.load(Ordering::SeqCst))
                .sum::<u64>(),
            declined
        );
        assert_eq!(
            WORKER_LOST_RACES[Role::Census as usize].load(Ordering::SeqCst),
            lost
        );
        assert_eq!(total(&DECLINED_FOR_WAITING_WORKER), for_waiting);
        assert_eq!(owner.busy.load(Ordering::SeqCst) & WORKER_WAITING, 0);
        assert!(owner.armed.load(Ordering::SeqCst));
    }

    #[test]
    fn a_worker_that_lost_a_race_is_owed_the_store_and_callbacks_cannot_starve_it() {
        static RETRY: Barrier = Barrier::new(2);
        static RESUME: Barrier = Barrier::new(2);
        fn pause_retry() {
            RETRY.wait();
            RESUME.wait();
        }
        fn pause_first_retry() {
            BEFORE_WORKER_RETRY.with(|hook| hook.set(Some(pause_retry)));
            pause_retry();
        }
        let _globals = test_globals();
        assert_eq!(FAULT.load(Ordering::SeqCst), 0);
        assert_eq!(FLIGHT.load(Ordering::SeqCst), 0);
        let owner = Owner::new(0, Store::new(None).unwrap());
        owner.armed.store(true, Ordering::SeqCst);
        let lost = WORKER_LOST_RACES[Role::Census as usize].load(Ordering::SeqCst);
        let waits = &WORKER_WAITS[Role::Census as usize][Role::Script as usize];
        let waited = waits.load(Ordering::SeqCst);
        let held_by_worker = &DECLINED[Role::Script as usize][Role::Census as usize];
        let held_before = held_by_worker.load(Ordering::SeqCst);
        let waiting = &DECLINED_FOR_WAITING_WORKER[Role::Script as usize];
        let for_waiting = waiting.load(Ordering::SeqCst);
        owner
            .busy
            .store(u64::from(Role::Script.held()), Ordering::SeqCst);
        let entered = Barrier::new(2);
        let leave = Barrier::new(2);
        let (first_waits, owed, lost_after, free, held, worker) = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                BEFORE_WORKER_RETRY.with(|hook| hook.set(Some(pause_first_retry)));
                owner.worker(Role::Census, |_| {
                    entered.wait();
                    leave.wait();
                })
            });
            RETRY.wait();
            let first_waits = waits.load(Ordering::SeqCst);
            owner
                .busy
                .store(u64::from(Role::Provenance.held()), Ordering::SeqCst);
            owner.releases.fetch_add(1, Ordering::SeqCst);
            RESUME.wait();
            RETRY.wait();
            let owed = owner.busy.load(Ordering::SeqCst);
            let lost_after = WORKER_LOST_RACES[Role::Census as usize].load(Ordering::SeqCst);
            owner.busy.fetch_and(!HOLDER, Ordering::SeqCst);
            owner.releases.fetch_add(1, Ordering::SeqCst);
            let free = owner.observe(Role::Script, |_| ());
            RESUME.wait();
            entered.wait();
            let held = owner.observe(Role::Script, |_| ());
            leave.wait();
            (first_waits, owed, lost_after, free, held, worker.join())
        });
        assert_eq!(first_waits, waited + 1);
        assert_eq!(owed, u64::from(Role::Provenance.held()) | WORKER_WAITING);
        assert_eq!(lost_after, lost + 1);
        assert_eq!(
            free,
            Err(OBSERVATION_OVERLAP),
            "an owed store is never taken"
        );
        assert_eq!(held, Err(OBSERVATION_OVERLAP));
        assert_eq!(worker.unwrap(), Ok(()));
        assert_eq!(owner.busy.load(Ordering::SeqCst) & WORKER_WAITING, 0);
        assert_eq!(waiting.load(Ordering::SeqCst) - for_waiting, 1);
        assert_eq!(held_by_worker.load(Ordering::SeqCst) - held_before, 1);
        assert_eq!(total_declines(&owner), 2);
    }

    #[test]
    fn a_callback_declines_for_an_owed_worker_and_counts_it_apart_from_holders() {
        let _globals = test_globals();
        let owner = Owner::new(0, Store::new(None).unwrap());
        owner.armed.store(true, Ordering::SeqCst);
        let waiting = &DECLINED_FOR_WAITING_WORKER[Role::Script as usize];
        let for_waiting = waiting.load(Ordering::SeqCst);
        let held = total(&DECLINED[Role::Script as usize]);
        owner.busy.store(WORKER_WAITING, Ordering::SeqCst);
        assert_eq!(
            owner.observe(Role::Script, |_| panic!("an owed store is never taken")),
            Err::<(), _>(OBSERVATION_OVERLAP)
        );
        assert_eq!(waiting.load(Ordering::SeqCst), for_waiting + 1);
        assert_eq!(total(&DECLINED[Role::Script as usize]), held);
        assert_eq!(total_declines(&owner), 1);
        assert_eq!(owner.access(Role::Settlement, |_| 3), Ok(3));
        assert_eq!(FAULT.load(Ordering::SeqCst), 0);
        owner.busy.fetch_and(!WORKER_WAITING, Ordering::SeqCst);
        assert_eq!(untaken(&owner), 1);
    }

    #[test]
    fn a_declined_callback_entry_attempt_breaks_a_baseline_cut() {
        let _globals = test_globals();
        assert_eq!(FLIGHT.load(Ordering::SeqCst), 0);
        let owner = Owner::new(0, Store::new(None).unwrap());
        owner.armed.store(true, Ordering::SeqCst);
        owner
            .worker(Role::Census, |_| {
                let cut = writer_cut().unwrap();
                assert!(writer_cut_holds(cut));
                std::thread::scope(|scope| {
                    scope
                        .spawn(|| assert!(owner.observe(Role::DelayedWriter, |_| ()).is_err()))
                        .join()
                        .unwrap();
                });
                assert!(!writer_cut_holds(cut));
                let cut = writer_cut().unwrap();
                let mut flight = crate::script_event_observer::Flight::enter();
                assert!(writer_cut().is_none());
                assert!(!writer_cut_holds(cut));
                flight.returned();
                drop(flight);
                assert!(!writer_cut_holds(cut));
            })
            .unwrap();
    }

    fn sealing_owner() -> Arc<Owner> {
        crate::world_lifetime::observe_for_test(true);
        forwarder(Forwarder::Politics, true);
        forwarder(Forwarder::Territory, true);
        forwarder(Forwarder::DelayedPass, true);
        let mut store = Store::new(None).unwrap();
        store.prepare_capacity(64).unwrap();
        let owner = Owner::new(0, store);
        owner.armed.store(true, Ordering::SeqCst);
        SHARED.store(Arc::as_ptr(&owner).cast_mut(), Ordering::SeqCst);
        owner
    }
    fn forwarders_off() {
        forwarder(Forwarder::Politics, false);
        forwarder(Forwarder::Territory, false);
        forwarder(Forwarder::DelayedPass, false);
    }
    fn publish_hour(
        owner: &Owner,
        output: &mut Snapshot,
        hour: i32,
        during: &dyn Fn(),
    ) -> crate::event_feed_history::Prepared {
        let pack = seal_unwritten(owner, output, hour, during);
        // SAFETY: Unit test holds `test_globals()` and resolves its own test receipt.
        unsafe { accepted(&pack) }.unwrap();
        pack
    }
    fn seal_unwritten(
        owner: &Owner,
        output: &mut Snapshot,
        hour: i32,
        during: &dyn Fn(),
    ) -> crate::event_feed_history::Prepared {
        owner
            .worker(Role::BoundaryPrepare, |store| {
                take_receipts(owner, store);
                store.prepare_boundary::<engine_native::memory::LiveMemory>(
                    crate::event_feed_history::OwnerBoundary {
                        generation: crate::world_lifetime::generation(),
                        read_hour: hour,
                        coverage: coverage(false),
                        failure: None,
                    },
                    None,
                )
            })
            .unwrap()
            .unwrap();
        seal_hour(output, hour, false, during);
        output.prepare_pack().unwrap()
    }
    #[doc = " Test only: the store while nothing holds it."]
    fn store(owner: &Owner) -> &Store {
        assert_eq!(owner.busy.load(Ordering::SeqCst) & HOLDER, 0);
        unsafe { &*owner.store.get() }
    }

    #[test]
    fn a_write_behind_backlog_seals_every_hour_and_counts_each_receipt_once() {
        use replay_data::event_feed_rows::EventFeedCoverage;
        let _globals = test_globals();
        let owner = sealing_owner();
        let mut output = Snapshot::new(0);
        let packs: Vec<_> = (1..=64)
            .map(|hour| seal_unwritten(&owner, &mut output, hour, &|| {}))
            .collect();
        assert!(packs.iter().all(|pack| pack.row().failure.is_none()));
        assert_eq!(ledger(&owner).cells.len(), 64);
        // SAFETY: Unit test holds `test_globals()` and resolves each receipt once in isolation.
        unsafe {
            discarded(packs[9].publication());
            for (index, pack) in packs.iter().enumerate() {
                if index != 9 {
                    accepted(pack).unwrap();
                }
            }
        }
        resolve(packs[9].publication(), true);
        resolve(packs[20].publication(), false);
        resolve(packs[63].publication() + 1000, false);
        assert!(ledger(&owner).cells.iter().all(|cell| cell.1.is_some()));
        let next = seal_unwritten(&owner, &mut output, 65, &|| {});
        assert_eq!(next.row().coverage, EventFeedCoverage::Unavailable);
        assert_eq!(ledger(&owner).cells.len(), 1);
        // SAFETY: Unit test holds `test_globals()` and resolves its own test receipt.
        unsafe { accepted(&next) }.unwrap();
        let losses = owner
            .access(Role::Detach, |store| {
                take_receipts(&owner, store);
                store.detach_losses()
            })
            .unwrap()
            .unwrap();
        assert_eq!(losses.failed_batches, 1);
        assert!(ledger(&owner).cells.is_empty());
        assert_eq!(store(&owner).unacknowledged(), 0);
        forwarders_off();
    }

    #[test]
    fn a_decline_during_the_seals_hold_refuses_only_the_sealed_row() {
        use replay_data::event_feed_rows::EventFeedCoverage;
        let _globals = test_globals();
        assert_eq!(FAULT.load(Ordering::SeqCst), 0);
        assert_eq!(FLIGHT.load(Ordering::SeqCst), 0);
        let owner = sealing_owner();
        let mut output = Snapshot::new(0);
        let first = publish_hour(&owner, &mut output, 1, &|| {});
        assert_eq!(first.row().coverage, EventFeedCoverage::Partial);
        let declined = publish_hour(&owner, &mut output, 2, &|| {
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        assert_eq!(
                            owner.observe(Role::Script, |_| ()),
                            Err(OBSERVATION_OVERLAP)
                        );
                    })
                    .join()
                    .unwrap();
            });
        });
        assert_eq!(declined.row().coverage, EventFeedCoverage::Unavailable);
        assert_eq!(
            declined.row().failure.as_deref(),
            Some(OBSERVATION_OVERLAP.as_bytes())
        );
        assert_eq!(declined.row().counts.unwrap().rejected, 1);
        assert_eq!(untaken(&owner), 0);
        assert_eq!(store(&owner).failure_for_test(), None);
        assert_eq!(
            owner
                .observe(Role::Script, |store| (
                    store.failure_for_test(),
                    store.rejected_for_test()
                ))
                .unwrap(),
            (Some(OBSERVATION_OVERLAP), 1)
        );
        let next = publish_hour(&owner, &mut output, 3, &|| {});
        assert_eq!(next.row().coverage, EventFeedCoverage::Partial);
        assert!(next.row().failure.is_none());
        assert_eq!(next.row().counts.unwrap().rejected, 1);
        assert_eq!(untaken(&owner), 0);
        forwarders_off();
    }

    #[test]
    fn declines_spammed_while_the_seal_releases_cannot_hold_it_and_each_is_taken_once() {
        use replay_data::event_feed_rows::EventFeedCoverage;
        let _globals = test_globals();
        assert_eq!(FAULT.load(Ordering::SeqCst), 0);
        assert_eq!(FLIGHT.load(Ordering::SeqCst), 0);
        let owner = sealing_owner();
        let mut output = Snapshot::new(0);
        publish_hour(&owner, &mut output, 1, &|| {});
        let rejected = store(&owner).rejected_for_test();
        let before = total_declines(&owner);
        let stop = AtomicBool::new(false);
        let (sealed, spammed) = std::thread::scope(|scope| {
            let spammer = scope.spawn(|| {
                let mut declined = 0_u64;
                while !stop.load(Ordering::SeqCst) {
                    declined += u64::from(owner.observe(Role::Script, |_| ()).is_err());
                }
                declined
            });
            let at = total_declines(&owner);
            let sealed = publish_hour(&owner, &mut output, 2, &|| {
                while total_declines(&owner) < at + 8 {
                    std::thread::yield_now();
                }
            });
            stop.store(true, Ordering::SeqCst);
            (sealed, spammer.join().unwrap())
        });
        assert_eq!(sealed.row().coverage, EventFeedCoverage::Unavailable);
        owner.observe(Role::Script, |_| ()).unwrap();
        let total = total_declines(&owner) - before;
        assert_eq!(spammed, total);
        assert_eq!(untaken(&owner), 0);
        assert_eq!(store(&owner).rejected_for_test() - rejected, total);
        forwarders_off();
    }

    #[test]
    fn a_worker_enters_while_declines_are_being_counted() {
        let _globals = test_globals();
        assert_eq!(FAULT.load(Ordering::SeqCst), 0);
        let owner = Owner::new(0, Store::new(None).unwrap());
        owner.armed.store(true, Ordering::SeqCst);
        let waits = &WORKER_WAITS[Role::Census as usize][Role::DelayedPass as usize];
        let waited = waits.load(Ordering::SeqCst);
        let release = AtomicBool::new(false);
        let stop = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let holder = scope.spawn(|| {
                owner
                    .observe(Role::DelayedPass, |_| {
                        while !release.load(Ordering::SeqCst) {
                            std::thread::yield_now();
                        }
                    })
                    .unwrap();
            });
            while owner.busy.load(Ordering::SeqCst) & HOLDER == 0 {
                std::thread::yield_now();
            }
            let spammer = scope.spawn(|| {
                while !stop.load(Ordering::SeqCst) {
                    let _ = owner.observe(Role::Script, |_| ());
                }
            });
            while total_declines(&owner) < 16 {
                std::thread::yield_now();
            }
            let worker = scope.spawn(|| owner.worker(Role::Census, |_| 7));
            while waits.load(Ordering::SeqCst) == waited {
                std::thread::yield_now();
            }
            release.store(true, Ordering::SeqCst);
            assert_eq!(worker.join().unwrap(), Ok(7));
            stop.store(true, Ordering::SeqCst);
            spammer.join().unwrap();
            holder.join().unwrap();
        });
    }

    #[test]
    fn a_decline_recorded_after_the_seals_release_still_counts_in_the_sealed_row() {
        use replay_data::event_feed_rows::EventFeedCoverage;
        let _globals = test_globals();
        assert_eq!(FAULT.load(Ordering::SeqCst), 0);
        let owner = sealing_owner();
        let mut output = Snapshot::new(0);
        publish_hour(&owner, &mut output, 1, &|| {});
        let phase = owner.phase.load(Ordering::SeqCst);
        let slot = (phase & 1) as usize;
        let sealed = std::thread::scope(|scope| {
            let paused = scope.spawn(|| {
                while owner.phase.load(Ordering::SeqCst) == phase {
                    std::thread::yield_now();
                }
                owner.declines[slot].fetch_add(1, Ordering::SeqCst);
                owner.leave_phase(slot);
            });
            let sealed = publish_hour(&owner, &mut output, 2, &|| {
                assert_eq!(owner.enter_phase(), slot);
            });
            paused.join().unwrap();
            sealed
        });
        assert_eq!(sealed.row().coverage, EventFeedCoverage::Unavailable);
        assert_eq!(
            sealed.row().failure.as_deref(),
            Some(OBSERVATION_OVERLAP.as_bytes())
        );
        assert_eq!(sealed.row().counts.unwrap().rejected, 1);
        let next = publish_hour(&owner, &mut output, 3, &|| {});
        assert_eq!(next.row().coverage, EventFeedCoverage::Partial);
        assert_eq!(next.row().counts.unwrap().rejected, 1);
        assert_eq!(untaken(&owner), 0);
        forwarders_off();
    }

    #[test]
    fn a_callback_admitted_after_the_seals_release_takes_nothing_from_the_closing_phase() {
        use replay_data::event_feed_rows::EventFeedCoverage;
        let _globals = test_globals();
        let owner = sealing_owner();
        let mut output = Snapshot::new(0);
        publish_hour(&owner, &mut output, 1, &|| {});
        let phase = owner.phase.load(Ordering::SeqCst);
        let old = (phase & 1) as usize;
        SEAL_RELEASED.store(false, Ordering::SeqCst);
        SEAL_RESUME.store(false, Ordering::SeqCst);
        let (sealed, seen) = std::thread::scope(|scope| {
            let late = scope.spawn(|| {
                while !SEAL_RELEASED.load(Ordering::SeqCst) {
                    std::thread::yield_now();
                }
                owner
                    .busy
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |word| {
                        (word & HOLDER == 0).then(|| word | u64::from(Role::Script.held()))
                    })
                    .unwrap();
                let seen = owner
                    .held(
                        |store| (store.failure_for_test(), store.rejected_for_test()),
                        true,
                        Some(phase),
                    )
                    .unwrap();
                owner.leave_phase(old);
                SEAL_RESUME.store(true, Ordering::SeqCst);
                seen
            });
            let sealed = publish_hour(&owner, &mut output, 2, &|| {
                assert_eq!(owner.enter_phase(), old);
                owner.declines[old].fetch_add(1, Ordering::SeqCst);
                owner.leave_phase(old);
                assert_eq!(owner.enter_phase(), old);
            });
            (sealed, late.join().unwrap())
        });
        assert_eq!(seen, (None, 0));
        assert_eq!(sealed.row().coverage, EventFeedCoverage::Unavailable);
        assert_eq!(
            sealed.row().failure.as_deref(),
            Some(OBSERVATION_OVERLAP.as_bytes())
        );
        assert_eq!(sealed.row().counts.unwrap().rejected, 1);
        assert_eq!(untaken(&owner), 0);
        forwarders_off();
    }

    #[test]
    fn a_callback_declined_after_the_flip_counts_in_the_next_row() {
        use replay_data::event_feed_rows::EventFeedCoverage;
        let _globals = test_globals();
        let owner = sealing_owner();
        let mut output = Snapshot::new(0);
        publish_hour(&owner, &mut output, 1, &|| {});
        let sealed = publish_hour(&owner, &mut output, 2, &|| {});
        assert_eq!(sealed.row().coverage, EventFeedCoverage::Complete);
        owner
            .worker(Role::Census, |_| {
                std::thread::scope(|scope| {
                    scope
                        .spawn(|| assert!(owner.observe(Role::Script, |_| ()).is_err()))
                        .join()
                        .unwrap();
                });
            })
            .unwrap();
        let next = publish_hour(&owner, &mut output, 3, &|| {});
        assert_eq!(next.row().coverage, EventFeedCoverage::Unavailable);
        assert_eq!(next.row().counts.unwrap().rejected, 1);
        forwarders_off();
    }

    #[test]
    fn a_callback_paused_before_joining_a_settled_phase_revalidates_onto_the_next() {
        use replay_data::event_feed_rows::EventFeedCoverage;
        let _globals = test_globals();
        let owner = sealing_owner();
        let mut output = Snapshot::new(0);
        publish_hour(&owner, &mut output, 1, &|| {});
        let stale = owner.phase.load(Ordering::SeqCst);
        let sealed = publish_hour(&owner, &mut output, 2, &|| {});
        assert_eq!(sealed.row().coverage, EventFeedCoverage::Complete);
        let slot = owner.validate_phase(stale);
        assert_eq!(slot, ((stale + 1) & 1) as usize);
        assert_eq!(
            owner.entering[(stale & 1) as usize].load(Ordering::SeqCst),
            0
        );
        owner.declines[slot].fetch_add(1, Ordering::SeqCst);
        owner.leave_phase(slot);
        let next = publish_hour(&owner, &mut output, 3, &|| {});
        assert_eq!(next.row().coverage, EventFeedCoverage::Unavailable);
        assert_eq!(next.row().counts.unwrap().rejected, 1);
        forwarders_off();
    }

    #[test]
    fn attempts_in_the_new_phase_cannot_hold_the_seals_drain() {
        use replay_data::event_feed_rows::EventFeedCoverage;
        let _globals = test_globals();
        let owner = sealing_owner();
        let mut output = Snapshot::new(0);
        publish_hour(&owner, &mut output, 1, &|| {});
        let phase = owner.phase.load(Ordering::SeqCst);
        let (old, new) = ((phase & 1) as usize, ((phase + 1) & 1) as usize);
        let done = AtomicBool::new(false);
        let sealed = std::thread::scope(|scope| {
            let stream = scope.spawn(|| {
                while !done.load(Ordering::SeqCst) {
                    let _ = owner.observe(Role::Script, |_| ());
                }
            });
            let paused = scope.spawn(|| {
                while owner.phase.load(Ordering::SeqCst) == phase {
                    std::thread::yield_now();
                }
                assert_eq!(owner.enter_phase(), new);
                owner.declines[old].fetch_add(1, Ordering::SeqCst);
                owner.leave_phase(old);
                while !done.load(Ordering::SeqCst) {
                    std::thread::yield_now();
                }
                owner.leave_phase(new);
            });
            let sealed = publish_hour(&owner, &mut output, 2, &|| {
                assert_eq!(owner.enter_phase(), old);
            });
            assert!(owner.entering[new].load(Ordering::SeqCst) >= 1);
            done.store(true, Ordering::SeqCst);
            stream.join().unwrap();
            paused.join().unwrap();
            sealed
        });
        assert_eq!(sealed.row().coverage, EventFeedCoverage::Unavailable);
        assert!(sealed.row().counts.unwrap().rejected >= 1);
        forwarders_off();
    }

    #[test]
    fn a_decline_recorded_in_this_interval_is_seen_by_certification() {
        let _globals = test_globals();
        let owner = Owner::new(0, Store::new(None).unwrap());
        owner.armed.store(true, Ordering::SeqCst);
        SHARED.store(Arc::as_ptr(&owner).cast_mut(), Ordering::SeqCst);
        owner
            .worker(Role::Census, |_| {
                assert_eq!(untaken_declines(), 0);
                std::thread::scope(|scope| {
                    scope
                        .spawn(|| assert!(owner.observe(Role::DelayedWriter, |_| ()).is_err()))
                        .join()
                        .unwrap();
                });
                assert_eq!(untaken_declines(), 1);
            })
            .unwrap();
        owner.observe(Role::Script, |_| ()).unwrap();
        assert_eq!(untaken_declines(), 0);
    }

    #[test]
    fn a_retained_owner_refuses_waiting_workers_and_skips_the_census() {
        let _globals = test_globals();
        assert_eq!(FAULT.load(Ordering::SeqCst), 0);
        let owner = Owner::new(0, Store::new(None).unwrap());
        owner.armed.store(true, Ordering::SeqCst);
        SHARED.store(Arc::as_ptr(&owner).cast_mut(), Ordering::SeqCst);
        owner
            .busy
            .store(u64::from(Role::Script.held()), Ordering::SeqCst);
        fault(OWNERSHIP_FAULT);
        let refusals = WORKER_REFUSALS[Role::Census as usize].load(Ordering::SeqCst);
        assert_eq!(
            owner.worker(Role::Census, |_| panic!(
                "a retained store is never entered"
            )),
            Err::<(), _>(WORKER_RETAINED)
        );
        assert!(begin_delayed_census(0, 0x1000, 1, false).is_none());
        assert_eq!(
            WORKER_REFUSALS[Role::Census as usize].load(Ordering::SeqCst),
            refusals + 2
        );
        assert_eq!(total_declines(&owner), 0);
        let mut snapshot = Snapshot::new(0);
        // SAFETY: Unit test holds `test_globals()` and exercises seal refusal in isolation.
        unsafe { seal(&mut snapshot, 1, false) };
        assert_eq!(snapshot.boundary.failure, Some(WORKER_RETAINED));
        assert_eq!(
            snapshot.boundary.coverage,
            replay_data::event_feed_rows::EventFeedCoverage::Unavailable
        );
        assert!(!owner.armed.load(Ordering::SeqCst));
        assert_eq!(total_declines(&owner), 0);
    }

    #[test]
    fn escaped_native_flight_retains_the_event_feed_owner_without_a_ticket() {
        extern "system" fn original() {}
        let _globals = test_globals();
        let code = crate::hook::TestCode::new();
        let target = original as *const () as usize;
        let (first, first_bytes) = code.stub(0, target);
        let (second, second_bytes) = code.stub(64, target);
        let held = Held {
            taps: Vec::new(),
            hooks: unsafe {
                vec![
                    Hook::new(first, target, first_bytes).unwrap(),
                    Hook::new(second, target, second_bytes).unwrap(),
                ]
            },
            owner: Owner::new(0, Store::new(None).unwrap()),
            restored: true,
        };
        assert_eq!(
            held.detach_losses(RETAIN | 1),
            Err("Event feed owner has unsettled native ownership")
        );
        assert_eq!(held.detach_losses(0), Ok(DetachLosses::default()));
    }

    #[test]
    fn a_disabled_forwarder_leaves_the_interval_partial() {
        use replay_data::event_feed_rows::EventFeedCoverage;
        let _globals = test_globals();
        forwarder(Forwarder::Politics, true);
        forwarder(Forwarder::Territory, true);
        forwarder(Forwarder::DelayedPass, true);
        assert_eq!(coverage(false), EventFeedCoverage::Complete);
        assert_eq!(coverage(true), EventFeedCoverage::Partial);
        for disabled in [
            Forwarder::Politics,
            Forwarder::Territory,
            Forwarder::DelayedPass,
        ] {
            forwarder(disabled, false);
            assert_eq!(coverage(false), EventFeedCoverage::Partial);
            forwarder(disabled, true);
        }
        forwarder(Forwarder::Politics, false);
        forwarder(Forwarder::Territory, false);
        forwarder(Forwarder::DelayedPass, false);
    }

    #[test]
    fn call_entries_unwind_to_the_native_return_with_the_original_stack() {
        for (entry, site) in [
            (event_war_entry as *const () as usize, native::WAR),
            (event_annex_entry as *const () as usize, native::ANNEX),
        ] {
            let first = unsafe { std::slice::from_raw_parts(entry as *const u8, 7) };
            assert_eq!(first, &[0x48, 0x81, 0xec, 8, 1, 0, 0]);
            let pc = entry as u64 + 7;
            let mut image_base = 0;
            let function = unsafe { RtlLookupFunctionEntry(pc, &mut image_base, ptr::null_mut()) };
            assert!(!function.is_null());
            assert_eq!(
                image_base + unsafe { (*function).BeginAddress } as u64,
                entry as u64
            );
            #[repr(align(16))]
            struct Stack([u64; 40]);
            let mut stack = Stack([0; 40]);
            let return_pc = 0x140000000 + site.rva.after(5).0 as u64;
            stack.0[33] = return_pc;
            let mut context: CONTEXT = unsafe { std::mem::zeroed() };
            context.Rip = pc;
            context.Rsp = stack.0.as_ptr() as u64;
            context.Rdi = 77;
            context.Rbp = 88;
            let mut handler_data = ptr::null_mut();
            let mut establisher = 0;
            unsafe {
                RtlVirtualUnwind(
                    0,
                    image_base,
                    pc,
                    function,
                    &mut context,
                    &mut handler_data,
                    &mut establisher,
                    ptr::null_mut(),
                );
            }
            assert_eq!(context.Rip, return_pc);
            assert_eq!(context.Rsp, stack.0.as_ptr() as u64 + 272);
            assert_eq!((context.Rdi, context.Rbp), (77, 88));
        }
    }
}
