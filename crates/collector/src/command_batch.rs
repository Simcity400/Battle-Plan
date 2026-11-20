//! The game's per-frame command batch: it runs every queued command, the hourly update among
//! them, and is the only place the game writes its simulation. A thread entering its outermost
//! batch counts itself, then passes the capture gate; every capture closes the gate, then requires
//! that no other thread is counted ([`crate::capture_point`]). The hook is published while no
//! thread may be inside the batch, where that can be shown within its attempts: every batch since
//! is then observed, and a batch's return with no observed batch around it on its thread tells the
//! capture point nothing writes until the next. Published without that proof, its returns are not
//! trusted and an hour is read straight after its update.
use crate::hook::Hook;
use engine_native::{Memory, cgame_state};
use std::{
    cell::Cell,
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
};

/// Every batch since the hook's publication is observed: captures may run beside the game.
static ENABLED: AtomicBool = AtomicBool::new(false);
/// The hook is published and every batch is observed: an hour can wait for its batch's return.
static OBSERVED: AtomicBool = AtomicBool::new(false);
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// Threads inside an observed batch; one waiting at the capture gate writes nothing and is not.
static BATCHING: AtomicUsize = AtomicUsize::new(0);
/// Publications held because a thread may have been inside the batch.
static HELD: AtomicU32 = AtomicU32::new(0);
static PROOF: AtomicU32 = AtomicU32::new(UNPUBLISHED);

const UNPUBLISHED: u32 = 0;
const PROVEN: u32 = 1;
const EXHAUSTED: u32 = 2;
const NO_RANGE: u32 = 3;
/// Held publications before the hook is published without the proof. The game leaves the batch
/// every frame and the runtime retries about every millisecond, so a few attempts normally do.
const ATTEMPTS: u32 = 200;

thread_local! {
    /// Observed batches this thread entered and has not left.
    static DEPTH: Cell<usize> = const { Cell::new(0) };
}

pub struct Hooks {
    hooks: Vec<Hook>,
    range: Option<Range<usize>>,
    held: u32,
}

impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let (at, original) = crate::located_hook(base, cgame_state::COMMAND_BATCH_RVA)?;
        let mut hook = unsafe { Hook::new(at, batch as *const () as usize, original) }
            .map_err(|_| "native hook construction failed")?;
        ORIGINAL.store(hook.trampoline(), Ordering::Release);
        // Located only where the whole function matches the reference: one block with one return
        // and no jump out, so every return its own calls leave lies in this range, and only there.
        let range = engine_sites::hook(base, cgame_state::COMMAND_BATCH_RVA)
            .ok()
            .map(|code| code.function..code.end)
            .filter(|range| range.contains(&at));
        hook.guard(range.clone());
        Ok(Self {
            hooks: vec![hook],
            range,
            held: 0,
        })
    }

    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
            OBSERVED.store(false, Ordering::SeqCst);
        }
        let published = Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?;
        let hook = &mut self.hooks[0];
        if !published {
            if enabled && hook.guard_held() {
                HELD.fetch_add(1, Ordering::Relaxed);
                self.held += 1;
                if self.held >= ATTEMPTS {
                    hook.guard(None);
                }
            }
            return Ok(false);
        }
        let proof = match (enabled, hook.guarded(), &self.range) {
            (false, ..) => UNPUBLISHED,
            (true, true, _) => PROVEN,
            (true, false, Some(_)) => EXHAUSTED,
            (true, false, None) => NO_RANGE,
        };
        if !enabled {
            hook.guard(self.range.clone());
            self.held = 0;
        }
        PROOF.store(proof, Ordering::Release);
        ENABLED.store(proof == PROVEN, Ordering::SeqCst);
        // An unproven publication may sit inside a running batch: its returns are not trusted.
        OBSERVED.store(proof == PROVEN, Ordering::SeqCst);
        Ok(true)
    }

    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}

/// Whether every batch since the hook's publication is observed, so captures may run beside the
/// game.
pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

/// Whether every batch is observed, so an hour can wait for its batch's return.
pub(crate) fn observed() -> bool {
    OBSERVED.load(Ordering::SeqCst)
}

