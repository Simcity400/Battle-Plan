use super::*;
use crate::freeze::faults;
use core::ffi::c_void;
use minhook::{MinHook, MH_STATUS};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

static FAULT_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

macro_rules! serialised {
    () => {
        let _fault = FAULT_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _ledger = crate::release_ledger::TEST_LEDGER_SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _suspend = crate::freeze::os_suspend_state_guard();
    };
}

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
        minhook::EnableIncoherentFault::disarm();
        for &target in &self.0 {
            let _ = unsafe { disable(target) };
            let _ = unsafe { remove(target) };
        }
    }
}

type Victim = extern "system" fn(u64) -> u64;

const ABSENT_FAULT_TARGET: *mut c_void = 0x5100 as *mut c_void;

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

fn undetoured_answer(index: usize) -> u64 {
    trio()[index].1(1)
}

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

fn patch_bytes(target: *mut c_void) -> Vec<u8> {
    let lock = minhook::MinHookLock::acquire().expect("the lock is free");
    let footprint =
        MinHook::patch_footprint(&lock, target).expect("the hook exists, so it has a footprint");
    drop(lock);
    let (start, end) = footprint.patch;
    // SAFETY: MinHook answered with the exact range an enable rewrites, which
    unsafe { std::slice::from_raw_parts(start as *const u8, end - start) }.to_vec()
}

#[test]
fn an_apply_refused_after_the_bytes_were_written_rolls_the_whole_batch_back() {
    serialised!();
    let originals: Vec<u64> = (0..3).map(undetoured_answer).collect();
    let targets = create_trio();
    let custody = HookCustody::of(&targets);
    let faulted = targets[1].1;

    let enabled_before = enabled_detour_count();
    let armed = minhook::EnableIncoherentFault::arm(faulted);
    let report = unsafe { enable_batch(&targets) };

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

    let (name, status) = report
        .refusal()
        .expect("a batch whose apply refused must name a refusal");
    assert_eq!(
        status.minhook_status(),
        Some(MH_STATUS::MH_ERROR_MEMORY_PROTECT),
        "the apply's own refusal must survive into the named failure: {report:?}"
    );
    assert!(
        !report.all_succeeded(),
        "a batch whose apply was refused has not all succeeded"
    );
    assert_eq!(
        report.failures().len(),
        3,
        "the apply names no target, so every target is refused with it: {:?}",
        report.outcomes()
    );
    for (target_name, error) in report.failures() {
        assert_eq!(
            error,
            InlineDetourEnableError::BatchApplyRefused {
                status: MH_STATUS::MH_ERROR_MEMORY_PROTECT,
                still_live: false,
                rolled_back: true,
            },
            "{target_name} must carry the BATCH's refusal, not a per-hook one"
        );
    }
    assert!(
        report.rolled_back(),
        "a refused apply undoes the batch: {report:?}"
    );
    assert_eq!(
        report.apply_refusal(),
        Some(InlineDetourEnableError::BatchApplyRefused {
            status: MH_STATUS::MH_ERROR_MEMORY_PROTECT,
            still_live: false,
            rolled_back: true,
        })
    );

    for (index, (victim_name, _)) in targets.iter().enumerate() {
        assert_eq!(
            trio()[index].1(1),
            originals[index],
            "{victim_name} must not be left detoured by a rolled-back batch"
        );
    }
    assert_eq!(
        enabled_detour_count(),
        enabled_before,
        "a batch that rolled itself back left no detour to count"
    );
    assert_eq!(
        report.still_live(),
        [false, false, false],
        "no target of a rolled-back batch is live: {report:?}"
    );

    let _ = unsafe { apply_queued() };
    for (index, (victim_name, _)) in targets.iter().enumerate() {
        assert_eq!(
            trio()[index].1(1),
            originals[index],
            "an unrelated apply must not patch what this batch rolled back \
             ({victim_name})"
        );
    }

    let frozen = crate::freeze_process().expect("the process freezes");
    let asked = unsafe { MinHook::remove_hook(frozen.minhook_lock(), faulted) };
    drop(frozen);
    assert_eq!(
        asked,
        Ok(()),
        "the rollback's disable must re-complete the write and clear the \
         incoherent latch; without it MH_RemoveHook refuses this target for ever"
    );

    let rendered = format!("{status}");
    assert!(
        rendered.contains("batch refused"),
        "the cause must say the BATCH refused, not name a per-hook status: \
         {rendered}"
    );
    assert!(
        rendered.contains("rolled back") && rendered.contains("not live"),
        "the cause must say the game is not left patched: {rendered}"
    );
    assert!(
        targets.iter().any(|(target_name, _)| *target_name == name),
        "the refusal must name one of the batch's own targets, not a \
         synthesised label: {name}"
    );

    drop(custody);
    for (index, original) in originals.iter().enumerate() {
        assert_eq!(trio()[index].1(1), *original, "cleanup left a detour live");
    }
}

