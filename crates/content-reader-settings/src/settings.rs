use crate::{
    fixed5::decimal_i32,
    gui::native::{Case, PersistentKey, PointAxis},
    parser::{self, Document, Item},
};
use std::{
    fs,
    io::{self, Read},
    path::Path,
};

fn bounded_read(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(256 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 256 * 1024 {
        return Err(io::Error::other("Settings file exceeds its size limit"));
    }
    Ok(bytes)
}

fn bounded_parse<T: Send + Sync + 'static>(path: &Path, parser: &'static str, read: impl FnOnce(&[u8]) -> io::Result<T>) -> io::Result<std::sync::Arc<T>> {
    let tables = engine_executable::executable_tables::current()
        .map_or_else(Vec::new, |tables| tables.identity().sha256.to_vec());
    content_reader_layers::content::Generation::parse_snapshot(path, parser, 1, &tables, bounded_read, |bytes| {
        if bytes.len() > 256 * 1024 { return Err(io::Error::other("Settings file exceeds its size limit")); }
        read(bytes)
    })
}

fn scalar<'a>(items: &'a [Item], key: &str) -> io::Result<&'a str> {
    items
        .iter()
        .find(|item| item.key.as_deref() == Some(key.as_bytes()))
        .and_then(|item| item.value.scalar_str())
        .ok_or_else(|| io::Error::other(format!("Missing {key} in game settings")))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsKeys(Vec<crate::gui::native::PersistentKey>, Absent);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Absent(bool);

pub mod constructed {
    pub const CAMERA_TILT: f32 = 100.0;
    pub const CAMERA_SPEED: f32 = 50.0;
    pub const ZOOM_SPEED: f32 = 50.0;
    pub const SCROLL_SPEED: f32 = 50.0;
    pub const CORNER_SCROLLING: bool = true;
    pub const GUI_SCALE: f32 = 1.0;
    pub const VSYNC: bool = true;
}

fn settings_case(keyword: &str) -> Option<Case> {
    match keyword {
        "graphics" => Some(Case::Object(graphics_case)),
        "maprenderingoptions" => Some(Case::Object(map_rendering_case)),
        "skip_setup"
        | "telemetry_enabled"
        | "lastplayer"
        | "camera_speed"
        | "lockable"
        | "centered_zoom_in"
        | "camera_tilt"
        | "lasthost"
        | "outliner"
        | "outliner_open"
        | "windowmovetime"
        | "radialstaytime"
        | "hints"
        | "strategic_regions"
        | "editor"
        | "debug_saves"
        | "autosave_tocloud"
        | "combat_sound_random"
        | "max_players"
        | "force_pow2_textures"
        | "difficulty"
        | "autosave"
        | "is_always_aprils_fools"
        | "show_allied_plans"
        | "save_date"
        | "event_sounds"
        | "popup_minor_events"
        | "counter_color_mode"
        | "template_counter"
        | "editor_postfix"
        | "hide_daynight_cycle"
        | "pause_on_popups"
        | "use_ingame_browser"
        | "save_as_binary"
        | "default_offline_wiki"
        | "popup_news"
        | "popup_events"
        | "awards_message_seen"
        | "show_doctrine_details"
        | "use_silhouette_portraits"
        | "hotkey_activation_delay"
        | "hotkey_actualization_delay"
        | "save_career_to_cloud"
        | "show_game_status"
        | "time_last_sub_message"
        | "dev_master_volume"
        | "language"
        | "sound_fx_volume"
        | "music_volume"
        | "scroll_speed"
        | "camera_rotation_speed"
        | "zoom_speed"
        | "mouse_speed"
        | "master_volume"
        | "ambient_volume"
        | "graceful_exit"
        | "voice_volume"
        | "minimap_visible"
        | "last_game_version"
        | "corner_scrolling" => Some(Case::Scalar),
        _ => None,
    }
}

fn graphics_case(keyword: &str) -> Option<Case> {
    match keyword {
        "size" | "min_gui" => Some(Case::Point),
        "adapter"
        | "fullscreen"
        | "refreshrate"
        | "firstrun"
        | "anti_alias_geometry"
        | "dynamic_lights"
        | "debugmonitor"
        | "useranisotropy"
        | "shadows"
        | "gamma"
        | "vsync"
        | "shadowsize"
        | "multisampling"
        | "anisotropicfiltering"
        | "anisotropic_filtering"
        | "multi_sampling"
        | "borderless"
        | "gui_scale"
        | "maxanisotropy"
        | "max_refresh_rate"
        | "disable_3d_model_viewer" => Some(Case::Scalar),
        _ => None,
    }
}

fn map_rendering_case(keyword: &str) -> Option<Case> {
    match keyword {
        "draw_terrain"
        | "draw_water"
        | "draw_borders"
        | "draw_trees"
        | "draw_rivers"
        | "draw_postfx"
        | "draw_sky"
        | "draw_bloom"
        | "draw_tooltips"
        | "draw_hires_terrain"
        | "draw_citysprawl"
        | "draw_shadows"
        | "draw_weather_effects"
        | "draw_water_reflections"
        | "texture_quality"
        | "draw_units"
        | "draw_buildings"
        | "high_gfx_shaders"
        | "draw_map_full_res" => Some(Case::Scalar),
        _ => None,
    }
}

impl SettingsKeys {
    pub fn read(bytes: &[u8]) -> Self {
        Self(crate::gui::native::persistent_keys(bytes, settings_case), Absent(false))
    }

    pub fn absent() -> Self {
        Self(Vec::new(), Absent(true))
    }

    pub fn is_absent(&self) -> bool {
        self.1.0
    }

    pub fn f32_or_constructed(&self, path: &[&str], constructed: f32) -> Option<f32> {
        self.f32_at(path).or(self.is_absent().then_some(constructed))
    }

    pub fn bool_or_constructed(&self, path: &[&str], constructed: bool) -> Option<bool> {
        self.bool_at(path).or(self.is_absent().then_some(constructed))
    }

    pub fn assignments<'a>(&'a self, keyword: &'a str) -> impl Iterator<Item = &'a PersistentKey> {
        self.0
            .iter()
            .filter(move |key| key.keyword == Some(keyword))
    }

    pub fn assignments_at(&self, path: &[&str]) -> Vec<&PersistentKey> {
        let mut level: Vec<&PersistentKey> = self.0.iter().collect();
        let mut found = Vec::new();
        for (depth, keyword) in path.iter().enumerate() {
            found = level
                .into_iter()
                .filter(|key| key.keyword == Some(*keyword))
                .collect();
            if depth + 1 < path.len() {
                level = found.iter().flat_map(|key| key.keys.iter()).collect();
                found = Vec::new();
            } else {
                level = Vec::new();
            }
        }
        found
    }

    fn stored<T>(&self, path: &[&str], convert: impl Fn(&[u8]) -> Option<T>) -> Option<T> {
        stored_in(self.assignments_at(path), convert)
    }

    pub fn bool_at(&self, path: &[&str]) -> Option<bool> {
        self.stored(path, crate::gui::native::bool_of)
    }

    pub fn i32_at(&self, path: &[&str]) -> Option<i32> {
        self.stored(path, |text| crate::crt::scan_i32(text).stored())
    }

    pub fn f32_at(&self, path: &[&str]) -> Option<f32> {
        self.stored(path, crate::crt::scan_f32)
    }

    pub fn string_at(&self, path: &[&str]) -> Option<Vec<u8>> {
        self.stored(path, |text| {
            Some(if text.contains(&b'\n') {
                b"Unreadable String".to_vec()
            } else {
                text.to_vec()
            })
        })
    }

