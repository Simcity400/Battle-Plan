use super::*;

fn decision() -> Decision {
    let id = b"sample_decision".to_vec();
    Decision {
        seed_component: installed_content::decisions::seed_component(&id),
        name: id.clone(),
        id,
        ..Default::default()
    }
}

#[test]
fn removal_finder_preserves_container_target_alias_and_first_ordinal() {
    let row = |ordinal, container, target, state| DecisionRow {
        country: 100,
        ordinal,
        container,
        token: b"Sample_Decision".to_vec(),
        country_target: target,
        numeric_target: state,
        lifecycle: -8,
        ignore: 1,
        ..Default::default()
    };
    let rows = [
        row(9, 3, 202, 7),
        row(1, 4, 202, 7),
        row(3, 3, 200, 7),
        row(2, 3, 200, 8),
    ];
    let slot = |raw| match raw {
        100 => Some(1),
        200 | 202 => Some(2),
        _ => None,
    };
    assert_eq!(
        matching(&rows, b"sample_decision", 1, false, Some(2), 7, slot)
            .unwrap()
            .unwrap()
            .ordinal,
        3
    );
    assert_eq!(
        matching(&rows, b"sample_decision", 1, true, Some(2), 7, slot)
            .unwrap()
            .unwrap()
            .ordinal,
        1
    );
    assert!(
        matching(&rows, b"sample_decision", 1, false, None, 7, slot)
            .unwrap()
            .is_none()
    );
    assert!(
        matching(&rows, b"different", 1, false, Some(2), 7, slot)
            .unwrap()
            .is_none()
    );
}

#[test]
fn unknown_target_link_is_not_an_absent_decision() {
    let row = DecisionRow {
        country: 100,
        container: 3,
        token: b"sample_decision".to_vec(),
        country_target: 202,
        ..Default::default()
    };
    let error = matching(&[row], b"sample_decision", 1, false, Some(2), 0, |raw| {
        (raw == 100).then_some(1)
    })
    .unwrap_err();
    assert!(error.contains("Decisions target 202"));
}

#[test]
fn decision_seed_distinguishes_unread_absence_and_fixed_random_seed() {
    let mut definition = decision();
    assert!(
        decision_seed(9, &definition, None, 2)
            .unwrap_err()
            .contains("description_seed")
    );
    let absent = decision_seed(9, &definition, Some(None), 2).unwrap();
    assert_eq!(
        absent,
        decision_seed(9, &definition, Some(Some(0)), 2).unwrap()
    );
    assert_eq!(
        decision_seed(9, &definition, Some(Some(13)), 2).unwrap(),
        absent.wrapping_add(13)
    );
    definition.fixed_random_seed = true;
    assert_eq!(decision_seed(9, &definition, None, 2).unwrap(), absent);
    definition.seed_component = i32::MAX;
    assert_eq!(decision_seed(1, &definition, None, 0).unwrap(), 0x8000_0000);
}

#[test]
fn native_decision_name_fallback_ladder() {
    let keys = [
        "COS_Name_group",
        "COS_Name",
        "ABC_Name_group",
        "ABC_Name",
        "Name_group",
        "Name",
    ];
    for first in 0..keys.len() {
        assert_eq!(
            name_key("Name", "COS", "ABC", "group", |key| keys[first..]
                .contains(&key)),
            keys[first]
        );
    }
    assert_eq!(name_key("Name", "", "ABC", "group", |_| false), "Name");
    assert_eq!(
        name_key("Name", "", "ABC", "group", |key| key == "Name_group"),
        "Name_group"
    );
}

use super::super::tests::Fixture;
use replay::{
    frame::TableView,
    rows::{ClockRow, PoliticalPartyRow, PoliticsRow, TableSchema},
};

fn set<T: TableSchema>(fixture: &mut Fixture, rows: Vec<T::Row>) {
    *fixture.frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
}

