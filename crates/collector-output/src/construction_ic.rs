pub use crate::output_log::Drained;
use crate::{
    opinion_changes::Padded,
    output_hooks,
    output_log::{DrainCounters, Log, Reserved, Spill},
};
use engine_native_memory::Memory;
use engine_sites::Rva;
use engine_production::{
    construction_ic_sites::BUILDING_COMPLETE, construction_ic_sites::BUILDING_RESET,
    construction_ic_sites::BUILDING_SPEED, construction_ic_sites::BUILDING_SPEND_RETURN,
    construction_ic_sites::COMPLETE_RETURN, construction_ic_sites::LINKED_COMPLETE_RETURN,
    construction_ic_sites::RAILWAY_ADVANCE, construction_ic_sites::RAILWAY_COMPLETE,
    construction_ic_sites::RAILWAY_PROGRESS, construction_ic_sites::REMOVE_GENERAL_LINE,
    construction_ic_sites::Site,
};
use std::{
    cell::Cell,
    mem::ManuallyDrop,
    ops::Range,
    ptr,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicPtr, AtomicU16, AtomicU64, AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct Slot {
    line: AtomicU64,
    identity: [AtomicU64; 4],
    partner: AtomicU64,
    amount: AtomicU64,
    bank: AtomicU64,
    flags: AtomicU16,
}

struct Storage {
    slots: Vec<Slot>,
}
impl Storage {
    fn new(capacity: usize) -> Option<Box<Self>> {
        let mut slots = engine_allocation::vec(capacity).ok()?;
        slots.resize_with(capacity, Slot::default);
        engine_allocation::boxed(Self { slots }).ok()
    }
}

#[derive(Default)]
struct LogSwap {
    current: Option<Box<Storage>>,
    next: Option<Box<Storage>>,
    retired: Option<Box<Storage>>,
}
static LOG_SWAP: Mutex<LogSwap> = Mutex::new(LogSwap {
    current: None,
    next: None,
    retired: None,
});
static LOG: AtomicPtr<Storage> = AtomicPtr::new(ptr::null_mut());
static SWAPS: AtomicU64 = AtomicU64::new(0);
static FAILED_OFFERS: AtomicU64 = AtomicU64::new(0);
static SPILLED: AtomicU64 = AtomicU64::new(0);
static OBSERVATION: Log = Log::new(0);
static SPILL: Spill<Entry> = Spill::new();
static CURSOR: &Padded<AtomicU64> = &OBSERVATION.cursor;
static ENABLED: &AtomicBool = &OBSERVATION.enabled;
pub fn active() -> usize {
    OBSERVATION.active()
}
static INSTALLED: [AtomicBool; 7] = [const { AtomicBool::new(false) }; 7];
static ORIGINAL: [AtomicUsize; 7] = [const { AtomicUsize::new(0) }; 7];
static BASE: AtomicUsize = AtomicUsize::new(0);
static RETURNS: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];
const RETURN_KEYS: [(Rva, &str); 3] = [
    (BUILDING_SPEND_RETURN, "construction spend caller withheld"),
    (COMPLETE_RETURN, "construction completion caller withheld"),
    (LINKED_COMPLETE_RETURN, "construction linked completion caller withheld"),
];

fn bind_returns(base: usize) -> Result<(), &'static str> {
    for (slot, (key, withheld)) in RETURNS.iter().zip(RETURN_KEYS) {
        let address = engine_sites::resolve(base, key).map_err(|_| withheld)?;
        slot.store(address, Ordering::Relaxed);
    }
    Ok(())
}
static CONSUMED: &AtomicU64 = &OBSERVATION.consumed;
static REFUSED: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
static REFUSAL_PENDING: AtomicBool = AtomicBool::new(false);
static UNHOOKED: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
static COUNTERS: [AtomicU64; 13] = [const { AtomicU64::new(0) }; 13];

pub fn capacity() -> usize {
    OBSERVATION.capacity.load(Ordering::Acquire)
}

pub(crate) fn presize(events: usize) {
    engine_allocation::advisory(|| presize_storage(events));
}

