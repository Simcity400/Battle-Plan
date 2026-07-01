"""Current-build field census for unvalidated CPurchaseContract residuals.

This probe does not assign semantics. It enumerates plausible scalar and pointer
offsets for speed/completed/delivery/progress follow-up fixtures, while
preserving the lane rule that unproven fields are not emitted.
"""

from __future__ import annotations

import argparse
import json
import os
import struct
import sys
import time
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

_HERE = Path(__file__).resolve()
_REPO = _HERE.parents[4]
sys.path.insert(0, str(_REPO / "memory_reader"))

import market_trade_live_compact as mt  # noqa: E402


OBJECT_READ_LEN = 0x220
KNOWN_OFFSETS = {
    0x00: "vtable",
    0x08: "id_type",
    0x0C: "id_id",
    0x38: "equipment_subbuf",
    0x58: "seller_tag_db_id",
    0x5C: "buyer_tag_db_id",
    0x80: "delivery_subbuf",
    0x128: "buyer_ptr",
}


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) else None


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


def accepted_contracts(p: mt.winmem.Process, maps: dict) -> list[tuple[int, mt.MarketTradeRow]]:
    contract_vtable = p.module_base + mt.CPURCHASE_CONTRACT_VTABLE_RVA
    variant_vtable = p.module_base + mt.VARIANT_VTABLE_RVA
    out = []
    for addr in mt.scan_contract_candidates(p):
        row, reason = mt.read_contract_candidate(
            p,
            addr,
            contract_vtable,
            variant_vtable,
            maps["id_to_country_dynamic"],
        )
        if row is not None and mt.is_complete_contract_row(row):
            out.append((addr, row))
    out.sort(key=lambda item: item[1].contract_id)
    return out


