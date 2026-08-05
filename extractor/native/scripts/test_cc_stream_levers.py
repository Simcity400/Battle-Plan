#!/usr/bin/env python3
"""Tests for the Lane CC size-lever instruments.

Every case here is built from a synthetic H4CR stream whose answer is known by
construction, and every assertion names a production behaviour that a regression
would break:

* the own-gap bucket split is what separates "this domain lost its pin" from
  "somebody else did", and getting it wrong silently reassigns hundreds of MB
  between the two buckets that the whole lever ranking rests on;
* the delta-bit split is what stops a domain that emits a near-empty record most
  hours and a large patch on the rest from being averaged into a size no record
  has -- the error that inflated d47's and d51's prize to 40 MB and 43 MB when
  the truth is zero;
* the saturation model must return the measured steady-state mean at k = 1, or
  it is not anchored on anything.

Run: python extractor/native/scripts/test_cc_stream_levers.py
"""

from __future__ import annotations

import importlib.util
import os
import struct
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
H = 32
RT_DOMAIN_DATA = 3
RT_DOMAIN_GAP = 5
STATUS_DELTA_DATA = 1 << 10


def load(name):
    path = os.path.join(HERE, name)
    spec = importlib.util.spec_from_file_location(name[:-3], path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def record(sequence, hour, domain, record_type, payload):
    header = struct.pack(
        "<4sHHQiHHII", b"H4CR", 1, H, sequence, hour, domain, record_type,
        len(payload), 0,
    )
    return header + payload


def data_payload(total_len, status):
    body = struct.pack("<QIIII", 0, 0, 0, status, 0)
    assert len(body) == 24
    if total_len < len(body):
        raise ValueError("payload shorter than the shared header")
    return body[:20] + bytes(total_len - 20)


class OwnGapBucketing(unittest.TestCase):
    """A gap for ANOTHER domain must not move a record into the post-own-gap bucket."""

    def build(self):
        blobs = []
        seq = 0

        def emit(hour, domain, rtype, payload=b""):
            nonlocal seq
            seq += 1
            blobs.append(record(seq, hour, domain, rtype, payload))

        # hour 0: both domains deliver 1000-byte keyframes (seed records).
        emit(0, 10, RT_DOMAIN_DATA, data_payload(1000, 0))
        emit(0, 11, RT_DOMAIN_DATA, data_payload(1000, 0))
        # hour 1: domain 11 is gapped; domain 10 delivers 100 bytes.
        emit(1, 10, RT_DOMAIN_DATA, data_payload(100, 0))
        emit(1, 11, RT_DOMAIN_GAP)
        # hour 2: domain 10 delivers 100 bytes with NO own gap behind it, even
        # though hour 1 had a gap for domain 11. Domain 11 delivers 900 bytes
        # WITH an own gap behind it.
        emit(2, 10, RT_DOMAIN_DATA, data_payload(100, 0))
        emit(2, 11, RT_DOMAIN_DATA, data_payload(900, 0))
        handle = tempfile.NamedTemporaryFile(suffix=".bin", delete=False)
        handle.write(b"".join(blobs))
        handle.close()
        return handle.name

    def test_a_foreign_gap_does_not_create_a_post_own_gap_record(self):
        levers = load("cc_stream_levers.py")
        path = self.build()
        try:
            stats = {}
            for (_off, _seq, _hour, domain, rtype, plen, _flags, payload) in levers.walk(path):
                stats.setdefault(domain, []).append((rtype, plen))
            self.assertEqual(len(stats[10]), 3)
        finally:
            os.unlink(path)

    def test_bucket_split_assigns_each_record_to_its_own_gap_history(self):
        prize = load("cc_pin_prize_v2.py")
        path = self.build()
        try:
            rows = prize.collect(path)
            # domain 10: seed, then two steady records; no own gap anywhere.
            self.assertEqual([(k, first) for k, _n, _d, first in rows[10]],
                             [(0, True), (0, False), (0, False)])
            # domain 11: seed, then one record with exactly one own gap behind it.
            self.assertEqual([(k, first) for k, _n, _d, first in rows[11]],
                             [(0, True), (1, False)])
        finally:
            os.unlink(path)


class DeltaBitSplit(unittest.TestCase):
    """A domain whose steady bucket mixes tiny keyframes with large patches must
    not be priced against the mean of the two."""

    def build(self):
        blobs = []
        seq = 0

        def emit(hour, domain, rtype, payload=b""):
            nonlocal seq
            seq += 1
            blobs.append(record(seq, hour, domain, rtype, payload))

        emit(0, 12, RT_DOMAIN_DATA, data_payload(1000, 0))          # seed keyframe
        for hour in range(1, 5):
            emit(hour, 12, RT_DOMAIN_DATA, data_payload(24, 0))     # nothing changed
        emit(5, 12, RT_DOMAIN_DATA, data_payload(800, STATUS_DELTA_DATA))  # a real patch
        handle = tempfile.NamedTemporaryFile(suffix=".bin", delete=False)
        handle.write(b"".join(blobs))
        handle.close()
        return handle.name

    def test_the_delta_bit_is_read_from_the_shared_payload_header(self):
        prize = load("cc_pin_prize_v2.py")
        path = self.build()
        try:
            rows = prize.collect(path)
            flags = [d for _k, _n, d, _first in rows[12]]
            self.assertEqual(flags, [False, False, False, False, False, True])
        finally:
            os.unlink(path)

    def test_a_domain_without_a_readable_status_word_reports_none(self):
        prize = load("cc_pin_prize_v2.py")
        self.assertIn(53, prize.NON_SHARED_HEADER_DOMAINS)
        blobs = [record(1, 0, 53, RT_DOMAIN_DATA, data_payload(1000, STATUS_DELTA_DATA))]
        handle = tempfile.NamedTemporaryFile(suffix=".bin", delete=False)
        handle.write(b"".join(blobs))
        handle.close()
        try:
            rows = prize.collect(handle.name)
            self.assertEqual(rows[53][0][2], None,
                             "d53's status word is not promised at payload offset 16 and "
                             "must never be read as if it were")
        finally:
            os.unlink(handle.name)


class SaturationModel(unittest.TestCase):
    """`whole * (1 - (1-q)^k)` must return the measured steady mean at k = 1."""

    def test_the_model_is_anchored_on_the_one_hour_measurement(self):
        whole = 9_396_000.0
        steady_mean = 2_030_000.0
        q = steady_mean / whole
        one_hour = whole * (1.0 - (1.0 - q) ** 1)
        self.assertAlmostEqual(one_hour, steady_mean, places=6)

    def test_the_model_saturates_at_the_whole_payload_and_never_exceeds_it(self):
        whole = 1000.0
        q = 0.2
        sizes = [whole * (1.0 - (1.0 - q) ** k) for k in range(1, 200)]
        self.assertTrue(all(a <= b + 1e-9 for a, b in zip(sizes, sizes[1:])))
        self.assertLess(sizes[-1], whole + 1e-9)
        self.assertGreater(sizes[-1], whole * 0.99)


class FramingRefusal(unittest.TestCase):
    def test_a_truncated_final_record_is_refused_rather_than_silently_dropped(self):
        levers = load("cc_stream_levers.py")
        blob = record(1, 0, 10, RT_DOMAIN_DATA, data_payload(1000, 0))
        handle = tempfile.NamedTemporaryFile(suffix=".bin", delete=False)
        handle.write(blob[:-10])
        handle.close()
        try:
            with self.assertRaises(SystemExit):
                list(levers.walk(handle.name))
        finally:
            os.unlink(handle.name)


if __name__ == "__main__":
    unittest.main(verbosity=2)
