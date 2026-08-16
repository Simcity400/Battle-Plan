//! THE FAILURE PATHS, DRIVEN.
//!
//! # Why this module exists
//!
//! Three lanes closed defects in this crate and its MinHook fork, and every one
//! of them is on a path the OS or the library takes only when something has
//! gone wrong:
//!
//! * KQ: `SuspendThread` refuses a live thread, or `GetThreadContext` cannot
//!   answer. Upstream MinHook returned silently from both and patched the bytes
//!   anyway, resuming a thread into a half-rewritten instruction stream.
//! * KZ / LI: a teardown frees a trampoline while a thread is still inside the
//!   caller's own detour body -- which the freeze cannot see, because that body
//!   is neither the target nor the trampoline.
//! * LI: "the enable reported failure, so nothing was patched" is FALSE in this
//!   product. `EnableHookLL` writes the JMP, sets `isEnabled`, and only then
//!   reports that the protection restore or the instruction-cache flush did not
//!   complete. Six markers in this workspace once rested on the opposite
//!   reading.
//!
//! LI's own closing note named the gap this module fills: a clean attach walks
//! past all of it, so "45/45 installed and torn down" says nothing about any of
//! these. What proves them is a FAULT-INJECTED one -- refuse an enable after
//! the bytes are written, hold a callback in flight across a teardown, refuse a
//! suspension and a context read -- and read the receipts.
//!
//! # What the seams are, and why they cannot ship
//!
//! Three, and no more:
//!
//! * [`crate::freeze::faults`], `cfg(test)` -- refuse one thread's suspension,
//!   or one thread's context read. `cfg(test)` is set only while THIS crate's
//!   test harness is compiled, so a collector that depends on `inline_detour`
//!   compiles constant-`false` arms.
//! * `minhook_h4`'s `test_support` feature -- make one target's enable report
//!   the completion failure with its bytes already written. It is a feature
//!   rather than `cfg(test)` because the C is compiled as a DEPENDENCY of this
//!   crate, and a second compilation of it would be a second `g_hooks` table
//!   that production never calls. Its absence from shipped builds is PROVED
//!   rather than argued, by the two checks in [`inertness`].
//!
//! # The hazard these tests carry, and what is done about it
//!
//! Allocating inside a held freeze can hang this process for ever -- that is
//! the defect KZ's audit exists to find, and a test that performs it
//! deliberately performs it deliberately. The one test that must
//! ([`the_probe_sees_an_allocation_inside_a_held_window`]) runs in a CHILD
//! PROCESS with one test thread and a parent-side timeout, so a hang fails a
//! test instead of wedging the battery for everyone.

use super::*;
use crate::freeze::faults;
use core::ffi::c_void;
use minhook::{MinHook, MH_STATUS};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

/// Serialises this module against itself. Taken FIRST, then the ledger, then
/// the suspend-state guard -- the order `tests` and `batch_tests` use, so no
/// two of the three modules can take the pair in opposite orders.
static FAULT_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The three process-global guards every test here holds, in the one order.
///
/// The suspend-state guard is what actually excludes `tests` and `batch_tests`:
/// this module arms process-global faults and takes process-global freezes, and
/// either one landing inside another module's delta window would corrupt that
/// module's measurement rather than this one's.
macro_rules! serialised {
    () => {
        let _fault = FAULT_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _ledger = crate::release_ledger::TEST_LEDGER_SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _suspend = crate::freeze::os_suspend_state_guard();
    };
}

/// Spin until `ready` answers true, or FAIL rather than spin for ever.
///
/// Every wait in this module is on a condition a REGRESSION can make
/// unreachable -- "the callback entered the detour" stops happening the moment
/// physical patching regresses while `enable` still reports success. A bare
/// loop turns that regression into a hung test binary, which wedges the battery
/// for every lane; a deadline turns it into a named failure.
fn wait_until(what: &str, ready: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !ready() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}: a test that waits for ever on a              regression hangs the battery instead of reporting it"
        );
        std::thread::yield_now();
    }
}

/// Physically restores a set of targets when it drops, WHATEVER happened.
///
/// Every hook here is a patch into this test binary's own code and a live
/// entry in the process-global MinHook table, so a test that panicked between
/// its enable and its cleanup would hand the next test a detoured function, a
/// dirty queue or a retained trampoline. The trailing cleanup call the first
/// spelling used does not run during an unwind; this does.
struct HookCustody(Vec<*mut c_void>);

