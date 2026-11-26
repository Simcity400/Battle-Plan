use super::*;
use crate::script_test_game as game;
use crate::scope::tests::positive_frame as frame;
use crate::scripted_text::{EventObject, EventScope, EventScopes, ScriptedText};
use content_reader_localisation::localisation::Localisation;
use replay_data::rows::{PoliticalPartyRow, TableId, TableRows};
use replay_frame::frame::{DecodedFrame, TableView};

const SCRIPTED: &str = "defined_text = { name = GetModText text = { localization_key = GER } }";

const TEXTS: &str = concat!(
    " RECORDED:0 \"[GetModText] of [ROOT.GetNameDef] in [GetYear]\"\n",
    " PARTY:0 \"[ROOT.GetRulingParty]\"\n",
);

fn party_frame() -> DecodedFrame {
    let mut frame = frame();
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 100,
            id: 3,
            token: b"fascism".to_vec(),
            tag: 100,
            name_default: 1,
            ..Default::default()
        }]));
    *frame.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(Vec::new()));
    if let TableView::Complete(TableRows::Politics(rows)) =
        &mut *frame.tables[TableId::Politics.index()]
    {
        rows[0].ruling_party = Some(3);
    }
    frame
}

#[test]
fn recorded_operands_expand_a_text_as_its_hour_did() {
    game::enter();
    let (catalogue, _folder) = super::super::tests::catalogue(SCRIPTED, TEXTS);
    let content = catalogue.scripted_content().unwrap();
    let frame = party_frame();
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    let scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let localised = |key: &[u8]| catalogue.localisation.localise(key).unwrap();
    let (raw, party) = (localised(b"RECORDED"), localised(b"PARTY"));
    let expected = text.expand_in(&raw, &scopes);
    let ruling = text.expand_in(&party, &scopes);
    assert!(
        expected.is_ok() && ruling.is_ok(),
        "{expected:?} {ruling:?}"
    );
    let mut operands = Operands::new(frame.hour);
    assert_eq!(
        text.record(b"RECORDED", &raw, &scopes, &mut operands),
        expected
    );
    assert_eq!(
        text.record(b"PARTY", &party, &scopes, &mut operands),
        ruling
    );
    drop(text);
    drop(frame);
    let expand = |text: &str, key: &[u8], localisation: &Localisation| {
        expand(text, key, Some(&operands), localisation, content)
    };
    assert_eq!(expand(&raw, b"RECORDED", &catalogue.localisation), expected);
    assert_eq!(expand(&party, b"PARTY", &catalogue.localisation), ruling);
    let Some(Ok(context)) = operands.text(b"RECORDED") else {
        panic!("RECORDED has no context");
    };
    assert!(matches!(
        &operands.contexts[*context as usize][0].value,
        Value::Entry(Some((key, _))) if key == b"GER"
    ));
    let mut german = Localisation::clone(&catalogue.localisation);
    german.language = "german".into();
    assert_eq!(expand(&raw, b"RECORDED", &german), expected);
    let error = expand(&party, b"PARTY", &german).unwrap_err();
    assert!(
        error.contains("[ROOT.GetRulingParty]") && error.contains("read in english"),
        "{error}"
    );
    let error = expand(
        "[ROOT.GetAdjective] [GetModText]",
        b"RECORDED",
        &catalogue.localisation,
    )
    .unwrap_err();
    assert!(
        error.contains("[ROOT.GetAdjective] was not evaluated"),
        "{error}"
    );
    let error = expand("[ROOT.GetName]", b"UNRECORDED", &catalogue.localisation).unwrap_err();
    assert!(
        error.contains("[ROOT.GetName] was not evaluated"),
        "{error}"
    );
    let restored: Operands =
        serde_json::from_slice(&serde_json::to_vec(&operands).unwrap()).unwrap();
    assert_eq!(restored, operands);
}

#[test]
fn recorded_refusals_and_values_match_the_hour() {
    game::enter();
    let (catalogue, _folder) = super::super::tests::catalogue(SCRIPTED, TEXTS);
    let content = catalogue.scripted_content().unwrap();
    let frame = party_frame();
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    let scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let texts = [
        "[ROOT.GetLeader]",
        "[Player.GetTag]",
        "[ROOT.Capital.GetName]",
        "[ROOT.Overlord.GetNameDef] / [SOV.GetAdjectiveCap]",
        "[ROOT.GetRulingIdeologyNoun] [ROOT.GetNonIdeologyName]",
        "[GetDateText] [GetMonth]",
        "[(ROOT.GetName?a:b)]",
        "[THIS.GetFlag] [ROOT.GetNameWithFlag]",
    ];
    let mut expected = Vec::new();
    let mut operands = Operands::new(frame.hour);
    for (at, raw) in texts.iter().enumerate() {
        let key = format!("TEXT_{at}");
        let hour = text.expand_in(raw, &scopes);
        assert_eq!(
            text.record(key.as_bytes(), raw, &scopes, &mut operands),
            hour
        );
        expected.push(hour);
    }
    drop(text);
    drop(frame);
    for (at, (raw, hour)) in texts.iter().zip(expected).enumerate() {
        let key = format!("TEXT_{at}");
        assert_eq!(
            expand(
                raw,
                key.as_bytes(),
                Some(&operands),
                &catalogue.localisation,
                content
            ),
            hour,
            "{raw}"
        );
    }
}

