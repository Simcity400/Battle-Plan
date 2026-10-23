//! Where each campaign records: the save folder, or its last New file's folder; one recording each.
use replay::{recorder_settings::NextLoad, recorder_settings::RecorderSettings, recording_files};
use std::{
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

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

/// The recording a campaign's sessions continue, when it has one there.
pub fn recording(folder: &Path, campaign: &str) -> Option<PathBuf> {
    recording_files::recording_of(folder, campaign.as_bytes())
        .ok()
        .flatten()
}

/// [`recording`], found again only once the folder's recordings change.
#[derive(Default)]
pub struct RecordingCache {
    key: Option<(PathBuf, String, Vec<(PathBuf, u64, SystemTime)>)>,
    found: Option<PathBuf>,
}

impl RecordingCache {
    pub fn get(&mut self, folder: &Path, campaign: &str) -> Option<PathBuf> {
        let listing = recording_files::listing(folder).unwrap_or_default();
        let key = (folder.to_path_buf(), campaign.to_owned(), listing);
        if self.key.as_ref() != Some(&key) {
            self.found = recording(folder, campaign);
            self.key = Some(key);
        }
        self.found.clone()
    }
}

/// Whether the folder may hold the campaign's recording; one that cannot be read might.
fn has_recording(folder: &Path, campaign: &str) -> bool {
    recording_files::recordings_of(folder, campaign.as_bytes())
        .map_or(true, |found| !found.is_empty())
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
/// used up either way; the returned flag records consumption under the settings lock.
pub fn session_target_at(
    settings_path: &Path,
    root: &Path,
    campaign: &str,
    recorded: bool,
) -> Result<(PathBuf, bool), String> {
    let settings = RecorderSettings::load_from(settings_path)
        .map_err(|error| format!("Recorder settings could not be read: {error}"))?;
    if settings.campaigns.get(campaign).map(|choice| choice.next) != Some(NextLoad::NewFile) {
        return Ok((current_folder(&settings, root, campaign), false));
    }
    RecorderSettings::update_at(settings_path, |settings| {
        let choice = settings.campaigns.entry(campaign.to_owned()).or_default();
        let current = campaign_folder(root, choice.folder.as_deref());
        if choice.next != NextLoad::NewFile {
            return (current, false);
        }
        choice.next = NextLoad::Continue;
        if !recorded && !has_recording(&current, campaign) {
            return (current, true);
        }
        let name = new_folder_name(root, campaign);
        let folder = root.join(&name);
        choice.folder = Some(name);
        (folder, true)
    })
    .map(|(_, target)| target)
    .map_err(|error| format!("Recorder settings could not be saved: {error}"))
}

/// Returns the folder and whether it consumed New file; this session has no spooled hours yet.
pub fn session_target(root: &Path, campaign: &str) -> Result<(PathBuf, bool), String> {
    session_target_at(&RecorderSettings::path(), root, campaign, false)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;

    const CAMPAIGN: &str = "00000000-0000-4000-8000-000000000000";

    /// A file with a recording's header naming `campaign`.
    pub(crate) fn recording_file(path: &Path, campaign: &str) {
        let replay = replay::container::Replay::from_records(
            campaign.as_bytes().to_vec(),
            &[vec![replay::spool::Record::SessionStart(
                replay_data::identity::SessionStart {
                    id: 1,
                    process_id: 1,
                    process_start_time: 1,
                    qpc_frequency: 1,
                    sample_boundary: replay_data::identity::SampleBoundary::EndOfHour,
                    identity: replay_data::identity::Identity {
                        game_version: "synthetic".into(),
                        exe_hash: [1; 32],
                        exe_size: 1,
                        vanilla_hash: [2; 32],
                        dlc: vec![],
                        mods: vec![],
                        coverage_policy: 1,
                        language: "english".into(),
                        recorder_version: "synthetic".into(),
                        collector_hash: [3; 32],
                        verification: replay_data::identity::Verification::Unchecked,
                    },
                },
            )]],
        )
        .unwrap();
        fs::write(path, replay.bytes()).unwrap();
    }

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
            (root.clone(), false)
        );
        // New file with no earlier recording: nothing to keep apart; used up.
        RecorderSettings::update_at(&settings, |settings| {
            settings.campaigns.entry(CAMPAIGN.into()).or_default().next = NextLoad::NewFile;
        })
        .unwrap();
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false).unwrap(),
            (root.clone(), true)
        );
        let saved = RecorderSettings::load_from(&settings).unwrap();
        assert!(
            !saved.campaigns.contains_key(CAMPAIGN),
            "the used-up choice is cleared"
        );
        // New file beside an earlier recording, whatever its name: a new folder, the earlier kept.
        let earlier = root.join("Germany 1936–1941.battleplan");
        recording_file(&earlier, CAMPAIGN);
        let contents = fs::read(&earlier).unwrap();
        assert_eq!(recording(&root, CAMPAIGN), Some(earlier.clone()));
        RecorderSettings::update_at(&settings, |settings| {
            settings.campaigns.entry(CAMPAIGN.into()).or_default().next = NextLoad::NewFile;
        })
        .unwrap();
        let second = root.join(format!("{CAMPAIGN} (2)"));
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false).unwrap(),
            (second.clone(), true)
        );
        assert_eq!(fs::read(&earlier).unwrap(), contents);
        assert_eq!(recording(&second, CAMPAIGN), None);
        let saved = RecorderSettings::load_from(&settings).unwrap();
        assert_eq!(saved.campaigns[CAMPAIGN].next, NextLoad::Continue);
        assert_eq!(
            saved.campaigns[CAMPAIGN].folder.as_deref(),
            Some(&*format!("{CAMPAIGN} (2)"))
        );
        // Later sessions continue the new recording.
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false).unwrap(),
            (second.clone(), false)
        );
        // Another New file skips folders already in use.
        fs::create_dir_all(&second).unwrap();
        recording_file(&second.join("Germany 1941.battleplan"), CAMPAIGN);
        fs::create_dir_all(root.join(format!("{CAMPAIGN} (3)"))).unwrap();
        RecorderSettings::update_at(&settings, |settings| {
            settings.campaigns.get_mut(CAMPAIGN).unwrap().next = NextLoad::NewFile;
        })
        .unwrap();
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false).unwrap(),
            (root.join(format!("{CAMPAIGN} (4)")), true)
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
        assert_eq!(recording(&root, CAMPAIGN), None);
        let second = root.join(format!("{CAMPAIGN} (2)"));
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, true).unwrap(),
            (second.clone(), true)
        );
        let saved = RecorderSettings::load_from(&settings).unwrap();
        assert_eq!(saved.campaigns[CAMPAIGN].next, NextLoad::Continue);
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false).unwrap(),
            (second, false)
        );
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn the_loaded_campaigns_recording_is_found_again_after_a_rename() {
        let root = scratch();
        let mut cache = RecordingCache::default();
        assert_eq!(cache.get(&root, CAMPAIGN), None);
        let first = root.join("Germany 1936.battleplan");
        recording_file(&first, CAMPAIGN);
        assert_eq!(cache.get(&root, CAMPAIGN), Some(first.clone()));
        let renamed = recording_files::rename(&first, "Germany 1936–1937").unwrap();
        assert_eq!(cache.get(&root, CAMPAIGN), Some(renamed));
        fs::remove_dir_all(root).unwrap();
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
