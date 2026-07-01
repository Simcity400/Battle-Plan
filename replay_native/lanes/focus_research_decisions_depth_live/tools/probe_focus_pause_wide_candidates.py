#!/usr/bin/env python3
"""Gate wider CFocusStatus paused-state candidates outside +0x20..+0x37.

This lane-local read-only probe deliberately excludes the byte window already
rejected by ``probe_focus_pause_candidates.py``. It only rejects offsets when
the current build gives exact evidence that the byte is part of an already
validated focus-status field, pointer storage, or non-boolean storage. It does
not claim a paused boolean without a same-build paused=true oracle.
"""

from __future__ import annotations

import argparse
import json
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import probe_current_roots as roots


DEFAULT_START = 0x00
DEFAULT_END_EXCLUSIVE = 0x80
REJECTED_PRIOR_WINDOW_START = 0x20
REJECTED_PRIOR_WINDOW_END = 0x38
EXPECTED_SHA256 = roots.EXPECTED_SHA256

KNOWN_FIELDS = [
    {
        "name": "object_vtable_or_type_pointer",
        "start": 0x00,
        "size": 8,
        "kind": "module_pointer",
        "reason": "CFocusStatus object header/vtable storage, not an independent paused flag.",
    },
    {
        "name": "current_focus_ptr",
        "start": roots.FOCUS_STATUS_CURRENT,
        "size": 8,
        "kind": "row_hex_ptr",
        "row_key": "current_ptr",
        "reason": "Validated current national-focus object pointer.",
    },
    {
        "name": "continuous_focus_ptr",
        "start": roots.FOCUS_STATUS_CONTINUOUS,
        "size": 8,
        "kind": "row_hex_ptr",
        "row_key": "continuous_ptr",
        "reason": "Validated continuous-focus object pointer.",
    },
    {
        "name": "progress_raw_i32",
        "start": roots.FOCUS_STATUS_PROGRESS,
        "size": 4,
        "kind": "row_i32",
        "row_key": "progress_raw",
        "reason": "Validated focus progress i32.",
    },
    {
        "name": "completed_vector_data",
        "start": roots.FOCUS_STATUS_COMPLETED_DATA,
        "size": 8,
        "kind": "row_hex_ptr",
        "row_key": "completed_data",
        "reason": "Validated completed-focus vector data pointer.",
    },
    {
        "name": "completed_vector_cap",
        "start": roots.FOCUS_STATUS_COMPLETED_CAP,
        "size": 4,
        "kind": "row_u32",
        "row_key": "completed_cap",
        "reason": "Validated completed-focus vector capacity scalar.",
    },
    {
        "name": "completed_vector_size",
        "start": roots.FOCUS_STATUS_COMPLETED_SIZE,
        "size": 4,
        "kind": "row_u32",
        "row_key": "completed_size",
        "reason": "Validated completed-focus vector size scalar.",
    },
]


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def hex_offset(value: int) -> str:
    return f"0x{value:X}"


def parse_int(value: str) -> int:
    return int(value, 0)


def parse_hex_ptr(value: Any) -> int | None:
    if not isinstance(value, str) or not value.startswith("0x"):
        return None
    try:
        return int(value, 16)
    except ValueError:
        return None


def offset_in_prior_window(offset: int) -> bool:
    return REJECTED_PRIOR_WINDOW_START <= offset < REJECTED_PRIOR_WINDOW_END


def field_for_offset(offset: int) -> dict[str, Any] | None:
    for field in KNOWN_FIELDS:
        start = int(field["start"])
        end = start + int(field["size"])
        if start <= offset < end:
            return field
    return None


def raw_window(row: dict[str, Any]) -> bytes:
    raw_hex = (row.get("focus") or {}).get("wide_raw_hex")
    if not isinstance(raw_hex, str):
        return b""
    try:
        return bytes.fromhex(raw_hex)
    except ValueError:
        return b""


def has_focus_context(row: dict[str, Any]) -> bool:
    progress = row.get("progress_raw")
    return bool(row.get("current") or row.get("continuous") or (isinstance(progress, int) and progress > 0))


def row_field_value(row: dict[str, Any], field: dict[str, Any]) -> int | None:
    focus = row.get("focus") or {}
    kind = field.get("kind")
    if kind == "row_hex_ptr":
        return parse_hex_ptr(focus.get(str(field["row_key"])))
    if kind == "row_i32":
        value = row.get(str(field["row_key"]))
        if isinstance(value, int):
            return value & 0xFFFFFFFF
    if kind == "row_u32":
        value = focus.get(str(field["row_key"]))
        if isinstance(value, int) and 0 <= value <= 0xFFFFFFFF:
            return value
    return None


