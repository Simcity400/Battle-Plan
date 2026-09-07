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
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gap_ledger


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
    witness = {"kind": kind}
    if kind == "hour_discontinuity":
        witness["previous_hour"] = start - 1
        witness["next_hour"] = end + 1
    elif kind == "torn_sweep":
        witness["tick_start"] = start
        witness["tick_end"] = end + 1
    elif kind == "status_bits":
        witness["status_bits"] = 256
    elif kind == "counter":
        witness["counter"] = "sink_backpressure_hours"
        witness["observed"] = end - start + 1
    elif kind in ("coverage", "coverage_and_status"):
        witness.update(expected_perf_records=1, observed_perf_records=0,
                       expected_data_records=1, observed_data_records=0)
        if kind == "coverage_and_status":
            witness["status_bits"] = 256
    return {
        "domains": list(domains),
        "hour_start": start,
        "hour_end": end,
        "reason": reason,
        "witness": witness,
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
            "gap_ledger": gap_ledger.pack_rows_for_tests(gaps, outcomes),
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
                derived([10], 1, 1, "source_unavailable"),
                derived([10], 2, 2, "source_unavailable"),
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
                {"game_hour_raw": 2, "domain_id": 11},
                {"game_hour_raw": 3, "domain_id": 11},
                {"game_hour_raw": 1, "domain_id": 10},
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


SOURCE_UNAVAILABLE = 1 << 8
PARTIAL_DATA = 1 << 9


def covered(domain_ids, per_domain, frames=4, gaps=()):
    """A manifest carrying only what the coverage-semantics axis reads."""
    return {
        "compact_stream": {
            "frame_end_record_count": frames,
            "expected_domains": [f"d{i}" for i in domain_ids],
            "per_domain": per_domain,
            "gap_ledger": gap_ledger.pack_rows_for_tests([], list(gaps)),
            "frame_degraded_outcomes": [],
        },
        "hourly_requirement": {},
        "replay_artifact": {},
    }


def domain(domain_id, *, data_records, rows, failure=0, gap_records=None):
    return {
        "domain_id": domain_id,
        "data_record_count": data_records,
        "gap_record_count": gap_records,
        "row_count_total": rows,
        "effective_failure_status_flags_or": failure,
    }


class CoverageSemantics(unittest.TestCase):
    """Covered-by-data must not read the same as covered-by-explanation.

    The recorder's coverage gate asks only whether the records arrived. d81
    delivers a SOURCE_UNAVAILABLE envelope for every one of its scheduled
    hours, so the gate reports `domain_coverage.ok` while the domain has never
    once carried rail state.
    """

    AXIS = "scheduled domains covered only by an explanation, never by data"

    def test_the_d81_shape_is_a_miss_and_names_its_status(self):
        book = covered(
            (81, 10),
            {
                "d81": domain(81, data_records=4, rows=0,
                              failure=SOURCE_UNAVAILABLE | PARTIAL_DATA),
                "d10": domain(10, data_records=4, rows=17),
            },
        )
        count, detail = axis(book, self.AXIS)
        self.assertEqual(count, 1)
        self.assertIn("d81", detail)
        self.assertIn("source_unavailable", detail)
        self.assertIn("partial_data", detail)
        self.assertIn("4 data records", detail)
        self.assertIn("0 rows", detail)
        self.assertNotIn("d10", detail)

    def test_a_domain_carrying_rows_is_covered_by_data(self):
        book = covered((10,), {"d10": domain(10, data_records=4, rows=1)})
        self.assertEqual(axis(book, self.AXIS)[0], 0)

    def test_an_empty_domain_with_no_failure_status_is_not_a_miss(self):
        """Nothing to report is not the same as nothing read.

        A capture in which no nuclear strike ever happened leaves d24 empty and
        clean. Counting that as a coverage miss would make the axis fire on
        every ordinary capture, and an axis that always fires is not read.
        """
        book = covered(
            (24, 10),
            {
                "d24": domain(24, data_records=4, rows=0),
                "d10": domain(10, data_records=4, rows=3),
            },
        )
        count, detail = axis(book, self.AXIS)
        self.assertEqual(count, 0)
        self.assertIn(
            "d24", detail,
            "a clean-empty domain must still be printed, not silently dropped")

    def test_a_domain_nothing_accounts_for_is_left_to_the_reconciliation_axis(self):
        """No data record and no gap is a worse condition with its own axis.

        Counting it here as well would report one absence twice and make the
        two axes disagree about the same domain.

        The fixture carries a FAILURE STATUS deliberately. Without one the
        domain sorts into the clean-empty bucket whether the guard exists or
        not, and the test passes for the wrong reason -- it did, until a
        mutation that deleted the guard survived it.
        """
        book = covered(
            (81,),
            {"d81": domain(81, data_records=0, rows=0,
                           failure=SOURCE_UNAVAILABLE)},
        )
        count, detail = axis(book, self.AXIS)
        self.assertEqual(
            count, 0,
            "an unaccounted domain-hour belongs to the reconciliation axis; "
            "counting it here reports one absence twice")
        self.assertNotIn("d81", detail)

    def test_hours_accounted_only_by_gap_records_still_count_as_explanation(self):
        """A domain that emitted no data at all, only gaps, is the same miss.

        The count printed is RECORDS, matching `per_domain.gap_record_count`:
        one H4DG covers the inclusive range 1..4, so four hours arrive as one
        record. The hours those ranges cover are counted by the gap axes above,
        and restating them here under a "records" label is how the two reports
        come to disagree about one ledger.
        """
        book = covered(
            (81,),
            {"d81": domain(81, data_records=0, rows=0,
                           failure=SOURCE_UNAVAILABLE)},
            gaps=[outcome(81, 1, 4, "source_unavailable")],
        )
        count, detail = axis(book, self.AXIS)
        self.assertEqual(count, 1)
        self.assertIn("1 gap records", detail)
        self.assertIn("0 data records", detail)

    def test_an_expected_domain_with_no_per_domain_record_is_not_double_counted(self):
        book = covered((81,), {})
        self.assertEqual(axis(book, self.AXIS)[0], 0)
        self.assertEqual(
            axis(book, "expected domains with no per-domain record")[0], 1)


class ResidualFailureStatus(unittest.TestCase):
    """The word lives in a different field depending on when the seal was cut."""

    def test_the_effective_word_is_preferred(self):
        self.assertEqual(
            SCORECARD.residual_failure_status({
                "effective_failure_status_flags_or": SOURCE_UNAVAILABLE,
                "status_flags_or": SOURCE_UNAVAILABLE | PARTIAL_DATA,
            }),
            SOURCE_UNAVAILABLE)

    def test_a_present_zero_does_not_fall_back(self):
        """A domain the recorder judged clean must stay clean.

        Falling back on falsiness rather than on absence would resurrect a
        lane-local badge bit as a failure and report an ordinary domain as a
        coverage miss.
        """
        self.assertEqual(
            SCORECARD.residual_failure_status({
                "effective_failure_status_flags_or": 0,
                "status_flags_or": SOURCE_UNAVAILABLE,
            }),
            0)

    def test_an_older_manifest_falls_back_to_status_flags_or(self):
        """The batch3/deepwar_b d81 shape: 750 data records, 0 rows, 768."""
        self.assertEqual(
            SCORECARD.residual_failure_status({"status_flags_or": 768}),
            SOURCE_UNAVAILABLE | PARTIAL_DATA)

    def test_the_fallback_strips_bits_outside_the_failure_vocabulary(self):
        """d49/d51/d53/d60/d99 badge bits are semantics, not failures."""
        self.assertEqual(
            SCORECARD.residual_failure_status({
                "status_flags_or": SOURCE_UNAVAILABLE | (1 << 16) | (1 << 17),
            }),
            SOURCE_UNAVAILABLE)


class FailureStatusNames(unittest.TestCase):
    def test_known_bits_are_spelled(self):
        self.assertEqual(
            SCORECARD.failure_status_names(SOURCE_UNAVAILABLE | PARTIAL_DATA),
            "source_unavailable|partial_data")

    def test_a_clean_word_spells_nothing(self):
        self.assertEqual(SCORECARD.failure_status_names(0), "")

    def test_an_unknown_bit_stays_visible(self):
        """A flag added to the crate must not vanish from the report."""
        spelled = SCORECARD.failure_status_names(SOURCE_UNAVAILABLE | (1 << 20))
        self.assertIn("source_unavailable", spelled)
        self.assertIn("unknown_bits=0x100000", spelled)


class DescheduledRecordTests(unittest.TestCase):
    """The deliberately-not-scheduled record must carry a PER-DOMAIN ruling.

    The rulings are not one event: d85 was ruled out on 2026-08-13 and the
    other five on 2026-08-03. A record that publishes six domains under one
    block-level date says something false about one of them, and the whole
    point of this record is that a reader never has to guess why a roster is
    short.

    kills: dropping the per-entry `decided_on`/`reason` loop from
    `capture_scorecard.score`, or reverting `descheduled_domains_json` in
    `recorder_host/src/main.rs` to a block-level-only reason and date.
    """

    AXIS = "faults in the deliberately-not-scheduled record"

    def block(self, domains):
        return {
            "compact_stream": {
                "descheduled_domains": {
                    "domains": domains,
                    "reason": "each domain below carries its own ruling",
                    "decided_on": "2026-08-03",
                    "decided_by": "owner",
                    "authority": "PROJECT.md",
                },
            },
        }

    def faults(self, manifest):
        for axis, count, detail in SCORECARD.score(manifest):
            if axis == self.AXIS:
                return count, detail
        self.fail(f"no {self.AXIS!r} axis")

    def test_an_entry_without_its_own_ruling_is_a_fault(self):
        _, detail = self.faults(self.block([
            {"domain": "selected_designer_identity_live"},
        ]))
        self.assertIn("selected_designer_identity_live publishes no decided_on", detail)
        self.assertIn("selected_designer_identity_live publishes no reason", detail)

    def test_a_per_domain_ruling_clears_that_fault(self):
        _, detail = self.faults(self.block([
            {
                "domain": "selected_designer_identity_live",
                "decided_on": "2026-08-13",
                "reason": "client UI selection state; cannot publish a clean word",
            },
        ]))
        self.assertNotIn("publishes no decided_on", detail)
        self.assertNotIn("publishes no reason", detail)


class DegradedReasonGrouping(unittest.TestCase):
    """A domain that produced no rows is named beside WHY, grouped by predicate.

    Reasons carry per-frame values, so counting distinct reason strings counts
    frames rather than causes: the same condition repeated across a capture
    presented as many different ones, and whichever frame happened to repeat
    became the "dominant reason". H4FD version 2 carries the predicate, which is
    one code per check, so it is the key that groups.

    It also carries the distinction the reason cannot: whether the producer
    diagnosed the domain itself or the frame-final sweep reconstructed the
    diagnosis from the published record. Those call for different work and this
    line is where an operator sees which.
    """

    AXIS = "domains that produced no rows"

    def scorecard(self, entries):
        manifest = {
            "compact_stream": {
                "frame_end_record_count": 3,
                "expected_domains": ["d47"],
                "per_domain": {
                    "d47": {"domain_id": 47, "data_record_count": 1,
                            "row_count_total": 0, "gap_record_count": None},
                },
                "gap_ledger": gap_ledger.pack_rows_for_tests([], []),
                "frame_degraded_outcomes": [
                    {"hour_raw": index, "domains": [entry]}
                    for index, entry in enumerate(entries)
                ],
            },
            "hourly_requirement": {},
            "replay_artifact": {},
        }
        return axis(manifest, self.AXIS)[1]

    def one_frame_scorecard(self, entries, axis_name):
        """Every entry in ONE frame, which `scorecard` above cannot express.

        `scorecard` puts each entry in its own hour, so entries never meet
        inside a single frame's offender set. Every defect that only appears
        when two entries are COMPARED to each other is invisible to it.
        """
        manifest = {
            "compact_stream": {
                "frame_end_record_count": 1,
                "expected_domains": ["d47"],
                "per_domain": {
                    "d47": {"domain_id": 47, "data_record_count": 1,
                            "row_count_total": 0, "gap_record_count": None},
                },
                "gap_ledger": gap_ledger.pack_rows_for_tests([], []),
                "frame_degraded_outcomes": [{"hour_raw": 0, "domains": entries}],
            },
            "hourly_requirement": {},
            "replay_artifact": {},
        }
        return axis(manifest, axis_name)[1]

    def entry(self, predicate, reason, code=0x00020011):
        return {
            "domain_id": 47,
            "status_flags": 0x100,
            "non_tolerated_flags": 0x100,
            "predicate_code": code,
            "predicate_name": predicate,
            "authority": "authority=self",
            "reason": reason,
        }

    def test_one_cause_with_per_frame_values_counts_once(self):
        detail = self.scorecard([
            self.entry("outcome_substantiated_from_record", "d47 saw 3 rows"),
            self.entry("outcome_substantiated_from_record", "d47 saw 7 rows"),
            self.entry("outcome_substantiated_from_record", "d47 saw 9 rows"),
        ])
        self.assertIn("3x outcome_substantiated_from_record", detail)

    def test_two_predicates_are_two_causes_however_the_reasons_read(self):
        """The reverse mistake: one reason string, two different checks.

        Grouping by reason would report a single cause with three hits and hide
        that a producer-declared failure and a swept one are both present.
        """
        detail = self.scorecard([
            self.entry("domain47_manager_is_readable", "d47 read refused"),
            self.entry("domain47_manager_is_readable", "d47 read refused"),
            self.entry("producer_declared_no_predicate", "d47 read refused",
                       code=0x00020012),
        ])
        self.assertIn("2x domain47_manager_is_readable", detail)

    def test_one_name_shared_by_two_codes_is_two_causes(self):
        """The CODE is the identity within a domain, not the name.

        The shared contract defines a predicate as a code plus a name and says
        the code is per-domain rather than globally unique. Keying on the name
        merges two genuinely different checks that happen to read alike, which
        is the same "one cause, counted twice" error the reason grouping made.
        """
        detail = self.scorecard([
            self.entry("read_refused", "a", code=0x00020011),
            self.entry("read_refused", "b", code=0x00020012),
            self.entry("read_refused", "c", code=0x00020012),
        ])
        self.assertIn("2x read_refused [0x00020012]", detail)
        self.assertNotIn("3x", detail)

    def test_one_code_whose_name_was_reworded_is_still_one_cause(self):
        """And the split in the other direction."""
        detail = self.scorecard([
            self.entry("manager_is_readable", "a"),
            self.entry("manager_readable", "b"),
            self.entry("manager_is_readable", "c"),
        ])
        self.assertIn("3x", detail)

    def test_the_reason_is_still_shown_as_the_sample(self):
        detail = self.scorecard([
            self.entry("outcome_substantiated_from_record", "d47 saw 3 rows"),
        ])
        self.assertIn("d47 saw 3 rows", detail)

    def test_an_entry_without_a_predicate_is_named_as_such_not_as_its_reason(self):
        """Version 2 always writes a predicate, so a missing one is a BROKEN
        capture, not an older one. Falling back to the reason -- what this file
        did before -- dressed that up as an ordinary cause and hid it. There is
        no pre-version-2 read path here; old captures are disposable.
        """
        detail = self.scorecard([
            {"domain_id": 47, "reason": "d47 read refused"},
        ])
        self.assertIn("1x (no readable predicate)", detail)
        self.assertNotIn("1x d47 read refused", detail)

    def test_two_differently_typed_ids_in_one_frame_do_not_crash_the_sort(self):
        """The defect a per-entry check cannot see.

        Admitting any hashable id let integer `47` and string `"47"` land in one
        offender set. Each is individually fine; together they made
        `sorted(always_degrading)` raise `TypeError: '<' not supported between
        instances of 'str' and 'int'`, which took the WHOLE scorecard down --
        every unrelated axis with it -- from inside the render, long after the
        entries were read.

        A domain id is a `u16` on the wire, so the string is malformed and is
        excluded and COUNTED rather than admitted or coerced. `int("47")` would
        merge it into d47's real numbers and invent an attribution.
        """
        detail = self.one_frame_scorecard(
            [
                {"domain_id": 47, "predicate_code": 0x11,
                 "predicate_name": "n", "reason": "a"},
                {"domain_id": "47", "predicate_code": 0x11,
                 "predicate_name": "n", "reason": "b"},
            ],
            "degraded frames",
        )
        self.assertIn("d47", detail)
        self.assertIn("1 degraded record unreadable", detail)

    def test_a_frame_whose_domains_is_not_a_list_is_counted_not_assumed_empty(self):
        """Treating it as an empty frame said "this hour degraded nothing",
        which is a CLAIM about the hour rather than an admission that the record
        could not be read. The two call for different work.
        """
        manifest = {
            "compact_stream": {
                "frame_end_record_count": 1,
                "expected_domains": ["d47"],
                "per_domain": {
                    "d47": {"domain_id": 47, "data_record_count": 1,
                            "row_count_total": 0, "gap_record_count": None},
                },
                "gap_ledger": gap_ledger.pack_rows_for_tests([], []),
                "frame_degraded_outcomes": [{"hour_raw": 0, "domains": "nope"}],
            },
            "hourly_requirement": {},
            "replay_artifact": {},
        }
        detail = axis(manifest, "degraded frames")[1]
        self.assertIn("1 degraded record unreadable", detail)

    def test_an_absent_outcomes_block_is_not_counted_as_unreadable(self):
        """Absent is not malformed: a clean capture omits the key, and counting
        that would put a permanent 1 on every clean capture.
        """
        manifest = {
            "compact_stream": {
                "frame_end_record_count": 1,
                "expected_domains": ["d47"],
                "per_domain": {
                    "d47": {"domain_id": 47, "data_record_count": 1,
                            "row_count_total": 0, "gap_record_count": None},
                },
                "gap_ledger": gap_ledger.pack_rows_for_tests([], []),
            },
            "hourly_requirement": {},
            "replay_artifact": {},
        }
        detail = axis(manifest, "degraded frames")[1]
        self.assertNotIn("unreadable", detail)

    def test_malformed_entries_cost_their_own_line_not_the_scorecard(self):
        """Shapes the recorder should never write, each of which used to raise
        on the way to a verdict: a report that dies on one bad entry says
        nothing about the other 72 domains.
        """
        detail = self.scorecard([
            "not a dict",
            {"domain_id": 47, "predicate_code": 10.9, "reason": "x"},
            {"domain_id": 47, "predicate_code": True, "reason": "x"},
            {"domain_id": 47, "predicate_code": 0x11, "predicate_name": 7,
             "reason": "x"},
            {"domain_id": 47, "predicate_code": 0x11, "predicate_name": "n",
             "reason": 7},
            {"domain_id": ["unhashable"], "predicate_code": 0x11,
             "predicate_name": "n", "reason": "x"},
            self.entry("manager_is_readable", "the readable one"),
        ])
        self.assertIn("d47", detail)


class EvidenceQualification(unittest.TestCase):
    def test_passing_gate_does_not_hide_a_failed_container_publication(self):
        value = manifest([], [], frames=1)
        value["replay_artifact"] = {
            "gate_problems": [], "reason": "gate_passed", "pack_failed": True,
            "written_file": None, "pack_error": "no game hour is verified",
        }
        self.assertEqual(axis(value, "replay gate problems")[0], 0)
        self.assertEqual(axis(value, "replay container packaging failed"),
                         (1, "no game hour is verified"))

    def test_identity_binds_parsed_bytes_even_if_manifest_is_replaced(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "native_session_manifest.json"
            raw = b'{"session_id":"historical"}\r\n'
            path.write_bytes(raw)
            loaded, source, identity = SCORECARD.load_manifest(directory)
            path.write_bytes(b'{"session_id":"replacement"}')
            self.assertEqual(loaded["session_id"], "historical")
            self.assertEqual(identity, {
                "path": str(path.resolve()), "bytes": len(raw),
                "sha256": hashlib.sha256(raw).hexdigest(),
            })
            self.assertEqual(source, str(path))

    def test_missing_or_forged_lifecycle_receipts_never_establish_authority(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "CAPTURE_TEARDOWN_V1.json"
            path.write_text('{"hooks_restored":true,"collector_unloaded":true}')
            refs = SCORECARD.evidence_references(directory, {})
            self.assertEqual(refs["scope"], "manifest_completeness_only")
            receipts = refs["lifecycle_receipts"]
            self.assertEqual(receipts["CAPTURE_READY_V2.json"]["status"], "missing")
            self.assertEqual(receipts[path.name]["status"], "present_not_authenticated")
            self.assertEqual(receipts[path.name]["identity"]["sha256"],
                             hashlib.sha256(path.read_bytes()).hexdigest())
            self.assertIn("hooks_restored_byte_for_byte", refs["not_evaluated"])
            self.assertIn("current_release_artifact_binding", refs["not_evaluated"])

    def test_unreadable_receipt_is_distinct_from_missing(self):
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / "CAPTURE_READY_V2.json").mkdir()
            refs = SCORECARD.evidence_references(directory, {})
            self.assertEqual(refs["lifecycle_receipts"]["CAPTURE_READY_V2.json"]["status"],
                             "unreadable")

    def test_empty_capture_cli_fails_with_qualified_json_and_exact_source(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "session_manifest.json"
            path.write_text(json.dumps(manifest([], [], frames=0, domain_ids=())))
            result = subprocess.run([sys.executable, "-B", str(SCRIPT),
                                     "--run-dir", directory, "--json"],
                                    capture_output=True, text=True, timeout=30)
            self.assertEqual(result.returncode, 1, result.stderr)
            receipt = json.loads(result.stdout)
            self.assertFalse(receipt["complete"])
            counts = {row["axis"]: row["count"] for row in receipt["axes"]}
            self.assertEqual(counts["capture has no observed frames"], 1)
            self.assertEqual(counts["capture has no expected domain roster"], 1)
            self.assertEqual(receipt["evidence"]["manifest_identity"]["sha256"],
                             hashlib.sha256(path.read_bytes()).hexdigest())
            self.assertIn("container_and_content_admission",
                          receipt["evidence"]["not_evaluated"])

    def test_witnessed_frame_and_roster_clear_only_their_own_axes(self):
        value = manifest([], [], frames=1)
        self.assertEqual(axis(value, "capture has no observed frames")[0], 0)
        self.assertEqual(axis(value, "capture has no expected domain roster")[0], 0)
        self.assertTrue(any(count for _, count, _ in SCORECARD.score(value)))


if __name__ == "__main__":
    unittest.main()
