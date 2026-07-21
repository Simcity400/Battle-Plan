#!/usr/bin/env python3
"""Independent offline grader for repeated A27 whole-sweep timelines."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

SCHEMA = "extractor.a27.live_heap_catalog_timeline.a729.v1"
PROGRAM_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
PROGRAM = "hoi4.exe 1.19.2.0.a729"
DATE_EPOCH_RAW = 43_799_999


def canonical_bytes(value: object) -> bytes:
    return json.dumps(
        value, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")


def stable_rows(snapshot: dict) -> dict[str, dict]:
    rows = {}
    for key, runtime_type in (("division_templates", 52), ("variants", 70)):
        source = snapshot.get(key)
        if not isinstance(source, list):
            raise ValueError(f"{key} is not an array")
        for ordinal, row in enumerate(source):
            if row.get("type") != runtime_type or row.get("catalog_scan_ordinal") != ordinal:
                raise ValueError(f"{key}/{ordinal} type or catalog ordinal is invalid")
            stable_id = f"{row.get('id')}:{runtime_type}"
            if stable_id in rows:
                raise ValueError(f"duplicate stable design identity {stable_id}")
            rows[stable_id] = row
    return rows


def pointer_free(value):
    if isinstance(value, list):
        return [pointer_free(item) for item in value]
    if isinstance(value, dict):
        return {
            key: pointer_free(item)
            for key, item in value.items()
            if key != "pointer_identity"
            and not key.endswith("_pointer")
            and key not in {"pid", "module_base", "game_state"}
        }
    return value


def semantic_row(row: dict) -> dict:
    """Native row identity without relocation or scan-order bookkeeping."""
    return {
        key: value
        for key, value in pointer_free(row).items()
        if key != "catalog_scan_ordinal"
    }


def apply_delta(previous: dict, delta: dict) -> dict:
    rows = {
        f"{row['id']}:{row['type']}": row
        for row in previous["division_templates"] + previous["variants"]
    }
    tombstones = delta.get("variant_tombstones", []) + delta.get(
        "template_tombstones", []
    )
    if len(tombstones) != len(set(tombstones)):
        raise ValueError("delta contains duplicate tombstones")
    for stable_id in tombstones:
        if rows.pop(stable_id, None) is None:
            raise ValueError(f"delta tombstones absent design {stable_id}")
    upserts = delta.get("variant_upserts", []) + delta.get("template_upserts", [])
    upsert_ids = [f"{row['id']}:{row['type']}" for row in upserts]
    if len(upsert_ids) != len(set(upsert_ids)) or set(upsert_ids) & set(tombstones):
        raise ValueError("delta contains duplicate/conflicting upserts")
    rows.update(zip(upsert_ids, upserts))
    frame = dict(delta["frame_metadata"])
    templates = []
    variants = []
    for stable_id in delta["ordered_design_ids"]:
        row = dict(rows.pop(stable_id))
        if row["type"] == 52:
            row["catalog_scan_ordinal"] = len(templates)
            templates.append(row)
        elif row["type"] == 70:
            row["catalog_scan_ordinal"] = len(variants)
            variants.append(row)
        else:
            raise ValueError(f"delta order has wrong-typed design {stable_id}")
    if rows:
        raise ValueError("delta order omits reconstructed designs")
    frame["division_templates"] = templates
    frame["variants"] = variants
    return frame


def grade(
    timeline: dict,
    first_hour: int | None,
    last_hour: int | None,
    require_mutation: bool = False,
    minimum_captures: int = 1,
) -> dict:
    errors = []
    transitions = []
    if timeline.get("schema") != SCHEMA:
        errors.append("timeline schema mismatch")
    authority = timeline.get("authority", {})
    if authority.get("program") != PROGRAM or authority.get("program_sha256") != PROGRAM_SHA256:
        errors.append("program identity mismatch")
    if timeline.get("complete_sequence") is not True:
        errors.append("timeline was not sealed complete")
    if timeline.get("transport") != "first_full_then_native_row_deltas":
        errors.append("timeline transport mismatch")
    if timeline.get("fatal") is not None:
        errors.append(f"timeline producer reported fatal state: {timeline.get('fatal')}")
    captures = timeline.get("captures")
    if not isinstance(captures, list) or not captures:
        errors.append("timeline has no captures")
        captures = []
    if len(captures) < minimum_captures:
        errors.append(
            f"capture count {len(captures)} is below required {minimum_captures}"
        )
    previous_hour = None
    previous_rows = None
    previous_snapshot = None
    for sequence, capture in enumerate(captures):
        if capture.get("sequence") != sequence:
            errors.append(f"capture {sequence}: sequence invalid")
            continue
        if not isinstance(capture.get("same_hour_rechecks"), int) or capture[
            "same_hour_rechecks"
        ] < 1:
            errors.append(f"capture {sequence}: same-hour mutation recheck missing")
        recheck_hashes = capture.get("same_hour_recheck_sha256")
        if (
            not isinstance(recheck_hashes, list)
            or len(recheck_hashes) != capture.get("same_hour_rechecks")
            or any(value != capture.get("snapshot_sha256") for value in recheck_hashes)
        ):
            errors.append(f"capture {sequence}: same-hour recheck hashes are invalid")
        try:
            if sequence == 0:
                snapshot = capture.get("snapshot")
                if not isinstance(snapshot, dict) or capture.get("delta") is not None:
                    raise ValueError("baseline is not one full snapshot")
            else:
                if capture.get("snapshot") is not None or not isinstance(
                    capture.get("delta"), dict
                ):
                    raise ValueError("non-baseline capture is not one delta")
                snapshot = apply_delta(previous_snapshot, capture["delta"])
        except (KeyError, TypeError, ValueError) as error:
            errors.append(f"capture {sequence}: {error}")
            continue
        digest = hashlib.sha256(canonical_bytes(snapshot)).hexdigest()
        if capture.get("snapshot_sha256") != digest:
            errors.append(f"capture {sequence}: snapshot hash mismatch")
        hour = capture.get("capture_hour_index")
        snapshot_authority = snapshot.get("authority", {})
        if (
            not isinstance(hour, int)
            or snapshot_authority.get("program") != PROGRAM
            or snapshot_authority.get("program_sha256") != PROGRAM_SHA256
            or snapshot_authority.get("capture_hour_index") != hour
            or snapshot_authority.get("current_date_raw") != DATE_EPOCH_RAW + hour
            or snapshot_authority.get("coherence")
            != "double_read_all_roots_fields_and_membership"
            or snapshot.get("full_replacement") is not True
            or snapshot.get("completeness")
            != {
                "definitions": True,
                "lifecycle": True,
                "membership": True,
                "owner": True,
                "ui_occurrences": True,
            }
            or not isinstance(snapshot.get("countries"), list)
            or snapshot.get("country_count") != len(snapshot["countries"])
        ):
            errors.append(f"capture {sequence}: authority/hour/completeness identity invalid")
            continue
        if previous_hour is not None and hour != previous_hour + 1:
            errors.append(f"capture {sequence}: replay-hour gap {previous_hour}->{hour}")
        try:
            rows = stable_rows(snapshot)
        except ValueError as error:
            errors.append(f"capture {sequence}: {error}")
            continue
        if previous_rows is not None:
            created = sorted(set(rows) - set(previous_rows))
            tombstones = sorted(set(previous_rows) - set(rows))
            mutated = sorted(
                stable_id
                for stable_id in set(rows) & set(previous_rows)
                if semantic_row(rows[stable_id])
                != semantic_row(previous_rows[stable_id])
            )
            transitions.append(
                {
                    "sequence": sequence,
                    "from_hour": previous_hour,
                    "to_hour": hour,
                    "created": created,
                    "tombstones": tombstones,
                    "mutated": mutated,
                    "ordered_design_ids": list(rows),
                }
            )
        previous_hour = hour
        previous_rows = rows
        previous_snapshot = snapshot
    if captures:
        observed_first = captures[0].get("capture_hour_index")
        observed_last = captures[-1].get("capture_hour_index")
        if (
            timeline.get("capture_count") != len(captures)
            or timeline.get("first_capture_hour_index") != observed_first
            or timeline.get("last_capture_hour_index") != observed_last
        ):
            errors.append("timeline summary disagrees with capture sequence")
        if timeline.get("same_hour_recheck_count") != sum(
            capture.get("same_hour_rechecks", 0)
            for capture in captures
            if isinstance(capture.get("same_hour_rechecks"), int)
        ):
            errors.append("timeline recheck summary disagrees with capture sequence")
        if first_hour is not None and observed_first != first_hour:
            errors.append(f"first capture hour {observed_first} != required {first_hour}")
        if last_hour is not None and observed_last != last_hour:
            errors.append(f"last capture hour {observed_last} != required {last_hour}")
    if require_mutation and not any(
        transition["created"]
        or transition["tombstones"]
        or transition["mutated"]
        for transition in transitions
    ):
        errors.append("required design mutation was not observed")
    return {
        "schema": "extractor.a27.live_heap_catalog_timeline_regrade.v1",
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "capture_count": len(captures),
        "rejected_observations": timeline.get("rejected_observations", []),
        "transitions": transitions,
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--timeline", required=True, type=Path)
    parser.add_argument("--report-out", required=True, type=Path)
    parser.add_argument("--first-hour", type=int)
    parser.add_argument("--last-hour", type=int)
    parser.add_argument("--require-mutation", action="store_true")
    parser.add_argument("--minimum-captures", type=int, default=1)
    args = parser.parse_args(argv)
    if args.minimum_captures < 1:
        parser.error("--minimum-captures must be positive")
    timeline = json.loads(args.timeline.read_text(encoding="utf-8"))
    report = grade(
        timeline,
        args.first_hour,
        args.last_hour,
        require_mutation=args.require_mutation,
        minimum_captures=args.minimum_captures,
    )
    args.report_out.parent.mkdir(parents=True, exist_ok=True)
    args.report_out.write_text(
        json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2),
        encoding="utf-8",
    )
    print(
        f"{report['status']}: {report['capture_count']} authenticated hourly A27 censuses"
    )
    return 0 if report["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
