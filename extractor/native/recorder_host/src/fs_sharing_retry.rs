//! Waiting out a Windows sharing violation on the recorder's FINALIZE path.
//!
//! # The defect
//!
//! On Windows a real-time antimalware scanner opens a file the moment it is
//! created or last written, with a share mode that excludes other openers.
//! While that handle is alive any other `CreateFile` asking for an incompatible
//! access fails with `ERROR_SHARING_VIOLATION` (32):
//!
//! ```text
//! The process cannot access the file because it is being used by another
//! process. (os error 32)
//! ```
//!
//! Three such failures were recorded in this repository across 2026-08-15/16,
//! all in test code, and lane KY closed those with `portable_test_fs_retry`.
//! This module is the PRODUCTION half of the same window, and it is a separate
//! module on purpose: that crate says in its own documentation that its budget
//! is a test ceiling and that no production path may adopt it without its own
//! evidence. The classification is deliberately identical — waiting on more
//! than one error code would be a different policy pretending to be the same
//! one — and the budget is stated here, for this path, with its own reason.
//!
//! # Where it may be used, and where it may NOT
//!
//! Only where BOTH hold:
//!
//! 1. the bytes being reopened were written by this process (or by a process
//!    this one has already synchronised with) and were already proven complete
//!    by a successful read, so a wait can only recover the same bytes; and
//! 2. the game is not synchronised with the caller, so a stall here cannot
//!    become a stall of the engine.
//!
//! ## What condition 2 is, and what it is NOT
//!
//! It is NOT "the target process has exited". An earlier draft of this module
//! said that, and it was simply wrong: the recorder is an OBSERVER. It holds no
//! termination right over the target (`"target_termination_right": false` on
//! the target lease), and in the ordinary attach case the humans are still
//! playing while post-capture finalization runs. A reviewer refuted the claim
//! by pointing at the retained `early_render_process` and at the recovery gate's
//! own comment, which says in as many words that finalization can run while the
//! captured game is up.
//!
//! A SECOND draft claimed instead that by this point the recorder has let go of
//! everything the engine could block on. The same reviewer refuted that too, and
//! narrowly: the data pipe reaches EOF when the collector's writer returns, but
//! the collector still holds its hooks and runs `finish_early_render_lifecycle`
//! afterwards, so "every collector-side path is finished" is not established.
//! It is recorded here rather than quietly dropped, because the tempting fix is
//! to assert it again in different words.
//!
//! A THIRD draft argued that the surrounding finalize work already takes
//! minutes (t30: `host_finalize.finalize_seconds = 315.7`), so a 2000 ms wait
//! could not introduce a class of stall the run did not already have. The same
//! reviewer refuted that as well, and it was right on three counts: the G2 seal
//! runs BEFORE `finalize_started`, `--duration-s 0` is accepted, and the
//! production early-launch raw-debug mode skips every fold and container build.
//! The surrounding work is not reliably minutes, so it cannot carry the
//! argument. The number stays in this file only as a scale note on the budget,
//! never as a licence.
//!
//! The claim this module actually rests on is a BARRIER. It has to be stated
//! per ROUTE, because there are exactly two that reach a G2 read and they are
//! not the same — a fourth draft gave one list for both and named steps that do
//! not run on the G2 route at all. Steps are NAMED CALLS rather than line
//! numbers, because a line number drifts on the next insertion and quietly
//! stops describing anything.
//!
//! **Route 1, the live capture.** G2 is only reachable under a normal
//! production EARLY LAUNCH: `--g2-capture-intent` refuses unless the profile is
//! production and `early_launch` is present, and it refuses a diagnostic
//! no-COMMIT probe and an acceptance test outright. So the seal is always
//! inside `recorder_host/src/main.rs::run`'s `early_render_process.is_some()`
//! arm, and on that arm both collector channels are resolved before it:
//!
//! * the authenticated H4CT control service has RETURNED and its outcome has
//!   been consumed (`early_render_host_channel`'s
//!   `serve_vfs_payload_requests`), and
//! * the data-pipe thread has been JOINED (the `pipe_joiner` join), which is
//!   what H4CR EOF proves — the collector has left its capture loop.
//!
//! Only then does `early_render_launch::seal_g2_capture_artifacts` run.
//!
//! The installed-hook lease is deliberately NOT in that list. `direct_hook_lease`
//! is assigned only on the DIRECT-attach arm, the one without an early-render
//! process, so `finalize_installed_hook_lease` and
//! `publish_capture_teardown_receipt` never run on the G2 route and citing them
//! as part of this barrier would be citing an event that does not happen.
//!
//! **Route 2, repack and recovery.** `--fold-and-repack-existing-run-dir`
//! reaches `read_stable_g2_packaging_file` through
//! `fold_and_repack_existing_run_dir`, an entry point that opens no collector
//! channel and injects nothing: there is no target process of ours to stall.
//!
//! On both routes the host is answering no collector request by the time these
//! reads run, so there is nothing for a collector-side wait to be satisfied BY.
//! A collector still executing `finish_early_render_lifecycle` does not need the
//! host to finish it, and one that somehow did would already be waiting on
//! channels nobody is serving — with or without this module, and whatever the
//! retry budget is.
//!
//! That is the whole argument. It is about an ordering this file does not own,
//! so if those calls are ever reordered relative to the seal, or if G2 is ever
//! admitted on a route without an early launch, this module's licence moves
//! with them.
//!
//! It must NOT be used on the hourly capture path, on the attach path, on the
//! control-channel path, or anywhere between creating the game process
//! suspended and resuming it. A bounded retry in any of those places IS a
//! bounded stall of the engine, and "never make the engine wait" is a standing
//! ruling rather than a tunable.
//!
//! # What is retried, and what deliberately is not
//!
//! Raw code 32 only.
//!
//! * `NotFound` is instant and loud — a missing artifact is a real capture
//!   defect and waiting cannot produce one.
//! * `PermissionDenied` (`ERROR_ACCESS_DENIED`, 5) is a real refusal. An ACL or
//!   a delete-pending file never becomes readable by waiting.
//! * `ERROR_LOCK_VIOLATION` (33) describes a byte-range lock, not an
//!   incompatible share mode. No recorded failure was one.
//! * A truncated, mutated or corrupt artifact does not raise 32 at all: it
//!   either reads fine and fails the digest comparison that already follows
//!   every one of these reads, or it fails with a different error that passes
//!   straight through. This is the reason a retry here cannot mask a real
//!   defect — the retried class is a property of who ELSE holds the file, never
//!   a property of its contents.
//! * A violation that does not clear inside the budget still fails, with the
//!   last error returned unchanged, so the caller's message still carries
//!   `os error 32` and the operator can see what happened.
//!
//! ## What the budget promises, exactly
//!
//! No retry is COMMITTED once a check observes [`FINALIZE_RETRY_BUDGET`]
//! exhausted. The loop checks twice per iteration — before committing to a
//! backoff and again after it.
//!
//! It is deliberately not stated as "no attempt begins after the deadline"
//! (the thread can be descheduled between the check and the `CreateFile`) and
//! not as a wall-clock ceiling on the call (the attempt in flight runs to
//! completion and the OS may oversleep). The guarantee is about what this code
//! DECIDES.
//!
//! # Off Windows
//!
//! The loop is not compiled at all. `raw_os_error() == Some(32)` is `ENOTTY` on
//! Linux and has nothing to do with sharing; keying a retry on the raw number
//! there would be a defect rather than a fix.

