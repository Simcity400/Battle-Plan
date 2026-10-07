use super::*;
use crate::triggers::{Eval, Evaluator, Scope};
use replay::rows::{DivisionTemplateIconRow, DivisionTemplateRow, TableId, TableRows};

fn frame() -> DecodedFrame {
    let mut frame = crate::scope::tests::positive_frame();
    for id in TableId::ALL {
        if matches!(&*frame.tables[id.index()], TableView::Unavailable { .. }) {
            *frame.tables[id.index()] = TableView::Complete(TableRows::empty(id));
        }
    }
    frame
}

#[test]
fn template_fallback_uses_recorded_language_and_template_country() {
    let (catalogue, _folder) = tests::catalogue(
        "",
        " force:0 \"Default Force\"\n GER_force:0 \"Intervention Force\"\n",
    );
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment(Default::default());
    let mut frame = frame();
    *frame.tables[TableId::DivisionTemplates.index()] =
        TableView::Complete(TableRows::from_rows::<tables::DivisionTemplates>(vec![
            DivisionTemplateRow {
                template: 9,
                owner: 100,
                name_key: replay::rows::Recorded::Value(b"force".to_vec()),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::DivisionTemplateIcons.index()] =
        TableView::Complete(TableRows::from_rows::<tables::DivisionTemplateIcons>(vec![
            DivisionTemplateIconRow {
                template: 9,
                country: 100,
                ..Default::default()
            },
        ]));
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    let evaluator = Evaluator::new(text.world().unwrap(), None);
    let triggers = installed_content::triggers::read(
        b"has_template = \"Intervention Force\"",
        &installed_content::key_names::KeyNames::default(),
    )
    .items;
    assert_eq!(
        evaluator.evaluate(&triggers, crate::triggers::Scopes::of(Scope::Country(1))),
        Eval::True
    );
    drop(evaluator);
    drop(text);
    *frame.tables[TableId::DivisionTemplateIcons.index()] =
        TableView::Complete(TableRows::from_rows::<tables::DivisionTemplateIcons>(vec![
            DivisionTemplateIconRow {
                template: 9,
                country: 999,
                ..Default::default()
            },
        ]));
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    let refused = Evaluator::new(text.world().unwrap(), None)
        .evaluate(&triggers, crate::triggers::Scopes::of(Scope::Country(1)))
        .decided()
        .unwrap_err();
    assert!(refused.contains("DivisionTemplateIcons"), "{refused}");
}

#[test]
fn entry_conditions_use_deep_links_and_description_variable_session() {
    let (catalogue, _folder) = tests::catalogue(
        "entry = { name = Deep text = { trigger = { FROM.FROM = { tag = SOV } check_variable = { temp_var:x = 7 } } localization_key = YES } text = { localization_key = NO } }",
        " YES:0 \"yes\"\n NO:0 \"no\"\n",
    );
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment(Default::default());
    let frame = frame();
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    let graph = EventScopes::new(vec![
        EventScope {
            from: Link::Scope(1),
            ..EventScope::of(EventObject::Country(1))
        },
        EventScope {
            from: Link::Scope(2),
            ..EventScope::of(EventObject::State(64))
        },
        EventScope::of(EventObject::Country(2)),
    ]);
    let evaluator = Evaluator::new(text.world().unwrap(), None);
    let caller = crate::triggers::Scopes::of(Scope::Country(1));
    evaluator.begin_description();
    let set = installed_content::triggers::read(
        b"set_temp_variable = { x = 7 }",
        &installed_content::key_names::KeyNames::default(),
    )
    .items;
    assert_eq!(evaluator.evaluate(&set, caller), Eval::True);
    assert_eq!(
        text.expand_in_session("[Deep]", &graph, 0, &evaluator),
        Ok("yes".into())
    );
    evaluator.end_description(caller).unwrap();
}

#[test]
fn text_target_priority_matches_trigger_context_and_saved_only_predicate() {
    let (catalogue, _folder) = tests::catalogue("", "");
    let content = catalogue.scripted_content().unwrap();
    content.set_synthetic_environment(Default::default());
    let frame = frame();
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    let mut graph = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    graph
        .tooltip_targets
        .insert(0, vec![(b"target".to_vec(), EventObject::Country(2))]);
    graph
        .global_tooltip_targets
        .push((b"target".to_vec(), EventObject::Country(1)));
    assert_eq!(
        text.expand_in_at("[target.GetTag]", &graph, 0),
        Ok("SOV".into())
    );
    let evaluator = Evaluator::new(text.world().unwrap(), None)
        .with_event_scopes(std::rc::Rc::new(std::cell::RefCell::new(graph)));
    let caller = evaluator.event_scopes(0).unwrap();
    let predicates = installed_content::triggers::read(
        b"has_event_target = target",
        &installed_content::key_names::KeyNames::default(),
    )
    .items;
    assert_eq!(evaluator.evaluate(&predicates, caller), Eval::False);
}

#[test]
fn balance_nested_description_inherits_the_active_variable_session() {
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
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
    let mut frame = frame();
    *frame.tables[TableId::PowerBalances.index()] =
        TableView::Complete(TableRows::from_rows::<tables::PowerBalances>(vec![
            replay::rows::PowerBalanceRow {
                balance: b"balance".to_vec(),
                active_range: b"active".to_vec(),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::PowerBalanceMembers.index()] =
        TableView::Complete(TableRows::from_rows::<tables::PowerBalanceMembers>(vec![
            replay::rows::PowerBalanceMemberRow {
                country: 100,
                ..Default::default()
            },
        ]));
    let text = ScriptedText::new(&frame, &catalogue.localisation, content, 1);
    let graph = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let evaluator = Evaluator::new(text.world().unwrap(), None);
    let caller = crate::triggers::Scopes::of(Scope::Country(1));
    evaluator.begin_description();
    let set = installed_content::triggers::read(
        b"set_temp_variable={x=7}",
        &installed_content::key_names::KeyNames::default(),
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
    let check = installed_content::triggers::read(
        b"check_variable={temp_var:x=7}",
        &installed_content::key_names::KeyNames::default(),
    )
    .items;
    assert_eq!(evaluator.evaluate(&check, caller), Eval::True);
    evaluator.end_description(caller).unwrap();
    assert_eq!(
        text.expand("[ROOT.GetActiveRangeActivationEffect]"),
        Ok("Activate:\nFresh\n".into())
    );
}
