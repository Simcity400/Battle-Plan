#!/usr/bin/env python3
"""Lane-local gates for unresolved division-state proof contracts.

This script validates JSON evidence for fields that must not enter the
production compact row from equality-only or aggregate evidence.
"""

from __future__ import annotations

import argparse
import json
import sys
import tempfile
from pathlib import Path
from typing import Any


DIVISION_TYPE = 4713
REJECTED_MANPOWER_LEVELS = {
    "country",
    "country_pool",
    "template",
    "template_requirement",
    "equipment",
    "equipment_bank",
}


class GateResult:
    def __init__(self, proof_kind: str) -> None:
        self.proof_kind = proof_kind
        self.accepted = False
        self.reasons: list[str] = []
        self.units: dict[str, int] = {}

    def reject(self, reason: str) -> None:
        self.reasons.append(reason)

    def add_unit(self, key: str, value: int) -> None:
        self.units[key] = self.units.get(key, 0) + int(value)

    def to_json(self) -> str:
        return json.dumps(
            {
                "proof_kind": self.proof_kind,
                "accepted": self.accepted,
                "reasons": self.reasons,
                "units": self.units,
            },
            indent=2,
            sort_keys=True,
        )


def _positive_int(value: Any) -> bool:
    return isinstance(value, int) and value > 0


def _as_bool(value: Any) -> bool:
    return value is True or value == 1


def _field(data: dict[str, Any], *names: str) -> Any:
    cur: Any = data
    for name in names:
        if not isinstance(cur, dict):
            return None
        cur = cur.get(name)
    return cur


def _offset_text(value: Any) -> str:
    if isinstance(value, int):
        return f"0x{value:X}"
    return str(value or "")


def validate_controller(data: dict[str, Any]) -> GateResult:
    result = GateResult("division_controller")
    if data.get("proof_kind") != "division_controller":
        result.reject("proof_kind must be division_controller")

    root_source = str(data.get("root_source") or "")
    if root_source in {"", "heap_scan_hot_loop", "unbounded_heap_scan"}:
        result.reject("root_source must be a production root or bounded one-time proof")

    candidate = data.get("candidate") if isinstance(data.get("candidate"), dict) else {}
    offset = _offset_text(candidate.get("offset") or data.get("candidate_offset"))
    if offset and offset.lower() not in {"0x1d0", "cdivision+0x1d0"}:
        result.reject(f"unexpected controller candidate offset {offset}")
    if not offset:
        result.reject("candidate.offset is required")

    rows = data.get("division_rows")
    if not isinstance(rows, list) or not rows:
        result.reject("division_rows must be a non-empty list")
        rows = []

    divergent_rows = 0
    matched_rows = 0
    for idx, row in enumerate(rows):
        if not isinstance(row, dict):
            result.reject(f"row {idx} is not an object")
            continue
        div_id = row.get("div_id")
        owner = row.get("owner_tag_db_id")
        candidate_controller = row.get("candidate_controller_tag_db_id")
        oracle_controller = row.get("oracle_controller_tag_db_id")
        if not _positive_int(div_id):
            result.reject(f"row {idx} has invalid div_id {div_id!r}")
        if not _positive_int(owner):
            result.reject(f"row {idx} has invalid owner_tag_db_id {owner!r}")
        if not _positive_int(candidate_controller):
            result.reject(
                f"row {idx} has invalid candidate_controller_tag_db_id {candidate_controller!r}"
            )
        if not _positive_int(oracle_controller):
            result.reject(
                f"row {idx} has invalid oracle_controller_tag_db_id {oracle_controller!r}"
            )
        validation = row.get("body_validation")
        if not isinstance(validation, dict):
            result.reject(f"row {idx} missing body_validation")
        else:
            if not _as_bool(validation.get("vtable_valid")):
                result.reject(f"row {idx} body_validation.vtable_valid is not true")
            type_tag = validation.get("type_tag")
            if type_tag != DIVISION_TYPE:
                result.reject(f"row {idx} body_validation.type_tag {type_tag!r} != {DIVISION_TYPE}")
            if validation.get("div_id") not in {None, div_id}:
                result.reject(f"row {idx} body_validation.div_id does not match div_id")
        if (
            _positive_int(owner)
            and _positive_int(candidate_controller)
            and _positive_int(oracle_controller)
        ):
            if candidate_controller == oracle_controller:
                matched_rows += 1
            else:
                result.reject(
                    f"row {idx} candidate_controller_tag_db_id {candidate_controller} "
                    f"!= oracle_controller_tag_db_id {oracle_controller}"
                )
            if owner != oracle_controller:
                divergent_rows += 1

    result.add_unit("controller_rows", len(rows))
    result.add_unit("controller_matched_rows", matched_rows)
    result.add_unit("controller_owner_divergent_rows", divergent_rows)
    if divergent_rows == 0:
        result.reject("no owner/controller-divergent row; equality-only evidence is not accepted")

    result.accepted = not result.reasons
    return result


