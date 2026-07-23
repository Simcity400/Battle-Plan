"""Strict, additive schema-v1 admission for exact Wave-4 replay carriers.

This module is deliberately pure-stdlib and side-effect free.  The carriers are
optional so recordings created before their introduction remain readable;
present non-null carriers, however, must satisfy their complete exact contract.
"""
from __future__ import annotations

import hashlib
import math
import re
from typing import Any, Dict, List, Optional, Set, Tuple


EXECUTABLE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
A25_TEXT_SETTER_RVA = 0x022B59E0
A25_FORMATTER_RVA = 0x02232AB0
A20_STATIC_RECEIPT_SHA256 = "f10a9cad9e53c660b50364f5a242891013be38f77cb873b870aede44da1d8f3a"
X9_PRODUCER_MANIFEST_SHA256 = "86f3aa3a916a203ea657cda01a84fb297e1812e03df59b481b004b16e0ad7ff6"
NUKE_PRODUCER_CONSUMER_MANIFEST_SHA256 = "0968ed85cd6103149ebd51415d6b64d1959bb05d1aff4cbfce9d31e437a6995c"
NUKE_CONTAINER_CONSTRUCTOR_MANIFEST_SHA256 = "233962bab3a8ca79150977f1f7e77312d1bbd262e89e582d1c01caaf01a89032"
NUKE_VTABLE_ARTIFACT_SHA256 = "cfd9d9e4088b4998398e579a5ebee222534868bad8a4529240b370faf467adbb"
B4_OWNER_SERIALIZER_MANIFEST_SHA256 = "2fd64c0a861d38e252fe262d52e62cbb4a5efa7f94ee7ab374c33e8ac368827e"
B4_OWNER_SERIALIZER_ARTIFACT_SHA256 = "bc679b4a175e75253a1fea1bfde625ccf8594ec06c562699aeaac75b9eaddf3a"
B4_ROW_MUTATION_MANIFEST_SHA256 = "b36682b2f942e7e4e7c8ae9dbfc05e2b52ee46dcf388fc9ad6e1fee24fdb802e"
B4_ROW_SERIALIZER_ARTIFACT_SHA256 = "005235886a2d847b111df481ae948395828ec999e3cebe39612d36d364fefe98"
B4_MUTATION_ARTIFACT_SHA256 = "2fa2e8a326081f82735b3824dc8b693130605a956964a170f1b5538e4ac1ef14"
ZERO_SHA256 = "0" * 64
DYNAMIC_MODIFIER_RECEIPT_SHA256 = "bdbac3cb1158c971b408586611d6aba618ecfe85b5265e3b9e8c2c0bf545e415"
FOCUS_MANIFEST_SHA256 = "b319dd1b721ed6a13e47088b7def034111f9882631080bffbf049fbb79265a2f"
FOCUS_RECEIPT_SHA256 = "b1c9d8c0678956f17bea2e7f9e06e7ff4baf31254c11b3c983a9a5ca9ae2d04b"
SUNK_SHIP_HASHES = {
    "function_manifest_sha256": "466880cc290e8961dab48c8323346d9d862089d85bbe0f4ba285b924cc959c82",
    "data_artifact_sha256": "5e0ed3e76724032f3e2f4cc5bcc9e5c38347ed862e0489821be8b7bbdd45ca21",
    "writer_sentinel_artifact_sha256": "dccd61e7573317458980fc0796fcba2ba945a434fbacbf3303a588c620307a4a",
    "all_writer_routes_manifest_sha256": "d02f0f1aaf2369db9dc9529062f5e8f59f3f6f751fb9e4c0ab81d47dea1e1de0",
    "csunk_ship_vtable_artifact_sha256": "61140fad7a24c150b60935580374129321719555bb5a4e1db88bd2bcd296937e",
    "csunk_ship_virtuals_manifest_sha256": "755a0011376d06bbd38456a2bb009236d99f4481a9b076789d9d57729c5673fd",
    "attacker_getters_manifest_sha256": "946eff0fb077863ce2dc1f1607ac2c0b9fcfee8f3377662ff14315101c30cbd6",
}
DECISION_PANEL_MANIFEST_SHA256 = "712233abfbd1352f0c84ede6267a99f4de8f5406cabfd20d76046a094b5fec0a"
DECISION_PANEL_DATA_SHA256 = "ccca7fbfe0378821f22ed82d1a371a58ea12c24a9601fd3dd36d97bef093f0ea"
DECISION_PANEL_COUNTRY_OWNER_SHA256 = "a31b9fa4d886d509ceceef46819b6048331ef65065c2f07a8cc086e0868dd07a"
POWER_BALANCE_MANIFEST_SHA256 = "a540af1c030d9ef2fbb1cd290627087627771a196e7a60e4241cb5287b66e4e2"
RESOLVED_TOOLTIP_MANIFEST_SHA256 = "a23b63ccefc021eeced9a1a918588d3e2fbba07e5d1afb4d53f83cc7c1640177"
A38_STATIC_AUTHORITY_JOB_ID = "20260720T020812Z-w3-a38-controller-owner-41ee5208"
A38_STATIC_AUTHORITY_RECEIPT_SHA256 = "7d0a36b1822faf85a949d628a79bc4a09f95dc9c9aa3ed4aecb715b72b0a9082"
CHARACTER_HASHES = {
    "trait_collection_manifest_sha256": "837c8b4db6a6cbc03fa5c950dc24c814f5156d6cf0de08f5041bfcccab7d78c9",
    "trait_state_manifest_sha256": "63ca92c9e5238a0a342e24209417188b7ceeabd7f01a21c45475f58bb3e29e8c",
    "trait_progress_manifest_sha256": "134d8fc1d5056e05023ebd46e33bda913ba3dde6c1f0945d423297d464dae547",
    "ace_refresh_manifest_sha256": "4be836cc67b9f59d748a05a73fa4c5d9c46db357d283d25b756c7ad18fcb2674",
    "ace_bind_sort_manifest_sha256": "ea545ce9457982590bd42741d1f573a7874594b7c3f836bf290ae8a276f83396",
    "ace_modifier_setter_artifact_sha256": "3b49c9eb735a03fc711f7322b74a5b86ad64b25ffdbcaa73a390b64a2edfd552",
    "ace_modifier_formatter_manifest_sha256": "79fd3a1e1b9eafb09566b2721260936c8430acffcf0ba408cc866c6292ac8f0b",
    "ace_selected_context_manifest_sha256": "b593bfc287648ac2bdced54567db5241f2acfe4032f8aed2a77632e29b468c5c",
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


def _i32(value: Any) -> bool:
    return _is_int(value) and -(1 << 31) <= value < (1 << 31)


def _country(value: Any) -> bool:
    return isinstance(value, str) and _COUNTRY.fullmatch(value) is not None


def _sha(value: Any) -> bool:
    return (isinstance(value, str) and _SHA256.fullmatch(value) is not None
            and any(ch != "0" for ch in value))


def _portable(value: Any, maximum: int = 1024) -> bool:
    return (isinstance(value, str) and 0 < len(value) <= maximum
            and _PORTABLE_ID.fullmatch(value) is not None and ".." not in value)


def _focus_id(value: Any) -> bool:
    """Mirror the NFP1/Rust portable focus-definition identity exactly."""
    if (not isinstance(value, str) or not 0 < len(value) <= 128
            or not (value[0].isascii() and (value[0].isalnum() or value[0] == "_"))
            or ".." in value
            or any(not (ch.isascii() and (ch.isalnum() or ch in "_-.@")) for ch in value)):
        return False
    pointer_digits = value[2:] if value.startswith(("0x", "0X")) else None
    return not (pointer_digits is not None and len(pointer_digits) >= 8
                and all(ch in "0123456789abcdefABCDEF" for ch in pointer_digits))


def _exact_text(value: Any, maximum_bytes: int = 65_536) -> bool:
    if not isinstance(value, str) or not value.strip():
        return False
    try:
        if len(value.encode("utf-8")) > maximum_bytes:
            return False
    except UnicodeEncodeError:
        return False
    return not any(ord(ch) <= 0x1F or 0x7F <= ord(ch) <= 0x9F for ch in value)


def _bounded_nonempty(value: Any, maximum_bytes: int = 65_536) -> bool:
    if not isinstance(value, str) or not value.strip():
        return False
    try:
        return len(value.encode("utf-8")) <= maximum_bytes
    except UnicodeEncodeError:
        return False


def _canonical_u64_string(value: Any) -> Optional[int]:
    if not isinstance(value, str) or not value or not value.isascii() or not value.isdigit():
        return None
    if len(value) > 1 and value.startswith("0"):
        return None
    parsed = int(value)
    return parsed if parsed <= (1 << 64) - 1 else None


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
    _validate_b4(snap, problems)
    _validate_a17(snap, problems)
    _validate_a18(snap, problems)
    _validate_a20_x9(snap, problems)
    _validate_a23(snap, problems)
    _validate_a25(snap, problems)
    _validate_delayed_event_lifecycle(snap, problems)
    _validate_a30(snap, problems)
    _validate_a33(snap, problems)
    _validate_a34(snap, problems)
    _validate_occupation_garrison_alert(snap, problems)
    _validate_resolved_tooltip_instances(snap, problems)
    _validate_power_balance_presentation(snap, problems)
    _validate_a38(snap, problems)
    _validate_a31(snap, problems)
    _validate_a39(snap, problems)
    _validate_selected_present_cross_carrier(snap, problems)
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


def _validate_b4_sidecar(
    snap: Dict[str, Any],
    key: str,
    authority: str,
    activation: bool,
    problems: List[str],
) -> Optional[List[Dict[str, Any]]]:
    if key not in snap or snap.get(key) is None:
        return None
    root = _exact_keys(snap[key], {
        "schema_version", "authority", "production_activation_authorized",
        "frame_hour", "source_sequence", "executable_sha256",
        "owner_serializer_manifest_sha256", "owner_serializer_artifact_sha256",
        "row_mutation_manifest_sha256", "row_serializer_artifact_sha256",
        "mutation_artifact_sha256", "live_fixture_receipt_sha256",
        "owner_root", "capacity_count_word", "row_vtable_rva", "row_extent",
        "observed_rows", "iterator_rows", "row_count", "ordering",
        "d18_identity_complete", "rows",
    }, key, problems)
    if root is None:
        return None
    receipt = root.get("live_fixture_receipt_sha256")
    receipt_ok = (isinstance(receipt, str)
                  and _SHA256.fullmatch(receipt) is not None
                  and (not activation or receipt != ZERO_SHA256))
    measured_countries = snap.get("measured_countries")
    d18_frame_known = (isinstance(measured_countries, dict)
                       and measured_countries.get("known") is True)
    if (root.get("schema_version") != 3
            or root.get("authority") != authority
            or root.get("production_activation_authorized") is not activation
            or root.get("frame_hour") != snap.get("hour_index")
            or not _uint(root.get("source_sequence"), (1 << 64) - 1, positive=True)
            or root.get("executable_sha256") != EXECUTABLE_SHA256
            or root.get("owner_serializer_manifest_sha256") != B4_OWNER_SERIALIZER_MANIFEST_SHA256
            or root.get("owner_serializer_artifact_sha256") != B4_OWNER_SERIALIZER_ARTIFACT_SHA256
            or root.get("row_mutation_manifest_sha256") != B4_ROW_MUTATION_MANIFEST_SHA256
            or root.get("row_serializer_artifact_sha256") != B4_ROW_SERIALIZER_ARTIFACT_SHA256
            or root.get("mutation_artifact_sha256") != B4_MUTATION_ARTIFACT_SHA256
            or not receipt_ok
            or root.get("owner_root") != "CGameState+0x5A8"
            or root.get("capacity_count_word") != "CGameState+0x5B0"
            or root.get("row_vtable_rva") != "0x295D008"
            or root.get("row_extent") != 0x18
            or root.get("ordering") != "engine_first_seen_aggregate_identity"
            or root.get("d18_identity_complete") is not True
            or not d18_frame_known):
        problems.append(f"{key} authority/frame/root header invalid")
    rows = root.get("rows")
    if not isinstance(rows, list) or len(rows) > 65_536:
        problems.append(f"{key}.rows should be a bounded list")
        return None
    if any(root.get(field) != len(rows)
           for field in ("observed_rows", "iterator_rows", "row_count")):
        problems.append(f"{key} owner/iterator/row counts disagree")
    d18 = _d18_map(snap)
    d18_id_counts: Dict[int, int] = {}
    for tag_db_id in d18.values():
        d18_id_counts[tag_db_id] = d18_id_counts.get(tag_db_id, 0) + 1
    seen: Set[Tuple[int, int, int]] = set()
    decoded: List[Dict[str, Any]] = []
    for index, raw in enumerate(rows):
        path = f"{key}.rows[{index}]"
        row = _exact_keys(raw, {
            "source_ordinal", "month", "convoys",
            "killer_country_tag_db_id", "owner_tag_db_id",
            "killer_country", "owner",
        }, path, problems)
        if row is None:
            continue
        month = row.get("month")
        convoys = row.get("convoys")
        killer_id = row.get("killer_country_tag_db_id")
        owner_id = row.get("owner_tag_db_id")
        killer = row.get("killer_country")
        owner = row.get("owner")
        identity = (month, killer_id, owner_id)
        if (row.get("source_ordinal") != index
                or not _i32(month)
                or not _uint(convoys, positive=True)
                or not _uint(killer_id, positive=True)
                or not _uint(owner_id, positive=True)
                or not _country(killer)
                or not _country(owner)
                or d18.get(killer) != killer_id
                or d18.get(owner) != owner_id
                or d18_id_counts.get(killer_id) != 1
                or d18_id_counts.get(owner_id) != 1
                or identity in seen):
            problems.append(f"{path} ordinal/value/identity/D18 join invalid")
        else:
            seen.add(identity)
        decoded.append(row)
    return decoded


def _validate_b4(snap: Dict[str, Any], problems: List[str]) -> None:
    carrier_declared = (
        "_r2_sunk_convoy_history_live_candidate" in snap
        or "_r2_sunk_convoy_history_live" in snap
    )
    _validate_b4_sidecar(
        snap,
        "_r2_sunk_convoy_history_live_candidate",
        "static_exact20_evidence_candidate",
        False,
        problems,
    )
    live = _validate_b4_sidecar(
        snap,
        "_r2_sunk_convoy_history_live",
        "exact_a729_sunk_convoy_history",
        True,
        problems,
    )
    world_events = snap.get("world_events")
    canonical = world_events.get("sunk_convoys") if isinstance(world_events, dict) else None
    if live is None:
        if carrier_declared and isinstance(canonical, list):
            problems.append("world_events.sunk_convoys lacks exact current-hour B4 authority")
        return
    if not isinstance(world_events, dict):
        problems.append("world_events missing for exact B4 projection")
        return
    if (world_events.get("sunk_convoys_ordering")
            != "engine_first_seen_aggregate_identity"
            or world_events.get("sunk_convoys_route_attribution") is not None
            or not isinstance(canonical, list)
            or len(canonical) != len(live)):
        problems.append("world_events sunk-convoy authority/order/cardinality mismatch")
        return
    for index, (exact, projected) in enumerate(zip(live, canonical)):
        expected = {
            "month": exact.get("month"),
            "convoys": exact.get("convoys"),
            "killer_country_tag_db_id": exact.get("killer_country_tag_db_id"),
            "owner_tag_db_id": exact.get("owner_tag_db_id"),
            "killer_country": exact.get("killer_country"),
            "owner": exact.get("owner"),
            "first_seen_aggregate_ordinal": index,
            "attribution_scope": "monthly_killer_owner_aggregate",
        }
        if projected != expected:
            problems.append(f"world_events.sunk_convoys[{index}] differs from exact B4 row")


def _validate_a17(snap: Dict[str, Any], problems: List[str]) -> None:
    if "_r2_nuke_stockpile_live" not in snap or snap.get("_r2_nuke_stockpile_live") is None:
        return
    path = "_r2_nuke_stockpile_live"
    root = _exact_keys(snap[path], {
        "schema_version", "authority", "executable_sha256",
        "producer_consumer_manifest_sha256", "container_constructor_manifest_sha256",
        "cnuke_vtable_artifact_sha256", "elapsed_wall_ms", "frame_hour",
        "source_sequence", "countries",
    }, path, problems)
    if root is None:
        return
    if (root.get("schema_version") != 2
            or root.get("authority") != "exact_a729_nuke_stockpile"
            or root.get("executable_sha256") != EXECUTABLE_SHA256
            or root.get("producer_consumer_manifest_sha256") != NUKE_PRODUCER_CONSUMER_MANIFEST_SHA256
            or root.get("container_constructor_manifest_sha256") != NUKE_CONTAINER_CONSTRUCTOR_MANIFEST_SHA256
            or root.get("cnuke_vtable_artifact_sha256") != NUKE_VTABLE_ARTIFACT_SHA256
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
        row = _exact_keys(raw, {"tag_db_id", "container_capacity", "nukes_total", "nukes_ready_total", "nukes"}, rpath, problems)
        if row is None:
            continue
        tag_db_id = row.get("tag_db_id")
        entries = row.get("nukes")
        if not _uint(tag_db_id, positive=True) or tag_db_id in seen:
            problems.append(f"{rpath}.tag_db_id invalid or duplicate")
            continue
        seen.add(tag_db_id)
        if (not _i32(row.get("container_capacity"))
                or row.get("container_capacity") < 2):
            problems.append(f"{rpath}.container_capacity violates exact a729 invariant")
        if not isinstance(entries, list) or len(entries) != 2:
            problems.append(f"{rpath}.nukes should contain the exact two a729 nuke types")
            continue
        built = 0
        ready = 0
        canonical = []
        for entry_index, raw_entry in enumerate(entries):
            epath = f"{rpath}.nukes[{entry_index}]"
            entry = _exact_keys(raw_entry, {"nuke_type_raw", "amount_raw", "amount", "nukes_ready"}, epath, problems)
            if entry is None:
                continue
            amount_raw = entry.get("amount_raw")
            amount = entry.get("amount")
            nukes_ready = entry.get("nukes_ready")
            if entry.get("nuke_type_raw") != entry_index:
                problems.append(f"{epath}.nuke_type_raw violates exact a729 type order")
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
    if (root.get("schema_version") != 3
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
            or not isinstance(contexts, list) or len(contexts) > 262_144):
        problems.append(f"{path} exact measured-complete envelope invalid")
        return
    seen: Set[str] = set()
    semantic_keys: Set[Tuple[str, str, str, Tuple[str, Any]]] = set()
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
        source_key: Optional[Tuple[str, Any]] = None
        if source is not None:
            if source.get("kind") == "authored_token":
                if set(source) != {"kind", "identity"} or not _portable(source.get("identity"), 4096):
                    problems.append(f"{cpath}.source authored identity invalid")
                else:
                    source_key = ("authored_token", source.get("identity"))
            elif source.get("kind") == "setter_caller_rva":
                if set(source) != {"kind", "rva"} or not _uint(source.get("rva"), positive=True):
                    problems.append(f"{cpath}.source setter caller invalid")
                else:
                    source_key = ("setter_caller_rva", source.get("rva"))
            else:
                problems.append(f"{cpath}.source kind invalid")
        if (source_key is not None
                and all(isinstance(row.get(name), str)
                        for name in ("pov_country_or_global", "view_id", "element_id"))):
            semantic_key = (
                row.get("pov_country_or_global"), row.get("view_id"),
                row.get("element_id"), source_key,
            )
            if semantic_key in semantic_keys:
                problems.append(f"{cpath} duplicates an exact runtime-text semantic identity")
            else:
                semantic_keys.add(semantic_key)
        final = _exact_keys(row.get("final_text"), {"bytes_hex", "utf8", "sha256"}, f"{cpath}.final_text", problems)
        if final is not None:
            hex_text = final.get("bytes_hex")
            try:
                if (not isinstance(hex_text, str) or len(hex_text) > 2 * (1 << 20)
                        or len(hex_text) % 2
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
        elif layout.get("mode") == "authored_reproducible":
            if set(layout) != {"mode"}:
                problems.append(f"{cpath}.layout authored mode has extra fields")
            final_utf8 = final.get("utf8") if isinstance(final, dict) else None
            if isinstance(final_utf8, str) and any(
                    ch == "\u00a3" or (ord(ch) <= 0x1F and ch not in {"\n", "\r", "\u0011"})
                    or 0x7F <= ord(ch) <= 0x9F for ch in final_utf8):
                problems.append(f"{cpath}.layout authored mode cannot represent inline/control data")
        elif layout.get("mode") == "captured_runtime":
            if (set(layout) != {"mode", "clip", "glyphs", "inline_icons"}
                    or not isinstance(layout.get("glyphs"), list)
                    or not isinstance(layout.get("inline_icons"), list)
                    or len(layout.get("glyphs", [])) > 262_144
                    or len(layout.get("inline_icons", [])) > 262_144):
                problems.append(f"{cpath}.layout captured mode invalid")
            else:
                mixed_ordinals: List[int] = []
                final_bytes_hex = final.get("bytes_hex") if isinstance(final, dict) else None
                if (isinstance(final_bytes_hex, str) and final_bytes_hex
                        and not layout["glyphs"] and not layout["inline_icons"]):
                    problems.append(f"{cpath}.layout captured mode has no runs for non-empty text")
                clip = layout.get("clip")
                if clip is not None:
                    if (not isinstance(clip, list) or len(clip) != 4
                            or any(not _i32(value) for value in clip)
                            or clip[2] <= 0 or clip[3] <= 0
                            or not _i32(clip[0] + clip[2])
                            or not _i32(clip[1] + clip[3])):
                        problems.append(f"{cpath}.layout.clip invalid")
                for gindex, glyph in enumerate(layout["glyphs"]):
                    gpath = f"{cpath}.layout.glyphs[{gindex}]"
                    exact = _exact_keys(glyph, {"run_ordinal", "font", "glyph_id", "x", "y", "rgba"}, gpath, problems)
                    if exact is None:
                        continue
                    ordinal = exact.get("run_ordinal")
                    rgba = exact.get("rgba")
                    if (not _uint(ordinal) or not _portable(exact.get("font"))
                            or not _uint(exact.get("glyph_id"))
                            or not _i32(exact.get("x")) or not _i32(exact.get("y"))
                            or not isinstance(rgba, list) or len(rgba) != 4
                            or any(not _uint(channel, 0xFF) for channel in rgba)):
                        problems.append(f"{gpath} invalid")
                    else:
                        mixed_ordinals.append(ordinal)
                for iindex, icon in enumerate(layout["inline_icons"]):
                    ipath = f"{cpath}.layout.inline_icons[{iindex}]"
                    exact = _exact_keys(icon, {"run_ordinal", "sprite", "frame", "x", "y", "baseline", "advance"}, ipath, problems)
                    if exact is None:
                        continue
                    ordinal = exact.get("run_ordinal")
                    if (not _uint(ordinal) or not _portable(exact.get("sprite"))
                            or not _uint(exact.get("frame"))
                            or not _i32(exact.get("x")) or not _i32(exact.get("y"))
                            or not _i32(exact.get("baseline"))
                            or not _is_int(exact.get("advance"))
                            or exact.get("advance") <= 0
                            or exact.get("advance") >= (1 << 31)
                            or not _i32(exact.get("x") + exact.get("advance"))
                            or not _i32(exact.get("y") + exact.get("baseline"))):
                        problems.append(f"{ipath} invalid")
                    else:
                        mixed_ordinals.append(ordinal)
                if sorted(mixed_ordinals) != list(range(len(layout["glyphs"]) + len(layout["inline_icons"]))):
                    problems.append(f"{cpath}.layout mixed glyph/icon run order is duplicate or incomplete")
        producer = _exact_keys(row.get("producer"), {"setter_entry_rva", "setter_caller_rva", "formatter_entry_rva", "identity_source"}, f"{cpath}.producer", problems)
        if producer is not None:
            expected_source = source.get("kind") if isinstance(source, dict) else None
            expected_identity = "authored_widget_registration" if expected_source == "authored_token" else "proven_setter_caller"
            caller = producer.get("setter_caller_rva")
            formatter = producer.get("formatter_entry_rva")
            if (producer.get("setter_entry_rva") != A25_TEXT_SETTER_RVA
                    or producer.get("identity_source") != expected_identity
                    or not (caller is None or _uint(caller, positive=True))
                    or not (formatter is None or formatter == A25_FORMATTER_RVA)
                    or (expected_source == "setter_caller_rva" and caller != source.get("rva"))):
                problems.append(f"{cpath}.producer exact authority invalid")


def _validate_delayed_event_lifecycle(snap: Dict[str, Any], problems: List[str]) -> None:
    key = "_r2_delayed_event_lifecycle_live"
    if key not in snap or snap.get(key) is None:
        return
    root = _exact_keys(snap[key], {
        "schema_version", "authority", "executable_sha256", "hourly_update_rva",
        "definition_cancel_rva", "shared_dispatch_rva", "hook_epoch",
        "first_sequence", "last_sequence", "complete", "events",
    }, key, problems)
    if root is None:
        return
    hook_epoch = _canonical_u64_string(root.get("hook_epoch"))
    first = _canonical_u64_string(root.get("first_sequence"))
    last = _canonical_u64_string(root.get("last_sequence"))
    events = root.get("events")
    header_valid = (
        _uint(root.get("schema_version"), 1) and root.get("schema_version") == 1
        and root.get("authority") == "exact_a729_delayed_event_lifecycle"
        and root.get("executable_sha256") == EXECUTABLE_SHA256
        and root.get("hourly_update_rva") == "0x006efe10"
        and root.get("definition_cancel_rva") == "0x006dd7f0"
        and root.get("shared_dispatch_rva") == "0x00a028a0"
        and hook_epoch is not None and hook_epoch > 0
        and first is not None and last is not None
        and root.get("complete") is True
        and isinstance(events, list) and len(events) <= 1_000_000
    )
    if header_valid:
        if not events:
            header_valid = first == 0 and last == 0
        else:
            header_valid = last >= first and last - first + 1 == len(events)
    if not header_valid:
        problems.append(f"{key} exact authority/sequence/completeness header invalid")
        return

    reasons = {
        "retained_decrement", "executed", "failed_trigger_consumed",
        "definition_cancelled",
    }
    for index, raw in enumerate(events):
        path = f"{key}.events[{index}]"
        row = _exact_keys(raw, {
            "sequence", "game_hour_raw", "owner_tag_db_id", "event_id",
            "originator_tag_db_id", "scope_random_seed", "due_before",
            "due_after", "reason",
        }, path, problems)
        if row is None:
            continue
        sequence = _canonical_u64_string(row.get("sequence"))
        scope_random = _canonical_u64_string(row.get("scope_random_seed"))
        game_hour_raw = row.get("game_hour_raw")
        due_before, due_after = row.get("due_before"), row.get("due_after")
        reason = row.get("reason")
        identity_valid = (
            sequence == first + index
            and _i32(game_hour_raw) and game_hour_raw != -(1 << 31)
            and _uint(row.get("owner_tag_db_id"), positive=True)
            and _uint(row.get("originator_tag_db_id"), positive=True)
            and _uint(row.get("event_id"))
            and 100_000 <= row.get("event_id") <= 100_099_999
            and scope_random is not None
            and _uint(due_before, 500_000)
            and (due_after is None or _uint(due_after, 500_000))
            and reason in reasons
        )
        if identity_valid:
            if reason == "retained_decrement":
                identity_valid = due_after is not None and due_after + 1 == due_before
            else:
                identity_valid = due_after is None
        if not identity_valid:
            problems.append(f"{path} exact identity/reason/countdown invalid")


def _validate_occupation_garrison_alert(snap: Dict[str, Any], problems: List[str]) -> None:
    key = "_r2_occupation_garrison_alert_live"
    if key not in snap or snap.get(key) is None:
        return
    root = _exact_keys(snap[key], {
        "schema_version", "authority", "executable_sha256", "producer_function_rva",
        "game_hour_raw", "controller_tag_db_id", "predicate_returned",
        "equipment_fixed_point_divisor", "missing_manpower", "missing_equipment",
        "complete",
    }, key, problems)
    if root is None:
        return
    equipment = root.get("missing_equipment")
    game_hour_raw = root.get("game_hour_raw")
    missing_manpower = root.get("missing_manpower")
    active = (_i64(missing_manpower) and missing_manpower > 0) or (
        isinstance(equipment, list) and bool(equipment)
    )
    if (not _uint(root.get("schema_version"), 1) or root.get("schema_version") != 1
            or root.get("authority") != "exact_a729_alert_not_enough_garrison"
            or root.get("executable_sha256") != EXECUTABLE_SHA256
            or root.get("producer_function_rva") != 0x00AFE8A0
            or not _i32(game_hour_raw)
            or game_hour_raw - 43_799_999 != snap.get("hour_index")
            or not _uint(root.get("controller_tag_db_id"), positive=True)
            or type(root.get("predicate_returned")) is not bool
            or root.get("predicate_returned") == active
            or root.get("equipment_fixed_point_divisor") != 100_000
            or not _i64(missing_manpower) or missing_manpower < 0
            or not isinstance(equipment, list) or len(equipment) > 4_096
            or root.get("complete") is not True):
        problems.append(f"{key} exact authority/hour/completeness header invalid")
        return

    controller_id = root["controller_tag_db_id"]
    countries = snap.get("countries")
    joined_tags = []
    if isinstance(countries, dict):
        joined_tags = [
            tag for tag, country in countries.items()
            if (isinstance(country, dict)
                and _uint(country.get("_r2_tag_db_id"), positive=True)
                and country.get("_r2_tag_db_id") == controller_id)
        ]
    if len(joined_tags) != 1 or not _country(joined_tags[0]):
        problems.append(f"{key} controller tag_db_id has no unique current-hour D18 join")

    previous = 0
    for index, raw in enumerate(equipment):
        path = f"{key}.missing_equipment[{index}]"
        row = _exact_keys(raw, {"archetype_id", "whole_units"}, path, problems)
        if row is None:
            continue
        archetype_id, whole_units = row.get("archetype_id"), row.get("whole_units")
        if (not _uint(archetype_id, positive=True) or archetype_id <= previous
                or not _i64(whole_units) or whole_units <= 0):
            problems.append(f"{path} archetype identity/order/amount invalid")
        else:
            previous = archetype_id


def _validate_resolved_tooltip_instances(snap: Dict[str, Any], problems: List[str]) -> None:
    key = "_r2_resolved_tooltip_instances_live"
    if key not in snap or snap.get(key) is None:
        return
    root = _exact_keys(snap[key], {
        "schema_version", "authority", "elapsed_wall_ms", "status_flags", "frame_hour",
        "selected_present_candidate_id", "selected_present_ordinal",
        "selected_present_qpc_ticks", "selected_present_receipt_sha256",
        "producer_manifest_sha256", "completeness_record_identity",
        "expected_instance_count", "instances",
    }, key, problems)
    if root is None:
        return
    instances = root.get("instances")
    completeness = root.get("completeness_record_identity")
    if (root.get("schema_version") != 2
            or root.get("authority") != "exact_a729_tooltip_controller"
            or not _uint(root.get("elapsed_wall_ms"), (1 << 64) - 1)
            or not _uint(root.get("status_flags"), 0)
            or not _uint(root.get("frame_hour"), (1 << 64) - 1)
            or root.get("frame_hour") != snap.get("hour_index")
            or not _uint(root.get("selected_present_candidate_id"), (1 << 64) - 1, positive=True)
            or not _uint(root.get("selected_present_ordinal"))
            or not _uint(root.get("selected_present_qpc_ticks"), (1 << 64) - 1, positive=True)
            or not _sha(root.get("selected_present_receipt_sha256"))
            or root.get("producer_manifest_sha256") != RESOLVED_TOOLTIP_MANIFEST_SHA256
            or not _bounded_nonempty(completeness)
            or completeness.strip() != completeness
            or not isinstance(instances, list) or len(instances) > 1
            or root.get("expected_instance_count") != len(instances)
            or not _uint(root.get("expected_instance_count"), 1)):
        problems.append(f"{key} exact authority/hour/selected-Present/count header invalid")
        return

    for index, raw in enumerate(instances):
        path = f"{key}.instances[{index}]"
        row = _exact_keys(raw, {
            "frame_hour", "pov_country_or_global", "hovered_element_authored_name",
            "tooltip_owner_authored_name", "repeat_index", "screen_rect",
            "viewport_width_px", "viewport_height_px", "producer_record_identity",
            "final_resolved_text",
        }, path, problems)
        if row is None:
            continue
        rect = _exact_keys(row.get("screen_rect"), {"x", "y", "width", "height"}, f"{path}.screen_rect", problems)
        if rect is None:
            continue
        pov = row.get("pov_country_or_global")
        portable_pov = pov == "global" or (
            isinstance(pov, str) and len(pov) == 3
            and pov.isascii() and pov.isalpha() and pov.isupper()
        )
        rect_values = [rect.get(name) for name in ("x", "y", "width", "height")]
        geometry_valid = all(_i32(value) for value in rect_values)
        if geometry_valid:
            x, y, width, height = rect_values
            viewport_width, viewport_height = row.get("viewport_width_px"), row.get("viewport_height_px")
            geometry_valid = (
                width > 0 and height > 0 and _i32(viewport_width) and viewport_width > 0
                and _i32(viewport_height) and viewport_height > 0
                and x + width > 0 and y + height > 0
                and x < viewport_width and y < viewport_height
            )
        if (not _uint(row.get("frame_hour"), (1 << 64) - 1)
                or row.get("frame_hour") != root.get("frame_hour") or not portable_pov
                or not _bounded_nonempty(row.get("hovered_element_authored_name"))
                or row.get("tooltip_owner_authored_name") != "ToolTip"
                or not _uint(row.get("repeat_index"), 0) or not geometry_valid
                or not _bounded_nonempty(row.get("producer_record_identity"))
                or not _bounded_nonempty(row.get("final_resolved_text"))):
            problems.append(f"{path} exact identity/geometry/resolved-text invalid")


def _validate_power_balance_presentation(snap: Dict[str, Any], problems: List[str]) -> None:
    key = "_r2_power_balance_presentation_live"
    if key not in snap or snap.get(key) is None:
        return
    root = _exact_keys(snap[key], {
        "schema_version", "authority", "elapsed_wall_ms", "frame_hour",
        "materialization_qpc_ticks", "selected_present", "producer_manifest_sha256",
        "completeness_record_identity", "expected_balance_count", "balances",
    }, key, problems)
    if root is None:
        return
    balances = root.get("balances")
    selected = _exact_keys(root.get("selected_present"), {
        "frame_hour", "selected_present_candidate_id", "selected_present_ordinal",
        "selected_present_qpc_ticks", "selected_present_cutoff_qpc_ticks",
        "qpc_frequency_hz", "selected_present_receipt_sha256",
    }, f"{key}.selected_present", problems)
    selected_valid = selected is not None and (
        selected.get("frame_hour") == snap.get("hour_index")
        and _uint(selected.get("frame_hour"), (1 << 64) - 1)
        and _uint(selected.get("selected_present_candidate_id"), (1 << 64) - 1, positive=True)
        and _uint(selected.get("selected_present_ordinal"))
        and _uint(selected.get("selected_present_qpc_ticks"), (1 << 64) - 1, positive=True)
        and _uint(selected.get("selected_present_cutoff_qpc_ticks"), (1 << 64) - 1)
        and selected.get("selected_present_cutoff_qpc_ticks") >= selected.get("selected_present_qpc_ticks")
        and _uint(selected.get("qpc_frequency_hz"), (1 << 64) - 1, positive=True)
        and _sha(selected.get("selected_present_receipt_sha256"))
        and _uint(root.get("materialization_qpc_ticks"), (1 << 64) - 1, positive=True)
        and root.get("materialization_qpc_ticks") <= selected.get("selected_present_qpc_ticks")
    )
    if (root.get("schema_version") != 3
            or root.get("authority") != "exact_a729_power_balance_controller"
            or not _uint(root.get("elapsed_wall_ms"), (1 << 64) - 1)
            or not _uint(root.get("frame_hour"), (1 << 64) - 1)
            or root.get("frame_hour") != snap.get("hour_index")
            or not selected_valid
            or root.get("producer_manifest_sha256") != POWER_BALANCE_MANIFEST_SHA256
            or not _exact_text(root.get("completeness_record_identity"))
            or not isinstance(balances, list) or len(balances) > 4_096
            or not _uint(root.get("expected_balance_count"), 4_096)
            or root.get("expected_balance_count") != len(balances)):
        problems.append(f"{key} exact authority/frame/selected-Present/completeness invalid")
        return

    country_tags: Set[str] = set()
    balance_ids: Set[Tuple[str, str]] = set()
    balance_sources: Set[str] = set()
    balance_keys = {
        "source_record_id", "country_tag", "balance_id", "display_ordinal",
        "active_side_id", "trending_side_id", "active_range_id", "effective_left_icon",
        "effective_right_icon", "launcher_visible", "launcher_percentage_text_ref",
        "launcher_levels_text_ref", "launcher_icon_sprite", "launcher_glow_visible",
        "panel_open", "bar_state", "range_indicator_frame", "range_indicator_top_left",
        "trend", "power_balance_modifier_desc_text_ref", "decisions_complete",
        "decisions", "resolved_text_refs",
    }
    for index, raw in enumerate(balances):
        path = f"{key}.balances[{index}]"
        row = _exact_keys(raw, balance_keys, path, problems)
        if row is None:
            continue
        country, balance_id, source = (
            row.get("country_tag"), row.get("balance_id"), row.get("source_record_id")
        )
        country_valid = (
            isinstance(country, str) and len(country) == 3 and country != "---"
            and country.isascii() and country.isalpha() and country.isupper()
        )
        launcher_refs = (
            row.get("launcher_percentage_text_ref"), row.get("launcher_levels_text_ref"),
            row.get("launcher_icon_sprite"),
        )
        top_left = row.get("range_indicator_top_left")
        trend = row.get("trend")
        bar_state = row.get("bar_state")
        if trend is None:
            trend_valid = bar_state in {"stable_left", "stable_right"}
        else:
            trend_row = _exact_keys(
                trend, {"cadence", "magnitude", "resolved_text_ref"},
                f"{path}.trend", problems,
            )
            magnitude = trend_row.get("magnitude") if trend_row is not None else None
            trend_valid = (
                trend_row is not None and bar_state in {"moving_left", "moving_right"}
                and trend_row.get("cadence") in {"daily", "weekly"}
                and isinstance(magnitude, (int, float)) and type(magnitude) is not bool
                and math.isfinite(magnitude) and magnitude > 0
                and _exact_text(trend_row.get("resolved_text_ref"))
            )
        resolved = _exact_keys(row.get("resolved_text_refs"), {
            "balance_name", "left_side_name", "right_side_name", "active_range_name",
            "balance_tooltip", "active_range_tooltip",
        }, f"{path}.resolved_text_refs", problems)
        decisions = row.get("decisions")
        row_valid = (
            country_valid and country not in country_tags
            and _exact_text(source) and source not in balance_sources
            and _exact_text(balance_id) and (country, balance_id) not in balance_ids
            and _uint(row.get("display_ordinal"), 0)
            and all(_exact_text(row.get(name)) for name in (
                "active_side_id", "trending_side_id", "active_range_id",
                "effective_left_icon", "effective_right_icon",
                "power_balance_modifier_desc_text_ref",
            ))
            and row.get("launcher_visible") is True
            and all(_exact_text(value) for value in launcher_refs)
            and type(row.get("launcher_glow_visible")) is bool
            and row.get("panel_open") is True
            and bar_state in {"stable_left", "stable_right", "moving_left", "moving_right"}
            and _uint(row.get("range_indicator_frame"), 1)
            and isinstance(top_left, list) and len(top_left) == 2
            and all(_i32(value) for value in top_left)
            and trend_valid
            and row.get("decisions_complete") is True
            and isinstance(decisions, list) and len(decisions) <= 131_072
            and resolved is not None
            and all(_exact_text(resolved.get(name)) for name in (
                "balance_name", "left_side_name", "right_side_name", "active_range_name",
                "balance_tooltip", "active_range_tooltip",
            ))
        )
        if not row_valid:
            problems.append(f"{path} exact balance identity/presentation invalid")
            continue
        country_tags.add(country)
        balance_ids.add((country, balance_id))
        balance_sources.add(source)

        decision_ids: Set[str] = set()
        decision_sources: Set[str] = set()
        for decision_index, decision_raw in enumerate(decisions):
            decision_path = f"{path}.decisions[{decision_index}]"
            decision = _exact_keys(decision_raw, {
                "decision_id", "source_record_id", "availability",
                "resolved_name_text_ref", "cost_text_ref", "timer_text_ref",
                "target_tag_ref",
            }, decision_path, problems)
            if decision is None:
                continue
            decision_id = decision.get("decision_id")
            decision_source = decision.get("source_record_id")
            optional_refs = (
                decision.get("cost_text_ref"), decision.get("timer_text_ref"),
                decision.get("target_tag_ref"),
            )
            if (not _exact_text(decision_id) or decision_id in decision_ids
                    or not _exact_text(decision_source) or decision_source in decision_sources
                    or decision.get("availability") not in {
                        "available", "unavailable", "active", "on_cooldown",
                    }
                    or not _exact_text(decision.get("resolved_name_text_ref"))
                    or any(value is not None and not _exact_text(value) for value in optional_refs)):
                problems.append(f"{decision_path} exact decision provenance/content invalid")
            else:
                decision_ids.add(decision_id)
                decision_sources.add(decision_source)


def _validate_selected_present_cross_carrier(
    snap: Dict[str, Any], problems: List[str]
) -> None:
    stamps: List[Tuple[str, Dict[str, Any], bool]] = []
    tooltip = snap.get("_r2_resolved_tooltip_instances_live")
    if isinstance(tooltip, dict):
        stamps.append(("_r2_resolved_tooltip_instances_live", {
            "candidate": tooltip.get("selected_present_candidate_id"),
            "ordinal": tooltip.get("selected_present_ordinal"),
            "ticks": tooltip.get("selected_present_qpc_ticks"),
            "receipt": tooltip.get("selected_present_receipt_sha256"),
        }, True))
    for key, ordinal_is_selected in (
        ("_r2_power_balance_presentation_live", True),
        ("_r2_decision_panel_live", True),
        ("_r2_national_focus_presentation_live", False),
    ):
        root = snap.get(key)
        selected = root.get("selected_present") if isinstance(root, dict) else None
        if not isinstance(selected, dict):
            continue
        stamps.append((key, {
            "candidate": selected.get("selected_present_candidate_id"),
            "ordinal": selected.get("selected_present_ordinal"),
            "ticks": selected.get("selected_present_qpc_ticks"),
            "receipt": selected.get("selected_present_receipt_sha256"),
            "frequency": selected.get("qpc_frequency_hz"),
        }, ordinal_is_selected))
    for field in ("candidate", "ticks", "receipt", "frequency"):
        values = {
            stamp.get(field) for _, stamp, _ in stamps
            if (isinstance(stamp.get(field), str) if field == "receipt"
                else _is_int(stamp.get(field)))
        }
        if len(values) > 1:
            problems.append(f"selected-Present {field} disagrees across exact presentation carriers")
    ordinals = {
        stamp.get("ordinal") for _, stamp, compare_ordinal in stamps
        if compare_ordinal and _is_int(stamp.get("ordinal"))
    }
    if len(ordinals) > 1:
        problems.append("selected-Present ordinal disagrees across exact presentation carriers")


def _validate_a30(snap: Dict[str, Any], problems: List[str]) -> None:
    world = snap.get("diplomacy_world")
    extra = world.get("domain53_extra") if isinstance(world, dict) else None
    if not isinstance(extra, dict):
        return
    d18 = _d18_map(snap)
    known_operative_ids: Set[int] = set()
    operative_ids_by_owner: Dict[str, Set[int]] = {}
    operative_identity_valid = True
    countries = snap.get("countries")
    if not isinstance(countries, dict):
        operative_identity_valid = False
    else:
        for country_tag, country in countries.items():
            if not isinstance(country, dict):
                operative_identity_valid = False
                continue
            intel = country.get("intel")
            intel_extra = intel.get("domain53_extra") if isinstance(intel, dict) else None
            rows = intel_extra.get("operative_lifecycle_rows") if isinstance(intel_extra, dict) else None
            if rows is None:
                continue
            if not isinstance(rows, list):
                operative_identity_valid = False
                continue
            for operative in rows:
                operative_id = operative.get("op_id") if isinstance(operative, dict) else None
                if (not _uint(operative_id, positive=True)
                        or operative_id in known_operative_ids):
                    operative_identity_valid = False
                else:
                    known_operative_ids.add(operative_id)
                    operative_ids_by_owner.setdefault(country_tag, set()).add(operative_id)
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
                definition = exact.get("definition_id")
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
                    target_valid = _uint(target.get("state_id"), 65_535, positive=True)
                op_assigned = exact.get("assigned_operative_ids")
                valid_assigned = isinstance(op_assigned, list) and len(op_assigned) <= 64
                if valid_assigned:
                    local: Set[int] = set()
                    for operative_id in op_assigned:
                        if (not _uint(operative_id, positive=True)
                                or operative_id in local or operative_id in assigned
                                or not operative_identity_valid
                                or operative_id not in known_operative_ids):
                            valid_assigned = False
                            break
                        local.add(operative_id)
                        assigned.add(operative_id)
                valid = (_uint(stable, positive=True) and stable not in seen_ops
                         and exact.get("operation_id") == f"operation:{stable}"
                         and _portable(definition, 256)
                         and all(char.isascii() and (char.isalnum() or char in "_-.:/")
                                 for char in definition)
                         and _country(owner)
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
                if (not preparing
                        and (not _uint(exact.get("start_hour"), positive=True)
                             or not _uint(exact.get("duration_hours"), positive=True)
                             or exact.get("duration_hours") % 24 != 0
                             or not _uint(snap.get("hour_index"), (1 << 64) - 1)
                             or exact.get("start_hour") > snap.get("hour_index"))):
                    valid = False
                if status == "prepared" and exact.get("progress_fixed") != 100_000:
                    valid = False
                if status == "finished" and (phase + 1 != phases or exact.get("progress_fixed") != 100_000):
                    valid = False
                if not valid:
                    problems.append(f"{rpath} exact operation identity/state invalid")
                else:
                    seen_ops.add(stable)
    token_memberships = extra.get("operation_token_memberships")
    if token_memberships is not None:
        path = "diplomacy_world.domain53_extra.operation_token_memberships"
        if not isinstance(token_memberships, list):
            problems.append(f"{path} should be list|null")
        else:
            seen_tokens: Set[Tuple[int, int, str]] = set()
            expected = {"owner", "owner_tag_db_id", "target", "target_tag_db_id", "token_id", "producer"}
            for index, row in enumerate(token_memberships):
                rpath = f"{path}[{index}]"
                exact = _exact_keys(row, expected, rpath, problems)
                if exact is None:
                    continue
                owner, owner_id = exact.get("owner"), exact.get("owner_tag_db_id")
                target, target_id = exact.get("target"), exact.get("target_tag_db_id")
                token = exact.get("token_id")
                identity = (owner_id, target_id, token)
                if (not _country(owner) or not _country(target)
                        or not _uint(owner_id, positive=True) or not _uint(target_id, positive=True)
                        or owner_id == target_id or not d18
                        or d18.get(owner) != owner_id or d18.get(target) != target_id
                        or not isinstance(token, str) or not token.isascii()
                        or not 0 < len(token.encode("utf-8")) <= 128
                        or any(not (char.isalnum() or char in "_-.:") for char in token)
                        or exact.get("producer") != "COperationTokensDatabase/COperationTokenManager exact census"
                        or identity in seen_tokens):
                    problems.append(f"{rpath} exact operation-token membership invalid")
                else:
                    seen_tokens.add(identity)
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
    _validate_a30_networks(
        extra.get("intel_network_graph_rows"), d18, operative_ids_by_owner, problems
    )
    _validate_a30_refreshes(extra, snap.get("hour_index"), d18, problems)


def _validate_a30_networks(
    value: Any,
    d18: Dict[str, int],
    operative_ids_by_owner: Dict[str, Set[int]],
    problems: List[str],
) -> None:
    if value is None:
        return
    path = "diplomacy_world.domain53_extra.intel_network_graph_rows"
    if not isinstance(value, list):
        problems.append(f"{path} should be list|null")
        return
    seen: Set[Tuple[int, int]] = set()
    assigned_operatives: Set[int] = set()
    expected = {"owner", "owner_tag_db_id", "strategic_operative_index", "network_slot", "target", "target_tag_db_id", "state_ids", "vertex_count", "state_strengths", "state_strength_scale", "state_strength_producer", "national_coverage"}
    assignment_fields = {"operative_ids", "operative_assignment_producer"}
    for index, row in enumerate(value):
        rpath = f"{path}[{index}]"
        row_expected = expected
        if isinstance(row, dict) and assignment_fields.intersection(row):
            row_expected = expected | assignment_fields
        exact = _exact_keys(row, row_expected, rpath, problems)
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
        operative_ids = exact.get("operative_ids")
        assignment_producer = exact.get("operative_assignment_producer")
        if operative_ids is None and assignment_producer is None:
            assignments_valid = True
        else:
            owner_ids = operative_ids_by_owner.get(owner)
            assignments_valid = (
                isinstance(operative_ids, list)
                and assignment_producer == "FUN_1411BCDD0/FUN_140202460"
                and owner is not None
                and all(_uint(operative_id, positive=True) for operative_id in operative_ids)
                and all(left < right for left, right in zip(operative_ids, operative_ids[1:]))
                and (not operative_ids or owner_ids is not None)
                and all(owner_ids is not None and operative_id in owner_ids
                        and operative_id not in assigned_operatives
                        for operative_id in operative_ids)
            )
            if assignments_valid:
                assigned_operatives.update(operative_ids)
        identity = (strategic, slot)
        if (not owner_valid or not _uint(strategic, positive=True) or not _uint(slot)
                or identity in seen or not _country(target) or not _uint(target_id, positive=True)
                or target_id == strategic
                or (d18 and d18.get(target) != target_id) or not states_valid
                or exact.get("vertex_count") != len(states or []) or not strengths_valid
                or not coverage_valid or not assignments_valid):
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


def _validate_a33(snap: Dict[str, Any], problems: List[str]) -> None:
    key = "_r2_sunk_ship_history_live"
    if key not in snap or snap.get(key) is None:
        return
    root_keys = {
        "schema_version", "authority", "executable_sha256", *SUNK_SHIP_HASHES.keys(),
        "elapsed_wall_ms", "frame_hour", "source_sequence", "producer_complete", "events",
    }
    root = _exact_keys(snap[key], root_keys, key, problems)
    if root is None:
        return
    events = root.get("events")
    sequence = root.get("source_sequence")
    if (root.get("schema_version") != 2
            or root.get("authority") != "exact_a729_sunk_ship_writer"
            or root.get("executable_sha256") != EXECUTABLE_SHA256
            or any(root.get(name) != expected for name, expected in SUNK_SHIP_HASHES.items())
            or not _uint(root.get("elapsed_wall_ms"), (1 << 64) - 1)
            or not _uint(root.get("frame_hour"), (1 << 64) - 1)
            or root.get("frame_hour") != snap.get("hour_index")
            or not _uint(sequence, (1 << 64) - 1, positive=True)
            or root.get("producer_complete") is not True
            or not isinstance(events, list) or len(events) > 65_536):
        problems.append(f"{key} exact authority/frame/header invalid")
        return

    def valid_text(value: Any, required: bool) -> bool:
        if not isinstance(value, str) or "\0" in value or (required and not value):
            return False
        try:
            return len(value.encode("utf-8")) <= 4_096
        except UnicodeEncodeError:
            return False

    event_keys = {
        "event_sequence", "combat_id", "combat_generation", "victim_country_id_raw",
        "attacker_country_id_raw", "ui_source_country_id_raw", "sink_date_raw",
        "victim_member_battle_slot_index", "killed", "assisted_kill",
        "victim_ship_reference_raw", "damage_source_reference_raw",
        "task_force_id_raw", "terminal_context_raw", "cause_record_flag",
        "ship_name", "ship_class_name", "attribution_source_name",
        "attribution_source_class_name",
    }
    previous = 1
    seen: Set[int] = set()
    for index, raw in enumerate(events):
        path = f"{key}.events[{index}]"
        row = _exact_keys(raw, event_keys, path, problems)
        if row is None:
            continue
        event_sequence = row.get("event_sequence")
        sink_date = row.get("sink_date_raw")
        victim_ref = row.get("victim_ship_reference_raw")
        damage_ref = row.get("damage_source_reference_raw")
        terminal_context = row.get("terminal_context_raw")
        references_valid = (
            isinstance(victim_ref, list) and len(victim_ref) == 2
            and all(_uint(value) for value in victim_ref) and victim_ref != [0, 0]
            and isinstance(damage_ref, list) and len(damage_ref) == 2
            and all(_uint(value) for value in damage_ref)
            and isinstance(terminal_context, list) and len(terminal_context) == 2
            and all(_uint(value) for value in terminal_context)
        )
        if (not _uint(event_sequence, (1 << 64) - 1, positive=True)
                or event_sequence <= previous or event_sequence in seen
                or not _uint(row.get("combat_id"), positive=True)
                or not _uint(row.get("combat_generation"), (1 << 64) - 1, positive=True)
                or any(not _uint(row.get(name), positive=True) for name in (
                    "victim_country_id_raw", "attacker_country_id_raw"
                ))
                or not _uint(row.get("ui_source_country_id_raw"))
                or not _is_int(sink_date) or not 0 < sink_date <= root.get("frame_hour")
                or not _uint(row.get("victim_member_battle_slot_index"), 4_096)
                or type(row.get("killed")) is not bool or type(row.get("assisted_kill")) is not bool
                or row.get("killed") == row.get("assisted_kill")
                or not _uint(row.get("task_force_id_raw"))
                or type(row.get("cause_record_flag")) is not bool
                or (row.get("cause_record_flag") and row.get("assisted_kill"))
                or not references_valid
                or not valid_text(row.get("ship_name"), True)
                or not valid_text(row.get("ship_class_name"), True)
                or not valid_text(row.get("attribution_source_name"), False)
                or not valid_text(row.get("attribution_source_class_name"), False)):
            problems.append(f"{path} exact writer event identity/content invalid")
        else:
            seen.add(event_sequence)
            previous = event_sequence
    expected_tail = events[-1].get("event_sequence") if events and isinstance(events[-1], dict) else 1
    if sequence != expected_tail:
        problems.append(f"{key} source sequence/history tail mismatch")


def _validate_a34(snap: Dict[str, Any], problems: List[str]) -> None:
    key = "_r2_decision_panel_live"
    if key not in snap or snap.get(key) is None:
        return
    root_keys = {
        "schema_version", "authority", "executable_sha256", "function_manifest_sha256",
        "data_artifact_sha256", "country_view_owner_tag_manifest_sha256",
        "semantic_leaf_manifest_sha256", "on_map_identity_manifest_sha256",
        "elapsed_wall_ms", "frame_hour", "source_sequence",
        "producer_complete", "country_tag", "decision_catalog_sha256",
        "materialization_qpc_ticks", "selected_present", "grid", "children",
        "available_alert_count", "timeout_alert_count", "rows",
    }
    root = _exact_keys(snap[key], root_keys, key, problems)
    if root is None:
        return
    children, rows = root.get("children"), root.get("rows")
    country = root.get("country_tag")
    d18 = _d18_map(snap)
    if (root.get("schema_version") != 3 or root.get("authority") != "exact_a729_decision_panel"
            or root.get("executable_sha256") != EXECUTABLE_SHA256
            or root.get("function_manifest_sha256") != DECISION_PANEL_MANIFEST_SHA256
            or root.get("data_artifact_sha256") != DECISION_PANEL_DATA_SHA256
            or root.get("country_view_owner_tag_manifest_sha256") != DECISION_PANEL_COUNTRY_OWNER_SHA256
            or root.get("semantic_leaf_manifest_sha256") != "082d3f982a260c085f19c320bf1368e79395821e3b0a42007eb1648bcee6348e"
            or root.get("on_map_identity_manifest_sha256") != "a412d4480732030ae8bbffd5c135f9e22029bdcdee40dcd760c025da30fc0130"
            or not _uint(root.get("elapsed_wall_ms"), (1 << 64) - 1)
            or not _uint(root.get("frame_hour"), (1 << 64) - 1)
            or root.get("frame_hour") != snap.get("hour_index")
            or not _uint(root.get("source_sequence"), (1 << 64) - 1, positive=True)
            or root.get("producer_complete") is not True or not _country(country)
            or (d18 and country not in d18) or not _sha(root.get("decision_catalog_sha256"))
            or not _uint(root.get("materialization_qpc_ticks"), (1 << 64) - 1, positive=True)
            or not isinstance(children, list) or len(children) > 32_768
            or not isinstance(rows, list) or len(rows) > 32_768
            or not _uint(root.get("available_alert_count"))
            or not _uint(root.get("timeout_alert_count"))):
        problems.append(f"{key} exact authority/frame/completeness invalid")
        return

    selected = _exact_keys(root.get("selected_present"), {
        "frame_hour", "selected_present_ordinal", "selected_present_qpc_ticks",
        "qpc_frequency_hz", "selected_present_receipt_sha256",
    }, f"{key}.selected_present", problems)
    selected_valid = selected is not None and (
        selected.get("frame_hour") == snap.get("hour_index")
        and _uint(selected.get("selected_present_ordinal"))
        and _uint(selected.get("selected_present_qpc_ticks"), (1 << 64) - 1, positive=True)
        and root.get("materialization_qpc_ticks") <= selected.get("selected_present_qpc_ticks")
        and _uint(selected.get("qpc_frequency_hz"), (1 << 64) - 1, positive=True)
        and _sha(selected.get("selected_present_receipt_sha256"))
    )
    if not selected_valid:
        problems.append(f"{key}.selected_present receipt metadata invalid")

    grid = _exact_keys(root.get("grid"), {
        "clip_left", "clip_top", "clip_right", "clip_bottom", "content_origin_x",
        "content_origin_y", "content_extent_width", "content_extent_height", "scroll_min",
        "scroll_max", "scroll_target", "scroll_current",
    }, f"{key}.grid", problems)
    if grid is not None:
        i32_names = ("clip_left", "clip_top", "clip_right", "clip_bottom", "content_origin_x", "content_origin_y", "content_extent_width", "content_extent_height")
        i64_names = ("scroll_min", "scroll_max", "scroll_target", "scroll_current")
        valid_grid = (all(_is_int(grid.get(name)) and -(1 << 31) <= grid.get(name) < (1 << 31) for name in i32_names)
                      and all(_i64(grid.get(name)) for name in i64_names))
        if valid_grid:
            valid_grid = (grid["clip_right"] > grid["clip_left"] and grid["clip_bottom"] > grid["clip_top"]
                          and grid["content_extent_width"] >= 0 and grid["content_extent_height"] >= 0
                          and grid["scroll_min"] <= grid["scroll_max"]
                          and grid["scroll_min"] <= grid["scroll_target"] <= grid["scroll_max"]
                          and grid["scroll_min"] <= grid["scroll_current"] <= grid["scroll_max"])
        if not valid_grid:
            problems.append(f"{key}.grid clip/content/scroll state invalid")

    def valid_text(value: Any, required: bool = True) -> bool:
        if not isinstance(value, str) or "\0" in value or (required and not value):
            return False
        try:
            return len(value.encode("utf-8")) <= 16_384
        except UnicodeEncodeError:
            return False

    referenced_rows: Set[int] = set()
    for ordinal, raw in enumerate(children):
        path = f"{key}.children[{ordinal}]"
        child = _exact_keys(raw, {"presentation_index", "kind", "decision_row_index", "identity", "name_text", "secondary_text", "expanded", "x", "y", "width", "height"}, path, problems)
        if child is None:
            continue
        kind, row_index = child.get("kind"), child.get("decision_row_index")
        is_decision = kind == "decision"
        reference_valid = (_uint(row_index) and row_index < len(rows) and row_index not in referenced_rows) if is_decision else row_index is None
        raw_identity = child.get("identity")
        event_id = raw_identity.removeprefix("event_") if kind == "event" and isinstance(raw_identity, str) else ""
        structural_valid = (
            (kind == "category" and child.get("name_text") is not None and type(child.get("expanded")) is bool)
            or (kind == "fixed_header" and child.get("identity") == "event_header"
                and child.get("secondary_text") is None and child.get("expanded") is None)
            or (kind == "event" and event_id.isascii() and event_id.isdigit()
                and int(event_id) <= 0xFFFFFFFF
                and child.get("name_text") is not None and child.get("expanded") is None)
            or (kind == "decision" and child.get("expanded") is None)
        )
        if (child.get("presentation_index") != ordinal
                or kind not in {"category", "fixed_header", "event", "decision"}
                or not reference_valid or not structural_valid or not valid_text(child.get("identity"))
                or not (child.get("name_text") is None or valid_text(child.get("name_text")))
                or not (child.get("secondary_text") is None or valid_text(child.get("secondary_text")))
                or not (child.get("expanded") is None or type(child.get("expanded")) is bool)
                or any(not (_is_int(child.get(name)) and -(1 << 31) <= child.get(name) < (1 << 31)) for name in ("x", "y", "width", "height"))
                or child.get("width", 0) <= 0 or child.get("height", 0) <= 0):
            problems.append(f"{path} heterogeneous child order/content invalid")
        elif is_decision:
            referenced_rows.add(row_index)

    identities: Set[Tuple[str, Any]] = set()
    available_alerts = 0
    timeout_alerts = 0
    availability = {"available", "unavailable", "active", "on_cooldown"}
    lifecycle = {"pending", "running", "completed", "failed", "removed"}
    for index, raw in enumerate(rows):
        path = f"{key}.rows[{index}]"
        row = _exact_keys(raw, {"decision_id", "category_id", "presentation_index", "availability", "lifecycle", "interface_state", "on_map_entry_ordinal", "target", "cost", "remaining_timer_days", "cooldown_remaining_days", "name_text", "timer_text", "alert_to_take", "alert_timeout"}, path, problems)
        if row is None:
            continue
        presentation_index = row.get("presentation_index")
        target = row.get("target")
        target_key: Any = None
        target_valid = target is None
        if isinstance(target, dict) and set(target) == {"kind", "tag"} and target.get("kind") == "country":
            tag = target.get("tag")
            target_valid = _country(tag) and (not d18 or tag in d18)
            target_key = ("country", tag)
        elif isinstance(target, dict) and set(target) == {"kind", "state_id"} and target.get("kind") == "state":
            target_valid = _uint(target.get("state_id"), positive=True)
            target_key = ("state", target.get("state_id"))
        cost = row.get("cost")
        cost_valid = cost is None
        if isinstance(cost, dict) and set(cost) == {"political_power", "custom_cost_available", "formatted_text"}:
            cost_valid = ((cost.get("political_power") is None or _i64(cost.get("political_power")))
                          and (cost.get("custom_cost_available") is None or type(cost.get("custom_cost_available")) is bool)
                          and (cost.get("formatted_text") is None or valid_text(cost.get("formatted_text"))))
        remaining, cooldown = row.get("remaining_timer_days"), row.get("cooldown_remaining_days")
        timers_valid = ((remaining is None or (_is_int(remaining) and 0 <= remaining < (1 << 31)))
                        and (cooldown is None or (_is_int(cooldown) and 0 <= cooldown < (1 << 31)))
                        and ((remaining is not None or cooldown is not None) == (row.get("timer_text") is not None))
                        and (row.get("timer_text") is None or valid_text(row.get("timer_text"))))
        interface_state = row.get("interface_state")
        on_map_entry_ordinal = row.get("on_map_entry_ordinal")
        family_shape_valid = (
            (interface_state is None) == (on_map_entry_ordinal is not None)
            and (on_map_entry_ordinal is None
                 or _uint(on_map_entry_ordinal, 0xFFFFFFFE))
        )
        if cooldown is not None and cooldown > 0:
            interface_state_valid = (
                row.get("availability") == "on_cooldown"
                and row.get("lifecycle") == "pending"
            )
        elif interface_state is None or interface_state in {1, 6, 9}:
            interface_state_valid = (
                row.get("availability") == "available"
                and row.get("lifecycle") == "pending"
            )
        elif interface_state in {3, 4, 10, 11}:
            interface_state_valid = (
                row.get("availability") == "active"
                and row.get("lifecycle") == "running"
            )
        elif interface_state == 12:
            interface_state_valid = (
                row.get("availability") == "unavailable"
                and row.get("lifecycle") == "completed"
            )
        elif interface_state in {0, 2, 5, 7, 8}:
            interface_state_valid = (
                row.get("availability") == "unavailable"
                and row.get("lifecycle") == "pending"
            )
        else:
            interface_state_valid = False
        identity = (row.get("decision_id"), target_key, on_map_entry_ordinal)
        child_join = (_uint(presentation_index) and presentation_index < len(children)
                      and isinstance(children[presentation_index], dict)
                      and children[presentation_index].get("kind") == "decision"
                      and children[presentation_index].get("decision_row_index") == index
                      and children[presentation_index].get("identity") == row.get("decision_id")
                      and children[presentation_index].get("name_text") == row.get("name_text")
                      and children[presentation_index].get("secondary_text") == row.get("timer_text"))
        if (not child_join or not valid_text(row.get("decision_id")) or not valid_text(row.get("category_id"))
                or not valid_text(row.get("name_text")) or row.get("availability") not in availability
                or row.get("lifecycle") not in lifecycle or not target_valid or not cost_valid or not timers_valid
                or not (row.get("interface_state") is None
                        or _uint(row.get("interface_state"), 12))
                or not interface_state_valid
                or not family_shape_valid
                or type(row.get("alert_to_take")) is not bool or type(row.get("alert_timeout")) is not bool
                or identity in identities):
            problems.append(f"{path} ordered identity/content invalid")
        else:
            identities.add(identity)
            available_alerts += row["alert_to_take"]
            timeout_alerts += row["alert_timeout"]
    if referenced_rows != set(range(len(rows))):
        problems.append(f"{key} child/decision row bijection invalid")
    if root.get("available_alert_count") != available_alerts or root.get("timeout_alert_count") != timeout_alerts:
        problems.append(f"{key} alert membership/count mismatch")


def _validate_a38(snap: Dict[str, Any], problems: List[str]) -> None:
    key = "_r2_scripted_gui_runtime_live"
    countries = snap.get("countries") if isinstance(snap.get("countries"), dict) else {}
    runtime_rows: Dict[Tuple[str, str], Dict[str, Any]] = {}
    for country_tag, country in countries.items():
        extra = country.get("extra") if isinstance(country, dict) else None
        runtime = extra.get("scripted_gui_runtime") if isinstance(extra, dict) else None
        if runtime is None:
            continue
        if not isinstance(runtime, dict):
            problems.append(f"countries.{country_tag}.extra.scripted_gui_runtime should be an object")
            continue
        for gui_id, row in runtime.items():
            if isinstance(row, dict):
                runtime_rows[(country_tag, gui_id)] = row
            else:
                problems.append(f"countries.{country_tag}.extra.scripted_gui_runtime.{gui_id} should be an object")
    if key not in snap or snap.get(key) is None:
        if runtime_rows:
            problems.append(f"{key} authority envelope missing beside d84 rows")
        return

    root = _exact_keys(snap[key], {
        "schema_version", "authority", "elapsed_wall_ms", "frame_hour",
        "producer_record_identity", "executable_sha256", "ui14_engine_vfs_sha256",
        "static_authority_job_id", "static_authority_receipt_sha256",
        "producer_manifest_sha256", "manager_order", "tombstones",
    }, key, problems)
    if root is None:
        return

    def portable(value: Any) -> bool:
        if not isinstance(value, str) or not value.strip() or "\0" in value:
            return False
        try:
            if len(value.encode("utf-8")) > 65_536:
                return False
        except UnicodeEncodeError:
            return False
        lower = value.lower()
        if "\\\\" in value or re.search(r"[A-Za-z]:[\\/]", value):
            return False
        if re.search(r"(?:^|[\s(='\"])/(?:\S*)", value):
            return False
        return re.search(r"0x[0-9a-f]{6,}", lower) is None

    manager_order, tombstones = root.get("manager_order"), root.get("tombstones")
    if (root.get("schema_version") != 1
            or root.get("authority") != "exact20_a38_engine_materialized"
            or not _uint(root.get("elapsed_wall_ms"), (1 << 64) - 1)
            or not _uint(root.get("frame_hour"), (1 << 64) - 1)
            or root.get("frame_hour") != snap.get("hour_index")
            or not portable(root.get("producer_record_identity"))
            or root.get("executable_sha256") != EXECUTABLE_SHA256
            or not _sha(root.get("ui14_engine_vfs_sha256"))
            or root.get("static_authority_job_id") != A38_STATIC_AUTHORITY_JOB_ID
            or root.get("static_authority_receipt_sha256") != A38_STATIC_AUTHORITY_RECEIPT_SHA256
            or not _sha(root.get("producer_manifest_sha256"))
            or not isinstance(manager_order, list) or len(manager_order) > 65_536
            or not isinstance(tombstones, list) or len(tombstones) > 65_536):
        problems.append(f"{key} exact authority/frame/header invalid")
        return

    def engine_result(value: Any, expected: type) -> bool:
        if not isinstance(value, dict) or set(value) != {"value", "unavailable_reason"}:
            return False
        measured = type(value.get("value")) is expected and value.get("unavailable_reason") is None
        unavailable = value.get("value") is None and portable(value.get("unavailable_reason"))
        return measured or unavailable

    actual_keys: Set[Tuple[str, str]] = set()
    for (country_tag, gui_id), row in runtime_rows.items():
        path = f"countries.{country_tag}.extra.scripted_gui_runtime.{gui_id}"
        exact = _exact_keys(row, {"manager_order_index", "definition", "visible", "enabled", "triggers", "properties", "effect_enabled"}, path, problems)
        if exact is None:
            continue
        definition = _exact_keys(exact.get("definition"), {"logical_path", "winning_layer_ordinal", "layer_kind", "layer_id", "source_sha256", "payload_sha256", "payload_bytes", "context_type", "window_name"}, f"{path}.definition", problems)
        definition_valid = False
        if definition is not None:
            logical_path = definition.get("logical_path")
            definition_valid = (
                isinstance(logical_path, str) and logical_path.startswith("common/scripted_guis/")
                and logical_path.endswith(".txt") and "\\" not in logical_path and ":" not in logical_path
                and all(part not in {"", ".", ".."} for part in logical_path.split("/"))
                and _uint(definition.get("winning_layer_ordinal"))
                and definition.get("layer_kind") in {"base", "integrated_dlc", "dlc", "mod"}
                and all(portable(definition.get(name)) for name in ("layer_id", "context_type", "window_name"))
                and _sha(definition.get("source_sha256")) and _sha(definition.get("payload_sha256"))
                and _uint(definition.get("payload_bytes"), (1 << 64) - 1, positive=True)
            )
        maps_valid = True
        for name in ("triggers", "effect_enabled"):
            values = exact.get(name)
            maps_valid &= isinstance(values, dict) and all(engine_result(value, bool) for value in values.values())
        properties = exact.get("properties")
        maps_valid &= (isinstance(properties, dict)
                       and all(isinstance(values, dict)
                               and all(engine_result(value, str) for value in values.values())
                               for values in properties.values()))
        if (not _country(country_tag) or not portable(gui_id)
                or not _uint(exact.get("manager_order_index")) or not definition_valid
                or not engine_result(exact.get("visible"), bool)
                or not engine_result(exact.get("enabled"), bool) or not maps_valid):
            problems.append(f"{path} exact materialized runtime row invalid")
        else:
            actual_keys.add((country_tag, gui_id))

    ordered_keys: Set[Tuple[str, str]] = set()
    for ordinal, raw in enumerate(manager_order):
        path = f"{key}.manager_order[{ordinal}]"
        row = _exact_keys(raw, {"manager_order_index", "country_tag", "gui_id"}, path, problems)
        if row is None:
            continue
        identity = (row.get("country_tag"), row.get("gui_id"))
        runtime = runtime_rows.get(identity)
        if (row.get("manager_order_index") != ordinal or not _country(identity[0])
                or not portable(identity[1]) or identity in ordered_keys or not isinstance(runtime, dict)
                or runtime.get("manager_order_index") != ordinal):
            problems.append(f"{path} exact manager order/runtime join invalid")
        else:
            ordered_keys.add(identity)
    if ordered_keys != actual_keys:
        problems.append(f"{key}.manager_order does not exactly cover d84 rows")

    previous: Optional[Tuple[str, str]] = None
    seen_tombstones: Set[Tuple[str, str]] = set()
    for index, raw in enumerate(tombstones):
        path = f"{key}.tombstones[{index}]"
        row = _exact_keys(raw, {"country_tag", "gui_id"}, path, problems)
        if row is None:
            continue
        identity = (row.get("country_tag"), row.get("gui_id"))
        if (not _country(identity[0]) or not portable(identity[1]) or identity in actual_keys
                or identity in seen_tombstones or (previous is not None and previous >= identity)):
            problems.append(f"{path} exact ordered tombstone invalid")
        else:
            seen_tombstones.add(identity)
            previous = identity


def _validate_a31(snap: Dict[str, Any], problems: List[str]) -> None:
    key = "_r2_national_focus_presentation_live"
    if key not in snap or snap.get(key) is None:
        return
    root = _exact_keys(snap[key], {
        "schema_version", "authority", "executable_sha256", "function_manifest_sha256",
        "job_receipt_sha256", "elapsed_wall_ms", "frame_hour",
        "materialization_qpc_ticks", "selected_present", "source_sequence",
        "producer_complete", "country_tag", "view_id", "producer_record_identity",
        "expected_visible_item_count", "items",
    }, key, problems)
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
            or not _uint(root.get("materialization_qpc_ticks"), (1 << 64) - 1, positive=True)
            or not _uint(sequence, (1 << 64) - 1, positive=True)
            or root.get("producer_complete") is not True or not _country(tag)
            or root.get("view_id") != expected_view or root.get("producer_record_identity") != expected_record
            or not isinstance(items, list) or len(items) > 131_072
            or root.get("expected_visible_item_count") != len(items)):
        problems.append(f"{key} exact authority/frame/completeness invalid")
        return
    selected = _exact_keys(root.get("selected_present"), {
        "frame_hour", "selected_present_global_order", "selected_present_qpc_ticks",
        "qpc_frequency_hz", "selected_present_receipt_sha256",
    }, f"{key}.selected_present", problems)
    if (selected is None
            or selected.get("frame_hour") != snap.get("hour_index")
            or not _uint(selected.get("selected_present_global_order"), positive=True)
            or not _uint(selected.get("selected_present_qpc_ticks"), (1 << 64) - 1, positive=True)
            or root.get("materialization_qpc_ticks") > selected.get("selected_present_qpc_ticks")
            or not _uint(selected.get("qpc_frequency_hz"), (1 << 64) - 1, positive=True)
            or not _sha(selected.get("selected_present_receipt_sha256"))):
        problems.append(f"{key}.selected_present receipt metadata invalid")
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
        if (row.get("source_ordinal") != ordinal or not _focus_id(focus) or focus in seen
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
    expected_root = {"schema_version", "authority", "executable_sha256", *CHARACTER_HASHES.keys(), "elapsed_wall_ms", "frame_hour", "completeness_record_identity", "source_sequence", "trait_trees", "ace_context", "ace_portraits"}
    root = _exact_keys(snap[key], expected_root, key, problems)
    if root is None:
        return
    trees, aces = root.get("trait_trees"), root.get("ace_portraits")
    if (root.get("schema_version") != 3 or root.get("authority") != "exact_a729_character_presentation"
            or root.get("executable_sha256") != EXECUTABLE_SHA256
            or any(root.get(name) != expected for name, expected in CHARACTER_HASHES.items())
            or root.get("frame_hour") != snap.get("hour_index")
            or not _uint(root.get("elapsed_wall_ms"), (1 << 64) - 1)
            or not isinstance(root.get("completeness_record_identity"), str)
            or not root.get("completeness_record_identity").strip()
            or len(root.get("completeness_record_identity").encode("utf-8")) > 1_024
            or not _uint(root.get("source_sequence"), (1 << 64) - 1, positive=True)
            or not isinstance(trees, list) or len(trees) != 2
            or not isinstance(aces, list) or len(aces) > 65_536):
        problems.append(f"{key} exact authority/frame/header invalid")
        return
    context = _exact_keys(root.get("ace_context"), {"kind", "owner_tag_db_id", "wing_id", "plane_type_mask_raw"}, f"{key}.ace_context", problems)
    if context is None:
        return
    selected_wing = (context.get("kind") == "selected_wing"
                     and _uint(context.get("owner_tag_db_id"), positive=True)
                     and _uint(context.get("wing_id"), positive=True)
                     and context.get("plane_type_mask_raw") is None)
    plane_mask = (context.get("kind") == "plane_type_mask"
                  and context.get("owner_tag_db_id") is None
                  and context.get("wing_id") is None
                  and _uint(context.get("plane_type_mask_raw"), positive=True))
    if not (selected_wing or plane_mask):
        problems.append(f"{key}.ace_context selected context invalid")
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
    selected_leader_identity: Optional[Tuple[int, int]] = None
    observed_tree_roots: Set[int] = set()
    total_nodes = 0
    for tree_index, raw in enumerate(trees):
        path = f"{key}.trait_trees[{tree_index}]"
        tree = _exact_keys(raw, {"owner_tag_db_id", "unit_leader_id", "tree_root_offset", "nodes"}, path, problems)
        if tree is None:
            continue
        owner, leader, offset, nodes = tree.get("owner_tag_db_id"), tree.get("unit_leader_id"), tree.get("tree_root_offset"), tree.get("nodes")
        identity = (owner, leader, offset)
        leader_identity = (owner, leader)
        if (not _uint(owner, positive=True) or (d18_ids and owner not in d18_ids)
                or not _uint(leader, (1 << 64) - 1, positive=True) or offset not in {0x120, 0x130}
                or identity in tree_keys
                or (selected_leader_identity is not None
                    and leader_identity != selected_leader_identity)
                or offset in observed_tree_roots or not isinstance(nodes, list)):
            problems.append(f"{path} exact tree identity invalid")
            continue
        if selected_leader_identity is None:
            selected_leader_identity = leader_identity
        tree_keys.add(identity)
        observed_tree_roots.add(offset)
        total_nodes += len(nodes)
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
    if observed_tree_roots != {0x120, 0x130}:
        problems.append(f"{key} trait tree root coverage is not exact")
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
    if len(owner_ordinals) > 1:
        problems.append(f"{key} ace rows span multiple active owners")
    if selected_wing and owner_ordinals and set(owner_ordinals) != {context.get("owner_tag_db_id")}:
        problems.append(f"{key} selected-wing owner contradicts ace rows")
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
            or root.get("task_force_count") != len(rows)
            or root.get("fixture_required_rows") != 0):
        problems.append(f"{path} exact authority/count envelope invalid")
        return
    d18 = _d18_map(snap)
    mission_names = ["HOLD", "PATROL", "STRIKE_FORCE", "CONVOY_RAIDING", "CONVOY_ESCORT", "MINES_PLANTING", "MINES_SWEEPING", "TRAINING", "RESERVE_FLEET", "NAVAL_INVASION_SUPPORT"]
    detached_names = ["none", "repairing", "moving_to_refit", "refitting", "reinforcing"]
    seen: Set[int] = set()
    count_fields = {name: 0 for name in ("validated_location_rows", "derived_region_rows", "ship_summary_rows", "dominant_hull_rows", "fixture_required_rows", "convoy_sunk_count_rows")}
    expected = {"task_force_id", "fleet_id", "owner", "owner_tag", "owner_tag_db_id", "location_province_id", "location_province", "strategic_region_id", "ship_count", "dominant_hull_class", "mission_enum", "mission_name", "mission_state", "detached_activity", "activity_key", "active_combat_epochs", "combat_membership_authority", "home_base_province_id", "repair_base_province_id", "sunk_convoys", "oob_command_task_force_seen"}
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
        nullable_u32 = ("location_province_id", "location_province", "strategic_region_id", "ship_count", "home_base_province_id", "repair_base_province_id", "sunk_convoys")
        dominant_hull = row.get("dominant_hull_class")
        if (not _uint(task_force, positive=True) or task_force in seen or not _uint(row.get("fleet_id"), positive=True)
                or not _country(owner) or row.get("owner") != owner or not _uint(owner_id, positive=True)
                or (d18 and d18.get(owner) != owner_id)
                or any(not (row.get(name) is None or _uint(row.get(name))) for name in nullable_u32)
                or not (dominant_hull is None or _portable(dominant_hull, 128))
                or row.get("location_province") != row.get("location_province_id")
                or row.get("mission_name") != expected_mission or row.get("detached_activity") != expected_detached
                or not epochs_valid or not activity_valid
                or row.get("oob_command_task_force_seen") is not True):
            problems.append(f"{rpath} exact task-force authority/content invalid")
        else:
            seen.add(task_force)
        count_fields["validated_location_rows"] += row.get("location_province_id") is not None
        count_fields["derived_region_rows"] += row.get("strategic_region_id") is not None
        count_fields["ship_summary_rows"] += row.get("ship_count") is not None
        count_fields["dominant_hull_rows"] += row.get("dominant_hull_class") is not None
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
    region_keys = {"region_id", "temperature", "temperature_raw", "phenomenon", "weather_kind", "next_weather_change_raw", "active_modifier_count", "intensity", "rain_light", "rain_heavy", "snow", "blizzard", "arctic_water", "active_signal", "phenomenon_active", "sandstorm"}
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
        named_weather = (
            ("rain_light", "rain_light"),
            ("rain_heavy", "rain_heavy"),
            ("snow", "snow"),
            ("blizzard", "blizzard"),
            ("sandstorm", "sandstorm"),
        )
        active_named = [kind for field, kind in named_weather if row.get(field) is True]
        derived_kind = active_named[0] if active_named else "no_phenomenon"
        if (not _uint(identity, 304) or str(identity) != key or row.get("region_id") != identity
                or not _is_int(temperature_raw) or not -10_000_000 <= temperature_raw <= 10_000_000
                or row.get("temperature") != row.get("temperature_raw") / 100_000
                or not _uint(row.get("phenomenon"), 16)
                or not _uint(row.get("next_weather_change_raw"), 2_147_483_647)
                or not _uint(row.get("active_modifier_count"), 4_096) or not _uint(row.get("intensity"), 255)
                or any(type(row.get(name)) is not bool for name in bools)
                or len(active_named) > 1 or row.get("weather_kind") != derived_kind
                or row.get("active_signal") != row.get("phenomenon_active")
                or row.get("active_signal") != (row.get("phenomenon") != 0)):
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
        # D24 serializes the exact engine province array indexed
        # 0..observed_province_rows-1. ID zero is part of the authenticated
        # completeness carrier even though gameplay references reserve it.
        valid = (_uint(identity, observed_provinces - 1) and str(identity) == key and row.get("province_id") == identity
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
