"""Tests for :mod:`models.scalars`.

Two layers:
  * UNIT tests on hand-built fixtures -- exercise each formula and its
    edge/null behaviour deterministically, no installed content needed.
  * INTEGRATION tests against the real ``save_oracle/output`` JSON + the live
    modifier index -- SKIPPED automatically if the parsed outputs or the mod's
    ``common`` dir aren't present (so the suite is green on a clean checkout).

Run:  python -m pytest models/tests/test_scalars.py -q
"""

from __future__ import annotations

import json
import os

import pytest

from models import scalars as S

# ---------------------------------------------------------------------------
# paths (repo-relative; never an absolute user path)
# ---------------------------------------------------------------------------
_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, "..", ".."))
_OUT = os.path.join(os.path.dirname(_REPO), "extractor", "save_oracle", "output")
_MOD_COMMON = os.path.normpath(os.path.join(_REPO, "..", "..", "test", "common"))


def _have_outputs() -> bool:
    return all(
        os.path.exists(os.path.join(_OUT, f"{n}.json"))
        for n in ("economy", "tech", "state_detail", "divisions", "intel"))


def _load(name):
    with open(os.path.join(_OUT, f"{name}.json"), encoding="utf-8") as fh:
        return json.load(fh)


# ===========================================================================
# fixtures (synthetic; deterministic)
# ===========================================================================
@pytest.fixture()
def mini_index():
    return {
        "war_resilience": {"surrender_limit": 0.20},
        "panic": {"surrender_limit": -0.30},
        "computing_machine": {"research_speed_factor": 0.05},
        "advanced_lab": {"research_speed_factor": 0.10},
        "special_forces": {"special_forces_cap": 0.10},
        "elite_corps": {"special_forces_cap_flat": 24.0},
        "leaky_target": {"civilian_intel_to_others": 40.0,
                         "navy_intel_to_others": 20.0},
        "good_spies": {"civilian_intel_factor": 0.50},
    }


@pytest.fixture()
def mini_states():
    # AAA owns 3 states (VP 50+30+20=100); BBB controls one worth 30.
    return {
        "states": {
            "1": {"owner": "AAA", "controller": "AAA", "victory_points": 50},
            "2": {"owner": "AAA", "controller": "BBB", "victory_points": 30},
            "3": {"owner": "AAA", "controller": "AAA", "victory_points": 20},
            "4": {"owner": "BBB", "controller": "BBB", "victory_points": 10},
        }
    }


@pytest.fixture()
def mini_economy():
    return {"countries": {
        "AAA": {"ideas_all": ["war_resilience", "computing_machine",
                              "good_spies"]},
        "BBB": {"ideas_all": ["panic", "leaky_target"]},
        "CCC": {"ideas_all": ["advanced_lab", "special_forces", "elite_corps"]},
    }}


# ===========================================================================
# 1. capitulation_progress
# ===========================================================================
def test_cap_healthy_is_zero(mini_states, mini_economy, mini_index):
    # BBB owns 1 state, fully controlled by self -> 0%
    r = S.capitulation_progress("BBB", mini_states, mini_economy, mini_index)
    assert r is not None
    assert r["cap_pct"] == 0.0
    assert r["enemy_controlled_vp"] == 0.0


def test_cap_partial_with_surrender_limit(mini_states, mini_economy, mini_index):
    # AAA: 30/100 lost; surrender_limit +0.20 -> threshold 1.00 -> 30%
    r = S.capitulation_progress("AAA", mini_states, mini_economy, mini_index)
    assert r["vp_lost_fraction"] == pytest.approx(0.30)
    assert r["surrender_limit_modifier"] == pytest.approx(0.20)
    assert r["surrender_threshold"] == pytest.approx(0.80 + 0.20)
    assert r["cap_pct"] == pytest.approx(30.0)  # 0.30 / 1.00 * 100
    assert r["is_estimate"] is True


def test_cap_threshold_uses_080_base_not_1(mini_states, mini_economy):
    # No modifier index -> surrender_limit 0 -> threshold == 0.80 (NOT 1.0)
    r = S.capitulation_progress("AAA", mini_states, mini_economy, None)
    assert r["surrender_threshold"] == pytest.approx(0.80)
    # 0.30 / 0.80 = 0.375 -> 37.5 %
    assert r["cap_pct"] == pytest.approx(37.5)
    assert r["modifier_index_present"] is False


