#!/usr/bin/env python3
"""Summarize residual blockers for focus/research/decision depth.

This consumes lane-local proof artifacts only.  It does not read live memory and
does not edit shared collector or recorder entrypoints.
"""

from __future__ import annotations

import argparse
import json
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


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


def summarize_targeted_timed(samples: list[dict[str, Any]]) -> dict[str, Any]:
    records: list[dict[str, Any]] = []
    owners = Counter()
    for row in samples:
        if row.get("list_name") != "active_targeted_timed_decision":
            continue
        owner = row.get("tag_db_id")
        if isinstance(owner, int):
            owners[owner] += 1
        records.extend(row.get("records") or [])

    by_decision: dict[str, dict[str, Any]] = {}
    offset_presence: dict[str, int] = Counter()
    offset_values: dict[str, Counter] = defaultdict(Counter)
    pointer_presence: dict[str, int] = Counter()
    pointer_values: dict[str, Counter] = defaultdict(Counter)
    for record in records:
        decision = record.get("decision") or "<missing>"
        item = by_decision.setdefault(
            decision,
            {
                "records": 0,
                "tag_offsets": defaultdict(Counter),
                "pointer_offsets": defaultdict(Counter),
            },
        )
        item["records"] += 1
        for candidate in record.get("tag_id_field_candidates") or []:
            offset = candidate.get("offset")
            tag_db_id = candidate.get("tag_db_id")
            tag = candidate.get("tag")
            if not isinstance(offset, str) or not isinstance(tag_db_id, int):
                continue
            value = f"{tag_db_id}:{tag or '?'}"
            item["tag_offsets"][offset][value] += 1
            offset_presence[offset] += 1
            offset_values[offset][value] += 1
        for candidate in record.get("target_country_pointer_candidates") or []:
            offset = candidate.get("offset")
            tag_db_id = candidate.get("tag_db_id")
            if not isinstance(offset, str) or not isinstance(tag_db_id, int):
                continue
            value = str(tag_db_id)
            item["pointer_offsets"][offset][value] += 1
            pointer_presence[offset] += 1
            pointer_values[offset][value] += 1

    decision_examples = []
    for decision, item in sorted(
        by_decision.items(), key=lambda kv: (-kv[1]["records"], kv[0])
    )[:12]:
        tag_offsets = {}
        for offset, values in item["tag_offsets"].items():
            tag_offsets[offset] = {
                "records": sum(values.values()),
                "unique_values": len(values),
                "top_values": [
                    {"value": value, "records": count}
                    for value, count in values.most_common(5)
                ],
            }
        decision_examples.append(
            {
                "decision": decision,
                "records": item["records"],
                "tag_offsets": dict(sorted(tag_offsets.items())),
            }
        )

    return {
        "records_sampled": len(records),
        "owner_country_count": len(owners),
        "decision_token_count": len(by_decision),
        "definition_pointer_valid_records": sum(1 for record in records if record.get("decision")),
        "tag_offset_presence": {
            offset: {
                "records": count,
                "coverage_ratio": round(count / len(records), 6) if records else 0,
                "unique_values": len(offset_values[offset]),
                "top_values": [
                    {"value": value, "records": records}
                    for value, records in offset_values[offset].most_common(6)
                ],
            }
            for offset, count in sorted(offset_presence.items(), key=lambda item: int(item[0], 16))
        },
        "pointer_offset_presence": {
            offset: {
                "records": count,
                "coverage_ratio": round(count / len(records), 6) if records else 0,
                "unique_values": len(pointer_values[offset]),
                "top_values": [
                    {"value": value, "records": records}
                    for value, records in pointer_values[offset].most_common(6)
                ],
            }
            for offset, count in sorted(pointer_presence.items(), key=lambda item: int(item[0], 16))
        },
        "decision_examples": decision_examples,
        "variant_layout_verdict": (
            "blocked_definition_aware_oracle_required"
            if records
            else "blocked_no_targeted_timed_records"
        ),
    }


