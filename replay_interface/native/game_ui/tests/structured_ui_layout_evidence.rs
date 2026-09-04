use serde_json::Value;
use std::collections::BTreeSet;

#[test]
fn ui_layout_engine_evidence_is_structured_and_build_bound() {
    let value: Value =
        serde_json::from_str(include_str!("fixtures/ui_layout_engine_evidence_a729.json"))
            .expect("structured UI-layout evidence parses");

    assert_eq!(value["schema"], "hoi4.ui-layout-engine-evidence.a729.v1");
    assert_eq!(value["executable"]["version"], "1.19.2.0.a729");
    assert_eq!(
        value["executable"]["sha256"],
        "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
    );

    let claims = value["claims"].as_array().expect("claims array");
    assert_eq!(claims.len(), 7);
    let ids: BTreeSet<&str> = claims
        .iter()
        .map(|claim| claim["id"].as_str().expect("claim id"))
        .collect();
    assert_eq!(ids.len(), claims.len(), "claim ids must be unique");
    assert!(claims.iter().all(|claim| {
        claim["statement"]
            .as_str()
            .is_some_and(|statement| !statement.is_empty())
    }));
}
