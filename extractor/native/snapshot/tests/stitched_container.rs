//! Container v2 (stitched / reload-relaunch) READER acceptance
//! (`STITCHING_DESIGN.md` §7, §9).
//!
//! Proves the reader:
//! * opens a v2 stitched container that declares `timeline_segments_v1`;
//! * serves the main timeline (visible snapshots) through the ordinary index;
//! * loads a SUPERSEDED (rollback) snapshot by its explicit segment-prefixed file
//!   name — retained, never deleted;
//! * parses the authoritative `segments.json` branch graph;
//! * FAILS CLOSED on a v2 container missing the required feature, and on any
//!   container version above the reader max.

use serde_json::{json, Value};
use snapshot::canon::manifest_canonical_bytes;
use snapshot::container::{
    canonical_stitch_input_digest, Container, ContainerError, ExpectedIdentity,
};
use snapshot::sha256::Sha256;
use snapshot::{pack_with, PackEntry, PackIdentity, PackOptions};

const GAME: &str = "campaign";
const MODSET: &str = "modset-xyz";
const CAMPAIGN_KEY: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const CAMPAIGN_DIGEST: &str = "66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925";

fn snap_bytes(hour: i64) -> Vec<u8> {
    format!(
        r#"{{"schema_version":"1","game_id":"{GAME}","hour_index":{hour},"provenance":{{"mod":{{"modset_id":"{MODSET}"}}}},"countries":{{}},"states":{{}}}}"#
    )
    .into_bytes()
}

fn ent(path: String, bytes: Vec<u8>, role: &'static str, hour: Option<i64>) -> PackEntry {
    PackEntry {
        path,
        bytes,
        role,
        hour_index: hour,
    }
}

fn raw_bytes(segment_id: u32, len: usize) -> Vec<u8> {
    vec![(segment_id as u8) + 1; len]
}

fn source_key(segment_id: u32, len: usize) -> String {
    let digest = Sha256::hex(&raw_bytes(segment_id, len));
    let mut bytes = Vec::new();
    for value in [
        format!("nonce-{segment_id}"),
        format!("pid-{segment_id}"),
        digest.clone(),
        digest,
    ] {
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(0);
    }
    Sha256::hex(&bytes)
}

fn owner(segment_id: u32) -> Value {
    json!({
        "campaign_key_digest": CAMPAIGN_DIGEST,
        "play_order": segment_id,
        "session_nonce": format!("nonce-{segment_id}"),
        "process_start_identity": format!("pid-{segment_id}"),
        "in_session_ordinal": 0,
        "epoch_token": format!("epoch-{segment_id}"),
    })
}

fn ticket(segment_id: u32) -> Value {
    json!({
        "campaign_key_digest": CAMPAIGN_DIGEST,
        "play_order": segment_id,
        "session_nonce": format!("nonce-{segment_id}"),
        "process_start_identity": format!("pid-{segment_id}"),
    })
}

fn participation_id(segment_id: u32, len: usize) -> String {
    let binding = json!({
        "ticket": ticket(segment_id),
        "scheme_major": 1,
        "identity_evidence_digest": "00".repeat(32),
        "record_seq_bounds": [0, (len / 50 - 1) as u64],
        "byte_bounds": [0, len as u64],
    });
    Sha256::hex(&canon(&binding))
}

fn observation(segment_id: u32, hour: i64, sequence: u64) -> Value {
    json!({
        "observation_id": format!("obs-{segment_id}-{hour}"),
        "owner": owner(segment_id),
        "record_seq_bounds": [sequence, sequence + 1],
        "byte_bounds": [sequence * 50, (sequence + 2) * 50],
        "kind": "snapshot",
        "sha256": Sha256::hex(&snap_bytes(hour)),
    })
}

fn hour_row(segment_id: u32, hour: i64, sequence: u64, file: &str) -> Value {
    json!({
        "hour_index": hour,
        "file": file,
        "kind": "snapshot",
        "sha256": Sha256::hex(&snap_bytes(hour)),
        "selection_rule": "greatest_record_sequence_v1",
        "selected_observation_id": format!("obs-{segment_id}-{hour}"),
        "owner": owner(segment_id),
        "source_record_seq_bounds": [sequence, sequence + 1],
        "source_byte_bounds": [sequence * 50, (sequence + 2) * 50],
        "observation_count": 1,
        "observations": [observation(segment_id, hour, sequence)],
    })
}

fn main_row(segment_id: u32, hour: i64, sequence: u64, file: &str) -> Value {
    let mut row = hour_row(segment_id, hour, sequence, file);
    let object = row.as_object_mut().unwrap();
    object.remove("hour_index");
    object.remove("observation_count");
    object.remove("observations");
    object.insert("hour_index".into(), json!(hour));
    row
}

fn source_ref(segment_id: u32, len: usize, participation_id: &str) -> Value {
    let digest = Sha256::hex(&raw_bytes(segment_id, len));
    json!({
        "source_key": source_key(segment_id, len),
        "participation_id": participation_id,
        "session_path": format!("sessions/source-{segment_id}.h4cr"),
        "stored_digest": digest,
        "decoded_digest": digest,
        "record_seq_bounds": [0, (len / 50 - 1) as u64],
        "byte_bounds": [0, len as u64],
    })
}

fn raw_source_row(segment_id: u32, len: usize, participation_id: &str) -> Value {
    let digest = Sha256::hex(&raw_bytes(segment_id, len));
    json!({
        "source_key": source_key(segment_id, len),
        "session_nonce": format!("nonce-{segment_id}"),
        "process_start_identity": format!("pid-{segment_id}"),
        "session_path": format!("sessions/source-{segment_id}.h4cr"),
        "stored_digest": digest,
        "decoded_digest": digest,
        "record_seq_bounds": [0, (len / 50 - 1) as u64],
        "byte_bounds": [0, len as u64],
        "storage": {
            "kind": "embedded_campaign",
            "campaign_key_digest": CAMPAIGN_DIGEST,
        },
        "participations": [{
            "participation_id": participation_id,
            "ticket": ticket(segment_id),
            "scheme_major": 1,
            "identity_evidence_digest": "00".repeat(32),
            "record_seq_bounds": [0, (len / 50 - 1) as u64],
            "byte_bounds": [0, len as u64],
        }],
    })
}

