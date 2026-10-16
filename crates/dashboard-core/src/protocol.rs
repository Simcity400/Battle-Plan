//! Small control messages between the desktop host and its native viewer.
//!
//! Map pixels and replay tables never travel through this pipe; data replies
//! use the fragmented channel in [`crate::transport`]. Handles and branch
//! identifiers use decimal strings so every bit survives JavaScript.

use crate::transport::{DataCommand, DataEvent};
use serde::{Deserialize, Serialize};
use std::{io, path::Path, time::UNIX_EPOCH};

pub const EVENT_PREFIX: &str = "BATTLE_PLAN_DESKTOP ";
/// Requests share the reply transport's byte budget, including their envelope.
pub const MAX_MESSAGE_BYTES: usize = crate::transport::MAX_REPLY_BYTES;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeCommand {
    MapAction {
        action: MapAction,
    },
    /// Show the Campaign map in the window instead of the dashboard's maps.
    CampaignMap,
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
    /// The controller the user last touched, for the map whose window is in front. The host reads
    /// controllers because only the process that owns the foreground window receives their input;
    /// mouse and keyboard reach the map's own window directly. The host alone decides which
    /// viewer holds the controller ([`GamepadMessage`]).
    Gamepad {
        message: GamepadMessage,
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
    /// The full map's camera mode changed, by [`MapAction::CameraMode`] or the map's own
    /// shortcut; the host's controls follow it.
    CameraMode {
        mode: CameraMode,
    },
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
    /// Retire only the Campaign map's cached recording with this canonical path.
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
        #[serde(rename = "viewId")]
        view_id: Option<String>,
    },
    Inspect,
    /// The host's saved content folders; the viewer applies them without saving.
    Folders {
        game: String,
        user: String,
    },
    /// The host saved map preferences; the viewer reads them again and reloads
    /// any map whose artwork, quality or anti-aliasing they change.
    MapSettingsSaved,
    /// How the full map's camera moves: the game's map camera, flying or walking.
    CameraMode {
        mode: CameraMode,
    },
}

/// How the full map's camera moves.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub enum CameraMode {
    /// The game's map camera; a controller also orbits and tilts it.
    #[default]
    Map,
    /// A free eye flying over the map.
    Fly,
    /// A free eye on the ground.
    Walk,
}

impl CameraMode {
    /// The mode the map's own shortcut moves to: map, fly, walk, then map again.
    pub fn next(self) -> Self {
        match self {
            Self::Map => Self::Fly,
            Self::Fly => Self::Walk,
            Self::Walk => Self::Map,
        }
    }
}

/// One handoff of the controller to one viewer: the host's number for that viewer (each viewer it
/// starts is a new one) and the handoff's own number, which only grows.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GamepadOwner {
    pub viewer: u64,
    pub handoff: u64,
}

/// The host's controller messages to one viewer, in the order it sends them.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum GamepadMessage {
    /// The viewer holds the controller under `owner` from now on. `state` is what is held: a
    /// button already down is held, not pressed, so a handoff never presses anything.
    Take {
        owner: GamepadOwner,
        state: GamepadState,
    },
    /// The controller held under `owner` changed. A viewer holding it under another handoff, or
    /// not at all, drops it.
    State {
        owner: GamepadOwner,
        state: GamepadState,
    },
    /// The viewer no longer holds the controller: it lets go of everything without pressing or
    /// releasing anything, and drops later states. The host never releases a viewer after
    /// handing it the controller again, so a release always applies.
    Release,
}

impl GamepadMessage {
    /// Refuses a state outside its ranges.
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Take { state, .. } | Self::State { state, .. } => state.validate(),
            Self::Release => Ok(()),
        }
    }

    /// The one message to keep when `next` follows `queued` before the receiver reads either. A
    /// take or a release replaces whatever waits; a state refreshes a waiting take of its owner,
    /// replaces a waiting state and never follows a waiting release.
    pub fn merge(queued: Option<Self>, next: Self) -> Self {
        match (queued, next) {
            (Some(Self::Take { owner, .. }), Self::State { owner: of, state }) if owner == of => {
                Self::Take { owner, state }
            }
            (Some(waiting @ (Self::Take { .. } | Self::Release)), Self::State { .. }) => waiting,
            (_, next) => next,
        }
    }
}

