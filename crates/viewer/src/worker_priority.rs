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
