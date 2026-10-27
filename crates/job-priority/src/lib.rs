//! The scheduling priority of the preparation job a thread works for.
//!
//! A job prepared ahead of the user (from the recording library) runs each
//! of its threads one step below the level that thread has in a job the user
//! is waiting for, so the relative order of a job's threads stays the same.
//! A thread's priority does not pass to the threads it starts, so every
//! preparation thread is started with [`JobPriority::current`] and enters it
//! first with [`JobPriority::enter`].

pub mod accounting;

use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

thread_local! {
    static JOB: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
    /// The thread runs in background mode, and so do the threads it starts.
    static BACKGROUND: Cell<bool> = const { Cell::new(false) };
}

/// Run the calling thread in background mode from now on, at the lowest CPU, I/O and memory
/// priority, and so every thread it starts with [`JobPriority::current`]: work no one waits for.
pub fn background_thread() {
    BACKGROUND.set(true);
    enter_background_mode();
}

/// Leave background CPU, I/O and memory mode on the calling worker only.
pub fn foreground_thread() {
    let _ = worker(false);
}

pub fn is_background_thread() -> bool {
    BACKGROUND.get()
}

/// Apply a promotable recorder job without claiming a refused mode transition succeeded.
pub fn worker(background: bool) -> std::io::Result<()> {
    if BACKGROUND.get() == background {
        return Ok(());
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
            THREAD_MODE_BACKGROUND_END,
        };
        let mode = if background {
            THREAD_MODE_BACKGROUND_BEGIN
        } else {
            THREAD_MODE_BACKGROUND_END
        };
        if SetThreadPriority(GetCurrentThread(), mode) == 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    BACKGROUND.set(background);
    Ok(())
}

fn enter_background_mode() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
        };
        SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
}

/// The level a thread runs at in a job the user is waiting for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// Coordinators and the families' own threads.
    Normal,
    /// Pre-pass and helper work that yields to the coordinators.
    BelowNormal,
}

/// The job a thread works for: whether it is (still) ahead of the user. The
/// flag is shared with the job's progress, so a job the user joins stops
/// lowering the threads it starts from then on.
#[derive(Clone, Debug, Default)]
pub struct JobPriority(Option<Arc<AtomicBool>>, bool);

impl JobPriority {
    /// The job this thread works for, to hand to a thread it starts.
    pub fn current() -> Self {
        JOB.with(|job| Self(job.borrow().clone(), BACKGROUND.get()))
    }

    /// Run `work` at the coordinator level for the job whose flag is
    /// `background`, restoring the previous job and thread priority afterward.
    pub fn run<T>(background: Arc<AtomicBool>, work: impl FnOnce() -> T) -> T {
        struct Restore(Option<Option<Arc<AtomicBool>>>);
        impl Drop for Restore {
            fn drop(&mut self) {
                if let Some(previous) = self.0.take() {
                    JOB.with(|job| *job.borrow_mut() = previous);
                }
            }
        }
        let caller = Caller::current();
        struct Priority(Option<Caller>);
        impl Drop for Priority {
            fn drop(&mut self) {
                if let Some(caller) = self.0.take() {
                    caller.enter();
                }
            }
        }
        let _priority = Priority(Some(caller));
        let _restore = Restore(Some(JOB.with(|job| job.replace(Some(background)))));
        JobPriority::current().enter(Level::Normal);
        work()
    }

    /// Whether the job is prepared ahead of the user now.
    pub fn background(&self) -> bool {
        self.0
            .as_ref()
            .is_some_and(|background| background.load(Ordering::Acquire))
    }

    /// Enter the job on a new or reused thread, which runs at `level`, one
    /// step lower while the job is ahead of the user; in background mode when
    /// the thread that started it is.
    pub fn enter(self, level: Level) {
        let lowered = self.background();
        JOB.with(|job| *job.borrow_mut() = self.0);
        if self.1 {
            background_thread();
        } else {
            set_thread_priority(level, lowered);
        }
    }
}

/// The calling thread's job and its own priority, for a helper thread that
/// works for it synchronously and so runs exactly as it does.
#[derive(Clone, Debug)]
pub struct Caller {
    job: JobPriority,
    priority: i32,
}

impl Caller {
    pub fn current() -> Self {
        #[cfg(windows)]
        let priority = unsafe {
            use windows_sys::Win32::System::Threading::{GetCurrentThread, GetThreadPriority};
            GetThreadPriority(GetCurrentThread())
        };
        #[cfg(not(windows))]
        let priority = 0;
        Self {
            job: JobPriority::current(),
            priority,
        }
    }

    /// Enter the caller's job at the caller's priority on a new thread.
    pub fn enter(self) {
        JOB.with(|job| *job.borrow_mut() = self.job.0);
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority};
            SetThreadPriority(GetCurrentThread(), self.priority);
        }
    }
}