def summarize_scalar_offsets(samples: list[tuple[int, mt.MarketTradeRow, bytes]], maps: dict) -> dict:
    scalar_summaries = []
    tag_summaries = []
    for off in range(0x10, OBJECT_READ_LEN - 3, 4):
        if off in KNOWN_OFFSETS:
            continue
        vals = []
        for _addr, _row, blob in samples:
            vals.append(struct.unpack_from("<i", blob, off)[0])
        nonzero = [value for value in vals if value != 0]
        unique = sorted(set(vals))
        if not nonzero:
            continue
        min_v = min(vals)
        max_v = max(vals)
        nonzero_count = len(nonzero)
        if 0 <= min_v and max_v <= 36500:
            scalar_summaries.append(
                {
                    "offset": off,
                    "offset_hex": hx(off),
                    "kind": "small_nonnegative_i32",
                    "nonzero_count": nonzero_count,
                    "unique_count": len(unique),
                    "min": min_v,
                    "max": max_v,
                    "sample_values": unique[:16],
                }
            )
        if 0 <= min_v and max_v <= mt.SCALE_RAW:
            scalar_summaries.append(
                {
                    "offset": off,
                    "offset_hex": hx(off),
                    "kind": "fixed_point_0_to_1_i32",
                    "nonzero_count": nonzero_count,
                    "unique_count": len(unique),
                    "min": min_v,
                    "max": max_v,
                    "sample_values": unique[:16],
                }
            )
        tag_hits = Counter()
        for value in vals:
            tag = maps["id_to_tag"].get(value)
            if tag:
                tag_hits[tag] += 1
        if tag_hits and sum(tag_hits.values()) >= max(2, len(samples) // 4):
            tag_summaries.append(
                {
                    "offset": off,
                    "offset_hex": hx(off),
                    "tag_hit_count": sum(tag_hits.values()),
                    "distinct_tags": len(tag_hits),
                    "top_tags": [
                        {"tag": tag, "count": count}
                        for tag, count in tag_hits.most_common(12)
                    ],
                }
            )
    scalar_summaries.sort(
        key=lambda row: (
            row["kind"] != "small_nonnegative_i32",
            -row["nonzero_count"],
            row["unique_count"],
            row["offset"],
        )
    )
    tag_summaries.sort(key=lambda row: (-row["tag_hit_count"], row["offset"]))
    return {
        "plausible_scalar_offsets": scalar_summaries[:64],
        "plausible_tag_offsets": tag_summaries[:32],
    }


def summarize_pointer_offsets(p: mt.winmem.Process, samples: list[tuple[int, mt.MarketTradeRow, bytes]]) -> list[dict]:
    out = []
    variant_vtable = p.module_base + mt.VARIANT_VTABLE_RVA
    country_vtable = p.module_base + mt.COUNTRY_VTABLE_RVA
    for off in range(0x10, OBJECT_READ_LEN - 7, 8):
        if off in KNOWN_OFFSETS:
            continue
        ptrs = [struct.unpack_from("<Q", blob, off)[0] for _addr, _row, blob in samples]
        heap_ptrs = [ptr for ptr in ptrs if mt.is_heap(ptr)]
        if len(heap_ptrs) < max(2, len(samples) // 4):
            continue
        pointee_vtables = Counter()
        readable_count = 0
        variant_subbuf_like = 0
        for ptr in heap_ptrs:
            vt = mt.safe_u64(p, ptr)
            if vt:
                readable_count += 1
                if p.module_base <= vt < p.module_base + p.module_size:
                    pointee_vtables[hx(vt - p.module_base)] += 1
                if vt == country_vtable:
                    pointee_vtables["CCountry"] += 1
            status, variant_id, amount_raw = mt.scan_variant_subbuffer(p, ptr, variant_vtable)
            if status == mt.STATUS_VALID and variant_id and amount_raw:
                variant_subbuf_like += 1
        out.append(
            {
                "offset": off,
                "offset_hex": hx(off),
                "heap_ptr_count": len(heap_ptrs),
                "readable_pointee_count": readable_count,
                "variant_subbuffer_like_count": variant_subbuf_like,
                "top_pointee_vtable_rvas": [
                    {"rva": rva, "count": count}
                    for rva, count in pointee_vtables.most_common(12)
                ],
                "sample_ptrs": [hx(ptr) for ptr in heap_ptrs[:12]],
            }
        )
    out.sort(
        key=lambda row: (
            -row["variant_subbuffer_like_count"],
            -row["heap_ptr_count"],
            row["offset"],
        )
    )
    return out[:48]


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
        contracts = accepted_contracts(p, maps)
        samples = []
        for addr, row in contracts[: args.max_contracts]:
            blob = p.try_read(addr, OBJECT_READ_LEN)
            if len(blob) == OBJECT_READ_LEN:
                samples.append((addr, row, blob))
        scalar = summarize_scalar_offsets(samples, maps)
        pointers = summarize_pointer_offsets(p, samples)
        proof = {
            "schema": "market_trade_live_purchase_contract_field_census_v1",
            "utc": datetime.now(timezone.utc).isoformat(),
            "pid": pid,
            "module": {
                "path": fp.get("path"),
                "base": hx(p.module_base),
                "sha256": sha,
            },
            "roots": {
                "CPurchaseContract.vtable_rva": hex(mt.CPURCHASE_CONTRACT_VTABLE_RVA),
                "CEquipmentVariant.vtable_rva": hex(mt.VARIANT_VTABLE_RVA),
            },
            "contracts": {
                "accepted_contract_rows": len(contracts),
                "sampled_contract_rows": len(samples),
                "known_offsets": {hx(k): v for k, v in sorted(KNOWN_OFFSETS.items())},
                "examples": [
                    {
                        "addr": hx(addr),
                        "contract_id": row.contract_id,
                        "seller_tag_db_id": row.seller_tag_db_id,
                        "buyer_tag_db_id": row.buyer_tag_db_id,
                        "equipment_variant_id": row.equipment_variant_id,
                        "amount_raw": row.amount_raw,
                    }
                    for addr, row, _blob in samples[:16]
                ],
            },
            "scalar_census": scalar,
            "pointer_census": pointers,
            "classification": {
                "accepted_for_emission": False,
                "reason": (
                    "Offsets are structural candidates only. No speed/completed/"
                    "delivery/progress field is semantically accepted without a "
                    "same-tick save/UI fixture keyed by contract_id."
                ),
            },
            "exact_blocker": (
                "purchase_contract_speed_completed and delivery_route_progress "
                "need a same-tick fixture. Use this census to validate candidate "
                "offsets against known contract_id rows; do not emit these fields "
                "from the census alone."
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
    parser.add_argument("--max-contracts", type=int, default=96)
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args(argv)
    proof = probe(args)
    print(json.dumps(proof["classification"], indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
