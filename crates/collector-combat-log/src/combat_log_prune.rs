//! The final values of every Combat Log a daily prune deletes.
//!
//! The game deletes a group's Combat Log in its daily prune
//! ([`engine_land_inputs::ncombat_log_cmanager::PRUNE_RVA`], per country inside
//! the daily update) once the group's orders group is gone. Orders
//! groups can be deleted inside the hourly update that runs the prune, after
//! the hourly sample, so no sample can see those logs' last changes. A detour
//! at the prune's entry reads, before the original runs, every log it is
//! about to delete for that reason ([`engine_land_inputs::ncombat_log_cmanager::deleted_logs`])
//! into a lock-free log; nothing can change those logs any more, so
//! the rows are their final values. The detour only reads and calls the
//! original; it never changes game behaviour and calls no game function.
//!
//! The daily update dispatches the per-country prune as a TBB task. This
//! is a daily job worker path; the simulation thread may also execute the task.
//! Its wall cost belongs to that thread, outside the hourly callback's timing.
//! The log is multi-producer: a prune counts its deleted logs' rows, reserves
//! that many with one `fetch_add`, then reads them into its reservation. The
//! log is sized off the hook ([`prepare`]) from the rows every retained group
//! log holds, any of which a prune can delete, and from any shortfall a prune
//! met; a drain installs the larger log. The hourly callback, which runs while
//! no daily update does, hands the rows over ([`drain`]). Rows that do not fit
//! (the whole pools that did are kept), a read that fails, or hooks that are
//! not installed or enabled make that drain unobserved: the table then says so
//! for the hour (`CombatLogState.prune_observed`).
//!
//! Each row carries the load signal (the global
//! [`engine_native::cgame_state::PLAY_SESSION_STARTED_AT_RVA`] every load sets) of
//! the session whose prune logged it. The drain hands over only the rows of
//! the sample's own session: rows an earlier session logged after its last
//! sample describe a branch the player left, and are discarded and counted.
//!
//! Stack: `deleted_logs` nests the bounded `for_each_group`, `read_entries`
//! and `read_pool` batch buffers: about 60 KB together, before native callers
//! and guarded-read frames. The pinned executable's PE SizeOfStackReserve is
//! 4 MB. The TBB workers live in TBB.DLL; their actual stack reserve is not
//! established by the executable task path alone. A live run must check it
//! and measure `observe_total_ns` and `observe_max_ns`; a smaller worker stack
//! needs a smaller batch footprint. Observation stays allocation-free and
//! lock-free.
use crate::{hook::Hook, opinion_changes::Padded};
use engine_land_inputs::{
    ncombat_log_cmanager as native, ncombat_log_cmanager::PRUNE_RVA,
    ncombat_log_cmanager::PRUNE_SENTINEL, ncombat_log_cmanager::Row,
};
use engine_native::{Memory, walk::Outcome};
use std::{
    cell::UnsafeCell,
    ops::Range,
    ptr,
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};

/// The rows prunes reserved since the last drain, and the load signal of the
/// session that logged each (`None`: reserved, but not read).
///
/// Safety argument, for every access below:
/// - A writer reserves `first..first + len` with one `CURSOR.fetch_add`, so
///   two writers' ranges never overlap. It writes only the part of its range
///   within the log, and marks every element of that part.
/// - Each writer runs inside a [`Call`], which increments `ACTIVE` before the
///   reservation and decrements it (`SeqCst`) after its last write.
/// - The drain runs at the hourly sample, when no prune can run: the prune
///   runs only inside the daily update, which the hourly update runs
///   after the sample, and the callback holds the game thread while it
///   drains. It reads only after loading `ACTIVE` as 0: every `ACTIVE` change
///   is a read-modify-write, so that load synchronises with every writer's
///   final decrement, and all their writes happen before the drain's reads.
///   A running writer (a violated precondition) makes the drain refuse.
/// - The drain reads only reserved elements within the log, all marked.
/// - A log is replaced or freed only with `HANDOFF` set and `ACTIVE` then
///   loaded as 0; a writer tests `HANDOFF` after entering its call, and loads
///   `LOG` only when it is clear. So no writer holds a replaced log.
struct Log {
    rows: Box<[UnsafeCell<Row>]>,
    signals: Box<[UnsafeCell<Option<i64>>]>,
}
// SAFETY: see the safety argument above.
unsafe impl Sync for Log {}

