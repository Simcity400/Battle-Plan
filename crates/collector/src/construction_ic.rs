//! Allocation-free observation of the civilian-factory output every construction
//! line consumes each day ([`engine::construction_ic_sites`]), which the game keeps
//! nowhere: `produced` holds only the progress toward a building's next level, and
//! a repair keeps its progress on the building. A detour only records and calls the
//! original; it never changes game behaviour and calls no game function.
//!
//! A building line spends the value its speed getter returns to one call site in
//! its advance; the detour on the getter records it only for that return address,
//! with whether the line was repairing, its CIC bank draw, and whether it is linked
//! to another country's line. A railway line spends the amount its advance passes
//! to the segment-progress function first; a second call in the same advance
//! carries an overdrawn amount to the next segment and is not new output. The
//! railway advance's detour marks its line on the thread, so the segment detour
//! knows whose output it is.
//!
//! Every clocked hourly boundary joins the worker that grows its owned frame
//! from the original pending burst ([`drain_growing`]); the spool totals it per
//! branch (`extended::construction_ic`). Producer storage follows the original
//! native construction queues and route counts at attach and input cadence,
//! plus complete attempted bursts. After its owned drain, the empty boundary
//! installs prepared storage; retired storage is released off-thread.
//! The detours run on the simulation thread (the daily production pass is serial);
//! the log is multi-producer safe anyway, and the railway pairing is per thread.
//!
//! A site whose prologue differs from the pinned sentinel, or that cannot be
//! detoured, leaves the family uninstalled: the collector still attaches and
//! ConstructionLineOutput is unavailable with the reason (`unhooked` in
//! `status.json`).
pub use crate::output_log::Drained;
use crate::{
    opinion_changes::Padded,
    output_hooks,
    output_log::{DrainCounters, Log},
};
use engine::{
    Memory,
    construction_ic_sites::{
        BUILDING_COMPLETE, BUILDING_RESET, BUILDING_SPEED, BUILDING_SPEND_RETURN, COMPLETE_RETURN,
        LINKED_COMPLETE_RETURN, RAILWAY_ADVANCE, RAILWAY_COMPLETE, RAILWAY_PROGRESS,
        REMOVE_GENERAL_LINE, Site,
    },
};
use std::{
    cell::Cell,
    mem::ManuallyDrop,
    ops::Range,
    ptr,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU64, AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct Slot {
    line: AtomicU64,
    partner: AtomicU64,
    amount: AtomicU64,
    bank: AtomicU64,
    flags: AtomicU8,
}

struct Storage {
    slots: Vec<Slot>,
}
impl Storage {
    fn new(capacity: usize) -> Option<Box<Self>> {
        let mut slots = engine::allocation::vec(capacity).ok()?;
        slots.resize_with(capacity, Slot::default);
        engine::allocation::boxed(Self { slots }).ok()
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
static REJECTED: AtomicU64 = AtomicU64::new(0);
static OBSERVATION: Log = Log::new(0);
static CURSOR: &Padded<AtomicU64> = &OBSERVATION.cursor;
static ENABLED: &AtomicBool = &OBSERVATION.enabled;
pub static ACTIVE: &Padded<AtomicUsize> = &OBSERVATION.active;
/// The spend and native completion trampolines.
static INSTALLED: [AtomicBool; 7] = [const { AtomicBool::new(false) }; 7];
static ORIGINAL: [AtomicUsize; 7] = [const { AtomicUsize::new(0) }; 7];
/// The module base the RVAs are relative to.
static BASE: AtomicUsize = AtomicUsize::new(0);
/// The next entry the hourly drain hands over; only the callback moves it.
static CONSUMED: &AtomicU64 = &OBSERVATION.consumed;
/// Why the hooks are not installed, when they are not.
static REFUSED: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
static REFUSAL_PENDING: AtomicBool = AtomicBool::new(false);
static UNHOOKED: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
/// `[building spends, other speed reads, railway advances, railway spends,
/// railway carries, entries, inexact entries, drains, entries handed over,
/// overflowed logs, unobserved drains, refused operand reads]`.
static COUNTERS: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];

pub fn capacity() -> usize {
    OBSERVATION.capacity.load(Ordering::Acquire)
}

/// Loader/sizing thread: two native passes cover the bounded hourly handoff,
/// independently of country count. Rejected bursts retain their whole demand.
pub fn presize(events: usize) {
    engine::allocation::advisory(|| presize_storage(events));
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

/// Parked hourly boundary, after its owned drain: no allocation or freeing.
pub fn install_offered() {
    let Ok(mut swap) = LOG_SWAP.try_lock() else {
        return;
    };
    if swap.retired.is_some()
        || ACTIVE.load(Ordering::SeqCst) != 0
        || CURSOR.load(Ordering::SeqCst) != CONSUMED.load(Ordering::Relaxed)
    {
        return;
    }
    let Some(mut next) = swap.next.take() else {
        return;
    };
    let capacity = next.slots.len();
    let pointer = &mut *next as *mut Storage;
    swap.retired = swap.current.replace(next);
    LOG.store(pointer, Ordering::SeqCst);
    OBSERVATION.capacity.store(capacity, Ordering::Release);
    if swap.retired.is_some() {
        SWAPS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Off-thread only: the replaced producer was empty and writer-free.
pub fn collect() {
    if let Ok(mut swap) = LOG_SWAP.lock() {
        drop(swap.retired.take());
    }
}

fn log() -> Option<&'static Storage> {
    // SAFETY: writers hold OBSERVATION.enter(). The sole worker drain finishes
    // before the same parked hourly boundary can install a replacement.
    unsafe { LOG.load(Ordering::SeqCst).as_ref() }
}

const RAILWAY: u8 = 1;
const REPAIR: u8 = 2;
const INSTANT: u8 = 4;
const LINKED: u8 = 8;
const CARRY: u8 = 16;
const COMPLETE: u8 = 32;
const CANCELLED: u8 = 64;
const RESET: u8 = 128;

/// One construction spend, railway carry or native completion decision.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    /// The line's CID, the construction tables' `id`.
    pub line: u64,
    /// Later segment call in the same advance; never new factory spending.
    pub carry: bool,
    /// The progress the day added (fixed5): the factories' output plus `bank`.
    pub amount: i64,
    /// Of it, drawn from the country's CIC bank.
    pub bank: i64,
    pub railway: bool,
    /// Spent on repairing the building, not on new levels.
    pub repair: bool,
    /// Under the instant-construction cheat: the amount is not factory output.
    pub instant: bool,
    /// A building line linked to another country's: its speed includes the
    /// partner's factories.
    pub partner: Option<u64>,
    /// A native daily or linked-line completion, independent of spending.
    pub completion: Option<replay::rows::ConstructionCompletion>,
    /// The secondary receiver completed inside its primary's advance; its share
    /// stays unknown even if the primary spend preceded this observation.
    pub linked_completion: bool,
    /// Signed quantity cleared on completion, never spending.
    pub count_reset: Option<i32>,
}

thread_local! {
    /// The railway line advancing on this thread; zero until its first spend.
    static RAILWAY_LINE: Cell<(usize, u32)> = const { Cell::new((0, 0)) };
}

/// A hook state change or a lapped writer: no drain can continue across it.
pub fn invalidate() {
    OBSERVATION.invalidate();
}

/// Buildings use their copied speed; railways supply the segment call's amount.
fn observe(line: usize, amount: i64, railway: bool, carry: bool) {
    let base = BASE.load(Ordering::Relaxed);
    let memory = unsafe { engine::memory::LiveMemory::new() };
    let operands = match engine::construction_ic_sites::operands(&memory, base, line, railway) {
        Ok(operands) => operands,
        Err(reason) => {
            refuse(reason);
            return;
        }
    };
    // The speed getter returns the receiver's stored speed at the filtered site.
    let amount = if railway { amount } else { operands.speed };
    // Railway instant progress is player-only; the amount identifies the path
    // without reading the player global.
    let instant = if railway {
        amount == engine::construction_ic_sites::RAILWAY_INSTANT_AMOUNT
    } else {
        operands.instant
    };
    // Building speed is stored unclamped after an optional assertion and
    // added signed to progress. Industrial modifiers alone clamp; the final
    // infrastructure product and speed+bank sum can still be signed. Bank
    // reservation is >=0. Railway spend requires speed>0; carry is gated
    // by RBX>=0. A nonnegative speed below its bank is not that sum.
    if (railway && amount < 0)
        || operands.bank < 0
        || (!carry && !instant && amount >= 0 && operands.bank > amount)
    {
        refuse("construction output amount or CIC bank draw unsupported");
        return;
    }
    // Instant construction raises new-building progress to max(cost, speed).
    // Repair uses the speed again, even with the cheat on.
    let amount = if instant && !railway && !operands.repair {
        amount.max(operands.cost)
    } else {
        amount
    };
    append(Entry {
        line: operands.line,
        amount,
        bank: if carry || instant { 0 } else { operands.bank },
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
    invalidate();
}

/// After the getter returned to its advance, its stored speed is the spend.
/// Other callers do not read observation operands.
fn observe_building(line: usize, caller: usize) {
    if caller
        != BASE
            .load(Ordering::Relaxed)
            .wrapping_add(BUILDING_SPEND_RETURN)
    {
        COUNTERS[1].fetch_add(1, Ordering::Relaxed);
        return;
    }
    COUNTERS[0].fetch_add(1, Ordering::Relaxed);
    observe(line, 0, false, false);
}

fn observe_completion(line: usize, caller: usize, result: u8, railway: bool) {
    let base = BASE.load(Ordering::Relaxed);
    if result == 0 || caller != base.wrapping_add(COMPLETE_RETURN) {
        return;
    }
    let memory = unsafe { engine::memory::LiveMemory::new() };
    let event = match engine::construction_ic_sites::completion(&memory, base, line, railway) {
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
    if caller != base.wrapping_add(LINKED_COMPLETE_RETURN) {
        return;
    }
    let memory = unsafe { engine::memory::LiveMemory::new() };
    match engine::construction_ic_sites::linked_completion(&memory, base, status, line) {
        Ok(event) => append_completion(event, false, true),
        Err(reason) => refuse(reason),
    }
}

fn append_completion(
    event: engine::construction_ic_sites::Completion,
    railway: bool,
    linked: bool,
) {
    append(Entry {
        line: event.line,
        railway,
        completion: Some(replay::rows::ConstructionCompletion {
            hour: event.hour,
            cancelled: event.cancelled,
        }),
        linked_completion: linked,
        ..Entry::default()
    });
}

/// After a segment-progress call with `amount`, inside the advance of the
/// thread's railway line (if any): its first call is the spend, later ones carry.
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

/// Store an entry, guarded by [`Log::enter`]. A writer that stalled until writers
/// had appended a whole log after its index may have overwritten a newer entry
/// in its slot; every drain meanwhile saw it active and handed nothing over, so
/// it ends the observation (a new epoch) before it leaves.
fn append(entry: Entry) {
    COUNTERS[5].fetch_add(1, Ordering::Relaxed);
    if entry.instant {
        COUNTERS[6].fetch_add(1, Ordering::Relaxed);
    }
    let (index, slot) = OBSERVATION.reserve_optional();
    let Some(slot) = slot.and_then(|slot| log()?.slots.get(slot)) else {
        REJECTED.fetch_add(1, Ordering::Relaxed);
        invalidate();
        return;
    };
    let flags = (u8::from(entry.railway) * RAILWAY)
        | (u8::from(entry.repair) * REPAIR)
        | (u8::from(entry.instant) * INSTANT)
        | (u8::from(entry.partner.is_some() || entry.linked_completion) * LINKED)
        | (u8::from(entry.carry) * CARRY)
        | (u8::from(entry.completion.is_some()) * COMPLETE)
        | (u8::from(entry.completion.is_some_and(|event| event.cancelled)) * CANCELLED)
        | (u8::from(entry.count_reset.is_some()) * RESET);
    slot.line.store(entry.line, Ordering::SeqCst);
    slot.partner
        .store(entry.partner.unwrap_or(0), Ordering::SeqCst);
    // Completion entries use this slot for the native hour, never IC.
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
    Entry {
        line: slot.line.load(Ordering::SeqCst),
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
        completion: (flags & COMPLETE != 0).then_some(replay::rows::ConstructionCompletion {
            hour: amount as i32,
            cancelled: flags & CANCELLED != 0,
        }),
        linked_completion: flags & (COMPLETE | LINKED) == (COMPLETE | LINKED),
        count_reset: (flags & RESET != 0).then_some(amount as i32),
    }
}

/// Copy settled entries without allocating; short storage defers the rest.
#[cfg(test)]
pub fn drain(output: &mut [Entry]) -> Result<Drained, &'static str> {
    let log = log();
    OBSERVATION.drain(
        output,
        |slot| entry(&log.unwrap().slots[slot]),
        || unavailable(log.is_some()),
        "construction line observation restarted during the drain",
        DrainCounters {
            drains: &COUNTERS[7],
            copied: &COUNTERS[8],
            overflow: &COUNTERS[9],
            unobserved: &COUNTERS[10],
        },
    )
}

/// Joined acquisition worker only: reserve the original complete pending burst
/// before advancing its consumed frontier; the producer is never reread.
pub fn drain_growing(output: &mut Vec<Entry>) -> Result<Drained, &'static str> {
    let log = log();
    OBSERVATION.drain_growing(
        output,
        |slot| entry(&log.unwrap().slots[slot]),
        || unavailable(log.is_some()),
        "construction line observation restarted during the drain",
        DrainCounters {
            drains: &COUNTERS[7],
            copied: &COUNTERS[8],
            overflow: &COUNTERS[9],
            unobserved: &COUNTERS[10],
        },
    )
}

fn unavailable(installed: bool) -> Option<&'static str> {
    if let Some(reason) = UNHOOKED.get() {
        Some(*reason)
    } else if REFUSAL_PENDING.swap(false, Ordering::SeqCst) {
        REFUSED.get().copied()
    } else if !ENABLED.load(Ordering::SeqCst) {
        Some("construction line hooks are not enabled")
    } else if !installed {
        Some("construction line log is not installed")
    } else {
        None
    }
}

#[path = "country_scale/construction_demand.rs"]
mod demand;

/// Attach/input-demand cadence, off-thread: census native pass storage and
/// retain independent sizing failures without withdrawing current coverage.
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
        "rejected_entries": REJECTED.load(Ordering::Relaxed),
        "sizing": demand::status(),
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
        "refused": REFUSED.get(),
        "scope": "cumulative since attach; building_spends count the speed reads at the advance's spend site (other_speed_reads every other caller, such as the repair rate and the production screen); entries include spends, railway carries, zero-IC building advances and native daily/linked completion decisions"
    }))
}

