//! When and where an hour is captured. An hour ends when the hourly update returns; its
//! capture reads the world as that update, and any command the same batch ran after it, left it
//! ([`crate::sampling::SAMPLE_BOUNDARY`]). The game writes its simulation only inside its
//! per-frame command batch, so once an outermost batch returns with no other thread inside one,
//! the capture runs on its own thread beside the frame, and game threads wait for it only where
//! they would write again ([`collector_timing::capture_gate`]). Where no capture thread runs
//! the capture holds the thread at that same batch return ([`crate::command_batch`]). Without a
//! command-batch hook proven to observe every batch no batch return is known: the hour is then
//! read holding the game straight after its update, before commands the same batch runs behind
//! it. Every capture, held or beside, first
//! closes the gate and reads only while no other thread is inside an observed batch. After a
//! refresh request the first hour to
//! end waits instead and is read in full, holding the game thread, as the update after it is
//! entered, so its frame has everything the game did in that hour. A world loaded since the last
//! capture began is read likewise as its first update is entered, so its loaded hour has a frame.
//! Every capture reads portrait holders as they were when it began ([`crate::portrait_changes`]).
//! An hour that ended and can no longer be read (its world was replaced, its capture failed,
//! another thread's batch kept it unread until the next update, or detach found it unread) is
//! recorded as a loss, by count: every capture, and detach's final account, records the hours
//! ended since the last capture that it does not read. An update still running at that account
//! is lost with them, and one entered after it is after the recording.
use crate::sampling::{self, Placement};
use collector_timing::capture_gate;
use std::{
    io,
    os::windows::io::AsRawHandle,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows_sys::Win32::System::Threading::{
    GetCurrentThread, GetThreadId, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
    THREAD_PRIORITY_BELOW_NORMAL, WaitOnAddress, WakeByAddressAll,
};

/// Readers that work beside the running game; the pool's others are its reserves.
pub(crate) const BESIDE_READERS: usize = 3;

const IDLE: u32 = 0;
const REQUESTED: u32 = 1;
const STOP: u32 = 2;

/// An hour ended, or the state at attach waits, and no capture has read it.
static PENDING: AtomicBool = AtomicBool::new(true);
/// The world generation the pending state belongs to; zero for the state at attach.
static PENDING_WORLD: AtomicU64 = AtomicU64::new(0);
/// The world generation the last capture began in ([`crate::world_lifetime::generation`]).
static CAPTURED_WORLD: AtomicU64 = AtomicU64::new(0);
/// A refresh was requested: the first hour to end after it waits for the update after it.
static SETTLE: AtomicBool = AtomicBool::new(false);
/// The pending hour waits for the next update entry and is then read in full.
static FULL: AtomicBool = AtomicBool::new(false);
/// Detach: no hour waits for another update, and the unread one is read at the next batch return.
static DRAINING: AtomicBool = AtomicBool::new(false);
/// Captures leave the game thread: the command batch is hooked and the capture thread runs.
static BESIDE: AtomicBool = AtomicBool::new(false);
static WORK: AtomicU32 = AtomicU32::new(IDLE);
/// Whether the requested capture reads in full.
static WORK_FULL: AtomicBool = AtomicBool::new(false);
/// The capture thread's handle while it runs, for the waiting game thread to raise it.
static THREAD: AtomicIsize = AtomicIsize::new(0);
/// One plus the processor the game thread left its command batch on; zero when unknown.
static GAME: AtomicU32 = AtomicU32::new(0);
/// A writer waits for the running capture: it reads at the hold pace.
static HURRIED: AtomicBool = AtomicBool::new(false);
/// The capture thread keeps off the game thread's core.
static CONFINED: AtomicBool = AtomicBool::new(false);
/// A capture panicked on the capture thread: every capture since holds the game.
static FAILED: AtomicBool = AtomicBool::new(false);
static BESIDE_CAPTURES: AtomicU64 = AtomicU64::new(0);
static CONFINED_CAPTURES: AtomicU64 = AtomicU64::new(0);
static HELD_CAPTURES: AtomicU64 = AtomicU64::new(0);
static CAPTURE_PANICS: AtomicU64 = AtomicU64::new(0);
static LOST_TO_LOAD: AtomicU64 = AtomicU64::new(0);
static LOST_TO_OTHER_BATCH: AtomicU64 = AtomicU64::new(0);
/// Hours ended, updates running, and the final account's mark, in one word: that account sees
/// every update as ended, running or after it.
static HOURS: AtomicU64 = AtomicU64::new(0);
const ENDED: u64 = (1 << RUNNING_SHIFT) - 1;
const RUNNING_SHIFT: u32 = 40;
const RUNNING: u64 = 1 << RUNNING_SHIFT;
/// Detach took its final account: an update entered from now is after the recording.
const CLOSED: u64 = 1 << 63;
/// The count of ended hours the frames account for, each read or recorded as lost. Only a
/// capture inside its admission and the final account move it.
static ACCOUNTED: AtomicU64 = AtomicU64::new(0);
/// The world generation the last hour ended in.
static ENDED_WORLD: AtomicU64 = AtomicU64::new(0);
/// The capture running on the capture thread accounted the last hour to end as the one it reads.
static CLAIMED: AtomicBool = AtomicBool::new(false);

/// What [`update_entered`] found, for the same update's [`hour_ended`]. It travels with the
/// call, so an update the game runs inside another keeps its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Update {
    /// No update was entered through [`update_entered`].
    #[default]
    Uncounted,
    /// Entered before the final account and counted as running until its hour ends.
    Counted,
    /// Entered after the final account: after the recording.
    AfterAccount,
}

