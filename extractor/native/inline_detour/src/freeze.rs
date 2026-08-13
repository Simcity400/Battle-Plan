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
//! The SAME recycling also makes `OpenThread` SUCCEED on a thread that is not
//! ours, and that half went unhandled for longer: another process running as
//! this user hands over [`FREEZE_THREAD_ACCESS`] without complaint, and the
//! freeze then suspended a stranger's thread and held it for the length of the
//! patch. Every successfully opened handle is now checked with
//! `GetProcessIdOfThread` before it is suspended; a thread belonging to another
//! process counts as a vanish (our thread with that id has ended) and an owner
//! that cannot be read refuses.
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
//! window. Buffers are therefore reserved before the first suspension and
//! reused across passes, and -- since the reservation was only ever a hope --
//! exhausting one inside the window now REFUSES
//! ([`FreezeRefusal::CapacityExceeded`]) instead of growing it.
//!
//! Two of OUR OWN allocator calls used to sit inside that window, which no
//! amount of reserving would have moved:
//!
//!  * the enumeration buffer was a local of [`freeze_other_threads_inner`], so
//!    Rust FREED it on the successful `return Ok(freeze)` -- with every other
//!    thread still suspended, on every successful freeze. Both buffers are now
//!    owned by [`ProcessFreeze`] and are freed only after its `Drop` has
//!    resumed everything;
//!  * the suspended-handle table was reserved one thread-count short of the
//!    enumeration's, so a burst that fitted in one could reallocate the other.
//!
//! What is left inside the window is `CreateToolhelp32Snapshot`, ours and
//! MinHook's. On Windows 10.0.26100 the `TH32CS_SNAPTHREAD` path allocates
//! with `NtAllocateVirtualMemory` and builds a section with `NtCreateSection`;
//! no `RtlAllocateHeap` and no loader-lock acquisition appear on it, so on that
//! build it does not wait for either lock. That is implementation evidence
//! about one build, not a documented contract, and `NtGetNextThread` would
//! remove the question rather than bound it.
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
    GetCurrentProcessId, GetCurrentThreadId, GetProcessIdOfThread, OpenThread, ResumeThread,
    SuspendThread, WaitForSingleObject, THREAD_GET_CONTEXT, THREAD_QUERY_INFORMATION,
    THREAD_SET_CONTEXT, THREAD_SUSPEND_RESUME,
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
///
/// The ownership check below needs a QUERY right and this mask already carries
/// one. `THREAD_QUERY_LIMITED_INFORMATION` was added here and then removed
/// again, because the claim that `GetProcessIdOfThread` requires it
/// SPECIFICALLY is wrong, and the mask is not the place for a redundant bit.
/// MEASURED on Windows 10.0.26200, opening this thread four times with one mask
/// each (`runs/ib_harness_fixes/probe_thread_query_rights_result.txt`):
///
/// ```text
/// THREAD_QUERY_INFORMATION only         -> GetProcessIdOfThread returned our pid
/// THREAD_QUERY_LIMITED_INFORMATION only -> GetProcessIdOfThread returned our pid
/// THREAD_SUSPEND_RESUME only            -> GetProcessIdOfThread FAILED, os error 5
/// both query rights                     -> GetProcessIdOfThread returned our pid
/// ```
///
/// So either query right authorises it and neither alone is load-bearing; what
/// IS load-bearing is that some query right is present, which is why the
/// suspend-only row is quoted rather than left as an assumption.
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
/// How many times a thread we SUCCESSFULLY opened turned out to belong to
/// another process, and was therefore left alone.
///
/// The population that was previously suspended without anyone noticing. It is
/// counted for the same reason as the three above: so a test can say the
/// ownership check was REACHED, rather than merely that nothing went wrong on
/// a machine where no id happened to be recycled across a process boundary.
static OPEN_SUCCESSES_EXCUSED_AS_ANOTHER_PROCESSES: AtomicU64 = AtomicU64::new(0);

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

