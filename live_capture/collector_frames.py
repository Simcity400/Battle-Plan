"""Adapter for in-process collector frames.

This module is the recorder-side contract for the new DLL path. It accepts
already-decoded collector frames, rejects malformed provenance/domains early,
and maps valid snapshot frames into the existing replay schema without reading
or requiring saves.
"""
from __future__ import annotations

import json
import math
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional

from replay import schema


COLLECTOR_BACKEND = "in_process_collector"
COLLECTOR_ABI_VERSION = 1
FRAME_TYPES = frozenset({
    "collector_started",
    "heartbeat",
    "snapshot",
    "session_start",
    "session_stop",
    "domain_delta",
    "event",
    "diagnostic",
    "gap",
})
SUPPORTED_SNAPSHOT_DOMAINS = frozenset({
    "clock",
    "country_identity",
    "country_misc",
    "country_scalars",
    "focus_live",
    "tech_live",
    "projects_live",
    "fuel_live",
    "decisions_live",
    "politics_live",
    "politics_residuals",
    "production_live",
    "diplomacy_live",
    "state_ownership",
    "state_control",
    "province_control",
    "buildings_live",
})
SCHEDULED_ONLY_SNAPSHOT_DOMAINS = frozenset({
    "focus_live",
    "tech_live",
    "projects_live",
    "fuel_live",
    "decisions_live",
    "politics_live",
    "politics_residuals",
    "production_live",
    "diplomacy_live",
    "state_ownership",
    "state_control",
    "province_control",
    "buildings_live",
})
MAP_CONTROL_SNAPSHOT_DOMAINS = frozenset({
    "state_ownership",
    "state_control",
    "province_control",
})
WORKER_POLL_FORBIDDEN_DOMAINS = SCHEDULED_ONLY_SNAPSHOT_DOMAINS
PRODUCTION_STOCKPILE_MAX_ENTRIES = 4096
PRODUCTION_VARIANT_ID_MAX = 10_000_000
PRODUCTION_LINE_MAX_ENTRIES = 4096
PRODUCTION_LINE_ID_MAX = 10_000_000
PRODUCTION_FACTORY_COUNT_MAX = 100_000
PRODUCTION_LINE_AMOUNT_MAX = 2_147_483_647
PRODUCTION_CONSTRUCTION_TEMPLATE_ID_MAX = 10_000_000
PRODUCTION_FIXED_POINT_ABS_MAX = 1_000_000_000_000.0
BUILDING_LEVEL_MAX = 1000
FUEL_MEASURE_ABS_MAX = 1_000_000_000.0
COUNTRY_MISC_ACE_COUNT_MAX = 100_000
COUNTRY_MISC_CONVOYS_DESTROYED_MAX = 100_000_000
COUNTRY_MISC_ACCIDENTS_SCORE_MAX = 10.0
COUNTRY_MISC_TOKEN_MAX_LENGTH = 96
COUNTRY_MISC_PAYLOAD_KEYS = frozenset((
    "convoys_destroyed",
    "accidents_score",
    "ace_count",
    "cosmetic_tag",
))
COUNTRY_POLITICS_IDEOLOGIES = frozenset((
    "democratic",
    "communism",
    "fascism",
    "neutrality",
))
COUNTRY_POLITICS_PAYLOAD_KEYS = frozenset((
    "popularities",
    "ruling_party",
    "leader",
    "ideas",
    "ideas_other",
    "national_spirits",
    "laws",
    "idea_groups",
    "advisor_ideas",
    "idea_count",
    "election_frequency",
    "party_count",
    "low_confidence",
    "last_election",
    "elections_allowed",
    "appointed_advisors",
    "appointed_advisor_details",
    "appointed_advisor_slots",
    "appointed_advisor_ledgers",
))
COUNTRY_POLITICS_LIVE_PAYLOAD_KEYS = frozenset((
    "popularities",
    "ruling_party",
    "leader",
    "ideas",
    "ideas_other",
    "national_spirits",
    "laws",
    "idea_groups",
    "advisor_ideas",
    "idea_count",
    "election_frequency",
    "party_count",
    "low_confidence",
))
COUNTRY_POLITICS_RESIDUAL_PAYLOAD_KEYS = frozenset((
    "last_election",
    "elections_allowed",
    "appointed_advisors",
    "appointed_advisor_details",
    "appointed_advisor_slots",
    "appointed_advisor_ledgers",
))
COUNTRY_POLITICS_MAX_IDEAS = 400
COUNTRY_POLITICS_MAX_LAWS = 32
COUNTRY_POLITICS_MAX_TOKEN_LENGTH = 160
COUNTRY_LEADER_CHAR_ID_MAX = 10_000_000
COUNTRY_LEADER_NAME_MAX_LENGTH = 256
COUNTRY_LEADER_PAYLOAD_KEYS = frozenset((
    "char_id",
    "name",
    "token",
    "ideology_group",
    "ambiguous",
    "source",
))
COUNTRY_FOCUS_PAYLOAD_KEYS = frozenset((
    "focus_tree",
    "completed",
    "completed_count",
    "current",
    "current_continuous",
    "progress",
))
COUNTRY_FOCUS_MAX_COMPLETED = 4000
COUNTRY_FOCUS_MAX_TOKEN_LENGTH = 160
COUNTRY_FOCUS_PROGRESS_MAX = 100_000.0
COUNTRY_RESEARCH_PAYLOAD_KEYS = frozenset((
    "completed_tech_count",
    "research_slot_count",
    "active_research",
))
COUNTRY_RESEARCH_MAX_COMPLETED_TECHS = 2000
COUNTRY_RESEARCH_MAX_SLOTS = 32
COUNTRY_RESEARCH_MAX_ACTIVE = 32
COUNTRY_RESEARCH_RAW_VALUE_MAX = 9_223_372_036_854_775_807
COUNTRY_RESEARCH_PROGRESS_UNITS_MAX = COUNTRY_RESEARCH_RAW_VALUE_MAX / 1000.0
COUNTRY_RESEARCH_TOKEN_MAX_LENGTH = 160
SPECIAL_PROJECTS_PAYLOAD_KEYS = frozenset((
    "facilities",
    "projects",
    "scientists",
    "count",
    "source",
))
SPECIAL_PROJECT_FACILITY_KEYS = frozenset((
    "id",
    "province",
    "building",
    "active_project_handle",
    "active",
    "scientist_char_id",
    "unread_prototype_rewards",
))
SPECIAL_PROJECT_ROW_KEYS = frozenset((
    "id",
    "static_def_handle",
    "iterations",
    "worked_at_facility",
))
SPECIAL_PROJECT_SCIENTIST_KEYS = frozenset((
    "char_id",
    "name_token",
    "display_name",
    "skills",
    "trait_count",
))
SPECIAL_PROJECT_SKILL_KEYS = frozenset((
    "specialization_nuclear",
    "specialization_naval",
    "specialization_air",
    "specialization_land",
))
SPECIAL_PROJECT_MAX_FACILITIES = 4096
SPECIAL_PROJECT_MAX_PROJECTS = 4096
SPECIAL_PROJECT_MAX_SCIENTISTS = 4096
SPECIAL_PROJECT_ID_MAX = 10_000_000
SPECIAL_PROJECT_PROVINCE_MAX = 1_000_000
SPECIAL_PROJECT_HANDLE_MAX = 9_223_372_036_854_775_807
SPECIAL_PROJECT_TOKEN_MAX_LENGTH = 160
SPECIAL_PROJECT_DISPLAY_MAX_LENGTH = 256
SPECIAL_PROJECT_ITERATIONS_MAX = 1_000_000
SPECIAL_PROJECT_UNREAD_REWARDS_MAX = 1_000_000
SPECIAL_PROJECT_TRAIT_COUNT_MAX = 256
SPECIAL_PROJECT_SKILL_LEVEL_MAX = 100
COUNTRY_DECISIONS_PAYLOAD_KEYS = frozenset((
    "status_present",
    "timed_live",
))
COUNTRY_DECISIONS_MAX_STATUS_KEYS = 32
COUNTRY_DECISIONS_STATUS_COUNT_MAX = 4096
COUNTRY_DECISIONS_MAX_TIMED = 4096
COUNTRY_DECISIONS_DAYS_MAX = 100_000
COUNTRY_DECISIONS_STATE_MAX = 1024
COUNTRY_DECISIONS_TOKEN_MAX_LENGTH = 160
COUNTRY_DIPLOMACY_PAYLOAD_KEYS = frozenset((
    "faction_group",
    "at_war",
    "casualties",
))
COUNTRY_DIPLOMACY_FACTION_GROUP_ABS_MAX = 10_000_000
COUNTRY_DIPLOMACY_CASUALTIES_MAX = 1_000_000_000_000
DIPLOMACY_LIVE_PAYLOAD_KEYS = frozenset((
    "war_relations",
    "faction_members",
    "subtype_warning",
    "source",
))
DIPLOMACY_LIVE_MAX_WAR_RELATIONS = 10_000
PRODUCTION_PAYLOAD_KEYS = frozenset((
    "stockpile",
    "stockpile_total",
    "military_lines",
    "naval_lines",
    "military_factories",
    "naval_factories",
    "construction_queue",
))
RAW_FRAME_TYPES = frozenset(("domain_delta", "event", "gap"))
RAW_CONFIDENCES = frozenset(("exact", "sampled", "proxy", "gap"))
RAW_DOMAIN_DELTA_OPS = frozenset(("upsert", "delete", "sample"))
RAW_GAP_KINDS = frozenset((
    "dropped_frames",
    "validation_failed",
    "overload",
    "layout_invalid",
    "race_detected",
    "unsupported",
))
RAW_SOURCE = "C"
RAW_SEQUENCE_MAX = 9_223_372_036_854_775_807
RAW_STRING_MAX_LENGTH = 512
RAW_SHORT_STRING_MAX_LENGTH = 128
RAW_CONTAINER_MAX_ENTRIES = 256
RAW_DETAIL_MAX_DEPTH = 5
RAW_DIVISION_ID_MAX = 2_147_483_647
RAW_DIVISION_PROVINCE_ID_MAX = 1_000_000
RAW_DIVISION_PROGRESS_BUCKET_MAX = 1_000_000
RAW_DIVISION_MOVEMENT_PROGRESS_MAX = 1_000_000.0
RAW_DIVISION_ORGANISATION_MAX = 1_000.0
RAW_DIVISION_STRENGTH_MAX = 10_000.0
RAW_DIVISION_COMBAT_REF_COUNT_MAX = 1_000
RAW_DIVISION_MOVEMENT_FIELDS = frozenset((
    "div_uid",
    "div_id",
    "div_type",
    "moving",
    "movement_source_province",
    "move_target_province",
    "location",
    "movement_progress",
    "progress_bucket",
    "owner",
    "organisation",
    "strength",
    "combat_ref_count",
    "in_combat",
))
RAW_DIVISION_EVENT_TYPES = frozenset((
    "division_movement_started",
    "division_movement_stopped",
    "division_org_strength_delta",
    "division_combat_ref_changed",
))
RAW_DIVISION_EVENT_DETAIL_FIELDS = RAW_DIVISION_MOVEMENT_FIELDS | frozenset((
    "changed_fields",
))
RAW_LAND_COMBAT_ID_MAX = 2_147_483_647
RAW_LAND_COMBAT_GENERATION_MAX = RAW_SEQUENCE_MAX
RAW_LAND_COMBAT_PROVINCE_ID_MAX = 1_000_000
RAW_LAND_COMBAT_AGE_DAYS_MAX = 100_000
RAW_LAND_COMBAT_PARTICIPANTS_MAX = RAW_CONTAINER_MAX_ENTRIES
RAW_LAND_COMBAT_SIDE_STATUSES = frozenset((
    "exact",
    "unknown",
    "validation_failed",
))
RAW_LAND_COMBAT_EVENT_TYPES = frozenset((
    "land_combat_started",
    "land_combat_updated",
    "land_combat_ended",
))
RAW_LAND_COMBAT_END_REASONS = frozenset((
    "resolved",
    "id_reuse",
    "body_changed",
    "manager_gap",
))
RAW_LAND_COMBAT_FIELDS = frozenset((
    "combat_key",
    "combat_id",
    "generation",
    "location",
    "day",
    "duration",
    "attacker_divisions",
    "defender_divisions",
    "attacker_status",
    "defender_status",
))
RAW_LAND_COMBAT_EVENT_DETAIL_FIELDS = RAW_LAND_COMBAT_FIELDS | frozenset((
    "changed_fields",
    "end_reason",
))
RAW_LAND_COMBAT_GAP_FIELDS = frozenset((
    "gap_key",
    "gap_kind",
    "status",
))
RAW_ARMY_ORDER_ID_MAX = 2_147_483_647
RAW_ARMY_ORDER_GENERATION_MAX = RAW_SEQUENCE_MAX
RAW_ARMY_ORDER_SLOT_INDEX_MAX = 255
RAW_ARMY_ORDER_TYPE_ABS_MAX = 10_000
RAW_ARMY_ORDER_TARGET_STATES_MAX = RAW_CONTAINER_MAX_ENTRIES
RAW_ARMY_ORDER_TARGET_STATE_ID_MAX = 65_535
RAW_ARMY_ORDER_TARGET_STATE_COUNT_MAX = 5_000
RAW_ARMY_ORDER_TARGET_STATUSES = frozenset((
    "exact",
    "unsupported",
    "overflow",
    "validation_failed",
))
RAW_ARMY_ORDER_EVENT_TYPES = frozenset((
    "army_order_added",
    "army_order_updated",
    "army_order_removed",
))
RAW_ARMY_ORDER_FIELDS = frozenset((
    "order_key",
    "army_id",
    "generation",
    "slot_index",
    "order_type",
    "target_states",
    "target_state_count",
    "target_status",
))
RAW_ARMY_ORDER_EVENT_DETAIL_FIELDS = RAW_ARMY_ORDER_FIELDS | frozenset((
    "changed_fields",
))
RAW_ARMY_ORDER_GAP_FIELDS = frozenset((
    "gap_key",
    "gap_kind",
    "status",
))
RAW_NAVAL_COMBAT_ID_MAX = 2_147_483_647
RAW_NAVAL_COMBAT_GENERATION_MAX = RAW_SEQUENCE_MAX
RAW_NAVAL_COMBAT_PROVINCE_ID_MAX = 1_000_000
RAW_NAVAL_COMBAT_SIDE_TAGS_MAX = 64
RAW_NAVAL_COMBAT_SHIP_COUNT_MAX = 20_000
RAW_NAVAL_COMBAT_POSITIONING_MAX = 10.0
RAW_NAVAL_COMBAT_SIDE_STATUSES = frozenset((
    "exact",
    "unknown",
    "validation_failed",
))
RAW_NAVAL_COMBAT_EVENT_TYPES = frozenset((
    "naval_combat_started",
    "naval_combat_updated",
    "naval_combat_ended",
))
RAW_NAVAL_COMBAT_END_REASONS = frozenset((
    "resolved",
    "id_reuse",
    "body_changed",
    "bridge_gap",
))
RAW_NAVAL_COMBAT_FIELDS = frozenset((
    "combat_key",
    "combat_id",
    "generation",
    "location",
    "attacker_tags",
    "defender_tags",
    "attacker_ship_count",
    "defender_ship_count",
    "attacker_commander",
    "defender_commander",
    "attacker_positioning",
    "defender_positioning",
    "attacker_status",
    "defender_status",
))
RAW_NAVAL_COMBAT_EVENT_DETAIL_FIELDS = RAW_NAVAL_COMBAT_FIELDS | frozenset((
    "changed_fields",
    "end_reason",
))
RAW_NAVAL_COMBAT_GAP_FIELDS = frozenset((
    "gap_key",
    "gap_kind",
    "status",
))
RAW_NAVAL_COMBAT_MEMBER_ID_MAX = 2_147_483_647
RAW_NAVAL_COMBAT_MEMBER_GENERATION_MAX = RAW_SEQUENCE_MAX
RAW_NAVAL_COMBAT_MEMBER_GROUP_INDEX_MAX = 256
RAW_NAVAL_COMBAT_MEMBER_INDEX_MAX = 1000
RAW_NAVAL_COMBAT_MEMBER_SLOT_INDEX_MAX = 20_000
RAW_NAVAL_COMBAT_MEMBER_UNIT_TYPE_MAX = 10_000
RAW_NAVAL_COMBAT_MEMBER_SHIP_INSTANCE_ID_MAX = 4_000_000
RAW_NAVAL_COMBAT_MEMBER_MAX_STRENGTH_RAW_MAX = 1_000_000_000
RAW_NAVAL_COMBAT_MEMBER_STATE_BITMASK_MAX = 255
RAW_NAVAL_COMBAT_MEMBER_ENGAGEMENT_WEIGHT_RAW_MAX = 10_000_000
RAW_NAVAL_COMBAT_MEMBER_SIDES = frozenset((
    "attacker",
    "defender",
))
RAW_NAVAL_COMBAT_MEMBER_FIELDS = frozenset((
    "member_key",
    "combat_key",
    "combat_id",
    "generation",
    "side",
    "group_index",
    "member_index",
    "unit_type_tag",
    "ship_instance_id",
    "is_convoy",
    "owner",
    "name",
    "type_display",
    "archetype",
    "designed_max_strength_raw",
    "battle_slot_index_raw",
    "state_bitmask",
    "engagement_weight_raw",
))
RAW_NAVAL_COMBAT_MEMBER_GAP_FIELDS = frozenset((
    "gap_key",
    "gap_kind",
    "status",
))
RAW_AIR_WING_ID_MAX = 2_147_483_647
RAW_AIR_WING_GENERATION_MAX = RAW_SEQUENCE_MAX
RAW_AIR_WING_PLANES_MAX = 10_000_000
RAW_AIR_WING_MISSION_MASK_MAX = 4_294_967_295
RAW_AIR_WING_REGION_ID_MAX = 65_535
RAW_AIR_WING_EVENT_TYPES = frozenset((
    "air_wing_observed",
    "air_wing_updated",
    "air_wing_lost",
))
RAW_AIR_WING_FIELDS = frozenset((
    "air_wing_key",
    "owner",
    "wing_id",
    "generation",
    "count",
    "mission_mask",
    "mission_region",
    "effective_planes",
))
RAW_AIR_WING_EVENT_DETAIL_FIELDS = RAW_AIR_WING_FIELDS | frozenset((
    "changed_fields",
))
RAW_AIR_WING_GAP_FIELDS = frozenset((
    "gap_key",
    "gap_kind",
    "status",
))
RAW_SHIP_INSTANCE_ID_MAX = 4_000_000
RAW_SHIP_FIXED_RAW_MAX = 1_000_000_000
RAW_SHIP_PERCENT_RAW_MAX = 100_000_000
RAW_SHIP_STRENGTH_HP_MAX = 10_000.0
RAW_SHIP_NORMALIZED_MAX = 1000.0
RAW_SHIP_SUNK_CONVOYS_MAX = 100_000_000
RAW_SHIP_CATEGORY_LABELS = {
    2: "submarine",
    3: "destroyer",
    5: "light_cruiser",
    6: "heavy_cruiser",
    8: "battle_cruiser",
    9: "battleship",
    10: "carrier",
}
RAW_SHIP_FIELDS = frozenset((
    "ship_key",
    "instance_id",
    "definition",
    "category_enum",
    "category",
    "strength_raw",
    "strength_hp",
    "organisation_raw",
    "organisation",
    "experience_raw",
    "experience",
    "sunk_convoys",
))
RAW_SHIP_GAP_FIELDS = frozenset((
    "gap_key",
    "gap_kind",
    "status",
))
RAW_OOB_COMMAND_ID_MAX = 4_000_000
RAW_OOB_COMMAND_IDS_MAX = RAW_CONTAINER_MAX_ENTRIES
RAW_OOB_COMMAND_SHIP_COUNT_MAX = 20_000
RAW_OOB_COMMAND_MEMBER_COUNT_MAX = 20_000
RAW_OOB_COMMAND_LEADER_SKILL_MAX = 15
RAW_OOB_COMMAND_LEADER_TRAITS_MAX = 64
RAW_OOB_COMMAND_ENTITY_KINDS = frozenset((
    "army",
    "army_group",
    "fleet",
    "task_force",
))
RAW_OOB_COMMAND_STATUSES = frozenset((
    "exact",
    "unknown",
    "overflow",
    "validation_failed",
))
RAW_OOB_COMMAND_FIELDS = frozenset((
    "oob_key",
    "entity_kind",
    "army_id",
    "army_group_id",
    "fleet_id",
    "task_force_id",
    "member_division_ids",
    "member_division_count",
    "member_division_status",
    "member_army_ids",
    "member_army_count",
    "member_army_status",
    "leader_id",
    "leader_status",
    "member_count_engine",
    "missing_member_count",
    "task_force_ids",
    "task_force_count",
    "task_force_status",
    "parent_status",
    "ship_ids",
    "ship_count",
    "ship_status",
    "leader_skill",
    "leader_traits",
    "leader_trait_status",
))
RAW_OOB_COMMAND_GAP_FIELDS = frozenset((
    "gap_key",
    "gap_kind",
    "status",
))
RAW_FORBIDDEN_KEY_NAMES = frozenset((
    "addr",
    "address",
    "ptr",
    "pointer",
    "body_addr",
    "tag_db_id",
))
RAW_FORBIDDEN_KEY_SUFFIXES = ("_addr", "_ptr", "_tag_db_id")


def make_heartbeat_frame(
    *,
    collector_version: str,
    sequence: Optional[int] = None,
    backend: str = COLLECTOR_BACKEND,
) -> Dict[str, Any]:
    """Build a minimal heartbeat frame matching the DLL skeleton ABI."""
    frame: Dict[str, Any] = {
        "type": "heartbeat",
        "backend": backend,
        "collector_version": collector_version,
        "abi_version": COLLECTOR_ABI_VERSION,
        "domains": [],
        "quality_status": "ok",
    }
    if sequence is not None:
        frame["sequence"] = sequence
    return frame


def validate_collector_frame(frame: Any) -> List[str]:
    """Return human-readable problems; empty means the frame is adapter-safe."""
    problems: List[str] = []
    if not isinstance(frame, dict):
        return ["collector frame not a dict"]

    frame_type = frame.get("type")
    if frame_type not in FRAME_TYPES:
        problems.append(f"collector frame type {frame_type!r} invalid")

    for key in ("backend", "collector_version", "exe_sha256", "mod_checksum",
                "profile_id", "quality_status", "capture_boundary", "transport",
                "transport_crc32", "session_id"):
        value = frame.get(key)
        if value is not None and not isinstance(value, str):
            problems.append(f"collector frame {key} {value!r} not str|null")
    backend = frame.get("backend")
    if backend != COLLECTOR_BACKEND:
        problems.append(
            f"collector frame backend {backend!r} != {COLLECTOR_BACKEND!r}")
    segment_id = frame.get("segment_id")
    if segment_id is not None and not isinstance(segment_id, (int, str)):
        problems.append(f"collector frame segment_id {segment_id!r} not int|str|null")

    abi = frame.get("abi_version")
    if abi is not None and not isinstance(abi, int):
        problems.append(f"collector frame abi_version {abi!r} not int|null")
    elif abi != COLLECTOR_ABI_VERSION:
        problems.append(
            f"collector frame abi_version {abi!r} != {COLLECTOR_ABI_VERSION}")
    sequence = frame.get("sequence")
    if sequence is not None and not isinstance(sequence, int):
        problems.append(f"collector frame sequence {sequence!r} not int|null")
    for key in ("process_id", "tick_ms", "transport_bytes", "session_sequence",
                "sample_ms", "duration_ms", "elapsed_wall_ms"):
        value = frame.get(key)
        if value is not None and not isinstance(value, int):
            problems.append(f"collector frame {key} {value!r} not int|null")

    date = frame.get("date")
    if date is not None and not isinstance(date, str):
        problems.append(f"collector frame date {date!r} not str|null")
    if frame_type == "snapshot" and date is None:
        problems.append("collector snapshot frame missing date")

    domains = frame.get("domains", [])
    if not isinstance(domains, list):
        problems.append(f"collector frame domains {domains!r} not list")
    else:
        for i, domain in enumerate(domains):
            if not isinstance(domain, str):
                problems.append(f"collector frame domains[{i}] {domain!r} not str")
            elif domain not in schema.DOMAINS:
                problems.append(f"collector frame domains[{i}] {domain!r} not canonical")

    countries = frame.get("countries")
    if countries is not None:
        if not isinstance(countries, dict):
            problems.append(f"collector frame countries {countries!r} not dict|null")
        else:
            problems.extend(_validate_country_rows(countries))

    states = frame.get("states")
    if states is not None:
        if not isinstance(states, dict):
            problems.append(f"collector frame states {states!r} not dict|null")
        else:
            problems.extend(_validate_state_rows(states))

    provinces = frame.get("provinces")
    if provinces is not None:
        if not isinstance(provinces, dict):
            problems.append(f"collector frame provinces {provinces!r} not dict|null")
        else:
            problems.extend(_validate_province_rows(provinces))

    special_projects = frame.get("special_projects")
    if special_projects is not None:
        if not isinstance(special_projects, dict):
            problems.append(
                f"collector frame special_projects {special_projects!r} not dict|null")
        else:
            problems.extend(_validate_special_projects(special_projects))

    diplomacy_live = frame.get("diplomacy_live")
    if diplomacy_live is not None:
        if not isinstance(diplomacy_live, dict):
            problems.append(
                f"collector frame diplomacy_live {diplomacy_live!r} not dict|null")
        else:
            problems.extend(_validate_diplomacy_live_sidecar(diplomacy_live))

    root_probe = frame.get("root_probe")
    scheduled_domain_claims = _claimed_scheduled_domains(frame, root_probe)
    if frame.get("capture_boundary") == "worker_poll":
        claimed_worker_heavy = sorted(
            scheduled_domain_claims & WORKER_POLL_FORBIDDEN_DOMAINS
        )
        if claimed_worker_heavy:
            problems.append(
                "collector worker_poll frame must not claim scheduled domains: "
                + ",".join(claimed_worker_heavy)
            )
        if _has_country_fuel_rows(countries):
            problems.append("collector worker_poll frame must not carry fuel payload")
        if _has_country_production_payload(countries):
            problems.append(
                "collector worker_poll frame must not carry production payload")
        if _has_country_focus_payload(countries):
            problems.append("collector worker_poll frame must not carry focus payload")
        if _has_country_research_payload(countries):
            problems.append("collector worker_poll frame must not carry research payload")
        if _has_country_politics_rows(countries):
            problems.append("collector worker_poll frame must not carry politics payload")
        if _has_country_decisions_live_rows(countries):
            problems.append("collector worker_poll frame must not carry decisions payload")
        if _has_country_diplomacy_rows(countries):
            problems.append("collector worker_poll frame must not carry diplomacy payload")
        if _has_diplomacy_live_sidecar(diplomacy_live):
            problems.append(
                "collector worker_poll frame must not carry diplomacy_live sidecar")
        if _has_payload_rows(states):
            problems.append("collector worker_poll frame must not carry states payload")
        if _has_payload_rows(provinces):
            problems.append("collector worker_poll frame must not carry provinces payload")
        if _has_special_projects_payload(special_projects):
            problems.append(
                "collector worker_poll frame must not carry special_projects payload")
    if (
        frame.get("capture_boundary") != "worker_poll"
        and
        frame.get("capture_boundary") != "scheduled_hour_snapshot"
        and ("fuel_live" in scheduled_domain_claims
             or _has_country_fuel_rows(countries))
    ):
        problems.append("collector fuel_live payload requires scheduled_hour_snapshot")
    if (
        frame.get("capture_boundary") != "worker_poll"
        and
        frame.get("capture_boundary") != "scheduled_hour_snapshot"
        and (
            "production_live" in scheduled_domain_claims
            or _has_country_production_payload(countries)
        )
    ):
        problems.append("collector production_live payload requires scheduled_hour_snapshot")
    if (
        frame.get("capture_boundary") != "worker_poll"
        and
        frame.get("capture_boundary") != "scheduled_hour_snapshot"
        and (
            "focus_live" in scheduled_domain_claims
            or _has_country_focus_payload(countries)
        )
    ):
        problems.append("collector focus_live payload requires scheduled_hour_snapshot")
    if (
        frame.get("capture_boundary") != "worker_poll"
        and
        frame.get("capture_boundary") != "scheduled_hour_snapshot"
        and (
            "tech_live" in scheduled_domain_claims
            or _has_country_research_payload(countries)
        )
    ):
        problems.append("collector tech_live payload requires scheduled_hour_snapshot")
    if (
        frame.get("capture_boundary") != "worker_poll"
        and
        frame.get("capture_boundary") != "scheduled_hour_snapshot"
        and (
            "politics_live" in scheduled_domain_claims
            or _has_country_politics_live_rows(countries)
        )
    ):
        problems.append("collector politics_live payload requires scheduled_hour_snapshot")
    if (
        frame.get("capture_boundary") != "worker_poll"
        and
        frame.get("capture_boundary") != "scheduled_hour_snapshot"
        and (
            "decisions_live" in scheduled_domain_claims
            or _has_country_decisions_live_rows(countries)
        )
    ):
        problems.append("collector decisions_live payload requires scheduled_hour_snapshot")
    if (
        frame.get("capture_boundary") != "worker_poll"
        and
        frame.get("capture_boundary") != "scheduled_hour_snapshot"
        and (
            "diplomacy_live" in scheduled_domain_claims
            or _has_country_diplomacy_rows(countries)
            or _has_diplomacy_live_sidecar(diplomacy_live)
        )
    ):
        problems.append("collector diplomacy_live payload requires scheduled_hour_snapshot")
    if (
        frame.get("capture_boundary") != "worker_poll"
        and
        frame.get("capture_boundary") != "scheduled_hour_snapshot"
        and (
            "politics_residuals" in scheduled_domain_claims
            or _has_country_politics_residual_rows(countries)
        )
    ):
        problems.append(
            "collector politics_residuals payload requires scheduled_hour_snapshot")
    if (
        frame.get("capture_boundary") != "worker_poll"
        and
        frame.get("capture_boundary") != "scheduled_hour_snapshot"
        and (
            scheduled_domain_claims & MAP_CONTROL_SNAPSHOT_DOMAINS
            or _has_payload_rows(states)
            or _has_payload_rows(provinces)
        )
    ):
        problems.append(
            "collector map-control payload requires scheduled_hour_snapshot")
    if (
        frame.get("capture_boundary") != "worker_poll"
        and
        frame.get("capture_boundary") != "scheduled_hour_snapshot"
        and (
            "buildings_live" in scheduled_domain_claims
            or _has_building_rows(states, provinces)
        )
    ):
        problems.append("collector buildings_live payload requires scheduled_hour_snapshot")
    if (
        frame.get("capture_boundary") != "worker_poll"
        and
        frame.get("capture_boundary") != "scheduled_hour_snapshot"
        and (
            "projects_live" in scheduled_domain_claims
            or _has_special_projects_payload(special_projects)
        )
    ):
        problems.append("collector projects_live payload requires scheduled_hour_snapshot")

    if root_probe is not None:
        if not isinstance(root_probe, dict):
            problems.append(f"collector frame root_probe {root_probe!r} not dict|null")
        else:
            problems.extend(_validate_root_probe(root_probe))
    if frame_type in RAW_FRAME_TYPES:
        problems.extend(_validate_raw_frame(frame_type, frame))

    return problems


