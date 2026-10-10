//! Small control messages between the desktop host and its native viewer.
//!
//! Map pixels and replay tables never travel through this pipe; data replies
//! use the fragmented channel in [`crate::transport`]. Handles and branch
//! identifiers use decimal strings so every bit survives JavaScript.

use crate::transport::{DataCommand, DataEvent};
use serde::{Deserialize, Serialize};
use std::{io, path::Path, time::UNIX_EPOCH};
use ts_rs::TS;

pub const EVENT_PREFIX: &str = "BATTLE_PLAN_DESKTOP ";
/// Requests share the reply transport's byte budget, including their envelope.
pub const MAX_MESSAGE_BYTES: usize = crate::transport::MAX_REPLY_BYTES;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeCommand {
    MapAction {
        action: MapAction,
    },
    Standalone,
    Visibility {
        visible: bool,
    },
    Bounds {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    Open {
        path: String,
        revision: String,
    },
    Seek {
        path: String,
        revision: String,
        branch: String,
        hour: i32,
        #[serde(default)]
        playback: Option<MapPlayback>,
    },
    /// Replaces the complete dashboard map layout and its absolute targets.
    MapViews {
        views: Vec<DashboardMapView>,
    },
    Data {
        request: DataCommand,
    },
    Close,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeEvent {
    MapDetails {
        details: MapDetails,
    },
    Ready {
        hwnd: String,
    },
    State {
        path: Option<String>,
        branch: Option<String>,
        hour: Option<i32>,
        revision: Option<String>,
        error: Option<String>,
    },
    Error {
        message: String,
    },
    MapStates {
        states: Vec<DashboardMapState>,
    },
    Data {
        response: DataEvent,
    },
    /// The user pressed the diagnostics shortcut in the native map. The host
    /// owns the saved choice, so it toggles it and sends the result back as
    /// [`MapAction::Diagnostics`].
    ToggleDiagnostics,
}

/// The manual map interface scales the viewer accepts. `RangeInclusive::contains`
/// is false for NaN, so it also refuses a non-number.
pub const MAP_UI_SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.5..=3.0;
/// The manual map interface scales the settings offer, in percent.
pub const MAP_UI_SCALE_CHOICES: [u32; 11] = [50, 75, 90, 100, 110, 125, 150, 175, 200, 250, 300];

/// Small, explicit map controls. No raw tables or image pixels cross this channel.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum MapAction {
    /// Retire only the cached standalone recording with this canonical path.
    Unload {
        path: String,
    },
    Vsync {
        enabled: bool,
    },
    /// The screen-space map interface's scale; `None` matches the game on screen.
    UiScale {
        scale: Option<f32>,
    },
    /// Shows or hides technical stats and data limitations over every map.
    Diagnostics {
        enabled: bool,
    },
    Configure {
        options: MapOptions,
    },
    Export {
        path: String,
        #[serde(default, rename = "viewId")]
        view_id: Option<String>,
    },
    Inspect,
    Folders {
        game: String,
        user: String,
    },
}

impl MapAction {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Unload { path } => validate_local_path(path)?,
            Self::Configure { options } => {
                // The same check as every product, so any tag a widget can
                // pick is one the map accepts.
                if let Some(tag) = &options.country {
                    crate::products::validate_tag(tag)?;
                }
            }
            Self::Export { path, view_id } => {
                validate_local_path(path)?;
                if view_id
                    .as_ref()
                    .is_some_and(|id| id.is_empty() || id.len() > 128 || id.contains('\0'))
                {
                    return Err("Invalid map widget id");
                }
            }
            Self::Folders { game, user } => {
                validate_local_path(game)?;
                validate_local_path(user)?;
            }
            Self::UiScale { scale } => {
                if scale.is_some_and(|scale| !MAP_UI_SCALE_RANGE.contains(&scale)) {
                    return Err("Invalid map UI scale");
                }
            }
            Self::Inspect | Self::Vsync { .. } | Self::Diagnostics { .. } => {}
        }
        Ok(())
    }
}

