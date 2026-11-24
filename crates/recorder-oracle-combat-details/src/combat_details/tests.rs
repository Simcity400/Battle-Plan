use super::*;
use replay_data::{
    rows::AirBaseRow, rows::AirWingRow, rows::LandCombatSideRow,
    rows::NavalCombatAirBaseWingHandlesRow, rows::NavalCombatShipRow, rows::Recorded,
    rows::TableRows,
};
use replay_frame::frame::TableView;

fn land_sides_fixture() -> (DecodedFrame, Save, u64) {
    let (mut frame, mut save, ..) = super::super::fixtures::division_fixture();
    let division = save.divisions.values_mut().next().unwrap();
    division.roster_owner = division.logical_owner.clone();
    division.expeditionary_owner = None;
    let unit = division.id;
    let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *frame.tables[TableId::Divisions.index()]
    else {
        unreachable!()
    };
    rows[0].roster_owner = rows[0].logical_owner;
    rows[0].expeditionary_owner = 0;
    let combat = (7_u64 << 32) | 62;
    save.land_combats.insert(
        combat,
        save::LandCombat {
            id: combat,
            province: 2,
            details: Default::default(),
            day: 0,
            duration: 0,
            attackers: vec![unit],
            defenders: Vec::new(),
        },
    );
    *frame.tables[TableId::LandCombatSides.index()] =
        TableView::Complete(TableRows::LandCombatSides(vec![LandCombatSideRow {
            combat,
            attacker_primary: vec![102],
            ..Default::default()
        }]));
    (frame, save, combat)
}

#[test]
fn land_sides_returned_expeditionary_owner_does_not_certify_primary_membership() {
    for defender in [false, true] {
        let (mut frame, mut save, combat) = land_sides_fixture();
        if defender {
            let saved = save.land_combats.get_mut(&combat).unwrap();
            std::mem::swap(&mut saved.attackers, &mut saved.defenders);
            let TableView::Complete(TableRows::LandCombatSides(rows)) =
                &mut *frame.tables[TableId::LandCombatSides.index()]
            else {
                unreachable!()
            };
            let row = &mut rows[0];
            std::mem::swap(&mut row.attacker_primary, &mut row.defender_primary);
        }
        let mut report = Report::default();
        sides(&frame, &save, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["LandCombatSides"], 1);
        assert_eq!(report.scalar_checks, 0);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("current owners cannot certify membership"))
        );
    }
}

#[test]
fn land_sides_missing_combat_is_a_difference() {
    let (mut frame, save, combat) = land_sides_fixture();
    *frame.tables[TableId::LandCombatSides.index()] =
        TableView::Complete(TableRows::LandCombatSides(Vec::new()));
    let mut report = Report::default();
    sides(&frame, &save, &mut report).unwrap();
    assert_eq!(
        report.differences,
        [format!(
            "LandCombatSides {combat}: replay \"absent\", save \"present\""
        )]
    );
}

#[test]
fn land_sides_extra_combat_is_a_difference() {
    let (frame, mut save, combat) = land_sides_fixture();
    save.land_combats.clear();
    let mut report = Report::default();
    sides(&frame, &save, &mut report).unwrap();
    assert_eq!(
        report.differences,
        [format!(
            "LandCombatSides {combat}: replay \"present\", save \"absent\""
        )]
    );
}

#[test]
fn land_sides_null_or_duplicate_combats_are_rejected() {
    for null in [false, true] {
        let (mut frame, save, combat) = land_sides_fixture();
        let row = LandCombatSideRow {
            combat: if null { 0 } else { combat },
            ..Default::default()
        };
        let rows = if null {
            vec![row]
        } else {
            vec![row.clone(), row]
        };
        *frame.tables[TableId::LandCombatSides.index()] =
            TableView::Complete(TableRows::LandCombatSides(rows));
        let mut report = Report::default();
        let error = sides(&frame, &save, &mut report).unwrap_err();
        assert_eq!(
            error.to_string(),
            "LandCombatSides: null or duplicate combat handle"
        );
    }
}

#[test]
fn land_sides_unresolved_primary_references_are_rejected_on_both_sides() {
    for defender in [false, true] {
        let (mut frame, save, combat) = land_sides_fixture();
        let mut row = LandCombatSideRow {
            combat,
            attacker_primary: vec![102],
            defender_primary: vec![102],
            ..Default::default()
        };
        let primary = if defender {
            &mut row.defender_primary
        } else {
            &mut row.attacker_primary
        };
        primary.push(999);
        *frame.tables[TableId::LandCombatSides.index()] =
            TableView::Complete(TableRows::LandCombatSides(vec![row]));
        let mut report = Report::default();
        let error = sides(&frame, &save, &mut report).unwrap_err();
        assert_eq!(error.to_string(), "unresolved country reference 999");
    }
}

#[test]
fn missing_retained_details_report_incomplete_coverage_without_waiving_differences() {
    use replay_data::rows::NavalCombatResultDetailsRow;
    let (mut frame, mut save, ..) = super::super::fixtures::fixture();
    save.naval_history.results.clear();
    for id in [101, 102] {
        let date = save::GameDate {
            year: 1940,
            month: 1,
            day: 1,
            hour: 1,
        };
        let mut sides = std::array::from_fn(|_| save::naval_history::ResultSide::default());
        sides[0].details.leader = Some(77);
        save.naval_history.results.insert(
            id,
            save::naval_history::NavalResult {
                id,
                province: 2,
                result_date: date,
                retire_date: date,
                category: 0,
                port_strike: false,
                naval_strike: false,
                sides,
                admitted: Vec::new(),
            },
        );
    }
    let mut readable = NavalCombatResultDetailsRow {
        result: 101,
        ..Default::default()
    };
    readable.sides[0].leader = 77;
    *frame.tables[TableId::NavalCombatResultDetails.index()] =
        TableView::Complete(TableRows::NavalCombatResultDetails(vec![readable.clone()]));
    let mut report = Report::default();
    let error = results(&frame, &save, &mut report).unwrap_err();
    assert!(error.to_string().contains("missing rows): [102]"));
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["NavalCombatResultDetails"], 1);
    readable.sides[0].leader = 78;
    *frame.tables[TableId::NavalCombatResultDetails.index()] =
        TableView::Complete(TableRows::NavalCombatResultDetails(vec![
            readable,
            NavalCombatResultDetailsRow {
                result: 999,
                ..Default::default()
            },
        ]));
    let mut report = Report::default();
    assert!(results(&frame, &save, &mut report).is_err());
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
}

