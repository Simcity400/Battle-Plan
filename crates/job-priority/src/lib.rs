//! The scheduling priority of the preparation job a thread works for.
//!
//! A job prepared ahead of the user (from the recording library) runs each
//! of its threads one step below the level that thread has in a job the user
//! is waiting for, so the relative order of a job's threads stays the same.
//! A thread's priority does not pass to the threads it starts, so every
//! preparation thread is started with [`JobPriority::current`] and enters it
//! first with [`JobPriority::enter`].
//!
//! Work read ahead of the user runs at the user's priority on half the
//! processors instead ([`confined`]), so a request that comes to wait for it
//! never waits behind other work, and the rest is left to the user's.

pub mod accounting;

use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// What a thread's work reads besides its priority: the executable tables of the recording it
/// works on. Every thread it starts with [`JobPriority::current`] or [`Caller::current`] works
/// with it too.
pub type Context = &'static (dyn std::any::Any + Send + Sync);

/// The calling thread's [`Context`].
pub fn context() -> Option<Context> {
    CONTEXT.get()
}

/// Makes `context` the calling thread's from now on, for a thread that works on one thing alone.
pub fn set_context(context: Option<Context>) {
    CONTEXT.set(context);
}

/// Runs `work` with `context` as the calling thread's, restoring the previous one afterward.
pub fn with_context<T>(context: Option<Context>, work: impl FnOnce() -> T) -> T {
    struct Restore(Option<Context>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CONTEXT.set(self.0);
        }
    }
    let _restore = Restore(CONTEXT.replace(context));
    work()
}

/// `work`, run with the calling thread's [`Context`] on whichever thread runs it.
pub fn carry<T>(work: impl FnOnce() -> T) -> impl FnOnce() -> T {
    let context = CONTEXT.get();
    move || with_context(context, work)
}

/// [`std::thread::spawn`], the new thread working with the calling thread's [`Context`].
pub fn spawn<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> std::thread::JoinHandle<T> {
    std::thread::spawn(carry(work))
}

/// [`std::thread::Scope::spawn`], the new thread working with the calling thread's [`Context`].
pub fn spawn_in<'scope, 'env, T: Send + 'scope>(
    scope: &'scope std::thread::Scope<'scope, 'env>,
    work: impl FnOnce() -> T + Send + 'scope,
) -> std::thread::ScopedJoinHandle<'scope, T> {
    scope.spawn(carry(work))
}

/// A named thread working with the calling thread's [`Context`].
pub trait SpawnCarried {
    fn spawn_carried<T: Send + 'static>(
        self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> std::io::Result<std::thread::JoinHandle<T>>;

    fn spawn_scoped_carried<'scope, 'env, T: Send + 'scope>(
        self,
        scope: &'scope std::thread::Scope<'scope, 'env>,
        work: impl FnOnce() -> T + Send + 'scope,
    ) -> std::io::Result<std::thread::ScopedJoinHandle<'scope, T>>;
}

impl SpawnCarried for std::thread::Builder {
    fn spawn_carried<T: Send + 'static>(
        self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> std::io::Result<std::thread::JoinHandle<T>> {
        self.spawn(carry(work))
    }

    fn spawn_scoped_carried<'scope, 'env, T: Send + 'scope>(
        self,
        scope: &'scope std::thread::Scope<'scope, 'env>,
        work: impl FnOnce() -> T + Send + 'scope,
    ) -> std::io::Result<std::thread::ScopedJoinHandle<'scope, T>> {
        self.spawn_scoped(scope, carry(work))
    }
}

/// A [`Context`] handed to another thread.
#[derive(Clone, Copy, Default)]
struct Carried(Option<Context>);

impl std::fmt::Debug for Carried {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(if self.0.is_some() { "Some(context)" } else { "None" })
    }
}

thread_local! {
    static CONTEXT: Cell<Option<Context>> = const { Cell::new(None) };
    static JOB: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
    /// The thread runs in background mode, and so do the threads it starts.
    static BACKGROUND: Cell<bool> = const { Cell::new(false) };
    /// The processors the thread's confined work runs on, and so do the threads it starts.
    static CONFINED: Cell<Option<Processors>> = const { Cell::new(None) };
}

/// Runs `work`, and every thread it starts with [`JobPriority::current`] or [`Caller::current`],
/// on half the cores the calling thread may run on: work ahead of the user, which leaves the
/// other half to the work the user waits for and runs at its priority, so a request that comes to
/// wait for it never waits behind other work. A system of more than one processor group, whose
/// threads a group affinity cannot restore, runs it unconfined.
pub fn confined<T>(work: impl FnOnce() -> T) -> T {
    struct Restore(Option<Processors>, Option<Processors>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CONFINED.set(self.0);
            if let Some(processors) = self.1 {
                processors.enter();
            }
        }
    }
    let all = Processors::current().filter(|_| Processors::one_group());
    let _restore = Restore(CONFINED.get(), all);
    if let Some(half) = all.and_then(Processors::half) {
        CONFINED.set(Some(half));
        half.enter();
    }
    work()
}

