"""Bounded read-only probe for the market_trade_live recording lane.

This probe does not modify the game and does not depend on save parsing. It
derives the current-build CPurchaseContract vtable from MSVC RTTI, scans the
heap for live contract objects, and validates the production field path for the
contract id, seller/buyer tag ids, buyer country pointer, and type-70 equipment
variant payloads.
"""

from __future__ import annotations

import argparse
import json
import os
import struct
import sys
import time
from datetime import datetime, timezone

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)

import winmem  # noqa: E402


PROC_NAME = "hoi4.exe"
EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

CG_STATE_DATE_OFFSET = 0x3316E90
COUNTRY_DATABASE_OFFSET = 0x33189C8
CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C
COUNTRY_VTABLE_RVA = 0x27D0E90
OBJ_TAG_DB_ID = 0x08

VARIANT_VTABLE_RVA = 0x293AFB8
VARIANT_TYPE = 70
VARIANT_TYPE_OFF = 0x08
VARIANT_ID_OFF = 0x0C

CPURCHASE_CONTRACT_RTTI = b".?AVCPurchaseContract@NInternationalMarket@@"
OLD_16022851_PURCHASE_CONTRACT_RVA = 0x2944B80

PC_ID_TYPE = 0x08
PC_ID_ID = 0x0C
PC_EQUIPMENT_SUBBUF = 0x38
PC_SELLER_TAGID = 0x58
PC_BUYER_TAGID = 0x5C
PC_DELIVERY_SUBBUF = 0x80
PC_BUYER_PTR = 0x128

FP = 100000.0
MAX_TAG_ROWS = 2048


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) and value else None


def is_heap(value: int | None, align: int = 8) -> bool:
    return isinstance(value, int) and HEAP_LO <= value < HEAP_HI and value % align == 0


def safe_u64(p: winmem.Process, addr: int) -> int | None:
    try:
        return p.u64(addr)
    except OSError:
        return None


def safe_u32(p: winmem.Process, addr: int) -> int | None:
    try:
        return p.u32(addr)
    except OSError:
        return None


def read_inline_tag(p: winmem.Process, addr: int) -> str | None:
    raw = p.try_read(addr, 4)
    if len(raw) < 3:
        return None
    end = raw.find(b"\0")
    if end < 0:
        end = 3
    try:
        tag = raw[:end].decode("latin-1")
    except UnicodeDecodeError:
        return None
    return tag if tag and all(32 <= ord(ch) < 127 for ch in tag) else None


def read_module_chunks(p: winmem.Process) -> list[tuple[int, bytes]]:
    chunks: list[tuple[int, bytes]] = []
    end = p.module_base + p.module_size
    for base, size in p.iter_regions(p.module_base, end, want_exec=None):
        blob = p.try_read(base, size)
        if blob:
            chunks.append((base, blob))
    return chunks


def derive_rtti_vtables(p: winmem.Process, chunks: list[tuple[int, bytes]], name: bytes):
    module_start = p.module_base
    module_end = p.module_base + p.module_size
    string_addrs: list[int] = []
    for base, blob in chunks:
        pos = 0
        while True:
            idx = blob.find(name, pos)
            if idx < 0:
                break
            string_addrs.append(base + idx)
            pos = idx + 1

    col_hits = []
    vtables = []
    for string_addr in string_addrs:
        type_descriptor = string_addr - 0x10
        type_descriptor_rva = type_descriptor - module_start
        if not (module_start <= type_descriptor < module_end):
            continue
        pattern = struct.pack("<I", type_descriptor_rva)
        for base, blob in chunks:
            pos = 0
            while True:
                idx = blob.find(pattern, pos)
                if idx < 0:
                    break
                col = base + idx - 0x0C
                raw = p.try_read(col, 24)
                if len(raw) >= 24:
                    sig, off, cd_off, td_ref, class_ref, self_ref = struct.unpack(
                        "<IIIIII", raw[:24]
                    )
                    if (
                        sig in (0, 1)
                        and td_ref == type_descriptor_rva
                        and self_ref == col - module_start
                    ):
                        col_hits.append(
                            {
                                "string_addr": string_addr,
                                "type_descriptor": type_descriptor,
                                "complete_object_locator": col,
                                "signature": sig,
                                "offset": off,
                                "cd_offset": cd_off,
                                "class_descriptor_rva": class_ref,
                            }
                        )
                pos = idx + 1

    for hit in col_hits:
        col_ptr = struct.pack("<Q", hit["complete_object_locator"])
        for base, blob in chunks:
            pos = 0
            while True:
                idx = blob.find(col_ptr, pos)
                if idx < 0:
                    break
                vtable = base + idx + 8
                if vtable not in vtables:
                    vtables.append(vtable)
                pos = idx + 1
    return string_addrs, col_hits, vtables


