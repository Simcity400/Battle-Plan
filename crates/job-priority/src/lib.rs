
pub mod accounting;

use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub type Context = &'static (dyn std::any::Any + Send + Sync);

pub fn context() -> Option<Context> {
    CONTEXT.get()
}

pub fn set_context(context: Option<Context>) {
    CONTEXT.set(context);
}

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

pub fn carry<T>(work: impl FnOnce() -> T) -> impl FnOnce() -> T {
    let context = CONTEXT.get();
    move || with_context(context, work)
}

pub fn spawn<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> std::thread::JoinHandle<T> {
    std::thread::spawn(carry(work))
}

pub fn spawn_in<'scope, 'env, T: Send + 'scope>(
    scope: &'scope std::thread::Scope<'scope, 'env>,
    work: impl FnOnce() -> T + Send + 'scope,
) -> std::thread::ScopedJoinHandle<'scope, T> {
    scope.spawn(carry(work))
}

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
    static BACKGROUND: Cell<bool> = const { Cell::new(false) };
    static BACKGROUND_CORE: Cell<Option<Processors>> = const { Cell::new(None) };
    static CONFINED: Cell<Option<Processors>> = const { Cell::new(None) };
    static LEVEL: Cell<Option<(Level, bool)>> = const { Cell::new(None) };
}

static BACKGROUND_CORE_TARGET: std::sync::OnceLock<Option<Processors>> =
    std::sync::OnceLock::new();

fn background_core_target() -> Option<Processors> {
    *BACKGROUND_CORE_TARGET.get_or_init(|| {
        if !Processors::one_group() {
            return None;
        }
        let process = Processors::process()?;
        let mut list: Vec<usize> = cores(process.group)
            .into_iter()
            .map(|core| core & process.mask)
            .filter(|core| *core != 0)
            .collect();
        if list.len() < 6 {
            return None;
        }
        list.sort_unstable();
        Some(Processors {
            mask: *list.last()?,
            group: process.group,
        })
    })
}

pub fn confine_background_core(confine: bool) {
    if confine {
        if BACKGROUND_CORE.get().is_some() {
            return;
        }
        let Some(target) = background_core_target() else {
            return;
        };
        let Some(previous) = Processors::current() else {
            return;
        };
        BACKGROUND_CORE.set(Some(previous));
        target.enter();
    } else if let Some(previous) = BACKGROUND_CORE.take() {
        previous.enter();
    }
}

pub fn follow() {
    let Some((level, lowered)) = LEVEL.get() else {
        return;
    };
    if lowered && !JobPriority::current().background() {
        set_thread_priority(level, false);
        LEVEL.set(Some((level, false)));
    }
}

pub fn confined<T>(work: impl FnOnce() -> T) -> T {
    struct Restore(Option<Processors>, Option<Processors>, Caller);
    impl Drop for Restore {
        fn drop(&mut self) {
            CONFINED.set(self.0);
            if let Some(processors) = self.1 {
                processors.enter();
            }
            self.2.clone().enter();
        }
    }
    let all = Processors::current().filter(|_| Processors::one_group());
    let _restore = Restore(CONFINED.get(), all, Caller::current());
    set_thread_priority(Level::BelowNormal, false);
    if let Some(half) = all.and_then(Processors::half) {
        CONFINED.set(Some(half));
        half.enter();
    }
    work()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Processors {
    mask: usize,
    group: u16,
}

impl Processors {
    fn current() -> Option<Self> {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::{
                SystemInformation::GROUP_AFFINITY,
                Threading::{GetCurrentThread, GetThreadGroupAffinity},
            };
            let mut affinity: GROUP_AFFINITY = std::mem::zeroed();
            (GetThreadGroupAffinity(GetCurrentThread(), &raw mut affinity) != 0).then_some(Self {
                mask: affinity.Mask,
                group: affinity.Group,
            })
        }
        #[cfg(not(windows))]
        None
    }

    fn one_group() -> bool {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::Threading::GetActiveProcessorGroupCount() == 1
        }
        #[cfg(not(windows))]
        false
    }

    fn process() -> Option<Self> {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessAffinityMask};
            let (mut process, mut system) = (0, 0);
            (GetProcessAffinityMask(GetCurrentProcess(), &raw mut process, &raw mut system) != 0)
                .then_some(Self {
                    mask: process,
                    group: 0,
                })
        }
        #[cfg(not(windows))]
        None
    }

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
            SetThreadGroupAffinity(GetCurrentThread(), &raw const affinity, std::ptr::null_mut());
        }
    }

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

fn upper_half(mut cores: Vec<usize>) -> Option<usize> {
    if cores.len() < 2 {
        return None;
    }
    cores.sort_unstable();
    let kept = cores.len() / 2;
    Some(cores[cores.len() - kept..].iter().fold(0, |mask, core| mask | core))
}

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
        GetLogicalProcessorInformationEx(RelationProcessorCore, std::ptr::null_mut(), &raw mut length);
        let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
        if GetLogicalProcessorInformationEx(
            RelationProcessorCore,
            buffer.as_mut_ptr().cast(),
            &raw mut length,
        ) == 0
        {
            return Vec::new();
        }
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
                    #[expect(clippy::cast_ptr_alignment, reason = "the pointer is only read through read_unaligned")]
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

pub fn background_thread() {
    BACKGROUND.set(true);
    enter_background_mode();
}

pub fn foreground_thread() {
    let _ = worker(false);
}

pub fn is_background_thread() -> bool {
    BACKGROUND.get()
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Normal,
    BelowNormal,
}

#[derive(Clone, Debug, Default)]
pub struct JobPriority(Option<Arc<AtomicBool>>, bool, Option<Processors>, Carried);

impl JobPriority {
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

    pub fn background(&self) -> bool {
        self.0
            .as_ref()
            .is_some_and(|background| background.load(Ordering::Acquire))
    }

    pub fn enter(self, level: Level) {
        let lowered = self.background();
        Processors::confine(self.2);
        CONTEXT.set(self.3.0);
        JOB.with(|job| *job.borrow_mut() = self.0);
        if self.1 {
            LEVEL.set(None);
            background_thread();
        } else {
            LEVEL.set(Some((level, lowered)));
            set_thread_priority(level, lowered);
        }
    }
}

#[derive(Clone, Debug)]
pub struct Caller {
    job: JobPriority,
    priority: i32,
    level: Option<(Level, bool)>,
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
            level: LEVEL.get(),
        }
    }

    pub fn enter(self) {
        Processors::confine(self.job.2);
        CONTEXT.set(self.job.3.0);
        JOB.with(|job| *job.borrow_mut() = self.job.0);
        LEVEL.set(self.level);
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

    #[cfg(windows)]
    fn confined_thread(then: impl FnOnce()) {
        let caller = confined(Caller::current);
        caller.enter();
        then();
    }
}
