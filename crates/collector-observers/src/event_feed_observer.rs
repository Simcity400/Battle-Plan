//! Native Event feed CALL observers, with hourly owned copies and output receipts.

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
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
};
use windows_sys::Win32::System::Threading::{WaitOnAddress, WakeByAddressAll};

const RETAIN: usize = 1 << 63;
const BEFORE_FAULT: u32 = 1;
const RETURNED_FAULT: u32 = 2;
const NATIVE_ESCAPE: u32 = 4;
const OWNERSHIP_FAULT: u32 = 8;

// Permanent aligned qword: low63 native flights; bit63 irreversible retention.
// Only entry ASM acquires and normal exit ASM releases. No reset or Rust Drop.
static FLIGHT: AtomicUsize = AtomicUsize::new(0);
static FAULT: AtomicU32 = AtomicU32::new(0);
static SHARED: AtomicPtr<Owner> = AtomicPtr::new(ptr::null_mut());
static TARGETS: [AtomicUsize; 2] = [AtomicUsize::new(0), AtomicUsize::new(0)];
static RECEIPTS: LazyLock<crossbeam_queue::ArrayQueue<(u64, bool)>> = LazyLock::new(|| {
    crossbeam_queue::ArrayQueue::new(crate::event_feed_history::PUBLICATION_CAPACITY)
});
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

/// The existing country worker's recovery of owner certificates, sealed in
/// the same hour's row. A certified owner does no additional native read.
#[derive(Clone, Copy)]
pub struct DelayedCensus {
    base: usize,
    state: usize,
    hour: i32,
    generation: u64,
    epoch: u64,
}

/// Owner certificates were retired, refused or may be missing since the
/// last complete census. Atomics only; no store entry.
pub fn recovery_pending() -> bool {
    crate::event_feed_history::RECOVERY.load(Ordering::Acquire)
        > crate::event_feed_history::RECOVERED.load(Ordering::Acquire)
}

/// Collector worker threads only. `full`, the daily census, requests a
/// recovery walk even with nothing pending. `None` without any refusal while
/// nothing may be read: a loss or refusal awaits its receipt, or the
/// boundary's registry baseline has not established this world.
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
/// Collector worker threads only: one live country of the census walk.
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
    // Keep each registry independent: its completed baseline is published
    // before the next registry starts reading. Every merge still rechecks
    // the cut of the complete private baseline it is about to publish.
    for registry in [
        crate::event_feed_history::OwnerRegistry::Decisions,
        crate::event_feed_history::OwnerRegistry::Delayed,
    ] {
        if !work[registry as usize] {
            continue;
        }
        let mut selected = [false; 2];
        selected[registry as usize] = true;
        // Reuse the selected owner's existing readers with a private owned
        // transaction. Neither its native reads nor its growth holds the lease.
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
        // The whole owner, surviving unacknowledged facts and its local refusal
        // reserve are now measured. Prepare the offered allocation off-lease.
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
/// Collector worker threads only. `complete`: the walk visited every live
/// country without a census error. Otherwise recovery stays pending for the
/// next hour's visit.
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
                false,
            )
        })();
        if let Err(reason) = result {
            store.reject(reason);
        }
        script_demands(store);
    });
}
/// `pass`: the authenticated iteration owner and the world its pass was
/// anchored in. A destroy outside a pass forwards none.
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

/// Snapshot offsets are duplicated only in the matching ASM/header.
/// The ticket bytes are opaque to C++; only Rust creates/consumes the ticket.
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

/// Who holds or wanted the shared store. Pointer-free admission and diagnostics.
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
    const ALL: [Self; ROLES] = [
        Self::WarCall,
        Self::Settlement,
        Self::Provenance,
        Self::Script,
        Self::ScriptInstance,
        Self::PendingRegistration,
        Self::DelayedWriter,
        Self::DelayedPass,
        Self::DecisionWriter,
        Self::PoliticalBalance,
        Self::Refusal,
        Self::BoundaryPrepare,
        Self::BoundarySeal,
        Self::Census,
        Self::Detach,
        Self::Staging,
    ];
    /// The busy word: zero is free.
    fn held(self) -> u8 {
        self as u8 + 1
    }
    /// The holder role in a busy word.
    fn holder(busy: u64) -> usize {
        ((busy & HOLDER) as usize).saturating_sub(1).min(ROLES - 1)
    }
    fn name(index: usize) -> &'static str {
        match Self::ALL[index] {
            Self::WarCall => "war_call",
            Self::Settlement => "settlement",
            Self::Provenance => "provenance",
            Self::Script => "script",
            Self::ScriptInstance => "script_instance",
            Self::PendingRegistration => "pending_registration",
            Self::DelayedWriter => "delayed_writer",
            Self::DelayedPass => "delayed_pass",
            Self::DecisionWriter => "decision_writer",
            Self::PoliticalBalance => "political_balance",
            Self::Refusal => "refusal",
            Self::BoundaryPrepare => "boundary_prepare",
            Self::BoundarySeal => "boundary_seal",
            Self::Census => "census",
            Self::Detach => "detach",
            Self::Staging => "staging",
        }
    }
}
/// Declined entries by (declined role, holder role).
static DECLINED: [[AtomicU64; ROLES]; ROLES] =
    [const { [const { AtomicU64::new(0) }; ROLES] }; ROLES];
/// Callback entries declined, by role, because a worker that lost a race
/// for the free store was owed it.
static DECLINED_FOR_WAITING_WORKER: [AtomicU64; ROLES] = [const { AtomicU64::new(0) }; ROLES];
/// Worker entries that waited, by (worker role, first observed holder role).
static WORKER_WAITS: [[AtomicU64; ROLES]; ROLES] =
    [const { [const { AtomicU64::new(0) }; ROLES] }; ROLES];
/// Worker entries that lost a race for the store and claimed the next one.
static WORKER_LOST_RACES: [AtomicU64; ROLES] = [const { AtomicU64::new(0) }; ROLES];
/// Worker entries refused because the owner was retained while they waited.
static WORKER_REFUSALS: [AtomicU64; ROLES] = [const { AtomicU64::new(0) }; ROLES];
/// Polling before a worker yields or a Candidate parks. Admission does not
/// depend on it: an owed worker has priority and a Candidate retries a live
/// Candidate holder without counting a decline.
const WORKER_SPINS: u32 = 64;
const WORKER_RETAINED: &str = "Event feed owner retained while the worker waited";

/// The busy word carries the holder's role and admission/wakeup flags.
const HOLDER: u64 = 0xff;
/// A worker that lost a race is owed the store: callbacks decline until it
/// enters.
const WORKER_WAITING: u64 = 1 << 8;
/// Only Candidate publication has the short, non-reentrant holder proof
/// needed for another native Candidate to wait. Other DecisionWriter sites
/// keep try-only admission. Separate from optional diagnostic attribution.
const CANDIDATE: u64 = 1 << 20;
/// A parked Candidate may need a wake on release. Cleared with ownership;
/// an uncontended release makes no address-wake call.
const CANDIDATE_WAITING: u64 = 1 << 21;

