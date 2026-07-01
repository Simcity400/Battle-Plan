"""Same-frame save/UI oracle observer for domain 48 construction target state.

This is a lane-local development fixture tool. It is not a live extraction path.
It captures the existing focused live proof from a specific hoi4.exe PID, waits
for or consumes a save made from the same paused frame, parses construction
general_lines from that save, and writes a filled oracle JSON that
target_state_oracle_contract.rs can merge.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from collections import defaultdict
from decimal import Decimal, ROUND_HALF_UP
from pathlib import Path
from typing import Any

PROBE_DIR = Path(__file__).resolve().parent
LANE_DIR = PROBE_DIR.parent
REPO_ROOT = LANE_DIR.parent.parent.parent
SAVE_ORACLE_DIR = REPO_ROOT / "save_oracle"

sys.path.insert(0, str(PROBE_DIR))
sys.path.insert(0, str(SAVE_ORACLE_DIR))

import deep_common as dc  # type: ignore  # noqa: E402
import parse_save as ps  # type: ignore  # noqa: E402
import target_candidate_focus_probe as focus_probe  # type: ignore  # noqa: E402

SCHEMA = "construction_buildings_state_damage_live_target_state_same_frame_observer_v1"
ORACLE_SCHEMA = "construction_buildings_state_damage_live_target_state_same_frame_oracle_v1"
FIXED_SCALE = Decimal("100000")
PLACEHOLDER = "FILL_FROM_TIME_ALIGNED_SAVE_OR_UI_ORACLE"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out-dir", required=True)
    parser.add_argument("--pid", type=int, default=None)
    parser.add_argument("--focus-proof", default=None)
    parser.add_argument("--save", default=None)
    parser.add_argument("--rakaly", default=None)
    parser.add_argument("--max-regions", type=int, default=None)
    parser.add_argument("--timeout-s", type=float, default=600.0)
    parser.add_argument("--poll-s", type=float, default=2.0)
    parser.add_argument("--stable-s", type=float, default=2.0)
    args = parser.parse_args()

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    started_wall = time.time()
    focus_path = out_dir / "target_candidate_focus_probe.json"
    if args.focus_proof:
        focus_path = Path(args.focus_proof)
        focus = read_json(focus_path)
    else:
        focus = focus_probe.run_probe(pid=args.pid, max_regions=args.max_regions)
        write_json(focus_path, focus)

    save_wait_started = time.time()
    save_path = Path(args.save) if args.save else wait_for_save_after(
        save_wait_started,
        timeout_s=args.timeout_s,
        poll_s=args.poll_s,
        stable_s=args.stable_s,
    )

    proof: dict[str, Any]
    oracle_path = out_dir / "filled_target_state_oracle.json"
    if save_path is None:
        unmatched = annotated_fixture_rows(focus, "no_save_after_live_focus_proof")
        oracle = build_empty_oracle(focus, focus_path, unmatched)
        proof = observer_proof(
            status="external_action_required_no_same_frame_save",
            focus=focus,
            focus_path=focus_path,
            oracle_path=oracle_path,
            save_path=None,
            save_meta=None,
            rows=[],
            unmatched=unmatched,
            started_wall=started_wall,
            extra={
                "failure": "no save appeared after the live focus proof",
                "external_action_required": external_action_required(),
            },
        )
        write_json(oracle_path, oracle)
        write_json(out_dir / "same_frame_oracle_observer_proof.json", proof)
        print(summary_line(proof))
        return 2

    rakaly = ps.find_rakaly(args.rakaly)
    try:
        text, was_melted = ps.melt_if_needed(save_path, rakaly, out_dir)
    except RuntimeError as exc:
        unmatched = annotated_fixture_rows(focus, "save_unreadable")
        oracle = build_empty_oracle(focus, focus_path, unmatched)
        proof = observer_proof(
            status="external_action_required_save_unreadable",
            focus=focus,
            focus_path=focus_path,
            oracle_path=oracle_path,
            save_path=save_path,
            save_meta=None,
            rows=[],
            unmatched=unmatched,
            started_wall=started_wall,
            extra={
                "failure": str(exc),
                "external_action_required": external_action_required(),
            },
        )
        write_json(oracle_path, oracle)
        write_json(out_dir / "same_frame_oracle_observer_proof.json", proof)
        print(summary_line(proof))
        return 3

    save_rows, save_meta = parse_save_construction_rows(text)
    save_meta = {
        **save_meta,
        "source_path": str(save_path),
        "source_filename": save_path.name,
        "was_melted": was_melted,
        "construction_rows": len(save_rows),
    }

    filled_rows, unmatched_rows, duplicate_line_ids = fill_oracle_rows(
        focus,
        save_rows,
    )
    oracle = {
        "schema": ORACLE_SCHEMA,
        "focus_proof": path_slash(focus_path),
        "save_oracle": {
            "source_path": path_slash(save_path),
            "source_filename": save_path.name,
            "was_melted": was_melted,
            "save_date": save_meta.get("date"),
            "save_version": save_meta.get("version"),
            "player": save_meta.get("player"),
            "construction_rows": len(save_rows),
            "duplicate_line_ids": duplicate_line_ids[:32],
        },
        "rows": filled_rows,
        "unmatched_required_rows": unmatched_rows,
    }
    write_json(oracle_path, oracle)

    unique_expected = sorted(
        {
            row["expected_target_state_id"]
            for row in filled_rows
            if isinstance(row.get("expected_target_state_id"), int)
        }
    )
    status = (
        "filled_oracle_ready_for_contract_gate"
        if len(filled_rows) >= 4 and len(unique_expected) >= 2 and not unmatched_rows
        else "external_action_required_save_not_same_frame_or_insufficient_rows"
    )
    proof = observer_proof(
        status=status,
        focus=focus,
        focus_path=focus_path,
        oracle_path=oracle_path,
        save_path=save_path,
        save_meta=save_meta,
        rows=filled_rows,
        unmatched=unmatched_rows,
        started_wall=started_wall,
        extra={
            "filled_rows": len(filled_rows),
            "required_rows": len(fixture_rows(focus)),
            "unique_expected_target_states": unique_expected,
            "duplicate_save_line_ids": duplicate_line_ids[:32],
            "external_action_required": (
                [] if status == "filled_oracle_ready_for_contract_gate"
                else external_action_required()
            ),
        },
    )
    write_json(out_dir / "same_frame_oracle_observer_proof.json", proof)
    print(summary_line(proof))
    return 0 if status == "filled_oracle_ready_for_contract_gate" else 4


def fixture_rows(focus: dict[str, Any]) -> list[dict[str, Any]]:
    return list(focus.get("fixture_request", {}).get("rows") or [])


def annotated_fixture_rows(focus: dict[str, Any], failure: str) -> list[dict[str, Any]]:
    return [{**row, "observer_failure": failure} for row in fixture_rows(focus)]


def build_empty_oracle(
    focus: dict[str, Any],
    focus_path: Path,
    unmatched: list[dict[str, Any]],
) -> dict[str, Any]:
    return {
        "schema": ORACLE_SCHEMA,
        "focus_proof": path_slash(focus_path),
        "rows": [],
        "unmatched_required_rows": unmatched,
    }


def observer_proof(
    *,
    status: str,
    focus: dict[str, Any],
    focus_path: Path,
    oracle_path: Path,
    save_path: Path | None,
    save_meta: dict[str, Any] | None,
    rows: list[dict[str, Any]],
    unmatched: list[dict[str, Any]],
    started_wall: float,
    extra: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema": SCHEMA,
        "status": status,
        "duration_ms": round((time.time() - started_wall) * 1000),
        "focus_proof": {
            "path": path_slash(focus_path),
            "schema": focus.get("schema"),
            "build_profile": focus.get("build_profile"),
            "process": focus.get("process"),
            "fingerprint": focus.get("fingerprint"),
            "verdict": focus.get("verdict"),
            "construction_lines": focus.get("construction_queue", {}).get("line_count"),
            "fixture_rows": len(fixture_rows(focus)),
        },
        "save": {
            "path": path_slash(save_path) if save_path else None,
            "meta": save_meta,
        },
        "oracle_output": {
            "path": path_slash(oracle_path),
            "schema": ORACLE_SCHEMA,
            "filled_rows": len(rows),
            "unmatched_rows": len(unmatched),
            "target_state_oracle_contract_command": (
                "cargo run --manifest-path "
                "replay_native\\lanes\\construction_buildings_state_damage_live\\Cargo.toml "
                "--bin construction_buildings_state_damage_target_state_oracle_contract "
                f"-- {path_slash(focus_path)} <out-dir> {path_slash(oracle_path)}"
            ),
        },
        **extra,
    }


def external_action_required() -> list[str]:
    return [
        "Pause the game before running the observer.",
        "Run this observer with --pid <hoi4-pid> and no --save argument.",
        "After it finishes the focused live proof and starts waiting, create a manual save without unpausing.",
        "If the game advances or construction queues change before the save is written, rerun the observer from the paused frame.",
    ]


def summary_line(proof: dict[str, Any]) -> str:
    return json.dumps(
        {
            "status": proof["status"],
            "filled_rows": proof["oracle_output"]["filled_rows"],
            "unmatched_rows": proof["oracle_output"]["unmatched_rows"],
            "oracle": proof["oracle_output"]["path"],
        },
        sort_keys=True,
    )


def fill_oracle_rows(
    focus: dict[str, Any],
    save_rows: list[dict[str, Any]],
) -> tuple[list[dict[str, Any]], list[dict[str, Any]], list[int]]:
    by_line: dict[int, list[dict[str, Any]]] = defaultdict(list)
    for row in save_rows:
        line_id = row.get("line_id")
        if isinstance(line_id, int):
            by_line[line_id].append(row)
    duplicate_line_ids = sorted(line_id for line_id, rows in by_line.items() if len(rows) > 1)

    filled = []
    unmatched = []
    for required in fixture_rows(focus):
        line_id = required.get("line_id")
        candidates = by_line.get(line_id, []) if isinstance(line_id, int) else []
        if len(candidates) != 1:
            reason = "missing_save_line_id" if not candidates else "duplicate_save_line_id"
            unmatched.append({**required, "observer_failure": reason})
            continue
        save = candidates[0]
        mismatches = same_frame_mismatches(required, save)
        if mismatches:
            unmatched.append(
                {
                    **required,
                    "observer_failure": "save_line_not_same_frame",
                    "same_frame_mismatches": mismatches,
                    "save_match": save_public(save),
                }
            )
            continue
        expected = save.get("location_state")
        if not isinstance(expected, int) or expected <= 0:
            unmatched.append(
                {
                    **required,
                    "observer_failure": "save_line_missing_location_state",
                    "save_match": save_public(save),
                }
            )
            continue
        filled.append(
            {
                **required,
                "expected_target_state_id": expected,
                "truth_source": "same_frame_save_production_general_lines_building_location",
                "oracle_note": (
                    "Matched save construction general_lines building by line_id, priority, "
                    "active_factories, cost_raw, and produced_raw."
                ),
                "save_match": save_public(save),
            }
        )
    return filled, unmatched, duplicate_line_ids


def same_frame_mismatches(required: dict[str, Any], save: dict[str, Any]) -> list[dict[str, Any]]:
    checks = [
        ("priority", "priority"),
        ("active_factories", "active_factories"),
        ("cost_raw", "cost_raw"),
        ("produced_raw", "produced_raw"),
    ]
    mismatches = []
    for live_key, save_key in checks:
        live_value = required.get(live_key)
        save_value = save.get(save_key)
        if live_value != save_value:
            mismatches.append(
                {
                    "field": live_key,
                    "live": live_value,
                    "save": save_value,
                }
            )
    if save.get("line_type") != 59:
        mismatches.append({"field": "line_type", "live": 59, "save": save.get("line_type")})
    return mismatches


def save_public(row: dict[str, Any]) -> dict[str, Any]:
    keys = [
        "tag",
        "line_id",
        "line_type",
        "priority",
        "active_factories",
        "cost",
        "cost_raw",
        "produced",
        "produced_raw",
        "template",
        "location_state",
        "created_date",
        "allied_build",
    ]
    return {key: row.get(key) for key in keys}


def parse_save_construction_rows(text: str) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    rows: list[dict[str, Any]] = []

    def parse_countries(tz: dc.Tokenizer) -> None:
        for tag, opener in dc.iter_block_entries(tz):
            if opener is not dc.LBRACE:
                continue
            for key, op in dc.iter_block_entries(tz):
                if op is dc.LBRACE:
                    if key == "production":
                        parse_production(tz, tag, rows)
                    else:
                        dc._skip_block_body(tz)

    meta = dc.top_level_pass(
        text,
        {"countries": parse_countries},
        scalar_keys=("date", "version", "player"),
    )
    return rows, meta


def parse_production(tz: dc.Tokenizer, tag: str, rows: list[dict[str, Any]]) -> None:
    for key, opener in dc.iter_block_entries(tz):
        if opener is dc.LBRACE:
            if key == "general_lines":
                parse_general_lines(tz, tag, rows)
            else:
                dc._skip_block_body(tz)


def parse_general_lines(tz: dc.Tokenizer, tag: str, rows: list[dict[str, Any]]) -> None:
    for key, opener in dc.iter_block_entries(tz):
        if opener is not dc.LBRACE:
            continue
        if key != "building":
            dc._skip_block_body(tz)
            continue
        row = {
            "tag": tag,
            "line_id": None,
            "line_type": None,
            "priority": None,
            "active_factories": None,
            "cost": None,
            "cost_raw": None,
            "produced": None,
            "produced_raw": None,
            "template": None,
            "location_state": None,
            "created_date": None,
            "allied_build": None,
        }
        for subkey, subop in dc.iter_block_entries(tz):
            if subop is dc.LBRACE:
                if subkey == "id":
                    row["line_id"], row["line_type"] = dc.read_id_block(tz)
                elif subkey == "building":
                    parse_inner_building(tz, row)
                else:
                    dc._skip_block_body(tz)
                continue
            value = dc.atom_value(subop)
            if value is None:
                continue
            if subkey == "active_factories":
                row["active_factories"] = dc.to_int(value)
            elif subkey == "priority":
                row["priority"] = dc.to_int(value)
            elif subkey == "cost":
                row["cost"] = dc.to_float(value)
                row["cost_raw"] = fixed_raw(value)
            elif subkey == "produced":
                row["produced"] = dc.to_float(value)
                row["produced_raw"] = fixed_raw(value)
            elif subkey == "created_date":
                row["created_date"] = value
            elif subkey == "allied_build":
                row["allied_build"] = value
        rows.append(row)


def parse_inner_building(tz: dc.Tokenizer, row: dict[str, Any]) -> None:
    for key, opener in dc.iter_block_entries(tz):
        if opener is dc.LBRACE:
            dc._skip_block_body(tz)
            continue
        value = dc.atom_value(opener)
        if value is None:
            continue
        if key == "template":
            row["template"] = value
        elif key == "location":
            row["location_state"] = dc.to_int(value)


def fixed_raw(value: str) -> int | None:
    try:
        scaled = Decimal(value) * FIXED_SCALE
    except Exception:
        return None
    return int(scaled.to_integral_value(rounding=ROUND_HALF_UP))


def wait_for_save_after(
    started: float,
    *,
    timeout_s: float,
    poll_s: float,
    stable_s: float,
) -> Path | None:
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        candidate = newest_save_after(started)
        if candidate and wait_until_stable(candidate, stable_s=stable_s, deadline=deadline):
            return candidate
        time.sleep(poll_s)
    return None


def newest_save_after(started: float) -> Path | None:
    candidates = []
    for directory in ps.candidate_save_dirs():
        if not directory.exists():
            continue
        for path in directory.iterdir():
            name = path.name.lower()
            if not (name.endswith(".hoi4") or name.endswith("_melted.txt")):
                continue
            try:
                stat = path.stat()
            except OSError:
                continue
            if stat.st_mtime >= started:
                candidates.append((stat.st_mtime, path))
    if not candidates:
        return None
    return max(candidates, key=lambda item: item[0])[1]


def wait_until_stable(path: Path, *, stable_s: float, deadline: float) -> bool:
    last = None
    stable_since = time.monotonic()
    while time.monotonic() < deadline:
        try:
            stat = path.stat()
            current = (stat.st_size, stat.st_mtime_ns)
        except OSError:
            return False
        if current != last:
            last = current
            stable_since = time.monotonic()
        if time.monotonic() - stable_since >= stable_s:
            return True
        time.sleep(0.25)
    return False


def read_json(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        json.dump(value, handle, indent=2, sort_keys=True)
        handle.write("\n")


def path_slash(path: Path) -> str:
    return str(path).replace("\\", "/")


if __name__ == "__main__":
    raise SystemExit(main())
