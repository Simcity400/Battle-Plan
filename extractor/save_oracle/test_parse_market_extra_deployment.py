#!/usr/bin/env python3
"""Tests for parse_market_extra.py + parse_deployment.py.

Self-contained: each parser's ``build`` runs over a SYNTHETIC melted-save text
reproducing the real 1.19 shapes (verified against re_scratch/campaign_melt_1940
.txt), so the suite runs with no save file present. If the real campaign melt is
on disk, extra tests cross-check the known counts against it.

Run:  python -m pytest save_oracle/test_parse_market_extra_deployment.py
  or:  python save_oracle/test_parse_market_extra_deployment.py
"""
from __future__ import annotations

import os
import sys
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import parse_market_extra as pmx
import parse_deployment as pdep



MARKET_SYN = """
date=1940.9.10.13
version="Operation Postern v1.19.1.0"
player="GER"
equipments={
\tlight_tank_chassis={ id={ id=36 type=70 } }
\tsmall_plane_airframe={ id={ id=830 type=70 } }
}
equipment_market={
\tcontracts={
\t\tpurchase_contract={
\t\t\tid={ id=126 type=81 }
\t\t\tcontract_definition={
\t\t\t\tcontract_draft={
\t\t\t\t\tseller="ENG"
\t\t\t\t\tbuyer="FRA"
\t\t\t\t\tequipments={ equipment={ id={ id=4 type=70 } amount=11 } }
\t\t\t\t\tspeed=1
\t\t\t\t}
\t\t\t\tprice_levels={ levels={ } }
\t\t\t\tprices={ equipment={ id={ id=4 type=70 } amount=70 } }
\t\t\t}
\t\t}
\t}
\trequests={ 144 }
}
countries={
\tGER={
\t\tequipment_market={
\t\t\tmarket_stockpile={ equipments={ equipment={ id={ id=1 type=70 } amount=140 } } }
\t\t\tprice_levels={ levels={ } }
\t\t\tsubsidies={ }
\t\t\tmarket_request_automation={ option={ auto_accept_market_access=yes } }
\t\t}
\t}
\tROM={
\t\tequipment_market={
\t\t\tprice_levels={ levels={ } }
\t\t\tsubsidies={
\t\t\t\tsubsidies={
\t\t\t\t\t{
\t\t\t\t\t\tcic=20
\t\t\t\t\t\tarchetype=light_tank_chassis
\t\t\t\t\t\ttargets={ "CZE" }
\t\t\t\t\t}
\t\t\t\t}
\t\t\t}
\t\t}
\t}
\tHUN={
\t\tequipment_market={
\t\t\tprice_levels={ levels={ } }
\t\t\tsubsidies={
\t\t\t\tsubsidies={
\t\t\t\t\t{
\t\t\t\t\t\tcic=1000
\t\t\t\t\t\tarchetype=small_plane_airframe
\t\t\t\t\t\ttrigger="is_allied_seller_trigger"
\t\t\t\t\t}
\t\t\t\t}
\t\t\t}
\t\t}
\t}
\tBUL={
\t\tequipment_market={
\t\t\tmarket_stockpile={ equipments={ } }
\t\t}
\t}
}
"""


