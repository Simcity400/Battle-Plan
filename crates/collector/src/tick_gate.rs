//! Frame-pipeline threads wait out the game's hourly tick, whose speed their memory traffic
//! costs more than their own work does; they run between ticks and in the capture pause.
//! A tick that begins with no free frame is not waited out: the next hour would otherwise
//! stop the game until the pipeline returns one. Live game-memory reads on a pipeline thread
//! ([`live_reads`], [`wait_live`]) still wait out every tick.
use serde_json::{Value, json};
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
/// `behind`: no frame is free for the next hour, so the pipeline keeps working through it.
pub(crate) fn enter(idle: Option<&crate::runtime::Event>, behind: bool) {
    GATE.enter(behind);
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
    static LIVE: Cell<bool> = const { Cell::new(false) };
}

/// Waits while the game ticks, at most [`CAP`]; never on the ticking thread itself. An open
/// tick is worked through, unless this thread is inside [`live_reads`].
pub(crate) fn wait() {
    GATE.wait(CAP, LIVE.with(Cell::get));
}

/// Before a live game-memory read on a pipeline thread: waits out any tick, open or not.
pub(crate) fn wait_live() {
    GATE.wait(CAP, true);
}

/// Marks this thread as reading live game memory until the guard drops: its waits
/// ([`wait`], phase spans) then wait out open ticks too.
pub(crate) fn live_reads() -> LiveReads {
    LiveReads(LIVE.with(|live| live.replace(true)))
}

pub(crate) struct LiveReads(bool);

impl Drop for LiveReads {
    fn drop(&mut self) {
        LIVE.with(|live| live.set(self.0));
    }
}

/// Whether the game is in a gated hourly tick, for work another thread can finish instead.
pub(crate) fn ticking() -> bool {
    GATE.ticking.load(Ordering::SeqCst) == GATED
}

pub(crate) fn status() -> Value {
    GATE.status()
}

/// `Gate::ticking`: no tick, a tick pipeline threads wait out, a tick they work through.
const IDLE: u32 = 0;
const GATED: u32 = 1;
const OPEN: u32 = 2;

struct Gate {
    ticking: AtomicU32,
    waits: AtomicU64,
    wait_ns: AtomicU64,
    capped: AtomicU64,
    open_ticks: AtomicU64,
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
            open_ticks: AtomicU64::new(0),
            #[cfg(test)]
            entered: AtomicU64::new(0),
        }
    }

    fn enter(&self, behind: bool) {
        if behind {
            self.open_ticks.fetch_add(1, Ordering::Relaxed);
        }
        self.ticking
            .store(if behind { OPEN } else { GATED }, Ordering::SeqCst);
    }

    fn leave(&self) {
        self.ticking.store(IDLE, Ordering::SeqCst);
        // SAFETY: an aligned u32 that outlives every waiter on it.
        unsafe { WakeByAddressAll(self.ticking.as_ptr().cast()) };
    }

    /// `live`: also wait out an open tick.
    fn wait(&self, cap: Duration, live: bool) {
        let blocks = |value: u32| value == GATED || (live && value == OPEN);
        let mut value = self.ticking.load(Ordering::SeqCst);
        if !blocks(value) || collector_extended_support::hourly::ticking() {
            return;
        }
        #[cfg(test)]
        self.entered.fetch_add(1, Ordering::SeqCst);
        let started = Instant::now();
        while blocks(value) {
            let Some(left) = cap.checked_sub(started.elapsed()).filter(|left| !left.is_zero())
            else {
                self.capped.fetch_add(1, Ordering::Relaxed);
                break;
            };
            // SAFETY: an aligned u32 compared with a local of the same size.
            unsafe {
                WaitOnAddress(
                    self.ticking.as_ptr().cast(),
                    (&value as *const u32).cast(),
                    size_of::<u32>(),
                    left.as_millis().clamp(1, u128::from(u32::MAX)) as u32,
                )
            };
            value = self.ticking.load(Ordering::SeqCst);
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
            "open_ticks": self.open_ticks.load(Ordering::Relaxed),
            "scope": "cumulative waits of frame-pipeline threads (decode, spool, helpers, sizing) at phase and pack boundaries while the game ran its hourly tick; capped counts waits that reached the 200 ms bound; open_ticks counts ticks that began with no free frame, which pipeline threads worked through",
        })
    }
}
