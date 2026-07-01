"""Bounded current-build probe for country resource-flow roots.

This is proof-only. It tests the `CCountry+0x5B8` clue from the fuel reverse
engineering notes and nearby compact-vector-looking fields. It does not emit
runtime replay rows unless a candidate structure is strong enough to satisfy the
lane's anti-fabrication bar.
"""

from __future__ import annotations

import argparse
import json
import os
import struct
import sys
import time
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path

_HERE = Path(__file__).resolve()
_REPO = _HERE.parents[4]
sys.path.insert(0, str(_REPO / "memory_reader"))

import market_trade_live_compact as mt  # noqa: E402


COUNTRY_RESOURCE_CLUE_OFF = 0x5B8
SCAN_START = 0x540
SCAN_END = 0x640
MAX_VECTOR_ROWS = 256
RESOURCE_ID_MAX = 128
RESOURCE_AMOUNT_ABS_MAX = 10_000_000_000
SAMPLE_TAGS = {
    "GER",
    "ENG",
    "SOV",
    "USA",
    "JAP",
    "ITA",
    "FRA",
    "BRA",
    "ROM",
    "HUN",
    "TUR",
    "SWE",
}


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) and value else None


def country_maps_with_dynamic_fallback(p: mt.winmem.Process) -> dict:
    maps = mt.read_country_maps(p)
    id_to_country = dict(maps["id_to_country"])
    country_vtable = p.module_base + mt.COUNTRY_VTABLE_RVA
    country_table = maps.get("country_table")
    limit = min(
        maps.get("country_count") or 0,
        maps.get("tag_count") or 0,
        mt.MAX_TAG_DB_ID,
    )
    if mt.is_heap(country_table):
        for tag_id in range(1, limit):
            tag = maps["id_to_tag"].get(tag_id)
            if not tag or not (len(tag) == 3 and tag[0] == "D" and tag[1:].isdigit()):
                continue
            ptr = mt.safe_u64(p, country_table + tag_id * 8)
            if mt.is_heap(ptr) and mt.safe_u64(p, ptr) == country_vtable:
                id_to_country.setdefault(tag_id, ptr)
    maps["id_to_country_dynamic"] = id_to_country
    return maps