#[test]
fn border_marker_witnesses_compare_stored_country_state_and_event_strings() {
    use replay_data::{
        rows::LandBorderWarInputs, rows::LandBorderWarSide, rows::LandCombatDetailsRow,
    };
    let (mut frame, mut save, ..) = super::super::fixtures::fixture();
    let combat = (7_u64 << 32) | 62;
    let border = LandBorderWarInputs {
        transfer_state: 128,
        combat_width: Recorded::NotRecorded,
        sides: [
            LandBorderWarSide {
                country: 102,
                state: 11,
                win_event: b"synthetic.001".to_vec(),
                lose_event: Vec::new(),
            },
            LandBorderWarSide {
                country: 102,
                state: 13,
                win_event: Vec::new(),
                lose_event: b"synthetic.2".to_vec(),
            },
        ],
    };
    *frame.tables[TableId::LandCombatDetails.index()] =
        TableView::Complete(TableRows::LandCombatDetails(vec![LandCombatDetailsRow {
            combat,
            province: 2,
            border_war: Recorded::Value(Some(border)),
            ..Default::default()
        }]));
    let mut details = save::combat_details::LandDetails {
        border_war: true,
        transfer_state: Some(true),
        ..Default::default()
    };
    for (side, state) in details.sides.iter_mut().zip([11, 13]) {
        side.border_country = Some(b"GER".to_vec());
        side.border_state = Some(state);
        side.organisation_loss_count = Some(0);
    }
    details.sides[0].border_win_event = Some(b"synthetic.001".to_vec());
    details.sides[1].border_lose_event = Some(b"synthetic.2".to_vec());
    save.land_combats.insert(
        combat,
        save::LandCombat {
            id: combat,
            province: 2,
            details,
            day: 0,
            duration: 0,
            attackers: Vec::new(),
            defenders: Vec::new(),
        },
    );
    let mut report = Report::default();
    land(&frame, &save, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    save.land_combats
        .get_mut(&combat)
        .unwrap()
        .details
        .transfer_state = None;
    let mut report = Report::default();
    land(&frame, &save, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    let saved = &mut save.land_combats.get_mut(&combat).unwrap().details;
    saved.transfer_state = Some(false);
    saved.sides[0].border_state = Some(12);
    saved.sides[0].border_win_event = Some(b"synthetic.1".to_vec());
    let mut report = Report::default();
    land(&frame, &save, &mut report).unwrap();
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    *frame.tables[TableId::LandCombatDetails.index()] =
        TableView::Complete(TableRows::LandCombatDetails(vec![LandCombatDetailsRow {
            combat,
            province: 3,
            border_war: Recorded::NotRecorded,
            ..Default::default()
        }]));
    let mut report = Report::default();
    land(&frame, &save, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("province"));
    assert_eq!(
        report.limitations["LandCombatDetails.border_war not recorded"].len(),
        1
    );
}

#[test]
fn cas_operands_compare_anti_air_exactly_and_prevented_damage_at_its_step() {
    use replay_data::{rows::LandCombatDetailsRow, rows::LandCombatSideInputs};
    let (mut frame, mut save, ..) = super::super::fixtures::fixture();
    let combat = (7_u64 << 32) | 62;
    let mut details = save::combat_details::LandDetails::default();
    for side in &mut details.sides {
        side.organisation_loss_count = Some(0);
    }
    details.sides[0].anti_air = Some(Decimal::parse(b"12.34567").unwrap());
    details.sides[0].damage[4] = Some(Decimal::parse(b"2.5").unwrap());
    details.sides[0].damage[5] = Some(Decimal::parse(b"0.75").unwrap());
    save.land_combats.insert(
        combat,
        save::LandCombat {
            id: combat,
            province: 2,
            details,
            day: 0,
            duration: 0,
            attackers: Vec::new(),
            defenders: Vec::new(),
        },
    );
    let run = |frame: &mut DecodedFrame, save: &Save, sides: [LandCombatSideInputs; 2]| {
        *frame.tables[TableId::LandCombatDetails.index()] =
            TableView::Complete(TableRows::LandCombatDetails(vec![LandCombatDetailsRow {
                combat,
                province: 2,
                border_war: Recorded::Value(None),
                side_inputs: Recorded::Value(sides),
                ..Default::default()
            }]));
        let mut report = Report {
            ..Default::default()
        };
        land(frame, save, &mut report).unwrap();
        report
    };
    let recorded = LandCombatSideInputs {
        anti_air_attack: 1_234_567,
        prevented_damage: [200_000, 0],
        ..Default::default()
    };
    let report = run(
        &mut frame,
        &save,
        [recorded.clone(), LandCombatSideInputs::default()],
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .display_compared
            .contains_key("LandCombatDetails.side_inputs.prevented_damage")
    );
    for sides in [
        [
            LandCombatSideInputs {
                anti_air_attack: 1_200_000,
                ..recorded.clone()
            },
            LandCombatSideInputs::default(),
        ],
        [
            LandCombatSideInputs {
                prevented_damage: [300_000, 0],
                ..recorded.clone()
            },
            LandCombatSideInputs::default(),
        ],
    ] {
        let report = run(&mut frame, &save, sides);
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    }
    let report = run(
        &mut frame,
        &save,
        [
            recorded,
            LandCombatSideInputs {
                prevented_damage: [0, 100_000],
                ..Default::default()
            },
        ],
    );
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("prevented_damage_org positive-field presence"));
}

#[test]
fn reserve_speed_shares_sit_exactly_on_the_saved_reserves() {
    use replay_data::rows::LandCombatDivisionRow;
    let (mut frame, mut save, ..) = super::super::fixtures::fixture();
    let combat = (7_u64 << 32) | 62;
    let (committed, reserve) = (0x9000_0001_u64, 0x9000_0002_u64);
    let mut details = save::combat_details::LandDetails::default();
    details.sides[0].committed = vec![committed];
    details.sides[0].reserves = vec![reserve];
    save.land_combats.insert(
        combat,
        save::LandCombat {
            id: combat,
            province: 2,
            details,
            day: 0,
            duration: 0,
            attackers: vec![committed, reserve],
            defenders: Vec::new(),
        },
    );
    for id in [committed, reserve] {
        save.divisions.insert(
            id,
            save::Division {
                id,
                roster_owner: b"SOV".to_vec(),
                logical_owner: b"SOV".to_vec(),
                expeditionary_owner: None,
                province: Some(2),
                template: None,
                alternate_template: None,
                separate_counter: false,
                hit_points: Decimal::parse(b"1").unwrap(),
                organisation: Decimal::parse(b"1").unwrap(),
                country_intel: Vec::new(),
                presentation: Default::default(),
            },
        );
    }
    let run = |frame: &mut DecodedFrame, shares: [Option<i64>; 2]| {
        let [first, second] = shares;
        *frame.tables[TableId::LandCombatDivisions.index()] =
            TableView::Complete(TableRows::LandCombatDivisions(vec![
                LandCombatDivisionRow {
                    combat,
                    side: 0,
                    ordinal: 0,
                    role: 1,
                    province: 2,
                    division: committed,
                    reinforcement_speed: first,
                    ..Default::default()
                },
                LandCombatDivisionRow {
                    combat,
                    side: 0,
                    ordinal: 1,
                    role: 2,
                    province: 2,
                    division: reserve,
                    reinforcement_speed: second,
                    ..Default::default()
                },
            ]));
        let mut report = Report::default();
        divisions(frame, &save, &mut report).unwrap();
        report
    };
    let shares = |report: &Report| {
        report
            .differences
            .iter()
            .filter(|difference| difference.contains("reserve speed share"))
            .count()
    };
    let report = run(&mut frame, [None, Some(150_000)]);
    assert_eq!(shares(&report), 0, "{:?}", report.differences);
    let report = run(&mut frame, [Some(0), Some(0)]);
    assert_eq!(shares(&report), 1, "{:?}", report.differences);
    let report = run(&mut frame, [None, None]);
    assert_eq!(shares(&report), 0, "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.starts_with("A reserve's speed share was refused at capture"))
    );
}

#[test]
fn border_width_oracle_compares_zero_precision_and_unconditional_presence() {
    use replay_data::{rows::LandBorderWarInputs, rows::LandCombatDetailsRow};
    let (mut frame, mut save, ..) = super::super::fixtures::fixture();
    let combat = (7_u64 << 32) | 62;
    let mut details = save::combat_details::LandDetails {
        border_war: true,
        transfer_state: Some(false),
        combat_width: Some(Decimal::parse(b"0").unwrap()),
        ..Default::default()
    };
    for side in &mut details.sides {
        side.border_state = Some(0);
        side.organisation_loss_count = Some(0);
    }
    save.land_combats.insert(
        combat,
        save::LandCombat {
            id: combat,
            province: 2,
            details,
            day: 0,
            duration: 0,
            attackers: Vec::new(),
            defenders: Vec::new(),
        },
    );
    let run = |frame: &mut DecodedFrame, save: &Save, combat_width| {
        *frame.tables[TableId::LandCombatDetails.index()] =
            TableView::Complete(TableRows::LandCombatDetails(vec![LandCombatDetailsRow {
                combat,
                province: 2,
                border_war: Recorded::Value(Some(LandBorderWarInputs {
                    combat_width,
                    ..Default::default()
                })),
                ..Default::default()
            }]));
        let mut report = Report::default();
        land(frame, save, &mut report).unwrap();
        report
    };
    let report = run(&mut frame, &save, Recorded::Value(0));
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let report = run(&mut frame, &save, Recorded::Value(-1));
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    save.land_combats
        .get_mut(&combat)
        .unwrap()
        .details
        .combat_width = None;
    let report = run(&mut frame, &save, Recorded::Value(0));
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    let report = run(&mut frame, &save, Recorded::NotRecorded);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|field| field == "LandBorderWarInputs.combat_width not recorded")
    );
}

