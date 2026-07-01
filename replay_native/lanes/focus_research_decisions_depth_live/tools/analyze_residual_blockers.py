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


def load_optional_json(path: Path | None) -> dict[str, Any] | None:
    if path is None:
        return None
    return load_json(path)


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


def production_flags(
    manifest: dict[str, Any] | None,
    depth_gate: dict[str, Any] | None,
    manifest_path: Path | None,
    replay_verified: bool = False,
) -> dict[str, Any]:
    if manifest is None:
        return {
            "dll_only": False,
            "compact_stream": False,
            "rust_decoded": bool(depth_gate and depth_gate.get("ok")),
            "hoi4replay_present": False,
            "hoi4replay_verified": False,
            "speed_five_current_equivalent_hourly_or_lower": False,
            "zero_skipped_hours": False,
            "no_partial_source_worker_bridge_failures": False,
            "manifest": None,
            "depth_gate_ok": bool(depth_gate and depth_gate.get("ok")),
        }

    compact = manifest.get("compact_stream") or {}
    replay_file = manifest.get("replay_file")
    replay_path_exists = False
    if manifest_path and isinstance(replay_file, str):
        replay_path_exists = (manifest_path.parent / replay_file).exists()

    parse_errors = manifest.get("parse_errors") or []
    ordering_errors = manifest.get("ordering_errors") or []
    hourly = manifest.get("hourly_requirement") or {}
    production_note = manifest.get("production_path_note") or ""
    no_failure_flags = [
        not bool(compact.get("partial_data_seen")),
        not bool(compact.get("source_unavailable_seen")),
        not bool(compact.get("bridge_unavailable_seen")),
        not bool(compact.get("worker_panic_seen")),
        not bool(parse_errors),
        not bool(ordering_errors),
    ]

    return {
        "dll_only": (
            manifest.get("stream_format") == "compact"
            and "no full JSON root frames" in production_note
            and bool(manifest.get("dll"))
        ),
        "compact_stream": (
            manifest.get("stream_format") == "compact"
            and bool(compact.get("decodable"))
            and bool((compact.get("domain_coverage") or {}).get("ok"))
        ),
        "rust_decoded": bool(depth_gate and depth_gate.get("ok")) and not bool(parse_errors),
        "hoi4replay_present": replay_path_exists,
        "hoi4replay_verified": replay_path_exists and replay_verified,
        "speed_five_current_equivalent_hourly_or_lower": bool(hourly.get("met")),
        "zero_skipped_hours": manifest.get("skipped_game_hours") == 0,
        "no_partial_source_worker_bridge_failures": all(no_failure_flags),
        "manifest": str(manifest_path) if manifest_path else None,
        "depth_gate_ok": bool(depth_gate and depth_gate.get("ok")),
        "game_hours_advanced": manifest.get("game_hours_advanced"),
        "unique_game_hour_count": manifest.get("unique_game_hour_count"),
        "scheduled_domains": manifest.get("scheduled_domains"),
        "domains_seen": compact.get("domains_seen"),
        "partial_data_seen": compact.get("partial_data_seen"),
        "source_unavailable_seen": compact.get("source_unavailable_seen"),
        "bridge_unavailable_seen": compact.get("bridge_unavailable_seen"),
        "worker_panic_seen": compact.get("worker_panic_seen"),
    }


def all_production_flags_met(flags: dict[str, Any]) -> bool:
    required = [
        "dll_only",
        "compact_stream",
        "rust_decoded",
        "hoi4replay_verified",
        "speed_five_current_equivalent_hourly_or_lower",
        "zero_skipped_hours",
        "no_partial_source_worker_bridge_failures",
    ]
    return all(bool(flags.get(key)) for key in required)


def compact_gate_summary(gate: dict[str, Any] | None) -> dict[str, Any]:
    if gate is None:
        return {}
    return {
        "ok": gate.get("ok"),
        "data_records": gate.get("data_records"),
        "focus_event_rows": gate.get("focus_event_rows"),
        "focus_completed_exact_rows": gate.get("focus_completed_exact_rows"),
        "focus_completed_inferred_rows": gate.get("focus_completed_inferred_rows"),
        "focus_completed_unresolved_rows": gate.get("focus_completed_unresolved_rows"),
        "research_eta_rows": gate.get("research_eta_rows"),
        "decision_status_rows": gate.get("decision_status_rows"),
        "decision_event_rows": gate.get("decision_event_rows"),
        "decision_target_rows": gate.get("decision_target_rows"),
        "decision_country_target_rows": gate.get("decision_country_target_rows"),
        "decision_numeric_target_rows": gate.get("decision_numeric_target_rows"),
        "badge_rows": gate.get("badge_rows"),
        "focus_pause_badge_rows": gate.get("focus_pause_badge_rows"),
        "decision_target_badge_rows": gate.get("decision_target_badge_rows"),
        "direct_event_badge_rows": gate.get("direct_event_badge_rows"),
        "parse_errors": gate.get("parse_errors"),
    }


