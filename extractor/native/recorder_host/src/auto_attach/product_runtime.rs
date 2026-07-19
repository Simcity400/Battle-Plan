//! End-user packaging and graceful-stop seams for the resident auto-attach app.
//!
//! This module deliberately does not enable duplex capture. It removes two
//! development-only assumptions from the supervisor entry point: the packaged
//! collector payload can be resolved beside the installed executable, and an OS
//! stop request reaches the coordinator's stop ownership instead of bypassing
//! it. Worker-side finalization remains governed by the activation-frozen duplex
//! lifecycle rather than being claimed by this signal bridge.

use std::fs;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};

pub const PACKAGED_COLLECTOR_DLL_NAME: &str = "collector.dll";
pub const PRODUCT_DATA_DIRECTORY_NAME: &str = "Battle Plan Recorder";

/// Resolve the collector payload for the installed product.
///
/// An explicit path remains a developer/support override. The ordinary product
/// path is exactly one non-symlink file named `collector.dll` beside the current
/// executable. Canonical-parent equality prevents a junction/symlink escape from
/// silently turning the install directory into an arbitrary payload search.
pub fn resolve_packaged_collector_dll(
    explicit: Option<PathBuf>,
    current_exe: &Path,
) -> Result<PathBuf, String> {
    if let Some(explicit) = explicit {
        return validate_payload_file(&explicit, None, "explicit collector DLL override");
    }

    let executable = fs::canonicalize(current_exe)
        .map_err(|error| format!("canonicalize installed recorder executable failed: {error}"))?;
    let install_dir = executable
        .parent()
        .ok_or("installed recorder executable has no parent directory")?;
    let candidate = install_dir.join(PACKAGED_COLLECTOR_DLL_NAME);
    validate_payload_file(&candidate, Some(install_dir), "packaged collector DLL")
}

fn validate_payload_file(
    candidate: &Path,
    required_parent: Option<&Path>,
    label: &str,
) -> Result<PathBuf, String> {
    let link_metadata = fs::symlink_metadata(candidate)
        .map_err(|error| format!("stat {label} {} failed: {error}", candidate.display()))?;
    if link_metadata.file_type().is_symlink() || !link_metadata.is_file() {
        return Err(format!(
            "{label} {} is not a regular non-symlink file",
            candidate.display()
        ));
    }
    if link_metadata.len() == 0 {
        return Err(format!("{label} {} is empty", candidate.display()));
    }
    let canonical = fs::canonicalize(candidate).map_err(|error| {
        format!(
            "canonicalize {label} {} failed: {error}",
            candidate.display()
        )
    })?;
    if let Some(required_parent) = required_parent {
        if canonical.parent() != Some(required_parent) {
            return Err(format!(
                "{label} resolved outside the installed recorder directory"
            ));
        }
    }
    Ok(canonical)
}

/// Resolve the ordinary per-user product data root through the Windows Known
/// Folder API. No drive letter, profile path, OneDrive policy, or environment
/// variable participates in the decision.
#[cfg(windows)]
pub fn default_product_data_root() -> Result<PathBuf, String> {
    use std::ffi::{c_void, OsString};
    use std::os::windows::ffi::OsStringExt;
    use std::ptr;

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    const FOLDERID_LOCAL_APP_DATA: Guid = Guid {
        data1: 0xF1B3_2785,
        data2: 0x6FBA,
        data3: 0x4FCF,
        data4: [0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91],
    };

    #[link(name = "shell32")]
    extern "system" {
        fn SHGetKnownFolderPath(
            folder_id: *const Guid,
            flags: u32,
            token: *mut c_void,
            path: *mut *mut u16,
        ) -> i32;
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoTaskMemFree(memory: *mut c_void);
    }

    unsafe {
        let mut raw: *mut u16 = ptr::null_mut();
        let result = SHGetKnownFolderPath(&FOLDERID_LOCAL_APP_DATA, 0, ptr::null_mut(), &mut raw);
        if result < 0 || raw.is_null() {
            return Err(format!(
                "resolve LocalAppData Known Folder failed: HRESULT 0x{:08x}",
                result as u32
            ));
        }
        let mut len = 0usize;
        while *raw.add(len) != 0 {
            len = len
                .checked_add(1)
                .ok_or("LocalAppData Known Folder path length overflow")?;
            if len > 32_767 {
                CoTaskMemFree(raw.cast());
                return Err("LocalAppData Known Folder path exceeds Windows limit".into());
            }
        }
        let root = PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(raw, len)));
        CoTaskMemFree(raw.cast());
        if root.as_os_str().is_empty() || !root.is_absolute() {
            return Err("LocalAppData Known Folder returned a non-absolute path".into());
        }
        Ok(root.join(PRODUCT_DATA_DIRECTORY_NAME))
    }
}

