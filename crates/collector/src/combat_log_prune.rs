//! The final values of every Combat Log a daily prune deletes.
//!
//! The game deletes a group's Combat Log in its daily prune
//! ([`engine::ncombat_log_cmanager::PRUNE_RVA`], per country inside
//! CGameState::DailyUpdate) once the group's orders group is gone. Orders
//! groups can be deleted inside the hourly update that runs the prune, after
//! the hourly sample, so no sample can see those logs' last changes. A detour
//! at the prune's entry reads, before the original runs, every log it is
//! about to delete for that reason ([`engine::ncombat_log_cmanager::deleted_logs`])
//! into a bounded lock-free log; nothing can change those logs any more, so
//! the rows are their final values. The detour only reads and calls the
//! original; it never changes game behaviour and calls no game function.
//!
//! DailyUpdate@1401d4390 dispatches its lambda_3 as a TBB start_for task;
//! its vf1@1401f6e20 -> 1401b7b80 -> range body 1401ba1e0 calls 1406db660
//! for each country, which calls the prune at 1406dba6a. This is a daily job
//! worker path; the simulation thread may also execute the task. Its wall
//! cost belongs to that thread, outside the hourly callback's timing.
//! The log is multi-producer: a writer reserves its rows with one `fetch_add`.
//! The hourly callback, which runs while no daily update does, hands them over
//! ([`drain`]). A log that fills, a read that fails or does not fit, or hooks
//! that are not installed or enabled make that drain unobserved: the table
//! then says so for the hour (`CombatLogState.prune_observed`).
//!
//! Each row carries the load signal (the global
//! [`engine::cgame_state::PLAY_SESSION_STARTED_AT_RVA`] every load sets) of
//! the session whose prune logged it. The drain hands over only the rows of
//! the sample's own session: rows an earlier session logged after its last
//! sample describe a branch the player left, and are discarded and counted.
//!
//! Stack: `[Row; CALL_ROWS]` is 32 KB. `deleted_logs` nests the bounded
//! `for_each_group`, `read_entries` and `read_pool` batch buffers: about
//! 60 KB together, before native callers and guarded-read frames. The pinned
//! executable's PE SizeOfStackReserve is 4 MB. The TBB workers live in
//! TBB.DLL; their actual stack reserve is not established by the executable
//! task path alone. A live run must check it and measure `observe_total_ns`
//! and `observe_max_ns`; a smaller worker stack needs a smaller batch/row
//! footprint. Observation stays allocation-free and lock-free.
use crate::{hook::Hook, opinion_changes::Padded};
use engine::{
    Memory,
    ncombat_log_cmanager::{self as native, PRUNE_RVA, PRUNE_SENTINEL, Row},
    walk::Outcome,
};
use std::{
    cell::UnsafeCell,
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

/// Rows the log holds between two drains. A deleted group's log holds the
/// retention define's months of five arrays of its variants.
pub const LOG: usize = 1 << 15;
/// Rows one prune call may read into its stack buffer.
const CALL_ROWS: usize = 1024;

/// A logged row and the load signal of the session whose prune logged it.
#[derive(Clone, Copy)]
struct Logged {
    signal: i64,
    row: Row,
}

/// The log's rows. Only raw element pointers are ever made from it, never a
/// reference to the array: concurrent writers each touch only the elements
/// they reserved, and a `&mut` to the whole array would alias theirs.
///
/// Safety argument, for every access below:
/// - A writer reserves `first..first + len` with one `CURSOR.fetch_add`, so
///   two writers' ranges never overlap; it writes only its range, and only
///   when the range ends within [`LOG`].
/// - Each writer runs inside a [`Call`], which increments `ACTIVE` before the
///   reservation and decrements it (`SeqCst`) after its last write.
/// - The drain runs at the hourly sample, when no prune can run: the prune
///   runs only inside CGameState::DailyUpdate, which the hourly update runs
///   after the sample, and the callback holds the game thread while it
///   drains. It reads only after loading `ACTIVE` as 0: every `ACTIVE` change
///   is a read-modify-write, so that load synchronises with every writer's
///   final decrement, and all their writes happen before the drain's reads.
///   A running writer (a violated precondition) makes the drain refuse.
/// - The drain reads only reserved elements below the first overflowing
///   reservation, which were all written.
struct Slots(UnsafeCell<[Logged; LOG]>);
// SAFETY: see the safety argument above.
unsafe impl Sync for Slots {}
static SLOTS: Slots = Slots(UnsafeCell::new(
    [Logged {
        signal: 0,
        row: Row {
            country: 0,
            group: 0,
            kind: 0,
            month: 0,
            variant: 0,
            date: 0,
            amount_raw: 0,
        },
    }; LOG],
));
/// Rows reserved since the last drain (may exceed [`LOG`]: overflow).
static CURSOR: Padded<AtomicUsize> = Padded(AtomicUsize::new(0));
/// Where the first reservation that did not fit starts: its rows and every
/// later reservation's were never written, so the drain stops there.
static OVERFLOW_AT: AtomicUsize = AtomicUsize::new(usize::MAX);
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
/// another session discarded]`.
static COUNTERS: [AtomicU64; 9] = [const { AtomicU64::new(0) }; 9];
static OBSERVE_TICKS: AtomicU64 = AtomicU64::new(0);
static OBSERVE_MAX_TICKS: AtomicU64 = AtomicU64::new(0);

/// Times the complete call, including stack probes, initialization and returns.
struct Observation(i64);
impl Drop for Observation {
    fn drop(&mut self) {
        let ticks = crate::sampling::qpc().saturating_sub(self.0).max(0) as u64;
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

/// Before the prune of `manager` runs: log its logs about to be deleted.
// Keep the observation's stack setup inside the timed call.
#[inline(never)]
fn observe(manager: usize) {
    COUNTERS[0].fetch_add(1, Ordering::Relaxed);
    let mut rows = [Row::default(); CALL_ROWS];
    // The prune reads the same objects on this thread; they are alive here.
    let memory = unsafe { engine::memory::LiveMemory::new() };
    let len = match native::deleted_logs(&memory, BASE.load(Ordering::Relaxed), manager, &mut rows)
    {
        Ok((_, Outcome::Rows { len })) => len,
        Ok(_) | Err(_) => {
            COUNTERS[3].fetch_add(1, Ordering::Relaxed);
            LOST.store(true, Ordering::SeqCst);
            return;
        }
    };
    if len == 0 {
        return;
    }
    let Some(signal) = BASE
        .load(Ordering::Relaxed)
        .checked_add(engine::cgame_state::PLAY_SESSION_STARTED_AT_RVA)
        .and_then(|address| engine::read::<i64>(&memory, address))
    else {
        COUNTERS[3].fetch_add(1, Ordering::Relaxed);
        LOST.store(true, Ordering::SeqCst);
        return;
    };
    append(signal, &rows[..len]);
}

fn append(signal: i64, rows: &[Row]) {
    COUNTERS[1].fetch_add(1, Ordering::Relaxed);
    let first = CURSOR.fetch_add(rows.len(), Ordering::SeqCst);
    let Some(end) = first.checked_add(rows.len()).filter(|&end| end <= LOG) else {
        COUNTERS[4].fetch_add(1, Ordering::Relaxed);
        OVERFLOW_AT.fetch_min(first, Ordering::SeqCst);
        LOST.store(true, Ordering::SeqCst);
        return;
    };
    // Element pointers only (see `Slots`).
    let slots = SLOTS.0.get().cast::<Logged>();
    for (index, &row) in (first..end).zip(rows) {
        // SAFETY: this writer alone reserved `first..end`, which ends within
        // the array (see `Slots`).
        unsafe { slots.add(index).write(Logged { signal, row }) };
    }
    COUNTERS[2].fetch_add(rows.len() as u64, Ordering::Relaxed);
}

/// Hand the rows the session with load signal `signal` logged over into
/// `output`, from the hourly sample, their only consumer, and empty the log;
/// rows another session logged are discarded. `Err` when some prune since
/// the last drain went unobserved (hooks not installed or enabled, or changed
/// state; a log that filled; a read that failed or did not fit): the rows it
/// did keep are still handed over.
pub fn drain(signal: i64, output: &mut Vec<Row>) -> Result<(), &'static str> {
    COUNTERS[5].fetch_add(1, Ordering::Relaxed);
    if ACTIVE.load(Ordering::SeqCst) != 0 {
        // A prune still running (never at the sampling boundary): wait.
        COUNTERS[7].fetch_add(1, Ordering::Relaxed);
        return Err("combat log prune running during the sample");
    }
    let reserved = CURSOR
        .load(Ordering::SeqCst)
        .min(OVERFLOW_AT.load(Ordering::SeqCst))
        .min(LOG);
    engine::allocation::reserve(output, reserved)
        .map_err(|_| "combat log drain allocation failed")?;
    // Element pointers only (see `Slots`).
    let slots = SLOTS.0.get().cast::<Logged>().cast_const();
    let mut other = 0_u64;
    for index in 0..reserved {
        // SAFETY: no writer is active and the first `reserved` elements were
        // all written (see `Slots`).
        let logged = unsafe { slots.add(index).read() };
        if logged.signal == signal {
            output.push(logged.row);
        } else {
            other += 1;
        }
    }
    COUNTERS[6].fetch_add(reserved as u64 - other, Ordering::Relaxed);
    COUNTERS[8].fetch_add(other, Ordering::Relaxed);
    CURSOR.store(0, Ordering::SeqCst);
    OVERFLOW_AT.store(usize::MAX, Ordering::SeqCst);
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
    crate::status::status!({
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
        "observe_total_ns": ns(&OBSERVE_TICKS),
        "observe_max_ns": ns(&OBSERVE_MAX_TICKS),
        "observe_scope": "wall in observe on the executing daily task thread; includes failed and empty reads; excludes the original prune",
        "scope": "cumulative since attach; a prune call is one country's daily Combat Log prune"
    })
}

pub struct Hooks {
    hook: Option<Hook>,
}

impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// Never fails: without the hook every drain reports the prunes unobserved.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Self {
        let hook = unsafe { Self::install(memory, base) }
            .map_err(|reason| {
                let _ = UNHOOKED.set(reason);
            })
            .ok();
        Self { hook }
    }

    unsafe fn install(memory: &impl Memory, base: usize) -> Result<Hook, &'static str> {
        if engine::read::<[u8; 16]>(memory, base + PRUNE_RVA) != Some(PRUNE_SENTINEL) {
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
        self.hook.iter().flat_map(Hook::executable_ranges)
    }
}

