//! What every stopped window of this process actually cost, recorded in
//! production.
//!
//! # The gap this closes
//!
//! Until this module existed, the only thing production knew about a freeze was
//! that one had been attempted: [`crate::freeze_attempts`] counts them and had
//! ZERO consumers anywhere in `collector/` or `recorder_host/`, so no receipt
//! and no recording carried the number. Nothing at all knew how LONG one was.
//! Every `Instant::now()` in `freeze.rs` lived inside its `#[cfg(test)] mod
//! tests`, so lane JU's 27.9-72.5 ms `MinHook::apply_queued` figures and its
//! 2.3-17.0 ms acquisition figures are measurements of a TEST PROGRAM holding
//! ten threads (`runs/ju_ix_residuals/FINDINGS.txt`), not of an attach into
//! `hoi4.exe`. Asked "how many times does a real attach stop the game, and for
//! how long", a recording could not answer, and lane JU's recommendation to
//! spend a lane on `MinHook::apply_queued` rests on exactly that number.
//!
//! This records, per stopped window: how long the whole window lasted, how much
//! of it was acquiring the freeze, how much of it was the MinHook call the
//! caller timed, how many targets it covered, and which entry point took it.
//!
//! # Why it is here and not in each caller
//!
//! Same reason the release ledger is here. Freezes are taken from seven entry
//! points in `lib.rs` AND directly through the public [`crate::freeze_process`],
//! which lanes use to patch their own bytes with no MinHook apply at all. A
//! census kept by callers would cover whichever callers remembered to write to
//! it. Recording it where the freeze guard is DROPPED covers every one of them
//! with no caller opt-in and no caller able to forget.
//!
//! # Why there is no lock in here
//!
//! `freeze.rs` pins the number of `.lock()` calls its own production text may
//! contain, and the reason it gives is the one that matters:
//! *"the freeze holds every other thread suspended, so a new lock is one a
//! suspended thread may already own with nobody left to release it"*. The
//! recording call sits in `ProcessFreeze::drop` AFTER every suspended thread has
//! been resumed, which would be enough to make a mutex safe -- but "safe because
//! of where it is called from" is a property a future caller can break silently,
//! and this module is reachable from the one code path in the product where that
//! mistake hangs a player's game.
//!
//! So the storage is a fixed array of atomic slots claimed with one
//! `fetch_add`. It takes no lock, allocates nothing, and never blocks, which
//! makes it safe to call from inside a frozen window as well as outside one.
//! The ordering argument is then only about publication, not about deadlock:
//! each slot's `ready` flag is stored with `Release` after its fields and read
//! with `Acquire` before them, so a reader either sees a whole row or no row.
//!
//! # Cost
//!
//! One uncontended `fetch_add` per row, and clock reads that depend on what the
//! window was: FIVE for a window whose caller timed a MinHook call -- four
//! explicit `Instant::now()` plus the one `Instant::elapsed()` performs -- of
//! which three are inside the window; THREE for a successful direct
//! [`crate::freeze_process`], which has no MinHook call to time; and TWO for an
//! attempt that suspended something and then refused. An attempt that refused
//! before suspending anything records no row and reads the clock once. On
//! Windows x86_64
//! `Instant::now()` is `QueryPerformanceCounter`, which allocates nothing, takes
//! no lock and blocks on nothing. A freeze already costs milliseconds; this is
//! nanoseconds, and it is paid once per install unit at capture start rather
//! than on any producer path.
//!
//! # Bounds
//!
//! [`CENSUS_CAPACITY`] rows, then an overflow COUNTER instead of storage. A
//! truncated census is a NAMED truncation -- [`freeze_windows_dropped`] is
//! published beside the rows -- never a silently short list. The total attempt
//! count is [`crate::freeze_attempts`] and is never dropped, so "how many"
//! stays answerable however long the run is.

use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

