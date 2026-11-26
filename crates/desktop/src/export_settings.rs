use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tauri::WebviewWindow;
use video_export::BackendChoice;

static LEASE_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendStatus {
    pub media_foundation: bool,
    pub media_foundation_detail: String,
    pub ffmpeg: bool,
    pub ffmpeg_path: Option<String>,
    pub ffmpeg_detail: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSettings {
    #[serde(default)]
    pub backend: BackendChoice,
    #[serde(default)]
    pub ffmpeg_path: Option<String>,
}

fn path() -> Result<PathBuf, String> {
    Ok(
        PathBuf::from(std::env::var_os("APPDATA").ok_or("User configuration folder unavailable")?)
            .join("BattlePlan")
            .join("export.json"),
    )
}

fn stages_dir() -> Result<PathBuf, String> {
    Ok(
        PathBuf::from(std::env::var_os("APPDATA").ok_or("User configuration folder unavailable")?)
            .join("BattlePlan")
            .join("export-stages"),
    )
}

#[derive(Deserialize, Serialize)]
struct StageRecord {
    version: u32,
    stage_path: PathBuf,
}

pub fn resolve_export_destination(raw: &Path) -> Result<PathBuf, String> {
    if raw.as_os_str().is_empty() {
        return Err("Choose a destination file before exporting.".into());
    }
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("Couldn’t resolve destination folder: {e}"))?
            .join(raw)
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err("Destination path is outside the file system root.".into());
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    let file_name = normalized
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "Choose a destination file name before exporting.".to_owned())?
        .to_owned();
    let parent = normalized
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| "Choose a destination folder before exporting.".to_owned())?;
    let mut existing = parent;
    let mut tail = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            break;
        };
        tail.push(name.to_os_string());
        let Some(up) = existing.parent() else {
            break;
        };
        existing = up;
    }
    let mut resolved = fs::canonicalize(existing).unwrap_or_else(|_| existing.to_path_buf());
    for part in tail.into_iter().rev() {
        resolved.push(part);
    }
    Ok(resolved.join(file_name))
}

pub fn is_valid_stage_path(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return false;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let Some(without_tmp) = name.strip_suffix(".tmp") else {
        return false;
    };
    let Some((before_seq, seq)) = without_tmp.rsplit_once('.') else {
        return false;
    };
    if seq.is_empty() || !seq.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let Some((base, pid)) = before_seq.rsplit_once('.') else {
        return false;
    };
    !base.is_empty() && !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit())
}

pub fn remove_stage_with_sidecars(path: &Path) -> bool {
    if !is_valid_stage_path(path) {
        return false;
    }
    let remove_one = |candidate: &Path| match fs::remove_file(candidate) {
        Ok(()) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => true,
        Err(_) => false,
    };
    let mut all_gone = remove_one(path);
    for suffix in [".faststart", ".video.mp4", ".audio.pcm"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        if !remove_one(Path::new(&sidecar)) {
            all_gone = false;
        }
    }
    all_gone
}

pub struct StageLease {
    lock: Option<File>,
    record_path: PathBuf,
    stage_path: PathBuf,
}

impl StageLease {
    pub fn acquire(stage_path: PathBuf) -> Result<Self, String> {
        let dir = stages_dir()?;
        Self::acquire_in(&dir, stage_path)
    }

