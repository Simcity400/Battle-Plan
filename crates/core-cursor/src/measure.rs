#[cfg(windows)]
pub fn thread_cycles() -> Option<u64> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> *mut std::ffi::c_void;
        fn QueryThreadCycleTime(thread: *mut std::ffi::c_void, cycles: *mut u64) -> i32;
    }
    let mut cycles = 0;
    (unsafe { QueryThreadCycleTime(GetCurrentThread(), &raw mut cycles) } != 0).then_some(cycles)
}
#[cfg(not(windows))]
pub fn thread_cycles() -> Option<u64> {
    None
}
