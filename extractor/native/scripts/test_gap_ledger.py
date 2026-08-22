#!/usr/bin/env python3
"""Hold `gap_ledger.py` to the same vector the Rust encoder is held to.

The sealed manifest's gap ledger is expanded by TWO implementations -- the
recorder's own (`recorder_host/src/gap_ledger.rs`) and this directory's, which
the capture scorecard and the sink verifier both read captures through. Two
expanders written from one description agree until the first time either learns
something the other did not, and the failure mode is a capture graded on hours
nobody recorded.

`gap_ledger_conformance.json` is the one vector both are held to. The Rust test
`the_cross_language_conformance_vector_matches_this_encoder` owns its content;
this file proves the Python side reaches the same rows from it.
"""

from __future__ import annotations

import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gap_ledger

VECTOR = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                      "gap_ledger_conformance.json")


def canonical(rows):
    """The order the vector stores rows in: each row by its own serialization."""
    return sorted(
        (json.loads(json.dumps(row, sort_keys=True)) for row in rows),
        key=lambda row: json.dumps(row, sort_keys=True, separators=(",", ":")),
    )


class ConformanceVector(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with open(VECTOR, encoding="utf-8") as handle:
            cls.vector = json.load(handle)
        cls.stream = {"gap_ledger": cls.vector["gap_ledger"]}

    def test_gaps_expand_to_the_rows_the_recorder_says_they_do(self):
        """WHAT PRODUCTION CHANGE MAKES THIS FAIL: any divergence between this
        expander and the Rust one -- a witness field left un-rebuilt, a run
        walked one hour short, a `shared` class expanded per domain."""
        rows = gap_ledger.expand_gaps(self.stream)
        self.assertEqual(canonical(rows), canonical(self.vector["expected_gaps"]))

    def test_outcomes_expand_to_the_rows_the_recorder_says_they_do(self):
        """Includes the derived `outcome_ordinal`, which is the field this
        encoding does not store and has to reconstruct from hour order."""
        rows = gap_ledger.expand_domain_gap_outcomes(self.stream)
        self.assertEqual(
            canonical(rows), canonical(self.vector["expected_domain_gap_outcomes"])
        )

    def test_the_vector_exercises_the_expander_on_real_volume(self):
        """A vector that expanded to nothing would pass both tests above while
        proving nothing at all.

        Counted from the EXPANDER's output, not from the checked-in arrays: a
        vector with 137 expected rows and an expander that returns none would
        satisfy a check on the file's array lengths and fail this one. Every
        witness kind and both groupings have to come out the other side, because
        each is a separate branch of `_witness` and `expand_gaps`."""
        gaps = gap_ledger.expand_gaps(self.stream)
        outcomes = gap_ledger.expand_domain_gap_outcomes(self.stream)
        self.assertGreater(len(gaps), 50)
        self.assertGreater(len(outcomes), 50)
        kinds = {row["witness"]["kind"] for row in gaps}
        self.assertEqual(
            kinds,
            {"coverage", "coverage_and_status", "counter", "status_bits",
             "hour_discontinuity", "torn_sweep"},
            "the vector no longer exercises every witness branch")
        self.assertIn([], [row["domains"] for row in gaps],
                      "the vector no longer carries a whole-clock (shared) row")


class Compatibility(unittest.TestCase):
    def test_a_manifest_carrying_only_the_old_flat_arrays_states_no_ledger(self):
        """There is ONE format. The flat arrays this replaced are not read.

        What matters is that the refusal is legible: `ledger_present` says no,
        so a caller cannot mistake the empty expansion for "this capture had no
        gaps" -- the strongest false claim this ledger can make.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: reintroducing a fallback that
        reads `compact_stream.gaps`."""
        flat = {
            "gaps": [{"hour_start": 5, "hour_end": 5, "reason": "skipped_hour",
                      "domains": [], "witness": {"kind": "hour_discontinuity",
                                                 "previous_hour": 4, "next_hour": 6}}],
            "domain_gap_outcomes": [{"domain_id": 10, "part": "both",
                                     "reason": "backpressure", "target_hour_raw": 5,
                                     "range_start_hour_raw": 5, "range_end_hour_raw": 5,
                                     "tick_before": 5, "tick_after": 5,
                                     "queue_capacity": 1, "queue_depth": 1,
                                     "queue_dropped": 1, "outcome_ordinal": 0}],
        }
        self.assertFalse(gap_ledger.ledger_present(flat))
        with self.assertRaises(gap_ledger.GapLedgerError):
            gap_ledger.expand_gaps(flat)
        with self.assertRaises(gap_ledger.GapLedgerError):
            gap_ledger.expand_domain_gap_outcomes(flat)

    def test_the_fixture_packer_round_trips_the_rows_it_was_given(self):
        """`pack_rows_for_tests` is what every fixture in this directory now goes
        through, so a packer that dropped a field would make those fixtures
        assert about rows nobody wrote.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: the packer or the expander
        learning a field the other does not."""
        gaps = [{"hour_start": 5, "hour_end": 7, "reason": "skipped_hour",
                 "domains": [], "witness": {"kind": "hour_discontinuity",
                                            "previous_hour": 4, "next_hour": 8}},
                {"hour_start": 9, "hour_end": 9, "reason": "worker_panic",
                 "domains": [12], "witness": {"kind": "status_bits",
                                              "status_bits": 64}}]
        outcomes = [{"domain_id": 10, "part": "both", "reason": "backpressure",
                     "target_hour_raw": 5, "range_start_hour_raw": 4,
                     "range_end_hour_raw": 5, "tick_before": 5, "tick_after": 6,
                     "queue_capacity": 1, "queue_depth": 1, "queue_dropped": 2,
                     "outcome_ordinal": 3}]
        stream = {"gap_ledger": gap_ledger.pack_rows_for_tests(gaps, outcomes)}
        self.assertEqual(canonical(gap_ledger.expand_gaps(stream)), canonical(gaps))
        self.assertEqual(canonical(gap_ledger.expand_domain_gap_outcomes(stream)),
                         canonical(outcomes))

    def test_the_fixture_packer_encodes_same_hour_multiplicity(self):
        """Two rows for one domain at one hour are TWO absences. The wire carries
        that as the domain listed twice in one class; a packer that emitted two
        runs at the same hour instead produced a run list its own expander
        refuses as non-ascending.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: packing one class per row, or
        grouping by an identity that includes the domains."""
        rows = [
            {"domain_id": 4, "part": "both", "reason": "backpressure",
             "target_hour_raw": 100, "range_start_hour_raw": 100,
             "range_end_hour_raw": 100, "tick_before": 100, "tick_after": 100,
             "queue_capacity": 1, "queue_depth": 1, "queue_dropped": 1,
             "outcome_ordinal": 7},
            {"domain_id": 4, "part": "both", "reason": "backpressure",
             "target_hour_raw": 100, "range_start_hour_raw": 100,
             "range_end_hour_raw": 100, "tick_before": 100, "tick_after": 100,
             "queue_capacity": 1, "queue_depth": 1, "queue_dropped": 1,
             "outcome_ordinal": 8},
        ]
        stream = {"gap_ledger": gap_ledger.pack_rows_for_tests([], rows)}
        packed = stream["gap_ledger"]["domain_gap_outcomes"]["classes"]
        self.assertEqual(len(packed), 1, "same-hour rows must share one class")
        self.assertEqual(packed[0]["domains"], [4, 4],
                         "multiplicity belongs in the domain list, not in two runs")
        self.assertEqual(canonical(gap_ledger.expand_domain_gap_outcomes(stream)),
                         canonical(rows))

    def test_a_class_carrying_a_field_this_format_does_not_define_is_refused(self):
        """THE STRUCTURAL CLOSE on the whole "ignored field" family. A document
        carrying something no reader uses expands to exactly what the document
        without it does -- and, worse, an unread field still counts toward a
        class's IDENTITY, so adding one splits an aggregation key and walks
        straight through the same-hour check.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: naming a new class field in the
        encoder without adding it to the reader's allowed set (which is the
        point: the reader must be told)."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        cases = [
            ("gaps", ["classes", 0], "novel_field"),
            ("domain_gap_outcomes", ["classes", 0], "repeat"),
            ("domain_gap_outcomes", ["lane_ordinal_first", 0], "extra"),
        ]
        for table, path, field in cases:
            ledger = json.loads(json.dumps(base))
            target = ledger[table]
            for step in path:
                target = target[step]
            target[field] = 999
            expand = (gap_ledger.expand_gaps if table == "gaps"
                      else gap_ledger.expand_domain_gap_outcomes)
            with self.assertRaises(gap_ledger.GapLedgerError) as caught:
                expand({"gap_ledger": ledger})
            self.assertIn("does not define", str(caught.exception))

        ledger = json.loads(json.dumps(base))
        ledger["gaps"]["classes"][0]["witness"]["novel"] = 1
        with self.assertRaises(gap_ledger.GapLedgerError):
            gap_ledger.expand_gaps({"gap_ledger": ledger})
        ledger = json.loads(json.dumps(base))
        ledger["domain_gap_outcomes"]["classes"][0]["tick_before"]["novel"] = 1
        with self.assertRaises(gap_ledger.GapLedgerError):
            gap_ledger.expand_domain_gap_outcomes({"gap_ledger": ledger})

    def test_a_table_carrying_a_field_this_format_does_not_define_is_refused(self):
        """Closing the CLASSES but not their CONTAINER left the same escape one
        level up: a field on the table is a spelling the encoder cannot emit, and
        the container is exactly where a future field would be added.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: adding a table-level field to the
        encoder without adding it to the reader's allowed set."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        for table, expand in (("gaps", gap_ledger.expand_gaps),
                              ("domain_gap_outcomes",
                               gap_ledger.expand_domain_gap_outcomes)):
            ledger = json.loads(json.dumps(base))
            ledger[table]["novel_table_field"] = 1
            with self.assertRaises(gap_ledger.GapLedgerError) as caught:
                expand({"gap_ledger": ledger})
            self.assertIn("does not define", str(caught.exception))

        for expand in (gap_ledger.expand_gaps, gap_ledger.expand_domain_gap_outcomes):
            ledger = json.loads(json.dumps(base))
            ledger["novel_ledger_field"] = 1
            with self.assertRaises(gap_ledger.GapLedgerError) as caught:
                expand({"gap_ledger": ledger})
            self.assertIn("does not define", str(caught.exception))

    def test_the_ledger_note_must_be_a_string(self):
        """Allowlisting `note` BY NAME left an object-shaped hole: a name
        allowlist says nothing about the value under it, so
        `note: {"novel_nested_field": 1}` walked through every reader.

        The exact prose is deliberately NOT pinned -- the note is documentation,
        part of no class identity, and cannot change a row -- but an object could
        carry structure, so the TYPE is required.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: allowlisting a field's name
        without saying what may be under it."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        for bad in ({"novel_nested_field": 1}, [1, 2], 7):
            ledger = json.loads(json.dumps(base))
            ledger["note"] = bad
            for expand in (gap_ledger.expand_gaps,
                           gap_ledger.expand_domain_gap_outcomes):
                with self.assertRaises(gap_ledger.GapLedgerError) as caught:
                    expand({"gap_ledger": ledger})
                self.assertIn("note", str(caught.exception))
        ledger = json.loads(json.dumps(base))
        ledger["note"] = "anything at all"
        self.assertTrue(gap_ledger.expand_gaps({"gap_ledger": ledger}))

    def test_a_shared_class_naming_one_domain_is_refused(self):
        """A one-domain record is always `per_domain` on the encoder's side, so
        `grouping:"shared", domains:[12], repeat:1` is a second spelling of rows
        the encoder writes one way.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: letting `shared` accept any
        domain list, which is what the grouping tag alone allowed."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        ledger = json.loads(json.dumps(base))
        target = next(class_ for class_ in ledger["gaps"]["classes"]
                      if class_["grouping"] == "shared")
        target["domains"] = [12]
        with self.assertRaises(gap_ledger.GapLedgerError) as caught:
            gap_ledger.expand_gaps({"gap_ledger": ledger})
        self.assertIn("per_domain", str(caught.exception))

    def test_the_work_bound_counts_rows_not_hours(self):
        """A shared class multiplies every hour by `repeat`, so an hour bound
        alone admitted `repeat: u64::MAX` and the loop under it -- a hang, on a
        ledger that declares one row.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: bounding the hours a class names
        rather than the rows it emits."""
        ledger = {
            "schema": gap_ledger.GAP_LEDGER_SCHEMA,
            "version": gap_ledger.GAP_LEDGER_VERSION,
            "gaps": {"row_count": 1, "class_count": 1, "classes": [{
                "reason": "skipped_hour", "grouping": "shared", "domains": [],
                "repeat": 2 ** 64 - 1, "span_hours": 1,
                "witness": {"kind": "hour_discontinuity",
                            "previous_hour_rel": -1, "next_hour_rel": 1},
                "hour_runs": [[0, 1]]}]},
            "domain_gap_outcomes": {"row_count": 0, "class_count": 0,
                                    "ordinal_encoding": "explicit", "classes": []},
        }
        with self.assertRaises(gap_ledger.GapLedgerError):
            gap_ledger.expand_gaps({"gap_ledger": ledger})

    def test_the_fixture_packer_keeps_shared_row_multiplicity(self):
        """A shared row carries its multiplicity in `repeat`, not in a domain
        list, so per-domain aggregation would have folded two identical
        whole-clock rows into one -- losing a row the fixture asked for.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: running the per-domain grouping
        over shared classes too."""
        row = {"hour_start": 5, "hour_end": 5, "reason": "skipped_hour",
               "domains": [], "witness": {"kind": "hour_discontinuity",
                                          "previous_hour": 4, "next_hour": 6}}
        rows = [json.loads(json.dumps(row)) for _ in range(3)]
        stream = {"gap_ledger": gap_ledger.pack_rows_for_tests(rows, [])}
        self.assertEqual(stream["gap_ledger"]["gaps"]["row_count"], 3)
        self.assertEqual(canonical(gap_ledger.expand_gaps(stream)), canonical(rows))

    def test_a_version_this_reader_does_not_implement_is_refused(self):
        """Refused, not guessed at: a later version may mean the same key
        differently, and a reader that assumed otherwise publishes hours it
        invented.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: reading any object that has a
        `classes` array regardless of what it says it is."""
        with open(VECTOR, encoding="utf-8") as handle:
            ledger = json.load(handle)["gap_ledger"]
        for broken in ({**ledger, "version": gap_ledger.GAP_LEDGER_VERSION + 1},
                       {**ledger, "version": float(gap_ledger.GAP_LEDGER_VERSION)},
                       {**ledger, "schema": "some-other-ledger"}):
            with self.assertRaises(gap_ledger.GapLedgerError):
                gap_ledger.expand_gaps({"gap_ledger": broken})

    def test_a_manifest_with_no_ledger_at_all_is_distinguishable_from_an_empty_one(self):
        """"No ledger" and "an empty ledger" are different claims, and a gate
        that cannot tell them apart passes vacuously."""
        self.assertFalse(gap_ledger.ledger_present({"record_count": 5}))
        empty = {"gap_ledger": gap_ledger.pack_rows_for_tests()}
        self.assertTrue(gap_ledger.ledger_present(empty))
        self.assertEqual(gap_ledger.expand_gaps(empty), [])
        with self.assertRaises(gap_ledger.GapLedgerError):
            gap_ledger.expand_gaps({"record_count": 5})

    def test_a_truncated_class_list_is_caught_by_the_declared_row_count(self):
        """WHAT PRODUCTION CHANGE MAKES THIS FAIL: trusting `row_count` instead
        of checking it, which would let a truncated ledger read as a shorter
        session."""
        with open(VECTOR, encoding="utf-8") as handle:
            ledger = json.load(handle)["gap_ledger"]
        ledger["gaps"]["classes"] = ledger["gaps"]["classes"][:-1]
        with self.assertRaises(gap_ledger.GapLedgerError):
            gap_ledger.expand_gaps({"gap_ledger": ledger})

    def test_a_malformed_class_raises_this_module_s_own_error_and_no_other(self):
        """Both callers catch `GapLedgerError` and report a problem. Any OTHER
        exception class walks straight past them and kills the tool, and a
        validator that dies mid-manifest publishes no verdict at all.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: indexing a class field directly
        (a `KeyError`), or assuming a list holds numbers (a `TypeError`)."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        breakages = [
            ("domain_gap_outcomes", lambda c: c.pop("range_start_rel")),
            ("domain_gap_outcomes", lambda c: c.__setitem__("tick_before", {"rel": "x"})),
            ("gaps", lambda c: c.pop("span_hours")),
            ("gaps", lambda c: c.__setitem__("witness", {"kind": "hour_discontinuity"})),
            ("gaps", lambda c: c.__setitem__("domains", ["not a domain id"])),
        ]
        for table, break_it in breakages:
            ledger = json.loads(json.dumps(base))
            break_it(ledger[table]["classes"][0])
            expand = (gap_ledger.expand_gaps if table == "gaps"
                      else gap_ledger.expand_domain_gap_outcomes)
            with self.assertRaises(gap_ledger.GapLedgerError):
                expand({"gap_ledger": ledger})

    def test_the_empty_ledger_a_zero_loss_capture_seals_expands_cleanly(self):
        """Every refusal in this file tightens what the expander accepts, and the
        one document it must never refuse is the ledger a promotable capture
        seals: empty `classes`, empty `lane_ordinal_first`, `row_count` 0.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: writing a presence check as a
        truthiness test, which reads an empty list as absent."""
        empty = {"gap_ledger": {
            "schema": gap_ledger.GAP_LEDGER_SCHEMA,
            "version": gap_ledger.GAP_LEDGER_VERSION,
            "gaps": {"row_count": 0, "class_count": 0, "classes": []},
            "domain_gap_outcomes": {"row_count": 0, "class_count": 0,
                                    "ordinal_encoding": "lane_sequential",
                                    "lane_ordinal_first": [], "classes": []},
        }}
        self.assertTrue(gap_ledger.ledger_present(empty["gap_ledger"] and empty))
        self.assertEqual(gap_ledger.expand_gaps(empty), [])
        self.assertEqual(gap_ledger.expand_domain_gap_outcomes(empty), [])

    def test_a_field_that_should_be_a_name_is_refused_when_it_is_not(self):
        """Copying `reason` unchecked was not caught by the exception guard,
        because nothing THREW: the expander returned rows whose `reason` was a
        dict and the CALLERS then died grouping by it. A wrapper cannot bound a
        failure that happens after it returns.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: reading a name field with
        `.get()` instead of checking it."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        cases = [
            ("domain_gap_outcomes", "reason"), ("domain_gap_outcomes", "part"),
            ("gaps", "reason"), ("gaps", "grouping"),
        ]
        explicit = json.loads(json.dumps(base))
        explicit["domain_gap_outcomes"]["ordinal_encoding"] = "explicit"
        del explicit["domain_gap_outcomes"]["lane_ordinal_first"]
        for class_ in explicit["domain_gap_outcomes"]["classes"]:
            hours = sum(run[1] for run in class_["hour_runs"])
            class_["ordinals"] = [{}] * (hours * len(class_["domains"]))
        with self.assertRaises(gap_ledger.GapLedgerError) as caught:
            gap_ledger.expand_domain_gap_outcomes({"gap_ledger": explicit})
        self.assertIn("ordinal", str(caught.exception))
        for table, field in cases:
            for bad in ({}, [], 7, None):
                ledger = json.loads(json.dumps(base))
                ledger[table]["classes"][0][field] = bad
                expand = (gap_ledger.expand_gaps if table == "gaps"
                          else gap_ledger.expand_domain_gap_outcomes)
                with self.assertRaises(gap_ledger.GapLedgerError):
                    expand({"gap_ledger": ledger})

    def test_a_witness_payload_is_rebuilt_and_typed_not_copied(self):
        """Copying the witness template wholesale let `status_bits: {}` through,
        which the recorder's own reader refuses and which reaches a caller as a
        row field it then masks and compares.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: `return dict(template)`."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        for index, class_ in enumerate(base["gaps"]["classes"]):
            for field, value in list(class_["witness"].items()):
                if field == "kind" or not isinstance(value, int):
                    continue
                for bad in ({}, "7"):
                    ledger = json.loads(json.dumps(base))
                    ledger["gaps"]["classes"][index]["witness"][field] = bad
                    with self.assertRaises(gap_ledger.GapLedgerError):
                        gap_ledger.expand_gaps({"gap_ledger": ledger})
        u32_fields = {"status_bits", "expected_perf_records", "observed_perf_records",
                      "expected_data_records", "observed_data_records"}
        checked = 0
        for index, class_ in enumerate(base["gaps"]["classes"]):
            for field in set(class_["witness"]) & u32_fields:
                for bad in (-1, 2 ** 32):
                    ledger = json.loads(json.dumps(base))
                    ledger["gaps"]["classes"][index]["witness"][field] = bad
                    with self.assertRaises(gap_ledger.GapLedgerError):
                        gap_ledger.expand_gaps({"gap_ledger": ledger})
                checked += 1
        self.assertGreater(checked, 0, "the vector carries no u32 witness field")

    def test_a_name_outside_the_contract_vocabulary_is_refused(self):
        """A reason this reader admits and the producer's decoder refuses is two
        readers of one document disagreeing about what it says. The sets are
        pinned to the contract enums by
        `compact_contract/tests/compact_contract_suite/script_status_vocabulary_sweep.rs`.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: accepting any non-empty string
        where a wire name belongs."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        cases = [("gaps", "reason", "not_a_reason"),
                 ("domain_gap_outcomes", "reason", "not_a_reason"),
                 ("domain_gap_outcomes", "part", "neither")]
        for table, field, bad in cases:
            ledger = json.loads(json.dumps(base))
            ledger[table]["classes"][0][field] = bad
            expand = (gap_ledger.expand_gaps if table == "gaps"
                      else gap_ledger.expand_domain_gap_outcomes)
            with self.assertRaises(gap_ledger.GapLedgerError):
                expand({"gap_ledger": ledger})

    @staticmethod
    def _rows_of(class_):
        """How many rows a class expands to, so a mutation can repair the count."""
        hours = sum(run[1] for run in class_.get("hour_runs", []))
        if class_.get("grouping") == "shared":
            return hours * class_.get("repeat", 1)
        return hours * len(class_.get("domains", []))

    def _repair_row_count(self, ledger, table, mutated_index):
        rows = sum(self._rows_of(class_)
                   for index, class_ in enumerate(ledger[table]["classes"])
                   if index != mutated_index)
        rows += self._rows_of(ledger[table]["classes"][mutated_index])
        ledger[table]["row_count"] = rows

    def test_a_class_that_names_no_hour_or_no_domain_is_refused(self):
        """A class with an empty run list, or a per-domain class with no domain,
        expands to nothing -- so a table could carry any number of them and still
        mean exactly what the empty table means. Two documents, one expansion.

        `row_count` is REPAIRED to what each mutated ledger really expands to, or
        the count check fires first and the test passes with the guard under test
        deleted. The expected message is asserted for the same reason.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: checking only that each RUN is
        non-empty and not that the run LIST is, or allowing an empty domain list
        where it does not mean "every domain"."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        cases = [
            ("gaps", "hour_runs", [], "names no row"),
            ("domain_gap_outcomes", "hour_runs", [], "names no row"),
            ("domain_gap_outcomes", "domains", [], "names no domain"),
        ]
        for table, field, value, fragment in cases:
            ledger = json.loads(json.dumps(base))
            ledger[table]["classes"][0][field] = value
            self._repair_row_count(ledger, table, 0)
            expand = (gap_ledger.expand_gaps if table == "gaps"
                      else gap_ledger.expand_domain_gap_outcomes)
            with self.assertRaises(gap_ledger.GapLedgerError) as caught:
                expand({"gap_ledger": ledger})
            self.assertIn(fragment, str(caught.exception))

        ledger = json.loads(json.dumps(base))
        index = next(i for i, class_ in enumerate(ledger["gaps"]["classes"])
                     if class_["grouping"] == "per_domain")
        ledger["gaps"]["classes"][index]["domains"] = []
        self._repair_row_count(ledger, "gaps", index)
        with self.assertRaises(gap_ledger.GapLedgerError) as caught:
            gap_ledger.expand_gaps({"gap_ledger": ledger})
        self.assertIn("names no domain", str(caught.exception))

    def test_an_unsigned_field_is_refused_when_it_is_negative(self):
        """The contract's counts, ticks and ordinals are unsigned. A reader that
        admits a negative one hands its caller a figure the producer could not
        have written and its own decoder refuses.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: checking only "is an integer"
        where the contract says `u32` or `u64`."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        for field in ("queue_capacity", "queue_depth", "queue_dropped"):
            ledger = json.loads(json.dumps(base))
            ledger["domain_gap_outcomes"]["classes"][0][field] = -1
            with self.assertRaises(gap_ledger.GapLedgerError):
                gap_ledger.expand_domain_gap_outcomes({"gap_ledger": ledger})

        ledger = json.loads(json.dumps(base))
        ledger["domain_gap_outcomes"]["lane_ordinal_first"][0]["first"] = -1
        with self.assertRaises(gap_ledger.GapLedgerError):
            gap_ledger.expand_domain_gap_outcomes({"gap_ledger": ledger})

        ledger = json.loads(json.dumps(base))
        ledger["domain_gap_outcomes"]["classes"][0]["tick_before"] = {"abs": -1}
        with self.assertRaises(gap_ledger.GapLedgerError):
            gap_ledger.expand_domain_gap_outcomes({"gap_ledger": ledger})

        ledger = json.loads(json.dumps(base))
        ledger["domain_gap_outcomes"]["classes"][0]["tick_before"] = {"rel": -(2 ** 40)}
        with self.assertRaises(gap_ledger.GapLedgerError):
            gap_ledger.expand_domain_gap_outcomes({"gap_ledger": ledger})

    def test_a_lane_entry_that_is_unused_duplicated_or_out_of_range_is_refused(self):
        """An unused or duplicated lane entry leaves the expansion unchanged, so
        a table carrying either means what the table without it means -- the same
        non-injectivity a class that names no row carries. A lane domain outside
        `u16` is a domain that cannot exist.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: loading the lane table into a map
        and only checking that every EXPANDED lane has a base."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]

        unused = json.loads(json.dumps(base))
        unused["domain_gap_outcomes"]["lane_ordinal_first"].append(
            {"domain_id": 999, "part": "both", "first": 0})
        with self.assertRaises(gap_ledger.GapLedgerError):
            gap_ledger.expand_domain_gap_outcomes({"gap_ledger": unused})

        duplicated = json.loads(json.dumps(base))
        lanes = duplicated["domain_gap_outcomes"]["lane_ordinal_first"]
        lanes.append(json.loads(json.dumps(lanes[0])))
        with self.assertRaises(gap_ledger.GapLedgerError):
            gap_ledger.expand_domain_gap_outcomes({"gap_ledger": duplicated})

        used = base["domain_gap_outcomes"]["lane_ordinal_first"][0]
        for bad in (-1, 65536):
            wide = json.loads(json.dumps(base))
            wide["domain_gap_outcomes"]["lane_ordinal_first"][0]["domain_id"] = bad
            for class_ in wide["domain_gap_outcomes"]["classes"]:
                class_["domains"] = [d if d != used["domain_id"] else bad
                                     for d in class_["domains"]]
            with self.assertRaises(gap_ledger.GapLedgerError) as caught:
                gap_ledger.expand_domain_gap_outcomes({"gap_ledger": wide})
            self.assertIn("domain id", str(caught.exception))

    def test_the_two_ordinal_encodings_do_not_carry_each_other_s_fields(self):
        """A `lane_sequential` table IGNORES a class's `ordinals`, and an
        `explicit` one IGNORES `lane_ordinal_first` -- so a document carrying the
        ignored field means exactly what the document without it means. Two
        documents, one expansion, which is the non-injectivity the empty-class and
        duplicate-lane checks also close.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: reading the field the branch uses
        and saying nothing about the one it does not."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]

        sequential = json.loads(json.dumps(base))
        self.assertEqual(sequential["domain_gap_outcomes"]["ordinal_encoding"],
                         "lane_sequential")
        sequential["domain_gap_outcomes"]["classes"][0]["ordinals"] = [999]
        with self.assertRaises(gap_ledger.GapLedgerError):
            gap_ledger.expand_domain_gap_outcomes({"gap_ledger": sequential})

        explicit = json.loads(json.dumps(base))
        table = explicit["domain_gap_outcomes"]
        rows = gap_ledger.expand_domain_gap_outcomes({"gap_ledger": explicit})
        table["ordinal_encoding"] = "explicit"
        position = 0
        for class_ in table["classes"]:
            count = sum(run[1] for run in class_["hour_runs"]) * len(class_["domains"])
            class_["ordinals"] = [0] * count
            position += count
        self.assertEqual(position, len(rows))
        with self.assertRaises(gap_ledger.GapLedgerError):
            gap_ledger.expand_domain_gap_outcomes({"gap_ledger": explicit})

    def test_a_derived_ordinal_that_overflows_u64_is_refused(self):
        """The ordinal is a `u64` on the wire and the recorder's own reader adds
        with `checked_add`. An unbounded sum publishes an ordinal no producer
        could have written.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: `first + offset` with Python's
        unbounded integers."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        ledger = json.loads(json.dumps(base))
        for entry in ledger["domain_gap_outcomes"]["lane_ordinal_first"]:
            entry["first"] = 2 ** 64 - 1
        with self.assertRaises(gap_ledger.GapLedgerError):
            gap_ledger.expand_domain_gap_outcomes({"gap_ledger": ledger})

    def test_a_tick_offset_is_signed_and_its_resolved_tick_is_not(self):
        """`{"rel": -1}` is what the recorder writes wherever a tick precedes its
        hour, so the OFFSET is signed; the tick it resolves to is a `u64`, so the
        RESOLVED value is not. Conflating the two refuses a document the producer
        routinely seals, or admits a tick it cannot write.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: bounding the offset as unsigned,
        or leaving the resolved value unbounded."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]

        legal = json.loads(json.dumps(base))
        legal["domain_gap_outcomes"]["classes"][0]["tick_before"] = {"rel": -1}
        rows = gap_ledger.expand_domain_gap_outcomes({"gap_ledger": legal})
        target = rows[0]["target_hour_raw"]
        self.assertEqual(rows[0]["tick_before"], target - 1)

        for bad in ({"rel": -(2 ** 40)}, {"abs": -1}, {"abs": 2 ** 64},
                    {"rel": 2 ** 63}):
            ledger = json.loads(json.dumps(base))
            ledger["domain_gap_outcomes"]["classes"][0]["tick_before"] = bad
            with self.assertRaises(gap_ledger.GapLedgerError):
                gap_ledger.expand_domain_gap_outcomes({"gap_ledger": ledger})

    def test_a_non_canonical_run_list_or_class_count_is_refused(self):
        """`[[101,1],[102,2]]` expands to exactly what `[[101,3]]` does, so
        admitting both lets two documents mean one thing -- the same
        non-injectivity an empty class carries. And `class_count` is published
        beside the list, so a count that disagrees is a second claim about it.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: walking the runs without
        requiring them ascending and maximal, or ignoring `class_count`."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]

        for table in ("gaps", "domain_gap_outcomes"):
            expand = (gap_ledger.expand_gaps if table == "gaps"
                      else gap_ledger.expand_domain_gap_outcomes)
            split = json.loads(json.dumps(base))
            done = False
            for class_ in split[table]["classes"]:
                for index, (first, count) in enumerate(class_["hour_runs"]):
                    if count >= 2:
                        class_["hour_runs"][index:index + 1] = [
                            [first, 1], [first + 1, count - 1]]
                        done = True
                        break
                if done:
                    break
            self.assertTrue(done, f"the vector has no multi-hour run in {table}")
            with self.assertRaises(gap_ledger.GapLedgerError) as caught:
                expand({"gap_ledger": split})
            self.assertIn("maximal", str(caught.exception))

            miscounted = json.loads(json.dumps(base))
            miscounted[table]["class_count"] = 999
            with self.assertRaises(gap_ledger.GapLedgerError) as caught:
                expand({"gap_ledger": miscounted})
            self.assertIn("class_count", str(caught.exception))

    def test_two_classes_with_one_identity_are_refused(self):
        """Splitting a run across two otherwise identical classes expands to
        exactly what the merged class does, so the within-class maximal-run rule
        alone does not close the non-injectivity. Two classes with one identity
        would have been ONE class.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: checking canonical form only
        inside each `hour_runs` list."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        for table, expand in (("gaps", gap_ledger.expand_gaps),
                              ("domain_gap_outcomes",
                               gap_ledger.expand_domain_gap_outcomes)):
            split = json.loads(json.dumps(base))
            classes = split[table]["classes"]
            donor = None
            for class_ in classes:
                for index, (first, count) in enumerate(class_["hour_runs"]):
                    if count >= 2:
                        donor, donor_index = class_, index
                        break
                if donor:
                    break
            self.assertIsNotNone(donor, f"{table} has no multi-hour run to split")
            first, count = donor["hour_runs"][donor_index]
            twin = json.loads(json.dumps(donor))
            donor["hour_runs"][donor_index] = [first, 1]
            twin["hour_runs"] = [[first + 1, count - 1]]
            if "ordinals" in twin:
                twin["ordinals"] = twin["ordinals"][:1]
            classes.append(twin)
            split[table]["class_count"] = len(classes)
            with self.assertRaises(gap_ledger.GapLedgerError) as caught:
                expand({"gap_ledger": split})
            self.assertIn("identity", str(caught.exception))

    def test_a_hour_s_domains_must_be_aggregated_into_one_class(self):
        """The encoder aggregates every domain affected at one hour into ONE
        class, so `domains:[4,5] @100` and `[4] @100` + `[5] @100` expand
        identically. The class-identity rule cannot see it, because the domain
        list is part of that identity.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: keying the aggregation check on
        an identity that includes the domains."""
        for table, expand in (("gaps", gap_ledger.expand_gaps),
                              ("domain_gap_outcomes",
                               gap_ledger.expand_domain_gap_outcomes)):
            base_class = {
                "reason": "torn" if table == "domain_gap_outcomes" else "torn_sweep",
                "hour_runs": [[100, 1]],
            }
            if table == "gaps":
                base_class.update(grouping="per_domain", span_hours=1,
                                  witness={"kind": "torn_sweep",
                                           "tick_start": {"rel": 0},
                                           "tick_end": {"rel": 1}})
            else:
                base_class.update(part="state", queue_capacity=0, queue_depth=0,
                                  queue_dropped=0, range_start_rel=0, range_end_rel=0,
                                  tick_before={"rel": 0}, tick_after={"rel": 0},
                                  ordinals=[0])
            first = {**base_class, "domains": [4]}
            second = {**base_class, "domains": [5]}
            ledger = {
                "schema": gap_ledger.GAP_LEDGER_SCHEMA,
                "version": gap_ledger.GAP_LEDGER_VERSION,
                "gaps": {"row_count": 0, "class_count": 0, "classes": []},
                "domain_gap_outcomes": {"row_count": 0, "class_count": 0,
                                        "ordinal_encoding": "explicit", "classes": []},
            }
            ledger[table] = {"row_count": 2, "class_count": 2,
                             "classes": [first, second]}
            if table == "domain_gap_outcomes":
                ledger[table]["ordinal_encoding"] = "explicit"
            with self.assertRaises(gap_ledger.GapLedgerError) as caught:
                expand({"gap_ledger": ledger})
            self.assertIn("aggregates", str(caught.exception))

    def test_a_gap_class_carrying_an_ignored_branch_field_is_refused(self):
        """`repeat` is read only for a `shared` class and `ordinals` only by the
        outcome table, so a gap class carrying either expands to exactly what the
        class without it does -- the same non-injectivity the outcome branch
        fields carried.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: reading the field the grouping
        uses and saying nothing about the one it does not."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        for field, value in (("repeat", 1), ("ordinals", [99])):
            ledger = json.loads(json.dumps(base))
            target = next(class_ for class_ in ledger["gaps"]["classes"]
                          if class_["grouping"] == "per_domain")
            target[field] = value
            with self.assertRaises(gap_ledger.GapLedgerError):
                gap_ledger.expand_gaps({"gap_ledger": ledger})

    def test_a_class_may_not_name_more_hours_than_the_table_declares(self):
        """A run count is a `u32`, so `[[-2147483648, 4294967295]]` is inside the
        type and still 4.29 billion hours -- a hang rather than a verdict. A
        class cannot name more hours than the table has rows, and `row_count` is
        itself checked against the expansion.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: bounding the count by its type
        and nothing else, which is what round 10 did."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        ledger = json.loads(json.dumps(base))
        ledger["gaps"]["classes"][0]["hour_runs"] = [[-(2 ** 31), 2 ** 32 - 1]]
        with self.assertRaises(gap_ledger.GapLedgerError) as caught:
            gap_ledger.expand_gaps({"gap_ledger": ledger})
        self.assertIn("more than", str(caught.exception))

    def test_a_domain_list_out_of_order_is_refused(self):
        """`[4,5]` and `[5,4]` expand to the same rows after the reader's own
        sort, so admitting both lets two documents mean one thing.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: sorting the expansion without
        requiring the stored list to be ordered."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        for table, expand in (("gaps", gap_ledger.expand_gaps),
                              ("domain_gap_outcomes",
                               gap_ledger.expand_domain_gap_outcomes)):
            ledger = json.loads(json.dumps(base))
            target = next(class_ for class_ in ledger[table]["classes"]
                          if len(class_.get("domains") or []) >= 2)
            target["domains"] = list(reversed(target["domains"]))
            with self.assertRaises(gap_ledger.GapLedgerError) as caught:
                expand({"gap_ledger": ledger})
            self.assertIn("ascending", str(caught.exception))

    def test_a_run_count_outside_u32_is_refused_before_it_is_materialized(self):
        """The recorder's own reader narrows the count to `u32`. Without the
        bound this expander would try to materialize 4.29 billion hours -- a hang
        rather than a verdict, on a ledger Rust refuses outright.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: requiring only a positive
        integer, which Python's unbounded ints make the easy mistake."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        ledger = json.loads(json.dumps(base))
        ledger["gaps"]["classes"][0]["hour_runs"] = [[-(2 ** 31), 2 ** 32]]
        with self.assertRaises(gap_ledger.GapLedgerError) as caught:
            gap_ledger.expand_gaps({"gap_ledger": ledger})
        self.assertIn("u32", str(caught.exception))

    def test_a_table_with_no_classes_is_refused_not_read_as_zero_rows(self):
        """`table.get("classes") or []` read a missing list as an empty one, so a
        malformed table graded as a capture with no gaps -- and the recorder's
        own reader refuses it.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: defaulting a missing class list
        to empty."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        for table, expand in (("gaps", gap_ledger.expand_gaps),
                              ("domain_gap_outcomes",
                               gap_ledger.expand_domain_gap_outcomes)):
            ledger = json.loads(json.dumps(base))
            del ledger[table]["classes"]
            ledger[table]["row_count"] = 0
            with self.assertRaises(gap_ledger.GapLedgerError):
                expand({"gap_ledger": ledger})

    def test_an_hour_outside_the_i32_range_is_refused(self):
        """Hours are `i32` on the wire and in the recorder's own reader. A reader
        that admitted 2147483648 would attribute a fact to an hour that cannot
        exist and that the producer's decoder refuses -- two readers of one
        document disagreeing about which hour a gap belongs to.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: expanding a run without bounding
        it, which Python's unlimited integers make the easy mistake."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        for table, expand in (("gaps", gap_ledger.expand_gaps),
                              ("domain_gap_outcomes",
                               gap_ledger.expand_domain_gap_outcomes)):
            for run in ([[2 ** 31, 1]], [[2 ** 31 - 1, 4]], [[-(2 ** 31) - 1, 1]]):
                ledger = json.loads(json.dumps(base))
                ledger[table]["classes"][0]["hour_runs"] = run
                with self.assertRaises(gap_ledger.GapLedgerError):
                    expand({"gap_ledger": ledger})

    def test_a_row_count_that_is_not_an_integer_is_refused(self):
        """The count is the only thing that can catch a truncated class list, so
        a check that skips itself when the field is absent or a string is the
        check not running.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: comparing `row_count` only when
        it happens to be an int."""
        with open(VECTOR, encoding="utf-8") as handle:
            base = json.load(handle)["gap_ledger"]
        for bad in ("180", None, 137.0):
            ledger = json.loads(json.dumps(base))
            if bad is None:
                del ledger["gaps"]["row_count"]
            else:
                ledger["gaps"]["row_count"] = bad
            with self.assertRaises(gap_ledger.GapLedgerError):
                gap_ledger.expand_gaps({"gap_ledger": ledger})


if __name__ == "__main__":
    unittest.main()