def compact_row(row: dict[str, Any], offset: int | None = None, value: int | None = None) -> dict[str, Any]:
    out = {
        "tag_db_id": row.get("tag_db_id"),
        "current": row.get("current"),
        "continuous": row.get("continuous"),
        "progress_raw": row.get("progress_raw"),
        "has_focus_context": has_focus_context(row),
        "submgr": (row.get("focus") or {}).get("submgr"),
    }
    if offset is not None:
        out["offset"] = hex_offset(offset)
    if value is not None:
        out["value"] = value
    return out


def field_validation(
    rows: list[dict[str, Any]],
    field: dict[str, Any],
    scan_start: int,
    module_base: int | None,
) -> dict[str, Any]:
    start = int(field["start"])
    size = int(field["size"])
    rel = start - scan_start
    records = 0
    expected_rows = 0
    match_rows = 0
    mismatch_examples: list[dict[str, Any]] = []
    module_pointer_like_rows = 0
    heap_pointer_like_rows = 0
    nonzero_rows = 0
    top_values: Counter[int] = Counter()

    for row in rows:
        raw = raw_window(row)
        if rel < 0 or len(raw) < rel + size:
            continue
        records += 1
        value = int.from_bytes(raw[rel:rel + size], "little", signed=False)
        top_values[value] += 1
        if value:
            nonzero_rows += 1
        if size == 8 and roots.is_heap_ptr(value):
            heap_pointer_like_rows += 1
        if size == 8 and module_base is not None and module_base <= value < module_base + 0x4000000:
            module_pointer_like_rows += 1

        expected = row_field_value(row, field)
        if expected is None:
            continue
        expected_rows += 1
        mask = (1 << (size * 8)) - 1
        if value == (expected & mask):
            match_rows += 1
        elif len(mismatch_examples) < 4:
            example = compact_row(row)
            example["observed"] = hex(value)
            example["expected"] = hex(expected & mask)
            mismatch_examples.append(example)

    if field.get("kind") == "module_pointer":
        validated = bool(records and module_pointer_like_rows == records)
        validation_reason = "all sampled rows contain a module pointer at this qword"
    else:
        validated = bool(records and expected_rows == records and match_rows == records)
        validation_reason = "all sampled rows match the root probe's validated field value"

    return {
        "name": field["name"],
        "range": f"{hex_offset(start)}..{hex_offset(start + size - 1)}",
        "size": size,
        "records": records,
        "expected_rows": expected_rows,
        "match_rows": match_rows,
        "module_pointer_like_rows": module_pointer_like_rows,
        "heap_pointer_like_rows": heap_pointer_like_rows,
        "nonzero_rows": nonzero_rows,
        "validated_for_rejection": validated,
        "validation_reason": validation_reason,
        "reason": field["reason"],
        "top_values": [
            {"value": hex(value), "records": count}
            for value, count in top_values.most_common(8)
        ],
        "mismatch_examples": mismatch_examples,
    }


