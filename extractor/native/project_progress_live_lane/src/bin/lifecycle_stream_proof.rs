use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::Read;
use std::path::Path;

use project_progress_live_lane::{
    decode_lane_frame, decode_lifecycle_events, derive_lifecycle_events, encode_lane_frame,
    encode_lifecycle_events, gate_lifecycle_events, LaneFrame, ProjectBreakthroughRow,
    ProjectLifecycleEvent, ProjectStateRow, PROJECT_BREAKTHROUGH_ROW_LEN,
    PROJECT_EVENT_KIND_BREAKTHROUGH_DELTA, PROJECT_EVENT_KIND_COMPLETION_CHANGED,
    PROJECT_STATE_ROW_LEN,
};
use serde_json::{json, Value};

const DOMAIN_PROJECTS_LIVE: u16 = 41;
const DOMAIN_FRAME: u16 = 1000;

const COMPACT_PROJECT_HEADER_LEN: usize = 64;
const COMPACT_PROJECT_HEADER_EXT_PROGRESS_V1: u32 = 3;
const COMPACT_PROJECT_HEADER_EXT_PROGRESS_V2: u32 = 4;
const COMPACT_PROJECT_ROOT_COUNTERS_LEN: usize = 24;
const COMPACT_PROJECT_ROOT_REASON_COUNTERS_LEN: usize = 48;
const COMPACT_PROJECT_PROGRESS_COUNTERS_LEN: usize = 16;
const COMPACT_PROJECT_FACILITY_ROW_LEN: usize = 32;
const COMPACT_PROJECT_ROW_LEN: usize = 24;
const COMPACT_PROJECT_SCIENTIST_ROW_LEN: usize = 32;
const PROJECT_STATE_FLAG_COMPLETED: u32 = 1;
const ROW_FLAG_DELETED: u32 = 1 << 31;
const FACILITY_KNOWN_FLAGS: u32 = 1 | (1 << 1) | (1 << 2) | ROW_FLAG_DELETED;
const PROJECT_KNOWN_FLAGS: u32 = 1 | ROW_FLAG_DELETED;
const SCIENTIST_KNOWN_FLAGS: u32 = 1 | (1 << 1) | (1 << 2) | (1 << 3) | ROW_FLAG_DELETED;

fn main() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let stream_path = args.next().ok_or_else(|| {
        "usage: project_progress_lifecycle_stream_proof <collector_stream.bin> <proof.json> <events.bin>".to_string()
    })?;
    let proof_path = args.next().ok_or_else(|| {
        "usage: project_progress_lifecycle_stream_proof <collector_stream.bin> <proof.json> <events.bin>".to_string()
    })?;
    let event_path = args.next().ok_or_else(|| {
        "usage: project_progress_lifecycle_stream_proof <collector_stream.bin> <proof.json> <events.bin>".to_string()
    })?;
    if args.next().is_some() {
        return Err("too many arguments".to_string());
    }

    let stream = read_stream_artifact(&stream_path)?;
    let parsed = parse_project_frames(&stream.decoded)?;
    let mut all_events = Vec::new();
    let mut transition_count = 0usize;
    let mut completion_events = 0usize;
    let mut breakthrough_events = 0usize;
    for pair in parsed.frames.windows(2) {
        transition_count += 1;
        let events = derive_lifecycle_events(&pair[0].frame, &pair[1].frame);
        completion_events += events
            .iter()
            .filter(|event| event.kind == PROJECT_EVENT_KIND_COMPLETION_CHANGED)
            .count();
        breakthrough_events += events
            .iter()
            .filter(|event| event.kind == PROJECT_EVENT_KIND_BREAKTHROUGH_DELTA)
            .count();
        all_events.extend(events);
    }

    let encoded = encode_lifecycle_events(&all_events);
    let decoded = decode_lifecycle_events(&encoded)?;
    let gate = gate_lifecycle_events(&encoded);
    let accepted_for_lifecycle_delta_slice = gate.ok
        && parsed.parse_errors.is_empty()
        && parsed.ends_at_clean_frame_end
        && transition_count > 0
        && !all_events.is_empty();
    fs::write(&event_path, &encoded)
        .map_err(|err| format!("failed to write {event_path}: {err}"))?;

    let proof = json!({
        "schema": "project_progress_lifecycle_stream_proof_v1",
        "artifact_kind": "extractor_lifecycle_delta_slice_proof",
        "production_certifying": false,
        "source_capture_verified": false,
        "certification": {
            "claimed": false,
            "commanded_duration_game_hours": Value::Null,
            "reason": "slice-only lifecycle evidence; this tool has no manifest-bound commanded-duration authority and does not certify a production capture",
        },
        "source_stream": path_slash(&stream_path),
        "stream_identity": {
            "compression": stream.compression,
            "stored_bytes": stream.stored_bytes,
            "stored_sha256": stream.stored_sha256,
            "decoded_bytes": stream.decoded.len(),
            "decoded_sha256": stream.decoded_sha256,
        },
        "event_compact_bin": path_slash(&event_path),
        "domain_id": DOMAIN_PROJECTS_LIVE,
        "record_count": parsed.record_count,
        "physical_record_count": parsed.physical_record_count,
        "project_data_records": parsed.frames.len(),
        "transition_count": transition_count,
        "parse_errors": parsed.parse_errors,
        "ends_at_clean_frame_end": parsed.ends_at_clean_frame_end,
        "accepted_for_lifecycle_delta_slice": accepted_for_lifecycle_delta_slice,
        "production_schema_change": false,
        "rust_decoded": decoded.len() == all_events.len(),
        "has_transition_pairs": transition_count > 0,
        "has_lifecycle_events": !all_events.is_empty(),
        "completion_transition_proven": completion_events > 0,
        "breakthrough_delta_proven": breakthrough_events > 0,
        "missing_completion_transition_fixture": completion_events == 0,
        "event_gate": {
            "ok": gate.ok,
            "events": gate.events,
            "errors": gate.errors,
        },
        "event_counts": {
            "completion_changed": completion_events,
            "breakthrough_delta": breakthrough_events,
            "total": all_events.len(),
        },
        "first_completion_changed": all_events
            .iter()
            .find(|event| event.kind == PROJECT_EVENT_KIND_COMPLETION_CHANGED)
            .map(event_json)
            .unwrap_or(Value::Null),
        "first_breakthrough_delta": all_events
            .iter()
            .find(|event| event.kind == PROJECT_EVENT_KIND_BREAKTHROUGH_DELTA)
            .map(event_json)
            .unwrap_or(Value::Null),
        "frame_summary": {
            "first_game_hour_raw": parsed.frames.first().map(|frame| frame.game_hour_raw),
            "last_game_hour_raw": parsed.frames.last().map(|frame| frame.game_hour_raw),
            "min_state_rows": parsed.frames.iter().map(|frame| frame.frame.state_rows.len()).min().unwrap_or(0),
            "max_state_rows": parsed.frames.iter().map(|frame| frame.frame.state_rows.len()).max().unwrap_or(0),
            "min_breakthrough_rows": parsed.frames.iter().map(|frame| frame.frame.breakthrough_rows.len()).min().unwrap_or(0),
            "max_breakthrough_rows": parsed.frames.iter().map(|frame| frame.frame.breakthrough_rows.len()).max().unwrap_or(0),
        },
        "event_examples": all_events.iter().take(32).map(|event| {
            json!({
                "kind": event.kind,
                "key_a": event.key_a,
                "key_b": event.key_b,
                "before": event.before,
                "after": event.after,
                "delta": event.delta,
            })
        }).collect::<Vec<_>>(),
    });
    fs::write(
        &proof_path,
        serde_json::to_string_pretty(&proof).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("failed to write {proof_path}: {err}"))?;
    if !accepted_for_lifecycle_delta_slice {
        return Err(format!("lifecycle stream proof failed; wrote {proof_path}"));
    }
    Ok(())
}

struct ParsedStream {
    record_count: usize,
    physical_record_count: usize,
    frames: Vec<ProjectFrame>,
    parse_errors: Vec<String>,
    ends_at_clean_frame_end: bool,
}

struct StreamArtifact {
    compression: &'static str,
    stored_bytes: usize,
    stored_sha256: String,
    decoded: Vec<u8>,
    decoded_sha256: String,
}

fn read_stream_artifact(path: &str) -> Result<StreamArtifact, String> {
    let stored = fs::read(path).map_err(|err| format!("failed to read {path}: {err}"))?;
    let mut reader =
        compact_stream::open(path).map_err(|err| format!("failed to open/decode {path}: {err}"))?;
    let compression = reader.compression().as_str();
    let mut decoded = Vec::new();
    reader
        .read_to_end(&mut decoded)
        .map_err(|err| format!("failed to decode {path}: {err}"))?;
    Ok(StreamArtifact {
        compression,
        stored_bytes: stored.len(),
        stored_sha256: sha256_hex(&stored),
        decoded_sha256: sha256_hex(&decoded),
        decoded,
    })
}

