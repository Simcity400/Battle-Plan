"""Same-tick purchase-contract detail root correlator.

This is bounded proof tooling. It joins live CPurchaseContract rows to a
same-tick save-oracle market.json by contract_id, then scans the contract object
and first-level heap pointers for speed/completed/days/delivery/progress roots.
It does not emit compact rows unless a field is accepted by an exact same-tick
match.
"""

from __future__ import annotations

import argparse
import json
import math
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
sys.path.insert(0, str(_HERE))

import market_trade_live_compact as mt  # noqa: E402
import world_extract_live as world_live  # noqa: E402
from purchase_contract_detail_probe import read_contract_detail_candidate  # noqa: E402
from resource_flow_root_probe import country_maps_with_dynamic_fallback  # noqa: E402


OBJECT_READ_LEN = 0x280
POINTER_READ_LEN = 0x200
SCALE_RAW = 100000
DOMAIN_ID = 43
RECORD_TYPE_DOMAIN_DATA = 3
RECORD_TYPE_FRAME_END = 2
H4CR_HEADER = struct.Struct("<4sHHQiHHII")
V1_HEADER = struct.Struct("<QIIIIIIIIII")
CORE_ROW = struct.Struct("<IIIIIIqIIII")
DETAIL_HEADER = struct.Struct("<4sHHHHIIIII")
DETAIL_ROW = struct.Struct("<IIqqiiiIiiiiiI")
DETAIL_MAGIC = b"MTD1"
DETAIL_VERSION = 1
DETAIL_HEADER_LEN = DETAIL_HEADER.size
DETAIL_ROW_LEN = DETAIL_ROW.size

DETAIL_FLAG_PRICE_VALID = 1
DETAIL_FLAG_AMOUNT_VALID = 1 << 1
DETAIL_FLAG_SPEED_VALID = 1 << 2
DETAIL_FLAG_COMPLETED_VALID = 1 << 3
DETAIL_FLAG_DAYS_VALID = 1 << 4
DETAIL_FLAG_DELIVERY_COUNTRY_VALID = 1 << 5
DETAIL_FLAG_CONVOYS_VALID = 1 << 6
DETAIL_FLAG_EFFICIENCY_VALID = 1 << 7
DETAIL_FLAG_FACTORY_PROGRESS_VALID = 1 << 8
DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID = 1 << 9


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) and value else None


def date_to_raw(value: str | None) -> int | None:
    if not value:
        return None
    try:
        return world_live.encode_datehour(value)
    except Exception:
        return None


def read_i32(blob: bytes, offset: int) -> int | None:
    if offset < 0 or offset + 4 > len(blob):
        return None
    return struct.unpack_from("<i", blob, offset)[0]


def read_u32(blob: bytes, offset: int) -> int | None:
    if offset < 0 or offset + 4 > len(blob):
        return None
    return struct.unpack_from("<I", blob, offset)[0]


def read_u64(blob: bytes, offset: int) -> int | None:
    if offset < 0 or offset + 8 > len(blob):
        return None
    return struct.unpack_from("<Q", blob, offset)[0]


def read_f32(blob: bytes, offset: int) -> float | None:
    if offset < 0 or offset + 4 > len(blob):
        return None
    value = struct.unpack_from("<f", blob, offset)[0]
    if math.isfinite(value):
        return value
    return None


def raw_scaled(value: float | int | None) -> int | None:
    if value is None:
        return None
    return int(round(float(value) * SCALE_RAW))


def as_int(value: float | int | None) -> int | None:
    if value is None:
        return None
    if abs(float(value) - round(float(value))) > 0.00001:
        return None
    return int(round(float(value)))


