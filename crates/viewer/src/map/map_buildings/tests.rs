#[test]
fn only_states_the_building_producer_cannot_select_are_refused() {
    let descriptors = installed_content::mesh::parse_descriptors(br#"
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

use super::super::perspective::{Choice, Country, Vision};
use super::*;
use installed_content::buildings;
use replay::{
    Hour,
    frame::{DecodedFrame, Metrics, TableView},
    rows::{
        BuildingRow, CountryLinkRow, LexicalTokenRow, LocalPlayerRow, RaidScope,
        RaidTargetStatusRow, TableId, TableRows,
    },
};

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
    buildings::parse(SAMPLE, &installed_content::key_names::KeyNames::default()).unwrap()
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
    super::super::camera::Placement { center, height }.on(
        super::super::camera::MapSize::TEST,
        super::super::camera::MapDefines::test(),
    )
}

/// Every drawn entity after one frame at `height` over the origin.
fn at(history: &mut BuildingHistory, height: f32, time: f32) -> Vec<Placed> {
    let _ = history.update(camera([0.0, 0.0], height), [1024, 768], time, true);
    history.collect(None, 0.0)
}

fn cold_instances(prepared: PreparedBuildings) -> Vec<Placed> {
    let mut history = BuildingHistory::default();
    history.set(prepared, false);
    at(&mut history, 50.0, 0.0)
}

fn group(bounds: [f32; 4], slots: Vec<u32>) -> ProvinceSlots {
    ProvinceSlots {
        province: 0,
        bounds,
        slots,
    }
}

fn prepared(hour: Hour, health: i128, landmark: bool) -> PreparedBuildings {
    PreparedBuildings {
        branch: 1,
        hour,
        shrink_distance: 180.0,
        popup_speed: 5.0,
        slots: vec![PreparedSlot {
            id: 0,
            drawn_at_distance: Some(landmark),
            entity: Some(PreparedEntity {
                healthy: Selection {
                    admitted: true,
                    mesh: 1,
                    definition: 1,
                    variant: 0,
                },
                ruined: Selection {
                    admitted: true,
                    mesh: 2,
                    definition: 2,
                    variant: 0,
                },
                destroyed: health <= 6_000_000,
                uncertain: health > 6_000_000 && health <= 8_000_000,
                damaged: health < 8_000_000,
                disable_grow_animation: false,
                instance: ModelInstance {
                    scale: 1.0,
                    ..Default::default()
                },
            }),
        }],
        groups: Arc::from([group([0.0, 0.0, 2.0, 2.0], vec![0])]),
    }
}

fn budget_buildings(provinces: &[(u32, usize)]) -> BuildingHistory {
    let mut input = prepared(10, 10_000_000, false);
    let template = input.slots[0].clone();
    input.slots.clear();
    let mut groups = Vec::new();
    for &(province, count) in provinces {
        let mut ids = Vec::new();
        for _ in 0..count {
            let mut slot = template.clone();
            slot.id = input.slots.len() as u32;
            let healthy = &mut slot.entity.as_mut().unwrap().healthy;
            healthy.mesh = 100 + slot.id as usize;
            healthy.definition = healthy.mesh;
            ids.push(slot.id);
            input.slots.push(slot);
        }
        groups.push(ProvinceSlots {
            province,
            bounds: [0.0, 0.0, 2.0, 2.0],
            slots: ids,
        });
    }
    input.groups = groups.into();
    let mut history = BuildingHistory::default();
    history.set(input, false);
    history
}

fn mixed_frame(
    units: &mut super::super::unit_history::History,
    buildings: &mut BuildingHistory,
    limit: i32,
    time: f32,
) -> Vec<usize> {
    let frame = units.update_models(
        camera([0.0, 0.0], 50.0),
        [1024, 768],
        time,
        super::super::unit_history::tests::GATES,
        limit,
        buildings,
        true,
    );
    let mut meshes: Vec<_> = frame
        .units
        .iter()
        .map(|(mesh, _)| *mesh)
        .chain(buildings.collect(None, 0.0).iter().map(|(mesh, _)| *mesh))
        .collect();
    meshes.sort();
    meshes
}

fn budget_units(
    targets: Vec<super::super::unit_history::Target>,
    steady: bool,
) -> super::super::unit_history::History {
    let bounds = targets
        .iter()
        .map(|t| (t.province, [0.0, 0.0, 2.0, 2.0]))
        .collect();
    let mut units = super::super::unit_history::History::default();
    units.set(targets, Arc::new(bounds), steady);
    units
}

#[test]
#[ignore = "release mixed model history measurement"]
fn mixed_model_budget_cost() {
    use super::super::unit_history::{Kind, Target, tests::target};
    for count in [1_000u32, 10_000, 50_000] {
        let targets: Vec<_> = (1..=count)
            .flat_map(|province| {
                [
                    target(province, Kind::Land),
                    Target {
                        mesh: 1,
                        ..target(province, Kind::Ship)
                    },
                ]
            })
            .collect();
        let mut units = budget_units(targets, false);
        let mut buildings = budget_buildings(&(1..=count).map(|id| (id, 1)).collect::<Vec<_>>());
        // Steady visibility first, then measure the actual live scheduler.
        units.settle();
        mixed_frame(&mut units, &mut buildings, 7, 0.0);
        units.steady_for_test(false);
        let mut samples = Vec::new();
        for frame in 0..200 {
            let start = std::time::Instant::now();
            std::hint::black_box(units.update_models(
                camera([0.0, 0.0], 50.0),
                [1024, 768],
                frame as f32 / 120.0,
                super::super::unit_history::tests::GATES,
                7,
                &mut buildings,
                true,
            ));
            samples.push(start.elapsed().as_secs_f64() * 1e3);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "MIXED_BUDGET provinces={count} models={} frame_p50_ms={:.4} frame_p95_ms={:.4} frame_max_ms={:.4}",
            count * 3,
            samples[100],
            samples[190],
            samples[199]
        );
    }
}

#[test]
fn mixed_budget_is_shared_in_province_order_and_retries_on_unchanged_frames() {
    use super::super::unit_history::{Kind, tests::target};
    // Input order and identity order deliberately differ from native slot order.
    let targets = vec![
        super::super::unit_history::Target {
            mesh: 3,
            identity: 1,
            order: 2,
            ..target(30, Kind::Land)
        },
        super::super::unit_history::Target {
            mesh: 2,
            identity: 2,
            order: 1,
            ..target(30, Kind::Land)
        },
        super::super::unit_history::Target {
            mesh: 1,
            identity: 3,
            order: 0,
            ..target(30, Kind::Land)
        },
        super::super::unit_history::Target {
            mesh: 11,
            part: 1,
            ..target(20, Kind::Ship)
        },
        super::super::unit_history::Target {
            mesh: 10,
            part: 0,
            ..target(20, Kind::Ship)
        },
    ];
    let mut exact = budget_units(targets.clone(), false);
    let mut early_building = budget_buildings(&[(10, 1)]);
    assert_eq!(
        mixed_frame(&mut exact, &mut early_building, 4, 0.0),
        [1, 10, 11, 100]
    );
    let mut units = budget_units(targets, false);
    let mut buildings = budget_buildings(&[(10, 1), (40, 1)]);
    // Early building, two hulls, one queued figure; the later building
    // consumes the remaining inline slot before that figure creates.
    assert_eq!(
        mixed_frame(&mut units, &mut buildings, 4, 0.0),
        [1, 10, 11, 100, 101]
    );
    assert_eq!(
        mixed_frame(&mut units, &mut buildings, 4, 0.1),
        [1, 2, 3, 10, 11, 100, 101]
    );
    // A fresh frame with N=3 fills only the building and both hulls.
    units.restart_live();
    buildings.restart_live();
    assert_eq!(
        mixed_frame(&mut units, &mut buildings, 3, 0.0),
        [10, 11, 100]
    );
    assert_eq!(
        mixed_frame(&mut units, &mut buildings, 3, 0.1),
        [1, 2, 3, 10, 11, 100, 101]
    );
}

#[test]
fn buildings_precede_hulls_and_held_hulls_keep_the_growing_root() {
    use super::super::unit_history::{Kind, Target, tests::target};
    let targets = (0..4)
        .rev()
        .map(|part| Target {
            part,
            mesh: part as usize,
            ..target(10, Kind::Ship)
        })
        .collect();
    let mut units = budget_units(targets, false);
    let mut buildings = budget_buildings(&[(10, 2)]);
    assert_eq!(
        mixed_frame(&mut units, &mut buildings, 3, 0.0),
        [0, 100, 101]
    );
    let frame = units.update_models(
        camera([0.0, 0.0], 50.0),
        [1024, 768],
        0.125,
        super::super::unit_history::tests::GATES,
        2,
        &mut buildings,
        true,
    );
    assert_eq!(
        frame.units.iter().map(|p| p.0).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert!(frame.units.iter().all(|p| p.1.scale == 0.25));
    assert_eq!(
        mixed_frame(&mut units, &mut buildings, 2, 0.25),
        [0, 1, 2, 3, 100, 101]
    );
}

#[test]
fn skipped_hours_carry_selector_without_creating_outside_a_frame() {
    let mut buildings = budget_buildings(&[(10, 1)]);
    let mut units = budget_units(Vec::new(), false);
    mixed_frame(&mut units, &mut buildings, 1, 0.0);
    let mut skipped = buildings.prepared.clone().unwrap();
    skipped.hour += 1;
    skipped.slots[0].entity.as_mut().unwrap().destroyed = true;
    buildings.advance(&skipped);
    assert!(buildings.collect(None, 0.0).is_empty());
    let mut current = skipped;
    current.hour += 1;
    current.slots[0].entity.as_mut().unwrap().uncertain = true;
    buildings.set(current, true);
    assert!(mixed_frame(&mut units, &mut buildings, 0, 0.1).is_empty());
    assert_eq!(mixed_frame(&mut units, &mut buildings, 1, 0.2), [2]);
}

#[test]
fn authored_growth_exemption_still_spends_one_creation_slot() {
    let mut buildings = budget_buildings(&[(10, 2)]);
    for slot in &mut buildings.prepared.as_mut().unwrap().slots {
        slot.entity.as_mut().unwrap().disable_grow_animation = true;
    }
    let mut units = budget_units(Vec::new(), false);
    assert_eq!(mixed_frame(&mut units, &mut buildings, 1, 0.0), [100]);
    assert_eq!(buildings.collect(None, 0.0)[0].1.scale, 1.0);
    assert_eq!(mixed_frame(&mut units, &mut buildings, 1, 0.0), [100, 101]);
    assert!(
        buildings
            .collect(None, 0.0)
            .iter()
            .all(|p| p.1.scale == 1.0)
    );
}

#[test]
fn damaged_pose_switch_does_not_consume_an_entity_creation() {
    let mut buildings = budget_buildings(&[(10, 1)]);
    let mut units = budget_units(Vec::new(), false);
    mixed_frame(&mut units, &mut buildings, 1, 0.0);
    mixed_frame(&mut units, &mut buildings, 1, 1.0);
    let generation = buildings.slots[&0].generation;
    let mut next = buildings.prepared.clone().unwrap();
    next.hour += 1;
    let entity = next.slots[0].entity.as_mut().unwrap();
    entity.healthy.mesh = 200; // Renderer-only posed copy of the same definition.
    entity.damaged = true;
    buildings.set(next, true);
    assert_eq!(mixed_frame(&mut units, &mut buildings, 0, 1.1), [200]);
    assert_eq!(buildings.slots[&0].generation, generation);
    assert_eq!(buildings.collect(None, 0.0)[0].1.scale, 1.0);
}

#[test]
fn selector_change_recreates_even_when_entity_definition_is_unchanged() {
    let mut buildings = budget_buildings(&[(10, 1)]);
    let mut units = budget_units(Vec::new(), false);
    mixed_frame(&mut units, &mut buildings, 1, 0.0);
    let generation = buildings.slots[&0].generation;
    let mut next = buildings.prepared.clone().unwrap();
    next.hour += 1;
    next.slots[0].entity.as_mut().unwrap().healthy.variant = 1;
    buildings.set(next, true);
    assert!(mixed_frame(&mut units, &mut buildings, 0, 0.1).is_empty());
    assert_eq!(mixed_frame(&mut units, &mut buildings, 1, 0.2), [100]);
    assert_ne!(buildings.slots[&0].generation, generation);
}

#[test]
fn exhausted_building_replacement_is_empty_and_retries() {
    let mut buildings = budget_buildings(&[(10, 1)]);
    let mut units = budget_units(Vec::new(), false);
    assert_eq!(mixed_frame(&mut units, &mut buildings, 1, 0.0), [100]);
    let mut next = buildings.prepared.clone().unwrap();
    next.hour += 1;
    next.slots[0].entity.as_mut().unwrap().healthy.mesh = 200;
    next.slots[0].entity.as_mut().unwrap().healthy.definition = 200;
    buildings.set(next, true);
    assert!(mixed_frame(&mut units, &mut buildings, 0, 0.1).is_empty());
    assert_eq!(mixed_frame(&mut units, &mut buildings, 1, 0.2), [200]);
    assert!((buildings.collect(None, 0.0)[0].1.scale - 0.5).abs() < 1e-6);
}

#[test]
fn building_file_order_and_settled_dump_ignore_interactive_budget() {
    use super::super::unit_history::{Kind, tests::target};
    let mut buildings = budget_buildings(&[(10, 3)]);
    Arc::make_mut(&mut buildings.prepared.as_mut().unwrap().groups)[0].slots = vec![2, 0, 1];
    let mut units = budget_units(vec![target(20, Kind::Land)], false);
    assert_eq!(mixed_frame(&mut units, &mut buildings, 1, 0.0), [102]);
    assert_eq!(mixed_frame(&mut units, &mut buildings, 1, 0.1), [100, 102]);
    units.settle();
    buildings.restart_live();
    assert_eq!(
        mixed_frame(&mut units, &mut buildings, 0, 1.0),
        [0, 100, 101, 102]
    );
    assert!(
        buildings
            .collect(None, 0.0)
            .iter()
            .all(|p| p.1.scale == 1.0)
    );
}

#[test]
fn destruction_hysteresis_survives_hide_and_resets_on_seek() {
    let mut history = BuildingHistory::default();
    history.set(prepared(10, 6_000_000, false), true);
    assert_eq!(at(&mut history, 50.0, 0.0)[0].0, 2);
    let mut hidden = prepared(11, 9_000_000, false);
    hidden.slots[0].entity = None;
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
    // Hour 11 was skipped by a slow frame; bombing destroyed the building.
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
    other.entity.as_mut().unwrap().instance.position = [2001.0, 0.0, 1001.0];
    input.slots.push(other);
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
    same_mesh.slots[0].entity.as_mut().unwrap().ruined.mesh = 1;
    history.set(same_mesh, true);
    let before = at(&mut history, 50.0, 0.0)[0].1.animation_epoch;
    history.set(prepared(11, 10_000_000, false), true);
    let after = at(&mut history, 50.0, 1.0);
    assert_eq!(after[0].0, 1);
    assert_ne!(after[0].1.animation_epoch, before);
}

/// ten units past the view's side planes included.
#[test]
fn buildings_just_past_the_view_edge_are_created() {
    let mut input = prepared(10, 10_000_000, true);
    let boxes = super::super::province_admission::edge_boxes();
    assert_eq!(boxes[&5], [3044.0, 1000.0, 3046.0, 1002.0]);
    let slot = input.slots[0].clone();
    input.slots = boxes
        .keys()
        .map(|&id| {
            let mut slot = slot.clone();
            slot.id = id - 1;
            slot.entity.as_mut().unwrap().instance.position = [id as f32, 0.0, 1000.0];
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

#[test]
fn a_bucket_needs_a_registered_entity_of_its_own_name() {
    let mut roster = Roster::default();
    for name in ["building_one", "building_many_2"] {
        roster
            .descriptors
            .entities
            .insert(name.into(), Default::default());
    }
    let spawn = |name: &str, max, meshes| SpawnPoint {
        name: name.into(),
        max,
        meshes,
        provincial: false,
        is_port: false,
    };
    assert!(bucket_registered(&spawn("one", 1, 1), &roster));
    assert!(!bucket_registered(&spawn("one", 0, 1), &roster));
    assert!(bucket_registered(&spawn("many", 1, 3), &roster));
    assert!(!bucket_registered(&spawn("many", 1, 1), &roster));
    assert!(!bucket_registered(&spawn("absent", 1, 1), &roster));
}

/// The fog loop: below fifty hides, fifty draws, no byte keeps the building.
/// The one-slot fixture: an `arms_factory` state slot at [`ONE_STATE`]
/// over [`ONE_PROVINCE`], its rows named by token [`ONE_TOKEN`].
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

/// An hour whose one building row is `row` at the fixture's state, seen by
/// a player whose fog byte over the fixture's province is `byte`.
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
    let types = buildings::parse(
            b"buildings={ arms_factory={ show_on_map=1 show_on_map_meshes=2 has_destroyed_mesh=yes level_cap={ state_max=20 } } }", &installed_content::key_names::KeyNames::default(),
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
    // 1416648D0 passes selector 0 for this ruin whatever its level, so a
    // level change neither recreates it nor spends the creation allowance.
    assert_eq!(low.ruined, high.ruined);
    assert_eq!((low.ruined.variant, low.ruined.definition), (0, 2));
}

#[test]
fn the_fog_loop_hides_below_fifty_and_keeps_the_undecided() {
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
    // The kept token map follows the hour's LexicalTokens: once the token
    // names something else, the row names no building and says so.
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

/// A foreign `need_detection` building needs the viewer's bit in its own
/// building target status; its own never does, and an incomplete table
/// cannot decide a foreign one.
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
            // A province target at the same location detected by everyone
            // names no building, so it cannot reveal one.
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
    assert_eq!(detection(1).detected(&row), Some(true), "our own building");
    assert_eq!(detection(0).detected(&row), Some(false));
    assert_eq!(detection(2).detected(&row), Some(true));
    let mut incomplete = detection(0);
    incomplete.targets = None;
    assert_eq!(incomplete.detected(&row), None);
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
    let types = token_types(&frame, &assets.keys, &assets.index).unwrap();
    // Token 0 names no one key; case folds; two keys under one fold are
    // refused; a token naming no type is left for the rows to word.
    assert!(!types.contains_key(&0));
    assert_eq!(types[&3], Ok(0));
    assert_eq!(
        types[&4],
        Err("token bunker names several content keys: BUNKER, Bunker".to_owned())
    );
    assert!(!types.contains_key(&5));
}

/// Assets over `ordered`, in that declaration order, with no slots.
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
    let mut types = types();
    let factory = types.types.get_mut("arms_factory").unwrap();
    factory.air_base = true;
    factory.rocket_launch_capacity = 1;
    factory.gun_emplacement = true;
    let assets = one_slot_assets(&types, BTreeMap::new());
    assert_eq!(assets.province_sites(ONE_STATE, ONE_PROVINCE), [true; 3]);
    assert_eq!(
        assets.province_sites(ONE_STATE, ONE_PROVINCE + 1),
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
            .is_facility("Facility")
            .unwrap_err()
            .contains("names several content keys")
    );
    let assets = assets_of(vec![BuildingType {
        name: "Facility".into(),
        specialization: vec!["science".into()],
        ..Default::default()
    }]);
    assert_eq!(assets.is_facility("FACILITY"), Ok(true));
}

/// A frame of `StateBuildings` rows of one country, the recorded player,
/// over `tokens` as the hour's `LexicalTokens`.
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

/// A row's token that names no building type is refused by reason:
/// absent, among the missing rows, not text, or spelled but not authored.
/// Type 0 is none, so it is neither drawn nor refused.
#[test]
fn building_rows_refuse_their_unnamed_type_tokens_by_reason() {
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
            ("token 78 is not in LexicalTokens".to_owned(), 1),
            ("token 79 is not spelled as text".to_owned(), 1),
            (
                "token 80 (not_a_building) names no authored building type".to_owned(),
                2
            ),
        ])
    );
    assert_eq!(
        summary.warnings()[0],
        "Map buildings: 1 rows not drawn, refused type token: token 78 is not in LexicalTokens."
    );
    // A truncated table cannot say the token does not exist.
    let (frame, vision) = buildings_frame(TableView::Truncated { rows, missing: 4 }, &[0, 78]);
    let (_, summary) = assets.field(&frame, &vision, &no_city()).unwrap();
    assert_eq!(summary.unnamed_tokens, 1);
    assert_eq!(
        summary.refused_tokens,
        BTreeMap::from([(
            "token 78 is among LexicalTokens' missing rows".to_owned(),
            1
        )])
    );
    // Only type 0: nothing drawn, nothing refused.
    let (frame, vision) = buildings_frame(
        TableView::Complete(TableRows::LexicalTokens(Vec::new())),
        &[0],
    );
    let (_, summary) = assets.field(&frame, &vision, &no_city()).unwrap();
    assert_eq!(
        (summary.built, summary.unnamed_tokens),
        (0, 0),
        "type 0 is none"
    );
    assert!(summary.refused_tokens.is_empty());
    assert!(summary.warnings().is_empty());
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
fn slots_are_looked_up_by_their_truncated_position() {
    // A 4 x 3 map: world z is `height - 1 - row`, so row 0 is z = 2.
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
fn ordinary_slots_require_land_in_their_declared_state() {
    use installed_content::map::{CountryColors, ProvinceDefinition};
    let definition = |id, kind| ProvinceDefinition {
        id,
        rgb: [0, 0, 0],
        kind,
        continent: 0,
    };
    let map = MapData {
        width: 2,
        height: 1,
        province_ids: vec![7, 8],
        definitions: BTreeMap::from([
            (7, definition(7, ProvinceKind::Land)),
            (8, definition(8, ProvinceKind::Sea)),
        ]),
        country_colors: CountryColors::default(),
    };
    let state = |id, provinces| StateDefinition {
        id,
        provinces,
        ..Default::default()
    };
    let states = BTreeMap::from([(3, state(3, vec![7])), (4, state(4, vec![8]))]);
    let mut position = BuildingPosition {
        state: 3,
        kind: "arms_factory".into(),
        x: 0.0,
        y: 0.0,
        z: 0.0,
        rotation: 0.0,
        adjacent_sea: 0,
    };
    assert!(admitted_position(&position, Some(7), &map, &states));
    assert!(!admitted_position(&position, None, &map, &states));
    assert!(!admitted_position(&position, Some(8), &map, &states));
    position.state = 4;
    assert!(!admitted_position(&position, Some(7), &map, &states));
}

#[test]
fn the_update_cutoff_is_water_relative() {
    assert!(!draws_map_buildings(302.960, 180.0));
    assert!(draws_map_buildings(115.293, 180.0));
    assert!(draws_map_buildings(189.4, 180.0));
    assert!(!draws_map_buildings(189.6, 180.0));
}

/// Undecided floating harbours reach the diagnostics by reason with the refused tokens,
/// through the summary the hour builder hands the window, a capture and map details.
#[test]
fn undecided_floating_harbours_are_listed_by_reason() {
    let summary = MapBuildingSummary {
        refused_tokens: BTreeMap::from([("token 78 is not in LexicalTokens".to_owned(), 1)]),
        unknown_floating_harbors: BTreeMap::from([
            ("the controller's floating-harbour deployment is unknown", 2),
            ("the fog byte of the harbour's sea province is unknown", 1),
        ]),
        ..Default::default()
    };
    assert_eq!(
        summary.warnings(),
        [
            "Map buildings: 1 rows not drawn, refused type token: token 78 is not in LexicalTokens.",
            "Map buildings: 2 floating harbours not drawn: the controller's floating-harbour deployment is unknown.",
            "Map buildings: 1 floating harbours not drawn: the fog byte of the harbour's sea province is unknown.",
        ]
    );
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
