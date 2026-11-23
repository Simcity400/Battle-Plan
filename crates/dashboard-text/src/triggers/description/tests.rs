use super::*;
use crate::{
    catalogue::tests::folder_catalogue,
    triggers::{Combat, ContentFacts, Eval, World},
};
use content_reader_script::trigger_keys::TriggerNames;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::frame::TableView;
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Side {
    calls: Mutex<Vec<String>>,
}

impl Combat for Side {
    fn country(&self) -> Result<u32, crate::triggers::Failure> {
        Ok(0)
    }

    fn trigger(&self, name: &str, item: &Item, _: Scope, evaluator: &Evaluator) -> Option<Eval> {
        self.calls.lock().unwrap().push(name.to_owned());
        match name {
            "is_attacker" => Some(Eval::from_bool(trigger_boolean(item).unwrap())),
            "frontage_full" | "has_reserves" => {
                Some(Eval::from_bool(!trigger_boolean(item).unwrap()))
            }
            "has_tactic" | "is_fighting_in_strategic_region" => Some(Eval::True),
            "phase" | "skill_advantage" | "hardness" | "has_artillery_ratio" => Some(Eval::True),
            "skill" => Some(Eval::from_bool(super::super::compare(
                300_000,
                evaluator
                    .required(
                        item.value.scalar().unwrap(),
                        item.quoted(),
                        false,
                        Scopes::combat(),
                    )
                    .unwrap(),
                item.operator,
                false,
            ))),
            _ => None,
        }
    }

    fn category_share(&self, _: u64) -> Result<i64, crate::triggers::Failure> {
        Err("no category operand in fixture".into())
    }
}

fn run(source: &str, scripted: &str, entries: &str, extra: &str) -> Result<String, String> {
    run_at(source, scripted, entries, extra, None)
}

fn run_at(source: &str, scripted: &str, entries: &str, extra: &str, at: Option<(usize, u8)>) -> Result<String, String> {
    let (catalogue, _folder) = folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        ("common/scripted_triggers/test.txt", scripted),
        ("common/scripted_localisation/test.txt", entries),
        (
            "localisation/test_l_english.yml",
            &format!(
                concat!(
                    "\u{feff}l_english:\n",
                    " TRIGGER_FULLFILLED_PREFIX:0 \"[yes]\"\n TRIGGER_UNFULLFILLED_PREFIX:0 \"[no]\"\n",
                    " TRIGGER_AND_STARTS:0 \"All\"\n TRIGGER_OR_STARTS:0 \"Any\"\n",
                    " IS_ATTACKER:0 \"Attack\"\n IS_NOT_ATTACKER:0 \"Defend\"\n",
                    " TRIGGER_IS_FRONTAGE_FULL:0 \"Full\"\n TRIGGER_IS_NOT_FRONTAGE_FULL:0 \"Space\"\n",
                    " TRIGGER_HAS_RESERVES:0 \"Reserve\"\n TRIGGER_DOES_NOT_HAVE_RESERVES:0 \"No reserve\"\n",
                    " PHASE_DEFAULT:0 \"Fixture idle phase\"\n opening:0 \"Opening\"\n IS_IN_PHASE:0 \"In $PHASE$\"\n",
                    " TRIGGER_SKILL_ADVANTAGE_GREATER_THEN:0 \"Advantage $VALUE|0$\"\n",
                    " TRIGGER_HARDNESS_GREATER_THAN:0 \"Hardness $VALUE|%0$\"\n",
                    " TRIGGER_MORE_THAN_ARTILLERY_RATIO:0 \"Artillery $NUM|%0$\"\n",
                    " TRIGGER_COMPARE_LEVEL:0 \"Level\"\n TRIGGER_MORE_THAN:0 \"above\"\n TRIGGER_AT_MOST:0 \"at most\"\n",
                    " TRIGGER_TYPE_COMPARISON_VALUE:0 \"$TYPE$ $COMPARISON$ $VALUE$\"\n{}"
                ),
                extra
            ),
        ),
    ]);
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment(Default::default());
    let mut frame = crate::scope::tests::frame();
    for id in TableId::ALL {
        if matches!(&*frame.tables[id.index()], TableView::Unavailable { .. }) {
            *frame.tables[id.index()] = TableView::Complete(TableRows::empty(id));
        }
    }
    let names = TriggerNames {
        scripted: content_reader_key_names::scripted_triggers::read(&content.layers.layers).unwrap(),
        counts: Some(Default::default()),
    };
    let dlcs = BTreeSet::new();
    let aliases = Arc::default();
    let roles = crate::triggers::LeaderRoles::new(
        content_reader_key_names::characters::leader_templates(&content.layers.layers).unwrap(),
    );
    let mut facts = ContentFacts::content_only(&dlcs, &names, &aliases);
    facts.leader_roles = Ok(&roles);
    let world = World::read(&frame, facts);
    let side = Side::default();
    let evaluator = Evaluator::new(&world, Some(&side));
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 0);
    let items = content_reader_parser::parser::parse(source.as_bytes())
        .unwrap()
        .items;
    let mut refusals = BTreeMap::new();
    let (localisation, phases, tactic) = (&catalogue.localisation, &[b"opening".to_vec()], &|name: &[u8]| -> Result<Vec<u8>, String> { Ok(name.to_vec()) });
    let result = match at {
        None => describe(&items, &evaluator, &text, localisation, Scopes::combat(), phases, tactic, &mut refusals),
        Some((indent, mode)) => {
            describe_at(&items, &evaluator, &text, localisation, Scopes::combat(), phases, tactic, indent, mode, &mut refusals)
        }
    };
    assert_eq!(
        evaluator.description_depth.get(),
        0,
        "a refusal must close the variable context"
    );
    match result {
        Ok(text) if !refusals.is_empty() => Err(format!("{text} {refusals:?}")),
        result => result,
    }
}