def load_oracle(path: Path, tag_to_id: dict[str, int]) -> tuple[dict[int, dict], dict]:
    market = json.loads(path.read_text(encoding="utf-8"))
    by_id: dict[int, dict] = {}
    for contract in market.get("contracts") or []:
        cid = contract.get("contract_id")
        if not isinstance(cid, list) or len(cid) != 2 or int(cid[1]) != mt.CPURCHASE_CONTRACT_TYPE:
            continue
        contract_id = int(cid[0])
        equipment = (contract.get("equipments") or [{}])[0] or {}
        price = (contract.get("prices") or [{}])[0] or {}
        delivery_state = contract.get("delivery_state") or {}
        convoys = contract.get("convoys") or {}
        delivery_country = contract.get("delivery_country")
        by_id[contract_id] = {
            "contract_id": contract_id,
            "seller": contract.get("seller"),
            "buyer": contract.get("buyer"),
            "speed": as_int(contract.get("speed")),
            "completed": as_int(contract.get("completed")),
            "days": as_int(contract.get("days")),
            "delivery_country": delivery_country,
            "delivery_country_tag_db_id": tag_to_id.get(delivery_country or ""),
            "convoys_assigned": as_int(convoys.get("assigned")),
            "convoys_total": as_int(convoys.get("total")),
            "efficiency_int": as_int(contract.get("delivery_efficiency")),
            "efficiency_raw": raw_scaled(contract.get("delivery_efficiency")),
            "factory_progress_int": as_int(delivery_state.get("factory_cic_progress")),
            "factory_progress_raw": raw_scaled(delivery_state.get("factory_cic_progress")),
            "equipment_progress_int": as_int(delivery_state.get("equipment_transfer_progress")),
            "equipment_progress_raw": raw_scaled(delivery_state.get("equipment_transfer_progress")),
            "collected_raw": raw_scaled(delivery_state.get("collected")),
            "amount_raw": raw_scaled(equipment.get("amount")),
            "price_raw": raw_scaled(price.get("amount")),
            "equip_id": int(equipment.get("equip_id") or 0),
        }
    return by_id, market.get("meta") or {}


def live_contract_samples(p: mt.winmem.Process, maps: dict) -> tuple[list[dict], Counter[str]]:
    contract_vtable = p.module_base + mt.CPURCHASE_CONTRACT_VTABLE_RVA
    variant_vtable = p.module_base + mt.VARIANT_VTABLE_RVA
    id_to_tag = maps["id_to_tag"]
    rows = []
    reasons: Counter[str] = Counter()
    for addr in mt.scan_contract_candidates(p):
        row, reason = mt.read_contract_candidate(
            p,
            addr,
            contract_vtable,
            variant_vtable,
            maps["id_to_country_dynamic"],
        )
        reasons[reason] += 1
        if row is None or not mt.is_complete_contract_row(row):
            continue
        detail, detail_reason = read_contract_detail_candidate(p, addr, id_to_tag)
        reasons[f"detail_{detail_reason}"] += 1
        blob = p.try_read(addr, OBJECT_READ_LEN)
        if len(blob) != OBJECT_READ_LEN:
            reasons["contract_object_unreadable"] += 1
            continue
        rows.append(
            {
                "addr": addr,
                "row": row,
                "detail": detail or {},
                "blob": blob,
            }
        )
    rows.sort(key=lambda item: item["row"].contract_id)
    return rows, reasons


def candidate_scalar_paths(
    p: mt.winmem.Process, samples: list[dict], include_pointers: bool
) -> dict[str, list[dict]]:
    paths: dict[str, list[dict]] = {}

    direct_paths = []
    for off in range(0x10, OBJECT_READ_LEN - 3, 4):
        direct_paths.append(("contract", None, off, "i32"))
        direct_paths.append(("contract", None, off, "u32"))
        direct_paths.append(("contract", None, off, "f32"))
    if include_pointers:
        pointer_offsets = set()
        for sample in samples:
            blob = sample["blob"]
            for off in range(0x10, OBJECT_READ_LEN - 7, 8):
                ptr = read_u64(blob, off)
                if mt.is_heap(ptr):
                    pointer_offsets.add(off)
        for ptr_off in sorted(pointer_offsets):
            for inner in range(0, POINTER_READ_LEN - 3, 4):
                direct_paths.append(("ptr", ptr_off, inner, "i32"))
                direct_paths.append(("ptr", ptr_off, inner, "u32"))
                direct_paths.append(("ptr", ptr_off, inner, "f32"))

    for kind, ptr_off, inner_off, value_type in direct_paths:
        paths.setdefault(value_type, []).append(
            {
                "kind": kind,
                "ptr_off": ptr_off,
                "inner_off": inner_off,
                "value_type": value_type,
            }
        )
    return paths


