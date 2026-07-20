"""Strict, additive schema-v1 admission for exact Wave-4 replay carriers.

This module is deliberately pure-stdlib and side-effect free.  The carriers are
optional so recordings created before their introduction remain readable;
present non-null carriers, however, must satisfy their complete exact contract.
"""
from __future__ import annotations

import hashlib
import re
from typing import Any, Dict, List, Optional, Set, Tuple


EXECUTABLE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
A20_STATIC_RECEIPT_SHA256 = "f10a9cad9e53c660b50364f5a242891013be38f77cb873b870aede44da1d8f3a"
X9_PRODUCER_MANIFEST_SHA256 = "86f3aa3a916a203ea657cda01a84fb297e1812e03df59b481b004b16e0ad7ff6"
NUKE_RECEIPT_SHA256 = "3befb21facb2fdd5724602b2a72655ade23995b8270521aeecbe5eddeed34301"
NUKE_DATA_SHA256 = "20f780f52303e5c59868fcce8ee09258d4dc997bf5d3aadbdec3a3c00ce509bf"
DYNAMIC_MODIFIER_RECEIPT_SHA256 = "bdbac3cb1158c971b408586611d6aba618ecfe85b5265e3b9e8c2c0bf545e415"
FOCUS_MANIFEST_SHA256 = "303351ed65468aa7655b904998819c96ca1c325f906b5a3bf32c4f18decc7969"
FOCUS_RECEIPT_SHA256 = "c6c3fb002bc05f1e092d7eb9b436aa720ca2cf19ad08f055747216dfa5871b4f"
CHARACTER_HASHES = {
    "trait_collection_manifest_sha256": "837c8b4db6a6cbc03fa5c950dc24c814f5156d6cf0de08f5041bfcccab7d78c9",
    "trait_state_manifest_sha256": "63ca92c9e5238a0a342e24209417188b7ceeabd7f01a21c45475f58bb3e29e8c",
    "trait_progress_manifest_sha256": "134d8fc1d5056e05023ebd46e33bda913ba3dde6c1f0945d423297d464dae547",
    "ace_refresh_manifest_sha256": "4be836cc67b9f59d748a05a73fa4c5d9c46db357d283d25b756c7ad18fcb2674",
    "ace_bind_sort_manifest_sha256": "ea545ce9457982590bd42741d1f573a7874594b7c3f836bf290ae8a276f83396",
    "ace_modifier_setter_artifact_sha256": "3b49c9eb735a03fc711f7322b74a5b86ad64b25ffdbcaa73a390b64a2edfd552",
    "ace_modifier_formatter_manifest_sha256": "79fd3a1e1b9eafb09566b2721260936c8430acffcf0ba408cc866c6292ac8f0b",
}

_COUNTRY = re.compile(r"[A-Z0-9]{3}\Z")
_SHA256 = re.compile(r"[0-9a-f]{64}\Z")
_PORTABLE_ID = re.compile(r"[A-Za-z0-9_][A-Za-z0-9_.:@/-]{0,1023}\Z")
_COMBAT_EPOCH = re.compile(r"[1-9][0-9]*:[1-9][0-9]*:(?:attacker|defender)\Z")


def _is_int(value: Any) -> bool:
    return type(value) is int


def _uint(value: Any, maximum: int = 0xFFFFFFFF, *, positive: bool = False) -> bool:
    return _is_int(value) and (0 < value if positive else 0 <= value) and value <= maximum


def _i64(value: Any) -> bool:
    return _is_int(value) and -(1 << 63) <= value < (1 << 63)


def _country(value: Any) -> bool:
    return isinstance(value, str) and _COUNTRY.fullmatch(value) is not None


def _sha(value: Any) -> bool:
    return (isinstance(value, str) and _SHA256.fullmatch(value) is not None
            and any(ch != "0" for ch in value))


def _portable(value: Any, maximum: int = 1024) -> bool:
    return (isinstance(value, str) and 0 < len(value) <= maximum
            and _PORTABLE_ID.fullmatch(value) is not None and ".." not in value)


def _exact_keys(value: Any, expected: Set[str], path: str, problems: List[str]) -> Optional[Dict[str, Any]]:
    if not isinstance(value, dict):
        problems.append(f"{path} should be an object")
        return None
    if set(value) != expected:
        problems.append(f"{path} has wrong exact field set")
    return value


def _d18_map(snap: Dict[str, Any]) -> Dict[str, int]:
    out: Dict[str, int] = {}
    countries = snap.get("countries")
    if isinstance(countries, dict):
        for tag, country in countries.items():
            if _country(tag) and isinstance(country, dict):
                ident = country.get("_r2_tag_db_id")
                if _uint(ident, positive=True):
                    out[tag] = ident
    return out


def validate_wave4_snapshot(snap: Dict[str, Any]) -> List[str]:
    problems: List[str] = []
    _validate_a17(snap, problems)
    _validate_a18(snap, problems)
    _validate_a20_x9(snap, problems)
    _validate_a23(snap, problems)
    _validate_a25(snap, problems)
    _validate_a30(snap, problems)
    _validate_a31(snap, problems)
    _validate_a39(snap, problems)
    _validate_d56(snap, problems)
    _validate_x2(snap, problems)
    _validate_weather(snap, problems)
    return problems


def _nuke_built_count(amount_raw: int) -> int:
    unsigned = amount_raw & ((1 << 64) - 1)
    condition = ((unsigned + 0x7FFFFFFFFFFFFFFE) & ((1 << 64) - 1)) < 0xFFFFFFFFFFFFFFFD
    if not condition:
        return 0xA7C5
    product = (amount_raw * 100_000) & ((1 << 64) - 1)
    if product >= 1 << 63:
        product -= 1 << 64
    quotient = abs(product) // 10_000_000_000_000
    return -quotient if product < 0 else quotient


def _validate_a17(snap: Dict[str, Any], problems: List[str]) -> None:
    if "_r2_nuke_stockpile_live" not in snap or snap.get("_r2_nuke_stockpile_live") is None:
        return
    path = "_r2_nuke_stockpile_live"
    root = _exact_keys(snap[path], {
        "schema_version", "authority", "executable_sha256", "receipt_sha256",
        "data_artifact_sha256", "elapsed_wall_ms", "frame_hour", "source_sequence",
        "countries",
    }, path, problems)
    if root is None:
        return
    if (root.get("schema_version") != 1
            or root.get("authority") != "exact_a729_nuke_stockpile"
            or root.get("executable_sha256") != EXECUTABLE_SHA256
            or root.get("receipt_sha256") != NUKE_RECEIPT_SHA256
            or root.get("data_artifact_sha256") != NUKE_DATA_SHA256
            or root.get("frame_hour") != snap.get("hour_index")
            or not _uint(root.get("elapsed_wall_ms"), (1 << 64) - 1)
            or not _uint(root.get("source_sequence"), (1 << 64) - 1, positive=True)):
        problems.append(f"{path} authority/frame/header invalid")
    rows = root.get("countries")
    if not isinstance(rows, list) or len(rows) > 2048:
        problems.append(f"{path}.countries should be a bounded list")
        return
    seen: Set[int] = set()
    countries = snap.get("countries") if isinstance(snap.get("countries"), dict) else {}
    for index, raw in enumerate(rows):
        rpath = f"{path}.countries[{index}]"
        row = _exact_keys(raw, {"tag_db_id", "nukes_total", "nukes_ready_total", "nukes"}, rpath, problems)
        if row is None:
            continue
        tag_db_id = row.get("tag_db_id")
        entries = row.get("nukes")
        if not _uint(tag_db_id, positive=True) or tag_db_id in seen:
            problems.append(f"{rpath}.tag_db_id invalid or duplicate")
            continue
        seen.add(tag_db_id)
        if not isinstance(entries, list) or len(entries) > 32:
            problems.append(f"{rpath}.nukes should be a bounded list")
            continue
        built = 0
        ready = 0
        canonical = []
        for entry_index, raw_entry in enumerate(entries):
            epath = f"{rpath}.nukes[{entry_index}]"
            entry = _exact_keys(raw_entry, {"amount_raw", "amount", "nukes_ready"}, epath, problems)
            if entry is None:
                continue
            amount_raw = entry.get("amount_raw")
            amount = entry.get("amount")
            nukes_ready = entry.get("nukes_ready")
            if not _i64(amount_raw) or not _is_int(amount) or not _uint(nukes_ready):
                problems.append(f"{epath} has invalid integer fields")
                continue
            expected_amount = _nuke_built_count(amount_raw)
            if amount != expected_amount:
                problems.append(f"{epath}.amount disagrees with exact fixed-point conversion")
            built = ((built + expected_amount + (1 << 31)) % (1 << 32)) - (1 << 31)
            ready += nukes_ready
            canonical.append({"amount": amount, "nukes_ready": nukes_ready})
        if (not _is_int(row.get("nukes_total"))
                or not -(1 << 31) <= row.get("nukes_total") < (1 << 31)
                or not _uint(row.get("nukes_ready_total"), (1 << 64) - 1)
                or row.get("nukes_total") != built or row.get("nukes_ready_total") != ready):
            problems.append(f"{rpath} mechanical totals disagree")
        matches = []
        for tag, country_obj in countries.items():
            extra = country_obj.get("extra") if isinstance(country_obj, dict) else None
            if isinstance(extra, dict) and extra.get("nuke_stockpile_tag_db_id") == tag_db_id:
                matches.append((tag, extra))
        if len(matches) != 1:
            problems.append(f"{rpath} does not join exactly one canonical country")
        else:
            tag, extra = matches[0]
            if (extra.get("nukes_total") != built
                    or extra.get("nukes_ready_total") != ready
                    or extra.get("nukes") != canonical):
                problems.append(f"{rpath} disagrees with canonical country projection")
            country_obj = countries.get(tag)
            if (isinstance(country_obj, dict) and country_obj.get("_r2_tag_db_id") is not None
                    and country_obj.get("_r2_tag_db_id") != tag_db_id):
                problems.append(f"{rpath} disagrees with current D18 country identity")
    for tag, country_obj in countries.items():
        extra = country_obj.get("extra") if isinstance(country_obj, dict) else None
        live_id = extra.get("nuke_stockpile_tag_db_id") if isinstance(extra, dict) else None
        if live_id is not None and live_id not in seen:
            problems.append(f"countries.{tag}.extra has an orphan A17 canonical projection")


