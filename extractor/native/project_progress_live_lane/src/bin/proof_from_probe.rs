use std::env;
use std::fs;
use std::path::Path;

use project_progress_live_lane::{
    encode_lane_frame, gate_lane_frame, LaneFrame, ProjectBreakthroughRow, ProjectStateRow,
    PROJECT_STATE_FLAG_COMPLETED, PROJECT_STATE_MACHINE_OFF_PROTOTYPE_STATE,
};
use serde_json::{json, Value};

fn main() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let probe_path = args.next().ok_or_else(|| {
        "usage: project_progress_lane_proof <probe.json> <proof.json> <compact.bin>".to_string()
    })?;
    let proof_path = args.next().ok_or_else(|| {
        "usage: project_progress_lane_proof <probe.json> <proof.json> <compact.bin>".to_string()
    })?;
    let compact_path = args.next().ok_or_else(|| {
        "usage: project_progress_lane_proof <probe.json> <proof.json> <compact.bin>".to_string()
    })?;
    if args.next().is_some() {
        return Err("too many arguments".to_string());
    }

    let probe_text = fs::read_to_string(&probe_path)
        .map_err(|err| format!("failed to read {probe_path}: {err}"))?;
    let probe: Value = serde_json::from_str(&probe_text)
        .map_err(|err| format!("failed to parse {probe_path}: {err}"))?;
    let results = probe
        .get("candidate_results")
        .ok_or_else(|| "probe missing candidate_results".to_string())?;
    let counts = probe
        .get("counts")
        .ok_or_else(|| "probe missing counts".to_string())?;

    let sampled_projects = counts_u64(counts, "sampled_projects")?;
    let state_machine_hits = nested_u64(results, &["CProject_plus_0x28_pointer", "hits"])?;
    let rejected_0x170_hits = nested_u64(results, &["CProject_plus_0x170_direct", "hits"])?;
    let current_selector_valid = counts_u64(counts, "current_state_selector_valid")?;
    let current_selector_invalid = counts_u64(counts, "current_state_selector_invalid")?;
    let breakthrough_rows = counts_u64(counts, "breakthrough_rows_total")?;
    let breakthrough_failures = counts_u64(counts, "breakthrough_parse_failures")?;
    let progress_nonzero = counts_u64(counts, "prototype_phase_nonzero")?;
    let state_selector_ok = state_machine_hits == sampled_projects
        && current_selector_valid == sampled_projects
        && current_selector_invalid == 0
        && sampled_projects > 0;
    let breakthrough_ok = breakthrough_rows > 0 && breakthrough_failures == 0;
    let rejected_0x170 = rejected_0x170_hits == 0;
    let raw_progress_ok = progress_nonzero > 0;

    let frame = representative_frame_from_probe(results)?;
    let compact = encode_lane_frame(&frame);
    let gate = gate_lane_frame(&compact);
    fs::write(&compact_path, &compact)
        .map_err(|err| format!("failed to write {compact_path}: {err}"))?;

    let accepted =
        state_selector_ok && breakthrough_ok && rejected_0x170 && raw_progress_ok && gate.ok;
    let proof = json!({
        "schema": "project_progress_live_lane_proof_v1",
        "source_probe": path_slash(&probe_path),
        "compact_bin": path_slash(&compact_path),
        "accepted_for_integration_patch": accepted,
        "production_complete": false,
        "completion_note": "lane-local proof only; shared collector/recorder integration and .battleplan speed-five proof still required",
        "validated_roots": {
            "CProject_plus_0x28_state_machine_pointer": {
                "hits": state_machine_hits,
                "sampled_projects": sampled_projects,
                "accepted": state_machine_hits == sampled_projects && sampled_projects > 0
            },
            "CProject_plus_0x170_direct_state_machine": {
                "hits": rejected_0x170_hits,
                "rejected": rejected_0x170
            },
            "CProjectStateMachine_plus_0x8_current_state_selector": {
                "valid": current_selector_valid,
                "invalid": current_selector_invalid,
                "accepted": state_selector_ok
            },
            "CProgramStatus_plus_0x50_embedded_CBreakthroughProgress": {
                "rows_total": breakthrough_rows,
                "parse_failures": breakthrough_failures,
                "accepted": breakthrough_ok
            },
            "CPrototypeProjectState_raw_progress_fields": {
                "path": "CProject+0x28 -> CProjectStateMachine+0x18",
                "phase_progress_raw_offset": "0x48",
                "phase_target_raw_offset": "0x50",
                "iterations_offset": "0x8C",
                "nonzero_phase_raw_projects": progress_nonzero,
                "accepted_as_raw_fields": raw_progress_ok
            }
        },
        "lane_compact_gate": {
            "ok": gate.ok,
            "state_rows": gate.state_rows,
            "breakthrough_rows": gate.breakthrough_rows,
            "status_flags": gate.status_flags,
            "errors": gate.errors
        },
        "representative_records": {
            "state_rows": frame.state_rows.len(),
            "breakthrough_rows": frame.breakthrough_rows.len(),
            "semantic_note": "phase_progress_raw, phase_target_raw, and prototype_iterations are emitted as raw/badged fields only"
        }
    });
    fs::write(
        &proof_path,
        serde_json::to_string_pretty(&proof).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("failed to write {proof_path}: {err}"))?;
    if !accepted {
        return Err(format!("lane proof failed; wrote {proof_path}"));
    }
    Ok(())
}

