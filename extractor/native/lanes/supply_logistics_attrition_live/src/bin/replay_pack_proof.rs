#![recursion_limit = "256"]

use compact_contract::{
    parse_compact_record_header, validate_compact_record_semantics, RecordFramingError,
};
use serde_json::json;
use snapshot::{pack, Container, ExpectedIdentity, PackEntry, PackIdentity};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "../../domain44_supply_logistics_attrition.rs"]
mod domain44;

const REPLAY_SCHEMA_VERSION: &str = "native_collector_session_v1";
const DEFAULT_GAME_ID: &str = "supply_logistics_attrition_live_lane_local";
const STREAM_ENTRY_PATH: &str = "collector_raw/collector_stream.bin";
const MANIFEST_ENTRY_PATH: &str = "recorder/native_session_manifest.json";
const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
const COMPACT_RECORD_HEADER_LEN: usize = 32;
const COMPACT_RECORD_VERSION: u16 = 1;
const RECORD_TYPE_FRAME_END: u16 = 2;
const RECORD_TYPE_DOMAIN_DATA: u16 = 3;
const LANE_FIXTURE_CLOSED_FRAMES: u64 = 1;
const BUILD_SHA256: &str = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let options = Options::parse()?;
    let payload = fs::read(&options.payload)
        .map_err(|err| format!("read {} failed: {err}", options.payload.display()))?;
    let payload_summary = domain44::decode_payload(0, &payload);
    if !payload_summary.parse_errors.is_empty() {
        return Err(format!(
            "domain44 payload gate failed before stream pack: {:?}",
            payload_summary.parse_errors
        ));
    }
    if payload_summary.csupply_input_rows_total == 0
        && payload_summary.pressure_candidate_rows_total == 0
    {
        return Err(
            "domain44 payload has zero csupply input rows and zero pressure candidate rows"
                .to_string(),
        );
    }

    fs::create_dir_all(&options.out_dir).map_err(|err| format!("create out dir failed: {err}"))?;
    let stream = build_stream(&payload, &payload_summary);
    let stream_gate = gate_stream(&stream)?;
    if !stream_gate.ok {
        return Err(format!(
            "domain44 compact stream gate failed before replay pack: {:?}",
            stream_gate.problems
        ));
    }
    if stream_gate.unique_game_hours != LANE_FIXTURE_CLOSED_FRAMES
        || stream_gate.observed_game_hour_advancements != Some(0)
    {
        return Err(format!(
            "lane-local fixture duration authority is inconsistent: closed_frames={} advancements={:?}",
            stream_gate.unique_game_hours, stream_gate.observed_game_hour_advancements
        ));
    }

    let manifest = manifest_json(&options, &stream, &stream_gate, &payload_summary)?;
    let replay = pack_replay(&options.game_id, &stream, &manifest)?;
    let verification = verify_replay(&options.game_id, &replay, &stream)?;

    let stream_out = options.out_dir.join("collector_stream.bin");
    let manifest_out = options.out_dir.join("native_session_manifest.json");
    let replay_out = options.out_dir.join("native_session.battleplan");
    let proof_out = options.out_dir.join("replay_pack_proof.json");
    let proof_md = options.out_dir.join("REPLAY_PACK_PROOF.md");
    fs::write(&stream_out, &stream).map_err(|err| format!("write stream failed: {err}"))?;
    fs::write(&manifest_out, &manifest).map_err(|err| format!("write manifest failed: {err}"))?;
    write_atomic(&replay_out, &replay)?;

    let proof = json!({
        "result": "verified",
        "schema": "supply_logistics_attrition_live.replay_pack_proof.v1",
        "domain": domain44::DOMAIN_NAME,
        "domain_id": domain44::DOMAIN_ID,
        "source_payload": options.payload.display().to_string(),
        "out_dir": options.out_dir.display().to_string(),
        "game_id": options.game_id,
        "collector_stream_path": stream_out.display().to_string(),
        "manifest_path": manifest_out.display().to_string(),
        "hoi4replay_path": replay_out.display().to_string(),
        "collector_stream_bytes": stream.len(),
        "hoi4replay_bytes": replay.len(),
        "verified_stream_entry": verification.stream_entry,
        "verified_stream_bytes": verification.stream_bytes,
        "collector_compact_stream_entries": verification.collector_stream_entries,
        "recorder_manifest_entries": verification.recorder_manifest_entries,
        "payload_gate": payload_summary_json(&payload_summary),
        "stream_gate": stream_gate.to_json(),
    });
    let proof_bytes = serde_json::to_vec_pretty(&proof).map_err(|err| err.to_string())?;
    fs::write(&proof_out, &proof_bytes).map_err(|err| format!("write proof failed: {err}"))?;
    fs::write(&proof_md, proof_markdown(&proof))
        .map_err(|err| format!("write proof md failed: {err}"))?;
    println!("{}", String::from_utf8_lossy(&proof_bytes));
    Ok(())
}

fn build_stream(payload: &[u8], summary: &domain44::Domain44GateSummary) -> Vec<u8> {
    let mut stream = Vec::new();
    append_compact_record(
        &mut stream,
        1,
        1,
        domain44::DOMAIN_ID,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
        &build_perf_payload(summary, read_u64_le(payload, 0)),
    );
    append_compact_record(
        &mut stream,
        2,
        1,
        domain44::DOMAIN_ID,
        RECORD_TYPE_DOMAIN_DATA,
        payload,
    );
    append_compact_record(
        &mut stream,
        3,
        1,
        compact_contract::COMPACT_DOMAIN_FRAME,
        RECORD_TYPE_FRAME_END,
        &build_frame_end_payload(0),
    );
    stream
}

fn build_frame_end_payload(
    status_flags: u32,
) -> [u8; compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN] {
    let mut payload = [0u8; compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN];
    payload[16..20].copy_from_slice(&1u32.to_le_bytes());
    payload[20..24].copy_from_slice(&1u32.to_le_bytes());
    payload[24..28].copy_from_slice(&status_flags.to_le_bytes());
    payload
}

fn build_perf_payload(
    summary: &domain44::Domain44GateSummary,
    elapsed_wall_ms: u64,
) -> [u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN] {
    let mut payload = [0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN];
    let row_count = summary.data_rows_total.min(u64::from(u32::MAX)) as u32;
    payload[0..8].copy_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload[16..20].copy_from_slice(&row_count.to_le_bytes());
    payload[20..24].copy_from_slice(&row_count.to_le_bytes());
    payload[28..32].copy_from_slice(&row_count.to_le_bytes());
    payload[32..36].copy_from_slice(&summary.status_flags_or.to_le_bytes());
    payload
}