#[test]
fn a_batch_that_names_no_refusal_is_a_batch_that_all_succeeded() {
    serialised!();

    fn agree(what: &str, report: &crate::BatchEnableReport) {
        assert_eq!(
            report.all_succeeded(),
            report.refusal().is_none(),
            "{what}: the verdict and the named refusal disagree: {report:?}"
        );
        assert_eq!(
            report.all_succeeded(),
            report.failures().is_empty(),
            "{what}: the verdict and the failure list disagree: {report:?}"
        );
        assert!(
            report.apply_refusal().is_none() || !report.all_succeeded(),
            "{what}: a batch-level refusal that reports success: {report:?}"
        );
        assert_eq!(
            report.still_live().len(),
            report.outcomes().len(),
            "{what}: still_live() must answer for every target: {report:?}"
        );
        assert!(
            !report.rolled_back() || !report.still_live().contains(&true),
            "{what}: rolled_back() over a live target: {report:?}"
        );
        assert!(
            !report.all_succeeded() || report.still_live().iter().all(|live| *live),
            "{what}: a successful batch with a target that is not live: {report:?}"
        );
    }

    agree("an empty batch", &unsafe { enable_batch(&[]) });

    let targets = create_trio();
    let custody = HookCustody::of(&targets);

    agree("a roster listing one target twice", &unsafe {
        enable_batch(&[targets[0], targets[1], targets[0]])
    });

    let clean = unsafe { enable_batch(&targets) };
    agree("a clean batch", &clean);
    assert!(clean.all_succeeded(), "{:?}", clean.outcomes());
    let released = unsafe { disable_batch(&targets) };
    assert!(released.all_succeeded(), "{:?}", released.outcomes());

    let armed = minhook::EnableIncoherentFault::arm(targets[2].1);
    let faulted = unsafe { enable_batch(&targets) };
    assert_eq!(
        minhook::EnableIncoherentFault::witness(),
        Some("MH_H4_FAULT_INJECTION_PRESENT"),
        "the faulted shape must actually have been reached"
    );
    drop(armed);
    agree("a batch whose apply refused", &faulted);
    assert!(!faulted.all_succeeded(), "{:?}", faulted.outcomes());

    drop(custody);
}

#[test]
fn a_batch_refusal_outranks_one_target_s_own_refusal_in_the_verdict() {
    serialised!();
    let originals: Vec<u64> = (0..2).map(undetoured_answer).collect();
    let targets = create_trio();
    let custody = HookCustody::of(&targets);
    let batch = [("absent.hook", ABSENT_FAULT_TARGET), targets[0], targets[1]];

    let enabled_before = enabled_detour_count();
    let armed = minhook::EnableIncoherentFault::arm(targets[1].1);
    let report = unsafe { enable_batch(&batch) };

    assert_eq!(
        minhook::EnableIncoherentFault::witness(),
        Some("MH_H4_FAULT_INJECTION_PRESENT"),
        "the injected branch inside EnableHookLL must have executed"
    );
    drop(armed);

    let (name, status) = report
        .refusal()
        .expect("a batch that refused twice still names a refusal");
    assert_ne!(
        name,
        "absent.hook",
        "the whole-batch cause outranks one target's own: {:?}",
        report.outcomes()
    );
    assert_eq!(
        status,
        InlineDetourEnableError::BatchApplyRefused {
            status: MH_STATUS::MH_ERROR_MEMORY_PROTECT,
            still_live: false,
            rolled_back: true,
        },
        "the verdict must carry the BATCH's cause: {:?}",
        report.outcomes()
    );

    assert_eq!(report.failures().len(), 3, "{:?}", report.outcomes());
    assert_eq!(
        report.outcomes()[0].1,
        Err(InlineDetourEnableError::MinHook(
            MH_STATUS::MH_ERROR_NOT_CREATED
        )),
        "the absent target keeps its own refusal: {:?}",
        report.outcomes()
    );

    assert!(report.rolled_back(), "{report:?}");
    assert_eq!(
        report.still_live(),
        [false, false, false],
        "nothing survives a rolled-back batch: {report:?}"
    );
    for index in 0..2 {
        assert_eq!(
            trio()[index].1(1),
            originals[index],
            "a rolled-back batch left a victim detoured"
        );
    }
    assert_eq!(
        enabled_detour_count(),
        enabled_before,
        "a batch that rolled itself back left no detour to count"
    );

    drop(custody);
}

static GATE_OPEN: AtomicBool = AtomicBool::new(false);
static IN_DETOUR: AtomicBool = AtomicBool::new(false);
static LEFT_DETOUR: AtomicBool = AtomicBool::new(false);
static GATED_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