/// How many individual stopped windows are kept in order before the census
/// starts counting overflow instead of storing it.
///
/// A passive attach's freeze count is UNVERIFIED on this tree -- that is the
/// question this module exists to answer -- so this is deliberately NOT sized
/// from a census. What bounds it from below is structure that can be read:
/// `PRODUCTION_INSTALL_UNITS` is declared with twelve units, the adversarial
/// review of lane JU's leg-5 gap predicted thirteen shared acquisitions from
/// the successful call graph, and teardown releases each installed group again
/// with a disable and a remove. This leaves room for several times that,
/// including retried install units, before anything is dropped.
pub const CENSUS_CAPACITY: usize = 256;

/// Which `inline_detour` entry point took the freeze.
///
/// Recorded because the windows are not interchangeable: a batch apply pays one
/// MinHook call for many targets, a single-target `enable` pays one for one, and
/// a bare [`crate::freeze_process`] pays none at all because the caller patches
/// its own bytes. A census that could not tell them apart would report a mean
/// over three different things.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum FreezeEntryPoint {
    /// `freeze_process()` taken directly by a lane. No MinHook apply inside it,
    /// so its `minhook_us` is legitimately zero.
    Direct = 0,
    Enable = 1,
    ApplyQueued = 2,
    EnableBatch = 3,
    Disable = 4,
    DisableBatch = 5,
    Remove = 6,
    RemoveBatch = 7,
}

impl FreezeEntryPoint {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "freeze_process",
            Self::Enable => "enable",
            Self::ApplyQueued => "apply_queued",
            Self::EnableBatch => "enable_batch",
            Self::Disable => "disable",
            Self::DisableBatch => "disable_batch",
            Self::Remove => "remove",
            Self::RemoveBatch => "remove_batch",
        }
    }

    const fn code(self) -> u32 {
        self as u32
    }

    /// The inverse of [`Self::code`]. Total rather than fallible: the only
    /// writer is [`Self::code`] in this same module, so an unknown code is a
    /// corrupted slot rather than an input, and reporting the window as
    /// `Direct` would be a lie about where it came from. It is reported as
    /// `Unknown` through the row's own accessor instead -- see
    /// [`FreezeWindow::entry_point_name`].
    fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            0 => Self::Direct,
            1 => Self::Enable,
            2 => Self::ApplyQueued,
            3 => Self::EnableBatch,
            4 => Self::Disable,
            5 => Self::DisableBatch,
            6 => Self::Remove,
            7 => Self::RemoveBatch,
            _ => return None,
        })
    }
}

/// One stopped window, as production observed it.
///
/// Every duration is microseconds, saturating. A `u32` of microseconds is just
/// over an hour; a freeze that lasted that long has hung the game, and the
/// saturation is not the finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreezeWindow {
    /// This process's freeze attempt ordinal, 1-based, in attempt order.
    ///
    /// Taken from the same counter [`crate::freeze_attempts`] publishes, so a
    /// row can be matched against it. A gap between consecutive ordinals is one
    /// of exactly two things and never anything else: a DROPPED row, counted by
    /// [`freeze_windows_dropped`], or an attempt that refused before it
    /// suspended anything, which stopped nothing and deliberately records no
    /// row. That is what lets the READY receipt's attempt count be reconciled
    /// against the rows rather than merely sitting beside them.
    pub ordinal: u64,
    /// This attempt never got a freeze, so nothing was patched -- but it HAD
    /// suspended at least one thread before it gave up, and resumed it on the
    /// way out, so the game really was stopped for part of `window_us` and the
    /// row exists to say so.
    ///
    /// An attempt that refused BEFORE its first suspension records no row at
    /// all: it stopped nothing, and a row for it would report a stopped window
    /// this process never had. Such an attempt is still counted by
    /// [`crate::freeze_attempts`], which counts attempts rather than windows --
    /// so the census can be SHORTER than that counter, and the difference is
    /// exactly the refusals that never got as far as stopping anything.
    ///
    /// It is recorded because the alternative was a false statement: the first
    /// version of this module claimed a refused freeze "suspends nothing", and
    /// `freeze_other_threads_inner` suspends thread by thread and can refuse
    /// several passes later. A census that skipped refusals would report fewer
    /// stopped intervals than the process performed, and the freeze-attempt
    /// count published in the READY receipt would not be the number the rows
    /// add up to.
    ///
    /// A refused row carries no `minhook_us` and no `acquire_us`: there was no
    /// acquisition to time and MinHook was never called.
    pub refused: bool,
    pub entry_point: Option<FreezeEntryPoint>,
    /// The first target's name, for the entry points that have one. Empty for
    /// [`FreezeEntryPoint::Direct`], which has no target list.
    ///
    /// It is what attributes a window to a lane: the batch entry points are
    /// called with the `&'static str` names the pinned roster already uses.
    pub label: &'static str,
    /// How many targets this window covered. Zero for `Direct`.
    pub target_count: u32,
    /// Freeze CALL entry to the end of the resume loop.
    ///
    /// AN UPPER BOUND on stopped time, not the stopped time itself, and the
    /// bound is loose at both ends. The clock starts before the freeze resolves
    /// ntdll, walks once to size its buffers and reserves them -- all with
    /// nothing suspended -- and it stops after the resume loop has also
    /// `CloseHandle`d every thread. Measuring the exact interval would need a
    /// timestamp taken inside the pass loop at the first successful suspension;
    /// that is a further clock read INSIDE the frozen window, and it is
    /// deliberately not taken.
    ///
    /// It is measured over this span because it is the span lane JU's paired
    /// figures were measured over, so the census and the evidence it is checked
    /// against are the same quantity.
    pub window_us: u32,
    /// The part of `window_us` spent acquiring the freeze -- the thread walk,
    /// the granted-access reads, the suspensions and the roster confirmation.
    pub acquire_us: u32,
    /// The part of `window_us` the caller timed around its MinHook call, which
    /// for `enable_batch` and `apply_queued` is `MH_ApplyQueued`. Zero means the
    /// caller marked nothing; for `Direct` that is the normal case, which is why
    /// this is not an `Option` -- zero and "no MinHook call happened" are the
    /// same statement here.
    pub minhook_us: u32,
    /// Fixed-point enumeration passes the acquisition spent. Two is a clean
    /// freeze.
    pub passes: u32,
}

