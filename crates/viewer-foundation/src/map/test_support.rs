//! Synthetic GUI fixtures shared by foundation and feature tests.

use crate::map::{
    native_effects::synthetic,
    native_gui::{self, Layout, Resources, SpriteInfo, SpriteKind, Texture},
    sprites::SpriteSheet,
};
use installed_content_core::gui::native;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::time::Duration;

/// The GUI viewport every test window is read and built in.
pub const VIEWPORT: [i32; 2] = [1920, 1080];

/// Synthetic sheets, one texture each, progress bars by type size, and one monospace font
/// for every name, with the game's own effect models.
pub struct Fixture {
    pub sheets: BTreeMap<Vec<u8>, SpriteSheet>,
    progress: BTreeMap<Vec<u8>, ([i32; 2], SpriteSheet)>,
    font: crate::map::native_text::Font,
    localisation: installed_content_localisation::localisation::Localisation,
    text_colors: native::fonts::TextColors,
    /// Missing sheets may name a session-registered text sprite; looking it up refuses.
    pub session_sprites: bool,
    /// Sheets that are `frameAnimatedSpriteType`s, looping and playing on show, by their
    /// seconds per frame.
    pub animated: BTreeMap<Vec<u8>, f32>,
    /// Sheets whose texture the resources refuse, by kind and why.
    pub(crate) refused: BTreeMap<Vec<u8>, (native_gui::RefusalKind, String)>,
}

impl Fixture {
    pub fn new(sheets: &[(&str, u32, [u32; 2])], progress: &[(&str, [i32; 2])]) -> Self {
        let sheet = |texture: usize, name: &str, frames: u32, size: [u32; 2]| SpriteSheet {
            name: name.to_owned(),
            texture,
            frames,
            size,
        };
        Self {
            sheets: sheets
                .iter()
                .enumerate()
                .map(|(texture, (name, frames, size))| {
                    (
                        name.as_bytes().to_vec(),
                        sheet(texture, name, *frames, *size),
                    )
                })
                .collect(),
            progress: progress
                .iter()
                .enumerate()
                .map(|(index, (name, size))| {
                    let texels = size.map(|value| value as u32);
                    (
                        name.as_bytes().to_vec(),
                        (*size, sheet(500 + index, name, 1, texels)),
                    )
                })
                .collect(),
            font: crate::map::native_text::monospace("fixture_mono", 7, 14),
            localisation: [].into_iter().collect(),
            text_colors: Default::default(),
            session_sprites: false,
            animated: BTreeMap::new(),
            refused: BTreeMap::new(),
        }
    }
}

impl Resources for Fixture {
    fn physical_viewport(&self) -> Option<[i32; 2]> {
        Some(VIEWPORT)
    }
    fn sprite(&self, name: &[u8]) -> Option<SpriteInfo> {
        if let Some((size, _)) = self.progress.get(name) {
            return Some(SpriteInfo {
                texture: *size,
                frames: 1,
                vertical: false,
                masked: false,
                animated: false,
                kind: SpriteKind::Progress {
                    size: *size,
                    steps: 100,
                    colors: [[1.0; 4]; 2],
                    horizontal: true,
                    textured: true,
                },
                anchor: [0.0; 2],
                effect: Ok(synthetic::progress()),
                hit: native_gui::Hit::Frame,
                missing: None,
                refusals: Vec::new(),
            });
        }
        let sheet = self.sheets.get(name)?;
        Some(SpriteInfo {
            texture: sheet.size.map(|value| value as i32),
            frames: sheet.frames as i32,
            vertical: false,
            masked: false,
            animated: false,
            kind: match self.animated.get(name) {
                Some(&seconds_per_frame) => SpriteKind::FrameAnimated {
                    seconds_per_frame,
                    looping: true,
                    play_on_show: true,
                    pause: 0.0,
                },
                None => SpriteKind::Plain,
            },
            anchor: [0.0; 2],
            effect: Ok(synthetic::buttonstate()),
            hit: native_gui::Hit::Frame,
            missing: None,
            refusals: self.refused.get(name).cloned().into_iter().collect(),
        })
    }

    fn font(&self, _: &[u8]) -> Option<&crate::map::native_text::Font> {
        Some(&self.font)
    }

    fn localisation(&self) -> &installed_content_localisation::localisation::Localisation {
        &self.localisation
    }

    fn text_colors(&self) -> &native::fonts::TextColors {
        &self.text_colors
    }

    fn text_effect(&self) -> Result<crate::map::native_effects::Effect, String> {
        Ok(synthetic::text())
    }

    fn effect(&self, file: &str) -> Result<crate::map::native_effects::Effect, String> {
        Err(format!("custom effect {file}"))
    }

    fn shield_flag(&self, _: bool, _: i32) -> Result<Option<(Texture, [f32; 4])>, String> {
        Ok(None)
    }

