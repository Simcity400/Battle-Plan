"""Tests for tools/check-nextest-config.py: python tools/check_nextest_config_test.py"""

import contextlib
import importlib.util
import io
import os
import tempfile
import tomllib
import unittest
from pathlib import Path

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SPEC = importlib.util.spec_from_file_location("check_nextest_config", os.path.join(ROOT, "tools", "check-nextest-config.py"))
cn = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cn)

NAMES = {"replay", "collector", "dashboard", "dashboard-core"}


def findings(text):
    return cn.findings(tomllib.loads(text), NAMES)


class Selectors(unittest.TestCase):
    def test_a_package_no_member_has_is_found_once_with_its_text(self):
        config = """
[[profile.default.overrides]]
filter = '''
(package(dashboard-stability) & test(/at_scale/))
| package(replay) | package(dashboard-stability)
'''
[profile.ci]
default-filter = "deps(gone) | rdeps(=collector)"
"""
        self.assertEqual(findings(config), ["package(dashboard-stability)", "deps(gone)"])

    def test_each_matcher_kind_matches_as_nextest_does(self):
        for matcher in ("dashboard", "=dashboard", "~board-co", "/^dash.*core$/", "#dash*", "dash*"):
            self.assertEqual(findings(f"[profile.x]\ndefault-filter = 'package({matcher})'"), [], matcher)
        for matcher in ("=dash", "~stability", "/^core/", "dash"):
            self.assertEqual(findings(f"[profile.x]\ndefault-filter = 'package({matcher})'"), [f"package({matcher})"])

    def test_a_regex_holding_a_parenthesis_is_read_whole(self):
        self.assertEqual(findings(r"[profile.x]" "\n" r"default-filter = 'package(/^(replay|collector)$/)'"), [])

    def test_spacing_another_functions_matcher_and_rust_only_regexes_are_read_as_nextest_does(self):
        self.assertEqual(findings("[profile.x]\ndefault-filter = 'package (removed) | test(/package(retired)/)'"),
                         ["package (removed)"])
        self.assertEqual(findings(r"[profile.x]" "\n" r"default-filter = 'package(/^\p{L}+$/)'"), [])
        self.assertEqual(findings(r"[profile.x]" "\n" r"default-filter = 'package({replay\,collector}) | package(#dash\*)'"), [])
        self.assertEqual(findings("[profile.x]\ndefault-filter = 'not (package(removed)) | all() and (package(gone))'"),
                         ["package(removed)", "package(gone)"])

    def test_members_follow_the_workspace_globs_and_exclusions(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/*"]\nexclude = ["crates/old"]\n')
            for name in ("replay", "old", "loose"):
                (root / "crates" / name).mkdir(parents=True)
            for name in ("replay", "old"):
                (root / "crates" / name / "Cargo.toml").write_text(f'[package]\nname = "{name}-pkg"\n')
            self.assertEqual(cn.members(root), {"replay-pkg"})
            (root / ".config").mkdir()
            (root / ".config" / "nextest.toml").write_text("[profile.x]\ndefault-filter = 'package(old-pkg)'\n")
            with contextlib.redirect_stdout(io.StringIO()) as out:
                self.assertEqual(cn.main(root), 1)
            self.assertIn("package(old-pkg): no workspace package matches", out.getvalue())

    def test_the_repository_config_names_only_workspace_packages(self):
        with contextlib.redirect_stdout(io.StringIO()) as out:
            self.assertEqual(cn.main(), 0, out.getvalue())


if __name__ == "__main__":
    unittest.main()
