use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::WebviewWindow;

#[derive(Serialize)]
pub struct ReplayFile {
    path: String,
    name: String,
    folder: String,
    partial: Option<dashboard::PartialRecording>,
}
#[derive(Serialize)]
pub struct ReplayLibrary {
    folder: Option<String>,
    files: Vec<ReplayFile>,
}
#[derive(Serialize, Deserialize)]
struct Settings {
    folder: String,
}

fn settings_path() -> Result<PathBuf, String> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or("User configuration folder is unavailable")?;
    Ok(base.join("battle-plan").join("replay-folder.json"))
}

fn scan(folder: &Path) -> Result<ReplayLibrary, String> {
    let root =
        std::fs::canonicalize(folder).map_err(|e| format!("Cannot read replay folder: {e}"))?;
    let mut pending = vec![root.clone()];
    let mut files = Vec::new();
    let mut visited = 0;
    while let Some(directory) = pending.pop() {
        for entry in
            std::fs::read_dir(&directory).map_err(|e| format!("Cannot read replay folder: {e}"))?
        {
            let entry = entry.map_err(|e| e.to_string())?;
            visited += 1;
            if visited > 50_000 {
                return Err(
                    "This folder is too large. Choose a folder containing only your replays."
                        .into(),
                );
            }
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() || entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("battleplan") || ext.eq_ignore_ascii_case("hoi4replay"))
            {
                files.push(ReplayFile {
                    partial: dashboard::metadata::read_partial_recording(&path)
                        .ok()
                        .flatten(),
                    path: path.to_string_lossy().into_owned(),
                    name: path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    folder: directory
                        .strip_prefix(&root)
                        .map_err(|e| e.to_string())?
                        .to_string_lossy()
                        .replace('\\', "/"),
                });
            }
        }
    }
    files.sort_by_cached_key(|file| format!("{}/{}", file.folder, file.name).to_lowercase());
    Ok(ReplayLibrary {
        folder: Some(root.to_string_lossy().into_owned()),
        files,
    })
}

#[tauri::command]
pub async fn replay_library() -> Result<ReplayLibrary, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let config = settings_path()?;
        match std::fs::read(config) {
            Ok(bytes) => {
                let settings: Settings =
                    serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                scan(Path::new(&settings.folder))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ReplayLibrary {
                folder: None,
                files: Vec::new(),
            }),
            Err(error) => Err(error.to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn set_replay_folder(folder: String) -> Result<ReplayLibrary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if folder.is_empty() || folder.len() > 32_000 {
            return Err("Choose a replay folder.".into());
        }
        let library = scan(Path::new(&folder))?;
        let config = settings_path()?;
        std::fs::create_dir_all(config.parent().unwrap()).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec(&Settings {
            folder: library.folder.clone().unwrap(),
        })
        .map_err(|e| e.to_string())?;
        std::fs::write(config, bytes).map_err(|e| e.to_string())?;
        Ok(library)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn choose_replay_folder(
    window: WebviewWindow,
    initial: Option<String>,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new()
            .set_parent(&window)
            .set_title("Choose your replay folder");
        if let Some(initial) = initial
            && let Ok(directory) = std::fs::canonicalize(initial)
            && directory.is_dir()
        {
            dialog = dialog.set_directory(directory);
        }
        dialog
            .pick_folder()
            .map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn choose_recording(window: WebviewWindow) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        rfd::FileDialog::new()
            .set_parent(&window)
            .set_title("Open recording")
            .add_filter("Battle Plan", &["battleplan", "hoi4replay"])
            .pick_file()
            .map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_lists_nested_replays_without_decoding_them() {
        let root = std::env::temp_dir().join(format!(
            "hoi4-library-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("Campaign A")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(
            root.join("Campaign A/session.battleplan"),
            b"not decoded for listing",
        )
        .unwrap();
        std::fs::write(root.join("second.battleplan"), b"").unwrap();
        std::fs::write(root.join("unfinished.battleplan.tmp"), b"").unwrap();
        std::fs::write(root.join(".hidden/ignored.battleplan"), b"").unwrap();
        let result = scan(&root).unwrap();
        assert_eq!(result.files.len(), 2);
        assert!(
            result
                .files
                .iter()
                .any(|file| file.name == "session" && file.folder == "Campaign A")
        );
        assert!(
            result
                .files
                .iter()
                .any(|file| file.name == "second" && file.folder.is_empty())
        );
        assert!(scan(&root.join("missing")).is_err());
        assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
        std::fs::remove_dir_all(root).unwrap();
    }
}