/// Whether the hook was published with no thread inside the batch, and if not why, for status.
pub(crate) fn publication_status() -> &'static str {
    match PROOF.load(Ordering::Acquire) {
        PROVEN => {
            "proven: no thread was inside the command batch when its hook was published, so every batch is observed"
        }
        EXHAUSTED => {
            "unproven: a thread may have been inside the command batch at every publication attempt allowed, so the hook was published without the proof and every capture holds the game straight after its update"
        }
        NO_RANGE => {
            "unprovable: the batch function's verified code range could not be obtained, so every capture holds the game straight after its update"
        }
        _ => "not published",
    }
}

/// Publications held because a thread may have been inside the batch.
pub(crate) fn publications_held() -> u32 {
    HELD.load(Ordering::Relaxed)
}

/// Whether a thread other than this one is inside an observed batch.
pub(crate) fn others_batching() -> bool {
    BATCHING.load(Ordering::SeqCst) > usize::from(DEPTH.get() > 0)
}

unsafe extern "system" fn batch(session: usize) -> usize {
    let depth = DEPTH.get();
    if depth == 0 {
        enter();
    }
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::CommandBatch);
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    DEPTH.set(depth + 1);
    let original: unsafe extern "system" fn(usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL.load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(session) });
    DEPTH.set(depth);
    if depth == 0 {
        BATCHING.fetch_sub(1, Ordering::SeqCst);
        crate::capture_point::batch_left();
    }
    ACTIVE.fetch_sub(1, Ordering::SeqCst);
    drop(timing);
    result
}

/// Counted, then the gate; a capture closes the gate, then looks at the count: one of the two
/// sees the other.
fn enter() {
    loop {
        BATCHING.fetch_add(1, Ordering::SeqCst);
        if !collector_timing::capture_gate::running() {
            return;
        }
        BATCHING.fetch_sub(1, Ordering::SeqCst);
        collector_timing::capture_gate::hold(crate::hook_timing::Family::CommandBatch as usize);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use collector_timing::capture_gate;

    pub(crate) fn observe(observed: bool) {
        OBSERVED.store(observed, Ordering::SeqCst);
    }

    /// Another thread is inside an observed batch, or leaves it.
    pub(crate) fn batching_elsewhere(inside: bool) {
        if inside {
            BATCHING.fetch_add(1, Ordering::SeqCst);
        } else {
            BATCHING.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn a_batch_entered_during_a_capture_waits_at_the_gate_uncounted() {
        static RAN: AtomicBool = AtomicBool::new(false);
        extern "system" fn native(session: usize) -> usize {
            RAN.store(true, Ordering::SeqCst);
            session
        }
        let _serial = crate::capture_point::tests::serial();
        ORIGINAL.store(native as *const () as usize, Ordering::SeqCst);
        assert!(capture_gate::begin());
        let entered = std::thread::spawn(|| unsafe { batch(7) });
        while capture_gate::waiting() == 0 {
            std::thread::yield_now();
        }
        assert!(!others_batching());
        assert!(!RAN.load(Ordering::SeqCst));
        capture_gate::end();
        assert_eq!(entered.join().unwrap(), 7);
        assert!(RAN.load(Ordering::SeqCst));
        assert!(!others_batching());
    }

    #[test]
    fn a_capture_closing_the_gate_sees_a_batch_another_thread_is_inside_but_not_its_own() {
        static INSIDE: AtomicBool = AtomicBool::new(false);
        static RELEASE: AtomicBool = AtomicBool::new(false);
        static OTHERS: AtomicBool = AtomicBool::new(true);
        extern "system" fn native(session: usize) -> usize {
            // A capture inside its own thread's batches is not refused for them.
            if session == 1 {
                return unsafe { batch(2) };
            }
            OTHERS.store(others_batching(), Ordering::SeqCst);
            INSIDE.store(true, Ordering::SeqCst);
            while !RELEASE.load(Ordering::SeqCst) {
                std::thread::yield_now();
            }
            session
        }
        let _serial = crate::capture_point::tests::serial();
        ORIGINAL.store(native as *const () as usize, Ordering::SeqCst);
        let inside = std::thread::spawn(|| unsafe { batch(1) });
        while !INSIDE.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        assert!(!OTHERS.load(Ordering::SeqCst));
        assert!(capture_gate::begin());
        assert!(others_batching());
        capture_gate::end();
        RELEASE.store(true, Ordering::SeqCst);
        assert_eq!(inside.join().unwrap(), 2);
        assert!(!others_batching());
    }
}
