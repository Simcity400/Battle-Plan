use super::*;
use replay::rows::{CharacterRow, DynamicModifierRow, StateDynamicModifierRow, StateRow};

fn fixture() -> DecodedFrame {
    let mut f = frame(
        &[
            (100, 11, "GER", 1, "GER", ""),
            (101, 12, "AAA", 1, "AAA", ""),
        ],
        &[],
    );
    *f.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        [58, 64, 685]
            .map(|id| StateRow {
                id,
                owner: 100,
                controller: 100,
                ..Default::default()
            })
            .into(),
    ));
    f
}

#[test]
fn dynamic_modifier_synthetic_channel_scope() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    // Independently authored channel-style rules exercise a direct state
    // condition and a state condition inside an OR with absent flag branches.
    let mut f = frame(&[(701, 17, "QXZ", 1, "QXZ", "")], &[]);
    *f.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        [117, 229]
            .map(|id| StateRow {
                id,
                owner: 701,
                controller: 701,
                ..Default::default()
            })
            .into(),
    ));
    let direct =
        "117 = { has_dynamic_modifier = { modifier = fixture_channel_alpha scope = THIS } }";
    let alternatives = "OR = { has_global_flag = FIXTURE_CHANNEL_CLOSED has_country_flag = FIXTURE_CHANNEL_OWNER_BLOCKED 229 = { has_dynamic_modifier = { modifier = fixture_channel_beta scope = THIS } } }";
    for (state, token, trigger) in [
        (117, b"fixture_channel_alpha".as_slice(), direct),
        (229, b"fixture_channel_beta".as_slice(), alternatives),
    ] {
        *f.tables[TableId::StateDynamicModifiers.index()] =
            TableView::Complete(TableRows::StateDynamicModifiers(vec![
                StateDynamicModifierRow {
                    state,
                    token: token.to_vec(),
                    scope_state: state as i32,
                    ..Default::default()
                },
            ]));
        assert_eq!(check(&f, "", 17, trigger), Eval::True, "{trigger}");
        *f.tables[TableId::StateDynamicModifiers.index()] =
            TableView::Complete(TableRows::StateDynamicModifiers(vec![]));
        assert_eq!(check(&f, "", 17, trigger), Eval::False, "{trigger}");
    }
}

#[test]
fn dynamic_modifier_state_container_scans_targets_in_container_order() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut f = fixture();
    let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *f.tables[TableId::CountryLinks.index()]
    else {
        panic!("links")
    };
    links.push(CountryLinkRow {
        id: 103,
        canonical_slot: 11,
    });
    *f.tables[TableId::StateDynamicModifiers.index()] =
        TableView::Complete(TableRows::StateDynamicModifiers(vec![
            // Deliberately out of wire order: ordinal is stored container order.
            StateDynamicModifierRow {
                state: 58,
                ordinal: 2,
                token: b"custom".to_vec(),
                ..Default::default()
            },
            StateDynamicModifierRow {
                state: 58,
                ordinal: 0,
                token: b"custom".to_vec(),
                scope_country: 101,
                scope_state: 64,
            },
            StateDynamicModifierRow {
                state: 58,
                ordinal: 1,
                token: b"custom".to_vec(),
                scope_country: 103,
                scope_state: 58,
            },
        ]));
    for (operand, expected) in [
        ("THIS", Eval::True),
        ("64", Eval::True),
        ("ROOT", Eval::True),
        ("AAA", Eval::True),
        ("685", Eval::False),
        ("0", Eval::False),
    ] {
        let trigger =
            format!("58 = {{ has_dynamic_modifier = {{ modifier = custom scope = {operand} }} }}");
        assert_eq!(check(&f, "", 11, &trigger), expected, "{trigger}");
    }
    // No target: stop at the first definition match, even though a later
    // entry has no target. Country targets ignore scope_state; state targets
    // ignore scope_country.
    assert_eq!(
        check(&f, "", 11, "58 = { has_dynamic_modifier = custom }"),
        Eval::False
    );
    assert_eq!(
        check(
            &f,
            "",
            11,
            "has_dynamic_modifier = { modifier = custom scope = 58 }"
        ),
        Eval::False
    );
}