#[test]
fn a_key_keeps_its_first_recording() {
    game::enter();
    let (catalogue, _folder) = super::super::tests::catalogue(SCRIPTED, TEXTS);
    let content = catalogue.scripted_content().unwrap();
    let frame = frame();
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    let mut operands = Operands::new(frame.hour);
    assert_eq!(
        text.record_root(b"PLAIN", "No brackets", &mut operands),
        Ok("No brackets".into())
    );
    assert!(operands.is_empty());
    assert_eq!(
        text.record_root(b"TAG", "[ROOT.GetTag]", &mut operands),
        Ok("GER".into())
    );
    let soviet = EventScopes::new(vec![EventScope::of(EventObject::Country(2))]);
    assert_eq!(
        text.record(b"TAG", "[ROOT.GetTag]", &soviet, &mut operands),
        Ok("SOV".into())
    );
    let mut later = Operands::new(frame.hour + 1);
    assert_eq!(
        text.record_root(b"TAG", "[ROOT.GetTag]", &mut later),
        Ok("GER".into())
    );
    drop(text);
    drop(frame);
    let error = expand(
        "[ROOT.GetTag]",
        b"TAG",
        Some(&later),
        &catalogue.localisation,
        content,
    )
    .unwrap_err();
    assert!(error.contains("read at hour"), "{error}");
    assert_eq!(
        expand(
            "[ROOT.GetTag]",
            b"TAG",
            Some(&operands),
            &catalogue.localisation,
            content
        ),
        Ok("GER".into())
    );
}

#[test]
fn recorded_variables_keep_their_value_and_order() {
    game::enter();
    let (catalogue, _folder) = super::super::tests::catalogue(SCRIPTED, "");
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment(Default::default());
    let mut frame = frame();
    *frame.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(Vec::new()));
    for id in [
        TableId::CountryCharacters,
        TableId::CountryRetiredCharacters,
    ] {
        *frame.tables[id.index()] = TableView::Complete(TableRows::empty(id));
    }
    *frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(
            [(100, 125_000), (200, 275_000)]
                .map(
                    |(scope_id, value_raw)| replay_data::rows::ScriptVariablesRow {
                        scope_kind: 1,
                        scope_id,
                        seed: replay_data::rows::Recorded::Value(None),
                        entries: vec![replay_data::rows::ScriptVariableEntry {
                            name: b"my_var".to_vec(),
                            value_raw,
                        }],
                    },
                )
                .into(),
        ));
    for (country, value) in [(1, "1.2"), (2, "2.7")] {
        let text = ScriptedText::new(&frame, &catalogue.localisation, content, country);
        let mut operands = Operands::new(frame.hour);
        let raw = "[?ROOT.my_var|1] of [GetModText]";
        let hour = text.record_root(b"BOTH", raw, &mut operands);
        assert_eq!(hour, Ok(format!("{value} of German Reich")));
        drop(text);
        let expand = |text: &str| {
            expand(
                text,
                b"BOTH",
                Some(&operands),
                &catalogue.localisation,
                content,
            )
        };
        assert_eq!(expand(raw), hour);
        let error = expand("[GetModText] of [?ROOT.my_var|1]").unwrap_err();
        assert!(error.contains("another order"), "{error}");
    }
}

#[test]
fn a_property_expanding_text_inside_records_only_its_text() {
    game::enter();
    for (range, expected, stateful) in [
        ("[ROOT.GetTag]", "GER range|SOV", false),
        ("[GetRangeText]", "Mod range|SOV", true),
    ] {
        balance_range(range, expected, stateful);
    }
}