fn append_compact_record(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    payload: &[u8],
) {
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain_id.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    out.extend_from_slice(&(payload.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(payload);
}

fn gate_stream(raw: &[u8]) -> Result<StreamGate, String> {
    let mut gate = StreamGate {
        total_bytes: raw.len(),
        ..StreamGate::default()
    };
    let mut offset = 0usize;
    let mut previous_sequence: Option<u64> = None;
    let mut previous_closed_hour: Option<i32> = None;
    let mut open_frame_hour: Option<i32> = None;
    let mut frame_data_records = 0u32;
    let mut frame_perf_records = 0u32;
    let mut frame_health_ok = true;
    let mut final_record_is_clean_frame_end = false;
    while offset < raw.len() {
        final_record_is_clean_frame_end = false;
        let header = match parse_compact_record_header(&raw[offset..]) {
            Ok(header) => header,
            Err(RecordFramingError::TruncatedHeader(_)) => {
                gate.problems
                    .push(format!("offset {offset}: truncated compact record header"));
                break;
            }
            Err(RecordFramingError::BadMagic) => {
                gate.problems
                    .push(format!("offset {offset}: invalid compact record magic"));
                break;
            }
            Err(error) => {
                gate.problems.push(format!(
                    "offset {offset}: invalid compact record header: {error}"
                ));
                break;
            }
        };
        let sequence = header.sequence;
        let game_hour_raw = header.game_hour_raw;
        let domain_id = header.domain_id;
        let record_type = header.record_type;
        let payload_len = header.payload_len;
        match previous_sequence {
            None if sequence != 1 => gate.problems.push(format!(
                "offset {offset}: sequence {sequence} does not start at 1 (stream prefix is missing)"
            )),
            Some(previous) => match previous.checked_add(1) {
                Some(expected) if sequence != expected => gate.problems.push(format!(
                    "offset {offset}: sequence discontinuity {previous} -> {sequence} (expected {expected})"
                )),
                None => gate.problems.push(format!(
                    "offset {offset}: sequence space exhausted after {previous}; got {sequence} (wrapping is not allowed)"
                )),
                _ => {}
            },
            _ => {}
        }
        previous_sequence = Some(sequence);
        let record_len = header.record_len();
        if record_len > raw.len().saturating_sub(offset) {
            gate.problems.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining stream bytes"
            ));
            break;
        }
        let payload_start = offset + COMPACT_RECORD_HEADER_LEN;
        let payload_end = payload_start + payload_len;
        let payload = &raw[payload_start..payload_end];
        if let Err(error) =
            validate_compact_record_semantics(domain_id, record_type, header.record_flags, payload)
        {
            gate.problems.push(format!(
                "offset {offset}: invalid compact record envelope: {error}"
            ));
            offset += record_len;
            continue;
        }
        gate.record_count = gate.record_count.saturating_add(1);
        match record_type {
            RECORD_TYPE_DOMAIN_DATA if domain_id == domain44::DOMAIN_ID => {
                observe_open_frame(
                    &mut gate,
                    &mut open_frame_hour,
                    previous_closed_hour,
                    game_hour_raw,
                    "DOMAIN_DATA",
                    offset,
                );
                gate.domain_data_records = gate.domain_data_records.saturating_add(1);
                frame_data_records = frame_data_records.saturating_add(1);
                let summary = domain44::decode_payload(offset, payload);
                if !summary.parse_errors.is_empty() {
                    gate.problems.extend(summary.parse_errors.clone());
                    frame_health_ok = false;
                }
                frame_health_ok &=
                    observe_status(&mut gate, "DOMAIN_DATA", summary.status_flags_or, offset);
                gate.province_rows_total = gate
                    .province_rows_total
                    .saturating_add(summary.province_rows_total);
                gate.division_rows_total = gate
                    .division_rows_total
                    .saturating_add(summary.division_rows_total);
                gate.badge_rows_total = gate
                    .badge_rows_total
                    .saturating_add(summary.badge_rows_total);
                gate.csupply_input_rows_total = gate
                    .csupply_input_rows_total
                    .saturating_add(summary.csupply_input_rows_total);
                gate.pressure_candidate_rows_total = gate
                    .pressure_candidate_rows_total
                    .saturating_add(summary.pressure_candidate_rows_total);
                gate.data_rows_total = gate.data_rows_total.saturating_add(summary.data_rows_total);
            }
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF
                if domain_id == domain44::DOMAIN_ID =>
            {
                observe_open_frame(
                    &mut gate,
                    &mut open_frame_hour,
                    previous_closed_hour,
                    game_hour_raw,
                    "DOMAIN_PERF",
                    offset,
                );
                gate.domain_perf_records = gate.domain_perf_records.saturating_add(1);
                frame_perf_records = frame_perf_records.saturating_add(1);
                frame_health_ok &=
                    observe_status(&mut gate, "DOMAIN_PERF", read_u32_le(payload, 32), offset);
            }
            RECORD_TYPE_FRAME_END if domain_id == compact_contract::COMPACT_DOMAIN_FRAME => {
                gate.frame_end_records = gate.frame_end_records.saturating_add(1);
                let frame_end_health_ok =
                    observe_status(&mut gate, "FRAME_END", read_u32_le(payload, 24), offset);
                let mut clean_boundary = frame_health_ok && frame_end_health_ok;
                let emitted_domains = read_u32_le(payload, 16);
                let scheduled_domains = read_u32_le(payload, 20);
                let reserved = read_u32_le(payload, 28);
                if emitted_domains != 1 || scheduled_domains != 1 {
                    gate.problems.push(format!(
                        "offset {offset}: lane-local FRAME_END emitted/scheduled domain counts {emitted_domains}/{scheduled_domains}, expected 1/1"
                    ));
                    clean_boundary = false;
                }
                if reserved != 0 {
                    gate.problems.push(format!(
                        "offset {offset}: FRAME_END reserved word is nonzero ({reserved:#010x})"
                    ));
                    clean_boundary = false;
                }
                if frame_perf_records != 1 {
                    gate.problems.push(format!(
                        "offset {offset}: closed frame hour {game_hour_raw} has {frame_perf_records} domain44 PERF records, expected exactly 1"
                    ));
                    clean_boundary = false;
                }
                if frame_data_records != 1 {
                    gate.problems.push(format!(
                        "offset {offset}: closed frame hour {game_hour_raw} has {frame_data_records} domain44 DATA records, expected exactly 1"
                    ));
                    clean_boundary = false;
                }
                match open_frame_hour {
                    Some(open) if open == game_hour_raw => {}
                    Some(open) => {
                        gate.problems.push(format!(
                            "offset {offset}: FRAME_END hour {game_hour_raw} does not close open frame hour {open}"
                        ));
                        clean_boundary = false;
                    }
                    None => {
                        gate.problems.push(format!(
                            "offset {offset}: FRAME_END hour {game_hour_raw} has no open domain44 frame"
                        ));
                        clean_boundary = false;
                    }
                }
                if let Some(previous) = previous_closed_hour {
                    match previous.checked_add(1) {
                        Some(expected) if game_hour_raw == expected => {}
                        Some(expected) => {
                            if game_hour_raw > expected {
                                gate.skipped_game_hours = gate.skipped_game_hours.saturating_add(
                                    (i64::from(game_hour_raw) - i64::from(expected)) as u64,
                                );
                            }
                            gate.problems.push(format!(
                                "offset {offset}: closed frame hour {game_hour_raw} is not consecutive after {previous} (expected {expected})"
                            ));
                            clean_boundary = false;
                        }
                        None => {
                            gate.problems.push(format!(
                                "offset {offset}: closed frame hour overflow after {previous}"
                            ));
                            clean_boundary = false;
                        }
                    }
                }
                if clean_boundary {
                    gate.first_closed_hour.get_or_insert(game_hour_raw);
                    gate.last_closed_hour = Some(game_hour_raw);
                    gate.unique_game_hours = gate.unique_game_hours.saturating_add(1);
                }
                open_frame_hour = None;
                previous_closed_hour = Some(game_hour_raw);
                frame_data_records = 0;
                frame_perf_records = 0;
                frame_health_ok = true;
                final_record_is_clean_frame_end = clean_boundary;
            }
            other => gate.problems.push(format!(
                "offset {offset}: unexpected compact record type {other} domain {domain_id}"
            )),
        }
        offset += record_len;
    }
    if let Some(open) = open_frame_hour {
        gate.problems
            .push(format!("stream ended with open domain44 frame hour {open}"));
    }
    if !final_record_is_clean_frame_end {
        gate.problems
            .push("stream is not closed by a final canonical FRAME_END record".to_string());
    }
    if gate.domain_data_records == 0 {
        gate.problems.push("no domain44 DATA records".to_string());
    }
    if gate.domain_perf_records == 0 {
        gate.problems.push("no domain44 PERF records".to_string());
    }
    if gate.frame_end_records == 0 {
        gate.problems.push("no FRAME_END records".to_string());
    }
    if gate.csupply_input_rows_total == 0 && gate.pressure_candidate_rows_total == 0 {
        gate.problems.push(
            "csupply_input_rows_total and pressure_candidate_rows_total are both zero".to_string(),
        );
    }
    gate.observed_game_hour_advancements = gate
        .first_closed_hour
        .zip(gate.last_closed_hour)
        .map(|(first, last)| i64::from(last) - i64::from(first));
    gate.ok = gate.problems.is_empty();
    Ok(gate)
}

