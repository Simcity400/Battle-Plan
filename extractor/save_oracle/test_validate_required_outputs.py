from __future__ import annotations

import json
from pathlib import Path

import validate
from output_contract import OUTPUTS_BY_PRODUCER, PRODUCERS_BY_OUTPUT, REQUIRED_OUTPUTS


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


def test_every_declared_parser_output_is_required():
    flattened = {
        name
        for names in OUTPUTS_BY_PRODUCER.values()
        for name in names
    }
    assert flattened == set(REQUIRED_OUTPUTS)
    assert len(REQUIRED_OUTPUTS) == sum(map(len, OUTPUTS_BY_PRODUCER.values()))
    assert all(len(producers) == 1 for producers in PRODUCERS_BY_OUTPUT.values())
    assert validate.ALL_FILES == REQUIRED_OUTPUTS
    # These were the material omissions in the old validator inventory.
    for name in (
        "projects.json", "intel.json", "weather.json", "designs.json",
        "campaign_identity_oracle.json", "abilities.json",
        "raid_instances.json",
    ):
        assert name in REQUIRED_OUTPUTS


def test_contract_accounts_for_every_parser_module_and_names_its_outputs():
    here = Path(__file__).resolve().parent
    parser_modules = {path.name for path in here.glob("parse_*.py")}
    self_describing_parsers = set(OUTPUTS_BY_PRODUCER)
    assert parser_modules == self_describing_parsers
    for producer, outputs in OUTPUTS_BY_PRODUCER.items():
        source = (here / producer).read_text(encoding="utf-8")
        for output in outputs:
            assert output in source, f"{producer} no longer declares {output}"


def test_newly_restored_output_is_required_by_default(tmp_path: Path):
    report = validate.Report()
    assert validate.load(tmp_path, "projects.json", report) is None
    assert report.rows == [
        ("FAIL", "load projects.json", "missing required oracle output")
    ]