def read_path_value(p: mt.winmem.Process, sample: dict, path: dict):
    if path["kind"] == "contract":
        blob = sample["blob"]
        off = path["inner_off"]
    else:
        ptr = read_u64(sample["blob"], path["ptr_off"])
        if not mt.is_heap(ptr):
            return None
        blob = p.try_read(ptr, POINTER_READ_LEN)
        if len(blob) < path["inner_off"] + 4:
            return None
        off = path["inner_off"]
    if path["value_type"] == "i32":
        return read_i32(blob, off)
    if path["value_type"] == "u32":
        return read_u32(blob, off)
    if path["value_type"] == "f32":
        return read_f32(blob, off)
    return None


def value_matches(actual, expected: int | float | None, value_type: str) -> bool:
    if expected is None or actual is None:
        return False
    if value_type == "f32":
        return abs(float(actual) - float(expected)) <= 0.0002
    return int(actual) == int(expected)


def score_field(
    p: mt.winmem.Process,
    samples: list[dict],
    oracle: dict[int, dict],
    field_name: str,
    expected_keys: list[tuple[str, str]],
    paths_by_type: dict[str, list[dict]],
) -> list[dict]:
    expected_by_contract = {
        sample["row"].contract_id: oracle[sample["row"].contract_id]
        for sample in samples
        if sample["row"].contract_id in oracle
    }
    out = []
    for value_type, expected_key in expected_keys:
        contracts_with_expected = [
            contract_id
            for contract_id, row in expected_by_contract.items()
            if row.get(expected_key) is not None
        ]
        if not contracts_with_expected:
            continue
        for path in paths_by_type.get(value_type, []):
            matches = 0
            mismatches = []
            values = []
            for sample in samples:
                contract_id = sample["row"].contract_id
                expected = expected_by_contract.get(contract_id, {}).get(expected_key)
                if expected is None:
                    continue
                actual = read_path_value(p, sample, path)
                if value_matches(actual, expected, value_type):
                    matches += 1
                elif len(mismatches) < 8:
                    mismatches.append(
                        {
                            "contract_id": contract_id,
                            "expected": expected,
                            "actual": actual,
                        }
                    )
                if len(values) < 12:
                    values.append(actual)
            if matches:
                out.append(
                    {
                        "field": field_name,
                        "expected_key": expected_key,
                        "value_type": value_type,
                        "kind": path["kind"],
                        "ptr_off": path["ptr_off"],
                        "ptr_off_hex": hx(path["ptr_off"]),
                        "inner_off": path["inner_off"],
                        "inner_off_hex": hx(path["inner_off"]),
                        "matched": matches,
                        "expected_count": len(contracts_with_expected),
                        "match_rate": matches / max(1, len(contracts_with_expected)),
                        "sample_values": values,
                        "mismatches": mismatches,
                    }
                )
    out.sort(key=lambda row: (-row["matched"], row["kind"] != "contract", row["ptr_off"] or -1, row["inner_off"]))
    return out[:24]


def accepted_fields(scores: dict[str, list[dict]], min_rows: int) -> dict[str, dict]:
    accepted = {}
    for field, rows in scores.items():
        if not rows:
            continue
        best = rows[0]
        if best["expected_count"] >= min_rows and best["matched"] == best["expected_count"]:
            accepted[field] = best
    return accepted