def qword_structural_rejections(
    rows: list[dict[str, Any]],
    scan_start: int,
    scan_end: int,
    module_base: int | None,
) -> tuple[list[dict[str, Any]], dict[int, dict[str, Any]]]:
    structural: list[dict[str, Any]] = []
    byte_rejections: dict[int, dict[str, Any]] = {}
    for qword_start in range(scan_start, scan_end - 7, 8):
        if any(offset_in_prior_window(offset) for offset in range(qword_start, qword_start + 8)):
            continue
        rel = qword_start - scan_start
        values: Counter[int] = Counter()
        heap_pointer_like_rows = 0
        module_pointer_like_rows = 0
        examples: list[dict[str, Any]] = []
        for row in rows:
            raw = raw_window(row)
            if len(raw) < rel + 8:
                continue
            value = int.from_bytes(raw[rel:rel + 8], "little", signed=False)
            values[value] += 1
            if roots.is_heap_ptr(value):
                heap_pointer_like_rows += 1
            if module_base is not None and module_base <= value < module_base + 0x4000000:
                module_pointer_like_rows += 1
            if value and len(examples) < 4:
                example = compact_row(row, qword_start)
                example["u64_value"] = hex(value)
                example["heap_pointer_like"] = roots.is_heap_ptr(value)
                example["module_pointer_like"] = bool(
                    module_base is not None and module_base <= value < module_base + 0x4000000
                )
                examples.append(example)

        verdict = "NOT_REJECTED"
        reason = "No pointer-like structural proof for this qword."
        if values and module_pointer_like_rows == sum(values.values()):
            verdict = "REJECTED_MODULE_POINTER_STORAGE"
            reason = "Every sampled row reads as module-pointer storage at this aligned qword."
        elif values and heap_pointer_like_rows and heap_pointer_like_rows == sum(1 for value, count in values.items() for _ in range(count) if value != 0):
            verdict = "REJECTED_HEAP_POINTER_STORAGE_WHEN_NONZERO"
            reason = "Every nonzero sampled row reads as heap-pointer storage at this aligned qword."

        item = {
            "offset": hex_offset(qword_start),
            "records": sum(values.values()),
            "nonzero_rows": sum(count for value, count in values.items() if value != 0),
            "heap_pointer_like_rows": heap_pointer_like_rows,
            "module_pointer_like_rows": module_pointer_like_rows,
            "verdict": verdict,
            "reason": reason,
            "top_values": [
                {"value": hex(value), "records": count}
                for value, count in values.most_common(8)
            ],
            "examples": examples,
        }
        structural.append(item)
        if verdict.startswith("REJECTED"):
            for offset in range(qword_start, qword_start + 8):
                byte_rejections[offset] = {
                    "qword_offset": hex_offset(qword_start),
                    "qword_verdict": verdict,
                    "reason": reason,
                }
    return structural, byte_rejections


def dword_scalar_structural_rejections(
    rows: list[dict[str, Any]],
    scan_start: int,
    scan_end: int,
) -> tuple[list[dict[str, Any]], dict[int, dict[str, Any]]]:
    structural: list[dict[str, Any]] = []
    byte_rejections: dict[int, dict[str, Any]] = {}
    for dword_start in range(scan_start, scan_end - 3, 4):
        if any(offset_in_prior_window(offset) for offset in range(dword_start, dword_start + 4)):
            continue
        if field_for_offset(dword_start):
            continue
        rel = dword_start - scan_start
        values: Counter[int] = Counter()
        byte_nonzero = Counter()
        examples: list[dict[str, Any]] = []
        for row in rows:
            raw = raw_window(row)
            if len(raw) < rel + 4:
                continue
            value = int.from_bytes(raw[rel:rel + 4], "little", signed=False)
            values[value] += 1
            for byte_index, byte_value in enumerate(raw[rel:rel + 4]):
                if byte_value:
                    byte_nonzero[dword_start + byte_index] += 1
            if value not in (0, 1) and len(examples) < 4:
                example = compact_row(row, dword_start)
                example["u32_value"] = hex(value)
                example["bytes"] = raw[rel:rel + 4].hex()
                examples.append(example)

        non_bool_rows = sum(count for value, count in values.items() if value not in (0, 1))
        verdict = "NOT_REJECTED"
        reason = "No non-boolean dword scalar overlap was observed."
        if non_bool_rows:
            verdict = "REJECTED_DWORD_SCALAR_STORAGE"
            reason = (
                "This byte overlaps an aligned dword whose sampled value is a "
                "non-boolean scalar. Treating a zero high byte as an independent "
                "paused boolean would split observed multi-byte scalar storage."
            )

        item = {
            "offset": hex_offset(dword_start),
            "records": sum(values.values()),
            "nonzero_rows": sum(count for value, count in values.items() if value != 0),
            "non_bool_rows": non_bool_rows,
            "verdict": verdict,
            "reason": reason,
            "byte_nonzero_rows": {
                hex_offset(offset): count for offset, count in sorted(byte_nonzero.items())
            },
            "top_values": [
                {"value": hex(value), "records": count}
                for value, count in values.most_common(8)
            ],
            "examples": examples,
        }
        structural.append(item)
        if verdict.startswith("REJECTED"):
            for offset in range(dword_start, dword_start + 4):
                byte_rejections[offset] = {
                    "dword_offset": hex_offset(dword_start),
                    "dword_verdict": verdict,
                    "reason": reason,
                }
    return structural, byte_rejections


