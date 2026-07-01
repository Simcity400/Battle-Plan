"""Validate/reject battleplan geometry candidates with save/live order evidence.

This lane-local probe uses the melted save's `orders_group.order_instance`
blocks as an exact geometry oracle only after live order rows are paired by
army id, slot order, and order type. The known type-5 `states` vector at
`order+0xE0/+0xE8` is the positive control for the pairing.
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import sys
import time
from typing import Any


REPO = Path(__file__).resolve().parents[3]
MEMORY_READER = REPO / "memory_reader"
SAVE_ORACLE = REPO / "save_oracle"
LANE_DIR = Path(__file__).resolve().parent
for path in (str(REPO), str(MEMORY_READER), str(SAVE_ORACLE), str(LANE_DIR)):
    if path not in sys.path:
        sys.path.insert(0, path)

import battleplan_army_scalar_probe as scalar_probe  # type: ignore  # noqa: E402
import battleplan_candidate_probe as candidate_probe  # type: ignore  # noqa: E402
import direct_oob_probe  # type: ignore  # noqa: E402
import winmem  # type: ignore  # noqa: E402
from save_oracle import parse_save as ps  # type: ignore  # noqa: E402


SCHEMA = "army_oob_orders_battleplans_save_geometry_probe_v1"
PROC_NAME = direct_oob_probe.PROC_NAME
EXPECTED_SHA256 = direct_oob_probe.EXPECTED_SHA256
ORDER_OFF_PATH_BEGIN = 0x70
ORDER_OFF_PATH_SIZE = 0x78
ORDER_OFF_TYPE3_SECONDARY_BEGIN = 0xC0
ORDER_OFF_TYPE3_SECONDARY_SIZE = 0xC8
MAX_PROVINCE_ID = 65_535
MIN_TYPE5_EXACT_CONTROL_ROWS = 64
MIN_PATH70_EXACT_ROWS = 32


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int)
    ap.add_argument("--save", type=Path, default=scalar_probe.default_autosave_path())
    ap.add_argument("--rakaly", type=Path)
    ap.add_argument("--out-dir", type=Path, required=True)
    args = ap.parse_args()

    payload = run(args.pid, args.save, args.rakaly, args.out_dir)
    view = {
        "status": "PASS" if not payload["errors"] else "FAIL",
        "errors": payload["errors"],
        "candidate_status": payload["candidate_status"],
        "pairing": payload["pairing_summary"],
        "positive_control": summarize_candidate(payload["positive_control"]),
        "candidate_results": {
            row["name"]: summarize_candidate(row)
            for row in payload["candidate_results"]
        },
    }
    print(json.dumps(view, indent=2, sort_keys=True))
    return 0 if not payload["errors"] else 1


def summarize_candidate(row: dict[str, Any]) -> dict[str, Any]:
    return {
        "status": row.get("status"),
        "tested_rows": row.get("tested_rows"),
        "valid_rows": row.get("valid_rows"),
        "exact_rows": row.get("exact_rows"),
        "exact_fraction": row.get("exact_fraction"),
        "compact_rows": len(row.get("compact_rows", [])),
    }


def run(
    pid: int | None,
    save_path: Path,
    rakaly_path: Path | None,
    out_dir: Path,
) -> dict[str, Any]:
    if pid is None:
        pid = winmem.find_pid(PROC_NAME)
    if pid is None:
        raise RuntimeError("hoi4.exe not running")
    out_dir.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    text, save_meta = melt_save(save_path, rakaly_path, out_dir)
    save_orders = parse_save_orders_groups(text)

    proc = winmem.Process(pid, PROC_NAME)
    try:
        fingerprint = proc.fingerprint()
        sha = str(fingerprint.get("sha256") or "").lower()
        if sha != EXPECTED_SHA256:
            raise RuntimeError(f"build gate failed: {sha} != {EXPECTED_SHA256}")
        roots, scan_stats = direct_oob_probe.enum_current_oob_roots(proc)
        live_orders = candidate_probe.enumerate_validated_orders(proc, roots["army"])
        live_by_army: dict[int, list[Any]] = defaultdict(list)
        for order in live_orders:
            live_by_army[int(order.army_id)].append(order)

        pairs, pairing_summary = pair_save_live_orders(save_orders, live_by_army)
        positive_control = evaluate_type5_positive_control(proc, pairs)
        path70 = evaluate_path_candidate(
            proc,
            pairs,
            name="non_type5_path_vector_plus_0x70",
            begin_off=ORDER_OFF_PATH_BEGIN,
            size_off=ORDER_OFF_PATH_SIZE,
            applies_to={1, 2, 3},
            min_exact_rows=MIN_PATH70_EXACT_ROWS,
        )
        type3_c0 = evaluate_path_candidate(
            proc,
            pairs,
            name="type3_secondary_vector_plus_0xC0",
            begin_off=ORDER_OFF_TYPE3_SECONDARY_BEGIN,
            size_off=ORDER_OFF_TYPE3_SECONDARY_SIZE,
            applies_to={3},
            min_exact_rows=1,
            reject_when_zero_exact=True,
        )
        candidate_results = [path70, type3_c0]
        candidate_status = classify_candidate_results(positive_control, candidate_results)
        payload = {
            "schema": SCHEMA,
            "created_utc": datetime.now(timezone.utc).isoformat(),
            "pid": pid,
            "exe_sha256": sha,
            "module_base": proc.module_base,
            "module_path": proc.module_path,
            "elapsed_s": time.monotonic() - started,
            "save": save_meta,
            "scan_stats": scan_stats,
            "save_order_groups": len(save_orders),
            "live_armies_with_orders": len(live_by_army),
            "live_order_rows": len(live_orders),
            "pairing_summary": pairing_summary,
            "positive_control": positive_control,
            "candidate_results": candidate_results,
            "candidate_status": candidate_status,
            "offsets": {
                "type5_states_begin": direct_oob_probe.ORDER_OFF_TYPE5_STATES_BEGIN,
                "type5_states_size": direct_oob_probe.ORDER_OFF_TYPE5_STATES_SIZE,
                "path_begin": ORDER_OFF_PATH_BEGIN,
                "path_size": ORDER_OFF_PATH_SIZE,
                "type3_secondary_begin": ORDER_OFF_TYPE3_SECONDARY_BEGIN,
                "type3_secondary_size": ORDER_OFF_TYPE3_SECONDARY_SIZE,
            },
            "production_position": (
                "save/live exact-row validation for geometry offsets only. The validated +0x70/+0x78 "
                "path vector still needs a domain-54 compact schema/feed expansion before it is a "
                "DLL-only production capture field."
            ),
            "errors": [],
        }
        payload["errors"] = validate_payload(payload)
        (out_dir / "battleplan_save_geometry_probe.json").write_text(
            json.dumps(payload, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        return payload
    finally:
        proc.close()


def melt_save(save_path: Path, rakaly_path: Path | None, out_dir: Path) -> tuple[str, dict[str, Any]]:
    rakaly = rakaly_path or ps.find_rakaly(None)
    text, was_melted = ps.melt_if_needed(save_path, rakaly, out_dir)
    melted_path = out_dir / "autosave_melted.txt"
    melted_path.write_text(text, encoding="utf-8", errors="replace")
    return text, {
        "path": str(save_path),
        "rakaly": str(rakaly) if rakaly else None,
        "was_melted": bool(was_melted),
        "melted_path": str(melted_path),
        "text_len": len(text),
    }


def parse_save_orders_groups(text: str) -> dict[int, list[dict[str, Any]]]:
    groups: dict[int, list[dict[str, Any]]] = {}
    pos = 0
    while True:
        idx = text.find("orders_group=", pos)
        if idx < 0:
            break
        brace = text.find("{", idx)
        end = scalar_probe.find_matching_brace(text, brace)
        if end is None:
            break
        block = text[brace + 1:end]
        pos = end + 1
        army_id = scalar_probe.parse_id_type_53(block)
        if army_id is None:
            continue
        orders = parse_order_instances(block)
        if orders:
            groups[army_id] = orders
    return groups


def parse_order_instances(group_block: str) -> list[dict[str, Any]]:
    orders = []
    pos = 0
    while True:
        idx = group_block.find("order_instance=", pos)
        if idx < 0:
            break
        brace = group_block.find("{", idx)
        end = scalar_probe.find_matching_brace(group_block, brace)
        if end is None:
            break
        block = group_block[brace + 1:end]
        pos = end + 1
        order_type = parse_int_field(block, "type")
        if order_type is None:
            continue
        orders.append(
            {
                "type": order_type,
                "instance_id": parse_int_field(block, "instance_id"),
                "path": parse_int_list_field(block, "path"),
                "states": parse_int_list_field(block, "states"),
                "can_execute": parse_yes_no_field(block, "can_execute"),
            }
        )
    return orders


def parse_int_field(block: str, field: str) -> int | None:
    match = re.search(rf"\b{re.escape(field)}\s*=\s*(-?\d+)\b", block)
    return int(match.group(1)) if match else None


def parse_yes_no_field(block: str, field: str) -> str | None:
    match = re.search(rf"\b{re.escape(field)}\s*=\s*(yes|no)\b", block)
    return str(match.group(1)) if match else None


def parse_int_list_field(block: str, field: str) -> list[int]:
    match = re.search(rf"\b{re.escape(field)}\s*=\s*\{{([^{{}}]*)\}}", block, re.S)
    if not match:
        return []
    return [int(value) for value in re.findall(r"-?\d+", match.group(1))]


def pair_save_live_orders(
    save_orders: dict[int, list[dict[str, Any]]],
    live_by_army: dict[int, list[Any]],
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    pairs = []
    matched_armies = same_count_armies = same_type_sequence_armies = 0
    paired_type_hist: Counter[int] = Counter()
    skipped_reason: Counter[str] = Counter()
    for army_id, save_rows in save_orders.items():
        live_rows = live_by_army.get(army_id)
        if not live_rows:
            skipped_reason["army_missing_in_live"] += 1
            continue
        matched_armies += 1
        if len(live_rows) != len(save_rows):
            skipped_reason["order_count_mismatch"] += 1
            continue
        same_count_armies += 1
        save_types = [int(row["type"]) for row in save_rows]
        live_types = [int(row.order_type) for row in live_rows]
        if save_types != live_types:
            skipped_reason["order_type_sequence_mismatch"] += 1
            continue
        same_type_sequence_armies += 1
        for slot_index, (save_row, live_row) in enumerate(zip(save_rows, live_rows)):
            paired_type_hist[int(save_row["type"])] += 1
            pairs.append(
                {
                    "army_id": army_id,
                    "slot_index": slot_index,
                    "save": save_row,
                    "live": live_row,
                }
            )
    return pairs, {
        "save_armies_with_orders": len(save_orders),
        "live_armies_with_orders": len(live_by_army),
        "matched_armies": matched_armies,
        "same_order_count_armies": same_count_armies,
        "same_type_sequence_armies": same_type_sequence_armies,
        "paired_order_rows": len(pairs),
        "paired_order_type_histogram": {str(key): paired_type_hist[key] for key in sorted(paired_type_hist)},
        "skipped_reason": dict(sorted(skipped_reason.items())),
    }


def evaluate_type5_positive_control(proc: Any, pairs: list[dict[str, Any]]) -> dict[str, Any]:
    rows = [pair for pair in pairs if int(pair["save"]["type"]) == 5 and pair["save"]["states"]]
    return evaluate_vector_against_save(
        proc,
        rows,
        begin_off=direct_oob_probe.ORDER_OFF_TYPE5_STATES_BEGIN,
        size_off=direct_oob_probe.ORDER_OFF_TYPE5_STATES_SIZE,
        expected_key="states",
        max_value=MAX_PROVINCE_ID,
        name="positive_control_type5_states_plus_0xE0",
        status_when_exact="positive_control_pass",
        min_exact_rows=MIN_TYPE5_EXACT_CONTROL_ROWS,
    )


def evaluate_path_candidate(
    proc: Any,
    pairs: list[dict[str, Any]],
    *,
    name: str,
    begin_off: int,
    size_off: int,
    applies_to: set[int],
    min_exact_rows: int,
    reject_when_zero_exact: bool = False,
) -> dict[str, Any]:
    rows = [
        pair
        for pair in pairs
        if int(pair["save"]["type"]) in applies_to and pair["save"]["path"]
    ]
    status_when_exact = "validated_save_paired_geometry_slice"
    result = evaluate_vector_against_save(
        proc,
        rows,
        begin_off=begin_off,
        size_off=size_off,
        expected_key="path",
        max_value=MAX_PROVINCE_ID,
        name=name,
        status_when_exact=status_when_exact,
        min_exact_rows=min_exact_rows,
    )
    if reject_when_zero_exact and result["exact_rows"] == 0 and result["tested_rows"] > 0:
        result["status"] = "rejected_save_paired_geometry_candidate"
        result["evidence"].append("0 save/live paired rows matched the candidate as the saved path vector")
    return result


def evaluate_vector_against_save(
    proc: Any,
    rows: list[dict[str, Any]],
    *,
    begin_off: int,
    size_off: int,
    expected_key: str,
    max_value: int,
    name: str,
    status_when_exact: str,
    min_exact_rows: int,
) -> dict[str, Any]:
    exact_rows = 0
    valid_rows = 0
    invalid_reasons: Counter[str] = Counter()
    per_type: dict[str, dict[str, int]] = defaultdict(lambda: {"tested": 0, "valid": 0, "exact": 0})
    mismatch_samples = []
    exact_samples = []
    compact_rows = []
    for pair in rows:
        save_row = pair["save"]
        live_order = pair["live"]
        typ = str(save_row["type"])
        per_type[typ]["tested"] += 1
        decoded = candidate_probe.read_vector(proc, live_order.order_addr, begin_off, size_off, max_value)
        if decoded.get("valid"):
            valid_rows += 1
            per_type[typ]["valid"] += 1
            values = decoded.get("values", [])
            if values == save_row[expected_key]:
                exact_rows += 1
                per_type[typ]["exact"] += 1
                compact_rows.append(compact_row(pair, values, expected_key))
                if len(exact_samples) < 8:
                    exact_samples.append(sample_pair(pair, values, expected_key))
            elif len(mismatch_samples) < 8:
                mismatch_samples.append(sample_pair(pair, values, expected_key))
        else:
            invalid_reasons[str(decoded.get("reason"))] += 1
            if len(mismatch_samples) < 8:
                mismatch_samples.append(sample_pair(pair, decoded.get("values", []), expected_key, decoded.get("reason")))
    if exact_rows >= min_exact_rows:
        status = status_when_exact
    elif not rows:
        status = "insufficient_rows"
    else:
        status = "not_validated"
    return {
        "name": name,
        "status": status,
        "begin_offset": f"+0x{begin_off:X}",
        "size_offset": f"+0x{size_off:X}",
        "tested_rows": len(rows),
        "valid_rows": valid_rows,
        "exact_rows": exact_rows,
        "exact_fraction": (exact_rows / len(rows)) if rows else 0.0,
        "valid_fraction": (valid_rows / len(rows)) if rows else 0.0,
        "invalid_reasons": dict(sorted(invalid_reasons.items())),
        "per_order_type": {key: value for key, value in sorted(per_type.items())},
        "compact_rows": compact_rows,
        "exact_samples": exact_samples,
        "mismatch_samples": mismatch_samples,
        "evidence": [
            f"{exact_rows}/{len(rows)} save/live paired rows exactly matched {name}",
            f"{valid_rows}/{len(rows)} rows decoded as bounded vectors at {name}",
        ],
    }


def compact_row(pair: dict[str, Any], actual_values: list[int], expected_key: str) -> dict[str, Any]:
    live = pair["live"]
    save = pair["save"]
    row = {
        "army_id": int(pair["army_id"]),
        "slot_index": int(pair["slot_index"]),
        "order_type": int(save["type"]),
        "instance_id": save.get("instance_id"),
        "generation": int(save.get("instance_id") or 0),
        "live_order_addr": f"0x{int(live.order_addr):X}",
        "can_execute": save.get("can_execute"),
        "target_states": [],
        "path_provinces": [],
    }
    if expected_key == "states":
        row["target_states"] = list(actual_values)
    elif expected_key == "path":
        row["path_provinces"] = list(actual_values)
    return row


def sample_pair(
    pair: dict[str, Any],
    actual_values: list[int],
    expected_key: str,
    reason: Any = None,
) -> dict[str, Any]:
    live = pair["live"]
    save = pair["save"]
    return {
        "army_id": int(pair["army_id"]),
        "slot_index": int(pair["slot_index"]),
        "order_type": int(save["type"]),
        "instance_id": save.get("instance_id"),
        "expected_count": len(save[expected_key]),
        "actual_count": len(actual_values),
        "expected_sample": save[expected_key][:16],
        "actual_sample": actual_values[:16],
        "live_order_addr": f"0x{int(live.order_addr):X}",
        "reason": reason,
    }


def classify_candidate_results(
    positive_control: dict[str, Any],
    candidate_results: list[dict[str, Any]],
) -> str:
    statuses = {row["name"]: row["status"] for row in candidate_results}
    if positive_control.get("status") != "positive_control_pass":
        return "SAVE_LIVE_GEOMETRY_CONTROL_FAILED"
    if (
        statuses.get("non_type5_path_vector_plus_0x70") == "validated_save_paired_geometry_slice"
        and statuses.get("type3_secondary_vector_plus_0xC0") == "rejected_save_paired_geometry_candidate"
    ):
        return "VALIDATED_PATH70_AND_REJECTED_TYPE3_C0_GEOMETRY"
    if statuses.get("non_type5_path_vector_plus_0x70") == "validated_save_paired_geometry_slice":
        return "VALIDATED_PATH70_GEOMETRY_SLICE"
    if statuses.get("type3_secondary_vector_plus_0xC0") == "rejected_save_paired_geometry_candidate":
        return "REJECTED_TYPE3_C0_GEOMETRY"
    return "PARTIAL_SAVE_LIVE_GEOMETRY_PROBE"


def validate_payload(payload: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if payload.get("schema") != SCHEMA:
        errors.append("unexpected schema")
    if str(payload.get("exe_sha256", "")).lower() != EXPECTED_SHA256:
        errors.append("unexpected executable sha256")
    positive = payload.get("positive_control", {})
    if positive.get("status") != "positive_control_pass":
        errors.append("type-5 save/live positive control did not pass")
    if positive.get("exact_rows", 0) < MIN_TYPE5_EXACT_CONTROL_ROWS:
        errors.append("not enough exact type-5 control rows")
    candidates = {row["name"]: row for row in payload.get("candidate_results", [])}
    path70 = candidates.get("non_type5_path_vector_plus_0x70", {})
    if path70.get("exact_rows", 0) < MIN_PATH70_EXACT_ROWS:
        errors.append("not enough exact +0x70 path geometry rows")
    type3_c0 = candidates.get("type3_secondary_vector_plus_0xC0", {})
    if type3_c0.get("tested_rows", 0) > 0 and type3_c0.get("exact_rows", 0) != 0:
        errors.append("type3 +0xC0 rejection has exact matches")
    return errors


if __name__ == "__main__":
    raise SystemExit(main())