def _validate_a18(snap: Dict[str, Any], problems: List[str]) -> None:
    key = "_r2_dynamic_modifier_lifecycle_live"
    if key not in snap or snap.get(key) is None:
        return
    root = _exact_keys(snap[key], {
        "schema_version", "authority", "source_binary_sha256",
        "static_authority_receipt_sha256", "elapsed_wall_ms", "frame_hour",
        "continuity", "transitions",
    }, key, problems)
    if root is None:
        return
    if (root.get("schema_version") != 2
            or root.get("authority") != "exact_a729_dynamic_modifier_instance"
            or root.get("source_binary_sha256") != EXECUTABLE_SHA256
            or root.get("static_authority_receipt_sha256") != DYNAMIC_MODIFIER_RECEIPT_SHA256
            or root.get("frame_hour") != snap.get("hour_index")
            or root.get("continuity") not in {"baseline", "continuous"}
            or not _uint(root.get("elapsed_wall_ms"), (1 << 64) - 1)):
        problems.append(f"{key} authority/frame/header invalid")
    measured = snap.get("measured_countries")
    measured_map: Dict[int, str] = {}
    roster_valid = False
    if isinstance(measured, dict) and measured.get("known") is True:
        ids, tags = measured.get("tag_db_ids"), measured.get("tags")
        if (measured.get("source") == "country_identity" and isinstance(ids, list)
                and isinstance(tags, list) and len(ids) == len(tags)
                and measured.get("count") == len(ids)):
            roster_valid = True
            seen_tags: Set[str] = set()
            for ident, tag in zip(ids, tags):
                if (_uint(ident, positive=True) and _country(tag)
                        and ident not in measured_map and tag not in seen_tags):
                    measured_map[ident] = tag
                    seen_tags.add(tag)
                else:
                    roster_valid = False
    if not roster_valid:
        problems.append(f"{key} requires a current measured-country identity roster")
    current: Dict[Tuple[str, str], Optional[int]] = {}
    countries = snap.get("countries") if isinstance(snap.get("countries"), dict) else {}
    total_instances = 0
    for tag, country_obj in countries.items():
        rows = country_obj.get("dynamic_modifier_instances") if isinstance(country_obj, dict) else None
        if not isinstance(rows, list):
            problems.append(f"countries.{tag}.dynamic_modifier_instances must be measured beside DMI2")
            continue
        seen_modifiers: Set[str] = set()
        total_instances += len(rows)
        for ordinal, raw in enumerate(rows):
            path = f"countries.{tag}.dynamic_modifier_instances[{ordinal}]"
            row = _exact_keys(raw, {"source_ordinal", "modifier", "enabled", "values_raw", "value_scale", "remaining_days"}, path, problems)
            if row is None:
                continue
            modifier = row.get("modifier")
            remaining = row.get("remaining_days")
            if (row.get("source_ordinal") != ordinal or not _portable(modifier, 128)
                    or modifier in seen_modifiers or type(row.get("enabled")) is not bool
                    or not isinstance(row.get("values_raw"), list)
                    or len(row.get("values_raw")) > 256
                    or not all(_i64(v) for v in row.get("values_raw"))
                    or row.get("value_scale") != 100_000
                    or not (remaining is None or _uint(remaining, 0x7FFFFFFF))):
                problems.append(f"{path} exact instance shape invalid")
                continue
            seen_modifiers.add(modifier)
            current[(tag, modifier)] = remaining
        if rows and tag not in measured_map.values():
            problems.append(f"countries.{tag}.dynamic_modifier_instances owner is not measured")
    if total_instances > 262_144:
        problems.append(f"{key} current instance count exceeds compact bound")
    transitions = root.get("transitions")
    if not isinstance(transitions, list) or len(transitions) > 524_288:
        problems.append(f"{key}.transitions should be a bounded list")
        return
    seen_transitions: Set[Tuple[int, str]] = set()
    for index, raw in enumerate(transitions):
        path = f"{key}.transitions[{index}]"
        row = _exact_keys(raw, {"owner_tag_db_id", "owner_tag", "modifier", "kind", "previous_remaining_days", "current_remaining_days", "expiry_classification"}, path, problems)
        if row is None:
            continue
        owner_id, owner_tag = row.get("owner_tag_db_id"), row.get("owner_tag")
        modifier, kind = row.get("modifier"), row.get("kind")
        previous, now, expiry = row.get("previous_remaining_days"), row.get("current_remaining_days"), row.get("expiry_classification")
        valid_days = lambda value: value is None or _uint(value, 0x7FFFFFFF)
        shape = {
            "baseline_present": previous is None and expiry is None,
            "added": previous is None and expiry is None,
            "zero_observed": _uint(previous, 0x7FFFFFFF, positive=True) and now == 0 and expiry is None,
            "timed_counter_changed": valid_days(previous) and previous is not None and _uint(now, 0x7FFFFFFF, positive=True) and previous != now and expiry is None,
            "duration_kind_changed": (previous is None) != (now is None) and expiry == "unavailable",
            "removed": now is None and ((previous == 0 and expiry == "removed_after_observed_zero") or (previous != 0 and expiry == "unavailable_without_zero_observation")),
        }.get(kind, False)
        if (not _uint(owner_id, 2048, positive=True) or measured_map.get(owner_id) != owner_tag
                or not _portable(modifier, 128) or not valid_days(previous) or not valid_days(now)
                or (owner_id, modifier) in seen_transitions or not shape
                or ((root.get("continuity") == "baseline") != (kind == "baseline_present"))):
            problems.append(f"{path} transition identity/shape invalid")
            continue
        seen_transitions.add((owner_id, modifier))
        current_value = current.get((owner_tag, modifier), object())
        if kind == "removed":
            joined = (owner_tag, modifier) not in current
        else:
            joined = current_value == now
        if not joined:
            problems.append(f"{path} disagrees with current instance projection")
    if root.get("continuity") == "baseline" and set(current) != {
        (row.get("owner_tag"), row.get("modifier"))
        for row in transitions if isinstance(row, dict)
    }:
        problems.append(f"{key} baseline transitions do not cover the current instance set")


