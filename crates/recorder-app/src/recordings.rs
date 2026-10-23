//! The panel's recordings, listed only while it asks, from directory listings and file metadata.
use crate::{flags::FlagImage, targets};
use recorder::controller::{self, Waiting, WaitingCampaign};
use replay::{container::Replay, recorder_settings::RecorderSettings, recording_files};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::SystemTime,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Recording,
    Waiting,
    Saving,
}

/// A campaign's recorded hours not saved yet.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unsaved {
    pub session: String,
    pub campaign: String,
    /// The saved recording these hours continue, when there is one.
    pub recording: Option<String>,
    /// Its file name, or the name its file will get; `None` until known.
    pub name: Option<String>,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
    /// Spool bytes on disk; `None` when its folder could not be read.
    pub spool_bytes: Option<u64>,
    pub status: Status,
    pub flag: Option<FlagImage>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub path: String,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
    /// The file's size from its metadata.
    pub bytes: u64,
    pub flag: Option<FlagImage>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recordings {
    pub unsaved: Vec<Unsaved>,
    pub saved: Vec<Saved>,
}

/// The running session, as the worker knows it.
pub struct Live<'a> {
    pub session: Option<&'a Path>,
    /// The folder it saves into.
    pub target: Option<&'a Path>,
    /// The campaign recording now.
    pub campaign: Option<&'a str>,
    /// The date it has reached.
    pub hour: Option<i32>,
    pub set_aside: &'a [Waiting],
    /// The name each of its campaigns' recordings in `target` would get so far.
    pub names: &'a BTreeMap<String, String>,
}

type Span = Option<(i32, i32)>;

/// Saved files' spans and ended sessions' waiting campaigns, read once per file version.
#[derive(Default)]
pub struct Library(Mutex<Cache>);

#[derive(Default)]
struct Cache {
    spans: HashMap<PathBuf, (u64, Option<SystemTime>, Span)>,
    sessions: HashMap<PathBuf, (Option<(SystemTime, u64)>, Vec<WaitingCampaign>)>,
    /// Each folder's recordings by campaign, read again once its listing changes.
    folders: HashMap<PathBuf, (Vec<(PathBuf, u64, SystemTime)>, recording_files::Found)>,
}

impl Cache {
    /// A folder's recordings by the campaign their header names; one that cannot be read names none.
    fn recordings(&mut self, folder: &Path) -> &recording_files::Found {
        let listing = recording_files::listing(folder).unwrap_or_default();
        let (known, found) = self.folders.entry(folder.to_path_buf()).or_default();
        if *known != listing {
            *found = recording_files::recordings(folder).unwrap_or_default();
            *known = listing;
        }
        found
    }

    /// The saved recording a row's hours continue, and the name it has or will get.
    fn row(
        &mut self,
        live: &Live<'_>,
        target: &Path,
        campaign: &str,
    ) -> (Option<String>, Option<String>) {
        let continued = self.continued(target, campaign);
        let name = match &continued {
            Some(path) => file_name(path),
            // Named by the running spool alone: not a recording elsewhere, nor one that adds to unsaved hours.
            None if live.target == Some(target)
                && matches!(
                    recording_files::unsaved_recording(target, campaign.as_bytes()),
                    Ok(None)
                ) =>
            {
                live.names
                    .get(campaign)
                    .and_then(|stem| recording_files::unique_path(target, stem, None).ok())
                    .as_deref()
                    .and_then(file_name)
            }
            None => None,
        };
        (continued.map(|path| path.display().to_string()), name)
    }

    /// The recording a campaign's unsaved hours continue in `folder`: its one recording there.
    fn continued(&mut self, folder: &Path, campaign: &str) -> Option<PathBuf> {
        match self
            .recordings(folder)
            .get(&campaign.as_bytes().to_ascii_lowercase())
            .map(Vec::as_slice)
        {
            Some([path]) => Some(path.clone()),
            _ => None,
        }
    }
}

fn file_name(path: &Path) -> Option<String> {
    path.file_stem()?.to_str().map(str::to_owned)
}