fn decision_fixture(seed: Option<Option<i32>>) -> (Fixture, installed_content::decisions::Database) {
    let mut fixture = Fixture::new();
    set::<tables::Clock>(&mut fixture, vec![ClockRow::default()]);
    set::<tables::Politics>(
        &mut fixture,
        vec![PoliticsRow {
            country: 100,
            ruling_party: Some(1),
            ..Default::default()
        }],
    );
    set::<tables::PoliticalParties>(
        &mut fixture,
        vec![PoliticalPartyRow {
            country: 100,
            id: 1,
            token: b"group".to_vec(),
            ..Default::default()
        }],
    );
    set::<tables::Decisions>(
        &mut fixture,
        vec![DecisionRow {
            country: 100,
            container: 3,
            token: b"SYN_DECISION".to_vec(),
            country_target: 200,
            description_seed: seed,
            // The description lookup does not test these execution-state fields.
            lifecycle: -1,
            ignore: 1,
            ..Default::default()
        }],
    );
    let mut database = installed_content::decisions::Database::default();
    database.entries.insert(
        b"syn_decision".to_vec(),
        Decision {
            id: b"SYN_DECISION".to_vec(),
            name: b"SYN_DECISION".to_vec(),
            seed_component: installed_content::decisions::seed_component(b"SYN_DECISION"),
            has_state_targets: true,
            target_states: [b"4".to_vec(), b"2".to_vec(), b"4".to_vec()].into(),
            ..Default::default()
        },
    );
    (fixture, database)
}

#[test]
fn removal_description_binds_highlighted_target_from_and_stored_execution_count() {
    let (fixture, database) = decision_fixture(Some(None));
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let caller = scopes.scopes[0];
    let description = fixture.render(
        "remove_targeted_decision = { decision = SYN_DECISION target = BBB }",
        &mut scopes,
        Content {
            decisions: Some(&database),
            ..Default::default()
        },
    );
    // 1407290D0 base 0xE2746C3C+target2; 14071C840 draw1504267454%2=0.
    // Candidate order is [2,4], despite authored [4,2,4].
    assert_eq!(description.text, "Remove Conflict in Two\n");
    assert!(
        description.refusals.is_empty(),
        "{:?}",
        description.refusals
    );
    assert_eq!(scopes.scopes[0], caller);
    let (fixture, database) = decision_fixture(Some(Some(1)));
    let description = fixture.render(
        "remove_targeted_decision = { decision = SYN_DECISION target = BBB }",
        &mut scopes,
        Content {
            decisions: Some(&database),
            ..Default::default()
        },
    );
    // The persisted additive count changes the draw to634125187%2=1.
    assert_eq!(description.text, "Remove Conflict in Four\n");
    assert!(
        description.refusals.is_empty(),
        "{:?}",
        description.refusals
    );
    assert_eq!(scopes.scopes[0], caller);
}

#[test]
fn removal_refuses_unread_seed_but_fixed_seed_bypasses_it_and_missing_instance_is_empty() {
    let (mut fixture, mut database) = decision_fixture(None);
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let body = "remove_targeted_decision = { decision = SYN_DECISION target = BBB }";
    let description = fixture.render(
        body,
        &mut scopes,
        Content {
            decisions: Some(&database),
            ..Default::default()
        },
    );
    assert!(description.text.is_empty());
    assert_eq!(description.refusals.values().sum::<usize>(), 1);
    assert!(
        description
            .refusals
            .keys()
            .next()
            .unwrap()
            .contains("description_seed")
    );
    database
        .entries
        .get_mut(b"syn_decision".as_slice())
        .unwrap()
        .fixed_random_seed = true;
    assert_eq!(
        fixture
            .render(
                body,
                &mut scopes,
                Content {
                    decisions: Some(&database),
                    ..Default::default()
                }
            )
            .text,
        "Remove Conflict in Two\n"
    );
    set::<tables::Decisions>(&mut fixture, vec![]);
    let absent = fixture.render(
        body,
        &mut scopes,
        Content {
            decisions: Some(&database),
            ..Default::default()
        },
    );
    assert!(absent.text.is_empty() && absent.refusals.is_empty());
    *fixture.frame.tables[replay::rows::TableId::Decisions.index()] = TableView::Unavailable {
        reason: "synthetic fault".into(),
    };
    let unavailable = fixture.render(
        body,
        &mut scopes,
        Content {
            decisions: Some(&database),
            ..Default::default()
        },
    );
    assert_eq!(unavailable.refusals.values().sum::<usize>(), 1);
}

