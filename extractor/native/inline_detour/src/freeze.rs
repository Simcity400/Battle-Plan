//! A fail-closed, process-wide thread freeze held ACROSS the MinHook call.
//!
//! # The defect this exists to close
//!
//! MinHook rewrites the first bytes of a live function. To do that safely it
//! must know no other thread is executing inside those bytes, so `Freeze()` in
//! `minhook/src/hook.c` enumerates the process's threads and suspends each one.
//! It then reads each suspended thread's context and, if the instruction
//! pointer landed inside the region being rewritten, relocates it
//! (`ProcessThreadIPs`).
//!
//! `Freeze()` FAILS OPEN. Reading the vendored 0.9.0 source:
//!
//! ```text
//! HANDLE hThread = OpenThread(THREAD_ACCESS, FALSE, pThreads->pItems[i]);
//! BOOL suspended = FALSE;
//! if (hThread != NULL) {
//!     DWORD result = SuspendThread(hThread);
//!     if (result != 0xFFFFFFFF) { suspended = TRUE; ProcessThreadIPs(...); }
//!     CloseHandle(hThread);
//! }
//! if (!suspended) { pThreads->pItems[i] = 0; }   // and that is all
//! ```
//!
//! The only status `Freeze` ever reports is `MH_ERROR_MEMORY_ALLOC`, from a
//! failed thread-array allocation. A thread that could not be opened or could
//! not be suspended is simply struck off the resume list; the patch is then
//! applied anyway, and because no context could be read, that thread's
//! instruction pointer is also never relocated. In a live game that is a
//! torn instruction stream in a running thread -- a nondeterministic crash.
//!
//! # What this module does instead
//!
//! It freezes the process itself, BEFORE MinHook is called, and holds the
//! freeze across the call. Every live thread is opened with an access mask
//! that is a superset of MinHook's own `THREAD_ACCESS` and suspended. If any
//! thread cannot be accounted for, the freeze is REFUSED and MinHook is never
//! called, so nothing is patched or unpatched.
//!
//! When the freeze succeeds, MinHook's `Freeze()` runs against a process where
//! every live thread is already suspended by us on a handle we already proved
//! openable at its access mask. Its `OpenThread` therefore cannot fail (the
//! grant cannot shrink and the thread cannot exit while suspended) and its
//! `SuspendThread` cannot fail -- but only because this module makes room for
//! it. `SuspendThread` fails rather than increment past
//! `MAXIMUM_SUSPEND_COUNT`, so a thread we suspend to exactly that ceiling is
//! one MinHook cannot suspend, and MinHook's answer to that is its fail-open
//! branch. Our suspension therefore refuses unless MinHook's own would still
//! fit on top of it. With that, the fail-open branch is unreachable for every
//! thread this module accounted for, and `ProcessThreadIPs` now runs for
//! threads it previously skipped.
//!
//! # "Accounted for" means suspended or PROVEN gone
//!
//! There is no error-code allowlist here, deliberately. The four hand-rolled
//! suspenders this module replaced reached that rule by guessing which code
//! Windows returns -- two of them excused `ERROR_INVALID_HANDLE` and
//! `ERROR_INVALID_PARAMETER` from `SuspendThread` as "the thread exited". A
//! terminating thread reports neither, and the guess is what made those
//! installers refuse under load.
//!
//! A thread is accounted for here only when
//!
//!  * it was opened and suspended -- it is provably not running; or
//!  * `OpenThread` failed with `ERROR_INVALID_PARAMETER`, meaning the kernel
//!    has no thread with that id, so the id in our snapshot is stale; or
//!  * `OpenThread` failed some OTHER way AND a fresh enumeration of THIS
//!    PROCESS, taken after that failure, no longer lists the id -- see
//!    "Thread ids are system-wide" below; or
//!  * the handle was opened, `SuspendThread` failed, AND the thread object
//!    becomes signalled within [`TERMINATION_PROOF_TIMEOUT_MS`], which is
//!    positive proof the thread has terminated and will execute no further
//!    user-mode instruction.
//!
//! Anything else refuses. Every id that is NOT already held suspended has its
//! disposition re-derived from scratch each pass, so a recycled id belonging
//! to a new live thread is opened and suspended like any other.
//!
//! # Thread ids are system-wide, and that is why `OpenThread` can be DENIED
//!
//! `OpenThread` takes a bare thread id -- `CLIENT_ID.UniqueProcess` is NULL --
//! so the kernel resolves it against the SYSTEM-WIDE id table, not against
//! this process. Ids are recycled from that table, and nothing reserves a
//! freed id for the process that last used it.
//!
//! So the sequence below is ordinary rather than exotic, and MEASURED here
//! (see `an_id_that_is_no_longer_ours_does_not_refuse_the_freeze`): one of our
//! threads exits between the snapshot and the open; the kernel hands its id to
//! a thread in a DIFFERENT process; our `OpenThread` therefore targets a
//! stranger's thread and is correctly refused with `ERROR_ACCESS_DENIED` (5).
//! Under the churn the module's own tests generate this happened in roughly
//! one run in five, and every single occurrence had the same shape: the id was
//! absent from a fresh enumeration of our own process, and a system-wide
//! snapshot named another process as its owner. Six of eight probed instances
//! could not even be opened for `THREAD_QUERY_LIMITED_INFORMATION`, so asking
//! the handle who owns it is not a usable test; re-enumerating our own
//! process is.
//!
//! Reading that denial as "a thread of ours that may be running" refused an
//! entire hook installation, at exactly the rate above. It is also
//! self-contradictory: a live thread of our own process IS openable at
//! [`FREEZE_THREAD_ACCESS`], which is the claim the paragraph about MinHook's
//! `OpenThread` above rests on. Both statements cannot hold of one thread.
//!
//! The disposition does NOT rest on that openability claim, deliberately. It
//! rests on the enumeration alone: a `CreateToolhelp32Snapshot` lists every
//! thread currently attached to this process, so if the id is absent from a
//! snapshot taken AFTER the failed open, then no thread of ours that was alive
//! at the moment of that open still exists. Our thread with that id is gone,
//! whatever the error code said and whoever holds the id now. If the id IS
//! still ours, the freeze refuses -- fail-closed, unchanged.
//!
//! That leaves one window, and it is the same window the
//! `ERROR_INVALID_PARAMETER` arm has always had: the kernel may hand the freed
//! id straight back to a NEW thread of ours after the re-enumeration. Such a
//! thread is genuinely new, it is counted as a vanish rather than as a
//! suspension, [`pass_is_quiet`] therefore cannot end the loop on this pass,
//! and the next pass enumerates it and suspends it like any other.
//!
//! NOT true of an id we already hold, and this is a known hole rather than a
//! covered case: membership in the frozen set is keyed on the thread id alone.
//! A suspended thread cannot exit by itself, so the only way to reach the hole
//! is for something to call `TerminateThread` on a thread we hold suspended
//! and for the kernel to recycle that id onto a new thread before the next
//! pass -- at which point the new thread is skipped as already-frozen and the
//! patch lands while it runs. Bounded, not eliminated: `hoi4.exe` does not
//! name `TerminateThread` anywhere (exact-a729 string scan; `SuspendThread`
//! and `ResumeThread` DO resolve in the same scan, at 0x14306b3f0 and
//! 0x14306b400, so the method does find import names). Loaded DLLs and
//! dynamically resolved names are not covered by that. Closing it means
//! testing each held handle with a zero-timeout wait before trusting the
//! entry.
//!
//! A thread created by ANOTHER process between the last pass and the patch
//! (`CreateRemoteThread`) is in no snapshot at all, and no user-mode snapshot
//! fixed point can see it. `hoi4.exe` does not name `CreateRemoteThread`
//! either, but this workspace's own injector does exactly that at attach.
//!
//! # Why it iterates
//!
//! A thread created after the enumeration is not in it. The loop therefore
//! re-enumerates until a pass finds nothing new; at that point every live
//! thread is suspended, and because only this thread is running and it creates
//! no threads, the set cannot grow again. Failing to reach that fixed point
//! within [`MAX_PASSES`] refuses.
//!
//! # Cost, and the hazard this trades for
//!
//! STATED PLAINLY because it is a real trade and not a free win: holding the
//! freeze across the MinHook call means MinHook's own
//! `CreateToolhelp32Snapshot` -- and this module's verification passes -- run
//! while every other thread is suspended. If any of those allocate from a heap
//! whose lock a suspended thread holds, the process hangs instead of crashing.
//! Today MinHook takes that snapshot with nothing suspended.
//!
//! This is not a new hazard class in this workspace: the four hand-rolled
//! suspenders this module replaced already pushed to a `Vec` (the Rust global
//! allocator, i.e. the process heap) with threads suspended. It is a wider
//! window. Buffers are therefore reserved generously before the first
//! suspension and reused across passes so the steady state allocates nothing
//! while frozen.
//!
//! The recycled-id check below adds one more snapshot inside that window, on
//! the rare path where `OpenThread` is denied. It builds no list -- it answers
//! as it walks -- so it adds no Rust allocation, only the `kernel32` call. A
//! reserved buffer would NOT have been good enough there: the reservation is
//! sized from the thread count at the start of the freeze, and a process
//! churning threads is exactly the one that can exceed it.
//!
//! What would close it rather than narrow it: enumerating with
//! `NtGetNextThread`, which returns handles straight from the kernel and needs
//! no user-mode allocation, and/or `HeapLock` on the process heap before the
//! first suspension. Neither removes MinHook's own snapshot from the frozen
//! window. Whether `CreateToolhelp32Snapshot` allocates from the process
//! default heap was not determined here; settling it needs kernelbase
//! disassembly or a live patched `hoi4.exe`.
//!
//! # Patching without MinHook
//!
//! Four units in this workspace rewrite bytes themselves rather than through
//! MinHook: three of them patch mid-function sites that are not function
//! entries, and one replaces whole prologues with a register-preserving
//! absolute jump. They cannot use [`super::enable`], so they take the freeze
//! directly through [`freeze_process`] and then ask
//! [`ProcessFreeze::any_instruction_pointer_in`] whether any frozen thread is
//! stopped inside the bytes they are about to rewrite. That question is
//! MinHook's `ProcessThreadIPs` restated for a caller that does its own
//! writing -- except that it REFUSES rather than relocating, because these
//! callers replace sequences whose interior is a legitimate branch target and
//! nothing here can prove where such a thread should resume.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    GetThreadContext, CONTEXT, CONTEXT_CONTROL_AMD64,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcessId, GetCurrentThreadId, OpenThread, ResumeThread, SuspendThread,
    WaitForSingleObject, THREAD_GET_CONTEXT, THREAD_QUERY_INFORMATION, THREAD_SET_CONTEXT,
    THREAD_SUSPEND_RESUME,
};