fn set_thread_priority(level: Level, lowered: bool) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
            THREAD_PRIORITY_LOWEST, THREAD_PRIORITY_NORMAL,
        };
        let priority = match (level, lowered) {
            (Level::Normal, false) => THREAD_PRIORITY_NORMAL,
            (Level::Normal, true) | (Level::BelowNormal, false) => THREAD_PRIORITY_BELOW_NORMAL,
            (Level::BelowNormal, true) => THREAD_PRIORITY_LOWEST,
        };
        SetThreadPriority(GetCurrentThread(), priority);
    }
    #[cfg(not(windows))]
    let _ = (level, lowered);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The priority a thread started for `priority` runs at, entering `level`.
    #[cfg(windows)]
    fn started_at(priority: &JobPriority, level: Level) -> i32 {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, GetThreadPriority};
        let priority = priority.clone();
        std::thread::spawn(move || {
            priority.enter(level);
            unsafe { GetThreadPriority(GetCurrentThread()) }
        })
        .join()
        .unwrap()
    }

    /// Existing runner threads and reused helpers must enter the requested
    /// level just as freshly spawned family threads do, and restore on return.
    #[test]
    #[cfg(windows)]
    fn jobs_set_and_restore_the_calling_threads_priority() {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, GetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
            THREAD_PRIORITY_NORMAL,
        };
        std::thread::spawn(|| {
            let current = || unsafe { GetThreadPriority(GetCurrentThread()) };
            JobPriority::default().enter(Level::BelowNormal);
            let previous = current();
            for background in [false, true] {
                JobPriority::run(Arc::new(AtomicBool::new(background)), || {
                    assert_eq!(
                        current(),
                        if background {
                            THREAD_PRIORITY_BELOW_NORMAL
                        } else {
                            THREAD_PRIORITY_NORMAL
                        }
                    );
                    // Switching a reused helper back from BelowNormal matters.
                    JobPriority::current().enter(Level::BelowNormal);
                    JobPriority::current().enter(Level::Normal);
                    assert_eq!(
                        current(),
                        if background {
                            THREAD_PRIORITY_BELOW_NORMAL
                        } else {
                            THREAD_PRIORITY_NORMAL
                        }
                    );
                });
                assert_eq!(current(), previous);
                assert!(!JobPriority::current().background());
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn a_helper_runs_exactly_as_its_caller() {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, GetThreadPriority, THREAD_PRIORITY_LOWEST,
        };
        let background = Arc::new(AtomicBool::new(true));
        JobPriority::run(background, || {
            let job = JobPriority::current();
            let (helper, background) = std::thread::spawn(move || {
                job.enter(Level::BelowNormal);
                let caller = Caller::current();
                std::thread::spawn(move || {
                    caller.enter();
                    (
                        unsafe { GetThreadPriority(GetCurrentThread()) },
                        JobPriority::current().background(),
                    )
                })
                .join()
                .unwrap()
            })
            .join()
            .unwrap();
            assert_eq!(helper, THREAD_PRIORITY_LOWEST);
            assert!(background);
        });
    }

    #[test]
    #[cfg(windows)]
    fn a_background_job_starts_every_thread_one_step_lower() {
        use windows_sys::Win32::System::Threading::{
            THREAD_PRIORITY_BELOW_NORMAL, THREAD_PRIORITY_LOWEST, THREAD_PRIORITY_NORMAL,
        };
        let outside = JobPriority::current();
        assert_eq!(started_at(&outside, Level::Normal), THREAD_PRIORITY_NORMAL);
        assert_eq!(
            started_at(&outside, Level::BelowNormal),
            THREAD_PRIORITY_BELOW_NORMAL
        );
        let background = Arc::new(AtomicBool::new(true));
        JobPriority::run(Arc::clone(&background), || {
            let job = JobPriority::current();
            assert_eq!(
                started_at(&job, Level::Normal),
                THREAD_PRIORITY_BELOW_NORMAL
            );
            assert_eq!(started_at(&job, Level::BelowNormal), THREAD_PRIORITY_LOWEST);
            // The user joins: threads started from then on run at their level.
            background.store(false, Ordering::Release);
            assert_eq!(started_at(&job, Level::Normal), THREAD_PRIORITY_NORMAL);
        });
    }

    #[test]
    fn a_persistent_recorder_worker_leaves_and_reenters_background_mode() {
        std::thread::spawn(|| {
            for requested in [true, false, true, false] {
                worker(requested).unwrap();
                assert_eq!(BACKGROUND.get(), requested);
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    fn threads_a_background_thread_starts_run_in_background_mode() {
        let (started, nested) = std::thread::spawn(|| {
            background_thread();
            let job = JobPriority::current();
            std::thread::spawn(move || {
                job.enter(Level::Normal);
                let inner = JobPriority::current();
                (
                    BACKGROUND.get(),
                    std::thread::spawn(move || {
                        inner.enter(Level::BelowNormal);
                        BACKGROUND.get()
                    })
                    .join()
                    .unwrap(),
                )
            })
            .join()
            .unwrap()
        })
        .join()
        .unwrap();
        assert!(started && nested);
        let outside = std::thread::spawn(|| {
            JobPriority::current().enter(Level::Normal);
            BACKGROUND.get()
        })
        .join()
        .unwrap();
        assert!(!outside);
    }

    #[test]
    fn started_threads_inherit_the_job_and_follow_a_join() {
        assert!(!JobPriority::current().background(), "no job outside one");
        let background = Arc::new(AtomicBool::new(true));
        JobPriority::run(Arc::clone(&background), || {
            let priority = JobPriority::current();
            assert!(priority.background());
            // A thread started for the job, and one it starts in turn.
            let nested = std::thread::spawn(move || {
                priority.enter(Level::BelowNormal);
                let inner = JobPriority::current();
                std::thread::spawn(move || {
                    inner.enter(Level::Normal);
                    JobPriority::current().background()
                })
                .join()
                .unwrap()
            })
            .join()
            .unwrap();
            assert!(nested, "nested threads keep the job");
            // The user joins the job: threads started from then on run at
            // their own level.
            background.store(false, Ordering::Release);
            assert!(!JobPriority::current().background());
        });
        assert!(
            !JobPriority::current().background(),
            "the job ends with its run"
        );
    }
}
