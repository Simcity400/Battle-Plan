use super::fixtures::{add_country, division_fixture, fixture};
use super::*;
mod name_serialization;
mod original_tag_refusal;
mod template_display_name;
use replay::rows::{
    ClockRow, CountryLinkRow, DivisionRow, DivisionStatusRow, FactionMemberRow, FactionRow,
    ProvinceRow, Recorded, StateRow, WarRelationRow,
};

fn compare(
    frame: &DecodedFrame,
    save: &Save,
    membership: &save::PartialMapMembership,
    equipment: &installed_content::equipment::Database,
) -> io::Result<Report> {
    compare_at(
        frame,
        save,
        membership,
        equipment,
        SampleBoundary::EndOfHour,
    )
}

fn compare_at(
    frame: &DecodedFrame,
    save: &Save,
    membership: &save::PartialMapMembership,
    equipment: &installed_content::equipment::Database,
    boundary: SampleBoundary,
) -> io::Result<Report> {
    compare_core(frame, boundary, save, Ok(membership), Ok(equipment), None)
}

fn comparison_error(result: io::Result<Report>) -> String {
    match result {
        Err(error) => error.to_string(),
        Ok(report) => {
            assert!(report.outcome().is_err());
            report
                .failed_comparisons
                .values()
                .chain(
                    report
                        .recorded_tables
                        .values()
                        .filter_map(|rows| rows.as_ref().err()),
                )
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}

/// A combat's transfer vector and a named live transfer's units compare in writer order;
/// a province entry joins the census of its class.
#[test]
fn naval_combat_transfers_and_units_compare_in_writer_order() {
    use replay::rows::{
        NavalCombatRow, NavalCombatTransferRow, NavalProvinceCombatRow, NavalTransferUnitRow,
    };
    let (mut frame, mut saved, _, _) = fixture();
    let handle = |id: u64| (id << 32) | 4713;
    let (combat, live, stale, unit) = (handle(5), handle(7), handle(8), handle(9));
    let zero = || Decimal::parse(b"0").unwrap();
    saved.naval_combats.insert(
        combat,
        save::NavalCombat {
            id: combat,
            province: 2,
            attackers: Vec::new(),
            defenders: Vec::new(),
            attacker_positioning: zero(),
            defender_positioning: zero(),
            attacker_anti_air: zero(),
            defender_anti_air: zero(),
            details: Default::default(),
            transfers: vec![live, 0, stale],
        },
    );
    saved
        .counter_inputs
        .naval_transfers
        .insert(live, vec![unit]);
    let set = |frame: &mut DecodedFrame, table: TableId, rows: TableRows| {
        *frame.tables[table.index()] = TableView::Complete(rows);
    };
    set(
        &mut frame,
        TableId::NavalCombats,
        TableRows::NavalCombats(vec![NavalCombatRow {
            id: combat,
            province: 2,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableId::NavalProvinceCombats,
        TableRows::NavalProvinceCombats(vec![NavalProvinceCombatRow {
            province: 2,
            combats: vec![combat],
            kinds: vec![NavalProvinceCombatRow::NAVAL],
        }]),
    );
    let transfers = |transfers: Vec<u64>| {
        TableRows::NavalCombatTransfers(vec![NavalCombatTransferRow { combat, transfers }])
    };
    let units = |rows: Vec<(u64, Vec<u64>)>| {
        TableRows::NavalTransferUnits(
            rows.into_iter()
                .map(|(transfer, units)| NavalTransferUnitRow { transfer, units })
                .collect(),
        )
    };
    set(
        &mut frame,
        TableId::NavalCombatTransfers,
        transfers(vec![live, 0, stale]),
    );
    set(
        &mut frame,
        TableId::NavalTransferUnits,
        units(vec![(live, vec![unit])]),
    );
    let differences = |frame: &DecodedFrame| {
        let mut report = Report::default();
        compare_naval_combat_sources(frame, &saved, &mut report).unwrap();
        report.differences.len()
    };
    assert_eq!(differences(&frame), 0);
    // Another order, an extra unit, a row for the stale transfer, a missing units row and a
    // province entry of the wrong class each differ.
    set(
        &mut frame,
        TableId::NavalCombatTransfers,
        transfers(vec![0, live, stale]),
    );
    assert_eq!(differences(&frame), 1);
    set(
        &mut frame,
        TableId::NavalCombatTransfers,
        transfers(vec![live, 0, stale]),
    );
    for rows in [
        vec![(live, vec![unit, handle(10)])],
        vec![(live, vec![unit]), (stale, vec![])],
        vec![],
    ] {
        set(&mut frame, TableId::NavalTransferUnits, units(rows));
        assert_eq!(differences(&frame), 1);
    }
    set(
        &mut frame,
        TableId::NavalTransferUnits,
        units(vec![(live, vec![unit])]),
    );
    set(
        &mut frame,
        TableId::NavalProvinceCombats,
        TableRows::NavalProvinceCombats(vec![NavalProvinceCombatRow {
            province: 2,
            combats: vec![combat],
            kinds: vec![NavalProvinceCombatRow::LAND],
        }]),
    );
    assert_eq!(differences(&frame), 1);
}

#[test]
fn state_flags_compare_boolean_and_full_census_without_missing_field_defaults() {
    use replay::rows::StateFlagRow;
    let (mut frame, mut saved, _, _) = fixture();
    let row = StateFlagRow {
        border_conflict_active: replay::rows::Recorded::Value(0),
        border_conflict_countries: replay::rows::Recorded::Value(Vec::new()),
        state: 1,
        demilitarized: 0,
    };
    let mut report = Report::default();
    compare_state_flags(&frame, &saved, &mut report).unwrap();
    assert!(report.difference_lines().is_empty());

    saved.states.get_mut(&1).unwrap().demilitarized = Some(true);
    let mut report = Report::default();
    compare_state_flags(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    for byte in [1, 2, 255] {
        *frame.tables[TableId::StateFlags.index()] =
            TableView::Complete(TableRows::StateFlags(vec![StateFlagRow {
                demilitarized: byte,
                ..row.clone()
            }]));
        let mut report = Report::default();
        compare_state_flags(&frame, &saved, &mut report).unwrap();
        assert!(report.difference_lines().is_empty());
    }
    saved.states.get_mut(&1).unwrap().demilitarized = None;
    assert!(
        compare_state_flags(&frame, &saved, &mut Report::default())
            .unwrap_err()
            .to_string()
            .contains("lacks its demilitarized")
    );
    saved.states.get_mut(&1).unwrap().demilitarized = Some(false);

    for rows in [
        vec![],
        vec![
            row.clone(),
            StateFlagRow {
                border_conflict_active: replay::rows::Recorded::Value(0),
                border_conflict_countries: replay::rows::Recorded::Value(Vec::new()),
                state: 978,
                demilitarized: 1,
            },
        ],
    ] {
        *frame.tables[TableId::StateFlags.index()] =
            TableView::Complete(TableRows::StateFlags(rows));
        let mut report = Report::default();
        compare_state_flags(&frame, &saved, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }
    for rows in [
        vec![row.clone(), row.clone()],
        vec![StateFlagRow::default()],
    ] {
        *frame.tables[TableId::StateFlags.index()] =
            TableView::Complete(TableRows::StateFlags(rows));
        assert!(compare_state_flags(&frame, &saved, &mut Report::default()).is_err());
    }
    *frame.tables[TableId::StateFlags.index()] = TableView::Truncated {
        rows: TableRows::StateFlags(vec![row]),
        missing: 1,
    };
    assert!(compare_state_flags(&frame, &saved, &mut Report::default()).is_err());
    *frame.tables[TableId::StateFlags.index()] = TableView::Unavailable {
        reason: "state source fault".into(),
    };
    assert!(compare_state_flags(&frame, &saved, &mut Report::default()).is_err());
}

#[test]
fn state_border_flag_compares_saved_boolean_and_rejects_missing_witness() {
    use replay::rows::StateFlagRow;
    let (mut frame, mut saved, _, _) = fixture();
    saved.states.get_mut(&1).unwrap().is_border_conflict = Some(true);
    let mut report = Report::default();
    compare_state_flags(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    for byte in [1, 2, 255] {
        *frame.tables[TableId::StateFlags.index()] =
            TableView::Complete(TableRows::StateFlags(vec![StateFlagRow {
                state: 1,
                border_conflict_active: replay::rows::Recorded::Value(byte),
                ..Default::default()
            }]));
        let mut report = Report::default();
        compare_state_flags(&frame, &saved, &mut report).unwrap();
        assert!(report.difference_lines().is_empty());
    }
    saved.states.get_mut(&1).unwrap().is_border_conflict = None;
    assert!(
        compare_state_flags(&frame, &saved, &mut Report::default())
            .unwrap_err()
            .to_string()
            .contains("lacks its is_border_conflict")
    );
}

#[test]
fn province_names_compare_sparse_add_remove_and_exact_bytes() {
    let (mut frame, mut save, _, _) = fixture();
    frame.hour = 1;
    save.provinces.get_mut(&2).unwrap().name = Some(b"G".to_vec());
    *frame.tables[TableId::ProvinceNames.index()] =
        TableView::Complete(TableRows::ProvinceNames(vec![
            replay::rows::ProvinceNameRow {
                province: 2,
                name: vec![b'G', 0, 255],
            },
        ]));
    let mut report = Report::default();
    compare_province_names(&frame, &save, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.scalar_checks, 1);
    save.provinces.get_mut(&2).unwrap().name = None;
    compare_province_names(&frame, &save, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    *frame.tables[TableId::ProvinceNames.index()] =
        TableView::Complete(TableRows::ProvinceNames(vec![]));
    save.provinces.get_mut(&2).unwrap().name = Some(b"Gdansk".to_vec());
    let mut report = Report::default();
    compare_province_names(&frame, &save, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    // Read every hour: any other hour compares too.
    frame.hour = 2;
    let mut report = Report::default();
    compare_province_names(&frame, &save, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(
        !report
            .limitations
            .contains_key(replay::cadence::STAGGER_LIMITATION)
    );
}

fn division_row_mut(frame: &mut DecodedFrame) -> &mut DivisionRow {
    let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *frame.tables[TableId::Divisions.index()]
    else {
        unreachable!()
    };
    &mut rows[0]
}

#[test]
fn division_presentation_oracle_checks_prefix_census_exact_values_and_omissions() {
    let (mut frame, mut saved, _, _) = division_fixture();
    let id = *saved.divisions.keys().next().unwrap();
    let path: Vec<u32> = (1..=300).collect();
    saved.divisions.get_mut(&id).unwrap().presentation = save::DivisionPresentation {
        fuel_requested: None,
        path: path.clone(),
        support_attack: Some(9),
        full_path: vec![9, 8, 9],
        previous: Some(7),
        movement_progress: Some(Decimal::parse(b"90071992547.40993").unwrap()),
        withdrawal: false,
        movement_priority: None,
        exile: false,
        dig_in: Some(Decimal::parse(b"0.00001").unwrap()),
        dig_in_cap: None,
        max_supply: Some(Decimal::parse(b"0").unwrap()),
        army_current_supply_ratio: Some(Decimal::parse(b"0").unwrap()),
        strategic_redeployment: true,
        disrupted_supply: None,
        out_of_supply_days: Some(30),
        fuel: Some(Decimal::parse(b"604.8").unwrap()),
        division_name: None,
        acclimatization: Some(save::Acclimatization {
            entries: vec![(b"hot_climate".to_vec(), Decimal::parse(b"8483.76").unwrap())],
            maximum: None,
        }),
    };
    let movement = replay::rows::DivisionMovementRow {
        division: id,
        target: 9,
        alternate_path: vec![9, 8, 9],
        alternate_total: 3,
        source: 7,
        numerator: 9_007_199_254_740_993,
        path_total: 300,
        path: path.clone(),
        strategic_redeployment: 255,
        withdrawal: Recorded::Value(Some(0)),
        movement_priority: Recorded::Value(Some(1)),
        base_speed: Recorded::Value(Some(400_000)),
        fuel: Recorded::Value(Some(60_480_000)),
        selected: Recorded::NotRecorded,
        ..Default::default()
    };
    // A recording before `camouflage` compares its entries.
    let status = DivisionStatusRow {
        division: id,
        supply: replay::rows::Recorded::NotRecorded,
        camouflage: replay::rows::Recorded::NotRecorded,
        out_of_supply_days: Recorded::Value(Some(30)),
        supply_ratio_exact: Recorded::Value(Some(0)),
        supply_grace_exact: Recorded::Value(Some(0)),
        entrenchment: 1,
        entrenchment_cap: -100,
        acclimatization_maximum: Some(save::Acclimatization::OMITTED_MAXIMUM_RAW),
        acclimatization: vec![
            replay::rows::DivisionAcclimatization {
                category: 9,
                weight: 0,
            },
            replay::rows::DivisionAcclimatization {
                category: 8,
                weight: 848_376_000,
            },
        ],
        ..Default::default()
    };
    let tokens = BTreeMap::from([(8, b"hot_climate".as_slice())]);
    *frame.tables[TableId::DivisionMovement.index()] =
        TableView::Complete(TableRows::DivisionMovement(vec![movement.clone()]));
    *frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(vec![status.clone()]));
    let mut report = Report::default();
    compare_division_movement(&frame, &saved, &mut report).unwrap();
    compare_division_status(&frame, &saved, &tokens, &Ok(50_000), &Ok(72), &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(report.limitations.iter().any(|(reason, subjects)| {
        reason.contains("remaining hours are estimated at playback")
            && reason.contains("route, progress numerator and base_speed")
            && reason.contains("days out of supply")
            && subjects
                .iter()
                .any(|subject| subject == "DivisionMovement arrival estimates")
    }));
    assert_eq!(report.scalar_checks, 19);
    // The fuel and days out of supply compare exactly, absence being zero.
    for (fuel, days, differences) in [
        (Some(&b"604.80001"[..]), Some(30), 1),
        (None, Some(30), 1),
        (Some(&b"604.8"[..]), None, 1),
        (Some(&b"604.8"[..]), Some(29), 1),
    ] {
        let mut changed = saved.clone();
        let presentation = &mut changed.divisions.get_mut(&id).unwrap().presentation;
        presentation.fuel = fuel.map(|fuel| Decimal::parse(fuel).unwrap());
        presentation.out_of_supply_days = days;
        let mut report = Report::default();
        compare_division_movement(&frame, &changed, &mut report).unwrap();
        compare_division_status(&frame, &changed, &tokens, &Ok(50_000), &Ok(72), &mut report)
            .unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
    }
    // The exact remaining grace compares below its display step, and is kept exactly
    // when days out of supply are nonzero; the exact ratio must round to the row's
    // stepped ratio.
    for (grace, exact, ratio, difference) in [
        (
            &b"0.5"[..],
            Some(0),
            Some(0),
            "supply_exact.grace_remaining",
        ),
        (&b"0"[..], None, None, "supply_exact"),
        (
            &b"0"[..],
            Some(0),
            Some(50_000),
            "supply_exact.ratio stepped",
        ),
    ] {
        let mut changed = saved.clone();
        changed
            .divisions
            .get_mut(&id)
            .unwrap()
            .presentation
            .army_current_supply_ratio = Some(Decimal::parse(grace).unwrap());
        let mut status = status.clone();
        status.supply_ratio_exact = Recorded::Value(ratio);
        status.supply_grace_exact = Recorded::Value(exact);
        let mut frame = frame.clone();
        *frame.tables[TableId::DivisionStatus.index()] =
            TableView::Complete(TableRows::DivisionStatus(vec![status]));
        let mut report = Report::default();
        compare_division_status(&frame, &changed, &tokens, &Ok(50_000), &Ok(72), &mut report)
            .unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(
            report.differences[0].contains(difference),
            "{:?}",
            report.differences
        );
    }
    for full_path in [vec![9, 9, 8], vec![]] {
        let mut changed = saved.clone();
        let presentation = &mut changed.divisions.get_mut(&id).unwrap().presentation;
        presentation.support_attack = None;
        presentation.full_path = full_path;
        let mut report = Report::default();
        compare_division_movement(&frame, &changed, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
        assert!(report.differences[0].contains("target"));
        assert!(report.differences[1].contains("alternate_path"));
    }
    let mut bad = movement.clone();
    bad.numerator -= 1;
    *frame.tables[TableId::DivisionMovement.index()] =
        TableView::Complete(TableRows::DivisionMovement(vec![bad]));
    let mut report = Report::default();
    compare_division_movement(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    *frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
            entrenchment_cap: 1,
            ..status.clone()
        }]));
    let mut report = Report::default();
    compare_division_status(&frame, &saved, &tokens, &Ok(50_000), &Ok(72), &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    // A changed weight or a stated maximum of 100 differs; an unreadable
    // acclimatization is only that field's limitation.
    for (maximum, weight, differences) in [
        (Some(save::Acclimatization::OMITTED_MAXIMUM_RAW), 1, 1),
        (Some(1), 848_376_000, 1),
        (None, 1, 0),
    ] {
        let mut changed = status.clone();
        changed.acclimatization_maximum = maximum;
        changed.acclimatization[1].weight = weight;
        *frame.tables[TableId::DivisionStatus.index()] =
            TableView::Complete(TableRows::DivisionStatus(vec![changed]));
        let mut report = Report::default();
        compare_division_status(&frame, &saved, &tokens, &Ok(50_000), &Ok(72), &mut report)
            .unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
    }
    // The game writes the block only when an entry weighs above zero
    // whatever the maximum: without one the save's
    // omission agrees, and a written block or a positive weight without
    // one differs.
    for (weights, maximum, block, differences) in [
        ([0, 0], save::Acclimatization::OMITTED_MAXIMUM_RAW, false, 0),
        ([0, -5], 1, false, 0),
        ([0, 1], save::Acclimatization::OMITTED_MAXIMUM_RAW, false, 1),
        ([0, -5], save::Acclimatization::OMITTED_MAXIMUM_RAW, true, 1),
    ] {
        let mut changed = status.clone();
        changed.acclimatization_maximum = Some(maximum);
        for (entry, weight) in changed.acclimatization.iter_mut().zip(weights) {
            entry.weight = weight;
        }
        let mut omitted = saved.clone();
        if !block {
            omitted
                .divisions
                .get_mut(&id)
                .unwrap()
                .presentation
                .acclimatization = None;
        }
        *frame.tables[TableId::DivisionStatus.index()] =
            TableView::Complete(TableRows::DivisionStatus(vec![changed]));
        let mut report = Report::default();
        compare_division_status(&frame, &omitted, &tokens, &Ok(50_000), &Ok(72), &mut report)
            .unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{weights:?} {:?}",
            report.differences
        );
    }
    *frame.tables[TableId::DivisionMovement.index()] =
        TableView::Complete(TableRows::DivisionMovement(vec![]));
    let mut report = Report::default();
    compare_division_movement(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    *frame.tables[TableId::DivisionMovement.index()] =
        TableView::Complete(TableRows::DivisionMovement(vec![
            movement.clone(),
            movement,
        ]));
    assert!(compare_division_movement(&frame, &saved, &mut Report::default()).is_err());
    *frame.tables[TableId::DivisionStatus.index()] = TableView::Truncated {
        rows: TableRows::DivisionStatus(vec![status]),
        missing: 1,
    };
    assert!(
        compare_division_status(
            &frame,
            &saved,
            &tokens,
            &Ok(50_000),
            &Ok(72),
            &mut Report::default()
        )
        .is_err()
    );
}

/// The recorded camouflage is the capture's category-selection decision; the oracle
/// makes it again over the saved block (nonzero weights in order, the
/// maximum unless it is 100) with the content's define.
#[test]
fn division_camouflage_oracle_decides_again_from_the_saved_block() {
    let (mut frame, mut saved, _, _) = division_fixture();
    let id = *saved.divisions.keys().next().unwrap();
    let tokens = BTreeMap::from([
        (7, b"cold_climate".as_slice()),
        (8, b"hot_climate".as_slice()),
    ]);
    let block = |entries: &[(&str, &str)], maximum: Option<&str>| {
        Some(save::Acclimatization {
            entries: entries
                .iter()
                .map(|(name, weight)| {
                    (
                        name.as_bytes().to_vec(),
                        Decimal::parse(weight.as_bytes()).unwrap(),
                    )
                })
                .collect(),
            maximum: maximum.map(|maximum| Decimal::parse(maximum.as_bytes()).unwrap()),
        })
    };
    let check = |frame: &mut replay::frame::DecodedFrame,
                 saved: &Save,
                 camouflage: Recorded<Option<replay::rows::DivisionCamouflage>>,
                 show_at: Result<i64, String>| {
        *frame.tables[TableId::DivisionStatus.index()] =
            TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
                // The day count compares only where a test names it.
                out_of_supply_days: Recorded::NotRecorded,
                division: id,
                camouflage,
                supply: Recorded::NotRecorded,
                ..Default::default()
            }]));
        let mut report = Report::default();
        compare_division_status(frame, saved, &tokens, &show_at, &Ok(72), &mut report).unwrap();
        report
    };
    let shown = |category| {
        Recorded::Value(Some(replay::rows::DivisionCamouflage {
            category: Some(category),
        }))
    };
    let none = Recorded::Value(Some(replay::rows::DivisionCamouflage { category: None }));
    // (saved block, recorded camouflage, differences)
    let cases = [
        // The first of the greatest weights wins at half the maximum of 100.
        (
            block(&[("cold_climate", "60"), ("hot_climate", "60")], None),
            shown(7),
            0,
        ),
        (
            block(&[("cold_climate", "60"), ("hot_climate", "60")], None),
            shown(8),
            1,
        ),
        (
            block(&[("cold_climate", "49.99999"), ("hot_climate", "-3")], None),
            none,
            0,
        ),
        (block(&[("cold_climate", "49.99999")], None), shown(7), 1),
        (block(&[("cold_climate", "50")], None), shown(7), 0),
        // A stated maximum moves the threshold.
        (block(&[("hot_climate", "50")], Some("200")), none, 0),
        (block(&[("hot_climate", "100")], Some("200")), shown(8), 0),
        // An omitted block shows nothing.
        (None, none, 0),
        (None, shown(7), 1),
    ];
    let base = check(&mut frame, &saved, Recorded::Value(None), Ok(50_000)).scalar_checks;
    for (acclimatization, camouflage, differences) in cases {
        saved
            .divisions
            .get_mut(&id)
            .unwrap()
            .presentation
            .acclimatization = acclimatization;
        let report = check(&mut frame, &saved, camouflage, Ok(50_000));
        assert_eq!(
            report.differences.len(),
            differences,
            "{camouflage:?} {:?}",
            report.differences
        );
        assert_eq!(report.scalar_checks, base + 1, "{camouflage:?}");
    }
    // A refused capture, a missing define or an unknown token is only that
    // field's limitation.
    for (camouflage, show_at, reason) in [
        (
            Recorded::Value(None),
            Ok(50_000),
            "camouflage was unreadable at capture",
        ),
        (
            shown(7),
            Err("no defines".to_owned()),
            "ACCLIMATIZATION_CAMO_SHOW_AT is unavailable",
        ),
        (
            shown(9),
            Ok(50_000),
            "absent from the captured lexical dictionary",
        ),
    ] {
        let report = check(&mut frame, &saved, camouflage, show_at);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.scalar_checks, base);
        assert!(
            report
                .limitations
                .iter()
                .any(|(limitation, _)| limitation.contains(reason)),
            "{reason}"
        );
    }
}

#[test]
fn division_supply_grace_compares_at_display_precision() {
    let (mut frame, mut saved, _, _) = division_fixture();
    let id = *saved.divisions.keys().next().unwrap();
    let presentation = &mut saved.divisions.get_mut(&id).unwrap().presentation;
    presentation.max_supply = Some(Decimal::parse(b"72").unwrap());
    presentation.army_current_supply_ratio = Some(Decimal::parse(b"36.71999").unwrap());
    // The remaining grace is stored at the whole percent of its share over
    // the content's SUPPLY_GRACE (72 hours: 0.72), toward zero
    // (DivisionStatusRow::displayed_grace); the maximum stays exact.
    *frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
            // The day count compares only where a test names it.
            out_of_supply_days: Recorded::NotRecorded,
            division: id,
            supply_grace_maximum: Recorded::Value(7_200_000),
            supply_grace_remaining: Recorded::Value(3_600_000),
            supply: Recorded::NotRecorded,
            ..Default::default()
        }]));
    let tokens = BTreeMap::new();
    let check = |saved: &Save, grace: Result<i32, String>| {
        let mut report = Report {
            ..Report::default()
        };
        compare_division_status(&frame, saved, &tokens, &Ok(50_000), &grace, &mut report).unwrap();
        report
    };
    let report = check(&saved, Ok(72));
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        report.display_precision_lines()[1..],
        ["  DivisionStatus.supply_grace_remaining: whole percent of the share over NMilitary.SUPPLY_GRACE (72 hours), toward zero".to_owned()]
    );
    // A save value inside the same percent agrees; the next percent, the
    // previous one and a comparison without the define differ.
    let at = |value: &[u8]| {
        let mut other = saved.clone();
        other
            .divisions
            .get_mut(&id)
            .unwrap()
            .presentation
            .army_current_supply_ratio = Some(Decimal::parse(value).unwrap());
        other
    };
    for (value, grace, differences) in [
        (&b"36.00000"[..], Ok(72), 0),
        (&b"36.71999"[..], Ok(72), 0),
        (&b"36.72000"[..], Ok(72), 1),
        (&b"35.99999"[..], Ok(72), 1),
        (&b"36.71999"[..], Err("no defines".to_owned()), 1),
    ] {
        let report = check(&at(value), grace);
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
    }
}

