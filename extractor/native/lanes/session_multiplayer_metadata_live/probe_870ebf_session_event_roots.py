#!/usr/bin/env python3
"""Bounded root-discovery probe for domain 57 session event sub-scopes.

The probe is deliberately exploratory and fail-closed. It discovers current
RTTI candidates, scans for live heap objects by vtable, samples text-like fields,
and tries to find std::vector-like owners for object pointer arrays. A successful
result is still a root candidate until the orchestrator repeats it with the
fixture-specific oracle described in PROBE_RUNSHEET.md.
"""

from __future__ import annotations

import argparse
import json
import time
from pathlib import Path
from typing import Any

import session_probe_common as common


SCOPES: dict[str, dict[str, Any]] = {
    "hotjoin_tag_switch": {
        "contract_row": "Hotjoin/tag switches",
        "event_kinds": ["hotjoin", "tag_switch"],
        "source_flag": "SOURCE_FLAG_COMMAND_HOOK_VALIDATED",
        "keywords": ["lobby", "hotjoin", "player", "session", "multiplayer"],
        "sp_fixture_expectation": (
            "Single-player may have lobby/session manager classes but should not "
            "prove join/leave/tag-switch event rows. MP fixture must create a "
            "join/leave or tag-switch and show monotonic root changes."
        ),
    },
    "chat_system_messages": {
        "contract_row": "Chat/messages",
        "event_kinds": ["chat", "system_message"],
        "source_flag": "SOURCE_FLAG_MESSAGE_HOOK_VALIDATED",
        "keywords": ["chat", "message", "player", "lobby", "system"],
        "sp_fixture_expectation": (
            "Single-player can only validate a pollable message-log container "
            "shape if one exists. Text/event semantics require a later MP fixture "
            "that sends chat and observes system join/leave text."
        ),
    },
    "alerts_notifications": {
        "contract_row": "Alerts/notifications",
        "event_kinds": ["alert", "notification"],
        "source_flag": "SOURCE_FLAG_NOTIFICATION_HOOK_VALIDATED",
        "keywords": ["alert", "notification", "notify", "popup", "important", "message"],
        "sp_fixture_expectation": (
            "The current wartime single-player save has active alerts, so a "
            "pollable alert/notification root must show live stable objects or a "
            "stable container now. Otherwise this sub-scope remains root-open."
        ),
    },
}


def class_rows_for_scope(
    p: common.winmem.Process, chunks: list[tuple[int, bytes]], scope: str
) -> list[dict[str, Any]]:
    keywords = SCOPES[scope]["keywords"]
    found = common.discover_decorated_rtti_names(chunks, keywords)
    rows: list[dict[str, Any]] = []
    for entry in found:
        low = entry["decorated_name"].lower()
        if not any(keyword in low for keyword in keywords):
            continue
        decorated = entry["decorated_name"].encode("ascii", "replace")
        derived = common.derive_rtti_vtables(p, chunks, decorated)
        if not derived["vtables"]:
            continue
        rows.append(
            {
                "decorated_name": entry["decorated_name"],
                "keyword_hits": entry["keyword_hits"],
                "string_matches": [common.hx(value) for value in entry["string_matches"]],
                "vtables": [
                    {"addr": common.hx(value), "rva": common.hx(value - p.module_base)}
                    for value in derived["vtables"]
                ],
                "vtable_values": derived["vtables"],
            }
        )
    return rows


def sample_heap_objects(
    p: common.winmem.Process,
    heap_scan: dict[str, Any],
    *,
    max_objects: int,
    warm_delay_ms: int,
) -> list[dict[str, Any]]:
    samples: list[dict[str, Any]] = []
    addresses = [value for value in heap_scan.get("hit_addresses", []) if isinstance(value, int)]
    for object_ptr in addresses[:max_objects]:
        vt1 = common.u64(p, object_ptr)
        strings1 = common.sample_object_strings(p, object_ptr)
        if warm_delay_ms > 0:
            time.sleep(warm_delay_ms / 1000.0)
        vt2 = common.u64(p, object_ptr)
        strings2 = common.sample_object_strings(p, object_ptr)
        samples.append(
            {
                "object_ptr": common.hx(object_ptr),
                "vtable": common.hx(vt1),
                "warm_vtable_stable": vt1 == vt2 and vt1 is not None,
                "string_fields": strings1,
                "warm_string_fields_stable": strings1 == strings2,
            }
        )
    return samples