/// `SYNCHRONIZE`. Lives under `Win32::Storage::FileSystem` in `windows-sys`,
/// which this crate does not otherwise need; the value is a standard access
/// right, not a game constant.
const SYNCHRONIZE: u32 = 0x0010_0000;

/// The access mask every thread is opened with.
///
/// MUST remain a superset of MinHook's `THREAD_ACCESS`
/// (`THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION |
/// THREAD_SET_CONTEXT`, `minhook/src/hook.c`). That superset relation is the
/// whole reason a successful freeze lets us say MinHook's own `OpenThread`
/// cannot fail: we already obtained everything it will ask for. `SYNCHRONIZE`
/// is ours, for the termination proof.
pub(crate) const FREEZE_THREAD_ACCESS: u32 = THREAD_SUSPEND_RESUME
    | THREAD_GET_CONTEXT
    | THREAD_QUERY_INFORMATION
    | THREAD_SET_CONTEXT
    | SYNCHRONIZE;

/// MinHook's own `THREAD_ACCESS`, restated so a test can assert the superset
/// relation instead of a reader having to trust the comment above.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const MINHOOK_THREAD_ACCESS: u32 =
    THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION | THREAD_SET_CONTEXT;

/// `OpenThread` reports this when no thread with the given id exists.
const ERROR_INVALID_PARAMETER: u32 = 87;

/// The only benign reason `Thread32Next` stops. Anything else means the list
/// was TRUNCATED, and a truncated list is a list of threads we did not see.
const ERROR_NO_MORE_FILES: u32 = 18;

/// Windows' `MAXIMUM_SUSPEND_COUNT`. `SuspendThread` fails rather than
/// incrementing past it.
const MAXIMUM_SUSPEND_COUNT: u32 = 127;

/// How many suspensions still have to fit after ours.
///
/// Exactly one: MinHook's `Freeze()` suspends every thread a second time,
/// inside the window we hold. Ours is not the last one that has to succeed.
const SUSPENSIONS_STILL_TO_COME: u32 = 1;

/// How long a suspend failure is given to resolve into a proven termination.
///
/// A thread that has entered `NtTerminateThread` reports ERROR_ACCESS_DENIED
/// from `SuspendThread` BEFORE its thread object is signalled, so a zero
/// timeout reads a thread that is already past all user-mode code as one that
/// might still be running. Measured: a zero-timeout probe saw every failure
/// signalled, and a 200-round test still hit the gap. The wait is bounded so a
/// thread that is genuinely unsuspendable refuses instead of blocking; it
/// costs nothing in the ordinary case, where the object signals immediately.
const TERMINATION_PROOF_TIMEOUT_MS: u32 = 50;

/// How many enumeration passes may run before the freeze is refused.
///
/// Reaching a fixed point takes two passes in a process that creates no
/// threads. The headroom covers a burst of thread creation racing the first
/// pass; a process that never settles is refused rather than retried forever.
pub(crate) const MAX_PASSES: u32 = 16;

/// Entered [`freeze_other_threads`], whatever the outcome.
static FREEZE_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
/// Left [`freeze_other_threads`] without a freeze, so nothing was patched.
static FREEZE_REFUSALS: AtomicU64 = AtomicU64::new(0);
/// How many times a `SuspendThread` failure was excused because the thread was
/// PROVEN terminated.
///
/// Exists so a test can say the excusal path was REACHED. Without it, a test
/// that merely asserts "no freeze was refused" passes in any run where no
/// thread happened to exit mid-enumeration, which is most runs on an idle
/// machine -- a guard that cannot tell "correct" from "never tried". Carried
/// over from `fired_event_popups_live`, which is where it was first needed.
static SUSPEND_FAILURES_EXCUSED_AS_TERMINATED: AtomicU64 = AtomicU64::new(0);
/// How many of those excusals needed the bounded wait -- the thread object was
/// NOT signalled at the instant the suspend was refused, and only became so a
/// moment later.
///
/// This is the population [`TERMINATION_PROOF_TIMEOUT_MS`] exists for.
/// Counting it separately is what lets a test prove the wait is load-bearing
/// instead of asserting that a constant is non-zero: drop the timeout to 0 and
/// this counter stops moving while the other one keeps going.
static SUSPEND_FAILURES_NEEDING_THE_WAIT: AtomicU64 = AtomicU64::new(0);
/// How many times an `OpenThread` failure that was NOT "no such thread id" was
/// excused because a fresh enumeration of this process no longer listed the
/// id.
///
/// This is the recycled-id population the module header describes, and it
/// exists for the same reason the two counters above do: so a test can say the
/// path was REACHED rather than merely not-refused. Every occurrence measured
/// here reported `ERROR_ACCESS_DENIED`.
static OPEN_FAILURES_EXCUSED_AS_NO_LONGER_OURS: AtomicU64 = AtomicU64::new(0);

