//! Desktop-only process discovery and background CPU/I/O scheduling.
use std::io;

pub(super) fn game_running() -> io::Result<bool> {
    process_running("hoi4.exe")
}

fn process_running(name: &str) -> io::Result<bool> {
    use std::{mem::size_of, os::windows::ffi::OsStringExt};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_NO_MORE_FILES, GetLastError, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::*,
    };
    struct Snapshot(windows_sys::Win32::Foundation::HANDLE);
    impl Drop for Snapshot {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let snapshot = Snapshot(raw);
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut found = unsafe { Process32FirstW(snapshot.0, &mut entry) };
    while found != 0 {
        let length = entry
            .szExeFile
            .iter()
            .position(|&v| v == 0)
            .unwrap_or(entry.szExeFile.len());
        if std::ffi::OsString::from_wide(&entry.szExeFile[..length])
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
        {
            return Ok(true);
        }
        found = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
    if unsafe { GetLastError() } != ERROR_NO_MORE_FILES {
        return Err(io::Error::last_os_error());
    }
    Ok(false)
}

pub(super) struct BackgroundPriority;
impl BackgroundPriority {
    pub(super) fn enter() -> io::Result<Self> {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
        };
        if unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self)
    }
}
impl Drop for BackgroundPriority {
    fn drop(&mut self) {
        unsafe {
            use windows_sys::Win32::System::Threading::{
                GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_END,
            };
            SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_END);
        }
    }
}

/// Atomic replacement for disposable app data, on the same volume, while other
/// instances may be reading it.
pub(super) fn replace_store(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> io::Result<()> {
    replay::publication::replace_file(source, destination)
}
