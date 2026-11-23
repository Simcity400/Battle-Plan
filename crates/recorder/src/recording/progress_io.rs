use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    os::windows::fs::OpenOptionsExt,
    path::Path,
    time::Duration,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, MOVEFILE_REPLACE_EXISTING,
    MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

pub(super) fn work(path: &Path, records: u64) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)?;
    file.write_all(&records.to_le_bytes())
}

pub(super) struct PassProgress {
    path: std::path::PathBuf,
    state: std::sync::Mutex<(u64, Option<std::time::Instant>, bool)>,
}
impl PassProgress {
    pub(super) fn new(directory: &Path) -> Self {
        Self {
            path: directory.join("finalization-pass-work"),
            state: std::sync::Mutex::new((0, None, false)),
        }
    }
    pub(super) fn completed_at(&self, now: std::time::Instant) {
        let mut state = super::locked(&self.state);
        state.0 = state.0.saturating_add(1);
        if state
            .1
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(1))
        {
            return;
        }
        state.1 = Some(now);
        if let Err(error) = work(&self.path, state.0)
            && !state.2
        {
            state.2 = true;
            eprintln!(
                "Finalization pass progress unavailable at {}: {error}",
                self.path.display()
            );
        }
    }
}

pub(super) fn passes(directory: &Path) -> replay_settings::progress::Guard {
    let progress = PassProgress::new(directory);
    replay_settings::progress::Progress::new(move || progress.completed_at(std::time::Instant::now()))
        .install()
}

fn replace_with(
    mut replace: impl FnMut() -> io::Result<()>,
    mut pause: impl FnMut(Duration),
) -> io::Result<()> {
    for delay in [10, 20, 0] {
        match replace() {
            Err(error) if delay != 0 && matches!(error.raw_os_error(), Some(5 | 32 | 33)) => {
                pause(Duration::from_millis(delay));
            }
            result => return result,
        }
    }
    unreachable!()
}

pub(super) fn publish(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let mut file = fs::File::create(&temporary).map_err(|e| super::failure_text(&temporary, e))?;
    file.write_all(&serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?)
        .and_then(|()| file.sync_all())
        .map_err(|e| super::failure_text(&temporary, e))?;
    drop(file);
    let from = super::windows::wide(temporary.as_os_str());
    let to = super::windows::wide(path.as_os_str());
    replace_with(
        || {
            if unsafe {
                MoveFileExW(
                    from.as_ptr(),
                    to.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            } == 0
            {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        },
        std::thread::sleep,
    )
    .map_err(|e| {
        format!(
            "atomic replacement of {} from {} failed: {e}; no blocking holder identified",
            path.display(),
            temporary.display(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_retries_transient_windows_errors_but_is_bounded() {
        for error in [5, 32, 33] {
            let mut attempts = 0;
            let mut pauses = Vec::new();
            replace_with(
                || {
                    attempts += 1;
                    if attempts < 3 {
                        Err(io::Error::from_raw_os_error(error))
                    } else {
                        Ok(())
                    }
                },
                |delay| pauses.push(delay),
            )
            .unwrap();
            assert_eq!(attempts, 3);
            assert_eq!(
                pauses,
                [Duration::from_millis(10), Duration::from_millis(20)]
            );
        }
        let mut attempts = 0;
        assert!(
            replace_with(
                || {
                    attempts += 1;
                    Err(io::Error::from_raw_os_error(5))
                },
                |_| {}
            )
            .is_err()
        );
        assert_eq!(attempts, 3);
        assert!(
            replace_with(
                || Err(io::ErrorKind::NotFound.into()),
                |_| panic!("not transient")
            )
            .is_err()
        );
    }
}
