use super::budget_fixture::{budget_buildings, group, prepared};
use viewer_gpu::camera::MapDefines;
use viewer_gpu::camera::MapSize;
use viewer_gpu::camera::Placement;
use viewer_gpu::province_admission::edge_boxes;
#[test]
fn only_states_the_building_producer_cannot_select_are_refused() {
    let descriptors = content_reader_mesh::mesh::parse_descriptors(br#"
            entity = { name = ruin state = { name = destroyed event = { particle = smoke light = alarm } } }
            entity = { name = default_ruin clone = ruin default_state = destroyed }
            entity = { name = linked_ruin clone = ruin state = { name = idle next_state = destroyed } }
        "#, "fixture.asset").unwrap();
    assert_eq!(
        super::unselected_meshless_states(&descriptors, "ruin"),
        [
            "entity ruin state destroyed: 2 map effects refused; state is not selected by the building producer"
        ]
    );
    assert!(super::unselected_meshless_states(&descriptors, "default_ruin").is_empty());
    assert!(super::unselected_meshless_states(&descriptors, "linked_ruin").is_empty());
}

use super::*;
use content_reader_key_names::buildings;
use replay_data::{
    rows::BuildingRow, rows::CountryLinkRow, rows::LexicalTokenRow, rows::LocalPlayerRow,
    rows::RaidScope, rows::RaidTargetStatusRow, rows::TableId, rows::TableRows,
};
use replay_frame::{frame::DecodedFrame, frame::Metrics, frame::TableView};
use viewer_foundation::perspective::{Choice, Country, Vision};

const SAMPLE: &[u8] = br#"
buildings = {
    arms_factory = { show_on_map = 6 level_cap = { state_max = 20 } }
    anti_air_building = { show_on_map = 3 level_cap = { state_max = 5 } }
    bunker = { show_on_map = 1 level_cap = { province_max = 10 } }
    supply_node = { show_on_map = 1 always_shown = yes level_cap = { province_max = 1 } }
    infrastructure = { level_cap = { state_max = 5 } }
    uncapped = { show_on_map = 4 }
}
"#;

fn types() -> Buildings {
    buildings::parse(SAMPLE, &content_reader_script::key_names::KeyNames::default()).unwrap()
}

fn defines() -> Defines {
    Defines {
        shrink_distance: 180.0,
        destroyed: 6_000_000,
        recovered: 8_000_000,
        damaged: 80_000,
        max_levels: 15,
        popup_speed: 5.0,
    }
}

fn no_city() -> CityTerrain<'static> {
    CityTerrain {
        width: 0,
        height: 0,
        spawn_city: &[],
    }
}

fn camera(center: [f32; 2], height: f32) -> Camera {
    Placement {
        center,
        height,
        pose: viewer_gpu::camera::Pose::Map,
    }
    .on(MapSize::TEST, MapDefines::test())
}

fn at(history: &mut BuildingHistory, height: f32, time: f32) -> Vec<Placed> {
    let _ = history.update(camera([0.0, 0.0], height), [1024, 768], time, true);
    history.collect(None, 0.0)
}

fn cold_instances(prepared: PreparedBuildings) -> Vec<Placed> {
    let mut history = BuildingHistory::default();
    history.set(prepared, false);
    at(&mut history, 50.0, 0.0)
}

#[test]
fn installed_building_snapshots_share_immutable_slots() {
    let input = prepared(10, 10_000_000, false);
    let mut history = BuildingHistory::default();
    history.set(input.clone(), false);
    assert!(Arc::ptr_eq(
        &input.city_damage,
        &history.prepared.as_ref().unwrap().city_damage
    ));
    assert!(
        input
            .slots
            .ptr_eq(&history.prepared.as_ref().unwrap().slots)
    );
    let mut next = input.clone();
    next.hour += 1;
    next.slots.edit(0, |slot| slot.entity = None);
    assert!(input.slots[0].entity.is_some());
    assert!(history.prepared.as_ref().unwrap().slots[0].entity.is_some());
}

#[test]
fn destruction_hysteresis_survives_hide_and_resets_on_seek() {
    let mut history = BuildingHistory::default();
    history.set(prepared(10, 6_000_000, false), true);
    assert_eq!(at(&mut history, 50.0, 0.0)[0].0, 2);
    let mut hidden = prepared(11, 9_000_000, false);
    hidden.slots.edit(0, |slot| slot.entity = None);
    assert!(!history.set(hidden, true));
    assert!(at(&mut history, 50.0, 1.0).is_empty());
    history.set(prepared(12, 7_000_000, false), true);
    assert_eq!(
        at(&mut history, 50.0, 2.0)[0].0,
        2,
        "still destroyed in the band"
    );
    history.set(prepared(13, 8_000_001, false), true);
    assert_eq!(at(&mut history, 50.0, 3.0)[0].0, 1);
    assert!(
        history.set(prepared(20, 7_000_000, false), false),
        "a seek starts cold"
    );
    assert_eq!(at(&mut history, 50.0, 4.0)[0].0, 1);
}

#[test]
fn a_forward_skip_advances_through_the_skipped_hours() {
    let mut history = BuildingHistory::default();
    history.set(prepared(10, 10_000_000, false), true);
    assert_eq!(at(&mut history, 50.0, 0.0)[0].0, 1);
    history.advance(&prepared(11, 0, false));
    assert!(!history.set(prepared(14, 7_000_000, false), true));
    assert_eq!(
        at(&mut history, 50.0, 1.0)[0].0,
        2,
        "the skipped hour latched the destroyed mesh"
    );
    history.advance(&prepared(3, 10_000_000, false));
    assert_eq!(
        at(&mut history, 50.0, 2.0)[0].0,
        2,
        "a past hour is ignored"
    );
    assert!(
        history.set(prepared(13, 7_000_000, false), true),
        "backwards is cold"
    );
}