/// The prune takes the CManager* in RCX and returns nothing.
unsafe extern "system" fn prune(manager: usize) {
    let _call = Call::enter();
    if ENABLED.load(Ordering::SeqCst) {
        let _wall = Observation(crate::sampling::qpc());
        observe(manager);
    }
    let original: unsafe extern "system" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL.load(Ordering::Acquire)) };
    unsafe { original(manager) };
}

/// Tests that touch the process-wide log take this first.
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The log as an enabled, installed hook would leave it, empty.
#[cfg(test)]
pub(crate) fn observe_for_test(enabled: bool) {
    ENABLED.store(enabled, Ordering::SeqCst);
    CHANGED.store(false, Ordering::SeqCst);
    LOST.store(false, Ordering::SeqCst);
    CURSOR.store(0, Ordering::SeqCst);
    OVERFLOW_AT.store(usize::MAX, Ordering::SeqCst);
}

/// Rows a prune in the session with load signal `signal` would have
/// logged. Hold [`test_lock`].
#[cfg(test)]
pub(crate) fn append_for_test(signal: i64, rows: &[Row]) {
    append(signal, rows);
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
        drop(Observation(crate::sampling::qpc() - 1_000_000));
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
    fn a_full_log_or_a_disabled_hook_is_unobserved() {
        let _lock = test_lock();
        observe_for_test(true);
        append_for_test(SIGNAL, &vec![row(1, 1); LOG - 1]);
        append_for_test(SIGNAL, &[row(2, 1), row(2, 2)]);
        let mut output = Vec::new();
        assert!(drain(SIGNAL, &mut output).is_err());
        assert_eq!(output.len(), LOG - 1);
        // The next drain is observed again.
        output.clear();
        assert_eq!(drain(SIGNAL, &mut output), Ok(()));
        observe_for_test(false);
        assert!(drain(SIGNAL, &mut output).is_err());
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
        let expected: u32 = (0..WRITERS)
            .flat_map(|writer| (0..CALLS).map(move |call| len(writer, call)))
            .sum();
        assert!((expected as usize) < LOG);
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