fn observe_open_frame(
    gate: &mut StreamGate,
    open_frame_hour: &mut Option<i32>,
    previous_closed_hour: Option<i32>,
    game_hour_raw: i32,
    record_label: &str,
    offset: usize,
) {
    match *open_frame_hour {
        Some(open) if open != game_hour_raw => gate.problems.push(format!(
            "offset {offset}: {record_label} hour {game_hour_raw} encountered before FRAME_END closed open frame hour {open}"
        )),
        Some(_) => {}
        None => {
            if let Some(previous) = previous_closed_hour {
                match previous.checked_add(1) {
                    Some(expected) if game_hour_raw == expected => {}
                    Some(expected) => gate.problems.push(format!(
                        "offset {offset}: {record_label} hour {game_hour_raw} follows closed frame hour {previous}; expected {expected}"
                    )),
                    None => gate.problems.push(format!(
                        "offset {offset}: closed frame hour {previous} exhausted; another frame is forbidden"
                    )),
                }
            }
            *open_frame_hour = Some(game_hour_raw);
        }
    }
}

fn observe_status(gate: &mut StreamGate, origin: &str, status_flags: u32, offset: usize) -> bool {
    gate.status_flags_or |= status_flags;
    let unknown = compact_contract::compact_status_unknown_flags_v1(status_flags);
    gate.unknown_status_flags_or |= unknown;
    if unknown != 0 {
        gate.problems.push(format!(
            "offset {offset}: {origin} status flags {status_flags:#010x} contain unknown bits {unknown:#010x}"
        ));
    }
    let failure = status_flags & compact_contract::COMPACT_STATUS_FAILURE_MASK;
    gate.failure_status_flags_or |= failure;
    if failure != 0 {
        gate.problems.push(format!(
            "offset {offset}: {origin} status flags {status_flags:#010x} contain canonical failure bits {failure:#010x}"
        ));
    }
    unknown == 0 && failure == 0
}

fn manifest_json(
    options: &Options,
    stream: &[u8],
    gate: &StreamGate,
    payload_summary: &domain44::Domain44GateSummary,
) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(&json!({
        "schema": "native_collector_session_v1",
        "session_id": options.game_id,
        "replay_game_id": options.game_id,
        "stream_format": "compact",
        "stream_file": "collector_stream.bin",
        "stream_role": "collector_compact_stream",
        "scheduled_domains": [domain44::DOMAIN_NAME],
        "source": "lane-local supply_logistics_attrition_live compact stream replay pack proof",
        "hourly_requirement": {
            "required": false,
            "requested_game_hours": null,
            "met": false,
            "reason": "lane-local single-frame fixture has zero observed game-hour advancements and cannot certify production duration",
            "problems": gate.problems,
        },
        "certification": {
            "claimed": false,
            "minimum_game_hour_advancements": null,
            "observed_game_hour_advancements": gate.observed_game_hour_advancements,
            "observed_closed_frames": gate.unique_game_hours,
            "reason": "lane-local replay packing fixture only; no live production-duration certification is claimed",
        },
        "unique_game_hour_count": gate.unique_game_hours,
            "skipped_game_hours": gate.skipped_game_hours,
            "first_closed_hour": gate.first_closed_hour,
            "last_closed_hour": gate.last_closed_hour,
            "observed_game_hour_advancements": gate.observed_game_hour_advancements,
        "parse_errors": gate.problems,
        "compact_stream": {
            "record_magic": "H4CR",
            "record_version": 1,
            "record_header_len": 32,
            "decodable": gate.ok,
            "ordered": true,
            "total_bytes": stream.len(),
            "record_count": gate.record_count,
            "domain_perf_record_count": gate.domain_perf_records,
            "domain_data_record_count": gate.domain_data_records,
            "frame_end_record_count": gate.frame_end_records,
            "status_flags_or": gate.status_flags_or,
            "failure_status_flags_or": gate.failure_status_flags_or,
            "failure_status_seen": gate.failure_status_flags_or != 0,
            "unknown_status_flags_or": gate.unknown_status_flags_or,
            "unknown_status_seen": gate.unknown_status_flags_or != 0,
            "worker_panic_seen": gate.status_flags_or & compact_contract::COMPACT_STATUS_WORKER_PANIC != 0,
            "bridge_unavailable_seen": gate.status_flags_or & compact_contract::COMPACT_STATUS_BRIDGE_UNAVAILABLE != 0,
            "source_unavailable_seen": gate.status_flags_or & compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE != 0,
            "partial_data_seen": gate.status_flags_or & compact_contract::COMPACT_STATUS_PARTIAL_DATA != 0,
            "control_invalid_seen": gate.status_flags_or & compact_contract::COMPACT_STATUS_CONTROL_INVALID != 0,
            "domains_seen": [domain44::DOMAIN_NAME],
            "expected_domains": [domain44::DOMAIN_NAME],
            "per_domain": {
                domain44::DOMAIN_NAME: {
                    "domain_id": domain44::DOMAIN_ID,
                    "data_records": gate.domain_data_records,
                    "data_rows_total": payload_summary.data_rows_total,
                    "province_rows_total": payload_summary.province_rows_total,
                    "division_rows_total": payload_summary.division_rows_total,
                    "badge_rows_total": payload_summary.badge_rows_total,
                    "csupply_input_rows_total": payload_summary.csupply_input_rows_total,
                    "observed_province_roots": payload_summary.observed_province_roots,
                    "observed_division_roots": payload_summary.observed_division_roots,
                }
            }
        },
    }))
    .map_err(|err| err.to_string())
}

