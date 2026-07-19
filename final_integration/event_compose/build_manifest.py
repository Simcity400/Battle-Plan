#!/usr/bin/env python3
"""Freeze only the current event/compose audit's mechanically referenced files."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent
WORKSPACE = ROOT.parents[1]
MANIFEST = ROOT / "manifest.json"

# These files are part of the freeze because they define, explain, or summarize
# the current audit.  Gate logs are added mechanically from mechanical_result.
CONTROL_FILES = {
    "build_manifest.py": ("manifest_builder", "builds and verifies this freeze"),
    "MASTER_LIST.md": ("master", "current concise promotion disposition"),
    "REPORT.md": ("report", "current 36-gate audit report"),
    "run_audit.py": ("runner", "defines and executes the current 36 gates"),
    "verify_ui09_ui10_current_join.py": (
        "runner_dependency",
        "command input of gate UI09_UI10_current_canvas_join",
    ),
}

# These JSON files are outputs of commands in the current runner.  They are not
# named by mechanical_result because that result records command stdout/stderr,
# so their producer/source anchors are checked below before they enter the set.
CURRENT_SIDE_RESULTS = {
    "runs/n02_result.json": (
        "gate_side_result",
        "JSON result of gate N02_map_label_projection",
        "run_audit.py",
        "n02_result.json",
    ),
    "runs/ui09_current_contract.json": (
        "gate_side_input",
        "generated current contract consumed by gate UI09_UI10_current_canvas_join",
        "verify_ui09_ui10_current_join.py",
        "ui09_current_contract.json",
    ),
    "runs/ui09_current_result.json": (
        "gate_side_result",
        "JSON result of gate UI09_UI10_current_canvas_join",
        "verify_ui09_ui10_current_join.py",
        "ui09_current_result.json",
    ),
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def checked_relative(relative: str) -> Path:
    path = ROOT / relative
    if not path.resolve().is_relative_to(ROOT.resolve()):
        raise SystemExit(f"refusing unsafe manifest path: {relative}")
    if not path.is_file():
        raise SystemExit(f"current audit reference is missing: {relative}")
    return path


def main() -> int:
    result_relative = "mechanical_result.json"
    result_path = checked_relative(result_relative)
    result = json.loads(result_path.read_text(encoding="utf-8"))
    if result.get("status") != "PASS":
        raise SystemExit("refusing to freeze: mechanical result is not PASS")
    if (
        result.get("passed"),
        result.get("not_applicable"),
        result.get("post_port_fixture_gaps"),
        result.get("external_capture_gaps"),
        result.get("failed"),
        result.get("gate_count"),
    ) != (31, 1, 0, 4, 0, 36):
        raise SystemExit("refusing to freeze: mechanical result counts changed")
    gates = result.get("gates")
    if not isinstance(gates, list) or len(gates) != 36:
        raise SystemExit("refusing to freeze: expected exactly 36 gate records")

    references: dict[str, list[dict[str, str]]] = {}

    def reference(relative: str, kind: str, anchor: str) -> None:
        checked_relative(relative)
        references.setdefault(relative, []).append({"kind": kind, "anchor": anchor})

    reference(result_relative, "audit_result", "current 36-gate mechanical result")
    for relative, (kind, anchor) in CONTROL_FILES.items():
        reference(relative, kind, anchor)

    seen_gate_ids: set[str] = set()
    for gate in gates:
        gate_id = gate.get("id")
        if not isinstance(gate_id, str) or not gate_id or gate_id in seen_gate_ids:
            raise SystemExit(f"invalid or duplicate gate id: {gate_id!r}")
        seen_gate_ids.add(gate_id)
        for stream_name in ("stdout", "stderr"):
            stream = gate.get(stream_name)
            if not isinstance(stream, dict):
                raise SystemExit(f"gate {gate_id} lacks {stream_name} reference")
            relative = stream.get("path")
            expected_hash = stream.get("sha256")
            if not isinstance(relative, str) or not relative.startswith("runs/"):
                raise SystemExit(f"gate {gate_id} has invalid {stream_name} path")
            path = checked_relative(relative)
            if sha256(path) != expected_hash:
                raise SystemExit(f"gate {gate_id} {stream_name} hash drift: {relative}")
            reference(relative, "gate_log", f"mechanical_result.gates[{gate_id}].{stream_name}")

    if sum(1 for rows in references.values() for row in rows if row["kind"] == "gate_log") != 72:
        raise SystemExit("refusing to freeze: current 36 gates must reference 72 logs")

    external_artifacts = []
    for gate in gates:
        for artifact in gate.get("promotion_artifacts", []):
            relative = artifact.get("path")
            if not isinstance(relative, str):
                raise SystemExit(f"gate {gate['id']} has invalid promotion artifact path")
            path = WORKSPACE / relative
            if not path.is_file():
                raise SystemExit(f"gate {gate['id']} promotion artifact is missing: {relative}")
            if path.stat().st_size != artifact.get("bytes") or sha256(path) != artifact.get("sha256"):
                raise SystemExit(f"gate {gate['id']} promotion artifact drift: {relative}")
            external_artifacts.append({"gate": gate["id"], **artifact})

    static_srv_closure = result.get("static_srv_closure")
    if not isinstance(static_srv_closure, dict) or set(static_srv_closure) != {
        "frame_bundle", "verification", "trees_oracle_manifest"
    }:
        raise SystemExit("current result lacks the complete static SRV closure")
    static_srv_reference_rows = []
    for name, artifact in static_srv_closure.items():
        relative = artifact.get("path")
        expected_hash = artifact.get("sha256")
        expected_bytes = artifact.get("bytes")
        if name == "frame_bundle":
            path = WORKSPACE / relative
            if not path.is_file() or path.stat().st_size != expected_bytes or sha256(path) != expected_hash:
                raise SystemExit(f"static SRV production source drift: {relative}")
            external_artifacts.append({"gate": "static_srv_closure", **artifact})
        else:
            path = checked_relative(relative)
            if path.stat().st_size != expected_bytes or sha256(path) != expected_hash:
                raise SystemExit(f"static SRV closure artifact drift: {relative}")
            reference(relative, "static_srv_closure", name)
        static_srv_reference_rows.append({"name": name, **artifact})

    for relative, (kind, anchor, producer, source_token) in CURRENT_SIDE_RESULTS.items():
        source = checked_relative(producer).read_text(encoding="utf-8")
        if source_token not in source:
            raise SystemExit(f"current side-result producer anchor drift: {relative}")
        side_path = checked_relative(relative)
        try:
            side_result = json.loads(side_path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as exc:
            raise SystemExit(f"current side result is not JSON: {relative}: {exc}") from exc
        if relative == "runs/n02_result.json":
            if side_result.get("disposition") != "PROVEN_PRODUCTION_PROMOTION_READY":
                raise SystemExit(f"current N02 side result is not promotion-ready: {relative}")
        elif relative != "runs/ui09_current_contract.json" and side_result.get("status") != "PASS":
            raise SystemExit(f"current side result is not PASS: {relative}")
        reference(relative, kind, anchor)

    included_paths = sorted(references, key=str.lower)
    files = []
    reference_rows = []
    for relative in included_paths:
        path = checked_relative(relative)
        files.append({"path": relative, "bytes": path.stat().st_size, "sha256": sha256(path)})
        reference_rows.append({"path": relative, "references": references[relative]})

    # Enumerate everything deliberately left outside the freeze.  This makes
    # stale files visible without allowing them to become authority merely by
    # remaining in the shared dirty working tree.
    existing = {
        path.relative_to(ROOT).as_posix()
        for path in ROOT.rglob("*")
        if path.is_file() and path != MANIFEST
    }
    excluded = sorted(existing - set(included_paths), key=str.lower)
    omitted = existing - set(included_paths) - set(excluded)
    if omitted:
        raise SystemExit(f"manifest classification omission: {sorted(omitted)}")
    obsolete_logs = [path for path in excluded if path.startswith("runs/")]

    payload = {
        "schema": "hoi4-rendering-laws-final-event-compose-manifest-v3",
        "status": "FROZEN",
        "scope": "mechanical event-scoped pass and final-frame composition audit",
        "authority": result["authority"],
        "result": {
            "path": result_relative,
            "sha256": sha256(result_path),
            "status": "PASS",
            "direct_passes": 31,
            "not_applicable_diagnostics": 1,
            "post_port_fixture_gaps": 0,
            "external_capture_gaps": 4,
            "failures": 0,
            "total": 36,
        },
        "reference_proof": {
            "rule": "only current result/log references, current runner inputs/results, and report/master/runner/manifest-builder controls are frozen",
            "included_file_count": len(files),
            "current_gate_log_count": 72,
            "current_promotion_artifact_count": len(external_artifacts),
            "current_promotion_artifacts": external_artifacts,
            "current_static_srv_closure": static_srv_reference_rows,
            "included": reference_rows,
            "excluded_existing_artifacts": [
                {
                    "path": relative,
                    "reason": "not referenced by the current 36-gate result or current runner side-result set",
                }
                for relative in excluded
            ],
            "excluded_obsolete_run_artifact_count": len(obsolete_logs),
            "excluded_obsolete_run_artifacts": obsolete_logs,
        },
        "ghidra_provenance": {
            "function_address": None,
            "job_id": None,
            "artifact_path": None,
            "manifest_hash": None,
            "reason": "No Ghidra operation or executable finding was used in this audit.",
        },
        "files": files,
    }

    MANIFEST.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    written = json.loads(MANIFEST.read_text(encoding="utf-8"))
    for entry in written["files"]:
        path = ROOT / entry["path"]
        if path.stat().st_size != entry["bytes"] or sha256(path) != entry["sha256"]:
            raise SystemExit(f"manifest verification failed: {entry['path']}")
    frozen_paths = {entry["path"] for entry in written["files"]}
    if frozen_paths != set(included_paths):
        raise SystemExit("manifest reference proof and file set disagree")
    if frozen_paths.intersection(obsolete_logs):
        raise SystemExit("obsolete run artifact entered the manifest")

    print(json.dumps({
        "status": written["status"],
        "files": len(written["files"]),
        "current_gate_logs": 72,
        "excluded_obsolete_run_artifacts": len(obsolete_logs),
        "all_included_files_current_referenced": True,
        "obsolete_run_artifacts_excluded": True,
        "manifest_sha256": sha256(MANIFEST),
        "result_sha256": written["result"]["sha256"],
    }, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
