use super::super::tests::Fixture;
use super::*;
use crate::scripted_text::{EventObject, EventScope};

#[test]
fn aliases_share_one_class_dispatch() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    let text = ScriptedText::new(
        &fixture.frame,
        &fixture.catalogue.localisation,
        fixture.catalogue.scripted_content().unwrap(),
        1,
    );
    let context = Context::new(
        &text,
        &fixture.catalogue.localisation,
        1,
        "synthetic",
        Content::default(),
    );
    let graph = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let scopes = crate::triggers::event_scopes::project(&graph, 0).unwrap();
    let mut renderer = Renderer {
        context: &context,
        graph: Rc::new(RefCell::new(graph)),
        evaluator: OnceCell::new(),
        out: Description::default(),
        scripted_stack: Vec::new(),
        concise: None,
    };
    let items = content_reader_parser::parser::parse(
        b"set_unit_leader_flag=x set_character_flag=x if={} else_if={}",
    )
    .unwrap()
    .items;
    assert!(renderer.effect_class(&items[0]).is_some());
    assert_eq!(
        renderer.effect_class(&items[0]),
        renderer.effect_class(&items[1])
    );
    assert_eq!(
        renderer.leaf(&items[0], scopes),
        renderer.leaf(&items[1], scopes)
    );
    assert_eq!(renderer.leaf(&items[0], scopes), Some(Ok(String::new())));
    assert!(renderer.effect_class(&items[2]).is_some());
    assert_eq!(
        renderer.effect_class(&items[2]),
        renderer.effect_class(&items[3])
    );
    assert_eq!(renderer.leaf(&items[2], scopes), Some(Ok(String::new())));
    assert_eq!(
        renderer.leaf(&items[2], scopes),
        renderer.leaf(&items[3], scopes)
    );
}

#[test]
fn unknown_keys_and_unported_classes_are_named_and_counted() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let output = fixture.render(
        "synthetic_unknown_effect=1 every_purchase_contract={} every_purchase_contract={}",
        &mut scopes,
        Content::default(),
    );
    assert!(output.text.is_empty());
    assert_eq!(output.refusals.values().sum::<usize>(), 3);
    assert_eq!(
        output
            .refusals
            .iter()
            .find(|(name, _)| name.contains("every_purchase_contract"))
            .map(|(_, count)| *count),
        Some(2)
    );
    assert!(
        output
            .refusals
            .keys()
            .any(|name| name.contains("unknown effect key synthetic_unknown_effect"))
    );
}
