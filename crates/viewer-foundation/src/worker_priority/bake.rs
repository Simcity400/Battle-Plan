use windows_sys::Win32::System::Threading::THREAD_PRIORITY_BELOW_NORMAL;

pub fn at_bake_priority<T>(work: impl FnOnce() -> T) -> T {
    with_priority(super::current_priority, set_priority, work)
}

fn with_priority<T>(
    mut read: impl FnMut() -> Option<i32>,
    mut set: impl FnMut(i32) -> bool,
    work: impl FnOnce() -> T,
) -> T {
    struct Restore<'a, F: FnMut(i32) -> bool> {
        before: i32,
        set: &'a mut F,
    }

    impl<F: FnMut(i32) -> bool> Drop for Restore<'_, F> {
        fn drop(&mut self) {
            (self.set)(self.before);
        }
    }

    let wanted = THREAD_PRIORITY_BELOW_NORMAL;
    let restore = match read() {
        Some(before) if before > wanted && set(wanted) => Some(Restore {
            before,
            set: &mut set,
        }),
        _ => None,
    };
    let _restore = if restore.is_some() && read() != Some(wanted) {
        drop(restore);
        None
    } else {
        restore
    };
    work()
}

fn set_priority(priority: i32) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority};
        unsafe { SetThreadPriority(GetCurrentThread(), priority) != 0 }
    }
    #[cfg(not(windows))]
    {
        let _ = priority;
        false
    }
}

#[cfg(test)]
mod tests;
