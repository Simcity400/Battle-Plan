#!/usr/bin/env python3
"""Hermetic tests for ship static definitions and GFX manifest helpers."""

from __future__ import annotations

import json
import sys
from pathlib import Path
from types import SimpleNamespace

_HERE = Path(__file__).resolve().parent
_DEFS = _HERE.parent
_REPO = _DEFS.parent.parent
_GAME_ASSETS = _REPO / "replay_interface" / "game_assets"
for _p in (str(_GAME_ASSETS), str(_DEFS)):
    if _p not in sys.path:
        sys.path.insert(0, _p)

from extract import Layer  # noqa: E402
from paradox_parser import parse  # noqa: E402
from static_artifact_common import sha256_bytes  # noqa: E402
import pytest  # noqa: E402
import parse_ship_definitions  # noqa: E402
import gfx_icon_manifest_gate  # noqa: E402
import build_all  # noqa: E402


def _write(root: Path, rel: str, text: str):
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def _write_bytes(root: Path, rel: str, data: bytes):
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def _allowlist(root: Path, entries=None) -> Path:
    path = root / "allowlist.json"
    path.write_text(
        json.dumps(
            {
                "schema": "gfx_icon_asset_manifest_static.missing_texture_allowlist.v1",
                "entries": entries or [],
            },
            indent=2,
        ),
        encoding="utf-8",
    )
    return path


def _gfx_args(vanilla: Path, userdata: Path, allowlist: Path) -> SimpleNamespace:
    return SimpleNamespace(
        vanilla=str(vanilla),
        mods=None,
        userdata=str(userdata),
        missing_texture_allowlist=allowlist,
        strict=True,
    )


def _patch_layers(monkeypatch, vanilla: Path, mod: Path | None = None):
    power_balance_gui = vanilla / "interface/powerbalanceview.gui"
    if not power_balance_gui.exists():
        _write(vanilla, "interface/powerbalanceview.gui", """
guiTypes = {
  iconType = { name = range_bar spriteType = GFX_range }
  iconType = { name = range_indicator spriteType = GFX_indicator }
  containerWindowType = {
    name = powerbalanceview
    background = { name = background quadTextureSprite = GFX_panel }
    iconType = { name = bop_bg quadTextureSprite = GFX_art }
    containerWindowType = {
      name = decision_container
      background = { name = Background spriteType = GFX_decision_frame }
    }
    iconType = { name = power_balance_header_bg spriteType = GFX_header }
    buttonType = { name = close quadTextureSprite = GFX_close }
    iconType = { name = power_balance_frame spriteType = GFX_bar_frame }
    iconType = { name = power_balance_value }
    iconType = { name = position_marker spriteType = GFX_marker }
    iconType = { name = left_power_icon }
    iconType = { name = right_power_icon }
    background = { name = Background quadTextureSprite = GFX_decision_bg }
  }
}
""")
    country_gui = vanilla / "interface/countrypoliticsview.gui"
    if not country_gui.exists():
        _write(vanilla, "interface/countrypoliticsview.gui", """
guiTypes = {
  containerWindowType = {
    name = country_politics
    containerWindowType = {
      name = power_balance_button
      buttonType = { name = show_power_balance quadTextureSprite = GFX_launcher }
      iconType = { name = show_power_balance_glow quadTextureSprite = GFX_glow }
      iconType = { name = power_balance_ICON quadTextureSprite = GFX_placeholder }
    }
  }
}
""")

    def fake_build_layers(vanilla=None, mods=None, userdata=None, strict_mods=False):
        layers = [Layer("vanilla", Path(vanilla))]
        if mod is not None:
            layers.append(Layer("Fixture Mod", mod))
        return layers

    monkeypatch.setattr(gfx_icon_manifest_gate.R, "build_layers", fake_build_layers)


