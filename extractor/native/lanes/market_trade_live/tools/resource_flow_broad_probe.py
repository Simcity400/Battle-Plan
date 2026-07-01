"""Broad bounded rejection probe for country resource-flow vector roots.

The first resource probe rejected the `CCountry+0x5B8` clue and nearby headers.
This follow-up scans the wider market/economy part of CCountry for compact
resource-id/amount vectors. It does not try to prove arbitrary inline scalar
arrays; it only rejects the bounded vector-header candidate family.
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

_HERE = Path(__file__).resolve()
_REPO = _HERE.parents[5]
sys.path.insert(0, str(_REPO / "extractor" / "memory_reader"))
sys.path.insert(0, str(_HERE))

import market_trade_live_compact as mt  # noqa: E402
from resource_flow_root_probe import country_maps_with_dynamic_fallback  # noqa: E402


SCAN_START = 0x400
SCAN_END = 0x1800
MAX_VECTOR_ROWS = 64
RESOURCE_ID_MAX = 32
RESOURCE_AMOUNT_ABS_MAX = 10_000_000_000
SAMPLE_TAGS = {
    "GER",
    "ENG",
    "SOV",
    "USA",
    "JAP",
    "ITA",
    "FRA",
    "BRA",
    "ROM",
    "HUN",
    "TUR",
    "SWE",
    "MEX",
    "ARG",
    "CAN",
    "AST",
}


def hx(value: int | None) -> str | None:
    return "0x%X" % value if isinstance(value, int) else None


def read_i32(raw: bytes, off: int) -> int | None:
    if off < 0 or off + 4 > len(raw):
        return None
    return struct.unpack_from("<i", raw, off)[0]


def read_u32(raw: bytes, off: int) -> int | None:
    if off < 0 or off + 4 > len(raw):
        return None
    return struct.unpack_from("<I", raw, off)[0]


def read_i64(raw: bytes, off: int) -> int | None:
    if off < 0 or off + 8 > len(raw):
        return None
    return struct.unpack_from("<q", raw, off)[0]


def read_u64(raw: bytes, off: int) -> int | None:
    if off < 0 or off + 8 > len(raw):
        return None
    return struct.unpack_from("<Q", raw, off)[0]


def plausible_resource_id(value: int | None) -> bool:
    return isinstance(value, int) and 0 <= value <= RESOURCE_ID_MAX


def plausible_amount(value: int | None) -> bool:
    return isinstance(value, int) and abs(value) <= RESOURCE_AMOUNT_ABS_MAX


def score_vector(raw: bytes, size: int, stride: int, resource_off: int, amount_off: int, amount_width: int):
    if size <= 0 or size > MAX_VECTOR_ROWS or len(raw) < size * stride:
        return None
    resource_ids = []
    nonzero = 0
    preview = []
    for idx in range(size):
        base = idx * stride
        resource_id = read_u32(raw, base + resource_off)
        amount = read_i64(raw, base + amount_off) if amount_width == 8 else read_i32(raw, base + amount_off)
        if not plausible_resource_id(resource_id) or not plausible_amount(amount):
            return None
        resource_ids.append(resource_id)
        if amount:
            nonzero += 1
        if len(preview) < 12:
            preview.append({"resource_id": resource_id, "amount_raw": amount})
    duplicate_ids = len(resource_ids) - len(set(resource_ids))
    return {
        "resource_ids": resource_ids,
        "nonzero_amounts": nonzero,
        "duplicate_ids": duplicate_ids,
        "rows_preview": preview,
    }


def scan_country(p: mt.winmem.Process, tag_id: int, tag: str, country_ptr: int) -> list[dict]:
    blob = p.try_read(country_ptr + SCAN_START, SCAN_END - SCAN_START)
    if len(blob) < SCAN_END - SCAN_START:
        return []
    out = []
    layouts = (
        (0, 4, 4),
        (4, 0, 4),
        (0, 8, 8),
        (8, 0, 8),
        (8, 16, 8),
        (16, 8, 8),
    )
    for rel in range(0, len(blob) - 16, 8):
        begin = read_u64(blob, rel)
        cap = read_u32(blob, rel + 8)
        size = read_u32(blob, rel + 12)
        if not mt.is_heap(begin) or size is None or cap is None:
            continue
        if size == 0 or size > cap or cap > MAX_VECTOR_ROWS:
            continue
        for stride in (8, 12, 16, 24, 32):
            raw = p.try_read(begin, size * stride)
            if len(raw) < size * stride:
                continue
            for amount_off, resource_off, amount_width in layouts:
                if max(amount_off + amount_width, resource_off + 4) > stride:
                    continue
                scored = score_vector(raw, size, stride, resource_off, amount_off, amount_width)
                if not scored:
                    continue
                out.append(
                    {
                        "tag_db_id": tag_id,
                        "tag": tag,
                        "country_ptr": hx(country_ptr),
                        "country_offset": SCAN_START + rel,
                        "country_offset_hex": hx(SCAN_START + rel),
                        "begin": hx(begin),
                        "size": size,
                        "cap": cap,
                        "stride": stride,
                        "amount_off": amount_off,
                        "amount_width": amount_width,
                        "resource_off": resource_off,
                        **scored,
                    }
                )
    return out


def summarize(candidates: list[dict], countries_scanned: int) -> dict:
    groups = defaultdict(list)
    for row in candidates:
        key = (
            row["country_offset"],
            row["stride"],
            row["amount_off"],
            row["amount_width"],
            row["resource_off"],
        )
        groups[key].append(row)
    summaries = []
    for (country_off, stride, amount_off, amount_width, resource_off), rows in groups.items():
        resource_sequences = Counter(tuple(row["resource_ids"]) for row in rows)
        tag_count = len({row["tag_db_id"] for row in rows})
        nonzero_tags = sum(1 for row in rows if row["nonzero_amounts"] > 0)
        duplicate_tags = sum(1 for row in rows if row["duplicate_ids"] > 0)
        summaries.append(
            {
                "country_offset": country_off,
                "country_offset_hex": hx(country_off),
                "stride": stride,
                "amount_off": amount_off,
                "amount_width": amount_width,
                "resource_off": resource_off,
                "tag_count": tag_count,
                "nonzero_tag_count": nonzero_tags,
                "duplicate_id_tag_count": duplicate_tags,
                "distinct_resource_id_sequences": len(resource_sequences),
                "top_resource_id_sequences": [
                    {"resource_ids": list(ids), "count": count}
                    for ids, count in resource_sequences.most_common(5)
                ],
                "examples": rows[:8],
            }
        )
    summaries.sort(
        key=lambda row: (
            -row["tag_count"],
            -row["nonzero_tag_count"],
            row["duplicate_id_tag_count"],
            row["distinct_resource_id_sequences"],
            row["country_offset"],
            row["stride"],
        )
    )
    accepted_like = [
        row
        for row in summaries
        if row["tag_count"] >= max(6, countries_scanned // 3)
        and row["nonzero_tag_count"] >= 2
        and row["duplicate_id_tag_count"] == 0
        and row["distinct_resource_id_sequences"] <= 3
    ]
    return {
        "candidate_layout_count": len(candidates),
        "candidate_group_count": len(summaries),
        "accepted_like_group_count": len(accepted_like),
        "accepted_for_emission": False,
        "best_groups": summaries[:16],
        "classification": (
            "rejected_vector_candidate_family"
            if not accepted_like
            else "unaccepted_vector_candidates_need_fixture"
        ),
        "reason": (
            "No stable compact resource-id/amount vector root was found in "
            "CCountry+0x400..0x1800."
            if not accepted_like
            else "Stable-looking compact vectors exist but require same-tick "
            "country resource oracle values before emission."
        ),
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
        selected = []
        for tag_id, tag in sorted(maps["id_to_tag"].items()):
            ptr = maps["id_to_country_dynamic"].get(tag_id)
            if not ptr:
                continue
            if tag in SAMPLE_TAGS or len(selected) < args.min_countries:
                selected.append((tag_id, tag, ptr))
            if len(selected) >= args.max_countries:
                break
        candidates = []
        for tag_id, tag, ptr in selected:
            candidates.extend(scan_country(p, tag_id, tag, ptr))
        summary = summarize(candidates, len(selected))
        proof = {
            "schema": "market_trade_live_resource_flow_broad_probe_v1",
            "utc": datetime.now(timezone.utc).isoformat(),
            "pid": pid,
            "module": {
                "path": fp.get("path"),
                "base": hx(p.module_base),
                "sha256": sha,
            },
            "scan": {
                "range": "CCountry+0x400..0x1800",
                "scan_start": hx(SCAN_START),
                "scan_end": hx(SCAN_END),
                "countries_scanned": len(selected),
                "max_vector_rows": MAX_VECTOR_ROWS,
                "resource_id_max": RESOURCE_ID_MAX,
            },
            "summary": summary,
            "exact_result": (
                "Reject compact-vector resource flow roots in CCountry+0x400..0x1800"
                if summary["classification"] == "rejected_vector_candidate_family"
                else "Compact-vector resource flow roots remain fixture-blocked"
            ),
            "elapsed_ms": round((time.perf_counter() - started) * 1000, 3),
        }
    if args.out_json:
        out = Path(args.out_json)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(proof, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return proof


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int)
    parser.add_argument("--out-json", required=True)
    parser.add_argument("--max-countries", type=int, default=96)
    parser.add_argument("--min-countries", type=int, default=32)
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args()
    proof = probe(args)
    print(json.dumps(proof["summary"], indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
