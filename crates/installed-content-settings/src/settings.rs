//! Read-only access to the configured game's current settings.txt and
//! pdx_settings.txt.
//!
//! `settings.txt` follows the shared settings reader ([`SettingsKeys`]);
//! the generic parser reads only the launcher's `pdx_settings.txt`, applied
//! entry by entry.
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
    fs::File::open(path)?
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 256 * 1024 {
        return Err(io::Error::other("Settings file exceeds its size limit"));
    }
    Ok(bytes)
}

/// A `pdx_settings.txt` entry's scalar.
fn scalar<'a>(items: &'a [Item], key: &str) -> io::Result<&'a str> {
    items
        .iter()
        .find(|item| item.key.as_deref() == Some(key.as_bytes()))
        .and_then(|item| item.value.scalar_str())
        .ok_or_else(|| io::Error::other(format!("Missing {key} in game settings")))
}

/// Top-level `settings.txt` keys, with every occurrence read in file order.
/// Keys dispatch through the settings and system-settings cases; an unknown
/// key is reported and skipped.
///
/// Bare words fold and look up registered keywords without case, so
/// `Template_Counter` is a keyword; quoted `"template_counter"` is a string
/// and reaches no case. Settings load before dynamic token registration.
/// Every consumer uses this walk
/// ([`gui::native::persistent_keys`](crate::gui::native::persistent_keys)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsKeys(Vec<crate::gui::native::PersistentKey>);

/// Scalar fields consume only their stored value. Nested objects use their own
/// field dispatch on the same reader; unhandled fields skip their whole block.
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

/// Point fields read their own tokens; all other admitted fields are scalar.
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
    /// `settings.txt`'s bytes walked as the game walks them.
    pub fn read(bytes: &[u8]) -> Self {
        Self(crate::gui::native::persistent_keys(bytes, settings_case))
    }

    /// Every occurrence dispatched to the keyword's registered lowercased
    /// spelling, in file order.
    pub fn assignments<'a>(&'a self, keyword: &'a str) -> impl Iterator<Item = &'a PersistentKey> {
        self.0
            .iter()
            .filter(move |key| key.keyword == Some(keyword))
    }

    /// Every occurrence of the key `path` names, in file order: the top level's `path[0]`, then
    /// within every occurrence of that sub-object its `path[1]`, and so on (registered lowered
    /// spellings).
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

    /// The value the field `path` holds once the file is read, when a conversion stored one
    /// ([`stored_in`]).
    fn stored<T>(&self, path: &[&str], convert: impl Fn(&[u8]) -> Option<T>) -> Option<T> {
        stored_in(self.assignments_at(path), convert)
    }

    /// Converts the field `path` names to `yes` or `no`.
    pub fn bool_at(&self, path: &[&str]) -> Option<bool> {
        self.stored(path, crate::gui::native::bool_of)
    }

    /// Scans the field `path` names as a signed integer (`%i`).
    pub fn i32_at(&self, path: &[&str]) -> Option<i32> {
        self.stored(path, |text| crate::crt::scan_i32(text).stored())
    }

    /// Scans the field `path` names as a float (`%f`).
    pub fn f32_at(&self, path: &[&str]) -> Option<f32> {
        self.stored(path, crate::crt::scan_f32)
    }

    /// Reads text to its first NUL, or "Unreadable String" when it holds
    /// a line feed.
    pub fn string_at(&self, path: &[&str]) -> Option<Vec<u8>> {
        self.stored(path, |text| {
            Some(if text.contains(&b'\n') {
                b"Unreadable String".to_vec()
            } else {
                text.to_vec()
            })
        })
    }

    /// Reads two integer axes: each keeps the last occurrence that reached
    /// it, `None` when none did. A first scan storing nothing would store an
    /// uninitialised value, which cannot be reproduced; `Err` names it.
    pub fn point_at(&self, path: &[&str]) -> Result<[Option<i32>; 2], String> {
        let mut point = [None, None];
        point_in(self.assignments_at(path), &path.join("."), &mut point)?;
        Ok(point)
    }

    /// Whether the file holds the key `path` names at all.
    pub fn has(&self, path: &[&str]) -> bool {
        !self.assignments_at(path).is_empty()
    }

    /// `force_pow2_textures` after reading the file: exact `yes` or `no`
    /// stores a value; anything else keeps the preceding one. The texture
    /// decoder's flag and the settings field both start false, so an omitted
    /// key leaves power-of-two padding off.
    pub fn force_pow2_textures(&self) -> bool {
        self.bool_at(&["force_pow2_textures"]).unwrap_or(false)
    }

    /// Client texture-decoding state supplied by this `settings.txt`.
    pub fn texture_decode(&self) -> crate::image::native::Decode {
        crate::image::native::Decode {
            force_pow2_textures: self.force_pow2_textures(),
        }
    }

    /// The `language` text as the bare name localisation files use after `l_`.
    pub fn language(&self) -> io::Result<String> {
        let text = self
            .string_at(&["language"])
            .ok_or_else(|| io::Error::other("Missing language in game settings"))?;
        std::str::from_utf8(&text)
            .ok()
            .and_then(|text| text.strip_prefix("l_"))
            .filter(|name| {
                !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            })
            .map(str::to_owned)
            .ok_or_else(|| io::Error::other("Unsupported language in game settings"))
    }
}

