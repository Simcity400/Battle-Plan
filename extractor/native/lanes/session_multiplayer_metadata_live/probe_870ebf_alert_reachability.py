#!/usr/bin/env python3
"""Reachability probe for active alert/notification roots.

This is the follow-up to the broad RTTI heap scan that found zero live heap
instances. Instead of guessing class names and scanning the whole heap for object
starts, this probe starts from already validated domain-57 roots:

- CGameState singleton at u64[module+0x3316E90]
- current human CCountry handles from CGameState player-country arrays

It then walks direct root members and one-level child objects, looking for
std::vector-like {begin,end,cap} members whose elements point at objects with a
current-build RTTI name containing one of:
`essage`, `lert`, `otification`, `vent`.

All reads are ReadProcessMemory only. The output is candidate evidence, not a
production root acceptance by itself.
"""

from __future__ import annotations

import argparse
import json
import time
from pathlib import Path
from typing import Any

import session_probe_common as common


RTTI_SUBSTRINGS = ["essage", "lert", "otification", "vent"]

GS_HUMAN_STATUS_ARRAY = 0x388
GS_SLOT_TAG_ARRAY = 0x358
GS_COUNTRY_COUNT = 0x394
GS_HANDLE_ARRAY = 0x310
COUNTRY_TAG_RECORD_STRIDE = 0x20
MAX_COUNTRY_COUNT = 4096


def int_auto(value: str) -> int:
    return int(value, 0)


def read_tag(p: common.winmem.Process, addr: int) -> str | None:
    raw = common.read_exact(p, addr, 4)
    if raw is None:
        return None
    raw = raw.split(b"\x00", 1)[0]
    try:
        value = raw.decode("ascii")
    except UnicodeDecodeError:
        return None
    if len(value) != 3:
        return None
    return value


def build_rtti_map(
    p: common.winmem.Process, chunks: list[tuple[int, bytes]]
) -> tuple[dict[int, str], list[dict[str, Any]]]:
    found = common.discover_decorated_rtti_names(chunks, RTTI_SUBSTRINGS)
    vtable_to_name: dict[int, str] = {}
    rows: list[dict[str, Any]] = []
    for entry in found:
        decorated = entry["decorated_name"].encode("ascii", "replace")
        derived = common.derive_rtti_vtables(p, chunks, decorated)
        for value in derived["vtables"]:
            vtable_to_name[value] = entry["decorated_name"]
        rows.append(
            {
                "decorated_name": entry["decorated_name"],
                "keyword_hits": entry["keyword_hits"],
                "string_matches": [common.hx(value) for value in entry["string_matches"]],
                "vtables": [
                    {"addr": common.hx(value), "rva": common.hx(value - p.module_base)}
                    for value in derived["vtables"]
                ],
            }
        )
    return vtable_to_name, rows


def read_human_country_roots(p: common.winmem.Process, game_state: int) -> dict[str, Any]:
    result: dict[str, Any] = {
        "country_count": None,
        "status_ptr": None,
        "handle_ptr": None,
        "slot_base": None,
        "human_countries": [],
        "status": "unreadable",
    }
    count = common.i32(p, game_state + GS_COUNTRY_COUNT)
    result["country_count"] = count
    if count is None or not (0 < count < MAX_COUNTRY_COUNT):
        result["status"] = "bad_country_count"
        return result

    status_ptr = common.u64(p, game_state + GS_HUMAN_STATUS_ARRAY)
    handle_ptr = common.u64(p, game_state + GS_HANDLE_ARRAY)
    slot_base = common.u64(p, game_state + GS_SLOT_TAG_ARRAY)
    result.update(
        {
            "status_ptr": common.hx(status_ptr),
            "handle_ptr": common.hx(handle_ptr),
            "slot_base": common.hx(slot_base),
        }
    )
    if not (common.is_heap(status_ptr) and common.is_heap(handle_ptr, 8) and common.is_heap(slot_base, 8)):
        result["status"] = "bad_player_country_roots"
        return result

    status_bytes = p.try_read(status_ptr, count)
    if len(status_bytes) != count:
        result["status"] = "status_array_short_read"
        return result

    rows: list[dict[str, Any]] = []
    for gs_index, marker in enumerate(status_bytes):
        if marker == 0:
            continue
        handle = common.u64(p, handle_ptr + gs_index * 8)
        if not common.is_heap(handle, 8):
            continue
        slot_index = common.i32(p, handle + 0x08)
        if slot_index is None or not (0 <= slot_index < count):
            continue
        tag = read_tag(p, slot_base + slot_index * COUNTRY_TAG_RECORD_STRIDE)
        name = common.read_msvc_string_at(p, handle + 0x10)
        rows.append(
            {
                "gs_index": gs_index,
                "human_marker": marker,
                "country_ptr": common.hx(handle),
                "slot_index": slot_index,
                "tag": tag,
                "name": name,
            }
        )
    result["human_countries"] = rows
    result["status"] = "ok" if rows else "no_human_country_rows"
    return result


