//! Battle Plan Recorder beside the replay app: starting it, shared campaign flags, and recording files.
use replay::{
    recorder_settings::RecorderSettings,
    recording_files::{self, NamesLock},
};
use replay_data::identity::is_campaign_uuid;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    time::SystemTime,
};
use windows_sys::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
};

const RECORDER: &str = "battle-plan-recorder.exe";

/// Start the sibling recorder when "With Battle Plan" is on. It runs on its
/// own: no console, no inherited pipes, outside any job this app is in, so
/// closing the app never stops a recording. The recorder allows one instance
/// per user, so a running recorder makes this a no-op.
pub fn start_recorder() {
    if !RecorderSettings::load().map_or(true, |settings| settings.start_with_replay) {
        return;
    }
    let Ok(executable) = std::env::current_exe().map(|path| path.with_file_name(RECORDER)) else {
        return;
    };
    if !executable.is_file() {
        return;
    }
    let spawn = |flags| {
        Command::new(&executable)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(flags)
            .spawn()
    };
    let detached = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    // A job that forbids breakaway refuses the flag; the recorder still starts.
    if let Err(error) = spawn(detached | CREATE_BREAKAWAY_FROM_JOB).or_else(|_| spawn(detached)) {
        eprintln!("Battle Plan Recorder could not start: {error}");
    }
}

/// A campaign's key in the frontend: its recorded identity's bytes in hex.
fn campaign_key(id: &str) -> String {
    id.bytes().map(|byte| format!("{byte:02x}")).collect()
}