def _validate_a20_x9(snap: Dict[str, Any], problems: List[str]) -> None:
    special = snap.get("special_projects")
    if not isinstance(special, dict):
        return
    authority = special.get("_r2_project_authority")
    progress_value = special.get("project_progress")
    progress_rows = progress_value.get("row_count") if isinstance(progress_value, dict) else None
    progress_walked = progress_value.get("walked_countries") if isinstance(progress_value, dict) else None
    progress_aborted = progress_value.get("aborted_countries") if isinstance(progress_value, dict) else None
    breakthroughs_value = special.get("_r2_breakthroughs")
    # PPA1 was added after the two inner carriers.  Inner-only snapshots remain
    # structurally readable as legacy/unverified data, but they are not exact:
    # replay_core requires this outer authority before exposing either exact
    # A20 progress or exact X9 breakthrough views.  When PPA1 is present, keep
    # admission fail-closed and require both complete inner carrier shapes.
    if authority is not None:
        path = "special_projects._r2_project_authority"
        root = _exact_keys(authority, {"schema", "status", "record_sequence", "game_hour_raw", "source_status_flags", "executable_sha256", "a20_static_receipt_sha256", "x9_producer_manifest_sha256", "d18_identity_sha256", "d18_country_count", "owner_progress_row_count", "breakthrough_row_count", "walked_countries", "aborted_countries", "authority_flags", "carrier_payload_sha256"}, path, problems)
        d18 = _d18_map(snap)
        preimage = bytearray(b"D18R1\0")
        preimage.extend(len(d18).to_bytes(4, "little"))
        for tag, ident in sorted(d18.items(), key=lambda row: row[1]):
            encoded = tag.encode("utf-8")
            preimage.extend(ident.to_bytes(4, "little"))
            preimage.extend(len(encoded).to_bytes(2, "little"))
            preimage.extend(encoded)
        digest = hashlib.sha256(preimage).hexdigest()
        expected_breakthroughs = len(breakthroughs_value) if isinstance(breakthroughs_value, list) else None
        all_specializations_exact = isinstance(breakthroughs_value, list) and all(
            isinstance(row, dict)
            and row.get("specialization") in {"specialization_nuclear", "specialization_naval", "specialization_air", "specialization_land"}
            and row.get("_r2_specialization_authority") == "exact_a729_serializer_token_enum_join"
            and "_r2_specialization_handle" not in row
            for row in breakthroughs_value
        )
        if (root is None or root.get("schema") != "a729_d41_project_authority_v1"
                or root.get("status") != "exact" or root.get("authority_flags") != 0x0F
                or not _uint(root.get("record_sequence"), positive=True)
                or type(root.get("game_hour_raw")) is not int
                or not _uint(root.get("source_status_flags"))
                or root.get("executable_sha256") != EXECUTABLE_SHA256
                or root.get("a20_static_receipt_sha256") != A20_STATIC_RECEIPT_SHA256
                or root.get("x9_producer_manifest_sha256") != X9_PRODUCER_MANIFEST_SHA256
                or root.get("d18_identity_sha256") != digest
                or root.get("d18_country_count") != len(d18)
                or root.get("owner_progress_row_count") != progress_rows
                or root.get("breakthrough_row_count") != expected_breakthroughs
                or root.get("walked_countries") != progress_walked
                or root.get("aborted_countries") != progress_aborted
                or not all_specializations_exact
                or root.get("aborted_countries") != 0
                or not _sha(root.get("carrier_payload_sha256"))):
            problems.append(f"{path} exact outer authority/cross-join invalid")
    progress = special.get("project_progress")
    if progress is not None:
        path = "special_projects.project_progress"
        root = _exact_keys(progress, {"authority", "countries", "row_count", "walked_countries", "aborted_countries"}, path, problems)
        if root is not None:
            countries = root.get("countries")
            if (root.get("authority") != "exact_a729_owner_joined_project_progress"
                    or not isinstance(countries, dict) or root.get("aborted_countries") != 0
                    or not _uint(root.get("walked_countries"))
                    or not _uint(root.get("row_count"))):
                problems.append(f"{path} incomplete exact authority")
            elif root.get("walked_countries") != len(countries):
                problems.append(f"{path}.walked_countries disagrees with country catalog")
            else:
                count = 0
                d18 = _d18_map(snap)
                for tag, rows in countries.items():
                    if not _country(tag) or not isinstance(rows, list) or tag not in d18:
                        problems.append(f"{path}.countries.{tag} invalid")
                        continue
                    seen: Set[int] = set()
                    previous_local_id = 0
                    for index, raw in enumerate(rows):
                        rpath = f"{path}.countries.{tag}[{index}]"
                        row = _exact_keys(raw, {"local_id", "static_def_handle", "current_state_offset", "current_state_enum", "prototype_state_enum", "simple_state_1_enum", "simple_state_2_enum", "simple_state_3_enum", "prototype_iterations", "phase_progress_raw", "phase_target_raw", "completed"}, rpath, problems)
                        if row is None:
                            continue
                        values = [row.get(name) for name in ("local_id", "static_def_handle", "current_state_offset", "current_state_enum", "prototype_state_enum", "simple_state_1_enum", "simple_state_2_enum", "simple_state_3_enum", "prototype_iterations", "phase_progress_raw", "phase_target_raw")]
                        valid = (all(_uint(v) for v in values) and values[0] > previous_local_id and values[0] not in seen
                                 and values[1] > 0 and values[2] in {0x18, 0xE8, 0x168, 0x1E8}
                                 and all(v <= 64 for v in values[3:8]) and values[8] <= 1_000_000
                                 and values[9] <= 1_000_000_000 and 0 < values[10] <= 1_000_000
                                 and type(row.get("completed")) is bool)
                        if not valid:
                            problems.append(f"{rpath} exact bounds/identity invalid")
                        else:
                            seen.add(values[0])
                            previous_local_id = values[0]
                        count += 1
                if count > 30_000:
                    problems.append(f"{path}.row_count exceeds compact producer bound")
                if root.get("row_count") != count:
                    problems.append(f"{path}.row_count mismatch")
    breakthroughs = special.get("_r2_breakthroughs")
    if breakthroughs is not None:
        if not isinstance(breakthroughs, list):
            problems.append("special_projects._r2_breakthroughs should be list|null")
            return
        d18 = _d18_map(snap)
        seen: Set[Tuple[int, str]] = set()
        for index, row in enumerate(breakthroughs):
            path = f"special_projects._r2_breakthroughs[{index}]"
            exact = _exact_keys(row, {"country", "tag_db_id", "specialization", "points_raw", "_r2_specialization_authority", "_r2_country_authority"}, path, problems)
            if exact is None:
                continue
            tag, ident, specialization = exact.get("country"), exact.get("tag_db_id"), exact.get("specialization")
            unavailable_specialization = (specialization is None
                and exact.get("_r2_specialization_authority") is None)
            exact_specialization = (specialization in {"specialization_nuclear", "specialization_naval", "specialization_air", "specialization_land"}
                and exact.get("_r2_specialization_authority") == "exact_a729_serializer_token_enum_join")
            if (not _country(tag) or not _uint(ident, positive=True) or d18.get(tag) != ident
                    or not (unavailable_specialization or exact_specialization)
                    or exact.get("_r2_country_authority") != "exact_country_identity"
                    or not _uint(exact.get("points_raw"))
                    or (exact_specialization and (ident, specialization) in seen)):
                problems.append(f"{path} exact specialization/D18 identity invalid")
            elif exact_specialization:
                seen.add((ident, specialization))


def _validate_a23(snap: Dict[str, Any], problems: List[str]) -> None:
    countries = snap.get("countries")
    if not isinstance(countries, dict):
        return
    for tag, country in countries.items():
        if not isinstance(country, dict) or "special_project_programs" not in country:
            continue
        rows = country.get("special_project_programs")
        if rows is None:
            continue
        path = f"countries.{tag}.special_project_programs"
        if not isinstance(rows, list):
            problems.append(f"{path} should be list|null")
            continue
        seen: Set[int] = set()
        for index, row in enumerate(rows):
            rpath = f"{path}[{index}]"
            exact = _exact_keys(row, {"program_id", "province", "unread_prototype_rewards"}, rpath, problems)
            if exact is None:
                continue
            program_id = exact.get("program_id")
            if (not _uint(program_id, positive=True) or program_id in seen
                    or not _uint(exact.get("province"))
                    or not _uint(exact.get("unread_prototype_rewards"))):
                problems.append(f"{rpath} owner-local identity/bounds invalid")
            else:
                seen.add(program_id)


