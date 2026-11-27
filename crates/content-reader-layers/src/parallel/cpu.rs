use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Default)]
struct Totals {
    ns: AtomicU64,
    threads: AtomicU64,
    missing: AtomicU64,
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<Totals>>> = const { RefCell::new(None) };
}

pub struct Measurement {
    totals: Arc<Totals>,
    scope: Scope,
}

pub struct Sample {
    pub ns: Option<u64>,
    pub threads: u64,
}

impl Measurement {
    pub fn start() -> Self {
        let totals = Arc::new(Totals::default());
        Self {
            scope: Scope::enter(Some(Arc::clone(&totals))),
            totals,
        }
    }

    pub fn finish(self) -> Sample {
        let Self { totals, scope } = self;
        drop(scope);
        Sample {
            ns: (totals.missing.load(Ordering::Relaxed) == 0)
                .then(|| totals.ns.load(Ordering::Relaxed)),
            threads: totals.threads.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone)]
pub(super) struct Captured(Option<Arc<Totals>>);

impl Captured {
    pub(super) fn capture() -> Self {
        Self(CURRENT.with(|current| current.borrow().clone()))
    }

    pub(super) fn enter(&self) -> Scope {
        Scope::enter(self.0.clone())
    }
}

pub(super) struct Scope {
    totals: Option<Arc<Totals>>,
    previous: Option<Arc<Totals>>,
    counted: bool,
    started: Option<u64>,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Scope {
    fn enter(totals: Option<Arc<Totals>>) -> Self {
        let previous = CURRENT.with(|current| current.replace(totals.clone()));
        let counted = match (&totals, &previous) {
            (Some(totals), Some(held)) => !Arc::ptr_eq(held, totals),
            (Some(_), None) => true,
            (None, _) => false,
        };
        let started = counted.then(current).flatten();
        Self {
            totals,
            previous,
            counted,
            started,
            _thread: std::marker::PhantomData,
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        if self.counted
            && let Some(totals) = &self.totals
        {
            totals.threads.fetch_add(1, Ordering::Relaxed);
            match self
                .started
                .zip(current())
                .and_then(|(from, to)| to.checked_sub(from))
            {
                Some(ns) => {
                    totals.ns.fetch_add(ns, Ordering::Relaxed);
                }
                None => {
                    totals.missing.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        CURRENT.with(|current| {
            current.replace(self.previous.take());
        });
    }
}

#[cfg(windows)]
fn current() -> Option<u64> {
    #[repr(C)]
    #[derive(Default)]
    struct Time {
        low: u32,
        high: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> *mut std::ffi::c_void;
        fn GetThreadTimes(
            thread: *mut std::ffi::c_void,
            created: *mut Time,
            exited: *mut Time,
            kernel: *mut Time,
            user: *mut Time,
        ) -> i32;
    }
    let (mut created, mut exited, mut kernel, mut user) = (
        Time::default(),
        Time::default(),
        Time::default(),
        Time::default(),
    );
    let ok = unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &raw mut created,
            &raw mut exited,
            &raw mut kernel,
            &raw mut user,
        )
    };
    (ok != 0).then(|| {
        ((u64::from(kernel.high) << 32 | u64::from(kernel.low))
            + (u64::from(user.high) << 32 | u64::from(user.low)))
            * 100
    })
}

#[cfg(not(windows))]
fn current() -> Option<u64> {
    None
}