impl HookCustody {
    fn of(targets: &[(&'static str, *mut c_void)]) -> Self {
        Self(targets.iter().map(|&(_, target)| target).collect())
    }

    fn one(target: *mut c_void) -> Self {
        Self(vec![target])
    }
}

impl Drop for HookCustody {
    fn drop(&mut self) {
        // THE FAULT FIRST. An arming still live would be consumed by one of
        // these disables, and the next test would find its own fault already
        // spent -- a vacuous pass, which is worse than a failure.
        minhook::EnableIncoherentFault::disarm();
        for &target in &self.0 {
            let _ = unsafe { disable(target) };
            let _ = unsafe { remove(target) };
        }
    }
}

// ---------------------------------------------------------------------------
// Victims. Distinct bodies, distinct constants: `#[inline(never)]` alone does
// not stop the linker folding two identical functions into one address, and
// two hooks on one address is a batch of one wearing a disguise.
// ---------------------------------------------------------------------------

type Victim = extern "system" fn(u64) -> u64;

static DETOURED: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

#[inline(never)]
extern "system" fn fault_victim_0(x: u64) -> u64 {
    x.wrapping_mul(13).wrapping_add(0xA000)
}
#[inline(never)]
extern "system" fn fault_victim_1(x: u64) -> u64 {
    x.wrapping_mul(17).wrapping_add(0xB000)
}
#[inline(never)]
extern "system" fn fault_victim_2(x: u64) -> u64 {
    x.wrapping_mul(19).wrapping_add(0xC000)
}

#[inline(never)]
extern "system" fn fault_detour_0(_x: u64) -> u64 {
    DETOURED[0].fetch_add(1, Ordering::AcqRel);
    0xFA0
}
#[inline(never)]
extern "system" fn fault_detour_1(_x: u64) -> u64 {
    DETOURED[1].fetch_add(1, Ordering::AcqRel);
    0xFA1
}
#[inline(never)]
extern "system" fn fault_detour_2(_x: u64) -> u64 {
    DETOURED[2].fetch_add(1, Ordering::AcqRel);
    0xFA2
}

fn trio() -> [(&'static str, Victim, Victim); 3] {
    [
        ("fault.victim.0", fault_victim_0, fault_detour_0),
        ("fault.victim.1", fault_victim_1, fault_detour_1),
        ("fault.victim.2", fault_victim_2, fault_detour_2),
    ]
}

/// The value each victim answers when it is NOT detoured. Computed by calling
/// it, not by copying the arithmetic: a copy would go on agreeing with itself
/// after the body changed.
fn undetoured_answer(index: usize) -> u64 {
    // Called before anything is created, so this is the original.
    trio()[index].1(1)
}

/// Create all three hooks, disabled, removing any prior instance first.
fn create_trio() -> Vec<(&'static str, *mut c_void)> {
    let mut targets = Vec::new();
    for (name, victim, detour) in trio() {
        let target = victim as *mut c_void;
        let _ = unsafe { disable(target) };
        let _ = unsafe { remove(target) };
        unsafe { MinHook::create_hook(target, detour as *mut c_void) }
            .unwrap_or_else(|status| panic!("{name} must be hookable: {status}"));
        targets.push((name, target));
    }
    targets
}

/// The first bytes an enable of `target` rewrites, read through MinHook's own
/// answer about the footprint rather than a guess.
///
/// A hard-coded "first five bytes" is wrong for a hot-patch target, whose patch
/// starts five bytes ABOVE the function and is seven bytes long. Asking is also
/// what makes the comparison mean "the bytes an enable would have written",
/// which is the claim.
fn patch_bytes(target: *mut c_void) -> Vec<u8> {
    let lock = minhook::MinHookLock::acquire().expect("the lock is free");
    let footprint =
        MinHook::patch_footprint(&lock, target).expect("the hook exists, so it has a footprint");
    drop(lock);
    let (start, end) = footprint.patch;
    // SAFETY: MinHook answered with the exact range an enable rewrites, which
    // is mapped, readable code in this process.
    unsafe { std::slice::from_raw_parts(start as *const u8, end - start) }.to_vec()
}

// ---------------------------------------------------------------------------
// (a) An enable refused AFTER the bytes were written.
// ---------------------------------------------------------------------------

/// LI'S FACT, DRIVEN: a batch whose apply was refused can report every target
/// `Ok`, and the targets are LIVE.
///
/// The fault makes one target's `EnableHookLL` complete its write, set
/// `isEnabled`, and then report `MH_ERROR_MEMORY_PROTECT` as a failed
/// instruction-cache flush would. That is the fork's documented behaviour and
/// until this seam existed nothing could reach it.
///
/// What the batch then reports is the finding, not just the fixture:
///
/// * `apply_refusal()` carries the refusal -- and is the ONLY thing that does;
/// * `failures()` is EMPTY and `first_failure()` is `None`, because the
///   completion pass re-enables each queued target and MinHook answers
///   `MH_ERROR_ENABLED` for the one already patched, which maps to `Ok`;
/// * `rolled_back()` is false, so nothing was taken back down;
/// * all three victims answer with their DETOUR, including the refused one.
///
/// Every production `enable_batch` caller in this workspace gates on
/// `first_failure()`, `failures()` or the per-target `outcomes()`. None reads
/// `apply_refusal()`. This test pins that reachable shape so the claim is a
/// measured one rather than an argument.
///
/// And the consequence is asserted too, because it is what a teardown meets: an
/// incoherent hook cannot be REMOVED (`MH_RemoveHook` refuses it by name), so
/// the trampoline is retained -- until a disable completes and clears the latch,
/// which is the recovery the product requires of every refusal.
///
/// kills: the reading that an enable-failure arm is an untouched-target arm.
/// A `minhook_h4` that reported `MH_OK` when the restore or the flush failed --
/// which is what upstream does -- would make `apply_refusal()` `None` here and
/// this test would fail on its first assertion.
#[test]
fn an_apply_refused_after_the_bytes_were_written_reports_every_target_ok() {
    serialised!();
    let originals: Vec<u64> = (0..3).map(undetoured_answer).collect();
    let targets = create_trio();
    let custody = HookCustody::of(&targets);
    let faulted = targets[1].1;

    let enabled_before = enabled_detour_count();
    let armed = minhook::EnableIncoherentFault::arm(faulted);
    let report = unsafe { enable_batch(&targets) };

    // THE FAULT WAS CONSUMED, and the BRANCH ITSELF RAN. Without both the whole
    // test is vacuous: an arming that never fired would leave a perfectly
    // ordinary batch and every assertion below would be about nothing. The
    // witness is written from inside the injected branch in `EnableHookLL`, so
    // a cleared arming that somehow did not run cannot satisfy it.
    assert!(
        minhook::EnableIncoherentFault::armed().is_none(),
        "the injected completion failure was never reached"
    );
    assert_eq!(
        minhook::EnableIncoherentFault::witness(),
        Some("MH_H4_FAULT_INJECTION_PRESENT"),
        "the injected branch inside EnableHookLL must have executed"
    );
    drop(armed);

    // The batch-level refusal is carried, and it is the only carrier.
    assert_eq!(
        report.apply_refusal().and_then(|e| e.minhook_status()),
        Some(MH_STATUS::MH_ERROR_MEMORY_PROTECT),
        "the apply's own refusal must survive into the report: {report:?}"
    );
    assert!(
        !report.all_succeeded(),
        "a batch whose apply was refused has not all succeeded"
    );
    assert!(
        report.first_failure().is_none() && report.failures().is_empty(),
        "THE FINDING: the completion pass heals every per-target outcome, so \
         the gate every production caller uses sees nothing wrong: {:?}",
        report.outcomes()
    );
    assert!(
        !report.rolled_back(),
        "nothing was rolled back, so the Ok outcomes describe LIVE detours"
    );

    // PHYSICAL, and this is what makes the paragraph above a fact about memory
    // rather than about a struct: every victim, including the refused one,
    // answers with its detour.
    for (index, (name, _)) in targets.iter().enumerate() {
        let answered = trio()[index].1(1);
        assert_ne!(
            answered, originals[index],
            "{name} must be detoured after this batch"
        );
        assert_eq!(answered, [0xFA0, 0xFA1, 0xFA2][index], "{name}");
    }
    assert_eq!(
        enabled_detour_count() - enabled_before,
        3,
        "all three were counted as enabled, which is correct -- they are"
    );

    // THE LATCH, ASKED OF MINHOOK DIRECTLY, and it has to be: `remove` refuses
    // EVERY enabled target before it reaches MinHook at all
    // (`InlineDetourReleaseError::StillEnabled`), so driving the latch through
    // the wrapper would assert nothing about `incoherent` and would stay green
    // with the whole latch deleted.
    //
    // `MH_RemoveHook` refuses an incoherent entry by name rather than freeing a
    // trampoline whose last write could not be completed. Asked inside a
    // freeze, which is what the call requires.
    let frozen = crate::freeze_process().expect("the process freezes");
    let asked = unsafe { MinHook::remove_hook(frozen.minhook_lock(), faulted) };
    drop(frozen);
    assert_eq!(
        asked,
        Err(MH_STATUS::MH_ERROR_MEMORY_PROTECT),
        "an incoherent entry must refuse removal by name: deleting the latch          makes this an Ok and frees a trampoline whose write never completed"
    );

    // And the wrapper's own, separate refusal: an ENABLED target is retained
    // whatever MinHook would have said.
    let retained_before = retained_detour_count();
    let refused = unsafe { remove(faulted) }.expect_err("an enabled target refuses removal");
    assert!(
        matches!(refused, InlineDetourReleaseError::StillEnabled),
        "{refused:?}"
    );
    assert_eq!(
        retained_detour_count() - retained_before,
        1,
        "a refused removal is counted as a retained detour"
    );

    // THE RECOVERY PROPERTY: a completed disable RE-ATTEMPTS the completion and
    // clears the latch, and only then is the trampoline freed. A product whose
    // refusals were terminal would fail here, and so would one whose disable
    // did not re-derive `incoherent`.
    unsafe { disable(faulted) }.expect("the disable completes and clears the incoherent latch");
    let frozen = crate::freeze_process().expect("the process freezes");
    let asked = unsafe { MinHook::remove_hook(frozen.minhook_lock(), faulted) };
    drop(frozen);
    assert_eq!(asked, Ok(()), "a cleared latch removes");
    assert_eq!(
        trio()[1].1(1),
        originals[1],
        "the victim is itself again once its detour is removed"
    );

    drop(custody);
    for (index, original) in originals.iter().enumerate() {
        assert_eq!(trio()[index].1(1), *original, "cleanup left a detour live");
    }
}

// ---------------------------------------------------------------------------
// (b) A callback in flight across a teardown.
// ---------------------------------------------------------------------------

static GATE_OPEN: AtomicBool = AtomicBool::new(false);
static IN_DETOUR: AtomicBool = AtomicBool::new(false);
/// Set only after the body has RETURNED FROM THE TRAMPOLINE, so it is the one
/// signal that says freeing the trampoline is now safe.
static LEFT_DETOUR: AtomicBool = AtomicBool::new(false);
static GATED_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

#[inline(never)]
extern "system" fn gated_victim(x: u64) -> u64 {
    x.wrapping_mul(23).wrapping_add(0xE000)
}

/// A detour body that PARKS, which is the whole point: while it is parked, this
/// thread is inside neither the target nor the trampoline, so the freeze's
/// instruction-pointer check cannot see it. That is not a gap in the check --
/// the body is the caller's own function at an address this crate is never
/// told -- and it is exactly why `remove`'s contract is the caller's.
#[inline(never)]
extern "system" fn gated_detour(x: u64) -> u64 {
    IN_DETOUR.store(true, Ordering::Release);
    while !GATE_OPEN.load(Ordering::Acquire) {
        // A bare spin, deliberately: this thread is about to be SUSPENDED by a
        // freeze, and being suspended inside a syscall is a wider claim than
        // this test needs to make.
        std::hint::spin_loop();
    }
    let original = GATED_ORIGINAL.load(Ordering::Acquire);
    assert_ne!(original, 0, "the trampoline address must be published first");
    // SAFETY: the trampoline MinHook returned for this target. If a teardown
    // had freed it, this call is what would fault -- which is the property.
    let original: Victim = unsafe { std::mem::transmute(original) };
    let answered = original(x).wrapping_add(1);
    // AFTER the trampoline has returned. Nothing below this line touches the
    // trampoline, so this is the instant at which a removal stops being a
    // use-after-free -- and it is what the custody guard waits for.
    LEFT_DETOUR.store(true, Ordering::Release);
    answered
}

/// Releases the parked callback, WAITS FOR IT TO LEAVE, and only then restores
/// the target -- in that order, including during an unwind.
///
/// The order is the whole point, and the first spelling had it wrong. Two
/// separate guards (one opening the gate, one removing the hook) drop in
/// declaration-reverse order, so the gate opened and the removal followed
/// IMMEDIATELY -- with the released thread somewhere between the gate and its
/// call through the trampoline. The freeze cannot see it there, so the removal
/// would have been allowed and would have freed the trampoline it was about to
/// call: the exact use-after-free this test exists to talk about, performed by
/// the test's own cleanup.
///
/// So there is one guard, and it drains before it frees. If the drain times
/// out it frees NOTHING and says so, because a leaked hook is recoverable and a
/// freed trampoline under a live thread is not.
struct ParkedCallbackCustody(*mut c_void);

impl Drop for ParkedCallbackCustody {
    fn drop(&mut self) {
        GATE_OPEN.store(true, Ordering::Release);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !LEFT_DETOUR.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        // DISABLE EITHER WAY, and free only when it has left. Restoring the
        // target's bytes frees nothing and strands nobody -- a thread inside
        // the body returns through the trampoline, which a disable leaves
        // alone -- so it is safe even on the timeout path, and it is exactly
        // what the production drain does before it decides whether to free.
        let _ = unsafe { disable(self.0) };
        if !LEFT_DETOUR.load(Ordering::Acquire) {
            // NOTHING IS FREED. Reported rather than panicked: a panic here
            // during an unwind aborts the process and destroys the report of
            // whatever failed first. The process is this test's own child, so
            // what it leaks dies with it.
            eprintln!(
                "fault_tests: the parked callback never left the detour body; the \
                 hook on {:p} is deliberately RETAINED rather than freed under it",
                self.0
            );
            return;
        }
        let _ = unsafe { remove(self.0) };
    }
}

/// A THREAD INSIDE THE DETOUR BODY IS INVISIBLE TO THE FREEZE, and `remove`'s
/// `StillEnabled` refusal is what stands between that and a freed trampoline.
///
/// The parked thread is held inside `gated_detour` -- not in the target, not in
/// the trampoline -- so the freeze holds it, reads its instruction pointer,
/// finds it in neither range, and has no objection. The refusal that matters is
/// the other one.
///
/// Three things are asserted, and the third is what stops the test being
/// satisfied by a `remove` that refuses everything:
///
/// 1. `remove` on the enabled target answers `StillEnabled`, counts a retained
///    detour and records the attempt in the ledger;
/// 2. the parked thread, released, returns THROUGH the trampoline and produces
///    the ORIGINAL function's answer -- so nothing was freed under it;
/// 3. after a `disable` and with nothing in flight, the same target removes
///    cleanly. A `remove` hard-wired to refuse would pass 1 and 2 and fail here.
///
/// kills: KZ's `StillEnabled` refusal being deleted or softened to a
/// `debug_assert!` (which checks nothing in the build that runs inside a
/// player's game); and any belief that the freeze's instruction-pointer check
/// covers a caller's detour body.
/// CONTAINED. See [`run_in_child`]: this test's own failure mode leaves a
/// thread parked or suspended and a detour live, so it runs in a process of its
/// own and the claims live on the child below.
#[test]
fn a_callback_parked_in_a_detour_body_is_not_freed_underneath() {
    run_in_child("fault_tests::child_a_callback_parked_in_a_detour_body_is_not_freed_underneath");
}

#[test]
#[ignore = "contained: driven by a_callback_parked_in_a_detour_body_is_not_freed_underneath"]
fn child_a_callback_parked_in_a_detour_body_is_not_freed_underneath() {
    assert!(
        std::env::var(CHILD_MARKER).is_ok(),
        "this test is only sound in the child process its parent spawns"
    );
    serialised!();
    let target = gated_victim as *mut c_void;
    let original_answer = gated_victim(7);
    let _ = unsafe { disable(target) };
    let _ = unsafe { remove(target) };
    GATE_OPEN.store(false, Ordering::Release);
    IN_DETOUR.store(false, Ordering::Release);
    LEFT_DETOUR.store(false, Ordering::Release);
    let trampoline = unsafe { MinHook::create_hook(target, gated_detour as *mut c_void) }
        .expect("the gated victim is hookable");
    GATED_ORIGINAL.store(trampoline as usize, Ordering::Release);
    unsafe { enable(target) }.expect("the gated detour goes live");

    let custody = ParkedCallbackCustody(target);
    let parked = std::thread::spawn(|| gated_victim(7));
    wait_until("the callback to enter the detour body", || {
        IN_DETOUR.load(Ordering::Acquire)
    });

    // 1. The refusal, with a thread demonstrably inside the body.
    let retained_before = retained_detour_count();
    let refused = unsafe { remove(target) }.expect_err("an enabled target refuses removal");
    assert!(
        matches!(refused, InlineDetourReleaseError::StillEnabled),
        "the refusal must be the enabled one, not a freeze refusal: {refused:?}"
    );
    assert_eq!(retained_detour_count() - retained_before, 1);
    let recorded = crate::release_ledger::last_release_attempt(target as usize)
        .expect("the refused removal is on the ledger");
    assert!(
        matches!(
            recorded,
            crate::release_ledger::ReleaseAttemptOutcome::Refused(
                InlineDetourReleaseError::StillEnabled
            )
        ),
        "the ledger must record the refusal as a refusal, by name: {recorded:?}"
    );
    assert!(
        !recorded.released(),
        "a refused removal must not read as released"
    );
    assert!(
        crate::release_ledger::release_is_unproven(target as usize),
        "a target whose last release was refused is unproven"
    );

    // 2. Released, the parked thread returns through a trampoline that is still
    // there, and produces the original's answer.
    GATE_OPEN.store(true, Ordering::Release);
    // BOUNDED BEFORE THE JOIN. A trampoline regression that loops instead of
    // returning would make a bare `join` hang the whole test binary; waiting on
    // the observable flag first turns that into a named failure, and the join
    // that follows it is then immediate.
    wait_until("the callback to return through the trampoline", || {
        LEFT_DETOUR.load(Ordering::Acquire)
    });
    let answered = parked.join().expect("the parked thread returns");
    assert_eq!(
        answered,
        original_answer.wrapping_add(1),
        "the body must have reached the ORIGINAL through the trampoline"
    );

    // 3. The control. With the detour off and nothing in flight, the same
    // target removes -- so the refusal above was about this target's state and
    // not a blanket one.
    unsafe { disable(target) }.expect("the detour comes off");
    unsafe { remove(target) }.expect("a disabled, quiet target is removable");
    assert_eq!(
        gated_victim(7),
        original_answer,
        "the victim is itself again"
    );
    drop(custody);
}

// ---------------------------------------------------------------------------
// (c) A refused suspension and a refused context read.
// ---------------------------------------------------------------------------

/// A worker that spins until told to stop, so there is a thread for the fault
/// to name that is certainly alive and certainly not the test's own.
struct Worker {
    stop: std::sync::Arc<AtomicBool>,
    id: u32,
    /// Advanced on every loop iteration, so a test can ask whether this thread
    /// is RUNNING rather than only whether it exists -- which is the whole
    /// question after a resume.
    progress: std::sync::Arc<AtomicU64>,
    finished: std::sync::Arc<AtomicBool>,
}

impl Worker {
    fn spawn() -> Self {
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let started = std::sync::Arc::new(AtomicU64::new(0));
        let progress = std::sync::Arc::new(AtomicU64::new(0));
        let finished = std::sync::Arc::new(AtomicBool::new(false));
        {
            let stop = std::sync::Arc::clone(&stop);
            let started = std::sync::Arc::clone(&started);
            let progress = std::sync::Arc::clone(&progress);
            let finished = std::sync::Arc::clone(&finished);
            std::thread::spawn(move || {
                started.store(
                    unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() } as u64,
                    Ordering::Release,
                );
                while !stop.load(Ordering::Acquire) {
                    progress.fetch_add(1, Ordering::AcqRel);
                    std::hint::spin_loop();
                }
                finished.store(true, Ordering::Release);
            });
        }
        wait_until("the worker thread to publish its id", || {
            started.load(Ordering::Acquire) != 0
        });
        Self {
            stop,
            id: started.load(Ordering::Acquire) as u32,
            progress,
            finished,
        }
    }

    /// Wait for this thread to advance, or FAIL. A thread left suspended never
    /// advances, so this is what turns a resume regression into a failure
    /// rather than a hang.
    fn wait_for_progress(&self) {
        let from = self.progress.load(Ordering::Acquire);
        wait_until("the worker thread to advance", || {
            self.progress.load(Ordering::Acquire) > from
        });
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // BOUNDED, and deliberately NOT a `join`. A thread this freeze failed
        // to resume never reaches its exit, and joining it would block this
        // test for ever -- turning the exact regression the module exists to
        // report into a wedged battery. The handle is dropped rather than
        // joined, so the wait is on an observable fact instead.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !self.finished.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        if !self.finished.load(Ordering::Acquire) {
            let said = "a worker thread never finished: it was left suspended, \
                        which is the failure this module exists to name";
            // NEVER A SECOND PANIC. `wait_for_progress` has already failed the
            // test by the time this can be reached on that path, and a panic
            // during an unwind ABORTS the process -- taking the report of the
            // original failure with it and killing every other test in the
            // binary. Panicking is right only when nothing else has.
            assert!(
                std::thread::panicking(),
                "{said}"
            );
            eprintln!("fault_tests: {said}");
        }
    }
}

/// A SUSPENSION THAT CANNOT BE TAKEN REFUSES THE PATCH, having patched nothing.
///
/// The seam refuses one named thread's `SuspendThread` for the whole attempt --
/// persistently, because the freeze re-derives an unsettled thread on the next
/// pass and a one-shot fault would simply succeed the second time and prove
/// nothing.
///
/// The proof that nothing was patched is the TARGET'S BYTES, read through
/// MinHook's own patch footprint before and after, not the absence of an error.
///
/// kills: MinHook's deleted `Freeze`, which skipped a thread whose suspension
/// failed and patched anyway. Restoring that shape -- treating a refused
/// suspension as a thread that need not be accounted for -- makes this freeze
/// succeed and the byte comparison fail.
/// CONTAINED. See [`run_in_child`]: this test's own failure mode leaves a
/// thread parked or suspended and a detour live, so it runs in a process of its
/// own and the claims live on the child below.
#[test]
fn a_suspension_that_is_refused_patches_nothing() {
    run_in_child("fault_tests::child_a_suspension_that_is_refused_patches_nothing");
}

#[test]
#[ignore = "contained: driven by a_suspension_that_is_refused_patches_nothing"]
fn child_a_suspension_that_is_refused_patches_nothing() {
    assert!(
        std::env::var(CHILD_MARKER).is_ok(),
        "this test is only sound in the child process its parent spawns"
    );
    serialised!();
    let worker = Worker::spawn();
    let target = fault_victim_0 as *mut c_void;
    let _ = unsafe { disable(target) };
    let _ = unsafe { remove(target) };
    unsafe { MinHook::create_hook(target, fault_detour_0 as *mut c_void) }
        .expect("the victim is hookable");
    let custody = HookCustody::one(target);
    let before = patch_bytes(target);
    let original_answer = fault_victim_0(1);
    let enabled_before = enabled_detour_count();

    let refused = {
        let _armed = faults::refuse_suspension_of(worker.id);
        unsafe { enable(target) }.expect_err("a freeze that cannot hold every thread refuses")
    };

    assert!(
        faults::SUSPEND_FAULT_HITS.load(Ordering::Acquire) > 0,
        "the seam was never reached, so this test asserts nothing"
    );
    let InlineDetourEnableError::FreezeRefused(refusal) = refused else {
        panic!("the refusal must be the freeze's: {refused:?}");
    };
    assert!(
        matches!(refusal, crate::FreezeRefusal::ThreadSuspend { thread, .. } if thread == worker.id),
        "the refusal must name the thread whose suspension was refused: {refusal:?}"
    );
    assert_eq!(
        patch_bytes(target),
        before,
        "NOTHING WAS PATCHED: the bytes an enable would have rewritten are \
         exactly as they were"
    );
    assert_eq!(fault_victim_0(1), original_answer, "the victim is untouched");
    assert_eq!(
        enabled_detour_count(),
        enabled_before,
        "a refused enable counts no detour"
    );

    drop(custody);
    assert_eq!(fault_victim_0(1), original_answer, "cleanup left a detour live");
    drop(worker);
}

/// A CONTEXT THAT CANNOT BE READ REFUSES THE PATCH, having patched nothing.
///
/// Armed for the PRE-PATCH question specifically (mode 2), so the read that
/// fails is the one production takes to decide whether a held thread is inside
/// the bytes it is about to rewrite -- not the freeze's own settle. That is
/// deliberate: a fault that landed during acquisition would also refuse, and a
/// build that had DELETED the pre-patch check would still pass. This way,
/// deleting it makes the enable succeed and the byte comparison fail.
///
/// kills: `ProcessThreadIPs`, which read every thread's context to relocate a
/// parked instruction pointer and RETURNED SILENTLY when the read failed --
/// then patched. The fork deletes the relocation; this is the refusal that
/// replaced it, driven by the same failure that used to be ignored.
/// CONTAINED. See [`run_in_child`]: this test's own failure mode leaves a
/// thread parked or suspended and a detour live, so it runs in a process of its
/// own and the claims live on the child below.
#[test]
fn a_context_read_that_is_refused_patches_nothing() {
    run_in_child("fault_tests::child_a_context_read_that_is_refused_patches_nothing");
}

#[test]
#[ignore = "contained: driven by a_context_read_that_is_refused_patches_nothing"]
fn child_a_context_read_that_is_refused_patches_nothing() {
    assert!(
        std::env::var(CHILD_MARKER).is_ok(),
        "this test is only sound in the child process its parent spawns"
    );
    serialised!();
    let worker = Worker::spawn();
    let target = fault_victim_2 as *mut c_void;
    let _ = unsafe { disable(target) };
    let _ = unsafe { remove(target) };
    unsafe { MinHook::create_hook(target, fault_detour_2 as *mut c_void) }
        .expect("the victim is hookable");
    let custody = HookCustody::one(target);
    let before = patch_bytes(target);
    let original_answer = fault_victim_2(1);

    let refused = {
        let _armed = faults::refuse_context_of(worker.id, 2);
        unsafe { enable(target) }.expect_err("a context that cannot be read refuses")
    };

    assert!(
        faults::CONTEXT_FAULT_HITS.load(Ordering::Acquire) > 0,
        "the seam was never reached, so this test asserts nothing"
    );
    let InlineDetourEnableError::FreezeRefused(refusal) = refused else {
        panic!("the refusal must be the freeze's: {refused:?}");
    };
    assert!(
        matches!(
            refusal,
            crate::FreezeRefusal::ThreadContext { thread, os_error }
                if thread == worker.id && os_error == 5
        ),
        "the refusal must name the thread and carry the OS error: {refusal:?}"
    );
    assert_eq!(
        patch_bytes(target),
        before,
        "NOTHING WAS PATCHED: the bytes an enable would have rewritten are \
         exactly as they were"
    );
    assert_eq!(fault_victim_2(1), original_answer, "the victim is untouched");

    drop(custody);
    assert_eq!(fault_victim_2(1), original_answer, "cleanup left a detour live");
    drop(worker);
}

/// A RESUME REFUSED ONCE IS RETRIED, and the thread runs again.
///
/// `resume_or_count` retries [`RESUME_ATTEMPTS`] times, probes a thread it
/// still cannot resume for termination, and counts a real failure. Until this
/// seam existed none of that could be driven: the module's own source pin says
/// so in as many words, because `ResumeThread` does not fail on a live thread
/// this process holds a `THREAD_SUSPEND_RESUME` handle to.
///
/// The seam refuses the ATTEMPT rather than undoing a real resume, so it can
/// never itself leave a thread suspended -- the syscall it stands in for is not
/// made.
///
/// Three assertions, and the third is the one that would catch the defect:
///
/// 1. the seam fired exactly once, so the test is about something;
/// 2. `resume_failures()` did NOT move -- the retry succeeded, and a first
///    failure followed by recovery is not a failure;
/// 3. the worker thread ADVANCES after the freeze drops, which is the only
///    statement that means "the game runs again".
///
/// kills: deleting the retry loop, which makes assertion 3 hang until the
/// bounded wait fails it and makes assertion 2 fail; and counting a recovered
/// resume as a failure, which makes assertion 2 fail on its own.
/// CONTAINED. See [`run_in_child`]: this test's own failure mode leaves a
/// thread parked or suspended and a detour live, so it runs in a process of its
/// own and the claims live on the child below.
#[test]
fn a_resume_refused_once_is_retried_and_the_thread_runs_again() {
    run_in_child("fault_tests::child_a_resume_refused_once_is_retried_and_the_thread_runs_again");
}

#[test]
#[ignore = "contained: driven by a_resume_refused_once_is_retried_and_the_thread_runs_again"]
fn child_a_resume_refused_once_is_retried_and_the_thread_runs_again() {
    assert!(
        std::env::var(CHILD_MARKER).is_ok(),
        "this test is only sound in the child process its parent spawns"
    );
    serialised!();
    let worker = Worker::spawn();
    let failures_before = crate::freeze::resume_failures();

    {
        let _armed = faults::refuse_one_resume_attempt();
        let frozen = crate::freeze_process().expect("the process freezes");
        assert!(frozen.frozen_thread_count() >= 1, "there is a thread to resume");
        drop(frozen);
    }

    assert_eq!(
        faults::RESUME_FAULT_HITS.load(Ordering::Acquire),
        1,
        "the seam must have refused exactly one attempt"
    );
    assert_eq!(
        crate::freeze::resume_failures(),
        failures_before,
        "a refusal the retry recovered from is not a resume failure"
    );
    worker.wait_for_progress();
    drop(worker);
}

// ---------------------------------------------------------------------------
// (d) The allocation probe, inside a held window, in a child process.
// ---------------------------------------------------------------------------

/// The env var that marks a child process, so the parent half of each pair
/// returns instead of recursing.
const CHILD_MARKER: &str = "INLINE_DETOUR_FAULT_CHILD";

/// Run one `#[ignore]`d test of THIS binary in a FRESH PROCESS, and fail if it
/// does not pass.
///
/// # Why several tests here are paired this way
///
/// Every fault in this module is a fault the product must survive, so each test
/// has a failure mode that is the regression itself -- a thread that never
/// resumes, a callback that never leaves a detour body, an allocation that
/// blocks on the process heap with every other thread stopped. In one process
/// those outcomes do not merely fail their own test: they leave a LOOPING or
/// SUSPENDED thread and an enabled detour behind, and every test that runs
/// afterwards in that binary is then running in a process the failure has
/// already damaged. A timed retain keeps the first report honest; it does not
/// make the rest of the run trustworthy.
///
/// A fresh process makes the blast radius the child. The parent asserts on its
/// exit status under a hard deadline and KILLS it otherwise, so a hang is a
/// failed test rather than a wedged battery -- and whatever the child leaked
/// dies with it.
///
/// The child also gets a process to itself in the other direction: one test
/// thread, an empty freeze census and untouched counters, so ordinals and
/// deltas are predictable rather than merely present.
fn run_in_child(test_name: &str) {
    assert!(
        std::env::var(CHILD_MARKER).is_err(),
        "a child must never spawn a grandchild"
    );
    let exe = std::env::current_exe().expect("the test binary's own path");
    let mut child = std::process::Command::new(exe)
        .args([
            "--exact",
            test_name,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_MARKER, "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("the child test process starts");

    // A HARD BOUND. Every operation these children perform is one that can
    // hang when it regresses, so the parent must never wait without one.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    let status = loop {
        match child.try_wait().expect("the child is waitable") {
            Some(status) => break status,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "{test_name} hung in its child process, which is the failure \
                     mode it exists to report; the child was killed"
                );
            }
            None => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    };
    let output = child.wait_with_output().unwrap_or_else(|_| std::process::Output {
        status,
        stdout: Vec::new(),
        stderr: Vec::new(),
    });
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        status.success(),
        "{test_name} failed in its child process:\n{stdout}\n{stderr}"
    );
    // AND IT RAN. `libtest` exits 0 when a filter matches NOTHING, so a renamed
    // child would leave every one of these proofs green while proving nothing
    // at all -- the vacuity every test in this module is written to avoid.
    assert!(
        stdout.contains("1 passed") && stdout.contains("0 failed"),
        "{test_name} was filtered away rather than run:\n{stdout}"
    );
}

