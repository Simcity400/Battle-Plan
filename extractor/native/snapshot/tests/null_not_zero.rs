//! M0 golden fixture: the no-fabrication contract.
//! measured-zero, captured-null, and never-captured must be three DISTINCT states —
//! never collapsed: `null != 0`, and a missing field is not a default.

use serde_json::{json, Value};
use snapshot::Store;

fn store() -> Store {
    Store::open(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data"))
}

#[test]
fn measured_zero_null_and_absent_are_distinct() {
    let snap = store().load("mini", 100).expect("load mini frame");

    // measured zero: present numeric 0.0
    assert_eq!(
        snap.country_path("AAA", "politics.stability"),
        Some(&json!(0.0))
    );
    // captured-but-unobtained: explicit JSON null
    assert_eq!(
        snap.country_path("AAA", "politics.war_support"),
        Some(&Value::Null)
    );
    // never captured: key absent along the path
    assert_eq!(snap.country_path("AAA", "politics.command_power"), None);

    // the three are mutually distinct (the no-fabrication contract)
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
    // a building key that does not exist -> absent, not zero
    assert_eq!(snap.state_path("1", "buildings.dockyard"), None);
}

#[test]
fn missing_game_is_an_error_not_a_panic() {
    let err = store().load("does_not_exist", 100);
    assert!(err.is_err());
}