pub struct Hooks {
    shared: ManuallyDrop<output_hooks::Hooks<7>>,
}
pub(crate) const SITES: [&Site; 7] = [
    &BUILDING_SPEED,
    &RAILWAY_ADVANCE,
    &RAILWAY_PROGRESS,
    &BUILDING_COMPLETE,
    &RAILWAY_COMPLETE,
    &REMOVE_GENERAL_LINE,
    &BUILDING_RESET,
];
#[cfg(test)]
fn validate_sentinels(memory: &impl Memory, base: usize) -> Result<(), &'static str> {
    output_hooks::validate_sentinels(
        memory,
        base,
        &SITES.map(output_hooks::Site::from),
        "construction line sentinel differs",
    )
}

impl Hooks {
    /// The runtime verified the executable hash. Installation failure leaves
    /// the output table unavailable; site order preserves paired observation.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Self {
        let _ = presize_from_live(memory, base);
        install_offered();
        let domain = output_hooks::Domain {
            sentinel_error: "construction line sentinel differs",
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
                    memory,
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
                    Ok(()),
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
        if ACTIVE.load(Ordering::SeqCst) != 0 {
            return;
        }
        // Runtime removes the family and joins drains before releasing it.
        unsafe { ManuallyDrop::drop(&mut self.shared) };
        LOG.store(ptr::null_mut(), Ordering::SeqCst);
        OBSERVATION.capacity.store(0, Ordering::Release);
        let mut swap = LOG_SWAP
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *swap = LogSwap::default();
    }
}