/// A minimal but faithful stitched store: segment 0 owns hours 0..=1; segment 1
/// (a rollback) supersedes hour 1. Main timeline = s0:hour0 + s1:hour1. Segment
/// 0's hour-1 snapshot is retained as rollback data.
fn stitched_entries() -> Vec<PackEntry> {
    let s0_h0 = "s00000000.000000000000.snapshot.json";
    let s0_h1_rollback = "s00000000.000000000001.snapshot.json"; // superseded
    let s1_h1 = "s00000001.000000000001.snapshot.json"; // visible
    let p0 = participation_id(0, 200);
    let p1 = participation_id(1, 100);

    // stitched-main-v1 index: only VISIBLE snapshots; a visible coverage gap; head.
    let index = json!({
        "game_id": GAME,
        "schema_version": "1",
        "index_schema": "stitched-main-v1",
        "segments_file": "segments.json",
        "main_head": { "segment_id": 1, "hour_index": 1 },
        "snapshots": [
            main_row(0, 0, 0, s0_h0),
            main_row(1, 1, 0, s1_h1)
        ],
        "coverage": []
    });
    let mut segments = json!({
        "stitch_input_digest": "dd".repeat(32),
        "play_order_ledger_digest": "cc".repeat(32),
        "fold_version": "world-fold-fixture-v1",
        "schema_version": "1",
        "canonical_writer_version": "snapshot-canon-fixture-v1",
        "packer_codec_version": "zip-fixture-v1",
        "publication": {
            "created_utc": "2026-07-12T00:00:00Z",
            "compression_method": "store",
            "compression_parameters": { "level": "none" },
            "entry_order": "lexicographic-path-manifest-last-v1",
        },
        "campaign": {
            "scheme_major": 1,
            "campaign_key": CAMPAIGN_KEY,
            "campaign_key_digest": CAMPAIGN_DIGEST
        },
        "stream_major": 2,
        "raw_sources": [raw_source_row(0, 200, &p0), raw_source_row(1, 100, &p1)],
        "main_slices": [
            { "segment_id": 0, "start_hour": 0, "end_hour": 0 },
            { "segment_id": 1, "start_hour": 1, "end_hour": 1 }
        ],
        "main_head": { "segment_id": 1, "hour_index": 1 },
        "segments": [
            {
                "segment_id": 0, "play_order": 0, "in_session_ordinal": 0,
                "session_nonce": "nonce-0", "process_start_identity": "pid-0",
                "epoch_token": "epoch-0", "ticket": ticket(0),
                "identity": {
                    "scheme_major": 1, "campaign_key_digest": CAMPAIGN_DIGEST,
                    "evidence_digest": "00".repeat(32)
                },
                "cutover_hour": 0, "head_hour": 1, "parent_segment": null,
                "empty_recovery": false, "index_file": "segments/s00000000.index.json",
                "source_ref": source_ref(0, 200, &p0),
                "visible_spans": [ { "start_hour": 0, "end_hour": 0 } ],
                "superseded_spans": [ { "start_hour": 1, "end_hour": 1, "superseded_by": 1 } ]
            },
            {
                "segment_id": 1, "play_order": 1, "in_session_ordinal": 0,
                "session_nonce": "nonce-1", "process_start_identity": "pid-1",
                "epoch_token": "epoch-1", "ticket": ticket(1),
                "identity": {
                    "scheme_major": 1, "campaign_key_digest": CAMPAIGN_DIGEST,
                    "evidence_digest": "00".repeat(32)
                },
                "cutover_hour": 1, "head_hour": 1, "parent_segment": 0,
                "empty_recovery": false, "index_file": "segments/s00000001.index.json",
                "source_ref": source_ref(1, 100, &p1),
                "visible_spans": [ { "start_hour": 1, "end_hour": 1 } ],
                "superseded_spans": []
            }
        ]
    });
    let seg0_index = json!({
        "segment_id": 0, "cutover_hour": 0, "head_hour": 1,
        "source_ref": source_ref(0, 200, &p0),
        "transition_coverage": [],
        "hours": [
            hour_row(0, 0, 0, s0_h0),
            hour_row(0, 1, 2, s0_h1_rollback)
        ]
    });
    let seg1_index = json!({
        "segment_id": 1, "cutover_hour": 1, "head_hour": 1,
        "source_ref": source_ref(1, 100, &p1),
        "transition_coverage": [],
        "hours": [hour_row(1, 1, 0, s1_h1)]
    });
    reseal_stitch_digests(&mut segments, &[seg0_index.clone(), seg1_index.clone()]);

    let mut entries = vec![
        ent(
            format!("payload/{GAME}/index.json"),
            canon(&index),
            "index",
            None,
        ),
        ent(
            format!("payload/{GAME}/segments.json"),
            canon(&segments),
            "segments",
            None,
        ),
        ent(
            format!("payload/{GAME}/segments/s00000000.index.json"),
            canon(&seg0_index),
            "segment_index",
            None,
        ),
        ent(
            format!("payload/{GAME}/segments/s00000001.index.json"),
            canon(&seg1_index),
            "segment_index",
            None,
        ),
        ent(
            format!("payload/{GAME}/{s0_h0}"),
            snap_bytes(0),
            "snapshot",
            Some(0),
        ),
        ent(
            format!("payload/{GAME}/{s0_h1_rollback}"),
            snap_bytes(1),
            "snapshot",
            Some(1),
        ),
        ent(
            format!("payload/{GAME}/{s1_h1}"),
            snap_bytes(1),
            "snapshot",
            Some(1),
        ),
        ent(
            format!("payload/{GAME}/sessions/source-0.h4cr"),
            raw_bytes(0, 200),
            "raw_source",
            None,
        ),
        ent(
            format!("payload/{GAME}/sessions/source-1.h4cr"),
            raw_bytes(1, 100),
            "raw_source",
            None,
        ),
    ];
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries
}

fn canon(v: &Value) -> Vec<u8> {
    let mut s = String::new();
    snapshot::canon::write_canonical(v, &mut s);
    s.into_bytes()
}

