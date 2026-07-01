use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use construction_buildings_state_damage_live::{
    encode_lane_frame, gate_lane_frame, BuildingRow, ConstructionQueueRow, LaneFrame,
    StateResourceRow, BUILD_PROFILE, DOMAIN_ID, DOMAIN_NAME, MAX_ACTIVE_FACTORIES,
    MAX_CONSTRUCTION_TEMPLATE_ID, MAX_STATE_ID, SCOPE_PROVINCE, SCOPE_STATE,
    SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE, SOURCE_FLAG_DLL_NATIVE,
    SOURCE_FLAG_HEALTHY_LEVEL_CANDIDATE, SOURCE_FLAG_SHARED_COUNTRY_ROOTS,
    SOURCE_FLAG_SHARED_MAP_TOPOLOGY, STATE_RESOURCE_MAX_ABS_RAW, STATE_RESOURCE_NO_ID,
};
use serde_json::{json, Value};

fn main() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let fixture_path = PathBuf::from(args.next().ok_or_else(|| {
        "usage: construction_buildings_state_damage_fixture_gate <fixture.json> <out-dir>"
            .to_string()
    })?);
    let out_dir = PathBuf::from(args.next().ok_or_else(|| {
        "usage: construction_buildings_state_damage_fixture_gate <fixture.json> <out-dir>"
            .to_string()
    })?);
    fs::create_dir_all(&out_dir)
        .map_err(|err| format!("failed to create {}: {err}", out_dir.display()))?;

    let fixture_text = fs::read_to_string(&fixture_path)
        .map_err(|err| format!("failed to read {}: {err}", fixture_path.display()))?;
    let fixture: Value = serde_json::from_str(&fixture_text)
        .map_err(|err| format!("fixture is not valid JSON: {err}"))?;

    let mut errors = Vec::new();
    let schema = get_str(&fixture, "schema").unwrap_or("");
    if schema != "construction_buildings_state_damage_live_fixture_contract_v1" {
        errors.push(format!("unexpected fixture schema {schema:?}"));
    }
    let build_profile = get_str(&fixture, "build_profile").unwrap_or("");
    if build_profile != BUILD_PROFILE {
        errors.push(format!(
            "fixture build_profile {build_profile:?} != expected {BUILD_PROFILE:?}"
        ));
    }

    let require_target_state =
        get_bool_path(&fixture, &["requires", "construction_target_state"], true);
    let require_damaged_building = get_bool_path(&fixture, &["requires", "damaged_building"], true);

    let construction_rows =
        parse_construction_observations(&fixture, require_target_state, &mut errors);
    let building_rows =
        parse_damaged_building_observations(&fixture, require_damaged_building, &mut errors);
    let resource_rows = parse_resource_observations(&fixture, &mut errors);
    let target_unavailable_flag = if require_target_state {
        0
    } else {
        SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE
    };

    let frame = LaneFrame {
        game_hour_raw: get_i64(&fixture, "game_hour_raw").unwrap_or(0) as i32,
        status_flags: 0,
        source_flags: SOURCE_FLAG_DLL_NATIVE
            | SOURCE_FLAG_SHARED_COUNTRY_ROOTS
            | SOURCE_FLAG_SHARED_MAP_TOPOLOGY
            | SOURCE_FLAG_HEALTHY_LEVEL_CANDIDATE
            | target_unavailable_flag,
        observed_country_roots: get_u64(&fixture, "observed_country_roots").unwrap_or(1) as u32,
        observed_state_roots: get_u64(&fixture, "observed_state_roots").unwrap_or(1) as u32,
        observed_province_roots: get_u64(&fixture, "observed_province_roots").unwrap_or(1) as u32,
        construction_rows,
        building_rows,
        resource_rows,
    };

    let compact = encode_lane_frame(&frame);
    let compact_gate = gate_lane_frame(&compact);
    if !compact_gate.ok {
        errors.extend(
            compact_gate
                .errors
                .iter()
                .map(|error| format!("compact gate: {error}")),
        );
    }
    let damaged_rows = frame
        .building_rows
        .iter()
        .filter(|row| row.healthy_level.is_some_and(|healthy| healthy < row.level))
        .count();
    if require_damaged_building && damaged_rows == 0 {
        errors.push("fixture does not contain a damaged building row".to_string());
    }
    if require_target_state
        && !frame
            .construction_rows
            .iter()
            .any(|row| row.target_state_id.is_some())
    {
        errors
            .push("fixture does not contain a validated construction target_state_id".to_string());
    }

    let compact_path = out_dir.join("fixture_contract.compact.bin");
    fs::write(&compact_path, &compact)
        .map_err(|err| format!("failed to write {}: {err}", compact_path.display()))?;
    let proof = json!({
        "schema": "construction_buildings_state_damage_live_fixture_gate_result_v1",
        "domain": {
            "id": DOMAIN_ID,
            "name": DOMAIN_NAME,
            "build_profile": BUILD_PROFILE,
            "fixture": path_slash(&fixture_path),
            "compact_bin": path_slash(&compact_path),
        },
        "gate_ok": errors.is_empty(),
        "requirements": {
            "construction_target_state": require_target_state,
            "damaged_building": require_damaged_building,
        },
        "observed": {
            "construction_rows": frame.construction_rows.len(),
            "construction_rows_with_target_state": frame.construction_rows.iter().filter(|row| row.target_state_id.is_some()).count(),
            "building_rows": frame.building_rows.len(),
            "damaged_building_rows": damaged_rows,
            "resource_rows": frame.resource_rows.len(),
        },
        "compact_gate": {
            "ok": compact_gate.ok,
            "status_flags": compact_gate.status_flags,
            "source_flags": compact_gate.source_flags,
            "construction_rows": compact_gate.construction_rows,
            "construction_rows_with_target_state": compact_gate.construction_rows_with_target_state,
            "building_rows": compact_gate.building_rows,
            "resource_rows": compact_gate.resource_rows,
            "damaged_building_rows": compact_gate.damaged_building_rows,
            "errors": compact_gate.errors,
        },
        "target_state_badge": {
            "status": if require_target_state { "fixture_requires_validated_target_state" } else { "unavailable_fixture_blocked" },
            "source_flag": "SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE",
            "source_flag_value": format_hex_u32(SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE),
            "flag_set": target_unavailable_flag != 0,
        },
        "errors": errors,
    });
    let proof_path = out_dir.join("fixture_contract_gate.json");
    fs::write(
        &proof_path,
        serde_json::to_string_pretty(&proof).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("failed to write {}: {err}", proof_path.display()))?;

    if proof["gate_ok"] != Value::Bool(true) {
        return Err(format!(
            "fixture gate failed; wrote {}",
            proof_path.display()
        ));
    }
    Ok(())
}

