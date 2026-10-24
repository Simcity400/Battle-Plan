//! Commands the panel invokes.
use crate::{
    panel,
    recordings::{Library, Live, Recordings},
    snapshot::Snapshot,
    startup,
    worker::{Recorder, recordings_changed, save_folder},
};
use replay::{recorder_settings::NextLoad, recorder_settings::RecorderSettings};
use replay_data::identity::is_campaign_uuid;
use replay_data::scope::CaptureMode;
use serde::{Deserialize, Serialize};
use std::{
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, PoisonError},
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

/// The settings with the folder recordings are actually saved to.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    #[serde(flatten)]
    settings: RecorderSettings,
    effective_save_folder: String,
}

impl From<RecorderSettings> for SettingsView {
    fn from(settings: RecorderSettings) -> Self {
        let effective_save_folder = save_folder(&settings).display().to_string();
        Self {
            settings,
            effective_save_folder,
        }
    }
}

fn update(
    change: impl FnOnce(&mut RecorderSettings) + Send + 'static,
) -> Result<SettingsView, String> {
    RecorderSettings::update(change)
        .map(|(settings, ())| settings.into())
        .map_err(|error| format!("Recorder settings could not be saved: {error}"))
}

#[tauri::command]
pub fn recorder_status(recorder: State<'_, Arc<Recorder>>) -> Snapshot {
    recorder.snapshot()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelOpened {
    ask_stop: bool,
    /// The window is the Stop prompt alone, beside the tray icon.
    compact: bool,
}

/// The panel has drawn: it learns whether to open on the Stop prompt.
#[tauri::command]
pub fn panel_opened(
    app: AppHandle,
    recorder: State<'_, Arc<Recorder>>,
    panel: String,
) -> PanelOpened {
    recorder.panel_opened(&app);
    let (ask_stop, compact) = panel::take_opening(&app, &panel);
    PanelOpened { ask_stop, compact }
}

/// The panel's content height changed: resize, place and (first time) show it.
#[tauri::command]
pub fn panel_fit(app: AppHandle, window: WebviewWindow, height: f64) -> Result<(), String> {
    panel::fit(&app, &window, height).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn close_panel(window: WebviewWindow) -> Result<(), String> {
    window.destroy().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn start_recording(recorder: State<'_, Arc<Recorder>>) {
    recorder.record();
}

#[tauri::command]
pub fn stop_recording(app: AppHandle, recorder: State<'_, Arc<Recorder>>, discard: bool) {
    recorder.stop(&app, discard);
}

/// The Stop prompt was cancelled by pressing Stop again.
#[tauri::command]
pub fn cancel_stop_prompt(recorder: State<'_, Arc<Recorder>>) {
    recorder.cancel_stop_prompt();
}

#[tauri::command]
pub async fn recorder_settings() -> Result<SettingsView, String> {
    blocking(|| {
        RecorderSettings::load()
            .map(SettingsView::from)
            .map_err(|error| format!("Recorder settings could not be read: {error}"))
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
    game_score: Option<bool>,
    operation_outcomes: Option<bool>,
    politics: Option<bool>,
    capture_mode: Option<String>,
}

/// Held while Start with Windows and its saved setting change together.
static START_WITH_WINDOWS: Mutex<()> = Mutex::new(());

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
    let view = blocking(move || {
        // The Run entry must change before its setting is saved.
        let _serial = START_WITH_WINDOWS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let previous = match patch.start_with_windows {
            Some(enabled) => {
                let previous = RecorderSettings::load()
                    .map_err(|error| format!("Recorder settings could not be read: {error}"))?
                    .start_with_windows;
                startup::set_start_with_windows(enabled)?;
                Some(previous)
            }
            None => None,
        };
        let view = update(move |settings| {
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
            set(&mut settings.start_with_windows, patch.start_with_windows);
            set(&mut settings.start_with_replay, patch.start_with_replay);
            set(&mut settings.game_score, patch.game_score);
            set(&mut settings.operation_outcomes, patch.operation_outcomes);
            set(&mut settings.politics, patch.politics);
            if let Some(mode) = capture_mode {
                settings.capture_mode = mode;
            }
        });
        if view.is_err()
            && let Some(previous) = previous
        {
            // A save can fail after replacement; restore the Run entry from what is now saved.
            let saved =
                RecorderSettings::load().map_or(previous, |settings| settings.start_with_windows);
            if let Err(error) = startup::set_start_with_windows(saved) {
                eprintln!("{error}");
            }
        }
        view
    })
    .await?;
    app.state::<Arc<Recorder>>().settings_changed();
    Ok(view)
}

/// One campaign's Record automatically and Next time it loads.
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
    let view = blocking(move || {
        update(move |settings| {
            let choice = settings.campaigns.entry(campaign).or_default();
            if let Some(record) = record {
                choice.record = Some(record);
            }
            if let Some(next) = next {
                choice.next = next;
            }
        })
    })
    .await?;
    app.state::<Arc<Recorder>>().settings_changed();
    Ok(view)
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
    blocking(move || {
        let current = save_folder(&RecorderSettings::load().unwrap_or_default());
        let Some(folder) =
            pick_folder(&app, &window, "Choose where recordings are saved", &current)
        else {
            return Ok(None);
        };
        update(move |settings| settings.save_folder = Some(folder)).map(Some)
    })
    .await
}

/// `choose` picks a Paradox user folder; otherwise it is found automatically.
#[tauri::command]
pub async fn set_user_folder(
    app: AppHandle,
    window: WebviewWindow,
    choose: bool,
) -> Result<Option<SettingsView>, String> {
    blocking(move || {
        if !choose {
            return update(|settings| settings.user_folder = None).map(Some);
        }
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
        update(move |settings| settings.user_folder = Some(folder)).map(Some)
    })
    .await
}

fn open_in_explorer(folder: &Path) -> Result<(), String> {
    Command::new("explorer")
        .arg(folder)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not open {}: {error}", folder.display()))
}

#[tauri::command]
pub async fn open_recordings_folder() -> Result<(), String> {
    blocking(|| {
        let folder = save_folder(&RecorderSettings::load().unwrap_or_default());
        std::fs::create_dir_all(&folder)
            .map_err(|error| format!("Could not open {}: {error}", folder.display()))?;
        open_in_explorer(&folder)
    })
    .await
}

/// The recordings not saved yet and the saved ones, listed while the panel asks.
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
            .map_err(|error| format!("Recorder settings could not be read: {error}"))?;
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
        Ok(app
            .state::<Library>()
            .list(&settings, view, &live, &recorder.flags(), move || {
                recordings_changed(&named)
            }))
    })
    .await
}

