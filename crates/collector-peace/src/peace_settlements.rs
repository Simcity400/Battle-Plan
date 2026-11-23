
use crate::hook::Hook;
use engine_native_memory::Memory;
use engine_peace::{
    peace_settlement as native, peace_settlement::Action, peace_settlement::Snapshot,
    peace_settlement::Subject, peace_settlement::Transfer,
};
use std::{
    cell::UnsafeCell,
    ops::Range,
    ptr,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capacities {
    pub events: usize,
    pub countries: usize,
    pub actions: usize,
    pub transfers: usize,
    pub name_bytes: usize,
}

collector_primitives::capacity_max!(Capacities {
    events,
    countries,
    actions,
    transfers,
    name_bytes
});
impl Capacities {
    pub fn for_counts(countries: usize, states: usize) -> Self {
        Self {
            events: countries.saturating_mul(2),
            countries: countries.saturating_mul(4),
            actions: states
                .saturating_mul(4)
                .saturating_add(countries.saturating_mul(2)),
            transfers: states.saturating_mul(4),
            name_bytes: 0,
        }
    }
    pub fn contains(self, other: Self) -> bool {
        self.events >= other.events
            && self.countries >= other.countries
            && self.actions >= other.actions
            && self.transfers >= other.transfers
            && self.name_bytes >= other.name_bytes
    }
    pub fn merge(self, other: Self) -> Self {
        Self {
            events: self.events.max(other.events),
            countries: self.countries.max(other.countries),
            actions: self.actions.max(other.actions),
            transfers: self.transfers.max(other.transfers),
            name_bytes: self.name_bytes.max(other.name_bytes),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Event {
    pub sequence: u64,
    pub hour: Option<i32>,
    pub snapshot: Snapshot,
    pub winners: usize,
    pub losers: usize,
    pub actions: usize,
    pub transfers: usize,
    pub subjects: usize,
    pub name_bytes: usize,
}

pub struct Storage {
    pub events: Vec<Event>,
    pub winners: Vec<u32>,
    pub losers: Vec<u32>,
    pub actions: Vec<Action>,
    pub transfers: Vec<Transfer>,
    pub subjects: Vec<Subject>,
    pub name_bytes: Vec<u8>,
    pub used: [usize; 7],
}
fn buffer<T: Default>(count: usize) -> Option<Vec<T>> {
    engine_allocation::default_vec(count).ok()
}
impl Storage {
    pub fn with_capacities(capacity: Capacities) -> Self {
        use engine_allocation::{default_vec, owned_vec};
        Self {
            events: owned_vec(default_vec(capacity.events)),
            winners: owned_vec(default_vec(capacity.countries)),
            losers: owned_vec(default_vec(capacity.countries)),
            actions: owned_vec(default_vec(capacity.actions)),
            transfers: owned_vec(default_vec(capacity.transfers)),
            subjects: owned_vec(default_vec(capacity.countries)),
            name_bytes: owned_vec(default_vec(capacity.name_bytes)),
            used: [0; 7],
        }
    }
    pub fn new(capacity: Capacities) -> Option<Self> {
        Some(Self {
            events: buffer(capacity.events)?,
            winners: buffer(capacity.countries)?,
            losers: buffer(capacity.countries)?,
            actions: buffer(capacity.actions)?,
            transfers: buffer(capacity.transfers)?,
            subjects: buffer(capacity.countries)?,
            name_bytes: buffer(capacity.name_bytes)?,
            used: [0; 7],
        })
    }
    pub fn capacity(&self) -> Capacities {
        Capacities {
            events: self.events.len(),
            countries: self.winners.len(),
            actions: self.actions.len(),
            transfers: self.transfers.len(),
            name_bytes: self.name_bytes.len(),
        }
    }
    pub fn allocated_bytes(&self) -> usize {
        self.events.capacity() * size_of::<Event>()
            + (self.winners.capacity() + self.losers.capacity()) * 4
            + self.actions.capacity() * size_of::<Action>()
            + self.transfers.capacity() * size_of::<Transfer>()
            + self.subjects.capacity() * size_of::<Subject>()
            + self.name_bytes.capacity()
    }
    pub fn observe(
        &mut self,
        memory: &impl Memory,
        base: usize,
        conference: usize,
        hour: Option<i32>,
        sequence: u64,
        settled: bool,
    ) -> bool {
        use engine_native_memory::walk::Buffer::GrowingFrom;
        let [
            events,
            winners,
            losers,
            actions,
            transfers,
            subjects,
            name_bytes,
        ] = self.used;
        if events == self.events.len()
            && engine_allocation::advisory(|| {
                engine_allocation::resize_default(
                    &mut self.events,
                    events.saturating_mul(2).max(events + 1),
                )
            })
            .is_err()
        {
            if settled {
                LOST.fetch_add(1, Ordering::Relaxed);
            }
            return false;
        }
        let output = native::Output {
            winners: GrowingFrom(&mut self.winners, winners),
            losers: GrowingFrom(&mut self.losers, losers),
            actions: GrowingFrom(&mut self.actions, actions),
            transfers: GrowingFrom(&mut self.transfers, transfers),
            subjects: GrowingFrom(&mut self.subjects, subjects),
            name_bytes: GrowingFrom(&mut self.name_bytes, name_bytes),
        };
        let mut snapshot = engine_allocation::advisory(|| {
            if settled {
                native::copy(memory, base, conference, output)
            } else {
                native::copy_bids(memory, base, conference, output)
            }
        });
        if hour.is_none() && snapshot.failure.is_none() {
            snapshot.failure = Some("peace finalizer hour unreadable");
        }
        self.record(snapshot, hour, sequence);
        if settled {
            OMITTED.fetch_add(snapshot.omitted as u64, Ordering::Relaxed);
            if snapshot.failure.is_some() {
                FAILED.fetch_add(1, Ordering::Relaxed);
            }
        }
        true
    }
    pub fn observe_bids(
        &mut self,
        memory: &impl Memory,
        base: usize,
        conference: usize,
        hour: i32,
        sequence: u64,
    ) {
        use engine_native_memory::walk::Buffer::GrowingFrom;
        let [_, winners, losers, actions, transfers, subjects, name_bytes] = self.used;
        let snapshot = native::copy_bids(
            memory,
            base,
            conference,
            native::Output {
                winners: GrowingFrom(&mut self.winners, winners),
                losers: GrowingFrom(&mut self.losers, losers),
                actions: GrowingFrom(&mut self.actions, actions),
                transfers: GrowingFrom(&mut self.transfers, transfers),
                subjects: GrowingFrom(&mut self.subjects, subjects),
                name_bytes: GrowingFrom(&mut self.name_bytes, name_bytes),
            },
        );
        self.record(snapshot, Some(hour), sequence);
    }
    fn record(&mut self, snapshot: Snapshot, hour: Option<i32>, sequence: u64) {
        let [
            events,
            winners,
            losers,
            actions,
            transfers,
            subjects,
            name_bytes,
        ] = self.used;
        self.events[events] = Event {
            sequence,
            hour,
            snapshot,
            winners,
            losers,
            actions,
            transfers,
            subjects,
            name_bytes,
        };
        self.used = [
            events + 1,
            winners + snapshot.winners,
            losers + snapshot.losers,
            actions + snapshot.actions,
            transfers + snapshot.transfers,
            subjects + snapshot.subjects,
            name_bytes + snapshot.name_bytes,
        ];
    }
}

struct Log {
    capacity: Capacities,
    storage: UnsafeCell<Storage>,
}
unsafe impl Sync for Log {}
#[derive(Default)]
struct Swap {
    current: Option<Box<Log>>,
    next: Option<Box<Log>>,
    retired: Option<Box<Log>>,
}
static SWAP: Mutex<Swap> = Mutex::new(Swap {
    current: None,
    next: None,
    retired: None,
});
static LOG: AtomicPtr<Log> = AtomicPtr::new(ptr::null_mut());
static BUSY: AtomicBool = AtomicBool::new(false);
static ENABLED: AtomicBool = AtomicBool::new(false);
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static RETURN: AtomicUsize = AtomicUsize::new(0);
static BASE: AtomicUsize = AtomicUsize::new(0);
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static LOST: AtomicU64 = AtomicU64::new(0);
static LOST_DRAINS: AtomicU64 = AtomicU64::new(0);
static DRAIN_INVALIDATED: AtomicBool = AtomicBool::new(false);
static OMITTED: AtomicU64 = AtomicU64::new(0);
static FAILED: AtomicU64 = AtomicU64::new(0);
static UNHOOKED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
struct Lease;
impl Lease {
    fn try_enter() -> Option<Self> {
        BUSY.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| Self)
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::Release);
    }
}

pub fn presize(capacity: Capacities) {
    engine_allocation::advisory(|| presize_storage(capacity));
}

fn presize_storage(capacity: Capacities) {
    let Ok(mut swap) = SWAP.lock() else {
        return;
    };
    let current = swap.current.as_ref().map(|log| {
        log.capacity
    });
    let offered = swap.next.as_ref().map(|log| log.capacity).or(current);
    if offered.is_some_and(|offered| offered.contains(capacity)) {
        return;
    }
    let capacity = offered.map_or(capacity, |offered| offered.merge(capacity));
    if let Some(storage) = Storage::new(capacity)
        && let Ok(next) = engine_allocation::boxed(Log {
            capacity,
            storage: UnsafeCell::new(storage),
        })
    {
        swap.next = Some(next);
    }
}
pub fn install_offered() {
    let Some(_lease) = Lease::try_enter() else {
        return;
    };
    let Ok(mut swap) = SWAP.try_lock() else {
        return;
    };
    if swap.retired.is_some() || ACTIVE.load(Ordering::SeqCst) != 0 {
        return;
    }
    if let Some(log) = swap.current.as_ref()
        && unsafe { (&*log.storage.get()).used[0] } != 0
    {
        return;
    }
    if let Some(mut next) = swap.next.take() {
        LOG.store(&mut *next, Ordering::Release);
        swap.retired = swap.current.replace(next);
    }
}
pub fn collect() {
    if let Ok(mut swap) = SWAP.lock() {
        drop(swap.retired.take());
    }
}

fn sizing_array<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    state: usize,
    offset: usize,
) -> Option<engine_native_memory::cpdx_array::Array> {
    let array = state
        .checked_add(offset)
        .and_then(|at| engine_native_memory::cpdx_array::Array::read(memory, at, size_of::<T>()))?;
    if let Some(last) = array.count.checked_sub(1) {
        engine_native_memory::read::<T>(memory, array.element(last, size_of::<T>())?)?;
    }
    Some(array)
}

pub fn capacity_for_world(memory: &impl Memory, base: usize) -> Capacities {
    let Some(state) = engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
        .and_then(|at| engine_native_memory::read::<u64>(memory, at))
    else {
        return Capacities::default();
    };
    let state = state as usize;
    let countries = [
        sizing_array::<u64>(memory, state, engine_native::cgame_state::COUNTRIES),
        sizing_array::<u32>(memory, state, engine_native::cgame_state::COUNTRY_LINKS),
        sizing_array::<[u8; engine_native_memory::msvc_string::STRIDE]>(
            memory,
            state,
            engine_native::cgame_state::COUNTRY_TAGS,
        ),
    ]
    .into_iter()
    .flatten()
    .map(|array| array.count as usize)
    .max()
    .unwrap_or(0);
    let states = sizing_array::<u64>(memory, state, engine_native::cgame_state::STATES)
        .map_or(0, |array| array.count as usize);
    let mut capacity = Capacities::for_counts(countries, states);
    if let Some(demand) = resident_capacity(memory, base, state) {
        capacity = capacity.merge(demand);
    }
    capacity
}

fn resident_capacity(memory: &impl Memory, base: usize, state: usize) -> Option<Capacities> {
    let roster = sizing_array::<u64>(
        memory,
        state,
        engine_native::cgame_state::PEACE_CONFERENCES_DATA,
    )?;
    let mut capacity = Capacities::default();
    for index in 0..roster.count {
        let pointer = roster
            .element(index, 8)
            .and_then(|at| engine_native_memory::read::<u64>(memory, at))?;
        let demand = native::demand(memory, base, pointer as usize);
        if demand.failure.is_some() {
            continue;
        }
        capacity.events = capacity.events.saturating_add(1);
        capacity.countries = capacity
            .countries
            .saturating_add(demand.winners.max(demand.losers).max(demand.subjects));
        capacity.actions = capacity.actions.saturating_add(demand.actions);
        capacity.transfers = capacity.transfers.saturating_add(demand.transfers);
        capacity.name_bytes = capacity.name_bytes.saturating_add(demand.name_bytes);
    }
    Some(capacity)
}

pub fn drain(output: &mut Storage, continuous: bool) -> Result<u64, &'static str> {
    drain_mode(output, continuous)
}
pub fn lost_drain() {
    LOST_DRAINS.fetch_add(1, Ordering::Relaxed);
    DRAIN_INVALIDATED.store(true, Ordering::Release);
}
fn drain_mode(output: &mut Storage, continuous: bool) -> Result<u64, &'static str> {
    output.used = [0; 7];
    let Some(_lease) = Lease::try_enter() else {
        return Err(replay_data::rows::PEACE_DRAIN_BUSY);
    };
    let Some(log) = (unsafe { LOG.load(Ordering::Acquire).as_ref() }) else {
        return Err("peace finalizer log unavailable");
    };
    if !ENABLED.load(Ordering::Acquire) {
        return Err("peace finalizer hook is not enabled");
    }
    let storage = unsafe { &mut *log.storage.get() };
    let invalidated = DRAIN_INVALIDATED.swap(false, Ordering::AcqRel);
    let continuous = continuous && !invalidated;
    if continuous {
        for (target, count) in [
            (
                engine_native_memory::walk::Buffer::Growing(&mut output.winners),
                storage.used[1],
            ),
            (
                engine_native_memory::walk::Buffer::Growing(&mut output.losers),
                storage.used[2],
            ),
        ] {
            let mut target = target;
            target.ensure(count)?;
        }
        engine_native_memory::walk::Buffer::Growing(&mut output.events).ensure(storage.used[0])?;
        engine_native_memory::walk::Buffer::Growing(&mut output.actions).ensure(storage.used[3])?;
        engine_native_memory::walk::Buffer::Growing(&mut output.transfers).ensure(storage.used[4])?;
        engine_native_memory::walk::Buffer::Growing(&mut output.subjects).ensure(storage.used[5])?;
        engine_native_memory::walk::Buffer::Growing(&mut output.name_bytes).ensure(storage.used[6])?;
    }
    let mut lost = LOST.swap(0, Ordering::Relaxed);
    if !continuous {
        lost = lost.saturating_add(storage.used[0] as u64);
    } else {
        for event in &storage.events[..storage.used[0]] {
            let [
                events,
                winners,
                losers,
                actions,
                transfers,
                subjects,
                name_bytes,
            ] = output.used;
            let s = event.snapshot;
            if events == output.events.len()
                || winners + s.winners > output.winners.len()
                || losers + s.losers > output.losers.len()
                || actions + s.actions > output.actions.len()
                || transfers + s.transfers > output.transfers.len()
                || subjects + s.subjects > output.subjects.len()
                || name_bytes + s.name_bytes > output.name_bytes.len()
            {
                lost += 1;
                continue;
            }
            output.winners[winners..winners + s.winners]
                .copy_from_slice(&storage.winners[event.winners..event.winners + s.winners]);
            output.losers[losers..losers + s.losers]
                .copy_from_slice(&storage.losers[event.losers..event.losers + s.losers]);
            output.actions[actions..actions + s.actions]
                .copy_from_slice(&storage.actions[event.actions..event.actions + s.actions]);
            output.transfers[transfers..transfers + s.transfers].copy_from_slice(
                &storage.transfers[event.transfers..event.transfers + s.transfers],
            );
            output.subjects[subjects..subjects + s.subjects]
                .copy_from_slice(&storage.subjects[event.subjects..event.subjects + s.subjects]);
            output.name_bytes[name_bytes..name_bytes + s.name_bytes].copy_from_slice(
                &storage.name_bytes[event.name_bytes..event.name_bytes + s.name_bytes],
            );
            output.events[events] = Event {
                winners,
                losers,
                actions,
                transfers,
                subjects,
                name_bytes,
                ..*event
            };
            output.used = [
                events + 1,
                winners + s.winners,
                losers + s.losers,
                actions + s.actions,
                transfers + s.transfers,
                subjects + s.subjects,
                name_bytes + s.name_bytes,
            ];
        }
    }
    storage.used = [0; 7];
    Ok(lost)
}