fn original(site: usize) -> usize {
    ORIGINAL[site].load(Ordering::Acquire)
}

unsafe extern "system-unwind" fn building_reset(line: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ConstructionIc);
    let _call = OBSERVATION.enter();
    let memory = unsafe { engine::memory::LiveMemory::new() };
    match engine::construction_ic_sites::count_reset(&memory, BASE.load(Ordering::Relaxed), line) {
        Ok((line, count)) => append(Entry {
            line,
            count_reset: Some(count),
            ..Entry::default()
        }),
        Err(reason) => refuse(reason),
    }
    let original: unsafe extern "system-unwind" fn(usize) =
        unsafe { std::mem::transmute(original(6)) };
    timing.original(|| unsafe { original(line) });
}

// The speed getter takes two integer arguments, so R8 is free: its entry copies
// its return address (the game's call site) there and jumps to the Rust body,
// which returns to the game itself. The entry touches no stack.
#[unsafe(naked)]
unsafe extern "system" fn speed_entry() {
    core::arch::naked_asm!("mov r8, qword ptr [rsp]", "jmp {body}", body = sym speed);
}

// The completion predicate takes only RCX and returns AL. RDX retains the
// game's return address without touching the stack; the body passes AL through.
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

// Linked-line removal takes RCX/RDX; R8 retains the native return address.
// The receiver must be observed before the original deletes it. Other removal
// callers perform no operand reads and emit no completion.
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

