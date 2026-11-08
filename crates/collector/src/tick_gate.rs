//! Pipeline work that reads live game memory waits out the game's hourly tick; it runs between
//! ticks and in the capture pause. Threads that work only on copies (decode, spool, conversion
//! helpers) work through the tick at idle priority, off the main game thread's core
//! ([`crate::frame_priority::Pipeline::tick`]); their few live reads skip a tick instead
//! ([`game_ticking`]).
use std::{
    cell::Cell,
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
    time::{Duration, Instant},
};
use windows_sys::Win32::System::Threading::{WaitOnAddress, WakeByAddressAll};

#[cfg(test)]
mod tests;

/// Far above any hourly tick, so only a wait the tick cannot end (a lock a hook needs) meets it.
const CAP: Duration = Duration::from_millis(200);

static GATE: Gate = Gate::new();

/// The game thread begins its hourly tick; the recorder's packing waits on `idle` too.
pub(crate) fn enter(idle: Option<&crate::runtime::Event>) {
    GATE.enter();
    if let Some(idle) = idle {
        idle.reset();
    }
}

/// The tick returned: release every waiting pipeline thread and the recorder's packing.
pub(crate) fn leave(idle: Option<&crate::runtime::Event>) {
    GATE.leave();
    if let Some(idle) = idle {
        idle.set();
    }
}

thread_local! {
    static COPY_ONLY: Cell<bool> = const { Cell::new(false) };
}

/// Marks the calling pipeline thread as working only on copies, so it works through the tick.
/// Its priority controller must be live: it is what keeps the thread off the game's way.
pub(crate) fn copy_only(copy_only: bool) {
    COPY_ONLY.set(copy_only);
}

/// Gates the calling thread like any live reader until the guard drops.
pub(crate) fn gated() -> Gated {
    Gated(COPY_ONLY.replace(false))
}

pub(crate) struct Gated(bool);

impl Drop for Gated {
    fn drop(&mut self) {
        COPY_ONLY.set(self.0);
    }
}

/// Waits while the game ticks, at most [`CAP`], unless this thread works only on copies;
/// never on the ticking thread itself.
pub(crate) fn wait() {
    if !COPY_ONLY.get() {
        GATE.wait(CAP);
    }
}

/// Whether this thread must leave work to another while the game ticks.
pub(crate) fn ticking() -> bool {
    !COPY_ONLY.get() && game_ticking()
}

/// Whether the game is in its hourly tick, whatever this thread works on.
pub(crate) fn game_ticking() -> bool {
    GATE.ticking.load(Ordering::SeqCst) != 0
}


struct Gate {
    ticking: AtomicU32,
    waits: AtomicU64,
    wait_ns: AtomicU64,
    capped: AtomicU64,
    #[cfg(test)]
    entered: AtomicU64,
}

impl Gate {
    const fn new() -> Self {
        Self {
            ticking: AtomicU32::new(0),
            waits: AtomicU64::new(0),
            wait_ns: AtomicU64::new(0),
            capped: AtomicU64::new(0),
            #[cfg(test)]
            entered: AtomicU64::new(0),
        }
    }

    fn enter(&self) {
        self.ticking.store(1, Ordering::SeqCst);
    }

    fn leave(&self) {
        self.ticking.store(0, Ordering::SeqCst);
        // SAFETY: an aligned u32 that outlives every waiter on it.
        unsafe { WakeByAddressAll(self.ticking.as_ptr().cast()) };
    }

    fn wait(&self, cap: Duration) {
        if self.ticking.load(Ordering::SeqCst) == 0 || collector_extended_support::hourly::ticking()
        {
            return;
        }
        #[cfg(test)]
        self.entered.fetch_add(1, Ordering::SeqCst);
        let started = Instant::now();
        let busy = 1_u32;
        while self.ticking.load(Ordering::SeqCst) == 1 {
            let Some(left) = cap.checked_sub(started.elapsed()).filter(|left| !left.is_zero())
            else {
                self.capped.fetch_add(1, Ordering::Relaxed);
                break;
            };
            // SAFETY: an aligned u32 compared with a local of the same size.
            unsafe {
                WaitOnAddress(
                    self.ticking.as_ptr().cast(),
                    (&busy as *const u32).cast(),
                    size_of::<u32>(),
                    left.as_millis().clamp(1, u128::from(u32::MAX)) as u32,
                )
            };
        }
        self.waits.fetch_add(1, Ordering::Relaxed);
        self.wait_ns.fetch_add(
            u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
    }

}
