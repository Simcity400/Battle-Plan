
use replay_core::{
    coalesce_same_hour_ended_combats, HourCoverageState, Leaf, ViewStatus, WorldError,
    WorldTimeline,
};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const GAME: &str = "fw14_fixture";

fn temp_root(tag: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "replay_core_fw14_{tag}_{}_{serial}_{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(root.join(GAME)).expect("create store root");
    root
}

fn write_store(root: &PathBuf, index: Value, bodies: &[(&str, Value)]) {
    fs::write(
        root.join(GAME).join("index.json"),
        serde_json::to_vec(&index).unwrap(),
    )
    .unwrap();
    for (file, body) in bodies {
        fs::write(
            root.join(GAME).join(file),
            serde_json::to_vec(body).unwrap(),
        )
        .unwrap();
    }
}

fn snapshot_body(hour: i64, date: &str, extra: Value) -> Value {
    let mut body = json!({
        "game_id": GAME,
        "schema_version": "1",
        "hour_index": hour,
        "date": date,
    });
    if let (Some(body_map), Some(extra_map)) = (body.as_object_mut(), extra.as_object()) {
        for (key, value) in extra_map {
            body_map.insert(key.clone(), value.clone());
        }
    }
    body
}

/// (`CoverageDisposition::to_value` + `add_main_ownership_fields`).
fn coverage_row(start: i64, end: i64, kind: &str, reason: &str) -> Value {
    json!({
        "start_hour": start,
        "end_hour": end,
        "kind": kind,
        "reason": reason,
        "evidence_id": "evid-fw14",
        "segment_id": 0,
        "owner": {
            "campaign_key_digest": "ab".repeat(32),
            "play_order": 0,
            "session_nonce": "session-0",
            "process_start_identity": "process-0",
            "in_session_ordinal": 0,
            "epoch_token": "epoch-0",
        },
        "selection_rule": "greatest_record_seq_wins_v1",
        "selected_observation_id": "obs-fw14",
        "source_record_seq_bounds": [3, 9],
        "source_byte_bounds": [1024, 2048],
    })
}

fn stitched_index(rows: Value, coverage: Value) -> Value {
    json!({
        "game_id": GAME,
        "schema_version": "1",
        "index_schema": "stitched-main-v1",
        "segments_file": "segments.json",
        "main_head": { "segment_id": 0, "hour_index": 4 },
        "snapshots": rows,
        "coverage": coverage,
    })
}

fn open_gap_timeline() -> (WorldTimeline, PathBuf) {
    let root = temp_root("declared_gap");
    write_store(
        &root,
        stitched_index(
            json!([
                {"hour_index": 1, "file": "h1.json", "date": "1936.1.1.1"},
                {"hour_index": 4, "file": "h4.json", "date": "1936.1.1.4"},
            ]),
            json!([
                coverage_row(0, 0, "gap", "no observation before cutover"),
                coverage_row(2, 3, "capture_failed", "collector fold failed for the observed hour"),
            ]),
        ),
        &[
            ("h1.json", snapshot_body(1, "1936.1.1.1", json!({}))),
            ("h4.json", snapshot_body(4, "1936.1.1.4", json!({}))),
        ],
    );
    let source = replay_core::open_recording(&root).expect("stitched-shaped store opens");
    (
        WorldTimeline::open(source).expect("timeline opens"),
        root,
    )
}

