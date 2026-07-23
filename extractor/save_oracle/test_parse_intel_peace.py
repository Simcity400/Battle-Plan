"""Focused fail-closed tests for the peace-conference save-oracle boundary.

No retained save contains a nonempty active conference.  These fixtures test
only the proven parser contract: an empty block is inactive, while any scalar
or nested value makes the opaque save block present.  They deliberately do not
invent participant, score, action, or picker semantics.
"""

from __future__ import annotations

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import parse_intel


SYN_PEACE_EMPTY = "date=1944.2.11.18 player=D04 peace_conference={ }"
SYN_PEACE_PRESENT = (
    "peace_conference={ fixture_scalar=fixture_value "
    "fixture_block={ value=fixture_value } }"
)


class ParseIntelPeaceConferenceTest(unittest.TestCase):
    def test_empty_block_is_inactive(self):
        _countries, peace, meta = parse_intel.parse_intel_text(
            SYN_PEACE_EMPTY
        )
        self.assertEqual(meta["date"], "1944.2.11.18")
        self.assertEqual(peace, {"active": False})

    def test_scalar_marks_block_present_and_is_preserved_opaquely(self):
        _countries, peace, _meta = parse_intel.parse_intel_text(
            SYN_PEACE_PRESENT
        )
        self.assertEqual(
            peace, {"active": True, "fixture_scalar": "fixture_value"}
        )

    def test_nested_value_marks_block_present_without_fabricated_projection(self):
        _countries, peace, _meta = parse_intel.parse_intel_text(
            SYN_PEACE_PRESENT
        )
        self.assertEqual(
            peace, {"active": True, "fixture_scalar": "fixture_value"}
        )
        self.assertNotIn("fixture_block", peace)


if __name__ == "__main__":
    unittest.main()