#[derive(Clone)]
struct ProjectFrame {
    game_hour_raw: i32,
    frame: LaneFrame,
}

#[derive(Clone)]
struct LogicalProjectState {
    row: ProjectStateRow,
    completed: Option<u32>,
}

struct DecodedProjectsPayload {
    status_flags: u32,
    row_count: u32,
    changed_rows: u32,
    delta: bool,
    project_states: Vec<ProjectStateMutation>,
    breakthroughs: Vec<BreakthroughMutation>,
}

enum ProjectStateMutation {
    Upsert(LogicalProjectState),
    Delete(u32),
}

enum BreakthroughMutation {
    Upsert(ProjectBreakthroughRow),
    Delete((u32, u32)),
}

#[derive(Clone, Copy)]
struct PerfSummary {
    row_count: u32,
    changed_rows: u32,
    status_flags: u32,
}

fn parse_project_frames(raw: &[u8]) -> Result<ParsedStream, String> {
    let mut offset = 0usize;
    let mut record_count = 0usize;
    let mut physical_record_count = 0usize;
    let mut frames = Vec::new();
    let mut parse_errors = Vec::new();
    if let Err(error) = compact_contract::stream_validator::validate_freeze_census_stream_placement(raw) {
        parse_errors.push(format!("stateful stream placement: {error}"));
    }
    let mut previous_sequence: Option<u64> = None;
    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut project_data: Option<DecodedProjectsPayload> = None;
    let mut project_perf: Option<PerfSummary> = None;
    let mut owned_perf_statuses = BTreeMap::new();
    let mut logical_states: BTreeMap<u32, LogicalProjectState> = BTreeMap::new();
    let mut logical_breakthroughs: BTreeMap<(u32, u32), ProjectBreakthroughRow> = BTreeMap::new();
    let mut baseline_seen = false;
    let mut ends_at_clean_frame_end = false;
    let mut release_epilogue_seen = false;
    while offset < raw.len() {
        // Physical EOF authority: any bytes after a clean FRAME_END invalidate
        // that boundary before the next header is even known to be parseable.
        let previous_record_closed_world = ends_at_clean_frame_end;
        ends_at_clean_frame_end = false;
        let remaining = raw.len().saturating_sub(offset);
        let header = match compact_contract::parse_compact_record_header(&raw[offset..]) {
            Ok(header) => header,
            Err(err) => {
                parse_errors.push(format!("offset {offset}: {err}"));
                break;
            }
        };
        let payload_len = header.payload_len;
        let record_len = header.record_len();
        if record_len > remaining {
            parse_errors.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining bytes {remaining}"
            ));
            break;
        }
        if let Some(previous) = previous_sequence {
            match previous.checked_add(1) {
                Some(expected) if header.sequence == expected => {}
                Some(expected) => {
                    parse_errors.push(format!(
                        "offset {offset}: sequence discontinuity {previous} -> {} (expected {expected})",
                        header.sequence
                    ));
                    break;
                }
                None => {
                    parse_errors.push(format!(
                        "offset {offset}: sequence overflow after {previous}; wrapping is forbidden"
                    ));
                    break;
                }
            }
        } else if header.sequence != 1 {
            parse_errors.push(format!(
                "offset {offset}: first compact record sequence is {}, expected 1",
                header.sequence
            ));
            break;
        }
        previous_sequence = Some(header.sequence);
        let payload =
            &raw[offset + compact_contract::COMPACT_RECORD_HEADER_LEN..offset + record_len];
        physical_record_count += 1;
        if header.domain_id != 250 {
            record_count += 1;
        }
        if let Err(error) = compact_contract::validate_compact_record_semantics(
            header.domain_id,
            header.record_type,
            header.record_flags,
            payload,
        ) {
            parse_errors.push(format!("offset {offset}: {error}"));
            break;
        }
        // Domain 250 is an instrumentation-only PERF probe. It remains subject
        // to physical framing, sequence, record-semantics, and final-EOF checks,
        // but it does not open a game-hour frame or contribute seal authority.
        if header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF
            && header.domain_id == 250
        {
            offset += record_len;
            continue;
        }
        let known_record_type =
            match compact_contract::KnownRecordType::try_from(header.record_type) {
                Ok(record_type) => record_type,
                Err(unknown) => {
                    parse_errors.push(format!(
                        "offset {offset}: unknown record type {}",
                        unknown.0
                    ));
                    break;
                }
            };
        if !matches!(
            known_record_type,
            compact_contract::KnownRecordType::FrameEnd
                | compact_contract::KnownRecordType::FreezeCensus
                | compact_contract::KnownRecordType::StreamHeader
        ) {
            match open_hour {
                Some(open) if open != header.game_hour_raw => {
                    parse_errors.push(format!(
                        "offset {offset}: record for hour {} while hour {open} is still open",
                        header.game_hour_raw
                    ));
                    break;
                }
                None => {
                    if let Some(closed) = last_closed_hour {
                        match closed.checked_add(1) {
                            Some(expected) if header.game_hour_raw == expected => {}
                            Some(expected) => {
                                parse_errors.push(format!(
                                    "offset {offset}: next frame hour is {}, expected consecutive hour {expected}",
                                    header.game_hour_raw
                                ));
                                break;
                            }
                            None => {
                                parse_errors.push(format!(
                                    "offset {offset}: game-hour overflow after {closed}"
                                ));
                                break;
                            }
                        }
                    }
                    open_hour = Some(header.game_hour_raw);
                }
                Some(_) => {}
            }
        }
        match known_record_type {
            compact_contract::KnownRecordType::DomainData => {
                if header.domain_id == DOMAIN_PROJECTS_LIVE {
                    if project_perf.is_none() {
                        parse_errors.push(format!(
                            "offset {offset}: projects_live DOMAIN_DATA precedes its DOMAIN_PERF authority for hour {}",
                            header.game_hour_raw
                        ));
                        break;
                    }
                    if project_data.is_some() {
                        parse_errors.push(format!(
                            "offset {offset}: duplicate projects_live DOMAIN_DATA for hour {}",
                            header.game_hour_raw
                        ));
                        break;
                    }
                    match parse_projects_payload(header.game_hour_raw, payload) {
                        Ok(decoded) => project_data = Some(decoded),
                        Err(err) => {
                            parse_errors.push(format!("offset {offset}: {err}"));
                            break;
                        }
                    }
                }
            }
            compact_contract::KnownRecordType::DomainPerf => {
                if compact_contract::assigned_descriptor_by_id(header.domain_id).is_none() {
                    parse_errors.push(format!(
                        "offset {offset}: unassigned domain {} DOMAIN_PERF cannot contribute frame authority",
                        header.domain_id
                    ));
                    break;
                }
                let status = match parse_owned_perf_status(header.domain_id, payload) {
                    Ok(status) => status,
                    Err(err) => {
                        parse_errors.push(format!("offset {offset}: {err}"));
                        break;
                    }
                };
                if owned_perf_statuses
                    .insert(header.domain_id, status)
                    .is_some()
                {
                    parse_errors.push(format!(
                        "offset {offset}: duplicate owned DOMAIN_PERF for domain {} in hour {}",
                        header.domain_id, header.game_hour_raw
                    ));
                    break;
                }
                if header.domain_id == DOMAIN_PROJECTS_LIVE {
                    if project_perf.is_some() {
                        parse_errors.push(format!(
                            "offset {offset}: duplicate projects_live DOMAIN_PERF for hour {}",
                            header.game_hour_raw
                        ));
                        break;
                    }
                    match parse_projects_perf(payload) {
                        Ok(perf) => project_perf = Some(perf),
                        Err(err) => {
                            parse_errors.push(format!("offset {offset}: {err}"));
                            break;
                        }
                    }
                }
            }
            compact_contract::KnownRecordType::FrameEnd => {
                let Some(open) = open_hour else {
                    parse_errors.push(format!(
                        "offset {offset}: bare FRAME_END for hour {}",
                        header.game_hour_raw
                    ));
                    break;
                };
                if open != header.game_hour_raw {
                    parse_errors.push(format!(
                        "offset {offset}: FRAME_END for hour {} while hour {open} is open",
                        header.game_hour_raw
                    ));
                    break;
                }
                let Some(data) = project_data.take() else {
                    parse_errors.push(format!(
                        "offset {offset}: hour {open} has no projects_live DOMAIN_DATA"
                    ));
                    break;
                };
                let Some(perf) = project_perf.take() else {
                    parse_errors.push(format!(
                        "offset {offset}: hour {open} has no projects_live DOMAIN_PERF"
                    ));
                    break;
                };
                if perf.row_count != data.row_count
                    || perf.changed_rows != data.changed_rows
                    || perf.status_flags != data.status_flags
                {
                    parse_errors.push(format!(
                        "offset {offset}: projects_live PERF/DATA authority mismatch for hour {open} (perf rows/changed/status {}/{}/{:#x}, data {}/{}/{:#x})",
                        perf.row_count,
                        perf.changed_rows,
                        perf.status_flags,
                        data.row_count,
                        data.changed_rows,
                        data.status_flags
                    ));
                    break;
                }
                if let Err(err) = validate_frame_end_payload(payload, &owned_perf_statuses) {
                    parse_errors.push(format!("offset {offset}: {err}"));
                    break;
                }
                if data.delta && !baseline_seen {
                    parse_errors.push(format!(
                        "offset {offset}: projects_live delta precedes a full baseline"
                    ));
                    break;
                }
                if !data.delta {
                    logical_states.clear();
                    logical_breakthroughs.clear();
                    baseline_seen = true;
                }
                apply_project_mutations(data, &mut logical_states, &mut logical_breakthroughs);
                frames.push(ProjectFrame {
                    game_hour_raw: open,
                    frame: LaneFrame {
                        game_hour_raw: open,
                        status_flags: 0,
                        state_rows: logical_states
                            .values()
                            .filter(|state| state.completed.is_some())
                            .map(|state| state.row.clone())
                            .collect(),
                        breakthrough_rows: logical_breakthroughs.values().cloned().collect(),
                    },
                });
                open_hour = None;
                last_closed_hour = Some(open);
                owned_perf_statuses.clear();
                ends_at_clean_frame_end = true;
            }
            compact_contract::KnownRecordType::FreezeCensus => {
                if let Ok(census) = compact_contract::freeze_census_record::FreezeCensusRecordV1::decode(payload) {
                    ends_at_clean_frame_end = compact_contract::freeze_census_record::release_epilogue_closes_stream_policy(previous_record_closed_world, census.phase(), release_epilogue_seen);
                    release_epilogue_seen |= census.phase() == compact_contract::freeze_census_record::FreezeCensusPhase::Release;
                }
            }
            compact_contract::KnownRecordType::ContentIdentityCensus
            | compact_contract::KnownRecordType::StreamHeader => {}
            compact_contract::KnownRecordType::DomainGap
            | compact_contract::KnownRecordType::FrameRefused
            | compact_contract::KnownRecordType::FrameDegraded
            | compact_contract::KnownRecordType::CaptureTerminated
            | compact_contract::KnownRecordType::ClockUnreadableInterval
            | compact_contract::KnownRecordType::UntrustedClockRecovery => {
                parse_errors.push(format!(
                    "offset {offset}: record type {} is outside the project proof grammar",
                    known_record_type.canonical_name()
                ));
                break;
            }
        }
        offset += record_len;
    }
    if parse_errors.is_empty() {
        if let Some(open) = open_hour {
            parse_errors.push(format!(
                "game hour {open} left open at EOF (stream does not end at FRAME_END)"
            ));
        } else if frames.is_empty() {
            parse_errors.push("stream closes no game hour (no FRAME_END records)".to_string());
        } else if !ends_at_clean_frame_end {
            parse_errors.push("stream does not end at a clean FRAME_END boundary".to_string());
        }
    }
    Ok(ParsedStream {
        record_count,
        physical_record_count,
        frames,
        parse_errors,
        ends_at_clean_frame_end,
    })
}

