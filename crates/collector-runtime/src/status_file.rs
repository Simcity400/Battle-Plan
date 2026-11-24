use std::{
    fs::{self, File},
    io,
    os::windows::{ffi::OsStrExt, io::FromRawHandle},
    path::Path,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, DELETE, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    OPEN_EXISTING,
};

pub(crate) fn write_status(path: &Path, status: &impl serde::Serialize) -> io::Result<()> {
    write_status_bytes(path, &collector_status::status::serialize(status)?)
}
pub(crate) fn write_status_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = path.with_extension("json.tmp");
    retry_transient(|| fs::write(&temp, bytes))?;
    replace_status(&temp, path)
}

const STATUS_RENAME_RETRY_BUDGET: Duration = Duration::from_millis(250);

fn retry_transient(mut op: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    let started = Instant::now();
    loop {
        let error = match op() {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        let Some(delay) = status_rename_retry_delay(&error, started.elapsed()) else {
            return Err(error);
        };
        std::thread::sleep(delay);
        if started.elapsed() >= STATUS_RENAME_RETRY_BUDGET {
            return Err(error);
        }
    }
}

fn status_rename_retry_delay(error: &io::Error, elapsed: Duration) -> Option<Duration> {
    if !matches!(error.raw_os_error(), Some(5 | 32)) {
        return None;
    }
    STATUS_RENAME_RETRY_BUDGET
        .checked_sub(elapsed)
        .filter(|remaining| !remaining.is_zero())
        .map(|remaining| remaining.min(Duration::from_millis(1)))
}

fn posix_replace(source: &[u16], target: &[u16]) -> io::Result<bool> {
    let handle = unsafe {
        CreateFileW(
            source.as_ptr(),
            DELETE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_handle(handle) };
    replay_settings::publication::posix_replace(&file, target)
}

static POSIX_RENAME_UNSUPPORTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn replace_once(source: &[u16], target: &[u16]) -> io::Result<()> {
    if !POSIX_RENAME_UNSUPPORTED.load(Ordering::Relaxed) {
        match posix_replace(source, target) {
            Ok(true) => return Ok(()),
            Ok(false) => POSIX_RENAME_UNSUPPORTED.store(true, Ordering::Relaxed),
            Err(error) => return Err(error),
        }
    }
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } != 0
    {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn replace_status(temp: &Path, path: &Path) -> io::Result<()> {
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let source = wide(temp);
    let target = wide(path);
    retry_transient(|| replace_once(&source, &target))
}

#[cfg(test)]
#[test]
fn status_rename_retries_only_transient_errors_before_deadline() {
    for code in [5, 32] {
        let error = io::Error::from_raw_os_error(code);
        assert_eq!(
            status_rename_retry_delay(&error, Duration::ZERO),
            Some(Duration::from_millis(1))
        );
        assert_eq!(
            status_rename_retry_delay(
                &error,
                STATUS_RENAME_RETRY_BUDGET - Duration::from_micros(500)
            ),
            Some(Duration::from_micros(500))
        );
        assert_eq!(
            status_rename_retry_delay(&error, STATUS_RENAME_RETRY_BUDGET),
            None
        );
        assert_eq!(
            status_rename_retry_delay(
                &error,
                STATUS_RENAME_RETRY_BUDGET + Duration::from_millis(1)
            ),
            None
        );
    }
    for code in [2, 3, 87, 112] {
        assert_eq!(
            status_rename_retry_delay(&io::Error::from_raw_os_error(code), Duration::ZERO),
            None
        );
    }
    assert_eq!(
        status_rename_retry_delay(
            &io::Error::from(io::ErrorKind::PermissionDenied),
            Duration::ZERO
        ),
        None
    );
}

#[cfg(test)]
pub(crate) fn posix_rename_supported_for_test() -> bool {
    !POSIX_RENAME_UNSUPPORTED.load(Ordering::Relaxed)
}
