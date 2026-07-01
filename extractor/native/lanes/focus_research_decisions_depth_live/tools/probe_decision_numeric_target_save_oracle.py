#!/usr/bin/env python3
"""Validate targeted-timed numeric decision targets against a save oracle.

This is lane-local proof code.  It compares save-side targeted decision
``target=<number>`` rows with live ``active_targeted_timed_decision`` record
integer candidates.  It does not edit shared collector or recorder entrypoints.
"""

from __future__ import annotations

import argparse
import json
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import probe_decision_target_save_oracle as country_oracle


EXPECTED_SHA256 = country_oracle.EXPECTED_SHA256
LIVE_LIST = country_oracle.LIVE_LIST
SAVE_TARGET_LISTS = country_oracle.SAVE_TARGET_LISTS
TARGETED_TIMED_VTABLE_RVA = country_oracle.TARGETED_TIMED_VTABLE_RVA


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def int_offset_key(offset: str) -> int:
    return int(offset, 16)


def candidate_values(record: dict[str, Any]) -> dict[str, int]:
    values: dict[str, int] = {}
    for candidate in record.get("small_i32_candidates_0x20_0x7c") or []:
        offset = candidate.get("offset")
        value = candidate.get("value")
        if isinstance(offset, str) and isinstance(value, int):
            values[offset] = value
    return values


def live_targeted_timed_rows(samples: list[dict[str, Any]]) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for row in samples:
        if row.get("list_name") != LIVE_LIST:
            continue
        owner_id = row.get("tag_db_id")
        for record in row.get("records") or []:
            decision = record.get("decision")
            if not isinstance(decision, str) or not decision:
                continue
            out.append(
                {
                    "decision": decision,
                    "owner_tag_db_id": owner_id if isinstance(owner_id, int) else None,
                    "record_ptr": record.get("record_ptr"),
                    "record_vtable_rva": record.get("record_vtable_rva"),
                    "small_i32": candidate_values(record),
                }
            )
    return out


