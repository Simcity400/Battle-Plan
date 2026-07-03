#!/usr/bin/env python3
"""Fail-closed tooltip parity probe for domain 44 supply residuals.

This is a live-session analysis probe, not a production emitter. It verifies
the running hoi4.exe build, parses a domain-44 compact payload, compares
candidate formulas against same-frame supply tooltip anchors, and writes
candidate/rejection evidence for exact province supply parity.
"""

from __future__ import annotations

import argparse
import json
import math
import struct
import sys
import time
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple


SCRIPT_DIR = Path(__file__).resolve().parent
MEMORY_READER = SCRIPT_DIR.parent
REPO_ROOT = MEMORY_READER.parents[1]
if str(MEMORY_READER) not in sys.path:
    sys.path.insert(0, str(MEMORY_READER))

import winmem  # type: ignore  # noqa: E402


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
DOMAIN_NAME = "supply_logistics_attrition_live"
DOMAIN_ID = 44
FIXED_SCALE_RAW = 100_000.0
MAX_PROVINCE_ID = 13_413

HEADER_STRUCT = struct.Struct("<Q11I")
ROW_STRUCT = struct.Struct("<5I5i")

ROW_KIND_PROVINCE_LOGISTICS = 1
ROW_KIND_DIVISION_SUPPLY = 2
ROW_KIND_COMPUTED_BADGE = 3
ROW_KIND_CSUPPLY_PROVINCE_INPUT = 4
ROW_KIND_FOREIGN_HOMEBASE_PRESSURE_CANDIDATE = 5


def hx(value: Optional[int]) -> Optional[str]:
    return "0x%X" % value if isinstance(value, int) else None


def now_utc() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def open_gated_process(pid: Optional[int]) -> Tuple[Optional[winmem.Process], Dict[str, Any]]:
    process = winmem.Process(pid, "hoi4.exe") if pid is not None else winmem.open_process("hoi4.exe")
    if process is None:
        return None, {
            "ok": False,
            "failure": "hoi4.exe not running or cannot be opened",
            "expected_sha256": EXPECTED_SHA256,
        }
    fp = process.fingerprint()
    fp["expected_sha256"] = EXPECTED_SHA256
    fp["build_ok"] = (fp.get("sha256") or "").lower() == EXPECTED_SHA256
    if not fp["build_ok"]:
        process.close()
        return None, {
            "ok": False,
            "failure": "build_sha_mismatch",
            "process": fp,
            "expected_sha256": EXPECTED_SHA256,
        }
    return process, {"ok": True, "process": fp}


def read_json(path: Path) -> Dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def load_anchors(path: Path) -> Tuple[List[Dict[str, Any]], Dict[str, Any]]:
    data = read_json(path)
    if (data.get("exe_sha256") or EXPECTED_SHA256).lower() != EXPECTED_SHA256:
        raise ValueError("anchor file exe_sha256 does not match expected 870ebf build")
    raw_anchors = data.get("anchors", data if isinstance(data, list) else [])
    anchors: List[Dict[str, Any]] = []
    for item in raw_anchors:
        province_id = int(item["province_id"])
        if province_id <= 0 or province_id > MAX_PROVINCE_ID:
            raise ValueError(f"invalid province_id {province_id}")
        row = dict(item)
        row["province_id"] = province_id
        anchors.append(row)
    if not anchors:
        raise ValueError("anchor file contains no anchors")
    return anchors, {
        "path": str(path),
        "contract": data.get("contract"),
        "game_date_hour": data.get("game_date_hour"),
        "source": data.get("source"),
        "anchor_count": len(anchors),
    }


