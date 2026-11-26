"""Tests for tools/check-game-content.py: python tools/check_game_content_test.py"""

import importlib.util
import os
import shutil
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SPEC = importlib.util.spec_from_file_location("check_game_content", os.path.join(ROOT, "tools", "check-game-content.py"))
cgc = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cgc)

PNG = b"\x89PNG\r\n\x1a\n" + bytes(64)
LONG = "a run of made-up words that is long enough to count as copied"


class Tree(unittest.TestCase):
    def test_an_asset_type_fails_unless_allowed_with_provenance(self):
        rules, errors = cgc.allowed("ui/art/*.png drawn for BattlePlan\n")
        self.assertEqual(errors, [])
        self.assertEqual(cgc.tree_findings([("ui/art/a.png", PNG)], rules), [])
        self.assertEqual(cgc.tree_findings([("ui/other/a.png", PNG)], rules),
                         ["ui/other/a.png:1: asset image file not in game-content.allow",
                          "tools/game-content.allow: stale-allow ui/art/*.png matches no file"])

    def test_an_entry_without_provenance_is_refused(self):
        _, errors = cgc.allowed("# comment\n\nui/a.png\nui/b.rs game-value\n")
        self.assertEqual(errors, ["tools/game-content.allow:3: allow entry gives no provenance",
                                  "tools/game-content.allow:4: allow entry gives no provenance"])

    def test_a_kind_entry_exempts_one_finding_and_never_an_asset(self):
        rules, _ = cgc.allowed("ui/b.rs game-value until #1 lands\nui/*.png drawn here\n")
        self.assertEqual(rules, [("ui/b.rs", "game-value", "until #1 lands"), ("ui/*.png", None, "drawn here")])
        self.assertEqual(cgc.allow_match("ui/b.rs", rules, "game-value"), "ui/b.rs")
        self.assertIsNone(cgc.allow_match("ui/b.rs", rules, "copied-text"))
        self.assertEqual(cgc.tree_findings([("ui/b.rs", PNG), ("ui/a.png", PNG)], rules),
                         ["ui/b.rs:1: asset binary file not in game-content.allow"])

    def test_a_glob_star_stays_in_its_folder(self):
        self.assertTrue(cgc.glob_match("a/b.png", "a/*.png"))
        self.assertFalse(cgc.glob_match("a/c/b.png", "a/*.png"))
        self.assertTrue(cgc.glob_match("a/c/b.png", "a/**.png"))

    def test_game_data_types_and_binary_content_are_assets_whatever_the_name(self):
        for name in ("x.dds", "x.gfx", "x.gui", "x.yml", "x.lua", "x.shader", "x.mesh", "x.ogg", "x.ttf", "x.battleplan"):
            self.assertIsNotNone(cgc.asset_kind(name, b"text"), name)
        self.assertEqual(cgc.asset_kind("renamed.rs", PNG), "binary")
        self.assertEqual(cgc.asset_kind("x.rs", "let nul = '\0';".encode()), None)
        self.assertEqual(cgc.asset_kind("x.rs", "naïve".encode()), None)

    def test_a_long_base64_run_is_an_embedded_asset(self):
        data = b"const ART: &str = \"" + b"QUJD" * 300 + b"\";\n"
        self.assertEqual(cgc.tree_findings([("x.rs", b"first\n" + data)], []),
                         ["x.rs:2: embedded base64 run of 1200 characters"])
        self.assertEqual(cgc.tree_findings([("x.rs", b"QUJD" * 200)], []), [])


class Text(unittest.TestCase):
    def test_segments_are_the_line_its_strings_and_its_comment_folded(self):
        line = f'    let s = "{LONG}"; // {LONG} too'
        self.assertEqual(cgc.segments(line), {line.strip(), LONG, f"{LONG} too"})

    def test_an_escaped_quote_and_spacing_do_not_hide_a_copy(self):
        rust = f'"key = \\"{LONG}\\""'
        self.assertIn(f'key = "{LONG}"', cgc.segments(rust))
        self.assertIn(LONG, cgc.segments(f"  {LONG.replace(' ', '   ')}  "))

    def test_short_runs_and_key_names_are_not_segments(self):
        self.assertEqual(cgc.segments("is_triggered_only = yes"), set())
        self.assertEqual(cgc.segments('k = "some_long_window_element_name";'), set())

    def test_a_short_string_of_words_counts_but_a_short_line_of_code_does_not(self):
        self.assertEqual(cgc.segments('label: "Made-up Banner Text Here",'), {"Made-up Banner Text Here"})
        self.assertEqual(cgc.segments(' key:0 "$NAME|H$ made-up factor: $VAL$"'), {"$NAME|H$ made-up factor: $VAL$"})
        self.assertEqual(cgc.segments("from shapes import Square"), set())
        self.assertEqual(cgc.segments('"float4 made : UP_NAME_HERE;"'), set())
        self.assertEqual(cgc.segments('"x = made up words here"'), set())