/// THE INSTRUMENT THAT FOUND D75, PROVEN TO SEE, and proven against the census
/// row for the very window the allocation happened in.
///
/// It runs in a child process because it deliberately performs the operation
/// KZ's audit exists to forbid: an allocation with every other thread of the
/// process suspended can block for ever on the process heap lock, and a hung
/// test binary wedges the battery for every lane. One test thread in a fresh
/// process also means the census is empty, so the window's ordinal is
/// predictable rather than merely present.
///
/// The claim is exactly this and no more: the probe observed an allocation
/// while a particular `ProcessFreeze` guard was held, and the production census
/// carries THAT window's ordinal. The census records stopped windows, not
/// allocations, and nothing here adds a counter to it.
///
/// kills: a probe that has stopped counting -- an `#[global_allocator]` that
/// was dropped, an `observed` that no longer arms the thread. Any of those
/// makes `the_probe_is_installed()` false or the count zero, and the audit's
/// every `assert_eq!(allocations, 0)` elsewhere becomes vacuous the same day.
#[test]
fn the_probe_sees_an_allocation_inside_a_held_window() {
    run_in_child("fault_tests::child_allocates_inside_a_held_freeze");
}

/// The child half. `#[ignore]`d so the ordinary battery never runs it in a
/// process with other test threads, which is where the allocation could hang.
#[test]
#[ignore = "driven by the_probe_sees_an_allocation_inside_a_held_window in a child process"]
fn child_allocates_inside_a_held_freeze() {
    assert!(
        std::env::var(CHILD_MARKER).is_ok(),
        "this test is only meaningful in the child process its parent spawns"
    );
    assert!(
        freeze_window_audit::the_probe_is_installed(),
        "WITHOUT THE PROBE EVERY ZERO IN THIS WORKSPACE IS VACUOUS: the crate's \
         test binary must install AllocationProbe as its global allocator"
    );
    let attempts_before = crate::freeze_attempts();

    let frozen = crate::freeze_process().expect("a fresh process freezes");
    // ONLY the deliberate allocation is observed. Wrapping the freeze itself
    // would count the buffers the acquisition legitimately reserves BEFORE it
    // suspends anything, and "the allocation happened inside the held window"
    // would no longer be what the number said.
    let (length, allocations) = freeze_window_audit::observed(|| {
        let mut grown: Vec<u8> = Vec::new();
        grown.push(1);
        grown.push(2);
        grown.len()
    });
    drop(frozen);

    assert_eq!(length, 2);
    assert!(
        allocations > 0,
        "the probe must see an allocation made while the freeze was held"
    );

    let ordinal = attempts_before + 1;
    let windows = crate::freeze_census::freeze_windows();
    let window = windows
        .iter()
        .find(|window| window.ordinal == ordinal)
        .unwrap_or_else(|| panic!("the census carries window {ordinal}: {windows:?}"));
    assert!(!window.refused, "the freeze was taken, not refused");
    assert_eq!(
        window.entry_point,
        Some(crate::freeze_census::FreezeEntryPoint::Direct),
        "a bare freeze_process is a Direct window"
    );
}

