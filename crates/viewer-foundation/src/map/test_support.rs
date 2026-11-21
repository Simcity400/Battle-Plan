
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

pub const VIEWPORT: [i32; 2] = [1920, 1080];

pub struct Fixture {
    pub sheets: BTreeMap<Vec<u8>, SpriteSheet>,
    progress: BTreeMap<Vec<u8>, ([i32; 2], SpriteSheet)>,
    font: crate::map::native_text::Font,
    localisation: installed_content_localisation::localisation::Localisation,
    text_colors: native::fonts::TextColors,
    pub session_sprites: bool,
    pub animated: BTreeMap<Vec<u8>, f32>,
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

pub(crate) static GPU_TESTS: Mutex<()> = Mutex::new(());

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

pub fn complete_frame(
    hour: i32,
    sample: impl Fn(replay_data::rows::TableId) -> Option<replay_data::rows::TableRows>,
) -> replay_frame::frame::DecodedFrame {
    use replay_data::rows::{TableId, TableRows};
    let mut frame = test_frame(hour);
    for table in TableId::ALL {
        let rows = sample(table).unwrap_or_else(|| match table.key_bits() {
            0 => TableRows::one_default(table),
            _ => TableRows::empty(table),
        });
        test_put(&mut frame, rows);
    }
    frame
}

pub fn put_sample_campaign(frame: &mut replay_frame::frame::DecodedFrame) {
    use replay_data::rows::*;
    let country = |slot: u32, tag: &[u8]| CountryRow {
        id: slot,
        canonical_slot: slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    test_put(
        frame,
        TableRows::Countries(vec![
            country(1, b"AAA"),
            country(2, b"BBB"),
            country(3, b"CCC"),
        ]),
    );
    test_put(
        frame,
        TableRows::CountryLinks(
            (1..=3)
                .map(|slot| CountryLinkRow {
                    id: slot,
                    canonical_slot: slot,
                })
                .collect(),
        ),
    );
    test_put(
        frame,
        TableRows::LocalPlayer(vec![LocalPlayerRow {
            country: 1,
            observer_country: 1,
        }]),
    );
    test_put(
        frame,
        TableRows::Subjects(vec![SubjectRow {
            overlord: 1,
            subject: 3,
            end_hour: i32::MAX,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::Autonomy(vec![AutonomyRow {
            subject: 3,
            state: b"autonomy_puppet".to_vec(),
            ..Default::default()
        }]),
    );
    let province = |id: u32, owner: u32| ProvinceRow {
        id,
        state: owner,
        owner,
        controller: owner,
        ..Default::default()
    };
    test_put(
        frame,
        TableRows::Provinces(vec![
            province(1, 1),
            province(2, 2),
            province(3, 0),
            province(4, 3),
            province(5, 0),
        ]),
    );
    let state = |id: u32| StateRow {
        id,
        owner: id,
        controller: id,
        ..Default::default()
    };
    test_put(frame, TableRows::States((1..=3).map(state).collect()));
    let division = |id: u64, owner: u32, province: u32| DivisionRow {
        id,
        roster_owner: owner,
        logical_owner: owner,
        province,
        template: 1,
        ..Default::default()
    };
    test_put(
        frame,
        TableRows::Divisions(vec![division(10, 1, 1), division(11, 2, 2)]),
    );
    test_put(
        frame,
        TableRows::DivisionStatus(vec![
            DivisionStatusRow {
                division: 10,
                ..Default::default()
            },
            DivisionStatusRow {
                division: 11,
                ..Default::default()
            },
        ]),
    );
    test_put(
        frame,
        TableRows::DivisionMovement(vec![DivisionMovementRow {
            division: 10,
            source: 1,
            target: 2,
            combat_count: 1,
            combat_province: 2,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::LandCombats(vec![LandCombatRow {
            id: 100,
            province: 2,
            attackers: vec![10],
            defenders: vec![11],
            attacker_classes: vec![0],
            defender_classes: vec![0],
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::LandCombatSides(vec![LandCombatSideRow {
            combat: 100,
            attacker_primary: vec![1],
            defender_primary: vec![2],
            ..Default::default()
        }]),
    );
    let fighting = |side: u8, division: u64| LandCombatDivisionRow {
        combat: 100,
        side,
        province: 2,
        division,
        own: Some(replay_data::rows::LandCombatMemberValues {
            ..Default::default()
        }),
        ..Default::default()
    };
    test_put(
        frame,
        TableRows::LandCombatDivisions(vec![fighting(0, 10), fighting(1, 11)]),
    );
    test_put(
        frame,
        TableRows::LandCombatDetails(vec![LandCombatDetailsRow {
            combat: 100,
            province: 2,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::NavalCombats(vec![NavalCombatRow {
            id: 200,
            province: 3,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::NavalCombatDetails(vec![NavalCombatDetailsRow {
            combat: 200,
            province: 3,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::NavalCombatShips(vec![NavalCombatShipRow {
            combat: 200,
            ship: 40,
            owner: 1,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::NavalCombatParticipants(vec![NavalCombatParticipantRow {
            combat: 200,
            ship: 40,
            owner: 1,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::NavalCombatResultDetails(vec![NavalCombatResultDetailsRow {
            result: 300,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::NavalResults(vec![NavalResultRow {
            id: 300,
            province: 3,
            retire_hour: i32::MAX,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: 20,
            fleet: 30,
            roster_owner: 1,
            logical_owner: 1,
            province: 3,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::Fleets(vec![FleetRow {
            id: 30,
            owner: 1,
            home_base: 1,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::Ships(vec![ShipRow {
            id: 40,
            task_force: 20,
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::TaskForceMissions(vec![TaskForceMissionRow {
            task_force: 20,
            owner: 1,
            current_region: 1,
            regions: vec![1],
        }]),
    );
    test_put(
        frame,
        TableRows::AirBases(vec![AirBaseRow {
            id: 50,
            state: Some(1),
            province: Some(1),
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::AirBaseRegistrations(vec![AirBaseRegistrationRow {
            base: 50,
            slot: 1,
            wings: vec![60],
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::AirWings(vec![AirWingRow {
            id: 60,
            owner: 1,
            air_base: 50,
            mission_mask: 2,
            mission_region: Some(1),
            ..Default::default()
        }]),
    );
    test_put(
        frame,
        TableRows::AirRegionalMissions(vec![AirRegionalMissionRow {
            wing: 60,
            ordinal: 1,
            active_order: None,
        }]),
    );
    test_put(
        frame,
        TableRows::WeatherProvinces(
            (1..=5)
                .map(|id| WeatherProvinceRow {
                    id,
                    ..Default::default()
                })
                .collect(),
        ),
    );
}

pub fn map_scoped(
    mut frame: replay_frame::frame::DecodedFrame,
) -> replay_frame::frame::DecodedFrame {
    use replay_data::scope::{self, CaptureMode};
    for table in replay_data::rows::TableId::ALL {
        if table.last_change().present(replay_data::FORMAT_VERSION)
            && !scope::records(CaptureMode::Map, table)
        {
            frame.tables[table.index()] = replay_frame::frame::TableView::NotRecorded {
                mode: CaptureMode::Map,
            }
            .into();
        }
    }
    frame
}

pub fn assert_reads_recorded<T>(
    frame: &replay_frame::frame::DecodedFrame,
    allowed: &[replay_data::rows::TableId],
    read: impl FnOnce() -> T,
) -> T {
    use replay_frame::frame::{TableView, view_reads};
    let (value, reads) = view_reads::record(read);
    let unrecorded: Vec<_> = replay_data::rows::TableId::ALL
        .into_iter()
        .filter(|table| !allowed.contains(table))
        .filter(|table| {
            let view = &frame.tables[table.index()];
            reads.iter().any(|read| read.is(view))
                && view_reads::unrecorded(|| matches!(**view, TableView::NotRecorded { .. }))
        })
        .collect();
    assert!(
        unrecorded.is_empty(),
        "read tables the recording leaves out: {unrecorded:?}"
    );
    value
}

pub mod allegiance;

pub const TEST_HANG_GUARD: Duration = Duration::from_secs(120);

pub mod gpu;
pub mod gui;
pub mod movement_timing;
pub mod province_geometry;
pub mod ui_scale;

pub mod image;
pub mod localisation;
pub mod unit_types;

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
