#!/usr/bin/env python3
"""Mechanically verify the pinned the game 1.19.2 building runtime-id registry."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import uuid
from pathlib import Path


EXE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
REGISTRY_ROW = re.compile(r'\(0x([0-9A-Fa-f]+),\s*"([^"]+)"\)')
BLOCK_OPEN = re.compile(r"^\s*([A-Za-z0-9_@.-]+)\s*=\s*\{")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_registry(path: Path) -> dict[int, str]:
    rows = {int(raw_id, 16): token for raw_id, token in REGISTRY_ROW.findall(path.read_text())}
    if len(rows) != 55 or len(set(rows.values())) != 55:
        raise AssertionError("registry must contain exactly 55 unique ids and tokens")
    if list(rows) != sorted(rows):
        raise AssertionError("registry rows must be sorted by runtime id")
    return rows


def building_definitions(path: Path) -> set[str]:
    depth = 0
    in_buildings = False
    definitions: set[str] = set()
    for raw_line in path.read_text(encoding="utf-8-sig").splitlines():
        line = raw_line.split("#", 1)[0]
        match = BLOCK_OPEN.match(line)
        if not in_buildings and depth == 0 and match and match.group(1) == "buildings":
            in_buildings = True
        elif in_buildings and depth == 1 and match:
            token = match.group(1)
            if token == "spawn_points":
                break
            if not token.endswith("_spawn"):
                definitions.add(token)
        depth += line.count("{") - line.count("}")
    return definitions


def save_json(rakaly: Path, save: Path, temp_parent: Path | None) -> dict:
    resolved_temp_parent = (temp_parent or Path.cwd()).resolve()
    melted = resolved_temp_parent / f"_b8_registry_verify_{uuid.uuid4().hex}.hoi4"
    try:
        melt_result = subprocess.run(
            [str(rakaly), "melt", str(save), "--format", "the game", "-u", "stringify", "-o", str(melted)],
            capture_output=True,
        )
        if not melted.is_file() or not melted.read_bytes()[:8].startswith(b"HOI4txt"):
            raise RuntimeError(melt_result.stderr.decode(errors="replace"))
        result = subprocess.run(
            [str(rakaly), "json", str(melted), "-k", "group"],
            check=True,
            capture_output=True,
        )
        return json.loads(result.stdout)
    finally:
        melted.unlink(missing_ok=True)


def building_level(location: object, token: str) -> int:
    if not isinstance(location, dict):
        return 0
    value = location.get("buildings", {}).get(token, 0)
    if isinstance(value, dict):
        return int(value.get("level", 0))
    if isinstance(value, list):
        return sum(building_level({"buildings": {token: item}}, token) for item in value)
    return int(value or 0)


def live_vectors(snapshot: dict, root: str, field: str) -> tuple[dict[int, dict[str, int]], int]:
    vectors: dict[int, dict[str, int]] = {}
    row_count = 0
    for location_id, location in snapshot[root].items():
        buildings = location.get("construction_buildings_state_damage_live", {}).get(field, {})
        for raw_id, detail in buildings.items():
            type_id = int(raw_id.removeprefix("type_0x"), 16)
            vectors.setdefault(type_id, {})[location_id] = int(detail["level"])
            row_count += 1
    return vectors, row_count


def infer(vectors: dict[int, dict[str, int]], oracle: dict, location_ids: set[str]) -> dict[int, list[str]]:
    tokens = sorted(
        {token for location in oracle.values() if isinstance(location, dict) for token in location.get("buildings", {})}
    )
    return {
        type_id: [
            token
            for token in tokens
            if all(
                vector.get(location_id, 0) == building_level(oracle.get(location_id, {}), token)
                for location_id in location_ids
            )
        ]
        for type_id, vector in vectors.items()
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--snapshot", type=Path, required=True)
    parser.add_argument("--save", type=Path, required=True)
    parser.add_argument("--rakaly", type=Path, required=True)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--registry", type=Path, required=True)
    parser.add_argument("--building-file", type=Path, action="append", required=True)
    parser.add_argument("--temp-dir", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    if sha256(args.exe) != EXE_SHA256:
        raise AssertionError("executable SHA-256 does not match the pinned build")
    registry = load_registry(args.registry)
    definitions: set[str] = set()
    for path in args.building_file:
        definitions.update(building_definitions(path))
    if definitions != set(registry.values()):
        raise AssertionError(
            f"game-file/registry token mismatch: missing={sorted(definitions - set(registry.values()))}, "
            f"extra={sorted(set(registry.values()) - definitions)}"
        )

    snapshot = json.loads(args.snapshot.read_text(encoding="utf-8"))
    oracle = save_json(args.rakaly, args.save, args.temp_dir)
    if snapshot["date"] != oracle["date"]:
        raise AssertionError("snapshot and save are not from the same game hour")
    state_vectors, state_rows = live_vectors(snapshot, "states", "state_buildings")
    province_vectors, province_rows = live_vectors(snapshot, "provinces", "buildings")
    state_candidates = infer(
        state_vectors, oracle["states"], set(snapshot["states"]) | set(oracle["states"])
    )
    province_candidates = infer(
        province_vectors,
        oracle["provinces"],
        set(snapshot["provinces"]) | set(oracle["provinces"]),
    )
    observed = state_candidates | province_candidates
    ambiguous = {f"0x{type_id:04X}": values for type_id, values in observed.items() if len(values) != 1}
    mismatches = {
        f"0x{type_id:04X}": {"inferred": values[0], "registry": registry.get(type_id)}
        for type_id, values in observed.items()
        if len(values) == 1 and registry.get(type_id) != values[0]
    }
    if ambiguous or mismatches or len(observed) != 43:
        raise AssertionError(f"live/save join failed: ambiguous={ambiguous}, mismatches={mismatches}")

    proof = {
        "schema": "battle-plan.b8_x6_building_registry_proof.v1",
        "result": "PASS",
        "game_hour": snapshot["date"],
        "pinned_exe_sha256": EXE_SHA256,
        "inputs": {
            "snapshot_sha256": sha256(args.snapshot),
            "save_sha256": sha256(args.save),
            "registry_sha256": sha256(args.registry),
            "building_file_sha256": {path.name: sha256(path) for path in args.building_file},
        },
        "game_file_definition_count": len(definitions),
        "registry_count": len(registry),
        "observed_runtime_id_count": len(observed),
        "uninstantiated_runtime_id_count": len(registry) - len(observed),
        "state_locations": len(snapshot["states"]),
        "province_locations": len(snapshot["provinces"]),
        "state_rows": state_rows,
        "province_rows": province_rows,
        "rows_verified": state_rows + province_rows,
        "ambiguous_ids": ambiguous,
        "mismatches": mismatches,
        "observed_mapping": {
            f"0x{type_id:04X}": values[0] for type_id, values in sorted(observed.items())
        },
        "uninstantiated_mapping": {
            f"0x{type_id:04X}": token for type_id, token in registry.items() if type_id not in observed
        },
    }
    encoded = json.dumps(proof, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(encoded, encoding="utf-8")
    print(encoded, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