def analyze_rows(
    rows: list[dict[str, Any]],
    process_meta: dict[str, Any],
    build_gate: dict[str, Any],
    scan_start: int,
    scan_end: int,
) -> dict[str, Any]:
    module_base = parse_hex_ptr(process_meta.get("base")) if process_meta else None
    field_validations = [
        field_validation(rows, field, scan_start, module_base)
        for field in KNOWN_FIELDS
        if scan_start <= int(field["start"]) and int(field["start"]) + int(field["size"]) <= scan_end
    ]
    validation_by_name = {item["name"]: item for item in field_validations}
    structural_candidates, structural_byte_rejections = qword_structural_rejections(
        rows,
        scan_start,
        scan_end,
        module_base,
    )
    scalar_candidates, scalar_byte_rejections = dword_scalar_structural_rejections(
        rows,
        scan_start,
        scan_end,
    )

    rejected_offsets: list[str] = []
    known_field_rejected_offsets: list[str] = []
    direct_rejected_offsets: list[str] = []
    structural_rejected_offsets: list[str] = []
    scalar_rejected_offsets: list[str] = []
    inconclusive_offsets: list[str] = []
    skipped_prior_window_offsets: list[str] = []
    byte_candidates: list[dict[str, Any]] = []

    for offset in range(scan_start, scan_end):
        if offset_in_prior_window(offset):
            skipped_prior_window_offsets.append(hex_offset(offset))
            continue
        rel = offset - scan_start
        values: Counter[int] = Counter()
        examples: list[dict[str, Any]] = []
        no_context_examples: list[dict[str, Any]] = []
        for row in rows:
            raw = raw_window(row)
            if len(raw) <= rel:
                continue
            value = raw[rel]
            values[value] += 1
            if value != 0 and len(examples) < 5:
                examples.append(compact_row(row, offset, value))
            if value != 0 and not has_focus_context(row) and len(no_context_examples) < 5:
                no_context_examples.append(compact_row(row, offset, value))

        field = field_for_offset(offset)
        field_summary = None
        if field:
            field_summary = validation_by_name.get(str(field["name"]))

        nonzero_rows = sum(count for value, count in values.items() if value != 0)
        non_bool_values = sorted(value for value in values if value not in (0, 1))
        verdict = "INCONCLUSIVE_ALL_ZERO_NO_PAUSED_TRUE_FIXTURE"
        reason = "All sampled values are zero, so a same-build paused=true oracle is required."
        structural_overlap = structural_byte_rejections.get(offset)
        scalar_overlap = scalar_byte_rejections.get(offset)

        if field_summary and field_summary.get("validated_for_rejection"):
            verdict = "REJECTED_CANONICAL_PAUSE_BOOL_KNOWN_FIELD_OVERLAP"
            reason = field_summary["reason"]
            rejected_offsets.append(hex_offset(offset))
            known_field_rejected_offsets.append(hex_offset(offset))
        elif non_bool_values:
            verdict = "REJECTED_CANONICAL_PAUSE_BOOL_NON_BOOL_VALUES"
            reason = "A canonical paused boolean byte must be 0 or 1; this byte has other stable values."
            rejected_offsets.append(hex_offset(offset))
            direct_rejected_offsets.append(hex_offset(offset))
        elif no_context_examples:
            verdict = "REJECTED_CANONICAL_PAUSE_BOOL_TRUE_WITHOUT_FOCUS_CONTEXT"
            reason = "A paused focus flag cannot be true on a row with no current/continuous focus and zero progress."
            rejected_offsets.append(hex_offset(offset))
            direct_rejected_offsets.append(hex_offset(offset))
        elif structural_overlap:
            verdict = "REJECTED_CANONICAL_PAUSE_BOOL_POINTER_QWORD_OVERLAP"
            reason = structural_overlap["reason"]
            rejected_offsets.append(hex_offset(offset))
            structural_rejected_offsets.append(hex_offset(offset))
        elif scalar_overlap:
            verdict = "REJECTED_CANONICAL_PAUSE_BOOL_DWORD_SCALAR_OVERLAP"
            reason = scalar_overlap["reason"]
            rejected_offsets.append(hex_offset(offset))
            scalar_rejected_offsets.append(hex_offset(offset))
        elif nonzero_rows:
            verdict = "INCONCLUSIVE_BOOL_SHAPED_NO_PAUSED_TRUE_GROUND_TRUTH"
            reason = "The byte is 0/1-shaped, but no paused=true oracle proves semantics."
            inconclusive_offsets.append(hex_offset(offset))
        else:
            inconclusive_offsets.append(hex_offset(offset))

        byte_candidates.append({
            "offset": hex_offset(offset),
            "records": sum(values.values()),
            "nonzero_rows": nonzero_rows,
            "values": {str(value): count for value, count in sorted(values.items())},
            "non_bool_values": non_bool_values,
            "known_field_overlap": field_summary,
            "structural_overlap": structural_overlap,
            "scalar_overlap": scalar_overlap,
            "verdict": verdict,
            "reason": reason,
            "examples": examples,
            "no_focus_context_examples": no_context_examples,
        })

    gate_ok = bool(rows) and bool(build_gate.get("ok")) and bool(rejected_offsets)
    return {
        "schema": "focus_research_decisions_depth_live_focus_pause_wide_candidate_gate_v2",
        "lane": "focus_research_decisions_depth_live",
        "created_utc": utc_now(),
        "completion_status": "NOT_COMPLETE",
        "backlog_reduced": gate_ok,
        "process": process_meta,
        "build_gate": build_gate,
        "focus_status_rows": len(rows),
        "focus_context_rows": sum(1 for row in rows if has_focus_context(row)),
        "no_focus_context_rows": sum(1 for row in rows if not has_focus_context(row)),
        "scan_range": f"CFocusStatus+{hex_offset(scan_start)}..+{hex_offset(scan_end - 1)}",
        "excluded_prior_rejected_window": "CFocusStatus+0x20..+0x37",
        "skipped_prior_window_offsets": skipped_prior_window_offsets,
        "outside_window_rejected_byte_candidate_offsets": rejected_offsets,
        "known_field_rejected_byte_offsets": known_field_rejected_offsets,
        "direct_rejected_byte_candidate_offsets": direct_rejected_offsets,
        "structural_rejected_byte_offsets": structural_rejected_offsets,
        "scalar_rejected_byte_offsets": scalar_rejected_offsets,
        "outside_window_inconclusive_byte_candidate_offsets": inconclusive_offsets,
        "known_field_validations": field_validations,
        "structural_candidates": structural_candidates,
        "scalar_structural_candidates": scalar_candidates,
        "byte_candidates": byte_candidates,
        "remaining_blocker": {
            "field": "focus_paused_state",
            "status": "BLOCKED",
            "reason": (
                "The wider CFocusStatus scan rejected additional outside-window "
                "known-field/non-bool/pointer-storage candidates, but no paused=true "
                "oracle or alternate production root is validated."
            ),
            "next_evidence_needed": (
                "same-build paused=true fixture through UI/save/dynamic oracle, "
                "or a root outside CFocusStatus proven by static function/argument evidence"
            ),
        },
        "gate": {
            "ok": gate_ok,
            "errors": [] if gate_ok else ["no focus rows, build mismatch, or no outside-window rejections"],
        },
    }