def _validate_a25(snap: Dict[str, Any], problems: List[str]) -> None:
    if "runtime_text" not in snap or snap.get("runtime_text") is None:
        return
    path = "runtime_text"
    root = _exact_keys(snap[path], {"schema", "state", "contexts_complete", "contexts"}, path, problems)
    if root is None:
        return
    contexts = root.get("contexts")
    if (root.get("schema") != "hoi4-runtime-text-v1" or root.get("state") != "measured"
            or root.get("contexts_complete") is not True
            or not isinstance(contexts, list) or len(contexts) > 131_072):
        problems.append(f"{path} exact measured-complete envelope invalid")
        return
    seen: Set[str] = set()
    for index, raw in enumerate(contexts):
        cpath = f"{path}.contexts[{index}]"
        row = _exact_keys(raw, {"context_id", "frame_hour", "pov_country_or_global", "view_id", "element_id", "source", "final_text", "variables", "data_functions", "dynamic_colors", "layout", "producer"}, cpath, problems)
        if row is None:
            continue
        context_id = row.get("context_id")
        if (not _portable(context_id, 4096) or context_id in seen
                or row.get("frame_hour") != snap.get("hour_index")
                or not (row.get("pov_country_or_global") == "global"
                        or _country(row.get("pov_country_or_global")))
                or not _portable(row.get("view_id"), 4096)
                or not _portable(row.get("element_id"), 4096)):
            problems.append(f"{cpath} context/frame/identity invalid")
        else:
            seen.add(context_id)
        source = _exact_keys(row.get("source"), set(row.get("source", {})) if isinstance(row.get("source"), dict) else set(), f"{cpath}.source", problems)
        if source is not None:
            if source.get("kind") == "authored_token":
                if set(source) != {"kind", "identity"} or not _portable(source.get("identity"), 4096):
                    problems.append(f"{cpath}.source authored identity invalid")
            elif source.get("kind") == "setter_caller_rva":
                if set(source) != {"kind", "rva"} or not _uint(source.get("rva"), positive=True):
                    problems.append(f"{cpath}.source setter caller invalid")
            else:
                problems.append(f"{cpath}.source kind invalid")
        final = _exact_keys(row.get("final_text"), {"bytes_hex", "utf8", "sha256"}, f"{cpath}.final_text", problems)
        if final is not None:
            hex_text = final.get("bytes_hex")
            try:
                if (not isinstance(hex_text, str) or len(hex_text) % 2
                        or re.fullmatch(r"[0-9a-f]*", hex_text) is None):
                    raise ValueError("noncanonical hex")
                encoded = bytes.fromhex(hex_text)
                decoded = encoded.decode("utf-8")
            except (ValueError, UnicodeDecodeError):
                encoded, decoded = b"", None
            if (decoded is None or decoded != final.get("utf8") or not _sha(final.get("sha256"))
                    or hashlib.sha256(encoded).hexdigest() != final.get("sha256")):
                problems.append(f"{cpath}.final_text bytes/UTF-8/hash disagree")
        for name in ("variables", "data_functions", "dynamic_colors"):
            if row.get(name) != {}:
                problems.append(f"{cpath}.{name} must be the exact measured-empty object")
        layout = row.get("layout")
        if not isinstance(layout, dict) or layout.get("mode") not in {"authored_reproducible", "captured_runtime"}:
            problems.append(f"{cpath}.layout invalid")
        elif layout.get("mode") == "authored_reproducible" and set(layout) != {"mode"}:
            problems.append(f"{cpath}.layout authored mode has extra fields")
        elif layout.get("mode") == "captured_runtime":
            if set(layout) != {"mode", "clip", "glyphs", "inline_icons"} or not isinstance(layout.get("glyphs"), list) or not isinstance(layout.get("inline_icons"), list):
                problems.append(f"{cpath}.layout captured mode invalid")
        producer = _exact_keys(row.get("producer"), {"setter_entry_rva", "setter_caller_rva", "formatter_entry_rva", "identity_source"}, f"{cpath}.producer", problems)
        if producer is not None:
            expected_source = source.get("kind") if isinstance(source, dict) else None
            expected_identity = "authored_widget_registration" if expected_source == "authored_token" else "proven_setter_caller"
            caller = producer.get("setter_caller_rva")
            formatter = producer.get("formatter_entry_rva")
            if (producer.get("setter_entry_rva") != 0x1732DAD0
                    or producer.get("identity_source") != expected_identity
                    or not (caller is None or _uint(caller, positive=True))
                    or not (formatter is None or formatter == 0x1732CB20)
                    or (expected_source == "setter_caller_rva" and caller != source.get("rva"))):
                problems.append(f"{cpath}.producer exact authority invalid")


def _validate_a30(snap: Dict[str, Any], problems: List[str]) -> None:
    world = snap.get("diplomacy_world")
    extra = world.get("domain53_extra") if isinstance(world, dict) else None
    if not isinstance(extra, dict):
        return
    d18 = _d18_map(snap)
    operations = extra.get("operation_instance_rows")
    if operations is not None:
        path = "diplomacy_world.domain53_extra.operation_instance_rows"
        if not isinstance(operations, list) or len(operations) > 131_072:
            problems.append(f"{path} should be a bounded list|null")
        else:
            seen_ops: Set[int] = set()
            assigned: Set[int] = set()
            for index, row in enumerate(operations):
                rpath = f"{path}[{index}]"
                expected = {"stable_operation_id", "operation_id", "definition_id", "owner_country", "owner_country_tag_db_id", "target_country_or_state", "target_country_tag_db_id", "status", "phase_index", "phase_count", "progress_fixed", "progress_scale", "start_hour", "duration_hours", "time_units", "assigned_operative_ids"}
                exact = _exact_keys(row, expected, rpath, problems)
                if exact is None:
                    continue
                stable = exact.get("stable_operation_id")
                owner, owner_id = exact.get("owner_country"), exact.get("owner_country_tag_db_id")
                phase, phases = exact.get("phase_index"), exact.get("phase_count")
                status = exact.get("status")
                preparing = status in {"preparing", "prepared"}
                target = exact.get("target_country_or_state")
                target_valid = False
                if isinstance(target, dict) and target.get("kind") == "country" and set(target) == {"kind", "country", "tag_db_id"}:
                    target_valid = (_country(target.get("country")) and _uint(target.get("tag_db_id"), positive=True)
                                    and target.get("tag_db_id") == exact.get("target_country_tag_db_id")
                                    and (not d18 or d18.get(target.get("country")) == target.get("tag_db_id")))
                elif isinstance(target, dict) and target.get("kind") == "state" and set(target) == {"kind", "state_id"}:
                    target_valid = _uint(target.get("state_id"), positive=True)
                op_assigned = exact.get("assigned_operative_ids")
                valid_assigned = isinstance(op_assigned, list) and len(op_assigned) <= 64
                if valid_assigned:
                    local: Set[int] = set()
                    for operative_id in op_assigned:
                        if (not _uint(operative_id, positive=True)
                                or operative_id in local or operative_id in assigned):
                            valid_assigned = False
                            break
                        local.add(operative_id)
                        assigned.add(operative_id)
                valid = (_uint(stable, positive=True) and stable not in seen_ops
                         and exact.get("operation_id") == f"operation:{stable}"
                         and _portable(exact.get("definition_id"), 256) and _country(owner)
                         and _uint(owner_id, positive=True) and (not d18 or d18.get(owner) == owner_id)
                         and _uint(exact.get("target_country_tag_db_id"), positive=True) and target_valid
                         and status in {"preparing", "prepared", "running", "finished"}
                         and _uint(phases, 32, positive=True) and _uint(phase, 31) and phase < phases
                         and exact.get("progress_scale") == 100_000
                         and _is_int(exact.get("progress_fixed")) and 0 <= exact.get("progress_fixed") <= 100_000
                         and exact.get("time_units") == "game_hours" and _uint(exact.get("start_hour"))
                         and _uint(exact.get("duration_hours")) and valid_assigned)
                if preparing and (exact.get("start_hour") != 0 or exact.get("duration_hours") != 0 or phase != 0):
                    valid = False
                if status == "prepared" and exact.get("progress_fixed") != 100_000:
                    valid = False
                if status == "finished" and (phase + 1 != phases or exact.get("progress_fixed") != 100_000):
                    valid = False
                if not valid:
                    problems.append(f"{rpath} exact operation identity/state invalid")
                else:
                    seen_ops.add(stable)
    levels = extra.get("intel_level_rows")
    if levels is not None:
        path = "diplomacy_world.domain53_extra.intel_level_rows"
        if not isinstance(levels, list):
            problems.append(f"{path} should be list|null")
        else:
            seen_pairs: Set[Tuple[int, int]] = set()
            expected = {"owner", "target", "owner_tag_db_id", "target_tag_db_id", "civilian_raw", "army_raw", "navy_raw", "air_raw", "scale", "lane_order", "producer"}
            for index, row in enumerate(levels):
                rpath = f"{path}[{index}]"
                exact = _exact_keys(row, expected, rpath, problems)
                if exact is None:
                    continue
                owner, target = exact.get("owner"), exact.get("target")
                owner_id, target_id = exact.get("owner_tag_db_id"), exact.get("target_tag_db_id")
                pair = (owner_id, target_id)
                values = [exact.get(k) for k in ("civilian_raw", "army_raw", "navy_raw", "air_raw")]
                if (not _country(owner) or not _country(target) or not _uint(owner_id, positive=True)
                        or not _uint(target_id, positive=True) or pair in seen_pairs
                        or (d18 and (d18.get(owner) != owner_id or d18.get(target) != target_id))
                        or exact.get("scale") != 100_000 or not all(_is_int(v) and 0 <= v <= 100_000 for v in values)
                        or exact.get("lane_order") != ["civilian", "army", "navy", "air"]
                        or exact.get("producer") != "FUN_141417F60"):
                    problems.append(f"{rpath} exact intel level identity/scale invalid")
                else:
                    seen_pairs.add(pair)
    _validate_a30_networks(extra.get("intel_network_graph_rows"), d18, problems)
    _validate_a30_refreshes(extra, snap.get("hour_index"), d18, problems)