/// A session folder of this recorder's save folder, and a recorded campaign in it.
fn waiting_part(session: &str, campaign: &str) -> Result<PathBuf, String> {
    let root = save_folder(&RecorderSettings::load().unwrap_or_default());
    let sessions = root
        .join("sessions")
        .canonicalize()
        .map_err(|error| format!("Unknown recording: {error}"))?;
    let session = PathBuf::from(session)
        .canonicalize()
        .map_err(|error| format!("Unknown recording: {error}"))?;
    if session.parent() != Some(sessions.as_path()) || !is_campaign_uuid(campaign.as_bytes()) {
        return Err("Unknown recording".into());
    }
    Ok(session)
}

/// Through the running session when it saves into the same folder. Returns why a save named its
/// countries by tag, when it did.
#[tauri::command]
pub async fn act_on_waiting(
    app: AppHandle,
    session: String,
    campaign: String,
    discard: bool,
) -> Result<Option<String>, String> {
    let session = waiting_part(&session, &campaign)?;
    let recorder = Arc::clone(app.state::<Arc<Recorder>>().inner());
    let target = match recorder.live_target(&session) {
        Some(target) => target,
        None => {
            let (session, campaign) = (session.clone(), campaign.clone());
            blocking(move || recorder::controller::waiting_folder(&session, &campaign)).await?
        }
    };
    if recorder.act_on_recording(&target, &campaign, discard) {
        return Ok(None);
    }
    let result = blocking(move || {
        if discard {
            recorder::controller::discard_waiting(&session, &campaign).map(|()| None)
        } else {
            recorder::controller::save_waiting(&session, &campaign).map(|saved| {
                saved
                    .by_tag
                    .map(|why| format!("Saved. Its countries are named by tag: {why}"))
            })
        }
    })
    .await;
    recordings_changed(&app);
    result
}

/// Show a saved recording selected in its folder.
#[tauri::command]
pub async fn reveal_recording(path: String) -> Result<(), String> {
    blocking(move || {
        let path = recording_file(&path)?;
        Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("Could not show {}: {error}", path.display()))
    })
    .await
}

fn recording_file(path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path)
        .canonicalize()
        .map_err(|error| format!("Cannot open recording: {error}"))?;
    if !path.is_file()
        || !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("battleplan") || extension.eq_ignore_ascii_case("hoi4replay"))
    {
        return Err("Choose a .battleplan recording".into());
    }
    // Explorer and the replay app take a plain path.
    let plain = path.display().to_string();
    Ok(plain
        .strip_prefix(r"\\?\")
        .map_or(path.clone(), PathBuf::from))
}

/// Details: the session folder holding the collector's status and completion records.
#[tauri::command]
pub async fn open_details(recorder: State<'_, Arc<Recorder>>) -> Result<(), String> {
    let session = recorder.snapshot().session;
    blocking(move || {
        let folder = match session {
            Some(session) if Path::new(&session).is_dir() => PathBuf::from(session),
            _ => save_folder(&RecorderSettings::load().unwrap_or_default()),
        };
        open_in_explorer(&folder)
    })
    .await
}

/// Start Battle Plan, the sibling executable, optionally on one recording.
pub fn launch_replay(recording: Option<&Path>) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .with_file_name("replay-desktop.exe");
    if !executable.is_file() {
        return Err(format!(
            "Battle Plan was not found at {}",
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
        .map_err(|error| format!("Could not start Battle Plan: {error}"))
}

/// Open Battle Plan; with a path, on that finished recording.
#[tauri::command]
pub async fn open_replay(path: Option<String>) -> Result<(), String> {
    blocking(move || {
        let recording = path.as_deref().map(recording_file).transpose()?;
        launch_replay(recording.as_deref())
    })
    .await
}