/// Each of `keys` in turn converted into one field: the value the last conversion that stored one
/// stored, a conversion that stores nothing keeping the value before it.
fn stored_in<'a, T>(
    keys: impl IntoIterator<Item = &'a PersistentKey>,
    convert: impl Fn(&[u8]) -> Option<T>,
) -> Option<T> {
    keys.into_iter()
        .filter_map(|key| convert(&key.value))
        .last()
}

/// Reads each of `keys` into the two axes of `point`, preserving an axis
/// a read did not reach. A first scan storing nothing would store an
/// uninitialised value, refused by name for `what`.
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

/// The user folder's `settings.txt` through [`SettingsKeys`].
pub fn read_keys(user: &Path) -> io::Result<SettingsKeys> {
    let bytes = bounded_read(&user.join("settings.txt"))?;
    // The game folds its keys through its keyword table: with none in scope it cannot be read.
    match engine_executable::executable_tables::current() {
        Some(tables) => drop(tables.keywords.as_ref().map_err(io::Error::from)?),
        None => {
            return Err(io::Error::other(
                "settings.txt is read through the game's keyword table, and none is in scope",
            ));
        }
    }
    Ok(SettingsKeys::read(&bytes))
}

/// `template_counter`'s value when `settings.txt` stores none: the plain frame.
pub const NATO_COUNTERS_DEFAULT: bool = false;

/// `template_counter`, whether the unit icons take the NATO frame, through [`SettingsKeys`]:
/// bare keywords fold without case, quoted keys reach no case, and each occurrence is read in
/// file order, the last winning. The signed integer scan ([`crate::crt::scan_i32`]) accepts C
/// radix prefixes, accumulates in 64 bits and keeps the low 32 bits; a value is clamped to 0..=1
/// and only 1 selects the frame. An omitted key keeps [`NATO_COUNTERS_DEFAULT`].
///
/// A scan converting nothing (a word, lone sign or brace) leaves an uninitialised stack value:
/// the scan reports an error without storing anything, and recovery does not reset the field.
/// `Err` names it.
pub fn nato_counters(settings: &SettingsKeys) -> Result<bool, String> {
    let mut nato = NATO_COUNTERS_DEFAULT;
    for key in settings.assignments("template_counter") {
        let value = crate::crt::scan_i32(&key.value).stored().ok_or_else(|| {
            format!(
                "settings.txt template_counter = {:?} is not an integer %i converts, so the                  game stores an uninitialised value because the integer scan stores nothing                  and recovery does not reset it",
                String::from_utf8_lossy(&key.value)
            )
        })?;
        nato = value.clamp(0, 1) == 1;
    }
    Ok(nato)
}