fn reseal_stitch_digests(graph: &mut Value, segment_indexes: &[Value]) {
    let ledger = Value::Array(
        graph["segments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|segment| {
                json!({
                    "campaign_key_digest": segment["ticket"]["campaign_key_digest"],
                    "play_order": segment["ticket"]["play_order"],
                    "in_session_ordinal": segment["in_session_ordinal"],
                    "session_nonce": segment["ticket"]["session_nonce"],
                    "process_start_identity": segment["ticket"]["process_start_identity"],
                    "epoch_token": segment["epoch_token"],
                })
            })
            .collect(),
    );
    graph["play_order_ledger_digest"] = json!(Sha256::hex(&canon(&ledger)));
    graph["stitch_input_digest"] =
        json!(canonical_stitch_input_digest(GAME, graph, segment_indexes));
}

fn reseal_entry_digests(entries: &mut [PackEntry]) {
    let mut segment_indexes: Vec<(String, Value)> = entries
        .iter()
        .filter(|entry| entry.path.contains("/segments/s") && entry.path.ends_with(".index.json"))
        .map(|entry| {
            (
                entry.path.clone(),
                serde_json::from_slice(&entry.bytes).expect("segment index fixture json"),
            )
        })
        .collect();
    segment_indexes.sort_by(|left, right| left.0.cmp(&right.0));
    let segment_indexes: Vec<Value> = segment_indexes
        .into_iter()
        .map(|(_, value)| value)
        .collect();
    let graph_entry = entries
        .iter_mut()
        .find(|entry| entry.path.ends_with("/segments.json"))
        .expect("segments fixture");
    let mut graph: Value = serde_json::from_slice(&graph_entry.bytes).expect("segments json");
    reseal_stitch_digests(&mut graph, &segment_indexes);
    graph_entry.bytes = canon(&graph);
}

fn reseal_entry_stitch_digest_only(entries: &mut [PackEntry]) {
    let mut segment_indexes: Vec<(String, Value)> = entries
        .iter()
        .filter(|entry| entry.path.contains("/segments/s") && entry.path.ends_with(".index.json"))
        .map(|entry| {
            (
                entry.path.clone(),
                serde_json::from_slice(&entry.bytes).expect("segment index fixture json"),
            )
        })
        .collect();
    segment_indexes.sort_by(|left, right| left.0.cmp(&right.0));
    let segment_indexes: Vec<Value> = segment_indexes
        .into_iter()
        .map(|(_, value)| value)
        .collect();
    let graph_entry = entries
        .iter_mut()
        .find(|entry| entry.path.ends_with("/segments.json"))
        .expect("segments fixture");
    let mut graph: Value = serde_json::from_slice(&graph_entry.bytes).expect("segments json");
    graph["stitch_input_digest"] = json!(canonical_stitch_input_digest(
        GAME,
        &graph,
        &segment_indexes
    ));
    graph_entry.bytes = canon(&graph);
}

fn identity() -> PackIdentity {
    PackIdentity {
        schema_version: Some("1".into()),
        build_exe_sha256: None,
        build_pinned: false,
        mod_identity: Some(json!({ "modset_id": MODSET })),
        created_utc: Some("2026-07-12T00:00:00Z".into()),
        content_manifest: None,
    }
}

fn stitched_opts() -> PackOptions {
    PackOptions {
        container_version: Some(2),
        required_features: Some(vec!["timeline_segments_v1".into()]),
        ..Default::default()
    }
}

fn build(opts: &PackOptions) -> Vec<u8> {
    pack_with(GAME, &stitched_entries(), &identity(), false, opts).expect("pack stitched")
}

fn build_entries(entries: &[PackEntry], opts: &PackOptions) -> Vec<u8> {
    pack_with(GAME, entries, &identity(), false, opts).expect("pack test container")
}

fn replace_json_entry(entries: &mut [PackEntry], suffix: &str, mutate: impl FnOnce(&mut Value)) {
    let entry = entries
        .iter_mut()
        .find(|e| e.path.ends_with(suffix))
        .unwrap_or_else(|| panic!("missing fixture entry {suffix}"));
    let mut value: Value = serde_json::from_slice(&entry.bytes).expect("fixture json");
    mutate(&mut value);
    entry.bytes = canon(&value);
    if suffix.ends_with("segments.json")
        || (suffix.contains("segments/s") && suffix.ends_with(".index.json"))
    {
        reseal_entry_digests(entries);
    }
}

fn replace_json_entry_unsealed(
    entries: &mut [PackEntry],
    suffix: &str,
    mutate: impl FnOnce(&mut Value),
) {
    let entry = entries
        .iter_mut()
        .find(|e| e.path.ends_with(suffix))
        .unwrap_or_else(|| panic!("missing fixture entry {suffix}"));
    let mut value: Value = serde_json::from_slice(&entry.bytes).expect("fixture json");
    mutate(&mut value);
    entry.bytes = canon(&value);
}

fn rewrite_play_order(entries: &mut [PackEntry], segment_id: u32, new_order: u64) {
    let graph_entry = entries
        .iter_mut()
        .find(|entry| entry.path.ends_with("segments.json"))
        .expect("segments fixture");
    let mut graph: Value = serde_json::from_slice(&graph_entry.bytes).unwrap();
    let source_key = {
        let segment = &mut graph["segments"][segment_id as usize];
        let source_key = segment["source_ref"]["source_key"]
            .as_str()
            .unwrap()
            .to_string();
        segment["play_order"] = json!(new_order);
        segment["ticket"]["play_order"] = json!(new_order);
        source_key
    };
    let source = graph["raw_sources"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|source| source["source_key"].as_str() == Some(&source_key))
        .unwrap();
    let participation = &mut source["participations"][0];
    participation["ticket"]["play_order"] = json!(new_order);
    let mut binding = participation.clone();
    binding.as_object_mut().unwrap().remove("participation_id");
    let participation_id = Sha256::hex(&canon(&binding));
    participation["participation_id"] = json!(participation_id);
    graph["segments"][segment_id as usize]["source_ref"]["participation_id"] =
        json!(participation_id);
    graph_entry.bytes = canon(&graph);

    let segment_suffix = format!("segments/s{segment_id:08}.index.json");
    replace_json_entry(entries, &segment_suffix, |index| {
        index["source_ref"]["participation_id"] = json!(participation_id);
        for collection in ["hours", "transition_coverage"] {
            for row in index[collection].as_array_mut().unwrap() {
                row["owner"]["play_order"] = json!(new_order);
                if let Some(observations) =
                    row.get_mut("observations").and_then(Value::as_array_mut)
                {
                    for observation in observations {
                        observation["owner"]["play_order"] = json!(new_order);
                    }
                }
            }
        }
    });
    replace_json_entry(entries, "index.json", |index| {
        for collection in ["snapshots", "coverage"] {
            for row in index[collection].as_array_mut().unwrap() {
                if row["owner"]["session_nonce"] == json!(format!("nonce-{segment_id}")) {
                    row["owner"]["play_order"] = json!(new_order);
                }
            }
        }
    });
}

