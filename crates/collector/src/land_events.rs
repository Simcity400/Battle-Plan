//! Growing multi-producer event-word log. Detours copy transient inputs only;
//! owned words are decoded off-thread. Failed reads/overflow poison the interval.
#[cfg(all(test, feature = "tap-foreign-probe"))]
mod foreign_unwind;
use crate::{hook::Hook, opinion_changes::Padded};
use engine::{Memory, land_events as native};
use std::{
    cell::Cell,
    ops::Range,
    ptr,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};

struct Storage {
    words: Vec<AtomicU64>,
    _charge: crate::raw_memory::Charge,
}
impl Storage {
    fn new(words: Vec<AtomicU64>, mut charge: crate::raw_memory::Charge) -> Self {
        charge.set(crate::raw_memory::Capacity {
            payload_vectors: crate::raw_memory::vector_bytes(&words),
            ..Default::default()
        });
        Self {
            words,
            _charge: charge,
        }
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
static LOG_CAPACITY: AtomicUsize = AtomicUsize::new(0);
static LOG_REQUIRED: AtomicUsize = AtomicUsize::new(0);
static LOG_SWAPS: AtomicU64 = AtomicU64::new(0);
static RESIZING: AtomicBool = AtomicBool::new(false);

/// Runs `body`, or `diverted` when the calling thread is a test thread
/// outside the land test that owns the process-wide log
/// ([`test_support::own`]): the parallel suite's other modules reach these
/// entry points from their own threads (a full decode queue, a dropped
/// frame, an observer drain), which must not move a land test's epoch,
/// losses or storage. The owner's own threads ([`test_support::adopt`])
/// are never diverted or blocked. Outside tests it only runs `body`.
#[inline(always)]
fn unless_foreign<T>(diverted: impl FnOnce() -> T, body: impl FnOnce() -> T) -> T {
    #[cfg(test)]
    let _entered = match test_support::enter() {
        Some(entered) => entered,
        None => return diverted(),
    };
    #[cfg(not(test))]
    let _ = diverted;
    body()
}

/// Sizing thread: grow from complete attempted-word demand, including records
/// that exceeded the old storage. No population or record ceiling is imposed.
pub fn prepare() {
    unless_foreign(|| (), || engine::allocation::advisory(prepare_storage));
}

fn prepare_storage() {
    let Ok(mut swap) = LOG_SWAP.lock() else {
        return;
    };
    drop(swap.retired.take());
    let required = LOG_REQUIRED.load(Ordering::Relaxed);
    let mut capacity = LOG_CAPACITY.load(Ordering::Acquire);
    if required <= capacity
        || swap
            .next
            .as_ref()
            .is_some_and(|s| s.words.len() >= required)
    {
        return;
    }
    crate::table_capture::grow_capacity(&mut capacity, required);
    if let Ok(mut words) = engine::allocation::vec(capacity) {
        words.resize_with(capacity, || AtomicU64::new(0));
        if let Ok(next) = engine::allocation::boxed(Storage::new(words, Default::default())) {
            swap.next = Some(next);
        }
    }
}

pub(crate) fn presize(words: usize) {
    unless_foreign(
        || (),
        || {
            LOG_REQUIRED.fetch_max(words, Ordering::Relaxed);
            engine::allocation::advisory(prepare_storage);
        },
    );
}

pub(crate) fn capacity() -> usize {
    LOG_REQUIRED.load(Ordering::Relaxed)
}

/// Sole hourly consumer, after draining: offer installation neither allocates
/// nor frees. Producers seeing the swap gate refuse and mark a coverage gap.
pub fn install_offered() {
    unless_foreign(|| (), install_offered_owned);
}
fn install_offered_owned() {
    let Ok(mut swap) = LOG_SWAP.try_lock() else {
        return;
    };
    if swap.next.is_none() || swap.retired.is_some() {
        return;
    }
    RESIZING.store(true, Ordering::SeqCst);
    if PRODUCERS.load(Ordering::SeqCst) == 0
        && CURSOR.load(Ordering::SeqCst) == CONSUMED.load(Ordering::SeqCst)
    {
        let mut next = swap.next.take().unwrap();
        LOG.store(&mut *next, Ordering::SeqCst);
        LOG_CAPACITY.store(next.words.len(), Ordering::Release);
        swap.retired = swap.current.replace(next);
        LOG_SWAPS.fetch_add(1, Ordering::Relaxed);
    }
    RESIZING.store(false, Ordering::SeqCst);
}

/// Off-thread after final drain and hook/IP/callback quiescence. Failed detach
/// retains every current, offered and retired allocation with its charge.
pub fn release() -> Result<(), &'static str> {
    unless_foreign(|| Ok(()), release_owned)
}
fn release_owned() -> Result<(), &'static str> {
    if ENABLED.load(Ordering::SeqCst)
        || ACTIVE.load(Ordering::SeqCst) != 0
        || PRODUCERS.load(Ordering::SeqCst) != 0
        || TAP_IN_FLIGHT.load(Ordering::SeqCst) != 0
    {
        return Err("land event storage release before callback quiescence");
    }
    let buffers = {
        let mut swap = LOG_SWAP.lock().unwrap_or_else(|error| error.into_inner());
        LOG.store(ptr::null_mut(), Ordering::SeqCst);
        LOG_CAPACITY.store(0, Ordering::Release);
        LOG_REQUIRED.store(0, Ordering::Relaxed);
        CONSUMED.store(CURSOR.load(Ordering::SeqCst), Ordering::SeqCst);
        std::mem::take(&mut *swap)
    };
    drop(buffers);
    Ok(())
}

fn log() -> Option<&'static Storage> {
    // Producers increment PRODUCERS before checking RESIZING. The sole consumer
    // replaces an empty log only with that gate set and no producer using it;
    // the spool frees retired storage. Detach quiescence retains active storage.
    unsafe { LOG.load(Ordering::SeqCst).as_ref() }
}
static CURSOR: AtomicU64 = AtomicU64::new(0);
static CONSUMED: AtomicU64 = AtomicU64::new(0);
static EPOCH: AtomicU64 = AtomicU64::new(1);
static OBSERVED_EPOCH: AtomicU64 = AtomicU64::new(0);
static LAST_HOUR: AtomicU64 = AtomicU64::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static TAP_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static TAP_ESCAPED_UNWINDS: AtomicU64 = AtomicU64::new(0);
const TAP_UNWIND_RETENTION: &str = "land verdict resolver escaped unwind; collector retained";
static CALL_ESCAPED_UNWINDS: AtomicU64 = AtomicU64::new(0);
const CALL_UNWIND_RETENTION: &str = "land callback escaped unwind; collector retained";
fn escaped_unwind_reason() -> Option<&'static str> {
    if TAP_ESCAPED_UNWINDS.load(Ordering::SeqCst) != 0 {
        Some(TAP_UNWIND_RETENTION)
    } else if CALL_ESCAPED_UNWINDS.load(Ordering::SeqCst) != 0 {
        Some(CALL_UNWIND_RETENTION)
    } else {
        None
    }
}
pub(crate) fn active() -> usize {
    ACTIVE
        .load(Ordering::SeqCst)
        .saturating_add(TAP_IN_FLIGHT.load(Ordering::SeqCst))
}
fn observing() -> bool {
    ENABLED.load(Ordering::Relaxed) && escaped_unwind_reason().is_none()
}
pub(crate) fn stop_observation() {
    unless_foreign(|| (), || ENABLED.store(false, Ordering::SeqCst));
}
static BASE: AtomicUsize = AtomicUsize::new(0);
pub static ACTIVE: Padded<AtomicUsize> = Padded(AtomicUsize::new(0));
// Hook lifetime protects unhooking; only append producers affect coverage.
static PRODUCERS: Padded<AtomicUsize> = Padded(AtomicUsize::new(0));
static LEADER_CALLS: Padded<AtomicU64> = Padded(AtomicU64::new(0));
const FINALIZE: usize = 0;
const DESTROYED: usize = 1;
const BORDER_FINALIZE: usize = 2;
const BORDER_TERMINATE: usize = 3;
const DISPATCH_ORIGINAL: usize = 4;
const WIPE_ORIGINAL: usize = 5;
const DESTRUCTION_ORIGINAL: usize = 6;
static ORIGINAL: [AtomicUsize; 7] = [const { AtomicUsize::new(0) }; 7];
const LOSS_NAMES: [&str; 145] = [
    "border final inputs unreadable",
    "border final side backlink differs",
    "border final side identity mismatch",
    "border-war finalizer not observed",
    "border-war finalizer verdict unobserved",
    "canonical country slot points to a different object",
    "capture buffer allocation failed",
    "capture buffer length overflow",
    "combat registry unreadable or invalid",
    "combat wipe address overflow",
    "combat wipe class unsupported",
    "combat wipe countries invalid",
    "combat wipe field unreadable",
    "combat wipe game state unreadable",
    "combat wipe null pointer",
    "combat wipe null-country links invalid",
    "combat wipe null-country offset overflow",
    "combat wipe opposite backlink differs",
    "combat wipe pointer identity differs",
    "combat wipe remaining class unsupported",
    "combat wipe remaining offset overflow",
    "combat wipe remaining references invalid",
    "combat wipe side discriminator differs",
    "combat wipe triggering reference was not removed",
    "combat wipe unit class unsupported",
    "country array unreadable or invalid",
    "country id is zero",
    "country id unreadable",
    "country link does not resolve to its canonical object",
    "country link resolves to null country",
    "country link unreadable or outside country table",
    "country links have no sentinel slot",
    "country links unreadable or invalid",
    "country pointer differs from its canonical object",
    "country pointer is null",
    "country pointer unreadable or outside country table",
    "country slot and object resolve to different countries",
    "country table has no country slots",
    "country vtable mismatch",
    "destroyed combat address overflow",
    "destroyed combat offset overflow",
    "destroyed division combat class unsupported",
    "destroyed division combat header invalid",
    "destruction diplomacy owner differs",
    "destruction relation address overflow",
    "destruction relation offset overflow",
    "destruction relation owner differs",
    "destruction relation slot out of bounds",
    "destruction relations invalid",
    "equipment entry unreadable",
    "equipment variant pointer overflow",
    "history country count invalid",
    "history first country null",
    "history raw tag out of bounds",
    "history side backlink differs",
    "history side identity mismatch",
    "history tag array invalid",
    "history tags address overflow",
    "land append escaped unwind",
    "land border finalization flags or sides unreadable",
    "land border finalization identity unavailable",
    "land callback escaped unwind",
    "land casualty snapshot read refused",
    "land combat class unsupported",
    "land decode detach drain panicked",
    "land decode worker panicked",
    "land destruction killer callback unpaired",
    "land destruction returned without killer observation",
    "land dispatch returned without both leader observations",
    "land dispatch sides differ",
    "land event country set invalid",
    "land event drain capacity exceeded",
    "land event field unreadable",
    "land event game state null",
    "land event log capacity exceeded",
    "land event log changed during drain",
    "land event object identity mismatch",
    "land event producer active at boundary",
    "land event record word count differs",
    "land event sizing address overflow",
    "land event sizing combat pointer unreadable",
    "land event sizing combat registry changed",
    "land event sizing division address overflow",
    "land event sizing division pointer unreadable",
    "land event sizing division roster changed",
    "land event sizing division roster invalid",
    "land event sizing overflow",
    "land event storage handoff overlaps producer",
    "land final spool flush failed",
    "land frame conversion failed",
    "land frame decode queue full",
    "land loss arena capacity exceeded",
    "land loss count overflow",
    "land loss entry bytes overflow",
    "land loss equipment address overflow",
    "land loss equipment entries unreadable",
    "land loss event words overflow",
    "land loss observer address overflow",
    "land loss observer header unreadable",
    "land loss observer headers unreadable",
    "land loss observer vtable address overflow",
    "land loss observer vtable mismatch",
    "land loss pool array invalid",
    "land loss pool vtable address overflow",
    "land loss pool vtable mismatch",
    "land loss side backlink differs",
    "land loss side null",
    "land loss sides alias",
    "land loss variant identities unreadable",
    "land loss variant pointer overflow",
    "land loss variant vtable address overflow",
    "land native verdict caller or side invalid",
    "land observer worker drain lost",
    "land observer frame not captured",
    "land prepared spool write failed",
    "land recycled spool write failed",
    "land result combatant blocks unreadable",
    "land result country set changed",
    "land result country set short",
    "land result side backlink mismatch",
    "land result side identity mismatch",
    "land result size overflow",
    "land result unit header invalid",
    "land result unit offset overflow",
    "land verdict resolver escaped unwind",
    "module address overflow",
    "null history tag absent",
    "null history tag array invalid",
    "raw country link observation context mismatch",
    "raw country selecting link controls unobserved",
    "reserved country id is not zero",
    "reserved country module address overflow",
    "reserved country object identity mismatch",
    "reserved country slot absent or unreadable",
    "reserved country slot is null",
    "string bytes unreadable",
    "string header invalid",
    "string header unreadable",
    "string heap pointer invalid",
    "string length overflow",
    "string offset overflow",
    "string terminator unreadable or invalid",
    "variant identity unreadable",
    "variant module address overflow",
    "variant vtable or reference type mismatch",
];
static LOSS_REASONS: [AtomicU64; LOSS_NAMES.len()] =
    [const { AtomicU64::new(0) }; LOSS_NAMES.len()];