impl FreezeWindow {
    /// The entry point's name, or `"unknown"` for a slot whose discriminant this
    /// module did not write.
    pub fn entry_point_name(&self) -> &'static str {
        self.entry_point
            .map(FreezeEntryPoint::as_str)
            .unwrap_or("unknown")
    }
}

/// One census slot. Claimed by index, written once, and never reused.
struct Slot {
    /// Zero until every field below has been stored. Written with `Release` and
    /// read with `Acquire`, which is the whole of the publication protocol.
    ready: AtomicU32,
    refused: AtomicU32,
    ordinal: AtomicU64,
    entry_point: AtomicU32,
    label_ptr: AtomicPtr<u8>,
    label_len: AtomicUsize,
    target_count: AtomicU32,
    window_us: AtomicU32,
    acquire_us: AtomicU32,
    minhook_us: AtomicU32,
    passes: AtomicU32,
}

impl Slot {
    const NEW: Self = Self {
        ready: AtomicU32::new(0),
        refused: AtomicU32::new(0),
        ordinal: AtomicU64::new(0),
        entry_point: AtomicU32::new(0),
        label_ptr: AtomicPtr::new(core::ptr::null_mut()),
        label_len: AtomicUsize::new(0),
        target_count: AtomicU32::new(0),
        window_us: AtomicU32::new(0),
        acquire_us: AtomicU32::new(0),
        minhook_us: AtomicU32::new(0),
        passes: AtomicU32::new(0),
    };
}

static SLOTS: [Slot; CENSUS_CAPACITY] = [Slot::NEW; CENSUS_CAPACITY];
/// The next slot to claim. Also the count of windows this process has recorded
/// or tried to, so it is what says the census is truncated.
static CURSOR: AtomicUsize = AtomicUsize::new(0);