def _minimal_gate_catalog(*, sprites=1, parsed=1, total=1, unresolved=0, allowed=0, missing=0, entries=0, unused=0, warnings=None):
    return {
        "meta": {
            "sprites": sprites,
            "interface_gfx_files_parsed": parsed,
            "interface_gfx_files": total,
            "unresolved_textures": unresolved,
            "allowed_missing_textures": allowed,
            "missing_textures": missing,
            "missing_texture_allowlist": {"entries": entries},
            "unused_missing_texture_allowlist_entries": unused,
            "warnings": warnings or [],
        }
    }


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
  progressBarType = {
    name = "GFX_progress_fixture"
    textureFile1 = "gfx/interface/front.dds"
    textureFile2 = "gfx/interface/back.dds"
  }
  frameAnimatedSpriteType = {
    name = "GFX_animated_fixture"
    texturefile = "gfx/interface/animated.dds"
  }
}
""")
    blocks = list(gfx_icon_manifest_gate._sprite_blocks(root))
    assert [block.get("name") for block in blocks] == [
        "GFX_decision_fixture",
        "GFX_goal_fixture",
        "GFX_progress_fixture",
        "GFX_animated_fixture",
    ]
    assert gfx_icon_manifest_gate._classify("GFX_decision_fixture", "gfx/interface/x.dds") == "decision_icon"
    assert gfx_icon_manifest_gate._classify("GFX_goal_fixture", "gfx/interface/x.dds") == "focus_icon"


def test_gfx_manifest_preserves_all_progress_bar_texture_slots(tmp_path, monkeypatch):
    vanilla = tmp_path / "vanilla"
    userdata = tmp_path / "userdata"
    userdata.mkdir()
    _write(userdata, "dlc_load.json", '{"enabled_mods":[],"disabled_dlcs":[]}')
    _write(vanilla, "interface/powerbalanceview.gfx", """
spriteTypes = {
  progressBarType = {
    name = "GFX_power_balance_left"
    textureFile1 = "gfx/interface/bop/front.dds"
    textureFile2 = "gfx/interface/absolute_nothing.dds"
    effectFile = "gfx/FX/progress_minmax.lua"
  }
}
""")
    _write_bytes(vanilla, "gfx/interface/bop/front.dds", b"front")
    _write_bytes(vanilla, "gfx/interface/absolute_nothing.dds", b"back")
    _patch_layers(monkeypatch, vanilla, None)

    catalog = gfx_icon_manifest_gate.build_catalog(
        _gfx_args(vanilla, userdata, _allowlist(tmp_path))
    )
    row = catalog["sprites"]["GFX_power_balance_left"]
    assert row["drawable_kind"] == "progressBarType"
    assert [slot["slot"] for slot in row["texture_slots"]] == [
        "textureFile1",
        "textureFile2",
    ]
    assert [slot["texturefile"] for slot in row["texture_slots"]] == [
        "gfx/interface/bop/front.dds",
        "gfx/interface/absolute_nothing.dds",
    ]
    assert all(slot["texture_sha256"] for slot in row["texture_slots"])


def test_gfx_manifest_derives_power_balance_roles_from_effective_mod_gui(
    tmp_path, monkeypatch
):
    vanilla = tmp_path / "vanilla"
    mod = tmp_path / "mod"
    userdata = tmp_path / "userdata"
    userdata.mkdir()
    _write(userdata, "dlc_load.json", '{"enabled_mods":[],"disabled_dlcs":[]}')
    _write(vanilla, "interface/base.gfx", """
spriteTypes = { spriteType = { name = GFX_fixture texturefile = "gfx/interface/x.dds" } }
""")
    _write_bytes(vanilla, "gfx/interface/x.dds", b"x")
    _write(mod, "interface/powerbalanceview.gui", """
