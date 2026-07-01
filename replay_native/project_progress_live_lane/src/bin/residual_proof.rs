use std::env;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

fn main() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let residual_path = args.next().ok_or_else(|| {
        "usage: project_progress_residual_proof <residual_probe.json> <proof.json>".to_string()
    })?;
    let proof_path = args.next().ok_or_else(|| {
        "usage: project_progress_residual_proof <residual_probe.json> <proof.json>".to_string()
    })?;
    if args.next().is_some() {
        return Err("too many arguments".to_string());
    }

    let text = fs::read_to_string(&residual_path)
        .map_err(|err| format!("failed to read {residual_path}: {err}"))?;
    let probe: Value = serde_json::from_str(&text)
        .map_err(|err| format!("failed to parse {residual_path}: {err}"))?;
    require_str(&probe, "schema", "project_progress_live_residual_probe_v1")?;

    let counts = obj(&probe, "counts")?;
    let sampled_projects = number(counts, "sampled_projects")?;
    let state_machine_invalid = number(counts, "state_machine_invalid")?;
    let current_state_invalid = number(counts, "current_state_invalid")?;
    let worked_proxy_projects = number(counts, "worked_proxy_projects")?;
    let completed_selector_proxy_projects = number(counts, "completed_selector_proxy_projects")?;
    let validated_facilities = number(counts, "validated_facilities")?;

    let reward = obj(&probe, "reward_history")?;
    let rtti_vtables = reward
        .get("rtti_vtables")
        .and_then(Value::as_array)
        .ok_or_else(|| "reward_history.rtti_vtables missing array".to_string())?;
    let reward_objects = number(reward, "selected_reward_object_count")?;
    let anchor_scan = obj(reward, "anchor_scan")?;
    let direct_hits = number(anchor_scan, "direct_hit_count")?;
    let one_hop_hits = number(anchor_scan, "one_hop_hit_count")?;
    let accepted_reward_fields = bool_value(reward, "accepted_reward_history_fields")?;
    let reward_blocker = bool_value(reward, "bounded_blocker")?;
    let save_compare = obj(&probe, "save_oracle_comparison")?;
    let save_matched_projects = number(save_compare, "matched_by_tag_and_project_id")?;
    let start_proxy_mismatches = number(save_compare, "start_proxy_mismatches")?;
    let completed_proxy_mismatches = number(save_compare, "completed_proxy_mismatches")?;

    let bool_scan = obj(&probe, "start_completed_boolean_scan")?;
    let readable_state_machines = number(bool_scan, "readable_state_machines")?;
    let save_bool_scan = obj(&probe, "start_completed_save_truth_scan")?;
    let save_bool_readable = number(save_bool_scan, "readable_state_machines")?;
    let accepted_start_completed =
        bool_value(save_bool_scan, "accepted_exact_start_completed_fields")?;
    let start_completed_rejected = bool_value(save_bool_scan, "bounded_rejection")?;
    let semantic_truth_available = bool_value(save_bool_scan, "semantic_truth_available")?;
    let save_start_u8 = array_len_path(
        save_bool_scan,
        &["perfect_proxy_separators_u8", "save_start"],
    )?;
    let save_start_u32 = array_len_path(
        save_bool_scan,
        &["perfect_proxy_separators_u32", "save_start"],
    )?;
    let save_completed_u8 = array_len_path(
        save_bool_scan,
        &["perfect_proxy_separators_u8", "save_completed"],
    )?;
    let save_completed_u32 = array_len_path(
        save_bool_scan,
        &["perfect_proxy_separators_u32", "save_completed"],
    )?;

    let existing_rows_safe = sampled_projects > 0
        && state_machine_invalid == 0
        && current_state_invalid == 0
        && worked_proxy_projects > 0
        && completed_selector_proxy_projects > 0
        && readable_state_machines == sampled_projects;
    let reward_history_rooted_but_blocked = !rtti_vtables.is_empty()
        && reward_objects > 0
        && one_hop_hits > 0
        && !accepted_reward_fields
        && reward_blocker;
    let completed_validated = semantic_truth_available
        && save_bool_readable == sampled_projects
        && save_completed_u8 == 1
        && save_completed_u32 == 0
        && completed_proxy_mismatches == 0;
    let start_rejected = semantic_truth_available
        && save_matched_projects == sampled_projects
        && start_proxy_mismatches > 0
        && save_start_u8 == 0
        && save_start_u32 == 0
        && !accepted_start_completed
        && start_completed_rejected;
    let accepted = existing_rows_safe
        && reward_history_rooted_but_blocked
        && completed_validated
        && start_rejected
        && validated_facilities > 0;

    let proof = json!({
        "schema": "project_progress_live_residual_proof_v1",
        "source_probe": path_slash(&residual_path),
        "accepted_residual_blocker": accepted,
        "production_schema_change": true,
        "production_complete": false,
        "existing_project_state_progress_breakthrough_rows_remain_safe": existing_rows_safe,
        "fired_reward_history_root_detected_but_fields_blocked": reward_history_rooted_but_blocked,
        "completed_boolean_validated_for_schema_expansion": completed_validated,
        "start_boolean_rejected_for_schema_expansion": start_rejected,
        "bounded_evidence": {
            "sampled_projects": sampled_projects,
            "save_matched_projects": save_matched_projects,
            "state_machine_invalid": state_machine_invalid,
            "current_state_invalid": current_state_invalid,
            "worked_proxy_projects": worked_proxy_projects,
            "completed_selector_proxy_projects": completed_selector_proxy_projects,
            "validated_facilities": validated_facilities,
            "reward_rtti_vtables": rtti_vtables.len(),
            "reward_objects": reward_objects,
            "reward_anchor_direct_hits": direct_hits,
            "reward_anchor_one_hop_hits": one_hop_hits,
            "state_machine_bool_scan_readable": readable_state_machines,
            "save_bool_scan_readable": save_bool_readable,
            "semantic_start_completed_truth_available": semantic_truth_available,
            "save_completed_u8_separators": save_completed_u8,
            "save_start_u8_separators": save_start_u8,
            "start_proxy_mismatches": start_proxy_mismatches,
            "completed_proxy_mismatches": completed_proxy_mismatches
        },
        "decision": "Add only the completed bit from CProjectStateMachine+0x10==0; do not add start or fired reward-history fields from this fixture."
    });
    fs::write(
        &proof_path,
        serde_json::to_string_pretty(&proof).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("failed to write {proof_path}: {err}"))?;
    if !accepted {
        return Err(format!("residual proof failed; wrote {proof_path}"));
    }
    Ok(())
}

fn obj<'a>(value: &'a Value, key: &str) -> Result<&'a Value, String> {
    value
        .get(key)
        .ok_or_else(|| format!("missing object {key}"))
}

fn require_str(value: &Value, key: &str, expected: &str) -> Result<(), String> {
    let actual = value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string {key}"))?;
    if actual != expected {
        return Err(format!("{key} {actual:?} != {expected:?}"));
    }
    Ok(())
}

fn number(value: &Value, key: &str) -> Result<u64, String> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing numeric {key}"))
}

fn bool_value(value: &Value, key: &str) -> Result<bool, String> {
    value
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("missing bool {key}"))
}

fn array_len_path(value: &Value, path: &[&str]) -> Result<u64, String> {
    let mut current = value;
    for key in path {
        current = current
            .get(*key)
            .ok_or_else(|| format!("missing {}", path.join(".")))?;
    }
    let len = current
        .as_array()
        .ok_or_else(|| format!("{} is not an array", path.join(".")))?
        .len();
    Ok(len as u64)
}

fn path_slash(path: &str) -> String {
    Path::new(path).to_string_lossy().replace('\\', "/")
}