#[test]
fn highlight_filters_use_native_fresh_country_from_state_and_all_state_fallback() {
    let (fixture, mut database) = decision_fixture(Some(None));
    let definition = database
        .entries
        .get_mut(b"syn_decision".as_slice())
        .unwrap();
    definition.target_trigger = installed_content::parser::parse(b"FROM = { state = 2 }")
        .unwrap()
        .items;
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let body = "remove_targeted_decision = { decision = SYN_DECISION target = BBB }";
    let output = fixture.render(
        body,
        &mut scopes,
        Content {
            decisions: Some(&database),
            ..Default::default()
        },
    );
    assert_eq!(output.text, "Remove Conflict in Two\n");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
    let definition = database
        .entries
        .get_mut(b"syn_decision".as_slice())
        .unwrap();
    definition.target_root_trigger = installed_content::parser::parse(b"tag = CCC").unwrap().items;
    definition.highlight_conditions = installed_content::parser::parse(b"state = 5").unwrap().items;
    let output = fixture.render(
        body,
        &mut scopes,
        Content {
            decisions: Some(&database),
            ..Default::default()
        },
    );
    assert_eq!(output.text, "Remove Conflict in Five\n");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn state_target_wins_before_authored_highlight_candidates() {
    let (mut fixture, database) = decision_fixture(Some(None));
    set::<tables::Decisions>(
        &mut fixture,
        vec![DecisionRow {
            country: 100,
            container: 3,
            token: b"SYN_DECISION".to_vec(),
            numeric_target: 3,
            description_seed: Some(None),
            ..Default::default()
        }],
    );
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let output = fixture.render(
        "remove_targeted_decision = { decision = SYN_DECISION target = 3 }",
        &mut scopes,
        Content {
            decisions: Some(&database),
            ..Default::default()
        },
    );
    assert_eq!(output.text, "Remove Conflict in Three\n");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn inactive_decision_skips_unsupported_title_and_empty_trigger_does_not_enable_highlight() {
    let (mut fixture, mut database) = decision_fixture(Some(None));
    database
        .entries
        .get_mut(b"syn_decision".as_slice())
        .unwrap()
        .refused = Some("unsupported synthetic highlight".into());
    set::<tables::Decisions>(&mut fixture, vec![]);
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let body = "remove_targeted_decision = { decision = SYN_DECISION target = BBB }";
    let output = fixture.render(
        body,
        &mut scopes,
        Content {
            decisions: Some(&database),
            ..Default::default()
        },
    );
    assert!(output.text.is_empty() && output.refusals.is_empty());
    let (mut fixture, mut database) = decision_fixture(Some(None));
    let definition = database
        .entries
        .get_mut(b"syn_decision".as_slice())
        .unwrap();
    definition.has_state_targets = false;
    definition.target_states.clear();
    definition.highlight_states = installed_content::parser::parse(b"highlight_states_trigger = {}")
        .unwrap()
        .items;
    set::<tables::Decisions>(
        &mut fixture,
        vec![DecisionRow {
            country: 100,
            container: 3,
            token: b"SYN_DECISION".to_vec(),
            numeric_target: 3,
            description_seed: Some(None),
            ..Default::default()
        }],
    );
    let output = fixture.render(
        "remove_targeted_decision = { decision = SYN_DECISION target = 3 }",
        &mut scopes,
        Content {
            decisions: Some(&database),
            ..Default::default()
        },
    );
    // No native +98C children and no +9E0: FROM.FROM remains the target itself.
    assert_eq!(output.text, "Remove Conflict in Three\n");
    assert!(output.refusals.is_empty());
    assert!(
        scopes
            .scopes
            .last()
            .is_some_and(|scope| scope.this == EventObject::State(3) && scope.from == Link::Itself)
    );
}
