"""Lane-local compact extractor/gate for live international-market trades.

This module intentionally does not touch the shared Rust collector/recorder
entrypoints. It uses the validated 870ebf CPurchaseContract vtable root and
packs a compact binary frame stream that mirrors the intended production row
shape for a later Rust integration slot.
"""

from __future__ import annotations

import argparse
import json
import os
import struct
import sys
import time
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from typing import Iterable

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)

import winmem  # noqa: E402
from market_trade_live_probe import (  # noqa: E402
    CGAMESTATE_COUNTRY_COUNT,
    CGAMESTATE_COUNTRY_TABLE,
    CG_STATE_DATE_OFFSET,
    COUNTRY_DATABASE_OFFSET,
    COUNTRY_VTABLE_RVA,
    EXPECTED_SHA256,
    HEAP_HI,
    HEAP_LO,
    OBJ_TAG_DB_ID,
    PROC_NAME,
    VARIANT_ID_OFF,
    VARIANT_TYPE,
    VARIANT_TYPE_OFF,
    VARIANT_VTABLE_RVA,
    is_heap,
    read_country_maps,
    safe_u32,
    safe_u64,
    scan_heap_vtables,
)

GS_CURRENT_DATE_OFFSET = 0x468

CPURCHASE_CONTRACT_VTABLE_RVA = 0x2954C70
CPURCHASE_CONTRACT_TYPE = 81
PC_ID_TYPE = 0x08
PC_ID_ID = 0x0C
PC_EQUIPMENT_SUBBUF = 0x38
PC_SELLER_TAGID = 0x58
PC_BUYER_TAGID = 0x5C
PC_DELIVERY_SUBBUF = 0x80
PC_BUYER_PTR = 0x128

SCALE_RAW = 100000
MAX_TAG_DB_ID = 2048
MAX_CONTRACT_ID = 2_147_483_647
MAX_VARIANT_ID = 2_147_483_647
MAX_AMOUNT_RAW = 1_000_000_000_000_000

MAGIC = b"H4MT"
VERSION = 1
FRAME_HEADER = struct.Struct("<4sHHHHIqIIIIIIIIII")
ROW = struct.Struct("<IIIIIIqIIII")
HEADER_LEN = FRAME_HEADER.size
ROW_LEN = ROW.size

STATUS_SOURCE_UNAVAILABLE = 1 << 8
STATUS_STRUCTURAL_PARTIAL = 1 << 9

FLAG_BUYER_PTR_VALID = 1
FLAG_EQUIPMENT_SUBBUF_VALID = 1 << 1
FLAG_DELIVERY_SUBBUF_VALID = 1 << 2
FLAG_VARIANT_VALID = 1 << 3
FLAG_AMOUNT_VALID = 1 << 4
KNOWN_FLAGS = (
    FLAG_BUYER_PTR_VALID
    | FLAG_EQUIPMENT_SUBBUF_VALID
    | FLAG_DELIVERY_SUBBUF_VALID
    | FLAG_VARIANT_VALID
    | FLAG_AMOUNT_VALID
)

STATUS_INVALID = 0
STATUS_VALID = 1
STATUS_UNREADABLE = 2


@dataclass(frozen=True)
class MarketTradeRow:
    contract_id: int
    contract_type: int
    flags: int
    seller_tag_db_id: int
    buyer_tag_db_id: int
    equipment_variant_id: int
    amount_raw: int
    equipment_subbuf_status: int
    delivery_subbuf_status: int
    buyer_ptr_status: int


