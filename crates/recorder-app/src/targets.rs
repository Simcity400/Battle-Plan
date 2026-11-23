use replay::campaign::Replay;
use replay_settings::recorder_settings::{NextLoad, RecorderSettings};
use replay_files::recording_files;
use replay_data::scope::CaptureMode;
use std::{
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

fn plain_folder(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

pub fn campaign_folder(root: &Path, folder: Option<&str>) -> PathBuf {
    match folder {
        Some(name) if plain_folder(name) => root.join(name),
        _ => root.to_path_buf(),
    }
}

pub fn recording(folder: &Path, campaign: &str) -> Option<PathBuf> {
    recording_files::recording_of(folder, campaign.as_bytes())
        .ok()
        .flatten()
}

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

fn waiting_sessions(root: &Path, folder: &Path, campaign: &str) -> Vec<PathBuf> {
    recorder::controller::session_folders(root)
        .into_iter()
        .filter(|session| {
            recorder::controller::waiting_campaigns(session)
                .iter()
                .any(|waiting| waiting.campaign == campaign && waiting.target == folder)
        })
        .collect()
}

fn holds_recording(root: &Path, folder: &Path, campaign: &str) -> bool {
    recording_files::recordings_of(folder, campaign.as_bytes())
        .map_or(true, |found| !found.is_empty())
        || replay_settings::publication::unsaved_path(folder, campaign).exists()
        || !waiting_sessions(root, folder, campaign).is_empty()
}

pub fn current_folder(settings: &RecorderSettings, root: &Path, campaign: &str) -> PathBuf {
    campaign_folder(
        root,
        settings
            .campaigns
            .get(campaign)
            .and_then(|choice| choice.folder.as_deref()),
    )
}

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

fn other_mode(folder: &Path, campaign: &str, mode: CaptureMode) -> bool {
    let other =
        |path: &Path| Replay::read_capture_mode(path).is_ok_and(|recorded| recorded != mode);
    let unsaved = replay_settings::publication::unsaved_path(folder, campaign);
    let continued = if unsaved.exists() {
        other(&unsaved)
    } else {
        recording(folder, campaign).is_some_and(|path| other(&path))
    };
    continued
}

pub fn session_target_at(
    settings_path: &Path,
    root: &Path,
    campaign: &str,
    recorded: bool,
    mode: CaptureMode,
) -> Result<(PathBuf, bool), String> {
    let settings = RecorderSettings::load_from(settings_path)
        .map_err(|error| format!("Recorder settings couldn’t be read: {error}"))?;
    let current = current_folder(&settings, root, campaign);
    if settings.campaigns.get(campaign).map(|choice| choice.next) != Some(NextLoad::NewFile)
        && !other_mode(&current, campaign, mode)
    {
        return Ok((current, false));
    }
    RecorderSettings::update_at(settings_path, |settings| {
        let choice = settings.campaigns.entry(campaign.to_owned()).or_default();
        let current = campaign_folder(root, choice.folder.as_deref());
        let new_file = choice.next == NextLoad::NewFile;
        let other = other_mode(&current, campaign, mode);
        if !new_file && !other {
            return (current, false);
        }
        choice.next = NextLoad::Continue;
        if !other && !recorded && !holds_recording(root, &current, campaign) {
            return (current, true);
        }
        let name = new_folder_name(root, campaign);
        let folder = root.join(&name);
        choice.folder = Some(name);
        (folder, new_file)
    })
    .map(|applied| applied.logged("The recording’s folder choice").result)
    .map_err(|error| format!("Recorder settings couldn’t be saved: {error}"))
}

pub fn session_target(
    root: &Path,
    campaign: &str,
    mode: CaptureMode,
) -> Result<(PathBuf, bool), String> {
    session_target_at(&RecorderSettings::path(), root, campaign, false, mode)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;

    const CAMPAIGN: &str = "00000000-0000-4000-8000-000000000000";

    pub(crate) fn recording_file(path: &Path, campaign: &str) {
        let replay = replay::campaign::Replay::from_records(
            recorder_derive::writers(),
            campaign.as_bytes().to_vec(),
            &[vec![replay_capture_spool::spool::Record::SessionStart(
                replay_data::identity::SessionStart {
                    id: 1,
                    process_id: 1,
                    process_start_time: 1,
                    qpc_frequency: 1,
                    sample_boundary: replay_data::identity::SampleBoundary::EndOfHour,
                    capture_mode: CaptureMode::Full,
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
            "battleplan-recorder-targets-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn recorded_in(folder: &Path, mode: CaptureMode) {
        use replay_capture_spool::spool::Record;
        use replay_data::identity::{SampleBoundary, SessionStart};
        let session = SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 1,
            sample_boundary: SampleBoundary::EndOfHour,
            capture_mode: mode,
            identity: replay_identity::test_fixtures::identity(),
        };
        let replay =
            Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[vec![Record::SessionStart(session)]]).unwrap();
        fs::create_dir_all(folder).unwrap();
        fs::write(folder.join("Recording.battleplan"), replay.bytes()).unwrap();
    }

    #[test]
    fn unsaved_hours_in_another_capture_mode_continue_in_a_new_folder_too() {
        let scratch = scratch();
        let settings = scratch.join("settings.json");
        let root = scratch.join("recordings");
        let unsaved = replay_settings::publication::unsaved_folder(&root);
        recorded_in(&unsaved, CaptureMode::Full);
        fs::rename(
            unsaved.join("Recording.battleplan"),
            replay_settings::publication::unsaved_path(&root, CAMPAIGN),
        )
        .unwrap();
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false, CaptureMode::Full).unwrap(),
            (root.clone(), false)
        );
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false, CaptureMode::Map).unwrap(),
            (root.join(format!("{CAMPAIGN} (2)")), false)
        );
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn a_recording_in_another_capture_mode_continues_in_a_new_folder() {
        let scratch = scratch();
        let settings = scratch.join("settings.json");
        let root = scratch.join("recordings");
        recorded_in(&root, CaptureMode::Full);
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false, CaptureMode::Full).unwrap(),
            (root.clone(), false)
        );
        let second = root.join(format!("{CAMPAIGN} (2)"));
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false, CaptureMode::Map).unwrap(),
            (second.clone(), false)
        );
        assert_eq!(
            Replay::read_capture_mode(&recording(&root, CAMPAIGN).unwrap()).unwrap(),
            CaptureMode::Full
        );
        let saved = RecorderSettings::load_from(&settings).unwrap();
        assert_eq!(saved.campaigns[CAMPAIGN].next, NextLoad::Continue);
        assert_eq!(
            saved.campaigns[CAMPAIGN].folder.as_deref(),
            Some(&*format!("{CAMPAIGN} (2)"))
        );
        recorded_in(&second, CaptureMode::Map);
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false, CaptureMode::Map).unwrap(),
            (second, false)
        );
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false, CaptureMode::Full).unwrap(),
            (root.join(format!("{CAMPAIGN} (3)")), false)
        );
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn new_file_starts_a_folder_beside_the_earlier_recording_once() {
        let scratch = scratch();
        let settings = scratch.join("settings.json");
        let root = scratch.join("recordings");
        fs::create_dir_all(&root).unwrap();
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false, CaptureMode::Full).unwrap(),
            (root.clone(), false)
        );
        let _ = RecorderSettings::update_at(&settings, |settings| {
            settings.campaigns.entry(CAMPAIGN.into()).or_default().next = NextLoad::NewFile;
        })
        .unwrap();
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false, CaptureMode::Full).unwrap(),
            (root.clone(), true)
        );
        let saved = RecorderSettings::load_from(&settings).unwrap();
        assert!(
            !saved.campaigns.contains_key(CAMPAIGN),
            "the used-up choice is cleared"
        );
        let earlier = root.join("Germany 1936–1941.battleplan");
        recording_file(&earlier, CAMPAIGN);
        let contents = fs::read(&earlier).unwrap();
        assert_eq!(recording(&root, CAMPAIGN), Some(earlier.clone()));
        let _ = RecorderSettings::update_at(&settings, |settings| {
            settings.campaigns.entry(CAMPAIGN.into()).or_default().next = NextLoad::NewFile;
        })
        .unwrap();
        let second = root.join(format!("{CAMPAIGN} (2)"));
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false, CaptureMode::Full).unwrap(),
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
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false, CaptureMode::Full).unwrap(),
            (second.clone(), false)
        );
        fs::create_dir_all(&second).unwrap();
        recording_file(&second.join("Germany 1941.battleplan"), CAMPAIGN);
        fs::create_dir_all(root.join(format!("{CAMPAIGN} (3)"))).unwrap();
        let _ = RecorderSettings::update_at(&settings, |settings| {
            settings.campaigns.get_mut(CAMPAIGN).unwrap().next = NextLoad::NewFile;
        })
        .unwrap();
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false, CaptureMode::Full).unwrap(),
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
        let _ = RecorderSettings::update_at(&settings, |settings| {
            settings.campaigns.entry(CAMPAIGN.into()).or_default().next = NextLoad::NewFile;
        })
        .unwrap();
        assert_eq!(recording(&root, CAMPAIGN), None);
        let second = root.join(format!("{CAMPAIGN} (2)"));
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, true, CaptureMode::Full).unwrap(),
            (second.clone(), true)
        );
        let saved = RecorderSettings::load_from(&settings).unwrap();
        assert_eq!(saved.campaigns[CAMPAIGN].next, NextLoad::Continue);
        assert_eq!(
            session_target_at(&settings, &root, CAMPAIGN, false, CaptureMode::Full).unwrap(),
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
