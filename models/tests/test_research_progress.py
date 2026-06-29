from __future__ import annotations

import math

from models import research_progress as RP


TECH_DEFS = {
    "basic_machine_tools": {
        "research_cost": 1.5,
        "start_year": 1936,
        "categories": ["industry"],
    },
    "electronic_mechanical_engineering": {
        "research_cost": 1,
        "start_year": 1936,
        "categories": ["electronics"],
    },
    "future_radar": {
        "research_cost": 2,
        "start_year": 1940,
        "categories": ["electronics"],
    },
}


def test_save_active_slot_computes_fraction_and_eta():
    research = {
        "in_progress": [{
            "name": "electronic_mechanical_engineering",
            "points_factor": 1.25,
        }],
        "technologies": [{
            "name": "electronic_mechanical_engineering",
            "level": None,
            "date": RP.IN_PROGRESS_TECH_DATE,
            "research_points": 55.0,
        }],
    }

    stats = RP.enrich_country_research_progress(
        research, TECH_DEFS, current_year=1936)

    slot = research["in_progress"][0]
    assert stats["fraction_computed"] == 1
    assert stats["eta_computed"] == 1
    assert slot["progress_points"] == 55.0
    assert slot["required_points_base"] == 110.0
    assert slot["progress_pct"] == 0.5
    assert slot["eta_days"] == math.ceil(55.0 / 1.25)
    assert slot["progress_basis"] == \
        "save_active_research_points/static_research_cost"


def test_live_slot_computes_fraction_but_does_not_use_save_eta():
    research = {
        "in_progress": [{
            "name": "basic_machine_tools",
            "points_factor": 99.0,
        }],
        "in_progress_live": [{
            "slot_index": 0,
            "tech_handle_raw": 0x1234,
            "tech_name": "basic_machine_tools",
            "progress_raw": 82500,
            "progress_units": 82.5,
        }],
    }

    RP.enrich_country_research_progress(research, TECH_DEFS, current_year=1936)

    slot = research["in_progress_live"][0]
    assert slot["tech_handle_raw"] == 0x1234
    assert slot["progress_raw"] == 82500
    assert slot["progress_pct"] == 0.5
    assert slot["required_points_base"] == 165.0
    assert "eta_days" not in slot
    assert slot["eta_unavailable_reason"] == "missing_live_points_factor"


def test_live_slot_can_compute_eta_when_live_speed_is_present():
    research = {
        "in_progress_live": [{
            "slot_index": 0,
            "tech_name": "basic_machine_tools",
            "progress_units": 82.5,
            "points_factor": 1.5,
        }],
    }

    RP.enrich_country_research_progress(research, TECH_DEFS, current_year=1936)

    slot = research["in_progress_live"][0]
    assert slot["progress_pct"] == 0.5
    assert slot["eta_days"] == math.ceil(82.5 / 1.5)


def test_live_slot_prefers_dll_base_required_points():
    research = {
        "in_progress_live": [{
            "slot_index": 0,
            "live_progress_units": 55.0,
            "base_required_points_units": 110.0,
        }],
    }

    RP.enrich_country_research_progress(research, None, current_year=1936)

    slot = research["in_progress_live"][0]
    assert slot["required_points_base"] == 110.0
    assert slot["remaining_points_base"] == 55.0
    assert slot["progress_pct"] == 0.5
    assert slot["progress_basis"] == \
        "live_progress_units/dll_base_required_points"


def test_live_slot_beyond_dll_base_required_points_fails_closed():
    research = {
        "in_progress_live": [{
            "slot_index": 0,
            "live_progress_units": 111.0,
            "base_required_points_units": 110.0,
        }],
    }

    RP.enrich_country_research_progress(research, None, current_year=1936)

    slot = research["in_progress_live"][0]
    assert "progress_pct" not in slot
    assert slot["progress_unavailable_reason"] == \
        "progress_exceeds_dll_base_required_points"