#[test]
fn declared_coverage_passes_through_the_timeline_verbatim() {
    let (timeline, root) = open_gap_timeline();
    let coverage = timeline
        .declared_coverage()
        .expect("the declared statement crosses WorldTimeline, not just ReplaySource");
    assert!(coverage.is_stitched());
    assert_eq!(coverage.gaps.len(), 2);
    let gap = timeline.declared_gap_at(2).expect("hour 2 is declared absent");
    assert_eq!((gap.start_hour, gap.end_hour), (2, 3));
    assert_eq!(gap.kind, "capture_failed");
    assert_eq!(gap.reason, "collector fold failed for the observed hour");
    assert!(gap.affects_all_domains());
    assert!(timeline.declared_gap_at(1).is_none());
    assert!(timeline.declared_gap_at(4).is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_declared_gap_hour_surfaces_every_domain_unobtained_with_the_named_reason() {
    let (timeline, root) = open_gap_timeline();
    for hour in [2_i64, 3] {
        let state = timeline
            .coverage_state_at(hour)
            .expect("a declared gap hour is an answer, not an error");
        let view = match state {
            HourCoverageState::DeclaredGap(view) => view,
            HourCoverageState::Frame { frame_hour, .. } => panic!(
                "hour {hour} is declared absent but resolved to frame hour {frame_hour} \
                 (stale measurements presented as the requested hour)"
            ),
        };
        assert_eq!(view.requested_hour(), hour);
        assert_eq!((view.gap().start_hour, view.gap().end_hour), (2, 3));
        assert_eq!(view.domain_status(), ViewStatus::DomainUnavailable);
        assert_eq!(view.leaf_status::<i64>(), Leaf::Unobtained);
        let warning = view
            .coverage_warnings()
            .first()
            .expect("a declared gap hour carries a warning");
        assert!(
            warning.contains("capture_failed")
                && warning.contains("collector fold failed for the observed hour")
                && warning.contains("2..=3"),
            "the warning names the gap's kind, reason, and span: {warning}"
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_declared_gap_outranks_the_before_first_inference() {
    let (timeline, root) = open_gap_timeline();
    let state = timeline.coverage_state_at(0).expect("declared gap answers");
    match state {
        HourCoverageState::DeclaredGap(view) => {
            assert_eq!(view.gap().kind, "gap");
            assert_eq!(view.requested_hour(), 0);
        }
        HourCoverageState::Frame { .. } => panic!("hour 0 has no frame to resolve to"),
    }
    let root2 = temp_root("before_first");
    write_store(
        &root2,
        stitched_index(
            json!([
                {"hour_index": 1, "file": "h1.json", "date": "1936.1.1.1"},
                {"hour_index": 4, "file": "h4.json", "date": "1936.1.1.4"},
            ]),
            json!([coverage_row(2, 3, "capture_failed", "collector fold failed for the observed hour")]),
        ),
        &[
            ("h1.json", snapshot_body(1, "1936.1.1.1", json!({}))),
            ("h4.json", snapshot_body(4, "1936.1.1.4", json!({}))),
        ],
    );
    let source = replay_core::open_recording(&root2).expect("store opens");
    let timeline2 = WorldTimeline::open(source).expect("timeline opens");
    assert!(matches!(
        timeline2.coverage_state_at(0),
        Err(WorldError::BeforeFirst { hour: 0, first: 1 })
    ));
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(root2);
}

#[test]
fn recorded_and_undeclared_hours_keep_their_existing_resolutions() {
    let (timeline, root) = open_gap_timeline();
    match timeline.coverage_state_at(1).expect("recorded hour") {
        HourCoverageState::Frame {
            frame,
            frame_hour,
            exact,
        } => {
            assert_eq!(frame_hour, 1);
            assert!(exact);
            assert_eq!(frame.hour_index(), 1);
        }
        HourCoverageState::DeclaredGap(_) => panic!("hour 1 is recorded, not declared absent"),
    }
    match timeline.coverage_state_at(5).expect("scrub fallback") {
        HourCoverageState::Frame {
            frame_hour, exact, ..
        } => {
            assert_eq!(frame_hour, 4);
            assert!(!exact);
        }
        HourCoverageState::DeclaredGap(_) => panic!("hour 5 was never declared absent"),
    }
    assert!(matches!(
        timeline.frame(2),
        Err(WorldError::NoSuchHour(2))
    ));
    assert_eq!(timeline.state_at(2).expect("scrub").hour_index(), 1);
    assert_eq!(
        timeline.seek(2).expect("seek").hour_index(),
        timeline.play_to(2).expect("play_to").hour_index()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_recording_that_declared_nothing_answers_none_and_resolves_like_state_at() {
    let root = temp_root("plain_v1");
    write_store(
        &root,
        json!({
            "game_id": GAME,
            "schema_version": "1",
            "snapshots": [
                {"hour_index": 1, "file": "h1.json", "date": "1936.1.1.1"},
                {"hour_index": 4, "file": "h4.json", "date": "1936.1.1.4"},
            ],
        }),
        &[
            ("h1.json", snapshot_body(1, "1936.1.1.1", json!({}))),
            ("h4.json", snapshot_body(4, "1936.1.1.4", json!({}))),
        ],
    );
    let source = replay_core::open_recording(&root).expect("plain store opens");
    let timeline = WorldTimeline::open(source).expect("timeline opens");
    assert!(timeline.declared_coverage().is_none());
    assert!(timeline.declared_gap_at(2).is_none());
    match timeline.coverage_state_at(2).expect("undeclared hole scrubs") {
        HourCoverageState::Frame {
            frame_hour, exact, ..
        } => {
            assert_eq!(frame_hour, 1);
            assert!(!exact);
        }
        HourCoverageState::DeclaredGap(_) => panic!("nothing was declared"),
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ended_combat_round_trip_from_raw_snapshots_through_the_coalescer() {
    let root = temp_root("ended_combat_roundtrip");
    let first_body = json!({
        "ended_combats": [
            {"combat_id": 42, "generation": 7, "kind": "land", "lifecycle": "ended",
             "location_province": 11, "side_detail_availability": "present",
             "land_combat_detail_events_live": {"phase_event": "ended", "generation": 7}},
        ]
    });
    let second_body = json!({
        "ended_combats": [
            {"combat_id": 42, "generation": 7, "kind": "land", "lifecycle": "ended",
             "side_detail_availability": "unavailable", "terrain": "urban"},
            {"combat_id": 9, "generation": 1, "kind": "land", "lifecycle": "ended",
             "side_detail_availability": "unavailable"},
        ]
    });
    write_store(
        &root,
        json!({
            "game_id": GAME,
            "schema_version": "1",
            "snapshots": [
                {"hour_index": 1, "file": "h1.json", "date": "1936.1.1.1"},
                {"hour_index": 2, "file": "h2.json", "date": "1936.1.1.2"},
            ],
        }),
        &[
            ("h1.json", snapshot_body(1, "1936.1.1.1", first_body)),
            ("h2.json", snapshot_body(2, "1936.1.1.2", second_body)),
        ],
    );
    let source = replay_core::open_recording(&root).expect("store opens");
    let timeline = WorldTimeline::open(source).expect("timeline opens");
    let first = timeline.frame(1).expect("frame 1");
    let second = timeline.frame(2).expect("frame 2");
    assert_eq!(first.ended_combats_status(), ViewStatus::Present);
    assert_eq!(second.ended_combats_status(), ViewStatus::Present);

    for (a, b) in [(&first, &second), (&second, &first)] {
        let (union, status) = coalesce_same_hour_ended_combats(
            a.ended_combats(),
            a.ended_combats_status(),
            b.ended_combats(),
            b.ended_combats_status(),
        );
        assert_eq!(status, ViewStatus::Present);
        assert_eq!(
            union.keys().copied().collect::<Vec<_>>(),
            vec![(9, 1), (42, 7)],
            "union by the complete (combat_id, generation) identity"
        );
        let collided = &union[&(42, 7)];
        assert_eq!(
            collided.side_detail_availability,
            Leaf::Measured("present".into()),
            "present beats unavailable in either input order"
        );
        assert!(
            matches!(&collided.detail, Leaf::Measured(detail)
                if detail.phase_event == Leaf::Measured("ended".into())),
            "the surviving per-side detail is never blanked"
        );
        assert_eq!(collided.terrain, Leaf::Measured("urban".into()));
        assert_eq!(collided.location_province, Leaf::Measured(11));
    }
    let _ = fs::remove_dir_all(root);
}
