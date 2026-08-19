#!/usr/bin/env python3
"""Tests for parse_projects.py (special projects + facilities + scientists).

Self-contained: drives the parser's ``build`` over a SYNTHETIC melted-save text
that reproduces the real 1.19 special-projects shapes (verified against a
campaign melt), so the suite runs with no save file present. If the real
campaign melt happens to be on disk, one extra test cross-checks the validated
GER project/facility/scientist numbers against it.

Run:  python -m pytest save_oracle/test_parse_projects.py
  or:  python save_oracle/test_parse_projects.py
"""
from __future__ import annotations

import os
import sys
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import parse_projects as pp


SYN = """
date=1940.9.10.13
version="Operation Postern v1.19.1.0"
player="GER"
character_manager={
\thistorical={
\t\tcharacter={
\t\t\tid={ id=1152 type=73 }
\t\t\ttoken="GER_von_braun"
\t\t\tname="GER_von_braun"
\t\t\tcountry="GER"
\t\t\tgender=male
\t\t\tscientist={
\t\t\t\ttraits={ scientist_trait_rocketry_specialist scientist_trait_bright }
\t\t\t\tskills={ specialization_nuclear={ } specialization_naval={ } specialization_air={ level=4 } specialization_land={ } }
\t\t\t}
\t\t}
\t\tcharacter={
\t\t\tid={ id=1156 type=73 }
\t\t\ttoken="GER_burkhardt"
\t\t\tname="GER_burkhardt"
\t\t\tcountry="GER"
\t\t\tgender=male
\t\t\tscientist={
\t\t\t\ttraits={ scientist_trait_surface_ship_specialist }
\t\t\t\tskills={ specialization_nuclear={ } specialization_naval={ level=2 } specialization_air={ } specialization_land={ } }
\t\t\t}
\t\t}
\t}
\tdynamic={
\t\tcharacter={
\t\t\tid={ id=2001 type=73 }
\t\t\ttoken="ROM_popescu"
\t\t\tname="ROM_popescu"
\t\t\tcountry="ROM"
\t\t\tgender=male
\t\t\tscientist={
\t\t\t\ttraits={ scientist_trait_armored_specialist }
\t\t\t\tskills={ specialization_nuclear={ } specialization_naval={ } specialization_air={ } specialization_land={ level=1 } }
\t\t\t}
\t\t}
\t\tcharacter={
\t\t\tid={ id=9999 type=73 }
\t\t\ttoken="GER_politician"
\t\t\tname="GER_politician"
\t\t\tcountry="GER"
\t\t\tgender=male
\t\t\tadvisors={ advisor={ slot="political_advisor" } }
\t\t}
\t}
}
countries={
\tGER={
\t\tprogram_status={
\t\t\tproject_pool={
\t\t\t\tproject={
\t\t\t\t\tsp_naval_support_ships={
\t\t\t\t\t\tid={ id=33 type=86 }
\t\t\t\t\t\tprogress={
\t\t\t\t\t\t\tcurrent_state=0
\t\t\t\t\t\t\tprototype_state={ project_progress=81 iterations=5 phase_progress=24.31002 }
\t\t\t\t\t\t\tstopping_state={ phase_progress=0 }
\t\t\t\t\t\t}
\t\t\t\t\t\tvariables={ }
\t\t\t\t\t\tflags={
\t\t\t\t\t\t\tsp_naval_generic_reward_major_progress_flag={ value=1 date=1940.6.29.1 }
\t\t\t\t\t\t\tsp_naval_generic_reward_test_failure_flag={ value=1 date=1940.5.7.1 }
\t\t\t\t\t\t}
\t\t\t\t\t\thistory={
\t\t\t\t\t\t\t{
\t\t\t\t\t\t\t\tcontext={ province=241 character={ id=1156 type=73 } progress=24 country=865255792 scientist=21544 state=499926426 }
\t\t\t\t\t\t\t\toption=sp_naval_generic_reward_option_major_breakthrough
\t\t\t\t\t\t\t\tprototype_reward=sp_naval_generic_reward_major_progress_2
\t\t\t\t\t\t\t\tdate=1940.6.29.1
\t\t\t\t\t\t\t}
\t\t\t\t\t\t}
\t\t\t\t\t\tstart=yes
\t\t\t\t\t\tcompleted=no
\t\t\t\t\t}
\t\t\t\t\tsp_land_flamethrower_tank={
\t\t\t\t\t\tid={ id=9 type=86 }
\t\t\t\t\t\tprogress={
\t\t\t\t\t\t\tcurrent_state=2
\t\t\t\t\t\t\tprototype_state={ project_progress=100 iterations=5 phase_progress=45.045 }
\t\t\t\t\t\t\tstopping_state={ phase_progress=0 }
\t\t\t\t\t\t}
\t\t\t\t\t\tvariables={ }
\t\t\t\t\t\tflags={ }
\t\t\t\t\t\tstart=yes
\t\t\t\t\t\tcompleted=yes
\t\t\t\t\t}
\t\t\t\t\tsp_air_jet_engine={
\t\t\t\t\t\tid={ id=5 type=86 }
\t\t\t\t\t\tprogress={
\t\t\t\t\t\t\tcurrent_state=0
\t\t\t\t\t\t\tprototype_state={ project_progress=0 iterations=0 phase_progress=0 }
\t\t\t\t\t\t\tstopping_state={ phase_progress=0 }
\t\t\t\t\t\t}
\t\t\t\t\t\tvariables={ }
\t\t\t\t\t\tflags={ }
\t\t\t\t\t\tstart=no
\t\t\t\t\t\tcompleted=no
\t\t\t\t\t}
\t\t\t\t}
\t\t\t\tresources={ steel=6 }
\t\t\t}
\t\t\tprogram={
\t\t\t\tid={ id=17 type=83 }
\t\t\t\tcountry="GER"
\t\t\t\tprovince=241
\t\t\t\tproject=sp_naval_support_ships
\t\t\t\tscientist={
\t\t\t\t\tscientist={ id=1156 type=73 }
\t\t\t\t\tdelayed_unassigned_scientist={ target=10 current=0 timed=no }
\t\t\t\t}
\t\t\t\tdelayed_dismantle_facility={ target=100 current=0 timed=no }
\t\t\t\tsupply={ tag="GER" province_id=241 id={ id=17 type=87 } motorization_level=0 }
\t\t\t\tbuilding="naval_facility"
\t\t\t\tunread_prototype_rewards=5
\t\t\t}
\t\t\tprogram={
\t\t\t\tid={ id=20 type=83 }
\t\t\t\tcountry="GER"
\t\t\t\tprovince=9388
\t\t\t\tscientist={
\t\t\t\t\tscientist={ id=1152 type=73 }
\t\t\t\t\tdelayed_unassigned_scientist={ target=10 current=0 timed=no }
\t\t\t\t}
\t\t\t\tdelayed_dismantle_facility={ target=100 current=42 timed=no }
\t\t\t\tsupply={ tag="GER" province_id=9388 id={ id=20 type=87 } motorization_level=1 }
\t\t\t\tbuilding="air_facility"
\t\t\t\tunread_prototype_rewards=0
\t\t\t}
\t\t\tbreakthrough={ specialization_nuclear=6204 specialization_naval=11895 specialization_air=7932 specialization_land=19390 }
\t\t}
\t}
\tROM={
\t\tprogram_status={
\t\t\tproject_pool={ project={ } }
\t\t\tbreakthrough={ specialization_land=50 }
\t\t}
\t}
}
"""


