use super::*;
use replay_data::rows::*;

#[test]
fn balance_property_methods_select_the_registered_handler() {
    assert!(matches_numeric_handler("GetPowerBalanceName", 46));
    assert!(matches_numeric_handler("GetPowerBalanceModDesc", 47));
    assert!(matches_numeric_handler("GetBopTrendTextIcon", 58));
    assert!(matches_numeric_handler("getpowerbalancename", 46));
    assert!(!matches_numeric_handler("GetPowerBalanceName", 47));
    assert!(!matches_numeric_handler("GetPowerBalanceNameExtra", 46));
    assert!(!matches_numeric_handler("GetName", 46));
    assert!(!matches_numeric_handler("GetPowerBalanceName", 45));
}

fn fixture(
    bop: Option<&str>,
) -> (
    crate::catalogue::Catalogue,
    crate::catalogue::tests::Folder,
    DecodedFrame,
) {
    let (catalogue, folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/country_tags/synthetic.txt",
            "GER=\"countries/synthetic_ger.txt\" SOV=\"countries/synthetic_sov.txt\"",
        ),
        (
            "common/defines/test.lua",
            "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 } }",
        ),
        ("common/units/synthetic.txt", "sub_units={syn_unit={}}"),
        (
            "common/bop/test.txt",
            bop.unwrap_or(
                r#"balance = {
                side = { id = side range = { id = active
                    modifier = { second = 1 first = 0.1 hidden_modifier = { hidden = 1 }
                        added_modifier = { first = 9 } custom_modifier_tooltip = TIP }
                    modifier = { first = 0.25 }
                    rule = { can_create_factions = yes can_join_factions = no
                        can_send_volunteers = no desc = IGNORED override = { 12 = IGNORED } }
                    on_activate = { custom_effect_tooltip = ONE custom_effect_tooltip = ACTIVATION
                        hidden_effect = { custom_effect_tooltip = ACTIVATION }
                        effect_tooltip = { custom_effect_tooltip = NEXT } }
                    on_deactivate = { custom_effect_tooltip = DEACTIVATION }
                } }
            }"#,
            ),
        ),
        (
            "common/scripted_localisation/synthetic.txt",
            "entry={name=BalanceEntry text={localization_key=ENTRY_BODY}}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " GER:0 \"Country\"\n FIRST:0 \"First\"\n SECOND:0 \"Second\"\n",
                " TIP:0 \"[ROOT.GetTag] tip\"\n PROGRESS_SECTION_MODIFIER:0 \"Modifiers:\\n$DESC$\"\n",
                " PROGRESS_SECTION_RULE:0 \"Rules:\\n$DESC$\"\n",
                " TRIGGER_FULLFILLED_PREFIX:0 \"Yes: \"\n TRIGGER_UNFULLFILLED_PREFIX:0 \"No: \"\n",
                " CAN_CREATE_FACTIONS:0 \"Create factions\"\n CAN_JOIN_FACTIONS:0 \"Join factions\"\n",
                " ACTIVATION:0 \"[ROOT.GetTag] activated\"\n NEXT:0 \"Next effect\"\n",
                " ONE:0 \"x\"\n",
                " syn_unit:0 \"Unit\"\n SUBUNIT_BONUS_HEADER:0 \"$ICON$$NAME$\"\n",
                " LOOP:0 \"[ROOT.GetActiveRangeActivationEffect]\"\n ENTRY_BODY:0 \"[ROOT.GetActiveRangeActivationEffect]\"\n ENTRY_TOOLTIP:0 \"[BalanceEntry]\"\n",
                " SOV:0 \"Other\"\n EFFECT_CONTEXT_COUNTRY:0 \"@$TAG$$COUNTRY$\"\n",
                " POLITICS_ADD_POLITICAL_POWER:0 \"PP$VAL|0$\"\n",
                " EFFECT_ADD_ARMY_EXPERIENCE:0 \"XP$VAL|0$\"\n",
                " DEACTIVATION:0 \"[ROOT.GetTag] deactivated\"\n",
                " PROGRESS_SECTION_ACTIVATION_EFFECT:0 \"Activate:\\n$DESC$\"\n",
                " PROGRESS_SECTION_DEACTIVATION_EFFECT:0 \"Deactivate:\\n$DESC$\"\n"
            ),
        ),
    ]);
    let mut frame = crate::scope::tests::positive_frame();
    *frame.tables[TableId::PowerBalances.index()] =
        TableView::Complete(TableRows::PowerBalances(vec![PowerBalanceRow {
            balance: b"balance".to_vec(),
            active_range: b"active".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::PowerBalanceMembers.index()] =
        TableView::Complete(TableRows::PowerBalanceMembers(vec![
            PowerBalanceMemberRow {
                country: 100,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        ["first", "second", "hidden"]
            .into_iter()
            .enumerate()
            .map(|(at, key)| LexicalTokenRow {
                token: at as u32 + 1,
                literal: key.as_bytes().to_vec(),
            })
            .collect(),
    ));
    *frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(
            ["FIRST", "SECOND", "HIDDEN"]
                .into_iter()
                .enumerate()
                .map(|(at, key)| ModifierDefinitionRow {
                    index: at as u32 + 1,
                    token: at as u32 + 1,
                    name_key: key.as_bytes().to_vec(),
                    category_mask: u32::MAX,
                    precision: 2,
                    flags: 3,
                    ..Default::default()
                })
                .collect(),
        ));
    (catalogue, folder, frame)
}

#[test]
fn nested_balance_effects_keep_the_viewer_nato_preference() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder, frame) = fixture(Some(
        "balance={range={id=active on_activate={add_unit_bonus={syn_unit={}}}}}",
    ));
    let content = catalogue.scripted_content().unwrap();
    let effects = content.effects().unwrap();
    let items = installed_content_core::parser::parse(b"custom_effect_tooltip=ENTRY_TOOLTIP")
        .unwrap()
        .items;
    for nato in [false, true] {
        let text =
            ScriptedText::new(&frame, &catalogue.localisation, content, 1).with_nato_counters(nato);
        let context = crate::effect_description::Context::new(
            &text,
            &catalogue.localisation,
            0,
            "synthetic border event",
            effects.view(None),
        );
        let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
        let out = crate::effect_description::describe(&items, &context, &mut scopes);
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
        let frame = if nato { 2 } else { 1 };
        assert_eq!(
            out.text,
            format!("Activate:\n\u{13}GFX_syn_unit_icon_small|{frame} Unit\n")
        );
    }
}

#[test]
fn balance_effect_sessions_keep_nesting_active_history_and_large_stack() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder, frame) = fixture(Some(
        "balance={range={id=active on_activate={custom_effect_tooltip=LOOP}}}",
    ));
    let content = catalogue.scripted_content().unwrap();
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    let graph = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let mut scopes = ExpansionScopes::new(&graph);
    scopes.on_large_stack = true;
    scopes.fresh_depth.set(MAX_SCRIPTED_NESTING);
    let effects = installed_content_core::parser::parse(b"custom_effect_tooltip=ACTIVATION")
        .unwrap()
        .items;
    // This bounded first assertion fails safely on the old head, before recursion.
    let why = text
        .active_range_effects(&effects, &mut Vec::new(), 1, "guard", &scopes)
        .unwrap_err();
    assert!(why.contains("native stack exhaustion"), "{why}");
    scopes.fresh_depth.set(MAX_SCRIPTED_NESTING - 1);
    {
        let appended = scopes.append_description(&graph);
        assert_eq!(appended.offset, 1);
        assert_eq!(scopes.scope(0), graph.scopes[0]);
        assert_eq!(scopes.scope(1), graph.scopes[0]);
        assert_eq!(scopes.fresh_depth.get(), MAX_SCRIPTED_NESTING - 1);
        assert!(scopes.on_large_stack);
    }
    assert_eq!(scopes.scopes.borrow().len(), 1);
    let effects = installed_content_core::parser::parse(b"custom_effect_tooltip=LOOP")
        .unwrap()
        .items;
    let mut refused = Vec::new();
    assert_eq!(
        text.active_range_effects(&effects, &mut refused, 1, "guard", &scopes)
            .unwrap(),
        ""
    );
    assert_eq!(refused.len(), 1);
    assert!(
        refused[0].contains("native stack exhaustion"),
        "{:?}",
        refused
    );
    assert_eq!(scopes.fresh_depth.get(), MAX_SCRIPTED_NESTING - 1);

    let (catalogue, _folder, frame) = fixture(Some(
        "balance={range={id=active on_activate={custom_effect_tooltip=ENTRY_TOOLTIP}}}",
    ));
    let text = ScriptedText::new(
        &frame,
        &catalogue.localisation,
        catalogue.scripted_content().unwrap(),
        1,
    );
    // The entry's cursor remains in the carried arena when the effect appends a
    // fresh graph. Its repeated effective context must be recognized immediately.
    let why = text.expand("[BalanceEntry]").unwrap_err();
    assert!(why.contains("cyclic entry invocation"), "{why}");
}

