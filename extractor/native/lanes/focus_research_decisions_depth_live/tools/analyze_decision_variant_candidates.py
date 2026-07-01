#!/usr/bin/env python3
"""Gate definition-local targeted-decision body candidates.

This consumes the recorded root probe from ``probe_current_roots.py``.  It does
not read live memory and does not edit shared collector/recorder entrypoints.
The goal is to reduce the residual decision-target search space after the
universal-offset gate has already failed.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[5]
GAME_DEFS = REPO_ROOT / "game_defs"
if str(GAME_DEFS) not in sys.path:
    sys.path.insert(0, str(GAME_DEFS))

TARGET_LIST = "active_targeted_timed_decision"
SCANNED_I32_TAG_WINDOW = "record+0x20..+0x7C i32 tag-id candidates"


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


def offset_sort_key(offset: str) -> int:
    try:
        return int(offset, 16)
    except ValueError:
        return 0


def top_values(counter: Counter[str], limit: int = 8) -> list[dict[str, Any]]:
    return [
        {"value": value, "records": count}
        for value, count in counter.most_common(limit)
    ]


def probe_vanilla_root(probe_manifest: dict[str, Any]) -> str | None:
    exe_path = probe_manifest.get("process", {}).get("path")
    if not isinstance(exe_path, str) or not exe_path:
        return None
    path = Path(exe_path)
    if path.name.lower() != "hoi4.exe":
        return None
    return str(path.parent)


def plain_node_keys(node: Any) -> set[str]:
    entries = getattr(node, "entries", None)
    if not isinstance(entries, list):
        return set()
    return {str(key).lower() for key, _value in entries if key}


def node_has_any(node: Any, keys: set[str]) -> bool:
    return bool(plain_node_keys(node) & keys)


def scalar_or_presence(value: Any) -> str | None:
    if value is None:
        return None
    if isinstance(value, str):
        return value
    return "<block>"


def looks_like_decision_body(node: Any) -> bool:
    keys = plain_node_keys(node)
    if not keys:
        return False
    decision_keys = {
        "icon",
        "cost",
        "allowed",
        "available",
        "visible",
        "target_trigger",
        "target_root_trigger",
        "target_array",
        "days_remove",
        "days_re_enable",
        "days_mission_timeout",
        "activation",
        "complete_effect",
        "remove_effect",
        "cancel_effect",
        "modifier",
        "ai_will_do",
    }
    return bool(keys & decision_keys)


def load_static_decision_defs(
    probe_manifest: dict[str, Any],
    vanilla: str | None,
    mods: list[str] | None,
    modset_json: Path | None,
    userdata: Path | None,
) -> tuple[dict[str, dict[str, Any]], dict[str, Any]]:
    """Return decision-token metadata using the repo's active mod resolver."""

    try:
        import resolver as R  # type: ignore
        from paradox_parser import Node  # type: ignore
    except Exception as exc:  # pragma: no cover - environment dependent
        return {}, {"ok": False, "error": f"static parser import failed: {exc}"}

    vanilla = vanilla or probe_vanilla_root(probe_manifest)
    try:
        layers = R.build_layers(
            vanilla=vanilla,
            mods=mods,
            modset_json=modset_json,
            userdata=userdata,
        )
    except Exception as exc:
        return {}, {"ok": False, "error": f"static resolver failed: {exc}"}

    decision_defs: dict[str, dict[str, Any]] = {}
    files_seen = 0
    parse_errors: list[str] = []
    for rel, abspath in R.ordered_files(layers, "common/decisions", (".txt",)):
        files_seen += 1
        try:
            root = R.parse_file(abspath)
        except Exception as exc:
            parse_errors.append(f"{rel}: {exc}")
            continue
        for category, body in root.entries:
            if not isinstance(body, Node):
                continue
            for token, decision_body in body.entries:
                if not token or not isinstance(decision_body, Node):
                    continue
                if token.lower() in {
                    "allowed",
                    "available",
                    "visible",
                    "icon",
                    "picture",
                    "priority",
                }:
                    continue
                if not looks_like_decision_body(decision_body):
                    continue
                keys = plain_node_keys(decision_body)
                target_array = scalar_or_presence(decision_body.get("target_array"))
                target_kind = "none"
                if "target_array" in keys:
                    if target_array and any(
                        needle in target_array.lower()
                        for needle in ("state", "core_states", "owned_states")
                    ):
                        target_kind = "state_array"
                    else:
                        target_kind = "target_array"
                elif "target_trigger" in keys or "target_root_trigger" in keys:
                    target_kind = "scripted_target"
                decision_defs[token] = {
                    "category": category,
                    "source_file": rel,
                    "has_target_trigger": "target_trigger" in keys,
                    "has_target_root_trigger": "target_root_trigger" in keys,
                    "target_array": target_array,
                    "target_kind": target_kind,
                    "has_days_remove": "days_remove" in keys,
                    "has_days_re_enable": "days_re_enable" in keys,
                    "has_days_mission_timeout": "days_mission_timeout" in keys,
                    "has_activation": "activation" in keys,
                }

    meta = {
        "ok": files_seen > 0 and not parse_errors,
        "files_seen": files_seen,
        "decision_defs": len(decision_defs),
        "layers": [getattr(layer, "label", "<unknown>") for layer in layers],
        "vanilla": vanilla,
        "parse_errors": parse_errors[:20],
    }
    return decision_defs, meta