guiTypes = {
  iconType = { name = range_bar spriteType = GFX_mod_range }
  iconType = { name = range_indicator spriteType = GFX_mod_indicator }
  containerWindowType = {
    name = powerbalanceview
    background = { name = background quadTextureSprite = GFX_mod_panel }
    iconType = { name = bop_bg quadTextureSprite = GFX_mod_art }
    containerWindowType = { name = decision_container background = { name = Background spriteType = GFX_mod_frame } }
    iconType = { name = power_balance_header_bg spriteType = GFX_mod_header }
    buttonType = { name = close quadTextureSprite = GFX_mod_close }
    iconType = { name = power_balance_frame spriteType = GFX_mod_bar_frame }
    iconType = { name = power_balance_value }
    iconType = { name = position_marker spriteType = GFX_mod_marker }
    iconType = { name = left_power_icon }
    iconType = { name = right_power_icon }
    background = { name = Background quadTextureSprite = GFX_mod_decision_bg }
  }
}
""")
    _patch_layers(monkeypatch, vanilla, mod)

    catalog = gfx_icon_manifest_gate.build_catalog(
        _gfx_args(vanilla, userdata, _allowlist(tmp_path))
    )
    gui = catalog["power_balance_gui"]
    assert gui["schema"] == "gfx_icon_asset_manifest_static.power_balance_gui_roles.v2"
    assert gui["sources"]["interface/powerbalanceview.gui"]["source_layer"] == "Fixture Mod"
    panel = gui["roles"]["panel_background"]
    decision = gui["roles"]["decision_background"]
    assert panel["sprite_name"] == "GFX_mod_panel"
    assert panel["hierarchy_path"] == "powerbalanceview/background"
    assert panel["parent_hierarchy_path"] == "powerbalanceview"
    assert panel["widget_type"] == "background"
    assert panel["widget_type_ordinal"] == 0
    assert panel["hierarchy_match"] == "exact_case_sensitive"
    assert decision["sprite_name"] == "GFX_mod_decision_bg"
    assert decision["hierarchy_path"] == "powerbalanceview/Background"
    assert decision["widget_type_ordinal"] == 1
    assert gui["roles"]["balance_value"]["binding_source"] == "engine"
    assert "sprite_name" not in gui["roles"]["balance_value"]


def test_gfx_manifest_rejects_ambiguous_power_balance_widget_roles(
    tmp_path, monkeypatch
):
    vanilla = tmp_path / "vanilla"
    userdata = tmp_path / "userdata"
    userdata.mkdir()
    _write(userdata, "dlc_load.json", '{"enabled_mods":[],"disabled_dlcs":[]}')
    _patch_layers(monkeypatch, vanilla)
    with (vanilla / "interface/powerbalanceview.gui").open("a", encoding="utf-8") as stream:
        stream.write("\nguiTypes = { containerWindowType = { name = powerbalanceview background = { name = background quadTextureSprite = GFX_duplicate } } }\n")

    with pytest.raises(RuntimeError, match="panel_background has 2 matches"):
        gfx_icon_manifest_gate.build_catalog(
            _gfx_args(vanilla, userdata, _allowlist(tmp_path))
        )


def test_gfx_manifest_casefolds_sprite_identity_like_product_registry(
    tmp_path, monkeypatch
):
    vanilla = tmp_path / "vanilla"
    userdata = tmp_path / "userdata"
    userdata.mkdir()
    _write(userdata, "dlc_load.json", '{"enabled_mods":[],"disabled_dlcs":[]}')
    _write(vanilla, "interface/a.gfx", """