    pub fn point_at(&self, path: &[&str]) -> Result<[Option<i32>; 2], String> {
        let mut point = [None, None];
        point_in(self.assignments_at(path), &path.join("."), &mut point)?;
        Ok(point)
    }

    pub fn has(&self, path: &[&str]) -> bool {
        !self.assignments_at(path).is_empty()
    }

    pub fn force_pow2_textures(&self) -> bool {
        self.bool_at(&["force_pow2_textures"]).unwrap_or(false)
    }

    pub fn texture_decode(&self) -> crate::image::native::Decode {
        crate::image::native::Decode {
            force_pow2_textures: self.force_pow2_textures(),
        }
    }

    pub fn language(&self) -> Option<String> {
        self.string_at(&["language"])
            .filter(|text| !text.is_empty())
            .map(|text| String::from_utf8_lossy(&text).into_owned())
    }
}

fn stored_in<'a, T>(
    keys: impl IntoIterator<Item = &'a PersistentKey>,
    convert: impl Fn(&[u8]) -> Option<T>,
) -> Option<T> {
    keys.into_iter()
        .filter_map(|key| convert(&key.value))
        .last()
}

fn point_in<'a>(
    keys: impl IntoIterator<Item = &'a PersistentKey>,
    what: &str,
    point: &mut [Option<i32>; 2],
) -> Result<(), String> {
    for key in keys {
        for (axis, value) in key.point.into_iter().flatten().enumerate() {
            match value {
                PointAxis::Untouched => {}
                PointAxis::Value(value) => point[axis] = Some(value),
                PointAxis::Undefined => {
                    return Err(format!(
                        "settings.txt {what} = {{ .. }} gives a first value %i converts nothing \
                         from, so the point reader stores an uninitialised value"
                    ));
                }
            }
        }
    }
    Ok(())
}