@dataclass(frozen=True)
class MarketTradeFrame:
    game_hour_raw: int
    elapsed_wall_ms: int
    rows: tuple[MarketTradeRow, ...]
    status_flags: int
    observed_contract_rows: int
    skipped_contract_rows: int
    omitted_candidate_rows: int = 0
    structural_partial_rows: int = 0
    omission_reasons: dict[str, int] | None = None

    @property
    def buyer_ptr_valid_rows(self) -> int:
        return sum(1 for row in self.rows if row.flags & FLAG_BUYER_PTR_VALID)

    @property
    def variant_valid_rows(self) -> int:
        return sum(1 for row in self.rows if row.flags & FLAG_VARIANT_VALID)

    @property
    def amount_valid_rows(self) -> int:
        return sum(1 for row in self.rows if row.flags & FLAG_AMOUNT_VALID)

    @property
    def partial_field_rows(self) -> int:
        return sum(
            1
            for row in self.rows
            if row.flags & FLAG_BUYER_PTR_VALID == 0
            or row.flags & FLAG_EQUIPMENT_SUBBUF_VALID == 0
            or row.flags & FLAG_DELIVERY_SUBBUF_VALID == 0
            or row.flags & FLAG_VARIANT_VALID == 0
            or row.flags & FLAG_AMOUNT_VALID == 0
        )


def safe_i32(p: winmem.Process, addr: int) -> int | None:
    blob = p.try_read(addr, 4)
    if len(blob) != 4:
        return None
    return struct.unpack("<i", blob)[0]


def read_game_hour_raw(p: winmem.Process) -> int | None:
    game_state = safe_u64(p, p.module_base + CG_STATE_DATE_OFFSET)
    if not is_heap(game_state):
        return None
    return safe_i32(p, game_state + GS_CURRENT_DATE_OFFSET)


def scan_variant_subbuffer(
    p: winmem.Process, ptr: int | None, variant_vtable: int, span: int = 0x80
) -> tuple[int, int | None, int | None]:
    if ptr is None:
        return STATUS_UNREADABLE, None, None
    if not is_heap(ptr):
        return STATUS_INVALID, None, None
    blob = p.try_read(ptr, span)
    if len(blob) < 0x18:
        return STATUS_UNREADABLE if not blob else STATUS_INVALID, None, None
    for off in range(0, len(blob) - 8, 8):
        qword = struct.unpack_from("<Q", blob, off)[0]
        if not is_heap(qword):
            continue
        if safe_u64(p, qword) != variant_vtable:
            continue
        if safe_u32(p, qword + VARIANT_TYPE_OFF) != VARIANT_TYPE:
            continue
        variant_id = safe_u32(p, qword + VARIANT_ID_OFF)
        if not variant_id or variant_id > MAX_VARIANT_ID:
            return STATUS_INVALID, None, None
        amount_raw = None
        for amount_off in range(off + 8, min(off + 0x18, len(blob) - 3), 4):
            raw = struct.unpack_from("<i", blob, amount_off)[0]
            if 0 < raw <= MAX_AMOUNT_RAW:
                amount_raw = raw
                break
        return STATUS_VALID, variant_id, amount_raw
    return STATUS_INVALID, None, None


def read_contract_row(
    p: winmem.Process,
    addr: int,
    contract_vtable: int,
    variant_vtable: int,
    id_to_country: dict[int, int],
) -> MarketTradeRow | None:
    row, _reason = read_contract_candidate(
        p, addr, contract_vtable, variant_vtable, id_to_country
    )
    return row


