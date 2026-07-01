"""Build fixture-gate input from before/after battleplan observations.

This is lane-local glue for the remaining blockers. It does not read or write
shared collector/recorder entrypoints. A future fixture harness can write the
observation JSON described here after the user redraws a non-type-5 order or
toggles execution; this tool converts those observations into the exact
`battleplan_fixture_gate.py` contract.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

from battleplan_fixture_gate import EXPECTED_SHA256, validate_fixture


OBSERVATION_SCHEMA = "army_oob_orders_battleplans_observation_v1"
FIXTURE_SCHEMA = "army_oob_orders_battleplans_fixture_v1"


def main() -> int:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    build = sub.add_parser("build", help="build fixture JSON from before/after observations")
    build.add_argument("observation_json", type=Path)
    build.add_argument("--out", type=Path, required=True)
    build.add_argument("--allow-rejection", action="store_true")
    validate = sub.add_parser("validate", help="validate observations without writing a fixture")
    validate.add_argument("observation_json", type=Path)
    validate.add_argument("--allow-rejection", action="store_true")
    template = sub.add_parser("write-template", help="write a fixture-observation template")
    template.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()

    if args.cmd == "write-template":
        args.out.write_text(json.dumps(observation_template(), indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(json.dumps({"status": "PASS", "wrote": str(args.out)}, indent=2, sort_keys=True))
        return 0

    observation = json.loads(args.observation_json.read_text(encoding="utf-8"))
    fixture, errors = build_fixture(observation, allow_rejection=args.allow_rejection)
    if args.cmd == "build" and fixture is not None:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(fixture, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                "status": "PASS" if not errors else "FAIL",
                "errors": errors,
                "geometry_rows": len((fixture or {}).get("non_type5_geometry", [])),
                "execution_rows": len((fixture or {}).get("battleplan_execution_state", [])),
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0 if not errors else 1


def build_fixture(observation: dict[str, Any], *, allow_rejection: bool) -> tuple[dict[str, Any] | None, list[str]]:
    errors = validate_observation_shape(observation)
    fixture = {
        "schema": FIXTURE_SCHEMA,
        "exe_sha256": observation.get("exe_sha256"),
        "capture": observation.get("capture", {}),
        "non_type5_geometry": [],
        "battleplan_execution_state": [],
    }
    actions = set(observation.get("operator_actions", []))
    for index, row in enumerate(observation.get("observations", [])):
        converted, row_errors = convert_observation_row(row, index, actions)
        errors.extend(row_errors)
        if converted is None:
            continue
        if converted["kind"] == "geometry":
            fixture["non_type5_geometry"].append(converted)
        elif converted["kind"] == "execution":
            fixture["battleplan_execution_state"].append(converted)
    errors.extend(validate_fixture(fixture, allow_rejection=allow_rejection))
    return fixture, errors


def validate_observation_shape(observation: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if observation.get("schema") != OBSERVATION_SCHEMA:
        errors.append(f"schema must be {OBSERVATION_SCHEMA}")
    if str(observation.get("exe_sha256", "")).lower() != EXPECTED_SHA256:
        errors.append("unexpected executable sha256")
    actions = observation.get("operator_actions")
    if not isinstance(actions, list) or not actions:
        errors.append("operator_actions must be a non-empty list")
    observations = observation.get("observations")
    if not isinstance(observations, list) or not observations:
        errors.append("observations must be a non-empty list")
    return errors


def convert_observation_row(row: Any, index: int, actions: set[str]) -> tuple[dict[str, Any] | None, list[str]]:
    name = f"observations[{index}]"
    errors: list[str] = []
    if not isinstance(row, dict):
        return None, [f"{name} must be an object"]
    kind = row.get("kind")
    if kind not in ("geometry", "execution"):
        errors.append(f"{name}.kind must be geometry or execution")
    action = row.get("operator_action")
    if not isinstance(action, str) or not action:
        errors.append(f"{name}.operator_action is required")
    elif action not in actions:
        errors.append(f"{name}.operator_action must be listed in operator_actions")
    if row.get("source_kind") == "passive_scan":
        errors.append(f"{name}.source_kind passive_scan cannot prove or reject a candidate")
    identity_errors = validate_identity(row, name)
    errors.extend(identity_errors)
    if kind == "geometry" and row.get("order_type") == 5:
        errors.append(f"{name}.order_type must not be 5 for geometry")
    candidate = row.get("candidate")
    if not isinstance(candidate, str) or not candidate.startswith("+0x"):
        errors.append(f"{name}.candidate must be a +0x... offset string")
    before = row.get("before", {}).get("value") if isinstance(row.get("before"), dict) else None
    after = row.get("after", {}).get("value") if isinstance(row.get("after"), dict) else None
    if before is None or after is None:
        errors.append(f"{name}.before.value and .after.value are required")
    field_name = row.get("field_name")
    field_type = row.get("field_type")
    if not isinstance(field_name, str) or not field_name:
        errors.append(f"{name}.field_name is required")
    if not isinstance(field_type, str) or not field_type:
        errors.append(f"{name}.field_type is required")
    if errors:
        return None, errors

    changed = before != after
    base = {
        "kind": kind,
        "army_id": row["army_id"],
        "slot_index": row["slot_index"],
        "order_type": row["order_type"],
        "order_generation": row["order_generation"],
    }
    validated_offsets = row.get("validated_offsets")
    if not isinstance(validated_offsets, list) or not validated_offsets:
        validated_offsets = [
            {
                "name": field_name,
                "offset": candidate,
                "type": field_type,
                "evidence": f"{action} changed {candidate} from before.value to after.value",
            }
        ]
    if changed:
        base.update(
            {
                "status": "exact",
                "validated_offsets": validated_offsets,
                "field_values": {field_name: after},
            }
        )
    else:
        base.update(
            {
                "status": "rejected",
                "rejection_evidence": {
                    "candidate": candidate,
                    "reason": "mutation_mismatch" if kind == "geometry" else "not_execution_state",
                    "evidence": [f"{action} was performed but before.value and after.value were identical"],
                },
            }
        )
    return base, []


def validate_identity(row: dict[str, Any], name: str) -> list[str]:
    errors: list[str] = []
    for key in ("army_id", "order_generation"):
        if not isinstance(row.get(key), int) or row[key] <= 0:
            errors.append(f"{name}.{key} must be a positive integer")
    if not isinstance(row.get("slot_index"), int) or row["slot_index"] < 0:
        errors.append(f"{name}.slot_index must be a nonnegative integer")
    if not isinstance(row.get("order_type"), int):
        errors.append(f"{name}.order_type must be an integer")
    return errors


def observation_template() -> dict[str, Any]:
    return {
        "schema": OBSERVATION_SCHEMA,
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
            "game_hours_advanced": 1,
            "max_frame_hour_gap": 1,
        },
        "operator_actions": ["redraw_non_type5_order", "toggle_battleplan_execution"],
        "observations": [
            {
                "kind": "geometry",
                "operator_action": "redraw_non_type5_order",
                "source_kind": "live_fixture",
                "army_id": 0,
                "slot_index": 0,
                "order_type": 2,
                "order_generation": 1,
                "candidate": "+0x70",
                "field_name": "province_path",
                "field_type": "vec<u32>",
                "before": {"value": []},
                "after": {"value": []},
            },
            {
                "kind": "execution",
                "operator_action": "toggle_battleplan_execution",
                "source_kind": "live_fixture",
                "army_id": 0,
                "slot_index": 0,
                "order_type": 2,
                "order_generation": 1,
                "candidate": "+0x150",
                "field_name": "is_executing",
                "field_type": "u8",
                "before": {"value": 0},
                "after": {"value": 1},
            },
        ],
    }


if __name__ == "__main__":
    raise SystemExit(main())
