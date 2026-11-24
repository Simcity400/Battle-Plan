
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
pub fn qpc() -> i64 {
    unsafe extern "system" {
        fn QueryPerformanceCounter(count: *mut i64) -> i32;
    }
    let mut ticks = 0;
    unsafe { QueryPerformanceCounter(&raw mut ticks) };
    ticks
}

#[cfg(not(all(windows, target_env = "msvc", target_arch = "x86_64")))]
pub fn qpc() -> i64 {
    0
}

pub fn since(started: i64) -> u64 {
    qpc().saturating_sub(started).max(0) as u64
}
