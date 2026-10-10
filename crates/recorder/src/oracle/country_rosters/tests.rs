use super::*;
use installed_content::save::{characters, country_rosters};
use replay::{
    frame::TableView,
    rows::{OwnedWargoalRow, Recorded, TableId, TableRows},
};

fn set(frame: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    *frame.tables[index] = TableView::Complete(rows);
}

fn saved_goals(input: &[u8]) -> country_rosters::Pack {
    country_rosters::parse(&installed_content::parser::parse(input).unwrap().items).unwrap()
}

fn owned_fixture() -> (DecodedFrame, Save) {
    let (mut frame, mut save, _, _) = super::super::tests::fixture();
    save.country_rosters = saved_goals(
        br"countries={GER={diplomacy={
            wargoals={
            mod_goal={id={id=1 type=4713} wargoaldata_actor=GER
                wargoaldata_recipient=GER type=mod_goal states={9 3 9}}
            empty_goal={id={id=2 type=4713} wargoaldata_actor=GER
                wargoaldata_recipient=GER type=empty_goal}
            }
            available_wargoals={wargoal={id=1 type=4713} wargoal={id=2 type=4713}}
        }}}",
    );
    set(
        &mut frame,
        TableRows::OwnedWargoals(vec![
            OwnedWargoalRow {
                country: 101,
                ordinal: 0,
                recipient: 102,
                goal_type: 5,
                states: vec![9, 3, 9],
            },
            OwnedWargoalRow {
                country: 101,
                ordinal: 1,
                recipient: 101,
                goal_type: 6,
                states: Vec::new(),
            },
        ]),
    );
    (frame, save)
}

fn owned_report(frame: &DecodedFrame, save: &Save) -> io::Result<Report> {
    let context = super::super::country_context(frame)?;
    let tokens = [(5, b"mod_goal".as_slice()), (6, b"empty_goal".as_slice())].into();
    let mut report = Report::default();
    compare_owned(frame, save, &context.links, &tokens, &mut report)?;
    Ok(report)
}

#[test]
fn owned_wargoals_compare_native_roster_order_type_and_claimed_states() {
    let (mut frame, mut save) = owned_fixture();
    let report = owned_report(&frame, &save).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["OwnedWargoals"], 2);
    assert_eq!(report.scalar_checks, 7);
    save.country_rosters
        .goals
        .get_mut(&((1 << 32) | 4713))
        .unwrap()
        .recipient = b"different_saved_country".to_vec();
    let report = owned_report(&frame, &save).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("recipient"));
    assert_eq!(report.scalar_checks, 7);
    save.country_rosters
        .goals
        .get_mut(&((1 << 32) | 4713))
        .unwrap()
        .recipient = b"GER".to_vec();
    save.country_rosters
        .goals
        .get_mut(&((1 << 32) | 4713))
        .unwrap()
        .states
        .swap(0, 1);
    let report = owned_report(&frame, &save).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("states"));
    assert_eq!(report.scalar_checks, 7);
    save.country_rosters
        .goals
        .get_mut(&((1 << 32) | 4713))
        .unwrap()
        .states
        .swap(0, 1);
    save.country_rosters
        .goals
        .get_mut(&((1 << 32) | 4713))
        .unwrap()
        .goal_type = b"different_saved_goal_type".to_vec();
    let report = owned_report(&frame, &save).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("goal_type"));
    assert_eq!(report.scalar_checks, 7);
    save.country_rosters
        .goals
        .get_mut(&((1 << 32) | 4713))
        .unwrap()
        .goal_type = b"mod_goal".to_vec();
    save.country_rosters
        .owned_goals
        .get_mut(b"GER".as_slice())
        .unwrap()
        .swap(0, 1);
    let report = owned_report(&frame, &save).unwrap();
    assert!(report.differences.iter().any(|d| d.contains("goal_type")));
    set(&mut frame, TableRows::OwnedWargoals(Vec::new()));
    let report = owned_report(&frame, &save).unwrap();
    assert!(report.differences.iter().any(|d| d.contains("count")));
}