def live_rows(scan_start: int, scan_end: int) -> tuple[list[dict[str, Any]], dict[str, Any], dict[str, Any]]:
    proc = roots.winmem.open_process("hoi4.exe", "hoi4.exe")
    if proc is None:
        return [], {"error": "hoi4.exe_not_running"}, {
            "ok": False,
            "expected_sha256": EXPECTED_SHA256,
            "actual_sha256": None,
        }

    with proc:
        fp = proc.fingerprint()
        module_base = proc.module_base
        game_state = roots.safe(lambda: proc.u64(module_base + roots.C_GAME_STATE_GLOBAL))
        country_database = roots.safe(lambda: proc.u64(module_base + roots.COUNTRY_DATABASE_GLOBAL))
        _tag_table, tag_by_id, tag_meta = roots.country_tag_table(proc, country_database)
        countries, table_meta = roots.country_rows(
            proc,
            module_base,
            game_state or 0,
            tag_by_id,
            tag_meta.get("row_count"),
        )
        rows: list[dict[str, Any]] = []
        read_len = scan_end - scan_start
        for country in countries:
            focus = roots.focus_status_probe(proc, country["object_ptr"])
            if focus is None:
                continue
            submgr = parse_hex_ptr(focus.get("submgr"))
            if submgr is None:
                continue
            raw = roots.safe(lambda addr=submgr: proc.read(addr + scan_start, read_len), b"")
            focus["wide_raw_hex"] = raw.hex() if isinstance(raw, bytes) else ""
            focus["wide_raw_range"] = f"{hex_offset(scan_start)}..{hex_offset(scan_end - 1)}"
            rows.append({
                "index": country["index"],
                "tag_db_id": country["tag_db_id"],
                "object_ptr": roots.hex0(country["object_ptr"]),
                "current": focus["current"],
                "continuous": focus["continuous"],
                "progress_raw": focus["progress_raw"],
                "focus": focus,
            })

        process = {
            "pid": fp.get("pid"),
            "module": fp.get("module"),
            "base": roots.hex0(module_base),
            "size": fp.get("size"),
            "path": fp.get("path"),
            "country_rows": len(countries),
            "valid_country_table": table_meta.get("valid_table"),
        }
        build_gate = {
            "expected_sha256": EXPECTED_SHA256,
            "actual_sha256": fp.get("sha256"),
            "ok": (fp.get("sha256") or "").lower() == EXPECTED_SHA256,
        }
        return rows, process, build_gate


