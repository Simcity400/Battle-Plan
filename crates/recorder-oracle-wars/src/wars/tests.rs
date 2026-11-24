use super::*;
use replay_data::{
    rows::TableId, rows::TableRows, rows::WarNameRuleRow, rows::WarOverviewCountryRow,
    rows::WarOwnedRelationRow,
};
use replay_frame::frame::TableView;

fn two_country_fixture() -> (DecodedFrame, Save) {
    let (mut frame, mut save, _, _) = super::super::fixtures::fixture();
    let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
    let mut second = countries[0].clone();
    second.id = 103;
    second.canonical_slot = 8;
    second.tag = b"TWO".to_vec();
    countries.push(second);
    frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries)).into();
    let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
    links.push(replay_data::rows::CountryLinkRow {
        id: 103,
        canonical_slot: 8,
    });
    frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(links)).into();
    save.countries.insert(
        b"TWO".to_vec(),
        content_reader_save::Country {
            tag: b"TWO".to_vec(),
            ..Default::default()
        },
    );
    (frame, save)
}

#[test]
fn participation_oracle_compares_every_saved_component_membership_and_binary_omission_class() {
    let (mut frame, mut save) = two_country_fixture();
    let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
    let mut third = countries[0].clone();
    third.id = 104;
    third.canonical_slot = 9;
    third.tag = b"THREE".to_vec();
    countries.push(third);
    frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries)).into();
    let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
    links.push(replay_data::rows::CountryLinkRow {
        id: 104,
        canonical_slot: 9,
    });
    frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(links)).into();
    let owned = [
        (101, 101, 104),
        (104, 101, 104),
        (103, 103, 104),
        (104, 103, 104),
    ]
    .into_iter()
    .enumerate()
    .map(
        |(ordinal, (owner, first_country, second_country))| WarOwnedRelationRow {
            owner,
            ordinal: ordinal as u32,
            first_country,
            second_country,
            ..Default::default()
        },
    )
    .collect();
    frame.tables[TableId::WarOwnedRelations.index()] =
        TableView::Complete(TableRows::WarOwnedRelations(owned)).into();
    let ledger = content_reader_save::war_participation::Ledger {
        first: b"GER".to_vec(),
        second: b"THREE".to_vec(),
        equipment_damage: 100_001,
        province_capture: -1,
        air_damage: 200_000,
        strategic_bombing: 300_000,
        sunk_ships: 400_000,
        sunk_convoys: 500_000,
        casualties: 600_000,
        lend_lease_sent: 700_000,
        lend_lease_received: -800_000,
        captured_provinces: vec![-7, 5000],
    };
    save.war_relations.push(content_reader_save::WarRelation {
        participation: [Some(ledger), None],
        roster_owner: b"GER".to_vec(),
        target: b"THREE".to_vec(),
        first: b"GER".to_vec(),
        second: b"THREE".to_vec(),
        first_goals: Vec::new(),
        second_goals: Vec::new(),
        start_date: Some(save.date),
        end_date: None,
        cancel: false,
        first_casualties: 123,
        second_casualties: 456,
        first_unknown_casualties: 77,
        second_unknown_casualties: 88,
        first_was_instigator: true,
    });
    let row = replay_data::rows::WarParticipationRow {
        war_first: 101,
        war_second: 104,
        direction: 0,
        first_country: 101,
        second_country: 104,
        equipment_damage: 100_001,
        province_capture: -1,
        air_damage: 200_000,
        strategic_bombing: 300_000,
        sunk_ships: 400_000,
        sunk_convoys: 500_000,
        casualties: 600_000,
        unknown_casualties: 999,
        lend_lease_sent: 700_000,
        lend_lease_received: -800_000,
    };
    frame.tables[TableId::WarParticipation.index()] =
        TableView::Complete(TableRows::WarParticipation(vec![row.clone()])).into();
    frame.tables[TableId::WarCapturedProvinces.index()] =
        TableView::Complete(TableRows::WarCapturedProvinces(
            [-7, 5000]
                .into_iter()
                .map(|province| replay_data::rows::WarCapturedProvinceRow {
                    war_first: 101,
                    war_second: 104,
                    direction: 0,
                    province,
                    credited: 1,
                })
                .collect(),
        ))
        .into();
    frame.tables[TableId::WarLendLeaseHistories.index()] =
        TableView::Complete(TableRows::WarLendLeaseHistories(vec![
            replay_data::rows::WarLendLeaseHistoryRow {
                country: 101,
                target: 103,
                industry_sent: 3,
                ..Default::default()
            },
        ]))
        .into();
    let check = |frame: &DecodedFrame| {
        let countries = super::super::country_context(frame).unwrap();
        let mut report = Report::default();
        participation::compare_participation(frame, &save, &countries, &mut report).unwrap();
        report
    };
    assert!(check(&frame).difference_lines().is_empty());
    frame.tables[TableId::WarOwnedRelations.index()] =
        TableView::Complete(TableRows::WarOwnedRelations(vec![
            WarOwnedRelationRow {
                owner: 104,
                first_country: 101,
                second_country: 104,
                ..Default::default()
            },
            WarOwnedRelationRow {
                owner: 104,
                ordinal: 1,
                first_country: 103,
                second_country: 104,
                retired: 1,
                ..Default::default()
            },
        ]))
        .into();
    assert!(check(&frame).difference_lines().is_empty());
    let changed = replay_data::rows::WarParticipationRow {
        casualties: 123,
        ..row
    };
    frame.tables[TableId::WarParticipation.index()] =
        TableView::Complete(TableRows::WarParticipation(vec![changed])).into();
    assert!(
        check(&frame)
            .difference_lines()
            .iter()
            .any(|line| line.contains("casualties"))
    );
    frame.tables[TableId::WarLendLeaseHistories.index()] =
        TableView::Complete(TableRows::WarLendLeaseHistories(vec![
            replay_data::rows::WarLendLeaseHistoryRow {
                country: 101,
                target: 103,
                industry_sent: 4,
                ..Default::default()
            },
        ]))
        .into();
    assert!(
        check(&frame)
            .difference_lines()
            .iter()
            .any(|line| line.contains("omitted binary-zero class"))
    );
}

