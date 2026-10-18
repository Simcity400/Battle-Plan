use super::*;
use replay_data::{
    rows::TableId, rows::TableRows, rows::TaskForceCounterInputRow, rows::TaskForceRow,
};
use replay_frame::{frame::Metrics, frame::TableView};
use viewer_foundation::map::unit_rules::ATTACKING;

/// Exclude combat task forces from counter sources after preparing their
/// model buckets.
#[test]
fn combat_task_forces_never_offer_sea_counter_sources() {
    let skips = viewer_foundation::map::task_force_visual::MissionSkips::fixture([(7, (0, 0))], []);
    for combat_references in [i32::MIN, -1, 0, 1, i32::MAX] {
        let force = replay_data::rows::TaskForceRow {
            id: 7,
            combat_references,
            ..Default::default()
        };
        for kind in [0, ATTACKING, DEFENDING] {
            let mut report = CounterReport::default();
            assert_eq!(
                stays_a_source(&skips, &force, kind, Some(3), &mut report),
                combat_references <= 0,
                "combat references {combat_references}, kind {kind}"
            );
            assert!(report.unplaced_task_forces.is_empty());
        }
    }
}

/// Combat task forces are excluded before sea-source grouping, while their
/// model buckets remain available.
#[test]
fn combat_task_forces_leave_sea_group_anchors_unchanged_at_both_heights() {
    use replay_data::{rows::ShipCounterInputs, rows::ShipModelRow, rows::ShipRow};
    use {
        crate::map::counter_groups, crate::map::counter_groups::Extents,
        crate::map::counter_groups::GroupDefines, crate::map::counter_groups::Icon,
        crate::map::counter_groups::Level,
    };

    let eng = TaskForceRow {
        id: (835_u64 << 32) | 61,
        province: 2107,
        roster_owner: 1,
        ..Default::default()
    };
    let pol = TaskForceRow {
        id: (3197_u64 << 32) | 61,
        province: 2333,
        roster_owner: 2,
        combat_references: 1,
        ..Default::default()
    };
    // The CL wins by individual strength even over greater combined DD strength.
    // Distinct anchors expose a wrong group mean.
    let rosters: Vec<(ShipRow, ShipModelRow)> = [
        (eng.id, b"CL".as_slice()),
        (eng.id, b"DD".as_slice()),
        (eng.id, b"DD".as_slice()),
        (eng.id, b"DD".as_slice()),
        (pol.id, b"SS".as_slice()),
    ]
    .into_iter()
    .enumerate()
    .map(|(ordinal, (task_force, name))| {
        let id = ordinal as u64 + 1;
        (
            ShipRow {
                id,
                task_force,
                strength: if name == b"CL" { 200_000 } else { 100_000 },
                ..Default::default()
            },
            ShipModelRow {
                ship: id,
                fallback_sprite: name.to_vec(),
                counter: Some(ShipCounterInputs {
                    definition_valid: 1,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
    })
    .collect();
    let roster = |force| {
        rosters
            .iter()
            .filter(move |(ship, _)| ship.task_force == force)
    };
    for (force, name) in [(eng.id, b"CL".as_slice()), (pol.id, b"SS".as_slice())] {
        assert_eq!(
            counter_order::hull_icon(
                roster(force).map(|(ship, model)| counter_order::Hull {
                    strength: ship.strength,
                    model,
                }),
                Err("unreached equipment content"),
            ),
            counter_order::HullIcon::Resolved(Some(name.to_vec()))
        );
    }

    let skips = viewer_foundation::map::task_force_visual::MissionSkips::fixture(
        [(eng.id, (0, 0)), (pol.id, (0, 0))],
        [],
    );
    let eng_anchor = [10.0, 9.5, 20.0];
    let pol_anchor = [50.0, 9.5, 60.0];
    let defines = GroupDefines::VANILLA;
    let extents = Extents::build(
        &[
            (2107, [0.0, 0.0, 20.0, 30.0]),
            (2333, [40.0, 40.0, 60.0, 70.0]),
        ]
        .into(),
        &BTreeMap::new(),
        &[(2107, 206), (2333, 206)].into(),
        &defines,
        100.0,
    );
    for (height, level) in [
        (114.5896835_f64 as f32, Level::Province),
        (198.1034088_f64 as f32, Level::State),
    ] {
        let mut report = CounterReport::default();
        let sources: Vec<_> = [(&eng, 0, eng_anchor), (&pol, ATTACKING, pol_anchor)]
            .into_iter()
            .filter(|(force, kind, _)| stays_a_source(&skips, force, *kind, Some(206), &mut report))
            .map(|(force, _, anchor)| Icon {
                kind: 1,
                owner: force.roster_owner,
                province: force.province,
                state: None,
                region: Some(206),
                anchor,
            })
            .collect();
        assert_eq!(defines.grouping(height), level);
        assert_eq!(sources.len(), 1, "height {height}: POL offers no plate");
        assert_eq!(sources[0].province, eng.province);
        let groups =
            counter_groups::group(&sources, &vec![level; sources.len()], &extents, &|_, _| {
                true
            });
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members, [0]);
        assert_eq!(groups[0].anchor, eng_anchor, "height {height}");
        assert!(report.unplaced_task_forces.is_empty());
    }
}

/// Runs `body` with a port context over `forces`, every one with default engagement
/// operands, viewed by the default vision (slot 0).
fn with_port_context<R>(
    forces: &[TaskForceRow],
    body: impl FnOnce(&mut NavalContext<'_>) -> R,
) -> R {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 1,
        tables: std::array::from_fn(|_| TableView::Unavailable {
            reason: "not needed".into(),
        })
        .map(Into::into),
        metrics: Metrics::default(),
    };
    *frame.tables[TableId::TaskForceCounterInputs.index()] =
        TableView::Complete(TableRows::TaskForceCounterInputs(
            forces
                .iter()
                .map(|force| TaskForceCounterInputRow {
                    task_force: force.id,
                    ..Default::default()
                })
                .collect(),
        ));
    let native = NativeInputs::read(&frame).unwrap();
    let vision = Vision::default();
    let country_ids = BTreeMap::new();
    let mut context = NavalContext {
        type_tints: &type_tint::Index::default(),
        vision: &vision,
        native: &native,
        ship_inputs: None,
        aggression: &[1],
        ship_task_forces: BTreeMap::new(),
        ships_complete: true,
        ships: BTreeMap::new(),
        country_ids: &country_ids,
        ideology: &|_| Ok(None),
        missions: BTreeMap::new(),
        mission_skips: Default::default(),
        forces,
        forces_complete: true,
        coarse: false,
    };
    body(&mut context)
}

fn port_unit(force: &TaskForceRow, slot: u32) -> NavalUnit<'_> {
    NavalUnit {
        force,
        ordinal: 0,
        slot,
        logical: Some(slot),
        flag_owner: 0,
        sight: Sight::Full,
        byte: 250,
        ships: 1,
        position: None,
        strength: 0,
    }
}

/// The political map's coarse sorter the game has no mission tie-break and the sort is
/// stable: three otherwise equal own task forces in one fleet keep their roster missions
/// `[1, 0, 1]`, so the port item's first and last missions agree and its mission icon draws.
#[test]
fn the_port_sort_keeps_roster_order_between_equal_missions() {
    let forces: Vec<TaskForceRow> = [1, 0, 1]
        .into_iter()
        .enumerate()
        .map(|(index, mission)| TaskForceRow {
            id: ((index as u64 + 1) << 32) | 51,
            fleet: (1 << 32) | 61,
            mission,
            ..Default::default()
        })
        .collect();
    with_port_context(&forces, |context| {
        let mut units: Vec<NavalUnit<'_>> =
            forces.iter().map(|force| port_unit(force, 0)).collect();
        sort_port_units(&mut units, context).unwrap();
        let missions: Vec<u32> = units.iter().map(|unit| unit.force.mission).collect();
        assert_eq!(missions, [1, 0, 1]);
        let item = context.flotilla(&units.iter().collect::<Vec<_>>(), true, false);
        assert_eq!(item.mission, Some(1));
    });
}

/// A camera that hides the unit counters reads no naval table: an hour whose `Ships` table
/// holds another table's rows builds no context and no error, while a shown hour meets the
/// corrupt table.
#[test]
fn a_hidden_unit_pass_reads_no_naval_table() {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 1,
        tables: std::array::from_fn(|_| TableView::Unavailable {
            reason: "not needed".into(),
        })
        .map(Into::into),
        metrics: Metrics::default(),
    };
    *frame.tables[TableId::Ships.index()] = TableView::Complete(TableRows::TaskForces(Vec::new()));
    let native = NativeInputs::read(&frame).unwrap();
    let vision = Vision::default();
    let country_ids = BTreeMap::new();
    let type_tints = type_tint::Index::default();
    let sources = NavalSources {
        type_tints: &type_tints,
        equipment: Err("unreached variant content"),
        vision: &vision,
        native: &native,
        country_ids: &country_ids,
        ideology: &|_| Ok(None),
        aggression: &[1],
        coarse: false,
    };
    assert!(
        NavalContext::read(false, &frame, sources)
            .unwrap()
            .is_none()
    );
    let error = NavalContext::read(true, &frame, sources).err().unwrap();
    assert!(
        error.to_string().contains("another table's rows"),
        "{error}"
    );
}

/// Subtract two fleet handles' words in `i32` and the game sorts on whatever
/// order that gives: a roster of 5,000 task forces whose fleet low words wrap sorts as the
/// game's `std::stable_sort` does, own task forces first, with nothing refused.
#[test]
fn a_5000_task_force_port_roster_with_wrapping_fleets_sorts_as_the_game_does() {
    let forces: Vec<TaskForceRow> = (1..=5_000_u64)
        .map(|id| TaskForceRow {
            id: (id << 32) | 51,
            fleet: ((id % 40 + 1) << 32) | 61,
            mission: (id % 3) as u32,
            ..Default::default()
        })
        // Own (its id is not a multiple of three), with fleet low words whose differences
        // from the others' 61 and from each other wrap.
        .chain(
            [(5_002, i32::MIN), (5_003, i32::MAX), (5_004, 0)].map(|(id, low)| TaskForceRow {
                id: (id << 32) | 51,
                fleet: (9 << 32) | u64::from(low as u32),
                ..Default::default()
            }),
        )
        .collect();
    // The default vision views as slot 0; every third task force is foreign.
    let unit = |force| port_unit(force, u32::from((force.id >> 32) % 3 == 0));
    with_port_context(&forces, |context| {
        let mut units: Vec<NavalUnit<'_>> = forces.iter().map(unit).collect();
        sort_port_units(&mut units, context).unwrap();
        // Own task forces sort first, and every task force is kept once.
        assert!(units.windows(2).all(|pair| pair[0].slot <= pair[1].slot));
        let mut ids: Vec<u64> = units.iter().map(|unit| unit.force.id).collect();
        let mut expected: Vec<u64> = forces.iter().map(|force| force.id).collect();
        expected.sort_unstable();
        ids.sort_unstable();
        assert_eq!(ids, expected);
    });
}

/// The comparator wraps as the game's does: low word `i32::MIN` precedes 0, 0 precedes
/// `i32::MAX`, and `i32::MAX` precedes `i32::MIN`; three such own task forces keep the
/// order the game's insertion sort gives them.
#[test]
fn the_port_comparator_wraps_and_the_insertion_sort_orders_the_cycle() {
    let key = |low: i32| PortSortKey {
        own: true,
        full: true,
        refused: false,
        reserve: false,
        fleet: (1 << 32) | u64::from(low as u32),
    };
    let (bottom, middle, top) = (key(i32::MIN), key(0), key(i32::MAX));
    assert!(port_sort_compare(&bottom, &middle) < 0);
    assert!(port_sort_compare(&middle, &top) < 0);
    assert!(port_sort_compare(&top, &bottom) < 0);
    // Equal low words compare the high words the same way.
    use crate::map::naval_combat_grid::fleet_difference;
    assert_eq!(fleet_difference(7 << 32, 5 << 32), 2);
    assert_eq!(
        fleet_difference(u64::from(i32::MIN as u32) << 32, 1 << 32),
        i32::MAX
    );
    let forces: Vec<TaskForceRow> = [0, i32::MAX, i32::MIN]
        .into_iter()
        .enumerate()
        .map(|(index, low)| TaskForceRow {
            id: ((index as u64 + 1) << 32) | 51,
            fleet: (1 << 32) | u64::from(low as u32),
            ..Default::default()
        })
        .collect();
    with_port_context(&forces, |context| {
        let mut units: Vec<NavalUnit<'_>> =
            forces.iter().map(|force| port_unit(force, 0)).collect();
        sort_port_units(&mut units, context).unwrap();
        let lows: Vec<i32> = units
            .iter()
            .map(|unit| unit.force.fleet as u32 as i32)
            .collect();
        // `i32::MAX` stays after 0; `i32::MIN` precedes the first (0) and goes to the front.
        assert_eq!(lows, [i32::MIN, 0, i32::MAX]);
    });
}

/// S4: a task force at a naval base that the port pass drops for an operand the hour lacks
/// (its ships not in an incomplete Ships census, its route row unrecorded, its roster owner
/// unlinked) leaves the base's roster undecided rather than dropping it from the roster; one
/// the game itself leaves out (no ships, not in the occupant array, under way) is out.
#[test]
fn a_port_force_dropped_for_a_missing_operand_leaves_its_roster_undecided() {
    let force = TaskForceRow {
        id: 7,
        province: 12,
        fleet: 3,
        roster_owner: 1,
        ..Default::default()
    };
    let forces = [force.clone()];
    let order =
        counter_order::RecordedOrder::from_rows(&[replay_data::rows::ProvinceUnitOrderRow {
            province: 12,
            mixed: Vec::new(),
            naval: vec![7],
        }]);
    let owner = |raw: u32| (raw < 100).then_some(raw);
    let unlinked = |_: u32| None;
    with_port_context(&forces, |context| {
        context.ships = BTreeMap::from([(7, (2, 0))]);
        // What the admission decides: in at its ordinal, out, or undecided by why.
        let admit = |context: &NavalContext<'_>,
                     docked: Result<bool, &'static str>,
                     owner: &dyn Fn(u32) -> Option<u32>| {
            match port_admission(&forces[0], context, &order, || docked, &owner) {
                PortAdmission::In(ordinal, _) => format!("in {ordinal}"),
                PortAdmission::Out => "out".to_owned(),
                PortAdmission::Undecided(why) => why.to_owned(),
            }
        };
        assert_eq!(admit(context, Ok(true), &owner), "in 0");
        assert_eq!(admit(context, Ok(false), &owner), "out");
        assert_eq!(
            admit(context, Err("task-force motion row absent"), &owner),
            "task-force motion row absent"
        );
        assert_eq!(admit(context, Ok(true), &unlinked), PORT_OWNER_UNLINKED);
        context.ships.clear();
        assert_eq!(admit(context, Ok(true), &owner), "out");
        context.ships_complete = false;
        assert_eq!(admit(context, Ok(true), &owner), PORT_SHIPS_UNRECORDED);
    });
}

/// A complete census, or an occupant array whose every task force is recorded,
/// settles port membership. An unavailable reached member leaves it unknown.
#[test]
fn a_port_force_the_incomplete_census_lacks_leaves_its_roster_undecided() {
    let recorded = TaskForceRow {
        id: 7,
        province: 12,
        fleet: 3,
        ..Default::default()
    };
    let forces = [recorded];
    let order = counter_order::RecordedOrder::from_rows(&[
        replay_data::rows::ProvinceUnitOrderRow {
            province: 12,
            mixed: Vec::new(),
            naval: vec![7, 8],
        },
        replay_data::rows::ProvinceUnitOrderRow {
            province: 13,
            mixed: Vec::new(),
            naval: vec![7],
        },
        replay_data::rows::ProvinceUnitOrderRow {
            province: 14,
            mixed: Vec::new(),
            naval: vec![9],
        },
    ]);
    // Province 14 holds no naval base.
    let bases = BTreeSet::from([12, 13]);
    with_port_context(&forces, |context| {
        assert!(unrecorded_port_forces(&bases, &order, context).is_empty());
        context.forces_complete = false;
        assert_eq!(
            unrecorded_port_forces(&bases, &order, context),
            [(12, vec![8])]
        );
    });
}
