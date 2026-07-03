#!/usr/bin/env python3
"""Hermetic tests for ship static definitions and GFX manifest helpers."""

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
from paradox_parser import parse  # noqa: E402
import parse_ship_definitions  # noqa: E402
import gfx_icon_manifest_gate  # noqa: E402


def _write(root: Path, rel: str, text: str):
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def test_ship_hulls_modules_slots_and_repeated_blocks(tmp_path):
    vanilla = tmp_path / "vanilla"
    _write(vanilla, "common/units/equipment/ship_hull_light.txt", """
equipments = {
  ship_hull_light = {
    year = 1936
    is_archetype = yes
    is_buildable = no
    type = screen_ship
    interface_category = interface_category_screen_ships
    module_slots = {
      fixed_ship_engine_slot = {
        required = yes
        allowed_module_categories = { light_ship_engine }
      }
    }
    module_count_limit = {
      category = ship_radar
      count < 2
    }
    default_modules = {
      fixed_ship_engine_slot = light_ship_engine_1
    }
    naval_speed = 32
    build_cost_ic = 400
  }
  destroyer_1 = {
    archetype = ship_hull_light
    module_slots = inherit
    default_modules = inherit
  }
}
""")
    _write(vanilla, "common/units/equipment/modules/naval_modules.txt", """
equipment_modules = {
  limit = { has_dlc = "Man the Guns" }
  light_ship_engine_1 = {
    category = light_ship_engine
    abbreviation = "eng1"
    add_stats = { naval_speed = 5 build_cost_ic = 120 }
    can_convert_from = { module_category = light_ship_engine convert_cost_ic = 60 }
    can_convert_from = { module = light_ship_engine_0 convert_cost_ic = 40 }
  }
}
""")
    out = parse_ship_definitions.parse_ship_definitions([Layer("vanilla", vanilla)], strict=True)

    assert out["meta"]["ship_hull_count"] == 2
    hull = out["ship_hulls"]["ship_hull_light"]
    assert hull["module_slots"]["slots"][0]["id"] == "fixed_ship_engine_slot"
    assert hull["module_slots"]["slots"][0]["required"] is True
    assert hull["default_modules"]["fixed_ship_engine_slot"] == "light_ship_engine_1"
    assert hull["stats"]["naval_speed"] == "32"
    assert out["ship_hulls"]["destroyer_1"]["module_slots"]["inherits"] is True
    assert "resources" not in out["ship_hulls"]["destroyer_1"]
    assert "upgrades" not in out["ship_hulls"]["destroyer_1"]

    module = out["ship_modules"]["light_ship_engine_1"]
    assert module["category"] == "light_ship_engine"
    assert module["add_stats"]["naval_speed"] == "5"
    assert len(module["can_convert_from"]) == 2
    assert "multiply_stats" not in module
    assert out["meta"]["module_files_parsed"] == out["meta"]["module_files_total"]


def test_gfx_sprite_blocks_and_category_classification():
    root = parse("""
spriteTypes = {
  SpriteType = {
    name = "GFX_decision_fixture"
    texturefile = "gfx/interface/decisions/fixture.dds"
    noOfFrames = 2
  }
  spriteType = {
    name = "GFX_goal_fixture"
    texturefile = "gfx/interface/goals/fixture.dds"
  }
}
""")
    blocks = list(gfx_icon_manifest_gate._sprite_blocks(root))
    assert [block.get("name") for block in blocks] == [
        "GFX_decision_fixture",
        "GFX_goal_fixture",
    ]
    assert gfx_icon_manifest_gate._classify("GFX_decision_fixture", "gfx/interface/x.dds") == "decision_icon"
    assert gfx_icon_manifest_gate._classify("GFX_goal_fixture", "gfx/interface/x.dds") == "focus_icon"
