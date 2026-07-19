#!/usr/bin/env python3
"""Self-tests for the game_defs static-def parsers.

Parses the REAL effective mod-over-vanilla set (vanilla from BATTLE_PLAN_VANILLA + the
AMSB-DEV mod via active_modset.json) and asserts each parser:
  * produces a non-empty catalog,
  * resolves a couple of KNOWN entries correctly,
  * proves mod-over-vanilla (an entry where the mod value wins).

Run:  BATTLE_PLAN_VANILLA=<install> python -m pytest game_defs/tests -q
or:   BATTLE_PLAN_VANILLA=<install> python game_defs/tests/test_game_defs.py
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

_HERE = Path(__file__).resolve().parent
_DEFS = _HERE.parent
sys.path.insert(0, str(_DEFS))

import pytest  # noqa: E402

import resolver as R          # noqa: E402
import parse_events           # noqa: E402
import parse_scripted_gui     # noqa: E402
import parse_tech             # noqa: E402
import parse_doctrines        # noqa: E402
import parse_catalogs         # noqa: E402
import parse_focus_def        # noqa: E402
import parse_templates        # noqa: E402
import parse_world_defs       # noqa: E402
import parse_characters       # noqa: E402
import parse_localisation     # noqa: E402


def _have_vanilla():
    try:
        R.vanilla_root()
        return True
    except Exception:
        return False


pytestmark = pytest.mark.skipif(
    not _have_vanilla(),
    reason="BATTLE_PLAN_VANILLA not set / install not found")


# Build the layer set + loc ONCE (parse of full set is non-trivial).
@pytest.fixture(scope="module")
def layers():
    return R.build_layers()


@pytest.fixture(scope="module")
def loc(layers):
    return R.load_loc(layers)


# --- resolver + mod-over-vanilla (KEY-level loc) ---------------------------
def test_resolver_layers(layers):
    labels = [ly.label for ly in layers]
    assert labels[0] == "vanilla"
    assert any("AMSB" in l for l in labels), labels


def test_loc_mod_override_wins(loc):
    # LOADING_TIP_0: vanilla = a Mussolini quote; the AMSB mod overrides it with a
    # joke tip. The mod (loaded after vanilla) MUST win at the key level.
    val = loc.resolve("LOADING_TIP_0")
    assert val, "LOADING_TIP_0 missing"
    assert "Mussolini" not in val and "humiliating" not in val, \
        f"vanilla loc leaked through, mod override failed: {val!r}"
    assert len(loc) > 50000, f"loc too small: {len(loc)}"


# --- events ---------------------------------------------------------------
def test_events(layers, loc):
    out = parse_events.parse_events(layers, loc)
    ev = out["events"]
    assert len(ev) > 2000, out["meta"]
    g = ev.get("generic.1")
    assert g and g["title"]["key"] == "generic.1.t"
    assert g["title"]["text"], "title loc unresolved"
    assert g["options"], "no options parsed"
    # mod overrides events/Germany.txt -> those events carry the mod source_file
    ger = [e for e in ev.values()
           if e["source_file"].lower() == "events/germany.txt"]
    assert ger, "no events from mod-overridden Germany.txt"


# --- scripted_gui element->variable ---------------------------------------
def test_scripted_gui(layers):
    out = parse_scripted_gui.parse_scripted_guis(layers)
    g = out["scripted_guis"]
    assert len(g) > 20, out["meta"]
    # the mod's topbar toggle: element_click writes resources_disabled, and the
    # container is visible-gated on it.
    tog = g.get("topbar_resource_toogle_window")
    assert tog and tog["source_file"].endswith("topbarresources.txt")
    assert "resources_disabled" in tog["writes"]
    eff = tog["effects"].get("toogle_button_click")
    assert eff and eff["on_click_of"] == "toogle_button"
    cont = g.get("topbar_resources")
    assert cont and "resources_disabled" in cont["visible_vars"]
    # [GET_X] dynamic-loc tokens classified
    we = g.get("war_escalation_decision_ui")
    assert we and we["dynamic_loc_calls"], "no dynamic loc calls captured"


# --- tech tree geometry + effects -----------------------------------------
def test_tech(layers):
    out = parse_tech.parse_technologies(layers)
    t = out["technologies"]
    assert len(t) > 300, out["meta"]
    # @var geometry resolves to a number (y=@1936 -> 2) AND keeps raw
    iw = t.get("infantry_weapons1")
    assert iw and iw["folder"]["y"] == 2 and iw["folder"]["y_raw"] == "@1936"
    # XOR mutual exclusion + modifier effect rows
    ci = t.get("concentrated_industry")
    assert ci and "dispersed_industry" in ci["xor"]
    assert "industrial_capacity_factory" in ci["modifiers"]
    assert not out["meta"]["unresolved_geometry_refs"], \
        out["meta"]["unresolved_geometry_refs"]


def test_tech_cost_present(layers):
    """Every tech that the tree DRAWS must carry a research cost; the only techs
    that may omit research_cost are the proven non-tree (hidden) flags."""
    out = parse_tech.parse_technologies(layers)
    t = out["technologies"]
    iw = t["infantry_weapons1"]
    assert iw["research_cost"] == 1.5, iw            # cost extracted
    # @var-fed cost still resolves; raw token kept
    assert iw["research_cost_raw"] == "1.5"
    # NO renderable (tree_visible) tech may be missing a cost.
    missing_cost_visible = [k for k, v in t.items()
                            if v["tree_visible"] and v["research_cost"] is None]
    assert not missing_cost_visible, missing_cost_visible
    # the techs that omit cost must all be proven hidden / non-tree
    for k in out["meta"]["no_research_cost"]:
        assert t[k]["tree_visible"] is False, (k, t[k])


def test_tech_every_node_renderable(layers):
    """557/557: every tech has either a resolved position (tree node) or is a
    PROVEN hidden/non-tree tech (position=null, tree_visible=False, with a
    documented reason). Zero techs are in an unresolved/ambiguous state."""
    out = parse_tech.parse_technologies(layers)
    t = out["technologies"]
    for k, v in t.items():
        # tree_visible must be a concrete bool (resolution ran for every tech)
        assert v["tree_visible"] in (True, False), (k, v["tree_visible"])
        if v["position"] is not None:
            # a positioned node carries x/y/source and is marked visible
            p = v["position"]
            assert p["x"] is not None and p["y"] is not None, (k, p)
            assert p["source"] in ("folder",) or p["source"].startswith("subtech_of:"), (k, p)
            assert v["tree_visible"] is True, k
        else:
            # no position -> MUST be flagged hidden with a proven reason
            assert v["tree_visible"] is False, k
            assert v.get("hidden_reason"), k
    # accounting closes: renderable + hidden == total, hidden_other == 0
    m = out["meta"]
    assert m["renderable"] + len(m["hidden_techs"]) == m["tech_count"], m
    assert m["position_stats"]["hidden_other"] == 0, m["position_stats"]
    # runtime_positioned (electronics/nuclear/SP engine-grid techs) is disjoint
    # from positioned nodes and only set on hidden ones (Codex R2 #2).
    for k, v in t.items():
        if v["position"] is not None:
            assert v["runtime_positioned"] is False, k
    rp = [k for k, v in t.items() if v["runtime_positioned"]]
    assert rp, "no runtime-positioned (secret-weapon/SP) techs flagged"
    assert all(t[k]["tree_visible"] is False for k in rp)
    # hidden-reason buckets are disjoint and sum to the hidden total (R2 #4)
    ps = m["position_stats"]
    hid_buckets = sum(v for kk, v in ps.items()
                      if kk.startswith("hidden_"))
    assert hid_buckets == ps["hidden"] == len(m["hidden_techs"]), ps
    # a known sub-technology inherits its parent's position (the 140-gap fix):
    # at least one tech is positioned via subtech inheritance.
    assert m["position_stats"]["subtech"] > 0, m["position_stats"]
    sub = next((v for v in t.values()
                if v["position"] and v["position"]["source"].startswith("subtech_of:")),
               None)
    assert sub, "no sub-tech inherited a parent position"
    assert sub["position"]["parent"] in t, sub["position"]
    parent_pos = t[sub["position"]["parent"]]["position"]
    # child sits at parent xy + slot offset (renderable relative position)
    assert sub["position"]["x"] == parent_pos["x"] + sub["position"]["slot_offset"]["dx"]
    assert sub["position"]["y"] == parent_pos["y"] + sub["position"]["slot_offset"]["dy"]


def test_tech_edges_and_deps(layers):
    """leads_to edges keep the per-path research_cost_coeff; dependencies emit."""
    out = parse_tech.parse_technologies(layers)
    t = out["technologies"]
    iw = t["infantry_weapons1"]
    edge = next((e for e in iw["leads_to_edges"]
                 if e["to"] == "infantry_weapons2"), None)
    assert edge and edge["research_cost_coeff"] is not None, iw["leads_to_edges"]
    # dependencies are now emitted (forced cross-tech prereqs)
    dep_tech = next((v for v in t.values() if v["dependencies"]), None)
    assert dep_tech, "no tech emitted a dependencies block"


# --- doctrines ------------------------------------------------------------
def test_doctrines(layers):
    out = parse_doctrines.parse_doctrines(layers)
    d = out["doctrines"]
    assert out["meta"]["by_kind"].get("subdoctrine", 0) > 50, out["meta"]
    # a known subdoctrine carries effect rows + its primary track
    sd = d.get("air_subdoctrine_fighter_homeland_defense")
    assert sd and sd["track"] == "fighter_aircraft"
    assert sd["effects"], "no doctrine effects"


def test_doctrine_positions_and_xor(layers):
    """Every doctrine node (subdoctrine/track/grand) has a derived grid position,
    the special-forces XOR pairs are parsed (not swept into effects), and the
    grand-doctrine one-of groups are detected."""
    out = parse_doctrines.parse_doctrines(layers)
    d = out["doctrines"]
    g = out["meta"]["geometry"]
    # 0 unpositioned subdoctrine/track/grand nodes
    assert g["unpositioned"] == [], g["unpositioned"]
    assert g["subdoctrines_positioned"] == g["subdoctrine_total"] > 50, g
    assert g["tracks_positioned"] == g["track_total"] > 5, g
    # every subdoctrine carries a concrete col/row + grid px
    for n, r in d.items():
        if r["kind"] == "subdoctrine":
            p = r["position"]
            assert p and p["col"] is not None and p["row"] is not None, (n, p)
            assert p["x"] is not None and p["y"] is not None, (n, p)
    # a sample subdoctrine resolves to its track column / row
    fhd = d["air_subdoctrine_fighter_homeland_defense"]
    assert fhd["position"]["track"] == "fighter_aircraft"
    assert fhd["position"]["folder"] == "air"
    # XOR: special-forces pairs are symmetric and OUT of effects
    m1 = d["mountaineers_1"]
    assert m1["xor"] == ["mountaineers_2"], m1["xor"]
    assert "xor" not in m1["effects"], "xor leaked into effects"
    assert d["mountaineers_2"]["xor"] == ["mountaineers_1"]
    # block-form track (special-forces nodes sit on two tracks) parses AND the
    # node is placed in BOTH track columns (no empty SF grid -- Codex R2 #3).
    assert "special_forces_first" in m1["tracks_member"]
    assert "special_forces_second" in m1["tracks_member"]
    assert m1["positions"] and len(m1["positions"]) == 2, m1["positions"]
    cols = {p["col"] for p in m1["positions"]}
    assert len(cols) == 2, cols       # one per track column
    assert g["multi_track_subdoctrines"] >= 8, g
    assert g["explicit_subdoctrine_xor"] >= 8, g
    # implicit grand-doctrine one-of groups (land has 4 mutually exclusive paths)
    land = g["grand_doctrine_xor_groups"].get("land")
    assert land and "mass_assault" in land and "superior_firepower" in land, land


def test_doctrine_mod_override(layers):
    """Mod-over-vanilla applies to doctrines too: the effective node set resolves
    from the layered file set (vanilla + AMSB). Sanity: a vanilla land grand
    doctrine is present and positioned (no mod removes it in this set)."""
    out = parse_doctrines.parse_doctrines(layers)
    d = out["doctrines"]
    ma = d.get("mass_assault")
    assert ma and ma["kind"] == "grand_doctrine" and ma["folder"] == "land"
    assert ma["position"] is not None and ma["tracks"], ma


# --- catalogs -------------------------------------------------------------
def test_catalogs(layers):
    out = parse_catalogs.parse_catalogs(layers)
    c = out["catalogs"]
    # occupation law + modifiers
    assert c["occupation_laws"].get("no_garrison", {}).get("state_modifier")
    # leader trait
    og = c["leader_traits"].get("old_guard")
    assert og and og["modifier"].get("max_dig_in") == "1"
    # equipment module add_stats
    eng = c["equipment_modules"].get("engine_1_1x")
    assert eng and eng["add_stats"].get("thrust") == "11"
    # combat tactic
    ta = c["combat_tactics"].get("tactic_basic_attack")
    assert ta and ta["countered_by"] == "tactic_counterattack"
    # operations, opinion_modifiers, aces, ideas non-empty
    assert len(c["operations"]) > 10
    assert len(c["opinion_modifiers"]) > 50
    assert len(c["ideas"]) > 1000
    # equipment archetypes (base DEF stats) + upgrades
    amp = c["equipment_archetypes"].get("amphibious_mechanized_equipment")
    assert amp and amp["build_cost_ic"] == "8" and amp["is_archetype"]
    assert len(c["equipment_upgrades"]) > 5
    # power-balance side defs + range thresholds
    bop = c["power_balances"].get("ITA_power_balance")
    assert bop and bop["initial_value"] == "0.35"
    assert bop["left_side"] == "ITA_grand_council_side"
    assert bop["right_side"] == "ITA_mussolini_side"
    assert bop["range_count"] > 10
    side = next((s for s in bop["sides"]
                 if s["id"] == "ITA_mussolini_side"), None)
    assert side and side["icon"] == "GFX_bop_ITA_mussolini_side"
    rng = next((r for r in side["ranges"]
                if r["id"] == "ITA_mussolini_high_control_range"), None)
    assert rng and rng["min"] == "0.6" and rng["max"] == "0.9"
    assert rng["modifier"]["political_power_factor"] == "0.1"

    def count_effect(value, wanted):
        if isinstance(value, dict):
            total = 0
            for key, child in value.items():
                if key.lower() == wanted:
                    total += len(child) if isinstance(child, list) else 1
                total += count_effect(child, wanted)
            return total
        if isinstance(value, list):
            return sum(count_effect(child, wanted) for child in value)
        return 0

    authored_gfx = 0
    for balance in c["power_balances"].values():
        ranges = list(balance["neutral_ranges"])
        ranges.extend(rng for side in balance["sides"] for rng in side["ranges"])
        for effect_range in ranges:
            authored_gfx += count_effect(
                effect_range["on_activate"], "set_power_balance_gfx")
            authored_gfx += count_effect(
                effect_range["on_deactivate"], "set_power_balance_gfx")
    assert authored_gfx == 46


def test_catalog_mod_override_wins(layers):
    # STRUCTURED-DEF mod-over-vanilla proof (not just loc): the AMSB mod overrides
    # common/opinion_modifiers/00_opinion_modifiers.txt, changing the `embargo`
    # opinion value from vanilla -150 to mod -100. The mod file (loaded after
    # vanilla) replaces the whole file, so the effective catalog MUST show -100.
    out = parse_catalogs.parse_catalogs(layers)
    emb = out["catalogs"]["opinion_modifiers"].get("embargo")
    assert emb, "embargo opinion modifier missing"
    assert emb["value"] == "-100", \
        f"mod override failed: embargo={emb['value']} (vanilla was -150)"


# --- focus defs (geometry + cost) -----------------------------------------
def test_focus_defs(layers, loc):
    out = parse_focus_def.parse_focus_defs(layers, loc)
    assert out["meta"]["focus_count"] > 2000, out["meta"]
    assert out["meta"]["default_focus_background_styles"] == ["default_style"]
    assert not out["meta"]["unresolved_focus_background_styles"]
    assert all(
        all(style[key] for key in ("unavailable", "available", "current", "completed"))
        for style in out["focus_background_styles"].values()
    )
    f = out["focuses"].get("AST_additional_militia_training")
    assert f and f["x"] == 2 and f["y"] == 0 and f["cost"] == 10
    assert f["background_style_name"] == "default_style"
    assert f["background_style"]["available"] == "GFX_focus_can_start"


# --- templates ------------------------------------------------------------
def test_templates(layers):
    out = parse_templates.parse_templates(layers)
    assert out["meta"]["template_count"] > 50, out["meta"]
    assert out["meta"]["subunit_defs"] > 10
    sample = out["templates"][0]
    assert sample["battalion_count"] >= 1
    assert "infantry" in str(sample["regiments"]) or sample["regiments"]


# --- world defs (names / terrain / wargoals / mio) ------------------------
def test_world_defs(layers, loc):
    out = parse_world_defs.parse_world_defs(layers, loc)
    assert out["state_names"].get("1") == "Corsica", out["meta"]
    assert out["meta"]["vp_names"] > 500
    assert "forest" in out["terrain"]
    assert "take_state" in out["wargoals"]
    assert out["meta"]["mio"] > 50


# --- characters / advisors / commanders -----------------------------------
def test_characters(layers):
    out = parse_characters.parse_characters(layers)
    assert out["meta"]["character_count"] > 500, out["meta"]
    # the modern advisor Codex flagged as missing from the ideas catalog is here,
    # keyed by its idea_token, with slot/ledger/traits/cost.
    adv = out["advisors_by_token"].get("ALB_halil_nergutti")
    assert adv and adv["slot"] == "high_command" and adv["ledger"] == "navy"
    assert adv["cost"] == "100" and "navy_capital_ship_2" in adv["traits"]
    # commander roster present (officer-corps / unit-leader views)
    assert out["meta"]["by_role"].get("corps_commander", 0) > 50



# --- localisation (R3 token -> localised-name join) -----------------------
def test_localisation(layers):
    out = parse_localisation.parse_localisation(layers)
    m = out["meta"]
    assert m["key_count"] > 50000, m
    assert m["overrides"] > 0, "no mod-over-vanilla loc overrides detected"
    # a known loc key resolves with the raw $VAR$ token intact (not resolved)
    wf = out["keys"].get("WHILE_FOCUSING")
    assert wf and wf["has_interpolation"] and "$EFF$" in wf["raw"]
    assert "$EFF$" in wf["name"], "interpolation token must stay verbatim"


def test_localisation_mod_override_wins(layers):
    # Same mod-override key the resolver-level test already pins
    # (test_loc_mod_override_wins), re-verified through parse_localisation's
    # richer per-key record (name/raw/source_layer), not just LocDB.resolve().
    out = parse_localisation.parse_localisation(layers)
    tip = out["keys"].get("LOADING_TIP_0")
    assert tip, "LOADING_TIP_0 missing"
    assert "Mussolini" not in tip["name"] and "humiliating" not in tip["name"], \
        f"vanilla loc leaked through, mod override failed: {tip}"
    assert tip["source_layer"] != "vanilla", tip


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
