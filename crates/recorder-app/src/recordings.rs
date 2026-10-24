//! The panel's recordings, listed only while it asks, from directory listings and file metadata.
use crate::{
    flags::{BackgroundPriority, FlagImage},
    targets,
};
use recorder::controller::{self, Waiting, WaitingCampaign, WaitingName, WaitingNames};
use replay::{container::Replay, recorder_settings::RecorderSettings, recording_files};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
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
    /// The name Save gives its file; `None` until known.
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
    /// The file name a save through it would give each recording in `target` now, by campaign.
    pub names: &'a BTreeMap<String, String>,
}

type Span = Option<(i32, i32)>;

/// The file name Save gives the recording each ended session left waiting of a campaign.
pub type Namer = dyn Fn(&[(PathBuf, String)]) -> Vec<WaitingName> + Send + Sync;

/// Saved files' spans and ended sessions' waiting campaigns, read once per file version, and the
/// names waiting recordings get, worked out off the listing's thread.
pub struct Library {
    cache: Mutex<Cache>,
    names: Arc<Mutex<Names>>,
    namer: Option<Arc<Namer>>,
    installation: Arc<dyn Fn() -> Result<(PathBuf, PathBuf), String> + Send + Sync>,
}

impl Default for Library {
    /// Names through each recording's own content stack in the game installation.
    fn default() -> Self {
        Self {
            cache: Mutex::default(),
            names: Arc::default(),
            namer: None,
            installation: Arc::new(WaitingNames::installation_key),
        }
    }
}

#[derive(Default)]
struct Cache {
    spans: HashMap<PathBuf, (u64, Option<SystemTime>, Span)>,
    sessions: HashMap<PathBuf, (Option<(SystemTime, u64)>, Vec<WaitingCampaign>)>,
    /// Each folder's recordings by campaign, read again once its listing changes.
    folders: HashMap<PathBuf, (Listing, recording_files::Found)>,
}

type Listing = Vec<(PathBuf, u64, SystemTime)>;

type Version = Option<(SystemTime, u64)>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct WaitingRevision {
    session: PathBuf,
    files: Vec<(PathBuf, Version)>,
    spools: Option<Listing>,
}

