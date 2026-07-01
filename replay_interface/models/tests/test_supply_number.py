"""Tests for models/supply_number.py.

stdlib ``unittest`` (repo convention; runs with
``python -m unittest models.tests.test_supply_number``) and pytest-discoverable.
Mixes pure-unit tests on hand-built fixtures (no installed content) with optional
integration tests over the real save_oracle/output dir (skipped when absent).

The headline invariants under test mirror the task's VALIDATE clause:
  * cut-off provinces estimate LOW relative to supplied ones;
  * the badge NEVER lies: high confidence / exact=True only for solver/proxy;
  * the T0 solver seam, when present, overrides the estimate (provenance=solver);
  * the T1 division-proxy seam fills troop provinces (provenance=proxy);
  * no fabrication: missing data -> supply=None / value_kind=None, never a guess.
"""
import json
import os
import sys
import tempfile
import unittest

sys.path.insert(
    0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
)

from models import supply_number as sn  # noqa: E402

_REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
_OUT = os.path.join(os.path.dirname(_REPO), "extractor", "save_oracle", "output")


# ---------------------------------------------------------------------------
# Synthetic fixtures (no installed content needed): write a tiny oracle dir to disk.
# ---------------------------------------------------------------------------
def _write_fixture(tmp):
    """A 4-state / 4-province world:
       state 1 (GER): big hub + infra, owned & controlled  -> high estimate, med
       state 2 (GER): hub + infra but CONTROLLED by ENG     -> cut-off, low
       state 3 (SOV): no hub, no rail, no infra             -> unknown (None)
       state 4 (FIN): hub present but its node is DISRUPTED  -> reduced
    """
    state_detail = {
        "meta": {"save_date": "1942.1.1.12", "save_version": "test"},
        "states": {
            "1": {"owner": "GER", "controller": "GER",
                  "buildings": {"infrastructure": 5},
                  "province_buildings": {"supply_node": 3, "rail_way": 8}},
            "2": {"owner": "GER", "controller": "ENG",
                  "buildings": {"infrastructure": 5},
                  "province_buildings": {"supply_node": 3, "rail_way": 8}},
            "3": {"owner": "SOV", "controller": "SOV",
                  "buildings": {}, "province_buildings": {}},
            "4": {"owner": "FIN", "controller": "FIN",
                  "buildings": {"infrastructure": 4},
                  "province_buildings": {"supply_node": 2, "rail_way": 4}},
        },
    }
    supply = {
        "meta": {},
        "countries": {
            "FIN": {"disrupted_supply": [{"node_province": 41, "value": 0.5}]},
        },
    }
    # province -> state, and a node province (41) inside state 4.
    p2s = {"10": 1, "20": 2, "30": 3, "41": 4}
    province_control = {"10": "GER", "20": "ENG", "30": "SOV", "41": "FIN"}

    with open(os.path.join(tmp, "state_detail.json"), "w", encoding="utf-8") as fh:
        json.dump(state_detail, fh)
    with open(os.path.join(tmp, "supply.json"), "w", encoding="utf-8") as fh:
        json.dump(supply, fh)
    with open(os.path.join(tmp, "province_to_state.json"), "w", encoding="utf-8") as fh:
        json.dump(p2s, fh)
    with open(os.path.join(tmp, "province_control.json"), "w", encoding="utf-8") as fh:
        json.dump(province_control, fh)


def _rows_by_province(model):
    return {r["province"]: r for r in model["provinces"]}


def _states_by_id(model):
    return {r["state"]: r for r in model["states"]}


class SyntheticTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        _write_fixture(self.tmp)
        self.model = sn.compute_supply_number(self.tmp)
        self.rows = _rows_by_province(self.model)
        self.states = _states_by_id(self.model)

    def test_supplied_core_is_med_structural(self):
        r = self.rows[10]
        self.assertEqual(r["provenance"], "stored_logistics")
        self.assertEqual(r["confidence"], "med")
        self.assertEqual(r["value_kind"], "structural_potential")
        self.assertFalse(r["exact"])
        self.assertGreater(r["supply"], 15.0)  # hub3+infra5 -> sizeable

    def test_cutoff_is_low_and_reduced(self):
        core = self.rows[10]["supply"]
        cut = self.rows[20]["supply"]
        self.assertTrue(self.rows[20]["cutoff"])
        self.assertEqual(self.rows[20]["confidence"], "low")
        # same buildings as state 1, but cut-off -> CUTOFF_MULT lower
        self.assertLess(cut, core)
        self.assertAlmostEqual(cut, round(core * sn.CUTOFF_MULT, 2), delta=0.5)

    def test_no_logistics_state_is_unknown_not_zero(self):
        r = self.rows[30]
        self.assertIsNone(r["supply"])            # NOT a fabricated 0
        self.assertIsNone(r["value_kind"])
        self.assertEqual(r["confidence"], "low")
        # unknown != "none": "none" is a measured near-zero, "unknown" = no value.
        self.assertEqual(r["band"], "unknown")

    def test_disruption_reduces_hub_component(self):
        # state 4 hub node (prov 41) is 50% disrupted -> below an equivalent
        # undisrupted hub. Compare against the node component math directly.
        r = self.rows[41]
        self.assertIn("disrupted", (r["notes"] or ""))
        # undisrupted estimate for the same buildings:
        full, node_c = sn._structural_estimate(2, 4, 4)
        expected = round((full - node_c) + node_c * 0.5, 2)
        self.assertAlmostEqual(r["supply"], expected, delta=0.01)

    def test_badge_never_lies(self):
        for r in self.model["provinces"]:
            self.assertIn(r["provenance"], sn.PROVENANCE)
            self.assertIn(r["confidence"], sn.CONFIDENCE)
            self.assertIn(r["value_kind"], sn.VALUE_KIND)
            # high confidence only from solver/proxy
            if r["confidence"] == "high":
                self.assertIn(r["provenance"], ("solver", "proxy"))
            # exact only from solver
            if r["exact"]:
                self.assertEqual(r["provenance"], "solver")
            # cut-off never med/high
            if r["cutoff"]:
                self.assertEqual(r["confidence"], "low")

    def test_state_rollup_coverage_and_min_confidence(self):
        s1 = self.states[1]
        self.assertEqual(s1["min_confidence"], "med")
        self.assertEqual(s1["dominant_provenance"], "stored_logistics")
        self.assertEqual(s1["coverage"]["provinces"], 1)
        self.assertIsNotNone(s1["estimated_supply"])
        s3 = self.states[3]
        self.assertIsNone(s3["estimated_supply"])    # no numeric provinces
        self.assertEqual(s3["min_confidence"], "low")

    def test_global_rollup_shape(self):
        g = self.model["global_rollup"]
        self.assertEqual(g["provinces"], 4)
        self.assertEqual(g["states"], 4)
        self.assertIn("by_provenance", g)
        self.assertIn("by_confidence", g)
        self.assertEqual(g["cutoff_provinces"], 1)
        self.assertFalse(self.model["meta"]["solver_present"])