impl Log {
    fn new(len: usize) -> std::io::Result<Box<Self>> {
        let rows = engine_allocation::boxed_slice(engine_allocation::default_vec(len)?)?;
        let signals = engine_allocation::boxed_slice(engine_allocation::default_vec(len)?)?;
        engine_allocation::boxed(Self { rows, signals })
    }

    fn len(&self) -> usize {
        self.rows.len()
    }

    /// # Safety
    /// The caller alone reserved `range`, which lies within the log.
    #[allow(clippy::mut_from_ref)]
    unsafe fn rows_mut(&self, range: Range<usize>) -> &mut [Row] {
        // Element pointers only: other writers own the rest of the log.
        unsafe {
            std::slice::from_raw_parts_mut(
                UnsafeCell::raw_get(self.rows.as_ptr().add(range.start)),
                range.len(),
            )
        }
    }

    /// # Safety
    /// As [`Log::rows_mut`], for `index`.
    unsafe fn mark(&self, index: usize, signal: Option<i64>) {
        unsafe { self.signals[index].get().write(signal) };
    }

    /// # Safety
    /// No writer is active and `index` was marked.
    unsafe fn read(&self, index: usize) -> (Option<i64>, Row) {
        unsafe {
            (
                self.signals[index].get().read(),
                self.rows[index].get().read(),
            )
        }
    }
}

/// The installed log, a larger one offered for the next drain, and a
/// replaced one, freed off the boundary.
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
/// `SWAP.current`, for writers; null until a drain installs one.
static LOG: AtomicPtr<Log> = AtomicPtr::new(ptr::null_mut());
/// Writers keep off the log while it is replaced or freed.
static HANDOFF: AtomicBool = AtomicBool::new(false);
static CAPACITY: AtomicUsize = AtomicUsize::new(0);
/// Rows the log must hold: retained group logs' rows, or a prune's shortfall.
static REQUIRED: AtomicUsize = AtomicUsize::new(0);
/// Rows reserved since the last drain (may exceed the log: overflow).
static CURSOR: Padded<AtomicUsize> = Padded(AtomicUsize::new(0));
/// A prune call whose deleted logs could not be read completely.
static LOST: AtomicBool = AtomicBool::new(false);
static ENABLED: AtomicBool = AtomicBool::new(false);
pub static ACTIVE: Padded<AtomicUsize> = Padded(AtomicUsize::new(0));
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static BASE: AtomicUsize = AtomicUsize::new(0);
static UNHOOKED: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
/// A hook state change since the last drain: the prunes between went unseen.
static CHANGED: AtomicBool = AtomicBool::new(true);
/// `[prune calls, calls with deleted logs, rows logged, failed reads,
/// overflowed logs, drains, rows handed over, unobserved drains, rows of
/// another session discarded, failed log offers, installed logs]`.
static COUNTERS: [AtomicU64; 11] = [const { AtomicU64::new(0) }; 11];
static OBSERVE_TICKS: AtomicU64 = AtomicU64::new(0);
static OBSERVE_MAX_TICKS: AtomicU64 = AtomicU64::new(0);

/// Times the complete call, including stack probes, initialization and returns.
struct Observation(i64);
impl Drop for Observation {
    fn drop(&mut self) {
        let ticks = crate::clock::qpc().saturating_sub(self.0).max(0) as u64;
        OBSERVE_TICKS.fetch_add(ticks, Ordering::Relaxed);
        OBSERVE_MAX_TICKS.fetch_max(ticks, Ordering::Relaxed);
    }
}

