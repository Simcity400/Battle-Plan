#!/usr/bin/env python3
"""Mechanical A26/d72 producer, registry, fold and capture-oracle promotion gate."""

from __future__ import annotations

import hashlib
import json
import os
import struct
import subprocess
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
NATIVE = HERE.parents[1]
ROOT = NATIVE.parents[1]
DOSSIER = ROOT / "extractor/documents/dossiers/A26_RESOLVED_TOOLTIP_INSTANCES.json"
ORACLES = ROOT / "replay_interface/re_evidence/rendering_laws/XUI05/capture_oracles.json"
REPLAY_SCHEMA = ROOT / "extractor/documents/production_replay_handoff/replay_payload.schema.json"
REGISTRY = ROOT / "extractor/native/domain_registry.json"
COMPACT_CONTRACT = ROOT / "extractor/native/compact_contract/src/lib.rs"
HOOK = HERE / "src/a729_hook.rs"
FOLD = ROOT / "extractor/native/recorder_host/src/world_fold.rs"
FOLD_TESTS = ROOT / "extractor/native/recorder_host/src/world_fold/tests.rs"
TARGET = ROOT / ".cargo-target-a26"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run(command: list[str], cwd: Path = NATIVE) -> dict:
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(TARGET)
    proc = subprocess.run(command, cwd=cwd, env=env, text=True, capture_output=True)
    return {
        "command": " ".join(command),
        "returncode": proc.returncode,
        "stderr_tail": proc.stderr[-2000:],
    }


