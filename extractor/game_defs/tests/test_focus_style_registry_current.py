"""Exact current-build national-focus state-background registry gate."""

from pathlib import Path
import sys


GAME_DEFS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(GAME_DEFS))

import parse_focus_def  # noqa: E402


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