#[inline(never)]
extern "system" fn gated_victim(x: u64) -> u64 {
    x.wrapping_mul(23).wrapping_add(0xE000)
}

#[inline(never)]
extern "system" fn gated_detour(x: u64) -> u64 {
    IN_DETOUR.store(true, Ordering::Release);
    while !GATE_OPEN.load(Ordering::Acquire) {
        std::hint::spin_loop();
    }
    let original = GATED_ORIGINAL.load(Ordering::Acquire);
    assert_ne!(
        original, 0,
        "the trampoline address must be published first"
    );
    // SAFETY: the trampoline MinHook returned for this target. If a teardown
    let original: Victim = unsafe { std::mem::transmute(original) };
    let answered = original(x).wrapping_add(1);
    // use-after-free -- and it is what the custody guard waits for.
    LEFT_DETOUR.store(true, Ordering::Release);
    answered
}

/// Releases the parked callback, WAITS FOR IT TO LEAVE, and only then restores
/// IMMEDIATELY -- with the released thread somewhere between the gate and its
/// call: the exact use-after-free this test exists to talk about, performed by
struct ParkedCallbackCustody(*mut c_void);

impl Drop for ParkedCallbackCustody {
    fn drop(&mut self) {
        GATE_OPEN.store(true, Ordering::Release);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !LEFT_DETOUR.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        let _ = unsafe { disable(self.0) };
        if !LEFT_DETOUR.load(Ordering::Acquire) {
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

/// 2. the parked thread, released, returns THROUGH the trampoline and produces
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
    GATE_OPEN.store(true, Ordering::Release);
    wait_until("the callback to return through the trampoline", || {
        LEFT_DETOUR.load(Ordering::Acquire)
    });
    let answered = parked.join().expect("the parked thread returns");
    assert_eq!(
        answered,
        original_answer.wrapping_add(1),
        "the body must have reached the ORIGINAL through the trampoline"
    );

    unsafe { disable(target) }.expect("the detour comes off");
    unsafe { remove(target) }.expect("a disabled, quiet target is removable");
    assert_eq!(
        gated_victim(7),
        original_answer,
        "the victim is itself again"
    );
    drop(custody);
}

struct Worker {
    stop: std::sync::Arc<AtomicBool>,
    id: u32,
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
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !self.finished.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        if !self.finished.load(Ordering::Acquire) {
            let said = "a worker thread never finished: it was left suspended, \
                        which is the failure this module exists to name";
            assert!(std::thread::panicking(), "{said}");
            eprintln!("fault_tests: {said}");
        }
    }
}

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
    assert_eq!(
        fault_victim_0(1),
        original_answer,
        "the victim is untouched"
    );
    assert_eq!(
        enabled_detour_count(),
        enabled_before,
        "a refused enable counts no detour"
    );

    drop(custody);
    assert_eq!(
        fault_victim_0(1),
        original_answer,
        "cleanup left a detour live"
    );
    drop(worker);
}

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
    assert_eq!(
        fault_victim_2(1),
        original_answer,
        "the victim is untouched"
    );

    drop(custody);
    assert_eq!(
        fault_victim_2(1),
        original_answer,
        "cleanup left a detour live"
    );
    drop(worker);
}

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
        assert!(
            frozen.frozen_thread_count() >= 1,
            "there is a thread to resume"
        );
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

const CHILD_MARKER: &str = "INLINE_DETOUR_FAULT_CHILD";

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
    let output = child
        .wait_with_output()
        .unwrap_or_else(|_| std::process::Output {
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
    assert!(
        stdout.contains("1 passed") && stdout.contains("0 failed"),
        "{test_name} was filtered away rather than run:\n{stdout}"
    );
}

#[test]
fn the_probe_sees_an_allocation_inside_a_held_window() {
    run_in_child("fault_tests::child_allocates_inside_a_held_freeze");
}

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

mod inertness {
    /// A dev-dependency is not in the graph of `cargo build --release -p
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
                let segments: Vec<&str> = split_table_path(&table);
                if matches!(segments.first(), Some(&"package") | Some(&"workspace"))
                    && segments.get(1) == Some(&"metadata")
                {
                    continue;
                }
                let at = format!("{file}:{} in [{table}]", number + 1);
                let kind = dependency_table_kind(&segments);
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

    fn names_the_feature(line: &str) -> bool {
        let mut rest = line;
        while let Some(at) = rest.find("test_support") {
            let before = rest[..at].chars().next_back();
            let after = rest[at + "test_support".len()..].chars().next();
            let bounded = |c: Option<char>| !matches!(c, Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if bounded(before) && bounded(after) {
                return true;
            }
            rest = &rest[at + "test_support".len()..];
        }
        false
    }

    fn dependency_table_kind<'a>(segments: &[&'a str]) -> Option<&'a str> {
        let (kind, rest) = if segments.first() == Some(&"target") {
            (segments.get(2)?, &segments[3..])
        } else {
            (segments.first()?, &segments[1..])
        };
        if rest.len() > 1 {
            return None;
        }
        Some(kind)
    }

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