def parse_payload(path: Path) -> Dict[str, Any]:
    data = path.read_bytes()
    if len(data) < HEADER_STRUCT.size:
        raise ValueError("payload shorter than domain44 header")
    header_values = HEADER_STRUCT.unpack_from(data, 0)
    header = {
        "frame_id": header_values[0],
        "row_count": header_values[1],
        "changed_rows": header_values[2],
        "status_flags": header_values[3],
        "fixed_scale_raw": header_values[4],
        "province_rows": header_values[5],
        "division_rows": header_values[6],
        "badge_rows": header_values[7],
        "province_rows_mirror": header_values[8],
        "division_rows_mirror": header_values[9],
        "csupply_input_rows": header_values[10],
        "pressure_candidate_rows": header_values[11],
    }
    expected_len = HEADER_STRUCT.size + header["row_count"] * ROW_STRUCT.size
    if len(data) != expected_len:
        raise ValueError(f"payload length {len(data)} != expected {expected_len}")
    rows = []
    cursor = HEADER_STRUCT.size
    for index in range(header["row_count"]):
        values = ROW_STRUCT.unpack_from(data, cursor)
        cursor += ROW_STRUCT.size
        rows.append({
            "row_index": index,
            "row_kind": values[0],
            "entity_id": values[1],
            "join_id": values[2],
            "owner_tag_db_id": values[3],
            "flags": values[4],
            "value_a_raw": values[5],
            "value_b_raw": values[6],
            "value_c_raw": values[7],
            "value_d_raw": values[8],
            "aux_raw": values[9],
        })
    return {"path": str(path), "header": header, "rows": rows}


def index_payload(rows: Sequence[Dict[str, Any]]) -> Dict[str, Any]:
    province_logistics: Dict[int, Dict[str, Any]] = {}
    csupply: Dict[int, Dict[str, Any]] = {}
    pressure: Dict[int, List[Dict[str, Any]]] = {}
    divisions_by_province: Dict[int, List[Dict[str, Any]]] = {}
    badges: List[Dict[str, Any]] = []
    unknown_rows = 0
    for row in rows:
        kind = row["row_kind"]
        if kind == ROW_KIND_PROVINCE_LOGISTICS:
            province_logistics[int(row["entity_id"])] = {
                "province_id": int(row["entity_id"]),
                "state_id": int(row["join_id"]),
                "rail": int(row["value_a_raw"]),
                "supply_node": int(row["value_b_raw"]),
                "naval_supply_hub": int(row["value_c_raw"]),
                "infrastructure": int(row["value_d_raw"]),
                "building_count": int(row["aux_raw"]),
            }
        elif kind == ROW_KIND_DIVISION_SUPPLY:
            division = {
                "division_id": int(row["entity_id"]),
                "province_id": int(row["join_id"]),
                "owner_tag_db_id": int(row["owner_tag_db_id"]),
                "current_supply": row["value_a_raw"] / FIXED_SCALE_RAW,
                "max_supply": row["value_b_raw"] / FIXED_SCALE_RAW,
                "supply_gain": row["value_c_raw"] / FIXED_SCALE_RAW,
                "supply_deficit": row["value_d_raw"] / FIXED_SCALE_RAW,
                "fuel_raw": int(row["aux_raw"]),
            }
            divisions_by_province.setdefault(division["province_id"], []).append(division)
        elif kind == ROW_KIND_CSUPPLY_PROVINCE_INPUT:
            csupply[int(row["entity_id"])] = {
                "province_id": int(row["entity_id"]),
                "input_value": row["value_a_raw"] / FIXED_SCALE_RAW,
                "input_raw": int(row["value_a_raw"]),
            }
        elif kind == ROW_KIND_FOREIGN_HOMEBASE_PRESSURE_CANDIDATE:
            pressure.setdefault(int(row["entity_id"]), []).append({
                "province_id": int(row["entity_id"]),
                "owner_tag_db_id": int(row["owner_tag_db_id"]),
                "supply": row["value_a_raw"] / FIXED_SCALE_RAW,
                "penalty": row["value_b_raw"] / FIXED_SCALE_RAW,
                "duplicate_province": int(row["value_c_raw"]),
                "record_offset": int(row["value_d_raw"]),
            })
        elif kind == ROW_KIND_COMPUTED_BADGE:
            badges.append(row)
        else:
            unknown_rows += 1
    return {
        "province_logistics": province_logistics,
        "divisions_by_province": divisions_by_province,
        "csupply": csupply,
        "pressure": pressure,
        "badges": badges,
        "unknown_rows": unknown_rows,
    }