struct Call;
impl Call {
    fn enter() -> Self {
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        Self
    }
}
impl Drop for Call {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

fn failed_read() {
    COUNTERS[3].fetch_add(1, Ordering::Relaxed);
    LOST.store(true, Ordering::SeqCst);
}

/// Rows the log could not hold; the next drain installs a log of `required`.
fn overflowed(required: usize) {
    REQUIRED.fetch_max(required, Ordering::SeqCst);
    COUNTERS[4].fetch_add(1, Ordering::Relaxed);
    LOST.store(true, Ordering::SeqCst);
}

/// Before the prune of `manager` runs: log its logs about to be deleted.
// Keep the observation's stack setup inside the timed call.
#[inline(never)]
fn observe(manager: usize) {
    COUNTERS[0].fetch_add(1, Ordering::Relaxed);
    // The prune reads the same objects on this thread; they are alive here.
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    // No room: the walk counts every deleted row without reading an amount.
    let demand = match native::deleted_logs(&memory, base, manager, &mut []) {
        Ok((_, Outcome::Rows { .. })) => return,
        Ok((_, Outcome::Truncated { unread_rows, .. })) => unread_rows as usize,
        Ok((_, Outcome::Unavailable { .. })) | Err(_) => return failed_read(),
    };
    let Some(signal) = base
        .checked_add(engine_native::cgame_state::PLAY_SESSION_STARTED_AT_RVA)
        .and_then(|address| engine_native::read::<i64>(&memory, address))
    else {
        return failed_read();
    };
    record(signal, demand, |rows| {
        native::deleted_logs(&memory, base, manager, rows).map(|(_, outcome)| outcome)
    });
}

/// One prune's `demand` rows, which `read` writes into the part of its
/// reservation within the log: whole pools, a prefix of what it counted.
fn record(
    signal: i64,
    demand: usize,
    read: impl FnOnce(&mut [Row]) -> Result<Outcome, &'static str>,
) {
    COUNTERS[1].fetch_add(1, Ordering::Relaxed);
    // Tested after this call entered `ACTIVE` (see `Log`).
    let log = if HANDOFF.load(Ordering::SeqCst) {
        None
    } else {
        // SAFETY: see `Log`.
        unsafe { LOG.load(Ordering::SeqCst).as_ref() }
    };
    let Some(log) = log else {
        return overflowed(demand);
    };
    let first = CURSOR.fetch_add(demand, Ordering::SeqCst);
    let end = first.saturating_add(demand);
    let stored = end.min(log.len());
    if end > log.len() {
        overflowed(end);
    }
    if first >= stored {
        return;
    }
    // SAFETY: this writer alone reserved `first..stored`, within the log.
    let kept = match read(unsafe { log.rows_mut(first..stored) }) {
        Ok(Outcome::Rows { len }) => len,
        Ok(Outcome::Truncated { len, unread_rows }) => {
            if end <= log.len() {
                // A deleted log grew between the count and the read.
                overflowed(first + len + unread_rows as usize);
            }
            len
        }
        Ok(Outcome::Unavailable { .. }) | Err(_) => {
            failed_read();
            0
        }
    };
    for index in first..stored {
        // SAFETY: as above.
        unsafe { log.mark(index, (index < first + kept).then_some(signal)) };
    }
    COUNTERS[2].fetch_add(kept as u64, Ordering::Relaxed);
}

/// Off the hook: the log must hold `demand` rows (the native row demand of
/// every retained group log). A larger log is allocated here, outside the
/// lock, and the next drain installs it.
pub fn prepare(demand: usize) {
    let required = REQUIRED.fetch_max(demand, Ordering::SeqCst).max(demand);
    let mut swap = SWAP.lock().unwrap_or_else(PoisonError::into_inner);
    let retired = swap.retired.take();
    let held = [&swap.current, &swap.next]
        .into_iter()
        .flatten()
        .map(|log| log.len())
        .max()
        .unwrap_or(0);
    drop(swap);
    drop(retired);
    if required <= held {
        return;
    }
    let Ok(log) = engine_allocation::advisory(|| {
        required
            .checked_next_power_of_two()
            .ok_or_else(engine_allocation::oom)
            .and_then(Log::new)
    }) else {
        COUNTERS[9].fetch_add(1, Ordering::Relaxed);
        return;
    };
    let mut swap = SWAP.lock().unwrap_or_else(PoisonError::into_inner);
    if swap.next.as_ref().is_none_or(|next| next.len() < log.len()) {
        swap.next = Some(log);
    }
}

/// At the drained boundary: the offered log replaces the installed one.
fn install() {
    let Ok(mut swap) = SWAP.try_lock() else {
        return;
    };
    if swap.retired.is_some() {
        return;
    }
    let Some(next) = swap.next.take() else {
        return;
    };
    HANDOFF.store(true, Ordering::SeqCst);
    if ACTIVE.load(Ordering::SeqCst) == 0 && CURSOR.load(Ordering::SeqCst) == 0 {
        LOG.store(ptr::from_ref::<Log>(&next).cast_mut(), Ordering::SeqCst);
        CAPACITY.store(next.len(), Ordering::SeqCst);
        swap.retired = swap.current.replace(next);
        COUNTERS[10].fetch_add(1, Ordering::Relaxed);
    } else {
        swap.next = Some(next);
    }
    HANDOFF.store(false, Ordering::SeqCst);
}

/// Detach, once the hook is restored and no prune runs: frees every log.
pub fn release_storage() -> Result<(), &'static str> {
    let mut swap = SWAP.lock().unwrap_or_else(PoisonError::into_inner);
    HANDOFF.store(true, Ordering::SeqCst);
    if ACTIVE.load(Ordering::SeqCst) != 0 {
        HANDOFF.store(false, Ordering::SeqCst);
        return Err("combat log prune storage release before quiescence");
    }
    LOG.store(ptr::null_mut(), Ordering::SeqCst);
    CAPACITY.store(0, Ordering::SeqCst);
    CURSOR.store(0, Ordering::SeqCst);
    let released = std::mem::take(&mut *swap);
    HANDOFF.store(false, Ordering::SeqCst);
    drop(swap);
    drop(released);
    Ok(())
}

