//! Where each campaign's sessions record. A campaign continues the recording
//! in its folder: the save folder itself, or the folder its last New file
//! started. File names never change; a new recording of a campaign keeps the
//! campaign's name in a new folder beside the earlier one.
use replay::recorder_settings::{NextLoad, RecorderSettings};
use std::path::{Component, Path, PathBuf};

/// A folder name the settings may hold: one plain path component.
fn plain_folder(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

/// A campaign's current folder from its saved choice.
pub fn campaign_folder(root: &Path, folder: Option<&str>) -> PathBuf {
    match folder {
        Some(name) if plain_folder(name) => root.join(name),
        _ => root.to_path_buf(),
    }
}

/// The recording a campaign's sessions continue.
pub fn recording_path(folder: &Path, campaign: &str) -> PathBuf {
    folder.join(format!("{campaign}.battleplan"))
}

/// The campaign's current folder, without using up a pending New file.
pub fn current_folder(settings: &RecorderSettings, root: &Path, campaign: &str) -> PathBuf {
    campaign_folder(
        root,
        settings
            .campaigns
            .get(campaign)
            .and_then(|choice| choice.folder.as_deref()),
    )
}

/// `<campaign> (2)`, `(3)` and so on: the first name not already in the save folder.
fn new_folder_name(root: &Path, campaign: &str) -> String {
    let mut number = 2_u64;
    loop {
        let name = format!("{campaign} ({number})");
        if !root.join(&name).exists() {
            return name;
        }
        number += 1;
    }
}

/// Where the campaign's next session (or next load) records. A pending New
/// file starts a new folder when the campaign already has a recording: one in
/// its current folder, or (`recorded`) hours of an earlier load in the running
/// session's spool, which become that recording when the session saves. It is
/// used up either way, so later sessions continue the new recording.
pub fn session_target_at(
    settings_path: &Path,
    root: &Path,
    campaign: &str,
    recorded: bool,
) -> Result<PathBuf, String> {
    let settings = RecorderSettings::load_from(settings_path)
        .map_err(|error| format!("Recorder settings could not be read: {error}"))?;
    if settings.campaigns.get(campaign).map(|choice| choice.next) != Some(NextLoad::NewFile) {
        return Ok(current_folder(&settings, root, campaign));
    }
    RecorderSettings::update_at(settings_path, |settings| {
        let choice = settings.campaigns.entry(campaign.to_owned()).or_default();
        let current = campaign_folder(root, choice.folder.as_deref());
        if choice.next != NextLoad::NewFile {
            return current;
        }
        choice.next = NextLoad::Continue;
        if !recorded && !recording_path(&current, campaign).exists() {
            return current;
        }
        let name = new_folder_name(root, campaign);
        let folder = root.join(&name);
        choice.folder = Some(name);
        folder
    })
    .map(|(_, folder)| folder)
    .map_err(|error| format!("Recorder settings could not be saved: {error}"))
}

/// Where a session attaching for this campaign records; nothing of it is in
/// that session's spool yet.
pub fn session_target(root: &Path, campaign: &str) -> Result<PathBuf, String> {
    session_target_at(&RecorderSettings::path(), root, campaign, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const CAMPAIGN: &str = "00000000-0000-4000-8000-000000000000";

    fn scratch() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "battle-plan-recorder-targets-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn new_file_starts_a_folder_beside_the_earlier_recording_once() {
        let scratch = scratch();
        let settings = scratch.join("settings.json");
        let root = scratch.join("recordings");
        fs::create_dir_all(&root).unwrap();
        // Continue records in the save folder itself.
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false).unwrap(),
            root
        );
        // New file with no earlier recording: nothing to keep apart; used up.
        RecorderSettings::update_at(&settings, |settings| {
            settings.campaigns.entry(CAMPAIGN.into()).or_default().next = NextLoad::NewFile;
        })
        .unwrap();
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false).unwrap(),
            root
        );
        let saved = RecorderSettings::load_from(&settings).unwrap();
        assert!(
            !saved.campaigns.contains_key(CAMPAIGN),
            "the used-up choice is cleared"
        );
        // New file beside an earlier recording: a new folder, and the earlier
        // recording keeps its name and contents.
        let earlier = recording_path(&root, CAMPAIGN);
        fs::write(&earlier, b"earlier recording").unwrap();
        RecorderSettings::update_at(&settings, |settings| {
            settings.campaigns.entry(CAMPAIGN.into()).or_default().next = NextLoad::NewFile;
        })
        .unwrap();
        let second = root.join(format!("{CAMPAIGN} (2)"));
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false).unwrap(),
            second
        );
        assert_eq!(fs::read(&earlier).unwrap(), b"earlier recording");
        let saved = RecorderSettings::load_from(&settings).unwrap();
        assert_eq!(saved.campaigns[CAMPAIGN].next, NextLoad::Continue);
        assert_eq!(
            saved.campaigns[CAMPAIGN].folder.as_deref(),
            Some(&*format!("{CAMPAIGN} (2)"))
        );
        // Later sessions continue the new recording.
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false).unwrap(),
            second
        );
        // Another New file skips folders already in use.
        fs::create_dir_all(&second).unwrap();
        fs::write(recording_path(&second, CAMPAIGN), b"second recording").unwrap();
        fs::create_dir_all(root.join(format!("{CAMPAIGN} (3)"))).unwrap();
        RecorderSettings::update_at(&settings, |settings| {
            settings.campaigns.get_mut(CAMPAIGN).unwrap().next = NextLoad::NewFile;
        })
        .unwrap();
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false).unwrap(),
            root.join(format!("{CAMPAIGN} (4)"))
        );
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn new_file_counts_hours_only_in_the_running_sessions_spool_as_a_recording() {
        let scratch = scratch();
        let settings = scratch.join("settings.json");
        let root = scratch.join("recordings");
        fs::create_dir_all(&root).unwrap();
        RecorderSettings::update_at(&settings, |settings| {
            settings.campaigns.entry(CAMPAIGN.into()).or_default().next = NextLoad::NewFile;
        })
        .unwrap();
        // No recording file yet, but the session spooled an earlier load: the
        // reload records into a new folder rather than the same recording.
        assert!(!recording_path(&root, CAMPAIGN).exists());
        let second = root.join(format!("{CAMPAIGN} (2)"));
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, true).unwrap(),
            second
        );
        let saved = RecorderSettings::load_from(&settings).unwrap();
        assert_eq!(saved.campaigns[CAMPAIGN].next, NextLoad::Continue);
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false).unwrap(),
            second
        );
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn only_a_plain_folder_name_moves_a_campaign() {
        let root = Path::new("C:\\Recordings");
        assert_eq!(campaign_folder(root, None), root);
        assert_eq!(campaign_folder(root, Some("a (2)")), root.join("a (2)"));
        for unsafe_name in ["..", "a\\b", "C:\\elsewhere", "", "."] {
            assert_eq!(
                campaign_folder(root, Some(unsafe_name)),
                root,
                "{unsafe_name}"
            );
        }
    }
}