/// Reads the user folder's `pdx_settings.txt`, the launcher's display
/// settings; `None` when it does not exist, as the engine skips it then.
pub fn read_pdx(user: &Path) -> io::Result<Option<Document>> {
    match bounded_read(&user.join("pdx_settings.txt")) {
        Ok(bytes) => Ok(Some(parser::parse(&bytes)?)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Reads both display files of a user folder in the engine's order.
pub fn read_display(user: &Path) -> io::Result<Display> {
    let settings = read_keys(user)?;
    let pdx = read_pdx(user)?;
    Display::read(&settings, pdx.as_ref().map(|pdx| pdx.items.as_slice()))
}

/// The display mode selected by full-screen and borderless flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    /// Exclusive full screen.
    Fullscreen,
    /// Neither flag.
    Windowed,
    /// Borderless without exclusive full screen: a window over the desktop.
    Borderless,
}

/// One monitor as the game sees it.
///
/// `hoi4.exe` declares no DPI awareness (its manifest is empty and it never
/// asks for any), so Windows gives it a virtualised desktop and stretches its
/// window by the monitor's scale. The SDL it links reads the desktop mode
/// through that virtualisation: at 125% a 2560x1440 monitor is a 2048x1152
/// desktop to the game, which is where `settings.txt`'s `size` comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Desktop {
    /// The monitor's mode in physical pixels.
    pub physical: [u32; 2],
    /// Its effective DPI; 96 is 100%.
    pub dpi: u32,
}

impl Desktop {
    /// The desktop a DPI-unaware process sees: `MulDiv(pixels, 96, dpi)`.
    pub fn virtualised(&self) -> [u32; 2] {
        let dpi = u64::from(self.dpi.max(1));
        self.physical
            .map(|pixels| ((u64::from(pixels) * 96 + dpi / 2) / dpi) as u32)
    }

    /// How many physical pixels one of the game's window pixels covers.
    pub fn dpi_scale(&self) -> f32 {
        self.dpi.max(1) as f32 / 96.0
    }
}

/// The display facts that decide how large the game's interface appears on
/// screen, in the engine's order.
///
/// Video is initialized before either settings file is read. Display
/// settings start borderless at scale 1.0 and display 0's desktop size.
/// `settings.txt` applies first; each `graphics` block clamps its scale
/// against the size and borderless flag at its end. `pdx_settings.txt`
/// applies next, entry by entry, changing mode, monitor and size without
/// changing scale. The game then applies command-line display overrides;
/// its launch command line is unavailable to the viewer, so those overrides
/// are not applied here. Window creation determines the final frame for
/// interface layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Display {
    /// Every `graphics` block's end in file order, clamping the scale that
    /// block left ([`Display::effective_gui_scale`]).
    pub clamps: Vec<Clamp>,
    /// Size after `pdx_settings.txt`: an unreadable resolution is zero;
    /// `None` retains display 0's desktop from construction.
    pub size: Option<[i32; 2]>,
    /// `graphics.min_gui`, the smallest frame for interface layout.
    pub min_gui: [u32; 2],
    /// The mode after `pdx_settings.txt`.
    pub mode: DisplayMode,
    /// Monitor after `pdx_settings.txt`: `graphics.adapter`, then
    /// `display_index`; negative means not chosen.
    pub display_index: i32,
}

/// One `graphics` block's fields when its closing read finalizes scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clamp {
    /// Last `gui_scale` float stored by the block; `None` retains the
    /// preceding clamp's result, or 1.0 before the first block.
    pub gui_scale: Option<f32>,
    /// Size at the block's end, retaining earlier blocks' writes; `None`
    /// selects the initial display 0 desktop.
    pub size: Option<[u32; 2]>,
    /// Borderless flag at the block's end.
    pub borderless: bool,
}

/// The window produced by the configured display settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Window {
    /// Exclusive full screen at the configured size.
    Fullscreen,
    /// A borderless window at the monitor's desktop.
    Borderless,
    /// Ordinary window at the configured size: windowed, or borderless on
    /// a monitor whose desktop query failed.
    Plain,
}

/// Reference frame size permitting a 1.0 interface scale.
const GUI_REFERENCE: [f32; 2] = [1280.0, 720.0];
/// The scale cap is truncated to tenths.
const GUI_SCALE_STEPS: f32 = 10.0;
/// Lower bound applied before the scale cap.
const GUI_SCALE_MIN: f32 = 0.5;
/// Initial size copied to `min_gui` before replacing it with display 0's
/// desktop.
const CONSTRUCTED_MIN_GUI: [u32; 2] = [1280, 720];
/// Initial interface scale.
const CONSTRUCTED_GUI_SCALE: f32 = 1.0;

