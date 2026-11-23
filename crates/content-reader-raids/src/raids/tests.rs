use super::*;
use crate::key_names::KeyNames;

const EQUIPMENT: Equipment<'static> = &|key| Some(matches!(key, b"convoy" | b"ship_hull_light"));

fn doc(bytes: &[u8]) -> io::Result<parser::Document> {
    reader::document(bytes, EQUIPMENT, &KeyNames::default(), &mut Vec::new())
}

#[test]
fn raid_reader_notes_keep_diagnostic_kinds() {
    content_reader_script::test_game::enter();
    let mut notes = Vec::new();
    reader::document(
        b"types = { a = { unit_model = { type = x } success_levels = { success = { actor_effects = { free_operative = { operative = x } } } } } }",
        EQUIPMENT,
        &KeyNames::default(),
        &mut notes,
    )
    .unwrap();
    assert!(
        notes
            .iter()
            .any(|(kind, _)| *kind == crate::DiagnosticKind::Misread),
        "{notes:?}"
    );
    assert!(
        notes
            .iter()
            .any(|(kind, _)| *kind == crate::DiagnosticKind::Unsupported),
        "{notes:?}"
    );
}

const TYPES: &[u8] = br#"
types = {
    facility_strike = {
        category = air_raids
        allowed = { has_dlc = x }
        command_power = 25
        custom_map_icon = GFX_raid_type_icon_facility_raid
        show_target = { NOT = { is_in_faction_with = FROM } }
        target_type = { building = { tags = facility level = { min = 1 } } }
        unit_requirements = { equipment = { type = { tactical_bomber } modules = { a b } amount = { min = 70 } } }
        starting_point = { types = { air_base carrier } }
        essential_equipment = { convoy = 40 nukes = 1 }
        additional_equipment = { ship_hull_light = 010 nukes = 010 }
        nuke_type = thermonuclear_bomb
        max_distance = 20
        launchable = { has_war_with = FROM }
        launchable_from = { is_ai = no }
    }
    nuclear_strike = { target_type = { province = any } }
    mine = { target_type = { state = { steel > 69 } state = 12 state = 13 state = any province = { 3 4 } province = 010 future = yes } }
}
"#;

fn types_doc() -> io::Result<parser::Document> {
    let mut names = KeyNames::default();
    names.counts.resources.insert(b"steel".to_vec());
    reader::document(TYPES, EQUIPMENT, &names, &mut Vec::new())
}

#[test]
fn a_type_keeps_the_fields_offers_read_in_database_order() {
    content_reader_script::test_game::enter();
    let categories =
        doc(b"categories = { air_raids = { free_targeting = yes visible = { has_dlc = x } } }")
            .unwrap();
    let catalog = parse(&[categories], &[types_doc().unwrap()], EQUIPMENT).unwrap();
    assert!(catalog.categories["air_raids"].free_targeting);
    let names: Vec<_> = catalog
        .types
        .iter()
        .map(|kind| kind.name.as_str())
        .collect();
    assert_eq!(names, ["facility_strike", "nuclear_strike", "mine"]);
    let facility = &catalog.types[0];
    assert_eq!(facility.allowed[0].key.as_deref(), Some(&b"has_dlc"[..]));
    assert!(catalog.types[1].allowed.is_empty(), "absent allows");
    assert_eq!(facility.command_power_raw, 2_500_000);
    assert_eq!(facility.max_distance_raw, Some(2_000_000));
    assert_eq!(facility.essential_equipment, [("convoy".to_owned(), 40)]);
    assert_eq!((facility.nukes, facility.thermonuclear), (1, true));
    assert_eq!(
        facility.additional_equipment,
        [("ship_hull_light".to_owned(), 10)]
    );
    assert_eq!(facility.additional_nukes, 8);
    assert_eq!(
        facility.launchable[0].key.as_deref(),
        Some(&b"has_war_with"[..])
    );
    assert_eq!(
        facility.launchable_from[0].key.as_deref(),
        Some(&b"is_ai"[..])
    );
    assert_eq!(
        facility.starting_point.types,
        [SourceKind::AirBase, SourceKind::Carrier]
    );
    assert_eq!(
        facility.unit_requirements[0].equipment[0].modules,
        [["a", "b"]]
    );
    assert_eq!(
        facility.target_types,
        [TargetType::Building(BuildingSelector {
            tags: vec!["facility".into()],
            minimum_level: Some(1),
            ..Default::default()
        })]
    );
    assert!(catalog.types[1].any_province);
    let mine = &catalog.types[2];
    assert_eq!(mine.target_types.len(), 2);
    assert_eq!(mine.target_types[0], TargetType::Provinces(vec![3, 4, 8]));
    let TargetType::State { ids, triggers } = &mine.target_types[1] else {
        panic!("state selector");
    };
    assert_eq!(
        (ids.as_slice(), triggers[0].key.as_deref()),
        (&[12, 13][..], Some(&b"steel"[..]))
    );
}