def save_numeric_rows(save_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for row in save_rows:
        if row.get("target_kind") != "state_or_numeric":
            continue
        if row.get("save_list_name") not in SAVE_TARGET_LISTS:
            continue
        target = row.get("target")
        try:
            target_id = int(target)
        except (TypeError, ValueError):
            continue
        copy = dict(row)
        copy["target_id"] = target_id
        out.append(copy)
    return out


def counter_rows(rows: list[dict[str, Any]], key_fields: tuple[str, ...]) -> Counter[tuple[Any, ...]]:
    counter: Counter[tuple[Any, ...]] = Counter()
    for row in rows:
        counter[tuple(row.get(field) for field in key_fields)] += 1
    return counter


def live_counter_for_offset(
    live_rows: list[dict[str, Any]],
    offset: str,
) -> tuple[Counter[tuple[str, int]], int]:
    counter: Counter[tuple[str, int]] = Counter()
    positive_rows = 0
    for row in live_rows:
        values = row.get("small_i32") or {}
        value = values.get(offset)
        if not isinstance(value, int) or value <= 0:
            continue
        positive_rows += 1
        counter[(row["decision"], value)] += 1
    return counter, positive_rows


def save_counter_by_decision(
    save_rows: list[dict[str, Any]],
) -> dict[str, Counter[int]]:
    out: dict[str, Counter[int]] = defaultdict(Counter)
    for row in save_rows:
        decision = row.get("decision")
        target_id = row.get("target_id")
        if isinstance(decision, str) and isinstance(target_id, int):
            out[decision][target_id] += 1
    return out


def live_counter_by_decision(
    live_rows: list[dict[str, Any]],
    offset: str,
) -> dict[str, Counter[int]]:
    out: dict[str, Counter[int]] = defaultdict(Counter)
    for row in live_rows:
        decision = row.get("decision")
        value = (row.get("small_i32") or {}).get(offset)
        if isinstance(decision, str) and isinstance(value, int) and value > 0:
            out[decision][value] += 1
    return out


def offset_summary(
    live_rows: list[dict[str, Any]],
    save_rows: list[dict[str, Any]],
    offset: str,
) -> dict[str, Any]:
    live_counter, live_positive_rows = live_counter_for_offset(live_rows, offset)
    save_counter = counter_rows(save_rows, ("decision", "target_id"))
    exact_multiset_match = live_counter == save_counter
    save_by_decision = save_counter_by_decision(save_rows)
    live_by_decision = live_counter_by_decision(live_rows, offset)
    decisions = sorted(set(save_by_decision) | set(live_by_decision))

    breakdown = []
    matched_decisions = []
    unmatched_decisions = []
    matched_intersection_rows = 0
    for decision in decisions:
        live_decision_counter = live_by_decision.get(decision, Counter())
        save_decision_counter = save_by_decision.get(decision, Counter())
        matched = live_decision_counter == save_decision_counter
        live_rows_for_decision = sum(live_decision_counter.values())
        save_rows_for_decision = sum(save_decision_counter.values())
        intersection = live_decision_counter & save_decision_counter
        matched_intersection_rows += sum(intersection.values())
        item = {
            "decision": decision,
            "matched": matched,
            "live_rows": live_rows_for_decision,
            "save_rows": save_rows_for_decision,
            "intersection_rows": sum(intersection.values()),
            "target_count": len(live_decision_counter),
            "sample_live_targets": [
                {"target_id": target_id, "records": count}
                for target_id, count in live_decision_counter.most_common(8)
            ],
            "live_minus_save": [
                {"target_id": target_id, "records": count}
                for (target_id,), count in (
                    Counter({(k,): v for k, v in live_decision_counter.items()})
                    - Counter({(k,): v for k, v in save_decision_counter.items()})
                ).items()
            ][:8],
            "save_minus_live": [
                {"target_id": target_id, "records": count}
                for (target_id,), count in (
                    Counter({(k,): v for k, v in save_decision_counter.items()})
                    - Counter({(k,): v for k, v in live_decision_counter.items()})
                ).items()
            ][:8],
        }
        breakdown.append(item)
        if matched:
            matched_decisions.append(item)
        else:
            unmatched_decisions.append(item)

    validated_live_rows = sum(int(row["live_rows"]) for row in matched_decisions)
    validated_save_rows = sum(int(row["save_rows"]) for row in matched_decisions)
    unmatched_live_rows = sum(int(row["live_rows"]) for row in unmatched_decisions)
    unmatched_save_rows = sum(int(row["save_rows"]) for row in unmatched_decisions)
    return {
        "offset": offset,
        "live_positive_rows": live_positive_rows,
        "save_numeric_rows": len(save_rows),
        "exact_row_count_match": live_positive_rows == len(save_rows),
        "exact_multiset_match_by_decision_target": exact_multiset_match,
        "validated_decision_count": len(matched_decisions),
        "unmatched_decision_count": len(unmatched_decisions),
        "validated_live_numeric_target_rows": validated_live_rows,
        "validated_save_numeric_target_rows": validated_save_rows,
        "unmatched_live_numeric_target_rows": unmatched_live_rows,
        "unmatched_save_numeric_target_rows": unmatched_save_rows,
        "matched_intersection_rows": matched_intersection_rows,
        "matched_intersection_ratio": round(matched_intersection_rows / len(save_rows), 6)
        if save_rows
        else 0,
        "live_extra_rows": sum((live_counter - save_counter).values()),
        "save_missing_rows": sum((save_counter - live_counter).values()),
        "decision_breakdown": breakdown,
        "sample_matched_decisions": matched_decisions[:20],
        "sample_unmatched_decisions": unmatched_decisions[:20],
    }


def discovered_offsets(live_rows: list[dict[str, Any]]) -> list[str]:
    offsets = set()
    for row in live_rows:
        offsets.update((row.get("small_i32") or {}).keys())
    return sorted(offsets, key=int_offset_key)


def choose_best_candidate(summaries: list[dict[str, Any]]) -> dict[str, Any] | None:
    if not summaries:
        return None
    return max(
        summaries,
        key=lambda item: (
            int(item["matched_intersection_rows"]),
            int(item["validated_live_numeric_target_rows"]),
            -int(item["live_extra_rows"]),
            -int(item["save_missing_rows"]),
        ),
    )


def build_gate(args: argparse.Namespace) -> dict[str, Any]:
    probe_dir = args.probe_dir
    root_manifest = country_oracle.load_json(probe_dir / "root_probe_manifest.json")
    samples = country_oracle.load_jsonl(probe_dir / "decision_body_samples.jsonl")
    live_rows = live_targeted_timed_rows(samples)
    live_vtable_rows = [
        row for row in live_rows if row.get("record_vtable_rva") == TARGETED_TIMED_VTABLE_RVA
    ]

    save_path = args.save or country_oracle.default_autosave_path()
    rakaly = country_oracle.ps.find_rakaly(args.rakaly)
    text, was_melted = country_oracle.ps.melt_if_needed(save_path, rakaly, args.out_dir)
    save_rows, save_meta = country_oracle.parse_save_decisions(text)
    numeric_rows = save_numeric_rows(save_rows)

    offsets = args.offset or discovered_offsets(live_rows)
    offset_summaries = [offset_summary(live_rows, numeric_rows, offset) for offset in offsets]
    best = choose_best_candidate(offset_summaries)
    ranked_summaries = sorted(
        offset_summaries,
        key=lambda item: (
            -int(item["matched_intersection_rows"]),
            -int(item["validated_live_numeric_target_rows"]),
            int(item["live_extra_rows"]),
            int(item["save_missing_rows"]),
            int_offset_key(item["offset"]),
        ),
    )
    runner_up = ranked_summaries[1] if len(ranked_summaries) > 1 else None
    build_ok = (
        bool(root_manifest.get("ok"))
        and (root_manifest.get("build_gate") or {}).get("actual_sha256", "").lower()
        == EXPECTED_SHA256
    )
    all_live_rows_targeted_timed_vtable = len(live_vtable_rows) == len(live_rows)
    best_intersection_rows = int((best or {}).get("matched_intersection_rows") or 0)
    best_save_rows = int((best or {}).get("save_numeric_rows") or 0)
    best_live_extra_rows = int((best or {}).get("live_extra_rows") or 0)
    best_save_missing_rows = int((best or {}).get("save_missing_rows") or 0)
    runner_up_intersection_rows = int((runner_up or {}).get("matched_intersection_rows") or 0)
    intersection_ratio = best_intersection_rows / best_save_rows if best_save_rows else 0.0
    residual_mismatch_rows = best_live_extra_rows + best_save_missing_rows
    residual_mismatch_limit = max(8, int(best_save_rows * 0.01)) if best_save_rows else 0
    matched_subset_evidence_ok = bool(
        best
        and best_intersection_rows >= 100
        and intersection_ratio >= 0.98
        and residual_mismatch_rows <= residual_mismatch_limit
        and best_intersection_rows >= max(10, runner_up_intersection_rows * 10)
    )
    exact_all_numeric_rows_ok = bool(
        build_ok
        and best
        and bool(live_rows)
        and bool(numeric_rows)
        and all_live_rows_targeted_timed_vtable
        and best.get("exact_multiset_match_by_decision_target")
        and best.get("exact_row_count_match")
    )
    partial_numeric_target_ok = bool(
        build_ok
        and best
        and bool(live_rows)
        and bool(numeric_rows)
        and all_live_rows_targeted_timed_vtable
        and matched_subset_evidence_ok
    )
    status = "NOT_VALIDATED"
    if exact_all_numeric_rows_ok:
        status = "VALIDATED_FOR_ALL_NUMERIC_TARGET_ROWS"
    elif partial_numeric_target_ok:
        status = "VALIDATED_FOR_EXACT_SAVE_LIVE_MATCHED_NUMERIC_TARGET_ROWS"

    validated_field = {
        "list": LIVE_LIST,
        "record_vtable_rva": TARGETED_TIMED_VTABLE_RVA,
        "offset": best.get("offset") if best else None,
        "field": "numeric_target_id",
        "target_encoding": "live positive i32 that matches save target=<number>",
        "status": status,
    }
    return {
        "schema": "focus_research_decisions_depth_live_decision_numeric_target_save_oracle_gate_v1",
        "lane": "focus_research_decisions_depth_live",
        "created_utc": utc_now(),
        "completion_status": "NOT COMPLETE",
        "probe_dir": str(probe_dir),
        "save_path": str(save_path),
        "save_was_melted": was_melted,
        "rakaly": str(rakaly) if rakaly else None,
        "build_gate": root_manifest.get("build_gate"),
        "save_meta": save_meta,
        "live_list": LIVE_LIST,
        "save_target_lists": sorted(SAVE_TARGET_LISTS),
        "validated_field": validated_field,
        "live_rows": len(live_rows),
        "live_vtable_rows": len(live_vtable_rows),
        "all_live_rows_have_targeted_timed_vtable": all_live_rows_targeted_timed_vtable,
        "save_target_rows": len(save_rows),
        "save_numeric_target_rows": len(numeric_rows),
        "candidate_offset_count": len(offset_summaries),
        "candidate_offsets": offsets,
        "best_offset_summary": best,
        "candidate_summaries": ranked_summaries,
        "runner_up_offset_summary": runner_up,
        "validation_basis": {
            "validated_rows": best_intersection_rows,
            "matched_intersection_ratio": round(intersection_ratio, 6),
            "live_extra_rows": best_live_extra_rows,
            "save_missing_rows": best_save_missing_rows,
            "residual_mismatch_rows": residual_mismatch_rows,
            "residual_mismatch_limit": residual_mismatch_limit,
            "runner_up_intersection_rows": runner_up_intersection_rows,
            "matched_subset_evidence_ok": matched_subset_evidence_ok,
        },
        "validated_live_numeric_target_rows": best_intersection_rows,
        "validated_save_numeric_target_rows": best_intersection_rows,
        "unmatched_live_numeric_target_rows": best_live_extra_rows,
        "unmatched_save_numeric_target_rows": best_save_missing_rows,
        "strict_all_numeric_rows_ok": exact_all_numeric_rows_ok,
        "backlog_reduced": partial_numeric_target_ok,
        "remaining_blocker": {
            "field": "decision_target_cooldown_body_fields",
            "status": "PARTIALLY_BLOCKED",
            "reason": (
                "A save/live oracle validates record+0x24 numeric target ids for "
                "the exact matched active_targeted_timed_decision subset, but "
                "cooldown/day/state fields, unmatched transition rows, and exact "
                "numeric target semantics still need same-frame or static contract "
                "evidence."
            ),
            "next_evidence_needed": (
                "same-frame save/live oracle for cooldown/day/state encodings and "
                "definition-level semantics for whether numeric target ids are state "
                "ids, province ids, or another decision-specific numeric body"
            ),
        },
        "gate": {
            "ok": partial_numeric_target_ok,
            "errors": []
            if partial_numeric_target_ok
            else [
                "build mismatch, missing live/save numeric rows, vtable mismatch, or no strong exact save/live numeric target subset"
            ],
        },
    }


def write_markdown(path: Path, gate: dict[str, Any]) -> None:
    field = gate["validated_field"]
    best = gate.get("best_offset_summary") or {}
    basis = gate.get("validation_basis") or {}
    lines = [
        "# Decision Numeric Target Save Oracle Gate",
        "",
        f"Created UTC: `{gate['created_utc']}`",
        f"Completion status: `{gate['completion_status']}`",
        f"Backlog reduced: `{str(gate['backlog_reduced']).lower()}`",
        "",
        "## Summary",
        "",
        f"- Live list: `{gate['live_list']}`.",
        f"- Save lists compared: `{', '.join(gate['save_target_lists'])}`.",
        f"- Validated field: `{field['list']} {field['record_vtable_rva']} {field['offset']} -> {field['field']}`.",
        f"- Field status: `{field['status']}`.",
        f"- Live rows: `{gate['live_rows']}`.",
        f"- Save numeric target rows: `{gate['save_numeric_target_rows']}`.",
        f"- Best offset: `{best.get('offset')}`.",
        f"- Best offset live positive rows: `{best.get('live_positive_rows')}`.",
        f"- Best offset exact row-count match: `{str(best.get('exact_row_count_match')).lower()}`.",
        f"- Best offset exact multiset match by `(decision,target_id)`: `{str(best.get('exact_multiset_match_by_decision_target')).lower()}`.",
        f"- Validated exact matched numeric target rows: `{basis.get('validated_rows')}`.",
        f"- Unmatched live numeric target rows: `{basis.get('live_extra_rows')}`.",
        f"- Unmatched save numeric target rows: `{basis.get('save_missing_rows')}`.",
        f"- Matched intersection rows: `{best.get('matched_intersection_rows')}`.",
        f"- Matched intersection ratio: `{best.get('matched_intersection_ratio')}`.",
        f"- Runner-up intersection rows: `{basis.get('runner_up_intersection_rows')}`.",
        f"- Matched subset evidence ok: `{str(basis.get('matched_subset_evidence_ok')).lower()}`.",
        f"- Strict all-numeric-row match: `{str(gate['strict_all_numeric_rows_ok']).lower()}`.",
        "",
        "## Candidate Ranking",
        "",
    ]
    for summary in gate["candidate_summaries"][:12]:
        lines.append(
            f"- `{summary['offset']}`: intersection `{summary['matched_intersection_rows']}`, "
            f"validated `{summary['validated_live_numeric_target_rows']}`, "
            f"live extra `{summary['live_extra_rows']}`, save missing `{summary['save_missing_rows']}`."
        )
    lines.extend(["", "## Matched Examples", ""])
    for row in best.get("sample_matched_decisions") or []:
        sample = ", ".join(
            f"{item['target_id']}:{item['records']}"
            for item in row.get("sample_live_targets", [])[:5]
        )
        lines.append(
            f"- `{row['decision']}`: `{row['live_rows']}` rows, "
            f"`{row['target_count']}` targets; sample `{sample}`."
        )
    if not best.get("sample_matched_decisions"):
        lines.append("- None.")
    if best.get("sample_unmatched_decisions"):
        lines.extend(["", "## Unmatched Decisions", ""])
        for row in best["sample_unmatched_decisions"][:20]:
            lines.append(
                f"- `{row['decision']}`: live `{row['live_rows']}`, save `{row['save_rows']}`, "
                f"intersection `{row['intersection_rows']}`."
            )
    lines.extend(
        [
            "",
            "## Remaining Blocker",
            "",
            f"- `{gate['remaining_blocker']['field']}`: {gate['remaining_blocker']['reason']}",
            f"  Next evidence: {gate['remaining_blocker']['next_evidence_needed']}.",
        ]
    )
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def run() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--probe-dir", type=Path, required=True)
    parser.add_argument("--save", type=Path, default=None)
    parser.add_argument("--rakaly", default=None)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument(
        "--offset",
        action="append",
        help="Specific live record offset to test, e.g. 0x24. Defaults to all discovered i32 offsets.",
    )
    args = parser.parse_args()
    args.out_dir.mkdir(parents=True, exist_ok=True)

    gate = build_gate(args)
    (args.out_dir / "decision_numeric_target_save_oracle_gate.json").write_text(
        json.dumps(gate, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    write_markdown(args.out_dir / "DECISION_NUMERIC_TARGET_SAVE_ORACLE_GATE.md", gate)
    best = gate.get("best_offset_summary") or {}
    print(
        json.dumps(
            {
                "ok": gate["gate"]["ok"],
                "backlog_reduced": gate["backlog_reduced"],
                "validated_field": gate["validated_field"],
                "live_rows": gate["live_rows"],
                "save_numeric_target_rows": gate["save_numeric_target_rows"],
                "best_offset": best.get("offset"),
                "validated_live_numeric_target_rows": best.get(
                    "matched_intersection_rows"
                ),
                "unmatched_live_numeric_target_rows": best.get("live_extra_rows"),
                "unmatched_save_numeric_target_rows": best.get("save_missing_rows"),
                "matched_intersection_rows": best.get("matched_intersection_rows"),
                "out_dir": str(args.out_dir),
            },
            indent=2,
        )
    )
    return 0 if gate["gate"]["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(run())
