from pathlib import Path

import a729_memory_reader
import live_a729_acceptance
from a729_layout import PROGRAM_SHA256, TEMPLATE_VTABLE, VARIANT_VTABLE


def test_pinned_layout_identity():
    assert PROGRAM_SHA256 == "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
    assert VARIANT_VTABLE == 0x14293AFB8
    assert TEMPLATE_VTABLE == 0x142938F70


def test_compatibility_entrypoint_delegates_to_live_capture(monkeypatch):
    sentinel = {"native": True}
    monkeypatch.setattr(live_a729_acceptance, "capture", lambda process: sentinel)
    assert a729_memory_reader.capture(object()) is sentinel


def test_synthetic_oracle_builder_is_removed():
    assert not hasattr(a729_memory_reader, "SparseMemory")
    assert not hasattr(a729_memory_reader, "FixturePredicates")
    assert not hasattr(a729_memory_reader, "build_oracle_fixture")
    source = Path(a729_memory_reader.__file__).read_text(encoding="utf-8")
    assert "definition_complete" not in source
    assert "--oracle" not in source


def test_capture_only_freezes_native_snapshot_without_oracle(monkeypatch, tmp_path):
    class Process:
        def __init__(self, pid, expected_name):
            assert pid == 123
            assert expected_name == "hoi4.exe"

        def __enter__(self):
            return self

        def __exit__(self, *_):
            return False

    snapshot = {
        "schema": "extractor.a27.live_heap_catalog.a729.v2",
        "variants": [{"id": 7}],
        "division_templates": [{"id": 8}],
    }
    monkeypatch.setattr(live_a729_acceptance.winmem, "Process", Process)
    monkeypatch.setattr(live_a729_acceptance, "capture", lambda process: snapshot)
    output = tmp_path / "owned" / "a27.json"
    assert live_a729_acceptance.main([
        "--capture-only", "--pid", "123", "--snapshot-out", str(output)
    ]) == 0
    assert output.exists()
    assert '"id": 7' in output.read_text(encoding="utf-8")
