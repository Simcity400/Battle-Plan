#!/usr/bin/env python3
"""Materialize a selected-modset UI14 receipt from one raw exact-a729 trace."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

import live_binding_resolver as LR
import static_bundle as SB


def strict_pairs(rows: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, child in rows:
        if key in value:
            raise LR.ResolutionError(f"duplicate JSON member: {key}")
        value[key] = child
    return value


def strict_load(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=strict_pairs)
    if not isinstance(value, dict):
        raise LR.ResolutionError(f"{path} is not a JSON object")
    return value


def capture_manifest_artifact(capture_root: Path, manifest: dict[str, Any], relative: str) -> Path:
    rows = manifest.get("artifacts")
    if not isinstance(rows, list):
        raise LR.ResolutionError("capture manifest has no artifact roster")
    matches = [row for row in rows if isinstance(row, dict) and row.get("path") == relative]
    if len(matches) != 1 or set(matches[0]) != {"path", "bytes", "sha256"}:
        raise LR.ResolutionError(f"capture manifest does not uniquely bind {relative}")
    row = matches[0]
    path = (capture_root / relative).resolve()
    try:
        path.relative_to(capture_root)
    except ValueError as exc:
        raise LR.ResolutionError(f"capture artifact escapes capture root: {relative}") from exc
    if (not path.is_file() or isinstance(row["bytes"], bool)
            or row["bytes"] != path.stat().st_size
            or row["sha256"] != SB.sha256_file(path)):
        raise LR.ResolutionError(f"capture artifact differs from manifest: {relative}")
    return path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture-root", required=True, type=Path)
    parser.add_argument("--active-dlc-load", required=True, type=Path)
    parser.add_argument("--userdir", required=True, type=Path)
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    try:
        capture_root = args.capture_root.resolve()
        manifest_path = capture_root / "manifest.json"
        if not manifest_path.is_file():
            raise LR.ResolutionError("capture root has no manifest.json")
        manifest = strict_load(manifest_path)
        if (not isinstance(manifest.get("artifact_count"), int)
                or isinstance(manifest.get("artifact_count"), bool)
                or manifest["artifact_count"] != len(manifest.get("artifacts", []))):
            raise LR.ResolutionError("capture manifest artifact count is noncanonical")
        active_path = capture_manifest_artifact(
            capture_root, manifest, "configuration/dlc_load.active.json"
        )
        ready_path = capture_manifest_artifact(
            capture_root, manifest, "cpu/cpu_capture_ready.json"
        )
        trace_path = capture_manifest_artifact(capture_root, manifest, "cpu/cpu_state.jsonl")
        if args.active_dlc_load.resolve() != active_path:
            raise LR.ResolutionError(
                "active DLC configuration must be the file bound by the capture manifest"
            )
        ready = strict_load(ready_path)
        if (ready.get("schema") != "hoi4-rendering-law-cpu-capture-ready-v1"
                or ready.get("executable_sha256") != LR.EXPECTED_EXE_SHA256
                or ready.get("packets") != ["UI14"]):
            raise LR.ResolutionError("capture ready receipt is not an exact-a729 UI14 run")
        active = strict_load(active_path)
        enabled = active.get("enabled_mods")
        if (not isinstance(enabled, list)
                or not all(isinstance(value, str) and value for value in enabled)):
            raise LR.ResolutionError("capture-time active configuration has no enabled mod roster")
        normalized_enabled = [value.replace("\\", "/") for value in enabled]
        if len(normalized_enabled) != len(set(normalized_enabled)):
            raise LR.ResolutionError("capture-time active configuration repeats a mod")

        events: list[dict[str, Any]] = []
        for line_number, line in enumerate(
            trace_path.read_text(encoding="utf-8").splitlines(), 1
        ):
            if not line:
                raise LR.ResolutionError(f"blank UI14 trace line {line_number}")
            row = json.loads(line, object_pairs_hook=strict_pairs)
            if (not isinstance(row, dict)
                    or row.get("schema") != "hoi4-rendering-law-cpu-events-v1"
                    or row.get("process_id") != ready.get("process_id")):
                raise LR.ResolutionError(f"UI14 trace line {line_number} is not ready-bound")
            events.append(row)
        entries = [row for row in events if row.get("type") == "ui14_descriptor_parser_entry"]
        userdir = args.userdir.resolve()
        layers: list[dict[str, Any]] = []
        inventories: dict[str, list[dict[str, Any]]] = {}
        mod_plan_rows: list[dict[str, Any]] = []
        for layer_id in normalized_enabled:
            matches = [row for row in entries
                       if str(row.get("descriptor_utf8", "")).replace("\\", "/") == layer_id]
            if (len(matches) != 1 or matches[0].get("caller_return_rva") != "0x2066E9E"
                    or not isinstance(matches[0].get("ordinal"), int)):
                raise LR.ResolutionError(f"raw parser trace does not prove mod layer {layer_id}")
            descriptor = (userdir / layer_id).resolve()
            try:
                descriptor.relative_to(userdir)
            except ValueError as exc:
                raise LR.ResolutionError(f"mod descriptor escapes userdir: {layer_id}") from exc
            descriptor_sha, _ = LR._hash_file_stable(descriptor)
            fields = LR.parse_descriptor(descriptor)
            mod_plan_rows.append({
                "id": layer_id,
                "descriptor_id": layer_id,
                "name": fields["name"],
                "dependencies": fields["dependencies"],
            })
            reference = Path(fields.get("path") or fields["archive"])
            content = (userdir / reference).resolve() if not reference.is_absolute() else reference.resolve()
            if content.is_file():
                _digest, rows, _container_sha, _container_bytes = LR._zip_namespace_rows(
                    content, layer_id, layer_kind="mod"
                )
            else:
                _digest, rows = LR.hash_tree(content, layer_kind="mod", layer_id=layer_id)
            inventories[layer_id] = rows
            ordinal = matches[0]["ordinal"]
            layers.append({
                "id": layer_id,
                "descriptor_sha256": descriptor_sha,
                "engine_descriptor_ordinal": ordinal,
                "engine_mount_ordinal": None,
            })
        layers.sort(key=lambda row: row["engine_descriptor_ordinal"])
        ordinals = [row["engine_descriptor_ordinal"] for row in layers]
        if len(ordinals) != len(set(ordinals)):
            raise LR.ResolutionError("UI14 trace repeats a descriptor ordinal")
        mount_plan = [
            row for row in LR._engine_mount_plan([], mod_plan_rows)
            if row["planner_kind"] == "mod"
        ]
        mount_by_id = {
            row["descriptor_id"]: ordinal for ordinal, row in enumerate(mount_plan)
        }
        if set(mount_by_id) != set(normalized_enabled):
            raise LR.ResolutionError("a729 mount planner does not cover the captured mod set")
        for layer in layers:
            layer["engine_mount_ordinal"] = mount_by_id[layer["id"]]

        by_path: dict[str, list[tuple[int, str, dict[str, Any]]]] = {}
        ordinal_by_id = {row["id"]: row["engine_mount_ordinal"] for row in layers}
        for layer_id, rows in inventories.items():
            for row in rows:
                key = row["relpath"].replace("\\", "/").casefold()
                by_path.setdefault(key, []).append((ordinal_by_id[layer_id], layer_id, row))
        winners: list[dict[str, Any]] = []
        for rows in by_path.values():
            if len(rows) < 2:
                continue
            _ordinal, layer_id, row = max(rows, key=lambda value: value[0])
            winners.append({
                "relpath": row["relpath"].replace("\\", "/"),
                "winner_id": layer_id,
                "sha256": row["sha256"],
            })
        winners.sort(key=lambda row: row["relpath"].casefold())
        LR.validate_ui14_mod_collision_winners(inventories, winners)
        exe = args.exe.resolve()
        if SB.sha256_file(exe) != LR.EXPECTED_EXE_SHA256:
            raise LR.ResolutionError("executable is not exact a729 authority")
        result = {
            "schema": LR.UI14_ENGINE_VFS_SCHEMA,
            "executable_sha256": LR.EXPECTED_EXE_SHA256,
            "layers": layers,
            "effective_winners": winners,
        }
        args.out.parent.mkdir(parents=True, exist_ok=True)
        if args.out.exists():
            raise LR.ResolutionError(f"refusing to overwrite UI14 receipt: {args.out}")
        args.out.write_bytes(SB.pretty_json(result))
        print(json.dumps(result, ensure_ascii=False, sort_keys=True, indent=2))
        return 0
    except (OSError, ValueError, json.JSONDecodeError, LR.ResolutionError) as exc:
        print(f"UI14 selected-modset materialization: FAIL: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