#[test]
fn every_field_updates_the_types_persistent_objects() {
    content_reader_script::test_game::enter();
    let catalog = parse(
        &[],
        &[doc(br#"types = {
                leader = {
                    target_type = { captured_army_leader = yes }
                    target_type = { captured_army_leader = no captured_army_leader = maybe }
                }
                fort = {
                    target_type = { building = { type = custom_fort future = yes } }
                    target_type = { building = { level = { min = 2 } } state = { a = yes } }
                    target_type = { state = { b = yes } }
                    starting_point = { types = { AIR_BASE "carrier" } }
                    starting_point = { types = { naval_base } allow_faction_buildings = yes }
                    essential_equipment = { convoy = 1 }
                    unit_requirements = {
                        battalion_types = { marine = { min = 1 } }
                        battalion_types = { marine = { min = 9 } paratrooper = { max = 2 } }
                    }
                    essential_equipment = { CONVOY = 3 nukes = x }
                    visible = { a = yes } visible = { b = yes }
                }
                quoted = {
                    target_type = { building = { "type" = custom_fort } state = { "always" = yes } }
                }
            }"#)
        .unwrap()],
        EQUIPMENT,
    )
    .unwrap();
    assert!(catalog.get("leader").unwrap().target_types.is_empty());
    let fort = catalog.get("fort").unwrap();
    let TargetType::State { triggers, .. } = &fort.target_types[1] else {
        panic!("state arm");
    };
    assert_eq!(
        fort.target_types[0],
        TargetType::Building(BuildingSelector {
            types: vec!["custom_fort".into()],
            minimum_level: Some(2),
            ..Default::default()
        })
    );
    assert_eq!(triggers.len(), 2);
    assert_eq!(
        (
            fort.starting_point.types.as_slice(),
            fort.starting_point.allow_faction_buildings
        ),
        (&[SourceKind::AirBase, SourceKind::NavalBase][..], true)
    );
    assert_eq!(
        (fort.essential_equipment.as_slice(), fort.nukes),
        (&[("convoy".to_owned(), 3)][..], 0)
    );
    assert_eq!(fort.visible.len(), 2);
    let battalions = &fort.unit_requirements[0].battalions;
    assert_eq!(
        battalions
            .iter()
            .map(|b| (b.name.as_str(), b.min, b.max))
            .collect::<Vec<_>>(),
        [("marine", Some(1), None), ("paratrooper", None, Some(2))]
    );
    let quoted = catalog.get("quoted").unwrap();
    let [TargetType::State { triggers, .. }] = quoted.target_types.as_slice() else {
        panic!("{:?}", quoted.target_types);
    };
    assert!(triggers[0].key_quoted);
}

