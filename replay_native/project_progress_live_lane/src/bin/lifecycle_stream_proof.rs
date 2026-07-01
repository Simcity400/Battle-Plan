use std::env;
use std::fs;
use std::path::Path;

use project_progress_live_lane::{
    decode_lifecycle_events, derive_lifecycle_events, encode_lifecycle_events,
    gate_lifecycle_events, LaneFrame, ProjectBreakthroughRow, ProjectLifecycleEvent,
    ProjectStateRow,
    PROJECT_BREAKTHROUGH_ROW_LEN, PROJECT_EVENT_KIND_BREAKTHROUGH_DELTA,
    PROJECT_EVENT_KIND_COMPLETION_CHANGED, PROJECT_STATE_ROW_LEN,
};
use serde_json::{json, Value};

const RECORD_MAGIC: &[u8; 4] = b"H4CR";
const RECORD_VERSION: u16 = 1;
const RECORD_HEADER_LEN: usize = 32;
const RECORD_TYPE_DOMAIN_DATA: u16 = 3;
const DOMAIN_PROJECTS_LIVE: u16 = 41;

const COMPACT_PROJECT_HEADER_LEN: usize = 64;
const COMPACT_PROJECT_HEADER_EXT_PROGRESS_V1: u32 = 3;
const COMPACT_PROJECT_HEADER_EXT_PROGRESS_V2: u32 = 4;
const COMPACT_PROJECT_ROOT_COUNTERS_LEN: usize = 24;
const COMPACT_PROJECT_ROOT_REASON_COUNTERS_LEN: usize = 48;
const COMPACT_PROJECT_PROGRESS_COUNTERS_LEN: usize = 16;
const COMPACT_PROJECT_FACILITY_ROW_LEN: usize = 32;
const COMPACT_PROJECT_ROW_LEN: usize = 24;
const COMPACT_PROJECT_SCIENTIST_ROW_LEN: usize = 32;

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

    let raw =
        fs::read(&stream_path).map_err(|err| format!("failed to read {stream_path}: {err}"))?;
    let parsed = parse_project_frames(&raw)?;
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
        && transition_count > 0
        && !all_events.is_empty();
    fs::write(&event_path, &encoded)
        .map_err(|err| format!("failed to write {event_path}: {err}"))?;

    let proof = json!({
        "schema": "project_progress_lifecycle_stream_proof_v1",
        "source_stream": path_slash(&stream_path),
        "event_compact_bin": path_slash(&event_path),
        "domain_id": DOMAIN_PROJECTS_LIVE,
        "record_count": parsed.record_count,
        "project_data_records": parsed.frames.len(),
        "transition_count": transition_count,
        "parse_errors": parsed.parse_errors,
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
    frames: Vec<ProjectFrame>,
    parse_errors: Vec<String>,
}

struct ProjectFrame {
    game_hour_raw: i32,
    frame: LaneFrame,
}

fn parse_project_frames(raw: &[u8]) -> Result<ParsedStream, String> {
    let mut offset = 0usize;
    let mut record_count = 0usize;
    let mut frames = Vec::new();
    let mut parse_errors = Vec::new();
    while offset < raw.len() {
        let remaining = raw.len().saturating_sub(offset);
        if remaining < RECORD_HEADER_LEN {
            parse_errors.push(format!(
                "offset {offset}: truncated compact record header with {remaining} byte(s)"
            ));
            break;
        }
        let header = &raw[offset..offset + RECORD_HEADER_LEN];
        if &header[0..4] != RECORD_MAGIC {
            parse_errors.push(format!("offset {offset}: compact record magic mismatch"));
            break;
        }
        let version = read_u16_le(header, 4);
        let header_len = read_u16_le(header, 6) as usize;
        let game_hour_raw = read_i32_le(header, 16);
        let domain_id = read_u16_le(header, 20);
        let record_type = read_u16_le(header, 22);
        let payload_len = read_u32_le(header, 24) as usize;
        if version != RECORD_VERSION {
            parse_errors.push(format!(
                "offset {offset}: version {version} != {RECORD_VERSION}"
            ));
            break;
        }
        if header_len != RECORD_HEADER_LEN {
            parse_errors.push(format!(
                "offset {offset}: header_len {header_len} != {RECORD_HEADER_LEN}"
            ));
            break;
        }
        let record_len = RECORD_HEADER_LEN.saturating_add(payload_len);
        if record_len > remaining {
            parse_errors.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining bytes {remaining}"
            ));
            break;
        }
        let payload = &raw[offset + RECORD_HEADER_LEN..offset + record_len];
        record_count += 1;
        if domain_id == DOMAIN_PROJECTS_LIVE && record_type == RECORD_TYPE_DOMAIN_DATA {
            match parse_projects_payload(game_hour_raw, payload) {
                Ok(frame) => frames.push(frame),
                Err(err) => parse_errors.push(format!("offset {offset}: {err}")),
            }
        }
        offset += record_len;
    }
    Ok(ParsedStream {
        record_count,
        frames,
        parse_errors,
    })
}

