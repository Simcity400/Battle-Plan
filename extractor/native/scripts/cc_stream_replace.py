#!/usr/bin/env python3
"""Lane CC: compress a REPLACEMENT stream, not a record-deleted one.

Deleting a record and compressing what is left does not bound what a real lever
saves. zstd's output is not monotone in its input: removing bytes removes match
sources as well as matched bytes, and on this corpus deleting d53's 460 MB of
post-own-gap records made the compressed stream 263,244 bytes LARGER. A deletion
ablation is therefore a reference point, never a ceiling.

What a retained delta pin actually does is REPLACE a large post-own-gap record
with a smaller one. So this tool builds that stream: every post-own-gap
DOMAIN_DATA record is replaced by a real record of the SAME DOMAIN taken from
the corpus itself, chosen so its payload length is closest to the size the pin
model predicts for a record spanning that many hours
(`whole * (1 - (1-q)^k)`, see cc_pin_prize_v2.py). Real bytes are used because
synthetic filler would compress unlike anything the producer emits.

The substitution is deterministic: no randomness, and the candidate pool is the
domain's own steady-state records, so the spliced payload is a payload that
domain really produced.

WHAT THIS IS NOT. The spliced record is a payload the domain produced for a
DIFFERENT hour, so this is an estimate of the compressed size, not a
reconstruction, and the estimate's error is not bounded. Two specific biases
are handled rather than assumed away -- donor rotation, so the same bytes are
not spliced into many records and rewarded as repetition the producer would
never emit; and a two-point realisation for bimodal domains, so a target
between the two modes is not quantised onto one of them. A third is not: the
real encoder would emit a patch whose content correlates with the surrounding
hours, and a donor from elsewhere in the capture does not. Whether that is
optimistic or pessimistic is not established here.

Read-only with respect to the corpus.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import time
from collections import defaultdict

import zstandard as zstd

MAGIC = b"H4CR"
H = 32
HDR = struct.Struct("<4sHHQiHHII")
RT_DOMAIN_DATA = 3
RT_DOMAIN_GAP = 5
STATUS_DELTA_DATA = 1 << 10
NON_SHARED_HEADER_DOMAINS = {48, 53, 83, 91, 93}
ZSTD_LEVEL = 3
ZSTD_WINDOW_LOG = 27


def compressor():
    params = zstd.ZstdCompressionParameters.from_level(
        ZSTD_LEVEL, window_log=ZSTD_WINDOW_LOG, enable_ldm=True, write_checksum=1
    )
    return zstd.ZstdCompressor(compression_params=params)


def survey(path):
    """Pass 1: per-domain record inventory (offset, payload_len, bucket, delta bit)."""
    gap_run = defaultdict(int)
    seen = defaultdict(bool)
    steady = defaultdict(list)     # domain -> [(payload_len, payload_offset)]
    postgap = defaultdict(list)    # domain -> [(record_offset, payload_len, gap_run, is_delta)]
    with open(path, "rb") as handle:
        offset = 0
        while True:
            header = handle.read(H)
            if not header:
                break
            (_m, _v, _h, _s, _hour, domain, rtype, plen, _f) = HDR.unpack_from(header, 0)
            payload_offset = offset + H
            if rtype == RT_DOMAIN_GAP:
                handle.seek(plen, 1)
                gap_run[domain] += 1
                offset = payload_offset + plen
                continue
            if rtype != RT_DOMAIN_DATA:
                handle.seek(plen, 1)
                offset = payload_offset + plen
                continue
            head = handle.read(min(plen, 20))
            handle.seek(plen - len(head), 1)
            if domain in NON_SHARED_HEADER_DOMAINS or len(head) < 20:
                is_delta = None
            else:
                is_delta = bool(struct.unpack_from("<I", head, 16)[0] & STATUS_DELTA_DATA)
            if seen[domain]:
                if gap_run[domain] > 0:
                    postgap[domain].append((offset, plen, gap_run[domain], is_delta))
                else:
                    steady[domain].append((plen, payload_offset))
            seen[domain] = True
            gap_run[domain] = 0
            offset = payload_offset + plen
    return steady, postgap


MIN_CLUSTER_SHARE = 0.10
MAX_CLUSTER_SPREAD = 4.0
MIN_CLUSTER_SEPARATION = 10.0


def is_bimodal(sizes):
    """True only when the domain's steady sizes really are TWO COMPACT CLUSTERS.

    d53 -- 72 bytes on 117 of its 198 records and 8.96 MB on the other 81 -- is
    the case this exists for: picking the record nearest an EXPECTED size is
    meaningless there, because no record has a size anywhere near the
    expectation. On this corpus the predicate returns true for seven domains
    (17, 33, 39, 40, 47, 53, 60); six of them receive substitutions, because
    d47's post-own-gap records are all already patches and are skipped before
    classification is ever reached.

    An earlier version tested only for SOME adjacent ratio >= 10 anywhere in the
    sorted sizes. That is not a two-cluster test and it did not behave like one:
    it substituted in 25 domains where this one substitutes in 6. d26 qualified
    on a single 40-byte record standing in front of 199 records spanning 808 to
    412,264 bytes, and d11's supposed upper cluster spanned 2,704 to 216,756
    bytes. Splitting a continuous distribution at its widest ratio and then
    pricing every record at one of the two ends is exactly the quantisation the
    two-point path exists to avoid, and it moved the headline result by 6.29
    points -- +0.80% against +7.09%.

    So all three conditions are required, not just the gap:

    * the split at the widest adjacent ratio separates the two sides by at least
      MIN_CLUSTER_SEPARATION,
    * each side spans no more than MAX_CLUSTER_SPREAD from its own min to its
      own max -- a cluster, not a range,
    * each side holds at least MIN_CLUSTER_SHARE of the records, so one stray
      record cannot declare a mode.
    """
    ordered = sorted(sizes)
    if len(ordered) < 8 or ordered[0] <= 0:
        return False
    split = max(range(len(ordered) - 1), key=lambda i: ordered[i + 1] / ordered[i])
    low, high = ordered[:split + 1], ordered[split + 1:]
    if not low or not high:
        return False
    if high[0] / low[-1] < MIN_CLUSTER_SEPARATION:
        return False
    if low[-1] / low[0] > MAX_CLUSTER_SPREAD or high[-1] / high[0] > MAX_CLUSTER_SPREAD:
        return False
    share = MIN_CLUSTER_SHARE * len(ordered)
    return len(low) >= share and len(high) >= share


def plan_same_size_control(steady, postgap):
    """The instrument's own artifact, measured.

    Substitute each post-own-gap record with a DIFFERENT record of the same
    domain whose length is as close as possible to its OWN length. Content
    changes by ~nothing, so any movement in the compressed size is entirely the
    cost of splicing a payload from a different hour into this one -- the bias
    that the real runs cannot avoid. Without this control a small measured
    saving cannot be told apart from the instrument.
    """
    substitutions = {}
    stats = defaultdict(lambda: {
        "records": 0, "bytes_before": 0, "bytes_after": 0, "mode": "same_size_control",
    })
    for domain, entries in postgap.items():
        pool = sorted(steady.get(domain, []))
        if not pool:
            continue
        for index, (record_offset, plen, _gap_run, is_delta) in enumerate(entries):
            if is_delta:
                continue
            nearest = sorted(pool, key=lambda entry: abs(entry[0] - plen))[:4]
            donor = nearest[index % len(nearest)]
            substitutions[record_offset] = (donor[1], donor[0])
            cell = stats[domain]
            cell["records"] += 1
            cell["bytes_before"] += plen
            cell["bytes_after"] += donor[0]
    return substitutions, stats


def plan(steady, postgap):
    """record_offset -> (substitute_payload_offset, substitute_len).

    Two realisations, because one does not fit both shapes of domain.

    CONTINUOUS domains get the steady record whose length is nearest the
    modelled target. Donors are ROTATED among the nearest few rather than
    always taking the single closest, because repeatedly splicing the SAME
    donor bytes into many records manufactures byte-identical repetition that
    the producer would never emit and that zstd rewards, biasing the result
    optimistically.

    BIMODAL domains get a two-point realisation: `p = (1-q)^k` is the
    probability that nothing changed over those k hours, so the records with
    the highest p -- the shortest gaps -- are the ones given the domain's
    "nothing changed" record, and exactly `round(sum(p))` of them are, which
    preserves the expected count without pretending any record has the mean
    size. The rest keep the whole payload they already carry.
    """
    substitutions = {}
    stats = defaultdict(lambda: {
        "records": 0, "bytes_before": 0, "bytes_after": 0, "mode": None,
    })
    for domain, entries in postgap.items():
        pool = steady.get(domain)
        if not pool:
            continue
        whole = max([plen for plen, _o in pool] + [plen for _o, plen, _k, _d in entries])
        steady_mean = sum(plen for plen, _o in pool) / len(pool)
        q = steady_mean / whole if whole else 0.0
        pool_sorted = sorted(pool)
        candidates = [entry for entry in entries if not entry[3]]
        if not candidates:
            continue
        cell = stats[domain]

        if is_bimodal([plen for plen, _o in pool]):
            cell["mode"] = "two_point"
            small_pool = [entry for entry in pool_sorted if entry[0] <= pool_sorted[0][0] * 2]
            # p = probability nothing changed across the record's own gap run.
            scored = sorted(
                ((1.0 - q) ** (gap_run + 1), record_offset, plen)
                for record_offset, plen, gap_run, _d in candidates
            )
            scored.reverse()
            unchanged_count = int(round(sum(p for p, _o, _n in scored)))
            for index, (_p, record_offset, plen) in enumerate(scored[:unchanged_count]):
                donor = small_pool[index % len(small_pool)]
                if donor[0] >= plen:
                    continue
                substitutions[record_offset] = (donor[1], donor[0])
                cell["records"] += 1
                cell["bytes_before"] += plen
                cell["bytes_after"] += donor[0]
            continue

        cell["mode"] = "nearest_rotated"
        for index, (record_offset, plen, gap_run, _d) in enumerate(candidates):
            hours = gap_run + 1
            target = whole * (1.0 - (1.0 - q) ** hours)
            nearest = sorted(pool_sorted, key=lambda entry: abs(entry[0] - target))[:4]
            donor = nearest[index % len(nearest)]
            if donor[0] >= plen:
                continue
            substitutions[record_offset] = (donor[1], donor[0])
            cell["records"] += 1
            cell["bytes_before"] += plen
            cell["bytes_after"] += donor[0]
    return substitutions, stats


def rebuild_and_compress(path, substitutions):
    cctx = compressor()
    chunker = cctx.chunker(chunk_size=1 << 22)
    compressed = 0
    kept = 0

    def feed(blob):
        nonlocal compressed
        for out in chunker.compress(blob):
            compressed += len(out)

    buffer = bytearray()
    source = open(path, "rb")
    donor = open(path, "rb")
    try:
        offset = 0
        while True:
            header = source.read(H)
            if not header:
                break
            (_m, _v, _h, _s, _hour, _d, _t, plen, _f) = HDR.unpack_from(header, 0)
            if offset in substitutions:
                donor_offset, donor_len = substitutions[offset]
                donor.seek(donor_offset)
                payload = donor.read(donor_len)
                source.seek(plen, 1)
                new_header = bytearray(header)
                struct.pack_into("<I", new_header, 24, donor_len)
                buffer += new_header
                buffer += payload
                kept += H + donor_len
            else:
                payload = source.read(plen)
                buffer += header
                buffer += payload
                kept += H + plen
            offset += H + plen
            if len(buffer) >= (1 << 22):
                feed(bytes(buffer))
                buffer.clear()
        if buffer:
            feed(bytes(buffer))
        for out in chunker.finish():
            compressed += len(out)
    finally:
        source.close()
        donor.close()
    return kept, compressed


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("stream")
    ap.add_argument("--only-domains", default="", help="restrict the lever to these domain ids")
    ap.add_argument("--same-size-control", action="store_true",
                    help="substitute same-length donors, to measure the splice artifact alone")
    args = ap.parse_args()
    started = time.time()
    steady, postgap = survey(args.stream)
    only = {int(v) for v in args.only_domains.split(",") if v.strip()}
    if only:
        postgap = {d: v for d, v in postgap.items() if d in only}
    if args.same_size_control:
        substitutions, stats = plan_same_size_control(steady, postgap)
    else:
        substitutions, stats = plan(steady, postgap)
    kept, compressed = rebuild_and_compress(args.stream, substitutions)
    print(json.dumps({
        "stream": args.stream,
        "only_domains": sorted(only) or None,
        "records_replaced": len(substitutions),
        "content_bytes_after": kept,
        "content_bytes_saved": sum(c["bytes_before"] - c["bytes_after"] for c in stats.values()),
        "compressed_bytes": compressed,
        "elapsed_s": round(time.time() - started, 1),
        "per_domain": {str(d): c for d, c in sorted(stats.items())},
    }, indent=2))


if __name__ == "__main__":
    sys.exit(main())