#[test]
fn type_and_category_names_are_folded_tokens() {
    content_reader_script::test_game::enter();
    let categories =
        doc(b"categories = { Air = { free_targeting = no } AIR = { free_targeting = yes } }")
            .unwrap();
    let catalog = parse(
        &[categories],
        &[doc(b"types = { strike = { category = air } Twice = {} TWICE = {} }").unwrap()],
        EQUIPMENT,
    )
    .unwrap();
    assert_eq!(catalog.categories.keys().collect::<Vec<_>>(), ["AIR"]);
    assert!(catalog.categories["AIR"].free_targeting);
    assert_eq!(catalog.types[0].category.as_deref(), Some("AIR"));
    assert_eq!(catalog.refused, ["Twice"]);
    assert_eq!(catalog.types.len(), 1);
}

#[test]
fn scalar_selector_names_read_the_lexers_current_token() {
    content_reader_script::test_game::enter();
    let catalog = parse(
            &[],
            &[doc(b"types = { t = { target_type = { building = { is_coastal = no type custom_fort } } } }")
                .unwrap()],
            EQUIPMENT,
        )
        .unwrap();
    let [TargetType::Building(selector)] = catalog.types[0].target_types.as_slice() else {
        panic!("{:?}", catalog.types[0].target_types);
    };
    assert_eq!(selector.types, ["custom_fort"]);
}

#[test]
fn launch_buildings_resolve_types_and_tags_in_database_order() {
    content_reader_script::test_game::enter();
    let buildings = crate::buildings::parse(
        b"buildings = {
                launcher = { tags = { launch_group } level_cap = { province_max = 1 } }
                Silo = { level_cap = { province_max = 2 } }
                depot = { tags = { LAUNCH_GROUP } level_cap = { state_max = 1 } }
                other = { level_cap = { province_max = 1 } }
            }",
        &KeyNames::default(),
    )
    .unwrap();
    let mut catalog = parse(
        &[],
        &[
            doc(b"types = { t = { starting_point = { building_types = { SILO launch_group } } } }")
                .unwrap(),
        ],
        EQUIPMENT,
    )
    .unwrap();
    resolve_launch_buildings(&mut catalog, &buildings);
    assert_eq!(
        catalog.types[0].starting_point.building_types,
        ["launcher", "Silo"]
    );
    assert!(
        catalog
            .misreads
            .iter()
            .any(|note| note.1.contains("state building depot"))
    );
}

#[test]
fn a_scalar_container_enters_nothing_and_skipped_fields_are_noted() {
    content_reader_script::test_game::enter();
    let mut notes = Vec::new();
    let types = reader::document(
        b"categories = no types = { t = { unit_model = { type = transport scale = 2 } } }",
        EQUIPMENT,
        &KeyNames::default(),
        &mut notes,
    )
    .unwrap();
    let files = [types];
    let catalog = parse(&files, &files, EQUIPMENT).unwrap();
    assert!(catalog.categories.is_empty());
    assert_eq!(catalog.types.len(), 1);
    assert_eq!(catalog.types[0].unit_model.scale_raw, 200_000);
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes[0].1.contains("categories has no block"), "{notes:?}");
    assert!(
        notes[1]
            .1
            .contains("unit_model field type is reported and skipped"),
        "{notes:?}"
    );
}

#[test]
fn a_lost_equipment_database_refuses_only_types_with_an_undecided_key() {
    content_reader_script::test_game::enter();
    let partial: Equipment<'static> = &|key| (key == b"convoy").then_some(true);
    let types = reader::document(
        br#"types = {
                known = { essential_equipment = { convoy = 2 nukes = 1 } }
                later = { essential_equipment = { convoy = 2 tank = 3 } }
            }"#,
        partial,
        &KeyNames::default(),
        &mut Vec::new(),
    )
    .unwrap();
    let catalog = parse(&[], &[types], partial).unwrap();
    assert_eq!(catalog.types.len(), 1);
    assert_eq!(
        catalog.types[0].essential_equipment,
        [("convoy".to_owned(), 2)]
    );
    assert_eq!(catalog.uncertain[0].name, "later");
    assert!(catalog.uncertain[0].reason.contains("equipment tank"));
    assert_eq!(
        catalog.uncertain[0].kind,
        crate::DiagnosticKind::Unsupported
    );
    let block = reader::document(
        b"types = { later = { essential_equipment = { tank = { 3 } } } }",
        partial,
        &KeyNames::default(),
        &mut Vec::new(),
    );
    assert!(
        block
            .unwrap_err()
            .to_string()
            .contains("raid equipment tank")
    );
}

