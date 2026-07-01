use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use event_timeline_change_log_live::{
    encode_event_compact_stream, gate_event_compact_stream, gate_event_families,
    gate_missing_source_adapters, missing_source_adapter_contracts, source_domain_name,
    summarize_compact_source_stream, CompactSourceSummary, Confidence, EventKind, EventRecord,
    SourceDomainUnitSnapshot, SourceKind, EVENT_FLAG_DERIVED, EVENT_FLAG_DIRECT_CAPTURE,
    EVENT_FLAG_PROOF_FIXTURE, SOURCE_DOMAIN_DECISIONS, SOURCE_DOMAIN_DIPLOMACY,
    SOURCE_DOMAIN_FOCUS, SOURCE_DOMAIN_LAND_COMBATS, SOURCE_DOMAIN_MAP_CONTROL, SOURCE_DOMAIN_TECH,
};
use serde_json::{json, Value};
use snapshot::{pack, Container, ExpectedIdentity, PackEntry, PackIdentity};

fn main() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let stream_path = args.next().ok_or_else(usage)?;
    let manifest_path = args.next().ok_or_else(usage)?;
    let out_dir = args.next().ok_or_else(usage)?;
    if args.next().is_some() {
        return Err(usage());
    }

    fs::create_dir_all(&out_dir).map_err(|err| format!("failed to create {out_dir}: {err}"))?;
    let stream = fs::read(&stream_path)
        .map_err(|err| format!("failed to read compact stream {stream_path}: {err}"))?;
    let manifest_text = fs::read_to_string(&manifest_path)
        .map_err(|err| format!("failed to read manifest {manifest_path}: {err}"))?;
    let manifest: Value = serde_json::from_str(&manifest_text)
        .map_err(|err| format!("failed to parse manifest {manifest_path}: {err}"))?;

    let source_summary = summarize_compact_source_stream(&stream);
    let families = gate_event_families(&source_summary);
    let source_domain_units = source_domain_units_from_manifest(&manifest, &source_summary);
    let missing_source_adapter_contracts = missing_source_adapter_contracts();
    let missing_source_adapter_gates = gate_missing_source_adapters(&source_domain_units);
    let first_hour = source_summary.first_hour.unwrap_or(0);
    let last_hour = source_summary.last_hour.unwrap_or(first_hour);
    let source_sequence = source_summary.record_count.max(1);
    let event_rows = proof_event_rows(first_hour, last_hour, source_sequence);
    let event_stream = encode_event_compact_stream(last_hour, &event_rows);
    let event_gate = gate_event_compact_stream(&event_stream);
    if !event_gate.ok {
        return Err(format!(
            "event contract gate failed before writing proof: {:?}",
            event_gate.payload_errors
        ));
    }

    let compact_path = Path::new(&out_dir).join("event_timeline_change_log_live.compact.bin");
    fs::write(&compact_path, &event_stream)
        .map_err(|err| format!("failed to write {}: {err}", compact_path.display()))?;

    let game_id = manifest
        .get("replay_game_id")
        .or_else(|| manifest.get("session_id"))
        .and_then(Value::as_str)
        .unwrap_or("event_timeline_change_log_live_proof");
    let source_manifest_checks = json!({
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
        "completion_basis": "lane-local event verification infrastructure and compact contract proof",
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
    let container_result = pack_and_verify(game_id, &event_stream, &proof, &replay_path);
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

fn proof_event_rows(first_hour: i32, last_hour: i32, source_sequence: u64) -> Vec<EventRecord> {
    let mut focus = EventRecord::new(
        EventKind::FocusCompleted,
        SOURCE_DOMAIN_FOCUS,
        SourceKind::DerivedDelta,
        Confidence::Exact,
        last_hour,
    )
    .with_keys(
        Some("TAG_HASH_PROOF"),
        None,
        Some("focus_id_hash_proof"),
        None,
        Some("completed"),
    )
    .with_source_position(source_sequence, 0)
    .mark_proof_fixture();
    focus.flags |= EVENT_FLAG_DERIVED | EVENT_FLAG_PROOF_FIXTURE;

    let mut tech = EventRecord::new(
        EventKind::TechResearched,
        SOURCE_DOMAIN_TECH,
        SourceKind::DerivedDelta,
        Confidence::Exact,
        last_hour,
    )
    .with_keys(
        Some("TAG_HASH_PROOF"),
        None,
        Some("tech_id_hash_proof"),
        None,
        Some("researched"),
    )
    .with_source_position(source_sequence, 1)
    .mark_proof_fixture();
    tech.flags |= EVENT_FLAG_DERIVED | EVENT_FLAG_PROOF_FIXTURE;

    let mut decision = EventRecord::new(
        EventKind::DecisionExpired,
        SOURCE_DOMAIN_DECISIONS,
        SourceKind::DerivedDelta,
        Confidence::Exact,
        last_hour,
    )
    .with_keys(
        Some("TAG_HASH_PROOF"),
        None,
        Some("decision_id_hash_proof"),
        Some("active"),
        Some("expired"),
    )
    .with_source_position(source_sequence, 2)
    .mark_proof_fixture();
    decision.flags |= EVENT_FLAG_DERIVED | EVENT_FLAG_PROOF_FIXTURE;

    let mut war = EventRecord::new(
        EventKind::WarDeclared,
        SOURCE_DOMAIN_DIPLOMACY,
        SourceKind::DerivedDelta,
        Confidence::Interval,
        last_hour,
    )
    .with_keys(
        Some("ATTACKER_HASH_PROOF"),
        Some("DEFENDER_HASH_PROOF"),
        Some("war_pair_hash_proof"),
        None,
        Some("at_war"),
    )
    .with_source_position(source_sequence, 3)
    .mark_proof_fixture();
    war.flags |= EVENT_FLAG_DERIVED | EVENT_FLAG_PROOF_FIXTURE;
    war.interval_start_hour_raw = first_hour;

    let mut combat = EventRecord::new(
        EventKind::CombatStarted,
        SOURCE_DOMAIN_LAND_COMBATS,
        SourceKind::DirectCaptured,
        Confidence::Inferred,
        last_hour,
    )
    .with_keys(
        Some("ATTACKER_HASH_PROOF"),
        Some("DEFENDER_HASH_PROOF"),
        Some("combat_identity_hash_proof"),
        None,
        None,
    )
    .with_source_position(source_sequence, 4)
    .with_generation(1)
    .with_detail("direct land_combat source record provenance")
    .mark_proof_fixture();
    combat.flags |= EVENT_FLAG_DIRECT_CAPTURE | EVENT_FLAG_PROOF_FIXTURE;

    let mut province = EventRecord::new(
        EventKind::ProvinceControllerChange,
        SOURCE_DOMAIN_MAP_CONTROL,
        SourceKind::DerivedDelta,
        Confidence::Exact,
        last_hour,
    )
    .with_keys(
        Some("NEW_CONTROLLER_HASH_PROOF"),
        None,
        Some("province_id_hash_proof"),
        Some("old_controller"),
        Some("new_controller"),
    )
    .with_source_position(source_sequence, 5)
    .mark_proof_fixture();
    province.flags |= EVENT_FLAG_DERIVED | EVENT_FLAG_PROOF_FIXTURE;

    vec![focus, tech, decision, war, combat, province]
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
    "usage: event_timeline_change_log_proof <collector_stream.bin> <native_session_manifest.json> <out_dir>".to_string()
}

fn path_slash(path: &str) -> String {
    PathBuf::from(path).to_string_lossy().replace('\\', "/")
}

fn path_slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