#[test]
fn division_supply_grace_oracle_requires_unconditional_save_fields_at_display_precision() {
    let (mut frame, mut saved, _, _) = division_fixture();
    let id = *saved.divisions.keys().next().unwrap();
    let presentation = &mut saved.divisions.get_mut(&id).unwrap().presentation;
    presentation.max_supply = Some(Decimal::parse(b"90071992547.40993").unwrap());
    presentation.army_current_supply_ratio = Some(Decimal::parse(b"-12.34567").unwrap());
    let row = DivisionStatusRow {
        // The day count compares only where a test names it.
        out_of_supply_days: Recorded::NotRecorded,
        division: id,
        supply_grace_maximum: Recorded::Value(9_007_199_254_740_993),
        supply_grace_remaining: Recorded::Value(-1_234_000),
        supply: Recorded::NotRecorded,
        ..Default::default()
    };
    *frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(vec![row.clone()]));
    let tokens = BTreeMap::new();
    let mut report = Report::default();
    compare_division_status(&frame, &saved, &tokens, &Ok(50_000), &Ok(72), &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    for field in 0..2 {
        let mut wrong = saved.clone();
        let p = &mut wrong.divisions.get_mut(&id).unwrap().presentation;
        if field == 0 {
            p.max_supply = None;
        } else {
            p.army_current_supply_ratio = Some(Decimal::parse(b"-12.33567").unwrap());
        }
        let mut report = Report::default();
        compare_division_status(&frame, &wrong, &tokens, &Ok(50_000), &Ok(72), &mut report)
            .unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    }
    // Zero is also unconditionally serialized: absence cannot prove zero.
    *frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
            // The day count compares only where a test names it.
            out_of_supply_days: Recorded::NotRecorded,
            supply_grace_maximum: Recorded::Value(0),
            supply_grace_remaining: Recorded::Value(0),
            ..row.clone()
        }]));
    let p = &mut saved.divisions.get_mut(&id).unwrap().presentation;
    p.max_supply = None;
    p.army_current_supply_ratio = None;
    let mut report = Report::default();
    compare_division_status(&frame, &saved, &tokens, &Ok(50_000), &Ok(72), &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
}

#[test]
fn division_status_naming_oracle_checks_generator_operands_and_absence() {
    let (mut frame, mut saved, _, _) = division_fixture();
    let id = *saved.divisions.keys().next().unwrap();
    let template = saved.divisions[&id].template.unwrap();
    saved.division_templates.insert(
        template,
        save::DivisionTemplate {
            template,
            country: b"GER".to_vec(),
            name: Vec::new(),
            name_present: false,
            name_key: Vec::new(),
            name_key_present: false,
            custom_icon: 0,
            custom_icon_nato: false,
            foreign_template_tag: None,
            composition: BTreeMap::new(),
            names_group: Some(b"GROUP".to_vec()),
        },
    );
    saved
        .divisions
        .get_mut(&id)
        .unwrap()
        .presentation
        .division_name = Some(save::DivisionName::default());
    let naming = replay::rows::NameOperands {
        ordered: 255,
        group: b"GROUP".to_vec(),
        ..Default::default()
    };
    let set = |frame: &mut DecodedFrame, naming| {
        *frame.tables[TableId::DivisionStatus.index()] =
            TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
                // The day count compares only where a test names it.
                out_of_supply_days: Recorded::NotRecorded,
                division: id,
                naming,
                supply: Recorded::NotRecorded,
                ..Default::default()
            }]));
    };
    let tokens = BTreeMap::new();
    set(&mut frame, Recorded::Value(Some(naming.clone())));
    let mut report = Report::default();
    compare_division_status(&frame, &saved, &tokens, &Ok(50_000), &Ok(72), &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    for changed in [
        replay::rows::NameOperands {
            kind: 1,
            ..naming.clone()
        },
        replay::rows::NameOperands {
            order: -1,
            ..naming.clone()
        },
        replay::rows::NameOperands {
            ordered: 0,
            ..naming.clone()
        },
        replay::rows::NameOperands {
            programmatic: 1,
            ..naming.clone()
        },
        replay::rows::NameOperands {
            custom: b"Renamed".to_vec(),
            ..naming.clone()
        },
        replay::rows::NameOperands {
            group: b"OTHER".to_vec(),
            ..naming.clone()
        },
    ] {
        set(&mut frame, Recorded::Value(Some(changed)));
        let mut report = Report::default();
        compare_division_status(&frame, &saved, &tokens, &Ok(50_000), &Ok(72), &mut report)
            .unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    }
    set(&mut frame, Recorded::Value(Some(naming)));
    saved
        .divisions
        .get_mut(&id)
        .unwrap()
        .presentation
        .division_name = None;
    let mut report = Report::default();
    compare_division_status(&frame, &saved, &tokens, &Ok(50_000), &Ok(72), &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    set(&mut frame, Recorded::Value(None));
    let mut report = Report::default();
    compare_division_status(&frame, &saved, &tokens, &Ok(50_000), &Ok(72), &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .contains_key("division naming operands unreadable at capture")
    );
    assert!(
        !report
            .limitations
            .contains_key("DivisionStatus.naming not recorded")
    );
    set(&mut frame, Recorded::NotRecorded);
    let mut report = Report::default();
    compare_division_status(&frame, &saved, &tokens, &Ok(50_000), &Ok(72), &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .contains_key("DivisionStatus.naming not recorded")
    );
    assert!(
        !report
            .limitations
            .contains_key("division naming operands unreadable at capture")
    );
}

#[test]
fn division_template_oracle_checks_unused_rosters_composition_and_forced_refresh() {
    let (mut frame, mut saved, _, _) = division_fixture();
    saved.divisions.clear();
    let template = u64::MAX - 203;
    saved.division_templates.insert(
        template,
        save::DivisionTemplate {
            template,
            country: b"GER".to_vec(),
            name: b"A".to_vec(), // The text writer omits the captured NUL suffix.
            name_present: true,
            name_key: b"FALLBACK".to_vec(),
            name_key_present: true,
            custom_icon: -1,
            custom_icon_nato: false,
            foreign_template_tag: Some(b"---".to_vec()),
            composition: BTreeMap::from([((0, 0, 1), b"infantry".to_vec())]),
            names_group: None,
        },
    );
    saved
        .division_template_owners
        .insert(template, b"GER".to_vec());
    let row = replay::rows::DivisionTemplateRow {
        template,
        owner: 102,
        name: vec![b'A', 0, 255],
        name_key: replay::rows::Recorded::Value(b"FALLBACK".to_vec()),
        ..Default::default()
    };
    *frame.tables[TableId::DivisionTemplates.index()] =
        TableView::Complete(TableRows::DivisionTemplates(vec![row]));
    frame.hour = 4; // Off slot3, but a forced refresh has measured work.
    frame.metrics.table_ns[TableId::DivisionTemplates.index()] = 1;
    let countries = country_context(&frame).unwrap();
    let mut report = Report::default();
    report.record_tables(&frame);
    compare_division_templates(&frame, &saved, &countries, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    saved
        .division_templates
        .get_mut(&template)
        .unwrap()
        .name_key
        .clear();
    let mut report = Report::default();
    report.record_tables(&frame);
    compare_division_templates(&frame, &saved, &countries, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("name_key"));
    saved
        .division_templates
        .get_mut(&template)
        .unwrap()
        .name_key = b"FALLBACK".to_vec();
    *frame.tables[TableId::DivisionTemplates.index()] =
        TableView::Complete(TableRows::DivisionTemplates(vec![]));
    let countries = country_context(&frame).unwrap();
    let mut report = Report::default();
    report.record_tables(&frame);
    compare_division_templates(&frame, &saved, &countries, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1); // Missing unused template cannot pass.

    // The hourly inputs compare the saved icon choice, the template's
    // country operands and its composition.
    let tokens = BTreeMap::from([(7, b"infantry".as_slice())]);
    let icons = replay::rows::DivisionTemplateIconRow {
        template,
        priority_unit: 7,
        priority_unit_name: b"infantry".to_vec(),
        custom_icon: -1,
        custom_icon_nato: 0,
        country: 102,
        foreign_country: -1,
        subunits: vec![replay::rows::TemplateSubunit {
            token: 7,
            local: 1,
            count: 1,
        }],
        ..Default::default()
    };
    let hourly = |frame: &mut DecodedFrame, rows| {
        *frame.tables[TableId::DivisionTemplateIcons.index()] =
            TableView::Complete(TableRows::DivisionTemplateIcons(rows));
    };
    let compare = |frame: &DecodedFrame| {
        let countries = country_context(frame).unwrap();
        let mut report = Report::default();
        compare_division_template_icons(frame, &saved, &countries, &tokens, &mut report).unwrap();
        report
    };
    hourly(&mut frame, vec![icons.clone()]);
    let report = compare(&frame);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|subject| subject.contains("foreign_template_tag"))
    );
    // A null template country is a stated limitation, never an assumed tag.
    hourly(
        &mut frame,
        vec![replay::rows::DivisionTemplateIconRow {
            country: -1,
            ..icons.clone()
        }],
    );
    let report = compare(&frame);
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|subject| subject.ends_with(".country"))
    );
    hourly(
        &mut frame,
        vec![replay::rows::DivisionTemplateIconRow {
            custom_icon: 3,
            custom_icon_nato: 1,
            subunits: Vec::new(),
            ..icons.clone()
        }],
    );
    assert_eq!(compare(&frame).differences.len(), 3);
    // Unreadable model inputs are that row's limitation, not a difference.
    hourly(
        &mut frame,
        vec![replay::rows::DivisionTemplateIconRow {
            subunits: Vec::new(),
            model_unreadable: 1,
            ..icons.clone()
        }],
    );
    let report = compare(&frame);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    // Its country operands still compare: a wrong country is reported.
    hourly(
        &mut frame,
        vec![replay::rows::DivisionTemplateIconRow {
            country: 301,
            subunits: Vec::new(),
            model_unreadable: 1,
            ..icons.clone()
        }],
    );
    let report = compare(&frame);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(
        report.differences[0].contains(".country"),
        "{:?}",
        report.differences
    );
    hourly(&mut frame, vec![]);
    assert_eq!(compare(&frame).differences.len(), 1);
}

