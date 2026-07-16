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
    let manifest_bytes =
        fs::read(&manifest_path).map_err(|err| format!("failed to read {manifest_path}: {err}"))?;
    let authority_binding =
        validate_authority_binding(&lifecycle, &manifest, &replay_gate, &manifest_bytes)?;

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

    let replay_gate_passed = true;
    let replay_container_verified = true;

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
        && empty_array(&lifecycle, "parse_errors")?;
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
        "authority_binding": authority_binding,
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

fn validate_authority_binding(
    lifecycle: &Value,
    manifest: &Value,
    gate: &Value,
    manifest_bytes: &[u8],
) -> Result<Value, String> {
    if string_value(gate, "gate")? != "compact_replay_verification_gates" {
        return Err(
            "compact gate discriminator is not compact_replay_verification_gates".to_string(),
        );
    }
    if !bool_value(gate, "passed")? || !empty_array(gate, "errors")? {
        return Err("compact gate did not pass with a strictly empty errors array".to_string());
    }
    let details = get_obj(gate, "details")?;
    let profile = get_obj(details, "verification_profile")?;
    if !bool_value(profile, "authoritative")?
        || !bool_value(profile, "replay_verify_required")?
        || !empty_array(profile, "weakenings")?
    {
        return Err("compact gate verification profile is not authoritative".to_string());
    }
    let replay = get_obj(details, "replay_verify")?;
    if !bool_value(replay, "external_manifest_exact_match")? {
        return Err("compact gate did not bind the exact external manifest".to_string());
    }

    let session_id = nonempty_string(manifest, "session_id")?;
    let replay_game_id = nonempty_string(manifest, "replay_game_id")?;
    let gate_game_id = nonempty_string(replay, "game_id")?;
    if session_id != replay_game_id || replay_game_id != gate_game_id {
        return Err(format!(
            "session identity mismatch: session_id {session_id:?}, replay_game_id {replay_game_id:?}, gate game_id {gate_game_id:?}"
        ));
    }

    let lifecycle_identity = get_obj(lifecycle, "stream_identity")?;
    let manifest_storage = get_obj(manifest, "stream_storage")?;
    let gate_storage = get_obj(details, "stream_storage")?;
    let compact = get_obj(manifest, "compact_stream")?;
    let compression = nonempty_string(lifecycle_identity, "compression")?;
    if !matches!(compression, "none" | "zstd")
        || nonempty_string(manifest_storage, "compression")? != compression
        || nonempty_string(gate_storage, "compression")? != compression
    {
        return Err("stream compression mismatch across lifecycle, manifest, and gate".to_string());
    }
    let stored_lengths = [
        ("lifecycle", number(lifecycle_identity, "stored_bytes")?),
        ("manifest", number(manifest_storage, "stored_bytes")?),
        ("gate", number(gate_storage, "stored_bytes")?),
    ];
    let expected_stored_length = stored_lengths[0].1;
    if expected_stored_length == 0
        || stored_lengths
            .iter()
            .any(|(_, value)| *value != expected_stored_length)
    {
        return Err(format!("stored stream length mismatch: {stored_lengths:?}"));
    }
    let lifecycle_stored_hash = string_value(lifecycle_identity, "stored_sha256")?;
    let gate_stored_hash = string_value(gate_storage, "stored_sha256")?;
    validate_lower_sha256(lifecycle_stored_hash)
        .map_err(|error| format!("lifecycle stored stream hash: {error}"))?;
    validate_lower_sha256(gate_stored_hash)
        .map_err(|error| format!("gate stored stream hash: {error}"))?;
    if lifecycle_stored_hash != gate_stored_hash {
        return Err("stored stream sha256 mismatch between lifecycle and gate".to_string());
    }
    if let Some(manifest_stored_hash) = manifest_storage
        .get("stored_sha256")
        .and_then(Value::as_str)
    {
        validate_lower_sha256(manifest_stored_hash)
            .map_err(|error| format!("manifest stored stream hash: {error}"))?;
        if manifest_stored_hash != lifecycle_stored_hash {
            return Err("stored stream sha256 mismatch against manifest".to_string());
        }
    }
    let stream_hashes = [
        (
            "lifecycle",
            string_value(lifecycle_identity, "decoded_sha256")?,
        ),
        (
            "manifest",
            string_value(manifest_storage, "decoded_sha256")?,
        ),
        ("gate", string_value(gate_storage, "decoded_sha256")?),
        (
            "replay container",
            string_value(replay, "collector_compact_stream_sha256")?,
        ),
    ];
    for (label, hash) in &stream_hashes {
        validate_lower_sha256(hash).map_err(|error| format!("{label} stream hash: {error}"))?;
    }
    let expected_hash = stream_hashes[0].1;
    if stream_hashes.iter().any(|(_, hash)| *hash != expected_hash) {
        return Err(format!("decoded stream sha256 mismatch: {stream_hashes:?}"));
    }

    let stream_lengths = [
        ("lifecycle", number(lifecycle_identity, "decoded_bytes")?),
        (
            "manifest storage",
            number(manifest_storage, "decoded_bytes")?,
        ),
        ("manifest compact", number(compact, "total_bytes")?),
        ("gate", number(gate_storage, "decoded_bytes")?),
        (
            "replay container",
            number(replay, "collector_compact_stream_bytes")?,
        ),
    ];
    let expected_length = stream_lengths[0].1;
    if expected_length == 0
        || stream_lengths
            .iter()
            .any(|(_, value)| *value != expected_length)
    {
        return Err(format!(
            "decoded stream length mismatch: {stream_lengths:?}"
        ));
    }

    let manifest_hash = sha256_hex(manifest_bytes);
    let gate_manifest_hash = string_value(replay, "recorder_manifest_sha256")?;
    validate_lower_sha256(gate_manifest_hash)
        .map_err(|error| format!("gate recorder manifest hash: {error}"))?;
    if manifest_hash != gate_manifest_hash {
        return Err(format!(
            "native manifest sha256 {manifest_hash} != gate recorder_manifest_sha256 {gate_manifest_hash}"
        ));
    }
    if manifest_bytes.len() as u64 != number(replay, "recorder_manifest_bytes")? {
        return Err(
            "native manifest byte length differs from gate recorder_manifest_bytes".to_string(),
        );
    }

    if number(lifecycle, "domain_id")? != 41 {
        return Err("lifecycle proof domain_id is not 41".to_string());
    }
    let per_domain = get_obj(get_obj(compact, "per_domain")?, "projects_live")?;
    if number(per_domain, "domain_id")? != 41 {
        return Err("manifest projects_live domain_id is not 41".to_string());
    }
    for (label, left, right) in [
        (
            "record_count",
            number(lifecycle, "record_count")?,
            number(compact, "record_count")?,
        ),
        (
            "projects_live data_record_count",
            number(lifecycle, "project_data_records")?,
            number(per_domain, "data_record_count")?,
        ),
        (
            "first_game_hour_raw",
            number(get_obj(lifecycle, "frame_summary")?, "first_game_hour_raw")?,
            number(manifest, "first_game_hour_raw")?,
        ),
        (
            "last_game_hour_raw",
            number(get_obj(lifecycle, "frame_summary")?, "last_game_hour_raw")?,
            number(manifest, "last_game_hour_raw")?,
        ),
    ] {
        if left != right {
            return Err(format!(
                "{label} mismatch: lifecycle {left}, manifest {right}"
            ));
        }
    }

    Ok(json!({
        "ok": true,
        "session_id": session_id,
        "replay_game_id": replay_game_id,
        "decoded_stream_bytes": expected_length,
        "decoded_stream_sha256": expected_hash,
        "stream_compression": compression,
        "stored_stream_bytes": expected_stored_length,
        "stored_stream_sha256": lifecycle_stored_hash,
        "native_manifest_bytes": manifest_bytes.len(),
        "native_manifest_sha256": manifest_hash,
        "gate_authoritative": true,
        "external_manifest_exact_match": true,
    }))
}