def decision_gate_summary(
    body_gate: dict[str, Any] | None,
    variant_gate: dict[str, Any] | None,
    target_save_oracle_gate: dict[str, Any] | None = None,
    numeric_target_save_oracle_gate: dict[str, Any] | None = None,
) -> dict[str, Any]:
    body_summary = {}
    if body_gate:
        body_summary = {
            "ok": (body_gate.get("gate") or {}).get("ok"),
            "rejected_candidate_count": body_gate.get("rejected_candidate_count"),
            "remaining_blocker_count": len(body_gate.get("remaining_blockers") or []),
            "per_list_records": {
                name: {
                    "records_sampled": details.get("records_sampled"),
                    "def_name_valid_records": details.get("def_name_valid_records"),
                    "timed_body_contract_records": details.get("timed_body_contract_records"),
                    "manifest_body_verdict": details.get("manifest_body_verdict"),
                    "vtable_rvas": details.get("vtable_rvas"),
                }
                for name, details in (body_gate.get("per_list_details") or {}).items()
            },
        }

    variant_summary = {}
    if variant_gate:
        static_meta = variant_gate.get("static_meta") or {}
        resolver = static_meta.get("resolver") or {}
        variant_summary = {
            "ok": (variant_gate.get("gate") or {}).get("ok"),
            "records_sampled": variant_gate.get("records_sampled"),
            "decision_token_count": variant_gate.get("decision_token_count"),
            "candidate_slices": len(variant_gate.get("candidate_slices") or []),
            "rejected_candidate_slices": len(
                variant_gate.get("rejected_candidate_slices") or []
            ),
            "static_defs_found_for_sampled_tokens": static_meta.get(
                "static_defs_found_for_sampled_tokens"
            ),
            "static_target_scoped_sampled_tokens": static_meta.get(
                "static_target_scoped_sampled_tokens"
            ),
            "static_target_scoped_with_candidate_offset": static_meta.get(
                "static_target_scoped_with_candidate_offset"
            ),
            "static_target_scoped_without_candidate_offset": static_meta.get(
                "static_target_scoped_without_candidate_offset"
            ),
            "static_resolver_ok": resolver.get("ok"),
            "remaining_blocker": variant_gate.get("remaining_blocker"),
        }

    target_oracle_summary = {}
    if target_save_oracle_gate:
        target_oracle_summary = {
            "ok": (target_save_oracle_gate.get("gate") or {}).get("ok"),
            "backlog_reduced": target_save_oracle_gate.get("backlog_reduced"),
            "validated_field": target_save_oracle_gate.get("validated_field"),
            "live_rows": target_save_oracle_gate.get("live_rows"),
            "live_candidate_rows": target_save_oracle_gate.get("live_candidate_rows"),
            "save_country_target_rows": target_save_oracle_gate.get("save_country_target_rows"),
            "validated_live_country_target_rows": target_save_oracle_gate.get(
                "validated_live_country_target_rows"
            ),
            "unmatched_live_country_target_rows": target_save_oracle_gate.get(
                "unmatched_live_country_target_rows"
            ),
            "unmatched_save_country_target_rows": target_save_oracle_gate.get(
                "unmatched_save_country_target_rows"
            ),
            "validated_decision_count": target_save_oracle_gate.get(
                "validated_decision_count"
            ),
            "unmatched_decision_count": target_save_oracle_gate.get(
                "unmatched_decision_count"
            ),
            "strict_all_country_rows_ok": target_save_oracle_gate.get(
                "strict_all_country_rows_ok"
            ),
        }

    numeric_target_oracle_summary = {}
    if numeric_target_save_oracle_gate:
        basis = numeric_target_save_oracle_gate.get("validation_basis") or {}
        numeric_target_oracle_summary = {
            "ok": (numeric_target_save_oracle_gate.get("gate") or {}).get("ok"),
            "backlog_reduced": numeric_target_save_oracle_gate.get("backlog_reduced"),
            "validated_field": numeric_target_save_oracle_gate.get("validated_field"),
            "live_rows": numeric_target_save_oracle_gate.get("live_rows"),
            "save_numeric_target_rows": numeric_target_save_oracle_gate.get(
                "save_numeric_target_rows"
            ),
            "validated_live_numeric_target_rows": numeric_target_save_oracle_gate.get(
                "validated_live_numeric_target_rows"
            ),
            "unmatched_live_numeric_target_rows": numeric_target_save_oracle_gate.get(
                "unmatched_live_numeric_target_rows"
            ),
            "unmatched_save_numeric_target_rows": numeric_target_save_oracle_gate.get(
                "unmatched_save_numeric_target_rows"
            ),
            "matched_intersection_ratio": basis.get("matched_intersection_ratio"),
            "runner_up_intersection_rows": basis.get("runner_up_intersection_rows"),
            "strict_all_numeric_rows_ok": numeric_target_save_oracle_gate.get(
                "strict_all_numeric_rows_ok"
            ),
        }

    return {
        "body_gate": body_summary,
        "variant_gate": variant_summary,
        "target_save_oracle_gate": target_oracle_summary,
        "numeric_target_save_oracle_gate": numeric_target_oracle_summary,
    }


def focus_pause_gate_summary(focus_pause_gate: dict[str, Any] | None) -> dict[str, Any]:
    if not focus_pause_gate:
        return {}
    return {
        "ok": (focus_pause_gate.get("gate") or {}).get("ok"),
        "backlog_reduced": focus_pause_gate.get("backlog_reduced"),
        "focus_status_rows": focus_pause_gate.get("focus_status_rows"),
        "focus_context_rows": focus_pause_gate.get("focus_context_rows"),
        "no_focus_context_rows": focus_pause_gate.get("no_focus_context_rows"),
        "rejected_byte_candidate_offsets": focus_pause_gate.get("rejected_byte_candidate_offsets"),
        "direct_rejected_byte_candidate_offsets": focus_pause_gate.get("direct_rejected_byte_candidate_offsets"),
        "structural_overlap_rejected_byte_offsets": focus_pause_gate.get("structural_overlap_rejected_byte_offsets"),
        "inconclusive_byte_candidate_offsets": focus_pause_gate.get("inconclusive_byte_candidate_offsets"),
        "remaining_blocker": focus_pause_gate.get("remaining_blocker"),
    }