def mean(values: Sequence[float]) -> Optional[float]:
    return sum(values) / len(values) if values else None


def feature_row(indexed: Dict[str, Any], province_id: int) -> Dict[str, Optional[float]]:
    logistics = indexed["province_logistics"].get(province_id, {})
    divisions = indexed["divisions_by_province"].get(province_id, [])
    current_supply = [float(row["current_supply"]) for row in divisions]
    max_supply = [float(row["max_supply"]) for row in divisions]
    pressure_rows = indexed["pressure"].get(province_id, [])
    pressure_supply = [float(row["supply"]) for row in pressure_rows]
    pressure_penalty = [float(row["penalty"]) for row in pressure_rows]
    csupply = indexed["csupply"].get(province_id, {})
    return {
        "csupply_input": csupply.get("input_value"),
        "infrastructure": float(logistics["infrastructure"]) if "infrastructure" in logistics else None,
        "rail": float(logistics["rail"]) if "rail" in logistics else None,
        "supply_node": float(logistics["supply_node"]) if "supply_node" in logistics else None,
        "naval_supply_hub": float(logistics["naval_supply_hub"]) if "naval_supply_hub" in logistics else None,
        "division_current_mean": mean(current_supply),
        "division_current_min": min(current_supply) if current_supply else None,
        "division_current_max": max(current_supply) if current_supply else None,
        "division_max_mean": mean(max_supply),
        "pressure_supply_mean": mean(pressure_supply),
        "pressure_penalty_mean": mean(pressure_penalty),
    }


def fixed_candidates(features: Dict[str, Optional[float]]) -> Dict[str, Optional[float]]:
    csupply = features.get("csupply_input")
    infra = features.get("infrastructure")
    rail = features.get("rail")
    supply_node = features.get("supply_node")
    naval_hub = features.get("naval_supply_hub")
    pressure_supply = features.get("pressure_supply_mean")
    pressure_penalty = features.get("pressure_penalty_mean")
    out: Dict[str, Optional[float]] = {
        "csupply_input_raw": csupply,
        "division_current_supply_mean": features.get("division_current_mean"),
        "division_current_supply_min": features.get("division_current_min"),
        "division_current_supply_max": features.get("division_current_max"),
        "division_max_supply_mean": features.get("division_max_mean"),
        "foreign_pressure_supply_candidate": pressure_supply,
        "foreign_pressure_supply_minus_penalty": (
            pressure_supply - pressure_penalty
            if pressure_supply is not None and pressure_penalty is not None
            else None
        ),
    }
    if infra is not None:
        out["state_infra_x0_3"] = infra * 0.3
        out["state_infra_x0_5"] = infra * 0.5
        out["state_base0_5_plus_infra_x0_3"] = 0.5 + infra * 0.3
    if csupply is not None and infra is not None:
        out["csupply_plus_state_infra_x0_3"] = csupply + infra * 0.3
        out["csupply_plus_state_base0_5_infra_x0_3"] = csupply + 0.5 + infra * 0.3
    if all(value is not None for value in (rail, supply_node, naval_hub, infra)):
        out["building_weighted_probe"] = (
            float(supply_node or 0) * 5.0
            + float(naval_hub or 0) * 3.0
            + float(rail or 0) * 0.25
            + float(infra or 0) * 0.3
        )
    return out


