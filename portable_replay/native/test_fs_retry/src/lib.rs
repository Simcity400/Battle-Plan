//! Bounded retry around the Windows sharing violation, for TEST code.
//!
//! # The defect this exists for
//!
//! On Windows, a real-time antimalware scanner (MsMpEng.exe for the in-box
//! one) opens a file the moment it is created or last written, and it opens it
//! with a share mode that excludes other openers. While that handle is alive,
//! any other `CreateFile` asking for an incompatible access fails with
//! `ERROR_SHARING_VIOLATION` (32):
//!
//! ```text
//! The process cannot access the file because it is being used by another
//! process. (os error 32)
//! ```
//!
//! The window is milliseconds and nothing in the calling program is wrong, so
//! the failure shows up as a test that passes alone and fails once in a while
//! in a full run. THREE such failures were recorded across 2026-08-15/16, in
//! three different crates, each on a test that wrote a container and opened it
//! immediately afterwards. In a panic Rust renders it as `Os { code: 32, kind:
//! Uncategorized, ... }`, which is what to grep a battery log for.
//!
//! A scanner exclusion for the build tree fixes it too, but that is a machine
//! security setting rather than a property of this repository, so it cannot be
//! what the tests depend on.
//!
//! # Test-only, and named that way
//!
//! Nothing shipped calls this. It is reached only through `[dev-dependencies]`,
//! and the package name says so, because the retry budget here is chosen for a
//! test harness — where a two-second stall is invisible — and NOT for the
//! recorder, where a stall on the wrong path is the thing the product is built
//! to avoid. The production write-then-read sites are a separate question with
//! a separate answer; do not reach for this crate to settle them.
//!
//! # What this retries, and what it deliberately does not
//!
//! It retries **one** error class — the transient sharing violation — for a
//! bounded time, and passes everything else through on the first attempt.
//! That distinction is the whole design:
//!
//! * `NotFound` stays instant and loud. Tests assert on absent files.
//! * `PermissionDenied` (`ERROR_ACCESS_DENIED`, 5) is a real refusal, not a
//!   scanner. A read-only file, an ACL, or a delete-pending file will never
//!   become openable by waiting, and several tests assert exactly that.
//! * `ERROR_LOCK_VIOLATION` (33) is NOT retried. It describes a byte-range
//!   lock, not an incompatible share mode, and no recorded failure here was
//!   ever a 33. It gets added if and when one is.
//! * A sharing violation that does not clear inside the budget still fails,
//!   with the last error returned unchanged — so an assertion on the error
//!   text or `raw_os_error` still sees the real thing. A handle our own code
//!   leaked keeps failing; it just fails [`RETRY_BUDGET`] later. That is the
//!   cost of this crate, and it is why it is applied only at edges where a
//!   file was demonstrably just written by this process.
//!
//! ## What the budget promises, exactly
//!
//! No retry is COMMITTED once a check observes [`RETRY_BUDGET`] exhausted. The
//! loop checks twice per iteration — before committing to a backoff, and again
//! after it — and gives up at the first check that finds the budget spent.
//!
//! That is deliberately weaker than "no attempt begins after the deadline",
//! which nothing in userland can promise: the thread can be descheduled between
//! the last check and the `CreateFile` that follows it. It is weaker still than
//! a wall-clock ceiling on the call, because the attempt already in flight runs
//! to completion and the operating system may oversleep a backoff. The
//! guarantee is about what this code DECIDES, not about when the machine gets
//! round to it.
//!
//! # Off Windows
//!
//! On any non-Windows target the retry loop is not compiled at all: every
//! operation runs exactly once. `raw_os_error() == Some(32)` means `ENOTTY` on
//! Linux, which has nothing to do with sharing, and a retry keyed on the raw
//! number would be a defect there rather than a fix. The wrappers are still not
//! all pure passthroughs — [`with_read_guard`] opens a real extra handle on
//! every target, because that is its purpose — but nothing here loops.
//!
//! # Why the reader retries, rather than the writer doing something different
//!
//! Two write-side "fixes" were considered and rejected:
//!
//! * `sync_all` before closing makes the bytes durable. It does not make the
//!   scanner finish, and Windows offers no guarantee that it shortens the
//!   window at all. Adding it to fixture writes would be cargo cult plus disk
//!   traffic.
//! * Writing under a temporary name and renaming into place does not avoid the
//!   scanner — it relocates the exposure onto the rename, which returns the
//!   same error 32 when the scanner holds the source. Renaming is worth doing
//!   for atomic publication; it is not a mitigation for this.
//!
//! A third was tried and REMOVED during review: writing and then probing with a
//! retried open that is immediately dropped. It leaves the file unheld between
//! the probe and the real read, so it narrows the window instead of closing it.
//! Where the consumer is somebody else's opener, [`with_read_guard`] holds a
//! handle ACROSS the call instead, and that does close it.
//!
//! The operation that fails is the read, so the read is what retries.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::time::Duration;
#[cfg(windows)]
use std::time::Instant;

/// How long one call may go on committing to new attempts; see the crate docs
/// for exactly what that does and does not promise.
///
/// Two seconds is far above the observed scanner window (single-digit
/// milliseconds) and far below any test timeout, so a call that runs out of
/// this budget is reporting a real, persistent holder rather than a scanner.
///
/// It is a provisional TEST ceiling, not a measured product policy. No
/// production path should adopt this number without its own evidence.
pub const RETRY_BUDGET: Duration = Duration::from_millis(2000);

