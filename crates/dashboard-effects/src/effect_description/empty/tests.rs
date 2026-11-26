use super::super::{tests::Fixture, *};
use crate::scripted_text::{EventObject, EventScope};
use crate::fixtures::game;

#[test]
fn empty_dispatch() {
    game::enter();
    let fixture = Fixture::new();
    for body in [
        "fixture_empty_scalar=authored",
        "fixture_empty_body={}",
        "fixture_empty_override={fixture_points=2}",
    ] {
        let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
        let out = fixture.render(body, &mut scopes, Content::default());
        assert!(out.text.is_empty(), "{body}: {}", out.text);
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
}

#[test]
fn empty_containers_describe_and_probe_children_once() {
    game::enter();
    let fixture = Fixture::new();
    let names = content_reader_script::key_names::KeyNames::default();
    let content = Content {
        key_names: Some(&names),
        ..Default::default()
    };
    for (effect_key, own) in [
        ("fixture_new_country", "original_tag=AAA copy_tag=BBB"),
        (
            "fixture_end_resistance",
            "clear=yes occupied=AAA occupier=BBB",
        ),
    ] {
        let body = format!("{effect_key}={{{own} add_political_power=2 add_political_power=3}}");
        let items = content_reader_script::effect_keys::read_body(body.as_bytes(), &names).unwrap();
        let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
        let out = fixture.render_native(&body, &mut scopes, content);
        assert_eq!(out.text, "PP2\nPP3\n", "{effect_key}: {:?}", out.refusals);
        assert!(out.refusals.is_empty(), "{effect_key}: {:?}", out.refusals);
        fixture.with_renderer(&mut scopes, content, |renderer| {
            let selected =
                crate::triggers::event_scopes::project(&renderer.graph.borrow(), 0).unwrap();
            assert!(renderer.multiple(&items, selected), "{effect_key}");
            assert!(renderer.out.refusals.is_empty());
        });
    }
}

#[test]
fn empty_containers_keep_child_refusals_once() {
    game::enter();
    let fixture = Fixture::new();
    for effect_key in ["fixture_new_country", "fixture_end_resistance"] {
        let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
        let out = fixture.render(
            &format!("{effect_key}={{add_political_power=2 every_purchase_contract={{}}}}"),
            &mut scopes,
            Content::default(),
        );
        assert_eq!(out.text, "PP2\n");
        assert_eq!(out.refusals.values().sum::<usize>(), 1);
        assert!(
            out.refusals
                .keys()
                .all(|why| { why.contains("every_purchase_contract") })
        );
    }
}