fn make_sources_standalone_with_paths(entries: &mut Vec<PackEntry>, paths: [&str; 2]) {
    replace_json_entry(entries, "segments.json", |graph| {
        for (segment_id, path) in paths.into_iter().enumerate() {
            graph["raw_sources"][segment_id]["session_path"] = json!(path);
            graph["raw_sources"][segment_id]["storage"] = json!({"kind":"standalone"});
            graph["segments"][segment_id]["source_ref"]["session_path"] = json!(path);
        }
    });
    for (segment_id, path) in paths.into_iter().enumerate() {
        replace_json_entry(
            entries,
            &format!("segments/s{segment_id:08}.index.json"),
            |index| index["source_ref"]["session_path"] = json!(path),
        );
    }
    entries.retain(|entry| entry.role != "raw_source");
}

fn assert_rejects(bytes: Vec<u8>, needle: &str) {
    let result = Container::from_bytes(bytes, &ExpectedIdentity::default());
    assert!(
        matches!(result.as_ref().err(), Some(ContainerError::Reject(message)) if message.contains(needle)),
        "expected rejection containing {needle:?}, got {:?}",
        result.err().map(|e| e.to_string())
    );
}

fn local_header_offset(bytes: &[u8], name: &str) -> usize {
    let name_bytes = name.as_bytes();
    bytes
        .windows(name_bytes.len())
        .enumerate()
        .find_map(|(name_start, candidate)| {
            let header = name_start.checked_sub(30)?;
            (candidate == name_bytes
                && bytes.get(header..header + 4) == Some(&[0x50, 0x4b, 0x03, 0x04]))
            .then_some(header)
        })
        .unwrap_or_else(|| panic!("missing local file header for {name}"))
}

fn local_payload_offset(bytes: &[u8], name: &str) -> usize {
    let header = local_header_offset(bytes, name);
    let name_len = u16::from_le_bytes(bytes[header + 26..header + 28].try_into().unwrap()) as usize;
    let extra_len =
        u16::from_le_bytes(bytes[header + 28..header + 30].try_into().unwrap()) as usize;
    header + 30 + name_len + extra_len
}

fn central_header_offset(bytes: &[u8], name: &str) -> usize {
    let name_bytes = name.as_bytes();
    bytes
        .windows(name_bytes.len())
        .enumerate()
        .find_map(|(name_start, candidate)| {
            let header = name_start.checked_sub(46)?;
            (candidate == name_bytes
                && bytes.get(header..header + 4) == Some(&[0x50, 0x4b, 0x01, 0x02]))
            .then_some(header)
        })
        .unwrap_or_else(|| panic!("missing central-directory header for {name}"))
}

fn eocd_offset(bytes: &[u8]) -> usize {
    bytes
        .windows(4)
        .rposition(|candidate| candidate == [0x50, 0x4b, 0x05, 0x06])
        .expect("missing EOCD")
}

fn replace_once_same_len(bytes: &mut [u8], old: &[u8], new: &[u8]) {
    assert_eq!(old.len(), new.len(), "replacement must preserve length");
    let matches: Vec<usize> = bytes
        .windows(old.len())
        .enumerate()
        .filter_map(|(offset, candidate)| (candidate == old).then_some(offset))
        .collect();
    assert_eq!(matches.len(), 1, "expected exactly one byte-pattern match");
    let offset = matches[0];
    bytes[offset..offset + new.len()].copy_from_slice(new);
}

fn insert_zip64_end_records(bytes: &mut Vec<u8>) -> usize {
    let eocd = eocd_offset(bytes);
    let entries = u16::from_le_bytes(bytes[eocd + 10..eocd + 12].try_into().unwrap()) as u64;
    let central_size = u32::from_le_bytes(bytes[eocd + 12..eocd + 16].try_into().unwrap()) as u64;
    let central_offset = u32::from_le_bytes(bytes[eocd + 16..eocd + 20].try_into().unwrap()) as u64;

    let mut zip64 = Vec::with_capacity(76);
    zip64.extend_from_slice(&0x0606_4b50u32.to_le_bytes());
    zip64.extend_from_slice(&44u64.to_le_bytes());
    zip64.extend_from_slice(&45u16.to_le_bytes());
    zip64.extend_from_slice(&45u16.to_le_bytes());
    zip64.extend_from_slice(&0u32.to_le_bytes());
    zip64.extend_from_slice(&0u32.to_le_bytes());
    zip64.extend_from_slice(&entries.to_le_bytes());
    zip64.extend_from_slice(&entries.to_le_bytes());
    zip64.extend_from_slice(&central_size.to_le_bytes());
    zip64.extend_from_slice(&central_offset.to_le_bytes());
    zip64.extend_from_slice(&0x0706_4b50u32.to_le_bytes());
    zip64.extend_from_slice(&0u32.to_le_bytes());
    zip64.extend_from_slice(&(eocd as u64).to_le_bytes());
    zip64.extend_from_slice(&1u32.to_le_bytes());
    assert_eq!(zip64.len(), 76);

    bytes.splice(eocd..eocd, zip64);
    eocd + 76
}

fn assert_zip_rejects(bytes: Vec<u8>, needle: &str) {
    let error = match Container::from_bytes(bytes, &ExpectedIdentity::default()) {
        Err(ContainerError::Zip(error)) => error,
        Err(other) => panic!("expected ZIP rejection containing {needle:?}, got {other}"),
        Ok(_) => panic!("expected ZIP rejection containing {needle:?}"),
    };
    let message = error.to_string();
    assert!(
        message.contains(needle),
        "expected ZIP rejection containing {needle:?}, got {message:?}"
    );
}