class Values(unittest.TestCase):
    def test_a_define_with_the_installed_value_is_found_but_not_trivial_or_other_values(self):
        installed = cgc.define_values("NDefines = {\n NGame = {\n  DEPTH = 30, -- note\n  SCALE = 0.25\n  OFF = 0\n }\n}\n")
        self.assertEqual(installed, {"DEPTH": {30.0}, "SCALE": {0.25}, "OFF": {0.0}})
        text = '"NDefines={NGame={DEPTH=30}}"\n"SCALE = .25, OFF = 0"\n"DEPTH = 17"\n"DEPTH = 300"\n'
        self.assertEqual(cgc.game_values(text, installed), [(1, "DEPTH"), (2, "SCALE")])


class Installation(unittest.TestCase):
    def setUp(self):
        self.folder = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.folder, True)

    def write(self, relative, data):
        path = self.folder / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data if isinstance(data, bytes) else data.encode())
        return path

    def test_copies_copied_text_and_game_values_are_found_and_never_quoted(self):
        game = self.folder / "game"
        art = bytes(range(256)) * 2
        self.write("game/gfx/a.dds", art)
        self.write("game/localisation/a.yml", f' key:0 "{LONG}"\n')
        self.write("game/common/defines/00.lua", "NDefines = { NGame = { DEPTH = 30 } }\n")
        index = cgc.Index(self.folder / "cache" / "index.sqlite3")
        self.addCleanup(index.db.close)
        notes = []
        index.refresh("game", game, "build 1", notes.append)
        repo = self.folder / "repo"
        files = [("art.bin", art), ("fixture.rs", f'let a = 1;\nlet t = "{LONG}";\nlet d = "DEPTH = 30";\n'.encode())]
        for name, data in files:
            self.write(f"repo/{name}", data)
        self.addCleanup(setattr, cgc, "ROOT", cgc.ROOT)
        cgc.ROOT = repo
        found = cgc.install_findings(index, files, [])
        self.assertEqual(found, ["art.bin:1: copy equals game file gfx/a.dds",
                                 "fixture.rs:3: game-value DEPTH has the installation's value",
                                 "fixture.rs:2: copied-text shares a run with game"])
        self.assertFalse(any(LONG in line for line in found))
        rules, _ = cgc.allowed("fixture.rs game-value routed to #1\nfixture.rs copied-text routed to #2\nother.rs game-value gone\n")
        self.assertEqual(cgc.install_findings(index, files, rules), ["art.bin:1: copy equals game file gfx/a.dds"])
        fixed = [("fixture.rs", b"let d = \"DEPTH = 17\";\n")]
        self.assertEqual(cgc.install_findings(index, fixed, rules),
                         ["tools/game-content.allow: stale-allow fixture.rs game-value is no longer needed",
                          "tools/game-content.allow: stale-allow fixture.rs copied-text is no longer needed"])

    def test_an_unchanged_stamp_is_not_indexed_again_and_a_walked_folder_follows_its_files(self):
        index = cgc.Index(self.folder / "cache" / "index.sqlite3")
        self.addCleanup(index.db.close)
        notes = []
        self.write("mod/a.txt", LONG)
        self.assertIsNone(index.current("mod", self.folder / "mod", None)[0])
        first = index.refresh("mod", self.folder / "mod", None, notes.append)
        index.refresh("mod", self.folder / "mod", None, notes.append)
        self.assertEqual(len(notes), 1)
        self.assertEqual(index.current("mod", self.folder / "mod", None)[0], first)
        self.write("mod/b.txt", LONG + " again")
        self.assertIsNone(index.current("mod", self.folder / "mod", None)[0])
        self.assertEqual(index.refresh("mod", self.folder / "mod", None, notes.append), first)
        self.assertEqual(len(notes), 2)
        self.assertIsNone(index.current("game", self.folder / "mod", "build 2")[0])

    def test_no_installation_is_reported_as_not_run(self):
        missing = SimpleNamespace(game=str(self.folder / "absent"), user_dir=None)
        roots, reason = cgc.locate(missing)
        self.assertEqual(roots, [])
        self.assertIn("does not exist", reason)


class Manifests(unittest.TestCase):
    def test_steam_key_values_parse_into_nested_blocks(self):
        text = '"AppState"\n{\n\t"buildid"\t\t"7"\n\t"InstalledDepots"\n\t{\n\t\t"1"\n\t\t{\n\t\t\t"manifest"\t\t"9"\n\t\t}\n\t}\n}\n'
        self.assertEqual(cgc.vdf_blocks(text), {"AppState": {"buildid": "7", "InstalledDepots": {"1": {"manifest": "9"}}}})


if __name__ == "__main__":
    unittest.main()