impl Display {
    /// Reads `settings.txt`'s `graphics` sub-object ([`SettingsKeys`]: bare
    /// keys in any case, each occurrence in order), then applies a parsed
    /// `pdx_settings.txt`.
    ///
    /// A conversion storing nothing retains the constructor value. Stored
    /// values the viewer cannot lay out with (non-finite scale, nonpositive
    /// size axis, or a size mixing a stored axis with a desktop axis) and
    /// uninitialised point reads are refused. Fields carry between graphics
    /// blocks; every block ends with its own clamp ([`Clamp`]).
    pub fn read(settings: &SettingsKeys, pdx: Option<&[Item]>) -> io::Result<Self> {
        let blocks = settings.assignments_at(&["graphics"]);
        if blocks.is_empty() {
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
        // The adapter starts at -1; this key and the command-line adapter
        // option both write it.
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
            // `size` keeps display 0's desktop on an axis no read reached, which the viewer does
            // not mix with a stored one.
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

    /// Applies every `Graphics` entry in file order. An entry without a
    /// `value` reads as empty text, which integer conversion makes zero.
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
                    // Decimal prefix conversion.
                    b"display_index" => self.display_index = decimal_i32(value.as_bytes()),
                    // borderless_fullscreen is 2, windowed is 1; any other
                    // word selects exclusive full screen.
                    b"display_mode" => {
                        self.mode = match value {
                            "borderless_fullscreen" => DisplayMode::Borderless,
                            "windowed" => DisplayMode::Windowed,
                            _ => DisplayMode::Fullscreen,
                        };
                    }
                    // Each resolution key writes size only in its own mode,
                    // as the mode stands when the entry is reached.
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

    /// Effective interface scale, starting at 1.0 and passing through every
    /// graphics block. A block's stored scale replaces the preceding result
    /// before its closing clamp ([`effective_gui_scale`]). Borderless blocks
    /// cap against display 0's virtualised desktop; other blocks cap against
    /// the configured size, using fields as that block left them.
    pub fn effective_gui_scale(&self, display0: Desktop) -> f32 {
        let mut scale = CONSTRUCTED_GUI_SCALE;
        for clamp in &self.clamps {
            if let Some(stored) = clamp.gui_scale {
                scale = stored;
            }
            let frame = match (clamp.borderless, clamp.size) {
                (false, Some(size)) => size,
                _ => display0.virtualised(),
            };
            scale = effective_gui_scale(scale, frame);
        }
        scale
    }

    /// The monitor the window opens on, and how it opened.
    ///
    /// `display_index` selects a desktop in SDL order. An index beyond the
    /// monitors makes the desktop query fail, turning borderless into an
    /// ordinary window at the configured size, centred on display 0.
    /// A negative index uses the chosen adapter, assumed to be display 0.
    pub fn window(&self, desktops: &[Desktop]) -> Option<(Desktop, Window)> {
        let display0 = *desktops.first()?;
        let named = usize::try_from(self.display_index)
            .ok()
            .map(|index| desktops.get(index).copied());
        let (shown_on, found) = match named {
            Some(Some(desktop)) => (desktop, true),
            Some(None) => (display0, false),
            None => (display0, true),
        };
        let window = match self.mode {
            DisplayMode::Fullscreen => Window::Fullscreen,
            DisplayMode::Borderless if found => Window::Borderless,
            DisplayMode::Borderless | DisplayMode::Windowed => Window::Plain,
        };
        Some((shown_on, window))
    }

    /// The frame the game renders, in its own pixels.
    ///
    /// A borderless window takes its monitor's desktop size, superseding
    /// configured full-screen resolution; other windows retain configured
    /// size. Rendering uses the resulting window size. `None` means no
    /// valid window size, as an unreadable resolution can leave behind.
    pub fn frame(&self, desktops: &[Desktop]) -> Option<[u32; 2]> {
        let display0 = *desktops.first()?;
        let (shown_on, window) = self.window(desktops)?;
        match (window, self.size) {
            (Window::Borderless, _) => Some(shown_on.virtualised()),
            (_, None) => Some(display0.virtualised()),
            (_, Some([width, height])) if width > 0 && height > 0 => {
                Some([width as u32, height as u32])
            }
            _ => None,
        }
    }

    /// Frame pixels per interface pixel.
    ///
    /// Interface layout uses the frame, or `min_gui` expanded to the frame's
    /// aspect when the frame is smaller, divided by scale and truncated.
    /// This is the horizontal ratio; the vertical ratio differs only by
    /// that truncation.
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

    /// Physical pixels per frame pixel.
    ///
    /// An ordinary or borderless window is stretched by its monitor's scale,
    /// since the game is not DPI aware. Exclusive full screen is assumed
    /// to fill the monitor, using the smaller axis when the aspects differ.
    pub fn stretch(&self, desktops: &[Desktop]) -> Option<f32> {
        let (shown_on, window) = self.window(desktops)?;
        Some(match window {
            Window::Plain | Window::Borderless => shown_on.dpi_scale(),
            Window::Fullscreen => {
                let frame = self.frame(desktops)?;
                (shown_on.physical[0] as f32 / frame[0] as f32)
                    .min(shown_on.physical[1] as f32 / frame[1] as f32)
            }
        })
    }

    /// Physical pixels per interface pixel: the game's interface scale as it
    /// appears on screen, given every monitor in SDL's display order.
    pub fn screen_scale(&self, desktops: &[Desktop]) -> Option<f32> {
        let scale = self.effective_gui_scale(*desktops.first()?);
        Some(self.interface_to_frame(self.frame(desktops)?, scale) * self.stretch(desktops)?)
    }
}

/// A `pdx_settings.txt` `WIDTHxHEIGHT`: split on `x`, keeping empty parts,
/// and read each part's decimal prefix ([`decimal_i32`]). Anything other
/// than two parts leaves the size alone.
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

/// One display adapter as `EnumDisplayDevicesW` lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Adapter {
    pub attached: bool,
    pub primary: bool,
    /// Its current mode and DPI.
    pub desktop: Option<Desktop>,
}

/// The monitors in SDL's display order, which the game's display indices and
/// `SDL_GetDesktopDisplayMode(0)` use: attached adapters, the primary first
/// and the rest in `EnumDisplayDevicesW` order, as SDL's Windows backend adds
/// them. **UNVERIFIED** beyond the primary: the order of several secondary
/// monitors has not been compared with a running game.
pub fn sdl_order(adapters: &[Adapter]) -> Vec<Desktop> {
    let attached = || adapters.iter().filter(|adapter| adapter.attached);
    attached()
        .filter(|adapter| adapter.primary)
        .chain(attached().filter(|adapter| !adapter.primary))
        .filter_map(|adapter| adapter.desktop)
        .collect()
}

/// Every monitor in SDL's display order.
///
/// Physical modes and per-monitor DPI are only what they say in a thread that
/// is per-monitor DPI aware; any other thread is refused rather than handed
/// virtualised numbers. A monitor whose DPI cannot be read is refused too.
/// This walks every display adapter, so call it off a frame's path.
#[cfg(windows)]
pub fn desktops() -> io::Result<Vec<Desktop>> {
    use windows_sys::Win32::{
        Foundation::POINT,
        Graphics::Gdi::{
            DEVMODEW, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, DISPLAY_DEVICE_PRIMARY_DEVICE,
            DISPLAY_DEVICEW, ENUM_CURRENT_SETTINGS, EnumDisplayDevicesW, EnumDisplaySettingsW,
            MONITOR_DEFAULTTONEAREST, MonitorFromPoint,
        },
        UI::HiDpi::{
            DPI_AWARENESS_PER_MONITOR_AWARE, GetAwarenessFromDpiAwarenessContext, GetDpiForMonitor,
            GetThreadDpiAwarenessContext, MDT_EFFECTIVE_DPI,
        },
    };
    // SAFETY: both calls take and return plain handles or values.
    let awareness = unsafe { GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext()) };
    if awareness != DPI_AWARENESS_PER_MONITOR_AWARE {
        return Err(io::Error::other(
            "Monitor sizes need a per-monitor DPI-aware thread",
        ));
    }
    let mut adapters = Vec::new();
    for index in 0..64 {
        let mut device = DISPLAY_DEVICEW {
            cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        // SAFETY: `device` is a writable DISPLAY_DEVICEW whose `cb` is set.
        if unsafe { EnumDisplayDevicesW(std::ptr::null(), index, &mut device, 0) } == 0 {
            break;
        }
        let attached = device.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP != 0;
        let mut mode = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        // SAFETY: `DeviceName` is the NUL-terminated name the call above wrote
        // and `mode` is a writable DEVMODEW whose `dmSize` is set.
        let found = unsafe {
            EnumDisplaySettingsW(device.DeviceName.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode)
        } != 0;
        let desktop = if attached && found && mode.dmPelsWidth > 0 && mode.dmPelsHeight > 0 {
            // SAFETY: a display's mode fills the position arm of the union.
            let position = unsafe { mode.Anonymous1.Anonymous2.dmPosition };
            // SAFETY: MonitorFromPoint takes a point by value.
            let monitor = unsafe {
                MonitorFromPoint(
                    POINT {
                        x: position.x,
                        y: position.y,
                    },
                    MONITOR_DEFAULTTONEAREST,
                )
            };
            let (mut dpi, mut unused) = (0u32, 0u32);
            // SAFETY: both outputs are writable u32s.
            let read =
                unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut unused) };
            if read != 0 || dpi == 0 {
                return Err(io::Error::other(format!(
                    "Could not read the display scale of monitor {index} (0x{read:08x})"
                )));
            }
            Some(Desktop {
                physical: [mode.dmPelsWidth, mode.dmPelsHeight],
                dpi,
            })
        } else {
            None
        };
        adapters.push(Adapter {
            attached,
            primary: device.StateFlags & DISPLAY_DEVICE_PRIMARY_DEVICE != 0,
            desktop,
        });
    }
    Ok(sdl_order(&adapters))
}

