from __future__ import annotations

import json
from pathlib import Path

import validate


def test_missing_required_output_fails_closed(tmp_path: Path):
    report = validate.Report()
    assert validate.load(tmp_path, "world.json", report) is None
    assert report.rows == [("FAIL", "load world.json", "missing required oracle output")]


def test_parser_debug_override_only_demotes_missing_output(tmp_path: Path):
    report = validate.Report()
    assert validate.load(tmp_path, "world.json", report, allow_missing=True) is None
    assert report.rows[0][0] == "WARN"


def test_non_object_oracle_output_is_corrupt(tmp_path: Path):
    (tmp_path / "world.json").write_text(json.dumps([]), encoding="utf-8")
    report = validate.Report()
    assert validate.load(tmp_path, "world.json", report) is None
    assert report.rows == [
        ("FAIL", "load world.json", "invalid root: expected JSON object, got list")
    ]


def test_valid_object_loads(tmp_path: Path):
    expected = {"meta": {"source_filename": "fixture.hoi4"}, "world": {}}
    (tmp_path / "world.json").write_text(json.dumps(expected), encoding="utf-8")
    report = validate.Report()
    assert validate.load(tmp_path, "world.json", report) == expected
    assert report.rows == []