static UNLISTED_LOSS: OnceLock<&'static str> = OnceLock::new();
static UNLISTED_LOSSES: AtomicU64 = AtomicU64::new(0);
static LOST: AtomicU64 = AtomicU64::new(0);
// This thread's bumps of `LOST` and `LEADER_CALLS`, in test builds only.
// Both counters are process-wide, and other modules' tests (dropped frames,
// a full decode queue, observer-boundary drains while a land test enables
// observation) bump `LOST` from their own threads while the parallel suite
// runs. A synthetic tap runs its observer body on the calling thread, so a
// land test measures its own bodies with these.
#[cfg(test)]
thread_local! {
    static THREAD_COUNTS: std::cell::Cell<[u64; 2]> = const { std::cell::Cell::new([0; 2]) };
    /// This thread's `EPOCH` advances: other modules' drains and losses
    /// advance the process-wide epoch from their own threads.
    static THREAD_EPOCHS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Starts a new observation epoch, returning the new value.
fn advance_epoch() -> u64 {
    #[cfg(test)]
    THREAD_EPOCHS.with(|epochs| epochs.set(epochs.get().wrapping_add(1)));
    EPOCH.fetch_add(1, Ordering::SeqCst) + 1
}
/// Count `lost` losses and `leader_calls` resolver calls on this thread too
/// (test builds only; see `THREAD_COUNTS`).
#[inline]
fn count_on_thread(lost: u64, leader_calls: u64) {
    #[cfg(test)]
    THREAD_COUNTS.with(|counts| {
        let [previous_lost, previous_calls] = counts.get();
        counts.set([
            previous_lost.wrapping_add(lost),
            previous_calls.wrapping_add(leader_calls),
        ]);
    });
    #[cfg(not(test))]
    let _ = (lost, leader_calls);
}
static EVENTS: AtomicU64 = AtomicU64::new(0);
static HOOK_TICKS: AtomicU64 = AtomicU64::new(0);
static MAX_HOOK_TICKS: AtomicU64 = AtomicU64::new(0);
static DRAIN_CAPACITY_FAILURES: AtomicU64 = AtomicU64::new(0);
static LOG_CAPACITY_FAILURES: AtomicU64 = AtomicU64::new(0);
static WIPE_UNSUPPORTED_CLASSES: [(&str, AtomicU64); 5] = [
    ("combat wipe class unsupported", AtomicU64::new(0)),
    ("combat wipe unit class unsupported", AtomicU64::new(0)),
    ("combat wipe remaining class unsupported", AtomicU64::new(0)),
    ("land combat class unsupported", AtomicU64::new(0)),
    (
        "destroyed division combat class unsupported",
        AtomicU64::new(0),
    ),
];
static DRAIN_CAPACITY: AtomicUsize = AtomicUsize::new(0);
static DRAIN_REQUIRED: AtomicUsize = AtomicUsize::new(0);
static UNHOOKED: std::sync::OnceLock<String> = std::sync::OnceLock::new();

#[derive(Clone, Copy)]
struct Border {
    combat: usize,
    handle: u64,
    sides: [usize; 2],
    flags: [u8; 3],
    terminating: [u8; 2],
}
const NO_BORDER: Border = Border {
    combat: 0,
    handle: 0,
    sides: [0; 2],
    flags: [0; 3],
    terminating: [255; 2],
};
thread_local! {static BORDER:Cell<Border>=const{Cell::new(NO_BORDER)};}
#[derive(Clone, Copy)]
struct Dispatch {
    combat: u64,
    sides: [usize; 2],
    leaders: [u64; 2],
    seen: [bool; 2],
    winner: u8,
}
const NO_DISPATCH: Dispatch = Dispatch {
    combat: 0,
    sides: [0; 2],
    leaders: [0; 2],
    seen: [false; 2],
    winner: 0,
};
thread_local! {static DISPATCH:Cell<Dispatch>=const{Cell::new(NO_DISPATCH)};}
// Round termination returns 1 after dispatch; the combat manager
// removes it in the same update loop. Consume the pointer at
// finalize, paired with its handle, so reuse cannot skip a fallback walk.
thread_local! {static LAST_DISPATCHED:Cell<(usize,u64,u64)>=const{Cell::new((0,0,0))};}
fn take_dispatched(combat: usize, handle: u64) -> bool {
    LAST_DISPATCHED.replace((0, 0, 0)) == (combat, handle, EPOCH.load(Ordering::SeqCst))
}
impl Dispatch {
    fn slot(self, side: usize, caller: usize, base: usize) -> Result<Option<usize>, ()> {
        if self.combat == 0 {
            return Ok(None);
        }
        let Some(role) = native::DISPATCH_LEADER_RETURNS
            .iter()
            .position(|rva| base.checked_add(*rva) == Some(caller))
        else {
            return Ok(None);
        };
        let winner = usize::from(self.winner == 2);
        let n = if role == 0 { 1 - winner } else { winner };
        if self.sides[n] != side || self.seen[n] {
            return Err(());
        }
        Ok(Some(n))
    }
}
impl Border {
    fn slot(self, side: usize, caller: usize, base: usize) -> Result<Option<usize>, ()> {
        if self.combat == 0 {
            return Ok(None);
        }
        let Some(n) = native::BORDER_TERMINATE_RETURNS
            .iter()
            .position(|rva| base.checked_add(*rva) == Some(caller))
        else {
            return Ok(None);
        };
        if self.sides[n] != side || self.terminating[n] != 255 {
            return Err(());
        }
        Ok(Some(n))
    }
}
#[derive(Clone, Copy, Default)]
struct Destruction {
    army: usize,
    from: usize,
    to: usize,
    division: u64,
    entry: u64,
    seen: bool,
    failed: bool,
}
thread_local! {static DESTRUCTION:Cell<Destruction>=const{Cell::new(Destruction{army:0,from:0,to:0,division:0,entry:0,seen:false,failed:false})};}

struct Call {
    completed: Cell<bool>,
    // Drops after Call::drop and records the whole callback.
    timing: crate::hook_timing::Span,
}
impl Call {
    fn enter() -> Self {
        let timing = crate::hook_timing::enter(crate::hook_timing::Family::LandEvents);
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        Self {
            completed: Cell::new(false),
            timing,
        }
    }
    fn finish(&self) {
        self.completed.set(true);
    }
    /// Run the native original, timed apart from this callback when sampled.
    fn original<T>(&self, original: impl FnOnce() -> T) -> T {
        self.timing.original(original)
    }
}
struct Restore<T: Copy + 'static> {
    cell: &'static std::thread::LocalKey<Cell<T>>,
    previous: T,
}
impl<T: Copy + 'static> Restore<T> {
    fn new(cell: &'static std::thread::LocalKey<Cell<T>>) -> Self {
        Self {
            cell,
            previous: cell.get(),
        }
    }
}
impl<T: Copy + 'static> Drop for Restore<T> {
    fn drop(&mut self) {
        self.cell.set(self.previous);
    }
}
impl Drop for Call {
    fn drop(&mut self) {
        // Rust cleanup cannot prove that Windows has finished traversing the DLL.
        if !self.completed.get() || std::thread::panicking() {
            CALL_ESCAPED_UNWINDS.fetch_add(1, Ordering::SeqCst);
            ENABLED.store(false, Ordering::SeqCst);
            fail("land callback escaped unwind");
            return;
        }
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}
pub(crate) fn fail(reason: &'static str) {
    unless_foreign(|| (), || fail_owned(reason));
}
fn fail_owned(reason: &'static str) {
    if let Some(index) = LOSS_NAMES.iter().position(|name| *name == reason) {
        LOSS_REASONS[index].fetch_add(1, Ordering::Relaxed);
    } else {
        let _ = UNLISTED_LOSS.set(reason);
        UNLISTED_LOSSES.fetch_add(1, Ordering::Relaxed);
    }

    count_on_thread(1, 0);
    LOST.fetch_add(1, Ordering::Relaxed);
    advance_epoch();
}

/// Independent hourly read failures count observations, never break event spans.
pub(crate) fn lost_casualty_reads(count: u64) -> u64 {
    unless_foreign(|| count, || lost_casualty_reads_owned(count))
}
fn lost_casualty_reads_owned(count: u64) -> u64 {
    let index = LOSS_NAMES
        .iter()
        .position(|name| *name == "land casualty snapshot read refused")
        .unwrap();
    LOSS_REASONS[index].fetch_add(count, Ordering::Relaxed);
    count_on_thread(count, 0);
    LOST.fetch_add(count, Ordering::Relaxed)
        .saturating_add(count)
}

/// Caller reserved one contiguous logical record. Atomics avoid any data race
/// even when a stalled producer is lapped; PRODUCERS/cursor/epoch checks reject it.
fn append(size: usize, copy: impl FnOnce(&mut dyn FnMut(u64)) -> Result<(), &'static str>) {
    append_at(size, |_, put| copy(put));
}
fn append_at(size: usize, copy: impl FnOnce(u64, &mut dyn FnMut(u64)) -> Result<(), &'static str>) {
    unless_foreign(|| (), || append_owned(size, copy));
}
fn append_owned(
    size: usize,
    copy: impl FnOnce(u64, &mut dyn FnMut(u64)) -> Result<(), &'static str>,
) {
    if escaped_unwind_reason().is_some() {
        return;
    }
    struct Producer {
        uncommitted: bool,
    }
    impl Drop for Producer {
        fn drop(&mut self) {
            if self.uncommitted {
                fail("land append escaped unwind");
            }
            PRODUCERS.fetch_sub(1, Ordering::SeqCst);
        }
    }
    PRODUCERS.fetch_add(1, Ordering::SeqCst);
    let mut producer = Producer { uncommitted: false };
    let first = CURSOR.fetch_add(size as u64, Ordering::SeqCst);
    let consumed = CONSUMED.load(Ordering::SeqCst);
    let required = CURSOR.load(Ordering::SeqCst).wrapping_sub(consumed) as usize;
    LOG_REQUIRED.fetch_max(required.max(size), Ordering::Relaxed);
    if RESIZING.load(Ordering::SeqCst) {
        fail("land event storage handoff overlaps producer");
        return;
    }
    let Some(log) = log().filter(|s| s.words.len() >= required && s.words.len() >= size) else {
        LOG_CAPACITY_FAILURES.fetch_add(1, Ordering::Relaxed);
        fail("land event log capacity exceeded");
        return;
    };
    let capacity = log.words.len() as u64;
    producer.uncommitted = true;
    log.words[(first % capacity) as usize].store(0, Ordering::Relaxed);
    let mut written = 1;
    let result = copy(first, &mut |word| {
        if written < size {
            log.words[(first.wrapping_add(written as u64) % capacity) as usize]
                .store(word, Ordering::Relaxed);
        }
        written += 1;
    });
    producer.uncommitted = false;
    let lapped = CURSOR.load(Ordering::SeqCst).wrapping_sub(first) > capacity;
    if lapped {
        LOG_CAPACITY_FAILURES.fetch_add(1, Ordering::Relaxed);
    }
    if let Err(reason) = result {
        count_class_refusal(reason);
    }
    if result.is_err() || written != size || lapped {
        fail(
            result
                .err()
                .unwrap_or("land event record word count differs"),
        );
        return;
    }
    log.words[(first % capacity) as usize].store(size as u64, Ordering::Release);
    EVENTS.fetch_add(1, Ordering::Relaxed);
}

#[derive(Clone, Copy, Debug)]
pub struct Drain {
    pub epoch: u64,
    pub first: u64,
    pub words: usize,
    pub required: usize,
    pub from_hour: i32,
    pub to_hour: i32,
    pub complete: bool,
    pub lost: u64,
    pub reason: &'static str,
}
/// A first boundary/load/gap discards its unknown prefix. An epoch
/// change, capacity shortage or concurrent producer refuses the whole interval.
/// The next interval can restart; consumers cannot infer its preceding events.
#[cfg(test)]
pub fn drain(output: &mut [u64], hour: i32, invalidating: bool) -> Drain {
    drain_buffer(engine::walk::Buffer::Fixed(output), hour, invalidating)
}
pub(crate) fn drain_growing(output: &mut Vec<u64>, hour: i32, invalidating: bool) -> Drain {
    drain_buffer(engine::walk::Buffer::Growing(output), hour, invalidating)
}
pub(crate) fn lost_drain(hour: i32, reason: &'static str) -> Drain {
    unless_foreign(|| diverted_drain(hour), || lost_drain_owned(hour, reason))
}
fn lost_drain_owned(hour: i32, reason: &'static str) -> Drain {
    fail_owned("land observer worker drain lost");
    let first = CONSUMED.load(Ordering::SeqCst);
    Drain {
        epoch: EPOCH.load(Ordering::SeqCst),
        first,
        words: 0,
        required: CURSOR.load(Ordering::SeqCst).wrapping_sub(first) as usize,
        from_hour: LAST_HOUR.load(Ordering::Relaxed) as i32,
        to_hour: hour,
        complete: false,
        lost: LOST.load(Ordering::Relaxed),
        reason,
    }
}
fn drain_buffer(output: engine::walk::Buffer<'_, u64>, hour: i32, invalidating: bool) -> Drain {
    unless_foreign(
        || diverted_drain(hour),
        || drain_owned(output, hour, invalidating),
    )
}
/// A drain refused to a test thread outside the owning land test.
#[cfg_attr(not(test), allow(dead_code))]
fn diverted_drain(hour: i32) -> Drain {
    Drain {
        epoch: 0,
        first: 0,
        words: 0,
        required: 0,
        from_hour: hour,
        to_hour: hour,
        complete: false,
        lost: 0,
        reason: "land event log owned by another test",
    }
}
fn drain_owned(mut output: engine::walk::Buffer<'_, u64>, hour: i32, invalidating: bool) -> Drain {
    if invalidating {
        advance_epoch();
    }
    let end = CURSOR.load(Ordering::SeqCst);
    let first = CONSUMED.swap(end, Ordering::SeqCst);
    let epoch = EPOCH.load(Ordering::SeqCst);
    let old_epoch = OBSERVED_EPOCH.swap(epoch, Ordering::SeqCst);
    let from = LAST_HOUR.swap(hour as u64, Ordering::Relaxed) as i32;
    let required = end.wrapping_sub(first) as usize;
    LOG_REQUIRED.fetch_max(required, Ordering::Relaxed);
    DRAIN_CAPACITY.store(output.len(), Ordering::Relaxed);
    DRAIN_REQUIRED.store(required, Ordering::Relaxed);
    let mut result = Drain {
        epoch,
        first,
        words: 0,
        required,
        from_hour: from,
        to_hour: hour,
        complete: false,
        lost: LOST.load(Ordering::Relaxed),
        reason: "land event interval not observed",
    };
    if let Some(reason) = escaped_unwind_reason() {
        result.reason = reason;
        return result;
    }
    if !ENABLED.load(Ordering::SeqCst) {
        result.reason = "land event hooks unavailable";
        return result;
    }
    if invalidating || old_epoch == 0 {
        result.reason = "land events before first sample/load/gap unknown";
        return result;
    }
    if from.checked_add(1) != Some(hour) {
        result.epoch = advance_epoch();
        OBSERVED_EPOCH.store(result.epoch, Ordering::SeqCst);
        result.reason = "land event hour unavailable or discontinuous";
        return result;
    }
    if required <= LOG_CAPACITY.load(Ordering::Acquire)
        && let Err(reason) = output.ensure(required)
    {
        fail(reason);
        result.epoch = EPOCH.load(Ordering::SeqCst);
        result.lost = LOST.load(Ordering::Relaxed);
        OBSERVED_EPOCH.store(result.epoch, Ordering::SeqCst);
        result.reason = reason;
        return result;
    }
    DRAIN_CAPACITY.store(output.len(), Ordering::Relaxed);
    if required > LOG_CAPACITY.load(Ordering::Acquire) || required > output.len() {
        let log_overflow = required > LOG_CAPACITY.load(Ordering::Acquire);
        if !log_overflow {
            DRAIN_CAPACITY_FAILURES.fetch_add(1, Ordering::Relaxed);
        }
        // Log shortages were already counted by the refusing append. Drain
        // shortages and the discarded interval are counted here as well.
        fail(if log_overflow {
            "land event log capacity exceeded"
        } else {
            "land event drain capacity exceeded"
        });
        result.epoch = EPOCH.load(Ordering::SeqCst);
        result.lost = LOST.load(Ordering::Relaxed);
        OBSERVED_EPOCH.store(result.epoch, Ordering::SeqCst);
        result.reason = if log_overflow {
            "land event log capacity exceeded"
        } else {
            "land event drain capacity exceeded"
        };
        return result;
    }
    if epoch != old_epoch {
        result.reason = "land event epoch changed after a failed read or overflow";
        return result;
    }
    if PRODUCERS.load(Ordering::SeqCst) != 0 {
        fail("land event producer active at boundary");
        result.epoch = EPOCH.load(Ordering::SeqCst);
        result.lost = LOST.load(Ordering::Relaxed);
        OBSERVED_EPOCH.store(result.epoch, Ordering::SeqCst);
        result.reason = "land event producer active at boundary";
        return result;
    }
    if required > 0 {
        let log = log().unwrap(); // Nonzero capacity was checked above.
        for (n, word) in output[..required].iter_mut().enumerate() {
            *word = log.words[(first.wrapping_add(n as u64) % log.words.len() as u64) as usize]
                .load(Ordering::Acquire);
        }
    }
    if PRODUCERS.load(Ordering::SeqCst) != 0
        || EPOCH.load(Ordering::SeqCst) != epoch
        || CURSOR.load(Ordering::SeqCst) != end
    {
        fail("land event log changed during drain");
        result.epoch = EPOCH.load(Ordering::SeqCst);
        result.lost = LOST.load(Ordering::Relaxed);
        OBSERVED_EPOCH.store(result.epoch, Ordering::SeqCst);
        result.reason = "land event log changed during drain";
        return result;
    }
    result.complete = true;
    result.words = required;
    result
}

fn observe(combat: usize) {
    if !observing() {
        return;
    }
    let start = crate::sampling::qpc();
    let memory = unsafe { engine::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    let border = BORDER.get();
    if border.combat == combat {
        let packed = u64::from(border.flags[0])
            | u64::from(border.flags[1]) << 8
            | u64::from(border.flags[2]) << 16
            | u64::from(border.terminating[0]) << 24
            | u64::from(border.terminating[1]) << 32;
        match native::final_inputs(&memory, base, combat) {
            Ok(inputs) => {
                append(8, |put| {
                    put(2);
                    put(border.handle);
                    put(packed);
                    for word in inputs {
                        put(word);
                    }
                    Ok(())
                });
            }
            Err(reason) => refusal(reason),
        }
    } else {
        match native::ResultPlan::read(&memory, base, combat, None) {
            Ok(plan) => {
                let plan = if take_dispatched(combat, plan.combat()) {
                    plan.without_candidates()
                } else {
                    plan
                };
                append(plan.words, |put| plan.copy(&memory, base, put));
            }
            Err(reason) => refusal(reason),
        }
    }
    timing(start);
}
fn timing(start: i64) {
    let ticks = crate::sampling::qpc().saturating_sub(start).max(0) as u64;
    HOOK_TICKS.fetch_add(ticks, Ordering::Relaxed);
    MAX_HOOK_TICKS.fetch_max(ticks, Ordering::Relaxed);
}
unsafe extern "system-unwind" fn finalize(combat: usize) {
    let _call = Call::enter();
    observe(combat);
    let original: unsafe extern "system-unwind" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL[FINALIZE].load(Ordering::Acquire)) };
    _call.original(|| unsafe { original(combat) });
    if observing() {
        let start = crate::sampling::qpc();
        let memory = unsafe { engine::memory::LiveMemory::new() };
        let base = BASE.load(Ordering::Relaxed);
        match native::LossesPlan::read(&memory, base, combat) {
            Ok(plan) => append(plan.words, |put| plan.copy(&memory, base, put)),
            Err(reason) => refusal(reason),
        }
        match native::HistoryPlan::read(&memory, base, combat) {
            Ok(plan) => append(plan.words, |put| plan.copy(&memory, put)),
            Err(reason) => refusal(reason),
        }
        timing(start);
    }
    _call.finish();
}

