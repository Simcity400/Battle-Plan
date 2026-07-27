"""Tests for `compress_arms.py`'s two span shapes, on synthetic offsets files.

The point of the clock-aware mode is that it agrees with index mode exactly
where the two indexings coincide, and disagrees exactly where they drift — so
both halves are asserted here. No real capture is needed: the arms are synthetic
byte strings with hand-written offset and hour lists.
"""
from __future__ import annotations

import os
import sys

import pytest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import compress_arms  # noqa: E402


def write_arm(tmp_path, name, hour_sizes, hours=None):
    """One synthetic arm: `hour_sizes[i]` bytes for measured hour i."""
    data = bytearray()
    offsets = [0]
    for i, size in enumerate(hour_sizes):
        data.extend(bytes([i % 251]) * size)
        offsets.append(len(data))
    (tmp_path / f"{name}.bin").write_bytes(bytes(data))
    (tmp_path / f"{name}.offsets").write_text(
        "\n".join(str(o) for o in offsets) + "\n", encoding="ascii"
    )
    if hours is not None:
        (tmp_path / f"{name}.hours").write_text(
            "\n".join(str(h) for h in hours) + "\n", encoding="ascii"
        )
    return bytes(data), offsets


# --- clock_span_bounds, the whole of the new slicing rule --------------------


def test_contiguous_hours_slice_identically_in_both_modes():
    # 100 contiguous hours: the offset index and the clock block coincide, which
    # is the case every measurement published so far was taken on. The clock
    # bounds must therefore be exactly the index bounds.
    hours = list(range(1000, 1100))
    bounds = compress_arms.clock_span_bounds(hours, 48)
    assert bounds == [(0, 48), (48, 96), (96, 100)]


def test_an_excluded_hour_moves_the_clock_boundary_but_not_the_index_boundary():
    # 60 contiguous hours with hours 1010..1019 excluded (a failure-status run,
    # say). Measured hour 10 is game hour 1020, so the first clock block holds
    # only 10 measured hours, not 48 — while index mode would still cut at 48.
    hours = [h for h in range(1000, 1060) if not 1010 <= h < 1020]
    bounds = compress_arms.clock_span_bounds(hours, 48)
    # Block 0 is game hours 1000..1047 -> the 38 measured hours in that range.
    measured_in_block0 = sum(1 for h in hours if h - hours[0] < 48)
    assert bounds[0] == (0, measured_in_block0)
    assert measured_in_block0 == 38
    # And the index-mode cut would have been at 48, which is inside block 1.
    assert bounds != [(0, 48), (48, 50)]
    # Every measured hour is covered exactly once, in order, with no gaps.
    assert bounds[0][0] == 0
    assert bounds[-1][1] == len(hours)
    for (_, end), (start, _) in zip(bounds, bounds[1:]):
        assert end == start


def test_a_wholly_empty_clock_block_produces_no_span():
    # A capture that skips a whole 48-hour block emits no span for it rather
    # than an empty one — an empty span would compress to a nonzero frame header
    # and inflate the total for hours that carry nothing.
    hours = [1000, 1001, 1200, 1201]
    bounds = compress_arms.clock_span_bounds(hours, 48)
    assert bounds == [(0, 2), (2, 4)]


def test_a_single_measured_hour_is_one_span():
    assert compress_arms.clock_span_bounds([7], 48) == [(0, 1)]


def test_no_measured_hours_is_no_spans():
    assert compress_arms.clock_span_bounds([], 48) == []


def test_a_non_positive_span_is_refused():
    with pytest.raises(ValueError):
        compress_arms.clock_span_bounds([1, 2, 3], 0)


def test_hours_that_are_not_ordered_still_cover_every_hour_once():
    # The bins emit hours in order, so this is defensive rather than expected —
    # but a slicing rule that could drop or duplicate an hour would silently
    # change a size, which is the one thing this module must not do.
    hours = [1000, 1001, 1049, 1002, 1050]
    bounds = compress_arms.clock_span_bounds(hours, 48)
    assert bounds[0][0] == 0
    assert bounds[-1][1] == len(hours)
    for (_, end), (start, _) in zip(bounds, bounds[1:]):
        assert end == start


