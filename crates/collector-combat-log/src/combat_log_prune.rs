use crate::{hook::Hook, opinion_changes::Padded};
use collector_primitives::spill_stack::SpillStack;
use engine_land_inputs::{
    ncombat_log_cmanager as native, ncombat_log_cmanager::PRUNE_RVA, ncombat_log_cmanager::Row,
};
use engine_native_memory::{Memory, walk::Outcome};
use std::{
    cell::UnsafeCell,
    ops::Range,
    ptr,
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};

#[doc = " The rows prunes reserved since the last drain, and the load signal of the"]
#[doc = " session that logged each (`None`: reserved, but not read)."]
struct Log {
    rows: Box<[UnsafeCell<Row>]>,
    signals: Box<[UnsafeCell<Option<i64>>]>,
}
// SAFETY: every access to a log's cells follows this protocol.
// - A writer reserves `first..first + len` with one `CURSOR.fetch_add`, so
//   two writers' ranges never overlap. It writes only the part of its range
//   within the log, and marks every element of that part.
// - Each native writer runs inside a [`Call`], which increments `ACTIVE` before
//   the reservation and decrements it (`SeqCst`) after its last write. Tests
//   write through `record` too, so `HANDOFF` and `CURSOR` apply to them; a test
//   writer that runs off the draining thread holds a `Call` and is joined
//   before the drain. `install_for_test` replaces the log without `HANDOFF`,
//   only with `ACTIVE` at 0, and `test_lock` keeps test cases apart.
// - The drain runs inside the capture boundary, which keeps new prune
//   writers out for the whole drain: the game thread is held, or a prune,
//   which runs in the game's command batch, waits at the capture gate. It
//   reads only after loading `ACTIVE` as 0: every `ACTIVE` change is a
//   read-modify-write, so that load synchronises with every earlier writer's
//   final decrement, and all their writes happen before the drain's reads.
//   A running writer (a violated precondition) makes the drain refuse.
// - The drain reads only reserved elements within the log, all marked.
// - A log's published pointer is removed only with `HANDOFF` set and `ACTIVE`
//   then loaded as 0; a writer tests `HANDOFF` after entering its call, and
//   loads `LOG` only when it is clear. So no writer holds a removed log, which
//   may then be freed later without `HANDOFF`.
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
    /// The caller alone reserved `range`, which lies within the log, and makes no other borrow of
    /// it while the returned slice lives.
    #[allow(clippy::mut_from_ref)]
    unsafe fn rows_mut(&self, range: Range<usize>) -> &mut [Row] {
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
static HANDOFF: AtomicBool = AtomicBool::new(false);
static CAPACITY: AtomicUsize = AtomicUsize::new(0);
static REQUIRED: AtomicUsize = AtomicUsize::new(0);
static CURSOR: Padded<AtomicUsize> = Padded(AtomicUsize::new(0));
static LOST: AtomicBool = AtomicBool::new(false);
static SPILL: SpillStack<Spilled> = SpillStack::new();
static HELD: Mutex<Vec<Spilled>> = Mutex::new(Vec::new());
static STORAGE_LOST: AtomicBool = AtomicBool::new(false);
const STORAGE_FAILED: &str = "combat log prune row storage allocation failed";
struct Spilled {
    first: usize,
    signal: i64,
    rows: Vec<Row>,
}
fn held() -> std::sync::MutexGuard<'static, Vec<Spilled>> {
    let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    if SPILL.take_into(&mut held).is_err() {
        STORAGE_LOST.store(true, Ordering::SeqCst);
    }
    held
}
static ENABLED: AtomicBool = AtomicBool::new(false);
static ACTIVE: Padded<AtomicUsize> = Padded(AtomicUsize::new(0));
pub fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static BASE: AtomicUsize = AtomicUsize::new(0);
static UNHOOKED: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
static CHANGED: AtomicBool = AtomicBool::new(true);
static COUNTERS: [AtomicU64; 11] = [const { AtomicU64::new(0) }; 11];
static OBSERVE_TICKS: AtomicU64 = AtomicU64::new(0);
static OBSERVE_MAX_TICKS: AtomicU64 = AtomicU64::new(0);

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