fn presize_storage(events: usize) {
    let Some(capacity) = events
        .checked_mul(2)
        .map(|native| native.max(OBSERVATION.required.load(Ordering::Acquire)))
    else {
        let _ = crate::allocation::oom();
        FAILED_OFFERS.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Ok(mut swap) = LOG_SWAP.lock() else {
        return;
    };
    let offered = swap.next.as_ref().map_or(0, |storage| storage.slots.len());
    if (swap.current.is_some() && capacity <= self::capacity())
        || (swap.next.is_some() && capacity <= offered)
    {
        return;
    }
    if let Some(next) = Storage::new(capacity) {
        swap.next = Some(next);
    } else {
        FAILED_OFFERS.fetch_add(1, Ordering::Relaxed);
    }
}

/// # Safety
/// Caller must hold exclusive pre-publication attach ownership (before enabling hooks), run on
/// the admitted post-drain observer boundary under the capture gate, or hold `test_lock()` in an
/// isolated test with hooks quiesced as the sole log-swap consumer.
pub unsafe fn install_offered() {
    let Ok(mut swap) = LOG_SWAP.try_lock() else {
        return;
    };
    if swap.retired.is_some()
        || active() != 0
        || CURSOR.load(Ordering::SeqCst) != CONSUMED.load(Ordering::Relaxed)
    {
        return;
    }
    let Some(mut next) = swap.next.take() else {
        return;
    };
    let capacity = next.slots.len();
    let pointer = &raw mut *next;
    swap.retired = swap.current.replace(next);
    LOG.store(pointer, Ordering::SeqCst);
    OBSERVATION.capacity.store(capacity, Ordering::Release);
    if swap.retired.is_some() {
        SWAPS.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn collect() {
    if let Ok(mut swap) = LOG_SWAP.lock() {
        drop(swap.retired.take());
    }
}

fn log() -> Option<&'static Storage> {
    // SAFETY: native writers hold OBSERVATION.enter(); a test that appends on the draining thread
    // holds `test_lock` and drains after. The observer boundary's job is the sole drain,
    // and it finishes with this log before it installs the replacement; the capture gate keeps
    // command-batch writers out through that sequence. Teardown frees the log only once the hooks
    // are removed and no writer is active.
    unsafe { LOG.load(Ordering::SeqCst).as_ref() }
}

const RAILWAY: u16 = 1;
const REPAIR: u16 = 2;
const INSTANT: u16 = 4;
const LINKED: u16 = 8;
const CARRY: u16 = 16;
const COMPLETE: u16 = 32;
const CANCELLED: u16 = 64;
const RESET: u16 = 128;
const ROUTE_FIRST: u16 = 256;
const ROUTE_LAST: u16 = 512;
const HAS_IDENTITY: u16 = 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub line: u64,
    pub identity: Option<engine_production::construction_ic_sites::Identity>,
    pub carry: bool,
    pub amount: i64,
    pub bank: i64,
    pub railway: bool,
    pub repair: bool,
    pub instant: bool,
    pub partner: Option<u64>,
    pub completion: Option<replay_data::rows::ConstructionCompletion>,
    pub linked_completion: bool,
    pub count_reset: Option<i32>,
}

thread_local! {
    static RAILWAY_LINE: Cell<(usize, u32)> = const { Cell::new((0, 0)) };
}

/// # Safety
/// Caller must own the active session's capture or publication failure path (or hold `test_lock()`
/// in an isolated test) when invalidating construction-IC observation.
pub unsafe fn invalidate() {
    OBSERVATION.invalidate();
}

fn observe(line: usize, amount: i64, railway: bool, carry: bool) {
    let base = BASE.load(Ordering::Relaxed);
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let operands =
        match engine_production::construction_ic_sites::operands(&memory, base, line, railway) {
            Ok(operands) => operands,
            Err(reason) => {
                refuse(reason);
                return;
            }
        };
    let amount = if railway { amount } else { operands.speed };
    let cheat = if railway {
        let instant = match operands.identity {
            Some(identity) => engine_production::construction_ic_sites::railway_instant_owned(
                &memory,
                base,
                identity.country,
            ),
            None => engine_production::construction_ic_sites::railway_instant(&memory, base, line),
        };
        match instant {
            Ok(cheat) => cheat,
            Err(reason) => {
                refuse(reason);
                return;
            }
        }
    } else {
        operands.instant
    };
    if railway
        && cheat
        && amount != engine_production::construction_ic_sites::RAILWAY_INSTANT_AMOUNT
    {
        refuse("construction output railway instant amount unsupported");
        return;
    }
    let unwithdrawn = cheat && !railway && operands.repair;
    let instant = cheat && !unwithdrawn;
    if (railway && amount < 0)
        || operands.bank < 0
        || (!carry && !instant && amount >= 0 && operands.bank > amount)
    {
        refuse("construction output amount or CIC bank draw unsupported");
        return;
    }
    let amount = if instant && !railway {
        amount.max(operands.cost)
    } else if unwithdrawn {
        amount.wrapping_sub(operands.bank)
    } else {
        amount
    };
    append(Entry {
        line: operands.line,
        identity: operands.identity,
        amount,
        bank: if carry || instant || unwithdrawn {
            0
        } else {
            operands.bank
        },
        railway,
        repair: operands.repair,
        instant,
        partner: operands.partner,
        carry,
        completion: None,
        linked_completion: false,
        count_reset: None,
    });
}

fn refuse(reason: &'static str) {
    COUNTERS[11].fetch_add(1, Ordering::Relaxed);
    let _ = REFUSED.set(reason);
    REFUSAL_PENDING.store(true, Ordering::SeqCst);
    OBSERVATION.invalidate();
}

fn observe_building(line: usize, caller: usize) {
    if caller != RETURNS[0].load(Ordering::Relaxed) {
        COUNTERS[1].fetch_add(1, Ordering::Relaxed);
        return;
    }
    COUNTERS[0].fetch_add(1, Ordering::Relaxed);
    observe(line, 0, false, false);
}

fn observe_completion(line: usize, caller: usize, result: u8, railway: bool) {
    let base = BASE.load(Ordering::Relaxed);
    if result == 0 || caller != RETURNS[1].load(Ordering::Relaxed) {
        return;
    }
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let event =
        match engine_production::construction_ic_sites::completion(&memory, base, line, railway) {
            Ok(event) => event,
            Err(reason) => {
                refuse(reason);
                return;
            }
        };
    append_completion(event, railway, false);
}

fn observe_linked_completion(status: usize, line: usize, caller: usize) {
    let base = BASE.load(Ordering::Relaxed);
    if caller != RETURNS[2].load(Ordering::Relaxed) {
        return;
    }
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    match engine_production::construction_ic_sites::linked_completion(&memory, base, status, line) {
        Ok(event) => append_completion(event, false, true),
        Err(reason) => refuse(reason),
    }
}

fn observe_count_reset(line: usize) {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    match engine_production::construction_ic_sites::count_reset(
        &memory,
        BASE.load(Ordering::Relaxed),
        line,
    ) {
        Ok((line, count, identity)) => append(Entry {
            line,
            identity,
            count_reset: Some(count),
            ..Entry::default()
        }),
        Err(reason) => refuse(reason),
    }
}

fn append_completion(
    event: engine_production::construction_ic_sites::Completion,
    railway: bool,
    linked: bool,
) {
    append(Entry {
        line: event.line,
        identity: event.identity,
        railway,
        completion: Some(replay_data::rows::ConstructionCompletion {
            hour: event.hour,
            cancelled: event.cancelled,
        }),
        linked_completion: linked,
        ..Entry::default()
    });
}

fn observe_segment(amount: i64) {
    let (line, calls) = RAILWAY_LINE.get();
    if line == 0 {
        return;
    }
    RAILWAY_LINE.set((line, 1));
    if calls == 0 {
        COUNTERS[3].fetch_add(1, Ordering::Relaxed);
        observe(line, amount, true, false);
    } else {
        COUNTERS[4].fetch_add(1, Ordering::Relaxed);
        observe(line, amount, true, true);
    }
}

fn append(entry: Entry) {
    COUNTERS[5].fetch_add(1, Ordering::Relaxed);
    if entry.identity.is_none() {
        COUNTERS[12].fetch_add(1, Ordering::Relaxed);
    }
    if entry.instant {
        COUNTERS[6].fetch_add(1, Ordering::Relaxed);
    }
    let (index, reserved) = OBSERVATION.reserve();
    let slot = match reserved {
        Reserved::Slot(slot) => log().and_then(|log| log.slots.get(slot)),
        Reserved::Spill => None,
    };
    let Some(slot) = slot else {
        SPILLED.fetch_add(1, Ordering::Relaxed);
        OBSERVATION.spill(&SPILL, index, entry);
        return;
    };
    let identity = entry.identity.unwrap_or_default();
    let flags = (u16::from(entry.railway) * RAILWAY)
        | (u16::from(entry.repair) * REPAIR)
        | (u16::from(entry.instant) * INSTANT)
        | (u16::from(entry.partner.is_some() || entry.linked_completion) * LINKED)
        | (u16::from(entry.carry) * CARRY)
        | (u16::from(entry.completion.is_some()) * COMPLETE)
        | (u16::from(entry.completion.is_some_and(|event| event.cancelled)) * CANCELLED)
        | (u16::from(entry.count_reset.is_some()) * RESET)
        | (u16::from(identity.route_first.is_some()) * ROUTE_FIRST)
        | (u16::from(identity.route_last.is_some()) * ROUTE_LAST)
        | (u16::from(entry.identity.is_some()) * HAS_IDENTITY);
    slot.line.store(entry.line, Ordering::SeqCst);
    let pair = |first: u32, second: u32| u64::from(first) | (u64::from(second) << 32);
    for (slot, value) in slot.identity.iter().zip([
        pair(identity.country, identity.building_type),
        pair(identity.target_flags, identity.target_location),
        pair(identity.target_state, identity.target_country),
        pair(
            identity.route_first.unwrap_or(0),
            identity.route_last.unwrap_or(0),
        ),
    ]) {
        slot.store(value, Ordering::SeqCst);
    }
    slot.partner
        .store(entry.partner.unwrap_or(0), Ordering::SeqCst);
    slot.amount.store(
        entry.completion.map_or_else(
            || entry.count_reset.map_or(entry.amount, i64::from),
            |event| i64::from(event.hour),
        ) as u64,
        Ordering::SeqCst,
    );
    slot.bank.store(entry.bank as u64, Ordering::SeqCst);
    slot.flags.store(flags, Ordering::SeqCst);
    OBSERVATION.finish(index);
}

fn entry(slot: &Slot) -> Entry {
    let flags = slot.flags.load(Ordering::SeqCst);
    let amount = slot.amount.load(Ordering::SeqCst) as i64;
    let identity = (flags & HAS_IDENTITY != 0).then(|| {
        let [owner_type, location, target, route] = slot
            .identity
            .each_ref()
            .map(|value| value.load(Ordering::SeqCst));
        engine_production::construction_ic_sites::Identity {
            country: owner_type as u32,
            building_type: (owner_type >> 32) as u32,
            target_flags: location as u32,
            target_location: (location >> 32) as u32,
            target_state: target as u32,
            target_country: (target >> 32) as u32,
            route_first: (flags & ROUTE_FIRST != 0).then_some(route as u32),
            route_last: (flags & ROUTE_LAST != 0).then_some((route >> 32) as u32),
        }
    });
    Entry {
        line: slot.line.load(Ordering::SeqCst),
        identity,
        carry: flags & CARRY != 0,
        amount: if flags & (COMPLETE | RESET) == 0 {
            amount
        } else {
            0
        },
        bank: slot.bank.load(Ordering::SeqCst) as i64,
        railway: flags & RAILWAY != 0,
        repair: flags & REPAIR != 0,
        instant: flags & INSTANT != 0,
        partner: (flags & LINKED != 0 && flags & COMPLETE == 0)
            .then(|| slot.partner.load(Ordering::SeqCst)),
        completion: (flags & COMPLETE != 0).then_some(replay_data::rows::ConstructionCompletion {
            hour: amount as i32,
            cancelled: flags & CANCELLED != 0,
        }),
        linked_completion: flags & (COMPLETE | LINKED) == (COMPLETE | LINKED),
        count_reset: (flags & RESET != 0).then_some(amount as i32),
    }
}

pub(crate) fn drain(output: &mut Vec<Entry>) -> Result<Drained, &'static str> {
    let log = log();
    OBSERVATION.drain(
        output,
        &SPILL,
        |slot| Some(entry(log?.slots.get(slot)?)),
        unavailable,
        "construction line observation restarted during the drain",
        DrainCounters {
            drains: &COUNTERS[7],
            copied: &COUNTERS[8],
            overflow: &COUNTERS[9],
            unobserved: &COUNTERS[10],
        },
    )
}