def test_save_slot_missing_active_progress_fails_closed():
    research = {
        "in_progress": [{
            "name": "basic_machine_tools",
            "points_factor": 1.0,
            "progress_pct": 0.5,
            "eta_days": 44,
            "required_points_base": 165.0,
            "remaining_points_base": 82.5,
            "progress_basis": "stale",
        }],
        "technologies": [],
    }

    RP.enrich_country_research_progress(research, TECH_DEFS, current_year=1936)

    slot = research["in_progress"][0]
    assert "progress_pct" not in slot
    assert "eta_days" not in slot
    assert "required_points_base" not in slot
    assert "remaining_points_base" not in slot
    assert "progress_basis" not in slot
    assert slot["progress_unavailable_reason"] == \
        "missing_active_research_points"


def test_save_slot_eta_unavailable_clears_stale_eta_only():
    research = {
        "in_progress": [{
            "name": "basic_machine_tools",
            "points_factor": 0.0,
            "eta_days": 44,
        }],
        "technologies": [{
            "name": "basic_machine_tools",
            "level": None,
            "date": RP.IN_PROGRESS_TECH_DATE,
            "research_points": 82.5,
        }],
    }

    RP.enrich_country_research_progress(research, TECH_DEFS, current_year=1936)

    slot = research["in_progress"][0]
    assert slot["progress_pct"] == 0.5
    assert "eta_days" not in slot
    assert slot["eta_unavailable_reason"] == "missing_positive_points_factor"


def test_missing_live_tech_name_fails_closed():
    research = {
        "in_progress_live": [{
            "slot_index": 0,
            "progress_units": 10.0,
            "progress_pct": 0.5,
            "eta_days": 44,
        }],
    }

    RP.enrich_country_research_progress(research, TECH_DEFS, current_year=1936)

    slot = research["in_progress_live"][0]
    assert "progress_pct" not in slot
    assert "eta_days" not in slot
    assert slot["progress_unavailable_reason"] == "missing_tech_name"


def test_ahead_of_time_effective_cost_fails_closed():
    research = {
        "in_progress": [{"name": "future_radar", "points_factor": 1.0}],
        "technologies": [{
            "name": "future_radar",
            "level": None,
            "date": RP.IN_PROGRESS_TECH_DATE,
            "research_points": 10.0,
        }],
    }

    RP.enrich_country_research_progress(research, TECH_DEFS, current_year=1936)

    slot = research["in_progress"][0]
    assert "progress_pct" not in slot
    assert slot["progress_unavailable_reason"] == \
        "ahead_of_time_effective_cost_unavailable"


def test_cost_reduction_context_fails_closed_but_speed_bonus_does_not():
    research = {
        "in_progress": [{"name": "basic_machine_tools", "points_factor": 1.0}],
        "technologies": [{
            "name": "basic_machine_tools",
            "level": None,
            "date": RP.IN_PROGRESS_TECH_DATE,
            "research_points": 10.0,
        }],
        "research_bonuses": [{
            "kind": "research_bonus",
            "categories": ["industry"],
            "bonus": 0.5,
        }],
    }

    RP.enrich_country_research_progress(research, TECH_DEFS, current_year=1936)
    assert "progress_pct" in research["in_progress"][0]

    research["in_progress"][0].pop("progress_pct", None)
    research["research_bonuses"] = [{
        "kind": "cost_reduction",
        "categories": ["industry"],
        "cost_reduction": 0.5,
    }]
    RP.enrich_country_research_progress(research, TECH_DEFS, current_year=1936)
    assert research["in_progress"][0]["progress_unavailable_reason"] == \
        "cost_reduction_effective_cost_unavailable"


def test_progress_beyond_static_cost_fails_closed():
    research = {
        "in_progress_live": [{
            "slot_index": 0,
            "tech_name": "electronic_mechanical_engineering",
            "progress_units": 111.0,
        }],
    }

    RP.enrich_country_research_progress(research, TECH_DEFS, current_year=1936)

    slot = research["in_progress_live"][0]
    assert "progress_pct" not in slot
    assert slot["progress_unavailable_reason"] == "progress_exceeds_static_cost"