#[test]
fn groups_share_evaluation_and_reset_descendant_mode() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_eq!(
        run(
            "is_attacker=yes OR={is_attacker=yes frontage_full=yes} NOT={has_reserves=no}",
            "",
            "",
            ""
        )
        .unwrap(),
        "[yes]Attack\n[yes]Any:\n   Attack\n   Full\n[no]Reserve\n"
    );
    assert_eq!(
        run("SYN_CHECK=no", "SYN_CHECK={is_attacker=yes}", "", "").unwrap(),
        "[no]Defend\n"
    );
    assert_eq!(
        run("SYN_CHECK={arg=yes}", "SYN_CHECK={is_attacker=yes}", "", "").unwrap(),
        "[no]Defend\n"
    );
}

#[test]
fn an_indented_conjunction_heads_its_children_without_checks_in_mode_zero() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let source = "is_attacker=yes OR={is_attacker=yes frontage_full=yes} NOT={has_reserves=no}";
    assert_eq!(
        run_at(source, "", "", "", Some((1, 0))).unwrap(),
        "   All:
      Attack
      Any:
         Attack
         Full
      Reserve
"
    );
    assert_eq!(
        run_at(source, "", "", "", Some((2, 0))).unwrap(),
        "      All:
         Attack
         Any:
            Attack
            Full
         Reserve
"
    );
    assert_eq!(run_at("is_attacker=yes", "", "", "", Some((2, 0))).unwrap(), "      Attack
");
    assert_eq!(run_at(source, "", "", "", Some((0, 1))), run(source, "", "", ""));
}

#[test]
fn operands_use_integer_fixed5_and_stored_phase_names() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_eq!(
        run(
            "phase=0 skill_advantage>2 hardness>0.5 has_artillery_ratio>0.1 skill>2 NOT={skill>2}",
            "",
            "",
            ""
        )
        .unwrap(),
        "[yes]In Opening\n[yes]Advantage 2\n[yes]Hardness 50%\n[yes]Artillery 10%\n[yes]Level above 2\n[no]Level at most 2\n"
    );
}

#[test]
fn conditional_entry_keeps_combat_world_and_tls_across_large_stack_handoff() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut entries = String::new();
    let mut loc = String::new();
    for index in 0..10 {
        entries.push_str(&format!(
            "entry={{name=Hop{index} text={{localization_key=HOP{index}}}}}\n"
        ));
        loc.push_str(&format!(" HOP{index}:0 \"[Hop{}]\"\n", index + 1));
    }
    entries.push_str("entry={name=Hop10 text={trigger={is_attacker=yes check_variable={temp_var:x=7}} localization_key=YES} text={localization_key=NO}}");
    loc.push_str(" YES:0 \"Correct side\"\n NO:0 \"Wrong side\"\n TIP:0 \"[Hop0]\"\n");
    assert_eq!(run("if={limit={set_temp_variable={x=7}} custom_trigger_tooltip={tooltip=TIP is_attacker=yes}}", "", &entries, &loc).unwrap(), "[yes]Correct side\n");
}

