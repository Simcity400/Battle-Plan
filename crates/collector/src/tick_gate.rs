//! Frame-pipeline threads wait out the game's hourly tick, whose speed their memory traffic
//! costs more than their own work does; they run between ticks and in the capture pause.
use serde_json::{Value, json};
use std::{
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

/// Waits while the game ticks, at most [`CAP`]; never on the ticking thread itself.
pub(crate) fn wait() {
    GATE.wait(CAP);
}

/// Whether the game is in its hourly tick, for work another thread can finish instead.
pub(crate) fn ticking() -> bool {
    GATE.ticking.load(Ordering::SeqCst) != 0
}

pub(crate) fn status() -> Value {
    GATE.status()
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

    fn status(&self) -> Value {
        json!({
            "waits": self.waits.load(Ordering::Relaxed),
            "wait_ns": self.wait_ns.load(Ordering::Relaxed),
            "capped": self.capped.load(Ordering::Relaxed),
            "scope": "cumulative waits of frame-pipeline threads (decode, spool, helpers, sizing) at phase and pack boundaries while the game ran its hourly tick; capped counts waits that reached the 200 ms bound",
        })
    }
}
