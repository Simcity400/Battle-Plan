#!/usr/bin/env python3
"""Prove domain 59's static catalog gate is MOD-AWARE, over synthetic layers.

`templates_equipment_catalog_gate.py` builds the only artifact replay has for
authored division templates, unit definitions and equipment definitions. It
resolves them across an ordered [vanilla, mod, mod, ...] layer list, and every
recording made with mods depends on that resolution being the game's own: a mod
file that replaces a vanilla one must WIN, a mod-only file must be ADDED, and a
mod that declares `replace_path` must drop the vanilla directory it replaces and
nothing else.

None of that had a test. The gate's own proof run needs an installed game and a
real mod, so it never ran in the battery; these cases need neither. They build
tiny Clausewitz files in a temp tree and drive the gate's own collectors, so a
change to the layering rules -- or to the gate's use of them -- fails here.

This is the mod-overlay half of domain 59's promotion bar. The absent/malformed
half is on the consumer side, in `game_ui::static_unit_catalogs`.
"""

from __future__ import annotations

import importlib.util
import os
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(os.path.abspath(__file__)).parents[3]
GATE_PATH = (
    REPO_ROOT
    / "extractor"
    / "native"
    / "lanes"
    / "templates_equipment_catalog_live"
    / "templates_equipment_catalog_gate.py"
)


def _load_gate():
    """Import the gate by path. It is a script, not a package module."""
    spec = importlib.util.spec_from_file_location(
        "templates_equipment_catalog_gate", GATE_PATH
    )
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


GATE = _load_gate()

UNIT_SUBDIR, UNIT_EXTS = GATE.STATIC_SUBDIRS["unit_definition_files"]
EQUIPMENT_SUBDIR, EQUIPMENT_EXTS = GATE.STATIC_SUBDIRS["equipment_definition_files"]
TEMPLATE_SUBDIR, TEMPLATE_EXTS = GATE.STATIC_SUBDIRS["division_template_files"]

UNIT_PATH = f"{UNIT_SUBDIR}/infantry.txt"
EQUIPMENT_PATH = f"{EQUIPMENT_SUBDIR}/infantry.txt"
TEMPLATE_PATH = f"{TEMPLATE_SUBDIR}/GER_1936.txt"

VANILLA_INFANTRY = "infantry = {\n\tmax_organisation = 60\n\tdefault_morale = 0.3\n}\n"
MOD_INFANTRY = "infantry = {\n\tmax_organisation = 90\n\tdefault_morale = 0.3\n}\n"
MOD_ONLY_UNIT = "space_marine = {\n\tmax_organisation = 200\n}\n"
VANILLA_EQUIPMENT = (
    "equipments = {\n\tinfantry_equipment_1 = {\n\t\tbuild_cost_ic = 0.43\n\t}\n}\n"
)
VANILLA_TEMPLATE = (
    'division_template = {\n\tname = "Infanterie"\n'
    "\tdivision_names_group = GER_INF_01\n"
    "\tregiments = {\n\t\tinfantry = { x = 0 y = 0 }\n\t}\n"
    "\tsupport = {\n\t\tengineer = { x = 0 y = 0 }\n\t}\n}\n"
)
MOD_TEMPLATE = (
    'division_template = {\n\tname = "Volksgrenadier"\n'
    "\tdivision_names_group = GER_INF_02\n"
    "\tregiments = {\n\t\tinfantry = { x = 0 y = 0 }\n\t\tinfantry = { x = 0 y = 1 }\n\t}\n"
    "\tsupport = {\n\t}\n}\n"
)


def write(root: Path, relative: str, text: str) -> Path:
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path