fn validate_frame_end_payload(
    payload: &[u8],
    owned_perf_statuses: &BTreeMap<u16, u32>,
) -> Result<(), String> {
    let emitted_domains = read_u32_le(payload, 16);
    let scheduled_domains = read_u32_le(payload, 20);
    let status_flags = read_u32_le(payload, 24);
    let reserved = read_u32_le(payload, 28);

    let unknown_status = compact_contract::compact_status_unknown_flags_v1(status_flags);
    if unknown_status != 0 {
        return Err(format!(
            "FRAME_END status {status_flags:#x} contains unknown bits {unknown_status:#x}"
        ));
    }
    let fatal_status =
        compact_contract::compact_status_failure_flags_after_expected_partial_tolerance(
            DOMAIN_FRAME,
            status_flags,
        );
    if fatal_status != 0 {
        return Err(format!(
            "FRAME_END status {status_flags:#x} contains fatal bits {fatal_status:#x}"
        ));
    }
    if reserved != 0 {
        return Err(format!(
            "FRAME_END reserved word is nonzero ({reserved:#x})"
        ));
    }
    if emitted_domains != scheduled_domains {
        return Err(format!(
            "FRAME_END emitted_domains {emitted_domains} != scheduled_domains {scheduled_domains}"
        ));
    }
    let observed_owned_perf_domains = u32::try_from(owned_perf_statuses.len())
        .map_err(|_| "observed owned DOMAIN_PERF roster exceeds u32".to_string())?;
    if emitted_domains != observed_owned_perf_domains {
        return Err(format!(
            "FRAME_END emitted_domains {emitted_domains} != independently observed owned DOMAIN_PERF roster count {observed_owned_perf_domains}"
        ));
    }
    let observed_status_or = owned_perf_statuses
        .values()
        .copied()
        .fold(0_u32, |combined, status| combined | status);
    if status_flags != observed_status_or {
        return Err(format!(
            "FRAME_END status {status_flags:#x} != OR of independently observed owned DOMAIN_PERF statuses {observed_status_or:#x}"
        ));
    }
    Ok(())
}

fn parse_owned_perf_status(domain_id: u16, payload: &[u8]) -> Result<u32, String> {
    if payload.len() < compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN {
        return Err(format!(
            "owned domain {domain_id} PERF payload too short: {}",
            payload.len()
        ));
    }
    if read_u32_le(payload, 36) != 0 {
        return Err(format!(
            "owned domain {domain_id} PERF reserved field is nonzero"
        ));
    }
    let status_flags = read_u32_le(payload, 32);
    let unknown_status = compact_contract::compact_status_unknown_flags_v1(status_flags);
    if unknown_status != 0 {
        return Err(format!(
            "owned domain {domain_id} PERF status {status_flags:#x} contains unknown bits {unknown_status:#x}"
        ));
    }
    Ok(status_flags)
}

