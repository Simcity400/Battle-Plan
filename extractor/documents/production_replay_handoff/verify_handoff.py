#!/usr/bin/env python3
"""Deterministic verifier for the EXTRACTOR-P replay handoff package."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
DOCUMENTS = HERE.parent
if str(DOCUMENTS) not in sys.path:
    sys.path.insert(0, str(DOCUMENTS))

from production_seal_common import (  # noqa: E402
    atomic_write_json,
    canonical_sha256,
    central_binding,
    repo_sha256,
    verify_central_attestation,
    write_or_check_json,
)

D48_PARTIAL_HEALTH_PER_LEVEL = 10_000_000
D48_U32_MAX = (1 << 32) - 1
D48_U64_MAX = (1 << 64) - 1
A38_EXECUTABLE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
A38_STATIC_AUTHORITY_JOB_ID = "20260720T020812Z-w3-a38-controller-owner-41ee5208"
A38_STATIC_AUTHORITY_RECEIPT_SHA256 = "7d0a36b1822faf85a949d628a79bc4a09f95dc9c9aa3ed4aecb715b72b0a9082"
A38_MAX_GUI_ROWS = 65_536
A38_MAX_RESULTS_PER_MAP = 131_072
HELPER_REL = "extractor/documents/production_seal_common.py"


def sha256(path: Path) -> str:
    return repo_sha256(path)


def load_json(rel: str) -> dict:
    return json.loads((ROOT / rel).read_text(encoding="utf-8"))


def require(value: bool, name: str, checks: list[dict[str, str]]) -> None:
    if not value:
        raise AssertionError(name)
    checks.append({"check": name, "status": "PASS"})


def function_body(source: str, start: str, end: str) -> str:
    start_at = source.index(start)
    end_at = source.index(end, start_at)
    return source[start_at:end_at]


def validate_d48_aggregate_partial_health(row: object) -> tuple[str, ...]:
    """Validate the A14 location/type aggregate bound deterministically.

    JSON Schema draft 2020-12 cannot compare sibling numeric properties.
    `replay_payload.schema.json` therefore declares the relation with the
    handoff-owned `x-hoi4-maximum-product` annotation and this enforcing
    verifier evaluates it. The bound is aggregate, never a clamp to one
    object's 10,000,000-unit accumulator.
    """
    if not isinstance(row, dict):
        return ("building detail is not an object",)
    level = row.get("level")
    if type(level) is not int or not 0 <= level <= D48_U32_MAX:
        return ("level is not an unsigned 32-bit integer",)
    if "partial_health_raw" not in row:
        return ()
    partial = row["partial_health_raw"]
    if type(partial) is not int or not 0 <= partial <= D48_U64_MAX:
        return ("partial_health_raw is not an unsigned 64-bit integer",)
    maximum = level * D48_PARTIAL_HEALTH_PER_LEVEL
    if partial > maximum:
        return (
            f"partial_health_raw {partial} exceeds level-dependent aggregate maximum {maximum}",
        )
    return ()


def a38_leaks_host_detail(value: str) -> bool:
    raw = value.encode("utf-8")
    rooted_unix = any(
        byte == ord("/")
        and (
            index == 0
            or chr(raw[index - 1]).isspace()
            or raw[index - 1] in b"(='\""
        )
        for index, byte in enumerate(raw)
    )
    if rooted_unix or "\\\\" in value:
        return True
    if any(
        chr(part[0]).isalpha() and part[1:3] in (b":\\", b":/")
        for part in (raw[index : index + 3] for index in range(max(0, len(raw) - 2)))
    ):
        return True
    lower = value.lower().encode("utf-8")
    return any(
        part[:2] == b"0x" and all(chr(byte) in "0123456789abcdef" for byte in part[2:])
        for part in (lower[index : index + 8] for index in range(max(0, len(lower) - 7)))
    )


def portable_a38_identity(value: object) -> bool:
    return (
        isinstance(value, str)
        and bool(value.strip())
        and len(value.encode("utf-8")) <= 65_536
        and "\0" not in value
        and not a38_leaks_host_detail(value)
    )


def canonical_a38_path(value: object) -> bool:
    if not isinstance(value, str) or not value.startswith("common/scripted_guis/") or not value.endswith(".txt"):
        return False
    stem = value.removeprefix("common/scripted_guis/").removesuffix(".txt")
    return (
        bool(stem)
        and not stem.endswith("/")
        and "\\" not in value
        and ":" not in value
        and all(part not in ("", ".", "..") for part in value.split("/"))
        and len(value.encode("utf-8")) <= 65_536
        and "\0" not in value
    )


def validate_a38_result(value: object, expected_type: type, ctx: str) -> tuple[str, ...]:
    if not isinstance(value, dict) or set(value) != {"value", "unavailable_reason"}:
        return (f"{ctx}: result shape",)
    concrete = value["value"]
    reason = value["unavailable_reason"]
    if expected_type is bool:
        concrete_ok = type(concrete) is bool
    else:
        concrete_ok = isinstance(concrete, str) and len(concrete.encode("utf-8")) <= 65_536 and "\0" not in concrete
    if concrete_ok and reason is None:
        return ()
    if concrete is None and portable_a38_identity(reason):
        return ()
    return (f"{ctx}: value/unavailable_reason pairing",)


def validate_a38_handoff_snapshot(snapshot: object) -> tuple[str, ...]:
    """Enforce joins JSON Schema cannot express for the d84 portable handoff."""
    if not isinstance(snapshot, dict):
        return ("snapshot: not an object",)
    problems: list[str] = []
    runtime: dict[tuple[str, str], dict] = {}
    countries = snapshot.get("countries", {})
    if not isinstance(countries, dict):
        return ("countries: not an object",)
    for country, country_value in countries.items():
        if not isinstance(country_value, dict):
            continue
        extra = country_value.get("extra")
        if not isinstance(extra, dict) or "scripted_gui_runtime" not in extra:
            continue
        gui_map = extra["scripted_gui_runtime"]
        if not isinstance(gui_map, dict) or len(gui_map) > A38_MAX_GUI_ROWS:
            problems.append(f"{country}: runtime map shape/bound")
            continue
        for gui_id, row in gui_map.items():
            ctx = f"{country}/{gui_id}"
            if (
                not isinstance(country, str)
                or len(country) != 3
                or not all(ch.isascii() and (ch.isupper() or ch.isdigit()) for ch in country)
                or not portable_a38_identity(gui_id)
                or not isinstance(row, dict)
            ):
                problems.append(f"{ctx}: runtime identity/row")
                continue
            required = {"manager_order_index", "definition", "visible", "enabled", "triggers", "properties", "effect_enabled"}
            if set(row) != required or type(row["manager_order_index"]) is not int or not 0 <= row["manager_order_index"] <= 0xFFFFFFFF:
                problems.append(f"{ctx}: runtime field set/order index")
                continue
            definition = row["definition"]
            definition_fields = {"logical_path", "winning_layer_ordinal", "layer_kind", "layer_id", "source_sha256", "payload_sha256", "payload_bytes", "context_type", "window_name"}
            if (
                not isinstance(definition, dict)
                or set(definition) != definition_fields
                or not canonical_a38_path(definition.get("logical_path"))
                or definition.get("layer_kind") not in {"base", "integrated_dlc", "dlc", "mod"}
                or not all(portable_a38_identity(definition.get(key)) for key in ("layer_id", "context_type", "window_name"))
                or not all(isinstance(definition.get(key), str) and len(definition[key]) == 64 and set(definition[key]) <= set("0123456789abcdef") and set(definition[key]) != {"0"} for key in ("source_sha256", "payload_sha256"))
                or type(definition.get("winning_layer_ordinal")) is not int
                or not 0 <= definition["winning_layer_ordinal"] <= 0xFFFFFFFF
                or type(definition.get("payload_bytes")) is not int
                or definition["payload_bytes"] <= 0
            ):
                problems.append(f"{ctx}: definition identity")
                continue
            problems.extend(validate_a38_result(row["visible"], bool, f"{ctx}.visible"))
            problems.extend(validate_a38_result(row["enabled"], bool, f"{ctx}.enabled"))
            for map_key, value_type in (("triggers", bool), ("effect_enabled", bool)):
                values = row[map_key]
                if not isinstance(values, dict) or len(values) > A38_MAX_RESULTS_PER_MAP:
                    problems.append(f"{ctx}.{map_key}: map shape/bound")
                    continue
                for name, result in values.items():
                    if not isinstance(name, str) or not name or len(name.encode("utf-8")) > 65_536 or "\0" in name:
                        problems.append(f"{ctx}.{map_key}: invalid name")
                    problems.extend(validate_a38_result(result, value_type, f"{ctx}.{map_key}.{name}"))
            properties = row["properties"]
            flattened = 0
            if not isinstance(properties, dict):
                problems.append(f"{ctx}.properties: map shape")
            else:
                for element, values in properties.items():
                    if not isinstance(element, str) or not element or len(element.encode("utf-8")) > 65_536 or "\0" in element or not isinstance(values, dict):
                        problems.append(f"{ctx}.properties: invalid element")
                        continue
                    flattened += len(values)
                    for name, result in values.items():
                        if not isinstance(name, str) or not name or len(name.encode("utf-8")) > 65_536 or "\0" in name:
                            problems.append(f"{ctx}.properties: invalid name")
                        problems.extend(validate_a38_result(result, str, f"{ctx}.properties.{element}.{name}"))
                if flattened > A38_MAX_RESULTS_PER_MAP:
                    problems.append(f"{ctx}.properties: flattened map bound")
            runtime[(country, gui_id)] = row

    authority = snapshot.get("_r2_scripted_gui_runtime_live")
    if authority is None:
        if runtime:
            problems.append("d84 runtime rows exist without exact authority")
        return tuple(problems)
    required_authority = {
        "schema_version", "authority", "elapsed_wall_ms", "frame_hour", "producer_record_identity",
        "executable_sha256", "ui14_engine_vfs_sha256", "static_authority_job_id",
        "static_authority_receipt_sha256", "producer_manifest_sha256", "manager_order", "tombstones",
    }
    if not isinstance(authority, dict) or set(authority) != required_authority:
        return tuple(problems + ["d84 authority field set"])
    if (
        type(authority["schema_version"]) is not int
        or authority["schema_version"] != 1
        or authority["authority"] != "exact20_a38_engine_materialized"
        or authority["executable_sha256"] != A38_EXECUTABLE_SHA256
        or authority["static_authority_job_id"] != A38_STATIC_AUTHORITY_JOB_ID
        or authority["static_authority_receipt_sha256"] != A38_STATIC_AUTHORITY_RECEIPT_SHA256
        or not portable_a38_identity(authority["producer_record_identity"])
    ):
        problems.append("d84 authority identity")
    for key in ("ui14_engine_vfs_sha256", "producer_manifest_sha256"):
        value = authority[key]
        if not isinstance(value, str) or len(value) != 64 or set(value) > set("0123456789abcdef") or set(value) == {"0"}:
            problems.append(f"d84 {key}")
    if type(authority["elapsed_wall_ms"]) is not int or authority["elapsed_wall_ms"] < 0 or type(authority["frame_hour"]) is not int or authority["frame_hour"] < 0:
        problems.append("d84 time fields")
    if "hour_index" in snapshot and authority["frame_hour"] != snapshot["hour_index"]:
        problems.append("d84 frame hour join")
    manager_order = authority["manager_order"]
    ordered_keys: list[tuple[str, str]] = []
    if not isinstance(manager_order, list) or len(manager_order) > A38_MAX_GUI_ROWS:
        problems.append("d84 manager order shape/bound")
    else:
        for position, item in enumerate(manager_order):
            if not isinstance(item, dict) or set(item) != {"manager_order_index", "country_tag", "gui_id"}:
                problems.append("d84 manager row shape")
                continue
            if (
                type(item["manager_order_index"]) is not int
                or not isinstance(item["country_tag"], str)
                or len(item["country_tag"]) != 3
                or not all(ch.isascii() and (ch.isupper() or ch.isdigit()) for ch in item["country_tag"])
                or not portable_a38_identity(item["gui_id"])
            ):
                problems.append("d84 manager row identity")
                continue
            key = (item["country_tag"], item["gui_id"])
            if item["manager_order_index"] != position or key in ordered_keys or key not in runtime or runtime[key]["manager_order_index"] != position:
                problems.append("d84 manager order/runtime join")
            ordered_keys.append(key)
        if set(ordered_keys) != set(runtime):
            problems.append("d84 manager order does not exactly cover runtime rows")
    tombstones = authority["tombstones"]
    previous: tuple[str, str] | None = None
    seen_tombstones: set[tuple[str, str]] = set()
    if not isinstance(tombstones, list) or len(tombstones) > A38_MAX_GUI_ROWS:
        problems.append("d84 tombstone shape/bound")
    else:
        for item in tombstones:
            if not isinstance(item, dict) or set(item) != {"country_tag", "gui_id"}:
                problems.append("d84 tombstone row shape")
                continue
            if not isinstance(item["country_tag"], str) or not isinstance(item["gui_id"], str):
                problems.append("d84 tombstone ordering/identity/disjointness")
                continue
            key = (item["country_tag"], item["gui_id"])
            if (
                len(key[0]) != 3
                or not all(ch.isascii() and (ch.isupper() or ch.isdigit()) for ch in key[0])
                or not portable_a38_identity(key[1])
                or key in runtime
                or key in seen_tombstones
                or (previous is not None and previous >= key)
            ):
                problems.append("d84 tombstone ordering/identity/disjointness")
            seen_tombstones.add(key)
            previous = key
    return tuple(problems)


def refresh_contract_bindings(contract: dict) -> None:
    if (
        contract.get("schema") not in {
            "hoi4.extractor.production_replay_handoff.v2",
            "hoi4.extractor.production_replay_handoff.v3",
        }
        or contract.get("status") != "PRODUCTION_ADMITTED"
        or contract.get("production_admission") is not True
    ):
        raise AssertionError("refusing to refresh a non-admitted or foreign handoff contract")
    upstream = contract.get("upstream_promotion_tuple", {})
    expected_upstream_paths = {
        "contract": "extractor/documents/production_promotion/contract.json",
        "verifier": "extractor/documents/production_promotion/verify_promotion.py",
        "verification": "extractor/documents/production_promotion/verification.json",
        "manifest": "extractor/documents/production_promotion/manifest.json",
    }
    if any(upstream.get(key) != value for key, value in expected_upstream_paths.items()):
        raise AssertionError("refusing to refresh an unexpected upstream promotion tuple")
    for key, relative in expected_upstream_paths.items():
        upstream[f"{key}_sha256"] = sha256(ROOT / relative)
    promotion_verification = load_json(upstream["verification"])
    passed = promotion_verification.get("checks_passed")
    if promotion_verification.get("status") != "PASS" or type(passed) is not int or passed <= 0:
        raise AssertionError("refusing to bind a non-passing upstream promotion result")
    upstream["verification_result"] = f"PASS {passed}/{passed}"

    pending = contract.get("implemented_pending_surfaces", [])
    if len(pending) != 1 or pending[0].get("packets") != ["A38", "XUI22"]:
        raise AssertionError("refusing to refresh an unexpected pending handoff set")
    for relative in pending[0]["frozen_sources"]:
        pending[0]["frozen_sources"][relative] = sha256(ROOT / relative)
    bounded = contract["bounded_integration"]
    for path_key, hash_key in (
        ("source_file", "source_sha256"),
        ("test_file", "test_sha256"),
        ("recorder_main", "recorder_main_sha256"),
    ):
        bounded[hash_key] = sha256(ROOT / bounded[path_key])
    bounded["domain_data_authority_sha256"] = sha256(
        ROOT / "extractor/native/recorder_host/src/domain_data_authority.rs"
    )
    contract["schema"] = "hoi4.extractor.production_replay_handoff.v3"
    contract["wave5_schema_extension"] = contract.pop(
        "wave4_schema_extension",
        "A38/XUI22/d84 implemented but not production-activated",
    )
    contract["central_rust_validation"] = central_binding(ROOT)
    contract["test_matrix"]["provenance"] = (
        "current Wave 5 source freeze; complete Rust validation is authenticated by the bound "
        "root-owned central Final/All attestation"
    )
    contract["admission_gate"].pop("wave4_central_rust_validation", None)
    contract["admission_gate"]["wave5_central_rust_validation"] = (
        "bound PASS Final/All attestation required before refreeze"
    )
    atomic_write_json(HERE / "handoff_contract.json", contract)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--refresh-bindings",
        action="store_true",
        help="guardedly refresh current source bindings and deterministic outputs",
    )
    mode.add_argument("--check", action="store_true", help="verify without writing (the default)")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    checks: list[dict[str, str]] = []
    contract_rel = "extractor/documents/production_replay_handoff/handoff_contract.json"
    schema_rel = "extractor/documents/production_replay_handoff/replay_payload.schema.json"
    verifier_rel = "extractor/documents/production_replay_handoff/verify_handoff.py"
    contract = load_json(contract_rel)
    if args.refresh_bindings:
        refresh_contract_bindings(contract)
        contract = load_json(contract_rel)
    schema = load_json(schema_rel)

    require(contract["schema"] == "hoi4.extractor.production_replay_handoff.v3", "handoff contract schema", checks)
    require(contract["status"] == "PRODUCTION_ADMITTED", "handoff status", checks)
    require(contract["production_admission"] is True, "production admission", checks)
    require(contract["no_new_binary_claims"] is True, "no new binary claims", checks)
    require(contract["bounded_integration"]["count"] == 0, "zero remaining bounded integration patches", checks)
    require(contract["bounded_integration"]["completed_packet"] == "A15", "completed bounded patch is A15", checks)
    require(contract["bounded_integration"]["collector_change"] is False, "collector change prohibited", checks)
    require(contract["bounded_integration"]["compact_schema_change"] is False, "compact schema change prohibited", checks)
    require(contract["bounded_integration"]["binary_research"] is False, "binary research prohibited", checks)
    central_hash = verify_central_attestation(
        ROOT, contract["central_rust_validation"], checks, require
    )
    pending = contract["implemented_pending_surfaces"]
    require(len(pending) == 1, "one Wave 5 externally gated handoff", checks)
    a38_pending = pending[0]
    require(a38_pending["packets"] == ["A38", "XUI22"] and a38_pending["domain_id"] == 84, "A38/XUI22 d84 handoff identity", checks)
    require(a38_pending["implementation"] == "COMPLETE_FAIL_CLOSED", "A38 implementation is fail closed", checks)
    require(a38_pending["production_activation"] is False, "A38 production activation remains false", checks)
    require(a38_pending["admission"] == "EXTERNAL_AUTHENTICATED_LIVE_FRAME_REQUIRED", "A38 authenticated live frame remains external gate", checks)
    require(len(a38_pending["required_external_gates"]) == 3, "A38 external gate list is exact", checks)

    upstream = contract["upstream_promotion_tuple"]
    expected_upstream = {
        key: upstream[f"{key}_sha256"]
        for key in ("contract", "verifier", "verification", "manifest")
    }
    frozen: dict[str, str] = {}
    for key, expected in expected_upstream.items():
        rel = upstream[key]
        require(canonical_sha256(expected), f"declared upstream {key} canonical hash", checks)
        actual = sha256(ROOT / rel)
        require(actual == expected, f"frozen upstream {key} hash", checks)
        frozen[rel] = actual

    promotion = load_json(upstream["contract"])
    promotion_verification = load_json(upstream["verification"])
    promotion_manifest = load_json(upstream["manifest"])
    require(promotion["status"] == "PROMOTION_READY", "upstream promotion ready", checks)
    require(promotion["scope_zero_residuals"] is True, "upstream batch zero residuals", checks)
    require(promotion["global_extractor_completion"] is False, "upstream not global completion", checks)
    require(promotion_verification["status"] == "PASS", "upstream verification pass", checks)
    upstream_passed = promotion_verification["checks_passed"]
    upstream_result = f"PASS {upstream_passed}/{upstream_passed}"
    require(
        upstream_passed == 121 and upstream.get("verification_result") == upstream_result,
        "upstream verification 121/121",
        checks,
    )
    require(promotion_manifest["status"] == "PASS", "upstream manifest pass", checks)
    for key in ("contract", "verifier", "verification"):
        require(promotion_manifest["files"][upstream[key]] == expected_upstream[key], f"upstream manifest binds {key}", checks)

    packet_ids = {item for packet in promotion["packets"] for item in packet["ids"]}
    require(packet_ids == {"A1", "A2", "A4", "A12", "A14", "A15", "A21", "A22", "B8", "X6"}, "exact upstream packet set", checks)
    require(contract["scope"] == "A1/A2/A4/A12/A14/A15/A21/A22/B8-X6 only", "handoff scope is exact", checks)

    surfaces = contract["surfaces"]
    require(len(surfaces) == 7, "seven replay surface groups", checks)
    surface_packets = {item for surface in surfaces for item in surface["packets"]}
    require(surface_packets == packet_ids, "all and only promoted packets handed off", checks)
    ready = [surface for surface in surfaces if surface["admission"] == "READY"]
    require(len(ready) == 7, "all seven replay surface groups ready", checks)
    a15 = next(surface for surface in surfaces if surface["packets"] == ["A15"])
    require(set(a15["current_projection"]) == {"players_by_country", "pause", "events.hotjoin", "events.tag_switch", "events.chat", "events.system_message"}, "A15 projection set is complete", checks)
    require(a15["missing_projection"] == [], "A15 has no missing projection", checks)

    require(schema["$schema"] == "https://json-schema.org/draft/2020-12/schema", "JSON Schema draft", checks)
    require(schema["$id"] == "hoi4.extractor.production_replay_handoff.payload.v2", "schema id", checks)
    require(schema["x-hoi4-cross-field-validators"] == ["d48_partial_health_aggregate", "a38_d84_authority_runtime_exact_join"], "schema declares enforcing cross-field validators", checks)
    top = schema["properties"]
    for key in ("countries", "states", "provinces", "strategic_air", "supply", "world_events", "_r2_session_multiplayer_metadata_live", "_r2_scripted_gui_runtime_live"):
        require(key in top, f"schema top-level surface {key}", checks)

    defs = schema["$defs"]
    channel_names = [
        "our_buildings_bombed", "our_troops_bombed", "our_ships_bombed", "our_fighters_lost",
        "our_bombers_lost", "our_cas_lost", "our_transports_lost", "our_bombers_disrupted",
        "our_trains_lost", "our_trucks_lost", "our_railway_guns_lost", "our_missiles_lost",
        "enemy_buildings_bombed", "enemy_troops_bombed", "enemy_ships_sunk", "enemy_fighters_lost",
        "enemy_bombers_lost", "enemy_cas_lost", "enemy_transports_lost", "enemy_bombers_disrupted",
        "enemy_trains_lost", "enemy_trucks_lost", "enemy_railway_guns_lost", "enemy_missiles_lost"
    ]
    require(defs["airChannelValues"]["required"] == channel_names, "ordered 24-channel value schema", checks)
    require(set(defs["airDisplayDivisors"]["required"]) == set(channel_names), "24-channel divisor schema", checks)
    divisors = defs["airDisplayDivisors"]["properties"]
    require(divisors["our_buildings_bombed"]["const"] == 1000 and divisors["enemy_buildings_bombed"]["const"] == 1000, "building display divisors", checks)
    require(all(divisors[name]["const"] == 100000 for name in channel_names if "buildings_bombed" not in name), "non-building display divisors", checks)
    require(defs["hubMotorizationRow"]["properties"]["motorization_level"]["enum"] == [1, 2], "motorization levels", checks)
    require(defs["hubMotorizationRow"]["properties"]["truck_fraction_raw"]["enum"] == [50000, 100000], "motorization truck fractions", checks)
    require(defs["airChannelRaw"]["additionalProperties"]["type"] == "integer", "raw air channels are integers", checks)
    require(defs["airChannelRaw"]["minProperties"] == 24 and defs["airChannelRaw"]["maxProperties"] == 24, "raw air channels are exactly 24", checks)
    require(len(defs["hubMotorizationRow"]["allOf"]) == 2, "motorization level/fraction pairing", checks)
    require(len(defs["knownRows"]["allOf"]) == 1, "unavailable event families are empty", checks)
    require(defs["buildingMap"]["patternProperties"].get("^type_0x[0-9a-f]{8}$") is not None, "numeric building key retained", checks)
    partial_schema = defs["buildingDetail"]["properties"]["partial_health_raw"]
    aggregate_rule = partial_schema.get("x-hoi4-maximum-product")
    require("maximum" not in partial_schema, "D48 aggregate has no incorrect per-object constant maximum", checks)
    require(
        aggregate_rule == {
            "property": "level",
            "multiplier": D48_PARTIAL_HEALTH_PER_LEVEL,
            "rule": "partial_health_raw <= level * 10000000",
        },
        "D48 level-dependent aggregate bound is declared exactly",
        checks,
    )
    d48_valid_multi_level = {
        "level": 2,
        "healthy_level": 0,
        "partial_health_raw": 10_000_001,
        "damaged": True,
    }
    d48_valid_ceiling = {
        "level": 2,
        "healthy_level": 0,
        "partial_health_raw": 20_000_000,
        "damaged": True,
    }
    d48_invalid_overflow = {
        "level": 2,
        "healthy_level": 0,
        "partial_health_raw": 20_000_001,
        "damaged": True,
    }
    require(
        validate_d48_aggregate_partial_health(d48_valid_multi_level) == (),
        "D48 multi-level aggregate above one-object bound is admitted",
        checks,
    )
    require(
        validate_d48_aggregate_partial_health(d48_valid_ceiling) == (),
        "D48 aggregate exact level-dependent ceiling is admitted",
        checks,
    )
    overflow_result = validate_d48_aggregate_partial_health(d48_invalid_overflow)
    require(
        overflow_result
        == ("partial_health_raw 20000001 exceeds level-dependent aggregate maximum 20000000",),
        "D48 aggregate above level-dependent ceiling is rejected",
        checks,
    )
    require(
        overflow_result == validate_d48_aggregate_partial_health(d48_invalid_overflow),
        "D48 aggregate cross-field validation is deterministic",
        checks,
    )
    require(set(defs["sessionEvent"]["properties"]["kind"]["enum"]) == {"hotjoin", "tag_switch", "chat", "system_message"}, "A15 event kinds", checks)
    require("events" in defs["sessionMetadata"]["required"], "target session schema requires events", checks)

    a38_authority_fields = [
        "schema_version", "authority", "elapsed_wall_ms", "frame_hour", "producer_record_identity",
        "executable_sha256", "ui14_engine_vfs_sha256", "static_authority_job_id",
        "static_authority_receipt_sha256", "producer_manifest_sha256", "manager_order", "tombstones",
    ]
    a38_input_fields = ["manager_order_index", "definition", "visible", "enabled", "triggers", "properties", "effect_enabled"]
    a38_definition_fields = ["logical_path", "winning_layer_ordinal", "layer_kind", "layer_id", "source_sha256", "payload_sha256", "payload_bytes", "context_type", "window_name"]
    require(defs["scriptedGuiRuntimeAuthority"]["required"] == a38_authority_fields, "A38 authority field enumeration", checks)
    require(defs["scriptedGuiRuntimeInput"]["required"] == a38_input_fields, "A38 runtime-input field enumeration", checks)
    require(defs["scriptedGuiDefinition"]["required"] == a38_definition_fields, "A38 definition field enumeration", checks)
    require(defs["scriptedGuiRuntimeAuthority"]["additionalProperties"] is False, "A38 authority denies unknown fields", checks)
    require(defs["scriptedGuiRuntimeInput"]["additionalProperties"] is False, "A38 runtime inputs deny unknown fields", checks)
    require(defs["scriptedGuiDefinition"]["additionalProperties"] is False, "A38 definitions deny unknown fields", checks)
    require(defs["scriptedGuiRuntimeAuthority"]["properties"]["executable_sha256"]["const"] == A38_EXECUTABLE_SHA256, "A38 exact executable identity", checks)
    require(defs["scriptedGuiRuntimeAuthority"]["properties"]["static_authority_job_id"]["const"] == A38_STATIC_AUTHORITY_JOB_ID, "A38 exact static-authority job", checks)
    require(defs["scriptedGuiRuntimeAuthority"]["properties"]["static_authority_receipt_sha256"]["const"] == A38_STATIC_AUTHORITY_RECEIPT_SHA256, "A38 exact static-authority receipt", checks)
    require(defs["scriptedGuiRuntimeAuthority"]["properties"]["manager_order"]["maxItems"] == A38_MAX_GUI_ROWS and defs["scriptedGuiRuntimeAuthority"]["properties"]["manager_order"]["uniqueItems"] is True, "A38 manager-order bound and structural uniqueness", checks)
    require(defs["scriptedGuiRuntimeAuthority"]["properties"]["tombstones"]["maxItems"] == A38_MAX_GUI_ROWS and defs["scriptedGuiRuntimeAuthority"]["properties"]["tombstones"]["uniqueItems"] is True, "A38 tombstone bound and structural uniqueness", checks)
    require(defs["scriptedGuiBoolResultMap"]["maxProperties"] == A38_MAX_RESULTS_PER_MAP, "A38 bool-result map bound", checks)
    require(defs["scriptedGuiStringResultMap"]["x-hoi4-maximum-flattened-properties"] == A38_MAX_RESULTS_PER_MAP, "A38 flattened property map bound declared", checks)

    valid_a38 = {
        "hour_index": 99,
        "countries": {
            "USA": {"extra": {"scripted_gui_runtime": {
                "usa_congress": {
                    "manager_order_index": 0,
                    "definition": {
                        "logical_path": "common/scripted_guis/USA_congress.txt",
                        "winning_layer_ordinal": 2,
                        "layer_kind": "mod",
                        "layer_id": "workshop:7",
                        "source_sha256": "2" * 64,
                        "payload_sha256": "3" * 64,
                        "payload_bytes": 44,
                        "context_type": "country",
                        "window_name": "congress_window",
                    },
                    "visible": {"value": True, "unavailable_reason": None},
                    "enabled": {"value": None, "unavailable_reason": "engine-only scope"},
                    "triggers": {"senate_visible": {"value": None, "unavailable_reason": "engine-only trigger"}},
                    "properties": {"icon": {"image": {"value": "GFX_senate", "unavailable_reason": None}}},
                    "effect_enabled": {"invest": {"value": False, "unavailable_reason": None}},
                }
            }}}
        },
        "_r2_scripted_gui_runtime_live": {
            "schema_version": 1,
            "authority": "exact20_a38_engine_materialized",
            "elapsed_wall_ms": 17,
            "frame_hour": 99,
            "producer_record_identity": "a38:99:frame-1",
            "executable_sha256": A38_EXECUTABLE_SHA256,
            "ui14_engine_vfs_sha256": "4" * 64,
            "static_authority_job_id": A38_STATIC_AUTHORITY_JOB_ID,
            "static_authority_receipt_sha256": A38_STATIC_AUTHORITY_RECEIPT_SHA256,
            "producer_manifest_sha256": "5" * 64,
            "manager_order": [{"manager_order_index": 0, "country_tag": "USA", "gui_id": "usa_congress"}],
            "tombstones": [{"country_tag": "GER", "gui_id": "former_panel"}],
        },
    }
    require(validate_a38_handoff_snapshot(valid_a38) == (), "A38 exact authority/runtime fixture admitted", checks)
    unavailable_a38 = {"hour_index": 99, "countries": {}, "_r2_scripted_gui_runtime_live": None}
    require(validate_a38_handoff_snapshot(unavailable_a38) == (), "A38 explicit unavailable with no rows admitted", checks)
    no_authority = json.loads(json.dumps(valid_a38))
    no_authority["_r2_scripted_gui_runtime_live"] = None
    require("d84 runtime rows exist without exact authority" in validate_a38_handoff_snapshot(no_authority), "A38 rows without authority rejected", checks)
    wrong_order = json.loads(json.dumps(valid_a38))
    wrong_order["_r2_scripted_gui_runtime_live"]["manager_order"][0]["manager_order_index"] = 1
    require("d84 manager order/runtime join" in validate_a38_handoff_snapshot(wrong_order), "A38 noncontiguous manager order rejected", checks)
    missing_cover = json.loads(json.dumps(valid_a38))
    missing_cover["_r2_scripted_gui_runtime_live"]["manager_order"] = []
    require("d84 manager order does not exactly cover runtime rows" in validate_a38_handoff_snapshot(missing_cover), "A38 incomplete authority cover rejected", checks)
    host_detail = json.loads(json.dumps(valid_a38))
    host_detail["_r2_scripted_gui_runtime_live"]["producer_record_identity"] = "C:\\capture\\frame.bin"
    require("d84 authority identity" in validate_a38_handoff_snapshot(host_detail), "A38 host-path identity rejected", checks)
    overlapping_tombstone = json.loads(json.dumps(valid_a38))
    overlapping_tombstone["_r2_scripted_gui_runtime_live"]["tombstones"] = [{"country_tag": "USA", "gui_id": "usa_congress"}]
    require("d84 tombstone ordering/identity/disjointness" in validate_a38_handoff_snapshot(overlapping_tombstone), "A38 tombstone/current-row overlap rejected", checks)

    world_fold_rel = "extractor/native/recorder_host/src/world_fold.rs"
    world_tests_rel = "extractor/native/recorder_host/src/world_fold/tests.rs"
    recorder_main_rel = "extractor/native/recorder_host/src/main.rs"
    authority_rel = "extractor/native/recorder_host/src/domain_data_authority.rs"
    world_fold_hash = sha256(ROOT / world_fold_rel)
    world_tests_hash = sha256(ROOT / world_tests_rel)
    recorder_main_hash = sha256(ROOT / recorder_main_rel)
    authority_hash = sha256(ROOT / authority_rel)
    require(world_fold_hash == contract["bounded_integration"]["source_sha256"], "frozen admitted world fold source", checks)
    require(world_tests_hash == contract["bounded_integration"]["test_sha256"], "frozen admitted world fold tests", checks)
    require(recorder_main_hash == contract["bounded_integration"]["recorder_main_sha256"], "frozen recorder main assertion", checks)
    require(authority_hash == contract["bounded_integration"]["domain_data_authority_sha256"], "frozen A4/A21 authority source", checks)
    frozen[world_fold_rel] = world_fold_hash
    frozen[world_tests_rel] = world_tests_hash
    frozen[recorder_main_rel] = recorder_main_hash
    frozen[authority_rel] = authority_hash

    a38_frozen_sources = a38_pending["frozen_sources"]
    expected_a38_paths = {
        "extractor/native/lanes/scripted_state_flags_variables_live/src/scripted_gui_runtime_carrier.rs",
        "extractor/native/lanes/scripted_state_flags_variables_live/src/scripted_gui_runtime_compact.rs",
        "extractor/native/collector/src/lib.rs",
        world_fold_rel,
        world_tests_rel,
        "replay_interface/native/replay_core/src/world.rs",
        "replay_interface/native/game_ui/src/scripted_gui_runtime.rs",
    }
    require(set(a38_frozen_sources) == expected_a38_paths, "A38 frozen producer/consumer source set is exact", checks)
    for rel, expected in sorted(a38_frozen_sources.items()):
        actual = sha256(ROOT / rel)
        require(actual == expected, f"A38 frozen source {rel}", checks)
        frozen[rel] = actual

    world_source = (ROOT / world_fold_rel).read_text(encoding="utf-8")
    d57_fold = function_body(world_source, "fn fold_session_multiplayer_metadata", "// --- DOMAIN 56")
    d57_projection = function_body(world_source, "fn build_domain57_session_sidecar", "// W6 D68")
    require("if kind != session57::SessionEventKind::Pause" not in d57_fold, "recorder no longer drops all non-pause D57 events", checks)
    require('"players_by_country": players_by_country' in d57_projection and '"pause": pause' in d57_projection, "current recorder projects D57 players and pause", checks)
    require('"events": events' in d57_projection, "current recorder projects typed A15 events", checks)
    for token in ("SessionEventKind::Hotjoin", "SessionEventKind::TagSwitch", "SessionEventKind::Chat", "SessionEventKind::SystemMessage", "SOURCE_FLAG_MESSAGE_HOOK_VALIDATED", "SOURCE_FLAG_COMMAND_HOOK_VALIDATED", "DOMAIN57_ROOT_CHAT_MESSAGES", "DOMAIN57_ROOT_PLAYER_EVENTS", "message_event_family_unavailable", "player_event_family_unavailable"):
        require(token in d57_fold, f"A15 fold token {token}", checks)
    for token in ("actor_key_hash", "actor_name", "related_value_i32", "command_sequence", "event_hour_raw", "event_source_id", "event_state"):
        require(token in world_source, f"A15 replay field {token}", checks)
    for token in ("_r2_air_region_combat_stats", "_r2_air_loss_stats_live", "_r2_hub_motorization_live", "sunk_convoys", "_r2_depth_targets", "target_location_id", "partial_health_raw"):
        require(token in world_source, f"existing replay projection {token}", checks)
    for token in (
        "fold_scripted_gui_runtime", "_r2_scripted_gui_runtime_live", '"scripted_gui_runtime"',
        '"exact20_a38_engine_materialized"', "EXACT20_A38_JOB_ID", "static_authority_receipt_sha256",
        "manager_order_index", "tombstones", "a38_portable_unavailable_reason",
    ):
        require(token in world_source, f"A38 recorder fold token {token}", checks)

    carrier_rel = "extractor/native/lanes/scripted_state_flags_variables_live/src/scripted_gui_runtime_carrier.rs"
    compact_rel = "extractor/native/lanes/scripted_state_flags_variables_live/src/scripted_gui_runtime_compact.rs"
    collector_rel = "extractor/native/collector/src/lib.rs"
    replay_world_rel = "replay_interface/native/replay_core/src/world.rs"
    replay_ui_rel = "replay_interface/native/game_ui/src/scripted_gui_runtime.rs"
    carrier_source = (ROOT / carrier_rel).read_text(encoding="utf-8")
    compact_source = (ROOT / compact_rel).read_text(encoding="utf-8")
    collector_source = (ROOT / collector_rel).read_text(encoding="utf-8")
    replay_world_source = (ROOT / replay_world_rel).read_text(encoding="utf-8")
    replay_ui_source = (ROOT / replay_ui_rel).read_text(encoding="utf-8")
    for token in (
        "EXACT_A729_EXECUTABLE_SHA256", "EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256",
        "authenticate_runtime_frame", "sanitize_runtime_diagnostics", "rows do not retain contiguous manager updater order",
        "runtime rows do not exactly cover authenticated effective GUI key set", "tombstone overlaps current effective GUI set",
        "property results do not exactly cover authenticated dynamic properties",
    ):
        require(token in carrier_source, f"A38 carrier token {token}", checks)
    for token in ("DOMAIN_ID: u16 = 84", 'u32::from_le_bytes(*b"SGR1")', "validate_envelope", "encode_source_unavailable", "decode_payload"):
        require(token in compact_source, f"A38 compact token {token}", checks)
    for token in ("AuthenticatedA38FrameSource", "collect_and_encode_scripted_gui_runtime", "authenticate_runtime_frame", "AuthenticatedA38CurrentManagerSnapshot"):
        require(token in collector_source, f"A38 collector token {token}", checks)
    for token in (
        "scripted_gui_runtime_authority", "unknown or missing authority fields", "manager order does not exactly cover folded d84 rows",
        "frame hour differs from the canonical replay index", "snapshot scripted-GUI runtime payload omitted its exact d84 authority envelope",
        "tombstone_keys", "A38_STATIC_AUTHORITY_RECEIPT_SHA256",
    ):
        require(token in replay_world_source, f"A38 replay typed-consumer token {token}", checks)
    for token in (
        "discover_effective_scripted_guis", "evaluate_registered_gui", "Truth::Unavailable",
        "captured_inputs_for", "apply_effect_preview", "unsupported_constructs",
    ):
        require(token in replay_ui_source, f"A38 generic UI-consumer token {token}", checks)

    recorder_main = (ROOT / recorder_main_rel).read_text(encoding="utf-8")
    authority = (ROOT / authority_rel).read_text(encoding="utf-8")
    require("COMPACT_AIR_STRENGTH_DELTA_HEADER_LEN: usize = 48" in authority, "frozen d42 header size 48", checks)
    require("COMPACT_AIR_STRENGTH_DELTA_ROW_LEN: usize = 76" in authority, "frozen d42 row size 76", checks)
    require("expected at least 124" in recorder_main and "116 byte(s)" in recorder_main, "d42 assertion matches actual diagnostic", checks)
    test_source = (ROOT / world_tests_rel).read_text(encoding="utf-8")
    for token in ("chunk2_d57_a15_replay_json_matches_handoff_schema", "chunk2_d57_message_family_mismatch_keeps_player_events_players_and_pause", "chunk2_d57_player_event_family_mismatch_keeps_messages_players_and_pause", "chunk2_d57_unknown_event_kind_fails_closed"):
        require(token in test_source, f"A15 focused test {token}", checks)

    for rel in (contract_rel, schema_rel, verifier_rel):
        frozen[rel] = sha256(ROOT / rel)
    frozen[contract["central_rust_validation"]["path"]] = central_hash
    frozen[HELPER_REL] = sha256(ROOT / HELPER_REL)
    verification = {
        "schema": "hoi4.extractor.production_replay_handoff.verification.v3",
        "status": "PASS",
        "production_admission": True,
        "reason": "all seven promoted replay surface groups pass static handoff admission; A38/d84 is schema-wired fail-closed and remains externally gated from production activation; complete Rust validation is authenticated by the bound repository central Final/All attestation",
        "rust_validation": contract["central_rust_validation"],
        "upstream": upstream_result,
        "checks_passed": len(checks),
        "checks": checks,
        "frozen_inputs": dict(sorted(frozen.items()))
    }
    verification_path = HERE / "verification.json"
    write_or_check_json(verification_path, verification, refresh=args.refresh_bindings)
    manifest_files = dict(frozen)
    manifest_files["extractor/documents/production_replay_handoff/verification.json"] = sha256(verification_path)
    manifest = {
        "schema": "hoi4.extractor.production_replay_handoff.manifest.v3",
        "status": "PASS",
        "production_admission": True,
        "upstream": upstream_result,
        "files": dict(sorted(manifest_files.items()))
    }
    write_or_check_json(HERE / "manifest.json", manifest, refresh=args.refresh_bindings)
    print(f"PASS {len(checks)}/{len(checks)}; production_admission=true")


if __name__ == "__main__":
    main()