def discover_container_candidates(
    p: common.winmem.Process,
    object_samples: list[dict[str, Any]],
    *,
    max_ref_targets: int,
    max_refs: int,
    ref_scan_seconds: float,
    owner_scan_seconds: float,
) -> list[dict[str, Any]]:
    targets: list[int] = []
    for row in object_samples:
        ptr = row.get("object_ptr")
        if isinstance(ptr, str):
            try:
                targets.append(int(ptr, 16))
            except ValueError:
                pass
        if len(targets) >= max_ref_targets:
            break
    if not targets:
        return []

    refs = common.find_aligned_qword_refs(
        p, targets, max_refs=max_refs, max_seconds=ref_scan_seconds
    )
    array_candidates: dict[int, dict[str, Any]] = {}
    for target, target_refs in refs.items():
        for ref in target_refs[:16]:
            for back in range(17):
                begin = ref - back * 8
                if begin <= 0:
                    continue
                array_candidates.setdefault(
                    begin,
                    {
                        "begin": begin,
                        "sample_object_ptr": target,
                        "contains_slot": ref,
                    },
                )

    # Keep the second heap pass bounded. These are candidates, not proof.
    candidate_begins = list(array_candidates)[:128]
    begin_refs = common.find_aligned_qword_refs(
        p,
        candidate_begins,
        max_refs=max_refs,
        max_seconds=owner_scan_seconds,
    )

    owners: list[dict[str, Any]] = []
    for begin, owner_refs in begin_refs.items():
        candidate = array_candidates.get(begin)
        if candidate is None:
            continue
        for owner in owner_refs[:16]:
            if owner == begin:
                continue
            end = common.u64(p, owner + 8)
            cap = common.u64(p, owner + 0x10)
            if end is None or cap is None:
                continue
            if not (begin < end <= cap and (end - begin) % 8 == 0):
                continue
            count = (end - begin) // 8
            if count <= 0 or count > 512:
                continue
            contains_slot = candidate["contains_slot"]
            if not (begin <= contains_slot < end):
                continue
            owners.append(
                {
                    "begin_field": common.hx(owner),
                    "begin": common.hx(begin),
                    "end": common.hx(end),
                    "cap": common.hx(cap),
                    "count": count,
                    "contains_slot": common.hx(contains_slot),
                    "sample_object_ptr": common.hx(candidate["sample_object_ptr"]),
                }
            )
    dedup: dict[tuple[str | None, str | None], dict[str, Any]] = {}
    for owner in owners:
        dedup[(owner.get("begin_field"), owner.get("begin"))] = owner
    return list(dedup.values())[:64]


def status_for_scope(
    scope: str,
    classes: list[dict[str, Any]],
    heap_scan: dict[str, Any],
    object_samples: list[dict[str, Any]],
    containers: list[dict[str, Any]],
    text_oracle: dict[str, Any] | None,
) -> dict[str, Any]:
    has_text = any(row.get("string_fields") for row in object_samples)
    stable_objects = any(row.get("warm_vtable_stable") for row in object_samples)
    expected_text_observed = bool(text_oracle and text_oracle.get("hit_count", 0) > 0)
    if containers:
        status = "pollable_container_candidate"
        decision = (
            "Candidate vector-like owner found. Orchestrator should rerun with "
            "fixture action and verify count/text delta before accepting."
        )
    elif heap_scan.get("hit_count", 0) > 0 and stable_objects:
        status = "live_heap_objects_observed_root_open"
        decision = (
            "Live RTTI objects were observed but no pollable owner/root was "
            "found. This is not production-ready without a container/root."
        )
    elif classes:
        status = "rtti_only_no_live_objects"
        decision = (
            "Current-build class/vtable candidates exist, but this fixture did "
            "not expose live heap instances. Treat as fixture-blocked or rejected "
            "only if the scope should be active in this fixture."
        )
    else:
        status = "no_current_rtti_candidate"
        decision = "No RTTI class candidate found by keyword scan; broaden search before rejecting."

    if scope == "alerts_notifications" and status == "rtti_only_no_live_objects":
        decision = (
            "SP wartime fixture was expected to have active alerts; if the "
            "orchestrator confirms active UI alerts at run time, this is a "
            "negative result for RTTI-object strategy, not a rejection of the "
            "scope."
        )
    if text_oracle is not None and not expected_text_observed:
        decision += " Expected text oracle was not found in heap."

    return {
        "status": status,
        "has_text_fields": has_text,
        "stable_object_samples": stable_objects,
        "expected_text_observed": expected_text_observed,
        "decision": decision,
    }


