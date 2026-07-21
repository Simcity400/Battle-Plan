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

    def test_save_combat_handles_are_typed_and_losslessly_retained(self):
        _dump(
            self.oracle,
            "oob.json",
            {
                "air_wings": [
                    {"id": "1001:69", "owner": "GER"},
                    {"id": "2002:69", "owner": "ENG"},
                ]
            },
        )
        _dump(
            self.oracle,
            "combats.json",
            {
                "combats": [
                    {
                        "combat_id": [77, 12],
                        "kind": "air",
                        "location": 42,
                        "day": 5,
                        "duration": 2,
                        "attacker": {"units": [[1001, 69]], "progress": 1.25},
                        "defender": {"units": [[2002, 69]], "progress": 2.5},
                    }
                ]
            },
        )
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        combat = snap["combats"][0]
        self.assertEqual(combat["combat_id"], 77)
        self.assertEqual(combat["attackers"], [1001])
        self.assertEqual(combat["defenders"], [2002])
        self.assertEqual(combat["modifiers"]["save_combat_handle"], [77, 12])
        self.assertEqual(
            combat["modifiers"]["save_attacker_unit_handles"], [[1001, 69]]
        )
        self.assertEqual(combat["modifiers"]["save_attacker_owners"], ["GER"])
        self.assertEqual(combat["modifiers"]["save_defender_owners"], ["ENG"])
        self.assertTrue(combat["modifiers"]["save_air_owner_join_complete"])

    def test_save_air_owner_join_fails_closed_on_ambiguous_or_missing_wings(self):
        _dump(
            self.oracle,
            "oob.json",
            {
                "air_wings": [
                    {"id": "1001:69", "owner": "GER"},
                    {"id": "1001:69", "owner": "ITA"},
                    {"id": "2002:70", "owner": "ENG"},
                ]
            },
        )
        _dump(
            self.oracle,
            "combats.json",
            {
                "combats": [
                    {
                        "combat_id": [77, 12],
                        "kind": "air",
                        "attacker": {"units": [[1001, 69]]},
                        # A bare-id fallback would incorrectly resolve this
                        # complete handle to the OOB wing with type 70.
                        "defender": {"units": [[2002, 69]]},
                    }
                ]
            },
        )
        combat = store.build_snapshot_from_oracle(self.oracle, "syn")["combats"][0]
        self.assertEqual(combat["modifiers"]["save_attacker_owners"], [])
        self.assertEqual(combat["modifiers"]["save_defender_owners"], [])
        self.assertFalse(combat["modifiers"]["save_air_owner_join_complete"])

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

    def test_atomic_write_failure_keeps_old_file_and_no_temp(self):
        # a failed dump must leave the previous content intact and remove the
        # temp file, never a truncated destination (crash-mid-write safety).
        path = os.path.join(self.scratch, "atomic.json")
        store._atomic_write_json(path, {"ok": 1})
        with self.assertRaises(TypeError):
            store._atomic_write_json(path, {"bad": object()})
        with open(path, "r", encoding="utf-8") as fh:
            self.assertEqual(json.load(fh), {"ok": 1})
        self.assertEqual(
            [n for n in os.listdir(self.scratch) if ".tmp." in n], [])

    def test_store_write_leaves_no_temp_files(self):
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        st = store.SnapshotStore(os.path.join(self.scratch, "store"))
        st.write_snapshot(snap)
        st.write_timeline("syn", [])
        gdir = st.game_dir("syn")
        self.assertEqual([n for n in os.listdir(gdir) if ".tmp." in n], [])

    def test_war_and_faction_key_collisions_are_preserved(self):
        # two distinct wars with the same pair+start_date and factions with
        # duplicate/missing ids must all survive under disambiguated keys.
        _dump(self.oracle, "diplomacy.json", {
            "wars": [
                {"a": "GER", "b": "POL", "start_date": "1939.9.1.4"},
                {"a": "GER", "b": "POL", "start_date": "1939.9.1.4",
                 "a_casualties": 5},
            ],
            "factions": [
                {"id": 1, "members": ["GER"], "template": "axis"},
                {"id": 1, "members": ["ITA"], "template": "axis2"},
                {"members": ["JAP"], "template": "co_prosperity"},
                {"members": ["SOV"], "template": "comintern"},
            ],
        })
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        wars = snap["diplomacy_world"]["wars"]
        self.assertIn("GER-POL@1939.9.1.4", wars)
        self.assertIn("GER-POL@1939.9.1.4#2", wars)
        self.assertEqual(wars["GER-POL@1939.9.1.4#2"]["a_casualties"], 5)
        facs = snap["diplomacy_world"]["factions"]
        self.assertEqual(set(facs), {"1", "1#2", "noid#1", "noid#2"})
        self.assertIs(facs["noid#1"].get("id_missing"), True)
        self.assertIs(facs["noid#2"].get("id_missing"), True)
        self.assertNotIn("id_missing", facs["1"])
        self.assertEqual(schema.validate_snapshot(snap), [])

    def test_abilities_rejects_non_tag_keys(self):
        # only real TAGs (schema.valid_tag) may create country entries.
        _dump(self.oracle, "abilities.json", {
            "aces": {"GER": {"aces": [{"id": 1}]}, "abc": {"aces": []},
                     "?": {"aces": []}},
            "commander_abilities": {"meta": []},
            "officer_corps": {},
        })
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        self.assertEqual(snap["countries"]["GER"]["abilities"]["aces"],
                         [{"id": 1}])
        for junk in ("abc", "meta", "?"):
            self.assertNotIn(junk, snap["countries"])

    def test_market_extra_contract_id_normalized(self):
        # price-band records must share the 'id:type' key space with
        # diplomacy_world.trade_contracts (join-key parity).
        _dump(self.oracle, "market_extra.json", {
            "contracts": [{"contract_id": [465, 81], "seller": "GER",
                           "buyer": "POL", "price_levels": [1, 2]}],
        })
        snap = store.build_snapshot_from_oracle(self.oracle, "syn")
        pl = snap["diplomacy_world"]["market_price_levels"]
        self.assertEqual(pl[0]["contract_id"], "465:81")


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
