use super::*;

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
    // A failed numeric block is skipped by RecordError, including its fields.
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
    // Quoted numbers are scanned too, and malformed exponents store nothing.
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
            .any(|warning| warning.contains("unexpected token")
                && warning.contains("unknown")
                && warning.contains("1 reader reports"))
    );
    // Quoted keys are not keywords; mode 0 keeps a BOM in the first key.
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
    .map(|(ship, formation_flags)| replay::rows::ShipModelRow {
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
    use replay::rows::{ShipCounterInputs, ShipModelRow};
    let row = |ship, ordinal, formation_flags| ShipModelRow {
        ship,
        formation_flags,
        counter: Some(ShipCounterInputs {
            roster_ordinal: ordinal,
            ..Default::default()
        }),
        ..Default::default()
    };
    // Handle order 1..4; roster order puts the battleship (bit 6) first.
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
    // Missing counter inputs cannot establish the formation's roster order.
    let incomplete = [ShipModelRow {
        ship: 9,
        formation_flags: 1 << 7,
        ..Default::default()
    }];
    assert!(formation_slots(&incomplete.iter().collect::<Vec<_>>()).is_empty());
}

/// The ship models share the counter pass's mission skip: an active patrol in the region it
/// patrols now has no formation, one outside it keeps its hulls, and unrecorded skip
/// operands leave the formation undrawn and counted.
#[test]
fn a_patrol_here_has_no_formation_and_one_elsewhere_keeps_its_hulls() {
    use crate::map::counter_inputs::PATROL;
    use replay::{
        frame::{Metrics, TableView},
        rows::{
            CountryLinkRow, CountryRow, ShipCounterInputs, ShipModelRow, ShipRow, TableRows,
            TaskForceCounterInputRow, TaskForceMapMotionRow, TaskForceMissionRow, TaskForceRow,
        },
    };
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
            hull_sprite: b"destroyer".to_vec(),
            formation_flags: 1 << 8,
            counter: Some(ShipCounterInputs::default()),
            ..Default::default()
        }]));
        set(TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
            task_force: FORCE,
            ..Default::default()
        }]));
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
            nodes: vec![mesh::Transform::from_trs(
                [0.0; 3],
                [0.0, 0.0, 0.0, 1.0],
                1.0,
            )],
            scale: 1.0,
        },
        // Province 5 has content bounds, which model admission requires.
        bounds: std::sync::Arc::new(BTreeMap::from([(5, [0.0, 0.0, 1.0, 1.0])])),
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
                },
                &mut report,
            )
            .unwrap();
        (placed.len(), report.undecided_mission_formations)
    };
    let recorded = frame(true);
    // Patrolling region 79 now: skipped. In region 76, where it does not patrol now: drawn.
    assert_eq!(hulls(&recorded, 79), (0, 0));
    assert_eq!(hulls(&recorded, 76), (1, 0));
    // Without its counter inputs the skip is undecided: no formation, counted.
    assert_eq!(hulls(&frame(false), 76), (0, 1));
}
