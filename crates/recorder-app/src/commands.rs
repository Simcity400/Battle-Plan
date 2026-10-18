//! Commands the panel invokes.
use crate::{
    panel,
    snapshot::Snapshot,
    startup,
    worker::{Recorder, save_folder},
};
use replay::{recorder_settings::NextLoad, recorder_settings::RecorderSettings};
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
}

/// The panel has drawn: it learns whether to open on the Stop prompt.
#[tauri::command]
pub fn panel_opened(app: AppHandle, recorder: State<'_, Arc<Recorder>>) -> PanelOpened {
    recorder.panel_opened(&app);
    PanelOpened {
        ask_stop: panel::take_ask_stop(&app),
    }
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
    start_with_windows: Option<bool>,
    start_with_replay: Option<bool>,
    game_score: Option<bool>,
    operation_outcomes: Option<bool>,
    politics: Option<bool>,
}

#[tauri::command]
pub async fn update_recorder_settings(
    app: AppHandle,
    patch: SettingsPatch,
) -> Result<SettingsView, String> {
    let view = blocking(move || {
        // The Run entry changes first, so a saved setting always matches it.
        if let Some(enabled) = patch.start_with_windows {
            startup::set_start_with_windows(enabled)?;
        }
        update(move |settings| {
            let set = |field: &mut bool, value: Option<bool>| {
                if let Some(value) = value {
                    *field = value;
                }
            };
            set(
                &mut settings.record_every_campaign,
                patch.record_every_campaign,
            );
            set(&mut settings.start_with_windows, patch.start_with_windows);
            set(&mut settings.start_with_replay, patch.start_with_replay);
            set(&mut settings.game_score, patch.game_score);
            set(&mut settings.operation_outcomes, patch.operation_outcomes);
            set(&mut settings.politics, patch.politics);
        })
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
        let recording = match path {
            Some(path) => {
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
                Some(path)
            }
            None => None,
        };
        launch_replay(recording.as_deref())
    })
    .await
}