#[test]
fn participation_oracle_compares_native_history_rows_outside_the_displayed_same_side() {
    let (mut frame, mut save) = two_country_fixture();
    for table in [TableId::WarParticipation, TableId::WarCapturedProvinces] {
        frame.tables[table.index()] = TableView::Complete(TableRows::empty(table)).into();
    }
    frame.tables[TableId::WarOwnedRelations.index()] =
        TableView::Complete(TableRows::WarOwnedRelations(vec![WarOwnedRelationRow {
            owner: 101,
            first_country: 101,
            second_country: 103,
            ..Default::default()
        }]))
        .into();
    frame.tables[TableId::WarLendLeaseHistories.index()] =
        TableView::Complete(TableRows::WarLendLeaseHistories(vec![
            replay_data::rows::WarLendLeaseHistoryRow {
                country: 101,
                target: 103,
                industry_sent: 100_000,
                ..Default::default()
            },
        ]))
        .into();
    save.war_delivery_histories
        .push(content_reader_save::war_participation::DeliveryHistory {
            owner: b"GER".to_vec(),
            target: b"TWO".to_vec(),
            industry_sent: 100_000,
            fuel_sent: 0,
            industry_received: 0,
            fuel_received: 0,
        });
    let countries = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    participation::compare_participation(&frame, &save, &countries, &mut report).unwrap();
    assert!(
        report.difference_lines().is_empty(),
        "{:?}",
        report.difference_lines()
    );
    frame.tables[TableId::WarLendLeaseHistories.index()] =
        TableView::Complete(TableRows::WarLendLeaseHistories(Vec::new())).into();
    let countries = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    participation::compare_participation(&frame, &save, &countries, &mut report).unwrap();
    assert!(
        report
            .difference_lines()
            .iter()
            .any(|line| line.contains("certified zero"))
    );
}

