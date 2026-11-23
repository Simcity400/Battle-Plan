use super::*;
use replay_data::{
    rows::FactionMemberRow, rows::FactionRow, rows::Recorded, rows::ReservedCountryNamingRow,
};

fn faction_frame() -> DecodedFrame {
    let mut frame = frame();
    for id in TableId::ALL {
        if matches!(&*frame.tables[id.index()], TableView::Unavailable { .. }) {
            *frame.tables[id.index()] = TableView::Complete(TableRows::empty(id));
        }
    }
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"ZZ0".to_vec(),
                faction: Recorded::Value(Some(5)),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 100,
                faction: 5,
                ordinal: 1,
            },
            FactionMemberRow {
                country: 0,
                faction: 5,
                ordinal: 0,
            },
        ]));
    *frame.tables[TableId::Factions.index()] =
        TableView::Complete(TableRows::Factions(vec![FactionRow {
            id: 5,
            name: b"Exact literal faction".to_vec(),
            template_token: None,
            ..Default::default()
        }]));
    frame
}

#[test]
fn actual_zero_text_leader_and_null_faction_remain_distinct() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder) = catalogue(SCRIPTED, " NO_FACTION:0 \"No faction\"\n");
    let content = catalogue.scripted_content().unwrap();
    let mut frame = faction_frame();
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    assert_eq!(
        text.expand("[ROOT.FactionLeader.GetTag]").as_deref(),
        Ok("ZZ0")
    );
    assert_eq!(text.faction_name(0).as_deref(), Ok("Exact literal faction"));
    let graph = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let scopes = ExpansionScopes::new(&graph);
    let mut active = 0;
    let mut event_active = true;
    assert_eq!(
        text.promote(
            Object::Faction(None),
            "Leader",
            &scopes,
            &mut active,
            &mut event_active
        ),
        Ok(Object::Faction(None))
    );
    assert_eq!(
        text.promote(
            Object::Faction(Some(5)),
            "Leader",
            &scopes,
            &mut active,
            &mut event_active
        ),
        Ok(Object::Country(0))
    );
    drop(text);
    drop(scopes);
    *frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "null faction does not demand members".into(),
    };
    if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *frame.tables[TableId::ReservedCountryNaming.index()]
    {
        rows[0].faction = Recorded::Value(Some(0));
    }
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    let scopes = ExpansionScopes::new(&graph);
    assert_eq!(text.faction_name(0).as_deref(), Ok("No faction"));
    assert_eq!(
        text.promote(
            Object::Country(0),
            "FactionLeader",
            &scopes,
            &mut active,
            &mut event_active
        ),
        Ok(Object::Country(0))
    );
}

#[test]
fn fresh_entry_binding_of_actual_zero_obeys_each_trigger_mask_and_keeps_prev() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        ("localisation/mod_l_english.yml", LOCALISATION),
        (
            "localisation/replace/extra_l_english.yml",
            "\u{feff}l_english:\n YES:0 \"selected\"\n NO:0 \"rejected\"\n",
        ),
        (
            "common/defines/00_defines.lua",
            "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 } }",
        ),
        (
            "common/scripted_triggers/mod.txt",
            "Masked = { always = yes }",
        ),
        (
            "common/scripted_localisation/mod.txt",
            r#"
            a = { name = Empty text = { trigger = {} localization_key = YES } }
            a = { name = Always text = { trigger = { always = yes } localization_key = YES } }
            a = { name = Context text = { trigger = { THIS = { always = yes } } localization_key = YES } text = { localization_key = NO } }
            a = { name = Scripted text = { trigger = { Masked = yes } localization_key = YES } text = { localization_key = NO } }
            a = { name = ScriptedNo text = { trigger = { Masked = no } localization_key = YES } text = { localization_key = NO } }
            a = { name = Country text = { trigger = { has_war = yes } localization_key = YES } text = { localization_key = NO } }
            a = { name = Previous text = { trigger = { PREV = { tag = GER } ROOT = { tag = GER } FROM = { tag = SOV } } localization_key = YES } text = { localization_key = NO } }
        "#,
        ),
    ]);
    let content = catalogue.scripted_content().unwrap();
    let mut scripted = content_reader_key_names::scripted_triggers::read(&content.layers.layers).unwrap();
    scripted
        .templates
        .get_mut(b"masked".as_slice())
        .unwrap()
        .body = Err("masked scripted body must not be reached".into());
    assert!(
        content
            .environment()
            .unwrap()
            .inputs
            .set(Ok(entries::EnvironmentInputs::synthetic().with_triggers(
                content_reader_script::trigger_keys::TriggerNames {
                    scripted,
                    ..Default::default()
                }
            )))
            .is_ok()
    );
    let mut frame = faction_frame();
    *frame.tables[TableId::WarRelations.index()] = TableView::Unavailable {
        reason: "masked country operand must not be reached".into(),
    };
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    let graph = EventScopes::new(vec![
        EventScope {
            from: Link::Scope(1),
            ..EventScope::of(EventObject::Country(1))
        },
        EventScope::of(EventObject::Country(2)),
    ]);
    for (entry, expected) in [
        ("Empty", "selected"),
        ("Always", "selected"),
        ("Context", "rejected"),
        ("Scripted", "rejected"),
        ("ScriptedNo", "selected"),
        ("Country", "rejected"),
        ("Previous", "selected"),
    ] {
        let expression = format!("[THIS.FactionLeader.{entry}]");
        assert_eq!(
            text.expand_in(&expression, &graph).as_deref(),
            Ok(expected),
            "{entry}"
        );
    }
    for entry in ["Scripted", "ScriptedNo"] {
        let expression = format!("[THIS.{entry}]");
        assert!(
            text.expand_in(&expression, &graph)
                .unwrap_err()
                .contains("masked scripted body must not be reached"),
            "{entry}: a populated country scope must retain the body refusal"
        );
    }
    assert_eq!(
        text.expand_in("[THIS.FactionLeader.GetTag]", &graph)
            .as_deref(),
        Ok("ZZ0")
    );
}