def _build():
    outputs, report = pp.build(SYN, Path("SYN_melted.txt"), False)
    return outputs["projects.json"], report



def test_meta_stats():
    out, _ = _build()
    st = out["meta"]["stats"]
    assert st["countries_with_program_status"] == 2
    assert st["projects_total"] == 3
    assert st["projects_started"] == 2
    assert st["projects_completed"] == 1
    assert st["facilities_total"] == 2
    assert st["scientists_total"] == 3
    assert st["scientist_countries"] == 2



def test_project_field_mapping():
    out, _ = _build()
    projs = {p["token"]: p for p in out["countries"]["GER"]["projects"]}
    nav = projs["sp_naval_support_ships"]
    assert nav["id"] == 33 and nav["type"] == 86
    assert nav["current_state"] == 0
    assert nav["project_progress"] == 81.0
    assert nav["iterations"] == 5
    assert nav["phase_progress"] == 24.31002
    assert nav["start"] is True and nav["completed"] is False
    flags = {f["flag"]: f for f in nav["project_flags"]}
    assert "sp_naval_generic_reward_major_progress_flag" in flags
    assert flags["sp_naval_generic_reward_major_progress_flag"]["value"] == 1
    assert flags["sp_naval_generic_reward_major_progress_flag"]["date"] == "1940.6.29.1"


def test_reward_history_with_context():
    out, _ = _build()
    projs = {p["token"]: p for p in out["countries"]["GER"]["projects"]}
    hist = projs["sp_naval_support_ships"]["reward_history"]
    assert len(hist) == 1
    ev = hist[0]
    assert ev["option"] == "sp_naval_generic_reward_option_major_breakthrough"
    assert ev["prototype_reward"] == "sp_naval_generic_reward_major_progress_2"
    assert ev["date"] == "1940.6.29.1"
    assert ev["context"]["province"] == 241
    assert ev["context"]["progress"] == 24.0
    assert ev["context"]["scientist_char_id"] == 1156


def test_active_vs_completed_split():
    out, _ = _build()
    ger = out["countries"]["GER"]
    assert ger["projects_active"] == 1
    assert ger["completed_projects"] == ["sp_land_flamethrower_tank"]
    ap = {p["token"]: p for p in ger["active_projects"]}
    assert "sp_naval_support_ships" in ap
    assert "sp_land_flamethrower_tank" not in ap
    assert "sp_air_jet_engine" not in ap