fn validate_local_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() || path.len() > 16_000 || path.contains('\0') {
        return Err("Invalid local path");
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MapOptions {
    pub country: Option<String>,
    pub fog: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MapGraphics {
    /// Viewer-local allied battle plans; no recorded client selection or toggle.
    #[serde(default)]
    pub show_allied_plans: bool,
    /// Viewer-local world target size; CMapRenderingOptions+0x19 defaults true.
    #[serde(default = "default_full_resolution")]
    pub full_resolution: bool,
    /// Viewer-local day/night display; never copied from the recording's UI state.
    #[serde(default = "default_day_night")]
    pub day_night: bool,
    /// Embedded viewer-local NATO icons; missing legacy choices inherit their parent.
    #[serde(default)]
    pub nato_counters: Option<bool>,
    /// CMapRenderingOptions+0x18; ctor1401F9640 defaults true.
    #[serde(default = "default_high_gfx_shaders")]
    pub high_gfx_shaders: bool,
    /// CMapRenderingOptions+0x15; ctor1401F9640 defaults true.
    #[serde(default = "default_water_reflections")]
    pub water_reflections: bool,
    /// CMapRenderingOptions+0x0e; ctor1401F9696 defaults true.
    #[serde(default = "default_sky")]
    pub sky: bool,
    #[serde(default)]
    pub texture_quality: u8,
    #[serde(default = "default_samples")]
    pub samples: u32,
    pub trees: bool,
    pub shadows: bool,
    pub rivers: bool,
    pub buildings: bool,
    pub units: bool,
    pub city_sprawl: bool,
    pub weather: bool,
}

impl Default for MapGraphics {
    fn default() -> Self {
        Self {
            show_allied_plans: false,
            full_resolution: true,
            day_night: true,
            nato_counters: None,
            high_gfx_shaders: true,
            water_reflections: true,
            sky: true,
            texture_quality: 0,
            samples: default_samples(),
            trees: true,
            shadows: true,
            rivers: true,
            buildings: true,
            units: true,
            city_sprawl: true,
            weather: true,
        }
    }
}

fn default_day_night() -> bool {
    true
}

fn default_full_resolution() -> bool {
    true
}

fn default_high_gfx_shaders() -> bool {
    true
}

fn default_water_reflections() -> bool {
    true
}

fn default_sky() -> bool {
    true
}

fn default_samples() -> u32 {
    4
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CampaignMapSettings {
    /// Inherit graphics from the parent scope (the game -> global -> campaign -> widget).
    /// The wire name is retained for existing saved preferences. Missing legacy markers are overrides.
    #[serde(default)]
    pub use_game_graphics: bool,
    pub graphics: MapGraphics,
    /// None inherits parent artwork; the root preserves the recording's original layers.
    pub visual_mods: Option<Vec<String>>,
}

impl Default for CampaignMapSettings {
    fn default() -> Self {
        Self {
            use_game_graphics: true,
            graphics: MapGraphics::default(),
            visual_mods: None,
        }
    }
}

impl CampaignMapSettings {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.graphics.texture_quality > 2 {
            return Err("Invalid texture quality");
        }
        if !matches!(self.graphics.samples, 1 | 2 | 4 | 8) {
            return Err("Invalid anti-aliasing sample count");
        }
        if let Some(mods) = &self.visual_mods
            && (mods.len() > 64
                || mods.iter().any(|id| {
                    id.len() > 512
                        || !id.starts_with("mod/")
                        || !id.ends_with(".mod")
                        || id[4..].contains(['/', '\\', '\0'])
                        || id.contains("..")
                })
                || mods.iter().collect::<std::collections::BTreeSet<_>>().len() != mods.len())
        {
            return Err("Invalid cosmetic mod selection");
        }
        Ok(())
    }
}

impl Default for MapOptions {
    fn default() -> Self {
        Self {
            country: None,
            fog: true,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MapDetails {
    pub content_ready: bool,
    pub error: Option<String>,
    pub game: String,
    pub user: String,
    pub path: Option<String>,
    pub branch: Option<String>,
    pub hour: Option<i32>,
    pub messages: Vec<String>,
}

/// The largest integer a JavaScript number holds exactly.
pub(crate) const MAX_EXACT_JS_INTEGER: u64 = 9_007_199_254_740_991;

/// A full-map clock update. Without this, `Seek` is an explicit seek.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MapPlayback {
    pub playing: bool,
    pub seek_sequence: u64,
}

impl MapPlayback {
    pub fn validate(self) -> Result<(), &'static str> {
        if self.seek_sequence > MAX_EXACT_JS_INTEGER {
            return Err("Map seek sequence exceeds the exact JavaScript integer range");
        }
        Ok(())
    }
}

/// A native map body in physical host-client pixels. `bounds` is the full
/// projection rectangle; `clip` is its visible portion after scrolling and
/// host overlays. Only the latter contributes to the native window region.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DashboardMapView {
    #[serde(default)]
    pub options: MapOptions,
    pub id: String,
    pub source_id: String,
    pub path: String,
    pub revision: String,
    pub branch_id: String,
    /// Playing: the map may show, and acknowledge, the newest hour built
    /// before `hour`. Paused targets and exports stay exact.
    #[serde(default)]
    pub playing: bool,
    pub hour: i32,
    pub fraction: f32,
    pub sequence: u64,
    /// Changes only on a seek. Omitted for pinned targets and older clients;
    /// their hour changes start a new animation history.
    #[serde(default)]
    pub seek_sequence: Option<u64>,
    pub bounds: ViewBounds,
    pub clip: ViewBounds,
    pub visible: bool,
}

impl DashboardMapView {
    pub fn validate(&self) -> Result<(), &'static str> {
        MapAction::Configure {
            options: self.options.clone(),
        }
        .validate()?;
        for text in [&self.id, &self.source_id, &self.revision] {
            if text.is_empty() || text.len() > 128 || text.contains('\0') {
                return Err("Map identity is missing or exceeds its control budget");
            }
        }
        if self.path.is_empty() || self.path.len() > 32_000 || self.path.contains('\0') {
            return Err("Map recording path is invalid");
        }
        if self.branch_id.is_empty()
            || !self.branch_id.bytes().all(|byte| byte.is_ascii_digit())
            || self.branch_id.parse::<u64>().is_err()
        {
            return Err("Map branch must be an unsigned decimal u64 string");
        }
        if !self.fraction.is_finite() || !(0.0..1.0).contains(&self.fraction) {
            return Err("Map hour fraction must be finite and in [0, 1)");
        }
        if self.sequence > MAX_EXACT_JS_INTEGER
            || self
                .seek_sequence
                .is_some_and(|sequence| sequence > MAX_EXACT_JS_INTEGER)
        {
            return Err("Map sequence exceeds the exact JavaScript integer range");
        }
        if self.bounds.x.abs_diff(0) > 32_768
            || self.bounds.y.abs_diff(0) > 32_768
            || self.bounds.width == 0
            || self.bounds.height == 0
            || self.bounds.width > 32_768
            || self.bounds.height > 32_768
        {
            return Err("Map body exceeds supported physical bounds");
        }
        if self.clip.x < 0
            || self.clip.y < 0
            || self.clip.x > 32_768
            || self.clip.y > 32_768
            || self.clip.width > 32_768
            || self.clip.height > 32_768
        {
            return Err("Map clip exceeds supported physical bounds");
        }
        Ok(())
    }
}

pub fn validate_map_views(views: &[DashboardMapView]) -> Result<(), &'static str> {
    if views.len() > 128 {
        return Err("Too many native map views");
    }
    let mut ids = std::collections::BTreeSet::new();
    for view in views {
        view.validate()?;
        if !ids.insert(&view.id) {
            return Err("Native map view ids must be unique");
        }
    }
    Ok(())
}

