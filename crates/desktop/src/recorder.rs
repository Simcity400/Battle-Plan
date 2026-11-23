use replay_settings::recorder_settings::RecorderSettings;
use replay_files::recording_files::{self, NamesLock};
use replay_data::identity::is_campaign_uuid;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::SystemTime,
};
use windows_sys::Win32::System::{
    JobObjects::IsProcessInJob,
    Threading::{CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS, GetCurrentProcess},
};

const RECORDER: &str = "battleplan-recorder.exe";

const HELD: &str = "BattlePlan Recorder wasn’t started with BattlePlan: the program that opened BattlePlan would close the recorder with it, ending a recording. Start BattlePlan Recorder from the Start menu, or turn on Start with Windows in its settings.";

pub fn start_recorder() {
    if !RecorderSettings::load().map_or(true, |settings| settings.start_with_replay) {
        return;
    }
    let Ok(executable) = std::env::current_exe().map(|path| path.with_file_name(RECORDER)) else {
        return;
    };
    if !executable.is_file() || recorder_running() {
        return;
    }
    let mut command = Command::new(&executable);
    command.arg(replay_settings::recorder_settings::STARTED_WITH_REPLAY);
    if let Err(refusal) = launch_independently(&mut command) {
        eprintln!("{refusal}");
        std::thread::spawn(move || {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title(app_update::recorder_name())
                .set_description(refusal)
                .set_buttons(rfd::MessageButtons::Ok)
                .show()
        });
    }
}

fn recorder_running() -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE},
    };
    let name: Vec<u16> = std::ffi::OsStr::new(replay_settings::recorder_settings::RECORDER_INSTANCE).encode_wide().chain([0]).collect();
    let held = unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr()) };
    if held.is_null() {
        return false;
    }
    unsafe { CloseHandle(held) };
    true
}

fn launch_independently(command: &mut Command) -> Result<Child, String> {
    use std::os::windows::io::AsRawHandle;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB)
        .spawn()
        .map_err(|error| refusal(&error))?;
    let mut held = 1;
    let checked = unsafe { IsProcessInJob(child.as_raw_handle(), std::ptr::null_mut(), &raw mut held) } != 0;
    if checked && held == 0 {
        return Ok(child);
    }
    let refused = if checked { HELD.to_owned() } else { format!("BattlePlan Recorder couldn’t start: {}", io::Error::last_os_error()) };
    let _ = child.kill();
    let _ = child.wait();
    Err(refused)
}

fn refusal(error: &io::Error) -> String {
    let mut in_job = 0;
    let held = error.kind() == io::ErrorKind::PermissionDenied
        && unsafe { IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &raw mut in_job) } != 0
        && in_job != 0;
    if held {
        HELD.to_owned()
    } else {
        format!("BattlePlan Recorder couldn’t start: {error}")
    }
}

fn campaign_key(id: &str) -> String {
    id.bytes().map(|byte| format!("{byte:02x}")).collect()
}

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

#[derive(Serialize)]
pub struct Warned<T> {
    #[serde(flatten)]
    value: T,
    warning: Option<String>,
}

impl<T> Warned<T> {
    pub fn new(value: T, warning: Option<String>) -> Self {
        Self { value, warning }
    }
}

fn saved(applied: replay_settings::exit::Applied<RecorderSettings>) -> Warned<RecorderSettings> {
    Warned::new(applied.value, applied.warning)
}

