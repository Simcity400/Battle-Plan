use crate::windows;
use std::{
    ffi::{OsStr, OsString},
    io,
    path::PathBuf,
};
use windows_sys::Win32::System::{
    Environment::ExpandEnvironmentStringsW,
    Registry::{
        HKEY_CURRENT_USER, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW,
    },
};

pub fn default_capture_directory() -> PathBuf {
    replay_settings::recorder_settings::default_save_folder()
}

pub fn resolve_user_directory() -> Result<PathBuf, String> {
    use std::os::windows::ffi::OsStringExt;
    let key = windows::wide(OsStr::new(
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders",
    ));
    let value = windows::wide(OsStr::new("Personal"));
    let mut buffer = vec![0_u16; 32768];
    let mut bytes = (buffer.len() * 2) as u32;
    if unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &raw mut bytes,
        )
    } == 0
    {
        let mut expanded = vec![0_u16; 32768];
        let len = unsafe {
            ExpandEnvironmentStringsW(
                buffer.as_ptr(),
                expanded.as_mut_ptr(),
                expanded.len() as u32,
            )
        };
        if len > 0 && (len as usize) <= expanded.len() {
            let path = PathBuf::from(OsString::from_wide(&expanded[..len as usize - 1]))
                .join("Paradox Interactive")
                .join("Hearts of Iron IV");
            if path.is_dir() {
                return Ok(path);
            }
        }
    }
    for variable in ["OneDrive", "USERPROFILE"] {
        if let Some(root) = std::env::var_os(variable) {
            let path = PathBuf::from(root)
                .join("Documents")
                .join("Paradox Interactive")
                .join("Hearts of Iron IV");
            if path.is_dir() {
                return Ok(path);
            }
        }
    }
    Err("Paradox user directory was not found in the registry or Documents; pass --user-dir".into())
}

pub fn default_game_directory() -> io::Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    for key in [
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Steam App 394360",
        r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Steam App 394360",
    ] {
        let key = crate::windows::wide(OsStr::new(key));
        let name = crate::windows::wide(OsStr::new("InstallLocation"));
        let mut buffer = vec![0_u16; 32768];
        let mut bytes = (buffer.len() * 2) as u32;
        let result = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &raw mut bytes,
            )
        };
        if result == 0 {
            let length = buffer
                .iter()
                .position(|&ch| ch == 0)
                .unwrap_or(buffer.len());
            let path = PathBuf::from(OsString::from_wide(&buffer[..length]));
            if path.join("hoi4.exe").is_file() {
                return Ok(path);
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "Steam the game installation was not found in the registry; pass --game-dir",
    ))
}