fn pack_replay(game_id: &str, stream: &[u8], manifest: &[u8]) -> Result<Vec<u8>, String> {
    let index_bytes = serde_json::to_vec_pretty(&json!({
        "game_id": game_id,
        "schema_version": REPLAY_SCHEMA_VERSION,
        "snapshots": [],
    }))
    .map_err(|err| err.to_string())?;
    let timeline_bytes = serde_json::to_vec_pretty(&json!({
        "format": "hoi4_compact_stream_timeline_v1",
        "stream_format": "compact",
        "stream": STREAM_ENTRY_PATH,
        "session_manifest": MANIFEST_ENTRY_PATH,
        "raw_stream_role": "collector_compact_stream",
        "session_manifest_role": "recorder_manifest",
        "domain_id": domain44::DOMAIN_ID,
        "lane": domain44::DOMAIN_NAME,
    }))
    .map_err(|err| err.to_string())?;
    let entries = vec![
        PackEntry {
            path: format!("payload/{game_id}/index.json"),
            bytes: index_bytes,
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/timeline.json"),
            bytes: timeline_bytes,
            role: "timeline",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/{STREAM_ENTRY_PATH}"),
            bytes: stream.to_vec(),
            role: "collector_compact_stream",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/{MANIFEST_ENTRY_PATH}"),
            bytes: manifest.to_vec(),
            role: "recorder_manifest",
            hour_index: None,
        },
    ];
    let identity = PackIdentity {
        schema_version: Some(REPLAY_SCHEMA_VERSION.to_string()),
        build_exe_sha256: Some(BUILD_SHA256.to_string()),
        build_pinned: true,
        created_utc: Some(created_label()),
        ..Default::default()
    };
    pack(game_id, &entries, &identity, true).map_err(|err| err.to_string())
}

fn verify_replay(
    game_id: &str,
    replay: &[u8],
    expected_stream: &[u8],
) -> Result<Verification, String> {
    let expected = ExpectedIdentity {
        build_exe_sha256: Some(BUILD_SHA256.to_string()),
        schema_version: Some(REPLAY_SCHEMA_VERSION.to_string()),
        ..Default::default()
    };
    let container =
        Container::from_bytes(replay.to_vec(), &expected).map_err(|err| err.to_string())?;
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
    let expected_path = format!("payload/{game_id}/{STREAM_ENTRY_PATH}");
    if stream_entries[0].path != expected_path {
        return Err(format!(
            "collector_compact_stream path {} != {}",
            stream_entries[0].path, expected_path
        ));
    }
    let stored_stream = container
        .read_entry_bytes(&stream_entries[0].path)
        .map_err(|err| err.to_string())?;
    if stored_stream != expected_stream {
        return Err("collector_compact_stream bytes differ after replay pack".to_string());
    }
    let replay_stream_gate = gate_stream(&stored_stream)?;
    if !replay_stream_gate.ok {
        return Err(format!(
            "replay readback stream gate failed: {:?}",
            replay_stream_gate.problems
        ));
    }
    Ok(Verification {
        stream_entry: stream_entries[0].path.clone(),
        stream_bytes: stored_stream.len(),
        collector_stream_entries: stream_entries.len(),
        recorder_manifest_entries: manifest_entries.len(),
    })
}

fn payload_summary_json(summary: &domain44::Domain44GateSummary) -> serde_json::Value {
    json!({
        "data_rows_total": summary.data_rows_total,
        "province_rows_total": summary.province_rows_total,
        "division_rows_total": summary.division_rows_total,
        "badge_rows_total": summary.badge_rows_total,
        "csupply_input_rows_total": summary.csupply_input_rows_total,
        "pressure_candidate_rows_total": summary.pressure_candidate_rows_total,
        "observed_province_roots": summary.observed_province_roots,
        "observed_division_roots": summary.observed_division_roots,
        "status_flags_or": summary.status_flags_or,
        "data_units_total": summary.data_units_total,
        "parse_errors": summary.parse_errors,
    })
}