def read_i32_from(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 4 > len(blob):
        return None
    return struct.unpack_from("<i", blob, off)[0]


def read_u32_from(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 4 > len(blob):
        return None
    return struct.unpack_from("<I", blob, off)[0]


def read_i64_from(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 8 > len(blob):
        return None
    return struct.unpack_from("<q", blob, off)[0]


def plausible_resource_id(value: int | None) -> bool:
    return isinstance(value, int) and 0 <= value <= RESOURCE_ID_MAX


def plausible_amount(value: int | None) -> bool:
    return isinstance(value, int) and abs(value) <= RESOURCE_AMOUNT_ABS_MAX


def score_rows(raw: bytes, size: int, stride: int, amount_off: int, resource_off: int, amount_width: int):
    if size <= 0 or size > MAX_VECTOR_ROWS or len(raw) < size * stride:
        return None
    rows = []
    resource_ids = []
    nonzero_amounts = 0
    for idx in range(size):
        base = idx * stride
        rid = read_u32_from(raw, base + resource_off)
        amount = read_i64_from(raw, base + amount_off) if amount_width == 8 else read_i32_from(raw, base + amount_off)
        if not plausible_resource_id(rid) or not plausible_amount(amount):
            return None
        resource_ids.append(rid)
        if amount:
            nonzero_amounts += 1
        rows.append({"resource_id": rid, "amount_raw": amount})
    unique_ids = len(set(resource_ids))
    duplicate_ids = len(resource_ids) - unique_ids
    return {
        "rows": rows,
        "nonzero_amounts": nonzero_amounts,
        "unique_ids": unique_ids,
        "duplicate_ids": duplicate_ids,
        "resource_ids": resource_ids,
    }


def scan_country_vectors(p: mt.winmem.Process, country_ptr: int, tag_id: int, tag: str) -> list[dict]:
    blob = p.try_read(country_ptr + SCAN_START, SCAN_END - SCAN_START)
    out = []
    if len(blob) < SCAN_END - SCAN_START:
        return out
    for rel in range(0, len(blob) - 16, 8):
        off = SCAN_START + rel
        begin = struct.unpack_from("<Q", blob, rel)[0]
        cap = read_u32_from(blob, rel + 8)
        size = read_u32_from(blob, rel + 12)
        if not mt.is_heap(begin) or size is None or cap is None:
            continue
        if size > cap or cap > MAX_VECTOR_ROWS or size == 0:
            continue
        for stride in (8, 12, 16, 24, 32):
            raw = p.try_read(begin, size * stride)
            if len(raw) < size * stride:
                continue
            layouts = [
                (0, 4, 4),
                (4, 0, 4),
                (0, 8, 8),
                (8, 0, 8),
                (8, 16, 8),
                (16, 8, 8),
            ]
            for amount_off, resource_off, amount_width in layouts:
                if max(amount_off + amount_width, resource_off + 4) > stride:
                    continue
                scored = score_rows(raw, size, stride, amount_off, resource_off, amount_width)
                if not scored:
                    continue
                out.append(
                    {
                        "tag_db_id": tag_id,
                        "tag": tag,
                        "country_ptr": hx(country_ptr),
                        "candidate_offset": off,
                        "candidate_offset_hex": hx(off),
                        "relative_to_clue": off - COUNTRY_RESOURCE_CLUE_OFF,
                        "begin": hx(begin),
                        "cap": cap,
                        "size": size,
                        "stride": stride,
                        "amount_off": amount_off,
                        "amount_width": amount_width,
                        "resource_off": resource_off,
                        "nonzero_amounts": scored["nonzero_amounts"],
                        "unique_ids": scored["unique_ids"],
                        "duplicate_ids": scored["duplicate_ids"],
                        "resource_ids": scored["resource_ids"],
                        "rows_preview": scored["rows"][:12],
                    }
                )
    return out


def classify_candidates(candidates: list[dict]) -> dict:
    groups: dict[tuple, list[dict]] = defaultdict(list)
    for cand in candidates:
        key = (
            cand["candidate_offset"],
            cand["stride"],
            cand["amount_off"],
            cand["amount_width"],
            cand["resource_off"],
        )
        groups[key].append(cand)
    summaries = []
    for key, rows in groups.items():
        offsets, stride, amount_off, amount_width, resource_off = key
        tag_count = len({row["tag_db_id"] for row in rows})
        resource_sets = Counter(tuple(row["resource_ids"]) for row in rows)
        nonzero_tags = sum(1 for row in rows if row["nonzero_amounts"] > 0)
        duplicate_tags = sum(1 for row in rows if row["duplicate_ids"] > 0)
        summaries.append(
            {
                "candidate_offset": offsets,
                "candidate_offset_hex": hx(offsets),
                "relative_to_clue": offsets - COUNTRY_RESOURCE_CLUE_OFF,
                "stride": stride,
                "amount_off": amount_off,
                "amount_width": amount_width,
                "resource_off": resource_off,
                "tag_count": tag_count,
                "nonzero_tag_count": nonzero_tags,
                "duplicate_id_tag_count": duplicate_tags,
                "distinct_resource_id_sequences": len(resource_sets),
                "top_resource_id_sequences": [
                    {"resource_ids": list(ids), "count": count}
                    for ids, count in resource_sets.most_common(5)
                ],
                "examples": rows[:8],
            }
        )
    summaries.sort(
        key=lambda row: (
            row["candidate_offset"] != COUNTRY_RESOURCE_CLUE_OFF,
            -row["tag_count"],
            -row["nonzero_tag_count"],
            row["duplicate_id_tag_count"],
            row["candidate_offset"],
            row["stride"],
        )
    )
    accepted = [
        row
        for row in summaries
        if row["candidate_offset"] == COUNTRY_RESOURCE_CLUE_OFF
        and row["tag_count"] >= 6
        and row["nonzero_tag_count"] >= 2
        and row["duplicate_id_tag_count"] == 0
        and row["distinct_resource_id_sequences"] <= 3
    ]
    return {
        "accepted_candidate_count": len(accepted),
        "accepted_for_emission": False,
        "best_candidates": summaries[:12],
        "rejection": (
            "No candidate reached production-emission confidence. This probe only "
            "accepts CCountry+0x5B8 if multiple countries share a stable resource-id "
            "sequence with nonzero variance and no duplicate ids; current evidence "
            "must be joined to same-tick save/UI resource values before compact rows."
        ),
    }


def probe(args: argparse.Namespace) -> dict:
    pid = args.pid or mt.winmem.find_pid(mt.PROC_NAME)
    if not pid:
        raise SystemExit(f"{mt.PROC_NAME} not running")
    started = time.perf_counter()
    with mt.winmem.Process(pid, mt.PROC_NAME) as p:
        fp = p.fingerprint()
        sha = str(fp.get("sha256") or "").lower()
        if sha != mt.EXPECTED_SHA256 and not args.force:
            raise SystemExit(f"live hoi4.exe sha256 {sha} != expected {mt.EXPECTED_SHA256}")
        maps = country_maps_with_dynamic_fallback(p)
        selected = []
        for tag_id, tag in sorted(maps["id_to_tag"].items()):
            ptr = maps["id_to_country_dynamic"].get(tag_id)
            if not ptr:
                continue
            if tag in SAMPLE_TAGS or len(selected) < args.min_countries:
                selected.append((tag_id, tag, ptr))
            if len(selected) >= args.max_countries:
                break
        candidates = []
        for tag_id, tag, ptr in selected:
            candidates.extend(scan_country_vectors(p, ptr, tag_id, tag))
        classification = classify_candidates(candidates)
        proof = {
            "schema": "market_trade_live_resource_flow_root_probe_v1",
            "utc": datetime.now(timezone.utc).isoformat(),
            "pid": pid,
            "module": {
                "path": fp.get("path"),
                "base": hx(p.module_base),
                "sha256": sha,
            },
            "root_tested": {
                "country_resource_clue": "CCountry+0x5B8",
                "scan_start": hx(SCAN_START),
                "scan_end": hx(SCAN_END),
                "source_note": "memory_reader/fuel_offsets.draft.json: fuel_status recompute reads country resources at country+0x5b8",
            },
            "countries_scanned": len(selected),
            "candidate_layouts": len(candidates),
            "classification": classification,
            "exact_blocker": (
                "country_resource_import_export_flows remain source-blocked. "
                "The +0x5B8 area has not been proven to map produced/exported/"
                "to_export/imported/shortage by resource id. Need same-tick "
                "country resource fixture or UI/save oracle values joined by tag."
            ),
            "elapsed_ms": round((time.perf_counter() - started) * 1000, 3),
        }
    if args.out_json:
        os.makedirs(os.path.dirname(os.path.abspath(args.out_json)), exist_ok=True)
        with open(args.out_json, "w", encoding="utf-8") as fh:
            json.dump(proof, fh, indent=2, sort_keys=True)
            fh.write("\n")
    return proof


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int)
    parser.add_argument("--out-json")
    parser.add_argument("--max-countries", type=int, default=64)
    parser.add_argument("--min-countries", type=int, default=24)
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args(argv)
    proof = probe(args)
    print(json.dumps(proof["classification"], indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