/// First backoff sleep. Doubles up to [`MAX_BACKOFF`].
#[cfg(windows)]
const FIRST_BACKOFF: Duration = Duration::from_millis(1);

/// Backoff ceiling. Small on purpose: the window being waited out is
/// milliseconds, so a large sleep would turn a 3 ms stall into a 64 ms one.
#[cfg(windows)]
const MAX_BACKOFF: Duration = Duration::from_millis(16);

/// `ERROR_SHARING_VIOLATION` — another process holds the file with a share mode
/// that excludes this open. The only code this crate waits on.
#[cfg(windows)]
const ERROR_SHARING_VIOLATION: i32 = 32;

/// Is this the transient class, i.e. the one worth waiting out?
///
/// False for every error on non-Windows targets: the retry has no meaning
/// there, and the raw number collides with an unrelated `errno`.
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
///
/// Only the loop uses this. It exists because the alternative — asserting a
/// timing policy against the real clock — produces a test that is either
/// vacuous (a wide tolerance passes an extra backoff) or flaky (a narrow one
/// fails under load). With a fake clock the assertions are exact.
#[cfg(windows)]
trait Clock {
    /// Time elapsed since the call began.
    fn elapsed(&self) -> Duration;
    /// Wait; a fake simply advances.
    fn sleep(&mut self, duration: Duration);
}

#[cfg(windows)]
struct RealClock(Instant);

#[cfg(windows)]
impl Clock for RealClock {
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// A test-only observer of transient failures, reached from inside the loop.
///
/// It exists so the load-bearing proofs can call the PUBLIC `open` and
/// `remove_file` — not a parallel copy of them — and still release the handle
/// that caused the sharing violation at a deterministic moment. Deleting the
/// retry from a public entry point therefore breaks those proofs, which is the
/// property a private mirror could not give.
///
/// Compiled only into this crate's own test build. There is no way to install a
/// hook from outside, and no `cfg`-off cost: the shipping build gets an empty
/// inline function.
#[cfg(all(windows, test))]
mod transient_hook {
    use std::cell::RefCell;

    thread_local! {
        static HOOK: RefCell<Option<Box<dyn FnMut()>>> = const { RefCell::new(None) };
    }

    /// Installs `hook` until the returned value is dropped, so a panicking test
    /// cannot leave one armed for the next test on this thread.
    pub(super) struct Installed;

    impl Drop for Installed {
        fn drop(&mut self) {
            HOOK.with(|hook| *hook.borrow_mut() = None);
        }
    }