#[test]
fn reported_keys_add_nothing_and_list_readers_leave_the_value_slot() {
    content_reader_script::test_game::enter();
    let mut notes = Vec::new();
    let types = reader::document(
        br#"types = {
                r = { target_type = { province = { 1 } state ignored } }
                q = {
                    target_type = { "state" = 12 }
                    essential_equipment = { "convoy" = 10 tank = { 3 } convoy = 2 nukes = 1 }
                }
            }"#,
        EQUIPMENT,
        &KeyNames::default(),
        &mut notes,
    )
    .unwrap();
    let catalog = parse(&[], &[types], EQUIPMENT).unwrap();
    assert_eq!(
        catalog.types[0].target_types,
        [
            TargetType::Provinces(vec![1]),
            TargetType::State {
                ids: vec![1],
                triggers: Triggers::default()
            }
        ]
    );
    let q = &catalog.types[1];
    assert!(q.target_types.is_empty());
    assert_eq!(
        (q.essential_equipment.as_slice(), q.nukes),
        (&[("convoy".to_owned(), 2)][..], 1)
    );
    for reported in [
        "`ignored`",
        "key state",
        "equipment convoy",
        "equipment tank",
    ] {
        assert!(
            notes.iter().any(|note| note.1.contains(reported)),
            "{reported}: {notes:?}"
        );
    }
}

#[test]
fn the_icon_key_follows_the_documented_two_step_lookup() {
    content_reader_script::test_game::enter();
    let catalog = parse(&[], &[types_doc().unwrap()], EQUIPMENT).unwrap();
    let facility = &catalog.types[0];
    let known = |name: &str| name.starts_with("GFX_raid_type_icon_facility_raid");
    assert_eq!(
        facility.icon(Some("land_facility"), known).as_deref(),
        Some("GFX_raid_type_icon_facility_raid_land_facility")
    );
    assert_eq!(catalog.types[2].icon(None, known), None);
}

