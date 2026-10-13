//! One recorder per user session, the optional start with Windows, and the
//! taskbar's light or dark appearance for drawing the tray icon.
use std::{
    ffi::{OsStr, OsString, c_void},
    os::windows::ffi::OsStrExt,
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, GetLastError,
        HANDLE,
    },
    System::{
        Registry::{
            HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD, RegDeleteKeyValueW, RegGetValueW,
            RegSetKeyValueW,
        },
        Threading::CreateMutexW,
    },
};

const INSTANCE_MUTEX: &str = "Local\\HOI4Replay.Recorder";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "the game Recorder";
const PERSONALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

/// Held for the process lifetime. The named mutex disappears with the process.
pub struct Instance(HANDLE);

impl Drop for Instance {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

/// `None` when another recorder already runs for this user session.
pub fn single_instance() -> Option<Instance> {
    let name = wide(OsStr::new(INSTANCE_MUTEX));
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        // Without the mutex a duplicate cannot be ruled out; recording still matters more.
        eprintln!(
            "Battle Plan Recorder could not check for another instance: {}",
            std::io::Error::last_os_error()
        );
        return Some(Instance(handle));
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            CloseHandle(handle);
        }
        return None;
    }
    Some(Instance(handle))
}

/// Write or remove this executable's entry in the user's Run key.
pub fn set_start_with_windows(enabled: bool) -> Result<(), String> {
    let key = wide(OsStr::new(RUN_KEY));
    let value = wide(OsStr::new(RUN_VALUE));
    let status = if enabled {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let mut command = OsString::from("\"");
        command.push(executable.as_os_str());
        command.push("\"");
        let data = wide(&command);
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                REG_SZ,
                data.as_ptr().cast::<c_void>(),
                (data.len() * 2) as u32,
            )
        }
    } else {
        match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) } {
            ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
            status => status,
        }
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!(
            "Start with Windows could not be changed: {}",
            std::io::Error::from_raw_os_error(status as i32)
        ))
    }
}

/// Whether the taskbar uses the light theme; dark when unknown.
pub fn light_taskbar() -> bool {
    let key = wide(OsStr::new(PERSONALIZE_KEY));
    let value = wide(OsStr::new("SystemUsesLightTheme"));
    let mut data = 0_u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut data as *mut u32).cast::<c_void>(),
            &mut size,
        )
    };
    status == ERROR_SUCCESS && data == 1
}
