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
    "runs/o04_portable_result.json": (
        "gate_side_result",
        "portable relocation-safe O04 reviewed-verifier result with the full raw-RDC battery",
        "run_audit.py",
        "o04_portable_result.json",
    ),
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


def canonical_text_payload(path: Path) -> bytes:
    """Return UTF-8 authority text with checkout-independent LF line endings."""
    payload = path.read_bytes()
    try:
        payload.decode("utf-8")
    except UnicodeDecodeError as error:
        raise SystemExit(f"static SRV closure is not UTF-8 text: {path}") from error
    return payload.replace(b"\r\n", b"\n").replace(b"\r", b"\n")


def canonical_text_sha256(path: Path) -> str:
    return hashlib.sha256(canonical_text_payload(path)).hexdigest()


def checked_relative(relative: str) -> Path:
    path = ROOT / relative
    if not path.resolve().is_relative_to(ROOT.resolve()):
        raise SystemExit(f"refusing unsafe manifest path: {relative}")
    if not path.is_file():
        raise SystemExit(f"current audit reference is missing: {relative}")
    return path


def checked_receipt_file(reference: dict, label: str) -> Path:
    path = checked_relative(reference.get("path", ""))
    if (
        path.stat().st_size != reference.get("bytes")
        or sha256(path) != reference.get("sha256")
    ):
        raise SystemExit(f"refusing to freeze: {label} drifted")
    return path