spriteTypes = {
  spriteType = {
    name = "GFX_Case"
    textureFile = "gfx/interface/first.dds"
  }
  spriteType = {
    name = "gfx_case"
    textureFile = "gfx/interface/last.dds"
  }
}
""")
    _write_bytes(vanilla, "gfx/interface/first.dds", b"first")
    _write_bytes(vanilla, "gfx/interface/last.dds", b"last")
    _patch_layers(monkeypatch, vanilla)

    catalog = gfx_icon_manifest_gate.build_catalog(
        _gfx_args(vanilla, userdata, _allowlist(tmp_path))
    )
    assert "GFX_Case" not in catalog["sprites"]
    assert catalog["sprites"]["gfx_case"]["texture_sha256"] == sha256_bytes(b"last")
    assert catalog["meta"]["id_collisions_last_wins"] == [{
        "id": "gfx_case",
        "first_id": "GFX_Case",
        "first": "interface/a.gfx",
        "override": "interface/a.gfx",
        "case_insensitive": True,
    }]


def test_gfx_asset_layers_include_dlc_and_mod_wins(tmp_path, monkeypatch):
    vanilla = tmp_path / "vanilla"
    mod = tmp_path / "mod"
    userdata = tmp_path / "userdata"
    userdata.mkdir()
    _write(userdata, "dlc_load.json", '{"enabled_mods":[],"disabled_dlcs":[]}')
    _write(vanilla, "interface/base.gfx", """
spriteTypes = {
  SpriteType = { name = "GFX_shared" texturefile = "gfx/interface/shared.dds" }
}
""")
    _write(vanilla, "integrated_dlc/dlc001/interface/integrated.gfx", """
spriteTypes = {
  SpriteType = { name = "GFX_integrated" texturefile = "gfx/interface/integrated.dds" }
}
""")
    _write(vanilla, "dlc/dlc010/interface/dlc.gfx", """
spriteTypes = {
  SpriteType = { name = "GFX_dlc" texturefile = "gfx/interface/dlc.dds" }
}
""")
    _write(mod, "interface/base.gfx", """