#[test]
fn v2_stitched_container_opens_and_serves_main_timeline() {
    let c = Container::from_bytes(build(&stitched_opts()), &ExpectedIdentity::default())
        .expect("v2 stitched container opens");
    // the index is recognized as stitched.
    assert!(c.index().is_stitched());
    assert_eq!(c.index().segments_file.as_deref(), Some("segments.json"));
    // main timeline: exactly the two visible hours.
    let hours: Vec<i64> = c
        .index()
        .snapshots
        .iter()
        .filter_map(|s| s.hour_index)
        .collect();
    assert_eq!(hours, vec![0, 1]);
    // hour 1 resolves to the LATER segment's file (not segment 0's rollback copy).
    let h1 = c
        .index()
        .snapshots
        .iter()
        .find(|s| s.hour_index == Some(1))
        .unwrap();
    assert_eq!(h1.file, "s00000001.000000000001.snapshot.json");
    // load by hour works through the index.
    assert_eq!(c.load(0).unwrap().hour_index, Some(0));
    assert_eq!(c.load(1).unwrap().hour_index, Some(1));
}

#[test]
fn v2_swapped_equal_length_local_names_fail_at_open() {
    let mut bytes = build(&stitched_opts());
    let first = format!("payload/{GAME}/sessions/source-0.h4cr");
    let second = format!("payload/{GAME}/sessions/source-1.h4cr");
    assert_eq!(first.len(), second.len());

    let first_name = local_header_offset(&bytes, &first) + 30;
    let second_name = local_header_offset(&bytes, &second) + 30;
    let first_original = bytes[first_name..first_name + first.len()].to_vec();
    let second_original = bytes[second_name..second_name + second.len()].to_vec();
    bytes[first_name..first_name + first.len()].copy_from_slice(&second_original);
    bytes[second_name..second_name + second.len()].copy_from_slice(&first_original);

    assert_zip_rejects(bytes, "local/central name mismatch");
}

#[test]
fn v2_unread_raw_source_local_method_mismatch_fails_at_open() {
    let mut bytes = build(&stitched_opts());
    let raw_source = format!("payload/{GAME}/sessions/source-0.h4cr");
    let local_header = local_header_offset(&bytes, &raw_source);
    bytes[local_header + 8..local_header + 10].copy_from_slice(&8u16.to_le_bytes());

    assert_zip_rejects(bytes, "local/central method mismatch");
}

#[test]
fn v2_unread_store_raw_source_payload_crc_mismatch_fails_at_open() {
    let mut bytes = build(&stitched_opts());
    let raw_source = format!("payload/{GAME}/sessions/source-0.h4cr");
    let payload = local_payload_offset(&bytes, &raw_source);
    bytes[payload] ^= 0x01;

    assert_zip_rejects(bytes, "payload crc32 mismatch");
}

#[test]
fn v2_manifest_downgrade_cannot_skip_whole_archive_payload_crc_validation() {
    let mut bytes = build(&stitched_opts());
    let manifest_header = local_header_offset(&bytes, "manifest.json");
    let manifest_len = u32::from_le_bytes(
        bytes[manifest_header + 22..manifest_header + 26]
            .try_into()
            .unwrap(),
    ) as usize;
    let manifest_payload = local_payload_offset(&bytes, "manifest.json");
    let mut manifest_bytes = bytes[manifest_payload..manifest_payload + manifest_len].to_vec();

    replace_once_same_len(
        &mut manifest_bytes,
        br#""container_version":2"#,
        br#""container_version":1"#,
    );
    replace_once_same_len(
        &mut manifest_bytes,
        br#"["timeline_segments_v1"]"#,
        b"[                      ]",
    );
    let manifest: Value = serde_json::from_slice(&manifest_bytes).expect("downgraded manifest");
    let old_digest = manifest["manifest_digest"]
        .as_str()
        .expect("manifest digest")
        .as_bytes()
        .to_vec();
    let new_digest = Sha256::hex(&manifest_canonical_bytes(&manifest));
    replace_once_same_len(&mut manifest_bytes, &old_digest, new_digest.as_bytes());
    bytes[manifest_payload..manifest_payload + manifest_len].copy_from_slice(&manifest_bytes);

    let raw_source = format!("payload/{GAME}/sessions/source-0.h4cr");
    let raw_payload = local_payload_offset(&bytes, &raw_source);
    bytes[raw_payload] ^= 0x01;

    assert_zip_rejects(bytes, "payload crc32 mismatch");
}

#[test]
fn v2_zip64_contradictory_ordinary_eocd_metadata_fails_at_open() {
    let mut bytes = build(&stitched_opts());
    let eocd = insert_zip64_end_records(&mut bytes);
    let central_size = u32::from_le_bytes(bytes[eocd + 12..eocd + 16].try_into().unwrap());
    bytes[eocd + 12..eocd + 16].copy_from_slice(&(central_size + 1).to_le_bytes());

    assert_zip_rejects(bytes, "ordinary EOCD central-directory size");
}

#[test]
fn v2_duplicate_conflicting_zip64_extra_fields_fail_at_open() {
    let mut bytes = build(&stitched_opts());
    let raw_source = format!("payload/{GAME}/sessions/source-0.h4cr");
    let central_header = central_header_offset(&bytes, &raw_source);
    let name_len = u16::from_le_bytes(
        bytes[central_header + 28..central_header + 30]
            .try_into()
            .unwrap(),
    ) as usize;
    let original_uncompressed_size = u32::from_le_bytes(
        bytes[central_header + 24..central_header + 28]
            .try_into()
            .unwrap(),
    ) as u64;
    bytes[central_header + 24..central_header + 28].copy_from_slice(&u32::MAX.to_le_bytes());

    let extra_start = central_header + 46 + name_len;
    let mut duplicate_fields = Vec::with_capacity(24);
    for size in [original_uncompressed_size, original_uncompressed_size + 1] {
        duplicate_fields.extend_from_slice(&1u16.to_le_bytes());
        duplicate_fields.extend_from_slice(&8u16.to_le_bytes());
        duplicate_fields.extend_from_slice(&size.to_le_bytes());
    }
    bytes.splice(extra_start..extra_start, duplicate_fields);
    bytes[central_header + 30..central_header + 32].copy_from_slice(&24u16.to_le_bytes());

    let eocd = eocd_offset(&bytes);
    let central_size = u32::from_le_bytes(bytes[eocd + 12..eocd + 16].try_into().unwrap());
    bytes[eocd + 12..eocd + 16].copy_from_slice(&(central_size + 24).to_le_bytes());

    assert_zip_rejects(bytes, "duplicate Zip64 extra field");
}

