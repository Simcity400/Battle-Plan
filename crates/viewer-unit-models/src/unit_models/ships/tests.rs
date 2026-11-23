use super::*;
use std::sync::Arc;
use viewer_gpu::camera::MapDefines;
use viewer_gpu::camera::MapSize;
use viewer_gpu::camera::Placement;
use viewer_gpu::province_map::HeightField;

#[test]
fn shared_animator_clock_preserves_empty_routes_and_names_missing_speed() {
    let mut report = ModelReport::default();
    assert_eq!(animator_min_arc(&[], None, &mut report), Some(0.0));
    assert!(report.selection_errors.is_empty());
    for (speed, expected) in [(0, 0.0), (3, 0.0), (4, 1.0)] {
        let clock = replay_data::rows::ClockRow {
            speed,
            ..Default::default()
        };
        assert_eq!(
            animator_min_arc(&[2], Some(clock.speed), &mut report),
            Some(expected)
        );
        assert_eq!(animator_min_arc(&[], Some(clock.speed), &mut report), Some(0.0));
    }
    assert_eq!(animator_min_arc(&[2], None, &mut report), None);
    assert_eq!(
        report.selection_errors["naval_animator_clock: Clock speed is unavailable"],
        1
    );
}

fn formation(bytes: &[u8]) -> (String, f32) {
    read_formation(bytes, &mut Vec::new()).unwrap()
}

#[test]
fn formation_reader_keeps_native_defaults_and_last_stored_values() {
    assert_eq!(formation(b""), (String::new(), 1.0));
    assert_eq!(
        formation(
            br#"
            naval_formation = first naval_formation = "last"
            naval_formation_scale = .6 naval_formation_scale = invalid
        "#
        ),
        ("last".into(), 0.6)
    );
    assert_eq!(
        formation(b"naval_formation_scale = { naval_formation_scale = 7 } naval_formation = kept"),
        ("kept".into(), 1.0)
    );
}

#[test]
fn formation_reader_uses_the_crt_number_forms() {
    for (text, expected) in [
        ("0x1.8p-1", 0.75),
        (".6suffix", 0.6),
        ("1e-50", 0.0),
        ("0x1.000003p0", 1.000_000_2),
    ] {
        assert_eq!(
            formation(format!("naval_formation_scale = {text}").as_bytes()).1,
            expected
        );
    }
    assert_eq!(
        formation(br#"naval_formation_scale = "  -.5px" naval_formation_scale = 1e"#).1,
        -0.5
    );
}

#[test]
fn formation_reader_replays_lists_variables_unknowns_and_persistent_stop() {
    let mut warnings = Vec::new();
    let parsed = read_formation(
        br#"
        @name = "mod_formation"
        @scale = 0x1p-1
        naval_hit_effects_big = { naval_formation = not_a_field }
        unknown = { @inside = 0x1p-2 naval_formation = ignored }
        naval_formation = @name naval_formation_scale = @scale
        } naval_formation = after_stop
    "#,
        &mut warnings,
    )
    .unwrap();
    assert_eq!(parsed, ("mod_formation".into(), 0.5));
    assert!(
        warnings
            .iter()
            .any(|(_, warning)| warning.contains("unexpected token")
                && warning.contains("unknown")
                && warning.contains("1 reader reports"))
    );
    assert_eq!(
        formation(
            b"\xef\xbb\xbfnaval_formation_scale = 9 \
            \"naval_formation\" = quoted naval_formation = bare"
        ),
        ("bare".into(), 1.0)
    );
}

