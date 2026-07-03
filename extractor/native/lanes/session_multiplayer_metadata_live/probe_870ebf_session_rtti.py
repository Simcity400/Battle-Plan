#!/usr/bin/env python3
"""Discover current-build RTTI candidates for domain 57 session event roots.

This probe is read-only and lane-local. It scans the live module for MSVC RTTI
type names related to chat/messages/alerts/lobby/session events, derives current
vtable RVAs, and optionally counts live heap objects with those vtables.
"""

from __future__ import annotations

import argparse
import json
import time
from pathlib import Path
from typing import Any

import session_probe_common as common


DEFAULT_KEYWORDS = [
    "chat",
    "message",
    "notification",
    "notify",
    "alert",
    "popup",
    "important",
    "lobby",
    "hotjoin",
    "session",
    "multiplayer",
    "player",
]

SCOPE_KEYWORDS = {
    "hotjoin_tag_switch": ["lobby", "hotjoin", "player", "session", "multiplayer"],
    "chat_system_messages": ["chat", "message", "player", "lobby"],
    "alerts_notifications": ["alert", "notification", "notify", "popup", "important", "message"],
}


def classify_scope(decorated_name: str, keyword_hits: list[str]) -> list[str]:
    low = decorated_name.lower()
    scopes: list[str] = []
    for scope, keywords in SCOPE_KEYWORDS.items():
        if any(keyword in low for keyword in keywords) or any(
            keyword in keyword_hits for keyword in keywords
        ):
            scopes.append(scope)
    return scopes or ["unclassified_candidate"]


def build_class_report(
    p: common.winmem.Process, chunks: list[tuple[int, bytes]], keywords: list[str]
) -> list[dict[str, Any]]:
    found = common.discover_decorated_rtti_names(chunks, keywords)
    rows: list[dict[str, Any]] = []
    for entry in found:
        decorated = entry["decorated_name"].encode("ascii", "replace")
        derived = common.derive_rtti_vtables(p, chunks, decorated)
        rows.append(
            {
                "decorated_name": entry["decorated_name"],
                "keyword_hits": entry["keyword_hits"],
                "scopes": classify_scope(entry["decorated_name"], entry["keyword_hits"]),
                "string_matches": [common.hx(value) for value in entry["string_matches"]],
                "complete_object_locators": [
                    {
                        "string_addr": common.hx(hit["string_addr"]),
                        "type_descriptor": common.hx(hit["type_descriptor"]),
                        "complete_object_locator": common.hx(hit["complete_object_locator"]),
                        "class_descriptor_rva": common.hx(hit["class_descriptor_rva"]),
                        "offset": hit["offset"],
                        "cd_offset": hit["cd_offset"],
                    }
                    for hit in derived["complete_object_locators"]
                ],
                "vtables": [
                    {"addr": common.hx(value), "rva": common.hx(value - p.module_base)}
                    for value in derived["vtables"]
                ],
                "vtable_values": derived["vtables"],
            }
        )
    return rows


def summarize(classes: list[dict[str, Any]], heap_scan: dict[str, Any] | None) -> dict[str, Any]:
    scope_counts: dict[str, int] = {}
    vtable_count = 0
    for row in classes:
        if row.get("vtables"):
            vtable_count += len(row["vtables"])
        for scope in row.get("scopes", []):
            scope_counts[scope] = scope_counts.get(scope, 0) + 1

    summary: dict[str, Any] = {
        "candidate_class_count": len(classes),
        "candidate_vtable_count": vtable_count,
        "candidate_classes_by_scope": scope_counts,
        "status": "rtti_candidates_found" if classes else "no_rtti_candidates_found",
    }
    if heap_scan is not None:
        summary["heap_vtable_hit_count"] = heap_scan.get("hit_count", 0)
        summary["heap_scan_status"] = heap_scan.get("status")
        summary["heap_scan_timed_out"] = heap_scan.get("timed_out")
    return summary


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument(
        "--keywords",
        nargs="*",
        default=DEFAULT_KEYWORDS,
        help="case-insensitive RTTI keyword filters",
    )
    ap.add_argument("--heap-scan", action="store_true", help="count heap vtable hits")
    ap.add_argument("--heap-scan-seconds", type=float, default=45.0)
    ap.add_argument("--max-heap-hits", type=int, default=512)
    args = ap.parse_args()

    args.out.parent.mkdir(parents=True, exist_ok=True)
    p = common.winmem.open_process(common.PROC_NAME)
    if p is None:
        raise SystemExit("hoi4.exe not running")

    with p:
        started = time.time()
        process = common.process_info(p)
        report: dict[str, Any] = {
            "schema": "battle-plan.session57.rtti_discovery.v1",
            "lane": "session_multiplayer_metadata_live",
            "domain_id": 57,
            "probe": "probe_870ebf_session_rtti",
            "expected_sha256": common.EXPECTED_SHA256,
            "process": process,
            "build_gate_ok": process.get("sha256") == common.EXPECTED_SHA256,
            "keywords": args.keywords,
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
        classes = build_class_report(p, chunks, args.keywords)
        report["classes"] = [
            {key: value for key, value in row.items() if key != "vtable_values"}
            for row in classes
        ]

        heap_scan = None
        if args.heap_scan:
            vtable_to_class: dict[int, str] = {}
            for row in classes:
                for value in row.get("vtable_values", []):
                    if isinstance(value, int):
                        vtable_to_class[value] = row["decorated_name"]
            heap_scan = common.scan_heap_vtable_hits(
                p,
                vtable_to_class,
                max_hits=max(1, args.max_heap_hits),
                max_seconds=max(1.0, args.heap_scan_seconds),
            )
            report["heap_vtable_scan"] = {
                key: value for key, value in heap_scan.items() if key != "hit_addresses"
            }
        else:
            report["heap_vtable_scan"] = {
                "status": "skipped",
                "reason": "pass --heap-scan to count current live objects",
            }

        report["summary"] = summarize(classes, heap_scan)
        report["status"] = report["summary"]["status"]
        report["elapsed_seconds"] = round(time.time() - started, 3)
        args.out.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
        print(f"probe_870ebf_session_rtti: status={report['status']} out={args.out}")
        return 0 if classes else 2


if __name__ == "__main__":
    raise SystemExit(main())