def _validate_a30_networks(value: Any, d18: Dict[str, int], problems: List[str]) -> None:
    if value is None:
        return
    path = "diplomacy_world.domain53_extra.intel_network_graph_rows"
    if not isinstance(value, list):
        problems.append(f"{path} should be list|null")
        return
    seen: Set[Tuple[int, int]] = set()
    expected = {"owner", "owner_tag_db_id", "strategic_operative_index", "network_slot", "target", "target_tag_db_id", "state_ids", "vertex_count", "state_strengths", "state_strength_scale", "state_strength_producer", "national_coverage"}
    for index, row in enumerate(value):
        rpath = f"{path}[{index}]"
        exact = _exact_keys(row, expected, rpath, problems)
        if exact is None:
            continue
        owner, owner_id = exact.get("owner"), exact.get("owner_tag_db_id")
        strategic, slot = exact.get("strategic_operative_index"), exact.get("network_slot")
        states = exact.get("state_ids")
        target, target_id = exact.get("target"), exact.get("target_tag_db_id")
        owner_valid = ((owner is None and owner_id is None)
                       or (_country(owner) and _uint(owner_id, positive=True)
                           and owner_id == strategic and (not d18 or d18.get(owner) == owner_id)))
        states_valid = (isinstance(states, list) and bool(states)
                        and all(_uint(state, positive=True) for state in states)
                        and all(left < right for left, right in zip(states, states[1:])))
        strengths = exact.get("state_strengths")
        if strengths is None:
            strengths_valid = exact.get("state_strength_scale") is None and exact.get("state_strength_producer") is None
        else:
            strengths_valid = (isinstance(strengths, list) and len(strengths) == len(states or [])
                               and exact.get("state_strength_scale") == 100_000
                               and exact.get("state_strength_producer") == "FUN_1411ACDD0/FUN_1411B2420/FUN_1411BDE90/FUN_1411C1FE0")
            if strengths_valid:
                for state, strength in zip(states, strengths):
                    strengths_valid &= (isinstance(strength, dict) and set(strength) == {"state_id", "strength_raw"}
                                        and strength.get("state_id") == state
                                        and _is_int(strength.get("strength_raw"))
                                        and 0 <= strength.get("strength_raw") <= 100_000)
        coverage_valid = _validate_a30_coverage(exact.get("national_coverage"))
        identity = (strategic, slot)
        if (not owner_valid or not _uint(strategic, positive=True) or not _uint(slot)
                or identity in seen or not _country(target) or not _uint(target_id, positive=True)
                or (d18 and d18.get(target) != target_id) or not states_valid
                or exact.get("vertex_count") != len(states or []) or not strengths_valid
                or not coverage_valid):
            problems.append(f"{rpath} exact network identity/graph invalid")
        else:
            seen.add(identity)


def _validate_a30_coverage(value: Any) -> bool:
    if value is None:
        return True
    expected = {"covered_core_states", "covered_controlled_states", "covered_worth", "controlled_strength_sum_raw", "core_strength_sum_raw", "total_core_states", "total_controlled_states", "total_worth", "weight_controlled_raw", "weight_owned_raw", "weight_owned_worth_raw", "national_coverage_raw", "scale", "producer"}
    if not isinstance(value, dict) or set(value) != expected:
        return False
    unsigned = ("covered_core_states", "covered_controlled_states", "covered_worth", "total_core_states", "total_controlled_states", "total_worth")
    signed_nonnegative = ("controlled_strength_sum_raw", "core_strength_sum_raw", "weight_controlled_raw", "weight_owned_raw", "weight_owned_worth_raw")
    if (not all(_uint(value.get(k)) for k in unsigned)
            or not all(_i64(value.get(k)) and value.get(k) >= 0 for k in signed_nonnegative)
            or value["covered_core_states"] > value["total_core_states"]
            or value["covered_controlled_states"] > value["total_controlled_states"]
            or value["covered_worth"] > value["total_worth"]
            or not _is_int(value.get("national_coverage_raw"))
            or not 0 <= value["national_coverage_raw"] <= 100_000
            or value.get("scale") != 100_000
            or value.get("producer") != "FUN_1411BD3A0/FUN_1411BD440"):
        return False
    def ratio(count: int, total: int, weight: int) -> int:
        return 0 if total == 0 else count * weight // total
    def average(total: int, count: int) -> int:
        return 0 if count == 0 else total // (count * 100)
    controlled = average(value["controlled_strength_sum_raw"], value["covered_controlled_states"])
    core = average(value["core_strength_sum_raw"], value["covered_core_states"])
    computed = controlled * ratio(value["covered_controlled_states"], value["total_controlled_states"], value["weight_controlled_raw"]) // 100_000
    computed += core * ratio(value["covered_core_states"], value["total_core_states"], value["weight_owned_raw"]) // 100_000
    computed += controlled * ratio(value["covered_worth"], value["total_worth"], value["weight_owned_worth_raw"]) // 100_000
    return max(0, min(100_000, computed)) == value["national_coverage_raw"]


def _validate_a30_refreshes(extra: Dict[str, Any], hour: Any, d18: Dict[str, int], problems: List[str]) -> None:
    value = extra.get("agency_upgrade_controller_refresh")
    if value is not None:
        path = "diplomacy_world.domain53_extra.agency_upgrade_controller_refresh"
        root = _exact_keys(value, {"schema_version", "frame_hour", "owner", "owner_tag_db_id", "producer_record_identity", "authority", "branches"}, path, problems)
        if root is not None:
            owner, owner_id = root.get("owner"), root.get("owner_tag_db_id")
            authority = root.get("authority")
            expected_authority = {"executable_sha256": EXECUTABLE_SHA256, "refresh_producer": "FUN_141A03550", "branch_materializer": "FUN_141A004B0", "filter_producer": "FUN_140617730", "definition_id_producer": "FUN_140616BC0", "complete": True, "pointer_free": True}
            branches = root.get("branches")
            if (root.get("schema_version") != 1 or root.get("frame_hour") != hour
                    or not _country(owner) or not _uint(owner_id, positive=True)
                    or (d18 and d18.get(owner) != owner_id) or not _sha(root.get("producer_record_identity"))
                    or authority != expected_authority or not isinstance(branches, list) or len(branches) > 64):
                problems.append(f"{path} exact authority/frame/owner invalid")
            else:
                definitions: Set[str] = set()
                display = 0
                for ordinal, branch in enumerate(branches):
                    bpath = f"{path}.branches[{ordinal}]"
                    branch_obj = _exact_keys(branch, {"branch_database_index", "branch_display_ordinal", "branch_id", "upgrades"}, bpath, problems)
                    if branch_obj is None:
                        continue
                    branch_id, upgrades = branch_obj.get("branch_id"), branch_obj.get("upgrades")
                    valid = (branch_obj.get("branch_database_index") == ordinal + 1
                             and branch_obj.get("branch_display_ordinal") == ordinal
                             and _portable(branch_id, 256) and branch_id not in definitions
                             and isinstance(upgrades, list) and len(upgrades) <= 256)
                    if not valid:
                        problems.append(f"{bpath} branch order/identity invalid")
                        continue
                    definitions.add(branch_id)
                    previous = -1
                    for upgrade in upgrades:
                        if (not isinstance(upgrade, dict) or set(upgrade) != {"controller_display_ordinal", "branch_source_ordinal", "upgrade_id"}
                                or upgrade.get("controller_display_ordinal") != display
                                or not _uint(upgrade.get("branch_source_ordinal"), 255)
                                or upgrade.get("branch_source_ordinal") <= previous
                                or not _portable(upgrade.get("upgrade_id"), 256)
                                or upgrade.get("upgrade_id") in definitions):
                            problems.append(f"{bpath} upgrade order/identity invalid")
                        else:
                            definitions.add(upgrade["upgrade_id"])
                        display += 1
                        previous = upgrade.get("branch_source_ordinal", previous)
    value = extra.get("cipher_controller_refresh")
    if value is not None:
        path = "diplomacy_world.domain53_extra.cipher_controller_refresh"
        root = _exact_keys(value, {"schema_version", "frame_hour", "owner", "owner_tag_db_id", "selected_target", "selected_target_tag_db_id", "producer_record_identity", "authority", "targets"}, path, problems)
        if root is None:
            return
        owner, owner_id = root.get("owner"), root.get("owner_tag_db_id")
        selected, selected_id = root.get("selected_target"), root.get("selected_target_tag_db_id")
        authority = root.get("authority")
        expected_authority = {"executable_sha256": EXECUTABLE_SHA256, "controller_producer": "FUN_141A0F0F0", "small_sort_producer": "FUN_141A0A8C0", "large_sort_producer": "FUN_141A0AA60", "row_bind_producer": "FUN_141A0EC20", "complete": True, "pointer_free": True}
        targets = root.get("targets")
        valid = (root.get("schema_version") == 1 and root.get("frame_hour") == hour
                 and _country(owner) and _uint(owner_id, positive=True)
                 and (not d18 or d18.get(owner) == owner_id) and _sha(root.get("producer_record_identity"))
                 and authority == expected_authority and isinstance(targets, list) and len(targets) <= 512
                 and ((selected is None and selected_id is None) or (_country(selected) and _uint(selected_id, positive=True))))
        names: Set[str] = set()
        ids: Set[int] = set()
        if valid:
            for ordinal, target in enumerate(targets):
                if (not isinstance(target, dict) or set(target) != {"display_ordinal", "target", "target_tag_db_id"}
                        or target.get("display_ordinal") != ordinal or not _country(target.get("target"))
                        or not _uint(target.get("target_tag_db_id"), positive=True)
                        or target.get("target") == owner or target.get("target_tag_db_id") == owner_id
                        or target.get("target") in names or target.get("target_tag_db_id") in ids
                        or (d18 and d18.get(target.get("target")) != target.get("target_tag_db_id"))):
                    valid = False
                    break
                names.add(target["target"]); ids.add(target["target_tag_db_id"])
            if selected is not None:
                valid &= bool(targets) and targets[0].get("target") == selected and targets[0].get("target_tag_db_id") == selected_id
        if not valid:
            problems.append(f"{path} exact authority/order/selection invalid")


