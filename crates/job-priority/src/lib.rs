//! The scheduling priority of the preparation job a thread works for.
//!
//! A job prepared ahead of the user (from the recording library) runs each
//! of its threads one step below the level that thread has in a job the user
//! is waiting for, so the relative order of a job's threads stays the same.
//! A thread's priority does not pass to the threads it starts, so every
//! preparation thread is started with [`JobPriority::current`] and enters it
//! first with [`JobPriority::enter`].

use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

thread_local! {
    static JOB: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
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
pub struct JobPriority(Option<Arc<AtomicBool>>);

impl JobPriority {
    /// The job this thread works for, to hand to a thread it starts.
    pub fn current() -> Self {
        JOB.with(|job| Self(job.borrow().clone()))
    }

    /// Run `work` on this thread for the job whose flag is `background`.
    pub fn run<T>(background: Arc<AtomicBool>, work: impl FnOnce() -> T) -> T {
        struct Restore(Option<Option<Arc<AtomicBool>>>);
        impl Drop for Restore {
            fn drop(&mut self) {
                if let Some(previous) = self.0.take() {
                    JOB.with(|job| *job.borrow_mut() = previous);
                }
            }
        }
        let _restore = Restore(Some(JOB.with(|job| job.replace(Some(background)))));
        work()
    }

    /// Whether the job is prepared ahead of the user now.
    pub fn background(&self) -> bool {
        self.0
            .as_ref()
            .is_some_and(|background| background.load(Ordering::Acquire))
    }

    /// Enter the job on a newly started thread, which runs at `level`, one
    /// step lower while the job is ahead of the user.
    pub fn enter(self, level: Level) {
        let lowered = self.background();
        JOB.with(|job| *job.borrow_mut() = self.0);
        set_thread_priority(level, lowered);
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
            THREAD_PRIORITY_LOWEST,
        };
        let priority = match (level, lowered) {
            // A new thread starts at normal priority.
            (Level::Normal, false) => return,
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
