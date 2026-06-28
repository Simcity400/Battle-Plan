"""Adapter for in-process collector frames.

This module is the recorder-side contract for the new DLL path. It accepts
already-decoded collector frames, rejects malformed provenance/domains early,
and maps valid snapshot frames into the existing replay schema without reading
or requiring saves.
"""
from __future__ import annotations

import json
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

    root_probe = frame.get("root_probe")
    if root_probe is not None:
        if not isinstance(root_probe, dict):
            problems.append(f"collector frame root_probe {root_probe!r} not dict|null")
        else:
            problems.extend(_validate_root_probe(root_probe))

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

    snap = schema.empty_snapshot(game_id, date)
    snap["provenance"]["collector"] = _collector_provenance(frame, domains)
    snap["provenance"]["collector"]["root_probe"] = root_probe
    _fold_country_identity(snap, frame_countries)
    _fold_country_scalars(snap, frame_countries)
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
    snap["provenance"]["collector"] = _collector_provenance(frame, domains)
    _fold_country_identity(snap, frame.get("countries") or {})
    _fold_country_scalars(snap, frame.get("countries") or {})

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
            for key in ("stability", "war_support", "political_power",
                        "command_power", "convoys"):
                value = scalars.get(key)
                if value is not None and not isinstance(value, (int, float)):
                    problems.append(
                        f"collector frame country {tag!r}.scalars.{key} not number|null")
            for key in ("num_ships", "manpower_pool"):
                value = scalars.get(key)
                if value is not None and not isinstance(value, int):
                    problems.append(
                        f"collector frame country {tag!r}.scalars.{key} not int|null")
    return problems


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
            if isinstance(value, (int, float)):
                politics[key] = float(value)
        military = country["military"]
        num_ships = clean.get("num_ships")
        if isinstance(num_ships, int):
            military["num_ships"] = num_ships
        command_power = clean.get("command_power")
        if isinstance(command_power, (int, float)):
            military["command_power"] = float(command_power)
        manpower_pool = clean.get("manpower_pool")
        if isinstance(manpower_pool, int):
            manpower = military.get("manpower")
            if not isinstance(manpower, dict):
                manpower = {}
                military["manpower"] = manpower
            manpower["available"] = manpower_pool
        convoys = clean.get("convoys")
        if isinstance(convoys, (int, float)):
            military["convoys_count"] = int(round(convoys))


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


__all__ = [
    "COLLECTOR_ABI_VERSION", "COLLECTOR_BACKEND", "FRAME_TYPES",
    "latest_verified_root_probe", "make_heartbeat_frame", "read_collector_jsonl",
    "snapshot_from_collector_frame", "snapshot_from_root_probe_frame",
    "validate_collector_frame",
]