// ---------------------------------------------------------------------------
// Inertness: the seams are ABSENT from what ships, and it is proved.
// ---------------------------------------------------------------------------

mod inertness {
    /// EVERY MANIFEST IN THE WORKSPACE, read with its TABLE CONTEXT, and the
    /// test-support feature is reachable from exactly one place: this crate's
    /// DEV-dependency table.
    ///
    /// A dev-dependency is not in the graph of `cargo build --release -p
    /// collector` at all, so the shipped artifact cannot carry the feature.
    /// `runs/ln_fault_inject/inertness_graph.txt` records cargo's own resolved
    /// answer for that command; this is the standing check that stops a future
    /// edit reopening the route.
    ///
    /// TABLE CONTEXT IS THE POINT. The first spelling of this test asked only
    /// whether the offending LINE mentioned `dev-dependencies`, which it never
    /// does -- the table header is a different line -- so moving the dependency
    /// verbatim into `[dependencies]` would have passed. Every `test_support`
    /// mention is now attributed to the `[table]` it is under, and three
    /// distinct routes are refused by name:
    ///
    /// * a NORMAL dependency activating it, anywhere;
    /// * a `[features]` entry in any crate forwarding `minhook_h4/test_support`
    ///   into some feature of its own, which would put it in the graph of every
    ///   dependent that enables that feature;
    /// * the feature being declared anywhere but `minhook_h4`.
    ///
    /// What production change would make this fail? Any of those three.
    #[test]
    fn the_test_support_feature_is_reachable_only_through_a_dev_dependency() {
        let native = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("inline_detour sits in extractor/native")
            .to_path_buf();
        let mut manifests = Vec::new();
        collect_manifests(&native, &mut manifests);
        assert!(
            manifests.len() > 40,
            "the sweep must actually reach the workspace: found {}",
            manifests.len()
        );

        let mut dev_activations = 0usize;
        let mut declarations = 0usize;
        for manifest in &manifests {
            let text = std::fs::read_to_string(manifest)
                .unwrap_or_else(|error| panic!("{}: {error}", manifest.display()));
            let file = manifest.to_string_lossy().replace('\\', "/");
            let mut table = String::new();
            for (number, raw) in text.lines().enumerate() {
                // THE COMMENT IS CUT FIRST. Half this file's manifests explain
                // the feature in prose beside it, and a scan that read those as
                // activations would fail on a correct tree -- a check that
                // cries wolf gets deleted, and then it is not there on the day
                // it matters.
                let line = raw.split('#').next().unwrap_or("").trim();
                if line.is_empty() {
                    continue;
                }
                if line.starts_with('[') && line.ends_with(']') {
                    table = line.trim_matches(['[', ']']).to_string();
                    continue;
                }
                if !names_the_feature(line) {
                    continue;
                }
                // TABLE NAMES ARE DOTTED PATHS. `[dev-dependencies]`,
                // `[target.'cfg(windows)'.dev-dependencies]` and
                // `[dev-dependencies.minhook_h4]` are all the same table for
                // this purpose, and the first spelling of this check accepted
                // only the second of the three.
                let segments: Vec<&str> = split_table_path(&table);
                // METADATA IS NOT A DEPENDENCY EDGE. `[package.metadata.*]` and
                // `[workspace.metadata.*]` are the two free-form tables cargo
                // hands to other tools -- docs.rs's `features` list is the
                // common one -- and neither activates anything. Failing on them
                // would fail a correct tree, and a check that cries wolf gets
                // deleted before the day it matters.
                if matches!(segments.first(), Some(&"package") | Some(&"workspace"))
                    && segments.get(1) == Some(&"metadata")
                {
                    continue;
                }
                let at = format!("{file}:{} in [{table}]", number + 1);
                let kind = dependency_table_kind(&segments);
                // The declaration itself, which must be the feature's own crate.
                if kind == Some("features") {
                    assert!(
                        file.ends_with("minhook_h4/Cargo.toml"),
                        "{at} forwards the test-support feature; a feature that \
                         forwards it puts the seam in the graph of every crate \
                         that enables that feature: {line}"
                    );
                    declarations += 1;
                    continue;
                }
                // Anything else is a dependency edge, and only a DEV one may
                // carry it. The table name is what decides, not the line.
                assert!(
                    kind == Some("dev-dependencies"),
                    "{at} activates the test-support feature from a table that \
                     ships; only a dev-dependency may: {line}"
                );
                assert!(
                    file.ends_with("inline_detour/Cargo.toml"),
                    "{at} activates the test-support feature outside \
                     inline_detour, which is the one crate whose tests need it: \
                     {line}"
                );
                dev_activations += 1;
            }
        }
        assert_eq!(
            declarations, 1,
            "the feature is declared exactly once, by the crate that owns it"
        );
        assert_eq!(
            dev_activations, 1,
            "exactly one dev-dependency activates it"
        );
    }