def test_active_project_in_facility_flag():
    out, _ = _build()
    ap = {p["token"]: p for p in out["countries"]["GER"]["active_projects"]}
    assert ap["sp_naval_support_ships"]["in_facility"] is True



def test_facility_field_mapping():
    out, _ = _build()
    facs = {f["id"]: f for f in out["countries"]["GER"]["facilities"]}
    f = facs[17]
    assert f["type"] == 83
    assert f["country"] == "GER" and f["province"] == 241
    assert f["building"] == "naval_facility"
    assert f["active_project"] == "sp_naval_support_ships"
    assert f["scientist_id"] == 1156
    assert f["unread_prototype_rewards"] == 5
    assert f["supply"]["tag"] == "GER"
    assert f["supply"]["province_id"] == 241
    assert f["supply"]["motorization_level"] == 0
    assert f["unassigned_scientist"] == {"target": 10, "current": 0, "timed": False}
    assert f["scientist_token"] == "GER_burkhardt"


def test_facility_idle_when_no_project():
    out, _ = _build()
    facs = {f["id"]: f for f in out["countries"]["GER"]["facilities"]}
    assert facs[20]["active_project"] is None
    assert facs[20]["building"] == "air_facility"
    assert facs[20]["supply"]["motorization_level"] == 1


def test_facility_dismantle_in_progress():
    out, _ = _build()
    facs = {f["id"]: f for f in out["countries"]["GER"]["facilities"]}
    assert facs[17]["dismantle_in_progress"] is False
    assert facs[20]["dismantle_in_progress"] is True
    assert facs[20]["dismantle"]["current"] == 42



def test_scientist_roster():
    out, _ = _build()
    ger = out["scientists"]["GER"]
    assert len(ger) == 2
    byid = {s["char_id"]: s for s in ger}
    braun = byid[1152]
    assert braun["country"] == "GER"
    assert braun["char_type"] == 73
    assert braun["name_token"] == "GER_von_braun"
    assert "scientist_trait_rocketry_specialist" in braun["traits"]
    assert "scientist_trait_bright" in braun["traits"]
    assert braun["skills"] == {
        "specialization_nuclear": 0, "specialization_naval": 0,
        "specialization_air": 4, "specialization_land": 0}


def test_scientist_non_scientist_ignored():
    out, _ = _build()
    ids = {s["char_id"] for s in out["scientists"]["GER"]}
    assert 9999 not in ids


def test_scientist_other_country():
    out, _ = _build()
    rom = out["scientists"]["ROM"]
    assert len(rom) == 1
    assert rom[0]["name_token"] == "ROM_popescu"
    assert rom[0]["skills"]["specialization_land"] == 1



def test_meta_and_no_path_leak():
    out, report = _build()
    m = out["meta"]
    assert m["save_date"] == "1940.9.10.13"
    assert m["player"] == "GER"
    blob = str(out) + report
    for bad in ("C:\\Users", "OneDrive", "/c/Users", "redacted"):
        assert bad not in blob
    assert "SPECIAL-PROJECTS" in report



def test_real_campaign_melt_if_present():
    melt = (Path(__file__).resolve().parent.parent
            / "re_scratch" / "campaign_melt_1940.txt")
    if not melt.is_file():
        return
    text = melt.read_text(encoding="utf-8", errors="replace")
    outputs, _ = pp.build(text, melt, False)
    out = outputs["projects.json"]
    st = out["meta"]["stats"]
    assert st["countries_with_program_status"] == 143
    assert st["projects_started"] == 30
    assert st["projects_completed"] == 30
    assert st["facilities_total"] == 17
    assert st["scientists_total"] == 290
    ger = out["countries"]["GER"]
    assert ger["projects_total"] == 47
    assert ger["projects_started"] == 5
    assert ger["projects_completed"] == 4
    facs = {f["id"]: f for f in ger["facilities"]}
    assert facs[17]["active_project"] == "sp_naval_support_ships"
    assert facs[17]["building"] == "naval_facility"
    assert facs[17]["province"] == 241
    assert facs[17]["unread_prototype_rewards"] == 5
    assert facs[17]["scientist_token"] == "GER_hermann_burkhardt"


def _run_standalone():
    """Run every test_* in this module without requiring pytest."""
    g = dict(globals())
    tests = sorted(n for n, f in g.items()
                   if n.startswith("test_") and callable(f))
    failed = 0
    for name in tests:
        try:
            g[name]()
            print(f"PASS {name}")
        except AssertionError as exc:
            failed += 1
            print(f"FAIL {name}: {exc}")
        except Exception as exc:
            failed += 1
            print(f"ERROR {name}: {exc!r}")
    print(f"\n{len(tests) - failed}/{len(tests)} passed")
    return 1 if failed else 0


if __name__ == "__main__":
    try:
        import pytest
        raise SystemExit(pytest.main([__file__, "-v"]))
    except ImportError:
        raise SystemExit(_run_standalone())
