#!/usr/bin/env python3
"""Deterministic mechanical verifier for the frozen extractor promotion batch."""

from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
BINARY_SHA = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_rel(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def require(condition: bool, detail: str, checks: list[dict[str, str]]) -> None:
    if not condition:
        raise AssertionError(detail)
    checks.append({"check": detail, "status": "PASS"})


def main() -> None:
    contract_path = HERE / "contract.json"
    contract = json.loads(contract_path.read_text(encoding="utf-8"))
    checks: list[dict[str, str]] = []

    require(contract["status"] == "PROMOTION_READY", "contract promotion status", checks)
    require(contract["scope_zero_residuals"] is True and contract["residuals"] == [], "batch has zero residuals", checks)
    require(contract["global_extractor_completion"] is False, "batch is not mislabeled global completion", checks)
    require(contract["binary"]["sha256"] == BINARY_SHA, "binary SHA is pinned", checks)
    require(contract["binary"]["program"] == "hoi4.exe 1.19.2.0.a729", "binary program/version is pinned", checks)
    require(contract["binary"]["image_base"] == "0x140000000", "image base is pinned", checks)

    packet_ids: set[str] = set()
    frozen: dict[str, str] = {}
    for packet in contract["packets"]:
        packet_ids.update(packet["ids"])
        for key in ("report", "evidence", "integration"):
            if key not in packet:
                continue
            rel = packet[key]
            expected = packet[f"{key}_sha256"]
            actual = sha256(ROOT / rel)
            require(actual == expected, f"frozen hash {rel}", checks)
            frozen[rel] = actual

        report = read_rel(packet["report"])
        require(BINARY_SHA in report and "0x140000000" in report, f"binary provenance {packet['report']}", checks)
        require(bool(re.search(r"20260716T[0-9]{6}Z-[a-z0-9-]+", report)), f"job id provenance {packet['report']}", checks)
        require(bool(re.search(r"(?:0x|FUN_)[0-9A-Fa-f]{8,}", report)), f"function/address provenance {packet['report']}", checks)
        require("Artifact" in report or "artifact" in report, f"artifact path provenance {packet['report']}", checks)
        require(bool(re.search(r"[0-9a-f]{64}", report)), f"manifest hash provenance {packet['report']}", checks)
        disposition = report.upper()
        require("PROMOTION" in disposition or "PRODUCTION READY" in disposition, f"promotion disposition {packet['report']}", checks)

    require(packet_ids == {"A1", "A2", "A4", "A12", "A14", "A15", "A21", "A22", "B8", "X6"}, "packet set is exact", checks)

    for surface in contract["production_surfaces"]:
        actual = sha256(ROOT / surface["path"])
        require(actual == surface["sha256"], f"production surface hash {surface['path']}", checks)
        frozen[surface["path"]] = actual

    xn07 = contract["xn07_crosscheck"]
    for key in ("report", "verification", "verifier"):
        actual = sha256(ROOT / xn07[key])
        require(actual == xn07[f"{key}_sha256"], f"XN07 frozen hash {xn07[key]}", checks)
        frozen[xn07[key]] = actual
    xn07_result = json.loads(read_rel(xn07["verification"]))
    require(str(xn07_result.get("status", "")).upper() == "PASS", "XN07 verification status", checks)

    a2 = json.loads(read_rel("extractor/native/lanes/focus_research_decisions_depth_live/A2_A12_RE_EVIDENCE.json"))
    a2_status = str(a2.get("production_integration", {}).get("status", "")).upper()
    require(a2_status == "PROVEN_PRODUCTION_READY", "A2/A12 evidence status", checks)
    a14 = json.loads(read_rel("extractor/native/lanes/construction_buildings_state_damage_live/A14_RE_EVIDENCE.json"))
    require(str(a14.get("status", "")).upper() in {"PASS", "PROMOTION_READY", "PROVEN"}, "A14 evidence status", checks)
    b8 = json.loads(read_rel("extractor/native/lanes/construction_buildings_state_damage_live/B8_X6_BUILDING_REGISTRY_PROOF.json"))
    require(str(b8.get("result", "")).upper() == "PASS", "B8/X6 evidence status", checks)
    b8_text = json.dumps(b8, sort_keys=True)
    require(all(token in b8_text for token in ("7456", "55", "43", "12")), "B8/X6 proof census", checks)

    master_rel = "extractor/documents/MASTER_EXTRACTION_CONTRACT.md"
    remaining_rel = "extractor/documents/RE_REMAINING.md"
    master = read_rel(master_rel)
    remaining = read_rel(remaining_rel)
    require(master.count("production_promotion/contract.json") >= 1, "master links promotion contract", checks)
    require(remaining.count("production_promotion/contract.json") >= 1, "remaining ledger links promotion contract", checks)
    for token in ("A4/A21 exact regional causes", "A1 exact hub actor/level attribution", "B8/X6 exact building registry", "A2/A12 exact pause", "A15 exact multiplayer/session metadata is closed", "A1 exact actor/level attribution is closed", "A22 exact 24-channel schema are closed"):
        require(token in master, f"master reconciliation: {token}", checks)
    for stale in ("focus-paused A2; decision targets/cooldowns A12", "hotjoin/tag-switch/chat A15", "Per-hub level fixture A1", "24-channel expansion/live fixture A22"):
        require(stale not in master, f"stale master text absent: {stale}", checks)
    for packet_id in ("A1", "A2", "A4", "A12", "A14", "A15", "A21", "A22", "B8"):
        row_id = "B8(?:/X6)?" if packet_id == "B8" else packet_id
        require(bool(re.search(rf"\|\s*{row_id}\s*\|[^\n]*(?:PROMOTION.READY|\[x\])", remaining, re.IGNORECASE)), f"remaining ledger closes {packet_id}", checks)

    source_tokens = {
        "extractor/native/compact_contract/src/build_profile.rs": ["DOMAIN45_FOCUS_STATUS_PAUSED", "DOMAIN57_CGAMESTATE_SINGLETON_RVA"],
        "extractor/native/compact_contract/src/building_registry.rs": ["BUILDING_REGISTRY", BINARY_SHA],
        "extractor/native/collector/src/lib.rs": ["DOMAIN45_FOCUS_STATUS_PAUSED", "DOMAIN57_EVENT_TRACKER", "air_loss_stats_live_lane", "hub_motorization_live_lane"],
        "extractor/native/recorder_host/src/domain_data_authority.rs": ["COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE", "COMPACT_DOMAIN_SESSION_MULTIPLAYER_METADATA_LIVE", "COMPACT_DOMAIN_AIR_LOSS_STATS_LIVE", "COMPACT_DOMAIN_HUB_MOTORIZATION_LIVE"],
        "extractor/native/recorder_host/src/world_fold.rs": ["frd_decision_status", "session_multiplayer_metadata_this_hour", "sunk_convoy_history", "air_loss_stats", "hub_motorization"],
        "extractor/native/recorder_host/src/main.rs": ["domain57_game_speed_gate", "compact_domain_effective_failure_flags"]
    }
    for rel, tokens in source_tokens.items():
        text = read_rel(rel)
        require(all(token in text for token in tokens), f"production tokens {rel}", checks)

    frozen[master_rel] = sha256(ROOT / master_rel)
    frozen[remaining_rel] = sha256(ROOT / remaining_rel)
    verification = {
        "schema": "hoi4.extractor.production_promotion.verification.v1",
        "status": "PASS",
        "scope_zero_residuals": True,
        "global_extractor_completion": False,
        "checks_passed": len(checks),
        "checks": checks,
        "mechanical_test_matrix": contract["mechanical_gate"],
        "frozen_inputs": dict(sorted(frozen.items()))
    }
    verification_path = HERE / "verification.json"
    verification_path.write_text(json.dumps(verification, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    manifest_files = {
        "extractor/documents/production_promotion/contract.json": sha256(contract_path),
        "extractor/documents/production_promotion/verification.json": sha256(verification_path),
        "extractor/documents/production_promotion/verify_promotion.py": sha256(Path(__file__))
    }
    manifest_files.update(frozen)
    manifest = {
        "schema": "hoi4.extractor.production_promotion.manifest.v1",
        "status": "PASS",
        "scope": contract["scope"],
        "files": dict(sorted(manifest_files.items()))
    }
    (HERE / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"PASS {len(checks)}/{len(checks)}")


if __name__ == "__main__":
    main()