#[test]
fn dynamic_modifier_country_container_uses_aliases_and_variable_targets() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut f = with_variables(
        fixture(),
        &[
            (0, 0, &[("target", 64 * ONE)]),
            (1, 100, &[("target", 64 * ONE)]),
        ],
    );
    let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *f.tables[TableId::CountryLinks.index()]
    else {
        panic!("links")
    };
    links.push(CountryLinkRow {
        id: 103,
        canonical_slot: 11,
    });
    *f.tables[TableId::DynamicModifiers.index()] =
        TableView::Complete(TableRows::DynamicModifiers(vec![
            DynamicModifierRow {
                country: 100,
                ordinal: 2,
                token: b"custom".to_vec(),
                scope_country: 103,
                scope_state: 64,
                enabled: 0,
                days_remaining: 0,
                ..Default::default()
            },
            DynamicModifierRow {
                country: 100,
                ordinal: 0,
                token: b"custom".to_vec(),
                scope_country: 999,
                scope_state: 685,
                ..Default::default()
            },
            DynamicModifierRow {
                country: 100,
                ordinal: 1,
                token: b"custom".to_vec(),
                scope_country: 101,
                scope_state: 64,
                ..Default::default()
            },
            DynamicModifierRow {
                country: 101,
                token: b"custom".to_vec(),
                ..Default::default()
            },
            DynamicModifierRow {
                country: 100,
                token: b"unlinked".to_vec(),
                scope_country: 999,
                ..Default::default()
            },
        ]));
    for operand in ["THIS", "ROOT", "AAA", "64", "global.target", "target"] {
        let trigger = format!("has_dynamic_modifier = {{ modifier = custom scope = {operand} }}");
        assert_eq!(check(&f, "", 11, &trigger), Eval::True, "{trigger}");
    }
    with_world(&f, "", Err("none"), |world| {
        let scopes = Scopes {
            from: Some(Scope::State(64)),
            prev: Some(Scope::Country(12)),
            ..Scopes::of(Scope::Country(11))
        };
        for operand in ["FROM", "PREV"] {
            let trigger = items(&format!(
                "has_dynamic_modifier = {{ modifier = custom scope = {operand} }}"
            ));
            assert_eq!(world.evaluate(&trigger, scopes), Eval::True);
        }
    });
    assert_eq!(
        check(&f, "", 11, "has_dynamic_modifier = custom"),
        Eval::False
    );
    assert_eq!(
        check(&f, "", 12, "has_dynamic_modifier = custom"),
        Eval::True
    );
    assert_eq!(
        check(
            &f,
            "",
            12,
            "has_dynamic_modifier = { modifier = custom scope = 64 }"
        ),
        Eval::False
    );
    // A later proven match wins over an earlier unresolved alias. Without
    // a match, the missing alias remains unknown, never a guessed false.
    let why = check(
        &f,
        "",
        11,
        "has_dynamic_modifier = { modifier = unlinked scope = ROOT }",
    )
    .decided()
    .unwrap_err();
    assert!(why.contains("999") && why.contains("CountryLinks"), "{why}");
}

