use crate::{
    compress::Jobs,
    panel,
    recordings::{Library, Live, Recordings},
    snapshot::Snapshot,
    startup,
    worker::{Recorder, recordings_changed, save_folder},
};
use replay_settings::recorder_settings::{NextLoad, RecorderSettings};
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
}

fn view(settings: RecorderSettings) -> Result<SettingsView, String> {
    let effective_save_folder = save_folder(&settings).display().to_string();
    Ok(SettingsView {
        settings,
        effective_save_folder,
        start_with_windows: startup::starts_with_windows()?,
    })
}

fn update(
    change: impl FnOnce(&mut RecorderSettings) + Send + 'static,
) -> Result<SettingsView, String> {
    RecorderSettings::update(change)
        .map_err(|error| format!("Recorder settings couldn’t be saved: {error}"))
        .and_then(|(settings, ())| view(settings))
}

#[tauri::command]
pub fn recorder_status(recorder: State<'_, Arc<Recorder>>) -> Snapshot {
    recorder.snapshot()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelOpened {
    ask_stop: bool,
    compact: bool,
    #[serde(flatten)]
    ask: panel::Ask,
}

#[tauri::command]
pub fn panel_opened(
    app: AppHandle,
    recorder: State<'_, Arc<Recorder>>,
    panel: String,
) -> PanelOpened {
    recorder.panel_opened(&app);
    let (ask_stop, compact, ask) = panel::take_opening(&app, &panel);
    PanelOpened {
        ask_stop,
        compact,
        ask,
    }
}

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

#[tauri::command]
pub fn cancel_stop_prompt(recorder: State<'_, Arc<Recorder>>) {
    recorder.cancel_stop_prompt();
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
    let view = blocking(move || {
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
    })
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
        update(move |settings| settings.choose_save_folder(folder)).map(Some)
    })
    .await
}

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
    let _acting = recorder.acting()?;
    let (session, live) = waiting_part(&recorder, &session, &campaign)?;
    let target = match live {
        Some(target) => target,
        None => {
            let (session, campaign) = (session.clone(), campaign.clone());
            blocking(move || recorder::controller::waiting_folder(&session, &campaign)).await?
        }
    };
    if recorder.act_on_recording(&target, &campaign, discard) {
        return Ok(None);
    }
    let jobs = Arc::clone(app.state::<Arc<Jobs>>().inner());
    let result = blocking(move || {
        if discard {
            recorder::controller::discard_waiting(&session, &campaign).map(|()| None)
        } else {
            recorder::controller::save_waiting(&session, &campaign).map(|saved| {
                if let Some(path) = &saved.recording {
                    jobs.saved(path);
                }
                saved.by_tag.map(|why| {
                    format!(
                        "Saved. Country names couldn’t be read, so countries show by tag: {why}"
                    )
                })
            })
        }
    })
    .await;
    recordings_changed(&app);
    result
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
    use super::plain_path;
    use std::path::PathBuf;

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
