#!/usr/bin/env python3
"""Lane CC: what a pre-compression stream lever is worth AFTER compression.

A lever that removes bytes from the H4CR stream is only worth what it removes
from the SHIPPED artifact, and the shipped artifact is the stream under
zstd-3/windowLog-27/long-distance-matching (the container profile,
`portable_replay/native/snapshot/src/pack.rs`). Redundant bytes are exactly
what that compressor already collapses, so a content saving says almost nothing
about a disk saving.

This tool builds an ABLATED stream -- the records named are deleted outright --
and compresses it with the container's own parameters.

**THE RESULT IS NOT A CEILING, and must never be quoted as one.** zstd's output
size is not monotone in its input: deleting a record removes match sources as
well as matched bytes. Measured here: deleting d53's 460 MB of post-own-gap
records makes the compressed stream 263,244 bytes LARGER, and so does deleting
d22's. A deletion run is one measured point on one constructed stream. It
bounds nothing about a stream in which those records are REPLACED rather than
removed -- that is `cc_stream_replace.py`, whose three realisations of the same
lever land at -4.09%, +0.80% and +7.09% against this instrument's 28.8%.

Read-only with respect to the corpus; the ablated stream is streamed straight
into the compressor and never written to disk.
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
HEADER_LEN = 32
RT_DOMAIN_DATA = 3
RT_DOMAIN_GAP = 5
STATUS_DELTA_DATA = 1 << 10
NON_SHARED_HEADER_DOMAINS = {48, 53, 83, 91, 93}

_HDR = struct.Struct("<4sHHQiHHII")

ZSTD_LEVEL = 3
ZSTD_WINDOW_LOG = 27


def _compressor():
    params = zstd.ZstdCompressionParameters.from_level(
        ZSTD_LEVEL, window_log=ZSTD_WINDOW_LOG, enable_ldm=True, write_checksum=1
    )
    return zstd.ZstdCompressor(compression_params=params)


def iter_records(path):
    with open(path, "rb") as handle:
        while True:
            header = handle.read(HEADER_LEN)
            if not header:
                return
            if len(header) < HEADER_LEN:
                raise SystemExit("truncated header at EOF")
            (magic, _version, header_len, _seq, _hour, domain_id, record_type,
             payload_len, _flags) = _HDR.unpack_from(header, 0)
            if magic != MAGIC or header_len != HEADER_LEN:
                raise SystemExit("framing error")
            payload = handle.read(payload_len)
            if len(payload) != payload_len:
                raise SystemExit("truncated payload at EOF")
            yield domain_id, record_type, header, payload


def make_predicate(mode, domains):
    """Return keep(domain_id, record_type, after_own_gap) -> bool."""
    if mode == "identity":
        return lambda d, t, g: True
    if mode == "drop-domain":
        return lambda d, t, g: not (t == RT_DOMAIN_DATA and d in domains)
    if mode == "drop-post-own-gap":
        if domains:
            return lambda d, t, g: not (t == RT_DOMAIN_DATA and g and d in domains)
        return lambda d, t, g: not (t == RT_DOMAIN_DATA and g)
    if mode == "drop-all-data":
        return lambda d, t, g: t != RT_DOMAIN_DATA
    raise SystemExit(f"unknown mode {mode}")


def run(path, mode, domains):
    keep = make_predicate(mode, domains)
    cctx = _compressor()
    compressor = cctx.chunker(chunk_size=1 << 22)
    gap_since_last_data = defaultdict(bool)
    seen_data = defaultdict(bool)
    kept_bytes = 0
    dropped_bytes = 0
    dropped_records = 0
    compressed = 0
    started = time.time()

    def feed(blob):
        nonlocal compressed
        for out in compressor.compress(blob):
            compressed += len(out)

    buffer = bytearray()
    for domain_id, record_type, header, payload in iter_records(path):
        after_own_gap = False
        if record_type == RT_DOMAIN_DATA:
            after_own_gap = seen_data[domain_id] and gap_since_last_data[domain_id]
        if keep(domain_id, record_type, after_own_gap):
            buffer += header
            buffer += payload
            kept_bytes += HEADER_LEN + len(payload)
            if len(buffer) >= (1 << 22):
                feed(bytes(buffer))
                buffer.clear()
        else:
            dropped_bytes += HEADER_LEN + len(payload)
            dropped_records += 1
        if record_type == RT_DOMAIN_GAP:
            gap_since_last_data[domain_id] = True
        elif record_type == RT_DOMAIN_DATA:
            seen_data[domain_id] = True
            gap_since_last_data[domain_id] = False
    if buffer:
        feed(bytes(buffer))
    for out in compressor.finish():
        compressed += len(out)

    return {
        "mode": mode,
        "domains": sorted(domains) if domains else None,
        "kept_bytes": kept_bytes,
        "dropped_bytes": dropped_bytes,
        "dropped_records": dropped_records,
        "compressed_bytes": compressed,
        "elapsed_s": round(time.time() - started, 1),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("stream")
    ap.add_argument("--mode", required=True,
                    choices=["identity", "drop-domain", "drop-post-own-gap", "drop-all-data"])
    ap.add_argument("--domains", default="", help="comma-separated domain ids")
    args = ap.parse_args()
    domains = {int(v) for v in args.domains.split(",") if v.strip()}
    print(json.dumps(run(args.stream, args.mode, domains)))


if __name__ == "__main__":
    sys.exit(main())
