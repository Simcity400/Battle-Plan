use super::*;
use replay::{
    frame::TableView,
    rows::{
        AirBaseRow, AirWingRow, NavalCombatAirBaseWingHandlesRow, NavalCombatShipRow, TableRows,
    },
};

#[test]
fn air_base_wing_rows_must_be_exactly_the_referenced_bases_in_the_air_censuses() {
    fn set(frame: &mut DecodedFrame, rows: TableRows) {
        let index = rows.table().index();
        *frame.tables[index] = TableView::Complete(rows);
    }
    let (mut frame, ..) = super::super::tests::fixture();
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
    // An unreferenced base, a base absent from AirBases and a wing absent from AirWings.
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
}

use replay::rows::{LandCombatCommanderTimedTrait, LandCombatSideDetails};

#[test]
fn naval_modifier_breakdowns_check_runtime_joins_without_claiming_save_values() {
    use replay::rows::{
        ModifierBreakdownNode, NavalCombatDetailsRow, NavalModifierBreakdownRow, NavalModifierUse,
    };
    fn set(frame: &mut DecodedFrame, rows: TableRows) {
        let index = rows.table().index();
        *frame.tables[index] = TableView::Complete(rows);
    }
    let (mut frame, ..) = super::super::tests::fixture();
    set(
        &mut frame,
        TableRows::NavalCombatDetails(vec![NavalCombatDetailsRow {
            combat: 123,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::NavalCombatShips(vec![NavalCombatShipRow {
            combat: 123,
            side: 1,
            battle_slot: 7,
            state: 8,
            ..Default::default()
        }]),
    );
    let tree = NavalModifierBreakdownRow {
        accumulator: 0x5678,
        key: 0x5b,
        uses: vec![NavalModifierUse {
            combat: 123,
            side: 1,
            battle_slot: 7,
            source_scope: 1,
            ..Default::default()
        }],
        nodes: vec![ModifierBreakdownNode {
            children: 0,
            value: 42,
            name: b"Runtime source".to_vec(),
        }],
    };
    set(
        &mut frame,
        TableRows::NavalModifierBreakdowns(vec![tree.clone()]),
    );
    let mut report = Report::default();
    naval_modifier_breakdowns(&frame, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["NavalModifierBreakdowns"], 1);
    assert!(
        report
            .limitations
            .keys()
            .any(|text| text.contains("does not serialize"))
    );

    let mut bad = tree;
    bad.uses.push(bad.uses[0].clone());
    bad.uses[1].combat = 999;
    set(&mut frame, TableRows::NavalModifierBreakdowns(vec![bad]));
    let mut report = Report::default();
    naval_modifier_breakdowns(&frame, &mut report).unwrap();
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    assert!(
        report
            .differences
            .iter()
            .any(|text| text.contains("source order"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|text| text.contains("999 combat"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|text| text.contains("999 slot"))
    );
}

#[test]
fn naval_air_localized_cache_is_not_a_saved_plane_comparator() {
    let date = installed_content::save::SavedDate::parse(b"1936.1.1.1").unwrap();
    let mut saved = installed_content::save::combat_details::NavalPlane {
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
        damage_mult: installed_content::save::Decimal::parse(b"1").unwrap(),
        naval_hits: Vec::new(),
        air_hits: Vec::new(),
    };
    let row = replay::rows::NavalCombatPlane {
        carrier_slot: -1,
        date: date.engine_hour().unwrap(),
        ..Default::default()
    };
    let compare = |saved: &installed_content::save::combat_details::NavalPlane| {
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

/// Each side's task forces compare with the saved `unit` list in writer order.
#[test]
fn naval_side_units_compare_with_the_saved_unit_list_in_order() {
    use replay::{
        frame::TableView,
        rows::{NavalCombatDetailsRow, TableId, TableRows},
    };
    let (frame, mut saved, _, _) = crate::oracle::tests::fixture();
    let combat = (5_u64 << 32) | 4713;
    let zero = || installed_content::save::Decimal::parse(b"0").unwrap();
    let units = [(244_u64 << 32) | 61, (9 << 32) | 61];
    saved.naval_combats.insert(
        combat,
        installed_content::save::NavalCombat {
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
    let save = installed_content::save::parse(
        br#"HOI4txt
        date=1942.9.6.2 game_unique_id=test version=test game_unique_seed=1 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
        countries={} states={} provinces={}
        equipment_market={contracts={}} supply_system_2={countries={}}
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
    let side = LandCombatSideDetails {
        commander: (4 << 32) | 4713,
        commander_category: 1,
        timed_traits: vec![timed(1, 20), timed(2, -1)],
        ..Default::default()
    };
    let tokens: TokenNames<'static> =
        BTreeMap::from([(1, b"sick".as_slice()), (2, b"wounded".as_slice())]);
    let compare = |side: &LandCombatSideDetails, tokens: &TokenNames<'_>| {
        let mut report = Report::default();
        timed_traits(&mut report, "side", side, &save, tokens);
        report
    };
    let report = compare(&side, &tokens);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    // Vector order and exact signed days, never a set.
    let swapped = LandCombatSideDetails {
        timed_traits: vec![timed(2, -1), timed(1, 20)],
        ..side.clone()
    };
    assert_eq!(compare(&swapped, &tokens).differences.len(), 4);
    // An unspelled token limits only its own spelling; days still compare.
    let report = compare(&side, &TokenNames::new());
    assert!(report.differences.is_empty());
    assert_eq!(report.limitations.values().flatten().count(), 2);
    // A capture longer than the save is a count difference.
    let extra = LandCombatSideDetails {
        timed_traits: vec![timed(1, 20), timed(2, -1), timed(1, 3)],
        ..side.clone()
    };
    assert_eq!(compare(&extra, &tokens).differences.len(), 1);
    // No commander carries no timed traits.
    let absent = LandCombatSideDetails {
        commander: 0,
        ..side.clone()
    };
    assert_eq!(compare(&absent, &tokens).differences.len(), 1);
}

#[test]
fn null_naval_commander_is_a_limitation_only_at_an_observed_load_hour() {
    use replay::branches::BranchReason::{
        HourDiscontinuity as Gap, LoadSignal as Load, SessionStart as Start,
    };
    use replay::branches::{BranchStart, Parentage};
    use replay::rows::{NavalCombatDetailsRow, NavalCombatSideDetails};

    let (mut frame, mut save, _, _) = super::super::tests::fixture();
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
            .any(|key| key.starts_with("Naval commander is null at the load hour"))
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

    // Only a null commander at an observed load's loaded and first sampled
    // hour is a limitation; any other branch start, hour or value differs.
    let h = frame.hour;
    for (reason, loaded, first, sample, commander, expected) in [
        (Load, Some(h), h, h, 0, "limited"),
        (Load, Some(h), h, h, leader, "equal"),
        (Load, Some(h), h, h, leader + 1, "differs"),
        (Load, Some(h), h, h + 1, 0, "differs"),
        (Load, None, h, h, 0, "differs"),
        (Load, Some(h), h + 1, h, 0, "differs"),
        (Start, Some(h), h, h, 0, "differs"),
        (Gap, Some(h), h, h, 0, "differs"),
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

    // Other fields of the same side stay exact at the load hour.
    frame.hour = h;
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
    let a = replay::rows::NavalCombatHit {
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
        template: 0,
        country: b"SOV".to_vec(),
        name: Vec::new(),
        custom_icon: 0,
        custom_icon_nato: false,
        foreign_template_tag: None,
        composition: BTreeMap::new(),
        names_group: group.map(<[u8]>::to_vec),
    };
    let templates = BTreeMap::from([
        (standard, template(Some(b"SOV_INF_01"))),
        (fake, template(None)),
    ]);
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
    // An ordered historical member: every conditional key is omitted.
    let row = replay::rows::LandCombatDivisionRow {
        name_order: 0,
        name_ordered: 1,
        name_group: b"SOV_INF_01".to_vec(),
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
    // A fake-intel template supplies the member's group instead.
    naming(
        &mut report,
        "fake intel",
        &row,
        &unit(saved.clone(), Some(fake)),
        &templates,
    );
    assert_eq!(report.differences.len(), 1);
    // A player's name, an unordered member and a nonzero order word.
    let custom = replay::rows::LandCombatDivisionRow {
        name_order: -1,
        name_custom: b"Guards".to_vec(),
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
    // A programmatic override with a set group is not shown (1409B5640)
    // and not recorded: the group is compared instead.
    let programmatic = replay::rows::LandCombatDivisionRow {
        name_programmatic: 1,
        name_custom: Vec::new(),
        name_group: b"SOV_INF_01".to_vec(),
        ..custom.clone()
    };
    let generated = save::DivisionName {
        programmatic: Some(true),
        custom: Some(b"1. Strelkovaya".to_vec()),
        ..named.clone()
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
    // With a null group (the fake-intel template saves none) the game
    // shows it, and it is compared.
    let shown = replay::rows::LandCombatDivisionRow {
        name_custom: b"1. Strelkovaya".to_vec(),
        name_group: Vec::new(),
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
    for (label, row, alternate) in [
        // Not recorded where the game shows it.
        ("unrecorded", &programmatic, Some(fake)),
        // Recorded where the game shows the group's entry.
        ("recorded", &shown, None),
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
    // A nonzero type's group is not read: a named limitation.
    let mut report = Report::default();
    naming(
        &mut report,
        "ship",
        &replay::rows::LandCombatDivisionRow {
            name_type: 1,
            ..programmatic.clone()
        },
        &unit(
            Some(save::DivisionName {
                kind: 1,
                ..generated
            }),
            None,
        ),
        &templates,
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.limitations.len(), 1);
    // Each operand difference and a missing block are reported, not waived.
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
                ..named.clone()
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
}

#[test]
fn commander_formations_match_the_saved_leader_group() {
    let formations = BTreeMap::from([(7_u64, 11_u64)]);
    let side = |commander, commander_formation| replay::rows::LandCombatSideDetails {
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