def read_contract_candidate(
    p: winmem.Process,
    addr: int,
    contract_vtable: int,
    variant_vtable: int,
    id_to_country: dict[int, int],
) -> tuple[MarketTradeRow | None, str]:
    if not is_heap(addr):
        return None, "candidate_vtable_invalid"
    blob = p.try_read(addr, PC_BUYER_PTR + 8)
    if len(blob) < PC_BUYER_PTR + 8:
        return None, "candidate_contract_unreadable"
    if struct.unpack_from("<Q", blob, 0)[0] != contract_vtable:
        return None, "candidate_vtable_invalid"
    contract_type = struct.unpack_from("<I", blob, PC_ID_TYPE)[0]
    contract_id = struct.unpack_from("<I", blob, PC_ID_ID)[0]
    seller_id = struct.unpack_from("<I", blob, PC_SELLER_TAGID)[0]
    buyer_id = struct.unpack_from("<I", blob, PC_BUYER_TAGID)[0]
    if contract_type != CPURCHASE_CONTRACT_TYPE:
        return None, "candidate_type_invalid"
    if not contract_id or contract_id > MAX_CONTRACT_ID:
        return None, "candidate_contract_id_invalid"
    if not seller_id or seller_id > MAX_TAG_DB_ID:
        return None, "candidate_seller_tag_invalid"
    if not buyer_id or buyer_id > MAX_TAG_DB_ID:
        return None, "candidate_buyer_tag_invalid"
    if seller_id not in id_to_country:
        return None, "candidate_seller_country_unresolved"
    if buyer_id not in id_to_country:
        return None, "candidate_buyer_country_unresolved"

    flags = 0
    buyer_ptr = struct.unpack_from("<Q", blob, PC_BUYER_PTR)[0]
    if buyer_ptr == id_to_country.get(buyer_id):
        buyer_ptr_status = STATUS_VALID
        flags |= FLAG_BUYER_PTR_VALID
    else:
        buyer_ptr_status = STATUS_INVALID

    equipment_subbuf = struct.unpack_from("<Q", blob, PC_EQUIPMENT_SUBBUF)[0]
    eq_status, eq_variant_id, _price_raw = scan_variant_subbuffer(
        p, equipment_subbuf, variant_vtable
    )
    if eq_status == STATUS_VALID:
        flags |= FLAG_EQUIPMENT_SUBBUF_VALID | FLAG_VARIANT_VALID
    delivery_subbuf = struct.unpack_from("<Q", blob, PC_DELIVERY_SUBBUF)[0]
    delivery_status, _delivery_variant_id, amount_raw = scan_variant_subbuffer(
        p, delivery_subbuf, variant_vtable
    )
    if delivery_status == STATUS_VALID:
        flags |= FLAG_DELIVERY_SUBBUF_VALID
    if amount_raw is not None:
        flags |= FLAG_AMOUNT_VALID

    return (
        MarketTradeRow(
            contract_id=contract_id,
            contract_type=contract_type,
            flags=flags,
            seller_tag_db_id=seller_id,
            buyer_tag_db_id=buyer_id,
            equipment_variant_id=eq_variant_id or 0,
            amount_raw=amount_raw or 0,
            equipment_subbuf_status=eq_status,
            delivery_subbuf_status=delivery_status,
            buyer_ptr_status=buyer_ptr_status,
        ),
        "emitted",
    )


def extract_frame(p: winmem.Process, elapsed_wall_ms: int) -> MarketTradeFrame:
    game_hour_raw = read_game_hour_raw(p)
    if game_hour_raw is None:
        return MarketTradeFrame(0, elapsed_wall_ms, (), STATUS_SOURCE_UNAVAILABLE, 0, 0)
    hits = scan_contract_candidates(p)
    return extract_frame_from_candidates(p, elapsed_wall_ms, game_hour_raw, hits)


def scan_contract_candidates(p: winmem.Process) -> list[int]:
    contract_vtable = p.module_base + CPURCHASE_CONTRACT_VTABLE_RVA
    return sorted(set(scan_heap_vtables(p, [contract_vtable]).get(contract_vtable, [])))