#[cfg(windows)]
mod stop {
    use super::*;

    type HandlerRoutine = Option<unsafe extern "system" fn(u32) -> i32>;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetConsoleCtrlHandler(handler: HandlerRoutine, add: i32) -> i32;
    }

    const CTRL_C_EVENT: u32 = 0;
    const CTRL_BREAK_EVENT: u32 = 1;
    const CTRL_CLOSE_EVENT: u32 = 2;
    const CTRL_LOGOFF_EVENT: u32 = 5;
    const CTRL_SHUTDOWN_EVENT: u32 = 6;

    static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
    static HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);

    unsafe extern "system" fn handler(event: u32) -> i32 {
        if matches!(
            event,
            CTRL_C_EVENT
                | CTRL_BREAK_EVENT
                | CTRL_CLOSE_EVENT
                | CTRL_LOGOFF_EVENT
                | CTRL_SHUTDOWN_EVENT
        ) {
            // A console handler must do no allocation, I/O, locking, joining, or
            // finalization. It only transfers ownership to the ordinary
            // supervisor loop, which requests stop from its owned workers.
            STOP_REQUESTED.store(true, Ordering::Release);
            1
        } else {
            0
        }
    }

    /// RAII registration for the resident supervisor's cooperative stop flag.
    pub struct GracefulStopRegistration {
        _private: (),
    }

    impl GracefulStopRegistration {
        pub fn install() -> Result<Self, String> {
            if HANDLER_INSTALLED.swap(true, Ordering::AcqRel) {
                return Err("auto-attach graceful-stop handler is already installed".into());
            }
            STOP_REQUESTED.store(false, Ordering::Release);
            if unsafe { SetConsoleCtrlHandler(Some(handler), 1) } == 0 {
                HANDLER_INSTALLED.store(false, Ordering::Release);
                return Err(format!(
                    "install auto-attach graceful-stop handler failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(Self { _private: () })
        }

        pub fn is_requested(&self) -> bool {
            STOP_REQUESTED.load(Ordering::Acquire)
        }
    }

    impl Drop for GracefulStopRegistration {
        fn drop(&mut self) {
            unsafe {
                SetConsoleCtrlHandler(Some(handler), 0);
            }
            HANDLER_INSTALLED.store(false, Ordering::Release);
        }
    }
}

#[cfg(windows)]
pub use stop::GracefulStopRegistration;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "hoi4_auto_attach_package_{tag}_{}_{}",
                std::process::id(),
                nonce
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_nonempty(path: &Path) {
        let mut file = fs::File::create(path).unwrap();
        file.write_all(b"fixture").unwrap();
        file.sync_all().unwrap();
    }

    #[test]
    fn ordinary_product_resolves_only_the_sibling_packaged_payload() {
        let dir = TempDir::new("sibling");
        let exe = dir.0.join("collector_record.exe");
        let dll = dir.0.join(PACKAGED_COLLECTOR_DLL_NAME);
        write_nonempty(&exe);
        write_nonempty(&dll);
        assert_eq!(
            resolve_packaged_collector_dll(None, &exe).unwrap(),
            fs::canonicalize(dll).unwrap()
        );
    }

    #[test]
    fn missing_or_empty_packaged_payload_fails_closed() {
        let dir = TempDir::new("missing_empty");
        let exe = dir.0.join("collector_record.exe");
        write_nonempty(&exe);
        assert!(resolve_packaged_collector_dll(None, &exe).is_err());
        fs::File::create(dir.0.join(PACKAGED_COLLECTOR_DLL_NAME)).unwrap();
        assert!(resolve_packaged_collector_dll(None, &exe).is_err());
    }

    #[test]
    fn explicit_support_override_is_still_validated_as_a_nonempty_file() {
        let dir = TempDir::new("override");
        let exe = dir.0.join("collector_record.exe");
        let override_dll = dir.0.join("support-build.dll");
        write_nonempty(&exe);
        write_nonempty(&override_dll);
        assert_eq!(
            resolve_packaged_collector_dll(Some(override_dll.clone()), &exe).unwrap(),
            fs::canonicalize(override_dll).unwrap()
        );
        assert!(resolve_packaged_collector_dll(Some(dir.0.clone()), &exe).is_err());
    }
}
