use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use tauri::WebviewWindow;
use video_export::BackendChoice;

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
}