use std::io;
use std::path::Path;
use std::time::Duration;
#[cfg(windows)]
use std::time::Instant;

/// The total time one THREAD may spend waiting out scanners, for its whole
/// life — not a per-call allowance.
///
/// Two seconds. Three numbers set the size:
///
/// * the observed scanner window is single-digit milliseconds, so this is
///   ~1000x headroom and a thread that exhausts it is reporting a persistent
///   holder rather than a scanner;
/// * for scale, an ordinary finalize of a large recording was measured in
///   MINUTES (t30: 315.7 s for one 341 MB / 129-hour stream) — a scale note on
///   the SIZE of the budget, and deliberately not a safety argument: a short
///   capture finalizes quickly and the seal runs before the timed work, which
///   is why the licence above rests on a barrier instead;
/// * losing the artifact is the alternative, and a G2 capture that has already
///   been admitted cannot be produced again.
///
/// # Why it is a whole-thread allowance and not a per-call budget
///
/// Because a per-call budget is not a bound. One `seal_g2_capture_artifacts`
/// performs EIGHT retried reads (two admissions of two reads each, plus two
/// intervening pairs), and packaging performs FOUR more per replay container —
/// `resolve_recorded_g2_engine_capture` reads two artifacts and
/// `read_stable_g2_packaging_file` reads each of them twice to prove it did not
/// change while being read. The container writer is called from inside the loop
/// over `descriptor.ordered_streams`, so the count is `8 + 4R` for a recording
/// with `R` rollover members. A 2000 ms per-call budget therefore had a worst
/// case of `16 + 8R` seconds, which GROWS WITH THE RECORDING. A reviewer raised
/// exactly that, and it was right: "bounded" has to mean bounded by something
/// that is not a property of the capture. (It also had to correct this count
/// twice — the first version of this comment said two per container, which
/// forgot that each artifact is read twice.)
///
/// [`spend_wait_allowance`] makes the bound real. What is charged is the WALL
/// TIME AROUND EACH SLEEP CALL -- see [`Clock::sleep`] for exactly what that
/// does and does not include, because it is not the same thing as "time asleep"
/// and this file used to say it was. Nothing is charged for the operation
/// itself, so a finalize with no scanner interference spends NOTHING and every
/// read still gets the whole allowance; and once a thread has spent it, every
/// later call on that thread is a single attempt with no wait at all.
///
/// The direction of the residue matters and it is the safe one. Descheduling
/// around a sleep is charged, and descheduling is not bounded, so a badly
/// loaded machine can exhaust the allowance EARLY -- meaning a later artifact
/// gets one attempt instead of a wait. That degrades to the behaviour this
/// module replaced, which is a refusal that names `os error 32`; it can never
/// overrun the bound.
///
/// Per-thread rather than per-process because the quantity being bounded is
/// WALL-CLOCK STALL: two threads each waiting 2 s in parallel still cost the
/// run 2 s. It also means a test that deliberately exhausts the allowance
/// cannot leak that state into a test running beside it.
///
/// The size is provisional in one direction only: if a real capture ever
/// exhausts it, the exhaustion is named in the error, and THAT measurement —
/// not this comment — is what would change the number.
pub const FINALIZE_RETRY_BUDGET: Duration = Duration::from_millis(2000);