def focus_pause_wide_gate_summary(focus_pause_wide_gate: dict[str, Any] | None) -> dict[str, Any]:
    if not focus_pause_wide_gate:
        return {}
    return {
        "ok": (focus_pause_wide_gate.get("gate") or {}).get("ok"),
        "backlog_reduced": focus_pause_wide_gate.get("backlog_reduced"),
        "focus_status_rows": focus_pause_wide_gate.get("focus_status_rows"),
        "focus_context_rows": focus_pause_wide_gate.get("focus_context_rows"),
        "no_focus_context_rows": focus_pause_wide_gate.get("no_focus_context_rows"),
        "scan_range": focus_pause_wide_gate.get("scan_range"),
        "excluded_prior_rejected_window": focus_pause_wide_gate.get("excluded_prior_rejected_window"),
        "outside_window_rejected_byte_candidate_offsets": focus_pause_wide_gate.get(
            "outside_window_rejected_byte_candidate_offsets"
        ),
        "known_field_rejected_byte_offsets": focus_pause_wide_gate.get("known_field_rejected_byte_offsets"),
        "direct_rejected_byte_candidate_offsets": focus_pause_wide_gate.get(
            "direct_rejected_byte_candidate_offsets"
        ),
        "structural_rejected_byte_offsets": focus_pause_wide_gate.get("structural_rejected_byte_offsets"),
        "scalar_rejected_byte_offsets": focus_pause_wide_gate.get("scalar_rejected_byte_offsets"),
        "outside_window_inconclusive_byte_candidate_offsets": focus_pause_wide_gate.get(
            "outside_window_inconclusive_byte_candidate_offsets"
        ),
        "known_field_validations": [
            {
                "name": item.get("name"),
                "range": item.get("range"),
                "records": item.get("records"),
                "match_rows": item.get("match_rows"),
                "module_pointer_like_rows": item.get("module_pointer_like_rows"),
                "validated_for_rejection": item.get("validated_for_rejection"),
            }
            for item in focus_pause_wide_gate.get("known_field_validations") or []
        ],
        "remaining_blocker": focus_pause_wide_gate.get("remaining_blocker"),
    }


def focus_pause_static_gap_gate_summary(focus_pause_static_gap_gate: dict[str, Any] | None) -> dict[str, Any]:
    if not focus_pause_static_gap_gate:
        return {}
    method_scan = focus_pause_static_gap_gate.get("method_scan") or {}
    vtable = focus_pause_static_gap_gate.get("vtable") or {}
    return {
        "ok": (focus_pause_static_gap_gate.get("gate") or {}).get("ok"),
        "backlog_reduced": focus_pause_static_gap_gate.get("backlog_reduced"),
        "vtable_rva": vtable.get("rva"),
        "vtable_entry_count": vtable.get("entry_count"),
        "method_count": method_scan.get("method_count"),
        "methods_with_target_qword_access": method_scan.get("methods_with_target_qword_access"),
        "methods_with_target_gap_direct_access": method_scan.get("methods_with_target_gap_direct_access"),
        "rejected_byte_candidate_offsets": focus_pause_static_gap_gate.get("rejected_byte_candidate_offsets"),
        "remaining_inconclusive_byte_candidate_offsets": focus_pause_static_gap_gate.get(
            "remaining_inconclusive_byte_candidate_offsets"
        ),
        "verdict": focus_pause_static_gap_gate.get("verdict"),
        "reason": focus_pause_static_gap_gate.get("reason"),
    }


def focus_pause_watch_summary(focus_pause_watch: dict[str, Any] | None) -> dict[str, Any]:
    if not focus_pause_watch:
        return {}
    summary = focus_pause_watch.get("summary") or {}
    return {
        "ok": (focus_pause_watch.get("gate") or {}).get("ok"),
        "fixture_status": focus_pause_watch.get("fixture_status"),
        "focus_candidate_window_rejected": focus_pause_watch.get("focus_candidate_window_rejected"),
        "unique_game_hour_count": summary.get("unique_game_hour_count"),
        "sample_count": summary.get("sample_count"),
        "focus_context_rows_max": summary.get("focus_context_rows_max"),
        "focus_keys_with_progress_changes": summary.get("focus_keys_with_progress_changes"),
        "focus_keys_with_candidate_window_changes": summary.get("focus_keys_with_candidate_window_changes"),
        "oracle_contract": focus_pause_watch.get("oracle_contract"),
        "remaining_blocker": focus_pause_watch.get("remaining_blocker"),
    }


def event_hook_gate_summary(event_hook_gate: dict[str, Any] | None) -> dict[str, Any]:
    if not event_hook_gate:
        return {}
    return {
        "ok": (event_hook_gate.get("gate") or {}).get("ok"),
        "backlog_reduced": event_hook_gate.get("backlog_reduced"),
        "rejected_candidate_count": len(event_hook_gate.get("rejected_candidates") or []),
        "possible_static_anchor_count": event_hook_gate.get("possible_static_anchor_count"),
        "rejected_candidates": event_hook_gate.get("rejected_candidates"),
        "remaining_blocker": event_hook_gate.get("remaining_blocker"),
    }