def validate_manpower(data: dict[str, Any]) -> GateResult:
    result = GateResult("division_manpower")
    if data.get("proof_kind") != "division_manpower":
        result.reject("proof_kind must be division_manpower")

    candidate = data.get("candidate") if isinstance(data.get("candidate"), dict) else {}
    source_level = str(candidate.get("source_level") or data.get("source_level") or "")
    if not source_level:
        result.reject("candidate.source_level is required")
    if source_level in REJECTED_MANPOWER_LEVELS:
        result.reject(
            f"{source_level} evidence is not per-division current manpower proof"
        )

    route = str(candidate.get("route") or data.get("route") or "")
    if route not in {"division_offset", "reachable_from_division", "derived_contract"}:
        result.reject(
            "candidate.route must be division_offset, reachable_from_division, or derived_contract"
        )

    if route in {"division_offset", "reachable_from_division"}:
        if not (candidate.get("offset") or candidate.get("path")):
            result.reject("candidate.offset or candidate.path is required for exact manpower proof")
    if route == "derived_contract":
        uses = set(candidate.get("uses_fields") or [])
        required = {"template_manpower", "division_strength"}
        missing = sorted(required - uses)
        if missing:
            result.reject(f"derived_contract missing uses_fields: {', '.join(missing)}")
        if not _as_bool(candidate.get("time_aligned_oracle")):
            result.reject("derived_contract requires time_aligned_oracle=true")

    rows = data.get("division_rows")
    if not isinstance(rows, list) or not rows:
        result.reject("division_rows must be a non-empty list")
        rows = []

    matched_rows = 0
    for idx, row in enumerate(rows):
        if not isinstance(row, dict):
            result.reject(f"row {idx} is not an object")
            continue
        div_id = row.get("div_id")
        live_value = row.get("live_manpower")
        oracle_value = row.get("oracle_manpower")
        computed_value = row.get("computed_manpower")
        if not _positive_int(div_id):
            result.reject(f"row {idx} has invalid div_id {div_id!r}")
        if route == "derived_contract" and computed_value is not None:
            live_value = computed_value
        if not isinstance(live_value, int) or live_value < 0:
            result.reject(f"row {idx} has invalid live/computed manpower {live_value!r}")
            continue
        if not isinstance(oracle_value, int) or oracle_value < 0:
            result.reject(f"row {idx} has invalid oracle_manpower {oracle_value!r}")
            continue
        if live_value != oracle_value:
            result.reject(
                f"row {idx} manpower {live_value} != oracle_manpower {oracle_value}"
            )
        else:
            matched_rows += 1

    result.add_unit("manpower_rows", len(rows))
    result.add_unit("manpower_matched_rows", matched_rows)
    if matched_rows == 0:
        result.reject("no rows matched oracle manpower")

    result.accepted = not result.reasons
    return result


def validate_path(path: Path) -> GateResult:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        result = GateResult("unknown")
        result.reject("top-level JSON must be an object")
        return result
    proof_kind = data.get("proof_kind")
    if proof_kind == "division_controller":
        return validate_controller(data)
    if proof_kind == "division_manpower":
        return validate_manpower(data)
    result = GateResult(str(proof_kind or "unknown"))
    result.reject("proof_kind must be division_controller or division_manpower")
    return result


def run_self_test() -> None:
    valid_controller = {
        "proof_kind": "division_controller",
        "root_source": "dll_native_bounded_probe",
        "candidate": {"offset": "CDivision+0x1D0"},
        "division_rows": [
            {
                "div_id": 10,
                "owner_tag_db_id": 1,
                "candidate_controller_tag_db_id": 2,
                "oracle_controller_tag_db_id": 2,
                "body_validation": {
                    "vtable_valid": True,
                    "type_tag": DIVISION_TYPE,
                    "div_id": 10,
                },
            }
        ],
    }
    equality_only_controller = {
        "proof_kind": "division_controller",
        "root_source": "dll_native_bounded_probe",
        "candidate": {"offset": "CDivision+0x1D0"},
        "division_rows": [
            {
                "div_id": 11,
                "owner_tag_db_id": 1,
                "candidate_controller_tag_db_id": 1,
                "oracle_controller_tag_db_id": 1,
                "body_validation": {
                    "vtable_valid": True,
                    "type_tag": DIVISION_TYPE,
                    "div_id": 11,
                },
            }
        ],
    }
    valid_manpower = {
        "proof_kind": "division_manpower",
        "candidate": {
            "source_level": "division",
            "route": "division_offset",
            "offset": "CDivision+0x000",
        },
        "division_rows": [
            {"div_id": 20, "live_manpower": 1234, "oracle_manpower": 1234}
        ],
    }
    template_only_manpower = {
        "proof_kind": "division_manpower",
        "candidate": {"source_level": "template", "route": "reachable_from_division"},
        "division_rows": [
            {"div_id": 21, "live_manpower": 9000, "oracle_manpower": 7231}
        ],
    }

    cases = [
        (valid_controller, True),
        (equality_only_controller, False),
        (valid_manpower, True),
        (template_only_manpower, False),
    ]
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        for idx, (case, expected) in enumerate(cases):
            path = root / f"case_{idx}.json"
            path.write_text(json.dumps(case), encoding="utf-8")
            result = validate_path(path)
            if result.accepted != expected:
                raise AssertionError(
                    f"case {idx} expected accepted={expected}, got {result.to_json()}"
                )


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("proof_json", nargs="?", type=Path)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)

    if args.self_test:
        run_self_test()
        print("gap_contract_gate self-test passed")
        return 0

    if args.proof_json is None:
        parser.error("proof_json is required unless --self-test is used")

    result = validate_path(args.proof_json)
    rendered = result.to_json()
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(rendered + "\n", encoding="utf-8")
    print(rendered)
    return 0 if result.accepted else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