def score_predictions(
    anchors: Sequence[Dict[str, Any]],
    predictions_by_province: Dict[int, Optional[float]],
    target_key: str,
    tolerance: float,
) -> Dict[str, Any]:
    errors: List[float] = []
    results: List[Dict[str, Any]] = []
    missing = 0
    target_missing = 0
    matches = 0
    for anchor in anchors:
        province_id = int(anchor["province_id"])
        target = anchor.get(target_key)
        pred = predictions_by_province.get(province_id)
        if target is None:
            target_missing += 1
            continue
        if pred is None:
            missing += 1
            results.append({
                "province_id": province_id,
                "target": target,
                "predicted": None,
                "error": None,
                "match": False,
                "reason": "missing_candidate_input",
            })
            continue
        err = float(pred) - float(target)
        errors.append(err)
        ok = abs(err) <= tolerance
        matches += int(ok)
        results.append({
            "province_id": province_id,
            "target": float(target),
            "predicted": round(float(pred), 6),
            "error": round(err, 6),
            "match": ok,
        })
    comparable = len(errors)
    rmse = math.sqrt(sum(err * err for err in errors) / comparable) if comparable else None
    max_abs = max(abs(err) for err in errors) if errors else None
    if comparable == 0:
        verdict = "not_testable_missing_candidate_or_target"
    elif matches == len(anchors) - target_missing and missing == 0 and comparable >= 3:
        verdict = "all_anchors_match_requires_temporal_differential"
    elif matches:
        verdict = "rejected_partial_match"
    else:
        verdict = "rejected_no_anchor_matches"
    return {
        "target": target_key,
        "anchors": len(anchors),
        "target_missing": target_missing,
        "candidate_missing": missing,
        "comparable": comparable,
        "matches_within_tolerance": matches,
        "tolerance": tolerance,
        "rmse": round(rmse, 6) if rmse is not None else None,
        "max_abs_error": round(max_abs, 6) if max_abs is not None else None,
        "verdict": verdict,
        "results": results,
    }


def solve_linear_system(matrix: List[List[float]], vector: List[float]) -> Optional[List[float]]:
    n = len(vector)
    a = [row[:] + [vector[i]] for i, row in enumerate(matrix)]
    for col in range(n):
        pivot = max(range(col, n), key=lambda r: abs(a[r][col]))
        if abs(a[pivot][col]) < 1e-9:
            return None
        a[col], a[pivot] = a[pivot], a[col]
        div = a[col][col]
        for j in range(col, n + 1):
            a[col][j] /= div
        for r in range(n):
            if r == col:
                continue
            factor = a[r][col]
            if factor == 0:
                continue
            for j in range(col, n + 1):
                a[r][j] -= factor * a[col][j]
    return [a[i][n] for i in range(n)]


def fit_linear(
    anchors: Sequence[Dict[str, Any]],
    features_by_province: Dict[int, Dict[str, Optional[float]]],
    target_key: str,
    feature_names: Sequence[str],
) -> Dict[str, Any]:
    x_rows: List[List[float]] = []
    y_values: List[float] = []
    used_provinces: List[int] = []
    for anchor in anchors:
        province_id = int(anchor["province_id"])
        target = anchor.get(target_key)
        if target is None:
            continue
        f = features_by_province[province_id]
        row = [1.0]
        ok = True
        for name in feature_names:
            value = f.get(name)
            if value is None:
                ok = False
                break
            row.append(float(value))
        if ok:
            x_rows.append(row)
            y_values.append(float(target))
            used_provinces.append(province_id)
    cols = len(feature_names) + 1
    if len(x_rows) < cols:
        return {
            "target": target_key,
            "features": ["bias", *feature_names],
            "status": "not_enough_rows",
            "rows": len(x_rows),
            "minimum_rows": cols,
            "promotion_safe": False,
        }
    normal = [[0.0 for _ in range(cols)] for _ in range(cols)]
    rhs = [0.0 for _ in range(cols)]
    for row, y in zip(x_rows, y_values):
        for i in range(cols):
            rhs[i] += row[i] * y
            for j in range(cols):
                normal[i][j] += row[i] * row[j]
    coeffs = solve_linear_system(normal, rhs)
    if coeffs is None:
        return {
            "target": target_key,
            "features": ["bias", *feature_names],
            "status": "singular",
            "rows": len(x_rows),
            "promotion_safe": False,
        }
    predictions = {
        province_id: sum(coeffs[i] * x_rows[row_i][i] for i in range(cols))
        for row_i, province_id in enumerate(used_provinces)
    }
    score = score_predictions(
        [anchor for anchor in anchors if int(anchor["province_id"]) in predictions],
        predictions,
        target_key,
        tolerance=0.055,
    )
    return {
        "target": target_key,
        "features": ["bias", *feature_names],
        "coefficients": [round(value, 8) for value in coeffs],
        "rows": len(x_rows),
        "province_ids": used_provinces,
        "score": score,
        "status": "fit_diagnostic_only",
        "promotion_safe": False,
        "reason": "fitted coefficients are not live evidence; use only to identify missing terms",
    }