def _validate_a31(snap: Dict[str, Any], problems: List[str]) -> None:
    key = "_r2_national_focus_presentation_live"
    if key not in snap or snap.get(key) is None:
        return
    root = _exact_keys(snap[key], {"schema_version", "authority", "executable_sha256", "function_manifest_sha256", "job_receipt_sha256", "elapsed_wall_ms", "frame_hour", "source_sequence", "producer_complete", "country_tag", "view_id", "producer_record_identity", "expected_visible_item_count", "items"}, key, problems)
    if root is None:
        return
    tag, sequence = root.get("country_tag"), root.get("source_sequence")
    items = root.get("items")
    expected_view = f"nationalfocusview:{tag}"
    expected_record = f"a729:a31:{root.get('frame_hour')}:{tag}:{sequence}"
    if (root.get("schema_version") != 2
            or root.get("authority") != "exact_a729_national_focus_view_items"
            or root.get("executable_sha256") != EXECUTABLE_SHA256
            or root.get("function_manifest_sha256") != FOCUS_MANIFEST_SHA256
            or root.get("job_receipt_sha256") != FOCUS_RECEIPT_SHA256
            or root.get("frame_hour") != snap.get("hour_index")
            or not _uint(root.get("elapsed_wall_ms"), (1 << 64) - 1)
            or not _uint(sequence, (1 << 64) - 1, positive=True)
            or root.get("producer_complete") is not True or not _country(tag)
            or root.get("view_id") != expected_view or root.get("producer_record_identity") != expected_record
            or not isinstance(items, list) or len(items) > 131_072
            or root.get("expected_visible_item_count") != len(items)):
        problems.append(f"{key} exact authority/frame/completeness invalid")
        return
    seen: Set[str] = set()
    for ordinal, raw in enumerate(items):
        path = f"{key}.items[{ordinal}]"
        row = _exact_keys(raw, {"focus_id", "source_ordinal", "presentation_state_code", "display_progress_percent", "continuous_glow_visible", "highlight_glow_visible", "historical_marker_visible", "joint_viewing_country_tag"}, path, problems)
        if row is None:
            continue
        focus, state, progress = row.get("focus_id"), row.get("presentation_state_code"), row.get("display_progress_percent")
        progress_valid = (_uint(state, 3) and _uint(progress, 100)
                          and ((state in {0, 2} and progress == 0)
                               or (state == 1 and progress == 100)
                               or state == 3))
        joint = row.get("joint_viewing_country_tag")
        if (row.get("source_ordinal") != ordinal or not _portable(focus, 128) or focus in seen
                or not progress_valid or type(row.get("continuous_glow_visible")) is not bool
                or row.get("continuous_glow_visible") != (state == 3)
                or type(row.get("highlight_glow_visible")) is not bool
                or type(row.get("historical_marker_visible")) is not bool
                or not (joint is None or _country(joint))):
            problems.append(f"{path} ordered presentation content invalid")
        else:
            seen.add(focus)


def _validate_a39(snap: Dict[str, Any], problems: List[str]) -> None:
    key = "_r2_character_presentation_live"
    if key not in snap or snap.get(key) is None:
        return
    expected_root = {"schema_version", "authority", "executable_sha256", *CHARACTER_HASHES.keys(), "elapsed_wall_ms", "frame_hour", "completeness_record_identity", "source_sequence", "trait_trees", "ace_portraits"}
    root = _exact_keys(snap[key], expected_root, key, problems)
    if root is None:
        return
    trees, aces = root.get("trait_trees"), root.get("ace_portraits")
    if (root.get("schema_version") != 2 or root.get("authority") != "exact_a729_character_presentation"
            or root.get("executable_sha256") != EXECUTABLE_SHA256
            or any(root.get(name) != expected for name, expected in CHARACTER_HASHES.items())
            or root.get("frame_hour") != snap.get("hour_index")
            or not _uint(root.get("elapsed_wall_ms"), (1 << 64) - 1)
            or not isinstance(root.get("completeness_record_identity"), str)
            or not root.get("completeness_record_identity").strip()
            or len(root.get("completeness_record_identity")) > 65_536
            or any(ord(ch) < 32 or ord(ch) == 127 for ch in root.get("completeness_record_identity"))
            or not _uint(root.get("source_sequence"), (1 << 64) - 1, positive=True)
            or not isinstance(trees, list) or len(trees) > 8192
            or not isinstance(aces, list) or len(aces) > 65_536):
        problems.append(f"{key} exact authority/frame/header invalid")
        return
    d18 = _d18_map(snap)
    d18_ids = set(d18.values())
    tag_by_id = {ident: tag for tag, ident in d18.items()}
    runtime_contexts: Dict[str, Dict[str, Any]] = {}
    runtime_text = snap.get("runtime_text")
    if isinstance(runtime_text, dict) and isinstance(runtime_text.get("contexts"), list):
        runtime_contexts = {row.get("context_id"): row for row in runtime_text["contexts"]
                            if isinstance(row, dict) and isinstance(row.get("context_id"), str)}
    tree_keys: Set[Tuple[int, int, int]] = set()
    reason_contexts: Set[str] = set()
    total_nodes = 0
    for tree_index, raw in enumerate(trees):
        path = f"{key}.trait_trees[{tree_index}]"
        tree = _exact_keys(raw, {"owner_tag_db_id", "unit_leader_id", "tree_root_offset", "nodes"}, path, problems)
        if tree is None:
            continue
        owner, leader, offset, nodes = tree.get("owner_tag_db_id"), tree.get("unit_leader_id"), tree.get("tree_root_offset"), tree.get("nodes")
        identity = (owner, leader, offset)
        if (not _uint(owner, positive=True) or (d18_ids and owner not in d18_ids)
                or not _uint(leader, (1 << 64) - 1, positive=True) or offset not in {0x120, 0x130}
                or identity in tree_keys or not isinstance(nodes, list)):
            problems.append(f"{path} exact tree identity invalid")
            continue
        tree_keys.add(identity); total_nodes += len(nodes)
        tokens: Set[str] = set()
        for ordinal, node in enumerate(nodes):
            npath = f"{path}.nodes[{ordinal}]"
            allowed = {"source_ordinal", "trait_token", "state_raw", "state", "progress_raw"}
            if isinstance(node, dict) and "reason_context_id" in node:
                allowed.add("reason_context_id")
            exact = _exact_keys(node, allowed, npath, problems)
            if exact is None:
                continue
            token, state_raw, state = exact.get("trait_token"), exact.get("state_raw"), exact.get("state")
            reason = exact.get("reason_context_id")
            state_valid = (state_raw, state) in {(0, "blocked"), (1, "pickable"), (2, "picked")}
            reason_row = runtime_contexts.get(reason) if isinstance(reason, str) else None
            reason_valid = (reason is None or (
                isinstance(reason, str) and 0 < len(reason) <= 1024 and not reason.isspace()
                and reason not in reason_contexts and reason_row is not None
                and reason_row.get("frame_hour") == snap.get("hour_index")
                and reason_row.get("pov_country_or_global") in {"global", tag_by_id.get(owner)}
            ))
            if (exact.get("source_ordinal") != ordinal or not _portable(token, 1024) or token in tokens
                    or not state_valid or not (exact.get("progress_raw") is None or _i64(exact.get("progress_raw")))
                    or not reason_valid):
                problems.append(f"{npath} order/state/runtime-text join invalid")
            else:
                tokens.add(token)
                if reason is not None:
                    reason_contexts.add(reason)
    if total_nodes > 131_072:
        problems.append(f"{key} trait node count exceeds exact bound")
    ace_keys: Set[Tuple[int, int]] = set()
    owner_ordinals: Dict[int, List[int]] = {}
    total_text = 0
    for index, raw in enumerate(aces):
        path = f"{key}.ace_portraits[{index}]"
        ace = _exact_keys(raw, {"owner_tag_db_id", "ace_id", "visible_ordinal", "portrait_gfx", "modifier_text"}, path, problems)
        if ace is None:
            continue
        owner, ace_id = ace.get("owner_tag_db_id"), ace.get("ace_id")
        identity = (owner, ace_id)
        text = ace.get("modifier_text")
        if (not _uint(owner, positive=True) or (d18_ids and owner not in d18_ids)
                or not _uint(ace_id, (1 << 64) - 1, positive=True) or identity in ace_keys
                or not _uint(ace.get("visible_ordinal")) or not _portable(ace.get("portrait_gfx"), 1024)
                or not isinstance(text, str) or len(text.encode("utf-8")) > (1 << 20)):
            problems.append(f"{path} exact portrait identity/content invalid")
            continue
        ace_keys.add(identity); owner_ordinals.setdefault(owner, []).append(ace["visible_ordinal"])
        total_text += len(text.encode("utf-8"))
    if total_text > (64 << 20):
        problems.append(f"{key} modifier text total exceeds exact bound")
    for owner, ordinals in owner_ordinals.items():
        if sorted(ordinals) != list(range(len(ordinals))):
            problems.append(f"{key} owner {owner} visible ordinals are not contiguous")