def example_rows(samples: list[dict], oracle: dict[int, dict], limit: int) -> list[dict]:
    out = []
    for sample in samples[:limit]:
        row = sample["row"]
        expected = oracle.get(row.contract_id) or {}
        out.append(
            {
                "contract_id": row.contract_id,
                "seller_tag_db_id": row.seller_tag_db_id,
                "buyer_tag_db_id": row.buyer_tag_db_id,
                "equipment_variant_id": row.equipment_variant_id,
                "amount_raw": row.amount_raw,
                "oracle": {
                    key: expected.get(key)
                    for key in [
                        "speed",
                        "completed",
                        "days",
                        "delivery_country_tag_db_id",
                        "convoys_assigned",
                        "convoys_total",
                        "efficiency_int",
                        "factory_progress_int",
                        "equipment_progress_int",
                    ]
                },
            }
        )
    return out


def accepted_value(p: mt.winmem.Process, sample: dict, accepted: dict, field: str):
    path = accepted.get(field)
    if not path:
        return None
    return read_path_value(p, sample, path)


def accepted_raw_value(p: mt.winmem.Process, sample: dict, accepted: dict, field: str):
    path = accepted.get(field)
    if not path:
        return None
    value = read_path_value(p, sample, path)
    if value is None:
        return None
    if str(path.get("expected_key") or "").endswith("_raw"):
        return int(round(float(value)))
    return raw_scaled(value)


def core_payload_header(elapsed_ms: int, samples: list[dict]) -> bytes:
    rows = [sample["row"] for sample in samples]
    return V1_HEADER.pack(
        int(elapsed_ms),
        len(rows),
        len(rows),
        0,
        len(rows),
        len(rows),
        sum(1 for row in rows if row.flags & mt.FLAG_BUYER_PTR_VALID),
        sum(1 for row in rows if row.flags & mt.FLAG_VARIANT_VALID),
        sum(1 for row in rows if row.flags & mt.FLAG_AMOUNT_VALID),
        sum(1 for row in rows if not mt.is_complete_contract_row(row)),
        SCALE_RAW,
    )


def encode_core_rows(samples: list[dict]) -> bytes:
    out = bytearray()
    for sample in samples:
        row = sample["row"]
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


def build_detail_rows(p: mt.winmem.Process, samples: list[dict], accepted: dict) -> list[dict]:
    rows = []
    for sample in samples:
        row = sample["row"]
        detail = sample["detail"]
        flags = 0
        price_raw = int(detail.get("price_raw") or 0)
        amount_raw = int(detail.get("amount_raw") or 0)
        if price_raw > 0:
            flags |= DETAIL_FLAG_PRICE_VALID
        if amount_raw > 0:
            flags |= DETAIL_FLAG_AMOUNT_VALID

        speed = accepted_value(p, sample, accepted, "speed")
        completed = accepted_value(p, sample, accepted, "completed")
        days = accepted_value(p, sample, accepted, "days")
        delivery_country = accepted_value(p, sample, accepted, "delivery_country_tag_db_id")
        convoys_assigned = accepted_value(p, sample, accepted, "convoys_assigned")
        convoys_total = accepted_value(p, sample, accepted, "convoys_total")
        efficiency_raw = accepted_raw_value(p, sample, accepted, "efficiency")
        factory_progress_raw = accepted_raw_value(p, sample, accepted, "factory_cic_progress")
        equipment_progress_raw = accepted_raw_value(p, sample, accepted, "equipment_transfer_progress")

        speed_raw = int(speed or 0)
        completed_raw = int(completed or 0)
        days_raw = int(days or 0)
        delivery_country_tag_db_id = int(delivery_country or 0)
        convoys_assigned_raw = int(convoys_assigned or 0)
        convoys_total_raw = int(convoys_total or 0)
        efficiency_raw = int(efficiency_raw or 0)
        factory_progress_raw = int(factory_progress_raw or 0)
        equipment_progress_raw = int(equipment_progress_raw or 0)

        if "speed" in accepted:
            flags |= DETAIL_FLAG_SPEED_VALID
        if "completed" in accepted:
            flags |= DETAIL_FLAG_COMPLETED_VALID
        if "days" in accepted:
            flags |= DETAIL_FLAG_DAYS_VALID
        if "delivery_country_tag_db_id" in accepted and delivery_country_tag_db_id > 0:
            flags |= DETAIL_FLAG_DELIVERY_COUNTRY_VALID
        if (
            "convoys_assigned" in accepted
            and "convoys_total" in accepted
            and (convoys_assigned_raw > 0 or convoys_total_raw > 0)
        ):
            flags |= DETAIL_FLAG_CONVOYS_VALID
        else:
            convoys_assigned_raw = 0
            convoys_total_raw = 0
        if "efficiency" in accepted:
            flags |= DETAIL_FLAG_EFFICIENCY_VALID
        else:
            efficiency_raw = 0
        if "factory_cic_progress" in accepted:
            flags |= DETAIL_FLAG_FACTORY_PROGRESS_VALID
        else:
            factory_progress_raw = 0
        if "equipment_transfer_progress" in accepted:
            flags |= DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID
        else:
            equipment_progress_raw = 0

        rows.append(
            {
                "contract_id": int(row.contract_id),
                "flags": flags,
                "price_raw": price_raw,
                "amount_raw": amount_raw,
                "speed_raw": speed_raw,
                "completed_raw": completed_raw,
                "days": days_raw,
                "delivery_country_tag_db_id": delivery_country_tag_db_id,
                "convoys_assigned": convoys_assigned_raw,
                "convoys_total": convoys_total_raw,
                "efficiency_raw": efficiency_raw,
                "factory_cic_progress_raw": factory_progress_raw,
                "equipment_transfer_progress_raw": equipment_progress_raw,
            }
        )
    return rows