/// The recorded campaign identity a frontend key names, when it is one.
fn campaign_id(key: &str) -> Option<String> {
    if !key.len().is_multiple_of(2) {
        return None;
    }
    let bytes = (0..key.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(key.get(at..at + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    is_campaign_uuid(&bytes)
        .then(|| String::from_utf8(bytes).ok())
        .flatten()
}

#[derive(Debug, Default, PartialEq, Serialize)]
pub struct RecordingPreferences {
    /// Chosen flag tags by campaign key.
    flags: BTreeMap<String, String>,
}

fn preferences(settings: &RecorderSettings) -> RecordingPreferences {
    RecordingPreferences {
        flags: settings
            .campaigns
            .iter()
            .filter_map(|(id, choice)| Some((campaign_key(id), choice.flag.clone()?)))
            .collect(),
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn recorder_settings() -> Result<RecorderSettings, String> {
    tauri::async_runtime::spawn_blocking(RecorderSettings::load)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recording_preferences() -> Result<RecordingPreferences, String> {
    blocking(|| {
        RecorderSettings::load()
            .map(|settings| preferences(&settings))
            .map_err(|error| format!("Recorder settings could not be read: {error}"))
    })
    .await
}

/// Rename a recording's file to the user's name, which later saves keep; returns its new path.
#[tauri::command]
pub async fn rename_recording(path: String, name: String) -> Result<String, String> {
    blocking(move || {
        rename(Path::new(&path), &name, &RecorderSettings::path())
            .map(|renamed| renamed.display().to_string())
    })
    .await
}

/// Succeeds exactly when the file moved.
fn rename(path: &Path, name: &str, settings: &Path) -> Result<PathBuf, String> {
    let failed = |error: std::io::Error| format!("The recording could not be renamed: {error}");
    let stem = recording_files::file_stem(name)
        .ok_or("Choose a name with a letter or number Windows allows in file names.")?;
    if !path.is_file() || !recording_files::is_recording(path) {
        return Err("Choose a .battleplan recording to rename.".into());
    }
    let folder = std::fs::canonicalize(path.parent().ok_or("This recording has no folder.")?)
        .map_err(failed)?;
    let _lock = NamesLock::try_now(&folder)
        .map_err(failed)?
        .ok_or("Battle Plan Recorder is saving into this folder. Try again in a moment.")?;
    let campaign = recording_files::campaign_of(path)
        .map_err(failed)?
        .and_then(|id| String::from_utf8(id).ok());
    // The name is the user's before the file moves, so no save renames it back.
    let added = match &campaign {
        Some(campaign) => {
            RecorderSettings::update_at(settings, |settings| {
                let before = settings.name_chosen(&folder, campaign.as_bytes());
                settings.choose_name(&folder, campaign, true);
                !before
            })
            .map_err(failed)?
            .1
        }
        None => false,
    };
    recording_files::rename(path, &stem).map_err(|error| {
        if let (true, Some(campaign)) = (added, &campaign) {
            let _ = RecorderSettings::update_at(settings, |settings| {
                settings.choose_name(&folder, campaign, false)
            });
        }
        // Sharing violation: the recorder holds the campaign it is recording.
        if error.raw_os_error() == Some(32) {
            "Battle Plan Recorder is recording into this file. Rename it once the session is saved."
                .to_owned()
        } else {
            failed(error)
        }
    })
}

/// A recording the frontend shows: where it was last, and its campaign.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    path: String,
    campaign_key: String,
}

type Listing = Vec<(PathBuf, u64, SystemTime)>;

/// Each folder's recordings by campaign, read again only once its recordings change.
static FOLDERS: Mutex<BTreeMap<PathBuf, (Listing, recording_files::Found)>> =
    Mutex::new(BTreeMap::new());

/// `listings` holds this request's folder listings, so each folder is listed once.
fn located(
    listings: &mut BTreeMap<PathBuf, Option<Listing>>,
    path: &Path,
    campaign_key: &str,
) -> Option<PathBuf> {
    let campaign = campaign_id(campaign_key)?.to_ascii_lowercase().into_bytes();
    let folder = path.parent()?;
    let listing = listings
        .entry(folder.to_path_buf())
        .or_insert_with(|| recording_files::listing(folder).ok())
        .as_ref()?;
    let mut folders = FOLDERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let found = match folders.get(folder) {
        Some((known, found)) if known == listing => found,
        _ => {
            let found = recording_files::recordings(folder).ok()?;
            &folders
                .entry(folder.to_path_buf())
                .insert_entry((listing.clone(), found))
                .into_mut()
                .1
        }
    };
    match found.get(&campaign)?.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    }
}

/// Where each recording is now, by its folder and campaign; `None` unless exactly one is there.
#[tauri::command]
pub async fn locate_recordings(recordings: Vec<Shown>) -> Result<Vec<Option<String>>, String> {
    blocking(move || {
        let mut listings = BTreeMap::new();
        Ok(recordings
            .iter()
            .map(|shown| {
                located(&mut listings, Path::new(&shown.path), &shown.campaign_key)
                    .map(|path| path.display().to_string())
            })
            .collect())
    })
    .await
}

/// Choose (or with no tag, clear) the flag a campaign shows in the sidebar.
#[tauri::command]
pub async fn set_campaign_flag(campaign_key: String, tag: Option<String>) -> Result<(), String> {
    let id = campaign_id(&campaign_key).ok_or("Unknown campaign")?;
    if tag
        .as_deref()
        .is_some_and(|tag| tag.is_empty() || tag.contains('\0'))
    {
        return Err("Invalid flag tag".into());
    }
    blocking(move || {
        RecorderSettings::update(|settings| settings.campaigns.entry(id).or_default().flag = tag)
            .map(|_| ())
            .map_err(|error| format!("Campaign flag could not be saved: {error}"))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAMPAIGN: &str = "00000000-0000-4000-8000-000000000000";

    #[test]
    fn frontend_campaign_keys_name_recorded_campaigns_only() {
        let key = campaign_key(CAMPAIGN);
        assert_eq!(key.len(), CAMPAIGN.len() * 2);
        assert_eq!(campaign_id(&key).as_deref(), Some(CAMPAIGN));
        let not_campaign = campaign_key("not a campaign");
        for key in ["", "abc", "zz", not_campaign.as_str()] {
            assert_eq!(campaign_id(key), None, "{key}");
        }
    }

    /// A file with a recording's header naming the campaign.
    fn recording(path: &Path) {
        let mut bytes = b"H4RP".to_vec();
        bytes.extend_from_slice(
            &replay_data::format::FormatVersion::CURRENT
                .version()
                .to_le_bytes(),
        );
        bytes.extend_from_slice(&720_u32.to_le_bytes());
        bytes.extend_from_slice(&(CAMPAIGN.len() as u32).to_le_bytes());
        bytes.extend_from_slice(CAMPAIGN.as_bytes());
        std::fs::write(path, bytes).unwrap();
    }

    fn scratch() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "battle-plan-rename-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("recordings")).unwrap();
        root
    }

    #[test]
    fn a_users_rename_renames_the_file_and_sticks() {
        let root = scratch();
        let settings = root.join("settings.json");
        let folder = root.join("recordings");
        let path = folder.join("Germany 1936–1941.battleplan");
        recording(&path);
        std::fs::write(folder.join("Barbarossa.battleplan"), b"another").unwrap();
        // A taken name gets the next number; unsafe characters are dropped.
        let renamed = rename(&path, " Barbarossa? ", &settings).unwrap();
        assert_eq!(renamed, folder.join("Barbarossa (2).battleplan"));
        assert!(!path.exists());
        assert_eq!(
            std::fs::read(folder.join("Barbarossa.battleplan")).unwrap(),
            b"another"
        );
        // Later saves keep the name, whatever the file is called next.
        let saved = RecorderSettings::load_from(&settings).unwrap();
        let identity = std::fs::canonicalize(&folder).unwrap();
        assert!(saved.name_chosen(&identity, CAMPAIGN.as_bytes()));
        // While the recorder holds the folder, nothing changes.
        let held = NamesLock::wait(&folder).unwrap();
        assert!(
            rename(&renamed, "Other", &settings)
                .unwrap_err()
                .contains("saving")
        );
        drop(held);
        assert!(renamed.exists());
        // A name with nothing Windows allows is refused before anything changes.
        assert!(rename(&renamed, " ?* ", &settings).is_err());
        assert!(renamed.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_failed_rename_leaves_the_name_to_the_recorder() {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        let root = scratch();
        let settings = root.join("settings.json");
        let folder = root.join("recordings");
        let path = folder.join("Germany 1936.battleplan");
        recording(&path);
        // The recorder holds the campaign it records without delete sharing.
        let lease = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .unwrap();
        let error = rename(&path, "Barbarossa", &settings).unwrap_err();
        assert!(error.contains("recording into this file"), "{error}");
        drop(lease);
        assert!(path.exists());
        assert!(
            RecorderSettings::load_from(&settings)
                .unwrap()
                .chosen_names
                .is_empty()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_shown_recording_is_found_again_by_its_folder_and_campaign() {
        let root = scratch();
        let folder = root.join("recordings");
        let path = folder.join("Germany 1936.battleplan");
        recording(&path);
        let key = campaign_key(CAMPAIGN);
        assert_eq!(
            located(&mut BTreeMap::new(), &path, &key),
            Some(path.clone())
        );
        let renamed = recording_files::rename(&path, "Germany 1936–1937").unwrap();
        assert_eq!(
            located(&mut BTreeMap::new(), &path, &key),
            Some(renamed.clone())
        );
        // A copy beside it: neither is guessed.
        std::fs::copy(&renamed, folder.join("Copy.battleplan")).unwrap();
        assert_eq!(located(&mut BTreeMap::new(), &renamed, &key), None);
        assert_eq!(
            located(&mut BTreeMap::new(), &renamed, "not a campaign"),
            None
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preferences_carry_flags_by_frontend_key() {
        let mut settings = RecorderSettings::default();
        settings.campaigns.entry(CAMPAIGN.into()).or_default().flag = Some("GER".into());
        settings.campaigns.entry("other".into()).or_default().record = Some(false);
        assert_eq!(
            preferences(&settings).flags,
            BTreeMap::from([(campaign_key(CAMPAIGN), "GER".to_owned())])
        );
    }
}