unsafe extern "system" fn observe(conference: usize) {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::PeaceSettlements);
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    if ENABLED.load(Ordering::Acquire) {
        if let Some(_lease) = Lease::try_enter() {
            if let Some(log) = unsafe { LOG.load(Ordering::Acquire).as_ref() } {
                let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
                let base = BASE.load(Ordering::Relaxed);
                let hour = engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
                    .and_then(|at| engine_native_memory::read::<u64>(&memory, at))
                    .and_then(|state| {
                        (state as usize).checked_add(engine_native::cgame_state::CURRENT_HOUR)
                    })
                    .and_then(|at| engine_native_memory::read(&memory, at));
                unsafe { &mut *log.storage.get() }
                    .observe(&memory, base, conference, hour, sequence, true);
            } else {
                LOST.fetch_add(1, Ordering::Relaxed);
            }
        } else {
            LOST.fetch_add(1, Ordering::Relaxed);
        }
    }
    ACTIVE.fetch_sub(1, Ordering::SeqCst);
}

core::arch::global_asm!(
    ".text",
    ".globl peace_finalizer_entry",
    ".seh_proc peace_finalizer_entry",
    "peace_finalizer_entry:",
    "push qword ptr [rip+{unwind_return}]",
    "pushfq", ".seh_stackalloc 8",
    "push rax", ".seh_pushreg rax",
    "push rcx", ".seh_pushreg rcx",
    "push rdx", ".seh_pushreg rdx",
    "push r8", ".seh_pushreg r8",
    "push r9", ".seh_pushreg r9",
    "push r10", ".seh_pushreg r10",
    "push r11", ".seh_pushreg r11",
    "sub rsp, 136", ".seh_stackalloc 136",
    "movdqu [rsp+32], xmm0", ".seh_savexmm xmm0, 32",
    "movdqu [rsp+48], xmm1", ".seh_savexmm xmm1, 48",
    "movdqu [rsp+64], xmm2", ".seh_savexmm xmm2, 64",
    "movdqu [rsp+80], xmm3", ".seh_savexmm xmm3, 80",
    "movdqu [rsp+96], xmm4", ".seh_savexmm xmm4, 96",
    "movdqu [rsp+112], xmm5", ".seh_savexmm xmm5, 112",
    ".seh_endprologue",
    "cld",
    "mov rcx, r13", "call {observe}",
    ".globl peace_finalizer_body",
    "peace_finalizer_body:",
    "movdqu xmm0, [rsp+32]", "movdqu xmm1, [rsp+48]", "movdqu xmm2, [rsp+64]",
    "movdqu xmm3, [rsp+80]", "movdqu xmm4, [rsp+96]", "movdqu xmm5, [rsp+112]",
    "add rsp, 136", "pop r11", "pop r10", "pop r9", "pop r8", "pop rdx", "pop rcx", "pop rax", "popfq",
    "lea rsp, [rsp+8]",
    "jmp qword ptr [rip+{original}]",
    ".seh_endproc",
    observe = sym observe, original = sym ORIGINAL, unwind_return = sym RETURN,
);
unsafe extern "system" {
    fn peace_finalizer_entry();
}

