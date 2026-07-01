"""Compare live CCountry+0x1230 candidate rows to save-oracle market stockpile.

The candidate is only accepted when a same-tick save oracle agrees on the
per-country type-70 equipment id and amount rows. Otherwise this proof stays
fail-closed and explains whether the blocker is a stale fixture or a rejected
root.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import time
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path

_HERE = Path(__file__).resolve()
_REPO = _HERE.parents[4]
sys.path.insert(0, str(_REPO / "memory_reader"))
sys.path.insert(0, str(_HERE))

import market_trade_live_compact as mt  # noqa: E402
import world_extract_live as world_live  # noqa: E402
from market_stockpile_root_probe import country_maps_with_dynamic_fallback  # noqa: E402


COUNTRY_MARKET_STOCKPILE_CANDIDATE_OFF = 0x1230
ROW_STRIDE = 0x10
ROW_DEF_OFF = 0x00
ROW_AMOUNT_OFF = 0x08
SCALE_RAW = 100000
MAX_ROWS = 4096
MAX_AMOUNT_RAW = 1_000_000_000_000_000


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) and value else None


def read_u32(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 4 > len(blob):
        return None
    return struct.unpack_from("<I", blob, off)[0]


def read_u64(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 8 > len(blob):
        return None
    return struct.unpack_from("<Q", blob, off)[0]


def read_i64(blob: bytes, off: int) -> int | None:
    if off < 0 or off + 8 > len(blob):
        return None
    return struct.unpack_from("<q", blob, off)[0]


def date_to_raw(value: str | None) -> int | None:
    if not value:
        return None
    try:
        return world_live.encode_datehour(value)
    except Exception:
        return None


def read_live_candidate_rows(p: mt.winmem.Process) -> tuple[list[dict], dict]:
    maps = country_maps_with_dynamic_fallback(p)
    variant_vtable = p.module_base + mt.VARIANT_VTABLE_RVA
    rows = []
    countries_seen = 0
    malformed_headers = 0
    bad_gated = 0
    for tag_id, country_ptr in sorted(maps["id_to_country_dynamic"].items()):
        tag = maps["id_to_tag"].get(tag_id)
        if not tag:
            continue
        header = p.try_read(country_ptr + COUNTRY_MARKET_STOCKPILE_CANDIDATE_OFF, 16)
        if len(header) < 16:
            malformed_headers += 1
            continue
        begin = read_u64(header, 0)
        cap = read_u32(header, 8)
        size = read_u32(header, 12)
        if not mt.is_heap(begin) or size is None or cap is None or size > cap or size > MAX_ROWS:
            continue
        countries_seen += 1
        blob = p.try_read(begin, size * ROW_STRIDE)
        if len(blob) < size * ROW_STRIDE:
            malformed_headers += 1
            continue
        for idx in range(size):
            base = idx * ROW_STRIDE
            def_ptr = read_u64(blob, base + ROW_DEF_OFF)
            amount_raw = read_i64(blob, base + ROW_AMOUNT_OFF)
            if not mt.is_heap(def_ptr) or amount_raw is None:
                continue
            if abs(amount_raw) > MAX_AMOUNT_RAW:
                bad_gated += 1
                continue
            if mt.safe_u64(p, def_ptr) != variant_vtable:
                bad_gated += 1
                continue
            if mt.safe_u32(p, def_ptr + mt.VARIANT_TYPE_OFF) != mt.VARIANT_TYPE:
                bad_gated += 1
                continue
            equip_id = mt.safe_u32(p, def_ptr + mt.VARIANT_ID_OFF)
            if not equip_id or amount_raw == 0:
                continue
            rows.append(
                {
                    "tag_db_id": tag_id,
                    "tag": tag,
                    "country_ptr": hx(country_ptr),
                    "begin": hx(begin),
                    "index": idx,
                    "equip_id": equip_id,
                    "equip_type": mt.VARIANT_TYPE,
                    "amount_raw": amount_raw,
                    "amount": amount_raw / SCALE_RAW,
                }
            )
    stats = {
        "countries_with_candidate_header": countries_seen,
        "malformed_headers": malformed_headers,
        "bad_gated_rows": bad_gated,
    }
    return rows, stats


def oracle_rows(market_json: dict) -> list[dict]:
    rows = []
    countries = market_json.get("countries") if isinstance(market_json, dict) else None
    if not isinstance(countries, dict):
        return rows
    for tag, rec in countries.items():
        if not isinstance(rec, dict):
            continue
        stockpile = rec.get("market_stockpile")
        if not isinstance(stockpile, list):
            continue
        for entry in stockpile:
            if not isinstance(entry, dict):
                continue
            equip_id = entry.get("equip_id")
            equip_type = entry.get("equip_type")
            amount = entry.get("amount")
            if equip_id is None or equip_type is None or amount is None:
                continue
            rows.append(
                {
                    "tag": tag,
                    "equip_id": int(equip_id),
                    "equip_type": int(equip_type),
                    "amount": float(amount),
                }
            )
    return rows


def key(row: dict) -> tuple:
    return row["tag"], row["equip_id"], row["equip_type"]


def compare_rows(live_rows: list[dict], save_rows: list[dict]) -> dict:
    live = {key(row): row for row in live_rows}
    save = {key(row): row for row in save_rows}
    matched = []
    amount_mismatches = []
    for item_key in sorted(set(live) & set(save)):
        live_amount = float(live[item_key]["amount"])
        save_amount = float(save[item_key]["amount"])
        if abs(live_amount - save_amount) <= 0.00001:
            matched.append(item_key)
        else:
            amount_mismatches.append(
                {
                    "tag": item_key[0],
                    "equip_id": item_key[1],
                    "equip_type": item_key[2],
                    "live_amount": live_amount,
                    "save_amount": save_amount,
                    "delta": live_amount - save_amount,
                }
            )
    live_only = sorted(set(live) - set(save))
    save_only = sorted(set(save) - set(live))
    by_tag = defaultdict(lambda: {"matched": 0, "live_only": 0, "save_only": 0, "mismatch": 0})
    for tag, _, _ in matched:
        by_tag[tag]["matched"] += 1
    for tag, _, _ in live_only:
        by_tag[tag]["live_only"] += 1
    for tag, _, _ in save_only:
        by_tag[tag]["save_only"] += 1
    for row in amount_mismatches:
        by_tag[row["tag"]]["mismatch"] += 1
    return {
        "live_row_count": len(live_rows),
        "save_row_count": len(save_rows),
        "matched_count": len(matched),
        "amount_mismatch_count": len(amount_mismatches),
        "live_only_count": len(live_only),
        "save_only_count": len(save_only),
        "matched_examples": [
            {"tag": tag, "equip_id": equip_id, "equip_type": equip_type}
            for tag, equip_id, equip_type in matched[:24]
        ],
        "amount_mismatch_examples": amount_mismatches[:24],
        "live_only_examples": [
            {"tag": tag, "equip_id": equip_id, "equip_type": equip_type}
            for tag, equip_id, equip_type in live_only[:24]
        ],
        "save_only_examples": [
            {"tag": tag, "equip_id": equip_id, "equip_type": equip_type}
            for tag, equip_id, equip_type in save_only[:24]
        ],
        "tag_summary": dict(sorted(by_tag.items())) if len(by_tag) <= 64 else dict(sorted(by_tag.items())[:64]),
    }


def classify(same_tick: bool, live_stats: dict, comparison: dict) -> dict:
    if live_stats["bad_gated_rows"]:
        return {
            "accepted_for_emission": False,
            "status": "rejected",
            "reason": "CCountry+0x1230 contains rows that fail type-70 CEquipmentVariant gating.",
        }
    if not same_tick:
        return {
            "accepted_for_emission": False,
            "status": "fixture_blocked",
            "reason": "Save oracle and live process are not same-tick; comparison cannot validate or reject the stockpile root.",
        }
    if comparison["save_row_count"] == 0:
        return {
            "accepted_for_emission": False,
            "status": "fixture_blocked",
            "reason": "Same-tick save has no market_stockpile rows to validate the candidate.",
        }
    if (
        comparison["matched_count"] == comparison["save_row_count"]
        and comparison["live_only_count"] == 0
        and comparison["amount_mismatch_count"] == 0
    ):
        return {
            "accepted_for_emission": True,
            "status": "validated",
            "reason": "CCountry+0x1230 exactly matches same-tick save-oracle market_stockpile rows.",
        }
    return {
        "accepted_for_emission": False,
        "status": "rejected",
        "reason": "Same-tick save-oracle market_stockpile rows do not match CCountry+0x1230 candidate rows.",
    }


def probe(args: argparse.Namespace) -> dict:
    started = time.perf_counter()
    market_json = json.loads(Path(args.market_json).read_text(encoding="utf-8"))
    with mt.winmem.Process(args.pid, mt.PROC_NAME) as p:
        fp = p.fingerprint()
        sha = str(fp.get("sha256") or "").lower()
        if sha != mt.EXPECTED_SHA256 and not args.force:
            raise SystemExit(f"live hoi4.exe sha256 {sha} != expected {mt.EXPECTED_SHA256}")
        live_raw = mt.read_game_hour_raw(p)
        live_date = world_live.decode_datehour(live_raw)
        save_date = (market_json.get("meta") or {}).get("save_date")
        save_raw = date_to_raw(save_date)
        live_rows, live_stats = read_live_candidate_rows(p)
    save_rows = oracle_rows(market_json)
    comparison = compare_rows(live_rows, save_rows)
    same_tick = bool(live_raw is not None and save_raw is not None and live_raw == save_raw)
    proof = {
        "schema": "market_trade_live_stockpile_save_compare_v1",
        "utc": datetime.now(timezone.utc).isoformat(),
        "pid": args.pid,
        "module": {
            "path": fp.get("path"),
            "sha256": sha,
        },
        "candidate": {
            "country_offset": COUNTRY_MARKET_STOCKPILE_CANDIDATE_OFF,
            "country_offset_hex": hx(COUNTRY_MARKET_STOCKPILE_CANDIDATE_OFF),
            "row_stride": ROW_STRIDE,
            "def_off": ROW_DEF_OFF,
            "amount_off": ROW_AMOUNT_OFF,
            "amount_scale": SCALE_RAW,
        },
        "fixture": {
            "market_json": str(args.market_json),
            "save_filename": (market_json.get("meta") or {}).get("source_filename"),
            "save_date": save_date,
            "save_date_raw": save_raw,
            "live_date": live_date,
            "live_date_raw": live_raw,
            "same_tick": same_tick,
            "delta_hours": (live_raw - save_raw)
            if isinstance(live_raw, int) and isinstance(save_raw, int)
            else None,
        },
        "live_candidate": live_stats,
        "comparison": comparison,
        "classification": classify(same_tick, live_stats, comparison),
        "elapsed_ms": round((time.perf_counter() - started) * 1000, 3),
    }
    if args.out_json:
        out_path = Path(args.out_json)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(json.dumps(proof, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return proof


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--market-json", required=True)
    parser.add_argument("--out-json", required=True)
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args()
    proof = probe(args)
    print(json.dumps(proof["classification"], indent=2, sort_keys=True))
    print(json.dumps(proof["fixture"], indent=2, sort_keys=True))
    print(json.dumps({k: proof["comparison"][k] for k in (
        "live_row_count",
        "save_row_count",
        "matched_count",
        "amount_mismatch_count",
        "live_only_count",
        "save_only_count",
    )}, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