fn parse_projects_payload(
    _game_hour_raw: i32,
    payload: &[u8],
) -> Result<DecodedProjectsPayload, String> {
    if payload.len() < COMPACT_PROJECT_HEADER_LEN {
        return Err(format!("projects payload too short: {}", payload.len()));
    }
    let status_flags = read_u32_le(payload, 16);
    let unknown_status = compact_contract::compact_status_unknown_flags_v1(status_flags);
    if unknown_status != 0 {
        return Err(format!(
            "projects payload status {status_flags:#x} contains unknown bits {unknown_status:#x}"
        ));
    }
    let failure_flags = compact_contract::compact_status_for_global_interpretation(
        DOMAIN_PROJECTS_LIVE,
        status_flags,
    ) & compact_contract::COMPACT_STATUS_FAILURE_MASK;
    if failure_flags != 0 {
        return Err(format!(
            "projects payload has non-authoritative failure status {failure_flags:#x}"
        ));
    }
    let delta = status_flags & compact_contract::COMPACT_STATUS_DELTA_DATA != 0;
    let row_count = read_u32_le(payload, 8);
    let changed_rows = read_u32_le(payload, 12);
    let facility_count = read_u32_le(payload, 20) as usize;
    let project_count = read_u32_le(payload, 24) as usize;
    let scientist_count = read_u32_le(payload, 28) as usize;
    let active_facility_count = read_u32_le(payload, 32) as usize;
    let active_project_count = read_u32_le(payload, 36) as usize;
    let active_scientist_count = read_u32_le(payload, 40) as usize;
    let source_kind = read_u32_le(payload, 48);
    if !matches!(source_kind, 1 | 2) {
        return Err(format!(
            "projects payload has invalid source_kind {source_kind}"
        ));
    }
    let header_extension = read_u32_le(payload, 52);
    let mut project_header_len = COMPACT_PROJECT_HEADER_LEN;
    match header_extension {
        COMPACT_PROJECT_HEADER_EXT_PROGRESS_V1 | COMPACT_PROJECT_HEADER_EXT_PROGRESS_V2 => {}
        other => {
            return Err(format!(
                "projects lifecycle proof requires progress header extension 3 or 4, got {other}"
            ))
        }
    }
    project_header_len = project_header_len
        .checked_add(COMPACT_PROJECT_ROOT_COUNTERS_LEN)
        .and_then(|value| value.checked_add(COMPACT_PROJECT_ROOT_REASON_COUNTERS_LEN))
        .and_then(|value| value.checked_add(COMPACT_PROJECT_PROGRESS_COUNTERS_LEN))
        .ok_or_else(|| "projects header length overflow".to_string())?;
    if payload.len() < project_header_len {
        return Err(format!(
            "projects payload too short for progress header: {} < {project_header_len}",
            payload.len()
        ));
    }
    let project_state_count = read_u32_le(payload, 136) as usize;
    let breakthrough_count = read_u32_le(payload, 140) as usize;
    let expected_rows = facility_count
        .checked_add(project_count)
        .and_then(|value| value.checked_add(scientist_count))
        .and_then(|value| value.checked_add(project_state_count))
        .and_then(|value| value.checked_add(breakthrough_count))
        .ok_or_else(|| "projects row count overflow".to_string())?;
    if row_count as usize != expected_rows || changed_rows as usize != expected_rows {
        return Err(format!(
            "projects header row_count/changed_rows {row_count}/{changed_rows} != emitted rows {expected_rows}"
        ));
    }
    if !delta
        && (facility_count > active_facility_count
            || project_count > active_project_count
            || scientist_count > active_scientist_count)
    {
        return Err("projects baseline emitted row count exceeds active count".to_string());
    }

    let facilities_start = project_header_len;
    let projects_start = checked_rows_end(
        facilities_start,
        facility_count,
        COMPACT_PROJECT_FACILITY_ROW_LEN,
    )?;
    let scientists_start =
        checked_rows_end(projects_start, project_count, COMPACT_PROJECT_ROW_LEN)?;
    let state_start = checked_rows_end(
        scientists_start,
        scientist_count,
        COMPACT_PROJECT_SCIENTIST_ROW_LEN,
    )?;
    let breakthrough_start =
        checked_rows_end(state_start, project_state_count, PROJECT_STATE_ROW_LEN)?;
    let fixed_end = checked_rows_end(
        breakthrough_start,
        breakthrough_count,
        PROJECT_BREAKTHROUGH_ROW_LEN,
    )?;
    if payload.len() < fixed_end {
        return Err(format!(
            "projects fixed rows exceed payload: need {fixed_end}, have {}",
            payload.len()
        ));
    }

    let mut facility_strings = Vec::with_capacity(facility_count);
    let mut seen_facilities = BTreeSet::new();
    for index in 0..facility_count {
        let base = facilities_start + index * COMPACT_PROJECT_FACILITY_ROW_LEN;
        let id = read_u32_le(payload, base);
        let flags = read_u32_le(payload, base + 8);
        if id == 0 || !seen_facilities.insert(id) {
            return Err(format!(
                "projects facility row {index} has duplicate/zero id {id}"
            ));
        }
        if flags & !FACILITY_KNOWN_FLAGS != 0 {
            return Err(format!(
                "projects facility row {index} has unknown flags {flags:#x}"
            ));
        }
        let deleted = flags & ROW_FLAG_DELETED != 0;
        if deleted {
            if !delta {
                return Err(format!(
                    "projects baseline facility row {index} is a tombstone"
                ));
            }
            if flags != ROW_FLAG_DELETED
                || payload[base + 4..base + COMPACT_PROJECT_FACILITY_ROW_LEN]
                    .chunks_exact(4)
                    .enumerate()
                    .any(|(word, bytes)| word != 1 && bytes != [0, 0, 0, 0])
            {
                return Err(format!(
                    "projects facility tombstone row {index} has live fields"
                ));
            }
        } else {
            let active_project_handle = read_u32_le(payload, base + 12);
            let scientist_char_id = read_u32_le(payload, base + 16);
            if flags & 1 == 0 && active_project_handle != 0 {
                return Err(format!(
                    "projects facility row {index} has active-project handle without flag"
                ));
            }
            if flags & (1 << 1) == 0 && scientist_char_id != 0 {
                return Err(format!(
                    "projects facility row {index} has scientist id without flag"
                ));
            }
            if read_u32_le(payload, base + 24) != 0 || read_u32_le(payload, base + 28) != 0 {
                return Err(format!(
                    "projects facility row {index} reserved fields are nonzero"
                ));
            }
        }
        facility_strings.push((flags, deleted));
    }

    let mut seen_projects = BTreeSet::new();
    for index in 0..project_count {
        let base = projects_start + index * COMPACT_PROJECT_ROW_LEN;
        let id = read_u32_le(payload, base);
        let flags = read_u32_le(payload, base + 16);
        if id == 0 || !seen_projects.insert(id) {
            return Err(format!(
                "projects project row {index} has duplicate/zero id {id}"
            ));
        }
        if flags & !PROJECT_KNOWN_FLAGS != 0 {
            return Err(format!(
                "projects project row {index} has unknown flags {flags:#x}"
            ));
        }
        if flags & ROW_FLAG_DELETED != 0 {
            if !delta
                || flags != ROW_FLAG_DELETED
                || payload[base + 4..base + 16].iter().any(|byte| *byte != 0)
                || read_u32_le(payload, base + 20) != 0
            {
                return Err(format!("projects project tombstone row {index} is invalid"));
            }
        } else {
            if flags & 1 == 0 && read_u32_le(payload, base + 12) != 0 {
                return Err(format!(
                    "projects project row {index} has worked-at-facility value without flag"
                ));
            }
            if read_u32_le(payload, base + 20) != 0 {
                return Err(format!(
                    "projects project row {index} reserved field is nonzero"
                ));
            }
        }
    }

    let mut scientist_strings = Vec::with_capacity(scientist_count);
    let mut seen_scientists = BTreeSet::new();
    for index in 0..scientist_count {
        let base = scientists_start + index * COMPACT_PROJECT_SCIENTIST_ROW_LEN;
        let id = read_u32_le(payload, base);
        let flags = read_u32_le(payload, base + 4);
        if id == 0 || !seen_scientists.insert(id) {
            return Err(format!(
                "projects scientist row {index} has duplicate/zero id {id}"
            ));
        }
        if flags & !SCIENTIST_KNOWN_FLAGS != 0 {
            return Err(format!(
                "projects scientist row {index} has unknown flags {flags:#x}"
            ));
        }
        let deleted = flags & ROW_FLAG_DELETED != 0;
        if deleted {
            if !delta
                || flags != ROW_FLAG_DELETED
                || payload[base + 8..base + COMPACT_PROJECT_SCIENTIST_ROW_LEN]
                    .iter()
                    .any(|byte| *byte != 0)
            {
                return Err(format!(
                    "projects scientist tombstone row {index} is invalid"
                ));
            }
        } else {
            if flags & (1 << 2) == 0 && read_u32_le(payload, base + 8) != 0 {
                return Err(format!(
                    "projects scientist row {index} has trait count without flag"
                ));
            }
            if flags & (1 << 3) == 0 && payload[base + 12..base + 28].iter().any(|byte| *byte != 0)
            {
                return Err(format!(
                    "projects scientist row {index} has skill values without flag"
                ));
            }
            if read_u32_le(payload, base + 28) != 0 {
                return Err(format!(
                    "projects scientist row {index} reserved field is nonzero"
                ));
            }
        }
        scientist_strings.push((flags, deleted));
    }

    let mut project_states = Vec::with_capacity(project_state_count);
    let mut live_state_rows = Vec::new();
    let mut seen_states = BTreeSet::new();
    let mut cursor = state_start;
    for index in 0..project_state_count {
        let row = ProjectStateRow {
            id: read_u32_le(payload, cursor),
            static_def_handle: read_u32_le(payload, cursor + 4),
            current_state_offset: read_u32_le(payload, cursor + 8),
            current_state_enum: read_u32_le(payload, cursor + 12),
            prototype_state_enum: read_u32_le(payload, cursor + 16),
            simple_state_1_enum: read_u32_le(payload, cursor + 20),
            simple_state_2_enum: read_u32_le(payload, cursor + 24),
            simple_state_3_enum: read_u32_le(payload, cursor + 28),
            prototype_iterations: read_u32_le(payload, cursor + 32),
            phase_progress_raw: read_u32_le(payload, cursor + 36),
            phase_target_raw: read_u32_le(payload, cursor + 40),
            state_flags: read_u32_le(payload, cursor + 44),
        };
        if row.id == 0 || !seen_states.insert(row.id) {
            return Err(format!(
                "projects state row {index} has duplicate/zero id {}",
                row.id
            ));
        }
        let deleted = row.state_flags & ROW_FLAG_DELETED != 0;
        let allowed_live = if header_extension == COMPACT_PROJECT_HEADER_EXT_PROGRESS_V2 {
            PROJECT_STATE_FLAG_COMPLETED
        } else {
            0
        };
        if deleted {
            if !delta
                || row.state_flags != ROW_FLAG_DELETED
                || payload[cursor + 4..cursor + PROJECT_STATE_ROW_LEN - 4]
                    .iter()
                    .any(|byte| *byte != 0)
            {
                return Err(format!("projects state tombstone row {index} is invalid"));
            }
            project_states.push(ProjectStateMutation::Delete(row.id));
        } else {
            if row.state_flags & !allowed_live != 0 {
                return Err(format!(
                    "projects v{header_extension} state row {index} has invalid flags {:#x}",
                    row.state_flags
                ));
            }
            let completed = (header_extension == COMPACT_PROJECT_HEADER_EXT_PROGRESS_V2)
                .then_some((row.state_flags & PROJECT_STATE_FLAG_COMPLETED != 0) as u32);
            live_state_rows.push(row.clone());
            project_states.push(ProjectStateMutation::Upsert(LogicalProjectState {
                row,
                completed,
            }));
        }
        cursor += PROJECT_STATE_ROW_LEN;
    }

    let mut breakthroughs = Vec::with_capacity(breakthrough_count);
    let mut live_breakthrough_rows = Vec::new();
    let mut seen_breakthroughs = BTreeSet::new();
    cursor = breakthrough_start;
    for index in 0..breakthrough_count {
        let row = ProjectBreakthroughRow {
            tag_db_id: read_u32_le(payload, cursor),
            specialization_handle: read_u32_le(payload, cursor + 4),
            points_raw: read_u32_le(payload, cursor + 8),
        };
        let flags = read_u32_le(payload, cursor + 12);
        let key = (row.tag_db_id, row.specialization_handle);
        if row.tag_db_id == 0 || row.specialization_handle == 0 || !seen_breakthroughs.insert(key) {
            return Err(format!(
                "projects breakthrough row {index} has duplicate/zero key"
            ));
        }
        if flags == ROW_FLAG_DELETED {
            if !delta || row.points_raw != 0 {
                return Err(format!(
                    "projects breakthrough tombstone row {index} is invalid"
                ));
            }
            breakthroughs.push(BreakthroughMutation::Delete(key));
        } else if flags != 0 {
            return Err(format!(
                "projects breakthrough row {index} has unknown flags {flags:#x}"
            ));
        } else {
            live_breakthrough_rows.push(row.clone());
            breakthroughs.push(BreakthroughMutation::Upsert(row));
        }
        cursor += PROJECT_BREAKTHROUGH_ROW_LEN;
    }

    let authority_frame = LaneFrame {
        game_hour_raw: 0,
        status_flags,
        state_rows: live_state_rows,
        breakthrough_rows: live_breakthrough_rows,
    };
    decode_lane_frame(&encode_lane_frame(&authority_frame))
        .map_err(|error| format!("projects canonical row decoder rejected payload: {error}"))?;

    cursor = fixed_end;
    for (index, (flags, deleted)) in facility_strings.into_iter().enumerate() {
        let building = read_compact_string(payload, &mut cursor, "facility building")?;
        if deleted && !building.is_empty()
            || !deleted && building.is_empty() != (flags & (1 << 2) == 0)
        {
            return Err(format!(
                "projects facility row {index} building flag/string mismatch"
            ));
        }
    }
    for (index, (flags, deleted)) in scientist_strings.into_iter().enumerate() {
        let token = read_compact_string(payload, &mut cursor, "scientist token")?;
        let display = read_compact_string(payload, &mut cursor, "scientist display")?;
        if deleted && (!token.is_empty() || !display.is_empty())
            || !deleted && token.is_empty() != (flags & 1 == 0)
            || !deleted && display.is_empty() != (flags & (1 << 1) == 0)
        {
            return Err(format!(
                "projects scientist row {index} string flag mismatch"
            ));
        }
    }
    if cursor != payload.len() {
        validate_lifecycle_trailer(payload, cursor, delta)?;
    }

    Ok(DecodedProjectsPayload {
        status_flags,
        row_count,
        changed_rows,
        delta,
        project_states,
        breakthroughs,
    })
}

