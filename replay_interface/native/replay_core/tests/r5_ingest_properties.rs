//! Ingest-boundary properties fixed in the R5 review round.
//!
//! Each test pins ONE property of `replay_core::source`, using the inputs the
//! review named. A directory store is the cheapest anchor that still runs the
//! real `open_store_impl` admission, so these fixtures use one wherever the
//! property under test is about the INDEX rather than about container bytes.

use replay_core::{open_recording as open_recording_raw, ReplaySource, SourceError};

use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// `Box<dyn ReplaySource>` is not `Debug`, so `expect_err` is unavailable; this
/// keeps the refusal path readable without weakening the assertion.
fn open_err(path: &std::path::Path, why: &str) -> SourceError {
    match open_recording_raw(path) {
        Ok(_) => panic!("{why}"),
        Err(error) => error,
    }
}

fn open_recording(path: &std::path::Path) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_raw(path)
}

const GAME: &str = "r5_ingest";

fn temp_root(tag: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "replay_core_r5_{tag}_{}_{serial}_{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(root.join(GAME)).expect("create store root");
    root
}

fn snapshot_body(hour: i64, date: &str) -> Value {
    json!({
        "game_id": GAME,
        "schema_version": "1",
        "hour_index": hour,
        "date": date,
    })
}

/// Write a directory store from an explicit index body plus explicit snapshot
/// bodies keyed by their file name.
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

fn plain_index(rows: Value) -> Value {
    json!({
        "game_id": GAME,
        "schema_version": "1",
        "snapshots": rows,
    })
}

fn stitched_index(rows: Value, coverage: Value) -> Value {
    json!({
        "game_id": GAME,
        "schema_version": "1",
        "index_schema": "stitched-main-v1",
        "segments_file": "segments.json",
        "main_head": { "segment_id": 0, "hour_index": 2 },
        "snapshots": rows,
        "coverage": coverage,
    })
}

/// The exact shape the stitcher emits for a main-timeline coverage row
/// (`CoverageDisposition::to_value` + `add_main_ownership_fields` +
/// start/end/segment, in `recorder_host/src/stitching.rs`).
fn coverage_row(start: i64, end: i64) -> Value {
    json!({
        "start_hour": start,
        "end_hour": end,
        "kind": "capture_failed",
        "reason": "collector fold failed for the observed hour",
        "evidence_id": "evid-0001",
        "segment_id": 0,
        "owner": {
            "campaign_key_digest": "aa".repeat(32),
            "play_order": 0,
            "session_nonce": "session-0",
            "process_start_identity": "process-0",
            "in_session_ordinal": 0,
            "epoch_token": "epoch-0",
        },
        "selection_rule": "greatest_record_seq_wins_v1",
        "selected_observation_id": "obs-0001",
        "source_record_seq_bounds": [12, 19],
        "source_byte_bounds": [4096, 8192],
    })
}

// -- H1: recorder-declared coverage gaps survive ingest ----------------------

