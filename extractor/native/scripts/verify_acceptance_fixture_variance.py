#!/usr/bin/env python3
"""Fail-closed verifier for Sequence-2 acceptance-fixture observations.

The production recorder folds one JSON snapshot per game hour into the replay
container.  This tool grades an *extracted* snapshot directory; it never infers
success from aggregate row counters in ``native_session_manifest.json``.

It asserts the four observations called out by LIVE_ACCEPTANCE_RUNBOOK:

* A16: the same authenticated current-status row changes autonomy state and/or
  signed score;
* A8: every observed naval mission enum has the exact a729 name;
* A9: a naval combat present in a clean snapshot later disappears in a clean
  snapshot (a lifecycle end, not a SOURCE_UNAVAILABLE hour);
* A20: the same weather region's recorded phenomenon state changes.

All evidence rows are bound to snapshot file SHA-256 values in the output.
Unknown/missing shapes fail closed.  No tuned thresholds are used.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any, Iterable


SCHEMA = "battle-plan.acceptance_fixture_variance.v2"
SNAPSHOT_RE = re.compile(r"^(\d{12})\.snapshot\.json$")
MISSION_NAMES = (
    "HOLD",
    "PATROL",
    "STRIKE_FORCE",
    "CONVOY_RAIDING",
    "CONVOY_ESCORT",
    "MINES_PLANTING",
    "MINES_SWEEPING",
    "TRAINING",
    "RESERVE_FLEET",
    "NAVAL_INVASION_SUPPORT",
)
WEATHER_VALUE_KEYS = (
    "rain_light",
    "rain_heavy",
    "snow",
    "blizzard",
    "arctic_water",
    "active_signal",
    "phenomenon_active",
    "sandstorm",
)
I64_MIN = -(1 << 63)
I64_MAX = (1 << 63) - 1
U32_MAX = (1 << 32) - 1


class VerifyError(RuntimeError):
    pass


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def strict_int(value: Any, label: str, minimum: int | None = None) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise VerifyError(f"{label} must be an integer")
    if minimum is not None and value < minimum:
        raise VerifyError(f"{label} must be >= {minimum}")
    return value


def strict_i64(value: Any, label: str) -> int:
    value = strict_int(value, label)
    if not I64_MIN <= value <= I64_MAX:
        raise VerifyError(f"{label} must fit signed 64-bit")
    return value


def strict_u32(value: Any, label: str, minimum: int = 0) -> int:
    value = strict_int(value, label, minimum)
    if value > U32_MAX:
        raise VerifyError(f"{label} must fit unsigned 32-bit")
    return value


def checked_i64(value: int, label: str) -> int:
    if not I64_MIN <= value <= I64_MAX:
        raise VerifyError(f"{label} overflows signed 64-bit")
    return value


def load_snapshots(root: Path) -> list[dict[str, Any]]:
    if not root.is_dir():
        raise VerifyError(f"snapshot directory does not exist: {root}")
    rows: list[dict[str, Any]] = []
    for path in root.rglob("*.snapshot.json"):
        match = SNAPSHOT_RE.match(path.name)
        if match is None:
            raise VerifyError(f"noncanonical snapshot filename: {path}")
        try:
            value = json.loads(path.read_text(encoding="utf-8-sig"))
        except (OSError, UnicodeError, json.JSONDecodeError) as exc:
            raise VerifyError(f"unreadable snapshot {path}: {exc}") from exc
        if not isinstance(value, dict):
            raise VerifyError(f"snapshot root must be an object: {path}")
        rows.append(
            {
                "hour_index": strict_int(int(match.group(1)), f"{path}.hour_index", 0),
                "path": str(path.resolve()),
                "sha256": sha256_file(path),
                "value": value,
            }
        )
    rows.sort(key=lambda row: (row["hour_index"], row["path"]))
    if len(rows) < 2:
        raise VerifyError("at least two hourly snapshots are required")
    hours = [row["hour_index"] for row in rows]
    if len(set(hours)) != len(hours):
        raise VerifyError("duplicate snapshot hour_index values")
    for previous, current in zip(hours, hours[1:]):
        if current != previous + 1:
            raise VerifyError(
                f"snapshot hour_index sequence is not contiguous: {previous} -> {current}"
            )
    return rows


def json_path(value: Any, *parts: str) -> Any:
    current = value
    for part in parts:
        if not isinstance(current, dict) or part not in current:
            return None
        current = current[part]
    return current


def domain_clean(snapshot: dict[str, Any], domain_ids: Iterable[int]) -> bool:
    wanted = set(domain_ids)
    problems = json_path(snapshot, "provenance", "collector", "domain_problems")
    if problems is None:
        raise VerifyError("snapshot lacks provenance.collector.domain_problems")
    if not isinstance(problems, list):
        raise VerifyError("provenance.collector.domain_problems must be an array")
    for row in problems:
        if not isinstance(row, dict):
            raise VerifyError("domain problem row must be an object")
        domain = row.get("domain_id")
        if isinstance(domain, bool):
            raise VerifyError("domain problem domain_id must be an integer")
        if isinstance(domain, int) and domain in wanted:
            return False
    return True


def evidence_ref(snapshot_row: dict[str, Any]) -> dict[str, Any]:
    return {
        "hour_index": snapshot_row["hour_index"],
        "snapshot_path": snapshot_row["path"],
        "snapshot_sha256": snapshot_row["sha256"],
    }


def grade_a16(rows: list[dict[str, Any]]) -> dict[str, Any]:
    previous: dict[tuple[int, int], tuple[dict[str, Any], tuple[int, int]]] = {}
    for snap in rows:
        if not domain_clean(snap["value"], (53,)):
            previous.clear()
            continue
        status = json_path(
            snap["value"],
            "diplomacy_world",
            "domain53_extra",
            "subject_autonomy_current_status",
        )
        current_rows = json_path(
            snap["value"],
            "diplomacy_world",
            "domain53_extra",
            "subject_autonomy_current_rows",
        )
        if status in (None, "unavailable"):
            if current_rows is not None:
                raise VerifyError("A16 unavailable current status must not carry rows")
            previous.clear()
            continue
        if status == "measured_empty":
            if current_rows != []:
                raise VerifyError("A16 measured-empty current status must carry an empty row array")
            previous.clear()
            continue
        if status != "measured_nonempty":
            raise VerifyError(f"A16 current-status witness is invalid: {status!r}")
        if not isinstance(current_rows, list) or not current_rows:
            raise VerifyError("A16 measured-nonempty current status must carry a nonempty row array")

        base_rows = json_path(
            snap["value"],
            "diplomacy_world",
            "domain53_extra",
            "subject_autonomy_rows",
        )
        if not isinstance(base_rows, list):
            raise VerifyError("A16 subject_autonomy_rows must be an array")
        base_keys: set[tuple[int, int]] = set()
        for index, row in enumerate(base_rows):
            if not isinstance(row, dict):
                raise VerifyError("A16 base subject row must be an object")
            overlord = strict_u32(
                row.get("overlord_tag_db_id"), f"A16 base row {index} overlord", 1
            )
            subject = strict_u32(
                row.get("subject_tag_db_id"), f"A16 base row {index} subject", 1
            )
            key = (overlord, subject)
            if overlord == subject or key in base_keys:
                raise VerifyError(f"A16 base subject identity is invalid or duplicated: {key}")
            base_keys.add(key)

        current: dict[tuple[int, int], tuple[dict[str, Any], tuple[int, int]]] = {}
        transition: dict[str, Any] | None = None
        for index, row in enumerate(current_rows):
            if not isinstance(row, dict):
                raise VerifyError("A16 current-status row must be an object")
            overlord = strict_u32(row.get("overlord_tag_db_id"), f"A16 row {index} overlord", 1)
            subject = strict_u32(row.get("subject_tag_db_id"), f"A16 row {index} subject", 1)
            state_hash = strict_u32(row.get("current_state_hash"), f"A16 row {index} state", 1)
            previous_state = row.get("previous_state_hash")
            next_state = row.get("next_state_hash")
            if previous_state is not None:
                previous_state = strict_u32(
                    previous_state, f"A16 row {index} previous state", 1
                )
            if next_state is not None:
                next_state = strict_u32(next_state, f"A16 row {index} next state", 1)
            if (
                state_hash in (previous_state, next_state)
                or (previous_state is not None and previous_state == next_state)
            ):
                raise VerifyError(f"A16 row {index} state identities are not distinct")
            score = strict_i64(row.get("current_score_raw"), f"A16 row {index} score")
            lower = strict_i64(row.get("lower_score_raw"), f"A16 row {index} lower")
            upper = strict_i64(row.get("upper_score_raw"), f"A16 row {index} upper")
            ratio = strict_int(row.get("ratio_raw"), f"A16 row {index} ratio", 0)
            if lower >= upper:
                raise VerifyError(f"A16 score bounds are not increasing: {lower}..{upper}")
            if ratio > 100_000:
                raise VerifyError(f"A16 ratio_raw out of range: {ratio}")
            forward = max(checked_i64(score - lower, "A16 score-lower"), 0)
            remaining = max(checked_i64(upper - score, "A16 upper-score"), 0)
            denominator = checked_i64(forward + remaining, "A16 ratio denominator")
            scaled = checked_i64(forward * 100_000, "A16 ratio numerator")
            if denominator <= 0 or scaled // denominator != ratio:
                raise VerifyError("A16 ratio_raw disagrees with the exact current-score law")
            date_raw = strict_u32(
                row.get("last_modified_date_raw"),
                f"A16 row {index} last-modified date",
            )
            date = row.get("last_modified_date")
            if date is not None and (not isinstance(date, str) or not date or date_raw == 0):
                raise VerifyError(f"A16 row {index} last-modified date is invalid")
            if row.get("direction") not in (
                None,
                "toward_previous",
                "stable",
                "toward_next",
            ):
                raise VerifyError(f"A16 row {index} direction is invalid")
            key = (overlord, subject)
            if overlord == subject or key in current:
                raise VerifyError(f"A16 current-status identity is invalid or duplicated: {key}")
            state = (state_hash, score)
            current[key] = (snap, state)
            if transition is None and key in previous and previous[key][1] != state:
                before_snap, before_state = previous[key]
                transition = {
                    "passed": True,
                    "relation": {"overlord_tag_db_id": overlord, "subject_tag_db_id": subject},
                    "before": {**evidence_ref(before_snap), "current_state_hash": before_state[0], "current_score_raw": before_state[1]},
                    "after": {**evidence_ref(snap), "current_state_hash": state_hash, "current_score_raw": score},
                }
        if base_keys != set(current):
            raise VerifyError("A16 current-status keys do not match the base subject family")
        if transition is not None:
            return transition
        previous = current
    return {
        "passed": False,
        "problem": "no same-relation authenticated current-state/current-score change observed",
    }


def grade_a8(rows: list[dict[str, Any]]) -> dict[str, Any]:
    evidence: list[dict[str, Any]] = []
    for snap in rows:
        if not domain_clean(snap["value"], (56,)):
            continue
        extra = json_path(
            snap["value"], "order_of_battle", "extra", "naval_missions_status_live"
        )
        if extra is None:
            continue
        if not isinstance(extra, dict):
            raise VerifyError("A8 naval_missions_status_live must be an object")
        task_forces = extra.get("task_forces")
        if not isinstance(task_forces, list):
            raise VerifyError("A8 task_forces must be an array")
        for index, row in enumerate(task_forces):
            if not isinstance(row, dict):
                raise VerifyError("A8 task-force row must be an object")
            raw = row.get("mission_enum")
            name = row.get("mission_name")
            if raw is None and name is None:
                continue
            enum = strict_int(raw, f"A8 task force {index} mission_enum", 0)
            if enum >= len(MISSION_NAMES):
                raise VerifyError(f"A8 mission enum outside proven 0..9 range: {enum}")
            if name != MISSION_NAMES[enum]:
                raise VerifyError(
                    f"A8 mission name mismatch for enum {enum}: expected {MISSION_NAMES[enum]!r}, got {name!r}"
                )
            evidence.append(
                {
                    **evidence_ref(snap),
                    "task_force_id": strict_int(row.get("task_force_id"), "A8 task_force_id", 1),
                    "mission_enum": enum,
                    "mission_name": name,
                }
            )
    if not evidence:
        return {"passed": False, "problem": "no exact naval mission observation found"}
    return {"passed": True, "observation_count": len(evidence), "first_observation": evidence[0]}


def grade_a9(rows: list[dict[str, Any]]) -> dict[str, Any]:
    active: dict[str, dict[str, Any]] = {}
    for snap in rows:
        if not domain_clean(snap["value"], (49, 50)):
            continue
        combats = snap["value"].get("naval_combats")
        if combats is None:
            continue
        if not isinstance(combats, dict):
            raise VerifyError("A9 naval_combats must be an object")
        current = set(combats)
        ended = sorted(set(active) - current)
        if ended:
            battle = ended[0]
            return {
                "passed": True,
                "battle_id": battle,
                "active": evidence_ref(active[battle]),
                "ended": evidence_ref(snap),
            }
        for battle in current:
            active.setdefault(battle, snap)
    return {"passed": False, "problem": "no clean active-to-absent naval-combat lifecycle transition observed"}


def weather_fingerprint(row: dict[str, Any]) -> tuple[tuple[str, Any], ...]:
    values: list[tuple[str, bool]] = []
    for key in WEATHER_VALUE_KEYS:
        value = row.get(key)
        if not isinstance(value, bool):
            raise VerifyError(f"A20 weather flag {key} must be a boolean")
        values.append((key, value))
    return tuple(values)


def grade_a20(rows: list[dict[str, Any]]) -> dict[str, Any]:
    seen: dict[str, tuple[dict[str, Any], tuple[tuple[str, Any], ...]]] = {}
    for snap in rows:
        if not domain_clean(snap["value"], (24,)):
            continue
        regions = json_path(snap["value"], "map", "_r2_weather_live", "regions")
        if regions is None:
            continue
        if not isinstance(regions, dict):
            raise VerifyError("A20 weather regions must be an object")
        for region_id, row in regions.items():
            if not isinstance(row, dict):
                raise VerifyError("A20 weather region row must be an object")
            fingerprint = weather_fingerprint(row)
            if region_id in seen and seen[region_id][1] != fingerprint:
                before_snap, before = seen[region_id]
                return {
                    "passed": True,
                    "region_id": region_id,
                    "before": {**evidence_ref(before_snap), "values": dict(before)},
                    "after": {**evidence_ref(snap), "values": dict(fingerprint)},
                }
            seen.setdefault(region_id, (snap, fingerprint))
    return {"passed": False, "problem": "no same-region weather-state change observed"}


def verify(snapshot_dir: Path) -> dict[str, Any]:
    rows = load_snapshots(snapshot_dir)
    inventory_sha256 = hashlib.sha256(
        "".join(f"{row['hour_index']}:{row['sha256']}\n" for row in rows).encode("ascii")
    ).hexdigest()
    checks = {
        "A16_subject_autonomy_change": grade_a16(rows),
        "A8_naval_mission_semantics": grade_a8(rows),
        "A9_naval_combat_lifecycle_end": grade_a9(rows),
        "A20_weather_drift": grade_a20(rows),
    }
    problems = [f"{name}: {check.get('problem', 'failed')}" for name, check in checks.items() if check.get("passed") is not True]
    return {
        "schema": SCHEMA,
        "snapshot_dir": str(snapshot_dir.resolve()),
        "snapshot_count": len(rows),
        "snapshot_inventory_sha256": inventory_sha256,
        "first_hour_index": rows[0]["hour_index"],
        "last_hour_index": rows[-1]["hour_index"],
        "checks": checks,
        "problems": problems,
        "ok": not problems,
    }


def write_report(path: Path, report: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--snapshot-dir", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        report = verify(args.snapshot_dir)
    except VerifyError as exc:
        report = {"schema": SCHEMA, "snapshot_dir": str(args.snapshot_dir), "checks": {}, "problems": [str(exc)], "ok": False}
    write_report(args.out, report)
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["ok"] else 2


if __name__ == "__main__":
    sys.exit(main())