#[test]
fn dynamic_modifier_reader_defaults_and_exact_definition_names() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut f = with_variables(fixture(), &[(1, 100, &[])]);
    *f.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![CharacterRow {
            id: 20,
            ..Default::default()
        }]));
    *f.tables[TableId::DynamicModifiers.index()] =
        TableView::Complete(TableRows::DynamicModifiers(vec![DynamicModifierRow {
            country: 100,
            token: b"custom".to_vec(),
            scope_country: u32::MAX,
            ..Default::default()
        }]));
    // Accept a scalar or read just modifier and scope from a block;
    // later modifier strings replace earlier ones; no scope defaults to 0.
    for trigger in [
        "has_dynamic_modifier = custom",
        "has_dynamic_modifier = { modifier = custom }",
        "has_dynamic_modifier = { modifier = missing modifier = custom }",
        "has_dynamic_modifier = { modifier = custom unknown = { ignored = yes } }",
        "has_dynamic_modifier = { modifier = custom scope = 0 }",
        "has_dynamic_modifier = { modifier = custom scope = absent_variable }",
        "has_dynamic_modifier = { modifier = custom scope = 1.5 }",
        "has_dynamic_modifier = { modifier = custom scope = ROOT scope = 0 }",
        // Decode the tagged value as a character:
        // the fresh auxiliary scope carries neither country nor state.
        "has_dynamic_modifier = { modifier = custom scope = 5368.70932 }",
    ] {
        assert_eq!(check(&f, "", 11, trigger), Eval::True, "{trigger}");
    }
    for trigger in [
        "has_dynamic_modifier = { }",
        "has_dynamic_modifier = { scope = 64 }",
        "has_dynamic_modifier = CUSTOM",
        "has_dynamic_modifier = { modifier = custom modifier = missing }",
    ] {
        assert_eq!(check(&f, "", 11, trigger), Eval::False, "{trigger}");
    }
    // A repeated nonliteral scoped-variable read retains earlier fields;
    // this zero-reach form stays a named refusal, rather than last-wins.
    let why = check(
        &f,
        "",
        11,
        "has_dynamic_modifier = { modifier = custom scope = ROOT scope = THIS }",
    )
    .decided()
    .unwrap_err();
    assert!(
        why.contains("scope is assigned 2 times") && why.contains("keeps fields"),
        "{why}"
    );
}

#[test]
fn dynamic_modifier_missing_tables_and_owner_links_remain_named() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut f = fixture();
    for (id, table_name, trigger) in [
        (
            TableId::DynamicModifiers,
            "DynamicModifiers",
            "has_dynamic_modifier = { modifier = custom scope = ROOT }",
        ),
        (
            TableId::StateDynamicModifiers,
            "StateDynamicModifiers",
            "58 = { has_dynamic_modifier = { modifier = custom scope = THIS } }",
        ),
    ] {
        for table in [
            TableView::Unavailable {
                reason: "test gap".into(),
            },
            TableView::Truncated {
                rows: TableRows::empty(id),
                missing: 1,
            },
        ] {
            *f.tables[id.index()] = table;
            let why = check(&f, "", 11, trigger).decided().unwrap_err();
            assert!(why.contains(table_name), "{why}");
        }
        *f.tables[id.index()] = TableView::Complete(TableRows::empty(id));
    }
    *f.tables[TableId::DynamicModifiers.index()] =
        TableView::Complete(TableRows::DynamicModifiers(vec![DynamicModifierRow {
            country: 999,
            token: b"custom".to_vec(),
            ..Default::default()
        }]));
    let why = check(
        &f,
        "",
        11,
        "has_dynamic_modifier = { modifier = custom scope = ROOT }",
    )
    .decided()
    .unwrap_err();
    assert!(
        why.contains("DynamicModifiers") && why.contains("999"),
        "{why}"
    );
}

#[test]
fn dynamic_modifier_country_index_at_5000_countries() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let tags: Vec<_> = (0..5000)
        .map(|id| {
            format!(
                "{}{}{}",
                char::from(b'A' + (id / 676) as u8),
                char::from(b'A' + (id / 26 % 26) as u8),
                char::from(b'A' + (id % 26) as u8)
            )
        })
        .collect();
    let countries: Vec<_> = tags
        .iter()
        .enumerate()
        .map(|(id, tag)| {
            (
                id as u32 + 100,
                id as u32,
                tag.as_str(),
                1,
                tag.as_str(),
                "",
            )
        })
        .collect();
    let mut f = frame(&countries, &[]);
    *f.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![StateRow {
        id: 64,
        ..Default::default()
    }]));
    *f.tables[TableId::DynamicModifiers.index()] =
        TableView::Complete(TableRows::DynamicModifiers(
            (0..5000)
                .map(|id| DynamicModifierRow {
                    country: id + 100,
                    token: b"modded_name".to_vec(),
                    scope_state: 64,
                    ..Default::default()
                })
                .collect(),
        ));
    with_world(&f, "", Err("none"), |world| {
        let trigger = items("has_dynamic_modifier = { modifier = modded_name scope = 64 }");
        for slot in [0, 2499, 4999] {
            assert_eq!(
                world.evaluate(&trigger, Scopes::of(Scope::Country(slot))),
                Eval::True
            );
        }
    });
}
