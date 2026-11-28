use super::*;
use content_reader_raids::{raids::RaidCategory, raids::RaidType as Kind, raids::TargetType};
use replay_data::{
    rows::LexicalTokenRow, rows::RaidCountryRow, rows::RaidTargetRow, rows::RaidTargetStatusRow,
    rows::TableSchema,
};
use replay_frame::frame::TableView;
use viewer_native_gui::test_support::gui;
pub fn parse(source: &str) -> Vec<content_reader_parser::parser::Item> {
    content_reader_parser::parser::parse(source.as_bytes())
        .unwrap()
        .items
}

pub fn set_counts(assets: &mut RaidAssets) {
    let facts = &mut assets.trigger_content;
    let ideologies = facts.ideologies.get_or_insert_with(Default::default);
    let mut names = (*facts.triggers).clone();
    names.counts = Some(content_reader_key_names::key_name_products::count_trigger_names(
        &facts.buildings,
        ideologies,
        &Default::default(),
    ));
    facts.triggers = Arc::new(names);
}

pub fn kind(name: &str, target: TargetType) -> Kind {
    Kind {
        name: name.into(),
        category: Some("air".into()),
        show_target: parse("NOT = { is_in_faction_with = FROM }"),
        target_types: vec![target],
        ..Default::default()
    }
}

pub fn assets() -> RaidAssets {
    content_reader_script::test_game::enter_viewer();
    let mut assets = unnamed_assets();
    set_counts(&mut assets);
    assets
}