#[test]
fn custom_only_balance_effects_do_not_require_world_or_environment() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder, mut frame) = fixture(None);
    *frame.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "synthetic province gap".into(),
    };
    let content = catalogue.scripted_content().unwrap();
    assert!(
        content
            .environment()
            .unwrap()
            .inputs
            .set(Err("synthetic unavailable environment".into()))
            .is_ok()
    );
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    assert_eq!(
        text.expand("[ROOT.GetActiveRangeActivationEffect]"),
        Ok("Activate:\nGER activated\nNext effect\n".into())
    );
    assert_eq!(
        text.expand("[ROOT.GetActiveRangeDeactivationEffect]"),
        Ok("Deactivate:\nGER deactivated\n".into())
    );
    assert!(text.world.get().is_none());
}

#[test]
fn balance_context_headers_use_the_viewing_country() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder, mut frame) = fixture(Some(
        "balance={range={id=active on_activate={SOV={custom_effect_tooltip=NEXT}}}}",
    ));
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment(Default::default());
    *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![]));
    if let TableView::Complete(TableRows::Politics(rows)) =
        &mut *frame.tables[TableId::Politics.index()]
    {
        rows.push(PoliticsRow {
            country: 200,
            ..Default::default()
        });
    }
    *frame.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(vec![]));
    if let TableView::Complete(TableRows::PowerBalanceMembers(members)) =
        &mut *frame.tables[TableId::PowerBalanceMembers.index()]
    {
        members[0].country = 200;
    }
    for (perspective, expected) in [
        (1, "Activate:\n\u{12}SOVOther Next effect\n"),
        (2, "Activate:\nNext effect\n"),
    ] {
        let text = ScriptedText::new(&frame, &catalogue.localisation, content, perspective);
        let graph = EventScopes::new(vec![EventScope::of(EventObject::Country(2))]);
        assert_eq!(
            text.expand_in("[ROOT.GetActiveRangeActivationEffect]", &graph),
            Ok(expected.into())
        );
    }
}