fn representative_frame_from_probe(results: &Value) -> Result<LaneFrame, String> {
    let mut state_rows = Vec::new();
    let hits = results
        .get("CProjectStateMachine_plus_0x8_current_state_selector")
        .and_then(|v| v.get("hits"))
        .and_then(Value::as_array)
        .ok_or_else(|| "probe missing current-state selector hits".to_string())?;
    for (idx, hit) in hits.iter().enumerate() {
        let offset = parse_hex_str(
            hit.get("offset")
                .and_then(Value::as_str)
                .ok_or_else(|| "selector hit missing offset".to_string())?,
        )?;
        let enum_value = hit
            .get("current_state_enum")
            .and_then(Value::as_u64)
            .ok_or_else(|| "selector hit missing current_state_enum".to_string())?
            as u32;
        state_rows.push(ProjectStateRow {
            id: 90_000 + idx as u32,
            static_def_handle: 6_000 + idx as u32,
            current_state_offset: offset as u32,
            current_state_enum: enum_value,
            prototype_state_enum: 0,
            simple_state_1_enum: 1,
            simple_state_2_enum: 2,
            simple_state_3_enum: 3,
            prototype_iterations: idx as u32,
            phase_progress_raw: if offset == PROJECT_STATE_MACHINE_OFF_PROTOTYPE_STATE {
                1
            } else {
                0
            },
            phase_target_raw: 60,
            state_flags: if enum_value == 1 {
                PROJECT_STATE_FLAG_COMPLETED
            } else {
                0
            },
        });
    }
    if state_rows.is_empty() {
        return Err("probe had no selector hits".to_string());
    }

    let mut breakthrough_rows = Vec::new();
    let handle_counts = results
        .get("CProgramStatus_plus_0x50_breakthrough_rows")
        .and_then(|v| v.get("specialization_handle_counts"))
        .and_then(Value::as_object)
        .ok_or_else(|| "probe missing breakthrough specialization counts".to_string())?;
    for (idx, handle) in handle_counts.keys().enumerate() {
        breakthrough_rows.push(ProjectBreakthroughRow {
            tag_db_id: 1 + idx as u32,
            specialization_handle: handle
                .parse::<u32>()
                .map_err(|err| format!("invalid specialization handle {handle}: {err}"))?,
            points_raw: 0,
        });
    }
    if breakthrough_rows.is_empty() {
        return Err("probe had no breakthrough handles".to_string());
    }

    Ok(LaneFrame {
        game_hour_raw: 0,
        status_flags: 0,
        state_rows,
        breakthrough_rows,
    })
}

fn counts_u64(counts: &Value, key: &str) -> Result<u64, String> {
    counts
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("probe counts missing numeric {key}"))
}

fn nested_u64(value: &Value, path: &[&str]) -> Result<u64, String> {
    let mut current = value;
    for key in path {
        current = current
            .get(*key)
            .ok_or_else(|| format!("probe missing {}", path.join(".")))?;
    }
    current
        .as_u64()
        .ok_or_else(|| format!("probe field {} is not numeric", path.join(".")))
}

fn parse_hex_str(value: &str) -> Result<usize, String> {
    usize::from_str_radix(value.trim_start_matches("0x"), 16)
        .map_err(|err| format!("invalid hex {value}: {err}"))
}

fn path_slash(path: &str) -> String {
    Path::new(path).to_string_lossy().replace('\\', "/")
}
