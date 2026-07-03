#!/usr/bin/env python3
"""Build a per-division save oracle for 870ebf division discovery probes.

The output is development evidence only. It is intended to be paired with live
read-only probes against the same paused save frame. It does not close any
production extraction row by itself.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import sys
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[3]
SAVE_ORACLE = REPO_ROOT / "extractor" / "save_oracle"
if str(SAVE_ORACLE) not in sys.path:
    sys.path.insert(0, str(SAVE_ORACLE))

import parse_divisions_full  # type: ignore  # noqa: E402
import parse_combats  # type: ignore  # noqa: E402
import parse_oob  # type: ignore  # noqa: E402
import parse_save as ps  # type: ignore  # noqa: E402
from parse_save import (  # type: ignore  # noqa: E402
    EQUALS,
    LBRACE,
    RBRACE,
    Tokenizer,
    atom_value,
    iter_block_entries,
    _skip_block_body,
)


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
SCHEMA = "battle-plan.division_save_oracle_870ebf.v1"
STRATEGIC_REDEPLOY_KEYWORDS = ("strategic", "redeploy", "transport", "embark")


class OracleValidationError(RuntimeError):
    """Raised when a save shape is unsafe for oracle-backed live probing."""


def id_key(value: Any) -> str | None:
    if not isinstance(value, str) or ":" not in value:
        return None
    left, right = value.split(":", 1)
    try:
        return f"{int(left)}:{int(right)}"
    except ValueError:
        return None


def read_id_block(tz: Tokenizer) -> tuple[int | None, int | None]:
    object_id = None
    object_type = None
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        value = atom_value(opener)
        if value is None:
            continue
        try:
            parsed = int(float(value))
        except ValueError:
            continue
        if key == "id":
            object_id = parsed
        elif key == "type":
            object_type = parsed
    return object_id, object_type


def scalar_value(opener: Any) -> Any:
    value = atom_value(opener)
    if value is None:
        return None
    if value in ("yes", "no"):
        return value == "yes"
    try:
        if any(ch in value for ch in ".eE"):
            return float(value)
        return int(value)
    except ValueError:
        return value


def parse_division_hint_block(tz: Tokenizer) -> dict[str, Any] | None:
    div_id = None
    div_type = None
    first_id = False
    hints: dict[str, Any] = {}
    scalar_keys: list[str] = []
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "id" and not first_id:
                div_id, div_type = read_id_block(tz)
                first_id = True
            else:
                _skip_block_body(tz)
            continue
        if opener is None:
            continue
        scalar_keys.append(key)
        low = key.lower()
        if any(part in low for part in STRATEGIC_REDEPLOY_KEYWORDS):
            hints[key] = scalar_value(opener)
    if div_id is None:
        return None
    return {
        "division_key": f"{div_id}:{div_type}",
        "strategic_redeploy_save_hints": hints,
        "scalar_key_count": len(scalar_keys),
    }


def parse_division_hints(text: str) -> dict[str, dict[str, Any]]:
    out: dict[str, dict[str, Any]] = {}

    def parse_units(tz: Tokenizer) -> None:
        for key, opener in iter_block_entries(tz):
            if opener is LBRACE:
                if key == "division":
                    row = parse_division_hint_block(tz)
                    if row is not None:
                        out[row["division_key"]] = row
                else:
                    _skip_block_body(tz)

    def parse_country(tz: Tokenizer) -> None:
        for key, opener in iter_block_entries(tz):
            if opener is LBRACE:
                if key == "units":
                    parse_units(tz)
                else:
                    _skip_block_body(tz)

    def parse_countries(tz: Tokenizer) -> None:
        for _tag, opener in iter_block_entries(tz):
            if opener is LBRACE:
                parse_country(tz)

    tz = Tokenizer(text)
    while True:
        tok = tz.next()
        if tok is None:
            break
        if tok in (LBRACE, RBRACE):
            if tok is LBRACE:
                _skip_block_body(tz)
            continue
        key = atom_value(tok)
        if key is None or tz.peek() != "=":
            continue
        tz.next()
        value = tz.next()
        if value is LBRACE:
            if key == "countries":
                parse_countries(tz)
            else:
                _skip_block_body(tz)
    return out


def parse_army_id(army: dict[str, Any]) -> int | None:
    key = id_key(army.get("id"))
    if key is None:
        return None
    return int(key.split(":", 1)[0])


def build_assignment_maps(oob: dict[str, Any]) -> tuple[dict[str, dict[str, Any]], dict[str, list[dict[str, Any]]]]:
    army_by_division: dict[str, dict[str, Any]] = {}
    orders_by_division: dict[str, list[dict[str, Any]]] = {}
    for army in oob.get("armies", []):
        army_id = parse_army_id(army)
        if army_id is None:
            continue
        army_key = id_key(army.get("id"))
        members = [key for key in (id_key(value) for value in army.get("members", [])) if key]
        for index, division_key in enumerate(members):
            army_by_division[division_key] = {
                "army_id": army_id,
                "army_key": army_key,
                "army_owner": army.get("owner"),
                "army_name": army.get("name"),
                "army_member_index": index,
                "army_member_count": len(members),
                "army_order_count": len(army.get("orders", [])),
            }
        for slot_index, order in enumerate(army.get("orders", [])):
            for member in order.get("scheduled_members", []):
                division_key = id_key(member)
                if division_key is None:
                    continue
                orders_by_division.setdefault(division_key, []).append(
                    {
                        "army_id": army_id,
                        "army_key": army_key,
                        "slot_index": slot_index,
                        "order_type": order.get("type"),
                        "instance_id": order.get("instance_id"),
                        "operation": order.get("operation"),
                        "path_count": len(order.get("path", [])),
                        "state_count": len(order.get("states", [])),
                        "scheduled_member_count": len(order.get("scheduled_members", [])),
                        "can_execute": order.get("can_execute"),
                    }
                )
    return army_by_division, orders_by_division


def compact_division_ref(row: dict[str, Any]) -> dict[str, Any]:
    return {
        "owner_division_key": row.get("owner_division_key"),
        "division_key": row.get("division_key"),
        "owner": row.get("owner"),
        "div_id": row.get("div_id"),
        "div_type": row.get("div_type"),
        "division_name": row.get("division_name"),
        "location_province": row.get("location_province"),
        "oracle_current_manpower": row.get("oracle_current_manpower"),
        "oracle_cumulative_killed": row.get("oracle_cumulative_killed"),
    }


def build_combat_fixtures(combats: list[dict[str, Any]], rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_division_key: dict[str, list[dict[str, Any]]] = {}
    for row in rows:
        key = row.get("division_key")
        if isinstance(key, str):
            by_division_key.setdefault(key, []).append(row)

    fixtures: list[dict[str, Any]] = []
    for combat in combats:
        if combat.get("kind") != "land":
            continue
        attacker = combat.get("attacker") if isinstance(combat.get("attacker"), dict) else {}
        defender = combat.get("defender") if isinstance(combat.get("defender"), dict) else {}
        attacker_units = attacker.get("units") if isinstance(attacker, dict) else []
        defender_units = defender.get("units") if isinstance(defender, dict) else []
        if not attacker_units or not defender_units:
            continue

        def watch(side_units: Any) -> list[dict[str, Any]]:
            out = []
            for ref in side_units:
                if not isinstance(ref, list) or len(ref) < 2:
                    continue
                div_key = f"{ref[0]}:{ref[1]}"
                matches = by_division_key.get(div_key, [])
                if matches:
                    out.append(compact_division_ref(matches[0]))
                else:
                    out.append({"division_key": div_key, "unresolved_in_division_oracle": True})
                if len(out) >= 3:
                    break
            return out

        fixtures.append(
            {
                "combat_id": combat.get("combat_id"),
                "location_province": combat.get("location"),
                "terrain": combat.get("terrain"),
                "day": combat.get("day"),
                "duration": combat.get("duration"),
                "attacker_unit_count": attacker.get("unit_count"),
                "defender_unit_count": defender.get("unit_count"),
                "attacker_watch": watch(attacker_units),
                "defender_watch": watch(defender_units),
                "fixture_rule": "Use this exact combat location and these watch divisions for the two-poll loss probe. If the UI shows the battle ended before polling, mark the fixture blocked.",
            }
        )
    return fixtures


def current_manpower(row: dict[str, Any]) -> int | None:
    manpower = row.get("army_manpower")
    if not isinstance(manpower, dict):
        return None
    value = manpower.get("value")
    return value if isinstance(value, int) and value >= 0 else None


def manpower_need(row: dict[str, Any]) -> int | None:
    manpower = row.get("army_manpower")
    if not isinstance(manpower, dict):
        return None
    value = manpower.get("need")
    return value if isinstance(value, int) and value >= 0 else None


def skip_block_checked(tz: Tokenizer, context: str) -> None:
    depth = 1
    while depth > 0:
        tok = tz.next()
        if tok is None:
            raise OracleValidationError(f"{context}: missing closing brace")
        if tok is LBRACE:
            depth += 1
        elif tok is RBRACE:
            depth -= 1


def iter_block_entries_checked(tz: Tokenizer, context: str):
    while True:
        tok = tz.next()
        if tok is None:
            raise OracleValidationError(f"{context}: missing closing brace")
        if tok is RBRACE:
            return
        if tok is LBRACE:
            skip_block_checked(tz, f"{context}.anonymous_block")
            continue
        key = atom_value(tok)
        if key is None:
            continue
        nxt = tz.peek()
        if nxt is not EQUALS:
            yield key, None
            continue
        tz.next()
        vpeek = tz.peek()
        if vpeek is RBRACE:
            raise OracleValidationError(f"{context}.{key}: missing value before closing brace")
        val = tz.next()
        if val is None:
            raise OracleValidationError(f"{context}.{key}: missing value at EOF")
        yield key, val


def checked_id_block_present(tz: Tokenizer, context: str) -> bool:
    seen_id = False
    seen_type = False
    for key, opener in iter_block_entries_checked(tz, context):
        if opener is LBRACE:
            skip_block_checked(tz, f"{context}.{key}")
            continue
        value = atom_value(opener)
        if value is None:
            continue
        if key == "id":
            seen_id = True
        elif key == "type":
            seen_type = True
    return seen_id and seen_type


def block_has_scalar_key(tz: Tokenizer, context: str, target: str) -> bool:
    found = False
    for key, opener in iter_block_entries_checked(tz, context):
        if opener is LBRACE:
            found = block_has_scalar_key(tz, f"{context}.{key}", target) or found
            continue
        if key == target and atom_value(opener) is not None:
            found = True
    return found


def scan_army_manpower_block(tz: Tokenizer, context: str) -> tuple[bool, bool]:
    has_value = False
    has_need = False
    for key, opener in iter_block_entries_checked(tz, context):
        if opener is not LBRACE:
            continue
        if key == "army_manpower_value":
            has_value = block_has_scalar_key(tz, f"{context}.{key}", "value")
        elif key == "army_manpower_need":
            has_need = block_has_scalar_key(tz, f"{context}.{key}", "value")
        else:
            skip_block_checked(tz, f"{context}.{key}")
    return has_value, has_need


def scan_division_block(tz: Tokenizer, context: str, stats: dict[str, int]) -> None:
    stats["division_blocks"] += 1
    has_id = False
    has_manpower_value = False
    has_manpower_need = False
    has_killed = False
    for key, opener in iter_block_entries_checked(tz, context):
        if opener is LBRACE:
            if key == "id":
                has_id = checked_id_block_present(tz, f"{context}.id")
            elif key == "army_manpower":
                has_manpower_value, has_manpower_need = scan_army_manpower_block(
                    tz, f"{context}.army_manpower"
                )
            else:
                skip_block_checked(tz, f"{context}.{key}")
            continue
        if key == "killed" and atom_value(opener) is not None:
            has_killed = True
    if has_id:
        stats["division_id_blocks"] += 1
    if has_manpower_value:
        stats["division_army_manpower_value_blocks"] += 1
    if has_manpower_need:
        stats["division_army_manpower_need_blocks"] += 1
    if has_killed:
        stats["division_killed_scalars"] += 1


def scan_members_block(tz: Tokenizer, context: str) -> int:
    unit_refs = 0
    for key, opener in iter_block_entries_checked(tz, context):
        if opener is not LBRACE:
            continue
        if key != "member":
            skip_block_checked(tz, f"{context}.{key}")
            continue
        member_has_unit = False
        for member_key, member_opener in iter_block_entries_checked(tz, f"{context}.member"):
            if member_opener is LBRACE:
                if member_key == "unit":
                    member_has_unit = checked_id_block_present(tz, f"{context}.member.unit")
                else:
                    skip_block_checked(tz, f"{context}.member.{member_key}")
        if member_has_unit:
            unit_refs += 1
    return unit_refs


def scan_orders_group_block(tz: Tokenizer, context: str, stats: dict[str, int]) -> None:
    stats["orders_group_blocks"] += 1
    for key, opener in iter_block_entries_checked(tz, context):
        if opener is not LBRACE:
            continue
        if key == "members":
            stats["oob_member_unit_refs"] += scan_members_block(tz, f"{context}.members")
        else:
            skip_block_checked(tz, f"{context}.{key}")


def walk_checked_block(tz: Tokenizer, context: str, stats: dict[str, int], in_units: bool = False) -> None:
    for key, opener in iter_block_entries_checked(tz, context):
        if opener is not LBRACE:
            continue
        child = f"{context}.{key}"
        if in_units and key == "division":
            scan_division_block(tz, child, stats)
        elif key == "orders_group":
            scan_orders_group_block(tz, child, stats)
        else:
            walk_checked_block(tz, child, stats, in_units=(key == "units"))


def scan_structural_expectations(text: str) -> dict[str, int]:
    stats = {
        "division_blocks": 0,
        "division_id_blocks": 0,
        "division_army_manpower_value_blocks": 0,
        "division_army_manpower_need_blocks": 0,
        "division_killed_scalars": 0,
        "orders_group_blocks": 0,
        "oob_member_unit_refs": 0,
    }
    tz = Tokenizer(text)
    while True:
        tok = tz.next()
        if tok is None:
            return stats
        if tok in (LBRACE, RBRACE):
            if tok is LBRACE:
                skip_block_checked(tz, "top_level.anonymous_block")
            continue
        key = atom_value(tok)
        if key is None or tz.peek() != "=":
            continue
        tz.next()
        value = tz.next()
        if value is None:
            raise OracleValidationError(f"top_level.{key}: missing value at EOF")
        if value is LBRACE:
            if key == "orders_group":
                scan_orders_group_block(tz, f"top_level.{key}", stats)
            else:
                walk_checked_block(tz, f"top_level.{key}", stats)


def validate_oracle_shape(
    *,
    structural: dict[str, int],
    divisions: list[dict[str, Any]],
    stats: dict[str, int],
) -> str:
    expected = structural["division_blocks"]
    parsed = len(divisions)
    if expected == 0:
        if parsed != 0:
            raise OracleValidationError(
                f"division_parse_mismatch: structural scan found 0 division blocks but parser returned {parsed}"
            )
        return "legitimately_empty_no_division_blocks"
    if parsed != expected:
        raise OracleValidationError(
            f"division_parse_mismatch: structural scan found {expected} division blocks but parser returned {parsed}"
        )
    checks = (
        ("division_id_blocks", "division id blocks"),
        ("division_army_manpower_value_blocks", "army_manpower value blocks"),
        ("division_army_manpower_need_blocks", "army_manpower need blocks"),
        ("division_killed_scalars", "killed scalars"),
    )
    for key, label in checks:
        observed = structural[key]
        if observed != expected:
            raise OracleValidationError(
                f"division_block_malformed: expected {expected} {label}, found {observed}"
            )
    if stats["current_manpower_rows"] != expected:
        raise OracleValidationError(
            f"army_manpower_parse_failed: expected {expected} current manpower rows, parsed {stats['current_manpower_rows']}"
        )
    if stats["manpower_need_rows"] != expected:
        raise OracleValidationError(
            f"army_manpower_need_parse_failed: expected {expected} manpower need rows, parsed {stats['manpower_need_rows']}"
        )
    if stats["cumulative_killed_rows"] != expected:
        raise OracleValidationError(
            f"killed_parse_failed: expected {expected} killed rows, parsed {stats['cumulative_killed_rows']}"
        )
    if structural["oob_member_unit_refs"] <= 0:
        raise OracleValidationError(
            f"oob_army_membership_missing_or_malformed: save has {expected} division blocks but no orders_group.members.member.unit refs"
        )
    if stats["assignment_rows"] <= 0:
        raise OracleValidationError(
            "oob_army_membership_parse_failed: structural scan found member.unit refs but parser produced 0 assignment rows"
        )
    return "validated_expected_division_blocks"


def build_oracle(text: str, save_path: Path, was_melted: bool) -> dict[str, Any]:
    structural = scan_structural_expectations(text)
    divisions, div_meta = parse_divisions_full.parse_text(text)
    combats, combat_kind_counts, _combat_meta = parse_combats.parse_combats_text(text)
    store = parse_oob.parse_oob_text(text)
    oob = parse_oob.build_oob_outputs(store, {}, save_path, was_melted)
    army_by_division, orders_by_division = build_assignment_maps(oob)
    hints = parse_division_hints(text)

    rows: list[dict[str, Any]] = []
    by_owner_key: dict[str, dict[str, Any]] = {}
    duplicate_owner_keys = 0
    for div in divisions:
        div_id = div.get("div_id")
        div_type = div.get("div_type")
        owner = div.get("owner")
        if not isinstance(div_id, int) or not isinstance(div_type, int) or not isinstance(owner, str):
            continue
        division_key = f"{div_id}:{div_type}"
        owner_key = f"{owner}|{division_key}"
        assignment = army_by_division.get(division_key, {})
        row = {
            "owner_division_key": owner_key,
            "division_key": division_key,
            "div_id": div_id,
            "div_type": div_type,
            "owner": owner,
            "location_province": div.get("location_province"),
            "division_name": div.get("division_name"),
            "template_id": div.get("template_id"),
            "template_type": div.get("template_type"),
            "organisation": div.get("organisation"),
            "strength": div.get("strength"),
            "experience": div.get("experience"),
            "oracle_current_manpower": current_manpower(div),
            "oracle_manpower_need": manpower_need(div),
            "oracle_cumulative_killed": div.get("killed") if isinstance(div.get("killed"), int) else None,
            "oracle_fields": {
                "current_manpower_source": "countries.<TAG>.units.division.army_manpower.army_manpower_value.value",
                "manpower_need_source": "countries.<TAG>.units.division.army_manpower.army_manpower_need.value",
                "cumulative_killed_source": "countries.<TAG>.units.division.killed",
            },
            "assignment": assignment or None,
            "scheduled_orders": orders_by_division.get(division_key, []),
            "strategic_redeploy_save_hints": (
                hints.get(division_key, {}).get("strategic_redeploy_save_hints", {})
            ),
        }
        if owner_key in by_owner_key:
            duplicate_owner_keys += 1
        by_owner_key[owner_key] = row
        rows.append(row)

    with_current = [r for r in rows if r["oracle_current_manpower"] is not None]
    with_need = [r for r in rows if r["oracle_manpower_need"] is not None]
    divergent_current_need = [
        r
        for r in rows
        if isinstance(r["oracle_current_manpower"], int)
        and isinstance(r["oracle_manpower_need"], int)
        and r["oracle_current_manpower"] != r["oracle_manpower_need"]
    ]
    with_killed = [r for r in rows if isinstance(r["oracle_cumulative_killed"], int)]
    nonzero_killed = [r for r in with_killed if r["oracle_cumulative_killed"] > 0]
    with_assignment = [r for r in rows if r["assignment"]]
    with_scheduled_orders = [r for r in rows if r["scheduled_orders"]]
    with_sr_hints = [r for r in rows if r["strategic_redeploy_save_hints"]]
    combat_fixtures = build_combat_fixtures(combats, rows)

    stats = {
        "division_rows": len(rows),
        "current_manpower_rows": len(with_current),
        "manpower_need_rows": len(with_need),
        "current_need_divergent_rows": len(divergent_current_need),
        "cumulative_killed_rows": len(with_killed),
        "nonzero_cumulative_killed_rows": len(nonzero_killed),
        "assignment_rows": len(with_assignment),
        "scheduled_order_rows": len(with_scheduled_orders),
        "strategic_redeploy_hint_rows": len(with_sr_hints),
        "duplicate_owner_division_keys": duplicate_owner_keys,
        "armies": len(oob.get("armies", [])),
        "army_groups": len(oob.get("army_groups", [])),
        "battle_plan_orders": sum(len(a.get("orders", [])) for a in oob.get("armies", [])),
        "active_land_combats": int(combat_kind_counts.get("land", 0)),
        "loss_probe_combat_fixture_rows": len(combat_fixtures),
    }
    validation_status = validate_oracle_shape(
        structural=structural,
        divisions=divisions,
        stats=stats,
    )

    return {
        "schema": SCHEMA,
        "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "expected_live_exe_sha256": EXPECTED_SHA256,
        "validation_status": validation_status,
        "source": {
            "path": str(save_path),
            "path_redacted": ps._redact(save_path),
            "was_melted": was_melted,
            "save_date": div_meta.get("date"),
            "save_version": div_meta.get("version"),
            "player": div_meta.get("player"),
        },
        "structural_expectations": structural,
        "stats": stats,
        "combat_fixtures": combat_fixtures,
        "notes": {
            "current_manpower": "Uses the literal save field army_manpower.army_manpower_value.value.",
            "manpower_need": "Uses the literal save field army_manpower.army_manpower_need.value.",
            "losses": "Uses the literal save field killed as cumulative division combat-history loss evidence when present.",
            "assignment": "Uses save OOB armies whose member.unit references contain the division id/type.",
            "strategic_redeploy": "The oracle records only save scalar keys containing strategic/redeploy/transport/embark. Absence is not proof of no strategic redeploy state.",
        },
        "rows": rows,
        "by_owner_division_key": by_owner_key,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--save", type=Path, default=None)
    parser.add_argument("--rakaly", type=Path, default=None)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--melted-copy", type=Path, default=None)
    args = parser.parse_args(argv)

    save_path = args.save or ps.find_latest_save()
    if save_path is None:
        raise SystemExit("no save supplied and no save found")
    if not save_path.exists():
        raise SystemExit(f"save not found: {save_path}")
    out_dir = args.out.parent
    out_dir.mkdir(parents=True, exist_ok=True)
    rakaly = ps.find_rakaly(str(args.rakaly) if args.rakaly else None)
    text, was_melted = ps.melt_if_needed(save_path, rakaly, out_dir)
    oracle = build_oracle(text, save_path, was_melted)
    if args.melted_copy:
        args.melted_copy.parent.mkdir(parents=True, exist_ok=True)
        args.melted_copy.write_text(text, encoding="utf-8", errors="replace")
    args.out.write_text(json.dumps(oracle, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                "status": "wrote_oracle",
                "out": str(args.out),
                "save_date": oracle["source"]["save_date"],
                "stats": oracle["stats"],
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
