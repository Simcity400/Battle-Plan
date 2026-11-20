//! The gate between a capture and the game's writers on other threads. The game writes its
//! simulation only in its command batch and the hooked writers that reaches, so a hooked callback
//! entered while a capture runs waits here until it has ended; the capture therefore reads one
//! settled state. Nothing waits while no capture runs.
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
/// Callers whose waits are counted apart; later ones share the last slot.
pub const SLOTS: usize = 128;

static STATE: AtomicU32 = AtomicU32::new(IDLE);
/// Threads inside [`wait`]; the collector stays loaded while one is.
static WAITING: AtomicU32 = AtomicU32::new(0);
static WAITS: [AtomicU64; SLOTS] = [const { AtomicU64::new(0) }; SLOTS];
/// Callers whose changes every reader takes as they were when the capture began, so they pass.
static EXEMPT: [AtomicBool; SLOTS] = [const { AtomicBool::new(false) }; SLOTS];
static WAIT_NS: AtomicU64 = AtomicU64::new(0);
static HURRY: OnceLock<fn()> = OnceLock::new();
thread_local!(static READER: Cell<bool> = const { Cell::new(false) });

/// The calling thread reads for the collector: it never waits for the collector's own capture.
pub fn reader() {
    READER.set(true);
}

/// What a waiting writer calls first, so the capture it waits for finishes sooner.
pub fn on_wait(hurry: fn()) {
    let _ = HURRY.set(hurry);
}

/// A capture begins. False when one already runs.
pub fn begin() -> bool {
    STATE
        .compare_exchange(IDLE, RUNNING, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

/// The capture ended: every waiting writer goes on.
pub fn end() {
    STATE.store(IDLE, Ordering::SeqCst);
    // A waiter counts itself, then looks at the state: it or this sees the other.
    if WAITING.load(Ordering::SeqCst) != 0 {
        // SAFETY: an aligned u32 that lives for the process.
        unsafe { WakeByAddressAll(STATE.as_ptr().cast()) };
    }
}

pub fn running() -> bool {
    STATE.load(Ordering::SeqCst) == RUNNING
}

/// Whether `slot`'s writers may run beside a capture: only while every reader of what they
/// write takes it as it was when the capture began.
pub fn exempt(slot: usize, exempt: bool) {
    EXEMPT[slot.min(SLOTS - 1)].store(exempt, Ordering::SeqCst);
}

/// A writer is about to run: wait for a running capture.
#[inline(always)]
pub fn pass(slot: usize) {
    if STATE.load(Ordering::Relaxed) == RUNNING {
        wait(slot);
    }
}

/// A writer of an exempt family that cannot run beside this capture after all waits for it,
/// counted as any waiting writer.
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
        // SAFETY: an aligned u32 compared with a local of its size; the timeout bounds a missed wake.
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

/// Threads waiting at the gate now.
pub fn waiting() -> u32 {
    WAITING.load(Ordering::SeqCst)
}

/// Waits since attach by `slot`.
pub fn waits(slot: usize) -> u64 {
    WAITS[slot.min(SLOTS - 1)].load(Ordering::Relaxed)
}

/// Time writers have spent waiting since attach, in nanoseconds.
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
        // Other tests' spans pass this gate too: none enters this family, has the exempt slot
        // or sets IS_WRITER.
        const WRITER: usize = Family::CommandBatch as usize;
        const EXEMPT_WRITER: usize = SLOTS - 1;
        thread_local!(static IS_WRITER: Cell<bool> = const { Cell::new(false) });
        static ARRIVED: AtomicBool = AtomicBool::new(false);
        on_wait(|| {
            if IS_WRITER.get() {
                ARRIVED.store(true, Ordering::Release);
            }
        });
        assert!(begin());
        assert!(!begin());
        // A hooked callback waits at its shared entry.
        let waiter = std::thread::spawn(|| {
            IS_WRITER.set(true);
            drop(enter(Family::CommandBatch));
        });
        while !ARRIVED.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        assert!(running());
        assert_ne!(waiting(), 0);
        end();
        waiter.join().unwrap();
        assert_eq!(waits(WRITER), 1);
        drop(enter(Family::CommandBatch));
        assert_eq!(waits(WRITER), 1);
        assert!(begin());
        exempt(EXEMPT_WRITER, true);
        pass(EXEMPT_WRITER);
        assert_eq!(waits(EXEMPT_WRITER), 0);
        // An exempt writer that cannot run beside the capture waits all the same.
        let holder = std::thread::spawn(|| {
            IS_WRITER.set(true);
            hold(EXEMPT_WRITER);
        });
        while waiting() == 0 {
            std::thread::yield_now();
        }
        // A collector reader's hooked call passes the running capture at once.
        std::thread::spawn(|| {
            reader();
            drop(enter(Family::CommandBatch));
        })
        .join()
        .unwrap();
        assert_eq!(waits(WRITER), 1);
        end();
        holder.join().unwrap();
        assert_eq!(waits(EXEMPT_WRITER), 1);
        hold(EXEMPT_WRITER);
        assert_eq!(waits(EXEMPT_WRITER), 1);
    }
}