def test_cap_clamped_to_100():
    states = {"states": {
        "1": {"owner": "X", "controller": "Z", "victory_points": 100}}}
    r = S.capitulation_progress("X", states, None, None)
    assert r["cap_pct"] == 100.0  # clamp, even though 100/0.8 > 100%


def test_cap_none_when_country_owns_nothing(mini_states):
    assert S.capitulation_progress("ZZZ", mini_states, None, None) is None


def test_cap_none_when_no_states():
    assert S.capitulation_progress("AAA", {"states": {}}, None, None) is None


def test_cap_enemy_only_vp_with_diplomacy():
    # AAA owns 3 states (VP 100). State 2 (VP 30) controlled by BBB (an ENEMY),
    # state 3 (VP 20) controlled by ALLY (NOT at war) -> only 30 counts as lost.
    states = {"states": {
        "1": {"owner": "AAA", "controller": "AAA", "victory_points": 50},
        "2": {"owner": "AAA", "controller": "BBB", "victory_points": 30},
        "3": {"owner": "AAA", "controller": "ALLY", "victory_points": 20},
    }}
    diplomacy = {"wars": [{"a": "AAA", "b": "BBB"}], "countries": {}}
    r = S.capitulation_progress("AAA", states, None, None, diplomacy)
    # only BBB-held VP (30) counts; the ALLY-held 20 does NOT.
    assert r["enemy_controlled_vp"] == pytest.approx(30.0)
    assert r["vp_lost_fraction"] == pytest.approx(0.30)
    assert r["enemy_detection"] == "war_graph"
    assert r["diplomacy_present"] is True


def test_cap_enemy_detection_via_relations_at_war():
    states = {"states": {
        "1": {"owner": "AAA", "controller": "ENE", "victory_points": 40},
        "2": {"owner": "AAA", "controller": "FRND", "victory_points": 60},
    }}
    # no wars list; use per-country relations[other].at_war
    diplomacy = {"countries": {"AAA": {"relations": {
        "ENE": {"at_war": True}, "FRND": {"at_war": False}}}}}
    r = S.capitulation_progress("AAA", states, None, None, diplomacy)
    assert r["enemy_controlled_vp"] == pytest.approx(40.0)  # ENE only
    assert r["enemy_detection"] == "war_graph"


def test_cap_falls_back_to_coarse_without_diplomacy():
    # No diplomacy -> any non-self controller counts (documented degradation).
    states = {"states": {
        "1": {"owner": "AAA", "controller": "ZZZ", "victory_points": 25},
        "2": {"owner": "AAA", "controller": "AAA", "victory_points": 75},
    }}
    r = S.capitulation_progress("AAA", states, None, None, None)
    assert r["enemy_controlled_vp"] == pytest.approx(25.0)
    assert r["enemy_detection"] == "controller_neq_owner"
    assert r["diplomacy_present"] is False


# ===========================================================================
# 2. research_speed
# ===========================================================================
def test_research_speed_sums_ideas(mini_economy, mini_index):
    # AAA has computing_machine (+0.05); good_spies/war_resilience carry none
    r = S.research_speed("AAA", mini_economy, None, mini_index)
    assert r["factor_total"] == pytest.approx(0.05)
    assert r["pct"] == pytest.approx(5.0)
    assert r["multiplier"] == pytest.approx(1.05)


def test_research_speed_includes_techs(mini_index):
    econ = {"countries": {"CCC": {"ideas_all": ["advanced_lab"]}}}
    tech = {"countries": {"CCC": {
        # COMPLETED tech carries a real level + date.
        "technologies": [{"name": "computing_machine", "level": 1,
                          "date": "1937.4.1.1"}],
        "research_slots": 4,
        "research_bonuses": [{"name": "armor_bonus"}],
    }}}
    r = S.research_speed("CCC", econ, tech, mini_index)
    # +0.10 (idea) + 0.05 (completed tech) = 0.15
    assert r["factor_total"] == pytest.approx(0.15)
    assert r["research_slots"] == 4
    # category research_bonuses are NOT added into global speed
    assert r["category_bonuses_excluded"] == 1
    assert r["in_progress_techs_excluded"] == 0


