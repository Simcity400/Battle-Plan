"""Does column-major beat row-major for d20, after compression?

`WIRE_FORMAT.md` §13 leaves the record layout open and says the format commits to
row-major only if it wins on the reference domain. ~80 encoders get built against
whichever survives, and after they exist the layout cannot change without
rewriting all of them, so this is measured rather than argued.

Run `cargo run -p wire_format --bin bench_layout` first to emit the two encodings,
then:

    python extractor/native/scripts/bench_layout.py

Raw bytes are the wrong measure on their own — the container compresses in
multi-hour blocks, so what matters is the compressed size at a realistic block
span, not one compressor pass over a whole month. Several spans are reported
because the answer can move with the span: a longer window gives the compressor
more cross-hour redundancy to find, which is exactly what row-major depends on.
"""
from __future__ import annotations

import io
import os
import sys

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")

try:
    import zstandard
except ImportError:
    raise SystemExit("needs `zstandard`: python -m pip install zstandard")

LEVEL = 3
HERE = os.path.dirname(os.path.abspath(__file__))
RUNS = os.path.join(HERE, "..", "runs", "d20_reference")


def load(name: str) -> bytes:
    path = os.path.join(RUNS, name)
    if not os.path.exists(path):
        raise SystemExit(
            f"missing {path}\nrun: cargo run -p wire_format --bin bench_layout"
        )
    with open(path, "rb") as handle:
        return handle.read()


HOURS = 720


def compressed(data: bytes) -> int:
    return len(zstandard.ZstdCompressor(level=LEVEL).compress(data))


def load_offsets(name: str) -> list[int]:
    """Exact byte offset of each hour, written by the encoder.

    Slicing by average hour size flipped a published tenth in three consecutive
    reviews. The encoder knows where every hour starts, so it records the
    boundaries and this reads them — the block-span figures are now exact rather
    than approximate.
    """
    path = os.path.join(RUNS, name)
    with open(path, "r", encoding="utf-8") as handle:
        return [int(line) for line in handle if line.strip()]


def compressed_in_blocks(data: bytes, offsets: list[int], span: int) -> int:
    """Compress in independent windows of `span` hours, as the container does."""
    c = zstandard.ZstdCompressor(level=LEVEL)
    hours = len(offsets) - 1
    total = 0
    for start in range(0, hours, span):
        lo = offsets[start]
        hi = offsets[min(start + span, hours)]
        total += len(c.compress(data[lo:hi]))
    return total


def control() -> None:
    """The full-churn control, if it has been generated.

    Run `cargo run -p wire_format --bin bench_layout -- --white-noise` first.

    Every carried value is redrawn independently each hour. That destroys the
    cross-hour redundancy row-major depends on — but it destroys columnar's too,
    so the ratio moves TOWARD parity, not away from it. The ratio is not monotone
    in churn: a mix of noisy and frozen columns is columnar's best case, and this
    end of the range is close to its worst.
    """
    a = os.path.join(RUNS, "row_major_white_noise.bin")
    b = os.path.join(RUNS, "column_major_white_noise.bin")
    if not (os.path.exists(a) and os.path.exists(b)):
        print("control  : not generated; run the bench with --white-noise")
        return
    with open(a, "rb") as h:
        row = h.read()
    with open(b, "rb") as h:
        col = h.read()
    print("control  : with EVERY value redrawn hourly, column-major is %.1f%% of row-major"
          % (100.0 * compressed(col) / compressed(row)))
    print("           (%.2f MB vs %.2f MB) — full churn moves the ratio toward parity,"
          % (compressed(row) / 1_048_576, compressed(col) / 1_048_576))
    print("           so this is a bound on the far side of the committed model, not below it.")


def main() -> None:
    row = load("row_major.bin")
    col = load("column_major.bin")
    mb = lambda n: n / 1_048_576

    print("model    : 3,051 divisions, 720 hourly baselines, d20 schema v1, zstd level %d" % LEVEL)
    print()
    print("%-22s %10s %12s" % ("", "raw", "compressed"))

    row_c = compressed(row)
    col_c = compressed(col)
    print("%-22s %9.2f MB %9.2f MB" % ("row-major", mb(len(row)), mb(row_c)))
    print("%-22s %9.2f MB %9.2f MB" % ("column-major", mb(len(col)), mb(col_c)))
    print()

    raw_ratio = 100.0 * len(col) / len(row)
    comp_ratio = 100.0 * col_c / row_c
    print("whole-stream: column-major is %.1f%% of row-major raw, %.1f%% compressed"
          % (raw_ratio, comp_ratio))
    print()
    print("At realistic block spans (the container never compresses a whole month at once):")
    print("%-22s %12s %12s %8s" % ("span", "row-major", "column-major", "ratio"))
    row_offsets = load_offsets("row_major.offsets")
    col_offsets = load_offsets("column_major.offsets")
    for span in (6, 24, 48):
        r = compressed_in_blocks(row, row_offsets, span)
        k = compressed_in_blocks(col, col_offsets, span)
        print("%-22s %9.2f MB %9.2f MB %7.1f%%" % ("%d-hour blocks" % span, mb(r), mb(k),
                                                   100.0 * k / r))
    print()

    if comp_ratio < 95.0:
        verdict = "column-major wins; the format should not commit to row-major"
    elif comp_ratio > 105.0:
        verdict = "row-major wins; §13 can close in its favour"
    else:
        verdict = "within noise; decide on decode cost and seek, not size"
    print("verdict  : %s" % verdict)
    print()
    control()
    print()
    print("Read this for what it is: one synthetic month of baselines. It does not")
    print("model deltas, which is where most real hours land, nor a real change-rate")
    print("profile. It bounds the layout question; it does not close the budget one.")


if __name__ == "__main__":
    main()