pub struct Hooks {
    hook: Option<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family, as for line_output.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize, capacity: Capacities) -> Self {
        presize(capacity);
        install_offered();
        let attempt: Result<Hook, String> = (|| {
            let located = engine_sites::hook(base, engine_peace::peace_finalizer_site::RVA)
                .and_then(|code| Ok((code, code.sentinel::<16>()?)))
                .and_then(|located| {
                    engine_sites::hook(base, engine_peace::peace_finalizer_site::RETURN_RVA)
                        .map(|resume| (located, resume.address))
                });
            let ((site, sentinel), resume) = located.map_err(|cause| {
                crate::hook::installation::refused(
                    crate::hook::installation::Kind::Sentinel,
                    None,
                    cause,
                );
                cause.to_owned()
            })?;
            if LOG.load(Ordering::Acquire).is_null() {
                return Err("peace finalizer allocation failed".into());
            }
            let hook = unsafe {
                Hook::new(
                    site.address,
                    peace_finalizer_entry as *const () as usize,
                    sentinel,
                )
            }?;
            ORIGINAL.store(hook.trampoline(), Ordering::Release);
            RETURN.store(resume, Ordering::Release);
            BASE.store(base, Ordering::Relaxed);
            Ok(hook)
        })();
        Self {
            hook: match attempt {
                Ok(hook) => Some(hook),
                Err(reason) => {
                    crate::hook::installation::refused(
                        crate::hook::installation::Kind::Construction,
                        None,
                        &reason,
                    );
                    let _ = UNHOOKED.set(reason);
                    None
                }
            },
        }
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            ENABLED.store(false, Ordering::Release);
        }
        if let Some(hook) = &mut self.hook {
            let changed = hook.set_enabled(enabled).inspect_err(|reason| {
                let _ = UNHOOKED.set(reason.clone());
            })?;
            if !changed {
                return Ok(false);
            }
            ENABLED.store(enabled, Ordering::Release);
        }
        Ok(true)
    }
    pub fn executable_ranges(&self) -> Vec<Range<usize>> {
        self.hook.iter().flat_map(Hook::executable_ranges).collect()
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        LOG.store(ptr::null_mut(), Ordering::Release);
        *SWAP
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Swap::default();
    }
}
pub fn status() -> impl serde::Serialize {
    crate::status::status!({"installed": ENABLED.load(Ordering::Acquire),
        "prepared": ORIGINAL.load(Ordering::Acquire) != 0,
        "enabled": ENABLED.load(Ordering::Acquire), "unhooked": UNHOOKED.get(),
        "observed": SEQUENCE.load(Ordering::Relaxed), "pending_lost_conferences": LOST.load(Ordering::Relaxed),
        "lost_worker_drains": LOST_DRAINS.load(Ordering::Relaxed),
        "omitted_operands": OMITTED.load(Ordering::Relaxed), "failed_conferences": FAILED.load(Ordering::Relaxed)})
}