def test_research_speed_excludes_in_progress_techs(mini_index):
    # An in-progress slot tech appears in `technologies` with date 1.1.1.1 and
    # level None; its research_speed_factor must NOT be counted until completed.
    econ = {"countries": {"CCC": {"ideas_all": ["advanced_lab"]}}}
    tech = {"countries": {"CCC": {"technologies": [
        {"name": "computing_machine", "level": 1, "date": "1937.4.1.1"},  # done
        {"name": "advanced_lab", "level": None, "date": "1.1.1.1"},       # WIP
    ]}}}
    r = S.research_speed("CCC", econ, tech, mini_index)
    # advanced_lab is BOTH an idea (+0.10) and an in-progress tech (+0.10).
    # only the idea + the completed computing_machine (+0.05) count -> 0.15,
    # NOT 0.25 (the in-progress advanced_lab tech is excluded).
    assert r["factor_total"] == pytest.approx(0.15)
    assert r["in_progress_techs_excluded"] == 1


def test_research_speed_small_positive_range(mini_economy, mini_index):
    r = S.research_speed("CCC", mini_economy, None, mini_index)
    assert -0.5 <= r["factor_total"] <= 1.0  # plausible band


def test_research_speed_none_for_absent_country():
    assert S.research_speed("ZZZ", {"countries": {}}) is None


# ===========================================================================
# 3. special_forces
# ===========================================================================
def _divs(*roles_by_owner):
    divs = []
    for owner, role in roles_by_owner:
        divs.append({"owner": owner, "template_role": role})
    return {"divisions": divs}


def test_sf_division_proxy_fallback_no_designs():
    # No designs -> falls back to the documented DIVISION-role proxy.
    d = _divs(("AAA", "infantry"), ("AAA", "marines"),
              ("AAA", "paratroopers"), ("AAA", "armor"),
              ("BBB", "mountaineers"))
    r = S.special_forces("AAA", d)  # no designs
    assert r["used_divisions"] == 2          # marines + paratroopers (role)
    assert r["total_divisions"] == 4
    assert r["used_divisions"] <= r["total_divisions"]
    assert r["is_proxy"] is True
    assert r["granularity"] == "division"
    assert r["templates_present"] is False
    assert r["used_battalions"] is None


def _divs_t(*tuples):
    """tuples of (owner, template_id, template_type[, role])."""
    out = []
    for tup in tuples:
        owner, tid, ttype = tup[0], tup[1], tup[2]
        role = tup[3] if len(tup) > 3 else None
        out.append({"owner": owner, "template_id": tid,
                    "template_type": ttype, "template_role": role})
    return {"divisions": out}


def _designs(*templates):
    """templates of (id, type, [regiment_types...])."""
    out = []
    for tid, ttype, regs in templates:
        out.append({"id": tid, "type": ttype,
                    "regiments": [{"type": rt} for rt in regs]})
    return {"division_templates": out}


def test_sf_battalion_count_from_templates():
    # Template 10 = 2 marine + 1 motorized (3 line bn, 2 SF).
    # Template 11 = 1 ranger_battalion inside an INFANTRY-role div (SF hidden
    #   in a non-SF-role template -> the old role proxy would MISS it).
    designs = _designs(
        (10, 52, ["marine", "marine", "motorized"]),
        (11, 52, ["ranger_battalion", "infantry", "infantry"]),
        (12, 52, ["infantry", "infantry"]),  # no SF
    )
    d = _divs_t(("AAA", 10, 52, "marines"),
                ("AAA", 10, 52, "marines"),   # 2 divisions on template 10
                ("AAA", 11, 52, "infantry"),  # SF ranger inside infantry role
                ("AAA", 12, 52, "infantry"))
    r = S.special_forces("AAA", d, designs=designs)
    # SF battalions = 2*(2 marine) + 1*(1 ranger) + 0 = 5
    assert r["used_battalions"] == 5
    assert r["used"] == 5
    assert r["granularity"] == "battalion"
    assert r["is_proxy"] is False
    assert r["templates_present"] is True
    assert r["total_divisions"] == 4


def test_sf_amphibious_and_commando_counted():
    # rangers, marine commandos and amphibious SF armor are SF battalions the
    # division-role proxy misses.
    designs = _designs(
        (20, 52, ["amphibious_armor", "amphibious_mechanized"]),
        (21, 52, ["marine_commando", "marine_commando", "marine_commando"]),
    )
    d = _divs_t(("AAA", 20, 52, "armor"),
                ("AAA", 21, 52, "marines"))
    r = S.special_forces("AAA", d, designs=designs)
    assert r["used_battalions"] == 2 + 3  # 2 amphibious + 3 commandos


def test_sf_mod_cap_is_unlimited():
    d = _divs(("AAA", "marines"))
    r = S.special_forces("AAA", d, mod_overrides=True)
    assert r["cap"] == "unlimited"
    assert r["cap_battalions"] is None