fn unavailable() -> Option<&'static str> {
    if let Some(reason) = UNHOOKED.get() {
        Some(*reason)
    } else if REFUSAL_PENDING.swap(false, Ordering::SeqCst) {
        REFUSED.get().copied()
    } else if !ENABLED.load(Ordering::SeqCst) {
        Some("construction line hooks are not enabled")
    } else {
        None
    }
}

mod demand;

pub fn presize_from_live(memory: &impl Memory, base: usize) -> Result<(), &'static str> {
    demand::presize(memory, base)
}

pub fn status() -> impl serde::Serialize {
    let counters = COUNTERS
        .each_ref()
        .map(|value| value.load(Ordering::Relaxed));
    let base = output_hooks::status(&OBSERVATION, &UNHOOKED, Some(&INSTALLED));
    base.join(crate::status::status!({
        "capacity": capacity(),
        "required": OBSERVATION.required.load(Ordering::Acquire),
        "swaps": SWAPS.load(Ordering::Relaxed),
        "failed_offers": FAILED_OFFERS.load(Ordering::Relaxed),
        "spilled_entries": SPILLED.load(Ordering::Relaxed),
        "building_spends": counters[0],
        "other_speed_reads": counters[1],
        "railway_advances": counters[2],
        "railway_spends": counters[3],
        "railway_carries": counters[4],
        "entries": counters[5],
        "instant_entries": counters[6],
        "drains": counters[7],
        "handed_over": counters[8],
        "overflowed_logs": counters[9],
        "unobserved_drains": counters[10],
        "refused_entries": counters[11],
        "identity_unread": counters[12],
        "identity_conflicts": crate::construction_tables::identity_conflicts(),
        "refused": REFUSED.get(),
        "scope": "cumulative since attach; building_spends count the speed reads at the advance's spend site (other_speed_reads every other caller, such as the repair rate and the production screen); entries include spends, railway carries, zero-IC building advances and native daily/linked completion decisions"
    }))
}

