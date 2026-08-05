#!/usr/bin/env python3
"""Lane CC: per-lever byte accounting over an H4CR collector stream.

Pass 1 (framing only): walks every record, splits each domain's DOMAIN_DATA
bytes by whether a DOMAIN_GAP for THAT domain intervened since the domain's
previous DOMAIN_DATA record, and reports the delta/keyframe split from the
payload status word.

This is the method §2.3 / §7 of DELTA_WIRE_DESIGN.md use; it is reproduced here
so the ranking can be taken over every domain rather than the seven that were
tabulated by hand.

Read-only with respect to the corpus.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
from collections import defaultdict

MAGIC = b"H4CR"
HEADER_LEN = 32
RT_DOMAIN_PERF = 1
RT_FRAME_END = 2
RT_DOMAIN_DATA = 3
RT_STREAM_HEADER = 4
RT_DOMAIN_GAP = 5
RT_FRAME_REFUSED = 6
RT_FRAME_DEGRADED = 7

STATUS_DELTA_DATA = 1 << 10

# Domains that do NOT use the shared 20-byte DOMAIN_DATA payload header, so
# reading a status word at payload[16..20] is not promised for them
# (DELTA_WIRE_DESIGN.md §2.3, "two caveats on the table").
NON_SHARED_HEADER_DOMAINS = {48, 53, 83, 91, 93}

_HDR = struct.Struct("<4sHHQiHHII")


def walk(path):
    """Yield (offset, sequence, game_hour, domain_id, record_type, payload_len,
    record_flags, payload_memoryview)."""
    # Plain buffered reads, not mmap. An mmap here has to hand the caller a
    # memoryview slice per record, and any one of those slices that outlives
    # the walk keeps the mapping exported, so the file stays open on Windows
    # for the life of the process and nothing downstream can delete or rewrite
    # what it just read. The walk is sequential and header-dominated, so the
    # mapping bought nothing to pay for that.
    with open(path, "rb") as handle:
        off = 0
        while True:
            header = handle.read(HEADER_LEN)
            if not header:
                break
            if len(header) < HEADER_LEN:
                raise SystemExit(
                    f"truncated record header at offset {off}: {len(header)} byte(s)"
                )
            (magic, version, header_len, sequence, game_hour, domain_id,
             record_type, payload_len, record_flags) = _HDR.unpack_from(header, 0)
            if magic != MAGIC:
                raise SystemExit(f"bad magic at offset {off}")
            if header_len != HEADER_LEN:
                raise SystemExit(f"unexpected header_len {header_len} at {off}")
            payload = handle.read(payload_len)
            if len(payload) != payload_len:
                raise SystemExit(
                    f"truncated record at {off}: payload_len={payload_len} "
                    f"but only {len(payload)} bytes remain"
                )
            yield (off, sequence, game_hour, domain_id, record_type,
                   payload_len, record_flags, payload)
            off += HEADER_LEN + payload_len


def status_of(domain_id, payload):
    if domain_id in NON_SHARED_HEADER_DOMAINS:
        return None
    if len(payload) < 20:
        return None
    return struct.unpack_from("<I", payload, 16)[0]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("stream")
    ap.add_argument("--out", default=None)
    args = ap.parse_args()

    # per-domain accumulators
    stats = defaultdict(lambda: {
        "data_records": 0,
        "data_payload_bytes": 0,
        "data_record_bytes": 0,
        "gaps": 0,
        # split by "did a DOMAIN_GAP for this domain intervene since my last DATA?"
        "after_own_gap_records": 0,
        "after_own_gap_payload": 0,
        "steady_records": 0,
        "steady_payload": 0,
        # first DATA record of the domain (seeds the pin) -- counted in neither split
        "seed_records": 0,
        "seed_payload": 0,
        # delta bit
        "delta_records": 0,
        "delta_payload": 0,
        "keyframe_records": 0,
        "keyframe_payload": 0,
        "unknown_status_records": 0,
    })
    gap_since_last_data = defaultdict(bool)
    seen_data = defaultdict(bool)
    record_type_counts = defaultdict(int)
    record_type_bytes = defaultdict(int)
    total_bytes = 0
    total_records = 0
    hours = set()
    frame_end_count = 0

    for (off, seq, hour, domain_id, rtype, plen, flags, payload) in walk(args.stream):
        total_records += 1
        total_bytes += HEADER_LEN + plen
        record_type_counts[rtype] += 1
        record_type_bytes[rtype] += HEADER_LEN + plen
        if rtype == RT_FRAME_END:
            frame_end_count += 1
        if rtype in (RT_DOMAIN_DATA, RT_DOMAIN_GAP, RT_DOMAIN_PERF):
            hours.add(hour)
        if rtype == RT_DOMAIN_GAP:
            stats[domain_id]["gaps"] += 1
            gap_since_last_data[domain_id] = True
            continue
        if rtype != RT_DOMAIN_DATA:
            continue

        s = stats[domain_id]
        s["data_records"] += 1
        s["data_payload_bytes"] += plen
        s["data_record_bytes"] += HEADER_LEN + plen

        status = status_of(domain_id, payload)
        if status is None:
            s["unknown_status_records"] += 1
        elif status & STATUS_DELTA_DATA:
            s["delta_records"] += 1
            s["delta_payload"] += plen
        else:
            s["keyframe_records"] += 1
            s["keyframe_payload"] += plen

        if not seen_data[domain_id]:
            seen_data[domain_id] = True
            s["seed_records"] += 1
            s["seed_payload"] += plen
        elif gap_since_last_data[domain_id]:
            s["after_own_gap_records"] += 1
            s["after_own_gap_payload"] += plen
        else:
            s["steady_records"] += 1
            s["steady_payload"] += plen
        gap_since_last_data[domain_id] = False

    out = {
        "stream": args.stream,
        "total_bytes": total_bytes,
        "total_records": total_records,
        "distinct_game_hours": len(hours),
        "frame_end_records": frame_end_count,
        "record_type_counts": {str(k): v for k, v in sorted(record_type_counts.items())},
        "record_type_bytes": {str(k): v for k, v in sorted(record_type_bytes.items())},
        "domains": {},
    }
    for domain_id in sorted(stats):
        s = stats[domain_id]
        steady_mean = (s["steady_payload"] / s["steady_records"]) if s["steady_records"] else None
        gap_mean = (s["after_own_gap_payload"] / s["after_own_gap_records"]) if s["after_own_gap_records"] else None
        delta_mean = (s["delta_payload"] / s["delta_records"]) if s["delta_records"] else None
        # NAIVE prize only, kept because it is the number the earlier hand
        # tabulations used and a reader comparing against them needs it.
        # It assumes a post-own-gap record shrinks to this domain's mean steady
        # payload, which is wrong twice: the mean averages populations that do
        # not belong together (d47 and d51 emit ~0.1 KB most hours and ~931 KB /
        # ~572 KB on the rest), and a record spanning k hours carries more than
        # one hour's change. `cc_pin_prize_v2.py` is the number to quote; this
        # one overstates it by 3x on t26 (752.64 MiB against 255.10 MiB).
        if steady_mean is not None and gap_mean is not None and gap_mean > steady_mean:
            pin_prize = int(round(s["after_own_gap_payload"] - s["after_own_gap_records"] * steady_mean))
        else:
            pin_prize = 0
        rec = dict(s)
        rec["steady_mean_payload"] = steady_mean
        rec["after_own_gap_mean_payload"] = gap_mean
        rec["delta_mean_payload"] = delta_mean
        rec["own_gap_ratio"] = (gap_mean / steady_mean) if (steady_mean and gap_mean) else None
        rec["pin_restoration_prize_bytes"] = pin_prize
        out["domains"][str(domain_id)] = rec

    text = json.dumps(out, indent=2)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as handle:
            handle.write(text)
        print(f"wrote {args.out}")
    else:
        print(text)


if __name__ == "__main__":
    sys.exit(main())