fn spilled(required: usize) {
    REQUIRED.fetch_max(required, Ordering::SeqCst);
    COUNTERS[4].fetch_add(1, Ordering::Relaxed);
}

#[inline(never)]
fn observe(manager: usize) {
    COUNTERS[0].fetch_add(1, Ordering::Relaxed);
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    let demand = match native::deleted_logs(&memory, base, manager, &mut []) {
        Ok((_, Outcome::Rows { .. })) => return,
        Ok((_, Outcome::Truncated { unread_rows, .. })) => unread_rows as usize,
        Ok((_, Outcome::Unavailable { .. })) | Err(_) => return failed_read(),
    };
    let Some(signal) = engine_sites::at(
        base,
        engine_native::cgame_state::PLAY_SESSION_STARTED_AT_RVA,
    )
    .and_then(|address| engine_native_memory::read::<i64>(&memory, address)) else {
        return failed_read();
    };
    record(signal, demand, |rows| {
        native::deleted_logs(&memory, base, manager, rows).map(|(_, outcome)| outcome)
    });
}

fn record(
    signal: i64,
    demand: usize,
    mut read: impl FnMut(&mut [Row]) -> Result<Outcome, &'static str>,
) {
    COUNTERS[1].fetch_add(1, Ordering::Relaxed);
    let log = if HANDOFF.load(Ordering::SeqCst) {
        None
    } else {
        unsafe { LOG.load(Ordering::SeqCst).as_ref() }
    };
    let first = CURSOR.fetch_add(demand, Ordering::SeqCst);
    let end = first.saturating_add(demand);
    let Some(log) = log.filter(|log| end <= log.len()) else {
        spilled(end);
        if let Some(log) = log {
            for index in first..end.min(log.len()) {
                // SAFETY: this writer alone reserved `first..end`.
                unsafe { log.mark(index, None) };
            }
        }
        return spill_rows(first, signal, demand, read);
    };
    // SAFETY: this writer alone reserved `first..end`, within the log.
    let kept = match read(unsafe { log.rows_mut(first..end) }) {
        Ok(Outcome::Rows { len }) => len,
        Ok(Outcome::Truncated { len, unread_rows }) => {
            for index in first..end {
                unsafe { log.mark(index, None) };
            }
            let required = len.saturating_add(unread_rows as usize);
            spilled(first.saturating_add(required));
            return spill_rows(first, signal, required, read);
        }
        Ok(Outcome::Unavailable { .. }) | Err(_) => {
            failed_read();
            0
        }
    };
    for index in first..end {
        unsafe { log.mark(index, (index < first + kept).then_some(signal)) };
    }
    COUNTERS[2].fetch_add(kept as u64, Ordering::Relaxed);
}

fn spill_rows(
    first: usize,
    signal: i64,
    mut demand: usize,
    mut read: impl FnMut(&mut [Row]) -> Result<Outcome, &'static str>,
) {
    let mut rows = Vec::new();
    loop {
        if engine_allocation::advisory(|| engine_allocation::resize_default(&mut rows, demand))
            .is_err()
        {
            STORAGE_LOST.store(true, Ordering::SeqCst);
            return;
        }
        match read(&mut rows) {
            Ok(Outcome::Rows { len }) => {
                rows.truncate(len);
                break;
            }
            Ok(Outcome::Truncated { len, unread_rows }) => {
                demand = len.saturating_add(unread_rows as usize);
            }
            Ok(Outcome::Unavailable { .. }) | Err(_) => return failed_read(),
        }
    }
    let kept = rows.len() as u64;
    if SPILL
        .push(Spilled {
            first,
            signal,
            rows,
        })
        .is_err()
    {
        STORAGE_LOST.store(true, Ordering::SeqCst);
        return;
    }
    COUNTERS[2].fetch_add(kept, Ordering::Relaxed);
}

