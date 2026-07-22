#!/usr/bin/env python3
"""Regrade an autonomous exact-a729 H-X2-LOSS/H-X2-WIDTH capture."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
from typing import Any

from verify_x2_combat_panel_observation import (
    EXE_SHA256,
    SOURCE as PANEL_SOURCE,
    load_object,
    lower_sha256,
    sha256,
    sha256_bytes,
    validate_action,
    validate_action_profile,
    validate_attestation,
    validate_manifest,
    validate_readiness,
)

WIDTH_SOURCE = "authenticated_a729_141299610_call"
LOSS_SOURCES = {
    "authenticated_a729_active_division_damage_140c7bfc0",
    "authenticated_a729_participant_lifecycle_141299850",
}


def exact_int(value: Any, minimum: int, maximum: int) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and minimum <= value <= maximum


def write_json_create_new_atomic(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        raise FileExistsError(f"refusing to replace immutable X2 regrade: {path}")
    payload = (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")
    descriptor, temporary_name = tempfile.mkstemp(prefix=path.name + ".", suffix=".tmp", dir=str(path.parent))
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        # A hard-link is a same-volume, create-new publication. Unlike
        # os.replace, it cannot overwrite a receipt created after the first
        # existence check.
        os.link(temporary, path)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def validate_width_call(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict) or value.get("source") != WIDTH_SOURCE:
        raise ValueError(f"{label}: width call lacks exact producer authority")
    sequence = value.get("capture_sequence")
    final_raw = value.get("final_width_raw")
    wrappers = value.get("wrappers")
    if not exact_int(sequence, 1, (1 << 64) - 1) or not exact_int(final_raw, -(1 << 63), (1 << 63) - 1):
        raise ValueError(f"{label}: width identity/result is malformed")
    if not isinstance(wrappers, list) or len(wrappers) > 128:
        raise ValueError(f"{label}: width wrapper roster is malformed")
    total = 0
    divisions: list[int] = []
    for ordinal, row in enumerate(wrappers):
        if not isinstance(row, dict) or row.get("ordinal") != ordinal:
            raise ValueError(f"{label}: width wrapper order is noncanonical")
        division_id = row.get("division_id")
        width_raw = row.get("width_raw")
        if not exact_int(division_id, 1, (1 << 32) - 1) or not exact_int(width_raw, -(1 << 63), (1 << 63) - 1):
            raise ValueError(f"{label}: width wrapper identity/result is malformed")
        divisions.append(division_id)
        total += width_raw
        if total < -(1 << 63) or total > (1 << 63) - 1:
            raise ValueError(f"{label}: checked width sum overflowed i64")
    if len(set(divisions)) != len(divisions) or total != final_raw:
        raise ValueError(f"{label}: wrapper identities are duplicate or exact sum differs from producer")
    return {
        "capture_sequence": sequence,
        "final_width_raw": final_raw,
        "wrapper_count": len(wrappers),
        "division_ids": divisions,
        "wrappers": wrappers,
    }


def validate_loss_call(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict) or value.get("source") not in LOSS_SOURCES:
        raise ValueError(f"{label}: loss call lacks an authenticated synchronous writer")
    sequence = value.get("capture_sequence")
    before = value.get("manpower_before_raw")
    after = value.get("manpower_after_raw")
    equipment = value.get("equipment")
    if (
        not exact_int(sequence, 1, (1 << 64) - 1)
        or not exact_int(before, -(1 << 63), (1 << 63) - 1)
        or not exact_int(after, -(1 << 63), (1 << 63) - 1)
        or not isinstance(equipment, list)
        or len(equipment) > 128
    ):
        raise ValueError(f"{label}: loss call identity/scalars are malformed")
    variants: list[dict[str, int]] = []
    equipment_ids: list[int] = []
    for ordinal, row in enumerate(equipment):
        if not isinstance(row, dict) or row.get("ordinal") != ordinal:
            raise ValueError(f"{label}: equipment loss order is noncanonical")
        equipment_id = row.get("equipment_id")
        equipment_before = row.get("before_raw")
        equipment_after = row.get("after_raw")
        if (
            not exact_int(equipment_id, 1, (1 << 32) - 1)
            or not exact_int(equipment_before, -(1 << 63), (1 << 63) - 1)
            or not exact_int(equipment_after, -(1 << 63), (1 << 63) - 1)
        ):
            raise ValueError(f"{label}: equipment loss row is malformed")
        equipment_ids.append(equipment_id)
        variants.append({
            "ordinal": ordinal,
            "equipment_id": equipment_id,
            "before_raw": equipment_before,
            "after_raw": equipment_after,
            "delta_raw": equipment_after - equipment_before,
        })
    if len(set(equipment_ids)) != len(equipment_ids):
        raise ValueError(f"{label}: equipment identities are duplicate")
    return {
        "capture_sequence": sequence,
        "source": value["source"],
        "manpower_before_raw": before,
        "manpower_after_raw": after,
        "manpower_delta_raw": after - before,
        "equipment": variants,
    }


def snapshot_rows(snapshot_root: Path) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    files = sorted(snapshot_root.rglob("*.json"))
    if not files:
        raise ValueError("acceptance snapshot directory contains no JSON snapshots")
    rows: list[dict[str, Any]] = []
    file_receipts: list[dict[str, Any]] = []
    seen_hours: set[int] = set()
    for path in files:
        value, _ = load_object(path, f"snapshot {path.name}")
        hour = value.get("hour_index")
        if not exact_int(hour, 1, (1 << 63) - 1):
            raise ValueError(f"snapshot {path.name}: hour_index is malformed")
        if hour in seen_hours:
            raise ValueError(f"acceptance snapshots repeat hour_index {hour}")
        seen_hours.add(hour)
        file_receipts.append({
            "path": path.relative_to(snapshot_root).as_posix(),
            "sha256": sha256(path),
            "hour_index": hour,
        })
        combats = value.get("combats")
        if combats is None:
            continue
        if not isinstance(combats, list):
            raise ValueError(f"snapshot {path.name}: combats is not a list or null")
        for combat in combats:
            if not isinstance(combat, dict) or combat.get("kind") != "land":
                continue
            combat_id = combat.get("combat_id")
            detail = combat.get("land_combat_detail_events_live")
            if not exact_int(combat_id, 1, (1 << 32) - 1) or not isinstance(detail, dict):
                continue
            generation = detail.get("generation")
            if not exact_int(generation, 1, (1 << 64) - 1):
                raise ValueError(f"snapshot {path.name}: active combat generation is malformed")
            if detail.get("side_detail_completeness") not in ("both", "both_counts_incomplete"):
                continue
            for side_name in ("attacker", "defender"):
                side = detail.get(side_name)
                if not isinstance(side, dict):
                    raise ValueError(f"snapshot {path.name}: complete combat lacks {side_name}")
                reserve = side.get("reserve_division_ids")
                if not isinstance(reserve, list) or any(not exact_int(v, 1, (1 << 32) - 1) for v in reserve):
                    continue
                if len(set(reserve)) != len(reserve):
                    raise ValueError(f"snapshot {path.name}: reserve identities are duplicate")
                width_values = side.get("width_observations", [])
                loss_values = side.get("current_loss_observations", [])
                if not isinstance(width_values, list) or not isinstance(loss_values, list):
                    raise ValueError(f"snapshot {path.name}: X2 observation roster is malformed")
                widths = [validate_width_call(v, f"{path.name}:{combat_id}:{generation}:{side_name}") for v in width_values]
                losses = [validate_loss_call(v, f"{path.name}:{combat_id}:{generation}:{side_name}") for v in loss_values]
                rows.append({
                    "snapshot": path.relative_to(snapshot_root).as_posix(),
                    "hour_index": hour,
                    "combat_id": combat_id,
                    "generation": generation,
                    "side": side_name,
                    "committed": side.get("committed"),
                    "reserve": side.get("reserve"),
                    "reserve_division_ids": reserve,
                    "selected_present": side.get("panel_observation_source") == PANEL_SOURCE,
                    "widths": widths,
                    "losses": losses,
                })
    rows.sort(key=lambda row: (row["combat_id"], row["generation"], row["side"], row["hour_index"]))
    file_receipts.sort(key=lambda row: row["hour_index"])
    return rows, file_receipts


def grade_observations(rows: list[dict[str, Any]]) -> tuple[dict[str, Any], dict[str, Any]]:
    selected_keys = {
        (row["combat_id"], row["generation"], row["side"])
        for row in rows
        if row["selected_present"]
    }
    loss_candidates: list[dict[str, Any]] = []
    for row in rows:
        key = (row["combat_id"], row["generation"], row["side"])
        if key not in selected_keys:
            continue
        for loss in row["losses"]:
            changed_equipment = [variant for variant in loss["equipment"] if variant["delta_raw"] != 0]
            if loss["manpower_delta_raw"] != 0 and changed_equipment:
                loss_candidates.append({
                    "snapshot_member": row["snapshot"],
                    "hour_index": row["hour_index"],
                    "combat_id": row["combat_id"],
                    "generation": row["generation"],
                    "side": row["side"],
                    **loss,
                    "changed_equipment": changed_equipment,
                    "selected_present_join": True,
                })
    if not loss_candidates:
        raise ValueError("H-X2-LOSS: no selected/present active side has one synchronous nonzero manpower and equipment-variant update")

    width_candidates: list[dict[str, Any]] = []
    grouped: dict[tuple[int, int, str], list[dict[str, Any]]] = {}
    for row in rows:
        grouped.setdefault((row["combat_id"], row["generation"], row["side"]), []).append(row)
    for key, timeline in grouped.items():
        if key not in selected_keys:
            continue
        timeline.sort(key=lambda row: row["hour_index"])
        for before, after in zip(timeline, timeline[1:]):
            if not exact_int(before["committed"], 0, 128) or not exact_int(after["committed"], 0, 128):
                continue
            before_reserve = set(before["reserve_division_ids"])
            after_reserve = set(after["reserve_division_ids"])
            transitioned = sorted(before_reserve - after_reserve)
            if after["committed"] <= before["committed"] or not transitioned or not before["widths"] or not after["widths"]:
                continue
            before_call = before["widths"][-1]
            after_call = after["widths"][0]
            joined = [division_id for division_id in transitioned if division_id not in before_call["division_ids"] and division_id in after_call["division_ids"]]
            if not joined:
                continue
            width_candidates.append({
                "combat_id": key[0],
                "generation": key[1],
                "side": key[2],
                "before_snapshot_member": before["snapshot"],
                "after_snapshot_member": after["snapshot"],
                "before_hour_index": before["hour_index"],
                "after_hour_index": after["hour_index"],
                "committed_before": before["committed"],
                "committed_after": after["committed"],
                "transitioned_division_ids": joined,
                "before_call": before_call,
                "after_call": after_call,
                "selected_present_join": True,
            })
    if not width_candidates:
        raise ValueError("H-X2-WIDTH: no selected/present same-epoch reserve-to-committed transition has exact before/after producer calls")
    return loss_candidates[0], width_candidates[0]


def verify(
    snapshots: Path,
    manifest_path: Path,
    action_path: Path,
    readiness_path: Path,
    profile_path: Path,
    attestation_path: Path,
) -> dict[str, Any]:
    manifest, manifest_raw = load_object(manifest_path, "capture manifest")
    manifest_identity = validate_manifest(manifest)
    attestation, attestation_raw = load_object(attestation_path, "central Release attestation")
    attestation_identity = validate_attestation(attestation)
    profile, profile_raw = load_object(profile_path, "frozen X2 action profile")
    validate_action_profile(profile)
    readiness, readiness_raw = load_object(readiness_path, "autonomous X2 readiness receipt")
    readiness_identity = validate_readiness(readiness, manifest_identity, sha256_bytes(profile_raw))
    action, action_raw = load_object(action_path, "autonomous X2 action receipt")
    action_identity = validate_action(action, manifest_identity, attestation_identity, sha256_bytes(attestation_raw))
    if action.get("selection_profile_sha256") != sha256_bytes(profile_raw):
        raise ValueError("autonomous action receipt differs from the supplied frozen profile")
    rows, files = snapshot_rows(snapshots)
    loss, width = grade_observations(rows)
    return {
        "schema": "hoi4.x2.loss-width-autonomous-regrade.v1",
        "status": "PASS",
        "production_admission_ready": True,
        "program_sha256": EXE_SHA256,
        "h_x2_loss": {"status": "PASS", "observation": loss},
        "h_x2_width": {"status": "PASS", "observation": width},
        "selected_present_source": PANEL_SOURCE,
        "autonomous_action": action_identity,
        "autonomous_readiness": readiness_identity,
        "evidence_files": files,
        "inputs": [
            {"kind": "capture_manifest", "path": manifest_path.name, "sha256": sha256_bytes(manifest_raw)},
            {"kind": "autonomous_action_receipt", "path": action_path.name, "sha256": sha256_bytes(action_raw)},
            {"kind": "autonomous_readiness_receipt", "path": readiness_path.name, "sha256": sha256_bytes(readiness_raw)},
            {"kind": "frozen_action_profile", "path": profile_path.name, "sha256": sha256_bytes(profile_raw)},
            {"kind": "central_release_attestation", "path": attestation_path.name, "sha256": sha256_bytes(attestation_raw)},
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--snapshots", type=Path, required=True)
    parser.add_argument("--capture-manifest", type=Path, required=True)
    parser.add_argument("--action-receipt", type=Path, required=True)
    parser.add_argument("--readiness-receipt", type=Path, required=True)
    parser.add_argument("--action-profile", type=Path, required=True)
    parser.add_argument("--prebuilt-attestation", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    try:
        result = verify(
            args.snapshots.resolve(),
            args.capture_manifest.resolve(),
            args.action_receipt.resolve(),
            args.readiness_receipt.resolve(),
            args.action_profile.resolve(),
            args.prebuilt_attestation.resolve(),
        )
    except Exception as error:
        result = {
            "schema": "hoi4.x2.loss-width-autonomous-regrade.v1",
            "status": "FAIL",
            "production_admission_ready": False,
            "errors": [str(error)],
        }
        try:
            write_json_create_new_atomic(out, result)
        except FileExistsError:
            pass
        return 1
    write_json_create_new_atomic(out, result)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