#[test]
fn formation_reader_refuses_unrepresentable_stored_content_by_name_and_count() {
    for text in ["inf", "nan", "1e39"] {
        let reason = read_formation(
            format!("naval_formation_scale = {text}").as_bytes(),
            &mut Vec::new(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            reason.contains("1 refused naval_formation_scale"),
            "{reason}"
        );
    }
    let reason = read_formation(b"naval_formation = \"\xfe\"", &mut Vec::new())
        .unwrap_err()
        .to_string();
    assert!(reason.contains("1 refused naval_formation"), "{reason}");
    assert_eq!(
        formation(b"naval_formation = \"a\nb\"").0,
        "Unreadable String"
    );
}

#[test]
fn vanilla_formation_and_scale_are_unchanged() {
    assert_eq!(
        formation(
            br#"
        naval_formation = "ships_formation_template"
        naval_formation_scale = 0.6
        naval_combat_formation = "ships_combat_template"
        naval_combat_formation_scale = 0.5
        naval_death_time = 15.0 naval_random_start_time = 1.5
        naval_hit_effects_big = { ship_hit_big_effect }
        naval_hit_effects_small = { ship_hit_small_effect }
        naval_miss_effects_big = { ship_miss_big_effect }
        naval_miss_effects_small = { ship_miss_small_effect }
    "#
        ),
        ("ships_formation_template".into(), 0.6)
    );
}

#[test]
fn ship_ladder_has_no_culture_step_and_supports_signed_model_numbers() {
    assert_eq!(
        ship_candidates("GER", "destroyer", -1),
        [
            "GER_destroyer_-1_entity",
            "GER_destroyer_entity",
            "destroyer_-1_entity",
            "destroyer_entity"
        ]
    );
    let ships = [
        (1, 1 << 7),
        (2, 1 << 8),
        (3, (1 << 8) | (1 << 39)),
        (4, 1 << 39),
    ]
    .map(|(ship, formation_flags)| replay_data::rows::ShipModelRow {
        ship,
        formation_flags,
        ..Default::default()
    });
    assert_eq!(
        support_ships(&ships.iter().collect::<Vec<_>>())
            .iter()
            .map(|r| r.ship)
            .collect::<Vec<_>>(),
        vec![2, 3, 1]
    );
}

#[test]
fn a_recorded_roster_puts_its_first_capital_in_slot_zero_and_supports_after_it() {
    use replay_data::{rows::ShipCounterInputs, rows::ShipModelRow};
    let row = |ship, ordinal, formation_flags| ShipModelRow {
        ship,
        formation_flags,
        counter: Some(ShipCounterInputs {
            roster_ordinal: ordinal,
            ..Default::default()
        }),
        ..Default::default()
    };
    let ships = [
        row(1, 3, 1 << 8),
        row(2, 1, 1 << 39),
        row(3, 0, 1 << 6),
        row(4, 2, 1 << 8),
    ];
    let slots = formation_slots(&ships.iter().collect::<Vec<_>>());
    assert_eq!(
        slots
            .iter()
            .map(|(slot, r)| (*slot, r.ship))
            .collect::<Vec<_>>(),
        vec![(0, 3), (1, 4), (2, 1), (3, 2)]
    );
    let ships = [row(1, 0, 1 << 8), row(2, 1, 1 << 22)];
    let slots = formation_slots(&ships.iter().collect::<Vec<_>>());
    assert_eq!(
        slots
            .iter()
            .map(|(slot, r)| (*slot, r.ship))
            .collect::<Vec<_>>(),
        vec![(1, 1)]
    );
    let incomplete = [ShipModelRow {
        ship: 9,
        formation_flags: 1 << 7,
        ..Default::default()
    }];
    assert!(formation_slots(&incomplete.iter().collect::<Vec<_>>()).is_empty());
}

#[test]
fn a_patrol_here_has_no_formation_and_one_elsewhere_keeps_its_hulls() {
    use replay_data::{
        rows::CountryLinkRow, rows::CountryRow, rows::ShipCounterInputs, rows::ShipModelRow,
        rows::ShipRow, rows::TableRows, rows::TaskForceCounterInputRow,
        rows::TaskForceMapMotionRow, rows::TaskForceMissionRow, rows::TaskForceRow,
    };
    use replay_frame::{frame::Metrics, frame::TableView};
    use viewer_foundation::unit_rules::PATROL;
    const FORCE: u64 = (6689 << 32) | 51;
    const SHIP: u64 = (12 << 32) | 51;
    let frame = |counter_inputs: bool| {
        let mut views = std::array::from_fn(|_| TableView::Unavailable {
            reason: "not needed".into(),
        });
        let mut set = |rows: TableRows| {
            let index = rows.table().index();
            views[index] = TableView::Complete(rows);
        };
        set(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 100,
            canonical_slot: 0,
        }]));
        set(TableRows::Countries(vec![CountryRow {
            id: 100,
            canonical_slot: 0,
            tag: b"JAP".to_vec(),
            ..Default::default()
        }]));
        set(TableRows::TaskForces(vec![TaskForceRow {
            id: FORCE,
            province: 5,
            logical_owner: 100,
            roster_owner: 100,
            mission: PATROL,
            ..Default::default()
        }]));
        set(TableRows::Ships(vec![ShipRow {
            id: SHIP,
            task_force: FORCE,
            ..Default::default()
        }]));
        set(TableRows::ShipModels(vec![ShipModelRow {
            ship: SHIP,
            fallback_sprite: b"destroyer".to_vec(),
            formation_flags: 1 << 8,
            counter: Some(ShipCounterInputs {
                definition_valid: 1,
                ..Default::default()
            }),
            ..Default::default()
        }]));
        set(TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
            task_force: FORCE,
            ..Default::default()
        }]));
        set(TableRows::ProvinceUnitOrder(vec![
            replay_data::rows::ProvinceUnitOrderRow {
                province: 5,
                mixed: vec![FORCE],
                naval: vec![FORCE],
            },
        ]));
        if counter_inputs {
            set(TableRows::TaskForceCounterInputs(vec![
                TaskForceCounterInputRow {
                    task_force: FORCE,
                    is_in_regions: 1,
                    ..Default::default()
                },
            ]));
        }
        set(TableRows::TaskForceMissions(vec![TaskForceMissionRow {
            task_force: FORCE,
            owner: 100,
            current_region: 79,
            regions: vec![76, 79],
        }]));
        DecodedFrame {
            branch: 1,
            hour: 0,
            tables: views.map(Into::into),
            metrics: Metrics::default(),
        }
    };
    let assets = UnitModelAssets {
        ships: ShipAssets {
            root: Some(mesh::Transform::from_trs(
                [0.0; 3],
                [0.0, 0.0, 0.0, 1.0],
                1.0,
            )),
            nodes: vec![mesh::Transform::from_trs(
                [0.0; 3],
                [0.0, 0.0, 0.0, 1.0],
                1.0,
            )],
            scale: 1.0,
            ..ShipAssets::default()
        },
        bounds: Arc::new(BTreeMap::from([(5, [0.0, 0.0, 1.0, 1.0])])),
        registered: ["destroyer_entity".to_owned()].into(),
        ..UnitModelAssets::off(String::new())
    };
    assets
        .library
        .write()
        .unwrap()
        .index
        .insert("destroyer_entity".into(), 0);
    let hulls = |frame: &DecodedFrame, region: u32| {
        let motions = TaskForceMotions::read(frame).unwrap();
        let missions = MissionSkips::read(frame).unwrap();
        let mut report = ModelReport::default();
        let placed = assets
            .prepare_ships(
                frame,
                &Vision::default(),
                &|_, _| Some([1.0, 0.0, 1.0]),
                None,
                &ShipPlaces {
                    motions: &motions,
                    movement: None,
                    land: &|_| false,
                    missions: &missions,
                    region: &|_| Some(region),
                    scale: &|_, _| 1.0,
                },
                &mut report,
            )
            .unwrap();
        (placed.len(), report.undecided_mission_formations)
    };
    let recorded = frame(true);
    assert_eq!(hulls(&recorded, 79), (0, 0));
    assert_eq!(hulls(&recorded, 76), (1, 0));
    assert_eq!(hulls(&frame(false), 76), (0, 1));
}

