"""Tests for the capture-side MOD IDENTITY reader (live_capture.mod_identity) +
the schema mod-set helpers + the assembler `provenance.mod` wiring (§K).

NO game, NO launcher required -- everything runs against synthetic temp
dlc_load.json + .mod descriptor fixtures.

Run:  python -m unittest discover -s live_capture
"""
from __future__ import annotations

import os
import sys
import tempfile
import unittest

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir, os.pardir))
if _REPO not in sys.path:
    sys.path.insert(0, _REPO)

from replay import schema  # noqa: E402
from live_capture import mod_identity as mi  # noqa: E402
from live_capture import assembler as asm  # noqa: E402


_USER_ROOT = "C:/Us" + "ers/somebody"
_CLOUD_DOCS = "One" + "Drive"
_STEAM_ROOT = "F:/Steam" + "Library"

_TEST_MOD = (
    'version="1.18"\n'
    'tags={\n\t"Balance"\n\t"Fixes"\n}\n'
    'replace_path="common/countries"\n'
    'name="AMSB - DEV"\n'
    'supported_version="1.19.*"\n'
    f'path="{_USER_ROOT}/{_CLOUD_DOCS}/Documents/Paradox Interactive/Hearts of Iron IV/mod/test"\n'
)
_HMM_MOD = (
    'version="1.18"\n'
    'name="HMM"\n'
    'supported_version="1.18.*"\n'
    f'path="{_STEAM_ROOT}/steamapps/workshop/content/394360/3365264825"\n'
    'remote_file_id="3365264825"\n'
)


def _write_userdata(tmp, enabled, descriptors):
    """Lay out a fake user-data dir: dlc_load.json + mod/<name>.mod files."""
    import json
    os.makedirs(os.path.join(tmp, "mod"), exist_ok=True)
    for ref, text in descriptors.items():
        p = os.path.join(tmp, ref.replace("\\", "/"))
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w", encoding="utf-8") as f:
            f.write(text)
    with open(os.path.join(tmp, "dlc_load.json"), "w", encoding="utf-8") as f:
        json.dump({"enabled_mods": enabled, "disabled_dlcs": []}, f)
    return tmp


class TestDescriptorParse(unittest.TestCase):
    def test_parse_local_mod_drops_path(self):
        f = mi.parse_mod_descriptor(_TEST_MOD)
        self.assertEqual(f["name"], "AMSB - DEV")
        self.assertEqual(f["version"], "1.18")
        self.assertEqual(f["supported_version"], "1.19.*")
        self.assertIsNone(f["workshop_id"])
        # the absolute personal path must NEVER be returned (privacy).
        self.assertNotIn("path", f)
        for v in f.values():
            self.assertNotIn(_CLOUD_DOCS, (v or ""))
            self.assertNotIn("C:/Us" + "ers", (v or ""))

    def test_parse_workshop_mod_gets_id(self):
        f = mi.parse_mod_descriptor(_HMM_MOD)
        self.assertEqual(f["name"], "HMM")
        self.assertEqual(f["workshop_id"], "3365264825")