std::thread_local! {
    /// Per-thread copy of [`FREEZE_ATTEMPTS`].
    ///
    /// The process-wide counter cannot say "this call froze exactly once"
    /// while other threads are freezing too, and the ordering guards on the
    /// patch wrappers need exactly that. Patching is single-threaded per
    /// operation, so the caller's own count answers the question without a
    /// lock and without a test-only code path in the freeze itself.
    static THREAD_FREEZE_ATTEMPTS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many times a freeze was attempted for a detour patch. Diagnostic, and
/// the observable that lets a test say the freeze ran at all.
pub fn freeze_attempts() -> u64 {
    FREEZE_ATTEMPTS.load(Ordering::Acquire)
}

/// How many freezes THIS thread has attempted. Exact under concurrency, which
/// [`freeze_attempts`] is not.
pub fn freeze_attempts_on_this_thread() -> u64 {
    THREAD_FREEZE_ATTEMPTS.with(|count| count.get())
}

/// How many of those attempts refused, leaving the target untouched.
pub fn freeze_refusals() -> u64 {
    FREEZE_REFUSALS.load(Ordering::Acquire)
}

/// How many `SuspendThread` failures were excused by proof of termination.
pub fn suspend_failures_excused_as_terminated() -> u64 {
    SUSPEND_FAILURES_EXCUSED_AS_TERMINATED.load(Ordering::Acquire)
}

/// How many of those needed [`TERMINATION_PROOF_TIMEOUT_MS`] to elapse rather
/// than being signalled the instant the suspend was refused.
pub fn suspend_failures_needing_the_termination_wait() -> u64 {
    SUSPEND_FAILURES_NEEDING_THE_WAIT.load(Ordering::Acquire)
}

/// How many `OpenThread` failures were excused because the id had left this
/// process by the time it was re-enumerated -- the recycled-id case.
pub fn open_failures_excused_as_no_longer_ours() -> u64 {
    OPEN_FAILURES_EXCUSED_AS_NO_LONGER_OURS.load(Ordering::Acquire)
}

/// Serialises every test in this crate that touches process-global suspend
/// state.
///
/// [`FREEZE_LOCK`] serialises freezes against each other, but not against a
/// test that ARRANGES state a later freeze will observe. The ceiling test
/// raises one thread's suspend count to 126 for a moment; any concurrent
/// freeze -- including the ones `lib.rs` drives through `enable`/`disable` --
/// then sees a thread it cannot make room for and correctly refuses, turning
/// this test's setup into another test's failure. Both modules take this.
#[cfg(test)]
pub(crate) static OS_SUSPEND_STATE_TESTS: Mutex<()> = Mutex::new(());

/// Take [`OS_SUSPEND_STATE_TESTS`], ignoring poisoning: a test that panicked
/// while holding it has already been reported, and refusing every later test
/// on that basis reports the same failure many times over.
#[cfg(test)]
pub(crate) fn os_suspend_state_guard() -> MutexGuard<'static, ()> {
    OS_SUSPEND_STATE_TESTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Why the process could not be frozen, and therefore why nothing was patched.
///
/// Every variant carries the OS error, because "could not suspend thread N" on
/// its own does not distinguish a thread that exited from one this process may
/// not touch, and those need different operator responses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreezeRefusal {
    /// `CreateToolhelp32Snapshot` failed.
    Snapshot(u32),
    /// `Thread32First` failed, so no thread list was obtained at all.
    Enumeration(u32),
    /// A thread could not be opened, the failure was not "no such thread id",
    /// and the id was still this process's own on re-enumeration.
    ThreadOpen { thread: u32, os_error: u32 },
    /// A thread could not be suspended and could not be proven terminated.
    ThreadSuspend { thread: u32, os_error: u32 },
    /// A thread was suspended, but too close to Windows' maximum suspend
    /// count for MinHook's own suspension to succeed on top of ours.
    SuspendCeiling { thread: u32, previous_count: u32 },
    /// Threads kept appearing; the process never went quiet.
    NotQuiescent { passes: u32, still_running: u32 },
    /// A frozen thread's context could not be read, so it could not be proven
    /// outside the bytes about to be rewritten. Only reachable through
    /// [`ProcessFreeze::any_instruction_pointer_in`].
    ThreadContext { thread: u32, os_error: u32 },
}

impl fmt::Display for FreezeRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Snapshot(code) => write!(
                f,
                "detour patch refused: thread snapshot failed (os error {code}); \
                 no target byte was touched"
            ),
            Self::Enumeration(code) => write!(
                f,
                "detour patch refused: thread enumeration failed (os error {code}); \
                 no target byte was touched"
            ),
            Self::ThreadOpen { thread, os_error } => write!(
                f,
                "detour patch refused: thread {thread} could not be opened \
                 (os error {os_error}) and is still this process's own, so it \
                 could not be proven outside the patch site; no target byte \
                 was touched"
            ),
            Self::ThreadSuspend { thread, os_error } => write!(
                f,
                "detour patch refused: thread {thread} could not be suspended \
                 (os error {os_error}) and is not terminated, so it may be \
                 executing inside the patch site; no target byte was touched"
            ),
            Self::SuspendCeiling {
                thread,
                previous_count,
            } => write!(
                f,
                "detour patch refused: thread {thread} was already suspended \
                 {previous_count} times, leaving no room for MinHook's own \
                 suspension on top of ours -- which would fail, and MinHook \
                 patches anyway when it does; no target byte was touched"
            ),
            Self::NotQuiescent {
                passes,
                still_running,
            } => write!(
                f,
                "detour patch refused: {still_running} thread(s) were still \
                 starting after {passes} passes, so the process never went \
                 quiet; no target byte was touched"
            ),
            Self::ThreadContext { thread, os_error } => write!(
                f,
                "detour patch refused: the context of frozen thread {thread} \
                 could not be read (os error {os_error}), so it could not be \
                 proven outside the patch site; no target byte was touched"
            ),
        }
    }
}

impl std::error::Error for FreezeRefusal {}

/// What a pass decided about one enumerated thread id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ThreadDisposition {
    /// The thread is provably not executing and needs nothing further.
    Gone,
    /// The thread may be executing and could not be stopped. FAIL CLOSED.
    Refuse,
}

/// What an `OpenThread` failure says ON ITS OWN, before anything is
/// re-enumerated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpenFailure {
    /// The kernel has NO thread with this id anywhere in the system, which is
    /// what `ERROR_INVALID_PARAMETER` means. The id in our snapshot is stale
    /// and nothing further needs asking.
    NoSuchThreadId,
    /// Some other code. The kernel found *a* thread with this id and would not
    /// give us a handle to it. Because ids are system-wide that thread need
    /// not be ours, so this decides NOTHING by itself -- see
    /// [`classify_unopenable_thread`].
    Unopenable,
}

/// `OpenThread` failed. Which of the two questions does the code answer?
///
/// Deliberately NOT a disposition. The old form of this function mapped every
/// code but `ERROR_INVALID_PARAMETER` -- explicitly including
/// `ERROR_ACCESS_DENIED` -- straight to a refusal, on the reading that such a
/// thread "may well be running". Measured, that reading was wrong about which
/// PROCESS the thread belongs to: see the module header. The code alone cannot
/// settle it, so this function no longer pretends to.
pub(crate) fn classify_open_failure(os_error: u32) -> OpenFailure {
    if os_error == ERROR_INVALID_PARAMETER {
        OpenFailure::NoSuchThreadId
    } else {
        OpenFailure::Unopenable
    }
}

/// The kernel has a thread with this id and would not let us open it. Is that
/// thread ours?
///
/// `id_is_still_ours` must come from an enumeration of THIS PROCESS taken
/// AFTER the failed open, because that ordering is the whole proof: a snapshot
/// lists every thread attached to the process at the moment it is taken, so an
/// id missing from it belongs to no thread of ours that was alive at the
/// earlier open. That thread is gone. An id still present is a live thread of
/// ours that we could not stop, and refuses.
///
/// FAIL CLOSED in the direction that matters: this excuses an open failure
/// only on positive evidence that the id left this process, never on the error
/// code, and never on a snapshot taken before the failure.
pub(crate) fn classify_unopenable_thread(id_is_still_ours: bool) -> ThreadDisposition {
    if id_is_still_ours {
        ThreadDisposition::Refuse
    } else {
        ThreadDisposition::Gone
    }
}

/// Did `Thread32Next` stop because the list ended, or because it broke?
///
/// `ERROR_NO_MORE_FILES` is the documented terminal result. Reading any other
/// stop as end-of-list certifies a TRUNCATED enumeration -- the threads past
/// the break are never suspended, and the fixed point then concludes the
/// process is quiet while they run.
pub(crate) fn enumeration_stop_is_end_of_list(os_error: u32) -> bool {
    os_error == ERROR_NO_MORE_FILES
}

/// Does our suspension leave room for the one MinHook still has to take?
///
/// `SuspendThread` returns the target's PREVIOUS suspend count and fails
/// rather than increment past `MAXIMUM_SUSPEND_COUNT`. A successful suspension
/// of ours is therefore not enough: MinHook's `Freeze()` suspends the same
/// thread again inside the window we hold, and when THAT fails MinHook strikes
/// the thread off its resume list, skips `ProcessThreadIPs`, and rewrites the
/// bytes anyway. Landing exactly on the ceiling hands MinHook its fail-open
/// branch on a thread that is genuinely live -- the one case this whole module
/// exists to make unreachable.
pub(crate) fn leaves_room_for_minhooks_suspension(previous_count: u32) -> bool {
    previous_count
        .saturating_add(1)
        .saturating_add(SUSPENSIONS_STILL_TO_COME)
        <= MAXIMUM_SUSPEND_COUNT
}

/// Is a pass allowed to conclude the process is quiet?
///
/// ONLY when it saw nothing outside the set it already holds suspended. A
/// thread that vanished mid-pass is not a free pass: it was running when the
/// snapshot was taken, so it may have started a successor that the snapshot
/// cannot contain, and that successor would never be enumerated if this pass
/// were allowed to be the last. Counting only newly SUSPENDED threads misses
/// exactly that case.
///
/// This still terminates. Once every live thread is suspended, no thread runs
/// to create another and no suspended thread can exit, so the vanished count
/// falls to zero as the kernel drops the terminated entries from its list.
pub(crate) fn pass_is_quiet(newly_suspended: u32, vanished: u32) -> bool {
    newly_suspended == 0 && vanished == 0
}

/// `SuspendThread` failed on a handle we hold.
///
/// The error code is deliberately NOT consulted. The question is not which
/// code Windows chose, it is whether the thread can still execute, and the
/// thread object being signalled answers that directly and without guessing.
pub(crate) fn classify_suspend_failure(signalled: bool) -> ThreadDisposition {
    if signalled {
        ThreadDisposition::Gone
    } else {
        ThreadDisposition::Refuse
    }
}

