use crate::flags::{BackgroundPriority, FlagImage};
use recorder::controller::{self, Waiting, WaitingCampaign, WaitingName, WaitingNames};
use replay::campaign::Replay;
use replay_settings::recorder_settings::{self, RecorderSettings};
use replay_files::recording_files;
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

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unsaved {
    pub session: String,
    pub campaign: String,
    pub recording: Option<String>,
    pub name: Option<String>,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
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
    pub bytes: u64,
    pub flag: Option<FlagImage>,
    pub level: Option<i32>,
    pub job: Option<crate::compress::Job>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unreadable {
    pub whole: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recordings {
    pub unsaved: Vec<Unsaved>,
    pub saved: Vec<Saved>,
    pub unreadable: Option<Unreadable>,
}

pub struct Live<'a> {
    pub session: Option<&'a Path>,
    pub target: Option<&'a Path>,
    pub campaign: Option<&'a str>,
    pub hour: Option<i32>,
    pub set_aside: &'a [Waiting],
    pub names: &'a BTreeMap<String, String>,
}

type Span = Option<(i32, i32)>;

pub type Namer = dyn Fn(&[(PathBuf, String)]) -> Vec<WaitingName> + Send + Sync;

pub struct Library {
    cache: Mutex<Cache>,
    names: Arc<Mutex<Names>>,
    namer: Option<Arc<Namer>>,
    installation: Arc<dyn Fn() -> Result<(PathBuf, PathBuf), String> + Send + Sync>,
    clock: Arc<dyn Fn() -> Instant + Send + Sync>,
    settings: PathBuf,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            cache: Mutex::default(),
            names: Arc::default(),
            namer: None,
            installation: Arc::new(WaitingNames::installation_key),
            clock: Arc::new(Instant::now),
            settings: RecorderSettings::path(),
        }
    }
}

#[derive(Default)]
struct Cache {
    spans: HashMap<PathBuf, (u64, Option<SystemTime>, Span, Option<i32>)>,
    sessions: HashMap<PathBuf, (Option<(SystemTime, u64)>, Vec<WaitingCampaign>)>,
    folders: HashMap<PathBuf, (Listing, recording_files::Found)>,
}

type Listing = Vec<(PathBuf, u64, SystemTime)>;

type Version = Option<(SystemTime, u64)>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct WaitingRevision {
    session: PathBuf,
    files: Vec<(PathBuf, Version)>,
}