#[test]
fn a_landmark_keeps_its_model_above_the_cutoff_only_once_created_below() {
    let mut history = BuildingHistory::default();
    history.set(prepared(10, 10_000_000, true), true);
    assert!(at(&mut history, 500.0, 0.0).is_empty());
    assert_eq!(at(&mut history, 50.0, 1.0)[0].0, 1);
    assert_eq!(at(&mut history, 1500.0, 2.0)[0].0, 1);
    history.set(prepared(11, 0, true), true);
    let far = at(&mut history, 1500.0, 3.0);
    assert_eq!(far[0].0, 1, "far away the landmark is not rebuilt");
    assert_eq!(far[0].1.animation, BUILDING_IDLE);
    assert_eq!(far[0].1.scale, 1.0, "a landmark takes no shrink step");
    assert_eq!(at(&mut history, 50.0, 4.0)[0].0, 2);
}

#[test]
fn an_ordinary_building_stays_drawn_far_after_one_shrink_step() {
    let mut history = BuildingHistory::default();
    history.set(prepared(10, 10_000_000, false), true);
    assert!(at(&mut history, 500.0, 0.0).is_empty());
    assert_eq!(at(&mut history, 50.0, 1.0)[0].1.scale, 1.0);
    let far = at(&mut history, 500.0, 1.02);
    assert!((far[0].1.scale - 0.9).abs() < 1e-4, "one frame's step");
    let still = at(&mut history, 1500.0, 2.0);
    assert!(
        (still[0].1.scale - 0.9).abs() < 1e-4,
        "only once per transition"
    );
    history.set(prepared(11, 0, false), true);
    assert_eq!(
        at(&mut history, 1400.0, 3.0)[0].0,
        1,
        "far away nothing is updated"
    );
    assert_eq!(at(&mut history, 50.0, 4.0)[0].0, 2);
    assert!(
        at(&mut history, 500.0, 5.0).is_empty(),
        "a step to zero scale removes it"
    );
}

#[test]
fn a_visited_view_reaches_the_next_stream() {
    let mut history = BuildingHistory::default();
    history.set(prepared(10, 10_000_000, false), false);
    history.visit(camera([0.0, 0.0], 150.0), [1024, 768]);
    let placed = history.update(camera([0.0, 0.0], 500.0), [1024, 768], 0.0, true);
    assert_eq!(placed.map(|placed| placed.len()), Some(1));
}

#[test]
fn a_damaged_building_goes_idle_with_its_far_step() {
    let mut history = BuildingHistory::default();
    history.set(prepared(10, 7_500_000, false), true);
    assert_eq!(at(&mut history, 50.0, 0.0)[0].1.animation, BUILDING_DAMAGED);
    let far = at(&mut history, 500.0, 0.02);
    assert_eq!(
        far[0].1.animation, BUILDING_IDLE,
        "no smoke or fire far away"
    );
    history.set(prepared(11, 7_500_000, true), true);
    at(&mut history, 50.0, 1.0);
    let landmark = at(&mut history, 500.0, 1.02);
    assert_eq!(
        landmark[0].1.animation, BUILDING_DAMAGED,
        "a landmark takes no step"
    );
}

#[test]
fn a_branch_or_a_backwards_clock_starts_cold() {
    let mut history = BuildingHistory::default();
    history.set(prepared(10, 0, true), true);
    at(&mut history, 50.0, 1.0);
    let mut changed = prepared(11, 7_000_000, true);
    changed.branch = 2;
    assert!(history.set(changed, true));
    assert_eq!(at(&mut history, 50.0, 2.0)[0].0, 1);
    history.set(prepared(12, 0, true), true);
    at(&mut history, 50.0, 3.0);
    assert!(at(&mut history, 500.0, 0.0).is_empty());
}

#[test]
fn only_provinces_in_view_are_created_and_recreation_restarts_the_clock() {
    let mut input = prepared(10, 10_000_000, true);
    let mut other = input.slots[0].clone();
    other.id = 1;
    other.entity_mut().instance.position = [2001.0, 0.0, 1001.0];
    input.slots = input.slots.iter().cloned().chain([other]).collect();
    input.groups = Arc::from([
        group([0.0, 0.0, 2.0, 2.0], vec![0]),
        group([2000.0, 1000.0, 2002.0, 1002.0], vec![1]),
    ]);
    let mut history = BuildingHistory::default();
    history.set(input, true);
    assert_eq!(at(&mut history, 50.0, 0.0).len(), 1);
    let there = camera([2000.0, 1000.0], 500.0);
    let _ = history.update(there, [1024, 768], 1.0, true);
    assert_eq!(
        history.collect(None, 0.0).len(),
        1,
        "a far view cannot create an unvisited landmark"
    );
    let close = camera([2000.0, 1000.0], 50.0);
    assert_eq!(
        history.update(close, [1024, 768], 2.0, true).unwrap().len(),
        1,
        "culled to the view"
    );
    assert_eq!(history.collect(None, 0.0).len(), 2);

    let mut history = BuildingHistory::default();
    let mut same_mesh = prepared(10, 0, false);
    same_mesh.slots.edit(0, |slot| slot.entity_mut().ruined.mesh = 1);
    history.set(same_mesh, true);
    let before = at(&mut history, 50.0, 0.0)[0].1.animation_epoch;
    history.set(prepared(11, 10_000_000, false), true);
    let after = at(&mut history, 50.0, 1.0);
    assert_eq!(after[0].0, 1);
    assert_ne!(after[0].1.animation_epoch, before);
}

#[test]
fn buildings_just_past_the_view_edge_are_created() {
    let mut input = prepared(10, 10_000_000, true);
    let boxes = edge_boxes();
    assert_eq!(boxes[&5], [3044.0, 1000.0, 3046.0, 1002.0]);
    let slot = input.slots[0].clone();
    input.slots = boxes
        .keys()
        .map(|&id| {
            let mut slot = slot.clone();
            slot.id = id - 1;
            slot.entity_mut().instance.position = [id as f32, 0.0, 1000.0];
            slot
        })
        .collect();
    input.groups = boxes
        .iter()
        .map(|(&id, &bounds)| {
            let mut group = group(bounds, vec![id - 1]);
            group.province = id;
            group
        })
        .collect();
    let mut history = BuildingHistory::default();
    history.set(input, true);
    let _ = history.update(camera([3000.0, 1000.0], 50.0), [1024, 768], 0.0, true);
    let created: Vec<f32> = history
        .collect(None, 0.0)
        .iter()
        .map(|(_, instance)| instance.position[0])
        .collect();
    assert_eq!(created, [1.0, 2.0, 5.0, 6.0]);
}

