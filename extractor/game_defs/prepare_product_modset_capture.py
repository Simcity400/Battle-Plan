#!/usr/bin/env python3
"""Prepare one selected product modset for autonomous production capture.

The only external observation accepted by this command is the current UI14
engine-VFS receipt for the already-launched, user-selected modset.  From that
receipt and process it resolves the current bindings, rebuilds all seven static
catalogs, creates the deferred bundle, and emits a pointer-free handoff for the
recorder's existing capture-bound token finalizer.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

import build_runtime_static_bundle as RB
import live_binding_resolver as LR
import static_bundle as SB


READY_SCHEMA = "extractor.product_modset_capture_ready.v1"
ABSOLUTE_WINDOWS = re.compile(r"^[A-Za-z]:[\\/]")


def _relative_file(root: Path, path: Path) -> dict[str, Any]:
    resolved_root = root.resolve()
    resolved = path.resolve()
    try:
        relative = resolved.relative_to(resolved_root).as_posix()
    except ValueError as exc:
        raise LR.ResolutionError(f"capture handoff file escapes output root: {path}") from exc
    if not resolved.is_file():
        raise LR.ResolutionError(f"capture handoff file is missing: {path}")
    return {
        "file": relative,
        "bytes": resolved.stat().st_size,
        "sha256": SB.sha256_file(resolved),
    }


def _reject_pointers(value: Any, label: str = "receipt") -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            _reject_pointers(child, f"{label}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            _reject_pointers(child, f"{label}[{index}]")
    elif isinstance(value, str):
        if ABSOLUTE_WINDOWS.match(value) or value.startswith(("/", "\\\\", "0x")):
            raise LR.ResolutionError(f"pointer/path leaked into {label}")


def prepare(
    *,
    exe: Path,
    userdir: Path,
    pid: int,
    ui14_engine_vfs: Path,
    output: Path,
) -> dict[str, Any]:
    if pid <= 0:
        raise LR.ResolutionError("pid must be positive")
    output = output.resolve()
    if output.exists():
        raise LR.ResolutionError(f"output already exists: {output}")
    exe = exe.resolve()
    userdir = userdir.resolve()
    ui14_engine_vfs = ui14_engine_vfs.resolve()
    if not exe.is_file() or not userdir.is_dir() or not ui14_engine_vfs.is_file():
        raise LR.ResolutionError("executable, userdir, and UI14 engine-VFS receipt must exist")

    bindings_dir = output / "bindings"
    resolved = LR.resolve_live_bindings(
        exe=exe,
        userdir=userdir,
        pid=pid,
        out_dir=bindings_dir,
        ui14_engine_vfs=ui14_engine_vfs,
    )
    static_output = output / "static_bundle"
    build = RB.build(
        bindings_dir=bindings_dir,
        vanilla=exe.parent,
        output=static_output,
        exe=exe,
        resume=False,
    )

    bundle_dir = static_output / "bundle"
    manifest_path = bundle_dir / "static_bundle_manifest.json"
    manifest = SB.load_object(manifest_path, "prepared static bundle manifest")
    manifest_bindings = manifest.get("bindings")
    if not isinstance(manifest_bindings, dict):
        raise LR.ResolutionError("prepared static bundle has no binding object")
    product_modset = SB.validate_product_modset_binding(
        manifest_bindings.get("product_modset")
    )
    token_join = manifest.get("token_join")
    if not isinstance(token_join, dict) or token_join.get("status") != "deferred":
        raise LR.ResolutionError("capture-ready static bundle must have a deferred token join")
    artifact_rows = manifest.get("artifacts")
    if not isinstance(artifact_rows, list):
        raise LR.ResolutionError("prepared static bundle artifact roster is not an array")
    roles = sorted(
        SB._required_string(row.get("role"), "prepared artifact role")
        for row in artifact_rows
        if isinstance(row, dict)
    )
    if len(roles) != len(artifact_rows):
        raise LR.ResolutionError("prepared static bundle has a malformed artifact row")
    expected_roles = sorted(spec.role for spec in SB.DEFAULT_SPECS)
    if roles != expected_roles:
        raise LR.ResolutionError("capture-ready bundle does not have the exact seven-role roster")

    report = resolved["report"]
    receipt = {
        "schema": READY_SCHEMA,
        "status": "ready_for_selected_modset_capture",
        "preparation_tool_sha256": SB.sha256_file(Path(__file__).resolve()),
        "product_modset_sha256": product_modset["product_modset_sha256"],
        "process": {
            "pid": report["process"]["pid"],
            "creation_filetime_utc": report["process"]["creation_filetime_utc"],
            "executable_sha256": report["process"]["exe_sha256"],
        },
        "engine_vfs_receipt": _relative_file(
            output, bindings_dir / "ui14_engine_vfs.json"
        ),
        "bindings": {
            name: _relative_file(output, bindings_dir / f"{name}.json")
            for name in (
                "mod_state",
                "dlc_state",
                "load_order",
                "source_inventory",
                "resolution_report",
            )
        },
        "static_bundle": {
            **_relative_file(output, manifest_path),
            "bundle_id": manifest["bundle_id"],
            "product_modset_sha256": product_modset["product_modset_sha256"],
            "artifact_roles": roles,
            "token_join_status": "deferred",
        },
        "finalization_contract": {
            "owner": "recorder_host.static_bundle_gate",
            "emitted_inventory_schema": SB.EMITTED_INVENTORY_SCHEMA,
            "final_join_schema": SB.FINAL_JOIN_SCHEMA,
            "required_provenance": ["capture_id", "session_id", "stream_sha256"],
            "unknown_token_count_must_equal": 0,
            "independent_post_capture_reverification_required": True,
            "same_product_modset_key_required": True,
        },
        "catalog_build": {
            "schema": build["schema"],
            "passed": build["passed"],
        },
    }
    _reject_pointers(receipt)
    ready_path = output / "product_modset_capture_ready.json"
    if ready_path.exists():
        raise LR.ResolutionError("capture-ready receipt already exists")
    ready_path.write_bytes(SB.pretty_json(receipt))
    return receipt


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--userdir", type=Path, required=True)
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--ui14-engine-vfs", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        receipt = prepare(
            exe=args.exe,
            userdir=args.userdir,
            pid=args.pid,
            ui14_engine_vfs=args.ui14_engine_vfs,
            output=args.out,
        )
    except (LR.ResolutionError, SB.BundleError, OSError, json.JSONDecodeError) as exc:
        print(f"product modset capture preparation: FAIL: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(receipt, ensure_ascii=False, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