fn candidate_held(word: u64) -> bool {
    word & (HOLDER | CANDIDATE | WORKER_WAITING)
        == (u64::from(Role::DecisionWriter.held()) | CANDIDATE)
}

/// Bumped by every native callback's store entry attempt, entered or not,
/// and by the entry and return of every bracketed native original. A
/// baseline sample is cut against it. Constant work.
static WRITER_EPOCH: AtomicU64 = AtomicU64::new(0);
pub(crate) fn writer_moved() {
    WRITER_EPOCH.fetch_add(1, Ordering::SeqCst);
}
/// Inside a worker lease: a baseline sample's cut, or `None` while an
/// event-feed native original is in flight.
pub(crate) fn writer_cut() -> Option<u64> {
    let epoch = WRITER_EPOCH.load(Ordering::SeqCst);
    (active() & !RETAIN == 0).then_some(epoch)
}
/// No native writer attempted the store, ran a bracketed original or is in
/// flight since `cut`.
pub(crate) fn writer_cut_holds(cut: u64) -> bool {
    active() & !RETAIN == 0 && WRITER_EPOCH.load(Ordering::SeqCst) == cut
}

struct Owner {
    base: usize,
    armed: AtomicBool,
    /// [`HOLDER`] role, Candidate identity/wakeup flags and [`WORKER_WAITING`].
    busy: AtomicU64,
    /// Releases of the store, so a waiting worker sees that the holder it
    /// found left and another entry took the store first.
    releases: AtomicU64,
    /// The decline accounting interval. Each seal advances it before its
    /// release; its low bit selects the slot of the counters below.
    phase: AtomicU64,
    /// Callback entry attempts in progress in each slot's phase, from
    /// their validated increment until their decline, if any, is recorded.
    entering: [AtomicU64; 2],
    /// Callback entries declined in each slot's phases. Only grows.
    declines: [AtomicU64; 2],
    /// Declines of each slot some entry or seal has taken. Only grows:
    /// whoever raises it owns the raised range exactly once.
    taken: [AtomicU64; 2],
    /// Declines the last seal drained for its row, stored before
    /// [`Self::seal_loss_publication`].
    seal_loss_count: AtomicU64,
    /// That row's publication, [`NO_SEAL`] when it published none, or zero.
    /// The next entrant applies it to the store before anything else: the
    /// seal never touches the store after its release.
    seal_loss_publication: AtomicU64,
    store: UnsafeCell<Store>,
}
/// A drained seal loss with no sealed row: plain missed observations.
const NO_SEAL: u64 = u64::MAX;
// Each short store phase has exclusive checked access. No lease crosses native.
unsafe impl Send for Owner {}
unsafe impl Sync for Owner {}

impl Owner {
    /// Off-thread setup with live-sized storage and the retained sequence seed.
    fn new(base: usize, store: Store) -> Arc<Self> {
        Arc::new(Self {
            base,
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

    /// Failure to enter acquires no store entry or ticket. The native call
    /// still runs and releases its own flight on normal return. Only the
    /// missed interval is lost; settlement and escape faults stay terminal.
    /// These callbacks never wait or allocate here.
    fn observe<T>(
        &self,
        role: Role,
        apply: impl FnOnce(&mut Store) -> T,
    ) -> Result<T, &'static str> {
        self.access_mode(role, apply, false, false)
    }

    /// Candidate publication alone may wait for another Candidate callback.
    /// Its holder copies selected guarded operands into prepared storage: no
    /// original, getter, forwarded notification or game-thread wait, so it
    /// cannot nest another callback or wait for this native caller. The
    /// enclosing native Flight and its operand lifetime remain on the caller.
    fn observe_candidate<T>(&self, apply: impl FnOnce(&mut Store) -> T) -> Result<T, &'static str> {
        self.access_mode(Role::DecisionWriter, apply, false, true)
    }

    /// Outside the decline phase: a parked attempt must not hold up a seal.
    /// The next admission attempt joins the then-current phase and moves the
    /// writer cut again. A changed holder or owed worker is never waited on.
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
        // Register against this exact Candidate ownership word before
        // parking. Release clears the flag even if its wake precedes wait.
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
        // A foreign escape can retain ownership without changing busy or
        // releasing it. Periodically recheck that terminal state rather than
        // sleeping forever. Timeout/spurious wake never discards a Candidate.
        // SAFETY: a stable aligned qword and same-size local comparison.
        unsafe {
            WaitOnAddress(
                self.busy.as_ptr().cast(),
                (&parked as *const u64).cast(),
                size_of::<u64>(),
                1,
            )
        };
    }

    /// Join the current decline phase: increment its slot's entering
    /// count, then validate that the phase is still the one read.
    ///
    /// Validation closes the window between reading the phase and the
    /// increment. A seal flips the phase (SeqCst) as the last act of its
    /// lease, before releasing, and then drains the old slot (SeqCst
    /// loads). If this validating load still reads the old phase, it
    /// precedes the flip in the single SeqCst order, so the earlier
    /// increment precedes the seal's drain: the seal waits for this
    /// attempt's decline. If it reads a newer phase, the attempt leaves the
    /// old slot and joins the new one, so its decline belongs to the next
    /// seal. No attempt can record into a slot already settled.
    ///
    /// Join from a phase read earlier; returns the phase actually joined.
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
    /// Test-only: join the current phase; its slot.
    #[cfg(test)]
    fn enter_phase(&self) -> usize {
        self.validate_phase(self.phase.load(Ordering::SeqCst))
    }
    /// Test-only: join from a phase read earlier; its slot.
    #[cfg(test)]
    fn validate_phase(&self, phase: u64) -> usize {
        (self.join_phase(phase) & 1) as usize
    }
    fn leave_phase(&self, slot: usize) {
        self.entering[slot].fetch_sub(1, Ordering::SeqCst);
    }

