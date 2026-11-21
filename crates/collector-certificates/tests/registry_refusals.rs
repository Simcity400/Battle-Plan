
use collector_certificates::watch_registry::{WatchRegistry, status};

fn filled(len: usize, receivers: usize) -> WatchRegistry {
    let registry = WatchRegistry::enabled(len);
    for receiver in 1..=receivers {
        registry.sequence(receiver * 8);
    }
    registry
}

fn json(value: &impl serde::Serialize) -> String {
    String::from_utf8(collector_status::status::serialize(value).unwrap()).unwrap()
}

#[test]
fn a_refusal_shows_in_its_registry_and_the_total_sums_every_registry() {
    let full = filled(4, 5);
    let half = filled(4, 2);
    let unnamed = filled(4, 6);
    assert_eq!(
        (full.refused(), half.refused(), unnamed.refused()),
        (1, 0, 2)
    );

    let block = json(&status([("full", &full), ("half", &half)]));
    assert!(block.contains(r#""refused_total":3,"#), "{block}");
    assert!(block.contains(r#""peak_load":1.0,"#), "{block}");
    assert!(block.contains(r#""longest_probe":null,"#), "{block}");
    assert!(
        block.ends_with(
            r#""registries":{"full":{"registered":4,"capacity":4,"refused":1},"half":{"registered":2,"capacity":4,"refused":0}}}"#
        ),
        "{block}"
    );

    let later = json(&status([("half", &half)]));
    assert!(
        later.contains(r#""peak_load":1.0,"#),
        "the peak holds: {later}"
    );
}