/// Hand the rows the session with load signal `signal` logged over into
/// `output`, from the hourly sample, their only consumer, and empty the log;
/// rows another session logged are discarded. Then install a larger log if one
/// is needed. `Err` when some prune since the last drain went unobserved
/// (hooks not installed or enabled, or changed state; rows that did not fit;
/// a read that failed): the rows it did keep are still handed over.
pub fn drain(signal: i64, output: &mut Vec<Row>) -> Result<(), &'static str> {
    COUNTERS[5].fetch_add(1, Ordering::Relaxed);
    if ACTIVE.load(Ordering::SeqCst) != 0 {
        // A prune still running (never at the sampling boundary): wait.
        COUNTERS[7].fetch_add(1, Ordering::Relaxed);
        return Err("combat log prune running during the sample");
    }
    // SAFETY: only this boundary and detach replace the installed log.
    let log = unsafe { LOG.load(Ordering::SeqCst).as_ref() };
    let reserved = log.map_or(0, |log| CURSOR.load(Ordering::SeqCst).min(log.len()));
    engine_allocation::reserve(output, reserved)
        .map_err(|_| "combat log drain allocation failed")?;
    let mut other = 0_u64;
    let mut handed = 0_u64;
    for index in 0..reserved {
        // SAFETY: no writer is active and every reserved element within the
        // log was marked (see `Log`).
        match log.map(|log| unsafe { log.read(index) }) {
            Some((Some(logged), row)) if logged == signal => {
                output.push(row);
                handed += 1;
            }
            Some((Some(_), _)) => other += 1,
            _ => {}
        }
    }
    COUNTERS[6].fetch_add(handed, Ordering::Relaxed);
    COUNTERS[8].fetch_add(other, Ordering::Relaxed);
    CURSOR.store(0, Ordering::SeqCst);
    prepare(0);
    install();
    let lost = LOST.swap(false, Ordering::SeqCst);
    let changed = CHANGED.swap(false, Ordering::SeqCst);
    let result = if UNHOOKED.get().is_some() {
        Err("combat log prune hook is not installed")
    } else if !ENABLED.load(Ordering::SeqCst) || changed {
        Err("combat log prune hook was not enabled throughout")
    } else if lost {
        Err("a pruned combat log could not be kept")
    } else {
        Ok(())
    };
    if result.is_err() {
        COUNTERS[7].fetch_add(1, Ordering::Relaxed);
    }
    result
}