fn relations_fixture() -> (
    DecodedFrame,
    Save,
    save::PartialMapMembership,
    installed_content::equipment::Database,
) {
    let (mut frame, mut save, membership, equipment) = fixture();
    add_country(&mut frame, &mut save, 201, 8, b"ENG");
    let start = save::GameDate::parse(b"1935.6.1.1").unwrap();
    save.war_relations.push(save::WarRelation {
        first_goals: Vec::new(),
        second_goals: Vec::new(),
        roster_owner: b"GER".to_vec(),
        target: b"ENG".to_vec(),
        first: b"GER".to_vec(),
        second: b"ENG".to_vec(),
        start_date: Some(start),
        end_date: None,
        cancel: false,
        first_casualties: 9_007_199_254_740_993,
        second_casualties: i64::MAX,
        first_unknown_casualties: i64::MIN,
        second_unknown_casualties: -17,
        first_was_instigator: false,
    });
    *frame.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
            first_country: 102,
            second_country: 201,
            start_hour: start.engine_hour().unwrap(),
            end_hour: 43_808_760,
            cancel: 0,
            first_casualties: 9_007_199_254_740_993,
            second_casualties: i64::MAX,
            first_unknown_casualties: i64::MIN,
            second_unknown_casualties: -17,
            first_was_instigator: 0,
        }]));
    // Both partial-zero handles are valid; do not collapse them to null or a shared type.
    save.factions.insert(
        88,
        save::Faction {
            id: 88,
            name: None,
            members: vec![b"ENG".to_vec(), b"GER".to_vec()],
            icon: None,
            // A template faction, coloured from its leader's colour.
            template: Some(b"faction_template_axis".to_vec()),
            color: Some(vec![127, 63, 255]),
            color_overridden: Some(false),
            theatres: Vec::new(),
            advisor_slots: Vec::new(),
        },
    );
    save.factions.insert(
        1_u64 << 63,
        save::Faction {
            id: 1_u64 << 63,
            name: Some(b"Literal".to_vec()),
            members: Vec::new(),
            icon: None,
            // No template; an overridden colour with its alpha written, since it is not 1.0.
            template: None,
            color: Some(vec![255, 0, 63, 127]),
            color_overridden: Some(true),
            theatres: Vec::new(),
            advisor_slots: Vec::new(),
        },
    );
    *frame.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(vec![
        FactionRow {
            id: 88,
            name: Vec::new(),
            template_token: Some(TEMPLATE_TOKEN),
            color_red: 0.5_f32.to_bits(),
            color_green: 0.25_f32.to_bits(),
            color_blue: 1.0_f32.to_bits(),
            color_alpha: 1.0_f32.to_bits(),
            color_overridden: 0,
        },
        FactionRow {
            id: 1_u64 << 63,
            name: b"Literal".to_vec(),
            template_token: None,
            color_red: 1.0_f32.to_bits(),
            color_green: 0.0_f32.to_bits(),
            color_blue: 0.25_f32.to_bits(),
            color_alpha: 0.5_f32.to_bits(),
            color_overridden: 1,
        },
    ]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            replay::rows::LexicalTokenRow {
                token: TEMPLATE_TOKEN,
                literal: b"faction_template_axis".to_vec(),
            },
        ]));
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 102,
                faction: 88,
                ordinal: 1,
            },
            FactionMemberRow {
                country: 201,
                faction: 88,
                ordinal: 0,
            },
        ]));
    (frame, save, membership, equipment)
}

/// The fixture's faction template token (any id the dictionary names).
const TEMPLATE_TOKEN: u32 = 0x4c1a;

fn war_row_mut(frame: &mut DecodedFrame) -> &mut WarRelationRow {
    let TableView::Complete(TableRows::WarRelations(rows)) =
        &mut *frame.tables[TableId::WarRelations.index()]
    else {
        unreachable!()
    };
    &mut rows[0]
}

#[test]
fn table_counts_collect_all_issues_and_unproved_sources_remain_incomplete() {
    let (mut frame, _, _, _) = fixture();
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "global lexer writer is not proved".into(),
    };
    *frame.tables[TableId::ScriptVariables.index()] = TableView::Unavailable {
        reason: "script scope roots are not proved".into(),
    };
    let counts = recorded_table_counts(&frame);
    assert_eq!(counts.len(), TableId::ALL.len());
    assert!(counts[&TableId::LexicalTokens].is_err());
    assert!(counts[&TableId::ScriptVariables].is_err());
    assert_eq!(counts[&TableId::Countries], Ok(1));
    let mut report = Report::default();
    report.record_tables(&frame);
    assert!(lexical_tokens(&frame, &mut report).unwrap().is_empty());
    assert!(report.limitations.values().any(|subjects| {
        subjects
            .iter()
            .any(|subject| subject == "global lexer writer is not proved")
    }));

    *frame.tables[TableId::Stockpiles.index()] = TableView::Unavailable {
        reason: "failed native walk".into(),
    };
    let counts = recorded_table_counts(&frame);
    let error = counts[&TableId::Stockpiles].as_ref().unwrap_err();
    assert!(error.contains("Stockpiles") && error.contains("failed native walk"));
    // Retired tables are named absences, never availability issues.
    assert_eq!(
        counts
            .iter()
            .filter(|(table, count)| count.is_err() && !report::retired_table(**table))
            .count(),
        3
    );
    assert!(report.differences.is_empty());
    assert!(
        report
            .outcome()
            .unwrap_err()
            .to_string()
            .contains("incomplete")
    );
    *frame.tables[TableId::Stockpiles.index()] =
        TableView::Complete(TableRows::Stockpiles(Vec::new()));
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Truncated {
        rows: TableRows::LexicalTokens(Vec::new()),
        missing: 7,
    };
    let counts = recorded_table_counts(&frame);
    assert!(
        counts[&TableId::LexicalTokens]
            .as_ref()
            .unwrap_err()
            .contains("7 missing")
    );
    assert!(lexical_tokens(&frame, &mut report).is_err());
}

#[test]
fn lexical_dictionary_preserves_exact_bytes_and_rejects_duplicate_or_wrong_tables() {
    use replay::rows::LexicalTokenRow;

    let (mut frame, _, _, _) = fixture();
    let entry = LexicalTokenRow {
        token: u32::MAX,
        literal: b"name\0suffix".to_vec(),
    };
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 0,
                literal: Vec::new(),
            },
            entry.clone(),
        ]));
    let mut report = Report::default();
    let tokens = lexical_tokens(&frame, &mut report).unwrap();
    assert_eq!(tokens[&0], b"");
    assert_eq!(tokens[&u32::MAX], b"name\0suffix");
    assert!(report.limitations.is_empty());
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![entry.clone(), entry]));
    assert!(lexical_tokens(&frame, &mut report).is_err());
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::Stockpiles(Vec::new()));
    assert!(lexical_tokens(&frame, &mut report).is_err());
    assert!(recorded_table_counts(&frame)[&TableId::LexicalTokens].is_err());
}

#[test]
fn nested_comparison_counts_assign_each_difference_once_and_retain_failure() {
    let mut report = Report::default();
    assert!(report.attempt("outer", |report| {
        report.difference("before", 1, 2);
        assert!(!report.attempt("inner", |report| {
            report.difference("inside", 3, 4);
            Err(invalid("independent row failure"))
        }));
        report.difference("after", 5, 6);
        Ok(())
    }));
    report.difference("direct", 7, 8);
    assert_eq!(report.difference_groups["outer"], 2);
    assert_eq!(report.difference_groups["inner"], 1);
    assert_eq!(report.difference_groups["direct comparisons"], 1);
    assert_eq!(
        report.difference_groups.values().sum::<usize>(),
        report.differences.len()
    );
    assert_eq!(
        report.failed_comparisons["inner"],
        "independent row failure"
    );
    assert!(report.active_comparison.is_none());
    assert!(report.outcome().is_err());
}

#[test]
fn garrison_equipment_dispatch_skips_carried_rows_and_compares_actual_refreshes() {
    use replay::rows::{
        GarrisonEquipment, GarrisonEquipmentInputs, LexicalTokenRow, StateGarrisonRow,
        VariantDefinitionRow,
    };
    let (mut frame, mut saved, membership, equipment) = fixture();
    let parsed = installed_content::parser::parse(
        br#"
        countries={GER={occupation_status={occupation={GER={state_garrison_data={
            1={equipment={equipment={id={id=9 type=70} amount=0.6} allow_zero_entries=no}
               equipment_need={mod_weapon=2}}
        }}}}}} states={} provinces={}
        "#,
    )
    .unwrap();
    saved.construction = save::construction::parse(&parsed.items).unwrap();
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 700,
            literal: b"mod_weapon".to_vec(),
        }]));
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: (9_u64 << 32) | 70,
            archetype_token: 700,
            ..Default::default()
        }]));
    *frame.tables[TableId::StateGarrisons.index()] =
        TableView::Complete(TableRows::StateGarrisons(vec![StateGarrisonRow {
            controller: 101,
            occupied: 101,
            state: 1,
            equipment: Recorded::Value(GarrisonEquipmentInputs {
                rows: vec![GarrisonEquipment {
                    equipment_token: 700,
                    archetype_token: 700,
                    archetype_active: 1,
                    required_raw: 100_000,
                    required_lookup_raw: 100_000,
                    allocated_raw: 50_000,
                    weight_raw: 100_000,
                }],
                failure: None,
            }),
            ..Default::default()
        }]));
    let read_hour = 96
        + match replay::cadence::cadence(TableId::StateGarrisons) {
            replay::cadence::Cadence::Daily(slot) => slot as Hour,
            other => panic!("StateGarrisons is {other:?}"),
        };
    let group = "construction equipment StateGarrisons";
    // The same mismatched requirements are compared on their slot or a forced read,
    // but retained daily rows are not compared against a later save.
    for (hour, forced, carried) in [
        (read_hour, false, false),
        (read_hour + 1, false, true),
        (read_hour + 1, true, false),
    ] {
        frame.hour = hour;
        frame.metrics.table_ns[TableId::StateGarrisons.index()] = u64::from(forced);
        let mut report = Report {
            read_hours: Some(BTreeMap::from([(
                TableId::StateGarrisons,
                if forced { hour } else { read_hour },
            )])),
            ..Report::default()
        };
        report.record_tables(&frame);
        compare_extended(
            &frame,
            &saved,
            ExtendedContent {
                membership: Ok(&membership),
                base_resources: Err("outside the garrison fixture".into()),
                railway_adjacency: Err("outside the garrison fixture".into()),
                buildings: Err("outside the garrison fixture".into()),
                defines: Err("outside the garrison fixture".into()),
                raid_graph: Err("outside the garrison fixture".into()),
                equipment: Ok(&equipment),
                leader_traits: Err("outside the garrison fixture".into()),
                intelligence: Err("outside the garrison fixture".into()),
                temperature: Err("outside the garrison fixture".into()),
                sub_units: Err("outside the garrison fixture".into()),
                essential_caps: Err("outside the garrison fixture".into()),
            },
            &mut report,
        );
        assert!(!report.failed_comparisons.contains_key(group));
        assert_eq!(
            report.carried.get(group).copied(),
            carried.then_some(Some(read_hour))
        );
        assert_eq!(
            report.difference_groups.get(group).copied().unwrap_or(0),
            if carried { 0 } else { 1 }
        );
        assert_eq!(
            report
                .compared_tables
                .get("StateGarrisons equipment")
                .copied(),
            (!carried).then_some(1)
        );
        if !carried {
            assert!(
                report
                    .difference_owners
                    .iter()
                    .zip(&report.differences)
                    .any(
                        |(owner, difference)| owner == group && difference.contains("requirements")
                    )
            );
        }
        // The save writes variants, not the allocation cache.
        assert_eq!(
            report.limitations.iter().any(|(reason, subjects)| {
                reason.contains("allocation cache")
                    && subjects
                        .iter()
                        .any(|subject| subject == "garrison.GER.1.equipment.allocations")
            }),
            !carried
        );
    }
}

#[test]
fn failed_content_reads_preserve_independent_map_and_pack_diagnostics() {
    let (frame, saved, membership, equipment) = fixture();
    let mut report = compare(&frame, &saved, &membership, &equipment).unwrap();
    assert!(report.outcome().is_ok());
    compare_extended(
        &frame,
        &saved,
        ExtendedContent {
            membership: Ok(&membership),
            base_resources: Err("history/states/101-Greenland.txt: malformed resource value".into()),
            railway_adjacency: Err("map/provinces.bmp: malformed adjacency input".into()),
            buildings: Ok(&installed_content::buildings::Buildings::default()),
            defines: Ok(&installed_content::defines::Defines::new().unwrap()),
            raid_graph: Err("map/adjacencies.csv: unused".into()),
            equipment: Err("common/units/equipment: unused".into()),
            leader_traits: Err("common/country_leader: unused".into()),
            sub_units: Err("common/units: unused".into()),
            essential_caps: Err("common/units: unused".into()),
            intelligence: Err("common/intelligence_agency_upgrades: unused".into()),
            temperature: Err("unused".into()),
        },
        &mut report,
    );
    assert_eq!(report.countries, 1);
    assert_eq!(report.provinces, 2);
    assert!(report.failed_comparisons["content base resources"].contains("101-Greenland.txt"));
    assert!(
        report.failed_comparisons["construction StateBaseResources"].contains("101-Greenland.txt")
    );
    assert!(report.compared_tables.contains_key("WeatherRegions"));
    assert!(report.failed_comparisons["content railway adjacency"].contains("map/provinces.bmp"));
    assert!(report.failed_comparisons["RailEdges"].contains("map/provinces.bmp"));
    assert!(report.compared_tables.contains_key("SunkConvoys"));
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("weather.provinces"))
    );
    assert!(report.outcome().is_err());

    let (mut frame, mut saved, _, _) = fixture();
    add_country(&mut frame, &mut saved, 202, 9, b"ENG");
    let report = compare_core(
        &frame,
        SampleBoundary::EndOfHour,
        &saved,
        Err("state membership is malformed".into()),
        Err("equipment definition is malformed".into()),
        None,
    )
    .unwrap();
    assert_eq!(report.countries, 2);
    assert_eq!(report.scalar_checks, 10);
    assert_eq!(report.provinces, 0);
    assert!(report.failed_comparisons["provinces"].contains("state membership is malformed"));
    assert!(
        report.failed_comparisons["country GER convoys"]
            .contains("equipment definition is malformed")
    );
    assert!(
        report.failed_comparisons["country ENG convoys"]
            .contains("equipment definition is malformed")
    );
    assert!(report.outcome().is_err());
}

#[test]
fn unavailable_sources_and_failed_groups_do_not_hide_later_independent_mismatches() {
    let (mut frame, mut saved, membership, equipment) = fixture();
    for (table, reason) in [
        (TableId::Divisions, "division read failed"),
        (TableId::ScriptVariables, "scope roots are not proved"),
        (TableId::LexicalTokens, "global lexer writer is not proved"),
    ] {
        *frame.tables[table.index()] = TableView::Unavailable {
            reason: reason.into(),
        };
    }
    *frame.tables[TableId::Players.index()] =
        TableView::Complete(TableRows::Players(vec![replay::rows::PlayerRow {
            id: 1,
            country: 101,
            user: b"later-player".to_vec(),
            ..Default::default()
        }]));
    let weather =
        installed_content::parser::parse(b"weather={provinces={0={province=0 snow=0.00001}} regions={}}")
            .unwrap();
    saved.weather = save::weather::parse(&weather.items).unwrap();
    *frame.tables[TableId::WeatherProvinces.index()] =
        TableView::Complete(TableRows::WeatherProvinces(vec![
            replay::rows::WeatherProvinceRow {
                id: 0,
                snow: 2,
                ..Default::default()
            },
        ]));
    let mut report = compare(&frame, &saved, &membership, &equipment).unwrap();
    assert_eq!(report.countries, 1);
    assert_eq!(report.provinces, 2);
    assert!(report.failed_comparisons["divisions"].contains("division read failed"));
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("later-player"))
    );
    compare_extended(
        &frame,
        &saved,
        ExtendedContent {
            membership: Ok(&membership),
            base_resources: Ok(&save::construction::PartialBaseResources::default()),
            railway_adjacency: Err("railway content is outside this fixture".into()),
            buildings: Ok(&installed_content::buildings::Buildings::default()),
            defines: Ok(&installed_content::defines::Defines::new().unwrap()),
            raid_graph: Err("map/adjacencies.csv: unused".into()),
            equipment: Err("common/units/equipment: unused".into()),
            leader_traits: Err("common/country_leader: unused".into()),
            sub_units: Err("common/units: unused".into()),
            essential_caps: Err("common/units: unused".into()),
            intelligence: Err("common/intelligence_agency_upgrades: unused".into()),
            temperature: Err("unused".into()),
        },
        &mut report,
    );
    assert!(report.failed_comparisons.contains_key("intel"));
    assert!(report.failed_comparisons["ScriptVariables"].contains("scope roots are not proved"));
    assert_eq!(report.compared_tables["WeatherProvinces"], 1);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("weather province 0.snow"))
    );
    assert_eq!(report.unavailable_tables(), 3);
    assert!(
        report
            .outcome()
            .unwrap_err()
            .to_string()
            .contains("incomplete")
    );

    *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "country links failed".into(),
    };
    let mut report = compare(&frame, &saved, &membership, &equipment).unwrap();
    compare_extended(
        &frame,
        &saved,
        ExtendedContent {
            membership: Ok(&membership),
            base_resources: Ok(&save::construction::PartialBaseResources::default()),
            railway_adjacency: Err("railway content is outside this fixture".into()),
            buildings: Ok(&installed_content::buildings::Buildings::default()),
            defines: Ok(&installed_content::defines::Defines::new().unwrap()),
            raid_graph: Err("map/adjacencies.csv: unused".into()),
            equipment: Err("common/units/equipment: unused".into()),
            leader_traits: Err("common/country_leader: unused".into()),
            sub_units: Err("common/units: unused".into()),
            essential_caps: Err("common/units: unused".into()),
            intelligence: Err("common/intelligence_agency_upgrades: unused".into()),
            temperature: Err("unused".into()),
        },
        &mut report,
    );
    assert!(report.failed_comparisons["map country context"].contains("country links failed"));
    assert!(report.failed_comparisons["air"].contains("requires country links"));
    assert_eq!(report.compared_tables["WeatherProvinces"], 1);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("weather province 0.snow"))
    );
    assert!(report.outcome().is_err());
}

#[test]
fn relations_compare_full_integer_precision_aliases_and_ordered_faction_members() {
    let (mut frame, save, membership, equipment) = relations_fixture();
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        (
            report.war_relations,
            report.factions,
            report.faction_members
        ),
        (1, 2, 2)
    );
    let row = war_row_mut(&mut frame);
    row.first_casualties -= 1;
    row.second_casualties -= 1;
    row.first_unknown_casualties += 1;
    row.second_unknown_casualties += 1;
    row.first_was_instigator = 1;
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 5, "{:?}", report.differences);
}

#[test]
fn omitted_relation_dates_check_the_proven_writer_conditions() {
    let (mut frame, mut save, membership, equipment) = relations_fixture();
    // Omission permits any end <= start, not only the constructor sentinel.
    let row = war_row_mut(&mut frame);
    row.end_hour = row.start_hour - 1;
    assert!(
        compare(&frame, &save, &membership, &equipment)
            .unwrap()
            .differences
            .is_empty()
    );
    let row = war_row_mut(&mut frame);
    row.end_hour = row.start_hour + 1;
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("end_date presence"));
    save.war_relations[0].end_date = Some(save.date);
    war_row_mut(&mut frame).end_hour = save.date.engine_hour().unwrap();
    assert!(
        compare(&frame, &save, &membership, &equipment)
            .unwrap()
            .differences
            .is_empty()
    );
    save.war_relations[0].cancel = true;
    save.war_relations[0].start_date = None;
    war_row_mut(&mut frame).cancel = 1;
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("raw start hour"))
    );
    war_row_mut(&mut frame).cancel = 0;
    assert_eq!(
        compare(&frame, &save, &membership, &equipment)
            .unwrap()
            .differences
            .len(),
        1
    );
}