    fn acquire_in(dir: &Path, stage_path: PathBuf) -> Result<Self, String> {
        if !is_valid_stage_path(&stage_path) {
            return Err("Export staging path is not an absolute BattlePlan stage file.".into());
        }
        fs::create_dir_all(dir)
            .map_err(|e| format!("Couldn’t prepare export cleanup directory: {e}"))?;
        let pid = std::process::id();
        let (mut file, record_path) = loop {
            let seq = LEASE_SEQ.fetch_add(1, Ordering::Relaxed);
            let candidate = dir.join(format!("job-{pid}-{seq}.stage"));
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(file) => break (file, candidate),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(format!("Couldn’t create export cleanup record: {error}"));
                }
            }
        };
        let setup = (|| -> io::Result<()> {
            file.try_lock().map_err(|e| match e {
                fs::TryLockError::WouldBlock => io::Error::new(io::ErrorKind::WouldBlock, "locked"),
                fs::TryLockError::Error(err) => err,
            })?;
            let payload = serde_json::to_vec(&StageRecord {
                version: 1,
                stage_path: stage_path.clone(),
            })
            .map_err(io::Error::other)?;
            file.write_all(&payload)?;
            file.sync_all()?;
            Ok(())
        })();
        if let Err(error) = setup {
            drop(file);
            let _ = fs::remove_file(&record_path);
            return Err(format!("Couldn’t persist export cleanup record: {error}"));
        }
        Ok(Self {
            lock: Some(file),
            record_path,
            stage_path,
        })
    }

    pub fn stage_path(&self) -> &Path {
        &self.stage_path
    }

    pub fn release_after_publish(mut self) {
        let _ = remove_stage_with_sidecars(&self.stage_path);
        drop(self.lock.take());
        let _ = fs::remove_file(&self.record_path);
    }

    pub fn cleanup_and_release(mut self) {
        let cleaned = remove_stage_with_sidecars(&self.stage_path);
        drop(self.lock.take());
        if cleaned {
            let _ = fs::remove_file(&self.record_path);
        }
    }

    pub fn abandon_for_unfinished_shutdown(mut self) {
        if let Some(lock) = self.lock.take() {
            std::mem::forget(lock);
        }
    }
}

fn sweep_pending_stages_in(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let record_path = entry.path();
        if record_path.extension().and_then(|e| e.to_str()) != Some("stage") {
            continue;
        }
        let Ok(mut file) = OpenOptions::new().read(true).write(true).open(&record_path) else {
            continue;
        };
        if file.try_lock().is_err() {
            continue;
        }
        let mut bytes = Vec::new();
        if file.read_to_end(&mut bytes).is_err() {
            continue;
        }
        let Ok(record) = serde_json::from_slice::<StageRecord>(&bytes) else {
            continue;
        };
        if record.version != 1 || !is_valid_stage_path(&record.stage_path) {
            continue;
        }
        let cleaned = remove_stage_with_sidecars(&record.stage_path);
        drop(file);
        if cleaned {
            let _ = fs::remove_file(&record_path);
        }
    }
}

pub fn sweep_pending_stages() {
    if let Ok(dir) = stages_dir() {
        sweep_pending_stages_in(&dir);
    }
}

pub fn load() -> ExportSettings {
    path()
        .ok()
        .and_then(|p| load_from(&p).ok())
        .unwrap_or_default()
}

fn load_from(path: &Path) -> Result<ExportSettings, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ExportSettings::default());
        }
        Err(error) => return Err(error.to_string()),
    };
    if bytes.len() > 8192 {
        return Err("Export settings exceed their size limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn save_to(path: &Path, settings: &ExportSettings) -> Result<(), String> {
    replay_settings::exit::EXIT
        .under_lock(
            &path.with_extension("lock"),
            replay_settings::exit::What::Change,
            |receipt| {
                let _ = receipt.replace(path, |out| {
                    serde_json::to_writer_pretty(out, settings).map_err(io::Error::from)
                })?;
                Ok(())
            },
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn export_settings() -> Result<ExportSettings, String> {
    load_from(&path()?)
}

#[tauri::command]
pub fn set_export_settings(mut settings: ExportSettings) -> Result<ExportSettings, String> {
    if let Some(raw) = &settings.ffmpeg_path {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            settings.ffmpeg_path = None;
        } else {
            settings.ffmpeg_path = Some(trimmed.to_owned());
        }
    }
    save_to(&path()?, &settings)?;
    Ok(settings)
}

#[tauri::command]
pub fn export_backends(ffmpeg_path: Option<String>) -> BackendStatus {
    let configured = ffmpeg_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            load()
                .ffmpeg_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        });
    let availability = video_export::probe_backends(configured.as_deref());
    let (media_foundation, media_foundation_detail) = match availability.media_foundation {
        Ok(detail) => (true, detail),
        Err(detail) => (false, detail),
    };
    let (ffmpeg, ffmpeg_path, ffmpeg_detail) = match availability.ffmpeg {
        Ok(info) => (
            true,
            Some(info.path.to_string_lossy().into_owned()),
            format!("{} ({})", info.encoder, info.version),
        ),
        Err(detail) => (false, None, detail),
    };
    BackendStatus {
        media_foundation,
        media_foundation_detail,
        ffmpeg,
        ffmpeg_path,
        ffmpeg_detail,
    }
}

