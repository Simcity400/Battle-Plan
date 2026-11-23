use crate::sampling::{self, Placement};
use collector_timing::capture_gate;
use std::{
    io,
    os::windows::io::{AsRawHandle, BorrowedHandle},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows_sys::Win32::System::Threading::{
    GetCurrentThread, GetThreadId, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
    THREAD_PRIORITY_BELOW_NORMAL, WaitOnAddress, WakeByAddressAll,
};

pub(crate) const BESIDE_READERS: usize = 3;

const IDLE: u32 = 0;
const REQUESTED: u32 = 1;
const STOP: u32 = 2;

static PENDING: AtomicBool = AtomicBool::new(true);
static PENDING_WORLD: AtomicU64 = AtomicU64::new(0);
static CAPTURED_WORLD: AtomicU64 = AtomicU64::new(0);
static SETTLE: AtomicBool = AtomicBool::new(false);
static FULL: AtomicBool = AtomicBool::new(false);
static DRAINING: AtomicBool = AtomicBool::new(false);
static BESIDE: AtomicBool = AtomicBool::new(false);
static WORK: AtomicU32 = AtomicU32::new(IDLE);
static WORK_FULL: AtomicBool = AtomicBool::new(false);
static THREAD: AtomicIsize = AtomicIsize::new(0);
static GAME: AtomicU32 = AtomicU32::new(0);
static HURRIED: AtomicBool = AtomicBool::new(false);
static CONFINED: AtomicBool = AtomicBool::new(false);
static FAILED: AtomicBool = AtomicBool::new(false);
static BESIDE_CAPTURES: AtomicU64 = AtomicU64::new(0);
static CONFINED_CAPTURES: AtomicU64 = AtomicU64::new(0);
static HELD_CAPTURES: AtomicU64 = AtomicU64::new(0);
static CAPTURE_PANICS: AtomicU64 = AtomicU64::new(0);
static LOST_TO_LOAD: AtomicU64 = AtomicU64::new(0);
static LOST_TO_OTHER_BATCH: AtomicU64 = AtomicU64::new(0);
static HOURS: AtomicU64 = AtomicU64::new(0);
const ENDED: u64 = (1 << RUNNING_SHIFT) - 1;
const RUNNING_SHIFT: u32 = 40;
const RUNNING: u64 = 1 << RUNNING_SHIFT;
const CLOSED: u64 = 1 << 63;
static ACCOUNTED: AtomicU64 = AtomicU64::new(0);
static ENDED_WORLD: AtomicU64 = AtomicU64::new(0);
static CLAIMED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Update {
    #[default]
    Uncounted,
    Counted,
    AfterAccount,
}

fn count(step: u64) -> bool {
    HOURS
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |hours| {
            (hours & CLOSED == 0).then_some(hours.wrapping_add(step))
        })
        .is_ok()
}

fn owed() -> bool {
    HOURS.load(Ordering::SeqCst) & ENDED != ACCOUNTED.load(Ordering::SeqCst)
}
static BESIDE_NS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn update_entered() -> Update {
    if !count(RUNNING) {
        return Update::AfterAccount;
    }
    let pending = PENDING.swap(false, Ordering::AcqRel);
    let full = pending && FULL.swap(false, Ordering::AcqRel);
    let world = world();
    if pending {
        forget_replaced(world);
    }
    if reads_before_update(pending, world, CAPTURED_WORLD.load(Ordering::SeqCst)) {
        match held(full) {
            Held::Read => {}
            Held::Busy if pending => {
                if owed() {
                    LOST_TO_OTHER_BATCH.fetch_add(1, Ordering::Relaxed);
                }
            }
            _ => {
                PENDING_WORLD.store(world, Ordering::Release);
                unread(full);
            }
        }
    }
    Update::Counted
}

fn reads_before_update(pending: bool, world: u64, captured: u64) -> bool {
    pending || (world != u64::MAX && world != captured)
}