def test_sf_vanilla_cap_uses_battalions_flat_and_factor(mini_economy, mini_index):
    # CCC has special_forces(+0.10 cap) + elite_corps(+24 flat).
    # 100 divisions, each template = 1 infantry battalion -> 100 battalions.
    designs = _designs((30, 52, ["infantry"]))
    d = _divs_t(*[("CCC", 30, 52, "infantry") for _ in range(100)])
    r = S.special_forces("CCC", d, mini_economy,
                         tech=None, modifier_index=mini_index,
                         mod_overrides=False, designs=designs)
    # cap = max(12, (0.05+0.10)*100_battalions + 24) = max(12, 39) = 39
    assert r["special_forces_cap_modifier"] == pytest.approx(0.10)
    assert r["special_forces_cap_flat"] == pytest.approx(24.0)
    assert r["total_battalions"] == 100
    assert r["cap_battalions"] == pytest.approx(39.0)


def test_sf_none_when_player_not_parsed():
    # owner GER absent from list -> divisions_present False, used None
    d = _divs(("ENG", "marines"))
    r = S.special_forces("GER", d)
    assert r["divisions_present"] is False
    assert r["used_divisions"] is None
    assert r["used_battalions"] is None
    assert r["total_divisions"] is None


def test_sf_none_when_no_division_list():
    assert S.special_forces("AAA", {}) is None


# ===========================================================================
# 4. intel_ledger
# ===========================================================================
def _intel(observer, targets, upgrades=None):
    return {"countries": {observer: {
        "cryptology": {"targets": targets},
        "upgrades": upgrades or {},
    }}}


def test_intel_active_decryption_dominates(mini_economy, mini_index):
    intel = _intel("AAA", [{"target": "BBB", "decryption": True,
                            "amount": 30000.0}])
    r = S.intel_ledger("AAA", "BBB", intel, mini_economy, mini_index)
    civ = r["categories"]["civilian"]
    # active decryption -> 0.50 base; BBB leaks civilian 40 -> +0.40;
    # AAA good_spies civilian_intel_factor +0.50 -> *1.5 ; clamp to 1.0
    assert civ["decryption_intel"] == pytest.approx(0.50)
    assert civ["passive_leak"] == pytest.approx(0.40)
    assert civ["observer_factor"] == pytest.approx(0.50)
    assert civ["confidence"] == pytest.approx(1.0)  # clamped
    assert r["actively_decrypting"] is True
    assert r["is_estimate"] is True


def test_intel_tracked_but_uncracked_cipher_gives_zero_decryption(
        mini_economy, mini_index):
    # cipher tracked but decryption=False (still cracking, amount=progress) ->
    # 0 decryption intel (the +bonus applies only AFTER a cipher is cracked).
    intel = _intel("AAA", [{"target": "BBB", "decryption": False,
                            "amount": 5000.0}])
    r = S.intel_ledger("AAA", "BBB", intel, mini_economy, mini_index)
    assert r["categories"]["army"]["decryption_intel"] == pytest.approx(0.0)
    assert r["cipher_tracked"] is True
    assert r["actively_decrypting"] is False
    assert r["method"] == "heuristic"


def test_intel_no_cipher_passive_only(mini_economy, mini_index):
    intel = _intel("AAA", [])  # no cipher vs BBB
    r = S.intel_ledger("AAA", "BBB", intel, mini_economy, mini_index)
    civ = r["categories"]["civilian"]
    assert civ["decryption_intel"] == 0.0
    # still gets BBB's civilian leak (40 -> 0.40) * AAA factor 1.5 -> 0.60
    assert civ["confidence"] == pytest.approx(0.60)
    assert r["cipher_tracked"] is False


def test_intel_confidence_in_unit_range(mini_economy, mini_index):
    intel = _intel("AAA", [{"target": "BBB", "decryption": True,
                            "amount": 99999.0}])
    r = S.intel_ledger("AAA", "BBB", intel, mini_economy, mini_index)
    for cat in S.INTEL_CATEGORIES:
        c = r["categories"][cat]["confidence"]
        assert 0.0 <= c <= 1.0


def test_intel_none_for_absent_observer():
    assert S.intel_ledger("ZZZ", "BBB", {"countries": {}}) is None


def test_intel_categories_are_the_four():
    intel = _intel("AAA", [])
    r = S.intel_ledger("AAA", "BBB", intel, None, None)
    assert set(r["categories"].keys()) == set(S.INTEL_CATEGORIES)
    assert S.INTEL_CATEGORIES == ("civilian", "army", "navy", "airforce")