pub fn unnamed_assets() -> RaidAssets {
    let shared = reach::test_support::unnamed_assets();
    RaidAssets {
        trigger_content: shared.trigger_content,
        glyphs: [
            "GFX_raid_type_icon_oilfield",
            "GFX_raid_type_icon_commando",
            UNKNOWN_GLYPH,
            OTHER_GLYPH,
        ]
        .map(str::to_owned)
        .into(),
        catalog: Catalog {
            types: vec![
                kind("oilfield", TargetType::Provinces(vec![1, 2, 3, 4])),
                Kind {
                    fire_only_once: true,
                    ..kind("commando", TargetType::Provinces(vec![2, 3]))
                },
                Kind {
                    any_province: true,
                    ..kind("nuke", TargetType::Provinces(vec![1]))
                },
            ],
            categories: BTreeMap::from([(
                "air".into(),
                RaidCategory {
                    name: "air".into(),
                    visible: parse("has_dlc = Gotterdammerung"),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        },
        priority: 30,
        geometry: shared.geometry.into(),
        paths: Err("no province graph in the test".into()),
        moves: shared.moves,
        recorded_indexes: Default::default(),
        table_indexes: Default::default(),
        batch: Ok(100),
        type_keys: Default::default(),
        template_names: Default::default(),
        glyph_keys: Default::default(),
        template_keys: Default::default(),
        source_distances: Default::default(),
        regions: shared.regions,
        region_provinces: BTreeMap::new(),
        day_night: shared.day_night,
        transport_sub_units: Err("the test has no shared sub-unit database".into()),
        combat_minimum_time: shared.combat_minimum_time,
        naval_combat_min_duration: shared.naval_combat_min_duration,
        avoid_enemy: shared.avoid_enemy,
        sites: BTreeMap::new(),
        refused_sites: None,
        equipment: Default::default(),
        equipment_groups: Default::default(),
        equipment_lost: None,
        tooltip: Default::default(),
        defines: Default::default(),
        warnings: Vec::new(),
    }
}

pub fn registered_command_power(
    slots: impl IntoIterator<Item = u32>,
) -> Vec<replay_data::rows::CountryCommandPowerRow> {
    slots
        .into_iter()
        .map(|canonical_slot| replay_data::rows::CountryCommandPowerRow {
            canonical_slot,
            allocated_command_power_raw: 0,
            max_command_power: Some(0),
            max_command_power_factor: Some(0),
            ace_effectiveness_factor: Some(0),
            air_ace_bonuses_factor: Some(0),
        })
        .collect()
}

pub fn registered_launch_inputs() -> replay_data::rows::RaidCountryInputRow {
    replay_data::rows::RaidCountryInputRow {
        air_range_factor: Some(0),
        naval_commando_range_factor: Some(0),
        ..Default::default()
    }
}

pub fn put<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
}

pub fn reserved_faction(frame: &mut DecodedFrame, faction: u64) {
    let mut rows = frame
        .complete::<tables::ReservedCountryNaming>()
        .unwrap()
        .to_vec();
    assert_eq!(rows.len(), 1);
    rows[0].faction = replay_data::rows::Recorded::Value(Some(faction));
    put::<tables::ReservedCountryNaming>(frame, rows);
}

pub fn status(ordinal: u32, province: u32) -> RaidTargetStatusRow {
    RaidTargetStatusRow {
        ordinal,
        valid: 1,
        scope: RaidScope {
            province,
            ..Default::default()
        },
        detected: Vec::new(),
    }
}

pub fn frame() -> DecodedFrame {
    let mut frame = reach::test_support::frame();
    put::<tables::LexicalTokens>(
        &mut frame,
        [
            (OILFIELD, "oilfield"),
            (COMMANDO, "commando"),
            (NUKE, "nuke"),
            (DAM, "dam"),
        ]
        .into_iter()
        .map(|(token, name)| LexicalTokenRow {
            token,
            literal: name.as_bytes().to_vec(),
        })
        .collect(),
    );
    put::<tables::RaidCountries>(
        &mut frame,
        vec![
            RaidCountryRow {
                canonical_slot: 0,
                country: 10,
                eligible_types: vec![OILFIELD, COMMANDO, NUKE],
                used_types: Vec::new(),
            },
            RaidCountryRow {
                canonical_slot: 1,
                country: 11,
                eligible_types: vec![OILFIELD],
                used_types: Vec::new(),
            },
        ],
    );
    frame
}

pub fn positive_frame() -> DecodedFrame {
    let mut frame = frame();
    put::<tables::ReservedCountryNaming>(&mut frame, Vec::new());
    let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
    for row in &mut countries {
        if row.canonical_slot == 0 {
            row.canonical_slot = 2;
        }
    }
    put::<tables::Countries>(&mut frame, countries);
    let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
    for row in &mut links {
        if row.canonical_slot == 0 {
            row.canonical_slot = 2;
        }
    }
    put::<tables::CountryLinks>(&mut frame, links);
    let mut raids = frame.complete::<tables::RaidCountries>().unwrap().to_vec();
    for row in &mut raids {
        if row.canonical_slot == 0 {
            row.canonical_slot = 2;
        }
    }
    raids.sort_by_key(|row| row.canonical_slot);
    put::<tables::RaidCountries>(&mut frame, raids);
    frame
}

pub fn raid(id: u32, parent: u32, ordinal: u32, phase: i32) -> RaidRow {
    RaidRow {
        reference_type: 84,
        id,
        parent_slot: parent,
        ordinal,
        phase,
        ..Default::default()
    }
}

pub fn target(id: u32, province: u32, detected: u8) -> RaidTargetRow {
    RaidTargetRow {
        reference_type: 84,
        id,
        scope: RaidScope {
            province,
            ..Default::default()
        },
        raid_type: OILFIELD,
        detected,
    }
}

pub fn path_assets() -> RaidAssets {
    let mut graph = content_reader_map::province_graph::ProvinceGraph::default();
    for id in 1..=4_u32 {
        let next: Vec<_> = [id.wrapping_sub(1), id + 1]
            .into_iter()
            .filter(|n| (1..=4).contains(n))
            .map(|n| (n, Some(50_000)))
            .collect();
        graph.edges.insert(id, next);
    }
    let mut assets = assets();
    std::sync::Arc::make_mut(&mut assets.geometry).land = BTreeSet::from([3, 4]);
    assets.moves = Ok(content_reader_unit_path::unit_path::MoveGraph::new(
        &graph,
        &assets.geometry,
    ));
    assets.paths = Ok(paths::PathSearch::new(graph));
    assets
}

pub fn draw_resources() -> gui::Fixture {
    let mut resources = gui::fixture();
    resources.add_sprite("GFX_on_map_raids_icons_frame_strip", [52, 52], 6);
    resources.add_sprite("glyph", [32, 32], 1);
    resources.add_sprite("plus", [32, 32], 1);
    resources.add_sprite(UNKNOWN_GLYPH, [32, 32], 1);
    resources.add_sprite(OTHER_GLYPH, [32, 32], 1);
    for glyph in ["GFX_raid_type_icon_oilfield", "GFX_raid_type_icon_commando"] {
        resources.add_sprite(glyph, [32, 32], 1);
    }
    resources
}

pub const OILFIELD: u32 = 100;

pub const COMMANDO: u32 = 101;

pub const NUKE: u32 = 102;

pub const DAM: u32 = 200;

pub const DRAW_GUI: &[u8] = br#"guiTypes = { containerWindowType = { name = raid_map_icon size = { width = 100 height = 100 }
        containerWindowType = { name = raid_map_icon_button_container position = { x = 25 y = 25 }
            buttonType = { name = raid_map_icon_frame spriteType = GFX_on_map_raids_icons_frame_strip }
            buttonType = { name = raid_map_icon_button spriteType = glyph position = { x = 10 y = 10 } }
            iconType = { name = plus_icon spriteType = plus position = { x = 10 y = 10 } }
        }
    } }"#;