pub(crate) fn hour_ended(update: Update) {
    let step = match update {
        Update::AfterAccount => return,
        Update::Counted => 1u64.wrapping_sub(RUNNING),
        Update::Uncounted => 1,
    };
    let world = world();
    ENDED_WORLD.store(world, Ordering::SeqCst);
    if !count(step) {
        return;
    }
    PENDING_WORLD.store(world, Ordering::Release);
    let full = SETTLE.swap(false, Ordering::AcqRel);
    if full && !DRAINING.load(Ordering::Acquire) {
        unread(true);
    } else if BESIDE.load(Ordering::Acquire)
        || crate::command_batch::observed()
        || held(full) != Held::Read
    {
        unread(full);
    }
}

pub(crate) fn batch_left() {
    if FULL.load(Ordering::Acquire) && !DRAINING.load(Ordering::Acquire) {
        return;
    }
    if !BESIDE.load(Ordering::Acquire) && !crate::command_batch::observed() {
        return;
    }
    if !PENDING.load(Ordering::Acquire) || !PENDING.swap(false, Ordering::AcqRel) {
        return;
    }
    let full = FULL.swap(false, Ordering::AcqRel);
    forget_replaced(world());
    if !BESIDE.load(Ordering::Acquire) {
        if held(full) != Held::Read {
            unread(full);
        }
        return;
    }
    // SAFETY: Capture-point admission opens the gate and pairs every true return with end() on
    // early return or via `Gate` in `serve()`.
    if !unsafe { capture_gate::begin() } {
        unread(full);
        return;
    }
    if crate::command_batch::others_batching() {
        // SAFETY: Releases the gate opened above before abandoning beside dispatch.
        unsafe { capture_gate::end() };
        unread(full);
        return;
    }
    if !BESIDE.load(Ordering::SeqCst) {
        // SAFETY: Releases the gate opened above before falling back to held capture.
        unsafe { capture_gate::end() };
        if held(full) != Held::Read {
            unread(full);
        }
        return;
    }
    HURRIED.store(false, Ordering::SeqCst);
    GAME.store(
        sampling::main_processor().map_or(0, |processor| processor.saturating_add(1)),
        Ordering::Relaxed,
    );
    WORK_FULL.store(full, Ordering::Release);
    WORK.store(REQUESTED, Ordering::Release);
    unsafe { WakeByAddressAll(WORK.as_ptr().cast()) };
}

/// # Safety
/// Only the active session's history recovery may request settlement, before a subsequent hour can
/// consume the request and before detaching.
pub unsafe fn settle_next() {
    SETTLE.store(true, Ordering::Release);
}