fn parse_projects_perf(payload: &[u8]) -> Result<PerfSummary, String> {
    let status_flags = parse_owned_perf_status(DOMAIN_PROJECTS_LIVE, payload)?;
    Ok(PerfSummary {
        row_count: read_u32_le(payload, 16),
        changed_rows: read_u32_le(payload, 20),
        status_flags,
    })
}

fn apply_project_mutations(
    decoded: DecodedProjectsPayload,
    states: &mut BTreeMap<u32, LogicalProjectState>,
    breakthroughs: &mut BTreeMap<(u32, u32), ProjectBreakthroughRow>,
) {
    for mutation in decoded.project_states {
        match mutation {
            ProjectStateMutation::Upsert(state) => {
                states.insert(state.row.id, state);
            }
            ProjectStateMutation::Delete(id) => {
                states.remove(&id);
            }
        }
    }
    for mutation in decoded.breakthroughs {
        match mutation {
            BreakthroughMutation::Upsert(row) => {
                breakthroughs.insert((row.tag_db_id, row.specialization_handle), row);
            }
            BreakthroughMutation::Delete(key) => {
                breakthroughs.remove(&key);
            }
        }
    }
}

fn checked_rows_end(start: usize, count: usize, row_len: usize) -> Result<usize, String> {
    count
        .checked_mul(row_len)
        .and_then(|bytes| start.checked_add(bytes))
        .ok_or_else(|| "projects payload row extent overflow".to_string())
}

fn read_compact_string<'a>(
    payload: &'a [u8],
    cursor: &mut usize,
    label: &str,
) -> Result<&'a str, String> {
    let prefix_end = cursor
        .checked_add(2)
        .ok_or_else(|| format!("projects {label} length prefix overflow"))?;
    if prefix_end > payload.len() {
        return Err(format!("projects {label} length prefix is truncated"));
    }
    let len = u16::from_le_bytes([payload[*cursor], payload[*cursor + 1]]) as usize;
    let start = prefix_end;
    let end = start
        .checked_add(len)
        .ok_or_else(|| format!("projects {label} length overflow"))?;
    if end > payload.len() {
        return Err(format!("projects {label} bytes are truncated"));
    }
    *cursor = end;
    std::str::from_utf8(&payload[start..end])
        .map_err(|error| format!("projects {label} is not UTF-8: {error}"))
}

fn validate_lifecycle_trailer(payload: &[u8], offset: usize, delta: bool) -> Result<(), String> {
    const HEADER_LEN: usize = 20;
    const ROW_LEN: usize = 12;
    let trailer = payload
        .get(offset..)
        .ok_or_else(|| "projects lifecycle trailer offset exceeds payload".to_string())?;
    if trailer.len() < HEADER_LEN || &trailer[0..4] != b"PLCF" {
        return Err("projects trailing bytes are not a complete PLCF trailer".to_string());
    }
    let version = u16::from_le_bytes([trailer[4], trailer[5]]);
    let header_len = u16::from_le_bytes([trailer[6], trailer[7]]) as usize;
    if version != 1 || header_len != HEADER_LEN {
        return Err(format!(
            "projects PLCF header shape invalid (version {version}, header_len {header_len})"
        ));
    }
    let rows = read_u32_le(trailer, 8) as usize;
    let aborted_countries = read_u32_le(trailer, 16);
    if aborted_countries != 0 {
        return Err(format!(
            "projects PLCF reports {aborted_countries} aborted countries in an authoritative frame"
        ));
    }
    let expected = checked_rows_end(HEADER_LEN, rows, ROW_LEN)?;
    if trailer.len() != expected {
        return Err(format!(
            "projects PLCF length {} != expected {expected}",
            trailer.len()
        ));
    }
    let mut seen = BTreeSet::new();
    for index in 0..rows {
        let base = HEADER_LEN + index * ROW_LEN;
        let tag = read_u32_le(trailer, base);
        let local_id = read_u32_le(trailer, base + 4);
        let flags = read_u32_le(trailer, base + 8);
        if tag == 0 || local_id == 0 || !seen.insert((tag, local_id)) {
            return Err(format!("projects PLCF row {index} has duplicate/zero key"));
        }
        if flags & !(1 | (1 << 1) | ROW_FLAG_DELETED) != 0 {
            return Err(format!(
                "projects PLCF row {index} has unknown flags {flags:#x}"
            ));
        }
        if flags & ROW_FLAG_DELETED != 0 && (!delta || flags != ROW_FLAG_DELETED) {
            return Err(format!("projects PLCF tombstone row {index} is invalid"));
        }
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut padded = bytes.to_vec();
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    let mut h = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    for chunk in padded.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (index, word) in chunk.chunks_exact(4).enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for index in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for index in 0..8 {
            h[index] = h[index].wrapping_add(v[index]);
        }
    }
    h.iter().map(|word| format!("{word:08x}")).collect()
}