def summarize_cprovince_candidates(path: Optional[Path]) -> Dict[str, Any]:
    if path is None:
        return {"supplied": False}
    data = read_json(path)
    out: Dict[str, Any] = {
        "supplied": True,
        "path": str(path),
        "status": data.get("status"),
        "production_safe_exact_supply_fields": data.get("summary", {}).get(
            "production_safe_exact_supply_fields"
        ),
        "anchor_value_matches_requiring_differential": data.get("summary", {}).get(
            "anchor_value_matches_requiring_differential"
        ),
        "candidates": [],
    }
    for chain in data.get("current_structure_chains", []):
        for slot in chain.get("candidate_slots", [])[:4]:
            out["candidates"].append({
                "chain": chain.get("name"),
                "first_offset": chain.get("first_offset"),
                "second_offset": chain.get("second_offset"),
                "slot": slot.get("offset"),
                "verdict": slot.get("anchor_check", {}).get("verdict"),
                "remaining_matches": slot.get("anchor_check", {}).get("remaining_matches"),
                "other_component_matches": slot.get("anchor_check", {}).get(
                    "other_component_matches"
                ),
            })
    return out


def build_report(args: argparse.Namespace, gate: Dict[str, Any]) -> Dict[str, Any]:
    anchors, anchor_meta = load_anchors(Path(args.anchors))
    payload = parse_payload(Path(args.payload))
    indexed = index_payload(payload["rows"])
    feature_by_province = {
        int(anchor["province_id"]): feature_row(indexed, int(anchor["province_id"]))
        for anchor in anchors
    }

    candidate_names: Dict[str, Dict[int, Optional[float]]] = {}
    for anchor in anchors:
        province_id = int(anchor["province_id"])
        for name, value in fixed_candidates(feature_by_province[province_id]).items():
            candidate_names.setdefault(name, {})[province_id] = value

    fixed_scores = []
    for name, predictions in sorted(candidate_names.items()):
        target_scores = {
            target: score_predictions(anchors, predictions, target, args.tolerance)
            for target in ("remaining", "hubs", "state", "sum")
        }
        best_target = sorted(
            target_scores.values(),
            key=lambda row: (
                row["matches_within_tolerance"],
                -(row["candidate_missing"]),
                -(row["max_abs_error"] or 999999.0),
            ),
            reverse=True,
        )[0]
        fixed_scores.append({
            "candidate": name,
            "best_target": best_target["target"],
            "best_verdict": best_target["verdict"],
            "target_scores": target_scores,
        })
    fixed_scores.sort(
        key=lambda row: (
            row["target_scores"][row["best_target"]]["matches_within_tolerance"],
            -(row["target_scores"][row["best_target"]]["candidate_missing"]),
            -(row["target_scores"][row["best_target"]]["max_abs_error"] or 999999.0),
        ),
        reverse=True,
    )

    diagnostic_fits = [
        fit_linear(anchors, feature_by_province, "remaining", ["csupply_input"]),
        fit_linear(anchors, feature_by_province, "hubs", ["csupply_input"]),
        fit_linear(anchors, feature_by_province, "state", ["infrastructure"]),
        fit_linear(anchors, feature_by_province, "remaining", ["csupply_input", "infrastructure"]),
        fit_linear(
            anchors,
            feature_by_province,
            "remaining",
            ["csupply_input", "infrastructure", "supply_node", "naval_supply_hub", "rail"],
        ),
    ]

    exact_candidate = any(
        score["best_verdict"] == "all_anchors_match_requires_temporal_differential"
        and score["best_target"] in ("remaining", "sum")
        for score in fixed_scores
    )
    status = (
        "tooltip_candidate_matched_requires_temporal_differential"
        if exact_candidate
        else "tooltip_formula_unresolved_unextracted_terms_likely"
    )
    return {
        "schema_version": 1,
        "lane": DOMAIN_NAME,
        "probe": "tooltip_parity_probe",
        "expected_sha256": EXPECTED_SHA256,
        "started_utc": args.started_utc,
        "completed_utc": now_utc(),
        "read_only": True,
        "writes_to_game_memory": False,
        "gate": gate,
        "status": status,
        "anchor_meta": anchor_meta,
        "payload": {
            "path": payload["path"],
            "header": payload["header"],
            "row_kind_counts": row_kind_counts(payload["rows"]),
        },
        "features_by_province": feature_by_province,
        "fixed_candidate_scores": fixed_scores,
        "diagnostic_linear_fits": diagnostic_fits,
        "cprovince_candidate_summary": summarize_cprovince_candidates(args.cprovince_json),
        "candidate_rejection_rule": (
            "Any candidate that is not all-anchor within tolerance is rejected for this "
            "same-frame anchor set only; do not promote or permanently discard without "
            "a temporal/network differential."
        ),
        "likely_unextracted_terms": likely_unextracted_terms(fixed_scores),
        "promotion_safe_exact_supply_fields": 0,
    }


