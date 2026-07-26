"""Compress both arms of a real-dynamics measurement at the container's zstd
level and report whole-stream and exact block-span ratios.

Generalizes `extractor/native/scripts/bench_real_dynamics.py` (which hardcodes
the d20 arm name) by taking the shipping arm's basename on the command line.

    python compress_arms.py <out_dir> <ship_arm_basename> [hours_per_span]

Inputs are what the measurement bin wrote: the shipping DATA records verbatim
and the wire-format bodies, each with hour-boundary offsets, so the block spans
slice on real hour boundaries rather than byte averages.
"""
from __future__ import annotations

import os
import sys

import zstandard

LEVEL = 3  # DEFAULT_ZSTD_COMPRESSION_LEVEL, portable_replay/native/snapshot/src/pack.rs


def load_offsets(path: str) -> list[int]:
    with open(path, encoding="ascii") as f:
        return [int(line) for line in f if line.strip()]


def compressed_whole(data: bytes) -> int:
    return len(zstandard.ZstdCompressor(level=LEVEL).compress(data))


def compressed_in_spans(data: bytes, offsets: list[int], hours_per_span: int) -> int:
    """Sum of independently compressed spans of `hours_per_span` hours each —
    the shape the container actually stores (one compressed block per span)."""
    total = 0
    c = zstandard.ZstdCompressor(level=LEVEL)
    for start_hour in range(0, len(offsets) - 1, hours_per_span):
        end_hour = min(start_hour + hours_per_span, len(offsets) - 1)
        total += len(c.compress(data[offsets[start_hour]:offsets[end_hour]]))
    return total


def main() -> None:
    out_dir = sys.argv[1]
    ship_arm = sys.argv[2]
    span_hours = int(sys.argv[3]) if len(sys.argv) > 3 else 48

    arms = {}
    for arm in (ship_arm, "wire_new"):
        data = open(os.path.join(out_dir, f"{arm}.bin"), "rb").read()
        offsets = load_offsets(os.path.join(out_dir, f"{arm}.offsets"))
        arms[arm] = (data, offsets)
        print(f"{arm}: raw {len(data):,} bytes over {len(offsets) - 1} hours")

    hour_counts = {len(o) for _, o in arms.values()}
    if len(hour_counts) != 1:
        raise SystemExit(f"arms cover different hour counts: {hour_counts}")

    for label, hours in (("whole stream", None), (f"{span_hours}-hour blocks", span_hours)):
        sizes = {}
        for arm, (data, offsets) in arms.items():
            if hours is None:
                sizes[arm] = compressed_whole(data)
            else:
                sizes[arm] = compressed_in_spans(data, offsets, hours)
        ratio = 100.0 * sizes["wire_new"] / sizes[ship_arm]
        print(
            f"zstd-{LEVEL} {label:15s} ship {sizes[ship_arm]:>12,} B   "
            f"wire {sizes['wire_new']:>12,} B   wire = {ratio:.1f}% of shipping"
        )


if __name__ == "__main__":
    main()