#[tauri::command]
pub async fn choose_ffmpeg_binary(window: WebviewWindow) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new()
            .set_parent(&window)
            .set_title("Choose FFmpeg executable");
        #[cfg(windows)]
        {
            dialog = dialog.add_filter("Executable", &["exe"]);
        }
        dialog
            .pick_file()
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_settings_round_trips_without_touching_video_json() {
        let dir = std::env::temp_dir().join(format!(
            "battleplan-export-settings-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("export.json");
        assert_eq!(load_from(&file).unwrap(), ExportSettings::default());
        let saved = ExportSettings {
            backend: BackendChoice::Ffmpeg,
            ffmpeg_path: Some("C:/tools/ffmpeg.exe".into()),
        };
        save_to(&file, &saved).unwrap();
        assert_eq!(load_from(&file).unwrap(), saved);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stage_lease_blocks_sweep_while_held_and_sweeps_after_exit() {
        let dir = std::env::temp_dir().join(format!(
            "battleplan-export-stage-lease-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let stage_dir = dir.join("export-stages");
        let dest = resolve_export_destination(&dir.join("campaign.mp4")).unwrap();
        let stage = replay_settings::exit::stage_path(&dest).unwrap();
        assert!(is_valid_stage_path(&stage));
        assert!(!is_valid_stage_path(&dest));
        assert!(!is_valid_stage_path(Path::new("relative.mp4.1.0.tmp")));

        let faststart = PathBuf::from(format!("{}.faststart", stage.display()));
        let video = PathBuf::from(format!("{}.video.mp4", stage.display()));
        let audio = PathBuf::from(format!("{}.audio.pcm", stage.display()));
        fs::write(&stage, b"partial-mp4").unwrap();
        fs::write(&faststart, b"fs").unwrap();
        fs::write(&video, b"vid").unwrap();
        fs::write(&audio, b"aud").unwrap();

        let lease = StageLease::acquire_in(&stage_dir, stage.clone()).unwrap();
        let record_path = lease.record_path.clone();

        sweep_pending_stages_in(&stage_dir);
        assert!(stage.exists());
        assert!(record_path.exists());

        drop(lease);
        sweep_pending_stages_in(&stage_dir);
        assert!(!stage.exists());
        assert!(!faststart.exists());
        assert!(!video.exists());
        assert!(!audio.exists());
        assert!(!record_path.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stage_sweep_preserves_unremovable_corrupt_and_invalid_records() {
        let dir = std::env::temp_dir().join(format!(
            "battleplan-export-stage-sweep-guard-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let stage_dir = dir.join("export-stages");
        fs::create_dir_all(&stage_dir).unwrap();

        let corrupt = stage_dir.join("job-1-0.stage");
        fs::write(&corrupt, b"{not json").unwrap();

        let target_real = dir.join("precious.mp4");
        fs::write(&target_real, b"keep-me").unwrap();
        let invalid_rec = stage_dir.join("job-1-1.stage");
        fs::write(
            &invalid_rec,
            serde_json::to_vec(&StageRecord {
                version: 1,
                stage_path: target_real.clone(),
            })
            .unwrap(),
        )
        .unwrap();

        let dest = resolve_export_destination(&dir.join("locked.mp4")).unwrap();
        let locked_stage = replay_settings::exit::stage_path(&dest).unwrap();
        fs::create_dir_all(locked_stage.join("held")).unwrap();
        let locked_lease = StageLease::acquire_in(&stage_dir, locked_stage.clone()).unwrap();
        let locked_rec = locked_lease.record_path.clone();
        drop(locked_lease);

        sweep_pending_stages_in(&stage_dir);
        assert!(corrupt.exists());
        assert!(target_real.exists());
        assert!(invalid_rec.exists());
        assert!(locked_rec.exists());

        fs::remove_dir_all(&locked_stage).unwrap();
        sweep_pending_stages_in(&stage_dir);
        assert!(!locked_rec.exists());
        assert!(corrupt.exists());
        assert!(invalid_rec.exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