#[test]
fn the_stream_is_rebuilt_only_when_an_entity_or_the_collected_view_changes() {
    let mut history = BuildingHistory::default();
    history.set(prepared(10, 10_000_000, false), true);
    let view = |x: f32| camera([x, 0.0], 50.0);
    assert!(history.update(view(0.0), [1024, 768], 0.0, true).is_some());
    assert!(
        history.update(view(0.0), [1024, 768], 0.1, true).is_none(),
        "still"
    );
    assert!(
        history.update(view(1.0), [1024, 768], 0.2, true).is_none(),
        "a small pan"
    );
    history.set(prepared(11, 10_000_000, false), true);
    assert!(
        history.update(view(1.0), [1024, 768], 0.3, true).is_none(),
        "an hour that changes nothing in view"
    );
    history.set(prepared(12, 0, false), true);
    assert!(history.update(view(1.0), [1024, 768], 0.4, true).is_some());
    assert!(
        history
            .update(view(1500.0), [1024, 768], 0.5, true)
            .is_some(),
        "left the rectangle"
    );
    assert!(
        history
            .update(view(1500.0), [1024, 768], 0.6, false)
            .is_none(),
        "disabled"
    );
}

#[test]
fn a_hub_slot_is_left_to_another_layer_on_a_naval_base_or_city_terrain() {
    let ports = BTreeSet::from([7u32]);
    assert_eq!(
        hub_left_to_another_layer(Some(7), &ports, true, false),
        Some(true)
    );
    assert_eq!(
        hub_left_to_another_layer(Some(8), &ports, true, false),
        Some(false)
    );
    assert_eq!(
        hub_left_to_another_layer(Some(8), &ports, false, false),
        None,
        "an incomplete hash cannot prove a missing port"
    );
    assert_eq!(
        hub_left_to_another_layer(Some(8), &ports, false, true),
        Some(true)
    );
    assert_eq!(
        hub_left_to_another_layer(None, &ports, false, true),
        Some(true)
    );
    assert_eq!(
        hub_left_to_another_layer(None, &ports, false, false),
        Some(false)
    );
}

const ONE_STATE: u32 = 7;
const ONE_PROVINCE: u32 = 9;
const ONE_TOKEN: u32 = 77;

fn one_slot_assets(
    types: &Buildings,
    entities: BTreeMap<(String, usize), usize>,
) -> MapBuildingAssets {
    let slots = BTreeMap::from([(
        (ONE_STATE, "arms_factory".to_owned()),
        vec![(
            BuildingPosition {
                state: ONE_STATE,
                kind: "arms_factory".to_owned(),
                x: 1.0,
                y: 0.0,
                z: 1.0,
                rotation: 0.0,
                adjacent_sea: 0,
            },
            Some(ONE_PROVINCE),
        )],
    )]);
    let groups = slot_groups(
        slots.values().flatten().map(|(_, province)| *province),
        &BTreeMap::from([(ONE_PROVINCE, [0.0, 0.0, 2.0, 2.0])]),
    );
    MapBuildingAssets::assemble(
        ModelLibrary::default(),
        types,
        defines(),
        groups,
        slots,
        entities,
        BTreeMap::new(),
    )
}

fn one_row_frame(row: BuildingRow, byte: Option<u8>) -> (DecodedFrame, Vision) {
    let mut views = std::array::from_fn(|_| TableView::Unavailable {
        reason: "not needed for one slot".into(),
    });
    views[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 100,
            canonical_slot: 0,
        }]));
    views[TableId::LocalPlayer.index()] =
        TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow {
            country: 100,
            ..Default::default()
        }]));
    views[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: ONE_TOKEN,
            literal: b"arms_factory".to_vec(),
        }]));
    views[TableId::StateBuildings.index()] =
        TableView::Complete(TableRows::StateBuildings(vec![BuildingRow {
            location: ONE_STATE,
            building_type: ONE_TOKEN,
            country: 100,
            ..row
        }]));
    let frame = DecodedFrame {
        branch: 1,
        hour: 60_832_633,
        tables: views.map(Into::into),
        metrics: Metrics::default(),
    };
    let mut vision = Vision::read(
        &frame,
        &BTreeMap::from([(100, 0)]),
        &BTreeMap::from([(0, "GER".to_owned())]),
        &Choice {
            country: Country::Recorded,
            fog: true,
        },
        Err("not needed for one slot"),
    )
    .unwrap();
    vision.set_visibility(byte.map(|byte| (ONE_PROVINCE, byte)));
    (frame, vision)
}

#[test]
fn a_ruin_with_its_own_mesh_keeps_variant_selector_zero_across_levels() {
    content_reader_script::test_game::enter();
    let types = buildings::parse(
            b"buildings={ arms_factory={ show_on_map=1 show_on_map_meshes=2 has_destroyed_mesh=yes level_cap={ state_max=20 } } }", &content_reader_script::key_names::KeyNames::default(),
        )
        .unwrap();
    let assets = one_slot_assets(
        &types,
        BTreeMap::from([
            (("arms_factory".to_owned(), 0), 0),
            (("arms_factory".to_owned(), 1), 1),
            (("arms_factory".to_owned(), usize::MAX), 2),
        ]),
    );
    let entity = |level| {
        let (frame, vision) = one_row_frame(
            BuildingRow {
                level,
                ..Default::default()
            },
            Some(50),
        );
        let (prepared, _) = assets.field(&frame, &vision, &no_city()).unwrap();
        prepared.slots[0].entity.clone().unwrap()
    };
    let (low, high) = (entity(2), entity(14));
    assert!(low.destroyed && high.destroyed);
    assert_eq!((low.healthy.variant, high.healthy.variant), (0, 1));
    assert_eq!(low.ruined, high.ruined);
    assert_eq!((low.ruined.variant, low.ruined.definition), (0, 2));
}