#[tauri::command]
pub async fn set_recording_compression(
    compression: replay_settings::compression::Compression,
) -> Result<Warned<RecorderSettings>, String> {
    blocking(move || {
        RecorderSettings::update(|settings| settings.choose_compression(compression))
            .map(saved)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn set_auto_compress(enabled: bool) -> Result<Warned<RecorderSettings>, String> {
    blocking(move || {
        RecorderSettings::update(|settings| settings.auto_compress = enabled)
            .map(saved)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn recording_preferences() -> Result<RecordingPreferences, String> {
    blocking(|| {
        RecorderSettings::load()
            .map(|settings| preferences(&settings))
            .map_err(|error| format!("Recorder settings couldn’t be read: {error}"))
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Renamed {
    path: String,
    warning: Option<String>,
}

#[tauri::command]
pub async fn rename_recording(path: String, name: String) -> Result<Renamed, String> {
    blocking(move || {
        rename(Path::new(&path), &name, &RecorderSettings::path())
            .map(|(renamed, warning)| Renamed { path: renamed.display().to_string(), warning })
    })
    .await
}

fn rename(path: &Path, name: &str, settings: &Path) -> Result<(PathBuf, Option<String>), String> {
    let failed = |error: std::io::Error| format!("The recording couldn’t be renamed: {error}");
    let stem = recording_files::file_stem(name)
        .ok_or("Choose a name with a letter or number Windows allows in file names.")?;
    if !path.is_file() || !recording_files::is_recording(path) {
        return Err("Choose a BattlePlan recording to rename.".into());
    }
    let folder = std::fs::canonicalize(path.parent().ok_or("This recording has no folder.")?)
        .map_err(failed)?;
    let _lock = NamesLock::try_now(&folder)
        .map_err(failed)?
        .ok_or("BattlePlan Recorder is saving into this folder. Try again in a moment.")?;
    let campaign = recording_files::campaign_of(path)
        .map_err(failed)?
        .and_then(|id| String::from_utf8(id).ok());
    let (renamed, warning) = RecorderSettings::hold_at(settings, |held| {
        let mut added = false;
        let mut warning = None;
        if let Some(campaign) = &campaign {
            warning = held
                .update(|settings| {
                    added = !settings.name_chosen(&folder, campaign.as_bytes());
                    settings.choose_name(&folder, campaign, true);
                })?
                .warning;
        }
        let renamed = recording_files::rename(path, &stem);
        if renamed.is_err()
            && let (true, Some(campaign)) = (added, &campaign)
        {
            warning = held.update(|settings| settings.choose_name(&folder, campaign, false))?.warning.or(warning);
        }
        Ok((renamed, warning))
    })
    .map_err(failed)?;
    let warning = warning.map(|warning| format!("The recording was renamed, but BattlePlan Recorder may not keep the name: {warning}"));
    renamed.map(|renamed| (renamed, warning)).map_err(|error| {
        if error.raw_os_error() == Some(32) {
            "BattlePlan Recorder is recording into this file. Rename it once the session is saved."
                .to_owned()
        } else {
            failed(error)
        }
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    path: String,
    campaign_key: String,
}

type Listing = Vec<(PathBuf, u64, SystemTime)>;

static FOLDERS: Mutex<BTreeMap<PathBuf, (Listing, recording_files::Found)>> =
    Mutex::new(BTreeMap::new());

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

#[tauri::command]
pub async fn set_campaign_flag(campaign_key: String, tag: Option<String>) -> Result<Option<String>, String> {
    let id = campaign_id(&campaign_key).ok_or("Unknown campaign")?;
    if tag
        .as_deref()
        .is_some_and(|tag| tag.is_empty() || tag.contains('\0'))
    {
        return Err("Invalid flag tag".into());
    }
    blocking(move || {
        RecorderSettings::update(|settings| settings.campaigns.entry(id).or_default().flag = tag)
            .map(|applied| applied.warning)
            .map_err(|error| format!("The campaign flag couldn’t be saved: {error}"))
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

    fn recording(path: &Path) {
        use replay_data::identity::{
            CaptureMode, Identity, SampleBoundary, SessionStart, Verification,
        };
        let session = SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 1,
            sample_boundary: SampleBoundary::EndOfHour,
            capture_mode: CaptureMode::Full,
            identity: Identity {
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
                verification: Verification::Unchecked,
            },
        };
        let bytes = replay::campaign::fixtures::encode_records(
            recorder_derive::writers(),
            CAMPAIGN.as_bytes().to_vec(),
            &[vec![replay_capture_spool::spool::Record::SessionStart(session)]],
        )
        .unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn scratch() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "battleplan-rename-{}-{}",
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
        let (renamed, warning) = rename(&path, " Barbarossa? ", &settings).unwrap();
        assert_eq!(warning, None);
        assert_eq!(renamed, folder.join("Barbarossa (2).battleplan"));
        assert!(!path.exists());
        assert_eq!(
            std::fs::read(folder.join("Barbarossa.battleplan")).unwrap(),
            b"another"
        );
        let saved = RecorderSettings::load_from(&settings).unwrap();
        let identity = std::fs::canonicalize(&folder).unwrap();
        assert!(saved.name_chosen(&identity, CAMPAIGN.as_bytes()));
        let held = NamesLock::wait(&folder).unwrap();
        assert!(
            rename(&renamed, "Other", &settings)
                .unwrap_err()
                .contains("saving")
        );
        drop(held);
        assert!(renamed.exists());
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
        std::fs::copy(&renamed, folder.join("Copy.battleplan")).unwrap();
        assert_eq!(located(&mut BTreeMap::new(), &renamed, &key), None);
        assert_eq!(
            located(&mut BTreeMap::new(), &renamed, "not a campaign"),
            None
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    const ROLE: &str = "BATTLEPLAN_TEST_LAUNCH_ROLE";

    fn this_test_binary(role: &str, test: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", test, "--nocapture", "--test-threads=1"])
            .env(ROLE, role);
        command
    }

    #[test]
    fn launched_recorder_stand_in() {
        if std::env::var(ROLE).as_deref() == Ok("recorder") {
            loop {
                std::thread::park();
            }
        }
    }

    #[test]
    fn launching_desktop_stand_in() {
        use std::io::{BufRead, Write};
        if std::env::var(ROLE).as_deref() != Ok("desktop") {
            return;
        }
        let mut stdin = std::io::stdin().lock();
        stdin.read_line(&mut String::new()).unwrap();
        let mut recorder =
            this_test_binary("recorder", "recorder::tests::launched_recorder_stand_in");
        let line = match launch_independently(&mut recorder) {
            Ok(child) => format!("launched {}", child.id()),
            Err(refusal) => format!("refused {refusal}"),
        };
        let mut out = std::io::stdout().lock();
        writeln!(out, "\n{line}").unwrap();
        out.flush().unwrap();
        stdin.read_line(&mut String::new()).unwrap();
    }

    #[test]
    fn the_recorder_outlives_the_desktop_or_is_refused_by_name() {
        use std::{
            io::{BufRead, BufReader, Write},
            os::windows::io::AsRawHandle,
        };
        use windows_sys::Win32::{
            Foundation::{CloseHandle, WAIT_TIMEOUT},
            System::{
                JobObjects::{
                    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
                    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                    JobObjectExtendedLimitInformation, SetInformationJobObject,
                },
                Threading::{
                    OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess,
                    WaitForSingleObject,
                },
            },
        };
        let job = |breakaway: bool| {
            let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            assert!(!job.is_null());
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags =
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | if breakaway { JOB_OBJECT_LIMIT_BREAKAWAY_OK } else { 0 };
            let set = unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&raw const limits).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            assert_ne!(set, 0);
            job
        };
        for (outer_breakaway, nested, frees) in [(true, false, true), (false, false, false), (false, true, false)] {
            let outer = job(outer_breakaway);
            let inner = nested.then(|| job(true));
            let mut desktop =
                this_test_binary("desktop", "recorder::tests::launching_desktop_stand_in")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .spawn()
                    .unwrap();
            for job in std::iter::once(outer).chain(inner) {
                assert_ne!(unsafe { AssignProcessToJobObject(job, desktop.as_raw_handle()) }, 0);
            }
            let mut input = desktop.stdin.take().unwrap();
            writeln!(input).unwrap();
            let line = BufReader::new(desktop.stdout.take().unwrap())
                .lines()
                .map(Result::unwrap)
                .find(|line| line.starts_with("launched ") || line.starts_with("refused "))
                .unwrap();
            let close = || {
                for job in std::iter::once(outer).chain(inner) {
                    unsafe { CloseHandle(job) };
                }
            };
            let mut runner = 0;
            let runner_held = unsafe { IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &raw mut runner) } != 0 && runner != 0;
            if !frees || (runner_held && line.starts_with("refused ")) {
                assert!(line.contains("Start BattlePlan Recorder from the Start menu"), "nested {nested}: {line}");
                close();
                desktop.wait().unwrap();
                continue;
            }
            let pid: u32 = line.strip_prefix("launched ").unwrap().parse().unwrap();
            let recorder = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, 0, pid) };
            assert!(!recorder.is_null());
            close();
            desktop.wait().unwrap();
            let running = unsafe { WaitForSingleObject(recorder, 0) } == WAIT_TIMEOUT;
            unsafe {
                TerminateProcess(recorder, 0);
                CloseHandle(recorder);
            }
            assert!(running, "the recorder ended with the desktop's job");
        }
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