// Every argument is passed through unchanged and every result returned.
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

/// Tests that touch the process-wide observation state take this first.
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Observation on (true) or off, with RVAs relative to `base` and the drain at
/// the current cursor. Hold [`test_lock`].
#[cfg(test)]
pub(crate) fn observe_for_test(enabled: bool, base: usize) {
    if enabled {
        reset_log_for_test(crate::output_log::CAPACITY);
    }
    BASE.store(base, Ordering::Relaxed);
    ENABLED.store(enabled, Ordering::SeqCst);
    CONSUMED.store(CURSOR.load(Ordering::SeqCst), Ordering::Relaxed);
    RAILWAY_LINE.set((0, 0));
}

#[cfg(test)]
pub(crate) fn reset_log_for_test(capacity: usize) {
    assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
    let mut current = Storage::new(capacity).unwrap();
    let pointer = &mut *current as *mut Storage;
    *LOG_SWAP.lock().unwrap() = LogSwap {
        current: Some(current),
        ..Default::default()
    };
    LOG.store(pointer, Ordering::SeqCst);
    OBSERVATION.capacity.store(capacity, Ordering::Release);
    OBSERVATION.required.store(0, Ordering::Release);
    CONSUMED.store(CURSOR.load(Ordering::SeqCst), Ordering::Relaxed);
}

