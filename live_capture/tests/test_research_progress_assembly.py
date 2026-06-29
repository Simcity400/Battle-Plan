from __future__ import annotations

import os
import sys
import unittest
from unittest import mock

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir, os.pardir))
if _REPO not in sys.path:
    sys.path.insert(0, _REPO)

from replay import schema  # noqa: E402
from live_capture import assembler as asm  # noqa: E402


class TestResearchProgressAssembly(unittest.TestCase):
    def test_attach_derived_models_adds_live_fraction_additively(self):
        snap = schema.empty_snapshot("g", "1936.1.1.12")
        country = schema.empty_country()
        country["research"]["in_progress_live"] = [{
            "slot_index": 0,
            "tech_handle_raw": 0x1234,
            "tech_def_index_raw": 0x1234,
            "tech_name": "basic_machine_tools",
            "progress_raw": 82500,
            "progress_units": 1.0,
            "live_progress_raw": 8250000,
            "live_progress_units": 82.5,
        }]
        snap["countries"]["GER"] = country

        tech_defs = {
            "basic_machine_tools": {
                "research_cost": 1.5,
                "start_year": 1936,
                "categories": ["industry"],
            },
        }
        with mock.patch(
                "models.research_progress.load_static_tech_defs",
                return_value=tech_defs):
            asm.attach_derived_models(snap, output_dir=None)

        slot = snap["countries"]["GER"]["research"]["in_progress_live"][0]
        self.assertEqual(slot["tech_handle_raw"], 0x1234)
        self.assertEqual(slot["tech_def_index_raw"], 0x1234)
        self.assertEqual(slot["progress_raw"], 82500)
        self.assertEqual(slot["progress_units"], 1.0)
        self.assertEqual(slot["live_progress_units"], 82.5)
        self.assertEqual(slot["progress_pct"], 0.5)
        self.assertEqual(slot["required_points_base"], 165.0)
        self.assertEqual(slot["eta_unavailable_reason"],
                         "missing_live_points_factor")
        self.assertEqual(schema.validate_snapshot(snap), [])


if __name__ == "__main__":
    unittest.main()
