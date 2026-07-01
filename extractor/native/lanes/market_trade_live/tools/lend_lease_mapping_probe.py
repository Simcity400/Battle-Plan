"""Bounded live proof for market_trade_live lend-lease mapping rows.

This is lane-local proof tooling, not a runtime Python loop. It scans the
current CPurchaseContract table once, derives one seller-outgoing and one
buyer-incoming mapping row per complete purchase contract, writes an H4CR
domain-43 stream with an MTLL trailer, and leaves full lend-lease delivery,
progress, stockpile, speed/completed, and diplomacy-context semantics blocked
until their own roots are validated.
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
from purchase_contract_detail_probe import read_contract_detail_candidate  # noqa: E402
from resource_flow_root_probe import country_maps_with_dynamic_fallback  # noqa: E402


DOMAIN_ID = 43
RECORD_TYPE_DOMAIN_DATA = 3
RECORD_TYPE_FRAME_END = 2

H4CR_HEADER = struct.Struct("<4sHHQiHHII")
V1_HEADER = struct.Struct("<QIIIIIIIIII")
CORE_ROW = struct.Struct("<IIIIIIqIIII")
MTLL_HEADER = struct.Struct("<4sHHHHIIIII")
MTLL_ROW = struct.Struct("<IIIIIIqqII")

MTLL_MAGIC = b"MTLL"
MTLL_VERSION = 1
MTLL_HEADER_LEN = MTLL_HEADER.size
MTLL_ROW_LEN = MTLL_ROW.size
SCALE_RAW = 100000

LEND_LEASE_DIRECTION_OUTGOING = 1
LEND_LEASE_DIRECTION_INCOMING = 2
LEND_LEASE_FLAG_AMOUNT_VALID = 1
LEND_LEASE_FLAG_PRICE_VALID = 1 << 1
LEND_LEASE_SOURCE_PURCHASE_CONTRACT = 1


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) and value else None


def complete_core_rows(frame: mt.MarketTradeFrame) -> list[mt.MarketTradeRow]:
    return [row for row in frame.rows if mt.is_complete_contract_row(row)]


def read_price_by_contract(
    p: mt.winmem.Process, hits: list[int], id_to_tag: dict[int, str]
) -> tuple[dict[int, int], Counter[str], int]:
    prices: dict[int, int] = {}
    reasons: Counter[str] = Counter()
    duplicate_conflicts = 0
    for addr in hits:
        detail, reason = read_contract_detail_candidate(p, addr, id_to_tag)
        reasons[reason] += 1
        if not detail or not detail.get("price_raw"):
            continue
        contract_id = int(detail["contract_id"])
        price_raw = int(detail["price_raw"])
        old = prices.get(contract_id)
        if old is None:
            prices[contract_id] = price_raw
        elif old != price_raw:
            duplicate_conflicts += 1
            reasons["price_duplicate_conflict"] += 1
    return prices, reasons, duplicate_conflicts


def mapping_rows_from_contracts(
    rows: list[mt.MarketTradeRow], price_by_contract: dict[int, int]
) -> list[dict]:
    mappings: list[dict] = []
    for row in rows:
        price_raw = int(price_by_contract.get(row.contract_id, 0) or 0)
        flags = 0
        if row.flags & mt.FLAG_AMOUNT_VALID and row.amount_raw > 0:
            flags |= LEND_LEASE_FLAG_AMOUNT_VALID
        if price_raw > 0:
            flags |= LEND_LEASE_FLAG_PRICE_VALID
        base = {
            "contract_id": int(row.contract_id),
            "equipment_variant_id": int(row.equipment_variant_id),
            "flags": flags,
            "amount_raw": int(row.amount_raw if flags & LEND_LEASE_FLAG_AMOUNT_VALID else 0),
            "price_raw": int(price_raw if flags & LEND_LEASE_FLAG_PRICE_VALID else 0),
            "source_record_kind": LEND_LEASE_SOURCE_PURCHASE_CONTRACT,
        }
        mappings.append(
            {
                **base,
                "perspective_tag_db_id": int(row.seller_tag_db_id),
                "counterpart_tag_db_id": int(row.buyer_tag_db_id),
                "direction": LEND_LEASE_DIRECTION_OUTGOING,
            }
        )
        mappings.append(
            {
                **base,
                "perspective_tag_db_id": int(row.buyer_tag_db_id),
                "counterpart_tag_db_id": int(row.seller_tag_db_id),
                "direction": LEND_LEASE_DIRECTION_INCOMING,
            }
        )
    return mappings


def mapping_invariants(rows: list[dict]) -> dict:
    by_contract: dict[int, Counter[int]] = defaultdict(Counter)
    seen: set[tuple[int, int, int]] = set()
    duplicate_keys = 0
    for row in rows:
        key = (
            int(row["contract_id"]),
            int(row["perspective_tag_db_id"]),
            int(row["direction"]),
        )
        if key in seen:
            duplicate_keys += 1
        seen.add(key)
        by_contract[int(row["contract_id"])][int(row["direction"])] += 1
    asymmetric = [
        {
            "contract_id": contract_id,
            "outgoing": counts[LEND_LEASE_DIRECTION_OUTGOING],
            "incoming": counts[LEND_LEASE_DIRECTION_INCOMING],
        }
        for contract_id, counts in sorted(by_contract.items())
        if counts[LEND_LEASE_DIRECTION_OUTGOING] != 1
        or counts[LEND_LEASE_DIRECTION_INCOMING] != 1
    ]
    return {
        "contracts": len(by_contract),
        "duplicate_keys": duplicate_keys,
        "asymmetric_contracts": asymmetric,
    }


def core_payload_header(frame: mt.MarketTradeFrame, rows: list[mt.MarketTradeRow]) -> bytes:
    return V1_HEADER.pack(
        int(frame.elapsed_wall_ms),
        len(rows),
        len(rows),
        int(frame.status_flags),
        int(frame.observed_contract_rows),
        len(rows),
        sum(1 for row in rows if row.flags & mt.FLAG_BUYER_PTR_VALID),
        sum(1 for row in rows if row.flags & mt.FLAG_VARIANT_VALID),
        sum(1 for row in rows if row.flags & mt.FLAG_AMOUNT_VALID),
        sum(1 for row in rows if not mt.is_complete_contract_row(row)),
        SCALE_RAW,
    )


def encode_core_rows(rows: list[mt.MarketTradeRow]) -> bytes:
    out = bytearray()
    for row in rows:
        out += CORE_ROW.pack(
            int(row.contract_id),
            int(row.contract_type),
            int(row.flags),
            int(row.seller_tag_db_id),
            int(row.buyer_tag_db_id),
            int(row.equipment_variant_id),
            int(row.amount_raw),
            int(row.equipment_subbuf_status),
            int(row.delivery_subbuf_status),
            int(row.buyer_ptr_status),
            0,
        )
    return bytes(out)


def encode_mtll_trailer(rows: list[dict]) -> bytes:
    outgoing = sum(1 for row in rows if row["direction"] == LEND_LEASE_DIRECTION_OUTGOING)
    incoming = sum(1 for row in rows if row["direction"] == LEND_LEASE_DIRECTION_INCOMING)
    amount_valid = sum(1 for row in rows if row["flags"] & LEND_LEASE_FLAG_AMOUNT_VALID)
    price_valid = sum(1 for row in rows if row["flags"] & LEND_LEASE_FLAG_PRICE_VALID)
    out = bytearray()
    out += MTLL_HEADER.pack(
        MTLL_MAGIC,
        MTLL_VERSION,
        MTLL_HEADER_LEN,
        MTLL_ROW_LEN,
        0,
        len(rows),
        outgoing,
        incoming,
        amount_valid,
        price_valid,
    )
    for row in rows:
        out += MTLL_ROW.pack(
            int(row["contract_id"]),
            int(row["perspective_tag_db_id"]),
            int(row["counterpart_tag_db_id"]),
            int(row["direction"]),
            int(row["equipment_variant_id"]),
            int(row["flags"]),
            int(row["amount_raw"]),
            int(row["price_raw"]),
            int(row["source_record_kind"]),
            0,
        )
    return bytes(out)


def encode_domain43_payload(
    frame: mt.MarketTradeFrame, rows: list[mt.MarketTradeRow], mappings: list[dict]
) -> bytes:
    return core_payload_header(frame, rows) + encode_core_rows(rows) + encode_mtll_trailer(mappings)


def append_h4cr(
    out: bytearray,
    sequence: int,
    game_hour: int,
    domain: int,
    record_type: int,
    payload: bytes,
) -> None:
    out += H4CR_HEADER.pack(
        b"H4CR",
        1,
        H4CR_HEADER.size,
        sequence,
        game_hour,
        domain,
        record_type,
        len(payload),
        0,
    )
    out += payload


def encode_stream(frame: mt.MarketTradeFrame, rows: list[mt.MarketTradeRow], mappings: list[dict]) -> bytes:
    out = bytearray()
    payload = encode_domain43_payload(frame, rows, mappings)
    append_h4cr(out, 1, int(frame.game_hour_raw or 0), DOMAIN_ID, RECORD_TYPE_DOMAIN_DATA, payload)
    append_h4cr(out, 2, int(frame.game_hour_raw or 0), 0, RECORD_TYPE_FRAME_END, b"")
    return bytes(out)


def example_mapping(row: dict, id_to_tag: dict[int, str]) -> dict:
    return {
        "contract_id": row["contract_id"],
        "perspective_tag_db_id": row["perspective_tag_db_id"],
        "perspective": id_to_tag.get(row["perspective_tag_db_id"]),
        "counterpart_tag_db_id": row["counterpart_tag_db_id"],
        "counterpart": id_to_tag.get(row["counterpart_tag_db_id"]),
        "direction": "outgoing"
        if row["direction"] == LEND_LEASE_DIRECTION_OUTGOING
        else "incoming",
        "equipment_variant_id": row["equipment_variant_id"],
        "amount_raw": row["amount_raw"],
        "price_raw": row["price_raw"],
        "flags": row["flags"],
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
        id_to_tag = maps["id_to_tag"]
        hits = mt.scan_contract_candidates(p)
        game_hour = mt.read_game_hour_raw(p) or 0
        frame = mt.extract_frame_from_candidates(
            p,
            int((time.perf_counter() - started) * 1000),
            game_hour,
            hits,
            maps["id_to_country_dynamic"],
        )
        rows = complete_core_rows(frame)
        price_by_contract, price_reasons, price_duplicate_conflicts = read_price_by_contract(
            p, hits, id_to_tag
        )
        mappings = mapping_rows_from_contracts(rows, price_by_contract)
        stream = encode_stream(frame, rows, mappings)
        module_base = p.module_base

    if args.out_bin:
        os.makedirs(os.path.dirname(os.path.abspath(args.out_bin)), exist_ok=True)
        with open(args.out_bin, "wb") as fh:
            fh.write(stream)

    invariants = mapping_invariants(mappings)
    price_valid_rows = sum(1 for row in mappings if row["flags"] & LEND_LEASE_FLAG_PRICE_VALID)
    amount_valid_rows = sum(1 for row in mappings if row["flags"] & LEND_LEASE_FLAG_AMOUNT_VALID)
    outgoing_rows = sum(1 for row in mappings if row["direction"] == LEND_LEASE_DIRECTION_OUTGOING)
    incoming_rows = sum(1 for row in mappings if row["direction"] == LEND_LEASE_DIRECTION_INCOMING)
    structural_failures = frame.structural_partial_rows + frame.partial_field_rows
    mapping_failures = len(invariants["asymmetric_contracts"]) + invariants["duplicate_keys"]
    proof = {
        "schema": "market_trade_live_lend_lease_mapping_probe_v1",
        "utc": datetime.now(timezone.utc).isoformat(),
        "pid": pid,
        "module": {
            "path": fp.get("path"),
            "base": hx(module_base),
            "sha256": sha,
        },
        "game_hour_raw": game_hour,
        "root": {
            "source": "derived from validated CPurchaseContract rows",
            "CPurchaseContract.vtable_rva": hex(mt.CPURCHASE_CONTRACT_VTABLE_RVA),
            "CEquipmentVariant.vtable_rva": hex(mt.VARIANT_VTABLE_RVA),
            "contract_fields": {
                "contract_id": "CPurchaseContract+0x0C",
                "seller_tag_db_id": "CPurchaseContract+0x58",
                "buyer_tag_db_id": "CPurchaseContract+0x5C",
                "equipment_variant_id": "CPurchaseContract+0x38 equipment_subbuf -> CEquipmentVariant",
                "amount_raw": "CPurchaseContract+0x80 delivery_subbuf -> CEquipmentVariant payload i32 amount",
                "price_raw": "CPurchaseContract+0x38 equipment_subbuf -> CEquipmentVariant payload i32 amount",
            },
        },
        "compact": {
            "domain_id": DOMAIN_ID,
            "mtll_magic": MTLL_MAGIC.decode("ascii"),
            "mtll_version": MTLL_VERSION,
            "mtll_header_len": MTLL_HEADER_LEN,
            "mtll_row_len": MTLL_ROW_LEN,
            "stream_bytes": len(stream),
        },
        "classification": {
            "candidate_rows": len(hits),
            "core_rows": len(rows),
            "observed_contract_rows": frame.observed_contract_rows,
            "omitted_candidate_rows": frame.omitted_candidate_rows,
            "structural_partial_rows": frame.structural_partial_rows,
            "partial_field_rows": frame.partial_field_rows,
            "mapping_rows": len(mappings),
            "outgoing_rows": outgoing_rows,
            "incoming_rows": incoming_rows,
            "amount_valid_rows": amount_valid_rows,
            "price_valid_rows": price_valid_rows,
            "price_contract_rows": len(price_by_contract),
            "price_duplicate_conflicts": price_duplicate_conflicts,
            "mapping_duplicate_keys": invariants["duplicate_keys"],
            "asymmetric_contract_rows": len(invariants["asymmetric_contracts"]),
            "all_core_contracts_mapped_twice": len(mappings) == len(rows) * 2,
            "all_mapped_contracts_symmetric": mapping_failures == 0,
            "source_or_partial_failures": structural_failures,
            "worker_or_bridge_failures": 0,
            "reasons": dict(sorted((frame.omission_reasons or {}).items())),
            "price_reasons": dict(sorted(price_reasons.items())),
        },
        "exact_remaining_blockers": {
            "delivery_route_progress": "MTLL does not validate convoy route, delivery country, efficiency, days, factory progress, or equipment progress roots.",
            "speed_completed": "MTLL does not validate purchase-contract speed or completed counters.",
            "stockpile_automation": "MTLL does not validate market stockpile, sellable equipment, price levels, subsidies, or automation roots.",
            "diplomatic_context": "MTLL is tag-pair keyed; market-access/equipment-purchase relation context still needs the diplomacy-domain join registration.",
            "full_lend_lease": "MTLL proves directed market-contract mapping only; full lend-lease replay still needs the remaining fields above.",
        },
        "mapping_invariants": invariants,
        "examples": [example_mapping(row, id_to_tag) for row in mappings[: args.examples]],
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
    parser.add_argument("--out-bin")
    parser.add_argument("--out-json")
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args(argv)
    proof = probe(args)
    classification = proof["classification"]
    print(json.dumps(classification, indent=2, sort_keys=True))
    ok = (
        classification["source_or_partial_failures"] == 0
        and classification["worker_or_bridge_failures"] == 0
        and classification["mapping_rows"] > 0
        and classification["all_core_contracts_mapped_twice"]
        and classification["all_mapped_contracts_symmetric"]
        and classification["mapping_duplicate_keys"] == 0
    )
    return 0 if ok else 2


if __name__ == "__main__":
    raise SystemExit(main())
