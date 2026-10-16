//! World tension's prepared series as the query hub serves them.

use super::world_tension::test_fixtures::tension_frame;
use crate::test_fixtures::{CAMPAIGN, records};
use replay::container::Replay;
use std::sync::atomic::AtomicBool;

#[test]
fn an_unprepared_cross_frame_tension_tail_is_pending() {
    let first = tension_frame(
        100,
        true,
        100,
        &[(9, 10_000)],
        &[(31, 90, b"Recorded cause", 10_000, 10_000)],
    );
    let tail = tension_frame(
        101,
        false,
        100,
        &[(9, 10_000)],
        &[(31, 90, b"Recorded cause", 10_000, 10_000)],
    );
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first, tail])]).unwrap();
    let root = std::env::temp_dir().join(format!("tension-pending-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("campaign.battleplan");
    std::fs::write(&path, replay.bytes()).unwrap();
    let flag = AtomicBool::new(false);
    let source = crate::Source::open(
        crate::SourceKey {
            source_id: "tension-pending-cross-frame".into(),
            revision: crate::protocol::replay_revision(&path).unwrap(),
        },
        path,
        &flag,
    )
    .unwrap();
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: "series-summary".into(),
        params: serde_json::json!({"branchId":"1","family":{"kind":"world-tension"},"from":100,"to":101,"width":32,"within":{}}),
    };
    let reply: serde_json::Value = serde_json::from_str(
        &crate::prepared::query::query(&source, &request, None, &flag).unwrap(),
    )
    .unwrap();
    assert!(
        reply["data"].is_null(),
        "an unprepared cross-frame tension tail is pending, never a zero-valued chart"
    );
    crate::prepared::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
    drop(source);
    std::fs::remove_dir_all(root).unwrap();
}
