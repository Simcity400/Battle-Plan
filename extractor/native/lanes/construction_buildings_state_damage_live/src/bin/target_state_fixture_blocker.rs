use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use construction_buildings_state_damage_live::{
    DOMAIN_ID, DOMAIN_NAME, SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE,
};
use serde_json::{json, Value};

const REGISTRY_PATH: &str = "extractor/native/domain_registry.json";
const DEFAULT_FIXTURE_DIR: &str =
    "extractor/native/lanes/construction_buildings_state_damage_live/fixtures";

fn main() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let focus_proof_path = PathBuf::from(args.next().ok_or_else(|| {
        "usage: construction_buildings_state_damage_target_state_fixture_blocker <target_candidate_focus_probe.json> <out-dir> [fixture-dir]".to_string()
    })?);
    let out_dir = PathBuf::from(args.next().ok_or_else(|| {
        "usage: construction_buildings_state_damage_target_state_fixture_blocker <target_candidate_focus_probe.json> <out-dir> [fixture-dir]".to_string()
    })?);
    let fixture_dir = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_FIXTURE_DIR));

    fs::create_dir_all(&out_dir)
        .map_err(|err| format!("failed to create {}: {err}", out_dir.display()))?;

    let registry_path = PathBuf::from(REGISTRY_PATH);
    let registry: Value = read_json(&registry_path)?;
    let focus_proof: Value = read_json(&focus_proof_path)?;
    let registry_entry = registry_entry(&registry, DOMAIN_ID, DOMAIN_NAME);
    let fixture_inventory = fixture_inventory(&fixture_dir)?;

    let proof_schema = get_str(&focus_proof, "schema").unwrap_or("");
    let focus_verdict = get_str(&focus_proof, "verdict").unwrap_or("");
    let fixture_request = focus_proof.get("fixture_request").unwrap_or(&Value::Null);
    let requested_rows = fixture_request
        .get("rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let rows_with_placeholder = requested_rows
        .iter()
        .filter(|row| {
            row.get("expected_target_state_id").and_then(Value::as_str)
                == Some("FILL_FROM_TIME_ALIGNED_SAVE_OR_UI_ORACLE")
        })
        .count();
    let filled_matching_fixture_rows = fixture_inventory
        .iter()
        .filter_map(|entry| {
            entry
                .get("filled_matching_candidate_rows")
                .and_then(Value::as_u64)
        })
        .sum::<u64>();

    let mut errors = Vec::new();
    if registry_entry.is_none() {
        errors.push(format!(
            "registry does not assign domain id {DOMAIN_ID} to {DOMAIN_NAME}"
        ));
    }
    if proof_schema != "construction_buildings_state_damage_live_target_candidate_focus_v1" {
        errors.push(format!("unexpected focus proof schema {proof_schema:?}"));
    }
    if focus_verdict != "remaining_target_state_candidates_fixture_blocked" {
        errors.push(format!("unexpected target focus verdict {focus_verdict:?}"));
    }
    if requested_rows.len() < minimum_rows(fixture_request).unwrap_or(4) as usize {
        errors.push("fixture request does not contain enough distinguishing rows".to_string());
    }
    if rows_with_placeholder == 0 && filled_matching_fixture_rows == 0 {
        errors.push(
            "focus proof has no placeholder rows and no filled target-state fixture rows"
                .to_string(),
        );
    }

    let c0 = candidate_summary(&focus_proof, "0xc0");
    let c8 = candidate_summary(&focus_proof, "0xc8");
    let target_state_fixture_gate_ok =
        filled_matching_fixture_rows > 0 && rows_with_placeholder == 0;
    let blocker_confirmed =
        errors.is_empty() && rows_with_placeholder > 0 && filled_matching_fixture_rows == 0;

    let proof = json!({
        "schema": "construction_buildings_state_damage_live_target_state_fixture_blocker_v1",
        "domain": {
            "id": DOMAIN_ID,
            "name": DOMAIN_NAME,
            "registry": path_slash(&registry_path),
            "registry_entry": registry_entry,
        },
        "focus_proof": {
            "path": path_slash(&focus_proof_path),
            "schema": proof_schema,
            "build_profile": get_str(&focus_proof, "build_profile"),
            "verdict": focus_verdict,
            "duration_ms": get_u64(&focus_proof, "duration_ms"),
            "fingerprint": focus_proof.get("fingerprint").cloned().unwrap_or(Value::Null),
        },
        "target_state_fixture_gate": {
            "ok": target_state_fixture_gate_ok,
            "failure": if target_state_fixture_gate_ok { Value::Null } else { Value::String("missing_time_aligned_save_or_ui_oracle".to_string()) },
            "filled_matching_fixture_rows": filled_matching_fixture_rows,
            "placeholder_rows_in_live_request": rows_with_placeholder,
        },
        "blocker_confirmed": blocker_confirmed,
        "blocker": {
            "status": if blocker_confirmed { "exact_fixture_blocker" } else { "not_confirmed" },
            "required_truth_source": fixture_request.get("required_truth_source").cloned().unwrap_or(Value::Null),
            "acceptance_rule": fixture_request.get("acceptance_rule").cloned().unwrap_or(Value::Null),
            "minimum_rows": fixture_request.get("minimum_rows").cloned().unwrap_or(Value::Null),
            "candidate_offsets": fixture_request.get("candidate_offsets").cloned().unwrap_or(Value::Null),
            "distinguishing_rows_required": requested_rows,
        },
        "candidate_status": [
            c0,
            c8,
        ],
        "fixture_inventory": {
            "fixture_dir": path_slash(&fixture_dir),
            "json_files": fixture_inventory,
            "usable_fixture_present": filled_matching_fixture_rows > 0,
        },
        "fail_closed_contract": {
            "construction_target_state": "do_not_emit",
            "source_flag": "SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE",
            "source_flag_value": format!("0x{:X}", SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE),
            "candidate_offsets_not_emitted": ["subobject+0xC0", "subobject+0xC8"],
        },
        "errors": errors,
    });

    let proof_path = out_dir.join("target_state_fixture_blocker.json");
    fs::write(
        &proof_path,
        serde_json::to_string_pretty(&proof).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("failed to write {}: {err}", proof_path.display()))?;

    if !blocker_confirmed {
        return Err(format!(
            "target-state fixture blocker was not confirmed; wrote {}",
            proof_path.display()
        ));
    }
    Ok(())
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

fn registry_entry(registry: &Value, domain_id: u16, domain_name: &str) -> Option<Value> {
    registry
        .get("domains")?
        .as_array()?
        .iter()
        .find(|entry| {
            entry.get("id").and_then(Value::as_u64) == Some(domain_id.into())
                && entry.get("name").and_then(Value::as_str) == Some(domain_name)
        })
        .cloned()
}

fn candidate_summary(focus_proof: &Value, offset: &str) -> Value {
    let Some(candidate) = focus_proof
        .get("candidate_offsets")
        .and_then(Value::as_array)
        .and_then(|rows| {
            rows.iter().find(|row| {
                get_str(row, "offset").is_some_and(|value| value.eq_ignore_ascii_case(offset))
            })
        })
    else {
        return json!({
            "offset": offset,
            "status": "missing_from_focus_proof",
        });
    };

    json!({
        "offset": get_str(candidate, "offset").unwrap_or(offset),
        "status": "fixture_blocked_not_promoted_not_rejected",
        "line_count": get_u64(candidate, "line_count"),
        "state_existing_line_count": get_u64(candidate, "state_existing_line_count"),
        "unique_state_count": get_u64(candidate, "unique_state_count"),
        "unique_state_ids": candidate.get("unique_state_ids").cloned().unwrap_or(Value::Null),
        "owner_or_controller_match_count": get_u64(candidate, "owner_or_controller_match_count"),
        "province_existing_line_count": get_u64(candidate, "province_existing_line_count"),
        "province_mapped_owner_or_controller_match_count": get_u64(candidate, "province_mapped_owner_or_controller_match_count"),
        "unique_mapped_state_count": get_u64(candidate, "unique_mapped_state_count"),
        "unique_mapped_state_ids": candidate.get("unique_mapped_state_ids").cloned().unwrap_or(Value::Null),
        "unique_owner_tag_ids": candidate.get("unique_owner_tag_ids").cloned().unwrap_or(Value::Null),
        "unique_controller_tag_ids": candidate.get("unique_controller_tag_ids").cloned().unwrap_or(Value::Null),
        "unique_mapped_owner_tag_ids": candidate.get("unique_mapped_owner_tag_ids").cloned().unwrap_or(Value::Null),
        "unique_mapped_controller_tag_ids": candidate.get("unique_mapped_controller_tag_ids").cloned().unwrap_or(Value::Null),
        "verdict": get_str(candidate, "verdict"),
        "rejection_reason": candidate.get("rejection_reason").cloned().unwrap_or(Value::Null),
    })
}

fn fixture_inventory(fixture_dir: &Path) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    if !fixture_dir.exists() {
        return Ok(out);
    }
    let entries = fs::read_dir(fixture_dir).map_err(|err| {
        format!(
            "failed to read fixture dir {}: {err}",
            fixture_dir.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let parsed = read_json(&path);
        let Ok(value) = parsed else {
            out.push(json!({
                "path": path_slash(&path),
                "parse_ok": false,
                "filled_matching_candidate_rows": 0,
            }));
            continue;
        };
        let observations = value
            .get("construction_target_state_observations")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let filled_matching_candidate_rows = observations
            .iter()
            .filter(|row| expected_state_is_numeric(row) && row_has_c0_or_c8_candidate(row))
            .count();
        let candidate_offsets = observations
            .iter()
            .filter_map(candidate_offset)
            .collect::<Vec<_>>();
        out.push(json!({
            "path": path_slash(&path),
            "parse_ok": true,
            "schema": get_str(&value, "schema"),
            "construction_target_state_observations": observations.len(),
            "candidate_offsets": candidate_offsets,
            "filled_matching_candidate_rows": filled_matching_candidate_rows,
            "usable_for_c0_c8_resolution": filled_matching_candidate_rows > 0,
        }));
    }
    out.sort_by_key(|entry| {
        entry
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    });
    Ok(out)
}

fn expected_state_is_numeric(row: &Value) -> bool {
    row.get("expected_target_state_id")
        .and_then(Value::as_u64)
        .is_some_and(|state_id| state_id > 0)
}

fn row_has_c0_or_c8_candidate(row: &Value) -> bool {
    candidate_offset(row).is_some_and(|offset| {
        let offset = offset.to_ascii_lowercase();
        offset == "0xc0"
            || offset == "0xc8"
            || offset == "sub+0xc0"
            || offset == "sub+0xc8"
            || offset == "subobject+0xc0"
            || offset == "subobject+0xc8"
    })
}

fn candidate_offset(row: &Value) -> Option<String> {
    row.get("candidate")
        .and_then(|candidate| candidate.get("offset"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            row.get("candidates")
                .and_then(Value::as_object)
                .and_then(|candidates| {
                    candidates
                        .keys()
                        .find(|key| {
                            let lower = key.to_ascii_lowercase();
                            lower == "sub+0xc0" || lower == "sub+0xc8"
                        })
                        .cloned()
                })
        })
}

fn minimum_rows(fixture_request: &Value) -> Option<u64> {
    fixture_request.get("minimum_rows").and_then(Value::as_u64)
}

fn get_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn get_u64(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn path_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