#[test]
fn war_pair_direction_missing_rows_and_saved_envelopes_are_not_guessed() {
    let (mut frame, mut save, membership, equipment) = relations_fixture();
    let row = war_row_mut(&mut frame);
    std::mem::swap(&mut row.first_country, &mut row.second_country);
    assert_eq!(
        compare(&frame, &save, &membership, &equipment)
            .unwrap()
            .differences
            .len(),
        2
    );
    save.war_relations[0].roster_owner = b"ENG".to_vec();
    assert!(comparison_error(compare(&frame, &save, &membership, &equipment)).contains("envelope"));
    save.war_relations[0].roster_owner = b"UNKNOWN".to_vec();
    assert!(
        comparison_error(compare(&frame, &save, &membership, &equipment))
            .contains("unknown saved country")
    );
}

#[test]
fn war_flags_and_duplicate_alias_pairs_that_saves_cannot_verify_are_refused() {
    let (frame, save, membership, equipment) = relations_fixture();
    for (cancel, instigator) in [(2, 0), (0, 255)] {
        let mut changed = frame.clone();
        let row = war_row_mut(&mut changed);
        row.cancel = cancel;
        row.first_was_instigator = instigator;
        assert!(
            comparison_error(compare(&changed, &save, &membership, &equipment))
                .contains("cannot verify these bits")
        );
    }
    let mut changed = frame.clone();
    let TableView::Complete(TableRows::WarRelations(rows)) =
        &mut *changed.tables[TableId::WarRelations.index()]
    else {
        unreachable!()
    };
    let mut duplicate = rows[0].clone();
    duplicate.first_country = 101;
    rows.push(duplicate);
    assert!(
        comparison_error(compare(&changed, &save, &membership, &equipment))
            .contains("duplicate resolved")
    );
    let mut duplicate_save = save.clone();
    duplicate_save
        .war_relations
        .push(save.war_relations[0].clone());
    assert!(
        compare(&frame, &duplicate_save, &membership, &equipment)
            .and_then(|report| report.outcome())
            .is_err()
    );
}

#[test]
fn faction_names_handles_and_membership_order_are_exact() {
    let (mut frame, save, membership, equipment) = relations_fixture();
    let TableView::Complete(TableRows::Factions(rows)) =
        &mut *frame.tables[TableId::Factions.index()]
    else {
        unreachable!()
    };
    let row = &mut rows[0];
    row.name = b"Axis".to_vec();
    let TableView::Complete(TableRows::FactionMembers(rows)) =
        &mut *frame.tables[TableId::FactionMembers.index()]
    else {
        unreachable!()
    };
    for row in rows {
        row.ordinal = 1 - row.ordinal;
    }
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    let (mut frame, save, membership, equipment) = relations_fixture();
    let TableView::Complete(TableRows::Factions(rows)) =
        &mut *frame.tables[TableId::Factions.index()]
    else {
        unreachable!()
    };
    let row = &mut rows[1];
    row.id ^= 1;
    assert_eq!(
        compare(&frame, &save, &membership, &equipment)
            .unwrap()
            .differences
            .len(),
        2
    );
    let (mut frame, save, membership, equipment) = relations_fixture();
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(Vec::new()));
    assert_eq!(
        compare(&frame, &save, &membership, &equipment)
            .unwrap()
            .differences
            .len(),
        2
    );
}

/// The template is named by the lexer dictionary, the colour compared through
/// colour's own write (a truncating cast of the f32 product) and the override byte.
#[test]
fn faction_templates_colours_and_overrides_compare_as_the_save_writes_them() {
    let (frame, save, membership, equipment) = relations_fixture();
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let mut changed = frame.clone();
    let TableView::Complete(TableRows::Factions(rows)) =
        &mut *changed.tables[TableId::Factions.index()]
    else {
        unreachable!()
    };
    // 0.3 * 255 is 76.5 in f32: written as 76, never rounded to 77.
    rows[0].color_red = 0.3_f32.to_bits();
    rows[0].template_token = Some(TEMPLATE_TOKEN + 1);
    rows[1].color_overridden = 0;
    let differences = compare(&changed, &save, &membership, &equipment)
        .unwrap()
        .differences;
    assert_eq!(differences.len(), 3, "{differences:?}");
    assert!(
        differences
            .iter()
            .any(|line| line.contains(".color: replay [76, 63, 255]")),
        "{differences:?}"
    );
    assert!(
        differences
            .iter()
            .any(|line| line.contains("not in LexicalTokens")),
        "{differences:?}"
    );
    assert!(
        differences
            .iter()
            .any(|line| line.contains(".is_color_overridden")),
        "{differences:?}"
    );
    // Without the dictionary the spelling of two present templates is a limitation,
    // not a difference.
    let spelling =
        "faction template tokens cannot be spelled without a complete LexicalTokens table";
    let unavailable = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let truncated = TableView::Truncated {
        rows: TableRows::LexicalTokens(Vec::new()),
        missing: 1,
    };
    for dictionary in [unavailable, truncated] {
        let mut unnamed = frame.clone();
        *unnamed.tables[TableId::LexicalTokens.index()] = dictionary;
        let report = compare(&unnamed, &save, &membership, &equipment).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(report.limitations.contains_key(spelling));
        // Presence needs no dictionary: the save writes `template` for every
        // non-null template, so a template the save lacks is a difference however
        // the dictionary was recorded.
        let TableView::Complete(TableRows::Factions(rows)) =
            &mut *unnamed.tables[TableId::Factions.index()]
        else {
            unreachable!()
        };
        rows[1].template_token = Some(42);
        let report = compare(&unnamed, &save, &membership, &equipment).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(
            report.differences[0].contains(".template: replay \"token 42\""),
            "{:?}",
            report.differences
        );
    }
    // A saved template the replay lacks, and a raw override byte above one, are named.
    let mut dropped = frame.clone();
    let TableView::Complete(TableRows::Factions(rows)) =
        &mut *dropped.tables[TableId::Factions.index()]
    else {
        unreachable!()
    };
    rows[0].template_token = None;
    rows[1].color_overridden = 2;
    let report = compare(&dropped, &save, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("is_color_overridden"))
    );
}

#[test]
fn a_colour_is_written_as_the_native_float_conversion_does() {
    let bits = |values: [f32; 4]| values.map(f32::to_bits);
    assert_eq!(
        relations::saved_colour(bits([0.5, 0.25, 1.0, 1.0])),
        [127, 63, 255]
    );
    assert_eq!(
        relations::saved_colour(bits([0.0, 1.0, 0.999, 0.5])),
        [0, 255, 254, 127]
    );
    // cvttss2si gives the integer indefinite for NaN and out-of-range products.
    assert_eq!(
        relations::saved_colour(bits([f32::NAN, 1e30, -1e30, 1.0])),
        [i64::from(i32::MIN); 3]
    );
    assert_eq!(
        relations::saved_colour(bits([-0.5, 0.0, 0.0, 1.0])),
        [-127, 0, 0]
    );
}

#[test]
fn faction_members_cannot_duplicate_aliases_skip_ordinals_or_reference_absent_factions() {
    let (frame, save, membership, equipment) = relations_fixture();
    for member in [
        FactionMemberRow {
            country: 101,
            faction: 88,
            ordinal: 2,
        },
        FactionMemberRow {
            country: 301,
            faction: 88,
            ordinal: 2,
        },
        FactionMemberRow {
            country: 102,
            faction: 777,
            ordinal: 0,
        },
    ] {
        let mut changed = frame.clone();
        let TableView::Complete(TableRows::FactionMembers(rows)) =
            &mut *changed.tables[TableId::FactionMembers.index()]
        else {
            unreachable!()
        };
        rows.push(member);
        assert!(
            compare(&changed, &save, &membership, &equipment)
                .and_then(|report| report.outcome())
                .is_err()
        );
    }
    let mut changed = frame.clone();
    let TableView::Complete(TableRows::FactionMembers(rows)) =
        &mut *changed.tables[TableId::FactionMembers.index()]
    else {
        unreachable!()
    };
    let row = &mut rows[0];
    row.ordinal = 3;
    assert!(
        comparison_error(compare(&changed, &save, &membership, &equipment)).contains("contiguous")
    );
    let mut duplicate_save = save.clone();
    duplicate_save
        .factions
        .get_mut(&(1_u64 << 63))
        .unwrap()
        .members
        .push(b"GER".to_vec());
    assert!(
        comparison_error(compare(&frame, &duplicate_save, &membership, &equipment))
            .contains("duplicate resolved country")
    );
}

#[test]
fn incomplete_or_wrong_relation_and_faction_tables_never_pass() {
    let (frame, save, membership, equipment) = relations_fixture();
    for table in [
        TableId::WarRelations,
        TableId::Factions,
        TableId::FactionMembers,
    ] {
        for view in [
            TableView::Unavailable {
                reason: "unreadable".into(),
            },
            TableView::Truncated {
                rows: TableRows::empty(table),
                missing: 1,
            },
            TableView::Complete(TableRows::Provinces(vec![ProvinceRow::default()])),
        ] {
            let mut changed = frame.clone();
            *changed.tables[table.index()] = view;
            assert!(
                compare(&changed, &save, &membership, &equipment)
                    .and_then(|report| report.outcome())
                    .is_err()
            );
        }
    }
}

#[test]
fn game_score_extended_oracle_compares_every_country_and_records_empty_wings() {
    let (mut frame, mut saved, membership, _) = fixture();
    add_country(&mut frame, &mut saved, 201, 8, b"BBB");
    let run = |frame: &DecodedFrame| {
        let mut report = Report::default();
        report.record_tables(frame);
        compare_extended(
            frame,
            &saved,
            ExtendedContent {
                membership: Ok(&membership),
                base_resources: Err("unused".into()),
                railway_adjacency: Err("unused".into()),
                buildings: Err("unused".into()),
                defines: Err("unused".into()),
                raid_graph: Err("unused".into()),
                equipment: Err("unused".into()),
                leader_traits: Err("unused".into()),
                intelligence: Err("unused".into()),
                temperature: Err("unused".into()),
                sub_units: Err("unused".into()),
                essential_caps: Err("unused".into()),
            },
            &mut report,
        );
        report
    };
    let report = run(&frame);
    assert_eq!(
        report.compared_tables["CountryGameScoreInputs saved rosters"],
        2
    );
    assert_eq!(
        report.compared_tables["CountryGameScoreInputs.land_count"],
        2
    );
    assert_eq!(report.compared_tables["AirWingGameScoreStats.planes"], 0);
    for group in [
        "CountryGameScoreInputs",
        "CountryGameScoreInputs.land_count",
        "AirWingGameScoreStats",
        "GameScoreMultipliers",
    ] {
        assert!(!report.failed_comparisons.contains_key(group));
        assert_eq!(report.difference_groups.get(group).copied().unwrap_or(0), 0);
    }
    let TableView::Complete(TableRows::CountryGameScoreInputs(rows)) =
        &mut *frame.tables[TableId::CountryGameScoreInputs.index()]
    else {
        unreachable!()
    };
    rows.pop();
    let report = run(&frame);
    assert_eq!(report.difference_groups["CountryGameScoreInputs"], 1);
    assert_eq!(
        report.difference_groups["CountryGameScoreInputs.land_count"],
        1
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("BBB.game_score"))
    );
    assert!(
        !report
            .failed_comparisons
            .contains_key("AirWingGameScoreStats")
    );
}

#[test]
fn full_comparison_resolves_aliases_and_independent_map_membership() {
    let (frame, save, membership, equipment) = fixture();
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        (
            report.provinces,
            report.states,
            report.countries,
            report.scalar_checks
        ),
        (2, 1, 1, 6)
    );
    assert!(!report.limitations.is_empty());
}

#[test]
fn manpower_oracle_compares_both_stored_operands_without_waiving_differences() {
    let (mut frame, mut saved, membership, equipment) = fixture();
    saved
        .countries
        .get_mut(b"GER".as_slice())
        .unwrap()
        .manpower_residual = i32::MIN;
    saved.states.get_mut(&1).unwrap().manpower_available = Some(i32::MAX);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows[0].manpower_residual = i32::MIN;
    }
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *frame.tables[TableId::States.index()]
    {
        rows[0].manpower_available = i32::MAX;
    }
    assert!(
        compare(&frame, &saved, &membership, &equipment)
            .unwrap()
            .differences
            .is_empty()
    );
    saved
        .countries
        .get_mut(b"GER".as_slice())
        .unwrap()
        .manpower_residual += 1;
    saved.states.get_mut(&1).unwrap().manpower_available = Some(0);
    let report = compare(&frame, &saved, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert!(report.outcome().is_err());
}

#[test]
fn province_victory_points_compare_native_integer_extremes_and_proven_field_omission() {
    let (mut frame, mut saved, membership, equipment) = fixture();
    for value in [i32::MIN, -1, 0, i32::MAX] {
        saved.provinces.get_mut(&2).unwrap().victory_points = Some(value);
        let TableView::Complete(TableRows::Provinces(rows)) =
            &mut *frame.tables[TableId::Provinces.index()]
        else {
            unreachable!()
        };
        rows[1].victory_points = value;
        let report = compare(&frame, &saved, &membership, &equipment).unwrap();
        assert!(report.outcome().is_ok(), "{report:?}");
        assert_eq!(report.compared_tables["Provinces.victory_points"], 2);
    }
    saved.provinces.get_mut(&2).unwrap().victory_points = None;
    let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *frame.tables[TableId::Provinces.index()]
    else {
        unreachable!()
    };
    rows[1].victory_points = 1;
    let report = compare(&frame, &saved, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 1, "{report:?}");
    assert!(report.differences[0].contains("province 2.victory_points"));
    assert!(report.outcome().is_err());
    let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *frame.tables[TableId::Provinces.index()]
    else {
        unreachable!()
    };
    rows[1].victory_points = 0;
    assert!(
        compare(&frame, &saved, &membership, &equipment)
            .unwrap()
            .outcome()
            .is_ok()
    );
    let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *frame.tables[TableId::Provinces.index()]
    else {
        unreachable!()
    };
    rows[0].victory_points = -1; // The whole saved province entry is absent.
    let report = compare(&frame, &saved, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("province 1.victory_points"));
}

#[test]
fn missing_extra_and_changed_province_rows_are_differences() {
    let (mut frame, save, membership, equipment) = fixture();
    let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *frame.tables[TableId::Provinces.index()]
    else {
        unreachable!()
    };
    rows.remove(0);
    let province = &mut rows[0];
    // Explicit saved control cannot be manufactured from captured raw zero.
    province.controller = 0;
    rows.push(ProvinceRow {
        id: 3,
        ..Default::default()
    });
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
}

#[test]
fn state_ownership_and_controller_differences_are_independent_of_provinces() {
    let (mut frame, save, membership, equipment) = fixture();
    let TableView::Complete(TableRows::States(rows)) = &mut *frame.tables[TableId::States.index()]
    else {
        unreachable!()
    };
    let state = &mut rows[0];
    state.owner = 0;
    state.controller = 0;
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert!(report.differences[0].starts_with("state 1.owner:"));
    assert!(report.differences[1].starts_with("state 1.controller:"));
    assert_eq!(report.provinces, 2);
}

/// The rename compares with the stored-length-gated saved `name`;
/// the unsaved owner name is a named limitation, never compared.
#[test]
fn state_rename_compares_with_the_saved_name_and_the_owner_name_is_a_limitation() {
    fn first(frame: &mut replay::frame::DecodedFrame) -> &mut StateRow {
        let TableView::Complete(TableRows::States(rows)) =
            &mut *frame.tables[TableId::States.index()]
        else {
            unreachable!()
        };
        &mut rows[0]
    }
    let (mut frame, mut save, membership, equipment) = fixture();
    first(&mut frame).owner_name = b"not in the save".to_vec();
    first(&mut frame).continent = Recorded::Value(Some(999));
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .values()
            .any(|subjects| subjects.iter().any(|s| s == "States.owner_name"))
    );
    assert!(
        report
            .limitations
            .values()
            .any(|subjects| subjects.iter().any(|subject| subject == "States.continent"))
    );
    first(&mut frame).custom_name = b"Sudetenland".to_vec();
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].starts_with("state 1.custom_name:"));
    save.states.get_mut(&1).unwrap().name = Some(b"Sudetenland".to_vec());
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
}

#[test]
fn omitted_state_controller_does_not_accept_raw_zero_or_hide_missing_rows() {
    let (mut frame, save, membership, equipment) = fixture();
    let TableView::Complete(TableRows::States(rows)) = &mut *frame.tables[TableId::States.index()]
    else {
        unreachable!()
    };
    let state = &mut rows[0];
    state.controller = 0;
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].starts_with("state 1.controller:"));

    *frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![StateRow {
            id: 2,
            owner: 101,
            controller: 101,
            ..StateRow::default()
        }]));
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert_eq!(report.states, 0);
}

#[test]
fn explicit_state_controller_is_compared_even_when_owner_and_provinces_match() {
    let (mut frame, mut save, membership, equipment) = fixture();
    add_country(&mut frame, &mut save, 201, 8, b"D01");
    save.states.get_mut(&1).unwrap().controller = Some(b"D01".to_vec());
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].starts_with("state 1.controller:"));
    let TableView::Complete(TableRows::States(rows)) = &mut *frame.tables[TableId::States.index()]
    else {
        unreachable!()
    };
    let state = &mut rows[0];
    state.controller = 201;
    assert!(
        compare(&frame, &save, &membership, &equipment)
            .unwrap()
            .differences
            .is_empty()
    );
}

#[test]
fn invalid_incomplete_or_unresolvable_states_are_refused() {
    let (frame, save, membership, equipment) = fixture();
    for view in [
        TableView::Unavailable {
            reason: "state pointer unreadable".into(),
        },
        TableView::Truncated {
            rows: TableRows::empty(TableId::States),
            missing: 1,
        },
        TableView::Complete(TableRows::States(vec![StateRow {
            id: 1,
            owner: 777,
            controller: 101,
            ..StateRow::default()
        }])),
        TableView::Complete(TableRows::States(vec![StateRow {
            id: 1,
            owner: 101,
            controller: 777,
            ..StateRow::default()
        }])),
        TableView::Complete(TableRows::States(vec![
            StateRow::default(),
            StateRow::default(),
        ])),
        TableView::Complete(TableRows::Provinces(vec![ProvinceRow::default()])),
    ] {
        let mut invalid = frame.clone();
        *invalid.tables[TableId::States.index()] = view;
        assert!(
            compare(&invalid, &save, &membership, &equipment)
                .and_then(|report| report.outcome())
                .is_err()
        );
    }
}