pub fn read_keys(user: &Path) -> io::Result<SettingsKeys> {
    match bounded_parse(&user.join("settings.txt"), "game settings keys", |bytes| {
        match engine_executable::executable_tables::current() {
            Some(tables) => drop(tables.keywords.as_ref().map_err(io::Error::from)?),
            None => return Err(io::Error::other(
                "settings.txt is read through the game's keyword table, and none is in scope",
            )),
        }
        Ok(SettingsKeys::read(bytes))
    }) {
        Ok(keys) => Ok(std::sync::Arc::unwrap_or_clone(keys)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(SettingsKeys::absent()),
        Err(error) => Err(error),
    }
}

#[derive(Default)]
struct Languages(std::sync::Mutex<std::collections::BTreeMap<
    (std::path::PathBuf, Option<[u8; 32]>),
    (Option<String>, content_reader_layers::content::diagnostics::Diagnostics),
>>);

pub fn language_in(layers: &content_reader_layers::ResolvedLayers, user: &Path) -> io::Result<Option<String>> {
    layers.admit_generation()?;
    let path = user.join("settings.txt");
    let tables = engine_executable::executable_tables::current().map(|tables| tables.identity().sha256);
    let cache = layers.derived::<Languages>();
    let mut cache = cache.0.lock().unwrap_or_else(|error| error.into_inner());
    let key = (path.clone(), tables);
    if let Some((language, diagnostics)) = cache.get(&key) {
        diagnostics.credit();
        return Ok(language.clone());
    }
    let (value, diagnostics) = content_reader_layers::content::diagnostics::capture(|| {
        layers.with_file_witness(&path, || read_keys(user).map(|keys| keys.language()))
    });
    diagnostics.credit();
    let value = value?;
    layers.admit_generation()?;
    cache.insert(key, (value.clone(), diagnostics));
    Ok(value)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MapOptions {
    pub day_night: bool,
    pub allied_plans: bool,
    pub country_counter_colours: bool,
    pub nato_counters: bool,
}

impl MapOptions {
    pub const DEFAULT: Self = Self {
        day_night: true,
        allied_plans: false,
        country_counter_colours: false,
        nato_counters: false,
    };

    pub fn read(settings: &SettingsKeys) -> (Self, Vec<String>) {
        let mut options = Self::DEFAULT;
        let mut unread = Vec::new();
        let note = |name: &str, value: &[u8], outcome: &str| {
            format!(
                "settings.txt {name} = {:?} could not be read: {outcome}",
                String::from_utf8_lossy(value)
            )
        };
        for (name, field, invert) in [
            ("hide_daynight_cycle", &mut options.day_night, true),
            ("show_allied_plans", &mut options.allied_plans, false),
        ] {
            for key in settings.assignments(name) {
                match crate::gui::native::bool_of(&key.value) {
                    Some(value) => *field = value != invert,
                    None => unread.push(note(name, &key.value, "the game keeps the value before it")),
                }
            }
        }
        for (name, field, selects) in [
            ("counter_color_mode", &mut options.country_counter_colours, 0),
            ("template_counter", &mut options.nato_counters, 1),
        ] {
            let mut stored = Some(*field);
            let mut unknown = None;
            for key in settings.assignments(name) {
                stored = crate::crt::scan_i32(&key.value)
                    .stored()
                    .map(|value| value.clamp(0, 1) == selects);
                if stored.is_none() {
                    if let Some(earlier) = unknown.replace(&key.value) {
                        unread.push(note(name, earlier, "a later line sets it"));
                    }
                } else if let Some(earlier) = unknown.take() {
                    unread.push(note(name, earlier, "a later line sets it"));
                }
            }
            match stored {
                Some(value) => *field = value,
                None => unread.push(note(
                    name,
                    unknown.expect("an unknown value came from a line"),
                    "the game holds an undefined value for it, so the map takes the game's starting value until the option is chosen",
                )),
            }
        }
        (options, unread)
    }
}

pub fn read_pdx(user: &Path) -> io::Result<Option<Document>> {
    match bounded_parse(&user.join("pdx_settings.txt"), "launcher display settings", parser::parse) {
        Ok(document) => Ok(Some(std::sync::Arc::unwrap_or_clone(document))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn read_display(user: &Path) -> io::Result<Display> {
    let settings = read_keys(user)?;
    let pdx = read_pdx(user)?;
    Display::read(&settings, pdx.as_ref().map(|pdx| pdx.items.as_slice()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    Fullscreen,
    Windowed,
    Borderless,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Desktop {
    pub physical: [u32; 2],
    pub dpi: Result<u32, String>,
}

impl Desktop {
    pub fn virtualised_result(&self) -> Result<[u32; 2], String> {
        let dpi = u64::from((*self.dpi.as_ref().map_err(Clone::clone)?).max(1));
        Ok(self
            .physical
            .map(|pixels| ((u64::from(pixels) * 96 + dpi / 2) / dpi) as u32))
    }

    pub fn dpi_scale_result(&self) -> Result<f32, String> {
        Ok((*self.dpi.as_ref().map_err(Clone::clone)?).max(1) as f32 / 96.0)
    }

    pub fn virtualised(&self) -> Option<[u32; 2]> {
        self.virtualised_result().ok()
    }

    pub fn dpi_scale(&self) -> Option<f32> {
        self.dpi_scale_result().ok()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Display {
    pub clamps: Vec<Clamp>,
    pub size: Option<[i32; 2]>,
    pub min_gui: [u32; 2],
    pub mode: DisplayMode,
    pub display_index: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clamp {
    pub gui_scale: Option<f32>,
    pub size: Option<[u32; 2]>,
    pub borderless: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Window {
    Fullscreen,
    Borderless,
    Plain,
}

const GUI_REFERENCE: [f32; 2] = [1280.0, 720.0];
const GUI_SCALE_STEPS: f32 = 10.0;
const GUI_SCALE_MIN: f32 = 0.5;
const CONSTRUCTED_MIN_GUI: [u32; 2] = [1280, 720];
const CONSTRUCTED_GUI_SCALE: f32 = constructed::GUI_SCALE;

impl Display {
    pub fn read(settings: &SettingsKeys, pdx: Option<&[Item]>) -> io::Result<Self> {
        let blocks = settings.assignments_at(&["graphics"]);
        if blocks.is_empty() && !settings.is_absent() {
            return Err(io::Error::other("Missing graphics block in game settings"));
        }
        let pair = |key: &str, point: [Option<i32>; 2], constructed: Option<[u32; 2]>| {
            let invalid = || io::Error::other(format!("Invalid {key} in game settings"));
            let axis = |value: Option<i32>, constructed: Option<u32>| match value {
                Some(value) => u32::try_from(value)
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(invalid),
                None => constructed.ok_or_else(invalid),
            };
            io::Result::Ok(match (point, constructed) {
                ([None, None], constructed) => constructed,
                ([x, y], constructed) => Some([
                    axis(x, constructed.map(|pair| pair[0]))?,
                    axis(y, constructed.map(|pair| pair[1]))?,
                ]),
            })
        };
        let (mut size_point, mut min_gui_point) = ([None, None], [None, None]);
        let (mut fullscreen, mut borderless) = (false, true);
        let mut display_index = -1;
        let mut clamps = Vec::with_capacity(blocks.len());
        let mut size = None;
        for block in blocks {
            let here = |keyword: &'static str| {
                block
                    .keys
                    .iter()
                    .filter(move |key| key.keyword == Some(keyword))
            };
            let gui_scale = match stored_in(here("gui_scale"), crate::crt::scan_f32) {
                Some(scale) if !scale.is_finite() => {
                    return Err(io::Error::other("Invalid gui_scale in game settings"));
                }
                scale => scale,
            };
            point_in(here("size"), "graphics.size", &mut size_point).map_err(io::Error::other)?;
            point_in(here("min_gui"), "graphics.min_gui", &mut min_gui_point)
                .map_err(io::Error::other)?;
            fullscreen =
                stored_in(here("fullscreen"), crate::gui::native::bool_of).unwrap_or(fullscreen);
            borderless =
                stored_in(here("borderless"), crate::gui::native::bool_of).unwrap_or(borderless);
            display_index = stored_in(here("adapter"), |text| crate::crt::scan_i32(text).stored())
                .unwrap_or(display_index);
            size = pair("size", size_point, None)?;
            clamps.push(Clamp {
                gui_scale,
                size,
                borderless,
            });
        }
        let min_gui = pair("min_gui", min_gui_point, Some(CONSTRUCTED_MIN_GUI))?
            .unwrap_or(CONSTRUCTED_MIN_GUI);
        let mut display = Self {
            clamps,
            size: size.map(|size| size.map(|value| value.min(i32::MAX as u32) as i32)),
            min_gui,
            mode: match (fullscreen, borderless) {
                (true, _) => DisplayMode::Fullscreen,
                (false, true) => DisplayMode::Borderless,
                (false, false) => DisplayMode::Windowed,
            },
            display_index,
        };
        if let Some(pdx) = pdx {
            display.apply_pdx(pdx);
        }
        Ok(display)
    }

    fn apply_pdx(&mut self, pdx: &[Item]) {
        for category in pdx {
            if category.key.as_deref() != Some(b"Graphics".as_slice()) {
                continue;
            }
            let Some(entries) = category.value.items() else {
                continue;
            };
            for entry in entries {
                let Some(key) = entry.key.as_deref() else {
                    continue;
                };
                let value = entry
                    .value
                    .items()
                    .and_then(|fields| scalar(fields, "value").ok())
                    .unwrap_or("");
                match key {
                    b"display_index" => self.display_index = decimal_i32(value.as_bytes()),
                    b"display_mode" => {
                        self.mode = match value {
                            "borderless_fullscreen" => DisplayMode::Borderless,
                            "windowed" => DisplayMode::Windowed,
                            _ => DisplayMode::Fullscreen,
                        };
                    }
                    b"fullscreen_resolution" if self.mode != DisplayMode::Windowed => {
                        if let Some(size) = resolution(value) {
                            self.size = Some(size);
                        }
                    }
                    b"windowed_resolution" if self.mode == DisplayMode::Windowed => {
                        if let Some(size) = resolution(value) {
                            self.size = Some(size);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    pub fn effective_gui_scale(&self, display0: Desktop) -> f32 {
        self.effective_gui_scale_result(&[Ok(display0)])
            .unwrap_or(CONSTRUCTED_GUI_SCALE)
    }

    pub fn effective_gui_scale_result(
        &self,
        desktops: &[Result<Desktop, String>],
    ) -> Result<f32, String> {
        let start = self
            .clamps
            .iter()
            .rposition(|clamp| clamp.gui_scale.is_some())
            .unwrap_or(0);
        let mut scale = CONSTRUCTED_GUI_SCALE;
        for clamp in &self.clamps[start..] {
            if let Some(stored) = clamp.gui_scale {
                scale = stored;
            }
            if scale < GUI_SCALE_MIN {
                scale = GUI_SCALE_MIN;
                continue;
            }
            let frame = match (clamp.borderless, clamp.size) {
                (false, Some(size)) => size,
                _ => primary_desktop(desktops)?.virtualised_result()?,
            };
            scale = effective_gui_scale(scale, frame);
        }
        Ok(scale)
    }

    pub fn window_result(
        &self,
        desktops: &[Result<Desktop, String>],
    ) -> Result<(Desktop, Window), String> {
        let (shown_on, found) = match usize::try_from(self.display_index) {
            Ok(index) => match desktops.get(index) {
                Some(Ok(desktop)) => (desktop.clone(), true),
                Some(Err(why)) => return Err(why.clone()),
                None => (primary_desktop(desktops)?, false),
            },
            Err(_) => (primary_desktop(desktops)?, true),
        };
        let window = match self.mode {
            DisplayMode::Fullscreen => Window::Fullscreen,
            DisplayMode::Borderless if found => Window::Borderless,
            DisplayMode::Borderless | DisplayMode::Windowed => Window::Plain,
        };
        Ok((shown_on, window))
    }

    pub fn window(&self, desktops: &[Result<Desktop, String>]) -> Option<(Desktop, Window)> {
        self.window_result(desktops).ok()
    }

    pub fn frame_result(&self, desktops: &[Result<Desktop, String>]) -> Result<[u32; 2], String> {
        let (shown_on, window) = self.window_result(desktops)?;
        match (window, self.size) {
            (Window::Borderless, _) => shown_on.virtualised_result(),
            (_, None) => primary_desktop(desktops)?.virtualised_result(),
            (_, Some([width, height])) if width > 0 && height > 0 => {
                Ok([width as u32, height as u32])
            }
            _ => Err("the game's window size is not one these monitors can show".into()),
        }
    }

    pub fn frame(&self, desktops: &[Result<Desktop, String>]) -> Option<[u32; 2]> {
        self.frame_result(desktops).ok()
    }

    pub fn interface_to_frame(&self, frame: [u32; 2], scale: f32) -> f32 {
        let [width, height] = frame.map(|value| value.max(1) as i64);
        let [min_width, min_height] = self.min_gui.map(i64::from);
        let base = if min_height * width < min_width * height {
            if width < min_width { min_width } else { width }
        } else if height < min_height {
            min_height * width / height
        } else {
            width
        };
        let logical = (base as f32 * (1.0 / scale)) as i32;
        width as f32 / logical.max(1) as f32
    }

    pub fn stretch_result(&self, desktops: &[Result<Desktop, String>]) -> Result<f32, String> {
        let (shown_on, window) = self.window_result(desktops)?;
        match window {
            Window::Plain | Window::Borderless => shown_on.dpi_scale_result(),
            Window::Fullscreen => {
                let frame = self.frame_result(desktops)?;
                Ok((shown_on.physical[0] as f32 / frame[0] as f32)
                    .min(shown_on.physical[1] as f32 / frame[1] as f32))
            }
        }
    }

    pub fn stretch(&self, desktops: &[Result<Desktop, String>]) -> Option<f32> {
        self.stretch_result(desktops).ok()
    }

    pub fn screen_scale_result(&self, desktops: &[Result<Desktop, String>]) -> Result<f32, String> {
        let scale = self.effective_gui_scale_result(desktops)?;
        Ok(self.interface_to_frame(self.frame_result(desktops)?, scale)
            * self.stretch_result(desktops)?)
    }

    pub fn screen_scale(&self, desktops: &[Result<Desktop, String>]) -> Option<f32> {
        self.screen_scale_result(desktops).ok()
    }
}

fn primary_desktop(desktops: &[Result<Desktop, String>]) -> Result<Desktop, String> {
    match desktops.first() {
        Some(Ok(desktop)) => Ok(desktop.clone()),
        Some(Err(why)) => Err(why.clone()),
        None => Err("the game's window size is not one these monitors can show".into()),
    }
}

fn resolution(value: &str) -> Option<[i32; 2]> {
    let parts: Vec<&str> = value.split('x').collect();
    let [width, height] = parts.as_slice() else {
        return None;
    };
    Some([
        decimal_i32(width.as_bytes()),
        decimal_i32(height.as_bytes()),
    ])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Monitor {
    pub device: Vec<u16>,
    pub primary: bool,
    pub has_mode: bool,
    pub desktop: Result<Desktop, String>,
}

pub fn sdl_order(monitors: &[Monitor]) -> Vec<Result<Desktop, String>> {
    let mut devices: Vec<&[u16]> = Vec::new();
    let mut ordered: Vec<Result<Desktop, String>> = Vec::new();
    for want_primary in [true, false] {
        for monitor in monitors
            .iter()
            .filter(|monitor| monitor.primary == want_primary && monitor.has_mode)
        {
            if let Some(index) = devices
                .iter()
                .position(|seen| *seen == monitor.device.as_slice())
            {
                ordered[index] = monitor.desktop.clone();
            } else {
                devices.push(monitor.device.as_slice());
                ordered.push(monitor.desktop.clone());
            }
        }
    }
    ordered
}

#[cfg(windows)]
pub fn desktops() -> io::Result<Vec<Result<Desktop, String>>> {
    use windows_sys::Win32::{
        Foundation::{LPARAM, RECT},
        Graphics::Gdi::{
            DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplayMonitors, EnumDisplaySettingsW,
            GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
        },
        UI::HiDpi::{
            DPI_AWARENESS_PER_MONITOR_AWARE, GetAwarenessFromDpiAwarenessContext, GetDpiForMonitor,
            GetThreadDpiAwarenessContext, MDT_EFFECTIVE_DPI,
        },
    };
    const MONITORINFOF_PRIMARY: u32 = 1;
    struct State {
        want_primary: bool,
        monitors: Vec<Monitor>,
    }
    unsafe extern "system" fn each(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> i32 {
        // SAFETY: `data` points to the `State` stack local alive for `EnumDisplayMonitors`.
        let state = unsafe { &mut *(data as *mut State) };
        let mut info = MONITORINFOEXW {
            monitorInfo: MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFOEXW>() as u32,
                ..Default::default()
            },
            szDevice: [0; 32],
        };
        // SAFETY: `info` is initialized with `cbSize = size_of::<MONITORINFOEXW>()`.
        if unsafe { GetMonitorInfoW(monitor, std::ptr::from_mut(&mut info).cast()) } == 0 {
            return 1;
        }
        let primary = info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0;
        if primary != state.want_primary {
            return 1;
        }
        let device_len = info
            .szDevice
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(info.szDevice.len());
        let device = info.szDevice[..device_len].to_vec();
        let mut mode = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        // SAFETY: `info.szDevice` is NUL-terminated by `GetMonitorInfoW` and `mode.dmSize` is set.
        let has_mode = unsafe {
            EnumDisplaySettingsW(info.szDevice.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode)
        } != 0;
        let desktop = if has_mode && mode.dmPelsWidth > 0 && mode.dmPelsHeight > 0 {
            let (mut dpi, mut unused) = (0u32, 0u32);
            // SAFETY: `monitor` is a live `HMONITOR` from `EnumDisplayMonitors` and outputs are writable.
            let read =
                unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut unused) };
            let dpi = if read != 0 || dpi == 0 {
                Err(format!(
                    "Could not read the display scale of monitor {} (0x{read:08x})",
                    String::from_utf16_lossy(&device)
                ))
            } else {
                Ok(dpi)
            };
            Ok(Desktop {
                physical: [mode.dmPelsWidth, mode.dmPelsHeight],
                dpi,
            })
        } else {
            Err(format!(
                "Monitor {} has no usable display mode",
                String::from_utf16_lossy(&device)
            ))
        };
        state.monitors.push(Monitor {
            device,
            primary,
            has_mode,
            desktop,
        });
        1
    }
    // SAFETY: `GetThreadDpiAwarenessContext` and `GetAwarenessFromDpiAwarenessContext` take no pointers.
    let awareness = unsafe { GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext()) };
    if awareness != DPI_AWARENESS_PER_MONITOR_AWARE {
        return Err(io::Error::other(
            "Monitor sizes need a per-monitor DPI-aware thread",
        ));
    }
    let mut state = State {
        want_primary: true,
        monitors: Vec::new(),
    };
    for want_primary in [true, false] {
        state.want_primary = want_primary;
        // SAFETY: `state` stays alive and exclusively borrowed for the duration of `EnumDisplayMonitors`.
        unsafe {
            EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                Some(each),
                std::ptr::from_mut(&mut state) as LPARAM,
            );
        }
    }
    Ok(sdl_order(&state.monitors))
}

#[cfg(not(windows))]
pub fn desktops() -> io::Result<Vec<Result<Desktop, String>>> {
    Err(io::Error::other("Monitor sizes are only read on Windows"))
}

#[cfg(windows)]
pub fn display_refresh_hz(window: Option<usize>) -> Option<u32> {
    use windows_sys::Win32::Graphics::Gdi::{
        DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW, GetMonitorInfoW, MONITORINFOEXW,
        MONITOR_DEFAULTTONEAREST, MonitorFromWindow,
    };
    let device = window.and_then(|window| {
        let mut info = MONITORINFOEXW {
            monitorInfo: windows_sys::Win32::Graphics::Gdi::MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFOEXW>() as u32,
                ..Default::default()
            },
            szDevice: [0; 32],
        };
        let found = unsafe {
            let monitor = MonitorFromWindow(window as _, MONITOR_DEFAULTTONEAREST);
            !monitor.is_null()
                && GetMonitorInfoW(monitor, std::ptr::from_mut(&mut info).cast()) != 0
        };
        found.then_some(info.szDevice)
    });
    let mut mode = DEVMODEW {
        dmSize: std::mem::size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    let name = device.as_ref().map_or(std::ptr::null(), |device| device.as_ptr());
    let found = unsafe { EnumDisplaySettingsW(name, ENUM_CURRENT_SETTINGS, &mut mode) } != 0;
    (found && mode.dmDisplayFrequency > 1).then_some(mode.dmDisplayFrequency)
}

#[cfg(not(windows))]
pub fn display_refresh_hz(_window: Option<usize>) -> Option<u32> {
    None
}

pub fn effective_gui_scale(authored: f32, frame: [u32; 2]) -> f32 {
    if authored < GUI_SCALE_MIN {
        return GUI_SCALE_MIN;
    }
    let fit = (frame[0] as f32 / GUI_REFERENCE[0]).min(frame[1] as f32 / GUI_REFERENCE[1]);
    let cap = (fit * GUI_SCALE_STEPS) as i32 as f32 / GUI_SCALE_STEPS;
    if cap < authored { cap } else { authored }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enter_settings_tables() {
        use engine_executable::executable_tables::{self, Tables};
        static TABLES: std::sync::OnceLock<Tables> = std::sync::OnceLock::new();
        let tables = TABLES.get_or_init(|| {
            content_reader_script::test_game::executable()
                .words([
                    "adapter",
                    "borderless",
                    "camera_tilt",
                    "counter_color_mode",
                    "draw_map_full_res",
                    "draw_trees",
                    "force_pow2_textures",
                    "graphics",
                    "gui_scale",
                    "hide_daynight_cycle",
                    "hints",
                    "language",
                    "maprenderingoptions",
                    "min_gui",
                    "show_allied_plans",
                    "template_counter",
                    "texture_quality",
                ])
                .build()
        });
        executable_tables::enter(Some(*tables));
    }

    #[cfg(windows)]
    #[test]
    fn cached_language_tracks_edits_absence_and_transient_read_failures() {
        use content_reader_layers::test_support::rewrite_preserving_modified_time;
        use std::os::windows::fs::OpenOptionsExt;
        enter_settings_tables();
        let root = std::env::temp_dir().join(format!("language-input-{}-{:?}", std::process::id(), std::thread::current().id()));
        fs::create_dir_all(&root).unwrap();
        let file = root.join("settings.txt");
        let english = b"language=\"l_english\"";
        let spanish = b"language=\"l_spanish\"";
        fs::write(&file, english).unwrap();
        let scope = content_reader_layers::content::Generation::enter();
        let layers = content_reader_layers::ResolvedLayers::default();
        let exclusive = fs::OpenOptions::new().read(true).write(true).share_mode(0).open(&file).unwrap();
        assert!(language_in(&layers, &root).is_err());
        drop(exclusive);
        let original = SettingsKeys::read(english).language();
        assert_eq!(language_in(&layers, &root).unwrap(), original);
        assert_eq!(language_in(&layers, &root).unwrap(), original);
        rewrite_preserving_modified_time(&file, spanish);
        assert!(layers.validate_generation().is_err());
        assert!(language_in(&layers, &root).is_err());
        drop(layers);
        drop(scope);
        let scope = content_reader_layers::content::Generation::enter();
        let layers = content_reader_layers::ResolvedLayers::default();
        let changed = SettingsKeys::read(spanish).language();
        assert_ne!(original, changed);
        assert_eq!(language_in(&layers, &root).unwrap(), changed);
        drop(layers);
        drop(scope);
        fs::remove_file(&file).unwrap();
        let scope = content_reader_layers::content::Generation::enter();
        let layers = content_reader_layers::ResolvedLayers::default();
        assert_eq!(language_in(&layers, &root).unwrap(), None);
        fs::write(&file, vec![b' '; 256 * 1024 + 1]).unwrap();
        assert!(layers.validate_generation().is_err());
        assert!(language_in(&layers, &root).is_err());
        drop(layers);
        drop(scope);
        let scope = content_reader_layers::content::Generation::enter();
        let layers = content_reader_layers::ResolvedLayers::default();
        assert!(language_in(&layers, &root).unwrap_err().to_string().contains("size limit"));
        drop(layers);
        drop(scope);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn map_options_read_as_the_game_reads_them_and_default_to_its_constructor() {
        enter_settings_tables();
        let read = |text: &str| MapOptions::read(&SettingsKeys::read(text.as_bytes()));
        assert_eq!(read(""), (MapOptions::DEFAULT, Vec::new()));
        let (stored, unread) = read(
            "hide_daynight_cycle=yes
show_allied_plans=yes
counter_color_mode=0
template_counter=1",
        );
        assert!(unread.is_empty());
        assert_eq!(
            stored,
            MapOptions {
                day_night: false,
                allied_plans: true,
                country_counter_colours: true,
                nato_counters: true,
            }
        );
        let counters = |text: &str| {
            let options = read(text).0;
            (options.country_counter_colours, options.nato_counters)
        };
        assert_eq!(counters("counter_color_mode=-3
template_counter=7"), (true, true));
        assert_eq!(counters("template_counter=0x1"), (false, true));
        assert_eq!(counters("template_counter=4294967296"), (false, false));
        assert_eq!(counters("template_counter=1
TEMPLATE_counter=0"), (false, false));
        assert_eq!(counters("Template_Counter=1"), (false, true));
        assert_eq!(counters(r#""template_counter"=1"#), (false, false));
        assert_eq!(counters(r#"template_counter="1""#), (false, true));
    }

    #[test]
    fn unreadable_map_option_lines_keep_the_games_value_and_are_named() {
        enter_settings_tables();
        let read = |text: &str| MapOptions::read(&SettingsKeys::read(text.as_bytes()));
        let (options, unread) = read(
            "hide_daynight_cycle=yes
hide_daynight_cycle=maybe
show_allied_plans={ 1 }
             counter_color_mode=yes
template_counter=1",
        );
        assert_eq!(
            options,
            MapOptions {
                day_night: false,
                nato_counters: true,
                ..MapOptions::DEFAULT
            }
        );
        let named = |option: &str| unread.iter().any(|line| line.contains(option));
        assert!(named("hide_daynight_cycle") && named("show_allied_plans"));
        assert!(named("counter_color_mode") && !named("template_counter"), "{unread:?}");
        let (options, unread) = read("template_counter=-
template_counter=1");
        assert!(options.nato_counters);
        assert_eq!(unread.len(), 1);
        let (options, _) = read("template_counter=1
template_counter=-");
        assert_eq!(options.nato_counters, MapOptions::DEFAULT.nato_counters);
    }

    #[test]
    fn language_is_the_stored_request() {
        enter_settings_tables();
        let language = |text: &str| SettingsKeys::read(text.as_bytes()).language();
        assert_eq!(language(r#"language="l_english""#).as_deref(), Some("l_english"));
        assert_eq!(language("language=l_braz_por").as_deref(), Some("l_braz_por"));
        assert_eq!(language(r#"language="english""#).as_deref(), Some("english"));
        assert_eq!(language(r#"language="l_""#).as_deref(), Some("l_"));
        assert_eq!(language(""), None);
        assert_eq!(language(r#"language="""#), None);
    }

    #[test]
    fn an_absent_settings_file_reads_as_the_constructed_settings() {
        enter_settings_tables();
        let folder = std::env::temp_dir().join(format!("absent-settings-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let keys = read_keys(&folder).unwrap();
        assert!(keys.is_absent());
        assert_eq!(keys.language(), None);
        assert_eq!(MapOptions::read(&keys), (MapOptions::DEFAULT, Vec::new()));
        assert_eq!(
            keys.f32_or_constructed(&["camera_tilt"], constructed::CAMERA_TILT),
            Some(100.0)
        );
        assert_eq!(keys.f32_at(&["camera_tilt"]), None);
        let display = Display::read(&keys, None).unwrap();
        assert!(display.clamps.is_empty() && display.size.is_none());
        assert_eq!(display.min_gui, CONSTRUCTED_MIN_GUI);
        assert_eq!(display.mode, DisplayMode::Borderless);
        assert!(Display::read(&SettingsKeys::read(b""), None).is_err());
        let present = SettingsKeys::read(b"");
        assert!(!present.is_absent());
        assert_eq!(present.f32_or_constructed(&["camera_tilt"], 100.0), None);
        std::fs::remove_dir(&folder).unwrap();
    }

    const OWNER_SETTINGS: &str = "graphics={ size={ x=2048 y=1152 } min_gui={ x=1280 y=720 } gui_scale=1.000000 fullScreen=no borderless=yes adapter=0 }";
    const OWNER_PDX: &str = r#""Graphics"={
        "display_mode"={ value="borderless_fullscreen" version=0 }
        "display_index"={ value="0" version=0 }
        "vsync"={ enabled=yes version=0 }
        "fullscreen_resolution"={ value="2560x1440" version=0 }
        "renderer"={ value="dx11" version=0 }
    }
    "System"={ "language"={ value="l_english" version=0 } }"#;
    const OWNER_MONITOR: Desktop = Desktop {
        physical: [2560, 1440],
        dpi: Ok(120),
    };

    fn display(settings: &str, pdx: Option<&str>) -> io::Result<Display> {
        enter_settings_tables();
        let pdx = pdx.map(|text| parser::parse(text.as_bytes())).transpose()?;
        Display::read(
            &SettingsKeys::read(settings.as_bytes()),
            pdx.as_ref().map(|pdx| pdx.items.as_slice()),
        )
    }

    #[test]
    fn the_engine_clamps_the_scale_to_half_and_to_the_frame_in_tenths() {
        assert_eq!(effective_gui_scale(1.0, [2048, 1152]), 1.0);
        assert_eq!(effective_gui_scale(0.3, [2048, 1152]), 0.5);
        assert_eq!(effective_gui_scale(2.0, [2048, 1152]), 1.6);
        assert_eq!(effective_gui_scale(1.75, [1920, 1200]), 1.5);
        assert_eq!(effective_gui_scale(1.25, [1366, 768]), 1.0);
        assert_eq!(effective_gui_scale(1.0, [800, 600]), 0.6);
        assert_eq!(effective_gui_scale(0.4, [800, 600]), 0.5);
    }

    #[test]
    fn settings_keys_follow_the_lexers_tokens() {
        enter_settings_tables();
        let pow2 = |text: &str| SettingsKeys::read(text.as_bytes()).force_pow2_textures();
        assert!(!pow2(""));
        assert!(pow2("force_pow2_textures=yes"));
        assert!(pow2("Force_Pow2_Textures=yes"));
        assert!(pow2("FORCE_POW2_TEXTURES = yes"));
        assert!(!pow2("\"force_pow2_textures\"=yes"));
        assert!(pow2("force_pow2_textures=\"yes\""));
        assert!(pow2("force_pow2_textures=yes\nforce_pow2_textures=maybe"));
        assert!(!pow2("force_pow2_textures=yes\nforce_pow2_textures=no"));
        assert!(!pow2("force_pow2_textures=YES"));
        assert!(pow2("@on = yes\nforce_pow2_textures=@on"));
        assert!(pow2(
            "graphics={ size={ x=1 y=2 } }\nforce_pow2_textures=yes"
        ));
        assert!(pow2("hints={ force_pow2_textures=yes }"));
        assert!(!pow2("graphics={ force_pow2_textures=yes }"));
        let keys = SettingsKeys::read(
            b"language=l_english graphics={ size={ x=640 y=480 } GUI_Scale=1.5 }
              mapRenderingOptions={ texture_quality=2 }
              synthetic_unknown={ force_pow2_textures=yes } template_counter=1",
        );
        assert_eq!(keys.assignments("language").count(), 1);
        assert_eq!(
            keys.assignments_at(&["graphics", "size"])[0].point,
            Some([PointAxis::Value(640), PointAxis::Value(480)])
        );
        assert_eq!(
            keys.assignments_at(&["graphics", "gui_scale"])[0]
                .value
                .as_slice(),
            b"1.5"
        );
        assert_eq!(
            keys.assignments_at(&["maprenderingoptions", "texture_quality"])[0]
                .value
                .as_slice(),
            b"2"
        );
        assert_eq!(keys.assignments("force_pow2_textures").count(), 0);
        assert_eq!(keys.assignments("template_counter").count(), 1);
        let keys =
            SettingsKeys::read(b"Template_Counter=1\n\"template_counter\"=0\ntemplate_counter=2");
        let values: Vec<&[u8]> = keys
            .assignments("template_counter")
            .map(|key| key.value.as_slice())
            .collect();
        assert_eq!(values, [&b"1"[..], b"2"]);
    }

    #[test]
    fn the_owners_borderless_game_is_stretched_by_windows_to_125_percent() {
        let game = display(OWNER_SETTINGS, Some(OWNER_PDX)).unwrap();
        assert_eq!(game.size, Some([2560, 1440]));
        assert_eq!(game.mode, DisplayMode::Borderless);
        let monitors = [Ok(OWNER_MONITOR)];
        assert_eq!(
            game.window(&monitors),
            Some((OWNER_MONITOR, Window::Borderless))
        );
        assert_eq!(game.frame(&monitors), Some([2048, 1152]));
        assert_eq!(game.effective_gui_scale(OWNER_MONITOR), 1.0);
        assert_eq!(game.interface_to_frame([2048, 1152], 1.0), 1.0);
        assert_eq!(game.stretch(&monitors), Some(1.25));
        assert_eq!(game.screen_scale(&monitors), Some(1.25));
        let large = Display {
            clamps: vec![Clamp {
                gui_scale: Some(2.0),
                ..game.clamps[0]
            }],
            ..game.clone()
        };
        assert_eq!(large.effective_gui_scale(OWNER_MONITOR), 1.6);
        assert_eq!(large.screen_scale(&monitors), Some(2.0));
    }

    #[test]
    fn every_graphics_block_ends_in_its_own_clamp() {
        let two = display(
            "graphics={ gui_scale=2.0 size={ x=1280 y=720 } borderless=no }              graphics={ size={ x=2048 y=1152 } }",
            None,
        )
        .unwrap();
        assert_eq!(two.effective_gui_scale(OWNER_MONITOR), 1.0);
        let once = effective_gui_scale(2.0, [2048, 1152]);
        assert_eq!(once, 1.6);
        let replaced = display(
            "graphics={ gui_scale=2.0 size={ x=1280 y=720 } borderless=no }              graphics={ size={ x=2048 y=1152 } gui_scale=1.5 }",
            None,
        )
        .unwrap();
        assert_eq!(replaced.effective_gui_scale(OWNER_MONITOR), 1.5);
        let small = display(
            "graphics={ gui_scale=0.3 size={ x=800 y=600 } borderless=no }              graphics={ size={ x=640 y=300 } }",
            None,
        )
        .unwrap();
        assert_eq!(small.effective_gui_scale(OWNER_MONITOR), 0.4);
        assert_eq!(effective_gui_scale(0.3, [640, 300]), 0.5);
    }

    #[test]
    fn the_clamp_sees_settings_txt_before_pdx_settings_change_the_mode() {
        let small = "graphics={ size={ x=1280 y=720 } gui_scale=1.5 fullScreen=no borderless=no }";
        let game = display(small, Some(OWNER_PDX)).unwrap();
        assert_eq!(game.mode, DisplayMode::Borderless);
        assert_eq!(game.effective_gui_scale(OWNER_MONITOR), 1.0);
    }

    #[test]
    fn absent_settings_keep_the_constructors_values() {
        let game = display("graphics={ }", None).unwrap();
        assert_eq!(
            game.clamps,
            [Clamp {
                gui_scale: None,
                size: None,
                borderless: true
            }]
        );
        assert_eq!(game.mode, DisplayMode::Borderless);
        assert_eq!(game.size, None);
        assert_eq!(game.min_gui, [1280, 720]);
        assert_eq!(game.display_index, -1);
        assert_eq!(game.screen_scale(&[Ok(OWNER_MONITOR)]), Some(1.25));
        let windowed = display("graphics={ gui_scale=2.0 borderless=no }", None).unwrap();
        assert_eq!(windowed.effective_gui_scale(OWNER_MONITOR), 1.6);
        assert_eq!(windowed.frame(&[Ok(OWNER_MONITOR)]), Some([2048, 1152]));
    }

    #[test]
    fn each_resolution_applies_only_in_its_own_mode_as_the_entries_are_reached() {
        let windowed = r#""Graphics"={
            "display_mode"={ value="windowed" }
            "fullscreen_resolution"={ value="2560x1440" }
            "windowed_resolution"={ value="1600x900" }
        }"#;
        let game = display(OWNER_SETTINGS, Some(windowed)).unwrap();
        assert_eq!(
            (game.mode, game.size),
            (DisplayMode::Windowed, Some([1600, 900]))
        );
        assert_eq!(game.frame(&[Ok(OWNER_MONITOR)]), Some([1600, 900]));
        assert_eq!(game.screen_scale(&[Ok(OWNER_MONITOR)]), Some(1.25));
        let late = r#""Graphics"={
            "fullscreen_resolution"={ value="1920x1080" }
            "display_mode"={ value="fullscreen" }
        }"#;
        let game = display(OWNER_SETTINGS, Some(late)).unwrap();
        assert_eq!(
            (game.mode, game.size),
            (DisplayMode::Fullscreen, Some([1920, 1080]))
        );
        let stretch = game.stretch(&[Ok(OWNER_MONITOR)]).unwrap();
        assert!((stretch - 4.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn pdx_values_read_as_the_engine_reads_them() {
        let pdx = |entries: &str| {
            display(
                OWNER_SETTINGS,
                Some(&format!(r#""Graphics"={{ {entries} }}"#)),
            )
            .unwrap()
        };
        let one = pdx(r#""fullscreen_resolution"={ value="2560" }"#);
        assert_eq!(one.size, Some([2048, 1152]));
        let bad = pdx(r#""fullscreen_resolution"={ value="2560xhigh" }"#);
        assert_eq!(bad.size, Some([2560, 0]));
        let full = Display {
            mode: DisplayMode::Fullscreen,
            ..bad
        };
        assert_eq!(full.screen_scale(&[Ok(OWNER_MONITOR)]), None);
        let padded = pdx(r#""fullscreen_resolution"={ value=" 1920x1080p" }"#);
        assert_eq!(padded.size, Some([1920, 1080]));
        let bare = pdx(r#""display_index"={ version=0 } "display_mode"={ version=0 }"#);
        assert_eq!(
            (bare.display_index, bare.mode),
            (0, DisplayMode::Fullscreen)
        );
        assert_eq!(pdx(r#""display_index"={ value="2nd" }"#).display_index, 2);
    }

    #[test]
    fn a_frame_smaller_than_min_gui_squeezes_the_interface() {
        let game = display(OWNER_SETTINGS, None).unwrap();
        assert_eq!(game.min_gui, [1280, 720]);
        assert_eq!(game.interface_to_frame([1024, 576], 1.0), 0.8);
        assert_eq!(game.interface_to_frame([1024, 768], 1.0), 0.8);
        assert!((game.interface_to_frame([2048, 1152], 1.25) - 2048.0 / 1638.0).abs() < 1e-6);
    }

    #[test]
    fn the_window_opens_on_the_monitor_display_index_names() {
        let second = r#""Graphics"={ "display_index"={ value="1" } }"#;
        let game = display(OWNER_SETTINGS, Some(second)).unwrap();
        let tv = Desktop {
            physical: [3840, 2160],
            dpi: Ok(192),
        };
        let desktops = [Ok(OWNER_MONITOR), Ok(tv.clone())];
        assert_eq!(game.window(&desktops), Some((tv, Window::Borderless)));
        assert_eq!(game.frame(&desktops), Some([1920, 1080]));
        assert_eq!(game.screen_scale(&desktops), Some(2.0));
        assert_eq!(game.screen_scale(&[]), None);
    }

    #[test]
    fn an_index_past_the_monitors_opens_an_ordinary_window_at_size() {
        let missing = r#""Graphics"={ "display_index"={ value="5" } }"#;
        let game = display(OWNER_SETTINGS, Some(missing)).unwrap();
        let monitors = [Ok(OWNER_MONITOR)];
        assert_eq!(game.window(&monitors), Some((OWNER_MONITOR, Window::Plain)));
        assert_eq!(game.frame(&monitors), Some([2048, 1152]));
        assert_eq!(game.screen_scale(&monitors), Some(1.25));
        let unchosen = Display {
            display_index: -1,
            ..game
        };
        assert_eq!(
            unchosen.window(&monitors),
            Some((OWNER_MONITOR, Window::Borderless))
        );
    }

    #[test]
    fn sdl_lists_the_primary_desktop_first_deduplicates_devices_and_skips_missing_modes() {
        let monitor = |name: &str, has_mode, primary, width| Monitor {
            device: name.encode_utf16().collect(),
            primary,
            has_mode,
            desktop: Ok(Desktop {
                physical: [width, 1080],
                dpi: Ok(96),
            }),
        };
        let monitors = [
            monitor(r"\\.\DISPLAY2", true, false, 1920),
            monitor(r"\\.\DISPLAY3", false, false, 800),
            monitor(r"\\.\DISPLAY1", true, true, 2560),
            monitor(r"\\.\DISPLAY2", true, false, 1920),
            monitor(r"\\.\DISPLAY4", true, false, 3840),
        ];
        let widths: Vec<Option<u32>> = sdl_order(&monitors)
            .iter()
            .map(|desktop| desktop.as_ref().ok().map(|desktop| desktop.physical[0]))
            .collect();
        assert_eq!(widths, [Some(2560), Some(1920), Some(3840)]);
    }

    #[test]
    fn unavailable_monitor_before_usable_preserves_secondary_display_index() {
        let primary = OWNER_MONITOR;
        let tv = Desktop {
            physical: [3840, 2160],
            dpi: Ok(192),
        };
        let dpi_err_msg =
            "Could not read the display scale of monitor \\\\.\\DISPLAY2 (0x80070057)".to_owned();
        let dpi_failed_tv = Desktop {
            physical: [3840, 2160],
            dpi: Err(dpi_err_msg.clone()),
        };
        let monitors = [
            Monitor {
                device: r"\\.\DISPLAY1".encode_utf16().collect(),
                primary: true,
                has_mode: true,
                desktop: Ok(primary.clone()),
            },
            Monitor {
                device: r"\\.\DISPLAY2".encode_utf16().collect(),
                primary: false,
                has_mode: true,
                desktop: Ok(dpi_failed_tv.clone()),
            },
            Monitor {
                device: r"\\.\DISPLAY3".encode_utf16().collect(),
                primary: false,
                has_mode: true,
                desktop: Ok(tv.clone()),
            },
        ];
        let desktops = sdl_order(&monitors);
        let at_index = |index: i32| {
            display(
                OWNER_SETTINGS,
                Some(&format!(
                    r#""Graphics"={{ "display_index"={{ value="{index}" }} }}"#
                )),
            )
            .unwrap()
        };
        assert_eq!(
            at_index(0).window(&desktops),
            Some((primary.clone(), Window::Borderless))
        );
        assert_eq!(at_index(0).screen_scale(&desktops), Some(1.25));
        assert_eq!(
            at_index(1).window(&desktops),
            Some((dpi_failed_tv.clone(), Window::Borderless))
        );
        assert!(
            at_index(1)
                .screen_scale_result(&desktops)
                .unwrap_err()
                .contains("DISPLAY2")
        );
        let fullscreen_on_dpi_failed = display(
            "graphics={ size={ x=1920 y=1080 } gui_scale=1.0 fullScreen=yes borderless=no }",
            Some(r#""Graphics"={ "display_index"={ value="1" } "display_mode"={ value="fullscreen" } "fullscreen_resolution"={ value="1920x1080" } }"#),
        )
        .unwrap();
        assert_eq!(fullscreen_on_dpi_failed.frame_result(&desktops), Ok([1920, 1080]));
        assert_eq!(fullscreen_on_dpi_failed.stretch_result(&desktops), Ok(2.0));
        assert_eq!(fullscreen_on_dpi_failed.screen_scale_result(&desktops), Ok(2.0));
        assert_eq!(
            at_index(2).window(&desktops),
            Some((tv.clone(), Window::Borderless))
        );
        assert_eq!(at_index(2).screen_scale(&desktops), Some(2.0));
        assert_eq!(
            at_index(3).window(&desktops),
            Some((primary.clone(), Window::Plain))
        );

        let missing_primary = [Err("primary DPI failed".into()), Ok(tv.clone())];
        assert_eq!(at_index(0).window(&missing_primary), None);
        assert_eq!(
            at_index(1).window(&missing_primary),
            Some((tv.clone(), Window::Borderless))
        );
        assert_eq!(at_index(1).frame(&missing_primary), Some([1920, 1080]));
        assert_eq!(at_index(1).stretch(&missing_primary), Some(2.0));
        assert!(
            at_index(1)
                .screen_scale_result(&missing_primary)
                .unwrap_err()
                .contains("primary DPI failed")
        );
        let explicit_secondary_without_primary_clamp = display(
            "graphics={ size={ x=2048 y=1152 } gui_scale=1.0 borderless=no }",
            Some(r#""Graphics"={ "display_index"={ value="1" } "display_mode"={ value="borderless_fullscreen" } }"#),
        )
        .unwrap();
        assert_eq!(
            explicit_secondary_without_primary_clamp.screen_scale(&missing_primary),
            Some(2.0)
        );
        let empty_clamps = Display {
            clamps: Vec::new(),
            ..at_index(1)
        };
        assert_eq!(empty_clamps.screen_scale(&missing_primary), Some(2.0));
        let min_clamped_borderless = display(
            "graphics={ size={ x=2048 y=1152 } gui_scale=0.3 borderless=yes }",
            Some(r#""Graphics"={ "display_index"={ value="1" } "display_mode"={ value="borderless_fullscreen" } }"#),
        )
        .unwrap();
        assert_eq!(
            min_clamped_borderless.effective_gui_scale_result(&missing_primary),
            Ok(0.5)
        );
        let replaced_scale_block = display(
            "graphics={ gui_scale=1.0 borderless=yes } graphics={ size={ x=1920 y=1080 } gui_scale=1.0 borderless=no }",
            Some(r#""Graphics"={ "display_index"={ value="1" } "display_mode"={ value="borderless_fullscreen" } }"#),
        )
        .unwrap();
        assert_eq!(
            replaced_scale_block.effective_gui_scale_result(&missing_primary),
            Ok(1.0)
        );
        assert_eq!(
            replaced_scale_block.screen_scale(&missing_primary),
            Some(2.0)
        );
    }

    #[test]
    fn present_but_unreadable_display_settings_are_refused() {
        for bad in [
            "",
            "graphics={ size={ x=0 y=1152 } }",
            "graphics={ size={ x=2048 } }",
            "graphics={ size={ x=bad y=1152 } }",
            "graphics={ gui_scale=nan }",
        ] {
            assert!(display(bad, None).is_err(), "{bad}");
        }
        let why = display("graphics={ size={ x=bad y=1152 } }", None)
            .unwrap_err()
            .to_string();
        assert_eq!(
            why,
            "settings.txt graphics.size = { .. } gives a first value %i converts nothing from, so the point reader stores an uninitialised value"
        );
        let kept = display(
            "graphics={ gui_scale=x borderless=maybe adapter=x min_gui={ x=1000 } }",
            None,
        )
        .unwrap();
        assert_eq!(kept.clamps[0].gui_scale, None);
        assert_eq!(
            kept.effective_gui_scale(OWNER_MONITOR),
            CONSTRUCTED_GUI_SCALE
        );
        assert_eq!(kept.mode, DisplayMode::Borderless);
        assert_eq!(kept.display_index, -1);
        assert_eq!(kept.min_gui, [1000, 720]);
    }

    #[test]
    fn nested_settings_keys_follow_the_lexers_tokens() {
        let mixed = display(
            "GRAPHICS={ Size={ x=2048 y=1152 } Min_GUI={ x=1280 y=720 } GUI_Scale=1.25 \
             FullScreen=no BORDERLESS=no Adapter=1 \"gui_scale\"=2.0 }",
            None,
        )
        .unwrap();
        assert_eq!(mixed.clamps[0].gui_scale, Some(1.25));
        assert_eq!(mixed.clamps[0].size, Some([2048, 1152]));
        assert_eq!(mixed.mode, DisplayMode::Windowed);
        assert_eq!(mixed.display_index, 1);
        let later = display(
            "graphics={ gui_scale=1.25 } graphics={ Gui_Scale=1.5 size={ x=1920 y=1080 } }",
            None,
        )
        .unwrap();
        assert_eq!(
            later.clamps,
            [
                Clamp {
                    gui_scale: Some(1.25),
                    size: None,
                    borderless: true
                },
                Clamp {
                    gui_scale: Some(1.5),
                    size: Some([1920, 1080]),
                    borderless: true
                }
            ]
        );
        let keys =
            SettingsKeys::read(b"MapRenderingOptions={ draw_trees={ Draw_Map_Full_Res=no } }");
        assert_eq!(
            keys.bool_at(&["maprenderingoptions", "draw_map_full_res"]),
            Some(false)
        );
        let keys = SettingsKeys::read(
            b"mapRenderingOptions={ DRAW_MAP_FULL_RES=no } \"mapRenderingOptions\"={ draw_map_full_res=yes }",
        );
        assert_eq!(
            keys.bool_at(&["maprenderingoptions", "draw_map_full_res"]),
            Some(false)
        );
        let tilt = SettingsKeys::read(b"Camera_Tilt=50\ncamera_tilt=x");
        assert_eq!(tilt.f32_at(&["camera_tilt"]), Some(50.0));
        assert_eq!(
            SettingsKeys::read(b"LANGUAGE=\"l_german\"")
                .language()
                .unwrap(),
            "l_german"
        );
    }
}
