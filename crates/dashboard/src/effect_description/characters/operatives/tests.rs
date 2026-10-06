use super::*;
use crate::catalogue::tests::{Folder, folder_catalogue};
use crate::effect_description::tests::Fixture;
use replay::{frame::TableView, rows::*};

fn fixture(event_argument: &str) -> (Fixture, Folder) {
    let mut fixture = Fixture::new();
    let localisation = format!(
        concat!(
            "\u{feff}l_english:\n",
            " AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n CCC:0 \"Gamma\"\n",
            " AAA_DEF:0 \"alpha union\"\n BBB_DEF:0 \"beta union\"\n CCC_DEF:0 \"gamma union\"\n",
            " EFFECT_CAPTURE_OPERATIVE:0 \"Capture $OPERATIVE$ by $CAPTURED_BY$ [Root.GetTag]\"\n",
            " EFFECT_KILL_OPERATIVE:0 \"Kill $OPERATIVE$ by $KILLED_BY$ [Root.GetTag]\"\n",
            " EFFECT_TURN_OPERATIVE:0 \"Turn $OPERATIVE$ by $TURNED_BY$ [Root.GetTag]\"\n",
            " EFFECT_FORCE_OPERATIVE_INTO_HIDING:0 \"Hide $NAME$/$CODENAME$/$DAYS|0$ [Root.GetTag]\"\n",
            " EFFECT_HARM_OPERATIVE:0 \"Harm $NAME$/$CODENAME$/$DAYS|0$ [Root.GetTag]\"\n",
            " EFFECT_COUNTRY_EVENT:0 \"Event ${}$ [Root.GetTag]\"\n",
            " UNIT_TITLE:0 \"Unit [From.GetTag]\"\n",
            " OPERATIVE_TITLE:0 \"Operative [From.GetTag]/[From.From.GetTag]/[Root.GetTag]\"\n",
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

fn event(key: &str) -> Arc<installed_content::events::Event> {
    Arc::new(installed_content::events::Event {
        id: b"synthetic.1".to_vec(),
        titles: installed_content::parser::parse(format!("title={key}").as_bytes())
            .unwrap()
            .items,
        ..Default::default()
    })
}

// common/on_actions/05_lar_on_actions.txt:47, with synthetic country.
#[test]
fn capture_operative() {
    let (fixture, _folder) = fixture("EVENT");
    let out = fixture.render(
        "capture_operative={captured_by=FROM}",
        &mut operative_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Capture Agent text by Beta union AAA");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

// common/on_actions/05_lar_on_actions.txt:52.
#[test]
fn kill_operative() {
    let (fixture, _folder) = fixture("EVENT");
    let out = fixture.render(
        "kill_operative={killed_by=ROOT}",
        &mut operative_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Kill Agent text by alpha union AAA");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

// common/on_actions/05_lar_on_actions.txt:107.
#[test]
fn turn_operative() {
    let (fixture, _folder) = fixture("EVENT");
    let out = fixture.render(
        "turn_operative={turned_by=ROOT}",
        &mut operative_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Turn Agent text by alpha union AAA");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn explicit_operative_uses_native_country_roster_and_exact_leader_name() {
    let (fixture, _folder) = fixture("EVENT");
    let out = fixture.render(
        "capture_operative={operative=synthetic_other_agent captured_by=FROM}",
        &mut operative_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Capture Other agent by Beta union AAA");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

// common/operations/00_operations.txt:45.
#[test]
fn free_random_operative_refuses_the_missing_captured_reference_roster() {
    let (fixture, _folder) = fixture("EVENT");
    let out = fixture.render(
        "free_random_operative={captured_by=FROM all=yes}",
        &mut operative_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([("synthetic effect CFreeRandomOperativeEfect: key free_random_operative: captured operative references (CIntelligenceAgency+0x108/+0x114, 140FC7860) are unrecorded".into(), 1)]));
}

// common/on_actions/05_lar_on_actions.txt:57.
#[test]
fn hide_operative_refuses_codename_only_after_native_gates() {
    let (fixture, _folder) = fixture("EVENT");
    let out = fixture.render(
        "force_operative_leader_into_hiding=28",
        &mut operative_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([("synthetic effect CForceOperativeLeaderIntoHiding: key force_operative_leader_into_hiding: operative codename (COperativeLeader+0x1030, 140C06040) not recorded".into(), 1)]));
    for days in ["0", "-1"] {
        let out = fixture.render(
            &format!("force_operative_leader_into_hiding={days}"),
            &mut operative_scopes(),
            Content::default(),
        );
        assert_eq!(out.text, "");
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
}

// common/on_actions/05_lar_on_actions.txt:171.
#[test]
fn harm_operative_refuses_codename_only_after_native_gates() {
    let (fixture, _folder) = fixture("EVENT");
    let out = fixture.render(
        "harm_operative_leader=90",
        &mut operative_scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([("synthetic effect CHarmOperativeEffect: key harm_operative_leader: operative codename (COperativeLeader+0x1030, 140C06040) not recorded".into(), 1)]));
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let out = fixture.render("harm_operative_leader=90", &mut scopes, Content::default());
    assert_eq!(out.text, "");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

// 140380AB0/140382790 substitute NAME, CODENAME, DAYS then expand;
// only harm appends LF. Both direct and registry-only roles carry operands.
#[test]
fn format98_codename_days_use_direct_and_referenced_roles() {
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
                "force_operative_leader_into_hiding=28",
                "Hide Agent text/Nightjar/28 AAA",
            ),
            (
                "harm_operative_leader=90",
                "Harm Agent text/Nightjar/90 AAA\n",
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
            for (source, class) in [
                (
                    "force_operative_leader_into_hiding=28",
                    "CForceOperativeLeaderIntoHiding",
                ),
                ("harm_operative_leader=90", "CHarmOperativeEffect"),
            ] {
                let out = fixture.render(source, &mut operative_scopes(), Content::default());
                assert_eq!(out.text, "");
                assert_eq!(
                    out.refusals,
                    BTreeMap::from([(
                        format!(
                            "synthetic effect {class}: key {}: operative codename (COperativeLeader+0x1030, 140C06040) not recorded",
                            source.split('=').next().unwrap()
                        ),
                        1
                    )])
                );
            }
        }
    }
}

// 14037AFB0/1403834B0/140398B70 all copy the selected leader's +0x40.
#[test]
fn operative_actions_use_format98_registry_only_name() {
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
    for (key, class, field, expected) in [
        (
            "capture_operative",
            "CCaptureOperativeEffect",
            "captured_by",
            "Capture Referenced agent by Beta union AAA",
        ),
        (
            "kill_operative",
            "CKillOperativeEffect",
            "killed_by",
            "Kill Referenced agent by beta union AAA",
        ),
        (
            "turn_operative",
            "CTurnOperativeLeaderEffect",
            "turned_by",
            "Turn Referenced agent by beta union AAA",
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
                    "synthetic effect {class}: key {key}: character 7 referenced unit leader name (+0x40, 140C16FB0) not recorded"
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

// common/decisions/ETH.txt:96; fresh scope's FROM is caller ROOT.
#[test]
fn unit_leader_event() {
    let (fixture, _folder) = fixture("EVENT");
    let lookup = |_: &[u8]| Ok(event("UNIT_TITLE"));
    let mut scopes = operative_scopes();
    let out = fixture.render(
        "unit_leader_event={id=synthetic.1 hours=2 random_hours=4}",
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
fn unit_event_conditional_titles_use_native_empty_scope_and_first_fallback() {
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
        let event = Arc::new(installed_content::events::Event {
            id: b"synthetic.1".to_vec(),
            titles: installed_content::parser::parse(titles.as_bytes()).unwrap().items,
            ..Default::default()
        });
        let lookup = |_: &[u8]| Ok(event.clone());
        let out = fixture.render(
            "unit_leader_event={id=synthetic.1}",
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
    let (fixture, _folder) = fixture("EVENT");
    let event = Arc::new(installed_content::events::Event {
        id: b"synthetic.1".to_vec(),
        titles: installed_content::parser::parse(b"title=UNIT_TITLE title=OPERATIVE_TITLE")
            .unwrap()
            .items,
        ..Default::default()
    });
    let lookup = |_: &[u8]| Ok(event.clone());
    let out = fixture.render(
        "unit_leader_event={id=synthetic.1}",
        &mut operative_scopes(),
        Content {
            events: Some(&lookup),
            ..Default::default()
        },
    );
    assert_eq!(out.text, "");
    assert_eq!(out.refusals, BTreeMap::from([("synthetic effect CUnitLeaderEventEffect: key unit_leader_event:  event synthetic.1 has different eligible title keys; 14116C180 selects with an unrecorded high-resolution seed".into(), 1)]));
}

// common/decisions/_debug_decisions.txt:666; synthetic event and tags.
#[test]
fn operative_leader_event() {
    let (fixture, _folder) = fixture("EFFECT");
    let lookup = |_: &[u8]| Ok(event("OPERATIVE_TITLE"));
    let mut scopes = operative_scopes();
    let out = fixture.render(
        "operative_leader_event={id=synthetic.1 set_from_from=BBB}",
        &mut scopes,
        Content {
            events: Some(&lookup),
            ..Default::default()
        },
    );
    // SetUnitLeader 14052EFD0 installs the caster character but clears the
    // carried country, so fresh ROOT.GetTag is empty.
    assert_eq!(out.text, "Event Operative AAA/BBB/ [Root.GetTag]");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert_eq!(scopes.scopes[0].random_seed, 124);
    assert_eq!(scopes.scopes[3].this, EventObject::Character(7));
}

// _debug_decisions.txt:666 references the operative-event declaration in
// LaR_agent_events.txt:68. The content ID and country are synthetic.
#[test]
fn operative_leader_event_renders_title_from_native_event_catalogue() {
    let (fixture, _folder) = fixture("EFFECT");
    let events = installed_content::events::parse(
        b"add_namespace=synthetic operative_leader_event={id=synthetic.5 title=OPERATIVE_TITLE desc=DESCRIPTION is_triggered_only=yes major=no option={name=OPTION add_unit_leader_trait=synthetic_trait}}",
        "events/synthetic.txt",
        &installed_content::key_names::KeyNames::default(),
    );
    let lookup = |id: &[u8]| events.get(id).map(|event| Arc::new(event.clone()));
    let out = fixture.render(
        "operative_leader_event={id=synthetic.5 set_from_from=BBB}",
        &mut operative_scopes(),
        Content {
            events: Some(&lookup),
            ..Default::default()
        },
    );
    assert_eq!(out.text, "Event Operative AAA/BBB/ [Root.GetTag]");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn operative_event_set_root_evaluates_set_from_operand_as_native_does() {
    let (fixture, _folder) = fixture("EFFECT");
    let lookup = |_: &[u8]| Ok(event("OPERATIVE_TITLE"));
    let out = fixture.render(
        "operative_leader_event={id=synthetic.1 set_from=BBB set_from_from=CCC set_root=AAA}",
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
    let (fixture, _folder) = fixture("EVENT");
    let lookup = |_: &[u8]| Ok(event("UNIT_TITLE"));
    for class in ["unit_leader_event", "operative_leader_event"] {
        let mut scopes = operative_scopes();
        let out = fixture.render(
            &format!("{class}={{id=synthetic.1 tooltip=\"RAW [Root.GetTag]\"}}"),
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