// Observation-only base-lifecycle bridge, after land XP/effect callbacks:
// Keep tail-jumping: unit deletion must see the deletion capture's
// COMBAT_DELETION_RETURN through the original caller chain. Calling the
// trampoline would replace that PC and break its combat cause classification.
// Every volatile register and incoming flag is restored before the trampoline;
// Win64 nonvolatile registers are preserved by the Rust observer's ABI. The
// frame's unwind metadata covers the observation CALL, without a native CALL.
core::arch::global_asm!(
    ".text", ".globl land_wipe_entry", ".seh_proc land_wipe_entry",
    "land_wipe_entry:",
    // LEA allocates without changing the incoming flags. RBP describes the
    // body's transient PUSHFQ, and the final LEA/POP/JMP is a Win64 epilogue.
    "push rbp", ".seh_pushreg rbp", "lea rsp, [rsp - 0xd0]", ".seh_stackalloc 0xd0",
    "lea rbp, [rsp + 0xd0]", ".seh_setframe rbp, 0xd0", ".seh_endprologue",
    "mov [rsp + 0x20], rax", "mov [rsp + 0x28], rcx", "mov [rsp + 0x30], rdx",
    "mov [rsp + 0x38], r8", "mov [rsp + 0x40], r9", "mov [rsp + 0x48], r10", "mov [rsp + 0x50], r11",
    "movdqu [rsp + 0x60], xmm0", "movdqu [rsp + 0x70], xmm1", "movdqu [rsp + 0x80], xmm2",
    "movdqu [rsp + 0x90], xmm3", "movdqu [rsp + 0xa0], xmm4", "movdqu [rsp + 0xb0], xmm5",
    "pushfq", "pop rax", "mov [rsp + 0xc0], rax",
    "call {body}",
    "movdqu xmm0, [rsp + 0x60]", "movdqu xmm1, [rsp + 0x70]", "movdqu xmm2, [rsp + 0x80]",
    "movdqu xmm3, [rsp + 0x90]", "movdqu xmm4, [rsp + 0xa0]", "movdqu xmm5, [rsp + 0xb0]",
    "mov rax, [rsp + 0x20]", "mov rcx, [rsp + 0x28]", "mov rdx, [rsp + 0x30]",
    "mov r8, [rsp + 0x38]", "mov r9, [rsp + 0x40]", "mov r10, [rsp + 0x48]", "mov r11, [rsp + 0x50]",
    "push qword ptr [rsp + 0xc0]", "popfq",
    "lea rsp, [rbp]", "pop rbp", "jmp qword ptr [rip + {original} + {offset}]",
    ".seh_endproc", body = sym observe_wipe, original = sym ORIGINAL,
    offset = const WIPE_ORIGINAL * size_of::<AtomicUsize>());
unsafe extern "system-unwind" {
    fn land_wipe_entry();
}
unsafe extern "system-unwind" fn observe_wipe(combatant: usize, army: usize) {
    let _call = Call::enter();
    if !observing() {
        _call.finish();
        return;
    }
    let start = crate::sampling::qpc();
    let memory = unsafe { engine::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    match engine::land_wipe::WipePlan::read(&memory, base, combatant, army) {
        Ok(Some(plan)) => append(plan.words, |put| plan.copy(&memory, base, put)),
        Ok(None) => {}
        Err(reason) => {
            refusal(reason);
        }
    }
    timing(start);
    #[cfg(test)]
    if WIPE_TEST_CLOBBER_XMM.load(Ordering::Relaxed) {
        unsafe {
            core::arch::asm!(
            "pxor xmm0, xmm0", "pxor xmm1, xmm1", "pxor xmm2, xmm2",
            "pxor xmm3, xmm3", "pxor xmm4, xmm4", "pxor xmm5, xmm5",
            out("xmm0") _, out("xmm1") _, out("xmm2") _,
            out("xmm3") _, out("xmm4") _, out("xmm5") _,
            options(nomem, nostack, preserves_flags));
        }
    }
    _call.finish();
}
#[cfg(test)]
static WIPE_TEST_CLOBBER_XMM: AtomicBool = AtomicBool::new(false);
fn refusal(reason: &'static str) {
    count_class_refusal(reason);
    fail(reason);
}
fn count_class_refusal(reason: &str) {
    if let Some((_, count)) = WIPE_UNSUPPORTED_CLASSES
        .iter()
        .find(|(name, _)| *name == reason)
    {
        count.fetch_add(1, Ordering::Relaxed);
    }
}
unsafe extern "system-unwind" fn destroyed(
    from: usize,
    to: usize,
    army: usize,
    pool: usize,
    killer: usize,
) {
    let _call = Call::enter();
    if observing() {
        let start = crate::sampling::qpc();
        let memory = unsafe { engine::memory::LiveMemory::new() };
        let base = BASE.load(Ordering::Relaxed);
        let mut context = DESTRUCTION.get();
        let matches = context.army == army && context.from == from && context.to == to;
        // A failed outer read already counted this observation.
        if !(matches && context.failed) {
            if matches && context.division != 0 && !context.seen {
                match native::killer_handle(&memory, base, killer) {
                    Ok(handle) => append(4, |put| {
                        put(6);
                        put(context.entry);
                        put(handle);
                        Ok(())
                    }),
                    Err(reason) => refusal(reason),
                }
                context.seen = true;
                DESTRUCTION.set(context);
            } else {
                fail("land destruction killer callback unpaired");
                if matches {
                    context.failed = true;
                    DESTRUCTION.set(context);
                }
            }
        }
        timing(start);
    }
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[DESTROYED].load(Ordering::Acquire)) };
    _call.original(|| unsafe { original(from, to, army, pool, killer) });
    _call.finish();
}
unsafe extern "system-unwind" fn destruction(army: usize, from: usize, to: usize) {
    let _call = Call::enter();
    let _restore = Restore::new(&DESTRUCTION);
    if observing() {
        let start = crate::sampling::qpc();
        let memory = unsafe { engine::memory::LiveMemory::new() };
        let base = BASE.load(Ordering::Relaxed);
        match native::DestructionPlan::read(&memory, base, from, to, army, 0) {
            Ok(plan) => {
                DESTRUCTION.set(Destruction {
                    army,
                    from,
                    to,
                    division: plan.division(),
                    entry: 0,
                    seen: false,
                    failed: false,
                });
                // Destruction callbacks run before deferred deletion. One division can
                // enter again; pair this native invocation by its reserved
                // record, not its division handle.
                append_at(plan.words, |first, put| {
                    let mut context = DESTRUCTION.get();
                    context.entry = first;
                    DESTRUCTION.set(context);
                    plan.copy(&memory, base, put)
                });
            }
            Err(reason) => {
                refusal(reason);
                DESTRUCTION.set(Destruction {
                    army,
                    from,
                    to,
                    failed: true,
                    ..Default::default()
                });
            }
        }
        timing(start);
    }
    let original: unsafe extern "system-unwind" fn(usize, usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[DESTRUCTION_ORIGINAL].load(Ordering::Acquire)) };
    _call.original(|| unsafe { original(army, from, to) });
    let start = crate::sampling::qpc();
    if observing() && !DESTRUCTION.get().seen && !DESTRUCTION.get().failed {
        fail("land destruction returned without killer observation");
    }
    timing(start);
    _call.finish();
}
unsafe extern "system-unwind" fn border_finalize(combat: usize) {
    let _call = Call::enter();
    let start = crate::sampling::qpc();
    let _restore = Restore::new(&BORDER);
    if observing() {
        let memory = unsafe { engine::memory::LiveMemory::new() };
        let fields =
            engine::read::<[u8; 3]>(&memory, combat + engine::cland_border_war_combat::END_FLAGS)
                .zip(engine::read::<[u8; 16]>(
                    &memory,
                    combat + engine::cland_combat::ATTACKER,
                ));
        if let Some((flags, sides)) = fields {
            BORDER.set(Border {
                combat,
                handle: engine::read::<u64>(&memory, combat + engine::cland_combat::HANDLE)
                    .unwrap_or(0),
                sides: [
                    u64::from_le_bytes(sides[..8].try_into().unwrap()) as usize,
                    u64::from_le_bytes(sides[8..].try_into().unwrap()) as usize,
                ],
                flags,
                terminating: [255; 2],
            });
        } else {
            fail("land border finalization flags or sides unreadable");
            BORDER.set(NO_BORDER);
        }
    }
    if observing() {
        let memory = unsafe { engine::memory::LiveMemory::new() };
        let base = BASE.load(Ordering::Relaxed);
        let border = BORDER.get();
        if border.combat == combat && border.handle != 0 {
            match native::ResultPlan::read(&memory, base, combat, Some((border.flags, [255; 2]))) {
                Ok(plan) => {
                    let plan = if take_dispatched(combat, plan.combat()) {
                        plan.without_candidates()
                    } else {
                        plan
                    };
                    append(plan.words, |put| plan.copy(&memory, base, put));
                }
                Err(reason) => refusal(reason),
            }
        } else {
            fail("land border finalization identity unavailable");
        }
    }
    timing(start);
    let original: unsafe extern "system-unwind" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL[BORDER_FINALIZE].load(Ordering::Acquire)) };
    _call.original(|| unsafe { original(combat) });
    _call.finish();
}
// Single-argument Win64 entries use the free RDX register to retain the
// game's return address before a Rust prologue. Tail jumps preserve the stack
// and original return; the body calls the trampoline with native arguments.
#[unsafe(naked)]
unsafe extern "system-unwind" fn border_terminate_entry() {
    core::arch::naked_asm!("mov rdx, qword ptr [rsp]", "jmp {body}", body = sym border_terminate);
}
// CALL relay leaves site+5 on both native and CET stacks. Keep its padding
// until the target has returned. The counter covers the body/native target;
// suspended-IP checks cover the relay and the stub before/after the counter.
// The 0x28 frame supplies Win64 home space/alignment and SEH unwind metadata.
core::arch::global_asm!(
    ".text", ".globl leader_tap_entry", ".seh_proc leader_tap_entry",
    "leader_tap_entry:", "sub rsp, 0x28", ".seh_stackalloc 0x28", ".seh_endprologue",
    "mov r8, qword ptr [rax + 0xe8]",
    "lock inc qword ptr [rip + {counter}]", "mov rdx, qword ptr [rsp + 0x28]",
    "lea rdx, [rdx + 1]", "call {body}",
    "lock dec qword ptr [rip + {counter}]", "add rsp, 0x28", "ret",
    ".seh_endproc", ".globl leader_tap_entry_end", "leader_tap_entry_end:",
    counter = sym TAP_IN_FLIGHT, body = sym leader_tap_body);
unsafe extern "system-unwind" {
    fn leader_tap_entry();
    fn leader_tap_entry_end();
}
fn tap_safety() -> crate::hook::TapSafety {
    crate::hook::TapSafety {
        in_flight: &TAP_IN_FLIGHT,
        stub: leader_tap_entry as *const () as usize..leader_tap_entry_end as *const () as usize,
    }
}
unsafe extern "system-unwind" fn leader_tap_body(
    side: usize,
    caller: usize,
    target: usize,
) -> usize {
    struct UnwindTap(bool);
    impl Drop for UnwindTap {
        fn drop(&mut self) {
            if self.0 {
                // ASM still owns the native return/stub unwind after this
                // Rust frame. Only its normal return may retire that flight.
                TAP_ESCAPED_UNWINDS.fetch_add(1, Ordering::SeqCst);
                ENABLED.store(false, Ordering::SeqCst);
                fail("land verdict resolver escaped unwind");
            }
        }
    }
    // No proved completion witness follows an escaped unwind. Retain the
    // ASM-owned flight across load/detach; bounded teardown pins the collector.
    let mut unwind = UnwindTap(true);
    let result = unsafe { leader_tap(side, caller, target) };
    unwind.0 = false;
    result
}
unsafe extern "system-unwind" fn border_terminate(side: usize, caller: usize) -> u8 {
    let _call = Call::enter();
    let original: unsafe extern "system-unwind" fn(usize) -> u8 =
        unsafe { std::mem::transmute(ORIGINAL[BORDER_TERMINATE].load(Ordering::Acquire)) };
    let result = _call.original(|| unsafe { original(side) });
    let start = crate::sampling::qpc();
    let mut border = BORDER.get();
    if observing() {
        match border.slot(side, caller, BASE.load(Ordering::Relaxed)) {
            Ok(Some(n)) => {
                border.terminating[n] = result;
                BORDER.set(border);
            }
            Ok(None) => {}
            Err(()) => fail("land native verdict caller or side invalid"),
        }
    }
    timing(start);
    _call.finish();
    result
}