def encode_detail_trailer(rows: list[dict]) -> bytes:
    price_valid = sum(1 for row in rows if row["flags"] & DETAIL_FLAG_PRICE_VALID)
    speed_valid = sum(1 for row in rows if row["flags"] & DETAIL_FLAG_SPEED_VALID)
    route_valid = sum(
        1
        for row in rows
        if row["flags"]
        & (
            DETAIL_FLAG_DELIVERY_COUNTRY_VALID
            | DETAIL_FLAG_CONVOYS_VALID
            | DETAIL_FLAG_EFFICIENCY_VALID
        )
    )
    progress_valid = sum(
        1
        for row in rows
        if row["flags"]
        & (DETAIL_FLAG_FACTORY_PROGRESS_VALID | DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID)
    )
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
        out += DETAIL_ROW.pack(
            row["contract_id"],
            row["flags"],
            row["price_raw"],
            row["amount_raw"],
            row["speed_raw"],
            row["completed_raw"],
            row["days"],
            row["delivery_country_tag_db_id"],
            row["convoys_assigned"],
            row["convoys_total"],
            row["efficiency_raw"],
            row["factory_cic_progress_raw"],
            row["equipment_transfer_progress_raw"],
            0,
        )
    return bytes(out)


def append_h4cr(out: bytearray, sequence: int, game_hour: int, domain: int, record_type: int, payload: bytes) -> None:
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


def encode_stream(game_hour: int, elapsed_ms: int, samples: list[dict], detail_rows: list[dict]) -> bytes:
    payload = core_payload_header(elapsed_ms, samples)
    payload += encode_core_rows(samples)
    payload += encode_detail_trailer(detail_rows)
    out = bytearray()
    append_h4cr(out, 1, game_hour, DOMAIN_ID, RECORD_TYPE_DOMAIN_DATA, payload)
    append_h4cr(out, 2, game_hour, 0, RECORD_TYPE_FRAME_END, b"")
    return bytes(out)


