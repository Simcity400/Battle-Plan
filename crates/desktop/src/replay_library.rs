use replay_settings::recorder_settings::RecorderSettings;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::WebviewWindow;

#[derive(Serialize)]
pub struct ReplayFile {
    pub(super) path: String,
    name: String,
    folder: String,
    partial: Option<dashboard_core::dto::PartialRecording>,
}
#[derive(Serialize)]
pub struct ReplayLibrary {
    folder: Option<String>,
    pub(super) files: Vec<ReplayFile>,
}
pub(super) fn scan(folder: &Path) -> Result<ReplayLibrary, String> {
    scan_with(folder, || Ok(()))
}

pub(super) fn scan_with(
    folder: &Path,
    mut checkpoint: impl FnMut() -> Result<(), String>,
) -> Result<ReplayLibrary, String> {
    let mut files = Vec::new();
    let root = walk(folder, &mut checkpoint, |root, directory, path, _| {
        files.push(ReplayFile {
            partial: dashboard_text::metadata::read_partial_recording(path)
                .ok()
                .flatten(),
            path: path.to_string_lossy().into_owned(),
            name: path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            folder: directory
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/"),
        });
        Ok(())
    })?;
    checkpoint()?;
    files.sort_by_cached_key(|file| format!("{}/{}", file.folder, file.name).to_lowercase());
    Ok(ReplayLibrary {
        folder: Some(root.to_string_lossy().into_owned()),
        files,
    })
}

fn walk(
    folder: &Path,
    checkpoint: &mut impl FnMut() -> Result<(), String>,
    mut each: impl FnMut(&Path, &Path, &Path, &std::fs::Metadata) -> Result<(), String>,
) -> Result<PathBuf, String> {
    checkpoint()?;
    let root =
        std::fs::canonicalize(folder).map_err(|e| format!("Couldn’t read the recordings folder: {e}"))?;
    let mut pending = vec![root.clone()];
    let mut visited = 0;
    while let Some(directory) = pending.pop() {
        checkpoint()?;
        for entry in
            std::fs::read_dir(&directory).map_err(|e| format!("Couldn’t read the recordings folder: {e}"))?
        {
            checkpoint()?;
            let entry = entry.map_err(|e| format!("Couldn’t read the recordings folder: {e}"))?;
            visited += 1;
            if visited > 50_000 {
                return Err(
                    "This folder is too large. Choose a folder that holds only your recordings."
                        .into(),
                );
            }
            let kind = entry
                .file_type()
                .map_err(|e| format!("Couldn’t read the recordings folder: {e}"))?;
            let metadata = entry
                .metadata()
                .map_err(|e| format!("Couldn’t read the recordings folder: {e}"))?;
            if kind.is_symlink()
                || entry.file_name().to_string_lossy().starts_with('.')
                || hidden(&metadata)
            {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() && replay_files::recording_files::is_recording(&path) {
                checkpoint()?;
                each(&root, &directory, &path, &metadata)?;
            }
        }
    }
    Ok(root)
}

const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;

fn hidden(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
}

pub(super) fn configured_folder() -> Result<PathBuf, String> {
    RecorderSettings::load()
        .map(|shared| shared.effective_save_folder())
        .map_err(|error| format!("Recorder settings couldn’t be read: {error}"))
}

#[tauri::command]
pub async fn replay_library() -> Result<ReplayLibrary, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let folder = configured_folder()?;
        if folder == replay_settings::recorder_settings::default_save_folder() {
            std::fs::create_dir_all(&folder)
                .map_err(|e| format!("Couldn’t create the recordings folder: {e}"))?;
        }
        scan(&folder)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn replay_library_stamp() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| stamp(&configured_folder()?))
        .await
        .map_err(|e| e.to_string())?
}

fn stamp(folder: &Path) -> Result<String, String> {
    use std::hash::{Hash, Hasher};
    let mut hash = std::hash::DefaultHasher::new();
    let root = walk(folder, &mut || Ok(()), |_, _, path, metadata| {
        path.hash(&mut hash);
        metadata.len().hash(&mut hash);
        metadata.modified().ok().hash(&mut hash);
        Ok(())
    })?;
    root.hash(&mut hash);
    Ok(format!("{:016x}", hash.finish()))
}

#[tauri::command]
pub async fn set_replay_folder(folder: String) -> Result<ReplayLibrary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if folder.is_empty() || folder.len() > 32_000 {
            return Err("Choose a recordings folder.".into());
        }
        let library = scan(Path::new(&folder))?;
        let chosen = PathBuf::from(folder);
        RecorderSettings::update(|shared| shared.choose_save_folder(chosen))
            .map_err(|error| format!("Recorder settings couldn’t be saved: {error}"))?;
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
            .set_title("Choose your recordings folder");
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
            .add_filter("BattlePlan", &[replay_files::recording_files::EXTENSION])
            .pick_file()
            .map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn cancellation_interrupts_a_scan_between_recording_metadata_reads() {
        let root = std::env::temp_dir().join(format!(
            "hoi4-library-cancel-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        for number in 0..10 {
            std::fs::write(
                root.join(format!("{number}.battleplan")),
                b"synthetic listing",
            )
            .unwrap();
        }
        let mut checks = 0;
        let error = scan_with(&root, || {
            checks += 1;
            if checks == 6 {
                Err("synthetic cancellation".into())
            } else {
                Ok(())
            }
        })
        .err()
        .unwrap();
        assert_eq!(error, "synthetic cancellation");
        assert_eq!(checks, 6, "the rest of the library was not scanned");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn library_lists_nested_replays_without_decoding_them() {
        let root = std::env::temp_dir().join(format!(
            "hoi4-library-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
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

    #[test]
    fn the_stamp_changes_between_two_empty_folders() {
        let root = std::env::temp_dir().join(format!(
            "hoi4-library-stamp-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("A")).unwrap();
        std::fs::create_dir_all(root.join("B")).unwrap();
        let first = stamp(&root.join("A")).unwrap();
        assert_eq!(stamp(&root.join("A")).unwrap(), first);
        assert_ne!(stamp(&root.join("B")).unwrap(), first);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn library_leaves_out_what_windows_marks_hidden() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = std::env::temp_dir().join(format!(
            "hoi4-library-hidden-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("Archived")).unwrap();
        std::fs::write(root.join("Archived/old.battleplan"), b"").unwrap();
        let marked = std::process::Command::new("attrib")
            .arg("+h")
            .arg(root.join("Archived"))
            .status()
            .unwrap();
        assert!(marked.success());
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .attributes(FILE_ATTRIBUTE_HIDDEN)
            .open(root.join("hidden.battleplan"))
            .unwrap();
        std::fs::write(root.join("shown.battleplan"), b"").unwrap();
        let result = scan(&root).unwrap();
        let names: Vec<_> = result.files.iter().map(|file| file.name.as_str()).collect();
        assert_eq!(names, ["shown"]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
