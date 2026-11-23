
use serde::{Deserialize, Serialize};
use std::{io, path::Path, time::UNIX_EPOCH};

pub const MAX_MESSAGE_BYTES: usize = crate::transport::MAX_REPLY_BYTES;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeCommand {
    MapAction {
        action: MapAction,
    },
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
        revision: Option<String>,
    },
    Seek {
        path: String,
        revision: String,
        branch: String,
        hour: i32,
        #[serde(default)]
        playback: Option<MapPlayback>,
    },
    MapViews {
        #[serde(default)]
        clock_id: Option<String>,
        views: Vec<DashboardMapView>,
    },
    Gamepad {
        message: GamepadMessage,
    },
    Close,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeEvent {
    MapDetails {
        details: MapDetails,
    },
    Ready {
        hwnd: String,
        sample_counts: Option<Vec<u32>>,
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
    ToggleDiagnostics,
    CameraMode {
        mode: CameraMode,
        #[serde(default)]
        overlays: bool,
    },
    FlyAircraft {
        aircraft: Option<String>,
        state: FlyAircraftState,
    },
    MapOptions {
        #[serde(rename = "viewId")]
        view_id: String,
        options: MapOptions,
    },
    MapModes {
        #[serde(rename = "viewId")]
        view_id: String,
        modes: Vec<MapModeChoice>,
    },
    MapToggle {
        #[serde(rename = "viewId")]
        view_id: String,
        toggle: MapToggle,
    },
    ObserveCountry {
        #[serde(rename = "viewId")]
        view_id: String,
        country: ObservedCountry,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservedCountry {
    pub slot: u32,
    pub tag: String,
    #[serde(default)]
    pub sequence: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlyAircraftStatus {
    pub aircraft: Option<String>,
    pub state: FlyAircraftState,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum FlyAircraftState {
    Loading,
    Ready,
    Refused {
        reason: String,
    },
}

pub const MAP_UI_SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.5..=3.0;
pub const MAP_UI_SCALE_CHOICES: [u32; 11] = [50, 75, 90, 100, 110, 125, 150, 175, 200, 250, 300];

pub const MAP_FRAME_LIMIT_RANGE: std::ops::RangeInclusive<f64> = 1.0..=1000.0;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum MapAction {
    Unload {
        path: String,
    },
    Vsync {
        enabled: bool,
    },
    FrameLimit {
        hz: Option<f64>,
    },
    UiScale {
        scale: Option<f32>,
    },
    Diagnostics {
        enabled: bool,
    },
    Mute {
        muted: bool,
    },
    Configure {
        options: MapOptions,
    },
    Inspect,
    Folders {
        game: String,
        user: String,
    },
    MapSettingsSaved,
    RefreshVisualMods,
    CameraMode {
        mode: CameraMode,
        #[serde(default)]
        overlays: bool,
    },
    FlyAircraft {
        aircraft: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub enum CameraMode {
    #[default]
    Map,
    Fly,
    Walk,
}

impl CameraMode {
    pub fn next(self) -> Self {
        match self {
            Self::Map => Self::Fly,
            Self::Fly => Self::Walk,
            Self::Walk => Self::Map,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GamepadOwner {
    pub viewer: u64,
    pub handoff: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum GamepadMessage {
    Take {
        owner: GamepadOwner,
        state: GamepadState,
    },
    State {
        owner: GamepadOwner,
        state: GamepadState,
        pressed: u32,
    },
    Release,
}

impl GamepadMessage {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Take { state, .. } => state.validate(),
            Self::State { state, pressed, .. } => {
                if *pressed >> GamepadButton::ALL.len() != 0 {
                    return Err("Controller state is outside its ranges");
                }
                state.validate()
            }
            Self::Release => Ok(()),
        }
    }

    pub fn merge(queued: Option<Self>, next: Self) -> Self {
        match (queued, next) {
            (
                Some(Self::Take { owner, .. }),
                Self::State {
                    owner: of, state, ..
                },
            ) if owner == of => Self::Take { owner, state },
            (
                Some(Self::State {
                    owner,
                    pressed: earlier,
                    ..
                }),
                Self::State {
                    owner: of,
                    state,
                    pressed,
                },
            ) if owner == of => Self::State {
                owner,
                state,
                pressed: earlier | pressed,
            },
            (Some(waiting @ (Self::Take { .. } | Self::Release)), Self::State { .. }) => waiting,
            (_, next) => next,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GamepadState {
    pub sticks: [[f32; 2]; 2],
    pub triggers: [f32; 2],
    pub buttons: u32,
}

impl GamepadState {
    pub fn held(&self, button: GamepadButton) -> bool {
        self.buttons & button.bit() != 0
    }

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GamepadButton {
    South,
    East,
    West,
    North,
    LeftBumper,
    RightBumper,
    Select,
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
                if let Some(tag) = &options.country {
                    validate_tag(tag)?;
                }
                options.validate_modes()?;
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
            Self::FlyAircraft { aircraft } => {
                if aircraft
                    .as_ref()
                    .is_some_and(|name| name.is_empty() || name.contains('\0'))
                {
                    return Err("Invalid aircraft name");
                }
            }
            Self::FrameLimit { hz } => {
                if hz.is_some_and(|hz| !MAP_FRAME_LIMIT_RANGE.contains(&hz)) {
                    return Err("Invalid map frame rate limit");
                }
            }
            Self::Inspect
            | Self::MapSettingsSaved
            | Self::RefreshVisualMods
            | Self::Vsync { .. }
            | Self::Diagnostics { .. }
            | Self::CameraMode { .. }
            | Self::Mute { .. } => {}
        }
        Ok(())
    }
}

pub fn validate_export(path: &str, view_id: Option<&str>) -> Result<(), &'static str> {
    validate_local_path(path)?;
    if view_id.is_some_and(|id| id.is_empty() || id.len() > 128 || id.contains('\0')) {
        return Err("Invalid map widget id");
    }
    Ok(())
}

fn validate_local_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() || path.len() > 16_000 || path.contains('\0') {
        return Err("Invalid local path");
    }
    Ok(())
}

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
    #[serde(default)]
    pub country_slot: Option<u32>,
    #[serde(default)]
    pub country_pinned: bool,
    pub fog: bool,
    #[serde(default)]
    pub map_mode: Option<String>,
    #[serde(default)]
    pub map_mode_slots: Vec<String>,
    #[serde(default)]
    pub picker_hidden: bool,
    #[serde(default)]
    pub observed: Option<u64>,
}

pub const CAMPAIGN_MAP_VIEW: &str = "campaign-map";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MapModeChoice {
    pub token: String,
    pub label: String,
    pub unavailable: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MapToggle {
    pub setting: MapSetting,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum MapSetting {
    DayNight,
    AlliedPlans,
    Fog,
}

impl MapOptions {
    pub fn validate_modes(&self) -> Result<(), &'static str> {
        let token = |token: &String| token.is_empty() || token.chars().any(char::is_control);
        if self.map_mode.as_ref().is_some_and(token) || self.map_mode_slots.iter().any(token) {
            return Err("Invalid map mode");
        }
        Ok(())
    }
}

pub use crate::map_display::{
    AirMission, CampaignMapSettings, CounterFilter, MapAudio, MapAudioOverrides, MapGraphics,
    MapGraphicsOverrides, MapTextKind, PlanKind, ResolvedMapSettings,
};

impl Default for MapOptions {
    fn default() -> Self {
        Self {
            country: None,
            country_slot: None,
            country_pinned: false,
            fog: crate::map_display::option_defaults::FOG,
            map_mode: None,
            map_mode_slots: Vec::new(),
            picker_hidden: false,
            observed: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default, Deserialize, Serialize)]
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

pub const MAX_EXACT_JS_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MapPlayback {
    pub playing: bool,
    pub seek_sequence: u64,
    pub fraction: f64,
    pub hours_per_second: f64,
    pub composed_at: f64,
    pub clock_id: String,
}

pub fn validate_clock_id(clock_id: &str) -> Result<(), &'static str> {
    if clock_id.is_empty() || clock_id.len() > 128 || clock_id.contains('\0') {
        return Err("Map clock identity is missing or exceeds its control budget");
    }
    Ok(())
}

impl MapPlayback {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.fraction.is_finite() || !(0.0..1.0).contains(&self.fraction) {
            return Err("Map hour fraction must be finite and in [0, 1)");
        }
        if !self.composed_at.is_finite() || self.composed_at < 0.0 {
            return Err("Map anchor compose time must be finite and not negative");
        }
        validate_clock_id(&self.clock_id)?;
        if !self.hours_per_second.is_finite() || self.hours_per_second <= 0.0 {
            return Err("Map playback speed must be finite and positive");
        }
        if self.seek_sequence > MAX_EXACT_JS_INTEGER {
            return Err("Map seek sequence exceeds the exact JavaScript integer range");
        }
        Ok(())
    }
}

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
    #[serde(default)]
    pub playing: bool,
    pub hours_per_second: f64,
    pub hour: i32,
    pub fraction: f32,
    pub sequence: u64,
    #[serde(default)]
    pub seek_sequence: Option<u64>,
    pub composed_at: f64,
    pub clock_id: String,
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
        validate_clock_id(&self.clock_id)?;
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
        if !self.hours_per_second.is_finite() || self.hours_per_second <= 0.0 {
            return Err("Map playback speed must be finite and positive");
        }
        if !self.composed_at.is_finite() || self.composed_at < 0.0 {
            return Err("Map anchor compose time must be finite and not negative");
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

pub fn validate_map_views(clock_id: &str, views: &[DashboardMapView]) -> Result<(), &'static str> {
    validate_clock_id(clock_id)?;
    if views.len() > 128 {
        return Err("Too many native map views");
    }
    let mut ids = std::collections::BTreeSet::new();
    for view in views {
        view.validate()?;
        if view.clock_id != clock_id {
            return Err("Map view clock identity does not match snapshot clock identity");
        }
        if !ids.insert(&view.id) {
            return Err("Native map view ids must be unique");
        }
    }
    Ok(())
}

pub fn validate_map_holes(holes: &[ViewBounds]) -> Result<(), &'static str> {
    if holes.len() > 128 {
        return Err("Too many native map holes");
    }
    holes.iter().try_for_each(|hole| hole.validate().map(drop))
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DashboardMapState {
    pub id: String,
    #[serde(default)]
    pub clock_id: String,
    pub source_id: Option<String>,
    pub revision: Option<String>,
    pub branch_id: Option<String>,
    pub hour: Option<i32>,
    pub fraction: Option<f32>,
    pub sequence: u64,
    pub requested_sequence: u64,
    pub loading: bool,
    pub behind: Option<u32>,
    pub behind_reason: Option<String>,
    pub error: Option<String>,
    pub map_mode: Option<String>,
    pub drawn_at: Option<f64>,
    pub hover: Option<String>,
    pub tooltip: Option<String>,
    pub tooltip_delay_ms: Option<f32>,
    pub zoom: Option<f32>,
    pub center: Option<[f32; 2]>,
    pub options: Option<MapOptions>,
    pub applied: Option<AppliedMapSettings>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppliedMapSettings {
    pub graphics: MapGraphics,
    pub audio: MapAudio,
    pub counters: CounterFilter,
    pub visual_mods: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ViewerState {
    pub path: Option<String>,
    pub branch: Option<String>,
    pub hour: Option<i32>,
    pub revision: Option<String>,
}

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
    fn full_map_clock_roundtrips_fraction_and_exact_rate_and_validates_them() {
        let clock = super::MapPlayback {
            playing: true,
            seek_sequence: 7,
            fraction: 0.375,
            hours_per_second: 1440.25,
            composed_at: 12_345.5,
            clock_id: "page-origin-1".into(),
        };
        let json = serde_json::to_value(&clock).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "playing": true, "seekSequence": 7, "fraction": 0.375, "hoursPerSecond": 1440.25,
                "composedAt": 12_345.5, "clockId": "page-origin-1",
            })
        );
        let decoded: super::MapPlayback = serde_json::from_value(json).unwrap();
        assert_eq!(clock, decoded);
        assert!(decoded.validate().is_ok());
        for fraction in [-0.1, 1.0, f64::NAN, f64::INFINITY] {
            assert!(
                super::MapPlayback {
                    fraction,
                    ..clock.clone()
                }
                .validate()
                .is_err()
            );
        }
        for hours_per_second in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(
                super::MapPlayback {
                    hours_per_second,
                    ..clock.clone()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            super::MapPlayback {
                seek_sequence: super::MAX_EXACT_JS_INTEGER + 1,
                ..clock.clone()
            }
            .validate()
            .is_err()
        );
        for composed_at in [-0.001, f64::NAN, f64::INFINITY] {
            assert!(
                super::MapPlayback {
                    composed_at,
                    ..clock.clone()
                }
                .validate()
                .is_err()
            );
        }
        for clock_id in [String::new(), "bad\0id".into(), "x".repeat(129)] {
            assert!(
                super::MapPlayback {
                    clock_id,
                    ..clock.clone()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            super::MapPlayback {
                composed_at: 0.0,
                ..clock
            }
            .validate()
            .is_ok(),
            "a page that composed its first anchor at its clock's origin"
        );
    }

    #[test]
    fn map_targets_preserve_seek_sequence_and_accept_pinned_targets_without_it() {
        let input = serde_json::json!({
            "id": "map", "sourceId": "source", "path": "replay.battleplan",
            "revision": "1", "branchId": "1", "playing": true, "hoursPerSecond": 0.25,
            "hour": 20, "fraction": 0.0, "sequence": 8, "seekSequence": 2, "composedAt": 9.5,
            "clockId": "page-origin-1",
            "bounds": { "x": 0, "y": 0, "width": 640, "height": 360 },
            "clip": { "x": 0, "y": 0, "width": 640, "height": 360 },
            "visible": true
        });
        let view: super::DashboardMapView = serde_json::from_value(input.clone()).unwrap();
        view.validate().unwrap();
        super::validate_map_views("page-origin-1", &[view.clone()]).unwrap();
        super::validate_map_views("page-origin-1", &[]).unwrap();
        assert!(super::validate_map_views("", &[]).is_err());
        assert!(super::validate_map_views("page-origin-2", &[view.clone()]).is_err());
        assert_eq!(view.seek_sequence, Some(2));
        assert_eq!(view.hours_per_second, 0.25);
        assert_eq!(view.clock_id, "page-origin-1");
        for speed in [0.0, -1.0, f64::INFINITY, f64::NAN] {
            let mut invalid = view.clone();
            invalid.hours_per_second = speed;
            assert!(invalid.validate().is_err());
        }
        for clock_id in [String::new(), "bad\0id".into(), "x".repeat(129)] {
            let mut invalid = view.clone();
            invalid.clock_id = clock_id;
            assert!(invalid.validate().is_err());
        }
        let mut pinned = input;
        pinned.as_object_mut().unwrap().remove("seekSequence");
        let mut view: super::DashboardMapView = serde_json::from_value(pinned).unwrap();
        assert_eq!(view.seek_sequence, None);
        view.validate().unwrap();
        view.seek_sequence = Some(9_007_199_254_740_992);
        assert!(view.validate().is_err());
    }

    #[test]
    fn map_modes_travel_in_the_options_and_bad_tokens_are_refused() {
        let options: super::MapOptions = serde_json::from_str(
            r#"{"country":null,"fog":true,"mapMode":"states","mapModeSlots":["terrain"]}"#,
        )
        .unwrap();
        assert_eq!(options.map_mode.as_deref(), Some("states"));
        assert!(
            super::MapAction::Configure {
                options: options.clone()
            }
            .validate()
            .is_ok()
        );
        let plain: super::MapOptions =
            serde_json::from_str(r#"{"country":null,"fog":false}"#).unwrap();
        assert_eq!(plain.map_mode, None);
        assert!(plain.map_mode_slots.is_empty());
        for bad in [
            super::MapOptions {
                map_mode: Some(String::new()),
                ..options.clone()
            },
            super::MapOptions {
                map_mode: Some(
                    "a
b"
                    .into(),
                ),
                ..options.clone()
            },
            super::MapOptions {
                map_mode_slots: vec!["terrain".into(), String::new()],
                ..options.clone()
            },
        ] {
            assert!(
                super::MapAction::Configure { options: bad }
                    .validate()
                    .is_err()
            );
        }
        let long = super::MapOptions {
            map_mode: Some(format!("scripted:{}", "m".repeat(249))),
            map_mode_slots: vec!["terrain".into(); 300],
            ..options.clone()
        };
        assert!(
            super::MapAction::Configure { options: long }
                .validate()
                .is_ok()
        );
        let event = super::NativeEvent::MapToggle {
            view_id: "m".into(),
            toggle: super::MapToggle {
                setting: super::MapSetting::DayNight,
            },
        };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"event":"map_toggle","viewId":"m","toggle":{"setting":"dayNight"}}"#
        );
        let observe = super::NativeEvent::ObserveCountry {
            view_id: "campaign-map".into(),
            country: super::ObservedCountry {
                slot: 1499,
                tag: "D01".into(),
                sequence: 7,
            },
        };
        assert_eq!(
            serde_json::to_string(&observe).unwrap(),
            r#"{"event":"observe_country","viewId":"campaign-map","country":{"slot":1499,"tag":"D01","sequence":7}}"#
        );
        let back: super::MapOptions =
            serde_json::from_str(r#"{"country":"D01","countrySlot":1499,"fog":true,"observed":7}"#).unwrap();
        assert_eq!((back.country_slot, back.country_pinned, back.observed), (Some(1499), false, Some(7)));
        let older: super::MapOptions =
            serde_json::from_str(r#"{"country":"D01","countrySlot":1499,"fog":true}"#).unwrap();
        assert_eq!(older.observed, None);
        let plain: super::MapOptions = serde_json::from_str(r#"{"country":null,"fog":true}"#).unwrap();
        assert_eq!(plain.country_slot, None);
        let modes = super::NativeEvent::MapModes {
            view_id: "m".into(),
            modes: vec![super::MapModeChoice {
                token: "terrain".into(),
                label: "Terrain".into(),
                unavailable: None,
            }],
        };
        assert_eq!(
            serde_json::to_string(&modes).unwrap(),
            r#"{"event":"map_modes","viewId":"m","modes":[{"token":"terrain","label":"Terrain","unavailable":null}]}"#
        );
    }

    #[test]
    fn a_map_state_names_the_mode_and_time_of_the_frame_drawn() {
        let state = super::DashboardMapState {
            id: "m".into(),
            clock_id: "page-origin-1".into(),
            hour: Some(7),
            map_mode: Some("terrain".into()),
            drawn_at: Some(1.5),
            ..Default::default()
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["clockId"], "page-origin-1");
        assert_eq!(json["mapMode"], "terrain");
        assert_eq!(json["drawnAt"], 1.5);
    }

    #[test]
    fn a_map_state_carries_the_settings_its_frame_drew_with_across_the_pipe() {
        let applied = super::AppliedMapSettings {
            graphics: super::MapGraphics {
                shadows: false,
                samples: 2,
                ..Default::default()
            },
            audio: super::MapAudio {
                music: 15.0,
                ..Default::default()
            },
            counters: super::CounterFilter::Countries {
                tags: vec!["Opaque mod | country".into()],
            },
            visual_mods: vec!["workshop/1".into()],
        };
        let state = super::DashboardMapState {
            id: "m".into(),
            applied: Some(applied.clone()),
            ..Default::default()
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["applied"]["graphics"]["shadows"], false);
        assert_eq!(json["applied"]["graphics"]["samples"], 2);
        assert_eq!(json["applied"]["audio"]["music"], 15.0);
        assert_eq!(json["applied"]["counters"]["kind"], "countries");
        assert_eq!(json["applied"]["visualMods"][0], "workshop/1");
        let back: super::DashboardMapState = serde_json::from_value(json).unwrap();
        assert_eq!(back.applied, Some(applied));
        let unsent: super::DashboardMapState =
            serde_json::from_str(r#"{"id":"m","sourceId":null,"revision":null,"branchId":null,"hour":null,"fraction":null,"sequence":0,"requestedSequence":0,"loading":true,"error":null}"#)
                .unwrap();
        assert_eq!(unsent.applied, None);
    }

    #[test]
    fn widget_exports_keep_their_target_and_reject_invalid_ids() {
        for view_id in [None, Some("campaign-map")] {
            assert!(super::validate_export("C:/map.png", view_id).is_ok());
        }
        assert!(super::validate_export("C:/map.png", Some("")).is_err());
    }

    #[test]
    fn visual_mod_selections_refuse_only_empty_and_repeated_ids() {
        let choice = |id: &str| content_reader_layers::content::visual::Choice {
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
            pressed: 0,
        };
        assert_eq!(
            GamepadMessage::merge(Some(take(3, 0.1)), update(3, 0.5)),
            take(3, 0.5)
        );
        assert_eq!(
            GamepadMessage::merge(Some(update(3, 0.1)), update(3, 0.5)),
            update(3, 0.5)
        );
        assert_eq!(
            GamepadMessage::merge(Some(GamepadMessage::Release), update(3, 0.5)),
            GamepadMessage::Release
        );
        assert_eq!(
            GamepadMessage::merge(Some(take(4, 0.1)), update(3, 0.5)),
            take(4, 0.1)
        );
        assert_eq!(
            GamepadMessage::merge(Some(take(4, 0.1)), GamepadMessage::Release),
            GamepadMessage::Release
        );
        assert_eq!(
            GamepadMessage::merge(Some(GamepadMessage::Release), take(5, 0.2)),
            take(5, 0.2)
        );
        assert_eq!(GamepadMessage::merge(None, update(5, 0.2)), update(5, 0.2));
        let tapped = |handoff, pressed| GamepadMessage::State {
            owner: owner(handoff),
            state: state(0.0),
            pressed,
        };
        let south = GamepadButton::South.bit();
        let east = GamepadButton::East.bit();
        assert_eq!(
            GamepadMessage::merge(Some(tapped(3, south)), tapped(3, east)),
            tapped(3, south | east)
        );
        assert_eq!(
            GamepadMessage::merge(Some(tapped(3, south)), tapped(4, 0)),
            tapped(4, 0)
        );
        let line = serde_json::to_string(&NativeCommand::Gamepad {
            message: take(2, 0.0),
        })
        .unwrap();
        assert!(
            line.starts_with(
                r#"{"command":"gamepad","message":{"kind":"take","owner":{"viewer":1,"handoff":2}"#
            ),
            "{line}"
        );
        assert!(
            GamepadMessage::State {
                owner: owner(1),
                state: GamepadState {
                    triggers: [2.0, 0.0],
                    ..GamepadState::default()
                },
                pressed: 0,
            }
            .validate()
            .is_err()
        );
        assert!(
            GamepadMessage::State {
                owner: owner(1),
                state: GamepadState::default(),
                pressed: 1 << GamepadButton::ALL.len(),
            }
            .validate()
            .is_err()
        );
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
                mode: CameraMode::Walk,
                overlays: true,
            })
            .unwrap(),
            r#"{"event":"camera_mode","mode":"walk","overlays":true}"#
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
    fn an_aircraft_choice_is_any_nonempty_name_without_nul_or_none() {
        let choose = |aircraft: Option<&str>| MapAction::FlyAircraft {
            aircraft: aircraft.map(str::to_owned),
        };
        for name in [
            Some("GER_fighter_equipment_1_entity"),
            Some("modded plane ü"),
            Some(&*"x".repeat(4096)),
            None,
        ] {
            assert!(choose(name).validate().is_ok(), "{name:?}");
        }
        for name in ["", "plane\0entity"] {
            assert!(choose(Some(name)).validate().is_err(), "{name:?}");
        }
        let action: MapAction =
            serde_json::from_str(r#"{"kind":"flyAircraft","aircraft":"light_plane_entity"}"#)
                .unwrap();
        assert!(
            matches!(action, MapAction::FlyAircraft { aircraft: Some(name) } if name == "light_plane_entity")
        );
        assert!(
            serde_json::from_str::<MapAction>(r#"{"kind":"flyAircraft","aircraft":null}"#).is_ok()
        );
        let event = NativeEvent::FlyAircraft {
            aircraft: Some("light_plane_entity".into()),
            state: FlyAircraftState::Refused {
                reason: "its mesh did not load".into(),
            },
        };
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            serde_json::json!({
                "event": "fly_aircraft",
                "aircraft": "light_plane_entity",
                "state": { "kind": "refused", "reason": "its mesh did not load" }
            })
        );
    }

    #[test]
    fn ready_carries_the_renderers_sample_counts() {
        for counts in [Some(vec![1, 2, 4, 8]), None] {
            let event = NativeEvent::Ready {
                hwnd: "42".into(),
                sample_counts: counts.clone(),
            };
            let text = serde_json::to_string(&event).unwrap();
            assert!(matches!(
                serde_json::from_str::<NativeEvent>(&text).unwrap(),
                NativeEvent::Ready { hwnd, sample_counts } if hwnd == "42" && sample_counts == counts
            ));
        }
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