#[test]
fn owned_wargoals_distinguish_observed_empty_missing_and_unserializable_inputs() {
    let (mut frame, mut save) = owned_fixture();
    set(&mut frame, TableRows::OwnedWargoals(Vec::new()));
    save.country_rosters = saved_goals(b"countries={GER={}}");
    let report = owned_report(&frame, &save).unwrap();
    assert!(report.differences.is_empty());
    *frame.tables[TableId::OwnedWargoals.index()] = TableView::Unavailable {
        reason: "owned-goal header unreadable".into(),
    };
    assert!(owned_report(&frame, &save).is_err());

    let (mut frame, mut save) = owned_fixture();
    save.country_rosters.goals.remove(&((2 << 32) | 4713));
    set(
        &mut frame,
        TableRows::OwnedWargoals(vec![
            OwnedWargoalRow {
                country: 101,
                ordinal: 0,
                recipient: 0,
                goal_type: -1,
                states: vec![9, 0, 3, 9],
            },
            OwnedWargoalRow {
                country: 101,
                ordinal: 1,
                ..Default::default()
            },
        ]),
    );
    let report = owned_report(&frame, &save).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    for reason in [
        "null database tag",
        "lexer literal",
        "null membership",
        "no serialized",
    ] {
        assert!(report.limitations.keys().any(|key| key.contains(reason)));
    }
    save.country_rosters
        .owned_goals
        .get_mut(b"GER".as_slice())
        .unwrap()[1] = 0;
    let report = owned_report(&frame, &save).unwrap();
    assert!(
        report
            .limitations
            .keys()
            .any(|key| key.contains("slot is a null"))
    );
    assert!(
        !report
            .limitations
            .keys()
            .any(|key| key.contains("no serialized"))
    );
}

#[test]
fn owned_wargoals_refuse_duplicate_and_noncontiguous_ordinals() {
    let (mut frame, save) = owned_fixture();
    for rows in [
        vec![OwnedWargoalRow {
            country: 101,
            ordinal: 1,
            ..Default::default()
        }],
        vec![
            OwnedWargoalRow {
                country: 101,
                ..Default::default()
            },
            OwnedWargoalRow {
                country: 102,
                ..Default::default()
            },
        ],
    ] {
        set(&mut frame, TableRows::OwnedWargoals(rows));
        assert!(owned_report(&frame, &save).is_err());
    }
}

#[test]
fn owned_wargoals_compare_five_thousand_country_payloads() {
    let (mut frame, mut save, _, _) = super::super::tests::fixture();
    let country = complete::<tables::Countries>(&frame).unwrap()[0].clone();
    let mut countries = Vec::new();
    let mut goals = Vec::new();
    save.country_rosters = country_rosters::Pack::default();
    for id in 1..=5000_u32 {
        let tag = format!("C{id}").into_bytes();
        let mut country = country.clone();
        country.id = id;
        country.tag = tag.clone();
        countries.push(country);
        let handle = (u64::from(id) << 32) | 4713;
        save.country_rosters
            .owned_goals
            .insert(tag.clone(), vec![handle]);
        save.country_rosters.goals.insert(
            handle,
            country_rosters::Goal {
                recipient: tag,
                goal_type: b"mod_goal".to_vec(),
                states: vec![id],
            },
        );
        goals.push(OwnedWargoalRow {
            country: id,
            ordinal: 0,
            recipient: id,
            goal_type: 5,
            states: vec![id],
        });
    }
    set(&mut frame, TableRows::OwnedWargoals(goals.clone()));
    let links = countries
        .iter()
        .map(|country| (country.id, country))
        .collect();
    let tokens = [(5, b"mod_goal".as_slice())].into();
    let mut report = Report::default();
    compare_owned(&frame, &save, &links, &tokens, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.scalar_checks, 20_000);
    assert_eq!(report.compared_tables["OwnedWargoals"], 5000);
    goals[4999].states = vec![1];
    set(&mut frame, TableRows::OwnedWargoals(goals));
    let mut report = Report::default();
    compare_owned(&frame, &save, &links, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
}

fn saved_roles() -> characters::Pack {
    characters::parse(
        &installed_content::parser::parse(
            br"character_manager={historical={
                character={id={id=7 type=73} token=general country=GER
                    corps_commander={id={id=17 type=4713} skill=1}}
                character={id={id=8 type=73} token=admiral country=GER
                    navy_leader={id={id=18 type=4713} skill=1}}
                character={id={id=9 type=73} token=operative country=GER
                    operative={id=19 type=4713}}
            }} countries={GER={}}",
        )
        .unwrap()
        .items,
    )
    .unwrap()
}