/// Acknowledges a rendered target, separately from the latest requested one.
/// An old image retains its actual branch/hour/sequence while another builds.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DashboardMapState {
    pub id: String,
    pub source_id: Option<String>,
    pub revision: Option<String>,
    pub branch_id: Option<String>,
    pub hour: Option<i32>,
    pub fraction: Option<f32>,
    pub sequence: u64,
    pub requested_sequence: u64,
    pub loading: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ViewerState {
    pub path: Option<String>,
    pub branch: Option<String>,
    pub hour: Option<i32>,
    pub revision: Option<String>,
}

/// A cheap file-change marker shared by both processes. This is length and
/// modification time, not a cryptographic content identity or asset witness.
pub fn replay_revision(path: impl AsRef<Path>) -> io::Result<String> {
    let metadata = path.as_ref().metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Recording is not a file",
        ));
    }
    let modified = metadata.modified()?;
    let stamp = match modified.duration_since(UNIX_EPOCH) {
        Ok(duration) => format!("{}:{}", duration.as_secs(), duration.subsec_nanos()),
        Err(error) => {
            let duration = error.duration();
            format!("-{}:{}", duration.as_secs(), duration.subsec_nanos())
        }
    };
    Ok(format!("{}:{stamp}", metadata.len()))
}

/// The frontend measures its content region in physical client pixels.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ViewBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl ViewBounds {
    pub fn validate(self) -> Result<Self, &'static str> {
        if self.x < 0 || self.y < 0 || self.width == 0 || self.height == 0 {
            return Err("The map area must have a positive size inside the window");
        }
        if self.x > 32_768 || self.y > 32_768 || self.width > 32_768 || self.height > 32_768 {
            return Err("The map area exceeds supported window bounds");
        }
        Ok(self)
    }

    pub fn command(self) -> NativeCommand {
        NativeCommand::Bounds {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn map_targets_preserve_seek_sequence_and_accept_pinned_targets_without_it() {
        let input = serde_json::json!({
            "id": "map", "sourceId": "source", "path": "replay.battleplan",
            "revision": "1", "branchId": "1", "playing": true,
            "hour": 20, "fraction": 0.0, "sequence": 8, "seekSequence": 2,
            "bounds": { "x": 0, "y": 0, "width": 640, "height": 360 },
            "clip": { "x": 0, "y": 0, "width": 640, "height": 360 },
            "visible": true
        });
        let view: super::DashboardMapView = serde_json::from_value(input.clone()).unwrap();
        view.validate().unwrap();
        assert_eq!(view.seek_sequence, Some(2));
        assert_eq!(serde_json::to_value(&view).unwrap()["seekSequence"], 2);
        let mut pinned = input;
        pinned.as_object_mut().unwrap().remove("seekSequence");
        let mut view: super::DashboardMapView = serde_json::from_value(pinned).unwrap();
        assert_eq!(view.seek_sequence, None);
        view.validate().unwrap();
        view.seek_sequence = Some(9_007_199_254_740_992);
        assert!(view.validate().is_err());
    }

    #[test]
    fn graphics_migrate_existing_preferences_and_validate_quality() {
        let old = r#"{"trees":true,"shadows":true,"rivers":true,"buildings":true,"units":true,"citySprawl":true,"weather":true}"#;
        let graphics: super::MapGraphics = serde_json::from_str(old).unwrap();
        assert_eq!(graphics.samples, 4);
        assert_eq!(graphics.texture_quality, 0);
        assert!(graphics.high_gfx_shaders);
        assert!(graphics.water_reflections && graphics.sky);
        assert!(graphics.day_night);
        assert_eq!(graphics.nato_counters, None);
        assert!(graphics.full_resolution);
        assert!(!graphics.show_allied_plans);
        let off = super::MapGraphics {
            show_allied_plans: true,
            full_resolution: false,
            day_night: false,
            nato_counters: Some(false),
            ..graphics
        };
        assert_eq!(
            serde_json::from_str::<super::MapGraphics>(&serde_json::to_string(&off).unwrap())
                .unwrap(),
            off
        );
        for nato_counters in [None, Some(false), Some(true)] {
            let configured = super::MapGraphics {
                nato_counters,
                ..graphics
            };
            assert_eq!(
                serde_json::from_str::<super::MapGraphics>(
                    &serde_json::to_string(&configured).unwrap()
                )
                .unwrap(),
                configured
            );
        }
        let old_settings = format!(r#"{{"graphics":{old},"visualMods":null}}"#);
        let old_settings: CampaignMapSettings = serde_json::from_str(&old_settings).unwrap();
        assert!(!old_settings.use_game_graphics);
        assert!(CampaignMapSettings::default().use_game_graphics);
        for samples in [0, 3, 16, u32::MAX] {
            let settings = super::CampaignMapSettings {
                graphics: super::MapGraphics {
                    samples,
                    ..graphics
                },
                visual_mods: None,
                use_game_graphics: false,
            };
            assert!(settings.validate().is_err());
        }
        assert!(
            super::CampaignMapSettings {
                graphics: super::MapGraphics {
                    texture_quality: 3,
                    ..graphics
                },
                visual_mods: None,
                use_game_graphics: false,
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn widget_exports_keep_their_target_and_reject_invalid_ids() {
        let action: MapAction = serde_json::from_str(
            r#"{"kind":"export","path":"C:/map.png","viewId":"campaign-map"}"#,
        )
        .unwrap();
        assert!(action.validate().is_ok());
        assert!(
            matches!(action, MapAction::Export { view_id: Some(id), .. } if id == "campaign-map")
        );
        assert!(
            MapAction::Export {
                path: "C:/map.png".into(),
                view_id: Some(String::new())
            }
            .validate()
            .is_err()
        );
        let legacy: MapAction =
            serde_json::from_str(r#"{"kind":"export","path":"C:/map.png"}"#).unwrap();
        assert!(matches!(legacy, MapAction::Export { view_id: None, .. }));
    }

    #[test]
    fn removed_map_controls_are_not_accepted() {
        assert!(serde_json::from_str::<super::MapAction>(r#"{"kind":"fit"}"#).is_err());
        assert!(
            serde_json::from_str::<super::MapOptions>(
                r#"{"country":null,"fog":true,"control":true}"#
            )
            .is_err()
        );
    }

    #[test]
    fn the_diagnostics_switch_crosses_the_pipe_both_ways() {
        let action: super::MapAction =
            serde_json::from_str(r#"{"kind":"diagnostics","enabled":true}"#).unwrap();
        assert!(matches!(
            action,
            super::MapAction::Diagnostics { enabled: true }
        ));
        assert!(action.validate().is_ok());
        let event = serde_json::to_string(&super::NativeEvent::ToggleDiagnostics).unwrap();
        assert_eq!(event, r#"{"event":"toggle_diagnostics"}"#);
        assert!(matches!(
            serde_json::from_str(&event).unwrap(),
            super::NativeEvent::ToggleDiagnostics
        ));
    }

    #[test]
    fn cosmetic_selections_reject_paths_duplicates_and_oversize_lists() {
        let valid = super::CampaignMapSettings {
            visual_mods: Some(vec!["mod/ugc_1.mod".into()]),
            ..Default::default()
        };
        assert!(valid.validate().is_ok());
        for selected in [
            vec!["mod/../other.mod".into()],
            vec!["mod/sub/other.mod".into()],
            vec!["C:/other.mod".into()],
            vec!["mod/a.mod".into(), "mod/a.mod".into()],
            vec!["mod/a.mod".into(); 65],
        ] {
            assert!(
                super::CampaignMapSettings {
                    visual_mods: Some(selected),
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
    }
    use super::*;

    #[test]
    fn map_controls_validate_country_and_paths_without_renderer_state() {
        let action = MapAction::Configure {
            options: MapOptions {
                country: Some("D01".into()),
                ..Default::default()
            },
        };
        assert!(action.validate().is_ok());
        let configure = |tag: &str| MapAction::Configure {
            options: MapOptions {
                country: Some(tag.into()),
                ..Default::default()
            },
        };
        // Every tag a product accepts is a tag the map accepts, with no
        // vanilla character set: modded and lower-case tags included.
        for tag in ["Z01", "mod", "A_1", "Ä01", "LONG_MOD_TAG", &"A".repeat(129)] {
            assert!(crate::products::validate_tag(tag).is_ok());
            assert!(configure(tag).validate().is_ok(), "{tag}");
        }
        for tag in ["", "A\0B"] {
            assert!(crate::products::validate_tag(tag).is_err());
            assert!(configure(tag).validate().is_err());
        }
        assert!(
            MapAction::Folders {
                game: String::new(),
                user: "user".into()
            }
            .validate()
            .is_err()
        );
        let wire = serde_json::to_string(&NativeCommand::MapAction { action }).unwrap();
        assert!(matches!(
            serde_json::from_str::<NativeCommand>(&wire).unwrap(),
            NativeCommand::MapAction { .. }
        ));
    }

    #[test]
    fn full_width_branch_survives_the_wire() {
        let command = NativeCommand::Seek {
            path: "campaign.battleplan".into(),
            revision: "snapshot".into(),
            branch: u64::MAX.to_string(),
            hour: 60_832_633,
            playback: Some(super::MapPlayback {
                playing: true,
                seek_sequence: 7,
            }),
        };
        let json = serde_json::to_string(&command).unwrap();
        assert!(json.contains("\"18446744073709551615\""));
        let NativeCommand::Seek {
            path,
            revision,
            branch,
            hour,
            playback,
        } = serde_json::from_str(&json).unwrap()
        else {
            panic!("wrong command");
        };
        assert_eq!(path, "campaign.battleplan");
        assert_eq!(revision, "snapshot");
        assert_eq!(branch.parse::<u64>().unwrap(), u64::MAX);
        assert_eq!(hour, 60_832_633);
        assert_eq!(
            playback,
            Some(super::MapPlayback {
                playing: true,
                seek_sequence: 7
            })
        );
    }

    #[test]
    fn invalid_geometry_is_rejected_before_native_calls() {
        for bounds in [
            ViewBounds {
                x: -1,
                y: 0,
                width: 100,
                height: 100,
            },
            ViewBounds {
                x: 80,
                y: 0,
                width: 0,
                height: 100,
            },
            ViewBounds {
                x: 80,
                y: 0,
                width: u32::MAX,
                height: 100,
            },
        ] {
            assert!(bounds.validate().is_err());
        }
        assert!(
            ViewBounds {
                x: 100,
                y: 0,
                width: 1500,
                height: 900
            }
            .validate()
            .is_ok()
        );
    }
}