def vector_triple_at(
    p: common.winmem.Process,
    owner: int,
    offset: int,
    *,
    max_vector_count: int,
    max_capacity_count: int,
) -> dict[str, Any] | None:
    begin = common.u64(p, owner + offset)
    end = common.u64(p, owner + offset + 8)
    cap = common.u64(p, owner + offset + 0x10)
    if not (common.is_heap(begin, 8) and common.is_heap(end, 8) and common.is_heap(cap, 8)):
        return None
    if not (begin <= end <= cap):
        return None
    span = end - begin
    cap_span = cap - begin
    if span % 8 != 0 or cap_span % 8 != 0:
        return None
    count = span // 8
    cap_count = cap_span // 8
    if count <= 0 or count > max_vector_count or cap_count < count or cap_count > max_capacity_count:
        return None
    return {
        "member_offset": offset,
        "begin": begin,
        "end": end,
        "cap": cap,
        "count": count,
        "capacity": cap_count,
    }


def classify_vector_elements(
    p: common.winmem.Process,
    vector: dict[str, Any],
    vtable_to_name: dict[int, str],
    *,
    sample_elements: int,
) -> dict[str, Any] | None:
    begin = int(vector["begin"])
    count = int(vector["count"])
    samples: list[dict[str, Any]] = []
    match_count = 0
    first_match = None
    first_element_kind = "unreadable"

    for idx in range(min(count, sample_elements)):
        slot = begin + idx * 8
        ptr = common.u64(p, slot)
        row: dict[str, Any] = {"index": idx, "slot": common.hx(slot), "ptr": common.hx(ptr)}
        if common.is_heap(ptr, 8):
            vt = common.u64(p, ptr)
            class_name = vtable_to_name.get(vt or 0)
            row.update(
                {
                    "interpretation": "pointer_element",
                    "vtable": common.hx(vt),
                    "vtable_rva": common.hx((vt or 0) - p.module_base) if vt else None,
                    "rtti_name": class_name,
                }
            )
            if class_name:
                match_count += 1
                row["string_fields"] = common.sample_object_strings(p, ptr, max_offset=0x180)
                if first_match is None:
                    first_match = row
            if idx == 0:
                first_element_kind = "pointer_match" if class_name else "pointer_no_match"
        else:
            inline_vt = common.u64(p, slot)
            inline_name = vtable_to_name.get(inline_vt or 0)
            row.update(
                {
                    "interpretation": "inline_or_scalar",
                    "inline_vtable": common.hx(inline_vt),
                    "inline_vtable_rva": common.hx((inline_vt or 0) - p.module_base) if inline_vt else None,
                    "inline_rtti_name": inline_name,
                }
            )
            if inline_name:
                match_count += 1
                if first_match is None:
                    first_match = row
            if idx == 0:
                first_element_kind = "inline_match" if inline_name else "no_heap_pointer"
        samples.append(row)

    if match_count == 0:
        return None
    return {
        "first_element_kind": first_element_kind,
        "first_element_matched": first_element_kind in ("pointer_match", "inline_match"),
        "matching_sample_count": match_count,
        "sampled_elements": len(samples),
        "first_match": first_match,
        "samples": samples,
    }


def warm_validate_vector(
    p: common.winmem.Process,
    owner: int,
    offset: int,
    original: dict[str, Any],
    *,
    delay_ms: int,
    max_vector_count: int,
    max_capacity_count: int,
) -> dict[str, Any]:
    if delay_ms > 0:
        time.sleep(delay_ms / 1000.0)
    reread = vector_triple_at(
        p,
        owner,
        offset,
        max_vector_count=max_vector_count,
        max_capacity_count=max_capacity_count,
    )
    if reread is None:
        return {"stable": False, "reread": None}
    stable = (
        reread["begin"] == original["begin"]
        and reread["end"] == original["end"]
        and reread["cap"] == original["cap"]
        and reread["count"] == original["count"]
    )
    return {
        "stable": stable,
        "reread": {
            "begin": common.hx(reread["begin"]),
            "end": common.hx(reread["end"]),
            "cap": common.hx(reread["cap"]),
            "count": reread["count"],
            "capacity": reread["capacity"],
        },
    }


