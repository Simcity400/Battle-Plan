#!/usr/bin/env python3
"""Hermetic tests for ``parse_localisation`` (R3 token -> localised-name join).

Feeds crafted localisation snippets straight through the parser, bypassing the
resolver's file-discovery (which needs a real the game install), so these ALWAYS
run — same hermetic pattern as ``test_clausewitz_edges.py``. They pin:

  * mod-over-vanilla override order (later-loaded layer wins, at KEY level)
  * §color-code stripping into ``name`` while ``raw`` stays verbatim
  * quote / escaped-quote handling
  * ``$VAR$`` / ``[Scope.Func]`` interpolation is left untouched, only flagged
  * the no-fabrication rule: a key absent from every layer is absent, not guessed
"""

from __future__ import annotations

import sys
from pathlib import Path

_HERE = Path(__file__).resolve().parent
_DEFS = _HERE.parent
_REPO = _DEFS.parent.parent
_GAME_ASSETS = _REPO / "replay_interface" / "game_assets"
for _p in (str(_GAME_ASSETS), str(_DEFS)):
    if _p not in sys.path:
        sys.path.insert(0, _p)

import parse_localisation as PL
from extract import Layer, build_file_map_layers


class _FakeExtractModule:
    """Swap in for ``resolver._ex`` so ``parse_localisation`` can be exercised
    against an in-memory file layout without touching a real the game install.
    Mirrors the two calls the parser actually uses: ``resolve_loc`` (file-level
    override, mod wins) and nothing else."""

    @staticmethod
    def resolve_loc(layers, language="english"):
        return build_file_map_layers(layers, f"localisation/{language}", (".yml",))


def _make_layers(tmp_path, files_by_layer: dict[str, dict[str, str]]) -> list[Layer]:
    """``files_by_layer``: {layer_label: {relative_loc_filename: file_text}},
    inserted in dict order (vanilla first, mods after — the caller controls
    load order exactly like the real vanilla-then-mod set)."""
    layers = []
    for label, files in files_by_layer.items():
        root = tmp_path / label
        loc_dir = root / "localisation" / "english"
        loc_dir.mkdir(parents=True, exist_ok=True)
        for name, text in files.items():
            fpath = loc_dir / name
            fpath.parent.mkdir(parents=True, exist_ok=True)
            fpath.write_bytes(("﻿" + text).encode("utf-8"))
        layers.append(Layer(label, root))
    return layers


def _parse(monkeypatch, layers):
    import resolver as R
    monkeypatch.setattr(R, "_ex", _FakeExtractModule)
    return PL.parse_localisation(layers)


def test_mod_overrides_vanilla_key(tmp_path, monkeypatch):
    layers = _make_layers(tmp_path, {
        "vanilla": {"a_l_english.yml": 'l_english:\n KEY_A:0 "Vanilla Value"\n'},
        "mymod": {"replace/a_override_l_english.yml": 'l_english:\n KEY_A:0 "Mod Value"\n'},
    })
    out = _parse(monkeypatch, layers)
    entry = out["keys"]["KEY_A"]
    assert entry["name"] == "Mod Value"
    assert entry["source_layer"] == "mymod"
    assert out["meta"]["overrides"] == 1
    assert out["meta"]["by_layer_override_keys"] == {"mymod": 1}


def test_same_named_file_fully_replaced_not_key_merged(tmp_path, monkeypatch):
    """When mod and vanilla ship a loc file at the IDENTICAL relpath, Clausewitz
    file-level override means the mod's file REPLACES vanilla's outright — a
    vanilla-only key in that same file is gone, not merged in. This is the
    engine's actual (if surprising) behavior for identically-named loc files,
    already encoded in ``build_file_map_layers``; this parser must not fight it."""
    layers = _make_layers(tmp_path, {
        "vanilla": {"a_l_english.yml": 'l_english:\n KEY_A:0 "Vanilla"\n KEY_VANILLA_ONLY:0 "Gone"\n'},
        "mymod": {"a_l_english.yml": 'l_english:\n KEY_A:0 "Mod"\n'},
    })
    out = _parse(monkeypatch, layers)
    assert out["keys"]["KEY_A"]["name"] == "Mod"
    assert "KEY_VANILLA_ONLY" not in out["keys"]


