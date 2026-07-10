#!/usr/bin/env python3
"""Post-compression byte attribution for a compact H4CR stream.

The raw-byte attribution ("d53 = 92.3% of stream bytes") does not answer where
the COMPRESSED bytes go — zstd compresses different domains very differently,
and byte-lever priority must follow compressed share, not raw share. This tool
stream-decodes a production collector_stream.bin, splits records by domain_id,
runs one incremental zstd compressor per domain (so per-domain redundancy is
credited to that domain, mimicking what a domain-grouped layout would achieve),
and reports raw vs compressed share per domain. It also recompresses the whole
decoded stream at a higher level to bracket the cheap container-side lever.

Read-only; no repo state is modified. Usage:
  python scripts/byte_attribution.py <run_dir> [--level 3] [--whole-levels 3,13,19]
"""
import argparse
import json
import os
import struct
import sys

import zstandard

HDR = 32


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("run_dir")
    ap.add_argument("--level", type=int, default=3,
                    help="per-domain compressor level (match stream level)")
    ap.add_argument("--whole-levels", default="3,13,19",
                    help="whole-stream recompression levels to bracket the container lever")
    args = ap.parse_args()

    stream = os.path.join(args.run_dir, "collector_stream.bin")
    stored = os.path.getsize(stream)

    class DomainSink:
        def __init__(self, level):
            self.raw = 0
            self.compressed = 0
            cctx = zstandard.ZstdCompressor(level=level)
            self._chunker = cctx.chunker()

        def feed(self, data):
            self.raw += len(data)
            for chunk in self._chunker.compress(data):
                self.compressed += len(chunk)

        def finish(self):
            for chunk in self._chunker.finish():
                self.compressed += len(chunk)

    sinks = {}
    whole_levels = [int(x) for x in args.whole_levels.split(",") if x]
    whole = {}
    for lv in whole_levels:
        whole[lv] = DomainSink(lv)

    dctx = zstandard.ZstdDecompressor()
    decoded_total = 0
    records = 0
    with open(stream, "rb") as f:
        reader = dctx.stream_reader(f)
        buf = b""
        while True:
            chunk = reader.read(8 << 20)
            if not chunk:
                break
            buf += chunk
            off = 0
            while len(buf) - off >= HDR:
                payload_len = struct.unpack_from("<I", buf, off + 24)[0]
                if len(buf) - off < HDR + payload_len:
                    break
                domain_id = struct.unpack_from("<H", buf, off + 20)[0]
                rec = buf[off:off + HDR + payload_len]
                sink = sinks.get(domain_id)
                if sink is None:
                    sink = sinks[domain_id] = DomainSink(args.level)
                sink.feed(rec)
                for w in whole.values():
                    w.feed(rec)
                decoded_total += len(rec)
                records += 1
                off += HDR + payload_len
            buf = buf[off:]

    for s in sinks.values():
        s.finish()
    for w in whole.values():
        w.finish()

    comp_total = sum(s.compressed for s in sinks.values())
    rows = sorted(sinks.items(), key=lambda kv: -kv[1].compressed)
    print(f"stream stored={stored/1e6:.1f}MB decoded={decoded_total/1e6:.1f}MB "
          f"records={records} per-domain-summed compressed={comp_total/1e6:.1f}MB")
    print(f"{'domain':>7} {'raw MB':>9} {'raw %':>6} {'comp MB':>9} {'comp %':>7} {'ratio':>7}")
    for dom, s in rows[:15]:
        print(f"{dom:>7} {s.raw/1e6:>9.1f} {s.raw*100/decoded_total:>6.1f} "
              f"{s.compressed/1e6:>9.2f} {s.compressed*100/comp_total:>7.1f} "
              f"{s.raw/max(1,s.compressed):>7.1f}x")
    rest_raw = sum(s.raw for d, s in rows[15:])
    rest_c = sum(s.compressed for d, s in rows[15:])
    if rest_c:
        print(f"{'rest':>7} {rest_raw/1e6:>9.1f} {rest_raw*100/decoded_total:>6.1f} "
              f"{rest_c/1e6:>9.2f} {rest_c*100/comp_total:>7.1f}")
    print("\nwhole-stream recompression bracket (interleaved order, as stored):")
    for lv, w in sorted(whole.items()):
        print(f"  level {lv:>2}: {w.compressed/1e6:.1f}MB "
              f"({decoded_total/max(1,w.compressed):.0f}x)")
    print(f"  domain-grouped @ level {args.level} (sum of per-domain streams): "
          f"{comp_total/1e6:.1f}MB — vs interleaved level {args.level}: "
          f"{whole[args.level].compressed/1e6:.1f}MB" if args.level in whole else "")


if __name__ == "__main__":
    main()
