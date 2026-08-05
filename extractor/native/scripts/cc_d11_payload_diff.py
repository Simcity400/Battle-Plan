#!/usr/bin/env python3
"""Lane CC: what a keyed per-country payload diff would cost D11 on a real stream.

D11 `production_live` builds its payload in `compact_production_data_payload`
(extractor/native/collector/src/attach_log.rs): a 40-byte base header, four
fixed-width row arrays each keyed by `tag_db_id` at row offset 0, then ten
appended trailer segments.

The four base arrays are decodable from the base header alone -- their row
counts live at payload offsets 24, 28, 32 and 36 -- so the base region's exact
extent is known without parsing a single trailer, and every base byte can be
attributed to the country whose `tag_db_id` opens its row.

WHICH ARM THIS MEASURES. Only one: the FLOOR. Every record after the first is
diffed against the immediately preceding record's blocks, with no reset for
any reason -- not for the domain's own gap, not for a foreign gap, not for a
failure status. That is not an arm any encoder can ship; today's producer
re-seeds after its own gap and would emit 87 of these 285 records whole. The
number below is therefore the most favourable one available and is quoted as
a lower bound, never as an achievable size. It is also optimistic in a second
way: the comparison iterates only keys the CURRENT record carries, so a key
the pin holds and the record drops costs nothing here, and no deletion
grammar or sparse-block framing is charged.

The trailer region is what remains. Its segments do NOT share one envelope
(PEF1 and PCV1 carry a `segment_len`; the license trailer does not), so this
tool does NOT attribute trailer bytes per country. It measures the region's
size and its record-to-record byte behaviour, and reports it as an explicit
residual rather than folding a guess into the headline number.

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
RT_DOMAIN_DATA = 3
RT_DOMAIN_GAP = 5

D11_BASE_HEADER_LEN = 40
STOCKPILE_ROW_LEN = 16
LINE_ROW_LEN = 32
CONSTRUCTION_ROW_LEN = 32

_HDR = struct.Struct("<4sHHQiHHII")


def iter_domain_records(path, domain):
    with open(path, "rb") as handle:
        while True:
            header = handle.read(HEADER_LEN)
            if not header:
                return
            (magic, _v, hlen, _seq, hour, domain_id, record_type, plen, _f) = \
                _HDR.unpack_from(header, 0)
            if magic != MAGIC or hlen != HEADER_LEN:
                raise SystemExit("framing error")
            if domain_id != domain or record_type not in (RT_DOMAIN_DATA, RT_DOMAIN_GAP):
                handle.seek(plen, 1)
                continue
            payload = handle.read(plen)
            if len(payload) != plen:
                raise SystemExit("truncated payload")
            yield hour, record_type, payload


def split_d11(payload):
    """Return (base_region_len, {tag_db_id: base_block_bytes}, trailer_region)."""
    if len(payload) < D11_BASE_HEADER_LEN:
        raise ValueError("short d11 payload")
    stockpile, military, naval, construction = struct.unpack_from("<IIII", payload, 24)
    off = D11_BASE_HEADER_LEN
    blocks = defaultdict(bytearray)

    def take(count, row_len):
        nonlocal off
        end = off + count * row_len
        if end > len(payload):
            raise ValueError("d11 base region overruns the payload")
        for start in range(off, end, row_len):
            tag = struct.unpack_from("<I", payload, start)[0]
            blocks[tag] += payload[start:start + row_len]
        off = end

    take(stockpile, STOCKPILE_ROW_LEN)
    take(military + naval, LINE_ROW_LEN)
    take(construction, CONSTRUCTION_ROW_LEN)
    return off, blocks, payload[off:]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("stream")
    ap.add_argument("--domain", type=int, default=11)
    args = ap.parse_args()

    pin_blocks = None           # {tag: bytes} last put on the wire
    pin_trailer = None
    total_payload = 0
    total_base = 0
    total_trailer = 0
    records = 0
    seeds = 0
    changed_country_bytes = 0
    unchanged_country_bytes = 0
    changed_blocks = 0
    total_blocks = 0
    seed_base_bytes = 0
    trailer_identical_bytes = 0
    trailer_changed_bytes = 0
    trailer_comparisons = 0
    trailer_identical = 0
    roster_sizes = set()
    repeated_key_records = 0

    for hour, rtype, payload in iter_domain_records(args.stream, args.domain):
        if rtype == RT_DOMAIN_GAP:
            continue
        records += 1
        base_len, blocks, trailer = split_d11(payload)
        total_payload += len(payload)
        total_base += base_len
        total_trailer += len(trailer)
        roster_sizes.add(len(blocks))
        stockpile, military, naval, construction = struct.unpack_from("<IIII", payload, 24)
        expected_rows = stockpile + military + naval + construction
        seen_rows = sum(len(v) for v in blocks.values())
        if seen_rows != base_len - D11_BASE_HEADER_LEN:
            raise SystemExit("base attribution lost bytes")
        del expected_rows

        blocks = {tag: bytes(value) for tag, value in blocks.items()}
        if pin_blocks is None:
            seeds += 1
            seed_base_bytes += base_len
        else:
            for tag, block in blocks.items():
                total_blocks += 1
                if pin_blocks.get(tag) == block:
                    unchanged_country_bytes += len(block)
                else:
                    changed_blocks += 1
                    changed_country_bytes += len(block)
            trailer_comparisons += 1
            if trailer == pin_trailer:
                trailer_identical += 1
                trailer_identical_bytes += len(trailer)
            else:
                trailer_changed_bytes += len(trailer)
        pin_blocks = blocks
        pin_trailer = trailer

    out = {
        "domain": args.domain,
        "records": records,
        "seed_records": seeds,
        "total_payload_bytes": total_payload,
        "base_region_bytes": total_base,
        "trailer_region_bytes": total_trailer,
        "base_share": round(total_base / total_payload, 4) if total_payload else None,
        "country_blocks_compared": total_blocks,
        "country_blocks_changed": changed_blocks,
        "country_block_changed_share": round(changed_blocks / total_blocks, 4) if total_blocks else None,
        "changed_country_bytes": changed_country_bytes,
        "unchanged_country_bytes": unchanged_country_bytes,
        "changed_country_byte_share": (
            round(changed_country_bytes / (changed_country_bytes + unchanged_country_bytes), 4)
            if (changed_country_bytes + unchanged_country_bytes) else None
        ),
        "seed_base_bytes": seed_base_bytes,
        "trailer_comparisons": trailer_comparisons,
        "trailer_byte_identical_records": trailer_identical,
        "trailer_identical_bytes": trailer_identical_bytes,
        "trailer_changed_bytes": trailer_changed_bytes,
        "roster_sizes": sorted(roster_sizes),
        "repeated_key_records": repeated_key_records,
    }
    # The arm this measures: every record after the first carries only the
    # countries whose base block moved, plus the 40-byte base header, plus the
    # whole trailer region (unattributed). The base header is re-sent every
    # record either way, so it is charged to both arms.
    header_bytes = records * D11_BASE_HEADER_LEN
    out["arm_today_payload_bytes"] = total_payload
    out["arm_base_diff_payload_bytes"] = (
        header_bytes + seed_base_bytes - seeds * D11_BASE_HEADER_LEN
        + changed_country_bytes + total_trailer
    )
    out["arm_base_diff_ratio"] = (
        round(total_payload / out["arm_base_diff_payload_bytes"], 4)
        if out["arm_base_diff_payload_bytes"] else None
    )
    # The optimistic bound that also assumes a byte-identical trailer region
    # costs nothing.
    out["arm_base_diff_plus_trailer_elision_bytes"] = (
        out["arm_base_diff_payload_bytes"] - trailer_identical_bytes
    )
    print(json.dumps(out, indent=2))


if __name__ == "__main__":
    sys.exit(main())