spriteTypes = {
  SpriteType = { name = "GFX_shared" texturefile = "gfx/interface/shared.dds" }
}
""")
    _write_bytes(vanilla, "gfx/interface/shared.dds", b"vanilla")
    _write_bytes(vanilla, "integrated_dlc/dlc001/gfx/interface/integrated.dds", b"integrated")
    _write_bytes(vanilla, "dlc/dlc010/gfx/interface/dlc.dds", b"dlc")
    _write_bytes(mod, "gfx/interface/shared.dds", b"mod")
    _patch_layers(monkeypatch, vanilla, mod)

    catalog = gfx_icon_manifest_gate.build_catalog(_gfx_args(vanilla, userdata, _allowlist(tmp_path)))

    assert [row["label"] for row in catalog["layers"]] == [
        "vanilla",
        "integrated_dlc/dlc001",
        "dlc/dlc010",
        "Fixture Mod",
    ]
    assert catalog["sprites"]["GFX_shared"]["source_layer"] == "Fixture Mod"
    assert catalog["sprites"]["GFX_shared"]["texture_layer"] == "Fixture Mod"
    assert catalog["sprites"]["GFX_shared"]["texture_sha256"] == sha256_bytes(b"mod")
    assert catalog["sprites"]["GFX_integrated"]["source_layer"] == "integrated_dlc/dlc001"
    assert catalog["sprites"]["GFX_dlc"]["source_layer"] == "dlc/dlc010"


def test_gfx_live_inventory_excludes_generated_dlc_descriptor_from_mod_layers(
    tmp_path, monkeypatch
):
    vanilla = tmp_path / "vanilla"
    userdata = tmp_path / "userdata"
    staged_dlc = tmp_path / "staged_dlc"
    vanilla.mkdir()
    userdata.mkdir()
    _write(userdata, "dlc_load.json", '{"enabled_mods":[],"disabled_dlcs":[]}')
    inventory_path = tmp_path / "source_inventory.json"
    inventory_path.write_text(
        json.dumps(
            {
                "inventory_sha256": "1" * 64,
                "namespaces": [
                    {
                        "binding": "base",
                        "kind": "base",
                        "id": "base",
                        "label": "vanilla",
                        "replace_paths": [],
                    }
                ],
            }
        ),
        encoding="utf-8",
    )
    monkeypatch.setattr(
        gfx_icon_manifest_gate.LR,
        "validate_source_inventory_document",
        lambda raw: raw,
    )

    def fake_build_layers(vanilla=None, mods=None, userdata=None, strict_mods=False):
        return [
            Layer("vanilla", Path(vanilla)),
            Layer("Rocket Launcher Unit Pack", staged_dlc),
        ]

    monkeypatch.setattr(gfx_icon_manifest_gate.R, "build_layers", fake_build_layers)
    layers, _meta = gfx_icon_manifest_gate.build_gfx_asset_layers(
        vanilla=str(vanilla),
        userdata=userdata,
        source_inventory_path=inventory_path,
    )

    assert [layer.label for layer in layers] == ["vanilla"]


def test_gfx_live_inventory_drives_exact_namespace_order_and_staged_archive_root(
    tmp_path, monkeypatch
):
    vanilla = tmp_path / "vanilla"
    userdata = tmp_path / "userdata"
    mod = tmp_path / "mod"
    staged = userdata / "_materialized_dlc/0004"
    for root in (
        vanilla,
        userdata,
        mod,
        staged,
        vanilla / "integrated_dlc/dlc023_man_the_guns",
        vanilla / "dlc/dlc014_wallpaper",
        vanilla / "dlc/dlc023_man_the_guns",
        vanilla / "dlc/dlc048_content_creator_pack_peace_for_our_time",
        vanilla / "dlc/m_product",
    ):
        root.mkdir(parents=True, exist_ok=True)
    _write(userdata, "dlc_load.json", '{"enabled_mods":[],"disabled_dlcs":[]}')
    _write(
        userdata,
        "static_mod_staging_manifest.json",
        json.dumps(
            {
                "schema": "extractor.runtime_static_mod_staging.v2",
                "dlc_archives": [
                    {
                        "dlc_id": "q_archive",
                        "content_path": str(staged),
                        "source_namespace_sha256": "9" * 64,
                    }
                ],
            }
        ),
    )
    namespaces = [
        {
            "binding": "base",
            "kind": "vanilla",
            "id": "vanilla",
            "label": "vanilla",
            "replace_paths": [],
            "content_sha256": "0" * 64,
        },
        {
            "binding": "integrated",
            "kind": "integrated_dlc",
            "id": "dlc023_man_the_guns",
            "label": "integrated_dlc/dlc023_man_the_guns",
            "replace_paths": [],
            "content_sha256": "1" * 64,
        },
        {
            "binding": "asset_only",
            "kind": "dlc",
            "id": "dlc014_wallpaper",
            "label": "dlc/dlc014_wallpaper",
            "replace_paths": [],
            "content_sha256": "2" * 64,
        },
        {
            "binding": "asset_only",
            "kind": "dlc",
            "id": "dlc023_man_the_guns",
            "label": "dlc/dlc023_man_the_guns",
            "replace_paths": [],
            "content_sha256": "5" * 64,
        },
        {
            "binding": "asset_only",
            "kind": "dlc",
            "id": "dlc048_content_creator_pack_peace_for_our_time",
            "label": "dlc/dlc048_content_creator_pack_peace_for_our_time",
            "replace_paths": [],
            "content_sha256": "6" * 64,
        },
        {
            "binding": "active_product",
            "kind": "dlc",
            "id": "q_archive",
            "label": "dlc/q_archive",
            "replace_paths": [],
            "content_sha256": "9" * 64,
        },
        {
            "binding": "active_product",
            "kind": "dlc",
            "id": "m_product",
            "label": "dlc/m_product",
            "replace_paths": [],
            "content_sha256": "3" * 64,
        },
        {
            "binding": "active_mod",
            "kind": "mod",
            "id": "fixture",
            "label": "Fixture Mod",
            "replace_paths": ["gfx/models"],
            "content_sha256": "4" * 64,
        },
    ]
    inventory_path = tmp_path / "source_inventory.json"
    inventory_path.write_text(
        json.dumps({"inventory_sha256": "f" * 64, "namespaces": namespaces}),
        encoding="utf-8",
    )
    monkeypatch.setattr(
        gfx_icon_manifest_gate.LR,
        "validate_source_inventory_document",
        lambda raw: raw,
    )

    def fake_build_layers(vanilla=None, mods=None, userdata=None, strict_mods=False):
        return [
            Layer("vanilla", Path(vanilla)),
            Layer("Generated DLC Descriptor", staged),
            Layer("Fixture Mod", mod, ["gfx/models"]),
        ]

    monkeypatch.setattr(gfx_icon_manifest_gate.R, "build_layers", fake_build_layers)
    layers, meta = gfx_icon_manifest_gate.build_gfx_asset_layers(
        vanilla=str(vanilla),
        userdata=userdata,
        source_inventory_path=inventory_path,
    )

    assert [layer.label for layer in layers] == [
        "vanilla",
        "integrated_dlc/dlc023_man_the_guns",
        "dlc/q_archive",
        "dlc/m_product",
        "Fixture Mod",
    ]
    assert layers[2].root == staged.resolve()
    assert layers[-1].replace_paths == ["gfx/models"]
    assert meta["source_inventory_sha256"] == "f" * 64
    assert meta["enabled_dlcs"] == ["q_archive", "m_product"]
    assert meta["excluded_asset_only_dlcs"] == [
        "dlc014_wallpaper",
        "dlc023_man_the_guns",
        "dlc048_content_creator_pack_peace_for_our_time",
    ]
    assert layers[0].meta["enabled_dlcs"] == ["q_archive", "m_product"]


def test_gfx_disabled_dlc_is_excluded(tmp_path, monkeypatch):
    vanilla = tmp_path / "vanilla"
    userdata = tmp_path / "userdata"
    userdata.mkdir()
    _write(
        userdata,
        "dlc_load.json",
        json.dumps({"enabled_mods": [], "disabled_dlcs": ["dlc/dlc020_disabled/dlc020.dlc"]}),
    )
    _write(vanilla, "dlc/dlc010_enabled/interface/enabled.gfx", """