def extract_targeted_timed_records(samples: list[dict[str, Any]]) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    for row in samples:
        if row.get("list_name") != TARGET_LIST:
            continue
        owner = row.get("tag_db_id")
        for record in row.get("records") or []:
            decision = record.get("decision")
            if not isinstance(decision, str) or not decision:
                continue
            enriched = dict(record)
            if isinstance(owner, int):
                enriched["owner_tag_db_id"] = owner
            records.append(enriched)
    return records


def summarize_by_decision(
    records: list[dict[str, Any]],
    static_defs: dict[str, dict[str, Any]],
    min_records: int,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]], list[dict[str, Any]], dict[str, Any]]:
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for record in records:
        grouped[record["decision"]].append(record)

    decision_summaries: list[dict[str, Any]] = []
    candidate_slices: list[dict[str, Any]] = []
    rejected_slices: list[dict[str, Any]] = []
    static_found = 0
    static_target_scoped = 0
    static_target_scoped_with_candidate = 0
    static_target_scoped_without_candidate = 0

    for decision, rows in sorted(grouped.items(), key=lambda item: (-len(item[1]), item[0])):
        owners = Counter(
            str(row.get("owner_tag_db_id"))
            for row in rows
            if isinstance(row.get("owner_tag_db_id"), int)
        )
        tag_offsets: dict[str, dict[str, Any]] = {}
        pointer_offsets: dict[str, dict[str, Any]] = {}
        raw_tag_offsets: dict[str, dict[str, Any]] = defaultdict(
            lambda: {"records": 0, "owner_equal_records": 0, "values": Counter()}
        )
        raw_pointer_offsets: dict[str, dict[str, Any]] = defaultdict(
            lambda: {"records": 0, "owner_equal_records": 0, "values": Counter()}
        )
        for row in rows:
            owner = row.get("owner_tag_db_id")
            seen_offsets: set[str] = set()
            for candidate in row.get("tag_id_field_candidates") or []:
                offset = candidate.get("offset")
                tag_db_id = candidate.get("tag_db_id")
                tag = candidate.get("tag")
                if not isinstance(offset, str) or not isinstance(tag_db_id, int):
                    continue
                if offset in seen_offsets:
                    continue
                seen_offsets.add(offset)
                item = raw_tag_offsets[offset]
                item["records"] += 1
                if isinstance(owner, int) and owner == tag_db_id:
                    item["owner_equal_records"] += 1
                item["values"][f"{tag_db_id}:{tag or '?'}"] += 1
            seen_pointer_offsets: set[str] = set()
            for candidate in row.get("target_country_pointer_candidates") or []:
                offset = candidate.get("offset")
                tag_db_id = candidate.get("tag_db_id")
                if not isinstance(offset, str) or not isinstance(tag_db_id, int):
                    continue
                if offset in seen_pointer_offsets:
                    continue
                seen_pointer_offsets.add(offset)
                item = raw_pointer_offsets[offset]
                item["records"] += 1
                if isinstance(owner, int) and owner == tag_db_id:
                    item["owner_equal_records"] += 1
                item["values"][str(tag_db_id)] += 1

        record_count = len(rows)
        full_nonconstant_offsets: list[dict[str, Any]] = []
        full_constant_offsets: list[dict[str, Any]] = []
        sparse_offsets: list[dict[str, Any]] = []
        for offset, raw in sorted(raw_tag_offsets.items(), key=lambda item: offset_sort_key(item[0])):
            values = raw["values"]
            coverage = raw["records"] / record_count if record_count else 0.0
            offset_summary = {
                "records": raw["records"],
                "coverage_ratio": round(coverage, 6),
                "owner_equal_records": raw["owner_equal_records"],
                "unique_value_count": len(values),
                "top_values": top_values(values),
            }
            tag_offsets[offset] = offset_summary
            if raw["records"] == record_count:
                if len(values) > 1:
                    full_nonconstant_offsets.append({"offset": offset, **offset_summary})
                else:
                    full_constant_offsets.append({"offset": offset, **offset_summary})
            else:
                sparse_offsets.append({"offset": offset, **offset_summary})

        for offset, raw in sorted(raw_pointer_offsets.items(), key=lambda item: offset_sort_key(item[0])):
            values = raw["values"]
            coverage = raw["records"] / record_count if record_count else 0.0
            pointer_offsets[offset] = {
                "records": raw["records"],
                "coverage_ratio": round(coverage, 6),
                "owner_equal_records": raw["owner_equal_records"],
                "unique_value_count": len(values),
                "top_values": top_values(values),
            }

        static = static_defs.get(decision)
        if static:
            static_found += 1
            if static.get("target_kind") != "none":
                static_target_scoped += 1
                if full_nonconstant_offsets:
                    static_target_scoped_with_candidate += 1
                else:
                    static_target_scoped_without_candidate += 1

        summary = {
            "decision": decision,
            "records": record_count,
            "owner_country_count": len(owners),
            "static": static,
            "tag_offsets": tag_offsets,
            "country_pointer_offsets": pointer_offsets,
            "full_nonconstant_tag_offsets": full_nonconstant_offsets,
            "full_constant_tag_offsets": full_constant_offsets,
            "sparse_tag_offsets": sparse_offsets,
        }
        decision_summaries.append(summary)

        if record_count >= min_records and full_nonconstant_offsets:
            candidate_slices.append(
                {
                    "decision": decision,
                    "status": "CANDIDATE_NOT_PRODUCTION",
                    "reason": (
                        "A definition-local nonconstant tag-id offset covers every "
                        "sampled row for this decision, but no same-frame oracle proves "
                        "that the value is the real target."
                    ),
                    "records": record_count,
                    "owner_country_count": len(owners),
                    "static": static,
                    "candidate_offsets": full_nonconstant_offsets,
                    "promotion_requirement": (
                        "same-frame owner+decision+known target country/state/province "
                        "oracle for this exact decision token and vtable 0x27D5038"
                    ),
                }
            )
        elif record_count >= min_records:
            rejected_slices.append(
                {
                    "decision": decision,
                    "candidate": f"{decision}: {SCANNED_I32_TAG_WINDOW} as definition-local exact country target",
                    "verdict": "REJECTED_FOR_COUNTRY_TAG_TARGET",
                    "reason": (
                        "No nonconstant valid tag-id offset covers every sampled row "
                        "for this decision token. The country-target tag, if present, "
                        "is not exposed by the scanned i32 tag-candidate window."
                    ),
                    "records": record_count,
                    "owner_country_count": len(owners),
                    "static": static,
                    "full_constant_tag_offsets": full_constant_offsets,
                    "sparse_tag_offsets": sparse_offsets[:8],
                }
            )

    meta = {
        "decision_token_count": len(grouped),
        "static_defs_found_for_sampled_tokens": static_found,
        "static_target_scoped_sampled_tokens": static_target_scoped,
        "static_target_scoped_with_candidate_offset": static_target_scoped_with_candidate,
        "static_target_scoped_without_candidate_offset": static_target_scoped_without_candidate,
    }
    return decision_summaries, candidate_slices, rejected_slices, meta