#[cfg(any(test, feature = "test-support"))]
static TEST_GLOBALS: Mutex<()> = Mutex::new(());
#[cfg(any(test, feature = "test-support"))]
pub fn test_globals() -> std::sync::MutexGuard<'static, ()> {
    TEST_GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peace_settlement_tests::fixture::Fixture;

    #[test]
    fn peace_storage_allocation_failure_is_returned() {
        let _globals = test_globals();
        static ALLOCATION_REFUSED: AtomicBool = AtomicBool::new(false);
        fn allocation_failed() {
            ALLOCATION_REFUSED.store(true, Ordering::Relaxed);
        }
        assert!(!engine_allocation::capture_active());
        ALLOCATION_REFUSED.store(false, Ordering::Relaxed);
        engine_allocation::set_hooks(None, Some(allocation_failed));
        let result = crate::test_support::with_allocations_denied(|| {
            Storage::new(Capacities::for_counts(1, 1))
        });
        engine_allocation::set_hooks(None, None);
        assert!(result.is_none());
        assert!(ALLOCATION_REFUSED.load(Ordering::Relaxed));
    }
    #[test]
    fn peace_status_serializes_without_allocating() {
        let _globals = test_globals();
        let mut bytes = Vec::with_capacity(512);
        crate::test_support::with_allocations_denied(|| {
            serde_json::to_writer(&mut bytes, &status())
        })
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for field in [
            "installed",
            "prepared",
            "enabled",
            "unhooked",
            "observed",
            "pending_lost_conferences",
            "omitted_operands",
            "failed_conferences",
        ] {
            assert!(
                value.get(field).is_some(),
                "missing peace status field {field}"
            );
        }
    }
    core::arch::global_asm!(
        ".text",
        ".globl peace_test_call",
        ".seh_proc peace_test_call",
        "peace_test_call:",
        "push r13",
        ".seh_pushreg r13",
        "sub rsp, 40",
        ".seh_stackalloc 40",
        ".seh_endprologue",
        "mov r13, rcx",
        "call peace_finalizer_entry",
        "add rsp, 40",
        "pop r13",
        "ret",
        ".seh_endproc",
        ".globl peace_test_resume",
        "peace_test_resume:",
        "ret",
    );
    unsafe extern "system" {
        fn peace_test_call(conference: usize);
        fn peace_test_resume();
    }

    #[test]
    fn bridge_unwinds_to_site_with_native_stack_and_registers() {
        use windows_sys::Win32::System::Diagnostics::Debug::{
            CONTEXT, RtlLookupFunctionEntry, RtlVirtualUnwind,
        };
        unsafe extern "system" {
            fn peace_finalizer_body();
        }
        let entry = peace_finalizer_entry as *const () as usize;
        let first = unsafe { std::slice::from_raw_parts(entry as *const u8, 6) };
        assert_eq!(
            &first[..2],
            &[0xff, 0x35],
            "first instruction must push the synthetic return"
        );
        let displacement = i32::from_le_bytes(first[2..].try_into().unwrap());
        assert_eq!(
            (entry + 6).wrapping_add_signed(displacement as isize),
            &RETURN as *const _ as usize
        );
        let pc = peace_finalizer_body as *const () as u64;
        let mut image_base = 0;
        let function = unsafe { RtlLookupFunctionEntry(pc, &mut image_base, ptr::null_mut()) };
        assert!(
            !function.is_null(),
            "bridge must have a runtime function record"
        );
        assert_eq!(
            image_base + unsafe { (*function).BeginAddress } as u64,
            peace_finalizer_entry as *const () as u64
        );
        #[repr(align(16))]
        struct Stack([u64; 28]);
        let mut stack = Stack([0; 28]);
        let registers = [11, 10, 9, 8, 3, 2, 1];
        stack.0[17..24].copy_from_slice(&registers);
        stack.0[24] = 0x202;
        let site = 0x140000000 + engine_peace::peace_finalizer_site::RETURN_RVA.0 as u64;
        stack.0[25] = site;
        let original_rsp = stack.0.as_ptr() as u64 + 208;
        let mut context: CONTEXT = unsafe { std::mem::zeroed() };
        context.Rip = pc;
        context.Rsp = stack.0.as_ptr() as u64;
        context.R13 = 0xabcdef;
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
        assert_eq!(context.Rip, site);
        assert_eq!(context.Rsp, original_rsp);
        assert_eq!(
            [
                context.R11,
                context.R10,
                context.R9,
                context.R8,
                context.Rdx,
                context.Rcx,
                context.Rax
            ],
            registers
        );
        assert_eq!(context.R13, 0xabcdef);
    }

    #[test]
    fn presize_reuses_an_offer_before_current_is_installed() {
        let _guard = test_globals();
        let capacity = Capacities::for_counts(50, 100);
        presize(capacity);
        assert_eq!(
            SWAP.lock().unwrap().next.as_ref().unwrap().capacity,
            capacity
        );
        let first = SWAP.lock().unwrap().next.as_ref().unwrap().as_ref() as *const Log;
        presize(capacity);
        let second = SWAP.lock().unwrap().next.as_ref().unwrap().as_ref() as *const Log;
        assert_eq!(first, second);
        *SWAP.lock().unwrap() = Swap::default();
    }

    #[test]
    fn status_distinguishes_a_prepared_hook_from_enabled_capture() {
        let _guard = test_globals();
        ORIGINAL.store(1, Ordering::Release);
        ENABLED.store(false, Ordering::Release);
        let value = serde_json::to_value(status()).unwrap();
        assert_eq!(value["prepared"], true);
        assert_eq!(value["installed"], false);
        assert_eq!(value["enabled"], false);
        ORIGINAL.store(0, Ordering::Release);
    }

    #[test]
    fn refused_optional_hook_leaves_capture_disabled() {
        let _guard = test_globals();
        let mut hooks = Hooks { hook: None };
        assert_eq!(hooks.set_enabled(true), Ok(true));
        assert_eq!(serde_json::to_value(status()).unwrap()["enabled"], false);
    }

    #[test]
    fn busy_drain_preserves_the_event_for_the_next_boundary() {
        let _guard = test_globals();
        presize(Capacities::for_counts(1, 1));
        install_offered();
        collect();
        ENABLED.store(true, Ordering::Release);
        let f = Fixture::new(0, 1);
        let log = unsafe { &*LOG.load(Ordering::Acquire) };
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &f.bytes,
        };
        unsafe { &mut *log.storage.get() }.observe(&memory, 0, 0x100, Some(91), 0, true);
        let mut output = Storage::new(Capacities::for_counts(1, 1)).unwrap();
        let lease = Lease::try_enter().unwrap();
        assert_eq!(
            drain(&mut output, true),
            Err(replay_data::rows::PEACE_DRAIN_BUSY)
        );
        assert_eq!(unsafe { &*log.storage.get() }.used[0], 1);
        drop(lease);
        drain(&mut output, true).unwrap();
        assert_eq!(output.events[0].hour, Some(91));
        assert_eq!(output.used[0], 1);
        ENABLED.store(false, Ordering::Release);
        LOG.store(ptr::null_mut(), Ordering::Release);
        *SWAP.lock().unwrap() = Swap::default();
    }

    #[test]
    #[ignore = "complete bridge and guarded observer cost on synthetic memory; no game"]
    fn peace_observer_cost() {
        let _guard = test_globals();
        for count in [50, 500, 5000] {
            let mut fixture = Fixture::new(count, count);
            fixture.populate_transfers(count);
            let (base, conference) = fixture.rebase_image(91);
            presize(Capacities::for_counts(count, count));
            install_offered();
            collect();
            BASE.store(base, Ordering::Relaxed);
            RETURN.store(
                base + engine_peace::peace_finalizer_site::RETURN_RVA.0,
                Ordering::Release,
            );
            ORIGINAL.store(peace_test_resume as *const () as usize, Ordering::Release);
            ENABLED.store(true, Ordering::Release);
            let mut samples = Vec::with_capacity(100);
            for _ in 0..100 {
                let log = unsafe { &*LOG.load(Ordering::Acquire) };
                unsafe { &mut *log.storage.get() }.used = [0; 7];
                let started = std::time::Instant::now();
                unsafe { peace_test_call(std::hint::black_box(conference)) };
                samples.push(started.elapsed().as_nanos());
                let event = unsafe { &*log.storage.get() }.events[0];
                assert_eq!(event.hour, Some(91));
                assert_eq!(event.snapshot.failure, None);
                assert_eq!(event.snapshot.omitted, 0);
                assert_eq!(event.snapshot.actions, count);
                assert_eq!(event.snapshot.transfers, count);
            }
            samples.sort_unstable();
            let bytes = unsafe { &*(*LOG.load(Ordering::Acquire)).storage.get() }.allocated_bytes();
            println!(
                "guarded synthetic bridge countries={count} actions={count} transfers={count}: p50={}ns p95={}ns max={}ns arena={}bytes",
                samples[49], samples[94], samples[99], bytes
            );
            ENABLED.store(false, Ordering::Release);
            ORIGINAL.store(0, Ordering::Release);
            RETURN.store(0, Ordering::Release);
            LOG.store(ptr::null_mut(), Ordering::Release);
            *SWAP.lock().unwrap() = Swap::default();
        }
    }
    #[test]
    fn peace_settlement_capacity_uses_resident_demand_without_population_caps() {
        let count = 262_145;
        let mut fixture = Fixture::new(65_537, count);
        fixture.populate_transfers(count);
        let roster = fixture.bytes.len();
        fixture.bytes.resize(roster + 8, 0);
        fixture.array(
            0x10000 + engine_native::cgame_state::PEACE_CONFERENCES_DATA,
            roster,
            1,
            8,
        );
        fixture.u64(roster, 0x100);
        let memory = fixture.memory();
        let capacity = resident_capacity(&memory, 0, 0x10000).unwrap();
        assert_eq!(
            capacity,
            Capacities {
                events: 1,
                countries: 65_537,
                actions: count,
                transfers: count,
                name_bytes: 0,
            }
        );
        let headroom = Capacities::for_counts(65_537, count);
        assert_eq!(headroom.events, 131_074);
        assert_eq!(headroom.countries, 262_148);
        assert!(headroom.actions > count);
        assert_eq!(headroom.transfers, count * 4);
    }

    #[test]
    fn peace_settlement_failed_demand_contributes_no_capacity() {
        let mut fixture = Fixture::new(3, 2);
        fixture.bytes[0x100 + engine_peace::peace_conference::FINALIZED] = 0;
        fixture.u32(0x908, 99);
        let roster = fixture.bytes.len();
        fixture.bytes.resize(roster + 8, 0);
        fixture.array(
            0x10000 + engine_native::cgame_state::PEACE_CONFERENCES_DATA,
            roster,
            1,
            8,
        );
        fixture.u64(roster, 0x100);
        let memory = fixture.memory();
        let demand = native::demand(&memory, 0, 0x100);
        assert_eq!(demand.winners, 3);
        assert_eq!(demand.failure, Some("peace bidding turn country mismatch"));
        assert_eq!(
            resident_capacity(&memory, 0, 0x10000),
            Some(Capacities::default())
        );
        let mut storage = Storage::new(Capacities::for_counts(3, 2)).unwrap();
        storage.observe(&memory, 0, 0x100, Some(91), 0, false);
        assert_eq!(storage.events[0].snapshot.failure, demand.failure);
    }

    #[test]
    fn peace_settlement_torn_demand_never_attempts_arena_growth() {
        let _globals = test_globals();
        static ALLOCATION_REFUSED: AtomicBool = AtomicBool::new(false);
        fn allocation_failed() {
            ALLOCATION_REFUSED.store(true, Ordering::Relaxed);
        }
        let mut fixture = Fixture::new(1, 0);
        fixture.array(
            0x100 + engine_peace::peace_conference::WINNERS,
            0x20000,
            i32::MAX as usize,
            4,
        );
        let roster = fixture.bytes.len();
        fixture.bytes.resize(roster + 8, 0);
        fixture.array(
            0x10000 + engine_native::cgame_state::PEACE_CONFERENCES_DATA,
            roster,
            1,
            8,
        );
        fixture.u64(roster, 0x100);
        let memory = fixture.memory();
        presize(Capacities::default());
        let offered = SWAP.lock().unwrap().next.as_ref().unwrap().as_ref() as *const Log;
        ALLOCATION_REFUSED.store(false, Ordering::Relaxed);
        engine_allocation::set_hooks(None, Some(allocation_failed));
        let capacity = crate::test_support::with_allocations_denied(|| {
            let capacity = resident_capacity(&memory, 0, 0x10000).unwrap();
            presize(capacity);
            capacity
        });
        engine_allocation::set_hooks(None, None);
        let unchanged = SWAP.lock().unwrap().next.as_ref().unwrap().as_ref() as *const Log;
        *SWAP.lock().unwrap() = Swap::default();
        assert_eq!(capacity, Capacities::default());
        assert_eq!(offered, unchanged);
        assert!(!ALLOCATION_REFUSED.load(Ordering::Relaxed));
    }

    #[test]
    fn peace_settlement_unbacked_world_rosters_do_not_size_storage() {
        let mut fixture = Fixture::new(1, 0);
        for (offset, width) in [
            (engine_native::cgame_state::COUNTRIES, 8),
            (engine_native::cgame_state::COUNTRY_LINKS, 4),
            (
                engine_native::cgame_state::COUNTRY_TAGS,
                engine_native_memory::msvc_string::STRIDE,
            ),
            (engine_native::cgame_state::STATES, 8),
            (engine_native::cgame_state::PEACE_CONFERENCES_DATA, 8),
        ] {
            fixture.array(0x10000 + offset, 0x20000, i32::MAX as usize, width);
        }
        assert_eq!(
            capacity_for_world(&fixture.memory(), 0),
            Capacities::default()
        );
    }
}