spriteTypes = {
  SpriteType = { name = "GFX_enabled" texturefile = "gfx/interface/enabled.dds" }
}
""")
    _write(vanilla, "dlc/dlc020_disabled/interface/disabled.gfx", """
spriteTypes = {
  SpriteType = { name = "GFX_disabled" texturefile = "gfx/interface/disabled.dds" }
}
""")
    _write_bytes(vanilla, "dlc/dlc010_enabled/gfx/interface/enabled.dds", b"enabled")
    _write_bytes(vanilla, "dlc/dlc020_disabled/gfx/interface/disabled.dds", b"disabled")
    _patch_layers(monkeypatch, vanilla)

    catalog = gfx_icon_manifest_gate.build_catalog(_gfx_args(vanilla, userdata, _allowlist(tmp_path)))

    assert "GFX_enabled" in catalog["sprites"]
    assert "GFX_disabled" not in catalog["sprites"]
    assert "dlc020_disabled" in catalog["meta"]["asset_layer_model"]["disabled_dlcs"]
    assert [row["label"] for row in catalog["layers"]] == ["vanilla", "dlc/dlc010_enabled"]


def test_gfx_extension_fallback_records_real_path(tmp_path, monkeypatch):
    vanilla = tmp_path / "vanilla"
    userdata = tmp_path / "userdata"
    userdata.mkdir()
    _write(userdata, "dlc_load.json", '{"enabled_mods":[],"disabled_dlcs":[]}')
    _write(vanilla, "interface/base.gfx", """
