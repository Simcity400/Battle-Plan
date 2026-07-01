use crate::decoder_gate::decode_domain53_record;
use crate::DOMAIN_ID;
use serde_json::json;
use snapshot::{pack, Container, ExpectedIdentity, PackEntry, PackIdentity};
use std::fs;
use std::path::Path;

pub const REPLAY_SCHEMA_VERSION: &str = "native_collector_session_v1";
pub const GAME_ID: &str = "domain53_lane_local";
pub const STREAM_PATH: &str = "payload/domain53_lane_local/collector_raw/collector_stream.bin";
pub const MANIFEST_PATH: &str =
    "payload/domain53_lane_local/recorder/native_session_manifest.json";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayVerification {
    pub replay_bytes: usize,
    pub stream_bytes: usize,
    pub collector_stream_entries: usize,
    pub recorder_manifest_entries: usize,
}

pub fn manifest_json(stream_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let gate = decode_domain53_record(0, stream_bytes);
    if !gate.parse_errors.is_empty() {
        return Err(format!(
            "domain53 compact stream failed gate: {:?}",
            gate.parse_errors
        ));
    }
    serde_json::to_vec_pretty(&json!({
        "session_id": "domain53_lane_local",
        "replay_game_id": GAME_ID,
        "stream_format": "compact",
        "stream_role": "collector_compact_stream",
        "stream_file": "collector_stream.bin",
        "scheduled_domains": ["diplomacy_intel_operatives_live"],
        "extraction_perf_gate": true,
        "parse_errors": [],
        "ordering_errors": [],
        "game_hours_advanced": 0,
        "skipped_game_hours": 0,
        "unique_game_hour_count": 1,
        "hourly_requirement": {
            "required": false,
            "met": null,
            "problems": ["lane-local single-record replay container; shared DLL coordinator slot not owned"]
        },
        "compact_stream": {
            "record_magic": "H4CR",
            "record_header_len": 32,
            "record_version": 1,
            "decodable": true,
            "ordered": true,
            "total_bytes": stream_bytes.len(),
            "max_record_len": stream_bytes.len(),
            "record_count": gate.record_count,
            "domain_data_record_count": gate.record_count,
            "frame_end_record_count": 0,
            "status_flags_or": gate.status_flags_or,
            "worker_panic_seen": false,
            "bridge_unavailable_seen": false,
            "source_unavailable_seen": false,
            "partial_data_seen": false,
            "expected_domains": ["diplomacy_intel_operatives_live"],
            "domains_seen": ["diplomacy_intel_operatives_live"],
            "per_domain": {
                "diplomacy_intel_operatives_live": {
                    "domain_id": DOMAIN_ID,
                    "data_records": gate.record_count,
                    "payload_bytes": gate.payload_bytes_total,
                    "status_flags_or": gate.status_flags_or,
                    "data_rows_total": gate.data_rows_total,
                    "data_changed_rows_total": gate.data_changed_rows_total,
                    "data_units_total": {
                        "diplomacy_intel_rows": gate.data_rows_total
                    },
                    "omitted_unresolved_fields": [
                        "operation_detail",
                        "intel_map_percentages",
                        "incoming_diplomatic_action_queue"
                    ]
                }
            }
        },
        "dll_slot_blocker": "shared integration slot not owned by diplomacy_intel_operatives_live"
    }))
    .map_err(|err| err.to_string())
}