def scan_object_for_vectors(
    p: common.winmem.Process,
    root_path: str,
    owner: int,
    vtable_to_name: dict[int, str],
    args: argparse.Namespace,
) -> list[dict[str, Any]]:
    candidates: list[dict[str, Any]] = []
    for offset in range(0, max(0, args.scan_bytes - 0x18), 8):
        vector = vector_triple_at(
            p,
            owner,
            offset,
            max_vector_count=args.max_vector_count,
            max_capacity_count=args.max_capacity_count,
        )
        if vector is None:
            continue
        element_report = classify_vector_elements(
            p,
            vector,
            vtable_to_name,
            sample_elements=args.sample_elements,
        )
        if element_report is None:
            continue
        warm = warm_validate_vector(
            p,
            owner,
            offset,
            vector,
            delay_ms=args.warm_delay_ms,
            max_vector_count=args.max_vector_count,
            max_capacity_count=args.max_capacity_count,
        )
        count_plausible = args.expect_alert_count is None or vector["count"] == args.expect_alert_count
        min_count_ok = vector["count"] >= args.expect_min_count
        candidates.append(
            {
                "root_path": root_path,
                "owner_ptr": common.hx(owner),
                "member_offset": common.hx(offset),
                "begin": common.hx(vector["begin"]),
                "end": common.hx(vector["end"]),
                "cap": common.hx(vector["cap"]),
                "count": vector["count"],
                "capacity": vector["capacity"],
                "count_matches_expected": count_plausible,
                "meets_min_count": min_count_ok,
                "element_classification": element_report,
                "warm_validation": warm,
            }
        )
        if len(candidates) >= args.max_candidates_per_root:
            break
    return candidates


def collect_child_roots(
    p: common.winmem.Process,
    root_path: str,
    owner: int,
    *,
    scan_bytes: int,
    max_children: int,
) -> list[dict[str, Any]]:
    children: list[dict[str, Any]] = []
    seen: set[int] = set()
    for offset in range(0, max(0, scan_bytes - 8), 8):
        ptr = common.u64(p, owner + offset)
        if not common.is_heap(ptr, 8) or ptr in seen:
            continue
        seen.add(ptr)
        vt = common.u64(p, ptr)
        if vt is None or not p.in_module(vt):
            continue
        children.append(
            {
                "root_path": f"{root_path}+{common.hx(offset)}",
                "ptr": ptr,
                "vtable": common.hx(vt),
                "vtable_rva": common.hx(vt - p.module_base),
            }
        )
        if len(children) >= max_children:
            break
    return children


