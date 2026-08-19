#!/usr/bin/env python3
"""Tests for parse_doctrine.py.

The real audit melts are not checked into this repo, so these tests drive a
synthetic doctrine manager block and assert the fail-closed contract rather than
claiming a real-world nested shape.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import parse_doctrine as pd


SYN = """
date=1940.9.10.13
version="Operation Postern v1.19.1.0"
player="GER"
doctrine={
\tcountries={
\t\tGER={
\t\t\tcompleted={ superior_firepower mass_assault unknown_fake_doctrine }
\t\t\tin_progress={
\t\t\t\tair_subdoctrine_aerial_reconnaissance={ progress=12.5 }
\t\t\t}
\t\t\tfolder_state={
\t\t\t\tland={ selected=superior_firepower }
\t\t\t}
\t\t}
\t\t39={
\t\t\tcompleted={ mass_assault }
\t\t}
\t}
}
"""


def _build():
    outputs, report = pd.build(SYN, Path("SYN_melted.txt"), False)
    return outputs["doctrine.json"], report


def test_doctrine_parser_preserves_raw_and_derives_known_ids_only():
    out, _ = _build()
    ger = out["countries"]["GER"]
    completed = {r["id"] for r in ger["completed"]}
    assert "superior_firepower" in completed
    assert "mass_assault" in completed
    assert "unknown_fake_doctrine" not in completed
    assert ger["completed_count"] == 2
    assert ger["in_progress"][0]["progress"] == 12.5
    assert ger["raw"]["folder_state"]["land"]["selected"] == "superior_firepower"


def test_doctrine_parser_keeps_non_tag_keys_separate():
    out, _ = _build()
    assert "39" not in out["countries"]
    assert "39" in out["countries_by_key"]
    assert out["countries_by_key"]["39"]["completed"][0]["id"] == "mass_assault"


def test_doctrine_report_and_meta_are_honest():
    out, report = _build()
    meta = out["meta"]
    assert meta["static_doctrine_catalog_loaded"] is True
    assert meta["stats"]["tag_countries_with_doctrine"] == 1
    assert meta["stats"]["non_tag_country_keys"] == 1
    assert "derived completed" in report
