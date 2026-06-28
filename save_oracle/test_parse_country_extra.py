#!/usr/bin/env python3
"""Focused tests for parse_country_extra.py.

These are synthetic melted-save snippets, so they do not need a local the game save.
"""
from __future__ import annotations

import os
import sys
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import parse_country_extra as pce  # noqa: E402


SYN = """
date=1940.9.10.13
version="Operation Postern v1.19.1.0"
player="GER"
countries={
    GER={
        flags={
            GER_has_started_war={ value=1 date=1939.9.2.1 }
            GER_congress_test={ value=3 date=1940.1.1.12 }
        }
        convoys_destroyed=7
    }
    ENG={
        flags={ }
    }
    SOV={
        convoys_destroyed=0
    }
}
"""


def _build():
    outputs, report = pce.build(SYN, Path("SYN_melted.txt"), False)
    return outputs["country_extra.json"], report


def test_country_script_flags_keep_value_and_date():
    out, report = _build()
    ger_flags = out["countries"]["GER"]["script_flags"]
    assert ger_flags["GER_has_started_war"] == {
        "value": 1, "date": "1939.9.2.1"}
    assert ger_flags["GER_congress_test"] == {
        "value": 3, "date": "1940.1.1.12"}
    assert out["meta"]["stats"]["script_flags"] == 2
    assert "GER flags     : 2" in report


def test_empty_flags_block_is_measured_empty_but_absent_block_is_omitted():
    out, _ = _build()
    assert out["countries"]["ENG"]["script_flags"] == {}
    assert "script_flags" not in out["countries"]["SOV"]
    assert out["meta"]["stats"]["countries_with_script_flags_block"] == 2


def test_no_path_leak():
    out, report = _build()
    blob = str(out) + report
    win_home = "C:" + chr(92) + "Users"
    cloud_docs = "One" + "Drive"
    msys_home = "/c/" + "Users"
    for bad in (win_home, cloud_docs, msys_home):
        assert bad not in blob