pub fn status() -> impl serde::Serialize {
    let c = COUNTERS
        .each_ref()
        .map(|value| value.load(Ordering::Relaxed));
    let mut frequency = 0;
    let clock = unsafe {
        windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency)
    } != 0
        && frequency > 0;
    let ns = |counter: &AtomicU64| {
        clock.then(|| {
            (u128::from(counter.load(Ordering::Relaxed)) * 1_000_000_000 / frequency as u128) as u64
        })
    };
    let observe_total_ns = ns(&OBSERVE_TICKS);
    crate::status::status!({
        "history_events": crate::history_events::status(),
        "unhooked": UNHOOKED.get(),
        "enabled": ENABLED.load(Ordering::SeqCst),
        "prune_calls": c[0],
        "calls_with_deleted_logs": c[1],
        "rows_logged": c[2],
        "failed_reads": c[3],
        "overflowed_logs": c[4],
        "drains": c[5],
        "handed_over": c[6],
        "unobserved_drains": c[7],
        "other_session_rows_discarded": c[8],
        "capacity_rows": CAPACITY.load(Ordering::SeqCst),
        "required_rows": REQUIRED.load(Ordering::SeqCst),
        "failed_log_offers": c[9],
        "installed_logs": c[10],
        "observe_total_ns": observe_total_ns,
        "observe_mean_ns": observe_total_ns.and_then(|total| total.checked_div(c[0])),
        "observe_max_ns": ns(&OBSERVE_MAX_TICKS),
        "observe_scope": "every call's wall in observe on the executing daily task thread; includes failed and empty reads and scheduling delays; excludes the original prune; mean is not extrapolated from hook timing samples",
        "scope": "cumulative since attach; a prune call is one country's daily Combat Log prune"
    })
}

pub struct Hooks {
    hook: Option<Hook>,
    history: crate::history_events::Hooks,
}

impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// Never fails: without the hook every drain reports the prunes unobserved.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Self {
        let hook = unsafe { Self::install(memory, base) }
            .map_err(|reason| {
                let _ = UNHOOKED.set(reason);
            })
            .ok();
        Self {
            hook,
            history: unsafe { crate::history_events::Hooks::new(memory, base) },
        }
    }

    unsafe fn install(memory: &impl Memory, base: usize) -> Result<Hook, &'static str> {
        if !crate::hook::installation::sentinel(memory, base + PRUNE_RVA, PRUNE_SENTINEL) {
            return Err("combat log prune sentinel differs");
        }
        BASE.store(base, Ordering::Relaxed);
        let hook = unsafe {
            Hook::new(
                base + PRUNE_RVA,
                prune as *const () as usize,
                PRUNE_SENTINEL,
            )
        }
        .map_err(|_| "native hook construction failed")?;
        ORIGINAL.store(hook.trampoline(), Ordering::Release);
        Ok(hook)
    }

    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !self
            .history
            .set_enabled(enabled)
            .map_err(|_| "combat log history hook publication failed")?
        {
            return Ok(false);
        }
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
        }
        CHANGED.store(true, Ordering::SeqCst);
        let Some(hook) = self.hook.as_mut() else {
            return Ok(true);
        };
        if !hook
            .set_enabled(enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        ENABLED.store(enabled, Ordering::SeqCst);
        Ok(true)
    }

    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hook
            .iter()
            .flat_map(Hook::executable_ranges)
            .chain(self.history.executable_ranges())
    }
}

/// The prune takes the combat log manager pointer in RCX and returns nothing.
unsafe extern "system" fn prune(manager: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::CombatLogPrune);
    let _call = Call::enter();
    if ENABLED.load(Ordering::SeqCst) {
        let _wall = Observation(crate::clock::qpc());
        observe(manager);
    }
    let original: unsafe extern "system" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL.load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(manager) });
}

/// Tests that touch the process-wide log take this first.
#[cfg(any(test, feature = "test-support"))]
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Rows the log [`observe_for_test`] installs holds.
#[cfg(any(test, feature = "test-support"))]
pub const TEST_ROWS: usize = 1 << 12;

/// The log as an enabled, installed hook would leave it, empty, holding
/// [`TEST_ROWS`]. Hold [`test_lock`].
#[cfg(any(test, feature = "test-support"))]
pub fn observe_for_test(enabled: bool) {
    ENABLED.store(enabled, Ordering::SeqCst);
    CHANGED.store(false, Ordering::SeqCst);
    LOST.store(false, Ordering::SeqCst);
    install_for_test(TEST_ROWS);
}

/// An empty installed log of exactly `rows`, nothing offered or required.
#[cfg(any(test, feature = "test-support"))]
fn install_for_test(rows: usize) {
    assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
    let log = Log::new(rows).unwrap();
    let mut swap = SWAP.lock().unwrap_or_else(PoisonError::into_inner);
    LOG.store(ptr::from_ref::<Log>(&log).cast_mut(), Ordering::SeqCst);
    CAPACITY.store(rows, Ordering::SeqCst);
    REQUIRED.store(0, Ordering::SeqCst);
    CURSOR.store(0, Ordering::SeqCst);
    *swap = Swap {
        current: Some(log),
        next: None,
        retired: None,
    };
}