spriteTypes = {
  SpriteType = { name = "GFX_checkbox" texturefile = "gfx/interface/checkbox.tga" }
}
""")
    _write_bytes(vanilla, "gfx/interface/checkbox.dds", b"dds-bytes")
    _patch_layers(monkeypatch, vanilla)

    catalog = gfx_icon_manifest_gate.build_catalog(_gfx_args(vanilla, userdata, _allowlist(tmp_path)))
    sprite = catalog["sprites"]["GFX_checkbox"]

    assert sprite["texturefile"] == "gfx/interface/checkbox.tga"
    assert sprite["resolved_texturefile"] == "gfx/interface/checkbox.dds"
    assert sprite["resolved_via"] == "extension_fallback"
    assert sprite["texture_sha256"] == sha256_bytes(b"dds-bytes")


def test_gfx_allowlisted_missing_counts_as_allowed_not_unresolved(tmp_path, monkeypatch):
    vanilla = tmp_path / "vanilla"
    userdata = tmp_path / "userdata"
    userdata.mkdir()
    _write(userdata, "dlc_load.json", '{"enabled_mods":[],"disabled_dlcs":[]}')
    _write(vanilla, "interface/base.gfx", """
spriteTypes = {
  SpriteType = { name = "GFX_missing" texturefile = "gfx/interface/missing.dds" }
}
""")
    allowlist = _allowlist(
        tmp_path,
        [
            {
                "sprite": "GFX_missing",
                "texturefile": "gfx/interface/missing.dds",
                "source_layer": "vanilla",
                "source_file": "interface/base.gfx",
                "reason": gfx_icon_manifest_gate.MISSING_TEXTURE_REASON,
            }
        ],
    )
    _patch_layers(monkeypatch, vanilla)

    catalog = gfx_icon_manifest_gate.build_catalog(_gfx_args(vanilla, userdata, allowlist))

    assert catalog["meta"]["missing_textures"] == 1
    assert catalog["meta"]["allowed_missing_textures"] == 1
    assert catalog["meta"]["unresolved_textures"] == 0
    assert catalog["meta"]["unused_missing_texture_allowlist_entries"] == 0
    assert gfx_icon_manifest_gate.catalog_gate_passed(catalog, min_sprites=1, store_verify_result={"passed": True})


def test_gfx_allowlist_match_requires_source_layer(tmp_path, monkeypatch):
    vanilla = tmp_path / "vanilla"
    mod = tmp_path / "mod"
    userdata = tmp_path / "userdata"
    userdata.mkdir()
    _write(userdata, "dlc_load.json", '{"enabled_mods":[],"disabled_dlcs":[]}')
    _write(vanilla, "interface/base.gfx", """
spriteTypes = {
  SpriteType = { name = "GFX_missing" texturefile = "gfx/interface/missing.dds" }
}
""")
    _write(mod, "interface/base.gfx", """
spriteTypes = {
  SpriteType = { name = "GFX_missing" texturefile = "gfx/interface/missing.dds" }
}
""")
    allowlist = _allowlist(
        tmp_path,
        [
            {
                "sprite": "GFX_missing",
                "texturefile": "gfx/interface/missing.dds",
                "source_layer": "vanilla",
                "source_file": "interface/base.gfx",
                "reason": gfx_icon_manifest_gate.MISSING_TEXTURE_REASON,
            }
        ],
    )
    _patch_layers(monkeypatch, vanilla, mod)

    catalog = gfx_icon_manifest_gate.build_catalog(_gfx_args(vanilla, userdata, allowlist))

    assert catalog["unresolved_textures"][0]["source_layer"] == "Fixture Mod"
    assert catalog["meta"]["allowed_missing_textures"] == 0
    assert catalog["meta"]["unresolved_textures"] == 1
    assert catalog["meta"]["unused_missing_texture_allowlist_entries"] == 1


def test_gfx_allowlist_duplicate_full_identity_rejected(tmp_path):
    duplicate = {
        "sprite": "GFX_missing",
        "texturefile": "gfx/interface/missing.dds",
        "source_layer": "vanilla",
        "source_file": "interface/base.gfx",
        "reason": gfx_icon_manifest_gate.MISSING_TEXTURE_REASON,
    }
    allowlist = _allowlist(tmp_path, [duplicate, dict(duplicate)])

    with pytest.raises(RuntimeError, match="duplicate missing texture allowlist identity"):
        gfx_icon_manifest_gate._load_missing_texture_allowlist(allowlist)


def test_gfx_invalid_absolute_texture_ref_is_unresolved_with_redacted_raw(tmp_path, monkeypatch):
    vanilla = tmp_path / "vanilla"
    userdata = tmp_path / "userdata"
    userdata.mkdir()
    _write(userdata, "dlc_load.json", '{"enabled_mods":[],"disabled_dlcs":[]}')
    _write(vanilla, "interface/base.gfx", """