#[test]
fn the_fog_loop_hides_below_fifty_and_keeps_the_undecided() {
    content_reader_script::test_game::enter();
    let assets = one_slot_assets(
        &types(),
        BTreeMap::from([(("arms_factory".to_owned(), 0), 0)]),
    );
    let frame_with = |byte| {
        one_row_frame(
            BuildingRow {
                level: 1,
                healthy_level: 1,
                ..Default::default()
            },
            byte,
        )
    };
    for (byte, fogged, drawn) in [(Some(49), 1, 0), (Some(50), 0, 1), (None, 0, 1)] {
        let (frame, vision) = frame_with(byte);
        let (field, summary) = assets.field(&frame, &vision, &no_city()).unwrap();
        assert_eq!(summary.fogged, fogged, "{byte:?}");
        assert_eq!(cold_instances(field).len(), drawn, "{byte:?}");
    }
    let (mut frame, vision) = frame_with(Some(50));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: ONE_TOKEN,
            literal: b"not_a_building".to_vec(),
        }]));
    let (field, summary) = assets.field(&frame, &vision, &no_city()).unwrap();
    assert_eq!(summary.built, 0);
    assert_eq!(
        summary.refused_tokens,
        BTreeMap::from([(
            format!("token {ONE_TOKEN} (not_a_building) names no authored building type"),
            1
        )])
    );
    assert!(cold_instances(field).is_empty());
}

#[test]
fn the_level_uses_the_content_denominator() {
    content_reader_script::test_game::enter();
    let types = types();
    let factory = &types.types["arms_factory"];
    for (level, slots) in [(0, 0), (-3, 0), (1, 1), (4, 2), (10, 4), (20, 6), (99, 6)] {
        assert_eq!(slots_for_level(factory, level, 15), slots, "{level}");
    }
    assert_eq!(slots_for_level(factory, 4, 10), 3);
    assert_eq!(slots_for_level(&types.types["anti_air_building"], 5, 15), 1);
    assert_eq!(slots_for_level(&types.types["infrastructure"], 5, 15), 0);
    assert_eq!(slots_for_level(&types.types["uncapped"], 9, 15), 3);
}

#[test]
fn foreign_detection_needs_the_viewers_bit() {
    let mut views = std::array::from_fn(|_| TableView::Unavailable {
        reason: "unused".into(),
    });
    views[TableId::RaidTargetStatuses.index()] =
        TableView::Complete(TableRows::RaidTargetStatuses(vec![
            RaidTargetStatusRow {
                ordinal: 0,
                valid: 1,
                scope: RaidScope {
                    building_type: 5,
                    building_location: 9,
                    building_flags: 1,
                    building_country: 101,
                    ..Default::default()
                },
                detected: vec![0b100],
            },
            RaidTargetStatusRow {
                ordinal: 1,
                valid: 1,
                scope: RaidScope {
                    province: 9,
                    ..Default::default()
                },
                detected: vec![0xff],
            },
        ]));
    let frame = DecodedFrame {
        branch: 1,
        hour: 0,
        tables: views.map(Into::into),
        metrics: Metrics::default(),
    };
    let links = BTreeMap::from([(100, 0), (101, 1), (102, 2)]);
    let row = BuildingRow {
        building_type: 5,
        location: 9,
        status_flags: 1,
        country: 101,
        ..Default::default()
    };
    let detection = |ours| Detection::read(&frame, &links, ours).unwrap();
    assert_eq!(
        detection(1).detected(&available::BuildingInput::from(&row)),
        Some(true),
        "our own building"
    );
    assert_eq!(
        detection(0).detected(&available::BuildingInput::from(&row)),
        Some(false)
    );
    assert_eq!(
        detection(2).detected(&available::BuildingInput::from(&row)),
        Some(true)
    );
    let mut incomplete = detection(0);
    incomplete.targets = None;
    assert_eq!(
        incomplete.detected(&available::BuildingInput::from(&row)),
        None
    );
}

#[test]
fn tokens_name_building_types_under_the_lexers_fold() {
    let token = |token, literal: &[u8]| LexicalTokenRow {
        token,
        literal: literal.to_vec(),
    };
    let mut views: [TableView; TableId::ALL.len()] =
        std::array::from_fn(|index| TableView::Complete(TableRows::empty(TableId::ALL[index])));
    views[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![
        token(0, b"air_base"),
        token(3, b"Air_Base"),
        token(4, b"bunker"),
        token(5, b"coastal_bunker"),
    ]));
    let frame = DecodedFrame {
        branch: 1,
        hour: 0,
        tables: views.map(Into::into),
        metrics: Metrics::default(),
    };
    let building = |name: &str| BuildingType {
        name: name.to_owned(),
        ..Default::default()
    };
    let assets = assets_of(vec![
        building("air_base"),
        building("BUNKER"),
        building("Bunker"),
    ]);
    let types = token_types(&frame, &assets.facts.keys, &assets.facts.index).unwrap();
    assert!(types.get(&0).is_none());
    assert_eq!(types.get(&3), Some(&Ok(0)));
    assert_eq!(
        types.get(&4),
        Some(&Err(
            "token bunker names several content keys: BUNKER, Bunker".to_owned()
        ))
    );
    assert!(types.get(&5).is_none());
}

fn assets_of(ordered: Vec<BuildingType>) -> MapBuildingAssets {
    let types = Buildings {
        declaration_order: ordered.iter().map(|b| b.name.clone()).collect(),
        types: ordered.into_iter().map(|b| (b.name.clone(), b)).collect(),
        ..Buildings::default()
    };
    MapBuildingAssets::assemble(
        ModelLibrary::default(),
        &types,
        defines(),
        slot_groups(std::iter::empty(), &BTreeMap::new()),
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
    )
}

#[test]
fn province_sites_are_indexed_and_ambiguous_facility_tokens_are_refused() {
    content_reader_script::test_game::enter();
    let mut types = types();
    let factory = types.types.get_mut("arms_factory").unwrap();
    factory.air_base = true;
    factory.rocket_launch_capacity = 1;
    factory.gun_emplacement = true;
    let assets = one_slot_assets(&types, BTreeMap::new());
    assert_eq!(
        assets.facts.province_sites(ONE_STATE, ONE_PROVINCE),
        [true; 3]
    );
    assert_eq!(
        assets.facts.province_sites(ONE_STATE, ONE_PROVINCE + 1),
        [false; 3]
    );

    let assets = assets_of(vec![
        BuildingType {
            name: "Facility".into(),
            specialization: vec!["science".into()],
            ..Default::default()
        },
        BuildingType {
            name: "facility".into(),
            ..Default::default()
        },
    ]);
    assert!(
        assets
            .facts
            .is_facility("Facility")
            .unwrap_err()
            .contains("names several content keys")
    );
    let assets = assets_of(vec![BuildingType {
        name: "Facility".into(),
        specialization: vec!["science".into()],
        ..Default::default()
    }]);
    assert_eq!(assets.facts.is_facility("FACILITY"), Ok(true));
}