#[test]
fn raids_empty_custom_map_icon_uses_generated_base_and_building_suffix() {
    content_reader_script::test_game::enter();
    let catalog = parse(
        &[],
        &[doc(br#"types = { synthetic_empty = { custom_map_icon = "" } }"#).unwrap()],
        EQUIPMENT,
    )
    .unwrap();
    let kind = &catalog.types[0];
    assert_eq!(kind.custom_map_icon.as_deref(), Some(""));
    assert_eq!(kind.base_icon(), "GFX_raid_type_icon_synthetic_empty");
    assert_eq!(
        kind.icon(Some("synthetic_building"), |key| {
            key == "GFX_raid_type_icon_synthetic_empty_synthetic_building"
        }),
        Some("GFX_raid_type_icon_synthetic_empty_synthetic_building".into())
    );
    assert_eq!(
        kind.icon(Some("synthetic_building"), |key| {
            key == "GFX_raid_type_icon_synthetic_empty"
        }),
        Some("GFX_raid_type_icon_synthetic_empty".into())
    );
    assert_eq!(kind.icon(None, |_| false), None);
}

#[test]
fn a_name_declared_twice_or_thrice_is_refused_once() {
    content_reader_script::test_game::enter();
    let catalog = parse(
        &[],
        &[doc(b"types = { a = {} b = {} a = {} a = { command_power = 1 } }").unwrap()],
        EQUIPMENT,
    )
    .unwrap();
    assert_eq!(catalog.refused, ["a"]);
    assert_eq!(catalog.types.len(), 1);
    assert!(catalog.unsupported.is_empty());
    assert_eq!(catalog.uncertain.len(), 1);
    assert_eq!(catalog.uncertain[0].name, "a");
    assert_eq!(catalog.uncertain[0].declarations.len(), 3);
    assert_eq!(catalog.uncertain[0].kind, crate::DiagnosticKind::Refused);
}

#[test]
fn a_type_with_an_unreadable_field_is_refused_and_the_rest_load() {
    content_reader_script::test_game::enter();
    let catalog = parse(
            &[],
            &[doc(br#"types = {
                    a = { unit_requirements = { battalion_types = { "tanks" = { min = 1 } } } }
                    b = { starting_point = { types = { building } } }
                    c = { command_power = 010 }
                    d = { unit_requirements = { tanks = 3 } starting_point = { types = { railway } } }
                }"#)
            .unwrap()],
            EQUIPMENT,
        )
        .unwrap();
    let refused: Vec<_> = catalog
        .unsupported
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(refused, ["a", "b"]);
    assert!(catalog.unsupported[0].1.contains("tanks"));
    let uncertain: Vec<_> = catalog.uncertain.iter().map(|kind| &kind.name).collect();
    assert_eq!(uncertain, ["a", "b"]);
    assert!(catalog.uncertain[0].declarations.is_empty());
    assert!(catalog.uncertain[0].reason.contains("tanks"));
    assert_eq!(catalog.types[0].command_power_raw, 800_000);
    let d = &catalog.types[1];
    assert_eq!(d.unit_requirements, [UnitRequirement::default()]);
    assert!(d.starting_point.types.is_empty());
}

#[test]
fn the_arrow_and_unit_model_keep_the_constructor_values_until_read() {
    content_reader_script::test_game::enter();
    let catalog = parse(
            &[],
            &[doc(br#"types = {
                    bare = {}
                    drop = {
                        arrow = { type = air }
                        unit_model = { equipment = air_transport start_offset = 15 scale = 0.5 }
                        speed_multiplier = 1.5
                    }
                    convoy = {
                        arrow = { type = land }
                        arrow = { type = naval }
                        unit_model = { entity = ship_entity default_entity = fallback_entity }
                        unit_model = { scale = 2 }
                    }
                    odd = { arrow = { type = air } arrow = { type = zeppelin "type" = land TYPE = LAND }
                        unit_model = { equipment = armor colour = red } }
                    quoted = { arrow = { type = "air" } unit_model = { equipment = "air_transport" } }
                }"#)
            .unwrap()],
            EQUIPMENT,
        )
        .unwrap();
    let bare = catalog.get("bare").unwrap();
    assert_eq!(bare.arrow, RaidArrowKind::Line);
    assert_eq!(bare.unit_model, RaidUnitModel::default());
    assert_eq!(bare.unit_model.scale_raw, 100_000);
    assert_eq!(bare.speed_multiplier_raw, 100_000);
    let drop = catalog.get("drop").unwrap();
    assert_eq!(drop.arrow, RaidArrowKind::Air);
    assert_eq!(
        Some(drop.unit_model.equipment),
        content_reader_script::equipment_stats::category_mask("air_transport")
    );
    assert_eq!(
        (drop.unit_model.start_offset_raw, drop.unit_model.scale_raw),
        (1_500_000, 50_000)
    );
    assert_eq!(drop.speed_multiplier_raw, 150_000);
    let convoy = catalog.get("convoy").unwrap();
    assert_eq!(convoy.arrow, RaidArrowKind::Naval);
    assert_eq!(convoy.unit_model.entity.as_deref(), Some("ship_entity"));
    assert_eq!(
        convoy.unit_model.default_entity.as_deref(),
        Some("fallback_entity")
    );
    assert_eq!(convoy.unit_model.scale_raw, 200_000);
    assert!(bare.unit_animations.is_empty());
    let odd = catalog.get("odd").unwrap();
    assert_eq!(odd.arrow, RaidArrowKind::Land);
    assert_eq!(odd.unit_model.equipment, 0);
    let quoted = catalog.get("quoted").unwrap();
    assert_eq!(quoted.arrow, RaidArrowKind::Line);
    assert_eq!(quoted.unit_model.equipment, 0);
    assert_eq!(RaidArrowKind::Naval.arrow_type(), "active_raid_naval");
}

const GRAPHICS_DEFINES: &str = r#"NDefines = { NMapMode = {
                    PREPARING_RAID_ARROW_COLOR = { 0.15, 0.25, 0.35, 0.45 },
                    READY_RAID_ARROW_COLOR = { 0.65, 0.55, 0.45, 0.35 },
                    ACTIVE_RAID_ARROW_COLOR = { 0.85, 0.75, 0.65, 0.55 } },
                  NGraphics = {
                    RAID_ARROW_BALLISTIC_SHAPE = 0.045, RAID_ARROW_BALLISTIC_MAX_HEIGHT = 17.5,
                    RAID_ARROW_BALLISTIC_MAX_SEGMENT_LENGTH = 2.25, RAID_ARROW_BALLISTIC_MAX_SEGMENTS = 73,
                    RAID_ARROW_AIR_HEIGHT = 14.25, RAID_ARROW_AIR_SLOPE_SOURCE_STEEPNESS = 0.35,
                    RAID_ARROW_AIR_SLOPE_SOURCE_LENGTH = 31.25, RAID_ARROW_AIR_SLOPE_TARGET_STEEPNESS = 0.45,
                    RAID_ARROW_AIR_SLOPE_TARGET_LENGTH = 9.5, RAID_ARROW_AIR_MAX_SEGMENT_LENGTH = 3.75,
                    RAID_ARROW_AIR_MAX_SEGMENTS = 67,
                    RAID_ARROW_NAVAL_SUBDIVISIONS = 9, RAID_ARROW_NAVAL_SHARP_TURN_SMOOTHNESS = 0.27,
                    RAID_ARROW_NAVAL_USE_MIDPOINTS = false,
                    RAID_ARROW_LAND_SUBDIVISIONS = 7, RAID_ARROW_LAND_SHARP_TURN_SMOOTHNESS = 0.38,
                    RAID_ARROW_LAND_USE_MIDPOINTS = true,
                    RAID_UNIT_ENTITY_BASE_SCALE = 1.75, RAID_UNIT_ENTITY_OFFSET = { 2.25, -3.5, 4.75 } },
                  NRaids = { RAID_UNIT_SPEED_MULTIPLIER = 0.375 } }"#;

