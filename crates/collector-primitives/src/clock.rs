
use windows_sys::Win32::System::Performance::QueryPerformanceCounter;

pub fn qpc() -> i64 {
    let mut ticks = 0;
    unsafe {
        QueryPerformanceCounter(&mut ticks);
    }
    ticks
}