def write_markdown(path: Path, gate: dict[str, Any]) -> None:
    lines = [
        "# Decision Variant Candidate Gate",
        "",
        f"Created UTC: `{gate['created_utc']}`",
        f"Completion status: `{gate['completion_status']}`",
        f"Backlog reduced: `{str(gate['backlog_reduced']).lower()}`",
        "",
        "## Summary",
        "",
        f"- Records sampled: `{gate['records_sampled']}`.",
        f"- Decision tokens sampled: `{gate['decision_token_count']}`.",
        f"- Candidate slices: `{len(gate['candidate_slices'])}`.",
        f"- Rejected token-local country-tag slices: `{len(gate['rejected_candidate_slices'])}`.",
        f"- Static defs found for sampled tokens: `{gate['static_meta'].get('static_defs_found_for_sampled_tokens', 0)}`.",
        "",
        "## Candidate Slices",
        "",
    ]
    for item in gate["candidate_slices"][:20]:
        offsets = ", ".join(offset["offset"] for offset in item["candidate_offsets"])
        target_kind = (item.get("static") or {}).get("target_kind", "unknown")
        lines.append(
            f"- `{item['decision']}`: offsets `{offsets}` cover `{item['records']}` sampled rows; static target kind `{target_kind}`. Not production without oracle."
        )
    if not gate["candidate_slices"]:
        lines.append("- None.")
    lines.extend(["", "## Rejected Slices", ""])
    for item in gate["rejected_candidate_slices"][:20]:
        lines.append(f"- `{item['decision']}`: {item['reason']}")
    if not gate["rejected_candidate_slices"]:
        lines.append("- None.")
    lines.extend(["", "## Remaining Blocker", ""])
    lines.append(
        "- Target body extraction remains blocked until a same-frame oracle proves "
        "which candidate field is the target for vtable `0x27D5038`, or until a "
        "new live root exposes target fields directly."
    )
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def build_gate(args: argparse.Namespace) -> dict[str, Any]:
    probe_dir = args.probe_dir
    root_manifest = load_json(probe_dir / "root_probe_manifest.json")
    samples = load_jsonl(probe_dir / "decision_body_samples.jsonl")
    records = extract_targeted_timed_records(samples)

    static_defs: dict[str, dict[str, Any]] = {}
    static_resolver_meta: dict[str, Any] = {"ok": False, "skipped": True}
    if args.with_static_defs:
        static_defs, static_resolver_meta = load_static_decision_defs(
            root_manifest,
            args.vanilla,
            args.mods,
            args.modset_json,
            args.userdata,
        )

    summaries, candidates, rejections, static_meta = summarize_by_decision(
        records,
        static_defs,
        args.min_records,
    )
    static_meta["resolver"] = static_resolver_meta

    return {
        "schema": "focus_research_decisions_depth_live_decision_variant_candidate_gate_v1",
        "lane": "focus_research_decisions_depth_live",
        "created_utc": utc_now(),
        "completion_status": "NOT COMPLETE",
        "backlog_reduced": bool(candidates or rejections),
        "probe_dir": str(probe_dir),
        "build_gate": root_manifest.get("build_gate", {}),
        "target_list": TARGET_LIST,
        "records_sampled": len(records),
        "decision_token_count": static_meta["decision_token_count"],
        "scanned_window": SCANNED_I32_TAG_WINDOW,
        "min_records": args.min_records,
        "static_meta": static_meta,
        "candidate_slices": candidates,
        "rejected_candidate_slices": rejections,
        "decision_summaries": summaries,
        "remaining_blocker": {
            "field": "active_targeted_timed_decision target body",
            "status": "BLOCKED",
            "reason": (
                "Definition-local tag candidates are still not target fields until "
                "same-frame owner+decision+known-target oracle evidence proves the "
                "field for vtable 0x27D5038. Sparse or absent tag candidates reject "
                "country-target tag extraction for those sampled decision tokens."
            ),
            "next_evidence_needed": (
                "same-frame oracle keyed by owner tag, decision token, and visible or "
                "save-corroborated target country/state/province"
            ),
        },
        "gate": {
            "ok": bool(root_manifest.get("ok")) and bool(records) and bool(candidates or rejections),
            "errors": [] if root_manifest.get("ok") else ["root probe manifest not ok"],
        },
    }


def run() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--probe-dir", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--min-records", type=int, default=4)
    parser.add_argument("--with-static-defs", action="store_true")
    parser.add_argument("--vanilla", default=None)
    parser.add_argument("--mods", nargs="*", default=None)
    parser.add_argument("--modset-json", type=Path, default=None)
    parser.add_argument("--userdata", type=Path, default=None)
    args = parser.parse_args()

    gate = build_gate(args)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "decision_variant_candidate_gate.json").write_text(
        json.dumps(gate, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    write_markdown(args.out_dir / "DECISION_VARIANT_CANDIDATE_GATE.md", gate)
    print(
        json.dumps(
            {
                "ok": gate["gate"]["ok"],
                "backlog_reduced": gate["backlog_reduced"],
                "records_sampled": gate["records_sampled"],
                "decision_token_count": gate["decision_token_count"],
                "candidate_slices": len(gate["candidate_slices"]),
                "rejected_candidate_slices": len(gate["rejected_candidate_slices"]),
                "out_dir": str(args.out_dir),
            },
            indent=2,
        )
    )
    return 0 if gate["gate"]["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(run())
