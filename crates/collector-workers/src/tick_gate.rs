use std::{
    cell::Cell,
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
    time::{Duration, Instant},
};
use windows_sys::Win32::System::Threading::{WaitOnAddress, WakeByAddressAll};

#[cfg(test)]
mod tests;

const CAP: Duration = Duration::from_millis(200);

static GATE: Gate = Gate::new();

/// # Safety
/// Only the admitted native tick may enter the live gate, using its session's idle event.
pub unsafe fn enter(idle: Option<&crate::event::Event>) {
    GATE.enter();
    if let Some(idle) = idle {
        // SAFETY: This worker is resetting the event it owns, or the isolated fixture owns the
        // panic witness.
        unsafe { idle.reset() };
    }
}

/// # Safety
/// Only the tick that entered this gate may leave it, after native work is complete and using the
/// same live idle event.
pub unsafe fn leave(idle: Option<&crate::event::Event>) {
    GATE.leave();
    if let Some(idle) = idle {
        // SAFETY: The tick exit owns the session's idle event and sets it after leaving the gate.
        unsafe { idle.set() };
    }
}

thread_local! {
    static COPY_ONLY: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn copy_only(copy_only: bool) {
    COPY_ONLY.set(copy_only);
}

pub fn gated() -> Gated {
    Gated(COPY_ONLY.replace(false), std::marker::PhantomData)
}

pub struct Gated(bool, std::marker::PhantomData<*mut ()>);

impl Drop for Gated {
    fn drop(&mut self) {
        COPY_ONLY.set(self.0);
    }
}

pub fn wait() {
    if !COPY_ONLY.get() {
        GATE.wait(CAP);
    }
}

pub fn ticking() -> bool {
    !COPY_ONLY.get() && game_ticking()
}

pub fn game_ticking() -> bool {
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
            let Some(left) = cap
                .checked_sub(started.elapsed())
                .filter(|left| !left.is_zero())
            else {
                self.capped.fetch_add(1, Ordering::Relaxed);
                break;
            };
            // SAFETY: an aligned u32 compared with a local of the same size, live through the wait.
            unsafe {
                WaitOnAddress(
                    self.ticking.as_ptr().cast(),
                    (&raw const busy).cast(),
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
