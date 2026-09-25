//! Application UI scale, the native map's interface scale and presentation settings. Never changes the user's monitor mode or installed content.
use crate::native::NativeHost;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, sync::Mutex};
use tauri::{Manager, State, WebviewWindow};

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
}
impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            use_game_settings: true,
            ui_scale: 100.0,
            vsync: true,
            map_ui_scale: None,
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
pub struct VideoState(pub Mutex<Result<VideoSettings, String>>);
impl Default for VideoState {
    fn default() -> Self {
        Self(Mutex::new(load()))
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
    let path = path()?;
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
fn save(settings: &VideoSettings) -> Result<(), String> {
    let path = path()?;
    fs::create_dir_all(path.parent().expect("settings directory")).map_err(|e| e.to_string())?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(
        &temp,
        serde_json::to_vec(settings).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(temp, path).map_err(|e| e.to_string())
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
fn game_user_folder(window: &WebviewWindow) -> Result<PathBuf, String> {
    let current = window
        .state::<NativeHost>()
        .status()
        .map_details
        .map(|details| PathBuf::from(details.user))
        .filter(|path| !path.as_os_str().is_empty());
    let user = match current {
        Some(user) => user,
        None => installed_content::settings::configured_user_folder().map_err(|e| e.to_string())?,
    };
    Ok(user)
}
fn game_settings(window: &WebviewWindow) -> Result<VideoSettings, String> {
    let user = game_user_folder(window)?;
    parse_game_display(&installed_content::settings::read(&user).map_err(|e| e.to_string())?)
}
/// The viewer computes the same from the recording's user folder and the same monitors.
fn game_map_ui_scale(window: &WebviewWindow) -> Option<f64> {
    let user = game_user_folder(window).ok()?;
    let display = installed_content::settings::read_display(&user).ok()?;
    let scale = display.screen_scale(&installed_content::settings::desktops().ok()?)?;
    dashboard::protocol::MAP_UI_SCALE_RANGE
        .contains(&scale)
        .then(|| (f64::from(scale) * 100.0).round())
}
fn parse_game_display(document: &installed_content::parser::Document) -> Result<VideoSettings, String> {
    use installed_content::settings::{block, scalar, toggle};
    let graphics = block(&document.items, "graphics").map_err(|e| e.to_string())?;
    let scale = scalar(graphics, "gui_scale")
        .map_err(|e| e.to_string())?
        .parse::<f64>()
        .map_err(|_| "Invalid game UI scale")?;
    let vsync = toggle(graphics, "vsync").map_err(|e| e.to_string())?;
    let settings = VideoSettings {
        use_game_settings: true,
        ui_scale: scale * 100.0,
        vsync,
        map_ui_scale: None,
    };
    settings.validate()?;
    Ok(settings)
}
fn reply(window: &WebviewWindow, settings: VideoSettings) -> VideoReply {
    let (game_settings, game_settings_error) = match game_settings(window) {
        Ok(settings) => (Some(settings), None),
        Err(error) => (None, Some(error)),
    };
    VideoReply {
        settings,
        game_settings,
        game_settings_error,
        game_map_ui_scale: game_map_ui_scale(window),
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
        game_settings(window)
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
            && let Ok(game) = game_settings(window)
        {
            *settings = game;
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
        && let Ok(game) = game_settings(&window)
    {
        if game.ui_scale != settings.ui_scale {
            apply_scale(&window, &game)?;
        }
        if host.status().ready {
            host.map_action(dashboard::protocol::MapAction::Vsync {
                enabled: game.vsync,
            })?;
        }
        settings = game;
        *stored = Ok(settings.clone());
    }
    Ok(reply(&window, settings))
}
#[tauri::command]
pub fn set_video_settings(
    window: WebviewWindow,
    state: State<'_, VideoState>,
    host: State<'_, NativeHost>,
    settings: VideoSettings,
) -> Result<VideoReply, String> {
    let settings = if settings.use_game_settings {
        game_settings(&window)?
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
    let result = (|| {
        if stored.is_err() || settings.ui_scale != previous.ui_scale {
            apply_scale(&window, &settings)?;
        }
        if host.status().ready {
            host.map_action(dashboard::protocol::MapAction::Vsync {
                enabled: settings.vsync,
            })?;
            host.map_action(settings.map_ui_scale_action())?;
        }
        save(&settings)
    })();
    if let Err(error) = result {
        let _ = apply_scale(&window, &previous);
        if host.status().ready {
            let _ = host.map_action(dashboard::protocol::MapAction::Vsync {
                enabled: previous.vsync,
            });
            let _ = host.map_action(previous.map_ui_scale_action());
        }
        return Err(error);
    }
    *stored = Ok(settings.clone());
    Ok(reply(&window, settings))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn game_display_defaults_keep_scale_and_vsync_without_resizing() {
        let doc = installed_content::parser::parse(b"graphics={size={x=2048 y=1152} gui_scale=1.125 fullScreen=no borderless=yes vsync=no}").unwrap();
        let settings = parse_game_display(&doc).unwrap();
        assert_eq!(settings.ui_scale, 112.5);
        assert!(settings.use_game_settings && !settings.vsync);
        assert_eq!(settings.map_ui_scale, None);
        let old: VideoSettings = serde_json::from_str(
            r#"{"width":1500,"height":950,"uiScale":100,"fullscreen":false,"vsync":true}"#,
        )
        .unwrap();
        assert!(!old.use_game_settings);
        assert_eq!(old.map_ui_scale, None);
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