/// One controller's state, before dead zones: sticks with `x` right and `y` up within -1..=1,
/// triggers within 0..=1 and the held buttons as [`GamepadButton`] bits.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GamepadState {
    /// Left then right stick.
    pub sticks: [[f32; 2]; 2],
    /// Left then right trigger.
    pub triggers: [f32; 2],
    pub buttons: u32,
}

impl GamepadState {
    /// Whether `button` is held.
    pub fn held(&self, button: GamepadButton) -> bool {
        self.buttons & button.bit() != 0
    }

    /// Refuses values outside their ranges or not numbers.
    pub fn validate(&self) -> Result<(), &'static str> {
        let axes = self.sticks.iter().flatten();
        if !axes.clone().all(|value| (-1.0..=1.0).contains(value))
            || !self
                .triggers
                .iter()
                .all(|value| (0.0..=1.0).contains(value))
            || self.buttons >> GamepadButton::ALL.len() != 0
        {
            return Err("Controller state is outside its ranges");
        }
        Ok(())
    }
}

/// A controller button, named by its position on a standard layout: the face buttons are
/// south (Xbox A, PlayStation cross), east (B, circle), west (X, square) and north (Y, triangle).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GamepadButton {
    South,
    East,
    West,
    North,
    LeftBumper,
    RightBumper,
    /// View, Back or Share.
    Select,
    /// Menu, Start or Options.
    Start,
    LeftStick,
    RightStick,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

impl GamepadButton {
    pub const ALL: [Self; 14] = [
        Self::South,
        Self::East,
        Self::West,
        Self::North,
        Self::LeftBumper,
        Self::RightBumper,
        Self::Select,
        Self::Start,
        Self::LeftStick,
        Self::RightStick,
        Self::DpadUp,
        Self::DpadDown,
        Self::DpadLeft,
        Self::DpadRight,
    ];

    pub fn bit(self) -> u32 {
        1 << self as u32
    }
}

