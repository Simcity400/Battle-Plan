use super::{
    Arc, BTreeMap, NativeInputs, NavalContext, NavalUnit, Sight, Vision,
    naval_members, port_flag_owner, type_tint,
};
use viewer_counter_kit::unit_flag_owner;
use viewer_foundation::unit_rules::DEFENDING;
use replay_data::rows::TaskForceRow;

fn unit(force: &TaskForceRow, slot: u32, strength: i64) -> NavalUnit<'_> {
    NavalUnit {
        force,
        ordinal: 0,
        slot,
        flag_owner: unit_flag_owner(slot, force.expeditionary_owner, false, &Some),
        sight: Sight::Full,
        ships: Some(1),
        position: None,
        strength,
        logical: Some(slot),
        byte: 255,
    }
}

#[test]
fn task_force_flags_distinguish_coarse_sea_and_detailed_port() {
    let vision = Vision::default();
    let native = NativeInputs::default();
    let context = NavalContext {
        vision: &vision,
        type_tints: &type_tint::Index::default(),
        native: &native,
        ship_inputs: None,
        aggression: &[],
        ship_task_forces: Arc::new(BTreeMap::new()),
        ships_complete: true,
        ships: Arc::new(BTreeMap::new()),
        country_ids: &BTreeMap::new(),
        ideology: &|_| Ok(None),
        missions: Arc::new(BTreeMap::new()),
        mission_skips: Default::default(),
        forces: &[],
        forces_complete: true,
    };
    for (expeditionary, detailed) in [(5_000, 5_000), (0, 7), (u32::MAX, 7)] {
        let force = TaskForceRow {
            roster_owner: 7,
            expeditionary_owner: expeditionary,
            logical_owner: 9,
            ..Default::default()
        };
        let first = unit(&force, 7, 0);
        let second = NavalUnit {
            sight: Sight::Partial,
            ..unit(&force, 7, 0)
        };
        for coarse in [false, true] {
            let sea = context.flotilla(&[&first, &second], false, coarse);
            assert_eq!(sea.flag_owner, Some(if coarse { 7 } else { detailed }));
            assert_eq!(sea.coarse, coarse);
            let port = context.flotilla(&[&first, &second], true, coarse);
            assert_eq!(port.flag_owner, Some(detailed));
            assert!(!port.coarse);
        }
    }
}

#[test]
fn mixed_port_flag_uses_lowest_owner_strength_then_native_reference_order() {
    let first = TaskForceRow {
        roster_owner: 30,
        expeditionary_owner: 99,
        logical_owner: 90,
        ..Default::default()
    };
    let other = TaskForceRow {
        roster_owner: 20,
        ..first
    };
    let a = unit(&first, 2, 40);
    let b = unit(&other, 8, 70);
    let reinforcement = unit(&first, 2, 50);
    assert_eq!(port_flag_owner(&[&a, &b], true), Some(2));
    let reinforcement = NavalUnit {
        sight: Sight::Partial,
        ..reinforcement
    };
    assert_eq!(port_flag_owner(&[&a, &b, &reinforcement], true), Some(8));
    let tied = unit(&other, 8, 40);
    assert_eq!(port_flag_owner(&[&a, &tied], true), Some(8));
    assert_eq!(port_flag_owner(&[&tied, &a], true), Some(8));
    assert_eq!(port_flag_owner(&[&a, &b], false), None);
    assert_eq!(port_flag_owner(&[&a, &a], false), Some(99));
    let negative = unit(&other, 8, -1);
    assert_eq!(port_flag_owner(&[&a, &negative], true), Some(8));
}

#[test]
fn the_counter_pass_leaves_a_patrol_here_to_the_mission_icon() {
    use viewer_unit_reads::task_force_visual::MissionSkips;
    use viewer_foundation::unit_rules::PATROL;
    let patrol = TaskForceRow {
        id: (6689 << 32) | 51,
        mission: PATROL,
        ..Default::default()
    };
    let skips = MissionSkips::fixture([(patrol.id, (1, 0))], [(patrol.id, 79)]);
    let sea = |kind, region, skips: &MissionSkips| naval_members::Fake { kind, region, skips: skips.clone(), ..naval_members::Fake::new(&patrol) }.sea();
    assert_eq!(sea(0, Some(79), &skips), naval_members::Sea::Out(naval_members::ON_MISSION));
    assert!(matches!(sea(0, Some(76), &skips), naval_members::Sea::Source { .. }));
    assert!(matches!(sea(DEFENDING, Some(79), &skips), naval_members::Sea::Source { .. }));
    assert_eq!(sea(0, Some(79), &MissionSkips::default()), naval_members::Sea::Refused(naval_members::SKIP_UNRECORDED));
}