    pub(super) fn install(hook: impl FnMut() + 'static) -> Installed {
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

/// The retry loop.
///
/// `on_transient` runs after each attempt that failed with the transient class
/// and BEFORE the backoff. It is how this crate's own proof drives a real
/// sharing violation deterministically: the test releases the handle that
/// caused the failure from inside this callback, so the recovery is caused by
/// the retry rather than by a sleep racing a thread. In every shipping call it
/// is a no-op closure.
///
/// The give-up error is the last one the operation produced, untouched.
#[cfg(windows)]
fn retry_core<T>(
    budget: Duration,
    operation: &str,
    mut op: impl FnMut() -> io::Result<T>,
    mut on_transient: impl FnMut(),
    clock: &mut dyn Clock,
) -> io::Result<T> {
    let mut backoff = FIRST_BACKOFF;
    let mut attempts = 0_u32;
    loop {
        attempts += 1;
        match op() {
            Ok(value) => {
                if attempts > 1 {
                    eprintln!(
                        "portable_test_fs_retry: {operation} recovered from a sharing violation \
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
                on_transient();
                fire_transient_hook();
                // The budget bounds what this loop COMMITS to, so it is
                // checked TWICE: once before committing to a sleep, and again
                // after it. Without the second check the truncated final sleep
                // would land exactly on the budget and the loop would still
                // commit to one more attempt past it.
                let elapsed = clock.elapsed();
                if elapsed >= budget {
                    return Err(error);
                }
                let remaining = budget - elapsed;
                clock.sleep(backoff.min(remaining));
                backoff = (backoff * 2).min(MAX_BACKOFF);
                if clock.elapsed() >= budget {
                    return Err(error);
                }
            }
        }
    }
}

/// The shipping entry point: [`RETRY_BUDGET`], the real clock.
///
/// # Visibility of a recovered attempt
///
/// A recovery prints ONE line to stderr — not one per attempt — naming the
/// operation, the attempt count and the elapsed time. Be clear about what that
/// buys: the battery's nextest profiles set `success-output = "never"`, so on a
/// passing test nobody sees it. It is there for a failing or manually re-run
/// test, where "this passed but it took four attempts and 40 ms" is the
/// difference between a scanner and a handle this repository is leaking.
#[cfg(windows)]
fn run<T>(
    operation: &str,
    op: impl FnMut() -> io::Result<T>,
    on_transient: impl FnMut(),
) -> io::Result<T> {
    retry_core(
        RETRY_BUDGET,
        operation,
        op,
        on_transient,
        &mut RealClock(Instant::now()),
    )
}

/// One attempt, on a target where the sharing violation does not exist.
#[cfg(not(windows))]
fn run<T>(
    _operation: &str,
    mut op: impl FnMut() -> io::Result<T>,
    _on_transient: impl FnMut(),
) -> io::Result<T> {
    op()
}

/// [`File::open`], waiting out a scanner.
pub fn open(path: impl AsRef<Path>) -> io::Result<File> {
    let path = path.as_ref();
    run("open", || File::open(path), || {})
}

/// [`OpenOptions::open`], waiting out a scanner.
///
/// Takes the options by reference and re-runs `open` on the SAME options, which
/// is what makes it usable for the append-mode reopens several callers do
/// straight after a write.
pub fn open_with(options: &OpenOptions, path: impl AsRef<Path>) -> io::Result<File> {
    let path = path.as_ref();
    run("open_with", || options.open(path), || {})
}

/// [`std::fs::read`], waiting out a scanner.
pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let path = path.as_ref();
    run("read", || std::fs::read(path), || {})
}

/// [`std::fs::read_to_string`], waiting out a scanner.
pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    let path = path.as_ref();
    run("read_to_string", || std::fs::read_to_string(path), || {})
}

/// [`std::fs::remove_file`], waiting out a scanner.
///
/// Teardown is exposed to the same window as setup, and more sharply: the file
/// was written moments ago, which is exactly when the scanner holds it. A
/// removal that fails at the end of a test fails the test just as hard as an
/// open that fails at the start.
///
/// An ABSENT file is still an error here, on the first attempt, exactly as
/// [`std::fs::remove_file`] reports it — a teardown that quietly tolerates a
/// file that was never created is a teardown that has stopped checking.
pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    run("remove_file", || std::fs::remove_file(path), || {})
}

/// [`std::fs::remove_dir_all`], waiting out a scanner.
///
/// The whole call is retried rather than each entry, because `remove_dir_all`
/// offers no way in. A partial removal followed by a retry is harmless for a
/// directory the test owns: the entries already gone stay gone and the next
/// pass removes the rest. Do not point it at a tree something else is also
/// deleting.
pub fn remove_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    run("remove_dir_all", || std::fs::remove_dir_all(path), || {})
}

/// Hold a read handle on `path` for the duration of `op`, retrying the
/// ACQUISITION of that handle until the scanner lets go.
///
/// This is the entry point for a caller whose opener is NOT ours and whose
/// error type does not carry the [`io::Error`]. The container openers in this
/// repository stringify it, so there is nothing structural to classify on, and
/// matching the rendered text across a crate boundary is exactly the coupling
/// those error types exist to prevent.
///
/// ```no_run
/// # fn open_recording(_: &std::path::Path) -> Result<(), String> { Ok(()) }
/// # let path = std::path::Path::new("x");
/// let source = portable_test_fs_retry::with_read_guard(path, || open_recording(path))
///     .expect("acquire the read guard")
///     .expect("open the recording");
/// ```
///
/// # Why it HOLDS the handle instead of opening and dropping one
///
/// Opening a probe handle and dropping it before the real call reintroduces the
/// race it is meant to remove: nothing stops a scanner taking the file again in
/// the gap. Keeping the handle alive across `op` closes that: while this
/// read handle is open, a `CreateFile` asking for the scanner's exclusive share
/// mode is refused, so it cannot get back in front of the caller.
///
/// Windows share-mode compatibility is a relation between the two opens, not a
/// property of one handle, so the claim is specific rather than general: this
/// guard is a read handle with the ordinary sharing mask, which conflicts with
/// a later `share_mode(0)` open and does NOT conflict with an ordinary reader.
/// A caller that opens for WRITING under this guard is a different question and
/// is not what any migrated site does.
///
/// The outer `Result` is the guard acquisition; the inner value is whatever
/// `op` returned. Everything after the guard is the caller's own outcome,
/// untouched.
pub fn with_read_guard<T>(path: impl AsRef<Path>, op: impl FnOnce() -> T) -> io::Result<T> {
    let guard = open(path)?;
    let outcome = op();
    drop(guard);
    Ok(outcome)
}

/// [`with_read_guard`] over several paths at once.
///
/// For a consumer that is handed a DIRECTORY and opens a known set of files
/// inside it — the shape every run-directory verifier in this repository has.
/// Every path is guarded before `op` runs, and all of them stay open until it
/// returns.
pub fn with_read_guards<T>(paths: &[&Path], op: impl FnOnce() -> T) -> io::Result<T> {
    let mut guards = Vec::with_capacity(paths.len());
    for path in paths {
        guards.push(open(path)?);
    }
    let outcome = op();
    drop(guards);
    Ok(outcome)
}

/// Hold a read handle on EVERY FILE under `root` for the duration of `op`.
///
/// This is the form to reach for when a test writes a fixture TREE and hands
/// the DIRECTORY to a foreign reader — a store loader, a mod resolver, a
/// package builder — which then opens files inside it that the caller never
/// names.
///
/// It takes either a directory or a single file, so a caller that does not know
/// which shape it has — "the recording", which is a directory store in some
/// tests and one container file in others — can use it unconditionally.
///
/// # Why a directory path must never go to [`with_read_guard`]
///
/// On Windows `File::open` on a directory fails outright with
/// `PermissionDenied`, so `with_read_guard(store_root, ...)` does not quietly
/// guard nothing — it makes the acquisition FAIL, and a caller that unwraps it
/// panics. Either way it does not hold a single file under that directory. That
/// mistake was made inside this very lane and would have broken ten tests, so
/// this function exists rather than a note telling people to be careful.
///
/// Files are enumerated ONCE, before `op` runs. A file the operation itself
/// creates is therefore not covered, which is correct: this guards the fixture
/// that was written before the call, not the call's own output.
///
/// Acquisition failures are REPORTED, not swallowed. An earlier version
/// ignored them, which meant a scanner refusing the directory listing produced
/// zero guards and a caller that believed it held the tree — silence in exactly
/// the situation the guards exist for. Everything that can be waited out is
/// waited out first; what comes back as `Err` is a real refusal.
pub fn with_tree_read_guard<T>(root: impl AsRef<Path>, op: impl FnOnce() -> T) -> io::Result<T> {
    let guards = tree_read_guards(root)?;
    let outcome = op();
    drop(guards);
    Ok(outcome)
}

/// The guards [`with_tree_read_guard`] holds, for a caller that needs to keep
/// them across something a closure cannot express — a `Command` spawn, or the
/// rest of a test body.
///
/// Bind it to a named local (`let _guards = ...`), never to `_`: binding to a
/// bare underscore drops the handles immediately and guards nothing.
#[must_use = "the guards do nothing unless they are held"]
pub fn tree_read_guards(root: impl AsRef<Path>) -> io::Result<Vec<File>> {
    fn walk(directory: &Path, guards: &mut Vec<File>) -> io::Result<()> {
        // The listing itself is retried: a scanner can refuse a directory read
        // for the same reason it refuses a file open, and a swallowed refusal
        // here would silently return zero guards for a whole subtree.
        let entries = run("read_dir", || std::fs::read_dir(directory), || {})?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            match entry.file_type()? {
                kind if kind.is_dir() => walk(&path, guards)?,
                kind if kind.is_file() => guards.push(open(&path)?),
                // A symlink, device or anything else is not a file this crate
                // can hold, and is not an error either: skipped, deliberately.
                _ => {}
            }
        }
        Ok(())
    }

