"""Compress both arms of the real-dynamics measurement at the container's
zstd level and report whole-stream and exact block-span ratios.

Inputs are what `bench_real_dynamics.rs` wrote: the shipping d20 records
verbatim and the wire-format bodies, each with hour-boundary offsets, so the
block spans slice on real hour boundaries rather than byte averages.
"""
from __future__ import annotations

import os
import sys

import zstandard

LEVEL = 3


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
    out_dir = sys.argv[1] if len(sys.argv) > 1 else r"extractor\native\runs\d20_real_dynamics"
    arms = {}
    for arm in ("ship_d20", "wire_new"):
        data = open(os.path.join(out_dir, f"{arm}.bin"), "rb").read()
        offsets = load_offsets(os.path.join(out_dir, f"{arm}.offsets"))
        arms[arm] = (data, offsets)
        print(f"{arm}: raw {len(data):,} bytes over {len(offsets) - 1} hours")

    for label, hours in (("whole stream", None), ("48-hour blocks", 48)):
        sizes = {}
        for arm, (data, offsets) in arms.items():
            if hours is None:
                sizes[arm] = compressed_whole(data)
            else:
                sizes[arm] = compressed_in_spans(data, offsets, hours)
        ratio = 100.0 * sizes["wire_new"] / sizes["ship_d20"]
        print(
            f"zstd-{LEVEL} {label:15s} ship {sizes['ship_d20']:>12,} B   "
            f"wire {sizes['wire_new']:>12,} B   wire = {ratio:.1f}% of shipping"
        )


if __name__ == "__main__":
    main()
