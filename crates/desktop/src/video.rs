//! Application UI scale, the native map's interface scale, presentation settings
//! and the maps' diagnostics and mute switches. Never changes the user's monitor mode or installed content.
use crate::native::NativeHost;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

#[derive(Clone, Debug, Deserialize, Serialize)]
// Ignore legacy window dimensions/mode when reading existing preferences.
#[serde(rename_all = "camelCase")]
pub struct VideoSettings {
    #[serde(default)]
    pub use_game_settings: bool,
    pub ui_scale: f64,
    pub vsync: bool,
    /// The native map interface's scale in percent; `None` matches the game on screen.
    #[serde(default)]
    pub map_ui_scale: Option<f64>,
    /// Whether maps show technical stats and data limitations. Off by default.
    /// It is not a game setting: inheriting the game's display settings and the
    /// Apply form keep it, and only [`set_map_diagnostics`] changes it.
    #[serde(default)]
    pub map_diagnostics: bool,
    /// Whether the maps' audio is muted. Off by default. Not a game setting
    /// either: only [`set_map_muted`] changes it.
    #[serde(default)]
    pub map_muted: bool,
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
        }
    }
}
impl VideoSettings {
    fn validate(&self) -> Result<(), String> {
        if !self.ui_scale.is_finite() || !(75.0..=150.0).contains(&self.ui_scale) {
            return Err("Invalid UI scale".into());
        }
        if self.map_ui_scale.is_some_and(|percent| {
            !dashboard::protocol::MAP_UI_SCALE_RANGE.contains(&((percent / 100.0) as f32))
        }) {
            return Err("Invalid map UI scale".into());
        }
        Ok(())
    }
    /// The viewer's map action for the map interface scale.
    fn map_ui_scale_action(&self) -> dashboard::protocol::MapAction {
        dashboard::protocol::MapAction::UiScale {
            scale: self.map_ui_scale.map(|percent| (percent / 100.0) as f32),
        }
    }
}
/// The saved settings, whether `--diagnostics` turned the maps'
/// diagnostics on for this run without saving it, and how many times the
/// maps' switches changed, counted under the settings' lock.
pub struct VideoState(
    pub Mutex<Result<VideoSettings, String>>,
    AtomicBool,
    AtomicU64,
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
        )
    }
}
fn path() -> Result<PathBuf, String> {
    Ok(
        PathBuf::from(std::env::var_os("APPDATA").ok_or("User configuration folder unavailable")?)
            .join("battle-plan")
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
/// Re-read under the lock so another window's saved choices are not overwritten.
fn update(
    path: &Path,
    fallback: Option<&VideoSettings>,
    change: impl FnOnce(&mut VideoSettings),
) -> Result<VideoSettings, String> {
    let folder = path.parent().ok_or("Video settings have no folder")?;
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("lock"))
        .map_err(|e| e.to_string())?;
    lock.lock().map_err(|e| e.to_string())?;
    let mut settings = match load_from(path) {
        Ok(saved) => saved,
        Err(error) => fallback.cloned().ok_or(error)?,
    };
    change(&mut settings);
    // The lock makes this process the only writer, so its staging file is its own.
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(
        &temp,
        serde_json::to_vec(&settings).map_err(|e| e.to_string())?,
    )
    .and_then(|()| fs::rename(&temp, path))
    .map_err(|e| e.to_string())?;
    Ok(settings)
}
/// Keep this window's applied settings; recover unreadable local settings from the saved ones.
fn commit(
    path: &Path,
    stored: &mut Result<VideoSettings, String>,
    change: impl Fn(&mut VideoSettings),
) -> Result<(), String> {
    let saved = update(path, stored.as_ref().ok(), &change)?;
    if let Ok(current) = stored {
        change(current);
    } else {
        *stored = Ok(saved);
    }
    Ok(())
}
/// Diagnostics and mute are absent from the display form and must stay as saved.
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
    /// What "match the game" works out to on these monitors, in percent.
    game_map_ui_scale: Option<f64>,
    /// The manual map interface scales to offer, in percent.
    map_ui_scale_choices: &'static [u32],
}
/// The host's effective user folder, after initializing the selected game's tables.
fn game_user_folder() -> Result<PathBuf, String> {
    let (config, _) = crate::dashboard_service::host_config()?;
    if config.user_dir.as_os_str().is_empty() {
        return Err("Choose the Paradox user folder in Settings".into());
    }
    engine_executable::executable_tables::initialize(&config.game_dir)
        .map_err(|error| format!("Game tables: {error}"))?;
    Ok(config.user_dir)
}
fn game_settings() -> Result<VideoSettings, String> {
    let user = game_user_folder()?;
    parse_game_display(&installed_content_settings::settings::read_keys(&user).map_err(|e| e.to_string())?)
}
/// The viewer computes the same from the recording's user folder and the same monitors.
fn game_map_ui_scale() -> Option<f64> {
    let user = game_user_folder().ok()?;
    let display = installed_content_settings::settings::read_display(&user).ok()?;
    let scale = display.screen_scale(&installed_content_settings::settings::desktops().ok()?)?;
    dashboard::protocol::MAP_UI_SCALE_RANGE
        .contains(&scale)
        .then(|| (f64::from(scale) * 100.0).round())
}
/// The preferences from `settings.txt`'s keys as the game reads them
/// ([`installed_content_settings::settings::SettingsKeys`]: any case, the last stored value of each field).
fn parse_game_display(
    keys: &installed_content_settings::settings::SettingsKeys,
) -> Result<VideoSettings, String> {
    if !keys.has(&["graphics"]) {
        return Err("Missing graphics block in game settings".to_owned());
    }
    let scale = f64::from(
        keys.f32_at(&["graphics", "gui_scale"])
            .ok_or("Invalid game UI scale")?,
    );
    let vsync = keys
        .bool_at(&["graphics", "vsync"])
        .ok_or("Missing vsync in game settings")?;
    let settings = VideoSettings {
        use_game_settings: true,
        ui_scale: scale * 100.0,
        vsync,
        map_ui_scale: None,
        map_diagnostics: false,
        map_muted: false,
    };
    settings.validate()?;
    Ok(settings)
}
fn reply(settings: VideoSettings) -> VideoReply {
    let (game_settings, game_settings_error) = match game_settings() {
        Ok(settings) => (Some(settings), None),
        Err(error) => (None, Some(error)),
    };
    VideoReply {
        settings,
        game_settings,
        game_settings_error,
        game_map_ui_scale: game_map_ui_scale(),
        map_ui_scale_choices: &dashboard::protocol::MAP_UI_SCALE_CHOICES,
    }
}
/// Resolve at process launch too, including after the user configured folders:
/// the vsync and map interface scale actions a new viewer starts with.
pub fn launch_actions(
    window: &WebviewWindow,
) -> Result<[dashboard::protocol::MapAction; 2], String> {
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
    // Using the game's settings also matches its interface scale.
    let map_ui_scale = if settings.use_game_settings {
        VideoSettings::default()
    } else {
        settings
    }
    .map_ui_scale_action();
    Ok([
        dashboard::protocol::MapAction::Vsync { enabled: vsync },
        map_ui_scale,
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
pub fn video_settings(
    window: WebviewWindow,
    state: State<'_, VideoState>,
    host: State<'_, NativeHost>,
) -> Result<VideoReply, String> {
    let mut stored = state.0.lock().map_err(|e| e.to_string())?;
    let mut settings = stored.clone()?;
    if settings.use_game_settings
        && let Ok(game) = game_settings()
    {
        if game.ui_scale != settings.ui_scale {
            apply_scale(&window, &game)?;
        }
        host.set_display(dashboard::protocol::MapAction::Vsync {
            enabled: game.vsync,
        });
        window
            .state::<crate::popout::PopOuts>()
            .mirror(&dashboard::protocol::MapAction::Vsync {
                enabled: game.vsync,
            });
        settings = inherit_game(&settings, game);
        *stored = Ok(settings.clone());
    }
    Ok(reply(settings))
}
#[tauri::command]
pub fn set_video_settings(
    window: WebviewWindow,
    state: State<'_, VideoState>,
    host: State<'_, NativeHost>,
    settings: VideoSettings,
) -> Result<VideoReply, String> {
    let mut settings = if settings.use_game_settings {
        game_settings()?
    } else {
        settings
    };
    settings.validate()?;
    let status = host.status();
    if status.running && !status.ready {
        return Err(
            "Wait for the native map to finish starting before changing display settings".into(),
        );
    }
    let mut stored = state.0.lock().map_err(|e| e.to_string())?;
    let previous = stored.clone().unwrap_or_default();
    // The form does not carry the diagnostics choice; keep the saved one, from
    // the file when the settings could not be restored.
    settings.map_diagnostics = match &*stored {
        Ok(stored) => stored.map_diagnostics,
        Err(_) => load().is_ok_and(|saved| saved.map_diagnostics),
    };
    settings.map_muted = match &*stored {
        Ok(stored) => stored.map_muted,
        Err(_) => load().is_ok_and(|saved| saved.map_muted),
    };
    let result = (|| {
        if stored.is_err() || settings.ui_scale != previous.ui_scale {
            apply_scale(&window, &settings)?;
        }
        // Kept for a viewer starting now too, which takes them over its launch-time read.
        host.set_display(dashboard::protocol::MapAction::Vsync {
            enabled: settings.vsync,
        });
        host.set_display(settings.map_ui_scale_action());
        update(&path()?, Some(&settings), |saved| {
            with_display(saved, &settings)
        })?;
        let popouts = window.state::<crate::popout::PopOuts>();
        popouts.mirror(&dashboard::protocol::MapAction::Vsync {
            enabled: settings.vsync,
        });
        popouts.mirror(&settings.map_ui_scale_action());
        Ok(())
    })();
    if let Err(error) = result {
        let _ = apply_scale(&window, &previous);
        host.set_display(dashboard::protocol::MapAction::Vsync {
            enabled: previous.vsync,
        });
        host.set_display(previous.map_ui_scale_action());
        return Err(error);
    }
    *stored = Ok(settings.clone());
    Ok(reply(settings))
}

/// Settings taken over from the game's display settings. The diagnostics
/// choice is not a game setting, so the current one is kept.
fn inherit_game(current: &VideoSettings, game: VideoSettings) -> VideoSettings {
    VideoSettings {
        map_diagnostics: current.map_diagnostics,
        map_muted: current.map_muted,
        ..game
    }
}

/// Whether maps show diagnostics: `--diagnostics` for this run until the user
/// changes it, otherwise the saved choice.
fn effective_map_diagnostics(requested: bool, stored: &Result<VideoSettings, String>) -> bool {
    requested
        || stored
            .as_ref()
            .is_ok_and(|settings| settings.map_diagnostics)
}

/// One of the maps' switches as the host holds it, with the number of the
/// switches' last change it reflects. Events and replies reach the frontend in
/// any order, so it keeps the value with the highest number.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct MapSwitch {
    value: bool,
    sequence: u64,
}

/// A switch's value and the number of the last change, read under the settings' lock.
fn read_switch(
    state: &VideoState,
    value: impl FnOnce(&Result<VideoSettings, String>) -> bool,
) -> MapSwitch {
    let stored = state.0.lock();
    MapSwitch {
        value: stored.as_deref().is_ok_and(value),
        sequence: state.2.load(Ordering::Acquire),
    }
}

/// Numbers a change to the switches; the caller holds the settings' lock.
fn next_change(state: &VideoState) -> u64 {
    state.2.fetch_add(1, Ordering::AcqRel) + 1
}

pub fn current_map_diagnostics(state: &VideoState) -> bool {
    diagnostics_switch(state).value
}

fn diagnostics_switch(state: &VideoState) -> MapSwitch {
    read_switch(state, |stored| {
        effective_map_diagnostics(state.1.load(Ordering::Acquire), stored)
    })
}

/// Saves the maps' diagnostics choice (`None` flips it), sends it to the
/// native viewer and tells the frontend. Returns the new choice.
pub fn change_map_diagnostics(app: &AppHandle, enabled: Option<bool>) -> Result<MapSwitch, String> {
    let state = app.state::<VideoState>();
    let mut stored = state.0.lock().map_err(|e| e.to_string())?;
    let enabled = enabled
        .unwrap_or_else(|| !effective_map_diagnostics(state.1.load(Ordering::Acquire), &stored));
    let muted = muted_in(&stored);
    commit(&path()?, &mut stored, |settings| {
        settings.map_diagnostics = enabled
    })?;
    state.1.store(false, Ordering::Release);
    // Sent before the lock is released, so two changes reach the viewer in the
    // order they were saved.
    let changed = announce_diagnostics(app, &state, enabled);
    // Saving over settings that could not be read loads the file, which can change the other switch.
    let muted = (muted_in(&stored) != muted).then(|| announce_muted(app, &state, !muted));
    drop(stored);
    let _ = app.emit("map-diagnostics", changed);
    if let Some(muted) = muted {
        let _ = app.emit("map-muted", muted);
    }
    Ok(changed)
}

/// The saved mute choice in `stored`; unreadable settings leave the maps sounding.
fn muted_in(stored: &Result<VideoSettings, String>) -> bool {
    stored.as_ref().is_ok_and(|settings| settings.map_muted)
}

/// Sends a mute choice to every map, numbered for the frontend; the caller holds the lock.
fn announce_muted(app: &AppHandle, state: &VideoState, muted: bool) -> MapSwitch {
    app.state::<NativeHost>().set_muted(muted);
    app.state::<crate::popout::PopOuts>()
        .mirror(&dashboard::protocol::MapAction::Mute { muted });
    MapSwitch {
        value: muted,
        sequence: next_change(state),
    }
}

/// Sends a diagnostics choice to every map, numbered for the frontend; the caller holds the lock.
fn announce_diagnostics(app: &AppHandle, state: &VideoState, enabled: bool) -> MapSwitch {
    app.state::<NativeHost>().set_diagnostics(enabled);
    app.state::<crate::popout::PopOuts>()
        .mirror(&dashboard::protocol::MapAction::Diagnostics { enabled });
    MapSwitch {
        value: enabled,
        sequence: next_change(state),
    }
}

/// The saved mute choice; an unreadable file leaves the maps sounding.
pub fn current_map_muted(state: &VideoState) -> bool {
    muted_switch(state).value
}

fn muted_switch(state: &VideoState) -> MapSwitch {
    read_switch(state, muted_in)
}

/// Saves the maps' mute choice, sends it to the native viewer and tells the
/// frontend. Returns the new choice.
pub fn change_map_muted(app: &AppHandle, muted: bool) -> Result<MapSwitch, String> {
    let state = app.state::<VideoState>();
    let mut stored = state.0.lock().map_err(|e| e.to_string())?;
    let diagnostics = effective_map_diagnostics(state.1.load(Ordering::Acquire), &stored);
    commit(&path()?, &mut stored, |settings| settings.map_muted = muted)?;
    // Sent before the lock is released, so changes arrive in saved order.
    let changed = announce_muted(app, &state, muted);
    // Saving over settings that could not be read loads the file, which can change the other switch.
    let diagnostics = (effective_map_diagnostics(state.1.load(Ordering::Acquire), &stored)
        != diagnostics)
        .then(|| announce_diagnostics(app, &state, !diagnostics));
    drop(stored);
    let _ = app.emit("map-muted", changed);
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
pub fn set_map_muted(app: AppHandle, muted: bool) -> Result<MapSwitch, String> {
    change_map_muted(&app, muted)
}

#[tauri::command]
pub fn map_diagnostics(state: State<'_, VideoState>) -> MapSwitch {
    diagnostics_switch(&state)
}

#[tauri::command]
pub fn set_map_diagnostics(app: AppHandle, enabled: Option<bool>) -> Result<MapSwitch, String> {
    // Settings shows the failure even when the keyboard shortcut asked.
    change_map_diagnostics(&app, enabled).inspect_err(|error| {
        let _ = app.emit("map-diagnostics-error", error);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn game_display_defaults_keep_scale_and_vsync_without_resizing() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let keys = installed_content_settings::settings::SettingsKeys::read(b"graphics={size={x=2048 y=1152} gui_scale=1.125 fullScreen=no borderless=yes VSync=no}");
        let settings = parse_game_display(&keys).unwrap();
        assert_eq!(settings.ui_scale, 112.5);
        assert!(settings.use_game_settings && !settings.vsync);
        assert_eq!(settings.map_ui_scale, None);
        let old: VideoSettings = serde_json::from_str(
            r#"{"width":1500,"height":950,"uiScale":100,"fullscreen":false,"vsync":true}"#,
        )
        .unwrap();
        assert!(!old.use_game_settings);
        // Diagnostics stay off unless a saved file turned them on.
        assert!(!old.map_diagnostics && !settings.map_diagnostics);
        assert!(!VideoSettings::default().map_diagnostics);
        assert_eq!(old.map_ui_scale, None);
    }

    #[test]
    fn diagnostics_follow_the_flag_until_changed_and_survive_game_settings() {
        let on = VideoSettings {
            map_diagnostics: true,
            ..VideoSettings::default()
        };
        let off = Ok(VideoSettings::default());
        // `--diagnostics` shows them for the run whatever is saved; without it
        // the saved choice decides, and an unreadable file means off.
        assert!(effective_map_diagnostics(true, &off));
        assert!(effective_map_diagnostics(false, &Ok(on.clone())));
        assert!(!effective_map_diagnostics(false, &off));
        assert!(!effective_map_diagnostics(false, &Err("unreadable".into())));
        // Inheriting the game's display settings (restore, video_settings)
        // keeps the choice.
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
        let mut first = Ok(VideoSettings::default());
        let mut second = Ok(VideoSettings::default());
        commit(&path, &mut first, |settings| settings.map_muted = true).unwrap();
        commit(&path, &mut second, |settings| {
            settings.map_diagnostics = true
        })
        .unwrap();
        let saved = load_from(&path).unwrap();
        assert!(saved.map_muted && saved.map_diagnostics);
        let second = second.unwrap();
        assert!(second.map_diagnostics && !second.map_muted);
        let form = VideoSettings {
            use_game_settings: false,
            ui_scale: 110.0,
            ..VideoSettings::default()
        };
        update(&path, Some(&form), |saved| with_display(saved, &form)).unwrap();
        let saved = load_from(&path).unwrap();
        assert!(saved.map_muted && saved.map_diagnostics && saved.ui_scale == 110.0);
        let mut lost = Err("zoom failed".to_owned());
        commit(&path, &mut lost, |settings| settings.map_muted = false).unwrap();
        let lost = lost.unwrap();
        assert!(lost.map_diagnostics && !lost.map_muted && lost.ui_scale == 110.0);
        fs::write(&path, b"not json").unwrap();
        assert!(commit(&path, &mut Err("unreadable".into()), |_| ()).is_err());
        commit(&path, &mut Ok(game_like(90.0)), |settings| {
            settings.map_muted = true
        })
        .unwrap();
        let saved = load_from(&path).unwrap();
        assert!(saved.map_muted && saved.ui_scale == 90.0);
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
            dashboard::protocol::MapAction::UiScale { scale: Some(1.25) }
        ));
        assert!(matches!(
            VideoSettings::default().map_ui_scale_action(),
            dashboard::protocol::MapAction::UiScale { scale: None }
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