#[test]
fn air_base_wing_rows_must_be_exactly_the_referenced_bases_in_the_air_censuses() {
    fn set(frame: &mut DecodedFrame, rows: TableRows) {
        let index = rows.table().index();
        *frame.tables[index] = TableView::Complete(rows);
    }
    let (mut frame, ..) = super::super::fixtures::fixture();
    let base = (3_u64 << 32) | 65;
    set(
        &mut frame,
        TableRows::AirBases(vec![AirBaseRow {
            id: base,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::AirWings(vec![AirWingRow {
            id: 9,
            ..Default::default()
        }]),
    );
    set(&mut frame, TableRows::NavalCombatDetails(Vec::new()));
    let mut ship = NavalCombatShipRow {
        live_ship: 1,
        ..Default::default()
    };
    ship.weapon_context.air_base = base;
    set(&mut frame, TableRows::NavalCombatShips(vec![ship]));
    set(
        &mut frame,
        TableRows::NavalCombatAirBaseWingHandles(vec![NavalCombatAirBaseWingHandlesRow {
            base,
            wings: vec![9, 9],
        }]),
    );
    let mut report = Report::default();
    air_base_wings(&frame, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    set(
        &mut frame,
        TableRows::NavalCombatAirBaseWingHandles(vec![
            NavalCombatAirBaseWingHandlesRow {
                base,
                wings: vec![10],
            },
            NavalCombatAirBaseWingHandlesRow {
                base: base + (1 << 32),
                wings: Vec::new(),
            },
        ]),
    );
    let mut report = Report::default();
    air_base_wings(&frame, &mut report).unwrap();
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
}

use replay_data::rows::LandCombatCommanderTimedTrait;

#[test]
fn naval_air_localized_cache_is_not_a_saved_plane_comparator() {
    let date = content_reader_save::SavedDate::parse(b"1936.1.1.1").unwrap();
    let mut saved = content_reader_save::combat_details::NavalPlane {
        source: 0,
        group_index: 0,
        index: 0,
        names: vec![b"Spitfire".to_vec(), b"Hurricane".to_vec()],
        carrier: None,
        air_base: None,
        wings: [Vec::new(), Vec::new()],
        initial: 0,
        current: 0,
        casualties: 0,
        time_duration: 0,
        country: b"---".to_vec(),
        date,
        flags: [false; 2],
        name: b"Mediterranean".to_vec(),
        damage_mult: content_reader_save::Decimal::parse(b"1").unwrap(),
        naval_hits: Vec::new(),
        air_hits: Vec::new(),
    };
    let row = replay_data::rows::NavalCombatPlane {
        carrier_slot: -1,
        date: date.engine_hour().unwrap(),
        ..Default::default()
    };
    let compare = |saved: &content_reader_save::combat_details::NavalPlane| {
        let mut report = Report::default();
        plane_values(&mut report, "plane", &row, saved, &BTreeMap::new()).unwrap();
        report.differences
    };
    assert!(compare(&saved).is_empty());
    saved.name = b"Atlantic".to_vec();
    saved.names.swap(0, 1);
    assert!(compare(&saved).is_empty());
    saved.current = 1;
    let differences = compare(&saved);
    assert_eq!(differences.len(), 1, "{differences:?}");
    assert!(differences[0].contains("plane current:"));
}

#[test]
fn naval_side_units_compare_with_the_saved_unit_list_in_order() {
    use replay_data::{rows::NavalCombatDetailsRow, rows::TableId, rows::TableRows};
    use replay_frame::frame::TableView;
    let (frame, mut saved, _, _) = recorder_oracle_support::fixtures::fixture();
    let combat = (5_u64 << 32) | 4713;
    let zero = || content_reader_save::Decimal::parse(b"0").unwrap();
    let units = [(244_u64 << 32) | 61, (9 << 32) | 61];
    saved.naval_combats.insert(
        combat,
        content_reader_save::NavalCombat {
            id: combat,
            province: 2,
            attacker_positioning: zero(),
            defender_positioning: zero(),
            attacker_anti_air: zero(),
            defender_anti_air: zero(),
            details: Default::default(),
            transfers: Vec::new(),
            attackers: units.to_vec(),
            defenders: Vec::new(),
        },
    );
    let differences = |recorded: Vec<u64>| {
        let mut row = NavalCombatDetailsRow {
            combat,
            province: 2,
            ..Default::default()
        };
        row.sides[0].units = recorded;
        let mut frame = frame.clone();
        *frame.tables[TableId::NavalCombatDetails.index()] =
            TableView::Complete(TableRows::NavalCombatDetails(vec![row]));
        let mut report = Report::default();
        naval(&frame, &saved, &mut report).unwrap();
        report
            .differences
            .iter()
            .filter(|line| line.contains(" units:"))
            .count()
    };
    assert_eq!(differences(units.to_vec()), 0);
    assert_eq!(differences(vec![units[1], units[0]]), 1);
    assert_eq!(differences(vec![units[0]]), 1);
}

#[test]
fn timed_traits_compare_saved_tokens_and_signed_days_in_vector_order() {
    let save = content_reader_save::parse(
        br#"HOI4txt
        date=1942.9.6.2 game_unique_id=test version=test game_unique_seed=1 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
        countries={} states={} provinces={}
        equipment_market={contracts={} requests={0}} supply_system_2={countries={}}
        character_manager={historical={character={id={id=8 type=73} token=x
            corps_commander={id={id=4 type=4713} skill=1 traits_to_remove={sick=20 wounded=-1}}
        }}}
        "#,
    )
    .unwrap();
    let timed = |token_id, remaining_days| LandCombatCommanderTimedTrait {
        token_id,
        definition_ordinal: 0,
        remaining_days,
    };
    let commander = (4 << 32) | 4713;
    let traits = vec![timed(1, 20), timed(2, -1)];
    let tokens: TokenNames<'static> =
        BTreeMap::from([(1, b"sick".as_slice()), (2, b"wounded".as_slice())]);
    let compare = |commander, traits: &[LandCombatCommanderTimedTrait], tokens: &TokenNames<'_>| {
        let mut report = Report::default();
        timed_traits(&mut report, "side", commander, 1, traits, &save, tokens);
        report
    };
    let report = compare(commander, &traits, &tokens);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let swapped = [timed(2, -1), timed(1, 20)];
    assert_eq!(compare(commander, &swapped, &tokens).differences.len(), 4);
    let report = compare(commander, &traits, &TokenNames::new());
    assert!(report.differences.is_empty());
    assert_eq!(report.limitations.values().flatten().count(), 2);
    let extra = [timed(1, 20), timed(2, -1), timed(1, 3)];
    assert_eq!(compare(commander, &extra, &tokens).differences.len(), 1);
    assert_eq!(compare(0, &traits, &tokens).differences.len(), 1);
    let mut save = save;
    save.characters
        .manager
        .as_mut()
        .unwrap()
        .unit_leaders
        .get_mut(&commander)
        .unwrap()
        .category = 2;
    let mut naval = replay_data::rows::NavalCommanderInputs {
        commander,
        category: 2,
        skills: [1, 0, 0, 0, 0],
        timed_traits: Recorded::Value(traits),
        ..Default::default()
    };
    let compare_naval = |actual: &replay_data::rows::NavalCommanderInputs| {
        let mut report = Report::default();
        naval_commander_values(&mut report, "naval", actual, &save, &tokens).unwrap();
        report
    };
    assert!(compare_naval(&naval).differences.is_empty());
    naval
        .timed_traits
        .require_mut("NavalCommanderInputs.timed_traits")
        .unwrap()[0]
        .remaining_days = 19;
    assert_eq!(compare_naval(&naval).differences.len(), 1);
    naval
        .timed_traits
        .require_mut("NavalCommanderInputs.timed_traits")
        .unwrap()
        .clear();
    assert_eq!(compare_naval(&naval).differences.len(), 1);
    naval.timed_traits = Recorded::NotRecorded;
    let report = compare_naval(&naval);
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason == "NavalCommanderInputs.timed_traits not recorded")
    );
}

#[test]
fn null_naval_commander_is_independent_of_the_saved_last_leader() {
    use replay_data::{rows::NavalCombatDetailsRow, rows::NavalCombatSideDetails};
    use replay_frame::{
        branches::BranchReason::HourDiscontinuity as Gap,
        branches::BranchReason::LoadSignal as Load, branches::BranchReason::SessionStart as Start,
    };
    use replay_frame::{branches::BranchStart, branches::Parentage};

    let (mut frame, mut save, _, _) = super::super::fixtures::fixture();
    let zero = Decimal::parse(b"0").unwrap();
    let combat = 17;
    let leader = (37_u64 << 32) | 4713;
    save.naval_combats.insert(
        combat,
        save::NavalCombat {
            id: combat,
            province: 2,
            attackers: Vec::new(),
            defenders: Vec::new(),
            transfers: Vec::new(),
            attacker_positioning: zero.clone(),
            defender_positioning: zero.clone(),
            attacker_anti_air: zero.clone(),
            defender_anti_air: zero.clone(),
            details: save::combat_details::NavalDetails {
                day: Some(0),
                duration: Some(0),
                progress: Some(zero.clone()),
                leaders: [Some(leader), None],
                escort_bonus: [Some(zero.clone()), Some(zero.clone())],
                original_health: [Some(zero.clone()), Some(zero.clone())],
                total_damage: [Some(zero.clone()), Some(zero.clone())],
                damage_history: [Some(vec![zero.clone(); 36]), Some(vec![zero; 36])],
                ..Default::default()
            },
        },
    );
    let set_commander = |frame: &mut DecodedFrame, commander| {
        *frame.tables[TableId::NavalCombatDetails.index()] =
            TableView::Complete(TableRows::NavalCombatDetails(vec![NavalCombatDetailsRow {
                combat,
                province: 2,
                sides: [
                    NavalCombatSideDetails {
                        commander,
                        ..Default::default()
                    },
                    NavalCombatSideDetails::default(),
                ],
                ..Default::default()
            }]));
    };
    let compare = |frame: &DecodedFrame, save: &Save, branch: BranchStart| {
        let mut report = Report {
            hour: frame.hour,
            ..Default::default()
        };
        report.set_load_hour(&branch);
        naval(frame, save, &mut report).unwrap();
        assert_eq!(report.compared_tables["NavalCombatDetails"], 1);
        report
    };
    let limited = |report: &Report| {
        report
            .limitations
            .keys()
            .any(|key| key.starts_with("Naval current commander is null"))
    };
    let id = frame.branch;
    let branch = |reason, loaded_hour, first_hour| BranchStart {
        id,
        session: 1,
        loaded_hour,
        first_hour,
        parent: Parentage::Unknown,
        reason,
    };

    let h = frame.hour;
    for (reason, loaded, first, sample, commander, expected) in [
        (Load, Some(h), h, h, 0, "limited"),
        (Load, Some(h), h, h, leader, "equal"),
        (Load, Some(h), h, h, leader + 1, "differs"),
        (Load, Some(h), h, h + 1, 0, "limited"),
        (Load, None, h, h, 0, "limited"),
        (Load, Some(h), h + 1, h, 0, "limited"),
        (Start, Some(h), h, h, 0, "limited"),
        (Gap, Some(h), h, h, 0, "limited"),
    ] {
        frame.hour = sample;
        set_commander(&mut frame, commander);
        let report = compare(&frame, &save, branch(reason, loaded, first));
        let outcome = match (report.differences.len(), limited(&report)) {
            (0, true) => "limited",
            (0, false) => "equal",
            (1, false) => "differs",
            _ => panic!("{report:?}"),
        };
        assert_eq!(
            outcome, expected,
            "{reason:?} {loaded:?} {first} {sample} {commander}"
        );
    }

    frame.hour = h + 1;
    set_commander(&mut frame, 0);
    save.naval_combats.get_mut(&combat).unwrap().details.day = Some(1);
    let report = compare(&frame, &save, branch(Load, Some(h), h));
    assert!(limited(&report));
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("day"));
}