DEPLOY_SYN = """
date=1940.9.10.13
version="Operation Postern v1.19.1.0"
player="GER"
military_deployment_line_index=9001
military_deployment_conveyor_index=501
equipments={
\tinfantry_equipment_1={ id={ id=24 type=70 } }
\tartillery_equipment_1={ id={ id=1977 type=70 } }
}
countries={
\tENG={
\t\treinforcement={ priority=3 }
\t\tdeployment={
\t\t\tmilitary_deployment_conveyor={
\t\t\t\tid={ id=473 type=64 }
\t\t\t\tdivision_template_id={ id=902 type=52 }
\t\t\t\tname="Infantry 1936 L"
\t\t\t\tamount=1
\t\t\t\tlocation=6103
\t\t\t\tpriority=1
\t\t\t\trole=infantry
\t\t\t\tclosed=no
\t\t\t\tmilitary_deployment_line={
\t\t\t\t\tid={ id=3875 type=63 }
\t\t\t\t\tdivision_name={ type=0 name_order=974 }
\t\t\t\t\tamount=1
\t\t\t\t\tmilitary_deployment={
\t\t\t\t\t\ttraining=0.91264
\t\t\t\t\t\tmax_training=0.95312
\t\t\t\t\t\tarmy_manpower_value={ value={ tag="ENG" value=6300 } }
\t\t\t\t\t\tarmy_manpower_need={ value={ tag="ENG" value=6300 } }
\t\t\t\t\t\tequipment={
\t\t\t\t\t\t\tequipment={ id={ id=24 type=70 } amount=560 }
\t\t\t\t\t\t\tequipment={ id={ id=1977 type=70 } amount=50 }
\t\t\t\t\t\t}
\t\t\t\t\t\tmax_manpower=6300
\t\t\t\t\t}
\t\t\t\t}
\t\t\t}
\t\t\tinitial_carrier_air_wing_deployment={ cv_small_plane_airframe=0.5 }
\t\t\tdefault_hq_template={ id=3 type=52 }
\t\t\tunit_modifiers={ { type=rewards number=0 id=mobile_infantry } }
\t\t\tunlocked_subunits={ { "infantry" yes } }
\t\t}
\t\tunits={
\t\t\tdivision={ id={ id=10 type=43 } logical_country="ENG" location=6103 division_name={ type=0 name_order=1 } }
\t\t}
\t}
\tFRA={
\t\treinforcement={ priority=2 }
\t\tdeployment={
\t\t\tmilitary_deployment_conveyor={
\t\t\t\tid={ id=419 type=64 }
\t\t\t\tdivision_template_id={ id=577 type=52 }
\t\t\t\tname="Paratroopers 1936 A"
\t\t\t\tamount=1
\t\t\t\tlocation=5139
\t\t\t\tpriority=1
\t\t\t\trole=paratroopers
\t\t\t\tclosed=no
\t\t\t\tmilitary_deployment_line={
\t\t\t\t\tid={ id=3583 type=63 }
\t\t\t\t\tdivision_name={ type=0 name_order=1 }
\t\t\t\t\tamount=1
\t\t\t\t\tmilitary_deployment={
\t\t\t\t\t\tmax_training=0.9455
\t\t\t\t\t\tarmy_manpower_value={ value={ tag="FRA" value=10160 } }
\t\t\t\t\t\tarmy_manpower_need={ value={ tag="FRA" value=10160 } }
\t\t\t\t\t\tequipment={
\t\t\t\t\t\t\tequipment={ id={ id=24 type=70 } amount=2 }
\t\t\t\t\t\t\tequipment={ id={ id=99999 type=70 } amount=6 }
\t\t\t\t\t\t}
\t\t\t\t\t\tmax_manpower=10160
\t\t\t\t\t}
\t\t\t\t}
\t\t\t}
\t\t}
\t}
\tTUR={
\t\treinforcement={ }
\t}
\tSOV={
\t\treinforcement={ priority=1 }
\t\tunits={
\t\t\tdivision={ id={ id=20 type=43 } logical_country="ENG" location=222 division_name={ type=0 name_order=7 } }
\t\t\tdivision={ id={ id=21 type=43 } logical_country="SOV" location=223 division_name={ type=0 name_order=8 } }
\t\t}
\t}
}
"""


def _build_market():
    outputs, report = pmx.build(MARKET_SYN, Path("SYN_market.txt"), False)
    return outputs["market_extra.json"], report


def _build_deploy():
    outputs, report = pdep.build(DEPLOY_SYN, Path("SYN_deploy.txt"), False)
    return outputs["deployment.json"], report