fn event_json(event: &ProjectLifecycleEvent) -> Value {
    json!({
        "kind": event.kind,
        "key_a": event.key_a,
        "key_b": event.key_b,
        "before": event.before,
        "after": event.after,
        "delta": event.delta,
    })
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn path_slash(path: &str) -> String {
    Path::new(path).to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_record(
        out: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        record_flags: u32,
        payload: &[u8],
    ) {
        out.extend_from_slice(compact_contract::COMPACT_RECORD_MAGIC);
        out.extend_from_slice(&compact_contract::COMPACT_RECORD_VERSION.to_le_bytes());
        out.extend_from_slice(&(compact_contract::COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour_raw.to_le_bytes());
        out.extend_from_slice(&domain_id.to_le_bytes());
        out.extend_from_slice(&record_type.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&record_flags.to_le_bytes());
        out.extend_from_slice(payload);
    }

    fn push_frame_end(out: &mut Vec<u8>, sequence: u64, game_hour_raw: i32) {
        push_frame_end_with(out, sequence, game_hour_raw, 1, 1, 0, 0);
    }

    fn push_frame_end_with(
        out: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        emitted_domains: u32,
        scheduled_domains: u32,
        status_flags: u32,
        reserved: u32,
    ) {
        let mut payload = [0; compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN];
        payload[16..20].copy_from_slice(&emitted_domains.to_le_bytes());
        payload[20..24].copy_from_slice(&scheduled_domains.to_le_bytes());
        payload[24..28].copy_from_slice(&status_flags.to_le_bytes());
        payload[28..32].copy_from_slice(&reserved.to_le_bytes());
        push_record(
            out,
            sequence,
            game_hour_raw,
            DOMAIN_FRAME,
            compact_contract::COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &payload,
        );
    }

    fn perf_base_payload(row_count: u32, changed_rows: u32, status_flags: u32) -> Vec<u8> {
        let mut payload = vec![0; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN];
        payload[16..20].copy_from_slice(&row_count.to_le_bytes());
        payload[20..24].copy_from_slice(&changed_rows.to_le_bytes());
        payload[32..36].copy_from_slice(&status_flags.to_le_bytes());
        payload
    }

    fn perf_payload_with_trailer(magic: &[u8; 4]) -> Vec<u8> {
        let mut payload = perf_base_payload(0, 0, 0);
        payload.extend_from_slice(magic);
        payload.extend_from_slice(
            &compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION.to_le_bytes(),
        );
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload
    }

    fn projects_payload(
        status_flags: u32,
        extension: u32,
        states: &[(u32, u32, u32)],
        breakthroughs: &[(u32, u32, u32, u32)],
    ) -> Vec<u8> {
        let row_count = (states.len() + breakthroughs.len()) as u32;
        let mut payload = vec![0; 152];
        payload[8..12].copy_from_slice(&row_count.to_le_bytes());
        payload[12..16].copy_from_slice(&row_count.to_le_bytes());
        payload[16..20].copy_from_slice(&status_flags.to_le_bytes());
        payload[36..40].copy_from_slice(&(states.len() as u32).to_le_bytes());
        payload[48..52].copy_from_slice(&1u32.to_le_bytes());
        payload[52..56].copy_from_slice(&extension.to_le_bytes());
        payload[136..140].copy_from_slice(&(states.len() as u32).to_le_bytes());
        payload[140..144].copy_from_slice(&(breakthroughs.len() as u32).to_le_bytes());
        payload[144..148].copy_from_slice(&(states.len() as u32).to_le_bytes());
        payload[148..152].copy_from_slice(&(breakthroughs.len() as u32).to_le_bytes());
        for &(id, static_handle, flags) in states {
            payload.extend_from_slice(&id.to_le_bytes());
            payload.extend_from_slice(&static_handle.to_le_bytes());
            payload.extend_from_slice(&0x18u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&2u32.to_le_bytes());
            payload.extend_from_slice(&3u32.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&10u32.to_le_bytes());
            payload.extend_from_slice(&60u32.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
        }
        for &(tag, handle, points, flags) in breakthroughs {
            payload.extend_from_slice(&tag.to_le_bytes());
            payload.extend_from_slice(&handle.to_le_bytes());
            payload.extend_from_slice(&points.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
        }
        payload
    }

    fn push_projects_frame(out: &mut Vec<u8>, sequence: &mut u64, hour: i32, payload: &[u8]) {
        let row_count = read_u32_le(payload, 8);
        let changed_rows = read_u32_le(payload, 12);
        let status_flags = read_u32_le(payload, 16);
        let perf = perf_base_payload(row_count, changed_rows, status_flags);
        push_record(
            out,
            *sequence,
            hour,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf,
        );
        *sequence += 1;
        push_record(
            out,
            *sequence,
            hour,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            payload,
        );
        *sequence += 1;
        push_frame_end_with(out, *sequence, hour, 1, 1, status_flags, 0);
        *sequence += 1;
    }

    #[test]
    fn accepts_sequence_one_origin_and_canonical_contextual_flags() {
        let mut raw = Vec::new();
        let payload = projects_payload(0, 4, &[(10, 20, 0)], &[(1, 30, 5, 0)]);
        push_record(
            &mut raw,
            1,
            72,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &{
                let mut perf = perf_base_payload(2, 2, 0);
                perf.extend_from_slice(
                    compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC,
                );
                perf.extend_from_slice(
                    &compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION.to_le_bytes(),
                );
                perf.extend_from_slice(&0u16.to_le_bytes());
                perf
            },
        );
        push_record(
            &mut raw,
            2,
            72,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        push_frame_end(&mut raw, 3, 72);

        let parsed = parse_project_frames(&raw).expect("framing result");
        assert!(parsed.parse_errors.is_empty(), "{:?}", parsed.parse_errors);
        assert!(parsed.ends_at_clean_frame_end);
        assert_eq!(parsed.record_count, 3);
    }

    #[test]
    fn instrumentation_probe_does_not_open_or_inflate_frame_authority() {
        let mut raw = Vec::new();
        push_record(
            &mut raw,
            1,
            i32::MIN,
            250,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_base_payload(0, 0, 0),
        );
        let payload = projects_payload(0, 4, &[(10, 20, 0)], &[]);
        let mut sequence = 2;
        push_projects_frame(&mut raw, &mut sequence, 73, &payload);

        let parsed = parse_project_frames(&raw).expect("framing result");
        assert!(parsed.parse_errors.is_empty(), "{:?}", parsed.parse_errors);
        assert!(parsed.ends_at_clean_frame_end);
        assert_eq!(parsed.record_count, 3);
        assert_eq!(parsed.physical_record_count, 4);
    }

    #[test]
    fn unassigned_perf_cannot_inflate_frame_authority() {
        let mut raw = Vec::new();
        push_record(
            &mut raw,
            1,
            73,
            u16::MAX,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_base_payload(0, 0, 0),
        );

        let parsed = parse_project_frames(&raw).expect("framing result");
        assert!(parsed
            .parse_errors
            .iter()
            .any(|error| error.contains("unassigned domain 65535 DOMAIN_PERF")));
        assert!(!parsed.ends_at_clean_frame_end);
    }

    #[test]
    fn rejects_wrapping_sequence_continuity() {
        let mut raw = Vec::new();
        push_record(
            &mut raw,
            u64::MAX - 1,
            72,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &[],
        );
        push_record(
            &mut raw,
            u64::MAX,
            72,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &[],
        );
        push_frame_end(&mut raw, 0, 72);

        let parsed = parse_project_frames(&raw).expect("framing result");
        assert!(parsed
            .parse_errors
            .iter()
            .any(|error| error.contains("first compact record sequence")));
    }

    #[test]
    fn rejects_missing_sequence_one_prefix() {
        let mut raw = Vec::new();
        push_record(
            &mut raw,
            2,
            72,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &[],
        );
        push_frame_end(&mut raw, 3, 72);

        let parsed = parse_project_frames(&raw).expect("framing result");
        assert!(parsed
            .parse_errors
            .iter()
            .any(|error| error.contains("first compact record sequence is 2, expected 1")));
    }

    #[test]
    fn rejects_bit30_without_a_perf_trailer() {
        let mut raw = Vec::new();
        push_record(
            &mut raw,
            1,
            73,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &perf_base_payload(0, 0, 0),
        );

        let parsed = parse_project_frames(&raw).expect("framing result");
        assert!(parsed
            .parse_errors
            .iter()
            .any(|error| { error.contains("DOMAIN_PERF diagnostic trailer header is truncated") }));
    }

    #[test]
    fn rejects_unannounced_perf_trailer() {
        let mut raw = Vec::new();
        push_record(
            &mut raw,
            1,
            74,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload_with_trailer(
                compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC,
            ),
        );

        let parsed = parse_project_frames(&raw).expect("framing result");
        assert!(parsed
            .parse_errors
            .iter()
            .any(|error| error.contains("unannounced trailer byte(s)")));
    }

    #[test]
    fn rejects_malformed_announced_perf_trailer() {
        let mut raw = Vec::new();
        push_record(
            &mut raw,
            1,
            75,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &perf_payload_with_trailer(b"BAD!"),
        );

        let parsed = parse_project_frames(&raw).expect("framing result");
        assert!(parsed
            .parse_errors
            .iter()
            .any(|error| error.contains("diagnostic trailer magic is invalid")));
    }

    #[test]
    fn accepts_valid_announced_perf_trailer() {
        let mut raw = Vec::new();
        let payload = projects_payload(0, 4, &[(10, 20, 0)], &[(1, 30, 5, 0)]);
        push_record(
            &mut raw,
            1,
            76,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &{
                let mut perf = perf_base_payload(2, 2, 0);
                perf.extend_from_slice(
                    compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC,
                );
                perf.extend_from_slice(
                    &compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION.to_le_bytes(),
                );
                perf.extend_from_slice(&0u16.to_le_bytes());
                perf
            },
        );
        push_record(
            &mut raw,
            2,
            76,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        push_frame_end(&mut raw, 3, 76);

        let parsed = parse_project_frames(&raw).expect("framing result");
        assert!(parsed.parse_errors.is_empty(), "{:?}", parsed.parse_errors);
        assert!(parsed.ends_at_clean_frame_end);
    }

    #[test]
    fn rejects_nonconsecutive_sequence_even_when_monotonic_or_wrapped() {
        for second in [3, u64::MAX] {
            let mut raw = Vec::new();
            push_record(
                &mut raw,
                1,
                81,
                10,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
                0,
                &[],
            );
            push_frame_end(&mut raw, second, 81);

            let parsed = parse_project_frames(&raw).expect("framing result");
            assert!(!parsed.ends_at_clean_frame_end);
            assert!(
                parsed
                    .parse_errors
                    .iter()
                    .any(|error| error.contains(&format!("sequence discontinuity 1 -> {second}"))),
                "{:?}",
                parsed.parse_errors
            );
        }
    }

    #[test]
    fn rejects_exact_domain_data_only_ending() {
        let mut raw = Vec::new();
        push_record(
            &mut raw,
            1,
            90,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &[],
        );

        let parsed = parse_project_frames(&raw).expect("framing result");
        assert!(!parsed.ends_at_clean_frame_end);
        assert!(
            parsed
                .parse_errors
                .iter()
                .any(|error| error.contains("left open at EOF")),
            "{:?}",
            parsed.parse_errors
        );
    }

    #[test]
    fn rejects_records_after_the_last_frame_end() {
        let mut raw = Vec::new();
        let payload = projects_payload(0, 4, &[(10, 20, 0)], &[(1, 30, 5, 0)]);
        let mut sequence = 1;
        push_projects_frame(&mut raw, &mut sequence, 100, &payload);
        push_record(
            &mut raw,
            sequence,
            101,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &[0; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN],
        );

        let parsed = parse_project_frames(&raw).expect("framing result");
        assert!(!parsed.ends_at_clean_frame_end);
        assert!(
            parsed
                .parse_errors
                .iter()
                .any(|error| error.contains("left open at EOF")),
            "{:?}",
            parsed.parse_errors
        );
    }

    #[test]
    fn truncated_tail_invalidates_exported_clean_eof_boundary() {
        let mut raw = Vec::new();
        let payload = projects_payload(0, 4, &[(10, 20, 0)], &[]);
        let mut sequence = 1;
        push_projects_frame(&mut raw, &mut sequence, 100, &payload);
        raw.extend_from_slice(b"tail");

        let parsed = parse_project_frames(&raw).expect("framing result");
        assert!(!parsed.ends_at_clean_frame_end);
        assert!(parsed
            .parse_errors
            .iter()
            .any(|error| error.contains("truncated record header")));
    }

    #[test]
    fn rejects_malformed_frame_end_boundary() {
        for (domain_id, payload_len, expected) in [
            (41, 32, "FRAME_END domain 41"),
            (DOMAIN_FRAME, 31, "FRAME_END payload length 31"),
        ] {
            let mut raw = Vec::new();
            push_record(
                &mut raw,
                1,
                101,
                domain_id,
                compact_contract::COMPACT_RECORD_TYPE_FRAME_END,
                0,
                &vec![0; payload_len],
            );

            let parsed = parse_project_frames(&raw).expect("framing result");
            assert!(!parsed.ends_at_clean_frame_end);
            assert!(
                parsed
                    .parse_errors
                    .iter()
                    .any(|error| error.contains(expected)),
                "{:?}",
                parsed.parse_errors
            );
        }
    }

    #[test]
    fn rejects_non_authoritative_frame_end_payload_fields() {
        for (emitted, scheduled, status, reserved, expected) in [
            (1, 1, 1 << 31, 0, "contains unknown bits"),
            (
                1,
                1,
                compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
                0,
                "contains fatal bits",
            ),
            (1, 1, 0, 7, "reserved word is nonzero"),
            (1, 2, 0, 0, "emitted_domains 1 != scheduled_domains 2"),
            (
                2,
                2,
                0,
                0,
                "independently observed owned DOMAIN_PERF roster count 1",
            ),
            (
                0,
                0,
                0,
                0,
                "independently observed owned DOMAIN_PERF roster count 1",
            ),
        ] {
            let mut raw = Vec::new();
            let payload = projects_payload(0, 4, &[(10, 20, 0)], &[]);
            let perf = perf_base_payload(1, 1, 0);
            push_record(
                &mut raw,
                1,
                105,
                DOMAIN_PROJECTS_LIVE,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
                0,
                &perf,
            );
            push_record(
                &mut raw,
                2,
                105,
                DOMAIN_PROJECTS_LIVE,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
                0,
                &payload,
            );
            push_frame_end_with(&mut raw, 3, 105, emitted, scheduled, status, reserved);

            let parsed = parse_project_frames(&raw).expect("framing result");
            assert!(!parsed.ends_at_clean_frame_end);
            assert!(
                parsed
                    .parse_errors
                    .iter()
                    .any(|error| error.contains(expected)),
                "expected {expected:?} in {:?}",
                parsed.parse_errors
            );
        }
    }

    #[test]
    fn rejects_projects_data_before_its_perf_authority() {
        let mut raw = Vec::new();
        let payload = projects_payload(0, 4, &[(10, 20, 0)], &[]);
        push_record(
            &mut raw,
            1,
            106,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        push_record(
            &mut raw,
            2,
            106,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_base_payload(1, 1, 0),
        );
        push_frame_end(&mut raw, 3, 106);

        let parsed = parse_project_frames(&raw).expect("framing result");
        assert!(!parsed.ends_at_clean_frame_end);
        assert!(
            parsed
                .parse_errors
                .iter()
                .any(|error| error.contains("DOMAIN_DATA precedes its DOMAIN_PERF authority")),
            "{:?}",
            parsed.parse_errors
        );
    }

    #[test]
    fn rejects_frame_end_status_not_equal_to_owned_perf_status_or() {
        let mut raw = Vec::new();
        let payload = projects_payload(0, 4, &[(10, 20, 0)], &[]);
        push_record(
            &mut raw,
            1,
            107,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_base_payload(0, 0, compact_contract::COMPACT_STATUS_EXTRACTION_PERF_GATE),
        );
        push_record(
            &mut raw,
            2,
            107,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_base_payload(1, 1, 0),
        );
        push_record(
            &mut raw,
            3,
            107,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        push_frame_end_with(&mut raw, 4, 107, 2, 2, 0, 0);

        let parsed = parse_project_frames(&raw).expect("framing result");
        assert!(!parsed.ends_at_clean_frame_end);
        assert!(
            parsed.parse_errors.iter().any(|error| error
                .contains("!= OR of independently observed owned DOMAIN_PERF statuses")),
            "{:?}",
            parsed.parse_errors
        );
    }

    #[test]
    fn accepts_frame_end_status_equal_to_owned_perf_status_or() {
        let mut raw = Vec::new();
        let payload = projects_payload(0, 4, &[(10, 20, 0)], &[]);
        let informational_status = compact_contract::COMPACT_STATUS_EXTRACTION_PERF_GATE;
        push_record(
            &mut raw,
            1,
            108,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_base_payload(0, 0, informational_status),
        );
        push_record(
            &mut raw,
            2,
            108,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_base_payload(1, 1, 0),
        );
        push_record(
            &mut raw,
            3,
            108,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        push_frame_end_with(&mut raw, 4, 108, 2, 2, informational_status, 0);

        let parsed = parse_project_frames(&raw).expect("framing result");
        assert!(parsed.parse_errors.is_empty(), "{:?}", parsed.parse_errors);
        assert!(parsed.ends_at_clean_frame_end);
        assert_eq!(parsed.frames.len(), 1);
    }

    #[test]
    fn rejects_duplicate_owned_perf_even_when_roster_count_would_deduplicate() {
        let mut raw = Vec::new();
        let perf = perf_base_payload(0, 0, 0);
        push_record(
            &mut raw,
            1,
            109,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf,
        );
        push_record(
            &mut raw,
            2,
            109,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf,
        );

        let parsed = parse_project_frames(&raw).expect("framing result");
        assert!(!parsed.ends_at_clean_frame_end);
        assert!(
            parsed
                .parse_errors
                .iter()
                .any(|error| error.contains("duplicate owned DOMAIN_PERF for domain 10")),
            "{:?}",
            parsed.parse_errors
        );
    }

    #[test]
    fn rejects_unknown_or_contextually_invalid_record_flag_bits() {
        for (domain_id, record_type, record_flags) in [
            (
                DOMAIN_PROJECTS_LIVE,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
                compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            ),
            (
                10,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
                1 << 29,
            ),
            (
                DOMAIN_FRAME,
                compact_contract::COMPACT_RECORD_TYPE_FRAME_END,
                compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            ),
        ] {
            let mut raw = Vec::new();
            push_record(
                &mut raw,
                1,
                110,
                domain_id,
                record_type,
                record_flags,
                &[0; 32],
            );

            let parsed = parse_project_frames(&raw).expect("framing result");
            assert!(!parsed.ends_at_clean_frame_end);
            assert!(
                parsed
                    .parse_errors
                    .iter()
                    .any(|error| error.contains("contain rejected bits")),
                "{:?}",
                parsed.parse_errors
            );
        }
    }

    #[test]
    fn rejects_mismatched_frame_end_hour() {
        let mut raw = Vec::new();
        push_record(
            &mut raw,
            1,
            120,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &[],
        );
        push_frame_end(&mut raw, 2, 121);

        let parsed = parse_project_frames(&raw).expect("framing result");
        assert!(!parsed.ends_at_clean_frame_end);
        assert!(
            parsed
                .parse_errors
                .iter()
                .any(|error| error.contains("FRAME_END for hour 121 while hour 120 is open")),
            "{:?}",
            parsed.parse_errors
        );
    }

    #[test]
    fn applies_delta_to_baseline_before_deriving_events() {
        let mut raw = Vec::new();
        let mut sequence = 1;
        let baseline = projects_payload(0, 4, &[(10, 20, 0)], &[(1, 30, 5, 0)]);
        let delta = projects_payload(
            compact_contract::COMPACT_STATUS_DELTA_DATA,
            4,
            &[(10, 20, PROJECT_STATE_FLAG_COMPLETED)],
            &[],
        );
        push_projects_frame(&mut raw, &mut sequence, 200, &baseline);
        push_projects_frame(&mut raw, &mut sequence, 201, &delta);

        let parsed = parse_project_frames(&raw).unwrap();
        assert!(parsed.parse_errors.is_empty(), "{:?}", parsed.parse_errors);
        assert_eq!(parsed.frames.len(), 2);
        assert_eq!(parsed.frames[1].frame.breakthrough_rows.len(), 1);
        let events = derive_lifecycle_events(&parsed.frames[0].frame, &parsed.frames[1].frame);
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == PROJECT_EVENT_KIND_COMPLETION_CHANGED)
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == PROJECT_EVENT_KIND_BREAKTHROUGH_DELTA)
                .count(),
            0
        );
    }

    #[test]
    fn rejects_delta_before_baseline() {
        let mut raw = Vec::new();
        let mut sequence = 1;
        let delta = projects_payload(
            compact_contract::COMPACT_STATUS_DELTA_DATA,
            4,
            &[(10, 20, 0)],
            &[],
        );
        push_projects_frame(&mut raw, &mut sequence, 210, &delta);
        let parsed = parse_project_frames(&raw).unwrap();
        assert!(parsed
            .parse_errors
            .iter()
            .any(|error| error.contains("delta precedes a full baseline")));
    }

    #[test]
    fn tombstones_delete_logical_state_instead_of_becoming_live_rows() {
        let mut raw = Vec::new();
        let mut sequence = 1;
        let baseline = projects_payload(0, 4, &[(10, 20, 0)], &[(1, 30, 5, 0)]);
        let mut delta = projects_payload(
            compact_contract::COMPACT_STATUS_DELTA_DATA,
            4,
            &[(10, 0, ROW_FLAG_DELETED)],
            &[(1, 30, 0, ROW_FLAG_DELETED)],
        );
        delta[156..196].fill(0);
        push_projects_frame(&mut raw, &mut sequence, 220, &baseline);
        push_projects_frame(&mut raw, &mut sequence, 221, &delta);
        let parsed = parse_project_frames(&raw).unwrap();
        assert!(parsed.parse_errors.is_empty(), "{:?}", parsed.parse_errors);
        assert!(parsed.frames[1].frame.state_rows.is_empty());
        assert!(parsed.frames[1].frame.breakthrough_rows.is_empty());
    }

    #[test]
    fn v3_completion_bit_is_not_treated_as_v4_authority() {
        let mut raw = Vec::new();
        let mut sequence = 1;
        let invalid_v3 = projects_payload(0, 3, &[(10, 20, PROJECT_STATE_FLAG_COMPLETED)], &[]);
        push_projects_frame(&mut raw, &mut sequence, 230, &invalid_v3);
        let parsed = parse_project_frames(&raw).unwrap();
        assert!(parsed
            .parse_errors
            .iter()
            .any(|error| error.contains("v3 state row") && error.contains("invalid flags")));
    }

    #[test]
    fn rejects_duplicate_or_missing_d41_records_per_closed_hour() {
        let payload = projects_payload(0, 4, &[(10, 20, 0)], &[]);

        let mut duplicate = Vec::new();
        let perf = perf_base_payload(1, 1, 0);
        push_record(
            &mut duplicate,
            1,
            240,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf,
        );
        push_record(
            &mut duplicate,
            2,
            240,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        push_record(
            &mut duplicate,
            3,
            240,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        push_frame_end(&mut duplicate, 4, 240);
        assert!(parse_project_frames(&duplicate)
            .unwrap()
            .parse_errors
            .iter()
            .any(|error| error.contains("duplicate projects_live DOMAIN_DATA")));

        let mut missing_perf = Vec::new();
        push_record(
            &mut missing_perf,
            1,
            241,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        push_frame_end(&mut missing_perf, 2, 241);
        assert!(parse_project_frames(&missing_perf)
            .unwrap()
            .parse_errors
            .iter()
            .any(|error| error.contains("DOMAIN_DATA precedes its DOMAIN_PERF authority")));
    }

    #[test]
    fn rejects_nonconsecutive_closed_hours_and_perf_data_mismatch() {
        let baseline = projects_payload(0, 4, &[(10, 20, 0)], &[]);
        let mut raw = Vec::new();
        let mut sequence = 1;
        push_projects_frame(&mut raw, &mut sequence, 250, &baseline);
        push_projects_frame(&mut raw, &mut sequence, 252, &baseline);
        assert!(parse_project_frames(&raw)
            .unwrap()
            .parse_errors
            .iter()
            .any(|error| error.contains("expected consecutive hour 251")));

        let mut mismatch = Vec::new();
        let perf = perf_base_payload(99, 1, 0);
        push_record(
            &mut mismatch,
            1,
            260,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf,
        );
        push_record(
            &mut mismatch,
            2,
            260,
            DOMAIN_PROJECTS_LIVE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &baseline,
        );
        push_frame_end(&mut mismatch, 3, 260);
        assert!(parse_project_frames(&mismatch)
            .unwrap()
            .parse_errors
            .iter()
            .any(|error| error.contains("PERF/DATA authority mismatch")));
    }

    #[test]
    fn rejects_non_authoritative_d41_status() {
        let payload = projects_payload(
            compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
            4,
            &[(10, 20, 0)],
            &[],
        );
        let mut raw = Vec::new();
        let mut sequence = 1;
        push_projects_frame(&mut raw, &mut sequence, 270, &payload);
        assert!(parse_project_frames(&raw)
            .unwrap()
            .parse_errors
            .iter()
            .any(|error| error.contains("non-authoritative failure status")));
    }

    #[test]
    fn sha256_matches_standard_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn stream_artifact_transparently_decodes_raw_and_zstd() {
        use std::io::Write as _;

        let bytes = b"H4CR canonical decoded bytes";
        let mut raw_path = std::env::temp_dir();
        raw_path.push(format!(
            "project_lifecycle_raw_{}_{}.bin",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut zstd_path = raw_path.clone();
        zstd_path.set_extension("zst");
        fs::write(&raw_path, bytes).unwrap();
        let mut writer = compact_stream::StreamWriter::create(
            &zstd_path,
            compact_stream::StreamCompression::Zstd,
        )
        .unwrap();
        writer.write_all(bytes).unwrap();
        writer.finish().unwrap();

        let raw = read_stream_artifact(raw_path.to_str().unwrap()).unwrap();
        let zstd = read_stream_artifact(zstd_path.to_str().unwrap()).unwrap();
        assert_eq!(raw.compression, "none");
        assert_eq!(zstd.compression, "zstd");
        assert_eq!(raw.decoded, bytes);
        assert_eq!(zstd.decoded, bytes);
        assert_eq!(raw.decoded_sha256, zstd.decoded_sha256);
        assert_ne!(raw.stored_sha256, zstd.stored_sha256);

        let _ = fs::remove_file(raw_path);
        let _ = fs::remove_file(zstd_path);
    }
}