def _validate_d56(snap: Dict[str, Any], problems: List[str]) -> None:
    oob = snap.get("order_of_battle")
    extra = oob.get("extra") if isinstance(oob, dict) else None
    if not isinstance(extra, dict) or "naval_missions_status_live" not in extra or extra.get("naval_missions_status_live") is None:
        return
    path = "order_of_battle.extra.naval_missions_status_live"
    root = _exact_keys(extra[path.rsplit(".", 1)[-1]], {"source", "confidence", "projection", "task_force_existence_owner_domain", "ship_existence_owner_domain", "task_force_count", "validated_location_rows", "derived_region_rows", "ship_summary_rows", "dominant_hull_rows", "fixture_required_rows", "convoy_sunk_count_rows", "oob_command_join_checked", "oob_command_orphan_rows", "task_forces"}, path, problems)
    if root is None:
        return
    rows = root.get("task_forces")
    if (root.get("source") != "C" or root.get("confidence") != "validated_slice"
            or root.get("projection") != "R2 order_of_battle.extra annotation"
            or root.get("task_force_existence_owner_domain") != "oob_command_live"
            or root.get("ship_existence_owner_domain") != "ships"
            or root.get("oob_command_join_checked") is not True
            or root.get("oob_command_orphan_rows") != 0 or not isinstance(rows, list)
            or root.get("task_force_count") != len(rows)):
        problems.append(f"{path} exact authority/count envelope invalid")
        return
    d18 = _d18_map(snap)
    mission_names = ["HOLD", "PATROL", "STRIKE_FORCE", "CONVOY_RAIDING", "CONVOY_ESCORT", "MINES_PLANTING", "MINES_SWEEPING", "TRAINING", "RESERVE_FLEET", "NAVAL_INVASION_SUPPORT"]
    detached_names = ["none", "repairing", "moving_to_refit", "refitting", "reinforcing"]
    seen: Set[int] = set()
    count_fields = {name: 0 for name in ("validated_location_rows", "derived_region_rows", "ship_summary_rows", "dominant_hull_rows", "fixture_required_rows", "convoy_sunk_count_rows")}
    expected = {"task_force_id", "fleet_id", "owner", "owner_tag", "owner_tag_db_id", "location_province_id", "location_province", "strategic_region_id", "ship_count", "dominant_hull_class", "mission_enum", "mission_name", "mission_state", "detached_activity", "activity_key", "active_combat_epochs", "combat_membership_authority", "home_base_province_id", "repair_base_province_id", "repair_progress_milli", "retreat_state", "sunk_convoys", "convoy_route_context_id", "fixture_required", "active_battle_fixture_required", "source_status_flags", "oob_command_task_force_seen"}
    for index, raw in enumerate(rows):
        rpath = f"{path}.task_forces[{index}]"
        row = _exact_keys(raw, expected, rpath, problems)
        if row is None:
            continue
        task_force, owner, owner_id = row.get("task_force_id"), row.get("owner_tag"), row.get("owner_tag_db_id")
        mission, state = row.get("mission_enum"), row.get("mission_state")
        epochs, authority = row.get("active_combat_epochs"), row.get("combat_membership_authority")
        epochs_valid = ((epochs is None and authority is None)
                        or (isinstance(epochs, list) and authority == "d49_d50_member_epoch"
                            and len(epochs) == len(set(epochs))
                            and all(isinstance(epoch, str) and _COMBAT_EPOCH.fullmatch(epoch) for epoch in epochs)))
        expected_mission = mission_names[mission] if _uint(mission, 9) else None
        expected_detached = detached_names[state] if _uint(state, 4) else None
        activity = row.get("activity_key")
        activity_valid = activity is None or (activity == "TASK_FORCE_ACTIVITY_DESC_IN_COMBAT"
                                               and authority == "d49_d50_member_epoch" and bool(epochs))
        nullable_u32 = ("location_province_id", "location_province", "strategic_region_id", "ship_count", "home_base_province_id", "repair_base_province_id", "repair_progress_milli", "retreat_state", "sunk_convoys", "convoy_route_context_id")
        dominant_hull = row.get("dominant_hull_class")
        repair_progress = row.get("repair_progress_milli")
        if (not _uint(task_force, positive=True) or task_force in seen or not _uint(row.get("fleet_id"), positive=True)
                or not _country(owner) or row.get("owner") != owner or not _uint(owner_id, positive=True)
                or (d18 and d18.get(owner) != owner_id)
                or any(not (row.get(name) is None or _uint(row.get(name))) for name in nullable_u32)
                or not (dominant_hull is None or _portable(dominant_hull, 128))
                or not (repair_progress is None or _uint(repair_progress, 1_000))
                or row.get("location_province") != row.get("location_province_id")
                or row.get("mission_name") != expected_mission or row.get("detached_activity") != expected_detached
                or not epochs_valid or not activity_valid
                or type(row.get("fixture_required")) is not bool
                or type(row.get("active_battle_fixture_required")) is not bool
                or not _uint(row.get("source_status_flags"))
                or row.get("oob_command_task_force_seen") is not True):
            problems.append(f"{rpath} exact task-force authority/content invalid")
        else:
            seen.add(task_force)
        count_fields["validated_location_rows"] += row.get("location_province_id") is not None
        count_fields["derived_region_rows"] += row.get("strategic_region_id") is not None
        count_fields["ship_summary_rows"] += row.get("ship_count") is not None
        count_fields["dominant_hull_rows"] += row.get("dominant_hull_class") is not None
        count_fields["fixture_required_rows"] += bool(row.get("fixture_required") or row.get("active_battle_fixture_required"))
        count_fields["convoy_sunk_count_rows"] += row.get("sunk_convoys") is not None
    for name, count in count_fields.items():
        if root.get(name) != count:
            problems.append(f"{path}.{name} count mismatch")


def _validate_x2(snap: Dict[str, Any], problems: List[str]) -> None:
    for family in ("combats", "ended_combats"):
        combats = snap.get(family)
        if not isinstance(combats, list):
            continue
        for combat_index, combat in enumerate(combats):
            detail = combat.get("land_combat_detail_events_live") if isinstance(combat, dict) else None
            if not isinstance(detail, dict):
                continue
            for side_name in ("attacker", "defender"):
                side = detail.get(side_name)
                if not isinstance(side, dict):
                    continue
                path = f"{family}[{combat_index}].land_combat_detail_events_live.{side_name}"
                losses_status = side.get("losses_status")
                losses = side.get("current_loss_observations")
                if losses_status == "exact_current_epoch_writer":
                    _validate_x2_losses(losses, f"{path}.current_loss_observations", problems)
                elif losses_status == "unavailable_no_authenticated_live_carrier":
                    if losses is not None:
                        problems.append(f"{path}.current_loss_observations conflicts with unavailable status")
                elif losses_status is not None:
                    problems.append(f"{path}.losses_status unknown")
                width_status = side.get("width_status")
                widths = side.get("width_observations")
                if width_status == "exact_synchronous_call":
                    _validate_x2_widths(widths, side, f"{path}.width_observations", problems)
                elif width_status == "unavailable_pending_x2_control":
                    if widths is not None:
                        problems.append(f"{path}.width_observations conflicts with unavailable status")
                elif width_status is not None:
                    problems.append(f"{path}.width_status unknown")