#[test]
fn division_oracle_compares_display_precision_and_three_distinct_owners() {
    let (frame, save, membership, equipment) = division_fixture();
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.divisions, 1);
    assert_eq!(report.scalar_checks, 20);
    // The report names each column compared at display precision.
    assert_eq!(
        report.display_precision_lines()[1..],
        [
            "  Divisions.hit_points: 0.1 (fixed5, toward zero, nonzero kept)".to_owned(),
            "  Divisions.organisation: 0.1 (fixed5, toward zero)".to_owned(),
        ]
    );
    let mut changed = frame.clone();
    let row = division_row_mut(&mut changed);
    row.hit_points -= 1;
    row.organisation += 1;
    row.roster_owner = 201;
    row.logical_owner = 301;
    row.expeditionary_owner = 101;
    row.province = 2;
    row.template ^= 1;
    let report = compare(&changed, &save, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 7, "{:?}", report.differences);
}

#[test]
fn division_type_word_is_part_of_identity_and_duplicate_full_handles_are_refused() {
    let (mut frame, save, membership, equipment) = division_fixture();
    division_row_mut(&mut frame).id ^= 1;
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert_eq!(report.divisions, 0);
    let (mut frame, save, membership, equipment) = division_fixture();
    let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *frame.tables[TableId::Divisions.index()]
    else {
        unreachable!()
    };
    rows.push(rows[0].clone());
    assert!(
        comparison_error(compare(&frame, &save, &membership, &equipment))
            .contains("duplicate full division handle")
    );
}

#[test]
fn division_optional_omissions_require_captured_nulls() {
    let (mut frame, mut save, membership, equipment) = division_fixture();
    let expected = save.divisions.values_mut().next().unwrap();
    expected.province = None;
    expected.template = None;
    expected.expeditionary_owner = None;
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    let row = division_row_mut(&mut frame);
    row.province = 0;
    row.template = 0;
    row.expeditionary_owner = 0;
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("null standard template"))
    );
    division_row_mut(&mut frame).expeditionary_owner = u32::MAX;
    assert!(
        comparison_error(compare(&frame, &save, &membership, &equipment))
            .contains("cannot verify these bits")
    );
}

#[test]
fn division_incomplete_tables_and_unresolved_references_cannot_pass() {
    let (frame, save, membership, equipment) = division_fixture();
    for table in [
        TableView::Unavailable {
            reason: "division unreadable".into(),
        },
        TableView::Truncated {
            rows: TableRows::empty(TableId::Divisions),
            missing: 1,
        },
        TableView::Complete(TableRows::Provinces(vec![ProvinceRow::default()])),
        TableView::Complete(TableRows::Divisions(vec![DivisionRow::default()])),
    ] {
        let mut invalid = frame.clone();
        *invalid.tables[TableId::Divisions.index()] = table;
        assert!(
            compare(&invalid, &save, &membership, &equipment)
                .and_then(|report| report.outcome())
                .is_err()
        );
    }
    let mut invalid = frame.clone();
    division_row_mut(&mut invalid).logical_owner = 777;
    assert!(
        comparison_error(compare(&invalid, &save, &membership, &equipment))
            .contains("unresolved country reference")
    );
    let mut invalid_save = save.clone();
    invalid_save
        .divisions
        .values_mut()
        .next()
        .unwrap()
        .logical_owner = b"UNKNOWN".to_vec();
    assert!(
        comparison_error(compare(&frame, &invalid_save, &membership, &equipment))
            .contains("unknown saved country")
    );
    let mut invalid_save = save;
    invalid_save.divisions.values_mut().next().unwrap().province = Some(999_999);
    assert!(
        comparison_error(compare(&frame, &invalid_save, &membership, &equipment))
            .contains("verified map")
    );
}

#[test]
fn incomplete_tables_and_unresolved_links_cannot_pass() {
    let (mut frame, save, membership, equipment) = fixture();
    *frame.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "bad pointer".into(),
    };
    assert!(
        comparison_error(compare(&frame, &save, &membership, &equipment)).contains("bad pointer")
    );
    *frame.tables[TableId::Provinces.index()] = TableView::Truncated {
        rows: TableRows::empty(TableId::Provinces),
        missing: 2,
    };
    assert!(
        compare(&frame, &save, &membership, &equipment)
            .and_then(|report| report.outcome())
            .is_err()
    );
    let (mut frame, save, membership, equipment) = fixture();
    let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    else {
        unreachable!()
    };
    rows.pop();
    assert!(comparison_error(compare(&frame, &save, &membership, &equipment)).contains("102"));
}

#[test]
fn ambiguous_branch_and_wrong_hour_are_refused() {
    assert_eq!(unique_branch(&[2]).unwrap(), 2);
    assert!(
        unique_branch(&[1, 2])
            .unwrap_err()
            .to_string()
            .contains("--branch")
    );
    assert!(unique_branch(&[]).is_err());
    assert_eq!(matching_hour(20, None).unwrap(), 20);
    assert!(matching_hour(20, Some(21)).is_err());
}

#[test]
fn fixed_point_difference_of_one_is_not_rounded_away() {
    let mut report = Report::default();
    report.scalar(
        "GER",
        "political_power",
        Some(9_007_199_254_740_992),
        Some(&Decimal::parse(b"90071992547.40993").unwrap()),
    );
    assert_eq!(report.differences.len(), 1);
    assert_eq!(report.scalar_checks, 1);
    report.scalar("GER", "command_power", Some(0), None);
    assert_eq!(report.differences.len(), 2);
    report.scalar(
        "GER",
        "political_power",
        None,
        Some(&Decimal::parse(b"1").unwrap()),
    );
    assert_eq!(report.limitations.len(), 1);
}

#[test]
fn missing_convoy_data_is_an_error_when_capture_has_a_value() {
    let (frame, mut save, membership, equipment) = fixture();
    save.countries
        .get_mut(b"GER".as_slice())
        .unwrap()
        .convoy_equipment = None;
    assert!(
        comparison_error(compare(&frame, &save, &membership, &equipment))
            .contains("convoy equipment pool")
    );
    save.countries
        .get_mut(b"GER".as_slice())
        .unwrap()
        .convoy_equipment = Some(vec![save::EquipmentAmount {
        id: 7,
        id_type: 70,
        amount: Decimal::parse(b"1").unwrap(),
    }]);
    assert!(
        comparison_error(compare(&frame, &save, &membership, &equipment))
            .contains("equipment definition")
    );
    save.equipment_definitions
        .insert((7, 70), b"unknown_variant".to_vec());
    assert!(
        comparison_error(compare(&frame, &save, &membership, &equipment))
            .contains("verified content has no equipment unknown_variant")
    );
}

#[test]
fn convoy_clamp_is_per_row_and_preserves_order() {
    assert_eq!(convoy_add(convoy_add(0, i64::MAX), -10), CONVOY_LIMIT - 10);
    assert_eq!(convoy_add(convoy_add(0, i64::MIN), 10), -CONVOY_LIMIT + 10);
}

/// A stockpile row counts as a convoy by its variant's flags:
/// the type's categories, else its
/// archetype's, within the archetype's `allowed_types`, normalized; the
/// type is found without case. A modular type is decided only while no
/// module of the content can add the convoy bit.
#[test]
fn convoy_variants_follow_the_variant_category() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let equipment = "equipments = {
            Convoy = { is_archetype = yes type = convoy }
            convoy_1 = { archetype = convoy }
            raider = { is_archetype = yes type = { convoy screen_ship } }
            barge = { is_archetype = yes type = { convoy armor } allowed_types = { armor } }
            barge_1 = { archetype = barge }
            hull = { is_archetype = yes type = screen_ship module_slots = { main = { } } }
        }";
    let load = |module: &str| {
        installed_content::equipment::fixture::load(&[
            (
                "common/units/equipment/modules/m.txt",
                &format!("equipment_modules = {{ gun = {{ add_equipment_type = {module} }} }}"),
            ),
            ("common/units/equipment/a.txt", equipment),
        ])
        .0
    };
    let database = load("capital_ship");
    let convoy = |name: &str| convoy_variant(&database, name.as_bytes());
    assert_eq!(convoy("CONVOY_1"), Ok(true));
    // The screen ship bit clears the convoy bit.
    assert_eq!(convoy("raider"), Ok(false));
    // The archetype allows armor only.
    assert_eq!(convoy("barge_1"), Ok(false));
    assert_eq!(convoy("hull"), Ok(false));
    assert!(
        convoy("missing")
            .unwrap_err()
            .contains("has no equipment missing")
    );
    let database = load("convoy");
    let refused = convoy_variant(&database, b"hull").unwrap_err();
    assert!(
        refused.contains("equipment hull has module slots"),
        "{refused}"
    );
}

#[test]
fn save_checksum_suffix_does_not_hide_a_different_build() {
    assert!(same_version(b"build (d245)", "build"));
    assert!(same_version(b"build", "build"));
    assert!(!same_version(b"build2 (d245)", "build"));
    assert!(!same_version(b"build (xxxx)", "build"));
}

#[test]
fn player_oracle_compares_refreshed_fields_and_reports_header_limits() {
    let (mut frame, mut saved, membership, equipment) = fixture();
    *frame.tables[TableId::Players.index()] =
        TableView::Complete(TableRows::Players(vec![replay::rows::PlayerRow {
            id: i32::MIN,
            country: 102,
            flags: 0x81,
            user: b"user".to_vec(),
            name: b"different display".to_vec(),
        }]));
    saved.players = vec![
        save::Player {
            id: i32::MIN,
            country: b"GER".to_vec(),
            user: b"user".to_vec(),
            country_leader: true,
        },
        save::Player {
            id: 20,
            country: b"GER".to_vec(),
            user: b"retained".to_vec(),
            country_leader: false,
        },
    ];
    let report = compare(&frame, &saved, &membership, &equipment).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.players, 1);
    assert!(
        report
            .limitations
            .keys()
            .any(|key| key.contains("retains users"))
    );
    saved.players[0].id += 1;
    saved.players[0].country_leader = false;
    assert_eq!(
        compare(&frame, &saved, &membership, &equipment)
            .unwrap()
            .differences
            .len(),
        2
    );
    saved.players[0].user = b"renamed".to_vec();
    let report = compare(&frame, &saved, &membership, &equipment).unwrap();
    assert_eq!(report.players, 0);
    assert_eq!(report.differences.len(), 1);
}

#[test]
fn player_duplicate_user_collapse_never_certifies_every_current_player() {
    let (mut frame, mut saved, membership, equipment) = fixture();
    *frame.tables[TableId::Players.index()] = TableView::Complete(TableRows::Players(vec![
        replay::rows::PlayerRow {
            id: 1,
            country: 101,
            user: Vec::new(),
            ..Default::default()
        },
        replay::rows::PlayerRow {
            id: 2,
            country: 0,
            user: Vec::new(),
            ..Default::default()
        },
    ]));
    saved.players = vec![save::Player {
        id: 2,
        country: b"---".to_vec(),
        user: Vec::new(),
        country_leader: false,
    }];
    let report = compare(&frame, &saved, &membership, &equipment).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.players, 0);
    assert!(
        report
            .limitations
            .keys()
            .any(|key| key.contains("collapses equal"))
    );
    saved.players[0].id = 3;
    assert_eq!(
        compare(&frame, &saved, &membership, &equipment)
            .unwrap()
            .differences
            .len(),
        1
    );
}

#[test]
fn land_combat_sides_match_the_saved_combat_census() {
    let (mut frame, mut saved, _, _) = fixture();
    let combat = (7_u64 << 32) | 62;
    saved.land_combats.insert(
        combat,
        save::LandCombat {
            details: Default::default(),
            id: combat,
            province: 2,
            day: -1,
            duration: 1,
            attackers: Vec::new(),
            defenders: Vec::new(),
        },
    );
    let other = (8_u64 << 32) | 62;
    *frame.tables[TableId::LandCombatSides.index()] =
        TableView::Complete(TableRows::LandCombatSides(vec![
            replay::rows::LandCombatSideRow {
                combat: other,
                ..Default::default()
            },
        ]));
    let mut report = Report::default();
    combat_details::compare(&frame, &saved, &mut report);
    for (combat, change) in [(combat, "absent"), (other, "present")] {
        assert!(
            report
                .differences
                .iter()
                .any(|d| d.starts_with(&format!("LandCombatSides {combat}")) && d.contains(change))
        );
    }
}

