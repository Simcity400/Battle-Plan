"""Bounded live probe for market_trade_live purchase-contract detail rows.

This is lane-local proof tooling, not a Python hot loop. It scans the current
CPurchaseContract vtable once, extracts the already reachable price/amount
subbuffer fields, and writes an H4CR domain-43 stream carrying an MTD1 detail
trailer for Rust gate verification.
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


DOMAIN_ID = 43
RECORD_TYPE_DOMAIN_DATA = 3
RECORD_TYPE_FRAME_END = 2
H4CR_HEADER = struct.Struct("<4sHHQiHHII")
V1_HEADER = struct.Struct("<QIIIIIIIIII")
DETAIL_HEADER = struct.Struct("<4sHHHHIIIII")
DETAIL_ROW = struct.Struct("<IIqqiiiIiiiiiI")

DETAIL_MAGIC = b"MTD1"
DETAIL_VERSION = 1
DETAIL_HEADER_LEN = DETAIL_HEADER.size
DETAIL_ROW_LEN = DETAIL_ROW.size
SCALE_RAW = 100000

DETAIL_FLAG_PRICE_VALID = 1
DETAIL_FLAG_AMOUNT_VALID = 1 << 1


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) and value else None


def read_contract_detail_candidate(p: mt.winmem.Process, addr: int, id_to_tag: dict[int, str]):
    contract_vtable = p.module_base + mt.CPURCHASE_CONTRACT_VTABLE_RVA
    variant_vtable = p.module_base + mt.VARIANT_VTABLE_RVA
    blob = p.try_read(addr, mt.PC_BUYER_PTR + 8)
    if len(blob) < mt.PC_BUYER_PTR + 8:
        return None, "candidate_contract_unreadable"
    if struct.unpack_from("<Q", blob, 0)[0] != contract_vtable:
        return None, "candidate_vtable_invalid"

    contract_type = struct.unpack_from("<I", blob, mt.PC_ID_TYPE)[0]
    contract_id = struct.unpack_from("<I", blob, mt.PC_ID_ID)[0]
    seller_id = struct.unpack_from("<I", blob, mt.PC_SELLER_TAGID)[0]
    buyer_id = struct.unpack_from("<I", blob, mt.PC_BUYER_TAGID)[0]
    if contract_type != mt.CPURCHASE_CONTRACT_TYPE:
        return None, "candidate_type_invalid"
    if not contract_id or contract_id > mt.MAX_CONTRACT_ID:
        return None, "candidate_contract_id_invalid"
    if not seller_id or seller_id > mt.MAX_TAG_DB_ID:
        return None, "candidate_seller_tag_invalid"
    if not buyer_id or buyer_id > mt.MAX_TAG_DB_ID:
        return None, "candidate_buyer_tag_invalid"
    if seller_id not in id_to_tag:
        return None, "candidate_seller_tag_unresolved"
    if buyer_id not in id_to_tag:
        return None, "candidate_buyer_tag_unresolved"

    equipment_subbuf = struct.unpack_from("<Q", blob, mt.PC_EQUIPMENT_SUBBUF)[0]
    eq_status, eq_variant_id, price_raw = mt.scan_variant_subbuffer(
        p, equipment_subbuf, variant_vtable
    )
    delivery_subbuf = struct.unpack_from("<Q", blob, mt.PC_DELIVERY_SUBBUF)[0]
    delivery_status, delivery_variant_id, amount_raw = mt.scan_variant_subbuffer(
        p, delivery_subbuf, variant_vtable
    )
    flags = 0
    if price_raw is not None:
        flags |= DETAIL_FLAG_PRICE_VALID
    if amount_raw is not None:
        flags |= DETAIL_FLAG_AMOUNT_VALID

    row = {
        "contract_id": contract_id,
        "flags": flags,
        "price_raw": int(price_raw or 0),
        "amount_raw": int(amount_raw or 0),
        "seller_tag_db_id": seller_id,
        "seller": id_to_tag.get(seller_id),
        "buyer_tag_db_id": buyer_id,
        "buyer": id_to_tag.get(buyer_id),
        "equipment_subbuf_status": eq_status,
        "delivery_subbuf_status": delivery_status,
        "equipment_variant_id": eq_variant_id,
        "delivery_variant_id": delivery_variant_id,
        "addr": addr,
    }
    if price_raw is None:
        return row, "candidate_price_missing"
    if amount_raw is None:
        return row, "candidate_delivery_amount_missing"
    return row, "emitted_detail"


def detail_row_to_bytes(row: dict) -> bytes:
    return DETAIL_ROW.pack(
        row["contract_id"],
        row["flags"],
        row["price_raw"],
        row["amount_raw"],
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    )


def encode_detail_trailer(rows: list[dict]) -> bytes:
    price_valid = sum(1 for row in rows if row["flags"] & DETAIL_FLAG_PRICE_VALID)
    speed_valid = 0
    route_valid = 0
    progress_valid = 0
    out = bytearray()
    out += DETAIL_HEADER.pack(
        DETAIL_MAGIC,
        DETAIL_VERSION,
        DETAIL_HEADER_LEN,
        DETAIL_ROW_LEN,
        0,
        len(rows),
        price_valid,
        speed_valid,
        route_valid,
        progress_valid,
    )
    for row in rows:
        out += detail_row_to_bytes(row)
    return bytes(out)


def encode_domain43_payload(rows: list[dict]) -> bytes:
    payload = bytearray()
    payload += V1_HEADER.pack(
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        SCALE_RAW,
    )
    payload += encode_detail_trailer(rows)
    return bytes(payload)


def append_h4cr(out: bytearray, sequence: int, game_hour: int, domain: int, record_type: int, payload: bytes):
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


def encode_stream(game_hour: int, rows: list[dict]) -> bytes:
    out = bytearray()
    append_h4cr(out, 1, game_hour, DOMAIN_ID, RECORD_TYPE_DOMAIN_DATA, encode_domain43_payload(rows))
    append_h4cr(out, 2, game_hour, 0, RECORD_TYPE_FRAME_END, b"")
    return bytes(out)


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
        game_hour = mt.read_game_hour_raw(p)
        hits = mt.scan_contract_candidates(p)
        country_maps = mt.read_country_maps(p)
        id_to_tag = country_maps["id_to_tag"]

        detail_by_id: dict[int, dict] = {}
        reasons = Counter()
        duplicate_conflicts = 0
        candidate_examples = []
        for addr in hits:
            row, reason = read_contract_detail_candidate(p, addr, id_to_tag)
            reasons[reason] += 1
            if row is None:
                continue
            if reason != "emitted_detail" and len(candidate_examples) < args.examples:
                candidate_examples.append(example_row(row, reason))
            old = detail_by_id.get(row["contract_id"])
            if old is None:
                detail_by_id[row["contract_id"]] = row
            elif old != row:
                duplicate_conflicts += 1
                reasons["candidate_duplicate_conflict"] += 1
        rows = [detail_by_id[key] for key in sorted(detail_by_id)]
        stream = encode_stream(game_hour or 0, rows)

    if args.out_bin:
        os.makedirs(os.path.dirname(os.path.abspath(args.out_bin)), exist_ok=True)
        with open(args.out_bin, "wb") as fh:
            fh.write(stream)

    proof = {
        "schema": "market_trade_live_purchase_contract_detail_probe_v1",
        "utc": datetime.now(timezone.utc).isoformat(),
        "pid": pid,
        "module": {
            "path": fp.get("path"),
            "base": hx(p.module_base),
            "sha256": sha,
        },
        "game_hour_raw": game_hour,
        "root": {
            "CPurchaseContract.vtable_rva": hex(mt.CPURCHASE_CONTRACT_VTABLE_RVA),
            "CEquipmentVariant.vtable_rva": hex(mt.VARIANT_VTABLE_RVA),
            "price_source": "CPurchaseContract+0x38 equipment_subbuf -> CEquipmentVariant payload i32 amount",
            "amount_source": "CPurchaseContract+0x80 delivery_subbuf -> CEquipmentVariant payload i32 amount",
        },
        "compact": {
            "domain_id": DOMAIN_ID,
            "detail_magic": DETAIL_MAGIC.decode("ascii"),
            "detail_version": DETAIL_VERSION,
            "detail_header_len": DETAIL_HEADER_LEN,
            "detail_row_len": DETAIL_ROW_LEN,
            "stream_bytes": len(stream),
        },
        "candidate_classification": {
            "candidate_rows": len(hits),
            "detail_rows": len(rows),
            "price_valid_rows": sum(1 for row in rows if row["flags"] & DETAIL_FLAG_PRICE_VALID),
            "amount_valid_rows": sum(1 for row in rows if row["flags"] & DETAIL_FLAG_AMOUNT_VALID),
            "speed_valid_rows": 0,
            "delivery_route_valid_rows": 0,
            "progress_valid_rows": 0,
            "duplicate_conflicts": duplicate_conflicts,
            "reasons": dict(sorted(reasons.items())),
        },
        "exact_blockers": {
            "speed": "No accepted CPurchaseContract speed offset. Requires same-tick contract fixture or UI/save oracle values joined by contract_id before emission.",
            "completed": "No accepted contract_meta.completed live field. Requires same-tick contract fixture or save-derived value joined by contract_id before emission.",
            "delivery_route": "No accepted live delivery_route_handler root for delivery country, convoys, efficiency, or days.",
            "delivery_progress": "No accepted live contract_delivery_state root for factory_cic_progress or equipment_transfer_progress.",
        },
        "examples": [example_row(row, "emitted_detail") for row in rows[: args.examples]],
        "candidate_examples": candidate_examples,
        "elapsed_ms": round((time.perf_counter() - started) * 1000, 3),
    }
    if args.out_json:
        os.makedirs(os.path.dirname(os.path.abspath(args.out_json)), exist_ok=True)
        with open(args.out_json, "w", encoding="utf-8") as fh:
            json.dump(proof, fh, indent=2, sort_keys=True)
            fh.write("\n")
    return proof


def example_row(row: dict, reason: str) -> dict:
    return {
        "reason": reason,
        "addr": hx(row["addr"]),
        "contract_id": row["contract_id"],
        "seller_tag_db_id": row["seller_tag_db_id"],
        "seller": row["seller"],
        "buyer_tag_db_id": row["buyer_tag_db_id"],
        "buyer": row["buyer"],
        "price_raw": row["price_raw"],
        "amount_raw": row["amount_raw"],
        "equipment_variant_id": row["equipment_variant_id"],
        "delivery_variant_id": row["delivery_variant_id"],
        "equipment_subbuf_status": row["equipment_subbuf_status"],
        "delivery_subbuf_status": row["delivery_subbuf_status"],
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int)
    parser.add_argument("--examples", type=int, default=12)
    parser.add_argument("--out-bin")
    parser.add_argument("--out-json")
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args(argv)
    proof = probe(args)
    print(json.dumps(proof["candidate_classification"], indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
