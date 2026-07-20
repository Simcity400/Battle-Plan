#!/usr/bin/env python3
"""Verify one raw A38 live-probe capture without promoting it to d84 authority."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


EXPECTED_EXECUTABLE = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
EXPECTED_JOB = "20260720T020812Z-w3-a38-controller-owner-41ee5208"
EXPECTED_RECEIPT = "7d0a36b1822faf85a949d628a79bc4a09f95dc9c9aa3ed4aecb715b72b0a9082"
REPO_ROOT = Path(__file__).resolve().parents[4]
EXPECTED_VECTORS = {
    "effect_enabled_widgets",
    "visible_widgets_primary",
    "visible_widgets_secondary",
    "dynamic_text_primary",
    "dynamic_text_secondary",
    "dynamic_image_or_frame",
    "dynamic_position_or_size",
    "resolved_property_pairs",
}


def lower_sha(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 64 and all(ch in "0123456789abcdef" for ch in value)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_sha(value: Any) -> str:
    payload = json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def resolve_receipt_path(value: str) -> Path:
    path = Path(value)
    return path if path.is_absolute() else REPO_ROOT / path


def verify(document: dict[str, Any], source: Path) -> dict[str, Any]:
    checks: list[dict[str, Any]] = []

    def check(name: str, passed: bool, detail: Any) -> None:
        checks.append({"name": name, "passed": bool(passed), "detail": detail})

    identity = document.get("identity") or {}
    authority = document.get("ghidra_authority") or {}
    registry = document.get("registry") or {}
    winner = document.get("effective_winner_index") or {}
    manager = document.get("validated_manager") or {}
    portable = document.get("portable_prerequisites") or {}
    updaters = manager.get("updaters") if isinstance(manager.get("updaters"), list) else []

    check("probe_schema", document.get("schema") == "extractor.a38_scripted_gui_runtime_live_probe.v1", document.get("schema"))
    check("read_only", document.get("read_only") is True, document.get("read_only"))
    check("exact_executable", str(identity.get("sha256", "")).lower() == EXPECTED_EXECUTABLE, identity)
    check(
        "exact20_authority",
        authority.get("promotion_allowed") is True
        and authority.get("job_id") == EXPECTED_JOB
        and authority.get("manifest_sha256") == EXPECTED_RECEIPT,
        authority,
    )

    registry_path = resolve_receipt_path(str(registry.get("path", "")))
    registry_ok = registry_path.is_file() and sha256_file(registry_path) == registry.get("sha256")
    check("registry_bytes", registry_ok, {"path": str(registry_path), "declared": registry.get("sha256")})
    registry_rows = json.loads(registry_path.read_text(encoding="utf-8")).get("scripted_guis", {}) if registry_ok else {}

    winner_path = resolve_receipt_path(str(winner.get("path", "")))
    winner_ok = winner_path.is_file() and sha256_file(winner_path) == winner.get("sha256")
    check("effective_winner_bytes", winner_ok, {"path": str(winner_path), "declared": winner.get("sha256")})
    winner_document = json.loads(winner_path.read_text(encoding="utf-8")) if winner_ok else {}
    winner_rows = {
        row.get("content", {}).get("logical_path"): row.get("content", {})
        for row in winner_document.get("ordered_rows", [])
        if isinstance(row, dict) and isinstance(row.get("content"), dict)
    }
    check("ui14_engine_vfs_bound", lower_sha(document.get("ui14_engine_vfs_sha256")), document.get("ui14_engine_vfs_sha256"))

    count = manager.get("updater_count")
    check("singleton_validated_manager", manager.get("status") == "validated", manager.get("status"))
    check("nonempty_complete_manager", isinstance(count, int) and count > 0 and count == len(updaters), {"declared": count, "rows": len(updaters)})

    row_failures: list[str] = []
    portable_rows = portable.get("rows") if isinstance(portable.get("rows"), list) else []
    for index, updater in enumerate(updaters):
        if updater.get("manager_order_index") != index or updater.get("status") != "validated":
            row_failures.append(f"row:{index}:identity")
        owner = updater.get("owner_country_tag")
        if not isinstance(owner, str) or len(owner) != 3 or not owner.isascii() or not owner.isalnum() or owner != owner.upper():
            row_failures.append(f"row:{index}:owner")
        definition = updater.get("portable_definition_identity") or {}
        if definition.get("portable_identity_complete") is not True:
            row_failures.append(f"row:{index}:definition")
        gui_id = updater.get("matched_registry_gui_id")
        registry_row = registry_rows.get(gui_id) if isinstance(registry_rows, dict) else None
        source_file = definition.get("source_file")
        if (
            not isinstance(registry_row, dict)
            or registry_row.get("source_file") != source_file
            or registry_row.get("context_type") != definition.get("context_type")
            or registry_row.get("window_name") != definition.get("window_name")
        ):
            row_failures.append(f"row:{index}:registry_rejoin")
        if winner_rows.get(source_file) != definition.get("effective_vfs_winner"):
            row_failures.append(f"row:{index}:winner_rejoin")
        vectors = updater.get("vectors") or {}
        if set(vectors) != EXPECTED_VECTORS:
            row_failures.append(f"row:{index}:vector_inventory")
        for name, vector in vectors.items():
            if vector.get("status") not in ("measured", "measured_empty"):
                row_failures.append(f"row:{index}:vector:{name}:{vector.get('status')}")
    check("all_rows_stable_and_joined", not row_failures, row_failures)
    check("pointer_free_projection_complete", len(portable_rows) == len(updaters) and portable.get("structural_capture_ready") is True, {"rows": len(portable_rows), "ready": portable.get("structural_capture_ready"), "blockers": portable.get("blockers")})
    check("ready_for_runtime_compiler", portable.get("ready_for_runtime_result_compilation") is True, portable.get("blockers"))
    check(
        "raw_capture_not_self_promoted",
        portable.get("promotion_allowed") is False
        and set(portable.get("production_admission_blockers") or [])
        == {"complete_engine_result_frame_not_compiled", "in_process_h4ct_winner_authorization_required"},
        portable.get("production_admission_blockers"),
    )

    accepted = all(row["passed"] for row in checks)
    return {
        "schema": "extractor.a38_live_probe_verification.v1",
        "status": "ACCEPTED_RAW_CAPTURE" if accepted else "REJECTED",
        "accepted": accepted,
        "production_admission_ready": False,
        "production_admission_blockers": [
            "complete_engine_result_frame_not_compiled",
            "in_process_h4ct_winner_authorization_required",
        ],
        "source": {"path": str(source), "sha256": sha256_file(source)},
        "pointer_free_projection_sha256": canonical_sha(portable_rows) if portable_rows else None,
        "checks": checks,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("probe", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    document = json.loads(args.probe.read_text(encoding="utf-8"))
    result = verify(document, args.probe.resolve())
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"accepted": result["accepted"], "out": str(args.out)}, sort_keys=True))
    return 0 if result["accepted"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
