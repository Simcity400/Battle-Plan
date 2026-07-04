#!/usr/bin/env python3
"""Hermetic tests for the E11/N4 RESOLVED JOIN tables added to the
``localisation_join`` static artifact (``localisation_gate.build_resolved_joins``).

Writes a self-contained fixture install (game-def files + english .yml loc) into
a tmp dir with real ``Layer`` roots, so ``resolver`` reads them off disk exactly
like a real vanilla+mod set — no the game install needed. Pins:

  * focus_id -> resolved title (parse_focus_def loc-join)
  * state_id -> STATE_<id> name; victory-point provid -> VICTORY_POINTS_<id> name
  * country TAG -> best-resolving name; cosmetic-tag -> name (cosmetic.txt)
  * mod-over-vanilla override (later layer wins) for a loc name
  * the NO-FABRICATION rule: an id whose best loc candidate is absent is OMITTED
  * dynamic (D01..) country tags are runtime-required, not in the static join
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

from extract import Layer  # noqa: E402
import parse_localisation  # noqa: E402
import localisation_gate  # noqa: E402


def _write(root: Path, rel: str, text: str) -> None:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    # BOM prefix matches how the engine (and the loc parser) reads .yml files.
    path.write_bytes(("﻿" + text).encode("utf-8") if rel.endswith(".yml") else text.encode("utf-8"))


def _make_install(tmp_path):
    vanilla = tmp_path / "vanilla"
    mod = tmp_path / "mymod"

    # --- game-def files -----------------------------------------------------
    _write(vanilla, "common/ideologies/00_ideologies.txt", """
ideologies = {
  democratic = { types = { liberalism = {} } }
  fascism = { types = { fascism_ideology = {} } }
}
""")
    _write(vanilla, "common/national_focus/focus.txt", """
focus_tree = {
  id = fixture_tree
  focus = { id = focus_alpha }
  focus = { id = focus_no_loc }
}
""")
    _write(vanilla, "history/states/1-One.txt", "state = { id = 1 }\n")
    _write(vanilla, "history/states/2-Two.txt", "state = { id = 2 }\n")
    _write(vanilla, "common/country_tags/00_countries.txt", """
GER = "countries/GER.txt"
FRA = "countries/FRA.txt"
NON = "countries/NON.txt"
""")
    _write(vanilla, "common/country_tags/zz_dynamic.txt", """
dynamic_tags = yes
D01 = "countries/D01.txt"
""")
    _write(vanilla, "common/countries/cosmetic.txt", """
GER_reich = {
    color = rgb { 1 2 3 }
}
FRA_commune = {
    color = rgb { 4 5 6 }
}
COS_no_loc = {
    color = rgb { 7 8 9 }
}
""")

    # --- english localisation ----------------------------------------------
    _write(vanilla, "localisation/english/focus_l_english.yml", """l_english:
 focus_alpha:0 "Alpha Focus"
""")
    _write(vanilla, "localisation/english/states_l_english.yml", """l_english:
 STATE_1:0 "First State"
 STATE_2:0 "Second State"
 VICTORY_POINTS_11:0 "Alpha VP"
 VICTORY_POINTS_22:0 "Beta VP"
""")
    _write(vanilla, "localisation/english/countries_l_english.yml", """l_english:
 GER:0 "Germany"
 GER_reich:0 "German Reich"
 FRA_DEF:0 "The French Republic"
 FRA_commune:0 "French Commune"
""")
    # NON has no loc key at all; NON must be OMITTED (no fabrication).
    # COS_no_loc likewise omitted. FRA resolves via FRA_DEF (bare_def fallback).

    # Mod overrides Germany's display name (later layer wins).
    _write(mod, "localisation/english/override_l_english.yml", """l_english:
 GER:0 "Greater Germany"
""")

    return [Layer("vanilla", vanilla), Layer("mymod", mod)]


def _resolved_joins(tmp_path):
    layers = _make_install(tmp_path)
    loc_out = parse_localisation.parse_localisation(layers)
    return localisation_gate.build_resolved_joins(layers, loc_out)


def test_focus_state_vp_resolved_joins(tmp_path):
    rj = _resolved_joins(tmp_path)
    # focus: resolved title present; the no-loc focus is omitted (not faked).
    assert rj["focus"]["focus_alpha"] == "Alpha Focus"
    assert "focus_no_loc" not in rj["focus"]
    # state names (STATE_<id>).
    assert rj["state"]["1"] == "First State"
    assert rj["state"]["2"] == "Second State"
    # victory-point names (VICTORY_POINTS_<provid>).
    assert rj["victory_point"]["11"] == "Alpha VP"
    assert rj["victory_point"]["22"] == "Beta VP"
    assert rj["counts"]["focus"] == 1
    assert rj["counts"]["state"] == 2
    assert rj["counts"]["victory_point"] == 2


def test_country_and_cosmetic_tag_joins_with_fallback_and_override(tmp_path):
    rj = _resolved_joins(tmp_path)
    country = rj["country"]
    # GER resolves via the bare tag key, and the MOD override wins.
    assert country["tags"]["GER"] == "Greater Germany"
    # FRA has no bare key but resolves via the FRA_DEF fallback candidate.
    assert country["tags"]["FRA"] == "The French Republic"
    # NON has no loc candidate at all -> OMITTED (never guessed).
    assert "NON" not in country["tags"]
    # dynamic D01 is runtime-required -> not in the static join.
    assert "D01" not in country["tags"]
    # cosmetic tags resolve from their own loc keys.
    assert country["cosmetic_tags"]["GER_reich"] == "German Reich"
    assert country["cosmetic_tags"]["FRA_commune"] == "French Commune"
    # a cosmetic tag with no loc is omitted.
    assert "COS_no_loc" not in country["cosmetic_tags"]
    assert rj["counts"]["country_tags"] == 2
    assert rj["counts"]["cosmetic_tags"] == 2


def test_resolved_joins_schema_and_no_fabrication(tmp_path):
    rj = _resolved_joins(tmp_path)
    assert rj["schema"] == "static_localisation_join_r3.resolved_joins.v1"
    # Every emitted value is a real string; no empty/None fabricated names.
    for table in ("focus", "state", "victory_point"):
        for key, name in rj[table].items():
            assert isinstance(name, str) and name, (table, key, name)
    for table in ("tags", "cosmetic_tags"):
        for key, name in rj["country"][table].items():
            assert isinstance(name, str) and name, (table, key, name)


def test_catalog_embeds_resolved_joins_additively(tmp_path):
    # build_catalog must keep `keys` and `coverage` AND add `resolved_joins`
    # (additive, open-object) — but build_coverage_report needs the wider
    # fixture set; here we assert the additive shape at the build_resolved_joins
    # level and that the join note documents the flat cosmetic lookup contract.
    rj = _resolved_joins(tmp_path)
    assert "cosmetic_tags[active_cosmetic_tag]" in rj["join_note"]
    assert set(rj["country"].keys()) == {"tags", "cosmetic_tags"}
    assert set(rj["counts"].keys()) == {
        "focus",
        "state",
        "victory_point",
        "country_tags",
        "cosmetic_tags",
    }
