use super::super::tests::Fixture;
use super::*;
use crate::scripted_text::{EventObject, EventScope};

#[test]
fn native_aliases_share_one_class_dispatch() {
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
    };
    let items =
        installed_content::parser::parse(b"set_unit_leader_flag=x set_character_flag=x if={} else_if={}")
            .unwrap()
            .items;
    // Native aliases share 142729218 and empty own text (140176EE0).
    assert_eq!(
        renderer.effect_class(&items[0]).map(|class| class.name),
        Some("CSetCharacterFlagEffect")
    );
    assert_eq!(
        renderer.effect_class(&items[0]),
        renderer.effect_class(&items[1])
    );
    assert_eq!(
        renderer.leaf(&items[0], scopes),
        renderer.leaf(&items[1], scopes)
    );
    assert_eq!(renderer.leaf(&items[0], scopes), Some(Ok(String::new())));
    // Conditional tails alias CIfEffect and reach the same landed own function.
    assert_eq!(
        renderer.effect_class(&items[2]).map(|class| class.name),
        Some("CIfEffect")
    );
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
    let fixture = Fixture::new();
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let output = fixture.render(
        "synthetic_unknown_effect=1 add_manpower=1 add_manpower=2",
        &mut scopes,
        Content::default(),
    );
    assert!(output.text.is_empty());
    assert_eq!(output.refusals.values().sum::<usize>(), 3);
    assert_eq!(
        output
            .refusals
            .iter()
            .find(|(name, _)| name.contains("CManpowerAddEffect"))
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
