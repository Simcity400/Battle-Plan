//! Live OS pressure and independent commit/physical headroom; viewer budgets are acceptance targets.

#[cfg(windows)]
pub(super) fn available() -> Option<(u64, u64)> {
    // MEMORYSTATUSEX layout; available commit already includes process job limits.
    #[repr(C)]
    #[derive(Default)]
    struct Status {
        length: u32,
        load: u32,
        total_physical: u64,
        available_physical: u64,
        total_commit: u64,
        available_commit: u64,
        total_virtual: u64,
        available_virtual: u64,
        available_extended_virtual: u64,
    }
    const _: () = assert!(std::mem::size_of::<Status>() == 64);
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(status: *mut Status) -> i32;
    }
    let mut status = Status {
        length: std::mem::size_of::<Status>() as u32,
        ..Default::default()
    };
    // SAFETY: the C-layout buffer is writable and its length names its complete size.
    let read = unsafe { GlobalMemoryStatusEx(&mut status) };
    (read != 0).then_some((status.available_commit, status.available_physical))
}

#[cfg(not(windows))]
pub(super) fn available() -> Option<(u64, u64)> {
    None
}

#[cfg(windows)]
pub(super) fn system_low() -> bool {
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::HANDLE;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateMemoryResourceNotification(kind: i32) -> HANDLE;
        fn QueryMemoryResourceNotification(notification: HANDLE, state: *mut i32) -> i32;
    }
    const LOW_MEMORY: i32 = 0;
    // The notification handle intentionally lives until process exit.
    static NOTIFICATION: OnceLock<usize> = OnceLock::new();
    let notification = *NOTIFICATION.get_or_init(|| {
        // SAFETY: LOW_MEMORY is the documented notification kind; failure returns null.
        unsafe { CreateMemoryResourceNotification(LOW_MEMORY) as usize }
    });
    if notification == 0 {
        return false;
    }
    let mut low = 0;
    // SAFETY: the handle remains open and low is writable BOOL storage.
    let read = unsafe { QueryMemoryResourceNotification(notification as HANDLE, &mut low) };
    read != 0 && low != 0
}

#[cfg(not(windows))]
pub(super) fn system_low() -> bool {
    false
}
