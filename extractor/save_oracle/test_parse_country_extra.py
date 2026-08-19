#!/usr/bin/env python3
"""Focused tests for parse_country_extra.py.

These are synthetic melted-save snippets, so they do not need a local the game save.
"""
from __future__ import annotations

import os
import sys
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import parse_country_extra as pce


SYN = """
date=1940.9.10.13
version="Operation Postern v1.19.1.0"
player="GER"
countries={
    GER={
        flags={
            GER_has_started_war={ value=1 date=1939.9.2.1 }
            GER_congress_test={ value=3 date=1940.1.1.12 }
        }
        strategic_navy={
            max_allowed_dockyards_to_repair=22
            num_used_dockyards=0
            regional_convoys={
                12={ required=3 assigned=5 }
                42={ required=0 assigned=0 }
            }
            naval_transport={
                id={ id=100 type=4713 }
                path={ 1 2 3 }
            }
            naval_transport={
                id={ id=101 type=4713 }
                active=yes
            }
            convoy_escort_presence_history={
                1940.9.10.13
                { region=12 presence=0.5 }
            }
        }
        convoys_destroyed=7
        dynamic_modifier={
            modifier={
                modifier="GER_timed_test"
                tag="D04"
                value={ 0.15 0.1 0 }
                enabled=yes
                days=716
            }
            modifier={ modifier="GER_permanent_test" enabled=yes }
        }
    }
    ENG={
        flags={ }
    }
    SOV={
        convoys_destroyed=0
    }
}
"""


def _build():
    outputs, report = pce.build(SYN, Path("SYN_melted.txt"), False)
    return outputs["country_extra.json"], report


def test_country_script_flags_keep_value_and_date():
    out, report = _build()
    ger_flags = out["countries"]["GER"]["script_flags"]
    assert ger_flags["GER_has_started_war"] == {
        "value": 1, "date": "1939.9.2.1"}
    assert ger_flags["GER_congress_test"] == {
        "value": 3, "date": "1940.1.1.12"}
    assert out["meta"]["stats"]["script_flags"] == 2
    assert "GER flags     : 2" in report


def test_empty_flags_block_is_measured_empty_but_absent_block_is_omitted():
    out, _ = _build()
    assert out["countries"]["ENG"]["script_flags"] == {}
    assert "script_flags" not in out["countries"]["SOV"]
    assert out["meta"]["stats"]["countries_with_script_flags_block"] == 2


def test_dynamic_modifier_preserves_timed_days_and_optional_tag():
    out, _ = _build()
    timed, permanent = out["countries"]["GER"]["dynamic_modifiers"]
    assert timed == {
        "modifier": "GER_timed_test",
        "enabled": True,
        "value": [0.15, 0.1, 0.0],
        "days": 716,
        "tag": "D04",
    }
    assert permanent["modifier"] == "GER_permanent_test"
    assert permanent["days"] is None
    assert permanent["tag"] is None


def test_strategic_navy_preserves_nested_save_shape_and_repeated_blocks():
    out, report = _build()
    sn = out["countries"]["GER"]["strategic_navy"]
    assert sn["max_allowed_dockyards_to_repair"] == 22
    assert sn["num_used_dockyards"] == 0
    assert sn["regional_convoys"]["12"] == {"required": 3, "assigned": 5}
    assert sn["regional_convoys"]["42"] == {"required": 0, "assigned": 0}
    assert len(sn["naval_transport"]) == 2
    assert sn["naval_transport"][0]["id"] == {"id": 100, "type": 4713}
    assert sn["naval_transport"][0]["path"] == [1, 2, 3]
    assert sn["naval_transport"][1]["active"] is True
    assert sn["convoy_escort_presence_history"]["_items"][0] == "1940.9.10.13"
    assert sn["convoy_escort_presence_history"]["_items"][1] == {
        "region": 12, "presence": 0.5}
    assert "strategic_navy" not in out["countries"]["SOV"]
    assert out["meta"]["stats"]["countries_with_strategic_navy"] == 1
    assert "GER strat-navy: present=True dockyards=22" in report


def test_nuke_amount_uses_exact_a729_fixed_point_consumer_and_keeps_raw_oracle():
    text = """
countries={
    GER={
        nukes={
            { amount=8000 nukes_ready=0 }
            { amount=250000000 nukes_ready=1 }
        }
    }
}
"""
    outputs, _ = pce.build(text, Path("SYN_nukes_melted.txt"), False)
    ger = outputs["country_extra.json"]["countries"]["GER"]
    assert ger["nukes"] == [
        {"amount": 0, "amount_raw": 8000, "nukes_ready": 0},
        {"amount": 2, "amount_raw": 250000000, "nukes_ready": 1},
    ]
    assert ger["nukes_total"] == 2
    assert ger["nukes_ready_total"] == 1


def test_no_path_leak():
    out, report = _build()
    blob = str(out) + report
    win_home = "C:" + chr(92) + "Users"
    cloud_docs = "One" + "Drive"
    msys_home = "/c/" + "Users"
    for bad in (win_home, cloud_docs, msys_home):
        assert bad not in blob