fn waiting_revision(session: &Path) -> WaitingRevision {
    WaitingRevision {
        session: session.to_owned(),
        files: [
            "completed.json",
            "admitted.json",
            "bin/recorder.json",
            "bin/collector.json",
        ]
        .into_iter()
        .map(|path| {
            let path = session.join(path);
            let stamp = version(&path);
            (path, stamp)
        })
        .collect(),
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

fn settled(root: &Path) -> bool {
    if !root.is_dir() {
        return false;
    }
    let entries = match fs::read_dir(root.join("sessions")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
        Err(_) => return false,
    };
    for entry in entries {
        let Ok(entry) = entry else {
            return false;
        };
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => {}
            Ok(_) => continue,
            Err(_) => return false,
        }
        let record = fs::read(entry.path().join("completed.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        let none_waiting = record
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .is_some_and(|record| match record.get("waiting") {
                None => true,
                Some(waiting) => waiting.as_array().is_some_and(Vec::is_empty),
            });
        if !none_waiting {
            return false;
        }
    }
    true
}

fn forget_settled(path: &Path, folders: &[PathBuf]) -> std::io::Result<()> {
    RecorderSettings::update_at(path, |settings| {
        for folder in folders {
            let root = folder.canonicalize().unwrap_or_else(|_| folder.clone());
            if settled(&root) {
                settings.forget_save_folder(folder);
            }
        }
    })
    .map(|applied| drop(applied.logged("Earlier save folders")))
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
            clock: Arc::new(Instant::now),
            settings: std::env::temp_dir().join(format!(
                "hoi4-library-settings-{}-{}.json",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )),
        }
    }

    #[cfg(test)]
    pub fn with_settings(mut self, path: PathBuf) -> Self {
        self.settings = path;
        self
    }

    pub fn register_panel(&self, panel: &str) {
        let mut names = self.names.lock().unwrap_or_else(|error| error.into_inner());
        names.panels.entry(panel.to_owned()).or_default();
    }

    pub fn view(&self, panel: &str, visit: u64, open: bool) -> Option<ViewToken> {
        let mut names = self.names.lock().unwrap_or_else(|error| error.into_inner());
        let previous = names.panels.get_mut(panel)?;
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

    pub fn list(
        &self,
        settings: &RecorderSettings,
        view: Option<ViewToken>,
        live: &Live<'_>,
        flags: &BTreeMap<String, FlagImage>,
        changed: impl FnOnce() + Send + 'static,
    ) -> Recordings {
        let root = settings.effective_save_folder();
        let made_when_needed = recorder_settings::same_path(&root, &recorder_settings::default_save_folder());
        let root = root.canonicalize().unwrap_or(root);
        let flag = |campaign: &str| {
            flags
                .iter()
                .find(|(known, _)| known.eq_ignore_ascii_case(campaign))
                .map(|(_, flag)| flag.clone())
        };
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
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
                if known.session != row.session {
                    known.spool_bytes = known.spool_bytes.zip(row.spool_bytes).map(|(a, b)| a + b);
                    sessions.push(session.to_owned());
                }
            }
            None => unsaved.push((target.to_owned(), vec![session.to_owned()], row)),
        };
        if let (Some(session), Some(target)) = (live.session, live.target) {
            if let Some(campaign) = live.campaign
                && !live
                    .set_aside
                    .iter()
                    .any(|waiting| waiting.campaign == campaign)
            {
                add(
                    target,
                    session,
                    Unsaved {
                        session: session.display().to_string(),
                        campaign: campaign.to_owned(),
                        recording: None,
                        name: None,
                        first_hour: None,
                        last_hour: live.hour,
                        spool_bytes: controller::unsaved_bytes(session, target, campaign),
                        status: Status::Recording,
                        flag: flag(campaign),
                    },
                );
            }
            for waiting in live.set_aside {
                add(
                    target,
                    session,
                    Unsaved {
                        session: session.display().to_string(),
                        campaign: waiting.campaign.clone(),
                        recording: None,
                        name: None,
                        first_hour: waiting.unsaved.first_hour,
                        last_hour: waiting.unsaved.last_hour,
                        spool_bytes: controller::unsaved_bytes(session, target, &waiting.campaign),
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
        let roots: Vec<(PathBuf, PathBuf)> = settings
            .session_roots()
            .into_iter()
            .map(|root| (root.canonicalize().unwrap_or_else(|_| root.clone()), root))
            .collect();
        let mut sessions: Vec<PathBuf> = roots
            .iter()
            .flat_map(|(root, _)| controller::session_folders(root))
            .collect();
        sessions.sort_by(|left, right| {
            right
                .file_name()
                .cmp(&left.file_name())
                .then_with(|| right.cmp(left))
        });
        cache
            .sessions
            .retain(|session, _| sessions.contains(session));
        for session in &sessions {
            if Some(session.as_path()) == live.session {
                continue;
            }
            let stamp = version(&session.join("completed.json"));
            let entry = cache
                .sessions
                .entry(session.clone())
                .or_insert_with(|| (None, Vec::new()));
            if entry.0 != stamp || stamp.is_none() {
                *entry = (stamp, controller::waiting_campaigns(session));
            }
            for waiting in entry.1.clone() {
                add(
                    &waiting.target,
                    session,
                    Unsaved {
                        session: session.display().to_string(),
                        campaign: waiting.campaign.clone(),
                        recording: None,
                        name: None,
                        first_hour: waiting.first_hour,
                        last_hour: waiting.last_hour,
                        spool_bytes: controller::unsaved_bytes(
                            session,
                            &waiting.target,
                            &waiting.campaign,
                        ),
                        status: Status::Waiting,
                        flag: flag(&waiting.campaign),
                    },
                );
            }
        }
        let forgotten: Vec<PathBuf> = roots
            .iter()
            .skip(1)
            .filter(|(root, _)| {
                live.session.is_none_or(|live| !live.starts_with(root)) && settled(root)
            })
            .map(|(_, chosen)| chosen.clone())
            .collect();
        if !forgotten.is_empty()
            && let Err(error) = forget_settled(&self.settings, &forgotten)
        {
            eprintln!("Earlier save folders could not be forgotten: {error}");
        }
        let settings_stamp = version(&RecorderSettings::path());
        let installation = (self.installation)();
        let keys: Vec<Option<NameKey>> = unsaved
            .iter_mut()
            .map(|(target, sessions, row)| {
                row.recording = cache
                    .continued(target, &row.campaign)
                    .map(|path| path.display().to_string());
                if live.target == Some(target.as_path()) {
                    row.name = live.names.get(&row.campaign).cloned();
                    return None;
                }
                view.as_ref()?;
                sessions.sort();
                Some(NameKey {
                    recordings: cache.recordings(target).0.clone(),
                    unsaved: recording_files::listing(&replay_settings::publication::unsaved_folder(target))
                        .ok(),
                    sessions: sessions
                        .iter()
                        .map(|session| waiting_revision(session))
                        .collect(),
                    chosen: target
                        .canonicalize()
                        .is_ok_and(|folder| settings.name_chosen(&folder, row.campaign.as_bytes())),
                    user_folder: settings.user_folder.clone(),
                    installation: installation.clone(),
                    settings: settings_stamp,
                })
            })
            .collect();
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
        let now = (self.clock)();
        for ((target, _, row), key) in unsaved.iter_mut().zip(keys) {
            let Some(key) = key.filter(|_| view_open) else {
                continue;
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
            let clock = Arc::clone(&self.clock);
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
                                KnownName::new(key, name, names.known.get(&id), clock());
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
        let (files, unreadable) = saved_files(&root, made_when_needed);
        for path in files {
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            let version = (metadata.len(), metadata.modified().ok());
            let (span, level) = match cache.spans.get(&path) {
                Some((bytes, time, span, level)) if (*bytes, *time) == version => (*span, *level),
                _ => {
                    let span = Replay::read_hour_span(&path).ok().flatten();
                    let level = replay_files::recompress::level_of(&path).ok().flatten();
                    cache
                        .spans
                        .insert(path.clone(), (version.0, version.1, span, level));
                    (span, level)
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
                    level,
                    job: None,
                },
            ));
        }
        let listed: Vec<_> = saved
            .iter()
            .map(|(_, saved)| PathBuf::from(&saved.path))
            .collect();
        cache.spans.retain(|path, _| listed.contains(path));
        cache.folders.retain(|folder, _| folders.contains(folder));
        saved.sort_by(|left, right| right.0.cmp(&left.0));
        Recordings {
            unsaved,
            saved: saved.into_iter().map(|(_, saved)| saved).collect(),
            unreadable,
        }
    }
}

fn hidden(entry: &fs::DirEntry) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    entry
        .metadata()
        .is_ok_and(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
}

fn recording_folder(entry: &fs::DirEntry) -> bool {
    let name = entry.file_name();
    name != "sessions" && !name.to_string_lossy().starts_with('.') && !hidden(entry)
}

pub(crate) fn recording_folders(root: &Path) -> Vec<PathBuf> {
    let mut folders = vec![root.to_path_buf()];
    for entry in fs::read_dir(root).into_iter().flatten().flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) && recording_folder(&entry) {
            folders.push(entry.path());
        }
    }
    folders
}

fn recordings_in(folder: &Path, files: &mut Vec<PathBuf>) -> Option<std::io::Error> {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) => return Some(error),
    };
    let mut failed = None;
    for entry in entries {
        match entry.and_then(|entry| Ok((entry.file_type()?, entry.path()))) {
            Ok((kind, path)) if kind.is_file() && recording_files::is_recording(&path) => files.push(path),
            Ok(_) => {}
            Err(error) => {
                failed.get_or_insert(error);
            }
        }
    }
    failed
}

pub(crate) fn saved_files(root: &Path, made_when_needed: bool) -> (Vec<PathBuf>, Option<Unreadable>) {
    let mut files = Vec::new();
    let failure = |whole: bool, folder: &Path, error: std::io::Error| Unreadable {
        whole,
        detail: format!("{}: {error}", folder.display()),
    };
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if made_when_needed && error.kind() == std::io::ErrorKind::NotFound => {
            return (files, None);
        }
        Err(error) => return (files, Some(failure(true, root, error))),
    };
    let mut unreadable = None;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                unreadable.get_or_insert_with(|| failure(false, root, error));
                continue;
            }
        };
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_file() && recording_files::is_recording(&path) => files.push(path),
            Ok(kind) if kind.is_dir() && recording_folder(&entry) => {
                if let Some(error) = recordings_in(&path, &mut files) {
                    unreadable.get_or_insert_with(|| failure(false, &path, error));
                }
            }
            Ok(_) => {}
            Err(error) => {
                unreadable.get_or_insert_with(|| failure(false, &path, error));
            }
        }
    }
    (files, unreadable)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAMPAIGN: &str = "00000000-0000-4000-8000-000000000000";
    const OTHER: &str = "00000000-0000-4000-8000-000000000001";

    fn scratch() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "battleplan-recorder-recordings-{}-{}",
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

    fn journal(campaign: &Path, segment: u32, bytes: usize) {
        let mut name = [0; replay_journal::columnar_journal::SEGMENT_NAME_BYTES];
        write(
            &campaign.join(replay_journal::columnar_journal::segment_name(
                0, segment, &mut name,
            )),
            bytes,
        );
    }

    fn recording(path: &Path, campaign: &str, bytes: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        crate::targets::tests::recording_file(path, campaign);
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(bytes)
            .unwrap();
    }

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
    fn hours_waiting_in_an_earlier_save_folder_stay_listed_until_none_waits() {
        let root = scratch();
        let (earlier, current) = (root.join("earlier"), root.join("current"));
        let gone = root.join("gone");
        fs::create_dir_all(&current).unwrap();
        let path = root.join("settings.json");
        let settings = RecorderSettings {
            save_folder: Some(current.clone()),
            earlier_save_folders: vec![earlier.clone(), gone.clone()],
            ..RecorderSettings::default()
        };
        let saved = settings.clone();
        let _ = RecorderSettings::update_at(&path, move |held| *held = saved).unwrap();
        let ended = earlier.join("sessions").join("1-1");
        journal(&ended.join(OTHER), 0, 100);
        let waiting = |waiting: serde_json::Value| {
            fs::write(
                ended.join("completed.json"),
                serde_json::to_vec(&serde_json::json!({"target": earlier, "waiting": waiting}))
                    .unwrap(),
            )
            .unwrap();
        };
        waiting(serde_json::json!([
            {"campaign": OTHER, "hours": 1, "first_hour": 3, "last_hour": 3}
        ]));
        let library = namer(BTreeMap::new(), Arc::default()).with_settings(path.clone());
        let names = BTreeMap::new();
        let live = Live {
            session: None,
            target: None,
            campaign: None,
            hour: None,
            set_aside: &[],
            names: &names,
        };
        let listed = library.list(
            &settings,
            library.view("panel", 1, true),
            &live,
            &BTreeMap::new(),
            || {},
        );
        let rows: Vec<_> = listed
            .unsaved
            .iter()
            .map(|row| (row.session.clone(), row.campaign.clone()))
            .collect();
        assert_eq!(rows, [(ended.display().to_string(), OTHER.to_owned())]);
        assert_eq!(
            RecorderSettings::load_from(&path)
                .unwrap()
                .earlier_save_folders,
            [earlier.clone(), gone.clone()]
        );
        waiting(serde_json::json!([]));
        let running = earlier.join("sessions").join("2-2");
        fs::create_dir_all(&running).unwrap();
        let live = Live {
            session: Some(&running),
            ..live
        };
        let listed = library.list(
            &settings,
            library.view("panel", 1, true),
            &live,
            &BTreeMap::new(),
            || {},
        );
        assert!(listed.unsaved.is_empty());
        assert_eq!(
            RecorderSettings::load_from(&path)
                .unwrap()
                .earlier_save_folders,
            [earlier.clone(), gone.clone()]
        );
        let live = Live {
            session: None,
            ..live
        };
        for unreadable in [&b"not a record"[..], b"null", b"[]", b"\"done\""] {
            fs::write(running.join("completed.json"), unreadable).unwrap();
            library.list(
                &settings,
                library.view("panel", 1, true),
                &live,
                &BTreeMap::new(),
                || {},
            );
            assert_eq!(
                RecorderSettings::load_from(&path)
                    .unwrap()
                    .earlier_save_folders,
                [earlier.clone(), gone.clone()]
            );
        }
        fs::write(running.join("completed.json"), b"{}").unwrap();
        library.list(
            &settings,
            library.view("panel", 1, true),
            &live,
            &BTreeMap::new(),
            || {},
        );
        assert_eq!(
            RecorderSettings::load_from(&path)
                .unwrap()
                .earlier_save_folders,
            [gone]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_session_begun_after_its_folder_was_found_settled_keeps_it() {
        let root = scratch();
        let earlier = root.join("earlier");
        fs::create_dir_all(&earlier).unwrap();
        let path = root.join("settings.json");
        let settings = RecorderSettings {
            save_folder: Some(root.join("current")),
            earlier_save_folders: vec![earlier.clone()],
            ..RecorderSettings::default()
        };
        let saved = settings.clone();
        let _ = RecorderSettings::update_at(&path, move |held| *held = saved).unwrap();
        assert!(settled(&earlier));
        fs::create_dir_all(earlier.join("sessions").join("3-3")).unwrap();
        forget_settled(&path, std::slice::from_ref(&earlier)).unwrap();
        assert_eq!(
            RecorderSettings::load_from(&path)
                .unwrap()
                .earlier_save_folders,
            [earlier.clone()]
        );
        fs::write(
            earlier.join("sessions").join("3-3").join("completed.json"),
            b"{}",
        )
        .unwrap();
        forget_settled(&path, std::slice::from_ref(&earlier)).unwrap();
        assert!(
            RecorderSettings::load_from(&path)
                .unwrap()
                .earlier_save_folders
                .is_empty()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_unreadable_or_missing_chosen_folder_is_reported_and_a_default_not_made_yet_is_empty() {
        let root = scratch();
        let file = root.join("not a folder");
        write(&file, 1);
        let (files, unreadable) = saved_files(&file, true);
        assert!(files.is_empty());
        assert!(unreadable.is_some_and(|failure| failure.whole));
        let missing = root.join("missing");
        assert_eq!(saved_files(&missing, true), (Vec::new(), None));
        let (files, unreadable) = saved_files(&missing, false);
        assert!(files.is_empty());
        let unreadable = unreadable.expect("a chosen folder that's gone is a failure, not an empty list");
        assert!(unreadable.whole);
        assert!(unreadable.detail.starts_with(&missing.display().to_string()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recordings_list_unsaved_spool_and_saved_files_from_metadata() {
        let root = scratch();
        let settings = RecorderSettings {
            save_folder: Some(root.clone()),
            ..RecorderSettings::default()
        };
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
        let ended = root.join("sessions").join("1-1");
        journal(&ended.join(OTHER), 0, 1000);
        journal(&ended.join(OTHER), 1, 500);
        fs::write(
            ended.join("completed.json"),
            serde_json::to_vec(&serde_json::json!({
                "target": root,
                "waiting": [{"campaign": OTHER, "hours": 5, "first_hour": 10, "last_hour": 14}],
            }))
            .unwrap(),
        )
        .unwrap();
        let older = root.join("sessions").join("1-0");
        journal(&older.join(OTHER), 0, 200);
        fs::write(
            older.join("completed.json"),
            serde_json::to_vec(&serde_json::json!({
                "target": root,
                "waiting": [{"campaign": OTHER, "hours": 2, "first_hour": 3, "last_hour": 4}],
            }))
            .unwrap(),
        )
        .unwrap();
        let running = root.join("sessions").join("2-2");
        journal(&running.join(CAMPAIGN), 0, 4000);
        let library = namer(BTreeMap::new(), Arc::default());
        let flag = FlagImage {
            width: 1,
            height: 1,
            pixels: vec![1, 2, 3, 255],
        };
        let flags = BTreeMap::from([(CAMPAIGN.to_owned(), flag.clone())]);
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
        assert_eq!(listed.unreadable, None);
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
    fn the_running_sessions_recordings_stay_in_its_folder_after_the_save_folder_changes() {
        let root = scratch();
        let settings = RecorderSettings {
            save_folder: Some(root.join("moved")),
            ..RecorderSettings::default()
        };
        let germany = root.join("Germany 1936.battleplan");
        recording(&germany, CAMPAIGN, 3000);
        let running = root.join("sessions").join("2-2");
        journal(&running.join(CAMPAIGN), 0, 4000);
        journal(&running.join(OTHER), 0, 1000);
        let names = BTreeMap::from([(CAMPAIGN.to_owned(), "Germany 1936–1943".to_owned())]);
        let set_aside = [Waiting {
            campaign: CAMPAIGN.into(),
            unsaved: Default::default(),
            saving: false,
        }];
        let live = Live {
            session: Some(&running),
            target: Some(&root),
            campaign: Some(OTHER),
            hour: Some(99),
            set_aside: &set_aside,
            names: &names,
        };
        let library = namer(BTreeMap::new(), Arc::default());
        let view = library.view("panel", 1, true);
        let listed = library.list(&settings, view, &live, &BTreeMap::new(), || {});
        let waiting = listed
            .unsaved
            .iter()
            .find(|row| row.campaign == CAMPAIGN)
            .unwrap();
        assert_eq!(waiting.recording, Some(germany.display().to_string()));
        assert_eq!(waiting.name.as_deref(), Some("Germany 1936–1943"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_waiting_recording_is_named_as_its_save_names_it_once_worked_out() {
        let root = scratch();
        let mut settings = RecorderSettings {
            save_folder: Some(root.clone()),
            ..RecorderSettings::default()
        };
        let map = format!("{CAMPAIGN} (2)");
        settings.campaigns.insert(
            CAMPAIGN.into(),
            replay_settings::recorder_settings::CampaignChoice {
                folder: Some(map.clone()),
                ..Default::default()
            },
        );
        let ended = root.join("sessions").join("1-1");
        journal(&ended.join(CAMPAIGN), 0, 1000);
        fs::write(
            ended.join("completed.json"),
            serde_json::to_vec(&serde_json::json!({
                "target": root,
                "waiting": [{"campaign": CAMPAIGN, "hours": 5, "first_hour": 10, "last_hour": 14}],
            }))
            .unwrap(),
        )
        .unwrap();
        let staged = replay_settings::publication::unsaved_path(&root, CAMPAIGN);
        recording(&staged, CAMPAIGN, 100);
        let running = root.join("sessions").join("2-2");
        journal(&running.join(CAMPAIGN), 0, 4000);
        let target = root.join(&map);
        recording(
            &replay_settings::publication::unsaved_path(&target, CAMPAIGN),
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
        let (listed, ready) = list();
        assert_eq!(listed.unsaved.len(), 2);
        assert_eq!(name(&listed, &running), Some("Germany 1936–1942".into()));
        assert_eq!(name(&listed, &ended), None);
        ready.recv().unwrap();
        let (listed, _) = list();
        assert_eq!(name(&listed, &ended), Some("Germany 1936".into()));
        assert_eq!(
            *asked.lock().unwrap(),
            [(ended.clone(), CAMPAIGN.to_owned())]
        );
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
    fn waiting_names_revalidate_resolution_settings_and_waiting_revisions() {
        let root = scratch();
        let mut settings = RecorderSettings {
            save_folder: Some(root.clone()),
            ..Default::default()
        };
        let session = root.join("sessions/1-1");
        fs::create_dir_all(&session).unwrap();
        let waiting = replay_settings::publication::unsaved_path(&root, CAMPAIGN);
        recording(&waiting, CAMPAIGN, 100);
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
        let ahead = Arc::new(Mutex::new(Duration::ZERO));
        library.clock = {
            let ahead = Arc::clone(&ahead);
            Arc::new(move || Instant::now() + *ahead.lock().unwrap())
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
                    known.checked = (library.clock)() + Duration::from_secs(86_400);
                }
            }
            let view = library.view("panel", visit.get(), true);
            let listed = library.list(settings, view, &live, &BTreeMap::new(), || {});
            assert_eq!(listed.unsaved[0].name.as_deref(), expected);
        };
        refresh(&settings, None);
        *response.lock().unwrap() = Some("Z01".into());
        recording(&waiting, CAMPAIGN, 200);
        refresh(&settings, Some("Z01"));
        *response.lock().unwrap() = Some("Named country".into());
        settings.user_folder = Some(root.join("other-user"));
        refresh(&settings, Some("Named country"));
        *installation.lock().unwrap() = Ok((root.join("other-game"), root.join("user")));
        refresh(&settings, Some("Named country"));
        write(&session.join("admitted.json"), 10);
        refresh(&settings, Some("Named country"));
        write(&session.join("bin/collector.json"), 10);
        refresh(&settings, Some("Named country"));
        *response.lock().unwrap() = Some("Z01".into());
        write(&session.join("admitted.json"), 20);
        refresh(&settings, Some("Z01"));
        *response.lock().unwrap() = Some("Repaired country".into());
        let now = (library.clock)();
        for known in library.names.lock().unwrap().known.values_mut() {
            known.checked = now;
        }
        *ahead.lock().unwrap() += NAME_RETRY;
        refresh(&settings, Some("Repaired country"));
        *response.lock().unwrap() = Some("Changed game installation".into());
        let now = (library.clock)();
        for known in library.names.lock().unwrap().known.values_mut() {
            known.checked = now;
        }
        *ahead.lock().unwrap() += NAME_RETRY_MAX;
        let view = library.view("panel", visit.get(), true);
        let listed = library.list(&settings, view, &live, &BTreeMap::new(), || {});
        assert_eq!(listed.unsaved[0].name.as_deref(), Some("Repaired country"));
        assert_eq!(*asked.lock().unwrap(), 8);
        let _ = library.view("panel", visit.get(), false);
        visit.set(visit.get() + 1);
        refresh(&settings, Some("Changed game installation"));
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
        journal(&session.join(CAMPAIGN), 0, 10);
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
        journal(&session.join(CAMPAIGN), 0, 10);
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
        journal(&session.join(CAMPAIGN), 0, 10);
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
        journal(&session.join(CAMPAIGN), 0, 10);
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
        journal(&session.join(CAMPAIGN), 0, 10);
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