#[test]
fn h1_declared_coverage_gap_reaches_the_source_with_reason_and_witness() {
    let root = temp_root("h1_carrier");
    write_store(
        &root,
        stitched_index(
            json!([
                {"hour_index": 0, "file": "h0.json", "date": "1936.1.1.0"},
                {"hour_index": 2, "file": "h2.json", "date": "1936.1.1.2"},
            ]),
            json!([coverage_row(1, 1)]),
        ),
        &[
            ("h0.json", snapshot_body(0, "1936.1.1.0")),
            ("h2.json", snapshot_body(2, "1936.1.1.2")),
        ],
    );

    let source = open_recording(&root).expect("stitched-shaped directory store opens");
    let coverage = source
        .declared_coverage()
        .expect("a declared coverage statement must cross the ingest boundary");
    assert!(coverage.is_stitched(), "index_schema is carried");
    assert_eq!(coverage.segments_file.as_deref(), Some("segments.json"));
    assert!(coverage.main_head.is_some(), "main_head is carried");
    assert_eq!(coverage.gaps.len(), 1);

    // A gap carries its hour, its reason, the domains it affects, and a witness.
    let gap = coverage.gap_at(1).expect("hour 1 is declared absent");
    assert_eq!((gap.start_hour, gap.end_hour), (1, 1));
    assert_eq!(gap.hour_count(), 1);
    assert_eq!(gap.kind, "capture_failed");
    assert_eq!(gap.reason, "collector fold failed for the observed hour");
    assert_eq!(gap.evidence_id, "evid-0001");
    assert_eq!(gap.segment_id, Some(0));
    assert!(gap.owner.is_some(), "the observation owner is a witness");
    let selection = gap
        .selection
        .as_ref()
        .expect("the recorder's selection evidence must survive ingest");
    assert_eq!(selection.selection_rule, "greatest_record_seq_wins_v1");
    assert_eq!(selection.selected_observation_id, "obs-0001");
    assert_eq!(selection.source_record_seq_bounds, (12, 19));
    assert_eq!(selection.source_byte_bounds, (4096, 8192));
    assert!(
        gap.affects_all_domains(),
        "an index coverage row is a whole-hour absence"
    );
    assert!(coverage.gap_at(0).is_none() && coverage.gap_at(2).is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn h1_a_coverage_row_without_a_witness_is_refused_not_dropped() {
    let root = temp_root("h1_no_witness");
    let mut row = coverage_row(1, 1);
    row.as_object_mut().unwrap().remove("evidence_id");
    write_store(
        &root,
        stitched_index(
            json!([{"hour_index": 0, "file": "h0.json", "date": "1936.1.1.0"}]),
            json!([row]),
        ),
        &[("h0.json", snapshot_body(0, "1936.1.1.0"))],
    );
    let error = open_err(&root, "a witnessless gap claim is not admissible");
    assert!(
        matches!(error, SourceError::Open(_)),
        "expected a typed open refusal, got {error}"
    );
    assert!(
        error.to_string().contains("evidence_id"),
        "the refusal names the missing witness: {error}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn h1_a_partial_selection_evidence_bundle_is_refused() {
    let root = temp_root("h1_partial_selection");
    let mut row = coverage_row(1, 1);
    // The recorder emits all four selection members together; half a bundle is
    // a rewritten row, and keeping the survivors would present partial
    // provenance as the whole claim.
    row.as_object_mut().unwrap().remove("source_byte_bounds");
    write_store(
        &root,
        stitched_index(
            json!([{"hour_index": 0, "file": "h0.json", "date": "1936.1.1.0"}]),
            json!([row]),
        ),
        &[("h0.json", snapshot_body(0, "1936.1.1.0"))],
    );
    let error = open_err(&root, "a partial selection bundle is not admissible");
    assert!(
        error.to_string().contains("selection-evidence members"),
        "{error}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn h1_a_cross_session_transition_gap_has_no_selection_bundle_and_is_still_admitted() {
    // The recorder's forward cross-session transition coverage has no logical
    // frame observation to select (`MainHour::selected_observation` is `None`),
    // so it emits segment_id + owner and no selection members. That is real
    // recorder output and must be admitted, not refused.
    let root = temp_root("h1_transition");
    let mut row = coverage_row(1, 2);
    row["kind"] = json!("gap");
    row["reason"] = json!("cross-session forward time jump");
    for member in [
        "selection_rule",
        "selected_observation_id",
        "source_record_seq_bounds",
        "source_byte_bounds",
    ] {
        row.as_object_mut().unwrap().remove(member);
    }
    write_store(
        &root,
        stitched_index(
            json!([
                {"hour_index": 0, "file": "h0.json", "date": "1936.1.1.0"},
                {"hour_index": 3, "file": "h3.json", "date": "1936.1.1.3"},
            ]),
            json!([row]),
        ),
        &[
            ("h0.json", snapshot_body(0, "1936.1.1.0")),
            ("h3.json", snapshot_body(3, "1936.1.1.3")),
        ],
    );
    let source = open_recording(&root).expect("real transition-gap output must be admitted");
    let coverage = source.declared_coverage().expect("coverage is carried");
    let gap = coverage.gap_at(2).expect("hours 1..=2 are declared absent");
    assert_eq!((gap.start_hour, gap.end_hour), (1, 2));
    assert_eq!(gap.hour_count(), 2);
    assert_eq!(gap.kind, "gap");
    assert_eq!(gap.evidence_id, "evid-0001");
    assert_eq!(gap.segment_id, Some(0));
    assert!(gap.owner.is_some(), "a transition gap still names its owner");
    assert!(
        gap.selection.is_none(),
        "a transition gap has no observation to select, and none is invented"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn h1_a_stitched_coverage_row_without_its_segment_and_owner_is_refused() {
    let root = temp_root("h1_unowned");
    let mut row = coverage_row(1, 1);
    row.as_object_mut().unwrap().remove("segment_id");
    write_store(
        &root,
        stitched_index(
            json!([{"hour_index": 0, "file": "h0.json", "date": "1936.1.1.0"}]),
            json!([row]),
        ),
        &[("h0.json", snapshot_body(0, "1936.1.1.0"))],
    );
    let error = open_err(&root, "a stitched gap must name its owning segment");
    assert!(
        error.to_string().contains("owning segment"),
        "{error}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn h1_a_declared_gap_that_is_also_a_playable_hour_is_refused() {
    let root = temp_root("h1_overlap");
    write_store(
        &root,
        stitched_index(
            json!([
                {"hour_index": 0, "file": "h0.json", "date": "1936.1.1.0"},
                {"hour_index": 1, "file": "h1.json", "date": "1936.1.1.1"},
            ]),
            json!([coverage_row(1, 1)]),
        ),
        &[
            ("h0.json", snapshot_body(0, "1936.1.1.0")),
            ("h1.json", snapshot_body(1, "1936.1.1.1")),
        ],
    );
    let error = open_err(&root, "an hour cannot be both absent and playable");
    assert!(error.to_string().contains("declares hours"), "{error}");
    let _ = fs::remove_dir_all(root);
}

// -- H2: a stitched index dates every visible hour ---------------------------

#[test]
fn h2_stitched_index_without_dates_is_refused_instead_of_disabling_date_navigation() {
    let root = temp_root("h2_undated");
    write_store(
        &root,
        stitched_index(
            json!([
                {"hour_index": 0, "file": "h0.json"},
                {"hour_index": 1, "file": "h1.json"},
            ]),
            json!([]),
        ),
        &[
            ("h0.json", snapshot_body(0, "1936.1.1.0")),
            ("h1.json", snapshot_body(1, "1936.1.1.1")),
        ],
    );
    let error = open_err(&root, "a dateless stitched index is not navigable");
    assert!(
        error.to_string().contains("carries no index date"),
        "{error}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn h2_stitched_index_dates_are_answered_from_the_index_not_the_body() {
    let root = temp_root("h2_dated");
    write_store(
        &root,
        stitched_index(
            json!([
                {"hour_index": 0, "file": "h0.json", "date": "1936.1.1.0"},
                {"hour_index": 1, "file": "h1.json", "date": "1936.1.1.1"},
            ]),
            json!([]),
        ),
        &[
            ("h0.json", snapshot_body(0, "1936.1.1.0")),
            ("h1.json", snapshot_body(1, "1936.1.1.1")),
        ],
    );
    let source = open_recording(&root).expect("a dated stitched index opens");
    assert_eq!(
        source.date_at(0).unwrap().as_deref(),
        Some("1936.1.1.0"),
        "every visible hour answers from the index"
    );
    assert_eq!(source.date_at(1).unwrap().as_deref(), Some("1936.1.1.1"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn h2_a_plain_v1_index_keeps_its_historical_optional_date() {
    let root = temp_root("h2_plain");
    write_store(
        &root,
        plain_index(json!([{"hour_index": 0, "file": "h0.json"}])),
        &[(
            "h0.json",
            json!({"game_id": GAME, "schema_version": "1", "hour_index": 0}),
        )],
    );
    let source = open_recording(&root).expect("a plain v1 index still opens without dates");
    assert_eq!(source.date_at(0).unwrap(), None);
    assert!(
        source.declared_coverage().is_none(),
        "a plain index declares nothing, which is not the same as declaring full coverage"
    );
    let _ = fs::remove_dir_all(root);
}

// -- M1: one hour, one claim -------------------------------------------------

#[test]
fn m1_two_index_rows_for_one_hour_are_refused_not_resolved_first_wins() {
    let root = temp_root("m1_duplicate");
    write_store(
        &root,
        plain_index(json!([
            {"hour_index": 7, "file": "a.json", "date": "1936.1.1.7"},
            {"hour_index": 7, "file": "b.json", "date": "1937.6.6.7"},
        ])),
        &[
            ("a.json", snapshot_body(7, "1936.1.1.7")),
            ("b.json", snapshot_body(7, "1937.6.6.7")),
        ],
    );
    let error = open_err(&root, "two claims for one hour are unresolvable");
    let text = error.to_string();
    assert!(text.contains("twice"), "{text}");
    assert!(
        !text.contains("keeping the first"),
        "the build must not silently pick one: {text}"
    );
    let _ = fs::remove_dir_all(root);
}

// -- M2: the store source joins the whole snapshot header --------------------

#[test]
fn m2_body_date_disagreeing_with_the_index_date_is_refused() {
    let root = temp_root("m2_date");
    write_store(
        &root,
        plain_index(json!([{"hour_index": 3, "file": "h3.json", "date": "1936.1.1.3"}])),
        &[("h3.json", snapshot_body(3, "1941.12.7.3"))],
    );
    let source = open_recording(&root).expect("the index alone is consistent");
    // `date_at` reads the index and `frame()` reads the body: without the join
    // they answered differently for one hour with no refusal anywhere.
    assert_eq!(source.date_at(3).unwrap().as_deref(), Some("1936.1.1.3"));
    let error = source
        .frame(3)
        .expect_err("a body date that contradicts the index is an inconsistent recording");
    assert!(error.to_string().contains("body date"), "{error}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn m2_body_game_id_disagreeing_with_the_selected_game_is_refused() {
    let root = temp_root("m2_game");
    let mut body = snapshot_body(3, "1936.1.1.3");
    body["game_id"] = json!("some-other-campaign");
    write_store(
        &root,
        plain_index(json!([{"hour_index": 3, "file": "h3.json", "date": "1936.1.1.3"}])),
        &[("h3.json", body)],
    );
    let source = open_recording(&root).expect("the index alone is consistent");
    let error = source.frame(3).expect_err("the body names another game");
    assert!(error.to_string().contains("body game_id"), "{error}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn m2_a_body_that_agrees_with_its_index_still_folds() {
    let root = temp_root("m2_ok");
    write_store(
        &root,
        plain_index(json!([{"hour_index": 3, "file": "h3.json", "date": "1936.1.1.3"}])),
        &[("h3.json", snapshot_body(3, "1936.1.1.3"))],
    );
    let source = open_recording(&root).expect("consistent recording opens");
    let frame = source.frame(3).expect("consistent header folds");
    assert_eq!(frame.hour_index, 3);
    assert_eq!(frame.date.as_deref(), Some("1936.1.1.3"));
    let _ = fs::remove_dir_all(root);
}

// -- M3: no unauthenticated A27 design catalog -------------------------------

#[test]
fn m3_directory_backed_design_catalog_without_a_byte_authority_is_refused() {
    let root = temp_root("m3_catalog");
    write_store(
        &root,
        plain_index(json!([{"hour_index": 0, "file": "h0.json", "date": "1936.1.1.0"}])),
        &[("h0.json", snapshot_body(0, "1936.1.1.0"))],
    );
    let catalog_path = root
        .join(GAME)
        .join(replay_core::A27_DESIGN_CATALOG_ARTIFACT_PATH);
    fs::create_dir_all(catalog_path.parent().unwrap()).unwrap();
    // Loose bytes on disk: no container entry digest, no recorder-manifest
    // sha256/length. Nothing can detect an edit.
    fs::write(&catalog_path, b"{\"schema\":\"anything\"}").unwrap();

    let error = open_err(&root, "an unauthenticated semantic catalog must not reach the fold");
    let text = error.to_string();
    assert!(text.contains("unauthenticated"), "{text}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn m3_a_directory_store_without_a_catalog_still_opens() {
    let root = temp_root("m3_absent");
    write_store(
        &root,
        plain_index(json!([{"hour_index": 0, "file": "h0.json", "date": "1936.1.1.0"}])),
        &[("h0.json", snapshot_body(0, "1936.1.1.0"))],
    );
    let source = open_recording(&root).expect("absence of a catalog is not a refusal");
    assert!(source.design_catalog().is_none());
    let _ = fs::remove_dir_all(root);
}

// -- L1: "not found" means not found -----------------------------------------

#[test]
fn l1_a_missing_path_is_not_found_with_its_historical_rendering() {
    let root = temp_root("l1_missing");
    let missing = root.join("nope.battleplan");
    let error = open_err(&missing, "a path that does not exist is NotFound");
    assert!(matches!(error, SourceError::NotFound(_)), "{error}");
    // The absent-path message is a contract (replay_app pins it). Metadata-based
    // classification must not change it.
    assert_eq!(
        error.to_string(),
        format!(
            "open: path does not exist (expected a .battleplan file or a snapshot store \
             directory): {}",
            missing.display()
        )
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn l1_an_unreadable_path_is_not_reported_as_absent() {
    let root = temp_root("l1_unreadable");
    let file = root.join("recording.battleplan");
    fs::write(&file, b"not a container").unwrap();
    // A path UNDER a regular file exists in no directory: the OS reports a
    // not-a-directory failure, not "no such file". `is_file()`/`is_dir()` both
    // answered false and the old code reported NotFound for it.
    let descendant = file.join("child.battleplan");
    match fs::metadata(&descendant) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // This platform genuinely reports absence; nothing to distinguish.
        }
        _ => {
            let error = open_err(&descendant, "this path cannot be opened");
            assert!(
                !matches!(error, SourceError::NotFound(_)),
                "an unreadable path must not be reported as absent: {error}"
            );
        }
    }
    let _ = fs::remove_dir_all(root);
}

// -- A2: an unknown coverage kind is not carried into playback ---------------

#[test]
fn a2_an_unknown_coverage_disposition_kind_is_refused() {
    let root = temp_root("a2_kind");
    let mut row = coverage_row(1, 1);
    row["kind"] = json!("teleported");
    write_store(
        &root,
        stitched_index(
            json!([{"hour_index": 0, "file": "h0.json", "date": "1936.1.1.0"}]),
            json!([row]),
        ),
        &[("h0.json", snapshot_body(0, "1936.1.1.0"))],
    );
    let error = open_err(&root, "an uninterpretable gap class is not admissible");
    assert!(
        error.to_string().contains("unknown coverage kind"),
        "{error}"
    );
    let _ = fs::remove_dir_all(root);
}

// -- L5: container identity is proved for EVERY prefolded hour ---------------

#[derive(Debug)]
struct LaterHourProvenanceMismatchPrefolder;

impl replay_core::CertifiedPrefoldDecoder for LaterHourProvenanceMismatchPrefolder {
    fn decoder_id(&self) -> &'static str {
        "test.r5.later-hour-provenance-mismatch"
    }

    fn decode_certified(
        &self,
        request: replay_core::CertifiedPrefoldRequest,
    ) -> Result<replay_core::PrefoldedReplay, replay_core::PrefoldDecodeError> {
        let (game_id, _) = request.into_parts();
        let mut entries = Vec::new();
        for hour in [0_i64, 1] {
            let mut body = json!({
                "game_id": game_id,
                "schema_version": "1",
                "hour_index": hour,
                "date": format!("1936.1.1.{hour}"),
            });
            // Only the SECOND hour disagrees with the container manifest. When
            // admission proved identity for entry 0 alone, this reached
            // playback unchecked.
            if hour == 1 {
                body["provenance"] = json!({"mod": {"modset_id": "snapshot-modset"}});
            }
            let bytes = serde_json::to_vec(&body).unwrap();
            entries.push(
                replay_core::PrefoldedHourEntry::from_json(hour, &bytes)
                    .map_err(|error| replay_core::PrefoldDecodeError::new(error.to_string()))?,
            );
        }
        replay_core::PrefoldedReplay::try_new(game_id, json!("1"), entries)
            .map_err(|error| replay_core::PrefoldDecodeError::new(error.to_string()))
    }
}

#[test]
fn l5_a_later_prefolded_hour_that_contradicts_the_container_is_refused() {
    let root = temp_root("l5_later_hour");
    let entries = vec![
        snapshot::PackEntry {
            path: format!("payload/{GAME}/index.json"),
            bytes: serde_json::to_vec(&json!({
                "game_id": GAME,
                "schema_version": "1",
                "snapshots": []
            }))
            .unwrap(),
            role: "index",
            hour_index: None,
        },
        snapshot::PackEntry {
            path: format!("payload/{GAME}/collector_raw/collector_stream.bin"),
            bytes: b"H4CRfixture-stream".to_vec(),
            role: "collector_compact_stream",
            hour_index: None,
        },
    ];
    let identity = snapshot::PackIdentity {
        mod_identity: Some(json!({"modset_id": "manifest-modset"})),
        ..Default::default()
    };
    let path = root.join("later_hour_mismatch.battleplan");
    fs::write(
        &path,
        snapshot::pack(GAME, &entries, &identity, true).unwrap(),
    )
    .unwrap();

    let error = match replay_core::open_recording_with_decoder(
        &path,
        &LaterHourProvenanceMismatchPrefolder,
    ) {
        Err(error) => error,
        Ok(_) => panic!("a later hour that contradicts the container must be refused"),
    };
    assert!(matches!(error, SourceError::ModsetMismatch(_)), "{error}");
    assert!(error.to_string().contains("snapshot-modset"), "{error}");
    let _ = fs::remove_dir_all(root);
}