fn parse_projects_payload(game_hour_raw: i32, payload: &[u8]) -> Result<ProjectFrame, String> {
    if payload.len() < COMPACT_PROJECT_HEADER_LEN {
        return Err(format!("projects payload too short: {}", payload.len()));
    }
    let status_flags = read_u32_le(payload, 16);
    let facility_count = read_u32_le(payload, 20) as usize;
    let project_count = read_u32_le(payload, 24) as usize;
    let scientist_count = read_u32_le(payload, 28) as usize;
    let header_extension = read_u32_le(payload, 52);
    let mut project_header_len = COMPACT_PROJECT_HEADER_LEN;
    let mut project_state_count = 0usize;
    let mut breakthrough_count = 0usize;
    match header_extension {
        0 => {}
        1 | 2 | COMPACT_PROJECT_HEADER_EXT_PROGRESS_V1 | COMPACT_PROJECT_HEADER_EXT_PROGRESS_V2 => {
            project_header_len += COMPACT_PROJECT_ROOT_COUNTERS_LEN;
            if header_extension == 2
                || header_extension == COMPACT_PROJECT_HEADER_EXT_PROGRESS_V1
                || header_extension == COMPACT_PROJECT_HEADER_EXT_PROGRESS_V2
            {
                project_header_len += COMPACT_PROJECT_ROOT_REASON_COUNTERS_LEN;
            }
            if header_extension == COMPACT_PROJECT_HEADER_EXT_PROGRESS_V1
                || header_extension == COMPACT_PROJECT_HEADER_EXT_PROGRESS_V2
            {
                project_header_len += COMPACT_PROJECT_PROGRESS_COUNTERS_LEN;
                if payload.len() < project_header_len {
                    return Err(format!(
                        "projects payload too short for progress header: {} < {project_header_len}",
                        payload.len()
                    ));
                }
                project_state_count = read_u32_le(payload, 136) as usize;
                breakthrough_count = read_u32_le(payload, 140) as usize;
            }
        }
        other => return Err(format!("unknown projects header extension {other}")),
    }

    let state_start = project_header_len
        .saturating_add(facility_count.saturating_mul(COMPACT_PROJECT_FACILITY_ROW_LEN))
        .saturating_add(project_count.saturating_mul(COMPACT_PROJECT_ROW_LEN))
        .saturating_add(scientist_count.saturating_mul(COMPACT_PROJECT_SCIENTIST_ROW_LEN));
    let breakthrough_start =
        state_start.saturating_add(project_state_count.saturating_mul(PROJECT_STATE_ROW_LEN));
    let fixed_end = breakthrough_start
        .saturating_add(breakthrough_count.saturating_mul(PROJECT_BREAKTHROUGH_ROW_LEN));
    if payload.len() < fixed_end {
        return Err(format!(
            "projects fixed rows exceed payload: need {fixed_end}, have {}",
            payload.len()
        ));
    }

    let mut state_rows = Vec::with_capacity(project_state_count);
    let mut cursor = state_start;
    for _ in 0..project_state_count {
        state_rows.push(ProjectStateRow {
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
        });
        cursor += PROJECT_STATE_ROW_LEN;
    }

    let mut breakthrough_rows = Vec::with_capacity(breakthrough_count);
    cursor = breakthrough_start;
    for _ in 0..breakthrough_count {
        breakthrough_rows.push(ProjectBreakthroughRow {
            tag_db_id: read_u32_le(payload, cursor),
            specialization_handle: read_u32_le(payload, cursor + 4),
            points_raw: read_u32_le(payload, cursor + 8),
        });
        cursor += PROJECT_BREAKTHROUGH_ROW_LEN;
    }

    Ok(ProjectFrame {
        game_hour_raw,
        frame: LaneFrame {
            game_hour_raw,
            status_flags,
            state_rows,
            breakthrough_rows,
        },
    })
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

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_i32_le(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn path_slash(path: &str) -> String {
    Path::new(path).to_string_lossy().replace('\\', "/")
}
