#!/usr/bin/env python3
"""Independent fail-closed admission for an H4CT-owned A38/d84 capture."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import tempfile
from pathlib import Path

EXE_SHA = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
JOB_ID = "20260720T020812Z-w3-a38-controller-owner-41ee5208"
RECEIPT_SHA = "7d0a36b1822faf85a949d628a79bc4a09f95dc9c9aa3ed4aecb715b72b0a9082"
HEX64 = re.compile(r"^[0-9a-f]{64}$")
TAG = re.compile(r"^[A-Z0-9]{3}$")
RECORD = re.compile(r"^a38-owner-[0-9a-f]{16}-[0-9]+-[1-9][0-9]*$")


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def atomic_new_json(path: Path, value: object) -> None:
    path = path.resolve()
    if path.exists():
        raise ValueError(f"refusing to replace admission receipt: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        # os.rename is create-new on Windows; unlike replace it cannot race an
        # existing authenticated admission receipt out of existence.
        os.rename(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def engine_result(value: object, kind: type) -> tuple[bool, bool]:
    if not isinstance(value, dict) or set(value) != {"value", "unavailable_reason"}:
        raise ValueError("engine result is not the exact value/unavailable envelope")
    measured = value["value"] is not None and value["unavailable_reason"] is None
    unavailable = value["value"] is None and isinstance(value["unavailable_reason"], str) and value["unavailable_reason"]
    if not measured and not unavailable:
        raise ValueError("engine result is neither measured nor explicitly unavailable")
    if measured and not isinstance(value["value"], kind):
        raise ValueError("engine result has the wrong measured type")
    reason = value["unavailable_reason"]
    if isinstance(reason, str) and ("0x" in reason.lower() or ":\\" in reason or reason.startswith("/")):
        raise ValueError("engine result leaks process/host identity")
    return measured, unavailable


def verify_snapshot(path: Path) -> dict[str, int | str] | None:
    document = json.loads(path.read_text(encoding="utf-8-sig"))
    if not isinstance(document, dict):
        raise ValueError(f"{path}: snapshot root is not an object")
    authority = document.get("_r2_scripted_gui_runtime_live")
    if authority is None:
        return None
    expected_authority = {
        "schema_version", "authority", "elapsed_wall_ms", "frame_hour",
        "producer_record_identity", "executable_sha256", "ui14_engine_vfs_sha256",
        "static_authority_job_id", "static_authority_receipt_sha256",
        "producer_manifest_sha256", "manager_order", "tombstones",
    }
    if not isinstance(authority, dict) or set(authority) != expected_authority:
        raise ValueError(f"{path}: A38 authority envelope is partial or has unknown fields")
    if authority["authority"] != "exact20_a38_engine_materialized" \
            or authority["executable_sha256"] != EXE_SHA \
            or authority["static_authority_job_id"] != JOB_ID \
            or authority["static_authority_receipt_sha256"] != RECEIPT_SHA \
            or not isinstance(authority["ui14_engine_vfs_sha256"], str) \
            or not HEX64.fullmatch(authority["ui14_engine_vfs_sha256"]) \
            or authority["ui14_engine_vfs_sha256"] == "0" * 64 \
            or not isinstance(authority["producer_manifest_sha256"], str) \
            or not HEX64.fullmatch(authority["producer_manifest_sha256"]) \
            or authority["producer_manifest_sha256"] == "0" * 64 \
            or not isinstance(authority["producer_record_identity"], str) \
            or not RECORD.fullmatch(authority["producer_record_identity"]):
        raise ValueError(f"{path}: A38 authority identity is not exact")
    order = authority["manager_order"]
    if not isinstance(order, list) or len(order) > 65536:
        raise ValueError(f"{path}: A38 manager order is malformed")
    keys: list[tuple[str, str]] = []
    for index, item in enumerate(order):
        if not isinstance(item, dict) or set(item) != {"manager_order_index", "country_tag", "gui_id"} \
                or item["manager_order_index"] != index \
                or not isinstance(item["country_tag"], str) or not TAG.fullmatch(item["country_tag"]) \
                or not isinstance(item["gui_id"], str) or not item["gui_id"]:
            raise ValueError(f"{path}: A38 manager order is not contiguous/canonical")
        keys.append((item["country_tag"], item["gui_id"]))
    if len(set(keys)) != len(keys):
        raise ValueError(f"{path}: A38 manager order repeats an identity")
    tombstones = authority["tombstones"]
    if not isinstance(tombstones, list):
        raise ValueError(f"{path}: A38 tombstones are malformed")
    tombstone_keys = []
    for item in tombstones:
        if not isinstance(item, dict) or set(item) != {"country_tag", "gui_id"} \
                or not isinstance(item["country_tag"], str) or not TAG.fullmatch(item["country_tag"]) \
                or not isinstance(item["gui_id"], str):
            raise ValueError(f"{path}: A38 tombstone identity is malformed")
        tombstone_keys.append((item["country_tag"], item["gui_id"]))
    if tombstone_keys != sorted(set(tombstone_keys)) or set(tombstone_keys) & set(keys):
        raise ValueError(f"{path}: A38 tombstones are not sorted, unique, and disjoint")
    countries = document.get("countries")
    if not isinstance(countries, dict):
        raise ValueError(f"{path}: countries root is absent")
    measured_properties = unavailable_properties = measured_bools = 0
    folded_keys = []
    for tag, gui_id in keys:
        runtime = (((countries.get(tag) or {}).get("extra") or {}).get("scripted_gui_runtime") or {})
        row = runtime.get(gui_id)
        if not isinstance(row, dict):
            raise ValueError(f"{path}: A38 manager row has no folded runtime row")
        folded_keys.append((tag, gui_id))
        definition = row.get("definition")
        if not isinstance(definition, dict) or set(definition) != {
            "logical_path", "winning_layer_ordinal", "layer_kind", "layer_id",
            "source_sha256", "payload_sha256", "payload_bytes", "context_type", "window_name",
        }:
            raise ValueError(f"{path}: A38 definition identity is incomplete")
        logical = definition["logical_path"]
        if not isinstance(logical, str) or not logical.startswith("common/scripted_guis/") \
                or not logical.endswith(".txt") or "\\" in logical or ":" in logical \
                or definition["layer_kind"] not in {"base", "integrated_dlc", "dlc", "mod"} \
                or not isinstance(definition["source_sha256"], str) \
                or not HEX64.fullmatch(definition["source_sha256"]) \
                or not isinstance(definition["payload_sha256"], str) \
                or not HEX64.fullmatch(definition["payload_sha256"]) \
                or not isinstance(definition["payload_bytes"], int) or definition["payload_bytes"] <= 0:
            raise ValueError(f"{path}: A38 definition identity is not portable/exact")
        for field in ("visible", "enabled"):
            measured, _ = engine_result(row.get(field), bool)
            measured_bools += int(measured)
        for field in ("triggers", "effect_enabled"):
            values = row.get(field)
            if not isinstance(values, dict):
                raise ValueError(f"{path}: A38 {field} map is malformed")
            for value in values.values():
                measured, _ = engine_result(value, bool)
                measured_bools += int(measured)
        properties = row.get("properties")
        if not isinstance(properties, dict):
            raise ValueError(f"{path}: A38 property map is malformed")
        for property_map in properties.values():
            if not isinstance(property_map, dict):
                raise ValueError(f"{path}: A38 nested property map is malformed")
            for value in property_map.values():
                measured, unavailable = engine_result(value, str)
                measured_properties += int(measured)
                unavailable_properties += int(unavailable)
    if folded_keys != keys:
        raise ValueError(f"{path}: A38 manager/fold order differs")
    return {
        "frame_hour": authority["frame_hour"],
        "producer_record_identity": authority["producer_record_identity"],
        "rows": len(keys),
        "measured_properties": measured_properties,
        "unavailable_properties": unavailable_properties,
        "measured_bools": measured_bools,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--capture", required=True, type=Path)
    parser.add_argument("--snapshots", required=True, type=Path)
    parser.add_argument("--fold-report", required=True, type=Path)
    parser.add_argument("--session-manifest", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    for path in (args.capture, args.snapshots, args.fold_report, args.session_manifest):
        if not path.exists():
            raise SystemExit(f"required A38 artifact is absent: {path}")
    session = json.loads(args.session_manifest.read_text(encoding="utf-8-sig"))
    if not isinstance(session, dict) or session.get("schema") != "native_collector_session_v1" \
            or not isinstance(session.get("session_id"), str) or not session["session_id"] \
            or not isinstance(session.get("pid"), int) or session["pid"] <= 0 \
            or not isinstance(session.get("first_game_hour_raw"), int) \
            or not isinstance(session.get("last_game_hour_raw"), int) \
            or session["last_game_hour_raw"] < session["first_game_hour_raw"]:
        raise SystemExit("A38 session manifest lacks an authenticated process/session/hour interval")
    totals = {"frames": 0, "nonempty_frames": 0, "rows": 0, "measured_properties": 0,
              "unavailable_properties": 0, "measured_bools": 0}
    admitted_files = []
    for path in sorted(args.snapshots.glob("*.json")):
        result = verify_snapshot(path)
        if result is None:
            continue
        totals["frames"] += 1
        totals["nonempty_frames"] += int(result["rows"] > 0)
        for key in ("rows", "measured_properties", "unavailable_properties", "measured_bools"):
            totals[key] += result[key]
        frame_hour = result["frame_hour"]
        if not isinstance(frame_hour, int) or not (
            session["first_game_hour_raw"] <= frame_hour <= session["last_game_hour_raw"]
        ):
            raise SystemExit(f"{path}: A38 frame hour is outside the authenticated session interval")
        admitted_files.append({
            "name": path.name,
            "bytes": path.stat().st_size,
            "sha256": sha256(path),
            **result,
            "joins": {
                "engine_owner_consumer": "observed",
                "ui_materialized_state": "observed",
                "world_frame": "observed_folded_snapshot",
                "selected_present": "not_applicable_to_a38_d84_carrier",
            },
        })
    if totals["frames"] == 0 or totals["nonempty_frames"] == 0 \
            or totals["measured_properties"] == 0 or totals["measured_bools"] == 0:
        failure = {
            "schema": "extractor.a38_production_capture_admission.v1",
            "status": "FAIL_CLOSED_A38_UNOBSERVED",
            "production_admission_ready": False,
            "missing_state_policy": "unavailable_never_fabricated",
            "reason": "no nonempty complete frame with synchronized property and boolean consumers was observed",
            "executable_sha256": EXE_SHA,
            "session": {
                "session_id": session["session_id"],
                "process_id": session["pid"],
                "first_game_hour_raw": session["first_game_hour_raw"],
                "last_game_hour_raw": session["last_game_hour_raw"],
            },
            "totals": totals,
            "artifacts": {
                "capture_h4cr_sha256": sha256(args.capture),
                "fold_report_sha256": sha256(args.fold_report),
                "session_manifest_sha256": sha256(args.session_manifest),
                "snapshots": admitted_files,
            },
        }
        atomic_new_json(args.out, failure)
        print(json.dumps(failure, sort_keys=True))
        return 2
    receipt = {
        "schema": "extractor.a38_production_capture_admission.v1",
        "status": "PASS_A38_H4CT_OWNER_CONSUMER_D84",
        "production_admission_ready": True,
        "executable_sha256": EXE_SHA,
        "static_authority_job_id": JOB_ID,
        "static_authority_receipt_sha256": RECEIPT_SHA,
        "session": {
            "session_id": session["session_id"],
            "process_id": session["pid"],
            "first_game_hour_raw": session["first_game_hour_raw"],
            "last_game_hour_raw": session["last_game_hour_raw"],
        },
        "totals": totals,
        "artifacts": {
            "capture_h4cr_sha256": sha256(args.capture),
            "fold_report_sha256": sha256(args.fold_report),
            "session_manifest_sha256": sha256(args.session_manifest),
            "snapshots": admitted_files,
        },
    }
    atomic_new_json(args.out, receipt)
    print(json.dumps(receipt, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
