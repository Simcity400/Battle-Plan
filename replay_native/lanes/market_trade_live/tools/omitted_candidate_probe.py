"""Current-build omitted-candidate classifier for market_trade_live.

This probe uses the lane-local CPurchaseContract root and the shared dynamic
D-tag country identity rule. It classifies every current vtable candidate into:

- emitted_complete: accepted purchase-contract row with all extractable fields
- omitted_not_partial: non-contract/invalid/duplicate candidate
- missing_extractable: candidate that should be captured but lacks a proven field

The output is proof-only JSON. It does not edit shared collector entrypoints.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

_HERE = Path(__file__).resolve()
_REPO = _HERE.parents[4]
sys.path.insert(0, str(_REPO / "memory_reader"))

import market_trade_live_compact as mt  # noqa: E402


DYNAMIC_TAG_RE = re.compile(r"^D\d{2}$")
OLD_16022851_PURCHASE_CONTRACT_RVA = 0x2944B80


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) and value else None


def country_maps_with_dynamic_fallback(p: mt.winmem.Process) -> dict:
    maps = mt.read_country_maps(p)
    id_to_country = dict(maps["id_to_country"])
    strict_count = len(id_to_country)
    country_vtable = p.module_base + mt.COUNTRY_VTABLE_RVA
    dynamic_fallbacks = []
    country_table = maps.get("country_table")
    limit = min(
        maps.get("country_count") or 0,
        maps.get("tag_count") or 0,
        mt.MAX_TAG_DB_ID,
    )
    if mt.is_heap(country_table):
        for tag_id in range(1, limit):
            tag = maps["id_to_tag"].get(tag_id)
            if not tag or not DYNAMIC_TAG_RE.match(tag):
                continue
            ptr = mt.safe_u64(p, country_table + tag_id * 8)
            obj_tag_id = mt.safe_u32(p, ptr + mt.OBJ_TAG_DB_ID) if mt.is_heap(ptr) else None
            if (
                mt.is_heap(ptr)
                and mt.safe_u64(p, ptr) == country_vtable
                and id_to_country.get(tag_id) != ptr
            ):
                id_to_country[tag_id] = ptr
                dynamic_fallbacks.append(
                    {
                        "tag_db_id": tag_id,
                        "tag": tag,
                        "country_ptr": hx(ptr),
                        "obj_tag_db_id": obj_tag_id,
                    }
                )
    maps["id_to_country_dynamic"] = id_to_country
    maps["strict_country_count"] = strict_count
    maps["dynamic_fallbacks"] = dynamic_fallbacks
    return maps


def classify_row(row: mt.MarketTradeRow | None, reason: str) -> tuple[str, str]:
    if row is None:
        if reason in mt.OMITTED_NOT_PARTIAL_REASONS:
            return "omitted_not_partial", reason
        return "missing_extractable", reason
    if not mt.is_complete_contract_row(row):
        return "missing_extractable", "candidate_partial_contract_fields"
    return "emitted_complete", "emitted_complete"


def row_example(row: mt.MarketTradeRow | None, reason: str, addr: int) -> dict:
    out = {"reason": reason, "addr": hx(addr)}
    if row is not None:
        out.update(
            {
                "contract_id": row.contract_id,
                "seller_tag_db_id": row.seller_tag_db_id,
                "buyer_tag_db_id": row.buyer_tag_db_id,
                "equipment_variant_id": row.equipment_variant_id,
                "amount_raw": row.amount_raw,
                "flags": row.flags,
                "equipment_subbuf_status": row.equipment_subbuf_status,
                "delivery_subbuf_status": row.delivery_subbuf_status,
                "buyer_ptr_status": row.buyer_ptr_status,
            }
        )
    return out


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
        contract_vtable = p.module_base + mt.CPURCHASE_CONTRACT_VTABLE_RVA
        variant_vtable = p.module_base + mt.VARIANT_VTABLE_RVA
        hits = mt.scan_contract_candidates(p)
        old_vtable = p.module_base + OLD_16022851_PURCHASE_CONTRACT_RVA
        old_hits = mt.scan_heap_vtables(p, [old_vtable]).get(old_vtable, [])

        buckets = Counter()
        reasons = Counter()
        examples = {"omitted_not_partial": [], "missing_extractable": []}
        rows_by_id: dict[int, mt.MarketTradeRow] = {}
        duplicates = Counter()
        for addr in hits:
            row, reason = mt.read_contract_candidate(
                p,
                addr,
                contract_vtable,
                variant_vtable,
                maps["id_to_country_dynamic"],
            )
            bucket, final_reason = classify_row(row, reason)
            if row is not None and bucket == "emitted_complete":
                old = rows_by_id.get(row.contract_id)
                if old is None:
                    rows_by_id[row.contract_id] = row
                elif old == row:
                    bucket = "omitted_not_partial"
                    final_reason = "candidate_duplicate_identical"
                    duplicates[final_reason] += 1
                else:
                    bucket = "missing_extractable"
                    final_reason = "candidate_duplicate_conflict"
                    duplicates[final_reason] += 1
            buckets[bucket] += 1
            if final_reason != "emitted_complete":
                reasons[final_reason] += 1
                if bucket in examples and len(examples[bucket]) < args.examples:
                    examples[bucket].append(row_example(row, final_reason, addr))

    proof = {
        "schema": "market_trade_live_omitted_candidate_probe_v1",
        "utc": datetime.now(timezone.utc).isoformat(),
        "pid": pid,
        "module": {
            "path": fp.get("path"),
            "base": hx(p.module_base),
            "sha256": sha,
        },
        "root": {
            "CPurchaseContract.vtable_rva": hex(mt.CPURCHASE_CONTRACT_VTABLE_RVA),
            "old_16022851_CPurchaseContract.vtable_rva": hex(
                OLD_16022851_PURCHASE_CONTRACT_RVA
            ),
            "CEquipmentVariant.vtable_rva": hex(mt.VARIANT_VTABLE_RVA),
        },
        "country_identity": {
            "strict_country_rows": maps["strict_country_count"],
            "dynamic_fallback_rows": len(maps["dynamic_fallbacks"]),
            "dynamic_fallbacks": maps["dynamic_fallbacks"],
        },
        "classification": {
            "candidate_rows": len(hits),
            "emitted_complete_rows": buckets["emitted_complete"],
            "omitted_not_partial_rows": buckets["omitted_not_partial"],
            "missing_extractable_rows": buckets["missing_extractable"],
            "reasons": dict(sorted(reasons.items())),
            "duplicate_reasons": dict(sorted(duplicates.items())),
            "missing_extractable_is_partial_failure": buckets["missing_extractable"] > 0,
            "omitted_candidates_are_partial_failure": False,
        },
        "rejected_candidates": {
            "old_16022851_vtable_hits": len(old_hits),
            "old_16022851_result": "rejected_for_870ebf"
            if len(old_hits) == 0
            else "not_rejected",
        },
        "examples": examples,
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
    parser.add_argument("--examples", type=int, default=12)
    parser.add_argument("--out-json")
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args(argv)
    proof = probe(args)
    print(json.dumps(proof["classification"], indent=2, sort_keys=True))
    return 0 if not proof["classification"]["missing_extractable_is_partial_failure"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