/// Adds `step` to the hour counts unless the final account was taken.
fn count(step: u64) -> bool {
    HOURS
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |hours| {
            (hours & CLOSED == 0).then_some(hours.wrapping_add(step))
        })
        .is_ok()
}

/// Whether an hour ended that no frame accounts for.
fn owed() -> bool {
    HOURS.load(Ordering::SeqCst) & ENDED != ACCOUNTED.load(Ordering::SeqCst)
}
static BESIDE_NS: AtomicU64 = AtomicU64::new(0);

/// The hourly update is about to run: the hour before it must be read first, when it ended
/// unread or a world has been loaded since the last capture began.
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
            // Another thread's batch kept it from being read, and this update writes over it.
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

/// An exhausted generation never matches again, so it never counts as a change.
fn reads_before_update(pending: bool, world: u64, captured: u64) -> bool {
    pending || (world != u64::MAX && world != captured)
}

/// The hourly update returned. Where batch returns are observed the hour waits for one, so
/// the commands its batch runs behind the update are in its frame wherever it is read.
pub(crate) fn hour_ended(update: Update) {
    let step = match update {
        Update::AfterAccount => return,
        // No longer running and ended, in one step.
        Update::Counted => 1u64.wrapping_sub(RUNNING),
        Update::Uncounted => 1,
    };
    let world = world();
    ENDED_WORLD.store(world, Ordering::SeqCst);
    // An update the final account found running is already lost there.
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

/// An outermost batch returned on this thread: once no other thread is inside one, nothing
/// writes the simulation until the next. Every update writes over an unread hour only after
/// [`update_entered`] handled it, so the hour pending here is the state's own.
pub(crate) fn batch_left() {
    if FULL.load(Ordering::Acquire) && !DRAINING.load(Ordering::Acquire) {
        return;
    }
    // An unproven hook may have been published inside a running batch on any thread: its
    // returns prove nothing, and the hour waits for the next update's entry.
    if !BESIDE.load(Ordering::Acquire) && !crate::command_batch::observed() {
        return;
    }
    if !PENDING.load(Ordering::Acquire) || !PENDING.swap(false, Ordering::AcqRel) {
        return;
    }
    let full = FULL.swap(false, Ordering::AcqRel);
    forget_replaced(world());
    if !BESIDE.load(Ordering::Acquire) {
        // This thread is in this batch's caller: it writes nothing while it reads here.
        if held(full) != Held::Read {
            unread(full);
        }
        return;
    }
    // One capture at a time; closed first, so a batch entering from now waits, and one already
    // inside is counted.
    if !capture_gate::begin() {
        unread(full);
        return;
    }
    if crate::command_batch::others_batching() {
        capture_gate::end();
        unread(full);
        return;
    }
    // `Thread::stop` clears BESIDE, then waits for the gate: it or this sees the other.
    if !BESIDE.load(Ordering::SeqCst) {
        capture_gate::end();
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
    // SAFETY: an aligned u32 that lives for the process.
    unsafe { WakeByAddressAll(WORK.as_ptr().cast()) };
}

/// The recorder's refresh request: the first hour whose update returns after this call waits,
/// and is read in full, holding the game, as the update after it is entered, so its frame has
/// everything the game did in that hour. An hour already ended when it is called is read as
/// usual.
pub(crate) fn settle_next() {
    SETTLE.store(true, Ordering::Release);
}

/// Detach, once the hourly update is restored: no hour waits for another update, and the one
/// still unread is read at the next command batch's return. True once none is unread and no
/// capture runs; false at `deadline`.
pub(crate) fn drain(deadline: Instant) -> bool {
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

/// The final account, once no capture runs or can start: the hours now lost. They are the
/// hours that ended with no frame accounting for them, whichever thread holds one unread, and
/// the updates still running, whose hours no capture can read. An update that enters later is
/// after the recording. Zero when taken again.
pub(crate) fn take_unread() -> u32 {
    let hours = HOURS.fetch_or(CLOSED, Ordering::SeqCst);
    if hours & CLOSED != 0 {
        return 0;
    }
    let ended = hours & ENDED;
    let unread = ended.saturating_sub(ACCOUNTED.swap(ended, Ordering::SeqCst));
    u32::try_from(unread.saturating_add(hours >> RUNNING_SHIFT)).unwrap_or(u32::MAX)
}

/// The running capture, inside its admission: the hours ended since the last capture that it
/// does not read, to record as lost before its own. It reads the last hour to end, unless that
/// hour's world has been replaced since.
pub(crate) fn account() -> u32 {
    let ended = HOURS.load(Ordering::SeqCst) & ENDED;
    let owed = ended.saturating_sub(ACCOUNTED.swap(ended, Ordering::SeqCst));
    let reads = owed != 0 && !replaced(ENDED_WORLD.load(Ordering::SeqCst), world());
    CLAIMED.store(reads, Ordering::SeqCst);
    u32::try_from(owed - u64::from(reads)).unwrap_or(u32::MAX)
}

/// A state of world `ended` can no longer be read in `world`.
fn replaced(ended: u64, world: u64) -> bool {
    ended != 0 && world != u64::MAX && ended != world
}

fn world() -> u64 {
    #[cfg(test)]
    if let Some(world) = tests::WORLD.get() {
        return world;
    }
    crate::world_lifetime::generation()
}

/// The pending state belongs to a world since replaced: it can no longer be read.
fn forget_replaced(world: u64) {
    if replaced(PENDING_WORLD.load(Ordering::Acquire), world) {
        if owed() {
            LOST_TO_LOAD.fetch_add(1, Ordering::Relaxed);
        }
        PENDING_WORLD.store(world, Ordering::Release);
    }
}

/// The hour stays unread, for the next point that can read it.
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
    // SAFETY: forwarded from the caller.
    unsafe { sampling::capture(placement, full) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    Read,
    /// Not admitted, or another capture runs: nothing was read.
    Refused,
    /// Another thread is inside a batch: nothing was read.
    Busy,
}

/// A capture on this thread, which writes nothing while it reads.
fn held(full: bool) -> Held {
    // Never two captures at once. Closed first: a batch entering from now waits, and one already
    // inside is counted.
    if !capture_gate::begin() {
        return Held::Refused;
    }
    let _gate = Gate;
    if crate::command_batch::others_batching() {
        return Held::Busy;
    }
    // Portrait writers on other threads run on, logging what they change.
    let _images = crate::portrait_changes::PreImagesCapture::begin();
    let world = world();
    // SAFETY: the game thread, between its writes, with no other capture or batch running.
    if !unsafe { read(Placement::Held, full) } {
        return Held::Refused;
    }
    HELD_CAPTURES.fetch_add(1, Ordering::Relaxed);
    CAPTURED_WORLD.store(world, Ordering::SeqCst);
    Held::Read
}

/// A writer waits for the capture reading beside the game: every reader, above normal priority
/// and on any core. The first waiter of a capture does it for all.
fn hurry() {
    if HURRIED.swap(true, Ordering::SeqCst) {
        return;
    }
    sampling::hurry();
    let thread = THREAD.load(Ordering::SeqCst);
    if thread != 0 {
        // SAFETY: `Thread::stop` joins, closing the handle, only once no waiter is counted.
        unsafe { SetThreadPriority(thread as _, THREAD_PRIORITY_ABOVE_NORMAL) };
        if CONFINED.swap(false, Ordering::SeqCst) {
            crate::frame_priority::select_away(thread as _, None);
        }
    }
}

/// Whether a writer waits for the running capture.
pub(crate) fn hurried() -> bool {
    HURRIED.load(Ordering::SeqCst)
}

/// Whether captures run beside the game. Set once every hook is installed and hourly captures
/// are admitted.
pub(crate) fn set_beside(beside: bool) {
    BESIDE.store(beside && !FAILED.load(Ordering::SeqCst), Ordering::SeqCst);
}

/// The thread that captures beside the game.
pub(crate) struct Thread {
    handle: Option<JoinHandle<()>>,
    id: u32,
}

impl Thread {
    pub(crate) fn start(stack_bytes: usize) -> io::Result<Self> {
        capture_gate::on_wait(hurry);
        WORK.store(IDLE, Ordering::Release);
        let handle = std::thread::Builder::new()
            .name("hoi4-capture".into())
            .stack_size(stack_bytes)
            .spawn(run)?;
        let raw = handle.as_raw_handle();
        THREAD.store(raw as isize, Ordering::Release);
        // SAFETY: a live thread handle owned by `handle`.
        let id = unsafe { GetThreadId(raw as _) };
        crate::thread_cpu::register_id(crate::thread_cpu::Role::Capture, id);
        Ok(Self {
            handle: Some(handle),
            id,
        })
    }

    pub(crate) fn id(&self) -> u32 {
        self.id
    }

    /// No further capture leaves the game thread; the running one ends and the thread is joined.
    pub(crate) fn stop(&mut self) {
        BESIDE.store(false, Ordering::SeqCst);
        while capture_gate::running() {
            std::thread::yield_now();
        }
        // A waiter counts itself before loading THREAD, so once none is counted none holds the
        // handle the join closes.
        THREAD.store(0, Ordering::SeqCst);
        while capture_gate::waiting() != 0 {
            std::thread::yield_now();
        }
        WORK.store(STOP, Ordering::Release);
        // SAFETY: an aligned u32 that lives for the process.
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
        self.stop();
    }
}

/// Opens the gate a capture closed on every exit, an unwind included: every waiting writer goes
/// on.
struct Gate;
impl Drop for Gate {
    fn drop(&mut self) {
        capture_gate::end();
    }
}

/// Paces the capture thread for a capture beside the game thread on processor `game`: below
/// its priority and off its physical core, unless a writer already waits. Whether it is off it.
fn beside(away: &mut crate::frame_priority::AwayFromGame, game: Option<u32>) -> bool {
    // SAFETY: the pseudo-handle names only this thread.
    let own = unsafe { GetCurrentThread() };
    // SAFETY: as above.
    unsafe { SetThreadPriority(own, THREAD_PRIORITY_BELOW_NORMAL) };
    away.follow(game);
    CONFINED.store(away.confined(), Ordering::SeqCst);
    // `hurry` sets HURRIED, then clears CONFINED: it or this sees the other.
    if HURRIED.load(Ordering::SeqCst) {
        // SAFETY: as above.
        unsafe { SetThreadPriority(own, THREAD_PRIORITY_ABOVE_NORMAL) };
        CONFINED.store(false, Ordering::SeqCst);
        away.follow(None);
    }
    away.confined()
}

fn run() {
    capture_gate::reader();
    let mut away = crate::frame_priority::AwayFromGame::default();
    let idle = IDLE;
    loop {
        match WORK.load(Ordering::Acquire) {
            STOP => return,
            REQUESTED => {
                WORK.store(IDLE, Ordering::Release);
                serve(|full| capture_beside(&mut away, full));
            }
            _ => {
                // SAFETY: an aligned u32 compared with a local of its size.
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

/// One requested capture beside the game. A panic in it is contained: the hour it was reading
/// is unaccounted again, so the next capture or the final account records it as lost, and
/// every later capture holds the game, so no writer waits for this thread.
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
        // Before the gate opens: the next batch's return sees captures hold the game.
        BESIDE.store(false, Ordering::SeqCst);
        // Detach joins this thread before its final account.
        if CLAIMED.swap(false, Ordering::SeqCst) {
            ACCOUNTED.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn capture_beside(away: &mut crate::frame_priority::AwayFromGame, full: bool) {
    let game = GAME.load(Ordering::Relaxed).checked_sub(1);
    let started = Instant::now();
    if beside(away, game) {
        CONFINED_CAPTURES.fetch_add(1, Ordering::Relaxed);
    }
    let _images = crate::portrait_changes::PreImagesCapture::begin();
    // Every world change waits at the gate for this capture to end.
    let world = world();
    // SAFETY: the gate holds every writer until this capture has ended.
    let admitted = unsafe { read(Placement::Beside(game), full) };
    CONFINED.store(false, Ordering::SeqCst);
    away.follow(None);
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

pub(crate) fn status() -> impl serde::Serialize {
    crate::status::status!({
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
        "off_game_core_failures": crate::frame_priority::away_failures(),
        "held": HELD_CAPTURES.load(Ordering::Relaxed),
        "batch_publication": crate::command_batch::publication_status(),
        "batch_publications_held": crate::command_batch::publications_held(),
        "portraits_withheld_writer": crate::portrait_changes::withheld_captures(),
        "capture_thread_panics": CAPTURE_PANICS.load(Ordering::Relaxed),
        "hours_lost_to_load": LOST_TO_LOAD.load(Ordering::Relaxed),
        "hours_lost_to_other_batch": LOST_TO_OTHER_BATCH.load(Ordering::Relaxed),
        "writer_wait_ns": capture_gate::wait_ns(),
        "scope": "cumulative since attach: captures run beside the game on the capture thread (beside, and their wall in beside_ns) once an outermost command batch returns, and on a game thread while it is held (held: at that same batch return without a capture thread or once the capture thread has failed (capture_thread_panics); straight after the update without a command-batch hook proven to observe every batch (batch_publication says why), when a frame misses commands the same batch runs behind the update; and as the next update is entered for an hour that waited on a refresh request, the hour a world was loaded at, or an hour no earlier point could read); every capture first closes the capture gate and reads only while no other thread is inside an observed batch; batch_publications_held counts publications of the command-batch hook held because a thread may have been inside the batch, which is published unproven once they reach their limit; every capture reads character portraits as they were when it began, and portraits_withheld_writer counts captures whose portrait reads were all withheld by name because a portrait writer was still running when the capture's bounded wait for writers ran out, or ran on the capturing thread itself; beside_off_game_core counts beside captures whose capture thread started off the game thread's physical core, its readers likewise, and off_game_core_failures the refused CPU-set selections or clearings for it; hours_lost_to_load counts hours that ended but whose world was replaced before any capture could read them, and hours_lost_to_other_batch hours another thread's batch kept from being read until the next update wrote over them, each recorded as a lost hour; writer_wait_ns is the total time game threads waited for a capture behind the closed gate, at a hooked writer's or the command batch's entry, or at a portrait writer the pre-image store could not hold: hook_timing.families.*.capture_waits counts those waits by family, and the entry waits lie outside every family's detour time",
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        sync::Mutex,
    };

    thread_local! {
        pub(super) static WORLD: Cell<Option<u64>> = const { Cell::new(None) };
        static READS: RefCell<Option<(bool, Vec<(Placement, bool)>)>> = const { RefCell::new(None) };
        /// Whether portrait writers logged during this thread's last read.
        static IMAGES: Cell<bool> = const { Cell::new(false) };
        /// Whether the capture gate was closed during this thread's last read.
        static GATED: Cell<bool> = const { Cell::new(false) };
        /// The hours this thread's admitted captures recorded as lost.
        static LOST: Cell<u32> = const { Cell::new(0) };
    }

    /// Captures on this thread are recorded and answered with the admission it set.
    pub(super) fn read(placement: Placement, full: bool) -> Option<bool> {
        IMAGES.set(crate::portrait_changes::pre_images_active());
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

    /// Serializes tests that drive the capture point's process-wide state, from a fresh one.
    pub(crate) fn serial() -> std::sync::MutexGuard<'static, ()> {
        let guard = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
        for flag in [
            &PENDING, &SETTLE, &FULL, &DRAINING, &BESIDE, &FAILED, &CLAIMED,
        ] {
            flag.store(false, Ordering::SeqCst);
        }
        for count in [&PENDING_WORLD, &CAPTURED_WORLD, &HOURS, &ACCOUNTED, &ENDED_WORLD] {
            count.store(0, Ordering::SeqCst);
        }
        LOST.set(0);
        crate::command_batch::tests::observe(false);
        guard
    }

    /// Records this thread's captures, each answered `admitted`, in world `world`.
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
        // Attached while paused: the pending state was read beside the game in world 3.
        assert!(!reads_before_update(false, 3, 3));
        assert!(reads_before_update(true, 3, 3));
        // A save loaded since: its hour is read as the first update is entered, then not again.
        assert!(reads_before_update(false, 5, 3));
        assert!(!reads_before_update(false, 5, 5));
        // Exhaustion is no world change, before or after a capture began in it.
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
        // A proven hook's batch return tries it again, and once admitted it is read and marked.
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
        // It read behind the closed gate, portrait writers on other threads logging, and both end.
        assert!(GATED.get());
        assert!(IMAGES.get());
        assert!(!capture_gate::running());
        assert!(!crate::portrait_changes::pre_images_active());
    }

    #[test]
    fn the_hour_that_waits_on_a_refresh_is_the_one_read_in_full() {
        let _serial = serial();
        record(true, 3);
        // An hour already ended when the request arrives is read as usual.
        hour_ended(Update::Uncounted);
        settle_next();
        assert_eq!(take_reads(), [(Placement::Held, false)]);
        // The first hour to end after it waits through its batch's return and is read in full
        // as the next update is entered.
        hour_ended(Update::Uncounted);
        batch_left();
        assert!(take_reads().is_empty());
        update_entered();
        assert_eq!(take_reads(), [(Placement::Held, true)]);
        // The hour after it is ordinary again.
        hour_ended(Update::Uncounted);
        assert_eq!(take_reads(), [(Placement::Held, false)]);
        // Not admitted, the waiting hour keeps its full read for the next attempt.
        settle_next();
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
        // Behind the commands its batch runs after the update, as beside the game.
        hour_ended(Update::Uncounted);
        assert!(take_reads().is_empty());
        assert!(PENDING.load(Ordering::SeqCst));
        batch_left();
        assert_eq!(take_reads(), [(Placement::Held, false)]);
        assert!(GATED.get());
        assert!(!PENDING.load(Ordering::SeqCst));
        // Busy there, it waits, and is lost by name when the next update writes over it.
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
        // The hook withdrawn while an hour waits: the next update's entry reads it, and its
        // capture records the hour lost before it.
        assert_eq!(take_lost(), 0);
        hour_ended(Update::Uncounted);
        crate::command_batch::tests::observe(false);
        update_entered();
        assert_eq!(take_reads(), [(Placement::Held, false)]);
        assert_eq!(take_lost(), 1);
        assert!(!PENDING.load(Ordering::SeqCst));
        // Without the hook no batch return is known: read as the update returns.
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
        // The hour stays unread at its update's end and at its batch's return, the gate open.
        hour_ended(Update::Uncounted);
        batch_left();
        assert!(take_reads().is_empty());
        assert!(PENDING.load(Ordering::SeqCst));
        assert!(!capture_gate::running());
        // The next update writes over it: a loss by name.
        let lost = LOST_TO_OTHER_BATCH.load(Ordering::SeqCst);
        update_entered();
        assert!(take_reads().is_empty());
        assert!(!PENDING.load(Ordering::SeqCst));
        assert_eq!(LOST_TO_OTHER_BATCH.load(Ordering::SeqCst), lost + 1);
        // Once the other batch has returned, the next hour is read behind the closed gate, and
        // its capture records the lost one.
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
        // A save is loaded before any capture is admitted: the old world's hour is lost.
        WORLD.set(Some(4));
        admit(true);
        let update = update_entered();
        assert_eq!(update, Update::Counted);
        assert_eq!(take_reads(), [(Placement::Held, false), (Placement::Held, false)]);
        assert_eq!(take_lost(), 1);
        assert_eq!(CAPTURED_WORLD.load(Ordering::SeqCst), 4);
        // Detach reads an hour that waits on a refresh at the next batch return, in full.
        settle_next();
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
        // An hour that was read is not lost.
        hour_ended(Update::Uncounted);
        batch_left();
        assert_eq!(take_reads(), [(Placement::Held, false)]);
        // One hour ends unread, held by no flag while an update that took it is refused, and
        // two updates are still running when no capture can run any more.
        admit(false);
        hour_ended(Update::Uncounted);
        let outer = update_entered();
        PENDING.store(false, Ordering::SeqCst);
        let inner = update_entered();
        assert_eq!((outer, inner), (Update::Counted, Update::Counted));
        assert_eq!(take_unread(), 3);
        assert_eq!(take_unread(), 0);
        // Their hours end behind that account, already lost there, and read nothing.
        let _ = take_reads();
        hour_ended(inner);
        hour_ended(outer);
        // An update entered after it is after the recording: its hour is not published.
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
        // Read straight after its update: counted as running, then ended, then accounted.
        let update = update_entered();
        hour_ended(update);
        assert_eq!(take_reads(), [(Placement::Held, false), (Placement::Held, false)]);
        assert_eq!(take_unread(), 0);
    }

    #[test]
    fn a_panicking_capture_opens_the_gate_and_every_later_capture_holds_the_game() {
        let _serial = serial();
        record(true, 3);
        BESIDE.store(true, Ordering::SeqCst);
        hour_ended(Update::Uncounted);
        assert!(capture_gate::begin());
        serve(|_| {
            assert_eq!(account(), 0);
            panic!("synthetic capture failure")
        });
        assert!(!capture_gate::running());
        assert!(!BESIDE.load(Ordering::SeqCst));
        assert!(FAILED.load(Ordering::SeqCst));
        // The hour it was reading is unaccounted again.
        assert!(owed());
        set_beside(true);
        assert!(!BESIDE.load(Ordering::SeqCst));
        // A request already posted reads nothing on the failed thread and leaves its hour.
        assert!(capture_gate::begin());
        serve(|_| unreachable!("a failed capture thread reads no more"));
        assert!(!capture_gate::running());
        assert!(PENDING.load(Ordering::SeqCst));
        assert_eq!(take_unread(), 1);
    }
}
