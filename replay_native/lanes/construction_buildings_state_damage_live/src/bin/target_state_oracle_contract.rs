use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use construction_buildings_state_damage_live::{
    DOMAIN_ID, DOMAIN_NAME, SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE,
};
use serde_json::{json, Value};

const PLACEHOLDER: &str = "FILL_FROM_TIME_ALIGNED_SAVE_OR_UI_ORACLE";

fn main() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let focus_proof_path = PathBuf::from(args.next().ok_or_else(|| {
        "usage: construction_buildings_state_damage_target_state_oracle_contract <target_candidate_focus_probe.json> <out-dir> [filled-oracle.json]".to_string()
    })?);
    let out_dir = PathBuf::from(args.next().ok_or_else(|| {
        "usage: construction_buildings_state_damage_target_state_oracle_contract <target_candidate_focus_probe.json> <out-dir> [filled-oracle.json]".to_string()
    })?);
    let oracle_path = args.next().map(PathBuf::from);

    fs::create_dir_all(&out_dir)
        .map_err(|err| format!("failed to create {}: {err}", out_dir.display()))?;

    let focus_proof = read_json(&focus_proof_path)?;
    let focus_schema = get_str(&focus_proof, "schema").unwrap_or("");
    if focus_schema != "construction_buildings_state_damage_live_target_candidate_focus_v1" {
        return Err(format!("unexpected focus proof schema {focus_schema:?}"));
    }

    let mut contract = build_contract(&focus_proof_path, &focus_proof);
    let oracle = if let Some(path) = &oracle_path {
        let value = read_json(path)?;
        merge_oracle_rows(&mut contract, &value);
        Some((path.clone(), value))
    } else {
        None
    };

    let contract_path = out_dir.join("target_state_oracle_contract.json");
    fs::write(
        &contract_path,
        serde_json::to_string_pretty(&contract).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("failed to write {}: {err}", contract_path.display()))?;

    let evaluation = evaluate_contract(&contract);
    let missing_oracle = evaluation.status == "missing_oracle";
    let terminal = matches!(
        evaluation.status.as_str(),
        "validated_subobject_0xc0"
            | "validated_subobject_0xc8"
            | "validated_subobject_0xc0_raw_province_id_mapped_state"
            | "validated_subobject_0xc8_raw_province_id_mapped_state"
            | "rejected_subobject_0xc0_and_0xc8"
    );
    let proof = json!({
        "schema": "construction_buildings_state_damage_live_target_state_oracle_contract_gate_v1",
        "domain": {
            "id": DOMAIN_ID,
            "name": DOMAIN_NAME,
        },
        "focus_proof": {
            "path": path_slash(&focus_proof_path),
            "build_profile": get_str(&focus_proof, "build_profile"),
            "fingerprint": focus_proof.get("fingerprint").cloned().unwrap_or(Value::Null),
            "verdict": get_str(&focus_proof, "verdict"),
        },
        "contract": {
            "path": path_slash(&contract_path),
            "schema": get_str(&contract, "schema"),
            "required_rows": contract
                .get("required_rows")
                .and_then(Value::as_array)
                .map(|rows| rows.len())
                .unwrap_or(0),
            "oracle_input": oracle
                .as_ref()
                .map(|(path, _)| Value::String(path_slash(path)))
                .unwrap_or(Value::Null),
        },
        "evaluation": {
            "status": evaluation.status,
            "blocker_confirmed": missing_oracle,
            "terminal_candidate_result": terminal,
            "filled_rows": evaluation.filled_rows,
            "placeholder_rows": evaluation.placeholder_rows,
            "distinguishing_rows": evaluation.distinguishing_rows,
            "c0_matches": evaluation.c0_matches,
            "c8_matches": evaluation.c8_matches,
            "neither_matches": evaluation.neither_matches,
            "no_candidate_interpretation_matches": evaluation.no_candidate_interpretation_matches,
            "mixed_or_inconclusive_rows": evaluation.mixed_or_inconclusive_rows,
            "unique_expected_target_states": evaluation.unique_expected_target_states,
            "candidate_interpretation_matches": evaluation.candidate_interpretation_matches,
            "winning_candidate_interpretation": evaluation.winning_candidate_interpretation,
            "result_detail": evaluation.result_detail,
        },
        "fail_closed_contract": {
            "construction_target_state": "omit_until_terminal_candidate_result",
            "source_flag": "SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE",
            "source_flag_value": format!("0x{:X}", SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE),
            "partial_target_state_allowed": false,
        },
    });

    let proof_path = out_dir.join("target_state_oracle_contract_gate.json");
    fs::write(
        &proof_path,
        serde_json::to_string_pretty(&proof).map_err(|err| err.to_string())?,
    )
    .map_err(|err| format!("failed to write {}: {err}", proof_path.display()))?;

    if missing_oracle || terminal {
        Ok(())
    } else {
        Err(format!(
            "target-state oracle contract inconclusive; wrote {}",
            proof_path.display()
        ))
    }
}