class CodexHardeningTests(unittest.TestCase):
    """Regression tests for the Codex impl-review findings (1-3) folded in."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        _write_fixture(self.tmp)
        self.model = sn.compute_supply_number(self.tmp)
        self.rows = _rows_by_province(self.model)

    def test_value_scope_present_and_correct(self):
        # T2 structural rows are per-STATE in scope; solver/proxy would be province.
        for r in self.model["provinces"]:
            self.assertIn("value_scope", r)
        self.assertEqual(self.rows[10]["value_scope"], "state")  # structural

    def test_med_requires_stored_logistics(self):
        # Codex impl-2: no row may be med unless it is hub-backed (stored_logistics).
        for r in self.model["provinces"]:
            if r["confidence"] == "med":
                self.assertEqual(r["provenance"], "stored_logistics")

    def test_disrupted_flip_drops_to_low(self):
        # state 4 hub (prov 41) is 50% disrupted; if that knocks the hub below
        # rail+infra it must NOT stay med.
        r = self.rows[41]
        if r["provenance"] == "estimate":
            self.assertEqual(r["confidence"], "low")

    def test_finite_guard(self):
        self.assertIsNone(sn._finite_nonneg(float("nan")))
        self.assertIsNone(sn._finite_nonneg(float("inf")))
        self.assertIsNone(sn._finite_nonneg(float("-inf")))
        self.assertIsNone(sn._finite_nonneg(-1.0))
        self.assertIsNone(sn._finite_nonneg("garbage"))
        self.assertIsNone(sn._finite_nonneg(None))
        self.assertEqual(sn._finite_nonneg("12.5"), 12.5)
        self.assertEqual(sn._finite_nonneg(0), 0.0)

    def test_nonfinite_solver_value_does_not_become_authoritative(self):
        # A NaN/inf solver value must NOT slip through as a trusted solver row; it
        # is dropped (belt+braces in _classify_province) and the province falls
        # back to the structural estimate. The KEY invariant: it is never tagged
        # provenance=solver / exact=True / value_kind=solver off a non-finite value.
        baseline = _rows_by_province(self.model)
        orig = sn._load_solver_values
        sn._load_solver_values = lambda output_dir: {10: float("nan"), 20: float("inf")}
        try:
            model = sn.compute_supply_number(self.tmp)
        finally:
            sn._load_solver_values = orig
        rows = _rows_by_province(model)
        for pid in (10, 20):
            self.assertNotEqual(rows[pid]["provenance"], "solver")
            self.assertFalse(rows[pid]["exact"])
            self.assertNotEqual(rows[pid]["value_kind"], "solver")
            # falls back to EXACTLY the structural estimate it would have had with
            # no solver at all (the bad value contributed nothing to the band).
            self.assertEqual(rows[pid]["supply"], baseline[pid]["supply"])
            self.assertEqual(rows[pid]["band"], baseline[pid]["band"])

    def test_nonfinite_proxy_value_falls_back(self):
        model = sn.compute_supply_number(
            self.tmp, proxy_by_province={10: {"current": float("nan"), "capacity": 5.0}})
        r = _rows_by_province(model)[10]
        self.assertNotEqual(r["provenance"], "proxy")   # NaN current -> not trusted
        self.assertEqual(r["value_kind"], "structural_potential")

    def test_cutoff_preserves_hub_dominance_ratio(self):
        # Codex impl-1: cut-off scales total AND node_c, so hub_dominated (hence
        # provenance) is cut-off-invariant; any cut-off stored_logistics row is low.
        for r in self.model["provinces"]:
            if r["cutoff"] and r["provenance"] == "stored_logistics":
                self.assertEqual(r["confidence"], "low")


class ProxySeamTests(unittest.TestCase):
    """T1: a live division proxy fills troop provinces with provenance=proxy."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        _write_fixture(self.tmp)

    def test_proxy_overrides_estimate(self):
        model = sn.compute_supply_number(
            self.tmp, proxy_by_province={10: {"current": 18.0, "capacity": 25.0}})
        r = _rows_by_province(model)[10]
        self.assertEqual(r["provenance"], "proxy")
        self.assertEqual(r["confidence"], "high")
        self.assertEqual(r["value_kind"], "division_proxy")
        self.assertFalse(r["exact"])               # proxy is NOT exact (Codex E)
        self.assertEqual(r["supply"], 18.0)
        self.assertEqual(r["capacity"], 25.0)

    def test_proxy_does_not_touch_non_troop_provinces(self):
        model = sn.compute_supply_number(
            self.tmp, proxy_by_province={10: {"current": 18.0, "capacity": 25.0}})
        # province 20 has no proxy -> still the structural estimate
        self.assertEqual(_rows_by_province(model)[20]["value_kind"],
                         "structural_potential")

    def test_accepts_enumerate_supply_full_shape(self):
        # enumerate_supply_full uses {"supply", ...} instead of {"current", ...}
        # (Codex impl2-4). Both must be accepted as T1 proxy.
        model = sn.compute_supply_number(
            self.tmp,
            proxy_by_province={10: {"supply": 14.0, "capacity": 20.0,
                                    "source": "division_proxy"}})
        r = _rows_by_province(model)[10]
        self.assertEqual(r["provenance"], "proxy")
        self.assertEqual(r["supply"], 14.0)
        self.assertEqual(r["capacity"], 20.0)
        # meta.proxy_provinces counts ACCEPTED rows, not raw input records.
        self.assertEqual(model["meta"]["proxy_provinces"], 1)

    def test_proxy_provinces_meta_counts_accepted_not_input(self):
        # one good proxy + one with an unusable value -> input=2, accepted=1.
        model = sn.compute_supply_number(
            self.tmp,
            proxy_by_province={10: {"current": 9.0},
                               20: {"current": float("nan")}})
        self.assertEqual(model["meta"]["proxy_provinces_in"], 2)
        self.assertEqual(model["meta"]["proxy_provinces"], 1)


