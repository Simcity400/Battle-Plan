#!/usr/bin/env python3
"""Gate current-build focus paused-state candidates.

This is a lane-local read-only probe. It does not edit shared collector or
recorder entrypoints and does not claim a paused boolean without a paused=true
fixture. Its job is to reject byte candidates in the already-scanned
``CFocusStatus+0x20..+0x37`` window that cannot be a canonical paused flag.
"""

from __future__ import annotations

import argparse
import json
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import probe_current_roots as roots


WINDOW_BASE = 0x20
WINDOW_END_EXCLUSIVE = 0x38
STRUCTURAL_QWORD_OFFSETS = (0x20, 0x28, 0x30)
EXPECTED_SHA256 = roots.EXPECTED_SHA256


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def load_jsonl(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    with path.open("r", encoding="utf-8") as handle:
        for line_no, line in enumerate(handle, start=1):
            line = line.strip()
            if not line:
                continue
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError as exc:
                raise ValueError(f"{path}:{line_no}: invalid json: {exc}") from exc
    return rows


def has_focus_context(row: dict[str, Any]) -> bool:
    progress = row.get("progress_raw")
    return bool(row.get("current") or row.get("continuous") or (isinstance(progress, int) and progress > 0))


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
        out["offset"] = f"0x{offset:X}"
    if value is not None:
        out["value"] = value
    return out


def read_u64_le(raw: bytes, rel: int) -> int:
    return int.from_bytes(raw[rel:rel + 8], "little", signed=False)


def row_window(row: dict[str, Any]) -> bytes:
    raw_hex = (row.get("focus") or {}).get("pause_candidate_window_0x20_0x3f_hex")
    if not isinstance(raw_hex, str):
        return b""
    try:
        return bytes.fromhex(raw_hex)
    except ValueError:
        return b""


def analyze_rows(
    rows: list[dict[str, Any]],
    process_meta: dict[str, Any] | None,
    build_gate: dict[str, Any] | None,
) -> dict[str, Any]:
    structural_results: list[dict[str, Any]] = []
    structural_byte_rejections: dict[int, dict[str, Any]] = {}

    for offset in STRUCTURAL_QWORD_OFFSETS:
        rel = offset - WINDOW_BASE
        values: Counter[int] = Counter()
        pointer_like = 0
        module_like = 0
        examples: list[dict[str, Any]] = []
        module_base = None
        if process_meta and isinstance(process_meta.get("base"), str):
            try:
                module_base = int(process_meta["base"], 16)
            except ValueError:
                module_base = None
        for row in rows:
            raw = row_window(row)
            if len(raw) < rel + 8:
                continue
            value = read_u64_le(raw, rel)
            values[value] += 1
            if roots.is_heap_ptr(value):
                pointer_like += 1
            if module_base is not None and module_base <= value < module_base + 0x4000000:
                module_like += 1
            if value and len(examples) < 6:
                item = compact_row(row)
                item["offset"] = f"0x{offset:X}"
                item["u64_value"] = f"0x{value:X}"
                item["heap_pointer_like"] = roots.is_heap_ptr(value)
                item["module_pointer_like"] = bool(module_base is not None and module_base <= value < module_base + 0x4000000)
                examples.append(item)

        if pointer_like or module_like:
            verdict = "REJECTED_CANONICAL_PAUSE_BOOL_POINTER_LIKE_STORAGE"
            reason = "The qword candidate resolves as pointer-like storage, not a canonical paused boolean."
        elif any(value not in (0, 1) for value in values):
            verdict = "REJECTED_CANONICAL_PAUSE_BOOL_NON_BOOL_QWORD_VALUES"
            reason = "The qword candidate has stable non-boolean values."
        else:
            verdict = "INCONCLUSIVE_QWORD_BOOL_SHAPED_NO_PAUSED_TRUE_GROUND_TRUTH"
            reason = "No paused=true fixture proves this qword's semantics."
        structural_results.append({
            "offset": f"0x{offset:X}",
            "records": sum(values.values()),
            "nonzero_rows": sum(count for value, count in values.items() if value != 0),
            "unique_values": len(values),
            "pointer_like_rows": pointer_like,
            "module_pointer_like_rows": module_like,
            "top_values": [
                {"value": f"0x{value:X}", "records": count}
                for value, count in values.most_common(8)
            ],
            "verdict": verdict,
            "reason": reason,
            "examples": examples,
        })
        if verdict.startswith("REJECTED"):
            for byte_offset in range(offset, min(offset + 8, WINDOW_END_EXCLUSIVE)):
                structural_byte_rejections.setdefault(byte_offset, {
                    "qword_offset": f"0x{offset:X}",
                    "qword_verdict": verdict,
                    "reason": (
                        "This byte overlaps rejected qword-sized storage. Treating it "
                        "as an independent paused boolean would overlap the observed "
                        "pointer/scalar field at the same address range."
                    ),
                })

    byte_results: list[dict[str, Any]] = []
    rejected_offsets: list[str] = []
    inconclusive_offsets: list[str] = []
    direct_rejected_offsets: list[str] = []
    structural_rejected_offsets: list[str] = []

    for offset in range(WINDOW_BASE, WINDOW_END_EXCLUSIVE):
        rel = offset - WINDOW_BASE
        values: Counter[int] = Counter()
        examples: list[dict[str, Any]] = []
        no_context_examples: list[dict[str, Any]] = []
        for row in rows:
            raw = row_window(row)
            if len(raw) <= rel:
                continue
            value = raw[rel]
            values[value] += 1
            if value != 0 and len(examples) < 6:
                examples.append(compact_row(row, offset, value))
            if value != 0 and not has_focus_context(row) and len(no_context_examples) < 6:
                no_context_examples.append(compact_row(row, offset, value))

        nonzero_rows = sum(count for value, count in values.items() if value != 0)
        non_bool_values = sorted(value for value in values if value not in (0, 1))
        if non_bool_values:
            verdict = "REJECTED_CANONICAL_PAUSE_BOOL_NON_BOOL_VALUES"
            reason = "A canonical paused boolean byte must be 0 or 1; this byte has other stable values."
            rejected_offsets.append(f"0x{offset:X}")
            direct_rejected_offsets.append(f"0x{offset:X}")
        elif no_context_examples:
            verdict = "REJECTED_CANONICAL_PAUSE_BOOL_TRUE_WITHOUT_FOCUS_CONTEXT"
            reason = "A paused focus flag cannot be true on a row with no current or continuous focus and zero progress."
            rejected_offsets.append(f"0x{offset:X}")
            direct_rejected_offsets.append(f"0x{offset:X}")
        elif offset in structural_byte_rejections:
            verdict = "REJECTED_CANONICAL_PAUSE_BOOL_OVERLAPS_QWORD_STORAGE"
            reason = structural_byte_rejections[offset]["reason"]
            rejected_offsets.append(f"0x{offset:X}")
            structural_rejected_offsets.append(f"0x{offset:X}")
        elif nonzero_rows == 0:
            verdict = "INCONCLUSIVE_ALL_ZERO_NO_PAUSED_TRUE_FIXTURE"
            reason = "All sampled values are zero, so this cannot be validated or rejected without a paused=true fixture."
            inconclusive_offsets.append(f"0x{offset:X}")
        else:
            verdict = "INCONCLUSIVE_BOOL_SHAPED_NO_PAUSED_TRUE_GROUND_TRUTH"
            reason = "The byte is 0/1-shaped, but no same-build paused=true ground truth proves semantics."
            inconclusive_offsets.append(f"0x{offset:X}")

        byte_results.append({
            "offset": f"0x{offset:X}",
            "records": sum(values.values()),
            "nonzero_rows": nonzero_rows,
            "values": {str(value): count for value, count in sorted(values.items())},
            "non_bool_values": non_bool_values,
            "verdict": verdict,
            "reason": reason,
            "structural_overlap": structural_byte_rejections.get(offset),
            "examples": examples,
            "no_focus_context_examples": no_context_examples,
        })

    gate_ok = bool(rows) and bool(rejected_offsets)
    return {
        "schema": "focus_research_decisions_depth_live_focus_pause_candidate_gate_v2",
        "lane": "focus_research_decisions_depth_live",
        "created_utc": utc_now(),
        "completion_status": "NOT_COMPLETE",
        "backlog_reduced": gate_ok,
        "process": process_meta,
        "build_gate": build_gate,
        "focus_status_rows": len(rows),
        "focus_context_rows": sum(1 for row in rows if has_focus_context(row)),
        "no_focus_context_rows": sum(1 for row in rows if not has_focus_context(row)),
        "window": "CFocusStatus+0x20..+0x37",
        "rejected_byte_candidate_offsets": rejected_offsets,
        "direct_rejected_byte_candidate_offsets": direct_rejected_offsets,
        "structural_overlap_rejected_byte_offsets": structural_rejected_offsets,
        "inconclusive_byte_candidate_offsets": inconclusive_offsets,
        "byte_candidates": byte_results,
        "structural_candidates": structural_results,
        "remaining_blocker": {
            "field": "focus_paused_state",
            "status": "BLOCKED",
            "reason": "The scanned CFocusStatus+0x20..+0x37 window is rejected as the paused-boolean home; no alternate production root has been validated.",
            "next_evidence_needed": "paused=true fixture with current or continuous focus and aligned UI/save proof, plus a wider root search outside this rejected window",
        },
        "gate": {
            "ok": gate_ok,
            "errors": [] if gate_ok else ["no focus rows or no rejected candidates"],
        },
    }


def live_rows() -> tuple[list[dict[str, Any]], dict[str, Any], dict[str, Any]]:
    proc = roots.winmem.open_process("hoi4.exe", "hoi4.exe")
    if proc is None:
        return [], {"error": "hoi4.exe_not_running"}, {"ok": False, "expected_sha256": EXPECTED_SHA256, "actual_sha256": None}
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
        for country in countries:
            focus = roots.focus_status_probe(proc, country["object_ptr"])
            if focus is None:
                continue
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
        "# Focus Pause Candidate Gate",
        "",
        f"Created UTC: `{gate['created_utc']}`",
        f"Completion status: `{gate['completion_status']}`",
        f"Backlog reduced: `{str(gate['backlog_reduced']).lower()}`",
        "",
        "## Summary",
        "",
        f"- Focus status rows: `{gate['focus_status_rows']}`.",
        f"- Rows with focus context: `{gate['focus_context_rows']}`.",
        f"- Rows without focus context: `{gate['no_focus_context_rows']}`.",
        f"- Rejected byte offsets: `{', '.join(gate['rejected_byte_candidate_offsets'])}`.",
        f"- Direct byte-shape rejections: `{', '.join(gate['direct_rejected_byte_candidate_offsets'])}`.",
        f"- Structural overlap rejections: `{', '.join(gate['structural_overlap_rejected_byte_offsets'])}`.",
        f"- Inconclusive byte offsets: `{', '.join(gate['inconclusive_byte_candidate_offsets'])}`.",
        "",
        "## Rejections",
        "",
    ]
    for item in gate["byte_candidates"]:
        if not item["verdict"].startswith("REJECTED"):
            continue
        values = ", ".join(f"{value}:{count}" for value, count in item["values"].items())
        lines.append(f"- `{item['offset']}`: {item['reason']} Values `{values}`.")
    if not any(item["verdict"].startswith("REJECTED") for item in gate["byte_candidates"]):
        lines.append("- None.")
    lines.extend(["", "## Remaining Blocker", ""])
    lines.append(
        "- Focus paused state remains blocked, but this scanned candidate window "
        "is rejected as the paused-boolean home. The next useful evidence is a "
        "same-build paused=true fixture or a wider root search outside this "
        "window."
    )
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def run() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()

    rows, process, build_gate = live_rows()
    gate = analyze_rows(rows, process, build_gate)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "focus_pause_candidate_gate.json").write_text(
        json.dumps(gate, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    write_markdown(args.out_dir / "FOCUS_PAUSE_CANDIDATE_GATE.md", gate)
    print(json.dumps({
        "ok": gate["gate"]["ok"],
        "backlog_reduced": gate["backlog_reduced"],
        "focus_status_rows": gate["focus_status_rows"],
        "rejected_byte_candidate_offsets": gate["rejected_byte_candidate_offsets"],
        "direct_rejected_byte_candidate_offsets": gate["direct_rejected_byte_candidate_offsets"],
        "structural_overlap_rejected_byte_offsets": gate["structural_overlap_rejected_byte_offsets"],
        "inconclusive_byte_candidate_offsets": gate["inconclusive_byte_candidate_offsets"],
        "out_dir": str(args.out_dir),
    }, indent=2))
    return 0 if gate["gate"]["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(run())