thread_local! {
    /// Wall time this thread has already been charged for waiting out scanners:
    /// the intervals around its sleep calls, not a sleep-only measurement.
    static WAIT_SPENT: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
}

/// What is left of this thread's allowance.
fn remaining_wait_allowance() -> Duration {
    WAIT_SPENT.with(|spent| FINALIZE_RETRY_BUDGET.saturating_sub(spent.get()))
}

/// Charge `waited` against this thread's allowance.
fn spend_wait_allowance(waited: Duration) {
    WAIT_SPENT.with(|spent| spent.set(spent.get().saturating_add(waited)));
}

/// Restore the allowance. Test-only: production never gives time back, which is
/// the whole point of the bound.
#[cfg(test)]
pub(crate) fn reset_wait_allowance_for_test() {
    WAIT_SPENT.with(|spent| spent.set(Duration::ZERO));
}

/// How many artifacts this thread has read through
/// [`read_proven_finalize_artifact`].
///
/// Test-only. It exists because the recovery proof can only demonstrate ONE
/// read waiting — the handle it releases is released once — so on its own it
/// would still pass with any LATER read reverted to a plain `std::fs::read`.
/// Counting the calls closes that: a read that stops going through this module
/// stops being counted.
#[cfg(test)]
thread_local! {
    static READS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn take_proven_read_count() -> u32 {
    READS.with(|reads| reads.replace(0))
}

/// First backoff sleep. Doubles up to [`MAX_BACKOFF`].
#[cfg(windows)]
const FIRST_BACKOFF: Duration = Duration::from_millis(1);

/// Backoff ceiling. Small on purpose: the window is milliseconds, so a large
/// sleep would turn a 3 ms stall into a 64 ms one.
#[cfg(windows)]
const MAX_BACKOFF: Duration = Duration::from_millis(16);

/// `ERROR_SHARING_VIOLATION` — another process holds the file with a share mode
/// that excludes this open. The only code this module waits on.
#[cfg(windows)]
const ERROR_SHARING_VIOLATION: i32 = 32;

/// Is this the transient class, i.e. the one worth waiting out?
///
/// False for every error on non-Windows targets.
pub fn is_transient_sharing_violation(error: &io::Error) -> bool {
    #[cfg(windows)]
    {
        error.raw_os_error() == Some(ERROR_SHARING_VIOLATION)
    }
    #[cfg(not(windows))]
    {
        let _ = error;
        false
    }
}

/// The passage of time, so the policy can be tested without spending it.
#[cfg(windows)]
trait Clock {
    /// Time elapsed since the call began.
    fn elapsed(&self) -> Duration;
    /// Wait, and report the WALL TIME AROUND THE SLEEP CALL.
    ///
    /// Be precise about what that is, because two drafts were not. It is not
    /// "sleep and nothing else": userland cannot measure that. The thread can be
    /// descheduled between taking the start reading and entering
    /// `thread::sleep`, and again between the sleep returning and the end
    /// reading, and both intervals land in this number. Exact sleep-only
    /// accounting would need OS-level wait accounting, which is not worth a
    /// dependency for a 2000 ms allowance.
    ///
    /// What it IS: measured inside the implementation, around the sleep call
    /// and nothing further out. An earlier version had `retry_core` bracket the
    /// call with two `elapsed()` readings, which billed the deadline check and
    /// the backoff arithmetic ON TOP of the same scheduling intervals this one
    /// has. Narrowing it to here removes what can be removed; it does not, and
    /// cannot, remove the scheduling intervals themselves.
    ///
    /// The residue is CONSERVATIVE in direction, and its SIZE is not bounded.
    /// Do not describe it as slight: descheduling is unbounded, so on a heavily
    /// loaded machine the charge can exceed the sleep by an arbitrary amount
    /// and the allowance can be exhausted well before 2000 ms of waiting has
    /// actually happened. What that costs is a later artifact getting a single
    /// attempt instead of a wait — a refusal that names `os error 32`, which is
    /// the behaviour this module replaced. It can never overrun the bound, and
    /// that asymmetry is the whole reason this measurement is acceptable.
    fn sleep(&mut self, duration: Duration) -> Duration;
}

#[cfg(windows)]
struct RealClock(Instant);

#[cfg(windows)]
impl Clock for RealClock {
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
    fn sleep(&mut self, duration: Duration) -> Duration {
        let entered = Instant::now();
        std::thread::sleep(duration);
        entered.elapsed()
    }
}

/// A test-only observer of transient failures, reached from inside the loop.
///
/// It exists so this crate's load-bearing proof can call the REAL production
/// entry point — `seal_g2_capture_artifacts`, not a parallel copy of the loop —
/// and still release the handle that caused the sharing violation at a
/// deterministic moment. Releasing it from a timer thread instead would produce
/// a test that can pass with the retry deleted whenever the main thread is
/// descheduled, which is exactly the vacuity this module exists to avoid.
///
/// Compiled only into this crate's own test build; the shipping build gets an
/// empty inline function and there is no way to install a hook from outside.
#[cfg(all(windows, test))]
pub(crate) mod transient_hook {
    use std::cell::RefCell;

    thread_local! {
        static HOOK: RefCell<Option<Box<dyn FnMut()>>> = const { RefCell::new(None) };
    }

    /// Installs `hook` until the returned value is dropped, so a panicking test
    /// cannot leave one armed for the next test on this thread.
    pub(crate) struct Installed;

    impl Drop for Installed {
        fn drop(&mut self) {
            HOOK.with(|hook| *hook.borrow_mut() = None);
        }
    }

    pub(crate) fn install(hook: impl FnMut() + 'static) -> Installed {
        HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
        Installed
    }

    pub(super) fn fire() {
        // Taken out of the slot for the call so a hook that itself reaches a
        // retrying operation cannot re-enter the `RefCell`.
        let taken = HOOK.with(|slot| slot.borrow_mut().take());
        if let Some(mut hook) = taken {
            hook();
            HOOK.with(|slot| {
                let mut slot = slot.borrow_mut();
                if slot.is_none() {
                    *slot = Some(hook);
                }
            });
        }
    }
}

#[cfg(all(windows, test))]
fn fire_transient_hook() {
    transient_hook::fire();
}

#[cfg(not(all(windows, test)))]
fn fire_transient_hook() {}

/// The retry loop. The give-up error is the last one the operation produced,
/// untouched.
///
/// `slept` accumulates what [`Clock::sleep`] reports -- the wall time around
/// each sleep call, which is as close to "time asleep" as userland gets. The
/// time the OPERATION itself takes is deliberately not charged: a slow disk is
/// not a scanner, and charging it would shrink the allowance for reasons that
/// have nothing to do with the window being waited out.
#[cfg(windows)]
fn retry_core<T>(
    budget: Duration,
    operation: &str,
    mut op: impl FnMut() -> io::Result<T>,
    clock: &mut dyn Clock,
    slept: &mut Duration,
) -> io::Result<T> {
    let mut backoff = FIRST_BACKOFF;
    let mut attempts = 0_u32;
    loop {
        attempts += 1;
        match op() {
            Ok(value) => {
                if attempts > 1 {
                    eprintln!(
                        "recorder note: {operation} recovered from a sharing violation \
                         (os error {ERROR_SHARING_VIOLATION}) after {attempts} attempts, {:?}",
                        clock.elapsed()
                    );
                }
                return Ok(value);
            }
            Err(error) => {
                if !is_transient_sharing_violation(&error) {
                    return Err(error);
                }
                fire_transient_hook();
                // The budget bounds what this loop COMMITS to, so it is checked
                // TWICE: once before committing to a sleep, and again after it.
                // Without the second check the truncated final sleep would land
                // exactly on the budget and the loop would still commit to one
                // more attempt past it.
                let elapsed = clock.elapsed();
                if elapsed >= budget {
                    return Err(error);
                }
                let remaining = budget - elapsed;
                let nap = backoff.min(remaining);
                // MEASURED, not requested, and measured by the clock itself.
                // `Sleep` on Windows rounds up to the timer resolution -- a
                // requested 1 ms routinely costs 15 ms on a loaded machine --
                // so charging the REQUESTED nap systematically under-charges
                // and the whole-thread bound stops being a wall-clock bound.
                // The battery caught exactly that. Bracketing the call with
                // `elapsed()` readings HERE would fix the undercharge and add a
                // wider overcharge -- the deadline check and the backoff
                // arithmetic on top of the scheduling intervals `Clock::sleep`
                // documents, which no measurement in userland can exclude.
                *slept = slept.saturating_add(clock.sleep(nap));
                backoff = (backoff * 2).min(MAX_BACKOFF);
                if clock.elapsed() >= budget {
                    return Err(error);
                }
            }
        }
    }
}

/// Run `op` against whatever is LEFT of this thread's allowance, and charge the
/// wall time it spends around its sleeps.
///
/// Split from [`run_within_allowance`] so the allowance ARITHMETIC can be
/// proved against a fake clock, exactly and without spending any real time.
#[cfg(windows)]
fn run_within_allowance_on<T>(
    operation: &str,
    op: impl FnMut() -> io::Result<T>,
    clock: &mut dyn Clock,
) -> io::Result<T> {
    let mut slept = Duration::ZERO;
    let outcome = retry_core(remaining_wait_allowance(), operation, op, clock, &mut slept);
    spend_wait_allowance(slept);
    outcome
}

#[cfg(windows)]
fn run_within_allowance<T>(
    operation: &str,
    op: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    run_within_allowance_on(operation, op, &mut RealClock(Instant::now()))
}

/// One attempt, on a target where the sharing violation does not exist.
#[cfg(not(windows))]
fn run_within_allowance<T>(
    _operation: &str,
    mut op: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    op()
}

/// [`std::fs::read`] of a finalize-path artifact whose bytes this process has
/// already proven complete, waiting out a scanner for whatever is left of this
/// thread's [`FINALIZE_RETRY_BUDGET`].
///
/// Read the module header before adding a call site: the two preconditions
/// there are what make the wait safe, and neither is checkable from here.
pub fn read_proven_finalize_artifact(path: &Path) -> io::Result<Vec<u8>> {
    #[cfg(test)]
    READS.with(|reads| reads.set(reads.get() + 1));
    run_within_allowance("read", || std::fs::read(path))
}

// There is deliberately no `read_dir` wrapper. A directory is not exposed to
// this window the way a file is — the scanner holds the FILE it just saw
// written — and on Windows `DirEntry::file_type` is answered from the
// `FIND_DATA` the enumeration already returned, so listing a directory whose
// child is held does not touch the held child at all. Wrapping it would be
// scaffolding that suggests a protection it does not provide.

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    struct FakeClock {
        now: Duration,
        slept: Vec<Duration>,
    }

    #[cfg(windows)]
    impl Clock for FakeClock {
        fn elapsed(&self) -> Duration {
            self.now
        }
        fn sleep(&mut self, duration: Duration) -> Duration {
            self.slept.push(duration);
            self.now += duration;
            duration
        }
    }

    /// kills: widening the retried class to any OS error, to `Uncategorized`,
    /// to message text, or to code 33/5; and deleting the `NotFound` fast path
    /// by classifying on anything other than the raw code.
    #[test]
    fn only_raw_code_32_is_the_transient_class() {
        let sharing = io::Error::from_raw_os_error(32);
        let lock = io::Error::from_raw_os_error(33);
        let denied = io::Error::from_raw_os_error(5);
        let not_found = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(is_transient_sharing_violation(&sharing), cfg!(windows));
        assert!(!is_transient_sharing_violation(&lock));
        assert!(!is_transient_sharing_violation(&denied));
        assert!(!is_transient_sharing_violation(&not_found));
    }

    /// kills: an unbounded loop; a constant or uncapped backoff; sleeping past
    /// the deadline; checking the deadline only after the sleep.
    #[cfg(windows)]
    #[test]
    fn the_backoff_schedule_and_the_deadline_are_exactly_what_is_documented() {
        let mut clock = FakeClock {
            now: Duration::ZERO,
            slept: Vec::new(),
        };
        let mut attempts = 0_u32;
        let outcome = retry_core::<()>(
            Duration::from_millis(50),
            "test",
            || {
                attempts += 1;
                Err(io::Error::from_raw_os_error(32))
            },
            &mut clock,
            &mut Duration::ZERO,
        );
        assert_eq!(outcome.unwrap_err().raw_os_error(), Some(32));
        let slept_ms: Vec<u64> = clock.slept.iter().map(|d| d.as_millis() as u64).collect();
        // 1,2,4,8,16,16 = 47 ms, then the last sleep is truncated to land
        // exactly on 50 ms, and the post-sleep check stops the loop there.
        assert_eq!(slept_ms, vec![1, 2, 4, 8, 16, 16, 3]);
        assert_eq!(clock.now, Duration::from_millis(50));
        assert_eq!(attempts, 7);
    }

    /// kills: treating the budget as "always at least one retry".
    #[cfg(windows)]
    #[test]
    fn a_spent_budget_starts_no_further_attempt() {
        let mut clock = FakeClock {
            now: Duration::from_millis(5),
            slept: Vec::new(),
        };
        let mut attempts = 0_u32;
        let outcome = retry_core::<()>(
            Duration::from_millis(1),
            "test",
            || {
                attempts += 1;
                Err(io::Error::from_raw_os_error(32))
            },
            &mut clock,
            &mut Duration::ZERO,
        );
        assert!(outcome.is_err());
        assert_eq!(attempts, 1);
        assert!(clock.slept.is_empty());
    }

    /// kills: retrying anything that is not the transient class — a `NotFound`
    /// artifact must fail on the first attempt, because waiting cannot produce
    /// a file the capture never wrote.
    #[cfg(windows)]
    #[test]
    fn a_non_transient_error_fails_on_the_first_attempt() {
        let mut clock = FakeClock {
            now: Duration::ZERO,
            slept: Vec::new(),
        };
        let mut attempts = 0_u32;
        let outcome = retry_core::<()>(
            Duration::from_millis(5000),
            "test",
            || {
                attempts += 1;
                Err(io::Error::from_raw_os_error(5))
            },
            &mut clock,
            &mut Duration::ZERO,
        );
        assert_eq!(outcome.unwrap_err().raw_os_error(), Some(5));
        assert_eq!(attempts, 1);
        assert!(clock.slept.is_empty());
    }

    /// The load-bearing proof for the reader itself: a real `share_mode(0)`
    /// handle — the scanner's exact shape — is held across the first attempt
    /// and released from inside the loop's own transient callback, so the
    /// recovery is caused by the retry and by nothing else.
    ///
    /// kills: making [`read_proven_finalize_artifact`] a plain `std::fs::read`.
    #[cfg(windows)]
    #[test]
    fn a_held_artifact_is_read_once_the_holder_lets_go() {
        use std::os::windows::fs::OpenOptionsExt;

        let root = std::env::temp_dir().join(format!(
            "ls_fs_sharing_retry_{}_{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("artifact.bin");
        std::fs::write(&path, b"proven-bytes").unwrap();

        let holder = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        // The premise, asserted rather than assumed: while that handle is
        // alive an ordinary read really is refused with 32.
        assert_eq!(
            std::fs::read(&path).unwrap_err().raw_os_error(),
            Some(32),
            "the exclusive holder must actually exclude an ordinary read"
        );

        let mut holder = Some(holder);
        let _armed = transient_hook::install(move || {
            drop(holder.take());
        });
        let bytes = read_proven_finalize_artifact(&path).unwrap();
        assert_eq!(bytes, b"proven-bytes");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// kills: returning a synthesised error instead of the last real one, which
    /// would hide `os error 32` from the operator when the wait is exhausted.
    #[cfg(windows)]
    #[test]
    fn an_exhausted_wait_still_reports_the_real_error() {
        let mut clock = FakeClock {
            now: Duration::ZERO,
            slept: Vec::new(),
        };
        let outcome = retry_core::<()>(
            Duration::from_millis(3),
            "test",
            || Err(io::Error::from_raw_os_error(32)),
            &mut clock,
            &mut Duration::ZERO,
        );
        let error = outcome.unwrap_err();
        assert_eq!(error.raw_os_error(), Some(32));
    }

    /// An OS that sleeps LONGER than asked. `Sleep` on Windows rounds up to the
    /// timer resolution, so a requested 1 ms routinely costs 15 ms.
    #[cfg(windows)]
    struct OversleepingClock {
        now: Duration,
        overshoot: Duration,
    }

    #[cfg(windows)]
    impl Clock for OversleepingClock {
        fn elapsed(&self) -> Duration {
            self.now
        }
        fn sleep(&mut self, duration: Duration) -> Duration {
            let actual = duration + self.overshoot;
            self.now += actual;
            actual
        }
    }

    /// kills: charging the REQUESTED nap instead of the time the sleep actually
    /// took. Under-charging is not a rounding error -- it is what stops the
    /// whole-thread allowance from being a wall-clock bound at all, and it is
    /// how the battery caught this the first time. `FakeClock` cannot see the
    /// defect because it advances by exactly the requested duration, so the
    /// proof needs a clock that overshoots.
    #[cfg(windows)]
    #[test]
    fn an_oversleep_is_charged_in_full() {
        let mut clock = OversleepingClock {
            now: Duration::ZERO,
            overshoot: Duration::from_millis(15),
        };
        let mut slept = Duration::ZERO;
        let _ = retry_core::<()>(
            Duration::from_millis(100),
            "test",
            || Err(io::Error::from_raw_os_error(32)),
            &mut clock,
            &mut slept,
        );
        // Every sleep overshot by 15 ms, and the charge has to include the
        // overshoot: charging the requested naps alone would report far less
        // than the clock actually advanced.
        assert_eq!(
            slept, clock.now,
            "the charge must be the time the clock actually advanced across the sleeps"
        );
        assert!(
            slept >= Duration::from_millis(100),
            "an oversleeping OS still spends the whole allowance: {slept:?}"
        );
    }

    /// kills: charging the allowance for the OPERATION's own time as well as
    /// the sleeps, which would let a slow disk consume a budget that exists for
    /// scanners. (It does not pin the sleep measurement itself -- that is
    /// `an_oversleep_is_charged_in_full` -- and it cannot see the scheduling
    /// intervals `Clock::sleep` documents, because a fake clock has none.)
    #[cfg(windows)]
    #[test]
    fn the_operations_own_time_is_never_charged() {
        let mut clock = FakeClock {
            now: Duration::from_millis(500),
            slept: Vec::new(),
        };
        let mut slept = Duration::ZERO;
        let mut attempts = 0_u32;
        let _ = retry_core::<()>(
            Duration::from_millis(520),
            "test",
            || {
                attempts += 1;
                Err(io::Error::from_raw_os_error(32))
            },
            &mut clock,
            &mut slept,
        );
        // The clock started at 500 ms of operation time and the loop only ever
        // committed to 20 ms of sleep; the charge is the 20, not the 520.
        assert_eq!(slept, Duration::from_millis(20));
        assert_eq!(clock.slept.iter().sum::<Duration>(), slept);
    }

    /// The real-handle half of the whole-thread bound: one call against a
    /// genuinely held file really does spend the allowance.
    ///
    /// Only a LOWER bound on the first call is asserted, because a lower bound
    /// on elapsed time cannot be met by a plain `std::fs::read` and cannot be
    /// crossed by a slow machine. The sharing arithmetic is proved separately
    /// and deterministically in `the_allowance_is_shared_across_calls_exactly`.
    ///
    /// kills: deleting the retry from `read_proven_finalize_artifact`, which
    /// would return on the first refusal in microseconds.
    #[cfg(windows)]
    #[test]
    fn the_wait_allowance_is_spent_once_for_the_whole_thread() {
        use std::os::windows::fs::OpenOptionsExt;

        reset_wait_allowance_for_test();
        let root = std::env::temp_dir().join(format!(
            "ls_fs_allowance_{}_{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("held.bin");
        std::fs::write(&path, b"held forever").unwrap();
        // Held across every call below, so each one exhausts whatever allowance
        // it is given. Dropped explicitly before the cleanup at the end: a
        // `share_mode(0)` handle refuses the removal too, and a cleanup that
        // silently fails leaves a directory behind on every battery run.
        let holder = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();

        // ONE call spends the allowance...
        let first_started = Instant::now();
        assert_eq!(
            read_proven_finalize_artifact(&path)
                .unwrap_err()
                .raw_os_error(),
            Some(32)
        );
        let first = first_started.elapsed();
        assert!(
            first >= FINALIZE_RETRY_BUDGET,
            "the first call must spend the allowance, spent {first:?}"
        );

        // What the LATER calls do is asserted deterministically in
        // `the_allowance_is_shared_across_calls_exactly` rather than by timing
        // real system calls here. A wall-clock upper bound on three refused
        // opens is a threshold a loaded machine can cross for reasons that have
        // nothing to do with the policy, and it would not have proved the
        // property anyway.
        assert_eq!(take_proven_read_count(), 1);
        drop(holder);
        reset_wait_allowance_for_test();
        std::fs::remove_dir_all(&root).expect("the released tree is removable");
    }

    /// The allowance arithmetic, proved exactly and without spending any real
    /// time: the FIRST call consumes the whole budget, and the second is given
    /// nothing, so it makes one attempt and never sleeps.
    ///
    /// This is what makes `8 + 4R` retried reads bounded by one budget rather
    /// than by `R`.
    ///
    /// kills: making the budget per-call again (the second call would sleep the
    /// full schedule); charging nothing (the second call would get a full
    /// budget); charging the wrong quantity (the first assertion is an exact
    /// equality against the budget, not a range).
    #[cfg(windows)]
    #[test]
    fn the_allowance_is_shared_across_calls_exactly() {
        reset_wait_allowance_for_test();
        assert_eq!(remaining_wait_allowance(), FINALIZE_RETRY_BUDGET);

        let mut first = FakeClock {
            now: Duration::ZERO,
            slept: Vec::new(),
        };
        let _ = run_within_allowance_on::<()>(
            "test",
            || Err(io::Error::from_raw_os_error(32)),
            &mut first,
        );
        assert_eq!(
            first.slept.iter().sum::<Duration>(),
            FINALIZE_RETRY_BUDGET,
            "the first call must sleep exactly the whole allowance"
        );
        assert_eq!(
            remaining_wait_allowance(),
            Duration::ZERO,
            "and the whole allowance must be charged"
        );

        let mut attempts = 0_u32;
        let mut second = FakeClock {
            now: Duration::ZERO,
            slept: Vec::new(),
        };
        let _ = run_within_allowance_on::<()>(
            "test",
            || {
                attempts += 1;
                Err(io::Error::from_raw_os_error(32))
            },
            &mut second,
        );
        assert_eq!(attempts, 1, "a spent allowance buys exactly one attempt");
        assert!(
            second.slept.is_empty(),
            "and no sleep at all: {:?}",
            second.slept
        );
        reset_wait_allowance_for_test();
    }
}