fn buildings_frame(tokens: TableView, types: &[u32]) -> (DecodedFrame, Vision) {
    let mut views = std::array::from_fn(|_| TableView::Unavailable {
        reason: "not needed for the token rows".into(),
    });
    views[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 100,
            canonical_slot: 0,
        }]));
    views[TableId::LocalPlayer.index()] =
        TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow {
            country: 100,
            ..Default::default()
        }]));
    views[TableId::LexicalTokens.index()] = tokens;
    views[TableId::StateBuildings.index()] = TableView::Complete(TableRows::StateBuildings(
        types
            .iter()
            .enumerate()
            .map(|(location, &building_type)| BuildingRow {
                location: location as u32 + 1,
                building_type,
                level: 1,
                healthy_level: 1,
                country: 100,
                ..Default::default()
            })
            .collect(),
    ));
    let frame = DecodedFrame {
        branch: 1,
        hour: 60_832_633,
        tables: views.map(Into::into),
        metrics: Metrics::default(),
    };
    let vision = Vision::read(
        &frame,
        &BTreeMap::from([(100, 0)]),
        &BTreeMap::from([(0, "GER".to_owned())]),
        &Choice {
            country: Country::Recorded,
            fog: false,
        },
        Err("not needed for the token rows"),
    )
    .unwrap();
    (frame, vision)
}

#[test]
fn building_rows_refuse_their_unnamed_type_tokens_by_reason() {
    content_reader_script::test_game::enter();
    let assets = MapBuildingAssets::assemble(
        ModelLibrary::default(),
        &types(),
        defines(),
        slot_groups(std::iter::empty(), &BTreeMap::new()),
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
    );
    let token = |token, literal: &[u8]| LexicalTokenRow {
        token,
        literal: literal.to_vec(),
    };
    let rows = TableRows::LexicalTokens(vec![
        token(77, b"Arms_Factory"),
        token(79, &[0xff]),
        token(80, b"not_a_building"),
    ]);
    let (frame, vision) =
        buildings_frame(TableView::Complete(rows.clone()), &[0, 77, 78, 79, 80, 80]);
    let (_, summary) = assets.field(&frame, &vision, &no_city()).unwrap();
    assert_eq!(summary.built, 1, "the folded spelling names arms_factory");
    assert_eq!(summary.unnamed_tokens, 2);
    assert_eq!(
        summary.refused_tokens,
        BTreeMap::from([
            ("token 78 is not in lexical tokens".to_owned(), 1),
            ("token 79 is not spelled as text".to_owned(), 1),
            (
                "token 80 (not_a_building) names no authored building type".to_owned(),
                2
            ),
        ])
    );
    assert_eq!(
        summary.warnings()[0],
        "Map buildings: 1 rows not drawn, refused type token: token 78 is not in lexical tokens."
    );
    let (frame, vision) = buildings_frame(TableView::Truncated { rows, missing: 4 }, &[0, 78]);
    let (_, summary) = assets.field(&frame, &vision, &no_city()).unwrap();
    assert_eq!(summary.unnamed_tokens, 1);
    assert_eq!(
        summary.refused_tokens,
        BTreeMap::from([(
            "token 78 is among lexical tokens' missing rows".to_owned(),
            1
        )])
    );
    let (mut frame, vision) = buildings_frame(
        TableView::Complete(TableRows::LexicalTokens(Vec::new())),
        &[0],
    );
    *frame.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(Vec::new()));
    *frame.tables[TableId::ProvinceBuildings.index()] =
        TableView::Complete(TableRows::ProvinceBuildings(Vec::new()));
    let (_, summary) = assets.field(&frame, &vision, &no_city()).unwrap();
    assert_eq!(
        (summary.built, summary.unnamed_tokens),
        (0, 0),
        "type 0 is none"
    );
    assert!(summary.refused_tokens.is_empty());
    assert!(summary.warnings().is_empty(), "{:?}", summary.warnings());
}

#[test]
fn health_thresholds_come_from_the_content() {
    let defines = defines();
    assert_eq!(health_state(1, 0, 6_000_000, &defines), (true, true, false));
    assert_eq!(health_state(1, 0, 6_000_001, &defines), (false, true, true));
    assert_eq!(
        health_state(1, 0, 8_000_000, &defines),
        (false, false, true)
    );
    assert_eq!(
        health_state(1, 0, 8_000_001, &defines),
        (false, false, false)
    );
    assert_eq!(health_state(5, 1, 0, &defines), (false, true, false));
    assert_eq!(health_state(5, 5, 0, &defines), (false, false, false));
    let changed = Defines {
        destroyed: 5_000_000,
        recovered: 7_000_000,
        damaged: 75_000,
        ..defines
    };
    assert_eq!(health_state(1, 0, 7_000_000, &changed), (false, true, true));
    assert_eq!(
        health_state(1, 0, 7_500_000, &changed),
        (false, false, false)
    );
}

#[test]
fn health_state_preserves_native_wrapping_and_exceptional_words() {
    let defines = defines();
    assert_eq!(
        health_state(1, 0, i64::MAX - 3, &defines),
        (false, true, false)
    );
    assert_eq!(health_state(2, 1, i64::MAX, &defines), (true, false, false));
    let sentinel = Defines {
        damaged: 0xffff_ffff + 1,
        ..defines
    };
    for word in [i64::MIN, i64::MIN + 1, i64::MAX] {
        assert!(health_state(1, 0, word, &sentinel).1);
        let at_threshold = Defines {
            damaged: 0xffff_ffff,
            ..sentinel
        };
        assert!(!health_state(1, 0, word, &at_threshold).1);
    }
    assert!(!health_state(0, 0, 0, &sentinel).1);
}