/// A RELEASE whose apply refuses leaves one target's restore WRITTEN AND
/// UNPUBLISHED, and this batch now clears it inside its own freeze.
///
/// The shape, exactly as production reaches it: `MH_ApplyQueued` refuses at one
/// target, whose bytes are already restored, whose `isEnabled` is already FALSE
/// and whose `incoherent` latch is set. The completion loop then calls
/// `MH_DisableHook` on it, MinHook answers `MH_ERROR_DISABLED` because
/// `isEnabled` already matches -- WITHOUT calling `EnableHookLL` -- and that was
/// folded into `Ok` and into `already_released`. So a release batch reported a
/// clean teardown over a target whose restore no processor was guaranteed to
/// execute, and MinHook then refused to remove that entry for the life of the
/// process (lane LT, residual 1).
///
/// The only thing that clears the latch is to re-enable the detour and disable
/// it again, which patches the running game to heal a bookkeeping latch. OWNER
/// RULING 2026-08-16: allowed, inside the freeze the release already holds.
///
/// What production change would make this fail?
///   * deleting the heal loop: `incoherent_cleared()` is empty (assertion 2)
///     and the removal below refuses with `MH_ERROR_MEMORY_PROTECT`
///     (assertion 4), which is the whole defect;
///   * healing without asking MinHook again -- reporting cleared on the strength
///     of the two calls returning `Ok` -- leaves assertion 3 unchecked, so that
///     assertion is written against `hook_is_incoherent` and not against them;
///   * moving the heal outside the freeze: the re-enabled detour becomes
///     observable to a game thread, which is what the ruling bounds. Not
///     assertable here (nothing else runs), and stated in the code.
///
/// NOT DRIVABLE HERE: the UNHEALED branch. The seam is one-shot and is consumed
/// by the apply, so a second completion failure cannot be injected into the
/// heal's own re-enable. That branch is argued from `EnableHookLL`'s source --
/// it re-runs the same completion and re-latches on the same failure -- and its
/// report path is the same one every other refusal in this function takes.
#[test]
fn a_release_whose_apply_refused_clears_the_unpublished_restore_it_left() {
    serialised!();
    let targets = create_trio();
    let custody = HookCustody::of(&targets);
    let faulted = targets[1].1;

    let enabled = unsafe { enable_batch(&targets) };
    assert!(
        enabled.all_succeeded(),
        "the fixture must start with three live detours: {enabled:?}"
    );

    let armed = minhook::EnableIncoherentFault::arm(faulted);
    let report = unsafe { disable_batch(&targets) };

    // 0. THE FAULT WAS CONSUMED AND ITS BRANCH RAN. Without this the test is
    //    about an ordinary release and asserts nothing.
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

    // 1. The batch still names the refusal it met.
    assert!(
        report.apply_refusal().is_some(),
        "an apply that refused must be named whatever the per-target outcomes say: {report:?}"
    );

    // 2. The unpublished restore was found and named BY NAME.
    assert_eq!(
        report.incoherent_cleared(),
        [targets[1].0],
        "the target whose restore was written and not published must be named: {report:?}"
    );
    assert!(
        report.incoherent_retained().is_empty(),
        "the heal completed, so nothing may be reported as still unpublished: {report:?}"
    );

    // 3. ASKED OF MINHOOK, not inferred from the two calls returning Ok.
    let lock = minhook::MinHookLock::acquire().expect("the fork's library lock");
    assert_eq!(
        MinHook::hook_is_incoherent(&lock, faulted),
        Ok(false),
        "the latch must be clear, not merely reported clear"
    );
    assert_eq!(
        MinHook::hook_is_enabled(&lock, faulted),
        Ok(false),
        "the heal must leave the detour DOWN, not reinstated"
    );
    drop(lock);

    // 4. THE POINT OF ALL OF IT: MinHook refuses to remove an incoherent entry,
    //    so an uncleared latch strands the trampoline for the life of the
    //    process. A cleared one removes.
    for &(name, target) in &targets {
        unsafe { remove(target) }.unwrap_or_else(|status| {
            panic!("{name} must be removable after a healed release: {status}")
        });
    }
    // The originals answer again: nothing is detoured.
    for (index, &(name, _)) in targets.iter().enumerate() {
        assert_eq!(
            trio()[index].1(1),
            undetoured_answer(index),
            "{name} is still detoured after a healed release"
        );
    }
    std::mem::forget(custody);
}
