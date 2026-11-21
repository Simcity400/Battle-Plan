use std::{ffi::OsStr, os::windows::ffi::OsStrExt, path::Path};
use windows_sys::Win32::UI::{
    Shell::ShellExecuteW,
    WindowsAndMessaging::{SW_SHOW, SW_SHOWNORMAL},
};

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

fn open(file: &OsStr, parameters: Option<&str>, show: i32) -> Result<(), String> {
    let file = wide(file);
    let parameters = parameters.map(|parameters| wide(OsStr::new(parameters)));
    let verb = wide(OsStr::new("open"));
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            parameters
                .as_ref()
                .map_or(std::ptr::null(), |parameters| parameters.as_ptr()),
            std::ptr::null(),
            show,
        )
    };
    if result as isize > 32 {
        Ok(())
    } else {
        Err(format!(
            "Windows refused to open it (code {})",
            result as isize
        ))
    }
}

pub fn launch(version: &str, package: &[u8], arguments: &str) -> Result<(), String> {
    let path = std::env::temp_dir().join(format!("battle-plan-{version}-setup.exe"));
    std::fs::write(&path, package).map_err(|error| {
        format!(
            "The installer couldn’t be saved to {}: {error}",
            path.display()
        )
    })?;
    start(&path, arguments)
}

fn start(path: &Path, arguments: &str) -> Result<(), String> {
    open(path.as_os_str(), Some(arguments), SW_SHOW)
        .map_err(|error| format!("The installer couldn’t be started: {error}."))
}

pub fn browse(url: &str) -> Result<(), String> {
    open(OsStr::new(url), None, SW_SHOWNORMAL)
        .map_err(|error| format!("The release page couldn’t be opened: {error}."))
}
