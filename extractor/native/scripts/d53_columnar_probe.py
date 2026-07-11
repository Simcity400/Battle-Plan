#!/usr/bin/env python3
"""O6 columnar d53 probe: does a field-major (columnar) layout of d53 payload
rows beat the shipped row-major layout AFTER production compression?

The byte study (OVERNIGHT_QUEUE doc) showed d53 = 86.6% of compressed bytes at
production settings (zstd level 3, window_log 27, long-range matching). This
probe re-encodes ONLY d53 DOMAIN_DATA payloads' fixed-stride row sections into
byte-plane columnar form (every field column contiguous), leaves every other
byte of every record verbatim, and compares whole-stream compressed size
against an identical re-encode with NO transform — same compressor, same
framing, so the delta is attributable to the layout alone.

The transform is trivially invertible (pure transpose with counts from the
72-byte payload header), so a win here is a lossless win by construction.
Variant B (delta+varint on sorted id columns) only matters if Variant A moves
the needle; the probe reports A.

Read-only. Usage:
  python scripts/d53_columnar_probe.py <run_dir> [--level 3] [--window-log 27]
"""
import argparse
import os
import struct

import zstandard

HDR = 32
D53 = 53
RECORD_TYPE_DOMAIN_DATA = 3
D53_HEADER_LEN = 72
# (section row length) in payload order; counts live at payload[28:72].
D53_SECTION_ROW_LENS = [24, 32, 16, 16, 24, 48, 40, 32, 32, 32, 24]


def transpose_sections(payload: bytes):
    """Byte-plane transpose of each fixed-stride section; trailers verbatim.
    Returns None when the payload does not fit the schema (caller keeps the
    original bytes and counts the bail — the measurement never corrupts)."""
    if len(payload) < D53_HEADER_LEN:
        return None
    counts = struct.unpack_from("<11I", payload, 28)
    out = bytearray(payload[:D53_HEADER_LEN])
    off = D53_HEADER_LEN
    for count, row_len in zip(counts, D53_SECTION_ROW_LENS):
        size = count * row_len
        if off + size > len(payload):
            return None
        section = payload[off:off + size]
        if count > 1:
            # column-major: byte plane b of every row, for b in 0..row_len
            for b in range(row_len):
                out.extend(section[b::row_len])
        else:
            out.extend(section)
        off += size
    out.extend(payload[off:])  # trailers verbatim
    assert len(out) == len(payload)
    return bytes(out)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("run_dir")
    ap.add_argument("--level", type=int, default=3)
    ap.add_argument("--window-log", type=int, default=27)
    args = ap.parse_args()

    stream = os.path.join(args.run_dir, "collector_stream.bin")
    stored = os.path.getsize(stream)

    params = zstandard.ZstdCompressionParameters.from_level(
        args.level, window_log=args.window_log,
        enable_ldm=True,
    )

    class Sink:
        def __init__(self):
            self.compressed = 0
            self._chunker = zstandard.ZstdCompressor(
                compression_params=params).chunker()

        def feed(self, data):
            for chunk in self._chunker.compress(data):
                self.compressed += len(chunk)

        def finish(self):
            for chunk in self._chunker.finish():
                self.compressed += len(chunk)

    base = Sink()       # decoded stream verbatim
    columnar = Sink()   # d53 data payloads transposed

    dctx = zstandard.ZstdDecompressor()
    decoded_total = 0
    d53_data_records = 0
    d53_data_bytes = 0
    bailed = 0
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
                record_type = struct.unpack_from("<H", buf, off + 22)[0]
                rec = buf[off:off + HDR + payload_len]
                decoded_total += len(rec)
                base.feed(rec)
                if domain_id == D53 and record_type == RECORD_TYPE_DOMAIN_DATA:
                    d53_data_records += 1
                    d53_data_bytes += payload_len
                    transformed = transpose_sections(rec[HDR:])
                    if transformed is None:
                        bailed += 1
                        columnar.feed(rec)
                    else:
                        columnar.feed(rec[:HDR] + transformed)
                else:
                    columnar.feed(rec)
                off += HDR + payload_len
            buf = buf[off:]
    base.finish()
    columnar.finish()

    print(f"stored on disk:               {stored/1e6:9.1f} MB")
    print(f"decoded total:                {decoded_total/1e6:9.1f} MB")
    print(f"d53 DATA records:             {d53_data_records} "
          f"({d53_data_bytes/1e6:.1f} MB raw, bailed={bailed})")
    print(f"re-encode BASELINE  (lvl {args.level}, wlog {args.window_log}): "
          f"{base.compressed/1e6:9.2f} MB")
    print(f"re-encode COLUMNAR  (lvl {args.level}, wlog {args.window_log}): "
          f"{columnar.compressed/1e6:9.2f} MB")
    delta = base.compressed - columnar.compressed
    print(f"columnar delta:               {delta/1e6:+9.2f} MB "
          f"({delta*100/max(1, base.compressed):+.1f}%)")


if __name__ == "__main__":
    main()