/// # Safety
/// Only the runtime's detach sequence may drain the pending capture after closing hourly admission,
/// before tearing down its capture thread and storage.
pub unsafe fn drain(deadline: Instant) -> bool {
    DRAINING.store(true, Ordering::SeqCst);
    loop {
        let unread = PENDING.load(Ordering::SeqCst) && PENDING_WORLD.load(Ordering::SeqCst) != 0;
        if !unread && !capture_gate::running() && sampling::ACTIVE.load(Ordering::SeqCst) == 0 {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// # Safety
/// Only the detaching runtime may consume the pending loss count, after draining the capture point
/// and before flushing the spool.
pub unsafe fn take_unread() -> u32 {
    let hours = HOURS.fetch_or(CLOSED, Ordering::SeqCst);
    if hours & CLOSED != 0 {
        return 0;
    }
    let ended = hours & ENDED;
    let unread = ended.saturating_sub(ACCOUNTED.swap(ended, Ordering::SeqCst));
    u32::try_from(unread.saturating_add(hours >> RUNNING_SHIFT)).unwrap_or(u32::MAX)
}

pub(crate) fn account() -> u32 {
    let ended = HOURS.load(Ordering::SeqCst) & ENDED;
    let owed = ended.saturating_sub(ACCOUNTED.swap(ended, Ordering::SeqCst));
    let reads = owed != 0 && !replaced(ENDED_WORLD.load(Ordering::SeqCst), world());
    CLAIMED.store(reads, Ordering::SeqCst);
    u32::try_from(owed - u64::from(reads)).unwrap_or(u32::MAX)
}

fn replaced(ended: u64, world: u64) -> bool {
    ended != 0 && world != u64::MAX && ended != world
}

fn world() -> u64 {
    #[cfg(test)]
    if let Some(world) = tests::WORLD.get() {
        return world;
    }
    collector_lifetime::world_lifetime::generation()
}

fn forget_replaced(world: u64) {
    if replaced(PENDING_WORLD.load(Ordering::Acquire), world) {
        if owed() {
            LOST_TO_LOAD.fetch_add(1, Ordering::Relaxed);
        }
        PENDING_WORLD.store(world, Ordering::Release);
    }
}

fn unread(full: bool) {
    if full {
        FULL.store(true, Ordering::Release);
    }
    PENDING.store(true, Ordering::Release);
}

/// Reads the hour now. False when the capture was not admitted: then nothing was read.
///
/// # Safety
/// As [`sampling::capture`].
unsafe fn read(placement: Placement, full: bool) -> bool {
    #[cfg(test)]
    if let Some(admitted) = tests::read(placement, full) {
        return admitted;
    }
    unsafe { sampling::capture(placement, full) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    Read,
    Refused,
    Busy,
}

fn held(full: bool) -> Held {
    // SAFETY: Paired immediately with the `Gate` RAII guard below.
    if !unsafe { capture_gate::begin() } {
        return Held::Refused;
    }
    let _gate = Gate;
    if crate::command_batch::others_batching() {
        return Held::Busy;
    }
    let _images = collector_characters::portrait_changes::PreImagesCapture::begin();
    let world = world();
    // SAFETY: a game thread that stays in its callback (the hourly update's entry or return, or the
    // outermost command batch's return) for the whole read, with no other capture and no other
    // observed command batch running. On an unproven publication a batch that entered before the
    // hook was published is not counted; that mode accepts the gap.
    if !unsafe { read(Placement::Held, full) } {
        return Held::Refused;
    }
    HELD_CAPTURES.fetch_add(1, Ordering::Relaxed);
    CAPTURED_WORLD.store(world, Ordering::SeqCst);
    Held::Read
}

fn hurry() {
    if HURRIED.swap(true, Ordering::SeqCst) {
        return;
    }
    // SAFETY: The capture point is acting for the current live hour.
    unsafe { sampling::hurry() };
    let thread = THREAD.load(Ordering::SeqCst);
    if thread != 0 {
        // SAFETY: `Thread::stop` joins, closing the handle, only once no waiter is counted.
        let thread = unsafe { BorrowedHandle::borrow_raw(thread as _) };
        unsafe { SetThreadPriority(thread.as_raw_handle(), THREAD_PRIORITY_ABOVE_NORMAL) };
        if CONFINED.swap(false, Ordering::SeqCst) {
            // SAFETY: `thread` is the live collector-owned capture thread held until waiters finish.
            unsafe { collector_workers::cpu_sets::select_away(thread, None) };
        }
    }
}

pub(crate) fn hurried() -> bool {
    HURRIED.load(Ordering::SeqCst)
}

/// # Safety
/// Only the attaching runtime may select beside capture, before installing the hourly hook and
/// after starting its worker when needed.
pub unsafe fn set_beside(beside: bool) {
    BESIDE.store(beside && !FAILED.load(Ordering::SeqCst), Ordering::SeqCst);
}

pub struct Thread {
    handle: Option<JoinHandle<()>>,
    id: u32,
}

impl Thread {
    /// # Safety
    /// Only the attaching runtime may start the process-wide capture thread, once during attach.
    /// Keep the returned handle through detach and stop it after closing hourly admission and
    /// draining the capture point.
    pub unsafe fn start(stack_bytes: usize) -> io::Result<Self> {
        // SAFETY: Runtime attach starts the capture thread once and registers its wait hurry callback.
        unsafe { capture_gate::on_wait(hurry) };
        WORK.store(IDLE, Ordering::Release);
        let handle = std::thread::Builder::new()
            .name("hoi4-capture".into())
            .stack_size(stack_bytes)
            .spawn(run)?;
        let raw = handle.as_raw_handle();
        THREAD.store(raw as isize, Ordering::Release);
        let id = unsafe { GetThreadId(raw as _) };
        // SAFETY: This updates the designated role of this runtime's owned worker.
        unsafe {
            collector_workers::thread_cpu::register_id(
                collector_workers::thread_cpu::Role::Capture,
                id,
            )
        };
        Ok(Self {
            handle: Some(handle),
            id,
        })
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    /// # Safety
    /// Only the runtime may stop its capture thread. First close hourly admission and settle the
    /// pending capture and keep shared state alive until the thread exits.
    pub unsafe fn stop(&mut self) {
        BESIDE.store(false, Ordering::SeqCst);
        while capture_gate::running() {
            std::thread::yield_now();
        }
        THREAD.store(0, Ordering::SeqCst);
        while capture_gate::waiting() != 0 {
            std::thread::yield_now();
        }
        WORK.store(STOP, Ordering::Release);
        unsafe { WakeByAddressAll(WORK.as_ptr().cast()) };
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        while capture_gate::waiting() != 0 {
            std::thread::yield_now();
        }
    }
}

impl Drop for Thread {
    fn drop(&mut self) {
        // SAFETY: This is the owning runtime's shutdown, while the worker's shared state is still
        // live.
        unsafe { self.stop() };
    }
}

struct Gate;
impl Drop for Gate {
    fn drop(&mut self) {
        // SAFETY: `Gate` is constructed only after a matching `capture_gate::begin()` returned true.
        unsafe { capture_gate::end() };
    }
}

fn beside(away: &mut collector_workers::cpu_sets::AwayFromGame, game: Option<u32>) -> bool {
    let own = unsafe { GetCurrentThread() };
    unsafe { SetThreadPriority(own, THREAD_PRIORITY_BELOW_NORMAL) };
    // SAFETY: Called on the collector-owned beside capture thread applying session affinity policy.
    unsafe { away.follow(game) };
    CONFINED.store(away.confined(), Ordering::SeqCst);
    if HURRIED.load(Ordering::SeqCst) {
        unsafe { SetThreadPriority(own, THREAD_PRIORITY_ABOVE_NORMAL) };
        CONFINED.store(false, Ordering::SeqCst);
        // SAFETY: Called on the collector-owned beside capture thread clearing affinity on hurry.
        unsafe { away.follow(None) };
    }
    away.confined()
}

fn run() {
    // SAFETY: This dedicated thread is the collector-owned capture thread that reads under the gate.
    unsafe { capture_gate::reader() };
    let mut away = collector_workers::cpu_sets::AwayFromGame::default();
    let idle = IDLE;
    loop {
        match WORK.load(Ordering::Acquire) {
            STOP => return,
            REQUESTED => {
                WORK.store(IDLE, Ordering::Release);
                serve(|full| capture_beside(&mut away, full));
            }
            _ => {
                unsafe {
                    WaitOnAddress(
                        WORK.as_ptr().cast(),
                        (&idle as *const u32).cast(),
                        size_of::<u32>(),
                        u32::MAX,
                    )
                };
            }
        }
    }
}

fn serve(capture: impl FnOnce(bool)) {
    let full = WORK_FULL.load(Ordering::Acquire);
    let _gate = Gate;
    if FAILED.load(Ordering::SeqCst) {
        unread(full);
        return;
    }
    CLAIMED.store(false, Ordering::SeqCst);
    if catch_unwind(AssertUnwindSafe(|| capture(full))).is_err() {
        CAPTURE_PANICS.fetch_add(1, Ordering::Relaxed);
        FAILED.store(true, Ordering::SeqCst);
        BESIDE.store(false, Ordering::SeqCst);
        if CLAIMED.swap(false, Ordering::SeqCst) {
            ACCOUNTED.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn capture_beside(away: &mut collector_workers::cpu_sets::AwayFromGame, full: bool) {
    let game = GAME.load(Ordering::Relaxed).checked_sub(1);
    let started = Instant::now();
    if beside(away, game) {
        CONFINED_CAPTURES.fetch_add(1, Ordering::Relaxed);
    }
    let _images = collector_characters::portrait_changes::PreImagesCapture::begin();
    let world = world();
    // SAFETY: the gate holds every command-batch writer until this capture has ended; portrait
    // writers proceed, and the portrait reader validates the live bytes it reads against the
    // pre-images `_images` keeps for this read.
    let admitted = unsafe { read(Placement::Beside(game), full) };
    CONFINED.store(false, Ordering::SeqCst);
    // SAFETY: Called on the collector-owned beside capture thread clearing affinity after capture.
    unsafe { away.follow(None) };
    if !admitted {
        unread(full);
        return;
    }
    CAPTURED_WORLD.store(world, Ordering::SeqCst);
    BESIDE_CAPTURES.fetch_add(1, Ordering::Relaxed);
    BESIDE_NS.fetch_add(
        u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
        Ordering::Relaxed,
    );
}

pub fn status() -> impl serde::Serialize {
    let withheld = collector_characters::portrait_changes::withheld_captures();
    collector_status::status::status!({
        "placement": if BESIDE.load(Ordering::Acquire) {
            "beside the game"
        } else if !crate::command_batch::observed() {
            "holding the game without a proven command-batch hook: a frame is read after its update and before commands the same batch runs behind it"
        } else if FAILED.load(Ordering::Acquire) {
            "holding the game: the capture thread failed"
        } else {
            "holding the game"
        },
        "beside": BESIDE_CAPTURES.load(Ordering::Relaxed),
        "beside_ns": BESIDE_NS.load(Ordering::Relaxed),
        "beside_off_game_core": CONFINED_CAPTURES.load(Ordering::Relaxed),
        "off_game_core_failures": collector_workers::cpu_sets::away_failures(),
        "held": HELD_CAPTURES.load(Ordering::Relaxed),
        "batch_publication": crate::command_batch::publication_status(),
        "batch_publications_held": crate::command_batch::publications_held(),
        "portraits_withheld_writer": withheld[0],
        "portraits_withheld_reader": withheld[1],
        "capture_thread_panics": CAPTURE_PANICS.load(Ordering::Relaxed),
        "hours_lost_to_load": LOST_TO_LOAD.load(Ordering::Relaxed),
        "hours_lost_to_other_batch": LOST_TO_OTHER_BATCH.load(Ordering::Relaxed),
        "writer_wait_ns": capture_gate::wait_ns(),
        "scope": "cumulative since attach: captures run beside the game on the capture thread (beside, and their wall in beside_ns) once an outermost command batch returns, and on a game thread while it is held (held: at that same batch return without a capture thread or once the capture thread has failed (capture_thread_panics); straight after the update without a command-batch hook proven to observe every batch (batch_publication says why), when a frame misses commands the same batch runs behind the update; and as the next update is entered for an hour that waited on a refresh request, the hour a world was loaded at, or an hour no earlier point could read); every capture first closes the capture gate and reads only while no other thread is inside an observed batch; batch_publications_held counts publications of the command-batch hook held because a thread may have been inside the batch, which is published unproven once they reach their limit; every capture reads character portraits as they were when it began, and portraits_withheld_writer counts captures whose portrait reads were all withheld by name because a portrait writer was still running when the capture's bounded wait for writers ran out, or ran on the capturing thread itself, and portraits_withheld_reader those withheld because a reader still held a pre-image from an earlier capture; beside_off_game_core counts beside captures whose capture thread started off the game thread's physical core, its readers likewise, and off_game_core_failures the refused CPU-set selections or clearings for it; hours_lost_to_load counts hours that ended but whose world was replaced before any capture could read them, and hours_lost_to_other_batch hours another thread's batch kept from being read until the next update wrote over them, each recorded as a lost hour; writer_wait_ns is the total time game threads waited for a capture behind the closed gate, at a hooked writer's or the command batch's entry, or at a portrait writer the pre-image store could not hold: hook_timing.families.*.capture_waits counts those waits by family, and the entry waits lie outside every family's detour time",
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        sync::Mutex,
    };

    thread_local! {
        pub(super) static WORLD: Cell<Option<u64>> = const { Cell::new(None) };
        static READS: RefCell<Option<(bool, Vec<(Placement, bool)>)>> = const { RefCell::new(None) };
        static IMAGES: Cell<bool> = const { Cell::new(false) };
        static GATED: Cell<bool> = const { Cell::new(false) };
        static LOST: Cell<u32> = const { Cell::new(0) };
    }

    pub(super) fn read(placement: Placement, full: bool) -> Option<bool> {
        IMAGES.set(collector_characters::portrait_changes::pre_images_active());
        GATED.set(capture_gate::running());
        let admitted = READS.with_borrow_mut(|reads| {
            let (admitted, log) = reads.as_mut()?;
            log.push((placement, full));
            Some(*admitted)
        })?;
        if admitted {
            LOST.set(LOST.get() + account());
        }
        Some(admitted)
    }

    fn take_lost() -> u32 {
        LOST.take()
    }

    static SERIAL: Mutex<()> = Mutex::new(());

    pub fn serial() -> std::sync::MutexGuard<'static, ()> {
        let guard = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
        for flag in [
            &PENDING, &SETTLE, &FULL, &DRAINING, &BESIDE, &FAILED, &CLAIMED,
        ] {
            flag.store(false, Ordering::SeqCst);
        }
        for count in [
            &PENDING_WORLD,
            &CAPTURED_WORLD,
            &HOURS,
            &ACCOUNTED,
            &ENDED_WORLD,
        ] {
            count.store(0, Ordering::SeqCst);
        }
        LOST.set(0);
        crate::command_batch::tests::observe(false);
        guard
    }

    fn record(admitted: bool, world: u64) {
        WORLD.set(Some(world));
        READS.set(Some((admitted, Vec::new())));
    }

    fn admit(admitted: bool) {
        READS.with_borrow_mut(|reads| reads.as_mut().unwrap().0 = admitted);
    }

    fn take_reads() -> Vec<(Placement, bool)> {
        READS.with_borrow_mut(|reads| std::mem::take(&mut reads.as_mut().unwrap().1))
    }

    #[test]
    fn a_loaded_world_is_read_once_before_its_first_update_and_an_exhausted_one_never() {
        assert!(!reads_before_update(false, 3, 3));
        assert!(reads_before_update(true, 3, 3));
        assert!(reads_before_update(false, 5, 3));
        assert!(!reads_before_update(false, 5, 5));
        assert!(!reads_before_update(false, u64::MAX, 5));
        assert!(!reads_before_update(false, u64::MAX, u64::MAX));
    }

    #[test]
    fn a_capture_that_was_not_admitted_leaves_its_hour_unread_and_its_world_unmarked() {
        let _serial = serial();
        record(false, 3);
        hour_ended(Update::Uncounted);
        assert_eq!(take_reads(), [(Placement::Held, false)]);
        assert!(PENDING.load(Ordering::SeqCst));
        assert_eq!(CAPTURED_WORLD.load(Ordering::SeqCst), 0);
        crate::command_batch::tests::observe(true);
        batch_left();
        assert_eq!(take_reads(), [(Placement::Held, false)]);
        assert!(PENDING.load(Ordering::SeqCst));
        admit(true);
        update_entered();
        assert_eq!(take_reads(), [(Placement::Held, false)]);
        assert!(!PENDING.load(Ordering::SeqCst));
        assert_eq!(CAPTURED_WORLD.load(Ordering::SeqCst), 3);
        assert_eq!(take_lost(), 0);
        assert!(GATED.get());
        assert!(IMAGES.get());
        assert!(!capture_gate::running());
        assert!(!collector_characters::portrait_changes::pre_images_active());
    }

    #[test]
    fn the_hour_that_waits_on_a_refresh_is_the_one_read_in_full() {
        let _serial = serial();
        record(true, 3);
        hour_ended(Update::Uncounted);
        // SAFETY: The current history fixture owns settlement, or the active history worker is
        // requesting refresh before detach.
        unsafe { settle_next() };
        assert_eq!(take_reads(), [(Placement::Held, false)]);
        hour_ended(Update::Uncounted);
        batch_left();
        assert!(take_reads().is_empty());
        update_entered();
        assert_eq!(take_reads(), [(Placement::Held, true)]);
        hour_ended(Update::Uncounted);
        assert_eq!(take_reads(), [(Placement::Held, false)]);
        // SAFETY: The current history fixture owns settlement, or the active history worker is
        // requesting refresh before detach.
        unsafe { settle_next() };
        hour_ended(Update::Uncounted);
        admit(false);
        update_entered();
        admit(true);
        update_entered();
        assert_eq!(
            take_reads(),
            [(Placement::Held, true), (Placement::Held, true)]
        );
    }

    #[test]
    fn with_the_batch_hook_a_held_hour_is_read_at_its_batch_return_and_without_it_at_its_update() {
        let _serial = serial();
        record(true, 3);
        crate::command_batch::tests::observe(true);
        hour_ended(Update::Uncounted);
        assert!(take_reads().is_empty());
        assert!(PENDING.load(Ordering::SeqCst));
        batch_left();
        assert_eq!(take_reads(), [(Placement::Held, false)]);
        assert!(GATED.get());
        assert!(!PENDING.load(Ordering::SeqCst));
        crate::command_batch::tests::batching_elsewhere(true);
        hour_ended(Update::Uncounted);
        batch_left();
        assert!(take_reads().is_empty());
        assert!(PENDING.load(Ordering::SeqCst));
        let lost = LOST_TO_OTHER_BATCH.load(Ordering::SeqCst);
        update_entered();
        assert!(take_reads().is_empty());
        assert_eq!(LOST_TO_OTHER_BATCH.load(Ordering::SeqCst), lost + 1);
        crate::command_batch::tests::batching_elsewhere(false);
        assert_eq!(take_lost(), 0);
        hour_ended(Update::Uncounted);
        crate::command_batch::tests::observe(false);
        update_entered();
        assert_eq!(take_reads(), [(Placement::Held, false)]);
        assert_eq!(take_lost(), 1);
        assert!(!PENDING.load(Ordering::SeqCst));
        hour_ended(Update::Uncounted);
        assert_eq!(take_reads(), [(Placement::Held, false)]);
        assert!(!PENDING.load(Ordering::SeqCst));
    }

    #[test]
    fn a_batch_return_asks_for_a_capture_beside_the_game_only_while_no_other_thread_batches() {
        let _serial = serial();
        record(true, 3);
        BESIDE.store(true, Ordering::SeqCst);
        WORK.store(IDLE, Ordering::SeqCst);
        hour_ended(Update::Uncounted);
        assert!(take_reads().is_empty());
        crate::command_batch::tests::batching_elsewhere(true);
        batch_left();
        assert!(PENDING.load(Ordering::SeqCst));
        assert!(!capture_gate::running());
        assert_eq!(WORK.load(Ordering::SeqCst), IDLE);
        crate::command_batch::tests::batching_elsewhere(false);
        batch_left();
        assert!(take_reads().is_empty());
        assert!(!PENDING.load(Ordering::SeqCst));
        assert!(capture_gate::running());
        assert_eq!(WORK.load(Ordering::SeqCst), REQUESTED);
        WORK.store(IDLE, Ordering::SeqCst);
        serve(|_| {});
        assert!(!capture_gate::running());
    }

    #[test]
    fn no_held_capture_reads_while_another_thread_is_inside_a_batch() {
        let _serial = serial();
        record(true, 3);
        crate::command_batch::tests::batching_elsewhere(true);
        hour_ended(Update::Uncounted);
        batch_left();
        assert!(take_reads().is_empty());
        assert!(PENDING.load(Ordering::SeqCst));
        assert!(!capture_gate::running());
        let lost = LOST_TO_OTHER_BATCH.load(Ordering::SeqCst);
        update_entered();
        assert!(take_reads().is_empty());
        assert!(!PENDING.load(Ordering::SeqCst));
        assert_eq!(LOST_TO_OTHER_BATCH.load(Ordering::SeqCst), lost + 1);
        crate::command_batch::tests::batching_elsewhere(false);
        hour_ended(Update::Uncounted);
        assert_eq!(take_reads(), [(Placement::Held, false)]);
        assert_eq!(take_lost(), 1);
        assert!(GATED.get());
        assert!(!capture_gate::running());
    }

    #[test]
    fn an_unread_hour_is_lost_by_name_to_a_load_and_read_at_detach() {
        let _serial = serial();
        record(false, 3);
        hour_ended(Update::Uncounted);
        WORLD.set(Some(4));
        admit(true);
        let update = update_entered();
        assert_eq!(update, Update::Counted);
        assert_eq!(
            take_reads(),
            [(Placement::Held, false), (Placement::Held, false)]
        );
        assert_eq!(take_lost(), 1);
        assert_eq!(CAPTURED_WORLD.load(Ordering::SeqCst), 4);
        // SAFETY: The current history fixture owns settlement, or the active history worker is
        // requesting refresh before detach.
        unsafe { settle_next() };
        hour_ended(update);
        DRAINING.store(true, Ordering::SeqCst);
        crate::command_batch::tests::observe(true);
        batch_left();
        assert_eq!(take_reads(), [(Placement::Held, true)]);
        assert_eq!(take_lost(), 0);
    }

    #[test]
    fn the_final_account_loses_each_unread_hour_and_each_running_update_once() {
        let _serial = serial();
        record(true, 3);
        crate::command_batch::tests::observe(true);
        hour_ended(Update::Uncounted);
        batch_left();
        assert_eq!(take_reads(), [(Placement::Held, false)]);
        admit(false);
        hour_ended(Update::Uncounted);
        let outer = update_entered();
        PENDING.store(false, Ordering::SeqCst);
        let inner = update_entered();
        assert_eq!((outer, inner), (Update::Counted, Update::Counted));
        assert_eq!(
            // SAFETY: Detach has drained capture and transfers the loss count to this session's spool.
            unsafe { take_unread() },
            3
        );
        assert_eq!(
            // SAFETY: Detach has drained capture and transfers the loss count to this session's spool.
            unsafe { take_unread() },
            0
        );
        let _ = take_reads();
        hour_ended(inner);
        hour_ended(outer);
        PENDING.store(false, Ordering::SeqCst);
        assert_eq!(update_entered(), Update::AfterAccount);
        hour_ended(Update::AfterAccount);
        assert!(take_reads().is_empty());
        assert!(!PENDING.load(Ordering::SeqCst));
        assert_eq!(take_lost(), 0);
    }

    #[test]
    fn an_update_that_ended_before_the_final_account_is_lost_only_while_unread() {
        let _serial = serial();
        record(true, 3);
        let update = update_entered();
        hour_ended(update);
        assert_eq!(
            take_reads(),
            [(Placement::Held, false), (Placement::Held, false)]
        );
        assert_eq!(
            // SAFETY: Detach has drained capture and transfers the loss count to this session's spool.
            unsafe { take_unread() },
            0
        );
    }

    #[test]
    fn a_panicking_capture_opens_the_gate_and_every_later_capture_holds_the_game() {
        let _serial = serial();
        record(true, 3);
        BESIDE.store(true, Ordering::SeqCst);
        hour_ended(Update::Uncounted);
        // SAFETY: Serialized test opens the gate and `serve` releases it via `Gate`.
        assert!(unsafe { capture_gate::begin() });
        serve(|_| {
            assert_eq!(account(), 0);
            panic!("synthetic capture failure")
        });
        assert!(!capture_gate::running());
        assert!(!BESIDE.load(Ordering::SeqCst));
        assert!(FAILED.load(Ordering::SeqCst));
        assert!(owed());
        // SAFETY: Attach selects the capture mode before hourly hook publication.
        unsafe { set_beside(true) };
        assert!(!BESIDE.load(Ordering::SeqCst));
        // SAFETY: Serialized test opens the gate and `serve` releases it via `Gate`.
        assert!(unsafe { capture_gate::begin() });
        serve(|_| unreachable!("a failed capture thread reads no more"));
        assert!(!capture_gate::running());
        assert!(PENDING.load(Ordering::SeqCst));
        assert_eq!(
            // SAFETY: Detach has drained capture and transfers the loss count to this session's spool.
            unsafe { take_unread() },
            1
        );
    }
}