def build_report(
    probe_dir: Path,
    depth_gate_path: Path | None,
    integrated_manifest_path: Path | None,
) -> dict[str, Any]:
    root_manifest = load_json(probe_dir / "root_probe_manifest.json")
    samples = load_jsonl(probe_dir / "decision_body_samples.jsonl")
    depth_gate = load_json(depth_gate_path) if depth_gate_path else None
    integrated_manifest = load_json(integrated_manifest_path) if integrated_manifest_path else None
    targeted_timed = summarize_targeted_timed(samples)
    focus_summary = root_manifest.get("focus", {})

    exact_rows = int((depth_gate or {}).get("focus_completed_exact_rows") or 0)
    inferred_rows = int((depth_gate or {}).get("focus_completed_inferred_rows") or 0)
    unresolved_rows = int((depth_gate or {}).get("focus_completed_unresolved_rows") or 0)
    compact_country_frame = bool((integrated_manifest or {}).get("compact_country_frame"))

    closures = []
    blockers = []

    if exact_rows:
        closures.append(
            {
                "field": "direct_completed_focus_token_closure",
                "status": "validated_slice",
                "evidence": {
                    "focus_completed_exact_rows": exact_rows,
                    "focus_completed_inferred_rows": inferred_rows,
                    "focus_completed_unresolved_rows": unresolved_rows,
                    "depth_gate_manifest": str(depth_gate_path),
                },
            }
        )
    else:
        blockers.append(
            {
                "field": "direct_completed_focus_token_closure",
                "status": "BLOCKED",
                "reason": (
                    "The integrated compact proof did not emit exact completed-token rows. "
                    "In the current shared collector, compact focus reads omit the completed "
                    "focus vector, so completion rows can only be inferred from current-focus "
                    "deltas unless the shared compact focus reader includes bounded completed "
                    "tokens."
                ),
                "evidence": {
                    "compact_country_frame": compact_country_frame,
                    "focus_completed_exact_rows": exact_rows,
                    "focus_completed_inferred_rows": inferred_rows,
                    "focus_completed_unresolved_rows": unresolved_rows,
                    "depth_gate_manifest": str(depth_gate_path) if depth_gate_path else None,
                    "shared_patch_needed": "read completed focus vector in compact focus rows or add a depth-lane collector hook that emits exact completed-token deltas",
                },
            }
        )

    blockers.append(
        {
            "field": "focus_paused_state",
            "status": "BLOCKED",
            "reason": "No paused=true ground truth was observed; sampled candidate bytes cannot be converted into a production boolean.",
            "evidence": {
                "focus_status_valid_rows": focus_summary.get("focus_status_valid_rows"),
                "active_or_progress_rows": focus_summary.get("active_or_progress_rows"),
                "paused_flag_verdict": focus_summary.get("paused_flag_verdict"),
            },
            "next_evidence_needed": "same-build paused=true focus fixture or UI/save-aligned paused focus row",
        }
    )

    blockers.append(
        {
            "field": "decision_target_cooldown_body_fields",
            "status": "BLOCKED",
            "reason": (
                "Non-timed targeted/cooldown vtables reject the timed body layout. "
                "Targeted-timed records validate record+0x10 -> CDecisionDef, but "
                "target-looking tag/pointer offsets are sparse or constant, which "
                "rejects definition-agnostic universal target extraction."
            ),
            "evidence": targeted_timed,
            "next_evidence_needed": "same-frame oracle keyed by owner tag, decision token, and known target country/state/province, or static decision-definition target-type metadata for this build/mod set",
        }
    )

    blockers.append(
        {
            "field": "direct_focus_research_decision_event_markers",
            "status": "BLOCKED",
            "reason": "No engine callback/transition function root was validated in this lane; production markers remain deterministic compact-frame deltas.",
            "evidence": {
                "direct_event_badge_rows": (depth_gate or {}).get("direct_event_badge_rows"),
                "root_probe_verdict": root_manifest.get("verdicts", {}).get("direct_transition_event_markers"),
            },
            "next_evidence_needed": "validated hook/callback root with function address, arguments, and live proof; otherwise keep badge kind 3",
        }
    )

    return {
        "schema": "focus_research_decisions_depth_live_residual_blocker_gate_v1",
        "lane": "focus_research_decisions_depth_live",
        "created_utc": utc_now(),
        "probe_dir": str(probe_dir),
        "depth_gate_manifest": str(depth_gate_path) if depth_gate_path else None,
        "integrated_manifest": str(integrated_manifest_path) if integrated_manifest_path else None,
        "build_gate": root_manifest.get("build_gate"),
        "completion_status": "NOT COMPLETE",
        "backlog_reduced": bool(closures or blockers),
        "validated_slices": closures,
        "remaining_blockers": blockers,
        "gate": {
            "ok": bool(root_manifest.get("ok")) and (depth_gate is None or bool(depth_gate.get("ok"))),
            "errors": [] if root_manifest.get("ok") else ["root probe manifest not ok"],
        },
    }


def write_markdown(path: Path, report: dict[str, Any]) -> None:
    lines = [
        "# Residual Blocker Gate",
        "",
        f"Created UTC: `{report['created_utc']}`",
        f"Completion status: `{report['completion_status']}`",
        f"Backlog reduced: `{str(report['backlog_reduced']).lower()}`",
        "",
        "## Validated Slices",
        "",
    ]
    for item in report["validated_slices"]:
        lines.append(f"- `{item['field']}`: {item['status']}.")
    if not report["validated_slices"]:
        lines.append("- None.")
    lines.extend(["", "## Remaining Blockers", ""])
    for item in report["remaining_blockers"]:
        lines.append(f"- `{item['field']}`: {item['reason']}")
        if item.get("next_evidence_needed"):
            lines.append(f"  Next evidence: {item['next_evidence_needed']}.")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def run() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--probe-dir", type=Path, required=True)
    parser.add_argument("--depth-gate", type=Path, default=None)
    parser.add_argument("--integrated-manifest", type=Path, default=None)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()

    report = build_report(args.probe_dir, args.depth_gate, args.integrated_manifest)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "residual_blocker_gate.json").write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    write_markdown(args.out_dir / "RESIDUAL_BLOCKER_GATE.md", report)
    print(
        json.dumps(
            {
                "ok": report["gate"]["ok"],
                "completion_status": report["completion_status"],
                "validated_slices": len(report["validated_slices"]),
                "remaining_blockers": len(report["remaining_blockers"]),
                "out_dir": str(args.out_dir),
            },
            indent=2,
        )
    )
    return 0 if report["gate"]["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(run())