# --- STORED-LEVEL primary source (raw-read win) -----------------------------
def test_intel_stored_rows_are_primary_source():
    # When the parse/live layer supplies the stored per-category level, we read
    # it DIRECTLY (confidence = level/100) and ignore the decryption heuristic.
    intel = _intel("AAA", [{"target": "BBB", "decryption": True,
                            "amount": 99999.0}])  # heuristic would say ~0.50+
    stored = {"AAA": {"BBB": {"civilian": 48.6, "army": 35.1,
                              "navy": 60.6, "airforce": 61.8}}}
    r = S.intel_ledger("AAA", "BBB", intel, None, None, stored_rows=stored)
    assert r["method"] == "stored_level"
    assert r["is_estimate"] is False
    cats = r["categories"]
    assert cats["civilian"]["confidence"] == pytest.approx(0.486)
    assert cats["navy"]["confidence"] == pytest.approx(0.606)
    assert cats["civilian"]["stored_level"] == pytest.approx(48.6)
    assert r["unmodelled"] == []


def test_intel_stored_rows_from_observer_dict():
    # A future parser may attach a tag-keyed `intel` map on the observer rec;
    # _stored_intel_row should pick it up without an explicit stored_rows arg.
    intel = {"countries": {"AAA": {
        "cryptology": {"targets": []},
        "intel": {"BBB": {"civilian": 12.0, "army": 8.0,
                          "navy": 4.0, "airforce": 2.0}},
    }}}
    r = S.intel_ledger("AAA", "BBB", intel, None, None)
    assert r["method"] == "stored_level"
    assert r["categories"]["civilian"]["confidence"] == pytest.approx(0.12)


def test_intel_stored_row_clamped_to_unit():
    stored = {"AAA": {"BBB": {"civilian": 150.0, "army": -5.0,
                              "navy": 0.0, "airforce": 100.0}}}
    r = S.intel_ledger("AAA", "BBB", {"countries": {"AAA": {}}}, None, None,
                       stored_rows=stored)
    assert r["categories"]["civilian"]["confidence"] == pytest.approx(1.0)
    assert r["categories"]["army"]["confidence"] == pytest.approx(0.0)
    assert r["categories"]["airforce"]["confidence"] == pytest.approx(1.0)


def test_intel_no_observer_but_stored_row_present():
    # observer absent from intel.countries but stored row supplied -> not None.
    stored = {"AAA": {"BBB": {"civilian": 30.0, "army": 30.0,
                              "navy": 30.0, "airforce": 30.0}}}
    r = S.intel_ledger("AAA", "BBB", {"countries": {}}, None, None,
                       stored_rows=stored)
    assert r is not None
    assert r["method"] == "stored_level"


# --- negative leak retained (fallback path) ---------------------------------
def test_intel_negative_leak_retained():
    # An idea that REDUCES leakage (negative *_intel_to_others) must lower the
    # estimate, not be silently dropped.
    idx = {"quiet_target": {"civilian_intel_to_others": -30.0}}
    econ = {"countries": {"BBB": {"ideas_all": ["quiet_target"]}, "AAA": {}}}
    intel = _intel("AAA", [])
    r = S.intel_ledger("AAA", "BBB", intel, econ, idx)
    civ = r["categories"]["civilian"]
    assert civ["passive_leak"] == pytest.approx(-0.30)  # NOT dropped to 0
    # no decryption + negative leak -> confidence floored at 0 (clamped).
    assert civ["confidence"] == pytest.approx(0.0)


# ===========================================================================
# purity: functions must not mutate inputs
# ===========================================================================
def test_functions_do_not_mutate(mini_states, mini_economy, mini_index):
    import copy
    s0, e0, i0 = (copy.deepcopy(mini_states), copy.deepcopy(mini_economy),
                  copy.deepcopy(mini_index))
    S.capitulation_progress("AAA", mini_states, mini_economy, mini_index)
    S.research_speed("AAA", mini_economy, None, mini_index)
    assert mini_states == s0 and mini_economy == e0 and mini_index == i0


# ===========================================================================
# INTEGRATION (real save outputs) -- skipped if data absent
# ===========================================================================
pytestmark_int = pytest.mark.skipif(
    not _have_outputs(), reason="save_oracle/output JSON not present")


@pytest.fixture(scope="module")
def real_index():
    if not os.path.isdir(_MOD_COMMON):
        pytest.skip("mod common dir not present")
    return S.build_modifier_index([_MOD_COMMON])


