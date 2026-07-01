"""Tests for replay/store.py â€” ingestion + the on-disk snapshot store.

Ingests the real save_oracle/output when present (read-only) and writes the store
ONLY to a temp scratch dir (never into the repo). Synthetic fallback otherwise.
"""
import json
import os
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from replay import schema, store  # noqa: E402

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ORACLE_OUT = os.path.join(os.path.dirname(REPO), "extractor", "save_oracle", "output")
HAVE_ORACLE = os.path.isfile(os.path.join(ORACLE_OUT, "country_snapshot.json"))


def _scratch_root():
    base = os.path.join(tempfile.gettempdir(), "re_scratch")
    os.makedirs(base, exist_ok=True)
    return tempfile.mkdtemp(prefix="replay_store_", dir=base)


def _dump(root, name, obj):
    with open(os.path.join(root, name), "w", encoding="utf-8") as fh:
        json.dump(obj, fh)


def _make_oracle_dir(root):
    """Write a tiny synthetic oracle output so the store test runs without real data."""
    os.makedirs(root, exist_ok=True)
    _dump(root, "meta.json",
          {"source_filename": "/secret/path/SYN_melted.txt", "save_date": "1937.6.1.12",
           "save_version": "syn"})
    _dump(root, "country_snapshot.json",
          {"GER": {"stability": 0.6, "war_support": 0.5, "political_power": 100.0,
                   "command_power": 30.0, "ruling_party": "fascism", "capital": 64,
                   "major": True, "num_ships": 3, "is_human": True,
                   "factories": {"civ": 20, "mil": 30, "dockyard": 5}}})
    _dump(root, "state_ownership.json",
          {"1": {"owner": "GER", "controller": "GER"},
           "2": {"owner": "POL", "controller": "GER"}})
    _dump(root, "state_control.json",
          {"1": {"owner": "GER", "controller": "GER"},
           "2": {"owner": "POL", "controller": "GER"}})
    _dump(root, "province_control.json", {"10": "GER", "11": "GER"})
    _dump(root, "divisions.json",
          {"meta": {"source_filename": "x"},
           "divisions": [{"div_id": 5, "owner": "GER", "location_province": 10,
                          "template_id": 1, "template_name": "Inf"}]})
    return root


class TestIngestSynthetic(unittest.TestCase):
    def setUp(self):
        self.scratch = _scratch_root()
        self.oracle = _make_oracle_dir(os.path.join(self.scratch, "oracle"))

    def tearDown(self):
        shutil.rmtree(self.scratch, ignore_errors=True)

    def test_build_and_validate(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertEqual(snap["date"], "1937.6.1.12")
        self.assertEqual(snap["countries"]["GER"]["politics"]["stability"], 0.6)
        self.assertEqual(snap["countries"]["GER"]["military"]["division_count"], 1)

    def test_source_path_is_redacted_to_basename(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        sf = snap["provenance"]["save"]["source_filename"]
        self.assertEqual(sf, "SYN_melted.txt")
        self.assertNotIn("/", sf)
        self.assertNotIn("secret", sf)

    def test_memory_provenance_null_no_fabrication(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn",
                                                memory_dir=os.path.join(os.path.dirname(REPO), "extractor", "memory_reader"))
        self.assertIsNone(snap["provenance"]["memory"])
        # memory-sourced fields that the save doesn't carry stay null
        self.assertIsNone(snap["divisions"][0]["organisation"])

    def test_store_write_and_reload(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        st = store.SnapshotStore(os.path.join(self.scratch, "store"))
        path = st.write_snapshot(snap)
        self.assertTrue(os.path.isfile(path))
        listing = st.list_snapshots("syn")
        self.assertEqual(len(listing), 1)
        reloaded = st.load_snapshot("syn", snap["hour_index"])
        self.assertEqual(reloaded["date"], snap["date"])

    def test_store_refuses_invalid_snapshot(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        snap["hour_index"] = 1  # corrupt
        st = store.SnapshotStore(os.path.join(self.scratch, "store"))
        with self.assertRaises(ValueError):
            st.write_snapshot(snap)

    def test_controller_override_from_control_file(self):
        # state 2 owner POL but controller GER (occupied)
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        self.assertEqual(snap["states"]["2"]["owner"], "POL")
        self.assertEqual(snap["states"]["2"]["controller"], "GER")

    def test_store_writes_collector_map_control_snapshot_without_save(self):
        snap = schema.empty_snapshot("collector-syn", "1937.6.1.13")
        snap["provenance"]["collector"] = {
            "captured": True,
            "backend": "in_process_collector",
            "collector_version": "0.1.0",
            "quality_status": "complete",
            "domains": ["clock", "state_ownership", "state_control",
                        "province_control"],
        }
        snap["states"]["1"] = schema.empty_state()
        snap["states"]["1"]["owner"] = "GER"
        snap["states"]["1"]["controller"] = "GER"
        snap["provinces"]["10"] = schema.empty_province()
        snap["provinces"]["10"]["controller"] = "GER"
        snap["provinces"]["10"]["state_id"] = 1
        snap["map"]["owner_by_state"] = {"1": "GER"}
        snap["map"]["control_by_province"] = {"10": "GER"}

        self.assertEqual(schema.validate_snapshot(snap), [])
        st = store.SnapshotStore(os.path.join(self.scratch, "store"))
        st.write_snapshot(snap)
        reloaded = st.load_snapshot("collector-syn", snap["hour_index"])

        self.assertIsNone(reloaded["provenance"]["save"])
        self.assertIsNone(reloaded["provenance"]["memory"])
        self.assertEqual(reloaded["provenance"]["collector"]["domains"],
                         ["clock", "state_ownership", "state_control",
                          "province_control"])
        self.assertEqual(reloaded["states"]["1"]["controller"], "GER")
        self.assertEqual(reloaded["provinces"]["10"]["controller"], "GER")


@unittest.skipUnless(HAVE_ORACLE, "real save_oracle/output not present")
class TestIngestRealOracle(unittest.TestCase):
    def setUp(self):
        self.scratch = _scratch_root()

    def tearDown(self):
        shutil.rmtree(self.scratch, ignore_errors=True)

    def test_full_ingest_validates(self):
        snap = store.build_snapshot_from_oracle(ORACLE_OUT, "real")
        self.assertEqual(schema.validate_snapshot(snap), [])
        self.assertGreater(len(snap["countries"]), 100)
        self.assertEqual(len(snap["states"]), 1081)
        self.assertGreater(len(snap["provinces"]), 10000)
        self.assertGreater(len(snap["divisions"]), 1000)

    def test_no_personal_path_anywhere_in_snapshot(self):
        snap = store.build_snapshot_from_oracle(ORACLE_OUT, "real")
        blob = json.dumps(snap)
        for needle in ("redacted", "OneDrive", "C:\\Users", "re_scratch", "Downloads"):
            self.assertNotIn(needle, blob, needle)

    def test_write_to_scratch_then_diff_self_is_empty(self):
        snap = store.build_snapshot_from_oracle(ORACLE_OUT, "real")
        st = store.SnapshotStore(os.path.join(self.scratch, "store"))
        st.write_snapshot(snap)
        from replay import diff_engine
        # diffing a snapshot against an identical copy -> no events
        self.assertEqual(diff_engine.diff_snapshots(snap, json.loads(json.dumps(snap))), [])


if __name__ == "__main__":
    unittest.main()
