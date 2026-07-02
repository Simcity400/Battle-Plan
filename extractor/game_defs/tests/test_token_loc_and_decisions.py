#!/usr/bin/env python3
"""Hermetic tests for idea/tech loc-key joins, decisions, and coverage."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

_HERE = Path(__file__).resolve().parent
_DEFS = _HERE.parent
_REPO = _DEFS.parent.parent
_GAME_ASSETS = _REPO / "replay_interface" / "game_assets"
for _p in (str(_GAME_ASSETS), str(_DEFS)):
    if _p not in sys.path:
        sys.path.insert(0, _p)

from extract import Layer  # noqa: E402
import parse_idea_tech_loc  # noqa: E402
import parse_decisions  # noqa: E402
import localisation_gate  # noqa: E402


def _write(root: Path, rel: str, text: str):
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def _make_layers(tmp_path):
    vanilla = tmp_path / "vanilla"
    mod = tmp_path / "mymod"

    _write(vanilla, "common/ideologies/00_ideologies.txt", """
ideologies = {
  democratic = { types = { liberalism = {} } }
  fascism = { types = { fascism_ideology = {} } }
}
""")

    _write(vanilla, "common/ideas/base.txt", """
ideas = {
  country = {
    base_idea = { picture = base_pic }
    explicit_idea = { name = explicit_idea_name picture = explicit_pic }
    literal_idea = { name = "Literal Idea" picture = literal_pic }
    quoted_known_key_idea = { name = "fixture_quoted_idea_key" picture = quoted_known_pic }
    quoted_missing_key_idea = { name = "fixture_quoted_missing_idea_key" picture = quoted_missing_pic }
    missing_explicit_key_idea = { name = missing_explicit_key picture = missing_pic }
    bad_idea = { name = { bad = yes } }
  }
}
""")
    _write(mod, "common/ideas/modded.txt", """
ideas = {
  country = {
    base_idea = { name = mod_base_idea_name picture = mod_pic }
  }
}
""")

    _write(vanilla, "common/technologies/base.txt", """
technologies = {
  infantry_weapons1 = {
    folder = { name = infantry_folder position = { x = 1 y = 2 } }
    categories = { infantry_weapons }
  }
  jet_engines = {
    desc = JET_ENGINES_SPECIAL
    categories = { electronics }
  }
  unlock_equipment_tech = {
    enable_equipments = { fixture_equipment_unlock }
  }
  unlock_module_tech = {
    enable_equipment_modules = { fixture_module_unlock }
  }
  tech_own_loc_wins = {
    enable_equipments = { z_equipment_own_fallback }
  }
  multi_enable_source_order_tech = {
    enable_equipments = { z_equipment_first_hit a_equipment_second_hit }
  }
  module_after_equipment_tech = {
    enable_equipments = { z_equipment_before_module }
    enable_equipment_modules = { a_module_after_equipment }
  }
  module_only_after_missing_equipment_tech = {
    enable_equipments = { missing_equipment_before_module }
    enable_equipment_modules = { module_only_hit }
  }
  bad_tech = { name = { bad = yes } }
}
""")
    _write(mod, "common/technologies/modded.txt", """
technologies = {
  infantry_weapons1 = {
    name = mod_infantry_title
    folder = { name = mod_folder position = { x = 3 y = 4 } }
    categories = { infantry_weapons support }
  }
}
""")

    _write(vanilla, "common/decisions/categories/00_categories.txt", """
political_actions = {
  icon = generic_political
  picture = GFX_decision_cat_picture_political
}
""")
    _write(vanilla, "common/decisions/base.txt", """
political_actions = {
  visible = { always = yes }
  base_decision = { icon = generic_decision }
  explicit_decision = { name = explicit_decision_name icon = explicit_icon }
  bad_decision = { name = { bad = yes } }
}
""")
    _write(mod, "common/decisions/modded.txt", """
political_actions = {
  base_decision = { name = mod_base_decision_name icon = mod_icon }
}
""")

    _write(vanilla, "common/national_focus/focus.txt", """
focus_tree = {
  id = fixture_tree
  focus = { id = focus_ok }
  focus = { id = focus_missing }
}
""")
    _write(vanilla, "common/units/equipment/equipment.txt", """