fn graphics_defines(replace: &[(&str, &str)]) -> io::Result<RaidGraphicsDefines> {
    let source = replace
        .iter()
        .fold(GRAPHICS_DEFINES.to_owned(), |source, (from, to)| {
            assert!(source.contains(from), "{from} is not in the fixture");
            source.replace(from, to)
        });
    let defines = crate::defines::Defines::new().unwrap();
    defines
        .load("common/defines/raid_fixture.lua", source.as_bytes())
        .unwrap();
    RaidGraphicsDefines::read(&defines)
}

#[test]
fn graphics_defines_keep_field_widths() {
    let read = graphics_defines(&[]).unwrap();
    assert_eq!(read.ready_color, [0.65, 0.55, 0.45, 0.35]);
    assert_eq!(read.air_source_length, 31.25);
    assert_eq!(read.ballistic_max_segments, 73);
    assert_eq!(
        read.land,
        PathArrowDefines {
            subdivisions: 7,
            sharp_turn_smoothness: 0.38,
            use_midpoints: true
        }
    );
    assert!(!read.naval.use_midpoints);
    assert_eq!(read.unit_offset, [2.25, -3.5, 4.75]);
    assert_eq!(read.speed_multiplier_raw, 37_500);
}

#[test]
fn segment_and_subdivision_counts_are_at_least_one() {
    let counts = |read: RaidGraphicsDefines| {
        [
            read.ballistic_max_segments,
            read.air_max_segments,
            read.naval.subdivisions,
            read.land.subdivisions,
        ]
    };
    let missing = graphics_defines(&[
        ("RAID_ARROW_BALLISTIC_MAX_SEGMENTS = 73,", ""),
        ("RAID_ARROW_AIR_MAX_SEGMENTS = 67,", ""),
        ("RAID_ARROW_NAVAL_SUBDIVISIONS = 9,", ""),
        ("RAID_ARROW_LAND_SUBDIVISIONS = 7,", ""),
    ]);
    assert_eq!(counts(missing.unwrap()), [1; 4]);
    let not_numbers = graphics_defines(&[
        (
            "RAID_ARROW_BALLISTIC_MAX_SEGMENTS = 73",
            "RAID_ARROW_BALLISTIC_MAX_SEGMENTS = \"73\"",
        ),
        (
            "RAID_ARROW_AIR_MAX_SEGMENTS = 67",
            "RAID_ARROW_AIR_MAX_SEGMENTS = true",
        ),
        (
            "RAID_ARROW_NAVAL_SUBDIVISIONS = 9",
            "RAID_ARROW_NAVAL_SUBDIVISIONS = { 9 }",
        ),
        (
            "RAID_ARROW_LAND_SUBDIVISIONS = 7",
            "RAID_ARROW_LAND_SUBDIVISIONS = \"7\"",
        ),
    ]);
    assert_eq!(counts(not_numbers.unwrap()), [1; 4]);
    let below_one = graphics_defines(&[
        (
            "RAID_ARROW_BALLISTIC_MAX_SEGMENTS = 73",
            "RAID_ARROW_BALLISTIC_MAX_SEGMENTS = 0.9",
        ),
        (
            "RAID_ARROW_AIR_MAX_SEGMENTS = 67",
            "RAID_ARROW_AIR_MAX_SEGMENTS = -4",
        ),
        (
            "RAID_ARROW_NAVAL_SUBDIVISIONS = 9",
            "RAID_ARROW_NAVAL_SUBDIVISIONS = 0",
        ),
        (
            "RAID_ARROW_LAND_SUBDIVISIONS = 7",
            "RAID_ARROW_LAND_SUBDIVISIONS = 2.5",
        ),
    ]);
    assert_eq!(counts(below_one.unwrap()), [1, 1, 1, 2]);
}