    /// Does this line name the feature ITSELF, rather than merely contain the
    /// letters?
    ///
    /// `my_test_support = []` is somebody else's feature and must not fail this
    /// check. The name has to be bounded on both sides by something that is not
    /// part of a cargo identifier.
    fn names_the_feature(line: &str) -> bool {
        let mut rest = line;
        while let Some(at) = rest.find("test_support") {
            let before = rest[..at].chars().next_back();
            let after = rest[at + "test_support".len()..].chars().next();
            let bounded = |c: Option<char>| {
                !matches!(c, Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-')
            };
            if bounded(before) && bounded(after) {
                return true;
            }
            rest = &rest[at + "test_support".len()..];
        }
        false
    }

    /// WHICH KIND OF TABLE this header names, by POSITION rather than by
    /// mentioning a word.
    ///
    /// The position is what cargo goes by, and a check that went by mention
    /// could be walked straight past: `[dependencies."dev-dependencies"]` is a
    /// perfectly ordinary SHIPPING dependency whose alias happens to be spelled
    /// `dev-dependencies`, and a scan asking whether any segment equals that
    /// string would wave it through into the collector's graph. So the kind is
    /// read from the one slot cargo reads it from -- the first segment, or the
    /// third when the table is `target.<selector>.<kind>` -- and anything after
    /// it is a dependency NAME and carries no authority at all.
    ///
    /// `None` for a shape this check does not understand -- and the caller
    /// treats `None` as a REFUSAL, deliberately: an unrecognised table that
    /// names the feature is exactly the case where guessing is wrong, and
    /// failing closed is what the product does everywhere else. The one shape
    /// that is genuinely not an edge, `[package.metadata.*]`, is skipped by the
    /// caller before it gets here.
    fn dependency_table_kind<'a>(segments: &[&'a str]) -> Option<&'a str> {
        let (kind, rest) = if segments.first() == Some(&"target") {
            // `target.<selector>.<kind>[.<name>]`
            (segments.get(2)?, &segments[3..])
        } else {
            (segments.first()?, &segments[1..])
        };
        // At most a dependency name may follow the kind. Anything deeper is not
        // a shape this check understands, and it must not be read as one.
        if rest.len() > 1 {
            return None;
        }
        Some(kind)
    }

    /// A TOML table header split into its dotted segments, with quoted segments
    /// -- `target.'cfg(windows)'.dev-dependencies` -- kept whole.
    fn split_table_path(table: &str) -> Vec<&str> {
        let mut segments = Vec::new();
        let mut quote: Option<char> = None;
        let mut start = 0usize;
        for (at, character) in table.char_indices() {
            match quote {
                Some(open) if character == open => quote = None,
                Some(_) => {}
                None if character == '\'' || character == '"' => quote = Some(character),
                None if character == '.' => {
                    segments.push(table[start..at].trim().trim_matches(['\'', '"']));
                    start = at + 1;
                }
                None => {}
            }
        }
        segments.push(table[start..].trim().trim_matches(['\'', '"']));
        segments
    }

    fn collect_manifests(root: &std::path::Path, into: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if name == "target" || name == ".git" || name.starts_with(".cargo-target") {
                    continue;
                }
                collect_manifests(&path, into);
            } else if name == "Cargo.toml" {
                into.push(path);
            }
        }
    }

    /// THE SENTINEL IS REACHABLE HERE, which is what makes its ABSENCE from a
    /// shipped DLL a statement rather than a tautology.
    ///
    /// The string is returned by the C, not copied on the Rust side: a Rust
    /// copy would still be here after the `#ifdef` block was deleted, and the
    /// artifact check would go on passing while proving nothing.
    ///
    /// The other half of this proof cannot live in a unit test, because a unit
    /// test cannot build the release collector. It is
    /// `runs/ln_fault_inject/inertness_artifact.txt`, which records BOTH
    /// directions: the string is absent from the release collector built
    /// without the feature and present in one built with it.
    #[test]
    fn the_sentinel_is_present_when_the_feature_is_on() {
        assert_eq!(
            minhook::EnableIncoherentFault::sentinel(),
            "MH_H4_FAULT_INJECTION_PRESENT"
        );
    }
}
