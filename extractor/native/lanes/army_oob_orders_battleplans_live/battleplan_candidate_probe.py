"""Bounded current-build candidate probe for battleplan geometry/execution gaps.

This is READ-ONLY proof code. It enumerates validated current-build army/order
objects, applies specific candidate field decoders, and either rejects invalid
candidates or marks them fixture-required. It does not edit shared collector or
recorder entrypoints.
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from dataclasses import dataclass
from datetime import datetime, timezone
import json
from pathlib import Path
import struct
import sys
import time
from typing import Any


REPO = Path(__file__).resolve().parents[4]
MEMORY_READER = REPO / "extractor" / "memory_reader"
for path in (str(REPO), str(MEMORY_READER), str(Path(__file__).resolve().parent)):
    if path not in sys.path:
        sys.path.insert(0, path)

import direct_oob_probe  # type: ignore  # noqa: E402
import winmem  # type: ignore  # noqa: E402


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
PROC_NAME = "hoi4.exe"

MAX_ORDER_ROWS = 4096
MAX_VECTOR_ELEMS = 5000
MAX_PROVINCE_ID = 65535
MAX_STATE_ID = 65535

STATUS_REJECTED = "rejected"
STATUS_FIXTURE_REQUIRED = "fixture_required"
STATUS_POSITIVE_CONTROL_PASS = "positive_control_pass"


@dataclass(frozen=True)
class OrderRef:
    army_id: int
    army_addr: int
    slot_index: int
    order_addr: int
    order_type: int


GEOMETRY_CANDIDATES = [
    {
        "name": "old_type2_province_vector_plus_0x70",
        "begin_off": 0x70,
        "size_off": 0x78,
        "kind": "geometry",
        "field_type": "vec<u32>",
        "applies_to": [1, 2, 3],
        "source": "memory_reader/construction_orders_offsets.draft.json type-2 provinces @+0x70/+0x78",
    },
    {
        "name": "old_type3_secondary_vector_plus_0xC0",
        "begin_off": 0xC0,
        "size_off": 0xC8,
        "kind": "geometry",
        "field_type": "vec<u32>",
        "applies_to": [3],
        "source": "memory_reader/construction_orders_offsets.draft.json type-3 provinces @+0x70/+0xC0 note",
    },
]

EXECUTION_CANDIDATES = [
    {
        "name": "template_is_executing_u8_plus_0x150",
        "offset": 0x150,
        "kind": "execution",
        "field_type": "u8",
        "applies_to": [1, 2, 3, 5],
        "source": "battleplan_observation_template.json placeholder candidate",
    }
]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int)
    ap.add_argument("--out-dir", type=Path, required=True)
    args = ap.parse_args()
    report = run(args.pid, args.out_dir)
    print(
        json.dumps(
            {
                "status": "PASS" if not report["errors"] else "FAIL",
                "errors": report["errors"],
                "orders": report["order_summary"],
                "candidate_statuses": {
                    item["name"]: item["status"] for item in report["candidate_results"]
                },
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0 if not report["errors"] else 1


def run(pid: int | None, out_dir: Path) -> dict[str, Any]:
    if pid is None:
        pid = winmem.find_pid(PROC_NAME)
    if pid is None:
        raise RuntimeError("hoi4.exe not running")
    started = time.monotonic()
    p = winmem.Process(pid, PROC_NAME)
    try:
        fingerprint = p.fingerprint()
        sha = str(fingerprint.get("sha256") or "").lower()
        if sha != EXPECTED_SHA256:
            raise RuntimeError(f"build gate failed: {sha} != {EXPECTED_SHA256}")
        roots, scan_stats = direct_oob_probe.enum_current_oob_roots(p)
        orders = enumerate_validated_orders(p, roots["army"])
        candidate_results = []
        candidate_results.append(validate_type5_positive_control(p, orders))
        for candidate in GEOMETRY_CANDIDATES:
            candidate_results.append(probe_vector_candidate(p, orders, candidate))
        for candidate in EXECUTION_CANDIDATES:
            candidate_results.append(probe_execution_candidate(p, orders, candidate))
        errors = validate_report_consistency(candidate_results)
        report = {
            "schema": "army_oob_orders_battleplans_candidate_probe_v1",
            "created_utc": datetime.now(timezone.utc).isoformat(),
            "pid": pid,
            "exe_sha256": sha,
            "module_base": p.module_base,
            "elapsed_s": time.monotonic() - started,
            "scan_stats": scan_stats,
            "order_summary": summarize_orders(orders),
            "candidate_results": candidate_results,
            "errors": errors,
            "proof_position": (
                "candidate rejection/fixture blocker only; no shared collector or recorder "
                "entrypoints were edited"
            ),
        }
        out_dir.mkdir(parents=True, exist_ok=True)
        (out_dir / "battleplan_candidate_probe.json").write_text(
            json.dumps(report, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        fixture_blocker = build_fixture_blocker(report)
        (out_dir / "battleplan_exact_fixture_blocker.json").write_text(
            json.dumps(fixture_blocker, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        return report
    finally:
        p.close()


def enumerate_validated_orders(p: Any, army_roots: list[Any]) -> list[OrderRef]:
    orders: list[OrderRef] = []
    order_vtable = p.module_base + direct_oob_probe.ORDER_VTABLE_RVA
    for army in army_roots:
        try:
            begin = p.u64(army.addr + direct_oob_probe.ARMY_OFF_ORDERS_VEC)
        except OSError:
            continue
        if not direct_oob_probe.heap_ptr(begin):
            continue
        for slot_index in range(direct_oob_probe.MAX_ARMY_ORDERS_PER_ARMY):
            try:
                order_ptr = p.u64(begin + slot_index * 8)
            except OSError:
                break
            if not direct_oob_probe.heap_ptr(order_ptr):
                break
            try:
                if p.u64(order_ptr) != order_vtable:
                    continue
                if p.u64(order_ptr + direct_oob_probe.ORDER_OFF_ARMY_BACKREF) != army.addr:
                    continue
                order_type = p.i32(order_ptr + direct_oob_probe.ORDER_OFF_TYPE)
            except OSError:
                continue
            orders.append(
                OrderRef(
                    army_id=army.id,
                    army_addr=army.addr,
                    slot_index=slot_index,
                    order_addr=order_ptr,
                    order_type=order_type,
                )
            )
            if len(orders) > MAX_ORDER_ROWS:
                raise RuntimeError(f"order row count exceeded sanity bound {MAX_ORDER_ROWS}")
    return orders


def summarize_orders(orders: list[OrderRef]) -> dict[str, Any]:
    hist = Counter(order.order_type for order in orders)
    return {
        "validated_order_rows": len(orders),
        "order_type_histogram": {str(key): hist[key] for key in sorted(hist)},
        "non_type5_rows": sum(1 for order in orders if order.order_type != 5),
        "type5_rows": hist.get(5, 0),
        "sample": [
            {
                "army_id": order.army_id,
                "slot_index": order.slot_index,
                "order_type": order.order_type,
            }
            for order in orders[:16]
        ],
    }


def validate_type5_positive_control(p: Any, orders: list[OrderRef]) -> dict[str, Any]:
    type5 = [order for order in orders if order.order_type == 5]
    ok_rows = 0
    rejected_rows = 0
    invalid_reasons: Counter[str] = Counter()
    samples = []
    for order in type5:
        decoded = read_vector(
            p,
            order.order_addr,
            direct_oob_probe.ORDER_OFF_TYPE5_STATES_BEGIN,
            direct_oob_probe.ORDER_OFF_TYPE5_STATES_SIZE,
            MAX_STATE_ID,
        )
        if decoded["valid"]:
            ok_rows += 1
            if len(samples) < 8:
                samples.append(sample_order(order, decoded))
        else:
            rejected_rows += 1
            invalid_reasons[str(decoded["reason"])] += 1
    valid_fraction = (ok_rows / len(type5)) if type5 else 0.0
    status = (
        STATUS_POSITIVE_CONTROL_PASS
        if ok_rows > 0 and valid_fraction >= 0.80
        else "positive_control_failed"
    )
    return {
        "name": "positive_control_type5_states_plus_0xE0",
        "kind": "positive_control",
        "status": status,
        "candidate": {
            "begin_offset": "+0xE0",
            "size_offset": "+0xE8",
            "field_type": "vec<u32>",
        },
        "tested_rows": len(type5),
        "valid_rows": ok_rows,
        "invalid_rows": rejected_rows,
        "invalid_reasons": dict(sorted(invalid_reasons.items())),
        "samples": samples,
        "evidence": [
            f"validated {ok_rows}/{len(type5)} current-build type-5 state vectors with order vtable/backref gates",
            f"invalid type-5 rows were fail-closed, not decoded as data: {dict(sorted(invalid_reasons.items()))}",
            "this proves the candidate probe can decode known-good vector fields in the same order objects",
        ],
    }


def probe_vector_candidate(p: Any, orders: list[OrderRef], candidate: dict[str, Any]) -> dict[str, Any]:
    applies = set(candidate["applies_to"])
    rows = [order for order in orders if order.order_type in applies]
    valid_rows = 0
    invalid_reasons: Counter[str] = Counter()
    value_hist: Counter[int] = Counter()
    samples = []
    per_type = defaultdict(lambda: {"tested": 0, "valid": 0})
    for order in rows:
        per_type[str(order.order_type)]["tested"] += 1
        decoded = read_vector(
            p,
            order.order_addr,
            candidate["begin_off"],
            candidate["size_off"],
            MAX_PROVINCE_ID,
        )
        if decoded["valid"]:
            valid_rows += 1
            per_type[str(order.order_type)]["valid"] += 1
            value_hist.update(decoded["values"][:32])
            if len(samples) < 8:
                samples.append(sample_order(order, decoded))
        else:
            invalid_reasons[str(decoded["reason"])] += 1
    if not rows:
        status = STATUS_FIXTURE_REQUIRED
        evidence = ["no rows of the candidate order types exist in this live frame"]
    elif valid_rows == 0:
        status = STATUS_REJECTED
        evidence = [
            f"0/{len(rows)} rows decoded as a bounded province vector",
            f"invalid reasons: {dict(sorted(invalid_reasons.items()))}",
            "candidate is rejected for current build as a production geometry path",
        ]
    else:
        status = STATUS_FIXTURE_REQUIRED
        evidence = [
            f"{valid_rows}/{len(rows)} rows decoded as bounded vector-like data",
            "passive vector-like data is not accepted as geometry without redraw fixture",
        ]
    return {
        "name": candidate["name"],
        "kind": candidate["kind"],
        "status": status,
        "candidate": {
            "begin_offset": f"+0x{candidate['begin_off']:X}",
            "size_offset": f"+0x{candidate['size_off']:X}",
            "field_type": candidate["field_type"],
            "source": candidate["source"],
        },
        "tested_rows": len(rows),
        "valid_rows": valid_rows,
        "invalid_rows": len(rows) - valid_rows,
        "invalid_reasons": dict(sorted(invalid_reasons.items())),
        "per_order_type": {key: value for key, value in sorted(per_type.items())},
        "top_values_sample": [value for value, _count in value_hist.most_common(16)],
        "samples": samples,
        "evidence": evidence,
    }


def probe_execution_candidate(p: Any, orders: list[OrderRef], candidate: dict[str, Any]) -> dict[str, Any]:
    applies = set(candidate["applies_to"])
    rows = [order for order in orders if order.order_type in applies]
    values = Counter()
    invalid_rows = 0
    samples = []
    for order in rows:
        try:
            value = p.u8(order.order_addr + candidate["offset"])
        except OSError:
            invalid_rows += 1
            continue
        values[value] += 1
        if len(samples) < 8:
            samples.append(
                {
                    "army_id": order.army_id,
                    "slot_index": order.slot_index,
                    "order_type": order.order_type,
                    "value": value,
                }
            )
    non_boolean = sum(count for value, count in values.items() if value not in (0, 1))
    if not rows:
        status = STATUS_FIXTURE_REQUIRED
        evidence = ["no rows of the candidate order types exist in this live frame"]
    elif invalid_rows:
        status = STATUS_REJECTED
        evidence = [
            f"{invalid_rows}/{len(rows)} rows were unreadable at candidate offset",
            "candidate is rejected as a current-build scalar path",
        ]
    elif non_boolean:
        status = STATUS_REJECTED
        evidence = [
            f"{non_boolean}/{len(rows)} rows read non-boolean u8 values at the candidate offset",
            f"value histogram: {dict(sorted(values.items()))}",
            "candidate is rejected for current build as an is_executing u8 path",
        ]
    else:
        status = STATUS_FIXTURE_REQUIRED
        evidence = [
            f"{len(rows)} rows read boolean-shaped values at candidate offset",
            "boolean-shaped passive data is not accepted as execution state without toggle fixture",
        ]
    return {
        "name": candidate["name"],
        "kind": candidate["kind"],
        "status": status,
        "candidate": {
            "offset": f"+0x{candidate['offset']:X}",
            "field_type": candidate["field_type"],
            "source": candidate["source"],
        },
        "tested_rows": len(rows),
        "valid_rows": len(rows) - invalid_rows,
        "invalid_rows": invalid_rows,
        "value_histogram": {str(key): values[key] for key in sorted(values)},
        "samples": samples,
        "evidence": evidence,
    }


def read_vector(p: Any, base: int, begin_off: int, size_off: int, max_value: int) -> dict[str, Any]:
    try:
        begin = p.u64(base + begin_off)
        count = p.u32(base + size_off)
    except OSError:
        return {"valid": False, "reason": "unreadable_header"}
    if count == 0:
        return {"valid": True, "reason": "empty_vector", "count": 0, "values": []}
    if count > MAX_VECTOR_ELEMS:
        return {"valid": False, "reason": "count_exceeds_bound", "begin": begin, "count": count}
    if not direct_oob_probe.heap_ptr(begin, 4):
        return {"valid": False, "reason": "invalid_pointer", "begin": begin, "count": count}
    raw = p.try_read(begin, count * 4)
    if len(raw) != count * 4:
        return {"valid": False, "reason": "short_read", "begin": begin, "count": count, "read": len(raw)}
    values = [struct.unpack_from("<I", raw, index * 4)[0] for index in range(count)]
    if any(value == 0 or value > max_value for value in values):
        return {
            "valid": False,
            "reason": "out_of_range_values",
            "begin": begin,
            "count": count,
            "sample": values[:16],
        }
    return {"valid": True, "reason": "decoded", "begin": begin, "count": count, "values": values}


def sample_order(order: OrderRef, decoded: dict[str, Any]) -> dict[str, Any]:
    return {
        "army_id": order.army_id,
        "slot_index": order.slot_index,
        "order_type": order.order_type,
        "count": decoded.get("count"),
        "reason": decoded.get("reason"),
        "values_sample": decoded.get("values", [])[:16],
    }


def validate_report_consistency(candidate_results: list[dict[str, Any]]) -> list[str]:
    errors = []
    positive = next(
        (row for row in candidate_results if row["name"] == "positive_control_type5_states_plus_0xE0"),
        None,
    )
    if positive is None or positive.get("status") != STATUS_POSITIVE_CONTROL_PASS:
        errors.append("type-5 positive control did not pass; candidate rejections are not trustworthy")
    for result in candidate_results:
        if result.get("status") == "positive_control_failed":
            errors.append(f"{result['name']} failed")
    return errors


def build_fixture_blocker(report: dict[str, Any]) -> dict[str, Any]:
    rejected_geometry = [
        result for result in report["candidate_results"] if result["kind"] == "geometry" and result["status"] == STATUS_REJECTED
    ]
    fixture_required_geometry = [
        result
        for result in report["candidate_results"]
        if result["kind"] == "geometry" and result["status"] == STATUS_FIXTURE_REQUIRED
    ]
    rejected_execution = [
        result
        for result in report["candidate_results"]
        if result["kind"] == "execution" and result["status"] == STATUS_REJECTED
    ]
    fixture_required_execution = [
        result
        for result in report["candidate_results"]
        if result["kind"] == "execution" and result["status"] == STATUS_FIXTURE_REQUIRED
    ]
    return {
        "schema": "army_oob_orders_battleplans_exact_fixture_blocker_v1",
        "created_utc": report["created_utc"],
        "exe_sha256": report["exe_sha256"],
        "live_probe": "battleplan_candidate_probe.py",
        "positive_control": "positive_control_type5_states_plus_0xE0",
        "rejected_geometry_candidates": [summarize_candidate_for_blocker(row) for row in rejected_geometry],
        "fixture_required_geometry_candidates": [
            summarize_candidate_for_blocker(row) for row in fixture_required_geometry
        ],
        "rejected_execution_candidates": [summarize_candidate_for_blocker(row) for row in rejected_execution],
        "fixture_required_execution_candidates": [
            summarize_candidate_for_blocker(row) for row in fixture_required_execution
        ],
        "unbreakable_blocker": {
            "geometry": (
                "Any vector-like non-type-5 candidate still needs redraw before/after evidence; "
                "passive vector shape is not sufficient for production geometry."
            ),
            "execution": (
                "Any boolean-shaped execution candidate still needs execution-toggle before/after "
                "evidence; passive scalar shape is not sufficient for production execution state."
            ),
        },
    }


def summarize_candidate_for_blocker(result: dict[str, Any]) -> dict[str, Any]:
    return {
        "name": result["name"],
        "status": result["status"],
        "candidate": result["candidate"],
        "tested_rows": result["tested_rows"],
        "valid_rows": result["valid_rows"],
        "invalid_rows": result["invalid_rows"],
        "evidence": result["evidence"],
    }


if __name__ == "__main__":
    raise SystemExit(main())
