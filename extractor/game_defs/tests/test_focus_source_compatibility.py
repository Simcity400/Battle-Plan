from __future__ import annotations

import hashlib
import sys
from pathlib import Path

import pytest


HERE = Path(__file__).resolve().parent
DEFS = HERE.parent
ASSETS = DEFS.parents[1] / "replay_interface" / "game_assets"
sys.path[:0] = [str(DEFS), str(ASSETS)]

import parse_focus_def as focus  # noqa: E402


def test_exact_trailing_root_closer_compatibility_is_byte_bound(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    rel = "common/national_focus/exact_fixture.txt"
    raw = b"joint_focus = { id = exact_fixture }\n}\n"
    path = tmp_path / "exact_fixture.txt"
    path.write_bytes(raw)
    monkeypatch.setattr(
        focus,
        "_TRAILING_ROOT_CLOSER_COMPAT",
        {rel: hashlib.sha256(raw).hexdigest()},
    )

    assert (
        focus._validate_focus_source(raw.decode("utf-8"), rel, path)
        == "a729_exact_trailing_root_closer"
    )

    path.write_bytes(raw + b"# changed\n")
    with pytest.raises(ValueError, match="unmatched closing brace"):
        focus._validate_focus_source(path.read_text(encoding="utf-8"), rel, path)


def test_unregistered_trailing_root_closer_stays_rejected(tmp_path: Path) -> None:
    rel = "common/national_focus/unregistered.txt"
    path = tmp_path / "unregistered.txt"
    path.write_text("joint_focus = { id = unregistered }\n}\n", encoding="utf-8")

    with pytest.raises(ValueError, match="unmatched closing brace"):
        focus._validate_focus_source(path.read_text(encoding="utf-8"), rel, path)
