use crate::fixtures::game;
use crate::{ScriptedEffects, effect_describer, effect_description};
use dashboard_text::{
    catalogue::{
        Catalogue,
        tests::{Folder, with_effect_describer},
    },
    scripted_text::{
        EventObject, EventScope, EventScopes, ScriptedText,
        balance_test_support::{
            BalanceEffectScopes, MAX_SCRIPTED_NESTING, description, fixture as text_fixture,
        },
    },
    triggers::{Eval, Evaluator, Scope, Scopes},
};
use replay_data::rows::*;
use replay_frame::frame::{DecodedFrame, TableView};

fn fixture(bop: Option<&str>) -> (Catalogue, Folder, DecodedFrame) {
    let (catalogue, folder, frame) = text_fixture(bop);
    (
        with_effect_describer(catalogue, effect_describer),
        folder,
        frame,
    )
}

#[test]
fn nested_balance_effects_keep_the_viewer_nato_preference() {
    game::enter();
    let (catalogue, _folder, frame) = fixture(Some(
        "balance={range={id=active on_activate={add_unit_bonus={syn_unit={}}}}}",
    ));
    let content = catalogue.scripted_content().unwrap();
    let effects = content.effects().unwrap();
    let items = content_reader_parser::parser::parse(b"custom_effect_tooltip=ENTRY_TOOLTIP")
        .unwrap()
        .items;
    for nato in [false, true] {
        let text =
            ScriptedText::new(&frame, &catalogue.localisation, content, 1).with_nato_counters(nato);
        let context = effect_description::Context::new(
            &text,
            &catalogue.localisation,
            0,
            "synthetic border event",
            effects.view(None),
        );
        let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
        let out = effect_description::describe(&items, &context, &mut scopes);
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
    game::enter();
    let (catalogue, _folder, frame) = fixture(Some(
        "balance={range={id=active on_activate={custom_effect_tooltip=LOOP}}}",
    ));
    let content = catalogue.scripted_content().unwrap();
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    let graph = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let scopes = BalanceEffectScopes::new(&graph, MAX_SCRIPTED_NESTING);
    let effects = content_reader_parser::parser::parse(b"custom_effect_tooltip=ACTIVATION")
        .unwrap()
        .items;
    let why = scopes
        .describe(&text, &effects, &mut Vec::new(), 1, "guard")
        .unwrap_err();
    assert!(why.contains("native stack exhaustion"), "{why}");
    scopes.set_depth(MAX_SCRIPTED_NESTING - 1);
    scopes.check_appended(&graph, MAX_SCRIPTED_NESTING - 1);
    let effects = content_reader_parser::parser::parse(b"custom_effect_tooltip=LOOP")
        .unwrap()
        .items;
    let mut refused = Vec::new();
    assert_eq!(
        scopes
            .describe(&text, &effects, &mut refused, 1, "guard")
            .unwrap(),
        ""
    );
    assert_eq!(refused.len(), 1);
    assert!(
        refused[0].contains("native stack exhaustion"),
        "{:?}",
        refused
    );
    assert_eq!(scopes.depth(), MAX_SCRIPTED_NESTING - 1);

    let (catalogue, _folder, frame) = fixture(Some(
        "balance={range={id=active on_activate={custom_effect_tooltip=ENTRY_TOOLTIP}}}",
    ));
    let text = ScriptedText::new(
        &frame,
        &catalogue.localisation,
        catalogue.scripted_content().unwrap(),
        1,
    );
    let why = text.expand("[BalanceEntry]").unwrap_err();
    assert!(why.contains("cyclic entry invocation"), "{why}");
}

#[test]
fn custom_only_balance_effects_do_not_require_world_or_environment() {
    game::enter();
    let (catalogue, _folder, mut frame) = fixture(None);
    *frame.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "synthetic province gap".into(),
    };
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment_error("synthetic unavailable environment".into());
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    assert_eq!(
        text.expand("[ROOT.GetActiveRangeActivationEffect]"),
        Ok("Activate:\nGER activated\nNext effect\n".into())
    );
    assert_eq!(
        text.expand("[ROOT.GetActiveRangeDeactivationEffect]"),
        Ok("Deactivate:\nGER deactivated\n".into())
    );
    assert!(!text.synthetic_world_loaded());
}