#[test]
fn empty_combat_scope_heading_and_counted_refusal_do_not_invent_country() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_eq!(
        run("ROOT={is_attacker=yes}", "", "", "").unwrap(),
        "[yes]: Attack\n"
    );
    assert_eq!(
        run("NOT={custom_trigger_tooltip={is_attacker=yes}}", "", "", "").unwrap(),
        ""
    );
    let refused = run("is_attacker=yes night=yes has_reserves=no", "", "", "").unwrap_err();
    assert!(refused.contains("night description"), "{refused}");
    assert!(
        refused.contains("[yes]Attack\n") && refused.contains("[yes]No reserve\n"),
        "{refused}"
    );
}

#[test]
fn refused_conditional_taints_later_temp_read_but_keeps_pure_siblings() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let result = run(
        "if={limit={set_temp_variable={x=7}} is_attacker=yes} if={limit={night=yes} set_temp_variable={x=8}} custom_trigger_tooltip={tooltip=TIP is_attacker=yes} has_reserves=no",
        "",
        "entry={name=Stale text={trigger={check_variable={temp_var:x=7}} localization_key=YES} text={localization_key=NO}}",
        " TIP:0 \"[Stale]\"\n YES:0 \"Stale kept\"\n NO:0 \"Stale lost\"\n",
    ).unwrap_err();
    assert!(
        result.contains("[yes]Attack\n") && result.contains("[yes]No reserve\n"),
        "{result}"
    );
    assert!(
        !result.contains("Stale kept") && !result.contains("Stale lost"),
        "{result}"
    );
    assert!(result.contains("session is uncertain"), "{result}");
}

#[test]
fn new_assignment_trigger_descriptions_use_native_arguments_and_inversion() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let localisation = concat!(
        " TRIGGER_FIGHTING_IN_STRATEGIC_REGION:0 \"Region $REGION$\"\n",
        " TRIGGER_NOT_FIGHTING_IN_STRATEGIC_REGION:0 \"Not region $REGION$\"\n",
        " TRIGGER_IS_DEPLOYED:0 \"Deployed\"\n",
        " TRIGGER_IS_NOT_DEPLOYED:0 \"Not deployed\"\n",
    );
    assert_eq!(
        run(
            "is_fighting_in_strategic_region=0x25 NOT={is_fighting_in_strategic_region=\"045\"} is_deployed=yes is_deployed=no NOT={is_deployed=yes} is_deployed=YES",
            "",
            "",
            localisation,
        ).unwrap(),
        "[yes]Region 37\n[no]Not region 37\n[no]Deployed\n[no]Not deployed\n[yes]Not deployed\n[no]Not deployed\n"
    );
    for source in [
        "is_fighting_in_strategic_region=unknown",
        "is_fighting_in_strategic_region=\"\"",
        "is_deployed=THIS",
    ] {
        let why = run(source, "", "", localisation).unwrap_err();
        assert!(why.contains(source.split('=').next().unwrap()), "{why}");
        assert!(why.contains("not reproduced"), "{why}");
    }
}