# --- the two mode paths through the compressor -------------------------------


def test_clock_and_index_totals_agree_on_a_contiguous_capture(tmp_path):
    sizes = [100 + 7 * i for i in range(96)]
    data, offsets = write_arm(tmp_path, "ship_dNN", sizes, hours=list(range(500, 596)))
    index_total = compress_arms.compressed_in_spans(data, offsets, 48)
    clock_total = compress_arms.compressed_in_clock_spans(
        data, offsets, list(range(500, 596)), 48
    )
    assert clock_total == index_total


def test_clock_and_index_totals_differ_once_hours_are_excluded(tmp_path):
    hours = [h for h in range(500, 560) if not 510 <= h < 520]
    sizes = [100 + 7 * i for i in range(len(hours))]
    data, offsets = write_arm(tmp_path, "ship_dNN", sizes, hours=hours)
    index_total = compress_arms.compressed_in_spans(data, offsets, 48)
    clock_total = compress_arms.compressed_in_clock_spans(data, offsets, hours, 48)
    assert clock_total != index_total


def test_clock_mode_refuses_an_hours_list_that_does_not_match_the_offsets(tmp_path):
    data, offsets = write_arm(tmp_path, "ship_dNN", [10, 20, 30])
    with pytest.raises(SystemExit):
        compress_arms.compressed_in_clock_spans(data, offsets, [1, 2], 48)


# --- the CLI, including the default-unchanged guarantee ----------------------


def run_main(tmp_path, args, capsys):
    compress_arms.main([str(tmp_path)] + args)
    return capsys.readouterr().out


def test_default_invocation_is_unchanged_and_needs_no_hours_file(tmp_path, capsys):
    # The default path must not read, require, or notice a `.hours` file: every
    # published measurement was taken without one.
    write_arm(tmp_path, "ship_dNN", [100, 200, 300, 400])
    write_arm(tmp_path, "wire_new", [50, 60, 70, 80])
    assert not (tmp_path / "ship_dNN.hours").exists()
    out = run_main(tmp_path, ["ship_dNN", "2"], capsys)
    assert "whole stream" in out
    assert "2-hour blocks" in out
    assert "clock" not in out


def test_clock_mode_says_so_and_needs_the_hours_file(tmp_path, capsys):
    write_arm(tmp_path, "ship_dNN", [100, 200, 300, 400])
    write_arm(tmp_path, "wire_new", [50, 60, 70, 80])
    with pytest.raises(SystemExit) as excinfo:
        run_main(tmp_path, ["ship_dNN", "2", "--clock"], capsys)
    assert "clock mode needs" in str(excinfo.value)

    write_arm(tmp_path, "ship_dNN", [100, 200, 300, 400], hours=[10, 11, 12, 13])
    write_arm(tmp_path, "wire_new", [50, 60, 70, 80], hours=[10, 11, 12, 13])
    out = run_main(tmp_path, ["ship_dNN", "2", "--clock"], capsys)
    assert "2-hour clock blocks" in out


def test_clock_mode_refuses_arms_that_disagree_on_the_clock(tmp_path, capsys):
    write_arm(tmp_path, "ship_dNN", [100, 200], hours=[10, 11])
    write_arm(tmp_path, "wire_new", [50, 60], hours=[10, 12])
    with pytest.raises(SystemExit) as excinfo:
        run_main(tmp_path, ["ship_dNN", "2", "--clock"], capsys)
    assert "disagree" in str(excinfo.value)


def test_arms_covering_different_hour_counts_are_still_refused(tmp_path, capsys):
    write_arm(tmp_path, "ship_dNN", [100, 200, 300])
    write_arm(tmp_path, "wire_new", [50, 60])
    with pytest.raises(SystemExit) as excinfo:
        run_main(tmp_path, ["ship_dNN", "2"], capsys)
    assert "different hour counts" in str(excinfo.value)