def read_collector_jsonl(path: Path) -> List[Dict[str, Any]]:
    """Read collector JSONL frames, failing closed on invalid JSON or frame shape."""
    path = Path(path)
    frames: List[Dict[str, Any]] = []
    with path.open("r", encoding="utf-8") as f:
        for lineno, raw in enumerate(f, 1):
            line = raw.strip()
            if not line:
                continue
            try:
                frame = json.loads(line)
            except json.JSONDecodeError as e:
                raise ValueError(f"{path}:{lineno}: invalid collector JSON: {e}") from e
            problems = validate_collector_frame(frame)
            if problems:
                raise ValueError(f"{path}:{lineno}: invalid collector frame: "
                                 + "; ".join(problems))
            frames.append(frame)
    return frames


def latest_verified_root_probe(frames: Iterable[Dict[str, Any]]) -> Optional[Dict[str, Any]]:
    """Return the newest verified root probe, ignoring controller-process attaches."""
    latest: Optional[Dict[str, Any]] = None
    for frame in frames:
        root_probe = frame.get("root_probe")
        if isinstance(root_probe, dict) and root_probe.get("status") == "verified":
            latest = root_probe
    return latest


def snapshot_from_root_probe_frame(frame: Dict[str, Any], game_id: str) -> Dict[str, Any]:
    """Convert a verified root-probe frame into a schema snapshot.

    A root probe always claims the live clock. If the collector also emits
    validated top-level country rows, those become `country_identity` evidence:
    country keys are measured-present, while undecoded identity fields remain
    null/absent.
    """
    problems = validate_collector_frame(frame)
    if problems:
        raise ValueError("invalid collector frame: " + "; ".join(problems))
    root_probe = frame.get("root_probe")
    if not isinstance(root_probe, dict) or root_probe.get("status") != "verified":
        raise ValueError("collector frame does not contain a verified root_probe")
    date = root_probe.get("current_game_date")
    if not isinstance(date, str):
        raise ValueError("collector root_probe missing current_game_date")

    frame_countries = frame.get("countries") or {}
    domains = ["clock"]
    if _has_present_country_rows(frame_countries):
        domains.append("country_identity")
    if _has_country_scalar_rows(frame_countries):
        domains.append("country_scalars")
    if _has_country_misc_rows(frame_countries):
        domains.append("country_misc")
    frame_states = frame.get("states") or {}
    frame_provinces = frame.get("provinces") or {}
    frame_special_projects = frame.get("special_projects") or {}
    claimed_domains = _explicit_scheduled_domains(frame, root_probe)
    if "fuel_live" in claimed_domains and _has_country_fuel_rows(frame_countries):
        domains.append("fuel_live")
    if (
        "production_live" in claimed_domains
        and _has_country_production_rows(frame_countries)
    ):
        domains.append("production_live")
    if "focus_live" in claimed_domains and _has_country_focus_rows(frame_countries):
        domains.append("focus_live")
    if "tech_live" in claimed_domains and _has_country_research_rows(frame_countries):
        domains.append("tech_live")
    if "politics_live" in claimed_domains and _has_country_politics_live_rows(frame_countries):
        domains.append("politics_live")
    if (
        "politics_residuals" in claimed_domains
        and _has_country_politics_residual_rows(frame_countries)
    ):
        domains.append("politics_residuals")
    if (
        "decisions_live" in claimed_domains
        and _has_country_decisions_live_rows(frame_countries)
    ):
        domains.append("decisions_live")
    if (
        "diplomacy_live" in claimed_domains
        and (
            _has_country_diplomacy_rows(frame_countries)
            or _has_diplomacy_live_sidecar(frame.get("diplomacy_live"))
        )
    ):
        domains.append("diplomacy_live")
    if "state_ownership" in claimed_domains and _has_state_owner_rows(frame_states):
        domains.append("state_ownership")
    if "state_control" in claimed_domains and _has_state_control_rows(frame_states):
        domains.append("state_control")
    if "province_control" in claimed_domains and _has_province_control_rows(frame_provinces):
        domains.append("province_control")
    if "buildings_live" in claimed_domains and _has_building_rows(
        frame_states,
        frame_provinces,
    ):
        domains.append("buildings_live")
    if "projects_live" in claimed_domains and _has_special_projects_rows(frame_special_projects):
        domains.append("projects_live")

    snap = schema.empty_snapshot(game_id, date)
    snap["provenance"]["collector"] = _collector_provenance(frame, domains)
    snap["provenance"]["collector"]["root_probe"] = root_probe
    _fold_country_identity(snap, frame_countries)
    _fold_country_scalars(snap, frame_countries)
    _fold_country_misc(snap, frame_countries, domains)
    _fold_country_fuel(snap, frame_countries, domains)
    _fold_country_production(snap, frame_countries, domains)
    _fold_country_focus(snap, frame_countries, domains)
    _fold_country_research(snap, frame_countries, domains)
    _fold_country_politics(snap, frame_countries, domains)
    _fold_country_politics_residuals(snap, frame_countries, domains)
    _fold_country_decisions_live(snap, frame_countries, domains)
    _fold_country_diplomacy_live(
        snap, frame_countries, domains, frame.get("diplomacy_live"))
    _fold_state_rows(snap, frame_states, domains)
    _fold_province_rows(snap, frame_provinces, domains)
    _fold_special_projects(snap, frame_special_projects, domains)
    _fill_map_mirrors(snap)
    world = snap.get("world")
    if not isinstance(world, dict):
        world = schema.empty_world()
        snap["world"] = world
    scenario = world.get("scenario")
    if not isinstance(scenario, dict):
        scenario = {}
        world["scenario"] = scenario
    scenario["current_date_live"] = date
    start_date = root_probe.get("start_date")
    if isinstance(start_date, str):
        scenario["start_date_live"] = start_date

    schema_problems = schema.validate_snapshot(snap)
    if schema_problems:
        raise ValueError("collector root-probe snapshot failed schema validation: "
                         + "; ".join(schema_problems))
    return snap


def snapshot_from_collector_frame(frame: Dict[str, Any], game_id: str) -> Dict[str, Any]:
    """Convert one collector snapshot frame into a schema-valid SNAPSHOT skeleton."""
    problems = validate_collector_frame(frame)
    if problems:
        raise ValueError("invalid collector frame: " + "; ".join(problems))
    if frame.get("type") != "snapshot":
        raise ValueError(f"collector frame type {frame.get('type')!r} is not a snapshot")

    snap = schema.empty_snapshot(game_id, frame.get("date"))
    domains = _clean_domains(frame.get("domains") or [])
    if frame.get("capture_boundary") != "scheduled_hour_snapshot":
        domains = [
            domain for domain in domains
            if domain not in SCHEDULED_ONLY_SNAPSHOT_DOMAINS
        ]
    unsupported = sorted(set(domains) - SUPPORTED_SNAPSHOT_DOMAINS)
    if unsupported:
        raise ValueError(
            "collector snapshot claims unsupported domains: "
            + ",".join(unsupported)
        )
    snap["provenance"]["collector"] = _collector_provenance(frame, domains)
    _fold_country_identity(snap, frame.get("countries") or {})
    _fold_country_scalars(snap, frame.get("countries") or {})
    _fold_country_misc(snap, frame.get("countries") or {}, domains)
    _fold_country_fuel(snap, frame.get("countries") or {}, domains)
    _fold_country_production(snap, frame.get("countries") or {}, domains)
    _fold_country_focus(snap, frame.get("countries") or {}, domains)
    _fold_country_research(snap, frame.get("countries") or {}, domains)
    _fold_country_politics(snap, frame.get("countries") or {}, domains)
    _fold_country_politics_residuals(snap, frame.get("countries") or {}, domains)
    _fold_country_decisions_live(snap, frame.get("countries") or {}, domains)
    _fold_country_diplomacy_live(
        snap, frame.get("countries") or {}, domains, frame.get("diplomacy_live"))
    _fold_state_rows(snap, frame.get("states") or {}, domains)
    _fold_province_rows(snap, frame.get("provinces") or {}, domains)
    _fold_special_projects(snap, frame.get("special_projects") or {}, domains)
    _fill_map_mirrors(snap)

    schema_problems = schema.validate_snapshot(snap)
    if schema_problems:
        raise ValueError("collector snapshot failed schema validation: "
                         + "; ".join(schema_problems))
    return snap


def _collector_provenance(frame: Dict[str, Any], domains: List[str]) -> Dict[str, Any]:
    prov: Dict[str, Any] = {
        "captured": True,
        "backend": frame.get("backend") or COLLECTOR_BACKEND,
        "collector_version": frame.get("collector_version"),
        "exe_sha256": frame.get("exe_sha256"),
        "mod_checksum": frame.get("mod_checksum"),
        "profile_id": frame.get("profile_id"),
        "quality_status": frame.get("quality_status") or "ok",
        "capture_boundary": frame.get("capture_boundary"),
        "transport": frame.get("transport"),
        "transport_crc32": frame.get("transport_crc32"),
        "transport_bytes": frame.get("transport_bytes"),
        "sequence": frame.get("sequence"),
        "session_id": frame.get("session_id"),
        "session_sequence": frame.get("session_sequence"),
        "duration_ms": frame.get("duration_ms"),
        "sample_ms": frame.get("sample_ms"),
        "elapsed_wall_ms": frame.get("elapsed_wall_ms"),
        "domains": domains,
    }
    abi = frame.get("abi_version")
    if abi is not None:
        prov["abi_version"] = abi
    return prov


def _clean_domains(domains: Iterable[Any]) -> List[str]:
    return sorted({d for d in domains if isinstance(d, str) and d in schema.DOMAINS})


