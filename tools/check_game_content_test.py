"""Tests for tools/check-game-content.py: python tools/check_game_content_test.py"""

import contextlib
import importlib.util
import io
import os
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch
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
        _, errors = cgc.allowed("\nui/a.png\nui/b.rs game-value\n")
        self.assertEqual(errors, ["tools/game-content.allow:2: allow entry gives no provenance",
                                  "tools/game-content.allow:3: allow entry gives no provenance"])

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
        self.assertEqual(cgc.asset_kind("x.rs", b"small\0binary"), "binary")
        self.assertEqual(cgc.asset_kind("x.rs", "naÃ¯ve".encode()), None)

    def test_a_long_base64_run_is_an_embedded_asset(self):
        data = b"const ART: &str = \"" + b"QUJD" * 300 + b"\";\n"
        self.assertEqual(cgc.tree_findings([("x.rs", b"first\n" + data)], []),
                         ["x.rs:2: embedded base64 run of 1200 characters"])
        self.assertEqual(cgc.tree_findings([("x.rs", b"QUJD" * 200)], []), [])


class Text(unittest.TestCase):
    def test_a_script_run_needs_three_consecutive_lines_in_the_same_order(self):
        rows = [f"trial_command = {{ sequence = {n} fixture_name = trial }}" for n in range(3)]
        hashes = lambda lines: set(h for h, _ in cgc.text_hashes("\n".join(lines)))
        self.assertTrue(hashes(rows))
        self.assertEqual(hashes(rows[:2]), set())
        self.assertTrue(hashes(rows).isdisjoint(hashes(rows[::-1])))
        self.assertEqual(hashes(rows), hashes(["  " + row for row in rows]))
        names = [f'b"very_long_fixture_interface_identifier_of_our_own_{n}",' for n in range(3)]
        self.assertEqual(hashes(names), set())

    def test_words_in_strings_and_comments_are_matched_after_unescaping_and_folding(self):
        self.assertIn(LONG, cgc.segments(f'  let s = "{LONG}";'))
        self.assertIn(LONG, cgc.segments(f'// {LONG}'))
        self.assertIn(LONG, cgc.segments(f'  key = \"{LONG}\"'.replace('"', r'\"')))
        self.assertIn(LONG, cgc.segments(LONG.replace(' ', '   ')))

    def test_a_short_string_of_words_counts_but_a_short_line_of_code_does_not(self):
        self.assertEqual(cgc.segments('label: "Made-up Banner Text Here",'), {"Made-up Banner Text Here"})
        self.assertEqual(cgc.segments(' key:0 "$NAME|H$ made-up factor: $VAL$"'), {"$NAME|H$ made-up factor: $VAL$"})
        self.assertEqual(cgc.segments("from shapes import Square"), set())
        self.assertEqual(cgc.segments('"float4 made : UP_NAME_HERE;"'), set())
        self.assertEqual(cgc.segments('"x = made up words here"'), set())


