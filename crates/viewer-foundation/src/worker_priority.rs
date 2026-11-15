mod bake;
pub use bake::at_bake_priority;

use std::{
    collections::BTreeMap,
    sync::{Mutex, PoisonError},
};

/// What a thread of ours is to the thread that draws the map. Each thread names its class once, at
/// its start ([`enter`]); its priority follows from the class, and the paced report lists every
/// thread entered with its class ([`entered`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    /// Draws the map: above normal, before every other thread of ours that is ready.
    Frame,
    /// Does the frame's own work while the frame waits for it: normal.
    FrameWork,
    /// Makes a product the frame takes when it is ready: below normal.
    Worker,
    /// Refines what is already shown: lowest.
    Refining,
    /// A worker at normal.
    Normal,
    /// Holds a lock the frame takes every frame while it works: normal, as a holder below the
    /// frame's other workers would keep the frame waiting longer.
    Kept,
}

impl Class {
    pub fn name(self) -> &'static str {
        match self {
            Self::Frame => "frame",
            Self::FrameWork => "frame work",
            Self::Worker => "worker",
            Self::Refining => "refining",
            Self::Normal => "normal",
            Self::Kept => "kept",
        }
    }

    /// Its thread priority.
    fn level(self) -> i32 {
        const LOWEST: i32 = -2;
        const BELOW_NORMAL: i32 = -1;
        const NORMAL: i32 = 0;
        const ABOVE_NORMAL: i32 = 1;
        match self {
            Self::Frame => ABOVE_NORMAL,
            Self::FrameWork | Self::Kept | Self::Normal => NORMAL,
            Self::Worker => BELOW_NORMAL,
            Self::Refining => LOWEST,
        }
    }
}

/// Threads entered, by name and class: the priority the last one read back, and how many entered.
static ENTERED: Mutex<BTreeMap<(String, Class), (Option<i32>, u64)>> = Mutex::new(BTreeMap::new());

/// A class of thread as the paced report lists it.
#[derive(Clone, Debug)]
pub struct Entered {
    pub name: String,
    pub class: Class,
    pub priority: Option<i32>,
    pub threads: u64,
}

/// The calling thread at its class's priority, listed for the paced report.
pub fn enter(class: Class) {
    set(class.level());
    let name = std::thread::current().name().unwrap_or("unnamed").to_owned();
    let priority = current_priority();
    let mut entered = ENTERED.lock().unwrap_or_else(PoisonError::into_inner);
    let held = entered.entry((name, class)).or_insert((priority, 0));
    *held = (priority, held.1 + 1);
}

/// Every thread entered so far, by name and class.
pub fn entered() -> Vec<Entered> {
    let entered = ENTERED.lock().unwrap_or_else(PoisonError::into_inner);
    entered.iter().map(|((name, class), &(priority, threads))| Entered { name: name.clone(), class: *class, priority, threads }).collect()
}

fn set(priority: i32) {
    #[cfg(windows)]
    {
        unsafe extern "system" {
            fn GetCurrentThread() -> *mut std::ffi::c_void;
            fn SetThreadPriority(thread: *mut std::ffi::c_void, priority: i32) -> i32;
        }
        // SAFETY: GetCurrentThread returns the calling thread's pseudo handle,
        // which needs no closing, and SetThreadPriority only reads it.
        unsafe {
            SetThreadPriority(GetCurrentThread(), priority);
        }
    }
    let _ = priority;
}

/// A worker whose product the frame takes when it is ready.
pub fn background_priority() {
    enter(Class::Worker);
}

/// The thread that draws the map: one step above normal, so it runs before every other thread of
/// the process (the stepper, the lanes' readers, the rows worker, the makers) when it is ready.
pub fn frame_priority() {
    enter(Class::Frame);
}

/// The calling thread's scheduling priority, `None` where it is not read.
pub fn current_priority() -> Option<i32> {
    #[cfg(windows)]
    {
        const THREAD_PRIORITY_ERROR_RETURN: i32 = i32::MAX;
        unsafe extern "system" {
            fn GetCurrentThread() -> *mut std::ffi::c_void;
            fn GetThreadPriority(thread: *mut std::ffi::c_void) -> i32;
        }
        // SAFETY: GetCurrentThread returns the calling thread's pseudo handle,
        // which needs no closing, and GetThreadPriority only reads it.
        let priority = unsafe { GetThreadPriority(GetCurrentThread()) };
        (priority != THREAD_PRIORITY_ERROR_RETURN).then_some(priority)
    }
    #[cfg(not(windows))]
    None
}

/// Runs `work` on the calling thread at `priority` when that is higher than
/// its own, then restores its own: a thread doing another thread's waited-on
/// work must not be starved below the waiting thread.
pub fn at_least(priority: Option<i32>, work: impl FnOnce()) {
    #[cfg(windows)]
    {
        unsafe extern "system" {
            fn GetCurrentThread() -> *mut std::ffi::c_void;
            fn SetThreadPriority(thread: *mut std::ffi::c_void, priority: i32) -> i32;
        }
        let own = current_priority();
        if let (Some(wanted), Some(own)) = (priority, own)
            && wanted > own
        {
            struct Restore(i32);
            impl Drop for Restore {
                fn drop(&mut self) {
                    // SAFETY: as below.
                    unsafe {
                        SetThreadPriority(GetCurrentThread(), self.0);
                    }
                }
            }
            // SAFETY: GetCurrentThread returns the calling thread's pseudo
            // handle, which needs no closing, and SetThreadPriority only
            // reads it.
            unsafe {
                SetThreadPriority(GetCurrentThread(), wanted);
            }
            let _restore = Restore(own);
            work();
            return;
        }
    }
    let _ = priority;
    work();
}

/// Below the hour builders and the playback decoder, for passes that only
/// refine what the window already shows.
pub fn lowest_priority() {
    enter(Class::Refining);
}

/// A worker at normal priority, even when it was started by a below-normal hour or document
/// thread; below normal with the frame first ([`Class::Normal`]).
pub fn normal_priority() {
    enter(Class::Normal);
}

#[cfg(test)]
mod tests {
    use super::Class;

    /// Every class keeps the priority its threads had before classes.
    #[test]
    fn each_class_keeps_todays_priority() {
        let today = [(Class::Frame, 1), (Class::FrameWork, 0), (Class::Worker, -1), (Class::Refining, -2), (Class::Normal, 0), (Class::Kept, 0)];
        for (class, level) in today {
            assert_eq!(class.level(), level, "{class:?}");
        }
    }
}