def test_market_contract_price_levels_block_present_but_empty():
    out, _ = _build_market()
    st = out["meta"]["stats"]
    assert st["contracts_with_price_levels_block"] == 1
    assert st["contract_price_level_entries"] == 0
    assert st["nonempty_price_level_entries"] == 0
    c = out["contracts"][0]
    assert c["seller"] == "ENG" and c["buyer"] == "FRA"
    assert tuple(c["contract_id"]) == (126, 81)
    assert c["price_levels"] == []


def test_market_requests_list():
    out, _ = _build_market()
    assert out["requests"] == [144]
    assert out["meta"]["stats"]["pending_requests"] == 1


def test_market_subsidy_targets_list():
    out, _ = _build_market()
    rom = out["countries"]["ROM"]
    assert rom["subsidies_count"] == 1
    s = rom["subsidies"][0]
    assert s["cic"] == 20.0
    assert s["archetype"] == "light_tank_chassis"
    assert s["targets"] == ["CZE"]
    assert s["trigger"] is None


def test_market_subsidy_trigger_variant():
    out, _ = _build_market()
    hun = out["countries"]["HUN"]
    s = hun["subsidies"][0]
    assert s["cic"] == 1000.0
    assert s["archetype"] == "small_plane_airframe"
    assert s["targets"] == []
    assert s["trigger"] == "is_allied_seller_trigger"


def test_market_does_not_emit_stockpile_or_automation():
    out, _ = _build_market()
    assert "GER" not in out["countries"]
    assert "BUL" not in out["countries"]
    assert set(out["countries"]) == {"ROM", "HUN"}
    assert out["meta"]["stats"]["country_subsidies"] == 2


def test_market_no_path_leak():
    out, report = _build_market()
    blob = str(out) + report
    for bad in ("C:\\Users", "OneDrive", "/c/Users"):
        assert bad not in blob
    assert out["meta"]["save_date"] == "1940.9.10.13"



def test_deploy_conveyor_counts():
    out, _ = _build_deploy()
    st = out["meta"]["stats"]
    assert out["meta"]["military_deployment_line_index"] == "9001"
    assert out["meta"]["military_deployment_conveyor_index"] == "501"
    assert st["countries_with_conveyor"] == 2
    assert st["total_conveyors"] == 2
    assert st["total_deploying_unit_lines"] == 2
    assert st["total_equipment_in_transit_entries"] == 4


def test_deploy_conveyor_fields_and_equipment_join():
    out, _ = _build_deploy()
    eng = out["countries"]["ENG"]
    assert eng["conveyor_count"] == 1
    c = eng["conveyors"][0]
    assert c["ordinal"] == 0
    assert tuple(c["id"]) == (473, 64)
    assert c["name"] == "Infantry 1936 L"
    assert tuple(c["template_id"]) == (902, 52)
    assert c["location"] == 6103 and c["priority"] == 1
    assert c["role"] == "infantry" and c["closed"] is False
    ln = c["lines"][0]
    assert ln["ordinal"] == 0
    assert ln["training"] == 0.91264 and ln["max_training"] == 0.95312
    assert ln["manpower_have"] == 6300 and ln["max_manpower"] == 6300
    eq = ln["equipment_in_transit"]
    assert [e["ordinal"] for e in eq] == [0, 1]
    assert {e["archetype"] for e in eq} == {"infantry_equipment_1",
                                            "artillery_equipment_1"}
    amt = {e["archetype"]: e["amount"] for e in eq}
    assert amt["infantry_equipment_1"] == 560.0


def test_deploy_line_without_training_key():
    out, _ = _build_deploy()
    fra = out["countries"]["FRA"]
    ln = fra["conveyors"][0]["lines"][0]
    assert ln["training"] is None
    assert ln["max_training"] == 0.9455
    arches = {e["archetype"] for e in ln["equipment_in_transit"]}
    assert None in arches
    assert out["meta"]["stats"]["unknown_equipment_ids"] == 1