def extract_frame_from_candidates(
    p: winmem.Process,
    elapsed_wall_ms: int,
    game_hour_raw: int,
    hits: list[int],
    id_to_country: dict[int, int] | None = None,
) -> MarketTradeFrame:
    contract_vtable = p.module_base + CPURCHASE_CONTRACT_VTABLE_RVA
    variant_vtable = p.module_base + VARIANT_VTABLE_RVA
    if id_to_country is None:
        country_maps = read_country_maps(p)
        id_to_country = country_maps["id_to_country"]
    if not id_to_country:
        return MarketTradeFrame(
            game_hour_raw, elapsed_wall_ms, (), STATUS_SOURCE_UNAVAILABLE, 0, 0
        )
    rows_by_id: dict[int, MarketTradeRow] = {}
    omitted = 0
    structural = 0
    reasons: dict[str, int] = {}

    def count(reason: str) -> None:
        reasons[reason] = reasons.get(reason, 0) + 1

    for addr in hits:
        row, reason = read_contract_candidate(
            p, addr, contract_vtable, variant_vtable, id_to_country
        )
        if row is None:
            omitted += 1
            count(reason)
        elif row.contract_id in rows_by_id:
            if rows_by_id[row.contract_id] == row:
                omitted += 1
                count("candidate_duplicate_identical")
            else:
                structural += 1
                count("candidate_duplicate_conflict")
        else:
            rows_by_id[row.contract_id] = row
    rows = [rows_by_id[key] for key in sorted(rows_by_id)]
    status_flags = STATUS_STRUCTURAL_PARTIAL if structural else 0
    return MarketTradeFrame(
        game_hour_raw=game_hour_raw,
        elapsed_wall_ms=elapsed_wall_ms,
        rows=tuple(rows),
        status_flags=status_flags,
        observed_contract_rows=len(hits),
        skipped_contract_rows=omitted + structural,
        omitted_candidate_rows=omitted,
        structural_partial_rows=structural,
        omission_reasons=dict(sorted(reasons.items())),
    )


def encode_frame(frame: MarketTradeFrame) -> bytes:
    out = bytearray()
    out += FRAME_HEADER.pack(
        MAGIC,
        VERSION,
        HEADER_LEN,
        ROW_LEN,
        0,
        frame.game_hour_raw,
        frame.elapsed_wall_ms,
        len(frame.rows),
        frame.status_flags,
        frame.observed_contract_rows,
        len(frame.rows),
        frame.buyer_ptr_valid_rows,
        frame.variant_valid_rows,
        frame.amount_valid_rows,
        frame.partial_field_rows,
        SCALE_RAW,
        0,
    )
    for row in frame.rows:
        out += ROW.pack(
            row.contract_id,
            row.contract_type,
            row.flags,
            row.seller_tag_db_id,
            row.buyer_tag_db_id,
            row.equipment_variant_id,
            row.amount_raw,
            row.equipment_subbuf_status,
            row.delivery_subbuf_status,
            row.buyer_ptr_status,
            0,
        )
    return bytes(out)