class TestReadModset(unittest.TestCase):
    def test_no_userdata_dir_is_none(self):
        # NO FABRICATION: a missing dir -> None, never a guessed mod-set.
        self.assertIsNone(mi.read_modset(None))
        self.assertIsNone(mi.read_modset(os.path.join(tempfile.gettempdir(),
                                                      "definitely_not_here_xyz")))

    def test_missing_dlc_load_is_none(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertIsNone(mi.read_modset(tmp))  # dir exists, no dlc_load.json

    def test_malformed_dlc_load_is_none(self):
        with tempfile.TemporaryDirectory() as tmp:
            with open(os.path.join(tmp, "dlc_load.json"), "w") as f:
                f.write("{not json")
            self.assertIsNone(mi.read_modset(tmp))

    def test_empty_enabled_is_measured_vanilla(self):
        # Distinct from None: launcher READ, zero mods -> measured vanilla.
        with tempfile.TemporaryDirectory() as tmp:
            _write_userdata(tmp, [], {})
            ms = mi.read_modset(tmp)
            self.assertIsNotNone(ms)
            self.assertEqual(ms["modset_id"], "vanilla")
            self.assertEqual(ms["mods"], [])
            self.assertEqual(ms["source"], "dlc_load.json")

    def test_single_local_mod(self):
        with tempfile.TemporaryDirectory() as tmp:
            _write_userdata(tmp, ["mod/test.mod"], {"mod/test.mod": _TEST_MOD})
            ms = mi.read_modset(tmp)
            self.assertEqual(ms["load_order"], ["test.mod"])
            self.assertEqual(len(ms["mods"]), 1)
            m = ms["mods"][0]
            self.assertEqual(m["name"], "AMSB - DEV")
            self.assertEqual(m["version"], "1.18")
            self.assertEqual(m["descriptor"], "test.mod")
            self.assertNotEqual(ms["modset_id"], "vanilla")
            # the whole serialized mod-set must be path-clean.
            import json
            blob = json.dumps(ms)
            self.assertNotIn(_CLOUD_DOCS, blob)
            self.assertNotIn("C:/Us" + "ers", blob)

    def test_enabled_but_missing_descriptor_no_fabrication(self):
        # Mod enabled in dlc_load.json but its .mod file is absent: recorded as
        # enabled-but-unreadable, NAME/VERSION null (never invented).
        with tempfile.TemporaryDirectory() as tmp:
            _write_userdata(tmp, ["mod/ghost.mod"], {})
            ms = mi.read_modset(tmp)
            self.assertEqual(len(ms["mods"]), 1)
            self.assertIsNone(ms["mods"][0]["name"])
            self.assertIsNone(ms["mods"][0]["version"])
            self.assertEqual(ms["mods"][0]["descriptor"], "ghost.mod")

    def test_load_order_preserved(self):
        with tempfile.TemporaryDirectory() as tmp:
            _write_userdata(tmp, ["mod/test.mod", "mod/hmm.mod"],
                            {"mod/test.mod": _TEST_MOD, "mod/hmm.mod": _HMM_MOD})
            ms = mi.read_modset(tmp)
            self.assertEqual(ms["load_order"], ["test.mod", "hmm.mod"])
            self.assertEqual([m["name"] for m in ms["mods"]], ["AMSB - DEV", "HMM"])


class TestModsetId(unittest.TestCase):
    def test_vanilla_sentinel(self):
        self.assertEqual(schema.modset_id([]), "vanilla")

    def test_order_sensitive(self):
        a = [{"workshop_id": "1", "version": "1.0"},
             {"workshop_id": "2", "version": "1.0"}]
        b = list(reversed(a))
        self.assertNotEqual(schema.modset_id(a), schema.modset_id(b))

    def test_version_sensitive(self):
        a = [{"workshop_id": "1", "version": "1.0"}]
        b = [{"workshop_id": "1", "version": "1.1"}]
        self.assertNotEqual(schema.modset_id(a), schema.modset_id(b))

    def test_id_falls_back_to_name(self):
        # local mod (no workshop id) is still identified by name.
        a = [{"workshop_id": None, "name": "MyUI", "version": "1.0"}]
        self.assertNotEqual(schema.modset_id(a), "vanilla")

    def test_delimiter_not_spoofable(self):
        # Codex review A: an '@'/'|' inside a name must not let two distinct
        # mod-sets collide. Canonical-JSON digest makes these distinct.
        a = [{"name": "a@1", "version": "0"}]
        b = [{"name": "a", "version": "1@0"}]
        self.assertNotEqual(schema.modset_id(a), schema.modset_id(b))


class TestAssemblerWiring(unittest.TestCase):
    def test_modset_none_keeps_provenance_mod_none(self):
        snap = asm.assemble_snapshot({}, game_id="g", date="1940.1.1.12",
                                     modset=None)
        self.assertIsNone(snap["provenance"]["mod"])
        self.assertEqual(schema.validate_snapshot(snap), [])

    def test_modset_stamped_into_provenance(self):
        ms = {"modset_id": "abc123", "source": "dlc_load.json",
              "load_order": ["test.mod"],
              "mods": [{"name": "AMSB", "workshop_id": None, "version": "1.18",
                        "supported_version": "1.19.*", "descriptor": "test.mod"}]}
        snap = asm.assemble_snapshot({}, game_id="g", date="1940.1.1.12",
                                     modset=ms)
        self.assertEqual(snap["provenance"]["mod"]["modset_id"], "abc123")
        # additive: the snapshot still validates with the mod-set present.
        self.assertEqual(schema.validate_snapshot(snap), [])

    def test_validate_rejects_bad_mod_shape(self):
        snap = asm.assemble_snapshot({}, game_id="g", date="1940.1.1.12")
        snap["provenance"]["mod"] = {"modset_id": "", "mods": "nope"}
        problems = schema.validate_snapshot(snap)
        self.assertTrue(any("modset_id" in p for p in problems))
        self.assertTrue(any("mods" in p for p in problems))


if __name__ == "__main__":
    unittest.main()
