use super::super::tests::Fixture;
use super::*;
use crate::scripted_text::{EventObject, EventScope};
use crate::fixtures::game;

#[test]
fn unknown_keys_and_unported_classes_are_named_and_counted() {
    game::enter();
    let fixture = Fixture::new();
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let output = fixture.render(
        "synthetic_unknown_effect=1 fixture_unported={} fixture_unported={}",
        &mut scopes,
        Content::default(),
    );
    assert!(output.text.is_empty());
    assert_eq!(
        output.refusals,
        BTreeMap::from([
            ("synthetic effect fixture_unported: key fixture_unported: effect key fixture_unported description is unsupported".into(), 2),
            ("synthetic effect synthetic_unknown_effect: key synthetic_unknown_effect: unknown effect key synthetic_unknown_effect".into(), 1),
        ]),
    );
}
