"""Fail-closed verifier for an A23 CProgram owner/save equality receipt."""

import argparse
import hashlib
import json
from pathlib import Path

EXPECTED_BINARY_SHA256 = (
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
)
EXPECTED_SCHEMA = "a23-program-owner-equality-probe-v1"
EXPECTED_ALIAS_IDS = [23, 24, 25, 27, 28]


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify(receipt, receipt_path):
    errors = []
    if receipt.get("schema") != EXPECTED_SCHEMA:
        errors.append("unexpected receipt schema")
    supported = receipt.get("supported_binary") or {}
    process = receipt.get("process") or {}
    if str(supported.get("sha256", "")).lower() != EXPECTED_BINARY_SHA256:
        errors.append("receipt does not pin exact a729")
    if str(process.get("observed_sha256", "")).lower() != EXPECTED_BINARY_SHA256:
        errors.append("observed executable is not exact a729")

    boundary = receipt.get("boundary") or {}
    if not boundary.get("same_raw_hour"):
        errors.append("live/save raw-hour equality is absent")
    if not boundary.get("double_rooted_walk_equal"):
        errors.append("the two rooted walks differ")

    gate = receipt.get("owner_gate") or {}
    counts = gate.get("counts") or {}
    if not gate.get("accepted") or not receipt.get("production_admission"):
        errors.append("owner equality gate is not admitted")
    if gate.get("problems"):
        errors.append("owner equality gate carries problems")
    for key, expected in (
        ("save_facilities", 26),
        ("live_distinct_programs", 26),
        ("compared_rows", 26),
        ("multiply_reachable_rows", 5),
        ("owner_primary_mirror_mismatches", 0),
        ("owner_save_mismatches", 0),
        ("province_save_mismatches", 0),
        ("payload_save_mismatches", 0),
        ("unresolved_owner_handles", 0),
        ("invalid_records", 0),
        ("invalid_programs", 0),
    ):
        if counts.get(key) != expected:
            errors.append(f"{key} != {expected}")
    if gate.get("required_multiply_reachable_program_ids") != EXPECTED_ALIAS_IDS:
        errors.append("required alias discriminator changed")
    if gate.get("observed_multiply_reachable_program_ids") != EXPECTED_ALIAS_IDS:
        errors.append("observed alias discriminator differs")
    for key in (
        "missing_live_save_program_ids",
        "missing_save_live_program_ids",
        "primary_mirror_mismatch_program_ids",
        "owner_mismatch_program_ids",
        "province_mismatch_program_ids",
        "payload_mismatch_program_ids",
        "unresolved_owner_program_ids",
        "ambiguous_owner_mismatch_program_ids",
        "missing_required_multiply_reachable_program_ids",
        "unexpected_multiply_reachable_program_ids",
    ):
        if gate.get(key) != []:
            errors.append(f"{key} is nonempty")
    controls = gate.get("negative_control_owner_match_counts") or {}
    if not controls or any(value == 26 for value in controls.values()):
        errors.append("adjacent owner negative-control rejection is absent")

    rows = receipt.get("pointer_free_rows")
    if not isinstance(rows, list) or len(rows) != 26:
        errors.append("pointer-free row census is not 26")
    else:
        ids = []
        for index, row in enumerate(rows):
            if any("ptr" in str(key).lower() or "address" in str(key).lower() for key in row):
                errors.append(f"row {index} contains a process pointer/address key")
            if not row.get("owner_fields_equal") or not row.get("province_fields_equal"):
                errors.append(f"row {index} lacks duplicated-field equality")
            if not row.get("resolved_owner_tag") or row.get("resolved_owner_tag") != row.get(
                "oracle_country"
            ):
                errors.append(f"row {index} owner/save join differs")
            ids.append(row.get("program_id"))
        if len(set(ids)) != 26 or any(not isinstance(value, int) or value <= 0 for value in ids):
            errors.append("pointer-free program ids are invalid or duplicated")

    save_path_text = receipt.get("save_projects_json")
    if not isinstance(save_path_text, str) or not save_path_text:
        errors.append("save oracle path is absent")
    else:
        save_path = Path(save_path_text)
        if not save_path.is_absolute():
            save_path = (receipt_path.parent / save_path).resolve()
        if not save_path.is_file():
            errors.append("save oracle file is absent")
        else:
            # Bind the verifier output to the exact bytes used by the probe.
            receipt["_verified_save_projects_sha256"] = sha256(save_path)
    return errors


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("receipt")
    parser.add_argument("--out")
    args = parser.parse_args(argv)
    path = Path(args.receipt).resolve()
    receipt = json.loads(path.read_text(encoding="utf-8"))
    errors = verify(receipt, path)
    result = {
        "schema": "a23-program-owner-equality-verification-v1",
        "receipt": str(path),
        "receipt_sha256": sha256(path),
        "save_projects_sha256": receipt.get("_verified_save_projects_sha256"),
        "production_admission": not errors,
        "errors": errors,
    }
    text = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.out:
        Path(args.out).write_text(text, encoding="utf-8")
    print(text, end="")
    raise SystemExit(0 if not errors else 1)


if __name__ == "__main__":
    main()