#[test]
fn land_combat_primary_sets_hold_every_saved_units_logical_owner() {
    let (mut frame, mut saved, _, _) = fixture();
    let combat = (7_u64 << 32) | 62;
    let (attacker, defender) = ((1_u64 << 32) | 51, (1_u64 << 32) | 52);
    saved.land_combats.insert(
        combat,
        save::LandCombat {
            details: Default::default(),
            id: combat,
            province: 2,
            day: -1,
            duration: 1,
            attackers: vec![attacker],
            defenders: vec![defender],
        },
    );
    for unit in [attacker, defender] {
        saved.divisions.insert(
            unit,
            save::Division {
                id: unit,
                roster_owner: b"GER".to_vec(),
                logical_owner: b"GER".to_vec(),
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
    // The attacker set names GER through its alias; the defender set
    // lacks it.
    *frame.tables[TableId::LandCombatSides.index()] =
        TableView::Complete(TableRows::LandCombatSides(vec![
            replay::rows::LandCombatSideRow {
                combat,
                attacker_primary: vec![0, 102],
                ..Default::default()
            },
        ]));
    let mut report = Report::default();
    combat_details::compare(&frame, &saved, &mut report);
    let found: Vec<_> = report
        .differences
        .iter()
        .filter(|d| d.starts_with("LandCombatSides"))
        .collect();
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("defender_primary") && found[0].contains("GER"));
}

#[test]
fn map_combat_and_order_oracle_preserves_type_words_order_and_raw_fields() {
    let (mut frame, mut saved, membership, equipment) = fixture();
    let combat = (u64::from(u32::MAX) << 32) | 62;
    let unit = (1_u64 << 32) | 51;
    saved.land_combats.insert(
        combat,
        save::LandCombat {
            details: Default::default(),
            id: combat,
            province: 1,
            day: -1,
            duration: 2,
            attackers: vec![unit, unit + 1],
            defenders: vec![],
        },
    );
    *frame.tables[TableId::LandCombats.index()] =
        TableView::Complete(TableRows::LandCombats(vec![replay::rows::LandCombatRow {
            id: combat,
            province: 1,
            day: -1,
            duration: 2,
            attackers: vec![unit, unit + 1],
            defenders: vec![],
        }]));
    let group = (u64::from(u32::MAX) << 32) | 53;
    saved.army_orders.insert(
        (group, u32::MAX),
        save::ArmyOrder {
            owner_group: group,
            instance: u32::MAX,
            country: b"GER".to_vec(),
            order_type: 4,
            active: 128,
            paradrop_target: u32::MAX,
            path: vec![2, 1, 2],
            target_states: vec![1],
        },
    );
    *frame.tables[TableId::ArmyOrders.index()] =
        TableView::Complete(TableRows::ArmyOrders(vec![replay::rows::ArmyOrderRow {
            owner_group: group,
            instance: u32::MAX,
            country: 102,
            order_type: 4,
            active: 128,
            paradrop_target: u32::MAX,
            path: vec![2, 1, 2],
            target_states: vec![1],
        }]));
    let report = compare(&frame, &saved, &membership, &equipment).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!((report.land_combats, report.army_orders), (1, 1));
    saved
        .land_combats
        .get_mut(&combat)
        .unwrap()
        .attackers
        .reverse();
    let order = saved.army_orders.get_mut(&(group, u32::MAX)).unwrap();
    order.path.swap(0, 1);
    order.active = 0;
    assert_eq!(
        compare(&frame, &saved, &membership, &equipment)
            .unwrap()
            .differences
            .len(),
        3
    );
}

#[test]
fn air_sortie_oracle_compares_the_saved_penalty_and_checks_operand_keys() {
    use replay::rows::{
        AirMissionInputRow, AirReferenceWitnessRow, AirRegionalMissionRow, AirSortieBaseCountryRow,
        AirSortieModifierMetadataRow, AirSortieModifierRow, AirSortieSpecialBaseRow,
    };
    let (mut frame, mut saved, _, _) = fixture();
    let wing = (1_u64 << 32) | 54;
    let base = (5_u64 << 32) | 63;
    saved.air_wings.insert(
        wing,
        save::AirWing {
            name: None,
            id: wing,
            owner: b"GER".to_vec(),
            country_override: b"---".to_vec(),
            air_base: base,
            planes: 100,
            mission_mask: 1,
            mission_region: Some(3),
            mission_active: Some(true),
            region_change_penalty: Some(90_000),
            pool_ordinal: 0,
        },
    );
    saved.air.bases.push(installed_content_save::air::AirBase {
        id: base,
        state: Some(1),
        host: None,
        capacity: 0,
        kind: 1,
    });
    let set = |frame: &mut DecodedFrame, rows: TableRows| {
        let index = rows.table().index();
        *frame.tables[index] = TableView::Complete(rows);
    };
    set(
        &mut frame,
        TableRows::AirWings(vec![replay::rows::AirWingRow {
            id: wing,
            owner: 102,
            air_base: base,
            mission_mask: 1,
            mission_region: Some(3),
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::AirRegionalMissions(vec![AirRegionalMissionRow { wing, ordinal: 0 }]),
    );
    set(
        &mut frame,
        TableRows::AirReferenceWitnesses(vec![AirReferenceWitnessRow {
            handle: base,
            base_kind: Some(1),
            state: Some(1),
            province: Some(1),
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::AirMissionInputs(vec![AirMissionInputRow {
            wing,
            reassignment_penalty_raw: 90_000,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::AirSortieBaseCountries(vec![AirSortieBaseCountryRow {
            base,
            country: 7,
            capacity_raw: 100_000,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::AirSortieModifiers(vec![
            AirSortieModifierRow {
                scope: AirSortieModifierRow::WEATHER,
                region: 3,
                raw: -10_000,
                ..Default::default()
            },
            AirSortieModifierRow {
                scope: AirSortieModifierRow::COUNTRY_REGION,
                subject: 7,
                region: 3,
                raw: 5_000,
            },
            AirSortieModifierRow {
                scope: AirSortieModifierRow::EFFICIENCY_OVERRIDE,
                subject: 9,
                region: 0,
                raw: 0,
            },
        ]),
    );
    set(
        &mut frame,
        TableRows::AirSortieSpecialBases(vec![AirSortieSpecialBaseRow {
            base,
            rocket_slots: Some(2),
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::AirSortieModifierMetadata(vec![AirSortieModifierMetadataRow {
            flags: 0x10,
            feature_selector: 8,
            callback_kind: 1,
        }]),
    );
    let run = |frame: &DecodedFrame, saved: &Save| {
        let mut report = Report::default();
        let countries = country_context(frame).unwrap();
        air_missions::compare(frame, saved, &countries, &mut report);
        report
    };
    let report = run(&frame, &saved);
    assert!(
        report.failed_comparisons.is_empty(),
        "{:?}",
        report.failed_comparisons
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["AirSortieModifiers"], 3);

    saved
        .air_wings
        .get_mut(&wing)
        .unwrap()
        .region_change_penalty = Some(90_001);
    let report = run(&frame, &saved);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("region_change_penalty"));

    // A stateful rocket base has no carrier stance.
    let TableView::Complete(TableRows::AirSortieSpecialBases(rows)) =
        &mut *frame.tables[TableId::AirSortieSpecialBases.index()]
    else {
        unreachable!()
    };
    rows[0].carrier_stance = Some(1);
    assert!(
        run(&frame, &saved)
            .failed_comparisons
            .contains_key("air sortie operands")
    );
    rows_reset(&mut frame);
    set(
        &mut frame,
        TableRows::AirSortieBaseCountries(vec![AirSortieBaseCountryRow {
            base,
            country: 8,
            capacity_raw: 1,
            ..Default::default()
        }]),
    );
    assert!(
        run(&frame, &saved)
            .failed_comparisons
            .contains_key("air sortie operands")
    );

    fn rows_reset(frame: &mut DecodedFrame) {
        *frame.tables[TableId::AirSortieSpecialBases.index()] =
            TableView::Complete(TableRows::AirSortieSpecialBases(vec![
                AirSortieSpecialBaseRow {
                    base: (5_u64 << 32) | 63,
                    rocket_slots: Some(2),
                    ..Default::default()
                },
            ]));
    }
}

#[test]
fn air_and_naval_oracle_compares_native_values_and_slot_identity() {
    let (mut frame, mut saved, membership, equipment) = fixture();
    let wing = (1_u64 << 32) | 54;
    let wing_name = b"Stored wing name".to_vec();
    saved.air_wings.insert(
        wing,
        save::AirWing {
            name: Some(wing_name.clone()),
            id: wing,
            owner: b"GER".to_vec(),
            country_override: b"---".to_vec(),
            air_base: 63,
            planes: 100,
            mission_mask: 2,
            mission_region: Some(3),
            mission_active: Some(true),
            region_change_penalty: None,
            pool_ordinal: 0,
        },
    );
    let ace = (7_u64 << 32) | 4713;
    saved.air.wings.push(save::air::Wing {
        id: wing,
        pool_type: b"fixture_plane".to_vec(),
        name: Some(wing_name.clone()),
        accidents: None,
        experience: Decimal::parse(b"0").unwrap(),
        equipment: save::air::EquipmentPool {
            entries: Vec::new(),
            allow_zero: false,
        },
        ace: Some(ace),
        raid: None,
    });
    *frame.tables[TableId::AirWings.index()] =
        TableView::Complete(TableRows::AirWings(vec![replay::rows::AirWingRow {
            base_range_raw: 0,
            ace,
            id: wing,
            name: replay::rows::Recorded::Value(Some(wing_name)),
            owner: 102,
            country_override: 0,
            air_base: 63,
            base_override: 99,
            planes: 100,
            mission_mask: 2,
            mission_region: Some(3),
            mission_active: 1,
            ..Default::default()
        }]));
    let combat = (1_u64 << 32) | 61;
    let zero = Decimal::parse(b"0").unwrap();
    saved.naval_combats.insert(
        combat,
        save::NavalCombat {
            details: Default::default(),
            id: combat,
            province: 2,
            attackers: Vec::new(),
            defenders: Vec::new(),
            attacker_positioning: Decimal::parse(b"0.00001").unwrap(),
            defender_positioning: zero.clone(),
            attacker_anti_air: zero.clone(),
            defender_anti_air: zero,
            transfers: Vec::new(),
        },
    );
    *frame.tables[TableId::NavalCombats.index()] =
        TableView::Complete(TableRows::NavalCombats(vec![
            replay::rows::NavalCombatRow {
                id: combat,
                province: 2,
                attacker_positioning: 1,
                ..Default::default()
            },
        ]));
    saved.naval_combat_participants.insert(
        (combat, 1, u32::MAX),
        save::NavalCombatParticipant {
            details: Default::default(),
            combat,
            side: 1,
            battle_slot: u32::MAX,
            ship: None,
            convoy: Some(69),
            owner: b"GER".to_vec(),
            state: 32,
        },
    );
    *frame.tables[TableId::NavalCombatParticipants.index()] =
        TableView::Complete(TableRows::NavalCombatParticipants(vec![
            replay::rows::NavalCombatParticipantRow {
                combat,
                side: 1,
                battle_slot: u32::MAX,
                ship: 0,
                owner: 101,
                state: 32,
            },
        ]));
    let report = compare(&frame, &saved, &membership, &equipment).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        (
            report.air_wings,
            report.naval_combats,
            report.naval_combat_participants
        ),
        (1, 1, 1)
    );
    saved.air_wings.get_mut(&wing).unwrap().mission_active = Some(false);
    // A wing listed elsewhere in its country's pools is another difference.
    saved.air_wings.get_mut(&wing).unwrap().pool_ordinal = 1;
    saved.air.wings[0].ace = Some(ace + (1 << 32));
    saved
        .naval_combats
        .get_mut(&combat)
        .unwrap()
        .attacker_positioning
        .raw = 0;
    saved
        .naval_combat_participants
        .get_mut(&(combat, 1, u32::MAX))
        .unwrap()
        .state = 0;
    assert_eq!(
        compare(&frame, &saved, &membership, &equipment)
            .unwrap()
            .differences
            .len(),
        5
    );
    // An omitted ace must not name a saved ace; a zero reference is none.
    saved.air.wings[0].ace = None;
    saved.air.aces = vec![save::air::Ace {
        id: ace,
        country: b"GER".to_vec(),
        ordinal: 0,
        modifier: None,
        female: false,
        portrait: 0,
        alive: true,
        handled: false,
        kill_type: 0,
        killer_ace: 0,
        killer_country: None,
        name: Vec::new(),
        surname: Vec::new(),
        callsign: None,
    }];
    let differences = |frame: &DecodedFrame, saved: &Save| {
        compare(frame, saved, &membership, &equipment)
            .unwrap()
            .differences
            .len()
    };
    assert_eq!(differences(&frame, &saved), 5);
    saved.air.aces.clear();
    assert_eq!(differences(&frame, &saved), 4);
    let TableView::Complete(TableRows::AirWings(rows)) =
        &mut *frame.tables[TableId::AirWings.index()]
    else {
        unreachable!()
    };
    rows[0].ace = 0;
    assert_eq!(differences(&frame, &saved), 4);
}

// Uses that module's existing fixture and add_country helpers.

fn naval_fixture() -> (
    DecodedFrame,
    Save,
    save::PartialMapMembership,
    installed_content::equipment::Database,
) {
    let (mut frame, mut saved, membership, equipment) = fixture();
    add_country(&mut frame, &mut saved, 201, 8, b"SOV");
    add_country(&mut frame, &mut saved, 301, 9, b"D01");
    let fleet = (u64::from(u32::MAX) << 32) | 61;
    let force = (2_147_483_648_u64 << 32) | 4713;
    let ship = (u64::from(u32::MAX) << 32) | 51;
    saved.fleets.insert(
        fleet,
        save::Fleet {
            id: fleet,
            owner: b"GER".to_vec(),
            home_base: Some(2),
            name: Some(b"Fixture Fleet".to_vec()),
            leader: None,
        },
    );
    saved.task_forces.insert(
        force,
        save::TaskForce {
            id: force,
            name: None,
            fleet,
            roster_owner: b"GER".to_vec(),
            logical_owner: b"SOV".to_vec(),
            expeditionary_owner: Some(b"D01".to_vec()),
            province: Some(1),
            mission: -1,
            country_intel: Vec::new(),
            path: vec![3, 7, 4],
            target: None,
            alternate_path: Vec::new(),
            movement_progress: None,
            retreat: false,
            withdrawal: false,
            exile: false,
        },
    );
    saved.ships.insert(
        ship,
        save::Ship {
            roster_ordinal: 0,
            id: ship,
            task_force: force,
            strength: Decimal::parse(b"90071992547.40993").unwrap(),
            organisation: Decimal::parse(b"78.72001").unwrap(),
            experience: Decimal::parse(b"34.32537").unwrap(),
            sunk_convoys: -1,
            equipment: None,
        },
    );
    *frame.tables[TableId::Fleets.index()] =
        TableView::Complete(TableRows::Fleets(vec![replay::rows::FleetRow {
            id: fleet,
            owner: 102,
            home_base: 2,
            name: replay::rows::Recorded::Value(Some(b"Fixture Fleet".to_vec())),
            leader: replay::rows::Recorded::Value(Some(0)),
        }]));
    *frame.tables[TableId::TaskForces.index()] =
        TableView::Complete(TableRows::TaskForces(vec![replay::rows::TaskForceRow {
            id: force,
            fleet,
            roster_owner: 102,
            logical_owner: 201,
            expeditionary_owner: 301,
            province: 1,
            mission: u32::MAX,
            route_count: 3,
            combat_references: 0,
        }]));
    *frame.tables[TableId::Ships.index()] =
        TableView::Complete(TableRows::Ships(vec![replay::rows::ShipRow {
            id: ship,
            task_force: force,
            strength: 9_007_199_254_740_993,
            organisation: 7_872_001,
            // The save's 34.32537 at its display step, a hundredth toward zero.
            experience: 3_432_000,
            sunk_convoys: -1,
            category: 9,
            class_id: -1,
        }]));
    (frame, saved, membership, equipment)
}

#[test]
fn naval_oracle_checks_precision_full_identity_and_parent_membership() {
    let (frame, saved, membership, equipment) = naval_fixture();
    let report = compare(&frame, &saved, &membership, &equipment).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!((report.fleets, report.task_forces, report.ships), (1, 1, 1));
    let mut changed = frame.clone();
    let TableView::Complete(TableRows::Ships(rows)) = &mut *changed.tables[TableId::Ships.index()]
    else {
        unreachable!()
    };
    let ship = &mut rows[0];
    ship.strength -= 1;
    ship.organisation += 1;
    ship.task_force ^= 1;
    let report = compare(&changed, &saved, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    let mut changed = frame.clone();
    let TableView::Complete(TableRows::Ships(rows)) = &mut *changed.tables[TableId::Ships.index()]
    else {
        unreachable!()
    };
    let ship = &mut rows[0];
    ship.id ^= 1; // Same numeric id, different type: missing and unexpected rows.
    let report = compare(&changed, &saved, &membership, &equipment).unwrap();
    assert_eq!(report.ships, 0);
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    // A route length other than the saved path's is a difference.
    let mut changed = frame.clone();
    let TableView::Complete(TableRows::TaskForces(rows)) =
        &mut *changed.tables[TableId::TaskForces.index()]
    else {
        unreachable!()
    };
    rows[0].route_count = -3;
    let report = compare(&changed, &saved, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    let mut changed = frame.clone();
    let TableView::Complete(TableRows::Ships(rows)) = &mut *changed.tables[TableId::Ships.index()]
    else {
        unreachable!()
    };
    rows[0].id ^= 1;
    let report = compare(&changed, &saved, &membership, &equipment).unwrap();
    assert_eq!(report.ships, 0);
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    for table in [TableId::Fleets, TableId::TaskForces, TableId::Ships] {
        let mut incomplete = frame.clone();
        *incomplete.tables[table.index()] = TableView::Unavailable {
            reason: "source unavailable".into(),
        };
        assert!(
            compare(&incomplete, &saved, &membership, &equipment)
                .and_then(|report| report.outcome())
                .is_err()
        );
    }
}

#[test]
fn naval_oracle_distinguishes_null_locations_and_expeditionary_omission_predicate() {
    let (mut frame, mut saved, membership, equipment) = naval_fixture();
    saved.fleets.values_mut().next().unwrap().home_base = None;
    let force = saved.task_forces.values_mut().next().unwrap();
    force.province = None;
    force.expeditionary_owner = None;
    let report = compare(&frame, &saved, &membership, &equipment).unwrap();
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    let TableView::Complete(TableRows::Fleets(rows)) = &mut *frame.tables[TableId::Fleets.index()]
    else {
        unreachable!()
    };
    let fleet = &mut rows[0];
    fleet.home_base = 0;
    let TableView::Complete(TableRows::TaskForces(rows)) =
        &mut *frame.tables[TableId::TaskForces.index()]
    else {
        unreachable!()
    };
    let force = &mut rows[0];
    force.province = 0;
    force.expeditionary_owner = u32::MAX; // Signed -1 is omitted, but not certified as raw zero.
    let report = compare(&frame, &saved, &membership, &equipment).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("signed-nonpositive expeditionary"))
    );
    saved.task_forces.values_mut().next().unwrap().province = Some(0);
    assert!(
        compare(&frame, &saved, &membership, &equipment)
            .and_then(|report| report.outcome())
            .is_err()
    );
}
/// A frame and save with two extra countries, so relation and intel rows can
/// use raw references that are aliases of a canonical slot.
fn relation_fixture() -> (
    DecodedFrame,
    Save,
    save::PartialMapMembership,
    installed_content::equipment::Database,
) {
    let (mut frame, mut save, membership, equipment) = fixture();
    // Tier-4 slot1 refresh hour: advance save and frame together to slot1.
    save.date.hour = 2;
    frame.hour = save.date.engine_hour().unwrap();
    if let TableView::Complete(TableRows::Clock(rows)) = &mut *frame.tables[TableId::Clock.index()]
    {
        rows[0].hour = frame.hour;
    }
    add_country(&mut frame, &mut save, 201, 8, b"D01");
    add_country(&mut frame, &mut save, 301, 9, b"SOV");
    let hour = frame.hour;
    relations_read_at(&mut frame, hour);
    (frame, save, membership, equipment)
}

/// SliceReads placing every linked country's relation rows at `hour`, as
/// a read of all their slices at that hour would.
fn relations_read_at(frame: &mut DecodedFrame, hour: i32) {
    let mut owners: Vec<u32> = frame
        .complete::<tables::CountryLinks>()
        .unwrap()
        .iter()
        .map(|link| link.id)
        .collect();
    owners.sort_unstable();
    owners.dedup();
    let mut reads = Vec::new();
    for table in [TableId::RelationOpinions, TableId::RelationTuples] {
        for &owner in &owners {
            reads.push(replay::rows::SliceReadRow {
                table: table.index() as u32,
                owner,
                sample_hour: hour,
            });
        }
    }
    reads.sort_by_key(|read| (read.table, read.owner));
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(reads));
}

/// Saved tags resolve through the context's tag map: aliased links still
/// name one canonical slot, and the lookup stays a map lookup for a
/// 5,000-country census. A tag the frame never captured, or one the save
/// does not declare, is refused rather than guessed.
#[test]
fn saved_country_tags_resolve_through_the_tag_map() {
    let (mut frame, mut save, _, _) = relation_fixture();
    for index in 0..5000_u32 {
        let tag = format!("Z{index:04}").into_bytes();
        add_country(&mut frame, &mut save, 10_000 + index, 100 + index, &tag);
    }
    let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    else {
        unreachable!()
    };
    links.push(CountryLinkRow {
        id: 302,
        canonical_slot: 9,
    });
    let context = country_context(&frame).unwrap();
    assert_eq!(saved_country_slot(b"SOV", &save, &context).unwrap(), 9);
    assert_eq!(saved_country_slot(b"D01", &save, &context).unwrap(), 8);
    for index in 0..5000_u32 {
        let tag = format!("Z{index:04}").into_bytes();
        assert_eq!(
            saved_country_slot(&tag, &save, &context).unwrap(),
            100 + index
        );
    }
    let saved = save.countries[b"GER".as_slice()].clone();
    save.countries.insert(b"XXX".to_vec(), saved);
    assert!(saved_country_slot(b"XXX", &save, &context).is_err());
    assert!(saved_country_slot(b"YYY", &save, &context).is_err());
}

fn saved_relation(
    save: &mut Save,
    owner: &[u8],
    target: &[u8],
    cached_sum: Option<i32>,
    tuples: &[(&[u8], u32)],
) {
    if let Some(cached_sum) = cached_sum {
        save.relation_opinions.push(save::RelationOpinion {
            owner: owner.to_vec(),
            target: target.to_vec(),
            cached_sum,
        });
    }
    for (kind, ordinal) in tuples {
        save.relation_tuples.push(save::RelationTuple {
            owner: owner.to_vec(),
            target: target.to_vec(),
            kind: kind.to_vec(),
            ordinal: *ordinal,
            first: owner.to_vec(),
            second: target.to_vec(),
            start_date: Some(save::GameDate {
                year: 1936,
                month: 1,
                day: 1,
                hour: 1,
            }),
            end_date: None,
            cancel: false,
        });
    }
}

fn opinions(frame: &mut DecodedFrame, rows: Vec<replay::rows::RelationOpinionRow>) {
    *frame.tables[TableId::RelationOpinions.index()] =
        TableView::Complete(TableRows::RelationOpinions(rows));
}

fn tuples(frame: &mut DecodedFrame, rows: Vec<replay::rows::RelationTupleRow>) {
    *frame.tables[TableId::RelationTuples.index()] =
        TableView::Complete(TableRows::RelationTuples(rows));
}

fn tuple_row(owner: u32, target: u32, ordinal: u16) -> replay::rows::RelationTupleRow {
    replay::rows::RelationTupleRow {
        owner_country: owner,
        target_country: target,
        type_id: 0x28da,
        ordinal,
        first_country: owner,
        second_country: target,
        start_hour: save::GameDate {
            year: 1936,
            month: 1,
            day: 1,
            hour: 1,
        }
        .engine_hour()
        .unwrap(),
        end_hour: 0,
        cancel: 0,
        retired: 0,
    }
}

fn differences(report: &Report) -> Vec<String> {
    report.differences.clone()
}

/// RelationTuples is registered in the extended pass, which needs verified
/// content; the comparator itself is exercised directly with the same
/// country links and recorded dictionary that pass supplies.
fn tuple_report(frame: &DecodedFrame, save: &Save, tokens: &[(u32, &'static [u8])]) -> Report {
    let mut report = Report::default();
    let context = country_context(frame).unwrap();
    let names: TokenNames<'static> = tokens.iter().copied().collect();
    report.attempt("RelationTuples", |report| {
        compare_relation_tuples(frame, save, &context, &names, report)
    });
    report
}

/// An air-access relation sets its first country's slot toward the second,
/// an attache relation the second's toward the first; a relation saved
/// under both endpoints is one slot.
#[test]
fn relation_links_match_the_saved_attache_and_air_access_relations() {
    let (frame, mut save, _, _) = relation_fixture();
    let names: TokenNames<'static> = [
        (engine::cattache_relation::TYPE, b"attache".as_slice()),
        (engine::cair_base_access_relation::TYPE, b"air_base_access"),
        (
            engine::coffer_air_base_access_relation::TYPE,
            b"offer_air_base_access",
        ),
        (engine::cmilitary_access_relation::TYPE, b"military_access"),
        (
            engine::coffer_military_access_relation::TYPE,
            b"offer_military_access",
        ),
        (engine::cdocking_rights_relation::TYPE, b"docking_rights"),
        (
            engine::coffer_docking_rights_relation::TYPE,
            b"offer_docking_rights",
        ),
    ]
    .into_iter()
    .collect();
    saved_relation(&mut save, b"D01", b"SOV", None, &[(b"air_base_access", 0)]);
    saved_relation(&mut save, b"D01", b"SOV", None, &[(b"attache", 1)]);
    // The same air-access relation, saved again under its second endpoint.
    save.relation_tuples.push(save::RelationTuple {
        owner: b"SOV".to_vec(),
        target: b"D01".to_vec(),
        ..save.relation_tuples[0].clone()
    });
    let run = |frame: &DecodedFrame, rows: Vec<replay::rows::RelationLinkRow>| {
        let mut frame = frame.clone();
        *frame.tables[TableId::RelationLinks.index()] =
            TableView::Complete(TableRows::RelationLinks(rows));
        let context = country_context(&frame).unwrap();
        let mut report = Report::default();
        visibility::compare_links(&frame, &save, &context, &names, &mut report).unwrap();
        report
    };
    let row = |country, target_index, links| replay::rows::RelationLinkRow {
        country,
        target_index,
        links,
        transport_subject: replay::rows::Recorded::NotRecorded,
    };
    // D01 is reference 201 at slot 8, SOV reference 301 at slot 9.
    let exact = vec![row(201, 9, 2), row(301, 8, 1)];
    let report = run(&frame, exact.clone());
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["RelationLinks"], 2);
    let report = run(&frame, vec![row(201, 9, 3), row(301, 8, 1)]);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    let report = run(&frame, vec![row(201, 9, 2)]);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    let mut extra = exact;
    extra.push(row(201, 8, 2));
    let report = run(&frame, extra);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
}

/// A military-access relation sets its first country's slot toward the
/// second; the offered form sets the second's toward the first.
#[test]
fn relation_links_match_the_saved_military_access_relations() {
    let (frame, mut save, _, _) = relation_fixture();
    let names: TokenNames<'static> = [
        (engine::cattache_relation::TYPE, b"attache".as_slice()),
        (engine::cair_base_access_relation::TYPE, b"air_base_access"),
        (
            engine::coffer_air_base_access_relation::TYPE,
            b"offer_air_base_access",
        ),
        (engine::cmilitary_access_relation::TYPE, b"military_access"),
        (
            engine::coffer_military_access_relation::TYPE,
            b"offer_military_access",
        ),
        (engine::cdocking_rights_relation::TYPE, b"docking_rights"),
        (
            engine::coffer_docking_rights_relation::TYPE,
            b"offer_docking_rights",
        ),
    ]
    .into_iter()
    .collect();
    saved_relation(&mut save, b"D01", b"SOV", None, &[(b"military_access", 0)]);
    saved_relation(
        &mut save,
        b"D01",
        b"SOV",
        None,
        &[(b"offer_military_access", 1)],
    );
    let mut frame = frame;
    let access = replay::rows::RelationLinkRow::MILITARY_ACCESS;
    *frame.tables[TableId::RelationLinks.index()] =
        TableView::Complete(TableRows::RelationLinks(vec![
            replay::rows::RelationLinkRow {
                country: 201,
                target_index: 9,
                links: access,
                transport_subject: replay::rows::Recorded::NotRecorded,
            },
            replay::rows::RelationLinkRow {
                country: 301,
                target_index: 8,
                links: access,
                transport_subject: replay::rows::Recorded::NotRecorded,
            },
        ]));
    let context = country_context(&frame).unwrap();
    let mut report = Report::default();
    visibility::compare_links(&frame, &save, &context, &names, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["RelationLinks"], 2);
}

/// Signed-positive navy access modes compare with the saved pairs;
/// high-bit bytes are omitted and remain explicit save limitations.
#[test]
fn naval_region_access_matches_the_saved_pairs() {
    let (mut frame, mut save, _, _) = relation_fixture();
    save.countries
        .get_mut(b"D01".as_slice())
        .unwrap()
        .naval_region_access = Some(vec![(18, 1), (79, 2)]);
    let row = |country, region, mode| replay::rows::NavalRegionAccessRow {
        country,
        region,
        mode,
    };
    let run = |frame: &mut DecodedFrame, rows| {
        *frame.tables[TableId::NavalRegionAccess.index()] =
            TableView::Complete(TableRows::NavalRegionAccess(rows));
        let context = country_context(frame).unwrap();
        let mut report = Report::default();
        visibility::compare_naval_access(frame, &save, &context, &mut report).unwrap();
        report
    };
    let report = run(&mut frame, vec![row(201, 18, 1), row(201, 79, 2)]);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["NavalRegionAccess"], 2);
    let report = run(&mut frame, vec![row(201, 18, 2), row(201, 79, 2)]);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    let report = run(
        &mut frame,
        vec![row(201, 18, 1), row(201, 79, 2), row(301, 5, 1)],
    );
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    let report = run(&mut frame, vec![row(201, 18, 1)]);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    let report = run(
        &mut frame,
        vec![row(201, 18, 1), row(201, 43, 128), row(201, 79, 2)],
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("omission proves signed nonpositive"))
    );
    let report = run(&mut frame, vec![row(201, 18, 255), row(201, 79, 2)]);
    assert_eq!(
        report.differences.len(),
        1,
        "a saved positive pair still disproves a high-bit current byte"
    );
}

#[test]
fn naval_region_access_refuses_saved_nonpositive_or_duplicate_pairs_and_zero_rows() {
    let (mut frame, mut save, _, _) = relation_fixture();
    for pairs in [
        vec![(18, 0)],
        vec![(18, 128)],
        vec![(18, 255)],
        vec![(18, 1), (18, 2)],
    ] {
        save.countries
            .get_mut(b"D01".as_slice())
            .unwrap()
            .naval_region_access = Some(pairs);
        *frame.tables[TableId::NavalRegionAccess.index()] =
            TableView::Complete(TableRows::NavalRegionAccess(vec![]));
        let context = country_context(&frame).unwrap();
        assert!(
            visibility::compare_naval_access(&frame, &save, &context, &mut Report::default())
                .is_err()
        );
    }
    save.countries
        .get_mut(b"D01".as_slice())
        .unwrap()
        .naval_region_access = None;
    *frame.tables[TableId::NavalRegionAccess.index()] =
        TableView::Complete(TableRows::NavalRegionAccess(vec![
            replay::rows::NavalRegionAccessRow {
                country: 201,
                region: 18,
                mode: 0,
            },
        ]));
    let context = country_context(&frame).unwrap();
    assert!(
        visibility::compare_naval_access(&frame, &save, &context, &mut Report::default()).is_err()
    );
}

/// Each essential archetype's stock is the sum of the saved stockpile's
/// variant amounts of that archetype, in whole units capped at its largest
/// raid requirement; a country with a variant the recorded definitions lack
/// is named, not derived, and a row of an archetype no raid type lists is a
/// difference.
#[test]
fn stockpile_archetypes_sum_the_saved_variant_amounts() {
    let (mut frame, mut save, _, _) = relation_fixture();
    save.production = save::production::parse(
        &installed_content::parser::parse(
            br#"countries={
                GER={production={equipments={allow_zero_entries=no}}}
                D01={production={equipments={
                    equipment={id={id=1 type=70} amount=3}
                    equipment={id={id=2 type=70} amount=1.5}
                    equipment={id={id=3 type=70} amount=2}
                    allow_zero_entries=no}}}
                SOV={production={equipments={
                    equipment={id={id=4 type=70} amount=1}
                    allow_zero_entries=no}}}}"#,
        )
        .unwrap()
        .items,
    )
    .unwrap();
    let variant = |id: u64| (id << 32) | 70;
    let definitions = |known: &[(u64, u32)]| {
        TableView::Complete(TableRows::VariantDefinitions(
            known
                .iter()
                .map(
                    |&(id, archetype_token)| replay::rows::VariantDefinitionRow {
                        id: variant(id),
                        archetype_token,
                        ..Default::default()
                    },
                )
                .collect(),
        ))
    };
    let row = |canonical_slot, archetype, units| replay::rows::StockpileArchetypeRow {
        canonical_slot,
        archetype,
        units,
    };
    // Archetypes 7, 8 and 9 are essential, required up to 3, 1 and 5.
    let tokens = BTreeMap::from([
        (7, b"bomber".as_slice()),
        (8, b"convoy".as_slice()),
        (9, b"tank".as_slice()),
        (10, b"rifle".as_slice()),
    ]);
    let caps: BTreeMap<Vec<u8>, i32> = [
        (b"bomber".to_vec(), 3),
        (b"convoy".to_vec(), 1),
        (b"tank".to_vec(), 5),
    ]
    .into();
    let run = |frame: &mut DecodedFrame, known: &[(u64, u32)], rows| {
        *frame.tables[TableId::VariantDefinitions.index()] = definitions(known);
        *frame.tables[TableId::StockpileArchetypes.index()] =
            TableView::Complete(TableRows::StockpileArchetypes(rows));
        let context = country_context(frame).unwrap();
        let mut report = Report::default();
        visibility::compare_stockpile_archetypes(
            frame,
            &save,
            &context,
            &tokens,
            Ok(&caps),
            &mut report,
        )
        .unwrap();
        report
    };
    let all = [(1, 7), (2, 7), (3, 8), (4, 7)];
    // D01 (slot 8): archetype 7 is 3 + 1.5, capped at 3; archetype 8 is 2,
    // capped at 1; SOV (slot 9) has 1 of archetype 7; a zero total without
    // variants is allowed.
    let exact = vec![row(8, 7, 3), row(8, 8, 1), row(8, 9, 0), row(9, 7, 1)];
    let report = run(&mut frame, &all, exact.clone());
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["StockpileArchetypes"], 3);
    // An uncapped total, a nonzero total without variants, a missing total.
    let report = run(
        &mut frame,
        &all,
        vec![row(8, 7, 4), row(8, 9, 1), row(9, 7, 1)],
    );
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    // Without variant 4's definition SOV is named and not compared.
    let report = run(&mut frame, &all[..3], exact.clone());
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["StockpileArchetypes"], 2);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("no VariantDefinitions row"))
    );
    // A non-essential archetype is not recorded.
    let mut extra = exact;
    extra.push(row(8, 10, 2));
    let report = run(&mut frame, &all, extra);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    // Without the caps nothing is compared.
    *frame.tables[TableId::VariantDefinitions.index()] = definitions(&all);
    let context = country_context(&frame).unwrap();
    let mut report = Report::default();
    visibility::compare_stockpile_archetypes(
        &frame,
        &save,
        &context,
        &tokens,
        Err("raid types unreadable".into()),
        &mut report,
    )
    .unwrap();
    assert_eq!(report.compared_tables["StockpileArchetypes"], 0);
}

/// Every country's ace rows compare their alive byte and assigned wing
/// with the saved roster and the wings' `ace` fields, whatever SliceReads
/// holds: the table is read every hour.
#[test]
fn ace_range_inputs_match_the_saved_rosters_and_wing_references() {
    let (frame, mut save, _, _) = relation_fixture();
    let ace = |country: &[u8], id: u64, alive| save::air::Ace {
        id,
        country: country.to_vec(),
        ordinal: 0,
        modifier: None,
        female: false,
        portrait: 0,
        alive,
        handled: false,
        kill_type: 0,
        killer_ace: 0,
        killer_country: None,
        name: Vec::new(),
        surname: Vec::new(),
        callsign: None,
    };
    let (assigned, dead, other) = ((1_u64 << 32) | 7, (2_u64 << 32) | 7, (3_u64 << 32) | 7);
    let wing = (5_u64 << 32) | 69;
    save.air.aces = vec![
        ace(b"D01", assigned, true),
        ace(b"D01", dead, false),
        ace(b"SOV", other, true),
    ];
    save.air.wings.push(save::air::Wing {
        id: wing,
        pool_type: b"fixture_plane".to_vec(),
        name: None,
        accidents: None,
        experience: Decimal::parse(b"0").unwrap(),
        equipment: save::air::EquipmentPool {
            entries: Vec::new(),
            allow_zero: false,
        },
        ace: Some(assigned),
        raid: None,
    });
    let row = |canonical_slot, ace, wing, alive| replay::rows::AceRangeInputRow {
        canonical_slot,
        ace,
        wing,
        alive,
        ..Default::default()
    };
    let run = |rows| {
        let mut frame = frame.clone();
        *frame.tables[TableId::AceRangeInputs.index()] =
            TableView::Complete(TableRows::AceRangeInputs(rows));
        let context = country_context(&frame).unwrap();
        let mut report = Report::default();
        visibility::compare_ace_inputs(&frame, &save, &context, &mut report).unwrap();
        report
    };
    let exact = vec![
        row(8, assigned, wing, 1),
        row(8, dead, 0, 0),
        row(9, other, 0, 1),
    ];
    let report = run(exact.clone());
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["AceRangeInputs"], 3);
    // SOV's wrong row is compared too.
    let report = run(vec![
        row(8, assigned, 0, 1),
        row(8, dead, wing, 1),
        row(9, other, 99, 0),
    ]);
    assert_eq!(report.differences.len(), 4, "{:?}", report.differences);
    // An unsaved ace, and both saved aces missing.
    let report = run(vec![row(8, assigned, wing, 1), row(8, 4 << 32, 0, 1)]);
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
}

#[test]
fn relation_opinions_compare_exactly_and_report_both_one_sided_pairs() {
    let (mut frame, mut save, membership, equipment) = relation_fixture();
    saved_relation(&mut save, b"GER", b"D01", Some(-100), &[]);
    opinions(
        &mut frame,
        vec![replay::rows::RelationOpinionRow {
            owner_country: 102,
            target_country: 201,
            opinion: -100,
        }],
    );
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert!(differences(&report).is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["RelationOpinions"], 1);

    // A changed value, a save-only pair and a replay-only pair all fail.
    let mut save = save.clone();
    save.relation_opinions[0].cached_sum = -35;
    saved_relation(&mut save, b"D01", b"GER", Some(7), &[]);
    let mut frame = frame.clone();
    opinions(
        &mut frame,
        vec![
            replay::rows::RelationOpinionRow {
                owner_country: 102,
                target_country: 201,
                opinion: -100,
            },
            replay::rows::RelationOpinionRow {
                owner_country: 201,
                target_country: 301,
                opinion: 5,
            },
        ],
    );
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    let differences = differences(&report);
    assert_eq!(differences.len(), 3, "{differences:?}");
    assert!(differences.iter().any(|line| line.contains("cached_sum")));
    assert!(
        differences
            .iter()
            .any(|line| line.contains("D01 -> GER") && line.contains("\"absent\""))
    );
}

/// A country's relation with itself is ordinary state: the live 2026-09-11
/// save serializes a self `active_relations` entry with its own cached sum,
/// so the comparator must compare it instead of refusing the whole group.
#[test]
fn a_self_relation_opinion_is_compared_like_any_other_pair() {
    let (mut frame, mut save, membership, equipment) = relation_fixture();
    saved_relation(&mut save, b"GER", b"GER", Some(-100), &[]);
    saved_relation(&mut save, b"GER", b"D01", Some(-35), &[]);
    opinions(
        &mut frame,
        vec![
            replay::rows::RelationOpinionRow {
                owner_country: 102,
                target_country: 102,
                opinion: -100,
            },
            replay::rows::RelationOpinionRow {
                owner_country: 102,
                target_country: 201,
                opinion: -35,
            },
        ],
    );
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert!(differences(&report).is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["RelationOpinions"], 2);

    // A wrong self opinion is still an ordinary difference, not a refusal.
    let mut save = save.clone();
    save.relation_opinions[0].cached_sum = -99;
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    let differences = differences(&report);
    assert_eq!(differences.len(), 1, "{differences:?}");
    assert!(
        differences[0].contains("GER -> GER") && differences[0].contains("cached_sum"),
        "{differences:?}"
    );
}

#[test]
fn a_recorded_zero_opinion_or_duplicate_pair_fails_the_comparison() {
    let (mut frame, save, membership, equipment) = relation_fixture();
    opinions(
        &mut frame,
        vec![replay::rows::RelationOpinionRow {
            owner_country: 102,
            target_country: 201,
            opinion: 0,
        }],
    );
    let error = comparison_error(compare(&frame, &save, &membership, &equipment));
    assert!(error.contains("omits a zero cached_sum"), "{error}");
    // Aliases 101 and 102 resolve to the same tag, so this is one pair twice.
    opinions(
        &mut frame,
        vec![
            replay::rows::RelationOpinionRow {
                owner_country: 101,
                target_country: 201,
                opinion: 1,
            },
            replay::rows::RelationOpinionRow {
                owner_country: 102,
                target_country: 201,
                opinion: 2,
            },
        ],
    );
    let error = comparison_error(compare(&frame, &save, &membership, &equipment));
    assert!(
        error.contains("duplicate resolved relation opinion"),
        "{error}"
    );
}

#[test]
fn relation_tuples_join_on_the_native_ordinal_and_the_recorded_token_spelling() {
    let (mut frame, mut save, _, _) = relation_fixture();
    saved_relation(&mut save, b"GER", b"D01", None, &[(b"guarantee", 3)]);
    tuples(&mut frame, vec![tuple_row(102, 201, 3)]);
    let named: &[(u32, &'static [u8])] = &[(0x28da, b"guarantee")];
    let report = tuple_report(&frame, &save, named);
    assert!(differences(&report).is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["RelationTuples"], 1);

    // A different ordinal is a different row on both sides.
    let mut moved = frame.clone();
    tuples(&mut moved, vec![tuple_row(102, 201, 4)]);
    let report = tuple_report(&moved, &save, named);
    assert_eq!(differences(&report).len(), 2, "{:?}", report.differences);

    // A spelling the dictionary contradicts is a named difference.
    let renamed: &[(u32, &'static [u8])] = &[(0x28da, b"embargo")];
    let report = tuple_report(&frame, &save, renamed);
    assert!(
        differences(&report)
            .iter()
            .any(|line| line.contains(".kind")),
        "{:?}",
        report.differences
    );

    // An unknown token compares every other field and prints the gap.
    let report = tuple_report(&frame, &save, &[]);
    assert!(differences(&report).is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("no spelling for some relation type tokens"))
    );

    // Endpoints, cancel and the conditional hours all compare exactly.
    let mut changed = frame.clone();
    tuples(&mut changed, {
        let mut row = tuple_row(102, 201, 3);
        row.second_country = 101;
        row.cancel = 1;
        row.end_hour = row.start_hour + 1;
        vec![row]
    });
    let report = tuple_report(&changed, &save, named);
    let lines = differences(&report);
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines.iter().any(|line| line.contains(".second")));
    assert!(lines.iter().any(|line| line.contains(".cancel")));
    assert!(lines.iter().any(|line| line.contains("end_date presence")));

    // A raw cancel byte outside 0/1 cannot be checked against yes/no.
    let mut raw = frame.clone();
    tuples(&mut raw, {
        let mut row = tuple_row(102, 201, 3);
        row.cancel = 0x80;
        vec![row]
    });
    let report = tuple_report(&raw, &save, named);
    assert!(report.failed_comparisons["RelationTuples"].contains("outside 0/1"));
}

#[test]
fn forced_full_refresh_compares_every_owner_whatever_its_read_hour() {
    let (mut frame, mut save, _, _) = relation_fixture();
    frame.hour = 0;
    // SliceReads places every owner earlier, but a daily table walked off
    // its slot shows that this hour read every owner.
    relations_read_at(&mut frame, -5);
    frame.metrics.table_ns[TableId::ResearchSlots.index()] = 1;
    saved_relation(&mut save, b"GER", b"D01", Some(5), &[]);
    opinions(
        &mut frame,
        vec![replay::rows::RelationOpinionRow {
            owner_country: 102,
            target_country: 201,
            opinion: 999,
        }],
    );
    let mut report = Report::default();
    report.record_tables(&frame);
    assert!(report.full_refresh());
    let context = country_context(&frame).unwrap();
    compare_relation_opinions(&frame, &save, &context, &mut report).unwrap();
    assert!(!report.differences.is_empty());
    // Without that signal no owner is current, and nothing is compared.
    let mut report = Report::default();
    compare_relation_opinions(&frame, &save, &context, &mut report).unwrap();
    assert!(report.differences.is_empty());
}

#[test]
fn owners_read_before_the_oracle_hour_are_not_compared() {
    let (mut frame, mut save, _, _) = relation_fixture();
    frame.hour = 0;
    relations_read_at(&mut frame, -5);
    saved_relation(&mut save, b"GER", b"D01", Some(5), &[]);
    opinions(
        &mut frame,
        vec![replay::rows::RelationOpinionRow {
            owner_country: 102,
            target_country: 201,
            opinion: 999,
        }],
    );
    let mut report = Report::default();
    let context = country_context(&frame).unwrap();
    compare_relation_opinions(&frame, &save, &context, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("one owner slice per hour")),
        "{:?}",
        report.limitations.keys()
    );
    let skipped = tuple_report(&frame, &save, &[]);
    assert!(skipped.differences.is_empty());
    assert!(
        skipped
            .limitations
            .keys()
            .any(|reason| reason.contains("one owner slice per hour")),
        "{:?}",
        skipped.limitations.keys()
    );
}

/// CountryIntel's walk can move off its slot when its shared buffer was
/// held; that is not a full hour, so carried owners stay uncompared.
#[test]
fn a_deferred_country_intel_walk_is_not_a_full_hour() {
    let (mut frame, mut save, _, _) = relation_fixture();
    frame.hour = 26;
    relations_read_at(&mut frame, 20);
    assert!(!replay::cadence::is_refresh(
        TableId::CountryIntel,
        frame.hour
    ));
    frame.metrics.table_ns[TableId::CountryIntel.index()] = 1;
    saved_relation(&mut save, b"GER", b"D01", Some(5), &[]);
    opinions(
        &mut frame,
        vec![replay::rows::RelationOpinionRow {
            owner_country: 102,
            target_country: 201,
            opinion: 999,
        }],
    );
    let mut report = Report::default();
    report.record_tables(&frame);
    assert!(!report.full_refresh());
    let context = country_context(&frame).unwrap();
    compare_relation_opinions(&frame, &save, &context, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
}

/// A country is current by its own ID, never by an alias link: GER (101,
/// alias 102) whose rows were carried from slice 5 is not current in
/// alias 102's slice 6, and a carried row whose target link vanished is
/// skipped before its target is resolved.
#[test]
fn an_alias_slice_does_not_make_a_carried_owner_current() {
    let (mut frame, mut save, _, _) = relation_fixture();
    assert_eq!(replay::cadence::slice_of(101), 5);
    assert_eq!(replay::cadence::slice_of(102), 6);
    frame.hour = 6;
    opinions(
        &mut frame,
        vec![
            replay::rows::RelationOpinionRow {
                owner_country: 101,
                target_country: 201,
                opinion: -100,
            },
            // A target reference no link names any more.
            replay::rows::RelationOpinionRow {
                owner_country: 101,
                target_country: 999,
                opinion: 4,
            },
        ],
    );
    *frame.tables[TableId::SliceReads.index()] =
        TableView::Complete(TableRows::SliceReads(vec![replay::rows::SliceReadRow {
            table: TableId::RelationOpinions.index() as u32,
            owner: 101,
            sample_hour: 5,
        }]));
    saved_relation(&mut save, b"GER", b"D01", Some(-35), &[]);
    let context = country_context(&frame).unwrap();
    let mut report = Report::default();
    compare_relation_opinions(&frame, &save, &context, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(!report.compared_tables.contains_key("RelationOpinions"));
}

/// Only the owners read at the oracle hour are compared: an owner
/// SliceReads places at the hour, and an owner without rows whose own
/// slice is the hour, because its read found none.
#[test]
fn a_sliced_hour_compares_exactly_the_owners_it_read() {
    let (mut frame, mut save, _, _) = relation_fixture();
    let ger = 102_u32;
    let d01 = 201_u32;
    // GER is read at this hour with one row; D01 has no rows and lies
    // outside this hour's slice; SOV has no rows and lies in it.
    frame.hour = replay::cadence::slice_of(301) as i32;
    assert_ne!(
        replay::cadence::slice_of(d01),
        replay::cadence::slice_of(301)
    );
    assert_ne!(
        replay::cadence::slice_of(ger),
        replay::cadence::slice_of(301)
    );
    opinions(
        &mut frame,
        vec![replay::rows::RelationOpinionRow {
            owner_country: ger,
            target_country: d01,
            opinion: -100,
        }],
    );
    *frame.tables[TableId::SliceReads.index()] =
        TableView::Complete(TableRows::SliceReads(vec![replay::rows::SliceReadRow {
            table: TableId::RelationOpinions.index() as u32,
            owner: ger,
            sample_hour: frame.hour,
        }]));
    saved_relation(&mut save, b"GER", b"D01", Some(-100), &[]);
    // D01 is not current: its saved pair is not compared.
    saved_relation(&mut save, b"D01", b"GER", Some(7), &[]);
    let context = country_context(&frame).unwrap();
    let mut report = Report::default();
    compare_relation_opinions(&frame, &save, &context, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["RelationOpinions"], 1);
    // SOV was read at this hour and found none: a saved SOV pair differs.
    saved_relation(&mut save, b"SOV", b"GER", Some(3), &[]);
    let mut report = Report::default();
    compare_relation_opinions(&frame, &save, &context, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
}

#[test]
fn country_intel_compares_every_positional_record_and_both_one_sided_lists() {
    let (mut frame, mut save, membership, equipment) = relation_fixture();
    save.country_intel.insert(
        b"GER".to_vec(),
        vec![
            save::CountryIntelRecord::default(),
            save::CountryIntelRecord {
                civilian: 3_331_020,
                army: -1_500_000,
                navy: i64::MAX,
                air: i64::MIN,
            },
        ],
    );
    *frame.tables[TableId::CountryIntel.index()] =
        TableView::Complete(TableRows::CountryIntel(vec![
            replay::rows::CountryIntelRow {
                owner_country: 102,
                target_index: 0,
                ..Default::default()
            },
            replay::rows::CountryIntelRow {
                owner_country: 102,
                target_index: 1,
                army: -1_500_000,
                air: i64::MIN,
                civilian: 3_331_020,
                navy: i64::MAX,
            },
        ]));
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert!(differences(&report).is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["CountryIntel"], 2);

    // One changed lane, one save-only index and one replay-only country.
    let mut save = save.clone();
    save.country_intel
        .get_mut(b"GER".as_slice())
        .unwrap()
        .push(save::CountryIntelRecord::default());
    save.country_intel.get_mut(b"GER".as_slice()).unwrap()[1].air = 0;
    let mut frame = frame.clone();
    let TableView::Complete(TableRows::CountryIntel(rows)) =
        &mut *frame.tables[TableId::CountryIntel.index()]
    else {
        unreachable!()
    };
    rows.push(replay::rows::CountryIntelRow {
        owner_country: 201,
        target_index: 0,
        ..Default::default()
    });
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    let differences = differences(&report);
    assert_eq!(differences.len(), 3, "{differences:?}");
    assert!(
        differences
            .iter()
            .any(|line| line.contains("GER.intel[1].airforce"))
    );
    assert!(differences.iter().any(|line| line.contains("GER.intel[2]")));
    assert!(differences.iter().any(|line| line.contains("D01.intel")));
}

/// The recorded table is sparse: an in-range index without a row compares
/// as zero lanes, and the greatest recorded index fixes the native count,
/// so a saved list longer or shorter than it is a difference.
#[test]
fn country_intel_reads_absent_in_range_records_as_zero_and_keeps_the_count() {
    let (mut frame, mut save, membership, equipment) = relation_fixture();
    let nonzero = save::CountryIntelRecord {
        army: -1_500_000,
        ..Default::default()
    };
    let mut saved = vec![save::CountryIntelRecord::default(); 5000];
    saved[1] = nonzero;
    save.country_intel.insert(b"GER".to_vec(), saved);
    let set = |frame: &mut DecodedFrame, last: u32| {
        *frame.tables[TableId::CountryIntel.index()] =
            TableView::Complete(TableRows::CountryIntel(vec![
                replay::rows::CountryIntelRow {
                    owner_country: 102,
                    target_index: 1,
                    army: -1_500_000,
                    ..Default::default()
                },
                replay::rows::CountryIntelRow {
                    owner_country: 102,
                    target_index: last,
                    ..Default::default()
                },
            ]));
    };
    set(&mut frame, 4999);
    let report = compare(&frame, &save, &membership, &equipment).unwrap();
    assert!(differences(&report).is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["CountryIntel"], 5000);

    // A recorded count short of the saved list leaves saved records absent.
    set(&mut frame, 3999);
    let lines = differences(&compare(&frame, &save, &membership, &equipment).unwrap());
    assert_eq!(lines.len(), 1000, "{:?}", &lines[..3]);
    assert!(lines.iter().all(|line| line.contains("GER.intel[")));

    // A recorded count past the saved list is one named difference.
    set(&mut frame, 5009);
    let lines = differences(&compare(&frame, &save, &membership, &equipment).unwrap());
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("GER.intel[5000..5010]"));
}

/// The oracle names the session's sample boundary and its instant limits.
#[test]
fn the_oracle_selects_the_session_sample_boundary() {
    let (mut frame, save, membership, equipment) = relation_fixture();
    let counters = save.counters;
    let TableView::Complete(TableRows::Clock(rows)) = &mut *frame.tables[TableId::Clock.index()]
    else {
        unreachable!()
    };
    rows[0].debug_current_ref_id = Some(counters.debug_current_ref_id + 2);
    rows[0].unit = Some(counters.unit);
    for (boundary, limitation, verdict) in [
        (
            SampleBoundary::EndOfHour,
            "the end-of-hour sample follows every command of hour H",
            "engine work ran after the save and before the end-of-hour sample: 1 counter(s) moved (debug_current_ref_id -2)",
        ),
        (
            SampleBoundary::AfterHourlyUpdate,
            "commands after the hourly callback can change state before saving",
            "1 counter(s) are lower in the save (debug_current_ref_id -2)",
        ),
    ] {
        let report = compare_at(&frame, &save, &membership, &equipment, boundary).unwrap();
        assert_eq!(report.sample_boundary, Some(boundary));
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.starts_with(limitation)),
            "{boundary}"
        );
        let found = report.boundary_verdict.clone().unwrap();
        assert!(found.contains(verdict), "{found}");
        assert!(differences(&report).is_empty());
    }
    assert!(boundary_description(SampleBoundary::EndOfHour).contains("end of hour H"));
}

#[test]
fn the_boundary_witness_reports_changes_without_certifying_an_equal_instant() {
    let after = SampleBoundary::AfterHourlyUpdate;
    let (mut frame, save, membership, equipment) = relation_fixture();
    let counters = save.counters;
    let TableView::Complete(TableRows::Clock(rows)) = &mut *frame.tables[TableId::Clock.index()]
    else {
        unreachable!()
    };
    rows[0].debug_current_ref_id = Some(counters.debug_current_ref_id);
    rows[0].unit = Some(counters.unit);
    rows[0].order_index = Some(counters.order_index);
    rows[0].front_index = Some(counters.front_index);
    rows[0].theatre_index = Some(counters.theatre_index);
    rows[0].theater_group_index = Some(counters.theater_group_index);
    rows[0].military_deployment_line_index = Some(counters.military_deployment_line_index);
    rows[0].military_deployment_conveyor_index = Some(counters.military_deployment_conveyor_index);
    rows[0].equipment_variant_index = Some(counters.equipment_variant_index);
    rows[0].country_leader_index = Some(counters.country_leader_index);
    rows[0].navy_id = Some(counters.navy_id);
    rows[0].land_combat_id = Some(counters.land_combat_id);
    rows[0].multiplayer_random_count = Some(counters.multiplayer_random_count);
    let report = compare_at(&frame, &save, &membership, &equipment, after).unwrap();
    let verdict = report.boundary_verdict.clone().unwrap();
    assert!(
        verdict.contains("all 13 recorded counters equal"),
        "{verdict}"
    );
    assert!(verdict.contains("does not prove"), "{verdict}");
    assert!(
        verdict.contains("same-hour writes and untracked allocations"),
        "{verdict}"
    );
    assert_eq!(report.boundary_witness.len(), 13);
    assert!(differences(&report).is_empty());
    assert_eq!(
        report.compared_tables["boundary counter witnesses (printed, never a difference)"],
        13
    );

    // Three objects allocated after the sample; the witness names them and
    // the comparison still passes, because it never waives a difference.
    let mut frame = frame.clone();
    let TableView::Complete(TableRows::Clock(rows)) = &mut *frame.tables[TableId::Clock.index()]
    else {
        unreachable!()
    };
    rows[0].debug_current_ref_id = Some(counters.debug_current_ref_id - 3);
    rows[0].navy_id = Some(counters.navy_id - 1);
    rows[0].multiplayer_random_count = None;
    let report = compare_at(&frame, &save, &membership, &equipment, after).unwrap();
    let verdict = report.boundary_verdict.clone().unwrap();
    assert!(verdict.contains("after further engine work"), "{verdict}");
    assert!(verdict.contains("debug_current_ref_id "), "{verdict}");
    // The same gap at the end-of-hour boundary cannot follow the sample.
    let end = compare(&frame, &save, &membership, &equipment).unwrap();
    let end_verdict = end.boundary_verdict.clone().unwrap();
    assert!(
        end_verdict.contains("higher in the save than at the end-of-hour sample"),
        "{end_verdict}"
    );
    assert!(differences(&end).is_empty());
    assert!(verdict.contains("navy_id "), "{verdict}");
    assert_eq!(
        report.compared_tables["boundary counter witnesses (printed, never a difference)"],
        12
    );
    assert!(
        report
            .boundary_witness
            .iter()
            .any(|line| line.contains("multiplayer_random_count: recorded unavailable"))
    );
    assert!(differences(&report).is_empty());

    // Counters that are not recorded at all leave the boundary unmeasured.
    let mut frame = frame.clone();
    let TableView::Complete(TableRows::Clock(rows)) = &mut *frame.tables[TableId::Clock.index()]
    else {
        unreachable!()
    };
    rows[0] = ClockRow {
        hour: rows[0].hour,
        seed: rows[0].seed,
        ..Default::default()
    };
    let report = compare_at(&frame, &save, &membership, &equipment, after).unwrap();
    assert!(
        report
            .boundary_verdict
            .clone()
            .unwrap()
            .contains("not measured"),
    );
    assert!(differences(&report).is_empty());
}

#[test]
fn strategic_action_oracle_names_unsaved_operands_and_keeps_saved_differences() {
    let (mut frame, mut saved, _, _) = division_fixture();
    let id = *saved.divisions.keys().next().unwrap();
    let presentation = &mut saved.divisions.get_mut(&id).unwrap().presentation;
    presentation.path = vec![9, 8];
    presentation.support_attack = None;
    presentation.full_path.clear();
    presentation.previous = Some(7);
    presentation.movement_progress = None;
    presentation.strategic_redeployment = true;
    let row = replay::rows::DivisionMovementRow {
        division: id,
        source: 7,
        path_total: 2,
        path: vec![9, 8],
        strategic_redeployment: 1,
        strategic_action: replay::rows::Recorded::Value(replay::rows::StrategicAction::Withheld(
            replay::rows::StrategicActionFailure::CountryModifier,
        )),
        withdrawal: Recorded::Value(Some(0)),
        movement_priority: Recorded::Value(Some(1)),
        base_speed: Recorded::Value(Some(400_000)),
        // A nonempty route records its fuel; the save's absent fuel is zero.
        fuel: Recorded::Value(Some(0)),
        ..Default::default()
    };
    *frame.tables[TableId::DivisionMovement.index()] =
        TableView::Complete(TableRows::DivisionMovement(vec![row]));
    let mut report = Report::default();
    compare_division_movement(&frame, &saved, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.scalar_checks, 9);
    assert!(report.limitations.iter().any(|(reason, subjects)| {
        reason.contains("strategic-action")
            && subjects
                .iter()
                .any(|subject| subject == "DivisionMovement.strategic_action")
    }));
    let mut mismatched = saved.clone();
    let presentation = &mut mismatched.divisions.get_mut(&id).unwrap().presentation;
    presentation.path.reverse();
    presentation.support_attack = Some(3);
    presentation.strategic_redeployment = false;
    let mut report = Report::default();
    compare_division_movement(&frame, &mismatched, &mut report).unwrap();
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("path/total"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("target"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("strategic_redeployment"))
    );
}