/// Serialises freezing, PROCESS-WIDE and not merely per patch operation.
///
/// Two threads freezing concurrently deadlock with certainty, not by
/// coincidence: each enumerates the other, each suspends the other partway
/// through, and neither ever reaches its resume. This was observed, not
/// reasoned about -- the first run of this module's own tests hung exactly
/// this way, with two test threads freezing at once.
///
/// The lock therefore belongs to the freeze itself rather than to the patch
/// wrappers above it, so that no future caller can reach a freeze without it.
/// A thread parked on this mutex is safe to suspend: it holds nothing the
/// holder needs, and the holder never waits on anything a suspended thread
/// owns.
static FREEZE_LOCK: Mutex<()> = Mutex::new(());

/// Every other thread in this process, suspended until this value is dropped.
///
/// # Safety obligation of every holder
///
/// Drop it promptly. Every other thread in the process is stopped for as long
/// as it lives, so anything that waits on another thread -- a `join`, a
/// channel receive, a lock another thread holds -- deadlocks the process with
/// certainty rather than by chance.
pub struct ProcessFreeze {
    /// Held for the lifetime of the freeze. Declared first so that, after
    /// `Drop::drop` below has resumed every thread, it is released last.
    _serialisation: MutexGuard<'static, ()>,
    /// `(thread id, handle)` for each thread this freeze suspended.
    frozen: Vec<(u32, HANDLE)>,
    /// Enumeration passes it took to reach the fixed point.
    passes: u32,
}

/// `CONTEXT` at the alignment `GetThreadContext` requires.
///
/// NOT decoration. `windows-sys` declares the AMD64 `CONTEXT` as plain
/// `#[repr(C)]`, so Rust is free to give it 8-byte alignment; the kernel
/// rejects a misaligned buffer with `ERROR_NOACCESS` (998) and reads nothing.
/// Measured here: without this wrapper every `GetThreadContext` in this module
/// failed with 998, which -- because the read fails closed -- turned every
/// patch into a refusal. Each of the four hand-rolled suspenders this module
/// replaced declared its own `#[repr(C, align(16))]` context for the same
/// reason.
#[repr(C, align(16))]
struct AlignedContext(CONTEXT);

const _: () = assert!(core::mem::align_of::<AlignedContext>() == 16);

impl ProcessFreeze {
    /// How many threads are held suspended.
    pub fn frozen_thread_count(&self) -> usize {
        self.frozen.len()
    }

    /// How many enumeration passes the fixed point took. Two is the quiet-
    /// process answer: one to suspend, one to observe nothing new.
    pub fn passes(&self) -> u32 {
        self.passes
    }

