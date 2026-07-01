"""Read-only D04/tag-db 368 identity blocker probe for market_trade_live.

This is discovery/validation only. It checks the country database tag row, the
CGameState country table slot, and any unresolved buyer pointers reported by a
market_trade_live probe JSON. It does not write game memory and is not a
production extractor.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from datetime import datetime, timezone

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", "..", ".."))
MEMORY_READER = os.path.join(ROOT, "extractor", "memory_reader")
sys.path.insert(0, MEMORY_READER)

import winmem  # noqa: E402
from market_trade_live_probe import (  # noqa: E402
    CGAMESTATE_COUNTRY_COUNT,
    CGAMESTATE_COUNTRY_TABLE,
    CG_STATE_DATE_OFFSET,
    COUNTRY_DATABASE_OFFSET,
    COUNTRY_VTABLE_RVA,
    EXPECTED_SHA256,
    OBJ_TAG_DB_ID,
    PROC_NAME,
    hx,
    is_heap,
    read_inline_tag,
    safe_u32,
    safe_u64,
)


D04_TAG_DB_ID = 368


def buyer_ptrs_from_probe(path: str | None) -> list[int]:
    if not path:
        return []
    with open(path, "r", encoding="utf-8") as fh:
        data = json.load(fh)
    ptrs: list[int] = []
    for row in (
        data.get("candidate_evidence_examples", {}).get("missing_extractable", [])
    ):
        if row.get("buyer_tag_db_id") != D04_TAG_DB_ID:
            continue
        text = row.get("buyer_ptr")
        if isinstance(text, str) and text.startswith("0x"):
            ptrs.append(int(text, 16))
    return sorted(set(ptrs))


def probe(args: argparse.Namespace) -> dict:
    pid = args.pid or winmem.find_pid(PROC_NAME)
    if not pid:
        raise SystemExit("hoi4.exe PID not found")
    with winmem.Process(pid, PROC_NAME) as p:
        fp = p.fingerprint()
        sha = str(fp.get("sha256") or "").lower()
        if sha != EXPECTED_SHA256 and not args.force:
            raise SystemExit(
                f"live hoi4.exe sha256 {sha} != expected {EXPECTED_SHA256}; pass --force"
            )
        game_state = safe_u64(p, p.module_base + CG_STATE_DATE_OFFSET)
        country_database = safe_u64(p, p.module_base + COUNTRY_DATABASE_OFFSET)
        tag_table = safe_u64(p, country_database) if is_heap(country_database) else None
        tag_count = safe_u32(p, country_database + 8) if is_heap(country_database) else None
        active_count = safe_u32(p, country_database + 12) if is_heap(country_database) else None
        country_table = safe_u64(p, game_state + CGAMESTATE_COUNTRY_TABLE) if is_heap(game_state) else None
        country_count = safe_u32(p, game_state + CGAMESTATE_COUNTRY_COUNT) if is_heap(game_state) else None
        country_vtable = p.module_base + COUNTRY_VTABLE_RVA

        tag_text = read_inline_tag(p, tag_table + D04_TAG_DB_ID * 32) if is_heap(tag_table) else None
        country_table_ptr = (
            safe_u64(p, country_table + D04_TAG_DB_ID * 8) if is_heap(country_table) else None
        )
        country_table_vtable = safe_u64(p, country_table_ptr) if is_heap(country_table_ptr) else None
        country_table_tag_id = (
            safe_u32(p, country_table_ptr + OBJ_TAG_DB_ID) if is_heap(country_table_ptr) else None
        )
        table_census = {
            "slot_count_scanned": 0,
            "heap_country_vtable_slots": 0,
            "obj_tag_db_id_matches_slot": 0,
            "obj_tag_db_id_mismatches_slot": 0,
            "valid_slot_index_fallback_candidates": 0,
            "mismatch_examples": [],
        }
        if is_heap(country_table):
            limit = min(country_count or 0, active_count or 0, 2048)
            for tag_id in range(1, limit):
                table_census["slot_count_scanned"] += 1
                ptr = safe_u64(p, country_table + tag_id * 8)
                if not is_heap(ptr) or safe_u64(p, ptr) != country_vtable:
                    continue
                table_census["heap_country_vtable_slots"] += 1
                obj_tag_id = safe_u32(p, ptr + OBJ_TAG_DB_ID)
                if obj_tag_id == tag_id:
                    table_census["obj_tag_db_id_matches_slot"] += 1
                    continue
                table_census["obj_tag_db_id_mismatches_slot"] += 1
                tag = read_inline_tag(p, tag_table + tag_id * 32) if is_heap(tag_table) else None
                if tag and len(tag) == 3:
                    table_census["valid_slot_index_fallback_candidates"] += 1
                if len(table_census["mismatch_examples"]) < 16:
                    table_census["mismatch_examples"].append(
                        {
                            "slot_tag_db_id": tag_id,
                            "tag": tag,
                            "ptr": hx(ptr),
                            "obj_tag_db_id": obj_tag_id,
                        }
                    )

        buyer_ptrs = buyer_ptrs_from_probe(args.probe_json)
        buyer_ptr_reports = []
        for buyer_ptr in buyer_ptrs:
            buyer_ptr_reports.append(
                {
                    "ptr": hx(buyer_ptr),
                    "is_heap": is_heap(buyer_ptr),
                    "vtable": hx(safe_u64(p, buyer_ptr)) if is_heap(buyer_ptr) else None,
                    "vtable_matches_country": (
                        safe_u64(p, buyer_ptr) == country_vtable if is_heap(buyer_ptr) else False
                    ),
                    "obj_tag_db_id": (
                        safe_u32(p, buyer_ptr + OBJ_TAG_DB_ID) if is_heap(buyer_ptr) else None
                    ),
                    "matches_country_table_slot": buyer_ptr == country_table_ptr,
                }
            )

        return {
            "schema": "market_trade_live_d04_country_identity_probe_v1",
            "utc": datetime.now(timezone.utc).isoformat(),
            "pid": pid,
            "module": {
                "path": fp.get("path"),
                "base": hx(p.module_base),
                "sha256": sha,
            },
            "roots": {
                "game_state": hx(game_state),
                "country_database": hx(country_database),
                "tag_table": hx(tag_table),
                "tag_count": tag_count,
                "active_count": active_count,
                "country_table": hx(country_table),
                "country_count": country_count,
                "country_vtable": hx(country_vtable),
                "obj_tag_db_id_offset": hx(OBJ_TAG_DB_ID),
            },
            "d04": {
                "tag_db_id": D04_TAG_DB_ID,
                "tag_table_text": tag_text,
                "country_table_slot_ptr": hx(country_table_ptr),
                "country_table_slot_vtable": hx(country_table_vtable),
                "country_table_slot_vtable_matches_country": country_table_vtable == country_vtable,
                "country_table_slot_obj_tag_db_id": country_table_tag_id,
                "country_table_slot_validated": (
                    is_heap(country_table_ptr)
                    and country_table_vtable == country_vtable
                    and country_table_tag_id == D04_TAG_DB_ID
                ),
                "unresolved_buyer_ptrs_from_probe": buyer_ptr_reports,
            },
            "country_table_census": table_census,
            "blocker": (
                "D04 tag row and CGameState.country_table[368] pointer validate the "
                "buyer object by table slot and vtable, but CCountry+0x08 reads 447 "
                "instead of 368. Existing object-tag-id validation is too strict for "
                "this dynamic tag; shared collector should add a bounded slot-index "
                "fallback before these buyer rows are emitted as complete."
            ),
        }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int)
    parser.add_argument("--probe-json")
    parser.add_argument("--out")
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args(argv)
    report = probe(args)
    if args.out:
        os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
        with open(args.out, "w", encoding="utf-8") as fh:
            json.dump(report, fh, indent=2, sort_keys=True)
            fh.write("\n")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if not report["d04"]["country_table_slot_validated"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