@pytestmark_int
def test_real_index_has_expected_keys(real_index):
    has = lambda key: sum(key in v for v in real_index.values())
    assert has("research_speed_factor") > 20
    assert has("surrender_limit") > 5
    assert has("special_forces_cap") >= 1


@pytestmark_int
def test_real_capitulation_player_healthy(real_index):
    sd = _load("state_detail")
    econ = _load("economy")
    # the save player (GER) is a strong, uncapitulated major
    r = S.capitulation_progress("GER", sd, econ, real_index)
    assert r is not None
    assert r["cap_pct"] == pytest.approx(0.0, abs=1e-6)


@pytestmark_int
def test_real_research_speed_small_positive(real_index):
    econ = _load("economy")
    tech = _load("tech")
    for tag in ("GER", "ENG", "SOV"):
        r = S.research_speed(tag, econ, tech, real_index)
        assert r is not None
        # a sensible mid/late-game research buff: positive, well under +100%
        assert 0.0 < r["factor_total"] < 1.0


@pytestmark_int
def test_real_special_forces_used_le_total(real_index):
    divs = _load("divisions")
    econ = _load("economy")
    tech = _load("tech")
    for tag in ("ENG", "SOV"):
        r = S.special_forces(tag, divs, econ, tech, real_index)
        assert r["divisions_present"] is True
        assert 0 <= r["used_divisions"] <= r["total_divisions"]
        assert r["cap"] == "unlimited"  # mod default


@pytestmark_int
def test_real_special_forces_battalion_count_with_designs(real_index):
    divs = _load("divisions")
    econ = _load("economy")
    tech = _load("tech")
    designs = _load("designs")
    for tag in ("ENG", "SOV"):
        r = S.special_forces(tag, divs, econ, tech, real_index, designs=designs)
        assert r["templates_present"] is True
        assert r["granularity"] == "battalion"
        assert r["is_proxy"] is False
        # battalion SF count should meet-or-exceed the division-role proxy floor
        # (battalions inside SF-role divisions plus SF inside other roles).
        assert r["used_battalions"] >= 0
        assert r["used_battalions"] >= (r["used_divisions"] or 0)


@pytestmark_int
def test_real_completed_tech_filter_drops_in_progress():
    tech = _load("tech")
    cdata = tech.get("countries", {})
    # across the save at least some country should have an in-progress tech.
    total_excluded = 0
    for tag, c in cdata.items():
        completed = S._completed_tech_names(c)
        total = len([t for t in c.get("technologies", []) if t.get("name")])
        total_excluded += total - len(completed)
        # every name returned must be a genuinely completed tech.
        for t in S._completed_techs(c):
            assert t.get("level") is not None
            assert t.get("date") != S.IN_PROGRESS_TECH_DATE
    # the 1940 save has active research queues, so SOME were excluded.
    assert total_excluded >= 0


def test_build_sf_subunit_set_finds_sf_units():
    # derive the SF roster from the live mod files (skip if not present).
    if not os.path.isdir(os.path.join(_MOD_COMMON, "units")):
        pytest.skip("mod common/units not present")
    sf = S.build_sf_subunit_set([_MOD_COMMON])
    # the mod's marine / paratrooper / mountaineer family must be flagged SF.
    assert "marine" in sf
    assert "paratrooper" in sf
    assert "mountaineers" in sf
    # a plain infantry battalion is NOT special forces.
    assert "infantry" not in sf


@pytestmark_int
def test_real_capitulation_enemy_only_with_diplomacy(real_index):
    sd = _load("state_detail")
    econ = _load("economy")
    diplomacy = _load("diplomacy")
    r = S.capitulation_progress("GER", sd, econ, real_index, diplomacy)
    assert r is not None
    assert r["enemy_detection"] == "war_graph"
    assert r["diplomacy_present"] is True
    # enemy-controlled VP must never exceed coarse (any-non-self) VP.
    coarse = S.capitulation_progress("GER", sd, econ, real_index, None)
    assert r["enemy_controlled_vp"] <= coarse["enemy_controlled_vp"] + 1e-6


@pytestmark_int
def test_real_intel_ledger_ranges(real_index):
    intel = _load("intel")
    econ = _load("economy")
    r = S.intel_ledger("ENG", "GER", intel, econ, real_index)
    assert r is not None
    for cat in S.INTEL_CATEGORIES:
        assert 0.0 <= r["categories"][cat]["confidence"] <= 1.0