#[test]
fn positive_only_writer_does_not_invent_zero_or_waive_missing_positive() {
    let mut report = Report::default();
    positive_decimal(&mut report, "omitted negative", -10, None);
    positive_decimal(&mut report, "omitted zero", 0, None);
    assert!(report.differences.is_empty());
    positive_decimal(&mut report, "missing positive", 1, None);
    positive_decimal(
        &mut report,
        "wrong raw",
        2,
        Some(&Decimal::parse(b"0.00001").unwrap()),
    );
    assert_eq!(report.differences.len(), 2);
    assert_eq!(report.scalar_checks, 4);
}
#[test]
fn hit_comparison_keeps_missing_entries_and_raw_damage_differences() {
    let a = replay_data::rows::NavalCombatHit {
        target_slot: u32::MAX,
        target_name: b"Convoy".to_vec(),
        convoy: 2,
        damage: 2,
        denominator: 100000,
        last_hit: 0,
    };
    let b = save::combat_details::NavalHit {
        target: u32::MAX,
        name: b"Convoy".to_vec(),
        convoy: true,
        damage: Decimal::parse(b"0.00001").unwrap(),
        strength: Decimal::parse(b"1").unwrap(),
        last_hit: false,
    };
    let mut report = Report::default();
    hit_values(&mut report, "slot", &[a.clone(), a], &[b]);
    assert_eq!(report.differences.len(), 2);
    assert!(
        report
            .differences
            .iter()
            .any(|v| v.contains("naval hit count"))
    );
    assert!(report.differences.iter().any(|v| v.contains("damage")));
}
#[test]
fn text_suffix_limitation_preserves_nonempty_field_presence() {
    let mut report = Report::default();
    optional_text_value(&mut report, "empty", b"", None);
    optional_text_value(&mut report, "NUL prefix", b"\0retained suffix", Some(b""));
    assert!(report.differences.is_empty());
    optional_text_value(&mut report, "missing field", b"\0retained suffix", None);
    assert_eq!(report.differences.len(), 1);
}
#[test]
fn division_naming_follows_the_division_name_writer_and_member_template() {
    let (standard, fake) = (1_u64 << 32 | 52, 2_u64 << 32 | 52);
    let template = |group: Option<&[u8]>| save::DivisionTemplate {
        origin_type: None,
        template: 0,
        country: b"SOV".to_vec(),
        name: Vec::new(),
        name_present: false,
        name_key: Vec::new(),
        name_key_present: false,
        custom_icon: 0,
        custom_icon_nato: false,
        foreign_template_tag: None,
        composition: BTreeMap::new(),
        names_group: group.map(<[u8]>::to_vec),
        ..Default::default()
    };
    let (older, foreign) = (3_u64 << 32 | 52, 4_u64 << 32 | 52);
    let saved_templates = BTreeMap::from([
        (standard, template(Some(b"SOV_INF_01"))),
        (fake, template(None)),
        (older, template(Some(b"SOV_INF_02"))),
        (
            foreign,
            save::DivisionTemplate {
                country: b"GER".to_vec(),
                ..template(Some(b"GER_INF_01"))
            },
        ),
    ]);
    let templates = NameTemplates {
        saved: &saved_templates,
        recorded: Ok(BTreeSet::from([standard, fake])),
    };
    let unit = |name: Option<save::DivisionName>, alternate| save::Division {
        id: 7,
        roster_owner: b"SOV".to_vec(),
        logical_owner: b"SOV".to_vec(),
        expeditionary_owner: None,
        province: Some(1),
        template: Some(standard),
        alternate_template: alternate,
        separate_counter: false,
        hit_points: Decimal::parse(b"1").unwrap(),
        organisation: Decimal::parse(b"1").unwrap(),
        country_intel: Vec::new(),
        presentation: save::DivisionPresentation {
            division_name: name,
            ..Default::default()
        },
    };
    let row = replay_data::rows::NameOperands {
        order: 0,
        ordered: 1,
        group: b"SOV_INF_01".to_vec(),
        ..Default::default()
    };
    let mut report = Report::default();
    let saved = Some(save::DivisionName::default());
    naming(
        &mut report,
        "ordered",
        &row,
        &unit(saved.clone(), None),
        &templates,
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    naming(
        &mut report,
        "fake intel",
        &row,
        &unit(saved, Some(fake)),
        &templates,
    );
    assert_eq!(report.differences.len(), 1);
    let custom = replay_data::rows::NameOperands {
        order: -1,
        custom: b"Guards".to_vec(),
        ..Default::default()
    };
    let named = save::DivisionName {
        order: Some(-1),
        ordered: Some(false),
        custom: Some(b"Guards".to_vec()),
        ..Default::default()
    };
    let mut report = Report::default();
    naming(
        &mut report,
        "custom",
        &custom,
        &unit(Some(named.clone()), None),
        &templates,
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let programmatic = replay_data::rows::NameOperands {
        programmatic: 1,
        custom: Vec::new(),
        group: b"SOV_INF_01".to_vec(),
        ..custom.clone()
    };
    let generated = save::DivisionName {
        programmatic: Some(true),
        custom: Some(b"1. Strelkovaya".to_vec()),
        ..named
    };
    naming(
        &mut report,
        "generated",
        &programmatic,
        &unit(Some(generated.clone()), None),
        &templates,
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(report.limitations.is_empty());
    let shown = replay_data::rows::NameOperands {
        custom: b"1. Strelkovaya".to_vec(),
        group: Vec::new(),
        ..programmatic.clone()
    };
    naming(
        &mut report,
        "shown",
        &shown,
        &unit(Some(generated.clone()), Some(fake)),
        &templates,
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let operands = replay_data::rows::NameOperands {
        order: custom.order,
        ordered: custom.ordered,
        programmatic: 1,
        custom: shown.custom.clone(),
        ..Default::default()
    };
    let mut operand_report = Report::default();
    naming(
        &mut operand_report,
        "whole roster generated",
        &operands,
        &unit(Some(generated.clone()), Some(fake)),
        &templates,
    );
    assert!(operand_report.differences.is_empty());
    assert!(operand_report.limitations.is_empty());
    for (label, row, alternate) in [
        ("unrecorded", &programmatic, Some(fake)),
        (
            "recorded",
            &replay_data::rows::NameOperands {
                group: b"SOV_INF_01".to_vec(),
                ..shown.clone()
            },
            None,
        ),
    ] {
        let mut report = Report::default();
        naming(
            &mut report,
            label,
            row,
            &unit(Some(generated.clone()), alternate),
            &templates,
        );
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("override")),
            "{label}: {:?}",
            report.differences
        );
    }
    let mut report = Report::default();
    naming(
        &mut report,
        "ship",
        &replay_data::rows::NameOperands {
            kind: 1,
            ..programmatic.clone()
        },
        &unit(
            Some(save::DivisionName {
                kind: 1,
                ..generated.clone()
            }),
            None,
        ),
        &templates,
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.limitations.len(), 1);
    for (label, saved) in [
        (
            "type",
            save::DivisionName {
                kind: 1,
                ..named.clone()
            },
        ),
        (
            "order",
            save::DivisionName {
                order: None,
                ..named.clone()
            },
        ),
        (
            "ordered",
            save::DivisionName {
                ordered: None,
                ..named.clone()
            },
        ),
        (
            "custom",
            save::DivisionName {
                custom: None,
                ..named.clone()
            },
        ),
        (
            "programmatic",
            save::DivisionName {
                programmatic: Some(true),
                ..named
            },
        ),
    ] {
        let mut report = Report::default();
        naming(
            &mut report,
            label,
            &custom,
            &unit(Some(saved), None),
            &templates,
        );
        assert_eq!(report.differences.len(), 1, "{label}");
    }
    let mut report = Report::default();
    naming(
        &mut report,
        "absent",
        &custom,
        &unit(None, None),
        &templates,
    );
    assert_eq!(report.differences.len(), 1);
    let deleted = 999_u64 << 32 | 52;
    let run = |operands: Naming<'_>, templates: &NameTemplates<'_>| {
        let mut report = Report::default();
        naming(
            &mut report,
            "member",
            operands,
            &unit(Some(generated.clone()), None),
            templates,
        );
        report
    };
    let report = run(
        Naming::from(&shown).with_template(MemberTemplate::Recorded(deleted)),
        &templates,
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(report.limitations.is_empty(), "{:?}", report.limitations);
    assert_eq!(
        report.compared_tables["name members of deleted templates"],
        1
    );
    for (label, operands, field) in [
        (
            "wrong override",
            Naming {
                custom: b"2. Strelkovaya",
                ..Naming::from(&shown)
            },
            "override",
        ),
        (
            "recorded group",
            Naming {
                custom: b"",
                ..Naming::from(&programmatic)
            },
            "name group",
        ),
    ] {
        let report = run(
            operands.with_template(MemberTemplate::Recorded(deleted)),
            &templates,
        );
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains(field)),
            "{label}: {:?}",
            report.differences
        );
    }
    let held = NameTemplates {
        saved: &saved_templates,
        recorded: Ok(BTreeSet::from([standard, fake, deleted])),
    };
    let report = run(
        Naming::from(&shown).with_template(MemberTemplate::Recorded(deleted)),
        &held,
    );
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("name member template"));
    let report = run(
        Naming::from(&shown).with_template(MemberTemplate::Recorded(999 << 32 | 53)),
        &templates,
    );
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("name member template"));
    let stale = NameTemplates {
        saved: &saved_templates,
        recorded: Err("stale"),
    };
    let report = run(
        Naming::from(&shown).with_template(MemberTemplate::Recorded(deleted)),
        &stale,
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(report.limitations.contains_key("stale"));
    let report = run(
        Naming::from(&shown).with_template(MemberTemplate::Recorded(0)),
        &templates,
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let report = run(
        Naming::from(&shown).with_template(MemberTemplate::Absent),
        &templates,
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        report.limitations.keys().collect::<Vec<_>>(),
        [MEMBER_TEMPLATE_ABSENT]
    );
    for (operands, admitted) in [
        (Naming::from(&shown), true),
        (
            Naming {
                group: b"SOV_INF_02",
                ..Naming::from(&programmatic)
            },
            true,
        ),
        (
            Naming {
                group: b"GER_INF_01",
                ..Naming::from(&programmatic)
            },
            false,
        ),
        (
            Naming {
                custom: b"2. Strelkovaya",
                ..Naming::from(&shown)
            },
            false,
        ),
    ] {
        let deleted_group = operands.group.is_empty();
        let report = run(operands, &templates);
        assert_eq!(
            report.differences.is_empty(),
            admitted,
            "{:?}",
            report.differences
        );
        assert!(
            report
                .limitations
                .contains_key(MEMBER_TEMPLATE_NOT_IN_RECORDING)
                == (admitted || deleted_group),
            "{:?}",
            report.limitations
        );
    }
    let report = run(
        Naming::from(&programmatic).with_template(MemberTemplate::Recorded(fake)),
        &templates,
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("name group")),
        "{:?}",
        report.differences
    );
}

