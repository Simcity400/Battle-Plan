#!/usr/bin/env python3
"""Deterministic verifier for the EXTRACTOR-P replay handoff package."""

from __future__ import annotations

import hashlib
import json
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


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


def run_gate(args: list[str], cwd: Path, name: str, checks: list[dict[str, str]]) -> None:
    result = subprocess.run(args, cwd=cwd, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        tail = (result.stdout + "\n" + result.stderr)[-4000:]
        raise AssertionError(f"{name} failed ({result.returncode}):\n{tail}")
    checks.append({"check": name, "status": "PASS"})


def main() -> None:
    checks: list[dict[str, str]] = []
    contract_rel = "extractor/documents/production_replay_handoff/handoff_contract.json"
    schema_rel = "extractor/documents/production_replay_handoff/replay_payload.schema.json"
    plan_rel = "extractor/documents/production_replay_handoff/PATCH_PLAN.md"
    verifier_rel = "extractor/documents/production_replay_handoff/verify_handoff.py"
    contract = load_json(contract_rel)
    schema = load_json(schema_rel)

    require(contract["status"] == "PRODUCTION_ADMITTED", "handoff status", checks)
    require(contract["production_admission"] is True, "production admission", checks)
    require(contract["no_new_binary_claims"] is True, "no new binary claims", checks)
    require(contract["bounded_integration"]["count"] == 0, "zero remaining bounded integration patches", checks)
    require(contract["bounded_integration"]["completed_packet"] == "A15", "completed bounded patch is A15", checks)
    require(contract["bounded_integration"]["collector_change"] is False, "collector change prohibited", checks)
    require(contract["bounded_integration"]["compact_schema_change"] is False, "compact schema change prohibited", checks)
    require(contract["bounded_integration"]["binary_research"] is False, "binary research prohibited", checks)

    upstream = contract["upstream_promotion_tuple"]
    expected_upstream = {
        "contract": "50286e9afead2d64766fca50d6c7502e82ab8ecf678b4569a23903d90eb777a9",
        "verifier": "ce941484b5180183917623f10450c2cde4c9af0462540e08ec924629b40e03a3",
        "verification": "c17b7d3ae5f8071a63ea03c7e6b402e0d98f96a6cc9647f14b17ebf79895059b",
        "manifest": "9cd6b83b70902fdd212f2370ab798938aabd268d1bfb0ee8b39ebefd797b43ca"
    }
    frozen: dict[str, str] = {}
    for key, expected in expected_upstream.items():
        rel = upstream[key]
        require(upstream[f"{key}_sha256"] == expected, f"declared upstream {key} hash", checks)
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
    require(promotion_verification["checks_passed"] == 106, "upstream verification 106/106", checks)
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
    require(schema["$id"] == "hoi4.extractor.production_replay_handoff.payload.v1", "schema id", checks)
    top = schema["properties"]
    for key in ("countries", "states", "provinces", "strategic_air", "supply", "world_events", "_r2_session_multiplayer_metadata_live"):
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
    require(set(defs["sessionEvent"]["properties"]["kind"]["enum"]) == {"hotjoin", "tag_switch", "chat", "system_message"}, "A15 event kinds", checks)
    require("events" in defs["sessionMetadata"]["required"], "target session schema requires events", checks)

    world_fold_rel = "extractor/native/recorder_host/src/world_fold.rs"
    world_tests_rel = "extractor/native/recorder_host/src/world_fold/tests.rs"
    recorder_main_rel = "extractor/native/recorder_host/src/main.rs"
    authority_rel = "extractor/native/recorder_host/src/domain_data_authority.rs"
    world_fold_hash = sha256(ROOT / world_fold_rel)
    world_tests_hash = sha256(ROOT / world_tests_rel)
    recorder_main_hash = sha256(ROOT / recorder_main_rel)
    authority_hash = sha256(ROOT / authority_rel)
    require(world_fold_hash == contract["bounded_integration"]["source_sha256"] == "d0e42d67c678a38ba6e214af0e542287730812fbd553c2bdc9a1b09d28bb6cab", "frozen admitted world fold source", checks)
    require(world_tests_hash == contract["bounded_integration"]["test_sha256"] == "55ba4f4d8ea813909f8e903f33cb4edf043e0d3ea960c2694e03ce82d93af07a", "frozen admitted world fold tests", checks)
    require(recorder_main_hash == contract["bounded_integration"]["recorder_main_sha256"] == "aa35d560d22ef409e5e91e04de811d09d12655861402933b42b012f669ed5a81", "frozen recorder main assertion", checks)
    require(authority_hash == "1bcc03a41790b4c77640f3f5c4ebe08b29fb61f90aed90367b6527bd2ac68ad1", "frozen A4/A21 authority source", checks)
    frozen[world_fold_rel] = world_fold_hash
    frozen[world_tests_rel] = world_tests_hash
    frozen[recorder_main_rel] = recorder_main_hash
    frozen[authority_rel] = authority_hash

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

    plan = (ROOT / plan_rel).read_text(encoding="utf-8")
    for token in ("world_fold.rs", "world_fold/tests.rs", "SOURCE_FLAG_MESSAGE_HOOK_VALIDATED", "SOURCE_FLAG_COMMAND_HOOK_VALIDATED", "1 << 3", "1 << 4", "hotjoin", "tag_switch", "chat", "system_message", "No compact payload, collector"):
        require(token in plan, f"bounded patch-plan token {token}", checks)
    require("PROGRAM_STATUS" in plan, "patch plan excludes PROGRAM_STATUS", checks)
    require("COMPLETED / PRODUCTION ADMITTED" in plan, "patch plan completion status", checks)
    require("offset 0: air combat losses live data payload is 116 byte(s), expected at least 124" in plan, "actual d42 diagnostic recorded", checks)
    recorder_main = (ROOT / recorder_main_rel).read_text(encoding="utf-8")
    authority = (ROOT / authority_rel).read_text(encoding="utf-8")
    require("COMPACT_AIR_STRENGTH_DELTA_HEADER_LEN: usize = 48" in authority, "frozen d42 header size 48", checks)
    require("COMPACT_AIR_STRENGTH_DELTA_ROW_LEN: usize = 76" in authority, "frozen d42 row size 76", checks)
    require("expected at least 124" in recorder_main and "116 byte(s)" in recorder_main, "d42 assertion matches actual diagnostic", checks)
    test_source = (ROOT / world_tests_rel).read_text(encoding="utf-8")
    for token in ("chunk2_d57_a15_replay_json_matches_handoff_schema", "chunk2_d57_message_family_mismatch_keeps_player_events_players_and_pause", "chunk2_d57_player_event_family_mismatch_keeps_messages_players_and_pause", "chunk2_d57_unknown_event_kind_fails_closed"):
        require(token in test_source, f"A15 focused test {token}", checks)

    native = ROOT / "extractor/native"
    run_gate(["cargo", "fmt", "-p", "recorder_host", "--", "--check"], native, "cargo fmt recorder_host", checks)
    run_gate(["cargo", "check", "-p", "recorder_host"], native, "cargo check recorder_host", checks)
    run_gate(["cargo", "test", "-p", "recorder_host", "--lib", "chunk2_d57"], native, "focused A15 recorder tests 6/6", checks)
    run_gate(["cargo", "test", "-p", "recorder_host", "--bin", "collector_record", "tests::compact_summary_rejects_stale_air_combat_losses_live_68_byte_row_shape", "--", "--exact"], native, "focused d42 diagnostic test 1/1", checks)
    run_gate(["cargo", "test", "-p", "recorder_host"], native, "full recorder_host suite", checks)

    for rel in (contract_rel, schema_rel, plan_rel, verifier_rel):
        frozen[rel] = sha256(ROOT / rel)
    verification = {
        "schema": "hoi4.extractor.production_replay_handoff.verification.v1",
        "status": "PASS",
        "production_admission": True,
        "reason": "all seven promoted replay surface groups and the full recorder matrix pass",
        "upstream": "PASS 106/106",
        "checks_passed": len(checks),
        "checks": checks,
        "frozen_inputs": dict(sorted(frozen.items()))
    }
    verification_path = HERE / "verification.json"
    verification_path.write_text(json.dumps(verification, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    manifest_files = dict(frozen)
    manifest_files["extractor/documents/production_replay_handoff/verification.json"] = sha256(verification_path)
    manifest = {
        "schema": "hoi4.extractor.production_replay_handoff.manifest.v1",
        "status": "PASS",
        "production_admission": True,
        "upstream": "PASS 106/106",
        "files": dict(sorted(manifest_files.items()))
    }
    (HERE / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"PASS {len(checks)}/{len(checks)}; production_admission=true")


if __name__ == "__main__":
    main()