fn build_contract(focus_path: &Path, focus_proof: &Value) -> Value {
    let fixture_request = focus_proof
        .get("fixture_request")
        .cloned()
        .unwrap_or(Value::Null);
    let rows = fixture_request
        .get("rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    json!({
        "schema": "construction_buildings_state_damage_live_target_state_oracle_contract_v1",
        "domain": {
            "id": DOMAIN_ID,
            "name": DOMAIN_NAME,
        },
        "source_focus_proof": path_slash(focus_path),
        "build_profile": get_str(focus_proof, "build_profile"),
        "fingerprint": focus_proof.get("fingerprint").cloned().unwrap_or(Value::Null),
        "current_candidate_summary": focus_proof.get("candidate_offsets").cloned().unwrap_or(Value::Null),
        "required_game_action": [
            "Use the same current-build 870ebf the game session or immediately rerun target_candidate_focus_probe.py after creating the fixture.",
            "Keep the game paused while collecting the live proof and the oracle observation.",
            "Preferred controlled fixture: in the construction UI, queue at least four building lines in at least two distinct known states, then immediately run the focused live probe and record the visible target state for those exact rows.",
            "Acceptable current-frame fixture: use the listed live rows and fill each expected_target_state_id from a save produced from the same paused frame or from an equivalent UI observation."
        ],
        "required_save_or_ui_observation": [
            "For every required row, provide a numeric expected_target_state_id from the save production/general_lines construction queue or from an unambiguous UI state observation.",
            "Rows must be matched by tag_db_id, line_id, priority, template_id, active_factories, cost_raw, and produced_raw. If any key differs, rerun the live probe and regenerate this contract.",
            "A state name is useful for human audit, but the executable gate requires the numeric state id."
        ],
        "required_fields_per_row": [
            "tag_db_id",
            "line_id",
            "priority",
            "template_id",
            "active_factories",
            "cost_raw",
            "produced_raw",
            "candidates.sub+0xC0.candidate_state_id",
            "candidates.sub+0xC0.candidate_mapped_state_id",
            "candidates.sub+0xC8.candidate_state_id",
            "candidates.sub+0xC8.candidate_mapped_state_id",
            "expected_target_state_id"
        ],
        "pass_fail_rule": {
            "minimum_filled_rows": 4,
            "minimum_unique_expected_target_states": 2,
            "validate_subobject_0xC0": "All filled distinguishing rows have expected_target_state_id == candidates.sub+0xC0.candidate_state_id and at least two unique expected states.",
            "validate_subobject_0xC0_province_mapped": "All filled distinguishing rows have expected_target_state_id == candidates.sub+0xC0.candidate_mapped_state_id and at least two unique expected states.",
            "validate_subobject_0xC8": "All filled distinguishing rows have expected_target_state_id == candidates.sub+0xC8.candidate_state_id and at least two unique expected states.",
            "validate_subobject_0xC8_province_mapped": "All filled distinguishing rows have expected_target_state_id == candidates.sub+0xC8.candidate_mapped_state_id and at least two unique expected states.",
            "reject_all": "All filled distinguishing rows have expected_target_state_id different from every candidate interpretation.",
            "inconclusive": "Any mixed match pattern, unchanged line-key mismatch, fewer than four filled rows, or fewer than two unique expected states."
        },
        "required_rows": rows,
    })
}

fn merge_oracle_rows(contract: &mut Value, oracle: &Value) {
    let Some(contract_rows) = contract
        .get_mut("required_rows")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    let oracle_rows = oracle_rows(oracle);
    for row in contract_rows {
        let Some(match_row) = oracle_rows
            .iter()
            .find(|candidate| row_keys_match(row, candidate))
        else {
            continue;
        };
        if let Some(expected) = match_row.get("expected_target_state_id").cloned() {
            row.as_object_mut()
                .expect("required row is object")
                .insert("expected_target_state_id".to_string(), expected);
        }
        for key in ["expected_target_state_name", "truth_source", "oracle_note"] {
            if let Some(value) = match_row.get(key).cloned() {
                row.as_object_mut()
                    .expect("required row is object")
                    .insert(key.to_string(), value);
            }
        }
    }
}