def read_country_maps(p: winmem.Process):
    game_state = safe_u64(p, p.module_base + CG_STATE_DATE_OFFSET)
    country_database = safe_u64(p, p.module_base + COUNTRY_DATABASE_OFFSET)
    tag_table = safe_u64(p, country_database) if is_heap(country_database) else None
    tag_count = safe_u32(p, country_database + 8) if is_heap(country_database) else None
    active_count = safe_u32(p, country_database + 12) if is_heap(country_database) else None
    country_table = safe_u64(p, game_state + CGAMESTATE_COUNTRY_TABLE) if is_heap(game_state) else None
    country_count = safe_u32(p, game_state + CGAMESTATE_COUNTRY_COUNT) if is_heap(game_state) else None

    id_to_tag: dict[int, str] = {}
    id_to_country: dict[int, int] = {}
    country_vtable = p.module_base + COUNTRY_VTABLE_RVA
    rows = min(tag_count or 0, active_count or 0, MAX_TAG_ROWS)
    if is_heap(tag_table):
        for tag_id in range(rows):
            tag = read_inline_tag(p, tag_table + tag_id * 32)
            if tag:
                id_to_tag[tag_id] = tag
    if is_heap(country_table):
        limit = min(country_count or rows, rows, MAX_TAG_ROWS)
        for tag_id in range(1, limit):
            country = safe_u64(p, country_table + tag_id * 8)
            if (
                is_heap(country)
                and safe_u64(p, country) == country_vtable
                and safe_u32(p, country + OBJ_TAG_DB_ID) == tag_id
            ):
                id_to_country[tag_id] = country

    return {
        "game_state": game_state,
        "country_database": country_database,
        "tag_table": tag_table,
        "tag_count": tag_count,
        "active_count": active_count,
        "country_table": country_table,
        "country_count": country_count,
        "id_to_tag": id_to_tag,
        "id_to_country": id_to_country,
    }


def scan_heap_vtables(p: winmem.Process, vtables: list[int]) -> dict[int, list[int]]:
    needles = {vt: struct.pack("<Q", vt) for vt in vtables}
    out = {vt: [] for vt in vtables}
    chunk = 64 << 20
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        pos = 0
        while pos < size:
            want = min(chunk, size - pos)
            blob = p.try_read(base + pos, want)
            if blob:
                for vt, needle in needles.items():
                    start = 0
                    while True:
                        idx = blob.find(needle, start)
                        if idx < 0:
                            break
                        addr = base + pos + idx
                        if addr % 8 == 0:
                            out[vt].append(addr)
                        start = idx + 1
                got = len(blob)
                if pos + got >= size or got < want:
                    pos += max(got, 0x1000)
                else:
                    pos += got - 16
            else:
                pos += max(want, 0x1000)
    return out


def scan_variant_in_buffer(p: winmem.Process, ptr: int, variant_vtable: int, span: int = 0x80):
    if not is_heap(ptr):
        return None, None
    blob = p.try_read(ptr, span)
    if len(blob) < 0x18:
        return None, None
    for off in range(0, len(blob) - 8, 8):
        qword = struct.unpack_from("<Q", blob, off)[0]
        if not is_heap(qword):
            continue
        if safe_u64(p, qword) != variant_vtable:
            continue
        if safe_u32(p, qword + VARIANT_TYPE_OFF) != VARIANT_TYPE:
            continue
        variant_id = safe_u32(p, qword + VARIANT_ID_OFF)
        amount = None
        for amount_off in range(off + 8, min(off + 0x18, len(blob) - 3), 4):
            raw = struct.unpack_from("<i", blob, amount_off)[0]
            if raw != 0:
                amount = raw / FP
                break
        return variant_id, amount
    return None, None


