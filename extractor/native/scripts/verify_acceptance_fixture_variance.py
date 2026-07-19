#!/usr/bin/env python3
"""Fail-closed verifier for Sequence-2 acceptance-fixture observations.

The production recorder folds one JSON snapshot per game hour into the replay
container.  This tool grades an *extracted* snapshot directory; it never infers
success from aggregate row counters in ``native_session_manifest.json``.

It asserts the four observations called out by LIVE_ACCEPTANCE_RUNBOOK:

* A16: the same subject relation changes autonomy state and/or ratio;
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


SCHEMA = "battle-plan.acceptance_fixture_variance.v1"
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
    seen: dict[tuple[int, int], tuple[dict[str, Any], tuple[int, int]]] = {}
    for snap in rows:
        if not domain_clean(snap["value"], (53,)):
            continue
        subject_rows = json_path(
            snap["value"], "diplomacy_world", "domain53_extra", "subject_autonomy_rows"
        )
        if subject_rows is None:
            continue
        if not isinstance(subject_rows, list):
            raise VerifyError("A16 subject_autonomy_rows must be an array")
        for index, row in enumerate(subject_rows):
            if not isinstance(row, dict):
                raise VerifyError("A16 subject row must be an object")
            overlord = strict_int(row.get("overlord_tag_db_id"), f"A16 row {index} overlord", 0)
            subject = strict_int(row.get("subject_tag_db_id"), f"A16 row {index} subject", 0)
            state_hash = strict_int(row.get("autonomy_state_hash"), f"A16 row {index} state", 1)
            ratio = strict_int(row.get("autonomy_ratio_raw"), f"A16 row {index} ratio", 0)
            if ratio > 100_000:
                raise VerifyError(f"A16 autonomy_ratio_raw out of range: {ratio}")
            key = (overlord, subject)
            state = (state_hash, ratio)
            if key in seen and seen[key][1] != state:
                before_snap, before_state = seen[key]
                return {
                    "passed": True,
                    "relation": {"overlord_tag_db_id": overlord, "subject_tag_db_id": subject},
                    "before": {**evidence_ref(before_snap), "autonomy_state_hash": before_state[0], "autonomy_ratio_raw": before_state[1]},
                    "after": {**evidence_ref(snap), "autonomy_state_hash": state_hash, "autonomy_ratio_raw": ratio},
                }
            seen.setdefault(key, (snap, state))
    return {"passed": False, "problem": "no same-relation autonomy state/ratio change observed"}


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
