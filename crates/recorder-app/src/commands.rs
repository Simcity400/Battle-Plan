use crate::{
    compress::Jobs,
    panel,
    recordings::{Library, Live, Recordings},
    snapshot::Snapshot,
    startup,
    worker::{Recorder, recordings_changed, save_folder},
};
use replay_settings::{
    exit::What,
    recorder_settings::{NextLoad, RecorderSettings},
};
use replay_data::identity::is_campaign_uuid;
use replay_data::scope::CaptureMode;
use serde::{Deserialize, Serialize};
use std::{
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    #[serde(flatten)]
    settings: RecorderSettings,
    effective_save_folder: String,
    start_with_windows: bool,
    warning: Option<String>,
}

fn view(settings: RecorderSettings) -> Result<SettingsView, String> {
    let effective_save_folder = save_folder(&settings).display().to_string();
    Ok(SettingsView {
        settings,
        effective_save_folder,
        start_with_windows: startup::starts_with_windows()?,
        warning: None,
    })
}

fn update(change: impl FnOnce(&mut RecorderSettings)) -> Result<SettingsView, String> {
    change_settings(None, change)
}

fn change_settings(
    start_with_windows: Option<bool>,
    change: impl FnOnce(&mut RecorderSettings),
) -> Result<SettingsView, String> {
    let applied = RecorderSettings::hold_at(&RecorderSettings::path(), |held| {
        patched(start_with_windows, &Windows, || held.update(change))
    })
    .map_err(|error| format!("Recorder settings couldn’t be saved: {error}"))?;
    let warning = applied.warning;
    view(applied.value).map(|view| SettingsView { warning, ..view })
}

trait Startup {
    fn read(&self) -> Result<Option<startup::RunValue>, String>;
    fn set(&self, enabled: bool) -> Result<(), String>;
    fn put_back(&self, value: Option<&startup::RunValue>) -> Result<(), String>;
}

struct Windows;

impl Startup for Windows {
    fn read(&self) -> Result<Option<startup::RunValue>, String> {
        startup::run_value()
    }

    fn set(&self, enabled: bool) -> Result<(), String> {
        startup::set_start_with_windows(enabled)
    }

    fn put_back(&self, value: Option<&startup::RunValue>) -> Result<(), String> {
        startup::write_run_value(value)
    }
}

fn patched(
    start_with_windows: Option<bool>,
    startup: &dyn Startup,
    save: impl FnOnce() -> std::io::Result<replay_settings::exit::Applied<RecorderSettings>>,
) -> std::io::Result<replay_settings::exit::Applied<RecorderSettings>> {
    let before = start_with_windows.map(|_| startup.read()).transpose().map_err(std::io::Error::other)?;
    if let Some(enabled) = start_with_windows {
        startup.set(enabled).map_err(std::io::Error::other)?;
    }
    save().inspect_err(|_| {
        if let Some(before) = &before
            && let Err(error) = startup.put_back(before.as_ref())
        {
            eprintln!("Start with Windows couldn’t be put back after the settings failed to save: {error}");
        }
    })
}