def row_kind_counts(rows: Sequence[Dict[str, Any]]) -> Dict[str, int]:
    names = {
        ROW_KIND_PROVINCE_LOGISTICS: "province_logistics",
        ROW_KIND_DIVISION_SUPPLY: "division_supply",
        ROW_KIND_COMPUTED_BADGE: "computed_badge",
        ROW_KIND_CSUPPLY_PROVINCE_INPUT: "csupply_province_input",
        ROW_KIND_FOREIGN_HOMEBASE_PRESSURE_CANDIDATE: "foreign_pressure_candidate",
    }
    counts: Dict[str, int] = {}
    for row in rows:
        name = names.get(row["row_kind"], f"unknown_{row['row_kind']}")
        counts[name] = counts.get(name, 0) + 1
    return counts


def likely_unextracted_terms(scores: Sequence[Dict[str, Any]]) -> List[str]:
    remaining_best = [
        row for row in scores
        if row.get("target_scores", {}).get("remaining", {}).get("comparable", 0)
    ]
    if not remaining_best:
        return [
            "No comparable remaining-supply candidate was present in the supplied domain44 payload."
        ]
    top = max(
        remaining_best,
        key=lambda row: row["target_scores"]["remaining"]["matches_within_tolerance"],
    )
    if top["target_scores"]["remaining"]["verdict"] == "all_anchors_match_requires_temporal_differential":
        return [
            "A fixed candidate matched remaining supply on this frame; still needs temporal/network differential and alias rejection."
        ]
    return [
        "Hub flow/distribution terms are not fully represented by the current CSupply +0x10 rows.",
        "State-local supply formula is not pinned by infrastructure-only rows.",
        "CProvince+0x2C8 leaf candidates and CSupplySystem sibling arrays remain the highest-priority exact-value containers.",
        "CCountrySupplySystem pressure/motorization fields may alter tooltip remaining supply and must be isolated by controlled fixtures.",
    ]