    /// Is any frozen thread stopped inside one of `ranges` (`[start, end)`)?
    ///
    /// The question a caller that writes its own bytes has to ask before it
    /// writes them. Suspension alone is not enough: a thread parked inside the
    /// sequence being replaced resumes into a half-old, half-new instruction
    /// stream, which is the crash this whole module exists to remove.
    ///
    /// FAIL CLOSED: a context that cannot be read is
    /// [`FreezeRefusal::ThreadContext`], never "not in the range". The answer
    /// is trustworthy only because the freeze is held -- every thread in
    /// `frozen` is suspended, so its context is stable and cannot move between
    /// this read and the caller's write.
    ///
    /// Threads accounted for as PROVEN GONE are absent from `frozen` and are
    /// correctly not consulted: a terminated thread executes no further
    /// user-mode instruction, so it cannot be inside the range.
    pub fn any_instruction_pointer_in(
        &self,
        ranges: &[(usize, usize)],
    ) -> Result<bool, FreezeRefusal> {
        for &(thread_id, handle) in &self.frozen {
            let mut context: AlignedContext = unsafe { std::mem::zeroed() };
            context.0.ContextFlags = CONTEXT_CONTROL_AMD64;
            if unsafe { GetThreadContext(handle, &mut context.0) } == 0 {
                return Err(FreezeRefusal::ThreadContext {
                    thread: thread_id,
                    os_error: unsafe { GetLastError() },
                });
            }
            let rip = context.0.Rip as usize;
            if ranges.iter().any(|(start, end)| rip >= *start && rip < *end) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl Drop for ProcessFreeze {
    fn drop(&mut self) {
        // Resume in reverse order of suspension, and resume every thread even
        // if one resume fails: a thread left suspended is a hung game.
        for (_, handle) in self.frozen.drain(..).rev() {
            unsafe {
                ResumeThread(handle);
                CloseHandle(handle);
            }
        }
    }
}

/// Snapshot this process's thread ids into `out`, replacing its contents.
///
/// Reuses `out`'s capacity so a verification pass taken while threads are
/// suspended does not allocate.
fn enumerate_thread_ids(out: &mut Vec<u32>) -> Result<(), FreezeRefusal> {
    out.clear();
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(FreezeRefusal::Snapshot(unsafe { GetLastError() }));
    }
    let process_id = unsafe { GetCurrentProcessId() };
    let current_thread_id = unsafe { GetCurrentThreadId() };
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = core::mem::size_of::<THREADENTRY32>() as u32;
    if unsafe { Thread32First(snapshot, &mut entry) } == 0 {
        let code = unsafe { GetLastError() };
        unsafe { CloseHandle(snapshot) };
        return Err(FreezeRefusal::Enumeration(code));
    }
    loop {
        if entry.th32OwnerProcessID == process_id && entry.th32ThreadID != current_thread_id {
            out.push(entry.th32ThreadID);
        }
        if unsafe { Thread32Next(snapshot, &mut entry) } == 0 {
            // Treating any stop as end-of-list would certify a partial
            // enumeration: the threads past the truncation point would never
            // be suspended, and the fixed point below would happily conclude
            // the process was quiet.
            let code = unsafe { GetLastError() };
            unsafe { CloseHandle(snapshot) };
            return if enumeration_stop_is_end_of_list(code) {
                Ok(())
            } else {
                Err(FreezeRefusal::Enumeration(code))
            };
        }
    }
}

/// Is `thread_id` still attached to THIS process, right now?
///
/// The evidence [`classify_unopenable_thread`] runs on, and the reason it is a
/// separate function is that the ordering is the proof: every caller must take
/// this AFTER the `OpenThread` it is explaining, never before.
///
/// It ALLOCATES NOTHING, and answers from the snapshot as it walks rather than
/// calling [`enumerate_thread_ids`] to materialise a list. That is not a
/// micro-optimisation: this runs inside the frozen window, and a `Vec` that
/// outgrows a reservation there takes the Rust global allocator's lock, which
/// a suspended thread may be holding -- the hang the module header describes.
/// A reservation could only ever be a bound on the thread count, and the churn
/// this exists to survive is precisely a process whose thread count is moving.
/// Not materialising the list removes the question instead of bounding it.
///
/// It does still take a `CreateToolhelp32Snapshot` inside the frozen window,
/// and that is unavoidable: fresh evidence is the whole point. Whether that
/// call allocates from the process default heap is undetermined, exactly as
/// the module header already records for the per-pass enumerations.
///
/// FAIL CLOSED: a snapshot that cannot be taken, or an enumeration that stops
/// partway, propagates as a refusal rather than answering "not ours". A
/// truncated list is a list of threads we did not see, and reading one as
/// proof that a thread left the process is exactly backwards. A POSITIVE match
/// is conclusive on its own and returns early, because finding the thread does
/// not depend on having seen the rest of the list.
fn id_is_still_in_this_process(thread_id: u32) -> Result<bool, FreezeRefusal> {
    if thread_id == unsafe { GetCurrentThreadId() } {
        // Cannot arise from the freeze loop, whose enumeration excludes this
        // thread, but answering "not in this process" about the thread doing
        // the asking would be absurd for any future caller.
        return Ok(true);
    }
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(FreezeRefusal::Snapshot(unsafe { GetLastError() }));
    }
    let process_id = unsafe { GetCurrentProcessId() };
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = core::mem::size_of::<THREADENTRY32>() as u32;
    if unsafe { Thread32First(snapshot, &mut entry) } == 0 {
        let code = unsafe { GetLastError() };
        unsafe { CloseHandle(snapshot) };
        return Err(FreezeRefusal::Enumeration(code));
    }
    loop {
        if entry.th32OwnerProcessID == process_id && entry.th32ThreadID == thread_id {
            unsafe { CloseHandle(snapshot) };
            return Ok(true);
        }
        if unsafe { Thread32Next(snapshot, &mut entry) } == 0 {
            let code = unsafe { GetLastError() };
            unsafe { CloseHandle(snapshot) };
            // Only a list that ran to its documented end can support "absent".
            // Absence from a TRUNCATED list is not absence, and excusing an
            // unopenable id on it would excuse a live thread of ours.
            return if enumeration_stop_is_end_of_list(code) {
                Ok(false)
            } else {
                Err(FreezeRefusal::Enumeration(code))
            };
        }
    }
}

/// Has this thread terminated, given a bounded moment to finish doing so?
///
/// Probed in two steps rather than one so the bounded wait's own population is
/// countable. The threads that are ALREADY signalled when the suspend is
/// refused would be excused by a zero timeout too; the ones that are not are
/// the entire reason [`TERMINATION_PROOF_TIMEOUT_MS`] is non-zero, and
/// [`SUSPEND_FAILURES_NEEDING_THE_WAIT`] is how a test proves that population
/// is non-empty instead of asserting that a constant is non-zero. The total
/// time bound is unchanged: nothing is waited on for longer than
/// `TERMINATION_PROOF_TIMEOUT_MS`.
fn thread_has_terminated(handle: HANDLE) -> bool {
    if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 {
        return true;
    }
    let signalled_after_the_wait =
        unsafe { WaitForSingleObject(handle, TERMINATION_PROOF_TIMEOUT_MS) } == WAIT_OBJECT_0;
    if signalled_after_the_wait {
        SUSPEND_FAILURES_NEEDING_THE_WAIT.fetch_add(1, Ordering::AcqRel);
    }
    signalled_after_the_wait
}

/// THE freeze, for a caller that rewrites bytes without MinHook.
///
/// [`enable`](super::enable), [`disable`](super::disable) and
/// [`remove`](super::remove) take this freeze themselves and are the right
/// entry points whenever MinHook owns the patch. This one exists for the units
/// that cannot use MinHook at all -- mid-function sites that are not function
/// entries, and whole-prologue replacements -- so that "how the process is
/// frozen" has exactly one implementation in the workspace whichever way the
/// bytes are written.
///
/// A caller that takes this owes two things MinHook would otherwise have done
/// for it: ask [`ProcessFreeze::any_instruction_pointer_in`] about every range
/// it is about to rewrite, and hold the returned guard across the write, the
/// protection restore AND the instruction-cache flush.
///
/// FAIL CLOSED: on `Err` nothing is suspended and the caller must not patch.
pub fn freeze_process() -> Result<ProcessFreeze, FreezeRefusal> {
    freeze_other_threads()
}

/// Suspend every other thread in this process, or refuse.
///
/// FAIL CLOSED: on `Err` nothing is suspended (any thread stopped on the way
/// to the failure has been resumed) and the caller must not patch.
///
/// # Safety
///
/// Held suspensions must be released promptly. The returned guard does that on
/// drop; leaking it hangs the process.
pub(crate) fn freeze_other_threads() -> Result<ProcessFreeze, FreezeRefusal> {
    FREEZE_ATTEMPTS.fetch_add(1, Ordering::AcqRel);
    THREAD_FREEZE_ATTEMPTS.with(|count| count.set(count.get().saturating_add(1)));
    match freeze_other_threads_inner() {
        Ok(freeze) => Ok(freeze),
        Err(refusal) => {
            FREEZE_REFUSALS.fetch_add(1, Ordering::AcqRel);
            Err(refusal)
        }
    }
}

fn freeze_other_threads_inner() -> Result<ProcessFreeze, FreezeRefusal> {
    // Taken before the first enumeration: a competing freezer must be either
    // entirely before us or entirely after us, never interleaved.
    let serialisation = FREEZE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let mut ids: Vec<u32> = Vec::new();
    enumerate_thread_ids(&mut ids)?;

    // Reserve before the first suspension. Every later pass reuses this
    // capacity, so the frozen window does not allocate in the steady state.
    let headroom = ids.len().saturating_mul(4).saturating_add(64);
    ids.reserve(headroom);
    let mut freeze = ProcessFreeze {
        _serialisation: serialisation,
        frozen: Vec::with_capacity(headroom),
        passes: 0,
    };

    for pass in 1..=MAX_PASSES {
        freeze.passes = pass;
        if pass > 1 {
            // A verification pass. `ids` keeps its reserved capacity.
            enumerate_thread_ids(&mut ids)?;
        }
        let mut newly_suspended = 0u32;
        let mut vanished = 0u32;
        for &thread_id in &ids {
            if freeze.frozen.iter().any(|(id, _)| *id == thread_id) {
                continue;
            }
            // Not already suspended by us: either it is new, or its id is
            // stale, or its id was recycled onto a live thread. All three are
            // decided from scratch, right now.
            let handle = unsafe { OpenThread(FREEZE_THREAD_ACCESS, 0, thread_id) };
            if handle.is_null() {
                let os_error = unsafe { GetLastError() };
                let disposition = match classify_open_failure(os_error) {
                    // The kernel has no thread with this id at all.
                    OpenFailure::NoSuchThreadId => ThreadDisposition::Gone,
                    // The kernel has one and would not open it for us. Ids are
                    // system-wide, so ask whether it is still OURS -- and ask
                    // AFTER the failed open, which is what makes the answer
                    // proof rather than a guess.
                    OpenFailure::Unopenable => {
                        let still_ours = id_is_still_in_this_process(thread_id)?;
                        if !still_ours {
                            OPEN_FAILURES_EXCUSED_AS_NO_LONGER_OURS.fetch_add(1, Ordering::AcqRel);
                        }
                        classify_unopenable_thread(still_ours)
                    }
                };
                match disposition {
                    ThreadDisposition::Gone => {
                        vanished += 1;
                        continue;
                    }
                    ThreadDisposition::Refuse => {
                        return Err(FreezeRefusal::ThreadOpen {
                            thread: thread_id,
                            os_error,
                        })
                    }
                }
            }
            let previous_count = unsafe { SuspendThread(handle) };
            if previous_count != u32::MAX
                && !leaves_room_for_minhooks_suspension(previous_count)
            {
                // Our suspension succeeded and MinHook's would not. Undo ours
                // and refuse: proceeding would hand MinHook its fail-open
                // branch on a thread that is provably still live.
                unsafe {
                    ResumeThread(handle);
                    CloseHandle(handle);
                }
                return Err(FreezeRefusal::SuspendCeiling {
                    thread: thread_id,
                    previous_count,
                });
            }
            if previous_count == u32::MAX {
                let os_error = unsafe { GetLastError() };
                let disposition = classify_suspend_failure(thread_has_terminated(handle));
                unsafe { CloseHandle(handle) };
                match disposition {
                    ThreadDisposition::Gone => {
                        SUSPEND_FAILURES_EXCUSED_AS_TERMINATED.fetch_add(1, Ordering::AcqRel);
                        vanished += 1;
                        continue;
                    }
                    ThreadDisposition::Refuse => {
                        return Err(FreezeRefusal::ThreadSuspend {
                            thread: thread_id,
                            os_error,
                        })
                    }
                }
            }
            freeze.frozen.push((thread_id, handle));
            newly_suspended += 1;
        }
        if pass_is_quiet(newly_suspended, vanished) {
            // A whole pass found nothing outside the suspended set. Only this
            // thread runs now, and it starts no threads, so the set is closed.
            return Ok(freeze);
        }
    }

    let still_running = {
        let mut count = 0u32;
        for &thread_id in &ids {
            if !freeze.frozen.iter().any(|(id, _)| *id == thread_id) {
                count += 1;
            }
        }
        count
    };
    Err(FreezeRefusal::NotQuiescent {
        passes: MAX_PASSES,
        still_running,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::Arc;

    /// Falsifiable: fails if the mask this module opens threads with ever stops
    /// covering everything MinHook will ask for.
    ///
    /// The superset relation is load-bearing, not cosmetic. It is the only
    /// reason a successful freeze permits the claim that MinHook's own
    /// `OpenThread` cannot fail; drop `THREAD_SET_CONTEXT` from our mask and
    /// MinHook could still be refused a handle for a thread we suspended,
    /// putting its fail-open branch back in reach.
    #[test]
    fn the_freeze_access_mask_covers_everything_minhook_asks_for() {
        assert_eq!(
            MINHOOK_THREAD_ACCESS & FREEZE_THREAD_ACCESS,
            MINHOOK_THREAD_ACCESS,
            "the freeze mask must be a superset of MinHook's THREAD_ACCESS"
        );
        assert_ne!(
            FREEZE_THREAD_ACCESS & SYNCHRONIZE,
            0,
            "SYNCHRONIZE is required for the termination proof"
        );
    }

    /// Falsifiable: fails if an `OpenThread` error code is ever taken as a
    /// disposition in its own right.
    ///
    /// Only `ERROR_INVALID_PARAMETER` settles anything by itself, and what it
    /// settles is that no thread anywhere has the id. Every other code --
    /// `ERROR_ACCESS_DENIED` above all -- means the kernel HAS a thread with
    /// this id and refused us a handle, which says nothing about whose thread
    /// it is, so it must arrive at [`classify_unopenable_thread`] undecided.
    #[test]
    fn only_a_missing_thread_id_is_settled_by_the_open_error_code_alone() {
        assert_eq!(
            classify_open_failure(ERROR_INVALID_PARAMETER),
            OpenFailure::NoSuchThreadId
        );
        for os_error in [0u32, 5, 6, 8, 87 + 1, 1450] {
            assert_eq!(
                classify_open_failure(os_error),
                OpenFailure::Unopenable,
                "os error {os_error} cannot be settled by its code alone"
            );
        }
    }

    /// Falsifiable, and the guard on the whole recycled-id disposition: fails
    /// if an unopenable id is ever excused while it is still this process's
    /// own.
    ///
    /// The excusal is sound only because it rests on the ENUMERATION, not on
    /// the error code: an id absent from a snapshot of this process taken
    /// after the failed open belongs to no thread of ours that was alive at
    /// that open. Invert this and a live thread of ours that we could not stop
    /// is waved through, which is the MinHook fail-open the module exists to
    /// close, reintroduced one step earlier.
    #[test]
    fn an_unopenable_id_is_excused_only_when_it_has_left_this_process() {
        assert_eq!(
            classify_unopenable_thread(true),
            ThreadDisposition::Refuse,
            "an id that is still ours is a live thread we could not stop"
        );
        assert_eq!(
            classify_unopenable_thread(false),
            ThreadDisposition::Gone,
            "an id that has left this process cannot name a running thread of ours"
        );
    }

    /// Falsifiable, and the one that observes the real OS behind
    /// [`classify_unopenable_thread`]: fails if
    /// [`id_is_still_in_this_process`] stops distinguishing this process's
    /// threads from everyone else's.
    ///
    /// Three shapes, and all three are needed. A live thread of ours must read
    /// TRUE -- a predicate stuck on `false` would excuse every unopenable id,
    /// which is the fail-open. A thread of ANOTHER process must read FALSE:
    /// that is the recycled-id case the fix turns on, and it is what a
    /// predicate stuck on `true` (or one that forgot the owner-pid filter and
    /// walked the whole system list) fails. An id that belongs to nobody must
    /// read FALSE too.
    ///
    /// The foreign id is taken from a live system-wide snapshot rather than
    /// invented, because the point is that such ids are indistinguishable from
    /// ours by value alone.
    ///
    /// None of the three answers can be vacuous: a snapshot that could not be
    /// taken, or an enumeration that stopped early, is an `Err`, and every
    /// assertion here is against an `Ok`.
    #[test]
    fn an_id_is_this_processs_own_only_while_the_thread_is() {
        let _serialised = os_suspend_state_guard();

        let stop = Arc::new(AtomicBool::new(false));
        let ours = Arc::new(AtomicU64::new(0));
        let worker = {
            let stop = Arc::clone(&stop);
            let ours = Arc::clone(&ours);
            std::thread::spawn(move || {
                ours.store(
                    u64::from(unsafe { GetCurrentThreadId() }),
                    Ordering::Release,
                );
                while !stop.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            })
        };
        while ours.load(Ordering::Acquire) == 0 {
            std::thread::yield_now();
        }
        let our_worker = ours.load(Ordering::Acquire) as u32;
        assert_eq!(
            id_is_still_in_this_process(our_worker),
            Ok(true),
            "a live thread of this process must read as ours"
        );

        let foreign = a_thread_id_belonging_to_another_process()
            .expect("some other process on this machine has a thread");
        assert_ne!(foreign, our_worker);
        assert_eq!(
            id_is_still_in_this_process(foreign),
            Ok(false),
            "a thread of another process must not read as ours merely because \
             thread ids are drawn from one system-wide space"
        );

        // Ids are allocated in multiples of four, so an odd one names nothing.
        assert_eq!(id_is_still_in_this_process(u32::MAX), Ok(false));

        stop.store(true, Ordering::Release);
        worker.join().expect("worker joins");
    }

    /// Falsifiable: fails if the recycled-id predicate starts building a list.
    ///
    /// It runs with every other thread in the process suspended. Materialising
    /// a `Vec` there means the Rust global allocator, and a reservation cannot
    /// bound it -- the thread count is exactly what is moving in the churn
    /// this path exists for. If one of the suspended threads holds the heap
    /// lock, the process HANGS rather than crashing, which is the one failure
    /// mode this module cannot detect at runtime. Text, because "did not
    /// allocate" is not observable from a test.
    ///
    /// It also pins the truncation rule for the second enumeration walk in
    /// this module, which the `enumerate_thread_ids` clause above does not
    /// reach: reading a broken walk as "absent" would excuse a live thread.
    ///
    /// WHAT THIS IS NOT, stated because the gap is invisible otherwise: it is
    /// a lexical guard over a named list of allocating constructs, not a proof
    /// of allocation freedom. A future body that allocates by calling some
    /// helper of ours that allocates internally passes this test. Proving the
    /// stronger property needs a `#[global_allocator]` shim that panics while
    /// a freeze is held, which this crate does not have. The list below is
    /// therefore the shapes a future edit is LIKELY to reach for, and its job
    /// is to make the invariant visible at the point of edit.
    #[test]
    fn the_recycled_id_predicate_builds_no_list_while_the_process_is_frozen() {
        let production = production_code();
        let predicate = production
            .split_once("fn id_is_still_in_this_process(")
            .expect("id_is_still_in_this_process is declared in production")
            .1
            .split_once("\n}\n")
            .expect("id_is_still_in_this_process has a closing brace")
            .0;
        for allocating in [
            "Vec", "push(", "to_vec", "collect", "String", "to_string", "format!", "vec!",
            "Box::", "Rc::", "Arc::", "HashSet", "HashMap", "to_owned",
        ] {
            assert!(
                !predicate.contains(allocating),
                "id_is_still_in_this_process must not `{allocating}`; it runs \
                 with every other thread suspended and the allocator's lock \
                 may be held by one of them"
            );
        }
        assert!(
            predicate.contains("enumeration_stop_is_end_of_list(code)"),
            "the predicate's own walk must classify its stop code; reading a \
             truncated list as `absent` excuses a live thread of ours"
        );
    }

    /// A thread id owned by some process other than this one, from a live
    /// system-wide snapshot. `None` only if this machine is running nothing
    /// else, which it is not.
    fn a_thread_id_belonging_to_another_process() -> Option<u32> {
        let process_id = unsafe { GetCurrentProcessId() };
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
        entry.dwSize = core::mem::size_of::<THREADENTRY32>() as u32;
        let mut found = None;
        if unsafe { Thread32First(snapshot, &mut entry) } != 0 {
            loop {
                if entry.th32OwnerProcessID != process_id && entry.th32OwnerProcessID != 0 {
                    found = Some(entry.th32ThreadID);
                    break;
                }
                if unsafe { Thread32Next(snapshot, &mut entry) } == 0 {
                    break;
                }
            }
        }
        unsafe { CloseHandle(snapshot) };
        found
    }

    /// Falsifiable: fails if a suspend failure is ever excused by anything
    /// other than positive proof the thread terminated.
    ///
    /// This is the guard that replaces the error-code allowlists elsewhere in
    /// the tree. Reintroducing one -- excusing `ERROR_INVALID_HANDLE`, say --
    /// means excusing a live thread whose handle we mishandled.
    #[test]
    fn only_a_signalled_thread_excuses_a_suspend_failure() {
        assert_eq!(classify_suspend_failure(true), ThreadDisposition::Gone);
        assert_eq!(classify_suspend_failure(false), ThreadDisposition::Refuse);
    }

    /// Falsifiable: fails if a broken thread enumeration is ever read as a
    /// complete one.
    ///
    /// A truncated list is a list of threads that were never suspended, and
    /// every later step -- the fixed point, the claim that MinHook's own
    /// suspension cannot fail -- is stated over the threads this enumeration
    /// returned. Accepting a truncation quietly makes all of it false.
    #[test]
    fn only_the_documented_end_of_list_ends_an_enumeration() {
        assert!(enumeration_stop_is_end_of_list(ERROR_NO_MORE_FILES));
        for os_error in [0u32, 5, 6, 8, 87, 24, 1450] {
            assert!(
                !enumeration_stop_is_end_of_list(os_error),
                "os error {os_error} is a broken enumeration, not the end of one"
            );
        }
    }

    /// The production text of this module, up to its test module.
    ///
    /// Line endings are normalised because the scans below match
    /// multi-line shapes, and git hands this file back with CRLF on a
    /// fresh checkout. Without this the guards go red on a clean tree
    /// and green on the tree that wrote them -- the worst direction.
    fn production_code() -> String {
        let source = include_str!("freeze.rs").replace("\r\n", "\n");
        source
            .split_once("#[cfg(test)]\nmod tests {")
            .expect("freeze.rs has a test module")
            .0
            .to_string()
    }

    /// Falsifiable, and the reason the classifiers above are not decorative:
    /// fails if a fail-open decision is taken anywhere but through them.
    ///
    /// A mutation battery found both classifier tests survive a reversion at
    /// the CALL SITE -- put `break` back in `enumerate_thread_ids`, or
    /// `newly_suspended == 0` back in the fixed point, and the classifiers
    /// stay green while the freeze fails open again. Neither decision can be
    /// driven from a test: `Thread32Next` cannot be made to fail on demand,
    /// and the vanish-with-a-successor interleaving is not schedulable. So the
    /// call sites are pinned by text rather than left unguarded.
    ///
    /// The recycled-id arm is pinned the same way and for a sharper reason:
    /// its soundness is entirely an ORDERING claim -- the re-enumeration has
    /// to happen after the failed open, in the same iteration, and its result
    /// has to be the thing the disposition is taken from. A mutation that
    /// hoists the re-enumeration, or that maps `Unopenable` straight to
    /// `Gone`, leaves both classifier tests green.
    #[test]
    fn the_fail_open_decisions_are_taken_only_through_their_classifiers() {
        let production = production_code();
        let enumerate = production
            .split_once("fn enumerate_thread_ids(")
            .expect("enumerate_thread_ids is declared in production")
            .1
            // To the closing brace at column 0, and no further. Ending at the
            // next `\nfn ` swept in the FOLLOWING item's doc comment, and this
            // guard then went red because a neighbouring comment used the word
            // `break` in prose.
            .split_once("\n}\n")
            .expect("enumerate_thread_ids has a closing brace")
            .0;
        assert!(
            enumerate.contains("enumeration_stop_is_end_of_list(code)"),
            "enumerate_thread_ids must classify the Thread32Next stop code; \
             ending the walk on any stop certifies a TRUNCATED thread list"
        );
        assert!(
            !enumerate.contains("break"),
            "a bare `break` in enumerate_thread_ids is the truncation fail-open"
        );
        let inner = production
            .split_once("fn freeze_other_threads_inner(")
            .expect("freeze_other_threads_inner is declared in production")
            .1;
        assert!(
            inner.contains("if pass_is_quiet(newly_suspended, vanished) {"),
            "the fixed point must ask pass_is_quiet with BOTH counts; asking \
             about newly_suspended alone lets a pass that saw a thread vanish \
             be the last one"
        );
        assert_eq!(
            inner.matches("vanished += 1;").count(),
            2,
            "both accounted-for-but-not-suspended arms must record the vanish"
        );
        assert!(
            inner.contains("!leaves_room_for_minhooks_suspension(previous_count)"),
            "a suspension that lands on the ceiling must refuse; MinHook's own \
             suspension is the one that then fails, and MinHook patches anyway"
        );

        // The recycled-id arm, in the one order that makes it a proof: the
        // failed `OpenThread`, then the re-enumeration, then the disposition
        // taken from it. Matched on fragments that survive rustfmt rewrapping.
        let opened = inner
            .find("OpenThread(FREEZE_THREAD_ACCESS, 0, thread_id)")
            .expect("the freeze loop must open each enumerated thread");
        let arm = inner
            .find("OpenFailure::Unopenable => {")
            .expect("the freeze loop must handle an unopenable id explicitly");
        let reenumerates = inner.find("id_is_still_in_this_process(thread_id").expect(
            "an unopenable id must be settled by re-enumerating THIS process; \
             the error code cannot say whose thread it is",
        );
        let classifies = inner
            .find("classify_unopenable_thread(still_ours)")
            .expect("the re-enumeration's answer must be the disposition");
        assert!(
            opened < arm && arm < reenumerates && reenumerates < classifies,
            "the re-enumeration must run after the OpenThread it explains and \
             before the disposition is taken from it; any other order proves \
             nothing about whether the id is still ours"
        );
        let arm_body = &inner[arm..classifies];
        assert!(
            !arm_body.contains("ThreadDisposition::Gone"),
            "the Unopenable arm must not reach Gone without the enumeration; \
             excusing an unopenable id on its error code is the fail-open"
        );
    }

    /// Falsifiable: fails if a suspension is allowed to consume the last slot
    /// under Windows' maximum suspend count.
    ///
    /// `SuspendThread` fails rather than increment past
    /// `MAXIMUM_SUSPEND_COUNT`, and MinHook suspends every thread a SECOND
    /// time inside the window we hold. A thread we suspend to exactly the
    /// ceiling is one MinHook cannot suspend -- and MinHook's response to that
    /// is to strike the thread off, skip `ProcessThreadIPs`, and rewrite the
    /// bytes regardless. Ours succeeding is not the question.
    #[test]
    fn a_suspension_must_leave_minhook_a_slot_of_its_own() {
        assert!(leaves_room_for_minhooks_suspension(0));
        assert!(
            leaves_room_for_minhooks_suspension(MAXIMUM_SUSPEND_COUNT - 2),
            "125 -> ours makes 126, MinHook's makes 127, which is the maximum"
        );
        assert!(
            !leaves_room_for_minhooks_suspension(MAXIMUM_SUSPEND_COUNT - 1),
            "126 -> ours makes 127 and MinHook's would be 128, which fails"
        );
        assert!(!leaves_room_for_minhooks_suspension(MAXIMUM_SUSPEND_COUNT));
        assert!(!leaves_room_for_minhooks_suspension(u32::MAX - 1));
    }

    // REMOVED, and the absence is load-bearing, so it is written down: an
    // end-to-end version of the ceiling guard lived here. It parked a thread,
    // raised its suspend count to 126, and asserted `freeze_other_threads()`
    // refused with `SuspendCeiling`. It worked -- the mutation that ignores
    // the ceiling made it fail BY NAME in 0.18s -- but under
    // `cargo test --workspace` it wedged this binary: four threads left
    // Suspended, no thread running the freeze, and the whole suite stopped at
    // 54 of ~200 targets. Taking a REAL process freeze inside a multi-test
    // binary is the allocate-while-frozen hazard this module's own header
    // documents, and a test that can stop the suite is worse than the
    // coverage it buys.
    //
    // The ceiling decision is guarded instead by
    // `a_suspension_must_leave_minhook_a_slot_of_its_own` (the arithmetic) and
    // by the call-site clause in
    // `the_fail_open_decisions_are_taken_only_through_their_classifiers`
    // (that the answer is consulted). Both go red under the mutation that
    // ignores the ceiling.
    //
    // NOT COVERED OFFLINE ANY MORE, in those words: that a live process
    // actually refuses. Closing that needs the freeze driven somewhere it
    // cannot wedge a shared test binary -- a dedicated single-test process --
    // or a live patched hoi4.exe.

    /// Falsifiable: fails if a pass that saw a thread vanish is ever allowed
    /// to be the last one.
    ///
    /// This is the subtle half of the fixed point. A thread whose id went
    /// stale during the pass WAS running when the snapshot was taken, so it
    /// had the opportunity to start a successor that the snapshot cannot
    /// contain. Concluding "quiet" on that pass returns a freeze with a live
    /// thread in the process -- the exact fail-open this module exists to
    /// close, reintroduced one level up. Only a pass that saw nothing at all
    /// outside the suspended set proves the set is closed.
    #[test]
    fn a_pass_that_saw_a_thread_vanish_is_never_the_last_one() {
        assert!(pass_is_quiet(0, 0), "nothing outside the set is the fixed point");
        assert!(
            !pass_is_quiet(0, 1),
            "a vanished thread may have started a successor the snapshot cannot show"
        );
        assert!(!pass_is_quiet(1, 0), "a newly suspended thread is a changed set");
        assert!(!pass_is_quiet(3, 2));
    }

    /// Falsifiable, and the one that observes the real OS: fails if the freeze
    /// does not actually stop other threads.
    ///
    /// A worker spins on a counter. While the freeze is held the counter must
    /// not move -- if `freeze_other_threads` stopped suspending, or suspended
    /// only the threads from its first enumeration, or resumed early, the
    /// counter advances and this fails. After the freeze drops it must advance
    /// again, which is what says the threads were resumed rather than lost.
    #[test]
    fn a_held_freeze_stops_other_threads_and_dropping_it_starts_them_again() {
        let _serialised = os_suspend_state_guard();
        let counter = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let counter = Arc::clone(&counter);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    counter.fetch_add(1, Ordering::AcqRel);
                    std::hint::spin_loop();
                }
            })
        };
        // Wait until the worker is demonstrably running, so a frozen counter
        // cannot be confused with a thread that never started.
        while counter.load(Ordering::Acquire) == 0 {
            std::thread::yield_now();
        }