class ModOverlay(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.vanilla = root / "vanilla"
        self.mod = root / "test_mod"
        write(self.vanilla, UNIT_PATH, VANILLA_INFANTRY)
        write(self.vanilla, EQUIPMENT_PATH, VANILLA_EQUIPMENT)
        write(self.vanilla, TEMPLATE_PATH, VANILLA_TEMPLATE)
        (self.vanilla / "hoi4.exe").write_bytes(b"not the real executable")
        self.mod.mkdir(parents=True, exist_ok=True)

    def tearDown(self):
        self.tmp.cleanup()

    def layers(self, replace_paths=None):
        return [
            GATE.Layer("vanilla", self.vanilla),
            GATE.Layer("test_mod", self.mod, replace_paths or []),
        ]

    def descriptor(self, *replace_paths: str, name: str = "Test Mod") -> Path:
        """A real `.mod` descriptor beside the mod root, the way the game ships
        them. The gate must derive both the label and the replaced directories
        from THIS file; nothing in these cases hands them over by hand."""
        lines = [f'name="{name}"', f'path="{self.mod.as_posix()}"']
        lines += [f'replace_path="{path}"' for path in replace_paths]
        path = Path(self.tmp.name) / "test_mod.mod"
        path.write_text("\n".join(lines) + "\n", encoding="utf-8")
        return path

    def catalog(self, descriptor: Path | None):
        """Drive the SHIPPED entrypoint, not a hand-built layer list."""
        args = GATE.argparse.Namespace(
            vanilla_root=self.vanilla,
            mod=self.mod,
            mod_descriptor=descriptor,
            allow_missing_vanilla=False,
            mods=None,
            userdata=None,
            strict_mods=False,
            source_inventory=None,
        )
        return GATE.build_catalog(args)

    def unit_blocks(self, layers):
        files = GATE.build_file_map_layers(layers, UNIT_SUBDIR, UNIT_EXTS)
        return GATE.collect_top_level_blocks(
            files, skip_names={"division_template", "sub_units", "group", "categories"}
        )

    def test_a_mod_file_at_the_same_path_replaces_the_vanilla_definition(self):
        """WHAT PRODUCTION CHANGE MAKES THIS FAIL: resolving the layer list in
        the wrong order, or reading the vanilla root directly, would put the
        vanilla value in the artifact and every modded recording would replay
        with an authored definition the game never used."""
        write(self.mod, UNIT_PATH, MOD_INFANTRY)
        blocks = self.unit_blocks(self.layers())
        self.assertIn("infantry", blocks)
        self.assertEqual(
            blocks["infantry"]["fields"]["max_organisation"],
            "90",
            "the mod's value must win over vanilla's",
        )
        self.assertEqual(
            blocks["infantry"]["source"],
            UNIT_PATH,
            "the relative path is the game's, not the layer's",
        )
        self.assertEqual(
            blocks["infantry"]["source_sha256"],
            GATE.sha256_bytes((self.mod / UNIT_PATH).read_bytes()),
            "the recorded source digest must be the file that actually won",
        )
        self.assertEqual(
            [key for key in blocks if key.startswith("infantry")], ["infantry"]
        )

    def test_a_mod_only_file_adds_its_definitions_beside_vanillas(self):
        """WHAT PRODUCTION CHANGE MAKES THIS FAIL: restricting the file map to
        paths vanilla already has, which would silently drop every definition a
        total-conversion mod adds."""
        write(self.mod, f"{UNIT_SUBDIR}/space_marine.txt", MOD_ONLY_UNIT)
        blocks = self.unit_blocks(self.layers())
        self.assertEqual(sorted(blocks), ["equipments", "infantry", "space_marine"])
        self.assertEqual(blocks["space_marine"]["fields"]["max_organisation"], "200")
        self.assertEqual(blocks["infantry"]["fields"]["max_organisation"], "60")

    def test_replace_path_drops_exactly_the_directory_it_names(self):
        """WHAT PRODUCTION CHANGE MAKES THIS FAIL: treating `replace_path` as a
        prefix instead of one exact directory. `common/units` would then also
        swallow `common/units/equipment`, and a mod that only replaces unit
        stats would lose every vanilla equipment definition from the artifact."""
        write(self.mod, f"{UNIT_SUBDIR}/mod_units.txt", MOD_ONLY_UNIT)
        layers = self.layers(replace_paths=[UNIT_SUBDIR])
        self.assertEqual(
            sorted(self.unit_blocks(layers)),
            ["equipments", "space_marine"],
            "the replaced directory keeps only the mod's own files, and the "
            "nested equipment directory is not part of it",
        )
        equipment = GATE.collect_top_level_blocks(
            GATE.build_file_map_layers(layers, EQUIPMENT_SUBDIR, EQUIPMENT_EXTS)
        )
        self.assertIn(
            "equipments",
            equipment,
            "a replace_path on common/units must not reach common/units/equipment",
        )

    def test_division_templates_follow_the_same_override_rules(self):
        """WHAT PRODUCTION CHANGE MAKES THIS FAIL: collecting templates from the
        vanilla root rather than from the resolved layer map, which would put a
        mod's starting order of battle in the artifact as vanilla's."""
        write(self.mod, TEMPLATE_PATH, MOD_TEMPLATE)
        write(self.mod, f"{TEMPLATE_SUBDIR}/SOV_1936.txt", VANILLA_TEMPLATE)
        files = GATE.build_file_map_layers(self.layers(), TEMPLATE_SUBDIR, TEMPLATE_EXTS)
        self.assertEqual(
            sorted(files), [f"{TEMPLATE_SUBDIR}/GER_1936.txt", f"{TEMPLATE_SUBDIR}/SOV_1936.txt"]
        )
        by_source = {
            row["source"]: row for row in GATE.collect_division_templates(files)
        }
        german = by_source[f"{TEMPLATE_SUBDIR}/GER_1936.txt"]
        self.assertEqual(german["name"], "Volksgrenadier")
        self.assertEqual(german["regiment_count"], 2)
        self.assertEqual(german["support_count"], 0)
        self.assertEqual(
            by_source[f"{TEMPLATE_SUBDIR}/SOV_1936.txt"]["name"], "Infanterie"
        )

    def test_the_gate_hands_its_layers_over_vanilla_first(self):
        """Every case above rests on the ORDER of the list the gate builds. If
        `make_layers` ever appended vanilla after the mods, each case here would
        still pass on a hand-built list while the shipped artifact silently
        became vanilla-only.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: appending the vanilla layer
        after the mod layer in `make_layers`."""
        write(self.mod, UNIT_PATH, MOD_INFANTRY)
        layers, _warnings, discovery = GATE.make_layers(
            self.vanilla, self.mod, None, False
        )
        self.assertEqual([layer.label for layer in layers], ["vanilla", self.mod.name])
        self.assertEqual(discovery["selected"], str(self.vanilla.resolve()))
        self.assertEqual(
            self.unit_blocks(layers)["infantry"]["fields"]["max_organisation"], "90"
        )


    def test_build_catalog_resolves_the_mod_over_vanilla(self):
        """The cases above drive the gate's collectors over layer lists this
        file builds. `build_catalog` is what the battery, the packer and every
        recording actually run, and it assembles its own layers.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: `build_catalog` building its
        file maps from the vanilla root alone, or dropping the mod layer before
        the maps are built -- the artifact would then carry vanilla's authored
        values for a modded game and every case above would still pass."""
        write(self.mod, UNIT_PATH, MOD_INFANTRY)
        write(self.mod, f"{UNIT_SUBDIR}/space_marine.txt", MOD_ONLY_UNIT)
        catalog = self.catalog(None)
        units = catalog["catalog"]["unit_definitions"]
        self.assertEqual(units["infantry"]["fields"]["max_organisation"], "90")
        self.assertIn("space_marine", units)
        self.assertEqual(
            units["infantry"]["source_sha256"],
            GATE.sha256_bytes((self.mod / UNIT_PATH).read_bytes()),
        )
        self.assertEqual(
            [(row["label"], row["root"]) for row in catalog["layers"]],
            [("vanilla", str(self.vanilla.resolve())), ("test_mod", str(self.mod))],
        )
        self.assertEqual(catalog["counts"]["unit_definitions"], len(units))
        without_digest = {
            key: value for key, value in catalog.items() if key != "artifact_sha256"
        }
        self.assertEqual(
            catalog["artifact_sha256"],
            GATE.sha256_bytes(GATE.canonical_json(without_digest)),
        )

    def test_build_catalog_takes_replace_path_from_the_mod_descriptor(self):
        """`replace_path` reaches production only by being parsed out of a
        `.mod` descriptor; every case above hands the directory over directly,
        so the parse itself was never covered.

        WHAT PRODUCTION CHANGE MAKES THIS FAIL: `build_catalog`/`make_layers`
        no longer reading `replace_path` (or `name`) out of the descriptor --
        a total-conversion mod that replaces `common/units` would keep every
        vanilla unit it deliberately removed, and replay would show authored
        units the game never loaded."""
        write(self.mod, f"{UNIT_SUBDIR}/mod_units.txt", MOD_ONLY_UNIT)
        catalog = self.catalog(self.descriptor(UNIT_SUBDIR))
        units = catalog["catalog"]["unit_definitions"]
        self.assertNotIn(
            "infantry",
            units,
            "the descriptor replaced common/units, so vanilla's units are gone",
        )
        self.assertIn("space_marine", units)
        self.assertIn(
            "equipments",
            catalog["catalog"]["equipment_definitions"],
            "a replace_path on common/units must not reach common/units/equipment",
        )
        mod_layer = catalog["layers"][1]
        self.assertEqual(mod_layer["replace_path"], [UNIT_SUBDIR])
        self.assertEqual(
            mod_layer["label"], "Test Mod", "the label is the descriptor's name"
        )
        self.assertIn("infantry", self.catalog(None)["catalog"]["unit_definitions"])


if __name__ == "__main__":
    unittest.main()