fn proof_markdown(proof: &serde_json::Value) -> String {
    let gate = &proof["stream_gate"];
    let payload = &proof["payload_gate"];
    format!(
        "# supply_logistics_attrition_live replay pack proof\n\n\
         - result: `{}`\n\
         - domain_id: `{}`\n\
         - hoi4replay_path: `{}`\n\
         - hoi4replay_bytes: `{}`\n\
         - collector_stream_bytes: `{}`\n\
         - domain_data_records: `{}`\n\
         - frame_end_records: `{}`\n\
         - unique_game_hours: `{}`\n\
         - skipped_game_hours: `{}`\n\
         - data_rows_total: `{}`\n\
         - csupply_input_rows_total: `{}`\n\
         - pressure_candidate_rows_total: `{}`\n\
         - parse_errors: `{}`\n\n\
         ## Verdict\n\n\
         - The lane-local compact stream was packed into `.battleplan`, reopened with `snapshot::Container`, byte-verified, and domain 44 decoded from the embedded stream.\n\
         - This proves replay packing for lane-local domain 44 compact rows. Candidate pressure rows are not a production capture and do not remove the mandatory convoy-pressure badge.\n",
        proof["result"].as_str().unwrap_or("unknown"),
        proof["domain_id"].as_u64().unwrap_or(0),
        proof["hoi4replay_path"].as_str().unwrap_or("none"),
        proof["hoi4replay_bytes"].as_u64().unwrap_or(0),
        proof["collector_stream_bytes"].as_u64().unwrap_or(0),
        gate["domain_data_records"].as_u64().unwrap_or(0),
        gate["frame_end_records"].as_u64().unwrap_or(0),
        gate["unique_game_hours"].as_u64().unwrap_or(0),
        gate["skipped_game_hours"].as_u64().unwrap_or(0),
        payload["data_rows_total"].as_u64().unwrap_or(0),
        payload["csupply_input_rows_total"].as_u64().unwrap_or(0),
        payload["pressure_candidate_rows_total"].as_u64().unwrap_or(0),
        payload["parse_errors"].as_array().map_or(0, Vec::len),
    )
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("hoi4replay.tmp");
    fs::write(&tmp, bytes).map_err(|err| format!("write replay tmp failed: {err}"))?;
    if path.exists() {
        fs::remove_file(path).map_err(|err| format!("replace replay failed: {err}"))?;
    }
    fs::rename(&tmp, path).map_err(|err| format!("finalize replay failed: {err}"))
}

fn created_label() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("unix_seconds:{secs}")
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("fixed u32 slice"),
    )
}

fn read_u64_le(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("fixed u64 slice"),
    )
}

#[derive(Default)]
struct StreamGate {
    ok: bool,
    total_bytes: usize,
    record_count: u64,
    domain_perf_records: u64,
    domain_data_records: u64,
    frame_end_records: u64,
    unique_game_hours: u64,
    skipped_game_hours: u64,
    first_closed_hour: Option<i32>,
    last_closed_hour: Option<i32>,
    observed_game_hour_advancements: Option<i64>,
    data_rows_total: u64,
    province_rows_total: u64,
    division_rows_total: u64,
    badge_rows_total: u64,
    csupply_input_rows_total: u64,
    pressure_candidate_rows_total: u64,
    status_flags_or: u32,
    failure_status_flags_or: u32,
    unknown_status_flags_or: u32,
    problems: Vec<String>,
}

impl StreamGate {
    fn to_json(&self) -> serde_json::Value {
        json!({
            "ok": self.ok,
            "total_bytes": self.total_bytes,
            "record_count": self.record_count,
            "domain_perf_records": self.domain_perf_records,
            "domain_data_records": self.domain_data_records,
            "frame_end_records": self.frame_end_records,
            "unique_game_hours": self.unique_game_hours,
            "skipped_game_hours": self.skipped_game_hours,
            "first_closed_hour": self.first_closed_hour,
            "last_closed_hour": self.last_closed_hour,
            "observed_game_hour_advancements": self.observed_game_hour_advancements,
            "data_rows_total": self.data_rows_total,
            "province_rows_total": self.province_rows_total,
            "division_rows_total": self.division_rows_total,
            "badge_rows_total": self.badge_rows_total,
            "csupply_input_rows_total": self.csupply_input_rows_total,
            "pressure_candidate_rows_total": self.pressure_candidate_rows_total,
            "status_flags_or": self.status_flags_or,
            "failure_status_flags_or": self.failure_status_flags_or,
            "unknown_status_flags_or": self.unknown_status_flags_or,
            "worker_panic_seen": self.status_flags_or & compact_contract::COMPACT_STATUS_WORKER_PANIC != 0,
            "bridge_unavailable_seen": self.status_flags_or & compact_contract::COMPACT_STATUS_BRIDGE_UNAVAILABLE != 0,
            "source_unavailable_seen": self.status_flags_or & compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE != 0,
            "partial_data_seen": self.status_flags_or & compact_contract::COMPACT_STATUS_PARTIAL_DATA != 0,
            "control_invalid_seen": self.status_flags_or & compact_contract::COMPACT_STATUS_CONTROL_INVALID != 0,
            "problems": self.problems,
        })
    }
}

struct Verification {
    stream_entry: String,
    stream_bytes: usize,
    collector_stream_entries: usize,
    recorder_manifest_entries: usize,
}

struct Options {
    payload: PathBuf,
    out_dir: PathBuf,
    game_id: String,
}

impl Options {
    fn parse() -> Result<Self, String> {
        let mut payload = None;
        let mut out_dir = None;
        let mut game_id = DEFAULT_GAME_ID.to_string();
        let args = env::args().skip(1).collect::<Vec<_>>();
        let mut i = 0usize;
        while i < args.len() {
            match args[i].as_str() {
                "--payload" => {
                    i += 1;
                    payload = Some(PathBuf::from(required(args.get(i), "--payload")?));
                }
                "--out-dir" => {
                    i += 1;
                    out_dir = Some(PathBuf::from(required(args.get(i), "--out-dir")?));
                }
                "--game-id" => {
                    i += 1;
                    game_id = required(args.get(i), "--game-id")?;
                    validate_game_id(&game_id)?;
                }
                "--help" | "-h" => return Err(usage()),
                other => return Err(format!("unknown argument {other}\n{}", usage())),
            }
            i += 1;
        }
        Ok(Self {
            payload: payload.ok_or_else(usage)?,
            out_dir: out_dir.ok_or_else(usage)?,
            game_id,
        })
    }
}

fn validate_game_id(game_id: &str) -> Result<(), String> {
    if game_id.is_empty()
        || !game_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err("game id must be non-empty ASCII alnum, '_' or '-'".to_string());
    }
    Ok(())
}