#[test]
fn directed_cache_and_zero_war_limits_distinguish_history_refusal_and_runtime_values() {
    let (_, mut save, _, _) = super::super::tests::fixture();
    // New runtime inputs must be reported before a missing saved-country join
    // skips the ordinary roster comparisons.
    save.countries.remove(b"GER".as_slice());
    let tags = [(101, b"GER".as_slice())].into();
    for (co_belligerents, zero_war, expected) in [
        (Recorded::NotRecorded, Recorded::NotRecorded, "not recorded"),
        (
            Recorded::Value(None),
            Recorded::Value(None),
            "unavailable at capture",
        ),
        (
            Recorded::Value(Some(vec![0, 2])),
            Recorded::Value(Some([1, 0x7f])),
            "runtime-only",
        ),
        (
            Recorded::Value(Some(Vec::new())),
            Recorded::Value(Some([0, 0])),
            "runtime-only",
        ),
    ] {
        let mut report = Report::default();
        compare_lists(
            &[CountryCounterInputRow {
                forward_visibility: replay::rows::Recorded::NotRecorded,
                country: 101,
                co_belligerents,
                zero_war,
                ..Default::default()
            }],
            &save,
            &tags,
            &mut report,
        )
        .unwrap();
        for field in ["co_belligerents", "zero_war"] {
            if expected == "not recorded" {
                // Historical field identity is in the reason; the subject names the row.
                assert_eq!(
                    report.limitations[&format!("CountryCounterInputs.{field} not recorded")],
                    vec!["CountryCounterInputs GER".to_string()],
                );
                continue;
            }
            assert!(
                report.limitations.iter().any(|(reason, labels)| {
                    reason.contains(expected) && labels.iter().any(|label| label.contains(field))
                }),
                "{:?}",
                report.limitations
            );
        }
        assert_eq!(report.differences.len(), 1);
        assert_eq!(report.scalar_checks, 0);
    }
}

#[test]
fn country_rosters_compare_sets_preserve_absence_and_name_null_limitations() {
    let (_, mut save, _, _) = super::super::tests::fixture();
    save.countries.get_mut(b"GER".as_slice()).unwrap().cores = vec![7, 3];
    save.characters = saved_roles();
    save.characters.character_status = characters::parse(
        &installed_content::parser::parse(
            br"countries={GER={characters={
            character_status={character={id=8 type=73} unit_leader=yes}
            character_status={character={id=7 type=73} unit_leader=yes}
            character_status={character={id=9 type=73} unit_leader=yes}
        }}}",
        )
        .unwrap()
        .items,
    )
    .unwrap()
    .character_status;
    let row = CountryCounterInputRow {
        forward_visibility: replay::rows::Recorded::NotRecorded,
        country: 101,
        owned_states: Some(vec![1, 1, 0]).into(),
        controlled_states: Some(vec![1]).into(),
        core_states: Some(vec![7, 3]).into(),
        army_leaders: Some(vec![(17 << 32) | 4713, 0]).into(),
        navy_leaders: Some(vec![(18 << 32) | 4713]).into(),
        ..Default::default()
    };
    let tags = [(101, b"GER".as_slice())].into();
    let mut report = Report::default();
    compare_lists(std::slice::from_ref(&row), &save, &tags, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(report.limitations.keys().any(|key| key.contains("null")));

    let missing = CountryCounterInputRow {
        forward_visibility: replay::rows::Recorded::NotRecorded,
        country: 101,
        ..Default::default()
    };
    let mut report = Report::default();
    compare_lists(&[missing], &save, &tags, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|key| key.contains("unavailable at capture"))
    );

    let changed = CountryCounterInputRow {
        controlled_states: Some(Vec::new()).into(),
        core_states: Some(vec![7]).into(),
        army_leaders: Some(vec![(18 << 32) | 4713]).into(),
        ..row
    };
    let mut report = Report::default();
    compare_lists(&[changed], &save, &tags, &mut report).unwrap();
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
}

#[test]
fn core_rosters_compare_saved_order_and_duplicate_multiplicity() {
    let (_, mut save, _, _) = super::super::tests::fixture();
    save.countries.get_mut(b"GER".as_slice()).unwrap().cores = vec![7, 3, 7];
    let tags = [(101, b"GER".as_slice())].into();
    for (states, differences) in [(vec![7, 3, 7], 0), (vec![3, 7, 7], 1), (vec![7, 3], 1)] {
        let mut report = Report::default();
        compare_lists(
            &[CountryCounterInputRow {
                forward_visibility: replay::rows::Recorded::NotRecorded,
                country: 101,
                core_states: Some(states).into(),
                ..Default::default()
            }],
            &save,
            &tags,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), differences);
    }
}