#[test]
fn combat_members_take_their_recorded_template_from_the_division_status() {
    use replay_data::{rows::DivisionStatusRow, rows::LandCombatDivisionRow, rows::Witnessed};
    let (mut frame, mut save, ..) = super::super::fixtures::fixture();
    let combat = (7_u64 << 32) | 62;
    let division = 0x9000_0001_u64;
    let (standard, deleted) = (1_u64 << 32 | 52, 999_u64 << 32 | 52);
    save.land_combats.insert(
        combat,
        save::LandCombat {
            id: combat,
            province: 2,
            details: Default::default(),
            day: 0,
            duration: 0,
            attackers: vec![division],
            defenders: Vec::new(),
        },
    );
    save.division_templates.insert(
        standard,
        save::DivisionTemplate {
            origin_type: None,
            template: 0,
            country: b"SOV".to_vec(),
            name: Vec::new(),
            name_present: false,
            name_key: Vec::new(),
            name_key_present: false,
            custom_icon: 0,
            custom_icon_nato: false,
            foreign_template_tag: None,
            composition: BTreeMap::new(),
            names_group: Some(b"SOV_INF_01".to_vec()),
            ..Default::default()
        },
    );
    save.divisions.insert(
        division,
        save::Division {
            id: division,
            roster_owner: b"SOV".to_vec(),
            logical_owner: b"SOV".to_vec(),
            expeditionary_owner: None,
            province: Some(2),
            template: Some(standard),
            alternate_template: None,
            separate_counter: false,
            hit_points: Decimal::parse(b"1").unwrap(),
            organisation: Decimal::parse(b"1").unwrap(),
            country_intel: Vec::new(),
            presentation: save::DivisionPresentation {
                division_name: Some(save::DivisionName {
                    programmatic: Some(true),
                    custom: Some(b"1. Strelkovaya".to_vec()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        },
    );
    *frame.tables[TableId::LandCombatDivisions.index()] =
        TableView::Complete(TableRows::LandCombatDivisions(vec![
            LandCombatDivisionRow {
                combat,
                side: 0,
                ordinal: 0,
                role: 1,
                province: 2,
                division,
                context: replay_data::rows::LandDivisionContext {
                    template: standard,
                    ..Default::default()
                },
                own: Some(replay_data::rows::LandCombatMemberValues {
                    naming: replay_data::rows::NameOperands {
                        ordered: 1,
                        programmatic: 1,
                        custom: b"1. Strelkovaya".to_vec(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            },
        ]));
    let naming_report = |frame: &DecodedFrame| {
        let mut report = Report::default();
        divisions(frame, &save, &mut report).map(|()| {
            let differences = report
                .differences
                .into_iter()
                .filter(|difference| {
                    difference.contains("override") || difference.contains("name ")
                })
                .collect::<Vec<_>>();
            (differences, report.limitations)
        })
    };
    let naming_differences = |frame: &DecodedFrame| naming_report(frame).map(|(d, _)| d);
    let status = |template: Option<u64>, naming: Option<replay_data::rows::NameOperands>| {
        TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
            division,
            name_template: Recorded::Value(Witnessed(template)),
            naming: Recorded::Value(naming),
            ..Default::default()
        }]))
    };
    let operands = replay_data::rows::NameOperands::default();
    *frame.tables[TableId::DivisionStatus.index()] = status(Some(deleted), Some(operands.clone()));
    let (differences, limitations) = naming_report(&frame).unwrap();
    assert_eq!(differences, Vec::<String>::new());
    assert!(!limitations.contains_key(MEMBER_TEMPLATE_NOT_IN_RECORDING));
    assert!(!limitations.contains_key(MEMBER_TEMPLATE_ABSENT));
    *frame.tables[TableId::DivisionStatus.index()] = status(None, Some(operands.clone()));
    let (differences, limitations) = naming_report(&frame).unwrap();
    assert_eq!(differences, Vec::<String>::new());
    assert!(limitations.contains_key(MEMBER_TEMPLATE_NOT_IN_RECORDING));
    *frame.tables[TableId::DivisionStatus.index()] = status(None, None);
    let (differences, limitations) = naming_report(&frame).unwrap();
    assert_eq!(differences, Vec::<String>::new());
    assert!(limitations.contains_key(MEMBER_TEMPLATE_ABSENT));
    assert!(!limitations.contains_key(MEMBER_TEMPLATE_NOT_IN_RECORDING));
    *frame.tables[TableId::DivisionStatus.index()] = status(Some(deleted), Some(operands));
    *frame.tables[TableId::DivisionTemplates.index()] =
        TableView::Complete(TableRows::DivisionTemplates(vec![
            replay_data::rows::DivisionTemplateRow {
                template: deleted,
                ..Default::default()
            },
        ]));
    assert_eq!(naming_differences(&frame).unwrap().len(), 1);
    *frame.tables[TableId::DivisionStatus.index()] = TableView::Unavailable {
        reason: "synthetic".into(),
    };
    assert!(naming_differences(&frame).is_err());
}

#[test]
fn commander_formations_match_the_saved_leader_group() {
    let formations = BTreeMap::from([(7_u64, 11_u64)]);
    let side = |commander, commander_formation| replay_data::rows::LandCombatSideDetails {
        commander,
        commander_formation,
        commander_category: 0,
        ..Default::default()
    };
    let mut report = Report::default();
    formation(&mut report, "led", &side(7, 11), &formations);
    formation(&mut report, "no commander", &side(0, 0), &formations);
    formation(&mut report, "unsaved", &side(13, 5), &formations);
    assert!(report.differences.is_empty());
    assert_eq!(report.limitations.len(), 1);
    formation(&mut report, "wrong group", &side(7, 12), &formations);
    formation(&mut report, "no commander", &side(0, 11), &formations);
    let mut navy = side(7, 11);
    navy.commander_category = 2;
    formation(&mut report, "navy leader", &navy, &formations);
    assert_eq!(report.differences.len(), 3);
}

#[test]
fn organisation_loss_compares_the_recorded_sum_with_the_saved_samples() {
    use replay_data::{
        rows::LandBorderWarInputs, rows::LandBorderWarSide, rows::LandCombatDetailsRow,
    };
    let (mut frame, mut save, ..) = super::super::fixtures::fixture();
    let combat = (7_u64 << 32) | 63;
    let border = LandBorderWarInputs {
        transfer_state: 128,
        combat_width: Recorded::NotRecorded,
        sides: std::array::from_fn(|_| LandBorderWarSide {
            country: 102,
            state: 11,
            ..Default::default()
        }),
    };
    let samples: Vec<i64> = (0..24)
        .map(|slot| if slot == 0 { i64::MAX } else { slot })
        .collect();
    let sum = samples
        .iter()
        .fold(0_i64, |sum, value| sum.wrapping_add(*value));
    let mut row = LandCombatDetailsRow {
        combat,
        province: 2,
        border_war: Recorded::Value(Some(border)),
        ..Default::default()
    };
    for side in &mut row.sides {
        side.organisation_loss_count = 24;
        side.organisation_loss_sum = sum;
    }
    *frame.tables[TableId::LandCombatDetails.index()] =
        TableView::Complete(TableRows::LandCombatDetails(vec![row]));
    let mut details = save::combat_details::LandDetails {
        border_war: true,
        transfer_state: Some(true),
        ..Default::default()
    };
    let decimals = |values: &[i64]| {
        values
            .iter()
            .map(|&raw| save::Decimal {
                raw,
                fractional_digits: 5,
                lexeme: Vec::new(),
            })
            .collect::<Vec<_>>()
    };
    for side in &mut details.sides {
        side.border_country = Some(b"GER".to_vec());
        side.border_state = Some(11);
        side.organisation_loss_count = Some(24);
        side.organisation_loss = Some(decimals(&samples));
    }
    save.land_combats.insert(
        combat,
        save::LandCombat {
            id: combat,
            province: 2,
            details,
            day: 0,
            duration: 0,
            attackers: Vec::new(),
            defenders: Vec::new(),
        },
    );
    let mut report = Report::default();
    land(&frame, &save, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let saved = &mut save.land_combats.get_mut(&combat).unwrap().details;
    let mut moved = samples.clone();
    moved[5] += 1;
    saved.sides[0].organisation_loss = Some(decimals(&moved));
    saved.sides[1].organisation_loss = Some(decimals(&samples[..23]));
    let mut report = Report::default();
    land(&frame, &save, &mut report).unwrap();
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
}

#[test]
fn reserve_trees_name_exactly_the_reserves_sources() {
    use replay_data::{
        rows::DivisionRow, rows::LandCombatDivisionRow, rows::ReserveModifierTreeRow,
    };
    let (mut frame, ..) = super::super::fixtures::fixture();
    let mut reserve = LandCombatDivisionRow {
        role: 2,
        division: 11,
        commander: 77,
        ..Default::default()
    };
    reserve.context.leader_modifier_kind = 1;
    let committed = LandCombatDivisionRow {
        role: 1,
        division: 12,
        ordinal: 1,
        ..Default::default()
    };
    *frame.tables[TableId::LandCombatDivisions.index()] =
        TableView::Complete(TableRows::LandCombatDivisions(vec![reserve, committed]));
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(vec![
        DivisionRow {
            id: 11,
            roster_owner: 4,
            expeditionary_owner: 9,
            ..Default::default()
        },
        DivisionRow {
            id: 12,
            roster_owner: 5,
            ..Default::default()
        },
    ]));
    let tree = |kind, owner| ReserveModifierTreeRow {
        kind,
        owner,
        ..Default::default()
    };
    let check = |frame: &mut DecodedFrame, rows: Vec<ReserveModifierTreeRow>| {
        *frame.tables[TableId::ReserveModifierTrees.index()] =
            TableView::Complete(TableRows::ReserveModifierTrees(rows));
        let mut report = Report::default();
        reserve_trees(frame, &mut report).unwrap();
        report.differences.len()
    };
    assert_eq!(check(&mut frame, vec![tree(0, 9), tree(1, 77)]), 0);
    assert_eq!(check(&mut frame, vec![tree(0, 9)]), 0);
    assert_eq!(check(&mut frame, vec![tree(0, 4)]), 1);
    assert_eq!(check(&mut frame, vec![tree(0, 5), tree(0, 9)]), 1);
    assert_eq!(check(&mut frame, vec![tree(0, 9), tree(2, 77)]), 1);
}