/// Record one stopped window.
///
/// Called from `ProcessFreeze::drop` after every suspended thread has been
/// resumed. Takes no lock and allocates nothing, so being called from somewhere
/// else -- including inside a frozen window -- is safe rather than a hang.
pub(crate) fn record(window: FreezeWindow) {
    let index = CURSOR.fetch_add(1, Ordering::AcqRel);
    let Some(slot) = SLOTS.get(index) else {
        // Past capacity. The cursor keeps counting, so the number of dropped
        // rows is exactly `CURSOR - CENSUS_CAPACITY` and is reported rather
        // than inferred.
        return;
    };
    slot.ordinal.store(window.ordinal, Ordering::Relaxed);
    slot.refused
        .store(u32::from(window.refused), Ordering::Relaxed);
    slot.entry_point.store(
        window
            .entry_point
            .map(FreezeEntryPoint::code)
            .unwrap_or(u32::MAX),
        Ordering::Relaxed,
    );
    slot.label_ptr
        .store(window.label.as_ptr().cast_mut(), Ordering::Relaxed);
    slot.label_len.store(window.label.len(), Ordering::Relaxed);
    slot.target_count.store(window.target_count, Ordering::Relaxed);
    slot.window_us.store(window.window_us, Ordering::Relaxed);
    slot.acquire_us.store(window.acquire_us, Ordering::Relaxed);
    slot.minhook_us.store(window.minhook_us, Ordering::Relaxed);
    slot.passes.store(window.passes, Ordering::Relaxed);
    slot.ready.store(1, Ordering::Release);
}

/// Every recorded window, in acquisition order.
///
/// A slot claimed but not yet published is skipped rather than reported half
/// written; the claim is one `fetch_add` and the publish is the next few
/// instructions on the same thread, so the only way to observe one is to read
/// the census from another thread while a freeze is being dropped.
pub fn freeze_windows() -> Vec<FreezeWindow> {
    let claimed = CURSOR.load(Ordering::Acquire).min(CENSUS_CAPACITY);
    let mut rows = Vec::with_capacity(claimed);
    for slot in &SLOTS[..claimed] {
        if slot.ready.load(Ordering::Acquire) == 0 {
            continue;
        }
        let label_ptr = slot.label_ptr.load(Ordering::Relaxed);
        let label_len = slot.label_len.load(Ordering::Relaxed);
        // SOUND because the only writer stores the pointer and length of a
        // `&'static str`: the bytes outlive the process, are never written
        // again, and were UTF-8 when they were a `str`. A null pointer is the
        // never-written case and reads as the empty label.
        let label: &'static str = if label_ptr.is_null() || label_len == 0 {
            ""
        } else {
            unsafe {
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(label_ptr, label_len))
            }
        };
        rows.push(FreezeWindow {
            ordinal: slot.ordinal.load(Ordering::Relaxed),
            refused: slot.refused.load(Ordering::Relaxed) != 0,
            entry_point: FreezeEntryPoint::from_code(slot.entry_point.load(Ordering::Relaxed)),
            label,
            target_count: slot.target_count.load(Ordering::Relaxed),
            window_us: slot.window_us.load(Ordering::Relaxed),
            acquire_us: slot.acquire_us.load(Ordering::Relaxed),
            minhook_us: slot.minhook_us.load(Ordering::Relaxed),
            passes: slot.passes.load(Ordering::Relaxed),
        });
    }
    rows
}

/// How many windows the census could not store. Published beside the rows, so a
/// truncated census reads as truncated instead of as a short run.
pub fn freeze_windows_dropped() -> u64 {
    windows_dropped_from_cursor(CURSOR.load(Ordering::Acquire))
}

/// The overflow rule, split out so it can be exercised at cursor values this
/// process will never reach.
///
/// Review round 2, finding 8: the test that claimed to pin truncation only
/// compared two live readings, which agree whenever the shared cursor is below
/// capacity -- and it is, in every test run. Replacing the whole function body
/// with `0` passed. A pure function CAN be driven past capacity, so the rule is
/// tested rather than the run.
const fn windows_dropped_from_cursor(claimed: usize) -> u64 {
    claimed.saturating_sub(CENSUS_CAPACITY) as u64
}

/// How many windows this process has completed, dropped rows included.
pub fn freeze_windows_recorded() -> u64 {
    CURSOR.load(Ordering::Acquire) as u64
}

/// The in-flight timing of one window, owned by the `ProcessFreeze` it belongs
/// to.
///
/// Split out so `freeze.rs` carries one field rather than six, and so the
/// arithmetic that turns three instants into two durations lives beside the type
/// that documents what they mean.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FreezeTiming {
    ordinal: u64,
    started: Instant,
    acquired: Instant,
    entry_point: FreezeEntryPoint,
    label: &'static str,
    target_count: u32,
    minhook_us: u32,
}