def _validate_raw_frame(frame_type: str, frame: Dict[str, Any]) -> List[str]:
    problems: List[str] = []
    for key in ("sequence", "tick_ms", "hour_index", "sample_ms"):
        problems.extend(_require_raw_int(frame, key))
    problems.extend(_require_raw_int(frame, "session_sequence", required=False))

    date = frame.get("date")
    if not _valid_raw_string(date, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        problems.append("collector raw frame date missing or not bounded str")
    else:
        parsed_hour = schema.date_to_hour_index(date)
        if parsed_hour is None:
            problems.append(f"collector raw frame date {date!r} not valid game date")
        elif (
            _valid_bounded_int(
                frame.get("hour_index"),
                minimum=0,
                maximum=RAW_SEQUENCE_MAX,
            )
            and frame.get("hour_index") != parsed_hour
        ):
            problems.append(
                "collector raw frame hour_index "
                f"{frame.get('hour_index')!r} != date_to_hour_index({date!r})"
            )

    capture_boundary = frame.get("capture_boundary")
    if not _valid_raw_string(capture_boundary, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        problems.append("collector raw frame capture_boundary missing or not bounded str")

    domain = frame.get("domain")
    if not _valid_raw_string(domain, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        problems.append("collector raw frame domain missing or not bounded str")
    elif domain not in schema.DOMAINS:
        problems.append(f"collector raw frame domain {domain!r} not canonical")

    entity_key = frame.get("entity_key")
    if not _valid_raw_key(entity_key):
        problems.append("collector raw frame entity_key missing or not stable str")

    source = frame.get("source")
    if source != RAW_SOURCE:
        problems.append(f"collector raw frame source {source!r} != {RAW_SOURCE!r}")

    confidence = frame.get("confidence")
    if confidence not in RAW_CONFIDENCES:
        problems.append(f"collector raw frame confidence {confidence!r} invalid")

    diagnostics = frame.get("diagnostics")
    if diagnostics is not None and not isinstance(diagnostics, (dict, list)):
        problems.append("collector raw frame diagnostics not object|list|null")
    elif diagnostics is not None:
        problems.extend(_validate_bounded_raw_value(
            diagnostics,
            "collector raw frame diagnostics",
        ))

    payload = frame.get("payload")
    if not isinstance(payload, dict):
        problems.append("collector raw frame payload missing or not dict")
        return problems
    problems.extend(_validate_bounded_raw_value(
        payload,
        "collector raw frame payload",
    ))
    if frame_type == "event":
        problems.extend(_validate_raw_event_payload(frame, payload))
    elif frame_type == "domain_delta":
        problems.extend(_validate_raw_domain_delta_payload(frame, payload))
    elif frame_type == "gap":
        problems.extend(_validate_raw_gap_payload(frame, payload))
    return problems


def _validate_raw_event_payload(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    _validate_payload_keys(
        problems,
        payload,
        allowed={"event_type", "key", "from", "to", "subject", "detail"},
        prefix="collector event payload",
    )
    event_type = payload.get("event_type")
    if not _valid_raw_string(event_type, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        problems.append("collector event payload event_type missing or not bounded str")
    key = payload.get("key")
    if not _valid_raw_key(key):
        problems.append("collector event payload key missing or not stable str")
    elif _valid_raw_key(frame.get("entity_key")) and key != frame.get("entity_key"):
        problems.append("collector event payload key != entity_key")
    subject = payload.get("subject")
    if subject is not None and not _valid_raw_string(
        subject,
        max_length=RAW_SHORT_STRING_MAX_LENGTH,
    ):
        problems.append("collector event payload subject not bounded str|null")
    detail = payload.get("detail")
    if detail is not None and not isinstance(detail, dict):
        problems.append("collector event payload detail not object|null")
    if frame.get("domain") == "divisions":
        problems.extend(_validate_raw_division_event(frame, payload))
    if frame.get("domain") == "land_combats":
        problems.extend(_validate_raw_land_combat_event(frame, payload))
    if frame.get("domain") == "army_orders":
        problems.extend(_validate_raw_army_order_event(frame, payload))
    if frame.get("domain") == "naval_air_combats":
        problems.extend(_validate_raw_naval_combat_event(frame, payload))
    if frame.get("domain") == "air_wings":
        problems.extend(_validate_raw_air_wing_event(frame, payload))
    return problems


def _validate_raw_division_event(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    event_type = payload.get("event_type")
    if event_type not in RAW_DIVISION_EVENT_TYPES:
        problems.append(f"collector division event_type {event_type!r} invalid")
    entity_uid = _parse_raw_division_entity_key(frame.get("entity_key"))
    if entity_uid is None:
        problems.append(
            "collector division event entity_key not stable division key"
        )
    subject = payload.get("subject")
    if subject != "division":
        problems.append("collector division event subject != 'division'")
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector division event payload contains raw pointer/tag-db "
            "diagnostic key"
        )
    detail = payload.get("detail")
    if not isinstance(detail, dict):
        problems.append("collector division event detail missing or not object")
        return problems
    for key in detail:
        if key not in RAW_DIVISION_EVENT_DETAIL_FIELDS:
            problems.append(f"collector division event detail.{key} unsupported")
    row = {
        key: value
        for key, value in detail.items()
        if key in RAW_DIVISION_MOVEMENT_FIELDS
    }
    problems.extend(_validate_raw_division_movement_row(
        row,
        "collector division event detail",
        entity_uid,
    ))
    changed_fields = detail.get("changed_fields")
    if not isinstance(changed_fields, list):
        problems.append("collector division event detail.changed_fields not list")
    else:
        if len(changed_fields) > RAW_CONTAINER_MAX_ENTRIES:
            problems.append(
                "collector division event detail.changed_fields too many entries"
            )
        for i, field in enumerate(changed_fields):
            if field not in RAW_DIVISION_EVENT_DETAIL_FIELDS:
                problems.append(
                    f"collector division event detail.changed_fields[{i}] invalid"
                )
    return problems


def _validate_raw_land_combat_event(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    event_type = payload.get("event_type")
    if event_type not in RAW_LAND_COMBAT_EVENT_TYPES:
        problems.append(f"collector land combat event_type {event_type!r} invalid")
    entity_uid = _parse_raw_land_combat_entity_key(frame.get("entity_key"))
    if entity_uid is None:
        problems.append(
            "collector land combat event entity_key not stable land_combat key"
        )
    subject = payload.get("subject")
    if subject != "land_combat":
        problems.append("collector land combat event subject != 'land_combat'")
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector land combat event payload contains raw pointer/tag-db "
            "diagnostic key"
        )
    detail = payload.get("detail")
    if not isinstance(detail, dict):
        problems.append("collector land combat event detail missing or not object")
        return problems
    problems.extend(_validate_raw_land_combat_event_detail(
        detail,
        "collector land combat event detail",
        entity_uid,
        event_type if isinstance(event_type, str) else None,
    ))
    return problems


def _validate_raw_land_combat_event_detail(
    detail: Dict[str, Any],
    prefix: str,
    entity_uid: Optional[tuple[int, int]],
    event_type: Optional[str],
) -> List[str]:
    problems: List[str] = []
    for key in detail:
        if key not in RAW_LAND_COMBAT_EVENT_DETAIL_FIELDS:
            problems.append(f"{prefix}.{key} unsupported")
    row = {
        key: value
        for key, value in detail.items()
        if key in RAW_LAND_COMBAT_FIELDS
    }
    problems.extend(_validate_raw_land_combat_row(
        row,
        prefix,
        entity_uid,
        require_identity=True,
    ))
    changed_fields = detail.get("changed_fields")
    if not isinstance(changed_fields, list):
        problems.append(f"{prefix}.changed_fields not list")
    else:
        if len(changed_fields) > RAW_CONTAINER_MAX_ENTRIES:
            problems.append(f"{prefix}.changed_fields too many entries")
        for i, field in enumerate(changed_fields):
            if field not in RAW_LAND_COMBAT_EVENT_DETAIL_FIELDS:
                problems.append(f"{prefix}.changed_fields[{i}] invalid")
    end_reason = detail.get("end_reason")
    if event_type == "land_combat_ended":
        if end_reason not in RAW_LAND_COMBAT_END_REASONS:
            problems.append(f"{prefix}.end_reason invalid")
    elif end_reason is not None:
        problems.append(f"{prefix}.end_reason set for non-ended event")
    return problems


def _validate_raw_domain_delta_payload(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    _validate_payload_keys(
        problems,
        payload,
        allowed={"op", "before", "after", "changed_fields"},
        prefix="collector domain_delta payload",
    )
    op = payload.get("op")
    if op not in RAW_DOMAIN_DELTA_OPS:
        problems.append(f"collector domain_delta payload op {op!r} invalid")
    if "after" not in payload:
        problems.append("collector domain_delta payload after missing")
    changed_fields = payload.get("changed_fields")
    if not isinstance(changed_fields, list):
        problems.append("collector domain_delta payload changed_fields not list")
    else:
        if len(changed_fields) > RAW_CONTAINER_MAX_ENTRIES:
            problems.append("collector domain_delta payload changed_fields too many entries")
        for i, field in enumerate(changed_fields):
            if not _valid_raw_string(field, max_length=RAW_SHORT_STRING_MAX_LENGTH):
                problems.append(
                    "collector domain_delta payload "
                    f"changed_fields[{i}] not bounded str"
                )
    if _looks_like_division_movement_delta(frame, payload):
        problems.extend(_validate_raw_division_movement_delta(frame, payload))
    if frame.get("domain") == "land_combats":
        problems.extend(_validate_raw_land_combat_delta(frame, payload))
    if frame.get("domain") == "army_orders":
        problems.extend(_validate_raw_army_order_delta(frame, payload))
    if frame.get("domain") == "naval_air_combats":
        problems.extend(_validate_raw_naval_combat_delta(frame, payload))
    if frame.get("domain") == "naval_combat_members":
        problems.extend(_validate_raw_naval_combat_member_delta(frame, payload))
    if frame.get("domain") == "air_wings":
        problems.extend(_validate_raw_air_wing_delta(frame, payload))
    if frame.get("domain") == "ships":
        problems.extend(_validate_raw_ship_delta(frame, payload))
    if frame.get("domain") == "oob_command":
        problems.extend(_validate_raw_oob_command_delta(frame, payload))
    return problems


def _looks_like_division_movement_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> bool:
    if frame.get("domain") != "divisions":
        return False
    after = payload.get("after")
    return isinstance(after, dict) and any(
        key in RAW_DIVISION_MOVEMENT_FIELDS for key in after
    )


def _validate_raw_division_movement_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    entity_uid = _parse_raw_division_entity_key(frame.get("entity_key"))
    if entity_uid is None:
        problems.append(
            "collector division movement delta entity_key not stable division key"
        )
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector division movement delta payload contains raw pointer/tag-db "
            "diagnostic key"
        )
    changed_fields = payload.get("changed_fields")
    if isinstance(changed_fields, list):
        for i, field in enumerate(changed_fields):
            if _forbidden_raw_key_name(field):
                problems.append(
                    "collector division movement delta "
                    f"changed_fields[{i}] raw pointer/tag-db diagnostic key"
                )
    for row_key in ("before", "after"):
        row = payload.get(row_key)
        if isinstance(row, dict):
            problems.extend(_validate_raw_division_movement_row(
                row,
                f"collector division movement delta {row_key}",
                entity_uid,
            ))
    return problems


def _validate_raw_division_movement_row(
    row: Dict[str, Any],
    prefix: str,
    entity_uid: Optional[tuple[int, int]],
) -> List[str]:
    problems: List[str] = []
    uid = row.get("div_uid")
    if uid is not None:
        parsed_uid = _parse_raw_division_uid(uid)
        if parsed_uid is None:
            problems.append(f"{prefix}.div_uid not stable id:type str|null")
        elif entity_uid is not None and parsed_uid != entity_uid:
            problems.append(f"{prefix}.div_uid != entity_key")
    for key in ("div_id", "div_type"):
        value = row.get(key)
        if value is not None and not _valid_bounded_int(
            value,
            minimum=0,
            maximum=RAW_DIVISION_ID_MAX,
        ):
            problems.append(f"{prefix}.{key} not nonnegative int|null")
    if entity_uid is not None:
        div_id = row.get("div_id")
        if (
            div_id is not None
            and _valid_bounded_int(div_id, minimum=0, maximum=RAW_DIVISION_ID_MAX)
            and div_id != entity_uid[0]
        ):
            problems.append(f"{prefix}.div_id != entity_key")
        div_type = row.get("div_type")
        if (
            div_type is not None
            and _valid_bounded_int(div_type, minimum=0, maximum=RAW_DIVISION_ID_MAX)
            and div_type != entity_uid[1]
        ):
            problems.append(f"{prefix}.div_type != entity_key")
    moving = row.get("moving")
    if moving is not None and not isinstance(moving, bool):
        problems.append(f"{prefix}.moving not bool|null")
    for key in ("movement_source_province", "move_target_province", "location"):
        value = row.get(key)
        if value is not None and not _valid_bounded_int(
            value,
            minimum=1,
            maximum=RAW_DIVISION_PROVINCE_ID_MAX,
        ):
            problems.append(f"{prefix}.{key} not positive int|null")
    progress = row.get("movement_progress")
    if progress is not None:
        if not (
            _is_finite_number(progress)
            and 0.0 <= float(progress) <= RAW_DIVISION_MOVEMENT_PROGRESS_MAX
        ):
            problems.append(
                f"{prefix}.movement_progress not finite nonnegative number|null "
                "in range"
            )
    progress_bucket = row.get("progress_bucket")
    if progress_bucket is not None and not _valid_bounded_int(
        progress_bucket,
        minimum=0,
        maximum=RAW_DIVISION_PROGRESS_BUCKET_MAX,
    ):
        problems.append(f"{prefix}.progress_bucket not nonnegative int|null in range")
    owner = row.get("owner")
    if owner is not None and not schema.valid_tag(owner):
        problems.append(f"{prefix}.owner not TAG|null")
    organisation = row.get("organisation")
    if organisation is not None:
        if not (
            _is_finite_number(organisation)
            and 0.0 <= float(organisation) <= RAW_DIVISION_ORGANISATION_MAX
        ):
            problems.append(f"{prefix}.organisation not finite number|null in range")
    strength = row.get("strength")
    if strength is not None:
        if not (
            _is_finite_number(strength)
            and 0.0 <= float(strength) <= RAW_DIVISION_STRENGTH_MAX
        ):
            problems.append(f"{prefix}.strength not finite number|null in range")
    combat_ref_count = row.get("combat_ref_count")
    if combat_ref_count is not None and not _valid_bounded_int(
        combat_ref_count,
        minimum=0,
        maximum=RAW_DIVISION_COMBAT_REF_COUNT_MAX,
    ):
        problems.append(
            f"{prefix}.combat_ref_count not nonnegative int|null in range"
        )
    in_combat = row.get("in_combat")
    if in_combat is not None and not isinstance(in_combat, bool):
        problems.append(f"{prefix}.in_combat not bool|null")
    return problems


def _validate_raw_land_combat_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    if _is_raw_land_combat_gap_delta(frame, payload):
        return _validate_raw_land_combat_gap_delta(frame, payload)

    entity_uid = _parse_raw_land_combat_entity_key(frame.get("entity_key"))
    if entity_uid is None:
        problems.append(
            "collector land combat delta entity_key not stable land_combat key"
        )
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector land combat delta payload contains raw pointer/tag-db "
            "diagnostic key"
        )
    changed_fields = payload.get("changed_fields")
    if isinstance(changed_fields, list):
        for i, field in enumerate(changed_fields):
            if _forbidden_raw_key_name(field):
                problems.append(
                    "collector land combat delta "
                    f"changed_fields[{i}] raw pointer/tag-db diagnostic key"
                )
    for row_key in ("before", "after"):
        row = payload.get(row_key)
        if isinstance(row, dict):
            problems.extend(_validate_raw_land_combat_row(
                row,
                f"collector land combat delta {row_key}",
                entity_uid,
                require_identity=(row_key == "after"),
            ))
    return problems


def _is_raw_land_combat_gap_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> bool:
    return (
        frame.get("confidence") == "gap"
        or _parse_raw_land_combat_gap_key(frame.get("entity_key")) is not None
        or (
            isinstance(payload.get("after"), dict)
            and "gap_kind" in payload["after"]
        )
    )


def _validate_raw_land_combat_gap_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    entity_key = frame.get("entity_key")
    gap_kind = _parse_raw_land_combat_gap_key(entity_key)
    if gap_kind is None:
        problems.append(
            "collector land combat gap entity_key not stable land_combats gap key"
        )
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector land combat gap payload contains raw pointer/tag-db "
            "diagnostic key"
        )
    changed_fields = payload.get("changed_fields")
    if isinstance(changed_fields, list):
        for i, field in enumerate(changed_fields):
            if _forbidden_raw_key_name(field):
                problems.append(
                    "collector land combat gap "
                    f"changed_fields[{i}] raw pointer/tag-db diagnostic key"
                )
    for row_key in ("before", "after"):
        row = payload.get(row_key)
        if isinstance(row, dict):
            problems.extend(_validate_raw_land_combat_gap_row(
                row,
                f"collector land combat gap {row_key}",
                entity_key if isinstance(entity_key, str) else None,
                gap_kind,
            ))
    return problems


def _validate_raw_land_combat_gap_row(
    row: Dict[str, Any],
    prefix: str,
    entity_key: Optional[str],
    entity_gap_kind: Optional[str],
) -> List[str]:
    problems: List[str] = []
    for key in row:
        if key not in RAW_LAND_COMBAT_GAP_FIELDS:
            problems.append(f"{prefix}.{key} unsupported")

    gap_key = row.get("gap_key")
    if not _valid_raw_key(gap_key):
        problems.append(f"{prefix}.gap_key missing or not stable str")
    elif entity_key is not None and gap_key != entity_key:
        problems.append(f"{prefix}.gap_key != entity_key")

    gap_kind = row.get("gap_kind")
    if not _valid_raw_string(gap_kind, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        problems.append(f"{prefix}.gap_kind missing or not bounded str")
    elif _valid_hex_pointer(gap_kind):
        problems.append(f"{prefix}.gap_kind looks like pointer")
    elif entity_gap_kind is not None and gap_kind != entity_gap_kind:
        problems.append(f"{prefix}.gap_kind != entity_key")

    status = row.get("status")
    if status not in ("validation_failed", "unsupported", "layout_invalid"):
        problems.append(f"{prefix}.status invalid")
    return problems


def _validate_raw_land_combat_row(
    row: Dict[str, Any],
    prefix: str,
    entity_uid: Optional[tuple[int, int]],
    *,
    require_identity: bool,
) -> List[str]:
    problems: List[str] = []
    for key in row:
        if key not in RAW_LAND_COMBAT_FIELDS:
            problems.append(f"{prefix}.{key} unsupported")

    combat_key = row.get("combat_key")
    if combat_key is not None:
        parsed_key = _parse_raw_land_combat_entity_key(combat_key)
        if parsed_key is None:
            problems.append(f"{prefix}.combat_key not stable land_combat key|null")
        elif entity_uid is not None and parsed_key != entity_uid:
            problems.append(f"{prefix}.combat_key != entity_key")
    elif require_identity:
        problems.append(f"{prefix}.combat_key missing")

    combat_id = row.get("combat_id")
    generation = row.get("generation")
    if combat_id is None:
        if require_identity:
            problems.append(f"{prefix}.combat_id missing")
    elif not _valid_bounded_int(
        combat_id,
        minimum=1,
        maximum=RAW_LAND_COMBAT_ID_MAX,
    ):
        problems.append(f"{prefix}.combat_id not positive int|null in range")
    elif entity_uid is not None and combat_id != entity_uid[0]:
        problems.append(f"{prefix}.combat_id != entity_key")

    if generation is None:
        if require_identity:
            problems.append(f"{prefix}.generation missing")
    elif not _valid_bounded_int(
        generation,
        minimum=0,
        maximum=RAW_LAND_COMBAT_GENERATION_MAX,
    ):
        problems.append(f"{prefix}.generation not nonnegative int|null in range")
    elif entity_uid is not None and generation != entity_uid[1]:
        problems.append(f"{prefix}.generation != entity_key")

    location = row.get("location")
    if location is not None and not _valid_bounded_int(
        location,
        minimum=1,
        maximum=RAW_LAND_COMBAT_PROVINCE_ID_MAX,
    ):
        problems.append(f"{prefix}.location not positive int|null in range")
    for key in ("day", "duration"):
        value = row.get(key)
        if value is not None and not _valid_bounded_int(
            value,
            minimum=0,
            maximum=RAW_LAND_COMBAT_AGE_DAYS_MAX,
        ):
            problems.append(f"{prefix}.{key} not nonnegative int|null in range")
    for key in ("attacker_divisions", "defender_divisions"):
        problems.extend(_validate_raw_land_combat_participants(
            row.get(key),
            f"{prefix}.{key}",
        ))
    for key in ("attacker_status", "defender_status"):
        value = row.get(key)
        if value is not None and value not in RAW_LAND_COMBAT_SIDE_STATUSES:
            problems.append(f"{prefix}.{key} invalid")
    return problems


def _validate_raw_land_combat_participants(
    value: Any,
    prefix: str,
) -> List[str]:
    if value is None:
        return []
    if not isinstance(value, list):
        return [f"{prefix} not list|null"]
    problems: List[str] = []
    if len(value) > RAW_LAND_COMBAT_PARTICIPANTS_MAX:
        problems.append(f"{prefix} too many entries")
    seen: set[int] = set()
    for i, participant in enumerate(value):
        if not _valid_bounded_int(
            participant,
            minimum=1,
            maximum=RAW_DIVISION_ID_MAX,
        ):
            problems.append(f"{prefix}[{i}] not positive division id")
            continue
        if participant in seen:
            problems.append(f"{prefix}[{i}] duplicate division id")
        seen.add(participant)
    return problems


def _validate_raw_army_order_event(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    event_type = payload.get("event_type")
    if event_type not in RAW_ARMY_ORDER_EVENT_TYPES:
        problems.append(f"collector army order event_type {event_type!r} invalid")
    entity_uid = _parse_raw_army_order_entity_key(frame.get("entity_key"))
    if entity_uid is None:
        problems.append("collector army order event entity_key not stable army_order key")
    subject = payload.get("subject")
    if subject != "army_order":
        problems.append("collector army order event subject != 'army_order'")
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector army order event payload contains raw pointer/tag-db "
            "diagnostic key"
        )
    detail = payload.get("detail")
    if not isinstance(detail, dict):
        problems.append("collector army order event detail missing or not object")
        return problems
    problems.extend(_validate_raw_army_order_event_detail(
        detail,
        "collector army order event detail",
        entity_uid,
    ))
    return problems


def _validate_raw_army_order_event_detail(
    detail: Dict[str, Any],
    prefix: str,
    entity_uid: Optional[tuple[int, int]],
) -> List[str]:
    problems: List[str] = []
    for key in detail:
        if key not in RAW_ARMY_ORDER_EVENT_DETAIL_FIELDS:
            problems.append(f"{prefix}.{key} unsupported")
    row = {
        key: value
        for key, value in detail.items()
        if key in RAW_ARMY_ORDER_FIELDS
    }
    problems.extend(_validate_raw_army_order_row(
        row,
        prefix,
        entity_uid,
        require_identity=True,
    ))
    changed_fields = detail.get("changed_fields")
    if not isinstance(changed_fields, list):
        problems.append(f"{prefix}.changed_fields not list")
    else:
        if len(changed_fields) > RAW_CONTAINER_MAX_ENTRIES:
            problems.append(f"{prefix}.changed_fields too many entries")
        for i, field in enumerate(changed_fields):
            if field not in RAW_ARMY_ORDER_EVENT_DETAIL_FIELDS:
                problems.append(f"{prefix}.changed_fields[{i}] invalid")
    return problems


def _validate_raw_army_order_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    if _is_raw_army_order_gap_delta(frame, payload):
        return _validate_raw_army_order_gap_delta(frame, payload)

    entity_uid = _parse_raw_army_order_entity_key(frame.get("entity_key"))
    if entity_uid is None:
        problems.append("collector army order delta entity_key not stable army_order key")
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector army order delta payload contains raw pointer/tag-db "
            "diagnostic key"
        )
    changed_fields = payload.get("changed_fields")
    if isinstance(changed_fields, list):
        for i, field in enumerate(changed_fields):
            if _forbidden_raw_key_name(field):
                problems.append(
                    "collector army order delta "
                    f"changed_fields[{i}] raw pointer/tag-db diagnostic key"
                )
    for row_key in ("before", "after"):
        row = payload.get(row_key)
        if isinstance(row, dict):
            problems.extend(_validate_raw_army_order_row(
                row,
                f"collector army order delta {row_key}",
                entity_uid,
                require_identity=(row_key == "after"),
            ))
    return problems


def _is_raw_army_order_gap_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> bool:
    return (
        frame.get("confidence") == "gap"
        or _parse_raw_army_order_gap_key(frame.get("entity_key")) is not None
        or (
            isinstance(payload.get("after"), dict)
            and "gap_kind" in payload["after"]
        )
    )


def _validate_raw_army_order_gap_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    entity_key = frame.get("entity_key")
    gap_kind = _parse_raw_army_order_gap_key(entity_key)
    if gap_kind is None:
        problems.append(
            "collector army order gap entity_key not stable army_orders gap key"
        )
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector army order gap payload contains raw pointer/tag-db "
            "diagnostic key"
        )
    changed_fields = payload.get("changed_fields")
    if isinstance(changed_fields, list):
        for i, field in enumerate(changed_fields):
            if _forbidden_raw_key_name(field):
                problems.append(
                    "collector army order gap "
                    f"changed_fields[{i}] raw pointer/tag-db diagnostic key"
                )
    for row_key in ("before", "after"):
        row = payload.get(row_key)
        if isinstance(row, dict):
            problems.extend(_validate_raw_army_order_gap_row(
                row,
                f"collector army order gap {row_key}",
                entity_key if isinstance(entity_key, str) else None,
                gap_kind,
            ))
    return problems


def _validate_raw_army_order_gap_row(
    row: Dict[str, Any],
    prefix: str,
    entity_key: Optional[str],
    entity_gap_kind: Optional[str],
) -> List[str]:
    problems: List[str] = []
    for key in row:
        if key not in RAW_ARMY_ORDER_GAP_FIELDS:
            problems.append(f"{prefix}.{key} unsupported")

    gap_key = row.get("gap_key")
    if not _valid_raw_key(gap_key):
        problems.append(f"{prefix}.gap_key missing or not stable str")
    elif entity_key is not None and gap_key != entity_key:
        problems.append(f"{prefix}.gap_key != entity_key")

    gap_kind = row.get("gap_kind")
    if not _valid_raw_string(gap_kind, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        problems.append(f"{prefix}.gap_kind missing or not bounded str")
    elif _valid_hex_pointer(gap_kind):
        problems.append(f"{prefix}.gap_kind looks like pointer")
    elif entity_gap_kind is not None and gap_kind != entity_gap_kind:
        problems.append(f"{prefix}.gap_kind != entity_key")

    status = row.get("status")
    if status not in ("validation_failed", "unsupported", "layout_invalid"):
        problems.append(f"{prefix}.status invalid")
    return problems


def _validate_raw_army_order_row(
    row: Dict[str, Any],
    prefix: str,
    entity_uid: Optional[tuple[int, int]],
    *,
    require_identity: bool,
) -> List[str]:
    problems: List[str] = []
    for key in row:
        if key not in RAW_ARMY_ORDER_FIELDS:
            problems.append(f"{prefix}.{key} unsupported")

    order_key = row.get("order_key")
    if order_key is not None:
        parsed_key = _parse_raw_army_order_entity_key(order_key)
        if parsed_key is None:
            problems.append(f"{prefix}.order_key not stable army_order key|null")
        elif entity_uid is not None and parsed_key != entity_uid:
            problems.append(f"{prefix}.order_key != entity_key")
    elif require_identity:
        problems.append(f"{prefix}.order_key missing")

    army_id = row.get("army_id")
    generation = row.get("generation")
    if army_id is None:
        if require_identity:
            problems.append(f"{prefix}.army_id missing")
    elif not _valid_bounded_int(
        army_id,
        minimum=1,
        maximum=RAW_ARMY_ORDER_ID_MAX,
    ):
        problems.append(f"{prefix}.army_id not positive int|null in range")
    elif entity_uid is not None and army_id != entity_uid[0]:
        problems.append(f"{prefix}.army_id != entity_key")

    if generation is None:
        if require_identity:
            problems.append(f"{prefix}.generation missing")
    elif not _valid_bounded_int(
        generation,
        minimum=0,
        maximum=RAW_ARMY_ORDER_GENERATION_MAX,
    ):
        problems.append(f"{prefix}.generation not nonnegative int|null in range")
    elif entity_uid is not None and generation != entity_uid[1]:
        problems.append(f"{prefix}.generation != entity_key")

    slot_index = row.get("slot_index")
    if slot_index is not None and not _valid_bounded_int(
        slot_index,
        minimum=0,
        maximum=RAW_ARMY_ORDER_SLOT_INDEX_MAX,
    ):
        problems.append(f"{prefix}.slot_index not nonnegative int|null in range")

    order_type = row.get("order_type")
    if order_type is not None and (
        isinstance(order_type, bool)
        or not isinstance(order_type, int)
        or abs(order_type) > RAW_ARMY_ORDER_TYPE_ABS_MAX
    ):
        problems.append(f"{prefix}.order_type not bounded int|null")

    target_states = row.get("target_states")
    problems.extend(_validate_raw_army_order_target_states(
        target_states,
        f"{prefix}.target_states",
    ))

    target_state_count = row.get("target_state_count")
    if target_state_count is not None and not _valid_bounded_int(
        target_state_count,
        minimum=0,
        maximum=RAW_ARMY_ORDER_TARGET_STATE_COUNT_MAX,
    ):
        problems.append(f"{prefix}.target_state_count not nonnegative int|null in range")
    elif isinstance(target_states, list) and isinstance(target_state_count, int):
        if target_state_count != len(target_states):
            problems.append(f"{prefix}.target_state_count != target_states length")

    target_status = row.get("target_status")
    if target_status is not None and target_status not in RAW_ARMY_ORDER_TARGET_STATUSES:
        problems.append(f"{prefix}.target_status invalid")
    return problems


def _validate_raw_army_order_target_states(
    value: Any,
    prefix: str,
) -> List[str]:
    if value is None:
        return []
    if not isinstance(value, list):
        return [f"{prefix} not list|null"]
    problems: List[str] = []
    if len(value) > RAW_ARMY_ORDER_TARGET_STATES_MAX:
        problems.append(f"{prefix} too many entries")
    seen: set[int] = set()
    for i, state_id in enumerate(value):
        if not _valid_bounded_int(
            state_id,
            minimum=1,
            maximum=RAW_ARMY_ORDER_TARGET_STATE_ID_MAX,
        ):
            problems.append(f"{prefix}[{i}] not positive state id")
            continue
        if state_id in seen:
            problems.append(f"{prefix}[{i}] duplicate state id")
        seen.add(state_id)
    return problems


def _validate_raw_naval_combat_event(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    event_type = payload.get("event_type")
    if event_type not in RAW_NAVAL_COMBAT_EVENT_TYPES:
        problems.append(f"collector naval combat event_type {event_type!r} invalid")
    entity_uid = _parse_raw_naval_combat_entity_key(frame.get("entity_key"))
    if entity_uid is None:
        problems.append(
            "collector naval combat event entity_key not stable naval_combat key"
        )
    subject = payload.get("subject")
    if subject != "naval_combat":
        problems.append("collector naval combat event subject != 'naval_combat'")
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector naval combat event payload contains raw pointer/tag-db "
            "diagnostic key"
        )
    detail = payload.get("detail")
    if not isinstance(detail, dict):
        problems.append("collector naval combat event detail missing or not object")
        return problems
    problems.extend(_validate_raw_naval_combat_event_detail(
        detail,
        "collector naval combat event detail",
        entity_uid,
        event_type if isinstance(event_type, str) else None,
    ))
    return problems


def _validate_raw_naval_combat_event_detail(
    detail: Dict[str, Any],
    prefix: str,
    entity_uid: Optional[tuple[int, int]],
    event_type: Optional[str],
) -> List[str]:
    problems: List[str] = []
    for key in detail:
        if key not in RAW_NAVAL_COMBAT_EVENT_DETAIL_FIELDS:
            problems.append(f"{prefix}.{key} unsupported")
    row = {
        key: value
        for key, value in detail.items()
        if key in RAW_NAVAL_COMBAT_FIELDS
    }
    problems.extend(_validate_raw_naval_combat_row(
        row,
        prefix,
        entity_uid,
        require_identity=True,
    ))
    changed_fields = detail.get("changed_fields")
    if not isinstance(changed_fields, list):
        problems.append(f"{prefix}.changed_fields not list")
    else:
        if len(changed_fields) > RAW_CONTAINER_MAX_ENTRIES:
            problems.append(f"{prefix}.changed_fields too many entries")
        for i, field in enumerate(changed_fields):
            if field not in RAW_NAVAL_COMBAT_EVENT_DETAIL_FIELDS:
                problems.append(f"{prefix}.changed_fields[{i}] invalid")
    end_reason = detail.get("end_reason")
    if event_type == "naval_combat_ended":
        if end_reason not in RAW_NAVAL_COMBAT_END_REASONS:
            problems.append(f"{prefix}.end_reason invalid")
    elif end_reason is not None:
        problems.append(f"{prefix}.end_reason set for non-ended event")
    return problems


def _validate_raw_naval_combat_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    if _is_raw_naval_combat_gap_delta(frame, payload):
        return _validate_raw_naval_combat_gap_delta(frame, payload)

    entity_uid = _parse_raw_naval_combat_entity_key(frame.get("entity_key"))
    if entity_uid is None:
        problems.append(
            "collector naval combat delta entity_key not stable naval_combat key"
        )
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector naval combat delta payload contains raw pointer/tag-db "
            "diagnostic key"
        )
    changed_fields = payload.get("changed_fields")
    if isinstance(changed_fields, list):
        for i, field in enumerate(changed_fields):
            if _forbidden_raw_key_name(field):
                problems.append(
                    "collector naval combat delta "
                    f"changed_fields[{i}] raw pointer/tag-db diagnostic key"
                )
    for row_key in ("before", "after"):
        row = payload.get(row_key)
        if isinstance(row, dict):
            problems.extend(_validate_raw_naval_combat_row(
                row,
                f"collector naval combat delta {row_key}",
                entity_uid,
                require_identity=(row_key == "after"),
            ))
    return problems


def _is_raw_naval_combat_gap_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> bool:
    return (
        frame.get("confidence") == "gap"
        or _parse_raw_naval_combat_gap_key(frame.get("entity_key")) is not None
        or (
            isinstance(payload.get("after"), dict)
            and "gap_kind" in payload["after"]
        )
    )


def _validate_raw_naval_combat_gap_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    entity_key = frame.get("entity_key")
    gap_kind = _parse_raw_naval_combat_gap_key(entity_key)
    if gap_kind is None:
        problems.append(
            "collector naval combat gap entity_key not stable naval_air_combats gap key"
        )
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector naval combat gap payload contains raw pointer/tag-db "
            "diagnostic key"
        )
    changed_fields = payload.get("changed_fields")
    if isinstance(changed_fields, list):
        for i, field in enumerate(changed_fields):
            if _forbidden_raw_key_name(field):
                problems.append(
                    "collector naval combat gap "
                    f"changed_fields[{i}] raw pointer/tag-db diagnostic key"
                )
    for row_key in ("before", "after"):
        row = payload.get(row_key)
        if isinstance(row, dict):
            problems.extend(_validate_raw_naval_combat_gap_row(
                row,
                f"collector naval combat gap {row_key}",
                entity_key if isinstance(entity_key, str) else None,
                gap_kind,
            ))
    return problems


def _validate_raw_naval_combat_gap_row(
    row: Dict[str, Any],
    prefix: str,
    entity_key: Optional[str],
    entity_gap_kind: Optional[str],
) -> List[str]:
    problems: List[str] = []
    for key in row:
        if key not in RAW_NAVAL_COMBAT_GAP_FIELDS:
            problems.append(f"{prefix}.{key} unsupported")

    gap_key = row.get("gap_key")
    if not _valid_raw_key(gap_key):
        problems.append(f"{prefix}.gap_key missing or not stable str")
    elif entity_key is not None and gap_key != entity_key:
        problems.append(f"{prefix}.gap_key != entity_key")

    gap_kind = row.get("gap_kind")
    if not _valid_raw_string(gap_kind, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        problems.append(f"{prefix}.gap_kind missing or not bounded str")
    elif _valid_hex_pointer(gap_kind):
        problems.append(f"{prefix}.gap_kind looks like pointer")
    elif entity_gap_kind is not None and gap_kind != entity_gap_kind:
        problems.append(f"{prefix}.gap_kind != entity_key")

    status = row.get("status")
    if status not in ("validation_failed", "unsupported", "layout_invalid"):
        problems.append(f"{prefix}.status invalid")
    return problems


def _validate_raw_naval_combat_row(
    row: Dict[str, Any],
    prefix: str,
    entity_uid: Optional[tuple[int, int]],
    *,
    require_identity: bool,
) -> List[str]:
    problems: List[str] = []
    for key in row:
        if key not in RAW_NAVAL_COMBAT_FIELDS:
            problems.append(f"{prefix}.{key} unsupported")

    combat_key = row.get("combat_key")
    if combat_key is not None:
        parsed_key = _parse_raw_naval_combat_entity_key(combat_key)
        if parsed_key is None:
            problems.append(f"{prefix}.combat_key not stable naval_combat key|null")
        elif entity_uid is not None and parsed_key != entity_uid:
            problems.append(f"{prefix}.combat_key != entity_key")
    elif require_identity:
        problems.append(f"{prefix}.combat_key missing")

    combat_id = row.get("combat_id")
    generation = row.get("generation")
    if combat_id is None:
        if require_identity:
            problems.append(f"{prefix}.combat_id missing")
    elif not _valid_bounded_int(
        combat_id,
        minimum=1,
        maximum=RAW_NAVAL_COMBAT_ID_MAX,
    ):
        problems.append(f"{prefix}.combat_id not positive int|null in range")
    elif entity_uid is not None and combat_id != entity_uid[0]:
        problems.append(f"{prefix}.combat_id != entity_key")

    if generation is None:
        if require_identity:
            problems.append(f"{prefix}.generation missing")
    elif not _valid_bounded_int(
        generation,
        minimum=0,
        maximum=RAW_NAVAL_COMBAT_GENERATION_MAX,
    ):
        problems.append(f"{prefix}.generation not nonnegative int|null in range")
    elif entity_uid is not None and generation != entity_uid[1]:
        problems.append(f"{prefix}.generation != entity_key")

    location = row.get("location")
    if location is not None and not _valid_bounded_int(
        location,
        minimum=1,
        maximum=RAW_NAVAL_COMBAT_PROVINCE_ID_MAX,
    ):
        problems.append(f"{prefix}.location not positive int|null in range")
    for key in ("attacker_tags", "defender_tags"):
        problems.extend(_validate_raw_naval_combat_tags(
            row.get(key),
            f"{prefix}.{key}",
        ))
    for key in ("attacker_ship_count", "defender_ship_count"):
        value = row.get(key)
        if value is not None and not _valid_bounded_int(
            value,
            minimum=0,
            maximum=RAW_NAVAL_COMBAT_SHIP_COUNT_MAX,
        ):
            problems.append(f"{prefix}.{key} not nonnegative int|null in range")
    for key in ("attacker_commander", "defender_commander"):
        value = row.get(key)
        if value is not None and not _valid_raw_string(
            value,
            max_length=RAW_SHORT_STRING_MAX_LENGTH,
        ):
            problems.append(f"{prefix}.{key} not bounded str|null")
    for key in ("attacker_positioning", "defender_positioning"):
        value = row.get(key)
        if value is not None and (
            not _is_finite_number(value)
            or value < 0
            or value > RAW_NAVAL_COMBAT_POSITIONING_MAX
        ):
            problems.append(f"{prefix}.{key} not finite number|null in range")
    for key in ("attacker_status", "defender_status"):
        value = row.get(key)
        if value is not None and value not in RAW_NAVAL_COMBAT_SIDE_STATUSES:
            problems.append(f"{prefix}.{key} invalid")
    return problems


def _validate_raw_naval_combat_tags(
    value: Any,
    prefix: str,
) -> List[str]:
    if value is None:
        return []
    if not isinstance(value, list):
        return [f"{prefix} not list|null"]
    problems: List[str] = []
    if len(value) > RAW_NAVAL_COMBAT_SIDE_TAGS_MAX:
        problems.append(f"{prefix} too many entries")
    seen: set[str] = set()
    for i, tag in enumerate(value):
        if not schema.valid_tag(tag):
            problems.append(f"{prefix}[{i}] not valid TAG")
            continue
        if tag in seen:
            problems.append(f"{prefix}[{i}] duplicate TAG")
        seen.add(tag)
    return problems


def _validate_raw_naval_combat_member_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    if _is_raw_naval_combat_member_gap_delta(frame, payload):
        return _validate_raw_naval_combat_member_gap_delta(frame, payload)

    entity_uid = _parse_raw_naval_combat_member_entity_key(frame.get("entity_key"))
    if entity_uid is None:
        problems.append(
            "collector naval combat member delta entity_key not stable "
            "naval_combat_members key"
        )
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector naval combat member delta payload contains raw pointer/tag-db "
            "diagnostic key"
        )
    changed_fields = payload.get("changed_fields")
    if isinstance(changed_fields, list):
        for i, field in enumerate(changed_fields):
            if _forbidden_raw_key_name(field):
                problems.append(
                    "collector naval combat member delta "
                    f"changed_fields[{i}] raw pointer/tag-db diagnostic key"
                )
            elif field not in RAW_NAVAL_COMBAT_MEMBER_FIELDS:
                problems.append(
                    "collector naval combat member delta "
                    f"changed_fields[{i}] invalid"
                )
    for row_key in ("before", "after"):
        row = payload.get(row_key)
        if isinstance(row, dict):
            problems.extend(_validate_raw_naval_combat_member_row(
                row,
                f"collector naval combat member delta {row_key}",
                entity_uid,
                require_identity=(row_key == "after"),
            ))
    return problems


def _is_raw_naval_combat_member_gap_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> bool:
    return (
        frame.get("confidence") == "gap"
        or _parse_raw_naval_combat_member_gap_key(frame.get("entity_key")) is not None
        or (
            isinstance(payload.get("after"), dict)
            and "gap_kind" in payload["after"]
        )
    )


def _validate_raw_naval_combat_member_gap_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    entity_key = frame.get("entity_key")
    gap_kind = _parse_raw_naval_combat_member_gap_key(entity_key)
    if gap_kind is None:
        problems.append(
            "collector naval combat member gap entity_key not stable "
            "naval_combat_members gap key"
        )
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector naval combat member gap payload contains raw pointer/tag-db "
            "diagnostic key"
        )
    changed_fields = payload.get("changed_fields")
    if isinstance(changed_fields, list):
        for i, field in enumerate(changed_fields):
            if _forbidden_raw_key_name(field):
                problems.append(
                    "collector naval combat member gap "
                    f"changed_fields[{i}] raw pointer/tag-db diagnostic key"
                )
            elif field not in RAW_NAVAL_COMBAT_MEMBER_GAP_FIELDS:
                problems.append(
                    f"collector naval combat member gap changed_fields[{i}] invalid"
                )
    for row_key in ("before", "after"):
        row = payload.get(row_key)
        if isinstance(row, dict):
            problems.extend(_validate_raw_naval_combat_member_gap_row(
                row,
                f"collector naval combat member gap {row_key}",
                entity_key if isinstance(entity_key, str) else None,
                gap_kind,
            ))
    return problems


def _validate_raw_naval_combat_member_gap_row(
    row: Dict[str, Any],
    prefix: str,
    entity_key: Optional[str],
    entity_gap_kind: Optional[str],
) -> List[str]:
    problems: List[str] = []
    for key in row:
        if key not in RAW_NAVAL_COMBAT_MEMBER_GAP_FIELDS:
            problems.append(f"{prefix}.{key} unsupported")

    gap_key = row.get("gap_key")
    if not _valid_raw_key(gap_key):
        problems.append(f"{prefix}.gap_key missing or not stable str")
    elif entity_key is not None and gap_key != entity_key:
        problems.append(f"{prefix}.gap_key != entity_key")

    gap_kind = row.get("gap_kind")
    if not _valid_raw_string(gap_kind, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        problems.append(f"{prefix}.gap_kind missing or not bounded str")
    elif _valid_hex_pointer(gap_kind):
        problems.append(f"{prefix}.gap_kind looks like pointer")
    elif entity_gap_kind is not None and gap_kind != entity_gap_kind:
        problems.append(f"{prefix}.gap_kind != entity_key")

    status = row.get("status")
    if status not in ("validation_failed", "unsupported", "layout_invalid"):
        problems.append(f"{prefix}.status invalid")
    return problems


def _validate_raw_naval_combat_member_row(
    row: Dict[str, Any],
    prefix: str,
    entity_uid: Optional[Dict[str, Any]],
    *,
    require_identity: bool,
) -> List[str]:
    problems: List[str] = []
    for key in row:
        if key not in RAW_NAVAL_COMBAT_MEMBER_FIELDS:
            problems.append(f"{prefix}.{key} unsupported")

    member_key = row.get("member_key")
    parsed_member_key = None
    if member_key is not None:
        parsed_member_key = _parse_raw_naval_combat_member_entity_key(member_key)
        if parsed_member_key is None:
            problems.append(
                f"{prefix}.member_key not stable naval_combat_members key|null"
            )
        elif entity_uid is not None and parsed_member_key != entity_uid:
            problems.append(f"{prefix}.member_key != entity_key")
    elif require_identity:
        problems.append(f"{prefix}.member_key missing")

    key_uid = parsed_member_key or entity_uid

    combat_key = row.get("combat_key")
    if combat_key is not None:
        combat_uid = _parse_raw_naval_combat_entity_key(combat_key)
        if combat_uid is None:
            problems.append(f"{prefix}.combat_key not stable naval_combat key|null")
        elif key_uid is not None and combat_uid != (
            key_uid["combat_id"],
            key_uid["generation"],
        ):
            problems.append(f"{prefix}.combat_key != member_key")
    elif require_identity:
        problems.append(f"{prefix}.combat_key missing")

    combat_id = row.get("combat_id")
    if combat_id is None:
        if require_identity:
            problems.append(f"{prefix}.combat_id missing")
    elif not _valid_bounded_int(
        combat_id,
        minimum=1,
        maximum=RAW_NAVAL_COMBAT_MEMBER_ID_MAX,
    ):
        problems.append(f"{prefix}.combat_id not positive int|null in range")
    elif key_uid is not None and combat_id != key_uid["combat_id"]:
        problems.append(f"{prefix}.combat_id != member_key")

    generation = row.get("generation")
    if generation is None:
        if require_identity:
            problems.append(f"{prefix}.generation missing")
    elif not _valid_bounded_int(
        generation,
        minimum=0,
        maximum=RAW_NAVAL_COMBAT_MEMBER_GENERATION_MAX,
    ):
        problems.append(f"{prefix}.generation not nonnegative int|null in range")
    elif key_uid is not None and generation != key_uid["generation"]:
        problems.append(f"{prefix}.generation != member_key")

    side = row.get("side")
    if side is None:
        if require_identity:
            problems.append(f"{prefix}.side missing")
    elif side not in RAW_NAVAL_COMBAT_MEMBER_SIDES:
        problems.append(f"{prefix}.side invalid")
    elif key_uid is not None and side != key_uid["side"]:
        problems.append(f"{prefix}.side != member_key")

    for key, maximum in (
        ("group_index", RAW_NAVAL_COMBAT_MEMBER_GROUP_INDEX_MAX),
        ("member_index", RAW_NAVAL_COMBAT_MEMBER_INDEX_MAX),
    ):
        value = row.get(key)
        if value is not None and not _valid_bounded_int(
            value,
            minimum=0,
            maximum=maximum,
        ):
            problems.append(f"{prefix}.{key} not nonnegative int|null in range")

    unit_type_tag = row.get("unit_type_tag")
    if unit_type_tag is not None and not _valid_bounded_int(
        unit_type_tag,
        minimum=0,
        maximum=RAW_NAVAL_COMBAT_MEMBER_UNIT_TYPE_MAX,
    ):
        problems.append(f"{prefix}.unit_type_tag not nonnegative int|null in range")

    ship_instance_id = row.get("ship_instance_id")
    if ship_instance_id is not None and not _valid_bounded_int(
        ship_instance_id,
        minimum=1,
        maximum=RAW_NAVAL_COMBAT_MEMBER_SHIP_INSTANCE_ID_MAX,
    ):
        problems.append(f"{prefix}.ship_instance_id not positive int|null in range")

    is_convoy = row.get("is_convoy")
    if is_convoy is not None and not isinstance(is_convoy, bool):
        problems.append(f"{prefix}.is_convoy not bool|null")

    if key_uid is not None:
        if key_uid["kind"] == "ship":
            if ship_instance_id is not None and ship_instance_id != key_uid["instance_id"]:
                problems.append(f"{prefix}.ship_instance_id != member_key")
            if unit_type_tag is not None and unit_type_tag != 51:
                problems.append(f"{prefix}.unit_type_tag invalid for ship")
            if is_convoy is not None and is_convoy:
                problems.append(f"{prefix}.is_convoy true for ship key")
        elif key_uid["kind"] == "convoy":
            if ship_instance_id is not None:
                problems.append(f"{prefix}.ship_instance_id set for convoy key")
            if unit_type_tag is not None and unit_type_tag != 0:
                problems.append(f"{prefix}.unit_type_tag invalid for convoy")
            if is_convoy is not None and not is_convoy:
                problems.append(f"{prefix}.is_convoy false for convoy key")
            for key in ("group_index", "member_index"):
                value = row.get(key)
                if isinstance(value, int) and value != key_uid[key]:
                    problems.append(f"{prefix}.{key} != member_key")
            slot = row.get("battle_slot_index_raw")
            if isinstance(slot, int) and slot != key_uid["battle_slot_index_raw"]:
                problems.append(f"{prefix}.battle_slot_index_raw != member_key")

    owner = row.get("owner")
    if owner is not None and not schema.valid_tag(owner):
        problems.append(f"{prefix}.owner not TAG|null")

    for key in ("name", "type_display", "archetype"):
        value = row.get(key)
        if value is not None and not _valid_raw_string(
            value,
            max_length=RAW_SHORT_STRING_MAX_LENGTH,
        ):
            problems.append(f"{prefix}.{key} not bounded str|null")

    designed_max_strength_raw = row.get("designed_max_strength_raw")
    if designed_max_strength_raw is not None and not _valid_bounded_int(
        designed_max_strength_raw,
        minimum=0,
        maximum=RAW_NAVAL_COMBAT_MEMBER_MAX_STRENGTH_RAW_MAX,
    ):
        problems.append(
            f"{prefix}.designed_max_strength_raw not nonnegative int|null in range"
        )

    battle_slot_index_raw = row.get("battle_slot_index_raw")
    if battle_slot_index_raw is None:
        if require_identity:
            problems.append(f"{prefix}.battle_slot_index_raw missing")
    elif not _valid_bounded_int(
        battle_slot_index_raw,
        minimum=0,
        maximum=RAW_NAVAL_COMBAT_MEMBER_SLOT_INDEX_MAX,
    ):
        problems.append(
            f"{prefix}.battle_slot_index_raw not nonnegative int|null in range"
        )

    state_bitmask = row.get("state_bitmask")
    if state_bitmask is not None and not _valid_bounded_int(
        state_bitmask,
        minimum=0,
        maximum=RAW_NAVAL_COMBAT_MEMBER_STATE_BITMASK_MAX,
    ):
        problems.append(f"{prefix}.state_bitmask not nonnegative int|null in range")

    engagement_weight_raw = row.get("engagement_weight_raw")
    if engagement_weight_raw is not None and not _valid_bounded_int(
        engagement_weight_raw,
        minimum=0,
        maximum=RAW_NAVAL_COMBAT_MEMBER_ENGAGEMENT_WEIGHT_RAW_MAX,
    ):
        problems.append(
            f"{prefix}.engagement_weight_raw not nonnegative int|null in range"
        )
    return problems


def _validate_raw_air_wing_event(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    event_type = payload.get("event_type")
    if event_type not in RAW_AIR_WING_EVENT_TYPES:
        problems.append(f"collector air wing event_type {event_type!r} invalid")
    entity_uid = _parse_raw_air_wing_entity_key(frame.get("entity_key"))
    if entity_uid is None:
        problems.append("collector air wing event entity_key not stable air_wing key")
    subject = payload.get("subject")
    if subject != "air_wing":
        problems.append("collector air wing event subject != 'air_wing'")
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector air wing event payload contains raw pointer/tag-db diagnostic key"
        )
    detail = payload.get("detail")
    if not isinstance(detail, dict):
        problems.append("collector air wing event detail missing or not object")
        return problems
    problems.extend(_validate_raw_air_wing_event_detail(
        detail,
        "collector air wing event detail",
        entity_uid,
    ))
    return problems


def _validate_raw_air_wing_event_detail(
    detail: Dict[str, Any],
    prefix: str,
    entity_uid: Optional[tuple[str, int, int]],
) -> List[str]:
    problems: List[str] = []
    for key in detail:
        if key not in RAW_AIR_WING_EVENT_DETAIL_FIELDS:
            problems.append(f"{prefix}.{key} unsupported")
    row = {
        key: value
        for key, value in detail.items()
        if key in RAW_AIR_WING_FIELDS
    }
    problems.extend(_validate_raw_air_wing_row(
        row,
        prefix,
        entity_uid,
        require_identity=True,
    ))
    changed_fields = detail.get("changed_fields")
    if not isinstance(changed_fields, list):
        problems.append(f"{prefix}.changed_fields not list")
    else:
        if len(changed_fields) > RAW_CONTAINER_MAX_ENTRIES:
            problems.append(f"{prefix}.changed_fields too many entries")
        for i, field in enumerate(changed_fields):
            if field not in RAW_AIR_WING_EVENT_DETAIL_FIELDS:
                problems.append(f"{prefix}.changed_fields[{i}] invalid")
    return problems


def _validate_raw_air_wing_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    if _is_raw_air_wing_gap_delta(frame, payload):
        return _validate_raw_air_wing_gap_delta(frame, payload)

    entity_uid = _parse_raw_air_wing_entity_key(frame.get("entity_key"))
    if entity_uid is None:
        problems.append("collector air wing delta entity_key not stable air_wing key")
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector air wing delta payload contains raw pointer/tag-db diagnostic key"
        )
    changed_fields = payload.get("changed_fields")
    if isinstance(changed_fields, list):
        for i, field in enumerate(changed_fields):
            if _forbidden_raw_key_name(field):
                problems.append(
                    "collector air wing delta "
                    f"changed_fields[{i}] raw pointer/tag-db diagnostic key"
                )
    for row_key in ("before", "after"):
        row = payload.get(row_key)
        if isinstance(row, dict):
            problems.extend(_validate_raw_air_wing_row(
                row,
                f"collector air wing delta {row_key}",
                entity_uid,
                require_identity=(row_key == "after"),
            ))
    return problems