    /// Enter the free store, or decline while it is held or owed to a
    /// waiting worker. The attempt runs inside its validated phase, and a
    /// decline is recorded in that phase before the attempt leaves it. A
    /// settlement follows the same protocol and is never declined for a
    /// waiting worker: its overlap would retain the owner. Only a Candidate
    /// facing another Candidate retries, outside the decline phase.
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
            // Retained; there is no missed observation to count.
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
        // The holder's own work stays armed; the next entry names this
        // loss as an overlap.
        Err(OBSERVATION_OVERLAP)
    }

    /// Collector worker threads only. A live holder never crosses native
    /// code while it holds the store, so it always releases; a retained
    /// owner ends the wait.
    /// The worker races for each release. Once it has lost one (the holder
    /// it found released and another entry took the store first) it is
    /// owed the next: callbacks decline until it enters, which is after at
    /// most the current hold. Nothing native was missed, so a refused
    /// worker counts no observation.
    fn worker<T>(
        &self,
        role: Role,
        apply: impl FnOnce(&mut Store) -> T,
    ) -> Result<T, &'static str> {
        self.enter_worker(role)?;
        self.held(apply, true, None)
    }

    /// The sealing lease. `apply` returns its value and the publication it
    /// sealed, if any. Its last act while still holding the store flips the
    /// decline phase; it then releases unconditionally and drains the old
    /// phase: it waits for every attempt that joined that phase to record
    /// its decline, then takes the slot's remaining declines. They were
    /// declined before the next interval began, so they refuse the sealed
    /// row. The store effects go to the next entrant. Returns how many the
    /// seal took.
    ///
    /// Flipping before the release means no entrant after the release can
    /// take the closing phase's declines: one that joined the new phase
    /// takes only that phase, and one that joined the old phase before the
    /// flip takes nothing ([`Self::take_missed`]), leaving them to this
    /// drain.
    ///
    /// The wait is bounded: only attempts that joined before the flip are
    /// counted, and each runs a few atomics with no native code before it
    /// leaves. Collector worker only.
    fn sealing<T>(
        &self,
        role: Role,
        apply: impl FnOnce(&mut Store) -> (T, Option<u64>),
    ) -> Result<(T, u64), &'static str> {
        self.enter_worker(role)?;
        let (value, publication, old) = self.held(
            |store| {
                let (value, publication) = apply(store);
                // The flip precedes the release.
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

    /// Wait for the store and enter it. Entry is a single update of the
    /// ownership word, so only ownership transitions bound it: counting
    /// declines never touches that word.
    fn enter_worker(&self, role: Role) -> Result<(), &'static str> {
        let mut spins = 0;
        let mut found: Option<u64> = None;
        let mut owed = false;
        loop {
            // This worker's claim is spent on entering.
            let word = match self
                .busy
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |word| {
                    (word & HOLDER == 0).then(|| (word & !WORKER_WAITING) | u64::from(role.held()))
                }) {
                Ok(_) => return Ok(()),
                Err(word) => word,
            };
            match found {
                None => {
                    WORKER_WAITS[role as usize][Role::holder(word)].fetch_add(1, Ordering::Relaxed);
                    found = Some(self.releases.load(Ordering::SeqCst));
                }
                Some(releases) if !owed => {
                    // Held although the holder it found released: another
                    // entry won the race for that release.
                    if self.releases.load(Ordering::SeqCst) != releases {
                        owed = true;
                        self.busy.fetch_or(WORKER_WAITING, Ordering::SeqCst);
                        WORKER_LOST_RACES[role as usize].fetch_add(1, Ordering::Relaxed);
                    }
                }
                // Another owed worker entering spends the shared claim; keep
                // this one's.
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
        }
    }

    /// The store is held. Applies the last seal's drained loss before
    /// anything else, then this entry's missed observations; releases
    /// unconditionally.
    /// `phase` is the decline phase a callback joined for its entry; a
    /// worker, which joins none, passes `None` and takes the current phase.
    fn held<T>(
        &self,
        apply: impl FnOnce(&mut Store) -> T,
        take_missed: bool,
        phase: Option<u64>,
    ) -> Result<T, &'static str> {
        // A Rust panic is contained. A foreign escape is contained by the C++
        // observer-only guard; it may leave busy set, which retains the owner.
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
                // No payload destructor or guessed lock/ticket cleanup after fault.
                std::mem::forget(payload);
                Err("Event feed callback panic; owner retained")
            }
        }
    }

    /// Declines of `slot` recorded so far that nobody has taken; raising
    /// its `taken` owns them exactly once.
    fn take_declines(&self, slot: usize) -> u64 {
        let declines = self.declines[slot].load(Ordering::SeqCst);
        declines.saturating_sub(self.taken[slot].fetch_max(declines, Ordering::SeqCst))
    }
    /// The current phase's slot.
    fn current(&self) -> usize {
        (self.phase.load(Ordering::SeqCst) & 1) as usize
    }
    /// Declines recorded in the current phase that nobody has taken.
    fn untaken(&self) -> u64 {
        let slot = self.current();
        self.declines[slot]
            .load(Ordering::SeqCst)
            .saturating_sub(self.taken[slot].load(Ordering::SeqCst))
    }

    /// Declines of the entry's own phase nobody has taken refuse its
    /// interval, and only while that phase is still current: a callback
    /// that joined the closing phase before a seal's flip takes nothing,
    /// leaving its declines to that seal's drain. One still in flight is
    /// caught by the next seal's drain, which then refuses the row it seals.
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

    /// Revoke before any unobserved mutation. This atomic operation does not
    /// settle tickets/flights or establish that a world replacement succeeded.
    fn revoke_world(&self) {
        self.armed.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
thread_local! {
    static BEFORE_CANDIDATE_WAIT: std::cell::Cell<Option<fn()>> = const { std::cell::Cell::new(None) };
}

/// Declines recorded in the current interval that no entry has taken: a
/// writer was lost while the caller held the store, so it certifies nothing.
pub(crate) fn untaken_declines() -> u64 {
    let lost = unsafe { SHARED.load(Ordering::SeqCst).as_ref() }.map_or(0, Owner::untaken);
    #[cfg(test)]
    let lost = lost + TEST_UNTAKEN_DECLINES.load(Ordering::SeqCst);
    lost
}
/// Test only: declines recorded in the current interval by a store a
/// history test does not own.
#[cfg(test)]
pub(crate) static TEST_UNTAKEN_DECLINES: AtomicU64 = AtomicU64::new(0);

/// Test-only: the seal has just released its lease and waits here until a
/// test admits a callback in exactly that window.
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

/// Native or callback ownership is retained: nothing may enter the store.
fn retained() -> bool {
    FAULT.load(Ordering::SeqCst) != 0 || active() & RETAIN != 0
}

fn fault(code: u32) {
    // Static pointer-free, allocation-free sink. Even an unacquired escape
    // prevents later certification/free; no logical or physical count settles.
    FAULT.fetch_or(code, Ordering::SeqCst);
    FLIGHT.fetch_or(RETAIN, Ordering::SeqCst);
}

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
        // Only this final normal write grants the single terminal callback.
        frame.terminal = 1;
        Ok(true)
    })?
}

/// Record an event a provenance hook proved ([`crate::war_provenance`]):
/// `observe` reads it from the current world, `Ok(None)` when the operation
/// changed nothing. Refused, counted and named like a CALL observation when
/// the world is not observed, an operand does not read or storage is full.
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
        // A refusal is counted and named by the store.
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