#[test]
fn balance_context_headers_use_the_viewing_country() {
    game::enter();
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

#[test]
fn activation_uses_the_shared_numeric_and_conditional_descriptions() {
    game::enter();
    let (catalogue, _folder, frame) = fixture(Some(
        "balance={range={id=active on_activate={add_political_power=2 if={limit={always=no} fixture_points=9} else={fixture_points=3}}}}",
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
fn active_range_effect_refusals_are_named_and_counted() {
    game::enter();
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
    game::enter();
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
            && error.contains("dlc mask of simulation features")
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

#[test]
fn balance_nested_description_inherits_the_active_variable_session() {
    game::enter();
    let (catalogue, _folder) = crate::fixtures::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 } }",
        ),
        (
            "common/bop/test.txt",
            "balance={range={id=active on_activate={if={limit={check_variable={temp_var:x=7}} custom_effect_tooltip=INHERITED} else={custom_effect_tooltip=FRESH}}}}",
        ),
        (
            "localisation/test_l_english.yml",
            "\u{feff}l_english:\n INHERITED:0 \"Inherited\"\n FRESH:0 \"Fresh\"\n PROGRESS_SECTION_ACTIVATION_EFFECT:0 \"Activate:\\n$DESC$\"\n",
        ),
    ]);
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment(Default::default());
    let mut frame = dashboard_text::scripted_text::complete_test_frame();
    *frame.tables[TableId::PowerBalances.index()] =
        TableView::Complete(TableRows::from_rows::<tables::PowerBalances>(vec![
            replay_data::rows::PowerBalanceRow {
                balance: b"balance".to_vec(),
                active_range: b"active".to_vec(),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::PowerBalanceMembers.index()] =
        TableView::Complete(TableRows::from_rows::<tables::PowerBalanceMembers>(vec![
            replay_data::rows::PowerBalanceMemberRow {
                country: 100,
                ..Default::default()
            },
        ]));
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    let graph = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let evaluator = Evaluator::new(text.world().unwrap(), None);
    let caller = Scopes::of(Scope::Country(1));
    evaluator.begin_description();
    let set = content_reader_script::triggers::read(
        b"set_temp_variable={x=7}",
        &content_reader_script::key_names::KeyNames::default(),
    )
    .items;
    assert_eq!(evaluator.evaluate(&set, caller), Eval::True);
    assert_eq!(
        text.expand_in_session(
            "[ROOT.GetActiveRangeActivationEffect]",
            &graph,
            0,
            &evaluator
        ),
        Ok("Activate:\nInherited\n".into())
    );
    let check = content_reader_script::triggers::read(
        b"check_variable={temp_var:x=7}",
        &content_reader_script::key_names::KeyNames::default(),
    )
    .items;
    assert_eq!(evaluator.evaluate(&check, caller), Eval::True);
    evaluator.end_description(caller).unwrap();
    assert_eq!(
        text.expand("[ROOT.GetActiveRangeActivationEffect]"),
        Ok("Activate:\nFresh\n".into())
    );
}

#[test]
fn balance_modifier_description_derives_from_sources_and_localised_static_keys() {
    game::executable().pinned_modifiers([
        (engine_executable::modifier_keys::POWER_BALANCE_DAILY, "fixture_balance_daily"),
        (engine_executable::modifier_keys::POWER_BALANCE_WEEKLY, "fixture_balance_weekly"),
    ]).enter();
    let (catalogue, _folder) = crate::fixtures::folder_catalogue(&[
        (
            "common/defines/00_defines.lua",
            "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 } }",
        ),
        (
            "localisation/replace/extra_l_english.yml",
            "\u{feff}l_english:\n SOURCE:0 \"Source\"\n TEST:0 \"[ROOT.GetPowerBalanceModDesc]\"\n",
        ),
        (
            "common/modifiers/00_balance.txt",
            "SOURCE = { fixture_balance_daily = 0.01234 fixture_balance_weekly = -0.02345 }\nZERO = { fixture_balance_daily = 0 }\nEMPTY = { }\n",
        ),
    ]);
    let scripted = catalogue.scripted_content().unwrap();
    assert!(!scripted.effects_read());
    let rendered = |frame: &DecodedFrame| {
        let context = ScriptedText::new(frame, &catalogue.localisation, scripted, 1);
        crate::catalogue::localised_in(&catalogue.localisation, "TEST", &[], &context)
    };
    fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
        *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
    }
    let mut f = dashboard_text::scripted_text::complete_test_frame();
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *f.tables[TableId::CountryAppearance.index()]
    {
        for row in rows {
            row.original_tag_reference = Recorded::Value(Some(0));
            row.original_tag = Recorded::Value(Some(Vec::new()));
        }
    }
    set::<tables::PowerBalances>(
        &mut f,
        vec![PowerBalanceRow {
            ordinal: 0,
            ..Default::default()
        }],
    );
    set::<tables::PowerBalanceMembers>(
        &mut f,
        vec![PowerBalanceMemberRow {
            country: 100,
            ..Default::default()
        }],
    );
    set::<tables::ModifierDefinitions>(
        &mut f,
        [
            (engine_executable::modifier_keys::POWER_BALANCE_DAILY, 7_001),
            (
                engine_executable::modifier_keys::POWER_BALANCE_WEEKLY,
                7_002,
            ),
        ]
        .into_iter()
        .map(|(index, token)| ModifierDefinitionRow {
            index,
            token,
            flags: 3,
            precision: 2,
            ..Default::default()
        })
        .collect(),
    );
    set::<tables::LexicalTokens>(
        &mut f,
        vec![
            LexicalTokenRow {
                token: 7_001,
                literal: b"fixture_balance_daily".to_vec(),
            },
            LexicalTokenRow {
                token: 7_002,
                literal: b"fixture_balance_weekly".to_vec(),
            },
        ],
    );
    set::<tables::PowerBalanceModifierBreakdowns>(
        &mut f,
        vec![PowerBalanceModifierBreakdownRow {
            record: 0,
            sources: vec![b"SOURCE".to_vec(), b"EMPTY".to_vec(), b"ZERO".to_vec()],
            daily: Vec::new(),
            weekly: Vec::new(),
        }],
    );
    assert_eq!(rendered(&f).unwrap(), "Source: +1.23%\nSource: -2.34%\n");
    assert!(scripted.effects_read());
    let witness = vec![
        ModifierBreakdownNode {
            children: 1,
            value: 0,
            name: Vec::new(),
        },
        ModifierBreakdownNode {
            children: 0,
            value: 99_999,
            name: b"SOURCE".to_vec(),
        },
    ];
    set::<tables::PowerBalanceModifierBreakdowns>(
        &mut f,
        vec![PowerBalanceModifierBreakdownRow {
            record: 0,
            sources: vec![b"SOURCE".to_vec(), b"EMPTY".to_vec(), b"ZERO".to_vec()],
            daily: witness.clone(),
            weekly: witness,
        }],
    );
    assert_eq!(rendered(&f).unwrap(), "Source: +1.23%\nSource: -2.34%\n");
    f.tables[TableId::PowerBalanceModifierBreakdowns.index()] = TableView::Unavailable {
        reason: "missing source capture".into(),
    }
    .into();
    assert!(
        rendered(&f)
            .unwrap_err()
            .contains("PowerBalanceModifierBreakdowns")
    );
    set::<tables::PowerBalanceMembers>(&mut f, Vec::new());
    assert_eq!(rendered(&f).unwrap(), "");
}
