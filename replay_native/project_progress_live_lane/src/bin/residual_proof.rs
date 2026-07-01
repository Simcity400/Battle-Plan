use std::env;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

fn main() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let residual_path = args.next().ok_or_else(|| {
        "usage: project_progress_residual_proof <residual_probe.json> <proof.json> [reward_container_shape_probe.json]".to_string()
    })?;
    let proof_path = args.next().ok_or_else(|| {
        "usage: project_progress_residual_proof <residual_probe.json> <proof.json> [reward_container_shape_probe.json]".to_string()
    })?;
    let reward_shape_path = args.next();
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
    let save_date = save_compare
        .get("save_date")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let save_projects = number(save_compare, "save_projects")?;
    let save_projects_missing_from_live = number(save_compare, "save_projects_missing_from_live")?;

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
    let reward_shape = match reward_shape_path.as_deref() {
        Some(path) => Some(load_reward_shape(path)?),
        None => None,
    };
    let status_vector = obj(reward, "status_plus_0x58_vector")?;
    let status_vector_invalid_headers = number(status_vector, "invalid_headers")?;
    let status_vector_entries = number(status_vector, "total_entries")?;
    let accepted = existing_rows_safe
        && reward_history_rooted_but_blocked
        && completed_validated
        && start_rejected
        && validated_facilities > 0;

    let proof = json!({
        "schema": "project_progress_live_residual_proof_v1",
        "source_probe": path_slash(&residual_path),
        "reward_container_shape_probe": reward_shape.as_ref().map(|shape| shape.source_probe.clone()),
        "accepted_residual_blocker": accepted,
        "missing_slice_production_complete": false,
        "missing_slice_registration_allowed": false,
        "production_schema_change": false,
        "production_complete": false,
        "existing_project_state_progress_breakthrough_rows_remain_safe": existing_rows_safe,
        "fired_reward_history_root_detected_but_fields_blocked": reward_history_rooted_but_blocked,
        "completed_boolean_validated_for_schema_expansion": completed_validated,
        "start_boolean_rejected_for_schema_expansion": start_rejected,
        "fixture": {
            "build_sha256": probe.get("sha256").cloned().unwrap_or(Value::Null),
            "module_base": probe.get("module_base").cloned().unwrap_or(Value::Null),
            "module_path": probe.get("module_path").cloned().unwrap_or(Value::Null),
            "save_date": save_date,
            "save_projects": save_projects,
            "matched_live_projects": save_matched_projects,
            "save_projects_missing_from_live": save_projects_missing_from_live
        },
        "start_semantics": {
            "accepted": false,
            "conclusive_rejection": start_rejected,
            "root": "CProject+0x28 -> CProjectStateMachine",
            "tested_window": save_bool_scan.get("window").cloned().unwrap_or(Value::Null),
            "tested_labels": ["save_start"],
            "candidate_widths": ["u8", "u32"],
            "save_start_true_rows": label_count(save_bool_scan, "save_start", "true"),
            "save_start_false_rows": label_count(save_bool_scan, "save_start", "false"),
            "u8_perfect_separators": save_start_u8,
            "u32_perfect_separators": save_start_u32,
            "rejected_proxy": {
                "name": "CProject+0x74 iterations != 0",
                "mismatches_against_save_start": start_proxy_mismatches
            },
            "duplicate_exclusions": [
                "CProject+0x74 iterations",
                "CProject+0x60 owner facility ref",
                "CProjectStateMachine+0x08 current-state selector",
                "CProjectStateMachine+0x10 inverse completed byte"
            ]
        },
        "completed_semantics": {
            "accepted": completed_validated,
            "root": "CProject+0x28 -> CProjectStateMachine",
            "offset": "0x10",
            "width": "u8",
            "encoding": "0=completed, 1=not_completed",
            "u8_perfect_separators": save_completed_u8,
            "u32_perfect_separators": save_completed_u32,
            "mismatches_against_save_completed": completed_proxy_mismatches
        },
        "reward_history": {
            "accepted": false,
            "conclusive_rejection": reward_shape
                .as_ref()
                .map(|shape| shape.rejected_status_plus_0x58_refs)
                .unwrap_or(false),
            "rtti": {
                "name": reward.get("rtti_name").cloned().unwrap_or(Value::Null),
                "vtables": reward.get("rtti_vtables").cloned().unwrap_or(Value::Null),
                "selected_heap_objects": reward_objects
            },
            "rejected_vtable_rvas": rejected_reward_vtable_rvas(reward),
            "rejected_containers": {
                "CProgramStatus+0x58_begin_end_cap_vector": {
                    "invalid_headers": status_vector_invalid_headers,
                    "decoded_entries": status_vector_entries,
                    "save_history_rows": status_vector
                        .get("save_compare")
                        .and_then(|v| v.get("save_total_history_entries"))
                        .cloned()
                        .unwrap_or(Value::Null),
                    "accepted": false
                },
                "CProgramStatus+0x58_pointer_payload_CPrototypeReward_refs": reward_shape
                    .as_ref()
                    .map(|shape| shape.summary.clone())
                    .unwrap_or(Value::Null)
            },
            "field_blocker": "No fired reward-history row has pinned tag/project, option/prototype token, date/hour, province, progress, and scientist context fields."
        },
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
        "blocker_contract": {
            "requires_controlled_start_transition": true,
            "requires_controlled_reward_option_transition": true,
            "start_transition": [
                "save before starting a never-started project",
                "start that exact project",
                "advance until live state updates",
                "save after start before completion",
                "candidate start field must flip false->true without duplicating iterations, owner facility, current-state selector, or +0x10"
            ],
            "reward_transition": [
                "save with unread_prototype_rewards > 0 before selecting option",
                "select a known reward option",
                "save immediately after the click",
                "accepted root must add exactly one fired row with pinned tag/project, option/prototype reward, date/hour, province, progress, and scientist context"
            ]
        },
        "decision": "For the missing start/reward slice: reject CProjectStateMachine[0..0x280] simple start fields and CProgramStatus+0x58 reward-reference payloads; keep start and fired reward-history omitted until the controlled transition fixture passes."
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

#[derive(Debug)]
struct RewardShapeSummary {
    source_probe: String,
    rejected_status_plus_0x58_refs: bool,
    summary: Value,
}

fn load_reward_shape(path: &str) -> Result<RewardShapeSummary, String> {
    let text = fs::read_to_string(path)
        .map_err(|err| format!("failed to read reward shape {path}: {err}"))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|err| format!("failed to parse reward shape {path}: {err}"))?;
    require_str(
        &value,
        "schema",
        "project_progress_reward_container_shape_probe_v1",
    )?;
    let accepted = bool_value(&value, "accepted_as_fired_reward_history_root")?;
    let save_compare = obj(&value, "save_compare")?;
    let live_ref_summary = obj(&value, "live_ref_summary")?;
    let summary = json!({
        "accepted": accepted,
        "decision": value.get("decision").cloned().unwrap_or(Value::Null),
        "scan_limit_bytes": value.get("scan_limit_bytes").cloned().unwrap_or(Value::Null),
        "live_total_refs": nested_value(save_compare, &["live_total_refs"]),
        "live_unique_reward_objects": nested_value(save_compare, &["live_unique_reward_objects"]),
        "save_history_total": nested_value(save_compare, &["save_history_total"]),
        "per_tag_ref_mismatches": nested_value(save_compare, &["per_tag_ref_mismatches"]),
        "per_tag_unique_mismatches": nested_value(save_compare, &["per_tag_unique_mismatches"]),
        "reused_reward_objects": nested_value(live_ref_summary, &["reused_reward_objects"]),
        "multi_tag_reward_objects": nested_value(live_ref_summary, &["multi_tag_reward_objects"]),
        "max_tags_per_reward_object": nested_value(live_ref_summary, &["max_tags_per_reward_object"])
    });
    Ok(RewardShapeSummary {
        source_probe: path_slash(path),
        rejected_status_plus_0x58_refs: !accepted,
        summary,
    })
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

fn label_count(value: &Value, label: &str, state: &str) -> Value {
    value
        .get("proxy_label_counts")
        .and_then(|v| v.get(label))
        .and_then(|v| v.get(state))
        .cloned()
        .unwrap_or(Value::Null)
}

fn rejected_reward_vtable_rvas(reward: &Value) -> Vec<Value> {
    reward
        .get("candidate_vtable_hits")
        .and_then(Value::as_object)
        .map(|candidates| {
            candidates
                .iter()
                .filter_map(|(name, row)| {
                    let heap_objects = row.get("heap_objects").and_then(Value::as_u64)?;
                    if heap_objects != 0 || name.starts_with("rtti_") || name == "CProgram_facility"
                    {
                        return None;
                    }
                    Some(json!({
                        "name": name,
                        "vtable_rva": row.get("vtable_rva").cloned().unwrap_or(Value::Null),
                        "heap_objects": heap_objects
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn nested_value(value: &Value, path: &[&str]) -> Value {
    let mut current = value;
    for key in path {
        match current.get(*key) {
            Some(next) => current = next,
            None => return Value::Null,
        }
    }
    current.clone()
}

fn path_slash(path: &str) -> String {
    Path::new(path).to_string_lossy().replace('\\', "/")
}
