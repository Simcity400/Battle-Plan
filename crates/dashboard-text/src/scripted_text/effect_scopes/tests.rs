use super::*;
use crate::catalogue::tests::script_game;
use crate::triggers::{Eval, Evaluator, Scope};
use replay_data::{
    rows::DivisionTemplateIconRow, rows::DivisionTemplateRow, rows::TableId, rows::TableRows,
};

use crate::scripted_text::tests::complete_frame as frame;

#[test]
fn template_fallback_uses_recorded_language_and_template_country() {
    script_game::enter();
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
                name_key: replay_data::rows::Recorded::Value(b"force".to_vec()),
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
    let triggers = content_reader_script::triggers::read(
        b"has_template = \"Intervention Force\"",
        &content_reader_script::key_names::KeyNames::default(),
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
    assert!(
        refused.contains(
            "division template icons names a country that no recorded country link holds"
        ),
        "{refused}"
    );
}

#[test]
fn entry_conditions_use_deep_links_and_description_variable_session() {
    script_game::enter();
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
    let set = content_reader_script::triggers::read(
        b"set_temp_variable = { x = 7 }",
        &content_reader_script::key_names::KeyNames::default(),
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
    script_game::enter();
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
    let predicates = content_reader_script::triggers::read(
        b"has_event_target = target",
        &content_reader_script::key_names::KeyNames::default(),
    )
    .items;
    assert_eq!(evaluator.evaluate(&predicates, caller), Eval::False);
}