class SolverSeamTests(unittest.TestCase):
    """T0: when the future Ghidra solver module is present, it overrides all."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        _write_fixture(self.tmp)

    def test_solver_overrides_when_present(self):
        # Simulate the future solver by monkeypatching the private loader, which
        # is the single seam that knows the solver's call shape.
        orig = sn._load_solver_values
        sn._load_solver_values = lambda output_dir: {10: 42.0, 20: 7.5}
        try:
            model = sn.compute_supply_number(self.tmp)
        finally:
            sn._load_solver_values = orig
        rows = _rows_by_province(model)
        r = rows[10]
        self.assertEqual(r["provenance"], "solver")
        self.assertEqual(r["confidence"], "high")
        self.assertTrue(r["exact"])
        self.assertEqual(r["value_kind"], "solver")
        self.assertEqual(r["supply"], 42.0)
        # even the cut-off province takes the solver value verbatim (authoritative)
        self.assertEqual(rows[20]["supply"], 7.5)
        self.assertEqual(rows[20]["provenance"], "solver")
        self.assertTrue(model["meta"]["solver_present"])

    def test_solver_loader_accepts_module_function_shape(self):
        """_load_solver_values accepts supply_solver.province_supply(...)."""
        class _Fake:
            @staticmethod
            def province_supply(output_dir):
                return {1: 10, "2": "20.5", "bad": "x"}
        orig = sn._supply_solver
        sn._supply_solver = _Fake
        try:
            vals = sn._load_solver_values(self.tmp)
        finally:
            sn._supply_solver = orig
        self.assertEqual(vals, {1: 10.0, 2: 20.5})   # coerced; bad rows dropped

    def test_no_solver_module_yields_none(self):
        orig = sn._supply_solver
        sn._supply_solver = None
        try:
            self.assertIsNone(sn._load_solver_values(self.tmp))
        finally:
            sn._supply_solver = orig


class HelperTests(unittest.TestCase):
    def test_band_edges(self):
        self.assertEqual(sn._band(None), "unknown")   # not "none" (Codex impl2-3)
        self.assertEqual(sn._band(0.0), "none")
        self.assertEqual(sn._band(3.0), "low")
        self.assertEqual(sn._band(10.0), "med")
        self.assertEqual(sn._band(40.0), "high")

    def test_structural_estimate_clamps(self):
        total, node_c = sn._structural_estimate(100, 100, 100)
        self.assertEqual(total, sn.SUPPLY_CLAMP)
        self.assertLessEqual(node_c, total)

    def test_dominant_provenance_not_best_present(self):
        # 1 proxy + 3 estimate -> dominant must be estimate, NOT proxy (Codex F)
        self.assertEqual(
            sn._dominant_provenance(["proxy", "estimate", "estimate", "estimate"]),
            "estimate")

    def test_missing_required_input_raises(self):
        tmp = tempfile.mkdtemp()  # empty dir, no state_detail.json
        with self.assertRaises(FileNotFoundError):
            sn.compute_supply_number(tmp)


@unittest.skipUnless(
    os.path.isdir(_OUT) and os.path.exists(os.path.join(_OUT, "state_detail.json")),
    "save_oracle/output not present")
class IntegrationTests(unittest.TestCase):
    """Run against the real 1.19 melt output; assert plausibility, not exactness."""

    @classmethod
    def setUpClass(cls):
        cls.model = sn.compute_supply_number(_OUT)

    def test_cutoff_provinces_estimate_lower(self):
        cut = [r["supply"] for r in self.model["provinces"]
               if r["cutoff"] and r["supply"] is not None]
        sup = [r["supply"] for r in self.model["provinces"]
               if not r["cutoff"] and r["supply"] is not None]
        self.assertTrue(cut and sup)
        self.assertLess(sum(cut) / len(cut), sum(sup) / len(sup))

    def test_no_badge_lies_at_scale(self):
        for r in self.model["provinces"]:
            if r["confidence"] == "high":
                self.assertIn(r["provenance"], ("solver", "proxy"))
            if r["exact"]:
                self.assertEqual(r["provenance"], "solver")
            if r["cutoff"]:
                self.assertEqual(r["confidence"], "low")

    def test_covers_full_province_map(self):
        # every province in the join map appears in the output
        self.assertGreater(self.model["global_rollup"]["provinces"], 9000)
        self.assertFalse(self.model["meta"]["solver_present"])  # offline


if __name__ == "__main__":
    unittest.main()