fn parse_construction_observations(
    fixture: &Value,
    required: bool,
    errors: &mut Vec<String>,
) -> Vec<ConstructionQueueRow> {
    let Some(rows) = fixture
        .get("construction_target_state_observations")
        .and_then(Value::as_array)
    else {
        if required {
            errors.push("missing construction_target_state_observations".to_string());
        }
        return Vec::new();
    };
    if required && rows.is_empty() {
        errors.push("construction_target_state_observations is empty".to_string());
    }
    let mut out = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let tag_db_id = required_u32(row, "tag_db_id", errors, index, "construction");
        let line_id = required_u32(row, "line_id", errors, index, "construction");
        let priority = required_u32(row, "priority", errors, index, "construction");
        let template_id = optional_u32(row, "template_id");
        let active_factories = get_i64(row, "active_factories").unwrap_or(0) as i32;
        let cost_raw = get_i64(row, "cost_raw").unwrap_or(0) as i32;
        let produced_raw = get_i64(row, "produced_raw").unwrap_or(0) as i32;
        let expected_state = required_u32(
            row,
            "expected_target_state_id",
            errors,
            index,
            "construction",
        );
        let mapped_state = row
            .get("candidate")
            .and_then(|candidate| get_u64(candidate, "mapped_state_id"))
            .map(|value| value as u32);
        if let (Some(expected), Some(mapped)) = (expected_state, mapped_state) {
            if expected != mapped {
                errors.push(format!(
                    "construction observation {index} candidate mapped_state_id {mapped} != expected_target_state_id {expected}"
                ));
            }
        } else if required {
            errors.push(format!(
                "construction observation {index} missing candidate.mapped_state_id"
            ));
        }
        if let Some(state_id) = expected_state {
            if state_id == 0 || state_id > MAX_STATE_ID {
                errors.push(format!(
                    "construction observation {index} invalid expected_target_state_id {state_id}"
                ));
            }
        }
        if let Some(template_id) = template_id {
            if template_id == 0 || template_id > MAX_CONSTRUCTION_TEMPLATE_ID {
                errors.push(format!(
                    "construction observation {index} invalid template_id {template_id}"
                ));
            }
        }
        if !(0..=MAX_ACTIVE_FACTORIES).contains(&active_factories) {
            errors.push(format!(
                "construction observation {index} invalid active_factories {active_factories}"
            ));
        }
        if cost_raw < 0 || produced_raw < 0 {
            errors.push(format!(
                "construction observation {index} has negative cost/progress"
            ));
        }
        if let (Some(tag_db_id), Some(line_id), Some(priority), Some(target_state_id)) =
            (tag_db_id, line_id, priority, expected_state)
        {
            out.push(ConstructionQueueRow {
                tag_db_id,
                line_id,
                template_id,
                active_factories,
                cost_raw,
                produced_raw,
                priority,
                target_state_id: Some(target_state_id),
            });
        }
    }
    out
}