def write_outputs(out_path: Path, report: Dict[str, Any]) -> None:
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    md_path = out_path.with_suffix(".md")
    lines = [
        "# Domain 44 Tooltip Parity Probe",
        "",
        f"- status: `{report.get('status')}`",
        f"- exe_sha256: `{report.get('gate', {}).get('process', {}).get('sha256')}`",
        f"- anchors: `{report.get('anchor_meta', {}).get('anchor_count')}`",
        f"- payload: `{report.get('payload', {}).get('path')}`",
        f"- promotion_safe_exact_supply_fields: `{report.get('promotion_safe_exact_supply_fields')}`",
        "",
        "## Fixed Candidate Scores",
        "",
    ]
    for row in report.get("fixed_candidate_scores", [])[:16]:
        score = row["target_scores"][row["best_target"]]
        lines.append(
            f"- `{row['candidate']}` best `{row['best_target']}`: "
            f"`{score['verdict']}`, matches `{score['matches_within_tolerance']}`/"
            f"`{score['comparable']}`, max_abs `{score['max_abs_error']}`."
        )
    lines.extend([
        "",
        "## Likely Missing Terms",
        "",
    ])
    for item in report.get("likely_unextracted_terms", []):
        lines.append(f"- {item}")
    lines.extend([
        "",
        "## Verdict",
        "",
        "- No exact tooltip root is promoted by this probe.",
        "- Full promotion still requires same-frame anchors plus a temporal/network differential that rejects aliases for hubs, state, and sum.",
    ])
    md_path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def write_fail(out_path: Path, status: str, gate: Dict[str, Any], reason: str) -> None:
    report = {
        "schema_version": 1,
        "lane": DOMAIN_NAME,
        "probe": "tooltip_parity_probe",
        "expected_sha256": EXPECTED_SHA256,
        "completed_utc": now_utc(),
        "read_only": True,
        "writes_to_game_memory": False,
        "status": status,
        "gate": gate,
        "reason": reason,
        "promotion_safe_exact_supply_fields": 0,
    }
    write_outputs(out_path, report)


def parse_args(argv: Optional[Iterable[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, default=None, help="hoi4.exe pid; defaults to process lookup")
    parser.add_argument("--payload", required=True, type=Path, help="domain44 compact payload bin")
    parser.add_argument("--anchors", required=True, type=Path, help="same-frame tooltip anchors JSON")
    parser.add_argument("--out", required=True, type=Path, help="JSON report path")
    parser.add_argument("--cprovince-json", type=Path, default=None, help="optional cprovince residual probe JSON")
    parser.add_argument("--tolerance", type=float, default=0.055, help="tooltip float tolerance")
    args = parser.parse_args(list(argv) if argv is not None else None)
    args.started_utc = now_utc()
    return args


def main(argv: Optional[Iterable[str]] = None) -> int:
    args = parse_args(argv)
    process, gate = open_gated_process(args.pid)
    if process is None:
        write_fail(args.out, "fail_closed", gate, gate.get("failure", "build gate failed"))
        return 2
    try:
        report = build_report(args, gate)
        write_outputs(args.out, report)
        print(json.dumps({
            "status": report["status"],
            "out": str(args.out),
            "anchors": report["anchor_meta"]["anchor_count"],
            "promotion_safe_exact_supply_fields": report["promotion_safe_exact_supply_fields"],
        }, indent=2))
        return 0
    except Exception as exc:  # noqa: BLE001
        write_fail(args.out, "probe_failed", gate, str(exc))
        print(json.dumps({"status": "probe_failed", "out": str(args.out), "reason": str(exc)}, indent=2))
        return 1
    finally:
        process.close()


if __name__ == "__main__":
    raise SystemExit(main())
