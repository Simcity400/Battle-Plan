use super::*;
use content_reader_save::{characters, country_rosters};
use replay_data::rows::Recorded;

fn saved_roles() -> characters::Pack {
    characters::parse(
        &content_reader_parser::parser::parse(
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
    let (_, mut save, _, _) = super::super::fixtures::fixture();
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
                forward_visibility: replay_data::rows::Recorded::NotRecorded,
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
    let (_, mut save, _, _) = super::super::fixtures::fixture();
    save.countries.get_mut(b"GER".as_slice()).unwrap().cores = vec![7, 3];
    save.characters = saved_roles();
    save.characters.character_status = characters::parse(
        &content_reader_parser::parser::parse(
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
        forward_visibility: replay_data::rows::Recorded::NotRecorded,
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
        forward_visibility: replay_data::rows::Recorded::NotRecorded,
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
    let (_, mut save, _, _) = super::super::fixtures::fixture();
    save.countries.get_mut(b"GER".as_slice()).unwrap().cores = vec![7, 3, 7];
    let tags = [(101, b"GER".as_slice())].into();
    for (states, differences) in [(vec![7, 3, 7], 0), (vec![3, 7, 7], 1), (vec![7, 3], 1)] {
        let mut report = Report::default();
        compare_lists(
            &[CountryCounterInputRow {
                forward_visibility: replay_data::rows::Recorded::NotRecorded,
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
    let (_, mut save, _, _) = super::super::fixtures::fixture();
    save.countries
        .get_mut(b"GER".as_slice())
        .unwrap()
        .cores
        .clear();
    let tags = [(101, b"GER".as_slice())].into();
    let historical = CountryCounterInputRow {
        forward_visibility: replay_data::rows::Recorded::NotRecorded,
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
                forward_visibility: replay_data::rows::Recorded::NotRecorded,
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
    let (_, mut save, _, _) = super::super::fixtures::fixture();
    let tags = [(101, b"GER".as_slice())].into();
    let rows = [CountryCounterInputRow {
        forward_visibility: replay_data::rows::Recorded::NotRecorded,
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
fn country_rosters_compare_every_country_without_owner_role_substitution() {
    let (_, mut save, _, _) = super::super::fixtures::fixture();
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
    for id in 1..=5_u32 {
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
            forward_visibility: replay_data::rows::Recorded::NotRecorded,
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
    assert_eq!(report.scalar_checks, 25);
    rows[4].navy_leaders = Some(vec![(5 << 32) | 4713]).into();
    let mut report = Report::default();
    compare_lists(&rows, &save, &tags, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
}