def probe_scope(
    p: common.winmem.Process,
    chunks: list[tuple[int, bytes]],
    scope: str,
    args: argparse.Namespace,
) -> dict[str, Any]:
    started = time.time()
    classes = class_rows_for_scope(p, chunks, scope)
    vtable_to_class: dict[int, str] = {}
    for row in classes:
        for value in row.get("vtable_values", []):
            if isinstance(value, int):
                vtable_to_class[value] = row["decorated_name"]

    heap_scan = common.scan_heap_vtable_hits(
        p,
        vtable_to_class,
        max_hits=max(1, args.max_heap_hits_per_scope),
        max_seconds=max(1.0, args.heap_scan_seconds_per_scope),
    )
    object_samples = sample_heap_objects(
        p,
        heap_scan,
        max_objects=max(0, args.max_object_samples),
        warm_delay_ms=max(0, args.warm_delay_ms),
    )
    containers: list[dict[str, Any]] = []
    if not args.skip_reference_scan:
        containers = discover_container_candidates(
            p,
            object_samples,
            max_ref_targets=max(1, args.max_ref_targets),
            max_refs=max(1, args.max_refs_per_scope),
            ref_scan_seconds=max(1.0, args.ref_scan_seconds_per_scope),
            owner_scan_seconds=max(1.0, args.owner_scan_seconds),
        )

    text_oracle = None
    if args.expect_text:
        text_oracle = common.scan_heap_text_needles(
            p,
            [args.expect_text],
            max_hits=max(1, args.max_text_hits),
            max_seconds=max(1.0, args.text_scan_seconds_per_scope),
        )

    class_report = [
        {key: value for key, value in row.items() if key != "vtable_values"}
        for row in classes
    ]
    scope_status = status_for_scope(
        scope, classes, heap_scan, object_samples, containers, text_oracle
    )
    return {
        "contract_row": SCOPES[scope]["contract_row"],
        "event_kinds": SCOPES[scope]["event_kinds"],
        "source_flag": SCOPES[scope]["source_flag"],
        "sp_fixture_expectation": SCOPES[scope]["sp_fixture_expectation"],
        "rtti_classes": class_report,
        "heap_vtable_scan": {key: value for key, value in heap_scan.items() if key != "hit_addresses"},
        "object_samples": object_samples,
        "container_candidates": containers,
        "text_oracle": text_oracle or {
            "status": "skipped",
            "reason": "pass --expect-text for MP chat/system-message fixture validation",
        },
        "summary": scope_status,
        "elapsed_seconds": round(time.time() - started, 3),
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument(
        "--scope",
        choices=["all", *SCOPES.keys()],
        default="all",
        help="sub-scope to probe",
    )
    ap.add_argument("--heap-scan-seconds-per-scope", type=float, default=45.0)
    ap.add_argument("--max-heap-hits-per-scope", type=int, default=384)
    ap.add_argument("--max-object-samples", type=int, default=24)
    ap.add_argument("--warm-delay-ms", type=int, default=250)
    ap.add_argument("--skip-reference-scan", action="store_true")
    ap.add_argument("--max-ref-targets", type=int, default=12)
    ap.add_argument("--max-refs-per-scope", type=int, default=768)
    ap.add_argument("--ref-scan-seconds-per-scope", type=float, default=45.0)
    ap.add_argument("--owner-scan-seconds", type=float, default=8.0)
    ap.add_argument(
        "--expect-text",
        help="unique fixture text to search for in heap, e.g. a sent MP chat string",
    )
    ap.add_argument("--text-scan-seconds-per-scope", type=float, default=30.0)
    ap.add_argument("--max-text-hits", type=int, default=64)
    args = ap.parse_args()

    args.out.parent.mkdir(parents=True, exist_ok=True)
    p = common.winmem.open_process(common.PROC_NAME)
    if p is None:
        raise SystemExit("hoi4.exe not running")

    with p:
        started = time.time()
        process = common.process_info(p)
        report: dict[str, Any] = {
            "schema": "battle-plan.session57.event_root_discovery.v1",
            "lane": "session_multiplayer_metadata_live",
            "domain_id": 57,
            "probe": "probe_870ebf_session_event_roots",
            "expected_sha256": common.EXPECTED_SHA256,
            "process": process,
            "build_gate_ok": process.get("sha256") == common.EXPECTED_SHA256,
            "args": {
                "scope": args.scope,
                "heap_scan_seconds_per_scope": args.heap_scan_seconds_per_scope,
                "max_heap_hits_per_scope": args.max_heap_hits_per_scope,
                "max_object_samples": args.max_object_samples,
                "warm_delay_ms": args.warm_delay_ms,
                "skip_reference_scan": args.skip_reference_scan,
                "expect_text": args.expect_text,
            },
        }
        if not report["build_gate_ok"]:
            report["status"] = "build_gate_failed"
            args.out.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
            return 2

        report["session_context"] = common.read_session_context(p)
        if not report["session_context"].get("ok"):
            report["status"] = "session_context_gate_failed"
            args.out.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
            return 3

        chunks = common.read_module_chunks(p)
        report["module_scan"] = {
            "chunk_count": len(chunks),
            "bytes_read": sum(len(blob) for _, blob in chunks),
        }
        selected_scopes = list(SCOPES) if args.scope == "all" else [args.scope]
        report["scopes"] = {
            scope: probe_scope(p, chunks, scope, args) for scope in selected_scopes
        }
        statuses = {
            scope: row.get("summary", {}).get("status")
            for scope, row in report["scopes"].items()
        }
        report["summary"] = {
            "statuses": statuses,
            "pollable_container_candidates": [
                scope for scope, status in statuses.items() if status == "pollable_container_candidate"
            ],
            "live_objects_without_root": [
                scope for scope, status in statuses.items() if status == "live_heap_objects_observed_root_open"
            ],
            "fixture_blocked_or_rtti_only": [
                scope for scope, status in statuses.items() if status == "rtti_only_no_live_objects"
            ],
        }
        report["status"] = (
            "pollable_candidate_found"
            if report["summary"]["pollable_container_candidates"]
            else "no_pollable_candidate_found"
        )
        report["elapsed_seconds"] = round(time.time() - started, 3)
        args.out.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
        print(f"probe_870ebf_session_event_roots: status={report['status']} out={args.out}")
        return 0 if report["status"] == "pollable_candidate_found" else 2


if __name__ == "__main__":
    raise SystemExit(main())