impl FreezeTiming {
    /// `started` is taken at entry to the freeze, before anything is suspended;
    /// `acquired` at the moment the freeze is established.
    pub(crate) fn new(ordinal: u64, started: Instant, acquired: Instant) -> Self {
        Self {
            ordinal,
            started,
            acquired,
            entry_point: FreezeEntryPoint::Direct,
            label: "",
            target_count: 0,
            minhook_us: 0,
        }
    }

    /// Name the entry point and its target list. Called by the `lib.rs` wrapper
    /// that took the freeze; a freeze nobody names stays
    /// [`FreezeEntryPoint::Direct`], which is exactly what a lane's own
    /// `freeze_process()` is.
    pub(crate) fn note_entry(
        &mut self,
        entry_point: FreezeEntryPoint,
        label: &'static str,
        target_count: u32,
    ) {
        self.entry_point = entry_point;
        self.label = label;
        self.target_count = target_count;
    }

    /// Record how long the caller's MinHook call took, timed by the caller
    /// around that call and nothing else.
    pub(crate) fn note_minhook(&mut self, span: std::time::Duration) {
        self.minhook_us = saturating_micros(span);
    }

    /// Close the window at `resumed` -- taken at the end of the resume loop,
    /// after the last `ResumeThread` AND its `CloseHandle` -- and produce the
    /// row.
    pub(crate) fn close(self, resumed: Instant, passes: u32) -> FreezeWindow {
        let window_us = saturating_micros(resumed.saturating_duration_since(self.started));
        let acquire_us = saturating_micros(self.acquired.saturating_duration_since(self.started));
        FreezeWindow {
            ordinal: self.ordinal,
            refused: false,
            entry_point: Some(self.entry_point),
            label: self.label,
            target_count: self.target_count,
            window_us,
            // CLAMPED TO THE WHOLE, in production and not only in a test.
            //
            // The three spans saturate INDEPENDENTLY, so a window long enough to
            // saturate `window_us` can carry an `acquire_us` that also saturates
            // plus a nonzero `minhook_us`, and the row would then say its parts
            // are bigger than itself. Every share computed from such a row is
            // above 100%, and the share is the whole point of the census. The
            // clamp makes the arithmetic true at the cost of a figure that was
            // already meaningless: reaching it takes a 71-minute freeze.
            acquire_us: acquire_us.min(window_us),
            minhook_us: self.minhook_us.min(window_us.saturating_sub(acquire_us.min(window_us))),
            passes,
        }
    }
}

/// The row a REFUSED acquisition leaves behind.
///
/// `started` is the instant the freeze call was entered and `ended` the instant
/// it gave up, both taken by `freeze_other_threads`. Between them the freeze may
/// have suspended threads and resumed them again -- the refusal path is
/// fail-closed, not no-op -- so the game really was stopped for some part of
/// this span and the row exists to stop that stopped time going unrecorded.
pub(crate) fn refused_window(ordinal: u64, started: Instant, ended: Instant) -> FreezeWindow {
    FreezeWindow {
        ordinal,
        refused: true,
        entry_point: None,
        label: "",
        target_count: 0,
        window_us: saturating_micros(ended.saturating_duration_since(started)),
        // There was no acquisition to time and MinHook was never reached. Zero
        // here is a statement, not a missing measurement, and `refused` is what
        // says which.
        acquire_us: 0,
        minhook_us: 0,
        passes: 0,
    }
}