/// Same owned occurrence stream and receipt boundary as diplomacy events.
/// Only the native writer's reached definition and bound graph are acquired.
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

/// A native original with no event-feed callback after its stores, which
/// a writer bracketed by another family forwards: from before its
/// publication through the original's return. Like a script Flight, it moves
/// the writer epoch on entry and drop, and an exit without
/// [`Self::returned`] retains native-flight ownership.
pub struct WriterFlight(crate::script_event_observer::Flight);
impl WriterFlight {
    pub fn enter() -> Self {
        Self(crate::script_event_observer::Flight::enter())
    }
    /// The original returned normally.
    pub fn returned(&mut self) {
        self.0.returned();
    }
}

/// Refuse a provenance event whose operands did not read.
pub fn refuse(reason: &'static str) {
    if let Some(owner) = unsafe { SHARED.load(Ordering::SeqCst).as_ref() } {
        let _ = owner.observe(Role::Refusal, |store| store.reject(reason));
    }
}

/// A detour latch appends its witness to the store's arena, tagged with its
/// frame token. A declined entry is a missed observation like any callback,
/// and arena exhaustion refuses by name and raises demand.
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
/// The witness arena's epoch, recorded by a detour at entry. `None` when
/// the entry was declined: a missed observation refuses the interval anyway.
pub(crate) fn staging_epoch() -> Option<u64> {
    let owner = unsafe { SHARED.load(Ordering::SeqCst).as_ref() }?;
    owner
        .observe(Role::Staging, |store| store.staged_epoch())
        .ok()
}
/// A detour's publication takes its frame's next staged witness in native
/// order. `Ok(None)` ends the frame, including when the entry was declined:
/// a missed observation refuses the interval either way. An arena reset
/// since `epoch` is a named refusal.
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

/// The observed world's generation, or `None` after refusing, as a CALL
/// observation does.
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
    // The store entry/proof was durable before original; ticket consumption is
    // once-only even if this callback fails after taking the opaque token.
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

// C++/ASM own these stack frames. No game pointer is dereferenced directly.
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
    /// The two CALL hooks, then the provenance entry detours.
    hooks: Vec<Hook>,
    taps: Vec<Tap>,
    owner: Arc<Owner>,
    restored: bool,
}
/// Hooks and history remain owned until restoration and the final quiet check.
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
    ) -> Result<Self, &'static str> {
        let mut store = Store::new(last_sequence)?;
        store.restore_counts(counts);
        LazyLock::force(&RECEIPTS);
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
            let address = native::verify(memory, base, site)?;
            unsafe {
                Hook::new_call_with_safety(
                    address,
                    callback,
                    site.sentinel,
                    TapSafety {
                        in_flight: &FLIGHT,
                        stub: range.clone(),
                    },
                )
            }
            .map_err(|_| "Event feed CALL construction failed")
        };
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
            return Err("Event feed previous owner retained");
        }
        TARGETS[0].store(base + native::WAR.target_rva, Ordering::SeqCst);
        TARGETS[1].store(base + native::ANNEX.target_rva, Ordering::SeqCst);
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

    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        let Some(held) = self.held.as_mut() else {
            return Ok(true);
        };
        // A delayed pass holds a cursor anchor until its original returns.
        // Keep its observations installed until that native flight completes.
        if !enabled && active() != 0 {
            return Ok(false);
        }
        if !enabled {
            held.owner.revoke_world();
        }
        let Held { hooks, taps, .. } = &mut **held;
        let mut family: Vec<_> = hooks
            .iter_mut()
            .chain(taps.iter_mut().map(Tap::publication_hook))
            .collect();
        let ready = Hook::set_enabled_batch_attach(&mut family, enabled).map_err(|cause| {
            let label = family
                .iter()
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
        self.held
            .as_ref()
            .into_iter()
            .flat_map(|held| {
                held.hooks
                    .iter()
                    .flat_map(Hook::executable_ranges)
                    .chain(held.taps.iter().flat_map(Tap::executable_ranges))
            })
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
            take_receipts(store);
            store.detach_losses()
        })?
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        // Also releases unpublished bootstrap storage on cancellation.
        let _ = self.release_after_quiet();
    }
}

fn take_receipts(store: &mut Store) {
    while let Some((publication, accepted)) = RECEIPTS.pop() {
        if accepted {
            store.acknowledged(publication);
        } else {
            store.discarded(publication);
        }
    }
}

/// Native flights of the CALL hooks (bit 63 retains the owner) and the
/// provenance entry detours in flight.
pub fn active() -> usize {
    FLIGHT
        .load(Ordering::SeqCst)
        .saturating_add(crate::war_provenance::active())
        .saturating_add(crate::script_event_observer::active())
}

/// A family whose patch forwards provenance to the event feed. While one is
/// not enabled its events or causes go unobserved, so no interval is
/// complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forwarder {
    /// [`crate::politics_changes`]: the diplomatic dispatch and civil war
    /// start that declarations happen in.
    Politics,
    /// [`crate::territory_changes`]: set owner, every state
    /// transfer.
    Territory,
    /// The script-variable family owns and forwards delayed event passes.
    DelayedPass,
}

static FORWARDERS: [AtomicBool; 3] = [const { AtomicBool::new(false) }; 3];

pub fn forwarder(forwarder: Forwarder, enabled: bool) {
    if matches!(forwarder, Forwarder::Politics) && !enabled {
        decisions::political_balance_lost();
    }
    FORWARDERS[forwarder as usize].store(enabled, Ordering::SeqCst);
}

/// An observed interval's coverage: complete only when the capture is and
/// every forwarder is enabled.
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

/// Live war relations the latest Wars walk counted, for sizing: one
/// conference can end every one of them in an hour.
static WARS: AtomicUsize = AtomicUsize::new(0);

pub fn note_wars(count: usize) {
    WARS.store(count, Ordering::Relaxed);
}

pub fn lost_drain() {
    if let Some(owner) = unsafe { SHARED.load(Ordering::SeqCst).as_ref() } {
        owner.revoke_world();
    }
}

/// The interval's input failure, read again by each boundary half.
fn input_failure(owner: &Owner) -> Option<&'static str> {
    if retained() {
        Some("Event feed native or callback ownership retained")
    } else if !owner.armed.load(Ordering::SeqCst) {
        Some("Event feed publication lost; interval unavailable")
    } else {
        crate::world_lifetime::unavailable_reason()
    }
}

