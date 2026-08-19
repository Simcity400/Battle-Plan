#!/usr/bin/env python3
"""Tests for parse_abilities.py (commander-abilities + aces + officer-corps).

Self-contained: drives the parser's ``build`` over a SYNTHETIC melted-save text
that reproduces the real 1.19 shapes (verified against a campaign melt), so the
suite runs with no save file present. If the real campaign melt happens to be on
disk, one extra test cross-checks the known GER ace + ace totals against it.

Run:  python -m pytest save_oracle/test_parse_abilities.py
  or:  python save_oracle/test_parse_abilities.py
"""
from __future__ import annotations

import os
import sys
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import parse_abilities as pa


SYN = """
date=1940.9.10.13
version="Operation Postern v1.19.1.0"
player="GER"
character_manager={
\thistorical={
\t\tcharacter={
\t\t\tid={ id=39 type=73 }
\t\t\ttoken="GER_a"
\t\t\tcountry="GER"
\t\t\tfield_marshal={
\t\t\t\tid={ id=3 type=4713 }
\t\t\t\tskill=3
\t\t\t\tcooldown_reason="reassigned"
\t\t\t\tleader_modifier_enable_date=1940.9.17.20
\t\t\t\tleader_cooldown_start_date=1940.9.2.20
\t\t\t\ttraits={ organizer logistics_wizard }
\t\t\t}
\t\t}
\t\tcharacter={
\t\t\tid={ id=40 type=73 }
\t\t\tcountry="ROM"
\t\t\tcorps_commander={
\t\t\t\tid={ id=25938 type=4713 }
\t\t\t\tskill=2
\t\t\t\tcaptured=yes
\t\t\t\tcaptured_by="BUL"
\t\t\t\tlocation=9783
\t\t\t}
\t\t}
\t\tcharacter={
\t\t\tid={ id=41 type=73 }
\t\t\tcountry="GER"
\t\t\tcorps_commander={
\t\t\t\tid={ id=99 type=4713 }
\t\t\t\tskill=4
\t\t\t}
\t\t}
\t}
}
countries={
\tGER={
\t\tace={
\t\t\tid={ id=12212 type=4713 }
\t\t\tmodifier="fighter_good"
\t\t\tname="Gunther"
\t\t\tsurname="Strohmann"
\t\t\tcallsign="Bubi"
\t\t\tportrait=73
\t\t\talive=no
\t\t\thandled=yes
\t\t\tkill_type=1
\t\t\tkiller_name={ id=19803 type=4713 }
\t\t\tkiller_country="ENG"
\t\t}
\t\tace={
\t\t\tid={ id=13250 type=4713 }
\t\t\tmodifier="fighter_genius"
\t\t\tname="Theodor"
\t\t\tsurname="Lissmann"
\t\t\tcallsign="Pritzl"
\t\t\tportrait=142
\t\t}
\t\tpolitics={
\t\t\tideas={
\t\t\t\twar_economy mobile_warfare_academy_spirit signals_training_spirit GER_some_national_idea
\t\t\t}
\t\t\ttimed_idea={ idea="signals_training_spirit" days=30 }
\t\t\truling_party=fascism
\t\t}
\t}
\tENG={
\t\tace={
\t\t\tid={ id=400 type=4713 }
\t\t\tmodifier="support_unique"
\t\t\tname="Reg"
\t\t\tsurname="Smith"
\t\t\tcallsign="Ace"
\t\t\tportrait=5
\t\t}
\t\tpolitics={
\t\t\tideas={ best_of_the_best_spirit }
\t\t}
\t}
\tBUL={
\t\tpolitics={ ideas={ } }
\t}
}
"""


def _build():
    outputs, report = pa.build(SYN, Path("SYN_melted.txt"), False)
    return outputs["abilities.json"], report



def test_ace_counts():
    out, _ = _build()
    st = out["meta"]["stats"]
    assert st["total_aces"] == 3
    assert st["countries_with_aces"] == 2
    assert st["aces_alive"] == 2 and st["aces_dead"] == 1
    assert out["aces"]["GER"]["count"] == 2
    assert out["aces"]["ENG"]["count"] == 1
    assert "BUL" not in out["aces"]