def test_vanilla_only_key_untouched_by_mod(tmp_path, monkeypatch):
    layers = _make_layers(tmp_path, {
        "vanilla": {"a_l_english.yml": 'l_english:\n KEY_ONLY_VANILLA:0 "Vanilla Only"\n'},
        "mymod": {"b_l_english.yml": 'l_english:\n KEY_OTHER:0 "Mod Only"\n'},
    })
    out = _parse(monkeypatch, layers)
    assert out["keys"]["KEY_ONLY_VANILLA"]["name"] == "Vanilla Only"
    assert out["keys"]["KEY_ONLY_VANILLA"]["source_layer"] == "vanilla"
    assert out["keys"]["KEY_OTHER"]["source_layer"] == "mymod"
    assert out["meta"]["overrides"] == 0


def test_override_across_different_filenames(tmp_path, monkeypatch):
    """Loc override is KEY-level, not file-level: a mod file with a totally
    different name still overrides the vanilla key."""
    layers = _make_layers(tmp_path, {
        "vanilla": {"focus_l_english.yml": 'l_english:\n SHARED_KEY:0 "Vanilla"\n'},
        "mymod": {"totally_different_l_english.yml": 'l_english:\n SHARED_KEY:0 "Mod"\n'},
    })
    out = _parse(monkeypatch, layers)
    assert out["keys"]["SHARED_KEY"]["name"] == "Mod"


def test_color_codes_stripped_from_name_but_kept_in_raw(tmp_path, monkeypatch):
    layers = _make_layers(tmp_path, {
        "vanilla": {"a_l_english.yml": 'l_english:\n COLORED:0 "§YHello§! World"\n'},
    })
    out = _parse(monkeypatch, layers)
    entry = out["keys"]["COLORED"]
    assert entry["name"] == "Hello World"
    assert entry["raw"] == "§YHello§! World"


def test_embedded_braces_and_hash_preserved_in_quoted_value(tmp_path, monkeypatch):
    layers = _make_layers(tmp_path, {
        "vanilla": {"a_l_english.yml": 'l_english:\n WEIRD:0 "a = b { c } # d"\n'},
    })
    out = _parse(monkeypatch, layers)
    assert out["keys"]["WEIRD"]["name"] == "a = b { c } # d"


def test_version_number_optional(tmp_path, monkeypatch):
    layers = _make_layers(tmp_path, {
        "vanilla": {"a_l_english.yml": 'l_english:\n NO_VERSION: "No Version Number"\n'},
    })
    out = _parse(monkeypatch, layers)
    assert out["keys"]["NO_VERSION"]["name"] == "No Version Number"


def test_var_and_scope_tokens_untouched_but_flagged(tmp_path, monkeypatch):
    layers = _make_layers(tmp_path, {
        "vanilla": {"a_l_english.yml": 'l_english:\n INTERP:0 "Hello $PLAYER$, see [GetName]"\n'},
    })
    out = _parse(monkeypatch, layers)
    entry = out["keys"]["INTERP"]
    assert entry["name"] == "Hello $PLAYER$, see [GetName]"
    assert entry["has_interpolation"] is True


def test_plain_text_not_flagged_as_interpolation(tmp_path, monkeypatch):
    layers = _make_layers(tmp_path, {
        "vanilla": {"a_l_english.yml": 'l_english:\n PLAIN:0 "Just plain text"\n'},
    })
    out = _parse(monkeypatch, layers)
    assert out["keys"]["PLAIN"]["has_interpolation"] is False


def test_missing_key_is_absent_not_guessed(tmp_path, monkeypatch):
    layers = _make_layers(tmp_path, {
        "vanilla": {"a_l_english.yml": 'l_english:\n KNOWN_KEY:0 "Known"\n'},
    })
    out = _parse(monkeypatch, layers)
    assert "NEVER_DEFINED_ANYWHERE" not in out["keys"]
    assert len(out["keys"]) == 1


def test_header_and_comment_lines_not_treated_as_keys(tmp_path, monkeypatch):
    text = (
        'l_english:\n'
        ' # a full-line comment\n'
        ' REAL_KEY:0 "Real Value" # trailing comment\n'
    )
    layers = _make_layers(tmp_path, {"vanilla": {"a_l_english.yml": text}})
    out = _parse(monkeypatch, layers)
    assert set(out["keys"]) == {"REAL_KEY"}
    assert out["keys"]["REAL_KEY"]["name"] == "Real Value"


def test_clean_name_helper_strips_color_only():
    assert PL.clean_name("§RRed§! plain") == "Red plain"
    assert PL.clean_name("no color here") == "no color here"


def test_has_interpolation_helper():
    assert PL.has_interpolation("$FOO$") is True
    assert PL.has_interpolation("[Root.GetName]") is True
    assert PL.has_interpolation("plain text") is False


if __name__ == "__main__":
    import pytest
    raise SystemExit(pytest.main([__file__, "-v"]))