unsafe extern "system-unwind" fn dispatch(combat: usize, loser: usize, winner: usize) {
    let _call = Call::enter();
    let start = crate::sampling::qpc();
    let _restore = Restore::new(&DISPATCH);
    if observing() {
        let memory = unsafe { engine::memory::LiveMemory::new() };
        match native::dispatch_inputs(&memory, BASE.load(Ordering::Relaxed), combat, loser, winner)
        {
            Ok((header, winner)) => {
                DISPATCH.set(Dispatch {
                    combat: header.handle(),
                    sides: header.sides(),
                    winner,
                    ..NO_DISPATCH
                });
                let verdict = if header.border_war() {
                    Some(([0; 3], [255; 2]))
                } else {
                    None
                };
                match native::ResultPlan::read_header(
                    &memory,
                    BASE.load(Ordering::Relaxed),
                    header,
                    verdict,
                ) {
                    Ok(plan) => {
                        let plan = plan.participants();
                        append(plan.words, |put| {
                            plan.copy(&memory, BASE.load(Ordering::Relaxed), put)
                        });
                    }
                    Err(reason) => refusal(reason),
                }
            }
            Err(reason) => {
                refusal(reason);
                DISPATCH.set(NO_DISPATCH);
            }
        }
    }
    timing(start);
    let original: unsafe extern "system-unwind" fn(usize, usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[DISPATCH_ORIGINAL].load(Ordering::Acquire)) };
    _call.original(|| unsafe { original(combat, loser, winner) });
    let start = crate::sampling::qpc();
    let observed = DISPATCH.get();
    if observing() {
        if observed.combat != 0 && observed.seen == [true; 2] {
            LAST_DISPATCHED.set((combat, observed.combat, EPOCH.load(Ordering::SeqCst)));
            append(6, |put| {
                put(4);
                put(observed.combat);
                put(observed.leaders[0]);
                put(observed.leaders[1]);
                put(u64::from(observed.winner));
                Ok(())
            });
        } else {
            fail("land dispatch returned without both leader observations");
        }
    }
    timing(start);
    _call.finish();
}
unsafe extern "system-unwind" fn leader_tap(side: usize, caller: usize, target: usize) -> usize {
    let _call = Call::enter();
    count_on_thread(0, 1);
    LEADER_CALLS.fetch_add(1, Ordering::Relaxed);
    let result = unsafe { leader_observation(&_call, side, caller, target) };
    _call.finish();
    result
}
unsafe fn leader_observation(call: &Call, side: usize, caller: usize, target: usize) -> usize {
    let original: unsafe extern "system-unwind" fn(usize) -> usize =
        unsafe { std::mem::transmute(target) };
    let result = call.original(|| unsafe { original(side) });
    let mut observed = DISPATCH.get();
    if observing() && observed.combat != 0 {
        let start = crate::sampling::qpc();
        match observed.slot(side, caller, BASE.load(Ordering::Relaxed)) {
            Ok(Some(n)) => {
                let memory = unsafe { engine::memory::LiveMemory::new() };
                match native::leader_reference(&memory, BASE.load(Ordering::Relaxed), result) {
                    Ok(handle) => {
                        observed.leaders[n] = handle;
                        observed.seen[n] = true;
                        DISPATCH.set(observed);
                    }
                    Err(reason) => refusal(reason),
                }
            }
            Ok(None) => {}
            Err(()) => fail("land native verdict caller or side invalid"),
        }
        timing(start);
    }
    result
}

pub struct Hooks {
    hooks: Vec<Hook>,
    refused: bool,
    publication_attempted: bool,
}
impl Hooks {
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Self {
        let hooks = unsafe { Self::install(memory, base) }.unwrap_or_else(|reason| {
            let _ = UNHOOKED.set(reason);
            Vec::new()
        });
        Self {
            refused: hooks.is_empty(),
            hooks,
            publication_attempted: false,
        }
    }
    unsafe fn install(memory: &impl Memory, base: usize) -> Result<Vec<Hook>, String> {
        let sites = [
            (
                FINALIZE,
                native::FINALIZE_RVA,
                native::FINALIZE_SENTINEL,
                finalize as *const () as usize,
            ),
            (
                DESTROYED,
                native::DESTROY_RVA,
                native::DESTROY_SENTINEL,
                destroyed as *const () as usize,
            ),
            (
                BORDER_FINALIZE,
                native::BORDER_FINALIZE_RVA,
                native::BORDER_FINALIZE_SENTINEL,
                border_finalize as *const () as usize,
            ),
            (
                BORDER_TERMINATE,
                native::BORDER_TERMINATE_RVA,
                native::BORDER_TERMINATE_SENTINEL,
                border_terminate_entry as *const () as usize,
            ),
            (
                DISPATCH_ORIGINAL,
                native::DISPATCH_RVA,
                native::DISPATCH_SENTINEL,
                dispatch as *const () as usize,
            ),
            (
                DESTRUCTION_ORIGINAL,
                native::DESTRUCTION_RVA,
                native::DESTRUCTION_SENTINEL,
                destruction as *const () as usize,
            ),
            (
                WIPE_ORIGINAL,
                engine::land_wipe::LIFECYCLE_RVA,
                engine::land_wipe::LIFECYCLE_SENTINEL,
                land_wipe_entry as *const () as usize,
            ),
        ];
        for (_, rva, sentinel, _) in sites {
            if engine::read::<[u8; 16]>(memory, base + rva) != Some(sentinel) {
                return Err(format!("land event sentinel differs at RVA {rva:x}"));
            }
        }
        let tap_sentinels = verify_leader_taps(memory, base)?;
        BASE.store(base, Ordering::Relaxed);
        let mut hooks = Vec::new();
        for (slot, rva, sentinel, callback) in sites {
            let hook = unsafe { Hook::new(base + rva, callback, sentinel) }?;
            ORIGINAL[slot].store(hook.trampoline(), Ordering::Release);
            hooks.push(hook);
        }
        for (rva, sentinel) in native::DISPATCH_LEADER_CALLS.into_iter().zip(tap_sentinels) {
            hooks.push(unsafe {
                Hook::new_indirect_call(
                    base + rva,
                    leader_tap_entry as *const () as usize,
                    sentinel,
                    tap_safety(),
                )
            }?);
        }
        Ok(hooks)
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        self.publication_attempted |= enabled;
        ENABLED.store(false, Ordering::SeqCst);
        advance_epoch();
        if let Some(reason) = escaped_unwind_reason().filter(|_| enabled) {
            return Err(reason.into());
        }
        if enabled && self.refused {
            return Ok(true);
        }
        match Hook::set_family_enabled(&mut self.hooks, enabled) {
            Ok(false) => return Ok(false),
            Err(reason) if enabled && reason == crate::hook::BYTE_VERIFICATION_FAILURE => {
                // Preflight certified every member disabled/flushed without writing.
                self.refused = true;
                let _ = UNHOOKED.set(format!("land hook family enable refused: {reason}"));
                return Ok(true);
            }
            Err(reason) => return Err(reason),
            Ok(true) => {}
        }
        #[cfg(test)]
        tests::escape_after_publication();
        ENABLED.store(enabled && self.hooks.len() == 9, Ordering::SeqCst);
        // Admission also checks the sticky latch: an escape racing this store
        // cannot publish observations before this post-publication refusal.
        if let Some(reason) = escaped_unwind_reason().filter(|_| enabled) {
            ENABLED.store(false, Ordering::SeqCst);
            return Err(reason.into());
        }
        Ok(true)
    }
    pub fn executable_ranges(&self) -> Vec<Range<usize>> {
        self.hooks
            .iter()
            .flat_map(Hook::executable_ranges)
            .chain(std::iter::once(tap_safety().stub))
            .collect()
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // Before the first publication attempt there can be no native writer.
        if !self.publication_attempted {
            let _ = release();
        }
    }
}
pub fn status() -> impl serde::Serialize {
    crate::status::status!({
        "enabled": observing(),
        "unhooked": UNHOOKED.get(),
        "epoch": EPOCH.load(Ordering::Relaxed),
        "events": EVENTS.load(Ordering::Relaxed),
        "lost_events": LOST.load(Ordering::Relaxed),
        "loss_reasons": crate::status::map(LOSS_NAMES.into_iter().zip(LOSS_REASONS.iter().map(|count| count.load(Ordering::Relaxed))).filter(|(_, count)| *count != 0)),
        "first_unlisted_loss_reason": UNLISTED_LOSS.get().copied(),
        "unlisted_loss_reasons": UNLISTED_LOSSES.load(Ordering::Relaxed),
        "leader_resolver_calls": LEADER_CALLS.load(Ordering::Relaxed),
        "verdict_taps_in_flight": TAP_IN_FLIGHT.load(Ordering::SeqCst),
        "verdict_tap_escaped_unwinds": TAP_ESCAPED_UNWINDS.load(Ordering::SeqCst),
        "verdict_tap_retention_reason": (TAP_ESCAPED_UNWINDS.load(Ordering::SeqCst) != 0).then_some(TAP_UNWIND_RETENTION),
        "callback_escaped_unwinds": CALL_ESCAPED_UNWINDS.load(Ordering::SeqCst),
        "callback_retention_reason": (CALL_ESCAPED_UNWINDS.load(Ordering::SeqCst) != 0).then_some(CALL_UNWIND_RETENTION),
        "hook_ticks": HOOK_TICKS.load(Ordering::Relaxed),
        "max_hook_ticks": MAX_HOOK_TICKS.load(Ordering::Relaxed),
        "log_words": LOG_CAPACITY.load(Ordering::Acquire),
        "log_required_words": LOG_REQUIRED.load(Ordering::Relaxed),
        "log_swaps": LOG_SWAPS.load(Ordering::Relaxed),
        "drain_words": DRAIN_CAPACITY.load(Ordering::Relaxed),
        "drain_required_words": DRAIN_REQUIRED.load(Ordering::Relaxed),
        "drain_capacity_failures": DRAIN_CAPACITY_FAILURES.load(Ordering::Relaxed),
        "log_capacity_failures": LOG_CAPACITY_FAILURES.load(Ordering::Relaxed),
        "wipe_unsupported_classes": {
            "combat wipe class unsupported": WIPE_UNSUPPORTED_CLASSES[0].1.load(Ordering::Relaxed),
            "combat wipe unit class unsupported": WIPE_UNSUPPORTED_CLASSES[1].1.load(Ordering::Relaxed),
            "combat wipe remaining class unsupported": WIPE_UNSUPPORTED_CLASSES[2].1.load(Ordering::Relaxed),
        },
        "land_unsupported_classes": {
            "land combat class unsupported": WIPE_UNSUPPORTED_CLASSES[3].1.load(Ordering::Relaxed),
            "destroyed division combat class unsupported": WIPE_UNSUPPORTED_CLASSES[4].1.load(Ordering::Relaxed),
        },
        "scope": "cumulative since attach; land_unsupported_classes counts named combat-class refusals, included in lost_events; wipe_unsupported_classes counts named unsupported-class refusals from wipe reads and copies, included in lost_events, not all read failures; leader_resolver_calls counts only the two DispatchLeaderOnActions verdict call-site tap hits (loser and winner), not UI/AI/script resolver calls; hook ticks are QPC observation work, decoding is off-thread; first sample, loads, gaps and failed intervals are unknown"
    })
}