def event_hook_static_contract_gate_summary(
    event_hook_static_contract_gate: dict[str, Any] | None,
) -> dict[str, Any]:
    if not event_hook_static_contract_gate:
        return {}
    return {
        "ok": (event_hook_static_contract_gate.get("gate") or {}).get("ok"),
        "backlog_reduced": event_hook_static_contract_gate.get("backlog_reduced"),
        "candidate_anchor_count": event_hook_static_contract_gate.get("candidate_anchor_count"),
        "rejected_static_anchor_count": event_hook_static_contract_gate.get("rejected_static_anchor_count"),
        "remaining_static_anchor_count": event_hook_static_contract_gate.get("remaining_static_anchor_count"),
        "rejected_static_anchors": [
            {
                "value": (item.get("anchor") or {}).get("value"),
                "rva": (item.get("anchor") or {}).get("rva"),
                "function_begin_rva": (item.get("function") or {}).get("begin_rva"),
                "function_end_rva": (item.get("function") or {}).get("end_rva"),
                "string_ref_count": (item.get("function") or {}).get("string_ref_count"),
                "xref_instruction": (item.get("contract") or {}).get("xref_instruction"),
                "next_call": (item.get("contract") or {}).get("next_call"),
                "verdict": item.get("verdict"),
            }
            for item in event_hook_static_contract_gate.get("rejected_static_anchors") or []
        ],
        "remaining_blocker": event_hook_static_contract_gate.get("remaining_blocker"),
    }


