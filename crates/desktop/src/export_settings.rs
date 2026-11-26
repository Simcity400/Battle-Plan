use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::WebviewWindow;
use video_export::BackendChoice;

static PENDING_STAGES_LOCK: Mutex<()> = Mutex::new(());

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

fn pending_stages_path() -> Result<PathBuf, String> {
    Ok(
        PathBuf::from(std::env::var_os("APPDATA").ok_or("User configuration folder unavailable")?)
            .join("BattlePlan")
            .join("export-pending-cleanup.json"),
    )
}

pub fn remove_stage_with_sidecars(path: &Path) -> bool {
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

fn read_pending_manifest(manifest: &Path) -> Vec<PathBuf> {
    let Ok(bytes) = fs::read(manifest) else {
        return Vec::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

fn write_pending_manifest(manifest: &Path, entries: &[PathBuf]) {
    if entries.is_empty() {
        let _ = fs::remove_file(manifest);
        return;
    }
    if let Some(parent) = manifest.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(bytes) = serde_json::to_vec_pretty(entries) else {
        return;
    };
    let tmp = manifest.with_extension("json.tmp");
    if fs::write(&tmp, &bytes).is_ok() {
        if fs::rename(&tmp, manifest).is_err() {
            let _ = fs::write(manifest, &bytes);
            let _ = fs::remove_file(&tmp);
        }
    } else {
        let _ = fs::write(manifest, &bytes);
    }
}

fn record_pending_stage_in(manifest: &Path, stage: &Path) {
    let _guard = PENDING_STAGES_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut entries = read_pending_manifest(manifest);
    if !entries.iter().any(|existing| existing == stage) {
        entries.push(stage.to_path_buf());
        write_pending_manifest(manifest, &entries);
    }
}

fn clear_pending_stage_in(manifest: &Path, stage: &Path) {
    let _guard = PENDING_STAGES_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut entries = read_pending_manifest(manifest);
    let before = entries.len();
    entries.retain(|existing| existing != stage);
    if entries.len() != before {
        write_pending_manifest(manifest, &entries);
    }
}

fn sweep_pending_stages_in(manifest: &Path) {
    let _guard = PENDING_STAGES_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let entries = read_pending_manifest(manifest);
    if entries.is_empty() {
        return;
    }
    let remaining: Vec<PathBuf> = entries
        .into_iter()
        .filter(|stage| !remove_stage_with_sidecars(stage))
        .collect();
    write_pending_manifest(manifest, &remaining);
}

pub fn record_pending_stage(stage: &Path) {
    if let Ok(manifest) = pending_stages_path() {
        record_pending_stage_in(&manifest, stage);
    }
}

pub fn clear_pending_stage(stage: &Path) {
    if let Ok(manifest) = pending_stages_path() {
        clear_pending_stage_in(&manifest, stage);
    }
}

pub fn sweep_pending_stages() {
    if let Ok(manifest) = pending_stages_path() {
        sweep_pending_stages_in(&manifest);
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
    fn pending_stage_sweep_cleans_stage_and_sidecars_and_retains_unremovable_entries() {
        let dir = std::env::temp_dir().join(format!(
            "battleplan-export-pending-stage-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let manifest = dir.join("export-pending-cleanup.json");
        let removable = dir.join(".battleplan-stage-1.mp4");
        let removable_faststart = dir.join(".battleplan-stage-1.mp4.faststart");
        let removable_video = dir.join(".battleplan-stage-1.mp4.video.mp4");
        let removable_audio = dir.join(".battleplan-stage-1.mp4.audio.pcm");
        fs::write(&removable, b"stage").unwrap();
        fs::write(&removable_faststart, b"fs").unwrap();
        fs::write(&removable_video, b"vid").unwrap();
        fs::write(&removable_audio, b"aud").unwrap();

        let locked = dir.join(".battleplan-stage-2.mp4");
        fs::create_dir_all(locked.join("held")).unwrap();

        record_pending_stage_in(&manifest, &removable);
        record_pending_stage_in(&manifest, &locked);
        sweep_pending_stages_in(&manifest);

        assert!(!removable.exists());
        assert!(!removable_faststart.exists());
        assert!(!removable_video.exists());
        assert!(!removable_audio.exists());
        assert_eq!(read_pending_manifest(&manifest), vec![locked.clone()]);

        fs::remove_dir_all(&locked).unwrap();
        sweep_pending_stages_in(&manifest);
        assert!(!manifest.exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