fn balance_range(range: &str, expected: &str, stateful: bool) {
    use replay_data::rows::{
        PowerBalanceMemberRow, PowerBalanceRow, Recorded, TableSchema, tables,
    };
    fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
        *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
    }
    let (catalogue, _folder) = super::super::tests::catalogue(
        "defined_text = { name = GetRangeText text = { localization_key = RANGE_TEXT } }",
        &format!(
            " MOD_BALANCE:0 \"Balance\"\n MOD_LEFT:0 \"Left\"\n MOD_RIGHT:0 \"Right\"\n MOD_RANGE:0 \"{range} range\"\n RANGE_TEXT:0 \"Mod\"\n"
        ),
    );
    let content = catalogue.scripted_content().unwrap();
    let mut frame = super::super::tests::complete_frame();
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *frame.tables[TableId::CountryAppearance.index()]
    {
        for row in rows {
            row.original_tag_reference = Recorded::Value(Some(0));
            row.original_tag = Recorded::Value(Some(Vec::new()));
        }
    }
    set::<tables::PowerBalances>(
        &mut frame,
        vec![PowerBalanceRow {
            ordinal: 0,
            value_raw: -1,
            balance: b"MOD_BALANCE".to_vec(),
            left: b"MOD_LEFT".to_vec(),
            right: b"MOD_RIGHT".to_vec(),
            trending: b"MOD_RIGHT".to_vec(),
            active_range: b"MOD_RANGE".to_vec(),
        }],
    );
    set::<tables::PowerBalanceMembers>(
        &mut frame,
        vec![PowerBalanceMemberRow {
            country: 100,
            ..Default::default()
        }],
    );
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 2);
    let raw = "[GER.GetActiveRangeName]|[ROOT.GetTag]";
    let hour = text.expand(raw);
    assert_eq!(hour.as_deref(), Ok(expected));
    let mut operands = Operands::new(frame.hour);
    assert_eq!(text.record_root(b"RANGE", raw, &mut operands), hour);
    drop(text);
    drop(frame);
    let (range, _) = expected.split_once('|').unwrap();
    assert_eq!(
        operands.contexts[0][0].value,
        Value::Read {
            text: range.to_owned(),
            stateful
        }
    );
    assert_eq!(
        expand(
            raw,
            b"RANGE",
            Some(&operands),
            &catalogue.localisation,
            content
        ),
        hour
    );
}

#[test]
fn a_scripted_localisation_leaving_out_what_later_values_read_is_refused() {
    game::enter();
    let command = |inner: &str, value: Value| Command {
        inner: inner.to_owned(),
        value,
    };
    let read = |stateful| Value::Read {
        text: String::new(),
        stateful,
    };
    for (set, stateful) in [
        (Value::Entry(None), true),
        (read(true), true),
        (read(false), false),
    ] {
        let operands = Operands {
            language: "english".into(),
            texts: vec![(b"TOP".to_vec(), Ok(0))],
            contexts: vec![
                vec![
                    command("Init", Value::Entry(None)),
                    command("Outer", Value::Entry(Some((b"OUTER".to_vec(), 1)))),
                    command("?temp", Value::Number(2)),
                ],
                vec![command("Set", set.clone())],
            ],
            ..Operands::default()
        };
        for (outer, refused) in [("[Set]", false), ("", stateful)] {
            let (catalogue, _folder) =
                super::super::tests::catalogue("", &format!(" OUTER:0 \"{outer}\"\n"));
            let content = catalogue.scripted_content().unwrap();
            let read = expand(
                "[Init][Outer][?temp]",
                b"TOP",
                Some(&operands),
                &catalogue.localisation,
                content,
            );
            if refused {
                let error = read.unwrap_err();
                assert!(
                    error.starts_with("[Outer]") && error.contains("leaves out"),
                    "{error}"
                );
            } else {
                assert_eq!(read, Ok(format_variable("temp", 2)), "{set:?} {outer:?}");
            }
        }
    }
}

#[test]
fn scripted_localisations_nested_to_the_backstop_are_read_again() {
    game::enter();
    let (catalogue, _folder) = super::super::tests::catalogue("", " DEEP:0 \"[Deep]\"\n");
    let content = catalogue.scripted_content().unwrap();
    let deep = |value: Value| {
        vec![Command {
            inner: "Deep".into(),
            value,
        }]
    };
    let mut contexts: Vec<_> = (1..=MAX_SCRIPTED_NESTING as u32)
        .map(|next| deep(Value::Entry(Some((b"DEEP".to_vec(), next)))))
        .collect();
    contexts.push(deep(Value::Entry(None)));
    let operands = Operands {
        language: "english".into(),
        texts: vec![(b"TOP".to_vec(), Ok(0))],
        contexts,
        ..Operands::default()
    };
    assert_eq!(
        expand(
            "[Deep]",
            b"TOP",
            Some(&operands),
            &catalogue.localisation,
            content
        ),
        Ok(String::new())
    );
}
