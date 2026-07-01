#!/usr/bin/env python3
"""Focused tests for parse_world.py."""
from __future__ import annotations

import os
import sys
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import parse_world as pw  # noqa: E402


SYN_EMPTY = """
date=1936.1.1.12
version="Operation Postern v1.19.1.0"
player="GER"
difficulty="normal"
start_date=1936.1.1.12
gameplaysettings={ difficulty=normal historical=1 }
difficulty_settings={ }
game_rules={ }
"""

SYN_NONEMPTY = """
date=1936.1.1.12
difficulty="hard"
difficulty_settings={ GER=1 }
"""

SYN_ABSENT = """
date=1936.1.1.12
difficulty="normal"
game_rules={ }
"""


def _world(text):
    outputs, report = pw.build(text, Path("SYN_melted.txt"), False)
    return outputs["world.json"], report


def test_difficulty_settings_empty_block_is_recorded():
    out, report = _world(SYN_EMPTY)
    scenario = out["scenario"]
    assert scenario["difficulty"] == "normal"
    assert scenario["difficulty_settings_empty"] is True
    assert scenario["game_rules_empty"] is True
    assert "difficulty_settings: empty=True" in report


def test_difficulty_settings_nonempty_block_is_not_claimed_empty():
    out, _ = _world(SYN_NONEMPTY)
    assert out["scenario"]["difficulty"] == "hard"
    assert out["scenario"]["difficulty_settings_empty"] is False


def test_absent_difficulty_settings_stays_unknown_not_false():
    out, _ = _world(SYN_ABSENT)
    assert out["scenario"]["difficulty_settings_empty"] is None


def test_no_path_leak():
    out, report = _world(SYN_EMPTY)
    blob = str(out) + report
    win_home = "C:" + chr(92) + "Users"
    cloud_docs = "One" + "Drive"
    msys_home = "/c/" + "Users"
    for bad in (win_home, cloud_docs, msys_home):
        assert bad not in blob
