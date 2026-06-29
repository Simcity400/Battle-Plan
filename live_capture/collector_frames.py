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
    "fuel_live",
    "production_live",
    "state_ownership",
    "state_control",
    "province_control",
})
SCHEDULED_ONLY_SNAPSHOT_DOMAINS = frozenset({
    "fuel_live",
    "production_live",
    "state_ownership",
    "state_control",
    "province_control",
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
PRODUCTION_FIXED_POINT_ABS_MAX = 1_000_000_000_000.0
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
PRODUCTION_PAYLOAD_KEYS = frozenset((
    "stockpile",
    "stockpile_total",
    "military_lines",
    "naval_lines",
    "military_factories",
    "naval_factories",
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
RAW_LAND_COMBAT_GAP_FIELDS = frozenset((
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
        if _has_payload_rows(states):
            problems.append("collector worker_poll frame must not carry states payload")
        if _has_payload_rows(provinces):
            problems.append("collector worker_poll frame must not carry provinces payload")
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
            scheduled_domain_claims & MAP_CONTROL_SNAPSHOT_DOMAINS
            or _has_payload_rows(states)
            or _has_payload_rows(provinces)
        )
    ):
        problems.append(
            "collector map-control payload requires scheduled_hour_snapshot")

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
    claimed_domains = _explicit_scheduled_domains(frame, root_probe)
    if "fuel_live" in claimed_domains and _has_country_fuel_rows(frame_countries):
        domains.append("fuel_live")
    if (
        "production_live" in claimed_domains
        and _has_country_production_rows(frame_countries)
    ):
        domains.append("production_live")
    if "state_ownership" in claimed_domains and _has_state_owner_rows(frame_states):
        domains.append("state_ownership")
    if "state_control" in claimed_domains and _has_state_control_rows(frame_states):
        domains.append("state_control")
    if "province_control" in claimed_domains and _has_province_control_rows(frame_provinces):
        domains.append("province_control")

    snap = schema.empty_snapshot(game_id, date)
    snap["provenance"]["collector"] = _collector_provenance(frame, domains)
    snap["provenance"]["collector"]["root_probe"] = root_probe
    _fold_country_identity(snap, frame_countries)
    _fold_country_scalars(snap, frame_countries)
    _fold_country_misc(snap, frame_countries, domains)
    _fold_country_fuel(snap, frame_countries, domains)
    _fold_country_production(snap, frame_countries, domains)
    _fold_state_rows(snap, frame_states, domains)
    _fold_province_rows(snap, frame_provinces, domains)
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
    _fold_state_rows(snap, frame.get("states") or {}, domains)
    _fold_province_rows(snap, frame.get("provinces") or {}, domains)
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
        state_id = row.get("state_id")
        if state_id is not None:
            if isinstance(state_id, bool) or not isinstance(state_id, int):
                problems.append(
                    f"collector frame province {pid!r}.state_id not int|null")
            elif state_id <= 0 or state_id > 100_000:
                problems.append(
                    f"collector frame province {pid!r}.state_id out of range")
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
        if clean:
            country = snap["countries"].setdefault(tag, schema.empty_country())
            country["production"].update(clean)


def _fold_state_rows(
    snap: Dict[str, Any],
    states: Dict[str, Any],
    domains: Iterable[str],
) -> None:
    domain_set = set(domains)
    fold_owner = "state_ownership" in domain_set
    fold_controller = "state_control" in domain_set
    if not (fold_owner or fold_controller):
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
        if clean:
            state = snap["states"].setdefault(str(int(sid)), schema.empty_state())
            state.update(clean)


def _fold_province_rows(
    snap: Dict[str, Any],
    provinces: Dict[str, Any],
    domains: Iterable[str],
) -> None:
    if "province_control" not in set(domains):
        return
    for pid, row in provinces.items():
        if not _valid_positive_int_key(pid, max_value=1_000_000) or not isinstance(row, dict):
            continue
        clean: Dict[str, Any] = {}
        if "controller" in row:
            controller = row.get("controller")
            clean["controller"] = controller if schema.valid_tag(controller) else None
        state_id = row.get("state_id")
        if isinstance(state_id, int) and not isinstance(state_id, bool):
            clean["state_id"] = state_id
        if clean:
            province = snap["provinces"].setdefault(str(int(pid)), schema.empty_province())
            province.update(clean)


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