def read_contract(p: winmem.Process, addr: int, country_maps: dict, variant_vtable: int):
    id_type = safe_u32(p, addr + PC_ID_TYPE)
    contract_id = safe_u32(p, addr + PC_ID_ID)
    seller_id = safe_u32(p, addr + PC_SELLER_TAGID)
    buyer_id = safe_u32(p, addr + PC_BUYER_TAGID)
    buyer_ptr = safe_u64(p, addr + PC_BUYER_PTR)
    id_to_tag = country_maps["id_to_tag"]
    id_to_country = country_maps["id_to_country"]
    seller = id_to_tag.get(seller_id)
    buyer = id_to_tag.get(buyer_id)
    buyer_ptr_expected = id_to_country.get(buyer_id)
    buyer_ptr_matches = bool(buyer_ptr_expected and buyer_ptr == buyer_ptr_expected)

    equipment_variant_id = price = delivery_variant_id = amount = None
    eq_subbuf = safe_u64(p, addr + PC_EQUIPMENT_SUBBUF)
    if is_heap(eq_subbuf):
        equipment_variant_id, price = scan_variant_in_buffer(p, eq_subbuf, variant_vtable)
    delivery_subbuf = safe_u64(p, addr + PC_DELIVERY_SUBBUF)
    if is_heap(delivery_subbuf):
        delivery_variant_id, amount = scan_variant_in_buffer(p, delivery_subbuf, variant_vtable)
    if equipment_variant_id is None:
        equipment_variant_id = delivery_variant_id

    return {
        "addr": addr,
        "contract_type": id_type,
        "contract_id": contract_id,
        "seller_tag_db_id": seller_id,
        "seller": seller,
        "buyer_tag_db_id": buyer_id,
        "buyer": buyer,
        "buyer_ptr": buyer_ptr,
        "buyer_ptr_matches": buyer_ptr_matches,
        "equip_variant_id": equipment_variant_id,
        "delivery_variant_id": delivery_variant_id,
        "amount": amount,
        "price": price,
    }