fn required(value: Option<&String>, flag: &str) -> Result<String, String> {
    value
        .cloned()
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn usage() -> String {
    "usage: supply_logistics_attrition_replay_pack_proof --payload <domain44_payload.bin> --out-dir <proof-dir> [--game-id <id>]".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_payload(status_flags: u32) -> Vec<u8> {
        let mut rows = vec![domain44::SupplyLogisticsRow {
            row_kind: domain44::ROW_KIND_CSUPPLY_PROVINCE_INPUT,
            entity_id: 3324,
            join_id: 0,
            owner_tag_db_id: 0,
            flags: domain44::ROW_FLAG_CSUPPLY_EFFICIENCY
                | domain44::ROW_FLAG_CSUPPLY_PROVINCE_ARRAY,
            value_a_raw: 95_000,
            value_b_raw: 0,
            value_c_raw: 0,
            value_d_raw: 0,
            aux_raw: 0,
        }];
        for (badge, flag) in [
            (
                domain44::BADGE_EXACT_PROVINCE_SUPPLY_TRANSIENT,
                domain44::ROW_FLAG_EXACT_SUPPLY_WALL,
            ),
            (
                domain44::BADGE_ATTRITION_RUNTIME_COMPUTED,
                domain44::ROW_FLAG_ATTRITION_WALL,
            ),
            (
                domain44::BADGE_MOTORIZATION_SETTINGS_UNPINNED,
                domain44::ROW_FLAG_MOTORIZATION_WALL,
            ),
            (
                domain44::BADGE_CONVOY_LOGISTICS_PRESSURE_UNPINNED,
                domain44::ROW_FLAG_CONVOY_PRESSURE_WALL,
            ),
            (
                domain44::BADGE_CSUPPLY_INPUT_ARRAYS_UNSAFE,
                domain44::ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED,
            ),
        ] {
            rows.push(domain44::SupplyLogisticsRow {
                row_kind: domain44::ROW_KIND_COMPUTED_BADGE,
                entity_id: 0,
                join_id: 0,
                owner_tag_db_id: 0,
                flags: flag,
                value_a_raw: badge,
                value_b_raw: 0,
                value_c_raw: 0,
                value_d_raw: 0,
                aux_raw: 0,
            });
        }
        domain44::encode_payload(
            &domain44::SupplyLogisticsHeader {
                elapsed_wall_ms: 1,
                status_flags,
                changed_rows: rows.len() as u32,
                province_rows: 0,
                division_rows: 0,
                badge_rows: 5,
                observed_province_roots: 0,
                observed_division_roots: 0,
                csupply_input_rows: 1,
                pressure_candidate_rows: 0,
            },
            &rows,
        )
    }

    fn perf_payload_with_status(
        status_flags: u32,
    ) -> [u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN] {
        let mut payload = [0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN];
        payload[32..36].copy_from_slice(&status_flags.to_le_bytes());
        payload
    }

    fn frame_end_payload_with_status(
        status_flags: u32,
    ) -> [u8; compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN] {
        build_frame_end_payload(status_flags)
    }

    fn canonical_stream(data_status: u32, perf_status: u32, frame_status: u32) -> Vec<u8> {
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            100,
            domain44::DOMAIN_ID,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(perf_status),
        );
        append_compact_record(
            &mut stream,
            2,
            100,
            domain44::DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &valid_payload(data_status),
        );
        append_compact_record(
            &mut stream,
            3,
            100,
            compact_contract::COMPACT_DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_end_payload_with_status(frame_status),
        );
        stream
    }

    fn append_canonical_frame(
        stream: &mut Vec<u8>,
        first_sequence: u64,
        game_hour_raw: i32,
        data_status: u32,
        perf_status: u32,
        frame_status: u32,
    ) {
        append_compact_record(
            stream,
            first_sequence,
            game_hour_raw,
            domain44::DOMAIN_ID,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(perf_status),
        );
        append_compact_record(
            stream,
            first_sequence + 1,
            game_hour_raw,
            domain44::DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &valid_payload(data_status),
        );
        append_compact_record(
            stream,
            first_sequence + 2,
            game_hour_raw,
            compact_contract::COMPACT_DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_end_payload_with_status(frame_status),
        );
    }

    fn sequence_gate(sequences: &[u64]) -> StreamGate {
        let mut stream = Vec::new();
        for &sequence in sequences {
            append_compact_record(
                &mut stream,
                sequence,
                100,
                compact_contract::COMPACT_DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &[0u8; compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN],
            );
        }
        gate_stream(&stream).expect("stream framing should decode")
    }

    fn one_record_gate(
        domain_id: u16,
        record_type: u16,
        record_flags: u32,
        payload: &[u8],
    ) -> StreamGate {
        let mut stream = Vec::new();
        append_compact_record(&mut stream, 1, 100, domain_id, record_type, payload);
        stream[28..32].copy_from_slice(&record_flags.to_le_bytes());
        gate_stream(&stream).expect("stream framing should decode")
    }

    fn perf_payload_with_empty_pdu1() -> Vec<u8> {
        let mut payload = vec![0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN];
        payload.extend_from_slice(compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC);
        payload.extend_from_slice(
            &compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION.to_le_bytes(),
        );
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload
    }

    #[test]
    fn record_flag_policy_accepts_canonical_schema_and_perf_flags() {
        let schema = one_record_gate(
            45,
            RECORD_TYPE_DOMAIN_DATA,
            compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &[],
        );
        let perf_payload = perf_payload_with_empty_pdu1();
        let perf = one_record_gate(
            domain44::DOMAIN_ID,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &perf_payload,
        );

        for gate in [&schema, &perf] {
            assert!(gate
                .problems
                .iter()
                .all(|problem| !problem.contains("invalid compact record envelope")));
        }
    }

    #[test]
    fn record_flag_policy_rejects_wrong_placement_and_unknown_bits() {
        let wrong_placement = one_record_gate(
            domain44::DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &[],
        );
        let unknown = one_record_gate(
            compact_contract::COMPACT_DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            1 << 29,
            &[0u8; compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN],
        );

        assert!(wrong_placement
            .problems
            .iter()
            .any(|problem| problem.contains("rejected bits 0x80000000")));
        assert!(unknown
            .problems
            .iter()
            .any(|problem| problem.contains("rejected bits 0x20000000")));
    }

    #[test]
    fn perf_envelope_accepts_announced_pdu1_trailer() {
        let payload = perf_payload_with_empty_pdu1();
        let gate = one_record_gate(
            domain44::DOMAIN_ID,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &payload,
        );

        assert!(gate
            .problems
            .iter()
            .all(|problem| !problem.contains("invalid compact record envelope")));
    }

    #[test]
    fn perf_envelope_rejects_missing_announced_trailer() {
        let payload = vec![0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN];
        let gate = one_record_gate(
            domain44::DOMAIN_ID,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &payload,
        );

        assert!(gate
            .problems
            .iter()
            .any(|problem| problem.contains("diagnostic trailer header is truncated")));
    }

    #[test]
    fn perf_envelope_rejects_unannounced_trailer() {
        let payload = perf_payload_with_empty_pdu1();
        let gate = one_record_gate(
            domain44::DOMAIN_ID,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &payload,
        );

        assert!(gate
            .problems
            .iter()
            .any(|problem| problem.contains("unannounced trailer byte(s)")));
    }

    #[test]
    fn perf_envelope_rejects_malformed_announced_trailer() {
        let mut payload = perf_payload_with_empty_pdu1();
        payload[compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN] = b'X';
        let gate = one_record_gate(
            domain44::DOMAIN_ID,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &payload,
        );

        assert!(gate
            .problems
            .iter()
            .any(|problem| problem.contains("diagnostic trailer magic is invalid")));
    }

    #[test]
    fn stream_gate_rejects_missing_sequence_prefix() {
        let gate = sequence_gate(&[2]);

        assert!(gate
            .problems
            .iter()
            .any(|problem| problem.contains("sequence 2 does not start at 1")));
    }

    #[test]
    fn stream_gate_rejects_sequence_gap() {
        let gate = sequence_gate(&[1, 3]);

        assert!(gate
            .problems
            .iter()
            .any(|problem| problem.contains("sequence discontinuity 1 -> 3 (expected 2)")));
    }

    #[test]
    fn stream_gate_rejects_repeated_sequence() {
        let gate = sequence_gate(&[1, 1]);

        assert!(gate
            .problems
            .iter()
            .any(|problem| problem.contains("sequence discontinuity 1 -> 1 (expected 2)")));
    }

    #[test]
    fn stream_gate_rejects_sequence_regression() {
        let gate = sequence_gate(&[1, 2, 1]);

        assert!(gate
            .problems
            .iter()
            .any(|problem| problem.contains("sequence discontinuity 2 -> 1 (expected 3)")));
    }

    #[test]
    fn stream_gate_rejects_sequence_wrap_after_max() {
        let gate = sequence_gate(&[u64::MAX, 0]);

        assert!(gate.problems.iter().any(|problem| {
            problem.contains("sequence space exhausted after 18446744073709551615")
                && problem.contains("wrapping is not allowed")
        }));
    }

    #[test]
    fn canonical_failure_bits_are_rejected_from_all_three_status_origins() {
        let fatal_bits = [
            compact_contract::COMPACT_STATUS_STATE_UNAVAILABLE,
            compact_contract::COMPACT_STATUS_PROFILE_MISMATCH,
            compact_contract::COMPACT_STATUS_NO_CLOCK,
            compact_contract::COMPACT_STATUS_NO_SCALAR_SOURCE,
            compact_contract::COMPACT_STATUS_WORKER_PANIC,
            compact_contract::COMPACT_STATUS_BRIDGE_UNAVAILABLE,
            compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
            compact_contract::COMPACT_STATUS_PARTIAL_DATA,
            compact_contract::COMPACT_STATUS_CONTROL_INVALID,
        ];
        assert_eq!(
            fatal_bits
                .iter()
                .copied()
                .fold(0u32, |mask, bit| mask | bit),
            compact_contract::COMPACT_STATUS_FAILURE_MASK
        );
        for bit in fatal_bits {
            for statuses in [(bit, 0, 0), (0, bit, 0), (0, 0, bit)] {
                let gate = gate_stream(&canonical_stream(statuses.0, statuses.1, statuses.2))
                    .expect("canonical framing must decode");
                assert!(!gate.ok, "bit={bit:#x}, problems={:?}", gate.problems);
                assert_ne!(gate.failure_status_flags_or & bit, 0);
                assert!(gate.problems.iter().any(|problem| {
                    problem.contains("canonical failure bits")
                        && problem.contains(&format!("{bit:#010x}"))
                }));
            }
        }
    }

    #[test]
    fn unknown_status_bits_are_rejected_from_all_three_status_origins() {
        let unknown = 1u32 << 31;
        assert_eq!(
            compact_contract::compact_status_unknown_flags_v1(unknown),
            unknown
        );
        for statuses in [(unknown, 0, 0), (0, unknown, 0), (0, 0, unknown)] {
            let gate = gate_stream(&canonical_stream(statuses.0, statuses.1, statuses.2))
                .expect("canonical framing must decode");
            assert!(!gate.ok, "problems={:?}", gate.problems);
            assert_eq!(gate.unknown_status_flags_or, unknown);
            assert!(gate
                .problems
                .iter()
                .any(|problem| problem.contains("contain unknown bits 0x80000000")));
        }
    }

    #[test]
    fn informational_status_is_accepted_and_health_is_evidence_derived() {
        let informational = compact_contract::COMPACT_STATUS_WAR_BRIDGE_APPLIED
            | compact_contract::COMPACT_STATUS_EXTRACTION_PERF_GATE
            | compact_contract::COMPACT_STATUS_DELTA_DATA
            | compact_contract::COMPACT_STATUS_TRANSITION_BASELINE_RESET
            | compact_contract::COMPACT_STATUS_WAR_SEGMENT_ABSENT;
        let gate = gate_stream(&canonical_stream(informational, 0, 0))
            .expect("canonical framing must decode");
        assert!(gate.ok, "{:?}", gate.problems);
        assert_eq!(gate.failure_status_flags_or, 0);
        assert_eq!(gate.unknown_status_flags_or, 0);
        assert_eq!(gate.status_flags_or, informational);

        let failed = gate_stream(&canonical_stream(
            compact_contract::COMPACT_STATUS_WORKER_PANIC
                | compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
            compact_contract::COMPACT_STATUS_BRIDGE_UNAVAILABLE,
            compact_contract::COMPACT_STATUS_PARTIAL_DATA
                | compact_contract::COMPACT_STATUS_CONTROL_INVALID,
        ))
        .expect("canonical framing must decode");
        let json = failed.to_json();
        assert_eq!(json["worker_panic_seen"], json!(true));
        assert_eq!(json["bridge_unavailable_seen"], json!(true));
        assert_eq!(json["source_unavailable_seen"], json!(true));
        assert_eq!(json["partial_data_seen"], json!(true));
        assert_eq!(json["control_invalid_seen"], json!(true));
    }

    #[test]
    fn built_fixture_stream_has_canonical_perf_counters_and_zero_duration() {
        let payload = valid_payload(0);
        let summary = domain44::decode_payload(0, &payload);
        let stream = build_stream(&payload, &summary);
        let perf_payload = COMPACT_RECORD_HEADER_LEN;
        let row_count = summary.data_rows_total as u32;

        assert_eq!(read_u64_le(&stream, perf_payload), 1);
        assert_eq!(read_u32_le(&stream, perf_payload + 16), row_count);
        assert_eq!(read_u32_le(&stream, perf_payload + 20), row_count);
        assert_eq!(read_u32_le(&stream, perf_payload + 24), 0);
        assert_eq!(read_u32_le(&stream, perf_payload + 28), row_count);
        assert_eq!(read_u32_le(&stream, perf_payload + 32), 0);
        assert_eq!(read_u32_le(&stream, perf_payload + 36), 0);

        let gate = gate_stream(&stream).expect("fixture stream should decode");
        assert!(gate.ok, "{:?}", gate.problems);
        assert_eq!(gate.unique_game_hours, 1);
        assert_eq!(gate.first_closed_hour, Some(1));
        assert_eq!(gate.last_closed_hour, Some(1));
        assert_eq!(gate.observed_game_hour_advancements, Some(0));
    }

    #[test]
    fn manifest_explicitly_declines_production_duration_certification() {
        let payload = valid_payload(0);
        let summary = domain44::decode_payload(0, &payload);
        let stream = build_stream(&payload, &summary);
        let gate = gate_stream(&stream).expect("fixture stream should decode");
        let options = Options {
            payload: PathBuf::from("fixture.bin"),
            out_dir: PathBuf::from("proof"),
            game_id: "fixture".to_string(),
        };
        let manifest: serde_json::Value = serde_json::from_slice(
            &manifest_json(&options, &stream, &gate, &summary).expect("manifest should encode"),
        )
        .expect("manifest should decode");

        assert_eq!(manifest["hourly_requirement"]["required"], json!(false));
        assert_eq!(
            manifest["hourly_requirement"]["requested_game_hours"],
            serde_json::Value::Null
        );
        assert_eq!(manifest["hourly_requirement"]["met"], json!(false));
        assert_eq!(manifest["certification"]["claimed"], json!(false));
        assert_eq!(
            manifest["certification"]["observed_game_hour_advancements"],
            json!(0)
        );
        assert_eq!(
            manifest["certification"]["observed_closed_frames"],
            json!(1)
        );
    }

    #[test]
    fn only_clean_owned_closed_frames_contribute_duration_authority() {
        let mut two_clean = Vec::new();
        append_canonical_frame(&mut two_clean, 1, 100, 0, 0, 0);
        append_canonical_frame(&mut two_clean, 4, 101, 0, 0, 0);
        let clean_gate = gate_stream(&two_clean).expect("stream should decode");
        assert!(clean_gate.ok, "{:?}", clean_gate.problems);
        assert_eq!(clean_gate.unique_game_hours, 2);
        assert_eq!(clean_gate.observed_game_hour_advancements, Some(1));

        let failed = gate_stream(&canonical_stream(
            compact_contract::COMPACT_STATUS_PARTIAL_DATA,
            0,
            0,
        ))
        .expect("stream should decode");
        assert!(!failed.ok);
        assert_eq!(failed.unique_game_hours, 0);
        assert_eq!(failed.observed_game_hour_advancements, None);
    }

    #[test]
    fn per_hour_cardinality_requires_exactly_one_perf_and_one_data() {
        let payload = valid_payload(0);
        let perf = perf_payload_with_status(0);
        for (label, records) in [
            (
                "missing PERF",
                vec![(RECORD_TYPE_DOMAIN_DATA, payload.as_slice())],
            ),
            (
                "duplicate PERF",
                vec![
                    (
                        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
                        perf.as_slice(),
                    ),
                    (
                        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
                        perf.as_slice(),
                    ),
                    (RECORD_TYPE_DOMAIN_DATA, payload.as_slice()),
                ],
            ),
            (
                "duplicate DATA",
                vec![
                    (
                        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
                        perf.as_slice(),
                    ),
                    (RECORD_TYPE_DOMAIN_DATA, payload.as_slice()),
                    (RECORD_TYPE_DOMAIN_DATA, payload.as_slice()),
                ],
            ),
        ] {
            let mut stream = Vec::new();
            let mut sequence = 1u64;
            for (record_type, record_payload) in records {
                append_compact_record(
                    &mut stream,
                    sequence,
                    100,
                    domain44::DOMAIN_ID,
                    record_type,
                    record_payload,
                );
                sequence += 1;
            }
            append_compact_record(
                &mut stream,
                sequence,
                100,
                compact_contract::COMPACT_DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &frame_end_payload_with_status(0),
            );
            let gate = gate_stream(&stream).expect("canonical framing must decode");
            assert!(!gate.ok, "{label} was accepted");
            assert!(gate.problems.iter().any(|problem| {
                problem.contains("expected exactly 1")
                    && (problem.contains("PERF") || problem.contains("DATA"))
            }));
        }
    }

    #[test]
    fn final_record_must_be_the_closing_frame_end() {
        let mut stream = canonical_stream(0, 0, 0);
        append_compact_record(
            &mut stream,
            4,
            101,
            domain44::DOMAIN_ID,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(0),
        );
        let gate = gate_stream(&stream).expect("canonical framing must decode");
        assert!(!gate.ok);
        assert!(gate.problems.iter().any(|problem| {
            problem == "stream is not closed by a final canonical FRAME_END record"
        }));
    }

    #[test]
    fn physical_trailing_bytes_after_frame_end_are_rejected() {
        let mut stream = canonical_stream(0, 0, 0);
        stream.extend_from_slice(b"trailing");
        let gate = gate_stream(&stream).expect("stream scanner returns a report");

        assert!(!gate.ok);
        assert!(gate
            .problems
            .iter()
            .any(|problem| problem.contains("truncated compact record header")));
        assert!(gate.problems.iter().any(|problem| {
            problem == "stream is not closed by a final canonical FRAME_END record"
        }));
    }

    #[test]
    fn frame_end_must_bind_the_lane_domain_count_and_reserved_word() {
        for corrupt in ["counts", "reserved"] {
            let mut stream = canonical_stream(0, 0, 0);
            let frame_payload = stream.len() - compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN;
            if corrupt == "counts" {
                stream[frame_payload + 16..frame_payload + 24].fill(0);
            } else {
                stream[frame_payload + 28..frame_payload + 32].copy_from_slice(&1u32.to_le_bytes());
            }
            let gate = gate_stream(&stream).expect("canonical framing must decode");
            assert!(!gate.ok, "{corrupt} corruption was accepted");
            assert!(gate.problems.iter().any(|problem| {
                if corrupt == "counts" {
                    problem.contains("emitted/scheduled domain counts 0/0")
                } else {
                    problem.contains("FRAME_END reserved word is nonzero")
                }
            }));
        }
    }

    #[test]
    fn frame_end_counts_must_both_equal_the_single_owned_perf_roster() {
        for (emitted, scheduled) in [(0u32, 0u32), (1, 0), (0, 1), (2, 2)] {
            let mut stream = canonical_stream(0, 0, 0);
            let frame_payload = stream.len() - compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN;
            stream[frame_payload + 16..frame_payload + 20].copy_from_slice(&emitted.to_le_bytes());
            stream[frame_payload + 20..frame_payload + 24]
                .copy_from_slice(&scheduled.to_le_bytes());
            let gate = gate_stream(&stream).expect("stream should decode");

            assert!(!gate.ok, "{emitted}/{scheduled} was accepted");
            assert_eq!(gate.unique_game_hours, 0);
            assert!(gate
                .problems
                .iter()
                .any(|problem| problem.contains(&format!(
                    "emitted/scheduled domain counts {emitted}/{scheduled}"
                ))));
        }
    }
}
