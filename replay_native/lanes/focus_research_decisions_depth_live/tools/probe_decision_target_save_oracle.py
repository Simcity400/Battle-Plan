#!/usr/bin/env python3
"""Validate targeted-timed decision target fields against a save oracle.

This is lane-local proof code. It reads a live root probe JSONL plus a same-run
save/autosave, then compares the live active_targeted_timed_decision
``record+0x20`` tag candidate against the save-side targeted decision
``target=TAG`` rows. It does not edit shared collector or recorder entrypoints.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[4]
SAVE_ORACLE = REPO_ROOT / "save_oracle"
if str(SAVE_ORACLE) not in sys.path:
    sys.path.insert(0, str(SAVE_ORACLE))

import parse_save as ps  # type: ignore  # noqa: E402
from parse_save import (  # type: ignore  # noqa: E402
    LBRACE,
    atom_value,
    iter_block_entries,
    _skip_block_body,
)


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
LIVE_LIST = "active_targeted_timed_decision"
SAVE_TARGET_LISTS = {
    "active_targeted_decision",
    "active_targeted_timed_decision",
}
TARGET_OFFSET = "0x20"
TARGETED_TIMED_VTABLE_RVA = "0x27D5038"
TAG_RE = re.compile(r"^[A-Z0-9-]{3}$")


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def load_jsonl(path: Path) -> list[dict[str, Any]]:
    rows = []
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


def default_autosave_path() -> Path:
    return (
        Path.home()
        / "OneDrive"
        / "Documents"
        / "Paradox Interactive"
        / "the game"
        / "save games"
        / "autosave.hoi4"
    )


def target_kind(value: str | None) -> str:
    if not value:
        return "missing"
    try:
        int(value)
    except ValueError:
        if TAG_RE.match(value) and any(ch.isalpha() for ch in value):
            return "country_tag"
        return "other"
    return "state_or_numeric"


def parse_decision_record(tz: ps.Tokenizer) -> dict[str, Any]:
    rec: dict[str, Any] = {
        "decision": None,
        "target": None,
        "target_kind": "missing",
        "ignore": None,
        "days": None,
        "state": None,
    }
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        val = atom_value(opener)
        if val is None:
            continue
        if key == "decision":
            rec["decision"] = val
        elif key == "target":
            rec["target"] = val
            rec["target_kind"] = target_kind(val)
        elif key == "ignore":
            rec["ignore"] = val
        elif key == "days":
            rec["days"] = val
        elif key == "state":
            rec["state"] = val
    return rec


def parse_decision_status(tz: ps.Tokenizer, owner_tag: str, rows: list[dict[str, Any]]) -> None:
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key in SAVE_TARGET_LISTS:
            rec = parse_decision_record(tz)
            rec["owner_tag"] = owner_tag
            rec["save_list_name"] = key
            if rec.get("decision"):
                rows.append(rec)
        else:
            _skip_block_body(tz)


def parse_country_block(tz: ps.Tokenizer, owner_tag: str, rows: list[dict[str, Any]]) -> None:
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key == "decision_status":
            parse_decision_status(tz, owner_tag, rows)
        else:
            _skip_block_body(tz)


def parse_save_decisions(text: str) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    tz = ps.Tokenizer(text)
    rows: list[dict[str, Any]] = []
    meta: dict[str, Any] = {
        "date": None,
        "version": None,
        "player": None,
    }
    while True:
        tok = tz.next()
        if tok is None:
            break
        if tok is LBRACE:
            _skip_block_body(tz)
            continue
        key = atom_value(tok)
        if key is None:
            continue
        if tz.peek() is not ps.EQUALS:
            continue
        tz.next()
        opener = tz.next()
        if opener is LBRACE:
            if key == "countries":
                for owner_tag, country_opener in iter_block_entries(tz):
                    if country_opener is LBRACE:
                        parse_country_block(tz, owner_tag, rows)
            else:
                _skip_block_body(tz)
        else:
            val = atom_value(opener)
            if key in meta:
                meta[key] = val
    meta["decision_rows"] = len(rows)
    meta["target_rows_by_kind"] = dict(Counter(row["target_kind"] for row in rows))
    meta["rows_by_save_list"] = dict(Counter(row["save_list_name"] for row in rows))
    return rows, meta


def candidate_at_offset(record: dict[str, Any], offset: str) -> dict[str, Any] | None:
    for candidate in record.get("tag_id_field_candidates") or []:
        if candidate.get("offset") == offset and isinstance(candidate.get("tag"), str):
            return candidate
    return None


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
            candidate = candidate_at_offset(record, TARGET_OFFSET)
            out.append(
                {
                    "decision": decision,
                    "owner_tag_db_id": owner_id if isinstance(owner_id, int) else None,
                    "record_ptr": record.get("record_ptr"),
                    "record_vtable_rva": record.get("record_vtable_rva"),
                    "target_offset": TARGET_OFFSET,
                    "target_tag": candidate.get("tag") if candidate else None,
                    "target_tag_db_id": candidate.get("tag_db_id") if candidate else None,
                    "has_target_candidate": candidate is not None,
                }
            )
    return out


def counter_rows(rows: list[dict[str, Any]], key_fields: tuple[str, ...]) -> Counter[tuple[Any, ...]]:
    counter: Counter[tuple[Any, ...]] = Counter()
    for row in rows:
        counter[tuple(row.get(field) for field in key_fields)] += 1
    return counter


def decision_breakdown(
    live_rows: list[dict[str, Any]],
    save_rows: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    live_by_decision: dict[str, Counter[str]] = defaultdict(Counter)
    save_by_decision: dict[str, Counter[str]] = defaultdict(Counter)
    for row in live_rows:
        if row.get("target_tag"):
            live_by_decision[row["decision"]][row["target_tag"]] += 1
    for row in save_rows:
        if row.get("target_kind") == "country_tag":
            save_by_decision[row["decision"]][row["target"]] += 1
    decisions = sorted(set(live_by_decision) | set(save_by_decision))
    out = []
    for decision in decisions:
        live_counter = live_by_decision[decision]
        save_counter = save_by_decision[decision]
        out.append(
            {
                "decision": decision,
                "live_rows": sum(live_counter.values()),
                "save_rows": sum(save_counter.values()),
                "matched": live_counter == save_counter,
                "target_count": len(live_counter),
                "sample_targets": [
                    {"target": target, "records": count}
                    for target, count in live_counter.most_common(8)
                ],
                "live_minus_save": [
                    {"decision": decision, "target": target, "records": count}
                    for (target,), count in (
                        Counter({(k,): v for k, v in live_counter.items()})
                        - Counter({(k,): v for k, v in save_counter.items()})
                    ).items()
                ][:8],
                "save_minus_live": [
                    {"decision": decision, "target": target, "records": count}
                    for (target,), count in (
                        Counter({(k,): v for k, v in save_counter.items()})
                        - Counter({(k,): v for k, v in live_counter.items()})
                    ).items()
                ][:8],
            }
        )
    return out


def build_gate(args: argparse.Namespace) -> dict[str, Any]:
    probe_dir = args.probe_dir
    root_manifest = load_json(probe_dir / "root_probe_manifest.json")
    samples = load_jsonl(probe_dir / "decision_body_samples.jsonl")
    live_rows = live_targeted_timed_rows(samples)
    live_candidate_rows = [row for row in live_rows if row.get("has_target_candidate")]
    live_vtable_rows = [
        row for row in live_rows if row.get("record_vtable_rva") == TARGETED_TIMED_VTABLE_RVA
    ]

    save_path = args.save or default_autosave_path()
    rakaly = ps.find_rakaly(args.rakaly)
    text, was_melted = ps.melt_if_needed(save_path, rakaly, args.out_dir)
    save_rows, save_meta = parse_save_decisions(text)
    save_country_rows = [
        row
        for row in save_rows
        if row.get("target_kind") == "country_tag"
        and row.get("save_list_name") in SAVE_TARGET_LISTS
    ]

    live_counter = counter_rows(live_candidate_rows, ("decision", "target_tag"))
    save_counter = counter_rows(save_country_rows, ("decision", "target"))
    exact_multiset_match = live_counter == save_counter
    all_live_rows_have_candidate = len(live_candidate_rows) == len(live_rows)
    all_live_rows_targeted_timed_vtable = len(live_vtable_rows) == len(live_rows)
    exact_row_count_match = len(live_candidate_rows) == len(save_country_rows)
    by_decision = decision_breakdown(live_candidate_rows, save_country_rows)
    validated_decisions = [row for row in by_decision if row["matched"]]
    unmatched_decisions = [row for row in by_decision if not row["matched"]]
    validated_live_country_rows = sum(int(row["live_rows"]) for row in validated_decisions)
    validated_save_country_rows = sum(int(row["save_rows"]) for row in validated_decisions)
    unmatched_live_country_rows = sum(int(row["live_rows"]) for row in unmatched_decisions)
    unmatched_save_country_rows = sum(int(row["save_rows"]) for row in unmatched_decisions)

    strict_all_country_rows_ok = (
        bool(root_manifest.get("ok"))
        and (root_manifest.get("build_gate") or {}).get("actual_sha256", "").lower()
        == EXPECTED_SHA256
        and bool(live_rows)
        and exact_multiset_match
        and all_live_rows_have_candidate
        and all_live_rows_targeted_timed_vtable
        and exact_row_count_match
    )
    partial_country_target_ok = (
        bool(root_manifest.get("ok"))
        and (root_manifest.get("build_gate") or {}).get("actual_sha256", "").lower()
        == EXPECTED_SHA256
        and all_live_rows_targeted_timed_vtable
        and validated_live_country_rows > 0
        and validated_live_country_rows == validated_save_country_rows
    )
    return {
        "schema": "focus_research_decisions_depth_live_decision_target_save_oracle_gate_v1",
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
        "validated_field": {
            "list": LIVE_LIST,
            "record_vtable_rva": TARGETED_TIMED_VTABLE_RVA,
            "offset": TARGET_OFFSET,
            "field": "country_target_tag_db_id",
            "target_encoding": "live tag_db_id that maps to save target country tag",
            "status": (
                "VALIDATED_FOR_ALL_COUNTRY_TARGET_ROWS"
                if strict_all_country_rows_ok
                else "VALIDATED_FOR_MATCHED_COUNTRY_TARGET_ROWS"
                if partial_country_target_ok
                else "NOT_VALIDATED"
            ),
        },
        "live_rows": len(live_rows),
        "live_vtable_rows": len(live_vtable_rows),
        "live_candidate_rows": len(live_candidate_rows),
        "save_target_rows": len(save_rows),
        "save_country_target_rows": len(save_country_rows),
        "exact_row_count_match": exact_row_count_match,
        "exact_multiset_match_by_decision_target": exact_multiset_match,
        "all_live_rows_have_record_0x20_candidate": all_live_rows_have_candidate,
        "all_live_rows_have_targeted_timed_vtable": all_live_rows_targeted_timed_vtable,
        "validated_decision_count": len(validated_decisions),
        "unmatched_decision_count": len(unmatched_decisions),
        "validated_live_country_target_rows": validated_live_country_rows,
        "validated_save_country_target_rows": validated_save_country_rows,
        "unmatched_live_country_target_rows": unmatched_live_country_rows,
        "unmatched_save_country_target_rows": unmatched_save_country_rows,
        "strict_all_country_rows_ok": strict_all_country_rows_ok,
        "decision_breakdown": by_decision,
        "backlog_reduced": partial_country_target_ok,
        "remaining_blocker": {
            "field": "decision_target_cooldown_body_fields",
            "status": "PARTIALLY_BLOCKED",
            "reason": (
                "active_targeted_timed_decision record+0x20 is validated as the "
                "country target field for the exact save/live matched country-"
                "target subset, but unmatched transition rows, cooldown/day/state "
                "body fields, and non-country target bodies still need exact "
                "same-frame oracle evidence or direct live roots."
            ),
            "next_evidence_needed": (
                "same-frame save/live oracle for days/state/cooldown encodings and "
                "numeric state/province target bodies, or static/dynamic roots for "
                "those fields"
            ),
        },
        "gate": {
            "ok": partial_country_target_ok,
            "errors": []
            if partial_country_target_ok
            else [
                "build mismatch, missing live rows, no exact save/live country-target subset, or target candidate gap"
            ],
        },
    }


def write_markdown(path: Path, gate: dict[str, Any]) -> None:
    field = gate["validated_field"]
    lines = [
        "# Decision Target Save Oracle Gate",
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
        f"- Live rows with `record+0x20` tag candidate: `{gate['live_candidate_rows']}`.",
        f"- Save country-target rows: `{gate['save_country_target_rows']}`.",
        f"- Validated live country-target rows: `{gate['validated_live_country_target_rows']}`.",
        f"- Unmatched live country-target rows: `{gate['unmatched_live_country_target_rows']}`.",
        f"- Unmatched save country-target rows: `{gate['unmatched_save_country_target_rows']}`.",
        f"- Exact row-count match: `{str(gate['exact_row_count_match']).lower()}`.",
        f"- Exact multiset match by `(decision,target)`: `{str(gate['exact_multiset_match_by_decision_target']).lower()}`.",
        f"- Strict all-country-row match: `{str(gate['strict_all_country_rows_ok']).lower()}`.",
        f"- Validated decisions: `{gate['validated_decision_count']}`.",
        f"- Unmatched decisions: `{gate['unmatched_decision_count']}`.",
        "",
        "## Matched Examples",
        "",
    ]
    matched = [row for row in gate["decision_breakdown"] if row["matched"]]
    for row in sorted(matched, key=lambda item: (-item["live_rows"], item["decision"]))[:20]:
        sample = ", ".join(
            f"{item['target']}:{item['records']}" for item in row["sample_targets"][:5]
        )
        lines.append(
            f"- `{row['decision']}`: `{row['live_rows']}` rows, "
            f"`{row['target_count']}` targets; sample `{sample}`."
        )
    if not matched:
        lines.append("- None.")
    if gate["unmatched_decision_count"]:
        lines.extend(["", "## Unmatched Decisions", ""])
        for row in [item for item in gate["decision_breakdown"] if not item["matched"]][:20]:
            lines.append(
                f"- `{row['decision']}`: live `{row['live_rows']}`, save `{row['save_rows']}`."
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
    args = parser.parse_args()
    args.out_dir.mkdir(parents=True, exist_ok=True)

    gate = build_gate(args)
    (args.out_dir / "decision_target_save_oracle_gate.json").write_text(
        json.dumps(gate, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    write_markdown(args.out_dir / "DECISION_TARGET_SAVE_ORACLE_GATE.md", gate)
    print(
        json.dumps(
            {
                "ok": gate["gate"]["ok"],
                "backlog_reduced": gate["backlog_reduced"],
                "validated_field": gate["validated_field"],
                "live_rows": gate["live_rows"],
                "save_country_target_rows": gate["save_country_target_rows"],
                "validated_live_country_target_rows": gate[
                    "validated_live_country_target_rows"
                ],
                "unmatched_live_country_target_rows": gate[
                    "unmatched_live_country_target_rows"
                ],
                "validated_decision_count": gate["validated_decision_count"],
                "unmatched_decision_count": gate["unmatched_decision_count"],
                "out_dir": str(args.out_dir),
            },
            indent=2,
        )
    )
    return 0 if gate["gate"]["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(run())