def test_ace_field_mapping():
    out, _ = _build()
    a = out["aces"]["GER"]["aces"][0]
    assert a["ace_id"] == 12212
    assert a["name"] == "Gunther" and a["surname"] == "Strohmann"
    assert a["callsign"] == "Bubi"
    assert a["modifier"] == "fighter_good"
    assert a["ace_category"] == "fighter" and a["tier"] == "good"
    assert a["alive"] is False
    assert a["kill_type"] == 1
    assert a["killer_id"] == 19803
    assert a["killer_type"] == 4713
    assert a["killer_country"] == "ENG"


def test_ace_alive_defaults_true_when_absent():
    out, _ = _build()
    a = out["aces"]["GER"]["aces"][1]
    assert a["alive"] is True
    assert a["ace_category"] == "fighter" and a["tier"] == "genius"
    assert a["killer_id"] is None and a["killer_country"] is None


def test_ace_modifier_split_support():
    out, _ = _build()
    a = out["aces"]["ENG"]["aces"][0]
    assert a["ace_category"] == "support" and a["tier"] == "unique"


def test_ace_modifier_split_last_underscore():
    cat, tier = pa._split_modifier("heavy_fighter_genius")
    assert cat == "heavy_fighter" and tier == "genius"
    cat2, tier2 = pa._split_modifier("fighter_experimental")
    assert cat2 == "fighter" and tier2 == "experimental"



def test_gated_commander_extracted():
    out, _ = _build()
    st = out["meta"]["stats"]
    assert st["gated_commanders"] == 2
    ger = out["commander_abilities"]["GER"]
    assert len(ger) == 1
    g = ger[0]
    assert g["role"] == "field_marshal"
    assert g["unit_leader_id"] == 3
    assert g["cooldown_reason"] == "reassigned"
    assert g["cooldown_start_date"] == "1940.9.2.20"
    assert g["modifier_enable_date"] == "1940.9.17.20"
    assert g["captured"] is False


def test_uncooled_commander_omitted():
    out, _ = _build()
    ids = {c["unit_leader_id"] for c in out["commander_abilities"]["GER"]}
    assert 99 not in ids


def test_captured_commander_surfaced():
    out, _ = _build()
    st = out["meta"]["stats"]
    assert st["captured_commanders"] == 1
    rom = out["commander_abilities"]["ROM"]
    assert rom[0]["captured"] is True
    assert rom[0]["captured_by"] == "BUL"
    assert rom[0]["location"] == 9783



def test_officer_corps_spirits_filtered():
    out, _ = _build()
    ger = out["officer_corps"]["GER"]
    toks = {s["token"] for s in ger["officer_corps_spirits"]}
    assert "mobile_warfare_academy_spirit" in toks
    assert "signals_training_spirit" in toks
    assert "war_economy" not in toks
    assert ger["spirit_count"] == 2


def test_officer_corps_timed_spirit_days():
    out, _ = _build()
    ger = out["officer_corps"]["GER"]
    timed = {s["token"]: s["days_remaining"]
             for s in ger["officer_corps_spirits"]}
    assert timed["signals_training_spirit"] == 30
    assert timed["mobile_warfare_academy_spirit"] is None


def test_captured_general_tallies():
    out, _ = _build()
    assert out["officer_corps"]["BUL"]["captured_enemy_generals"] == 1
    assert out["officer_corps"]["ROM"]["own_generals_captured"] == 1


def test_empty_officer_corps_country_omitted():
    out, _ = _build()
    assert "BUL" in out["officer_corps"]



def test_meta_and_no_path_leak():
    out, report = _build()
    m = out["meta"]
    assert m["save_date"] == "1940.9.10.13"
    assert m["player"] == "GER"
    blob = str(out) + report
    for bad in ("C:\\Users", "OneDrive", "/c/Users"):
        assert bad not in blob
    assert "ACES" in report and "ABILITIES" in report.upper()



def test_real_campaign_melt_if_present():
    melt = (Path(__file__).resolve().parent.parent
            / "re_scratch" / "campaign_melt_1940.txt")
    if not melt.is_file():
        return
    text = melt.read_text(encoding="utf-8", errors="replace")
    outputs, _ = pa.build(text, melt, False)
    out = outputs["abilities.json"]
    st = out["meta"]["stats"]
    assert st["total_aces"] == 38
    assert st["countries_with_aces"] == 7
    assert out["aces"]["GER"]["count"] == 18
    a0 = out["aces"]["GER"]["aces"][0]
    assert a0["name"] == "Gunther" and a0["surname"] == "Strohmann"
    assert a0["callsign"] == "Bubi" and a0["killer_country"] == "ENG"
    assert st["gated_commanders"] == 162
    assert st["captured_commanders"] == 1


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