fn description(
    catalogue: &crate::catalogue::Catalogue,
    frame: &DecodedFrame,
    property: &str,
) -> Result<String, String> {
    let text = ScriptedText::new(
        frame,
        &catalogue.localisation,
        catalogue.scripted_content()?,
        1,
    );
    crate::catalogue::localised_in(
        &catalogue.localisation,
        &format!("[ROOT.{property}]"),
        &[],
        &text,
    )
}

#[test]
fn activation_uses_the_shared_numeric_and_conditional_descriptions() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder, frame) = fixture(Some(
        "balance={range={id=active on_activate={add_political_power=2 if={limit={always=no} army_experience=9} else={army_experience=3}}}}",
    ));
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    let mut frame = frame;
    *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![]));
    assert_eq!(
        description(&catalogue, &frame, "GetActiveRangeActivationEffect").unwrap(),
        "Activate:\nPP2\nXP3\n"
    );
}

#[test]
fn active_range_mod_description() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder, frame) = fixture(None);
    assert_eq!(
        description(&catalogue, &frame, "GetActiveRangeModDesc").unwrap(),
        "Modifiers:\nFirst: +25.00%\nSecond: +100.00%\nGER tip\n"
    );
}

#[test]
fn active_range_modifier_feature_gate() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder, mut frame) = fixture(None);
    for (mask, eligibility, shown) in [
        (0b1010, 1, true),
        (0b0010, 1, true),
        (0b1000, 1, true),
        (0b0001, 1, false),
        (0, 1, false),
        (0b1010, 0, false),
    ] {
        if let TableView::Complete(TableRows::ModifierDefinitions(rows)) =
            &mut *frame.tables[TableId::ModifierDefinitions.index()]
        {
            rows[0].feature_selector = 0b1010;
            rows[0].eligibility = eligibility;
        }
        *frame.tables[TableId::SimulationFeatures.index()] =
            TableView::Complete(TableRows::SimulationFeatures(vec![SimulationFeatureRow {
                dlc_mask: mask,
                ..Default::default()
            }]));
        assert_eq!(
            description(&catalogue, &frame, "GetActiveRangeModDesc").unwrap(),
            if shown {
                "Modifiers:\nFirst: +25.00%\nSecond: +100.00%\nGER tip\n"
            } else {
                "Modifiers:\nSecond: +100.00%\nGER tip\n"
            },
            "mask {mask:#x}, eligibility {eligibility}"
        );
    }
}

