mod bake;
pub use bake::at_bake_priority;

use std::{
    collections::BTreeMap,
    sync::{Mutex, PoisonError},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    Playback,
    Frame,
    Worker,
    Refining,
    Normal,
    Kept,
}

impl Class {
    pub fn name(self) -> &'static str {
        match self {
            Self::Playback => "playback",
            Self::Frame => "frame",
            Self::Worker => "worker",
            Self::Refining => "refining",
            Self::Normal => "normal",
            Self::Kept => "kept",
        }
    }

    fn level(self) -> i32 {
        const LOWEST: i32 = -2;
        const BELOW_NORMAL: i32 = -1;
        const NORMAL: i32 = 0;
        const ABOVE_NORMAL: i32 = 1;
        const TIME_CRITICAL: i32 = 15;
        match self {
            Self::Playback => TIME_CRITICAL,
            Self::Frame => ABOVE_NORMAL,
            Self::Kept | Self::Normal => NORMAL,
            Self::Worker => BELOW_NORMAL,
            Self::Refining => LOWEST,
        }
    }
}

static ENTERED: Mutex<BTreeMap<(String, Class), (Option<i32>, u64)>> = Mutex::new(BTreeMap::new());

#[derive(Clone, Debug)]
pub struct Entered {
    pub name: String,
    pub class: Class,
    pub priority: Option<i32>,
    pub threads: u64,
}

pub fn enter(class: Class) {
    set(class.level());
    let name = std::thread::current().name().unwrap_or("unnamed").to_owned();
    let priority = current_priority();
    let mut entered = ENTERED.lock().unwrap_or_else(PoisonError::into_inner);
    let held = entered.entry((name, class)).or_insert((priority, 0));
    *held = (priority, held.1 + 1);
}

pub fn entered() -> Vec<Entered> {
    let entered = ENTERED.lock().unwrap_or_else(PoisonError::into_inner);
    entered.iter().map(|((name, class), &(priority, threads))| Entered { name: name.clone(), class: *class, priority, threads }).collect()
}

fn set(priority: i32) {
    if cfg!(feature = "test-support") {
        return;
    }
    #[cfg(windows)]
    {
        unsafe extern "system" {
            fn GetCurrentThread() -> *mut std::ffi::c_void;
            fn SetThreadPriority(thread: *mut std::ffi::c_void, priority: i32) -> i32;
        }
        unsafe {
            SetThreadPriority(GetCurrentThread(), priority);
        }
    }
    let _ = priority;
}

pub fn background_priority() {
    enter(Class::Worker);
}

pub fn frame_priority() {
    enter(Class::Frame);
}

pub fn current_priority() -> Option<i32> {
    #[cfg(windows)]
    {
        const THREAD_PRIORITY_ERROR_RETURN: i32 = i32::MAX;
        unsafe extern "system" {
            fn GetCurrentThread() -> *mut std::ffi::c_void;
            fn GetThreadPriority(thread: *mut std::ffi::c_void) -> i32;
        }
        let priority = unsafe { GetThreadPriority(GetCurrentThread()) };
        (priority != THREAD_PRIORITY_ERROR_RETURN).then_some(priority)
    }
    #[cfg(not(windows))]
    None
}

pub fn waking(wake: impl Fn() + Send + Sync + 'static) -> impl Fn() + Send + Sync + 'static {
    move || at_least(Some(Class::Kept.level()), || wake())
}

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
                    unsafe {
                        SetThreadPriority(GetCurrentThread(), self.0);
                    }
                }
            }
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

pub fn lowest_priority() {
    enter(Class::Refining);
}

pub fn normal_priority() {
    enter(Class::Normal);
}

#[cfg(test)]
mod tests {
    use super::Class;

    #[test]
    fn each_class_keeps_todays_priority() {
        let today = [(Class::Frame, 1), (Class::Worker, -1), (Class::Refining, -2), (Class::Normal, 0), (Class::Kept, 0)];
        for (class, level) in today {
            assert_eq!(class.level(), level, "{class:?}");
        }
    }
}