fn verify_leader_taps(memory: &impl Memory, base: usize) -> Result<[[u8; 16]; 2], String> {
    let mut sentinels = [[0; 16]; 2];
    for (n, &rva) in native::DISPATCH_LEADER_CALLS.iter().enumerate() {
        let sentinel = engine::read::<[u8; 16]>(memory, base + rva)
            .filter(|bytes| bytes[..6] == native::DISPATCH_LEADER_CALL_SENTINEL)
            .ok_or_else(|| {
                format!(
                    "land event call tap {} sentinel differs at RVA {rva:x}",
                    native::DISPATCH_LEADER_CALL_NAMES[n]
                )
            })?;
        sentinels[n] = sentinel;
    }
    Ok(sentinels)
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    pub(super) static EXCLUSIVE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    /// Held (written) by the owning land test; every other thread's entry
    /// holds it shared for the call, or is diverted while it is owned.
    static GATE: std::sync::RwLock<()> = std::sync::RwLock::new(());
    /// The owning land test's generation, zero while none owns the log.
    static OWNER: AtomicU64 = AtomicU64::new(0);
    static GENERATIONS: AtomicU64 = AtomicU64::new(0);
    thread_local! {
        /// The owning generation this thread belongs to.
        static OWNED: Cell<u64> = const { Cell::new(0) };
    }

    /// A land test's ownership of the process-wide log: the land fixtures'
    /// lock and the gate, which other threads' entries finish before it is
    /// granted and are diverted from while it is held.
    pub struct Owned {
        _gate: std::sync::RwLockWriteGuard<'static, ()>,
        _exclusive: std::sync::MutexGuard<'static, ()>,
    }
    impl Drop for Owned {
        fn drop(&mut self) {
            OWNER.store(0, Ordering::SeqCst);
            OWNED.with(|owned| owned.set(0));
        }
    }

    /// Owns the land log for this thread and the threads it [`adopt`]s.
    pub fn own() -> Owned {
        let exclusive = EXCLUSIVE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let gate = GATE
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = GENERATIONS.fetch_add(1, Ordering::SeqCst) + 1;
        OWNED.with(|owned| owned.set(generation));
        OWNER.store(generation, Ordering::SeqCst);
        Owned {
            _gate: gate,
            _exclusive: exclusive,
        }
    }

    /// A thread the owning land test started: it reaches the log as the
    /// owner does.
    pub fn adopt() {
        OWNED.with(|owned| owned.set(OWNER.load(Ordering::SeqCst)));
    }

    /// How the calling thread enters the log: as the owner, sharing the
    /// gate while no test owns it, or `None` (diverted) while another does.
    pub(super) enum Entered {
        Owner,
        Shared(#[allow(dead_code)] std::sync::RwLockReadGuard<'static, ()>),
    }
    pub(super) fn enter() -> Option<Entered> {
        let owner = OWNER.load(Ordering::SeqCst);
        if owner != 0 && OWNED.with(Cell::get) == owner {
            return Some(Entered::Owner);
        }
        match GATE.try_read() {
            Ok(shared) => Some(Entered::Shared(shared)),
            Err(std::sync::TryLockError::Poisoned(shared)) => {
                Some(Entered::Shared(shared.into_inner()))
            }
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }

    /// The land fixtures' ownership, after the process-wide patching test
    /// lock: these tests publish patches on code pages other modules'
    /// patching tests share, and a panic in one must not poison the rest.
    pub fn exclusive() -> (std::sync::MutexGuard<'static, ()>, Owned) {
        let family = crate::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (family, own())
    }
    pub fn reset() {
        reset_with_words(1024); // Synthetic fixture storage, no production floor.
    }
    pub fn reset_with_words(words: usize) {
        let mut swap = LOG_SWAP.lock().unwrap();
        swap.next = None;
        swap.retired = None;
        let mut storage = Box::new(Storage::new(
            (0..words).map(|_| AtomicU64::new(0)).collect(),
            Default::default(),
        ));
        LOG.store(&mut *storage, Ordering::SeqCst);
        LOG_CAPACITY.store(words, Ordering::SeqCst);
        LOG_REQUIRED.store(0, Ordering::SeqCst);
        LOG_SWAPS.store(0, Ordering::SeqCst);
        RESIZING.store(false, Ordering::SeqCst);
        swap.current = Some(storage);
        CURSOR.store(0, Ordering::SeqCst);
        CONSUMED.store(0, Ordering::SeqCst);
        EPOCH.store(1, Ordering::SeqCst);
        OBSERVED_EPOCH.store(0, Ordering::SeqCst);
        LAST_HOUR.store(0, Ordering::SeqCst);
        LOST.store(0, Ordering::SeqCst);
        ACTIVE.store(0, Ordering::SeqCst);
        CALL_ESCAPED_UNWINDS.store(0, Ordering::SeqCst);
        PRODUCERS.store(0, Ordering::SeqCst);
        LAST_DISPATCHED.set((0, 0, 0));
        DRAIN_CAPACITY_FAILURES.store(0, Ordering::SeqCst);
        LOG_CAPACITY_FAILURES.store(0, Ordering::SeqCst);
        for (_, count) in &WIPE_UNSUPPORTED_CLASSES {
            count.store(0, Ordering::SeqCst);
        }
        ENABLED.store(true, Ordering::SeqCst);
    }
    pub fn append_record(payload: &[u64]) {
        append(payload.len() + 1, |put| {
            for &word in payload {
                put(word);
            }
            Ok(())
        });
    }
    pub fn disable() {
        ENABLED.store(false, Ordering::SeqCst);
    }
    /// This thread's losses (`LOST` bumps), which no other test's thread moves.
    pub fn thread_lost() -> u64 {
        THREAD_COUNTS.with(|counts| counts.get()[0])
    }
    /// This thread's verdict-resolver tap calls (`LEADER_CALLS` bumps).
    pub fn thread_leader_calls() -> u64 {
        THREAD_COUNTS.with(|counts| counts.get()[1])
    }
    /// This thread's `EPOCH` advances, which no other test's thread moves.
    pub fn thread_epochs() -> u64 {
        THREAD_EPOCHS.with(|epochs| epochs.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_support::{exclusive, reset};

    #[test]
    fn worker_drain_keeps_first_and_later_mod_sized_five_thousand_country_words() {
        let _guard = exclusive();
        test_support::reset_with_words(36_000);
        let mut words = Vec::new();
        assert!(!drain_growing(&mut words, 9, true).complete);
        for (hour, count) in [(10, 5000), (11, 12_000)] {
            for index in 0..count {
                test_support::append_record(&[1, (index % 5000) as u64]);
            }
            let drained = drain_growing(&mut words, hour, false);
            assert!(drained.complete, "{}", drained.reason);
            assert_eq!(drained.words, count * 3);
            assert_eq!(
                &words[drained.words - 3..drained.words],
                &[3, 1, ((count - 1) % 5000) as u64]
            );
            assert_eq!(DRAIN_CAPACITY_FAILURES.load(Ordering::Relaxed), 0);
        }
        test_support::disable();
    }
    static ESCAPE_AFTER_PUBLICATION: AtomicBool = AtomicBool::new(false);
    pub(super) fn escape_after_publication() {
        if !ESCAPE_AFTER_PUBLICATION.swap(false, Ordering::SeqCst) {
            return;
        }
        unsafe extern "system-unwind" fn unwind(_: usize) -> usize {
            panic!("direct synthetic enable-race unwind");
        }
        // No ASM frame exists in this deterministic publication interleaving.
        TAP_IN_FLIGHT.store(1, Ordering::SeqCst);
        assert!(
            std::panic::catch_unwind(|| unsafe {
                leader_tap_body(0, 0, unwind as *const () as usize)
            })
            .is_err()
        );
    }
    #[test]
    fn read_losses_and_publication_losses_keep_separate_named_counts() {
        let _guard = exclusive();
        reset();
        let before = test_support::thread_lost();
        crate::test_support::with_allocations_denied(|| {
            refusal("land event field unreadable");
            fail("land frame decode queue full");
        });
        assert_eq!(test_support::thread_lost(), before + 2);
        // Another thread's loss, like another module's test in the parallel
        // suite, moves the process-wide count but not this thread's.
        let global = LOST.load(Ordering::Relaxed);
        let calls = test_support::thread_leader_calls();
        std::thread::spawn(|| {
            test_support::adopt();
            fail("land frame decode queue full")
        })
        .join()
        .unwrap();
        assert!(LOST.load(Ordering::Relaxed) > global);
        assert_eq!(test_support::thread_lost(), before + 2);
        assert_eq!(test_support::thread_leader_calls(), calls);
        let value = serde_json::to_value(status()).unwrap();
        assert!(
            value["loss_reasons"]["land event field unreadable"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert!(
            value["loss_reasons"]["land frame decode queue full"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(value["log_capacity_failures"], 0);
        ENABLED.store(false, Ordering::SeqCst);
    }

    #[test]
    fn escaped_resolver_racing_final_enable_cannot_reopen_observation() {
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc, VirtualFree,
        };
        let _guard = exclusive();
        reset();
        let escaped = TAP_ESCAPED_UNWINDS.load(Ordering::SeqCst);
        assert_eq!(TAP_IN_FLIGHT.load(Ordering::SeqCst), 0);
        let page = unsafe {
            VirtualAlloc(
                ptr::null(),
                4096,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            )
        }
        .cast::<u8>();
        assert!(!page.is_null());
        let sentinel = [0x90; 16];
        let mut hooks = Vec::new();
        for index in 0..9 {
            let site = unsafe { page.add(index * 32) };
            unsafe {
                ptr::copy_nonoverlapping(sentinel.as_ptr(), site, sentinel.len());
            }
            hooks.push(
                unsafe { Hook::new(site as usize, finalize as *const () as usize, sentinel) }
                    .unwrap(),
            );
        }
        let mut group = Hooks {
            hooks,
            refused: false,
            publication_attempted: false,
        };
        ESCAPE_AFTER_PUBLICATION.store(true, Ordering::SeqCst);
        assert_eq!(group.set_enabled(true), Err(TAP_UNWIND_RETENTION.into()));
        assert!(!ENABLED.load(Ordering::SeqCst));
        // Even a stale concurrent enabled write cannot admit a new producer.
        ENABLED.store(true, Ordering::SeqCst);
        assert!(!observing());
        let cursor = CURSOR.load(Ordering::SeqCst);
        append(2, |_| {
            panic!("retained unwind must refuse before reading payload")
        });
        assert_eq!(CURSOR.load(Ordering::SeqCst), cursor);
        ENABLED.store(false, Ordering::SeqCst);
        // The direct synthetic observer has fully unwound; no ASM frame was
        // entered. Retire only that test count before restoring these fixtures.
        assert_eq!(ACTIVE.load(Ordering::SeqCst), 1);
        assert_eq!(CALL_ESCAPED_UNWINDS.load(Ordering::SeqCst), 1);
        TAP_IN_FLIGHT.store(0, Ordering::SeqCst);
        TAP_ESCAPED_UNWINDS.store(escaped, Ordering::SeqCst);
        ACTIVE.store(0, Ordering::SeqCst);
        CALL_ESCAPED_UNWINDS.store(0, Ordering::SeqCst);
        assert!(group.set_enabled(false).unwrap());
        drop(group);
        release().unwrap();
        assert_ne!(unsafe { VirtualFree(page.cast(), 0, MEM_RELEASE) }, 0);
    }
    #[test]
    fn event_storage_charge_follows_offers_and_quiet_release() {
        let _guard = exclusive();
        reset();
        test_support::disable();
        release().unwrap();
        static TRACKER: crate::raw_memory::Tracker = crate::raw_memory::Tracker::new();
        let buffer = |size| {
            Box::new(Storage::new(
                (0..size).map(|_| AtomicU64::new(0)).collect(),
                crate::raw_memory::Charge::empty_on(&TRACKER),
            ))
        };
        let mut swap = LOG_SWAP.lock().unwrap();
        swap.current = Some(buffer(4));
        swap.next = Some(buffer(8));
        swap.retired = Some(buffer(2));
        LOG.store(swap.current.as_deref_mut().unwrap(), Ordering::SeqCst);
        drop(swap);
        assert_eq!(TRACKER.snapshot().current.payload_vectors, 14 * 8);
        ACTIVE.store(1, Ordering::SeqCst);
        assert!(release().is_err());
        assert_eq!(TRACKER.snapshot().current.payload_vectors, 14 * 8);
        ACTIVE.store(0, Ordering::SeqCst);
        release().unwrap();
        assert!(LOG.load(Ordering::SeqCst).is_null());
        assert_eq!(TRACKER.snapshot().current, Default::default());
        LOG_SWAP.lock().unwrap().next = Some(buffer(3));
        drop(Hooks {
            hooks: vec![],
            refused: true,
            publication_attempted: false,
        });
        assert_eq!(TRACKER.snapshot().current, Default::default());
    }
    #[test]
    fn land_restore_retries_original_bytes_pending_verification() {
        let _guard = exclusive();
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc, VirtualFree,
        };
        let page = unsafe {
            VirtualAlloc(
                ptr::null(),
                4096,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            )
        }
        .cast::<u8>();
        assert!(!page.is_null());
        let sentinel = [0x90; 16];
        unsafe {
            ptr::copy_nonoverlapping(sentinel.as_ptr(), page, 16);
        }
        for refused in [false, true] {
            let mut hook =
                unsafe { Hook::new(page as usize, finalize as *const () as usize, sentinel) }
                    .unwrap();
            crate::hook::fail_flushes(2);
            assert!(hook.set_enabled(true).is_err());
            assert!(hook.is_enabled());
            assert_eq!(
                unsafe { ptr::read_unaligned(page.cast::<[u8; 16]>()) },
                sentinel
            );
            let mut group = Hooks {
                hooks: vec![hook],
                refused,
                publication_attempted: true,
            };
            // Both publication flushes failed: originals remain owned until
            // cache publication and verification succeed on a restore retry.
            unsafe {
                *page.add(8) = 0xcc;
            }
            if refused {
                assert_eq!(group.set_enabled(true), Ok(true));
            }
            let refused_restore = group.set_enabled(false).is_err();
            unsafe {
                *page.add(8) = sentinel[8];
            }
            let restored = group.set_enabled(false);
            assert!(refused_restore);
            assert_eq!(restored, Ok(true));
            drop(group);
        }
        unsafe {
            VirtualFree(page.cast(), 0, MEM_RELEASE);
        }
    }
    #[test]
    fn verdict_tap_disable_refuses_while_suspended_thread_is_inside_target() {
        let _guard = exclusive();
        static ENTERED: AtomicBool = AtomicBool::new(false);
        static RELEASE: AtomicBool = AtomicBool::new(false);
        unsafe extern "system-unwind" fn target(side: usize) -> usize {
            ENTERED.store(true, Ordering::Release);
            while !RELEASE.load(Ordering::Acquire) {
                std::hint::spin_loop();
            }
            side + 1
        }
        let enabled = ENABLED.swap(false, Ordering::SeqCst);
        ENTERED.store(false, Ordering::SeqCst);
        RELEASE.store(false, Ordering::SeqCst);
        let site = land_leader_tap_test_site as *const () as usize;
        let sentinel = unsafe { ptr::read_unaligned(site as *const [u8; 16]) };
        let mut hook = unsafe {
            Hook::new_indirect_call(
                site,
                leader_tap_entry as *const () as usize,
                sentinel,
                tap_safety(),
            )
        }
        .unwrap();
        assert!(hook.set_enabled(true).unwrap());
        let worker = std::thread::spawn(|| {
            test_support::adopt();
            let mut vtable = [0usize; 30];
            vtable[29] = target as *const () as usize;
            let mut preserved = [0usize; 8];
            unsafe {
                land_leader_tap_test_caller(41, vtable.as_ptr() as usize, preserved.as_mut_ptr())
            }
        });
        while !ENTERED.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        // set_enabled suspends the worker in target, outside the patch/relay/
        // stub. Its buried site+5 return must still prevent restoration.
        let refused = hook.set_enabled(false);
        let still_enabled = hook.is_enabled();
        let in_flight = TAP_IN_FLIGHT.load(Ordering::SeqCst);
        RELEASE.store(true, Ordering::Release);
        let result = worker.join().unwrap();
        assert_eq!(refused, Ok(false));
        assert!(still_enabled);
        assert_eq!(in_flight, 1);
        assert_eq!(result, 42);
        assert_eq!(TAP_IN_FLIGHT.load(Ordering::SeqCst), 0);
        assert!(hook.set_enabled(false).unwrap());
        assert_eq!(
            unsafe { ptr::read_unaligned(site as *const [u8; 16]) },
            sentinel
        );
        ENABLED.store(enabled, Ordering::SeqCst);
    }
    #[test]
    fn verdict_tap_stub_has_seh_stack_unwind_metadata() {
        use windows_sys::Win32::System::Diagnostics::Debug::RtlLookupFunctionEntry;
        let stub = tap_safety().stub;
        let code = unsafe { std::slice::from_raw_parts(stub.start as *const u8, 12) };
        assert_eq!(
            &code[4..11],
            &[0x4c, 0x8b, 0x80, 0xe8, 0, 0, 0],
            "faultable target load precedes count"
        );
        assert_eq!(code[11], 0xf0, "lock inc follows the load");
        let mut base = 0;
        let entry =
            unsafe { RtlLookupFunctionEntry((stub.start + 4) as u64, &mut base, ptr::null_mut()) };
        assert!(!entry.is_null());
        let entry = unsafe { &*entry };
        assert_eq!(base as usize + entry.BeginAddress as usize, stub.start);
        assert_eq!(base as usize + entry.EndAddress as usize, stub.end);
        let info = base as usize + unsafe { entry.Anonymous.UnwindInfoAddress } as usize;
        let info = unsafe { std::slice::from_raw_parts(info as *const u8, 6) };
        assert_eq!(info[0] & 7, 1); // UNWIND_INFO version1.
        assert_eq!(&info[1..3], &[4, 1]); // Four-byte prologue, one unwind code.
        assert_eq!(&info[4..6], &[4, 0x42]); // UWOP_ALLOC_SMALL, 0x28 bytes.
    }
    #[test]
    fn refused_detach_stops_land_production() {
        let _guard = exclusive();
        reset();
        let mut retained = false;
        let result = crate::runtime::refuse_land_detach_for_test(|| {
            assert!(
                !ENABLED.load(Ordering::SeqCst),
                "stop before retaining collector state"
            );
            retained = true;
        });
        assert_eq!(result, Err("synthetic land detach refused"));
        assert!(retained);
        let lost = test_support::thread_lost();
        let required = LOG_REQUIRED.load(Ordering::Relaxed);
        unsafe {
            observe_wipe(0, 0);
        }
        assert!(!ENABLED.load(Ordering::SeqCst));
        assert_eq!(test_support::thread_lost(), lost);
        assert_eq!(LOG_REQUIRED.load(Ordering::Relaxed), required);
    }
    static IP_ENTERED: AtomicBool = AtomicBool::new(false);
    static IP_RELEASE: AtomicBool = AtomicBool::new(false);
    core::arch::global_asm!(
        ".text", ".globl land_tap_ip_test_stub", ".seh_proc land_tap_ip_test_stub",
        "land_tap_ip_test_stub:", ".seh_endprologue",
        "mov byte ptr [rip + {entered}], 1",
        ".Lland_tap_ip_wait:", "cmp byte ptr [rip + {release}], 0",
        "je .Lland_tap_ip_wait", "ret", ".seh_endproc",
        ".globl land_tap_ip_test_stub_end", "land_tap_ip_test_stub_end:",
        entered = sym IP_ENTERED, release = sym IP_RELEASE);
    unsafe extern "system" {
        fn land_tap_ip_test_stub();
        fn land_tap_ip_test_stub_end();
    }
    #[test]
    fn verdict_tap_disable_refuses_relay_and_stub_ips_with_zero_counter() {
        use windows_sys::Win32::System::Memory::PAGE_READWRITE;
        use windows_sys::Win32::System::{
            Diagnostics::Debug::FlushInstructionCache, Memory::VirtualProtect,
            Threading::GetCurrentProcess,
        };
        let _guard = exclusive();
        let site = land_leader_tap_test_site as *const () as usize;
        let sentinel = unsafe { ptr::read_unaligned(site as *const [u8; 16]) };
        let stub = land_tap_ip_test_stub as *const () as usize;
        let mut hook = unsafe {
            Hook::new_indirect_call(
                site,
                stub,
                sentinel,
                crate::hook::TapSafety {
                    in_flight: &TAP_IN_FLIGHT,
                    stub: stub..land_tap_ip_test_stub_end as *const () as usize,
                },
            )
        }
        .unwrap();
        // A leaf loop in the relay page exercises its IP check before a body
        // could increment the counter. Leave the installed relay itself intact.
        let relay_loop = hook.executable_ranges()[0].start + 32;
        let mut bytes = [0u8; 29];
        bytes[..2].copy_from_slice(&[0x48, 0xb8]); // mov rax, entered
        bytes[2..10].copy_from_slice(&((&IP_ENTERED as *const _ as usize) as u64).to_le_bytes());
        bytes[10..15].copy_from_slice(&[0xc6, 0x00, 1, 0x48, 0xb8]);
        bytes[15..23].copy_from_slice(&((&IP_RELEASE as *const _ as usize) as u64).to_le_bytes());
        bytes[23..].copy_from_slice(&[0x80, 0x38, 0, 0x74, 0xfb, 0xc3]); // wait; ret
        let mut protection = 0;
        assert_ne!(
            unsafe {
                VirtualProtect(
                    relay_loop as *const _,
                    bytes.len(),
                    PAGE_READWRITE,
                    &mut protection,
                )
            },
            0
        );
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), relay_loop as *mut u8, bytes.len());
        }
        let mut ignored = 0;
        assert_ne!(
            unsafe {
                VirtualProtect(
                    relay_loop as *const _,
                    bytes.len(),
                    protection,
                    &mut ignored,
                )
            },
            0
        );
        assert_ne!(
            unsafe {
                FlushInstructionCache(GetCurrentProcess(), relay_loop as *const _, bytes.len())
            },
            0
        );
        for address in [relay_loop, stub] {
            IP_ENTERED.store(false, Ordering::SeqCst);
            IP_RELEASE.store(false, Ordering::SeqCst);
            assert!(hook.set_enabled(true).unwrap());
            let patched = unsafe { ptr::read_unaligned(site as *const [u8; 16]) };
            let worker = std::thread::spawn(move || {
                test_support::adopt();
                let wait: unsafe extern "system" fn() = unsafe { std::mem::transmute(address) };
                unsafe {
                    wait();
                }
            });
            while !IP_ENTERED.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            let in_flight = TAP_IN_FLIGHT.load(Ordering::SeqCst);
            let refused = hook.set_enabled(false);
            let still_enabled = hook.is_enabled();
            let unchanged = unsafe { ptr::read_unaligned(site as *const [u8; 16]) } == patched;
            IP_RELEASE.store(true, Ordering::Release);
            worker.join().unwrap();
            assert_eq!(in_flight, 0);
            assert_eq!(refused, Ok(false));
            assert!(still_enabled && unchanged);
            assert!(hook.set_enabled(false).unwrap());
            assert_eq!(
                unsafe { ptr::read_unaligned(site as *const [u8; 16]) },
                sentinel
            );
        }
    }
    #[test]
    fn repeated_division_killer_records_use_the_reserved_destruction_entry() {
        let _guard = exclusive();
        reset();
        unsafe extern "system-unwind" fn writer(_: usize, _: usize, _: usize, _: usize, _: usize) {}
        let original = ORIGINAL[DESTROYED].swap(writer as *const () as usize, Ordering::SeqCst);
        let _restore = Restore::new(&DESTRUCTION);
        let mut output = [0; 32];
        drain(&mut output, 10, true);
        let lost = test_support::thread_lost();
        for _ in 0..2 {
            append_at(10, |entry, put| {
                DESTRUCTION.set(Destruction {
                    army: 11,
                    from: 12,
                    to: 13,
                    division: 71,
                    entry,
                    ..Default::default()
                });
                for word in [1, 71, 1, 2, 0, 12, 13, 11, 0] {
                    put(word);
                }
                Ok(())
            });
            unsafe {
                destroyed(12, 13, 11, 0, 0);
            }
        }
        let observed = drain(&mut output, 11, false);
        ORIGINAL[DESTROYED].store(original, Ordering::SeqCst);
        test_support::disable();
        assert!(observed.complete);
        assert_eq!(test_support::thread_lost(), lost);
        assert_eq!(observed.words, 28);
        assert_eq!(&output[10..14], &[4, 6, 0, 0]);
        assert_eq!(&output[24..28], &[4, 6, 14, 0]);
        assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn failed_destruction_read_is_counted_once_across_nested_writer() {
        let _guard = exclusive();
        reset();
        unsafe extern "system-unwind" fn writer(_: usize, _: usize, _: usize, _: usize, _: usize) {}
        unsafe extern "system-unwind" fn original(army: usize, from: usize, to: usize) {
            unsafe {
                destroyed(from, to, army, 0, 0);
            }
        }
        let base = BASE.swap(0, Ordering::SeqCst);
        let outer =
            ORIGINAL[DESTRUCTION_ORIGINAL].swap(original as *const () as usize, Ordering::SeqCst);
        let inner = ORIGINAL[DESTROYED].swap(writer as *const () as usize, Ordering::SeqCst);
        let lost = test_support::thread_lost();
        unsafe {
            destruction(0, 0, 0);
        }
        assert_eq!(test_support::thread_lost() - lost, 1);
        assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
        ORIGINAL[DESTRUCTION_ORIGINAL].store(outer, Ordering::SeqCst);
        ORIGINAL[DESTROYED].store(inner, Ordering::SeqCst);
        BASE.store(base, Ordering::SeqCst);
        test_support::disable();
    }
    #[test]
    fn late_enable_verification_failure_withholds_only_land_and_rolls_back_prefix() {
        let _guard = exclusive();
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc, VirtualFree,
        };
        let page = unsafe {
            VirtualAlloc(
                ptr::null(),
                4096,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            )
        }
        .cast::<u8>();
        assert!(!page.is_null());
        let sentinel = [0x90; 16];
        unsafe {
            ptr::copy_nonoverlapping(sentinel.as_ptr(), page, 16);
            ptr::copy_nonoverlapping(sentinel.as_ptr(), page.add(64), 16);
        }
        let one =
            unsafe { Hook::new(page as usize, finalize as *const () as usize, sentinel) }.unwrap();
        let two = unsafe {
            Hook::new(
                page.add(64) as usize,
                finalize as *const () as usize,
                sentinel,
            )
        }
        .unwrap();
        // A byte changed after preparation, before enable's second check.
        unsafe {
            *page.add(79) = 0xcc;
        }
        let mut group = Hooks {
            hooks: vec![one, two],
            refused: false,
            publication_attempted: false,
        };
        assert!(
            group.set_enabled(true).unwrap(),
            "collector install may continue"
        );
        assert!(group.refused);
        assert!(!ENABLED.load(Ordering::SeqCst));
        assert!(group.hooks.iter().all(|h| !h.is_enabled()));
        assert_eq!(unsafe { std::slice::from_raw_parts(page, 16) }, sentinel);
        assert_eq!(
            unsafe { *page.add(79) },
            0xcc,
            "mismatching site must not be patched"
        );
        assert!(group.set_enabled(true).unwrap()); // No later partial admission.
        assert!(group.set_enabled(false).unwrap()); // Teardown skips unpatched mismatch.
        drop(group);
        unsafe {
            VirtualFree(page.cast(), 0, MEM_RELEASE);
        }
    }
    #[test]
    fn terminal_losses_are_read_after_native_pending_damage_flush() {
        let _guard = exclusive();
        reset();
        let world = crate::extended::combat_progress::tests::World::new(1);
        unsafe extern "system-unwind" fn flush(combat: usize) {
            let side = unsafe {
                ptr::read_unaligned((combat + engine::cland_combat::ATTACKER) as *const usize)
            };
            let observer = side + engine::cland_combatant::STATS_OBSERVER;
            unsafe {
                ptr::write_unaligned(
                    (observer + engine::cstats_observer::MANPOWER_LOSS) as *mut i32,
                    73,
                );
            }
            let entries = unsafe {
                ptr::read_unaligned(
                    (observer
                        + engine::cstats_observer::EQUIPMENT_LOSSES
                        + engine::cequipment_variant_pool::ITEMS)
                        as *const usize,
                )
            };
            unsafe {
                ptr::write_unaligned((entries + 8) as *mut i64, 234_567);
            }
        }
        let old_base = BASE.swap(0, Ordering::SeqCst);
        let old_original = ORIGINAL[FINALIZE].swap(flush as *const () as usize, Ordering::SeqCst);
        // Only valid synthetic combat/side objects. Missing state/history
        // correctly refuses their independent observations; terminal sampling
        // itself must still read the original's changes into a complete record.
        let family = crate::hook_timing::Family::LandEvents;
        let _sampled = crate::hook_timing::sample_all();
        let timed = crate::hook_timing::counts(family);
        unsafe {
            finalize(world.combat.as_ptr() as usize);
        }
        let after = crate::hook_timing::counts(family);
        assert!(after.calls > timed.calls && after.original > timed.original);
        let log = log().unwrap();
        let words = (0..CURSOR.load(Ordering::SeqCst) as usize)
            .map(|n| log.words[n].load(Ordering::Acquire))
            .collect::<Vec<_>>();
        assert_eq!(words[0], 11);
        assert_eq!(words[1], 8);
        assert_eq!(words[3], 73);
        assert_eq!(words[4], 1);
        assert_eq!(words[5], 1 << 32 | 70);
        assert_eq!(words[6], 234_567);
        assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
        ORIGINAL[FINALIZE].store(old_original, Ordering::SeqCst);
        BASE.store(old_base, Ordering::SeqCst);
        test_support::disable();
    }
    #[test]
    fn wipe_unsupported_class_refusals_are_named_counted_and_keep_interval_unknown() {
        let _guard = exclusive();
        reset();
        let old_base = BASE.swap(0, Ordering::SeqCst);
        let mut output = [0; 8];
        drain(&mut output, 10, true);
        let lost = test_support::thread_lost();
        let unknown_side = 0u64;
        let land_side = engine::cland_combatant::VTABLE_RVA as u64;
        let unsupported_unit = [engine::carmy::VTABLE_RVA as u64, 1];
        unsafe {
            observe_wipe(&unknown_side as *const _ as usize, 0);
            observe_wipe(
                &land_side as *const _ as usize,
                unsupported_unit.as_ptr() as usize,
            );
        }
        // Copy errors follow the same append refusal path; no partial record
        // may become complete just because its diagnostic was counted.
        append(2, |_| Err("combat wipe remaining class unsupported"));
        refusal("land combat class unsupported");
        refusal("destroyed division combat class unsupported");
        unsafe {
            observe_wipe(0, 0);
        } // Ordinary short/null read is not a class refusal.
        let observed = serde_json::to_value(status()).unwrap();
        let interval = drain(&mut output, 11, false);
        BASE.store(old_base, Ordering::SeqCst);
        test_support::disable();
        for (reason, _) in &WIPE_UNSUPPORTED_CLASSES[..3] {
            assert_eq!(observed["wipe_unsupported_classes"][*reason], 1);
        }
        for (reason, _) in &WIPE_UNSUPPORTED_CLASSES[3..] {
            assert_eq!(observed["land_unsupported_classes"][*reason], 1);
        }
        assert_eq!(test_support::thread_lost() - lost, 6);
        assert!(interval.lost >= 6);
        assert!(!interval.complete);
        assert!(
            observed["scope"]
                .as_str()
                .unwrap()
                .contains("not all read failures")
        );
    }
    #[test]
    fn win64_observation_entries_preserve_native_argument_and_return() {
        let _guard = exclusive();
        unsafe extern "system" fn commander(side: usize) -> usize {
            side.rotate_left(17)
        }
        unsafe extern "system" fn predicate(side: usize) -> u8 {
            ((side >> 40) ^ side) as u8
        }
        let enabled = ENABLED.swap(false, Ordering::SeqCst);
        let old_border =
            ORIGINAL[BORDER_TERMINATE].swap(predicate as *const () as usize, Ordering::SeqCst);
        // Synthetic originals in this test process only. No native game
        // pointer is read, no detour is installed and no process is opened.
        let border: unsafe extern "system" fn(usize) -> u8 =
            unsafe { std::mem::transmute(border_terminate_entry as *const ()) };
        let observed = [0, 0x8000_0001_2345_6789, usize::MAX].map(|side| unsafe {
            (
                side,
                leader_tap(side, 0, commander as *const () as usize),
                border(side),
            )
        });
        ORIGINAL[BORDER_TERMINATE].store(old_border, Ordering::SeqCst);
        ENABLED.store(enabled, Ordering::SeqCst);
        assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
        for (side, actual_leader, actual_border) in observed {
            assert_eq!(actual_leader, side.rotate_left(17));
            assert_eq!(actual_border, ((side >> 40) ^ side) as u8);
        }
    }
    core::arch::global_asm!(
        ".text",
        ".globl land_leader_tap_test_caller",
        ".seh_proc land_leader_tap_test_caller",
        "land_leader_tap_test_caller:",
        "push rbx",
        ".seh_pushreg rbx",
        "push rbp",
        ".seh_pushreg rbp",
        "push rsi",
        ".seh_pushreg rsi",
        "push rdi",
        ".seh_pushreg rdi",
        "push r12",
        ".seh_pushreg r12",
        "push r13",
        ".seh_pushreg r13",
        "push r14",
        ".seh_pushreg r14",
        "push r15",
        ".seh_pushreg r15",
        "sub rsp, 0x38",
        ".seh_stackalloc 0x38",
        ".seh_endprologue",
        "mov [rsp + 0x20], r8",
        "mov rax, rdx",
        "mov rbx, 0x1101",
        "mov rbp, 0x2202",
        "mov rsi, 0x3303",
        "mov rdi, 0x4404",
        "mov r12, 0x5505",
        "mov r13, 0x6606",
        "mov r14, 0x7707",
        "mov r15, 0x8808",
        ".globl land_leader_tap_test_site",
        "land_leader_tap_test_site:",
        "call qword ptr [rax + 0xe8]",
        ".rept 10",
        "nop",
        ".endr",
        "mov r10, [rsp + 0x20]",
        "mov [r10], rbx",
        "mov [r10 + 8], rbp",
        "mov [r10 + 16], rsi",
        "mov [r10 + 24], rdi",
        "mov [r10 + 32], r12",
        "mov [r10 + 40], r13",
        "mov [r10 + 48], r14",
        "mov [r10 + 56], r15",
        "add rsp, 0x38",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rdi",
        "pop rsi",
        "pop rbp",
        "pop rbx",
        "ret",
        ".seh_endproc"
    );
    unsafe extern "system-unwind" {
        pub(super) fn land_leader_tap_test_caller(
            side: usize,
            vtable: usize,
            preserved: *mut usize,
        ) -> usize;
        pub(super) fn land_leader_tap_test_site();
    }
    core::arch::global_asm!(
        ".text", ".globl land_wipe_test_caller", ".seh_proc land_wipe_test_caller",
        "land_wipe_test_caller:", "sub rsp, 0x28", ".seh_stackalloc 0x28", ".seh_endprologue",
        "mov rax, 1", "movq xmm0, rax", "punpcklqdq xmm0, xmm0",
        "mov rax, 2", "movq xmm1, rax", "punpcklqdq xmm1, xmm1",
        "mov rax, 3", "movq xmm2, rax", "punpcklqdq xmm2, xmm2",
        "mov rax, 4", "movq xmm3, rax", "punpcklqdq xmm3, xmm3",
        "mov rax, 5", "movq xmm4, rax", "punpcklqdq xmm4, xmm4",
        "mov rax, 6", "movq xmm5, rax", "punpcklqdq xmm5, xmm5",
        "mov rax, 0x101", "mov r8, 0x808", "mov r9, 0x909", "mov r10, 0xa0a", "mov r11, 0xb0b",
        "stc", "pushfq", "pop qword ptr [rsp + 0x20]", "call {entry}",
        ".globl land_wipe_test_return", "land_wipe_test_return:",
        "mov rax, [rsp + 0x20]", "add rsp, 0x28", "ret", ".seh_endproc",
        ".globl land_wipe_test_native", "land_wipe_test_native:",
        "mov [rcx], rax", "mov [rcx + 8], rdx", "mov [rcx + 16], r8", "mov [rcx + 24], r9",
        "mov [rcx + 32], r10", "mov [rcx + 40], r11", "mov rax, [rsp]", "mov [rcx + 48], rax",
        "pushfq", "pop rax", "mov [rcx + 56], rax",
        "movdqu [rcx + 64], xmm0", "movdqu [rcx + 80], xmm1", "movdqu [rcx + 96], xmm2",
        "movdqu [rcx + 112], xmm3", "movdqu [rcx + 128], xmm4", "movdqu [rcx + 144], xmm5",
        "ret", entry = sym land_wipe_test_site);
    core::arch::global_asm!(
        ".text", ".globl land_wipe_test_site", ".seh_proc land_wipe_test_site",
        "land_wipe_test_site:",
        // Same five-byte displaced instruction as the native lifecycle entry.
        "mov [rsp + 8], rbx", ".seh_endprologue",
        "jmp {native}", ".rept 16", "nop", ".endr", ".seh_endproc",
        native = sym land_wipe_test_native);
    unsafe extern "system-unwind" {
        fn land_wipe_test_caller(output: *mut u64, army: usize) -> u64;
        fn land_wipe_test_native();
        fn land_wipe_test_site();
        fn land_wipe_test_return();
    }
    #[test]
    fn wipe_observation_bridge_preserves_native_caller_registers_and_flags() {
        let _guard = exclusive();
        let enabled = ENABLED.swap(true, Ordering::SeqCst);
        let base = BASE.swap(0, Ordering::SeqCst);
        let clobber = WIPE_TEST_CLOBBER_XMM.swap(true, Ordering::SeqCst);
        let lost = test_support::thread_lost();
        let site = land_wipe_test_site as *const () as usize;
        let sentinel: [u8; 16] = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
            .try_into()
            .unwrap();
        assert_eq!(sentinel[..5], engine::land_wipe::LIFECYCLE_SENTINEL[..5]);
        let mut hook =
            unsafe { Hook::new(site, land_wipe_entry as *const () as usize, sentinel) }.unwrap();
        let previous = ORIGINAL[WIPE_ORIGINAL].swap(hook.trampoline(), Ordering::SeqCst);
        for tapped in [false, true, false] {
            assert!(hook.set_enabled(tapped).unwrap());
            let mut output = [0u64; 20];
            let flags = unsafe { land_wipe_test_caller(output.as_mut_ptr(), 0x202) };
            assert_eq!(&output[..6], &[0x101, 0x202, 0x808, 0x909, 0xa0a, 0xb0b]);
            // The mock unit deletion receives the CALL's original return PC,
            // including through the installed relay, bridge and trampoline.
            assert_eq!(
                output[6],
                land_wipe_test_return as *const () as usize as u64
            );
            assert_eq!(output[7], flags);
            for n in 0..6 {
                assert_eq!(&output[8 + n * 2..10 + n * 2], &[n as u64 + 1; 2]);
            }
            assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
        }
        ORIGINAL[WIPE_ORIGINAL].store(previous, Ordering::SeqCst);
        assert_eq!(
            test_support::thread_lost() - lost,
            1,
            "refusing observer body ran"
        );
        BASE.store(base, Ordering::SeqCst);
        WIPE_TEST_CLOBBER_XMM.store(clobber, Ordering::SeqCst);
        ENABLED.store(enabled, Ordering::SeqCst);
        assert_eq!(
            unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
            sentinel
        );
    }
    #[test]
    fn win64_verdict_tap_preserves_argument_return_and_nonvolatile_integer_registers() {
        let _guard = exclusive();
        let enabled = ENABLED.swap(true, Ordering::SeqCst);
        unsafe extern "system-unwind" fn commander(side: usize) -> usize {
            side.rotate_left(17)
        }
        unsafe extern "system-unwind" fn other_commander(side: usize) -> usize {
            !side
        }
        let mut vtable = [0usize; 30];
        let site = land_leader_tap_test_site as *const () as usize;
        let base = BASE.swap(
            site + 6 - native::DISPATCH_LEADER_RETURNS[0],
            Ordering::SeqCst,
        );
        let saved_dispatch = DISPATCH.get();
        let lost = test_support::thread_lost();
        let sentinel: [u8; 16] = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
            .try_into()
            .unwrap();
        assert_eq!(sentinel[..6], native::DISPATCH_LEADER_CALL_SENTINEL);
        let mut hook = unsafe {
            Hook::new_indirect_call(
                site,
                leader_tap_entry as *const () as usize,
                sentinel,
                tap_safety(),
            )
        }
        .unwrap();
        let hits = test_support::thread_leader_calls();
        for tapped in [false, true, false] {
            assert!(hook.set_enabled(tapped).unwrap());
            for side in [0, 0x8000_0001_2345_6789, usize::MAX] {
                for target in [
                    commander as *const () as usize,
                    other_commander as *const () as usize,
                ] {
                    // Correct return-site classification with a deliberately
                    // mismatching side: observation must time and fail.
                    DISPATCH.set(Dispatch {
                        combat: 1,
                        sides: [side.wrapping_add(1); 2],
                        ..NO_DISPATCH
                    });
                    vtable[29] = target;
                    let mut preserved = [0; 8];
                    let result = unsafe {
                        land_leader_tap_test_caller(
                            side,
                            vtable.as_ptr() as usize,
                            preserved.as_mut_ptr(),
                        )
                    };
                    assert_eq!(
                        result,
                        if target == commander as *const () as usize {
                            side.rotate_left(17)
                        } else {
                            !side
                        }
                    );
                    assert_eq!(
                        preserved,
                        [
                            0x1101, 0x2202, 0x3303, 0x4404, 0x5505, 0x6606, 0x7707, 0x8808
                        ]
                    );
                }
            }
        }
        assert_eq!(test_support::thread_leader_calls() - hits, 6);
        assert_eq!(
            test_support::thread_lost() - lost,
            6,
            "all tapped observer bodies ran"
        );
        assert_eq!(TAP_IN_FLIGHT.load(Ordering::SeqCst), 0);
        assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
        assert_eq!(
            unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
            sentinel
        );
        ENABLED.store(enabled, Ordering::SeqCst);
        BASE.store(base, Ordering::SeqCst);
        DISPATCH.set(saved_dispatch);
    }
    #[test]
    #[ignore = "print-only counted verdict-tap differential bench; owned test-process hook only"]
    fn land_leader_tap_cost() {
        let _guard = exclusive();
        unsafe extern "system-unwind" fn native_target(side: usize) -> usize {
            side
        }
        const SAMPLES: usize = 201;
        const HITS: usize = 2048; // Benchmark batch, never an observation limit.
        let enabled = ENABLED.swap(true, Ordering::SeqCst);
        let site = land_leader_tap_test_site as *const () as usize;
        let base = site + 6 - native::DISPATCH_LEADER_RETURNS[0];
        let saved_base = BASE.swap(base, Ordering::SeqCst);
        let saved_dispatch = DISPATCH.get();
        let sentinel = unsafe { ptr::read_unaligned(site as *const [u8; 16]) };
        let mut hook = unsafe {
            Hook::new_indirect_call(
                site,
                leader_tap_entry as *const () as usize,
                sentinel,
                tap_safety(),
            )
        }
        .unwrap();
        let mut vtable = [0usize; 30];
        vtable[29] = native_target as *const () as usize;
        let mut leader = vec![0u8; engine::cunit_leader::TYPE + 8];
        let pointer = leader.as_mut_ptr() as usize;
        leader[..8]
            .copy_from_slice(&((base + engine::carmy_leader::VTABLE_RVA) as u64).to_le_bytes());
        leader[engine::cunit_leader::TYPE..].copy_from_slice(&17u64.to_le_bytes());
        // Both paths reset identical TLS context, so the paired delta includes
        // the real relay/stub, both in-flight atomics and the observation body.
        for with_commander in [false, true] {
            let context = if with_commander {
                Dispatch {
                    combat: 1,
                    sides: [pointer, 0],
                    winner: 2,
                    ..NO_DISPATCH
                }
            } else {
                NO_DISPATCH
            };
            let mut times: [Vec<u128>; 2] = std::array::from_fn(|_| Vec::with_capacity(SAMPLES));
            let mut preserved = [0usize; 8];
            for sample in 0..SAMPLES {
                // Alternate order; patching and warmup stay outside the timer.
                for step in 0..2 {
                    let mode = (sample + step) % 2;
                    assert!(hook.set_enabled(mode == 1).unwrap());
                    DISPATCH.set(context);
                    assert_eq!(
                        unsafe {
                            land_leader_tap_test_caller(
                                pointer,
                                vtable.as_ptr() as usize,
                                preserved.as_mut_ptr(),
                            )
                        },
                        pointer
                    );
                    let mut result = 0;
                    let start = std::time::Instant::now();
                    for _ in 0..HITS {
                        DISPATCH.set(context);
                        result = std::hint::black_box(unsafe {
                            land_leader_tap_test_caller(
                                pointer,
                                vtable.as_ptr() as usize,
                                preserved.as_mut_ptr(),
                            )
                        });
                    }
                    let elapsed = start.elapsed().as_nanos();
                    assert_eq!(result, pointer);
                    assert_eq!(DISPATCH.get().seen[0], mode == 1 && with_commander);
                    assert_eq!(TAP_IN_FLIGHT.load(Ordering::SeqCst), 0);
                    assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
                    times[mode].push(elapsed);
                }
            }
            let mut added = times[1]
                .iter()
                .zip(&times[0])
                .map(|(tap, native)| *tap as i128 - *native as i128)
                .collect::<Vec<_>>();
            let negative_pairs = added.iter().filter(|&&delta| delta < 0).count();
            added.sort_unstable();
            for time in &mut times {
                time.sort_unstable();
            }
            let ns_per_call = |ns: u128| ns as f64 / HITS as f64;
            println!(
                "LAND_LEADER_COST mode={} samples={SAMPLES} hits_per_batch={HITS} native_p50_ns_per_call={:.3} native_p95_ns_per_call={:.3} native_max_ns_per_call={:.3} tap_p50_ns_per_call={:.3} tap_p95_ns_per_call={:.3} tap_max_ns_per_call={:.3} added_p50_ns_per_call={:.3} added_p95_ns_per_call={:.3} added_max_ns_per_call={:.3} added_min_ns_per_call={:.3} negative_pairs={negative_pairs} in_flight_atomic_ops_per_call=2 profile={} scope=synthetic_uncontended_native_vs_installed_tap_identical_caller_and_context_reset",
                if with_commander {
                    "verdict_tap_with_commander"
                } else {
                    "verdict_tap_no_dispatch"
                },
                ns_per_call(times[0][SAMPLES / 2]),
                ns_per_call(times[0][(SAMPLES - 1) * 95 / 100]),
                ns_per_call(times[0][SAMPLES - 1]),
                ns_per_call(times[1][SAMPLES / 2]),
                ns_per_call(times[1][(SAMPLES - 1) * 95 / 100]),
                ns_per_call(times[1][SAMPLES - 1]),
                added[SAMPLES / 2] as f64 / HITS as f64,
                added[(SAMPLES - 1) * 95 / 100] as f64 / HITS as f64,
                added[SAMPLES - 1] as f64 / HITS as f64,
                added[0] as f64 / HITS as f64,
                if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "optimized"
                }
            );
        }
        assert!(hook.set_enabled(false).unwrap());
        DISPATCH.set(saved_dispatch);
        BASE.store(saved_base, Ordering::SeqCst);
        ENABLED.store(enabled, Ordering::SeqCst);
    }
    #[test]
    fn verdict_tap_sentinel_mismatch_refuses_by_site_name() {
        struct Bytes {
            base: usize,
            values: [[u8; 16]; 2],
        }
        impl Memory for Bytes {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let Some(n) = native::DISPATCH_LEADER_CALLS
                    .iter()
                    .position(|rva| self.base + rva == address)
                else {
                    return false;
                };
                if output.len() != 16 {
                    return false;
                }
                output.copy_from_slice(&self.values[n]);
                true
            }
        }
        let mut bytes = Bytes {
            base: 0x140000000,
            values: [[0; 16]; 2],
        };
        for value in &mut bytes.values {
            value[..6].copy_from_slice(&native::DISPATCH_LEADER_CALL_SENTINEL);
        }
        assert!(verify_leader_taps(&bytes, bytes.base).is_ok());
        for n in 0..2 {
            bytes.values[n][5] = 1;
            let refusal = verify_leader_taps(&bytes, bytes.base).unwrap_err();
            assert!(refusal.contains(native::DISPATCH_LEADER_CALL_NAMES[n]));
            bytes.values[n][5] = 0;
        }
    }
    #[test]
    fn verdict_resolver_rust_unwind_retains_the_asm_owned_flight() {
        let _guard = exclusive();
        unsafe extern "system-unwind" fn native_unwind(_: usize) -> usize {
            panic!("synthetic resolver unwind");
        }
        reset();
        assert_eq!(TAP_IN_FLIGHT.load(Ordering::SeqCst), 0);
        let escaped = TAP_ESCAPED_UNWINDS.load(Ordering::SeqCst);
        let storage = LOG.load(Ordering::SeqCst);
        // Fresh direct body fixture: no ASM/native return frame exists. This
        // exercises its real owner guard; Rust panic is not foreign evidence.
        TAP_IN_FLIGHT.store(1, Ordering::SeqCst);
        let result = std::panic::catch_unwind(|| {
            unsafe { leader_tap_body(0, 0, native_unwind as *const () as usize) };
        });
        assert!(result.is_err());
        assert_eq!(ACTIVE.load(Ordering::SeqCst), 1);
        assert_eq!(TAP_IN_FLIGHT.load(Ordering::SeqCst), 1);
        assert_eq!(TAP_ESCAPED_UNWINDS.load(Ordering::SeqCst), escaped + 1);
        assert!(crate::runtime::observers_active());
        assert!(!ENABLED.load(Ordering::SeqCst));
        assert!(release().is_err());
        assert_eq!(LOG.load(Ordering::SeqCst), storage);
        let mut out = [0; 32];
        for (hour, load) in [(10, false), (11, true), (12, false)] {
            let rejected = drain(&mut out, hour, load);
            assert!(!rejected.complete);
            assert_eq!(rejected.reason, TAP_UNWIND_RETENTION);
        }
        let mut group = Hooks {
            hooks: vec![],
            refused: false,
            publication_attempted: false,
        };
        assert_eq!(group.set_enabled(true), Err(TAP_UNWIND_RETENTION.into()));
        // Only this fresh direct fixture is retired: no ASM frame was entered,
        // and catch_unwind has finished the actual Rust body/guard traversal.
        TAP_IN_FLIGHT.store(0, Ordering::SeqCst);
        TAP_ESCAPED_UNWINDS.store(escaped, Ordering::SeqCst);
        ACTIVE.store(0, Ordering::SeqCst);
        CALL_ESCAPED_UNWINDS.store(0, Ordering::SeqCst);
        release().unwrap();
    }
    #[test]
    fn original_unwind_retains_active_and_restores_nested_tls() {
        let _guard = exclusive();
        reset();
        let enabled = ENABLED.swap(false, Ordering::SeqCst);
        unsafe extern "system-unwind" fn unwind_three(_: usize, _: usize, _: usize) {
            DISPATCH.set(NO_DISPATCH);
            BORDER.set(NO_BORDER);
            DESTRUCTION.set(Destruction::default());
            panic!("synthetic native unwind");
        }
        unsafe extern "system-unwind" fn unwind_one(_: usize) {
            BORDER.set(NO_BORDER);
            panic!("synthetic native unwind");
        }
        for (slot, callback) in [
            (DISPATCH_ORIGINAL, dispatch as *const () as usize),
            (DESTRUCTION_ORIGINAL, destruction as *const () as usize),
        ] {
            let old = ORIGINAL[slot].swap(unwind_three as *const () as usize, Ordering::SeqCst);
            DISPATCH.set(Dispatch {
                combat: 91,
                ..NO_DISPATCH
            });
            DESTRUCTION.set(Destruction {
                division: 92,
                ..Destruction::default()
            });
            let run: unsafe extern "system-unwind" fn(usize, usize, usize) =
                unsafe { std::mem::transmute(callback) };
            assert!(std::panic::catch_unwind(|| unsafe { run(0, 0, 0) }).is_err());
            assert_ne!(ACTIVE.load(Ordering::SeqCst), 0);
            if slot == DISPATCH_ORIGINAL {
                assert_eq!(DISPATCH.get().combat, 91);
            } else {
                assert_eq!(DESTRUCTION.get().division, 92);
            }
            ORIGINAL[slot].store(old, Ordering::SeqCst);
        }
        for (slot, callback) in [
            (BORDER_FINALIZE, border_finalize as *const () as usize),
            (FINALIZE, finalize as *const () as usize),
        ] {
            let old = ORIGINAL[slot].swap(unwind_one as *const () as usize, Ordering::SeqCst);
            BORDER.set(Border {
                combat: 93,
                ..NO_BORDER
            });
            let run: unsafe extern "system-unwind" fn(usize) =
                unsafe { std::mem::transmute(callback) };
            assert!(std::panic::catch_unwind(|| unsafe { run(0) }).is_err());
            if slot == BORDER_FINALIZE {
                assert_eq!(BORDER.get().combat, 93);
            }
            assert_ne!(ACTIVE.load(Ordering::SeqCst), 0);
            ORIGINAL[slot].store(old, Ordering::SeqCst);
        }
        assert_eq!(ACTIVE.load(Ordering::SeqCst), 4);
        assert!(crate::runtime::observers_active());
        assert!(release().is_err());
        let mut output = [0; 8];
        for (hour, load) in [(10, false), (11, true), (12, false)] {
            let refused = drain(&mut output, hour, load);
            assert!(!refused.complete);
            assert_eq!(refused.reason, CALL_UNWIND_RETENTION);
        }
        let mut group = Hooks {
            hooks: vec![],
            refused: false,
            publication_attempted: false,
        };
        assert_eq!(group.set_enabled(true), Err(CALL_UNWIND_RETENTION.into()));
        // Direct fixture callbacks and their catch handlers have now returned.
        ACTIVE.store(0, Ordering::SeqCst);
        CALL_ESCAPED_UNWINDS.store(0, Ordering::SeqCst);
        DISPATCH.set(NO_DISPATCH);
        BORDER.set(NO_BORDER);
        DESTRUCTION.set(Destruction::default());
        ENABLED.store(enabled, Ordering::SeqCst);
    }
    #[test]
    fn append_unwind_releases_producer_and_refuses_interval() {
        let _guard = exclusive();
        reset();
        let mut output = [0; 8];
        drain(&mut output, 10, true);
        let lost = test_support::thread_lost();
        let result = std::panic::catch_unwind(|| {
            append(4, |put| {
                put(6);
                panic!("synthetic writer unwind");
            })
        });
        assert!(result.is_err());
        assert_eq!(PRODUCERS.load(Ordering::SeqCst), 0);
        let refused = drain(&mut output, 11, false);
        assert!(!refused.complete);
        assert_eq!(test_support::thread_lost() - lost, 1);
        assert!(refused.lost >= 1);
        ENABLED.store(false, Ordering::SeqCst);
    }
    #[test]
    fn resolver_observations_require_the_native_verdict_callers() {
        let base = 0x140000000;
        for winner in [1, 2] {
            let dispatch = Dispatch {
                combat: 17,
                sides: [0x1000, 0x2000],
                winner,
                ..NO_DISPATCH
            };
            let win = usize::from(winner == 2);
            for (role, n) in [(0, 1 - win), (1, win)] {
                let caller = base + native::DISPATCH_LEADER_RETURNS[role];
                assert_eq!(dispatch.slot(dispatch.sides[n], caller, base), Ok(Some(n)));
                assert!(dispatch.slot(dispatch.sides[1 - n], caller, base).is_err());
                let mut seen = dispatch;
                seen.seen[n] = true;
                assert!(seen.slot(seen.sides[n], caller, base).is_err());
            }
            // A scripted query on either side within dispatch must not
            // replace its actual on-action commander, even after it was seen.
            for side in dispatch.sides {
                assert_eq!(dispatch.slot(side, base + 0xc0f330, base), Ok(None));
            }
        }
        let border = Border {
            combat: 17,
            sides: [0x1000, 0x2000],
            ..NO_BORDER
        };
        for n in 0..2 {
            let caller = base + native::BORDER_TERMINATE_RETURNS[n];
            assert_eq!(border.slot(border.sides[n], caller, base), Ok(Some(n)));
            assert!(border.slot(border.sides[1 - n], caller, base).is_err());
            let mut queried = border;
            queried.terminating[n] = 0;
            assert!(queried.slot(queried.sides[n], caller, base).is_err());
            assert_eq!(
                border.slot(border.sides[n], base + 0xa026a0, base),
                Ok(None)
            );
        }
    }
    #[test]
    fn dispatched_match_is_consumed_once() {
        let _guard = exclusive();
        reset();
        LAST_DISPATCHED.set((0x1000, 17, 1));
        assert!(take_dispatched(0x1000, 17));
        assert!(!take_dispatched(0x1000, 17));
        ENABLED.store(false, Ordering::SeqCst);
    }
    #[test]
    fn dispatched_pointer_reuse_with_a_different_handle_keeps_fallback() {
        let _guard = exclusive();
        reset();
        LAST_DISPATCHED.set((0x1000, 17, 1));
        assert!(!take_dispatched(0x1000, 18));
        assert!(!take_dispatched(0x1000, 17));
        ENABLED.store(false, Ordering::SeqCst);
    }
    #[test]
    fn failed_dispatch_append_invalidates_the_skip_witness() {
        let _guard = exclusive();
        reset();
        // Dispatch sets the witness before appending its commander record.
        let epoch = EPOCH.load(Ordering::SeqCst);
        let lost = test_support::thread_lost();
        LAST_DISPATCHED.set((0x1000, 17, epoch));
        append(6, |_| Err("failed dispatch append"));
        assert!(EPOCH.load(Ordering::SeqCst) > epoch);
        assert_eq!(test_support::thread_lost() - lost, 1);
        assert!(!take_dispatched(0x1000, 17));
        ENABLED.store(false, Ordering::SeqCst);
    }
    #[test]
    fn first_gap_read_failure_capacity_and_busy_producer_are_unknown() {
        let _guard = exclusive();
        reset();
        let mut out = [0; 32];
        assert!(!drain(&mut out, 10, false).complete);
        assert!(drain(&mut out, 11, false).complete);
        append(4, |put| {
            put(2);
            put(19);
            put(0);
            Ok(())
        });
        assert_eq!(drain(&mut out, 12, false).words, 4);
        let lost = test_support::thread_lost();
        append(4, |_| Err("short native read"));
        let bad = drain(&mut out, 13, false);
        assert!(!bad.complete);
        assert_eq!(test_support::thread_lost() - lost, 1);
        assert!(bad.lost >= 1);
        assert!(drain(&mut out, 14, false).complete);
        assert!(!drain(&mut out, 20, true).complete);
        assert!(drain(&mut out, 21, false).complete);
        append(4, |put| {
            put(2);
            put(19);
            put(0);
            Ok(())
        });
        let short = drain(&mut [], 22, false);
        assert!(!short.complete);
        assert_eq!(short.reason, "land event drain capacity exceeded");
        assert_eq!(DRAIN_CAPACITY_FAILURES.load(Ordering::SeqCst), 1);
        assert_eq!(LOG_CAPACITY_FAILURES.load(Ordering::SeqCst), 0);
        drain(&mut out, 23, true);
        PRODUCERS.store(1, Ordering::SeqCst);
        assert!(!drain(&mut out, 24, false).complete);
        PRODUCERS.store(0, Ordering::SeqCst);
        ENABLED.store(false, Ordering::SeqCst);
    }
    #[test]
    fn land_log_growth_exceeds_the_former_ceiling_and_counts_refused_intervals() {
        let _guard = exclusive();
        test_support::reset_with_words(0);
        let demand = (1 << 20) + 17; // Regression fixture beyond the removed cap.
        let mut out = vec![0; demand];
        drain(&mut out, 10, true);
        append(demand, |_| {
            panic!("short log must not read transient payload")
        });
        let overflow = drain(&mut out, 11, false);
        assert!(!overflow.complete);
        assert_eq!(overflow.reason, "land event log capacity exceeded");
        assert_eq!(LOG_CAPACITY_FAILURES.load(Ordering::SeqCst), 1);
        assert!(overflow.lost > 0);
        let lost = test_support::thread_lost();
        assert_eq!(overflow.required, demand);
        // Growth starts no epoch; other modules' tests may on their threads.
        let epochs = test_support::thread_epochs();
        prepare();
        install_offered();
        assert!(LOG_CAPACITY.load(Ordering::SeqCst) >= demand);
        assert_eq!(test_support::thread_epochs(), epochs);
        append(demand, |put| {
            for _ in 1..demand {
                put(7);
            }
            Ok(())
        });
        let recovered = drain(&mut out, 12, false);
        assert!(recovered.complete);
        assert_eq!(recovered.words, demand);
        assert_eq!(out[0], demand as u64);
        assert!(out[1..].iter().all(|&word| word == 7));
        assert_eq!(test_support::thread_lost(), lost);
        ENABLED.store(false, Ordering::SeqCst);
    }

    #[test]
    fn live_word_hint_presizes_the_log_and_drains_before_any_event_overflow() {
        let _guard = exclusive();
        test_support::reset_with_words(0);
        presize(50_005);
        let prepared = capacity();
        assert!(prepared >= 50_005);
        let mut out = vec![0; prepared];
        install_offered();
        drain(&mut out, 10, true);
        append(50_005, |put| {
            for _ in 1..50_005 {
                put(7);
            }
            Ok(())
        });
        let observed = drain(&mut out, 11, false);
        assert!(observed.complete);
        assert_eq!(observed.words, 50_005);
        assert_eq!(LOG_CAPACITY_FAILURES.load(Ordering::Relaxed), 0);
        assert_eq!(DRAIN_CAPACITY_FAILURES.load(Ordering::Relaxed), 0);
        ENABLED.store(false, Ordering::SeqCst);
    }

    #[test]
    fn frame_headroom_never_reenters_land_log_demand() {
        let _guard = exclusive();
        test_support::reset_with_words(0);
        let mut capacities = crate::extended::land_events::Capacities { words: 9 };
        let mut frames: [crate::extended::land_events::Captured; 4] = std::array::from_fn(|_| {
            crate::extended::land_events::Captured::with_capacities(&capacities)
        });
        presize(9);
        for hour in 10..18 {
            for frame in &mut frames {
                frame.grow(&mut capacities);
            }
            frames[(hour as usize - 10) % 4].drain(hour, hour == 10);
            assert_eq!(capacities.words, 9);
            assert_eq!(LOG_REQUIRED.load(Ordering::Relaxed), 9);
            assert_eq!(LOG_CAPACITY.load(Ordering::Acquire), 16);
        }
        assert_eq!(LOG_CAPACITY_FAILURES.load(Ordering::Relaxed), 0);
        assert_eq!(DRAIN_CAPACITY_FAILURES.load(Ordering::Relaxed), 0);
        ENABLED.store(false, Ordering::SeqCst);
    }

    #[test]
    fn a_real_producer_overlapping_the_boundary_poisoned_only_that_interval() {
        let _guard = exclusive();
        reset();
        let mut out = [0; 32];
        drain(&mut out, 10, true);
        let entered = std::sync::Barrier::new(2);
        let release = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let producer = scope.spawn(|| {
                test_support::adopt();
                let _call = Call::enter();
                append(4, |put| {
                    put(6);
                    entered.wait();
                    release.wait();
                    put(19);
                    put(0);
                    Ok(())
                });
                _call.finish();
            });
            entered.wait();
            LOG_REQUIRED.fetch_max(4096, Ordering::SeqCst);
            prepare();
            install_offered();
            assert_eq!(LOG_CAPACITY.load(Ordering::SeqCst), 1024);
            let bad = drain(&mut out, 11, false);
            assert!(!bad.complete);
            assert!(bad.lost > 0);
            release.wait();
            producer.join().unwrap();
        });
        install_offered();
        assert_eq!(LOG_CAPACITY.load(Ordering::SeqCst), 4096);
        let next = drain(&mut out, 12, false);
        assert!(next.complete);
        assert_eq!(next.words, 0);
        ENABLED.store(false, Ordering::SeqCst);
    }
    #[test]
    fn resolver_lifetime_at_boundary_does_not_poison_an_interval() {
        let _guard = exclusive();
        reset();
        let mut out = [0; 32];
        drain(&mut out, 10, true);
        let _resolver_call = Call::enter();
        assert_ne!(ACTIVE.load(Ordering::SeqCst), 0);
        assert_eq!(PRODUCERS.load(Ordering::SeqCst), 0);
        assert!(drain(&mut out, 11, false).complete);
        ENABLED.store(false, Ordering::SeqCst);
        _resolver_call.finish();
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::test_support::*;
    use super::*;

    /// Another module's thread reaching the log while a land test owns it
    /// (a full decode queue, an observer drain, a sizing hint) leaves the
    /// owner's interval, epoch and storage alone; a thread the owner adopted
    /// reaches it as the owner does. Before, the foreign loss advanced the
    /// epoch and refused the owner's next interval.
    #[test]
    fn a_foreign_thread_leaves_an_owned_log_alone() {
        let _guard = exclusive();
        reset();
        let mut out = [0; 32];
        drain(&mut out, 10, true);
        let epoch = EPOCH.load(Ordering::SeqCst);
        let capacity = LOG_CAPACITY.load(Ordering::SeqCst);
        std::thread::spawn(|| {
            fail("land frame decode queue full");
            assert_eq!(lost_casualty_reads(3), 3);
            let drained = lost_drain(11, "land observer frame not captured");
            assert!(!drained.complete);
            assert_eq!(drained.reason, "land event log owned by another test");
            presize(1 << 16);
            stop_observation();
            append_record(&[5]);
        })
        .join()
        .unwrap();
        assert_eq!(EPOCH.load(Ordering::SeqCst), epoch);
        assert_eq!(LOG_CAPACITY.load(Ordering::SeqCst), capacity);
        assert!(ENABLED.load(Ordering::SeqCst));
        append_record(&[9]);
        let owned = drain(&mut out, 11, false);
        assert!(owned.complete);
        assert_eq!(&out[..owned.words], &[2, 9]);
        // An adopted thread is the owner's.
        std::thread::spawn(|| {
            adopt();
            fail("land frame decode queue full");
        })
        .join()
        .unwrap();
        assert_ne!(EPOCH.load(Ordering::SeqCst), epoch);
        assert!(!drain(&mut out, 12, false).complete);
        disable();
    }
}