def summarize(candidates: list[dict[str, Any]]) -> dict[str, Any]:
    strong = [
        row
        for row in candidates
        if row["element_classification"].get("first_element_matched")
        and row["warm_validation"].get("stable")
        and row.get("meets_min_count")
        and row.get("count_matches_expected")
    ]
    warm_stable = [row for row in candidates if row["warm_validation"].get("stable")]
    return {
        "status": "reachable_vector_candidates_found" if candidates else "no_reachable_vector_candidates",
        "candidate_count": len(candidates),
        "warm_stable_candidate_count": len(warm_stable),
        "strong_candidate_count": len(strong),
        "decision": (
            "Carry strong candidates to extraction-slice design only if their count "
            "matches the visible alert count or a fixture delta."
            if candidates
            else "No reachable vector candidate found from validated CGameState/player CCountry roots."
        ),
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--scan-bytes", type=int_auto, default=0x8000)
    ap.add_argument("--child-scan-bytes", type=int_auto, default=0x1800)
    ap.add_argument("--max-child-roots", type=int, default=256)
    ap.add_argument("--max-vector-count", type=int, default=4096)
    ap.add_argument("--max-capacity-count", type=int, default=8192)
    ap.add_argument("--sample-elements", type=int, default=16)
    ap.add_argument("--warm-delay-ms", type=int, default=250)
    ap.add_argument("--max-candidates-per-root", type=int, default=64)
    ap.add_argument("--expect-alert-count", type=int)
    ap.add_argument("--expect-min-count", type=int, default=1)
    args = ap.parse_args()

    args.out.parent.mkdir(parents=True, exist_ok=True)
    p = common.winmem.open_process(common.PROC_NAME)
    if p is None:
        raise SystemExit("hoi4.exe not running")

    with p:
        started = time.time()
        process = common.process_info(p)
        report: dict[str, Any] = {
            "schema": "battle-plan.session57.alert_reachability.v1",
            "lane": "session_multiplayer_metadata_live",
            "domain_id": 57,
            "probe": "probe_870ebf_alert_reachability",
            "expected_sha256": common.EXPECTED_SHA256,
            "process": process,
            "build_gate_ok": process.get("sha256") == common.EXPECTED_SHA256,
            "args": {
                "scan_bytes": common.hx(args.scan_bytes),
                "child_scan_bytes": common.hx(args.child_scan_bytes),
                "max_child_roots": args.max_child_roots,
                "max_vector_count": args.max_vector_count,
                "max_capacity_count": args.max_capacity_count,
                "sample_elements": args.sample_elements,
                "warm_delay_ms": args.warm_delay_ms,
                "expect_alert_count": args.expect_alert_count,
                "expect_min_count": args.expect_min_count,
            },
        }
        if not report["build_gate_ok"]:
            report["status"] = "build_gate_failed"
            args.out.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
            return 2

        session_context = common.read_session_context(p)
        report["session_context"] = session_context
        if not session_context.get("ok"):
            report["status"] = "session_context_gate_failed"
            args.out.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
            return 3

        game_state_s = session_context["cgame_state"]["ptr"]
        game_state = int(game_state_s, 16)
        player_roots = read_human_country_roots(p, game_state)
        report["player_country_roots"] = player_roots

        chunks = common.read_module_chunks(p)
        vtable_to_name, rtti_rows = build_rtti_map(p, chunks)
        report["rtti_substring_scan"] = {
            "substrings": RTTI_SUBSTRINGS,
            "class_count": len(rtti_rows),
            "vtable_count": len(vtable_to_name),
            "classes": rtti_rows,
        }

        roots: list[dict[str, Any]] = [
            {
                "root_path": "CGameState",
                "ptr": game_state,
                "source": "u64[module+0x3316E90]",
                "scan_bytes": args.scan_bytes,
            }
        ]
        for row in player_roots.get("human_countries", []):
            ptr_s = row.get("country_ptr")
            if isinstance(ptr_s, str):
                roots.append(
                    {
                        "root_path": f"human_country[{row.get('tag') or row.get('slot_index')}]",
                        "ptr": int(ptr_s, 16),
                        "source": "CGameState+0x310 handle array for nonzero +0x388 human marker",
                        "country": row,
                        "scan_bytes": args.scan_bytes,
                    }
                )

        candidates: list[dict[str, Any]] = []
        scanned_roots: list[dict[str, Any]] = []
        for root in roots:
            root_path = root["root_path"]
            ptr = int(root["ptr"])
            direct = scan_object_for_vectors(p, root_path, ptr, vtable_to_name, args)
            candidates.extend(direct)
            children = collect_child_roots(
                p,
                root_path,
                ptr,
                scan_bytes=args.scan_bytes,
                max_children=args.max_child_roots,
            )
            child_candidates: list[dict[str, Any]] = []
            original_scan_bytes = args.scan_bytes
            args.scan_bytes = args.child_scan_bytes
            try:
                for child in children:
                    child_candidates.extend(
                        scan_object_for_vectors(
                            p,
                            child["root_path"],
                            int(child["ptr"]),
                            vtable_to_name,
                            args,
                        )
                    )
            finally:
                args.scan_bytes = original_scan_bytes
            candidates.extend(child_candidates)
            scanned_roots.append(
                {
                    "root_path": root_path,
                    "ptr": common.hx(ptr),
                    "source": root.get("source"),
                    "direct_candidate_count": len(direct),
                    "child_root_count": len(children),
                    "child_candidate_count": len(child_candidates),
                    "child_roots_sample": [
                        {**child, "ptr": common.hx(child["ptr"])} for child in children[:24]
                    ],
                }
            )

        candidates.sort(
            key=lambda row: (
                not row["element_classification"].get("first_element_matched"),
                not row["warm_validation"].get("stable"),
                -int(row.get("count", 0)),
                row.get("root_path", ""),
            )
        )
        report["scanned_roots"] = scanned_roots
        report["candidates"] = candidates[:512]
        report["summary"] = summarize(candidates)
        report["status"] = report["summary"]["status"]
        report["elapsed_seconds"] = round(time.time() - started, 3)
        args.out.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
        print(f"probe_870ebf_alert_reachability: status={report['status']} out={args.out}")
        return 0 if candidates else 2


if __name__ == "__main__":
    raise SystemExit(main())