/// How many threads we opened successfully belonged to another process, and
/// were therefore never suspended -- the recycled-id case that SUCCEEDS.
pub fn open_successes_excused_as_another_processes() -> u64 {
    OPEN_SUCCESSES_EXCUSED_AS_ANOTHER_PROCESSES.load(Ordering::Acquire)
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
    /// A thread was opened, but which process owns it could not be read, so it
    /// could not be established that suspending it would stop a thread of
    /// OURS rather than a stranger's.
    ThreadOwner { thread: u32, os_error: u32 },
    /// A buffer reserved before the first suspension was exhausted while
    /// threads were held suspended.
    ///
    /// Growing it there is a call into the Rust global allocator, whose lock a
    /// suspended thread may hold -- the hang the module header describes, and
    /// the one failure mode that is strictly worse than refusing. So this
    /// refuses instead, and the reservation is a BOUND rather than a hope.
    CapacityExceeded { buffer: &'static str, capacity: usize },
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
            Self::ThreadOwner { thread, os_error } => write!(
                f,
                "detour patch refused: thread {thread} was opened but its \
                 owning process could not be read (os error {os_error}), so it \
                 could not be established as this process's own; no target \
                 byte was touched"
            ),
            Self::CapacityExceeded { buffer, capacity } => write!(
                f,
                "detour patch refused: the {buffer} buffer reserved before the \
                 freeze ({capacity} entries) was exhausted while threads were \
                 held, and growing it would take the allocator's lock that a \
                 suspended thread may hold; no target byte was touched"
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

/// Who owns the thread behind a handle `OpenThread` DID give us.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpenedThread {
    /// A thread of this process. It is ours to suspend.
    Ours,
    /// The id was recycled onto another process's thread. Our thread with that
    /// id has ended, and this one is not ours to touch.
    AnotherProcesses,
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

/// `OpenThread` SUCCEEDED. Is the thread on the other end of that handle ours?
///
/// The module header already establishes that ids are system-wide and that our
/// thread with a given id can exit between the snapshot and the open. It drew
/// only half the conclusion. The other half is this: when the kernel then hands
/// that id to a thread in a DIFFERENT process, `OpenThread` does not
/// necessarily fail. Another process running as the same user, at the same
/// integrity level, hands out `FREEZE_THREAD_ACCESS` without complaint -- which
/// is the ordinary case for sibling test processes, and for anything else the
/// user is running.
///
/// Without this check the next line SUSPENDS that stranger's thread, keeps its
/// handle in `frozen`, and resumes it whenever this freeze is dropped. That is
/// a recorder reaching outside its own process to stop another program's
/// threads, which is not a hazard this module is allowed to trade for anything.
///
/// A thread that is NOT ours is `Gone` in exactly the sense the enumeration
/// meant: our thread with that id has ended. It is counted as a vanish, so
/// [`pass_is_quiet`] cannot end the loop on this pass and a successor thread
/// that reused the id is still enumerated next pass.
pub(crate) fn classify_opened_thread_owner(owner: u32, ours: u32) -> OpenedThread {
    if owner == ours {
        OpenedThread::Ours
    } else {
        OpenedThread::AnotherProcesses
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
/// # Why the buffers live HERE and not in the function that fills them
///
/// Rust destroys a function's locals before it returns. A `Vec` held as a
/// local of [`freeze_other_threads_inner`] is therefore FREED on the successful
/// `return Ok(freeze)` -- while the returned freeze still holds every other
/// thread suspended. On Windows that free is a `HeapFree` on the process heap,
/// and if any suspended thread owns that heap's lock it waits for a thread that
/// cannot run. That is the exact hang this module's header warns about, taken
/// on EVERY successful freeze, and the generous reservations do not touch it:
/// they stop the buffer growing, not it being dropped.
///
/// Owning the buffers here moves both frees after `Drop::drop` has resumed
/// every thread. Field order matters and is deliberate: fields drop in
/// declaration order AFTER `Drop::drop` runs, so the two buffers are freed
/// while `_serialisation` is still held, and no competing freezer can be
/// suspending anyone while we call the allocator.
pub struct ProcessFreeze {
    /// `(thread id, handle)` for each thread this freeze suspended.
    frozen: Vec<(u32, HANDLE)>,
    /// Scratch for the per-pass thread enumeration. Reserved before the first
    /// suspension, reused by every pass, and never grown while frozen.
    enumeration: Vec<u32>,
    /// Held for the lifetime of the freeze, and released only once both
    /// buffers above are gone.
    _serialisation: MutexGuard<'static, ()>,
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

/// May a buffer being filled right now call the allocator?
///
/// Only outside the frozen window. Inside it, a reallocation takes the Rust
/// global allocator's lock, which a suspended thread may hold, and the process
/// hangs with nobody left to release it. Passing this explicitly is what stops
/// the answer being re-derived by whoever edits the loop next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Growth {
    /// Nothing is suspended yet.
    Allowed,
    /// Threads are held. Exhausting the reservation REFUSES.
    Forbidden,
}

/// How many entries each frozen-window buffer must hold, given the thread
/// count seen before the first suspension.
///
/// One function for both buffers, because the defect it replaces was them
/// disagreeing: the enumeration got `n + (4n + 64)` and the handle table got
/// `4n + 64`, so between those two sizes lay a burst that the ids fitted and
/// their handles did not -- and that is the overflow that happens with a
/// thread already suspended and nothing yet owning the suspension.
///
/// The headroom covers a burst of thread creation racing the first pass. It is
/// a BOUND, not an estimate: exceeding it refuses, so being wrong about the
/// burst size costs a refusal and never an allocation while frozen.
pub(crate) fn frozen_window_buffer_bound(thread_count: usize) -> usize {
    thread_count.saturating_add(thread_count.saturating_mul(4).saturating_add(64))
}

/// Snapshot this process's thread ids into `out`, replacing its contents.
///
/// Reuses `out`'s capacity so a verification pass taken while threads are
/// suspended does not allocate. Under [`Growth::Forbidden`] that reuse is
/// ENFORCED rather than hoped for: a snapshot with more ids than the
/// reservation holds refuses instead of growing the buffer.
fn enumerate_thread_ids(out: &mut Vec<u32>, growth: Growth) -> Result<(), FreezeRefusal> {
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
            if growth == Growth::Forbidden && out.len() == out.capacity() {
                unsafe { CloseHandle(snapshot) };
                return Err(FreezeRefusal::CapacityExceeded {
                    buffer: "thread enumeration",
                    capacity: out.capacity(),
                });
            }
            out.push(entry.th32ThreadID);
        }
        if unsafe { Thread32Next(snapshot, &mut entry) } == 0 {
            // Do NOT break on an arbitrary Thread32Next error. Treating any
            // stop as end-of-list would certify a partial enumeration: the
            // threads past the truncation point would never be suspended, and
            // the fixed point below would happily conclude the process was
            // quiet. (This sentence is also the regression guard for the source
            // scan that enforces it -- it once failed on prose like this.)
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
        return record_termination_probe(true, false);
    }
    let signalled_after_the_wait =
        unsafe { WaitForSingleObject(handle, TERMINATION_PROOF_TIMEOUT_MS) } == WAIT_OBJECT_0;
    record_termination_probe(false, signalled_after_the_wait)
}

/// The whole decision [`thread_has_terminated`] makes, with the two OS probes
/// as inputs.
///
/// Split out so the rule -- and specifically the fact that only the DELAYED
/// signal increments [`SUSPEND_FAILURES_NEEDING_THE_WAIT`] -- can be driven
/// from a test directly, both orderings, in microseconds. Establishing the
/// same rule through the OS means waiting for a thread to be caught between
/// `SuspendThread` refusing it and its thread object signalling, which is a
/// rare interleaving whose arrival time is a random variable: measured over 13
/// runs of that test it ranged from 1.7 s to 176.8 s, and failed once. A gate
/// cannot be built on that, and it does not have to be: the OS's part is
/// "sometimes a thread is caught mid-exit", which the churn test still proves,
/// and this is the part that is pure logic.
///
/// `signalled_after_wait` is meaningless when `signalled_immediately` is true,
/// and the caller never performs the second wait in that case -- the
/// short-circuit is preserved, so the bound on the whole probe is still one
/// [`TERMINATION_PROOF_TIMEOUT_MS`].
pub(crate) fn record_termination_probe(
    signalled_immediately: bool,
    signalled_after_wait: bool,
) -> bool {
    if signalled_immediately {
        return true;
    }
    if signalled_after_wait {
        SUSPEND_FAILURES_NEEDING_THE_WAIT.fetch_add(1, Ordering::AcqRel);
    }
    signalled_after_wait
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
    // Growth::Allowed is correct HERE and nowhere else in this function:
    // nothing is suspended yet, so taking the allocator's lock is ordinary. It
    // is also the only place it may appear, and the source pin in the tests
    // counts the occurrences -- over code with comments stripped, so that this
    // very comment does not break the build it exists to explain.
    enumerate_thread_ids(&mut ids, Growth::Allowed)?;

    // Reserve before the first suspension. Every later pass reuses this
    // capacity, so the frozen window does not allocate -- enforced from here
    // on by `Growth::Forbidden` and the capacity check before each push, not
    // left to the reservation being big enough.
    //
    // `try_reserve`, not `reserve`: an allocation that cannot be satisfied is
    // a refusal, not an abort, and refusing here costs nothing because nothing
    // is suspended yet.
    let bound = frozen_window_buffer_bound(ids.len());
    if ids.try_reserve(bound.saturating_sub(ids.len())).is_err() {
        return Err(FreezeRefusal::CapacityExceeded {
            buffer: "thread enumeration",
            capacity: ids.capacity(),
        });
    }
    // The suspended-handle table gets the SAME bound, from the same function.
    // It used to get the headroom alone while the enumeration got the headroom
    // PLUS the threads already counted, so a burst could fit in the list of
    // ids and not in the list of their handles -- and the handle table is the
    // one whose overflow happens with a thread already suspended.
    let mut frozen: Vec<(u32, HANDLE)> = Vec::new();
    if frozen.try_reserve(bound).is_err() {
        return Err(FreezeRefusal::CapacityExceeded {
            buffer: "suspended handle table",
            capacity: frozen.capacity(),
        });
    }
    let mut freeze = ProcessFreeze {
        frozen,
        enumeration: ids,
        _serialisation: serialisation,
        passes: 0,
    };

    for pass in 1..=MAX_PASSES {
        freeze.passes = pass;
        if pass > 1 {
            // A verification pass, inside the frozen window: reuse only, never
            // Growth::Allowed.
            /* Every comment shape appears here on purpose, because the source
               scan that enforces this rule is what reads this function and must
               not be satisfiable -- or breakable -- by prose naming the thing it
               counts. A `//` inside a block comment, as in a URL like
               https://example, once truncated this comment at that point and
               destroyed the closing delimiter below. /* And these nest. */ */
            enumerate_thread_ids(&mut freeze.enumeration, Growth::Forbidden)?;
        }
        let mut newly_suspended = 0u32;
        let mut vanished = 0u32;
        // Indexed rather than iterated: the body needs `&mut freeze.frozen`
        // while reading `freeze.enumeration`, and every refusal below returns
        // `freeze` out of the function.
        for index in 0..freeze.enumeration.len() {
            let thread_id = freeze.enumeration[index];
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
            // The handle is real; whose thread is on the other end is a
            // SEPARATE question, and the one the enumeration cannot answer. An
            // id recycled onto another process's thread opens without error
            // when that process is the same user's, and suspending it here is
            // this recorder stopping a stranger's threads.
            let owner = unsafe { GetProcessIdOfThread(handle) };
            if owner == 0 {
                let os_error = unsafe { GetLastError() };
                unsafe { CloseHandle(handle) };
                return Err(FreezeRefusal::ThreadOwner {
                    thread: thread_id,
                    os_error,
                });
            }
            if classify_opened_thread_owner(owner, unsafe { GetCurrentProcessId() })
                == OpenedThread::AnotherProcesses
            {
                unsafe { CloseHandle(handle) };
                OPEN_SUCCESSES_EXCUSED_AS_ANOTHER_PROCESSES.fetch_add(1, Ordering::AcqRel);
                vanished += 1;
                continue;
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
            // The suspension has happened and is not yet owned by anything
            // that would undo it, so a push that reallocates here is the one
            // allocator call with no safe outcome. Undo this thread's
            // suspension and refuse; `freeze` resumes the rest on the way out.
            if freeze.frozen.len() == freeze.frozen.capacity() {
                unsafe {
                    ResumeThread(handle);
                    CloseHandle(handle);
                }
                return Err(FreezeRefusal::CapacityExceeded {
                    buffer: "suspended handle table",
                    capacity: freeze.frozen.capacity(),
                });
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
        for &thread_id in &freeze.enumeration {
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
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
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
        // NO separate assertion for the ownership check's query right. It would
        // be vacuous: MINHOOK_THREAD_ACCESS already contains
        // THREAD_QUERY_INFORMATION, so the superset assertion above cannot pass
        // unless FREEZE_THREAD_ACCESS carries a query right, and any mutation
        // removing every query right fails that assertion first. Measured on
        // 10.0.26200 (runs/ib_harness_fixes/probe_thread_query_rights_result.txt),
        // either query right authorises GetProcessIdOfThread and a suspend-only
        // mask fails it with ERROR_ACCESS_DENIED - so the property is real, and
        // it is already held here by the line above.
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
    /// helper of ours that allocates internally passes this test. The list
    /// below is therefore the shapes a future edit is LIKELY to reach for, and
    /// its job is to make the invariant visible at the point of edit.
    ///
    /// Nothing available here proves the stronger property either. The closest
    /// is a `#[global_allocator]` shim that ABORTS when it is entered while a
    /// freeze is held -- abort and not panic, because unwinding out of a
    /// `GlobalAlloc` method is undefined behaviour -- run in a dedicated
    /// process so the abort cannot take the shared test binary down. Even that
    /// would only catch the paths a run actually exercises, so it is a
    /// detector and not a proof. This crate has neither.
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

    /// Falsifiable, and the deterministic half of the bounded wait's proof:
    /// fails if the delayed-signal population stops being counted separately
    /// from the already-signalled one.
    ///
    /// kills: counting every excusal as a waited one, or counting none.
    ///
    /// This is the RULE the churn test used to establish by waiting for the OS
    /// to catch a thread between `SuspendThread` refusing it and its thread
    /// object signalling. That wait took between 1.7 s and 176.8 s over 13
    /// measured runs and failed once; the logic it was waiting to demonstrate
    /// is right here and takes no time at all.
    ///
    /// It does NOT kill a zero [`TERMINATION_PROOF_TIMEOUT_MS`], and saying it
    /// did was this test's own first mistake: it SUPPLIES the probe results
    /// instead of measuring them, so a zero timeout satisfies every row below
    /// unchanged. [`the_termination_proof_actually_spends_the_bounded_wait`] is
    /// what kills that, by measuring what the wait costs against a floor that
    /// is not derived from the constant.
    #[test]
    fn only_a_delayed_signal_counts_as_a_waited_termination_proof() {
        let _serialised = os_suspend_state_guard();

        // Already signalled when the suspend was refused: terminated, and NOT
        // a member of the population the timeout exists for.
        let before = suspend_failures_needing_the_termination_wait();
        assert!(record_termination_probe(true, false));
        assert_eq!(
            suspend_failures_needing_the_termination_wait(),
            before,
            "a thread that was already signalled did not need the wait"
        );

        // Not signalled at the refusal, signalled within the timeout: this is
        // the population, and a zero timeout would never see it.
        assert!(record_termination_probe(false, true));
        assert_eq!(
            suspend_failures_needing_the_termination_wait(),
            before + 1,
            "the delayed signal is what the bounded wait is for"
        );

        // Never signalled: not terminated, so it refuses, and it is not
        // counted as a wait that paid off.
        assert!(!record_termination_probe(false, false));
        assert_eq!(
            suspend_failures_needing_the_termination_wait(),
            before + 1,
            "a thread that never signalled proves nothing"
        );
    }

    /// Falsifiable, and the OTHER half of the bounded wait's proof: fails if
    /// [`TERMINATION_PROOF_TIMEOUT_MS`] stops being spent.
    ///
    /// kills: setting [`TERMINATION_PROOF_TIMEOUT_MS`] to zero, or to anything
    /// under the floor below, and any change that stops the probe SPENDING that
    /// time before it answers.
    ///
    /// It does NOT kill replacing the second `WaitForSingleObject` with a
    /// `sleep` of the same length and a hard-coded `false`: this measures
    /// elapsed time and the verdict, not that the handle was probed. Nothing
    /// here can see that, and the evidence-tier test is what would eventually
    /// notice the delayed-signal population had stopped existing.
    ///
    /// The seam test above proves the RULE and cannot prove the DURATION,
    /// because it supplies the probe results instead of measuring them -- a
    /// zero timeout would satisfy it unchanged, which is the gap this closes.
    /// Measuring it needs no thread race and no contrived handle: a thread that
    /// is deliberately still RUNNING never signals its thread object, so
    /// `thread_has_terminated` spends the whole configured timeout on it and
    /// then correctly answers "not terminated". That is also the exact
    /// production shape -- a live thread we could not suspend is the case the
    /// timeout exists to adjudicate -- so this measures the real entry point on
    /// a real thread handle, opened at the real access mask.
    ///
    /// Deterministic in the only direction that matters. A timed wait may
    /// return LATE under load -- which still passes -- and the OS does not
    /// return early from one, so this cannot flake the way the churn test it
    /// replaces did. Half the constant is the threshold rather than the whole
    /// of it purely to absorb timer granularity; at zero the elapsed time is
    /// microseconds, so the mutation is nowhere near the margin.
    #[test]
    fn the_termination_proof_actually_spends_the_bounded_wait() {
        // A LITERAL floor, deliberately NOT derived from
        // TERMINATION_PROOF_TIMEOUT_MS. The first version of this test computed
        // it as `TERMINATION_PROOF_TIMEOUT_MS / 2`, which is precisely how it
        // survived the mutation it was written to kill: at a zero timeout the
        // floor went to zero with it and every assertion passed. A threshold
        // derived from the thing under test is not a threshold.
        //
        // 25 ms is half the 50 ms the constant is set to, which leaves ample
        // room for timer granularity while sitting three orders of magnitude
        // above the microseconds a zero timeout returns in. If the constant is
        // ever lowered below this deliberately, this test is the review gate
        // that says so.
        const BOUNDED_WAIT_FLOOR_MS: u64 = 25;

        // The constant, checked directly, BESIDE the measurement and not
        // instead of it. Timing alone cannot settle this: with the timeout at
        // zero both waits return immediately, but a preemption inside the
        // measured interval can still inflate the elapsed time past the floor,
        // and then a mutated build passes. This assertion has no such gap.
        //
        // It is not a substitute for the measurement either -- it says nothing
        // about whether the second wait is still PERFORMED, which is what the
        // elapsed time below is for. Deleting either leaves a live mutation.
        assert!(
            u64::from(TERMINATION_PROOF_TIMEOUT_MS) >= BOUNDED_WAIT_FLOOR_MS,
            "TERMINATION_PROOF_TIMEOUT_MS is {TERMINATION_PROOF_TIMEOUT_MS} ms, under \
             the {BOUNDED_WAIT_FLOOR_MS} ms floor; a thread that has entered \
             NtTerminateThread reports ERROR_ACCESS_DENIED from SuspendThread \
             BEFORE its thread object is signalled, so a timeout this short reads \
             a thread that is already past all user-mode code as one that might \
             still be running"
        );

        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let live_thread_id = Arc::new(AtomicU32::new(0));
        let worker = {
            let stop = Arc::clone(&stop);
            let live_thread_id = Arc::clone(&live_thread_id);
            std::thread::spawn(move || {
                live_thread_id.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
                while !stop.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            })
        };
        // Its own id, published by the thread itself: `JoinHandle` does not
        // expose the OS id on stable, and guessing one would be a different
        // thread's.
        let mut thread_id = 0u32;
        while thread_id == 0 {
            thread_id = live_thread_id.load(Ordering::Acquire);
            std::thread::yield_now();
        }
        let handle = unsafe { OpenThread(FREEZE_THREAD_ACCESS, 0, thread_id) };
        assert!(
            !handle.is_null(),
            "a live thread of this process must be openable at the freeze mask"
        );

        let waited_before = suspend_failures_needing_the_termination_wait();
        let started = std::time::Instant::now();
        let verdict = thread_has_terminated(handle);
        let elapsed = started.elapsed();
        unsafe { CloseHandle(handle) };
        stop.store(true, Ordering::Release);
        worker.join().expect("worker joins");

        assert!(
            !verdict,
            "a thread that is still running is not proof of termination"
        );
        assert!(
            elapsed >= std::time::Duration::from_millis(BOUNDED_WAIT_FLOOR_MS),
            "the termination proof returned after {elapsed:?}, under the \
             {BOUNDED_WAIT_FLOOR_MS} ms floor, so it did not spend the wait it is \
             supposed to give a thread to finish terminating; a zero timeout reads \
             a thread that is already past all user-mode code as one that might \
             still be running"
        );
        assert_eq!(
            suspend_failures_needing_the_termination_wait(),
            waited_before,
            "a wait that timed out is not a wait that paid off"
        );
    }

    /// Falsifiable: fails if a successfully opened handle is suspended without
    /// establishing whose thread it is.
    ///
    /// kills: inverting the ownership rule -- treating a foreign owner as ours,
    /// or our own as foreign.
    ///
    /// It does NOT kill DELETING the production `GetProcessIdOfThread` read;
    /// this test only exercises the classifier. That deletion is killed by the
    /// open -> owner -> suspend ordering pin in
    /// [`the_fail_open_decisions_are_taken_only_through_their_classifiers`].
    ///
    /// Ids are system-wide and `OpenThread` succeeds against another process
    /// running as this user, so "the open worked" says nothing about ownership.
    /// A thread that is not ours is `AnotherProcesses` -- our thread with that
    /// id has ended -- and must never be suspended.
    #[test]
    fn an_opened_handle_is_only_ours_when_the_kernel_says_the_owner_is_us() {
        let ours = unsafe { GetCurrentProcessId() };
        assert_eq!(classify_opened_thread_owner(ours, ours), OpenedThread::Ours);
        assert_eq!(
            classify_opened_thread_owner(ours.wrapping_add(4), ours),
            OpenedThread::AnotherProcesses
        );
        // 0 is `GetProcessIdOfThread`'s failure return and the freeze loop
        // REFUSES on it before reaching this classifier -- an owner that could
        // not be read is not evidence of anything. The classifier is still
        // total, so that a future caller which does reach it with 0 cannot be
        // told the thread is ours.
        assert_eq!(
            classify_opened_thread_owner(0, ours),
            OpenedThread::AnotherProcesses
        );
    }

    /// Falsifiable: fails if an enumeration taken inside the frozen window can
    /// grow its buffer.
    ///
    /// kills: dropping the `Growth::Forbidden` capacity check itself.
    ///
    /// It does NOT kill passing `Growth::Allowed` from the verification pass --
    /// this test calls `enumerate_thread_ids` directly, so the production call
    /// site is invisible to it. That mutation is killed at the call site by
    /// [`the_fail_open_decisions_are_taken_only_through_their_classifiers`],
    /// which also bounds how many enumerations the frozen window may contain.
    ///
    /// Growing there is a call into the Rust global allocator with every other
    /// thread suspended, which is the one failure this module cannot recover
    /// from: whoever owns that lock is stopped and there is nobody left to
    /// release it. Refusing is the only safe answer, and it is the answer the
    /// reservation was previously only hoping to avoid needing.
    #[test]
    fn a_frozen_window_enumeration_refuses_rather_than_growing_its_buffer() {
        let _serialised = os_suspend_state_guard();

        // Not vacuous: this process really does have another thread to list,
        // so the zero-capacity call below MUST reach the capacity check.
        let mut outside: Vec<u32> = Vec::new();
        enumerate_thread_ids(&mut outside, Growth::Allowed)
            .expect("a test process enumerates its own threads");
        assert!(
            !outside.is_empty(),
            "the enumeration found no other thread, so the refusal below would prove nothing"
        );

        let mut inside: Vec<u32> = Vec::new();
        assert_eq!(
            enumerate_thread_ids(&mut inside, Growth::Forbidden),
            Err(FreezeRefusal::CapacityExceeded {
                buffer: "thread enumeration",
                capacity: 0,
            })
        );
        assert_eq!(
            inside.capacity(),
            0,
            "the buffer was grown inside a window that may not allocate"
        );
    }

    /// Falsifiable: fails if the two frozen-window buffers can be sized
    /// differently again.
    ///
    /// kills: a bound that does not leave room for a burst, or that wraps on an
    /// absurd thread count instead of saturating.
    ///
    /// It does NOT kill reserving the headroom alone for the suspended-handle
    /// table -- that is a CALL-SITE property, invisible to a test of the
    /// function, and it is pinned as one in
    /// [`the_fail_open_decisions_are_taken_only_through_their_classifiers`],
    /// which requires both reservations to be taken from this bound.
    #[test]
    fn both_frozen_window_buffers_take_the_same_bound() {
        for count in [0usize, 1, 9, 69, 1_024] {
            let bound = frozen_window_buffer_bound(count);
            assert!(
                bound >= count,
                "the bound must at least hold the threads already enumerated"
            );
            assert!(bound > count, "and leave room for a burst racing the first pass");
        }
        // Saturating, so an absurd count refuses on `try_reserve` rather than
        // wrapping to a small bound that would then be silently exceeded.
        assert_eq!(frozen_window_buffer_bound(usize::MAX), usize::MAX);
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
    /// Source with comments removed and layout normalised, for the pins below.
    ///
    /// Neither comments nor line breaks are the contract; the statements are.
    /// Both narrowings were bought with real defects:
    ///
    ///  * counting over prose meant a comment that merely NAMED the thing being
    ///    counted failed a CORRECT build -- and only full-line `//` was stripped
    ///    at first, so a trailing `// ...` or a `/* ... */` still did it;
    ///  * matching exact single lines meant a legitimate rustfmt at a narrower
    ///    width broke a pin without changing a single statement.
    ///
    /// Whitespace is collapsed to single spaces, so every `contains` below is
    /// written as one line and matches however the formatter wrapped it.
    ///
    /// HONEST LIMIT: a `//` or `/*` inside a STRING LITERAL is treated as the
    /// start of a comment. The functions scanned here contain no such literal;
    /// a scanner that had to be sound about that would need the lexer, not a
    /// substring pass.
    fn code_only(source: &str) -> String {
        // ONE pass, three states. Two separate passes could not be made
        // correct in either order, and both orders were tried and shipped:
        // blocks-first let a `/*` inside a `//` comment open a block that never
        // closed and silently discarded the rest of the function;
        // lines-first truncated `/* see https://example */` at its `//`,
        // deleting the closing `*/` and doing the same thing. A comment's
        // meaning depends on the state it starts in, so the state has to be
        // carried rather than approximated.
        let mut code = String::with_capacity(source.len());
        let characters: Vec<char> = source.chars().collect();
        let mut block_depth = 0usize;
        let mut in_line_comment = false;
        let mut index = 0usize;
        while index < characters.len() {
            let current = characters[index];
            let next = characters.get(index + 1).copied();
            if in_line_comment {
                // Only the newline ends it. `*/` inside it is text.
                if current == '\n' {
                    in_line_comment = false;
                    code.push('\n');
                }
                index += 1;
                continue;
            }
            if block_depth > 0 {
                // Rust's block comments NEST, and `//` inside one is text.
                if current == '/' && next == Some('*') {
                    block_depth += 1;
                    index += 2;
                    continue;
                }
                if current == '*' && next == Some('/') {
                    block_depth -= 1;
                    if block_depth == 0 {
                        // A block comment SEPARATES tokens. Deleting it without
                        // leaving whitespace turns the valid
                        // `let/* why */bound = ...` into `letbound = ...`, and
                        // the count below then failed a correct build.
                        code.push(' ');
                    }
                    index += 2;
                    continue;
                }
                index += 1;
                continue;
            }
            if current == '/' && next == Some('/') {
                in_line_comment = true;
                index += 2;
                continue;
            }
            if current == '/' && next == Some('*') {
                block_depth += 1;
                index += 2;
                continue;
            }
            code.push(current);
            index += 1;
        }
        assert_eq!(
            block_depth, 0,
            "the scanned source has an unterminated block comment, so this scan \
             saw less of it than the compiler did"
        );
        // Finally, layout. Collapsing whitespace alone was NOT enough to make a
        // match survive rustfmt at a narrower width: wrapping a call across
        // lines also inserts a trailing comma before the `)`, so the collapsed
        // text still differed. Normalising the punctuation is what actually
        // makes these assertions independent of where the formatter broke.
        let collapsed = code.split_whitespace().collect::<Vec<_>>().join(" ");
        collapsed
            .replace("( ", "(")
            .replace(" )", ")")
            .replace(",)", ")")
            .replace(" ,", ",")
            // Method chains: rustfmt at a narrow width breaks BEFORE the dot,
            // so `ids.try_reserve(..)` becomes `ids\n    .try_reserve(..)` and
            // collapsed to `ids .try_reserve(..)`. Same statement, and the
            // assertions must not care where the line broke.
            .replace(" .", ".")
            .replace(". ", ".")
            // A block comment may sit between ANY two tokens, and the space
            // this filter leaves in its place must not become the difference
            // between two spellings of the same call. `foo/* why */(x)` and
            // `Vec/* why */::new()` are both valid and both rustfmt-stable.
            .replace(" (", "(")
            .replace(" ::", "::")
            .replace(":: ", "::")
    }

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
        // Through the same filter as everything else. Bounding the extent fixed
        // the NEIGHBOURING comment; a comment INSIDE the function still broke
        // it, and `// Do not break on an arbitrary Thread32Next error` is
        // exactly the comment this code invites. The positive assertion had the
        // mirror-image hole: prose quoting the classifier satisfied it.
        let enumerate = code_only(enumerate);
        let enumerate = enumerate.as_str();
        assert!(
            enumerate.contains("enumeration_stop_is_end_of_list(code)"),
            "enumerate_thread_ids must classify the Thread32Next stop code; \
             ending the walk on any stop certifies a TRUNCATED thread list"
        );
        assert!(
            !enumerate.contains("break"),
            "a bare `break` in enumerate_thread_ids is the truncation fail-open"
        );
        // The function BODY, and its CODE. Both narrowings are load-bearing for
        // the counting assertions further down, and neither was there when they
        // were written:
        //
        //  * unbounded, `inner` ran to the end of the production source, so a
        //    count of "how many times does this function do X" was really a
        //    count over every function after it as well;
        //  * uncommented, a count includes prose. A comment that merely NAMES
        //    `Growth::Allowed` -- exactly the comment a careful maintainer
        //    writes next to it -- would have failed a correct build. A gate that
        //    punishes documenting the thing it guards does not survive contact
        //    with maintenance.
        let inner_body = production
            .split_once("fn freeze_other_threads_inner(")
            .expect("freeze_other_threads_inner is declared in production")
            .1
            .split_once("\n}\n")
            .expect("freeze_other_threads_inner has a closing brace")
            .0;
        let inner = code_only(inner_body);
        let inner = inner.as_str();
        assert!(
            inner.contains("if pass_is_quiet(newly_suspended, vanished) {"),
            "the fixed point must ask pass_is_quiet with BOTH counts; asking \
             about newly_suspended alone lets a pass that saw a thread vanish \
             be the last one"
        );
        assert_eq!(
            inner.matches("vanished += 1;").count(),
            3,
            "all three accounted-for-but-not-suspended arms must record the \
             vanish: the unopenable id, the refused suspend, and the handle \
             that opened onto another process's thread"
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

        // The suspend-failure arm. This used to be held by a churn test that
        // waited for the OS to catch a thread mid-exit and then asserted the
        // excusal counter had moved -- coverage whose ARRIVAL was a random
        // variable, which is the shape this lane removed from the routine gate.
        // The mutation it existed to kill is a source property and is pinned as
        // one: the disposition has to come from the thread object, and nothing
        // in that arm may consult the error code.
        let suspend_failed = inner
            .find("if previous_count == u32::MAX {")
            .expect("the freeze loop must handle a failed suspend explicitly");
        let suspend_arm_end = inner[suspend_failed..]
            .find("freeze.frozen.len() == freeze.frozen.capacity()")
            .expect("the failed-suspend arm is followed by the capacity guard")
            + suspend_failed;
        let suspend_arm = &inner[suspend_failed..suspend_arm_end];
        // The classifier's result must BE the disposition, not merely be
        // computed near it. Requiring only that the call APPEARS let a mutation
        // keep it as `let _proof = classify_suspend_failure(...)` and derive the
        // real disposition from an error-code table beside it. So: exactly one
        // binding of `disposition` in the arm, and it is the classifier call.
        // Exactly one binding, exactly one classifier call, and the match reads
        // the BINDING. Each of the three is load-bearing and none implies the
        // others: one binding still allowed `match classify_suspend_failure(true)`
        // to decide instead, and requiring `match disposition` still allowed a
        // second classifier call to compute it.
        assert_eq!(
            suspend_arm.matches("let disposition").count(),
            1,
            "the failed-suspend arm binds `disposition` more than once, so which \
             binding the match reads is no longer decided here"
        );
        assert_eq!(
            suspend_arm.matches("classify_suspend_failure").count(),
            1,
            "the failed-suspend arm calls classify_suspend_failure more than \
             once; a second call can decide the match while the first satisfies \
             the pin below"
        );
        assert!(
            suspend_arm.contains("match disposition {"),
            "the failed-suspend arm must match on the `disposition` it bound; \
             matching on anything else leaves that binding computed and unused"
        );
        assert!(
            suspend_arm.contains(
                "let disposition = classify_suspend_failure(thread_has_terminated(handle));"
            ),
            "a failed suspend must be settled by the thread object being \
             signalled; the error-code allowlist this replaced excused only \
             ERROR_INVALID_HANDLE and ERROR_INVALID_PARAMETER, neither of which \
             the OS returns for a suspend, so an ordinary thread exit refused an \
             entire hook installation"
        );
        for code_test in [
            "os_error ==",
            "== os_error",
            "os_error !=",
            "match os_error",
            "contains(&os_error)",
            "matches!(os_error",
        ] {
            assert!(
                !suspend_arm.contains(code_test),
                "the failed-suspend arm consults the error code (`{code_test}`); \
                 which code Windows chose is not the question, whether the thread \
                 can still execute is, and only the thread object answers that"
            );
        }
        // Every mention of a disposition in this arm is a match PATTERN. One
        // CONSTRUCTED here would be a second source of truth beside the
        // classifier, which is the shape of every allowlist this arm has ever
        // grown.
        for disposition in ["ThreadDisposition::Gone", "ThreadDisposition::Refuse"] {
            for occurrence in suspend_arm.match_indices(disposition) {
                let tail = &suspend_arm[occurrence.0 + disposition.len()..];
                assert!(
                    tail.trim_start().starts_with("=>"),
                    "the failed-suspend arm CONSTRUCTS {disposition} instead of \
                     only matching on it, so the disposition has a source other \
                     than the thread object"
                );
            }
        }

        // The verification pass must forbid buffer growth. This is a call-site
        // property and nothing else can see it: the growth test drives
        // enumerate_thread_ids directly, so flipping this one argument to
        // `Growth::Allowed` leaves the whole suite green while a real thread
        // burst reallocates with every other thread suspended -- the exact hang
        // the argument exists to prevent.
        // The CALL, not the `?`. Pinning the propagation operator too rejected
        // an equivalent fail-closed `match` that returns the same Err, which is
        // a correct refactor and none of this test's business.
        assert!(
            inner.contains("enumerate_thread_ids(&mut freeze.enumeration, Growth::Forbidden)"),
            "the per-pass enumeration inside the frozen window must be taken \
             with Growth::Forbidden"
        );
        // And it must be the ONLY enumeration in the window, which is asked of
        // the WINDOW rather than of the whole function. Counting over the whole
        // body also forbade a second enumeration BEFORE the first suspension,
        // where growing a buffer is ordinary and harmless -- a gate has no
        // business refusing that, and the property it exists to protect is
        // about the frozen region alone.
        let frozen_region = inner
            .split_once("for pass in 1..=MAX_PASSES {")
            .expect("the freeze iterates to a fixed point")
            .1;
        assert_eq!(
            frozen_region.matches("enumerate_thread_ids(").count(),
            1,
            "the frozen window enumerates more than once; every extra call is \
             another chance to allocate with every other thread suspended"
        );
        assert!(
            !frozen_region.contains("Growth::Allowed"),
            "the frozen window takes an enumeration that may GROW its buffer; \
             growing it there takes the allocator's lock, which a suspended \
             thread may hold, and there is nobody left to release it"
        );
        // Naming three constructs was never "no allocation" -- it was a list of
        // the three that had gone wrong. A refactor collecting `still_running`
        // into a Vec, or a `format!` added for a diagnostic, allocates in the
        // window and passed every pin.
        // Fragments, not full call spellings: `.collect()` missed
        // `collect::<Vec<_>>()`, and `Vec::with_capacity` missed
        // `String::with_capacity`. A roster of exact calls is a list of the
        // ones already thought of.
        //
        // STILL A ROSTER, not a proof. It cannot see a helper that allocates
        // inside itself, and no substring pass can. What it catches is the
        // shape an ordinary edit takes -- building a message, collecting an
        // iterator, creating a vector, taking a lock -- which is every
        // occurrence of this defect the crate has actually had.
        //
        // `push` is deliberately ABSENT and its absence is the rule, not an
        // omission: the window's whole design is buffers reserved before the
        // first suspension and pushed into afterwards, so a push into reserved
        // capacity is the CORRECT shape here and the capacity guard beside it
        // is what keeps it from growing. `Vec::new` and `reserve(` are banned
        // for the same reason they would be allocations.
        //
        // lib.rs scans its own held-freeze windows for the same defect with a
        // shorter exact-spelling list of its own. The two are NOT shared, and
        // this comment does not claim they are: adding a construct here does
        // not protect lib.rs's windows, and vice versa.
        for allocating in [
            concat!("Vec::", "new"),
            concat!("Default::", "default"),
            concat!("reser", "ve("),
            concat!("with_cap", "acity"),
            concat!("coll", "ect"),
            concat!("form", "at!"),
            concat!("to_st", "ring"),
            concat!("to_o", "wned"),
            concat!("to_v", "ec"),
            concat!(".clo", "ne("),
            concat!("Box::", "new"),
            concat!("String::", "from"),
            concat!("Vec::", "from"),
            concat!("vec", "!["),
            concat!("print", "ln!"),
            concat!("lo", "ck("),
        ] {
            assert!(
                !frozen_region.contains(allocating),
                "the frozen window contains `{allocating}`, which can allocate or \
                 block while every other thread in the process is suspended; the \
                 allocator's lock may be held by one of them and there is nobody \
                 left to release it"
            );
        }
        // Counting CALLS only works if the name is always a call. Binding the
        // function as a value -- `let alias = enumerate_thread_ids;` -- and
        // calling the alias inside the window would keep the count right while
        // adding an enumeration.
        for occurrence in inner.match_indices("enumerate_thread_ids") {
            let tail = &inner[occurrence.0 + "enumerate_thread_ids".len()..];
            assert!(
                tail.starts_with('('),
                "enumerate_thread_ids is used as a value rather than called, so \
                 the enumerations in this function can no longer be counted"
            );
        }

        // Both frozen-window buffers must be sized from the SAME bound. The
        // bound's own test can only say the function is sensible; that both
        // reservations call it is a call-site property, and reserving the
        // headroom alone for the handle table is the shape that let a burst fit
        // in the id list and overflow the list of their handles.
        assert!(
            inner.contains("let bound = frozen_window_buffer_bound(ids.len());"),
            "the frozen-window bound must be derived once, from the thread count \
             seen before the first suspension"
        );
        // Once, and never shadowed. `let bound = bound.saturating_sub(..)`
        // between the two reservations satisfies every substring below while
        // recreating exactly the headroom-only handle table this replaced.
        assert_eq!(
            inner.matches("let bound").count(),
            1,
            "the frozen-window bound is rebound; the two reservations then take \
             different values while still naming `bound`"
        );
        assert!(
            inner.contains("ids.try_reserve(bound.saturating_sub(ids.len()))"),
            "the enumeration buffer must be reserved to the shared bound"
        );
        assert!(
            inner.contains("frozen.try_reserve(bound)"),
            "the suspended-handle table must be reserved to the SAME shared \
             bound, not to the headroom alone"
        );
        // Reserving it correctly and then REPLACING it is the same defect with
        // an extra line: `frozen = Vec::with_capacity(bound - ids.len());` after
        // the reservation keeps every substring above and restores the historic
        // headroom-only table. There is exactly one `frozen` binding here and it
        // is never reassigned, and no capacity is ever chosen by any means but
        // the shared bound.
        assert_eq!(
            inner.matches("frozen = ").count(),
            0,
            "the suspended-handle table is reassigned after it was reserved; \
             whatever replaces it did not come from the shared bound"
        );
        // WHAT THIS PAIR OF PINS DOES NOT ESTABLISH, said plainly rather than
        // implied away: that the vector reserved above is the one that becomes
        // `ProcessFreeze::frozen`. A mutation can reserve correctly and then
        // hand the struct a different vector -- `frozen.into_boxed_slice()
        // .into_vec()` discards the excess capacity without ever naming
        // `Vec::with_capacity` or reassigning `frozen`. No substring scan can
        // close that; establishing it needs the borrow checker's own view of
        // which value moved where. What this DOES catch is every accidental
        // shape: the reservation deleted, sized from something other than the
        // shared bound, or the bound shadowed between the two calls.

        // The ownership arm, pinned for the same reason and in the same way:
        // its soundness is an ORDERING claim too. `GetProcessIdOfThread` has to
        // be asked about the handle we just opened, and answered, BEFORE that
        // handle is suspended. A mutation that moves the check after the
        // `SuspendThread`, or that suspends first and closes later, leaves
        // `classify_opened_thread_owner`'s own test green while the freeze goes
        // back to stopping another process's threads.
        let owner_read = inner
            .find("GetProcessIdOfThread(handle)")
            .expect("every opened handle must be asked which process owns it");
        let owner_classified = inner
            .find("classify_opened_thread_owner(owner,")
            .expect("the owner the kernel reported must be the disposition");
        let suspended = inner
            .find("SuspendThread(handle)")
            .expect("the freeze loop must suspend the threads it opened");
        assert!(
            opened < owner_read && owner_read < owner_classified && owner_classified < suspended,
            "ownership must be established between the open and the suspend; \
             suspending first and asking afterwards has already stopped a \
             thread that may not be ours"
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
    ///
    /// # Why this is `#[ignore]`d, and what still runs in the routine gate
    ///
    /// It waits for TWO shapes, and the second is rare: a thread caught between
    /// `SuspendThread` refusing it and its thread object signalling. Measured
    /// over 13 runs of the routine gate it arrived in anywhere from 1.7 s to
    /// 176.8 s -- a 105x spread on one test -- and one of those 13 runs failed
    /// outright. A default-tier test has to be deterministic; this one's
    /// runtime and its verdict are both functions of how the OS scheduled 64
    /// churn threads that minute, which is why it read as a hang and why it
    /// went red without anything being wrong.
    ///
    /// Nothing was deleted to achieve that. The two mutations this exists to
    /// kill are both still killed, in a tier each:
    ///
    ///  * the HISTORICAL allowlist -- the one that refused threads which had in
    ///    fact terminated -- dies in the routine gate, in
    ///    [`the_fail_open_decisions_are_taken_only_through_their_classifiers`],
    ///    which pins the failed-suspend arm to a single
    ///    `classify_suspend_failure(thread_has_terminated(handle))` that the
    ///    match must consume, and in the behavioural churn test beside it. An
    ///    allowlist that EXCUSES on the codes the OS really returns is not
    ///    killed by either, and the note on that test says so rather than
    ///    claiming otherwise;
    ///  * a zero [`TERMINATION_PROOF_TIMEOUT_MS`] dies in the routine gate in
    ///    TWO deterministic places:
    ///    [`only_a_delayed_signal_counts_as_a_waited_termination_proof`] pins
    ///    the rule (only a DELAYED signal is counted as a wait that paid off)
    ///    by driving [`record_termination_probe`] directly, and
    ///    [`the_termination_proof_actually_spends_the_bounded_wait`] pins the
    ///    duration by waiting on a handle that never signals and measuring
    ///    what was spent.
    ///
    /// What is left here is the one claim neither of those can make: that the
    /// delayed-signal population is NON-EMPTY against a real OS. That is worth
    /// keeping and worth paying minutes for, in `-Mode Evidence`, under the
    /// evidence profile's own timeout -- not in every Final.
    #[test]
    #[ignore = "evidence tier: waits for a rare OS interleaving; 1.7-176.8 s measured over 13 runs"]
    fn a_termination_proof_that_needed_the_bounded_wait_is_observed_against_the_os() {
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

    /// Falsifiable, BEHAVIOURALLY: fails if an ordinary thread exit refuses a
    /// freeze, and fails if that path was never reached.
    ///
    /// kills: an error-code allowlist that REFUSES a thread which has in fact
    /// terminated -- the historical defect, where excusing only
    /// ERROR_INVALID_HANDLE and ERROR_INVALID_PARAMETER refused an entire hook
    /// installation about one run in ten because the OS reports
    /// ERROR_ACCESS_DENIED instead.
    ///
    /// What it does NOT kill, stated because the honest limit is short and the
    /// overclaim is not: an allowlist that EXCUSES on the codes the OS actually
    /// returns. The mid-exit failures under this churn are ERROR_ACCESS_DENIED,
    /// and a mutation that excused exactly those would move the same counter by
    /// the same amount. Neither this nor the source pin beside it can separate
    /// "Gone because the thread object was signalled" from "Gone because the
    /// code was 5" -- doing that needs a live thread whose suspend fails with 5,
    /// which is not a state a test can arrange. The source pin narrows the
    /// SHAPES that mutation can take; it does not eliminate it, and no test here
    /// claims to.
    ///
    /// # Why this exists beside the source pin, and why it is not the old test
    ///
    /// [`the_fail_open_decisions_are_taken_only_through_their_classifiers`]
    /// pins this arm's TEXT, which is deterministic but only lexical: a
    /// mutation can compute an allowlist before the arm, or spell the test as
    /// `[..].contains(&os_error)`, `matches!`, or a helper call, and satisfy
    /// every forbidden-string check. Only running the thing catches those. So
    /// the lexical pin catches the crude mutation on every run, and this
    /// catches the clever one.
    ///
    /// It is NOT the test this lane removed. That one waited for BOTH the
    /// common excusal and the rare delayed signal, and measured over 13 runs it
    /// took 1.7 s to 176.8 s and failed once. This waits only for the common
    /// shape -- a suspend failure excused by a thread object that is already
    /// signalled -- which the same 13 runs put at 0.6 s to 2.9 s, and 10 runs
    /// of the final tree at 0.52 s to 1.82 s. The budget below is 30 s: a 16x
    /// margin on the measured worst case, after which it FAILS BY NAME rather
    /// than running on.
    ///
    /// The bound is honest about its own edges. The deadline is checked BETWEEN
    /// rounds, so the true worst case is 30 s plus one freeze plus the churn
    /// join, not exactly 30 s; and the OS race is empirical, so a correct build
    /// CAN fail here if the scheduler never once catches a thread mid-exit in
    /// that window. 23 measured runs say that is remote, and the workspace's
    /// `terminate-after` is the hard backstop; it is a residual risk taken
    /// deliberately, not an absence of one.
    ///
    /// [`FreezeRefusal::NotQuiescent`] is TOLERATED: the churn is deliberately
    /// pathological and a fixed point that cannot be reached under it is the
    /// freeze correctly refusing to certify a process that never went quiet.
    /// EVERY other refusal fails, which is where the coverage is.
    #[test]
    fn a_thread_exiting_mid_enumeration_is_excused_by_its_signalled_thread_object() {
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

        // Bounded by WALL CLOCK, not by a round count. Rounds are not the cost
        // that matters and a round cap says nothing about how long the gate
        // will sit here; a deadline does, and it is the number that can be
        // justified against a measurement.
        let budget = std::time::Duration::from_secs(30);
        let deadline = std::time::Instant::now() + budget;
        let excused_before = suspend_failures_excused_as_terminated();
        let foreign_before = open_successes_excused_as_another_processes();
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
            if excused > 0 {
                break Ok((rounds, excused));
            }
            if std::time::Instant::now() >= deadline {
                break Ok((rounds, 0));
            }
        };
        stop.store(true, Ordering::Release);
        churn.join().expect("churn thread joins");
        // Both recycled-id populations are reported on failure: distinguishing
        // "the recycled-id paths are broken" from "they were never reached" is
        // otherwise guesswork, and they are the paths most likely to explain an
        // unexpected refusal shape.
        let recycled = open_failures_excused_as_no_longer_ours() - recycled_before;
        let foreign = open_successes_excused_as_another_processes() - foreign_before;
        match outcome {
            Err((round, refusal)) => panic!(
                "round {round}: a thread exiting mid-enumeration must not refuse \
                 the freeze: {refusal} ({recycled} open failure(s) and {foreign} \
                 opened handle(s) had already been excused as no-longer-ours \
                 during this run)"
            ),
            Ok((rounds, 0)) => panic!(
                "{rounds} freezes under churn in {budget:?} never caught a thread \
                 exiting mid-suspend, so this test proved nothing; measured \
                 arrival on this machine was 0.6-2.9 s, so either the churn has \
                 stopped producing exits or the excusal path is no longer \
                 reached -- make the churn harder rather than letting it pass \
                 vacuously"
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