#[test]
fn ordered_repeated_goals_aliases_and_missing_objects_compare_without_normalizing() {
    let (mut frame, mut save, _, _) = super::super::fixtures::fixture();
    let handle = (3_u64 << 32) | 12_345;
    save.war_relations.push(content_reader_save::WarRelation {
        participation: [None, None],
        roster_owner: b"GER".to_vec(),
        target: b"GER".to_vec(),
        first: b"GER".to_vec(),
        second: b"GER".to_vec(),
        start_date: Some(save.date),
        end_date: None,
        cancel: false,
        first_was_instigator: true,
        first_casualties: 0,
        second_casualties: 0,
        first_unknown_casualties: 0,
        second_unknown_casualties: 0,
        first_goals: vec![handle, 0, handle],
        second_goals: vec![],
    });
    let goal = WarGoal {
        actor: b"GER".to_vec(),
        recipient: b"GER".to_vec(),
        kind: b"star_contest".to_vec(),
        states: vec![9, 7],
        countries: vec![b"GER".to_vec()],
        puppets: vec![],
        expires: None,
        name: b"star_contest".to_vec(),
    };
    save.war_goals.insert(handle, goal);
    frame.tables[TableId::WarOwnedRelations.index()] =
        TableView::Complete(TableRows::WarOwnedRelations(vec![WarOwnedRelationRow {
            owner: 101,
            first_country: 101,
            second_country: 102,
            first_goal_count: 3,
            ..Default::default()
        }]))
        .into();
    let mut rows = vec![
        WarRelationGoalRow {
            owner: 101,
            ordinal: 0,
            handle,
            resolved: 1,
            type_token: 42,
            actor: 102,
            recipient: 101,
            state_count: 2,
            first_state: 9,
            country_count: 1,
            first_country: 102,
            ..Default::default()
        },
        WarRelationGoalRow {
            owner: 101,
            ordinal: 1,
            ..Default::default()
        },
    ];
    let mut repeat = rows[0].clone();
    repeat.ordinal = 2;
    rows.push(repeat);
    frame.tables[TableId::WarRelationGoals.index()] =
        TableView::Complete(TableRows::WarRelationGoals(rows.clone())).into();
    frame.tables[TableId::WarNameRules.index()] =
        TableView::Complete(TableRows::WarNameRules(vec![WarNameRuleRow {
            viewpoint: 101,
            index: 0,
            prefix: b"NATIVE_PREFIX".to_vec(),
        }]))
        .into();
    frame.tables[TableId::WarOverviewCountries.index()] =
        TableView::Complete(TableRows::WarOverviewCountries(vec![
            WarOverviewCountryRow {
                country: 101,
                major: u8::from(save.countries[b"GER".as_slice()].major),
                ..Default::default()
            },
        ]))
        .into();
    let countries = super::super::country_context(&frame).unwrap();
    let tokens = BTreeMap::from([(42, b"star_contest".as_slice())]);
    let mut report = Report::default();
    compare(&frame, &save, &countries, &tokens, &mut report).unwrap();
    assert!(report.difference_lines().is_empty());
    for reason in LIMITATIONS {
        assert!(report.limitations.contains_key(*reason));
    }
    save.war_relations[0].first_goals.swap(0, 1);
    let mut changed = Report::default();
    compare(&frame, &save, &countries, &tokens, &mut changed).unwrap();
    assert!(
        changed
            .difference_lines()
            .iter()
            .any(|line| line.contains("ordered handles"))
    );
    rows[0].first_state = 7;
    rows[0].recipient = 99;
    assert!(
        compare_goal(
            &rows[0],
            save.war_goals.get(&handle),
            &countries,
            &tokens,
            "test",
            &mut Report::default()
        )
        .is_err()
    );
    rows[0].recipient = 101;
    let mut changed = Report::default();
    compare_goal(
        &rows[0],
        save.war_goals.get(&handle),
        &countries,
        &tokens,
        "test",
        &mut changed,
    )
    .unwrap();
    assert!(
        changed
            .difference_lines()
            .iter()
            .any(|line| line.contains("first_state"))
    );
    compare_goal(
        &rows[0],
        None,
        &countries,
        &tokens,
        "missing object",
        &mut changed,
    )
    .unwrap();
    assert!(
        changed
            .difference_lines()
            .iter()
            .any(|line| line.contains("resolved"))
    );
}

