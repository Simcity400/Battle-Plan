use super::super::tests::Fixture;
use super::*;
use crate::scripted_text::{EventObject, EventScope};

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
