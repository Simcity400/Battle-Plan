"""Does compressing per hour cost meaningfully more than compressing in blocks?

This measures the penalty from resetting the compressor every hour rather than
asserting it.

The model is a domain that re-emits full state hourly while almost nothing
changes -- the shape ~75 of ~80 domains have today. Run:

    python extractor/native/scripts/bench_chunked_compression.py

Real captures will differ. This bounds the design question: is cross-hour
redundancy worth keeping, or not?
"""
from __future__ import annotations

import io
import json
import os
import sys

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")

try:
    import zstandard
except ImportError:
    raise SystemExit("needs `zstandard`: python -m pip install zstandard")

HOURS = 3000
LEVEL = 3
ROWS = 200
CHURN_PER_HOUR = 2


def synthetic_hours():
    """Full-state frames where a couple of rows move per hour."""
    rows = [
        {
            "id": i,
            "tag": "TAG%03d" % (i % 90),
            "province": 1000 + i * 7,
            "manpower": 10_000 + i * 13,
            "equipment": 500 + i * 3,
            "state": "active",
        }
        for i in range(ROWS)
    ]
    out = []
    for h in range(HOURS):
        for k in range(CHURN_PER_HOUR):
            r = rows[(h * CHURN_PER_HOUR + k) % ROWS]
            r["manpower"] += 17
        out.append(json.dumps({"hour": h, "rows": rows},
                              separators=(",", ":"), sort_keys=True).encode())
    return out


def per_hour(frames):
    c = zstandard.ZstdCompressor(level=LEVEL)
    return sum(len(c.compress(f)) for f in frames)


def blocked(frames, span):
    c = zstandard.ZstdCompressor(level=LEVEL)
    total = 0
    for i in range(0, len(frames), span):
        total += len(c.compress(b"\n".join(frames[i:i + span])))
    return total


def main():
    frames = synthetic_hours()
    raw = sum(len(f) for f in frames)
    mb = lambda n: n / 1_048_576

    print("model    : %d hours, %d rows, %d rows changing per hour, zstd level %d"
          % (HOURS, ROWS, CHURN_PER_HOUR, LEVEL))
    print("raw      : %8.2f MB" % mb(raw))
    print()

    one = per_hour(frames)
    print("%-28s %8.2f MB   (independent frame per hour)" % ("per-hour reset:", mb(one)))
    for span in (6, 12, 24, 48):
        b = blocked(frames, span)
        print("%-28s %8.2f MB   %5.1f%% of per-hour   (seek: one block = %d h)"
              % ("%d-hour blocks:" % span, mb(b), 100.0 * b / one, span))

    best = blocked(frames, 24)
    print()
    print("24-hour blocks cut compressed bytes by %.1f%% versus resetting every hour."
          % (100.0 * (one - best) / one))
    print("Seek cost in exchange: decode one block instead of one frame.")


if __name__ == "__main__":
    main()