#[cfg(test)]
#[path = "construction_ic/growth_tests.rs"]
mod growth_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use engine::construction_ic_sites::{
        CIC_BANK_DRAW, INSTANT_CONSTRUCTION, INVALID_REFERENCE, LINE_HANDLE, LINE_VTABLES,
        LINKED_LINE, REPAIR_LEVELS,
    };
    use engine::cproduction_line;

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
            let mut entries = [Entry::default(); 1];
            drain(&mut entries).unwrap();
            assert_eq!(entries[0], expected);
        }
        observe_for_test(false, 0);
    }

    #[test]
    fn completion_entries_keep_native_date_and_do_not_become_output() {
        let _lock = test_lock();
        observe_for_test(true, 0);
        // Neither a false result nor another caller may read this null line.
        observe_completion(0, COMPLETE_RETURN, 0, false);
        observe_completion(0, COMPLETE_RETURN + 1, 1, false);
        // A command/hourly removal may never become a linked completion.
        observe_linked_completion(0, 0, LINKED_COMPLETE_RETURN + 1);
        for cancelled in [false, true] {
            append(Entry {
                line: 9,
                railway: true,
                completion: Some(replay::rows::ConstructionCompletion {
                    hour: -1,
                    cancelled,
                }),
                ..Entry::default()
            });
        }
        append_completion(
            engine::construction_ic_sites::Completion {
                line: 10,
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

    /// Owned memory standing in for the two globals the detours read (the instant
    /// flag and the invalid reference), placed so each is at its RVA from `base`;
    /// vtables are only compared as values.
    struct Image {
        bytes: Vec<u64>,
    }
    impl Image {
        const INVALID: u64 = 0xffff_ffff_0000_0000;
        fn new() -> Self {
            let mut bytes = vec![0_u64; (INVALID_REFERENCE - INSTANT_CONSTRUCTION) / 8 + 1];
            bytes[(INVALID_REFERENCE - INSTANT_CONSTRUCTION) / 8] = Self::INVALID;
            Self { bytes }
        }
        fn base(&self) -> usize {
            (self.bytes.as_ptr() as usize) - INSTANT_CONSTRUCTION
        }
        fn set_instant(&mut self, on: bool) {
            self.bytes[0] = u64::from(on);
        }
    }

    /// A live line image; owner pointers are intentionally absent.
    struct Line {
        line: Vec<u64>,
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
            line[0] = (base + LINE_VTABLES[usize::from(railway)]) as u64;
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
            Self { line }
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

    fn spend_site(image: &Image) -> usize {
        image.base() + BUILDING_SPEND_RETURN
    }

    /// A building line's spend is the speed its advance reads; the repair rate's
    /// and the production screen's reads are not spends. Repair, the bank draw
    /// and a link are recorded with it.
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
        // Other callers do not even read the receiver.
        observe_building(usize::MAX, spend_site(&image) + 1);
        observe_speed(&mut repairing, 900_000, spend_site(&image));
        // A matching advance counts even when it adds no output.
        observe_speed(&mut idle, 0, spend_site(&image));
        let (result, entries) = drained(8);
        assert!(result.unwrap().complete());
        assert_eq!(
            entries,
            [
                Entry {
                    line: 0x11_0000_003b,
                    amount: 1_850_000,
                    bank: 50_000,
                    ..Entry::default()
                },
                Entry {
                    line: 0x12_0000_003b,
                    amount: 900_000,
                    repair: true,
                    partner: Some(0x7_0000_003b),
                    ..Entry::default()
                },
                Entry {
                    line: 0x13_0000_003b,
                    ..Entry::default()
                },
            ]
        );
        observe_for_test(false, 0);
    }

    /// A railway's spend is its advance's first segment call; the carry to the
    /// next segment is not output, and a call outside an advance is nobody's.
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
                    amount: 1_800_000,
                    railway: true,
                    ..Entry::default()
                },
                Entry {
                    line: 0x21_0000_1269,
                    amount: 300_000,
                    railway: true,
                    carry: true,
                    ..Entry::default()
                }
            ]
        );
        observe_for_test(false, 0);
    }

    /// Under instant construction every spend is recorded as such, even of zero.
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
                // A zero speed must not hide a positive bank draw.
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
        // Native ADD: MAX factory speed plus one bank unit becomes MIN.
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
        let rail = Line::new(&image, true, 0x32_0000_1269, 0, 0, false);
        // Speed < 1: the production advance makes no progress call.
        RAILWAY_LINE.set((rail.address(), 0));
        RAILWAY_LINE.set((0, 0));
        assert!(drained(8).1.is_empty());
        image.set_instant(true);
        RAILWAY_LINE.set((rail.address(), 0));
        for _ in 0..6 {
            observe_segment(10_000_000_000);
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
        let rail = Line::new(&image, true, 0x35_0000_1269, 50_000, 0, false);
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

    /// Release-only incremental game-thread cost: guarded owned-line/global reads,
    /// TLS, ACTIVE, atomic appends, the full drain and empty hourly drains.
    /// No native detour/trampoline or game simulation is timed.
    #[test]
    #[ignore = "synthetic release cost; run alone with --nocapture"]
    fn construction_ic_scale() {
        use std::{
            hint::black_box,
            time::{Duration, Instant},
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetProcessAffinityMask};
        let _lock = test_lock();
        assert_ne!(
            unsafe { SetProcessAffinityMask(GetCurrentProcess(), 0xf) },
            0
        );
        let image = Image::new();
        for (countries, count) in [(1500, 500), (1500, 3000), (5000, 10000), (5000, 20000)] {
            let lines: Vec<_> = (0..count)
                .map(|i| {
                    Line::new(
                        &image,
                        i % 5 == 0,
                        (i as u64 + 1) << 32 | 23,
                        20_000,
                        i32::from(i % 4 == 0),
                        false,
                    )
                })
                .collect();
            let mut output = vec![Entry::default(); count * 2];
            let mut best_day = Duration::MAX;
            let mut best_drain = Duration::MAX;
            let mut sum_hour = 0.0;
            let mut best_hour = f64::INFINITY;
            for _ in 0..20 {
                observe_for_test(true, image.base());
                let start = Instant::now();
                for (i, line) in lines.iter().enumerate() {
                    let _call = OBSERVATION.enter();
                    if i % 5 == 0 {
                        COUNTERS[2].fetch_add(1, Ordering::Relaxed);
                        RAILWAY_LINE.set((line.address(), 0));
                        {
                            let _progress = OBSERVATION.enter();
                            observe_segment(180_000);
                        }
                        if i % 10 == 0 {
                            let _progress = OBSERVATION.enter();
                            observe_segment(30_000);
                        }
                        RAILWAY_LINE.set((0, 0));
                    } else {
                        observe_building(black_box(line.address()), spend_site(&image));
                    }
                }
                let day = start.elapsed();
                let start = Instant::now();
                let d = drain(&mut output).unwrap();
                let full = start.elapsed();
                assert!(d.complete());
                assert_eq!(d.copied, count + count / 10);
                black_box(&output[..d.copied]);
                let start = Instant::now();
                for _ in 0..23 {
                    assert_eq!(drain(&mut output).unwrap().copied, 0);
                }
                let empty = start.elapsed();
                best_day = best_day.min(day);
                best_drain = best_drain.min(full);
                let hour = (day + full + empty).as_secs_f64() * 1000.0 / 24.0;
                sum_hour += hour;
                best_hour = best_hour.min(hour);
            }
            println!(
                "construction_ic_scale countries={countries} lines/day={count} affinity=0xF repeats=20 day_best_ms={:.4} full_drain_best_ms={:.4} hour_best_ms={best_hour:.4} hour_mean_ms={:.4} budget_4core_ms=0.5 log_bytes={} drain_bytes={}",
                best_day.as_secs_f64() * 1000.0,
                best_drain.as_secs_f64() * 1000.0,
                sum_hour / 20.0,
                capacity() * size_of::<Slot>(),
                output.len() * size_of::<Entry>()
            );
        }
        observe_for_test(false, 0);
    }

    /// Short storage defers entries; an active writer defers all; an overflow
    /// restarts past the loss; disabled hooks hand over nothing.
    #[test]
    fn drains_defer_restart_and_refuse_as_the_line_output_log_does() {
        let _lock = test_lock();
        let image = Image::new();
        observe_for_test(true, image.base());
        let mut building = Line::new(&image, false, 0x51_0000_003b, 0, 0, false);
        for amount in 1..=5 {
            observe_speed(&mut building, amount, spend_site(&image));
        }
        let (result, entries) = drained(2);
        let first = result.unwrap();
        assert_eq!((first.copied, first.pending), (2, 5));
        assert_eq!(entries.iter().map(|e| e.amount).collect::<Vec<_>>(), [1, 2]);
        let writer = OBSERVATION.enter();
        assert_eq!(drained(8).0.unwrap().copied, 0);
        drop(writer);
        let (result, entries) = drained(8);
        assert!(result.unwrap().complete());
        assert_eq!(
            entries.iter().map(|e| e.amount).collect::<Vec<_>>(),
            [3, 4, 5]
        );
        for _ in 0..=capacity() {
            observe_speed(&mut building, 1, spend_site(&image));
        }
        let (result, entries) = drained(4);
        let overflowed = result.unwrap();
        assert!(entries.is_empty());
        assert_eq!(overflowed.first, overflowed.cursor);
        ENABLED.store(false, Ordering::SeqCst);
        assert_eq!(drained(4).0, Err("construction line hooks are not enabled"));
        observe_for_test(false, 0);
    }

    #[test]
    fn unrecognised_prologues_leave_the_table_unavailable() {
        let _lock = test_lock();
        let bytes = vec![0_u8; 0x10];
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let hooks = validate_sentinels(&memory, 0);
        assert_eq!(hooks.err(), Some("construction line sentinel differs"));
    }

    #[test]
    fn every_sentinel_is_checked_before_installation() {
        struct Prologues([[u8; 16]; 7]);
        impl Memory for Prologues {
            fn copy(&self, address: usize, out: &mut [u8]) -> bool {
                let Some(index) = SITES.iter().position(|s| s.rva == address) else {
                    return false;
                };
                if out.len() != 16 {
                    return false;
                }
                out.copy_from_slice(&self.0[index]);
                true
            }
        }
        for bad in 0..SITES.len() {
            let mut memory = Prologues(SITES.map(|s| s.sentinel));
            assert!(validate_sentinels(&memory, 0).is_ok());
            memory.0[bad][15] ^= 1;
            assert_eq!(
                validate_sentinels(&memory, 0).unwrap_err(),
                "construction line sentinel differs"
            );
        }
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