fn kotka_formation(combat_role: u8) -> (DecodedFrame, UnitModelAssets) {
    let flags = std::iter::repeat_n(1 << 22, 3)
        .chain(std::iter::repeat_n(1 << 6, 4))
        .chain(std::iter::repeat_n(1 << 6, 3))
        .chain(std::iter::repeat_n(1 << 6, 1))
        .chain(std::iter::repeat_n(1 << 7, 8))
        .chain(std::iter::repeat_n(1 << 8, 30));
    formation_fixture((944 << 32) | 61, 9206, flags, combat_role)
}

fn formation_fixture(
    force: u64,
    province: u32,
    flags: impl IntoIterator<Item = u64>,
    combat_role: u8,
) -> (DecodedFrame, UnitModelAssets) {
    use replay_data::{
        rows::CountryLinkRow, rows::CountryRow, rows::ShipCounterInputs, rows::ShipModelRow,
        rows::ShipRow, rows::TableRows, rows::TaskForceCounterInputRow,
        rows::TaskForceMapMotionRow, rows::TaskForceRow,
    };
    use replay_frame::{frame::Metrics, frame::TableView};
    let models: Vec<_> = flags
        .into_iter()
        .enumerate()
        .map(|(ordinal, formation_flags)| ShipModelRow {
            ship: ((ordinal as u64 + 1) << 32) | 51,
            formation_flags,
            fallback_sprite: b"synthetic_hull".to_vec(),
            counter: Some(ShipCounterInputs {
                roster_ordinal: ordinal as u32,
                definition_valid: 1,
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect();
    let ships = models
        .iter()
        .map(|model| ShipRow {
            id: model.ship,
            task_force: force,
            ..Default::default()
        })
        .collect();
    let mut views = std::array::from_fn(|_| TableView::Unavailable {
        reason: "unused".into(),
    });
    for table in [
        TableRows::CountryLinks(vec![CountryLinkRow {
            id: 100,
            canonical_slot: 0,
        }]),
        TableRows::Countries(vec![CountryRow {
            id: 100,
            canonical_slot: 0,
            tag: b"TEST".to_vec(),
            ..Default::default()
        }]),
        TableRows::TaskForces(vec![TaskForceRow {
            id: force,
            province,
            roster_owner: 100,
            logical_owner: 100,
            ..Default::default()
        }]),
        TableRows::ProvinceUnitOrder(vec![replay_data::rows::ProvinceUnitOrderRow {
            province,
            mixed: vec![force],
            naval: vec![force],
        }]),
        TableRows::Ships(ships),
        TableRows::ShipModels(models),
        TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
            task_force: force,
            combat_role,
            ..Default::default()
        }]),
        TableRows::TaskForceCounterInputs(vec![TaskForceCounterInputRow {
            task_force: force,
            ..Default::default()
        }]),
    ] {
        let index = table.table().index();
        views[index] = TableView::Complete(table);
    }
    let assets = UnitModelAssets {
        ships: ShipAssets {
            root: Some(mesh::Transform::IDENTITY),
            nodes: vec![mesh::Transform::from_trs(
                [20.0, 2.0, 30.0],
                [0.0, 0.0, 0.0, 1.0],
                1.0,
            )],
            scale: 0.6,
            ..Default::default()
        },
        gates: Gates {
            ships: 240.0,
            ..Gates::OFF
        },
        bounds: Arc::new(BTreeMap::from([(
            province,
            [3240.0, 1740.0, 3260.0, 1760.0],
        )])),
        registered: ["synthetic_hull_entity".to_owned()].into(),
        ..UnitModelAssets::off(String::new())
    };
    assets
        .library
        .write()
        .unwrap()
        .index
        .insert("synthetic_hull_entity".into(), 0);
    (
        DecodedFrame {
            branch: 1,
            hour: 60827796,
            tables: views.map(Into::into),
            metrics: Metrics::default(),
        },
        assets,
    )
}

fn kotka_hulls(
    frame: &DecodedFrame,
    assets: &UnitModelAssets,
    land: bool,
    scale: f32,
) -> (Vec<Target>, ModelReport) {
    let motions = TaskForceMotions::read(frame).unwrap();
    let missions = MissionSkips::read(frame).unwrap();
    let mut report = ModelReport::default();
    let provinces: Vec<_> = rows::<tables::TaskForces>(frame)
        .unwrap()
        .rows()
        .iter()
        .map(|force| {
            (
                force.province,
                content_reader_map::movement::ProvinceTerrain {
                    flags: u8::from(land),
                    index: 1,
                },
                Vec::new(),
            )
        })
        .collect();
    let map = content_reader_map::movement::MovementMap::from_parts(&provinces);
    let world = counter_move::RouteWorld {
        map: &map,
        width: 5632.0,
        heights: HeightField {
            width: 2,
            height: 2,
            values: &[0; 4],
        },
    };
    let placed = assets
        .prepare_ships(
            frame,
            &Vision::default(),
            &|_, _| Some([3246.0, 9.53, 1749.0]),
            Some(&world),
            &ShipPlaces {
                motions: &motions,
                movement: None,
                land: &|_| land,
                missions: &missions,
                region: &|_| Some(206),
                scale: &|_, _| scale,
            },
            &mut report,
        )
        .unwrap();
    (placed, report)
}

#[test]
fn capital_uses_root_and_authored_animator_scale() {
    let (frame, assets) = kotka_formation(0);
    let (placed, report) = kotka_hulls(&frame, &assets, false, 0.22);
    assert_eq!(placed.len(), 4);
    let capital = placed.iter().find(|hull| hull.part == 0).unwrap();
    assert_eq!(capital.instance.position, capital.origin);
    assert_eq!(capital.instance.scale, 0.6 * 0.22);
    let support = placed.iter().find(|hull| hull.part == 1).unwrap();
    assert_eq!(
        support.instance.position,
        [
            3246.0 + 20.0 * 0.132,
            9.53 + 2.0 * 0.132,
            1749.0 + 30.0 * 0.132
        ]
    );
    assert_eq!(report.ship_hulls, 4);
}

#[test]
fn ordinary_formations_exclude_both_combat_slots() {
    for role in [1, 2] {
        let (frame, assets) = formation_fixture((3197 << 32) | 61, 2333, [1 << 39; 8], role);
        let (placed, report) = kotka_hulls(&frame, &assets, false, 0.22);
        assert!(placed.is_empty());
        assert_eq!(report.ship_hulls, 0);
        assert_eq!(
            report
                .selection_errors
                .iter()
                .filter(|(reason, _)| reason.starts_with("naval_combat_references:"))
                .map(|(_, count)| count)
                .sum::<usize>(),
            1
        );
    }
}

#[test]
fn missing_capital_root_is_named_and_counted_without_losing_supports() {
    let (frame, mut assets) = kotka_formation(0);
    assets.ships.root = None;
    let (placed, report) = kotka_hulls(&frame, &assets, false, 1.0);
    assert_eq!(placed.len(), 3);
    assert!(placed.iter().all(|hull| hull.part != 0));
    assert_eq!(report.selection_errors.values().sum::<usize>(), 1);
    assert!(
        report
            .selection_errors
            .keys()
            .next()
            .unwrap()
            .starts_with("naval_formation_capital_root:")
    );
}

#[test]
fn capital_attach_point_can_be_a_skeleton_joint_without_a_locator() {
    let mesh = mesh::Mesh {
        skeleton: vec![mesh::Joint {
            name: "root".into(),
            index: 0,
            parent: None,
            inverse_bind: Some(mesh::Transform::IDENTITY),
        }],
        ..Default::default()
    };
    assert_eq!(capital_joint(&mesh, None), Some(mesh::Transform::IDENTITY));
    assert!(mesh.locators.is_empty());
}

fn replace_table(frame: &mut DecodedFrame, table: replay_data::rows::TableRows) {
    let index = table.table().index();
    *frame.tables[index] = replay_frame::frame::TableView::Complete(table);
}

#[test]
fn carrier_plate_only_when_another_task_force_holds_the_visual_slot() {
    use replay_data::{
        rows::ProvinceUnitOrderRow, rows::TableRows, rows::TaskForceCounterInputRow,
        rows::TaskForceMapMotionRow,
    };
    let (mut frame, assets) = kotka_formation(0);
    let force = (944 << 32) | 61;
    let escort = (7 << 32) | 61;
    let mut forces = rows::<tables::TaskForces>(&frame).unwrap().rows().to_vec();
    let mut other = forces[0].clone();
    other.id = escort;
    forces.push(other);
    replace_table(&mut frame, TableRows::TaskForces(forces));
    let mut ships = rows::<tables::Ships>(&frame).unwrap().rows().to_vec();
    let mut other = ships[0].clone();
    other.id = (98 << 32) | 32;
    other.task_force = escort;
    let ship = other.id;
    ships.push(other);
    replace_table(&mut frame, TableRows::Ships(ships));
    let mut models = rows::<tables::ShipModels>(&frame).unwrap().rows().to_vec();
    let mut other = models[0].clone();
    other.ship = ship;
    other.formation_flags = 1 << 7;
    other.counter.as_mut().unwrap().roster_ordinal = 0;
    models.push(other);
    replace_table(&mut frame, TableRows::ShipModels(models));
    replace_table(
        &mut frame,
        TableRows::ProvinceUnitOrder(vec![ProvinceUnitOrderRow {
            province: 9206,
            mixed: vec![force, escort],
            naval: vec![force, escort],
        }]),
    );
    replace_table(
        &mut frame,
        TableRows::TaskForceMapMotion(
            [force, escort]
                .into_iter()
                .map(|task_force| TaskForceMapMotionRow {
                    task_force,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    replace_table(
        &mut frame,
        TableRows::TaskForceCounterInputs(
            [force, escort]
                .into_iter()
                .map(|task_force| TaskForceCounterInputRow {
                    task_force,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let (targets, report) = kotka_hulls(&frame, &assets, false, 0.22);
    assert_eq!(
        targets.len(),
        1,
        "the escort has one hull, the CV only its plate"
    );
    assert_eq!(targets[0].identity, escort);
    assert!(
        report.selection_errors.is_empty(),
        "this is native admission, not a gap"
    );
    replace_table(
        &mut frame,
        TableRows::ProvinceUnitOrder(vec![ProvinceUnitOrderRow {
            province: 9206,
            mixed: vec![escort, force],
            naval: vec![escort, force],
        }]),
    );
    assert_eq!(kotka_hulls(&frame, &assets, false, 0.22).0.len(), 4);
}

#[test]
fn rosters_hide_port_hulls_and_share_one_slot_at_sea() {
    use replay_data::{
        rows::ProvinceUnitOrderRow, rows::TableRows, rows::TaskForceCounterInputRow,
        rows::TaskForceMapMotionRow,
    };
    let compositions = [
        (497, vec![1 << 39; 8]),
        (1671, vec![1 << 8; 8]),
        (1497, vec![1 << 39; 8]),
        (7926, vec![1 << 8; 8]),
        (
            8537,
            std::iter::once(1 << 8)
                .chain(std::iter::repeat_n(1 << 39, 6))
                .collect(),
        ),
    ];
    let mut frames = Vec::new();
    for (id, flags) in compositions {
        frames.push(formation_fixture((id << 32) | 61, 9279, flags, 0));
    }
    let (mut frame, assets) = frames.remove(0);
    let mut forces = rows::<tables::TaskForces>(&frame).unwrap().rows().to_vec();
    let mut ships = rows::<tables::Ships>(&frame).unwrap().rows().to_vec();
    let mut models = rows::<tables::ShipModels>(&frame).unwrap().rows().to_vec();
    for (index, (other, _)) in frames.iter().enumerate() {
        forces.extend_from_slice(rows::<tables::TaskForces>(other).unwrap().rows());
        let offset = (index as u64 + 1) << 40;
        ships.extend(
            rows::<tables::Ships>(other)
                .unwrap()
                .rows()
                .iter()
                .cloned()
                .map(|mut row| {
                    row.id += offset;
                    row
                }),
        );
        models.extend(
            rows::<tables::ShipModels>(other)
                .unwrap()
                .rows()
                .iter()
                .cloned()
                .map(|mut row| {
                    row.ship += offset;
                    row
                }),
        );
    }
    replace_table(&mut frame, TableRows::TaskForces(forces.clone()));
    replace_table(&mut frame, TableRows::Ships(ships));
    replace_table(&mut frame, TableRows::ShipModels(models));
    replace_table(
        &mut frame,
        TableRows::TaskForceMapMotion(
            forces
                .iter()
                .map(|force| TaskForceMapMotionRow {
                    task_force: force.id,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    replace_table(
        &mut frame,
        TableRows::TaskForceCounterInputs(
            forces
                .iter()
                .map(|force| TaskForceCounterInputRow {
                    task_force: force.id,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    replace_table(
        &mut frame,
        TableRows::ProvinceUnitOrder(vec![ProvinceUnitOrderRow {
            province: 9279,
            mixed: forces.iter().map(|force| force.id).collect(),
            naval: forces.iter().map(|force| force.id).collect(),
        }]),
    );
    assert!(assets.gates.ships(112.619_415_283_203_12_f64 as f32));
    assert!(
        kotka_hulls(&frame, &assets, true, 0.22).0.is_empty(),
        "recorded Malmö port slot19 is hidden"
    );
    let at_sea = kotka_hulls(&frame, &assets, false, 0.22).0;
    assert_eq!(
        at_sea.len(),
        3,
        "synthetic sea placement chooses ONE representative, not five formations"
    );
    assert!(
        at_sea
            .iter()
            .all(|target| target.identity == forces.last().unwrap().id)
    );
    let motions = TaskForceMotions::read(&frame).unwrap();
    let missions = MissionSkips::read(&frame).unwrap();
    let sources = assets
        .prepare_naval(
            &frame,
            &Vision::default(),
            &|_, _| Some([3246.0, 9.53, 1749.0]),
            None,
            &ShipPlaces {
                motions: &motions,
                movement: None,
                land: &|_| false,
                missions: &missions,
                region: &|_| Some(206),
                scale: &|_, _| 0.22,
            },
            None,
            &mut ModelReport::default(),
        )
        .unwrap();
    assert_eq!(sources.settled_targets(assets.gates).len(), 3);
    let mut history = viewer_unit_history::unit_history::History::default();
    history.set(Vec::new(), assets.bounds.clone(), false);
    history.set_naval(sources);
    let camera = Placement {
        center: [3246.0, 1749.0],
        height: 112.619_415_283_203_12_f64 as f32,
        pose: viewer_gpu::camera::Pose::Map,
    }
    .on(MapSize::TEST, MapDefines::test());
    for count in [1, 2, 3, 3] {
        let frame = history.update_models(
            camera,
            [1024, 768],
            0.0,
            assets.gates,
            1,
            &mut viewer_buildings::map_buildings::BuildingHistory::default(),
            false,
        );
        assert_eq!(history.drawn(&frame.units).len(), count);
    }
}

mod name_pass;
