use std::{
    ffi::{OsStr, OsString, c_void},
    os::windows::ffi::OsStrExt,
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS,
        GetLastError, HANDLE,
    },
    System::{
        Registry::{
            HKEY_CURRENT_USER, REG_SZ, RRF_NOEXPAND, RRF_RT_ANY, RRF_RT_REG_DWORD, RegDeleteKeyValueW,
            RegGetValueW, RegSetKeyValueW,
        },
        Threading::CreateMutexW,
    },
};

const INSTANCE_MUTEX: &str = replay_settings::recorder_settings::RECORDER_INSTANCE;
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "BattlePlan Recorder";
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

fn unread(status: u32) -> String {
    format!("Start with Windows couldn’t be read: {}", std::io::Error::from_raw_os_error(status as i32))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunValue {
    pub kind: u32,
    pub data: Vec<u8>,
}

impl RunValue {
    pub fn command(command: &OsStr) -> Self {
        Self { kind: REG_SZ, data: wide(command).iter().flat_map(|unit| unit.to_le_bytes()).collect() }
    }
}

pub fn run_value() -> Result<Option<RunValue>, String> {
    let key = wide(OsStr::new(RUN_KEY));
    let value = wide(OsStr::new(RUN_VALUE));
    loop {
        let (mut kind, mut size) = (0_u32, 0_u32);
        // SAFETY: a size query; every pointer is valid for the call.
        let status = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_ANY | RRF_NOEXPAND, &mut kind, std::ptr::null_mut(), &mut size)
        };
        match status {
            ERROR_SUCCESS => {}
            ERROR_FILE_NOT_FOUND => return Ok(None),
            status => return Err(unread(status)),
        }
        let mut data = vec![0_u8; size as usize];
        let mut read = size;
        // SAFETY: the buffer holds `read` bytes, as the call is told.
        let status = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_ANY | RRF_NOEXPAND, &mut kind, data.as_mut_ptr().cast::<c_void>(), &mut read)
        };
        match status {
            ERROR_SUCCESS => {
                data.truncate(read as usize);
                return Ok(Some(RunValue { kind, data }));
            }
            ERROR_MORE_DATA => continue,
            ERROR_FILE_NOT_FOUND => return Ok(None),
            status => return Err(unread(status)),
        }
    }
}

pub fn starts_with_windows() -> Result<bool, String> {
    run_value().map(|value| value.is_some())
}

pub fn write_run_value(value: Option<&RunValue>) -> Result<(), String> {
    let key = wide(OsStr::new(RUN_KEY));
    let status = match value {
        Some(value) => {
            let name = wide(OsStr::new(RUN_VALUE));
            // SAFETY: the data outlives the call, which reads its stated length.
            unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), value.kind, value.data.as_ptr().cast::<c_void>(), value.data.len() as u32) }
        }
        None => remove_run_value(&key, RUN_VALUE),
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

fn remove_run_value(key: &[u16], name: &str) -> u32 {
    let value = wide(OsStr::new(name));
    match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) } {
        ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
        status => status,
    }
}

pub fn set_start_with_windows(enabled: bool) -> Result<(), String> {
    if !enabled {
        return write_run_value(None);
    }
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut command = OsString::from("\"");
    command.push(executable.as_os_str());
    command.push("\"");
    write_run_value(Some(&RunValue::command(&command)))
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

pub fn held_by_a_job() -> bool {
    use windows_sys::Win32::System::{JobObjects::IsProcessInJob, Threading::GetCurrentProcess};
    let mut held = 1;
    let checked = unsafe { IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &raw mut held) };
    checked == 0 || held != 0
}