def test_deploy_skips_settings_blocks():
    out, _ = _build_deploy()
    eng = out["countries"]["ENG"]
    assert set(eng.keys()) == {"queue_complete", "reinforcement_priority", "conveyors",
                               "conveyor_count"}
    assert "default_hq_template" not in str(eng)
    assert "unlocked_subunits" not in str(eng)


def test_deploy_reinforcement_priority():
    out, _ = _build_deploy()
    assert out["countries"]["ENG"]["reinforcement_priority"] == 3
    assert out["countries"]["FRA"]["reinforcement_priority"] == 2
    assert out["countries"]["SOV"]["reinforcement_priority"] == 1
    assert out["countries"]["TUR"]["queue_complete"] is True
    assert out["countries"]["TUR"]["conveyors"] == []
    assert out["meta"]["stats"]["countries_with_reinforcement_priority"] == 3



def test_expeditionary_lent_division_detected():
    out, _ = _build_deploy()
    st = out["meta"]["stats"]
    assert st["expeditionary_pairs"] == 1
    assert st["expeditionary_divisions"] == 1
    e = out["expeditionary"][0]
    assert e["owner"] == "ENG" and e["controller"] == "SOV"
    assert e["division_count"] == 1
    d = e["divisions"][0]
    assert d["div_id"] == 20 and d["div_type"] == 43
    assert d["location"] == 222 and d["name_order"] == 7


def test_expeditionary_own_units_not_flagged():
    out, _ = _build_deploy()
    assert out["meta"]["stats"]["divisions_scanned"] == 3
    pairs = {(e["owner"], e["controller"]) for e in out["expeditionary"]}
    assert ("ENG", "ENG") not in pairs
    assert ("SOV", "SOV") not in pairs


def test_deploy_no_path_leak():
    out, report = _build_deploy()
    blob = str(out) + report
    for bad in ("C:\\Users", "OneDrive", "/c/Users"):
        assert bad not in blob
    assert "DEPLOYMENT" in report.upper()



_MELT = (Path(__file__).resolve().parent.parent
         / "re_scratch" / "campaign_melt_1940.txt")


def test_real_melt_market_extra_if_present():
    if not _MELT.is_file():
        return
    text = _MELT.read_text(encoding="utf-8", errors="replace")
    outputs, _ = pmx.build(text, _MELT, False)
    out = outputs["market_extra.json"]
    st = out["meta"]["stats"]
    assert st["contracts_with_price_levels_block"] == 7
    assert st["nonempty_price_level_entries"] == 0
    assert st["pending_requests"] == 1
    assert out["requests"] == [144]
    assert set(out["countries"]) == {"HUN", "PER", "ROM", "YUG"}
    assert st["country_subsidies"] == 4
    assert out["countries"]["ROM"]["subsidies"][0]["targets"] == ["CZE"]
    assert (out["countries"]["HUN"]["subsidies"][0]["trigger"]
            == "is_allied_seller_trigger")
    assert st["unknown_equipment_ids"] == 0


def test_real_melt_deployment_if_present():
    if not _MELT.is_file():
        return
    text = _MELT.read_text(encoding="utf-8", errors="replace")
    outputs, _ = pdep.build(text, _MELT, False)
    out = outputs["deployment.json"]
    st = out["meta"]["stats"]
    assert st["countries_with_conveyor"] == 12
    assert st["total_conveyors"] == 25
    assert st["total_deploying_unit_lines"] == 57
    assert st["total_equipment_in_transit_entries"] == 226
    assert st["countries_with_reinforcement_priority"] == 45
    assert st["expeditionary_pairs"] == 0
    assert st["divisions_scanned"] == 1654
    assert st["unknown_equipment_ids"] == 0
    eng = out["countries"]["ENG"]
    names = {c["name"] for c in eng["conveyors"]}
    assert "Infantry 1936 L" in names


def _run_standalone():
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
