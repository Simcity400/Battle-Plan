use collector_hooks::hook::Hook;
use engine_native_memory::Memory;
use engine_native::cgame_state;
use std::{
    cell::Cell,
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
};

static ENABLED: AtomicBool = AtomicBool::new(false);
static OBSERVED: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

pub fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static BATCHING: AtomicUsize = AtomicUsize::new(0);
static HELD: AtomicU32 = AtomicU32::new(0);
static PROOF: AtomicU32 = AtomicU32::new(UNPUBLISHED);

const UNPUBLISHED: u32 = 0;
const PROVEN: u32 = 1;
const EXHAUSTED: u32 = 2;
const NO_RANGE: u32 = 3;
const ATTEMPTS: u32 = 200;

thread_local! {
    static DEPTH: Cell<usize> = const { Cell::new(0) };
}

pub struct Hooks {
    hooks: Vec<Hook>,
    range: Option<Range<usize>>,
    held: u32,
}

impl Hooks {
    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let (at, original) = collector_hooks::located_hook(base, cgame_state::COMMAND_BATCH_RVA)?;
        let mut hook = unsafe { Hook::new(at, batch as *const () as usize, original) }
            .map_err(|_| "native hook construction failed")?;
        ORIGINAL.store(hook.trampoline(), Ordering::Release);
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

    /// # Safety
    /// Only the owning runtime may publish or restore these taps, in the corresponding attach or
    /// detach round. Keep the taps alive until the round finishes and callbacks are quiet.
    pub unsafe fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
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
        OBSERVED.store(proof == PROVEN, Ordering::SeqCst);
        Ok(true)
    }

    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

pub fn observed() -> bool {
    OBSERVED.load(Ordering::SeqCst)
}

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

pub(crate) fn publications_held() -> u32 {
    HELD.load(Ordering::Relaxed)
}

pub(crate) fn others_batching() -> bool {
    BATCHING.load(Ordering::SeqCst) > usize::from(DEPTH.get() > 0)
}

unsafe extern "system" fn batch(session: usize) -> usize {
    let depth = DEPTH.get();
    if depth == 0 {
        enter();
    }
    let timing =
        collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::CommandBatch);
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

fn enter() {
    loop {
        BATCHING.fetch_add(1, Ordering::SeqCst);
        if !collector_timing::capture_gate::running() {
            return;
        }
        BATCHING.fetch_sub(1, Ordering::SeqCst);
        collector_timing::capture_gate::hold(
            collector_timing::hook_timing::Family::CommandBatch as usize,
        );
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use collector_timing::capture_gate;

    pub fn observe(observed: bool) {
        OBSERVED.store(observed, Ordering::SeqCst);
    }

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
        // SAFETY: Serialized test owns the capture gate and pairs begin() with end().
        assert!(unsafe { capture_gate::begin() });
        let entered = std::thread::spawn(|| unsafe { batch(7) });
        while capture_gate::waiting() == 0 {
            std::thread::yield_now();
        }
        assert!(!others_batching());
        assert!(!RAN.load(Ordering::SeqCst));
        // SAFETY: Pairs with the begin() above in this serialized test.
        unsafe { capture_gate::end() };
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
        // SAFETY: Serialized test owns the capture gate and pairs begin() with end().
        assert!(unsafe { capture_gate::begin() });
        assert!(others_batching());
        // SAFETY: Pairs with the begin() above in this serialized test.
        unsafe { capture_gate::end() };
        RELEASE.store(true, Ordering::SeqCst);
        assert_eq!(inside.join().unwrap(), 2);
        assert!(!others_batching());
    }
}