#[test]
fn active_range_rule_description() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder, frame) = fixture(None);
    assert_eq!(
        description(&catalogue, &frame, "GetActiveRangeRuleDesc").unwrap(),
        "Rules:\nYes: Create factions\nNo: Join factions\n"
    );
}

#[test]
fn active_range_effect_refusals_are_named_and_counted() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder, frame) = fixture(Some(
        r#"balance={range={id=active
            on_activate={unported=1 hidden_effect={other=2}
                custom_effect_tooltip="fmt|value" custom_effect_tooltip={localization_key=KEY}}
        }}"#,
    ));
    let error = description(&catalogue, &frame, "GetActiveRangeActivationEffect").unwrap_err();
    for name in [
        "3 unsupported effect descriptions",
        "unported",
        "other",
        "formatter fmt requires core registry callback",
        "bound localisation",
    ] {
        assert!(error.contains(name), "{error}");
    }
    assert_eq!(
        description(&catalogue, &frame, "GetActiveRangeDeactivationEffect").unwrap(),
        ""
    );
    // The bound-localisation reader accepts a localization_key block;
    // it is independent of the unsupported formatter callback.
    let (catalogue, _folder, frame) = fixture(Some(
        "balance={range={id=active on_activate={custom_effect_tooltip={localization_key=KEY}}}}",
    ));
    assert_eq!(
        description(&catalogue, &frame, "GetActiveRangeActivationEffect").unwrap(),
        "Activate:\nKEY\n"
    );
}

#[test]
fn active_range_missing_feature_mask_and_null_balance() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder, mut frame) = fixture(None);
    if let TableView::Complete(TableRows::ModifierDefinitions(rows)) =
        &mut *frame.tables[TableId::ModifierDefinitions.index()]
    {
        for row in &mut rows[..2] {
            row.feature_selector = 1;
            row.eligibility = 1;
        }
    }
    let error = description(&catalogue, &frame, "GetActiveRangeModDesc").unwrap_err();
    assert!(
        error.contains("2 feature-gated modifiers (first, second)")
            && error.contains("SimulationFeatures.dlc_mask")
    );
    *frame.tables[TableId::PowerBalanceMembers.index()] =
        TableView::Complete(TableRows::PowerBalanceMembers(Vec::new()));
    for property in [
        "GetActiveRangeModDesc",
        "GetActiveRangeRuleDesc",
        "GetActiveRangeActivationEffect",
        "GetActiveRangeDeactivationEffect",
    ] {
        assert_eq!(description(&catalogue, &frame, property).unwrap(), "");
    }
}