#[test]
fn slots_are_looked_up_by_their_truncated_position() {
    let ids = [1, 1, 2, 2, 1, 1, 2, 0, 3, 3, 3, 3];
    let map = ProvinceMap {
        width: 4,
        height: 3,
        ids: &ids,
    };
    assert_eq!(map.at(0.0, 2.0), Some(1));
    assert_eq!(
        map.at(3.0, 1.0),
        None,
        "an unclaimed pixel names no province"
    );
    assert_eq!(map.at(-1.0, 1.0), None);
    assert_eq!(map.at(-0.25, 1.0), Some(1), "truncation before the bounds");
    assert_eq!(map.at(4.0, 1.0), None);
    assert_eq!(map.at(0.0, -0.25), Some(3));
    assert_eq!(map.at(1.6, 1.6), Some(1));
    assert_eq!(map.at(2.4, 1.6), Some(2));
    let city = CityTerrain {
        width: 2,
        height: 1,
        spawn_city: &[true, false],
    };
    assert!(city.at(-0.25, -0.25));
    assert!(!city.at(2.0, 0.0));
    assert!(!city.at(f32::NAN, 0.0));
}

#[test]
fn the_update_cutoff_is_water_relative() {
    assert!(!draws_map_buildings(302.960, 180.0));
    assert!(draws_map_buildings(115.293, 180.0));
    assert!(draws_map_buildings(189.4, 180.0));
    assert!(!draws_map_buildings(189.6, 180.0));
}

#[test]
fn a_floating_harbour_takes_the_first_record_of_its_controllers_system() {
    let supply = ProvinceSupplyRow {
        province: 9,
        controller_index: 2,
        flags: 1,
        ..Default::default()
    };
    let record = |country, ordinal, status| FloatingHarborDeploymentRow {
        country,
        ordinal,
        province: 9,
        status,
    };
    let deployments = [record(2, 5, 1), record(2, 1, 0), record(3, 0, 1)];
    assert_eq!(
        floating_harbor_deployed(9, Some(&supply), true, &deployments, true),
        Some(false),
        "the first record wins over a later active one"
    );
    assert_eq!(
        floating_harbor_deployed(9, Some(&supply), true, &deployments[..1], true),
        Some(true)
    );
    assert_eq!(
        floating_harbor_deployed(9, Some(&supply), true, &deployments, false),
        None
    );
    for (flags, controller_index) in [(0, 2), (1, 0)] {
        let inactive = ProvinceSupplyRow {
            flags,
            controller_index,
            ..supply.clone()
        };
        assert_eq!(
            floating_harbor_deployed(9, Some(&inactive), true, &[], false),
            Some(false)
        );
    }
    assert_eq!(floating_harbor_deployed(9, None, false, &[], true), None);
    assert_eq!(
        floating_harbor_deployed(9, None, true, &[], true),
        Some(false)
    );
}

#[test]
fn settled_groups_match_full_reapplication_through_camera_hour_and_budget_changes() {
    let (mut cached, mut reference) = (
        budget_buildings(&[(1, 2), (2, 3)]),
        budget_buildings(&[(1, 2), (2, 3)]),
    );
    for frame in 0..160 {
        let time = frame as f32 / 60.;
        if frame == 70 {
            for history in [&mut cached, &mut reference] {
                let mut next = history.prepared.as_ref().unwrap().clone();
                next.hour += 1;
                next.slots.edit(1, |slot| slot.entity_mut().damaged = true);
                next.slots.edit(3, |slot| slot.entity = None);
                history.set(next, true);
            }
        }
        let camera = camera(
            [frame as f32 * 0.01, 0.],
            if (90..110).contains(&frame) {
                200.
            } else {
                50.
            },
        );
        let mut left = viewer_scene_base::mesh_budget::Budget::new(if frame < 4 { 1 } else { 7 }, false);
        let mut right =
            viewer_scene_base::mesh_budget::Budget::new(if frame < 4 { 1 } else { 7 }, false);
        reference.settled.fill(false);
        cached
            .frame(camera, [1024, 768], time, true, false)
            .finish(&mut left);
        reference
            .frame(camera, [1024, 768], time, true, false)
            .finish(&mut right);
        assert_eq!(
            cached.collect(None, 0.),
            reference.collect(None, 0.),
            "frame {frame}"
        );
        assert_eq!(left.remaining(), right.remaining(), "frame {frame}");
        assert_eq!(cached.near, reference.near, "frame {frame}");
    }
    assert!(cached.settled.iter().all(|settled| *settled));
    for history in [&mut cached, &mut reference] {
        let mut next = history.prepared.as_ref().unwrap().clone();
        next.hour += 1;
        next.popup_speed = -1.;
        history.set(next, true);
    }
    for frame in 160..164 {
        let camera = camera([frame as f32 * 0.01, 0.], 50.);
        let time = (frame - 1) as f32 / 60.;
        reference.settled.fill(false);
        for history in [&mut cached, &mut reference] {
            history
                .frame(camera, [1024, 768], time, true, false)
                .finish(&mut viewer_scene_base::mesh_budget::Budget::new(7, false));
        }
        assert_eq!(cached.collect(None, 0.), reference.collect(None, 0.));
    }
}
fn repeated_slot_assets(types: &Buildings) -> MapBuildingAssets {
    let mut assets = one_slot_assets(types, BTreeMap::new());
    let slots = assets
        .slots
        .get_mut(&(ONE_STATE, "arms_factory".to_owned()))
        .unwrap();
    slots.resize(8, slots[0].clone());
    assets.groups = slot_groups(
        assets
            .slots
            .values()
            .flatten()
            .map(|(_, province)| *province),
        &BTreeMap::from([(ONE_PROVINCE, [0.0, 0.0, 2.0, 2.0])]),
    );
    assets
}

#[test]
fn missing_building_meshes_are_counted_per_reached_slot() {
    content_reader_script::test_game::enter();
    let mut types = types();
    types
        .types
        .get_mut("arms_factory")
        .unwrap()
        .show_on_map_meshes = 2;
    let assets = repeated_slot_assets(&types);
    for (step, (byte, unresolved, fogged)) in [
        (Some(50), 8, 0),
        (Some(49), 0, 8),
        (None, 8, 0),
        (Some(50), 8, 0),
    ]
    .into_iter()
    .enumerate()
    {
        let (mut frame, vision) = one_row_frame(
            BuildingRow {
                level: 15,
                healthy_level: 15,
                ..Default::default()
            },
            byte,
        );
        frame.hour += step as i32;
        let (field, summary) = assets.field(&frame, &vision, &no_city()).unwrap();
        assert_eq!((summary.slots, summary.built, summary.placed), (8, 1, 0));
        assert_eq!(summary.fogged, fogged);
        assert_eq!(
            summary.unresolved_by_type,
            if unresolved == 0 {
                BTreeMap::new()
            } else {
                BTreeMap::from([("arms_factory".to_owned(), unresolved)])
            },
        );
        assert!(summary.refused_tokens.is_empty());
        assert!(summary.missing_by_type.is_empty());
        assert_eq!(field.slots.len(), 8);
        for (id, slot) in field.slots.iter().enumerate() {
            assert_eq!(slot.id, id as u32);
            assert_eq!(slot.drawn_at_distance, Some(false));
            assert!(slot.entity.is_none());
        }
    }
}

