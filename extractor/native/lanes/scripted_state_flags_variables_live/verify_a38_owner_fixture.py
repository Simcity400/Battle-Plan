#!/usr/bin/env python3
"""Grade a same-process A38 diagnostic owner-fixture observation matrix.

This grader never promotes cold ReadProcessMemory observations to d84. It
proves only that the acquisition session exercised the lifecycle cases needed
to validate the in-process post-FUN_140B79C90 owner:

* stable non-empty manager;
* committed empty manager;
* generation-sentinel or retained-row/window unavailability; and
* a later stable snapshot that removes at least one prior manager key.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any

EXPECTED_EXECUTABLE = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
EXPECTED_PROBE_SCHEMA = "extractor.a38_scripted_gui_runtime_live_probe.v1"
EXPECTED_VERIFICATION_SCHEMA = "extractor.a38_live_probe_verification.v1"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path}: expected JSON object")
    return value


def resolve_from_bundle(bundle_path: Path, value: Any) -> Path | None:
    if not isinstance(value, str) or not value:
        return None
    path = Path(value)
    if path.is_absolute():
        return path
    return (bundle_path.parent / path).resolve()


def manager_keys(document: dict[str, Any]) -> list[tuple[str, str]] | None:
    manager = document.get("validated_manager")
    if not isinstance(manager, dict):
        return None
    rows = manager.get("updaters")
    if not isinstance(rows, list):
        return None
    keys: list[tuple[str, str]] = []
    for index, row in enumerate(rows):
        if (
            not isinstance(row, dict)
            or row.get("manager_order_index") != index
            or not isinstance(row.get("owner_country_tag"), str)
            or not isinstance(row.get("matched_registry_gui_id"), str)
        ):
            return None
        keys.append((row["owner_country_tag"], row["matched_registry_gui_id"]))
    return keys if len(keys) == len(set(keys)) else None


def unavailable_class(document: dict[str, Any]) -> str | None:
    manager = document.get("validated_manager")
    portable = document.get("portable_prerequisites")
    if not isinstance(manager, dict) or not isinstance(portable, dict):
        return None
    blockers = portable.get("blockers")
    blockers = blockers if isinstance(blockers, list) else []
    if (
        manager.get("rebuild_generation_committed") is not True
        or "manager_rebuild_generation_uncommitted" in blockers
    ):
        return "generation_unavailable"
    if any(
        isinstance(value, str)
        and value.startswith(("updater_window_not_live:", "updater_not_joined:"))
        for value in blockers
    ):
        return "retained_row_unavailable"
    return None


def verify(bundle_path: Path) -> dict[str, Any]:
    bundle = load_json(bundle_path)
    attempts = bundle.get("attempts")
    checks: list[dict[str, Any]] = []

    def check(name: str, passed: bool, detail: Any) -> None:
        checks.append({"name": name, "passed": bool(passed), "detail": detail})

    check(
        "bundle_schema",
        bundle.get("schema") == "extractor.a38_autonomous_acquisition.v1",
        bundle.get("schema"),
    )
    check(
        "exact_executable",
        bundle.get("executable_sha256") == EXPECTED_EXECUTABLE,
        bundle.get("executable_sha256"),
    )
    if not isinstance(attempts, list):
        attempts = []
    check("attempts_present", bool(attempts), len(attempts))

    rows: list[dict[str, Any]] = []
    process_ids: set[int] = set()
    for ordinal, attempt in enumerate(attempts):
        if not isinstance(attempt, dict):
            rows.append({"attempt": ordinal + 1, "class": "invalid_attempt"})
            continue
        raw_path = resolve_from_bundle(bundle_path, attempt.get("raw"))
        if raw_path is None or not raw_path.is_file():
            rows.append({"attempt": ordinal + 1, "class": "missing_raw"})
            continue
        raw = load_json(raw_path)
        identity = raw.get("identity") if isinstance(raw.get("identity"), dict) else {}
        pid = identity.get("pid")
        if isinstance(pid, int) and pid > 0:
            process_ids.add(pid)
        raw_identity_ok = (
            raw.get("schema") == EXPECTED_PROBE_SCHEMA
            and str(identity.get("sha256", "")).lower() == EXPECTED_EXECUTABLE
            and raw.get("read_only") is True
        )
        verification_path = resolve_from_bundle(bundle_path, attempt.get("verification"))
        verification = (
            load_json(verification_path)
            if verification_path is not None and verification_path.is_file()
            else {}
        )
        verification_checks = verification.get("checks")
        verification_ok = (
            verification.get("schema") == EXPECTED_VERIFICATION_SCHEMA
            and verification.get("status") == "ACCEPTED_RAW_CAPTURE"
            and verification.get("accepted") is True
            and verification.get("production_admission_ready") is False
            and isinstance(verification_checks, list)
            and bool(verification_checks)
            and all(
                isinstance(row, dict) and row.get("passed") is True
                for row in verification_checks
            )
            and isinstance(verification.get("source"), dict)
            and verification["source"].get("sha256") == sha256_file(raw_path)
        )
        keys = manager_keys(raw)
        manager = raw.get("validated_manager")
        manager = manager if isinstance(manager, dict) else {}
        if raw_identity_ok and verification_ok and keys is not None:
            classification = "stable_nonempty" if keys else "committed_empty"
        elif raw_identity_ok:
            classification = unavailable_class(raw) or "rejected_other"
        else:
            classification = "invalid_identity"
        rows.append(
            {
                "attempt": attempt.get("attempt", ordinal + 1),
                "class": classification,
                "pid": pid,
                "raw": str(raw_path),
                "raw_sha256": sha256_file(raw_path),
                "manager_status": manager.get("status"),
                "manager_keys": [
                    {"country_tag": country_tag, "gui_id": gui_id}
                    for country_tag, gui_id in (keys or [])
                ],
            }
        )

    stable_nonempty = [row for row in rows if row["class"] == "stable_nonempty"]
    committed_empty = [row for row in rows if row["class"] == "committed_empty"]
    unavailable = [
        row
        for row in rows
        if row["class"] in ("generation_unavailable", "retained_row_unavailable")
    ]
    removal: dict[str, Any] | None = None
    stable_rows = [
        row
        for row in rows
        if row["class"] in ("stable_nonempty", "committed_empty")
    ]
    for prior_index, prior in enumerate(stable_rows):
        prior_keys = {
            (row["country_tag"], row["gui_id"]) for row in prior["manager_keys"]
        }
        for current in stable_rows[prior_index + 1 :]:
            if prior.get("pid") != current.get("pid"):
                continue
            current_keys = {
                (row["country_tag"], row["gui_id"]) for row in current["manager_keys"]
            }
            removed = sorted(prior_keys - current_keys)
            if removed:
                removal = {
                    "prior_attempt": prior["attempt"],
                    "current_attempt": current["attempt"],
                    "removed": [
                        {"country_tag": country_tag, "gui_id": gui_id}
                        for country_tag, gui_id in removed
                    ],
                }
                break
        if removal is not None:
            break

    check("single_process", len(process_ids) == 1, sorted(process_ids))
    check("stable_nonempty_observed", bool(stable_nonempty), [row["attempt"] for row in stable_nonempty])
    check("committed_empty_observed", bool(committed_empty), [row["attempt"] for row in committed_empty])
    check(
        "unavailable_observed",
        bool(unavailable),
        [{"attempt": row["attempt"], "class": row["class"]} for row in unavailable],
    )
    check("prior_to_current_removal_observed", removal is not None, removal)
    accepted = all(row["passed"] for row in checks)
    return {
        "schema": "extractor.a38_owner_fixture_matrix_verification.v1",
        "status": "ACCEPTED_DIAGNOSTIC_OWNER_MATRIX" if accepted else "INCOMPLETE",
        "accepted": accepted,
        "production_admission_ready": False,
        "production_admission_blockers": [
            "post_FUN_140B79C90_in_process_owner_receipt_required",
            "complete_engine_result_frame_not_compiled",
            "synchronized_property_value_consumer_observations_required",
            "in_process_h4ct_winner_authorization_required",
        ],
        "source_bundle": {
            "path": str(bundle_path),
            "sha256": sha256_file(bundle_path),
        },
        "observations": rows,
        "removal": removal,
        "checks": checks,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    result = verify(args.bundle.resolve())
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps({"accepted": result["accepted"], "out": str(args.out)}, sort_keys=True))
    return 0 if result["accepted"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
