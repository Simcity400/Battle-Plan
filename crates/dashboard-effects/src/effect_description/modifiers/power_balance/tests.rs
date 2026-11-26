use super::*;
use crate::effect_description::tests::{Fixture, fresh};
use content_reader_politics::power_balance::{Database, Definition, Range};
use replay_data::rows::*;
use replay_frame::frame::TableView;
use crate::fixtures::game;

fn fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.catalogue.localisation = fixture.catalogue.localisation.iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned())).chain([
            ("SYN_BALANCE", "Balance"), ("SYN_OTHER", "Other"), ("SYN_LEFT", "Left"), ("SYN_RIGHT", "Right"),
            ("SYN_LOW", "Low"), ("SYN_HIGH", "High"), ("SYN_MOD", "Modifier"),
            ("EFFECT_ADD_POWER_BALANCE_MODIFIER", "Add $MODIFIER$/$POWER_BALANCE$:$DESC$"),
            ("EFFECT_REMOVE_POWER_BALANCE_MODIFIER", "Remove $MODIFIER$/$POWER_BALANCE$:$DESC$"),
            ("EFFECT_REMOVE_POWER_BALANCE", "Remove $POWER_BALANCE$"),
            ("EFFECT_ADD_POWER_BALANCE", "Add $POWER_BALANCE$:$SIDE1$/$SIDE2$/$VALUE|2$/$TRENDING_SIDE$"),
            ("EFFECT_REMOVE_POWER_BALANCE_SIDE", "Remove $SIDE$/$POWER_BALANCE$"),
            ("EFFECT_ADD_POWER_BALANCE_SIDE", "Add $SIDE$/$POWER_BALANCE$"),
            ("EFFECT_SET_POWER_BALANCE_VALUE", "Set $POWER_BALANCE$/$VALUE|2$/$TRENDING_SIDE$"),
            ("EFFECT_ADD_POWER_BALANCE_VALUE", "$POWER_BALANCE$:$TREND$$PERCENT|2$/$SIDE$->$PREDICTED_VALUE|2$/$PREDICTED_SIDE$"),
            ("EFFECT_POWER_BALANCE_VALUE_MOVED", "$POWER_BALANCE$:$TREND$$RANGE$"),
        ].into_iter().map(|(key, value)| (key.into(), value.into())))
        .collect::<Localisation>().into();
    *fixture.frame.tables[TableId::PowerBalances.index()] =
        TableView::Complete(TableRows::PowerBalances(vec![PowerBalanceRow {
            ordinal: 4,
            value_raw: -20_000,
            balance: b"SYN_BALANCE".to_vec(),
            left: b"SYN_LEFT".to_vec(),
            right: b"SYN_RIGHT".to_vec(),
            active_range: b"SYN_LOW".to_vec(),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::PowerBalanceMembers.index()] =
        TableView::Complete(TableRows::PowerBalanceMembers(vec![
            PowerBalanceMemberRow {
                record: 4,
                country: 100,
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::PowerBalanceRanges.index()] =
        TableView::Complete(TableRows::PowerBalanceRanges(vec![
            PowerBalanceRangeRow {
                record: 4,
                ordinal: 0,
                identity: b"SYN_LOW".to_vec(),
            },
            PowerBalanceRangeRow {
                record: 4,
                ordinal: 1,
                identity: b"SYN_HIGH".to_vec(),
            },
        ]));
    fixture
}

fn database() -> Database {
    Database::from_parsed(["SYN_BALANCE", "SYN_OTHER"].map(|id| {
        (
            Definition {
                id: id.as_bytes().to_vec(),
                initial_value: -25_000,
                left: b"SYN_LEFT".to_vec(),
                right: b"SYN_RIGHT".to_vec(),
                sides: vec![b"SYN_LEFT".to_vec(), b"SYN_RIGHT".to_vec()],
            },
            vec![
                Range {
                    id: b"SYN_LOW".to_vec(),
                    min: -100_000,
                    max: 0,
                    ..Default::default()
                },
                Range {
                    id: b"SYN_HIGH".to_vec(),
                    min: 0,
                    max: 100_000,
                    ..Default::default()
                },
            ],
        )
    }))
}

fn render(fixture: &Fixture, database: &Database, body: &str) -> Description {
    let out = fixture.render(
        body,
        &mut fresh(),
        Content {
            power_balances: Some(database),
            ..Default::default()
        },
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    out
}

#[test]
fn static_balance_modifiers_use_the_shared_raw_formatter_without_lf() {
    game::enter();
    let fixture = fixture();
    let database = database();
    let static_modifiers = content_reader_politics::static_modifiers::Descriptions::from_blocks([(
        "SYN_MOD".into(),
        Modifiers::default(),
    )]);
    for (key, verb) in [
        ("fixture_balance_bonus", "Add"),
        ("fixture_revoke_balance_bonus", "Remove"),
    ] {
        let out = fixture.render(
            &format!("{key}={{id=SYN_BALANCE modifier=SYN_MOD}}"),
            &mut fresh(),
            Content {
                power_balances: Some(&database),
                static_modifiers: Some(&static_modifiers),
                ..Default::default()
            },
        );
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
        assert_eq!(out.text, format!("{verb} Modifier/Balance:"));
    }
}

#[test]
fn set_and_remove_compare_the_first_country_membership_and_keep_order() {
    game::enter();
    let fixture = fixture();
    let database = database();
    assert_eq!(
        render(&fixture, &database, "fixture_end_balance={id=SYN_OTHER}").text,
        ""
    );
    assert_eq!(
        render(&fixture, &database, "fixture_end_balance={id=syn_balance}").text,
        "Remove Balance\n"
    );
    assert_eq!(
        render(
            &fixture,
            &database,
            "fixture_begin_balance={id=SYN_OTHER set_default=yes}"
        )
        .text,
        "Remove Balance\nAdd Other:Left/Right/0.25/Left\n"
    );
    assert_eq!(
        render(
            &fixture,
            &database,
            "fixture_begin_balance={id=SYN_BALANCE left_side=SYN_RIGHT set_value=0.5}"
        )
        .text,
        "Remove Left/Balance\nAdd Right/Balance\nSet Balance/0.50/Right\n"
    );
}

#[test]
fn predicted_value_clamps_and_range_changes_use_strict_bounds_and_two_newlines() {
    game::enter();
    let fixture = fixture();
    let database = database();
    assert_eq!(
        render(
            &fixture,
            &database,
            "fixture_shift_balance={id=SYN_BALANCE value=0.4}"
        )
        .text,
        "Balance:\u{13}BoP_right_texticon 0.40/Right->0.20/Right\n\nBalance:\u{13}BoP_right_texticon High\n"
    );
    assert_eq!(
        render(
            &fixture,
            &database,
            "fixture_shift_balance={id=SYN_BALANCE value=0.2 tooltip_side=SYN_LEFT}"
        )
        .text,
        "Balance:\u{13}BoP_right_texticon 0.20/Left->0.00/Right"
    );
    assert_eq!(
        render(
            &fixture,
            &database,
            "fixture_shift_balance={id=SYN_BALANCE value=2}"
        )
        .text,
        "Balance:\u{13}BoP_right_texticon 2.00/Right->1.00/Right"
    );
}

#[test]
fn duplicate_runtime_range_identity_is_a_named_refusal() {
    game::enter();
    let fixture = fixture();
    let definition = database().definition(b"SYN_BALANCE").unwrap().clone();
    let database = Database::from_parsed([(
        definition,
        vec![
            Range {
                id: b"SYN_LOW".to_vec(),
                min: -100_000,
                max: 0,
                ..Default::default()
            },
            Range {
                id: b"SYN_LOW".to_vec(),
                min: -50_000,
                max: 0,
                ..Default::default()
            },
        ],
    )]);
    let out = fixture.render(
        "fixture_shift_balance={id=SYN_BALANCE value=0}",
        &mut fresh(),
        Content {
            power_balances: Some(&database),
            ..Default::default()
        },
    );
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals.keys().next().unwrap().contains("duplicate")
            || out.refusals.keys().next().unwrap().contains("ranges named")
    );
}

#[test]
fn moving_range_formats_the_numeric_difference_with_a_forced_plus() {
    game::enter();
    let mut fixture = fixture();
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 1,
            literal: b"attack".to_vec(),
        }]));
    *fixture.frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(vec![
            ModifierDefinitionRow {
                token: 1,
                name_key: b"ATTACK".to_vec(),
                category_mask: 1,
                precision: 2,
                flags: crate::modifier_text::POSITIVE_IS_GOOD,
                ..Default::default()
            },
        ]));
    let definition = database().definition(b"SYN_BALANCE").unwrap().clone();
    let modifiers = |body: &[u8]| {
        content_reader_script::modifiers::values(&content_reader_parser::parser::parse(body).unwrap().items)
    };
    let database = Database::from_parsed([(
        definition,
        vec![
            Range {
                id: b"SYN_LOW".to_vec(),
                min: -100_000,
                max: 0,
                modifiers: modifiers(b"attack=1"),
                ..Default::default()
            },
            Range {
                id: b"SYN_HIGH".to_vec(),
                min: 0,
                max: 100_000,
                modifiers: modifiers(b"attack=2"),
                ..Default::default()
            },
        ],
    )]);
    assert_eq!(
        render(
            &fixture,
            &database,
            "fixture_shift_balance={id=SYN_BALANCE value=0.4}"
        )
        .text,
        "Balance:\u{13}BoP_right_texticon 0.40/Right->0.20/Right\n\nBalance:\u{13}BoP_right_texticon High\n Attack: \u{11}G+1.00\u{11}!\n"
    );
}

#[test]
fn changing_an_unnamed_recorded_side_refuses_its_lost_null_identity() {
    game::enter();
    let mut fixture = fixture();
    let TableView::Complete(TableRows::PowerBalances(rows)) =
        &mut *fixture.frame.tables[TableId::PowerBalances.index()]
    else {
        unreachable!()
    };
    rows[0].left.clear();
    let mut definition = database().definition(b"SYN_BALANCE").unwrap().clone();
    definition.sides.push(Vec::new());
    let database = Database::from_parsed([(definition, Vec::new())]);
    let out = fixture.render(
        "fixture_begin_balance={id=SYN_BALANCE left_side=SYN_RIGHT}",
        &mut fresh(),
        Content {
            power_balances: Some(&database),
            ..Default::default()
        },
    );
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals
            .keys()
            .next()
            .unwrap()
            .contains("valid/null identity")
    );
}