/// Rows a prune in the session with load signal `signal` would have
/// logged, as many as fit. Hold [`test_lock`].
#[cfg(any(test, feature = "test-support"))]
pub fn append_for_test(signal: i64, rows: &[Row]) {
    record(signal, rows.len(), |out| {
        out.copy_from_slice(&rows[..out.len()]);
        Ok(if out.len() == rows.len() {
            Outcome::Rows { len: rows.len() }
        } else {
            Outcome::Truncated {
                len: out.len(),
                unread_rows: (rows.len() - out.len()) as u32,
            }
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIGNAL: i64 = 1_778_888_888;

    fn row(group: u32, variant: u32) -> Row {
        Row {
            country: 7,
            group,
            kind: 0,
            month: 23_325,
            variant,
            date: 60_827_868,
            amount_raw: 100_000,
        }
    }

    #[test]
    fn drains_what_prunes_logged_once() {
        let _lock = test_lock();
        observe_for_test(true);
        append_for_test(SIGNAL, &[row(1, 1), row(1, 2)]);
        append_for_test(SIGNAL, &[row(2, 1)]);
        let mut output = Vec::new();
        assert_eq!(drain(SIGNAL, &mut output), Ok(()));
        assert_eq!(output, [row(1, 1), row(1, 2), row(2, 1)]);
        output.clear();
        assert_eq!(drain(SIGNAL, &mut output), Ok(()));
        assert!(output.is_empty());
    }

    #[test]
    fn status_reports_observation_wall_time() {
        let _lock = test_lock();
        let before = OBSERVE_TICKS.load(Ordering::Relaxed);
        // Exercise the same timer on an early-return path, without live memory.
        drop(Observation(crate::clock::qpc() - 1_000_000));
        let total = OBSERVE_TICKS.load(Ordering::Relaxed);
        let max = OBSERVE_MAX_TICKS.load(Ordering::Relaxed);
        assert!(total >= before + 1_000_000);
        assert!(max >= 1_000_000 && max <= total);
        let status = serde_json::to_value(status()).unwrap();
        assert!(status["observe_total_ns"].is_u64());
        assert!(status["observe_max_ns"].is_u64());
        assert!(status["observe_total_ns"].as_u64() >= status["observe_max_ns"].as_u64());
    }

    #[test]
    #[ignore = "synthetic per-call hook measurement"]
    fn hook_cost_combat_log_prune() {
        use engine_land_inputs::ncombat_log_corders_group_logs as logs;
        use engine_native::{corders_group, creference_registry as registry, reference_bucket};
        const CALLS: usize = 10_000;
        const MANAGER: usize = 0x100;
        const POINTERS: usize = 0x200;
        const REGISTRY: usize = 0x500;
        const BUCKETS: usize = 0x600;
        const GROUPS: usize = 0x1000;
        const GROUP_COUNT: usize = 64;
        #[inline(never)]
        unsafe extern "system" fn original(manager: usize) {
            std::hint::black_box(manager);
        }

        let _lock = test_lock();
        let mut bytes = vec![0_u64; (GROUPS + GROUP_COUNT * logs::SIZE).div_ceil(8)];
        let at = bytes.as_ptr() as usize;
        let base = at - (registry::LOW_ROOTS_RVA + corders_group::REFERENCE_TYPE as usize * 8);
        let handle = (1_u64 << 32) | u64::from(corders_group::REFERENCE_TYPE);
        let hash = handle
            .rotate_left(32)
            .wrapping_mul(registry::HASH_MULTIPLIER);
        let bucket = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
        let mut word = |offset: usize, value: u64| bytes[offset / 8] = value;
        word(0, (at + REGISTRY) as u64);
        word(REGISTRY + registry::ENTRIES, (at + BUCKETS) as u64);
        word(REGISTRY + registry::COUNT, 1 | (7_u64 << 32));
        word(REGISTRY + registry::OVERFLOW_DISTANCE, 5);
        word(
            BUCKETS + 13 * reference_bucket::STRIDE,
            u64::from(reference_bucket::TERMINAL_DISTANCE) << (reference_bucket::DISTANCE * 8),
        );
        word(
            BUCKETS + bucket * reference_bucket::STRIDE,
            1_u64 << (reference_bucket::DISTANCE * 8),
        );
        word(
            BUCKETS + bucket * reference_bucket::STRIDE + reference_bucket::TYPE,
            handle,
        );
        word(
            BUCKETS + bucket * reference_bucket::STRIDE + reference_bucket::OBJECT,
            1,
        );
        word(MANAGER, (base + native::VTABLE_RVA) as u64);
        word(MANAGER + native::GROUPS, (at + POINTERS) as u64);
        word(MANAGER + native::COUNTRY, 7);
        for index in 0..GROUP_COUNT {
            let group = GROUPS + index * logs::SIZE;
            word(POINTERS + index * 8, (at + group) as u64);
            word(group, (base + logs::VTABLE_RVA) as u64);
            word(group + logs::GROUP, handle);
        }
        BASE.store(base, Ordering::Relaxed);
        ORIGINAL.store(original as *const () as usize, Ordering::Release);
        observe_for_test(true);
        let manager = at + MANAGER;
        let native = collector_test_support::per_call_ns(CALLS, || {
            for _ in 0..CALLS {
                unsafe { original(std::hint::black_box(manager)) };
            }
        });
        for groups in [0, 1, 16, GROUP_COUNT] {
            bytes[(MANAGER + native::GROUPS + 8) / 8] = groups as u64 | ((groups as u64) << 32);
            collector_test_support::with_allocations_denied(|| unsafe { prune(manager) });
            let before = COUNTERS[0].load(Ordering::Relaxed);
            let hooked = collector_test_support::per_call_ns(CALLS, || {
                for _ in 0..CALLS {
                    unsafe { prune(std::hint::black_box(manager)) };
                }
            });
            assert_eq!(
                COUNTERS[0].load(Ordering::Relaxed) - before,
                8 * CALLS as u64
            );
            assert_eq!(COUNTERS[3].load(Ordering::Relaxed), 0);
            assert_eq!(CURSOR.load(Ordering::SeqCst), 0);
            assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
            let status = serde_json::to_value(status()).unwrap();
            assert_eq!(
                status["observe_mean_ns"].as_u64(),
                Some(
                    status["observe_total_ns"].as_u64().unwrap()
                        / status["prune_calls"].as_u64().unwrap()
                )
            );
            println!(
                "hook_cost combat_log_prune {groups} live groups: {:.1} ns/call (hooked {hooked:.1}, native {native:.1})",
                hooked - native
            );
        }
        std::hint::black_box(bytes);
    }

    /// Rows that do not fit keep the prefix that did, make the drain
    /// unobserved, and size the log the drain installs; a disabled hook is
    /// unobserved.
    #[test]
    fn rows_that_do_not_fit_keep_their_prefix_and_grow_the_log() {
        let _lock = test_lock();
        observe_for_test(true);
        install_for_test(8);
        append_for_test(SIGNAL, &[row(1, 1); 6]);
        append_for_test(SIGNAL, &[row(2, 1), row(2, 2), row(2, 3), row(2, 4)]);
        let mut output = Vec::new();
        assert!(drain(SIGNAL, &mut output).is_err());
        assert_eq!(output.len(), 8);
        assert_eq!(output[6..], [row(2, 1), row(2, 2)]);
        // The failed drain installed a log for the ten rows.
        assert!(CAPACITY.load(Ordering::SeqCst) >= 10);
        output.clear();
        append_for_test(SIGNAL, &[row(1, 1); 6]);
        append_for_test(SIGNAL, &[row(2, 1), row(2, 2), row(2, 3), row(2, 4)]);
        assert_eq!(drain(SIGNAL, &mut output), Ok(()));
        assert_eq!(output.len(), 10);
        observe_for_test(false);
        assert!(drain(SIGNAL, &mut output).is_err());
    }

    /// No fixed count limits one prune or the rows between drains: the log
    /// holds the native demand it was prepared for.
    #[test]
    fn a_prepared_log_keeps_every_row_of_large_prunes() {
        let _lock = test_lock();
        observe_for_test(true);
        prepare(40_000);
        let mut output = Vec::new();
        assert_eq!(drain(SIGNAL, &mut output), Ok(()));
        assert!(CAPACITY.load(Ordering::SeqCst) >= 40_000);
        let mut expected = 0;
        for group in 0..20 {
            let rows: Vec<Row> = (0..2_000).map(|variant| row(group, variant)).collect();
            append_for_test(SIGNAL, &rows);
            expected += rows.len();
        }
        assert_eq!(drain(SIGNAL, &mut output), Ok(()));
        assert_eq!(output.len(), expected);
        assert!(
            output
                .chunks(2_000)
                .enumerate()
                .all(|(group, rows)| rows.iter().all(|row| row.group == group as u32))
        );
    }

    /// Before any log is installed a prune is lost and requests its rows; a
    /// read that stops early leaves only what it read; a failed read keeps
    /// nothing of its own while other prunes' rows are handed over.
    #[test]
    fn unread_reservations_are_never_handed_over() {
        let _lock = test_lock();
        observe_for_test(true);
        release_storage().unwrap();
        append_for_test(SIGNAL, &[row(1, 1); 8]);
        assert!(REQUIRED.load(Ordering::SeqCst) >= 8);
        let mut output = Vec::new();
        assert!(drain(SIGNAL, &mut output).is_err());
        assert!(output.is_empty());
        assert!(CAPACITY.load(Ordering::SeqCst) >= 8);
        // Fewer rows than counted: a deleted log shrank between the reads.
        record(SIGNAL, 3, |out| {
            out[0] = row(2, 1);
            Ok(Outcome::Rows { len: 1 })
        });
        record(SIGNAL, 2, |out| {
            out[0] = row(3, 1);
            Err("combat log pool variant unreadable")
        });
        append_for_test(SIGNAL, &[row(4, 1)]);
        assert!(drain(SIGNAL, &mut output).is_err());
        assert_eq!(output, [row(2, 1), row(4, 1)]);
        output.clear();
        record(SIGNAL, 3, |out| {
            out[0] = row(5, 1);
            Ok(Outcome::Rows { len: 1 })
        });
        assert_eq!(drain(SIGNAL, &mut output), Ok(()));
        assert_eq!(output, [row(5, 1)]);
    }

    /// A load between two samples: the rows the left session logged after
    /// its last sample never reach the new branch.
    #[test]
    fn rows_another_session_logged_are_discarded() {
        let _lock = test_lock();
        observe_for_test(true);
        append_for_test(SIGNAL - 1, &[row(1, 1), row(1, 2)]);
        append_for_test(SIGNAL, &[row(2, 1)]);
        let mut output = Vec::new();
        assert_eq!(drain(SIGNAL, &mut output), Ok(()));
        assert_eq!(output, [row(2, 1)]);
    }

    /// Many writers with reservations of different lengths race, each inside
    /// a prune call as the detour runs them: every row arrives once, and
    /// each reservation stays whole and in order.
    #[test]
    fn concurrent_writers_never_share_a_row() {
        const WRITERS: u32 = 8;
        const CALLS: u32 = 400;
        let len = |writer: u32, call: u32| 1 + (writer + call) % 5;
        let _lock = test_lock();
        observe_for_test(true);
        let expected: u32 = (0..WRITERS)
            .flat_map(|writer| (0..CALLS).map(move |call| len(writer, call)))
            .sum();
        install_for_test(expected as usize);
        let start = std::sync::Barrier::new(WRITERS as usize);
        std::thread::scope(|scope| {
            for writer in 0..WRITERS {
                let start = &start;
                scope.spawn(move || {
                    start.wait();
                    for call in 0..CALLS {
                        let rows: Vec<Row> = (0..len(writer, call))
                            .map(|index| Row {
                                country: writer,
                                amount_raw: i64::from(index),
                                ..row(call, len(writer, call))
                            })
                            .collect();
                        let _call = Call::enter();
                        append_for_test(SIGNAL, &rows);
                    }
                });
            }
        });
        let mut output = Vec::new();
        assert_eq!(drain(SIGNAL, &mut output), Ok(()));
        assert_eq!(output.len(), expected as usize);
        // Each reservation is contiguous: a call's rows follow each other.
        let mut at = 0;
        let mut calls = Vec::new();
        while at < output.len() {
            let first = output[at];
            calls.push((first.country, first.group));
            for index in 0..first.variant as usize {
                let row = output[at + index];
                assert_eq!(
                    (row.country, row.group, row.amount_raw),
                    (first.country, first.group, index as i64)
                );
            }
            at += first.variant as usize;
        }
        calls.sort_unstable();
        calls.dedup();
        assert_eq!(calls.len(), (WRITERS * CALLS) as usize);
    }
}
