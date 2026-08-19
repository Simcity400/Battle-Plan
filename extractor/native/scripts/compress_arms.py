"""Compress both arms of a real-dynamics measurement at the container's zstd
level and report whole-stream and exact block-span ratios.

Generalizes `extractor/native/scripts/bench_real_dynamics.py` (which hardcodes
the d20 arm name) by taking the shipping arm's basename on the command line.

    python compress_arms.py <out_dir> <ship_arm_basename> [hours_per_span] [--clock]

Inputs are what the measurement bin wrote: the shipping DATA records verbatim
and the wire-format bodies, each with hour-boundary offsets, so the block spans
slice on real hour boundaries rather than byte averages.

Two span shapes — and which one is correct depends on the BIN, not on taste
------------------------------------------------------------------------------

A span shape is only meaningful if its edges are the edges the measurement bin
actually placed its BASELINEs on. **The bins do not agree on that**, so this
script cannot have one right answer:

- **Index-based bins — every recipe-form bin except d38.** They open a block on
  the *measured-hour count*: `block_start = hours_seen % BLOCK_HOURS == 0`
  (`bench_real_d63.rs:638`, `bench_real_d10.rs:604`, `bench_real_d22.rs:722`,
  and so on). Their baselines land on offset-list indices, so **index mode — the
  default — is the shape that matches them**, on any capture, excluded hours or
  not. Clock mode would cut them at boundaries where no baseline was emitted.
- **Clock-based bins — d38, so far alone.** It opens a block on the *game clock*:
  `block_index = (game_hour - first_measured_hour).div_euclid(BLOCK_HOURS)`, a
  new block wherever that index changes (`bench_real_d38.rs:799`-`:801`). Its
  baselines land on clock boundaries, so **clock mode is the shape that matches
  it** — but only visibly so on a capture with excluded hours. On a contiguous
  capture the two indexings coincide exactly and both modes give the same
  number, which is why every measurement published so far is unaffected either
  way (`bench_real_d38.rs:112`-`:120` records the latent gap this closes).

So: **pass `--clock` when and only when the arms came from a clock-based bin.**
Neither mode is more correct in general; a mismatched mode compresses spans whose
edges are not the edges the baselines were written for, and a span can then
contain two baselines while the next contains none.

Clock mode needs each arm's per-hour game clock, which index mode never needed:
a `<arm>.hours` file beside `<arm>.offsets`, one integer per measured hour (so
`len(hours) == len(offsets) - 1`), in the same order. Both arms index the same
hours and must agree. A bin that does not write one cannot be sliced by clock,
and the error says so rather than silently falling back — a silent fallback is
exactly the drift this mode exists to make visible.
"""
from __future__ import annotations

import os
import sys

import zstandard

LEVEL = 3


def load_offsets(path: str) -> list[int]:
    with open(path, encoding="ascii") as f:
        return [int(line) for line in f if line.strip()]


def load_hours(path: str) -> list[int]:
    """The per-hour game clock, one integer per measured hour."""
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


def clock_span_bounds(hours: list[int], hours_per_span: int) -> list[tuple[int, int]]:
    """Group measured-hour INDICES into game-clock blocks.

    Returns `[(start_index, end_index), ...]` with `end_index` exclusive, in
    order, covering every measured hour exactly once. The block index is the one
    a clock-based bin uses (`bench_real_d38.rs:799`-`:801`):
    `(game_hour - hours[0]).div_euclid(hours_per_span)`, and a new span starts
    wherever it changes — so an excluded hour widens the clock gap without
    splitting a block, and a whole empty block simply produces no span.
    """
    if hours_per_span <= 0:
        raise ValueError(f"hours_per_span must be positive, got {hours_per_span}")
    bounds: list[tuple[int, int]] = []
    first = hours[0] if hours else 0
    last_block: int | None = None
    start = 0
    for i, hour in enumerate(hours):
        block = (hour - first) // hours_per_span
        if last_block is not None and block != last_block:
            bounds.append((start, i))
            start = i
        last_block = block
    if hours:
        bounds.append((start, len(hours)))
    return bounds


def compressed_in_clock_spans(
    data: bytes, offsets: list[int], hours: list[int], hours_per_span: int
) -> int:
    """`compressed_in_spans`, but with the spans cut on game-clock block edges."""
    if len(hours) != len(offsets) - 1:
        raise SystemExit(
            f"clock mode: {len(hours)} hour(s) listed but the offset list covers "
            f"{len(offsets) - 1}"
        )
    total = 0
    c = zstandard.ZstdCompressor(level=LEVEL)
    for start, end in clock_span_bounds(hours, hours_per_span):
        total += len(c.compress(data[offsets[start]:offsets[end]]))
    return total


def main(argv: list[str] | None = None) -> None:
    argv = list(sys.argv[1:] if argv is None else argv)
    clock = "--clock" in argv
    argv = [a for a in argv if a != "--clock"]

    out_dir = argv[0]
    ship_arm = argv[1]
    span_hours = int(argv[2]) if len(argv) > 2 else 48

    arms = {}
    for arm in (ship_arm, "wire_new"):
        data = open(os.path.join(out_dir, f"{arm}.bin"), "rb").read()
        offsets = load_offsets(os.path.join(out_dir, f"{arm}.offsets"))
        hours = None
        if clock:
            hours_path = os.path.join(out_dir, f"{arm}.hours")
            if not os.path.exists(hours_path):
                raise SystemExit(
                    f"clock mode needs {hours_path}: one game-clock hour per measured "
                    "hour, in order. Without it the spans can only be cut on offset "
                    "index, which is what this mode exists to stop doing silently."
                )
            hours = load_hours(hours_path)
        arms[arm] = (data, offsets, hours)
        print(f"{arm}: raw {len(data):,} bytes over {len(offsets) - 1} hours")

    hour_counts = {len(o) for _, o, _ in arms.values()}
    if len(hour_counts) != 1:
        raise SystemExit(f"arms cover different hour counts: {hour_counts}")
    if clock:
        clocks = {tuple(h) for _, _, h in arms.values()}
        if len(clocks) != 1:
            raise SystemExit("arms disagree on their per-hour game clock")

    span_label = f"{span_hours}-hour {'clock ' if clock else ''}blocks"
    for label, hours_per_span in (("whole stream", None), (span_label, span_hours)):
        sizes = {}
        for arm, (data, offsets, hours) in arms.items():
            if hours_per_span is None:
                sizes[arm] = compressed_whole(data)
            elif clock:
                sizes[arm] = compressed_in_clock_spans(
                    data, offsets, hours, hours_per_span
                )
            else:
                sizes[arm] = compressed_in_spans(data, offsets, hours_per_span)
        ratio = 100.0 * sizes["wire_new"] / sizes[ship_arm]
        print(
            f"zstd-{LEVEL} {label:21s} ship {sizes[ship_arm]:>12,} B   "
            f"wire {sizes['wire_new']:>12,} B   wire = {ratio:.1f}% of shipping"
        )


if __name__ == "__main__":
    main()
