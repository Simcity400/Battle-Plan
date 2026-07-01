"""Bounded current-build probe for market stockpile / automation roots.

This proof-only probe derives current-build RTTI vtables for likely equipment
market stockpile classes, scans for live instances, and attempts to connect any
candidate stockpile object back to a CCountry object. It does not emit replay
rows unless both the object row shape and country ownership path are proven.
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
_REPO = _HERE.parents[5]
sys.path.insert(0, str(_REPO / "extractor" / "memory_reader"))

import market_trade_live_compact as mt  # noqa: E402
from market_trade_live_probe import derive_rtti_vtables, read_module_chunks  # noqa: E402


RTTI_NAMES = {
    "CMarketStockpile": b".?AVCMarketStockpile@NInternationalMarket@@",
    "CEquipmentMarketStatus": b".?AVCEquipmentMarketStatus@NInternationalMarket@@",
    "CMarketRequestAutomation": b".?AVCMarketRequestAutomation@NInternationalMarket@@",
}

MAX_TAG_DB_ID = 2048
MAX_STOCKPILE_ROWS = 4096
MAX_AMOUNT_RAW = 1_000_000_000_000_000
COUNTRY_SCAN_START = 0x400
COUNTRY_SCAN_END = 0x1800


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
        MAX_TAG_DB_ID,
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


def read_u32(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 4 > len(blob):
        return None
    return struct.unpack_from("<I", blob, off)[0]


def read_i32(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 4 > len(blob):
        return None
    return struct.unpack_from("<i", blob, off)[0]


def read_u64(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 8 > len(blob):
        return None
    return struct.unpack_from("<Q", blob, off)[0]


def read_i64(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 8 > len(blob):
        return None
    return struct.unpack_from("<q", blob, off)[0]


def scan_variant_amount_rows(
    p: mt.winmem.Process,
    begin: int,
    size: int,
    stride: int,
    def_off: int,
    amount_off: int,
    amount_width: int,
    variant_vtable: int,
) -> dict | None:
    if size <= 0 or size > MAX_STOCKPILE_ROWS:
        return None
    raw = p.try_read(begin, size * stride)
    if len(raw) < size * stride:
        return None
    rows = []
    tombstones = 0
    bad_gated = 0
    nonzero = 0
    for idx in range(size):
        base = idx * stride
        def_ptr = read_u64(raw, base + def_off)
        if not mt.is_heap(def_ptr):
            tombstones += 1
            continue
        if mt.safe_u64(p, def_ptr) != variant_vtable:
            bad_gated += 1
            continue
        if mt.safe_u32(p, def_ptr + mt.VARIANT_TYPE_OFF) != mt.VARIANT_TYPE:
            bad_gated += 1
            continue
        variant_id = mt.safe_u32(p, def_ptr + mt.VARIANT_ID_OFF)
        amount = read_i64(raw, base + amount_off) if amount_width == 8 else read_i32(raw, base + amount_off)
        if not variant_id or not amount or abs(amount) > MAX_AMOUNT_RAW:
            continue
        nonzero += 1
        rows.append(
            {
                "index": idx,
                "variant_id": variant_id,
                "amount_raw": amount,
            }
        )
    if not rows:
        return None
    return {
        "row_count": len(rows),
        "nonzero_rows": nonzero,
        "tombstones": tombstones,
        "bad_gated": bad_gated,
        "rows_preview": rows[:12],
    }


def scan_stockpile_object_rows(p: mt.winmem.Process, obj: int, variant_vtable: int) -> list[dict]:
    blob = p.try_read(obj, 0x300)
    out = []
    if len(blob) < 0x80:
        return out
    for rel in range(0x08, min(len(blob) - 16, 0x280), 8):
        begin = read_u64(blob, rel)
        cap = read_u32(blob, rel + 8)
        size = read_u32(blob, rel + 12)
        if not mt.is_heap(begin) or size is None or cap is None:
            continue
        if size > cap or cap > MAX_STOCKPILE_ROWS or size == 0:
            continue
        for stride in (0x10, 0x18, 0x20):
            for def_off, amount_off, amount_width in (
                (0x00, 0x08, 8),
                (0x08, 0x00, 8),
                (0x00, 0x08, 4),
                (0x08, 0x10, 8),
            ):
                if max(def_off + 8, amount_off + amount_width) > stride:
                    continue
                scored = scan_variant_amount_rows(
                    p,
                    begin,
                    size,
                    stride,
                    def_off,
                    amount_off,
                    amount_width,
                    variant_vtable,
                )
                if not scored:
                    continue
                out.append(
                    {
                        "object_vector_offset": rel,
                        "object_vector_offset_hex": hx(rel),
                        "begin": hx(begin),
                        "cap": cap,
                        "size": size,
                        "stride": stride,
                        "def_off": def_off,
                        "amount_off": amount_off,
                        "amount_width": amount_width,
                        **scored,
                    }
                )
    return out


def find_country_pointer_refs(
    p: mt.winmem.Process,
    candidate_ptrs: set[int],
    country_rows: list[tuple[int, str, int]],
) -> dict[int, list[dict]]:
    refs: dict[int, list[dict]] = defaultdict(list)
    if not candidate_ptrs:
        return refs
    for tag_id, tag, country_ptr in country_rows:
        blob = p.try_read(country_ptr + COUNTRY_SCAN_START, COUNTRY_SCAN_END - COUNTRY_SCAN_START)
        if len(blob) < 8:
            continue
        for rel in range(0, len(blob) - 7, 8):
            ptr = struct.unpack_from("<Q", blob, rel)[0]
            if ptr in candidate_ptrs:
                refs[ptr].append(
                    {
                        "tag_db_id": tag_id,
                        "tag": tag,
                        "country_ptr": hx(country_ptr),
                        "country_relative_offset": COUNTRY_SCAN_START + rel,
                        "country_relative_offset_hex": hx(COUNTRY_SCAN_START + rel),
                    }
                )
    return refs


def scan_country_inline_variant_vectors(
    p: mt.winmem.Process,
    country_rows: list[tuple[int, str, int]],
    variant_vtable: int,
    max_examples: int,
) -> dict:
    groups: dict[tuple, list[dict]] = defaultdict(list)
    for tag_id, tag, country_ptr in country_rows:
        blob = p.try_read(country_ptr + COUNTRY_SCAN_START, COUNTRY_SCAN_END - COUNTRY_SCAN_START)
        if len(blob) < 0x40:
            continue
        for rel in range(0, len(blob) - 16, 8):
            off = COUNTRY_SCAN_START + rel
            begin = read_u64(blob, rel)
            cap = read_u32(blob, rel + 8)
            size = read_u32(blob, rel + 12)
            if not mt.is_heap(begin) or size is None or cap is None:
                continue
            if size > cap or cap > MAX_STOCKPILE_ROWS or size == 0:
                continue
            for stride in (0x10, 0x18, 0x20):
                for def_off, amount_off, amount_width in (
                    (0x00, 0x08, 8),
                    (0x08, 0x00, 8),
                    (0x00, 0x08, 4),
                    (0x08, 0x10, 8),
                ):
                    if max(def_off + 8, amount_off + amount_width) > stride:
                        continue
                    scored = scan_variant_amount_rows(
                        p,
                        begin,
                        size,
                        stride,
                        def_off,
                        amount_off,
                        amount_width,
                        variant_vtable,
                    )
                    if not scored:
                        continue
                    key = (off, stride, def_off, amount_off, amount_width)
                    groups[key].append(
                        {
                            "tag_db_id": tag_id,
                            "tag": tag,
                            "country_ptr": hx(country_ptr),
                            "begin": hx(begin),
                            "cap": cap,
                            "size": size,
                            **scored,
                        }
                    )
    summaries = []
    for (off, stride, def_off, amount_off, amount_width), rows in groups.items():
        summaries.append(
            {
                "country_relative_offset": off,
                "country_relative_offset_hex": hx(off),
                "stride": stride,
                "def_off": def_off,
                "amount_off": amount_off,
                "amount_width": amount_width,
                "tag_count": len({row["tag_db_id"] for row in rows}),
                "nonzero_tag_count": sum(1 for row in rows if row["nonzero_rows"] > 0),
                "row_count_total": sum(row["row_count"] for row in rows),
                "bad_gated_total": sum(row["bad_gated"] for row in rows),
                "examples": rows[:max_examples],
            }
        )
    summaries.sort(
        key=lambda row: (
            -row["tag_count"],
            -row["nonzero_tag_count"],
            row["bad_gated_total"],
            row["country_relative_offset"],
            row["stride"],
        )
    )
    accepted = [
        row
        for row in summaries
        if row["tag_count"] >= 3 and row["nonzero_tag_count"] >= 1 and row["bad_gated_total"] == 0
    ]
    return {
        "candidate_group_count": len(summaries),
        "accepted_group_count": len(accepted),
        "accepted_for_emission": False,
        "best_groups": summaries[:12],
        "rejection": (
            "Inline CCountry scan did not prove a market-stockpile root. Any "
            "variant/amount vector here still needs same-tick market_stockpile "
            "fixture values and a way to distinguish sellable market stockpile "
            "from ordinary production stockpile before compact emission."
        ),
    }


def summarize_candidate_hits(
    p: mt.winmem.Process,
    hits_by_class: dict[str, list[int]],
    country_rows: list[tuple[int, str, int]],
    variant_vtable: int,
    max_examples: int,
) -> dict:
    all_candidates = {addr for rows in hits_by_class.values() for addr in rows}
    refs = find_country_pointer_refs(p, all_candidates, country_rows)
    classes = {}
    for class_name, hits in hits_by_class.items():
        examples = []
        owned_count = 0
        row_shape_count = 0
        for addr in hits[:max_examples]:
            row_shapes = scan_stockpile_object_rows(p, addr, variant_vtable)
            owner_refs = refs.get(addr, [])
            if owner_refs:
                owned_count += 1
            if row_shapes:
                row_shape_count += 1
            examples.append(
                {
                    "addr": hx(addr),
                    "country_refs": owner_refs[:8],
                    "row_shapes": row_shapes[:4],
                }
            )
        classes[class_name] = {
            "vtable_hits": len(hits),
            "examples_scanned": min(len(hits), max_examples),
            "examples_with_country_refs": owned_count,
            "examples_with_variant_amount_rows": row_shape_count,
            "examples": examples,
        }
    return classes


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
        chunks = read_module_chunks(p)
        rtti = {}
        vtables_by_class = {}
        for class_name, name in RTTI_NAMES.items():
            strings, col_hits, vtables = derive_rtti_vtables(p, chunks, name)
            rtti[class_name] = {
                "decorated_name": name.decode("ascii"),
                "string_hits": len(strings),
                "complete_object_locator_hits": len(col_hits),
                "vtables": [hx(vt) for vt in vtables],
                "vtable_rvas": [hx(vt - p.module_base) for vt in vtables],
            }
            if vtables:
                vtables_by_class[class_name] = vtables
        scan_targets = [vt for vtables in vtables_by_class.values() for vt in vtables]
        hits_by_class = {name: [] for name in RTTI_NAMES}
        if scan_targets:
            raw_hits = mt.scan_heap_vtables(p, scan_targets)
            for class_name, vtables in vtables_by_class.items():
                hits = []
                for vt in vtables:
                    hits.extend(raw_hits.get(vt, []))
                hits_by_class[class_name] = sorted(set(hits))
        maps = country_maps_with_dynamic_fallback(p)
        country_rows = [
            (tag_id, maps["id_to_tag"].get(tag_id, str(tag_id)), ptr)
            for tag_id, ptr in sorted(maps["id_to_country_dynamic"].items())
            if ptr
        ]
        class_summary = summarize_candidate_hits(
            p,
            hits_by_class,
            country_rows,
            p.module_base + mt.VARIANT_VTABLE_RVA,
            args.max_examples,
        )
        inline_vectors = scan_country_inline_variant_vectors(
            p,
            country_rows,
            p.module_base + mt.VARIANT_VTABLE_RVA,
            args.max_examples,
        )
        accepted = []
        for class_name, summary in class_summary.items():
            for example in summary["examples"]:
                if example["country_refs"] and example["row_shapes"]:
                    accepted.append({"class": class_name, **example})
        proof = {
            "schema": "market_trade_live_market_stockpile_root_probe_v1",
            "utc": datetime.now(timezone.utc).isoformat(),
            "pid": pid,
            "module": {
                "path": fp.get("path"),
                "base": hx(p.module_base),
                "sha256": sha,
            },
            "rtti": rtti,
            "country_ref_scan": {
                "country_count": len(country_rows),
                "scan_start": hx(COUNTRY_SCAN_START),
                "scan_end": hx(COUNTRY_SCAN_END),
            },
            "classes": class_summary,
            "country_inline_variant_vectors": inline_vectors,
            "classification": {
                "accepted_candidate_count": len(accepted),
                "accepted_for_emission": False,
                "reason": (
                    "No candidate has both a country-owned pointer path and a validated "
                    "type-70 variant/amount row shape. Market stockpile/automation "
                    "remains source-blocked until a CCountry equipment_market root is pinned."
                ),
                "accepted_candidates": accepted[:8],
            },
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
    parser.add_argument("--max-examples", type=int, default=48)
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args(argv)
    proof = probe(args)
    print(json.dumps(proof["classification"], indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