#[tauri::command]
pub fn recorder_status(recorder: State<'_, Arc<Recorder>>) -> Snapshot {
    recorder.snapshot()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelOpened {
    compact: bool,
}

#[tauri::command]
pub fn panel_opened(
    app: AppHandle,
    window: WebviewWindow,
    recorder: State<'_, Arc<Recorder>>,
    panel: String,
) -> Result<PanelOpened, String> {
    let compact = panel::opening(&app, &window, &panel)?;
    recorder.panel_opened(&app);
    Ok(PanelOpened { compact })
}

#[tauri::command]
pub fn panel_fit(app: AppHandle, window: WebviewWindow, height: f64) -> Result<(), String> {
    panel::fit(&app, &window, height)
}

#[tauri::command]
pub async fn close_panel(
    app: AppHandle,
    window: WebviewWindow,
    recorder: State<'_, Arc<Recorder>>,
    revision: u64,
) -> Result<bool, String> {
    let instance = panel::instance_of(&app, &window).ok_or(panel::CLOSED)?;
    if !recorder.dismiss(instance, revision) {
        return Ok(false);
    }
    window.destroy().map_err(|error| {
        recorder.dismissal_failed(instance);
        error.to_string()
    })?;
    Ok(true)
}

#[tauri::command]
pub fn prompt_seen(
    app: AppHandle,
    window: WebviewWindow,
    recorder: State<'_, Arc<Recorder>>,
    prompt: u64,
) -> Result<(), String> {
    if window.label() != panel::LABEL {
        return Err("Only the recorder panel shows its prompts.".into());
    }
    let instance = panel::instance_of(&app, &window).ok_or(panel::CLOSED)?;
    recorder.prompt_seen(instance, prompt);
    Ok(())
}


#[tauri::command]
pub fn start_recording(recorder: State<'_, Arc<Recorder>>) {
    recorder.record();
}

#[tauri::command]
pub fn stop_recording(
    app: AppHandle,
    recorder: State<'_, Arc<Recorder>>,
    discard: bool,
    session_key: u64,
) -> Result<(), String> {
    recorder.stop(&app, discard, session_key)
}

#[tauri::command]
pub fn cancel_stop_prompt(app: AppHandle, recorder: State<'_, Arc<Recorder>>, prompt: u64) {
    recorder.cancel_prompt(&app, prompt);
}

#[tauri::command]
pub async fn recorder_settings() -> Result<SettingsView, String> {
    blocking(|| {
        RecorderSettings::load()
            .map_err(|error| format!("Recorder settings couldn’t be read: {error}"))
            .and_then(view)
    })
    .await
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    record_every_campaign: Option<bool>,
    auto_save: Option<bool>,
    start_with_windows: Option<bool>,
    start_with_replay: Option<bool>,
    capture_mode: Option<String>,
    compression: Option<replay_settings::compression::Compression>,
    auto_compress: Option<bool>,
}

#[tauri::command]
pub async fn update_recorder_settings(
    app: AppHandle,
    patch: SettingsPatch,
) -> Result<SettingsView, String> {
    let capture_mode = patch
        .capture_mode
        .as_deref()
        .map(|name| {
            CaptureMode::from_name(name).ok_or_else(|| format!("Unknown capture mode {name:?}"))
        })
        .transpose()?;
    let (request, receipt) = admitted(&app)?;
    let view = blocking(move || receipt.carry(|| {
        let _request = request;
        change_settings(patch.start_with_windows, move |settings| {
            let set = |field: &mut bool, value: Option<bool>| {
                if let Some(value) = value {
                    *field = value;
                }
            };
            set(
                &mut settings.record_every_campaign,
                patch.record_every_campaign,
            );
            set(&mut settings.auto_save, patch.auto_save);
            set(&mut settings.start_with_replay, patch.start_with_replay);
            set(&mut settings.auto_compress, patch.auto_compress);
            if let Some(compression) = patch.compression {
                settings.choose_compression(compression);
            }
            if let Some(mode) = capture_mode {
                settings.capture_mode = mode;
            }
        })
    }))
    .await?;
    app.state::<Arc<Recorder>>().settings_changed();
    Ok(view)
}

#[tauri::command]
pub async fn set_campaign_choice(
    app: AppHandle,
    campaign: String,
    record: Option<bool>,
    next: Option<NextLoad>,
) -> Result<SettingsView, String> {
    if !replay_data::identity::is_campaign_uuid(campaign.as_bytes()) {
        return Err("Unknown campaign".into());
    }
    let (request, receipt) = admitted(&app)?;
    let view = blocking(move || receipt.carry(|| {
        let _request = request;
        update(move |settings| {
            let choice = settings.campaigns.entry(campaign).or_default();
            if let Some(record) = record {
                choice.record = Some(record);
            }
            if let Some(next) = next {
                choice.next = next;
            }
        })
    }))
    .await?;
    app.state::<Arc<Recorder>>().settings_changed();
    Ok(view)
}

fn admitted(app: &AppHandle) -> Result<(panel::Request, replay_settings::exit::Receipt<'static>), String> {
    let request = panel::request(app);
    let receipt = app.state::<Arc<Recorder>>().admit(What::Change)?;
    Ok((request, receipt))
}

fn pick_folder(
    app: &AppHandle,
    window: &WebviewWindow,
    title: &str,
    start: &Path,
) -> Option<PathBuf> {
    panel::with_dialog(app, || {
        let mut dialog = rfd::FileDialog::new().set_parent(window).set_title(title);
        if let Ok(directory) = start.canonicalize()
            && directory.is_dir()
        {
            dialog = dialog.set_directory(directory);
        }
        dialog.pick_folder()
    })
}

#[tauri::command]
pub async fn choose_save_folder(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Option<SettingsView>, String> {
    let request = panel::request(&app);
    blocking(move || {
        let _request = request;
        let current = save_folder(&RecorderSettings::load().unwrap_or_default());
        let Some(folder) =
            pick_folder(&app, &window, "Choose where recordings are saved", &current)
        else {
            return Ok(None);
        };
        let receipt = app.state::<Arc<Recorder>>().admit(What::Change)?;
        receipt.carry(|| update(move |settings| settings.choose_save_folder(folder))).map(Some)
    })
    .await
}

#[tauri::command]
pub async fn set_user_folder(
    app: AppHandle,
    window: WebviewWindow,
    choose: bool,
) -> Result<Option<SettingsView>, String> {
    if !choose {
        let (request, receipt) = admitted(&app)?;
        return blocking(move || {
            let _request = request;
            receipt.carry(|| update(|settings| settings.user_folder = None)).map(Some)
        })
        .await;
    }
    let request = panel::request(&app);
    blocking(move || {
        let _request = request;
        let current = RecorderSettings::load()
            .ok()
            .and_then(|settings| settings.user_folder)
            .or_else(|| recorder::controller::resolve_user_directory().ok())
            .unwrap_or_default();
        let Some(folder) = pick_folder(
            &app,
            &window,
            "Choose your Paradox user folder",
            &current,
        ) else {
            return Ok(None);
        };
        let receipt = app.state::<Arc<Recorder>>().admit(What::Change)?;
        receipt.carry(|| update(move |settings| settings.user_folder = Some(folder))).map(Some)
    })
    .await
}

fn open_in_explorer(folder: &Path, what: &str) -> Result<(), String> {
    Command::new("explorer")
        .arg(folder)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Couldn’t open {what}: {}: {error}", folder.display()))
}

#[tauri::command]
pub async fn open_recordings_folder() -> Result<(), String> {
    blocking(|| {
        let folder = save_folder(&RecorderSettings::load().unwrap_or_default());
        std::fs::create_dir_all(&folder).map_err(|error| {
            format!(
                "Couldn’t open the recordings folder: {}: {error}",
                folder.display()
            )
        })?;
        open_in_explorer(&folder, "the recordings folder")
    })
    .await
}

#[tauri::command]
pub async fn recordings(
    app: AppHandle,
    panel: String,
    visit: u64,
    open: bool,
) -> Result<Recordings, String> {
    let view = app.state::<Library>().view(&panel, visit, open);
    blocking(move || {
        let settings = RecorderSettings::load()
            .map_err(|error| format!("Recorder settings couldn’t be read: {error}"))?;
        let recorder = app.state::<Arc<Recorder>>();
        let snapshot = recorder.snapshot();
        let (session, set_aside, names) = recorder.set_aside();
        let live = Live {
            session: session.as_ref().map(|(session, _)| session.as_path()),
            target: session.as_ref().map(|(_, target)| target.as_path()),
            campaign: snapshot
                .campaign
                .as_deref()
                .filter(|_| snapshot.phase == crate::snapshot::Phase::Recording),
            hour: snapshot.hour,
            set_aside: &set_aside,
            names: &names,
        };
        let named = app.clone();
        let mut listed = app
            .state::<Library>()
            .list(&settings, view, &live, &recorder.flags(), move || {
                recordings_changed(&named)
            });
        let jobs = app.state::<Arc<Jobs>>();
        for row in &mut listed.saved {
            row.job = jobs.job(Path::new(&row.path));
        }
        Ok(listed)
    })
    .await
}

#[tauri::command]
pub async fn compress_recording(app: AppHandle, path: String) -> Result<(), String> {
    let jobs = Arc::clone(app.state::<Arc<Jobs>>().inner());
    blocking(move || jobs.request(Path::new(&path))).await?;
    recordings_changed(&app);
    Ok(())
}

fn waiting_part(
    recorder: &Recorder,
    session: &str,
    campaign: &str,
) -> Result<(PathBuf, Option<PathBuf>), String> {
    let session = PathBuf::from(session)
        .canonicalize()
        .map_err(|error| format!("Couldn’t find that recording: {error}"))?;
    if !is_campaign_uuid(campaign.as_bytes()) {
        return Err("Couldn’t find that recording.".into());
    }
    if let Some(target) = recorder.live_target(&session) {
        return Ok((session, Some(target)));
    }
    let settings = RecorderSettings::load().unwrap_or_default();
    let known = settings.session_roots().iter().any(|root| {
        root.join("sessions")
            .canonicalize()
            .is_ok_and(|sessions| session.parent() == Some(sessions.as_path()))
    });
    if !known {
        return Err("Couldn’t find that recording.".into());
    }
    Ok((session, None))
}

#[tauri::command]
pub async fn act_on_waiting(
    app: AppHandle,
    session: String,
    campaign: String,
    discard: bool,
) -> Result<Option<String>, String> {
    let recorder = Arc::clone(app.state::<Arc<Recorder>>().inner());
    let (session, live) = waiting_part(&recorder, &session, &campaign)?;
    let target = match live {
        Some(target) => Some(target),
        None => {
            let (session, campaign) = (session.clone(), campaign.clone());
            blocking(move || recorder::controller::waiting_folder(&session, &campaign)).await.ok()
        }
    };
    if let Some(target) = target {
        let _admitted = recorder.admit(What::WaitingRecording)?;
        if recorder.act_on_recording(&target, &campaign, discard) {
            return Ok(None);
        }
    }
    let jobs = Arc::clone(app.state::<Arc<Jobs>>().inner());
    let result = blocking(move || {
        let waiting = recorder::controller::lock_waiting(&session, &campaign)?;
        let receipt = recorder.admit(What::WaitingRecording)?;
        receipt.carry(|| finish_waiting(&waiting, discard, &jobs))
    })
    .await;
    recordings_changed(&app);
    result
}

fn finish_waiting(waiting: &recorder::controller::WaitingLock, discard: bool, jobs: &Jobs) -> Result<Option<String>, String> {
    if discard {
        return waiting.discard().map(|()| None);
    }
    waiting.save().map(|saved| {
        if let Some(path) = &saved.recording {
            jobs.saved(path);
        }
        saved.by_tag.map(|why| format!("Saved. Country names couldn’t be read, so countries show by tag: {why}"))
    })
}

#[tauri::command]
pub async fn reveal_recording(path: String) -> Result<(), String> {
    blocking(move || {
        let path = recording_file(&path)?;
        Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn()
            .map(|_| ())
            .map_err(|error| {
                format!(
                    "Couldn’t show {} in its folder: {}: {error}",
                    recording_name(&path),
                    path.display()
                )
            })
    })
    .await
}

fn recording_file(path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path)
        .canonicalize()
        .map_err(|error| format!("Couldn’t find that recording: {path}: {error}"))?;
    if !path.is_file() || !replay_files::recording_files::is_recording(&path) {
        return Err("Choose a BattlePlan recording.".into());
    }
    Ok(plain_path(path))
}

fn recording_name(path: &Path) -> String {
    path.file_stem().map_or_else(
        || path.display().to_string(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

fn plain_path(path: PathBuf) -> PathBuf {
    let text = path.display().to_string();
    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{share}"));
    }
    text.strip_prefix(r"\\?\").map_or(path, PathBuf::from)
}

#[tauri::command]
pub async fn show_session_files(recorder: State<'_, Arc<Recorder>>) -> Result<(), String> {
    let session = recorder.snapshot().session;
    blocking(move || match session {
        Some(session) if Path::new(&session).is_dir() => {
            open_in_explorer(Path::new(&session), "the session’s files")
        }
        _ => Err("This session’s files are no longer on disk.".into()),
    })
    .await
}

pub fn launch_replay(recording: Option<&Path>) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .with_file_name("replay-desktop.exe");
    if !executable.is_file() {
        return Err(format!(
            "Couldn’t find BattlePlan: {} is missing.",
            executable.display()
        ));
    }
    let mut command = Command::new(&executable);
    if let Some(recording) = recording {
        command.arg(recording);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Couldn’t start BattlePlan: {error}"))
}

#[tauri::command]
pub async fn open_replay(path: Option<String>) -> Result<(), String> {
    blocking(move || {
        let recording = path.as_deref().map(recording_file).transpose()?;
        launch_replay(recording.as_deref())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{Startup, patched, plain_path};
    use crate::startup::RunValue;
    use replay_settings::{exit::Exit, recorder_settings::RecorderSettings};
    use std::{cell::RefCell, ffi::OsStr, path::PathBuf};

    const OURS: &str = "\"C:\\this\\recorder.exe\"";

    struct Registry(RefCell<Vec<Option<RunValue>>>);

    impl Registry {
        fn holding(value: Option<RunValue>) -> Self {
            Self(RefCell::new(vec![value]))
        }

        fn now(&self) -> Option<RunValue> {
            self.0.borrow().last().cloned().flatten()
        }
    }

    impl Startup for Registry {
        fn read(&self) -> Result<Option<RunValue>, String> {
            Ok(self.now())
        }

        fn set(&self, enabled: bool) -> Result<(), String> {
            self.0.borrow_mut().push(enabled.then(|| RunValue::command(OsStr::new(OURS))));
            Ok(())
        }

        fn put_back(&self, value: Option<&RunValue>) -> Result<(), String> {
            self.0.borrow_mut().push(value.cloned());
            Ok(())
        }
    }

    #[test]
    fn a_settings_change_and_its_windows_startup_value_are_one_admission_so_a_quit_refuses_both_or_neither() {
        let folder = std::env::temp_dir().join(format!("battleplan-recorder-patch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let path = folder.join("recorder.json");
        let exit = Exit::new();
        let registry = Registry::holding(None);
        let saved = RecorderSettings::hold_under(&exit, &path, |held| {
            patched(Some(true), &registry, || held.update(|settings| settings.auto_save = true))
        })
        .unwrap();
        assert!(saved.value.auto_save);
        assert_eq!(registry.now(), Some(RunValue::command(OsStr::new(OURS))));

        exit.seal();
        let refused = RecorderSettings::hold_under(&exit, &path, |held| {
            patched(Some(false), &registry, || held.update(|settings| settings.auto_save = false))
        });
        assert!(refused.is_err(), "a patch asked for once Quit commits is refused whole");
        assert_eq!(registry.0.borrow().len(), 2, "With Windows isn’t changed by a patch whose settings were refused");
        assert!(RecorderSettings::load_from(&path).unwrap().auto_save);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_failed_save_puts_back_the_startup_value_windows_held_with_its_type_and_an_unconfirmed_one_keeps_the_new_value() {
        let expanding = "\"%LOCALAPPDATA%\\older install\\battleplan-recorder.exe\" --tray";
        let earlier = RunValue {
            kind: windows_sys::Win32::System::Registry::REG_EXPAND_SZ,
            data: expanding.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect(),
        };
        let registry = Registry::holding(Some(earlier.clone()));
        let failed = patched(Some(true), &registry, || Err(std::io::Error::other("the rename was refused")));
        assert!(failed.is_err());
        assert_eq!(registry.now(), Some(earlier), "the value put back is the one Windows held, bytes and type, not this recorder’s path");

        let registry = Registry::holding(None);
        let unconfirmed = patched(Some(true), &registry, || {
            Ok(replay_settings::exit::Applied::unconfirmed(&std::io::Error::other("the disk didn’t answer")).with(RecorderSettings::default(), ()))
        });
        assert!(unconfirmed.is_ok_and(|applied| applied.warning.is_some()), "an unconfirmed save is applied, and says so");
        assert_eq!(
            registry.now(),
            Some(RunValue::command(OsStr::new(OURS))),
            "the new settings are already the file, so With Windows keeps the value they name"
        );
    }

    #[test]
    fn a_settings_change_admitted_on_arrival_is_saved_though_quit_seals_while_it_waits_on_the_settings_lock() {
        use replay_settings::exit::What;
        let folder = std::env::temp_dir().join(format!("battleplan-recorder-arrival-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let path = folder.join("recorder.json");
        let exit: &'static Exit = Box::leak(Box::new(Exit::new()));
        let mut lock = path.as_os_str().to_owned();
        lock.push(".lock");
        let another_writer = replay_settings::exit::lock_file(std::path::Path::new(&lock)).unwrap();
        let receipt = exit.admit(What::Change).unwrap();
        let (started, waiting) = std::sync::mpsc::channel();
        let writer = {
            std::thread::spawn(move || {
                receipt.carry(|| {
                    started.send(()).unwrap();
                    RecorderSettings::hold_under(exit, &path, |held| held.update(|settings| settings.auto_save = true))
                })
            })
        };
        waiting.recv().unwrap();
        assert!(exit.seal(), "Quit seals while the change waits for the settings lock");
        assert!(!exit.empty_within(std::time::Duration::ZERO), "and waits for the change it admitted when it arrived");
        drop(another_writer);
        assert!(writer.join().unwrap().is_ok_and(|applied| applied.value.auto_save), "the change is saved, not refused by the seal");
        assert!(exit.empty_within(std::time::Duration::ZERO));
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn plain_paths_keep_the_network_root() {
        let plain = |text: &str| plain_path(PathBuf::from(text)).display().to_string();
        assert_eq!(
            plain(r"\\?\C:\saves\a.battleplan"),
            r"C:\saves\a.battleplan"
        );
        assert_eq!(
            plain(r"\\?\UNC\server\share\a.battleplan"),
            r"\\server\share\a.battleplan"
        );
        assert_eq!(plain(r"D:\a.battleplan"), r"D:\a.battleplan");
    }
}
