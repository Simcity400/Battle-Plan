"""Exact current-build national-focus state-background registry gate."""

from pathlib import Path
import sys


GAME_DEFS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(GAME_DEFS))

import parse_focus_def


GAME = Path(r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV")


def test_current_focus_style_registry_is_complete():
    assert GAME.is_dir(), f"pinned game root is missing: {GAME}"
    result = parse_focus_def.build(vanilla=str(GAME), mods=[])
    meta = result["meta"]
    assert meta["focus_count"] == 10888
    assert meta["joint_focus_count"] == 280
    assert meta["focus_background_style_count"] == 22
    assert meta["default_focus_background_styles"] == ["default_style"]
    assert meta["unresolved_focus_background_styles"] == []

    styles = result["focus_background_styles"]
    assert all(
        all(style[state] for state in ("unavailable", "available", "current", "completed"))
        for style in styles.values()
    )
    assert not [
        focus["id"]
        for focus in result["focuses"].values()
        if focus["kind"] == "joint_focus" and not focus["text_icon"]
    ]

    gfx = (GAME / "interface/nationalfocusview.gfx").read_text(
        encoding="utf-8", errors="replace"
    )
    assert all(
        style[state] in gfx
        for style in styles.values()
        for state in ("unavailable", "available", "current", "completed")
    )

    overlay_tokens = [
        focus["overlay"]
        for focus in result["focuses"].values()
        if focus["overlay"]
    ]
    assert len(overlay_tokens) == 93
    assert len(set(overlay_tokens)) == 25
    all_gfx = "\n".join(
        path.read_text(encoding="utf-8", errors="replace")
        for path in (GAME / "interface").glob("*.gfx")
    )
    assert all(token in all_gfx for token in overlay_tokens)


def test_focus_text_declaration_replaces_the_title_localisation_key():
    """`focus = { id = X text = Y }` localises under Y, not X."""
    import resolver as R

    assert GAME.is_dir(), f"pinned game root is missing: {GAME}"
    layers = R.build_layers(vanilla=str(GAME), mods=[])
    loc = R.load_loc(layers)
    result = parse_focus_def.parse_focus_defs(layers, loc)
    focuses = result["focuses"]

    assert result["meta"]["focus_text_loc_key_overrides"] == [
        "CHI_the_soviet_volunteer_group",
        "CZE_join_comintern_pfot",
        "ETH_peoples_army",
        "PRC_found_the_academy_of_science",
        "USA_CAS",
        "uk_extra_tech_slot",
        "uk_industrial_focus",
    ]
    assert {
        focus_id: focuses[focus_id]["title_loc_key"]
        for focus_id in result["meta"]["focus_text_loc_key_overrides"]
    } == {
        "CHI_the_soviet_volunteer_group": "CHI_the_soviet_volunteer_group_focus",
        "CZE_join_comintern_pfot": "CZE_join_comintern",
        "ETH_peoples_army": "ETH_peoples_army_focus",
        "PRC_found_the_academy_of_science": "PRC_form_the_academy_of_sciences",
        "USA_CAS": "CAS_effort",
        "uk_extra_tech_slot": "extra_tech_slot",
        "uk_industrial_focus": "industrial_effort",
    }

    for focus in focuses.values():
        assert focus["description_loc_key"] == focus["title_loc_key"] + "_desc"
    # Every declared key resolves; the ids the old join used mostly do not.
    assert [
        focus_id
        for focus_id in result["meta"]["focus_text_loc_key_overrides"]
        if focuses[focus_id]["loc"]["title"] is None
    ] == []
    assert [
        focus_id
        for focus_id in result["meta"]["focus_text_loc_key_overrides"]
        if loc.resolve(focus_id) is None
    ] == [
        "CHI_the_soviet_volunteer_group",
        "CZE_join_comintern_pfot",
        "ETH_peoples_army",
        "USA_CAS",
        "uk_extra_tech_slot",
        "uk_industrial_focus",
    ]
    assert loc.resolve("PRC_found_the_academy_of_science") != loc.resolve(
        "PRC_form_the_academy_of_sciences"
    )
