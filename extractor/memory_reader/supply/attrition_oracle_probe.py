#!/usr/bin/env python3
"""UI/save-oracle attrition probe for domain 44 residuals.

This probe verifies the running hoi4.exe build, samples validated division
bodies, and cross-checks scalar/byte candidates against a same-frame UI oracle
listing divisions with and without visible attrition icons/reasons. It also
summarizes any save-oracle attrition fields, but it does not promote attrition
state without per-division same-frame evidence and negative controls.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple


SCRIPT_DIR = Path(__file__).resolve().parent
MEMORY_READER = SCRIPT_DIR.parent
REPO_ROOT = MEMORY_READER.parents[1]
LANE_DIR = REPO_ROOT / "extractor" / "native" / "lanes" / "supply_logistics_attrition_live"
for path in (MEMORY_READER, LANE_DIR):
    if str(path) not in sys.path:
        sys.path.insert(0, str(path))

import attrition_residual_probe as attrition_probe  # type: ignore  # noqa: E402
import live_probe  # type: ignore  # noqa: E402
import winmem  # type: ignore  # noqa: E402


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
DOMAIN_NAME = live_probe.DOMAIN_NAME
FIXED_SCALE_RAW = float(live_probe.FIXED_SCALE_RAW)
HEADER_STRUCT = struct.Struct("<Q11I")
ROW_STRUCT = struct.Struct("<5I5i")
ROW_KIND_DIVISION_SUPPLY = 2
ROW_KIND_CSUPPLY_PROVINCE_INPUT = 4
DEFAULT_SCAN_BYTES = 0x900
MAX_REPORTED = 128


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


def normalize_division_list(items: Sequence[Dict[str, Any]], role: str) -> List[Dict[str, Any]]:
    rows: List[Dict[str, Any]] = []
    for item in items:
        div_id = item.get("division_id")
        if not isinstance(div_id, int) or div_id <= 0:
            continue
        row = dict(item)
        row["division_id"] = div_id
        row["oracle_role"] = role
        rows.append(row)
    return rows


def load_ui_oracle(path: Path) -> Dict[str, Any]:
    data = read_json(path)
    if (data.get("exe_sha256") or EXPECTED_SHA256).lower() != EXPECTED_SHA256:
        raise ValueError("ui oracle exe_sha256 does not match expected 870ebf build")
    positives = normalize_division_list(
        data.get("positive_divisions") or data.get("attrition_divisions") or [],
        "positive",
    )
    negatives = normalize_division_list(
        data.get("negative_divisions") or data.get("control_divisions") or [],
        "negative",
    )
    if not positives:
        raise ValueError("ui oracle needs at least one positive attrition division")
    if len(negatives) < 2:
        raise ValueError("ui oracle needs at least two negative control divisions")
    return {
        "path": str(path),
        "game_date_hour": data.get("game_date_hour"),
        "source": data.get("source"),
        "positive_divisions": positives,
        "negative_divisions": negatives,
        "positive_count": len(positives),
        "negative_count": len(negatives),
    }


def known_value_aliases(row: Dict[str, Any]) -> Dict[str, int]:
    return {
        "province_id": int(row.get("province_id") or 0),
        "owner_tag_db_id": int(row.get("owner_tag_db_id") or 0),
        "current_supply_raw": int(row.get("current_supply_raw") or 0),
        "max_supply_raw": int(row.get("max_supply_raw") or 0),
        "supply_gain_raw": int(row.get("supply_gain_raw") or 0),
        "supply_deficit_raw": int(row.get("supply_deficit_raw") or 0),
        "fuel_raw_low32": int(row.get("fuel_raw") or 0) & 0xFFFFFFFF,
    }


def candidate_alias(values_by_division: Dict[int, int], divisions: Dict[int, Dict[str, Any]]) -> Optional[str]:
    for alias in (
        "province_id",
        "owner_tag_db_id",
        "current_supply_raw",
        "max_supply_raw",
        "supply_gain_raw",
        "supply_deficit_raw",
        "fuel_raw_low32",
    ):
        ok = True
        for div_id, value in values_by_division.items():
            if known_value_aliases(divisions[div_id]).get(alias) != value:
                ok = False
                break
        if ok:
            return alias
    return None


def classify_offset(
    offset: int,
    width: int,
    values_by_division: Dict[int, int],
    positives: Set[int],
    negatives: Set[int],
    divisions: Dict[int, Dict[str, Any]],
    oracle_rows: Dict[int, Dict[str, Any]],
) -> Dict[str, Any]:
    pos_values = [values_by_division[div_id] for div_id in positives if div_id in values_by_division]
    neg_values = [values_by_division[div_id] for div_id in negatives if div_id in values_by_division]
    all_values = pos_values + neg_values
    alias = candidate_alias(values_by_division, divisions)
    pos_nonzero = sum(1 for value in pos_values if value != 0)
    neg_nonzero = sum(1 for value in neg_values if value != 0)
    pos_set = sorted(set(pos_values))
    neg_set = sorted(set(neg_values))
    reason_values: Dict[str, List[int]] = defaultdict(list)
    for div_id in positives:
        reason = str(oracle_rows.get(div_id, {}).get("reason") or "unspecified")
        if div_id in values_by_division:
            reason_values[reason].append(values_by_division[div_id])
    reason_distinct = {
        reason: sorted(set(values))
        for reason, values in sorted(reason_values.items())
    }
    if alias:
        verdict = "rejected_alias_" + alias
    elif not all_values or len(set(all_values)) < 2:
        verdict = "rejected_constant_or_unreadable"
    elif pos_nonzero == len(pos_values) and neg_nonzero == 0:
        verdict = "candidate_positive_nonzero_negative_zero_requires_fixture_diff"
    elif set(pos_values).isdisjoint(set(neg_values)):
        verdict = "candidate_positive_negative_disjoint_requires_fixture_diff"
    elif pos_nonzero > 0 and neg_nonzero > 0:
        verdict = "rejected_positive_negative_overlap"
    elif pos_nonzero == 0:
        verdict = "rejected_positive_zero"
    else:
        verdict = "rejected_unclassified_overlap"
    samples = []
    for div_id in sorted(values_by_division):
        row = divisions[div_id]
        samples.append({
            "division_id": div_id,
            "role": "positive" if div_id in positives else "negative",
            "value": values_by_division[div_id],
            "province_id": row.get("province_id"),
            "current_supply": round((row.get("current_supply_raw") or 0) / FIXED_SCALE_RAW, 5),
            "supply_deficit": round((row.get("supply_deficit_raw") or 0) / FIXED_SCALE_RAW, 5),
            "reason": oracle_rows.get(div_id, {}).get("reason"),
        })
    return {
        "offset": "+0x%X" % offset,
        "width": width,
        "encoding": "u8" if width == 1 else "u32",
        "positive_values": pos_set,
        "negative_values": neg_set,
        "positive_nonzero": pos_nonzero,
        "negative_nonzero": neg_nonzero,
        "reason_distinct_values": reason_distinct,
        "alias": alias,
        "verdict": verdict,
        "samples": samples,
        "promotion_safe": False,
    }


def scan_division_candidates(
    p: Any,
    divisions: Dict[int, Dict[str, Any]],
    oracle: Dict[str, Any],
    scan_bytes: int,
) -> Dict[str, Any]:
    oracle_rows = {
        int(row["division_id"]): row
        for row in oracle["positive_divisions"] + oracle["negative_divisions"]
    }
    positives = {int(row["division_id"]) for row in oracle["positive_divisions"]}
    negatives = {int(row["division_id"]) for row in oracle["negative_divisions"]}
    selected = positives | negatives
    blobs: Dict[int, bytes] = {}
    missing: List[int] = []
    for div_id in selected:
        row = divisions.get(div_id)
        if not row:
            missing.append(div_id)
            continue
        blob = p.try_read(int(row["body"]), scan_bytes)
        if len(blob) >= scan_bytes:
            blobs[div_id] = blob
        else:
            missing.append(div_id)
    candidates: List[Dict[str, Any]] = []
    rejected: List[Dict[str, Any]] = []
    known_offsets = set(attrition_probe.KNOWN_DIVISION_OFFSETS)
    for width in (1, 4):
        step = 1 if width == 1 else 4
        for offset in range(0, scan_bytes - width + 1, step):
            if width == 4 and offset in known_offsets:
                continue
            values_by_division: Dict[int, int] = {}
            for div_id, blob in blobs.items():
                if width == 1:
                    value = blob[offset]
                else:
                    value = struct.unpack_from("<I", blob, offset)[0]
                    if value > 4096 and value not in known_value_aliases(divisions[div_id]).values():
                        continue
                values_by_division[div_id] = int(value)
            if len(values_by_division) < max(2, len(selected) - len(missing)):
                continue
            row = classify_offset(
                offset,
                width,
                values_by_division,
                positives,
                negatives,
                divisions,
                oracle_rows,
            )
            if row["verdict"].startswith("candidate_"):
                candidates.append(row)
            elif row["verdict"] not in ("rejected_constant_or_unreadable",):
                rejected.append(row)
    candidates.sort(
        key=lambda row: (
            row["verdict"] == "candidate_positive_nonzero_negative_zero_requires_fixture_diff",
            row["positive_nonzero"],
            -row["negative_nonzero"],
        ),
        reverse=True,
    )
    rejected.sort(key=lambda row: (row["verdict"], row["offset"], row["encoding"]))
    return {
        "selected_divisions": sorted(selected),
        "missing_or_unreadable_divisions": sorted(missing),
        "scan_bytes": scan_bytes,
        "candidate_offsets": candidates,
        "candidate_offset_count": len(candidates),
        "rejected_candidates": rejected,
        "rejected_candidate_count": len(rejected),
        "rejection_reason_counts": dict(Counter(row["verdict"] for row in rejected)),
    }


def parse_domain44_payload(path: Optional[Path]) -> Dict[str, Any]:
    if path is None:
        return {"supplied": False}
    data = path.read_bytes()
    if len(data) < HEADER_STRUCT.size:
        raise ValueError("domain44 payload shorter than header")
    header_values = HEADER_STRUCT.unpack_from(data, 0)
    row_count = header_values[1]
    expected = HEADER_STRUCT.size + row_count * ROW_STRUCT.size
    if len(data) != expected:
        raise ValueError(f"domain44 payload length {len(data)} != expected {expected}")
    division_rows: Dict[int, Dict[str, Any]] = {}
    csupply_by_province: Dict[int, float] = {}
    cursor = HEADER_STRUCT.size
    for _idx in range(row_count):
        values = ROW_STRUCT.unpack_from(data, cursor)
        cursor += ROW_STRUCT.size
        kind, entity_id, join_id, owner, _flags, a, b, c, d, aux = values
        if kind == ROW_KIND_DIVISION_SUPPLY:
            division_rows[int(entity_id)] = {
                "province_id": int(join_id),
                "owner_tag_db_id": int(owner),
                "current_supply": a / FIXED_SCALE_RAW,
                "max_supply": b / FIXED_SCALE_RAW,
                "supply_gain": c / FIXED_SCALE_RAW,
                "supply_deficit": d / FIXED_SCALE_RAW,
                "fuel_raw": aux,
            }
        elif kind == ROW_KIND_CSUPPLY_PROVINCE_INPUT:
            csupply_by_province[int(entity_id)] = a / FIXED_SCALE_RAW
    return {
        "supplied": True,
        "path": str(path),
        "row_count": row_count,
        "division_rows": division_rows,
        "csupply_by_province": csupply_by_province,
    }


def summarize_supply_crosscheck(
    oracle: Dict[str, Any],
    divisions: Dict[int, Dict[str, Any]],
    payload: Dict[str, Any],
) -> Dict[str, Any]:
    rows = []
    for item in oracle["positive_divisions"] + oracle["negative_divisions"]:
        div_id = int(item["division_id"])
        live = divisions.get(div_id, {})
        province_id = live.get("province_id")
        payload_div = payload.get("division_rows", {}).get(div_id, {}) if payload.get("supplied") else {}
        rows.append({
            "division_id": div_id,
            "role": item["oracle_role"],
            "reason": item.get("reason"),
            "province_id": province_id,
            "live_current_supply": (
                round((live.get("current_supply_raw") or 0) / FIXED_SCALE_RAW, 5)
                if live else None
            ),
            "live_supply_deficit": (
                round((live.get("supply_deficit_raw") or 0) / FIXED_SCALE_RAW, 5)
                if live else None
            ),
            "payload_current_supply": payload_div.get("current_supply"),
            "payload_supply_deficit": payload_div.get("supply_deficit"),
            "csupply_input_at_province": (
                payload.get("csupply_by_province", {}).get(int(province_id))
                if payload.get("supplied") and isinstance(province_id, int)
                else None
            ),
        })
    return {
        "payload_supplied": payload.get("supplied", False),
        "rows": rows,
        "verdict": (
            "supply_rows_are_pressure_context_not_attrition_reason"
            if rows
            else "no_oracle_rows"
        ),
    }


def collect_attrition_paths(obj: Any, path: str = "") -> List[Dict[str, Any]]:
    rows: List[Dict[str, Any]] = []
    if isinstance(obj, dict):
        for key, value in obj.items():
            child = f"{path}.{key}" if path else str(key)
            if "attrition" in str(key).lower():
                rows.append({"path": child, "value_type": type(value).__name__, "sample": value if not isinstance(value, (dict, list)) else None})
            rows.extend(collect_attrition_paths(value, child))
    elif isinstance(obj, list):
        for index, value in enumerate(obj[:64]):
            rows.extend(collect_attrition_paths(value, f"{path}[{index}]"))
    return rows


def summarize_save_oracle(path: Optional[Path]) -> Dict[str, Any]:
    if path is None:
        return {"supplied": False}
    data = read_json(path)
    attrition_paths = collect_attrition_paths(data)
    countries = data.get("countries") or {}
    country_loss_rows = []
    for tag, rec in countries.items():
        attrition_total = rec.get("lost_trucks_attrition_total")
        killed_total = rec.get("lost_trucks_killed_total")
        if attrition_total or killed_total:
            country_loss_rows.append({
                "tag": tag,
                "lost_trucks_attrition_total": attrition_total,
                "lost_trucks_killed_total": killed_total,
            })
    return {
        "supplied": True,
        "path": str(path),
        "save_date": data.get("meta", {}).get("save_date"),
        "attrition_key_paths_sample": attrition_paths[:MAX_REPORTED],
        "attrition_key_path_count": len(attrition_paths),
        "country_loss_rows": country_loss_rows[:MAX_REPORTED],
        "country_loss_row_count": len(country_loss_rows),
        "per_division_attrition_rows_found": any(
            "division" in row["path"].lower() and "attrition" in row["path"].lower()
            for row in attrition_paths
        ),
        "verdict": (
            "save_oracle_has_country_attrition_context_only"
            if country_loss_rows
            else "save_oracle_has_no_attrition_context"
        ),
    }


def build_report(args: argparse.Namespace, p: Any, gate: Dict[str, Any]) -> Dict[str, Any]:
    oracle = load_ui_oracle(args.ui_oracle)
    game_state = p.u64(p.module_base + live_probe.CGAMESTATE_RVA)
    divisions_list, division_summary = attrition_probe._division_bodies(p, p.module_base, game_state)
    divisions = {int(row["division_id"]): row for row in divisions_list}
    scan = scan_division_candidates(p, divisions, oracle, args.scan_bytes)
    payload = parse_domain44_payload(args.payload)
    save_summary = summarize_save_oracle(args.supply_json)
    has_candidate = scan["candidate_offset_count"] > 0
    status = (
        "attrition_candidates_require_fixture_diff"
        if has_candidate
        else "attrition_oracle_rejected_simple_division_scalars"
    )
    return {
        "schema_version": 1,
        "lane": DOMAIN_NAME,
        "probe": "attrition_oracle_probe",
        "expected_sha256": EXPECTED_SHA256,
        "started_utc": args.started_utc,
        "completed_utc": now_utc(),
        "read_only": True,
        "writes_to_game_memory": False,
        "status": status,
        "gate": gate,
        "ui_oracle": oracle,
        "division_summary": division_summary,
        "roots_validated": {
            "cgame_state": "module+0x3316E90 -> CCurrentGameState",
            "division": "CGameState+0x310 country table -> country+0x290 wrapper vector -> wrapper+0x10 body",
            "division_vtable": "module+0x2943CB0",
        },
        "division_body_scan": scan,
        "supply_crosscheck": summarize_supply_crosscheck(oracle, divisions, payload),
        "save_oracle_summary": save_summary,
        "promotion_safe_attrition_fields": 0,
        "promotion_rule": (
            "A field is promotable only after the same division is observed before/after "
            "attrition reason changes, positives and negative controls remain stable, and "
            "the value is proven not to alias supply, fuel, movement, combat, template, or UI cache."
        ),
    }


def write_outputs(out_path: Path, report: Dict[str, Any]) -> None:
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    scan = report.get("division_body_scan", {})
    lines = [
        "# Domain 44 Attrition Oracle Probe",
        "",
        f"- status: `{report.get('status')}`",
        f"- exe_sha256: `{report.get('gate', {}).get('process', {}).get('sha256')}`",
        f"- positives/negatives: `{report.get('ui_oracle', {}).get('positive_count')}` / `{report.get('ui_oracle', {}).get('negative_count')}`",
        f"- candidate_offsets: `{scan.get('candidate_offset_count')}`",
        f"- rejected_candidates: `{scan.get('rejected_candidate_count')}`",
        f"- promotion_safe_attrition_fields: `{report.get('promotion_safe_attrition_fields')}`",
        "",
        "## Candidate Offsets",
        "",
    ]
    for row in scan.get("candidate_offsets", [])[:16]:
        lines.append(
            f"- `{row.get('offset')}` `{row.get('encoding')}`: `{row.get('verdict')}`, "
            f"positive `{row.get('positive_values')}`, negative `{row.get('negative_values')}`."
        )
    if not scan.get("candidate_offsets"):
        lines.append("- none")
    lines.extend([
        "",
        "## Rejection Summary",
        "",
    ])
    for reason, count in sorted(scan.get("rejection_reason_counts", {}).items()):
        lines.append(f"- `{reason}`: `{count}` offsets.")
    lines.extend([
        "",
        "## Save Oracle",
        "",
        f"- verdict: `{report.get('save_oracle_summary', {}).get('verdict')}`",
        f"- per_division_attrition_rows_found: `{report.get('save_oracle_summary', {}).get('per_division_attrition_rows_found')}`",
        "",
        "## Verdict",
        "",
        "- No attrition reason/flag row is promoted by this probe.",
        "- Supply and CSupply rows are pressure context only; they are not exact attrition reasons.",
    ])
    out_path.with_suffix(".md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def write_fail(out_path: Path, status: str, gate: Dict[str, Any], reason: str) -> None:
    report = {
        "schema_version": 1,
        "lane": DOMAIN_NAME,
        "probe": "attrition_oracle_probe",
        "expected_sha256": EXPECTED_SHA256,
        "completed_utc": now_utc(),
        "read_only": True,
        "writes_to_game_memory": False,
        "status": status,
        "gate": gate,
        "reason": reason,
        "promotion_safe_attrition_fields": 0,
    }
    write_outputs(out_path, report)


def parse_args(argv: Optional[Iterable[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, default=None, help="hoi4.exe pid; defaults to process lookup")
    parser.add_argument("--ui-oracle", required=True, type=Path, help="same-frame visible attrition oracle JSON")
    parser.add_argument("--out", required=True, type=Path, help="JSON report path")
    parser.add_argument("--payload", type=Path, default=None, help="optional domain44 compact payload bin")
    parser.add_argument("--supply-json", type=Path, default=None, help="optional save supply oracle JSON")
    parser.add_argument("--scan-bytes", type=lambda raw: int(raw, 0), default=DEFAULT_SCAN_BYTES)
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
        report = build_report(args, process, gate)
        write_outputs(args.out, report)
        print(json.dumps({
            "status": report["status"],
            "out": str(args.out),
            "candidate_offsets": report["division_body_scan"]["candidate_offset_count"],
            "promotion_safe_attrition_fields": report["promotion_safe_attrition_fields"],
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
