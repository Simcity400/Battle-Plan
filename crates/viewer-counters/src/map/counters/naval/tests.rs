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
            let joins = naval_members::Fake { kind, region: Some(3), skips: skips.clone(), ..naval_members::Fake::new(&force) };
            assert_eq!(
                matches!(joins.sea(), naval_members::Sea::Source { .. }),
                combat_references <= 0,
                "combat references {combat_references}, kind {kind}"
            );
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
        let sources: Vec<_> = [(&eng, 0, eng_anchor), (&pol, ATTACKING, pol_anchor)]
            .into_iter()
            .filter(|(force, kind, _)| {
                let joins = naval_members::Fake { kind: *kind, region: Some(206), skips: skips.clone(), ..naval_members::Fake::new(force) };
                matches!(joins.sea(), naval_members::Sea::Source { .. })
            })
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
        ship_task_forces: Arc::new(BTreeMap::new()),
        ships_complete: true,
        ships: Arc::new(BTreeMap::new()),
        country_ids: &country_ids,
        ideology: &|_| Ok(None),
        missions: Arc::new(BTreeMap::new()),
        mission_skips: Default::default(),
        forces,
        forces_complete: true,
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
        ships: Some(1),
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
    };
    assert!(
        NavalContext::read(false, &frame, sources)
            .unwrap()
            .is_none()
    );
    let error = NavalContext::read(true, &frame, sources).err().unwrap();
    assert!(
        error.to_string().contains("ships holds rows of another kind"),
        "{error}"
    );
}