fn oracle_rows(oracle: &Value) -> Vec<Value> {
    oracle
        .get("required_rows")
        .or_else(|| oracle.get("rows"))
        .or_else(|| oracle.get("construction_target_state_observations"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn row_keys_match(left: &Value, right: &Value) -> bool {
    for key in [
        "tag_db_id",
        "line_id",
        "priority",
        "template_id",
        "active_factories",
        "cost_raw",
        "produced_raw",
    ] {
        if get_u64(left, key) != get_u64(right, key) {
            return false;
        }
    }
    true
}

#[derive(Debug, PartialEq, Eq)]
struct Evaluation {
    status: String,
    filled_rows: usize,
    placeholder_rows: usize,
    distinguishing_rows: usize,
    c0_matches: usize,
    c8_matches: usize,
    neither_matches: usize,
    no_candidate_interpretation_matches: usize,
    mixed_or_inconclusive_rows: usize,
    unique_expected_target_states: Vec<u64>,
    candidate_interpretation_matches: BTreeMap<String, usize>,
    winning_candidate_interpretation: Option<String>,
    result_detail: String,
}

fn evaluate_contract(contract: &Value) -> Evaluation {
    let rows = contract
        .get("required_rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut placeholder_rows = 0usize;
    let mut filled_rows = 0usize;
    let mut distinguishing_rows = 0usize;
    let mut c0_matches = 0usize;
    let mut c8_matches = 0usize;
    let mut neither_matches = 0usize;
    let mut no_candidate_interpretation_matches = 0usize;
    let mut mixed_or_inconclusive_rows = 0usize;
    let mut expected_states = BTreeSet::new();
    let mut candidate_interpretation_matches = BTreeMap::new();

    for row in rows {
        if row.get("expected_target_state_id").and_then(Value::as_str) == Some(PLACEHOLDER) {
            placeholder_rows += 1;
            continue;
        }
        let Some(expected) = get_u64(&row, "expected_target_state_id") else {
            mixed_or_inconclusive_rows += 1;
            continue;
        };
        filled_rows += 1;
        expected_states.insert(expected);
        let c0 = candidate_state(&row, "sub+0xC0");
        let c8 = candidate_state(&row, "sub+0xC8");
        let interpretations = candidate_interpretations(&row);
        let unique_candidate_states: BTreeSet<u64> = interpretations
            .iter()
            .map(|(_, state_id)| *state_id)
            .collect();
        if unique_candidate_states.len() > 1 || (c0.is_some() && c8.is_some() && c0 != c8) {
            distinguishing_rows += 1;
        }
        match (c0 == Some(expected), c8 == Some(expected)) {
            (true, false) => c0_matches += 1,
            (false, true) => c8_matches += 1,
            (false, false) => neither_matches += 1,
            (true, true) => mixed_or_inconclusive_rows += 1,
        }
        let mut matched_any_interpretation = false;
        for (label, state_id) in interpretations {
            if state_id == expected {
                *candidate_interpretation_matches.entry(label).or_insert(0) += 1;
                matched_any_interpretation = true;
            }
        }
        if !matched_any_interpretation {
            no_candidate_interpretation_matches += 1;
        }
    }

    let unique_expected_target_states: Vec<u64> = expected_states.into_iter().collect();
    let minimum_rows = contract
        .get("pass_fail_rule")
        .and_then(|rule| get_u64(rule, "minimum_filled_rows"))
        .unwrap_or(4) as usize;
    let minimum_unique_states = contract
        .get("pass_fail_rule")
        .and_then(|rule| get_u64(rule, "minimum_unique_expected_target_states"))
        .unwrap_or(2) as usize;

    let winning_candidate_interpretation = candidate_interpretation_matches
        .iter()
        .find(|(_, count)| **count == filled_rows)
        .map(|(label, _)| label.clone());

    let (status, result_detail) = if filled_rows == 0 && placeholder_rows > 0 {
        (
            "missing_oracle".to_string(),
            "No numeric expected_target_state_id rows were provided; a time-aligned save/UI oracle is required.".to_string(),
        )
    } else if filled_rows < minimum_rows {
        (
            "inconclusive".to_string(),
            format!("filled_rows {filled_rows} < required {minimum_rows}"),
        )
    } else if unique_expected_target_states.len() < minimum_unique_states {
        (
            "inconclusive".to_string(),
            format!(
                "unique expected target states {} < required {minimum_unique_states}",
                unique_expected_target_states.len()
            ),
        )
    } else if distinguishing_rows < minimum_rows {
        (
            "inconclusive".to_string(),
            format!("distinguishing_rows {distinguishing_rows} < required {minimum_rows}"),
        )
    } else if c0_matches == filled_rows && c8_matches == 0 && neither_matches == 0 {
        (
            "validated_subobject_0xc0".to_string(),
            "All filled distinguishing rows match subobject+0xC0.".to_string(),
        )
    } else if c8_matches == filled_rows && c0_matches == 0 && neither_matches == 0 {
        (
            "validated_subobject_0xc8".to_string(),
            "All filled distinguishing rows match subobject+0xC8.".to_string(),
        )
    } else if let Some(label) = &winning_candidate_interpretation {
        (
            validation_status_for_label(label),
            format!("All filled distinguishing rows match {label}."),
        )
    } else if no_candidate_interpretation_matches == filled_rows {
        (
            "rejected_subobject_0xc0_and_0xc8".to_string(),
            "All filled distinguishing rows differ from every candidate interpretation."
                .to_string(),
        )
    } else {
        (
            "inconclusive".to_string(),
            "Filled rows have a mixed c0/c8/neither match pattern.".to_string(),
        )
    };

    Evaluation {
        status,
        filled_rows,
        placeholder_rows,
        distinguishing_rows,
        c0_matches,
        c8_matches,
        neither_matches,
        no_candidate_interpretation_matches,
        mixed_or_inconclusive_rows,
        unique_expected_target_states,
        candidate_interpretation_matches,
        winning_candidate_interpretation,
        result_detail,
    }
}

fn candidate_state(row: &Value, key: &str) -> Option<u64> {
    row.get("candidates")
        .and_then(|candidates| candidates.get(key))
        .and_then(|candidate| candidate.get("candidate_state_id"))
        .and_then(Value::as_u64)
}

fn candidate_interpretations(row: &Value) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    for key in ["sub+0xC0", "sub+0xC8"] {
        let Some(candidate) = row
            .get("candidates")
            .and_then(|candidates| candidates.get(key))
        else {
            continue;
        };
        if let Some(state_id) = candidate.get("candidate_state_id").and_then(Value::as_u64) {
            out.push((format!("{key}.raw_state_id"), state_id));
        }
        if let Some(state_id) = candidate
            .get("candidate_mapped_state_id")
            .and_then(Value::as_u64)
        {
            out.push((format!("{key}.raw_province_id_mapped_state"), state_id));
        }
        if let Some(interpretations) = candidate
            .get("candidate_interpretations")
            .and_then(Value::as_object)
        {
            if let Some(state_id) = interpretations
                .get("raw_state_id")
                .and_then(|value| value.get("candidate_state_id"))
                .and_then(Value::as_u64)
            {
                let label = format!("{key}.raw_state_id");
                if !out.iter().any(|(existing, _)| existing == &label) {
                    out.push((label, state_id));
                }
            }
            if let Some(state_id) = interpretations
                .get("raw_province_id_mapped_state")
                .and_then(|value| value.get("candidate_state_id"))
                .and_then(Value::as_u64)
            {
                let label = format!("{key}.raw_province_id_mapped_state");
                if !out.iter().any(|(existing, _)| existing == &label) {
                    out.push((label, state_id));
                }
            }
        }
    }
    out
}

fn validation_status_for_label(label: &str) -> String {
    match label {
        "sub+0xC0.raw_state_id" => "validated_subobject_0xc0".to_string(),
        "sub+0xC8.raw_state_id" => "validated_subobject_0xc8".to_string(),
        "sub+0xC0.raw_province_id_mapped_state" => {
            "validated_subobject_0xc0_raw_province_id_mapped_state".to_string()
        }
        "sub+0xC8.raw_province_id_mapped_state" => {
            "validated_subobject_0xc8_raw_province_id_mapped_state".to_string()
        }
        _ => format!(
            "validated_{}",
            label.replace(['+', '.'], "_").to_ascii_lowercase()
        ),
    }
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

fn get_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn get_u64(value: &Value, key: &str) -> Option<u64> {
    let value = value.get(key)?;
    if let Some(number) = value.as_u64() {
        return Some(number);
    }
    value.as_str()?.parse().ok()
}

fn path_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_missing_oracle_as_exact_blocker() {
        let contract = json!({
            "pass_fail_rule": {
                "minimum_filled_rows": 4,
                "minimum_unique_expected_target_states": 2,
            },
            "required_rows": [
                row(10, 31, PLACEHOLDER),
                row(11, 32, PLACEHOLDER),
                row(12, 33, PLACEHOLDER),
                row(13, 34, PLACEHOLDER),
            ],
        });

        let eval = evaluate_contract(&contract);

        assert_eq!(eval.status, "missing_oracle");
        assert_eq!(eval.placeholder_rows, 4);
    }

    #[test]
    fn validates_c0_only_when_all_filled_rows_match_c0() {
        let contract = json!({
            "pass_fail_rule": {
                "minimum_filled_rows": 4,
                "minimum_unique_expected_target_states": 2,
            },
            "required_rows": [
                row(10, 31, 10),
                row(11, 32, 11),
                row(12, 33, 12),
                row(13, 34, 13),
            ],
        });

        let eval = evaluate_contract(&contract);

        assert_eq!(eval.status, "validated_subobject_0xc0");
        assert_eq!(eval.c0_matches, 4);
        assert_eq!(eval.c8_matches, 0);
    }

    #[test]
    fn rejects_both_when_no_filled_rows_match_either_candidate() {
        let contract = json!({
            "pass_fail_rule": {
                "minimum_filled_rows": 4,
                "minimum_unique_expected_target_states": 2,
            },
            "required_rows": [
                row(10, 31, 101),
                row(11, 32, 102),
                row(12, 33, 103),
                row(13, 34, 104),
            ],
        });

        let eval = evaluate_contract(&contract);

        assert_eq!(eval.status, "rejected_subobject_0xc0_and_0xc8");
        assert_eq!(eval.neither_matches, 4);
    }

    #[test]
    fn validates_province_mapped_state_candidate_before_rejecting() {
        let contract = json!({
            "pass_fail_rule": {
                "minimum_filled_rows": 4,
                "minimum_unique_expected_target_states": 2,
            },
            "required_rows": [
                row_with_c8_mapped_state(10, 31, 400, 400),
                row_with_c8_mapped_state(11, 15, 213, 213),
                row_with_c8_mapped_state(12, 31, 400, 400),
                row_with_c8_mapped_state(13, 15, 213, 213),
            ],
        });

        let eval = evaluate_contract(&contract);

        assert_eq!(
            eval.status,
            "validated_subobject_0xc8_raw_province_id_mapped_state"
        );
        assert_eq!(eval.neither_matches, 4);
        assert_eq!(eval.no_candidate_interpretation_matches, 0);
        assert_eq!(
            eval.winning_candidate_interpretation.as_deref(),
            Some("sub+0xC8.raw_province_id_mapped_state")
        );
    }

    fn row(c0: u64, c8: u64, expected: impl Into<Value>) -> Value {
        json!({
            "tag_db_id": 1,
            "line_id": c0 + c8,
            "priority": c0,
            "template_id": 20002,
            "active_factories": 1,
            "cost_raw": 720000000u64,
            "produced_raw": 1u64,
            "candidates": {
                "sub+0xC0": { "candidate_state_id": c0 },
                "sub+0xC8": { "candidate_state_id": c8 },
            },
            "expected_target_state_id": expected.into(),
        })
    }

    fn row_with_c8_mapped_state(c0: u64, c8: u64, c8_mapped: u64, expected: u64) -> Value {
        json!({
            "tag_db_id": 1,
            "line_id": c0 + c8 + c8_mapped,
            "priority": c0,
            "template_id": 20002,
            "active_factories": 1,
            "cost_raw": 720000000u64,
            "produced_raw": 1u64,
            "candidates": {
                "sub+0xC0": { "candidate_state_id": c0 },
                "sub+0xC8": {
                    "candidate_state_id": c8,
                    "candidate_mapped_state_id": c8_mapped,
                    "candidate_interpretations": {
                        "raw_state_id": {
                            "candidate_state_id": c8
                        },
                        "raw_province_id_mapped_state": {
                            "candidate_state_id": c8_mapped,
                            "candidate_province_id": c8
                        }
                    }
                },
            },
            "expected_target_state_id": expected,
        })
    }
}