def build_report(
    probe_dir: Path,
    depth_gate_path: Path | None,
    integrated_manifest_path: Path | None,
    focus_pause_gate_path: Path | None = None,
    focus_pause_watch_path: Path | None = None,
    completed_depth_gate_path: Path | None = None,
    completed_integrated_manifest_path: Path | None = None,
    decision_body_gate_path: Path | None = None,
    decision_variant_gate_path: Path | None = None,
    decision_target_save_oracle_gate_path: Path | None = None,
    decision_numeric_target_save_oracle_gate_path: Path | None = None,
    event_hook_gate_path: Path | None = None,
    focus_pause_wide_gate_path: Path | None = None,
    focus_pause_static_gap_gate_path: Path | None = None,
    event_hook_static_contract_gate_path: Path | None = None,
    decision_target_compact_schema_proved: bool = False,
    integrated_replay_verified: bool = False,
    completed_replay_verified: bool = False,
) -> dict[str, Any]:
    root_manifest = load_json(probe_dir / "root_probe_manifest.json")
    samples = load_jsonl(probe_dir / "decision_body_samples.jsonl")
    depth_gate = load_optional_json(depth_gate_path)
    integrated_manifest = load_optional_json(integrated_manifest_path)
    focus_pause_gate = load_optional_json(focus_pause_gate_path)
    focus_pause_watch = load_optional_json(focus_pause_watch_path)
    completed_depth_gate = load_optional_json(completed_depth_gate_path)
    completed_integrated_manifest = load_optional_json(completed_integrated_manifest_path)
    decision_body_gate = load_optional_json(decision_body_gate_path)
    decision_variant_gate = load_optional_json(decision_variant_gate_path)
    decision_target_save_oracle_gate = load_optional_json(
        decision_target_save_oracle_gate_path
    )
    decision_numeric_target_save_oracle_gate = load_optional_json(
        decision_numeric_target_save_oracle_gate_path
    )
    event_hook_gate = load_optional_json(event_hook_gate_path)
    focus_pause_wide_gate = load_optional_json(focus_pause_wide_gate_path)
    focus_pause_static_gap_gate = load_optional_json(focus_pause_static_gap_gate_path)
    event_hook_static_contract_gate = load_optional_json(event_hook_static_contract_gate_path)
    targeted_timed = summarize_targeted_timed(samples)
    focus_summary = root_manifest.get("focus", {})

    exact_rows = int((depth_gate or {}).get("focus_completed_exact_rows") or 0)
    inferred_rows = int((depth_gate or {}).get("focus_completed_inferred_rows") or 0)
    unresolved_rows = int((depth_gate or {}).get("focus_completed_unresolved_rows") or 0)
    compact_country_frame = bool((integrated_manifest or {}).get("compact_country_frame"))
    completed_exact_rows = int(
        (completed_depth_gate or {}).get("focus_completed_exact_rows") or 0
    )
    completed_inferred_rows = int(
        (completed_depth_gate or {}).get("focus_completed_inferred_rows") or 0
    )
    completed_unresolved_rows = int(
        (completed_depth_gate or {}).get("focus_completed_unresolved_rows") or 0
    )
    integrated_flags = production_flags(
        integrated_manifest,
        depth_gate,
        integrated_manifest_path,
        integrated_replay_verified,
    )
    completed_flags = production_flags(
        completed_integrated_manifest,
        completed_depth_gate,
        completed_integrated_manifest_path,
        completed_replay_verified,
    )
    decisions_summary = decision_gate_summary(
        decision_body_gate,
        decision_variant_gate,
        decision_target_save_oracle_gate,
        decision_numeric_target_save_oracle_gate,
    )
    focus_pause_summary = focus_pause_gate_summary(focus_pause_gate)
    focus_pause_wide_summary = focus_pause_wide_gate_summary(focus_pause_wide_gate)
    focus_pause_static_gap_summary = focus_pause_static_gap_gate_summary(focus_pause_static_gap_gate)
    focus_watch_summary = focus_pause_watch_summary(focus_pause_watch)
    event_summary = event_hook_gate_summary(event_hook_gate)
    event_static_contract_summary = event_hook_static_contract_gate_summary(
        event_hook_static_contract_gate
    )

    closures = []
    rejections = []
    blockers = []

    if completed_exact_rows and completed_inferred_rows == 0 and completed_unresolved_rows == 0:
        closures.append(
            {
                "field": "direct_completed_focus_token_closure",
                "status": "validated_slice",
                "evidence": {
                    "focus_completed_exact_rows": completed_exact_rows,
                    "focus_completed_inferred_rows": completed_inferred_rows,
                    "focus_completed_unresolved_rows": completed_unresolved_rows,
                    "depth_gate_manifest": str(completed_depth_gate_path),
                    "integrated_manifest": str(completed_integrated_manifest_path),
                    "production_flags": completed_flags,
                },
                "shared_patch_status": (
                    "requires orchestrator acceptance of the compact focus completed-token "
                    "reader behavior; this lane records the exact patch-plan hunk rather "
                    "than extending shared collector edits"
                ),
            }
        )
    elif exact_rows:
        closures.append(
            {
                "field": "direct_completed_focus_token_closure",
                "status": "validated_slice_existing_integrated_stream",
                "evidence": {
                    "focus_completed_exact_rows": exact_rows,
                    "focus_completed_inferred_rows": inferred_rows,
                    "focus_completed_unresolved_rows": unresolved_rows,
                    "depth_gate_manifest": str(depth_gate_path),
                    "production_flags": integrated_flags,
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

    if decision_target_save_oracle_gate:
        target_gate_ok = bool((decision_target_save_oracle_gate.get("gate") or {}).get("ok"))
        validated_rows = int(
            decision_target_save_oracle_gate.get("validated_live_country_target_rows") or 0
        )
        if target_gate_ok and validated_rows:
            closures.append(
                {
                    "field": "active_targeted_timed_decision_country_target_field",
                    "status": "validated_root",
                    "exact_backlog_item_removed_or_narrowed": (
                        "CDecisionStatus+0xE8 vtable 0x27D5038 record+0x20 "
                        "validated as country target tag_db_id for exact save/live "
                        "matched country-target rows"
                    ),
                    "validated_field": decision_target_save_oracle_gate.get(
                        "validated_field"
                    ),
                    "validated_live_country_target_rows": validated_rows,
                    "validated_decision_count": decision_target_save_oracle_gate.get(
                        "validated_decision_count"
                    ),
                    "unmatched_live_country_target_rows": decision_target_save_oracle_gate.get(
                        "unmatched_live_country_target_rows"
                    ),
                    "unmatched_decision_count": decision_target_save_oracle_gate.get(
                        "unmatched_decision_count"
                    ),
                    "gate_path": str(decision_target_save_oracle_gate_path),
                }
            )

    if decision_numeric_target_save_oracle_gate:
        numeric_gate_ok = bool(
            (decision_numeric_target_save_oracle_gate.get("gate") or {}).get("ok")
        )
        validated_rows = int(
            decision_numeric_target_save_oracle_gate.get(
                "validated_live_numeric_target_rows"
            )
            or 0
        )
        if numeric_gate_ok and validated_rows:
            closures.append(
                {
                    "field": "active_targeted_timed_decision_numeric_target_field",
                    "status": "validated_root",
                    "exact_backlog_item_removed_or_narrowed": (
                        "CDecisionStatus+0xE8 vtable 0x27D5038 record+0x24 "
                        "validated as numeric target id for exact save/live matched "
                        "target=<number> rows"
                    ),
                    "validated_field": decision_numeric_target_save_oracle_gate.get(
                        "validated_field"
                    ),
                    "validated_live_numeric_target_rows": validated_rows,
                    "unmatched_live_numeric_target_rows": decision_numeric_target_save_oracle_gate.get(
                        "unmatched_live_numeric_target_rows"
                    ),
                    "unmatched_save_numeric_target_rows": decision_numeric_target_save_oracle_gate.get(
                        "unmatched_save_numeric_target_rows"
                    ),
                    "validation_basis": decision_numeric_target_save_oracle_gate.get(
                        "validation_basis"
                    ),
                    "gate_path": str(decision_numeric_target_save_oracle_gate_path),
                }
            )

    if decision_target_compact_schema_proved:
        closures.append(
            {
                "field": "active_targeted_timed_decision_target_compact_schema",
                "status": "validated_lane_local_schema_gate",
                "exact_backlog_item_removed_or_narrowed": (
                    "lane-local compact schema, Rust decoder, and gate now accept "
                    "validated active_targeted_timed_decision country and numeric "
                    "target rows with owner tag, target id, decision token, source "
                    "kind, and country/record+0x20 or numeric/record+0x24 flags "
                    "while preserving fail-closed badge kind 2 for unresolved "
                    "cooldown/day/state fields and unmatched transition rows"
                ),
                "evidence": {
                    "proof_command": (
                        "cargo test --manifest-path "
                        "replay_native\\lanes\\focus_research_decisions_depth_live\\Cargo.toml"
                    ),
                    "passing_tests": [
                        "derives_targeted_timed_target_rows_from_extended_decisions_payload",
                        "depth_gate_rejects_bad_decision_country_target_row",
                        "depth_payload_round_trip_gates_transition_rows",
                    ],
                    "compact_header_extension": (
                        "depth payload bytes 0..4 carry decision_target_count; "
                        "bytes 4..8 remain schema flags/reserved zero"
                    ),
                    "compact_row_shape": (
                        "u32 owner_tag_db_id, u32 target_id, u32 flags, "
                        "u32 source_kind, compact string decision"
                    ),
                    "target_flags": (
                        "country rows require country_target+record+0x20 flags; "
                        "numeric rows require numeric_target+record+0x24 flags"
                    ),
                    "shared_patch_status": (
                        "not applied; production capture still requires an orchestrator "
                        "slot to extend the DLL collector and recorder entrypoints"
                    ),
                },
            }
        )

    if decision_body_gate:
        rejected_count = int(decision_body_gate.get("rejected_candidate_count") or 0)
        if rejected_count:
            rejections.append(
                {
                    "field": "decision_target_cooldown_body_fields",
                    "status": "rejected_candidates",
                    "exact_backlog_item_removed_or_narrowed": (
                        "current-build universal/timed-layout body candidates that "
                        "should not be retried for non-timed targeted/cooldown lists"
                    ),
                    "rejected_candidate_count": rejected_count,
                    "gate_path": str(decision_body_gate_path),
                }
            )

    if decision_variant_gate:
        rejected_slices = len(decision_variant_gate.get("rejected_candidate_slices") or [])
        if rejected_slices:
            rejections.append(
                {
                    "field": "active_targeted_timed_decision_definition_local_target_fields",
                    "status": "rejected_candidate_slices",
                    "exact_backlog_item_removed_or_narrowed": (
                        "definition-local country-tag target candidates in the scanned "
                        "record+0x20..+0x7C i32 window for sampled decision tokens"
                    ),
                    "rejected_candidate_slices": rejected_slices,
                    "candidate_slices_not_production": len(
                        decision_variant_gate.get("candidate_slices") or []
                    ),
                    "gate_path": str(decision_variant_gate_path),
                }
            )

    if focus_pause_gate:
        rejected_offsets = focus_pause_gate.get("rejected_byte_candidate_offsets") or []
        if rejected_offsets:
            rejections.append(
                {
                    "field": "focus_paused_state",
                    "status": "rejected_candidates",
                    "exact_backlog_item_removed_or_narrowed": (
                        "canonical paused-boolean byte candidates in CFocusStatus+0x20..+0x37"
                    ),
                    "rejected_candidate_count": len(rejected_offsets),
                    "rejected_offsets": rejected_offsets,
                    "gate_path": str(focus_pause_gate_path),
                }
            )

    if focus_pause_wide_gate:
        wide_rejected_offsets = focus_pause_wide_gate.get("outside_window_rejected_byte_candidate_offsets") or []
        if wide_rejected_offsets:
            rejections.append(
                {
                    "field": "focus_paused_state",
                    "status": "rejected_candidates",
                    "exact_backlog_item_removed_or_narrowed": (
                        "canonical paused-boolean byte candidates outside "
                        "CFocusStatus+0x20..+0x37 in the bounded CFocusStatus+0x00..+0x7F scan"
                    ),
                    "rejected_candidate_count": len(wide_rejected_offsets),
                    "rejected_offsets": wide_rejected_offsets,
                    "inconclusive_offsets": focus_pause_wide_gate.get(
                        "outside_window_inconclusive_byte_candidate_offsets"
                    ),
                    "gate_path": str(focus_pause_wide_gate_path),
                }
            )

    if focus_pause_static_gap_gate:
        static_gap_rejected_offsets = focus_pause_static_gap_gate.get("rejected_byte_candidate_offsets") or []
        if static_gap_rejected_offsets:
            rejections.append(
                {
                    "field": "focus_paused_state",
                    "status": "rejected_candidates",
                    "exact_backlog_item_removed_or_narrowed": (
                        "remaining CFocusStatus+0x3C..+0x3F paused-boolean byte "
                        "candidates rejected by CFocusStatus vtable-method qword-access evidence"
                    ),
                    "rejected_candidate_count": len(static_gap_rejected_offsets),
                    "rejected_offsets": static_gap_rejected_offsets,
                    "gate_path": str(focus_pause_static_gap_gate_path),
                }
            )

    if event_hook_gate:
        rejected_event_candidates = event_hook_gate.get("rejected_candidates") or []
        if rejected_event_candidates:
            rejections.append(
                {
                    "field": "direct_engine_event_hooks",
                    "status": "rejected_candidates",
                    "exact_backlog_item_removed_or_narrowed": (
                        "named/string-anchored focus or decision callback hook routes "
                        "without an exact string/text-xref function anchor"
                    ),
                    "rejected_candidate_count": len(rejected_event_candidates),
                    "possible_static_anchor_count": event_hook_gate.get("possible_static_anchor_count"),
                    "gate_path": str(event_hook_gate_path),
                }
            )

    if event_hook_static_contract_gate:
        rejected_static_anchors = event_hook_static_contract_gate.get("rejected_static_anchors") or []
        if rejected_static_anchors:
            rejections.append(
                {
                    "field": "direct_engine_event_hooks",
                    "status": "rejected_candidates",
                    "exact_backlog_item_removed_or_narrowed": (
                        "remaining static string/xref anchors rejected as "
                        "registration/catalog arguments rather than runtime callbacks"
                    ),
                    "rejected_candidate_count": len(rejected_static_anchors),
                    "remaining_static_anchor_count": event_hook_static_contract_gate.get(
                        "remaining_static_anchor_count"
                    ),
                    "gate_path": str(event_hook_static_contract_gate_path),
                }
            )

    blockers.append(
        {
            "field": "focus_paused_state",
            "status": "BLOCKED",
            "reason": (
                "The scanned CFocusStatus+0x20..+0x37 candidate window is rejected "
                "as the paused-boolean home. The wider CFocusStatus+0x00..+0x7F "
                "scan plus static qword-access gap gate reject every bounded byte "
                "candidate in that object range; no alternate pause root outside "
                "CFocusStatus or paused=true oracle is validated."
            ),
            "evidence": {
                "focus_status_valid_rows": focus_summary.get("focus_status_valid_rows"),
                "active_or_progress_rows": focus_summary.get("active_or_progress_rows"),
                "paused_flag_verdict": focus_summary.get("paused_flag_verdict"),
                "focus_pause_candidate_gate": focus_pause_summary,
                "focus_pause_wide_candidate_gate": focus_pause_wide_summary,
                "focus_pause_static_gap_gate": focus_pause_static_gap_summary,
                "focus_pause_fixture_watch": focus_watch_summary,
            },
            "next_evidence_needed": "same-build paused=true fixture through the watcher oracle contract, or a root outside CFocusStatus proven by static function/argument evidence",
        }
    )

    blockers.append(
        {
            "field": "decision_target_cooldown_body_fields",
            "status": "BLOCKED",
            "reason": (
                "Non-timed targeted/cooldown vtables reject the timed body layout. "
                "Targeted-timed records validate record+0x10 -> CDecisionDef, and "
                "record+0x20 is now validated as the country target tag_db_id for "
                "the exact save/live matched country-target subset. Record+0x24 is "
                "validated as the numeric target id for exact save/live matched "
                "target=<number> rows. Cooldown/day/state fields, unmatched "
                "transition rows, and numeric target semantic classification remain "
                "blocked."
            ),
            "evidence": {
                "targeted_timed_summary": targeted_timed,
                "decision_gates": decisions_summary,
            },
            "next_evidence_needed": "same-frame oracle for cooldown/day/state encodings, exact classification of numeric targets as state/province/body ids, and unmatched transition rows; or a direct live root that exposes those fields",
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
                "event_hook_candidate_gate": event_summary,
                "event_hook_static_contract_gate": event_static_contract_summary,
            },
            "next_evidence_needed": "validated hook/callback root with function address, arguments, and live proof; otherwise keep badge kind 3",
        }
    )

    exact_completed_validated = any(
        item["field"] == "direct_completed_focus_token_closure" for item in closures
    )
    decision_candidate_rejections = sum(
        int(item.get("rejected_candidate_count") or item.get("rejected_candidate_slices") or 0)
        for item in rejections
    )
    gate_ok = bool(root_manifest.get("ok"))
    for gate in [depth_gate, completed_depth_gate]:
        if gate is not None:
            gate_ok = gate_ok and bool(gate.get("ok"))
    for gate in [decision_body_gate, decision_variant_gate]:
        if gate is not None:
            gate_ok = gate_ok and bool((gate.get("gate") or {}).get("ok"))
    if decision_target_save_oracle_gate is not None:
        gate_ok = gate_ok and bool((decision_target_save_oracle_gate.get("gate") or {}).get("ok"))
    if decision_numeric_target_save_oracle_gate is not None:
        gate_ok = gate_ok and bool((decision_numeric_target_save_oracle_gate.get("gate") or {}).get("ok"))
    if focus_pause_gate is not None:
        gate_ok = gate_ok and bool((focus_pause_gate.get("gate") or {}).get("ok"))
    if focus_pause_watch is not None:
        gate_ok = gate_ok and bool((focus_pause_watch.get("gate") or {}).get("ok"))
    if event_hook_gate is not None:
        gate_ok = gate_ok and bool((event_hook_gate.get("gate") or {}).get("ok"))
    if focus_pause_wide_gate is not None:
        gate_ok = gate_ok and bool((focus_pause_wide_gate.get("gate") or {}).get("ok"))
    if focus_pause_static_gap_gate is not None:
        gate_ok = gate_ok and bool((focus_pause_static_gap_gate.get("gate") or {}).get("ok"))
    if event_hook_static_contract_gate is not None:
        gate_ok = gate_ok and bool((event_hook_static_contract_gate.get("gate") or {}).get("ok"))

    return {
        "schema": "focus_research_decisions_depth_live_residual_blocker_gate_v10",
        "lane": "focus_research_decisions_depth_live",
        "created_utc": utc_now(),
        "probe_dir": str(probe_dir),
        "depth_gate_manifest": str(depth_gate_path) if depth_gate_path else None,
        "integrated_manifest": str(integrated_manifest_path) if integrated_manifest_path else None,
        "focus_pause_gate": str(focus_pause_gate_path) if focus_pause_gate_path else None,
        "focus_pause_watch": str(focus_pause_watch_path) if focus_pause_watch_path else None,
        "completed_depth_gate_manifest": str(completed_depth_gate_path)
        if completed_depth_gate_path
        else None,
        "completed_integrated_manifest": str(completed_integrated_manifest_path)
        if completed_integrated_manifest_path
        else None,
        "decision_body_gate": str(decision_body_gate_path) if decision_body_gate_path else None,
        "decision_variant_gate": str(decision_variant_gate_path)
        if decision_variant_gate_path
        else None,
        "decision_target_save_oracle_gate": str(decision_target_save_oracle_gate_path)
        if decision_target_save_oracle_gate_path
        else None,
        "decision_numeric_target_save_oracle_gate": str(decision_numeric_target_save_oracle_gate_path)
        if decision_numeric_target_save_oracle_gate_path
        else None,
        "event_hook_gate": str(event_hook_gate_path) if event_hook_gate_path else None,
        "focus_pause_wide_gate": str(focus_pause_wide_gate_path)
        if focus_pause_wide_gate_path
        else None,
        "focus_pause_static_gap_gate": str(focus_pause_static_gap_gate_path)
        if focus_pause_static_gap_gate_path
        else None,
        "event_hook_static_contract_gate": str(event_hook_static_contract_gate_path)
        if event_hook_static_contract_gate_path
        else None,
        "decision_target_compact_schema_proved": decision_target_compact_schema_proved,
        "build_gate": root_manifest.get("build_gate"),
        "completion_status": "NOT COMPLETE",
        "whole_residual_slice_complete": False,
        "backlog_reduced": bool(closures or rejections),
        "production_flags": {
            "baseline_integrated_stream": integrated_flags,
            "exact_completed_stream": completed_flags,
        },
        "acceptance": {
            "direct_completed_focus_token_closure": (
                "validated_slice" if exact_completed_validated else "blocked"
            ),
            "focus_paused_state": "blocked",
            "decision_target_cooldown_body_fields": "blocked",
            "direct_engine_event_hooks": "blocked",
            "all_required_production_flags_met_for_exact_completed_slice": all_production_flags_met(
                completed_flags
            ),
            "whole_slice_complete": False,
        },
        "validated_slices": closures,
        "rejected_candidates": rejections,
        "remaining_blockers": blockers,
        "gate": {
            "ok": gate_ok,
            "errors": [] if gate_ok else ["one or more proof inputs failed"],
            "decision_candidate_rejections": decision_candidate_rejections,
        },
    }


def write_markdown(path: Path, report: dict[str, Any]) -> None:
    lines = [
        "# Residual Blocker Gate",
        "",
        f"Created UTC: `{report['created_utc']}`",
        f"Completion status: `{report['completion_status']}`",
        f"Whole residual slice complete: `{str(report['whole_residual_slice_complete']).lower()}`",
        f"Backlog reduced: `{str(report['backlog_reduced']).lower()}`",
        "",
        "## Acceptance",
        "",
        f"- Direct completed-focus token closure: `{report['acceptance']['direct_completed_focus_token_closure']}`.",
        f"- Focus paused state: `{report['acceptance']['focus_paused_state']}`.",
        f"- Decision target/cooldown/body fields: `{report['acceptance']['decision_target_cooldown_body_fields']}`.",
        f"- Direct engine event hooks: `{report['acceptance']['direct_engine_event_hooks']}`.",
        f"- Exact-completed production flags all met: `{str(report['acceptance']['all_required_production_flags_met_for_exact_completed_slice']).lower()}`.",
        "",
        "## Validated Slices",
        "",
    ]
    for item in report["validated_slices"]:
        lines.append(f"- `{item['field']}`: {item['status']}.")
    if not report["validated_slices"]:
        lines.append("- None.")
    lines.extend(["", "## Rejected Candidates", ""])
    for item in report["rejected_candidates"]:
        if item.get("rejected_candidate_count") is not None:
            lines.append(
                f"- `{item['field']}`: rejected `{item['rejected_candidate_count']}` current-build candidates."
            )
        elif item.get("rejected_candidate_slices") is not None:
            lines.append(
                f"- `{item['field']}`: rejected `{item['rejected_candidate_slices']}` definition-local slices."
            )
    if not report["rejected_candidates"]:
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
    parser.add_argument("--focus-pause-gate", type=Path, default=None)
    parser.add_argument("--focus-pause-watch", type=Path, default=None)
    parser.add_argument("--completed-depth-gate", type=Path, default=None)
    parser.add_argument("--completed-integrated-manifest", type=Path, default=None)
    parser.add_argument("--decision-body-gate", type=Path, default=None)
    parser.add_argument("--decision-variant-gate", type=Path, default=None)
    parser.add_argument("--decision-target-save-oracle-gate", type=Path, default=None)
    parser.add_argument("--decision-numeric-target-save-oracle-gate", type=Path, default=None)
    parser.add_argument("--event-hook-gate", type=Path, default=None)
    parser.add_argument("--focus-pause-wide-gate", type=Path, default=None)
    parser.add_argument("--focus-pause-static-gap-gate", type=Path, default=None)
    parser.add_argument("--event-hook-static-contract-gate", type=Path, default=None)
    parser.add_argument("--decision-target-compact-schema-proved", action="store_true")
    parser.add_argument("--integrated-replay-verified", action="store_true")
    parser.add_argument("--completed-replay-verified", action="store_true")
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()

    report = build_report(
        args.probe_dir,
        args.depth_gate,
        args.integrated_manifest,
        args.focus_pause_gate,
        args.focus_pause_watch,
        args.completed_depth_gate,
        args.completed_integrated_manifest,
        args.decision_body_gate,
        args.decision_variant_gate,
        args.decision_target_save_oracle_gate,
        args.decision_numeric_target_save_oracle_gate,
        args.event_hook_gate,
        args.focus_pause_wide_gate,
        args.focus_pause_static_gap_gate,
        args.event_hook_static_contract_gate,
        args.decision_target_compact_schema_proved,
        args.integrated_replay_verified,
        args.completed_replay_verified,
    )
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
                "rejected_candidates": len(report["rejected_candidates"]),
                "remaining_blockers": len(report["remaining_blockers"]),
                "out_dir": str(args.out_dir),
            },
            indent=2,
        )
    )
    return 0 if report["gate"]["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(run())
