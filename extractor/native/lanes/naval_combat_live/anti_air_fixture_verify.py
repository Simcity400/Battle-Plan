#!/usr/bin/env python3
"""Fail-closed verifier for the one remaining d49 anti-air live fixture."""

from __future__ import annotations

import argparse
import json
from decimal import Decimal, InvalidOperation
from pathlib import Path


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
SIDE_VTABLE_RVA = 0x29CD298
COMBAT_VTABLE_RVA = 0x29C6F18
SCALE = Decimal(100_000)
EXPECTED_ORACLE = {
    (2804, "attacker"): Decimal("0"),
    (2804, "defender"): Decimal("0"),
    (1032, "attacker"): Decimal("0"),
    (1032, "defender"): Decimal("0"),
    (8819, "attacker"): Decimal("49.35661"),
    (8819, "defender"): Decimal("1.655"),
    (6434, "attacker"): Decimal("0"),
    (6434, "defender"): Decimal("6.15106"),
}


def integer(value: object, field: str) -> int:
    if isinstance(value, bool):
        raise ValueError(f"{field} must be an integer")
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        return int(value, 0)
    raise ValueError(f"{field} must be an integer or integer string")


def verify(document: dict[str, object]) -> dict[str, object]:
    errors: list[str] = []
    if str(document.get("binary_sha256", "")).lower() != EXPECTED_SHA256:
        errors.append("binary_sha256 does not match pinned build 870ebf")
    try:
        image_base = integer(document.get("image_base"), "image_base")
    except (TypeError, ValueError) as error:
        image_base = 0
        errors.append(str(error))
    try:
        game_hour = integer(document.get("paused_game_hour"), "paused_game_hour")
        if game_hour <= 0:
            errors.append("paused_game_hour must be positive")
    except (TypeError, ValueError) as error:
        game_hour = 0
        errors.append(str(error))

    reads = document.get("reads")
    if not isinstance(reads, list) or len(reads) != 8:
        errors.append("reads must contain exactly eight side rows")
        reads = []
    seen_keys: set[tuple[int, str]] = set()
    side_addresses: set[int] = set()
    combat_roles: dict[tuple[int, int, int, int], set[str]] = {}
    body_addresses: set[int] = set()
    decoded: list[dict[str, object]] = []
    for index, row in enumerate(reads):
        prefix = f"reads[{index}]"
        if not isinstance(row, dict):
            errors.append(f"{prefix} must be an object")
            continue
        try:
            location = integer(row.get("location_id"), f"{prefix}.location_id")
            combat_id = integer(row.get("combat_id"), f"{prefix}.combat_id")
            generation = integer(row.get("generation"), f"{prefix}.generation")
            body = integer(row.get("body_address"), f"{prefix}.body_address")
            body_vtable_first = integer(
                row.get("body_vtable_first"), f"{prefix}.body_vtable_first"
            )
            body_vtable_second = integer(
                row.get("body_vtable_second"), f"{prefix}.body_vtable_second"
            )
            body_side_pointer_first = integer(
                row.get("body_side_pointer_first"), f"{prefix}.body_side_pointer_first"
            )
            body_side_pointer_second = integer(
                row.get("body_side_pointer_second"), f"{prefix}.body_side_pointer_second"
            )
            side_address = integer(row.get("side_address"), f"{prefix}.side_address")
            vtable_first = integer(row.get("side_vtable_first"), f"{prefix}.side_vtable_first")
            vtable_second = integer(row.get("side_vtable_second"), f"{prefix}.side_vtable_second")
            raw_first = integer(row.get("raw_i64_first"), f"{prefix}.raw_i64_first")
            raw_second = integer(row.get("raw_i64_second"), f"{prefix}.raw_i64_second")
        except (TypeError, ValueError) as error:
            errors.append(str(error))
            continue
        role = row.get("side")
        if role not in ("attacker", "defender"):
            errors.append(f"{prefix}.side must be attacker or defender")
            continue
        key = (location, role)
        if key in seen_keys:
            errors.append(f"duplicate location/side key {key}")
        seen_keys.add(key)
        if side_address in side_addresses:
            errors.append(f"duplicate side_address 0x{side_address:x}")
        side_addresses.add(side_address)
        combat_key = (combat_id, generation, location, body)
        combat_roles.setdefault(combat_key, set()).add(role)
        body_addresses.add(body)
        expected_body_vtable = image_base + COMBAT_VTABLE_RVA
        if body_vtable_first != expected_body_vtable or body_vtable_second != expected_body_vtable:
            errors.append(f"{prefix} failed repeated CNavalCombat vtable gate")
        if body_side_pointer_first != side_address or body_side_pointer_second != side_address:
            errors.append(f"{prefix} body side pointer did not stably equal side_address")
        expected_vtable = image_base + SIDE_VTABLE_RVA
        if vtable_first != expected_vtable or vtable_second != expected_vtable:
            errors.append(f"{prefix} failed repeated CNavalCombatant vtable gate")
        if raw_first != raw_second:
            errors.append(f"{prefix} raw +0x150 changed between paused reads")
        if min(combat_id, generation, body, side_address) <= 0:
            errors.append(f"{prefix} has a non-positive identity/address")
        expected = EXPECTED_ORACLE.get(key)
        if expected is None:
            errors.append(f"{prefix} has unexpected oracle key {key}")
            continue
        try:
            captured_oracle = Decimal(str(row.get("oracle_decimal")))
        except InvalidOperation:
            errors.append(f"{prefix}.oracle_decimal is invalid")
            continue
        if captured_oracle != expected:
            errors.append(f"{prefix}.oracle_decimal {captured_oracle} != expected {expected}")
        expected_raw = int(expected * SCALE)
        if raw_first != expected_raw:
            errors.append(f"{prefix} raw {raw_first} != exact oracle raw {expected_raw}")
        decoded.append(
            {
                "location_id": location,
                "side": role,
                "raw_i64": raw_first,
                "decoded_decimal": str(Decimal(raw_first) / SCALE),
            }
        )
    if seen_keys != set(EXPECTED_ORACLE):
        errors.append("location/side keys do not exactly cover the four D04 combat oracles")
    if (
        len(combat_roles) != 4
        or len(body_addresses) != 4
        or any(roles != {"attacker", "defender"} for roles in combat_roles.values())
    ):
        errors.append(
            "combat identity tuples must form exactly four unique-body attacker/defender pairs"
        )
    return {
        "schema": "naval_combat_side_anti_air_fixture_verification_v1",
        "ok": not errors,
        "binary_sha256": str(document.get("binary_sha256", "")).lower(),
        "paused_game_hour": game_hour,
        "verified_side_count": len(decoded) if not errors else 0,
        "decoded": decoded,
        "errors": errors,
    }


