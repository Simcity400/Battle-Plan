use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use event_timeline_change_log_live::{
    gate_event_compact_stream, gate_event_families, gate_missing_source_adapters,
    missing_source_adapter_contracts, source_domain_name, CompactSourceSummary, EventKind,
    SourceDomainUnitSnapshot,
};
use serde_json::{json, Value};
use snapshot::{pack, Container, ExpectedIdentity, PackEntry, PackIdentity};

fn main() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let stream_path = args.next().ok_or_else(usage)?;
    let manifest_path = args.next().ok_or_else(usage)?;
    let out_dir = args.next().ok_or_else(usage)?;
    let mut commanded_game_hour_advancements = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game-hours" => {
                let raw = args.next().ok_or_else(usage)?;
                let parsed = raw
                    .parse::<i64>()
                    .map_err(|_| format!("--game-hours needs an integer, got {raw:?}"))?;
                if parsed <= 0 {
                    return Err(format!(
                        "--game-hours must be explicitly positive, got {parsed}"
                    ));
                }
                if commanded_game_hour_advancements.replace(parsed).is_some() {
                    return Err("--game-hours may be supplied only once".to_string());
                }
            }
            _ => return Err(usage()),
        }
    }
    let commanded_game_hour_advancements = commanded_game_hour_advancements.ok_or_else(|| {
        "--game-hours is required explicitly; mutable manifest fallback is forbidden".to_string()
    })?;

    fs::create_dir_all(&out_dir).map_err(|err| format!("failed to create {out_dir}: {err}"))?;
    let stream = fs::read(&stream_path)
        .map_err(|err| format!("failed to read compact stream {stream_path}: {err}"))?;
    let manifest_bytes = fs::read(&manifest_path)
        .map_err(|err| format!("failed to read manifest {manifest_path}: {err}"))?;
    let manifest: Value = snapshot::parse_json_strict(&manifest_bytes)
        .map_err(|err| format!("failed to parse manifest {manifest_path}: {err}"))?;

    let event_gate = gate_event_compact_stream(&stream, commanded_game_hour_advancements);
    if !event_gate.ok {
        return Err(format!(
            "supplied compact event stream failed production gate: framing={:?}; ordering={:?}; event={:?}",
            event_gate.source_summary.parse_errors,
            event_gate.source_summary.ordering_errors,
            event_gate.payload_errors
        ));
    }
    let source_summary = &event_gate.source_summary;
    let manifest_errors =
        validate_manifest_health(&manifest, source_summary, commanded_game_hour_advancements);
    if !manifest_errors.is_empty() {
        return Err(format!(
            "source manifest failed production-health gate: {}",
            manifest_errors.join("; ")
        ));
    }
    let families = gate_event_families(source_summary);
    let source_domain_units = source_domain_units_from_manifest(&manifest, source_summary);
    let missing_source_adapter_contracts = missing_source_adapter_contracts();
    let missing_source_adapter_gates = gate_missing_source_adapters(&source_domain_units);

    let compact_path = Path::new(&out_dir).join("event_timeline_change_log_live.compact.bin");
    fs::write(&compact_path, &stream)
        .map_err(|err| format!("failed to write {}: {err}", compact_path.display()))?;

    let game_id = manifest
        .get("replay_game_id")
        .or_else(|| manifest.get("session_id"))
        .and_then(Value::as_str)
        .unwrap_or("event_timeline_change_log_live_proof");
    let source_manifest_checks = json!({
        "health_gate_passed": true,
        "commanded_game_hour_advancements": commanded_game_hour_advancements,
        "required_closed_frames": commanded_game_hour_advancements.saturating_add(1),
        "game_hours_advanced": manifest.get("game_hours_advanced").and_then(Value::as_i64),
        "unique_game_hour_count": manifest.get("unique_game_hour_count").and_then(Value::as_i64),
        "decodable": manifest_pointer_bool(&manifest, &["compact_stream", "decodable"]),
        "ordered": manifest_pointer_bool(&manifest, &["compact_stream", "ordered"]),
        "partial_data_seen": manifest_pointer_bool(&manifest, &["compact_stream", "partial_data_seen"]),
        "bridge_unavailable_seen": manifest_pointer_bool(&manifest, &["compact_stream", "bridge_unavailable_seen"]),
        "source_unavailable_seen": manifest_pointer_bool(&manifest, &["compact_stream", "source_unavailable_seen"]),
        "worker_panic_seen": manifest_pointer_bool(&manifest, &["compact_stream", "worker_panic_seen"]),
        "skipped_game_hours": manifest.get("skipped_game_hours").and_then(Value::as_i64),
        "hourly_requirement_met": manifest_pointer_bool(&manifest, &["hourly_requirement", "met"]),
        "replay_file": manifest.get("replay_file").and_then(Value::as_str),
    });

    let mut proof = json!({
        "schema": "event_timeline_change_log_live_proof_v1",
        "source_stream": path_slash(&stream_path),
        "source_manifest": path_slash(&manifest_path),
        "event_compact_stream": path_slash_path(&compact_path),
        "event_domain_id": event_timeline_change_log_live::DOMAIN_ID,
        "production_capture_integrated": false,
        "supplied_stream_production_gate_passed": true,
        "completion_basis": "the supplied source stream passed canonical event source/provenance/frame/status validation without proof fixtures",
        "source_stream_summary": {
            "total_bytes": source_summary.total_bytes,
            "record_count": source_summary.record_count,
            "domain_data_record_count": source_summary.domain_data_record_count,
            "domain_perf_record_count": source_summary.domain_perf_record_count,
            "frame_end_record_count": source_summary.frame_end_record_count,
            "first_hour": source_summary.first_hour,
            "last_hour": source_summary.last_hour,
            "unique_hour_count": source_summary.unique_hours.len(),
            "skipped_hours": source_summary.skipped_hours,
            "domains_seen": source_summary.domains_seen.iter().map(|id| json!({
                "id": id,
                "name": source_domain_name(*id),
                "data_records": source_summary.per_domain_data_record_count.get(id).copied().unwrap_or(0),
                "perf_records": source_summary.per_domain_perf_record_count.get(id).copied().unwrap_or(0),
            })).collect::<Vec<_>>(),
            "parse_errors": source_summary.parse_errors,
            "ordering_errors": source_summary.ordering_errors,
        },
        "source_manifest_checks": source_manifest_checks,
        "event_family_gates": families.iter().map(|gate| json!({
            "family": gate.family,
            "supported_by_selected_stream": gate.supported,
            "required_domains": gate.required_domain_ids.iter().map(|id| json!({"id": id, "name": source_domain_name(*id)})).collect::<Vec<_>>(),
            "optional_domains_present": gate.optional_domain_ids.iter().filter(|id| gate.present_domain_ids.contains(id)).map(|id| json!({"id": id, "name": source_domain_name(*id)})).collect::<Vec<_>>(),
            "missing_domains": gate.missing_domain_ids.iter().map(|id| json!({"id": id, "name": source_domain_name(*id)})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "missing_source_adapter_contracts": missing_source_adapter_contracts.iter().map(|contract| json!({
            "family": contract.family,
            "source_domain": {"id": contract.source_domain_id, "name": source_domain_name(contract.source_domain_id)},
            "event_kinds": contract.event_kinds.iter().map(|kind| kind.name()).collect::<Vec<_>>(),
            "identity_fields": contract.identity_fields,
            "provenance_fields": contract.provenance_fields,
            "unit_requirements": contract.unit_requirements.iter().map(|req| json!({
                "unit": req.unit,
                "check": req.check.name(),
            })).collect::<Vec<_>>(),
            "adapter_note": contract.adapter_note,
        })).collect::<Vec<_>>(),
        "missing_source_adapter_readiness": missing_source_adapter_gates.iter().map(|gate| json!({
            "family": gate.family,
            "source_domain": {"id": gate.source_domain_id, "name": source_domain_name(gate.source_domain_id)},
            "ready": gate.ready,
            "blockers": gate.blockers,
            "unit_requirements": gate.unit_requirements.iter().map(|req| json!({
                "unit": req.unit,
                "check": req.check.name(),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "event_contract_gate": {
            "ok": event_gate.ok,
            "event_payloads": event_gate.event_payloads,
            "event_rows_total": event_gate.event_rows_total,
            "frame_end_record_count": event_gate.source_summary.frame_end_record_count,
            "event_kind_counts": event_gate.event_kind_counts.iter().map(|(kind, count)| json!({
                "kind_id": kind,
                "kind": EventKind::from_raw(*kind).map(EventKind::name).unwrap_or("unknown"),
                "count": count,
            })).collect::<Vec<_>>(),
            "source_domains": event_gate.source_domains.iter().map(|id| json!({"id": id, "name": source_domain_name(*id)})).collect::<Vec<_>>(),
            "payload_errors": event_gate.payload_errors,
            "stream_parse_errors": event_gate.source_summary.parse_errors,
            "stream_ordering_errors": event_gate.source_summary.ordering_errors,
        },
        "integration_patch_plan": [
            "Reserve compact domain 44 in shared collector/recorder registries.",
            "Emit event_timeline_change_log_live DOMAIN_DATA after source-domain records and before FRAME_END.",
            "Use this decoder gate in recorder_host; fail closed on invalid identity/provenance/flags.",
            "Store event stream in .battleplan with manifest role event_timeline_compact_stream.",
            "Promote only validated direct source rows or deterministic adjacent-hour deltas; source gaps emit no guessed gameplay events."
        ],
    });

    let replay_path = Path::new(&out_dir).join("event_timeline_change_log_live.battleplan");
    let proof_path = Path::new(&out_dir).join("event_timeline_change_log_live_proof.json");
    let container_result = pack_and_verify(game_id, &stream, &proof, &replay_path);
    proof["event_hoi4replay"] = json!({
        "path": path_slash_path(&replay_path),
        "packed": container_result.is_ok(),
        "verified": container_result.as_ref().map(|_| true).unwrap_or(false),
        "error": container_result.as_ref().err().cloned(),
    });

    fs::write(
        &proof_path,
        serde_json::to_string_pretty(&proof).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("failed to write {}: {err}", proof_path.display()))?;
    if let Err(err) = container_result {
        return Err(format!(
            "wrote {}, but .battleplan verification failed: {err}",
            proof_path.display()
        ));
    }
    println!("{}", proof_path.display());
    Ok(())
}

fn validate_manifest_health(
    manifest: &Value,
    source_summary: &CompactSourceSummary,
    commanded_game_hour_advancements: i64,
) -> Vec<String> {
    let mut errors = Vec::new();
    let hourly = manifest.get("hourly_requirement").unwrap_or(&Value::Null);
    require_bool(hourly, "required", true, "hourly_requirement", &mut errors);
    require_bool(hourly, "met", true, "hourly_requirement", &mut errors);
    require_i64(
        hourly,
        "requested_game_hours",
        commanded_game_hour_advancements,
        "hourly_requirement",
        &mut errors,
    );
    match hourly.get("problems").and_then(Value::as_array) {
        Some(problems) if problems.is_empty() => {}
        other => errors.push(format!(
            "hourly_requirement.problems is not an empty array: {other:?}"
        )),
    }

    let observed_advanced = source_summary
        .first_hour
        .zip(source_summary.last_hour)
        .map(|(first, last)| i64::from(last) - i64::from(first))
        .unwrap_or(-1);
    let observed_unique = i64::try_from(source_summary.unique_hours.len()).unwrap_or(i64::MAX);
    require_i64(
        manifest,
        "game_hours_advanced",
        observed_advanced,
        "manifest",
        &mut errors,
    );
    require_i64(
        manifest,
        "unique_game_hour_count",
        observed_unique,
        "manifest",
        &mut errors,
    );
    require_i64(manifest, "skipped_game_hours", 0, "manifest", &mut errors);
    if observed_advanced != commanded_game_hour_advancements {
        errors.push(format!(
            "game_hours_advanced {observed_advanced} does not equal explicit command {commanded_game_hour_advancements}"
        ));
    }
    let expected_unique = observed_advanced.checked_add(1).unwrap_or(i64::MAX);
    if observed_unique != expected_unique {
        errors.push(format!(
            "stream unique_game_hour_count {observed_unique} != game_hours_advanced + 1 ({expected_unique})"
        ));
    }

    let compact = manifest.get("compact_stream").unwrap_or(&Value::Null);
    if compact.get("record_magic").and_then(Value::as_str) != Some("H4CR") {
        errors.push("compact_stream.record_magic is not H4CR".to_string());
    }
    require_i64(
        compact,
        "record_header_len",
        32,
        "compact_stream",
        &mut errors,
    );
    require_i64(compact, "record_version", 1, "compact_stream", &mut errors);
    require_bool(compact, "decodable", true, "compact_stream", &mut errors);
    require_bool(compact, "ordered", true, "compact_stream", &mut errors);
    require_bool(
        compact,
        "worker_panic_seen",
        false,
        "compact_stream",
        &mut errors,
    );
    require_bool(
        compact,
        "bridge_unavailable_seen",
        false,
        "compact_stream",
        &mut errors,
    );
    require_bool(
        compact,
        "source_unavailable_seen",
        false,
        "compact_stream",
        &mut errors,
    );
    require_bool(
        compact,
        "partial_data_seen",
        false,
        "compact_stream",
        &mut errors,
    );
    require_i64(
        compact,
        "frame_end_record_count",
        observed_unique,
        "compact_stream",
        &mut errors,
    );
    let coverage = compact.get("domain_coverage").unwrap_or(&Value::Null);
    require_bool(
        coverage,
        "ok",
        true,
        "compact_stream.domain_coverage",
        &mut errors,
    );
    match coverage.get("errors").and_then(Value::as_array) {
        Some(coverage_errors) if coverage_errors.is_empty() => {}
        other => errors.push(format!(
            "compact_stream.domain_coverage.errors is not an empty array: {other:?}"
        )),
    }
    if compact
        .get("per_domain")
        .and_then(Value::as_object)
        .is_none()
    {
        errors.push("compact_stream.per_domain is missing or not an object".to_string());
    }
    if manifest
        .get("replay_file")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .is_none()
    {
        errors.push("replay_file is missing or empty".to_string());
    }
    errors
}

fn require_bool(object: &Value, key: &str, expected: bool, prefix: &str, errors: &mut Vec<String>) {
    if object.get(key).and_then(Value::as_bool) != Some(expected) {
        errors.push(format!(
            "{prefix}.{key} is {:?}, expected {expected}",
            object.get(key)
        ));
    }
}

fn require_i64(object: &Value, key: &str, expected: i64, prefix: &str, errors: &mut Vec<String>) {
    if object.get(key).and_then(Value::as_i64) != Some(expected) {
        errors.push(format!(
            "{prefix}.{key} is {:?}, expected {expected}",
            object.get(key)
        ));
    }
}

fn source_domain_units_from_manifest(
    manifest: &Value,
    source_summary: &CompactSourceSummary,
) -> BTreeMap<u16, SourceDomainUnitSnapshot> {
    let mut out = BTreeMap::new();
    let global_error_count =
        (source_summary.parse_errors.len() + source_summary.ordering_errors.len()) as u64;
    let Some(per_domain) = manifest
        .get("compact_stream")
        .and_then(|stream| stream.get("per_domain"))
        .and_then(Value::as_object)
    else {
        return out;
    };
    for domain in per_domain.values() {
        let Some(domain_id) = domain.get("domain_id").and_then(Value::as_u64) else {
            continue;
        };
        let mut snapshot = SourceDomainUnitSnapshot {
            domain_id: domain_id as u16,
            data_record_count: domain
                .get("data_record_count")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            parse_error_count: global_error_count,
            units: BTreeMap::new(),
        };
        if let Some(units) = domain.get("data_units_total").and_then(Value::as_object) {
            for (key, value) in units {
                if let Some(count) = value.as_u64() {
                    snapshot.units.insert(key.clone(), count);
                }
            }
        }
        out.insert(snapshot.domain_id, snapshot);
    }
    out
}

fn pack_and_verify(
    game_id: &str,
    event_stream: &[u8],
    proof: &Value,
    replay_path: &Path,
) -> Result<(), String> {
    let stream_entry =
        format!("payload/{game_id}/collector_raw/event_timeline_change_log_live.compact.bin");
    let proof_entry =
        format!("payload/{game_id}/recorder/event_timeline_change_log_live_proof.json");
    let index_entry = format!("payload/{game_id}/index.json");
    let timeline_entry = format!("payload/{game_id}/timeline.json");
    let index_bytes = serde_json::to_vec_pretty(&json!({
        "game_id": game_id,
        "schema_version": "event_timeline_change_log_live_proof_v1",
        "snapshots": []
    }))
    .map_err(|err| err.to_string())?;
    let timeline_bytes = serde_json::to_vec_pretty(&json!({
        "schema": "event_timeline_change_log_live_timeline_v1",
        "compact_event_stream": "collector_raw/event_timeline_change_log_live.compact.bin",
        "proof": "recorder/event_timeline_change_log_live_proof.json"
    }))
    .map_err(|err| err.to_string())?;
    let proof_bytes = serde_json::to_vec_pretty(proof).map_err(|err| err.to_string())?;
    let entries = vec![
        PackEntry {
            path: index_entry,
            bytes: index_bytes,
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: stream_entry.clone(),
            bytes: event_stream.to_vec(),
            role: "event_timeline_compact_stream",
            hour_index: None,
        },
        PackEntry {
            path: proof_entry,
            bytes: proof_bytes,
            role: "recorder_manifest",
            hour_index: None,
        },
        PackEntry {
            path: timeline_entry,
            bytes: timeline_bytes,
            role: "timeline",
            hour_index: None,
        },
    ];
    let bytes = pack(
        game_id,
        &entries,
        &PackIdentity {
            schema_version: Some("event_timeline_change_log_live_proof_v1".to_string()),
            build_exe_sha256: None,
            build_pinned: false,
            mod_identity: None,
            created_utc: None,
        },
        true,
    )
    .map_err(|err| err.to_string())?;
    fs::write(replay_path, bytes)
        .map_err(|err| format!("failed to write {}: {err}", replay_path.display()))?;
    let container = Container::open(replay_path, &ExpectedIdentity::default())
        .map_err(|err| format!("container open failed: {err}"))?;
    container
        .verify_all()
        .map_err(|err| format!("container verify_all failed: {err}"))?;
    let read_back = container
        .read_entry_bytes(&stream_entry)
        .map_err(|err| format!("event stream readback failed: {err}"))?;
    if read_back != event_stream {
        return Err("event stream byte-identity check failed".to_string());
    }
    Ok(())
}

fn manifest_pointer_bool(manifest: &Value, path: &[&str]) -> Option<bool> {
    let mut value = manifest;
    for segment in path {
        value = value.get(*segment)?;
    }
    value.as_bool()
}

fn usage() -> String {
    "usage: event_timeline_change_log_proof <collector_stream.bin> <native_session_manifest.json> <out_dir> --game-hours <positive advancements>".to_string()
}

fn path_slash(path: &str) -> String {
    PathBuf::from(path).to_string_lossy().replace('\\', "/")
}

fn path_slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn source_summary(first: i32, last: i32) -> CompactSourceSummary {
        CompactSourceSummary {
            first_hour: Some(first),
            last_hour: Some(last),
            unique_hours: (first..=last).collect::<BTreeSet<_>>(),
            frame_end_record_count: (i64::from(last) - i64::from(first) + 1) as u64,
            ..CompactSourceSummary::default()
        }
    }

    fn healthy_manifest(advanced: i64) -> Value {
        json!({
            "game_hours_advanced": advanced,
            "unique_game_hour_count": advanced + 1,
            "skipped_game_hours": 0,
            "replay_file": "native_session.battleplan",
            "hourly_requirement": {
                "required": true,
                "met": true,
                "requested_game_hours": advanced,
                "problems": []
            },
            "compact_stream": {
                "record_magic": "H4CR",
                "record_header_len": 32,
                "record_version": 1,
                "decodable": true,
                "ordered": true,
                "worker_panic_seen": false,
                "bridge_unavailable_seen": false,
                "source_unavailable_seen": false,
                "partial_data_seen": false,
                "frame_end_record_count": advanced + 1,
                "domain_coverage": {
                    "ok": true,
                    "errors": []
                },
                "per_domain": {}
            }
        })
    }

    #[test]
    fn healthy_manifest_matches_explicit_48_advancement_contract() {
        let summary = source_summary(1_000, 1_048);
        let errors = validate_manifest_health(&healthy_manifest(48), &summary, 48);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn unhealthy_manifest_flags_fail_closed() {
        let summary = source_summary(1_000, 1_048);
        let mut manifest = healthy_manifest(48);
        manifest["compact_stream"]["decodable"] = json!(false);
        manifest["compact_stream"]["partial_data_seen"] = json!(true);
        manifest["hourly_requirement"]["problems"] = json!(["lost hour"]);
        manifest["replay_file"] = json!("");

        let errors = validate_manifest_health(&manifest, &summary, 48);
        assert!(errors.iter().any(|error| error.contains("decodable")));
        assert!(errors
            .iter()
            .any(|error| error.contains("partial_data_seen")));
        assert!(errors
            .iter()
            .any(|error| error.contains("hourly_requirement.problems")));
        assert!(errors.iter().any(|error| error.contains("replay_file")));
    }

    #[test]
    fn manifest_cannot_override_short_stream_duration() {
        let summary = source_summary(1_000, 1_047);
        let forged_long_manifest = healthy_manifest(48);
        let errors = validate_manifest_health(&forged_long_manifest, &summary, 48);
        assert!(errors
            .iter()
            .any(|error| error.contains("manifest.game_hours_advanced")));
        assert!(errors
            .iter()
            .any(|error| error.contains("does not equal explicit command 48")));
    }

    #[test]
    fn manifest_must_bind_the_exact_commanded_duration() {
        let summary = source_summary(1_000, 1_048);
        let mut manifest = healthy_manifest(48);
        manifest["hourly_requirement"]["requested_game_hours"] = json!(49);
        let errors = validate_manifest_health(&manifest, &summary, 48);
        assert!(errors
            .iter()
            .any(|error| error.contains("hourly_requirement.requested_game_hours")));
    }
}