/// A set of processors a thread may run on, within its processor group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Processors {
    mask: usize,
    group: u16,
}

impl Processors {
    /// The processors the calling thread may run on.
    fn current() -> Option<Self> {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::{
                SystemInformation::GROUP_AFFINITY,
                Threading::{GetCurrentThread, GetThreadGroupAffinity},
            };
            let mut affinity: GROUP_AFFINITY = std::mem::zeroed();
            (GetThreadGroupAffinity(GetCurrentThread(), &mut affinity) != 0).then_some(Self {
                mask: affinity.Mask,
                group: affinity.Group,
            })
        }
        #[cfg(not(windows))]
        None
    }

    /// Whether the system has a single processor group.
    fn one_group() -> bool {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::Threading::GetActiveProcessorGroupCount() == 1
        }
        #[cfg(not(windows))]
        false
    }

    /// The processors the process may run on, in a system of one processor group.
    fn process() -> Option<Self> {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessAffinityMask};
            let (mut process, mut system) = (0, 0);
            (GetProcessAffinityMask(GetCurrentProcess(), &mut process, &mut system) != 0)
                .then_some(Self {
                    mask: process,
                    group: 0,
                })
        }
        #[cfg(not(windows))]
        None
    }

    /// The upper half of these processors' cores, whole; none of a single core.
    fn half(self) -> Option<Self> {
        let cores: Vec<usize> = cores(self.group)
            .into_iter()
            .map(|core| core & self.mask)
            .filter(|core| *core != 0)
            .collect();
        Some(Self {
            mask: upper_half(cores)?,
            ..self
        })
    }

    /// Runs the calling thread on these processors.
    fn enter(self) {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::{
                SystemInformation::GROUP_AFFINITY,
                Threading::{GetCurrentThread, SetThreadGroupAffinity},
            };
            let affinity = GROUP_AFFINITY {
                Mask: self.mask,
                Group: self.group,
                Reserved: [0; 3],
            };
            SetThreadGroupAffinity(GetCurrentThread(), &affinity, std::ptr::null_mut());
        }
    }

    /// Enters the confined processors of the thread that started this one's work, or leaves a
    /// confinement earlier work left for the process's processors.
    fn confine(processors: Option<Self>) {
        match processors {
            Some(processors) => {
                CONFINED.set(Some(processors));
                processors.enter();
            }
            None => {
                if CONFINED.take().is_some()
                    && let Some(all) = Self::process()
                {
                    all.enter();
                }
            }
        }
    }
}

/// The upper half of `cores`, each a mask of one core's processors; none of a single core.
fn upper_half(mut cores: Vec<usize>) -> Option<usize> {
    if cores.len() < 2 {
        return None;
    }
    cores.sort_unstable();
    let kept = cores.len() / 2;
    Some(cores[cores.len() - kept..].iter().fold(0, |mask, core| mask | core))
}