#[test]
fn hidden_trigger_writes_temps_and_malformed_or_unknown_logic_preserves_siblings() {
    content_reader_script::test_game::with(|game| {
        use content_reader_script::trigger_keys::Reader;
        game.trigger("synthetic_unported_trigger", 0, Reader::Assign, 0)
            .trigger("set_temp_variable", 0, Reader::Body, 0)
            .trigger("add_to_temp_variable", 0, Reader::Body, 0)
            .trigger("check_variable", 0, Reader::Body, 0)
    });
    struct Folder(std::path::PathBuf);
    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let root = std::env::temp_dir().join(format!(
        "hoi4_trigger_description_test_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let _folder = Folder(root.clone());
    let defines = root.join("common/defines/test.lua");
    std::fs::create_dir_all(defines.parent().unwrap()).unwrap();
    std::fs::write(&defines, "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}").unwrap();
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let resolved = content_reader_layers::content::ResolvedContent {
        layers,
        stats: Default::default(),
        changed: Vec::new(),
        notes: Vec::new(),
    };
    let content = crate::scripted_text::ScriptedContent::read(Arc::new(resolved), String::new())
        .unwrap();
    content.set_synthetic_environment(Default::default());
    let mut frame = crate::scope::tests::frame();
    for id in TableId::ALL {
        if matches!(&*frame.tables[id.index()], TableView::Unavailable { .. }) {
            *frame.tables[id.index()] = TableView::Complete(TableRows::empty(id));
        }
    }
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        vec![replay_data::rows::LexicalTokenRow {
            token: 1,
            literal: b"missing_flag".to_vec(),
        }],
    ));
    *frame.tables[TableId::ScriptFlags.index()] = TableView::Unavailable {
        reason: "flag read refused".into(),
    };
    let names = TriggerNames {
        scripted: Default::default(),
        counts: Some(Default::default()),
    };
    let dlcs = BTreeSet::new();
    let aliases = Arc::default();
    let roles = crate::triggers::LeaderRoles::new(Default::default());
    let mut facts = ContentFacts::content_only(&dlcs, &names, &aliases);
    facts.leader_roles = Ok(&roles);
    let world = World::read(&frame, facts);
    let localisation: Localisation = [
        ("TRIGGER_FULLFILLED_PREFIX", "[yes]"),
        ("TRIGGER_UNFULLFILLED_PREFIX", "[no]"),
        ("TRIGGER_AND_STARTS", "All"),
        ("TRIGGER_OR_STARTS", "Any"),
        ("TRIGGER_ALWAYS_TRUE", "Always"),
        ("TRIGGER_ALWAYS_FALSE", "Never"),
        ("custom", "Custom"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect();
    let side = Side::default();
    let text = ScriptedText::new(&frame, &localisation, &content, 0);
    let run_synthetic = |source: &str| {
        let evaluator = Evaluator::new(&world, Some(&side));
        let items = content_reader_parser::parser::parse(source.as_bytes())
            .unwrap()
            .items;
        let mut refusals = BTreeMap::new();
        let tactic = |name: &[u8]| -> Result<Vec<u8>, String> { Ok(name.to_vec()) };
        let result = describe(
            &items,
            &evaluator,
            &text,
            &localisation,
            Scopes::combat(),
            &[],
            &tactic,
            &mut refusals,
        );
        assert_eq!(evaluator.description_depth.get(), 0);
        match result {
            Ok(out) if !refusals.is_empty() => Err(format!("{out} {refusals:?}")),
            other => other,
        }
    };
    assert_eq!(
        run_synthetic(
            "hidden_trigger={set_temp_variable={x=7}} if={limit={check_variable={temp_var:x=7}} always=no}"
        )
        .unwrap(),
        "[no]Never\n"
    );
    assert_eq!(
        run_synthetic(
            "hidden_trigger={set_temp_variable={x=7}} if={limit={check_variable={temp_var:x=8}} always=no}"
        )
        .unwrap(),
        ""
    );
    assert_eq!(
        run_synthetic(
            "ROOT={hidden_trigger={add_to_temp_variable={x=1}}} if={limit={check_variable={temp_var:x=1}} always=no}"
        )
        .unwrap(),
        "[no]Never\n"
    );
    assert_eq!(
        run_synthetic(
            "OR={always=yes hidden_trigger={add_to_temp_variable={x=1}}} if={limit={check_variable={temp_var:x=0}} always=no}"
        )
        .unwrap(),
        "[yes]Any:\n   Always\n[no]Never\n"
    );
    assert_eq!(
        run_synthetic(
            "OR={ROOT={always=no hidden_trigger={add_to_temp_variable={x=1}}} always=yes} if={limit={check_variable={temp_var:x=0}} always=no}"
        )
        .unwrap(),
        "[yes]Any:\n   :\n      Never\n   Always\n[no]Never\n"
    );
    assert_eq!(
        run_synthetic(
            "NOT={always=yes hidden_trigger={add_to_temp_variable={x=1}}} if={limit={check_variable={temp_var:x=0}} always=no}"
        )
        .unwrap(),
        "[no]Never\n[no]Never\n"
    );
    for source in [
        "OR={check_variable={temp_var:x=0} hidden_trigger={add_to_temp_variable={x=1}}} if={limit={check_variable={temp_var:x=0}} always=no}",
        "OR={ROOT={check_variable={temp_var:x=1} hidden_trigger={add_to_temp_variable={x=1}}} always=yes} if={limit={check_variable={temp_var:x=0}} always=no}",
        "OR={check_variable={temp_var:x=1} hidden_trigger={add_to_temp_variable={x=1}}} if={limit={check_variable={temp_var:x=1}} always=no}",
    ] {
        let display_refused = run_synthetic(source).unwrap_err();
        assert!(
            display_refused.contains("check_variable"),
            "{source}: {display_refused}"
        );
        assert!(
            display_refused.contains("[no]Never\n"),
            "{source}: {display_refused}"
        );
        assert!(
            !display_refused.contains("session is uncertain"),
            "{source}: {display_refused}"
        );
    }
    for source in [
        "hidden_trigger={set_temp_variable={x=7}} OR={has_global_flag=missing_flag always=yes} if={limit={check_variable={temp_var:x=7}} always=no}",
        "hidden_trigger={set_temp_variable={x=7}} OR={if={limit={has_global_flag=missing_flag} always=no} always=yes} if={limit={check_variable={temp_var:x=7}} always=no}",
    ] {
        let kept = run_synthetic(source).unwrap_err();
        assert!(kept.contains("[no]Never\n"), "{source}: {kept}");
        assert!(!kept.contains("session is uncertain"), "{source}: {kept}");
    }
    assert_eq!(
        run_synthetic(
            "hidden_trigger={set_temp_variable={x=7}} OR={hidden_trigger={has_global_flag=missing_flag} always=yes} if={limit={check_variable={temp_var:x=7}} always=no}"
        )
        .unwrap(),
        "[yes]Any:\n   Always\n[no]Never\n"
    );
    assert_eq!(
        run_synthetic(
            "hidden_trigger={set_temp_variable={x=7}} OR={custom_trigger_tooltip={tooltip=custom has_global_flag=missing_flag} always=yes} if={limit={check_variable={temp_var:x=7}} always=no}"
        )
        .unwrap(),
        "[yes]Any:\n   Custom\n   Always\n[no]Never\n"
    );
    let dependent = run_synthetic(
        "hidden_trigger={set_temp_variable={x=7}} OR={has_global_flag=missing_flag hidden_trigger={set_temp_variable={x=8}} always=yes} if={limit={check_variable={temp_var:x=7}} always=no}",
    )
    .unwrap_err();
    assert!(!dependent.contains("[no]Never\n"), "{dependent}");
    assert!(
        dependent.contains("depends on an unknown earlier condition"),
        "{dependent}"
    );
    let undecided_write = run_synthetic(
        "hidden_trigger={set_temp_variable={x=7}} if={limit={has_global_flag=missing_flag} set_temp_variable={x=8}} if={limit={check_variable={temp_var:x=7}} always=no}",
    )
    .unwrap_err();
    assert!(!undecided_write.contains("[no]Never\n"), "{undecided_write}");
    assert!(
        undecided_write.contains("session is uncertain"),
        "{undecided_write}"
    );
    let definite_after_if = run_synthetic(
        "if={limit={has_global_flag=missing_flag} always=yes} hidden_trigger={set_temp_variable={x=7}} if={limit={check_variable={temp_var:x=7}} always=no}",
    )
    .unwrap_err();
    assert!(
        definite_after_if.contains("[no]Never\n"),
        "{definite_after_if}"
    );
    assert!(
        definite_after_if.contains("ScriptFlags is unavailable"),
        "{definite_after_if}"
    );
    let retainted = run_synthetic(
        "OR={has_global_flag=missing_flag hidden_trigger={set_temp_variable={y=1}}} hidden_trigger={set_temp_variable={x=7}} if={limit={has_global_flag=missing_flag} set_temp_variable={x=8}} if={limit={check_variable={temp_var:x=7}} always=no} always=no",
    )
    .unwrap_err();
    assert_eq!(retainted.matches("[no]Never\n").count(), 1, "{retainted}");
    let or_refused = run_synthetic("OR={always=no synthetic_unported_trigger=yes}").unwrap_err();
    assert!(or_refused.contains("   Never\n"), "{or_refused}");
    assert!(
        or_refused.contains("synthetic_unported_trigger"),
        "{or_refused}"
    );
    let malformed = run_synthetic("always=no NOT=yes").unwrap_err();
    assert!(malformed.contains("[no]Never\n"), "{malformed}");
    assert!(
        malformed.contains("logical trigger requires block"),
        "{malformed}"
    );
}
