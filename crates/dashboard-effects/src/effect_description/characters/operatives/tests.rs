use super::*;
use crate::fixtures::{Folder, folder_catalogue};
use crate::effect_description::tests::Fixture;
use replay_data::rows::*;
use replay_frame::frame::TableView;
use crate::fixtures::game;

fn fixture(event_argument: &str) -> (Fixture, Folder) {
    let mut fixture = Fixture::new();
    let localisation = format!(
        concat!(
            "\u{feff}l_english:\n",
            " AAA:0 \"Amber\"\n BBB:0 \"Beryl\"\n CCC:0 \"Cedar\"\n",
            " AAA_DEF:0 \"amber union\"\n BBB_DEF:0 \"beryl union\"\n CCC_DEF:0 \"cedar union\"\n",
            " EFFECT_CAPTURE_OPERATIVE:0 \"Capture $OPERATIVE$ by $CAPTURED_BY$ [Root.GetTag]\"\n",
            " EFFECT_KILL_OPERATIVE:0 \"Kill $OPERATIVE$ by $KILLED_BY$ [Root.GetTag]\"\n",
            " EFFECT_TURN_OPERATIVE:0 \"Turn $OPERATIVE$ by $TURNED_BY$ [Root.GetTag]\"\n",
            " EFFECT_FORCE_OPERATIVE_INTO_HIDING:0 \"Hide $NAME$/$CODENAME$/$DAYS|0$ [Root.GetTag]\"\n",
            " EFFECT_HARM_OPERATIVE:0 \"Harm $NAME$/$CODENAME$/$DAYS|0$ [Root.GetTag]\"\n",
            " EFFECT_COUNTRY_EVENT:0 \"Event ${}$ [Root.GetTag]\"\n",
            " UNIT_TITLE:0 \"Unit [From.GetTag]\"\n",
            " OPERATIVE_TITLE:0 \"Operative [From.GetTag]/[From.From.GetTag]/[Root.GetTag]\"\n",
            " EFFECT_FREE_RANDOM_OPERATIVE:0 \"Free $COUNTRY$\"\n EFFECT_FREE_RANDOM_OPERATIVE_ALL:0 \"Free all $COUNTRY$\"\n",
            " RAW:0 \"Wrong localisation\"\n"
        ),
        event_argument,
    );
    let (catalogue, folder) = folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        ("localisation/test_l_english.yml", &localisation),
        (
            "common/units/codenames_operatives/test.txt",
            "synthetic_codes={type=codename unique={\"Nightjar\"}}",
        ),
    ]);
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    fixture.catalogue = catalogue;
    *fixture.frame.tables[TableId::Characters.index()] =
        TableView::Complete(TableRows::Characters(vec![
            CharacterRow {
                id: 7,
                country: 100,
                name_token_id: 10,
                name: b"Character text".to_vec(),
                ..Default::default()
            },
            CharacterRow {
                id: 8,
                country: 100,
                name_token_id: 11,
                name: b"Other character".to_vec(),
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![
            UnitLeaderRow {
                reference_type: 55,
                id: 5,
                character_id: 7,
                category: 3,
                owner: 100,
                inherited_text_b: b"Agent text".to_vec(),
                codename: Recorded::NotRecorded,
                ..Default::default()
            },
            UnitLeaderRow {
                reference_type: 55,
                id: 6,
                character_id: 8,
                category: 3,
                owner: 100,
                inherited_text_b: b"Other agent".to_vec(),
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 10,
                literal: b"synthetic_agent".to_vec(),
            },
            LexicalTokenRow {
                token: 11,
                literal: b"synthetic_other_agent".to_vec(),
            },
        ]));
    *fixture.frame.tables[TableId::CountryCharacters.index()] =
        TableView::Complete(TableRows::CountryCharacters(vec![
            CountryCharacterRow {
                country: 100,
                ordinal: 0,
                character: Some(7),
            },
            CountryCharacterRow {
                country: 100,
                ordinal: 1,
                character: Some(8),
            },
        ]));
    crate::effect_description::characters::tests::empty_variable_stores(&mut fixture);
    (fixture, folder)
}

fn operative_scopes() -> EventScopes {
    let mut own = EventScope::of(EventObject::Character(7));
    own.root = Link::Scope(1);
    own.from = Link::Scope(2);
    own.random_seed = 123;
    own.random_counter = 456;
    EventScopes::new(vec![
        own,
        EventScope::of(EventObject::Country(1)),
        EventScope::of(EventObject::Country(2)),
    ])
}

fn country_scopes() -> EventScopes {
    let mut scopes = operative_scopes();
    scopes.scopes[0].this = EventObject::Country(1);
    scopes
}

fn event(key: &str) -> Arc<content_reader_events::events::Event> {
    Arc::new(content_reader_events::events::Event {
        id: b"synthetic.1".to_vec(),
        titles: content_reader_parser::parser::parse(format!("title={key}").as_bytes())
            .unwrap()
            .items,
        ..Default::default()
    })
}

#[test]
fn operative_actions_resolve_country_operands() {
    game::enter();
    let (fixture, _folder) = fixture("EVENT");
    for (body, expected) in [
        (
            "fixture_capture_agent={captured_by=FROM}",
            "Capture Agent text by Beryl union AAA",
        ),
        (
            "kill_operative={killed_by=ROOT}",
            "Kill Agent text by amber union AAA",
        ),
        (
            "turn_operative={turned_by=ROOT}",
            "Turn Agent text by amber union AAA",
        ),
    ] {
        let out = fixture.render(body, &mut operative_scopes(), Content::default());
        assert_eq!(out.text, expected, "{body}");
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
}

#[test]
fn explicit_operative_uses_country_roster_and_exact_leader_name() {
    game::enter();
    let (fixture, _folder) = fixture("EVENT");
    let out = fixture.render(
        "fixture_capture_agent={operative=synthetic_other_agent captured_by=FROM}",
        &mut operative_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Capture Other agent by Beryl union AAA");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn free_random_operative_refuses_the_missing_captured_reference_roster() {
    game::enter();
    let (mut fixture, _folder) = fixture("EVENT");
    *fixture.frame.tables[TableId::CapturedOperativeReferences.index()] = TableView::Unavailable {
        reason: "captured references not recorded".into(),
    };
    let out = fixture.render(
        "fixture_release_agent={captured_by=FROM all=yes}",
        &mut country_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([("synthetic effect fixture_release_agent: key fixture_release_agent: hour 1 CapturedOperativeReferences is unavailable: captured references not recorded".into(), 1)]));
}

#[test]
fn hide_operative_refuses_codename_only_after_gates() {
    game::enter();
    let (fixture, _folder) = fixture("EVENT");
    let out = fixture.render(
        "fixture_hide_agent=19",
        &mut operative_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([("synthetic effect fixture_hide_agent: key fixture_hide_agent: operative codename not recorded".into(), 1)]));
    for days in ["0", "-1"] {
        let out = fixture.render(
            &format!("fixture_hide_agent={days}"),
            &mut operative_scopes(),
            Content::default(),
        );
        assert_eq!(out.text, "");
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
}

#[test]
fn harm_operative_refuses_codename_only_after_gates() {
    game::enter();
    let (fixture, _folder) = fixture("EVENT");
    let out = fixture.render(
        "fixture_injure_agent=43",
        &mut operative_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([("synthetic effect fixture_injure_agent: key fixture_injure_agent: operative codename not recorded".into(), 1)]));
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let out = fixture.render("fixture_injure_agent=43", &mut scopes, Content::default());
    assert_eq!(out.text, "");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn format98_codename_days_use_direct_and_referenced_roles() {
    game::enter();
    let (mut fixture, _folder) = fixture("EVENT");
    let codename = NameOperands {
        kind: 2,
        order: 0,
        group: b"synthetic_codes".to_vec(),
        ..Default::default()
    };
    for referenced in [false, true] {
        if referenced {
            *fixture.frame.tables[TableId::UnitLeaders.index()] =
                TableView::Complete(TableRows::UnitLeaders(vec![]));
            *fixture.frame.tables[TableId::CharacterOperatives.index()] =
                TableView::Complete(TableRows::CharacterOperatives(vec![
                    CharacterOperativeRow {
                        character: 7,
                        reference: (5_u64 << 32) | 55,
                        resolved: true,
                        category: 3,
                        name: Recorded::Value(Some(b"Agent text".to_vec())),
                        codename: Recorded::Value(Some(codename.clone())),
                        ..Default::default()
                    },
                ]));
        } else if let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *fixture.frame.tables[TableId::UnitLeaders.index()]
        {
            rows[0].kind = 3;
            rows[0].codename = Recorded::Value(Some(codename.clone()));
        }
        for (source, expected) in [
            (
                "fixture_hide_agent=19",
                "Hide Agent text/Nightjar/19 AAA",
            ),
            (
                "fixture_injure_agent=43",
                "Harm Agent text/Nightjar/43 AAA\n",
            ),
        ] {
            let out = fixture.render(source, &mut operative_scopes(), Content::default());
            assert_eq!(out.text, expected);
            assert!(out.refusals.is_empty(), "{:?}", out.refusals);
        }
        if referenced {
            if let TableView::Complete(TableRows::CharacterOperatives(rows)) =
                &mut *fixture.frame.tables[TableId::CharacterOperatives.index()]
            {
                rows[0].codename = Recorded::NotRecorded;
            }
            for (source, effect_key) in [
                (
                    "fixture_hide_agent=19",
                    "fixture_hide_agent",
                ),
                ("fixture_injure_agent=43", "fixture_injure_agent"),
            ] {
                let out = fixture.render(source, &mut operative_scopes(), Content::default());
                assert_eq!(out.text, "");
                assert_eq!(
                    out.refusals,
                    BTreeMap::from([(
                        format!(
                            "synthetic effect {effect_key}: key {}: operative codename not recorded",
                            source.split('=').next().unwrap()
                        ),
                        1
                    )])
                );
            }
        }
    }
}

#[test]
fn operative_actions_use_format98_registry_only_name() {
    game::enter();
    let (mut fixture, _folder) = fixture("EVENT");
    *fixture.frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![]));
    *fixture.frame.tables[TableId::CharacterOperatives.index()] =
        TableView::Complete(TableRows::CharacterOperatives(vec![
            CharacterOperativeRow {
                character: 7,
                reference: (5_u64 << 32) | 55,
                resolved: true,
                category: 3,
                name: Recorded::Value(Some(b"Referenced agent".to_vec())),
                ..Default::default()
            },
        ]));
    for (key, effect_key, field, expected) in [
        (
            "fixture_capture_agent",
            "fixture_capture_agent",
            "captured_by",
            "Capture Referenced agent by Beryl union AAA",
        ),
        (
            "kill_operative",
            "kill_operative",
            "killed_by",
            "Kill Referenced agent by beryl union AAA",
        ),
        (
            "turn_operative",
            "turn_operative",
            "turned_by",
            "Turn Referenced agent by beryl union AAA",
        ),
    ] {
        let source = format!("{key}={{{field}=FROM}}");
        let out = fixture.render(&source, &mut operative_scopes(), Content::default());
        assert_eq!(out.text, expected);
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
        if let TableView::Complete(TableRows::CharacterOperatives(rows)) =
            &mut *fixture.frame.tables[TableId::CharacterOperatives.index()]
        {
            rows[0].name = Recorded::NotRecorded;
        }
        let out = fixture.render(&source, &mut operative_scopes(), Content::default());
        assert_eq!(out.text, "");
        assert_eq!(
            out.refusals,
            BTreeMap::from([(
                format!(
                    "synthetic effect {effect_key}: key {key}: character 7 referenced unit leader name not recorded"
                ),
                1
            )])
        );
        if let TableView::Complete(TableRows::CharacterOperatives(rows)) =
            &mut *fixture.frame.tables[TableId::CharacterOperatives.index()]
        {
            rows[0].name = Recorded::Value(Some(b"Referenced agent".to_vec()));
        }
    }
}

#[test]
fn unit_leader_event() {
    game::enter();
    let (fixture, _folder) = fixture("EVENT");
    let lookup = |_: &[u8]| Ok(event("UNIT_TITLE"));
    let mut scopes = operative_scopes();
    let out = fixture.render(
        "fixture_commander_event={random_hours=11 id=synthetic.1 hours=7}",
        &mut scopes,
        Content {
            events: Some(&lookup),
            ..Default::default()
        },
    );
    assert_eq!(out.text, "Event Unit AAA [Root.GetTag]");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert_eq!(scopes.scopes[0].random_seed, 124);
    assert_eq!(scopes.scopes[0].random_counter, 456);
    assert_eq!(scopes.scopes[3].this, EventObject::Country(0));
    assert_eq!(scopes.scopes[3].root, Link::Itself);
    assert_eq!(scopes.scopes[3].from, Link::Scope(1));
}

#[test]
fn unit_event_conditional_titles_use_empty_scope_and_first_fallback() {
    game::enter();
    let (fixture, _folder) = fixture("EVENT");
    for (titles, expected) in [
        (
            "title={text=WRONG trigger={always=no}} title={key=UNIT_TITLE trigger={FROM={tag=AAA}}}",
            "Event Unit AAA [Root.GetTag]",
        ),
        (
            "title={text=UNIT_TITLE trigger={always=no}} title={text=WRONG trigger={always=no}}",
            "Event Unit AAA [Root.GetTag]",
        ),
    ] {
        let event = Arc::new(content_reader_events::events::Event {
            id: b"synthetic.1".to_vec(),
            titles: content_reader_parser::parser::parse(titles.as_bytes())
                .unwrap()
                .items,
            ..Default::default()
        });
        let lookup = |_: &[u8]| Ok(event.clone());
        let out = fixture.render(
            "fixture_commander_event={id=synthetic.1}",
            &mut operative_scopes(),
            Content {
                events: Some(&lookup),
                ..Default::default()
            },
        );
        assert_eq!(out.text, expected, "{:?}", out.refusals);
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
}

#[test]
fn unit_event_different_eligible_titles_have_named_counted_clock_refusal() {
    game::enter();
    let (fixture, _folder) = fixture("EVENT");
    let event = Arc::new(content_reader_events::events::Event {
        id: b"synthetic.1".to_vec(),
        titles: content_reader_parser::parser::parse(b"title=UNIT_TITLE title=OPERATIVE_TITLE")
            .unwrap()
            .items,
        ..Default::default()
    });
    let lookup = |_: &[u8]| Ok(event.clone());
    let out = fixture.render(
        "fixture_commander_event={id=synthetic.1}",
        &mut operative_scopes(),
        Content {
            events: Some(&lookup),
            ..Default::default()
        },
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([("synthetic effect fixture_commander_event: key fixture_commander_event:  event synthetic.1 has different eligible title keys; title selection needs an unrecorded high-resolution seed".into(), 1)]));
}

#[test]
fn operative_leader_event() {
    game::enter();
    let (fixture, _folder) = fixture("EFFECT");
    let events = content_reader_events::events::parse(
        b"add_namespace=synthetic operative_leader_event={major=yes option={fixture_points=7 name=CHOICE} is_triggered_only=yes desc=EXPLANATION title=OPERATIVE_TITLE id=synthetic.23}",
        "events/synthetic.txt",
        &content_reader_script::key_names::KeyNames::default(),
    );
    let lookup = |id: &[u8]| events.get(id).map(|event| Arc::new(event.clone()));
    let mut scopes = operative_scopes();
    let out = fixture.render(
        "fixture_agent_event={set_from_from=BBB id=synthetic.23}",
        &mut scopes,
        Content {
            events: Some(&lookup),
            ..Default::default()
        },
    );
    assert_eq!(out.text, "Event Operative AAA/BBB/ [Root.GetTag]");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert_eq!(scopes.scopes[0].random_seed, 124);
    assert_eq!(scopes.scopes[3].this, EventObject::Character(7));
}

#[test]
fn operative_event_set_root_evaluates_set_from_operand() {
    game::enter();
    let (fixture, _folder) = fixture("EFFECT");
    let lookup = |_: &[u8]| Ok(event("OPERATIVE_TITLE"));
    let out = fixture.render(
        "fixture_agent_event={id=synthetic.1 set_from=BBB set_from_from=CCC set_root=AAA}",
        &mut operative_scopes(),
        Content {
            events: Some(&lookup),
            ..Default::default()
        },
    );
    assert_eq!(out.text, "Event Operative BBB/CCC/BBB [Root.GetTag]");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn event_tooltips_expand_raw_text_without_localising_it_or_drawing() {
    game::enter();
    let (fixture, _folder) = fixture("EVENT");
    let lookup = |_: &[u8]| Ok(event("UNIT_TITLE"));
    for effect_key in ["fixture_commander_event", "fixture_agent_event"] {
        let mut scopes = operative_scopes();
        let out = fixture.render(
            &format!("{effect_key}={{id=synthetic.1 tooltip=\"RAW [Root.GetTag]\"}}"),
            &mut scopes,
            Content {
                events: Some(&lookup),
                ..Default::default()
            },
        );
        assert_eq!(out.text, "RAW AAA\n");
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
        assert_eq!(scopes.scopes.len(), 3);
        assert_eq!(scopes.scopes[0].random_seed, 123);
    }
}

#[test]
fn format99_free_random_operative_requires_resolved_owner_in_captured_roster() {
    game::enter();
    let (mut fixture, _folder) = fixture("EVENT");
    *fixture.frame.tables[TableId::CapturedOperativeReferences.index()] =
        TableView::Complete(TableRows::CapturedOperativeReferences(vec![
            CapturedOperativeReferenceRow {
                country: 200,
                ordinal: 0,
                operative: 55 | 5 << 32,
                owner: None,
            },
            CapturedOperativeReferenceRow {
                country: 200,
                ordinal: 1,
                operative: 55 | 6 << 32,
                owner: Some(100),
            },
        ]));
    let out = fixture.render(
        "fixture_release_agent={captured_by=FROM all=yes}",
        &mut country_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Free all \u{11}YBeryl\u{11}!");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let TableView::Complete(TableRows::CapturedOperativeReferences(rows)) =
        &mut *fixture.frame.tables[TableId::CapturedOperativeReferences.index()]
    else {
        unreachable!()
    };
    rows[1].owner = Some(300);
    let out = fixture.render(
        "fixture_release_agent={captured_by=FROM}",
        &mut country_scopes(),
        Content::default(),
    );
    assert!(out.text.is_empty() && out.refusals.is_empty());
}