pub(crate) fn prepare(demand: usize) {
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

/// # Safety
/// Caller must own session detach/teardown after prune hooks are disabled and quiesced (or hold
/// `test_lock()` in an isolated test with no live hooks or concurrent consumers).
pub unsafe fn release_storage() -> Result<(), &'static str> {
    let mut swap = SWAP.lock().unwrap_or_else(PoisonError::into_inner);
    HANDOFF.store(true, Ordering::SeqCst);
    if ACTIVE.load(Ordering::SeqCst) != 0 {
        HANDOFF.store(false, Ordering::SeqCst);
        return Err("combat log prune storage release before quiescence");
    }
    LOG.store(ptr::null_mut(), Ordering::SeqCst);
    CAPACITY.store(0, Ordering::SeqCst);
    CURSOR.store(0, Ordering::SeqCst);
    STORAGE_LOST.store(false, Ordering::SeqCst);
    let released = (std::mem::take(&mut *swap), std::mem::take(&mut *held()));
    HANDOFF.store(false, Ordering::SeqCst);
    drop(swap);
    drop(released);
    Ok(())
}

/// # Safety
/// Caller must run on the admitted hourly capture boundary as the sole consumer of prune rows
/// while prune storage stays live and teardown is excluded.
pub(crate) unsafe fn drain(signal: i64, output: &mut Vec<Row>) -> Result<(), &'static str> {
    COUNTERS[5].fetch_add(1, Ordering::Relaxed);
    if ACTIVE.load(Ordering::SeqCst) != 0 {
        COUNTERS[7].fetch_add(1, Ordering::Relaxed);
        return Err("combat log prune running during the sample");
    }
    // SAFETY: only this boundary and detach replace the installed log.
    let log = unsafe { LOG.load(Ordering::SeqCst).as_ref() };
    let reserved = log.map_or(0, |log| CURSOR.load(Ordering::SeqCst).min(log.len()));
    let mut pending = held();
    let spilled_rows = pending.iter().map(|spilled| spilled.rows.len()).sum::<usize>();
    engine_allocation::reserve(output, reserved.saturating_add(spilled_rows))
        .map_err(|_| "combat log drain allocation failed")?;
    pending.sort_unstable_by_key(|spilled| spilled.first);
    let mut other = 0_u64;
    let mut handed = 0_u64;
    let mut take = |logged: i64, row: Row| {
        if logged == signal {
            output.push(row);
            handed += 1;
        } else {
            other += 1;
        }
    };
    let mut spills = pending.drain(..).peekable();
    for index in 0..reserved {
        while let Some(spilled) = spills.next_if(|spilled| spilled.first <= index) {
            spilled.rows.iter().for_each(|&row| take(spilled.signal, row));
        }
        // SAFETY: no writer is active and every reserved element within the
        // log was marked (see the `Sync` impl for `Log`).
        if let Some((Some(logged), row)) = log.map(|log| unsafe { log.read(index) }) {
            take(logged, row);
        }
    }
    spills.for_each(|spilled| spilled.rows.iter().for_each(|&row| take(spilled.signal, row)));
    drop(pending);
    COUNTERS[6].fetch_add(handed, Ordering::Relaxed);
    COUNTERS[8].fetch_add(other, Ordering::Relaxed);
    CURSOR.store(0, Ordering::SeqCst);
    prepare(0);
    install();
    let lost = LOST.swap(false, Ordering::SeqCst);
    let storage_lost = STORAGE_LOST.swap(false, Ordering::SeqCst);
    let changed = CHANGED.swap(false, Ordering::SeqCst);
    let result = if UNHOOKED.get().is_some() {
        Err("combat log prune hook is not installed")
    } else if !ENABLED.load(Ordering::SeqCst) || changed {
        Err("combat log prune hook was not enabled throughout")
    } else if storage_lost {
        Err(STORAGE_FAILED)
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
        "spilled_reservations": c[4],
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
    /// The runtime's build gate admitted this family before construction.
    /// Never fails: without the hook every drain reports the prunes unobserved.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Self {
        let hook = unsafe { Self::install(base) }
            .map_err(|reason| {
                let _ = UNHOOKED.set(reason);
            })
            .ok();
        Self {
            hook,
            history: unsafe { crate::history_events::Hooks::new(memory, base) },
        }
    }

    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    unsafe fn install(base: usize) -> Result<Hook, &'static str> {
        crate::located(base, PRUNE_RVA)?;
        let code = engine_sites::hook(base, PRUNE_RVA)?;
        BASE.store(base, Ordering::Relaxed);
        let hook = unsafe {
            Hook::new(
                code.address,
                prune as *const () as usize,
                code.sentinel::<16>()?,
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

#[cfg(any(test, feature = "test-support"))]
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
const TEST_ROWS: usize = 1 << 12;

#[cfg(test)]
pub(crate) fn observe_for_test(enabled: bool) {
    ENABLED.store(enabled, Ordering::SeqCst);
    CHANGED.store(false, Ordering::SeqCst);
    LOST.store(false, Ordering::SeqCst);
    held().clear();
    STORAGE_LOST.store(false, Ordering::SeqCst);
    install_for_test(TEST_ROWS);
}

#[cfg(test)]
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

#[cfg(test)]
pub(crate) fn append_for_test(signal: i64, rows: &[Row]) {
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
        assert_eq!(unsafe { drain(SIGNAL, &mut output) }, Ok(()));
        assert_eq!(output, [row(1, 1), row(1, 2), row(2, 1)]);
        output.clear();
        assert_eq!(unsafe { drain(SIGNAL, &mut output) }, Ok(()));
        assert!(output.is_empty());
    }

    #[test]
    fn status_reports_observation_wall_time() {
        let _lock = test_lock();
        let before = OBSERVE_TICKS.load(Ordering::Relaxed);
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
        use engine_native::corders_group;
        use engine_native_parts::{creference_registry as registry, reference_bucket};
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
        let base = at - (registry::LOW_ROOTS_RVA.0 + corders_group::REFERENCE_TYPE as usize * 8);
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
        word(MANAGER, (base + native::VTABLE_RVA.0) as u64);
        word(MANAGER + native::GROUPS, (at + POINTERS) as u64);
        word(MANAGER + native::COUNTRY, 7);
        for index in 0..GROUP_COUNT {
            let group = GROUPS + index * logs::SIZE;
            word(POINTERS + index * 8, (at + group) as u64);
            word(group, (base + logs::VTABLE_RVA.0) as u64);
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

    #[test]
    fn rows_past_the_log_are_kept_whole_in_order_and_grow_the_log() {
        let _lock = test_lock();
        observe_for_test(true);
        install_for_test(8);
        append_for_test(SIGNAL, &[row(1, 1); 6]);
        append_for_test(SIGNAL, &[row(2, 1), row(2, 2), row(2, 3), row(2, 4)]);
        append_for_test(SIGNAL, &[row(3, 1)]);
        let mut output = Vec::new();
        assert_eq!(unsafe { drain(SIGNAL, &mut output) }, Ok(()));
        assert_eq!(output.len(), 11);
        assert_eq!(
            output[6..],
            [row(2, 1), row(2, 2), row(2, 3), row(2, 4), row(3, 1)]
        );
        assert!(held().is_empty());
        assert!(CAPACITY.load(Ordering::SeqCst) >= 11);
        output.clear();
        append_for_test(SIGNAL, &[row(1, 1); 6]);
        append_for_test(SIGNAL, &[row(2, 1), row(2, 2), row(2, 3), row(2, 4)]);
        assert_eq!(unsafe { drain(SIGNAL, &mut output) }, Ok(()));
        assert_eq!(output.len(), 10);
        observe_for_test(false);
        assert!(unsafe { drain(SIGNAL, &mut output) }.is_err());
    }

    #[test]
    fn a_prepared_log_keeps_every_row_of_large_prunes() {
        let _lock = test_lock();
        observe_for_test(true);
        prepare(40_000);
        let mut output = Vec::new();
        assert_eq!(unsafe { drain(SIGNAL, &mut output) }, Ok(()));
        assert!(CAPACITY.load(Ordering::SeqCst) >= 40_000);
        let mut expected = 0;
        for group in 0..20 {
            let rows: Vec<Row> = (0..2_000).map(|variant| row(group, variant)).collect();
            append_for_test(SIGNAL, &rows);
            expected += rows.len();
        }
        assert_eq!(unsafe { drain(SIGNAL, &mut output) }, Ok(()));
        assert_eq!(output.len(), expected);
        assert!(
            output
                .chunks(2_000)
                .enumerate()
                .all(|(group, rows)| rows.iter().all(|row| row.group == group as u32))
        );
    }

    #[test]
    fn unread_reservations_are_never_handed_over() {
        let _lock = test_lock();
        observe_for_test(true);
        unsafe { release_storage() }.unwrap();
        append_for_test(SIGNAL, &[row(1, 1); 8]);
        assert!(REQUIRED.load(Ordering::SeqCst) >= 8);
        let mut output = Vec::new();
        assert_eq!(unsafe { drain(SIGNAL, &mut output) }, Ok(()));
        assert_eq!(output, [row(1, 1); 8]);
        assert!(CAPACITY.load(Ordering::SeqCst) >= 8);
        output.clear();
        record(SIGNAL, 3, |out| {
            out[0] = row(2, 1);
            Ok(Outcome::Rows { len: 1 })
        });
        record(SIGNAL, 2, |out| {
            out[0] = row(3, 1);
            Err("combat log pool variant unreadable")
        });
        append_for_test(SIGNAL, &[row(4, 1)]);
        assert!(unsafe { drain(SIGNAL, &mut output) }.is_err());
        assert_eq!(output, [row(2, 1), row(4, 1)]);
        output.clear();
        record(SIGNAL, 3, |out| {
            out[0] = row(5, 1);
            Ok(Outcome::Rows { len: 1 })
        });
        assert_eq!(unsafe { drain(SIGNAL, &mut output) }, Ok(()));
        assert_eq!(output, [row(5, 1)]);
    }

    #[test]
    fn a_reservation_that_grew_is_reread_whole_and_a_failed_spill_is_named() {
        let _lock = test_lock();
        observe_for_test(true);
        install_for_test(8);
        let grown = [row(1, 1), row(1, 2), row(1, 3)];
        record(SIGNAL, 2, |out| {
            let len = out.len().min(grown.len());
            out[..len].copy_from_slice(&grown[..len]);
            Ok(if len == grown.len() {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated {
                    len,
                    unread_rows: (grown.len() - len) as u32,
                }
            })
        });
        append_for_test(SIGNAL, &[row(2, 1)]);
        let mut output = Vec::new();
        assert_eq!(unsafe { drain(SIGNAL, &mut output) }, Ok(()));
        assert_eq!(output, [row(1, 1), row(1, 2), row(1, 3), row(2, 1)]);
        output.clear();
        install_for_test(1);
        collector_test_support::with_allocations_denied(|| {
            append_for_test(SIGNAL, &[row(3, 1), row(3, 2)]);
        });
        append_for_test(SIGNAL, &[row(4, 1)]);
        assert_eq!(unsafe { drain(SIGNAL, &mut output) }, Err(STORAGE_FAILED));
        assert_eq!(output, [row(4, 1)]);
    }

    #[test]
    fn a_spill_never_waits_for_the_drain_and_a_burst_keeps_every_row_in_order() {
        let _lock = test_lock();
        observe_for_test(true);
        install_for_test(4);
        let burst = |call: u32| -> Vec<Row> { (0..3).map(|variant| row(call, variant)).collect() };
        let consumer = held();
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                for call in 0..200 {
                    let _call = Call::enter();
                    append_for_test(SIGNAL, &burst(call));
                }
                done.send(()).unwrap();
            });
            finished
                .recv_timeout(std::time::Duration::from_secs(30))
                .expect("a combat log prune spill waited for the drain's lock");
            drop(consumer);
        });
        let mut output = Vec::new();
        assert_eq!(unsafe { drain(SIGNAL, &mut output) }, Ok(()));
        assert_eq!(output, (0..200).flat_map(burst).collect::<Vec<_>>());
        assert!(held().is_empty());
    }

    #[test]
    fn rows_another_session_logged_are_discarded() {
        let _lock = test_lock();
        observe_for_test(true);
        append_for_test(SIGNAL - 1, &[row(1, 1), row(1, 2)]);
        append_for_test(SIGNAL, &[row(2, 1)]);
        let mut output = Vec::new();
        assert_eq!(unsafe { drain(SIGNAL, &mut output) }, Ok(()));
        assert_eq!(output, [row(2, 1)]);
    }

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
        assert_eq!(unsafe { drain(SIGNAL, &mut output) }, Ok(()));
        assert_eq!(output.len(), expected as usize);
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