pub fn pack_replay(stream_bytes: &[u8], manifest_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let index_bytes = serde_json::to_vec_pretty(&json!({
        "game_id": GAME_ID,
        "schema_version": REPLAY_SCHEMA_VERSION,
        "snapshots": [],
    }))
    .map_err(|err| err.to_string())?;
    let timeline_bytes = serde_json::to_vec_pretty(&json!({
        "format": "hoi4_compact_stream_timeline_v1",
        "stream_format": "compact",
        "stream": "collector_raw/collector_stream.bin",
        "session_manifest": "recorder/native_session_manifest.json",
        "raw_stream_role": "collector_compact_stream",
        "session_manifest_role": "recorder_manifest",
    }))
    .map_err(|err| err.to_string())?;
    let entries = vec![
        PackEntry {
            path: format!("payload/{GAME_ID}/index.json"),
            bytes: index_bytes,
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{GAME_ID}/timeline.json"),
            bytes: timeline_bytes,
            role: "timeline",
            hour_index: None,
        },
        PackEntry {
            path: STREAM_PATH.to_string(),
            bytes: stream_bytes.to_vec(),
            role: "collector_compact_stream",
            hour_index: None,
        },
        PackEntry {
            path: MANIFEST_PATH.to_string(),
            bytes: manifest_bytes.to_vec(),
            role: "recorder_manifest",
            hour_index: None,
        },
    ];
    let identity = PackIdentity {
        schema_version: Some(REPLAY_SCHEMA_VERSION.to_string()),
        build_exe_sha256: Some(crate::extractor::EXPECTED_HOI4_SHA256_870EBF.to_string()),
        build_pinned: true,
        created_utc: Some("lane-local-domain53-proof".to_string()),
        ..Default::default()
    };
    pack(GAME_ID, &entries, &identity, true).map_err(|err| err.to_string())
}

pub fn verify_replay_bytes(
    replay_bytes: &[u8],
    expected_stream: &[u8],
) -> Result<ReplayVerification, String> {
    let container = Container::from_bytes(replay_bytes.to_vec(), &ExpectedIdentity::default())
        .map_err(|err| err.to_string())?;
    container.verify_all().map_err(|err| err.to_string())?;
    let stream_entries = container.entries_by_role("collector_compact_stream");
    if stream_entries.len() != 1 {
        return Err(format!(
            "collector_compact_stream entries {} != 1",
            stream_entries.len()
        ));
    }
    let manifest_entries = container.entries_by_role("recorder_manifest");
    if manifest_entries.len() != 1 {
        return Err(format!(
            "recorder_manifest entries {} != 1",
            manifest_entries.len()
        ));
    }
    let stored_stream = container
        .read_entry_bytes(&stream_entries[0].path)
        .map_err(|err| err.to_string())?;
    if stored_stream != expected_stream {
        return Err("collector_compact_stream bytes differ after replay pack".to_string());
    }
    Ok(ReplayVerification {
        replay_bytes: replay_bytes.len(),
        stream_bytes: stored_stream.len(),
        collector_stream_entries: stream_entries.len(),
        recorder_manifest_entries: manifest_entries.len(),
    })
}

pub fn write_replay_artifacts(
    stream_path: &Path,
    manifest_path: &Path,
    replay_path: &Path,
    stream_bytes: &[u8],
) -> Result<ReplayVerification, String> {
    let manifest = manifest_json(stream_bytes)?;
    let replay = pack_replay(stream_bytes, &manifest)?;
    let verification = verify_replay_bytes(&replay, stream_bytes)?;
    if let Some(parent) = stream_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    if let Some(parent) = manifest_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    if let Some(parent) = replay_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    fs::write(stream_path, stream_bytes).map_err(|err| err.to_string())?;
    fs::write(manifest_path, manifest).map_err(|err| err.to_string())?;
    fs::write(replay_path, replay).map_err(|err| err.to_string())?;
    Ok(verification)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder_gate::encode_domain53_data_record;
    use crate::extractor::{
        build_domain53_payload, proof_fixture_rows, CURRENT_VALIDATED_SOURCE_PROOFS,
    };

    #[test]
    fn packs_and_verifies_domain53_replay_container() {
        let extraction = build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);
        assert!(extraction.ok(), "{:?}", extraction.errors);
        let stream = encode_domain53_data_record(1, 43_800_024, &extraction.payload);
        let manifest = manifest_json(&stream).unwrap();
        let replay = pack_replay(&stream, &manifest).unwrap();

        let verified = verify_replay_bytes(&replay, &stream).unwrap();

        assert_eq!(verified.stream_bytes, stream.len());
        assert_eq!(verified.collector_stream_entries, 1);
        assert_eq!(verified.recorder_manifest_entries, 1);
    }
}