fn version(path: &Path) -> Option<(SystemTime, u64)> {
    let metadata = fs::metadata(path).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

impl Library {
    pub fn list(
        &self,
        settings: &RecorderSettings,
        live: &Live<'_>,
        flags: &BTreeMap<String, FlagImage>,
    ) -> Recordings {
        let root = settings.effective_save_folder();
        let root = root.canonicalize().unwrap_or(root);
        let flag = |campaign: &str| {
            flags
                .iter()
                .find(|(known, _)| known.eq_ignore_ascii_case(campaign))
                .map(|(_, flag)| flag.clone())
        };
        let mut cache = self.0.lock().unwrap_or_else(|error| error.into_inner());
        // One row per recording: what the running session and every ended one hold of it.
        let mut unsaved: Vec<(PathBuf, Unsaved)> = Vec::new();
        let mut add = |target: &Path, row: Unsaved| match unsaved
            .iter_mut()
            .find(|(known, known_row)| known == target && known_row.campaign == row.campaign)
        {
            Some((_, known)) => {
                known.first_hour = match (known.first_hour, row.first_hour) {
                    (Some(known), Some(first)) => Some(known.min(first)),
                    (known, first) => known.or(first),
                };
                known.last_hour = known.last_hour.max(row.last_hour);
                // One session's spool is counted once.
                if known.session != row.session {
                    known.spool_bytes = known.spool_bytes.zip(row.spool_bytes).map(|(a, b)| a + b);
                }
            }
            None => unsaved.push((target.to_owned(), row)),
        };
        if let Some(session) = live.session {
            // Left for another campaign that has not written an hour yet, it waits rather than records.
            if let Some(campaign) = live.campaign
                && !live
                    .set_aside
                    .iter()
                    .any(|waiting| waiting.campaign == campaign)
            {
                let target = targets::current_folder(settings, &root, campaign);
                let (continued, name) = cache.row(live, &target, campaign);
                add(
                    &target,
                    Unsaved {
                        session: session.display().to_string(),
                        campaign: campaign.to_owned(),
                        recording: continued,
                        name,
                        first_hour: None,
                        last_hour: live.hour,
                        spool_bytes: controller::unsaved_spool_bytes(&session.join(campaign)),
                        status: Status::Recording,
                        flag: flag(campaign),
                    },
                );
            }
            for waiting in live.set_aside {
                let target = targets::current_folder(settings, &root, &waiting.campaign);
                let (continued, name) = cache.row(live, &target, &waiting.campaign);
                add(
                    &target,
                    Unsaved {
                        session: session.display().to_string(),
                        campaign: waiting.campaign.clone(),
                        recording: continued,
                        name,
                        first_hour: waiting.unsaved.first_hour,
                        last_hour: waiting.unsaved.last_hour,
                        spool_bytes: controller::unsaved_spool_bytes(
                            &session.join(&waiting.campaign),
                        ),
                        status: if waiting.saving {
                            Status::Saving
                        } else {
                            Status::Waiting
                        },
                        flag: flag(&waiting.campaign),
                    },
                );
            }
        }
        let mut sessions = controller::session_folders(&root);
        // Newest first: session folders are named by game start and time.
        sessions.sort_by(|left, right| right.cmp(left));
        cache
            .sessions
            .retain(|session, _| sessions.contains(session));
        for session in sessions {
            if Some(session.as_path()) == live.session {
                continue;
            }
            let stamp = version(&session.join("completed.json"));
            let entry = cache
                .sessions
                .entry(session.clone())
                .or_insert_with(|| (None, Vec::new()));
            if entry.0 != stamp || stamp.is_none() {
                *entry = (stamp, controller::waiting_campaigns(&session));
            }
            for waiting in entry.1.clone() {
                let (continued, name) = cache.row(live, &waiting.target, &waiting.campaign);
                add(
                    &waiting.target,
                    Unsaved {
                        session: session.display().to_string(),
                        campaign: waiting.campaign.clone(),
                        recording: continued,
                        name,
                        first_hour: waiting.first_hour,
                        last_hour: waiting.last_hour,
                        spool_bytes: controller::unsaved_spool_bytes(
                            &session.join(&waiting.campaign),
                        ),
                        status: Status::Waiting,
                        flag: flag(&waiting.campaign),
                    },
                );
            }
        }
        let mut folders: BTreeSet<PathBuf> =
            unsaved.iter().map(|(target, _)| target.clone()).collect();
        let unsaved = unsaved.into_iter().map(|(_, row)| row).collect();
        let mut saved = Vec::new();
        for path in saved_files(&root) {
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            let version = (metadata.len(), metadata.modified().ok());
            let span = match cache.spans.get(&path) {
                Some((bytes, time, span)) if (*bytes, *time) == version => *span,
                _ => {
                    let span = Replay::read_hour_span(&path).ok().flatten();
                    cache
                        .spans
                        .insert(path.clone(), (version.0, version.1, span));
                    span
                }
            };
            let campaign = path.parent().and_then(|folder| {
                folders.insert(folder.to_path_buf());
                cache
                    .recordings(folder)
                    .iter()
                    .find(|(_, paths)| paths.contains(&path))
                    .map(|(campaign, _)| String::from_utf8_lossy(campaign).into_owned())
            });
            saved.push((
                version.1,
                Saved {
                    path: path.display().to_string(),
                    first_hour: span.map(|(first, _)| first),
                    last_hour: span.map(|(_, last)| last),
                    bytes: metadata.len(),
                    flag: campaign.as_deref().and_then(flag),
                },
            ));
        }
        let listed: Vec<_> = saved
            .iter()
            .map(|(_, saved)| PathBuf::from(&saved.path))
            .collect();
        cache.spans.retain(|path, _| listed.contains(path));
        cache.folders.retain(|folder, _| folders.contains(folder));
        // Most recently saved first.
        saved.sort_by(|left, right| right.0.cmp(&left.0));
        Recordings {
            unsaved,
            saved: saved.into_iter().map(|(_, saved)| saved).collect(),
        }
    }
}

/// Recordings in the save folder and the campaign folders beside them.
fn saved_files(root: &Path) -> Vec<PathBuf> {
    let recordings = |folder: &Path| -> Vec<PathBuf> {
        fs::read_dir(folder)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("battleplan") || extension.eq_ignore_ascii_case("hoi4replay"))
            })
            .collect()
    };
    let mut files = recordings(root);
    for entry in fs::read_dir(root).into_iter().flatten().flatten() {
        let name = entry.file_name();
        if entry.file_type().is_ok_and(|kind| kind.is_dir())
            && name != "sessions"
            && !name.to_string_lossy().starts_with('.')
        {
            files.extend(recordings(&entry.path()));
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAMPAIGN: &str = "00000000-0000-4000-8000-000000000000";
    const OTHER: &str = "00000000-0000-4000-8000-000000000001";

    fn scratch() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "battle-plan-recorder-recordings-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    fn write(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![0; bytes]).unwrap();
    }

    /// A file of `bytes` whose header names `campaign`, cut short after it.
    fn recording(path: &Path, campaign: &str, bytes: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        targets::tests::recording_file(path, campaign);
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(bytes)
            .unwrap();
    }

    #[test]
    fn recordings_list_unsaved_spool_and_saved_files_from_metadata() {
        let root = scratch();
        let settings = RecorderSettings {
            save_folder: Some(root.clone()),
            ..RecorderSettings::default()
        };
        // Saved: in the save folder and in a campaign's own folder, found by header.
        let germany = root.join("Germany 1936–1941.battleplan");
        recording(&germany, CAMPAIGN, 3000);
        recording(
            &root
                .join(format!("{OTHER} (2)"))
                .join("Italy 1936.battleplan"),
            OTHER,
            2000,
        );
        write(&root.join("notes.txt"), 10);
        // An ended session that left the other campaign waiting.
        let ended = root.join("sessions").join("1-1");
        write(&ended.join(OTHER).join("spool.0000.h4s"), 1000);
        write(&ended.join(OTHER).join("spool.0001.h4s"), 500);
        fs::write(
            ended.join("completed.json"),
            serde_json::to_vec(&serde_json::json!({
                "target": root,
                "waiting": [{"campaign": OTHER, "hours": 5, "first_hour": 10, "last_hour": 14}],
            }))
            .unwrap(),
        )
        .unwrap();
        // An older ended session left the same recording waiting: one row for both.
        let older = root.join("sessions").join("1-0");
        write(&older.join(OTHER).join("spool.0000.h4s"), 200);
        fs::write(
            older.join("completed.json"),
            serde_json::to_vec(&serde_json::json!({
                "target": root,
                "waiting": [{"campaign": OTHER, "hours": 2, "first_hour": 3, "last_hour": 4}],
            }))
            .unwrap(),
        )
        .unwrap();
        // The running session records the first campaign.
        let running = root.join("sessions").join("2-2");
        write(&running.join(CAMPAIGN).join("spool.0000.h4s"), 4000);
        let library = Library::default();
        let flag = FlagImage {
            width: 1,
            height: 1,
            pixels: vec![1, 2, 3, 255],
        };
        let flags = BTreeMap::from([(CAMPAIGN.to_owned(), flag.clone())]);
        let live = Live {
            session: Some(&running),
            target: Some(&root),
            campaign: Some(CAMPAIGN),
            hour: Some(99),
            set_aside: &[],
            names: &BTreeMap::new(),
        };
        let listed = library.list(&settings, &live, &flags);
        assert_eq!(
            listed.unsaved,
            [
                Unsaved {
                    session: running.display().to_string(),
                    campaign: CAMPAIGN.into(),
                    recording: Some(germany.display().to_string()),
                    name: Some("Germany 1936–1941".into()),
                    first_hour: None,
                    last_hour: Some(99),
                    spool_bytes: Some(4000),
                    status: Status::Recording,
                    flag: Some(flag.clone()),
                },
                Unsaved {
                    session: ended.display().to_string(),
                    campaign: OTHER.into(),
                    // It saves into the save folder, where it has no recording yet.
                    recording: None,
                    name: None,
                    first_hour: Some(3),
                    last_hour: Some(14),
                    spool_bytes: Some(1700),
                    status: Status::Waiting,
                    flag: None,
                },
            ]
        );
        let mut sizes: Vec<_> = listed.saved.iter().map(|saved| saved.bytes).collect();
        sizes.sort();
        assert_eq!(sizes, [2000, 3000]);
        // Cut short, so no span is known: it stays unknown, never zero.
        assert!(listed.saved.iter().all(|saved| saved.first_hour.is_none()));
        assert_eq!(
            listed
                .saved
                .iter()
                .find(|saved| saved.bytes == 3000)
                .unwrap()
                .flag,
            Some(flag)
        );
        // A save the ended sessions' records no longer list leaves the list.
        for session in [&ended, &older] {
            fs::write(
                session.join("completed.json"),
                serde_json::to_vec(&serde_json::json!({"target": root, "waiting": []})).unwrap(),
            )
            .unwrap();
        }
        let after = library.list(&settings, &live, &BTreeMap::new());
        assert_eq!(after.unsaved.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_running_sessions_names_are_only_of_its_own_recordings_in_its_folder() {
        let root = scratch();
        let mut settings = RecorderSettings {
            save_folder: Some(root.clone()),
            ..RecorderSettings::default()
        };
        // A Map capture of the campaign moved to a folder of its own.
        let map = format!("{CAMPAIGN} (2)");
        settings.campaigns.insert(
            CAMPAIGN.into(),
            replay::recorder_settings::CampaignChoice {
                folder: Some(map.clone()),
                ..Default::default()
            },
        );
        // A Full capture of 1936 waits in the save folder, which holds no recording of it.
        let ended = root.join("sessions").join("1-1");
        write(&ended.join(CAMPAIGN).join("spool.0000.h4s"), 1000);
        fs::write(
            ended.join("completed.json"),
            serde_json::to_vec(&serde_json::json!({
                "target": root,
                "waiting": [{"campaign": CAMPAIGN, "hours": 5, "first_hour": 10, "last_hour": 14}],
            }))
            .unwrap(),
        )
        .unwrap();
        // The running Map capture, of 1942, records into its folder.
        let running = root.join("sessions").join("2-2");
        write(&running.join(CAMPAIGN).join("spool.0000.h4s"), 4000);
        let target = root.join(&map);
        let names = BTreeMap::from([(CAMPAIGN.to_owned(), "Germany 1942".to_owned())]);
        let live = Live {
            session: Some(&running),
            target: Some(&target),
            campaign: Some(CAMPAIGN),
            hour: Some(99),
            set_aside: &[],
            names: &names,
        };
        let library = Library::default();
        let name = |listed: &Recordings, session: &Path| {
            listed
                .unsaved
                .iter()
                .find(|row| row.session == session.display().to_string())
                .unwrap()
                .name
                .clone()
        };
        let listed = library.list(&settings, &live, &BTreeMap::new());
        assert_eq!(listed.unsaved.len(), 2);
        assert_eq!(name(&listed, &running), Some("Germany 1942".into()));
        // The waiting Full recording's name is not known yet: never the Map capture's.
        assert_eq!(name(&listed, &ended), None);
        // Adding to unsaved hours in its folder, the running capture's own name is not known either.
        let unsaved = replay::publication::unsaved_path(&target, CAMPAIGN);
        recording(&unsaved, CAMPAIGN, 100);
        let listed = library.list(&settings, &live, &BTreeMap::new());
        assert_eq!(name(&listed, &running), None);
        assert_eq!(name(&listed, &ended), None);
        fs::remove_dir_all(root).unwrap();
    }
}