def write_markdown(path: Path, gate: dict[str, Any]) -> None:
    lines = [
        "# Focus Pause Wide Candidate Gate",
        "",
        f"Created UTC: `{gate['created_utc']}`",
        f"Completion status: `{gate['completion_status']}`",
        f"Backlog reduced: `{str(gate['backlog_reduced']).lower()}`",
        "",
        "## Summary",
        "",
        f"- Scan range: `{gate['scan_range']}`.",
        f"- Excluded prior window: `{gate['excluded_prior_rejected_window']}`.",
        f"- Focus status rows: `{gate['focus_status_rows']}`.",
        f"- Rows with focus context: `{gate['focus_context_rows']}`.",
        f"- Rows without focus context: `{gate['no_focus_context_rows']}`.",
        f"- Outside-window rejected offsets: `{', '.join(gate['outside_window_rejected_byte_candidate_offsets'])}`.",
        f"- Known-field overlap rejections: `{', '.join(gate['known_field_rejected_byte_offsets'])}`.",
        f"- Direct byte-shape rejections: `{', '.join(gate['direct_rejected_byte_candidate_offsets'])}`.",
        f"- Structural qword rejections: `{', '.join(gate['structural_rejected_byte_offsets'])}`.",
        f"- Structural dword-scalar rejections: `{', '.join(gate['scalar_rejected_byte_offsets'])}`.",
        f"- Outside-window inconclusive offsets: `{', '.join(gate['outside_window_inconclusive_byte_candidate_offsets'])}`.",
        "",
        "## Known Field Proof",
        "",
    ]
    for item in gate["known_field_validations"]:
        lines.append(
            f"- `{item['name']}` `{item['range']}`: "
            f"validated `{str(item['validated_for_rejection']).lower()}`, "
            f"matches `{item['match_rows']}/{item['records']}`, "
            f"module-pointer rows `{item['module_pointer_like_rows']}`."
        )
    lines.extend(["", "## Remaining Blocker", ""])
    lines.append(
        "- Focus paused state remains blocked. This gate rejects additional "
        "outside-window candidates, but promotion still requires a same-build "
        "paused=true oracle or another root with exact function/argument proof."
    )
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def run() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--start-offset", type=parse_int, default=DEFAULT_START)
    parser.add_argument("--end-offset", type=parse_int, default=DEFAULT_END_EXCLUSIVE)
    args = parser.parse_args()

    if args.start_offset < 0 or args.end_offset <= args.start_offset:
        raise SystemExit("--end-offset must be greater than --start-offset")
    if args.end_offset - args.start_offset > 0x400:
        raise SystemExit("refusing to scan more than 0x400 bytes in this bounded gate")

    rows, process, build_gate = live_rows(args.start_offset, args.end_offset)
    gate = analyze_rows(rows, process, build_gate, args.start_offset, args.end_offset)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "focus_pause_wide_candidate_gate.json").write_text(
        json.dumps(gate, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    write_markdown(args.out_dir / "FOCUS_PAUSE_WIDE_CANDIDATE_GATE.md", gate)
    print(json.dumps({
        "ok": gate["gate"]["ok"],
        "backlog_reduced": gate["backlog_reduced"],
        "focus_status_rows": gate["focus_status_rows"],
        "scan_range": gate["scan_range"],
        "excluded_prior_rejected_window": gate["excluded_prior_rejected_window"],
        "outside_window_rejected_byte_candidate_offsets": gate["outside_window_rejected_byte_candidate_offsets"],
        "known_field_rejected_byte_offsets": gate["known_field_rejected_byte_offsets"],
        "direct_rejected_byte_candidate_offsets": gate["direct_rejected_byte_candidate_offsets"],
        "structural_rejected_byte_offsets": gate["structural_rejected_byte_offsets"],
        "scalar_rejected_byte_offsets": gate["scalar_rejected_byte_offsets"],
        "outside_window_inconclusive_byte_candidate_offsets": gate["outside_window_inconclusive_byte_candidate_offsets"],
        "out_dir": str(args.out_dir),
    }, indent=2))
    return 0 if gate["gate"]["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(run())