def _validate_x2_losses(value: Any, path: str, problems: List[str]) -> None:
    if not isinstance(value, list) or not value:
        problems.append(f"{path} must be a nonempty exact list")
        return
    sequences: Set[int] = set()
    for index, raw in enumerate(value):
        rpath = f"{path}[{index}]"
        row = _exact_keys(raw, {"capture_sequence", "manpower_before_raw", "manpower_after_raw", "equipment", "source"}, rpath, problems)
        if row is None:
            continue
        sequence, equipment = row.get("capture_sequence"), row.get("equipment")
        valid = (_uint(sequence, (1 << 64) - 1, positive=True) and sequence not in sequences
                 and _i64(row.get("manpower_before_raw")) and _i64(row.get("manpower_after_raw"))
                 and row.get("manpower_before_raw") != row.get("manpower_after_raw")
                 and row.get("source") in {"authenticated_a729_active_division_damage_140c7bfc0", "authenticated_a729_participant_lifecycle_141299850"}
                 and isinstance(equipment, list) and bool(equipment))
        ids: Set[int] = set()
        if isinstance(equipment, list):
            for ordinal, delta in enumerate(equipment):
                if (not isinstance(delta, dict) or set(delta) != {"ordinal", "equipment_id", "before_raw", "after_raw"}
                        or delta.get("ordinal") != ordinal or not _uint(delta.get("equipment_id"), positive=True)
                        or delta.get("equipment_id") in ids or not _i64(delta.get("before_raw"))
                        or not _i64(delta.get("after_raw")) or delta.get("before_raw") == delta.get("after_raw")):
                    valid = False
                else:
                    ids.add(delta["equipment_id"])
        if not valid:
            problems.append(f"{rpath} authenticated loss delta invalid")
        else:
            sequences.add(sequence)


def _validate_x2_widths(value: Any, side: Dict[str, Any], path: str, problems: List[str]) -> None:
    if not isinstance(value, list) or not value:
        problems.append(f"{path} must be a nonempty exact list")
        return
    sequences: Set[int] = set()
    for index, raw in enumerate(value):
        rpath = f"{path}[{index}]"
        row = _exact_keys(raw, {"capture_sequence", "final_width_raw", "wrappers", "source"}, rpath, problems)
        if row is None:
            continue
        sequence, wrappers = row.get("capture_sequence"), row.get("wrappers")
        valid = (_uint(sequence, (1 << 64) - 1, positive=True) and sequence not in sequences
                 and _i64(row.get("final_width_raw")) and row.get("source") == "authenticated_a729_141299610_call"
                 and isinstance(wrappers, list) and bool(wrappers))
        ids: Set[int] = set(); total = 0
        if isinstance(wrappers, list):
            for ordinal, wrapper in enumerate(wrappers):
                if (not isinstance(wrapper, dict) or set(wrapper) != {"ordinal", "division_id", "width_raw"}
                        or wrapper.get("ordinal") != ordinal or not _uint(wrapper.get("division_id"), positive=True)
                        or wrapper.get("division_id") in ids or not _i64(wrapper.get("width_raw"))):
                    valid = False
                else:
                    ids.add(wrapper["division_id"]); total += wrapper["width_raw"]
        if total != row.get("final_width_raw"):
            valid = False
        if (type(side.get("counts_incomplete")) is not bool
                or not _uint(side.get("committed"))
                or (side.get("counts_incomplete") is False
                    and side.get("committed") != len(wrappers or []))):
            valid = False
        if not valid:
            problems.append(f"{rpath} authenticated width decomposition invalid")
        else:
            sequences.add(sequence)


def _validate_weather(snap: Dict[str, Any], problems: List[str]) -> None:
    map_obj = snap.get("map")
    if not isinstance(map_obj, dict) or "_r2_weather_live" not in map_obj or map_obj.get("_r2_weather_live") is None:
        return
    path = "map._r2_weather_live"
    root = _exact_keys(map_obj["_r2_weather_live"], {"source", "active_region_count", "observed_region_rows", "observed_province_rows", "scale_raw", "regions", "provinces"}, path, problems)
    if root is None:
        return
    regions, provinces = root.get("regions"), root.get("provinces")
    observed_regions, observed_provinces = root.get("observed_region_rows"), root.get("observed_province_rows")
    if (root.get("source") != "weather_live" or root.get("scale_raw") != 100_000
            or observed_regions != 305 or not _uint(root.get("active_region_count"), 305)
            or not _uint(observed_provinces, 65_536)
            or (observed_provinces != 0 and observed_provinces < 10_000)
            or not isinstance(regions, dict) or len(regions) != observed_regions
            or not isinstance(provinces, dict) or len(provinces) != observed_provinces):
        problems.append(f"{path} exact source/scale/completeness counts invalid")
        return
    active = 0
    region_keys = {"region_id", "temperature", "temperature_raw", "phenomenon", "next_weather_change_raw", "active_modifier_count", "intensity", "rain_light", "rain_heavy", "snow", "blizzard", "arctic_water", "active_signal", "phenomenon_active", "sandstorm"}
    for key, raw in regions.items():
        rpath = f"{path}.regions.{key}"
        row = _exact_keys(raw, region_keys, rpath, problems)
        try:
            identity = int(key)
        except (TypeError, ValueError):
            identity = -1
        if row is None:
            continue
        bools = ("rain_light", "rain_heavy", "snow", "blizzard", "arctic_water", "active_signal", "phenomenon_active", "sandstorm")
        temperature_raw = row.get("temperature_raw")
        if (not _uint(identity, 304) or str(identity) != key or row.get("region_id") != identity
                or not _is_int(temperature_raw) or not -10_000_000 <= temperature_raw <= 10_000_000
                or row.get("temperature") != row.get("temperature_raw") / 100_000
                or not _uint(row.get("phenomenon"), 16) or not _uint(row.get("next_weather_change_raw"))
                or not _uint(row.get("active_modifier_count"), 4_096) or not _uint(row.get("intensity"))
                or any(type(row.get(name)) is not bool for name in bools)):
            problems.append(f"{rpath} exact region row invalid")
        active += bool(row.get("active_signal"))
    if root.get("active_region_count") != active:
        problems.append(f"{path}.active_region_count mismatch")
    province_keys = {"province_id", "temperature", "temperature_raw", "temperature_change", "temperature_change_raw", "water", "water_raw", "snow", "snow_raw", "mud_present", "mud_active", "custom_modifier_count", "struct_validated", "map_recolor", "dynamic_lut"}
    for key, raw in provinces.items():
        ppath = f"{path}.provinces.{key}"
        row = _exact_keys(raw, province_keys, ppath, problems)
        try:
            identity = int(key)
        except (TypeError, ValueError):
            identity = -1
        if row is None:
            continue
        valid = (_uint(identity, positive=True) and str(identity) == key and row.get("province_id") == identity
                 and type(row.get("mud_present")) is bool and type(row.get("struct_validated")) is bool
                 and type(row.get("map_recolor")) is bool and type(row.get("dynamic_lut")) is bool)
        raw_bounds = {
            "temperature_raw": (-10_000_000, 10_000_000),
            "temperature_change_raw": (-10_000_000, 10_000_000),
            "water_raw": (0, 100_000_000),
            "snow_raw": (0, 100_000_000),
        }
        for value_name, raw_name in (("temperature", "temperature_raw"), ("temperature_change", "temperature_change_raw"), ("water", "water_raw"), ("snow", "snow_raw")):
            value, raw_value = row.get(value_name), row.get(raw_name)
            valid &= ((value is None and raw_value is None)
                      or (_is_int(raw_value) and isinstance(value, (int, float)) and type(value) is not bool
                          and raw_bounds[raw_name][0] <= raw_value <= raw_bounds[raw_name][1]
                          and value == raw_value / 100_000))
        valid &= (row.get("mud_active") is None if not row.get("mud_present") else type(row.get("mud_active")) is bool)
        valid &= row.get("custom_modifier_count") is None or _uint(row.get("custom_modifier_count"), 4_096)
        present = {name: row.get(name) is not None for name in ("temperature_raw", "water_raw", "snow_raw")}
        valid &= row.get("map_recolor") == any(present.values())
        valid &= row.get("dynamic_lut") == (present["snow_raw"] or row.get("mud_present"))
        if not valid:
            problems.append(f"{ppath} exact province row/null semantics invalid")
