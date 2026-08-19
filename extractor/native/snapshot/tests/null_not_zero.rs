use serde_json::{json, Value};
use snapshot::Store;

fn store() -> Store {
    Store::open(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data"))
}

#[test]
fn measured_zero_null_and_absent_are_distinct() {
    let snap = store().load("mini", 100).expect("load mini frame");

    assert_eq!(
        snap.country_path("AAA", "politics.stability"),
        Some(&json!(0.0))
    );
    assert_eq!(
        snap.country_path("AAA", "politics.war_support"),
        Some(&Value::Null)
    );
    assert_eq!(snap.country_path("AAA", "politics.command_power"), None);

    let zero = snap.country_path("AAA", "politics.stability");
    let nul = snap.country_path("AAA", "politics.war_support");
    let absent = snap.country_path("AAA", "politics.command_power");
    assert_ne!(zero, nul);
    assert_ne!(nul, absent);
    assert_ne!(zero, absent);
}

#[test]
fn header_and_state_load() {
    let snap = store().load("mini", 100).expect("load mini frame");
    assert_eq!(snap.hour_index, Some(100));
    assert_eq!(snap.game_id.as_deref(), Some("mini"));

    let st = snap.state("1").expect("state 1 present");
    assert_eq!(st.get("owner"), Some(&json!("AAA")));
    assert_eq!(
        snap.state_path("1", "buildings.infrastructure"),
        Some(&json!(3))
    );
    assert_eq!(snap.state_path("1", "buildings.dockyard"), None);
}

#[test]
fn missing_game_is_an_error_not_a_panic() {
    let err = store().load("does_not_exist", 100);
    assert!(err.is_err());
}
