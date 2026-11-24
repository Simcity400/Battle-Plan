use super::NavalLinks;
use super::{
    Allegiance, Arc, BTreeMap, BTreeSet, CounterDefines, CounterFrame, Countries, DecodedFrame,
    NativeInputs, NavalContext, NavalSources, NavalTables, NavalUnit, PORT_FORCE_GONE,
    PORT_OWNER_UNLINKED, PORT_SHIPS_UNRECORDED, PortGrouping, PortPlans, PortSortKey, PortTables,
    PriorityTable, ShipInputs, Sight, Visibility, Vision, counter_order, native, naval_members,
    naval_regional, port_grouping, port_items, port_runs, port_sort_compare, port_undecided,
    sort_port_units, type_tint, unrecorded_port_forces,
};
use viewer_counter_kit::CounterColorMode;
use viewer_foundation::unit_rules::DEFENDING;
use viewer_native_gui::native_windows::MAPICONS_GUI;
use viewer_runtime::unit_handle::Unit;
use replay_data::{
    rows::TableId, rows::TableRows, rows::TaskForceCounterInputRow, rows::TaskForceRow,
};
use replay_frame::{frame::Metrics, frame::TableView};
use viewer_foundation::unit_rules::ATTACKING;

#[test]
fn combat_task_forces_never_offer_sea_counter_sources() {
    let skips = viewer_unit_reads::task_force_visual::MissionSkips::fixture([(7, (0, 0))], []);
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

#[test]
fn combat_task_forces_leave_sea_group_anchors_unchanged_at_both_heights() {
    use replay_data::{rows::ShipCounterInputs, rows::ShipModelRow, rows::ShipRow};
    use {
        viewer_counter_kit::counter_groups, viewer_counter_kit::counter_groups::Extents,
        viewer_counter_kit::counter_groups::GroupDefines, viewer_counter_kit::counter_groups::Icon,
        viewer_counter_kit::counter_groups::Level,
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

    let skips = viewer_unit_reads::task_force_visual::MissionSkips::fixture(
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
                track: None,
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

#[test]
fn a_port_roster_with_wrapping_fleets_sorts_as_the_game_does() {
    let forces: Vec<TaskForceRow> = (1..=100_u64)
        .map(|id| TaskForceRow {
            id: (id << 32) | 51,
            fleet: ((id % 40 + 1) << 32) | 61,
            mission: (id % 3) as u32,
            ..Default::default()
        })
        .chain(
            [(102, i32::MIN), (103, i32::MAX), (104, 0)].map(|(id, low)| TaskForceRow {
                id: (id << 32) | 51,
                fleet: (9 << 32) | u64::from(low as u32),
                ..Default::default()
            }),
        )
        .collect();
    let unit = |force| port_unit(force, u32::from((force.id >> 32) % 3 == 0));
    with_port_context(&forces, |context| {
        let mut units: Vec<NavalUnit<'_>> = forces.iter().map(unit).collect();
        sort_port_units(&mut units, context).unwrap();
        assert!(units.windows(2).all(|pair| pair[0].slot <= pair[1].slot));
        let mut ids: Vec<u64> = units.iter().map(|unit| unit.force.id).collect();
        let mut expected: Vec<u64> = forces.iter().map(|force| force.id).collect();
        expected.sort_unstable();
        ids.sort_unstable();
        assert_eq!(ids, expected);
    });
}

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
    use viewer_counter_kit::naval_combat_grid::fleet_difference;
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
        assert_eq!(lows, [i32::MIN, 0, i32::MAX]);
    });
}

#[test]
fn a_port_force_dropped_for_a_missing_operand_leaves_its_roster_undecided() {
    let force = TaskForceRow {
        id: 7,
        province: 12,
        fleet: 3,
        roster_owner: 1,
        ..Default::default()
    };
    let forces = [force];
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

#[test]
fn an_incomplete_ship_census_leaves_task_force_counts_unknown() {
    let force = TaskForceRow {
        id: 7,
        province: 12,
        fleet: 3,
        roster_owner: 1,
        ..Default::default()
    };
    let forces = [force];
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
            port_runs(grouping, &keys, |_| (0, Some(Allegiance::Ours)))
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
        let far = port_grouping(&defines, strategic, 420.0);
        assert_eq!(prepare(far, &BTreeSet::new()), [vec![2, 1, 3, 4]]);
        assert_eq!(
            prepare(far, &BTreeSet::from([Unit::TaskForce(1)])),
            [vec![1], vec![2, 3, 4]]
        );
        let political = port_grouping(&defines, Visibility::POLITICAL, 80.0);
        assert_eq!(prepare(political, &BTreeSet::new()), [vec![1, 2, 3, 4]]);
        assert!(!port_grouping(&defines, Visibility::POLITICAL, 100.0).cleared);
        assert!(port_grouping(&defines, Visibility::POLITICAL, 100.01).cleared);
        assert!(
            !port_grouping(&defines, strategic, 701.0).cleared,
            "coarse strategic ports retain their grids; only political mode clears boxes"
        );
        let other = Visibility {
            priorities: PriorityTable::Default,
            ..strategic
        };
        assert!(!port_grouping(&defines, other, 310.0).allegiance);
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
    use viewer_foundation::test_support::{test_frame, test_put};
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

#[test]
fn a_port_roster_drawn_over_another_hours_task_forces_joins_them_by_handle() {
    content_reader_script::test_game::enter();
    let guis = native::Guis::new(vec![(
        MAPICONS_GUI.into(),
        native::read(
            br#"guiTypes={
        containerWindowType={name=unit_counter_navy size={width=81 height=29}
            buttonType={name=count_bg spriteType=GFX_synthetic_plate}
            iconType={name=counter_overlay spriteType=GFX_synthetic_overlay}}
        containerWindowType={name=naval_combat_mapicon}
        containerWindowType={name=unit_counter size={width=81 height=29}
            buttonType={name=count_bg spriteType=GFX_synthetic_plate}
            iconType={name=counter_overlay spriteType=GFX_synthetic_overlay}}
    }"#,
        ),
    )]);
    viewer_counter_kit::fixtures::with_counter_inputs_fixture(
        guis,
        |_| {},
        |content| {
            content.defines.coarse_distance = [100., 160.];
            content.defines.groups.strategic_distance = 200.;
        },
        |mut inputs| {
            inputs.settings.color_mode = CounterColorMode::Allegiance;
            inputs.visibility.political_mode = false;
            inputs.camera.height = 50.;
            let forces: Vec<TaskForceRow> = (1..=3)
                .map(|id| TaskForceRow { id, ..Default::default() })
                .collect();
            let countries = Arc::new(Countries {
                sides: [(0, Allegiance::Ours)].into(),
                ..Default::default()
            });
            let vision = Arc::new(Vision::default());
            let frame = DecodedFrame {
                branch: 1,
                hour: 1,
                metrics: Default::default(),
                tables: TableId::ALL.map(|table| TableView::Complete(TableRows::empty(table)).into()),
            };
            let ports = PortTables {
                rosters: [(7, vec![(1, 0), (2, 1), (3, 2)])].into(),
                ..Default::default()
            };
            let items = |held: usize| {
                with_port_context(&forces[..held], |context| {
                    context.ships = Arc::new(forces.iter().map(|force| (force.id, (1, 100_000))).collect());
                    let tables = NavalLinks {
                        countries: &countries,
                        vision: &vision,
                        order: None,
                        task_forces: &Default::default(),
                    };
                    let mut out = CounterFrame::<(), crate::base_windows::PreparedBases, (), crate::naval_windows::Plan>::default();
                    let ports = port_items(&inputs, &frame, &|raw| Some(raw), &tables, context, &ports, None, &mut out).unwrap();
                    (ports.get(&7).unwrap().clone(), out.report.unplaced_task_forces.clone())
                })
            };
            let (whole, refused) = items(3);
            assert!(whole.is_ok_and(|items| !items.is_empty()), "every task force of the roster joins the rows that hold it");
            assert!(refused.is_empty(), "nothing refused while the rows hold them all");
            let (gone, refused) = items(1);
            assert_eq!(gone.err().as_deref(), Some(port_undecided(PORT_FORCE_GONE).as_ref()), "a task force the drawn rows do not hold refuses the base's items by name");
            assert_eq!(refused, [(PORT_FORCE_GONE, BTreeSet::from([2]))].into(), "named with the handle the drawn rows lack");
        },
    );
}

#[test]
fn retained_port_plans_preserve_ordered_calls_across_thresholds_and_roster_occurrences() {
    content_reader_script::test_game::enter();
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
    viewer_counter_kit::fixtures::with_counter_inputs_fixture(
        guis,
        |_| {},
        |content| {
            content.defines.coarse_distance = [100., 160.];
            content.defines.groups.strategic_distance = 200.;
        },
        |mut inputs| {
            use viewer_counter_kit::plan_probe;
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
                let tables = NavalLinks {
                    countries: &countries,
                    vision: &vision,
                    order: None,
                    task_forces: &Default::default(),
                };
                let frame = DecodedFrame {
                    branch: 1,
                    hour: 1,
                    metrics: Default::default(),
                    tables: TableId::ALL
                        .map(|table| TableView::Complete(TableRows::empty(table)).into()),
                };
                let mut ports = PortTables {
                    rosters: [(7, vec![(3, 0), (1, 1), (3, 2), (2, 3)])].into(),
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
                        ports.rosters.get_mut(&7).unwrap().rotate_left(1);
                    }
                    let mut a = CounterFrame::<(), crate::base_windows::PreparedBases, (), crate::naval_windows::Plan>::default();
                    let mut b = CounterFrame::<(), crate::base_windows::PreparedBases, (), crate::naval_windows::Plan>::default();
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


#[test]
fn port_grouping_keeps_distinct_foreign_owners_apart_when_relations_are_unavailable() {
    content_reader_script::test_game::enter();
    let guis = native::Guis::new(vec![(
        MAPICONS_GUI.into(),
        native::read(
            br#"guiTypes={
        containerWindowType={name=unit_counter_navy size={width=81 height=29}
            buttonType={name=count_bg spriteType=GFX_synthetic_plate}
            iconType={name=counter_overlay spriteType=GFX_synthetic_overlay}}
        containerWindowType={name=naval_combat_mapicon}
        containerWindowType={name=unit_counter size={width=81 height=29}
            buttonType={name=count_bg spriteType=GFX_synthetic_plate}
            iconType={name=counter_overlay spriteType=GFX_synthetic_overlay}}
    }"#,
        ),
    )]);
    viewer_counter_kit::fixtures::with_counter_inputs_fixture(
        guis,
        |_| {},
        |content| {
            content.defines.coarse_distance = [100., 160.];
            content.defines.groups.strategic_distance = 200.;
        },
        |mut inputs| {
            inputs.settings.color_mode = CounterColorMode::Allegiance;
            inputs.visibility.political_mode = true;
            inputs.camera.height = 50.;
            let forces = vec![
                TaskForceRow { id: 1, roster_owner: 10, ..Default::default() },
                TaskForceRow { id: 2, roster_owner: 10, ..Default::default() },
                TaskForceRow { id: 3, roster_owner: 20, ..Default::default() },
            ];
            let countries = Arc::new(Countries {
                sides: [(0, Allegiance::Ours)].into(),
                ..Default::default()
            });
            let vision = Arc::new(Vision::default());
            let frame = DecodedFrame {
                branch: 1,
                hour: 1,
                metrics: Default::default(),
                tables: TableId::ALL.map(|table| TableView::Complete(TableRows::empty(table)).into()),
            };
            let ports = PortTables {
                rosters: [(7, vec![(1, 0), (2, 1), (3, 2)])].into(),
                ..Default::default()
            };
            with_port_context(&forces, |context| {
                context.ships = Arc::new(forces.iter().map(|force| (force.id, (1, 100_000))).collect());
                let tables = NavalLinks {
                    countries: &countries,
                    vision: &vision,
                    order: None,
                    task_forces: &Default::default(),
                };
                let mut out = CounterFrame::<(), crate::base_windows::PreparedBases, (), crate::naval_windows::Plan>::default();
                let items = port_items(&inputs, &frame, &|raw| Some(raw), &tables, context, &ports, None, &mut out)
                    .unwrap()
                    .remove(&7)
                    .unwrap()
                    .unwrap();
                let grouped: Vec<Vec<u64>> = items
                    .into_iter()
                    .map(|item| item.selectable.map_or_else(Vec::new, |(_, members)| members.to_vec()))
                    .collect();
                assert_eq!(grouped, vec![vec![1, 2], vec![3]]);

                inputs.visibility.political_mode = false;
                let mut out = CounterFrame::<(), crate::base_windows::PreparedBases, (), crate::naval_windows::Plan>::default();
                let non_allegiance_items = port_items(&inputs, &frame, &|raw| Some(raw), &tables, context, &ports, None, &mut out)
                    .unwrap()
                    .remove(&7)
                    .unwrap()
                    .unwrap();
                let non_allegiance_grouped: Vec<Vec<u64>> = non_allegiance_items
                    .into_iter()
                    .map(|item| item.selectable.map_or_else(Vec::new, |(_, members)| members.to_vec()))
                    .collect();
                assert_eq!(non_allegiance_grouped, vec![vec![1, 2, 3]]);
            });
        },
    );
}

#[test]
fn non_finite_fleet_color_refuses_group_marker_by_name() {
    use replay_data::rows::FleetCounterInputRow;
    let check = |fleet_rows: Vec<FleetCounterInputRow>, port: bool| {
        let force = TaskForceRow {
            id: 1,
            fleet: 42,
            mission: 1,
            ..Default::default()
        };
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|_| TableView::Unavailable {
                reason: "not needed".into(),
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::FleetCounterInputs.index()] =
            TableView::Complete(TableRows::FleetCounterInputs(fleet_rows));
        let native = NativeInputs::read(&frame).unwrap();
        let vision = Vision::default();
        let country_ids = BTreeMap::new();
        let forces = [force];
        let context = NavalContext {
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
            missions: Arc::new(BTreeMap::from([(1, 1)])),
            mission_skips: Default::default(),
            forces: &forces,
            forces_complete: true,
        };
        let unit = port_unit(&forces[0], 0);
        context.flotilla(&[&unit], port, false).marker
    };
    let valid = [0.2f32, 0.4, 0.6, 1.0];
    let valid_row = FleetCounterInputRow {
        fleet: 42,
        task_forces: vec![1],
        red: valid[0].to_bits(),
        green: valid[1].to_bits(),
        blue: valid[2].to_bits(),
        alpha: valid[3].to_bits(),
        ..Default::default()
    };
    assert_eq!(check(vec![valid_row.clone()], false), Ok(Some(valid)));
    assert_eq!(check(vec![valid_row], true), Ok(Some(valid)));
    let absent = check(Vec::new(), true).unwrap_err();
    assert!(absent.contains("is not recorded"), "{absent}");
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for channel in 0..4 {
            let mut raw = valid.map(f32::to_bits);
            raw[channel] = bad.to_bits();
            let row = FleetCounterInputRow {
                fleet: 42,
                task_forces: vec![1],
                red: raw[0],
                green: raw[1],
                blue: raw[2],
                alpha: raw[3],
                ..Default::default()
            };
            for port in [false, true] {
                let err = check(vec![row.clone()], port)
                    .expect_err("non-finite fleet colour must be refused");
                assert!(err.contains("non-finite"), "{err}");
                assert!(err.contains("42"), "{err}");
            }
        }
    }
}
