#!/usr/bin/env python3
"""Gate decision body target/cooldown candidates from a root probe.

The input is produced by ``probe_current_roots.py``.  This analyzer does not
read live memory; it reduces the recorded sample into exact candidate
rejections so the lane does not keep retrying offsets that already failed the
current-build evidence.
"""

from __future__ import annotations

import argparse
import json
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


TARGET_LISTS = {
    "decisions_taken_bucket",
    "active_targeted_decision",
    "active_targeted_timed_decision",
    "decision_to_re_enable",
}


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def offset_key(offset: str) -> int:
    try:
        return int(offset, 16)
    except ValueError:
        return 0


def load_jsonl(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    with path.open("r", encoding="utf-8") as fh:
        for line_no, line in enumerate(fh, start=1):
            line = line.strip()
            if not line:
                continue
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError as exc:
                raise ValueError(f"{path}:{line_no}: invalid json: {exc}") from exc
    return rows


def summarize_records(rows: list[dict[str, Any]]) -> dict[str, Any]:
    lists: dict[str, dict[str, Any]] = {}
    for row in rows:
        list_name = row.get("list_name")
        if not isinstance(list_name, str):
            continue
        summary = lists.setdefault(
            list_name,
            {
                "records_sampled": 0,
                "owner_tags": Counter(),
                "decision_tokens": Counter(),
                "vtable_rvas": Counter(),
                "def_name_valid_records": 0,
                "timed_body_contract_records": 0,
                "tag_offsets": defaultdict(
                    lambda: {
                        "records": 0,
                        "owner_equal_records": 0,
                        "values": Counter(),
                    }
                ),
                "country_pointer_offsets": defaultdict(
                    lambda: {
                        "records": 0,
                        "owner_equal_records": 0,
                        "values": Counter(),
                    }
                ),
            },
        )
        owner = row.get("tag_db_id")
        if isinstance(owner, int):
            summary["owner_tags"][owner] += 1
        for record in row.get("records") or []:
            summary["records_sampled"] += 1
            rva = record.get("record_vtable_rva") or record.get("record_vtable")
            if rva:
                summary["vtable_rvas"][str(rva)] += 1
            decision = record.get("decision")
            if isinstance(decision, str) and decision:
                summary["decision_tokens"][decision] += 1
                summary["def_name_valid_records"] += 1
            if record.get("timed_body_contract"):
                summary["timed_body_contract_records"] += 1
            for candidate in record.get("tag_id_field_candidates") or []:
                offset = candidate.get("offset")
                tag_db_id = candidate.get("tag_db_id")
                tag = candidate.get("tag")
                if not isinstance(offset, str) or not isinstance(tag_db_id, int):
                    continue
                offset_summary = summary["tag_offsets"][offset]
                offset_summary["records"] += 1
                if isinstance(owner, int) and tag_db_id == owner:
                    offset_summary["owner_equal_records"] += 1
                offset_summary["values"][f"{tag_db_id}:{tag or '?'}"] += 1
            for candidate in record.get("target_country_pointer_candidates") or []:
                offset = candidate.get("offset")
                tag_db_id = candidate.get("tag_db_id")
                if not isinstance(offset, str) or not isinstance(tag_db_id, int):
                    continue
                offset_summary = summary["country_pointer_offsets"][offset]
                offset_summary["records"] += 1
                if isinstance(owner, int) and tag_db_id == owner:
                    offset_summary["owner_equal_records"] += 1
                offset_summary["values"][str(tag_db_id)] += 1
    return lists


def compact_offset_stats(raw: dict[str, Any]) -> dict[str, Any]:
    values = raw["values"]
    return {
        "records": raw["records"],
        "owner_equal_records": raw["owner_equal_records"],
        "unique_value_count": len(values),
        "top_values": [
            {"value": value, "records": count}
            for value, count in values.most_common(8)
        ],
    }


def build_rejections(
    manifest: dict[str, Any], summaries: dict[str, dict[str, Any]]
) -> tuple[list[dict[str, Any]], list[dict[str, Any]], dict[str, Any]]:
    rejections: list[dict[str, Any]] = []
    blockers: list[dict[str, Any]] = []
    details: dict[str, Any] = {}
    manifest_lists = manifest.get("decisions", {}).get("lists", {})

    for list_name, summary in sorted(summaries.items()):
        records = int(summary["records_sampled"])
        owner_count = len(summary["owner_tags"])
        decision_token_count = len(summary["decision_tokens"])
        tag_offsets = {
            offset: compact_offset_stats(raw)
            for offset, raw in sorted(
                summary["tag_offsets"].items(), key=lambda item: offset_key(item[0])
            )
        }
        pointer_offsets = {
            offset: compact_offset_stats(raw)
            for offset, raw in sorted(
                summary["country_pointer_offsets"].items(),
                key=lambda item: offset_key(item[0]),
            )
        }
        details[list_name] = {
            "records_sampled": records,
            "owner_country_count": owner_count,
            "decision_token_count": decision_token_count,
            "vtable_rvas": dict(summary["vtable_rvas"]),
            "def_name_valid_records": summary["def_name_valid_records"],
            "timed_body_contract_records": summary["timed_body_contract_records"],
            "tag_offsets": tag_offsets,
            "country_pointer_offsets": pointer_offsets,
            "manifest_body_verdict": manifest_lists.get(list_name, {}).get("body_verdict"),
        }

        if list_name not in TARGET_LISTS or records == 0:
            continue

        if summary["def_name_valid_records"] == 0:
            rejections.append(
                {
                    "candidate": f"{list_name}: record+0x10 -> CDecisionDef timed-body layout",
                    "verdict": "REJECTED",
                    "reason": (
                        "No sampled record resolved a decision token through the timed "
                        "decision body contract; record+0x18/+0x1C days/state also do "
                        "not satisfy the timed body gate for this vtable."
                    ),
                    "records_sampled": records,
                    "vtable_rvas": dict(summary["vtable_rvas"]),
                    "def_name_valid_records": summary["def_name_valid_records"],
                    "timed_body_contract_records": summary["timed_body_contract_records"],
                }
            )

        for offset, offset_summary in tag_offsets.items():
            count = int(offset_summary["records"])
            unique_count = int(offset_summary["unique_value_count"])
            top_values = offset_summary["top_values"]
            if count < records:
                rejections.append(
                    {
                        "candidate": f"{list_name}: record+{offset} as universal country-target tag id",
                        "verdict": "REJECTED",
                        "reason": (
                            "The offset is absent as a valid tag id on at least one "
                            "sampled record, so it cannot be the required exact "
                            "country-target field for the whole list."
                        ),
                        "records_with_candidate": count,
                        "records_sampled": records,
                        "owner_country_count": owner_count,
                        "top_values": top_values,
                    }
                )
            elif unique_count == 1 and owner_count > 1:
                rejections.append(
                    {
                        "candidate": f"{list_name}: record+{offset} as exact target tag id",
                        "verdict": "REJECTED",
                        "reason": (
                            "The offset resolves to one constant tag across records "
                            "from multiple owners, so it is not a record-specific "
                            "target field."
                        ),
                        "records_with_candidate": count,
                        "records_sampled": records,
                        "owner_country_count": owner_count,
                        "top_values": top_values,
                    }
                )

        for offset, offset_summary in pointer_offsets.items():
            count = int(offset_summary["records"])
            if count < records:
                rejections.append(
                    {
                        "candidate": f"{list_name}: record+{offset} as universal country-target pointer",
                        "verdict": "REJECTED",
                        "reason": (
                            "The offset resolves to a country pointer only sparsely, "
                            "so it cannot be the required exact country-target pointer "
                            "for the whole list."
                        ),
                        "records_with_candidate": count,
                        "records_sampled": records,
                        "owner_country_count": owner_count,
                        "top_values": offset_summary["top_values"],
                    }
                )

    targeted_timed = details.get("active_targeted_timed_decision")
    if targeted_timed and targeted_timed["def_name_valid_records"]:
        blockers.append(
            {
                "candidate": "active_targeted_timed_decision target body",
                "status": "BLOCKED",
                "reason": (
                    "record+0x10 -> CDecisionDef is validated for sampled rows, "
                    "but candidate target tag/pointer offsets are either sparse or "
                    "constant and no stronger same-frame save/UI oracle is attached."
                ),
                "records_sampled": targeted_timed["records_sampled"],
                "def_name_valid_records": targeted_timed["def_name_valid_records"],
                "exact_next_evidence_needed": (
                    "same-frame oracle rows keyed by owner tag, decision token, and "
                    "known target country/state/province for records with vtable 0x27D5038"
                ),
            }
        )
    return rejections, blockers, details


def write_markdown(path: Path, gate: dict[str, Any]) -> None:
    lines = [
        "# Decision Body Candidate Gate",
        "",
        f"Created UTC: `{gate['created_utc']}`",
        f"Probe: `{gate['probe_dir']}`",
        "",
        f"Backlog reduced: `{str(gate['backlog_reduced']).lower()}`",
        f"Rejected candidate count: `{gate['rejected_candidate_count']}`",
        "",
        "## Rejected Candidates",
        "",
    ]
    for rejection in gate["rejected_candidates"]:
        lines.extend(
            [
                f"- `{rejection['candidate']}`: {rejection['reason']}",
                f"  Evidence: `{rejection.get('records_with_candidate', rejection.get('def_name_valid_records'))}` / `{rejection['records_sampled']}` sampled records; vtables `{rejection.get('vtable_rvas', 'see json')}`.",
            ]
        )
    if not gate["rejected_candidates"]:
        lines.append("- None.")
    lines.extend(["", "## Remaining Blockers", ""])
    for blocker in gate["remaining_blockers"]:
        lines.extend(
            [
                f"- `{blocker['candidate']}`: {blocker['reason']}",
                f"  Next evidence: {blocker['exact_next_evidence_needed']}.",
            ]
        )
    if not gate["remaining_blockers"]:
        lines.append("- None.")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def run(probe_dir: Path, out_dir: Path) -> int:
    manifest_path = probe_dir / "root_probe_manifest.json"
    samples_path = probe_dir / "decision_body_samples.jsonl"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    rows = load_jsonl(samples_path)
    summaries = summarize_records(rows)
    rejections, blockers, details = build_rejections(manifest, summaries)
    gate = {
        "schema": "focus_research_decisions_depth_live_decision_body_candidate_gate_v1",
        "lane": "focus_research_decisions_depth_live",
        "created_utc": utc_now(),
        "probe_dir": str(probe_dir),
        "build_gate": manifest.get("build_gate", {}),
        "roots": {
            "decision_status_vtable": manifest.get("roots", {}).get("decision_status_vtable"),
            "timed_decision_vtable": manifest.get("roots", {}).get("timed_decision_vtable"),
        },
        "backlog_reduced": bool(rejections),
        "rejected_candidate_count": len(rejections),
        "rejected_candidates": rejections,
        "remaining_blockers": blockers,
        "per_list_details": details,
        "gate": {
            "ok": bool(manifest.get("ok")) and bool(rejections),
            "errors": [] if manifest.get("ok") else ["root probe manifest not ok"],
        },
    }
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "decision_body_candidate_gate.json").write_text(
        json.dumps(gate, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    write_markdown(out_dir / "DECISION_BODY_CANDIDATE_GATE.md", gate)
    print(
        json.dumps(
            {
                "ok": gate["gate"]["ok"],
                "backlog_reduced": gate["backlog_reduced"],
                "rejected_candidate_count": gate["rejected_candidate_count"],
                "remaining_blocker_count": len(gate["remaining_blockers"]),
                "out_dir": str(out_dir),
            },
            indent=2,
        )
    )
    return 0 if gate["gate"]["ok"] else 1


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--probe-dir", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()
    return run(args.probe_dir, args.out_dir)


if __name__ == "__main__":
    raise SystemExit(main())