def _is_raw_air_wing_gap_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> bool:
    return (
        frame.get("confidence") == "gap"
        or _parse_raw_air_wing_gap_key(frame.get("entity_key")) is not None
        or (
            isinstance(payload.get("after"), dict)
            and "gap_kind" in payload["after"]
        )
    )


def _validate_raw_air_wing_gap_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    entity_key = frame.get("entity_key")
    gap_kind = _parse_raw_air_wing_gap_key(entity_key)
    if gap_kind is None:
        problems.append("collector air wing gap entity_key not stable air_wings gap key")
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector air wing gap payload contains raw pointer/tag-db diagnostic key"
        )
    changed_fields = payload.get("changed_fields")
    if isinstance(changed_fields, list):
        for i, field in enumerate(changed_fields):
            if _forbidden_raw_key_name(field):
                problems.append(
                    "collector air wing gap "
                    f"changed_fields[{i}] raw pointer/tag-db diagnostic key"
                )
    for row_key in ("before", "after"):
        row = payload.get(row_key)
        if isinstance(row, dict):
            problems.extend(_validate_raw_air_wing_gap_row(
                row,
                f"collector air wing gap {row_key}",
                entity_key if isinstance(entity_key, str) else None,
                gap_kind,
            ))
    return problems


def _validate_raw_air_wing_gap_row(
    row: Dict[str, Any],
    prefix: str,
    entity_key: Optional[str],
    entity_gap_kind: Optional[str],
) -> List[str]:
    problems: List[str] = []
    for key in row:
        if key not in RAW_AIR_WING_GAP_FIELDS:
            problems.append(f"{prefix}.{key} unsupported")

    gap_key = row.get("gap_key")
    if not _valid_raw_key(gap_key):
        problems.append(f"{prefix}.gap_key missing or not stable str")
    elif entity_key is not None and gap_key != entity_key:
        problems.append(f"{prefix}.gap_key != entity_key")

    gap_kind = row.get("gap_kind")
    if not _valid_raw_string(gap_kind, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        problems.append(f"{prefix}.gap_kind missing or not bounded str")
    elif _valid_hex_pointer(gap_kind):
        problems.append(f"{prefix}.gap_kind looks like pointer")
    elif entity_gap_kind is not None and gap_kind != entity_gap_kind:
        problems.append(f"{prefix}.gap_kind != entity_key")

    status = row.get("status")
    if status not in ("validation_failed", "unsupported", "layout_invalid"):
        problems.append(f"{prefix}.status invalid")
    return problems


def _validate_raw_air_wing_row(
    row: Dict[str, Any],
    prefix: str,
    entity_uid: Optional[tuple[str, int, int]],
    *,
    require_identity: bool,
) -> List[str]:
    problems: List[str] = []
    for key in row:
        if key not in RAW_AIR_WING_FIELDS:
            problems.append(f"{prefix}.{key} unsupported")

    air_wing_key = row.get("air_wing_key")
    if air_wing_key is not None:
        parsed_key = _parse_raw_air_wing_entity_key(air_wing_key)
        if parsed_key is None:
            problems.append(f"{prefix}.air_wing_key not stable air_wing key|null")
        elif entity_uid is not None and parsed_key != entity_uid:
            problems.append(f"{prefix}.air_wing_key != entity_key")
    elif require_identity:
        problems.append(f"{prefix}.air_wing_key missing")

    owner = row.get("owner")
    wing_id = row.get("wing_id")
    generation = row.get("generation")
    if owner is None:
        if require_identity:
            problems.append(f"{prefix}.owner missing")
    elif not schema.valid_tag(owner):
        problems.append(f"{prefix}.owner not valid TAG|null")
    elif entity_uid is not None and owner != entity_uid[0]:
        problems.append(f"{prefix}.owner != entity_key")

    if wing_id is None:
        if require_identity:
            problems.append(f"{prefix}.wing_id missing")
    elif not _valid_bounded_int(
        wing_id,
        minimum=1,
        maximum=RAW_AIR_WING_ID_MAX,
    ):
        problems.append(f"{prefix}.wing_id not positive int|null in range")
    elif entity_uid is not None and wing_id != entity_uid[1]:
        problems.append(f"{prefix}.wing_id != entity_key")

    if generation is None:
        if require_identity:
            problems.append(f"{prefix}.generation missing")
    elif not _valid_bounded_int(
        generation,
        minimum=0,
        maximum=RAW_AIR_WING_GENERATION_MAX,
    ):
        problems.append(f"{prefix}.generation not nonnegative int|null in range")
    elif entity_uid is not None and generation != entity_uid[2]:
        problems.append(f"{prefix}.generation != entity_key")

    for key in ("count", "effective_planes"):
        value = row.get(key)
        if value is not None and not _valid_bounded_int(
            value,
            minimum=0,
            maximum=RAW_AIR_WING_PLANES_MAX,
        ):
            problems.append(f"{prefix}.{key} not nonnegative int|null in range")

    mission_mask = row.get("mission_mask")
    if mission_mask is not None and not _valid_bounded_int(
        mission_mask,
        minimum=0,
        maximum=RAW_AIR_WING_MISSION_MASK_MAX,
    ):
        problems.append(f"{prefix}.mission_mask not nonnegative u32|null")

    mission_region = row.get("mission_region")
    if mission_region is not None and not _valid_bounded_int(
        mission_region,
        minimum=1,
        maximum=RAW_AIR_WING_REGION_ID_MAX,
    ):
            problems.append(f"{prefix}.mission_region not positive int|null in range")
    return problems


def _validate_raw_ship_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    if _is_raw_ship_gap_delta(frame, payload):
        return _validate_raw_ship_gap_delta(frame, payload)

    entity_uid = _parse_raw_ship_entity_key(frame.get("entity_key"))
    if entity_uid is None:
        problems.append("collector ship delta entity_key not stable ship key")
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector ship delta payload contains raw pointer/tag-db diagnostic key"
        )
    changed_fields = payload.get("changed_fields")
    if isinstance(changed_fields, list):
        for i, field in enumerate(changed_fields):
            if _forbidden_raw_key_name(field):
                problems.append(
                    "collector ship delta "
                    f"changed_fields[{i}] raw pointer/tag-db diagnostic key"
                )
            elif field not in RAW_SHIP_FIELDS:
                problems.append(f"collector ship delta changed_fields[{i}] invalid")
    for row_key in ("before", "after"):
        row = payload.get(row_key)
        if isinstance(row, dict):
            problems.extend(_validate_raw_ship_row(
                row,
                f"collector ship delta {row_key}",
                entity_uid,
                require_identity=(row_key == "after"),
            ))
    return problems


def _is_raw_ship_gap_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> bool:
    return (
        frame.get("confidence") == "gap"
        or _parse_raw_ship_gap_key(frame.get("entity_key")) is not None
        or (
            isinstance(payload.get("after"), dict)
            and "gap_kind" in payload["after"]
        )
    )


def _validate_raw_ship_gap_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    entity_key = frame.get("entity_key")
    gap_kind = _parse_raw_ship_gap_key(entity_key)
    if gap_kind is None:
        problems.append("collector ship gap entity_key not stable ships gap key")
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector ship gap payload contains raw pointer/tag-db diagnostic key"
        )
    changed_fields = payload.get("changed_fields")
    if isinstance(changed_fields, list):
        for i, field in enumerate(changed_fields):
            if _forbidden_raw_key_name(field):
                problems.append(
                    "collector ship gap "
                    f"changed_fields[{i}] raw pointer/tag-db diagnostic key"
                )
            elif field not in RAW_SHIP_GAP_FIELDS:
                problems.append(f"collector ship gap changed_fields[{i}] invalid")
    for row_key in ("before", "after"):
        row = payload.get(row_key)
        if isinstance(row, dict):
            problems.extend(_validate_raw_ship_gap_row(
                row,
                f"collector ship gap {row_key}",
                entity_key if isinstance(entity_key, str) else None,
                gap_kind,
            ))
    return problems


def _validate_raw_ship_gap_row(
    row: Dict[str, Any],
    prefix: str,
    entity_key: Optional[str],
    entity_gap_kind: Optional[str],
) -> List[str]:
    problems: List[str] = []
    for key in row:
        if key not in RAW_SHIP_GAP_FIELDS:
            problems.append(f"{prefix}.{key} unsupported")

    gap_key = row.get("gap_key")
    if not _valid_raw_key(gap_key):
        problems.append(f"{prefix}.gap_key missing or not stable str")
    elif entity_key is not None and gap_key != entity_key:
        problems.append(f"{prefix}.gap_key != entity_key")

    gap_kind = row.get("gap_kind")
    if not _valid_raw_string(gap_kind, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        problems.append(f"{prefix}.gap_kind missing or not bounded str")
    elif _valid_hex_pointer(gap_kind):
        problems.append(f"{prefix}.gap_kind looks like pointer")
    elif entity_gap_kind is not None and gap_kind != entity_gap_kind:
        problems.append(f"{prefix}.gap_kind != entity_key")

    status = row.get("status")
    if status not in ("validation_failed", "unsupported", "layout_invalid"):
        problems.append(f"{prefix}.status invalid")
    return problems


def _validate_raw_ship_row(
    row: Dict[str, Any],
    prefix: str,
    entity_uid: Optional[int],
    *,
    require_identity: bool,
) -> List[str]:
    problems: List[str] = []
    for key in row:
        if key not in RAW_SHIP_FIELDS:
            problems.append(f"{prefix}.{key} unsupported")

    ship_key = row.get("ship_key")
    if ship_key is not None:
        parsed_key = _parse_raw_ship_entity_key(ship_key)
        if parsed_key is None:
            problems.append(f"{prefix}.ship_key not stable ship key|null")
        elif entity_uid is not None and parsed_key != entity_uid:
            problems.append(f"{prefix}.ship_key != entity_key")
    elif require_identity:
        problems.append(f"{prefix}.ship_key missing")

    instance_id = row.get("instance_id")
    if instance_id is None:
        if require_identity:
            problems.append(f"{prefix}.instance_id missing")
    elif not _valid_bounded_int(
        instance_id,
        minimum=1,
        maximum=RAW_SHIP_INSTANCE_ID_MAX,
    ):
        problems.append(f"{prefix}.instance_id not positive int|null in range")
    elif entity_uid is not None and instance_id != entity_uid:
        problems.append(f"{prefix}.instance_id != entity_key")

    definition = row.get("definition")
    if definition is not None:
        if not _valid_raw_string(definition, max_length=RAW_SHORT_STRING_MAX_LENGTH):
            problems.append(f"{prefix}.definition not bounded str|null")
        elif _valid_hex_pointer(definition):
            problems.append(f"{prefix}.definition looks like pointer")

    category_enum = row.get("category_enum")
    category = row.get("category")
    if category_enum is not None and not _valid_bounded_int(
        category_enum,
        minimum=0,
        maximum=100,
    ):
        problems.append(f"{prefix}.category_enum not nonnegative int|null in range")
    elif category_enum is not None and category_enum not in RAW_SHIP_CATEGORY_LABELS:
        problems.append(f"{prefix}.category_enum invalid")

    if category is not None:
        if not _valid_raw_string(category, max_length=RAW_SHORT_STRING_MAX_LENGTH):
            problems.append(f"{prefix}.category not bounded str|null")
        elif category not in set(RAW_SHIP_CATEGORY_LABELS.values()):
            problems.append(f"{prefix}.category invalid")
        elif (
            category_enum in RAW_SHIP_CATEGORY_LABELS
            and category != RAW_SHIP_CATEGORY_LABELS[category_enum]
        ):
            problems.append(f"{prefix}.category != category_enum")

    raw_bounds = (
        ("strength_raw", RAW_SHIP_FIXED_RAW_MAX),
        ("organisation_raw", RAW_SHIP_PERCENT_RAW_MAX),
        ("experience_raw", RAW_SHIP_PERCENT_RAW_MAX),
    )
    for key, maximum in raw_bounds:
        value = row.get(key)
        if value is not None and not _valid_bounded_int(
            value,
            minimum=0,
            maximum=maximum,
        ):
            problems.append(f"{prefix}.{key} not nonnegative int|null in range")

    float_bounds = (
        ("strength_hp", RAW_SHIP_STRENGTH_HP_MAX),
        ("organisation", RAW_SHIP_NORMALIZED_MAX),
        ("experience", RAW_SHIP_NORMALIZED_MAX),
    )
    for key, maximum in float_bounds:
        value = row.get(key)
        if value is not None and (
            not _is_finite_number(value)
            or value < 0.0
            or value > maximum
        ):
            problems.append(f"{prefix}.{key} not finite number|null in range")

    sunk_convoys = row.get("sunk_convoys")
    if sunk_convoys is not None and not _valid_bounded_int(
        sunk_convoys,
        minimum=0,
        maximum=RAW_SHIP_SUNK_CONVOYS_MAX,
    ):
        problems.append(f"{prefix}.sunk_convoys not nonnegative int|null in range")
    return problems


def _validate_raw_oob_command_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    if _is_raw_oob_command_gap_delta(frame, payload):
        return _validate_raw_oob_command_gap_delta(frame, payload)

    entity = _parse_raw_oob_command_entity_key(frame.get("entity_key"))
    if entity is None:
        problems.append("collector oob_command delta entity_key not stable OOB key")
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector oob_command delta payload contains raw pointer/tag-db "
            "diagnostic key"
        )
    changed_fields = payload.get("changed_fields")
    if isinstance(changed_fields, list):
        for i, field in enumerate(changed_fields):
            if _forbidden_raw_key_name(field):
                problems.append(
                    "collector oob_command delta "
                    f"changed_fields[{i}] raw pointer/tag-db diagnostic key"
                )
            elif field not in RAW_OOB_COMMAND_FIELDS:
                problems.append(
                    f"collector oob_command delta changed_fields[{i}] invalid"
                )
    for row_key in ("before", "after"):
        row = payload.get(row_key)
        if isinstance(row, dict):
            problems.extend(_validate_raw_oob_command_row(
                row,
                f"collector oob_command delta {row_key}",
                entity,
                require_identity=(row_key == "after"),
            ))
    return problems


def _is_raw_oob_command_gap_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> bool:
    return (
        frame.get("confidence") == "gap"
        or _parse_raw_oob_command_gap_key(frame.get("entity_key")) is not None
        or (
            isinstance(payload.get("after"), dict)
            and "gap_kind" in payload["after"]
        )
    )