def probe(args: argparse.Namespace) -> dict:
    started = time.perf_counter()
    pid = args.pid or mt.winmem.find_pid(mt.PROC_NAME)
    if not pid:
        raise SystemExit(f"{mt.PROC_NAME} not running")
    with mt.winmem.Process(pid, mt.PROC_NAME) as p:
        fp = p.fingerprint()
        sha = str(fp.get("sha256") or "").lower()
        if sha != mt.EXPECTED_SHA256 and not args.force:
            raise SystemExit(f"live hoi4.exe sha256 {sha} != expected {mt.EXPECTED_SHA256}")
        maps = country_maps_with_dynamic_fallback(p)
        tag_to_id = {tag: tag_id for tag_id, tag in maps["id_to_tag"].items()}
        oracle, meta = load_oracle(Path(args.market_json), tag_to_id)
        live_raw = mt.read_game_hour_raw(p)
        save_raw = date_to_raw(meta.get("save_date"))
        samples, reasons = live_contract_samples(p, maps)
        joined = [sample for sample in samples if sample["row"].contract_id in oracle]
        if args.max_contracts:
            joined = joined[: args.max_contracts]
        paths = candidate_scalar_paths(p, joined, include_pointers=not args.direct_only)
        field_specs = {
            "speed": [("i32", "speed"), ("u32", "speed")],
            "completed": [("i32", "completed"), ("u32", "completed")],
            "days": [("i32", "days"), ("u32", "days")],
            "delivery_country_tag_db_id": [
                ("i32", "delivery_country_tag_db_id"),
                ("u32", "delivery_country_tag_db_id"),
            ],
            "convoys_assigned": [("i32", "convoys_assigned"), ("u32", "convoys_assigned")],
            "convoys_total": [("i32", "convoys_total"), ("u32", "convoys_total")],
            "efficiency": [
                ("i32", "efficiency_raw"),
                ("u32", "efficiency_raw"),
                ("i32", "efficiency_int"),
                ("u32", "efficiency_int"),
                ("f32", "efficiency_int"),
            ],
            "factory_cic_progress": [
                ("i32", "factory_progress_raw"),
                ("u32", "factory_progress_raw"),
                ("i32", "factory_progress_int"),
                ("u32", "factory_progress_int"),
                ("f32", "factory_progress_int"),
            ],
            "equipment_transfer_progress": [
                ("i32", "equipment_progress_raw"),
                ("u32", "equipment_progress_raw"),
                ("i32", "equipment_progress_int"),
                ("u32", "equipment_progress_int"),
                ("f32", "equipment_progress_int"),
            ],
        }
        scores = {
            field: score_field(p, joined, oracle, field, specs, paths)
            for field, specs in field_specs.items()
        }
        accepted = accepted_fields(scores, args.min_accept_rows)
        elapsed_ms = int((time.perf_counter() - started) * 1000)
        detail_rows = build_detail_rows(p, joined, accepted)
        stream = encode_stream(live_raw or 0, elapsed_ms, joined, detail_rows)
    same_tick = bool(live_raw is not None and save_raw is not None and live_raw == save_raw)
    accepted_for_emission = bool(same_tick and accepted)
    compact_stream_written = False
    if args.out_bin and accepted_for_emission:
        out_bin = Path(args.out_bin)
        out_bin.parent.mkdir(parents=True, exist_ok=True)
        out_bin.write_bytes(stream)
        compact_stream_written = True
    detail_counts = {
        "detail_rows": len(detail_rows),
        "price_valid_rows": sum(1 for row in detail_rows if row["flags"] & DETAIL_FLAG_PRICE_VALID),
        "amount_valid_rows": sum(1 for row in detail_rows if row["flags"] & DETAIL_FLAG_AMOUNT_VALID),
        "speed_valid_rows": sum(1 for row in detail_rows if row["flags"] & DETAIL_FLAG_SPEED_VALID),
        "completed_valid_rows": sum(1 for row in detail_rows if row["flags"] & DETAIL_FLAG_COMPLETED_VALID),
        "days_valid_rows": sum(1 for row in detail_rows if row["flags"] & DETAIL_FLAG_DAYS_VALID),
        "delivery_country_valid_rows": sum(
            1 for row in detail_rows if row["flags"] & DETAIL_FLAG_DELIVERY_COUNTRY_VALID
        ),
        "convoys_valid_rows": sum(1 for row in detail_rows if row["flags"] & DETAIL_FLAG_CONVOYS_VALID),
        "efficiency_valid_rows": sum(1 for row in detail_rows if row["flags"] & DETAIL_FLAG_EFFICIENCY_VALID),
        "factory_progress_valid_rows": sum(
            1 for row in detail_rows if row["flags"] & DETAIL_FLAG_FACTORY_PROGRESS_VALID
        ),
        "equipment_progress_valid_rows": sum(
            1 for row in detail_rows if row["flags"] & DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID
        ),
    }
    proof = {
        "schema": "market_trade_live_purchase_contract_same_tick_probe_v1",
        "utc": datetime.now(timezone.utc).isoformat(),
        "pid": pid,
        "module": {
            "path": fp.get("path"),
            "sha256": sha,
        },
        "fixture": {
            "market_json": str(args.market_json),
            "save_filename": meta.get("source_filename"),
            "save_date": meta.get("save_date"),
            "save_date_raw": save_raw,
            "live_date": world_live.decode_datehour(live_raw) if live_raw is not None else None,
            "live_date_raw": live_raw,
            "same_tick": same_tick,
            "delta_hours": live_raw - save_raw if isinstance(live_raw, int) and isinstance(save_raw, int) else None,
        },
        "roots": {
            "CPurchaseContract.vtable_rva": hex(mt.CPURCHASE_CONTRACT_VTABLE_RVA),
            "CEquipmentVariant.vtable_rva": hex(mt.VARIANT_VTABLE_RVA),
            "object_read_len": OBJECT_READ_LEN,
            "pointer_read_len": POINTER_READ_LEN,
            "accepted_offsets": {
                field: {
                    "kind": row["kind"],
                    "ptr_off_hex": row["ptr_off_hex"],
                    "inner_off_hex": row["inner_off_hex"],
                    "value_type": row["value_type"],
                    "matched": row["matched"],
                    "expected_count": row["expected_count"],
                }
                for field, row in sorted((accepted if same_tick else {}).items())
            },
        },
        "compact": {
            "domain_id": DOMAIN_ID,
            "detail_magic": DETAIL_MAGIC.decode("ascii"),
            "detail_version": DETAIL_VERSION,
            "detail_header_len": DETAIL_HEADER_LEN,
            "detail_row_len": DETAIL_ROW_LEN,
            "stream_bytes": len(stream),
            **detail_counts,
        },
        "contracts": {
            "live_complete_rows": len(samples),
            "oracle_rows": len(oracle),
            "joined_rows": len(joined),
            "candidate_reasons": dict(sorted(reasons.items())),
            "examples": example_rows(joined, oracle, args.examples),
        },
        "accepted_fields": accepted if same_tick else {},
        "field_scores": scores,
        "classification": {
            "accepted_for_emission": accepted_for_emission,
            "same_tick": same_tick,
            "accepted_field_count": len(accepted) if same_tick else 0,
            "accepted_fields": sorted(accepted) if same_tick else [],
            "compact_stream_written": compact_stream_written,
            "reason": (
                "same-tick field roots accepted"
                if same_tick and accepted
                else "no exact same-tick field roots accepted"
            ),
        },
        "elapsed_ms": round((time.perf_counter() - started) * 1000, 3),
    }
    if args.out_json:
        out_path = Path(args.out_json)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(json.dumps(proof, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return proof


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int)
    parser.add_argument("--market-json", required=True)
    parser.add_argument("--out-json", required=True)
    parser.add_argument("--out-bin")
    parser.add_argument("--max-contracts", type=int, default=96)
    parser.add_argument("--min-accept-rows", type=int, default=12)
    parser.add_argument("--examples", type=int, default=12)
    parser.add_argument("--direct-only", action="store_true")
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args(argv)
    proof = probe(args)
    print(json.dumps(proof["classification"], indent=2, sort_keys=True))
    if args.out_bin and not proof["classification"]["compact_stream_written"]:
        return 3
    return 0 if proof["classification"]["same_tick"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
