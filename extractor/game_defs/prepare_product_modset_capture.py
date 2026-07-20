#!/usr/bin/env python3
"""Prepare one selected product modset for autonomous production capture.

Live mode resolves a current UI14 engine-VFS receipt and process. Offline mode
copies a previously authenticated binding corpus and revalidates its complete
content without querying the process. Both rebuild all seven static catalogs,
create the deferred bundle, and emit the same pointer-free recorder handoff.
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
BINDING_FILES = (
    "mod_state.json",
    "dlc_state.json",
    "load_order.json",
    "source_inventory.json",
    "resolution_report.json",
    "ui14_engine_vfs.json",
)


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


def _copy_binding_corpus(source: Path, destination: Path) -> dict[str, Any]:
    """Copy one immutable binding corpus and return its path-free identity."""
    source = source.resolve()
    if not source.is_dir():
        raise LR.ResolutionError(f"binding corpus is not a directory: {source}")
    destination.mkdir(parents=True, exist_ok=False)
    rows = []
    for name in BINDING_FILES:
        path = source / name
        if not path.is_file():
            raise LR.ResolutionError(f"binding corpus is missing {name}")
        before = path.read_bytes()
        after = path.read_bytes()
        if before != after:
            raise LR.ResolutionError(f"binding corpus changed while copying {name}")
        target = destination / name
        target.write_bytes(before)
        if target.read_bytes() != before:
            raise LR.ResolutionError(f"copied binding corpus changed for {name}")
        # Strict JSON decoding happens again in the role-specific validators;
        # this early read rejects a non-object handoff before static work starts.
        SB.load_object(target, f"copied binding corpus {name}")
        rows.append({"name": name, "bytes": len(before), "sha256": SB.sha256_bytes(before)})
    core = {"schema": "extractor.product_modset_binding_corpus.v1", "files": rows}
    return {**core, "sha256": SB.sha256_bytes(SB.canonical_json(core))}


def _validate_binding_corpus(exe: Path, bindings_dir: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    """Validate every cross-document join needed by live or offline preparation."""
    exe_sha = SB.sha256_file(exe)
    # Keep the exact wire documents alongside their normalized validation
    # results.  The runtime builder's source re-hasher consumes the strict
    # mod_state.v1 wire shape (``mods``), while the cross-document joins below
    # deliberately consume the normalized shape (``items``).  Passing the
    # normalized document to the wire-shape helper used to fail with a KeyError
    # before an otherwise valid offline corpus could be admitted.
    wire_documents = {
        role: SB.load_object(
            bindings_dir / f"{role}.json", f"capture {role}"
        )
        for role in ("mod_state", "dlc_state", "load_order")
    }
    documents = {
        role: SB._validate_binding_document(wire_documents[role], role)
        for role in ("mod_state", "dlc_state", "load_order")
    }
    SB._validate_binding_identity(documents)
    source_inventory = LR.validate_source_inventory_document(
        SB.load_object(bindings_dir / "source_inventory.json", "capture source inventory")
    )
    if source_inventory["identity"] != documents["mod_state"]["identity"]:
        raise LR.ResolutionError("binding corpus source inventory identity mismatch")
    SB.validate_source_inventory_bindings(source_inventory, documents)

    report = SB.load_object(bindings_dir / "resolution_report.json", "capture resolution report")
    process = report.get("process")
    if (
        report.get("schema") != LR.RESOLUTION_SCHEMA
        or report.get("passed") is not True
        or not isinstance(process, dict)
        or process.get("exe_sha256") != exe_sha
        or report.get("binding_identity") != documents["mod_state"]["identity"]
        or report.get("source_inventory_rows") != len(source_inventory["rows"])
        or report.get("source_inventory_sha256") != source_inventory["inventory_sha256"]
    ):
        raise LR.ResolutionError("binding corpus resolution report does not match its content")
    if (
        not isinstance(process.get("pid"), int)
        or isinstance(process.get("pid"), bool)
        or process["pid"] <= 0
        or not isinstance(process.get("creation_filetime_utc"), int)
        or isinstance(process.get("creation_filetime_utc"), bool)
        or process["creation_filetime_utc"] <= 0
    ):
        raise LR.ResolutionError("binding corpus process identity is invalid")

    ui14 = SB.load_object(bindings_dir / "ui14_engine_vfs.json", "capture UI14 receipt")
    enabled_mod_ids = {
        row["id"] for row in documents["mod_state"]["items"] if row["enabled"]
    }
    layers = ui14.get("layers")
    winners = ui14.get("effective_winners")
    if (
        set(ui14) != {"schema", "executable_sha256", "layers", "effective_winners"}
        or ui14.get("schema") != LR.UI14_ENGINE_VFS_SCHEMA
        or ui14.get("executable_sha256") != exe_sha
        or not isinstance(layers, list)
        or not all(isinstance(row, dict) for row in layers)
        or len(layers) != len(enabled_mod_ids)
        or not isinstance(winners, list)
        or not all(isinstance(row, dict) for row in winners)
    ):
        raise LR.ResolutionError("binding corpus UI14 receipt does not match its executable/mod set")

    # This independently re-hashes every selected descriptor, directory tree,
    # and archive container before any generated catalog can consume it.
    sources = RB._load_resolved_mod_sources(bindings_dir, wire_documents["mod_state"])
    source_by_id = {row["id"]: row for row in sources}
    descriptor_ordinals: set[int] = set()
    mount_ordinals: set[int] = set()
    layer_ids: set[str] = set()
    for row in layers:
        if set(row) != {
            "id", "descriptor_sha256", "engine_descriptor_ordinal", "engine_mount_ordinal"
        }:
            raise LR.ResolutionError("binding corpus UI14 layer row is malformed")
        layer_id = row.get("id")
        source = source_by_id.get(layer_id)
        descriptor_ordinal = row.get("engine_descriptor_ordinal")
        mount_ordinal = row.get("engine_mount_ordinal")
        if (
            source is None
            or layer_id in layer_ids
            or row.get("descriptor_sha256") != source["descriptor_sha256"]
            or not isinstance(descriptor_ordinal, int)
            or isinstance(descriptor_ordinal, bool)
            or descriptor_ordinal < 0
            or descriptor_ordinal in descriptor_ordinals
            or not isinstance(mount_ordinal, int)
            or isinstance(mount_ordinal, bool)
            or mount_ordinal < 0
            or mount_ordinal in mount_ordinals
        ):
            raise LR.ResolutionError("binding corpus UI14 layer identity/order is invalid")
        layer_ids.add(layer_id)
        descriptor_ordinals.add(descriptor_ordinal)
        mount_ordinals.add(mount_ordinal)
    if layer_ids != enabled_mod_ids:
        raise LR.ResolutionError("binding corpus UI14 layer set does not match enabled mods")

    inventory_by_id: dict[str, list[dict[str, Any]]] = {}
    for source in sources:
        content = Path(source["content_path"])
        if source["content_kind"] == "archive":
            _digest, rows, _container_sha, _container_bytes = LR._zip_namespace_rows(
                content, source["id"], layer_kind="mod"
            )
        else:
            _digest, rows = LR.hash_tree(
                content, layer_kind="mod", layer_id=source["id"]
            )
        inventory_by_id[source["id"]] = rows
    winner_paths: set[str] = set()
    for row in winners:
        if set(row) != {"relpath", "winner_id", "sha256"}:
            raise LR.ResolutionError("binding corpus UI14 winner row is malformed")
        relpath = row.get("relpath")
        if not isinstance(relpath, str):
            raise LR.ResolutionError("binding corpus UI14 winner path is invalid")
        normalized = relpath.replace("\\", "/")
        key = normalized.casefold()
        if (
            not normalized
            or normalized.startswith("/")
            or (len(normalized) >= 2 and normalized[0].isalpha() and normalized[1] == ":")
            or any(part in {"", ".", ".."} for part in Path(normalized).parts)
            or key in winner_paths
            or row.get("winner_id") not in layer_ids
            or not isinstance(row.get("sha256"), str)
            or len(row["sha256"]) != 64
            or not all(char in "0123456789abcdef" for char in row["sha256"])
        ):
            raise LR.ResolutionError("binding corpus UI14 winner identity is invalid")
        winner_paths.add(key)
    LR.validate_ui14_mod_collision_winners(inventory_by_id, winners)
    product_modset = SB.product_modset_binding(exe_sha, documents, source_inventory)
    return report, product_modset


def _complete_preparation(
    *, exe: Path, bindings_dir: Path, output: Path, preparation_mode: str,
    binding_corpus: dict[str, Any] | None = None,
) -> dict[str, Any]:
    report, expected_product_modset = _validate_binding_corpus(exe, bindings_dir)
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
    if product_modset != expected_product_modset:
        raise LR.ResolutionError("prepared static bundle product-modset binding drifted")
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

    corpus = binding_corpus or {
        "schema": "extractor.product_modset_binding_corpus.v1",
        "files": [
            {
                "name": name,
                "bytes": (bindings_dir / name).stat().st_size,
                "sha256": SB.sha256_file(bindings_dir / name),
            }
            for name in BINDING_FILES
        ],
    }
    if "sha256" not in corpus:
        corpus["sha256"] = SB.sha256_bytes(SB.canonical_json(corpus))

    receipt = {
        "schema": READY_SCHEMA,
        "status": "ready_for_selected_modset_capture",
        "preparation_mode": preparation_mode,
        "preparation_tool_sha256": SB.sha256_file(Path(__file__).resolve()),
        "binding_corpus_sha256": corpus["sha256"],
        "binding_corpus": corpus,
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
    LR.resolve_live_bindings(
        exe=exe,
        userdir=userdir,
        pid=pid,
        out_dir=bindings_dir,
        ui14_engine_vfs=ui14_engine_vfs,
    )
    return _complete_preparation(
        exe=exe,
        bindings_dir=bindings_dir,
        output=output,
        preparation_mode="live_binding_resolution",
    )


def prepare_offline(*, exe: Path, bindings: Path, output: Path) -> dict[str, Any]:
    """Prepare from an already-authenticated, immutable binding corpus.

    This route performs no process query and consumes no launcher/log state. It
    re-hashes every selected source and rebuilds the same seven-role deferred
    bundle without weakening any content join. The resulting receipt remains
    bound to the corpus process identity; the production launcher independently
    requires that exact process to still be alive before capture.
    """
    output = output.resolve()
    exe = exe.resolve()
    bindings = bindings.resolve()
    if output.exists():
        raise LR.ResolutionError(f"output already exists: {output}")
    if not exe.is_file() or not bindings.is_dir():
        raise LR.ResolutionError("executable and binding corpus must exist")
    try:
        output.relative_to(bindings)
    except ValueError:
        pass
    else:
        raise LR.ResolutionError("offline output cannot be nested inside its binding corpus")
    output.mkdir(parents=True, exist_ok=False)
    copied_bindings = output / "bindings"
    corpus = _copy_binding_corpus(bindings, copied_bindings)
    return _complete_preparation(
        exe=exe,
        bindings_dir=copied_bindings,
        output=output,
        preparation_mode="authenticated_binding_corpus",
        binding_corpus=corpus,
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--userdir", type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--pid", type=int)
    mode.add_argument(
        "--bindings",
        type=Path,
        help="previously authenticated binding directory; performs no live process query",
    )
    parser.add_argument("--ui14-engine-vfs", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        if args.bindings is not None:
            if args.userdir is not None or args.ui14_engine_vfs is not None:
                parser.error("--bindings cannot be combined with --userdir/--ui14-engine-vfs")
            receipt = prepare_offline(
                exe=args.exe,
                bindings=args.bindings,
                output=args.out,
            )
        else:
            if args.userdir is None or args.ui14_engine_vfs is None:
                parser.error("live --pid mode requires --userdir and --ui14-engine-vfs")
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
