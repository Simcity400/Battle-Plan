"""Tests for tools/check-machine-paths.py: python tools/check_machine_paths_test.py"""

import importlib.util
import os
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SPEC = importlib.util.spec_from_file_location("check_machine_paths", os.path.join(ROOT, "tools", "check-machine-paths.py"))
cmp = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cmp)

CHECKOUT = "C:" + "\\" + "hoi4_mp" + "_stats" + "\\captures"
BUILD = "/c/" + "hoi4_" + "build/ci"
HOME = "C:" + "\\" + "Users" + "\\" + "jsmith" + "\\x"


class Scan(unittest.TestCase):
    def test_a_checkout_path_is_found_in_either_slash_style(self):
        self.assertEqual(cmp.scan("a.rs", f'let p = r"{CHECKOUT}";', []), [(1, "checkout")])
        self.assertEqual(cmp.scan("a.sh", f"cd {BUILD}", []), [(1, "checkout")])
        self.assertEqual(cmp.scan("a.py", CHECKOUT.replace("\\", "\\\\"), []), [(1, "checkout")])

    def test_a_home_path_is_found_unless_its_name_is_a_placeholder(self):
        self.assertEqual(cmp.scan("a.py", HOME, []), [(1, "home")])
        self.assertEqual(cmp.scan("a.py", HOME.replace("jsmith", "Player"), []), [])
        self.assertEqual(cmp.scan("a.py", HOME.replace("jsmith", "<name>"), []), [])

    def test_a_home_path_in_msys_style_is_found_too(self):
        self.assertEqual(cmp.scan("a.sh", "/c/" + "Users/jsmith/private", []), [(1, "home")])
        self.assertEqual(cmp.scan("a.sh", "/c/" + "Users/Player/x", []), [])

    def test_a_folder_that_merely_contains_the_words_is_not_a_machine_path(self):
        self.assertEqual(cmp.scan("a.py", "my_hoi4_build_notes and the BattlePlan repo", []), [])
        self.assertEqual(cmp.scan("a.py", "$HOME/hoi4_build", []), [])

    def test_the_allow_file_exempts_one_kind_for_matching_files_only(self):
        rules = cmp.allowed("# why\ntools/runner*.py checkout\n\n")
        self.assertEqual(cmp.scan("tools/runner_a.py", CHECKOUT, rules), [])
        self.assertEqual(cmp.scan("tools/other.py", CHECKOUT, rules), [(1, "checkout")])
        self.assertEqual(cmp.scan("tools/runner_a.py", HOME, rules), [(1, "home")])


if __name__ == "__main__":
    unittest.main()