def validate_replaced_receipts(
    receipts: object,
    prior_result: dict,
    expected_ids: list[str],
    anchor_prefix: str,
) -> list[tuple[str, str, str]]:
    if not isinstance(receipts, list) or [row.get("id") for row in receipts] != expected_ids:
        raise SystemExit("refusing to freeze: replaced gate receipt set drifted")
    prior_gates = {gate["id"]: gate for gate in prior_result.get("gates", [])}
    references = []
    for receipt in receipts:
        gate_id = receipt["id"]
        prior_gate = prior_gates.get(gate_id)
        if not isinstance(prior_gate, dict):
            raise SystemExit(f"refusing to freeze: prior gate is absent: {gate_id}")
        gate_record = receipt.get("gate_record", {})
        gate_record_path = checked_receipt_file(gate_record, f"{gate_id}.gate_record")
        if json.loads(gate_record_path.read_text(encoding="utf-8")) != prior_gate:
            raise SystemExit(f"refusing to freeze: replaced gate record drifted: {gate_id}")
        references.append((
            gate_record["path"],
            "replaced_gate_receipt",
            f"{anchor_prefix}[{gate_id}].gate_record",
        ))
        logs = receipt.get("logs")
        if not isinstance(logs, dict) or set(logs) != {"stdout", "stderr"}:
            raise SystemExit(f"refusing to freeze: replaced gate logs are incomplete: {gate_id}")
        for stream, archived in logs.items():
            checked_receipt_file(archived, f"{gate_id}.{stream}")
            if (
                archived.get("original_sha256") != prior_gate[stream]["sha256"]
                or archived.get("sha256") != prior_gate[stream]["sha256"]
            ):
                raise SystemExit(
                    f"refusing to freeze: replaced gate log drifted: {gate_id}.{stream}"
                )
            references.append((
                archived["path"],
                "replaced_gate_receipt",
                f"{anchor_prefix}[{gate_id}].{stream}",
            ))
        snapshots = receipt.get("artifact_snapshots")
        prior_artifacts = prior_gate.get("promotion_artifacts", [])
        if (
            not isinstance(snapshots, list)
            or [row.get("source_path") for row in snapshots]
            != [row.get("path") for row in prior_artifacts]
        ):
            raise SystemExit(
                f"refusing to freeze: replaced artifact receipt set drifted: {gate_id}"
            )
        for snapshot, prior_artifact in zip(snapshots, prior_artifacts):
            checked_receipt_file(
                snapshot, f"{gate_id}.{snapshot.get('source_path')}.artifact"
            )
            if (
                snapshot.get("prior_gate_bytes") != prior_artifact.get("bytes")
                or snapshot.get("prior_gate_sha256") != prior_artifact.get("sha256")
                or snapshot.get("matches_prior_gate")
                != (
                    snapshot.get("bytes") == prior_artifact.get("bytes")
                    and snapshot.get("sha256") == prior_artifact.get("sha256")
                )
            ):
                raise SystemExit(
                    f"refusing to freeze: replaced artifact receipt drifted: "
                    f"{gate_id}: {snapshot.get('source_path')}"
                )
            references.append((
                snapshot["path"],
                "replaced_gate_artifact_snapshot",
                f"{anchor_prefix}[{gate_id}].artifact_snapshots",
            ))
    return references


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
    ) != (29, 3, 0, 4, 0, 36):
        raise SystemExit("refusing to freeze: mechanical result counts changed")
    gates = result.get("gates")
    if not isinstance(gates, list) or len(gates) != 36:
        raise SystemExit("refusing to freeze: expected exactly 36 gate records")
    central_authority = result.get("authority", {}).get("central_validation")
    central_driver = "extractor/native/scripts/Invoke-CentralRustValidation.ps1"
    central_driver_path = WORKSPACE / central_driver
    if (
        not isinstance(central_authority, dict)
        or central_authority.get("path") != central_driver
        or central_authority.get("sha256") != sha256(central_driver_path)
        or central_authority.get("target_directory") != ".cargo-target"
        or central_authority.get("direct_cargo_invocation") is not False
    ):
        raise SystemExit("refusing to freeze: central Rust authority is missing or drifted")

    centralized_gates = [gate for gate in gates if "central_validation" in gate]
    if len(centralized_gates) != 18:
        raise SystemExit("refusing to freeze: expected 18 central-runner-owned Rust gates")
    central_modes = [gate["central_validation"].get("mode") for gate in centralized_gates]
    if (
        central_modes.count("Test") != 5
        or central_modes.count("Evidence") != 12
        or central_modes.count("Build") != 1
    ):
        raise SystemExit("refusing to freeze: central Rust gate tier counts changed")

    gate18_control = result.get("gate18_negative_control")
    if (
        not isinstance(gate18_control, dict)
        or gate18_control.get("classification")
        != "PINNED_LEGACY_NEGATIVE_CONTROL_NOT_A_FINAL_FRAME_PARITY_ORACLE"
    ):
        raise SystemExit("refusing to freeze: gate18 negative-control identity is absent")
    gate18_root = Path(gate18_control.get("capture", ""))
    gate18_files = gate18_control.get("files")
    if not isinstance(gate18_files, dict) or set(gate18_files) != {
        "map_frame_bundle.json",
        "terrain_chunk_library.json",
        "terrain_capture_events.jsonl",
    }:
        raise SystemExit("refusing to freeze: gate18 negative-control file set drifted")
    for name, pin in gate18_files.items():
        path = gate18_root / name
        if (
            not path.is_file()
            or path.stat().st_size != pin.get("bytes")
            or sha256(path) != pin.get("sha256")
        ):
            raise SystemExit(f"refusing to freeze: gate18 negative-control input drift: {path}")
    gate18_statuses = {
        gate["id"]: gate["status"]
        for gate in gates
        if gate["id"] in {
            "production_t01_lod_selection_gate18",
            "production_t01_vsnap_corpus_gate18",
            "production_t01_mesh_corpus_gate18",
            "production_final_frame_gate18",
        }
    }
    if gate18_statuses != {
        "production_t01_lod_selection_gate18": "NOT_APPLICABLE_INCOMPLETE_LEGACY_FIXTURE",
        "production_t01_vsnap_corpus_gate18": "PASS",
        "production_t01_mesh_corpus_gate18": "NOT_APPLICABLE_KNOWN_NEGATIVE_CONTROL",
        "production_final_frame_gate18": "NOT_APPLICABLE_KNOWN_NEGATIVE_CONTROL",
    }:
        raise SystemExit("refusing to freeze: gate18 positive/negative-control split drifted")
    for gate in centralized_gates:
        invocation = gate.get("central_validation")
        if (
            not isinstance(invocation, dict)
            or invocation.get("driver") != central_driver
            or invocation.get("driver_sha256") != sha256(central_driver_path)
            or invocation.get("workspace") != "Replay"
            or invocation.get("package") != "map_viewer"
            or invocation.get("target_directory") != ".cargo-target"
            or invocation.get("direct_cargo_invocation") is not False
        ):
            raise SystemExit(f"gate {gate.get('id')} lacks current central validation authority")
        if invocation.get("mode") == "Build":
            if invocation.get("target") != "bin:map_viewer" or invocation.get("filter_expression") is not None:
                raise SystemExit("renderer build is not the targeted central map_viewer build")
        elif invocation.get("target") != "lib" or not isinstance(invocation.get("filter_expression"), str):
            raise SystemExit(f"gate {gate.get('id')} is not a targeted central library test")

    renderer_build = next(gate for gate in centralized_gates if gate["central_validation"]["mode"] == "Build")
    renderer = renderer_build.get("renderer")
    if not isinstance(renderer, dict) or renderer.get("path") != ".cargo-target/debug/map_viewer.exe":
        raise SystemExit("renderer build lacks the shared-target binary pin")
    renderer_path = WORKSPACE / renderer["path"]
    if (
        not renderer_path.is_file()
        or renderer_path.stat().st_size != renderer.get("bytes")
        or sha256(renderer_path) != renderer.get("sha256")
    ):
        raise SystemExit("shared-target renderer changed after the central build")

    references: dict[str, list[dict[str, str]]] = {}

    def reference(relative: str, kind: str, anchor: str) -> None:
        checked_relative(relative)
        references.setdefault(relative, []).append({"kind": kind, "anchor": anchor})

    reference(result_relative, "audit_result", "current 36-gate mechanical result")
    resume = result.get("resume")
    if resume is not None:
        mode = resume.get("mode") if isinstance(resume, dict) else None
        if mode == "FRESH_AUDIT_FAILED_L02_L03_UI09_ONLY":
            failed_ids = [
                "L02_shadow_matrix_composition",
                "L03_object_pass_projection",
                "UI09_UI10_current_canvas_join",
            ]
            identity = {
                "bytes": 47545,
                "sha256": "bd4c992508078e0a3305cd4a39832e4dd1c15387036f9e516267ffa609acc24c",
            }
            if (
                resume.get("accepted_gates_preserved") != 33
                or resume.get("previous_failed_gate_ids") != failed_ids
                or resume.get("drifted_lightweight_packet_artifacts") != {}
                or resume.get("rerun_gate_ids") != failed_ids
                or resume.get("prior_resume_mode") is not None
                or resume.get("input_identity_contract") != identity
                or resume.get("aborted_resume_recovered_without_rerun") is not True
            ):
                raise SystemExit("refusing to freeze: fresh packet-repair authority drifted")
            resume_input = resume.get("input", {})
            if (
                resume_input.get("bytes") != identity["bytes"]
                or resume_input.get("sha256") != identity["sha256"]
            ):
                raise SystemExit("refusing to freeze: fresh input identity drifted")
            resume_path = checked_receipt_file(resume_input, "fresh packet-repair input")
            prior_result = json.loads(resume_path.read_text(encoding="utf-8"))
            prior_failed = [
                gate.get("id")
                for gate in prior_result.get("gates", [])
                if gate.get("status") == "FAIL"
            ]
            if (
                prior_result.get("resume") is not None
                or (
                    prior_result.get("passed"),
                    prior_result.get("not_applicable"),
                    prior_result.get("post_port_fixture_gaps"),
                    prior_result.get("external_capture_gaps"),
                    prior_result.get("failed"),
                    prior_result.get("gate_count"),
                ) != (26, 3, 0, 4, 3, 36)
                or prior_failed != failed_ids
            ):
                raise SystemExit("refusing to freeze: fresh input audit state drifted")
            for relative, kind, anchor in validate_replaced_receipts(
                resume.get("replaced_gate_receipts"),
                prior_result,
                failed_ids,
                "resume.replaced_gate_receipts",
            ):
                reference(relative, kind, anchor)
            reference(resume_input["path"], "resume_input", "authenticated fresh audit input")
        elif mode == "SECOND_STAGE_FAILED_UI15_ONLY":
            if (
                resume.get("accepted_gates_preserved") != 35
                or resume.get("previous_failed_gate_ids")
                != ["UI15_archive_mount_promotion"]
                or resume.get("drifted_lightweight_packet_artifacts") != {}
                or resume.get("rerun_gate_ids") != ["UI15_archive_mount_promotion"]
                or resume.get("prior_resume_mode")
                != "FAILED_AND_DRIFTED_LIGHTWEIGHT_PACKET_GATES_ONLY"
            ):
                raise SystemExit("refusing to freeze: second-stage resume authority drifted")
            resume_input = resume.get("input", {})
            resume_path = checked_receipt_file(resume_input, "second-stage resume input")
            prior_result = json.loads(resume_path.read_text(encoding="utf-8"))
            for relative, kind, anchor in validate_replaced_receipts(
                resume.get("replaced_gate_receipts"),
                prior_result,
                ["UI15_archive_mount_promotion"],
                "resume.replaced_gate_receipts",
            ):
                reference(relative, kind, anchor)
            prior_resume = prior_result.get("resume")
            if (
                not isinstance(prior_resume, dict)
                or prior_resume.get("mode")
                != "FAILED_AND_DRIFTED_LIGHTWEIGHT_PACKET_GATES_ONLY"
                or prior_resume.get("accepted_gates_preserved") != 34
                or prior_resume.get("previous_failed_gate_ids")
                != ["UI15_archive_mount_promotion"]
                or prior_resume.get("drifted_lightweight_packet_artifacts")
                != {
                    "M01_unit_event_pass": [
                        "replay_interface/re_evidence/rendering_laws/M01/runtime_admission/verification_result.json"
                    ]
                }
                or prior_resume.get("rerun_gate_ids")
                != ["M01_unit_event_pass", "UI15_archive_mount_promotion"]
            ):
                raise SystemExit("refusing to freeze: first-stage resume chain drifted")
            first_input = prior_resume.get("input", {})
            first_input_path = checked_receipt_file(first_input, "first-stage resume input")
            first_prior_result = json.loads(first_input_path.read_text(encoding="utf-8"))
            for relative, kind, anchor in validate_replaced_receipts(
                prior_resume.get("replaced_gate_receipts"),
                first_prior_result,
                ["M01_unit_event_pass", "UI15_archive_mount_promotion"],
                "prior_resume.replaced_gate_receipts",
            ):
                reference(relative, kind, anchor)
            first_o04 = prior_resume.get("preserved_o04_full_battery", {})
            first_o04_path = checked_relative(first_o04.get("path", ""))
            if (
                first_o04.get("raw_rdc_count") != 199
                or first_o04.get("all_sha256_verified") is not True
                or sha256(first_o04_path) != first_o04.get("sha256")
            ):
                raise SystemExit("refusing to freeze: first-stage O04 receipt drifted")
            reference(first_input["path"], "resume_input", "authenticated first-stage input")
            reference(resume_input["path"], "resume_input", "authenticated second-stage input")
        else:
            raise SystemExit("refusing to freeze: unsupported resume mode")

        preserved_o04 = resume.get("preserved_o04_full_battery", {})
        o04_path = checked_relative(preserved_o04.get("path", ""))
        if (
            preserved_o04.get("raw_rdc_count") != 199
            or preserved_o04.get("all_sha256_verified") is not True
            or sha256(o04_path) != preserved_o04.get("sha256")
        ):
            raise SystemExit("refusing to freeze: preserved O04 full-battery receipt drifted")
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
            if (
                not path.is_file()
                or len(canonical_text_payload(path)) != expected_bytes
                or canonical_text_sha256(path) != expected_hash
            ):
                raise SystemExit(f"static SRV production source drift: {relative}")
            external_artifacts.append({"gate": "static_srv_closure", **artifact})
        else:
            path = checked_relative(relative)
            if (
                len(canonical_text_payload(path)) != expected_bytes
                or canonical_text_sha256(path) != expected_hash
            ):
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
        elif relative == "runs/o04_portable_result.json":
            if (
                side_result.get("status") != "PASS"
                or side_result.get("reviewed_verifier_sha256")
                != "a19ad56fbc28a53ce10ff2998bc237af90c7148c0284c6f69a0770136918f9fa"
                or side_result.get("raw_rdc_inventory", {}).get("rdc_count") != 199
                or side_result.get("raw_rdc_inventory", {}).get("all_sha256_verified_this_run") is not True
            ):
                raise SystemExit(f"current O04 portable result is not full-battery PASS: {relative}")
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
            "direct_passes": 29,
            "not_applicable_diagnostics": 3,
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