spriteTypes = {
  SpriteType = { name = "GFX_bad_absolute" texturefile = "F:/bad/path/foo.dds" }
}
""")
    _patch_layers(monkeypatch, vanilla)

    catalog = gfx_icon_manifest_gate.build_catalog(_gfx_args(vanilla, userdata, _allowlist(tmp_path)))
    missing = catalog["unresolved_textures"][0]

    assert catalog["meta"]["missing_textures"] == 1
    assert catalog["meta"]["unresolved_textures"] == 1
    assert missing["texturefile"] == "<absolute>/foo.dds"
    assert missing["raw_texturefile"] == "<absolute>/foo.dds"
    assert missing["invalid_texturefile"] is True
    assert catalog["sprites"]["GFX_bad_absolute"]["raw"]["texturefile"] == "<absolute>/foo.dds"


def test_gfx_recoverable_absolute_texture_ref_redacts_raw_block(tmp_path, monkeypatch):
    vanilla = tmp_path / "vanilla"
    userdata = tmp_path / "userdata"
    userdata.mkdir()
    _write(userdata, "dlc_load.json", '{"enabled_mods":[],"disabled_dlcs":[]}')
    _write(vanilla, "gfx/interface/foo.dds", "dds-bytes")
    _write(vanilla, "interface/base.gfx", """
spriteTypes = {
  SpriteType = { name = "GFX_recovered_absolute" texturefile = "F:/bad/path/gfx/interface/foo.dds" }
}
""")
    _patch_layers(monkeypatch, vanilla)

    catalog = gfx_icon_manifest_gate.build_catalog(_gfx_args(vanilla, userdata, _allowlist(tmp_path)))
    row = catalog["sprites"]["GFX_recovered_absolute"]

    # The game-relative path is recovered and resolves...
    assert row["texturefile"] == "gfx/interface/foo.dds"
    assert row["texture_sha256"]
    assert catalog["meta"]["unresolved_textures"] == 0
    # ...but the absolute original may not survive anywhere in the catalog.
    assert row["raw"]["texturefile"] == "<absolute>/foo.dds"
    assert row["raw_texturefile"] == "<absolute>/foo.dds"
    assert "F:/bad" not in json.dumps(catalog)


def test_gfx_gate_shared_checks_reject_zero_sprite_catalog():
    catalog = _minimal_gate_catalog(sprites=0)

    checks = gfx_icon_manifest_gate.catalog_gate_checks(catalog, min_sprites=1)

    assert checks["min_sprites"] is False
    assert not gfx_icon_manifest_gate.catalog_gate_passed(catalog, min_sprites=1)
    with pytest.raises(RuntimeError, match="min_sprites"):
        build_all._assert_gfx_gate_passed(catalog)


def test_gfx_gate_allows_unused_rows_from_inactive_dlc_selection():
    catalog = _minimal_gate_catalog(
        allowed=1,
        missing=1,
        entries=2,
        unused=1,
    )

    checks = gfx_icon_manifest_gate.catalog_gate_checks(catalog, min_sprites=1)

    assert checks["allowed_missing_textures"] is True
    assert gfx_icon_manifest_gate.catalog_gate_passed(catalog, min_sprites=1)
