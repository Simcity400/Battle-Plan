"""Build lane-local naval compact records from historical raw collector frames."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Tuple

THIS_DIR = Path(__file__).resolve().parent
REPO_ROOT = THIS_DIR.parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from memory_reader.combat.naval_combat_compact_lane import (  # noqa: E402
    NavalCombatLifecycleRow,
    NavalCombatMemberRow,
    encode_records,
    gate_stream,
    schema_manifest,
)

RAW_LIFECYCLE_DOMAIN = "naval_air_combats"
RAW_MEMBER_DOMAIN = "naval_combat_members"


def convert_raw_jsonl_to_compact(
    source_path: Path,
    out_dir: Path,
    *,
    first_hour_only: bool = True,
    max_domain_rows: int = 10_000,
) -> Dict[str, object]:
    out_dir.mkdir(parents=True, exist_ok=True)
    selected_hour: Optional[int] = None
    combat_rows: List[NavalCombatLifecycleRow] = []
    member_rows: List[NavalCombatMemberRow] = []
    raw_event_type_counts: Dict[str, int] = {}
    raw_domain_counts: Dict[str, int] = {}
    raw_gap_count = 0
    scanned_lines = 0

    with source_path.open("r", encoding="utf-8") as handle:
        for line in handle:
            scanned_lines += 1
            frame = json.loads(line)
            domain = frame.get("domain")
            frame_type = frame.get("type")
            if domain not in (RAW_LIFECYCLE_DOMAIN, RAW_MEMBER_DOMAIN):
                continue
            hour = frame.get("hour_index")
            if not isinstance(hour, int):
                continue
            if selected_hour is None:
                selected_hour = hour
            if first_hour_only and selected_hour is not None and hour != selected_hour:
                if combat_rows or member_rows:
                    break
                continue
            if frame.get("confidence") == "gap":
                raw_gap_count += 1
            if frame_type == "event":
                event_type = (frame.get("payload") or {}).get("event_type")
                if isinstance(event_type, str):
                    raw_event_type_counts[event_type] = raw_event_type_counts.get(event_type, 0) + 1
                continue
            if frame_type != "domain_delta":
                continue
            raw_domain_counts[domain] = raw_domain_counts.get(domain, 0) + 1
            payload = frame.get("payload") or {}
            if payload.get("op") != "sample":
                continue
            after = payload.get("after")
            if not isinstance(after, dict):
                continue
            if domain == RAW_LIFECYCLE_DOMAIN:
                combat_rows.append(_combat_row_from_raw(after))
            else:
                member_rows.append(_member_row_from_raw(after))
            if len(combat_rows) + len(member_rows) >= max_domain_rows:
                break

    if selected_hour is None:
        report = _report(
            status="no_naval_raw_rows",
            source_path=source_path,
            out_dir=out_dir,
            selected_hour=None,
            scanned_lines=scanned_lines,
            combat_rows=[],
            member_rows=[],
            raw_domain_counts=raw_domain_counts,
            raw_event_type_counts=raw_event_type_counts,
            raw_gap_count=raw_gap_count,
            compact_path=None,
            gate=None,
        )
        _write_report(out_dir, report)
        return report

    stream = encode_records(
        sequence_start=1,
        game_hour_raw=selected_hour,
        elapsed_wall_ms=0,
        combat_rows=combat_rows,
        member_rows=member_rows,
    )
    compact_path = out_dir / "naval_combat_historical_raw_compact.bin"
    compact_path.write_bytes(stream)
    gate = gate_stream(stream, require_two_sided=True)
    report = _report(
        status="passed" if gate.ok else "failed",
        source_path=source_path,
        out_dir=out_dir,
        selected_hour=selected_hour,
        scanned_lines=scanned_lines,
        combat_rows=combat_rows,
        member_rows=member_rows,
        raw_domain_counts=raw_domain_counts,
        raw_event_type_counts=raw_event_type_counts,
        raw_gap_count=raw_gap_count,
        compact_path=compact_path,
        gate={
            "ok": gate.ok,
            "errors": gate.errors,
            "summary": gate.summary,
        },
    )
    _write_report(out_dir, report)
    return report


def _combat_row_from_raw(after: Dict[str, object]) -> NavalCombatLifecycleRow:
    return NavalCombatLifecycleRow(
        combat_id=int(after["combat_id"]),
        generation=int(after["generation"]),
        body_ptr=int(after.get("body_ptr") or after.get("body_addr") or 0),
        location=_opt_int(after.get("location")),
        attacker_tags=_opt_tags(after.get("attacker_tags")),
        defender_tags=_opt_tags(after.get("defender_tags")),
        attacker_ship_count=_opt_int(after.get("attacker_ship_count")),
        defender_ship_count=_opt_int(after.get("defender_ship_count")),
        attacker_commander=_opt_str(after.get("attacker_commander")),
        defender_commander=_opt_str(after.get("defender_commander")),
        attacker_positioning=_opt_float(after.get("attacker_positioning")),
        defender_positioning=_opt_float(after.get("defender_positioning")),
        attacker_status=str(after.get("attacker_status") or "unknown"),
        defender_status=str(after.get("defender_status") or "unknown"),
    )


def _member_row_from_raw(after: Dict[str, object]) -> NavalCombatMemberRow:
    return NavalCombatMemberRow(
        combat_id=int(after["combat_id"]),
        generation=int(after["generation"]),
        side=str(after["side"]),
        group_index=int(after["group_index"]),
        member_index=int(after["member_index"]),
        unit_type_tag=int(after["unit_type_tag"]),
        ship_instance_id=_opt_int(after.get("ship_instance_id")),
        cship_join_validated=bool(after.get("cship_join_validated"))
        or after.get("cship_status") == "resolved",
        is_convoy=bool(after.get("is_convoy")),
        owner_tag_db_id=_opt_int(after.get("owner_tag_db_id")),
        owner=_opt_str(after.get("owner")),
        name=_opt_str(after.get("name")),
        type_display=_opt_str(after.get("type_display")),
        archetype=_opt_str(after.get("archetype")),
        designed_max_strength_raw=_opt_int(after.get("designed_max_strength_raw")),
        battle_slot_index_raw=int(after.get("battle_slot_index_raw") or 0),
        state_bitmask=_opt_int(after.get("state_bitmask")),
        engagement_weight_raw=_opt_int(after.get("engagement_weight_raw")),
    )


def _report(
    *,
    status: str,
    source_path: Path,
    out_dir: Path,
    selected_hour: Optional[int],
    scanned_lines: int,
    combat_rows: List[NavalCombatLifecycleRow],
    member_rows: List[NavalCombatMemberRow],
    raw_domain_counts: Dict[str, int],
    raw_event_type_counts: Dict[str, int],
    raw_gap_count: int,
    compact_path: Optional[Path],
    gate: Optional[Dict[str, object]],
) -> Dict[str, object]:
    combat_ids = sorted({row.combat_id for row in combat_rows})
    member_sides: Dict[int, set] = {}
    for row in member_rows:
        member_sides.setdefault(row.combat_id, set()).add(row.side)
    two_sided_ids = sorted(
        combat_id for combat_id in combat_ids
        if {"attacker", "defender"}.issubset(member_sides.get(combat_id, set()))
    )
    return {
        "status": status,
        "source_path": str(source_path),
        "selected_hour": selected_hour,
        "scanned_lines": scanned_lines,
        "raw_domain_counts": raw_domain_counts,
        "raw_event_type_counts": raw_event_type_counts,
        "raw_gap_count": raw_gap_count,
        "combat_row_count": len(combat_rows),
        "member_row_count": len(member_rows),
        "combat_ids": combat_ids,
        "two_sided_combat_ids": two_sided_ids,
        "compact_path": str(compact_path) if compact_path else None,
        "gate": gate,
        "schema": schema_manifest(),
        "notes": [
            "Historical raw frames omit body addresses by policy; lifecycle body_ptr is encoded as 0 in this historical conversion.",
            "Live watcher proof keeps real body pointers from bridge.txt.",
        ],
    }


def _write_report(out_dir: Path, report: Dict[str, object]) -> None:
    (out_dir / "naval_combat_historical_raw_compact_proof.json").write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def _opt_int(value: object) -> Optional[int]:
    if value is None:
        return None
    return int(value)


def _opt_float(value: object) -> Optional[float]:
    if value is None:
        return None
    return float(value)


def _opt_str(value: object) -> Optional[str]:
    if value is None:
        return None
    return str(value)


def _opt_tags(value: object) -> Optional[Tuple[str, ...]]:
    if value is None:
        return None
    if not isinstance(value, Iterable) or isinstance(value, (str, bytes)):
        return None
    return tuple(str(part) for part in value)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("source", type=Path)
    ap.add_argument("--out-dir", type=Path, required=True)
    ap.add_argument("--all-hours", action="store_true")
    ap.add_argument("--max-domain-rows", type=int, default=10_000)
    args = ap.parse_args()

    report = convert_raw_jsonl_to_compact(
        args.source,
        args.out_dir,
        first_hour_only=not args.all_hours,
        max_domain_rows=args.max_domain_rows,
    )
    out_path = args.out_dir / "naval_combat_historical_raw_compact_proof.json"
    print(json.dumps({
        "status": report["status"],
        "out_path": str(out_path),
        "selected_hour": report["selected_hour"],
        "combat_row_count": report["combat_row_count"],
        "member_row_count": report["member_row_count"],
        "two_sided_combat_ids": report["two_sided_combat_ids"],
        "gate_ok": (report.get("gate") or {}).get("ok"),
    }, indent=2, sort_keys=True))
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
