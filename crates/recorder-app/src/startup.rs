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
            HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW,
            RegGetValueW, RegSetKeyValueW,
        },
        Threading::CreateMutexW,
    },
};

const INSTANCE_MUTEX: &str = "Local\\HOI4Replay.Recorder";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "BattlePlan Recorder";
const LEGACY_RUN_VALUE: &str = "the game Recorder";
const PERSONALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

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

pub fn single_instance() -> Option<Instance> {
    let name = wide(OsStr::new(INSTANCE_MUTEX));
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        eprintln!(
            "BattlePlan Recorder could not check for another instance: {}",
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

fn run_value_set(name: &str) -> Result<bool, String> {
    let key = wide(OsStr::new(RUN_KEY));
    let value = wide(OsStr::new(name));
    let mut size = 0_u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    match status {
        ERROR_SUCCESS => Ok(true),
        ERROR_FILE_NOT_FOUND => Ok(false),
        status => Err(format!(
            "Start with Windows couldn’t be read: {}",
            std::io::Error::from_raw_os_error(status as i32)
        )),
    }
}

pub fn starts_with_windows() -> Result<bool, String> {
    Ok(run_value_set(RUN_VALUE)? || run_value_set(LEGACY_RUN_VALUE)?)
}

fn remove_run_value(key: &[u16], name: &str) -> u32 {
    let value = wide(OsStr::new(name));
    match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) } {
        ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
        status => status,
    }
}

pub fn set_start_with_windows(enabled: bool) -> Result<(), String> {
    let key = wide(OsStr::new(RUN_KEY));
    let status = if enabled {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let mut command = OsString::from("\"");
        command.push(executable.as_os_str());
        command.push("\"");
        let data = wide(&command);
        let value = wide(OsStr::new(RUN_VALUE));
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
        remove_run_value(&key, RUN_VALUE)
    };
    let status = match status {
        ERROR_SUCCESS => remove_run_value(&key, LEGACY_RUN_VALUE),
        failed => failed,
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!(
            "Start with Windows couldn’t be changed: {}",
            std::io::Error::from_raw_os_error(status as i32)
        ))
    }
}

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
