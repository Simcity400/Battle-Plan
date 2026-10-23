mod bake;
pub use bake::at_bake_priority;

pub fn background_priority() {
    #[cfg(windows)]
    {
        const THREAD_PRIORITY_BELOW_NORMAL: i32 = -1;
        unsafe extern "system" {
            fn GetCurrentThread() -> *mut std::ffi::c_void;
            fn SetThreadPriority(thread: *mut std::ffi::c_void, priority: i32) -> i32;
        }
        // SAFETY: GetCurrentThread returns the calling thread's pseudo handle,
        // which needs no closing, and SetThreadPriority only reads it.
        unsafe {
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
        }
    }
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
    #[cfg(windows)]
    {
        const THREAD_PRIORITY_LOWEST: i32 = -2;
        unsafe extern "system" {
            fn GetCurrentThread() -> *mut std::ffi::c_void;
            fn SetThreadPriority(thread: *mut std::ffi::c_void, priority: i32) -> i32;
        }
        // SAFETY: as in `background_priority`.
        unsafe {
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST);
        }
    }
}

/// Protected synchronous frame helpers start at normal priority, even when
/// their service was created by a below-normal hour or document thread.
pub fn normal_priority() {
    #[cfg(windows)]
    {
        unsafe extern "system" {
            fn GetCurrentThread() -> *mut std::ffi::c_void;
            fn SetThreadPriority(thread: *mut std::ffi::c_void, priority: i32) -> i32;
        }
        // SAFETY: the calling thread's pseudo handle needs no closing.
        unsafe {
            SetThreadPriority(GetCurrentThread(), 0);
        }
    }
}
