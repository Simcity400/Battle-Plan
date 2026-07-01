"""Proof gate for live collector session manifests.

This module is intentionally read-only: it validates the recorder's existing
``session_manifest.json`` so new DLL-backed layers can share one acceptance
check during live speed-five proof runs.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional


SCHEDULED_DOMAIN_FRAME_COUNT_KEYS = {
    "focus_live": "scheduled_focus_domain_frames",
    "tech_live": "scheduled_tech_domain_frames",
    "projects_live": "scheduled_projects_domain_frames",
    "decisions_live": "scheduled_decisions_domain_frames",
    "diplomacy_live": "scheduled_diplomacy_domain_frames",
    "politics_live": "scheduled_politics_domain_frames",
    "politics_residuals": "scheduled_politics_residuals_domain_frames",
    "production_live": "scheduled_production_domain_frames",
    "state_ownership": "state_ownership_frames",
    "state_control": "state_control_frames",
    "state_detail": "scheduled_state_detail_domain_frames",
    "province_control": "province_control_frames",
    "buildings_live": "scheduled_buildings_domain_frames",
}

DIVISION_RAW_EVENT_TYPES = (
    "division_movement_started",
    "division_movement_stopped",
    "division_org_strength_delta",
    "division_combat_ref_changed",
)

LAND_COMBAT_RAW_EVENT_TYPES = (
    "land_combat_started",
    "land_combat_updated",
    "land_combat_ended",
)

ARMY_ORDER_RAW_EVENT_TYPES = (
    "army_order_added",
    "army_order_updated",
    "army_order_removed",
)

def load_session_manifest(path: Path) -> Dict[str, Any]:
    payload = json.loads(Path(path).read_text(encoding="utf-8"))
    if not isinstance(payload, dict):
        raise ValueError("session manifest root is not an object")
    return payload


def collector_session_proof_errors(
    manifest: Dict[str, Any],
    *,
    required_snapshot_domains: Iterable[str] = (),
    required_scheduled_snapshot_domains: Iterable[str] = (),
    required_raw_delta_domains: Iterable[str] = (),
    required_raw_event_types: Iterable[str] = (),
    min_game_hours: Optional[int] = None,
    require_replay_verified: bool = True,
    require_hourly: bool = True,
    require_no_raw_gaps: bool = True,
    require_no_lightweight_heavy_payloads: bool = True,
    require_diplomacy_war_relations: bool = False,
    require_politics_leaders: bool = False,
    require_politics_idea_classification: bool = False,
    require_politics_advisor_residuals: bool = False,
    require_projects_live_payload: bool = False,
    require_projects_live_links: bool = False,
    require_production_construction: bool = False,
    require_state_resources: bool = False,
    require_tech_live_names: bool = False,
    require_tech_live_progress: bool = False,
    require_tech_live_static_cost_progress: bool = False,
    require_tech_live_dll_base_progress: bool = False,
    require_tech_live_eta: bool = False,
    require_division_raw_telemetry: bool = False,
    require_land_combat_raw_lifecycle: bool = False,
    require_army_order_raw_telemetry: bool = False,
    require_army_order_raw_removals: bool = False,
    require_army_order_raw_updates: bool = False,
    require_air_wing_raw_telemetry: bool = False,
    require_ship_raw_telemetry: bool = False,
    require_oob_command_raw_telemetry: bool = False,
    require_oob_command_leader_detail: bool = False,
) -> List[str]:
    errors: List[str] = []
    session_errors = manifest.get("session_errors")
    if session_errors:
        errors.append("session_errors present: " + "; ".join(map(str, session_errors[:8])))

    if require_replay_verified and manifest.get("replay_verified") is not True:
        errors.append("replay_verified is not true")

    if min_game_hours is not None:
        advanced = _as_int(manifest.get("game_hours_advanced"))
        if advanced is None or advanced < int(min_game_hours):
            errors.append(
                f"game_hours_advanced {advanced!r} below required {int(min_game_hours)}"
            )

    hourly = manifest.get("hourly_requirement")
    if require_hourly:
        if not isinstance(hourly, dict):
            errors.append("hourly_requirement missing")
        elif hourly.get("met") is not True:
            problems = hourly.get("problems")
            suffix = ""
            if isinstance(problems, list) and problems:
                suffix = ": " + "; ".join(map(str, problems[:8]))
            errors.append("hourly_requirement not met" + suffix)

    summary = _payload_summary(manifest)
    snapshot_domains = _string_set(summary.get("domains_seen"))
    for domain in required_snapshot_domains:
        if str(domain) not in snapshot_domains:
            errors.append(f"snapshot domain {domain!r} not observed")
    for domain in required_scheduled_snapshot_domains:
        frame_count = _scheduled_domain_frame_count(summary, str(domain))
        if frame_count is None:
            errors.append(f"scheduled snapshot domain {domain!r} is not countable")
        elif frame_count <= 0:
            errors.append(f"scheduled snapshot domain {domain!r} not observed")

    raw_delta_domains = _string_set(summary.get("raw_delta_domains_seen"))
    raw_gap_domain_counts = _int_map(summary.get("raw_gap_domain_counts"))
    for domain in required_raw_delta_domains:
        raw_domain = str(domain)
        if raw_domain not in raw_delta_domains:
            errors.append(f"raw delta domain {domain!r} not observed")
        if require_no_raw_gaps:
            gap_count = raw_gap_domain_counts.get(raw_domain, 0)
            if gap_count > 0:
                errors.append(
                    f"raw delta domain {domain!r} emitted gap frames: {gap_count}"
                )

    raw_event_types = _string_set(summary.get("raw_event_types_seen"))
    for event_type in required_raw_event_types:
        if str(event_type) not in raw_event_types:
            errors.append(f"raw event type {event_type!r} not observed")

    if require_no_lightweight_heavy_payloads:
        heavy_on_light = _as_int(summary.get("heavy_payload_on_lightweight_frames"))
        if heavy_on_light not in (0, None):
            errors.append(
                "heavy payloads appeared on lightweight frames: "
                f"{heavy_on_light}"
            )
    if require_diplomacy_war_relations:
        scheduled = _scheduled_domain_frame_count(summary, "diplomacy_live")
        required_frames = int(min_game_hours or 1)
        if scheduled is None or scheduled < required_frames:
            errors.append(
                "diplomacy war proof scheduled frames "
                f"{scheduled!r} below required {required_frames}"
            )
        if manifest.get("war_relation_bridge_supplied") is not True:
            errors.append("war_relation_bridge was not supplied")
        if _as_int(manifest.get("war_relation_bridge_rows")) in (None, 0):
            errors.append("war_relation_bridge_rows not positive")
        if _as_int(summary.get("diplomacy_war_relation_frames")) is None or _as_int(
            summary.get("diplomacy_war_relation_frames")
        ) < required_frames:
            errors.append("diplomacy war relation frames below required cadence")
        if _as_int(summary.get("max_diplomacy_war_relations_per_frame")) in (None, 0):
            errors.append("diplomacy war relations were not observed")
        if _as_int(summary.get("country_rows_with_diplomacy_at_war")) in (None, 0):
            errors.append("country diplomacy at_war rows were not observed")
        if _as_int(summary.get("country_rows_with_diplomacy_casualties")) in (None, 0):
            errors.append("country diplomacy casualty rows were not observed")
        if _as_int(summary.get("max_diplomacy_casualty_sum_per_frame")) in (None, 0):
            errors.append("positive diplomacy casualty sum was not observed")
        if _as_int(summary.get("diplomacy_war_subtype_warning_frames")) not in (0, None):
            errors.append("diplomacy war subtype warnings observed")
        if _as_int(summary.get("diplomacy_on_lightweight_frames")) not in (0, None):
            errors.append("diplomacy payloads appeared on lightweight frames")
    if require_politics_leaders:
        scheduled = _scheduled_domain_frame_count(summary, "politics_live")
        required_frames = int(min_game_hours or 1)
        if scheduled is None or scheduled < required_frames:
            errors.append(
                "politics leader proof scheduled frames "
                f"{scheduled!r} below required {required_frames}"
            )
        if _as_int(summary.get("politics_leader_frames")) is None or _as_int(
            summary.get("politics_leader_frames")
        ) < required_frames:
            errors.append("politics leader frames below required cadence")
        if _as_int(summary.get("country_rows_with_politics_leader")) in (None, 0):
            errors.append("country politics leader rows were not observed")
        if _as_int(summary.get("max_politics_leader_rows_per_frame")) in (None, 0):
            errors.append("politics leader rows were empty in every frame")
        if _as_int(summary.get("politics_on_lightweight_frames")) not in (0, None):
            errors.append("politics payloads appeared on lightweight frames")
    if require_politics_idea_classification:
        scheduled = _scheduled_domain_frame_count(summary, "politics_live")
        required_frames = int(min_game_hours or 1)
        if scheduled is None or scheduled < required_frames:
            errors.append(
                "politics idea-classification proof scheduled frames "
                f"{scheduled!r} below required {required_frames}"
            )
        if manifest.get("idea_group_filter_supplied") is not True:
            errors.append("idea_group_filter was not supplied")
        if _as_int(manifest.get("idea_group_filter_rows")) in (None, 0):
            errors.append("idea_group_filter_rows not positive")
        if _as_int(summary.get("frames_with_politics_payloads")) is None or _as_int(
            summary.get("frames_with_politics_payloads")
        ) < required_frames:
            errors.append("politics payload frames below required cadence")
        if _as_int(summary.get("country_rows_with_politics_payloads")) in (None, 0):
            errors.append("country politics payload rows were not observed")
        if _as_int(summary.get("country_rows_with_politics_ideas")) in (None, 0):
            errors.append("country politics active idea rows were not observed")
        if _as_int(summary.get("max_politics_idea_rows_per_country")) in (None, 0):
            errors.append("active politics ideas were empty in every country row")
        if _as_int(summary.get("country_rows_with_politics_laws")) in (None, 0):
            errors.append("country politics law rows were not observed")
        if _as_int(summary.get("max_politics_law_keys_per_country")) in (None, 0):
            errors.append("politics law buckets were empty in every country row")
        if _as_int(summary.get("country_rows_with_politics_idea_groups")) in (
            None,
            0,
        ):
            errors.append("country politics idea-group rows were not observed")
        if _as_int(summary.get("max_politics_idea_group_keys_per_country")) in (
            None,
            0,
        ):
            errors.append("politics idea-group buckets were empty in every country row")
        if _as_int(summary.get("country_rows_with_politics_advisor_ideas")) in (
            None,
            0,
        ):
            errors.append("country politics advisor-idea rows were not observed")
        if _as_int(summary.get("max_politics_advisor_idea_keys_per_country")) in (
            None,
            0,
        ):
            errors.append("politics advisor-idea buckets were empty in every country row")
        if _as_int(summary.get("max_politics_advisor_idea_tokens_per_country")) in (
            None,
            0,
        ):
            errors.append("politics advisor-idea tokens were empty in every country row")
        if _as_int(summary.get("country_rows_with_politics_ideas_other")) in (None, 0):
            errors.append("country politics non-law idea rows were not observed")
        if _as_int(summary.get("max_politics_ideas_other_tokens_per_country")) in (
            None,
            0,
        ):
            errors.append("politics non-law ideas were empty in every country row")
        if _as_int(summary.get("country_rows_with_politics_national_spirits")) in (
            None,
            0,
        ):
            errors.append("country politics national-spirit rows were not observed")
        if _as_int(summary.get("max_politics_national_spirit_tokens_per_country")) in (
            None,
            0,
        ):
            errors.append("politics national spirits were empty in every country row")
        if _as_int(summary.get("politics_on_lightweight_frames")) not in (0, None):
            errors.append("politics payloads appeared on lightweight frames")
    if require_politics_advisor_residuals:
        scheduled = _scheduled_domain_frame_count(summary, "politics_residuals")
        required_frames = int(min_game_hours or 1)
        if scheduled is None or scheduled < required_frames:
            errors.append(
                "politics advisor-residual proof scheduled frames "
                f"{scheduled!r} below required {required_frames}"
            )
        if manifest.get("advisor_token_filter_supplied") is not True:
            errors.append("advisor_token_filter was not supplied")
        if _as_int(manifest.get("advisor_token_filter_tokens")) in (None, 0):
            errors.append("advisor_token_filter_tokens not positive")
        if _as_int(
            summary.get("frames_with_politics_residual_payloads")
        ) is None or _as_int(
            summary.get("frames_with_politics_residual_payloads")
        ) < required_frames:
            errors.append("politics residual payload frames below required cadence")
        if _as_int(summary.get("country_rows_with_politics_residual_payloads")) in (
            None,
            0,
        ):
            errors.append("country politics residual rows were not observed")
        if _as_int(summary.get("country_rows_with_appointed_advisors")) in (None, 0):
            errors.append("appointed_advisors fields were not observed")
        if _as_int(summary.get("country_rows_with_appointed_advisor_details")) in (
            None,
            0,
        ):
            errors.append("appointed_advisor_details fields were not observed")
        if _as_int(summary.get("country_rows_with_appointed_advisor_slots")) in (
            None,
            0,
        ):
            errors.append("appointed_advisor_slots fields were not observed")
        if _as_int(summary.get("country_rows_with_appointed_advisor_ledgers")) in (
            None,
            0,
        ):
            errors.append("appointed_advisor_ledgers fields were not observed")
        if _as_int(summary.get("politics_residuals_on_lightweight_frames")) not in (
            0,
            None,
        ):
            errors.append("politics residual payloads appeared on lightweight frames")
    if require_projects_live_payload:
        scheduled = _scheduled_domain_frame_count(summary, "projects_live")
        required_frames = int(min_game_hours or 1)
        bridge_rows = _as_int(manifest.get("special_projects_bridge_rows"))
        payload_frames = _as_int(
            summary.get("frames_with_special_projects_payloads")
        )
        facility_rows = _as_int(summary.get("special_project_facility_rows"))
        project_rows = _as_int(summary.get("special_project_rows"))
        max_facilities = _as_int(
            summary.get("max_special_project_facilities_per_frame")
        )
        max_projects = _as_int(summary.get("max_special_project_rows_per_frame"))
        lightweight_projects = _as_int(
            summary.get("projects_on_lightweight_frames")
        )
        if scheduled is None or scheduled < required_frames:
            errors.append(
                "projects_live proof scheduled frames "
                f"{scheduled!r} below required {required_frames}"
            )
        if manifest.get("special_projects_bridge_supplied") is not True:
            errors.append("special_projects_bridge was not supplied")
        if bridge_rows in (None, 0):
            errors.append("special_projects_bridge_rows not positive")
        if payload_frames is None or payload_frames < required_frames:
            errors.append("special_projects payload frames below required cadence")
        if facility_rows in (None, 0):
            errors.append("special project facility rows were not observed")
        if max_facilities in (None, 0):
            errors.append("special project facilities were empty in every frame")
        if project_rows in (None, 0):
            errors.append("special project rows were not observed")
        if max_projects in (None, 0):
            errors.append("special project rows were empty in every frame")
        if lightweight_projects not in (0, None):
            errors.append("special_projects payloads appeared on lightweight frames")
    if require_projects_live_links:
        active_facilities = _as_int(
            summary.get("special_project_active_facility_rows")
        )
        active_project_links = _as_int(
            summary.get("special_project_facility_active_project_links")
        )
        facility_scientist_links = _as_int(
            summary.get("special_project_facility_scientist_links")
        )
        iteration_rows = _as_int(summary.get("special_project_iteration_rows"))
        worked_facility_links = _as_int(
            summary.get("special_project_worked_facility_links")
        )
        max_active_facilities = _as_int(
            summary.get("max_special_project_active_facilities_per_frame")
        )
        max_active_project_links = _as_int(
            summary.get("max_special_project_active_project_links_per_frame")
        )
        max_facility_scientist_links = _as_int(
            summary.get("max_special_project_facility_scientist_links_per_frame")
        )
        max_worked_facility_links = _as_int(
            summary.get("max_special_project_worked_facility_links_per_frame")
        )
        if active_facilities in (None, 0) or max_active_facilities in (None, 0):
            errors.append("active special-project facilities were not observed")
        if active_project_links in (None, 0) or max_active_project_links in (None, 0):
            errors.append("special-project active facility/project links missing")
        if facility_scientist_links in (None, 0) or max_facility_scientist_links in (
            None,
            0,
        ):
            errors.append("special-project facility/scientist links missing")
        if iteration_rows in (None, 0):
            errors.append("special-project iteration rows were not observed")
        if worked_facility_links in (None, 0) or max_worked_facility_links in (
            None,
            0,
        ):
            errors.append("special-project worked-facility links missing")
    if require_production_construction:
        scheduled = _scheduled_domain_frame_count(summary, "production_live")
        required_frames = int(min_game_hours or 1)
        if scheduled is None or scheduled < required_frames:
            errors.append(
                "production construction proof scheduled frames "
                f"{scheduled!r} below required {required_frames}"
            )
        if _as_int(summary.get("construction_queue_frames")) is None or _as_int(
            summary.get("construction_queue_frames")
        ) < required_frames:
            errors.append("construction queue frames below required cadence")
        if _as_int(summary.get("country_rows_with_construction_queue")) in (None, 0):
            errors.append("country construction queue rows were not observed")
        if _as_int(summary.get("construction_queue_rows")) in (None, 0):
            errors.append("construction queue line rows were not observed")
        if _as_int(summary.get("max_construction_queue_rows_per_frame")) in (None, 0):
            errors.append("construction queue rows were empty in every frame")
        if _as_int(summary.get("production_on_lightweight_frames")) not in (0, None):
            errors.append("production payloads appeared on lightweight frames")
    if require_state_resources:
        scheduled = _scheduled_domain_frame_count(summary, "state_detail")
        required_frames = int(min_game_hours or 1)
        if scheduled is None or scheduled < required_frames:
            errors.append(
                "state resources proof scheduled frames "
                f"{scheduled!r} below required {required_frames}"
            )
        if _as_int(summary.get("state_resource_frames")) is None or _as_int(
            summary.get("state_resource_frames")
        ) < required_frames:
            errors.append("state resource frames below required cadence")
        max_state_rows = _as_int(summary.get("max_state_rows"))
        if max_state_rows is None or max_state_rows < 1000:
            errors.append("max state rows missing or below full-state coverage")
        resource_rows = _as_int(summary.get("state_rows_with_resources"))
        scheduled_frames = scheduled if scheduled is not None else required_frames
        min_resource_rows = (
            scheduled_frames * max_state_rows
            if max_state_rows is not None
            else required_frames * 1000
        )
        if resource_rows is None or resource_rows < min_resource_rows:
            errors.append(
                "state resource rows "
                f"{resource_rows!r} below required {min_resource_rows}"
            )
        if _as_int(summary.get("state_rows_with_nonempty_resources")) in (None, 0):
            errors.append("nonempty state resource rows were not observed")
        if _as_int(summary.get("state_resource_entries")) in (None, 0):
            errors.append("state resource entries were not observed")
        max_resource_rows = _as_int(summary.get("max_state_resource_rows_per_frame"))
        if max_resource_rows is None or max_resource_rows < 1000:
            errors.append("state resource rows per frame below full-state coverage")
        elif max_state_rows is not None and max_resource_rows < max_state_rows:
            errors.append(
                "state resource rows per frame "
                f"{max_resource_rows!r} below max state rows {max_state_rows}"
            )
    if require_tech_live_names:
        scheduled = _scheduled_domain_frame_count(summary, "tech_live")
        required_frames = int(min_game_hours or 1)
        if scheduled is None or scheduled < required_frames:
            errors.append(
                "tech live names proof scheduled frames "
                f"{scheduled!r} below required {required_frames}"
            )
        if _as_int(summary.get("frames_with_research_payloads")) in (None, 0):
            errors.append("research payload frames were not observed")
        if _as_int(summary.get("country_rows_with_research_payloads")) in (None, 0):
            errors.append("country research rows were not observed")
        if _as_int(summary.get("max_research_active_rows_per_country")) in (None, 0):
            errors.append("active research rows were not observed")
        if _as_int(summary.get("research_named_slot_frames")) is None or _as_int(
            summary.get("research_named_slot_frames")
        ) < required_frames:
            errors.append("research named-slot frames below required cadence")
        if _as_int(summary.get("research_named_slot_rows")) in (None, 0):
            errors.append("research named-slot rows were not observed")
        if _as_int(summary.get("country_rows_with_research_named_slots")) in (None, 0):
            errors.append("country research named-slot rows were not observed")
        if _as_int(summary.get("max_research_named_slot_rows_per_frame")) in (None, 0):
            errors.append("research named-slot rows were empty in every frame")
        if _as_int(summary.get("tech_on_lightweight_frames")) not in (0, None):
            errors.append("research payloads appeared on lightweight frames")
    if require_tech_live_progress:
        scheduled = _scheduled_domain_frame_count(summary, "tech_live")
        required_frames = int(min_game_hours or 1)
        if scheduled is None or scheduled < required_frames:
            errors.append(
                "tech live progress proof scheduled frames "
                f"{scheduled!r} below required {required_frames}"
            )
        if _as_int(summary.get("frames_with_research_payloads")) in (None, 0):
            errors.append("research payload frames were not observed")
        if _as_int(summary.get("country_rows_with_research_payloads")) in (None, 0):
            errors.append("country research rows were not observed")
        if _as_int(summary.get("max_research_active_rows_per_country")) in (None, 0):
            errors.append("active research rows were not observed")
        if _as_int(summary.get("research_live_progress_slot_frames")) is None or _as_int(
            summary.get("research_live_progress_slot_frames")
        ) < required_frames:
            errors.append("research live-progress frames below required cadence")
        if _as_int(summary.get("research_live_progress_slot_rows")) in (None, 0):
            errors.append("research live-progress rows were not observed")
        if _as_int(summary.get("country_rows_with_research_live_progress_slots")) in (
            None,
            0,
        ):
            errors.append("country research live-progress rows were not observed")
        if _as_int(summary.get("max_research_live_progress_rows_per_frame")) in (
            None,
            0,
        ):
            errors.append("research live-progress rows were empty in every frame")
        research_progress = _research_live_progress_summary(manifest)
        if _as_int(research_progress.get("speed_frames")) in (None, 0):
            errors.append("research live progress did not move between frames")
        if _as_int(research_progress.get("speed_rows")) in (None, 0):
            errors.append("research live progress moving rows were not observed")
        if _as_int(summary.get("tech_on_lightweight_frames")) not in (0, None):
            errors.append("research payloads appeared on lightweight frames")
    if require_tech_live_static_cost_progress:
        scheduled = _scheduled_domain_frame_count(summary, "tech_live")
        required_frames = int(min_game_hours or 1)
        if scheduled is None or scheduled < required_frames:
            errors.append(
                "tech live static-cost progress proof scheduled frames "
                f"{scheduled!r} below required {required_frames}"
            )
        if _as_int(summary.get("frames_with_research_payloads")) in (None, 0):
            errors.append("research payload frames were not observed")
        if _as_int(summary.get("country_rows_with_research_payloads")) in (None, 0):
            errors.append("country research rows were not observed")
        if _as_int(summary.get("max_research_active_rows_per_country")) in (None, 0):
            errors.append("active research rows were not observed")
        if _as_int(summary.get("tech_on_lightweight_frames")) not in (0, None):
            errors.append("research payloads appeared on lightweight frames")
        research_progress = _research_live_progress_summary(manifest)
        static_cost_progress_frames = _as_int(
            research_progress.get("static_cost_progress_frames")
        )
        if (
            static_cost_progress_frames is None
            or static_cost_progress_frames < required_frames
        ):
            errors.append("research static-cost progress frames below required cadence")
        if _as_int(research_progress.get("static_cost_progress_rows")) in (None, 0):
            errors.append("research static-cost progress rows were not observed")
        if _as_int(
            research_progress.get("max_static_cost_progress_rows_per_frame")
        ) in (None, 0):
            errors.append("research static-cost progress rows were empty in every frame")
    if require_tech_live_dll_base_progress:
        scheduled = _scheduled_domain_frame_count(summary, "tech_live")
        required_frames = int(min_game_hours or 1)
        if scheduled is None or scheduled < required_frames:
            errors.append(
                "tech live DLL-base progress proof scheduled frames "
                f"{scheduled!r} below required {required_frames}"
            )
        if _as_int(summary.get("frames_with_research_payloads")) in (None, 0):
            errors.append("research payload frames were not observed")
        if _as_int(summary.get("country_rows_with_research_payloads")) in (None, 0):
            errors.append("country research rows were not observed")
        if _as_int(summary.get("max_research_active_rows_per_country")) in (None, 0):
            errors.append("active research rows were not observed")
        if _as_int(summary.get("tech_on_lightweight_frames")) not in (0, None):
            errors.append("research payloads appeared on lightweight frames")
        research_progress = _research_live_progress_summary(manifest)
        dll_base_progress_frames = _as_int(
            research_progress.get("dll_base_required_progress_frames")
        )
        if (
            dll_base_progress_frames is None
            or dll_base_progress_frames < required_frames
        ):
            errors.append("research DLL-base progress frames below required cadence")
        if _as_int(research_progress.get("dll_base_required_progress_rows")) in (
            None,
            0,
        ):
            errors.append("research DLL-base progress rows were not observed")
        if _as_int(
            research_progress.get("max_dll_base_required_progress_rows_per_frame")
        ) in (None, 0):
            errors.append("research DLL-base progress rows were empty in every frame")
    if require_tech_live_eta:
        scheduled = _scheduled_domain_frame_count(summary, "tech_live")
        required_frames = int(min_game_hours or 1)
        if scheduled is None or scheduled < required_frames:
            errors.append(
                "tech live ETA proof scheduled frames "
                f"{scheduled!r} below required {required_frames}"
            )
        if _as_int(summary.get("frames_with_research_payloads")) in (None, 0):
            errors.append("research payload frames were not observed")
        if _as_int(summary.get("country_rows_with_research_payloads")) in (None, 0):
            errors.append("country research rows were not observed")
        if _as_int(summary.get("max_research_active_rows_per_country")) in (None, 0):
            errors.append("active research rows were not observed")
        if _as_int(summary.get("tech_on_lightweight_frames")) not in (0, None):
            errors.append("research payloads appeared on lightweight frames")
        research_progress = _research_live_progress_summary(manifest)
        if _as_int(research_progress.get("speed_frames")) in (None, 0):
            errors.append("research live speed frames were not observed")
        if _as_int(research_progress.get("speed_rows")) in (None, 0):
            errors.append("research live speed rows were not observed")
        if _as_int(research_progress.get("eta_frames")) in (None, 0):
            errors.append("research live ETA frames were not observed")
        if _as_int(research_progress.get("eta_rows")) in (None, 0):
            errors.append("research live ETA rows were not observed")
    if require_division_raw_telemetry:
        required_frames = int(min_game_hours or 1)
        if manifest.get("division_body_bridge_supplied") is not True:
            errors.append("division body bridge was not supplied")
        if _as_int(manifest.get("division_body_bridge_rows")) in (None, 0):
            errors.append("division body bridge rows not positive")
        division_delta_frames = _map_int(
            summary,
            "raw_delta_domain_counts",
            "divisions",
        )
        if division_delta_frames < required_frames:
            errors.append(
                "division raw telemetry frames "
                f"{division_delta_frames} below required {required_frames}"
            )
        if _map_int(summary, "raw_delta_domain_object_counts", "divisions") == 0:
            errors.append("division raw telemetry object rows were not observed")
        division_event_rows = _event_rows(summary, DIVISION_RAW_EVENT_TYPES)
        if division_event_rows == 0:
            errors.append("division raw telemetry change events were not observed")
        if _map_int(summary, "raw_gap_domain_counts", "divisions") > 0:
            errors.append("division raw telemetry gap frames observed")
    if require_land_combat_raw_lifecycle:
        required_frames = int(min_game_hours or 1)
        land_delta_frames = _map_int(
            summary,
            "raw_delta_domain_counts",
            "land_combats",
        )
        if land_delta_frames < required_frames:
            errors.append(
                "land combat raw telemetry frames "
                f"{land_delta_frames} below required {required_frames}"
            )
        if _map_int(summary, "raw_delta_domain_object_counts", "land_combats") == 0:
            errors.append("land combat raw telemetry object rows were not observed")
        raw_event_counts = _int_map(summary.get("raw_event_type_counts"))
        for event_type in LAND_COMBAT_RAW_EVENT_TYPES:
            if raw_event_counts.get(event_type, 0) <= 0:
                errors.append(f"land combat raw event {event_type!r} not observed")
        if _map_int(summary, "raw_gap_domain_counts", "land_combats") > 0:
            errors.append("land combat raw telemetry gap frames observed")
    if require_army_order_raw_telemetry:
        if manifest.get("army_body_bridge_supplied") is not True:
            errors.append("army body bridge was not supplied")
        if _as_int(manifest.get("army_body_bridge_rows")) in (None, 0):
            errors.append("army body bridge rows not positive")
        if _map_int(summary, "raw_delta_domain_counts", "army_orders") == 0:
            errors.append("army order raw telemetry frames were not observed")
        if _map_int(summary, "raw_delta_domain_object_counts", "army_orders") == 0:
            errors.append("army order raw telemetry object rows were not observed")
        raw_event_counts = _int_map(summary.get("raw_event_type_counts"))
        if raw_event_counts.get("army_order_added", 0) <= 0:
            errors.append("army order raw add events were not observed")
        if _map_int(summary, "raw_gap_domain_counts", "army_orders") > 0:
            errors.append("army order raw telemetry gap frames observed")
    if require_army_order_raw_removals:
        if manifest.get("army_body_bridge_supplied") is not True:
            errors.append("army body bridge was not supplied")
        if _as_int(manifest.get("army_body_bridge_rows")) in (None, 0):
            errors.append("army body bridge rows not positive")
        if _map_int(summary, "raw_delta_domain_counts", "army_orders") == 0:
            errors.append("army order raw telemetry frames were not observed")
        raw_event_counts = _int_map(summary.get("raw_event_type_counts"))
        if raw_event_counts.get("army_order_removed", 0) <= 0:
            errors.append("army order raw removal events were not observed")
        if _map_int(summary, "raw_gap_domain_counts", "army_orders") > 0:
            errors.append("army order raw telemetry gap frames observed")
    if require_army_order_raw_updates:
        if manifest.get("army_body_bridge_supplied") is not True:
            errors.append("army body bridge was not supplied")
        if _as_int(manifest.get("army_body_bridge_rows")) in (None, 0):
            errors.append("army body bridge rows not positive")
        if _map_int(summary, "raw_delta_domain_counts", "army_orders") == 0:
            errors.append("army order raw telemetry frames were not observed")
        if _map_int(summary, "raw_delta_domain_object_counts", "army_orders") == 0:
            errors.append("army order raw telemetry object rows were not observed")
        raw_event_counts = _int_map(summary.get("raw_event_type_counts"))
        if raw_event_counts.get("army_order_updated", 0) <= 0:
            errors.append("army order raw update events were not observed")
        if _map_int(summary, "raw_gap_domain_counts", "army_orders") > 0:
            errors.append("army order raw telemetry gap frames observed")
    if require_air_wing_raw_telemetry:
        if manifest.get("air_wing_body_bridge_supplied") is True:
            errors.append("air wing body bridge was supplied; expected DLL-native root")
        bridge_rows = _as_int(manifest.get("air_wing_body_bridge_rows"))
        if bridge_rows not in (None, 0):
            errors.append("air wing body bridge rows present; expected DLL-native root")
        if _map_int(summary, "raw_delta_domain_counts", "air_wings") == 0:
            errors.append("air wing raw telemetry frames were not observed")
        if _map_int(summary, "raw_delta_domain_object_counts", "air_wings") == 0:
            errors.append("air wing raw telemetry object rows were not observed")
        raw_event_counts = _int_map(summary.get("raw_event_type_counts"))
        if raw_event_counts.get("air_wing_observed", 0) <= 0:
            errors.append("air wing raw observed events were not observed")
        if _map_int(summary, "raw_gap_domain_counts", "air_wings") > 0:
            errors.append("air wing raw telemetry gap frames observed")
    if require_ship_raw_telemetry:
        if manifest.get("ship_body_bridge_supplied") is not True:
            errors.append("ship body bridge was not supplied")
        if _as_int(manifest.get("ship_body_bridge_rows")) in (None, 0):
            errors.append("ship body bridge rows not positive")
        if _map_int(summary, "raw_delta_domain_counts", "ships") == 0:
            errors.append("ship raw telemetry frames were not observed")
        if _map_int(summary, "raw_delta_domain_object_counts", "ships") == 0:
            errors.append("ship raw telemetry object rows were not observed")
        if _map_int(summary, "raw_gap_domain_counts", "ships") > 0:
            errors.append("ship raw telemetry gap frames observed")
    if require_oob_command_raw_telemetry or require_oob_command_leader_detail:
        if manifest.get("oob_command_bridge_supplied") is not True:
            errors.append("OOB command bridge was not supplied")
        if _as_int(manifest.get("oob_command_bridge_rows")) in (None, 0):
            errors.append("OOB command bridge rows not positive")
        if _map_int(summary, "raw_delta_domain_counts", "oob_command") == 0:
            errors.append("OOB command raw telemetry frames were not observed")
        if _map_int(summary, "raw_delta_domain_object_counts", "oob_command") == 0:
            errors.append("OOB command raw telemetry object rows were not observed")
        if _map_int(summary, "raw_gap_domain_counts", "oob_command") > 0:
            errors.append("OOB command raw telemetry gap frames observed")
    if require_oob_command_leader_detail:
        if _as_int(summary.get("oob_command_exact_army_leader_detail_rows")) in (
            None,
            0,
        ):
            errors.append(
                "OOB command exact army leader detail rows were not observed"
            )
        if _as_int(summary.get("oob_command_exact_army_leader_trait_tokens")) in (
            None,
            0,
        ):
            errors.append(
                "OOB command exact army leader trait tokens were not observed"
            )
    return errors


def collector_session_proof_summary(
    manifest: Dict[str, Any],
    *,
    required_snapshot_domains: Iterable[str] = (),
    required_scheduled_snapshot_domains: Iterable[str] = (),
    required_raw_delta_domains: Iterable[str] = (),
    required_raw_event_types: Iterable[str] = (),
    min_game_hours: Optional[int] = None,
    require_replay_verified: bool = True,
    require_hourly: bool = True,
    require_no_raw_gaps: bool = True,
    require_diplomacy_war_relations: bool = False,
    require_politics_leaders: bool = False,
    require_politics_idea_classification: bool = False,
    require_politics_advisor_residuals: bool = False,
    require_projects_live_payload: bool = False,
    require_projects_live_links: bool = False,
    require_production_construction: bool = False,
    require_state_resources: bool = False,
    require_tech_live_names: bool = False,
    require_tech_live_progress: bool = False,
    require_tech_live_static_cost_progress: bool = False,
    require_tech_live_dll_base_progress: bool = False,
    require_tech_live_eta: bool = False,
    require_division_raw_telemetry: bool = False,
    require_land_combat_raw_lifecycle: bool = False,
    require_army_order_raw_telemetry: bool = False,
    require_army_order_raw_removals: bool = False,
    require_army_order_raw_updates: bool = False,
    require_air_wing_raw_telemetry: bool = False,
    require_ship_raw_telemetry: bool = False,
    require_oob_command_raw_telemetry: bool = False,
    require_oob_command_leader_detail: bool = False,
) -> Dict[str, Any]:
    summary = _payload_summary(manifest)
    errors = collector_session_proof_errors(
        manifest,
        required_snapshot_domains=required_snapshot_domains,
        required_scheduled_snapshot_domains=required_scheduled_snapshot_domains,
        required_raw_delta_domains=required_raw_delta_domains,
        required_raw_event_types=required_raw_event_types,
        min_game_hours=min_game_hours,
        require_replay_verified=require_replay_verified,
        require_hourly=require_hourly,
        require_no_raw_gaps=require_no_raw_gaps,
        require_diplomacy_war_relations=require_diplomacy_war_relations,
        require_politics_leaders=require_politics_leaders,
        require_politics_idea_classification=require_politics_idea_classification,
        require_politics_advisor_residuals=require_politics_advisor_residuals,
        require_projects_live_payload=require_projects_live_payload,
        require_projects_live_links=require_projects_live_links,
        require_production_construction=require_production_construction,
        require_state_resources=require_state_resources,
        require_tech_live_names=require_tech_live_names,
        require_tech_live_progress=require_tech_live_progress,
        require_tech_live_static_cost_progress=require_tech_live_static_cost_progress,
        require_tech_live_dll_base_progress=require_tech_live_dll_base_progress,
        require_tech_live_eta=require_tech_live_eta,
        require_division_raw_telemetry=require_division_raw_telemetry,
        require_land_combat_raw_lifecycle=require_land_combat_raw_lifecycle,
        require_army_order_raw_telemetry=require_army_order_raw_telemetry,
        require_army_order_raw_removals=require_army_order_raw_removals,
        require_army_order_raw_updates=require_army_order_raw_updates,
        require_air_wing_raw_telemetry=require_air_wing_raw_telemetry,
        require_ship_raw_telemetry=require_ship_raw_telemetry,
        require_oob_command_raw_telemetry=require_oob_command_raw_telemetry,
        require_oob_command_leader_detail=require_oob_command_leader_detail,
    )
    hourly = manifest.get("hourly_requirement") or {}
    return {
        "ok": not errors,
        "errors": errors,
        "game_hours_advanced": manifest.get("game_hours_advanced"),
        "stored_snapshot_count": manifest.get("stored_snapshot_count"),
        "replay_verified": manifest.get("replay_verified"),
        "hourly_requirement_met": (
            hourly.get("met") if isinstance(hourly, dict) else None
        ),
        "hourly_elapsed_wall_s": (
            hourly.get("elapsed_wall_s") if isinstance(hourly, dict) else None
        ),
        "domains_seen": sorted(_string_set(summary.get("domains_seen"))),
        "scheduled_snapshot_domain_frames": _scheduled_domain_counts(summary),
        "raw_delta_domains_seen": sorted(
            _string_set(summary.get("raw_delta_domains_seen"))
        ),
        "raw_event_types_seen": sorted(
            _string_set(summary.get("raw_event_types_seen"))
        ),
        "raw_event_type_counts": _int_map(summary.get("raw_event_type_counts")),
        "raw_event_domain_counts": _int_map(
            summary.get("raw_event_domain_counts")),
        "raw_event_type_domain_counts": _int_map(
            summary.get("raw_event_type_domain_counts")),
        "raw_delta_domain_counts": _int_map(
            summary.get("raw_delta_domain_counts")),
        "raw_delta_domain_object_counts": _int_map(
            summary.get("raw_delta_domain_object_counts")),
        "raw_delta_domain_null_counts": _int_map(
            summary.get("raw_delta_domain_null_counts")),
        "raw_delta_domain_worker_poll_counts": _int_map(
            summary.get("raw_delta_domain_worker_poll_counts")),
        "raw_delta_domain_scheduled_counts": _int_map(
            summary.get("raw_delta_domain_scheduled_counts")),
        "raw_gap_domain_counts": _int_map(summary.get("raw_gap_domain_counts")),
        "heavy_payload_on_lightweight_frames": summary.get(
            "heavy_payload_on_lightweight_frames"
        ),
        "diplomacy_war_relation_frames": summary.get(
            "diplomacy_war_relation_frames"),
        "max_diplomacy_war_relations_per_frame": summary.get(
            "max_diplomacy_war_relations_per_frame"),
        "max_diplomacy_casualty_sum_per_frame": summary.get(
            "max_diplomacy_casualty_sum_per_frame"),
        "country_rows_with_diplomacy_at_war": summary.get(
            "country_rows_with_diplomacy_at_war"),
        "country_rows_with_diplomacy_casualties": summary.get(
            "country_rows_with_diplomacy_casualties"),
        "politics_leader_frames": summary.get("politics_leader_frames"),
        "country_rows_with_politics_leader": summary.get(
            "country_rows_with_politics_leader"),
        "max_politics_leader_rows_per_frame": summary.get(
            "max_politics_leader_rows_per_frame"),
        "country_rows_with_politics_ideas": summary.get(
            "country_rows_with_politics_ideas"),
        "max_politics_idea_rows_per_country": summary.get(
            "max_politics_idea_rows_per_country"),
        "country_rows_with_politics_laws": summary.get(
            "country_rows_with_politics_laws"),
        "max_politics_law_keys_per_country": summary.get(
            "max_politics_law_keys_per_country"),
        "country_rows_with_politics_idea_groups": summary.get(
            "country_rows_with_politics_idea_groups"),
        "max_politics_idea_group_keys_per_country": summary.get(
            "max_politics_idea_group_keys_per_country"),
        "country_rows_with_politics_advisor_ideas": summary.get(
            "country_rows_with_politics_advisor_ideas"),
        "max_politics_advisor_idea_keys_per_country": summary.get(
            "max_politics_advisor_idea_keys_per_country"),
        "max_politics_advisor_idea_tokens_per_country": summary.get(
            "max_politics_advisor_idea_tokens_per_country"),
        "country_rows_with_politics_ideas_other": summary.get(
            "country_rows_with_politics_ideas_other"),
        "max_politics_ideas_other_tokens_per_country": summary.get(
            "max_politics_ideas_other_tokens_per_country"),
        "country_rows_with_politics_national_spirits": summary.get(
            "country_rows_with_politics_national_spirits"),
        "max_politics_national_spirit_tokens_per_country": summary.get(
            "max_politics_national_spirit_tokens_per_country"),
        "frames_with_politics_residual_payloads": summary.get(
            "frames_with_politics_residual_payloads"),
        "country_rows_with_politics_residual_payloads": summary.get(
            "country_rows_with_politics_residual_payloads"),
        "max_politics_residual_country_rows_per_frame": summary.get(
            "max_politics_residual_country_rows_per_frame"),
        "country_rows_with_appointed_advisors": summary.get(
            "country_rows_with_appointed_advisors"),
        "country_rows_with_appointed_advisor_details": summary.get(
            "country_rows_with_appointed_advisor_details"),
        "country_rows_with_appointed_advisor_slots": summary.get(
            "country_rows_with_appointed_advisor_slots"),
        "country_rows_with_appointed_advisor_ledgers": summary.get(
            "country_rows_with_appointed_advisor_ledgers"),
        "oob_command_leader_skill_rows": summary.get(
            "oob_command_leader_skill_rows"),
        "oob_command_leader_traits_rows": summary.get(
            "oob_command_leader_traits_rows"),
        "oob_command_leader_trait_tokens": summary.get(
            "oob_command_leader_trait_tokens"),
        "oob_command_leader_trait_exact_rows": summary.get(
            "oob_command_leader_trait_exact_rows"),
        "max_oob_command_leader_trait_tokens_per_row": summary.get(
            "max_oob_command_leader_trait_tokens_per_row"),
        "oob_command_exact_army_leader_detail_rows": summary.get(
            "oob_command_exact_army_leader_detail_rows"),
        "oob_command_exact_army_leader_trait_tokens": summary.get(
            "oob_command_exact_army_leader_trait_tokens"),
        "max_oob_command_exact_army_leader_trait_tokens_per_row": summary.get(
            "max_oob_command_exact_army_leader_trait_tokens_per_row"),
        "appointed_advisor_rows": summary.get("appointed_advisor_rows"),
        "appointed_advisor_detail_rows": summary.get(
            "appointed_advisor_detail_rows"),
        "appointed_advisor_slot_rows": summary.get("appointed_advisor_slot_rows"),
        "appointed_advisor_ledger_rows": summary.get(
            "appointed_advisor_ledger_rows"),
        "max_appointed_advisors_per_country": summary.get(
            "max_appointed_advisors_per_country"),
        "max_appointed_advisor_details_per_country": summary.get(
            "max_appointed_advisor_details_per_country"),
        "max_appointed_advisor_slots_per_country": summary.get(
            "max_appointed_advisor_slots_per_country"),
        "max_appointed_advisor_ledgers_per_country": summary.get(
            "max_appointed_advisor_ledgers_per_country"),
        "frames_with_special_projects_payloads": summary.get(
            "frames_with_special_projects_payloads"),
        "special_project_facility_rows": summary.get(
            "special_project_facility_rows"),
        "special_project_rows": summary.get("special_project_rows"),
        "special_project_scientist_rows": summary.get(
            "special_project_scientist_rows"),
        "special_project_active_facility_rows": summary.get(
            "special_project_active_facility_rows"),
        "special_project_facility_active_project_links": summary.get(
            "special_project_facility_active_project_links"),
        "special_project_facility_scientist_links": summary.get(
            "special_project_facility_scientist_links"),
        "special_project_iteration_rows": summary.get(
            "special_project_iteration_rows"),
        "special_project_worked_facility_links": summary.get(
            "special_project_worked_facility_links"),
        "max_special_project_facilities_per_frame": summary.get(
            "max_special_project_facilities_per_frame"),
        "max_special_project_rows_per_frame": summary.get(
            "max_special_project_rows_per_frame"),
        "max_special_project_scientists_per_frame": summary.get(
            "max_special_project_scientists_per_frame"),
        "max_special_project_active_facilities_per_frame": summary.get(
            "max_special_project_active_facilities_per_frame"),
        "max_special_project_active_project_links_per_frame": summary.get(
            "max_special_project_active_project_links_per_frame"),
        "max_special_project_facility_scientist_links_per_frame": summary.get(
            "max_special_project_facility_scientist_links_per_frame"),
        "max_special_project_worked_facility_links_per_frame": summary.get(
            "max_special_project_worked_facility_links_per_frame"),
        "projects_on_lightweight_frames": summary.get(
            "projects_on_lightweight_frames"),
        "construction_queue_frames": summary.get("construction_queue_frames"),
        "country_rows_with_construction_queue": summary.get(
            "country_rows_with_construction_queue"),
        "construction_queue_rows": summary.get("construction_queue_rows"),
        "max_construction_queue_rows_per_frame": summary.get(
            "max_construction_queue_rows_per_frame"),
        "max_construction_queue_rows_per_country": summary.get(
            "max_construction_queue_rows_per_country"),
        "state_detail_frames": summary.get("state_detail_frames"),
        "scheduled_state_detail_domain_frames": summary.get(
            "scheduled_state_detail_domain_frames"),
        "state_resource_frames": summary.get("state_resource_frames"),
        "max_state_rows": summary.get("max_state_rows"),
        "state_rows_with_resources": summary.get("state_rows_with_resources"),
        "state_rows_with_nonempty_resources": summary.get(
            "state_rows_with_nonempty_resources"),
        "state_resource_entries": summary.get("state_resource_entries"),
        "max_state_resource_rows_per_frame": summary.get(
            "max_state_resource_rows_per_frame"),
        "max_state_resource_entries_per_state": summary.get(
            "max_state_resource_entries_per_state"),
        "research_named_slot_frames": summary.get("research_named_slot_frames"),
        "research_named_slot_rows": summary.get("research_named_slot_rows"),
        "country_rows_with_research_named_slots": summary.get(
            "country_rows_with_research_named_slots"),
        "max_research_named_slot_rows_per_frame": summary.get(
            "max_research_named_slot_rows_per_frame"),
        "max_research_named_slot_rows_per_country": summary.get(
            "max_research_named_slot_rows_per_country"),
        "research_live_progress_slot_frames": summary.get(
            "research_live_progress_slot_frames"),
        "research_live_progress_slot_rows": summary.get(
            "research_live_progress_slot_rows"),
        "country_rows_with_research_live_progress_slots": summary.get(
            "country_rows_with_research_live_progress_slots"),
        "max_research_live_progress_rows_per_frame": summary.get(
            "max_research_live_progress_rows_per_frame"),
        "max_research_live_progress_rows_per_country": summary.get(
            "max_research_live_progress_rows_per_country"),
        "research_live_progress": _research_live_progress_summary(manifest),
    }


def _payload_summary(manifest: Dict[str, Any]) -> Dict[str, Any]:
    value = manifest.get("collector_frame_payload_summary")
    return value if isinstance(value, dict) else {}


def _research_live_progress_summary(manifest: Dict[str, Any]) -> Dict[str, Any]:
    diagnostics = manifest.get("recording_diagnostics")
    if not isinstance(diagnostics, dict):
        return {}
    collector = diagnostics.get("collector")
    if not isinstance(collector, dict):
        return {}
    progress = collector.get("research_live_progress")
    return progress if isinstance(progress, dict) else {}


def _string_set(value: Any) -> set[str]:
    if not isinstance(value, list):
        return set()
    return {str(item) for item in value if isinstance(item, str)}


def _scheduled_domain_counts(summary: Dict[str, Any]) -> Dict[str, int]:
    out: Dict[str, int] = {}
    for domain in sorted(SCHEDULED_DOMAIN_FRAME_COUNT_KEYS):
        count = _scheduled_domain_frame_count(summary, domain)
        if count is not None:
            out[domain] = count
    return out


def _scheduled_domain_frame_count(
    summary: Dict[str, Any],
    domain: str,
) -> Optional[int]:
    key = SCHEDULED_DOMAIN_FRAME_COUNT_KEYS.get(domain)
    if key is None:
        return None
    return _as_int(summary.get(key))


def _as_int(value: Any) -> Optional[int]:
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    return None


def _int_map(value: Any) -> Dict[str, int]:
    if not isinstance(value, dict):
        return {}
    out: Dict[str, int] = {}
    for key, raw_count in value.items():
        count = _as_int(raw_count)
        if isinstance(key, str) and count is not None:
            out[key] = count
    return dict(sorted(out.items()))


def _map_int(summary: Dict[str, Any], map_key: str, item_key: str) -> int:
    return _int_map(summary.get(map_key)).get(item_key, 0)


def _event_rows(summary: Dict[str, Any], event_types: Iterable[str]) -> int:
    counts = _int_map(summary.get("raw_event_type_counts"))
    return sum(counts.get(event_type, 0) for event_type in event_types)


def _main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        description="Validate a live collector session manifest proof gate."
    )
    parser.add_argument("manifest", type=Path)
    parser.add_argument(
        "--require-domain",
        action="append",
        default=[],
        help="snapshot domain that must appear in collector_frame_payload_summary",
    )
    parser.add_argument(
        "--require-scheduled-domain",
        action="append",
        default=[],
        help="snapshot domain that must appear on a scheduled_hour_snapshot frame",
    )
    parser.add_argument(
        "--require-raw-domain",
        action="append",
        default=[],
        help="raw domain_delta domain that must appear in collector_frame_payload_summary",
    )
    parser.add_argument(
        "--require-raw-event-type",
        action="append",
        default=[],
        help="raw event type that must appear in collector_frame_payload_summary",
    )
    parser.add_argument("--min-game-hours", type=int, default=None)
    parser.add_argument("--allow-unverified-replay", action="store_true")
    parser.add_argument("--allow-hourly-missing", action="store_true")
    parser.add_argument("--allow-raw-gaps", action="store_true")
    parser.add_argument(
        "--require-diplomacy-war-relations",
        action="store_true",
        help="require positive scheduled diplomacy war/casualty rows",
    )
    parser.add_argument(
        "--require-politics-leaders",
        action="store_true",
        help="require positive scheduled politics active-leader rows",
    )
    parser.add_argument(
        "--require-politics-idea-classification",
        action="store_true",
        help="require live politics idea law/advisor/spirit classification rows",
    )
    parser.add_argument(
        "--require-politics-advisor-residuals",
        action="store_true",
        help="require live politics appointed-advisor residual fields",
    )
    parser.add_argument(
        "--require-projects-live-payload",
        action="store_true",
        help="require scheduled projects_live facility/project payload rows",
    )
    parser.add_argument(
        "--require-projects-live-links",
        action="store_true",
        help="require scheduled projects_live facility/project/scientist links",
    )
    parser.add_argument(
        "--require-production-construction",
        action="store_true",
        help="require positive scheduled production construction-queue rows",
    )
    parser.add_argument(
        "--require-state-resources",
        action="store_true",
        help="require positive scheduled live state resource deposit rows",
    )
    parser.add_argument(
        "--require-tech-live-names",
        action="store_true",
        help="require positive scheduled live research rows with resolved tech_name",
    )
    parser.add_argument(
        "--require-tech-live-progress",
        action="store_true",
        help="require positive scheduled live research rows and moving live progress",
    )
    parser.add_argument(
        "--require-tech-live-static-cost-progress",
        action="store_true",
        help="require replay-written live research progress fractions from static tech cost",
    )
    parser.add_argument(
        "--require-tech-live-dll-base-progress",
        action="store_true",
        help="require replay-written live research progress fractions from DLL base denominator",
    )
    parser.add_argument(
        "--require-tech-live-eta",
        action="store_true",
        help="require replay-written live research speed and ETA rows",
    )
    parser.add_argument(
        "--require-division-raw-telemetry",
        action="store_true",
        help="require live DLL division raw rows and change events",
    )
    parser.add_argument(
        "--require-land-combat-raw-lifecycle",
        action="store_true",
        help="require live DLL land-combat started/updated/ended lifecycle rows",
    )
    parser.add_argument(
        "--require-army-order-raw-telemetry",
        action="store_true",
        help="require live DLL army-order raw rows and add events",
    )
    parser.add_argument(
        "--require-army-order-raw-removals",
        action="store_true",
        help="require live DLL army-order removal events",
    )
    parser.add_argument(
        "--require-army-order-raw-updates",
        action="store_true",
        help="require live DLL army-order update events, not only initial rows",
    )
    parser.add_argument(
        "--require-air-wing-raw-telemetry",
        action="store_true",
        help="require live DLL air-wing raw rows and observed events",
    )
    parser.add_argument(
        "--require-ship-raw-telemetry",
        action="store_true",
        help="require live DLL ship raw telemetry rows",
    )
    parser.add_argument(
        "--require-oob-command-raw-telemetry",
        action="store_true",
        help="require live DLL OOB command hierarchy rows",
    )
    parser.add_argument(
        "--require-oob-command-leader-detail",
        action="store_true",
        help="require live DLL OOB army leader skill and trait rows",
    )
    args = parser.parse_args(argv)

    manifest = load_session_manifest(args.manifest)
    proof = collector_session_proof_summary(
        manifest,
        required_snapshot_domains=args.require_domain,
        required_scheduled_snapshot_domains=args.require_scheduled_domain,
        required_raw_delta_domains=args.require_raw_domain,
        required_raw_event_types=args.require_raw_event_type,
        min_game_hours=args.min_game_hours,
        require_replay_verified=not args.allow_unverified_replay,
        require_hourly=not args.allow_hourly_missing,
        require_no_raw_gaps=not args.allow_raw_gaps,
        require_diplomacy_war_relations=args.require_diplomacy_war_relations,
        require_politics_leaders=args.require_politics_leaders,
        require_politics_idea_classification=(
            args.require_politics_idea_classification
        ),
        require_politics_advisor_residuals=args.require_politics_advisor_residuals,
        require_projects_live_payload=args.require_projects_live_payload,
        require_projects_live_links=args.require_projects_live_links,
        require_production_construction=args.require_production_construction,
        require_state_resources=args.require_state_resources,
        require_tech_live_names=args.require_tech_live_names,
        require_tech_live_progress=args.require_tech_live_progress,
        require_tech_live_static_cost_progress=(
            args.require_tech_live_static_cost_progress
        ),
        require_tech_live_dll_base_progress=args.require_tech_live_dll_base_progress,
        require_tech_live_eta=args.require_tech_live_eta,
        require_division_raw_telemetry=args.require_division_raw_telemetry,
        require_land_combat_raw_lifecycle=args.require_land_combat_raw_lifecycle,
        require_army_order_raw_telemetry=args.require_army_order_raw_telemetry,
        require_army_order_raw_removals=args.require_army_order_raw_removals,
        require_army_order_raw_updates=args.require_army_order_raw_updates,
        require_air_wing_raw_telemetry=args.require_air_wing_raw_telemetry,
        require_ship_raw_telemetry=args.require_ship_raw_telemetry,
        require_oob_command_raw_telemetry=args.require_oob_command_raw_telemetry,
        require_oob_command_leader_detail=args.require_oob_command_leader_detail,
    )
    print(json.dumps(proof, indent=2, sort_keys=True))
    return 0 if proof["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(_main())
