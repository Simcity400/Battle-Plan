use std::{
    cell::Cell,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
};
use windows_sys::Win32::System::Threading::{WaitOnAddress, WakeByAddressAll};

const IDLE: u32 = 0;
const RUNNING: u32 = 1;
pub const SLOTS: usize = 128;

static STATE: AtomicU32 = AtomicU32::new(IDLE);
static WAITING: AtomicU32 = AtomicU32::new(0);
static WAITS: [AtomicU64; SLOTS] = [const { AtomicU64::new(0) }; SLOTS];
static EXEMPT: [AtomicBool; SLOTS] = [const { AtomicBool::new(false) }; SLOTS];
static WAIT_NS: AtomicU64 = AtomicU64::new(0);
static HURRY: OnceLock<fn()> = OnceLock::new();
thread_local!(static READER: Cell<bool> = const { Cell::new(false) });

pub fn reader() {
    READER.set(true);
}

pub fn on_wait(hurry: fn()) {
    let _ = HURRY.set(hurry);
}

/// # Safety
/// Only the admitted hourly capture may take the process-wide gate. A successful acquisition must
/// be released once after capture finishes, including if the beside worker takes ownership.
pub unsafe fn begin() -> bool {
    STATE
        .compare_exchange(IDLE, RUNNING, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

/// # Safety
/// Release only the live gate acquired by this hour's successful begin, after the protected capture
/// finishes or is refused and before allowing the next hour.
pub unsafe fn end() {
    STATE.store(IDLE, Ordering::SeqCst);
    if WAITING.load(Ordering::SeqCst) != 0 {
        unsafe { WakeByAddressAll(STATE.as_ptr().cast()) };
    }
}

pub fn running() -> bool {
    STATE.load(Ordering::SeqCst) == RUNNING
}

pub fn exempt(slot: usize, exempt: bool) {
    EXEMPT[slot.min(SLOTS - 1)].store(exempt, Ordering::SeqCst);
}

#[inline(always)]
pub fn pass(slot: usize) {
    if STATE.load(Ordering::Relaxed) == RUNNING {
        wait(slot);
    }
}

pub fn hold(slot: usize) {
    if STATE.load(Ordering::SeqCst) == RUNNING && !READER.get() {
        wait_out(slot);
    }
}

#[cold]
fn wait(slot: usize) {
    if READER.get() || EXEMPT[slot.min(SLOTS - 1)].load(Ordering::SeqCst) {
        return;
    }
    wait_out(slot);
}

#[cold]
fn wait_out(slot: usize) {
    WAITING.fetch_add(1, Ordering::SeqCst);
    let started = std::time::Instant::now();
    if let Some(hurry) = HURRY.get() {
        hurry();
    }
    let running = RUNNING;
    while STATE.load(Ordering::SeqCst) == RUNNING {
        unsafe {
            WaitOnAddress(
                STATE.as_ptr().cast(),
                (&running as *const u32).cast(),
                size_of::<u32>(),
                100,
            )
        };
    }
    WAITS[slot.min(SLOTS - 1)].fetch_add(1, Ordering::Relaxed);
    WAIT_NS.fetch_add(
        u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
        Ordering::Relaxed,
    );
    WAITING.fetch_sub(1, Ordering::SeqCst);
}

pub fn waiting() -> u32 {
    WAITING.load(Ordering::SeqCst)
}

pub fn waits(slot: usize) -> u64 {
    WAITS[slot.min(SLOTS - 1)].load(Ordering::Relaxed)
}

pub fn wait_ns() -> u64 {
    WAIT_NS.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_writer_waits_for_the_running_capture_and_is_counted_once_and_a_reader_never() {
        use crate::hook_timing::{Family, enter};
        use std::cell::Cell;
        const WRITER: usize = Family::CommandBatch as usize;
        const EXEMPT_WRITER: usize = SLOTS - 1;
        thread_local!(static IS_WRITER: Cell<bool> = const { Cell::new(false) });
        static ARRIVED: AtomicBool = AtomicBool::new(false);
        on_wait(|| {
            if IS_WRITER.get() {
                ARRIVED.store(true, Ordering::Release);
            }
        });
        assert!(
            // SAFETY: The admitted producer owns this hour's gate and settles it on completion or
            // refusal.
            unsafe { begin() }
        );
        assert!(!
        // SAFETY: The admitted producer owns this hour's gate and settles it on completion or
        // refusal.
        unsafe { begin() });
        let waiter = std::thread::spawn(|| {
            IS_WRITER.set(true);
            drop(enter(Family::CommandBatch));
        });
        while !ARRIVED.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        assert!(running());
        assert_ne!(waiting(), 0);
        // SAFETY: This is the release of the admitted hour's gate after its protected work ends.
        unsafe { end() };
        waiter.join().unwrap();
        assert_eq!(waits(WRITER), 1);
        drop(enter(Family::CommandBatch));
        assert_eq!(waits(WRITER), 1);
        assert!(
            // SAFETY: The admitted producer owns this hour's gate and settles it on completion or
            // refusal.
            unsafe { begin() }
        );
        exempt(EXEMPT_WRITER, true);
        pass(EXEMPT_WRITER);
        assert_eq!(waits(EXEMPT_WRITER), 0);
        let holder = std::thread::spawn(|| {
            IS_WRITER.set(true);
            hold(EXEMPT_WRITER);
        });
        while waiting() == 0 {
            std::thread::yield_now();
        }
        std::thread::spawn(|| {
            reader();
            drop(enter(Family::CommandBatch));
        })
        .join()
        .unwrap();
        assert_eq!(waits(WRITER), 1);
        // SAFETY: This is the release of the admitted hour's gate after its protected work ends.
        unsafe { end() };
        holder.join().unwrap();
        assert_eq!(waits(EXEMPT_WRITER), 1);
        hold(EXEMPT_WRITER);
        assert_eq!(waits(EXEMPT_WRITER), 1);
    }
}