#[test]
fn redeclaration_does_not_compare_retired_goals_with_live_goals_and_checks_reverse_coverage() {
    let (mut frame, mut save) = two_country_fixture();
    save.war_relations.push(content_reader_save::WarRelation {
        participation: [None, None],
        roster_owner: b"GER".to_vec(),
        target: b"TWO".to_vec(),
        first: b"GER".to_vec(),
        second: b"TWO".to_vec(),
        start_date: Some(save.date),
        end_date: None,
        cancel: false,
        first_was_instigator: true,
        first_casualties: 0,
        second_casualties: 0,
        first_unknown_casualties: 0,
        second_unknown_casualties: 0,
        first_goals: vec![22],
        second_goals: vec![],
    });
    let retired = WarOwnedRelationRow {
        owner: 101,
        first_country: 101,
        second_country: 103,
        retired: 1,
        first_goal_count: 1,
        ..Default::default()
    };
    let live = WarOwnedRelationRow {
        ordinal: 1,
        retired: 0,
        ..retired
    };
    let set_owned = |frame: &mut DecodedFrame, rows| {
        frame.tables[TableId::WarOwnedRelations.index()] =
            TableView::Complete(TableRows::WarOwnedRelations(rows)).into();
    };
    set_owned(
        &mut frame,
        vec![
            retired.clone(),
            live.clone(),
            WarOwnedRelationRow {
                owner: 103,
                ..retired
            },
            WarOwnedRelationRow { owner: 103, ..live },
        ],
    );
    let goals = vec![
        WarRelationGoalRow {
            owner: 101,
            handle: 11,
            ..Default::default()
        },
        WarRelationGoalRow {
            owner: 101,
            relation_ordinal: 1,
            handle: 22,
            ..Default::default()
        },
    ];
    let mut goals = goals;
    let other: Vec<_> = goals
        .iter()
        .cloned()
        .map(|mut r| {
            r.owner = 103;
            r
        })
        .collect();
    goals.extend(other);
    frame.tables[TableId::WarRelationGoals.index()] =
        TableView::Complete(TableRows::WarRelationGoals(goals)).into();
    frame.tables[TableId::WarOverviewCountries.index()] =
        TableView::Complete(TableRows::WarOverviewCountries(vec![
            WarOverviewCountryRow {
                country: 101,
                major: u8::from(save.countries[b"GER".as_slice()].major),
                ..Default::default()
            },
            WarOverviewCountryRow {
                country: 103,
                ..Default::default()
            },
        ]))
        .into();
    frame.tables[TableId::WarNameRules.index()] =
        TableView::Complete(TableRows::WarNameRules(vec![WarNameRuleRow {
            viewpoint: 101,
            index: 0,
            prefix: b"PREFIX".to_vec(),
        }]))
        .into();
    let check = |frame: &DecodedFrame| {
        let countries = super::super::country_context(frame).unwrap();
        let mut report = Report::default();
        compare(frame, &save, &countries, &BTreeMap::new(), &mut report).unwrap();
        report
    };
    let report = check(&frame);
    assert!(
        report.difference_lines().is_empty(),
        "{:?}",
        report.difference_lines()
    );
    assert_eq!(report.limitations[ENDED_WAR_LIMITATION].len(), 2);
    frame.tables[TableId::WarOverviewCountries.index()] =
        TableView::Complete(TableRows::WarOverviewCountries(vec![])).into();
    assert!(
        check(&frame)
            .difference_lines()
            .iter()
            .any(|s| s.contains("owner 101 coverage"))
    );
    set_owned(&mut frame, vec![retired]);
    frame.tables[TableId::WarRelationGoals.index()] =
        TableView::Complete(TableRows::WarRelationGoals(vec![WarRelationGoalRow {
            owner: 101,
            handle: 11,
            ..Default::default()
        }]))
        .into();
    assert!(
        check(&frame)
            .difference_lines()
            .iter()
            .any(|s| s.contains("live occurrence coverage"))
    );
}

#[test]
fn each_saved_relation_requires_both_live_endpoint_occurrences() {
    let (mut frame, mut save) = two_country_fixture();
    save.war_relations.push(content_reader_save::WarRelation {
        participation: [None, None],
        roster_owner: b"GER".to_vec(),
        target: b"TWO".to_vec(),
        first: b"GER".to_vec(),
        second: b"TWO".to_vec(),
        start_date: Some(save.date),
        end_date: None,
        cancel: false,
        first_was_instigator: true,
        first_casualties: 0,
        second_casualties: 0,
        first_unknown_casualties: 0,
        second_unknown_casualties: 0,
        first_goals: vec![],
        second_goals: vec![],
    });
    frame.tables[TableId::WarNameRules.index()] =
        TableView::Complete(TableRows::WarNameRules(vec![WarNameRuleRow {
            viewpoint: 101,
            index: 0,
            prefix: b"PREFIX".to_vec(),
        }]))
        .into();
    let rows = vec![
        WarOwnedRelationRow {
            owner: 101,
            first_country: 102,
            second_country: 103,
            ..Default::default()
        },
        WarOwnedRelationRow {
            owner: 103,
            first_country: 101,
            second_country: 103,
            ..Default::default()
        },
    ];
    frame.tables[TableId::WarOverviewCountries.index()] =
        TableView::Complete(TableRows::WarOverviewCountries(vec![
            WarOverviewCountryRow {
                country: 101,
                major: u8::from(save.countries[b"GER".as_slice()].major),
                ..Default::default()
            },
            WarOverviewCountryRow {
                country: 103,
                ..Default::default()
            },
        ]))
        .into();
    let check = |frame: &DecodedFrame| {
        let countries = super::super::country_context(frame).unwrap();
        let mut report = Report::default();
        compare(frame, &save, &countries, &BTreeMap::new(), &mut report).unwrap();
        report
    };
    for owner in [101, 103] {
        let missing = rows.iter().filter(|r| r.owner != owner).cloned().collect();
        frame.tables[TableId::WarOwnedRelations.index()] =
            TableView::Complete(TableRows::WarOwnedRelations(missing)).into();
        let report = check(&frame);
        assert_eq!(
            report.differences.len(),
            1,
            "{:?}",
            report.difference_lines()
        );
    }
    frame.tables[TableId::WarOwnedRelations.index()] =
        TableView::Complete(TableRows::WarOwnedRelations(rows)).into();
    assert!(check(&frame).difference_lines().is_empty());
}