#[test]
fn v2_central_directory_inside_eocd_comment_fails_at_open() {
    let mut bytes = build(&stitched_opts());
    let eocd = eocd_offset(&bytes);
    let central_size = u32::from_le_bytes(bytes[eocd + 12..eocd + 16].try_into().unwrap()) as usize;
    let central_offset =
        u32::from_le_bytes(bytes[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
    let copied_central_directory = bytes[central_offset..central_offset + central_size].to_vec();
    let comment_offset = eocd + 22;
    let comment_len = u16::try_from(central_size).expect("fixture central directory fits comment");

    bytes.extend_from_slice(&copied_central_directory);
    bytes[eocd + 16..eocd + 20].copy_from_slice(&(comment_offset as u32).to_le_bytes());
    bytes[eocd + 20..eocd + 22].copy_from_slice(&comment_len.to_le_bytes());

    assert_zip_rejects(bytes, "central directory does not end");
}

#[test]
fn v2_central_directory_nonzero_disk_start_fails_at_open() {
    let base = build(&stitched_opts());
    let raw_source = format!("payload/{GAME}/sessions/source-0.h4cr");
    let mut bytes = base.clone();
    let central_header = central_header_offset(&bytes, &raw_source);
    bytes[central_header + 34..central_header + 36].copy_from_slice(&1u16.to_le_bytes());
    assert_zip_rejects(bytes, "multi-disk");

    for disk_field_offset in [4, 6] {
        let mut bytes = base.clone();
        let eocd = eocd_offset(&bytes);
        bytes[eocd + disk_field_offset..eocd + disk_field_offset + 2]
            .copy_from_slice(&1u16.to_le_bytes());
        assert_zip_rejects(bytes, "multi-disk");
    }
}

#[test]
fn v2_zip64_sentinel_with_zero_length_zip64_extra_fails_at_open() {
    let mut bytes = build(&stitched_opts());
    let raw_source = format!("payload/{GAME}/sessions/source-0.h4cr");
    let central_header = central_header_offset(&bytes, &raw_source);
    let name_len = u16::from_le_bytes(
        bytes[central_header + 28..central_header + 30]
            .try_into()
            .unwrap(),
    ) as usize;
    let extra_len = u16::from_le_bytes(
        bytes[central_header + 30..central_header + 32]
            .try_into()
            .unwrap(),
    );
    assert_eq!(extra_len, 0);

    let original_uncompressed_size = u32::from_le_bytes(
        bytes[central_header + 24..central_header + 28]
            .try_into()
            .unwrap(),
    ) as u64;
    bytes[central_header + 24..central_header + 28].copy_from_slice(&u32::MAX.to_le_bytes());

    // The Zip64 field declares a zero-byte body. The following eight bytes are
    // deliberately inside the overall extra area so an incorrectly whole-extra-
    // bounded parser can consume them as the sentinel-required size.
    let extra_start = central_header + 46 + name_len;
    let mut malformed_extra = Vec::with_capacity(12);
    malformed_extra.extend_from_slice(&1u16.to_le_bytes());
    malformed_extra.extend_from_slice(&0u16.to_le_bytes());
    malformed_extra.extend_from_slice(&original_uncompressed_size.to_le_bytes());
    bytes.splice(extra_start..extra_start, malformed_extra);
    bytes[central_header + 30..central_header + 32].copy_from_slice(&12u16.to_le_bytes());

    let eocd = eocd_offset(&bytes);
    let central_size = u32::from_le_bytes(bytes[eocd + 12..eocd + 16].try_into().unwrap());
    bytes[eocd + 12..eocd + 16].copy_from_slice(&(central_size + 12).to_le_bytes());

    assert_zip_rejects(bytes, "zip64 extra");
}

#[test]
fn v2_stale_stitch_input_digest_fails_with_regenerated_manifest() {
    let mut entries = stitched_entries();
    replace_json_entry_unsealed(&mut entries, "segments.json", |graph| {
        graph["fold_version"] = json!("world-fold-fixture-v2")
    });
    assert_rejects(
        build_entries(&entries, &stitched_opts()),
        "stitch_input_digest",
    );
}

#[test]
fn v2_arbitrary_nonempty_stitch_input_digest_fails_with_regenerated_manifest() {
    let mut entries = stitched_entries();
    replace_json_entry_unsealed(&mut entries, "segments.json", |graph| {
        graph["stitch_input_digest"] = json!("ee".repeat(32))
    });
    assert_rejects(
        build_entries(&entries, &stitched_opts()),
        "stitch_input_digest",
    );
}

#[test]
fn v2_non_sha256_stitch_input_digest_fails_with_regenerated_manifest() {
    let mut entries = stitched_entries();
    replace_json_entry_unsealed(&mut entries, "segments.json", |graph| {
        graph["stitch_input_digest"] = json!("not-a-sha256")
    });
    assert_rejects(
        build_entries(&entries, &stitched_opts()),
        "stitch_input_digest",
    );
}

#[test]
fn v2_segment_evidence_tamper_cannot_bypass_stitch_digest() {
    let mut entries = stitched_entries();
    replace_json_entry_unsealed(&mut entries, "segments/s00000000.index.json", |index| {
        index["hours"][0]["selected_observation_id"] = json!("handcrafted-observation");
        index["hours"][0]["observations"][0]["observation_id"] = json!("handcrafted-observation");
    });
    replace_json_entry_unsealed(&mut entries, "index.json", |index| {
        index["snapshots"][0]["selected_observation_id"] = json!("handcrafted-observation");
    });
    assert_rejects(
        build_entries(&entries, &stitched_opts()),
        "stitch_input_digest",
    );
}

#[test]
fn v2_duplicate_and_reversed_ticket_authority_fail_at_open() {
    let mut duplicate = stitched_entries();
    rewrite_play_order(&mut duplicate, 1, 0);
    assert_rejects(build_entries(&duplicate, &stitched_opts()), "play_order");

    let mut reversed = stitched_entries();
    rewrite_play_order(&mut reversed, 0, 1);
    rewrite_play_order(&mut reversed, 1, 0);
    assert_rejects(build_entries(&reversed, &stitched_opts()), "play_order");
}

#[test]
fn v2_forged_play_order_ledger_claim_fails_even_with_matching_stitch_digest() {
    let mut entries = stitched_entries();
    replace_json_entry_unsealed(&mut entries, "segments.json", |graph| {
        graph["play_order_ledger_digest"] = json!("ff".repeat(32));
    });
    reseal_entry_stitch_digest_only(&mut entries);
    assert_rejects(
        build_entries(&entries, &stitched_opts()),
        "play_order_ledger_digest",
    );
}

#[test]
fn v2_windows_source_aliases_collide_but_distinct_sources_remain_distinct() {
    let mut aliased = stitched_entries();
    make_sources_standalone_with_paths(
        &mut aliased,
        [r"C:\Replay\S.h4cr", r"\\?\c:/replay/./s.h4cr"],
    );
    assert_rejects(
        build_entries(&aliased, &stitched_opts()),
        "session_path aliases distinct raw source identities",
    );

    let mut unc_aliased = stitched_entries();
    make_sources_standalone_with_paths(
        &mut unc_aliased,
        [
            r"\\Server\Share\Replay\S.h4cr",
            r"\\?\UNC\server\share\replay\.\s.h4cr",
        ],
    );
    assert_rejects(
        build_entries(&unc_aliased, &stitched_opts()),
        "session_path aliases distinct raw source identities",
    );

    let mut unicode_aliased = stitched_entries();
    make_sources_standalone_with_paths(
        &mut unicode_aliased,
        ["C:\\Replay\\Cafe\u{301}.h4cr", "c:/replay/caf\u{e9}.h4cr"],
    );
    assert_rejects(
        build_entries(&unicode_aliased, &stitched_opts()),
        "session_path aliases distinct raw source identities",
    );

    let mut distinct = stitched_entries();
    make_sources_standalone_with_paths(&mut distinct, [r"C:\Replay\S.h4cr", r"c:/replay/T.h4cr"]);
    Container::from_bytes(
        build_entries(&distinct, &stitched_opts()),
        &ExpectedIdentity::default(),
    )
    .expect("genuinely distinct normalized sources must remain distinct");
}

#[test]
fn v2_standalone_raw_source_must_not_remain_embedded() {
    let mut entries = stitched_entries();
    replace_json_entry(&mut entries, "segments.json", |graph| {
        graph["raw_sources"][0]["storage"] = json!({"kind":"standalone"});
    });
    assert_rejects(
        build_entries(&entries, &stitched_opts()),
        "standalone raw source must not be embedded",
    );
}

#[test]
fn superseded_rollback_snapshot_is_retained_and_loadable() {
    let c = Container::from_bytes(build(&stitched_opts()), &ExpectedIdentity::default()).unwrap();
    // the superseded hour-1 snapshot (segment 0) is NOT in the main index...
    assert!(!c
        .index()
        .snapshots
        .iter()
        .any(|s| s.file == "s00000000.000000000001.snapshot.json"));
    // ...but is retained, manifest-listed, and loadable by explicit file name.
    let rollback = c
        .load_file("s00000000.000000000001.snapshot.json")
        .expect("rollback snapshot retained and loadable");
    assert_eq!(rollback.hour_index, Some(1));
    // full verification passes (every retained entry hash-checks).
    c.verify_all().expect("verify_all incl. rollback entry");
}

#[test]
fn segments_graph_parses_with_spans() {
    let c = Container::from_bytes(build(&stitched_opts()), &ExpectedIdentity::default()).unwrap();
    let graph = c
        .segments()
        .expect("read segments.json")
        .expect("graph present");
    assert_eq!(graph.stream_major, Some(2));
    assert_eq!(graph.segments.len(), 2);
    let seg0 = graph.segments.iter().find(|s| s.segment_id == 0).unwrap();
    assert_eq!(seg0.head_hour, Some(1));
    assert_eq!(seg0.visible_spans.len(), 1);
    assert_eq!(seg0.superseded_spans.len(), 1);
    assert_eq!(
        seg0.superseded_spans[0]["superseded_by"].as_i64(),
        Some(1),
        "segment 0's hour 1 superseded_by segment 1"
    );
    let seg1 = graph.segments.iter().find(|s| s.segment_id == 1).unwrap();
    assert_eq!(seg1.parent_segment, Some(0));
    assert!(seg1.superseded_spans.is_empty());
}

#[test]
fn v2_without_feature_fails_closed() {
    // §9 coupling: a v2 container that omits `timeline_segments_v1` is malformed.
    let opts = PackOptions {
        container_version: Some(2),
        required_features: None,
        ..Default::default()
    };
    let res = Container::from_bytes(build(&opts), &ExpectedIdentity::default());
    assert!(
        matches!(res.as_ref().err(), Some(ContainerError::Reject(m)) if m.contains("timeline_segments_v1")),
        "v2 without the segment feature must reject, got {:?}",
        res.err().map(|e| e.to_string())
    );
}

#[test]
fn v1_with_stitched_feature_fails_closed() {
    let opts = PackOptions {
        container_version: Some(1),
        required_features: Some(vec!["timeline_segments_v1".into()]),
        ..Default::default()
    };
    assert_rejects(build(&opts), "container_version");
}

#[test]
fn v2_with_v1_index_and_no_segment_graph_fails_closed() {
    let plain = vec![
        ent(
            format!("payload/{GAME}/index.json"),
            canon(&json!({
                "game_id": GAME,
                "schema_version": "1",
                "snapshots": [
                    { "hour_index": 0, "file": "000000000000.snapshot.json" }
                ]
            })),
            "index",
            None,
        ),
        ent(
            format!("payload/{GAME}/000000000000.snapshot.json"),
            snap_bytes(0),
            "snapshot",
            Some(0),
        ),
    ];
    assert_rejects(build_entries(&plain, &stitched_opts()), "stitched-main-v1");
}

#[test]
fn v2_missing_segment_index_fails_at_open() {
    let mut entries = stitched_entries();
    entries.retain(|e| !e.path.ends_with("segments/s00000001.index.json"));
    assert_rejects(build_entries(&entries, &stitched_opts()), "segment index");
}

#[test]
fn v2_unknown_coverage_discriminant_fails_at_open() {
    let mut entries = stitched_entries();
    replace_json_entry(&mut entries, "segments/s00000000.index.json", |index| {
        index["hours"][0]["kind"] = json!("future_kind")
    });
    assert_rejects(build_entries(&entries, &stitched_opts()), "discriminant");
}

#[test]
fn v2_truncated_segment_graph_fails_at_open() {
    let mut entries = stitched_entries();
    let graph = entries
        .iter_mut()
        .find(|e| e.path.ends_with("segments.json"))
        .expect("segments fixture");
    graph.bytes = br#"{"segments":["#.to_vec();
    let result = Container::from_bytes(
        build_entries(&entries, &stitched_opts()),
        &ExpectedIdentity::default(),
    );
    assert!(
        result.is_err(),
        "truncated segments.json must fail during open"
    );
}

#[test]
fn v2_duplicate_visible_hour_fails_at_open() {
    let mut entries = stitched_entries();
    replace_json_entry(&mut entries, "index.json", |index| {
        index["snapshots"][1]["hour_index"] = json!(0);
    });
    assert_rejects(
        build_entries(&entries, &stitched_opts()),
        "duplicate visible hour",
    );
}

#[test]
fn v2_segment_snapshot_hash_disagreement_fails_at_open() {
    let mut entries = stitched_entries();
    replace_json_entry(&mut entries, "segments/s00000001.index.json", |index| {
        index["hours"][0]["sha256"] = json!("00".repeat(32));
    });
    assert_rejects(
        build_entries(&entries, &stitched_opts()),
        "graph/index/hash disagreement",
    );
}

#[test]
fn v2_main_slices_disagreement_fails_at_open() {
    let mut entries = stitched_entries();
    replace_json_entry(&mut entries, "segments.json", |graph| {
        graph["main_slices"][1]["segment_id"] = json!(0);
    });
    assert_rejects(
        build_entries(&entries, &stitched_opts()),
        "main_slices disagree",
    );
}

#[test]
fn v2_unclassified_retained_hour_fails_at_open() {
    let mut entries = stitched_entries();
    replace_json_entry(&mut entries, "segments.json", |graph| {
        graph["segments"][0]["superseded_spans"] = json!([]);
    });
    assert_rejects(
        build_entries(&entries, &stitched_opts()),
        "do not classify every retained hour",
    );
}

#[test]
fn v2_missing_fold_or_writer_input_fails_at_open() {
    let mut entries = stitched_entries();
    replace_json_entry(&mut entries, "segments.json", |graph| {
        graph.as_object_mut().unwrap().remove("fold_version");
    });
    assert_rejects(build_entries(&entries, &stitched_opts()), "fold_version");
}

#[test]
fn v2_repeated_observation_count_drift_fails_at_open() {
    let mut entries = stitched_entries();
    replace_json_entry(&mut entries, "segments/s00000000.index.json", |index| {
        index["hours"][0]["observation_count"] = json!(2);
    });
    assert_rejects(
        build_entries(&entries, &stitched_opts()),
        "observation_count",
    );

    let mut entries = stitched_entries();
    replace_json_entry(&mut entries, "segments/s00000000.index.json", |index| {
        index["hours"][1]["source_record_seq_bounds"] = json!([0, 1]);
        index["hours"][1]["source_byte_bounds"] = json!([0, 100]);
        index["hours"][1]["observations"][0]["record_seq_bounds"] = json!([0, 1]);
        index["hours"][1]["observations"][0]["byte_bounds"] = json!([0, 100]);
    });
    assert_rejects(
        build_entries(&entries, &stitched_opts()),
        "overlaps across hours",
    );
}

#[test]
fn v2_selected_observation_owner_drift_fails_at_open() {
    let mut entries = stitched_entries();
    replace_json_entry(&mut entries, "segments/s00000000.index.json", |index| {
        index["hours"][0]["owner"]["session_nonce"] = json!("foreign-session");
    });
    assert_rejects(build_entries(&entries, &stitched_opts()), "owner disagrees");
}

#[test]
fn v2_embedded_cross_campaign_source_fails_at_open() {
    let mut entries = stitched_entries();
    replace_json_entry(&mut entries, "segments.json", |graph| {
        let participation = &mut graph["raw_sources"][0]["participations"][0];
        participation["ticket"]["campaign_key_digest"] = json!("bb".repeat(32));
        let mut binding = participation.clone();
        binding.as_object_mut().unwrap().remove("participation_id");
        participation["participation_id"] = json!(Sha256::hex(&canon(&binding)));
    });
    assert_rejects(
        build_entries(&entries, &stitched_opts()),
        "cross-campaign participation",
    );
}

#[test]
fn v2_parent_rewritten_from_final_graph_fails_at_open() {
    let mut entries = stitched_entries();
    replace_json_entry(&mut entries, "segments.json", |graph| {
        graph["segments"][1]["parent_segment"] = Value::Null;
    });
    assert_rejects(build_entries(&entries, &stitched_opts()), "insertion time");
}

#[test]
fn version_above_reader_max_rejects() {
    // a future v3 (feature declared, so it clears the coupling gate) is still above
    // the reader max and must reject — forward-compat fail-closed.
    let opts = PackOptions {
        container_version: Some(3),
        required_features: Some(vec!["timeline_segments_v1".into()]),
        ..Default::default()
    };
    let res = Container::from_bytes(build(&opts), &ExpectedIdentity::default());
    assert!(
        matches!(res.as_ref().err(), Some(ContainerError::Reject(m)) if m.contains("reader max")),
        "container_version above max must reject, got {:?}",
        res.err().map(|e| e.to_string())
    );
}

#[test]
fn plain_v1_container_has_no_segment_graph() {
    // a plain v1 container still opens and reports no stitched graph (back-compat).
    let plain = vec![
        ent(
            format!("payload/{GAME}/index.json"),
            canon(&json!({
                "game_id": GAME, "schema_version": "1",
                "snapshots": [ { "hour_index": 0, "file": "000000000000.snapshot.json" } ]
            })),
            "index",
            None,
        ),
        ent(
            format!("payload/{GAME}/000000000000.snapshot.json"),
            snap_bytes(0),
            "snapshot",
            Some(0),
        ),
    ];
    let bytes = pack_with(GAME, &plain, &identity(), false, &PackOptions::default()).unwrap();
    let c = Container::from_bytes(bytes, &ExpectedIdentity::default()).unwrap();
    assert!(!c.index().is_stitched());
    assert!(
        c.segments().unwrap().is_none(),
        "no segments.json in a v1 container"
    );
}