def main() -> None:
    checks: list[dict] = []

    def check(name: str, passed: bool, detail=None) -> None:
        checks.append({"name": name, "pass": bool(passed), "detail": detail})

    dossier = json.loads(DOSSIER.read_text(encoding="utf-8"))
    oracles = json.loads(ORACLES.read_text(encoding="utf-8"))
    replay_schema = json.loads(REPLAY_SCHEMA.read_text(encoding="utf-8"))
    check(
        "authority exact",
        dossier["authority"]["executable_sha256"]
        == "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
    )
    check("promotion status exact", dossier["status"] == "PROMOTION_READY_EXACT_A729_D72")
    carrier = dossier["production_carrier"]
    check(
        "domain 72 promotion carrier",
        carrier["domain_id"] == 72
        and carrier["registry_status"] == "validated_slice_schedulable"
        and carrier["live_dependency"] is None,
    )
    provenance = dossier["ghidra_provenance"]
    check(
        "primary Ghidra tuple exact",
        provenance["function_address"] == "0x1402A4080"
        and provenance["job_id"]
        == "20260717T113739Z-a25-ctextbox-vtable-identity-accessors-7ddacf03"
        and provenance["artifact_path"] == "evidence/func_1402a4080.decompile.c"
        and provenance["artifact_sha256"]
        == "a1838600f7f016606ed6287926ef47d5b836a5501bc83ef59467c5dd61a1c071"
        and provenance["manifest_hash"]
        == "2bc2bc389cfcadb3903ad6c79a03645a5fe4b7102feac1eb982f89e6aabac9c5",
    )

    registry = json.loads(REGISTRY.read_text(encoding="utf-8"))
    domain72 = next(row for row in registry["domains"] if row["id"] == 72)
    check(
        "domain 72 schedulable registry",
        domain72["name"] == "resolved_tooltip_instances_live"
        and domain72["status"] == "validated_slice"
        and domain72["schedulable"] is True
        and domain72["requires_data_record"] is True,
    )
    compact_source = COMPACT_CONTRACT.read_text(encoding="utf-8")
    d72_block = compact_source.split("// A26/XUI05", 1)[1].split("// A24/XUI11", 1)[0]
    check(
        "domain 72 compact descriptor promoted",
        '72,' in d72_block
        and "CompactDomainStatus::ValidatedSlice" in d72_block
        and "true," in d72_block,
    )

    metadata = run(["cargo", "metadata", "--locked", "--offline", "--no-deps", "--format-version", "1"])
    lane = run(["cargo", "test", "--locked", "--offline", "-p", "resolved_tooltip_instances_live_lane"])
    registry_ids = run(["cargo", "test", "--locked", "--offline", "-p", "compact_contract", "all_registry_document_ids_are_assigned"])
    registry_json = run(["cargo", "test", "--locked", "--offline", "-p", "compact_contract", "native_json_registry_is_derived_from_descriptors"])
    collector = run(["cargo", "check", "--locked", "--offline", "-p", "collector"])
    recorder_tests = run(
        ["cargo", "test", "--locked", "--offline", "-p", "recorder_host", "a26_domain72", "--lib"]
    )
    recorder_check = run(["cargo", "check", "--locked", "--offline", "-p", "recorder_host"])
    formatting = run(
        [
            "rustfmt",
            "--edition",
            "2021",
            "--check",
            str(HERE / "src/lib.rs"),
            str(HOOK),
        ],
        ROOT,
    )
    for name, result in [
        ("workspace metadata", metadata),
        ("lane tests 6/6", lane),
        ("registry ID gate", registry_ids),
        ("registry JSON gate", registry_json),
        ("collector integration compiles", collector),
        ("recorder domain-72 tests 2/2", recorder_tests),
        ("recorder integration compiles", recorder_check),
        ("lane source and hook rustfmt", formatting),
    ]:
        check(name, result["returncode"] == 0, result)

    derived_rects = []
    raw_rows = []
    for capture in oracles["captures"]:
        rdc = Path(capture["rdc"])
        check(f"RDC hash {rdc.name}", rdc.is_file() and sha256(rdc) == capture["rdc_sha256"])
        for key in ("chrome_vertex_blob", "quad_blob", "constant_buffer_zip"):
            path = Path(capture[key])
            check(
                f"artifact hash {path.name}",
                path.is_file() and sha256(path) == capture[f"{key}_sha256"],
            )
        with zipfile.ZipFile(capture["constant_buffer_zip"]) as archive:
            data = archive.read(capture["constant_buffer_entry"])
        check(
            f"constant-buffer entry hash {capture['constant_buffer_entry']}",
            hashlib.sha256(data).hexdigest() == capture["constant_buffer_entry_sha256"],
        )
        tx, ty = struct.unpack_from("<ff", data, 48)
        width, height = capture["offscreen_viewport_px"]
        x = round((tx + 1.0) * 960.0)
        y = round((1.0 - ty) * 540.0 - height)
        rect = [x, y, width, height]
        derived_rects.append(rect)
        raw_rows.append({"entry": capture["constant_buffer_entry"], "translation": [tx, ty], "derived_rect": rect})
        check(f"constant-buffer rectangle {capture['constant_buffer_entry']}", rect == capture["screen_rect_px"], raw_rows[-1])

    check("two exact capture rectangles", derived_rects == [[1223, 671, 297, 148], [1238, 876, 291, 164]])
    source = (HERE / "src/lib.rs").read_text(encoding="utf-8")
    hook = HOOK.read_text(encoding="utf-8")
    fold = FOLD.read_text(encoding="utf-8")
    fold_tests = FOLD_TESTS.read_text(encoding="utf-8")
    check("no screen-derived producer fallback", "OCR" in source and "RenderDoc" in source and "ExactTooltipInstanceSource" in source)
    check("complete-frame fields implemented", all(token in source for token in ["producer_manifest_sha256", "completeness_record_identity", "expected_instance_count"]))
    check("availability states implemented", all(token in source for token in ["Resolved", "NotApplicable", "DataBlocked"]))
    check(
        "exact a729 hook boundary implemented",
        all(
            token in hook
            for token in [
                "TOOLTIP_CONTROLLER_RVA: u32 = 0x002A_4080",
                "TOOLTIP_EXTENT_ACCESSOR_RVA: u32 = 0x0228_F100",
                "CONTROLLER_SENTINEL",
                "install_a729_runtime_text_hooks",
                "register_post_setter_observer",
                "register_pre_destructor_observer",
                "snapshot_after_controller(controller)",
                "GUI_VISIBLE_FLAGS_OFFSET",
                "PRODUCER_MANIFEST_SHA256",
            ]
        ),
    )
    check(
        "no duplicate final-setter detour",
        "FUN_1422B59E0" not in hook
        and hook.count("MinHook::create_hook(") == 1
        and "hook_tooltip_controller" in hook,
    )
    check(
        "domain 72 folded without inference",
        all(
            token in fold
            for token in [
                "72 => self.fold_resolved_tooltip_instances(payload)",
                "fn fold_resolved_tooltip_instances",
                '"_r2_resolved_tooltip_instances_live"',
                '"authority": "exact_a729_tooltip_controller"',
                'tooltip72::TooltipLineKind::Resolved => "resolved"',
                'tooltip72::RowAvailability::DataBlocked { field }',
                "frame_matches_snapshot",
            ]
        ),
    )
    check(
        "domain 72 fold tests frozen",
        "a26_domain72_folds_exact_instances_rows_geometry_and_provenance" in fold_tests
        and "a26_domain72_distinguishes_measured_empty_from_wrong_hour" in fold_tests,
    )
    tooltip_schema = replay_schema["$defs"]["resolvedTooltipInstances"]
    measured_schema = tooltip_schema["oneOf"][1]
    check(
        "production replay schema carrier",
        replay_schema["properties"]["resolved_tooltip_instances"]["$ref"]
        == "#/$defs/resolvedTooltipInstances"
        and measured_schema["properties"]["schema"]["const"]
        == "hoi4-resolved-tooltip-instances-v1"
        and {
            "producer_manifest_sha256",
            "completeness_record_identity",
            "expected_instance_count",
            "instances",
        }.issubset(measured_schema["required"]),
    )

    passed = sum(row["pass"] for row in checks)
    result = {
        "schema": "extractor-a26-resolved-tooltip-verification-v1",
        "status": "PASS_PRODUCTION_PROMOTION_READY_EXACT_A729_D72" if passed == len(checks) else "FAIL",
        "passed": passed,
        "total": len(checks),
        "checks": checks,
        "capture_oracle_derivations": raw_rows,
        "live_data_dependency": None,
        "human_input_required": [],
        "ghidra_provenance": {
            "function_address": "0x1402A4080",
            "job_id": "20260717T113739Z-a25-ctextbox-vtable-identity-accessors-7ddacf03",
            "artifact_path": "evidence/func_1402a4080.decompile.c",
            "artifact_sha256": "a1838600f7f016606ed6287926ef47d5b836a5501bc83ef59467c5dd61a1c071",
            "manifest_hash": "2bc2bc389cfcadb3903ad6c79a03645a5fe4b7102feac1eb982f89e6aabac9c5",
        },
    }
    (HERE / "verification.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": result["status"], "passed": passed, "total": len(checks)}, indent=2))
    raise SystemExit(0 if result["status"].startswith("PASS_") else 1)


if __name__ == "__main__":
    main()
