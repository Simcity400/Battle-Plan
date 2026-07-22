#!/usr/bin/env python3
"""Fail-closed admission for the complete autonomous A30 acquisition."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


EXE_SHA = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
SAVE_SHA = "c8488b05a3b87c718eeff979976537b79e867634d0b4dccb1eef6c5a45362735"
FRAME_HOUR = 60830441
OWNER_ID = 368
SURFACES = {"agency", "upgrades", "operatives", "operations", "ciphers", "intel_ledger", "networks"}
REQUIRED_ACTIONS = {
    "agency_cipher_selected",
    "agency_operations_reselected",
    "intel_ledger_info_tab",
    "agency_final_present",
}
AUTHORITY = {
    "ghidra_manifest_sha256": "2ae11e05831234e108f53f2f5ad9eba359196c22cdb15e595a02746b69455096",
    "row_bind_manifest_sha256": "ad26c3ca3531041e43ba98c52ca3d42f6ab70120438ebb3b9bbe7b495af49b8e",
    "trait_nationality_manifest_sha256": "c67cadc86e5ee501f11ab873c58409cac450118f9213eccf916284b48b0f3996",
    "trait_identity_manifest_sha256": "1cbd56c8f9ed89c2fc9c0c079d58b0728a87aca7899e3615f6927bc51c47fc8a",
    "action_producer_manifest_sha256": "b7d38da93555d033bae897991e545b1d1b0ed9bb28cace96a133da2bf41344e8",
    "mission_implementation_manifest_sha256": "0f44c560ba1d1761768ee2d95aa519d8e67ed788afdfea5357b597772c323d0d",
    "assignment_consumer_manifest_sha256": "fb6a2e920abb641a5fcd0195a70bd6b9ad0806c0e081c5508aa2acc718bcc589",
    "boost_ideology_manifest_sha256": "8abae8f7075aa4b59d68bb8e20d998c4e61b2b3b2bc05be5d0e80d7309911ab9",
}
CAPTURE_AUTHORITY = {
    "capture_producer_manifest_sha256": "e33a74c6e0fb507b62cf38f5a0817263e24ad97d1cc75701870a7454bd33f162",
    "recruitment_vtable_artifact_sha256": "7402b9e321881fec939ae85489d9193b5727c28ec1b16943bb3ddd175aa4521c",
    "recruitment_serializer_manifest_sha256": "a7a0073f50149772c678512dfca3fa9de0a86f8f36889949bc9f3af8bf7f80c0",
    "recruitment_item_codec_manifest_sha256": "1e12452c61956000ea3af1b71bf7d095da1d6fb811b0e8d54a113e426803b272",
    "recruitment_owner_path_manifest_sha256": "d8bfc5aa0347c0eac443328566d572203a91fb2f44db94c8479e1d4899a4e229",
    "recruitable_vector_source_manifest_sha256": "07f911d109aead3e6c139fb37951ef535b789ddbb4ac34c03970e25c7a8b888a",
}


class GateError(RuntimeError):
    pass


def require(value: bool, message: str) -> None:
    if not value:
        raise GateError(message)


def load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise GateError(f"invalid JSON {path.name}: {exc}") from exc


def file_sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def canonical_sha(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()


def is_sha(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def validate_action(action: dict[str, Any], profile_sha: str) -> dict[str, Any]:
    require(action.get("schema") == "hoi4.a30.autonomous-action-receipt.v1", "action schema mismatch")
    require(action.get("status") == "actions_complete_selected_present_receipts_published", "actions incomplete")
    require(action.get("executable_sha256") == EXE_SHA, "action executable mismatch")
    require(action.get("profile_sha256") == profile_sha, "action profile hash mismatch")
    require(action.get("client_size") == [1920, 1080], "action client size mismatch")
    require(action.get("promotion_authorized") is False, "action receipt may not self-authorize promotion")
    initial, final = action.get("initial_identity"), action.get("final_identity")
    for name, identity in (("initial", initial), ("final", final)):
        require(isinstance(identity, dict), f"{name} identity absent")
        require(identity.get("game_hour_raw") == FRAME_HOUR, f"{name} hour mismatch")
        require(identity.get("owner_tag_db_id") == OWNER_ID, f"{name} owner mismatch")
        require(identity.get("pause_byte") == 1, f"{name} pause gate mismatch")
    steps = action.get("steps")
    require(isinstance(steps, list) and steps, "action steps absent")
    labels = {step.get("label") for step in steps if isinstance(step, dict)}
    require(REQUIRED_ACTIONS <= labels, "required A30 action labels absent")
    selected_hashes: list[str] = []
    selected_actions: dict[str, dict[str, Any]] = {}
    for step in steps:
        require(isinstance(step, dict), "action step is not an object")
        require(isinstance(step.get("qpc_before"), int) and isinstance(step.get("qpc_after"), int), "action QPC bracket absent")
        require(step["qpc_before"] <= step["qpc_after"], "action QPC bracket reversed")
        if step.get("label") in REQUIRED_ACTIONS:
            selected = step.get("selected_present")
            require(isinstance(selected, dict) and is_sha(selected.get("sha256")), "required action lacks selected-Present receipt")
            digest = selected["sha256"]
            require(selected.get("file_name") == f"{digest}.json", "action receipt does not name content-addressed selected Present")
            selected_hashes.append(digest)
            selected_actions[digest] = {
                "label": step["label"],
                "qpc_before": step["qpc_before"],
                "qpc_after": step["qpc_after"],
            }
    require(len(selected_hashes) == len(REQUIRED_ACTIONS) and len(set(selected_hashes)) == len(selected_hashes), "selected-Present action receipts are missing or reused")
    return {"selected_present_sha256": sorted(selected_hashes), "selected_present_actions": selected_actions, "step_count": len(steps)}


def gate_map(receipt: dict[str, Any]) -> dict[str, str]:
    visibility = receipt.get("visibility_join")
    require(isinstance(visibility, dict), "visibility join absent")
    require(visibility.get("schema") == "hoi4.a30.intelligence_visibility.v1", "visibility schema mismatch")
    require(visibility.get("executable_sha256") == EXE_SHA, "visibility executable mismatch")
    gates = visibility.get("gates")
    require(isinstance(gates, list) and len(gates) == 7, "visibility gate count mismatch")
    result: dict[str, str] = {}
    for gate in gates:
        require(isinstance(gate, dict), "visibility gate invalid")
        surface, disposition, reason = gate.get("surface"), gate.get("disposition"), gate.get("reason")
        require(surface in SURFACES and surface not in result, "visibility surface missing or duplicate")
        require(disposition in {"visible", "hidden", "unavailable"}, "visibility disposition invalid")
        require((disposition == "visible" and reason is None) or (disposition != "visible" and isinstance(reason, str) and reason), "visibility reason invariant failed")
        result[surface] = disposition
    require(set(result) == SURFACES, "seven-surface visibility set mismatch")
    return result


def validate_companion(receipt: dict[str, Any]) -> dict[str, Any]:
    require(receipt.get("schema") == "hoi4.a30.selected_present_companion.v4", "companion schema mismatch")
    require(receipt.get("executable_sha256") == EXE_SHA, "companion executable mismatch")
    require(receipt.get("frame_hour") == FRAME_HOUR and receipt.get("owner_tag_db_id") == OWNER_ID, "companion D04 identity mismatch")
    require(is_sha(receipt.get("producer_record_identity")), "companion producer identity invalid")
    require(receipt.get("production_scheduling_enabled") is False and receipt.get("dropped_callbacks") == 0, "companion evidence-only/drop invariant failed")
    for key, expected in AUTHORITY.items():
        require(receipt.get(key) == expected, f"companion authority mismatch: {key}")
    selected = receipt.get("selected_present")
    require(isinstance(selected, dict) and selected.get("selected_game_hour_raw") == FRAME_HOUR, "selected Present hour mismatch")
    require(is_sha(selected.get("selected_present_receipt_sha256")), "selected Present hash invalid")
    require(isinstance(selected.get("selected_qpc_ticks"), int) and selected.get("selected_qpc_ticks", 0) > 0, "selected Present QPC missing")
    require(selected.get("selection_cutoff_qpc_ticks", 0) >= selected["selected_qpc_ticks"], "selected Present cutoff reversed")
    operative_order, operation_order, rows = receipt.get("operative_order"), receipt.get("operation_order"), receipt.get("operative_raw_rows")
    require(isinstance(operative_order, list) and operative_order and len(set(operative_order)) == len(operative_order), "operative controller order invalid")
    require(isinstance(operation_order, list) and len(set(operation_order)) == len(operation_order), "operation controller order invalid")
    require(isinstance(rows, list) and [row.get("operative_id") for row in rows] == operative_order, "operative semantic rows do not preserve controller order")
    traits = nationalities = missions = 0
    for row in rows:
        require(isinstance(row.get("trait_definition_ids_utf8_at_0xdc8"), list), "trait identity vector absent")
        require(isinstance(row.get("nationality_tag_db_ids_at_0xf68"), list), "nationality identity vector absent")
        require(isinstance(row.get("portrait_gfx_utf8_at_0x100"), str) and row["portrait_gfx_utf8_at_0x100"], "authored portrait identity absent")
        traits += len(row["trait_definition_ids_utf8_at_0xdc8"])
        nationalities += len(row["nationality_tag_db_ids_at_0xf68"])
        if row.get("mission_product_identity") is not None:
            mission = row["mission_product_identity"]
            require(row.get("assignment_enum_raw_at_0x1080") == 3 and isinstance(mission, dict), "mission product is not assignment 3")
            require(isinstance(mission.get("authored_mission_localization_key"), str) and mission["authored_mission_localization_key"], "mission authored key absent")
            require(isinstance(mission.get("authored_action_localization_key"), str) and mission["authored_action_localization_key"], "mission action key absent")
            missions += 1
    portraits = receipt.get("operative_portrait_join", {}).get("portraits")
    require(isinstance(portraits, list) and [row.get("operative_id") for row in portraits] == operative_order, "final portrait join does not preserve operative order")
    require(all(isinstance(row.get("identity"), str) and row["identity"] for row in portraits), "final portrait identity absent")
    capture = receipt.get("capture_recruitable_join")
    require(isinstance(capture, dict) and capture.get("schema") == "hoi4.a30.capture_recruitable_join.v1", "capture/recruitable join absent")
    require(capture.get("frame_hour") == FRAME_HOUR and capture.get("owner_tag_db_id") == OWNER_ID and capture.get("producer_record_identity") == receipt["producer_record_identity"], "capture/recruitable authority is torn")
    for key, expected in CAPTURE_AUTHORITY.items():
        require(capture.get(key) == expected, f"capture/recruitable authority mismatch: {key}")
    capture_rows = capture.get("rows")
    require(isinstance(capture_rows, list) and len(capture_rows) >= len(operative_order), "capture/recruitable rows incomplete")
    for row in capture_rows:
        require(isinstance(row.get("captured"), bool) and isinstance(row.get("recruitable_pool_member"), bool), "capture/recruitable semantic booleans absent")
        if row["captured"]:
            require(isinstance(row.get("captured_by_tag_db_id"), int) and row["captured_by_tag_db_id"] > 0 and isinstance(row.get("capture_end_date_raw"), int), "captured row is half populated")
        else:
            require(row.get("captured_by_tag_db_id") is None and row.get("capture_end_date_raw") is None, "uncaptured row has capture payload")
    return {
        "selected_qpc_ticks": selected["selected_qpc_ticks"],
        "selected_present_receipt_sha256": selected["selected_present_receipt_sha256"],
        "visibility": gate_map(receipt),
        "traits": traits,
        "nationalities": nationalities,
        "missions": missions,
        "captured": sum(row["captured"] for row in capture_rows),
        "recruitable": sum(row["recruitable_pool_member"] for row in capture_rows),
        "operative_rows": len(rows),
        "operation_rows": len(operation_order),
    }


def validate_controller(receipt: dict[str, Any]) -> dict[str, Any]:
    require(receipt.get("schema") == "hoi4.a30.controller_evidence.v1", "controller evidence schema mismatch")
    require(receipt.get("executable_sha256") == EXE_SHA, "controller evidence executable mismatch")
    require(receipt.get("evidence_install") is True and receipt.get("domain53_multiplex_ready") is True, "controller evidence not multiplex-ready")
    require(receipt.get("production_scheduling_enabled") is False, "controller evidence unexpectedly production scheduled")
    diagnostics = receipt.get("diagnostics")
    require(isinstance(diagnostics, dict) and diagnostics.get("completeness_poisoned") is False, "controller evidence completeness poisoned")
    for key in ("dropped_reentrant", "dropped_unreadable", "dropped_lock_busy", "dropped_overflow", "dropped_unconsumed_arm"):
        require(diagnostics.get(key) == 0, f"controller evidence {key} is nonzero")
    records = receipt.get("records")
    require(isinstance(records, list) and records, "controller evidence records absent")
    families = {record.get("family") for record in records if isinstance(record, dict)}
    require({"AgencyUpgrade", "CipherCountry"} <= families, "agency or cipher controller family absent")
    require(all(record.get("frame_hour") == FRAME_HOUR and is_sha(record.get("payload_sha256")) for record in records), "controller record hour/hash mismatch")
    return {"record_count": len(records), "families": sorted(families)}


def write_create_new(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    fd = os.open(path, flags, 0o644)
    try:
        with os.fdopen(fd, "wb", closefd=False) as target:
            target.write(payload)
            target.flush()
            os.fsync(target.fileno())
    finally:
        os.close(fd)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--capture-root", type=Path, required=True)
    parser.add_argument("--action-receipt", type=Path, required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--controller-receipt", type=Path, required=True)
    parser.add_argument("--session-manifest", type=Path, required=True)
    parser.add_argument("--save", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        require(file_sha(args.save) == SAVE_SHA, "frozen D04 save hash mismatch")
        profile_sha = file_sha(args.profile)
        action_path = args.action_receipt.resolve()
        controller_path = args.controller_receipt.resolve()
        manifest_path = args.session_manifest.resolve()
        root = args.capture_root.resolve()
        selected_root = root / "a30_selected_present_receipts"
        require(selected_root.is_dir(), "selected-Present receipt directory absent")
        selected_files = sorted(path for path in selected_root.glob("*.json") if path.is_file())
        require(len(selected_files) >= len(REQUIRED_ACTIONS), "insufficient immutable selected-Present receipts")
        action_claim = validate_action(load_json(action_path), profile_sha)
        file_hashes = {file_sha(path): path for path in selected_files}
        require(len(file_hashes) == len(selected_files), "selected-Present corpus repeats content")
        require(all(path.stem == digest for digest, path in file_hashes.items()), "selected-Present receipt is not content addressed")
        require(set(action_claim["selected_present_sha256"]) <= set(file_hashes), "action-selected receipts differ from capture corpus")
        companion_claims = [validate_companion(load_json(file_hashes[digest])) for digest in action_claim["selected_present_sha256"]]
        for digest, companion in zip(action_claim["selected_present_sha256"], companion_claims, strict=True):
            bracket = action_claim["selected_present_actions"][digest]
            require(bracket["qpc_before"] <= companion["selected_qpc_ticks"] <= bracket["qpc_after"], f"selected Present is outside physical action bracket: {bracket['label']}")
        require(sum(claim["traits"] for claim in companion_claims) > 0, "portrait/trait group has no positive trait identity")
        require(sum(claim["nationalities"] for claim in companion_claims) > 0, "portrait/nationality group has no positive nationality identity")
        require(sum(claim["missions"] for claim in companion_claims) > 0, "mission group has no assignment-3 product")
        require(sum(claim["captured"] for claim in companion_claims) > 0, "captured group has no positive D04 witness")
        require(sum(claim["recruitable"] for claim in companion_claims) > 0, "recruitable group has no positive D04 witness")
        operations = [claim for claim in companion_claims if claim["visibility"]["operations"] == "visible" and claim["visibility"]["ciphers"] == "hidden"]
        ciphers = [claim for claim in companion_claims if claim["visibility"]["ciphers"] == "visible" and claim["visibility"]["operations"] == "hidden"]
        ledgers = [claim for claim in companion_claims if claim["visibility"]["intel_ledger"] == "visible"]
        require(operations and ciphers and ledgers, "operations/ciphers/intel-ledger visibility transitions incomplete")
        require(any(claim["visibility"]["agency"] == "visible" and claim["visibility"]["upgrades"] == "visible" and claim["visibility"]["operatives"] == "visible" for claim in companion_claims), "agency/upgrades/operatives visibility incomplete")
        controller_claim = validate_controller(load_json(controller_path))
        manifest = load_json(manifest_path)
        require(manifest.get("executable_sha256") == EXE_SHA, "native session executable mismatch")
        require(isinstance(manifest.get("session_id"), str) and manifest["session_id"], "native session id absent")
        require(isinstance(manifest.get("capture_id"), str) and manifest["capture_id"], "native capture id absent")
        binding_root = root / "a30_owner_bindings"
        binding_files = sorted(path for path in binding_root.glob("*.json") if path.is_file()) if binding_root.is_dir() else []
        require(len(binding_files) == 1, "A30 in-process owner binding is missing or ambiguous")
        binding_path = binding_files[0]
        require(binding_path.stem == file_sha(binding_path), "A30 owner binding is not content addressed")
        binding = load_json(binding_path)
        require(binding.get("schema") == "hoi4.a30.in_process_owner_binding.v1", "A30 owner binding schema mismatch")
        require(binding.get("executable_sha256") == EXE_SHA, "A30 owner binding executable mismatch")
        require(binding.get("session_id") == manifest["session_id"] and binding.get("capture_id") == manifest["capture_id"], "A30 owner binding differs from native session")
        require(is_sha(binding.get("nonce_sha256")), "A30 owner nonce hash invalid")
        require(binding.get("owner_tag_db_id") == OWNER_ID and binding.get("expected_game_hour_raw") == FRAME_HOUR, "A30 owner D04 identity mismatch")
        require(binding.get("production_scheduling_enabled") is False, "A30 owner unexpectedly enabled production scheduling")
        artifacts = []
        for kind, path in [("action", action_path), ("controller", controller_path), ("owner_binding", binding_path), ("session_manifest", manifest_path)]:
            artifacts.append({"kind": kind, "file_name": path.name, "bytes": path.stat().st_size, "sha256": file_sha(path)})
        for digest in action_claim["selected_present_sha256"]:
            path = file_hashes[digest]
            artifacts.append({"kind": "selected_present_companion", "file_name": f"a30_selected_present_receipts/{path.name}", "bytes": path.stat().st_size, "sha256": digest})
        artifacts.sort(key=lambda row: (row["kind"], row["file_name"]))
        claims = {
            "schema": "hoi4.a30.autonomous-production-acquisition-claims.v1",
            "executable_sha256": EXE_SHA,
            "save_sha256": SAVE_SHA,
            "frame_hour": FRAME_HOUR,
            "owner_tag_db_id": OWNER_ID,
            "session_id": manifest["session_id"],
            "capture_id": manifest["capture_id"],
            "groups": {
                "visibility": "pass",
                "portrait_trait_nationality": "pass",
                "captured_recruitable": "pass",
                "missions": "pass",
                "agency_semantics": "pass",
                "cipher_semantics": "pass",
            },
            "selected_present_count": len(companion_claims),
            "controller": controller_claim,
            "artifacts": artifacts,
        }
        receipt = {
            "schema": "hoi4.a30.autonomous-production-acquisition-receipt.v1",
            "status": "pass",
            "generated_utc": datetime.now(timezone.utc).isoformat(),
            "content_sha256": canonical_sha(claims),
            "claims": claims,
        }
        write_create_new(args.output, receipt)
        print(json.dumps(receipt, indent=2, sort_keys=True))
        return 0
    except GateError as exc:
        print(json.dumps({"schema": "hoi4.a30.autonomous-production-acquisition-receipt.v1", "status": "fail", "error": str(exc)}, indent=2, sort_keys=True))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