#[test]
fn country_rosters_distinguish_history_capture_failure_and_native_empty() {
    let (_, mut save, _, _) = super::super::tests::fixture();
    save.countries
        .get_mut(b"GER".as_slice())
        .unwrap()
        .cores
        .clear();
    let tags = [(101, b"GER".as_slice())].into();
    let historical = CountryCounterInputRow {
        forward_visibility: replay::rows::Recorded::NotRecorded,
        country: 101,
        owned_states: Recorded::NotRecorded,
        controlled_states: Recorded::NotRecorded,
        core_states: Recorded::NotRecorded,
        army_leaders: Recorded::NotRecorded,
        navy_leaders: Recorded::NotRecorded,
        ..Default::default()
    };
    let mut report = Report::default();
    compare_lists(&[historical], &save, &tags, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.scalar_checks, 0);
    for name in [
        "owned_states",
        "controlled_states",
        "core_states",
        "army_leaders",
        "navy_leaders",
    ] {
        assert_eq!(
            report.limitations[&format!("CountryCounterInputs.{name} not recorded")].len(),
            1,
        );
    }
    for (core_states, checks) in [(None, 0), (Some(Vec::new()), 1)] {
        let mut report = Report::default();
        compare_lists(
            &[CountryCounterInputRow {
                forward_visibility: replay::rows::Recorded::NotRecorded,
                country: 101,
                core_states: core_states.into(),
                ..Default::default()
            }],
            &save,
            &tags,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.scalar_checks, checks);
        assert!(
            !report
                .limitations
                .keys()
                .any(|key| key.contains("not recorded"))
        );
        assert!(
            report.limitations["country roster operand was unavailable at capture"]
                .iter()
                .any(|subject| subject.ends_with(".owned_states"))
        );
    }
}

#[test]
fn country_rosters_report_missing_saved_countries_and_refuse_duplicate_roles() {
    let (_, mut save, _, _) = super::super::tests::fixture();
    let tags = [(101, b"GER".as_slice())].into();
    let rows = [CountryCounterInputRow {
        forward_visibility: replay::rows::Recorded::NotRecorded,
        country: 101,
        ..Default::default()
    }];
    save.characters = saved_roles();
    let manager = save.characters.manager.as_mut().unwrap();
    let mut duplicate = manager.unit_leaders.values().next().unwrap().clone();
    duplicate.id = (27 << 32) | 4713;
    manager.unit_leaders.insert(duplicate.id, duplicate);
    let mut report = Report::default();
    let error = compare_lists(&rows, &save, &tags, &mut report).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("duplicate saved military leader roles")
    );

    save.characters = saved_roles();
    save.countries.clear();
    let mut report = Report::default();
    compare_lists(&rows, &save, &tags, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("absent from save"));
}

#[test]
fn country_rosters_compare_five_thousand_countries_without_owner_role_substitution() {
    let (_, mut save, _, _) = super::super::tests::fixture();
    let country = save.countries.values().next().unwrap().clone();
    let state = save.states.values().next().unwrap().clone();
    let leader = saved_roles()
        .manager
        .unwrap()
        .unit_leaders
        .into_values()
        .next()
        .unwrap();
    save.countries.clear();
    save.states.clear();
    save.country_rosters = country_rosters::Pack::default();
    save.characters.manager = Some(characters::Manager::default());
    let mut rows = Vec::new();
    let mut tags = BTreeMap::new();
    for id in 1..=5000_u32 {
        let tag = format!("C{id}").into_bytes();
        let mut country = country.clone();
        country.tag = tag.clone();
        country.cores = vec![id];
        save.countries.insert(tag.clone(), country);
        let mut state = state.clone();
        state.id = id;
        state.owner = Some(tag.clone());
        state.controller = None;
        save.states.insert(id, state);
        let character = (u64::from(id) << 32) | 73;
        let reference = (u64::from(id) << 32) | 4713;
        let mut leader = leader.clone();
        leader.character = character;
        leader.id = reference;
        save.characters
            .manager
            .as_mut()
            .unwrap()
            .unit_leaders
            .insert(reference, leader);
        save.characters.character_status.insert(
            tag.clone(),
            vec![characters::CharacterStatus {
                character,
                advisor: None,
                unit_leader: Some(true),
            }],
        );
        tags.insert(id, tag);
        rows.push(CountryCounterInputRow {
            forward_visibility: replay::rows::Recorded::NotRecorded,
            country: id,
            owned_states: Some(vec![id]).into(),
            controlled_states: Some(vec![id]).into(),
            core_states: Some(vec![id]).into(),
            army_leaders: Some(vec![reference]).into(),
            navy_leaders: Some(Vec::new()).into(),
            ..Default::default()
        });
    }
    let tags = tags.iter().map(|(&id, tag)| (id, tag.as_slice())).collect();
    let mut report = Report::default();
    compare_lists(&rows, &save, &tags, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.scalar_checks, 25_000);
    rows[4999].navy_leaders = Some(vec![(5000 << 32) | 4713]).into();
    let mut report = Report::default();
    compare_lists(&rows, &save, &tags, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
}