pub struct Hooks {
    shared: ManuallyDrop<output_hooks::Hooks<7>>,
}
pub const SITES: [&Site; 7] = [
    &BUILDING_SPEED,
    &RAILWAY_ADVANCE,
    &RAILWAY_PROGRESS,
    &BUILDING_COMPLETE,
    &RAILWAY_COMPLETE,
    &REMOVE_GENERAL_LINE,
    &BUILDING_RESET,
];
impl Hooks {
    /// The runtime's build gate admitted this family. Installation failure leaves
    /// the output table unavailable; site order preserves paired observation.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Self {
        let _ = presize_from_live(memory, base);
        // SAFETY: Caller exclusively owns this hook family during attach before enabling hooks.
        unsafe { install_offered() };
        let domain = output_hooks::Domain {
            failure: output_hooks::Failure::Unavailable,
            observation: &OBSERVATION,
            base: &BASE,
            originals: &ORIGINAL,
            installed: Some(&INSTALLED),
            unhooked: &UNHOOKED,
        };
        Self {
            shared: ManuallyDrop::new(unsafe {
                output_hooks::Hooks::new(
                    base,
                    SITES.map(output_hooks::Site::from),
                    [
                        speed_entry as *const () as usize,
                        railway_advance as *const () as usize,
                        railway_progress as *const () as usize,
                        building_complete_entry as *const () as usize,
                        railway_complete_entry as *const () as usize,
                        linked_complete_entry as *const () as usize,
                        building_reset as *const () as usize,
                    ],
                    domain,
                    bind_returns(base),
                )
            }),
        }
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        self.shared.set_enabled(enabled)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.shared.executable_ranges()
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        if active() != 0 {
            return;
        }
        unsafe { ManuallyDrop::drop(&mut self.shared) };
        LOG.store(ptr::null_mut(), Ordering::SeqCst);
        OBSERVATION.capacity.store(0, Ordering::Release);
        let mut swap = LOG_SWAP
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *swap = LogSwap::default();
        SPILL.release();
    }
}

fn original(site: usize) -> usize {
    ORIGINAL[site].load(Ordering::Acquire)
}

unsafe extern "system-unwind" fn building_reset(line: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ConstructionIc);
    let _call = OBSERVATION.enter();
    observe_count_reset(line);
    let original: unsafe extern "system-unwind" fn(usize) =
        unsafe { std::mem::transmute(original(6)) };
    timing.original(|| unsafe { original(line) });
}

#[unsafe(naked)]
unsafe extern "system" fn speed_entry() {
    core::arch::naked_asm!("mov r8, qword ptr [rsp]", "jmp {body}", body = sym speed);
}

#[unsafe(naked)]
unsafe extern "system" fn building_complete_entry() {
    core::arch::naked_asm!("mov rdx, qword ptr [rsp]", "jmp {body}", body = sym building_complete);
}
#[unsafe(naked)]
unsafe extern "system" fn railway_complete_entry() {
    core::arch::naked_asm!("mov rdx, qword ptr [rsp]", "jmp {body}", body = sym railway_complete);
}

unsafe extern "system" fn building_complete(line: usize, caller: usize) -> u8 {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ConstructionIc);
    let _call = OBSERVATION.enter();
    let original: unsafe extern "system" fn(usize) -> u8 =
        unsafe { std::mem::transmute(original(3)) };
    let result = timing.original(|| unsafe { original(line) });
    observe_completion(line, caller, result, false);
    result
}
unsafe extern "system" fn railway_complete(line: usize, caller: usize) -> u8 {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ConstructionIc);
    let _call = OBSERVATION.enter();
    let original: unsafe extern "system" fn(usize) -> u8 =
        unsafe { std::mem::transmute(original(4)) };
    let result = timing.original(|| unsafe { original(line) });
    observe_completion(line, caller, result, true);
    result
}

#[unsafe(naked)]
unsafe extern "system" fn linked_complete_entry() {
    core::arch::naked_asm!("mov r8, qword ptr [rsp]", "jmp {body}", body = sym linked_complete);
}
unsafe extern "system" fn linked_complete(status: usize, line: usize, caller: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ConstructionIc);
    let _call = OBSERVATION.enter();
    observe_linked_completion(status, line, caller);
    let original: unsafe extern "system" fn(usize, usize) =
        unsafe { std::mem::transmute(original(5)) };
    timing.original(|| unsafe { original(status, line) });
}

unsafe extern "system" fn speed(line: usize, out: usize, caller: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ConstructionIc);
    let _call = OBSERVATION.enter();
    let original: unsafe extern "system" fn(usize, usize) -> usize =
        unsafe { std::mem::transmute(original(0)) };
    let result = timing.original(|| unsafe { original(line, out) });
    observe_building(line, caller);
    result
}

unsafe extern "system" fn railway_advance(line: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ConstructionIc);
    let _call = OBSERVATION.enter();
    COUNTERS[2].fetch_add(1, Ordering::Relaxed);
    let outer = RAILWAY_LINE.replace((line, 0));
    let original: unsafe extern "system" fn(usize) = unsafe { std::mem::transmute(original(1)) };
    timing.original(|| unsafe { original(line) });
    RAILWAY_LINE.set(outer);
}

unsafe extern "system" fn railway_progress(
    manager: usize,
    from: u32,
    to: u32,
    amount: i64,
    country: usize,
) -> u8 {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ConstructionIc);
    let _call = OBSERVATION.enter();
    let original: unsafe extern "system" fn(usize, u32, u32, i64, usize) -> u8 =
        unsafe { std::mem::transmute(original(2)) };
    let completed = timing.original(|| unsafe { original(manager, from, to, amount, country) });
    observe_segment(amount);
    completed
}

#[cfg(any(test, feature = "test-support"))]
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
pub(crate) fn observe_for_test(enabled: bool, base: usize) {
    if enabled {
        // SAFETY: In-crate tests hold `test_lock()` with hooks quiescent before enabling.
        unsafe { reset_log_for_test(crate::output_log::CAPACITY) };
    }
    BASE.store(base, Ordering::Relaxed);
    bind_returns(base).unwrap();
    ENABLED.store(enabled, Ordering::SeqCst);
    CONSUMED.store(CURSOR.load(Ordering::SeqCst), Ordering::Relaxed);
    RAILWAY_LINE.set((0, 0));
}

/// # Safety
/// Caller holds [`test_lock()`] (or exclusive fixture ownership) with construction-IC hooks
/// disabled and quiescent (`ACTIVE == 0`).
#[cfg(any(test, feature = "test-support"))]
pub unsafe fn reset_log_for_test(capacity: usize) {
    assert_eq!(active(), 0);
    let mut current = Storage::new(capacity).unwrap();
    let pointer = &raw mut *current;
    *LOG_SWAP.lock().unwrap() = LogSwap {
        current: Some(current),
        ..Default::default()
    };
    LOG.store(pointer, Ordering::SeqCst);
    OBSERVATION.capacity.store(capacity, Ordering::Release);
    OBSERVATION.required.store(0, Ordering::Release);
    CONSUMED.store(CURSOR.load(Ordering::SeqCst), Ordering::Relaxed);
    SPILL.clear();
}