    let root = root.as_ref();
    let mut guards = Vec::new();
    // A FILE is guarded as itself. Accepting both shapes is deliberate: the
    // callers that need this are handing a foreign reader "the recording",
    // which is a directory store in some tests and a single container file in
    // others, and a helper that silently guarded nothing for one of them would
    // be the very defect this exists to prevent.
    //
    // The classification is a RETRIED metadata read, not `Path::is_file()`.
    // `is_file()` collapses every error to `false`, so a scanner refusing the
    // metadata call would send a plain file down the directory branch, where
    // `read_dir` fails and the caller gets zero guards while believing it holds
    // the tree — silence in exactly the situation the guards exist for.
    let metadata = run("metadata", || std::fs::metadata(root), || {})?;
    if metadata.is_file() {
        guards.push(open(root)?);
        return Ok(guards);
    }
    walk(root, &mut guards)?;
    Ok(guards)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A scratch directory of this crate's own, named from the process id and a
    /// monotonic counter so two tests in the same binary — which nextest may
    /// run in parallel — never share one.
    fn scratch(label: &str) -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "portable_test_fs_retry_{label}_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create scratch directory");
        dir
    }

    #[cfg(windows)]
    fn sharing_violation() -> io::Error {
        io::Error::from_raw_os_error(ERROR_SHARING_VIOLATION)
    }

    /// A clock that never advances on its own, so the policy's arithmetic is
    /// observed rather than raced against.
    #[cfg(windows)]
    #[derive(Default)]
    struct FakeClock {
        now: Duration,
        slept: RefCell<Vec<Duration>>,
    }

    #[cfg(windows)]
    impl Clock for FakeClock {
        fn elapsed(&self) -> Duration {
            self.now
        }
        fn sleep(&mut self, duration: Duration) {
            self.slept.borrow_mut().push(duration);
            self.now += duration;
        }
    }

    /// The loop retries the transient class until it clears, and returns the
    /// value from the attempt that cleared.
    ///
    /// Production change that would make this fail: retrying zero times, or
    /// classifying the sharing violation as terminal.
    #[cfg(windows)]
    #[test]
    fn a_sharing_violation_is_retried_until_it_clears() {
        let attempts = Cell::new(0);
        let mut clock = FakeClock::default();
        let value = retry_core(
            RETRY_BUDGET,
            "test",
            || {
                attempts.set(attempts.get() + 1);
                if attempts.get() < 4 {
                    Err(sharing_violation())
                } else {
                    Ok(attempts.get())
                }
            },
            || {},
            &mut clock,
        )
        .expect("the violation clears on the fourth attempt");
        assert_eq!(value, 4);
        assert_eq!(attempts.get(), 4);
    }

    /// Everything that is not the transient class costs exactly one attempt,
    /// and comes back unchanged.
    ///
    /// This is the half that keeps the retry from becoming a blanket "try
    /// again": a missing file and a refused permission are ANSWERS, and many
    /// migrated tests assert on them. Retried, every such assertion would take
    /// `RETRY_BUDGET` to arrive.
    ///
    /// `33` is in the table deliberately: `ERROR_LOCK_VIOLATION` is a
    /// byte-range lock, no recorded failure was one, and it stays terminal
    /// until one is.
    ///
    /// Production change that would make this fail: classifying on
    /// `ErrorKind::Uncategorized`, on "any os error", on the error's message
    /// text, or widening the code table without evidence.
    #[cfg(windows)]
    #[test]
    fn every_other_error_fails_on_the_first_attempt() {
        for error in [
            io::Error::new(io::ErrorKind::NotFound, "absent"),
            io::Error::new(io::ErrorKind::PermissionDenied, "refused"),
            io::Error::from_raw_os_error(5),   // ERROR_ACCESS_DENIED
            io::Error::from_raw_os_error(2),   // ERROR_FILE_NOT_FOUND
            io::Error::from_raw_os_error(33),  // ERROR_LOCK_VIOLATION
            io::Error::from_raw_os_error(183), // ERROR_ALREADY_EXISTS
        ] {
            let raw = error.raw_os_error();
            let kind = error.kind();
            let attempts = Cell::new(0);
            let cell = Cell::new(Some(error));
            let mut clock = FakeClock::default();
            let returned = retry_core::<()>(
                RETRY_BUDGET,
                "test",
                || {
                    attempts.set(attempts.get() + 1);
                    Err(cell
                        .take()
                        .unwrap_or_else(|| io::Error::new(io::ErrorKind::Other, "reattempted")))
                },
                || panic!("the observer must only fire for the transient class"),
                &mut clock,
            )
            .expect_err("this class must not succeed");
            assert_eq!(attempts.get(), 1, "{kind:?}/{raw:?} was retried");
            assert_eq!(returned.raw_os_error(), raw, "the error was rewritten");
            assert_eq!(returned.kind(), kind, "the error was rewritten");
            assert!(clock.slept.borrow().is_empty(), "{kind:?} slept");
        }
    }

    /// The exact backoff schedule, and the exact point the loop stops starting
    /// new attempts.
    ///
    /// With a fake clock there is no tolerance to argue about: the sleeps are
    /// 1, 2, 4, 8, 16, 16, ... capped at `MAX_BACKOFF`, the last one is
    /// TRUNCATED so it lands exactly on the budget rather than past it, and no
    /// attempt is started afterwards. A real-clock version of this assertion
    /// would either pass an extra 16 ms backoff or fail under load.
    ///
    /// Production change that would make this fail: an unbounded loop, a
    /// constant backoff, an uncapped one, sleeping past the deadline, or
    /// checking the deadline after the sleep instead of before it.
    #[cfg(windows)]
    #[test]
    fn the_backoff_schedule_and_the_deadline_are_exactly_what_is_documented() {
        let budget = Duration::from_millis(50);
        let attempts = Cell::new(0);
        let mut clock = FakeClock::default();
        let error = retry_core::<()>(
            budget,
            "test",
            || {
                attempts.set(attempts.get() + 1);
                Err(sharing_violation())
            },
            || {},
            &mut clock,
        )
        .expect_err("a violation that never clears must still fail");

        assert_eq!(
            error.raw_os_error(),
            Some(ERROR_SHARING_VIOLATION),
            "the give-up error was rewritten"
        );
        let slept = clock.slept.borrow().clone();
        assert_eq!(
            slept,
            [1, 2, 4, 8, 16, 16, 3].map(Duration::from_millis).to_vec(),
            "the schedule must double to the cap and truncate onto the budget"
        );
        assert_eq!(
            slept.iter().sum::<Duration>(),
            budget,
            "the loop must stop exactly at the budget, never past it"
        );
        // ONE attempt per sleep and no more: the truncated final sleep lands
        // exactly on the budget, and the post-sleep check refuses to commit to
        // another attempt.
        assert_eq!(
            attempts.get(),
            slept.len(),
            "an attempt was committed to at or past the deadline"
        );
    }

    /// A budget already spent when the first attempt fails buys no retry at
    /// all, and still returns the real error.
    ///
    /// Production change that would make this fail: treating the budget as "at
    /// least one retry, always", which would make a zero budget unbounded by
    /// one attempt.
    #[cfg(windows)]
    #[test]
    fn a_spent_budget_starts_no_further_attempt() {
        let attempts = Cell::new(0);
        let mut clock = FakeClock::default();
        let error = retry_core::<()>(
            Duration::ZERO,
            "test",
            || {
                attempts.set(attempts.get() + 1);
                Err(sharing_violation())
            },
            || {},
            &mut clock,
        )
        .expect_err("a spent budget must fail");
        assert_eq!(error.raw_os_error(), Some(ERROR_SHARING_VIOLATION));
        assert_eq!(attempts.get(), 1);
        assert!(clock.slept.borrow().is_empty());
    }

    /// THE LOAD-BEARING PROOF for `open`, against a REAL Windows sharing
    /// violation and with no timing in it.
    ///
    /// A non-shared handle on the file is exactly what the scanner does to us.
    /// It is held across the first attempt — asserted, by requiring a plain
    /// open to fail with error 32 — and released from inside the loop's own
    /// transient callback, so the recovery on the second attempt is caused by
    /// the retry and by nothing else. No thread races a sleep; descheduling
    /// cannot change the outcome.
    ///
    /// The function under test is the PUBLIC `open` itself. The release is
    /// delivered through a test-only thread-local hook the loop fires, so there
    /// is no private mirror that could keep passing after the retry was deleted
    /// from the shipping entry point.
    ///
    /// Production change that would make this fail: making `open` a plain
    /// `File::open` — the first attempt's error 32 would be returned.
    #[cfg(windows)]
    #[test]
    fn open_recovers_from_a_real_sharing_violation_it_actually_observed() {
        use std::os::windows::fs::OpenOptionsExt as _;

        let dir = scratch("real_open");
        let path = dir.join("held.bin");
        std::fs::write(&path, b"contents").expect("write the file");

        // share_mode(0): no other process may read, write, or delete.
        let guard = Rc::new(RefCell::new(Some(
            OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&path)
                .expect("take the non-shared handle"),
        )));
        assert_eq!(
            File::open(&path)
                .expect_err("a plain open must be refused while held")
                .raw_os_error(),
            Some(ERROR_SHARING_VIOLATION),
            "the reproduction no longer reproduces"
        );

        // The hook releases the handle from inside the loop, so the recovery on
        // the second attempt is caused by the retry and by nothing else. The
        // call under test is the PUBLIC `open`: deleting the retry from it is
        // what this proof is here to catch.
        let releases = Rc::new(Cell::new(0));
        let counter = Rc::clone(&releases);
        let _hook = transient_hook::install(move || {
            counter.set(counter.get() + 1);
            drop(guard.borrow_mut().take());
        });
        let mut opened = open(&path).expect("the retry must recover once the handle is released");
        assert_eq!(
            releases.get(),
            1,
            "the loop must have observed exactly one real sharing violation"
        );

        let mut read_back = Vec::new();
        io::Read::read_to_end(&mut opened, &mut read_back).expect("read the reopened file");
        assert_eq!(read_back, b"contents");
        drop(opened);
        let _ = remove_dir_all(&dir);
    }

    /// The same proof for teardown: a non-shared handle also refuses
    /// `remove_file`, and the retry is what gets past it.
    ///
    /// Production change that would make this fail: dropping `remove_file` from
    /// this crate and leaving teardown on `std::fs`.
    #[cfg(windows)]
    #[test]
    fn removal_recovers_from_a_real_sharing_violation_it_actually_observed() {
        use std::os::windows::fs::OpenOptionsExt as _;

        let dir = scratch("real_remove");
        let path = dir.join("held.bin");
        std::fs::write(&path, b"contents").expect("write the file");

        let guard = Rc::new(RefCell::new(Some(
            OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&path)
                .expect("take the non-shared handle"),
        )));
        assert_eq!(
            std::fs::remove_file(&path)
                .expect_err("a plain removal must be refused while held")
                .raw_os_error(),
            Some(ERROR_SHARING_VIOLATION),
            "the reproduction no longer reproduces"
        );

        let releases = Rc::new(Cell::new(0));
        let counter = Rc::clone(&releases);
        let _hook = transient_hook::install(move || {
            counter.set(counter.get() + 1);
            drop(guard.borrow_mut().take());
        });
        remove_file(&path).expect("the retry must recover once the handle is released");
        assert_eq!(releases.get(), 1);
        assert!(!path.exists());
        let _ = remove_dir_all(&dir);
    }

    /// The retained guard is what makes the foreign-opener form work: while it
    /// is alive an exclusive open — the scanner's shape — is refused, and an
    /// ordinary read still succeeds, so nothing can get in front of the
    /// caller's own open and the guard does not block it either.
    ///
    /// Production change that would make this fail: turning `with_read_guard`
    /// into an open-and-drop probe, which is the version that still races —
    /// the exclusive open inside the closure would then succeed.
    #[cfg(windows)]
    #[test]
    fn the_retained_guard_locks_a_scanner_out_for_the_length_of_the_call() {
        use std::os::windows::fs::OpenOptionsExt as _;

        let dir = scratch("guard");
        let path = dir.join("guarded.bin");
        std::fs::write(&path, b"guarded").expect("write the file");

        let observed = with_read_guard(&path, || {
            let exclusive = OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&path)
                .expect_err("an exclusive open must be refused while the guard is held");
            assert_eq!(exclusive.raw_os_error(), Some(ERROR_SHARING_VIOLATION));
            std::fs::read(&path).expect("the guard must not refuse an ordinary reader")
        })
        .expect("acquire the guard");
        assert_eq!(observed, b"guarded");

        // Released afterwards: the exclusive open now succeeds, which proves
        // the refusal above came from the guard and not from something else.
        OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .expect("the guard must be released when the call returns");
        let _ = remove_dir_all(&dir);
    }

    /// `with_read_guards` holds EVERY path at once, not just the first.
    ///
    /// Production change that would make this fail: guarding only one of them,
    /// or dropping each guard as the next is taken.
    #[cfg(windows)]
    #[test]
    fn the_multi_path_guard_holds_all_of_them_at_once() {
        use std::os::windows::fs::OpenOptionsExt as _;

        let dir = scratch("guards");
        let paths = ["a.bin", "b.bin", "c.bin"].map(|name| dir.join(name));
        for path in &paths {
            std::fs::write(path, b"x").expect("write");
        }
        let refs = paths.iter().map(|p| p.as_path()).collect::<Vec<_>>();
        with_read_guards(&refs, || {
            for path in &paths {
                assert_eq!(
                    OpenOptions::new()
                        .read(true)
                        .share_mode(0)
                        .open(path)
                        .expect_err("every guarded path must refuse an exclusive open")
                        .raw_os_error(),
                    Some(ERROR_SHARING_VIOLATION),
                    "{} was not held",
                    path.display()
                );
            }
        })
        .expect("acquire the guards");
        let _ = remove_dir_all(&dir);
    }

    /// An absent path is still absent, through EVERY entry point, immediately.
    ///
    /// Production change that would make this fail: retrying on `NotFound`,
    /// which would turn every negative assertion in the migrated tests into a
    /// two-second wait.
    #[test]
    fn an_absent_path_is_refused_immediately_through_every_entry_point() {
        let dir = scratch("absent");
        let path = dir.join("never_written.bin");
        let absent_dir = dir.join("never_created");
        let started = std::time::Instant::now();

        assert_eq!(open(&path).unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(
            open_with(OpenOptions::new().read(true), &path)
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(read(&path).unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(
            read_to_string(&path).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            remove_file(&path).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            remove_dir_all(&absent_dir).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            with_read_guard(&path, || unreachable!("the guard must not be acquired"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            with_read_guards(&[path.as_path()], || unreachable!(
                "the guards must not be acquired"
            ))
            .unwrap_err()
            .kind(),
            io::ErrorKind::NotFound
        );
        // The two TREE forms are entry points too, and the timing assertion
        // below is the reason they are repeated here rather than left to the
        // test that owns their error behaviour: an absent root reaches the
        // classifying `metadata` call, which is inside the retry loop, so a
        // policy that treated `NotFound` as transient would wait out the whole
        // budget HERE while still returning the right error kind there.
        assert_eq!(
            tree_read_guards(&absent_dir).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            with_tree_read_guard(&absent_dir, || unreachable!(
                "the guards must not be acquired"
            ))
            .unwrap_err()
            .kind(),
            io::ErrorKind::NotFound
        );
        assert!(
            started.elapsed() < RETRY_BUDGET,
            "an absent path waited out the retry budget"
        );
        let _ = remove_dir_all(&dir);
    }

    /// A directory guard is NOT a file guard, and the tree form is.
    ///
    /// The left half is the mistake this API exists to prevent: holding the
    /// DIRECTORY leaves every file under it open to an exclusive opener. The
    /// right half is the fix. Both halves are asserted, so neither can quietly
    /// become the other.
    ///
    /// Production change that would make this fail: implementing
    /// `with_tree_read_guard` as a single guard on the root, or having it walk
    /// only the top level and skip subdirectories.
    #[cfg(windows)]
    #[test]
    fn a_directory_guard_holds_nothing_beneath_it_but_the_tree_guard_does() {
        use std::os::windows::fs::OpenOptionsExt as _;

        let dir = scratch("tree");
        std::fs::create_dir_all(dir.join("nested")).expect("create nested");
        let shallow = dir.join("shallow.bin");
        let deep = dir.join("nested/deep.bin");
        std::fs::write(&shallow, b"s").expect("write shallow");
        std::fs::write(&deep, b"d").expect("write deep");

        let exclusive = |path: &std::path::Path| {
            OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(path)
                .map(drop)
        };

        // A DIRECTORY handed to the single-path guard does not guard its
        // contents — it cannot even be acquired.
        assert_eq!(
            with_read_guard(&dir, || unreachable!("the guard must not be acquired"))
                .expect_err("a directory is not a guardable file")
                .kind(),
            io::ErrorKind::PermissionDenied
        );

        // Guarding the TREE: neither is, at any depth.
        with_tree_read_guard(&dir, || {
            for path in [&shallow, &deep] {
                assert_eq!(
                    exclusive(path)
                        .expect_err("every file in the tree must refuse an exclusive open")
                        .raw_os_error(),
                    Some(ERROR_SHARING_VIOLATION),
                    "{} was not held",
                    path.display()
                );
            }
        })
        .expect("acquire the tree guards");

        // Released afterwards.
        exclusive(&deep).expect("the tree guards must be released when the call returns");

        // And a single FILE handed to the tree form guards that file, so a
        // caller that does not know which shape it has can use it either way.
        with_tree_read_guard(&shallow, || {
            assert_eq!(
                exclusive(&shallow)
                    .expect_err("a file handed to the tree form must still be held")
                    .raw_os_error(),
                Some(ERROR_SHARING_VIOLATION)
            );
        })
        .expect("acquire the guard for a single file");
        let _ = remove_dir_all(&dir);
    }

    /// A tree the guards cannot be acquired over is an ERROR, not silence.
    ///
    /// The earlier implementation swallowed every acquisition failure and
    /// returned an empty guard list, so a caller would run its operation
    /// believing the tree was held. An absent root is the cheapest way to drive
    /// that path deterministically.
    ///
    /// Production change that would make this fail: going back to `is_file()`,
    /// which collapses the error to `false`, sends a missing path down the
    /// directory branch, and yields zero guards.
    ///
    /// SCOPE, precisely. An absent root returns at the classifying `metadata`
    /// call and never enters `walk`, so this test says nothing about the
    /// failures INSIDE the walk — restoring the swallowed `read_dir` or `open`
    /// results would not fail it. The child-open half of that is covered by
    /// `a_file_the_walk_cannot_open_fails_the_whole_tree_guard` below; the
    /// `read_dir` and `file_type` halves are propagated by construction (`?`
    /// on both, with no fallback arm to fall into) and are not separately
    /// driven, because making `read_dir` fail on a directory that `metadata`
    /// just succeeded on needs a reparse point and buys less than it costs.
    #[test]
    fn a_tree_that_cannot_be_guarded_is_an_error_not_an_empty_guard_list() {
        let dir = scratch("tree_absent");
        let absent = dir.join("never_created");
        assert_eq!(
            tree_read_guards(&absent).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            with_tree_read_guard(&absent, || unreachable!(
                "the operation must not run without its guards"
            ))
            .unwrap_err()
            .kind(),
            io::ErrorKind::NotFound
        );
        let _ = remove_dir_all(&dir);
    }

    /// A file the walk cannot open fails the WHOLE call — no partial list.
    ///
    /// This is the half the absent-root test above cannot reach: the root
    /// classifies as a directory, `walk` runs, and a child refuses to open.
    /// The swallowed-error implementation returned the guards it had managed
    /// to take and `Ok`, so a caller holding two of three files believed it
    /// held the tree — worse than holding none, because it looks like success.
    ///
    /// The refusal is produced the same way the real defect produces it: a
    /// second handle opened with `share_mode(0)`, which is precisely what a
    /// real-time scanner does to a freshly written file.
    ///
    /// Production change that would make this fail: ignoring the `open` result
    /// inside `walk` (pushing only the successes), or collecting errors and
    /// returning `Ok` with a partial list.
    ///
    /// COST: this is the one test in the crate that waits — about 4 s, two
    /// exhausted budgets for the two calls — because the exclusive holder
    /// never lets go and each call must spend `RETRY_BUDGET` before it may
    /// report the failure. That is the budget doing its job. Nothing here
    /// asserts a wall-clock ceiling, and the comment at the end of the test
    /// says why.
    #[cfg(windows)]
    #[test]
    fn a_file_the_walk_cannot_open_fails_the_whole_tree_guard() {
        use std::os::windows::fs::OpenOptionsExt as _;

        let dir = scratch("tree_unopenable");
        std::fs::create_dir_all(dir.join("nested")).expect("create nested");
        let reachable = dir.join("reachable.bin");
        let held = dir.join("nested/held.bin");
        std::fs::write(&reachable, b"r").expect("write reachable");
        std::fs::write(&held, b"h").expect("write held");

        // Held for the whole call, so the retry can never succeed.
        //
        // Taken through this crate's OWN retrying opener, not a bare
        // `OpenOptions::open`. `held.bin` was written on the line above, and an
        // un-retried exclusive open of a freshly written file is precisely the
        // exposure this crate exists to remove — the test would then be able to
        // fail for the very reason it is testing.
        let exclusive = open_with(OpenOptions::new().read(true).share_mode(0), &held)
            .expect("take the exclusive handle");

        let error = tree_read_guards(&dir).expect_err("a tree with an unopenable file is an error");
        assert_eq!(
            error.raw_os_error(),
            Some(ERROR_SHARING_VIOLATION),
            "the child's own error must arrive unchanged"
        );
        assert!(
            with_tree_read_guard(&dir, || unreachable!(
                "the operation must not run over a partially guarded tree"
            ))
            .is_err(),
            "the closure form must refuse the same tree"
        );
        // NO WALL-CLOCK ASSERTION HERE, deliberately. The crate documents that
        // it promises no ceiling on the call — the attempt in flight runs to
        // completion and the OS may oversleep a backoff — so a "finished within
        // N seconds" assertion would be asserting something production does not
        // guarantee, and would flake under battery load rather than catch a
        // defect. That both calls RETURNED at all, while the holder still has
        // the file, is the termination proof; the budget policy itself is
        // pinned exactly by the fake-clock tests above.

        drop(exclusive);
        // With the holder gone the same tree is guardable, which is what makes
        // the failure above about the sharing violation and not about the tree.
        tree_read_guards(&dir).expect("the tree is guardable once the file is released");
        let _ = remove_dir_all(&dir);
    }

    /// On a target with no sharing violation the classifier is unconditionally
    /// false and the loop does not exist, so every operation runs once.
    #[cfg(not(windows))]
    #[test]
    fn nothing_is_transient_off_windows_and_nothing_loops() {
        for raw in [32, 33, 5, 2] {
            assert!(!is_transient_sharing_violation(
                &io::Error::from_raw_os_error(raw)
            ));
        }
        let attempts = std::cell::Cell::new(0);
        let outcome = run::<()>(
            "test",
            || {
                attempts.set(attempts.get() + 1);
                Err(io::Error::from_raw_os_error(32))
            },
            || panic!("no observer may fire off Windows"),
        );
        assert!(outcome.is_err());
        assert_eq!(attempts.get(), 1);
    }
}
