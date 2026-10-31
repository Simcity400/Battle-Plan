"""Tests for tools/daily_tests.py: python tools/daily_tests_test.py"""

import contextlib
import io
import os
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import daily_tests as dt  # noqa: E402


class GameDir(unittest.TestCase):
    def test_a_relative_game_dir_means_the_folder_it_names_where_it_is_given(self):
        previous = os.getcwd()
        with tempfile.TemporaryDirectory() as here:
            os.chdir(here)
            try:
                given = os.path.join(os.getcwd(), "game")
                with mock.patch.dict(os.environ, {"BATTLE_PLAN_GAME_DIR": ""}):
                    for command in ("install", "run"):
                        self.assertEqual(dt.parse([command, "--game-dir", "game"]).game_dir, given)
                with mock.patch.dict(os.environ, {"BATTLE_PLAN_GAME_DIR": "game"}):
                    self.assertEqual(dt.parse(["run"]).game_dir, given)
            finally:
                os.chdir(previous)

    def test_an_empty_variable_names_no_game_dir(self):
        with mock.patch.dict(os.environ, {"BATTLE_PLAN_GAME_DIR": ""}), \
                contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            dt.parse(["run"])


if __name__ == "__main__":
    unittest.main()