        let freeze = freeze_other_threads().expect("a test process must be freezable");
        assert!(
            freeze.frozen_thread_count() >= 1,
            "the spinning worker must have been suspended"
        );
        let first = counter.load(Ordering::Acquire);
        std::thread::sleep(std::time::Duration::from_millis(40));
        let second = counter.load(Ordering::Acquire);
        assert_eq!(
            first, second,
            "the counter advanced while the process was supposed to be frozen"
        );
        assert!(
            freeze.passes() >= 2,
            "the fixed point needs a verification pass that found nothing new"
        );
        drop(freeze);

        let resumed_from = counter.load(Ordering::Acquire);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while counter.load(Ordering::Acquire) == resumed_from {
            assert!(
                std::time::Instant::now() < deadline,
                "the worker was never resumed after the freeze was dropped"
            );
            std::thread::yield_now();
        }
        stop.store(true, Ordering::Release);
        worker.join().expect("worker joins");
    }

    /// Falsifiable: fails if a freeze stops being counted, which is the
    /// observable every ordering test in `lib.rs` reads.
    #[test]
    fn every_freeze_attempt_is_counted() {
        let _serialised = os_suspend_state_guard();
        let before = freeze_attempts();
        let freeze = freeze_other_threads().expect("a test process must be freezable");
        drop(freeze);
        assert!(
            freeze_attempts() > before,
            "the attempt counter must move on every call"
        );
    }

    /// Falsifiable, and the reason a thread that merely exited does not refuse
    /// a patch: fails if a `SuspendThread` failure is ever excused by anything
    /// but the thread object being signalled, and fails if the BOUNDED WAIT
    /// that excusal depends on stops being load-bearing.
    ///
    /// MOVED HERE from `fired_event_popups_live`, whose hand-rolled suspender
    /// this freeze replaced. What was measured there, with a standalone probe
    /// over 98 enumeration rounds against churning threads: every one of the 47
    /// `SuspendThread` failures reported ERROR_ACCESS_DENIED (5), and every one
    /// of those threads was already signalled. The rule that replaced excused
    /// only ERROR_INVALID_HANDLE (6) and ERROR_INVALID_PARAMETER (87) -- neither
    /// of which the OS ever returned for a suspend -- so an ordinary thread exit
    /// refused an entire installation, about one run in ten.
    ///
    /// Two mutations turn this red. Restore an error-code allowlist and the
    /// excused count stops moving. Drop [`TERMINATION_PROOF_TIMEOUT_MS`] to
    /// zero and the WAITED count stops moving, because the threads that are
    /// already signalled at the instant the suspend is refused would be excused
    /// by a zero timeout too -- stopping at that first, easier shape is what let
    /// a zero-timeout mutation survive an earlier version of this test. Both
    /// shapes are required.
    ///
    /// [`FreezeRefusal::NotQuiescent`] is TOLERATED and is not a defect here:
    /// the churn below is deliberately pathological -- 64 threads starting and
    /// exiting continuously -- and a fixed point that cannot be reached in
    /// [`MAX_PASSES`] under that load is the freeze correctly refusing to
    /// certify a process that never went quiet. Every OTHER refusal fails the
    /// test, which is where the coverage is: `ThreadSuspend` is exactly the
    /// fail-closed-on-a-dead-thread regression this exists to catch.
    ///
    /// `ThreadOpen` is the SECOND refusal shape this catches, and the reason
    /// the recycled-id disposition exists. Before it, this test failed on
    /// about one run in five with "thread N could not be opened (os error 5)"
    /// -- an `ERROR_ACCESS_DENIED` on an id the kernel had already handed to
    /// another process. The counter below is reported on failure because
    /// distinguishing "the recycled-id path is broken" from "the recycled-id
    /// path was never reached" is otherwise guesswork.
    ///
    /// Nothing is joined while a freeze is held: each freeze is dropped at the
    /// end of its own iteration, and the churn threads are stopped and joined
    /// only after the loop.
    #[test]
    fn a_thread_exiting_mid_enumeration_is_excused_only_by_its_signalled_thread_object() {
        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let churn = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let mut producers = Vec::new();
                for _ in 0..4 {
                    let stop = Arc::clone(&stop);
                    producers.push(std::thread::spawn(move || {
                        while !stop.load(Ordering::Acquire) {
                            let workers: Vec<_> = (0..16)
                                .map(|_| std::thread::spawn(std::thread::yield_now))
                                .collect();
                            for worker in workers {
                                let _ = worker.join();
                            }
                        }
                    }));
                }
                for producer in producers {
                    let _ = producer.join();
                }
            })
        };

        const MAX_ROUNDS: u32 = 20_000;
        let excused_before = suspend_failures_excused_as_terminated();
        let waited_before = suspend_failures_needing_the_termination_wait();
        let recycled_before = open_failures_excused_as_no_longer_ours();
        let mut rounds = 0u32;
        let outcome = loop {
            rounds += 1;
            match freeze_other_threads() {
                Ok(freeze) => drop(freeze),
                Err(FreezeRefusal::NotQuiescent { .. }) => {}
                Err(refusal) => break Err((rounds, refusal)),
            }
            let excused = suspend_failures_excused_as_terminated() - excused_before;
            let waited = suspend_failures_needing_the_termination_wait() - waited_before;
            if excused > 0 && waited > 0 {
                break Ok((rounds, excused));
            }
            if rounds >= MAX_ROUNDS {
                break Ok((rounds, 0));
            }
        };
        stop.store(true, Ordering::Release);
        churn.join().expect("churn thread joins");
        let recycled = open_failures_excused_as_no_longer_ours() - recycled_before;
        match outcome {
            Err((round, refusal)) => panic!(
                "round {round}: a thread exiting mid-enumeration must not refuse \
                 the freeze: {refusal} \
                 ({recycled} open failure(s) had already been excused as \
                 no-longer-ours during this run)"
            ),
            Ok((rounds, 0)) => panic!(
                "{rounds} freezes under churn never hit both shapes of a thread \
                 exiting mid-suspend, so this test proved nothing; make the churn \
                 harder rather than letting it pass vacuously"
            ),
            Ok((_, excused)) => assert!(excused > 0),
        }
    }

    /// Falsifiable, and the guard on the question a non-MinHook patcher has to
    /// ask: fails if [`ProcessFreeze::any_instruction_pointer_in`] stops
    /// actually reading thread contexts.
    ///
    /// The fail-open mutation is the obvious one -- return `Ok(false)`
    /// unconditionally, so every patch site always looks clear. A range that
    /// spans the whole address space must find a frozen thread; the reserved
    /// null page must find none. The first assertion is what dies under that
    /// mutation, and it cannot pass vacuously because the freeze is asserted to
    /// hold at least one thread first.
    ///
    /// It also pins the CONTEXT alignment: `windows-sys` declares the AMD64
    /// `CONTEXT` as plain `#[repr(C)]`, and without the 16-byte wrapper every
    /// `GetThreadContext` fails with ERROR_NOACCESS -- which, because the read
    /// fails closed, turns the FIRST assertion into an `Err` and fails here.
    #[test]
    fn a_frozen_thread_is_found_where_it_is_stopped_and_nowhere_else() {
        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            })
        };

        let freeze = freeze_other_threads().expect("a test process must be freezable");
        let frozen = freeze.frozen_thread_count();
        let everywhere = freeze.any_instruction_pointer_in(&[(0, usize::MAX)]);
        let nowhere = freeze.any_instruction_pointer_in(&[(0x1000, 0x1000 + 16)]);
        let empty = freeze.any_instruction_pointer_in(&[]);
        drop(freeze);

        assert!(frozen >= 1, "the spinning worker must have been suspended");
        assert_eq!(
            everywhere,
            Ok(true),
            "every frozen thread is stopped somewhere, so a range covering the \
             whole address space must find one"
        );
        assert_eq!(
            nowhere,
            Ok(false),
            "no thread can be stopped in the reserved null page"
        );
        assert_eq!(empty, Ok(false), "no ranges is nothing to be inside");

        stop.store(true, Ordering::Release);
        worker.join().expect("worker joins");
    }

    /// Falsifiable: fails if threads that come and go are ever left running by
    /// a freeze that declared success.
    ///
    /// This is the racing case the loop exists for, and the one the "flake" in
    /// `fired_event_popups_live` is made of: threads starting and exiting while
    /// the enumeration runs. Every worker must be either suspended or gone --
    /// never running -- by the time the freeze is held.
    #[test]
    fn threads_churning_during_the_freeze_are_all_accounted_for() {
        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let churn = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    let handles: Vec<_> = (0..8)
                        .map(|_| std::thread::spawn(|| std::thread::yield_now()))
                        .collect();
                    for handle in handles {
                        let _ = handle.join();
                    }
                }
            })
        };

        for _ in 0..25 {
            let freeze = freeze_other_threads()
                .expect("threads exiting mid-enumeration must not refuse the freeze");
            assert!(
                freeze.passes() <= MAX_PASSES,
                "the pass bound must hold on success"
            );
            drop(freeze);
        }
        stop.store(true, Ordering::Release);
        churn.join().expect("churn thread joins");
    }
}
