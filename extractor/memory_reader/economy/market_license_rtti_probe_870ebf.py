#!/usr/bin/env python3
"""Fail-closed RTTI/heap probe for market stockpile and licensing roots.

This is a next-candidate probe only. It rediscoveres current-build vtables from
MSVC RTTI names/strings, optionally heap-scans live instances, and looks for
country-owned object paths or variant/amount row shapes. It explicitly keeps the
same-tick rejected CCountry+0x1230 stockpile path rejected.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import time
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

MEMORY_READER = Path(__file__).resolve().parents[1]
if str(MEMORY_READER) not in sys.path:
    sys.path.insert(0, str(MEMORY_READER))

import winmem  # type: ignore  # noqa: E402
import market_trade_live_probe as mtp  # type: ignore  # noqa: E402


PROC_NAME = "hoi4.exe"
EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

CCOUNTRY_SCAN_START = 0x400
CCOUNTRY_SCAN_END = 0x2400
REJECTED_MARKET_STOCKPILE_OFFSET = 0x1230
MAX_VECTOR_ROWS = 4096
MAX_AMOUNT_RAW = 1_000_000_000_000_000
OBJECT_SCAN_BYTES = 0x800

SPECIFIC_RTTI = {
    "CPurchaseContract": b".?AVCPurchaseContract@NInternationalMarket@@",
    "CMarketStockpile": b".?AVCMarketStockpile@NInternationalMarket@@",
    "CEquipmentMarketStatus": b".?AVCEquipmentMarketStatus@NInternationalMarket@@",
    "CMarketRequestAutomation": b".?AVCMarketRequestAutomation@NInternationalMarket@@",
    "CEquipmentProductionLicense": b".?AVCEquipmentProductionLicense@@",
    "CLicensedProductionConsumer": b".?AVCLicensedProductionConsumer@@",
    "CLicensedProductionStatus": b".?AVCLicensedProductionStatus@@",
    "CLicenseProductionEntry": b".?AVCLicenseProductionEntry@@",
    "CLicenceProductionAccessRelation": b".?AVCLicenceProductionAccessRelation@@",
}
STRING_NEEDLES = (b"Market", b"Stockpile", b"Automation", b"Licence", b"License", b"Licensed")


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) else None


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


def safe_u64(p: Any, addr: int) -> int | None:
    try:
        return p.u64(addr)
    except OSError:
        return None


def extract_c_string(blob: bytes, idx: int, max_len: int = 256) -> bytes:
    end = blob.find(b"\0", idx, min(len(blob), idx + max_len))
    if end < 0:
        end = min(len(blob), idx + max_len)
    raw = blob[idx:end]
    return raw if raw and all(32 <= ch < 127 for ch in raw) else b""


def collect_decorated_rtti_strings(chunks: list[tuple[int, bytes]], limit: int) -> list[bytes]:
    seen: dict[bytes, int] = {}
    for _base, blob in chunks:
        for needle in STRING_NEEDLES:
            pos = 0
            while True:
                idx = blob.find(needle, pos)
                if idx < 0:
                    break
                start = idx
                while start > 0 and blob[start - 1] != 0:
                    start -= 1
                    if idx - start > 96:
                        start = idx
                        break
                raw = extract_c_string(blob, start)
                if raw.startswith((b".?AV", b".?AU")):
                    seen.setdefault(raw, 1)
                pos = idx + 1
    out = sorted(seen)
    return out[:limit]


def country_maps_with_dynamic_fallback(p: Any) -> dict[str, Any]:
    maps = mtp.read_country_maps(p)
    id_to_country = dict(maps["id_to_country"])
    table = maps.get("country_table")
    tag_count = maps.get("tag_count") or 0
    country_count = maps.get("country_count") or 0
    country_vtable = p.module_base + mtp.COUNTRY_VTABLE_RVA
    if mtp.is_heap(table):
        for tag_id in range(1, min(tag_count, country_count, mtp.MAX_TAG_ROWS)):
            ptr = safe_u64(p, table + tag_id * 8)
            if (
                mtp.is_heap(ptr)
                and safe_u64(p, ptr) == country_vtable
                and mtp.safe_u32(p, ptr + mtp.OBJ_TAG_DB_ID) == tag_id
            ):
                id_to_country.setdefault(tag_id, ptr)
    maps["id_to_country_dynamic"] = id_to_country
    return maps


def derive_vtables(p: Any, chunks: list[tuple[int, bytes]], max_discovered: int) -> tuple[dict[str, dict[str, Any]], dict[int, str]]:
    names: dict[str, bytes] = dict(SPECIFIC_RTTI)
    for raw in collect_decorated_rtti_strings(chunks, max_discovered):
        label = raw.decode("ascii", "replace")
        names.setdefault(label, raw)
    rtti = {}
    labels_by_vtable = {}
    for label, decorated in sorted(names.items()):
        string_addrs, col_hits, vtables = mtp.derive_rtti_vtables(p, chunks, decorated)
        if not (string_addrs or col_hits or vtables or label in SPECIFIC_RTTI):
            continue
        rtti[label] = {
            "decorated_name": decorated.decode("ascii", "replace"),
            "string_hits": len(string_addrs),
            "complete_object_locator_hits": len(col_hits),
            "vtable_rvas": [hx(vt - p.module_base) for vt in vtables],
        }
        for vt in vtables:
            labels_by_vtable[vt] = label
    return rtti, labels_by_vtable


def scan_variant_amount_rows(p: Any, begin: int, size: int, stride: int, def_off: int, amount_off: int, amount_width: int) -> dict[str, Any] | None:
    if size <= 0 or size > MAX_VECTOR_ROWS:
        return None
    raw = p.try_read(begin, size * stride)
    if len(raw) < size * stride:
        return None
    variant_vtable = p.module_base + mtp.VARIANT_VTABLE_RVA
    rows = []
    bad_gated = 0
    for idx in range(size):
        base = idx * stride
        def_ptr = read_u64(raw, base + def_off)
        if not mtp.is_heap(def_ptr):
            continue
        if safe_u64(p, def_ptr) != variant_vtable or mtp.safe_u32(p, def_ptr + mtp.VARIANT_TYPE_OFF) != mtp.VARIANT_TYPE:
            bad_gated += 1
            continue
        variant_id = mtp.safe_u32(p, def_ptr + mtp.VARIANT_ID_OFF)
        amount = read_i64(raw, base + amount_off) if amount_width == 8 else read_i32(raw, base + amount_off)
        if isinstance(variant_id, int) and isinstance(amount, int) and amount and abs(amount) <= MAX_AMOUNT_RAW:
            rows.append({"index": idx, "variant_id": variant_id, "amount_raw": amount})
    if not rows:
        return None
    return {"row_count": len(rows), "bad_gated": bad_gated, "rows_preview": rows[:12]}


def scan_variant_vectors_in_blob(p: Any, blob: bytes, base_addr: int) -> list[dict[str, Any]]:
    out = []
    for rel in range(0, len(blob) - 16, 8):
        begin = read_u64(blob, rel)
        cap = read_u32(blob, rel + 8)
        size = read_u32(blob, rel + 12)
        if not mtp.is_heap(begin) or not isinstance(size, int) or not isinstance(cap, int):
            continue
        if size == 0 or size > cap or cap > MAX_VECTOR_ROWS:
            continue
        for stride in (0x10, 0x18, 0x20, 0x28):
            for def_off, amount_off, amount_width in ((0, 8, 8), (8, 0, 8), (0, 8, 4), (8, 16, 8)):
                if max(def_off + 8, amount_off + amount_width) > stride:
                    continue
                scored = scan_variant_amount_rows(p, begin, size, stride, def_off, amount_off, amount_width)
                if scored:
                    out.append(
                        {
                            "offset": base_addr + rel,
                            "relative_offset": rel,
                            "relative_offset_hex": hx(rel),
                            "begin": hx(begin),
                            "size": size,
                            "cap": cap,
                            "stride": stride,
                            "def_off": def_off,
                            "amount_off": amount_off,
                            "amount_width": amount_width,
                            **scored,
                        }
                    )
    return out


def inspect_object(p: Any, obj: int, labels_by_vtable: dict[int, str], country_ptrs: dict[int, tuple[int, str]]) -> dict[str, Any]:
    blob = p.try_read(obj, OBJECT_SCAN_BYTES)
    nested = []
    countries = []
    variants = []
    if len(blob) >= 8:
        for rel in range(0, len(blob) - 7, 8):
            ptr = read_u64(blob, rel)
            if not mtp.is_heap(ptr):
                continue
            vt = safe_u64(p, ptr)
            if vt in labels_by_vtable:
                nested.append({"offset": hx(rel), "object": hx(ptr), "label": labels_by_vtable[vt], "vtable_rva": hx(vt - p.module_base)})
            elif ptr in country_ptrs:
                tag_id, tag = country_ptrs[ptr]
                countries.append({"offset": hx(rel), "object": hx(ptr), "tag_db_id": tag_id, "tag": tag})
            elif vt == p.module_base + mtp.VARIANT_VTABLE_RVA and mtp.safe_u32(p, ptr + mtp.VARIANT_TYPE_OFF) == mtp.VARIANT_TYPE:
                variants.append({"offset": hx(rel), "object": hx(ptr), "variant_id": mtp.safe_u32(p, ptr + mtp.VARIANT_ID_OFF)})
    return {
        "object": hx(obj),
        "nested_known_object_count": len(nested),
        "country_pointer_count": len(countries),
        "variant_pointer_count": len(variants),
        "variant_amount_vectors": scan_variant_vectors_in_blob(p, blob, obj)[:8],
        "nested_examples": nested[:16],
        "country_examples": countries[:16],
        "variant_examples": variants[:16],
    }


def scan_country_owner_paths(p: Any, labels_by_vtable: dict[int, str], heap_objects: set[int], maps: dict[str, Any], max_examples: int) -> dict[str, Any]:
    country_ptrs = {
        ptr: (tag_id, maps["id_to_tag"].get(tag_id, str(tag_id)))
        for tag_id, ptr in maps["id_to_country_dynamic"].items()
    }
    direct = []
    vector_hits = []
    inline_variant_vectors = []
    for tag_id, country in sorted(maps["id_to_country_dynamic"].items()):
        tag = maps["id_to_tag"].get(tag_id, str(tag_id))
        blob = p.try_read(country + CCOUNTRY_SCAN_START, CCOUNTRY_SCAN_END - CCOUNTRY_SCAN_START)
        if len(blob) < 0x40:
            continue
        for rel in range(0, len(blob) - 7, 8):
            ptr = read_u64(blob, rel)
            if ptr in heap_objects:
                direct.append({"tag_db_id": tag_id, "tag": tag, "country_offset": hx(CCOUNTRY_SCAN_START + rel), "object": hx(ptr)})
                if len(direct) >= max_examples:
                    break
        for vec in scan_variant_vectors_in_blob(p, blob, country + CCOUNTRY_SCAN_START):
            offset = vec["offset"] - country
            row = {
                "tag_db_id": tag_id,
                "tag": tag,
                "country_offset": hx(offset),
                "status": "REJECTED_PRIOR_SAME_TICK_NO_RETRY" if offset == REJECTED_MARKET_STOCKPILE_OFFSET else "CANDIDATE_INLINE_VARIANT_AMOUNT_VECTOR",
                **{k: v for k, v in vec.items() if k != "offset"},
            }
            inline_variant_vectors.append(row)
            if len(inline_variant_vectors) >= max_examples:
                break
        for rel in range(0, len(blob) - 16, 8):
            begin = read_u64(blob, rel)
            cap = read_u32(blob, rel + 8)
            size = read_u32(blob, rel + 12)
            if not mtp.is_heap(begin) or not isinstance(size, int) or not isinstance(cap, int):
                continue
            if size == 0 or size > cap or cap > MAX_VECTOR_ROWS:
                continue
            raw = p.try_read(begin, min(size * 32, 0x8000))
            for inner in range(0, len(raw) - 7, 8):
                ptr = read_u64(raw, inner)
                if not mtp.is_heap(ptr):
                    continue
                vt = safe_u64(p, ptr)
                if vt in labels_by_vtable:
                    vector_hits.append(
                        {
                            "tag_db_id": tag_id,
                            "tag": tag,
                            "country_offset": hx(CCOUNTRY_SCAN_START + rel),
                            "begin": hx(begin),
                            "size": size,
                            "cap": cap,
                            "row_offset": hx(inner),
                            "object": hx(ptr),
                            "label": labels_by_vtable[vt],
                        }
                    )
                    if len(vector_hits) >= max_examples:
                        break
            if len(vector_hits) >= max_examples:
                break
        if len(direct) >= max_examples and len(vector_hits) >= max_examples and len(inline_variant_vectors) >= max_examples:
            break
    return {
        "country_count": len(country_ptrs),
        "direct_object_ref_examples": direct[:max_examples],
        "vector_object_ref_examples": vector_hits[:max_examples],
        "inline_variant_amount_vectors": inline_variant_vectors[:max_examples],
        "rejected_inline_path": {
            "path": "CCountry+0x1230",
            "reason": "Same-tick stockpile fixture rejected it: 272 live rows, 494 save rows, 0 exact matches.",
        },
    }


def market_oracle_summary(path: str | None) -> dict[str, Any] | None:
    if not path:
        return None
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    countries = data.get("countries", {})
    rows = []
    for tag, country in countries.items():
        for item in country.get("market_stockpile", []):
            if isinstance(item.get("equip_id"), int) and isinstance(item.get("amount"), (int, float)):
                rows.append({"tag": tag, "variant_id": item["equip_id"], "amount": item["amount"]})
    return {
        "path": path,
        "save_date": data.get("meta", {}).get("save_date"),
        "purchase_contracts": len(data.get("contracts", [])),
        "countries_with_market_data": len(countries),
        "stockpile_rows": len(rows),
        "stockpile_examples": rows[:16],
    }


def build_report(args: argparse.Namespace) -> tuple[dict[str, Any], int]:
    pid = args.pid or winmem.find_pid(PROC_NAME)
    if not pid:
        return {"status": "process_not_found", "process": PROC_NAME, "fail_closed": True}, 3
    with winmem.Process(pid, PROC_NAME) as p:
        fp = p.fingerprint()
        sha = str(fp.get("sha256") or "").lower()
        base_report: dict[str, Any] = {
            "schema": "battle-plan.market_license_rtti_probe_870ebf.v1",
            "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "expected_sha256": EXPECTED_SHA256,
            "process": {"pid": pid, "module_base": hx(p.module_base), "path": fp.get("path"), "sha256": sha},
            "fail_closed": sha != EXPECTED_SHA256,
        }
        if sha != EXPECTED_SHA256:
            base_report["status"] = "build_gate_failed"
            return base_report, 2
        started = time.perf_counter()
        chunks = mtp.read_module_chunks(p)
        rtti, labels_by_vtable = derive_vtables(p, chunks, args.max_discovered_rtti)
        heap_hits = {vt: [] for vt in labels_by_vtable}
        if args.heap_scan and labels_by_vtable:
            heap_hits = mtp.scan_heap_vtables(p, sorted(labels_by_vtable))
        label_counts = Counter()
        for vt, hits in heap_hits.items():
            label_counts[labels_by_vtable[vt]] += len(hits)
        maps = country_maps_with_dynamic_fallback(p)
        heap_objects = {addr for hits in heap_hits.values() for addr in hits}
        country_ptrs = {
            ptr: (tag_id, maps["id_to_tag"].get(tag_id, str(tag_id)))
            for tag_id, ptr in maps["id_to_country_dynamic"].items()
        }
        object_inspections = []
        for obj in sorted(heap_objects)[: args.max_objects]:
            object_inspections.append(inspect_object(p, obj, labels_by_vtable, country_ptrs))
        owner_paths = scan_country_owner_paths(p, labels_by_vtable, heap_objects, maps, args.max_examples)
        license_body_count = sum(count for label, count in label_counts.items() if "Licen" in label or "License" in label)
        report = {
            **base_report,
            "status": "probe_completed_candidates_only",
            "scan": {
                "heap_scan": args.heap_scan,
                "specific_rtti": {k: v.decode("ascii") for k, v in SPECIFIC_RTTI.items()},
                "market_oracle": market_oracle_summary(args.market_oracle),
                "expect_active_license": args.expect_active_license,
            },
            "rtti": rtti,
            "heap_body_counts_by_label": dict(sorted(label_counts.items())),
            "object_inspections": object_inspections,
            "country_owner_paths": owner_paths,
            "classification": {
                "accepted_for_emission": False,
                "license_fixture_status": (
                    "active_license_objects_observed_candidates_only"
                    if args.expect_active_license and license_body_count
                    else "active_license_fixture_missing_or_no_license_bodies"
                    if args.expect_active_license
                    else "active_license_not_required_for_this_run"
                ),
                "market_stockpile_status": (
                    "candidate_paths_require_same_tick_market_oracle"
                    if owner_paths["direct_object_ref_examples"] or owner_paths["vector_object_ref_examples"]
                    else "no_non_rejected_country_owned_market_stockpile_root_observed"
                ),
                "licensing_status": "fixture_required_do_not_promote_without_known_active_license",
            },
            "promotion_rule": (
                "Promote only a non-rejected market/licensing root with current-build RTTI, live heap bodies, "
                "a country owner path, validated row semantics, and same-frame market/active-license oracle equality."
            ),
            "elapsed_ms": round((time.perf_counter() - started) * 1000, 3),
        }
        return report, 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int)
    parser.add_argument("--out-json", required=True)
    parser.add_argument("--market-oracle")
    parser.add_argument("--heap-scan", action="store_true")
    parser.add_argument("--expect-active-license", action="store_true")
    parser.add_argument("--max-discovered-rtti", type=int, default=96)
    parser.add_argument("--max-examples", type=int, default=64)
    parser.add_argument("--max-objects", type=int, default=64)
    args = parser.parse_args()
    report, code = build_report(args)
    out = Path(args.out_json)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": report.get("status"), "out": str(out)}, sort_keys=True))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