def _validate_raw_oob_command_gap_delta(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    entity_key = frame.get("entity_key")
    gap_kind = _parse_raw_oob_command_gap_key(entity_key)
    if gap_kind is None:
        problems.append("collector oob_command gap entity_key not stable gap key")
    if _contains_forbidden_raw_key(payload):
        problems.append(
            "collector oob_command gap payload contains raw pointer/tag-db "
            "diagnostic key"
        )
    changed_fields = payload.get("changed_fields")
    if isinstance(changed_fields, list):
        for i, field in enumerate(changed_fields):
            if _forbidden_raw_key_name(field):
                problems.append(
                    "collector oob_command gap "
                    f"changed_fields[{i}] raw pointer/tag-db diagnostic key"
                )
            elif field not in RAW_OOB_COMMAND_GAP_FIELDS:
                problems.append(f"collector oob_command gap changed_fields[{i}] invalid")
    for row_key in ("before", "after"):
        row = payload.get(row_key)
        if isinstance(row, dict):
            problems.extend(_validate_raw_oob_command_gap_row(
                row,
                f"collector oob_command gap {row_key}",
                entity_key if isinstance(entity_key, str) else None,
                gap_kind,
            ))
    return problems


def _validate_raw_oob_command_gap_row(
    row: Dict[str, Any],
    prefix: str,
    entity_key: Optional[str],
    entity_gap_kind: Optional[str],
) -> List[str]:
    problems: List[str] = []
    for key in row:
        if key not in RAW_OOB_COMMAND_GAP_FIELDS:
            problems.append(f"{prefix}.{key} unsupported")

    gap_key = row.get("gap_key")
    if not _valid_raw_key(gap_key):
        problems.append(f"{prefix}.gap_key missing or not stable str")
    elif entity_key is not None and gap_key != entity_key:
        problems.append(f"{prefix}.gap_key != entity_key")

    gap_kind = row.get("gap_kind")
    if not _valid_raw_string(gap_kind, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        problems.append(f"{prefix}.gap_kind missing or not bounded str")
    elif _valid_hex_pointer(gap_kind):
        problems.append(f"{prefix}.gap_kind looks like pointer")
    elif entity_gap_kind is not None and gap_kind != entity_gap_kind:
        problems.append(f"{prefix}.gap_kind != entity_key")

    status = row.get("status")
    if status not in ("validation_failed", "unsupported", "layout_invalid"):
        problems.append(f"{prefix}.status invalid")
    return problems


def _validate_raw_oob_command_row(
    row: Dict[str, Any],
    prefix: str,
    entity: Optional[tuple[str, int]],
    *,
    require_identity: bool,
) -> List[str]:
    problems: List[str] = []
    for key in row:
        if key not in RAW_OOB_COMMAND_FIELDS:
            problems.append(f"{prefix}.{key} unsupported")

    oob_key = row.get("oob_key")
    parsed_key = None
    if oob_key is not None:
        parsed_key = _parse_raw_oob_command_entity_key(oob_key)
        if parsed_key is None:
            problems.append(f"{prefix}.oob_key not stable OOB key|null")
        elif entity is not None and parsed_key != entity:
            problems.append(f"{prefix}.oob_key != entity_key")
    elif require_identity:
        problems.append(f"{prefix}.oob_key missing")

    effective_entity = entity or parsed_key
    entity_kind = row.get("entity_kind")
    if entity_kind is not None and entity_kind not in RAW_OOB_COMMAND_ENTITY_KINDS:
        problems.append(f"{prefix}.entity_kind invalid")
    elif (
        effective_entity is not None
        and entity_kind is not None
        and entity_kind != effective_entity[0]
    ):
        problems.append(f"{prefix}.entity_kind != entity_key")
    elif require_identity and entity_kind is None:
        problems.append(f"{prefix}.entity_kind missing")

    army_id = row.get("army_id")
    army_group_id = row.get("army_group_id")
    fleet_id = row.get("fleet_id")
    task_force_id = row.get("task_force_id")
    leader_id = row.get("leader_id")
    for key, value in (
        ("army_id", army_id),
        ("army_group_id", army_group_id),
        ("fleet_id", fleet_id),
        ("task_force_id", task_force_id),
        ("leader_id", leader_id),
    ):
        if value is not None and not _valid_bounded_int(
            value,
            minimum=1,
            maximum=RAW_OOB_COMMAND_ID_MAX,
        ):
            problems.append(f"{prefix}.{key} not positive int|null in range")

    if effective_entity is not None:
        kind, object_id = effective_entity
        if kind == "army":
            if army_id != object_id:
                problems.append(f"{prefix}.army_id != entity_key")
            if fleet_id is not None:
                problems.append(f"{prefix}.fleet_id set on army row")
            if task_force_id is not None:
                problems.append(f"{prefix}.task_force_id set on army row")
        elif kind == "army_group":
            if army_group_id != object_id:
                problems.append(f"{prefix}.army_group_id != entity_key")
            if army_id is not None:
                problems.append(f"{prefix}.army_id set on army_group row")
            if fleet_id is not None:
                problems.append(f"{prefix}.fleet_id set on army_group row")
            if task_force_id is not None:
                problems.append(f"{prefix}.task_force_id set on army_group row")
        elif kind == "fleet":
            if fleet_id != object_id:
                problems.append(f"{prefix}.fleet_id != entity_key")
            if army_id is not None:
                problems.append(f"{prefix}.army_id set on fleet row")
            if army_group_id is not None:
                problems.append(f"{prefix}.army_group_id set on fleet row")
            if task_force_id is not None:
                problems.append(f"{prefix}.task_force_id set on fleet row")
        elif kind == "task_force":
            if task_force_id != object_id:
                problems.append(f"{prefix}.task_force_id != entity_key")
            if army_id is not None:
                problems.append(f"{prefix}.army_id set on task_force row")
            if army_group_id is not None:
                problems.append(f"{prefix}.army_group_id set on task_force row")

    problems.extend(_validate_oob_id_list(
        row.get("member_division_ids"),
        f"{prefix}.member_division_ids",
    ))
    problems.extend(_validate_oob_id_list(
        row.get("member_army_ids"),
        f"{prefix}.member_army_ids",
    ))
    problems.extend(_validate_oob_id_list(
        row.get("task_force_ids"),
        f"{prefix}.task_force_ids",
    ))
    problems.extend(_validate_oob_id_list(
        row.get("ship_ids"),
        f"{prefix}.ship_ids",
    ))

    for key in (
        "member_division_count",
        "member_army_count",
        "member_count_engine",
        "missing_member_count",
    ):
        value = row.get(key)
        if value is not None and not _valid_bounded_int(
            value,
            minimum=0,
            maximum=RAW_OOB_COMMAND_MEMBER_COUNT_MAX,
        ):
            problems.append(f"{prefix}.{key} not nonnegative int|null in range")

    for key in ("task_force_count", "ship_count"):
        value = row.get(key)
        if value is not None and not _valid_bounded_int(
            value,
            minimum=0,
            maximum=RAW_OOB_COMMAND_SHIP_COUNT_MAX,
        ):
            problems.append(f"{prefix}.{key} not nonnegative int|null in range")

    member_count_engine = row.get("member_count_engine")
    missing_member_count = row.get("missing_member_count")
    if (
        _valid_bounded_int(
            member_count_engine,
            minimum=0,
            maximum=RAW_OOB_COMMAND_MEMBER_COUNT_MAX,
        )
        and _valid_bounded_int(
            missing_member_count,
            minimum=0,
            maximum=RAW_OOB_COMMAND_MEMBER_COUNT_MAX,
        )
        and missing_member_count > member_count_engine
    ):
        problems.append(f"{prefix}.missing_member_count exceeds member_count_engine")

    for key in (
        "member_division_status",
        "member_army_status",
        "leader_status",
        "leader_trait_status",
        "task_force_status",
        "parent_status",
        "ship_status",
    ):
        status = row.get(key)
        if status is not None and status not in RAW_OOB_COMMAND_STATUSES:
            problems.append(f"{prefix}.{key} invalid")

    leader_skill = row.get("leader_skill")
    if leader_skill is not None and not _valid_bounded_int(
        leader_skill,
        minimum=0,
        maximum=RAW_OOB_COMMAND_LEADER_SKILL_MAX,
    ):
        problems.append(f"{prefix}.leader_skill not nonnegative int|null in range")

    leader_traits = row.get("leader_traits")
    if leader_traits is not None:
        if not isinstance(leader_traits, list):
            problems.append(f"{prefix}.leader_traits not list|null")
        elif len(leader_traits) > RAW_OOB_COMMAND_LEADER_TRAITS_MAX:
            problems.append(f"{prefix}.leader_traits too many entries")
        else:
            seen_traits = set()
            for i, trait in enumerate(leader_traits):
                if not isinstance(trait, str) or not schema.valid_script_token(trait):
                    problems.append(f"{prefix}.leader_traits[{i}] invalid token")
                elif trait in seen_traits:
                    problems.append(f"{prefix}.leader_traits[{i}] duplicate token")
                else:
                    seen_traits.add(trait)
    return problems


def _validate_oob_id_list(value: Any, prefix: str) -> List[str]:
    problems: List[str] = []
    if value is None:
        return problems
    if not isinstance(value, list):
        return [f"{prefix} not list|null"]
    if len(value) > RAW_OOB_COMMAND_IDS_MAX:
        problems.append(f"{prefix} too many entries")
    seen = set()
    for i, item in enumerate(value):
        if not _valid_bounded_int(
            item,
            minimum=1,
            maximum=RAW_OOB_COMMAND_ID_MAX,
        ):
            problems.append(f"{prefix}[{i}] not positive int in range")
        elif item in seen:
            problems.append(f"{prefix}[{i}] duplicate id")
        else:
            seen.add(item)
    return problems


def _validate_raw_gap_payload(
    frame: Dict[str, Any],
    payload: Dict[str, Any],
) -> List[str]:
    problems: List[str] = []
    _validate_payload_keys(
        problems,
        payload,
        allowed={
            "gap_kind",
            "domain",
            "start_sequence",
            "end_sequence",
            "reason",
            "detail",
        },
        prefix="collector gap payload",
    )
    gap_kind = payload.get("gap_kind")
    if gap_kind not in RAW_GAP_KINDS:
        problems.append(f"collector gap payload gap_kind {gap_kind!r} invalid")
    domain = payload.get("domain")
    if not _valid_raw_string(domain, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        problems.append("collector gap payload domain missing or not bounded str")
    elif domain not in schema.DOMAINS:
        problems.append(f"collector gap payload domain {domain!r} not canonical")
    elif _valid_raw_string(frame.get("domain"), max_length=RAW_SHORT_STRING_MAX_LENGTH):
        if domain != frame.get("domain"):
            problems.append("collector gap payload domain != raw frame domain")
    start = payload.get("start_sequence")
    end = payload.get("end_sequence")
    for key, value in (("start_sequence", start), ("end_sequence", end)):
        if key not in payload:
            problems.append(f"collector gap payload {key} missing")
        elif value is not None and not _valid_bounded_int(
            value,
            minimum=0,
            maximum=RAW_SEQUENCE_MAX,
        ):
            problems.append(f"collector gap payload {key} not int|null in range")
    if start is not None and end is not None:
        if (
            _valid_bounded_int(start, minimum=0, maximum=RAW_SEQUENCE_MAX)
            and _valid_bounded_int(end, minimum=0, maximum=RAW_SEQUENCE_MAX)
            and start > end
        ):
            problems.append("collector gap payload start_sequence > end_sequence")
    reason = payload.get("reason")
    if not _valid_gap_reason(reason):
        problems.append("collector gap payload reason missing or not machine-readable str")
    detail = payload.get("detail")
    if detail is not None and not isinstance(detail, (dict, list)):
        problems.append("collector gap payload detail not object|list|null")
    return problems


def _validate_payload_keys(
    problems: List[str],
    payload: Dict[str, Any],
    *,
    allowed: set[str],
    prefix: str,
) -> None:
    for key in payload:
        if key not in allowed:
            problems.append(f"{prefix} key {key!r} unsupported")


def _require_raw_int(
    frame: Dict[str, Any],
    key: str,
    *,
    required: bool = True,
) -> List[str]:
    if key not in frame or frame.get(key) is None:
        return [f"collector raw frame {key} missing"] if required else []
    if not _valid_bounded_int(
        frame.get(key),
        minimum=0,
        maximum=RAW_SEQUENCE_MAX,
    ):
        return [f"collector raw frame {key} not int in range"]
    return []


def _valid_raw_string(value: Any, *, max_length: int) -> bool:
    return (
        isinstance(value, str)
        and 0 < len(value) <= max_length
    )


def _valid_raw_key(value: Any) -> bool:
    return (
        _valid_raw_string(value, max_length=RAW_STRING_MAX_LENGTH)
        and not _valid_hex_pointer(value)
    )


def _parse_raw_division_entity_key(value: Any) -> Optional[tuple[int, int]]:
    if not isinstance(value, str) or not value.startswith("division:"):
        return None
    return _parse_raw_division_uid(value[len("division:"):])


def _parse_raw_land_combat_entity_key(value: Any) -> Optional[tuple[int, int]]:
    if not isinstance(value, str) or not value.startswith("land_combat:"):
        return None
    raw = value[len("land_combat:"):]
    parts = raw.split(":")
    if len(parts) != 2:
        return None
    combat_id = _parse_raw_nonnegative_int_string(
        parts[0],
        maximum=RAW_LAND_COMBAT_ID_MAX,
    )
    generation = _parse_raw_nonnegative_int_string(
        parts[1],
        maximum=RAW_LAND_COMBAT_GENERATION_MAX,
    )
    if combat_id is None or combat_id == 0 or generation is None:
        return None
    return (combat_id, generation)


def _parse_raw_land_combat_gap_key(value: Any) -> Optional[str]:
    if not isinstance(value, str) or not value.startswith("land_combats:gap:"):
        return None
    gap_kind = value[len("land_combats:gap:"):]
    if not _valid_raw_string(gap_kind, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        return None
    if _valid_hex_pointer(gap_kind):
        return None
    return gap_kind


def _parse_raw_army_order_entity_key(value: Any) -> Optional[tuple[int, int]]:
    if not isinstance(value, str) or not value.startswith("army_order:"):
        return None
    raw = value[len("army_order:"):]
    parts = raw.split(":")
    if len(parts) != 2:
        return None
    army_id = _parse_raw_nonnegative_int_string(
        parts[0],
        maximum=RAW_ARMY_ORDER_ID_MAX,
    )
    generation = _parse_raw_nonnegative_int_string(
        parts[1],
        maximum=RAW_ARMY_ORDER_GENERATION_MAX,
    )
    if army_id is None or army_id == 0 or generation is None:
        return None
    return (army_id, generation)


def _parse_raw_army_order_gap_key(value: Any) -> Optional[str]:
    if not isinstance(value, str) or not value.startswith("army_orders:gap:"):
        return None
    gap_kind = value[len("army_orders:gap:"):]
    if not _valid_raw_string(gap_kind, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        return None
    if _valid_hex_pointer(gap_kind):
        return None
    return gap_kind


def _parse_raw_naval_combat_entity_key(value: Any) -> Optional[tuple[int, int]]:
    if not isinstance(value, str) or not value.startswith("naval_combat:"):
        return None
    raw = value[len("naval_combat:"):]
    parts = raw.split(":")
    if len(parts) != 2:
        return None
    combat_id = _parse_raw_nonnegative_int_string(
        parts[0],
        maximum=RAW_NAVAL_COMBAT_ID_MAX,
    )
    generation = _parse_raw_nonnegative_int_string(
        parts[1],
        maximum=RAW_NAVAL_COMBAT_GENERATION_MAX,
    )
    if combat_id is None or combat_id == 0 or generation is None:
        return None
    return (combat_id, generation)


def _parse_raw_naval_combat_gap_key(value: Any) -> Optional[str]:
    if not isinstance(value, str) or not value.startswith("naval_air_combats:gap:"):
        return None
    gap_kind = value[len("naval_air_combats:gap:"):]
    if not _valid_raw_string(gap_kind, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        return None
    if _valid_hex_pointer(gap_kind):
        return None
    return gap_kind


def _parse_raw_naval_combat_member_entity_key(value: Any) -> Optional[Dict[str, Any]]:
    prefix = "naval_combat_members:naval_combat:"
    if not isinstance(value, str) or not value.startswith(prefix):
        return None
    raw = value[len(prefix):]
    parts = raw.split(":")
    if len(parts) not in (5, 7):
        return None
    combat_id = _parse_raw_nonnegative_int_string(
        parts[0],
        maximum=RAW_NAVAL_COMBAT_MEMBER_ID_MAX,
    )
    generation = _parse_raw_nonnegative_int_string(
        parts[1],
        maximum=RAW_NAVAL_COMBAT_MEMBER_GENERATION_MAX,
    )
    side = parts[2]
    kind = parts[3]
    if combat_id is None or combat_id == 0 or generation is None:
        return None
    if side not in RAW_NAVAL_COMBAT_MEMBER_SIDES:
        return None
    if kind == "ship" and len(parts) == 5:
        instance_id = _parse_raw_nonnegative_int_string(
            parts[4],
            maximum=RAW_NAVAL_COMBAT_MEMBER_SHIP_INSTANCE_ID_MAX,
        )
        if instance_id is None or instance_id == 0:
            return None
        return {
            "combat_id": combat_id,
            "generation": generation,
            "side": side,
            "kind": kind,
            "instance_id": instance_id,
        }
    if kind == "convoy" and len(parts) == 7:
        group_index = _parse_raw_nonnegative_int_string(
            parts[4],
            maximum=RAW_NAVAL_COMBAT_MEMBER_GROUP_INDEX_MAX,
        )
        member_index = _parse_raw_nonnegative_int_string(
            parts[5],
            maximum=RAW_NAVAL_COMBAT_MEMBER_INDEX_MAX,
        )
        slot_index = _parse_raw_nonnegative_int_string(
            parts[6],
            maximum=RAW_NAVAL_COMBAT_MEMBER_SLOT_INDEX_MAX,
        )
        if group_index is None or member_index is None or slot_index is None:
            return None
        return {
            "combat_id": combat_id,
            "generation": generation,
            "side": side,
            "kind": kind,
            "group_index": group_index,
            "member_index": member_index,
            "battle_slot_index_raw": slot_index,
        }
    return None


def _parse_raw_naval_combat_member_gap_key(value: Any) -> Optional[str]:
    if not isinstance(value, str) or not value.startswith("naval_combat_members:gap:"):
        return None
    gap_kind = value[len("naval_combat_members:gap:"):]
    if not _valid_raw_string(gap_kind, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        return None
    if _valid_hex_pointer(gap_kind):
        return None
    return gap_kind


def _parse_raw_air_wing_entity_key(value: Any) -> Optional[tuple[str, int, int]]:
    if not isinstance(value, str) or not value.startswith("air_wing:"):
        return None
    raw = value[len("air_wing:"):]
    parts = raw.split(":")
    if len(parts) != 3:
        return None
    owner = parts[0]
    if not schema.valid_tag(owner):
        return None
    wing_id = _parse_raw_nonnegative_int_string(
        parts[1],
        maximum=RAW_AIR_WING_ID_MAX,
    )
    generation = _parse_raw_nonnegative_int_string(
        parts[2],
        maximum=RAW_AIR_WING_GENERATION_MAX,
    )
    if wing_id is None or wing_id == 0 or generation is None:
        return None
    return (owner, wing_id, generation)


def _parse_raw_air_wing_gap_key(value: Any) -> Optional[str]:
    if not isinstance(value, str) or not value.startswith("air_wings:gap:"):
        return None
    gap_kind = value[len("air_wings:gap:"):]
    if not _valid_raw_string(gap_kind, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        return None
    if _valid_hex_pointer(gap_kind):
        return None
    return gap_kind


def _parse_raw_ship_entity_key(value: Any) -> Optional[int]:
    if not isinstance(value, str) or not value.startswith("ship:"):
        return None
    raw = value[len("ship:"):]
    instance_id = _parse_raw_nonnegative_int_string(
        raw,
        maximum=RAW_SHIP_INSTANCE_ID_MAX,
    )
    if instance_id is None or instance_id == 0:
        return None
    return instance_id


def _parse_raw_ship_gap_key(value: Any) -> Optional[str]:
    if not isinstance(value, str) or not value.startswith("ships:gap:"):
        return None
    gap_kind = value[len("ships:gap:"):]
    if not _valid_raw_string(gap_kind, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        return None
    if _valid_hex_pointer(gap_kind):
        return None
    return gap_kind


def _parse_raw_oob_command_entity_key(value: Any) -> Optional[tuple[str, int]]:
    if not isinstance(value, str) or not value.startswith("oob_command:"):
        return None
    raw = value[len("oob_command:"):]
    parts = raw.split(":")
    if len(parts) != 2:
        return None
    kind = parts[0]
    if kind not in RAW_OOB_COMMAND_ENTITY_KINDS:
        return None
    object_id = _parse_raw_nonnegative_int_string(
        parts[1],
        maximum=RAW_OOB_COMMAND_ID_MAX,
    )
    if object_id is None or object_id == 0:
        return None
    return (kind, object_id)


def _parse_raw_oob_command_gap_key(value: Any) -> Optional[str]:
    if not isinstance(value, str) or not value.startswith("oob_command:gap:"):
        return None
    gap_kind = value[len("oob_command:gap:"):]
    if not _valid_raw_string(gap_kind, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        return None
    if _valid_hex_pointer(gap_kind):
        return None
    return gap_kind


def _parse_raw_division_uid(value: Any) -> Optional[tuple[int, int]]:
    if not _valid_raw_string(value, max_length=RAW_SHORT_STRING_MAX_LENGTH):
        return None
    parts = value.split(":")
    if len(parts) != 2:
        return None
    div_id = _parse_raw_nonnegative_int_string(
        parts[0],
        maximum=RAW_DIVISION_ID_MAX,
    )
    div_type = _parse_raw_nonnegative_int_string(
        parts[1],
        maximum=RAW_DIVISION_ID_MAX,
    )
    if div_id is None or div_type is None:
        return None
    return (div_id, div_type)


def _parse_raw_nonnegative_int_string(
    value: Any,
    *,
    maximum: int,
) -> Optional[int]:
    if not isinstance(value, str) or not value:
        return None
    try:
        parsed = int(value)
    except ValueError:
        return None
    if str(parsed) != value:
        return None
    if not 0 <= parsed <= maximum:
        return None
    return parsed


def _valid_gap_reason(value: Any) -> bool:
    return (
        _valid_raw_string(value, max_length=RAW_SHORT_STRING_MAX_LENGTH)
        and all(ch.isalnum() or ch in "._:-/" for ch in value)
    )


def _validate_bounded_raw_value(
    value: Any,
    prefix: str,
    *,
    depth: int = 0,
) -> List[str]:
    problems: List[str] = []
    if depth > RAW_DETAIL_MAX_DEPTH:
        return [f"{prefix} too deep"]
    if isinstance(value, dict):
        if len(value) > RAW_CONTAINER_MAX_ENTRIES:
            problems.append(f"{prefix} too many entries")
        for key, child in value.items():
            if not _valid_raw_string(key, max_length=RAW_STRING_MAX_LENGTH):
                problems.append(f"{prefix} key {key!r} not bounded str")
                continue
            problems.extend(_validate_bounded_raw_value(
                child,
                f"{prefix}.{key}",
                depth=depth + 1,
            ))
        return problems
    if isinstance(value, list):
        if len(value) > RAW_CONTAINER_MAX_ENTRIES:
            problems.append(f"{prefix} too many entries")
        for i, child in enumerate(value):
            problems.extend(_validate_bounded_raw_value(
                child,
                f"{prefix}[{i}]",
                depth=depth + 1,
            ))
        return problems
    if value is None or isinstance(value, bool):
        return problems
    if isinstance(value, str):
        if len(value) > RAW_STRING_MAX_LENGTH:
            problems.append(f"{prefix} str too long")
        return problems
    if isinstance(value, int):
        if abs(value) > RAW_SEQUENCE_MAX:
            problems.append(f"{prefix} int out of range")
        return problems
    if isinstance(value, float):
        if not _is_finite_number(value):
            problems.append(f"{prefix} not finite number")
        return problems
    problems.append(f"{prefix} unsupported value type")
    return problems


def _contains_forbidden_raw_key(value: Any) -> bool:
    if isinstance(value, dict):
        for key, child in value.items():
            if _forbidden_raw_key_name(key):
                return True
            if _contains_forbidden_raw_key(child):
                return True
    if isinstance(value, list):
        return any(_contains_forbidden_raw_key(child) for child in value)
    return False


def _forbidden_raw_key_name(key: Any) -> bool:
    if not isinstance(key, str):
        return False
    lowered = key.lower()
    return (
        lowered in RAW_FORBIDDEN_KEY_NAMES
        or lowered.endswith(RAW_FORBIDDEN_KEY_SUFFIXES)
    )


def _explicit_scheduled_domains(
    frame: Dict[str, Any],
    root_probe: Optional[Dict[str, Any]] = None,
) -> set[str]:
    if frame.get("capture_boundary") != "scheduled_hour_snapshot":
        return set()
    raw_domains = frame.get("domains")
    if not isinstance(raw_domains, list) and isinstance(root_probe, dict):
        raw_domains = root_probe.get("domains")
    return set(_clean_domains(raw_domains or []))


def _validate_country_rows(countries: Dict[str, Any]) -> List[str]:
    problems: List[str] = []
    for tag, row in countries.items():
        if not schema.valid_tag(tag):
            problems.append(f"collector frame country key {tag!r} not a valid TAG")
            continue
        if not isinstance(row, dict):
            problems.append(f"collector frame country {tag!r} not a dict")
            continue
        collector_object = row.get("collector_object")
        if collector_object is not None and not isinstance(collector_object, str):
            problems.append(f"collector frame country {tag!r}.collector_object not str|null")
        present = row.get("present")
        if present is not None and not isinstance(present, bool):
            problems.append(f"collector frame country {tag!r}.present not bool|null")
        identity = row.get("identity")
        if identity is not None:
            if not isinstance(identity, dict):
                problems.append(f"collector frame country {tag!r}.identity not dict|null")
                continue
            original_capital = identity.get("original_capital")
            if original_capital is not None and not isinstance(original_capital, int):
                problems.append(
                    f"collector frame country {tag!r}.identity.original_capital not int|null")
            tag_db_id = identity.get("tag_db_id")
            if tag_db_id is not None and not isinstance(tag_db_id, int):
                problems.append(
                    f"collector frame country {tag!r}.identity.tag_db_id not int|null")
            elif tag_db_id is not None and not (1 <= tag_db_id <= 1024):
                problems.append(
                    f"collector frame country {tag!r}.identity.tag_db_id out of range")
            original_tag = identity.get("original_tag")
            if original_tag is not None and not schema.valid_tag(original_tag):
                problems.append(
                    f"collector frame country {tag!r}.identity.original_tag not TAG|null")
            for key in ("reserved_dynamic_country", "templates_locked"):
                value = identity.get(key)
                if value is not None and not isinstance(value, bool):
                    problems.append(
                        f"collector frame country {tag!r}.identity.{key} not bool|null")
        scalars = row.get("scalars")
        if scalars is not None:
            if not isinstance(scalars, dict):
                problems.append(f"collector frame country {tag!r}.scalars not dict|null")
                continue
            if present is not True:
                problems.append(
                    f"collector frame country {tag!r}.scalars requires present=true")
            if not _has_valid_scalar_identity(row):
                problems.append(
                    f"collector frame country {tag!r}.scalars requires identity.tag_db_id")
            if collector_object is None or not _valid_hex_pointer(collector_object):
                problems.append(
                    f"collector frame country {tag!r}.scalars requires hex collector_object")
            for key in ("stability", "war_support", "political_power",
                        "command_power", "convoys"):
                value = scalars.get(key)
                if value is not None and not _is_finite_number(value):
                    problems.append(
                        f"collector frame country {tag!r}.scalars.{key} not number|null")
                elif key in ("stability", "war_support") and value is not None:
                    if not (0.0 <= float(value) <= 1.0):
                        problems.append(
                            f"collector frame country {tag!r}.scalars.{key} out of range")
                elif key in ("command_power", "convoys") and value is not None:
                    if float(value) < 0.0:
                        problems.append(
                            f"collector frame country {tag!r}.scalars.{key} out of range")
            for key in ("num_ships", "manpower_pool"):
                value = scalars.get(key)
                if value is not None and (isinstance(value, bool)
                                          or not isinstance(value, int)):
                    problems.append(
                        f"collector frame country {tag!r}.scalars.{key} not int|null")
                elif value is not None and value < 0:
                    problems.append(
                        f"collector frame country {tag!r}.scalars.{key} out of range")
        misc = row.get("misc")
        if misc is not None:
            if not isinstance(misc, dict):
                problems.append(f"collector frame country {tag!r}.misc not dict|null")
            else:
                if present is not True:
                    problems.append(
                        f"collector frame country {tag!r}.misc requires present=true")
                if not _has_valid_scalar_identity(row):
                    problems.append(
                        f"collector frame country {tag!r}.misc requires identity.tag_db_id")
                if collector_object is None or not _valid_hex_pointer(collector_object):
                    problems.append(
                        f"collector frame country {tag!r}.misc requires hex collector_object")
                if not any(key in misc for key in COUNTRY_MISC_PAYLOAD_KEYS):
                    problems.append(
                        f"collector frame country {tag!r}.misc has no measured leaves")
                for key, maximum in (
                    ("convoys_destroyed", COUNTRY_MISC_CONVOYS_DESTROYED_MAX),
                    ("ace_count", COUNTRY_MISC_ACE_COUNT_MAX),
                ):
                    value = misc.get(key)
                    if value is not None and not _valid_bounded_int(
                        value, minimum=0, maximum=maximum
                    ):
                        problems.append(
                            f"collector frame country {tag!r}.misc.{key} "
                            "not int in range|null"
                        )
                accidents_score = misc.get("accidents_score")
                if accidents_score is not None:
                    if not _is_finite_number(accidents_score):
                        problems.append(
                            f"collector frame country {tag!r}.misc.accidents_score "
                            "not number|null"
                        )
                    elif not (
                        0.0
                        <= float(accidents_score)
                        <= COUNTRY_MISC_ACCIDENTS_SCORE_MAX
                    ):
                        problems.append(
                            f"collector frame country {tag!r}.misc.accidents_score "
                            "out of range"
                        )
                cosmetic_tag = misc.get("cosmetic_tag")
                if cosmetic_tag is not None and not _valid_country_misc_token(cosmetic_tag):
                    problems.append(
                        f"collector frame country {tag!r}.misc.cosmetic_tag "
                        "not token|null"
                    )
        fuel = row.get("fuel")
        if fuel is not None:
            if not isinstance(fuel, dict):
                problems.append(f"collector frame country {tag!r}.fuel not dict|null")
            else:
                if present is not True:
                    problems.append(
                        f"collector frame country {tag!r}.fuel requires present=true")
                if not _has_valid_scalar_identity(row):
                    problems.append(
                        f"collector frame country {tag!r}.fuel requires identity.tag_db_id")
                if collector_object is None or not _valid_hex_pointer(collector_object):
                    problems.append(
                        f"collector frame country {tag!r}.fuel requires hex collector_object")
                for key in ("current", "max"):
                    value = fuel.get(key)
                    if value is not None and not _is_finite_number(value):
                        problems.append(
                            f"collector frame country {tag!r}.fuel.{key} not number|null")
                    elif value is not None and float(value) < 0.0:
                        problems.append(
                            f"collector frame country {tag!r}.fuel.{key} out of range")
                    elif value is not None and float(value) > FUEL_MEASURE_ABS_MAX:
                        problems.append(
                            f"collector frame country {tag!r}.fuel.{key} out of range")
                for key in ("gain", "cost"):
                    value = fuel.get(key)
                    if value is not None and not _is_finite_number(value):
                        problems.append(
                            f"collector frame country {tag!r}.fuel.{key} not number|null")
                    elif value is not None and abs(float(value)) > FUEL_MEASURE_ABS_MAX:
                        problems.append(
                            f"collector frame country {tag!r}.fuel.{key} out of range")
        production = row.get("production")
        if production is not None:
            if not isinstance(production, dict):
                problems.append(
                    f"collector frame country {tag!r}.production not dict|null")
            else:
                if present is not True:
                    problems.append(
                        f"collector frame country {tag!r}.production requires present=true")
                if not _has_valid_scalar_identity(row):
                    problems.append(
                        f"collector frame country {tag!r}.production requires identity.tag_db_id")
                if collector_object is None or not _valid_hex_pointer(collector_object):
                    problems.append(
                        f"collector frame country {tag!r}.production requires hex collector_object")
                if not any(key in production for key in PRODUCTION_PAYLOAD_KEYS):
                    problems.append(
                        f"collector frame country {tag!r}.production has no measured leaves")
                stockpile = production.get("stockpile")
                if stockpile is not None:
                    problems.extend(_validate_production_stockpile(tag, stockpile))
                stockpile_total = production.get("stockpile_total")
                if stockpile_total is not None and not _valid_signed_measure(
                    stockpile_total
                ):
                    problems.append(
                        f"collector frame country {tag!r}.production.stockpile_total "
                        "not finite number in range|null"
                    )
                for key in ("military_lines", "naval_lines"):
                    value = production.get(key)
                    if value is not None:
                        problems.extend(_validate_production_lines(tag, key, value))
                for key in ("military_factories", "naval_factories"):
                    value = production.get(key)
                    if value is not None and not _valid_bounded_int(
                        value, minimum=0, maximum=PRODUCTION_FACTORY_COUNT_MAX
                    ):
                        problems.append(
                            f"collector frame country {tag!r}.production.{key} "
                            "not int in range|null"
                        )
                construction_queue = production.get("construction_queue")
                if construction_queue is not None:
                    problems.extend(
                        _validate_construction_queue(tag, construction_queue))
        focus = row.get("focus")
        if focus is not None:
            if not isinstance(focus, dict):
                problems.append(f"collector frame country {tag!r}.focus not dict|null")
            else:
                if present is not True:
                    problems.append(
                        f"collector frame country {tag!r}.focus requires present=true")
                if not _has_valid_scalar_identity(row):
                    problems.append(
                        f"collector frame country {tag!r}.focus requires identity.tag_db_id")
                if collector_object is None or not _valid_hex_pointer(collector_object):
                    problems.append(
                        f"collector frame country {tag!r}.focus requires hex collector_object")
                if not any(key in focus for key in COUNTRY_FOCUS_PAYLOAD_KEYS):
                    problems.append(
                        f"collector frame country {tag!r}.focus has no measured leaves")
                for key in ("focus_tree", "current", "current_continuous"):
                    value = focus.get(key)
                    if value is not None and not _valid_focus_token(value):
                        problems.append(
                            f"collector frame country {tag!r}.focus.{key} not token|null")
                current = focus.get("current")
                current_continuous = focus.get("current_continuous")
                if current is not None and current_continuous is not None:
                    problems.append(
                        f"collector frame country {tag!r}.focus current and "
                        "current_continuous both set"
                    )
                completed = focus.get("completed")
                if completed is not None:
                    if (
                        not isinstance(completed, list)
                        or len(completed) > COUNTRY_FOCUS_MAX_COMPLETED
                    ):
                        problems.append(
                            f"collector frame country {tag!r}.focus.completed "
                            "not bounded list|null"
                        )
                    else:
                        for idx, token in enumerate(completed):
                            if not _valid_focus_token(token):
                                problems.append(
                                    f"collector frame country {tag!r}.focus."
                                    f"completed[{idx}] not token"
                                )
                                break
                completed_count = focus.get("completed_count")
                if completed_count is not None and not _valid_bounded_int(
                    completed_count,
                    minimum=0,
                    maximum=COUNTRY_FOCUS_MAX_COMPLETED,
                ):
                    problems.append(
                        f"collector frame country {tag!r}.focus.completed_count "
                        "not int in range|null"
                    )
                elif isinstance(completed, list) and completed_count is not None:
                    if completed_count != len(completed):
                        problems.append(
                            f"collector frame country {tag!r}.focus.completed_count "
                            "does not match completed length"
                        )
                progress = focus.get("progress")
                if progress is not None:
                    if not _is_finite_number(progress):
                        problems.append(
                            f"collector frame country {tag!r}.focus.progress "
                            "not number|null"
                        )
                    elif not (0.0 <= float(progress) <= COUNTRY_FOCUS_PROGRESS_MAX):
                        problems.append(
                            f"collector frame country {tag!r}.focus.progress out of range")
        research = row.get("research")
        if research is not None:
            if not isinstance(research, dict):
                problems.append(
                    f"collector frame country {tag!r}.research not dict|null")
            else:
                if present is not True:
                    problems.append(
                        f"collector frame country {tag!r}.research requires present=true")
                if not _has_valid_scalar_identity(row):
                    problems.append(
                        f"collector frame country {tag!r}.research requires identity.tag_db_id")
                if collector_object is None or not _valid_hex_pointer(collector_object):
                    problems.append(
                        f"collector frame country {tag!r}.research requires hex collector_object")
                if not any(key in research for key in COUNTRY_RESEARCH_PAYLOAD_KEYS):
                    problems.append(
                        f"collector frame country {tag!r}.research has no measured leaves")
                completed_tech_count = research.get("completed_tech_count")
                if completed_tech_count is not None and not _valid_bounded_int(
                    completed_tech_count,
                    minimum=0,
                    maximum=COUNTRY_RESEARCH_MAX_COMPLETED_TECHS,
                ):
                    problems.append(
                        f"collector frame country {tag!r}.research.completed_tech_count "
                        "not int in range|null"
                    )
                research_slot_count = research.get("research_slot_count")
                if research_slot_count is not None and not _valid_bounded_int(
                    research_slot_count,
                    minimum=0,
                    maximum=COUNTRY_RESEARCH_MAX_SLOTS,
                ):
                    problems.append(
                        f"collector frame country {tag!r}.research.research_slot_count "
                        "not int in range|null"
                    )
                active_research = research.get("active_research")
                if active_research is not None:
                    problems.extend(_validate_country_research_active_rows(
                        tag, active_research))
        politics = row.get("politics")
        if politics is not None:
            if not isinstance(politics, dict):
                problems.append(
                    f"collector frame country {tag!r}.politics not dict|null")
            else:
                if present is not True:
                    problems.append(
                        f"collector frame country {tag!r}.politics requires present=true")
                if not _has_valid_scalar_identity(row):
                    problems.append(
                        f"collector frame country {tag!r}.politics requires identity.tag_db_id")
                if collector_object is None or not _valid_hex_pointer(collector_object):
                    problems.append(
                        f"collector frame country {tag!r}.politics requires hex collector_object")
                if not any(key in politics for key in COUNTRY_POLITICS_PAYLOAD_KEYS):
                    problems.append(
                        f"collector frame country {tag!r}.politics has no measured leaves")
                popularities = politics.get("popularities")
                if popularities is not None:
                    if not isinstance(popularities, dict):
                        problems.append(
                            f"collector frame country {tag!r}.politics.popularities "
                            "not dict|null"
                        )
                    else:
                        for ideology, value in popularities.items():
                            if ideology not in COUNTRY_POLITICS_IDEOLOGIES:
                                problems.append(
                                    f"collector frame country {tag!r}.politics."
                                    f"popularities key {ideology!r} not canonical"
                                )
                                continue
                            if not _is_finite_number(value) or not (
                                0.0 <= float(value) <= 100.0001
                            ):
                                problems.append(
                                    f"collector frame country {tag!r}.politics."
                                    f"popularities.{ideology} not number in range"
                                )
                ruling_party = politics.get("ruling_party")
                if ruling_party is not None and ruling_party not in COUNTRY_POLITICS_IDEOLOGIES:
                    problems.append(
                        f"collector frame country {tag!r}.politics.ruling_party "
                        "not ideology|null"
                    )
                leader = politics.get("leader")
                if leader is not None:
                    problems.extend(_validate_country_leader_row(tag, leader))
                ideas = politics.get("ideas")
                if ideas is not None:
                    if not isinstance(ideas, list) or len(ideas) > COUNTRY_POLITICS_MAX_IDEAS:
                        problems.append(
                            f"collector frame country {tag!r}.politics.ideas "
                            "not bounded list|null"
                        )
                    else:
                        for idx, idea in enumerate(ideas):
                            if not _valid_politics_token(idea):
                                problems.append(
                                    f"collector frame country {tag!r}.politics."
                                    f"ideas[{idx}] not token"
                                )
                                break
                ideas_other = politics.get("ideas_other")
                if ideas_other is not None:
                    if (
                        not isinstance(ideas_other, list)
                        or len(ideas_other) > COUNTRY_POLITICS_MAX_IDEAS
                    ):
                        problems.append(
                            f"collector frame country {tag!r}.politics.ideas_other "
                            "not bounded list|null"
                        )
                    else:
                        for idx, token in enumerate(ideas_other):
                            if not _valid_politics_token(token):
                                problems.append(
                                    f"collector frame country {tag!r}.politics."
                                    f"ideas_other[{idx}] not token"
                                )
                                break
                national_spirits = politics.get("national_spirits")
                if national_spirits is not None:
                    if (
                        not isinstance(national_spirits, list)
                        or len(national_spirits) > COUNTRY_POLITICS_MAX_IDEAS
                    ):
                        problems.append(
                            f"collector frame country {tag!r}.politics.national_spirits "
                            "not bounded list|null"
                        )
                    else:
                        for idx, token in enumerate(national_spirits):
                            if not _valid_politics_token(token):
                                problems.append(
                                    f"collector frame country {tag!r}.politics."
                                    f"national_spirits[{idx}] not token"
                                )
                                break
                laws = politics.get("laws")
                if laws is not None:
                    if not isinstance(laws, dict) or len(laws) > COUNTRY_POLITICS_MAX_LAWS:
                        problems.append(
                            f"collector frame country {tag!r}.politics.laws "
                            "not bounded dict|null"
                        )
                    else:
                        for group, token in laws.items():
                            if not _valid_politics_token(group):
                                problems.append(
                                    f"collector frame country {tag!r}.politics."
                                    f"laws key {group!r} not token"
                                )
                                continue
                            if not _valid_politics_token(token):
                                problems.append(
                                    f"collector frame country {tag!r}.politics."
                                    f"laws.{group} not token"
                                )
                idea_groups = politics.get("idea_groups")
                if idea_groups is not None:
                    problems.extend(_validate_politics_token_map(
                        tag, idea_groups, "idea_groups"))
                advisor_ideas = politics.get("advisor_ideas")
                if advisor_ideas is not None:
                    problems.extend(_validate_politics_token_map(
                        tag, advisor_ideas, "advisor_ideas"))
                for key, minimum, maximum in (
                    ("idea_count", 0, COUNTRY_POLITICS_MAX_IDEAS),
                    ("election_frequency", 1, 600),
                    ("party_count", 1, 16),
                ):
                    value = politics.get(key)
                    if value is not None and not _valid_bounded_int(
                        value, minimum=minimum, maximum=maximum
                    ):
                        problems.append(
                            f"collector frame country {tag!r}.politics.{key} "
                            "not int in range|null"
                        )
                low_confidence = politics.get("low_confidence")
                if low_confidence is not None and not isinstance(low_confidence, bool):
                    problems.append(
                        f"collector frame country {tag!r}.politics.low_confidence "
                        "not bool|null"
                    )
                last_election = politics.get("last_election")
                if last_election is not None and not _valid_datehour_string(last_election):
                    problems.append(
                        f"collector frame country {tag!r}.politics.last_election "
                        "not datehour|null"
                    )
                elections_allowed = politics.get("elections_allowed")
                if elections_allowed is not None and not isinstance(elections_allowed, bool):
                    problems.append(
                        f"collector frame country {tag!r}.politics.elections_allowed "
                        "not bool|null"
                    )
                appointed_advisors = politics.get("appointed_advisors")
                if appointed_advisors is not None:
                    if (
                        not isinstance(appointed_advisors, list)
                        or len(appointed_advisors) > COUNTRY_POLITICS_MAX_IDEAS
                    ):
                        problems.append(
                            f"collector frame country {tag!r}.politics."
                            "appointed_advisors not bounded list|null"
                        )
                    else:
                        for idx, token in enumerate(appointed_advisors):
                            if not _valid_politics_token(token):
                                problems.append(
                                    f"collector frame country {tag!r}.politics."
                                    f"appointed_advisors[{idx}] not token"
                                )
                                break
                appointed_advisor_details = politics.get("appointed_advisor_details")
                if appointed_advisor_details is not None:
                    problems.extend(_validate_appointed_advisor_details(
                        tag, appointed_advisor_details))
                appointed_advisor_slots = politics.get("appointed_advisor_slots")
                if appointed_advisor_slots is not None:
                    problems.extend(_validate_appointed_advisor_token_map(
                        tag, appointed_advisor_slots, "appointed_advisor_slots"))
                appointed_advisor_ledgers = politics.get("appointed_advisor_ledgers")
                if appointed_advisor_ledgers is not None:
                    problems.extend(_validate_appointed_advisor_token_map(
                        tag, appointed_advisor_ledgers, "appointed_advisor_ledgers"))
        decisions = row.get("decisions")
        if decisions is not None:
            problems.extend(_validate_country_decisions_row(
                tag,
                row,
                decisions,
                present=present,
                collector_object=collector_object,
            ))
        diplomacy = row.get("diplomacy")
        if diplomacy is not None:
            problems.extend(_validate_country_diplomacy_row(
                tag,
                row,
                diplomacy,
                present=present,
                collector_object=collector_object,
            ))
    return problems


def _validate_country_leader_row(tag: str, value: Any) -> List[str]:
    problems: List[str] = []
    prefix = f"collector frame country {tag!r}.politics.leader"
    if not isinstance(value, dict):
        return [f"{prefix} not dict|null"]
    for key in value:
        if key not in COUNTRY_LEADER_PAYLOAD_KEYS:
            problems.append(f"{prefix}.{key} unsupported")
    char_id = value.get("char_id")
    if not _valid_bounded_int(
        char_id,
        minimum=1,
        maximum=COUNTRY_LEADER_CHAR_ID_MAX,
    ):
        problems.append(f"{prefix}.char_id not positive int in range")
    name = value.get("name")
    if name is not None and (
        not isinstance(name, str)
        or not (0 < len(name) <= COUNTRY_LEADER_NAME_MAX_LENGTH)
    ):
        problems.append(f"{prefix}.name not bounded str|null")
    token = value.get("token")
    if token is not None and not _valid_politics_token(token):
        problems.append(f"{prefix}.token not token|null")
    ideology_group = value.get("ideology_group")
    if ideology_group is not None and ideology_group not in COUNTRY_POLITICS_IDEOLOGIES:
        problems.append(f"{prefix}.ideology_group not ideology|null")
    ambiguous = value.get("ambiguous")
    if ambiguous is not None and not isinstance(ambiguous, bool):
        problems.append(f"{prefix}.ambiguous not bool|null")
    source = value.get("source")
    if source is not None and source != RAW_SOURCE:
        problems.append(f"{prefix}.source {source!r} != {RAW_SOURCE!r}")
    return problems


def _validate_country_research_active_rows(tag: str, value: Any) -> List[str]:
    problems: List[str] = []
    if not isinstance(value, list) or len(value) > COUNTRY_RESEARCH_MAX_ACTIVE:
        return [
            f"collector frame country {tag!r}.research.active_research "
            "not bounded list|null"
        ]
    allowed = {
        "slot_index",
        "tech_handle_raw",
        "tech_def_index_raw",
        "progress_raw",
        "tech_name",
        "progress_units",
        "live_progress_raw",
        "live_progress_units",
        "base_required_points_raw",
        "base_required_points_units",
    }
    for idx, item in enumerate(value):
        if not isinstance(item, dict):
            problems.append(
                f"collector frame country {tag!r}.research."
                f"active_research[{idx}] not dict"
            )
            continue
        for key in item:
            if key not in allowed:
                problems.append(
                    f"collector frame country {tag!r}.research."
                    f"active_research[{idx}].{key} unsupported"
                )
        slot_index = item.get("slot_index")
        if slot_index is not None and not _valid_bounded_int(
            slot_index,
            minimum=0,
            maximum=COUNTRY_RESEARCH_MAX_SLOTS - 1,
        ):
            problems.append(
                f"collector frame country {tag!r}.research."
                f"active_research[{idx}].slot_index not int in range|null"
            )
        for key in (
            "tech_handle_raw",
            "tech_def_index_raw",
            "progress_raw",
            "live_progress_raw",
            "base_required_points_raw",
        ):
            value_raw = item.get(key)
            if value_raw is not None and not _valid_bounded_int(
                value_raw,
                minimum=0,
                maximum=COUNTRY_RESEARCH_RAW_VALUE_MAX,
            ):
                problems.append(
                    f"collector frame country {tag!r}.research."
                    f"active_research[{idx}].{key} not int in range|null"
                )
        tech_name = item.get("tech_name")
        if tech_name is not None and not _valid_research_token(tech_name):
            problems.append(
                f"collector frame country {tag!r}.research."
                f"active_research[{idx}].tech_name not script token|null"
            )
        progress_units = item.get("progress_units")
        if progress_units is not None and not (
            _is_finite_number(progress_units)
            and 0.0 <= float(progress_units) <= COUNTRY_RESEARCH_PROGRESS_UNITS_MAX
        ):
            problems.append(
                f"collector frame country {tag!r}.research."
                f"active_research[{idx}].progress_units not finite number in range|null"
            )
        live_progress_units = item.get("live_progress_units")
        if live_progress_units is not None and not (
            _is_finite_number(live_progress_units)
            and 0.0 <= float(live_progress_units)
            <= COUNTRY_RESEARCH_PROGRESS_UNITS_MAX
        ):
            problems.append(
                f"collector frame country {tag!r}.research."
                f"active_research[{idx}].live_progress_units not finite number in range|null"
            )
        base_required_points_units = item.get("base_required_points_units")
        if base_required_points_units is not None and not (
            _is_finite_number(base_required_points_units)
            and 0.0 < float(base_required_points_units)
            <= COUNTRY_RESEARCH_PROGRESS_UNITS_MAX
        ):
            problems.append(
                f"collector frame country {tag!r}.research."
                "active_research[%d].base_required_points_units "
                "not finite positive number in range|null" % idx
            )
    return problems


def _validate_appointed_advisor_details(tag: str, value: Any) -> List[str]:
    problems: List[str] = []
    if not isinstance(value, list) or len(value) > COUNTRY_POLITICS_MAX_IDEAS:
        return [
            f"collector frame country {tag!r}.politics.appointed_advisor_details "
            "not bounded list|null"
        ]
    for idx, item in enumerate(value):
        if not isinstance(item, dict):
            problems.append(
                f"collector frame country {tag!r}.politics."
                f"appointed_advisor_details[{idx}] not dict"
            )
            continue
        token = item.get("idea_token")
        if not _valid_politics_token(token):
            problems.append(
                f"collector frame country {tag!r}.politics."
                f"appointed_advisor_details[{idx}].idea_token not token"
            )
        for key in ("slot", "ledger", "character"):
            meta = item.get(key)
            if meta is not None and not _valid_politics_token(meta):
                problems.append(
                    f"collector frame country {tag!r}.politics."
                    f"appointed_advisor_details[{idx}].{key} not token|null"
                )
    return problems


def _validate_politics_token_map(tag: str, value: Any, field: str) -> List[str]:
    problems: List[str] = []
    if not isinstance(value, dict) or len(value) > COUNTRY_POLITICS_MAX_IDEAS:
        return [
            f"collector frame country {tag!r}.politics.{field} "
            "not bounded dict|null"
        ]
    for group, tokens in value.items():
        if not _valid_politics_token(group):
            problems.append(
                f"collector frame country {tag!r}.politics."
                f"{field} key {group!r} not token"
            )
            continue
        if (
            not isinstance(tokens, list)
            or len(tokens) > COUNTRY_POLITICS_MAX_IDEAS
        ):
            problems.append(
                f"collector frame country {tag!r}.politics."
                f"{field}.{group} not bounded list"
            )
            continue
        for idx, token in enumerate(tokens):
            if not _valid_politics_token(token):
                problems.append(
                    f"collector frame country {tag!r}.politics."
                    f"{field}.{group}[{idx}] not token"
                )
                break
    return problems


def _validate_appointed_advisor_token_map(
    tag: str,
    value: Any,
    field: str,
) -> List[str]:
    problems: List[str] = []
    if not isinstance(value, dict) or len(value) > COUNTRY_POLITICS_MAX_IDEAS:
        return [
            f"collector frame country {tag!r}.politics.{field} "
            "not bounded dict|null"
        ]
    for bucket, tokens in value.items():
        if not _valid_politics_token(bucket):
            problems.append(
                f"collector frame country {tag!r}.politics."
                f"{field} key {bucket!r} not token"
            )
            continue
        if not isinstance(tokens, list) or len(tokens) > COUNTRY_POLITICS_MAX_IDEAS:
            problems.append(
                f"collector frame country {tag!r}.politics."
                f"{field}.{bucket} not bounded list"
            )
            continue
        for idx, token in enumerate(tokens):
            if not _valid_politics_token(token):
                problems.append(
                    f"collector frame country {tag!r}.politics."
                    f"{field}.{bucket}[{idx}] not token"
                )
                break
    return problems


def _validate_country_decisions_row(
    tag: str,
    row: Dict[str, Any],
    decisions: Any,
    *,
    present: Any,
    collector_object: Any,
) -> List[str]:
    problems: List[str] = []
    if not isinstance(decisions, dict):
        return [f"collector frame country {tag!r}.decisions not dict|null"]
    if present is not True:
        problems.append(f"collector frame country {tag!r}.decisions requires present=true")
    if not _has_valid_scalar_identity(row):
        problems.append(
            f"collector frame country {tag!r}.decisions requires identity.tag_db_id")
    if collector_object is None or not _valid_hex_pointer(collector_object):
        problems.append(
            f"collector frame country {tag!r}.decisions requires hex collector_object")
    if not any(key in decisions for key in COUNTRY_DECISIONS_PAYLOAD_KEYS):
        problems.append(
            f"collector frame country {tag!r}.decisions has no measured leaves")

    status_present = decisions.get("status_present")
    if status_present is not None:
        if (
            not isinstance(status_present, dict)
            or len(status_present) > COUNTRY_DECISIONS_MAX_STATUS_KEYS
        ):
            problems.append(
                f"collector frame country {tag!r}.decisions.status_present "
                "not bounded dict|null"
            )
        else:
            for key, value in status_present.items():
                if not _valid_decision_token(key):
                    problems.append(
                        f"collector frame country {tag!r}.decisions."
                        f"status_present key {key!r} not token"
                    )
                    continue
                if not _valid_bounded_int(
                    value,
                    minimum=0,
                    maximum=COUNTRY_DECISIONS_STATUS_COUNT_MAX,
                ):
                    problems.append(
                        f"collector frame country {tag!r}.decisions."
                        f"status_present.{key} not int in range"
                    )

    timed_live = decisions.get("timed_live")
    if timed_live is not None:
        if (
            not isinstance(timed_live, list)
            or len(timed_live) > COUNTRY_DECISIONS_MAX_TIMED
        ):
            problems.append(
                f"collector frame country {tag!r}.decisions.timed_live "
                "not bounded list|null"
            )
        else:
            for idx, item in enumerate(timed_live):
                problems.extend(_validate_country_decision_timed_row(tag, idx, item))
                if problems and any(
                    f"timed_live[{idx}]" in problem for problem in problems
                ):
                    break
    return problems


def _validate_country_decision_timed_row(
    tag: str,
    idx: int,
    item: Any,
) -> List[str]:
    prefix = f"collector frame country {tag!r}.decisions.timed_live[{idx}]"
    problems: List[str] = []
    if not isinstance(item, dict):
        return [f"{prefix} not dict"]
    decision = item.get("decision")
    if not _valid_decision_token(decision):
        problems.append(f"{prefix}.decision not token")
    days = item.get("days")
    if not _valid_bounded_int(days, minimum=0, maximum=COUNTRY_DECISIONS_DAYS_MAX):
        problems.append(f"{prefix}.days not int in range")
    state = item.get("state")
    if not _valid_bounded_int(state, minimum=0, maximum=COUNTRY_DECISIONS_STATE_MAX):
        problems.append(f"{prefix}.state not int in range")
    state_name = item.get("state_name")
    if state_name is not None and not _valid_decision_token(state_name):
        problems.append(f"{prefix}.state_name not token|null")
    return problems


def _validate_country_diplomacy_row(
    tag: str,
    row: Dict[str, Any],
    diplomacy: Any,
    *,
    present: Any,
    collector_object: Any,
) -> List[str]:
    problems: List[str] = []
    if not isinstance(diplomacy, dict):
        return [f"collector frame country {tag!r}.diplomacy not dict|null"]
    if present is not True:
        problems.append(f"collector frame country {tag!r}.diplomacy requires present=true")
    if not _has_valid_scalar_identity(row):
        problems.append(
            f"collector frame country {tag!r}.diplomacy requires identity.tag_db_id")
    if collector_object is None or not _valid_hex_pointer(collector_object):
        problems.append(
            f"collector frame country {tag!r}.diplomacy requires hex collector_object")
    if not any(key in diplomacy for key in COUNTRY_DIPLOMACY_PAYLOAD_KEYS):
        problems.append(
            f"collector frame country {tag!r}.diplomacy has no measured leaves")
    for key in diplomacy:
        if key not in COUNTRY_DIPLOMACY_PAYLOAD_KEYS:
            problems.append(
                f"collector frame country {tag!r}.diplomacy key {key!r} unsupported")
    if "faction_group" in diplomacy and not _valid_bounded_int(
        diplomacy.get("faction_group"),
        minimum=-COUNTRY_DIPLOMACY_FACTION_GROUP_ABS_MAX,
        maximum=COUNTRY_DIPLOMACY_FACTION_GROUP_ABS_MAX,
    ):
        problems.append(
            f"collector frame country {tag!r}.diplomacy.faction_group "
            "not int in range"
        )
    at_war = diplomacy.get("at_war")
    if "at_war" in diplomacy:
        if not isinstance(at_war, list):
            problems.append(
                f"collector frame country {tag!r}.diplomacy.at_war not list")
        elif not all(schema.valid_tag(enemy) for enemy in at_war):
            problems.append(
                f"collector frame country {tag!r}.diplomacy.at_war contains non-TAG")
    if "casualties" in diplomacy and not _valid_bounded_int(
        diplomacy.get("casualties"),
        minimum=0,
        maximum=COUNTRY_DIPLOMACY_CASUALTIES_MAX,
    ):
        problems.append(
            f"collector frame country {tag!r}.diplomacy.casualties "
            "not nonnegative int in range"
        )
    return problems


def _validate_diplomacy_live_sidecar(diplomacy_live: Dict[str, Any]) -> List[str]:
    problems: List[str] = []
    for key in diplomacy_live:
        if key not in DIPLOMACY_LIVE_PAYLOAD_KEYS:
            problems.append(f"collector frame diplomacy_live key {key!r} unsupported")
    war_relations = diplomacy_live.get("war_relations")
    if war_relations is not None:
        if not isinstance(war_relations, list):
            problems.append("collector frame diplomacy_live.war_relations not list|null")
        elif len(war_relations) > DIPLOMACY_LIVE_MAX_WAR_RELATIONS:
            problems.append("collector frame diplomacy_live.war_relations too many rows")
        else:
            for idx, row in enumerate(war_relations):
                if not isinstance(row, dict):
                    problems.append(
                        f"collector frame diplomacy_live.war_relations[{idx}] not object")
                    continue
                allowed = {
                    "first", "second", "first_casualties", "second_casualties"}
                for key in row:
                    if key not in allowed:
                        problems.append(
                            "collector frame diplomacy_live."
                            f"war_relations[{idx}] key {key!r} unsupported")
                first = row.get("first")
                second = row.get("second")
                if not schema.valid_tag(first):
                    problems.append(
                        f"collector frame diplomacy_live.war_relations[{idx}].first not TAG")
                if not schema.valid_tag(second):
                    problems.append(
                        f"collector frame diplomacy_live.war_relations[{idx}].second not TAG")
                for key in ("first_casualties", "second_casualties"):
                    if not _valid_bounded_int(
                        row.get(key),
                        minimum=0,
                        maximum=COUNTRY_DIPLOMACY_CASUALTIES_MAX,
                    ):
                        problems.append(
                            "collector frame diplomacy_live."
                            f"war_relations[{idx}].{key} not nonnegative int in range")
    faction_members = diplomacy_live.get("faction_members")
    if faction_members is not None:
        if not isinstance(faction_members, dict):
            problems.append("collector frame diplomacy_live.faction_members not dict|null")
        else:
            for group, members in faction_members.items():
                try:
                    int(str(group))
                except (TypeError, ValueError):
                    problems.append(
                        "collector frame diplomacy_live.faction_members group "
                        f"{group!r} not int-like")
                    continue
                if not isinstance(members, list):
                    problems.append(
                        "collector frame diplomacy_live.faction_members "
                        f"{group!r} not list")
                elif not all(schema.valid_tag(tag) for tag in members):
                    problems.append(
                        "collector frame diplomacy_live.faction_members "
                        f"{group!r} contains non-TAG")
    subtype_warning = diplomacy_live.get("subtype_warning")
    if subtype_warning is not None and not isinstance(subtype_warning, dict):
        problems.append("collector frame diplomacy_live.subtype_warning not dict|null")
    source = diplomacy_live.get("source")
    if source is not None and source not in {"C", "M"}:
        problems.append("collector frame diplomacy_live.source not C|M|null")
    return problems


def _has_valid_scalar_identity(row: Dict[str, Any]) -> bool:
    identity = row.get("identity")
    if not isinstance(identity, dict):
        return False
    tag_db_id = identity.get("tag_db_id")
    return isinstance(tag_db_id, int) and 1 <= tag_db_id <= 1024


def _valid_hex_pointer(value: Any) -> bool:
    if not isinstance(value, str) or not value.startswith("0x"):
        return False
    try:
        return int(value[2:], 16) > 0
    except ValueError:
        return False


def _valid_country_misc_token(value: Any) -> bool:
    if not isinstance(value, str):
        return False
    if not (0 < len(value) <= COUNTRY_MISC_TOKEN_MAX_LENGTH):
        return False
    return all(
        ("A" <= ch <= "Z")
        or ("a" <= ch <= "z")
        or ("0" <= ch <= "9")
        or ch in "_-"
        for ch in value
    )


def _valid_politics_token(value: Any) -> bool:
    if not isinstance(value, str):
        return False
    if not (0 < len(value) <= COUNTRY_POLITICS_MAX_TOKEN_LENGTH):
        return False
    return all(
        ("A" <= ch <= "Z")
        or ("a" <= ch <= "z")
        or ("0" <= ch <= "9")
        or ch in "_-.:"
        for ch in value
    )


def _valid_focus_token(value: Any) -> bool:
    if not isinstance(value, str):
        return False
    if not (0 < len(value) <= COUNTRY_FOCUS_MAX_TOKEN_LENGTH):
        return False
    return all(
        ("A" <= ch <= "Z")
        or ("a" <= ch <= "z")
        or ("0" <= ch <= "9")
        or ch in "_-.:"
        for ch in value
    )


def _valid_research_token(value: Any) -> bool:
    if not isinstance(value, str):
        return False
    if not (0 < len(value) <= COUNTRY_RESEARCH_TOKEN_MAX_LENGTH):
        return False
    return all(
        ("A" <= ch <= "Z")
        or ("a" <= ch <= "z")
        or ("0" <= ch <= "9")
        or ch in "_-.:"
        for ch in value
    )


def _valid_decision_token(value: Any) -> bool:
    if not isinstance(value, str):
        return False
    if not (0 < len(value) <= COUNTRY_DECISIONS_TOKEN_MAX_LENGTH):
        return False
    return all(
        ("A" <= ch <= "Z")
        or ("a" <= ch <= "z")
        or ("0" <= ch <= "9")
        or ch in "_-.:"
        for ch in value
    )


def _valid_special_project_token(value: Any) -> bool:
    if not isinstance(value, str):
        return False
    if not (0 < len(value) <= SPECIAL_PROJECT_TOKEN_MAX_LENGTH):
        return False
    return all(
        ("A" <= ch <= "Z")
        or ("a" <= ch <= "z")
        or ("0" <= ch <= "9")
        or ch in "_-.:/"
        for ch in value
    )


def _valid_special_project_display(value: Any) -> bool:
    return (
        isinstance(value, str)
        and 0 < len(value) <= SPECIAL_PROJECT_DISPLAY_MAX_LENGTH
        and all(32 <= ord(ch) <= 126 for ch in value)
    )


def _valid_datehour_string(value: Any) -> bool:
    if not isinstance(value, str):
        return False
    parts = value.split(".")
    if len(parts) != 4:
        return False
    try:
        year, month, day, hour = [int(part) for part in parts]
    except ValueError:
        return False
    return (
        0 <= year <= 9999
        and 1 <= month <= 12
        and 1 <= day <= 31
        and 0 <= hour <= 23
    )


def _is_finite_number(value: Any) -> bool:
    return (
        not isinstance(value, bool)
        and isinstance(value, (int, float))
        and math.isfinite(float(value))
    )


def _valid_signed_measure(value: Any) -> bool:
    return (
        _is_finite_number(value)
        and abs(float(value)) <= PRODUCTION_FIXED_POINT_ABS_MAX
    )


def _valid_nonnegative_measure(value: Any) -> bool:
    return (
        _is_finite_number(value)
        and 0.0 <= float(value) <= PRODUCTION_FIXED_POINT_ABS_MAX
    )


def _valid_bounded_int(value: Any, *, minimum: int, maximum: int) -> bool:
    return (
        not isinstance(value, bool)
        and isinstance(value, int)
        and minimum <= value <= maximum
    )


def _validate_production_stockpile(tag: str, stockpile: Any) -> List[str]:
    problems: List[str] = []
    if not isinstance(stockpile, dict):
        return [f"collector frame country {tag!r}.production.stockpile not dict|null"]
    if len(stockpile) > PRODUCTION_STOCKPILE_MAX_ENTRIES:
        problems.append(
            f"collector frame country {tag!r}.production.stockpile too many entries")
    for variant_id, amount in stockpile.items():
        if not _valid_positive_int_key(variant_id, max_value=PRODUCTION_VARIANT_ID_MAX):
            problems.append(
                f"collector frame country {tag!r}.production.stockpile key "
                f"{variant_id!r} not valid positive id"
            )
        if not _valid_signed_measure(amount):
            problems.append(
                f"collector frame country {tag!r}.production.stockpile"
                f"[{variant_id!r}] not finite number in range"
            )
    return problems


def _validate_production_lines(tag: str, key: str, value: Any) -> List[str]:
    problems: List[str] = []
    if not isinstance(value, list):
        return [f"collector frame country {tag!r}.production.{key} not list|null"]
    if len(value) > PRODUCTION_LINE_MAX_ENTRIES:
        problems.append(
            f"collector frame country {tag!r}.production.{key} too many entries")
    for i, line in enumerate(value):
        prefix = f"collector frame country {tag!r}.production.{key}[{i}]"
        if not isinstance(line, dict):
            problems.append(f"{prefix} not a dict")
            continue
        line_id = line.get("id")
        if line_id is None:
            problems.append(f"{prefix}.id missing")
        elif not _valid_bounded_int(
            line_id, minimum=0, maximum=PRODUCTION_LINE_ID_MAX
        ):
            problems.append(f"{prefix}.id not int in range")
        active = line.get("active_factories")
        if active is None:
            problems.append(f"{prefix}.active_factories missing")
        elif not _valid_bounded_int(
            active, minimum=0, maximum=PRODUCTION_FACTORY_COUNT_MAX
        ):
            problems.append(f"{prefix}.active_factories not int in range")
        for field in ("cost", "speed", "produced"):
            measured = line.get(field)
            if measured is None:
                problems.append(f"{prefix}.{field} missing")
            elif not _valid_nonnegative_measure(measured):
                problems.append(f"{prefix}.{field} not finite nonnegative number in range")
        amount = line.get("amount")
        if amount is None:
            problems.append(f"{prefix}.amount missing")
        elif not (
            amount == -1
            or _valid_bounded_int(
                amount, minimum=0, maximum=PRODUCTION_LINE_AMOUNT_MAX
            )
        ):
            problems.append(f"{prefix}.amount not -1 or nonnegative int in range")
    return problems


def _validate_construction_queue(tag: str, value: Any) -> List[str]:
    problems: List[str] = []
    if not isinstance(value, list):
        return [
            f"collector frame country {tag!r}.production.construction_queue "
            "not list|null"
        ]
    if len(value) > PRODUCTION_LINE_MAX_ENTRIES:
        problems.append(
            f"collector frame country {tag!r}.production.construction_queue "
            "too many entries"
        )
    for i, line in enumerate(value):
        prefix = f"collector frame country {tag!r}.production.construction_queue[{i}]"
        if not isinstance(line, dict):
            problems.append(f"{prefix} not a dict")
            continue
        line_id = line.get("id")
        if line_id is None:
            problems.append(f"{prefix}.id missing")
        elif not _valid_bounded_int(
            line_id, minimum=0, maximum=PRODUCTION_LINE_ID_MAX
        ):
            problems.append(f"{prefix}.id not int in range")
        template_id = line.get("template_id")
        if template_id is not None and not _valid_bounded_int(
            template_id,
            minimum=0,
            maximum=PRODUCTION_CONSTRUCTION_TEMPLATE_ID_MAX,
        ):
            problems.append(f"{prefix}.template_id not int|null in range")
        active = line.get("active_factories")
        if active is None:
            problems.append(f"{prefix}.active_factories missing")
        elif not _valid_bounded_int(
            active, minimum=0, maximum=PRODUCTION_FACTORY_COUNT_MAX
        ):
            problems.append(f"{prefix}.active_factories not int in range")
        for field in ("cost", "produced"):
            measured = line.get(field)
            if measured is None:
                problems.append(f"{prefix}.{field} missing")
            elif not _valid_nonnegative_measure(measured):
                problems.append(f"{prefix}.{field} not finite nonnegative number in range")
        priority = line.get("priority")
        if priority is None:
            problems.append(f"{prefix}.priority missing")
        elif not _valid_bounded_int(
            priority, minimum=0, maximum=PRODUCTION_LINE_MAX_ENTRIES
        ):
            problems.append(f"{prefix}.priority not int in range")
    return problems


def _validate_state_rows(states: Dict[str, Any]) -> List[str]:
    problems: List[str] = []
    for sid, row in states.items():
        if not _valid_positive_int_key(sid, max_value=100_000):
            problems.append(f"collector frame state key {sid!r} not valid positive id")
            continue
        if not isinstance(row, dict):
            problems.append(f"collector frame state {sid!r} not a dict")
            continue
        for key in ("owner", "controller"):
            value = row.get(key)
            if value is not None and not schema.valid_tag(value):
                problems.append(
                    f"collector frame state {sid!r}.{key} not TAG|null")
        buildings = row.get("buildings")
        if buildings is not None:
            problems.extend(_validate_building_map(
                buildings,
                f"collector frame state {sid!r}.buildings",
            ))
    return problems


def _validate_province_rows(provinces: Dict[str, Any]) -> List[str]:
    problems: List[str] = []
    for pid, row in provinces.items():
        if not _valid_positive_int_key(pid, max_value=1_000_000):
            problems.append(f"collector frame province key {pid!r} not valid positive id")
            continue
        if not isinstance(row, dict):
            problems.append(f"collector frame province {pid!r} not a dict")
            continue
        controller = row.get("controller")
        if controller is not None and not schema.valid_tag(controller):
            problems.append(
                f"collector frame province {pid!r}.controller not TAG|null")
        controller_tag_db_id = row.get("controller_tag_db_id")
        if controller_tag_db_id is not None:
            if (
                isinstance(controller_tag_db_id, bool)
                or not isinstance(controller_tag_db_id, int)
            ):
                problems.append(
                    f"collector frame province {pid!r}.controller_tag_db_id not int|null")
            elif controller_tag_db_id < 0 or controller_tag_db_id > 100_000:
                problems.append(
                    f"collector frame province {pid!r}.controller_tag_db_id out of range")
        state_id = row.get("state_id")
        if state_id is not None:
            if isinstance(state_id, bool) or not isinstance(state_id, int):
                problems.append(
                    f"collector frame province {pid!r}.state_id not int|null")
            elif state_id <= 0 or state_id > 100_000:
                problems.append(
                    f"collector frame province {pid!r}.state_id out of range")
        buildings = row.get("buildings")
        if buildings is not None:
            problems.extend(_validate_building_map(
                buildings,
                f"collector frame province {pid!r}.buildings",
            ))
    return problems


def _validate_building_map(buildings: Any, prefix: str) -> List[str]:
    if not isinstance(buildings, dict):
        return [f"{prefix} not dict|null"]
    problems: List[str] = []
    for building, level in buildings.items():
        if not schema.valid_script_token(building):
            problems.append(f"{prefix} key {building!r} not script token")
        if not _valid_bounded_int(level, minimum=0, maximum=BUILDING_LEVEL_MAX):
            problems.append(f"{prefix}.{building!s} not int in range")
    return problems


def _validate_special_projects(special_projects: Dict[str, Any]) -> List[str]:
    problems: List[str] = []
    for key in special_projects:
        if key not in SPECIAL_PROJECTS_PAYLOAD_KEYS:
            problems.append(f"collector frame special_projects key {key!r} unsupported")
    if _contains_forbidden_raw_key(special_projects):
        problems.append(
            "collector frame special_projects contains raw pointer/tag-db diagnostic key")

    source = special_projects.get("source")
    if source is not None and source != "C":
        problems.append(f"collector frame special_projects.source {source!r} != 'C'")

    facilities = special_projects.get("facilities")
    projects = special_projects.get("projects")
    scientists = special_projects.get("scientists")
    problems.extend(_validate_special_project_facilities(facilities))
    problems.extend(_validate_special_project_rows(projects))
    problems.extend(_validate_special_project_scientists(scientists))

    count = special_projects.get("count")
    if count is not None:
        if not isinstance(count, dict):
            problems.append("collector frame special_projects.count not dict|null")
        else:
            for key in count:
                if key not in {"facilities", "projects", "scientists"}:
                    problems.append(
                        f"collector frame special_projects.count key {key!r} unsupported")
            for key, rows in (
                ("facilities", facilities),
                ("projects", projects),
                ("scientists", scientists),
            ):
                value = count.get(key)
                if value is not None and not _valid_bounded_int(
                    value,
                    minimum=0,
                    maximum=SPECIAL_PROJECT_MAX_PROJECTS,
                ):
                    problems.append(
                        f"collector frame special_projects.count.{key} not int in range|null")
                elif isinstance(rows, list) and value is not None and value != len(rows):
                    problems.append(
                        f"collector frame special_projects.count.{key} "
                        f"{value!r} != {len(rows)}"
                    )
    if _has_special_projects_payload(special_projects) and source != "C":
        problems.append("collector frame special_projects payload requires source='C'")
    return problems


def _validate_special_project_facilities(value: Any) -> List[str]:
    if value is None:
        return []
    if not isinstance(value, list):
        return ["collector frame special_projects.facilities not list|null"]
    problems: List[str] = []
    if len(value) > SPECIAL_PROJECT_MAX_FACILITIES:
        problems.append("collector frame special_projects.facilities too many entries")
    for idx, row in enumerate(value):
        prefix = f"collector frame special_projects.facilities[{idx}]"
        if not isinstance(row, dict):
            problems.append(f"{prefix} not dict")
            continue
        for key in row:
            if key not in SPECIAL_PROJECT_FACILITY_KEYS:
                problems.append(f"{prefix}.{key} unsupported")
        for key in ("id", "province"):
            if not _valid_bounded_int(
                row.get(key),
                minimum=1,
                maximum=(
                    SPECIAL_PROJECT_PROVINCE_MAX
                    if key == "province" else SPECIAL_PROJECT_ID_MAX
                ),
            ):
                problems.append(f"{prefix}.{key} not positive int in range")
        active_project_handle = row.get("active_project_handle")
        if not _valid_bounded_int(
            active_project_handle,
            minimum=0,
            maximum=SPECIAL_PROJECT_HANDLE_MAX,
        ):
            problems.append(f"{prefix}.active_project_handle not int in range")
        active = row.get("active")
        if not isinstance(active, bool):
            problems.append(f"{prefix}.active not bool")
        scientist_char_id = row.get("scientist_char_id")
        if scientist_char_id is not None and not _valid_bounded_int(
            scientist_char_id,
            minimum=1,
            maximum=SPECIAL_PROJECT_ID_MAX,
        ):
            problems.append(f"{prefix}.scientist_char_id not positive int|null")
        unread = row.get("unread_prototype_rewards")
        if not _valid_bounded_int(
            unread,
            minimum=0,
            maximum=SPECIAL_PROJECT_UNREAD_REWARDS_MAX,
        ):
            problems.append(f"{prefix}.unread_prototype_rewards not int in range")
        building = row.get("building")
        if building is not None and not _valid_special_project_token(building):
            problems.append(f"{prefix}.building not token|null")
    return problems


def _validate_special_project_rows(value: Any) -> List[str]:
    if value is None:
        return []
    if not isinstance(value, list):
        return ["collector frame special_projects.projects not list|null"]
    problems: List[str] = []
    if len(value) > SPECIAL_PROJECT_MAX_PROJECTS:
        problems.append("collector frame special_projects.projects too many entries")
    for idx, row in enumerate(value):
        prefix = f"collector frame special_projects.projects[{idx}]"
        if not isinstance(row, dict):
            problems.append(f"{prefix} not dict")
            continue
        for key in row:
            if key not in SPECIAL_PROJECT_ROW_KEYS:
                problems.append(f"{prefix}.{key} unsupported")
        if not _valid_bounded_int(
            row.get("id"),
            minimum=1,
            maximum=SPECIAL_PROJECT_ID_MAX,
        ):
            problems.append(f"{prefix}.id not positive int in range")
        if not _valid_bounded_int(
            row.get("static_def_handle"),
            minimum=0,
            maximum=SPECIAL_PROJECT_HANDLE_MAX,
        ):
            problems.append(f"{prefix}.static_def_handle not int in range")
        if not _valid_bounded_int(
            row.get("iterations"),
            minimum=0,
            maximum=SPECIAL_PROJECT_ITERATIONS_MAX,
        ):
            problems.append(f"{prefix}.iterations not int in range")
        worked_at_facility = row.get("worked_at_facility")
        if worked_at_facility is not None and not _valid_bounded_int(
            worked_at_facility,
            minimum=1,
            maximum=SPECIAL_PROJECT_ID_MAX,
        ):
            problems.append(f"{prefix}.worked_at_facility not positive int|null")
    return problems


def _validate_special_project_scientists(value: Any) -> List[str]:
    if value is None:
        return []
    if not isinstance(value, list):
        return ["collector frame special_projects.scientists not list|null"]
    problems: List[str] = []
    if len(value) > SPECIAL_PROJECT_MAX_SCIENTISTS:
        problems.append("collector frame special_projects.scientists too many entries")
    for idx, row in enumerate(value):
        prefix = f"collector frame special_projects.scientists[{idx}]"
        if not isinstance(row, dict):
            problems.append(f"{prefix} not dict")
            continue
        for key in row:
            if key not in SPECIAL_PROJECT_SCIENTIST_KEYS:
                problems.append(f"{prefix}.{key} unsupported")
        if not _valid_bounded_int(
            row.get("char_id"),
            minimum=1,
            maximum=SPECIAL_PROJECT_ID_MAX,
        ):
            problems.append(f"{prefix}.char_id not positive int in range")
        name_token = row.get("name_token")
        if name_token is not None and not _valid_special_project_token(name_token):
            problems.append(f"{prefix}.name_token not token|null")
        display_name = row.get("display_name")
        if display_name is not None and not _valid_special_project_display(display_name):
            problems.append(f"{prefix}.display_name not printable str|null")
        trait_count = row.get("trait_count")
        if trait_count is not None and not _valid_bounded_int(
            trait_count,
            minimum=0,
            maximum=SPECIAL_PROJECT_TRAIT_COUNT_MAX,
        ):
            problems.append(f"{prefix}.trait_count not int in range|null")
        skills = row.get("skills")
        if skills is None:
            continue
        if not isinstance(skills, dict):
            problems.append(f"{prefix}.skills not dict|null")
            continue
        if len(skills) > len(SPECIAL_PROJECT_SKILL_KEYS):
            problems.append(f"{prefix}.skills too many entries")
        for skill, level in skills.items():
            if skill not in SPECIAL_PROJECT_SKILL_KEYS:
                problems.append(f"{prefix}.skills.{skill} unsupported")
                continue
            if not _valid_bounded_int(
                level,
                minimum=0,
                maximum=SPECIAL_PROJECT_SKILL_LEVEL_MAX,
            ):
                problems.append(f"{prefix}.skills.{skill} not int in range")
    return problems


def _valid_positive_int_key(value: Any, *, max_value: int) -> bool:
    if isinstance(value, bool):
        return False
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        return False
    return str(parsed) == str(value) and 1 <= parsed <= max_value


def _validate_root_probe(root_probe: Dict[str, Any]) -> List[str]:
    problems: List[str] = []
    status = root_probe.get("status")
    if status not in {"profile_mismatch", "verified"}:
        problems.append(f"collector frame root_probe.status {status!r} invalid")
    for key in ("profile", "image_base", "daily_update", "get_long_string",
                "date_root_ptr", "country_database_root_ptr", "reason",
                "current_game_date", "start_date"):
        value = root_probe.get(key)
        if value is not None and not isinstance(value, str):
            problems.append(f"collector frame root_probe.{key} {value!r} not str|null")
    for key in ("daily_update_sentinel_match", "long_string_sentinel_match"):
        value = root_probe.get(key)
        if value is not None and not isinstance(value, bool):
            problems.append(f"collector frame root_probe.{key} {value!r} not bool|null")
    if status == "verified":
        for key in ("daily_update_sentinel_match", "long_string_sentinel_match"):
            if root_probe.get(key) is not True:
                problems.append(
                    f"collector frame verified root_probe.{key} not true")
    for key in ("country_tag_count", "country_active_count",
                "current_game_date_raw", "start_date_raw"):
        value = root_probe.get(key)
        if value is not None and not isinstance(value, int):
            problems.append(f"collector frame root_probe.{key} {value!r} not int|null")
    tag_sample = root_probe.get("tag_sample")
    if tag_sample is not None:
        if not isinstance(tag_sample, list):
            problems.append(f"collector frame root_probe.tag_sample {tag_sample!r} not list|null")
        else:
            for i, tag in enumerate(tag_sample):
                if not isinstance(tag, str):
                    problems.append(
                        f"collector frame root_probe.tag_sample[{i}] {tag!r} not str")
    domains = root_probe.get("domains")
    if domains is not None:
        if not isinstance(domains, list):
            problems.append(f"collector frame root_probe.domains {domains!r} not list|null")
        else:
            for i, domain in enumerate(domains):
                if not isinstance(domain, str):
                    problems.append(
                        f"collector frame root_probe.domains[{i}] {domain!r} not str")
                elif domain not in schema.DOMAINS:
                    problems.append(
                        f"collector frame root_probe.domains[{i}] {domain!r} not canonical")
    return problems


def _fold_country_identity(snap: Dict[str, Any], countries: Dict[str, Any]) -> None:
    for tag, row in countries.items():
        if not schema.valid_tag(tag) or not isinstance(row, dict):
            continue
        country: Optional[Dict[str, Any]] = None
        if row.get("present") is True:
            country = snap["countries"].setdefault(tag, schema.empty_country())
        identity = row.get("identity")
        if not isinstance(identity, dict):
            continue

        clean: Dict[str, Any] = {}
        for key in ("tag_db_id", "original_capital", "original_tag",
                    "reserved_dynamic_country", "templates_locked"):
            if key in identity:
                clean[key] = identity[key]
        if not clean:
            continue
        if country is None:
            country = snap["countries"].setdefault(tag, schema.empty_country())
        country["identity"] = clean


def _fold_country_scalars(snap: Dict[str, Any], countries: Dict[str, Any]) -> None:
    for tag, row in countries.items():
        if not schema.valid_tag(tag) or not isinstance(row, dict):
            continue
        if row.get("present") is not True or not _has_valid_scalar_identity(row):
            continue
        scalars = row.get("scalars")
        if not isinstance(scalars, dict):
            continue
        clean = {k: v for k, v in scalars.items() if v is not None}
        if not clean:
            continue
        country = snap["countries"].setdefault(tag, schema.empty_country())
        politics = country["politics"]
        for key in ("stability", "war_support", "political_power", "command_power"):
            value = clean.get(key)
            if _is_finite_number(value):
                politics[key] = float(value)
        military = country["military"]
        num_ships = clean.get("num_ships")
        if isinstance(num_ships, int) and not isinstance(num_ships, bool):
            military["num_ships"] = num_ships
        command_power = clean.get("command_power")
        if _is_finite_number(command_power):
            military["command_power"] = float(command_power)
        manpower_pool = clean.get("manpower_pool")
        if isinstance(manpower_pool, int) and not isinstance(manpower_pool, bool):
            manpower = military.get("manpower")
            if not isinstance(manpower, dict):
                manpower = {}
                military["manpower"] = manpower
            manpower["available"] = manpower_pool
        convoys = clean.get("convoys")
        if _is_finite_number(convoys):
            military["convoys_count"] = int(round(convoys))


def _fold_country_misc(
    snap: Dict[str, Any],
    countries: Dict[str, Any],
    domains: Iterable[str],
) -> None:
    if "country_misc" not in set(domains):
        return
    for tag, row in countries.items():
        if not schema.valid_tag(tag) or not isinstance(row, dict):
            continue
        if row.get("present") is not True or not _has_valid_scalar_identity(row):
            continue
        misc = row.get("misc")
        if not isinstance(misc, dict):
            continue
        clean: Dict[str, Any] = {}
        convoys_destroyed = misc.get("convoys_destroyed")
        if _valid_bounded_int(
            convoys_destroyed,
            minimum=0,
            maximum=COUNTRY_MISC_CONVOYS_DESTROYED_MAX,
        ):
            clean["convoys_destroyed"] = convoys_destroyed
        accidents_score = misc.get("accidents_score")
        if (
            _is_finite_number(accidents_score)
            and 0.0 <= float(accidents_score) <= COUNTRY_MISC_ACCIDENTS_SCORE_MAX
        ):
            clean["accidents_score"] = float(accidents_score)
        ace_count = misc.get("ace_count")
        if _valid_bounded_int(
            ace_count,
            minimum=0,
            maximum=COUNTRY_MISC_ACE_COUNT_MAX,
        ):
            clean["ace_count"] = ace_count
        if "cosmetic_tag" in misc:
            cosmetic_tag = misc.get("cosmetic_tag")
            if cosmetic_tag is None or _valid_country_misc_token(cosmetic_tag):
                clean["cosmetic_tag"] = cosmetic_tag
        if clean:
            country = snap["countries"].setdefault(tag, schema.empty_country())
            country_misc = country.get("misc")
            if not isinstance(country_misc, dict):
                country_misc = {}
                country["misc"] = country_misc
            country_misc.update(clean)


def _fold_country_focus(
    snap: Dict[str, Any],
    countries: Dict[str, Any],
    domains: Iterable[str],
) -> None:
    if "focus_live" not in set(domains):
        return
    measured_completed_tags = set()
    for tag, row in countries.items():
        if not schema.valid_tag(tag) or not isinstance(row, dict):
            continue
        if row.get("present") is not True or not _has_valid_scalar_identity(row):
            continue
        focus_payload = row.get("focus")
        if not isinstance(focus_payload, dict):
            continue
        clean: Dict[str, Any] = {}
        for key in ("focus_tree", "current", "current_continuous"):
            value = focus_payload.get(key)
            if _valid_focus_token(value):
                clean[key] = value
        current = clean.get("current")
        current_continuous = clean.get("current_continuous")
        if current is not None and current_continuous is not None:
            continue
        completed = focus_payload.get("completed")
        if (
            isinstance(completed, list)
            and len(completed) <= COUNTRY_FOCUS_MAX_COMPLETED
            and all(_valid_focus_token(token) for token in completed)
        ):
            clean["completed"] = list(completed)
            measured_completed_tags.add(tag)
        completed_count = focus_payload.get("completed_count")
        if _valid_bounded_int(
            completed_count,
            minimum=0,
            maximum=COUNTRY_FOCUS_MAX_COMPLETED,
        ):
            if "completed" not in clean or completed_count == len(clean["completed"]):
                clean["completed_count"] = completed_count
        progress = focus_payload.get("progress")
        if (
            _is_finite_number(progress)
            and 0.0 <= float(progress) <= COUNTRY_FOCUS_PROGRESS_MAX
        ):
            clean["progress"] = float(progress)
        if clean:
            country = snap["countries"].setdefault(tag, schema.empty_country())
            focus = country.get("focus")
            if not isinstance(focus, dict):
                focus = {}
                country["focus"] = focus
            focus.update(clean)
    if measured_completed_tags:
        measured = snap.setdefault("_live_measured_leaf_tags", {})
        existing = set(measured.get("focus.completed") or [])
        measured["focus.completed"] = sorted(existing | measured_completed_tags)


def _fold_country_research(
    snap: Dict[str, Any],
    countries: Dict[str, Any],
    domains: Iterable[str],
) -> None:
    if "tech_live" not in set(domains):
        return
    measured_active_tags = set()
    for tag, row in countries.items():
        if not schema.valid_tag(tag) or not isinstance(row, dict):
            continue
        if row.get("present") is not True or not _has_valid_scalar_identity(row):
            continue
        research_payload = row.get("research")
        if not isinstance(research_payload, dict):
            continue
        clean: Dict[str, Any] = {}
        completed_tech_count = research_payload.get("completed_tech_count")
        if _valid_bounded_int(
            completed_tech_count,
            minimum=0,
            maximum=COUNTRY_RESEARCH_MAX_COMPLETED_TECHS,
        ):
            clean["tech_count"] = completed_tech_count
        research_slot_count = research_payload.get("research_slot_count")
        if _valid_bounded_int(
            research_slot_count,
            minimum=0,
            maximum=COUNTRY_RESEARCH_MAX_SLOTS,
        ):
            clean["research_slots"] = research_slot_count
        active_research = research_payload.get("active_research")
        clean_active = _clean_country_research_active_rows(active_research)
        if clean_active is not None:
            clean["in_progress_live"] = clean_active
            measured_active_tags.add(tag)
        if clean:
            country = snap["countries"].setdefault(tag, schema.empty_country())
            research = country.get("research")
            if not isinstance(research, dict):
                research = {}
                country["research"] = research
            research.update(clean)
    if measured_active_tags:
        measured = snap.setdefault("_live_measured_leaf_tags", {})
        existing = set(measured.get("research.in_progress_live") or [])
        measured["research.in_progress_live"] = sorted(existing | measured_active_tags)


def _clean_country_research_active_rows(value: Any) -> Optional[List[Dict[str, Any]]]:
    if value is None:
        return None
    if not isinstance(value, list) or len(value) > COUNTRY_RESEARCH_MAX_ACTIVE:
        return None
    out: List[Dict[str, Any]] = []
    for item in value:
        if not isinstance(item, dict):
            return None
        clean: Dict[str, Any] = {}
        slot_index = item.get("slot_index")
        if slot_index is not None:
            if not _valid_bounded_int(
                slot_index,
                minimum=0,
                maximum=COUNTRY_RESEARCH_MAX_SLOTS - 1,
            ):
                return None
            clean["slot_index"] = slot_index
        for key in (
            "tech_handle_raw",
            "tech_def_index_raw",
            "progress_raw",
            "live_progress_raw",
            "base_required_points_raw",
        ):
            raw_value = item.get(key)
            if raw_value is None:
                continue
            if not _valid_bounded_int(
                raw_value,
                minimum=0,
                maximum=COUNTRY_RESEARCH_RAW_VALUE_MAX,
            ):
                return None
            clean[key] = raw_value
        tech_name = item.get("tech_name")
        if tech_name is not None:
            if not _valid_research_token(tech_name):
                return None
            clean["tech_name"] = tech_name
        progress_units = item.get("progress_units")
        if progress_units is not None:
            if not (
                _is_finite_number(progress_units)
                and 0.0 <= float(progress_units) <= COUNTRY_RESEARCH_PROGRESS_UNITS_MAX
            ):
                return None
            clean["progress_units"] = float(progress_units)
        live_progress_units = item.get("live_progress_units")
        if live_progress_units is not None:
            if not (
                _is_finite_number(live_progress_units)
                and 0.0 <= float(live_progress_units)
                <= COUNTRY_RESEARCH_PROGRESS_UNITS_MAX
            ):
                return None
            clean["live_progress_units"] = float(live_progress_units)
        base_required_points_units = item.get("base_required_points_units")
        if base_required_points_units is not None:
            if not (
                _is_finite_number(base_required_points_units)
                and 0.0 < float(base_required_points_units)
                <= COUNTRY_RESEARCH_PROGRESS_UNITS_MAX
            ):
                return None
            clean["base_required_points_units"] = float(base_required_points_units)
        out.append(clean)
    return out


def _fold_country_politics(
    snap: Dict[str, Any],
    countries: Dict[str, Any],
    domains: Iterable[str],
) -> None:
    if "politics_live" not in set(domains):
        return
    measured_ideas_other_tags = set()
    measured_national_spirit_tags = set()
    measured_leader_tags = set()
    for tag, row in countries.items():
        if not schema.valid_tag(tag) or not isinstance(row, dict):
            continue
        if row.get("present") is not True or not _has_valid_scalar_identity(row):
            continue
        politics_payload = row.get("politics")
        if not isinstance(politics_payload, dict):
            continue
        clean: Dict[str, Any] = {}
        popularities = politics_payload.get("popularities")
        if isinstance(popularities, dict):
            clean_popularities: Dict[str, float] = {}
            for ideology, value in popularities.items():
                if ideology in COUNTRY_POLITICS_IDEOLOGIES and (
                    _is_finite_number(value)
                    and 0.0 <= float(value) <= 100.0001
                ):
                    clean_popularities[ideology] = float(value)
            if clean_popularities:
                clean["party_popularities"] = clean_popularities
        ruling_party = politics_payload.get("ruling_party")
        if ruling_party in COUNTRY_POLITICS_IDEOLOGIES:
            clean["ruling_party"] = ruling_party
        leader = _clean_country_leader_payload(politics_payload.get("leader"))
        if leader is not None:
            country = snap["countries"].setdefault(tag, schema.empty_country())
            commanders = country.get("commanders")
            if not isinstance(commanders, dict):
                commanders = {}
                country["commanders"] = commanders
            commanders["leader"] = leader
            measured_leader_tags.add(tag)
        ideas = politics_payload.get("ideas")
        if (
            isinstance(ideas, list)
            and len(ideas) <= COUNTRY_POLITICS_MAX_IDEAS
            and all(_valid_politics_token(idea) for idea in ideas)
        ):
            clean["ideas"] = list(ideas)
        ideas_other = politics_payload.get("ideas_other")
        if (
            isinstance(ideas_other, list)
            and len(ideas_other) <= COUNTRY_POLITICS_MAX_IDEAS
            and all(_valid_politics_token(token) for token in ideas_other)
        ):
            clean["ideas_other"] = list(ideas_other)
            measured_ideas_other_tags.add(tag)
        national_spirits = politics_payload.get("national_spirits")
        if (
            isinstance(national_spirits, list)
            and len(national_spirits) <= COUNTRY_POLITICS_MAX_IDEAS
            and all(_valid_politics_token(token) for token in national_spirits)
        ):
            clean["national_spirits"] = list(national_spirits)
            measured_national_spirit_tags.add(tag)
        laws = politics_payload.get("laws")
        if (
            isinstance(laws, dict)
            and len(laws) <= COUNTRY_POLITICS_MAX_LAWS
            and all(
                _valid_politics_token(group) and _valid_politics_token(token)
                for group, token in laws.items()
            )
        ):
            clean["laws"] = dict(laws)
        idea_groups = _clean_politics_token_map(politics_payload.get("idea_groups"))
        if idea_groups is not None:
            clean["idea_groups"] = idea_groups
        advisor_ideas = _clean_politics_token_map(
            politics_payload.get("advisor_ideas"))
        if advisor_ideas is not None:
            clean["advisor_ideas"] = advisor_ideas
        for key, minimum, maximum in (
            ("idea_count", 0, COUNTRY_POLITICS_MAX_IDEAS),
            ("election_frequency", 1, 600),
            ("party_count", 1, 16),
        ):
            value = politics_payload.get(key)
            if _valid_bounded_int(value, minimum=minimum, maximum=maximum):
                clean[key] = value
        low_confidence = politics_payload.get("low_confidence")
        if isinstance(low_confidence, bool):
            clean["low_confidence"] = low_confidence
        if clean:
            country = snap["countries"].setdefault(tag, schema.empty_country())
            politics = country.get("politics")
            if not isinstance(politics, dict):
                politics = {}
                country["politics"] = politics
            politics.update(clean)
    if measured_ideas_other_tags or measured_national_spirit_tags or measured_leader_tags:
        measured = snap.setdefault("_live_measured_leaf_tags", {})
    if measured_ideas_other_tags:
        measured["politics.ideas_other"] = sorted(measured_ideas_other_tags)
    if measured_national_spirit_tags:
        measured["politics.national_spirits"] = sorted(measured_national_spirit_tags)
    if measured_leader_tags:
        existing = set(measured.get("commanders.leader") or [])
        measured["commanders.leader"] = sorted(existing | measured_leader_tags)


def _clean_politics_token_map(value: Any) -> Optional[Dict[str, List[str]]]:
    if not isinstance(value, dict) or len(value) > COUNTRY_POLITICS_MAX_IDEAS:
        return None
    out: Dict[str, List[str]] = {}
    for group, tokens in value.items():
        if not _valid_politics_token(group):
            return None
        if (
            not isinstance(tokens, list)
            or len(tokens) > COUNTRY_POLITICS_MAX_IDEAS
            or not all(_valid_politics_token(token) for token in tokens)
        ):
            return None
        out[group] = list(tokens)
    return out


def _clean_country_leader_payload(value: Any) -> Optional[Dict[str, Any]]:
    if not isinstance(value, dict):
        return None
    char_id = value.get("char_id")
    if not _valid_bounded_int(
        char_id,
        minimum=1,
        maximum=COUNTRY_LEADER_CHAR_ID_MAX,
    ):
        return None
    clean: Dict[str, Any] = {"char_id": char_id, "source": RAW_SOURCE}
    name = value.get("name")
    if isinstance(name, str) and 0 < len(name) <= COUNTRY_LEADER_NAME_MAX_LENGTH:
        clean["name"] = name
    token = value.get("token")
    if token is not None:
        if not _valid_politics_token(token):
            return None
        clean["token"] = token
    ideology_group = value.get("ideology_group")
    if ideology_group is not None:
        if ideology_group not in COUNTRY_POLITICS_IDEOLOGIES:
            return None
        clean["ideology_group"] = ideology_group
    ambiguous = value.get("ambiguous")
    if isinstance(ambiguous, bool):
        clean["ambiguous"] = ambiguous
    source = value.get("source")
    if source is not None and source != RAW_SOURCE:
        return None
    return clean


def _fold_country_politics_residuals(
    snap: Dict[str, Any],
    countries: Dict[str, Any],
    domains: Iterable[str],
) -> None:
    if "politics_residuals" not in set(domains):
        return
    for tag, row in countries.items():
        if not schema.valid_tag(tag) or not isinstance(row, dict):
            continue
        if row.get("present") is not True or not _has_valid_scalar_identity(row):
            continue
        politics_payload = row.get("politics")
        if not isinstance(politics_payload, dict):
            continue
        clean: Dict[str, Any] = {}
        last_election = politics_payload.get("last_election")
        if last_election is not None and _valid_datehour_string(last_election):
            clean["last_election"] = last_election
        elections_allowed = politics_payload.get("elections_allowed")
        if isinstance(elections_allowed, bool):
            clean["elections_allowed"] = elections_allowed
        appointed_advisors = politics_payload.get("appointed_advisors")
        if (
            isinstance(appointed_advisors, list)
            and len(appointed_advisors) <= COUNTRY_POLITICS_MAX_IDEAS
            and all(_valid_politics_token(token) for token in appointed_advisors)
        ):
            clean["appointed_advisors"] = list(appointed_advisors)
        appointed_advisor_details = politics_payload.get("appointed_advisor_details")
        clean_details = _clean_appointed_advisor_details(appointed_advisor_details)
        if clean_details is not None:
            clean["appointed_advisor_details"] = clean_details
        appointed_advisor_slots = politics_payload.get("appointed_advisor_slots")
        clean_slots = _clean_appointed_advisor_token_map(appointed_advisor_slots)
        if clean_slots is not None:
            clean["appointed_advisor_slots"] = clean_slots
        appointed_advisor_ledgers = politics_payload.get("appointed_advisor_ledgers")
        clean_ledgers = _clean_appointed_advisor_token_map(appointed_advisor_ledgers)
        if clean_ledgers is not None:
            clean["appointed_advisor_ledgers"] = clean_ledgers
        if clean:
            country = snap["countries"].setdefault(tag, schema.empty_country())
            politics = country.get("politics")
            if not isinstance(politics, dict):
                politics = {}
                country["politics"] = politics
            politics.update(clean)


def _clean_appointed_advisor_details(value: Any) -> Optional[List[Dict[str, Any]]]:
    if not isinstance(value, list) or len(value) > COUNTRY_POLITICS_MAX_IDEAS:
        return None
    out: List[Dict[str, Any]] = []
    for item in value:
        if not isinstance(item, dict):
            return None
        token = item.get("idea_token")
        if not _valid_politics_token(token):
            return None
        clean: Dict[str, Any] = {"idea_token": token}
        for key in ("slot", "ledger", "character"):
            meta = item.get(key)
            if meta is not None:
                if not _valid_politics_token(meta):
                    return None
                clean[key] = meta
            else:
                clean[key] = None
        out.append(clean)
    return out


def _clean_appointed_advisor_token_map(value: Any) -> Optional[Dict[str, List[str]]]:
    if not isinstance(value, dict) or len(value) > COUNTRY_POLITICS_MAX_IDEAS:
        return None
    out: Dict[str, List[str]] = {}
    for bucket, tokens in value.items():
        if not _valid_politics_token(bucket):
            return None
        if not isinstance(tokens, list) or len(tokens) > COUNTRY_POLITICS_MAX_IDEAS:
            return None
        clean_tokens = []
        for token in tokens:
            if not _valid_politics_token(token):
                return None
            clean_tokens.append(token)
        out[bucket] = clean_tokens
    return out


def _fold_country_decisions_live(
    snap: Dict[str, Any],
    countries: Dict[str, Any],
    domains: Iterable[str],
) -> None:
    if "decisions_live" not in set(domains):
        return
    measured_timed_tags = set()
    for tag, row in countries.items():
        if not schema.valid_tag(tag) or not isinstance(row, dict):
            continue
        if row.get("present") is not True or not _has_valid_scalar_identity(row):
            continue
        decisions_payload = row.get("decisions")
        if not isinstance(decisions_payload, dict):
            continue
        clean: Dict[str, Any] = {}
        status_present = decisions_payload.get("status_present")
        if (
            isinstance(status_present, dict)
            and len(status_present) <= COUNTRY_DECISIONS_MAX_STATUS_KEYS
            and all(
                _valid_decision_token(key)
                and _valid_bounded_int(
                    value,
                    minimum=0,
                    maximum=COUNTRY_DECISIONS_STATUS_COUNT_MAX,
                )
                for key, value in status_present.items()
            )
        ):
            clean["status_present"] = dict(status_present)
        timed_live = decisions_payload.get("timed_live")
        if isinstance(timed_live, list) and len(timed_live) <= COUNTRY_DECISIONS_MAX_TIMED:
            clean_timed = []
            for item in timed_live:
                if not isinstance(item, dict):
                    clean_timed = None
                    break
                decision = item.get("decision")
                days = item.get("days")
                state = item.get("state")
                state_name = item.get("state_name")
                if (
                    not _valid_decision_token(decision)
                    or not _valid_bounded_int(
                        days,
                        minimum=0,
                        maximum=COUNTRY_DECISIONS_DAYS_MAX,
                    )
                    or not _valid_bounded_int(
                        state,
                        minimum=0,
                        maximum=COUNTRY_DECISIONS_STATE_MAX,
                    )
                    or (
                        state_name is not None
                        and not _valid_decision_token(state_name)
                    )
                ):
                    clean_timed = None
                    break
                clean_timed.append({
                    "decision": decision,
                    "days": days,
                    "state": state,
                    "state_name": state_name,
                })
            if clean_timed is not None:
                clean["timed_live"] = clean_timed
                measured_timed_tags.add(tag)
        if clean:
            country = snap["countries"].setdefault(tag, schema.empty_country())
            decisions = country.get("decisions")
            if not isinstance(decisions, dict):
                decisions = {}
                country["decisions"] = decisions
            decisions.update(clean)
    if measured_timed_tags:
        measured = snap.setdefault("_live_measured_leaf_tags", {})
        existing = set(measured.get("decisions.timed_live") or [])
        measured["decisions.timed_live"] = sorted(existing | measured_timed_tags)


def _fold_country_diplomacy_live(
    snap: Dict[str, Any],
    countries: Dict[str, Any],
    domains: Iterable[str],
    diplomacy_live_payload: Any = None,
) -> None:
    if "diplomacy_live" not in set(domains):
        return
    measured_faction_tags = set()
    measured_at_war_tags = set()
    measured_casualty_tags = set()
    members_by_group: Dict[str, List[str]] = {}
    for tag, row in countries.items():
        if not schema.valid_tag(tag) or not isinstance(row, dict):
            continue
        if row.get("present") is not True or not _has_valid_scalar_identity(row):
            continue
        diplomacy_payload = row.get("diplomacy")
        if not isinstance(diplomacy_payload, dict):
            continue
        clean: Dict[str, Any] = {}
        faction_group = diplomacy_payload.get("faction_group")
        if _valid_bounded_int(
            faction_group,
            minimum=-COUNTRY_DIPLOMACY_FACTION_GROUP_ABS_MAX,
            maximum=COUNTRY_DIPLOMACY_FACTION_GROUP_ABS_MAX,
        ):
            clean["faction_group"] = faction_group
            measured_faction_tags.add(tag)
            members_by_group.setdefault(str(faction_group), []).append(tag)
        at_war = diplomacy_payload.get("at_war")
        if isinstance(at_war, list) and all(schema.valid_tag(enemy) for enemy in at_war):
            clean["at_war"] = sorted(at_war)
            measured_at_war_tags.add(tag)
        casualties = diplomacy_payload.get("casualties")
        if _valid_bounded_int(
            casualties,
            minimum=0,
            maximum=COUNTRY_DIPLOMACY_CASUALTIES_MAX,
        ):
            clean["casualties"] = casualties
            measured_casualty_tags.add(tag)
        if not clean:
            continue
        country = snap["countries"].setdefault(tag, schema.empty_country())
        diplomacy = country.get("diplomacy")
        if not isinstance(diplomacy, dict):
            diplomacy = {}
            country["diplomacy"] = diplomacy
        diplomacy.update(clean)

    clean_sidecar = _clean_diplomacy_live_sidecar(diplomacy_live_payload)
    if measured_faction_tags or measured_at_war_tags or measured_casualty_tags or clean_sidecar:
        measured = snap.setdefault("_live_measured_leaf_tags", {})
        if measured_faction_tags:
            existing = set(measured.get("diplomacy.faction_group") or [])
            measured["diplomacy.faction_group"] = sorted(
                existing | measured_faction_tags)
        if measured_at_war_tags:
            existing = set(measured.get("diplomacy.at_war") or [])
            measured["diplomacy.at_war"] = sorted(existing | measured_at_war_tags)
        if measured_casualty_tags:
            existing = set(measured.get("diplomacy.casualties") or [])
            measured["diplomacy.casualties"] = sorted(
                existing | measured_casualty_tags)
        dl_side = snap.get("diplomacy_live")
        if not isinstance(dl_side, dict):
            dl_side = schema.empty_diplomacy_live()
            snap["diplomacy_live"] = dl_side
        if clean_sidecar:
            if clean_sidecar.get("war_relations") is not None:
                dl_side["war_relations"] = clean_sidecar["war_relations"]
            if clean_sidecar.get("faction_members") is not None:
                dl_side["faction_members"] = clean_sidecar["faction_members"]
            if clean_sidecar.get("subtype_warning") is not None:
                dl_side["subtype_warning"] = clean_sidecar["subtype_warning"]
        elif members_by_group:
            dl_side["faction_members"] = {
                group: sorted(tags) for group, tags in sorted(members_by_group.items())
            }
        if dl_side.get("faction_members") is None and members_by_group:
            dl_side["faction_members"] = {
                group: sorted(tags) for group, tags in sorted(members_by_group.items())
            }
        dl_side["source"] = "C"


def _clean_diplomacy_live_sidecar(value: Any) -> Optional[Dict[str, Any]]:
    if not isinstance(value, dict):
        return None
    clean: Dict[str, Any] = {}
    war_relations = value.get("war_relations")
    if isinstance(war_relations, list) and len(war_relations) <= DIPLOMACY_LIVE_MAX_WAR_RELATIONS:
        clean_rows = []
        for row in war_relations:
            if not isinstance(row, dict):
                continue
            first = row.get("first")
            second = row.get("second")
            first_casualties = row.get("first_casualties")
            second_casualties = row.get("second_casualties")
            if not (schema.valid_tag(first) and schema.valid_tag(second)):
                continue
            if not (
                _valid_bounded_int(
                    first_casualties,
                    minimum=0,
                    maximum=COUNTRY_DIPLOMACY_CASUALTIES_MAX,
                )
                and _valid_bounded_int(
                    second_casualties,
                    minimum=0,
                    maximum=COUNTRY_DIPLOMACY_CASUALTIES_MAX,
                )
            ):
                continue
            clean_rows.append({
                "first": first,
                "second": second,
                "first_casualties": first_casualties,
                "second_casualties": second_casualties,
            })
        clean["war_relations"] = clean_rows
    faction_members = value.get("faction_members")
    if isinstance(faction_members, dict):
        clean_members: Dict[str, List[str]] = {}
        for group, members in faction_members.items():
            try:
                group_key = str(int(str(group)))
            except (TypeError, ValueError):
                continue
            if not isinstance(members, list):
                continue
            tags = sorted(tag for tag in members if schema.valid_tag(tag))
            clean_members[group_key] = tags
        clean["faction_members"] = clean_members
    subtype_warning = value.get("subtype_warning")
    if isinstance(subtype_warning, dict):
        clean["subtype_warning"] = dict(subtype_warning)
    return clean or None


def _fold_country_fuel(
    snap: Dict[str, Any],
    countries: Dict[str, Any],
    domains: Iterable[str],
) -> None:
    if "fuel_live" not in set(domains):
        return
    for tag, row in countries.items():
        if not schema.valid_tag(tag) or not isinstance(row, dict):
            continue
        if row.get("present") is not True or not _has_valid_scalar_identity(row):
            continue
        fuel = row.get("fuel")
        if not isinstance(fuel, dict):
            continue
        clean: Dict[str, float] = {}
        current = fuel.get("current")
        if (
            _is_finite_number(current)
            and 0.0 <= float(current) <= FUEL_MEASURE_ABS_MAX
        ):
            clean["current"] = float(current)
        max_fuel = fuel.get("max")
        if (
            _is_finite_number(max_fuel)
            and 0.0 <= float(max_fuel) <= FUEL_MEASURE_ABS_MAX
        ):
            clean["max"] = float(max_fuel)
        gain = fuel.get("gain")
        if _is_finite_number(gain) and abs(float(gain)) <= FUEL_MEASURE_ABS_MAX:
            clean["gain"] = float(gain)
        cost = fuel.get("cost")
        if _is_finite_number(cost) and abs(float(cost)) <= FUEL_MEASURE_ABS_MAX:
            clean["cost"] = float(cost)
        if clean:
            country = snap["countries"].setdefault(tag, schema.empty_country())
            economy = country["economy"]
            existing = economy.get("fuel")
            if not isinstance(existing, dict):
                existing = {}
                economy["fuel"] = existing
            existing.update(clean)


def _fold_country_production(
    snap: Dict[str, Any],
    countries: Dict[str, Any],
    domains: Iterable[str],
) -> None:
    if "production_live" not in set(domains):
        return
    for tag, row in countries.items():
        if not schema.valid_tag(tag) or not isinstance(row, dict):
            continue
        if row.get("present") is not True or not _has_valid_scalar_identity(row):
            continue
        production = row.get("production")
        if not isinstance(production, dict):
            continue
        clean: Dict[str, Any] = {}
        stockpile = production.get("stockpile")
        if isinstance(stockpile, dict):
            clean_stockpile: Dict[str, float] = {}
            for variant_id, amount in stockpile.items():
                if (
                    _valid_positive_int_key(
                        variant_id, max_value=PRODUCTION_VARIANT_ID_MAX
                    )
                    and _valid_signed_measure(amount)
                ):
                    clean_stockpile[str(int(variant_id))] = float(amount)
            clean["stockpile"] = clean_stockpile
        stockpile_total = production.get("stockpile_total")
        if _valid_signed_measure(stockpile_total):
            clean["stockpile_total"] = float(stockpile_total)
        for key in ("military_lines", "naval_lines"):
            lines = production.get(key)
            if isinstance(lines, list):
                clean_lines: List[Dict[str, Any]] = []
                for line in lines:
                    if not isinstance(line, dict):
                        continue
                    line_id = line.get("id")
                    active = line.get("active_factories")
                    cost = line.get("cost")
                    speed = line.get("speed")
                    produced = line.get("produced")
                    amount = line.get("amount")
                    if not (
                        _valid_bounded_int(
                            line_id, minimum=0, maximum=PRODUCTION_LINE_ID_MAX
                        )
                        and _valid_bounded_int(
                            active, minimum=0,
                            maximum=PRODUCTION_FACTORY_COUNT_MAX
                        )
                        and _valid_nonnegative_measure(cost)
                        and _valid_nonnegative_measure(speed)
                        and _valid_nonnegative_measure(produced)
                        and (
                            amount == -1
                            or _valid_bounded_int(
                                amount, minimum=0,
                                maximum=PRODUCTION_LINE_AMOUNT_MAX
                            )
                        )
                    ):
                        continue
                    clean_lines.append({
                        "id": line_id,
                        "active_factories": active,
                        "cost": float(cost),
                        "speed": float(speed),
                        "produced": float(produced),
                        "amount": amount,
                    })
                clean[key] = clean_lines
        for key in ("military_factories", "naval_factories"):
            factories = production.get(key)
            if _valid_bounded_int(
                factories, minimum=0, maximum=PRODUCTION_FACTORY_COUNT_MAX
            ):
                clean[key] = factories
        construction_queue = production.get("construction_queue")
        if isinstance(construction_queue, list):
            clean_queue: List[Dict[str, Any]] = []
            for line in construction_queue:
                if not isinstance(line, dict):
                    continue
                line_id = line.get("id")
                template_id = line.get("template_id")
                active = line.get("active_factories")
                cost = line.get("cost")
                produced = line.get("produced")
                priority = line.get("priority")
                if not (
                    _valid_bounded_int(
                        line_id, minimum=0, maximum=PRODUCTION_LINE_ID_MAX
                    )
                    and (
                        template_id is None
                        or _valid_bounded_int(
                            template_id,
                            minimum=0,
                            maximum=PRODUCTION_CONSTRUCTION_TEMPLATE_ID_MAX,
                        )
                    )
                    and _valid_bounded_int(
                        active, minimum=0, maximum=PRODUCTION_FACTORY_COUNT_MAX
                    )
                    and _valid_nonnegative_measure(cost)
                    and _valid_nonnegative_measure(produced)
                    and _valid_bounded_int(
                        priority, minimum=0, maximum=PRODUCTION_LINE_MAX_ENTRIES
                    )
                ):
                    continue
                clean_line: Dict[str, Any] = {
                    "id": line_id,
                    "active_factories": active,
                    "cost": float(cost),
                    "produced": float(produced),
                    "priority": priority,
                }
                if template_id is not None:
                    clean_line["template_id"] = template_id
                clean_queue.append(clean_line)
            clean["construction_queue"] = clean_queue
        if clean:
            country = snap["countries"].setdefault(tag, schema.empty_country())
            country["production"].update(clean)


def _fold_special_projects(
    snap: Dict[str, Any],
    special_projects: Dict[str, Any],
    domains: Iterable[str],
) -> None:
    if "projects_live" not in set(domains):
        return
    clean = _clean_special_projects(special_projects)
    if clean is not None:
        snap["special_projects"] = clean


def _clean_special_projects(value: Any) -> Optional[Dict[str, Any]]:
    if not isinstance(value, dict):
        return None
    facilities = _clean_special_project_facilities(value.get("facilities"))
    projects = _clean_special_project_rows(value.get("projects"))
    scientists = _clean_special_project_scientists(value.get("scientists"))
    if facilities is None and projects is None and scientists is None:
        return None
    out: Dict[str, Any] = {
        "facilities": facilities if facilities is not None else [],
        "projects": projects if projects is not None else [],
        "scientists": scientists if scientists is not None else [],
        "source": "C",
    }
    count = value.get("count")
    if isinstance(count, dict):
        clean_count: Dict[str, int] = {}
        for key, rows in (
            ("facilities", out["facilities"]),
            ("projects", out["projects"]),
            ("scientists", out["scientists"]),
        ):
            raw_count = count.get(key)
            clean_count[key] = (
                raw_count
                if _valid_bounded_int(
                    raw_count,
                    minimum=0,
                    maximum=SPECIAL_PROJECT_MAX_PROJECTS,
                )
                and raw_count == len(rows)
                else len(rows)
            )
        out["count"] = clean_count
    else:
        out["count"] = {
            "facilities": len(out["facilities"]),
            "projects": len(out["projects"]),
            "scientists": len(out["scientists"]),
        }
    return out


def _clean_special_project_facilities(value: Any) -> Optional[List[Dict[str, Any]]]:
    if value is None:
        return None
    if not isinstance(value, list) or len(value) > SPECIAL_PROJECT_MAX_FACILITIES:
        return None
    clean_rows: List[Dict[str, Any]] = []
    for row in value:
        if not isinstance(row, dict):
            return None
        facility_id = row.get("id")
        province = row.get("province")
        active_project_handle = row.get("active_project_handle")
        active = row.get("active")
        unread = row.get("unread_prototype_rewards")
        if not (
            _valid_bounded_int(
                facility_id, minimum=1, maximum=SPECIAL_PROJECT_ID_MAX)
            and _valid_bounded_int(
                province, minimum=1, maximum=SPECIAL_PROJECT_PROVINCE_MAX)
            and _valid_bounded_int(
                active_project_handle,
                minimum=0,
                maximum=SPECIAL_PROJECT_HANDLE_MAX,
            )
            and isinstance(active, bool)
            and _valid_bounded_int(
                unread,
                minimum=0,
                maximum=SPECIAL_PROJECT_UNREAD_REWARDS_MAX,
            )
        ):
            return None
        scientist_char_id = row.get("scientist_char_id")
        if scientist_char_id is not None and not _valid_bounded_int(
            scientist_char_id,
            minimum=1,
            maximum=SPECIAL_PROJECT_ID_MAX,
        ):
            return None
        building = row.get("building")
        if building is not None and not _valid_special_project_token(building):
            return None
        clean_rows.append({
            "id": facility_id,
            "province": province,
            "building": building,
            "active_project_handle": active_project_handle,
            "active": active,
            "scientist_char_id": scientist_char_id,
            "unread_prototype_rewards": unread,
        })
    return clean_rows


def _clean_special_project_rows(value: Any) -> Optional[List[Dict[str, Any]]]:
    if value is None:
        return None
    if not isinstance(value, list) or len(value) > SPECIAL_PROJECT_MAX_PROJECTS:
        return None
    clean_rows: List[Dict[str, Any]] = []
    for row in value:
        if not isinstance(row, dict):
            return None
        project_id = row.get("id")
        static_def_handle = row.get("static_def_handle")
        iterations = row.get("iterations")
        if not (
            _valid_bounded_int(project_id, minimum=1, maximum=SPECIAL_PROJECT_ID_MAX)
            and _valid_bounded_int(
                static_def_handle,
                minimum=0,
                maximum=SPECIAL_PROJECT_HANDLE_MAX,
            )
            and _valid_bounded_int(
                iterations,
                minimum=0,
                maximum=SPECIAL_PROJECT_ITERATIONS_MAX,
            )
        ):
            return None
        worked_at_facility = row.get("worked_at_facility")
        if worked_at_facility is not None and not _valid_bounded_int(
            worked_at_facility,
            minimum=1,
            maximum=SPECIAL_PROJECT_ID_MAX,
        ):
            return None
        clean_rows.append({
            "id": project_id,
            "static_def_handle": static_def_handle,
            "iterations": iterations,
            "worked_at_facility": worked_at_facility,
        })
    return clean_rows


def _clean_special_project_scientists(value: Any) -> Optional[List[Dict[str, Any]]]:
    if value is None:
        return None
    if not isinstance(value, list) or len(value) > SPECIAL_PROJECT_MAX_SCIENTISTS:
        return None
    clean_rows: List[Dict[str, Any]] = []
    for row in value:
        if not isinstance(row, dict):
            return None
        char_id = row.get("char_id")
        if not _valid_bounded_int(char_id, minimum=1, maximum=SPECIAL_PROJECT_ID_MAX):
            return None
        name_token = row.get("name_token")
        if name_token is not None and not _valid_special_project_token(name_token):
            return None
        display_name = row.get("display_name")
        if display_name is not None and not _valid_special_project_display(display_name):
            return None
        trait_count = row.get("trait_count")
        if trait_count is not None and not _valid_bounded_int(
            trait_count,
            minimum=0,
            maximum=SPECIAL_PROJECT_TRAIT_COUNT_MAX,
        ):
            return None
        skills = row.get("skills")
        clean_skills: Dict[str, int] = {}
        if isinstance(skills, dict):
            for skill, level in skills.items():
                if skill not in SPECIAL_PROJECT_SKILL_KEYS or not _valid_bounded_int(
                    level,
                    minimum=0,
                    maximum=SPECIAL_PROJECT_SKILL_LEVEL_MAX,
                ):
                    return None
                clean_skills[skill] = level
        elif skills is not None:
            return None
        clean_rows.append({
            "char_id": char_id,
            "name_token": name_token,
            "display_name": display_name,
            "skills": clean_skills,
            "trait_count": trait_count,
        })
    return clean_rows


def _fold_state_rows(
    snap: Dict[str, Any],
    states: Dict[str, Any],
    domains: Iterable[str],
) -> None:
    domain_set = set(domains)
    fold_owner = "state_ownership" in domain_set
    fold_controller = "state_control" in domain_set
    fold_buildings = "buildings_live" in domain_set
    if not (fold_owner or fold_controller or fold_buildings):
        return
    for sid, row in states.items():
        if not _valid_positive_int_key(sid, max_value=100_000) or not isinstance(row, dict):
            continue
        clean: Dict[str, Any] = {}
        if fold_owner and "owner" in row:
            owner = row.get("owner")
            clean["owner"] = owner if schema.valid_tag(owner) else None
        if fold_controller and "controller" in row:
            controller = row.get("controller")
            clean["controller"] = controller if schema.valid_tag(controller) else None
        if fold_buildings and "buildings" in row:
            buildings = _clean_building_map(row.get("buildings"))
            if buildings is not None:
                clean["buildings"] = buildings
        if clean:
            state = snap["states"].setdefault(str(int(sid)), schema.empty_state())
            state.update(clean)


def _fold_province_rows(
    snap: Dict[str, Any],
    provinces: Dict[str, Any],
    domains: Iterable[str],
) -> None:
    domain_set = set(domains)
    fold_control = "province_control" in domain_set
    fold_buildings = "buildings_live" in domain_set
    if not (fold_control or fold_buildings):
        return
    for pid, row in provinces.items():
        if not _valid_positive_int_key(pid, max_value=1_000_000) or not isinstance(row, dict):
            continue
        clean: Dict[str, Any] = {}
        if fold_control and "controller" in row:
            controller = row.get("controller")
            clean["controller"] = controller if schema.valid_tag(controller) else None
        state_id = row.get("state_id")
        if fold_control and isinstance(state_id, int) and not isinstance(state_id, bool):
            clean["state_id"] = state_id
        if fold_buildings and "buildings" in row:
            buildings = _clean_building_map(row.get("buildings"))
            if buildings is not None:
                clean["buildings"] = buildings
        if clean:
            province = snap["provinces"].setdefault(str(int(pid)), schema.empty_province())
            province.update(clean)


def _clean_building_map(buildings: Any) -> Optional[Dict[str, int]]:
    if not isinstance(buildings, dict):
        return None
    clean: Dict[str, int] = {}
    for key, value in buildings.items():
        if not schema.valid_script_token(key):
            return None
        if not _valid_bounded_int(value, minimum=0, maximum=BUILDING_LEVEL_MAX):
            return None
        clean[key] = int(value)
    return clean


def _fill_map_mirrors(snap: Dict[str, Any]) -> None:
    owner_by_state = {
        sid: state.get("owner")
        for sid, state in (snap.get("states") or {}).items()
        if isinstance(state, dict) and state.get("owner") is not None
    }
    control_by_province = {
        pid: province.get("controller")
        for pid, province in (snap.get("provinces") or {}).items()
        if isinstance(province, dict) and province.get("controller") is not None
    }
    snap["map"]["owner_by_state"] = owner_by_state or None
    snap["map"]["control_by_province"] = control_by_province or None


def _has_state_owner_rows(states: Any) -> bool:
    return isinstance(states, dict) and any(
        isinstance(row, dict) and "owner" in row
        for row in states.values()
    )


def _has_state_control_rows(states: Any) -> bool:
    return isinstance(states, dict) and any(
        isinstance(row, dict) and "controller" in row
        for row in states.values()
    )


def _has_province_control_rows(provinces: Any) -> bool:
    return isinstance(provinces, dict) and any(
        isinstance(row, dict) and "controller" in row
        for row in provinces.values()
    )


def _has_building_rows(states: Any, provinces: Any) -> bool:
    return (
        isinstance(states, dict)
        and any(isinstance(row, dict) and isinstance(row.get("buildings"), dict)
                for row in states.values())
    ) or (
        isinstance(provinces, dict)
        and any(isinstance(row, dict) and isinstance(row.get("buildings"), dict)
                for row in provinces.values())
    )


def _has_country_fuel_rows(countries: Any) -> bool:
    return isinstance(countries, dict) and any(
        isinstance(row, dict) and isinstance(row.get("fuel"), dict)
        for row in countries.values()
    )


def _has_country_production_payload(countries: Any) -> bool:
    return isinstance(countries, dict) and any(
        isinstance(row, dict) and row.get("production") is not None
        for row in countries.values()
    )


def _has_country_production_rows(countries: Any) -> bool:
    return isinstance(countries, dict) and any(
        isinstance(row, dict) and isinstance(row.get("production"), dict)
        for row in countries.values()
    )


def _has_country_focus_payload(countries: Any) -> bool:
    return isinstance(countries, dict) and any(
        isinstance(row, dict) and row.get("focus") is not None
        for row in countries.values()
    )


def _has_country_focus_rows(countries: Any) -> bool:
    return isinstance(countries, dict) and any(
        isinstance(row, dict)
        and isinstance(row.get("focus"), dict)
        and any(key in row["focus"] for key in COUNTRY_FOCUS_PAYLOAD_KEYS)
        for row in countries.values()
    )


def _has_country_research_payload(countries: Any) -> bool:
    return isinstance(countries, dict) and any(
        isinstance(row, dict) and row.get("research") is not None
        for row in countries.values()
    )


def _has_country_research_rows(countries: Any) -> bool:
    return isinstance(countries, dict) and any(
        isinstance(row, dict)
        and isinstance(row.get("research"), dict)
        and any(key in row["research"] for key in COUNTRY_RESEARCH_PAYLOAD_KEYS)
        for row in countries.values()
    )


def _has_country_politics_rows(countries: Any) -> bool:
    return isinstance(countries, dict) and any(
        isinstance(row, dict) and isinstance(row.get("politics"), dict)
        for row in countries.values()
    )


def _has_country_politics_live_rows(countries: Any) -> bool:
    return isinstance(countries, dict) and any(
        isinstance(row, dict)
        and isinstance(row.get("politics"), dict)
        and any(key in row["politics"] for key in COUNTRY_POLITICS_LIVE_PAYLOAD_KEYS)
        for row in countries.values()
    )


def _has_country_politics_residual_rows(countries: Any) -> bool:
    return isinstance(countries, dict) and any(
        isinstance(row, dict)
        and isinstance(row.get("politics"), dict)
        and any(key in row["politics"] for key in COUNTRY_POLITICS_RESIDUAL_PAYLOAD_KEYS)
        for row in countries.values()
    )


def _has_country_decisions_live_rows(countries: Any) -> bool:
    return isinstance(countries, dict) and any(
        isinstance(row, dict)
        and isinstance(row.get("decisions"), dict)
        and any(key in row["decisions"] for key in COUNTRY_DECISIONS_PAYLOAD_KEYS)
        for row in countries.values()
    )


def _has_country_diplomacy_rows(countries: Any) -> bool:
    return isinstance(countries, dict) and any(
        isinstance(row, dict)
        and isinstance(row.get("diplomacy"), dict)
        and any(key in row["diplomacy"] for key in COUNTRY_DIPLOMACY_PAYLOAD_KEYS)
        for row in countries.values()
    )


def _has_diplomacy_live_sidecar(diplomacy_live: Any) -> bool:
    return (
        isinstance(diplomacy_live, dict)
        and any(key in diplomacy_live for key in DIPLOMACY_LIVE_PAYLOAD_KEYS)
    )


def _has_special_projects_payload(special_projects: Any) -> bool:
    if not isinstance(special_projects, dict):
        return False
    return any(
        key in special_projects and special_projects.get(key) is not None
        for key in SPECIAL_PROJECTS_PAYLOAD_KEYS
    )


def _has_special_projects_rows(special_projects: Any) -> bool:
    if not isinstance(special_projects, dict):
        return False
    return any(
        isinstance(special_projects.get(key), list)
        for key in ("facilities", "projects", "scientists")
    )


def _claimed_scheduled_domains(
    frame: Dict[str, Any],
    root_probe: Any,
) -> set[str]:
    claims: set[str] = set()
    domains = frame.get("domains")
    if isinstance(domains, list):
        claims.update(
            domain for domain in domains
            if isinstance(domain, str) and domain in SCHEDULED_ONLY_SNAPSHOT_DOMAINS
        )
    if isinstance(root_probe, dict):
        root_domains = root_probe.get("domains")
        if isinstance(root_domains, list):
            claims.update(
                domain for domain in root_domains
                if (
                    isinstance(domain, str)
                    and domain in SCHEDULED_ONLY_SNAPSHOT_DOMAINS
                )
            )
    return claims


def _has_payload_rows(value: Any) -> bool:
    return isinstance(value, dict) and bool(value)


def _has_present_country_rows(countries: Any) -> bool:
    if not isinstance(countries, dict):
        return False
    return any(
        schema.valid_tag(tag) and isinstance(row, dict) and row.get("present") is True
        for tag, row in countries.items()
    )


def _has_country_scalar_rows(countries: Any) -> bool:
    if not isinstance(countries, dict):
        return False
    return any(
        schema.valid_tag(tag)
        and isinstance(row, dict)
        and isinstance(row.get("scalars"), dict)
        and any(value is not None for value in row["scalars"].values())
        for tag, row in countries.items()
    )


def _has_country_misc_rows(countries: Any) -> bool:
    if not isinstance(countries, dict):
        return False
    return any(
        schema.valid_tag(tag)
        and isinstance(row, dict)
        and isinstance(row.get("misc"), dict)
        and any(key in row["misc"] for key in COUNTRY_MISC_PAYLOAD_KEYS)
        for tag, row in countries.items()
    )


__all__ = [
    "COLLECTOR_ABI_VERSION", "COLLECTOR_BACKEND", "FRAME_TYPES",
    "SUPPORTED_SNAPSHOT_DOMAINS",
    "latest_verified_root_probe", "make_heartbeat_frame", "read_collector_jsonl",
    "snapshot_from_collector_frame", "snapshot_from_root_probe_frame",
    "validate_collector_frame",
]