def decode_frames(blob: bytes) -> tuple[list[MarketTradeFrame], list[str]]:
    frames: list[MarketTradeFrame] = []
    errors: list[str] = []
    pos = 0
    while pos < len(blob):
        if len(blob) - pos < HEADER_LEN:
            errors.append(f"offset {pos}: truncated frame header")
            break
        unpacked = FRAME_HEADER.unpack_from(blob, pos)
        (
            magic,
            version,
            header_len,
            row_len,
            reserved0,
            game_hour_raw,
            elapsed_wall_ms,
            row_count,
            status_flags,
            observed_contract_rows,
            emitted_contract_rows,
            buyer_valid_rows,
            variant_valid_rows,
            amount_valid_rows,
            partial_field_rows,
            scale_raw,
            reserved1,
        ) = unpacked
        if magic != MAGIC:
            errors.append(f"offset {pos}: bad magic {magic!r}")
            break
        if version != VERSION or header_len != HEADER_LEN or row_len != ROW_LEN:
            errors.append(f"offset {pos}: incompatible frame header")
        if reserved0 or reserved1:
            errors.append(f"offset {pos}: nonzero reserved header fields")
        if emitted_contract_rows != row_count:
            errors.append(f"offset {pos}: emitted rows {emitted_contract_rows} != {row_count}")
        if observed_contract_rows < row_count:
            errors.append(f"offset {pos}: observed rows {observed_contract_rows} < {row_count}")
        if scale_raw != SCALE_RAW:
            errors.append(f"offset {pos}: scale {scale_raw} != {SCALE_RAW}")
        cursor = pos + HEADER_LEN
        end = cursor + row_count * ROW_LEN
        if end > len(blob):
            errors.append(f"offset {pos}: truncated rows")
            break
        rows = []
        seen_ids: set[int] = set()
        computed_buyer = computed_variant = computed_amount = computed_partial = 0
        for idx in range(row_count):
            fields = ROW.unpack_from(blob, cursor)
            cursor += ROW_LEN
            row = MarketTradeRow(*fields[:-1])
            row_reserved = fields[-1]
            rows.append(row)
            if row_reserved:
                errors.append(f"offset {pos}: row {idx} has nonzero reserved field")
            if not (0 < row.contract_id <= MAX_CONTRACT_ID):
                errors.append(f"offset {pos}: row {idx} invalid contract id {row.contract_id}")
            elif row.contract_id in seen_ids:
                errors.append(f"offset {pos}: row {idx} duplicate contract id {row.contract_id}")
            seen_ids.add(row.contract_id)
            if row.contract_type != CPURCHASE_CONTRACT_TYPE:
                errors.append(f"offset {pos}: row {idx} invalid type {row.contract_type}")
            if not (0 < row.seller_tag_db_id <= MAX_TAG_DB_ID):
                errors.append(f"offset {pos}: row {idx} invalid seller {row.seller_tag_db_id}")
            if not (0 < row.buyer_tag_db_id <= MAX_TAG_DB_ID):
                errors.append(f"offset {pos}: row {idx} invalid buyer {row.buyer_tag_db_id}")
            if row.flags & ~KNOWN_FLAGS:
                errors.append(f"offset {pos}: row {idx} unknown flags {row.flags}")
            buyer_valid = bool(row.flags & FLAG_BUYER_PTR_VALID)
            eq_valid = bool(row.flags & FLAG_EQUIPMENT_SUBBUF_VALID)
            delivery_valid = bool(row.flags & FLAG_DELIVERY_SUBBUF_VALID)
            variant_valid = bool(row.flags & FLAG_VARIANT_VALID)
            amount_valid = bool(row.flags & FLAG_AMOUNT_VALID)
            computed_buyer += int(buyer_valid)
            computed_variant += int(variant_valid)
            computed_amount += int(amount_valid)
            computed_partial += int(
                not (buyer_valid and eq_valid and delivery_valid and variant_valid and amount_valid)
            )
            if buyer_valid != (row.buyer_ptr_status == STATUS_VALID):
                errors.append(f"offset {pos}: row {idx} buyer flag/status mismatch")
            if eq_valid != (row.equipment_subbuf_status == STATUS_VALID):
                errors.append(f"offset {pos}: row {idx} equipment flag/status mismatch")
            if delivery_valid != (row.delivery_subbuf_status == STATUS_VALID):
                errors.append(f"offset {pos}: row {idx} delivery flag/status mismatch")
            if variant_valid:
                if not (0 < row.equipment_variant_id <= MAX_VARIANT_ID):
                    errors.append(f"offset {pos}: row {idx} invalid variant id")
            elif row.equipment_variant_id != 0:
                errors.append(f"offset {pos}: row {idx} variant present without valid flag")
            if amount_valid:
                if not (0 < row.amount_raw <= MAX_AMOUNT_RAW):
                    errors.append(f"offset {pos}: row {idx} invalid amount")
            elif row.amount_raw != 0:
                errors.append(f"offset {pos}: row {idx} amount present without valid flag")
        if buyer_valid_rows != computed_buyer:
            errors.append(f"offset {pos}: buyer count mismatch")
        if variant_valid_rows != computed_variant:
            errors.append(f"offset {pos}: variant count mismatch")
        if amount_valid_rows != computed_amount:
            errors.append(f"offset {pos}: amount count mismatch")
        if partial_field_rows != computed_partial:
            errors.append(f"offset {pos}: partial field count mismatch")
        skipped_contract_rows = max(0, observed_contract_rows - emitted_contract_rows)
        structural_partial_rows = (
            skipped_contract_rows if status_flags & STATUS_STRUCTURAL_PARTIAL else 0
        )
        omitted_candidate_rows = (
            0 if status_flags & STATUS_STRUCTURAL_PARTIAL else skipped_contract_rows
        )
        frames.append(
            MarketTradeFrame(
                game_hour_raw,
                elapsed_wall_ms,
                tuple(rows),
                status_flags,
                observed_contract_rows,
                skipped_contract_rows,
                omitted_candidate_rows,
                structural_partial_rows,
            )
        )
        pos = end
    return frames, errors