#[test]
fn damaged_only_building_meshes_supply_definition_fallback_per_slot() {
    content_reader_script::test_game::enter();
    let mut types = types();
    let building = types.types.get_mut("arms_factory").unwrap();
    building.show_on_map_meshes = 3;
    building.has_destroyed_mesh = true;
    let mut assets = repeated_slot_assets(&types);
    let index = assets.facts.index["arms_factory"];
    assets.damaged_entities.insert((index, 1), 21);
    assets.damaged_entities.insert((index, usize::MAX), 23);
    let (frame, vision) = one_row_frame(
        BuildingRow {
            level: 7,
            healthy_level: 1,
            ..Default::default()
        },
        Some(50),
    );
    let (field, summary) = assets.field(&frame, &vision, &no_city()).unwrap();
    assert_eq!((summary.slots, summary.built, summary.placed), (8, 1, 8));
    assert_eq!(
        (summary.damaged, summary.destroyed, summary.uncertain_damage),
        (1, 0, 0)
    );
    assert!(summary.unresolved_by_type.is_empty());
    assert_eq!(field.slots.len(), 8);
    for (id, slot) in field.slots.iter().enumerate() {
        let entity = slot.entity.as_ref().unwrap();
        assert_eq!(
            entity.healthy,
            Selection {
                admitted: true,
                mesh: 21,
                definition: 21,
                variant: 1
            },
        );
        assert_eq!(
            entity.ruined,
            Selection {
                admitted: id < 3,
                mesh: 23,
                definition: 21,
                variant: 0
            },
        );
        assert!(!entity.destroyed && entity.damaged && !entity.uncertain);
        let expected = ModelInstance {
            position: [1.0, 0.0, 1.0],
            scale: 1.0,
            animation_seed: id as u32,
            ..Default::default()
        };
        assert_eq!(
            bytemuck::bytes_of(&entity.instance),
            bytemuck::bytes_of(&expected)
        );
    }
}

#[test]
fn symbolic_buildings_preserve_the_independent_local_history_schedule() {
    use map_display_data::buildings as wire;
    let key = |slot: usize| {
        Arc::new(wire::AssetKey::Selected {
            entity: format!("synthetic-{slot}"),
            namespace: Vec::new(),
            role: 0,
            contract: 1,
            force_pow2_textures: false,
        })
    };
    let authored = Arc::new(vec![wire::AuthoredSlot {
        id: 0,
        location: 1,
        bucket: "synthetic".into(),
        order: 0,
        province: Some(1),
        position: [0.0; 3],
        rotation: 0.0,
        floating: false,
    }]);
    let mut original = BuildingHistory::default();
    let mut restored = BuildingHistory::default();
    let view = camera([0.0, 0.0], 60.0);
    for (index, health) in [10_000_000, 5_000_000, 7_000_000, 10_000_000]
        .into_iter()
        .enumerate()
    {
        let mut source = prepared(10 + index as i32, health, false);
        source.slots.edit(0, |slot| {
            slot.drawn_at_distance = (index != 2).then_some(false);
        });
        let closed = source
            .display_with(
                [9; 32],
                1501,
                true,
                vec!["second warning".into(), "first warning".into()],
                |slot| Ok(key(slot)),
                &[],
                Arc::clone(&authored),
                None,
            )
            .unwrap();
        let bytes = product_codec::encode(&closed).unwrap();
        let closed: wire::Buildings = product_codec::decode(&bytes).unwrap();
        assert_eq!(closed.perspective, 1501);
        assert_eq!(
            closed.slots[0][0].drawn_at_distance,
            source.slots[0].drawn_at_distance
        );
        assert_eq!(closed.warnings, ["second warning", "first warning"]);
        let installed = PreparedBuildings::install(
            &closed,
            &source.groups,
            |asset| {
                [1, 2]
                    .into_iter()
                    .find(|&slot| key(slot).as_ref() == asset)
                    .ok_or_else(|| io::Error::other("unexpected symbolic key"))
            },
            None,
        )
        .unwrap();
        assert!(source.bit_eq(&installed));
        original.set(source, index != 0);
        restored.set(installed, index != 0);
        assert_eq!(
            original.update(view, [1024, 768], index as f32, true),
            restored.update(view, [1024, 768], index as f32, true)
        );
    }
}

#[test]
fn quiet_tail_reuse_compares_bits_not_float_equality() {
    let source = prepared(10, 10_000_000, false);
    let mut tail = source.slots[0].clone();
    tail.entity_mut().instance.position[0] = -0.0;
    assert_eq!(
        tail.entity.as_ref().unwrap().instance.position[0],
        source.slots[0].entity.as_ref().unwrap().instance.position[0]
    );
    assert_ne!(tail, source.slots[0]);
    let changed = source
        .slots
        .patched(&[], 0, &[tail.clone()], PreparedSlot::same_bits)
        .unwrap()
        .expect("unchanged slot count");
    assert!(!changed.ptr_eq(&source.slots));
    assert_eq!(
        changed[0].entity.as_ref().unwrap().instance.position[0].to_bits(),
        (-0.0f32).to_bits()
    );
    let same = changed
        .patched(&[], 0, &[tail], PreparedSlot::same_bits)
        .unwrap()
        .expect("unchanged slot count");
    assert!(same.ptr_eq(&changed));
}