def self_test() -> None:
    base = 0x140000000
    reads = []
    for index, ((location, side), value) in enumerate(EXPECTED_ORACLE.items()):
        raw = int(value * SCALE)
        reads.append(
            {
                "combat_id": 100 + index // 2,
                "generation": 1,
                "location_id": location,
                "side": side,
                "body_address": hex(0x10000000000 + (index // 2) * 0x1000),
                "body_vtable_first": hex(base + COMBAT_VTABLE_RVA),
                "body_vtable_second": hex(base + COMBAT_VTABLE_RVA),
                "side_address": hex(0x20000000000 + index * 0x400),
                "body_side_pointer_first": hex(0x20000000000 + index * 0x400),
                "body_side_pointer_second": hex(0x20000000000 + index * 0x400),
                "side_vtable_first": hex(base + SIDE_VTABLE_RVA),
                "side_vtable_second": hex(base + SIDE_VTABLE_RVA),
                "raw_i64_first": raw,
                "raw_i64_second": raw,
                "oracle_decimal": str(value),
            }
        )
    fixture = {
        "binary_sha256": EXPECTED_SHA256,
        "image_base": hex(base),
        "paused_game_hour": 60830441,
        "reads": reads,
    }
    assert verify(fixture)["ok"]
    fixture["reads"][0]["raw_i64_second"] = 1
    assert not verify(fixture)["ok"]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("fixture", type=Path, nargs="?")
    parser.add_argument("output", type=Path, nargs="?")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        print("anti-air fixture verifier self-test: PASS")
        return
    if args.fixture is None or args.output is None:
        parser.error("fixture and output are required unless --self-test is used")
    result = verify(json.loads(args.fixture.read_text(encoding="utf-8")))
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2))
    raise SystemExit(0 if result["ok"] else 1)


if __name__ == "__main__":
    main()
