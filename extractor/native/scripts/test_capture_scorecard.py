#!/usr/bin/env python3
"""Tests for capture_scorecard.py's gap reconciliation.

Every fixture here is built to the shape the recorder actually seals, taken
from live capture t17: a derived `gaps` ledger that is a SUPERSET of the
producer's `domain_gap_outcomes` -- every H4DG projected into it, plus
whole-clock skipped hours, plus one record per delivered domain-hour that
carried a failure status bit.

The axis these replace demanded the two ledgers agree per reason. On t17 that
read `source_unavailable: 4703 vs 501` and invited the reading that 4202
domain-hours were lost; all 4202 have a DOMAIN_DATA record in the stream.
"""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("capture_scorecard.py")
SPEC = importlib.util.spec_from_file_location("capture_scorecard", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
SCORECARD = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = SCORECARD
SPEC.loader.exec_module(SCORECARD)


def outcome(domain_id, start, end, reason):
    return {
        "domain_id": domain_id,
        "reason": reason,
        "range_start_hour_raw": start,
        "range_end_hour_raw": end,
        "target_hour_raw": end,
        "outcome_ordinal": 0,
        "part": "both",
    }


def derived(domains, start, end, reason, kind="status_bits"):
    return {
        "domains": list(domains),
        "hour_start": start,
        "hour_end": end,
        "reason": reason,
        "witness": {"kind": kind},
    }


def manifest(outcomes, gaps, frames=4, domain_ids=(10, 11)):
    """A manifest carrying only what the gap axes read."""
    return {
        "compact_stream": {
            "frame_end_record_count": frames,
            "expected_domains": [f"d{i}" for i in domain_ids],
            "per_domain": {
                f"d{i}": {"domain_id": i, "data_record_count": 0,
                          "row_count_total": 1, "gap_record_count": None}
                for i in domain_ids
            },
            "domain_gap_outcomes": outcomes,
            "gaps": gaps,
            "frame_degraded_outcomes": [],
        },
        "hourly_requirement": {},
        "replay_artifact": {},
    }


def axis(manifest_dict, name):
    for label, count, detail in SCORECARD.score(manifest_dict):
        if label == name:
            return count, detail
    raise AssertionError(f"no axis named {name!r}")


UNPROJECTED = "producer gap domain-hours the derived ledger never claimed"
FLAGGED = "domain-hours delivered with a producer failure status"
GAPPED = "domain-hours with a gap instead of data"


class GapReconciliation(unittest.TestCase):
    def test_a_superset_ledger_reports_no_missing_projection(self):
        """The t17 shape must reconcile clean.

        d10 left a producer gap for hour 1; d10 and d11 delivered hours 2 and 3
        with a failure status. The derived ledger carries all four records and
        the reasons do not line up per-reason -- which is exactly the case the
        old equality axis called a disagreement.
        """
        book = manifest(
            outcomes=[outcome(10, 1, 1, "source_unavailable")],
            gaps=[
                derived([10], 1, 1, "source_unavailable"),   # the projection
                derived([10], 2, 2, "source_unavailable"),   # delivered, flagged
                derived([11], 2, 2, "partial_data"),
                derived([11], 3, 3, "source_unavailable"),
            ],
        )
        self.assertEqual(axis(book, UNPROJECTED)[0], 0)
        self.assertEqual(axis(book, FLAGGED)[0], 3)
        self.assertEqual(axis(book, GAPPED)[0], 1)

    def test_a_renamed_reason_is_not_counted_as_a_flagged_delivery(self):
        """The recorder renames some reasons on the way into the ledger.

        An H4DG `torn` projects to a ledger `torn_sweep` and an `overrun` to a
        `backpressure`. Joining on the reason string counts those projections as
        host-derived annotations; joining on (hour, domain) does not.
        """
        book = manifest(
            outcomes=[outcome(10, 1, 1, "torn"), outcome(11, 1, 2, "overrun")],
            gaps=[
                derived([10], 1, 1, "torn_sweep", kind="torn_sweep"),
                derived([11], 1, 2, "backpressure", kind="counter"),
            ],
        )
        self.assertEqual(axis(book, UNPROJECTED)[0], 0)
        self.assertEqual(
            axis(book, FLAGGED)[0], 0,
            "a projection under a renamed reason is a gap, not a delivered hour")
        self.assertEqual(
            axis(book, GAPPED)[0], 3,
            "an inclusive coalesced range is 2 domain-hours, not 1 record")

    def test_a_producer_gap_missing_from_the_ledger_is_a_miss(self):
        """The check the old axis was reaching for, stated soundly.

        A producer said it could not fill this domain-hour and the ledger the
        replay is graded against never heard about it. That is real loss and it
        must not score clean.
        """
        book = manifest(
            outcomes=[outcome(10, 1, 3, "backpressure")],
            gaps=[derived([10], 1, 1, "backpressure", kind="counter")],
        )
        self.assertEqual(
            axis(book, UNPROJECTED)[0], 2,
            "hours 2 and 3 of the coalesced range reached no derived record")

    def test_a_coverage_shortfall_is_an_absence_not_a_delivery(self):
        """`derive_gap_records` raises TWO kinds of host-derived record.

        A COVERAGE witness says the domain-hour produced no PERF or too few
        DATA -- the host inferred the absence and no producer named it. A STATUS
        witness says records arrived carrying a failure flag. Counting the first
        as a delivery understates loss exactly as badly as the equality axis this
        replaced overstated it.
        """
        book = manifest(
            outcomes=[],
            gaps=[
                {
                    "domains": [10], "hour_start": 1, "hour_end": 1,
                    "reason": "coverage_shortfall",
                    "witness": {
                        "kind": "coverage",
                        "expected_perf_records": 1, "observed_perf_records": 0,
                        "expected_data_records": 1, "observed_data_records": 0,
                    },
                },
                derived([11], 1, 1, "partial_data"),
            ],
        )
        self.assertEqual(
            axis(book, "domain-hours absent with no producer statement")[0], 1,
            "a domain-hour that observed zero PERF and zero DATA is an absence")
        self.assertEqual(
            axis(book, FLAGGED)[0], 1,
            "only the status record is a delivery")

    def test_a_coverage_and_status_record_is_also_an_absence(self):
        """The composite arm is still a shortfall.

        `derive_gap_records` emits `CoverageAndStatus` only for a domain-hour it
        already put in the shortfall set, so its status bits do not make it a
        delivery.
        """
        book = manifest(
            outcomes=[],
            gaps=[{
                "domains": [10], "hour_start": 1, "hour_end": 1,
                "reason": "source_unavailable",
                "witness": {
                    "kind": "coverage_and_status",
                    "expected_perf_records": 1, "observed_perf_records": 0,
                    "expected_data_records": 1, "observed_data_records": 0,
                    "status_bits": 256,
                },
            }],
        )
        self.assertEqual(
            axis(book, "domain-hours absent with no producer statement")[0], 1)
        self.assertEqual(axis(book, FLAGGED)[0], 0)

    def test_a_global_skipped_hour_is_not_a_per_domain_annotation(self):
        """A whole-clock skipped hour names no domain and claims one hour.

        Counting it once per expected domain would inflate the flagged-delivery
        count by the size of the roster for every skipped hour.
        """
        book = manifest(
            outcomes=[],
            gaps=[derived([], 5, 5, "skipped_hour", kind="hour_discontinuity")],
        )
        self.assertEqual(axis(book, FLAGGED)[0], 0)
        self.assertEqual(axis(book, UNPROJECTED)[0], 0)


class ExcludedRecordAccounting(unittest.TestCase):
    def test_an_excluded_record_that_looks_clean_is_named_as_such(self):
        """An excluded payload is an envelope with no content.

        If it also carries a failure status it is at least visible as suspect
        elsewhere; if it does not, nothing but this axis distinguishes it from a
        clean delivery, and that is the case worth naming out loud.
        """
        book = manifest(
            outcomes=[outcome(10, 1, 1, "backpressure")],
            gaps=[
                derived([10], 1, 1, "backpressure", kind="counter"),
                derived([11], 2, 2, "partial_data"),
            ],
        )
        book["compact_stream"]["excluded_domain_data"] = {
            "total_records": 3,
            "per_domain": {"11": 2, "10": 1},
            "records": [
                {"game_hour_raw": 2, "domain_id": 11},   # flagged
                {"game_hour_raw": 3, "domain_id": 11},   # looks clean
                {"game_hour_raw": 1, "domain_id": 10},   # inside a producer gap
            ],
        }
        count, detail = axis(book, "records excluded from the seal")
        self.assertEqual(count, 3)
        self.assertIn("1 also carry a producer failure status", detail)
        self.assertIn("1 sit inside a producer gap", detail)
        self.assertIn("1 are otherwise indistinguishable from a clean delivery", detail)


class DegradedFrameAccounting(unittest.TestCase):
    def test_always_degrading_domains_are_separated_from_the_rest(self):
        """505 degraded frames is one number with two possible meanings.

        A domain offending in every degraded frame is a standing condition; the
        count that matters for anyone deciding what to fix is how many frames
        would still be degraded without it.
        """
        book = manifest(outcomes=[], gaps=[])
        book["compact_stream"]["frame_degraded_record_count"] = 3
        book["compact_stream"]["frame_degraded_outcomes"] = [
            {"hour_raw": 1, "domains": [{"domain_id": 85}, {"domain_id": 43}]},
            {"hour_raw": 2, "domains": [{"domain_id": 85}]},
            {"hour_raw": 3, "domains": [{"domain_id": 85}, {"domain_id": 43}]},
        ]
        count, detail = axis(book, "degraded frames")
        self.assertEqual(count, 3)
        self.assertIn("always-degrading: d85", detail)
        self.assertIn("2 of 3 frames would still be degraded without them", detail)


class HourFloorVacuity(unittest.TestCase):
    VACUOUS = "game-hour floor requested was zero, so it asserts nothing"

    def test_a_zero_floor_is_reported_as_vacuous_not_as_a_pass(self):
        """The old label said the requirement "passed" without reading `met`.

        On t17 it printed that while `met` was false and the run had been
        demoted to a recovery artifact.
        """
        book = manifest(outcomes=[], gaps=[])
        book["hourly_requirement"] = {
            "required": True, "requested_game_hours": 0,
            "met": False, "problems": ["gap_accounting_invalid"],
        }
        count, detail = axis(book, self.VACUOUS)
        self.assertEqual(count, 1)
        self.assertNotIn("passed", detail)
        self.assertIn("hourly_requirement.met is False", detail)

    def test_a_real_floor_raises_no_vacuity_axis(self):
        book = manifest(outcomes=[], gaps=[])
        book["hourly_requirement"] = {
            "required": True, "requested_game_hours": 24,
            "met": True, "problems": [],
        }
        with self.assertRaises(AssertionError):
            axis(book, self.VACUOUS)


if __name__ == "__main__":
    unittest.main()