class Values(unittest.TestCase):
    def test_a_define_with_the_installed_value_is_found_but_not_trivial_or_other_values(self):
        installed = cgc.define_values("NDefines = {\n NGame = {\n  DEPTH = 30, -- DEPTH = 47\n -- OFF = 72\n  SCALE = 0.25\n  OFF = 0\n }\n}\n")
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
        active = {index.refresh("game", game, "build 1", notes.append)}
        files = [("art.bin", art), ("fixture.rs", f'let a = 1;\nlet t = "{LONG}";\nlet d = "DEPTH = 30";\n'.encode())]
        found = cgc.install_findings(index, files, [], active)
        self.assertEqual(found, ["art.bin:1: copy equals game file gfx/a.dds",
                                 "fixture.rs:3: game-value DEPTH has the installation's value",
                                 "fixture.rs:2: copied-text shares a run with game"])
        self.assertFalse(any(LONG in line for line in found))
        rules, _ = cgc.allowed("fixture.rs game-value routed to #1\nfixture.rs copied-text routed to #2\nother.rs game-value gone\n")
        self.assertEqual(cgc.install_findings(index, files, rules, active), ["art.bin:1: copy equals game file gfx/a.dds"])
        fixed = [("fixture.rs", b"let d = \"DEPTH = 17\";\n")]
        self.assertEqual(cgc.install_findings(index, fixed, rules, active), [])
        self.assertEqual(cgc.install_findings(index, files, [], set()), [])

    @patch.object(cgc, "registry_value", return_value=None)
    def test_push_checks_the_changed_and_untracked_paths_with_a_read_only_index(self, _registry):
        repo = self.folder / "repo"
        game = self.folder / "game"
        cache = self.folder / "cache"
        snippet = "fixture_script = { flag = violet }"
        self.write("game/trial.txt", snippet)
        self.write("repo/already.rs", snippet)
        self.write("repo/new.rs", "original")
        self.write("repo/tools/game-content.allow", "")
        self.addCleanup(setattr, cgc, "ROOT", cgc.ROOT)
        self.addCleanup(setattr, cgc, "ALLOW", cgc.ALLOW)
        cgc.ROOT, cgc.ALLOW = repo, repo / cgc.ALLOW_NAME
        index = cgc.Index(cache / f"index-v{cgc.INDEX_VERSION}.sqlite3")
        index.refresh("game", game, None, lambda _: None)
        index.db.close()
        def git(*args):
            subprocess.run(["git", *args], cwd=repo, capture_output=True, check=True)
        git("init")
        git("add", ".")
        git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-m", "fixture")
        args = ["--against", "HEAD", "--game", str(game), "--user-dir", str(repo), "--cache", str(cache)]
        self.assertEqual(cgc.main(args), 0)
        self.write("repo/new.rs", snippet)
        self.write("repo/untracked.dat", "tiny")
        self.assertEqual(cgc.changed_since("HEAD"), {"new.rs", "untracked.dat"})
        self.assertEqual(cgc.main(args), 1)
        (repo / "untracked.dat").unlink()
        self.assertEqual(cgc.main(args), 1)
        self.write("repo/new.rs", "ours")
        self.assertEqual(cgc.main(args), 0)

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

    def init_repo(self):
        repo = self.folder / "repo"
        self.write("repo/new.rs", "original")
        self.write("repo/tools/game-content.allow", "")
        self.addCleanup(setattr, cgc, "ROOT", cgc.ROOT)
        self.addCleanup(setattr, cgc, "ALLOW", cgc.ALLOW)
        cgc.ROOT, cgc.ALLOW = repo, repo / cgc.ALLOW_NAME
        def git(*args):
            subprocess.run(["git", *args], cwd=repo, capture_output=True, check=True)
        git("init")
        git("add", ".")
        git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-m", "fixture")
        return repo

    def run_main(self, argv):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code = cgc.main(argv)
        return code, output.getvalue()

    @patch.object(cgc, "registry_value", return_value=None)
    def test_push_requires_a_current_index_when_an_installation_is_present(self, _registry):
        repo = self.init_repo()
        game = self.folder / "game"
        user = self.folder / "user"
        cache = self.folder / "cache"
        snippet = "fixture_script = { flag = violet }"
        self.write("game/trial.txt", snippet)
        self.write("user/mod/extra/mod.txt", "mod_fixture = { tone = amber }")
        index_path = cache / f"index-v{cgc.INDEX_VERSION}.sqlite3"
        base_args = ["--against", "HEAD", "--game", str(game), "--user-dir", str(user), "--cache", str(cache)]

        notes = lambda text: [line for line in text.splitlines() if line.startswith("not run here: ")]

        code, out = self.run_main(base_args)
        self.assertEqual(code, 1)
        self.assertFalse(index_path.exists())
        self.assertTrue(any("no index at " in line and "python tools/check-game-content.py" in line for line in notes(out)))

        code, out = self.run_main(["--tree-only", *base_args])
        self.assertEqual(code, 0)
        self.assertFalse(index_path.exists())
        self.assertEqual(notes(out), ["not run here: installation comparison (--tree-only)"])

        clean_env = {k: v for k, v in os.environ.items() if k not in ("BATTLEPLAN_GAME_DIR", "BATTLEPLAN_USER_DIR")}
        with patch.dict(os.environ, clean_env, clear=True):
            code, out = self.run_main(["--against", "HEAD", "--cache", str(cache)])
        self.assertEqual(code, 0)
        self.assertFalse(index_path.exists())
        self.assertTrue(any("no game installation" in line for line in notes(out)))

        self.write(f"cache/index-v{cgc.INDEX_VERSION}.sqlite3", "not a sqlite database")
        code, out = self.run_main(base_args)
        self.assertEqual(code, 1)
        self.assertTrue(any("unreadable index at " in line and "python tools/check-game-content.py" in line for line in notes(out)))

        real_is_file = Path.is_file
        def deny_cache(path):
            if path == index_path.resolve():
                raise PermissionError("access denied")
            return real_is_file(path)
        with patch.object(Path, "is_file", deny_cache):
            code, out = self.run_main(base_args)
        self.assertEqual(code, 1)
        self.assertTrue(any("cache inaccessible at " in line and "python tools/check-game-content.py" in line for line in notes(out)))

        code, _ = self.run_main(["--game", str(game), "--user-dir", str(user), "--cache", str(cache)])
        self.assertEqual(code, 0)
        self.assertTrue(index_path.is_file())

        code, out = self.run_main(base_args)
        self.assertEqual(code, 0)
        self.assertEqual(notes(out), [])
        self.assertIn("installation comparison: 2 game and mod folders", out)

        self.write("repo/new.rs", snippet)
        code, out = self.run_main(base_args)
        self.assertEqual(code, 1)
        self.assertIn("copy equals game file trial.txt", out)

        self.write("user/mod/extra/mod.txt", "mod_fixture = { tone = amber, changed = yes }")
        code, out = self.run_main(base_args)
        self.assertEqual(code, 1)
        self.assertTrue(any("for mod extra" in line and "python tools/check-game-content.py" in line for line in notes(out)))
        self.assertIn("copy equals game file trial.txt", out)
        with contextlib.closing(cgc.Index(index_path, readonly=True).db) as db:
            probe = cgc.Index.__new__(cgc.Index)
            probe.db = db
            self.assertIsNone(probe.current("mod extra", user / "mod" / "extra", None)[0])

        self.write("repo/new.rs", "ours")
        code, out = self.run_main(base_args)
        self.assertEqual(code, 1)
        self.assertTrue(any("for mod extra" in line and "python tools/check-game-content.py" in line for line in notes(out)))
        self.assertIn("index out of date for mod extra", out)
        self.assertIn("python tools/check-game-content.py", out)

    @patch.object(cgc, "registry_value", return_value=None)
    def test_mods_with_the_same_folder_name_are_indexed_and_checked_independently(self, _registry):
        self.init_repo()
        game = self.folder / "game"
        user = self.folder / "user"
        cache = self.folder / "cache"
        first = self.folder / "mods_a" / "content"
        second = self.folder / "mods_b" / "content"
        self.write("game/trial.txt", "game_fixture = { value = 42 }")
        self.write("mods_a/content/a.txt", "first_mod_script = { flag = teal }")
        self.write("mods_b/content/b.txt", "second_mod_script = { flag = ochre }")
        self.write("user/mod/first.mod", f'name="First"\npath="{first.as_posix()}"\n')
        self.write("user/mod/second.mod", f'name="Second"\npath="{second.as_posix()}"\n')
        build_args = ["--game", str(game), "--user-dir", str(user), "--cache", str(cache)]
        push_args = ["--against", "HEAD", *build_args]

        code, _ = self.run_main(build_args)
        self.assertEqual(code, 0)
        code, out = self.run_main(push_args)
        self.assertEqual(code, 0)
        self.assertIn("installation comparison: 3 game and mod folders", out)

        updated = "first_mod_script = { flag = teal, extra = cobalt }"
        self.write("mods_a/content/a.txt", updated)
        self.write("repo/new.rs", updated)
        code, out = self.run_main(push_args)
        self.assertEqual(code, 1)
        self.assertTrue(any(line.startswith("not run here: installation comparison for mod content (") for line in out.splitlines()))
        self.assertIn("python tools/check-game-content.py", out)

        code, _ = self.run_main(build_args)
        self.assertEqual(code, 1)
        code, out = self.run_main(push_args)
        self.assertEqual(code, 1)
        self.assertIn("copy equals mod content (", out)
        self.assertIn("file a.txt", out)

        self.write("repo/new.rs", "ours")
        code, out = self.run_main(push_args)
        self.assertEqual(code, 0)
        self.assertNotIn("not run here:", out)

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
