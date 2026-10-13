use super::*;
use replay::{
    frame::TableView,
    rows::{TableId, TableRows, WarNameRuleRow, WarOverviewCountryRow, WarOwnedRelationRow},
};

fn two_country_fixture() -> (DecodedFrame, Save) {
    let (mut frame, mut save, _, _) = super::super::tests::fixture();
    let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
    let mut second = countries[0].clone();
    second.id = 103;
    second.canonical_slot = 8;
    second.tag = b"TWO".to_vec();
    countries.push(second);
    frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries)).into();
    let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
    links.push(replay::rows::CountryLinkRow {
        id: 103,
        canonical_slot: 8,
    });
    frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(links)).into();
    save.countries.insert(
        b"TWO".to_vec(),
        installed_content::save::Country {
            tag: b"TWO".to_vec(),
            ..Default::default()
        },
    );
    (frame, save)
}

#[test]
fn ordered_repeated_goals_aliases_and_missing_objects_compare_without_normalizing() {
    let (mut frame, mut save, _, _) = super::super::tests::fixture();
    let handle = (3_u64 << 32) | 12_345;
    save.war_relations.push(installed_content::save::WarRelation {
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
    save.war_relations.push(installed_content::save::WarRelation {
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
        ..retired.clone()
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
                ..retired.clone()
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
    // A redeclaration has the same endpoints but a different first goal.
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
    save.war_relations.push(installed_content::save::WarRelation {
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
