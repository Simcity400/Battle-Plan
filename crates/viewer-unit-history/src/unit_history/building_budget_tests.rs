use viewer_buildings::map_buildings::{
    BuildingHistory,
    budget_fixture::{budget_buildings, damage, destruction, exempt_growth, file_order, generation, hour_after, next_hour, replace, variant},
};
use std::sync::Arc;
use viewer_gpu::camera::{Camera, MapDefines, MapSize, Placement};

fn camera(center: [f32; 2], height: f32) -> Camera {
    Placement {
        center,
        height,
        pose: viewer_gpu::camera::Pose::Map,
    }
    .on(MapSize::TEST, MapDefines::test())
}

fn mixed_frame(
    units: &mut crate::unit_history::History,
    buildings: &mut BuildingHistory,
    limit: i32,
    time: f32,
) -> Vec<usize> {
    let frame = units.update_models(
        camera([0.0, 0.0], 50.0),
        [1024, 768],
        time,
        crate::unit_history::test_support::GATES,
        limit,
        buildings,
        true,
    );
    let mut meshes: Vec<_> = units
        .drawn(&frame.units)
        .iter()
        .map(|(mesh, _)| *mesh)
        .chain(buildings.collect(None, 0.0).iter().map(|(mesh, _)| *mesh))
        .collect();
    meshes.sort();
    meshes
}

fn budget_units(
    targets: Vec<crate::unit_history::Target>,
    steady: bool,
) -> crate::unit_history::History {
    let bounds = targets
        .iter()
        .map(|t| (t.province, [0.0, 0.0, 2.0, 2.0]))
        .collect();
    let mut units = crate::unit_history::History::default();
    units.set(targets, Arc::new(bounds), steady);
    units
}

#[test]
fn mixed_budget_is_shared_in_province_order_and_retries_on_unchanged_frames() {
    use crate::unit_history::{Kind, test_support::target};
    let targets = vec![
        crate::unit_history::Target {
            mesh: 3,
            identity: 1,
            order: 2,
            ..target(30, Kind::Land)
        },
        crate::unit_history::Target {
            mesh: 2,
            identity: 2,
            order: 1,
            ..target(30, Kind::Land)
        },
        crate::unit_history::Target {
            mesh: 1,
            identity: 3,
            order: 0,
            ..target(30, Kind::Land)
        },
        crate::unit_history::Target {
            mesh: 11,
            part: 1,
            ..target(20, Kind::Ship)
        },
        crate::unit_history::Target {
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
    assert_eq!(
        mixed_frame(&mut units, &mut buildings, 4, 0.0),
        [1, 10, 11, 100, 101]
    );
    assert_eq!(
        mixed_frame(&mut units, &mut buildings, 4, 0.1),
        [1, 2, 3, 10, 11, 100, 101]
    );
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
    use crate::unit_history::{Kind, Target, test_support::target};
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
        crate::unit_history::test_support::GATES,
        2,
        &mut buildings,
        true,
    );
    let drawn = units.drawn(&frame.units);
    assert_eq!(drawn.iter().map(|p| p.0).collect::<Vec<_>>(), [0, 1, 2]);
    assert!(drawn.iter().all(|p| p.1.scale == 0.25));
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
    let mut skipped = next_hour(&buildings);
    destruction(&mut skipped, true, false);
    buildings.advance(&skipped);
    assert!(buildings.collect(None, 0.0).is_empty());
    let mut current = hour_after(skipped);
    destruction(&mut current, false, true);
    buildings.set(current, true);
    assert!(mixed_frame(&mut units, &mut buildings, 0, 0.1).is_empty());
    assert_eq!(mixed_frame(&mut units, &mut buildings, 1, 0.2), [2]);
}

#[test]
fn authored_growth_exemption_still_spends_one_creation_slot() {
    let mut buildings = budget_buildings(&[(10, 2)]);
    exempt_growth(&mut buildings);
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
    let previous_generation = generation(&buildings);
    let mut next = next_hour(&buildings);
    damage(&mut next, 200);
    buildings.set(next, true);
    assert_eq!(mixed_frame(&mut units, &mut buildings, 0, 1.1), [200]);
    assert_eq!(generation(&buildings), previous_generation);
    assert_eq!(buildings.collect(None, 0.0)[0].1.scale, 1.0);
}

#[test]
fn selector_change_recreates_even_when_entity_definition_is_unchanged() {
    let mut buildings = budget_buildings(&[(10, 1)]);
    let mut units = budget_units(Vec::new(), false);
    mixed_frame(&mut units, &mut buildings, 1, 0.0);
    let previous_generation = generation(&buildings);
    let mut next = next_hour(&buildings);
    variant(&mut next, 1);
    buildings.set(next, true);
    assert!(mixed_frame(&mut units, &mut buildings, 0, 0.1).is_empty());
    assert_eq!(mixed_frame(&mut units, &mut buildings, 1, 0.2), [100]);
    assert_ne!(generation(&buildings), previous_generation);
}

#[test]
fn exhausted_building_replacement_is_empty_and_retries() {
    let mut buildings = budget_buildings(&[(10, 1)]);
    let mut units = budget_units(Vec::new(), false);
    assert_eq!(mixed_frame(&mut units, &mut buildings, 1, 0.0), [100]);
    let mut next = next_hour(&buildings);
    replace(&mut next, 200, 200);
    buildings.set(next, true);
    assert!(mixed_frame(&mut units, &mut buildings, 0, 0.1).is_empty());
    assert_eq!(mixed_frame(&mut units, &mut buildings, 1, 0.2), [200]);
    assert!((buildings.collect(None, 0.0)[0].1.scale - 0.5).abs() < 1e-6);
}

#[test]
fn building_file_order_and_settled_dump_ignore_interactive_budget() {
    use crate::unit_history::{Kind, test_support::target};
    let mut buildings = budget_buildings(&[(10, 3)]);
    file_order(&mut buildings, vec![2, 0, 1]);
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
