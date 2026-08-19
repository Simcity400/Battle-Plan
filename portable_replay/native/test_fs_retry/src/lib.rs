use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::time::Duration;
#[cfg(windows)]
use std::time::Instant;

pub const RETRY_BUDGET: Duration = Duration::from_millis(2000);

#[cfg(windows)]
const FIRST_BACKOFF: Duration = Duration::from_millis(1);

#[cfg(windows)]
const MAX_BACKOFF: Duration = Duration::from_millis(16);

#[cfg(windows)]
const ERROR_SHARING_VIOLATION: i32 = 32;

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

#[cfg(windows)]
trait Clock {
    fn elapsed(&self) -> Duration;
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

/// `remove_file` — not a parallel copy of them — and still release the handle
#[cfg(all(windows, test))]
mod transient_hook {
    use std::cell::RefCell;

    thread_local! {
        static HOOK: RefCell<Option<Box<dyn FnMut()>>> = const { RefCell::new(None) };
    }

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

/// sharing violation deterministically: the test releases the handle that
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

#[cfg(not(windows))]
fn run<T>(
    _operation: &str,
    mut op: impl FnMut() -> io::Result<T>,
    _on_transient: impl FnMut(),
) -> io::Result<T> {
    op()
}

pub fn open(path: impl AsRef<Path>) -> io::Result<File> {
    let path = path.as_ref();
    run("open", || File::open(path), || {})
}

pub fn open_with(options: &OpenOptions, path: impl AsRef<Path>) -> io::Result<File> {
    let path = path.as_ref();
    run("open_with", || options.open(path), || {})
}

pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let path = path.as_ref();
    run("read", || std::fs::read(path), || {})
}

pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    let path = path.as_ref();
    run("read_to_string", || std::fs::read_to_string(path), || {})
}

pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    run("remove_file", || std::fs::remove_file(path), || {})
}

pub fn remove_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    run("remove_dir_all", || std::fs::remove_dir_all(path), || {})
}

///     .expect("acquire the read guard")
pub fn with_read_guard<T>(path: impl AsRef<Path>, op: impl FnOnce() -> T) -> io::Result<T> {
    let guard = open(path)?;
    let outcome = op();
    drop(guard);
    Ok(outcome)
}

pub fn with_read_guards<T>(paths: &[&Path], op: impl FnOnce() -> T) -> io::Result<T> {
    let mut guards = Vec::with_capacity(paths.len());
    for path in paths {
        guards.push(open(path)?);
    }
    let outcome = op();
    drop(guards);
    Ok(outcome)
}

pub fn with_tree_read_guard<T>(root: impl AsRef<Path>, op: impl FnOnce() -> T) -> io::Result<T> {
    let guards = tree_read_guards(root)?;
    let outcome = op();
    drop(guards);
    Ok(outcome)
}

#[must_use = "the guards do nothing unless they are held"]
pub fn tree_read_guards(root: impl AsRef<Path>) -> io::Result<Vec<File>> {
    fn walk(directory: &Path, guards: &mut Vec<File>) -> io::Result<()> {
        let entries = run("read_dir", || std::fs::read_dir(directory), || {})?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            match entry.file_type()? {
                kind if kind.is_dir() => walk(&path, guards)?,
                kind if kind.is_file() => guards.push(open(&path)?),
                _ => {}
            }
        }
        Ok(())
    }

    let root = root.as_ref();
    let mut guards = Vec::new();
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

    #[cfg(windows)]
    #[test]
    fn every_other_error_fails_on_the_first_attempt() {
        for error in [
            io::Error::new(io::ErrorKind::NotFound, "absent"),
            io::Error::new(io::ErrorKind::PermissionDenied, "refused"),
            io::Error::from_raw_os_error(5),
            io::Error::from_raw_os_error(2),
            io::Error::from_raw_os_error(33),
            io::Error::from_raw_os_error(183),
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
        assert_eq!(
            attempts.get(),
            slept.len(),
            "an attempt was committed to at or past the deadline"
        );
    }

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

    /// open to fail with error 32 — and released from inside the loop's own
    /// The function under test is the PUBLIC `open` itself. The release is
    #[cfg(windows)]
    #[test]
    fn open_recovers_from_a_real_sharing_violation_it_actually_observed() {
        use std::os::windows::fs::OpenOptionsExt as _;

        let dir = scratch("real_open");
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
            File::open(&path)
                .expect_err("a plain open must be refused while held")
                .raw_os_error(),
            Some(ERROR_SHARING_VIOLATION),
            "the reproduction no longer reproduces"
        );

        // The hook releases the handle from inside the loop, so the recovery on
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
        OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .expect("the guard must be released when the call returns");
        let _ = remove_dir_all(&dir);
    }

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

        // contents — it cannot even be acquired.
        assert_eq!(
            with_read_guard(&dir, || unreachable!("the guard must not be acquired"))
                .expect_err("a directory is not a guardable file")
                .kind(),
            io::ErrorKind::PermissionDenied
        );

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

        drop(exclusive);
        tree_read_guards(&dir).expect("the tree is guardable once the file is released");
        let _ = remove_dir_all(&dir);
    }

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
