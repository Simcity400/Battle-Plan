use crate::native::NativeHost;
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSettings {
    #[serde(default)]
    pub use_game_settings: bool,
    pub ui_scale: f64,
    pub vsync: bool,
    #[serde(default)]
    pub map_ui_scale: Option<f64>,
    #[serde(default)]
    pub map_diagnostics: bool,
    #[serde(default)]
    pub map_muted: bool,
    #[serde(default)]
    pub map_frame_limit: Option<f64>,
}
impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            use_game_settings: true,
            ui_scale: 100.0,
            vsync: true,
            map_ui_scale: None,
            map_diagnostics: false,
            map_muted: false,
            map_frame_limit: None,
        }
    }
}
const UI_SCALE_RANGE: std::ops::RangeInclusive<f64> = 75.0..=150.0;
impl VideoSettings {
    fn validate(&self) -> Result<(), String> {
        if !self.ui_scale.is_finite() || !UI_SCALE_RANGE.contains(&self.ui_scale) {
            return Err("Invalid UI scale".into());
        }
        if self.map_ui_scale.is_some_and(|percent| {
            !dashboard_core::protocol::MAP_UI_SCALE_RANGE.contains(&((percent / 100.0) as f32))
        }) {
            return Err("Invalid map UI scale".into());
        }
        if self
            .map_frame_limit
            .is_some_and(|hz| !dashboard_core::protocol::MAP_FRAME_LIMIT_RANGE.contains(&hz))
        {
            return Err("Invalid map frame rate limit".into());
        }
        Ok(())
    }
    fn map_ui_scale_action(&self) -> dashboard_core::protocol::MapAction {
        dashboard_core::protocol::MapAction::UiScale {
            scale: self.map_ui_scale.map(|percent| (percent / 100.0) as f32),
        }
    }
    fn map_frame_limit_action(&self) -> dashboard_core::protocol::MapAction {
        dashboard_core::protocol::MapAction::FrameLimit {
            hz: self.map_frame_limit,
        }
    }
}
pub struct VideoState(
    pub Mutex<Result<VideoSettings, String>>,
    AtomicBool,
    AtomicU64,
    Mutex<()>,
    Mutex<[Option<String>; 2]>,
);
impl Default for VideoState {
    fn default() -> Self {
        let requested = std::env::args_os()
            .skip(1)
            .any(|argument| argument == "--diagnostics");
        Self(
            Mutex::new(load()),
            AtomicBool::new(requested),
            AtomicU64::new(0),
            Mutex::new(()),
            Mutex::new([None, None]),
        )
    }
}
fn path() -> Result<PathBuf, String> {
    Ok(
        PathBuf::from(std::env::var_os("APPDATA").ok_or("User configuration folder unavailable")?)
            .join("BattlePlan")
            .join("video.json"),
    )
}
fn load() -> Result<VideoSettings, String> {
    load_from(&path()?)
}
fn load_from(path: &Path) -> Result<VideoSettings, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(VideoSettings::default());
        }
        Err(error) => return Err(error.to_string()),
    };
    if bytes.len() > 4096 {
        return Err("Video settings exceed their size limit".into());
    }
    let settings: VideoSettings = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    settings.validate()?;
    Ok(settings)
}
fn update(
    path: &Path,
    fallback: Option<&VideoSettings>,
    change: impl FnOnce(&mut VideoSettings),
) -> Result<(VideoSettings, Option<String>), String> {
    replay_settings::exit::EXIT
        .under_lock(&path.with_extension("lock"), replay_settings::exit::What::Change, |receipt| {
            let mut settings = match load_from(path) {
                Ok(saved) => saved,
                Err(error) => fallback.cloned().ok_or_else(|| io::Error::other(error))?,
            };
            change(&mut settings);
            let applied = receipt.replace(path, |out| serde_json::to_writer(out, &settings).map_err(io::Error::from))?;
            Ok((settings, applied.warning))
        })
        .map_err(|e| e.to_string())
}
type Stored = Mutex<Result<VideoSettings, String>>;
fn stored(state: &Stored) -> Result<std::sync::MutexGuard<'_, Result<VideoSettings, String>>, String> {
    state.lock().map_err(|e| e.to_string())
}
fn commit(
    path: &Path,
    state: &Stored,
    change: impl FnOnce(&mut VideoSettings),
    take: impl FnOnce(&mut VideoSettings, &VideoSettings),
) -> Result<(VideoSettings, Option<String>), String> {
    let fallback = stored(state)?.as_ref().ok().cloned();
    let (saved, warning) = update(path, fallback.as_ref(), change)?;
    let mut current = stored(state)?;
    match &mut *current {
        Ok(current) => take(current, &saved),
        Err(_) => *current = Ok(saved.clone()),
    }
    Ok((saved, warning))
}
fn with_display(saved: &mut VideoSettings, form: &VideoSettings) {
    *saved = VideoSettings {
        map_diagnostics: saved.map_diagnostics,
        map_muted: saved.map_muted,
        ..form.clone()
    };
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoReply {
    settings: VideoSettings,
    game_settings: Option<VideoSettings>,
    game_settings_error: Option<String>,
    game_map_ui_scale: Option<f64>,
    map_ui_scale_choices: &'static [u32],
    warning: Option<String>,
}
fn read_game_settings<T>(read: impl FnOnce(&Path) -> io::Result<T>) -> Result<T, String> {
    let (config, _) = crate::dashboard_service::host_config()?;
    if config.user_dir.as_os_str().is_empty() {
        return Err("Choose the Paradox user folder in Settings".into());
    }
    engine_executable::executable_tables::or_installed(&config.game_dir, || read(&config.user_dir))
        .map_err(|error| error.to_string())
}
fn game_settings() -> Result<VideoSettings, String> {
    parse_game_display(&read_game_settings(
        content_reader_settings::settings::read_keys,
    )?)
}
fn game_map_ui_scale() -> Option<f64> {
    let display = read_game_settings(content_reader_settings::settings::read_display).ok()?;
    let scale = display.screen_scale(&content_reader_settings::settings::desktops().ok()?)?;
    dashboard_core::protocol::MAP_UI_SCALE_RANGE
        .contains(&scale)
        .then(|| (f64::from(scale) * 100.0).round())
}
fn parse_game_display(
    keys: &content_reader_settings::settings::SettingsKeys,
) -> Result<VideoSettings, String> {
    use content_reader_settings::settings::constructed;
    if !keys.is_absent() && !keys.has(&["graphics"]) {
        return Err("Missing graphics block in game settings".to_owned());
    }
    let scale = f64::from(
        keys.f32_or_constructed(&["graphics", "gui_scale"], constructed::GUI_SCALE)
            .filter(|scale| scale.is_finite())
            .ok_or("Invalid game UI scale")?,
    );
    let vsync = keys
        .bool_or_constructed(&["graphics", "vsync"], constructed::VSYNC)
        .ok_or("Missing vsync in game settings")?;
    let settings = VideoSettings {
        use_game_settings: true,
        ui_scale: (scale * 100.0).clamp(*UI_SCALE_RANGE.start(), *UI_SCALE_RANGE.end()),
        vsync,
        map_ui_scale: None,
        map_diagnostics: false,
        map_muted: false,
        map_frame_limit: None,
    };
    settings.validate()?;
    Ok(settings)
}
fn reply(settings: VideoSettings) -> VideoReply {
    let (game_settings, game_settings_error) = match game_settings() {
        Ok(game) => (Some(inherit_game(&settings, game)), None),
        Err(error) => (None, Some(error)),
    };
    VideoReply {
        settings,
        game_settings,
        game_settings_error,
        game_map_ui_scale: game_map_ui_scale(),
        map_ui_scale_choices: &dashboard_core::protocol::MAP_UI_SCALE_CHOICES,
        warning: None,
    }
}
pub fn launch_actions(
    window: &WebviewWindow,
) -> Result<[dashboard_core::protocol::MapAction; 3], String> {
    let settings = window
        .state::<VideoState>()
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .unwrap_or_default();
    let vsync = if settings.use_game_settings {
        game_settings()
            .map(|game| game.vsync)
            .unwrap_or(settings.vsync)
    } else {
        settings.vsync
    };
    let map_ui_scale = if settings.use_game_settings {
        VideoSettings::default()
    } else {
        settings.clone()
    }
    .map_ui_scale_action();
    Ok([
        dashboard_core::protocol::MapAction::Vsync { enabled: vsync },
        map_ui_scale,
        settings.map_frame_limit_action(),
    ])
}
fn apply_scale(window: &WebviewWindow, settings: &VideoSettings) -> Result<(), String> {
    window
        .set_zoom(settings.ui_scale / 100.0)
        .map_err(|e| e.to_string())
}
pub fn restore(window: &WebviewWindow) -> Result<(), String> {
    let state = window.state::<VideoState>();
    let mut stored = state.0.lock().map_err(|e| e.to_string())?;
    if let Ok(settings) = &mut *stored {
        if settings.use_game_settings
            && let Ok(game) = game_settings()
        {
            *settings = inherit_game(settings, game);
        }
        apply_scale(window, settings)?;
    }
    Ok(())
}
#[tauri::command]
pub async fn video_settings(window: WebviewWindow) -> Result<VideoReply, String> {
    tauri::async_runtime::spawn_blocking(move || read_video_settings(&window))
        .await
        .map_err(|error| error.to_string())?
}
fn read_video_settings(window: &WebviewWindow) -> Result<VideoReply, String> {
    let state = window.state::<VideoState>();
    let host = window.state::<NativeHost>();
    let _changing = changing(&state);
    let mut settings = stored(&state.0)?.clone()?;
    if settings.use_game_settings
        && let Ok(game) = game_settings()
    {
        if game.ui_scale != settings.ui_scale {
            apply_scale(window, &game)?;
        }
        host.set_display(dashboard_core::protocol::MapAction::Vsync {
            enabled: game.vsync,
        });
        window
            .state::<crate::popout::PopOuts>()
            .mirror(&dashboard_core::protocol::MapAction::Vsync {
                enabled: game.vsync,
            });
        let mut current = stored(&state.0)?;
        if let Ok(current) = &mut *current
            && current.use_game_settings
        {
            *current = inherit_game(current, game);
            settings = current.clone();
        }
    }
    Ok(reply(settings))
}
#[tauri::command]
pub async fn set_video_settings(
    window: WebviewWindow,
    settings: VideoSettings,
) -> Result<VideoReply, String> {
    tauri::async_runtime::spawn_blocking(move || save_video_settings(&window, settings))
        .await
        .map_err(|error| error.to_string())?
}
fn save_video_settings(window: &WebviewWindow, settings: VideoSettings) -> Result<VideoReply, String> {
    let state = window.state::<VideoState>();
    let host = window.state::<NativeHost>();
    let _changing = changing(&state);
    let mut settings = if settings.use_game_settings {
        game_settings()?
    } else {
        settings
    };
    settings.validate()?;
    let status = host.status();
    if status.running && !status.ready {
        return Err(
            "Wait for the map to start before changing display settings".into(),
        );
    }
    let before = stored(&state.0)?.clone();
    let previous = before.clone().unwrap_or_default();
    settings.map_diagnostics = match &before {
        Ok(stored) => stored.map_diagnostics,
        Err(_) => load().is_ok_and(|saved| saved.map_diagnostics),
    };
    settings.map_muted = match &before {
        Ok(stored) => stored.map_muted,
        Err(_) => load().is_ok_and(|saved| saved.map_muted),
    };
    if settings.use_game_settings {
        settings.map_frame_limit = match &before {
            Ok(stored) => stored.map_frame_limit,
            Err(_) => load().ok().and_then(|saved| saved.map_frame_limit),
        };
    }
    let result = (|| {
        if before.is_err() || settings.ui_scale != previous.ui_scale {
            apply_scale(window, &settings)?;
        }
        host.set_display(dashboard_core::protocol::MapAction::Vsync {
            enabled: settings.vsync,
        });
        host.set_display(settings.map_ui_scale_action());
        host.set_display(settings.map_frame_limit_action());
        let (_, warning) = update(&path()?, Some(&settings), |saved| {
            with_display(saved, &settings)
        })?;
        let popouts = window.state::<crate::popout::PopOuts>();
        popouts.mirror(&dashboard_core::protocol::MapAction::Vsync {
            enabled: settings.vsync,
        });
        popouts.mirror(&settings.map_ui_scale_action());
        popouts.mirror(&settings.map_frame_limit_action());
        Ok(warning)
    })();
    let warning = match result {
        Ok(warning) => warning,
        Err(error) => {
        let _ = apply_scale(window, &previous);
        host.set_display(dashboard_core::protocol::MapAction::Vsync {
            enabled: previous.vsync,
        });
        host.set_display(previous.map_ui_scale_action());
        host.set_display(previous.map_frame_limit_action());
        return Err(error);
        }
    };
    let settings = {
        let mut current = stored(&state.0)?;
        match &mut *current {
            Ok(current) => with_display(current, &settings),
            Err(_) => *current = Ok(settings),
        }
        current.clone()?
    };
    Ok(VideoReply { warning, ..reply(settings) })
}

fn inherit_game(current: &VideoSettings, game: VideoSettings) -> VideoSettings {
    VideoSettings {
        map_diagnostics: current.map_diagnostics,
        map_muted: current.map_muted,
        map_frame_limit: current.map_frame_limit,
        ..game
    }
}

fn effective_map_diagnostics(requested: bool, stored: &Result<VideoSettings, String>) -> bool {
    requested
        || stored
            .as_ref()
            .is_ok_and(|settings| settings.map_diagnostics)
}

#[derive(Clone, Debug, Serialize)]
pub struct MapSwitch {
    value: bool,
    sequence: u64,
    warning: Option<String>,
}

const DIAGNOSTICS: usize = 0;
const MUTED: usize = 1;

fn warnings(state: &VideoState) -> std::sync::MutexGuard<'_, [Option<String>; 2]> {
    state.4.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn read_switch(
    state: &VideoState,
    which: usize,
    value: impl FnOnce(&Result<VideoSettings, String>) -> bool,
) -> MapSwitch {
    let stored = state.0.lock();
    MapSwitch {
        value: stored.as_deref().is_ok_and(value),
        sequence: state.2.load(Ordering::Acquire),
        warning: warnings(state)[which].clone(),
    }
}

fn publish(state: &VideoState, which: usize, value: bool, warning: Option<String>) -> MapSwitch {
    warnings(state)[which].clone_from(&warning);
    MapSwitch { value, sequence: next_change(state), warning }
}

fn next_change(state: &VideoState) -> u64 {
    state.2.fetch_add(1, Ordering::AcqRel) + 1
}

pub fn current_map_diagnostics(state: &VideoState) -> bool {
    diagnostics_switch(state).value
}

fn diagnostics_switch(state: &VideoState) -> MapSwitch {
    read_switch(state, DIAGNOSTICS, |stored| {
        effective_map_diagnostics(state.1.load(Ordering::Acquire), stored)
    })
}

fn changing(state: &VideoState) -> std::sync::MutexGuard<'_, ()> {
    state.3.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub type ToggleFailed = (String, Option<bool>);

fn set_diagnostics(path: &Path, state: &VideoState, enabled: Option<bool>) -> Result<(bool, Option<String>), ToggleFailed> {
    let consumed = std::cell::Cell::new(false);
    let decided = std::cell::Cell::new(None);
    let saved = commit(
        path,
        &state.0,
        |settings| {
            let requested = state.1.swap(false, Ordering::AcqRel);
            consumed.set(requested);
            let wanted = enabled.unwrap_or(!(requested || settings.map_diagnostics));
            decided.set(Some(wanted));
            settings.map_diagnostics = wanted;
        },
        |current, saved| current.map_diagnostics = saved.map_diagnostics,
    );
    if saved.is_err() && consumed.get() {
        state.1.store(true, Ordering::Release);
    }
    saved.map(|(saved, warning)| (saved.map_diagnostics, warning)).map_err(|error| (error, decided.get()))
}

fn toggle_diagnostics<T>(path: &Path, state: &VideoState, enabled: Option<bool>, announce: impl FnOnce(bool, Option<String>, bool) -> T) -> Result<T, ToggleFailed> {
    let _changing = changing(state);
    let muted = muted_in(&*stored(&state.0).map_err(|error| (error, None))?);
    let (enabled, warning) = set_diagnostics(path, state, enabled)?;
    Ok(announce(enabled, warning, muted))
}

pub fn diagnostics_failed(app: &AppHandle, enabled: Option<bool>, (error, decided): &ToggleFailed) {
    let wanted = decided.or(enabled).unwrap_or_else(|| !diagnostics_switch(&app.state::<VideoState>()).value);
    let _ = app.emit("map-diagnostics-error", serde_json::json!({ "error": error, "wanted": wanted }));
}

pub fn change_map_diagnostics(app: &AppHandle, enabled: Option<bool>) -> Result<MapSwitch, ToggleFailed> {
    let state = app.state::<VideoState>();
    let (changed, muted) = toggle_diagnostics(&path().map_err(|error| (error, None))?, &state, enabled, |enabled, warning, muted| {
        let changed = announce_diagnostics(app, &state, enabled, warning);
        let now_muted = stored(&state.0).map_or(muted, |stored| muted_in(&stored));
        (changed, (now_muted != muted).then(|| announce_muted(app, &state, !muted, None)))
    })?;
    let _ = app.emit("map-diagnostics", &changed);
    if let Some(muted) = muted {
        let _ = app.emit("map-muted", muted);
    }
    Ok(changed)
}

fn muted_in(stored: &Result<VideoSettings, String>) -> bool {
    stored.as_ref().is_ok_and(|settings| settings.map_muted)
}

fn announce_muted(app: &AppHandle, state: &VideoState, muted: bool, warning: Option<String>) -> MapSwitch {
    app.state::<NativeHost>().set_muted(muted);
    app.state::<crate::popout::PopOuts>()
        .mirror(&dashboard_core::protocol::MapAction::Mute { muted });
    publish(state, MUTED, muted, warning)
}

fn announce_diagnostics(app: &AppHandle, state: &VideoState, enabled: bool, warning: Option<String>) -> MapSwitch {
    app.state::<NativeHost>().set_diagnostics(enabled);
    app.state::<crate::popout::PopOuts>()
        .mirror(&dashboard_core::protocol::MapAction::Diagnostics { enabled });
    publish(state, DIAGNOSTICS, enabled, warning)
}

pub fn current_map_muted(state: &VideoState) -> bool {
    muted_switch(state).value
}

fn muted_switch(state: &VideoState) -> MapSwitch {
    read_switch(state, MUTED, muted_in)
}

pub fn change_map_muted(app: &AppHandle, muted: bool) -> Result<MapSwitch, String> {
    let state = app.state::<VideoState>();
    let _changing = changing(&state);
    let diagnostics = effective_map_diagnostics(state.1.load(Ordering::Acquire), &*stored(&state.0)?);
    let (_, warning) = commit(&path()?, &state.0, |settings| settings.map_muted = muted, |current, saved| current.map_muted = saved.map_muted)?;
    let changed = announce_muted(app, &state, muted, warning);
    let now = effective_map_diagnostics(state.1.load(Ordering::Acquire), &*stored(&state.0)?);
    let diagnostics = (now != diagnostics).then(|| announce_diagnostics(app, &state, !diagnostics, None));
    let _ = app.emit("map-muted", &changed);
    if let Some(diagnostics) = diagnostics {
        let _ = app.emit("map-diagnostics", diagnostics);
    }
    Ok(changed)
}

#[tauri::command]
pub fn map_muted(state: State<'_, VideoState>) -> MapSwitch {
    muted_switch(&state)
}

#[tauri::command]
pub async fn set_map_muted(app: AppHandle, muted: bool) -> Result<MapSwitch, String> {
    tauri::async_runtime::spawn_blocking(move || change_map_muted(&app, muted))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn map_diagnostics(state: State<'_, VideoState>) -> MapSwitch {
    diagnostics_switch(&state)
}

#[tauri::command]
pub async fn set_map_diagnostics(app: AppHandle, enabled: Option<bool>) -> Result<MapSwitch, String> {
    tauri::async_runtime::spawn_blocking(move || {
        change_map_diagnostics(&app, enabled).map_err(|failed| {
            diagnostics_failed(&app, enabled, &failed);
            failed.0
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enter_display_settings() {
        content_reader_core::test_game::with(|game| {
            game.words(["graphics", "gui_scale", "fullscreen", "borderless", "vsync"])
        });
    }

    #[test]
    fn game_display_defaults_keep_scale_and_vsync_without_resizing() {
        enter_display_settings();
        let keys = content_reader_settings::settings::SettingsKeys::read(b"graphics={size={x=2048 y=1152} gui_scale=1.125 fullScreen=no borderless=yes VSync=no}");
        let settings = parse_game_display(&keys).unwrap();
        assert_eq!(settings.ui_scale, 112.5);
        assert!(settings.use_game_settings && !settings.vsync);
        assert_eq!(settings.map_ui_scale, None);
        let old: VideoSettings = serde_json::from_str(
            r#"{"width":1500,"height":950,"uiScale":100,"fullscreen":false,"vsync":true}"#,
        )
        .unwrap();
        assert!(!old.use_game_settings);
        assert!(!old.map_diagnostics && !settings.map_diagnostics);
        assert!(!VideoSettings::default().map_diagnostics);
        assert_eq!(old.map_ui_scale, None);
    }

    #[test]
    fn a_game_scale_outside_the_app_range_keeps_the_game_settings() {
        enter_display_settings();
        let parse = |text: &str| {
            parse_game_display(&content_reader_settings::settings::SettingsKeys::read(
                text.as_bytes(),
            ))
        };
        let large = parse("graphics={size={x=2048 y=1152} gui_scale=2.0 VSync=no}").unwrap();
        assert_eq!(large.ui_scale, 150.0);
        assert!(large.use_game_settings && !large.vsync);
        large.validate().unwrap();
        let small = parse("graphics={gui_scale=0.4 VSync=yes}").unwrap();
        assert_eq!(small.ui_scale, 75.0);
        assert!(small.vsync);
        assert_eq!(
            parse("graphics={gui_scale=inf VSync=no}").unwrap_err(),
            "Invalid game UI scale"
        );
        let manual = VideoSettings {
            use_game_settings: false,
            ui_scale: 200.0,
            ..VideoSettings::default()
        };
        assert!(manual.validate().is_err());
    }

    #[test]
    fn diagnostics_follow_the_flag_until_changed_and_survive_game_settings() {
        let on = VideoSettings {
            map_diagnostics: true,
            ..VideoSettings::default()
        };
        let off = Ok(VideoSettings::default());
        assert!(effective_map_diagnostics(true, &off));
        assert!(effective_map_diagnostics(false, &Ok(on.clone())));
        assert!(!effective_map_diagnostics(false, &off));
        assert!(!effective_map_diagnostics(false, &Err("unreadable".into())));
        let game = VideoSettings {
            ui_scale: 125.0,
            ..VideoSettings::default()
        };
        let inherited = inherit_game(&on, game);
        assert!(inherited.map_diagnostics && inherited.ui_scale == 125.0);
    }

    #[test]
    fn two_windows_changing_different_settings_keep_both() {
        let folder =
            std::env::temp_dir().join(format!("hoi4-video-settings-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        let path = folder.join("video.json");
        let first = Mutex::new(Ok(VideoSettings::default()));
        let second = Mutex::new(Ok(VideoSettings::default()));
        commit(&path, &first, |settings| settings.map_muted = true, |current, saved| current.map_muted = saved.map_muted).unwrap();
        commit(&path, &second, |settings| settings.map_diagnostics = true, |current, saved| current.map_diagnostics = saved.map_diagnostics).unwrap();
        let saved = load_from(&path).unwrap();
        assert!(saved.map_muted && saved.map_diagnostics);
        let second = second.into_inner().unwrap().unwrap();
        assert!(second.map_diagnostics && !second.map_muted);
        let form = VideoSettings {
            use_game_settings: false,
            ui_scale: 110.0,
            map_frame_limit: Some(60.0),
            ..VideoSettings::default()
        };
        update(&path, Some(&form), |saved| with_display(saved, &form)).unwrap();
        let saved = load_from(&path).unwrap();
        assert!(saved.map_muted && saved.map_diagnostics && saved.ui_scale == 110.0);
        assert_eq!(saved.map_frame_limit, Some(60.0));
        let lost = Mutex::new(Err("zoom failed".to_owned()));
        commit(&path, &lost, |settings| settings.map_muted = false, |current, saved| current.map_muted = saved.map_muted).unwrap();
        let lost = lost.into_inner().unwrap().unwrap();
        assert!(lost.map_diagnostics && !lost.map_muted && lost.ui_scale == 110.0);
        fs::write(&path, b"not json").unwrap();
        assert!(commit(&path, &Mutex::new(Err("unreadable".into())), |_| (), |_, _| ()).is_err());
        commit(&path, &Mutex::new(Ok(game_like(90.0))), |settings| settings.map_muted = true, |current, saved| current.map_muted = saved.map_muted).unwrap();
        let saved = load_from(&path).unwrap();
        assert!(saved.map_muted && saved.ui_scale == 90.0);
        fs::remove_dir_all(&folder).unwrap();
    }



    #[test]
    fn a_diagnostics_toggle_is_decided_under_the_lock_that_saves_it_so_two_toggles_never_collapse() {
        let folder = std::env::temp_dir().join(format!("hoi4-video-toggle-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        let path = folder.join("video.json");
        let state = VideoState(Mutex::new(Ok(VideoSettings::default())), AtomicBool::new(false), AtomicU64::new(0), Mutex::new(()), Mutex::new([None, None]));
        update(&path, None, |saved| saved.map_diagnostics = true).unwrap();
        assert!(!set_diagnostics(&path, &state, None).unwrap().0, "a toggle that waited on another writer flips what that writer saved, not what this window last read");
        assert!(!load_from(&path).unwrap().map_diagnostics);
        assert!(set_diagnostics(&path, &state, None).unwrap().0, "so a second toggle undoes the first");
        assert!(state.0.lock().unwrap().as_ref().unwrap().map_diagnostics);
        assert!(load_from(&path).unwrap().map_diagnostics);

        let requested = VideoState(Mutex::new(Ok(VideoSettings::default())), AtomicBool::new(true), AtomicU64::new(0), Mutex::new(()), Mutex::new([None, None]));
        assert!(!set_diagnostics(&path, &requested, None).unwrap().0, "--diagnostics shows them, so the first toggle hides them");
        assert!(set_diagnostics(&path, &requested, None).unwrap().0);
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn concurrent_diagnostics_toggles_announce_the_value_they_saved_last() {
        let folder = std::env::temp_dir().join(format!("hoi4-video-concurrent-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        let path = folder.join("video.json");
        let state = VideoState(Mutex::new(Ok(VideoSettings::default())), AtomicBool::new(true), AtomicU64::new(0), Mutex::new(()), Mutex::new([None, None]));
        let announced = Mutex::new(Vec::new());
        let announce = |value: bool, warning: Option<String>, _muted: bool| {
            assert!(state.3.try_lock().is_err(), "no other toggle decides, saves or announces while this one announces");
            let switch = publish(&state, DIAGNOSTICS, value, warning);
            announced.lock().unwrap().push(switch.clone());
            switch
        };
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..25 {
                        toggle_diagnostics(&path, &state, None, announce).unwrap();
                    }
                });
            }
        });
        let announced = announced.into_inner().unwrap();
        let last = announced.iter().max_by_key(|switch| switch.sequence).unwrap().clone();
        assert_eq!(last.value, load_from(&path).unwrap().map_diagnostics, "the newest announcement is what the file holds");
        assert!(announced.windows(2).all(|pair| pair[0].sequence < pair[1].sequence && pair[0].value != pair[1].value), "each toggle announces in the order it saved, and flips the one before it");
        assert!(!announced[0].value, "--diagnostics was consumed by the first toggle, under its file lock");
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn the_mute_choice_is_off_by_default_and_survives_game_settings() {
        assert!(!VideoSettings::default().map_muted);
        let current = VideoSettings {
            map_muted: true,
            ..VideoSettings::default()
        };
        assert!(inherit_game(&current, game_like(125.0)).map_muted);
    }

    #[test]
    fn the_frame_rate_limit_is_off_by_default_survives_game_settings_and_refuses_a_rate_no_map_draws()
    {
        assert_eq!(VideoSettings::default().map_frame_limit, None);
        let saved: VideoSettings =
            serde_json::from_str(r#"{"uiScale":100,"vsync":true}"#).unwrap();
        assert_eq!(saved.map_frame_limit, None);
        let limited = VideoSettings {
            use_game_settings: false,
            map_frame_limit: Some(60.0),
            ..VideoSettings::default()
        };
        limited.validate().unwrap();
        assert_eq!(
            inherit_game(&limited, game_like(125.0)).map_frame_limit,
            Some(60.0)
        );
        assert!(matches!(
            limited.map_frame_limit_action(),
            dashboard_core::protocol::MapAction::FrameLimit { hz: Some(60.0) }
        ));
        assert!(matches!(
            VideoSettings::default().map_frame_limit_action(),
            dashboard_core::protocol::MapAction::FrameLimit { hz: None }
        ));
        for bad in [0.0, -60.0, 0.5, 1_000.1, f64::NAN] {
            let invalid = VideoSettings {
                map_frame_limit: Some(bad),
                ..VideoSettings::default()
            };
            assert_eq!(
                invalid.validate().unwrap_err(),
                "Invalid map frame rate limit",
                "{bad}"
            );
        }
    }

    fn game_like(ui_scale: f64) -> VideoSettings {
        VideoSettings {
            use_game_settings: false,
            ui_scale,
            map_diagnostics: true,
            ..VideoSettings::default()
        }
    }

    #[test]
    fn a_manual_map_ui_scale_is_stored_in_percent_and_sent_as_a_factor() {
        let settings = VideoSettings {
            use_game_settings: false,
            map_ui_scale: Some(125.0),
            ..VideoSettings::default()
        };
        settings.validate().unwrap();
        assert!(matches!(
            settings.map_ui_scale_action(),
            dashboard_core::protocol::MapAction::UiScale { scale: Some(1.25) }
        ));
        assert!(matches!(
            VideoSettings::default().map_ui_scale_action(),
            dashboard_core::protocol::MapAction::UiScale { scale: None }
        ));
        for bad in [10.0, 400.0, f64::NAN] {
            let invalid = VideoSettings {
                map_ui_scale: Some(bad),
                ..VideoSettings::default()
            };
            assert!(invalid.validate().is_err(), "{bad}");
        }
    }
}
