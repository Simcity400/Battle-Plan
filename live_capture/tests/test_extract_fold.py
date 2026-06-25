"""Tests for folding the Tier-1 EXTRACT save domains through the ASSEMBLER's
``fold_save_bundle`` backstop path (live snapshot + melted-save oracle dir).

All MOCK/fixture-based -- NO live memory, NO live game. Covers:
  * fold_save_bundle carries the new save-only domains (strategic_air / world /
    world_events / raids / strategic_operatives) into the live snapshot, plus the
    per-country extra / human_name / economy.logistics, when LIVE left them empty,
  * provenance honesty: the EXTRACT domains land under provenance.save with
    fills_canonical_domains=True and a `domains` list -- the memory channel is NOT
    touched and the save data is NOT mislabeled as a memory domain,
  * a genuine 0/0.0 from the save survives the fold (no-fabrication),
  * NO user path leaks into the folded snapshot.
"""
from __future__ import annotations

import json
import os
import shutil
import sys
import tempfile
import unittest

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir, os.pardir))
if _REPO not in sys.path:
    sys.path.insert(0, _REPO)

from replay import schema  # noqa: E402
from live_capture import assembler as asm  # noqa: E402

ORACLE_OUT = os.path.join(_REPO, "save_oracle", "output")
HAVE_FIXTURES = os.path.isfile(os.path.join(ORACLE_OUT, "world.json"))


def _scratch_root():
    base = os.path.join(tempfile.gettempdir(), "re_scratch")
    os.makedirs(base, exist_ok=True)
    return tempfile.mkdtemp(prefix="integ_extract_fold_", dir=base)


def _dump(root, name, obj):
    with open(os.path.join(root, name), "w", encoding="utf-8") as fh:
        json.dump(obj, fh)


def _make_oracle_dir(root):
    """A small synthetic oracle output carrying the EXTRACT domains + a leak path."""
    os.makedirs(root, exist_ok=True)
    _dump(root, "meta.json",
          {"source_filename": "/home/u/secret/SYN_melted.txt",
           "source_path_redacted": "/home/u/secret/SYN_melted.txt",
           "save_date": "1937.6.1.12", "save_version": "syn"})
    _dump(root, "country_snapshot.json", {"GER": {"stability": 0.75}})
    _dump(root, "supply.json", {
        "meta": {},
        "countries": {"GER": {"priority": 2, "node_count": 3, "truck_count": 0}}})
    _dump(root, "strategic_air.json", {
        "meta": {},
        "air_bases": [{"id": 1, "type": 65, "state": 1, "capacity": 200,
                       "countries": [{"country": "GER", "capacity_penalty": 1.0}]}],
        "indices": {"air_theatre_index": 0, "air_group_index": 0},
        "per_country_region_blocks": {"GER": 4}})
    _dump(root, "world.json", {
        "meta": {},
        "scenario": {"start_date": "1936.1.1.12", "ideology": "communism"},
        "variables": {"axis_score": 2.0}, "flags": {},
        "tech_sharing_groups": [], "faction_system": [], "power_balances": [],
        "saved_event_targets": []})
    _dump(root, "events.json", {
        "meta": {}, "fired_event_names": ["e1"], "pending_events": [],
        "threat": [], "sunk_convoys_history": []})
    _dump(root, "country_extra.json", {
        "meta": {},
        "countries": {"GER": {"nukes_total": 0, "ace_count": 1,
                              "aces": [{"name": "A", "surname": "B",
                                        "callsign": "X"}]}}})
    _dump(root, "players.json", {
        "meta": {}, "players": {"GER": {"user": "WooooWoooo", "id": 1}}})
    return root


class TestExtractFold(unittest.TestCase):
    def setUp(self):
        self.scratch = _scratch_root()
        self.oracle = _make_oracle_dir(os.path.join(self.scratch, "oracle"))

    def tearDown(self):
        shutil.rmtree(self.scratch, ignore_errors=True)

    def _folded(self):
        # a pure live skeleton (no captured domains), then fold the save under it
        snap = asm.assemble_snapshot({}, game_id="g", date="1937.6.1.12")
        return asm.fold_save_bundle(snap, self.oracle, game_id="g")

    def test_folded_validates(self):
        self.assertEqual(schema.validate_snapshot(self._folded()), [])

    def test_world_domains_folded(self):
        f = self._folded()
        self.assertEqual(f["world"]["scenario"]["start_date"], "1936.1.1.12")
        self.assertEqual(len(f["strategic_air"]["air_bases"]), 1)
        self.assertEqual(f["world_events"]["fired_events"], ["e1"])

    def test_per_country_extract_folded(self):
        f = self._folded()
        ger = f["countries"]["GER"]
        self.assertEqual(ger["human_name"], "WooooWoooo")
        self.assertEqual(ger["extra"]["ace_count"], 1)
        # genuine zero survives the fold
        self.assertEqual(ger["extra"]["nukes_total"], 0)
        self.assertEqual(ger["economy"]["logistics"]["truck_count"], 0)

    def test_provenance_save_not_memory(self):
        f = self._folded()
        prov = f["provenance"]
        sv = prov["save"]
        self.assertTrue(sv["fills_canonical_domains"])
        self.assertFalse(sv["is_live"])
        for d in ("supply_logistics", "strategic_air", "world", "world_events",
                  "country_extra", "players"):
            self.assertIn(d, sv["domains"], d)
        # memory channel untouched: no EXTRACT domain mislabeled as memory
        mem = prov.get("memory") or {}
        for d in ("strategic_air", "world", "country_extra", "players"):
            self.assertNotIn(d, mem.get("domains") or [])

    def test_no_personal_path_leak(self):
        blob = json.dumps(self._folded())
        for n in ("/home/u/secret", "source_path_redacted"):
            self.assertNotIn(n, blob, n)


@unittest.skipUnless(HAVE_FIXTURES, "EXTRACT fixtures not present")
class TestRealFixtureFold(unittest.TestCase):
    def test_real_fold_validates_and_carries_domains(self):
        snap = asm.assemble_snapshot({}, game_id="g", date="1946.5.19.9")
        folded = asm.fold_save_bundle(snap, ORACLE_OUT, game_id="g")
        self.assertEqual(schema.validate_snapshot(folded), [])
        self.assertEqual(folded["countries"]["GER"]["human_name"], "WooooWoooo")
        self.assertEqual(len(folded["strategic_air"]["air_bases"]), 337)
        self.assertTrue(folded["provenance"]["save"]["fills_canonical_domains"])
        blob = json.dumps(folded)
        for n in ("redacted", "OneDrive", "AppData", "source_path_redacted"):
            self.assertNotIn(n, blob, n)


if __name__ == "__main__":
    unittest.main()