impl MapAction {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Unload { path } => validate_local_path(path)?,
            Self::Configure { options } => {
                // The same check as every product, so any tag a widget can
                // pick is one the map accepts.
                if let Some(tag) = &options.country {
                    validate_tag(tag)?;
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
            Self::Inspect
            | Self::MapSettingsSaved
            | Self::Vsync { .. }
            | Self::Diagnostics { .. }
            | Self::CameraMode { .. } => {}
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

/// The one check for a requested country tag, shared by products and the map.
/// Tags are lookup keys compared with recorded tags, never paths. Definition
/// validity belongs to the content loader; requests refuse only emptiness and NUL.
pub fn validate_tag(tag: &str) -> Result<(), &'static str> {
    if tag.is_empty() || tag.contains('\0') {
        return Err("Invalid country tag");
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MapOptions {
    pub country: Option<String>,
    pub fog: bool,
}

pub use crate::map_display::{
    AirMission, CampaignMapSettings, CounterFilter, MapAudio, MapAudioOverrides, MapGraphics,
    MapGraphicsOverrides, MapTextKind, PlanKind, ResolvedMapSettings,
};

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
pub const MAX_EXACT_JS_INTEGER: u64 = 9_007_199_254_740_991;

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
        let mut pinned = input;
        pinned.as_object_mut().unwrap().remove("seekSequence");
        let mut view: super::DashboardMapView = serde_json::from_value(pinned).unwrap();
        assert_eq!(view.seek_sequence, None);
        view.validate().unwrap();
        view.seek_sequence = Some(9_007_199_254_740_992);
        assert!(view.validate().is_err());
    }

    #[test]
    fn widget_exports_keep_their_target_and_reject_invalid_ids() {
        for view_id in [None, Some("campaign-map".into())] {
            assert!(
                MapAction::Export {
                    path: "C:/map.png".into(),
                    view_id
                }
                .validate()
                .is_ok()
            );
        }
        assert!(
            MapAction::Export {
                path: "C:/map.png".into(),
                view_id: Some(String::new())
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn visual_mod_selections_refuse_only_empty_and_repeated_ids() {
        let choice = |id: &str| installed_content_core::content::visual::Choice {
            id: id.into(),
            name: "Art".into(),
        };
        let many = (0..5_000).map(|index| choice(&format!("workshop/{index}")));
        for selected in [
            vec![choice("workshop/1")],
            vec![choice("mod/ugc_1.mod"), choice("workshop/1")],
            many.collect(),
        ] {
            let valid = super::CampaignMapSettings {
                visual_mods: Some(selected),
                ..Default::default()
            };
            assert!(valid.validate().is_ok());
        }
        for selected in [
            vec![choice("")],
            vec![choice("workshop/1"), choice("workshop/1")],
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
    fn waiting_controller_messages_keep_their_handoffs() {
        let owner = |handoff| GamepadOwner { viewer: 1, handoff };
        let state = |x| GamepadState {
            sticks: [[x, 0.0], [0.0; 2]],
            ..GamepadState::default()
        };
        let take = |handoff, x| GamepadMessage::Take {
            owner: owner(handoff),
            state: state(x),
        };
        let update = |handoff, x| GamepadMessage::State {
            owner: owner(handoff),
            state: state(x),
        };
        // A newer state of a waiting take stays a take, so the viewer still takes it.
        assert_eq!(GamepadMessage::merge(Some(take(3, 0.1)), update(3, 0.5)), take(3, 0.5));
        assert_eq!(GamepadMessage::merge(Some(update(3, 0.1)), update(3, 0.5)), update(3, 0.5));
        // A stale state never replaces a release or another handoff's take.
        assert_eq!(GamepadMessage::merge(Some(GamepadMessage::Release), update(3, 0.5)), GamepadMessage::Release);
        assert_eq!(GamepadMessage::merge(Some(take(4, 0.1)), update(3, 0.5)), take(4, 0.1));
        // A release or a take replaces whatever waits.
        assert_eq!(GamepadMessage::merge(Some(take(4, 0.1)), GamepadMessage::Release), GamepadMessage::Release);
        assert_eq!(GamepadMessage::merge(Some(GamepadMessage::Release), take(5, 0.2)), take(5, 0.2));
        assert_eq!(GamepadMessage::merge(None, update(5, 0.2)), update(5, 0.2));
        let line = serde_json::to_string(&NativeCommand::Gamepad {
            message: take(2, 0.0),
        })
        .unwrap();
        assert!(line.starts_with(r#"{"command":"gamepad","message":{"kind":"take","owner":{"viewer":1,"handoff":2}"#), "{line}");
        assert!(GamepadMessage::State { owner: owner(1), state: GamepadState { triggers: [2.0, 0.0], ..GamepadState::default() } }.validate().is_err());
    }

    #[test]
    fn controller_states_hold_their_ranges_and_camera_modes_cycle() {
        let state = GamepadState {
            sticks: [[1.0, -1.0], [0.0; 2]],
            triggers: [0.0, 1.0],
            buttons: GamepadButton::Start.bit() | GamepadButton::DpadRight.bit(),
        };
        assert!(state.validate().is_ok());
        assert!(state.held(GamepadButton::Start) && !state.held(GamepadButton::Select));
        for bad in [
            GamepadState {
                sticks: [[1.01, 0.0], [0.0; 2]],
                ..GamepadState::default()
            },
            GamepadState {
                triggers: [-0.1, 0.0],
                ..GamepadState::default()
            },
            GamepadState {
                sticks: [[f32::NAN, 0.0], [0.0; 2]],
                ..GamepadState::default()
            },
            GamepadState {
                buttons: 1 << GamepadButton::ALL.len(),
                ..GamepadState::default()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
        let mut mode = CameraMode::default();
        let cycle: Vec<_> = (0..3)
            .map(|_| {
                mode = mode.next();
                mode
            })
            .collect();
        assert_eq!(cycle, [CameraMode::Fly, CameraMode::Walk, CameraMode::Map]);
        assert_eq!(
            serde_json::to_string(&NativeEvent::CameraMode {
                mode: CameraMode::Walk
            })
            .unwrap(),
            r#"{"event":"camera_mode","mode":"walk"}"#
        );
    }

    #[test]
    fn map_controls_validate_country_and_paths_without_renderer_state() {
        let configure = |tag: &str| MapAction::Configure {
            options: MapOptions {
                country: Some(tag.into()),
                ..Default::default()
            },
        };
        // Every tag a product accepts is a tag the map accepts, with no
        // vanilla character set: modded and lower-case tags included.
        for tag in ["Z01", "mod", "A_1", "Ä01", "LONG_MOD_TAG", &"A".repeat(129)] {
            assert!(configure(tag).validate().is_ok(), "{tag}");
        }
        for tag in ["", "A\0B"] {
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