equipments = {
  equipment_ok = { is_archetype = yes }
  equipment_missing = { is_archetype = yes }
}
""")
    _write(vanilla, "common/characters/chars.txt", """
characters = {
  char_ok = { name = char_ok_name }
  char_literal = { name = "Literal Person" }
  char_quoted_single_token_literal = { name = "Alaverdov" }
  char_quoted_key_known = { name = "char_quoted_known_key" }
  char_missing_key = { name = char_missing_key_name }
  char_missing_name = { advisor = { idea_token = idea_token_only } }
}
""")
    _write(vanilla, "history/states/1-Test.txt", "state = { id = 1 }\n")
    _write(vanilla, "history/states/2-Missing.txt", "state = { id = 2 }\n")
    _write(vanilla, "common/country_tags/00_countries.txt", """
TAG = "countries/TAG.txt"
MIS = "countries/MIS.txt"
""")
    _write(vanilla, "common/country_tags/zz_dynamic_countries.txt", """
dynamic_tags = yes
D01 = "countries/D01.txt"
""")

    return [Layer("vanilla", vanilla), Layer("mymod", mod)]


def test_idea_tech_loc_keys_override_explicit_and_fail_closed(tmp_path):
    layers = _make_layers(tmp_path)
    out = parse_idea_tech_loc.parse_idea_tech_loc_keys(layers)

    base = out["tokens"]["ideas"]["base_idea"]
    assert base["loc_key"] == "mod_base_idea_name"
    assert base["explicit_name"] is True
    assert base["source_layer"] == "mymod"
    assert base["picture"] == "mod_pic"

    explicit = out["tokens"]["ideas"]["explicit_idea"]
    assert explicit["loc_key"] == "explicit_idea_name"
    assert explicit["category"] == "country"

    tech = out["tokens"]["technologies"]["infantry_weapons1"]
    assert tech["loc_key"] == "mod_infantry_title"
    assert tech["folders"] == ["mod_folder"]
    assert tech["categories"] == ["infantry_weapons", "support"]

    jet = out["tokens"]["technologies"]["jet_engines"]
    assert jet["loc_key"] == "jet_engines"
    assert jet["explicit_name"] is False
    assert jet["desc_loc_key"] == "JET_ENGINES_SPECIAL"

    literal = out["tokens"]["ideas"]["literal_idea"]
    assert literal["loc_key"] == "Literal Idea"
    assert literal["literal_name"] == "Literal Idea"
    assert literal["name_token_kind"] == "str"

    quoted_known = out["tokens"]["ideas"]["quoted_known_key_idea"]
    assert quoted_known["loc_key"] == "fixture_quoted_idea_key"
    assert quoted_known["literal_name"] is None
    assert quoted_known["name_token_kind"] == "str"

    quoted_missing = out["tokens"]["ideas"]["quoted_missing_key_idea"]
    assert quoted_missing["loc_key"] == "fixture_quoted_missing_idea_key"
    assert quoted_missing["literal_name"] is None
    assert quoted_missing["name_token_kind"] == "str"

    equip = out["tokens"]["technologies"]["unlock_equipment_tech"]
    assert equip["enable_equipments"] == ["fixture_equipment_unlock"]
    module = out["tokens"]["technologies"]["unlock_module_tech"]
    assert module["enable_equipment_modules"] == ["fixture_module_unlock"]
    multi = out["tokens"]["technologies"]["multi_enable_source_order_tech"]
    assert multi["enable_equipments"] == [
        "z_equipment_first_hit",
        "a_equipment_second_hit",
    ]
    module_after_equipment = out["tokens"]["technologies"]["module_after_equipment_tech"]
    assert module_after_equipment["enable_equipments"] == ["z_equipment_before_module"]
    assert module_after_equipment["enable_equipment_modules"] == ["a_module_after_equipment"]

    assert [r["id"] for r in out["omitted"]["ideas"]] == ["bad_idea"]
    assert [r["id"] for r in out["omitted"]["technologies"]] == ["bad_tech"]
    assert out["meta"]["idea_count"] == 7
    assert out["meta"]["tech_count"] == 9


def test_tech_equipment_loc_candidates_are_ordered_and_labeled(tmp_path):
    layers = _make_layers(tmp_path)
    out = parse_idea_tech_loc.parse_idea_tech_loc_keys(layers)
    items = {
        item["id"]: item
        for item in localisation_gate._items_from_records(out["tokens"]["technologies"])
    }

    own = localisation_gate._loc_candidates(items["tech_own_loc_wins"])
    assert own == [
        {"key": "tech_own_loc_wins", "rule": "id"},
        {"key": "z_equipment_own_fallback", "rule": "resolved_via_equipment_loc"},
    ]

    multi = localisation_gate._loc_candidates(items["multi_enable_source_order_tech"])
    assert multi == [
        {"key": "multi_enable_source_order_tech", "rule": "id"},
        {"key": "z_equipment_first_hit", "rule": "resolved_via_equipment_loc"},
        {"key": "a_equipment_second_hit", "rule": "resolved_via_equipment_loc"},
    ]

    module_after_equipment = localisation_gate._loc_candidates(items["module_after_equipment_tech"])
    assert module_after_equipment == [
        {"key": "module_after_equipment_tech", "rule": "id"},
        {"key": "z_equipment_before_module", "rule": "resolved_via_equipment_loc"},
        {"key": "a_module_after_equipment", "rule": "resolved_via_equipment_loc"},
    ]


def test_decisions_parser_categories_override_explicit_and_fail_closed(tmp_path):
    layers = _make_layers(tmp_path)
    out = parse_decisions.parse_decisions(layers)

    assert out["categories"]["political_actions"]["icon"] == "generic_political"
    base = out["decisions"]["base_decision"]
    assert base["category"] == "political_actions"
    assert base["loc_key"] == "mod_base_decision_name"
    assert base["icon"] == "mod_icon"
    assert base["source_layer"] == "mymod"

    explicit = out["decisions"]["explicit_decision"]
    assert explicit["loc_key"] == "explicit_decision_name"
    assert explicit["explicit_name"] is True

    assert "visible" not in out["decisions"]
    assert [r["id"] for r in out["omitted"]] == ["bad_decision"]
    assert out["meta"]["decision_count"] == 3


def test_localisation_coverage_reports_unresolvable_without_faking(tmp_path):
    layers = _make_layers(tmp_path)
    loc_keys = {
        "focus_ok": {},
        "mod_base_idea_name": {},
        "explicit_idea_name": {},
        "fixture_quoted_idea_key": {},
        "mod_infantry_title": {},
        "fixture_equipment_unlock": {},
        "fixture_module_unlock": {},
        "tech_own_loc_wins": {},
        "z_equipment_own_fallback": {},
        "z_equipment_first_hit": {},
        "a_equipment_second_hit": {},
        "z_equipment_before_module": {},
        "a_module_after_equipment": {},
        "module_only_hit": {},
        "mod_base_decision_name": {},
        "equipment_ok": {},
        "STATE_1": {},
        "char_ok_name": {},
        "char_quoted_known_key": {},
        "TAG_democratic": {},
        "TAG_DEF": {},
    }
    coverage = localisation_gate.build_coverage_report(layers, loc_keys)
    cats = coverage["categories"]

    assert cats["focus"]["total_seen"] == 2
    assert cats["focus"]["static_resolvable_total"] == 2
    assert cats["focus"]["resolved"] == 1
    assert cats["idea"]["total_seen"] == 7
    assert cats["idea"]["static_resolvable_total"] == 7
    assert cats["idea"]["resolved"] == 4
    assert cats["idea"]["resolved_by_literal"] == 1
    assert cats["idea"]["resolved_by_rule"]["explicit_name"] == 3
    assert cats["idea"]["quoted_key_missing_loc"] == 1
    assert cats["tech"]["total_seen"] == 9
    assert cats["tech"]["static_resolvable_total"] == 9
    assert cats["tech"]["resolved"] == 7
    assert cats["tech"]["resolved_by_rule"]["id"] == 1
    assert cats["tech"]["resolved_by_rule"]["resolved_via_equipment_loc"] == 5
    assert cats["decision"]["total_seen"] == 3
    assert cats["decision"]["resolved"] == 1
    assert cats["equipment"]["total_seen"] == 2
    assert cats["equipment"]["resolved"] == 1
    assert cats["state"]["total_seen"] == 2
    assert cats["state"]["resolved"] == 1
    assert cats["character"]["total_seen"] == 6
    assert cats["character"]["static_resolvable_total"] == 6
    assert cats["character"]["resolved"] == 3
    assert cats["character"]["resolved_by_literal"] == 1
    assert cats["character"]["quoted_key_missing_loc"] == 1
    assert cats["country_tag"]["total_seen"] == 3
    assert cats["country_tag"]["static_resolvable_total"] == 2
    assert cats["country_tag"]["resolved"] == 1
    assert cats["country_tag"]["unresolved"] == 1
    assert cats["country_tag"]["unresolvable"] == 1
    assert cats["country_tag"]["missing_localisation_key"] == 1
    assert cats["country_tag"]["runtime_required"] == 1
    assert cats["country_tag"]["resolved_by_rule"]["ideology_name"] == 1
    assert cats["country_tag"]["dynamic_runtime_required"] == 1
    assert cats["country_tag"]["loc_resolvable_percent"] == 50.0
    assert cats["country_tag"]["runtime_required_details"][0]["reason"] == (
        "dynamic_country_tag_requires_runtime_origin"
    )

    idea_unresolved = cats["idea"]["unresolvable_details"]
    assert idea_unresolved == [{
        "id": "bad_idea",
        "loc_key": None,
        "reason": "non_scalar_name",
        "source_layer": "vanilla",
        "source_file": "common/ideas/base.txt",
    }, {
        "id": "missing_explicit_key_idea",
        "loc_key": "missing_explicit_key",
        "reason": "missing_localisation_key",
        "source_layer": "vanilla",
        "source_file": "common/ideas/base.txt",
        "loc_candidates": [{"key": "missing_explicit_key", "rule": "explicit_name"}],
    }, {
        "id": "quoted_missing_key_idea",
        "loc_key": "fixture_quoted_missing_idea_key",
        "reason": "quoted_key_missing_loc",
        "source_layer": "vanilla",
        "source_file": "common/ideas/base.txt",
        "loc_candidates": [{"key": "fixture_quoted_missing_idea_key", "rule": "explicit_name"}],
    }]
    tech_ids = {r["id"] for r in cats["tech"]["unresolvable_details"]}
    assert tech_ids == {"bad_tech", "jet_engines"}
    character_reasons = {
        r["id"]: r["reason"]
        for r in cats["character"]["unresolvable_details"]
    }
    assert character_reasons["char_quoted_single_token_literal"] == "quoted_key_missing_loc"


def test_new_parser_outputs_are_deterministic(tmp_path):
    layers = _make_layers(tmp_path)
    first = {
        "idea_tech": parse_idea_tech_loc.parse_idea_tech_loc_keys(layers),
        "decisions": parse_decisions.parse_decisions(layers),
        "coverage": localisation_gate.build_coverage_report(layers, {"focus_ok": {}}),
    }
    second = {
        "idea_tech": parse_idea_tech_loc.parse_idea_tech_loc_keys(layers),
        "decisions": parse_decisions.parse_decisions(layers),
        "coverage": localisation_gate.build_coverage_report(layers, {"focus_ok": {}}),
    }
    assert json.dumps(first, sort_keys=True) == json.dumps(second, sort_keys=True)


def test_gate_report_redacts_paths_and_rejects_leaks():
    repo_path = localisation_gate.REPO_ROOT / "extractor" / "native" / "Cargo.toml"
    proof = {
        "paths": {
            "repo": localisation_gate.redact_path(repo_path),
            "external": localisation_gate.redact_path(
                r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV"
            ),
        },
        "cmd": localisation_gate._redact_cmd(["cargo", str(repo_path)]),
        "stderr": localisation_gate._redact_text(
            r"failed at C:\Users\Example\OneDrive\Documents\repo\file.rs "
            r"and X:\SteamLibrary\steamapps\common\Hearts of Iron IV"
        ),
    }
    text = json.dumps(proof, sort_keys=True)
    assert "extractor/native/Cargo.toml" in text
    assert "the game" in text
    assert "C:\\Users" not in text
    assert "OneDrive" not in text
    assert "SteamLibrary" not in text
    localisation_gate.assert_no_report_path_leaks(proof)

    with pytest.raises(AssertionError):
        localisation_gate.assert_no_report_path_leaks({"bad": r"C:\Users\Example\leak"})
