"""Executable fixture contract for unresolved battleplan gaps.

The active compact lane already accepts exact type-5 target-state rows and
explicitly badges non-type-5 geometry as unsupported. This gate defines what a
future live fixture must prove before non-type-5 geometry or execution state can
be promoted. It reads a compact observation JSON produced by a fixture harness;
with missing evidence it fails closed and prints the exact missing clauses.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

REQUIRED_CAPTURE_FIELDS = (
    "dll_only",
    "compact_stream",
    "rust_decoded",
    "hoi4replay_verified",
    "zero_skipped_hours",
    "no_failures",
)

REQUIRED_FAILURES = (
    "source_unavailable",
    "partial_data",
    "bridge_unavailable",
    "worker_panic",
)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("fixture_json", type=Path, nargs="?")
    ap.add_argument(
        "--allow-rejection",
        action="store_true",
        help="accept exact invalid-candidate rejection evidence instead of exact live rows",
    )
    ap.add_argument("--self-test", action="store_true", help="run built-in gate acceptance tests")
    args = ap.parse_args()

    if args.self_test:
        errors = self_test()
        print(json.dumps({"status": "PASS" if not errors else "FAIL", "errors": errors}, indent=2, sort_keys=True))
        return 0 if not errors else 1
    if args.fixture_json is None:
        ap.error("fixture_json is required unless --self-test is used")
    fixture = json.loads(args.fixture_json.read_text(encoding="utf-8"))
    errors = validate_fixture(fixture, allow_rejection=args.allow_rejection)
    result = {
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "contract": {
            "requires_non_type5_geometry": True,
            "requires_battleplan_execution_state": True,
            "requires_dll_compact_replay_path": True,
            "accepts_exact_rejection_with_allow_rejection": args.allow_rejection,
        },
    }
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if not errors else 1


def validate_fixture(fixture: dict[str, Any], *, allow_rejection: bool) -> list[str]:
    errors: list[str] = []
    if fixture.get("schema") != "army_oob_orders_battleplans_fixture_v1":
        errors.append("schema must be army_oob_orders_battleplans_fixture_v1")
    if str(fixture.get("exe_sha256", "")).lower() != EXPECTED_SHA256:
        errors.append("unexpected executable sha256")
    capture = fixture.get("capture", {})
    if not isinstance(capture, dict):
        return errors + ["missing capture object"]
    for field in REQUIRED_CAPTURE_FIELDS:
        if capture.get(field) is not True:
            errors.append(f"capture.{field} must be true")
    for field in REQUIRED_FAILURES:
        if capture.get(field) not in (False, None):
            errors.append(f"capture.{field} must be false or omitted")
    if int(capture.get("game_hours_advanced", 0)) < 1:
        errors.append("capture.game_hours_advanced must be at least 1")
    if int(capture.get("max_frame_hour_gap", 9999)) > 1:
        errors.append("capture.max_frame_hour_gap must be <= 1")

    geometry = fixture.get("non_type5_geometry", [])
    execution = fixture.get("battleplan_execution_state", [])
    if not isinstance(geometry, list):
        errors.append("non_type5_geometry must be a list")
        geometry = []
    if not isinstance(execution, list):
        errors.append("battleplan_execution_state must be a list")
        execution = []
    if not any(row_exact(row, "geometry") for row in geometry):
        if allow_rejection and any(row_rejected(row, "geometry") for row in geometry):
            pass
        else:
            errors.append(
                "need at least one non-type-5 order with exact geometry, or exact rejection with --allow-rejection"
            )
    if not any(row_exact(row, "execution") for row in execution):
        if allow_rejection and any(row_rejected(row, "execution") for row in execution):
            pass
        else:
            errors.append(
                "need at least one battleplan execution-state row, or exact rejection with --allow-rejection"
            )
    for index, row in enumerate(geometry):
        validate_row(errors, row, f"non_type5_geometry[{index}]", required_kind="geometry")
    for index, row in enumerate(execution):
        validate_row(errors, row, f"battleplan_execution_state[{index}]", required_kind="execution")
    return errors


def row_exact(row: Any, kind: str) -> bool:
    return (
        isinstance(row, dict)
        and row.get("kind") == kind
        and row.get("status") == "exact"
        and valid_order_identity(row)
        and valid_offsets(row.get("validated_offsets"))
        and isinstance(row.get("field_values"), dict)
        and bool(row.get("field_values"))
    )


def row_rejected(row: Any, kind: str) -> bool:
    return (
        isinstance(row, dict)
        and row.get("kind") == kind
        and row.get("status") == "rejected"
        and valid_order_identity(row)
        and valid_rejection(row.get("rejection_evidence"))
    )


def validate_row(errors: list[str], row: Any, name: str, *, required_kind: str) -> None:
    if not isinstance(row, dict):
        errors.append(f"{name} must be an object")
        return
    if row.get("kind") != required_kind:
        errors.append(f"{name}.kind must be {required_kind}")
    if row.get("status") not in ("exact", "rejected"):
        errors.append(f"{name}.status must be exact or rejected")
        if row.get("status") == "candidate" or row.get("passive_scan") is not None:
            errors.append(f"{name}: passive candidate rows are not accepted as replay evidence")
    if not valid_order_identity(row):
        errors.append(f"{name} must include positive army_id, slot_index, order_type, and order_generation")
    if required_kind == "geometry" and int(row.get("order_type", 5)) == 5:
        errors.append(f"{name}.order_type must not be 5")
    if row.get("status") == "exact":
        if not valid_offsets(row.get("validated_offsets")):
            errors.append(f"{name}.validated_offsets must include named offset/type/evidence objects")
        if not isinstance(row.get("field_values"), dict) or not row.get("field_values"):
            errors.append(f"{name}.field_values must be non-empty for exact rows")
    if row.get("status") == "rejected" and not valid_rejection(row.get("rejection_evidence")):
        errors.append(f"{name}.rejection_evidence must include candidate, reason, and evidence list")


def valid_order_identity(row: dict[str, Any]) -> bool:
    return (
        is_nonnegative_int(row.get("slot_index"))
        and positive_int(row.get("army_id"))
        and isinstance(row.get("order_type"), int)
        and positive_int(row.get("order_generation"))
    )


def valid_offsets(value: Any) -> bool:
    if not isinstance(value, list) or not value:
        return False
    for item in value:
        if not isinstance(item, dict):
            return False
        if not isinstance(item.get("name"), str) or not item["name"]:
            return False
        if not isinstance(item.get("offset"), str) or not item["offset"].startswith("+0x"):
            return False
        if not isinstance(item.get("type"), str) or not item["type"]:
            return False
        if not isinstance(item.get("evidence"), str) or not item["evidence"]:
            return False
    return True


def valid_rejection(value: Any) -> bool:
    if not isinstance(value, dict):
        return False
    if not isinstance(value.get("candidate"), str) or not value["candidate"]:
        return False
    if value.get("reason") not in {
        "invalid_pointer",
        "mutation_mismatch",
        "not_execution_state",
        "not_geometry",
        "not_live_state",
        "stale_cached_value",
    }:
        return False
    evidence = value.get("evidence")
    return isinstance(evidence, list) and bool(evidence) and all(isinstance(item, str) and item for item in evidence)


def positive_int(value: Any) -> bool:
    return isinstance(value, int) and value > 0


def is_nonnegative_int(value: Any) -> bool:
    return isinstance(value, int) and value >= 0


def self_test() -> list[str]:
    base = {
        "schema": "army_oob_orders_battleplans_fixture_v1",
        "exe_sha256": EXPECTED_SHA256,
        "capture": {
            "dll_only": True,
            "compact_stream": True,
            "rust_decoded": True,
            "hoi4replay_verified": True,
            "zero_skipped_hours": True,
            "no_failures": True,
            "source_unavailable": False,
            "partial_data": False,
            "bridge_unavailable": False,
            "worker_panic": False,
            "game_hours_advanced": 2,
            "max_frame_hour_gap": 1,
        },
    }
    exact = dict(base)
    exact["non_type5_geometry"] = [
        {
            "kind": "geometry",
            "status": "exact",
            "army_id": 17,
            "slot_index": 0,
            "order_type": 2,
            "order_generation": 4,
            "validated_offsets": [
                {"name": "line_points", "offset": "+0x120", "type": "vec<u32>", "evidence": "fixture mutation matched"}
            ],
            "field_values": {"province_path": [1, 2, 3]},
        }
    ]
    exact["battleplan_execution_state"] = [
        {
            "kind": "execution",
            "status": "exact",
            "army_id": 17,
            "slot_index": 0,
            "order_type": 2,
            "order_generation": 4,
            "validated_offsets": [
                {"name": "is_executing", "offset": "+0x150", "type": "u8", "evidence": "toggle matched"}
            ],
            "field_values": {"is_executing": True},
        }
    ]
    rejected = dict(base)
    rejected["non_type5_geometry"] = [
        {
            "kind": "geometry",
            "status": "rejected",
            "army_id": 17,
            "slot_index": 0,
            "order_type": 2,
            "order_generation": 4,
            "rejection_evidence": {
                "candidate": "order+0x120",
                "reason": "mutation_mismatch",
                "evidence": ["fixture changed line geometry, candidate bytes did not change"],
            },
        }
    ]
    rejected["battleplan_execution_state"] = [
        {
            "kind": "execution",
            "status": "rejected",
            "army_id": 17,
            "slot_index": 0,
            "order_type": 2,
            "order_generation": 4,
            "rejection_evidence": {
                "candidate": "order+0x150",
                "reason": "not_execution_state",
                "evidence": ["execution toggle did not affect candidate field"],
            },
        }
    ]
    errors: list[str] = []
    exact_errors = validate_fixture(exact, allow_rejection=False)
    if exact_errors:
        errors.append(f"exact fixture should pass: {exact_errors}")
    rejection_without_flag = validate_fixture(rejected, allow_rejection=False)
    if not rejection_without_flag:
        errors.append("rejection fixture should fail without --allow-rejection")
    rejection_errors = validate_fixture(rejected, allow_rejection=True)
    if rejection_errors:
        errors.append(f"rejection fixture should pass with --allow-rejection: {rejection_errors}")
    missing_errors = validate_fixture({**base, "non_type5_geometry": [], "battleplan_execution_state": []}, allow_rejection=False)
    if len(missing_errors) != 2:
        errors.append(f"missing fixture should report exactly two blocker clauses: {missing_errors}")
    return errors


if __name__ == "__main__":
    raise SystemExit(main())
