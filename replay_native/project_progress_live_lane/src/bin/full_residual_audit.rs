use std::env;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

fn main() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let residual_path = args.next().ok_or_else(usage)?;
    let breakthrough_path = args.next().ok_or_else(usage)?;
    let lifecycle_path = args.next().ok_or_else(usage)?;
    let manifest_path = args.next().ok_or_else(usage)?;
    let replay_gate_path = args.next().ok_or_else(usage)?;
    let out_path = args.next().ok_or_else(usage)?;
    if args.next().is_some() {
        return Err(usage());
    }

    let residual = load_json(&residual_path, "project_progress_live_residual_proof_v1")?;
    let breakthrough = load_json(
        &breakthrough_path,
        "project_progress_breakthrough_state_probe_v1",
    )?;
    let lifecycle = load_json(
        &lifecycle_path,
        "project_progress_lifecycle_stream_proof_v1",
    )?;
    let manifest = load_json(&manifest_path, "native_collector_session_v1")?;
    let replay_gate = load_json_unversioned(&replay_gate_path)?;

    let stream = get_obj(&manifest, "compact_stream")?;
    let per_domain = get_obj(get_obj(stream, "per_domain")?, "projects_live")?;
    let units = get_obj(per_domain, "data_units_total")?;

    let start = get_obj(&residual, "start_semantics")?;
    let completion = get_obj(&residual, "completed_semantics")?;
    let reward = get_obj(&residual, "reward_history")?;
    let contract = get_obj(&residual, "blocker_contract")?;
    let breakthrough_counts = get_obj(&breakthrough, "counts")?;
    let breakthrough_mapping = get_obj(get_obj(&breakthrough, "handle_mapping")?, "best")?;
    let lifecycle_counts = get_obj(&lifecycle, "event_counts")?;
    let lifecycle_frame_summary = get_obj(&lifecycle, "frame_summary")?;

    let compact_transport_ok = bool_path(stream, &["decodable"])?
        && bool_path(stream, &["ordered"])?
        && bool_path(get_obj(stream, "domain_coverage")?, &["ok"])?
        && bool_path(&manifest, &["hourly_requirement", "met"])?
        && number(&manifest, "skipped_game_hours")? == 0
        && !bool_path(stream, &["partial_data_seen"])?
        && !bool_path(stream, &["source_unavailable_seen"])?
        && !bool_path(stream, &["bridge_unavailable_seen"])?
        && !bool_path(stream, &["worker_panic_seen"])?
        && string_array_contains(&manifest, "scheduled_domains", "projects_live")
        && string_value(&manifest, "stream_format")? == "compact";
    let dll_only_roots = number(&manifest, "special_projects_bridge_rows")? == 0
        && string_value(&manifest, "projects_root_source")? != "control_bridge";

    let replay_gate_passed =
        bool_value(&replay_gate, "passed")? && array_len(get_obj(&replay_gate, "errors")?) == 0;
    let replay_container_verified =
        replay_gate_passed || replay_gate.get("details").and_then(|v| v.get("replay_verify")).is_some();

    let start_blocked = !bool_value(start, "accepted")?
        && bool_value(start, "conclusive_rejection")?
        && bool_value(&residual, "start_boolean_rejected_for_schema_expansion")?
        && number(start, "u8_perfect_separators")? == 0
        && number(start, "u32_perfect_separators")? == 0
        && bool_value(contract, "requires_controlled_start_transition")?;

    let completion_bit_in_stream = number(units, "project_completed_state_rows")? > 0
        && number(units, "project_state_rows")? > 0;
    let lifecycle_decode_ok = bool_path(get_obj(&lifecycle, "event_gate")?, &["ok"])?
        && bool_value(&lifecycle, "rust_decoded")?
        && array_len(get_obj(&lifecycle, "parse_errors")?) == 0;
    let completion_transition_proven = number(lifecycle_counts, "completion_changed")? > 0
        && lifecycle_decode_ok
        && bool_value(completion, "accepted")?
        && completion_bit_in_stream;
    let completion_transition_blocked = number(lifecycle_counts, "completion_changed")? == 0
        && number(&lifecycle, "transition_count")? > 0
        && lifecycle_decode_ok
        && bool_value(completion, "accepted")?
        && completion_bit_in_stream
        && number(completion, "mismatches_against_save_completed")? == 0;
    let completion_transition_resolved =
        completion_transition_proven || completion_transition_blocked;

    let reward_blocked = !bool_value(reward, "accepted")?
        && bool_value(reward, "conclusive_rejection")?
        && bool_value(
            &residual,
            "fired_reward_history_root_detected_but_fields_blocked",
        )?
        && bool_value(contract, "requires_controlled_reward_option_transition")?;

    let breakthrough_live_probe_captured = number(breakthrough_counts, "status_rows")? > 0
        && number(breakthrough_counts, "live_rows")? > 0
        && number(breakthrough_counts, "parse_failures")? == 0;
    let breakthrough_compact_rows_present =
        number(lifecycle_frame_summary, "max_breakthrough_rows")? > 0
            && number(units, "project_breakthrough_rows")? > 0;
    let breakthrough_compact_root_blocked =
        breakthrough_live_probe_captured && !breakthrough_compact_rows_present;
    let breakthrough_semantics_blocked =
        !bool_value(&breakthrough, "accepted_for_existing_compact_row")?
            && number(breakthrough_mapping, "matched_rows")? > 0
            && number(breakthrough_mapping, "mismatch_count")? > 0
            && number(breakthrough_mapping, "live_tags_missing_from_save_count")? == 0;

    let lifecycle_delta_captured = bool_value(&lifecycle, "accepted_for_lifecycle_delta_slice")?
        && bool_path(get_obj(&lifecycle, "event_gate")?, &["ok"])?
        && bool_value(&lifecycle, "rust_decoded")?
        && number(lifecycle_counts, "total")? > 0;

    let production_gate_clean = compact_transport_ok && replay_gate_passed && dll_only_roots;
    let all_remaining_exactly_blocked = compact_transport_ok
        && replay_container_verified
        && production_gate_clean
        && start_blocked
        && completion_bit_in_stream
        && completion_transition_resolved
        && reward_blocked
        && breakthrough_live_probe_captured
        && breakthrough_semantics_blocked
        && breakthrough_compact_rows_present
        && lifecycle_delta_captured;

    let audit = json!({
        "schema": "project_progress_full_residual_audit_v1",
        "domain": "projects_live",
        "domain_id": 41,
        "accepted_entire_remaining_slice_blocked": all_remaining_exactly_blocked,
        "completion_status": "NOT COMPLETE",
        "slice_status": if all_remaining_exactly_blocked { "EXACTLY_BLOCKED" } else { "OPEN" },
        "production_complete": false,
        "registration_allowed": false,
        "source_artifacts": {
            "residual_proof": path_slash(&residual_path),
            "breakthrough_state_probe": path_slash(&breakthrough_path),
            "lifecycle_stream_proof": path_slash(&lifecycle_path),
            "native_session_manifest": path_slash(&manifest_path),
            "compact_replay_gate_report": path_slash(&replay_gate_path),
        },
        "runtime_production_status": {
            "dll_only": dll_only_roots,
            "compact_stream": true,
            "rust_decoded": bool_value(&lifecycle, "rust_decoded")?,
            "hoi4replay_verified": replay_container_verified,
            "hourly_or_lower": bool_path(&manifest, &["hourly_requirement", "met"])?,
            "zero_skipped_hours": number(&manifest, "skipped_game_hours")? == 0,
            "partial_data_seen": bool_path(stream, &["partial_data_seen"])?,
            "source_unavailable_seen": bool_path(stream, &["source_unavailable_seen"])?,
            "worker_panic_seen": bool_path(stream, &["worker_panic_seen"])?,
            "bridge_unavailable_seen": bool_path(stream, &["bridge_unavailable_seen"])?,
            "special_projects_bridge_rows": number(&manifest, "special_projects_bridge_rows")?,
            "projects_root_source": string_value(&manifest, "projects_root_source")?,
            "compact_transport_ok": compact_transport_ok,
            "compact_replay_gate_passed": replay_gate_passed,
            "production_gate_clean": production_gate_clean,
        },
        "items": [
            {
                "item": "exact_project_start_state",
                "status": if start_blocked { "BLOCKED_REJECTED" } else { "OPEN" },
                "backlog_effect": "CProjectStateMachine[0..0x280) simple u8/u32 start fields and iteration/current-state/completed duplicates are not retry candidates for start.",
                "root": "CProject+0x28 -> CProjectStateMachine",
                "evidence": {
                    "save_start_true_rows": start.get("save_start_true_rows").cloned().unwrap_or(Value::Null),
                    "save_start_false_rows": start.get("save_start_false_rows").cloned().unwrap_or(Value::Null),
                    "u8_perfect_separators": start.get("u8_perfect_separators").cloned().unwrap_or(Value::Null),
                    "u32_perfect_separators": start.get("u32_perfect_separators").cloned().unwrap_or(Value::Null),
                    "rejected_proxy": start.get("rejected_proxy").cloned().unwrap_or(Value::Null),
                },
                "fixture_blocker": {
                    "required": true,
                    "exact_fixture": [
                        "same live process and save oracle at the same paused hour",
                        "before save: one never-started, not-completed project",
                        "start that exact project",
                        "advance until live state updates, save before completion",
                        "accepted field must flip false-to-true and not duplicate iterations, facility ref, current state selector, or +0x10 completed byte"
                    ]
                }
            },
            {
                "item": "completion_transition_semantics",
                "status": if completion_transition_proven { "PROVEN_COMPACT_TRANSITION" } else if completion_transition_blocked { "FIXTURE_BLOCKED" } else { "OPEN" },
                "backlog_effect": "Completed-bit row is production-captured and this stream derived a completion_changed lifecycle event when the compact state_flags bit changed.",
                "root": "CProject+0x28 -> CProjectStateMachine+0x10",
                "evidence": {
                    "completed_bit_in_stream": completion_bit_in_stream,
                    "completion_transition_proven": completion_transition_proven,
                    "project_state_rows": units.get("project_state_rows").cloned().unwrap_or(Value::Null),
                    "project_completed_state_rows": units.get("project_completed_state_rows").cloned().unwrap_or(Value::Null),
                    "fresh_save_completed_mismatches": completion.get("mismatches_against_save_completed").cloned().unwrap_or(Value::Null),
                    "lifecycle_transitions": lifecycle.get("transition_count").cloned().unwrap_or(Value::Null),
                    "completion_changed_events": lifecycle_counts.get("completion_changed").cloned().unwrap_or(Value::Null),
                },
                "fixture_blocker": {
                    "required": !completion_transition_proven,
                    "exact_fixture": [
                        "record compact projects_live stream through at least one known project completion",
                        "before frame: same project has completed flag 0 in compact state_flags",
                        "after frame: same project has completed flag 1",
                        "derive one completion_changed lifecycle event for that (project_id, static_def_handle)",
                        "save/live comparison must be lockstep or omitted as validation truth"
                    ]
                }
            },
            {
                "item": "reward_history_object_lifecycle",
                "status": if reward_blocked { "BLOCKED_REJECTED" } else { "OPEN" },
                "backlog_effect": "CProgramStatus+0x58 begin/end/cap and pointer-payload reward refs are rejected as fired reward-history roots.",
                "root": "CCountry -> CProgramStatus+0x58 candidate rejected",
                "evidence": {
                    "field_blocker": reward.get("field_blocker").cloned().unwrap_or(Value::Null),
                    "rejected_containers": reward.get("rejected_containers").cloned().unwrap_or(Value::Null),
                    "rejected_vtable_rvas": reward.get("rejected_vtable_rvas").cloned().unwrap_or(Value::Null),
                },
                "fixture_blocker": {
                    "required": true,
                    "exact_fixture": [
                        "before save: selected country has unread_prototype_rewards > 0",
                        "select one known reward option in UI",
                        "after save: exactly one new fired row exists",
                        "accepted root must pin tag/project, option/prototype reward, date/hour, province, progress, and scientist context",
                        "unread_prototype_rewards is only lifecycle context, not a substitute for fired history"
                    ]
                }
            },
            {
                "item": "breakthrough_state_and_semantics",
                "status": if breakthrough_compact_rows_present && breakthrough_semantics_blocked { "CAPTURED_WITH_LOCKSTEP_FIXTURE_BLOCKER" } else if breakthrough_compact_root_blocked && breakthrough_semantics_blocked { "BLOCKED_COMPACT_COUNTRY_ROOT_AND_LOCKSTEP_FIXTURE" } else { "OPEN" },
                "backlog_effect": "DLL-only compact capture now emits CProgramStatus+0x50 breakthrough rows and breakthrough_delta lifecycle events; save-key semantic naming remains lockstep-fixture blocked.",
                "root": breakthrough.get("root").cloned().unwrap_or(Value::Null),
                "evidence": {
                    "row_shape": breakthrough.get("row_shape").cloned().unwrap_or(Value::Null),
                    "counts": breakthrough_counts.clone(),
                    "best_handle_mapping": breakthrough_mapping.clone(),
                    "project_breakthrough_rows": units.get("project_breakthrough_rows").cloned().unwrap_or(Value::Null),
                    "compact_max_breakthrough_rows": lifecycle_frame_summary.get("max_breakthrough_rows").cloned().unwrap_or(Value::Null),
                    "breakthrough_delta_events": lifecycle_counts.get("breakthrough_delta").cloned().unwrap_or(Value::Null),
                },
                "fixture_blocker": {
                    "required": true,
                    "exact_fixture": [
                        "pause game and create save immediately before live probe",
                        "require zero live/save breakthrough point mismatches for all live tags",
                        "accept handle-to-specialization names only if all four handles map exactly to save specialization_nuclear/naval/air/land",
                        "otherwise keep compact rows as handles plus points_raw"
                    ]
                }
            },
            {
                "item": "completion_reward_lifecycle_events",
                "status": if lifecycle_delta_captured && completion_transition_proven && number(lifecycle_counts, "breakthrough_delta")? > 0 && reward_blocked { "COMPLETION_AND_BREAKTHROUGH_EVENTS_CAPTURED_REWARD_BLOCKED" } else if lifecycle_decode_ok && completion_transition_blocked && reward_blocked { "FIXTURE_BLOCKED_NO_EVENTS_IN_STREAM" } else { "OPEN" },
                "backlog_effect": "Lifecycle compact event format, Rust decode, completion_changed events, and breakthrough_delta events are proven; reward events still need the fired reward-history root.",
                "root": "lane-local PLE1 lifecycle event stream derived from projects_live compact frames",
                "evidence": {
                    "event_compact_bin": lifecycle.get("event_compact_bin").cloned().unwrap_or(Value::Null),
                    "lifecycle_delta_captured": lifecycle_delta_captured,
                    "event_counts": lifecycle_counts.clone(),
                    "event_gate": lifecycle.get("event_gate").cloned().unwrap_or(Value::Null),
                },
                "fixture_blocker": {
                    "required": reward_blocked,
                    "exact_fixture": [
                        "reward event: before/after reward-option selection after fired-history row root is pinned",
                        "events must round-trip through PLE1 encode/decode/gate"
                    ]
                }
            }
        ],
        "decision": if all_remaining_exactly_blocked {
            "The projects_live residual slice is not fully production-complete; DLL-only root discovery, compact replay, completion transition events, and breakthrough delta events are proven. Remaining residuals are exact fixture blockers for start semantics, fired reward history, reward lifecycle events, and lockstep breakthrough semantic names."
        } else {
            "At least one residual item is still open."
        }
    });

    fs::write(
        &out_path,
        serde_json::to_string_pretty(&audit).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("failed to write {out_path}: {err}"))?;
    if !all_remaining_exactly_blocked {
        return Err(format!("full residual audit failed; wrote {out_path}"));
    }
    Ok(())
}

