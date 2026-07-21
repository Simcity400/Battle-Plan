#!/usr/bin/env python3
"""Admit exact X2 combat-panel leader/modifier observations from folded snapshots."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any, Iterable

SOURCE = "authenticated_a729_combatview_materializer_1415c9860"
ORDER = [
    "BORDER_CONFLICT_MODIFIER",
    "BM_AIR_SUPPORT",
    "BM_ENEMY_AIR_SUPERIORITY",
    "BM_FORT_MODIFIER",
    "BM_SHORE_BOMBARD",
    "BM_RAILWAY_GUN_BOMBARD",
    "BM_STACKING",
    "BM_MULTIPLE_DIRECTIONS",
    "BM_DECRYPTION",
    "BM_ACTIVE_CODE_CRACKING_BONUSES",
]


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def objects(value: Any) -> Iterable[dict[str, Any]]:
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from objects(child)
    elif isinstance(value, list):
        for child in value:
            yield from objects(child)


def admit_side(side: dict[str, Any]) -> dict[str, Any] | None:
    if side.get("panel_observation_source") != SOURCE:
        return None
    sequence = side.get("panel_capture_sequence")
    if not isinstance(sequence, int) or isinstance(sequence, bool) or sequence <= 0:
        raise ValueError("panel capture sequence is not a positive integer")
    leader_status = side.get("leader_status")
    leader_id = side.get("leader_id")
    if leader_status == "exact":
        if not isinstance(leader_id, int) or isinstance(leader_id, bool) or leader_id <= 0:
            raise ValueError("exact leader status lacks a positive leader id")
    elif leader_status == "exact_none":
        if leader_id is not None:
            raise ValueError("exact-none leader status carries an id")
    else:
        raise ValueError("authenticated panel row has unknown leader status")
    if side.get("effective_displayed_modifiers_status") != "exact":
        raise ValueError("authenticated panel row lacks exact modifier status")
    modifiers = side.get("effective_displayed_modifiers")
    if not isinstance(modifiers, list) or any(not isinstance(value, str) for value in modifiers):
        raise ValueError("exact displayed modifiers are not a string list")
    ranks = []
    for token in modifiers:
        if token not in ORDER:
            raise ValueError(f"unknown displayed modifier token: {token}")
        ranks.append(ORDER.index(token))
    if ranks != sorted(set(ranks)):
        raise ValueError("displayed modifiers are duplicate or out of engine order")
    return {
        "capture_sequence": sequence,
        "leader_status": leader_status,
        "leader_id": leader_id,
        "effective_displayed_modifiers": modifiers,
        "source": SOURCE,
    }


def verify(snapshot_root: Path) -> dict[str, Any]:
    if not snapshot_root.is_dir():
        raise ValueError(f"snapshot directory does not exist: {snapshot_root}")
    files = sorted(snapshot_root.rglob("*.json"))
    admitted: list[dict[str, Any]] = []
    evidence_files: list[dict[str, Any]] = []
    for path in files:
        value = json.loads(path.read_text(encoding="utf-8"))
        file_rows = []
        for obj in objects(value):
            for side_name in ("attacker", "defender"):
                side = obj.get(side_name)
                if isinstance(side, dict):
                    row = admit_side(side)
                    if row is not None:
                        row["side"] = side_name
                        file_rows.append(row)
        if file_rows:
            admitted.extend(file_rows)
            evidence_files.append(
                {
                    "path": str(path.resolve()),
                    "sha256": sha256(path),
                    "observations": len(file_rows),
                }
            )
    if not admitted:
        raise ValueError(
            "no authenticated X2 panel observation found; open one active land-combat panel during capture"
        )
    return {
        "schema": "hoi4.x2.combat_panel_observation.verification.v1",
        "status": "accepted",
        "program_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
        "source": SOURCE,
        "observation_count": len(admitted),
        "evidence_files": evidence_files,
        "observations": admitted,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--snapshots", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = verify(args.snapshots.resolve())
    except Exception as error:
        result = {
            "schema": "hoi4.x2.combat_panel_observation.verification.v1",
            "status": "rejected",
            "error": str(error),
        }
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        return 1
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