/// The observer boundary job's half on a collector worker: take receipts
/// and missed observations, install offered capacity and acquire the
/// registry baseline. The same hour's census may then recover owners before
/// [`seal_growing`] publishes it. An error invalidates and is returned.
pub fn prepare_growing(hour: i32, partial: bool) -> Result<(), &'static str> {
    let generation = crate::world_lifetime::generation();
    let owner =
        unsafe { SHARED.load(Ordering::SeqCst).as_ref() }.ok_or("Event feed hooks unavailable")?;
    owner
        .worker(Role::BoundaryPrepare, |store| {
            take_receipts(store);
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

/// Only pointer-stable, ticket-free storage may be replaced. Allocation and
/// destruction remain on the sizing worker, outside this short merge lease.
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

/// The prepared hour's publication on a collector worker, after its census.
/// One lease measures it, the output grows off the lease, and a second lease
/// seals without growing. Nothing allocates under the shared-store lease.
pub fn seal_growing(output: &mut Snapshot, hour: i32, partial: bool) {
    seal_hour(output, hour, partial, || {});
}

/// `during` runs while the sealing lease is held, after the seal.
fn seal_hour(output: &mut Snapshot, hour: i32, partial: bool, during: impl FnOnce()) {
    let generation = crate::world_lifetime::generation();
    let Some(owner) = (unsafe { SHARED.load(Ordering::SeqCst).as_ref() }) else {
        output.refuse(generation, hour, "Event feed hooks unavailable");
        return;
    };
    match owner.worker(Role::BoundarySeal, |store| store.seal_demand()) {
        // A failed reservation leaves the sealing lease to refuse the hour
        // with its storage shortage.
        Ok(Ok(demand)) => {
            let _ = output.reserve_with_headroom(demand);
        }
        // The sealing lease names the same demand failure.
        Ok(Err(_)) => {}
        Err(reason) => {
            owner.revoke_world();
            output.refuse(generation, hour, reason);
            return;
        }
    }
    let sealed = owner.sealing(Role::BoundarySeal, |store| {
        let boundary = crate::event_feed_history::OwnerBoundary {
            generation,
            read_hour: hour,
            coverage: coverage(partial),
            failure: input_failure(owner),
        };
        // Callbacks may have added facts since the measure: a shortfall
        // refuses the hour with its storage reason.
        let result = store.seal(output, boundary, false);
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
    match sealed {
        Ok((Ok(()), declined)) => {
            if declined != 0 {
                // Declined while this seal held the store: before the next
                // interval began, so they refuse this row and only this row.
                output.declined_during_seal(declined);
            }
        }
        Ok((Err(reason), _)) => output.refuse(generation, hour, reason),
        Err(reason) => {
            // Like a lost drain: the next interval is named unavailable.
            owner.revoke_world();
            output.refuse(generation, hour, reason);
        }
    }
}

pub fn accepted(pack: &crate::event_feed_history::Prepared) -> std::io::Result<()> {
    if pack.publication() != 0 {
        RECEIPTS
            .push((pack.publication(), true))
            .map_err(|_| std::io::Error::other("Event feed output receipt queue full"))?;
    }
    Ok(())
}

pub fn discarded(publication: u64) {
    if publication != 0 && RECEIPTS.push((publication, false)).is_err() {
        // The frame pool plus the prepared spool write bound receipts.
        fault(OWNERSHIP_FAULT);
    }
}

/// Spool-only growth and retirement; native callbacks never allocate or free.
pub fn presize(slots: usize) {
    engine_allocation::advisory(|| presize_storage(slots));
}

fn presize_storage(slots: usize) {
    if active() & RETAIN != 0 {
        return;
    }
    if let Ok(mut retired) = RETIRED.lock() {
        drop(retired.take());
    }
    let mut capacity = CAPACITY.load(Ordering::Acquire);
    crate::table_capture::grow_capacity(
        &mut capacity,
        // Two war links per country, and one hour's state owner changes and
        // ended war relations at most every state and war at once.
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

const CONTENTION_SCOPE: &str = concat!(
    "since load: native callbacks the shared store declined, by declined and holder role; ",
    "callbacks declined for a worker owed the free store, by declined role; ",
    "collector worker entries that waited, by worker and first holder role; ",
    "worker entries that lost a race for the store, by worker role; ",
    "worker entries refused because the owner was retained, by worker role",
);
#[derive(serde::Serialize)]
struct Contention {
    declined: &'static str,
    holder: &'static str,
    count: u64,
}
#[derive(serde::Serialize)]
struct WorkerWait {
    worker: &'static str,
    holder: &'static str,
    count: u64,
}
#[derive(serde::Serialize)]
struct WorkerRefusal {
    worker: &'static str,
    count: u64,
}
#[derive(serde::Serialize)]
struct WaitingDecline {
    declined: &'static str,
    count: u64,
}
/// Nonzero per-role counters.
fn roles(counters: &'static [AtomicU64; ROLES]) -> impl Iterator<Item = (&'static str, u64)> {
    counters.iter().enumerate().filter_map(|(role, count)| {
        let count = count.load(Ordering::Relaxed);
        (count != 0).then(|| (Role::name(role), count))
    })
}
/// Nonzero cells of a (requesting role, holder role) matrix.
fn cells(
    matrix: &'static [[AtomicU64; ROLES]; ROLES],
) -> impl Iterator<Item = (&'static str, &'static str, u64)> {
    matrix.iter().enumerate().flat_map(|(role, holders)| {
        holders
            .iter()
            .enumerate()
            .filter_map(move |(holder, count)| {
                let count = count.load(Ordering::Relaxed);
                (count != 0).then(|| (Role::name(role), Role::name(holder), count))
            })
    })
}
/// Streams a sequence as it is read; status collects nothing.
struct Streamed<F>(F);
impl<F, I> serde::Serialize for Streamed<F>
where
    F: Fn() -> I,
    I: Iterator,
    I::Item: serde::Serialize,
{
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq((self.0)())
    }
}

fn repeated(registry: crate::event_feed_history::OwnerRegistry) -> u64 {
    crate::event_feed_history::REPEATED_OWNER_REFUSALS[registry as usize].load(Ordering::Relaxed)
}

pub fn status() -> impl serde::Serialize {
    let contention = Streamed(|| {
        cells(&DECLINED).map(|(declined, holder, count)| Contention {
            declined,
            holder,
            count,
        })
    });
    let worker_waits = Streamed(|| {
        cells(&WORKER_WAITS).map(|(worker, holder, count)| WorkerWait {
            worker,
            holder,
            count,
        })
    });
    let workers = |counters: &'static [AtomicU64; ROLES]| {
        Streamed(move || roles(counters).map(|(worker, count)| WorkerRefusal { worker, count }))
    };
    let declined_for_waiting_worker = Streamed(|| {
        roles(&DECLINED_FOR_WAITING_WORKER)
            .map(|(declined, count)| WaitingDecline { declined, count })
    });
    let crossed =
        |sample: usize| crate::event_feed_history::CROSSED_SAMPLES[sample].load(Ordering::Relaxed);
    crate::status::status!({
        "enabled": unsafe { SHARED.load(Ordering::SeqCst).as_ref() }.is_some_and(|owner| owner.armed.load(Ordering::SeqCst)),
        "unhooked": crate::world_lifetime::unavailable_reason(),
        "native_flights": active(), "faults": FAULT.load(Ordering::SeqCst),
        "provenance": crate::war_provenance::status(),
        "recovery_pending": recovery_pending(),
        // Refusals of an owner already refused in its loss epoch: counted,
        // never published again.
        "repeated_owner_refusals": {
            "delayed_events": repeated(crate::event_feed_history::OwnerRegistry::Delayed),
            "decisions": repeated(crate::event_feed_history::OwnerRegistry::Decisions),
        },
        // Baseline samples a native writer crossed: none was certified.
        "crossed_samples": {
            "registry": crossed(crate::event_feed_history::REGISTRY_SAMPLE),
            "delayed": crossed(crate::event_feed_history::OwnerRegistry::Delayed as usize),
            "decisions": crossed(crate::event_feed_history::OwnerRegistry::Decisions as usize),
        },
        "contention": contention,
        "declined_for_waiting_worker": declined_for_waiting_worker,
        "worker_waits": worker_waits,
        "worker_lost_races": workers(&WORKER_LOST_RACES),
        "worker_refusals": workers(&WORKER_REFUSALS),
        "contention_scope": CONTENTION_SCOPE,
    })
}

/// Test only: install a fresh SHARED owner whose store has a prepared
/// staging arena, holding the family lock, and restore the globals on drop.
/// Lets other modules' detour tests drive staging through the real store.
#[cfg(test)]
pub(crate) struct StagingTest {
    shared: *mut Owner,
    faults: u32,
    flights: usize,
    _owner: Arc<Owner>,
    _guard: std::sync::MutexGuard<'static, ()>,
}
#[cfg(test)]
pub(crate) fn staging_test(capacity: usize) -> StagingTest {
    let guard = crate::hook::FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let test = StagingTest {
        shared: SHARED.load(Ordering::SeqCst),
        faults: FAULT.load(Ordering::SeqCst),
        flights: FLIGHT.load(Ordering::SeqCst),
        _owner: {
            let mut store = Store::new(None).unwrap();
            store.prepare_decisions(capacity).unwrap();
            Owner::new(0, store)
        },
        _guard: guard,
    };
    FAULT.store(0, Ordering::SeqCst);
    FLIGHT.store(0, Ordering::SeqCst);
    test._owner.armed.store(true, Ordering::SeqCst);
    SHARED.store(Arc::as_ptr(&test._owner).cast_mut(), Ordering::SeqCst);
    test
}
#[cfg(test)]
impl StagingTest {
    /// Live witnesses staged in the store but not yet published.
    pub(crate) fn staged_live(&self) -> usize {
        unsafe { (*self._owner.store.get()).staged_live() }
    }
    /// A boundary's first half on a worker, which resets the witness arena.
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
    /// The store's interval failure and rejections, read between entries.
    pub(crate) fn failure(&self) -> (Option<&'static str>, u64) {
        let store = unsafe { &*self._owner.store.get() };
        (store.failure_for_test(), store.rejected_for_test())
    }
}
#[cfg(test)]
impl Drop for StagingTest {
    fn drop(&mut self) {
        SHARED.store(self.shared, Ordering::SeqCst);
        FAULT.store(self.faults, Ordering::SeqCst);
        FLIGHT.store(self.flights, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        CONTEXT, RtlLookupFunctionEntry, RtlVirtualUnwind,
    };

    struct RestoreTestGlobals {
        shared: *mut Owner,
        target: usize,
        faults: u32,
        flights: usize,
    }
    impl RestoreTestGlobals {
        fn new() -> Self {
            Self {
                shared: SHARED.load(Ordering::SeqCst),
                target: TARGETS[0].load(Ordering::SeqCst),
                faults: FAULT.load(Ordering::SeqCst),
                flights: FLIGHT.load(Ordering::SeqCst),
            }
        }
    }
    impl Drop for RestoreTestGlobals {
        fn drop(&mut self) {
            // Test fixture isolation only. Production never resets a terminal
            // fault or a retained flight, including after an observation gap.
            SHARED.store(self.shared, Ordering::SeqCst);
            TARGETS[0].store(self.target, Ordering::SeqCst);
            FAULT.store(self.faults, Ordering::SeqCst);
            FLIGHT.store(self.flights, Ordering::SeqCst);
        }
    }

    /// Declines counted that no store entry has taken.
    fn untaken(owner: &Owner) -> u64 {
        (0..2)
            .map(|slot| {
                owner.declines[slot].load(Ordering::SeqCst)
                    - owner.taken[slot].load(Ordering::SeqCst)
            })
            .sum()
    }
    /// Declines recorded in either phase slot.
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
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
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
        // The exact holder is named, and the holder's own later work is not
        // misreported as an unobserved world.
        assert_eq!(declined.load(Ordering::SeqCst), declined_before + 1);
        assert!(owner.armed.load(Ordering::SeqCst));
        let mut snapshot = Snapshot::new(0);
        owner
            .observe(Role::BoundarySeal, |store| {
                assert_eq!(store.outstanding(), 0);
                store.snapshot_growing(
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
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
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

    /// Already copied Candidate operands: repeated tokens remain separate
    /// occurrences. No native pointer is needed by this admission fixture.
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
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
        let owner = sealing_owner();
        owner
            .worker(Role::Census, |store| store.prepare_scripts(64, 0, 0))
            .unwrap()
            .unwrap();
        let mut output = Snapshot::new(0);
        // Production establishes the world before callbacks can publish.
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
                    // The allocator's denial guard names one thread at a
                    // time: finish this guard before starting the waiter.
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
            // Release (and optionally seal/phase flip) before the actual
            // address wait: its compare must prevent a missed wakeup.
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
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
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
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
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
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
        let owner = Owner::new(0, Store::new(None).unwrap());
        let holder = u64::from(Role::DecisionWriter.held()) | CANDIDATE;
        owner.busy.store(holder, Ordering::SeqCst);
        std::thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                BEFORE_CANDIDATE_WAIT.with(|hook| hook.set(Some(before_wait)));
                owner.observe_candidate(|_| panic!("a retained Candidate store was entered"))
            });
            REGISTERED.wait();
            // An escaped holder need not release or change the address being
            // waited on. The wait must still observe terminal retention.
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
        use crate::event_feed_history::{WITNESS_STORAGE, Witness};
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
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
                Err(WITNESS_STORAGE)
            );
        });
        assert_eq!(store.capacity_demand(), 4097);
        script_demands(&store);
        // Entry and script capacities already cover demand, on both the
        // active store and offered spare. Only witnesses force replacement.
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
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
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
                        // Release only once the worker has had to wait.
                        while WORKER_WAITS[Role::Census as usize][Role::DelayedPass as usize]
                            .load(Ordering::SeqCst)
                            == waited
                        {
                            std::thread::yield_now();
                        }
                    })
                    .unwrap();
            })
        };
        while !entered.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
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
        // It won the race at the release: no claim, no callback declined.
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
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
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
        // A callback holds the store when the worker arrives.
        owner
            .busy
            .store(u64::from(Role::Script.held()), Ordering::SeqCst);
        let entered = Arc::new(AtomicBool::new(false));
        let leave = Arc::new(AtomicBool::new(false));
        let worker = {
            let owner = Arc::clone(&owner);
            let entered = Arc::clone(&entered);
            let leave = Arc::clone(&leave);
            std::thread::spawn(move || {
                owner.worker(Role::Census, |_| {
                    entered.store(true, Ordering::SeqCst);
                    while !leave.load(Ordering::SeqCst) {
                        std::thread::yield_now();
                    }
                })
            })
        };
        while waits.load(Ordering::SeqCst) == waited {
            std::thread::yield_now();
        }
        // That holder releases and another callback takes the store first.
        owner
            .busy
            .store(u64::from(Role::Provenance.held()), Ordering::SeqCst);
        owner.releases.fetch_add(1, Ordering::SeqCst);
        while owner.busy.load(Ordering::SeqCst) & WORKER_WAITING == 0 {
            std::thread::yield_now();
        }
        assert_eq!(
            WORKER_LOST_RACES[Role::Census as usize].load(Ordering::SeqCst),
            lost + 1
        );
        // The second holder releases: from now on every callback attempt is
        // declined and counted until the owed worker has entered.
        owner.busy.fetch_and(!HOLDER, Ordering::SeqCst);
        owner.releases.fetch_add(1, Ordering::SeqCst);
        let mut attempts = 0;
        while !entered.load(Ordering::SeqCst) {
            assert_eq!(
                owner.observe(Role::Script, |_| panic!("an owed store is never taken")),
                Err::<(), _>(OBSERVATION_OVERLAP)
            );
            attempts += 1;
        }
        // Held by the worker, the store still declines.
        assert!(owner.observe(Role::Script, |_| ()).is_err());
        attempts += 1;
        leave.store(true, Ordering::SeqCst);
        assert_eq!(worker.join().unwrap(), Ok(()));
        assert_eq!(owner.busy.load(Ordering::SeqCst) & WORKER_WAITING, 0);
        assert_eq!(
            waiting.load(Ordering::SeqCst) - for_waiting + held_by_worker.load(Ordering::SeqCst)
                - held_before,
            attempts
        );
        assert_eq!(total_declines(&owner), attempts);
    }

    #[test]
    fn a_callback_declines_for_an_owed_worker_and_counts_it_apart_from_holders() {
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
        let owner = Owner::new(0, Store::new(None).unwrap());
        owner.armed.store(true, Ordering::SeqCst);
        let waiting = &DECLINED_FOR_WAITING_WORKER[Role::Script as usize];
        let for_waiting = waiting.load(Ordering::SeqCst);
        let held = total(&DECLINED[Role::Script as usize]);
        // Free, but owed to a worker that lost a race.
        owner.busy.store(WORKER_WAITING, Ordering::SeqCst);
        assert_eq!(
            owner.observe(Role::Script, |_| panic!("an owed store is never taken")),
            Err::<(), _>(OBSERVATION_OVERLAP)
        );
        assert_eq!(waiting.load(Ordering::SeqCst), for_waiting + 1);
        assert_eq!(total(&DECLINED[Role::Script as usize]), held);
        assert_eq!(total_declines(&owner), 1);
        // A settlement is never declined for a waiting worker: its overlap
        // would retain the owner.
        assert_eq!(owner.access(Role::Settlement, |_| 3), Ok(3));
        assert_eq!(FAULT.load(Ordering::SeqCst), 0);
        // A settlement takes no missed observation: the decline waits for
        // the next observing entry.
        owner.busy.fetch_and(!WORKER_WAITING, Ordering::SeqCst);
        assert_eq!(untaken(&owner), 1);
    }

    #[test]
    fn a_declined_callback_entry_attempt_breaks_a_baseline_cut() {
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
        assert_eq!(FLIGHT.load(Ordering::SeqCst), 0);
        let owner = Owner::new(0, Store::new(None).unwrap());
        owner.armed.store(true, Ordering::SeqCst);
        owner
            .worker(Role::Census, |_| {
                let cut = writer_cut().unwrap();
                assert!(writer_cut_holds(cut));
                // A native callback on another thread is declined while this
                // worker holds the store, yet its attempt crosses the sample.
                std::thread::scope(|scope| {
                    scope
                        .spawn(|| assert!(owner.observe(Role::DelayedWriter, |_| ()).is_err()))
                        .join()
                        .unwrap();
                });
                assert!(!writer_cut_holds(cut));
                // So does a bracketed native original, even after it returned.
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

    /// An armed published owner, every forwarder enabled and this thread's
    /// world observed: the production seal path without native memory.
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
    /// The boundary's first half without a registry baseline, then the seal
    /// with `during` inside its lease, and the output's receipt.
    fn publish_hour(
        owner: &Owner,
        output: &mut Snapshot,
        hour: i32,
        during: &dyn Fn(),
    ) -> crate::event_feed_history::Prepared {
        owner
            .worker(Role::BoundaryPrepare, |store| {
                take_receipts(store);
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
        let pack = output.prepare_pack().unwrap();
        accepted(&pack).unwrap();
        pack
    }
    /// Test only: the store while nothing holds it.
    fn store(owner: &Owner) -> &Store {
        assert_eq!(owner.busy.load(Ordering::SeqCst) & HOLDER, 0);
        unsafe { &*owner.store.get() }
    }

    #[test]
    fn a_decline_during_the_seals_hold_refuses_only_the_sealed_row() {
        use replay_data::event_feed_rows::EventFeedCoverage;
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
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
        // The seal drained the decline after its release; the store never
        // saw it then. The next entrant applies the certificate retirement,
        // pinned to the sealed row.
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
        // Its receipt ends the loss: the first healthy endpoint is Partial.
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
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
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
            // Hold the sealing lease until callbacks have been declined in
            // it; they keep arriving while it releases.
            let sealed = publish_hour(&owner, &mut output, 2, &|| {
                while total_declines(&owner) < at + 8 {
                    std::thread::yield_now();
                }
            });
            stop.store(true, Ordering::SeqCst);
            (sealed, spammer.join().unwrap())
        });
        assert_eq!(sealed.row().coverage, EventFeedCoverage::Unavailable);
        // One more entry takes whatever the seal's read did not.
        owner.observe(Role::Script, |_| ()).unwrap();
        let total = total_declines(&owner) - before;
        assert_eq!(spammed, total);
        assert_eq!(untaken(&owner), 0);
        // The seal's share and every later entry's share add up exactly.
        assert_eq!(store(&owner).rejected_for_test() - rejected, total);
        forwarders_off();
    }

    #[test]
    fn a_worker_enters_while_declines_are_being_counted() {
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
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
            // The holder releases amid the declines; the worker enters.
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
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
        assert_eq!(FAULT.load(Ordering::SeqCst), 0);
        let owner = sealing_owner();
        let mut output = Snapshot::new(0);
        publish_hour(&owner, &mut output, 1, &|| {});
        let phase = owner.phase.load(Ordering::SeqCst);
        let slot = (phase & 1) as usize;
        let sealed = std::thread::scope(|scope| {
            // A callback failed its entry during the seal's hold and pauses
            // before recording its decline, until after the release and flip.
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
        // Settled once: the first healthy endpoint remains Partial.
        let next = publish_hour(&owner, &mut output, 3, &|| {});
        assert_eq!(next.row().coverage, EventFeedCoverage::Partial);
        assert_eq!(next.row().counts.unwrap().rejected, 1);
        assert_eq!(untaken(&owner), 0);
        forwarders_off();
    }

    #[test]
    fn a_callback_admitted_after_the_seals_release_takes_nothing_from_the_closing_phase() {
        use replay_data::event_feed_rows::EventFeedCoverage;
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
        let owner = sealing_owner();
        let mut output = Snapshot::new(0);
        publish_hour(&owner, &mut output, 1, &|| {});
        let phase = owner.phase.load(Ordering::SeqCst);
        let old = (phase & 1) as usize;
        // The seal pauses right after its release; nothing waits for the
        // flip. Had the flip followed the release, the phase would still be
        // the old one here and this callback's take_missed would take the
        // decline below, leaving the sealed row Complete.
        SEAL_RELEASED.store(false, Ordering::SeqCst);
        SEAL_RESUME.store(false, Ordering::SeqCst);
        let (sealed, seen) = std::thread::scope(|scope| {
            // A callback that joined the old phase during the hold reaches
            // its entry attempt in the window right after the release.
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
                // It still holds the old phase, so the seal cannot settle
                // yet: anything it saw comes from its own take_missed.
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
                // Declined during the hold: recorded in the old phase.
                assert_eq!(owner.enter_phase(), old);
                owner.declines[old].fetch_add(1, Ordering::SeqCst);
                owner.leave_phase(old);
                // The late callback's validated join of the old phase.
                assert_eq!(owner.enter_phase(), old);
            });
            (sealed, late.join().unwrap())
        });
        // The late entrant took nothing from the closing phase.
        assert_eq!(seen, (None, 0));
        // The seal settled the decline into the sealed row.
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
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
        let owner = sealing_owner();
        let mut output = Snapshot::new(0);
        publish_hour(&owner, &mut output, 1, &|| {});
        let sealed = publish_hour(&owner, &mut output, 2, &|| {});
        assert_eq!(sealed.row().coverage, EventFeedCoverage::Complete);
        // After the flip, the next holder declines a callback.
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
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
        let owner = sealing_owner();
        let mut output = Snapshot::new(0);
        publish_hour(&owner, &mut output, 1, &|| {});
        // The callback read the phase, then paused before its increment.
        let stale = owner.phase.load(Ordering::SeqCst);
        // Meanwhile the seal releases, flips and drains: nothing joined.
        let sealed = publish_hour(&owner, &mut output, 2, &|| {});
        assert_eq!(sealed.row().coverage, EventFeedCoverage::Complete);
        // The callback resumes: its validation moves it to the new phase.
        let slot = owner.validate_phase(stale);
        assert_eq!(slot, ((stale + 1) & 1) as usize);
        assert_eq!(
            owner.entering[(stale & 1) as usize].load(Ordering::SeqCst),
            0
        );
        // Declined by the next holder, it is recorded in the new phase.
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
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
        let owner = sealing_owner();
        let mut output = Snapshot::new(0);
        publish_hour(&owner, &mut output, 1, &|| {});
        let phase = owner.phase.load(Ordering::SeqCst);
        let (old, new) = ((phase & 1) as usize, ((phase + 1) & 1) as usize);
        let done = AtomicBool::new(false);
        let sealed = std::thread::scope(|scope| {
            // Sustained callback entries throughout.
            let stream = scope.spawn(|| {
                while !done.load(Ordering::SeqCst) {
                    let _ = owner.observe(Role::Script, |_| ());
                }
            });
            // An attempt of the old phase is paused across the release. After
            // the flip it joins the new phase and stays there, then records
            // its old decline.
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
            // The drain finished while the new phase still had an attempt.
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
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
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
                // A writer was lost while this lease held the store.
                assert_eq!(untaken_declines(), 1);
            })
            .unwrap();
        // The next entry takes it as missed.
        owner.observe(Role::Script, |_| ()).unwrap();
        assert_eq!(untaken_declines(), 0);
    }

    #[test]
    fn a_retained_owner_refuses_waiting_workers_and_skips_the_census() {
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
        let _restore = RestoreTestGlobals::new();
        assert_eq!(FAULT.load(Ordering::SeqCst), 0);
        let owner = Owner::new(0, Store::new(None).unwrap());
        owner.armed.store(true, Ordering::SeqCst);
        SHARED.store(Arc::as_ptr(&owner).cast_mut(), Ordering::SeqCst);
        // A callback escaped while holding the store; its guard retained
        // the owner.
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
        seal_growing(&mut snapshot, 1, false);
        assert_eq!(snapshot.boundary.failure, Some(WORKER_RETAINED));
        assert_eq!(
            snapshot.boundary.coverage,
            replay_data::event_feed_rows::EventFeedCoverage::Unavailable
        );
        // Like a lost drain, the next interval is named unavailable.
        assert!(!owner.armed.load(Ordering::SeqCst));
        assert_eq!(total_declines(&owner), 0);
    }

    #[test]
    fn escaped_native_flight_retains_the_event_feed_owner_without_a_ticket() {
        extern "system" fn original() {}
        let _guard = crate::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    /// An interval is complete only while both provenance forwarders are
    /// enabled: without them its state transfers or declaration causes went
    /// unobserved.
    #[test]
    fn a_disabled_forwarder_leaves_the_interval_partial() {
        use replay_data::event_feed_rows::EventFeedCoverage;
        let _guard = crate::hook::FAMILY_TEST.lock().unwrap();
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
            let return_pc = 0x140000000 + site.return_rva as u64;
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