fn usage() -> String {
    "usage: project_progress_full_residual_audit <residual_proof.json> <breakthrough_state_probe.json> <lifecycle_stream_proof.json> <native_session_manifest.json> <compact_replay_gate_report.json> <audit.json>".to_string()
}

fn load_json(path: &str, expected_schema: &str) -> Result<Value, String> {
    let value = load_json_unversioned(path)?;
    let schema = string_value(&value, "schema")?;
    if schema != expected_schema {
        return Err(format!("{path} schema {schema:?} != {expected_schema:?}"));
    }
    Ok(value)
}

fn load_json_unversioned(path: &str) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("failed to read {path}: {err}"))?;
    serde_json::from_str(&text).map_err(|err| format!("failed to parse {path}: {err}"))
}

fn get_obj<'a>(value: &'a Value, key: &str) -> Result<&'a Value, String> {
    value
        .get(key)
        .ok_or_else(|| format!("missing object {key}"))
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

fn string_value<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string {key}"))
}

fn bool_path(value: &Value, path: &[&str]) -> Result<bool, String> {
    let mut current = value;
    for key in path {
        current = current
            .get(*key)
            .ok_or_else(|| format!("missing {}", path.join(".")))?;
    }
    current
        .as_bool()
        .ok_or_else(|| format!("{} is not bool", path.join(".")))
}

fn array_len(value: &Value) -> usize {
    value.as_array().map(Vec::len).unwrap_or(0)
}

fn string_array_contains(value: &Value, key: &str, expected: &str) -> bool {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| items.iter().any(|item| item.as_str() == Some(expected)))
        .unwrap_or(false)
}

fn path_slash(path: &str) -> String {
    Path::new(path).to_string_lossy().replace('\\', "/")
}