/// Subtract two fleet handles' words in `i32` and the game sorts on whatever
/// order that gives: a roster of 100 task forces whose fleet low words wrap sorts as the
/// game's `std::stable_sort` does, own task forces first, with nothing refused.
#[test]
fn a_port_roster_with_wrapping_fleets_sorts_as_the_game_does() {
    let forces: Vec<TaskForceRow> = (1..=100_u64)
        .map(|id| TaskForceRow {
            id: (id << 32) | 51,
            fleet: ((id % 40 + 1) << 32) | 61,
            mission: (id % 3) as u32,
            ..Default::default()
        })
        // Own (its id is not a multiple of three), with fleet low words whose differences
        // from the others' 61 and from each other wrap.
        .chain(
            [(102, i32::MIN), (103, i32::MAX), (104, 0)].map(|(id, low)| TaskForceRow {
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
        selected: false,
        handle: 0,
        mission: 0,
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
    let joins = |docked: Result<bool, &'static str>, linked: bool, ships: &[(u64, (usize, i64))], complete: bool| naval_members::Fake {
        order: Some(order.clone()),
        ships: ships.iter().copied().collect(),
        ships_complete: complete,
        docked,
        linked,
        ..naval_members::Fake::new(&forces[0])
    };
    // What the admission decides: in at its ordinal, out, or undecided by why.
    let admit = |joins: naval_members::Fake<'_>| match joins.port() {
        naval_members::Port::In(ordinal) => format!("in {ordinal}"),
        naval_members::Port::Out => "out".to_owned(),
        naval_members::Port::Undecided(why) => why.to_owned(),
    };
    let ships = [(7, (2, 0))];
    assert_eq!(admit(joins(Ok(true), true, &ships, true)), "in 0");
    assert_eq!(admit(joins(Ok(false), true, &ships, true)), "out");
    assert_eq!(admit(joins(Err("task-force motion row absent"), true, &ships, true)), "task-force motion row absent");
    assert_eq!(admit(joins(Ok(true), false, &ships, true)), PORT_OWNER_UNLINKED);
    assert_eq!(admit(joins(Ok(true), true, &[], true)), "out");
    assert_eq!(admit(joins(Ok(true), true, &[], false)), PORT_SHIPS_UNRECORDED);
}

/// A Ships census that may lack a task force's ships leaves its count, and every sum over it,
/// unknown rather than zero or the part the hour holds.
#[test]
fn an_incomplete_ship_census_leaves_task_force_counts_unknown() {
    let force = TaskForceRow {
        id: 7,
        province: 12,
        fleet: 3,
        roster_owner: 1,
        ..Default::default()
    };
    let forces = [force.clone()];
    let owner = |raw: u32| (raw < 100).then_some(raw);
    with_port_context(&forces, |context| {
        context.ships = Arc::new(BTreeMap::from([(7, (2, 0))]));
        let counts = |context: &NavalContext<'_>| {
            let unit = context.unit(&forces[0], &owner, 0, None).unwrap();
            let item = context.flotilla(&[&unit], true, false).item.unwrap();
            (unit.ships, item.ships)
        };
        assert_eq!(counts(&*context), (Some(2), Some(2)));
        context.ships_complete = false;
        assert_eq!(
            counts(&*context),
            (None, None),
            "a truncated census may hold a part"
        );
        Arc::make_mut(&mut context.ships).clear();
        assert_eq!(
            counts(&*context),
            (None, None),
            "an absent row is not an empty force"
        );
        context.ships_complete = true;
        assert_eq!(counts(&*context), (Some(0), Some(0)));
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

/// Strategic views below the content region-group distance must not merge all
/// own fleets into the political map's single allegiance plate.
#[test]
fn strategic_port_frames_keep_fleet_mission_and_selected_splits() {
    let mut defines = CounterDefines::VANILLA;
    defines.coarse_distance = [100.0, 700.0];
    defines.groups.province_distance = 70.0;
    defines.groups.state_distance = 150.0;
    defines.groups.strategic_distance = 420.0;
    let strategic = Visibility {
        political_mode: false,
        strategic_grouping: true,
        priorities: PriorityTable::StrategicAir,
        ..Visibility::POLITICAL
    };
    let forces: Vec<_> = [(1, 11, 1), (2, 11, 0), (3, 11, 1), (4, 22, 1)]
        .map(|(id, fleet, mission)| TaskForceRow {
            id,
            fleet,
            mission,
            ..Default::default()
        })
        .into();
    with_port_context(&forces, |context| {
        let prepare = |grouping: PortGrouping, selected: &BTreeSet<Unit>| {
            let mut units: Vec<_> = forces.iter().map(|force| port_unit(force, 0)).collect();
            let keys =
                naval_regional::sort_own(&mut units, selected, context, grouping.coarse).unwrap();
            port_runs(grouping, &keys, |_| Allegiance::Ours)
                .into_iter()
                .map(|run| units[run].iter().map(|u| u.force.id).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        };
        let detail = port_grouping(&defines, strategic, 310.0);
        assert_eq!(
            detail,
            PortGrouping {
                cleared: false,
                coarse: false,
                allegiance: false
            }
        );
        assert_eq!(
            prepare(detail, &BTreeSet::new()),
            [vec![2], vec![1, 3], vec![4]]
        );
        assert_eq!(
            prepare(
                detail,
                &BTreeSet::from([Unit::TaskForce(1), Unit::TaskForce(3)])
            ),
            [vec![1], vec![3], vec![2], vec![4]]
        );
        // Level equality selects allegiance grouping; selected own members remain a separate run.
        let far = port_grouping(&defines, strategic, 420.0);
        assert_eq!(prepare(far, &BTreeSet::new()), [vec![2, 1, 3, 4]]);
        assert_eq!(
            prepare(far, &BTreeSet::from([Unit::TaskForce(1)])),
            [vec![1], vec![2, 3, 4]]
        );
        // Political mode keeps its coarse stable order even at a close camera.
        let political = port_grouping(&defines, Visibility::POLITICAL, 80.0);
        assert_eq!(prepare(political, &BTreeSet::new()), [vec![1, 2, 3, 4]]);
        assert!(!port_grouping(&defines, Visibility::POLITICAL, 100.0).cleared);
        assert!(port_grouping(&defines, Visibility::POLITICAL, 100.01).cleared);
        assert!(
            !port_grouping(&defines, strategic, 701.0).cleared,
            "coarse strategic ports retain their grids; only political mode clears boxes"
        );
        // The priority table alone does not turn another mode into political mode.
        let other = Visibility {
            priorities: PriorityTable::Default,
            ..strategic
        };
        assert!(!port_grouping(&defines, other, 310.0).allegiance);
        // Coarse and geographical group cutoffs are independent and content-defined.
        defines.coarse_distance[1] = 250.0;
        let coarse = port_grouping(&defines, strategic, 310.0);
        assert_eq!(
            coarse,
            PortGrouping {
                cleared: false,
                coarse: true,
                allegiance: false
            }
        );
        assert_eq!(prepare(coarse, &BTreeSet::new()), [vec![1, 2, 3], vec![4]]);
    });
}

#[test]
fn cached_naval_operands_equal_fresh_joins_and_share_the_model_column() {
    use replay_data::rows::{ShipCounterInputs, ShipModelRow, ShipRow};
    use viewer_foundation::map::test_support::{test_frame, test_put};
    let mut frame = test_frame(7);
    test_put(
        &mut frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: 10,
            mission: 3,
            ..Default::default()
        }]),
    );
    test_put(
        &mut frame,
        TableRows::Ships(vec![
            ShipRow {
                id: 2,
                task_force: 10,
                strength: 30,
                organisation: 20,
                ..Default::default()
            },
            ShipRow {
                id: 1,
                task_force: 10,
                strength: 50,
                organisation: 40,
                ..Default::default()
            },
        ]),
    );
    test_put(
        &mut frame,
        TableRows::ShipModels(
            [1, 2]
                .map(|ship| ShipModelRow {
                    ship,
                    counter: Some(ShipCounterInputs {
                        roster_ordinal: 0,
                        maximum_strength: 100,
                        maximum_organisation: 100,
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .into(),
        ),
    );
    let vision = Vision::sharing(1, []);
    let cached = NavalTables::read(&frame, &vision).unwrap();
    let models = &cached.ship_inputs.as_ref().unwrap().models;
    assert!(replay_frame::frame::SharedView::ptr_eq(
        models,
        &frame.tables[TableId::ShipModels.index()]
    ));
    let sources = NavalSources {
        type_tints: &Default::default(),
        equipment: Err("unused equipment"),
        vision: &vision,
        native: &Default::default(),
        country_ids: &Default::default(),
        aggression: &[],
        ideology: &|_| Ok(None),
    };
    let prepared = NavalContext::cached(&frame, sources, &cached).unwrap();
    let fresh = NavalContext::build(&frame, sources).unwrap();
    assert_eq!(prepared.ships, fresh.ships);
    assert_eq!(prepared.ship_task_forces, fresh.ship_task_forces);
    assert_eq!(prepared.missions, fresh.missions);
    assert!(Arc::ptr_eq(&prepared.ships, &cached.ships));
    let signature = |inputs: &ShipInputs<'_>| {
        inputs
            .hulls(10)
            .map(|hull| (hull.model.ship, hull.strength))
            .collect::<Vec<_>>()
    };
    let prepared = prepared.ship_inputs.as_ref().unwrap();
    let fresh = fresh.ship_inputs.as_ref().unwrap();
    assert_eq!(
        signature(prepared),
        [(2, 30), (1, 50)],
        "equal ordinals keep Ships row order"
    );
    assert_eq!(signature(prepared), signature(fresh));
    assert_eq!(prepared.ratio(10), fresh.ratio(10));
    assert_eq!(
        prepared.cells(&[10], false).hull,
        fresh.cells(&[10], false).hull
    );
}

/// The semantic memo may skip the sort/fold, never the resource-bearing update generation.
#[test]
fn retained_port_plans_preserve_ordered_calls_across_thresholds_and_roster_occurrences() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let guis = native::Guis::new(vec![(
        MAPICONS_GUI.into(),
        native::read(
            br#"guiTypes={
        containerWindowType={name=unit_counter_navy size={width=81 height=29}
            buttonType={name=count_bg spriteType=GFX_synthetic_plate}
            iconType={name=counter_overlay spriteType=GFX_synthetic_overlay}
            iconType={name=mission_type spriteType=GFX_synthetic_actions}}
        containerWindowType={name=naval_combat_mapicon}
        containerWindowType={name=unit_counter size={width=81 height=29}
            buttonType={name=count_bg spriteType=GFX_synthetic_plate}
            iconType={name=onmap_action_icon spriteType=GFX_synthetic_actions}
            iconType={name=counter_overlay spriteType=GFX_synthetic_overlay}}
    }"#,
        ),
    )]);
    crate::map::counters::tests::with_counter_inputs_fixture(
        guis,
        |_| {},
        |content| {
            content.defines.coarse_distance = [100., 160.];
            content.defines.groups.strategic_distance = 200.;
        },
        |mut inputs| {
            use crate::map::counters::preparation::plan_probe;
            inputs.settings.color_mode = CounterColorMode::Allegiance;
            inputs.visibility.political_mode = false;
            inputs.visibility.strategic_grouping = true;
            let forces = [0, 1, 0].map(|mission| TaskForceRow {
                id: 0,
                mission,
                ..Default::default()
            });
            let forces: Vec<_> = forces
                .into_iter()
                .enumerate()
                .map(|(at, mut row)| {
                    row.id = at as u64 + 1;
                    row
                })
                .collect();
            with_port_context(&forces, |context| {
                context.ships = Arc::new(
                    forces
                        .iter()
                        .map(|force| (force.id, (1, 100_000)))
                        .collect(),
                );
                let countries = Arc::new(Countries {
                    sides: [(0, Allegiance::Ours)].into(),
                    ..Default::default()
                });
                let vision = Arc::new(Vision::default());
                let tables = CounterTables {
                    type_tints: context.type_tints,
                    countries: &countries,
                    icons: &Arc::new(TemplateIcons::default()),
                    status: &Default::default(),
                    movement: &Default::default(),
                    actions: &Default::default(),
                    vision: &vision,
                    order: None,
                    native: context.native,
                    orders: &Default::default(),
                    guns: &Default::default(),
                    abilities: &Default::default(),
                    task_forces: &Default::default(),
                    hq_providers: &Default::default(),
                    portraits: &Default::default(),
                    icon_inputs: &Default::default(),
                };
                let frame = DecodedFrame {
                    branch: 1,
                    hour: 1,
                    metrics: Default::default(),
                    tables: TableId::ALL
                        .map(|table| TableView::Complete(TableRows::empty(table)).into()),
                };
                let mut ports = PortTables {
                    rosters: [(7, vec![(2, 0), (0, 1), (2, 2), (1, 3)])].into(),
                    ..Default::default()
                };
                let mut memo = PortPlans::default();
                let mut hits = 0;
                let mut calls = 0;
                for (step, height) in [50., 50., 159.999, 160., 160., 199.999, 200., 200., 50., 50.]
                    .into_iter()
                    .enumerate()
                {
                    inputs.camera.height = height;
                    inputs.camera.center[0] += 0.125;
                    if step == 4 {
                        // Same set, different order; the repeated handle's occurrences stay distinct.
                        ports.rosters.get_mut(&7).unwrap().rotate_left(1);
                    }
                    let mut a = CounterFrame::default();
                    let mut b = CounterFrame::default();
                    let fresh = port_items(
                        &inputs,
                        &frame,
                        &|raw| Some(raw),
                        &tables,
                        context,
                        &ports,
                        None,
                        &mut a,
                    )
                    .unwrap();
                    let (cached, counts) = plan_probe::collect(|| {
                        port_items(
                            &inputs,
                            &frame,
                            &|raw| Some(raw),
                            &tables,
                            context,
                            &ports,
                            Some(&mut memo),
                            &mut b,
                        )
                        .unwrap()
                    });
                    assert_eq!(fresh, cached, "step {step}");
                    assert_eq!(
                        format!("{fresh:?}"),
                        format!("{cached:?}"),
                        "step {step} signed-zero operands"
                    );
                    assert_eq!(a.report, b.report, "step {step}");
                    assert_eq!(counts.semantic_plans, 1);
                    if step == 0 || step == 4 {
                        assert_eq!(counts.semantic_plan_hits, 0);
                    }
                    if step == 4 {
                        assert_eq!(counts.ordered_group_members_equal, 0);
                    }
                    hits += counts.semantic_plan_hits;
                    let updates = cached.get(&7).unwrap().as_ref().unwrap();
                    for update in updates {
                        assert!(
                            update
                                .selectable
                                .as_ref()
                                .is_some_and(|(_, members)| !members.is_empty())
                        );
                        calls += update.calls.len();
                    }
                }
                assert!(hits > 0);
                assert!(
                    calls > 0,
                    "must compare real setter streams, not only refused/empty plans"
                );
            });
        },
    );
}

/// A view sought back from 14 Sep 1940 to 8 Jun 1940 still holds the counter tables made at the
/// later hour while the rows at the head are the earlier hour's. Its counters lay out the rows
/// their tables were made from, never one hour's port rosters over another hour's task forces,
/// and its retained memory never carries a port plan from one hour's tables to another's.
#[test]
fn counters_laid_out_across_a_seek_back_never_mix_their_tables_hour_with_another() {
    use replay_data::rows::TaskForceCounterInputRow;
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let guis = native::Guis::new(vec![(
        MAPICONS_GUI.into(),
        native::read(
            br#"guiTypes={
        containerWindowType={name=unit_counter_navy size={width=81 height=29}
            buttonType={name=count_bg spriteType=GFX_synthetic_plate}
            iconType={name=counter_overlay spriteType=GFX_synthetic_overlay}
            iconType={name=mission_type spriteType=GFX_synthetic_actions}}
        containerWindowType={name=naval_combat_mapicon}
        containerWindowType={name=naval_base_mapicon size={width=81 height=29}}
        containerWindowType={name=unit_counter size={width=81 height=29}
            buttonType={name=count_bg spriteType=GFX_synthetic_plate}
            iconType={name=onmap_action_icon spriteType=GFX_synthetic_actions}
            iconType={name=counter_overlay spriteType=GFX_synthetic_overlay}}
    }"#,
        ),
    )]);
    crate::map::counters::tests::with_counter_inputs_fixture(
        guis,
        |_| {},
        |content| content.aggression = Arc::new(vec![1]),
        |mut inputs| {
            inputs.visibility.political_mode = false;
            let frame = |hour: i32, forces: &[u64]| {
                let mut frame = DecodedFrame {
                    branch: 1,
                    hour,
                    metrics: Default::default(),
                    tables: TableId::ALL
                        .map(|table| TableView::Complete(TableRows::empty(table)).into()),
                };
                *frame.tables[TableId::TaskForces.index()] = TableView::Complete(TableRows::TaskForces(
                    forces
                        .iter()
                        .map(|&id| TaskForceRow { id, province: 7, roster_owner: 10, logical_owner: 10, ..Default::default() })
                        .collect(),
                ));
                *frame.tables[TableId::TaskForceCounterInputs.index()] = TableView::Complete(TableRows::TaskForceCounterInputs(
                    forces.iter().map(|&task_force| TaskForceCounterInputRow { task_force, ..Default::default() }).collect(),
                ));
                frame
            };
            let tables = |frame: &DecodedFrame, rosters: BTreeMap<u32, Vec<(usize, u32)>>| {
                let moves = viewer_foundation::map::movement_reads::test_support::unnamed_assets();
                let mut tables = hour_tables(
                    frame,
                    inputs.map,
                    Arc::new(Vision::sharing(1, [])),
                    inputs.content,
                    inputs.assets,
                    inputs.equipment,
                    inputs.localisation,
                    &Default::default(),
                    None,
                    None,
                    None,
                    &type_tint::Memo::new(type_tint::TABLES),
                    &MissionMemo::new(),
                    moves.movement_reads(),
                    Err("unreached synthetic naval topology".into()),
                )
                .unwrap();
                tables.links = [(10, 1)].into();
                tables.countries = Arc::new(Countries { sides: [(1, Allegiance::Ours)].into(), ..Default::default() });
                tables.ports = PortTables { rosters, ..Default::default() };
                tables
            };
            // Each port's items, by the task forces each item holds.
            let members = |prepared: &PreparedCounters| -> BTreeMap<u32, Vec<Vec<u64>>> {
                let pass = prepared.fixed.naval.bases.as_ref().expect("the naval base windows are planned");
                let ports = pass.ports.as_ref().expect("the port items are decided").as_ref().expect("and filled");
                ports
                    .iter()
                    .map(|(&province, items)| {
                        let items = items.as_ref().expect("the port's roster is decided");
                        (province, items.iter().map(|item| item.selectable.as_ref().map_or_else(Vec::new, |(_, members)| members.to_vec())).collect())
                    })
                    .collect()
            };
            // Three task forces in port at the later hour; one, the third, at the earlier hour. Port 8's
            // roster holds the first row at both hours, a different task force at each.
            let late = frame(60_800_559, &[1, 2, 3]);
            let early = frame(60_798_213, &[3]);
            let late_tables = tables(&late, [(7, vec![(1, 0), (2, 1)]), (8, vec![(0, 0)])].into());
            let early_tables = tables(&early, [(8, vec![(0, 0)])].into());
            let mut memo = Preparation::default();
            // Played, then sought back: the view lays out the later hour's tables again (art loaded
            // meanwhile) until the earlier hour's arrive, and nothing of their plans is taken over.
            for resources in [0, 1] {
                let played = prepare_retained(&inputs, &late_tables, resources, &mut memo).unwrap();
                assert_eq!(members(&played), [(7, vec![vec![2, 3]]), (8, vec![vec![1]])].into());
            }
            let arrived = prepare_retained(&inputs, &early_tables, 1, &mut memo).unwrap();
            assert_eq!(members(&arrived), [(8, vec![vec![3]])].into());
            let fresh = prepare_cached(&inputs, &early_tables).unwrap();
            assert_eq!(members(&arrived), members(&fresh));
            assert_eq!(arrived.fixed.report, fresh.fixed.report);
        },
    );
}