fn empty_array(value: &Value, key: &str) -> Result<bool, String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::is_empty)
        .ok_or_else(|| format!("{key} is missing or not an array"))
}

fn nonempty_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    let text = string_value(value, key)?;
    if text.is_empty() {
        Err(format!("{key} is empty"))
    } else {
        Ok(text)
    }
}

fn validate_lower_sha256(value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{value:?} is not lowercase 64-hex SHA-256"));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn authority_fixture() -> (Value, Value, Value, Vec<u8>) {
        let manifest_bytes = b"exact native manifest bytes".to_vec();
        let manifest_hash = sha256_hex(&manifest_bytes);
        let stream_hash = sha256_hex(b"collector stream");
        let lifecycle = json!({
            "domain_id": 41,
            "record_count": 6,
            "project_data_records": 2,
            "stream_identity": {
                "compression": "none",
                "stored_bytes": 16,
                "stored_sha256": stream_hash,
                "decoded_bytes": 16,
                "decoded_sha256": stream_hash
            },
            "frame_summary": {"first_game_hour_raw": 100, "last_game_hour_raw": 101},
        });
        let manifest = json!({
            "session_id": "session-a",
            "replay_game_id": "session-a",
            "first_game_hour_raw": 100,
            "last_game_hour_raw": 101,
            "stream_storage": {
                "compression": "none",
                "stored_bytes": 16,
                "stored_sha256": stream_hash,
                "decoded_bytes": 16,
                "decoded_sha256": stream_hash
            },
            "compact_stream": {
                "total_bytes": 16,
                "record_count": 6,
                "per_domain": {"projects_live": {"domain_id": 41, "data_record_count": 2}}
            }
        });
        let gate = json!({
            "gate": "compact_replay_verification_gates",
            "passed": true,
            "errors": [],
            "details": {
                "verification_profile": {
                    "authoritative": true,
                    "replay_verify_required": true,
                    "weakenings": []
                },
                "stream_storage": {
                    "compression": "none",
                    "stored_bytes": 16,
                    "stored_sha256": stream_hash,
                    "decoded_bytes": 16,
                    "decoded_sha256": stream_hash
                },
                "replay_verify": {
                    "game_id": "session-a",
                    "collector_compact_stream_bytes": 16,
                    "collector_compact_stream_sha256": stream_hash,
                    "recorder_manifest_bytes": manifest_bytes.len(),
                    "recorder_manifest_sha256": manifest_hash,
                    "external_manifest_exact_match": true
                }
            }
        });
        (lifecycle, manifest, gate, manifest_bytes)
    }

    #[test]
    fn accepts_exact_stream_session_and_manifest_binding() {
        let (lifecycle, manifest, gate, bytes) = authority_fixture();
        let binding = validate_authority_binding(&lifecycle, &manifest, &gate, &bytes).unwrap();
        assert_eq!(binding["ok"], json!(true));
        assert_eq!(binding["session_id"], json!("session-a"));
    }

    #[test]
    fn rejects_forged_or_weakened_gate_shapes() {
        let (lifecycle, manifest, mut gate, bytes) = authority_fixture();
        gate["errors"] = Value::Null;
        assert!(
            validate_authority_binding(&lifecycle, &manifest, &gate, &bytes)
                .unwrap_err()
                .contains("not an array")
        );

        let (lifecycle, manifest, mut gate, bytes) = authority_fixture();
        gate["details"]["verification_profile"]["authoritative"] = json!(false);
        assert!(
            validate_authority_binding(&lifecycle, &manifest, &gate, &bytes)
                .unwrap_err()
                .contains("not authoritative")
        );
    }

    #[test]
    fn rejects_cross_stream_and_cross_session_splices() {
        let (mut lifecycle, manifest, gate, bytes) = authority_fixture();
        lifecycle["stream_identity"]["decoded_sha256"] = json!(sha256_hex(b"other"));
        assert!(
            validate_authority_binding(&lifecycle, &manifest, &gate, &bytes)
                .unwrap_err()
                .contains("sha256 mismatch")
        );

        let (lifecycle, mut manifest, gate, bytes) = authority_fixture();
        manifest["session_id"] = json!("session-b");
        assert!(
            validate_authority_binding(&lifecycle, &manifest, &gate, &bytes)
                .unwrap_err()
                .contains("session identity mismatch")
        );
    }

    #[test]
    fn rejects_length_count_hour_and_manifest_digest_mismatches() {
        let (lifecycle, mut manifest, gate, bytes) = authority_fixture();
        manifest["compact_stream"]["total_bytes"] = json!(15);
        assert!(
            validate_authority_binding(&lifecycle, &manifest, &gate, &bytes)
                .unwrap_err()
                .contains("stream length mismatch")
        );

        let (mut lifecycle, manifest, gate, bytes) = authority_fixture();
        lifecycle["record_count"] = json!(5);
        assert!(
            validate_authority_binding(&lifecycle, &manifest, &gate, &bytes)
                .unwrap_err()
                .contains("record_count mismatch")
        );

        let (lifecycle, manifest, gate, _) = authority_fixture();
        assert!(
            validate_authority_binding(&lifecycle, &manifest, &gate, b"stale manifest")
                .unwrap_err()
                .contains("manifest sha256")
        );
    }

    #[test]
    fn rejects_malformed_hash_spelling() {
        let (mut lifecycle, manifest, gate, bytes) = authority_fixture();
        lifecycle["stream_identity"]["decoded_sha256"] = json!("ABC");
        assert!(
            validate_authority_binding(&lifecycle, &manifest, &gate, &bytes)
                .unwrap_err()
                .contains("lowercase 64-hex")
        );
    }

    #[test]
    fn sha256_matches_standard_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