/// Each core of processor group `group`, as the mask of its processors.
fn cores(group: u16) -> Vec<usize> {
    #[cfg(windows)]
    unsafe {
        use std::mem::{offset_of, size_of};
        use windows_sys::Win32::System::SystemInformation::{
            GROUP_AFFINITY, GetLogicalProcessorInformationEx, LOGICAL_PROCESSOR_RELATIONSHIP,
            PROCESSOR_RELATIONSHIP, RelationProcessorCore, SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
        };
        type Record = SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX;
        let mut length = 0;
        GetLogicalProcessorInformationEx(RelationProcessorCore, std::ptr::null_mut(), &mut length);
        let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
        if GetLogicalProcessorInformationEx(
            RelationProcessorCore,
            buffer.as_mut_ptr().cast(),
            &mut length,
        ) == 0
        {
            return Vec::new();
        }
        // A core's record is shorter than its type, whose union holds larger relationships: each
        // field is read where it lies, within the record's own size.
        let bytes = std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), length as usize);
        let field = |at: usize, offset: usize, size: usize| bytes.get(at + offset..)?.get(..size);
        let relationship = (
            offset_of!(Record, Relationship),
            size_of::<LOGICAL_PROCESSOR_RELATIONSHIP>(),
        );
        let processor = offset_of!(Record, Anonymous);
        let masks = processor + offset_of!(PROCESSOR_RELATIONSHIP, GroupMask);
        let mut cores = Vec::new();
        let mut at = 0;
        while let (Some(kind), Some(size)) = (
            field(at, relationship.0, relationship.1),
            field(at, offset_of!(Record, Size), size_of::<u32>()),
        ) {
            let kind = kind.as_ptr().cast::<LOGICAL_PROCESSOR_RELATIONSHIP>().read_unaligned();
            let size = size.as_ptr().cast::<u32>().read_unaligned() as usize;
            if size == 0 || at + size > bytes.len() {
                break;
            }
            let count = field(
                at,
                processor + offset_of!(PROCESSOR_RELATIONSHIP, GroupCount),
                size_of::<u16>(),
            )
            .map(|count| count.as_ptr().cast::<u16>().read_unaligned());
            if kind == RelationProcessorCore
                && let Some(count) = count
            {
                for index in 0..usize::from(count) {
                    let offset = masks + index * size_of::<GROUP_AFFINITY>();
                    if offset + size_of::<GROUP_AFFINITY>() > size {
                        break;
                    }
                    let mask = bytes[at + offset..].as_ptr().cast::<GROUP_AFFINITY>();
                    let mask = mask.read_unaligned();
                    if mask.Group == group {
                        cores.push(mask.Mask);
                    }
                }
            }
            at += size;
        }
        cores
    }
    #[cfg(not(windows))]
    {
        let _ = group;
        Vec::new()
    }
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
pub struct JobPriority(Option<Arc<AtomicBool>>, bool, Option<Processors>, Carried);

impl JobPriority {
    /// The job this thread works for, to hand to a thread it starts.
    pub fn current() -> Self {
        JOB.with(|job| {
            Self(
                job.borrow().clone(),
                BACKGROUND.get(),
                CONFINED.get(),
                Carried(CONTEXT.get()),
            )
        })
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
        Processors::confine(self.2);
        CONTEXT.set(self.3.0);
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

    /// Enter the caller's job at the caller's priority on a new thread, on its processors.
    pub fn enter(self) {
        Processors::confine(self.job.2);
        CONTEXT.set(self.job.3.0);
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

    #[test]
    fn half_the_cores_are_the_upper_half_whole() {
        assert_eq!(upper_half(vec![0b11, 0b1100, 0b11_0000]), Some(0b11_0000));
        assert_eq!(
            upper_half(vec![0b1100_0000, 0b11, 0b1100, 0b11_0000]),
            Some(0b1111_0000)
        );
        assert_eq!(upper_half(vec![0b1, 0b10]), Some(0b10));
        assert_eq!(upper_half(vec![0b11]), None);
    }

    /// The half is whole cores of the system's own topology.
    #[test]
    #[cfg(windows)]
    fn half_the_processors_never_splits_a_core() {
        let all = Processors::current().unwrap();
        if let Some(half) = all.half() {
            for core in cores(all.group) {
                let kept = core & all.mask & half.mask;
                assert!(kept == 0 || kept == core & all.mask, "{core:#x} split by {half:?}");
            }
        }
    }

    /// Confined work and the threads it starts run on half the processors, and the thread runs
    /// on all of them again afterwards.
    #[test]
    #[cfg(windows)]
    fn confined_work_and_its_threads_run_on_half_the_processors() {
        std::thread::spawn(|| {
            let all = Processors::current().unwrap();
            let half = all.half();
            let (inside, helper, job) = confined(|| {
                let caller = Caller::current();
                let job = JobPriority::current();
                (
                    Processors::current(),
                    std::thread::spawn(move || {
                        caller.enter();
                        Processors::current()
                    })
                    .join()
                    .unwrap(),
                    std::thread::spawn(move || {
                        job.enter(Level::Normal);
                        Processors::current()
                    })
                    .join()
                    .unwrap(),
                )
            });
            let confined = half.filter(|_| Processors::one_group()).or(Some(all));
            assert_eq!((inside, helper, job), (confined, confined, confined));
            assert_eq!(Processors::current(), Some(all));
            assert_eq!(CONFINED.get(), None);
            // A thread that ran confined work and then enters unconfined work leaves it.
            let unconfined = JobPriority::current();
            let left = std::thread::spawn(move || {
                confined_thread(|| unconfined.enter(Level::Normal));
                Processors::current()
            })
            .join()
            .unwrap();
            let expected = if confined == Some(all) {
                Some(all)
            } else {
                Processors::process()
            };
            assert_eq!(left, expected);
        })
        .join()
        .unwrap();
    }

    /// Runs `then` on a thread confined as one a confined thread started is.
    #[cfg(windows)]
    fn confined_thread(then: impl FnOnce()) {
        let caller = confined(Caller::current);
        caller.enter();
        then();
    }
}