def probe(args):
    with winmem.Process(args.pid, PROC_NAME) as p:
        fp = p.fingerprint()
        sha = str(fp.get("sha256") or "").lower()
        if sha != EXPECTED_SHA256 and not args.force:
            raise SystemExit(
                "live hoi4.exe sha256 %s != expected %s; pass --force to probe anyway"
                % (sha, EXPECTED_SHA256)
            )

        chunks = read_module_chunks(p)
        string_addrs, col_hits, vtables = derive_rtti_vtables(p, chunks, CPURCHASE_CONTRACT_RTTI)
        if not vtables:
            raise SystemExit("CPurchaseContract RTTI found no vtable")
        old_vtable = p.module_base + OLD_16022851_PURCHASE_CONTRACT_RVA
        scan_targets = sorted(set(vtables + [old_vtable]))
        heap_hits = scan_heap_vtables(p, scan_targets)
        country_maps = read_country_maps(p)
        variant_vtable = p.module_base + VARIANT_VTABLE_RVA

        selected_vtable = vtables[0]
        contracts = [
            read_contract(p, addr, country_maps, variant_vtable)
            for addr in heap_hits.get(selected_vtable, [])
        ]
        valid_contracts = [
            row
            for row in contracts
            if row["contract_type"] == 81
            and row["contract_id"]
            and row["seller"]
            and row["buyer"]
            and row["buyer_ptr_matches"]
        ]
        variant_decoded = sum(1 for row in valid_contracts if row["equip_variant_id"] is not None)
        amount_decoded = sum(1 for row in valid_contracts if row["amount"] is not None)
        price_decoded = sum(1 for row in valid_contracts if row["price"] is not None)

        return {
            "schema": "market_trade_live_probe_v1",
            "utc": datetime.now(timezone.utc).isoformat(),
            "pid": args.pid,
            "module": {
                "path": fp.get("path"),
                "base": hx(p.module_base),
                "size": hx(p.module_size),
                "sha256": sha,
            },
            "rtti": {
                "decorated_name": CPURCHASE_CONTRACT_RTTI.decode("ascii"),
                "string_matches": [hx(value) for value in string_addrs],
                "complete_object_locators": [
                    {
                        "string_addr": hx(hit["string_addr"]),
                        "type_descriptor": hx(hit["type_descriptor"]),
                        "complete_object_locator": hx(hit["complete_object_locator"]),
                        "class_descriptor_rva": hx(hit["class_descriptor_rva"]),
                    }
                    for hit in col_hits
                ],
                "derived_vtables": [
                    {"addr": hx(value), "rva": hx(value - p.module_base)} for value in vtables
                ],
            },
            "roots": {
                "country_database": hx(country_maps["country_database"]),
                "game_state": hx(country_maps["game_state"]),
                "tag_table": hx(country_maps["tag_table"]),
                "tag_count": country_maps["tag_count"],
                "active_count": country_maps["active_count"],
                "country_table": hx(country_maps["country_table"]),
                "country_count": country_maps["country_count"],
                "resolved_tags": len(country_maps["id_to_tag"]),
                "resolved_country_objects": len(country_maps["id_to_country"]),
                "variant_vtable": hx(variant_vtable),
                "variant_vtable_rva": hx(VARIANT_VTABLE_RVA),
            },
            "candidate_results": {
                "current_rtti_cpurchase_contract_vtable": {
                    "vtable": hx(selected_vtable),
                    "vtable_rva": hx(selected_vtable - p.module_base),
                    "heap_instances": len(heap_hits.get(selected_vtable, [])),
                    "contract_type_81": sum(1 for row in contracts if row["contract_type"] == 81),
                    "seller_resolved": sum(1 for row in contracts if row["seller"]),
                    "buyer_resolved": sum(1 for row in contracts if row["buyer"]),
                    "buyer_ptr_matches": sum(1 for row in contracts if row["buyer_ptr_matches"]),
                    "valid_contract_rows": len(valid_contracts),
                    "equipment_variant_decoded": variant_decoded,
                    "amount_decoded": amount_decoded,
                    "price_decoded": price_decoded,
                },
                "old_16022851_cpurchase_contract_vtable_rva": {
                    "vtable": hx(old_vtable),
                    "vtable_rva": hx(OLD_16022851_PURCHASE_CONTRACT_RVA),
                    "heap_instances": len(heap_hits.get(old_vtable, [])),
                    "result": "rejected_for_870ebf"
                    if len(heap_hits.get(old_vtable, [])) == 0
                    else "not_rejected",
                },
            },
            "validated_offsets": {
                "CPurchaseContract.vtable_rva": hx(selected_vtable - p.module_base),
                "CPurchaseContract.id_type": hx(PC_ID_TYPE),
                "CPurchaseContract.id_id": hx(PC_ID_ID),
                "CPurchaseContract.seller_tag_db_id": hx(PC_SELLER_TAGID),
                "CPurchaseContract.buyer_tag_db_id": hx(PC_BUYER_TAGID),
                "CPurchaseContract.buyer_ptr": hx(PC_BUYER_PTR),
                "CPurchaseContract.equipment_subbuf": hx(PC_EQUIPMENT_SUBBUF),
                "CPurchaseContract.delivery_subbuf": hx(PC_DELIVERY_SUBBUF),
            },
            "examples": [
                {
                    key: (hx(value) if key in ("addr", "buyer_ptr") else value)
                    for key, value in row.items()
                }
                for row in sorted(valid_contracts, key=lambda item: item["contract_id"] or 0)[
                    : args.examples
                ]
            ],
        }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description="Read-only market trade live root probe")
    parser.add_argument("--pid", type=int, default=None)
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--examples", type=int, default=16)
    parser.add_argument("--out", default=None)
    args = parser.parse_args(argv)
    if args.pid is None:
        args.pid = winmem.find_pid(PROC_NAME)
    if not args.pid:
        raise SystemExit("hoi4.exe PID not found")
    started = time.perf_counter()
    summary = probe(args)
    summary["elapsed_ms"] = round((time.perf_counter() - started) * 1000, 3)
    if args.out:
        os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
        with open(args.out, "w", encoding="utf-8") as fh:
            json.dump(summary, fh, indent=2, sort_keys=True)
            fh.write("\n")
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