fn saturating_micros(span: std::time::Duration) -> u32 {
    u32::try_from(span.as_micros()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// kills: replacing the saturating conversion with `as u32`, which turns a
    /// window longer than 71.6 minutes -- a hung game, the one window anybody
    /// would ever look for -- into a small number that reads as healthy.
    #[test]
    fn an_absurd_span_saturates_rather_than_wrapping() {
        assert_eq!(
            saturating_micros(std::time::Duration::from_secs(u64::from(u32::MAX))),
            u32::MAX
        );
        assert_eq!(saturating_micros(std::time::Duration::from_micros(7)), 7);
    }

    /// kills: writing the entry-point discriminant and reading it back through a
    /// different mapping, which would silently re-attribute every window in the
    /// census to another entry point.
    #[test]
    fn every_entry_point_survives_the_round_trip_through_a_slot() {
        for entry in [
            FreezeEntryPoint::Direct,
            FreezeEntryPoint::Enable,
            FreezeEntryPoint::ApplyQueued,
            FreezeEntryPoint::EnableBatch,
            FreezeEntryPoint::Disable,
            FreezeEntryPoint::DisableBatch,
            FreezeEntryPoint::Remove,
            FreezeEntryPoint::RemoveBatch,
        ] {
            assert_eq!(FreezeEntryPoint::from_code(entry.code()), Some(entry));
        }
        // And the sentinel the writer uses for a row whose entry point was not
        // one of those must NOT decode as one of them.
        assert_eq!(FreezeEntryPoint::from_code(u32::MAX), None);
    }

    /// kills: reporting a truncated census as complete -- replacing the
    /// overflow derivation with `0`, with `claimed`, or with an off-by-one.
    ///
    /// DRIVEN PAST CAPACITY, which is the whole point and which the first
    /// version of this test could not do. It compared two live readings of the
    /// shared census, and those agree at every cursor value this suite reaches,
    /// so `-> 0` passed it (review round 2, finding 8). The rule is a pure
    /// function of the cursor, so it is asserted at cursor values no run will
    /// ever produce.
    #[test]
    fn the_dropped_count_is_the_overflow_of_the_cursor() {
        assert_eq!(windows_dropped_from_cursor(0), 0);
        assert_eq!(windows_dropped_from_cursor(CENSUS_CAPACITY - 1), 0);
        assert_eq!(windows_dropped_from_cursor(CENSUS_CAPACITY), 0);
        assert_eq!(windows_dropped_from_cursor(CENSUS_CAPACITY + 1), 1);
        assert_eq!(
            windows_dropped_from_cursor(CENSUS_CAPACITY + 9_999),
            9_999,
            "a census that overflowed by ten thousand must say ten thousand"
        );
        // And the live reading is that same rule over the live cursor, so the
        // two cannot drift apart.
        assert_eq!(
            freeze_windows_dropped(),
            windows_dropped_from_cursor(freeze_windows_recorded() as usize)
        );
        assert!(freeze_windows().len() <= CENSUS_CAPACITY);
    }

    /// kills: dropping the clamp that keeps a row's parts inside its whole. The
    /// three spans saturate independently, so without it a long-enough window
    /// reports shares above 100%.
    #[test]
    fn a_saturated_row_never_reports_parts_larger_than_its_whole() {
        let start = Instant::now();
        let mut timing = FreezeTiming::new(1, start, start + std::time::Duration::from_secs(9_000));
        timing.note_minhook(std::time::Duration::from_secs(9_000));
        let row = timing.close(start + std::time::Duration::from_secs(1), 2);
        assert!(
            row.acquire_us <= row.window_us,
            "acquire {} exceeds window {}",
            row.acquire_us,
            row.window_us
        );
        assert!(
            row.acquire_us.saturating_add(row.minhook_us) <= row.window_us,
            "acquire {} + minhook {} exceeds window {}",
            row.acquire_us,
            row.minhook_us,
            row.window_us
        );
    }

    /// kills: dropping the refusal row. A refused acquisition suspends threads
    /// and resumes them, so it is a stopped window; if it recorded nothing, the
    /// census would be shorter than the attempt count the receipt publishes.
    #[test]
    fn a_refused_attempt_still_produces_a_row_that_says_it_was_refused() {
        let start = Instant::now();
        let row = refused_window(7, start, start + std::time::Duration::from_millis(3));
        assert!(row.refused);
        assert_eq!(row.ordinal, 7);
        assert_eq!(row.window_us, 3_000);
        assert_eq!(row.acquire_us, 0, "a refusal never acquired anything");
        assert_eq!(row.minhook_us, 0, "a refusal never reached MinHook");
        assert_eq!(row.entry_point, None);
    }
}