#[cfg(test)]
#[path = "construction_ic/growth_tests.rs"]
mod growth_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::{ccountry, cgame_state::{self, SINGLETON_RVA}, cproduction_status};
    use engine_native_parts::cproduction_line;
    use engine_production::{
        construction_ic_sites::CIC_BANK_DRAW, construction_ic_sites::INSTANT_CONSTRUCTION,
        construction_ic_sites::INVALID_REFERENCE, construction_ic_sites::LINE_HANDLE,
        construction_ic_sites::LINE_VTABLES, construction_ic_sites::LINKED_LINE,
        construction_ic_sites::RAILWAY_INSTANT_AMOUNT, construction_ic_sites::REPAIR_LEVELS,
    };

    #[test]
    fn count_reset_entries_preserve_quantity_without_spending_or_completion() {
        let _lock = test_lock();
        observe_for_test(true, 0);
        for count in [0, 1, i32::MAX, i32::MIN] {
            let expected = Entry {
                line: 19,
                count_reset: Some(count),
                ..Entry::default()
            };
            let _writer = OBSERVATION.enter();
            append(expected);
            drop(_writer);
            let mut entries = vec![Entry::default(); 1];
            drain(&mut entries).unwrap();
            assert_eq!(entries[0], expected);
        }
        observe_for_test(false, 0);
    }

    #[test]
    fn completion_entries_keep_native_date_and_do_not_become_output() {
        let _lock = test_lock();
        observe_for_test(true, 0);
        observe_completion(0, COMPLETE_RETURN.0, 0, false);
        observe_completion(0, COMPLETE_RETURN.0 + 1, 1, false);
        observe_linked_completion(0, 0, LINKED_COMPLETE_RETURN.0 + 1);
        for cancelled in [false, true] {
            append(Entry {
                line: 9,
                railway: true,
                completion: Some(replay_data::rows::ConstructionCompletion {
                    hour: -1,
                    cancelled,
                }),
                ..Entry::default()
            });
        }
        append_completion(
            engine_production::construction_ic_sites::Completion {
                line: 10,
                identity: Default::default(),
                hour: -1,
                cancelled: false,
            },
            false,
            true,
        );
        let (result, rows) = drained(4);
        assert!(result.unwrap().complete());
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows.iter()
                .map(|row| row.completion.unwrap().cancelled)
                .collect::<Vec<_>>(),
            [false, true, false]
        );
        assert!(
            rows.iter()
                .all(|row| row.amount == 0 && row.bank == 0 && row.completion.unwrap().hour == -1)
        );
        assert!(rows[2].linked_completion && rows[2].partner.is_none());
        observe_for_test(false, 0);
    }

    struct Image {
        bytes: Vec<u64>,
        owner: Option<Owner>,
    }
    impl Image {
        const INVALID: u64 = 0xffff_ffff_0000_0000;
        fn new() -> Self {
            let mut bytes = vec![0_u64; (INVALID_REFERENCE.0 - SINGLETON_RVA.0) / 8 + 1];
            bytes[(INVALID_REFERENCE.0 - SINGLETON_RVA.0) / 8] = Self::INVALID;
            let mut image = Self { bytes, owner: None };
            image.owner = Some(Owner::new(&mut image, 7, 0));
            image
        }
        fn base(&self) -> usize {
            (self.bytes.as_ptr() as usize) - SINGLETON_RVA.0
        }
        fn set_instant(&mut self, on: bool) {
            self.bytes[(INSTANT_CONSTRUCTION.0 - SINGLETON_RVA.0) / 8] = u64::from(on);
        }
        fn set_state(&mut self, state: usize) {
            self.bytes[0] = state as u64;
        }
    }

    struct Owner {
        status: Vec<u64>,
        country: Vec<u64>,
        state: Vec<u64>,
        links: Vec<u32>,
        map_state: Vec<u64>,
        map: Vec<u64>,
        building: Vec<u64>,
    }
    impl Owner {
        fn new(image: &mut Image, id: u32, playing: u32) -> Self {
            let base = image.base();
            let mut owner = Self {
                status: vec![0; cproduction_status::COUNTRY / 8 + 1],
                country: vec![0; ccountry::PRODUCTION_STATUS / 8 + 1],
                state: vec![0; cgame_state::LOCAL_PLAYER_COUNTRY / 8 + 1],
                links: vec![0, 1, 2, 3, 4, 5, 6, 5],
                map_state: vec![0; 0x200 / 8],
                map: vec![0; 2],
                building: vec![0; 0x200 / 8],
            };
            let status = owner.status.as_ptr() as u64;
            let country = owner.country.as_ptr() as u64;
            owner.status[0] = (base + cproduction_status::VTABLE_RVA.0) as u64;
            owner.status[cproduction_status::COUNTRY / 8] = country;
            owner.country[0] = (base + ccountry::VTABLE_RVA.0) as u64;
            owner.country[ccountry::ID / 8] = u64::from(id);
            owner.country[ccountry::PRODUCTION_STATUS / 8] = status;
            owner.state[cgame_state::COUNTRY_LINKS / 8] = owner.links.as_ptr() as u64;
            owner.state[cgame_state::COUNTRY_LINKS / 8 + 1] = 8 | 8 << 32;
            owner.state[cgame_state::LOCAL_PLAYER_COUNTRY / 8] = u64::from(playing);
            use engine_native_parts::cbuilding;
            use engine_native::{cprovince::buildings as cbuilding_status, cstate};
            owner.map[1] = owner.map_state.as_ptr() as u64;
            for field in [cgame_state::STATES, cgame_state::PROVINCES] {
                owner.state[field / 8] = owner.map.as_ptr() as u64;
                owner.state[field / 8 + 1] = 2 | 2 << 32;
            }
            owner.map_state[0] = (base + cstate::VTABLE_RVA.0) as u64;
            owner.map_state[cstate::ID / 8] = 1;
            let status = cstate::BUILDING_STATUS;
            owner.map_state[status / 8] = (base + cbuilding_status::VTABLE_RVA.0) as u64;
            owner.map_state[(status + cbuilding_status::FLAGS) / 8] = 2 | 1 << 32;
            owner.building[0] = (base + cbuilding::VTABLE_RVA.0) as u64;
            owner.building[cbuilding::TYPE_ID / 8] = 9;
            owner.building[cbuilding::BUILDING_STATUS / 8] =
                owner.map_state.as_ptr() as u64 + status as u64;
            image.set_state(owner.state.as_ptr() as usize);
            owner
        }
    }

    struct Line {
        line: Vec<u64>,
        _route: Vec<u32>,
    }
    impl Line {
        fn new(
            image: &Image,
            railway: bool,
            handle: u64,
            bank: i64,
            repair: i32,
            linked: bool,
        ) -> Self {
            let base = image.base();
            let mut line = vec![0_u64; 0x20];
            line[0] = (base + LINE_VTABLES[usize::from(railway)].0) as u64;
            line[LINE_HANDLE / 8] = handle;
            line[cproduction_line::COST / 8] = 2_000_000;
            line[cproduction_line::SPEED / 8] = 180_000;
            line[CIC_BANK_DRAW / 8] = bank as u64;
            line[REPAIR_LEVELS / 8] = repair as u32 as u64;
            line[LINKED_LINE / 8] = if linked {
                0x7_0000_003b
            } else {
                Image::INVALID
            };
            let owner = image.owner.as_ref().unwrap();
            line[cproduction_line::PRODUCTION_STATUS / 8] = owner.status.as_ptr() as u64;
            let route = vec![3_u32, 4];
            if railway {
                line[engine_production::crailway_production_line::ROUTE / 8] =
                    route.as_ptr() as u64;
                line[engine_production::crailway_production_line::ROUTE / 8 + 1] = 2 | 2 << 32;
            } else {
                line[engine_production::cbuilding_production_line::BUILDING / 8] =
                    owner.building.as_ptr() as u64;
            }
            Self {
                line,
                _route: route,
            }
        }
        fn owned_by(mut self, owner: &Owner) -> Self {
            self.line[cproduction_line::PRODUCTION_STATUS / 8] = owner.status.as_ptr() as u64;
            self
        }
        fn address(&self) -> usize {
            self.line.as_ptr() as usize
        }
    }

    fn observe_speed(line: &mut Line, amount: i64, caller: usize) {
        line.line[cproduction_line::SPEED / 8] = amount as u64;
        observe_building(line.address(), caller);
    }

    fn drained(capacity: usize) -> (Result<Drained, &'static str>, Vec<Entry>) {
        let mut output = vec![Entry::default(); capacity];
        let result = drain(&mut output);
        let copied = result.map_or(0, |drained| drained.copied);
        output.truncate(copied);
        (result, output)
    }

    fn building_identity() -> engine_production::construction_ic_sites::Identity {
        engine_production::construction_ic_sites::Identity {
            country: 7,
            building_type: 9,
            target_flags: 2,
            target_location: 1,
            target_state: 1,
            ..Default::default()
        }
    }
    fn railway_identity() -> engine_production::construction_ic_sites::Identity {
        engine_production::construction_ic_sites::Identity {
            country: 7,
            route_first: Some(3),
            route_last: Some(4),
            ..Default::default()
        }
    }

    fn spend_site(image: &Image) -> usize {
        image.base() + BUILDING_SPEND_RETURN.0
    }

    #[test]
    fn a_building_spend_is_the_speed_read_at_its_advance() {
        let _lock = test_lock();
        let image = Image::new();
        observe_for_test(true, image.base());
        let mut building = Line::new(&image, false, 0x11_0000_003b, 50_000, 0, false);
        let mut repairing = Line::new(&image, false, 0x12_0000_003b, 0, 2, true);
        let mut idle = Line::new(&image, false, 0x13_0000_003b, 0, 0, false);
        observe_speed(&mut building, 1_850_000, spend_site(&image));
        observe_speed(&mut building, 1_850_000, 0x1234);
        observe_building(usize::MAX, spend_site(&image) + 1);
        observe_speed(&mut repairing, 900_000, spend_site(&image));
        observe_speed(&mut idle, 0, spend_site(&image));
        let (result, entries) = drained(8);
        assert!(result.unwrap().complete());
        assert_eq!(
            entries,
            [
                Entry {
                    line: 0x11_0000_003b,
                    identity: Some(building_identity()),
                    amount: 1_850_000,
                    bank: 50_000,
                    ..Entry::default()
                },
                Entry {
                    line: 0x12_0000_003b,
                    identity: Some(building_identity()),
                    amount: 900_000,
                    repair: true,
                    partner: Some(0x7_0000_003b),
                    ..Entry::default()
                },
                Entry {
                    line: 0x13_0000_003b,
                    identity: Some(building_identity()),
                    ..Entry::default()
                },
            ]
        );
        observe_for_test(false, 0);
    }

    #[test]
    fn a_railway_spend_is_the_first_segment_call_of_its_advance() {
        let _lock = test_lock();
        let image = Image::new();
        observe_for_test(true, image.base());
        let rail = Line::new(&image, true, 0x21_0000_1269, 0, 0, false);
        RAILWAY_LINE.set((rail.address(), 0));
        observe_segment(1_800_000);
        observe_segment(300_000);
        RAILWAY_LINE.set((0, 0));
        observe_segment(700_000);
        let (_, entries) = drained(8);
        assert_eq!(
            entries,
            [
                Entry {
                    line: 0x21_0000_1269,
                    identity: Some(railway_identity()),
                    amount: 1_800_000,
                    railway: true,
                    ..Entry::default()
                },
                Entry {
                    line: 0x21_0000_1269,
                    identity: Some(railway_identity()),
                    amount: 300_000,
                    railway: true,
                    carry: true,
                    ..Entry::default()
                }
            ]
        );
        observe_for_test(false, 0);
    }

    #[test]
    fn instant_construction_is_marked_not_factory_output() {
        let _lock = test_lock();
        let mut image = Image::new();
        image.set_instant(true);
        observe_for_test(true, image.base());
        let mut building = Line::new(&image, false, 0x31_0000_003b, 0, 0, false);
        observe_speed(&mut building, 0, spend_site(&image));
        let (_, entries) = drained(8);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].instant);
        assert_eq!(entries[0].amount, 2_000_000);
        observe_speed(&mut building, 3_000_000, spend_site(&image));
        assert_eq!(drained(1).1[0].amount, 3_000_000);
        observe_for_test(false, 0);
    }

    #[test]
    fn a_repair_under_instant_construction_is_factory_output_without_a_bank_draw() {
        let _lock = test_lock();
        let mut image = Image::new();
        image.set_instant(true);
        observe_for_test(true, image.base());
        let mut repairing = Line::new(&image, false, 0x36_0000_003b, 20_000, 2, false);
        observe_speed(&mut repairing, 100_000, spend_site(&image));
        let (result, entries) = drained(2);
        assert!(result.unwrap().complete());
        assert_eq!(
            entries,
            [Entry {
                line: 0x36_0000_003b,
                identity: Some(building_identity()),
                amount: 80_000,
                repair: true,
                ..Entry::default()
            }]
        );
        observe_speed(&mut repairing, 10_000, spend_site(&image));
        assert!(drained(2).0.is_err());
        observe_for_test(false, 0);
    }

    #[test]
    fn partner_and_zero_ic_repair_fields_survive_the_log() {
        let _lock = test_lock();
        let image = Image::new();
        observe_for_test(true, image.base());
        let mut building = Line::new(&image, false, 0x33_0000_003b, 0, 2, true);
        observe_speed(&mut building, 0, spend_site(&image));
        let entries = drained(1).1;
        assert_eq!(entries.len(), 1);
        assert_eq!(
            (entries[0].partner, entries[0].amount, entries[0].repair,),
            (Some(0x7_0000_003b), 0, true)
        );
        observe_for_test(false, 0);
    }

    #[test]
    fn unread_identities_keep_spends_resets_and_completions_without_invalidating() {
        let _lock = test_lock();
        let image = Image::new();
        observe_for_test(true, image.base());
        let epoch = drained(0).0.unwrap().epoch;
        let refused = COUNTERS[11].load(Ordering::Relaxed);
        let unread = COUNTERS[12].load(Ordering::Relaxed);
        let mut building = Line::new(&image, false, 0x34_0000_003b, 0, 0, false);
        building.line[engine_production::cbuilding_production_line::BUILDING / 8] = 0;
        observe_speed(&mut building, 99, spend_site(&image));
        building.line[cproduction_line::AMOUNT / 8] = 3;
        observe_count_reset(building.address());
        observe_completion(building.address(), image.base() + COMPLETE_RETURN.0, 1, false);
        let mut known = Line::new(&image, false, 0x35_0000_003b, 0, 0, false);
        observe_speed(&mut known, 55, spend_site(&image));
        let (result, entries) = drained(4);
        let result = result.unwrap();
        assert!(result.complete());
        assert_eq!(result.epoch, epoch);
        assert_eq!(COUNTERS[11].load(Ordering::Relaxed), refused);
        assert_eq!(COUNTERS[12].load(Ordering::Relaxed), unread + 3);
        assert_eq!(entries.len(), 4);
        assert!(entries[..3].iter().all(|entry| entry.identity.is_none()));
        assert_eq!((entries[0].amount, entries[1].count_reset), (99, Some(3)));
        assert!(entries[2].completion.is_some());
        assert_eq!(
            (entries[3].amount, entries[3].identity),
            (55, Some(building_identity()))
        );
        let diagnostics = serde_json::to_value(status()).unwrap();
        assert_eq!(diagnostics["identity_unread"].as_u64(), Some(unread + 3));
        assert!(diagnostics["identity_conflicts"].is_u64());
        observe_for_test(false, 0);
    }

    #[test]
    fn unread_railway_route_keeps_spend_and_carry_in_the_same_observation() {
        let _lock = test_lock();
        let mut image = Image::new();
        observe_for_test(true, image.base());
        let epoch = drained(0).0.unwrap().epoch;
        let refused = COUNTERS[11].load(Ordering::Relaxed);
        let mut rail = Line::new(&image, true, 0x36_0000_1269, 0, 0, false);
        rail.line[engine_production::crailway_production_line::ROUTE / 8] = 0;
        RAILWAY_LINE.set((rail.address(), 0));
        observe_segment(80);
        observe_segment(20);
        let (result, entries) = drained(2);
        assert_eq!(result.unwrap().epoch, epoch);
        assert_eq!(COUNTERS[11].load(Ordering::Relaxed), refused);
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|entry| entry.identity.is_none()));
        assert_eq!((entries[0].amount, entries[0].carry), (80, false));
        assert_eq!((entries[1].amount, entries[1].carry), (20, true));
        image.set_instant(true);
        let owner = Owner::new(&mut image, 7, 7);
        let rail = rail.owned_by(&owner);
        RAILWAY_LINE.set((rail.address(), 0));
        observe_segment(RAILWAY_INSTANT_AMOUNT);
        let (result, entries) = drained(1);
        assert_eq!(result.unwrap().epoch, epoch);
        assert_eq!(COUNTERS[11].load(Ordering::Relaxed), refused);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].identity.is_none() && entries[0].instant);
        assert_eq!(entries[0].amount, RAILWAY_INSTANT_AMOUNT);
        observe_for_test(false, 0);
    }

    #[test]
    fn unreadable_spends_and_unsupported_bank_amounts_break_continuity() {
        let _lock = test_lock();
        let image = Image::new();
        observe_for_test(true, image.base());
        let mut building = Line::new(&image, false, 0x34_0000_003b, 50_000, 0, false);
        let mut previous = drained(1).0.unwrap().epoch;
        for unreadable in [true, false] {
            let refused = COUNTERS[11].load(Ordering::Relaxed);
            if unreadable {
                observe_building(usize::MAX, spend_site(&image));
            } else {
                observe_speed(&mut building, 0, spend_site(&image));
            }
            assert!(drained(1).0.is_err());
            assert_eq!(COUNTERS[11].load(Ordering::Relaxed), refused + 1);
            let (next, entries) = drained(1);
            let next = next.unwrap();
            assert!(next.epoch > previous && next.complete());
            assert!(entries.is_empty());
            previous = next.epoch;
        }
        observe_for_test(false, 0);
    }

    #[test]
    fn native_signed_building_speed_is_output_without_changing_counters() {
        let _lock = test_lock();
        let image = Image::new();
        observe_for_test(true, image.base());
        let mut building = Line::new(&image, false, 0x35_0000_003b, 0, 0, false);
        observe_speed(&mut building, -25, spend_site(&image));
        building.line[CIC_BANK_DRAW / 8] = 1;
        observe_speed(&mut building, i64::MIN, spend_site(&image));
        let (result, entries) = drained(4);
        assert!(result.unwrap().complete());
        assert_eq!(
            entries
                .iter()
                .map(|e| (e.amount, e.bank))
                .collect::<Vec<_>>(),
            [(-25, 0), (i64::MIN, 1)]
        );
        assert!(entries.iter().all(|e| e.completion.is_none() && !e.carry));
        observe_for_test(false, 0);
    }

    #[test]
    fn railway_without_a_segment_call_spends_nothing_and_instant_many_calls_stay_apart() {
        let _lock = test_lock();
        let mut image = Image::new();
        observe_for_test(true, image.base());
        let owner = Owner::new(&mut image, 5, 5);
        let rail = Line::new(&image, true, 0x32_0000_1269, 0, 0, false).owned_by(&owner);
        RAILWAY_LINE.set((rail.address(), 0));
        RAILWAY_LINE.set((0, 0));
        assert!(drained(8).1.is_empty());
        image.set_instant(true);
        RAILWAY_LINE.set((rail.address(), 0));
        for _ in 0..6 {
            observe_segment(RAILWAY_INSTANT_AMOUNT);
        }
        RAILWAY_LINE.set((0, 0));
        let entries = drained(8).1;
        assert_eq!(entries.len(), 6);
        assert!(entries.iter().all(|e| e.instant && e.bank == 0));
        assert_eq!(entries.iter().filter(|e| !e.carry).count(), 1);
        observe_for_test(false, 0);
    }

    #[test]
    fn normal_railway_spend_keeps_its_bank_draw_with_the_console_flag_set() {
        let _lock = test_lock();
        let mut image = Image::new();
        image.set_instant(true);
        observe_for_test(true, image.base());
        let owner = Owner::new(&mut image, 6, 5);
        let rail = Line::new(&image, true, 0x35_0000_1269, 50_000, 0, false).owned_by(&owner);
        RAILWAY_LINE.set((rail.address(), 0));
        observe_segment(180_000);
        observe_segment(30_000);
        RAILWAY_LINE.set((0, 0));
        let entries = drained(2).1;
        assert_eq!(
            (entries[0].amount, entries[0].bank, entries[0].instant),
            (180_000, 50_000, false)
        );
        assert_eq!(
            (entries[1].bank, entries[1].instant, entries[1].carry),
            (0, false, true)
        );
        observe_for_test(false, 0);
    }

    #[test]
    fn an_ordinary_railway_spend_equal_to_the_instant_amount_keeps_its_bank_draw() {
        let _lock = test_lock();
        let mut image = Image::new();
        observe_for_test(true, image.base());
        let other = Owner::new(&mut image, 6, 5);
        let rail = Line::new(&image, true, 0x37_0000_1269, 50_000, 0, false).owned_by(&other);
        for instant in [false, true] {
            image.set_instant(instant);
            RAILWAY_LINE.set((rail.address(), 0));
            observe_segment(RAILWAY_INSTANT_AMOUNT);
            observe_segment(RAILWAY_INSTANT_AMOUNT);
            RAILWAY_LINE.set((0, 0));
            let (result, entries) = drained(2);
            assert!(result.unwrap().complete());
            assert_eq!(
                entries
                    .iter()
                    .map(|e| (e.amount, e.bank, e.instant, e.carry))
                    .collect::<Vec<_>>(),
                [
                    (RAILWAY_INSTANT_AMOUNT, 50_000, false, false),
                    (RAILWAY_INSTANT_AMOUNT, 0, false, true)
                ]
            );
        }
        let player = Owner::new(&mut image, 7, 5);
        let rail = Line::new(&image, true, 0x38_0000_1269, 50_000, 0, false).owned_by(&player);
        let refused = COUNTERS[11].load(Ordering::Relaxed);
        RAILWAY_LINE.set((rail.address(), 0));
        observe_segment(180_000);
        RAILWAY_LINE.set((0, 0));
        assert!(drained(2).0.is_err());
        assert_eq!(COUNTERS[11].load(Ordering::Relaxed), refused + 1);
        observe_for_test(false, 0);
    }

    #[test]
    fn drains_defer_restart_and_refuse_as_the_line_output_log_does() {
        let _lock = test_lock();
        let image = Image::new();
        observe_for_test(true, image.base());
        let mut building = Line::new(&image, false, 0x51_0000_003b, 0, 0, false);
        for amount in 1..=5 {
            observe_speed(&mut building, amount, spend_site(&image));
        }
        let writer = OBSERVATION.enter();
        assert_eq!(drained(8).0.unwrap().copied, 0);
        drop(writer);
        let (result, entries) = drained(2);
        let first = result.unwrap();
        assert!(first.complete());
        assert_eq!((first.copied, first.pending), (5, 5));
        assert_eq!(
            entries.iter().map(|e| e.amount).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5]
        );
        let burst = capacity() + 1;
        for _ in 0..burst {
            observe_speed(&mut building, 1, spend_site(&image));
        }
        let (result, entries) = drained(4);
        let whole = result.unwrap();
        assert!(whole.complete());
        assert_eq!((whole.copied, whole.pending), (burst, burst));
        assert!(entries.iter().all(|entry| entry.amount == 1));
        ENABLED.store(false, Ordering::SeqCst);
        assert_eq!(drained(4).0, Err("construction line hooks are not enabled"));
        observe_for_test(false, 0);
    }

    #[test]
    fn railway_pairing_is_local_to_each_producer_thread() {
        let _lock = test_lock();
        let image = Image::new();
        observe_for_test(true, image.base());
        let lines = [
            Line::new(&image, true, 0x61_0000_1269, 0, 0, false),
            Line::new(&image, true, 0x62_0000_1269, 0, 0, false),
        ];
        std::thread::scope(|scope| {
            for line in &lines {
                scope.spawn(move || {
                    for _ in 0..100 {
                        let _advance = OBSERVATION.enter();
                        RAILWAY_LINE.set((line.address(), 0));
                        {
                            let _progress = OBSERVATION.enter();
                            observe_segment(100);
                        }
                        {
                            let _progress = OBSERVATION.enter();
                            observe_segment(20);
                        }
                        RAILWAY_LINE.set((0, 0));
                    }
                });
            }
        });
        let (d, entries) = drained(512);
        assert!(d.unwrap().complete());
        assert_eq!(entries.len(), 400);
        for line in &lines {
            let entries: Vec<_> = entries
                .iter()
                .filter(|e| e.line == line.line[LINE_HANDLE / 8])
                .collect();
            assert_eq!(
                entries
                    .iter()
                    .filter(|e| !e.carry)
                    .map(|e| e.amount)
                    .sum::<i64>(),
                10_000
            );
            assert_eq!(
                entries
                    .iter()
                    .filter(|e| e.carry)
                    .map(|e| e.amount)
                    .sum::<i64>(),
                2_000
            );
        }
        observe_for_test(false, 0);
    }
}