#[test]
fn unit_animation_stages_keep_authored_order_across_blocks() {
    content_reader_script::test_game::enter();
    let catalog = parse(
        &[],
        &[doc(br#"types = { militia = {
                    unit_animations = { stage = { state = "move" } }
                    unit_animations = {
                        stage = { state = "idle2" intro = "idle2_intro" at_progress = 0.33 }
                        stage = { at_progress = 0.66 state = idle3 }
                    }
                } }"#)
        .unwrap()],
        EQUIPMENT,
    )
    .unwrap();
    let stages = &catalog.get("militia").unwrap().unit_animations;
    assert_eq!(
        stages,
        &[
            RaidAnimationStage {
                state: "move".into(),
                intro: None,
                at_progress_raw: 0
            },
            RaidAnimationStage {
                state: "idle2".into(),
                intro: Some("idle2_intro".into()),
                at_progress_raw: 33_000
            },
            RaidAnimationStage {
                state: "idle3".into(),
                intro: None,
                at_progress_raw: 66_000
            },
        ]
    );
}

#[test]
fn a_repeated_category_is_the_later_declaration_alone() {
    content_reader_script::test_game::enter();
    let categories = doc(
        b"categories = { air = { free_targeting = yes visible = { always = yes } } }
              categories = { air = { available = { always = no } } }",
    )
    .unwrap();
    let catalog = parse(&[categories], &[], EQUIPMENT).unwrap();
    let air = &catalog.categories["air"];
    assert!(!air.free_targeting && air.visible.is_empty());
    assert_eq!(air.available.len(), 1);
}
