#!/usr/bin/env python3
"""Fail-closed pre-launch regrade of a prepared product-modset handoff.

This tool never queries or starts a process. It rejects duplicate JSON members,
re-hashes the complete prepared corpus and bundle, and repeats the source-tree,
archive, UI14 winner and product-modset joins used by preparation.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path
from typing import Any

import prepare_product_modset_capture as PREPARE
import static_bundle as SB


EXACT_A729_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"


def strict_pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, child in pairs:
        if key in value:
            raise ValueError(f"duplicate JSON member: {key}")
        value[key] = child
    return value


def load_strict(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=strict_pairs)
    if not isinstance(value, dict):
        raise ValueError(f"{path} is not a JSON object")
    return value


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def resolve_member(root: Path, row: Any, label: str) -> Path:
    if not isinstance(row, dict) or set(row) - {"file", "bytes", "sha256", "bundle_id",
                                                   "product_modset_sha256", "artifact_roles",
                                                   "token_join_status"}:
        raise ValueError(f"{label} receipt row is malformed")
    relative = row.get("file")
    if not isinstance(relative, str) or not relative or Path(relative).is_absolute():
        raise ValueError(f"{label} file is not a relative path")
    path = (root / relative).resolve()
    try:
        path.relative_to(root)
    except ValueError as exc:
        raise ValueError(f"{label} file escapes the prepared root") from exc
    if (not path.is_file() or isinstance(row.get("bytes"), bool)
            or row.get("bytes") != path.stat().st_size
            or row.get("sha256") != sha256_file(path)):
        raise ValueError(f"{label} bytes differ from the prepared receipt")
    return path


def verify(exe: Path, root: Path) -> dict[str, Any]:
    exe = exe.resolve()
    root = root.resolve()
    if not exe.is_file() or not root.is_dir():
        raise ValueError("executable or prepared product-modset directory is missing")
    exe_sha = sha256_file(exe)
    if exe_sha != EXACT_A729_SHA256:
        raise ValueError("executable is not exact a729 authority")
    ready_path = root / "product_modset_capture_ready.json"
    ready = load_strict(ready_path)
    if (ready.get("schema") != PREPARE.READY_SCHEMA
            or ready.get("status") != "ready_for_selected_modset_capture"
            or ready.get("preparation_mode") not in {
                "live_binding_resolution", "authenticated_binding_corpus"
            }):
        raise ValueError("prepared receipt header is noncanonical")
    evidence = ready.get("ui14_capture_evidence")
    if (not isinstance(evidence, dict)
            or set(evidence) != {
                "manifest", "active_config", "ready", "trace", "capture_nonce_sha256"
            }):
        raise ValueError("prepared receipt has no retained process-bound UI14 evidence")
    expected_tool = Path(PREPARE.__file__).resolve()
    if ready.get("preparation_tool_sha256") != sha256_file(expected_tool):
        raise ValueError("preparation tool differs from the receipt")

    bindings = root / "bindings"
    corpus = ready.get("binding_corpus")
    rows = corpus.get("files") if isinstance(corpus, dict) else None
    if (not isinstance(rows, list) or len(rows) != len(PREPARE.BINDING_FILES)
            or [row.get("name") for row in rows if isinstance(row, dict)]
            != list(PREPARE.BINDING_FILES)):
        raise ValueError("binding corpus roster is noncanonical")
    for row in rows:
        name = row["name"]
        path = bindings / name
        load_strict(path)
        if (set(row) != {"name", "bytes", "sha256"}
                or row.get("bytes") != path.stat().st_size
                or row.get("sha256") != sha256_file(path)):
            raise ValueError(f"binding corpus differs for {name}")
    corpus_core = {"schema": corpus.get("schema"), "files": rows}
    corpus_sha = hashlib.sha256(SB.canonical_json(corpus_core)).hexdigest()
    if (corpus.get("schema") != "extractor.product_modset_binding_corpus.v1"
            or corpus.get("sha256") != corpus_sha
            or ready.get("binding_corpus_sha256") != corpus_sha):
        raise ValueError("binding corpus aggregate digest mismatch")

    expected_roles = {
        "mod_state": "mod_state.json",
        "dlc_state": "dlc_state.json",
        "load_order": "load_order.json",
        "source_inventory": "source_inventory.json",
        "resolution_report": "resolution_report.json",
    }
    binding_rows = ready.get("bindings")
    if not isinstance(binding_rows, dict) or set(binding_rows) != set(expected_roles):
        raise ValueError("prepared binding receipt roster is noncanonical")
    for role, name in expected_roles.items():
        if resolve_member(root, binding_rows[role], role) != (bindings / name).resolve():
            raise ValueError(f"prepared {role} path does not name its canonical binding")
    if resolve_member(root, ready.get("engine_vfs_receipt"), "engine_vfs_receipt") \
            != (bindings / "ui14_engine_vfs.json").resolve():
        raise ValueError("prepared UI14 receipt path is noncanonical")
    retained_manifest = resolve_member(root, evidence.get("manifest"), "ui14_capture_manifest")
    retained_active = resolve_member(root, evidence.get("active_config"), "ui14_active_config")
    retained_ready = resolve_member(root, evidence.get("ready"), "ui14_capture_ready")
    retained_trace = resolve_member(root, evidence.get("trace"), "ui14_capture_trace")
    canonical_evidence = (root / "ui14_capture_evidence").resolve()
    if (retained_manifest != canonical_evidence / "manifest.json"
            or retained_active != canonical_evidence / "dlc_load.active.json"
            or retained_ready != canonical_evidence / "cpu_capture_ready.json"
            or retained_trace != canonical_evidence / "cpu_state.jsonl"):
        raise ValueError("retained UI14 evidence paths are noncanonical")
    capture_manifest = load_strict(retained_manifest)
    capture_rows = capture_manifest.get("artifacts")
    if (not isinstance(capture_rows, list)
            or isinstance(capture_manifest.get("artifact_count"), bool)
            or capture_manifest.get("artifact_count") != len(capture_rows)):
        raise ValueError("retained UI14 capture manifest roster is malformed")
    capture_paths = [row.get("path") for row in capture_rows if isinstance(row, dict)]
    if len(capture_paths) != len(capture_rows) or len(capture_paths) != len(set(capture_paths)):
        raise ValueError("retained UI14 capture manifest has malformed or duplicate paths")
    retained_artifacts = {
        "configuration/dlc_load.active.json": retained_active,
        "cpu/cpu_capture_ready.json": retained_ready,
        "cpu/cpu_state.jsonl": retained_trace,
    }
    for relative, retained in retained_artifacts.items():
        matches = [row for row in capture_rows if row.get("path") == relative]
        if (len(matches) != 1 or set(matches[0]) != {"path", "bytes", "sha256"}
                or isinstance(matches[0].get("bytes"), bool)
                or matches[0].get("bytes") != retained.stat().st_size
                or matches[0].get("sha256") != sha256_file(retained)):
            raise ValueError(f"retained UI14 capture manifest does not bind {relative}")
    manifest_path = resolve_member(root, ready.get("static_bundle"), "static_bundle")
    if manifest_path.name != "static_bundle_manifest.json":
        raise ValueError("prepared static-bundle manifest filename is noncanonical")

    _report, product_modset = PREPARE._validate_binding_corpus(exe, bindings)
    report = load_strict(bindings / "resolution_report.json")
    ui14_binding = report.get("ui14_capture_binding")
    if (not isinstance(ui14_binding, dict)
            or ui14_binding.get("schema") != "extractor.ui14_capture_binding.v1"
            or ui14_binding.get("manifest_file") != "ui14_capture_evidence/manifest.json"
            or ui14_binding.get("active_config_file")
                != "ui14_capture_evidence/dlc_load.active.json"
            or ui14_binding.get("ready_file")
                != "ui14_capture_evidence/cpu_capture_ready.json"
            or ui14_binding.get("trace_file") != "ui14_capture_evidence/cpu_state.jsonl"
            or ui14_binding.get("process_id") != report.get("process", {}).get("pid")
            or ui14_binding.get("process_creation_filetime_utc")
                != report.get("process", {}).get("creation_filetime_utc")
            or ui14_binding.get("capture_nonce_sha256") != evidence.get("capture_nonce_sha256")
            or ui14_binding.get("capture_manifest_sha256") != sha256_file(retained_manifest)
            or ui14_binding.get("active_dlc_load_sha256") != sha256_file(retained_active)
            or ui14_binding.get("ready_sha256") != sha256_file(retained_ready)
            or ui14_binding.get("trace_sha256") != sha256_file(retained_trace)):
        raise ValueError("retained UI14 evidence differs from its process-bound resolution report")
    active = load_strict(retained_active)
    enabled = active.get("enabled_mods")
    normalized_enabled = ([value.replace("\\", "/") for value in enabled]
                          if isinstance(enabled, list)
                          and all(isinstance(value, str) and value for value in enabled)
                          else None)
    ui14 = load_strict(bindings / "ui14_engine_vfs.json")
    layers = ui14.get("layers")
    layer_ids = ([row.get("id") for row in layers]
                 if isinstance(layers, list) and all(isinstance(row, dict) for row in layers)
                 else None)
    if (normalized_enabled is None or layer_ids is None
            or len(normalized_enabled) != len(set(normalized_enabled))
            or set(normalized_enabled) != set(layer_ids)):
        raise ValueError("retained active mod roster differs from the UI14 engine receipt")
    if ready.get("product_modset_sha256") != product_modset["product_modset_sha256"]:
        raise ValueError("prepared product-modset key differs from revalidated bindings")
    manifest = load_strict(manifest_path)
    bound = SB.validate_product_modset_binding(manifest.get("bindings", {}).get("product_modset"))
    if bound != product_modset:
        raise ValueError("static bundle product-modset binding differs from the corpus")
    if ready["static_bundle"].get("product_modset_sha256") != product_modset["product_modset_sha256"]:
        raise ValueError("static-bundle receipt product-modset key mismatch")
    return {
        "schema": "extractor.product_modset_capture_admission.v1",
        "verdict": "passed",
        "executable_sha256": exe_sha,
        "prepared_receipt_sha256": sha256_file(ready_path),
        "binding_corpus_sha256": corpus_sha,
        "static_bundle_manifest_sha256": sha256_file(manifest_path),
        "product_modset_sha256": product_modset["product_modset_sha256"],
        "paths": {
            "static_bundle_manifest": str(manifest_path),
            **{role: str((bindings / name).resolve()) for role, name in expected_roles.items()},
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--prepared", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    try:
        result = verify(args.exe, args.prepared)
    except (OSError, ValueError, json.JSONDecodeError, SB.BundleError) as exc:
        print(f"prepared product-modset admission: FAIL: {exc}", file=sys.stderr)
        return 1
    encoded = json.dumps(result, ensure_ascii=False, sort_keys=True, indent=2) + "\n"
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(encoded, encoding="utf-8")
    print(encoded, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