    fn text_flag(&self, _: &str) -> Result<Option<Texture>, String> {
        Ok(None)
    }

    fn gui_type(&self, _: &[u8]) -> Option<&native::Element> {
        None
    }

    fn session_text_sprite(&self, name: &[u8]) -> bool {
        self.session_sprites && self.sprite(name).is_none()
    }
}

pub fn part<'l>(layout: &'l Layout, name: &str) -> Option<&'l native_gui::Part> {
    layout
        .parts
        .iter()
        .find(|part| part.path.rsplit('.').next() == Some(name))
}

pub const SYNTHETIC_NAVAL_GUI: &str = r#"guiTypes = {
    containerWindowType = { name = "naval_combat_mapicon" size = { width = 47 height = 59 }
        buttonType = { name = "bg_btn" spriteType = "GFX_fixture_combat"
            position = { x = 3 y = -24 } orientation = center centerPosition = yes }
        buttonType = { name = "frame_btn" spriteType = "GFX_fixture_hq"
            position = { x = 23 y = -11 } centerPosition = yes }
        gridBoxType = { name = "attacker_adds_grid" position = { x = -41 y = -12 }
            size = { width = 100% height = 100%% } slotsize = { width = 42 height = 24 }
            max_slots_horizontal = 1 format = RIGHT }
        gridBoxType = { name = "defender_adds_grid" position = { x = 43 y = -18 }
            size = { width = 72 height = 38 } slotsize = { width = 72 height = 22 }
            max_slots_horizontal = 1 }
        iconType = { name = "progress" spriteType = "GFX_fixture_progress"
            position = { x = 2 y = 4 } orientation = center centerPosition = yes }
        iconType = { name = "progress_small" spriteType = "GFX_fixture_progress_small"
            position = { x = -2 y = -9 } orientation = center centerPosition = yes }
        gridBoxType = { name = "attacker_grid" position = { x = -70 y = 39 }
            size = { width = 100% height = 100%% } slotsize = { width = 44 height = 42 }
            max_slots_horizontal = 1 format = UPPER_LEFT }
        gridBoxType = { name = "defender_grid" position = { x = 39 y = 39 }
            size = { width = 100% height = 100%% } slotsize = { width = 44 height = 42 }
            max_slots_horizontal = 1 format = UPPER_LEFT }
    }
    containerWindowType = { name = "GFX_mapicon_tiny_unit_counter"
        size = { width = 72 height = 92 }
        iconType = { name = "background" spriteType = "GFX_fixture_tiny_plate" }
        iconType = { name = "icon" spriteType = "GFX_fixture_transport"
            position = { x = 5 y = 6 } }
        instantTextboxType = { name = "count" font = "fixture_mono"
            position = { x = 39 y = 5 } maxWidth = 24 maxHeight = 14 format = center }
        containerWindowType = { name = "spotting_bar_container" position = { x = 64 y = 5 }
            iconType = { name = "spotting_bar_bg" spriteType = "GFX_fixture_tiny_spot_bg" }
            containerWindowType = { name = "spotting_bar_fg_container"
                position = { x = 2 y = 3 } size = { width = 5 height = 17 } clipping = yes
                iconType = { name = "spotting_bar_fg" spriteType = "GFX_fixture_tiny_spot_fg" }
            }
        }
    }
}"#;

