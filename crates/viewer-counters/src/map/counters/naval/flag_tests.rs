use super::*;
use replay::rows::TaskForceRow;

fn unit(force: &TaskForceRow, slot: u32, strength: i64) -> NavalUnit<'_> {
    NavalUnit {
        force,
        ordinal: 0,
        slot,
        flag_owner: unit_flag_owner(slot, force.expeditionary_owner, false, &Some),
        sight: Sight::Full,
        ships: 1,
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
    let mut context = NavalContext {
        vision: &vision,
        type_tints: &type_tint::Index::default(),
        native: &native,
        ship_inputs: None,
        aggression: &[],
        ship_task_forces: BTreeMap::new(),
        ships_complete: true,
        ships: BTreeMap::new(),
        country_ids: &BTreeMap::new(),
        ideology: &|_| Ok(None),
        missions: BTreeMap::new(),
        mission_skips: Default::default(),
        forces: &[],
        forces_complete: true,
        coarse: false,
    };
    for (expeditionary, detailed) in [(5_000, 5_000), (0, 7), (u32::MAX, 7)] {
        let force = TaskForceRow {
            roster_owner: 7,
            expeditionary_owner: expeditionary,
            logical_owner: 9,
            ..Default::default()
        };
        let first = unit(&force, 7, 0);
        // Neither a partial-intel flag nor port flags depend on visibility or strength.
        let second = NavalUnit {
            sight: Sight::Partial,
            ..unit(&force, 7, 0)
        };
        for coarse in [false, true] {
            context.coarse = coarse;
            let sea = context.flotilla(&[&first, &second], false);
            assert_eq!(sea.flag_owner, Some(if coarse { 7 } else { detailed }));
            let port = context.flotilla(&[&first, &second], true);
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
        ..first.clone()
    };
    let a = unit(&first, 2, 40);
    let b = unit(&other, 8, 70);
    let reinforcement = unit(&first, 2, 50);
    assert_eq!(port_flag_owner(&[&a, &b], true), Some(2));
    // Combine all task forces of each owner, even partially visible ones.
    let reinforcement = NavalUnit {
        sight: Sight::Partial,
        ..reinforcement
    };
    assert_eq!(port_flag_owner(&[&a, &b, &reinforcement], true), Some(8));
    let tied = unit(&other, 8, 40);
    // Raw owner 20 sorts before 30, though its canonical slot sorts after 2.
    assert_eq!(port_flag_owner(&[&a, &tied], true), Some(8));
    assert_eq!(port_flag_owner(&[&tied, &a], true), Some(8));
    assert_eq!(port_flag_owner(&[&a, &b], false), None);
    assert_eq!(port_flag_owner(&[&a, &a], false), Some(99));
    // Native comparison is signed and sums wrap like its ADD instructions.
    let negative = unit(&other, 8, -1);
    assert_eq!(port_flag_owner(&[&a, &negative], true), Some(8));
}

#[test]
fn unavailable_mixed_port_flags_are_counted_and_reported() {
    let report = CounterReport {
        unavailable_naval_flags: 2,
        ..Default::default()
    };
    let mut copied = CounterReport::default();
    copied.assign(&report);
    assert_eq!(copied, report);
    assert_eq!(
        copied.warnings().collect::<Vec<_>>(),
        ["Counters: 2 mixed-owner port flags unavailable: incomplete Ships census."]
    );
}

/// The counter pass drops an active patrol in the region it patrols now and keeps one
/// outside it, in combat, or whose skip is decided otherwise; an undecided one is refused
/// and listed.
#[test]
fn the_counter_pass_leaves_a_patrol_here_to_the_mission_icon() {
    use {
        viewer_foundation::map::task_force_visual::MissionSkips,
        viewer_foundation::map::unit_rules::PATROL,
    };
    let patrol = TaskForceRow {
        id: (6689 << 32) | 51,
        mission: PATROL,
        ..Default::default()
    };
    let skips = MissionSkips::fixture([(patrol.id, (1, 0))], [(patrol.id, 79)]);
    let mut report = CounterReport::default();
    assert!(!stays_a_source(&skips, &patrol, 0, Some(79), &mut report));
    assert!(stays_a_source(&skips, &patrol, 0, Some(76), &mut report));
    assert!(stays_a_source(
        &skips,
        &patrol,
        DEFENDING,
        Some(79),
        &mut report
    ));
    assert!(report.unplaced_task_forces.is_empty());
    assert!(!stays_a_source(
        &MissionSkips::default(),
        &patrol,
        0,
        Some(79),
        &mut report
    ));
    assert_eq!(
        report
            .unplaced_task_forces
            .values()
            .flatten()
            .collect::<Vec<_>>(),
        [&patrol.id]
    );
}

/// A reset group's number keeps its raw bits through `frame update`'s `int`: `0xFFFFFFFF` is -1,
/// not a refusal; a numbered group's is its place.
#[test]
fn a_reset_ideology_number_reaches_set_frame_as_its_bits() {
    assert_eq!(set_frame_argument(0xFFFF_FFFF), -1);
    assert_eq!(set_frame_argument(3), 3);
}