def skipped_hours(hours: Iterable[int]) -> list[int]:
    ordered = sorted(set(hours))
    missing: list[int] = []
    for prev, nxt in zip(ordered, ordered[1:]):
        if nxt > prev + 1:
            missing.extend(range(prev + 1, nxt))
    return missing


def gate_frames(frames: list[MarketTradeFrame], errors: list[str], min_game_hours: int) -> dict:
    hours = [frame.game_hour_raw for frame in frames]
    missing = skipped_hours(hours)
    status_or = 0
    for frame in frames:
        status_or |= frame.status_flags
    row_counts = [len(frame.rows) for frame in frames]
    partial_field_rows = [frame.partial_field_rows for frame in frames]
    omitted_candidate_rows = [frame.omitted_candidate_rows for frame in frames]
    structural_partial_rows = [frame.structural_partial_rows for frame in frames]
    problems = []
    if errors:
        problems.append("decode_errors")
    if len(set(hours)) < min_game_hours:
        problems.append("insufficient_game_hours")
    if missing:
        problems.append("skipped_game_hours")
    if status_or & STATUS_SOURCE_UNAVAILABLE:
        problems.append("source_unavailable")
    if status_or & STATUS_STRUCTURAL_PARTIAL:
        problems.append("structural_partial")
    return {
        "ok": not problems,
        "problems": problems,
        "frame_count": len(frames),
        "unique_game_hour_count": len(set(hours)),
        "first_game_hour_raw": min(hours) if hours else None,
        "last_game_hour_raw": max(hours) if hours else None,
        "skipped_game_hours": len(missing),
        "decode_error_count": len(errors),
        "status_flags_or": status_or,
        "source_unavailable_seen": bool(status_or & STATUS_SOURCE_UNAVAILABLE),
        "structural_partial_seen": bool(status_or & STATUS_STRUCTURAL_PARTIAL),
        "row_count_min": min(row_counts) if row_counts else 0,
        "row_count_max": max(row_counts) if row_counts else 0,
        "partial_field_rows_min": min(partial_field_rows) if partial_field_rows else 0,
        "partial_field_rows_max": max(partial_field_rows) if partial_field_rows else 0,
        "omitted_candidate_rows_min": (
            min(omitted_candidate_rows) if omitted_candidate_rows else 0
        ),
        "omitted_candidate_rows_max": (
            max(omitted_candidate_rows) if omitted_candidate_rows else 0
        ),
        "structural_partial_rows_min": (
            min(structural_partial_rows) if structural_partial_rows else 0
        ),
        "structural_partial_rows_max": (
            max(structural_partial_rows) if structural_partial_rows else 0
        ),
    }


def aggregate_omission_reasons(frames: Iterable[MarketTradeFrame]) -> dict[str, int]:
    out: dict[str, int] = {}
    for frame in frames:
        for reason, count in (frame.omission_reasons or {}).items():
            out[reason] = out.get(reason, 0) + count
    return dict(sorted(out.items()))