#[cfg(not(windows))]
pub fn desktops() -> io::Result<Vec<Desktop>> {
    Err(io::Error::other("Monitor sizes are only read on Windows"))
}

/// A scale below 0.5 becomes 0.5; otherwise cap at
/// `min(width / 1280, height / 720)` truncated to tenths.
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

    #[test]
    fn nato_counters_clamp_as_the_game_reads_them_and_default_to_its_constructor() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let read = |text: &str| nato_counters(&SettingsKeys::read(text.as_bytes()));
        assert_eq!(read(""), Ok(NATO_COUNTERS_DEFAULT));
        assert_eq!(read("template_counter=1"), Ok(true));
        assert_eq!(read("template_counter=7"), Ok(true));
        assert_eq!(read("template_counter=-3"), Ok(false));
        // `%i` reads a C hexadecimal prefix, and keeps the low 32 bits: 2^32 stores 0.
        assert_eq!(read("template_counter=0x1"), Ok(true));
        assert_eq!(read("template_counter=4294967296"), Ok(false));
        // Ordered writes: the last occurrence wins.
        assert_eq!(read("template_counter=1
template_counter=0"), Ok(false));
        // Bare keywords fold without case; quoted keys are strings and reach no case.
        assert_eq!(read("Template_Counter=1"), Ok(true));
        assert_eq!(read("template_counter=1
TEMPLATE_counter=0"), Ok(false));
        assert_eq!(read(r#""template_counter"=1"#), Ok(NATO_COUNTERS_DEFAULT));
        // A quoted value is converted like a bare one.
        assert_eq!(read(r#"template_counter="1""#), Ok(true));
        // Counter colours are a map option, never read here, so their value cannot refuse.
        assert_eq!(read("counter_color_mode=yes
template_counter=1"), Ok(true));
        // A value `%i` converts nothing from is the game's undefined read, named.
        for bad in ["template_counter=-", "template_counter={ 1 }", "template_counter=yes"] {
            let why = read(bad).unwrap_err();
            assert!(
                why.contains("integer scan stores nothing")
                    && why.contains("recovery does not reset it"),
                "{bad}: {why}"
            );
        }
    }

    #[test]
    fn language_is_the_bare_name_and_refuses_other_forms() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let language = |text: &str| SettingsKeys::read(text.as_bytes()).language();
        assert_eq!(language(r#"language="l_english""#).unwrap(), "english");
        assert_eq!(language("language=l_braz_por").unwrap(), "braz_por");
        for bad in [
            r#"language="english""#,
            r#"language="l_""#,
            r#"language="l_../x""#,
            "",
        ] {
            assert!(language(bad).is_err(), "{bad}");
        }
    }

    /// Two display files and a 2560x1440 monitor at 125%.
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
        dpi: 120,
    };

    fn display(settings: &str, pdx: Option<&str>) -> io::Result<Display> {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
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
        // 2048 / 1280 = 1152 / 720 = 1.6.
        assert_eq!(effective_gui_scale(2.0, [2048, 1152]), 1.6);
        // min(1920 / 1280, 1200 / 720) = 1.5.
        assert_eq!(effective_gui_scale(1.75, [1920, 1200]), 1.5);
        // 1366 / 1280 = 1.067 truncates to 1.0, not 1.1.
        assert_eq!(effective_gui_scale(1.25, [1366, 768]), 1.0);
        // Below the reference frame the cap can fall under the lower bound.
        assert_eq!(effective_gui_scale(1.0, [800, 600]), 0.6);
        assert_eq!(effective_gui_scale(0.4, [800, 600]), 0.5);
    }

    /// Bare keys fold without case; quoted keys reach no case, variables
    /// resolve, and every occurrence is read in order.
    #[test]
    fn settings_keys_follow_the_lexers_tokens() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let pow2 = |text: &str| SettingsKeys::read(text.as_bytes()).force_pow2_textures();
        assert!(!pow2(""));
        assert!(pow2("force_pow2_textures=yes"));
        assert!(pow2("Force_Pow2_Textures=yes"));
        assert!(pow2("FORCE_POW2_TEXTURES = yes"));
        // A quoted key is a string and reaches no case.
        assert!(!pow2("\"force_pow2_textures\"=yes"));
        // A quoted value still supplies the boolean conversion's text.
        assert!(pow2("force_pow2_textures=\"yes\""));
        // Any other text stores nothing and keeps the field; the last stored value holds.
        assert!(pow2("force_pow2_textures=yes\nforce_pow2_textures=maybe"));
        assert!(!pow2("force_pow2_textures=yes\nforce_pow2_textures=no"));
        assert!(!pow2("force_pow2_textures=YES"));
        assert!(pow2("@on = yes\nforce_pow2_textures=@on"));
        // A block passed over whole by an unexpected key does not hide what follows it.
        assert!(pow2(
            "graphics={ size={ x=1 y=2 } }\nforce_pow2_textures=yes"
        ));
        // A scalar case given a `{` reads nothing further: the block's keys are the file's.
        assert!(pow2("hints={ force_pow2_textures=yes }"));
        assert!(!pow2("graphics={ force_pow2_textures=yes }"));
        // Field dispatch retains scalar, point and nested-object consumption.
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
        // Occurrences keep file order, keyed by the registered spelling.
        let keys =
            SettingsKeys::read(b"Template_Counter=1\n\"template_counter\"=0\ntemplate_counter=2");
        let values: Vec<&[u8]> = keys
            .assignments("template_counter")
            .map(|key| key.value.as_slice())
            .collect();
        assert_eq!(values, [&b"1"[..], b"2"]);
    }

    #[test]
    fn a_monitor_scaled_to_125_percent_is_a_smaller_desktop_to_the_game() {
        assert_eq!(OWNER_MONITOR.virtualised(), [2048, 1152]);
        assert_eq!(OWNER_MONITOR.dpi_scale(), 1.25);
        let plain = Desktop {
            physical: [1920, 1080],
            dpi: 96,
        };
        assert_eq!(plain.virtualised(), [1920, 1080]);
    }

    #[test]
    fn the_owners_borderless_game_is_stretched_by_windows_to_125_percent() {
        let game = display(OWNER_SETTINGS, Some(OWNER_PDX)).unwrap();
        // pdx_settings.txt's 2560x1440 is applied, then superseded: a
        // borderless window is created at the (virtualised) desktop mode.
        assert_eq!(game.size, Some([2560, 1440]));
        assert_eq!(game.mode, DisplayMode::Borderless);
        let monitors = [OWNER_MONITOR];
        assert_eq!(
            game.window(&monitors),
            Some((OWNER_MONITOR, Window::Borderless))
        );
        assert_eq!(game.frame(&monitors), Some([2048, 1152]));
        assert_eq!(game.effective_gui_scale(OWNER_MONITOR), 1.0);
        assert_eq!(game.interface_to_frame([2048, 1152], 1.0), 1.0);
        assert_eq!(game.stretch(&monitors), Some(1.25));
        assert_eq!(game.screen_scale(&monitors), Some(1.25));
        // Display 0's virtualised 2048x1152 caps a larger scale at 1.6.
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

    /// Every graphics block clamps the fields it leaves. A 2.0 scale
    /// capped to 1.0 by the first block's 1280x720 size stays 1.0 when a
    /// second block only enlarges size, whereas one final clamp would give
    /// 1.6. A later stored scale replaces the preceding clamp; a scale raised
    /// to 0.5 at one end can be capped lower at the next.
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
        // 0.3 is raised to 0.5 at the first end (800x600 would cap at 0.6).
        let small = display(
            "graphics={ gui_scale=0.3 size={ x=800 y=600 } borderless=no }              graphics={ size={ x=640 y=300 } }",
            None,
        )
        .unwrap();
        // min(640 / 1280, 300 / 720) = 0.416 truncates to 0.4: 0.5 is capped at 0.4, which one
        // clamp of the stored 0.3 (raised to 0.5, returning at once) would not give.
        assert_eq!(small.effective_gui_scale(OWNER_MONITOR), 0.4);
        assert_eq!(effective_gui_scale(0.3, [640, 300]), 0.5);
    }

    #[test]
    fn the_clamp_sees_settings_txt_before_pdx_settings_change_the_mode() {
        // Windowed in settings.txt: the 1280x720 size caps 1.5 at 1.0, though
        // pdx_settings.txt then makes the window borderless.
        let small = "graphics={ size={ x=1280 y=720 } gui_scale=1.5 fullScreen=no borderless=no }";
        let game = display(small, Some(OWNER_PDX)).unwrap();
        assert_eq!(game.mode, DisplayMode::Borderless);
        assert_eq!(game.effective_gui_scale(OWNER_MONITOR), 1.0);
    }

    #[test]
    fn absent_settings_keep_the_constructors_values() {
        // Borderless at 1.0 over display 0's desktop, min_gui 1280x720.
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
        assert_eq!(game.screen_scale(&[OWNER_MONITOR]), Some(1.25));
        // Windowed with no size: a window of display 0's desktop, the cap
        // measured on that same desktop.
        let windowed = display("graphics={ gui_scale=2.0 borderless=no }", None).unwrap();
        assert_eq!(windowed.effective_gui_scale(OWNER_MONITOR), 1.6);
        assert_eq!(windowed.frame(&[OWNER_MONITOR]), Some([2048, 1152]));
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
        // A windowed game is stretched by the monitor's scale all the same.
        assert_eq!(game.frame(&[OWNER_MONITOR]), Some([1600, 900]));
        assert_eq!(game.screen_scale(&[OWNER_MONITOR]), Some(1.25));
        // The mode entry comes after the resolution: the resolution was
        // applied under `settings.txt`'s borderless mode.
        let late = r#""Graphics"={
            "fullscreen_resolution"={ value="1920x1080" }
            "display_mode"={ value="fullscreen" }
        }"#;
        let game = display(OWNER_SETTINGS, Some(late)).unwrap();
        assert_eq!(
            (game.mode, game.size),
            (DisplayMode::Fullscreen, Some([1920, 1080]))
        );
        // An exclusive 1920x1080 mode fills the 2560x1440 panel.
        let stretch = game.stretch(&[OWNER_MONITOR]).unwrap();
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
        // A value that does not split in two leaves the size alone.
        let one = pdx(r#""fullscreen_resolution"={ value="2560" }"#);
        assert_eq!(one.size, Some([2048, 1152]));
        // Each part is `atol`: digits up to the first other character.
        let bad = pdx(r#""fullscreen_resolution"={ value="2560xhigh" }"#);
        assert_eq!(bad.size, Some([2560, 0]));
        let full = Display {
            mode: DisplayMode::Fullscreen,
            ..bad
        };
        // No window has a zero height, so the scale is not claimed.
        assert_eq!(full.screen_scale(&[OWNER_MONITOR]), None);
        let padded = pdx(r#""fullscreen_resolution"={ value=" 1920x1080p" }"#);
        assert_eq!(padded.size, Some([1920, 1080]));
        // An entry without a value is the empty string: `atol` 0 and, for the
        // mode, neither word, so exclusive full screen.
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
        // 1024x576 is laid out on 1280x720: 0.8 frame pixels per interface pixel.
        assert_eq!(game.interface_to_frame([1024, 576], 1.0), 0.8);
        // A taller frame widens nothing; the height is raised to the aspect.
        assert_eq!(game.interface_to_frame([1024, 768], 1.0), 0.8);
        // At scale 1.25 a 2048-wide frame is 1638 interface pixels wide.
        assert!((game.interface_to_frame([2048, 1152], 1.25) - 2048.0 / 1638.0).abs() < 1e-6);
    }

    #[test]
    fn the_window_opens_on_the_monitor_display_index_names() {
        let second = r#""Graphics"={ "display_index"={ value="1" } }"#;
        let game = display(OWNER_SETTINGS, Some(second)).unwrap();
        let tv = Desktop {
            physical: [3840, 2160],
            dpi: 192,
        };
        let desktops = [OWNER_MONITOR, tv];
        assert_eq!(game.window(&desktops), Some((tv, Window::Borderless)));
        // A 200% 4K monitor is a 1920x1080 borderless frame stretched twice.
        assert_eq!(game.frame(&desktops), Some([1920, 1080]));
        assert_eq!(game.screen_scale(&desktops), Some(2.0));
        assert_eq!(game.screen_scale(&[]), None);
    }

    #[test]
    fn an_index_past_the_monitors_opens_an_ordinary_window_at_size() {
        let missing = r#""Graphics"={ "display_index"={ value="5" } }"#;
        let game = display(OWNER_SETTINGS, Some(missing)).unwrap();
        let monitors = [OWNER_MONITOR];
        // The desktop query fails: no borderless style, the 2048x1152 size,
        // centred on display 0 and stretched by its scale.
        assert_eq!(game.window(&monitors), Some((OWNER_MONITOR, Window::Plain)));
        assert_eq!(game.frame(&monitors), Some([2048, 1152]));
        assert_eq!(game.screen_scale(&monitors), Some(1.25));
        // A negative index is the chosen adapter, display 0.
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
    fn sdl_lists_the_primary_desktop_first_and_skips_detached_adapters() {
        let adapter = |attached, primary, width| Adapter {
            attached,
            primary,
            desktop: Some(Desktop {
                physical: [width, 1080],
                dpi: 96,
            }),
        };
        let adapters = [
            adapter(true, false, 1920),
            adapter(false, false, 800),
            adapter(true, true, 2560),
            adapter(true, false, 3840),
        ];
        let widths: Vec<u32> = sdl_order(&adapters)
            .iter()
            .map(|desktop| desktop.physical[0])
            .collect();
        assert_eq!(widths, [2560, 1920, 3840]);
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
        // A conversion that stores nothing keeps the constructor's field, as the game's does.
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

    /// The `graphics` sub-object's keys go through the lexer as the top level's do: any case, a
    /// quoted key reaching no case, the last occurrence of each field, across every `graphics`
    /// block; `mapRenderingOptions` alike.
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
        // A scalar case given a block leaves its keys to the sub-object.
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
            "german"
        );
    }
}