pub const SYNTHETIC_NAVY_GUI: &str = r#"guiTypes = {
    containerWindowType = { name = "unit_counter_navy" size = { width = 82 height = 45 }
        buttonType = { name = "count_bg" spriteType = "GFX_fixture_navy_plate" }
        iconType = { name = "group_marker" spriteType = "GFX_fixture_group"
            position = { x = 76 y = 4 } }
        iconType = { name = "counter_ideology" spriteType = "GFX_fixture_ideology"
            position = { x = 48 y = 11 } }
        iconType = { name = "counter_overlay" spriteType = "GFX_fixture_overlay"
            position = { x = 5 y = 6 } }
        iconType = { name = "count_selected" spriteType = "GFX_fixture_selected" }
        iconType = { name = "type" spriteType = "GFX_fixture_hull_placeholder"
            position = { x = 5 y = 6 } }
        iconType = { name = "capital_ship" spriteType = "GFX_fixture_capital"
            position = { x = 65 y = 8 } }
        instantTextboxType = { name = "capital_count_txt" font = "fixture_mono"
            position = { x = 39 y = 5 } maxWidth = 32 maxHeight = 14 format = center }
        instantTextboxType = { name = "count_txt" font = "fixture_mono"
            position = { x = 39 y = 23 } maxWidth = 32 maxHeight = 14 format = center }
        iconType = { name = "mission_type" spriteType = "GFX_fixture_mission"
            position = { x = 75 y = 8 } }
        containerWindowType = { name = "spotting_bar_container" position = { x = 81 y = 6 }
            iconType = { name = "spotting_bar_bg" spriteType = "GFX_fixture_spot_background" }
            containerWindowType = { name = "spotting_bar_fg_container"
                position = { x = 2 y = 3 } size = { width = 6 height = 31 } clipping = yes
                iconType = { name = "spotting_bar_fg" spriteType = "GFX_fixture_spot_foreground" }
            }
        }
        iconType = { name = "bar_org" spriteType = "GFX_fixture_org"
            position = { x = 18 y = 21 } rotation = 1.5708 }
        iconType = { name = "bar_str" spriteType = "GFX_fixture_strength"
            position = { x = 18 y = 27 } rotation = 1.5708 }
        iconType = { name = "flag" spriteType = "GFX_fixture_flag"
            position = { x = 7 y = 32 } }
        iconType = { name = "large_flag" spriteType = "GFX_fixture_large_flag"
            position = { x = 6 y = 17 } scale = 0.65 }
        iconType = { name = "low_supply" spriteType = "GFX_fixture_supply"
            position = { x = 30 y = -6 } scale = 0.8 }
        iconType = { name = "pride_of_the_fleet_icon" spriteType = "GFX_fixture_pride"
            position = { x = 29 y = 12 } }
        buttonType = { name = "air_base_icon" spriteType = "GFX_mapicon_air_base"
            position = { x = -22 y = 5 } }
        gridBoxType = { name = "wings_box" position = { x = -42 y = 40 }
            size = { width = 100%% height = 100%% } slotsize = { width = 64 height = 29 }
            max_slots_horizontal = 1 format = UPPER_LEFT }
        iconType = { name = "invalid_orders" spriteType = "GFX_fixture_invalid_orders"
            position = { x = -11 y = -5 } }
        iconType = { name = "no_orders" spriteType = "GFX_fixture_no_orders"
            position = { x = -11 y = -5 } }
    }
}"#;

/// Concurrent WARP devices in one process can fault. The viewer shares one device per
/// process; GPU tests take this lock before requesting an adapter to prevent overlapping
/// test devices.
pub(crate) static GPU_TESTS: Mutex<()> = Mutex::new(());

/// Several WARP devices used at once in one process fault inside `d3d10warp.dll` (probed
/// 2026-09-26: 5 of 6 runs with three concurrent devices, 0 of 6 with one device shared by three
/// threads). The viewer creates one device per process, so only tests need this.
///
/// The one-device-at-a-time guard every GPU test in the crate takes before its adapter
/// ([`GPU_TESTS`]): each harness takes it here, the crate's other
/// GPU tests through here too, so no second device exists while another test runs.
pub fn serial() -> MutexGuard<'static, ()> {
    GPU_TESTS.lock().unwrap_or_else(|error| error.into_inner())
}

pub fn test_tga_rgba8(width: u16, height: u16) -> Vec<u8> {
    let mut tga = vec![0; 18 + usize::from(width) * usize::from(height) * 4];
    tga[2] = 2;
    tga[12..14].copy_from_slice(&width.to_le_bytes());
    tga[14..16].copy_from_slice(&height.to_le_bytes());
    tga[16] = 32;
    tga[17] = 8;
    tga
}

pub fn test_frame(hour: i32) -> replay_frame::frame::DecodedFrame {
    use replay_frame::{frame::DecodedFrame, frame::Metrics, frame::TableView};

    DecodedFrame {
        branch: 1,
        hour,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "unused".into(),
            }
            .into()
        }),
        metrics: Metrics::default(),
    }
}

pub fn test_put(frame: &mut replay_frame::frame::DecodedFrame, rows: replay_data::rows::TableRows) {
    let index = rows.table().index();
    frame.tables[index] = replay_frame::frame::TableView::Complete(rows).into();
}

pub mod allegiance;

/// How long a test waits on another thread before failing instead of hanging.
/// Only a hang guard, never a timing check: a full build saturates every core,
/// and a below-normal-priority worker can then be starved for many seconds.
pub const TEST_HANG_GUARD: Duration = Duration::from_secs(120);

pub mod gpu;
pub mod gui;
pub mod movement_timing;
pub mod province_geometry;
pub mod ui_scale;

pub mod image;
pub mod localisation;
pub mod unit_types;

/// The same ordered equipment documents consumed by model and counter fixtures.
pub fn equipment_from_documents(documents: &[&[u8]]) -> installed_content_core::equipment::Database {
    let paths: Vec<String> = (0..documents.len())
        .map(|index| format!("common/units/equipment/{index:03}.txt"))
        .collect();
    let files: Vec<(&str, &str)> = paths
        .iter()
        .zip(documents)
        .map(|(path, text)| (path.as_str(), std::str::from_utf8(text).unwrap()))
        .collect();
    let (equipment, _) = installed_content_core::equipment::fixture::load(&files);
    assert!(equipment.lost.is_none(), "{:?}", equipment.lost);
    equipment
}