fn waiting_revision(session: &Path, campaign: &str) -> WaitingRevision {
    let mut spools = (|| {
        let mut listing = Vec::new();
        for entry in fs::read_dir(session.join(campaign)).ok()? {
            let entry = entry.ok()?;
            let name = entry.file_name();
            let name = name.to_str()?;
            if name.starts_with("spool.") && name.ends_with(".h4s") {
                let (modified, length) = version(&entry.path())?;
                listing.push((entry.path(), length, modified));
            }
        }
        Some(listing)
    })();
    if let Some(listing) = &mut spools {
        listing.sort();
    }
    WaitingRevision {
        session: session.to_owned(),
        files: [
            PathBuf::from("completed.json"),
            PathBuf::from("admitted.json"),
            PathBuf::from("recordings.json"),
            PathBuf::from("bin/recorder.json"),
            PathBuf::from("bin/collector.json"),
            PathBuf::from(campaign).join("saved.json"),
            PathBuf::from(campaign).join("checkpoint.json"),
        ]
        .into_iter()
        .map(|path| {
            let path = session.join(path);
            let stamp = version(&path);
            (path, stamp)
        })
        .collect(),
        spools,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NameKey {
    recordings: Listing,
    unsaved: Option<Listing>,
    sessions: Vec<WaitingRevision>,
    chosen: bool,
    user_folder: Option<PathBuf>,
    installation: Result<(PathBuf, PathBuf), String>,
    settings: Version,
}

const NAME_RETRY: Duration = Duration::from_secs(30);
const NAME_RETRY_MAX: Duration = Duration::from_secs(600);

struct KnownName {
    key: NameKey,
    name: Option<String>,
    checked: Instant,
    retry: Option<Duration>,
}

impl KnownName {
    fn current(&self, key: &NameKey, now: Instant) -> bool {
        self.key == *key
            && self
                .retry
                .is_none_or(|delay| now.saturating_duration_since(self.checked) < delay)
    }

    fn new(key: NameKey, named: WaitingName, previous: Option<&Self>, now: Instant) -> Self {
        if let Some(previous) = previous
            && previous.current(&key, now)
            && previous.name == named.name
            && previous.retry.is_some() == named.retry
        {
            return Self {
                key,
                name: named.name,
                checked: previous.checked,
                retry: previous.retry,
            };
        }
        let retry = named.retry.then(|| {
            previous
                .filter(|known| known.key == key)
                .and_then(|known| known.retry)
                .map_or(NAME_RETRY, |delay| (delay * 2).min(NAME_RETRY_MAX))
        });
        Self {
            key,
            name: named.name,
            checked: now,
            retry,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct NameScope {
    user_folder: Option<PathBuf>,
    settings: Version,
}

#[derive(Clone, Copy)]
pub struct ViewToken(u64);

#[derive(Default)]
struct PanelVisit {
    visit: u64,
    open: bool,
}

#[derive(Default)]
struct Names {
    panels: HashMap<String, PanelVisit>,
    request: u64,
    scope: Option<NameScope>,
    generation: u64,
    // Mod changes while the view stays open appear when it reopens or on Save.
    content: Arc<Mutex<Option<WaitingNames>>>,
    known: HashMap<(PathBuf, String), KnownName>,
    waiting: Vec<(PathBuf, String)>,
    busy: bool,
}

impl Names {
    fn view(&mut self, scope: Option<NameScope>) {
        if self.scope != scope {
            self.scope = scope;
            self.generation += 1;
            self.content = Arc::default();
            self.known.clear();
            self.waiting.clear();
        }
    }
}

impl Cache {
    /// A folder's listing and its recordings by the campaign their header names; one that cannot
    /// be read names none.
    fn recordings(&mut self, folder: &Path) -> &(Listing, recording_files::Found) {
        let listing = recording_files::listing(folder).unwrap_or_default();
        let known = self.folders.entry(folder.to_path_buf()).or_default();
        if known.0 != listing {
            *known = (
                listing,
                recording_files::recordings(folder).unwrap_or_default(),
            );
        }
        known
    }

    /// The recording a campaign's unsaved hours continue in `folder`: its one recording there.
    fn continued(&mut self, folder: &Path, campaign: &str) -> Option<PathBuf> {
        match self
            .recordings(folder)
            .1
            .get(&campaign.as_bytes().to_ascii_lowercase())
            .map(Vec::as_slice)
        {
            Some([path]) => Some(path.clone()),
            _ => None,
        }
    }
}

fn version(path: &Path) -> Option<(SystemTime, u64)> {
    let metadata = fs::metadata(path).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

impl Library {
    #[cfg(test)]
    pub fn with_namer(namer: Arc<Namer>) -> Self {
        Self {
            cache: Mutex::default(),
            names: Arc::default(),
            namer: Some(namer),
            installation: Arc::new(|| Err("custom recording namer".into())),
        }
    }

    pub fn register_panel(&self, panel: &str) {
        let mut names = self.names.lock().unwrap_or_else(|error| error.into_inner());
        names.panels.entry(panel.to_owned()).or_default();
    }

    pub fn view(&self, panel: &str, visit: u64, open: bool) -> Option<ViewToken> {
        let mut names = self.names.lock().unwrap_or_else(|error| error.into_inner());
        let previous = names.panels.get_mut(panel)?;
        // IPC futures can start out of order; closing a visit always wins over its open requests.
        if visit < previous.visit || (visit == previous.visit && !previous.open) {
            return None;
        }
        let newer = visit > previous.visit;
        *previous = PanelVisit { visit, open };
        if newer || !open {
            names.request += 1;
            names.view(None);
        }
        open.then_some(ViewToken(names.request))
    }

    pub fn close(&self, panel: &str) {
        let mut names = self.names.lock().unwrap_or_else(|error| error.into_inner());
        if names.panels.remove(panel).is_some() {
            names.request += 1;
            names.view(None);
        }
    }

    /// `changed` is called once names worked out off this thread are ready to be listed.
    pub fn list(
        &self,
        settings: &RecorderSettings,
        view: Option<ViewToken>,
        live: &Live<'_>,
        flags: &BTreeMap<String, FlagImage>,
        changed: impl FnOnce() + Send + 'static,
    ) -> Recordings {
        let root = settings.effective_save_folder();
        let root = root.canonicalize().unwrap_or(root);
        let flag = |campaign: &str| {
            flags
                .iter()
                .find(|(known, _)| known.eq_ignore_ascii_case(campaign))
                .map(|(_, flag)| flag.clone())
        };
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        // One row per recording: what the running session and every ended one hold of it.
        let mut unsaved: Vec<(PathBuf, Vec<PathBuf>, Unsaved)> = Vec::new();
        let mut add = |target: &Path, session: &Path, row: Unsaved| match unsaved
            .iter_mut()
            .find(|(known, _, known_row)| known == target && known_row.campaign == row.campaign)
        {
            Some((_, sessions, known)) => {
                known.first_hour = match (known.first_hour, row.first_hour) {
                    (Some(known), Some(first)) => Some(known.min(first)),
                    (known, first) => known.or(first),
                };
                known.last_hour = known.last_hour.max(row.last_hour);
                // One session's spool is counted once.
                if known.session != row.session {
                    known.spool_bytes = known.spool_bytes.zip(row.spool_bytes).map(|(a, b)| a + b);
                    sessions.push(session.to_owned());
                }
            }
            None => unsaved.push((target.to_owned(), vec![session.to_owned()], row)),
        };
        if let Some(session) = live.session {
            // Left for another campaign that has not written an hour yet, it waits rather than records.
            if let Some(campaign) = live.campaign
                && !live
                    .set_aside
                    .iter()
                    .any(|waiting| waiting.campaign == campaign)
            {
                add(
                    &targets::current_folder(settings, &root, campaign),
                    session,
                    Unsaved {
                        session: session.display().to_string(),
                        campaign: campaign.to_owned(),
                        recording: None,
                        name: None,
                        first_hour: None,
                        last_hour: live.hour,
                        spool_bytes: controller::unsaved_spool_bytes(&session.join(campaign)),
                        status: Status::Recording,
                        flag: flag(campaign),
                    },
                );
            }
            for waiting in live.set_aside {
                add(
                    &targets::current_folder(settings, &root, &waiting.campaign),
                    session,
                    Unsaved {
                        session: session.display().to_string(),
                        campaign: waiting.campaign.clone(),
                        recording: None,
                        name: None,
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
                add(
                    &waiting.target,
                    &session,
                    Unsaved {
                        session: session.display().to_string(),
                        campaign: waiting.campaign.clone(),
                        recording: None,
                        name: None,
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
        let settings_stamp = version(&RecorderSettings::path());
        let installation = (self.installation)();
        let mut names = self.names.lock().unwrap_or_else(|error| error.into_inner());
        let view_open = view.is_some_and(|view| view.0 == names.request);
        if view_open {
            names.view(Some(NameScope {
                user_folder: settings.user_folder.clone(),
                settings: settings_stamp,
            }));
        }
        let mut wanted = Vec::new();
        let mut refresh = false;
        let now = Instant::now();
        for (target, sessions, row) in &mut unsaved {
            row.recording = cache
                .continued(target, &row.campaign)
                .map(|path| path.display().to_string());
            // Saved through the running session, they are named as it names them.
            if live.target == Some(target.as_path()) {
                row.name = live.names.get(&row.campaign).cloned();
                continue;
            }
            if !view_open {
                continue;
            }
            sessions.sort();
            let key = NameKey {
                recordings: cache.recordings(target).0.clone(),
                unsaved: recording_files::listing(&replay::publication::unsaved_folder(target))
                    .ok(),
                sessions: sessions
                    .iter()
                    .map(|session| waiting_revision(session, &row.campaign))
                    .collect(),
                chosen: target
                    .canonicalize()
                    .is_ok_and(|folder| settings.name_chosen(&folder, row.campaign.as_bytes())),
                user_folder: settings.user_folder.clone(),
                installation: installation.clone(),
                settings: settings_stamp,
            };
            let id = (target.clone(), row.campaign.clone());
            match names.known.get(&id) {
                Some(known) if known.current(&key, now) => row.name = known.name.clone(),
                _ => refresh = true,
            }
            wanted.push((id, key, PathBuf::from(&row.session)));
        }
        if view_open {
            names.known.retain(|(target, campaign), _| {
                unsaved
                    .iter()
                    .any(|(known, _, row)| known == target && row.campaign == *campaign)
            });
        }
        let asked: Vec<_> = wanted
            .iter()
            .map(|((_, campaign), _, session)| (session.clone(), campaign.clone()))
            .collect();
        if view_open && (refresh || names.waiting != asked) && !names.busy {
            names.busy = true;
            let previous_waiting = std::mem::replace(&mut names.waiting, asked.clone());
            let (shared, namer) = (Arc::clone(&self.names), self.namer.clone());
            let (content, generation) = (Arc::clone(&names.content), names.generation);
            let spawned = std::thread::Builder::new()
                .name("recording-names".into())
                .spawn(move || {
                    let named = match namer {
                        Some(namer) => namer(&asked),
                        None => {
                            let _priority = BackgroundPriority::enter();
                            let mut content = content
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            content
                                .get_or_insert_with(WaitingNames::installed)
                                .predictions(&asked)
                        }
                    };
                    let mut names = shared.lock().unwrap_or_else(|error| error.into_inner());
                    if names.generation == generation {
                        for ((id, key, _), name) in wanted.into_iter().zip(named) {
                            let known =
                                KnownName::new(key, name, names.known.get(&id), Instant::now());
                            names.known.insert(id, known);
                        }
                    }
                    names.busy = false;
                    drop(names);
                    changed();
                });
            if let Err(error) = spawned {
                names.busy = false;
                names.waiting = previous_waiting;
                eprintln!("Waiting recordings' names could not be worked out: {error}");
            }
        }
        drop(names);
        let mut folders: BTreeSet<PathBuf> = unsaved
            .iter()
            .map(|(target, _, _)| target.clone())
            .collect();
        let unsaved = unsaved.into_iter().map(|(_, _, row)| row).collect();
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
                    .1
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

    /// Names each waiting recording by its campaign from `names`, noting what it is asked.
    fn namer(
        names: BTreeMap<String, String>,
        asked: Arc<Mutex<Vec<(PathBuf, String)>>>,
    ) -> Library {
        let library = Library::with_namer(Arc::new(move |waiting: &[(PathBuf, String)]| {
            asked.lock().unwrap().extend(waiting.iter().cloned());
            waiting
                .iter()
                .map(|(_, campaign)| WaitingName {
                    name: names.get(campaign).cloned(),
                    retry: !names.contains_key(campaign),
                })
                .collect()
        }));
        library.register_panel("panel");
        library
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
        let library = namer(BTreeMap::new(), Arc::default());
        let flag = FlagImage {
            width: 1,
            height: 1,
            pixels: vec![1, 2, 3, 255],
        };
        let flags = BTreeMap::from([(CAMPAIGN.to_owned(), flag.clone())]);
        // The name its save gives the file it continues, as the running session works it out.
        let names = BTreeMap::from([(CAMPAIGN.to_owned(), "Germany 1936–1943".to_owned())]);
        let live = Live {
            session: Some(&running),
            target: Some(&root),
            campaign: Some(CAMPAIGN),
            hour: Some(99),
            set_aside: &[],
            names: &names,
        };
        let view = library.view("panel", 1, true);
        let listed = library.list(&settings, view, &live, &flags, || {});
        assert_eq!(
            listed.unsaved,
            [
                Unsaved {
                    session: running.display().to_string(),
                    campaign: CAMPAIGN.into(),
                    recording: Some(germany.display().to_string()),
                    name: Some("Germany 1936–1943".into()),
                    first_hour: None,
                    last_hour: Some(99),
                    spool_bytes: Some(4000),
                    status: Status::Recording,
                    flag: Some(flag.clone()),
                },
                Unsaved {
                    session: ended.display().to_string(),
                    campaign: OTHER.into(),
                    // It saves into the save folder, where it has no recording yet, through the
                    // running session, which has not named it.
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
        let view = library.view("panel", 1, true);
        let after = library.list(&settings, view, &live, &BTreeMap::new(), || {});
        assert_eq!(after.unsaved.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_waiting_recording_is_named_as_its_save_names_it_once_worked_out() {
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
        // A Full capture of 1936 waits in the save folder, its hours staged there.
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
        let staged = replay::publication::unsaved_path(&root, CAMPAIGN);
        recording(&staged, CAMPAIGN, 100);
        // The running Map capture records into its folder, adding to unsaved hours there.
        let running = root.join("sessions").join("2-2");
        write(&running.join(CAMPAIGN).join("spool.0000.h4s"), 4000);
        let target = root.join(&map);
        recording(
            &replay::publication::unsaved_path(&target, CAMPAIGN),
            CAMPAIGN,
            100,
        );
        let names = BTreeMap::from([(CAMPAIGN.to_owned(), "Germany 1936–1942".to_owned())]);
        let live = Live {
            session: Some(&running),
            target: Some(&target),
            campaign: Some(CAMPAIGN),
            hour: Some(99),
            set_aside: &[],
            names: &names,
        };
        let asked = Arc::<Mutex<Vec<(PathBuf, String)>>>::default();
        let library = namer(
            BTreeMap::from([(CAMPAIGN.to_owned(), "Germany 1936".to_owned())]),
            Arc::clone(&asked),
        );
        let name = |listed: &Recordings, session: &Path| {
            listed
                .unsaved
                .iter()
                .find(|row| row.session == session.display().to_string())
                .unwrap()
                .name
                .clone()
        };
        let list = || {
            let (done, ready) = std::sync::mpsc::channel();
            let view = library.view("panel", 1, true);
            let listed = library.list(&settings, view, &live, &BTreeMap::new(), move || {
                done.send(()).unwrap();
            });
            (listed, ready)
        };
        // The running capture's, its unsaved hours and all, is the one it works out.
        let (listed, ready) = list();
        assert_eq!(listed.unsaved.len(), 2);
        assert_eq!(name(&listed, &running), Some("Germany 1936–1942".into()));
        // The waiting Full recording's is not known until worked out: never the Map capture's.
        assert_eq!(name(&listed, &ended), None);
        ready.recv().unwrap();
        let (listed, _) = list();
        assert_eq!(name(&listed, &ended), Some("Germany 1936".into()));
        assert_eq!(
            *asked.lock().unwrap(),
            [(ended.clone(), CAMPAIGN.to_owned())]
        );
        // Worked out again only once what it depends on changes: here its unsaved hours.
        recording(&staged, CAMPAIGN, 200);
        let (listed, ready) = list();
        assert_eq!(name(&listed, &ended), None);
        ready.recv().unwrap();
        let (listed, _) = list();
        assert_eq!(name(&listed, &ended), Some("Germany 1936".into()));
        assert_eq!(asked.lock().unwrap().len(), 2);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn waiting_names_revalidate_resolution_settings_and_spool_journal_revisions() {
        let root = scratch();
        let mut settings = RecorderSettings {
            save_folder: Some(root.clone()),
            ..Default::default()
        };
        let session = root.join("sessions/1-1");
        let spool = session.join(CAMPAIGN).join("spool.0000.h4s");
        write(&spool, 10);
        fs::write(
            session.join("completed.json"),
            serde_json::to_vec(&serde_json::json!({
                "target": root, "waiting": [{"campaign": CAMPAIGN, "hours": 1}],
            }))
            .unwrap(),
        )
        .unwrap();
        let response = Arc::new(Mutex::new(None::<String>));
        let asked = Arc::new(Mutex::new(0));
        let mut library = Library::with_namer({
            let (response, asked) = (Arc::clone(&response), Arc::clone(&asked));
            Arc::new(move |waiting| {
                *asked.lock().unwrap() += 1;
                waiting
                    .iter()
                    .map(|_| {
                        let name = response.lock().unwrap().clone();
                        let retry = name.as_deref().is_none_or(|name| name == "Z01");
                        WaitingName { name, retry }
                    })
                    .collect()
            })
        });
        library.register_panel("panel");
        let installation = Arc::new(Mutex::new(Ok((root.join("game"), root.join("user")))));
        library.installation = {
            let installation = Arc::clone(&installation);
            Arc::new(move || installation.lock().unwrap().clone())
        };
        let live = Live {
            session: None,
            target: None,
            campaign: None,
            hour: None,
            set_aside: &[],
            names: &BTreeMap::new(),
        };
        let visit = std::cell::Cell::new(1);
        let refresh = |settings: &RecorderSettings, expected: Option<&str>| {
            let (done, ready) = std::sync::mpsc::channel();
            let view = library.view("panel", visit.get(), true);
            let listed = library.list(settings, view, &live, &BTreeMap::new(), move || {
                done.send(()).unwrap();
            });
            assert_eq!(listed.unsaved[0].name, None);
            ready.recv().unwrap();
            {
                let mut names = library.names.lock().unwrap();
                for known in names.known.values_mut() {
                    known.checked = Instant::now() + Duration::from_secs(86_400);
                }
            }
            let view = library.view("panel", visit.get(), true);
            let listed = library.list(settings, view, &live, &BTreeMap::new(), || {});
            assert_eq!(listed.unsaved[0].name.as_deref(), expected);
        };
        refresh(&settings, None);
        *response.lock().unwrap() = Some("Z01".into());
        write(&spool, 20);
        refresh(&settings, Some("Z01"));
        *response.lock().unwrap() = Some("Named country".into());
        settings.user_folder = Some(root.join("other-user"));
        refresh(&settings, Some("Named country"));
        *installation.lock().unwrap() = Ok((root.join("other-game"), root.join("user")));
        refresh(&settings, Some("Named country"));
        write(&session.join(CAMPAIGN).join("saved.json"), 10);
        refresh(&settings, Some("Named country"));
        write(&session.join(CAMPAIGN).join("checkpoint.json"), 10);
        refresh(&settings, Some("Named country"));
        *response.lock().unwrap() = Some("Z01".into());
        write(&session.join(CAMPAIGN).join("saved.json"), 20);
        refresh(&settings, Some("Z01"));
        // A repaired content installation is retried without waiting on a timer in this test.
        *response.lock().unwrap() = Some("Repaired country".into());
        for known in library.names.lock().unwrap().known.values_mut() {
            known.checked = Instant::now() - NAME_RETRY;
        }
        refresh(&settings, Some("Repaired country"));
        *response.lock().unwrap() = Some("Changed installed content".into());
        for known in library.names.lock().unwrap().known.values_mut() {
            known.checked = Instant::now() - NAME_RETRY_MAX;
        }
        let view = library.view("panel", visit.get(), true);
        let listed = library.list(&settings, view, &live, &BTreeMap::new(), || {});
        assert_eq!(listed.unsaved[0].name.as_deref(), Some("Repaired country"));
        assert_eq!(*asked.lock().unwrap(), 8);
        let _ = library.view("panel", visit.get(), false);
        visit.set(visit.get() + 1);
        refresh(&settings, Some("Changed installed content"));
        assert_eq!(*asked.lock().unwrap(), 9);
        fs::write(
            session.join("completed.json"),
            serde_json::to_vec(&serde_json::json!({
                "target": root, "waiting": []
            }))
            .unwrap(),
        )
        .unwrap();
        let (done, ready) = std::sync::mpsc::channel();
        let view = library.view("panel", visit.get(), true);
        let listed = library.list(&settings, view, &live, &BTreeMap::new(), move || {
            done.send(()).unwrap();
        });
        assert!(listed.unsaved.is_empty());
        ready.recv().unwrap();
        assert_eq!(*asked.lock().unwrap(), 10);
        assert!(library.names.lock().unwrap().known.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn naming_cache_reuses_the_open_view_and_drops_on_close_or_settings_change() {
        let mut names = Names::default();
        let mut scope = NameScope {
            user_folder: Some(PathBuf::from("synthetic-user")),
            settings: Some((std::time::UNIX_EPOCH, 1)),
        };
        names.view(Some(scope.clone()));
        *names.content.lock().unwrap() = Some(WaitingNames::new(Err("synthetic content".into())));
        let content = Arc::downgrade(&names.content);
        let generation = names.generation;
        names.view(Some(scope.clone()));
        assert_eq!(names.generation, generation);
        assert!(Arc::ptr_eq(&content.upgrade().unwrap(), &names.content));
        assert!(names.content.lock().unwrap().is_some());
        names.view(None);
        assert!(content.upgrade().is_none());
        assert!(names.content.lock().unwrap().is_none());
        assert!(names.generation > generation);
        for change in ["reopened", "settings", "user-folder"] {
            let content = Arc::downgrade(&names.content);
            let generation = names.generation;
            match change {
                "settings" => scope.settings = Some((std::time::UNIX_EPOCH, 2)),
                "user-folder" => scope.user_folder = Some(PathBuf::from("other-user")),
                _ => {}
            }
            names.view(Some(scope.clone()));
            assert!(content.upgrade().is_none(), "{change}");
            assert!(names.content.lock().unwrap().is_none(), "{change}");
            assert!(names.generation > generation, "{change}");
            *names.content.lock().unwrap() = Some(WaitingNames::new(Err(change.into())));
        }
    }

    #[test]
    fn a_closed_views_naming_job_cannot_restore_its_names() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc,
        };
        let root = scratch();
        let settings = RecorderSettings {
            save_folder: Some(root.clone()),
            ..Default::default()
        };
        let session = root.join("sessions/1-1");
        write(&session.join(CAMPAIGN).join("spool.0000.h4s"), 10);
        fs::write(
            session.join("completed.json"),
            serde_json::to_vec(&serde_json::json!({
                "target": root, "waiting": [{"campaign": CAMPAIGN, "hours": 1}],
            }))
            .unwrap(),
        )
        .unwrap();
        let (entered, started) = mpsc::channel();
        let (resume, paused) = mpsc::channel();
        let paused = Mutex::new(paused);
        let calls = Arc::new(AtomicUsize::new(0));
        let library = Library::with_namer({
            let calls = Arc::clone(&calls);
            Arc::new(move |waiting| {
                let first = calls.fetch_add(1, Ordering::Relaxed) == 0;
                if first {
                    entered.send(()).unwrap();
                    paused.lock().unwrap().recv().unwrap();
                }
                waiting
                    .iter()
                    .map(|_| WaitingName {
                        name: Some(
                            if first {
                                "Closed view"
                            } else {
                                "Reopened view"
                            }
                            .into(),
                        ),
                        retry: false,
                    })
                    .collect()
            })
        });
        library.register_panel("panel");
        let live = Live {
            session: None,
            target: None,
            campaign: None,
            hour: None,
            set_aside: &[],
            names: &BTreeMap::new(),
        };
        let visit = std::cell::Cell::new(1);
        let list = || {
            let (done, ready) = mpsc::channel();
            let view = library.view("panel", visit.get(), true);
            let listed = library.list(&settings, view, &live, &BTreeMap::new(), move || {
                done.send(()).unwrap();
            });
            (listed, ready)
        };
        let (listed, ready) = list();
        assert_eq!(listed.unsaved[0].name, None);
        started.recv().unwrap();
        let _ = library.view("panel", visit.get(), false);
        visit.set(visit.get() + 1);
        let view = library.view("panel", visit.get(), true);
        let listed = library.list(&settings, view, &live, &BTreeMap::new(), || {
            panic!("job still busy")
        });
        assert_eq!(listed.unsaved[0].name, None);
        resume.send(()).unwrap();
        ready.recv().unwrap();
        assert!(library.names.lock().unwrap().known.is_empty());
        let (listed, ready) = list();
        assert_eq!(listed.unsaved[0].name, None);
        ready.recv().unwrap();
        let view = library.view("panel", visit.get(), true);
        let listed = library.list(&settings, view, &live, &BTreeMap::new(), || {
            panic!("name already cached")
        });
        assert_eq!(listed.unsaved[0].name.as_deref(), Some("Reopened view"));
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        let view = library.view("panel", visit.get(), false);
        let listed = library.list(&settings, view, &live, &BTreeMap::new(), || {
            panic!("view closed")
        });
        assert_eq!(listed.unsaved[0].name, None);
        let names = library.names.lock().unwrap();
        assert!(names.scope.is_none());
        assert!(names.known.is_empty());
        assert!(names.content.lock().unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_listing_paused_before_naming_cannot_reopen_a_closed_view() {
        use std::sync::mpsc;

        let root = scratch();
        let settings = RecorderSettings {
            save_folder: Some(root.clone()),
            ..Default::default()
        };
        let session = root.join("sessions/1-1");
        write(&session.join(CAMPAIGN).join("spool.0000.h4s"), 10);
        fs::write(
            session.join("completed.json"),
            serde_json::to_vec(&serde_json::json!({
                "target": root, "waiting": [{"campaign": CAMPAIGN, "hours": 1}],
            }))
            .unwrap(),
        )
        .unwrap();
        let asked = Arc::default();
        let mut library = namer(BTreeMap::new(), Arc::clone(&asked));
        let (entered, started) = mpsc::channel();
        let (resume, paused) = mpsc::channel();
        let paused = Mutex::new(paused);
        library.installation = Arc::new(move || {
            entered.send(()).unwrap();
            paused.lock().unwrap().recv().unwrap();
            Err("synthetic installation".into())
        });
        let view = library.view("panel", 1, true);
        let cached = {
            let mut names = library.names.lock().unwrap();
            names.view(Some(NameScope {
                user_folder: settings.user_folder.clone(),
                settings: None,
            }));
            *names.content.lock().unwrap() =
                Some(WaitingNames::new(Err("synthetic content".into())));
            Arc::downgrade(&names.content)
        };
        std::thread::scope(|threads| {
            let listing = threads.spawn(|| {
                let live = Live {
                    session: None,
                    target: None,
                    campaign: None,
                    hour: None,
                    set_aside: &[],
                    names: &BTreeMap::new(),
                };
                library.list(&settings, view, &live, &BTreeMap::new(), || {
                    panic!("closed view must not start naming")
                })
            });
            started.recv().unwrap();
            library.close("panel");
            let (generation, content) = {
                let names = library.names.lock().unwrap();
                (names.generation, Arc::downgrade(&names.content))
            };
            resume.send(()).unwrap();
            let listed = listing.join().unwrap();
            assert!(cached.upgrade().is_none());
            assert_eq!(listed.unsaved.len(), 1);
            assert_eq!(listed.unsaved[0].name, None);
            let names = library.names.lock().unwrap();
            assert_eq!(names.generation, generation);
            assert!(names.scope.is_none());
            assert!(names.known.is_empty());
            assert!(names.waiting.is_empty());
            assert!(!names.busy);
            assert!(Arc::ptr_eq(&content.upgrade().unwrap(), &names.content));
            assert!(names.content.lock().unwrap().is_none());
            assert!(asked.lock().unwrap().is_empty());
        });
        fs::remove_dir_all(root).unwrap();
    }

    fn delayed_open_after_close(destroyed: bool) {
        let root = scratch();
        let settings = RecorderSettings {
            save_folder: Some(root.clone()),
            ..Default::default()
        };
        let session = root.join("sessions/1-1");
        write(&session.join(CAMPAIGN).join("spool.0000.h4s"), 10);
        fs::write(
            session.join("completed.json"),
            serde_json::to_vec(&serde_json::json!({
                "target": root, "waiting": [{"campaign": CAMPAIGN, "hours": 1}],
            }))
            .unwrap(),
        )
        .unwrap();
        let asked = Arc::default();
        let library = namer(BTreeMap::new(), Arc::clone(&asked));
        let live = Live {
            session: None,
            target: None,
            campaign: None,
            hour: None,
            set_aside: &[],
            names: &BTreeMap::new(),
        };
        if destroyed {
            library.close("panel");
        } else {
            assert!(library.view("panel", 1, false).is_none());
        }
        let (generation, content) = {
            let names = library.names.lock().unwrap();
            (names.generation, Arc::downgrade(&names.content))
        };
        // This open was dispatched before the close, but its async body has not run until now.
        let delayed = library.view("panel", 1, true);
        assert!(delayed.is_none());
        let listed = library.list(&settings, delayed, &live, &BTreeMap::new(), || {
            panic!("a delayed open must not start naming")
        });
        assert_eq!(listed.unsaved.len(), 1);
        assert_eq!(listed.unsaved[0].name, None);
        if destroyed {
            assert!(library.view("panel", 2, true).is_none());
            assert!(library.view("panel", 2, false).is_none());
            assert!(library.names.lock().unwrap().panels.is_empty());
        }
        {
            let names = library.names.lock().unwrap();
            assert_eq!(names.generation, generation);
            assert!(names.scope.is_none());
            assert!(names.known.is_empty());
            assert!(names.waiting.is_empty());
            assert!(!names.busy);
            assert!(Arc::ptr_eq(&content.upgrade().unwrap(), &names.content));
            assert!(names.content.lock().unwrap().is_none());
        }
        assert!(asked.lock().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_open_body_delayed_until_after_its_close_cannot_open_the_view() {
        delayed_open_after_close(false);
    }

    #[test]
    fn an_open_body_delayed_until_after_panel_destruction_cannot_open_the_view() {
        delayed_open_after_close(true);
    }

    #[test]
    fn out_of_order_visit_bodies_and_listings_preserve_the_latest_view() {
        let root = scratch();
        let settings = RecorderSettings {
            save_folder: Some(root.clone()),
            ..Default::default()
        };
        let session = root.join("sessions/1-1");
        write(&session.join(CAMPAIGN).join("spool.0000.h4s"), 10);
        fs::write(
            session.join("completed.json"),
            serde_json::to_vec(&serde_json::json!({
                "target": root, "waiting": [{"campaign": CAMPAIGN, "hours": 1}],
            }))
            .unwrap(),
        )
        .unwrap();
        let asked = Arc::default();
        let library = namer(
            BTreeMap::from([(CAMPAIGN.to_owned(), "Current view".to_owned())]),
            Arc::clone(&asked),
        );
        let live = Live {
            session: None,
            target: None,
            campaign: None,
            hour: None,
            set_aside: &[],
            names: &BTreeMap::new(),
        };
        let old_open = library.view("panel", 1, true);
        let old_close = library.view("panel", 1, false);
        let latest = library.view("panel", 2, true);
        let listed = library.list(&settings, old_open, &live, &BTreeMap::new(), || {
            panic!("superseded open must not start naming")
        });
        assert_eq!(listed.unsaved[0].name, None);
        {
            let names = library.names.lock().unwrap();
            assert!(names.scope.is_none());
            assert!(names.known.is_empty());
            assert!(names.waiting.is_empty());
            assert!(!names.busy);
            assert!(names.content.lock().unwrap().is_none());
        }
        let (done, ready) = std::sync::mpsc::channel();
        let listed = library.list(&settings, latest, &live, &BTreeMap::new(), move || {
            done.send(()).unwrap();
        });
        assert_eq!(listed.unsaved[0].name, None);
        ready.recv().unwrap();
        let (generation, content) = {
            let names = library.names.lock().unwrap();
            *names.content.lock().unwrap() =
                Some(WaitingNames::new(Err("synthetic content".into())));
            (names.generation, Arc::downgrade(&names.content))
        };
        let listed = library.list(&settings, old_close, &live, &BTreeMap::new(), || {
            panic!("superseded close must not start naming")
        });
        assert_eq!(listed.unsaved[0].name, None);
        {
            let names = library.names.lock().unwrap();
            assert_eq!(names.generation, generation);
            assert!(names.scope.is_some());
            assert!(Arc::ptr_eq(&content.upgrade().unwrap(), &names.content));
            assert!(names.content.lock().unwrap().is_some());
        }
        for visit in [1, 0] {
            for open in [true, false] {
                let delayed = library.view("panel", visit, open);
                assert!(delayed.is_none());
                let listed = library.list(&settings, delayed, &live, &BTreeMap::new(), || {
                    panic!("an older visit must not start naming")
                });
                assert_eq!(listed.unsaved[0].name, None);
                let names = library.names.lock().unwrap();
                assert_eq!(names.generation, generation);
                assert!(names.scope.is_some());
                assert!(Arc::ptr_eq(&content.upgrade().unwrap(), &names.content));
                assert!(names.content.lock().unwrap().is_some());
            }
        }
        let listed = library.list(&settings, latest, &live, &BTreeMap::new(), || {
            panic!("latest view must retain its cached name")
        });
        assert_eq!(listed.unsaved[0].name.as_deref(), Some("Current view"));
        assert_eq!(*asked.lock().unwrap(), [(session, CAMPAIGN.to_owned())]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn same_visit_refresh_bodies_in_reverse_order_keep_cached_names() {
        let root = scratch();
        let settings = RecorderSettings {
            save_folder: Some(root.clone()),
            ..Default::default()
        };
        let session = root.join("sessions/1-1");
        write(&session.join(CAMPAIGN).join("spool.0000.h4s"), 10);
        fs::write(
            session.join("completed.json"),
            serde_json::to_vec(&serde_json::json!({
                "target": root, "waiting": [{"campaign": CAMPAIGN, "hours": 1}],
            }))
            .unwrap(),
        )
        .unwrap();
        let asked = Arc::default();
        let library = namer(
            BTreeMap::from([(CAMPAIGN.to_owned(), "Cached name".to_owned())]),
            Arc::clone(&asked),
        );
        let live = Live {
            session: None,
            target: None,
            campaign: None,
            hour: None,
            set_aside: &[],
            names: &BTreeMap::new(),
        };
        let opened = library.view("panel", 1, true).unwrap();
        let (done, ready) = std::sync::mpsc::channel();
        let listed = library.list(
            &settings,
            Some(opened),
            &live,
            &BTreeMap::new(),
            move || {
                done.send(()).unwrap();
            },
        );
        assert_eq!(listed.unsaved[0].name, None);
        ready.recv().unwrap();

        let refresh_a = || library.view("panel", 1, true);
        let refresh_b = || library.view("panel", 1, true);
        // A was dispatched first, but B's async body runs before A's.
        let view_b = refresh_b();
        let view_a = refresh_a();
        for view in [view_b, view_a] {
            let listed = library.list(&settings, view, &live, &BTreeMap::new(), || {
                panic!("same-visit refresh must retain its cached name")
            });
            assert_eq!(listed.unsaved.len(), 1);
            assert_eq!(listed.unsaved[0].name.as_deref(), Some("Cached name"));
            assert_eq!(view.unwrap().0, opened.0);
        }
        assert!(!library.names.lock().unwrap().busy);
        assert_eq!(*asked.lock().unwrap(), [(session, CAMPAIGN.to_owned())]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_new_panel_starts_its_own_visits_and_rejects_the_destroyed_panels_requests() {
        let library = Library::default();
        library.register_panel("old-panel");
        assert!(library.view("old-panel", 10, true).is_some());
        library.close("old-panel");
        library.register_panel("new-panel");
        let current = library.view("new-panel", 1, true).unwrap();
        assert!(library.view("old-panel", 11, true).is_none());
        assert!(library.view("old-panel", 11, false).is_none());
        let names = library.names.lock().unwrap();
        assert_eq!(current.0, names.request);
        assert_eq!(names.panels.len(), 1);
        assert_eq!(names.panels["new-panel"].visit, 1);
        assert!(names.panels["new-panel"].open);
    }

    #[test]
    fn successful_names_stay_current_and_failures_keep_bounded_backoff() {
        let mut key = NameKey {
            recordings: vec![],
            unsaved: None,
            sessions: vec![],
            chosen: false,
            user_folder: None,
            installation: Err("synthetic missing mod".into()),
            settings: None,
        };
        let mut now = Instant::now();
        let success = KnownName::new(
            key.clone(),
            WaitingName {
                name: Some("Named country".into()),
                retry: false,
            },
            None,
            now,
        );
        assert!(success.current(&key, now + NAME_RETRY));
        assert!(success.current(&key, now + NAME_RETRY_MAX * 100));
        for name in [None, Some("Z01".to_owned())] {
            let mut known = KnownName::new(
                key.clone(),
                WaitingName {
                    name: name.clone(),
                    retry: true,
                },
                None,
                now,
            );
            for delay in [30, 60, 120, 240, 480, 600, 600] {
                let delay = Duration::from_secs(delay);
                assert_eq!(known.retry, Some(delay));
                assert!(known.current(&key, now + delay - Duration::from_nanos(1)));
                now += delay;
                assert!(!known.current(&key, now));
                known = KnownName::new(
                    key.clone(),
                    WaitingName {
                        name: name.clone(),
                        retry: true,
                    },
                    Some(&known),
                    now,
                );
            }
            key.chosen = !key.chosen;
            assert!(!known.current(&key, now));
            let changed = KnownName::new(
                key.clone(),
                WaitingName { name, retry: true },
                Some(&known),
                now,
            );
            assert_eq!(changed.retry, Some(NAME_RETRY));
        }
    }
}