fn parse_damaged_building_observations(
    fixture: &Value,
    required: bool,
    errors: &mut Vec<String>,
) -> Vec<BuildingRow> {
    let Some(rows) = fixture
        .get("damaged_building_observations")
        .and_then(Value::as_array)
    else {
        if required {
            errors.push("missing damaged_building_observations".to_string());
        }
        return Vec::new();
    };
    if required && rows.is_empty() {
        errors.push("damaged_building_observations is empty".to_string());
    }
    let mut out = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let scope = match get_str(row, "scope") {
            Some("state") => Some(SCOPE_STATE),
            Some("province") => Some(SCOPE_PROVINCE),
            Some(other) => {
                errors.push(format!(
                    "building observation {index} invalid scope {other:?}"
                ));
                None
            }
            None => {
                errors.push(format!("building observation {index} missing scope"));
                None
            }
        };
        let location_id = required_u32(row, "location_id", errors, index, "building");
        let state_id = required_u32(row, "state_id", errors, index, "building");
        let type_id = required_u32(row, "type_id", errors, index, "building");
        let level = required_u32(row, "level_at_0x40", errors, index, "building");
        let healthy = required_u32(row, "healthy_level_at_0x42", errors, index, "building");
        let expected_damaged = get_bool(row, "expected_damaged").unwrap_or(false);
        if let (Some(level), Some(healthy)) = (level, healthy) {
            if healthy > level {
                errors.push(format!(
                    "building observation {index} healthy_level_at_0x42 {healthy} > level_at_0x40 {level}"
                ));
            }
            if expected_damaged && healthy >= level {
                errors.push(format!(
                    "building observation {index} expected damaged but healthy_level_at_0x42 {healthy} >= level_at_0x40 {level}"
                ));
            }
            if !expected_damaged && healthy != level {
                errors.push(format!(
                    "building observation {index} expected undamaged but healthy_level_at_0x42 {healthy} != level_at_0x40 {level}"
                ));
            }
        }
        if let (Some(scope), Some(location_id), Some(state_id), Some(type_id), Some(level)) =
            (scope, location_id, state_id, type_id, level)
        {
            out.push(BuildingRow {
                scope,
                location_id,
                state_id,
                type_id,
                level,
                healthy_level: healthy,
            });
        }
    }
    out
}

fn parse_resource_observations(fixture: &Value, errors: &mut Vec<String>) -> Vec<StateResourceRow> {
    let Some(rows) = fixture
        .get("resource_observations")
        .and_then(Value::as_array)
    else {
        errors.push(
            "missing resource_observations; compact gate requires at least one resource row"
                .to_string(),
        );
        return Vec::new();
    };
    if rows.is_empty() {
        errors.push(
            "resource_observations is empty; compact gate requires at least one row".to_string(),
        );
    }
    let mut out = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let state_id = required_u32(row, "state_id", errors, index, "resource");
        let resource_id = required_u32(row, "resource_id", errors, index, "resource");
        let amount_raw = get_i64(row, "amount_raw").unwrap_or(-1) as i32;
        if let Some(resource_id) = resource_id {
            if resource_id == 0 || resource_id == STATE_RESOURCE_NO_ID {
                errors.push(format!(
                    "resource observation {index} invalid resource_id {resource_id}"
                ));
            }
        }
        if !(0..=STATE_RESOURCE_MAX_ABS_RAW).contains(&amount_raw) {
            errors.push(format!(
                "resource observation {index} invalid amount_raw {amount_raw}"
            ));
        }
        if let (Some(state_id), Some(resource_id)) = (state_id, resource_id) {
            out.push(StateResourceRow {
                state_id,
                resource_id,
                amount_raw,
            });
        }
    }
    out
}

fn required_u32(
    value: &Value,
    key: &str,
    errors: &mut Vec<String>,
    index: usize,
    row_kind: &str,
) -> Option<u32> {
    let parsed = get_u64(value, key).and_then(|value| u32::try_from(value).ok());
    if parsed.is_none() {
        errors.push(format!("{row_kind} observation {index} missing {key}"));
    }
    parsed
}

fn optional_u32(value: &Value, key: &str) -> Option<u32> {
    get_u64(value, key).and_then(|value| u32::try_from(value).ok())
}

fn get_bool_path(value: &Value, path: &[&str], default: bool) -> bool {
    let mut cursor = value;
    for key in path {
        let Some(next) = cursor.get(*key) else {
            return default;
        };
        cursor = next;
    }
    cursor.as_bool().unwrap_or(default)
}

fn get_bool(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

fn get_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn get_i64(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(Value::as_i64)
}

fn get_u64(value: &Value, key: &str) -> Option<u64> {
    let value = value.get(key)?;
    if let Some(number) = value.as_u64() {
        return Some(number);
    }
    parse_u64_string(value.as_str()?)
}

fn parse_u64_string(value: &str) -> Option<u64> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).ok()
    } else {
        value.parse().ok()
    }
}

fn format_hex_u32(value: u32) -> String {
    format!("0x{value:X}")
}

fn path_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