def capture(args: argparse.Namespace) -> dict:
    pid = args.pid or winmem.find_pid(args.process)
    if not pid:
        raise SystemExit(f"{args.process} not running")
    frames: list[MarketTradeFrame] = []
    encoded = bytearray()
    started = time.monotonic()
    candidate_hits: list[int] = []
    cached_id_to_country: dict[int, int] = {}
    refresh_count = 0
    next_refresh_at = 0.0
    with winmem.Process(pid, PROC_NAME) as p:
        sha = str((p.fingerprint().get("sha256") or "")).lower()
        if sha != EXPECTED_SHA256 and not args.force:
            raise SystemExit(f"live hoi4.exe sha256 {sha} != expected {EXPECTED_SHA256}")
        last_hour = None
        deadline = started + args.duration_s
        while time.monotonic() <= deadline or not frames:
            now = time.monotonic()
            elapsed_ms = int((time.monotonic() - started) * 1000)
            game_hour_raw = read_game_hour_raw(p)
            if game_hour_raw and game_hour_raw != last_hour:
                if not candidate_hits or (args.refresh_s > 0 and now >= next_refresh_at):
                    candidate_hits = scan_contract_candidates(p)
                    cached_id_to_country = read_country_maps(p)["id_to_country"]
                    refresh_count += 1
                    next_refresh_at = time.monotonic() + args.refresh_s
                    elapsed_ms = int((time.monotonic() - started) * 1000)
                frame = extract_frame_from_candidates(
                    p, elapsed_ms, game_hour_raw, candidate_hits, cached_id_to_country
                )
                frames.append(frame)
                encoded += encode_frame(frame)
                last_hour = frame.game_hour_raw
            if args.sample_ms > 0:
                time.sleep(args.sample_ms / 1000.0)
    decoded, errors = decode_frames(bytes(encoded))
    gate = gate_frames(decoded, errors, args.min_game_hours)
    proof = {
        "schema": "market_trade_live_lane_local_compact_proof_v1",
        "utc": datetime.now(timezone.utc).isoformat(),
        "pid": pid,
        "duration_s": args.duration_s,
        "sample_ms": args.sample_ms,
        "compact": {
            "format": "H4MT",
            "version": VERSION,
            "header_len": HEADER_LEN,
            "row_len": ROW_LEN,
            "bytes": len(encoded),
        },
        "candidate_cache": {
            "refresh_count": refresh_count,
            "refresh_s": args.refresh_s,
            "candidate_count": len(candidate_hits),
            "country_identity_rows": len(cached_id_to_country),
        },
        "root": {
            "CPurchaseContract.vtable_rva": hex(CPURCHASE_CONTRACT_VTABLE_RVA),
            "CPurchaseContract.type": CPURCHASE_CONTRACT_TYPE,
            "variant_vtable_rva": hex(VARIANT_VTABLE_RVA),
        },
        "gate": gate,
        "extraction_diagnostics": {
            "omission_reasons": aggregate_omission_reasons(frames),
            "omitted_candidate_rows_total": sum(
                frame.omitted_candidate_rows for frame in frames
            ),
            "structural_partial_rows_total": sum(
                frame.structural_partial_rows for frame in frames
            ),
        },
        "decode_errors": errors[:20],
        "first_frame": asdict(decoded[0]) if decoded else None,
    }
    if args.out_bin:
        os.makedirs(os.path.dirname(os.path.abspath(args.out_bin)), exist_ok=True)
        with open(args.out_bin, "wb") as f:
            f.write(encoded)
    if args.out_json:
        os.makedirs(os.path.dirname(os.path.abspath(args.out_json)), exist_ok=True)
        with open(args.out_json, "w", encoding="utf-8") as f:
            json.dump(proof, f, indent=2, sort_keys=True)
    return proof


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int)
    parser.add_argument("--process", default=PROC_NAME)
    parser.add_argument("--duration-s", type=int, default=20)
    parser.add_argument("--sample-ms", type=int, default=0)
    parser.add_argument("--refresh-s", type=int, default=0)
    parser.add_argument("--min-game-hours", type=int, default=12)
    parser.add_argument("--out-bin")
    parser.add_argument("--out-json")
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args(argv)
    proof = capture(args)
    print(json.dumps(proof["gate"], indent=2, sort_keys=True))
    return 0 if proof["gate"]["ok"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
