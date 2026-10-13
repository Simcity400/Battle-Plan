//! Battle Plan Recorder beside the replay app: starting it when this app opens, and
//! the recording names and campaign flags both programs read from the
//! recorder's shared settings.
use replay::{identity::is_campaign_uuid, recorder_settings::RecorderSettings};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    os::windows::process::CommandExt,
    process::{Command, Stdio},
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
    /// Display names by normalized recording path.
    names: BTreeMap<String, String>,
    /// Chosen flag tags by campaign key.
    flags: BTreeMap<String, String>,
}

fn preferences(settings: &RecorderSettings) -> RecordingPreferences {
    RecordingPreferences {
        names: settings.recording_names.clone(),
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

/// Set (or with no name, clear) individual display names by normalized
/// recording path, leaving every other name as the file holds it.
fn apply_name_changes(
    names: &mut BTreeMap<String, String>,
    changes: BTreeMap<String, Option<String>>,
) {
    for (path, name) in changes {
        match name {
            Some(name) => {
                names.insert(path, name);
            }
            None => {
                names.remove(&path);
            }
        }
    }
}

/// Change the given display names inside one locked settings update, so a
/// rename in either program never drops another; the frontend owns their
/// normalization and rules.
#[tauri::command]
pub async fn set_recording_names(changes: BTreeMap<String, Option<String>>) -> Result<(), String> {
    if changes.iter().any(|(path, name)| {
        path.is_empty()
            || path.contains('\0')
            || name.as_deref().is_some_and(|name| name.trim().is_empty())
    }) {
        return Err("Invalid recording name".into());
    }
    blocking(move || {
        RecorderSettings::update(|settings| {
            apply_name_changes(&mut settings.recording_names, changes)
        })
        .map(|_| ())
        .map_err(|error| format!("Recording names could not be saved: {error}"))
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

    #[test]
    fn a_rename_changes_only_its_own_recording() {
        let mut names = BTreeMap::from([
            (
                "c:/recordings/a.battleplan".to_owned(),
                "Saved by the recorder".to_owned(),
            ),
            ("c:/recordings/b.battleplan".to_owned(), "Old".to_owned()),
            (
                "c:/recordings/c.battleplan".to_owned(),
                "Cleared".to_owned(),
            ),
        ]);
        // The app knew neither a's name nor that b changed elsewhere; it renames only b and clears c.
        apply_name_changes(
            &mut names,
            BTreeMap::from([
                (
                    "c:/recordings/b.battleplan".to_owned(),
                    Some("New".to_owned()),
                ),
                ("c:/recordings/c.battleplan".to_owned(), None),
            ]),
        );
        assert_eq!(
            names,
            BTreeMap::from([
                (
                    "c:/recordings/a.battleplan".to_owned(),
                    "Saved by the recorder".to_owned()
                ),
                ("c:/recordings/b.battleplan".to_owned(), "New".to_owned()),
            ])
        );
    }

    #[test]
    fn preferences_carry_names_and_flags_by_frontend_key() {
        let mut settings = RecorderSettings::default();
        settings
            .recording_names
            .insert("c:/recordings/a.battleplan".into(), "Barbarossa".into());
        settings.campaigns.entry(CAMPAIGN.into()).or_default().flag = Some("GER".into());
        settings.campaigns.entry("other".into()).or_default().record = Some(false);
        let preferences = preferences(&settings);
        assert_eq!(
            preferences.names["c:/recordings/a.battleplan"],
            "Barbarossa"
        );
        assert_eq!(
            preferences.flags,
            BTreeMap::from([(campaign_key(CAMPAIGN), "GER".to_owned())])
        );
    }
}