#[test]
fn symbolic_buildings_validate_authored_group_order_not_bucket_slot_ids() {
    use map_display_data::buildings as data;
    let mut source = prepared(10, 10_000_000, false);
    let sample = source.slots[0].clone();
    source.slots = (0..4)
        .map(|id| PreparedSlot {
            id,
            ..sample.clone()
        })
        .collect();
    source.groups = Arc::from([
        ProvinceSlots {
            province: 9,
            bounds: [0.0, 0.0, 2.0, 2.0],
            slots: vec![2, 0, 1],
        },
        ProvinceSlots {
            province: 18,
            bounds: [2.0, 0.0, 4.0, 2.0],
            slots: vec![3],
        },
    ]);
    let authored = Arc::new(
        (0..4)
            .map(|id| data::AuthoredSlot {
                id,
                location: 1,
                bucket: format!("bucket-{id}"),
                order: 0,
                province: Some(if id == 3 { 18 } else { 9 }),
                position: [0.0; 3],
                rotation: 0.0,
                floating: id == 3,
            })
            .collect(),
    );
    let value = source
        .display_with(
            [7; 32],
            1,
            true,
            Vec::new(),
            |slot| {
                Ok(Arc::new(data::AssetKey::Selected {
                    entity: slot.to_string(),
                    namespace: Vec::new(),
                    role: 0,
                    contract: 1,
                    force_pow2_textures: false,
                }))
            },
            &[],
            authored,
            None,
        )
        .unwrap();
    let bytes = product_codec::encode(&value).unwrap();
    let value: data::Buildings = product_codec::decode(&bytes).unwrap();
    assert_eq!(value.groups[0].slots, [2, 0, 1]);
    let binding = |key: &data::AssetKey| match key {
        data::AssetKey::Selected { entity, .. } => entity.parse().map_err(io::Error::other),
        data::AssetKey::Unbound {} => Err(io::Error::other("fixture asset is unbound")),
    };
    let installed = PreparedBuildings::install(&value, &source.groups, binding, None).unwrap();
    assert!(source.bit_eq(&installed));
    assert_eq!(&*installed.groups, &*source.groups);
    assert_eq!(product_codec::encode(&value).unwrap(), bytes);
    let mut original = BuildingHistory::default();
    let mut restored = BuildingHistory::default();
    original.set(source.clone(), false);
    restored.set(installed, false);
    let camera = camera([0.0, 0.0], 60.0);
    assert_eq!(
        original.update(camera, [1024, 768], 0.0, true),
        restored.update(camera, [1024, 768], 0.0, true)
    );

    let invalid_groups = [
        vec![
            data::Group {
                slots: vec![0, 1, 2],
                ..value.groups[0].clone()
            },
            value.groups[1].clone(),
        ],
        vec![
            data::Group {
                slots: vec![2, 0, 0],
                ..value.groups[0].clone()
            },
            value.groups[1].clone(),
        ],
        vec![
            data::Group {
                slots: vec![2, 0, 4],
                ..value.groups[0].clone()
            },
            value.groups[1].clone(),
        ],
        vec![value.groups[0].clone(), value.groups[0].clone()],
        vec![value.groups[1].clone(), value.groups[0].clone()],
        vec![value.groups[0].clone()],
        vec![
            data::Group {
                bounds: [-0.0, 0.0, 2.0, 2.0],
                ..value.groups[0].clone()
            },
            value.groups[1].clone(),
        ],
    ];
    for groups in invalid_groups {
        let malformed = data::Buildings {
            groups: Arc::new(groups),
            ..value.clone()
        };
        let error =
            PreparedBuildings::install(&malformed, &source.groups, binding, None).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            error.to_string(),
            "building projection group order is invalid"
        );
    }
}

#[test]
fn shared_slot_pages_install_as_the_layers_slots_and_share_untouched_pages() {
    let kind = BuildingType { name: "infrastructure".into(), show_on_map: Some(1), show_on_map_meshes: 1, ..Default::default() };
    let types = Buildings { declaration_order: vec![kind.name.clone()], types: BTreeMap::from([(kind.name.clone(), kind)]), ..Default::default() };
    let defines = Defines { shrink_distance: 180.0, destroyed: 6_000_000, recovered: 8_000_000, damaged: 80_000, max_levels: 10, popup_speed: 5.0 };
    let at = |x: f32| content_reader_map::positions::BuildingPosition { state: 100, kind: "infrastructure".into(), x, y: 0.0, z: x, rotation: 0.0, adjacent_sea: 0 };
    let authored = (0..150).map(|id| (100, "infrastructure".to_owned(), at(id as f32), None)).collect();
    let assets = MapBuildingAssets::stub(&types, defines, authored, Vec::new(), Vec::new(), None);
    let hour = |at: i32| WireHour { branch: 0, hour: at, perspective: 1, fog: true, warnings: Vec::new(), city_damage: Arc::default() };
    let mut slots: Vec<PreparedSlot> = (0..150).map(PreparedSlot::empty).collect();
    let mut history = BuildingHistory::default();
    let first = assets.slot_pages(&slots, &[], None).unwrap();
    let product = assets.shared_product(&first, [7; 32], hour(1)).unwrap();
    let installed = history.install_display(&product, &assets).unwrap();
    assert!((0..150).all(|id| installed.slots[id] == slots[id]));
    for id in [70, 140] {
        slots[id].source = Some((0, id as u32));
        slots[id].drawn_at_distance = Some(false);
    }
    let patched = assets.slot_pages(&slots, &[2, 1, 1], Some(&first)).unwrap();
    assert!(Arc::ptr_eq(patched.0.chunks.get(0).unwrap(), first.0.chunks.get(0).unwrap()), "an untouched page is shared");
    assert_eq!(patched.0, assets.slot_pages(&slots, &[], None).unwrap().0, "patched pages equal pages made whole");
    let groups = Arc::clone(&product.groups);
    let product = assets.shared_product(&patched, [7; 32], hour(2)).unwrap();
    let installed = history.install_display(&product, &assets).unwrap();
    assert!((0..150).all(|id| installed.slots[id] == slots[id]), "the install takes the layer's slots as they are");
    assert!(installed.slots.ptr_eq(&patched.0), "the install shares the pages, converting nothing");
    assert!(Arc::ptr_eq(&groups, &product.groups), "every product shares one set of province boxes");
    assert!(Arc::ptr_eq(&groups, &assets.shared_product(&first, [7; 32], hour(3)).unwrap().groups));
}
