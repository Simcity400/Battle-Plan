#!/usr/bin/env python3
"""Fail-closed verifier for the H-A18 exact lifecycle acquisition bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

EXPECTED_SHA = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
EXPECTED_JOB = "20260720T101858Z-w5-a18-dmi2-synchronous-zero-removal-rec-845d4d32"
EXPECTED_MANIFEST = "9281a49e3f046984a699c3c42fd1ad172361d670aed9586631329255b038cd25"
EXPECTED_UPDATE_RVA = 0x54BEB0
EXPECTED_ERASE_RVA = 0x604D30
PHASES = ("positive_before", "positive_after", "post_removal")


def exact_int(value, minimum, maximum):
    return isinstance(value, int) and not isinstance(value, bool) and minimum <= value <= maximum


def hex_sha256(value):
    return isinstance(value, str) and len(value) == 64 and all(ch in "0123456789abcdef" for ch in value)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def find_forbidden_expiry(value, path="$"):
    findings = []
    if isinstance(value, dict):
        for key, child in value.items():
            child_path = "%s.%s" % (path, key)
            if key in ("absolute_expiry_hour", "inferred_expiry_hour", "expiry_hour"):
                findings.append(child_path)
            findings.extend(find_forbidden_expiry(child, child_path))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            findings.extend(find_forbidden_expiry(child, "%s[%d]" % (path, index)))
    return findings


def row_map(rows):
    if not isinstance(rows, list):
        raise ValueError("witness row set is not a list")
    out = {}
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError("witness row is not an object")
        modifier_id = row.get("modifier_id")
        if not isinstance(modifier_id, str) or not modifier_id or modifier_id in out:
            raise ValueError("witness modifier identity is absent or duplicated")
        if row.get("remaining_days") is not None and (
            not exact_int(row.get("remaining_days"), 0, (1 << 32) - 1)
        ):
            raise ValueError("witness remaining_days is not null|u32")
        if not isinstance(row.get("enabled"), bool) or not isinstance(row.get("values_raw"), list):
            raise ValueError("witness exact fields are malformed")
        if not all(exact_int(value, -(1 << 63), (1 << 63) - 1) for value in row["values_raw"]):
            raise ValueError("witness value vector is not signed i64")
        masked = row.get("pointer_masked_entry_bytes")
        if not isinstance(masked, list) or len(masked) != 0x40 or not all(
            exact_int(value, 0, 255) for value in masked
        ):
            raise ValueError("witness lacks the coherent pointer-masked 0x40-byte entry")
        for start in (0, 0x18, 0x28):
            if any(masked[start : start + 8]):
                raise ValueError("witness pointer slot was not redacted")
        days_raw = int.from_bytes(bytes(masked[0x10:0x14]), "little", signed=True)
        expected_days = -1 if row.get("remaining_days") is None else row["remaining_days"]
        if days_raw != expected_days:
            raise ValueError("witness masked bytes disagree with remaining_days")
        if masked[0x20] not in (0, 1) or bool(masked[0x20]) is not row["enabled"]:
            raise ValueError("witness masked bytes disagree with enabled")
        capacity = int.from_bytes(bytes(masked[0x30:0x34]), "little")
        count = int.from_bytes(bytes(masked[0x34:0x38]), "little")
        if count > capacity or count != len(row["values_raw"]):
            raise ValueError("witness masked value-vector header is inconsistent")
        out[modifier_id] = row
    return out


def same_witness_row(left, right):
    return (
        left.get("modifier_id") == right.get("modifier_id")
        and left.get("enabled") == right.get("enabled")
        and left.get("values_raw") == right.get("values_raw")
        and left.get("remaining_days") == right.get("remaining_days")
    )


def row_after_daily_decrement(row):
    row_map([row])
    remaining = row.get("remaining_days")
    next_days = None if remaining is None else max(0, remaining - 1)
    masked = list(row["pointer_masked_entry_bytes"])
    masked[0x10:0x14] = list((-1 if next_days is None else next_days).to_bytes(4, "little", signed=True))
    return {
        "modifier_id": row["modifier_id"],
        "enabled": row["enabled"],
        "values_raw": list(row["values_raw"]),
        "remaining_days": next_days,
        "pointer_masked_entry_bytes": masked,
    }


def simulate_daily_transform(witness):
    before = witness.get("before")
    after = witness.get("after")
    row_map(before)
    row_map(after)
    current = [row_after_daily_decrement(row) for row in before]
    boundaries = witness.get("erase_boundaries")
    if not isinstance(boundaries, list):
        raise ValueError("hook erase-boundary list is malformed")
    prior_start = None
    retained_boundaries = []
    for boundary in boundaries:
        if not isinstance(boundary, dict):
            raise ValueError("hook erase boundary is malformed")
        start, end = boundary.get("start_index"), boundary.get("end_index")
        rows = boundary.get("rows_before_erase")
        if (
            not exact_int(start, 0, (1 << 32) - 1)
            or not exact_int(end, 1, (1 << 32) - 1)
            or end != start + 1
            or end > len(current)
            or (prior_start is not None and start >= prior_start)
        ):
            raise ValueError("hook erase calls are not reverse-order singleton ranges")
        row_map(rows)
        if len(rows) != 1 or not same_witness_row(current[start], rows[0]):
            raise ValueError("hook erased row is not the exact current post-decrement vector slice")
        retained_boundaries.append(rows[0])
        del current[start:end]
        prior_start = start
    if len(current) != len(after) or any(
        not same_witness_row(expected, observed)
        for expected, observed in zip(current, after)
    ):
        raise ValueError("hook after-vector is not the exact decrement-plus-erase result")
    return {
        "before": row_map(before),
        "after": row_map(after),
        "erased": row_map(retained_boundaries),
    }


def join_map(checkpoint):
    joins = checkpoint.get("joins")
    if not isinstance(joins, list):
        raise ValueError("checkpoint joins are absent")
    out = {}
    for join in joins:
        modifier_id = join.get("modifier_id") if isinstance(join, dict) else None
        if not isinstance(modifier_id, str) or modifier_id in out:
            raise ValueError("checkpoint join identity is absent or duplicated")
        if join.get("accepted") is not True:
            raise ValueError("checkpoint contains a rejected join")
        if "remaining_days" in join:
            checks = join.get("checks")
            if not isinstance(checks, dict) or any(
                checks.get(field) is not True
                for field in ("enabled_equal", "values_fixed_five_equal", "remaining_days_equal")
            ):
                raise ValueError("checkpoint exact-field join checks are incomplete")
            if not isinstance(join.get("enabled"), bool):
                raise ValueError("checkpoint enabled field is malformed")
            if join.get("value_scale") != 100_000:
                raise ValueError("checkpoint value scale is not exact fixed-five")
            if not isinstance(join.get("values_raw"), list) or not all(
                exact_int(value, -(1 << 63), (1 << 63) - 1)
                for value in join["values_raw"]
            ):
                raise ValueError("checkpoint values are not signed fixed-five i64")
            days = join.get("remaining_days")
            if days is not None and not exact_int(days, 0, (1 << 32) - 1):
                raise ValueError("checkpoint remaining_days is not null|u32")
        out[modifier_id] = join
    return out


def same_exact_instance(left, right):
    return (
        left.get("modifier_id") == right.get("modifier_id")
        and left.get("enabled") == right.get("enabled")
        and left.get("values_raw") == right.get("values_raw")
    )


def process_key(identity):
    if not isinstance(identity, dict):
        raise ValueError("process identity is absent")
    pid = identity.get("pid")
    created = identity.get("create_time_100ns")
    sha = identity.get("executable_sha256")
    if not exact_int(pid, 1, (1 << 32) - 1) or not exact_int(created, 1, (1 << 64) - 1) or sha != EXPECTED_SHA:
        raise ValueError("process identity is incomplete or not exact a729")
    return pid, created, sha


def exact_target_chain(models, start_index, owner_id, timed_id, permanent_id):
    initial_witness, initial_model = models[start_index]
    expected_timed = initial_model["after"].get(timed_id)
    permanent = initial_model["after"].get(permanent_id)
    if expected_timed is None or permanent is None or permanent.get("remaining_days") is not None:
        return None
    prior_hour = initial_witness["game_hour_raw_after"]
    steps = 1
    for witness, model in models[start_index + 1 :]:
        if witness.get("owner_tag_db_id") != owner_id:
            continue
        if (
            witness.get("thread_id") != initial_witness.get("thread_id")
            or witness.get("game_hour_raw_before") != witness.get("game_hour_raw_after")
            or witness.get("game_hour_raw_before") - prior_hour != 24
        ):
            return None
        timed_before = model["before"].get(timed_id)
        permanent_before = model["before"].get(permanent_id)
        permanent_after = model["after"].get(permanent_id)
        if (
            timed_before is None
            or not same_witness_row(timed_before, expected_timed)
            or permanent_before is None
            or permanent_after is None
            or not same_witness_row(permanent_before, permanent)
            or not same_witness_row(permanent_after, permanent)
        ):
            return None
        decremented = row_after_daily_decrement(timed_before)
        steps += 1
        if timed_before.get("remaining_days") == 1:
            if (
                timed_id in model["after"]
                or not any(same_witness_row(row, decremented) for row in model["erased"].values())
            ):
                return None
            return {
                "zero_witness": witness,
                "daily_steps": steps,
                "first_hour": initial_witness["game_hour_raw_after"],
                "zero_hour": witness["game_hour_raw_after"],
            }
        timed_after = model["after"].get(timed_id)
        if timed_after is None or not same_witness_row(timed_after, decremented):
            return None
        expected_timed = timed_after
        prior_hour = witness["game_hour_raw_after"]
    return None


def verify_documents(checkpoints, hook, manifest):
    errors = []
    details = {}
    try:
        if len(checkpoints) != 3:
            raise ValueError("exactly three checkpoints are required")
        by_phase = {}
        for checkpoint in checkpoints:
            if checkpoint.get("schema") != "hoi4.a18.dynamic_modifier_save_live_checkpoint.v1":
                raise ValueError("checkpoint schema mismatch")
            phase = checkpoint.get("phase")
            if phase not in PHASES or phase in by_phase:
                raise ValueError("checkpoint phases are missing or duplicated")
            if checkpoint.get("accepted") is not True or checkpoint.get("same_hour") is not True:
                raise ValueError("checkpoint is not accepted same-hour evidence")
            live = checkpoint.get("live", {})
            save = checkpoint.get("save", {})
            if live.get("game_hour_raw_before") != live.get("game_hour_raw_after"):
                raise ValueError("checkpoint live bracket changed hour")
            if live.get("game_hour_raw_before") != save.get("game_hour_raw"):
                raise ValueError("checkpoint save/live hour differs")
            if not hex_sha256(save.get("sha256")):
                raise ValueError("checkpoint save hash is absent")
            join_map(checkpoint)
            by_phase[phase] = checkpoint
        if set(by_phase) != set(PHASES):
            raise ValueError("all three checkpoint phases are required")

        target = by_phase["positive_before"].get("target")
        if not isinstance(target, dict):
            raise ValueError("checkpoint target is absent")
        permanent_id, timed_id = target.get("permanent_modifier"), target.get("timed_modifier")
        if not isinstance(permanent_id, str) or not permanent_id or not isinstance(timed_id, str) or not timed_id or permanent_id == timed_id:
            raise ValueError("target modifier identities are invalid")
        owner_tag = by_phase["positive_before"].get("live", {}).get("owner_tag")
        owner_id = by_phase["positive_before"].get("live", {}).get("owner_tag_db_id")
        if not isinstance(owner_tag, str) or len(owner_tag) != 3 or not owner_tag.isascii() or not owner_tag.isalnum() or owner_tag != owner_tag.upper() or not exact_int(owner_id, 1, (1 << 32) - 1):
            raise ValueError("owner identity is invalid")

        identities = []
        save_hashes = set()
        for phase in PHASES:
            checkpoint = by_phase[phase]
            if checkpoint.get("target") != target:
                raise ValueError("checkpoint target changed")
            if checkpoint.get("live", {}).get("owner_tag") != owner_tag or checkpoint.get("live", {}).get("owner_tag_db_id") != owner_id:
                raise ValueError("checkpoint owner changed")
            identities.append(process_key(checkpoint.get("process_identity")))
            save_hashes.add(checkpoint["save"]["sha256"])
        if len(set(identities)) != 1:
            raise ValueError("checkpoints do not belong to one process instance")
        if len(save_hashes) != 3:
            raise ValueError("checkpoint saves are not three content-distinct states")
        phase_hours = [by_phase[phase]["save"]["game_hour_raw"] for phase in PHASES]
        if not all(exact_int(hour, -(1 << 31), (1 << 31) - 1) for hour in phase_hours):
            raise ValueError("checkpoint game hours are malformed")
        if by_phase["positive_before"].get("continuity") != {"kind": "fresh_baseline"}:
            raise ValueError("positive-before checkpoint is not a fresh baseline")
        for phase, previous_phase in (
            ("positive_after", "positive_before"),
            ("post_removal", "positive_after"),
        ):
            continuity = by_phase[phase].get("continuity")
            previous = by_phase[previous_phase]
            if (
                not isinstance(continuity, dict)
                or continuity.get("kind") != "exact_prior_checkpoint"
                or continuity.get("previous_phase") != previous_phase
                or continuity.get("previous_save_sha256") != previous["save"]["sha256"]
                or continuity.get("previous_game_hour_raw") != previous["save"]["game_hour_raw"]
                or continuity.get("game_hour_delta")
                != by_phase[phase]["save"]["game_hour_raw"] - previous["save"]["game_hour_raw"]
            ):
                raise ValueError("checkpoint continuity binding is incomplete")
        first_delta = phase_hours[1] - phase_hours[0]
        if not 0 < first_delta <= 24 or phase_hours[2] < phase_hours[1]:
            raise ValueError("checkpoint phases do not bracket the next daily decrement in order")

        before_joins = join_map(by_phase["positive_before"])
        after_joins = join_map(by_phase["positive_after"])
        removed_joins = join_map(by_phase["post_removal"])
        for joins in (before_joins, after_joins, removed_joins):
            if permanent_id not in joins or timed_id not in joins:
                raise ValueError("target joins are incomplete")
        before_timed, after_timed = before_joins[timed_id], after_joins[timed_id]
        if not exact_int(before_timed.get("remaining_days"), 2, (1 << 32) - 1):
            raise ValueError("positive-before timed counter must be at least two")
        if after_timed.get("remaining_days") != before_timed["remaining_days"] - 1:
            raise ValueError("save/live checkpoints do not show exact N-to-N-minus-one")
        if not same_exact_instance(before_timed, after_timed):
            raise ValueError("timed modifier identity/enabled/value changed across decrement")
        if before_joins[permanent_id].get("remaining_days") is not None or after_joins[permanent_id].get("remaining_days") is not None:
            raise ValueError("permanent modifier did not retain the -1/null sentinel")
        if not same_exact_instance(before_joins[permanent_id], after_joins[permanent_id]):
            raise ValueError("permanent modifier changed across decrement")
        if removed_joins[permanent_id].get("remaining_days") is not None or not same_exact_instance(
            after_joins[permanent_id], removed_joins[permanent_id]
        ):
            raise ValueError("permanent modifier did not remain exact through timed removal")
        if not (removed_joins[timed_id].get("absent_live") is True and removed_joins[timed_id].get("absent_save") is True):
            raise ValueError("post-removal checkpoint does not prove live/save absence")

        if hook.get("schema") != "hoi4.a18.dynamic_modifier_lifecycle_witness.v1":
            raise ValueError("hook receipt schema mismatch")
        if hook.get("executable_sha256") != EXPECTED_SHA or hook.get("exact20_job_id") != EXPECTED_JOB or hook.get("exact20_manifest_sha256") != EXPECTED_MANIFEST:
            raise ValueError("hook receipt exact-a729 authority mismatch")
        if hook.get("update_function_rva") != EXPECTED_UPDATE_RVA or hook.get("erase_function_rva") != EXPECTED_ERASE_RVA:
            raise ValueError("hook receipt function RVA mismatch")
        if hook.get("evidence_install") is not True or not exact_int(hook.get("hook_epoch"), 1, (1 << 64) - 1):
            raise ValueError("hook receipt is not an evidence installation")
        if hook.get("production_admission_ready") is not False:
            raise ValueError("raw hook receipt illegally claims production admission")
        if process_key(hook.get("process_identity")) != identities[0]:
            raise ValueError("hook receipt and checkpoints are from different process instances")
        if not isinstance(manifest, dict) or manifest.get("schema") != "native_collector_session_v1":
            raise ValueError("capture manifest is absent or malformed")
        if manifest.get("pid") != identities[0][0]:
            raise ValueError("capture manifest pid does not match the observed process")
        modset = manifest.get("modset")
        if not isinstance(modset, dict) or modset.get("resolved") is not True or modset.get("errors") != []:
            raise ValueError("capture manifest modset/executable preflight is unresolved")
        game = modset.get("game")
        if not isinstance(game, dict) or game.get("exe_sha256") != EXPECTED_SHA:
            raise ValueError("capture manifest executable SHA-256 mismatch")
        if manifest.get("scheduled_domains") != ["country_dynamic_modifiers_live"]:
            raise ValueError("capture manifest does not preserve the exact A18-only domain roster")
        compact = manifest.get("compact_stream")
        if not isinstance(compact, dict) or compact.get("decodable") is not True:
            raise ValueError("capture manifest compact stream is not decodable")
        if manifest.get("collector_control_errors") != [] or manifest.get("parse_errors") != []:
            raise ValueError("capture manifest reports collector or parse errors")
        diagnostics = hook.get("diagnostics")
        if not isinstance(diagnostics, dict):
            raise ValueError("hook diagnostics are absent")
        for field in (
            "retained_updates",
            "ignored_without_timed_instance",
            "dropped_reentrant",
            "dropped_unreadable",
            "dropped_lock_busy",
            "dropped_overflow",
        ):
            if not exact_int(diagnostics.get(field), 0, (1 << 64) - 1):
                raise ValueError("hook diagnostic is malformed: %s" % field)
        for field in ("dropped_reentrant", "dropped_unreadable", "dropped_lock_busy", "dropped_overflow"):
            if diagnostics.get(field) != 0:
                raise ValueError("hook completeness loss: %s" % field)
        if diagnostics.get("completeness_poisoned") is not False:
            raise ValueError("hook completeness was poisoned")
        witnesses = hook.get("witnesses")
        if not isinstance(witnesses, list) or not witnesses:
            raise ValueError("hook retained no lifecycle witnesses")
        if diagnostics.get("retained_updates") != len(witnesses):
            raise ValueError("hook retained-update count does not match witness count")
        sequences = [witness.get("sequence") for witness in witnesses if isinstance(witness, dict)]
        if len(sequences) != len(witnesses) or not all(exact_int(value, 1, (1 << 64) - 1) for value in sequences) or sequences != sorted(set(sequences)):
            raise ValueError("hook witness sequence is not strict")

        models = []
        decrement_candidates = []
        for witness in witnesses:
            if not exact_int(witness.get("thread_id"), 1, (1 << 32) - 1):
                raise ValueError("hook witness thread id is malformed")
            if not exact_int(witness.get("owner_tag_db_id"), 1, (1 << 32) - 1):
                raise ValueError("hook witness owner id is malformed")
            if not exact_int(witness.get("game_hour_raw_before"), -(1 << 31), (1 << 31) - 1) or not exact_int(
                witness.get("game_hour_raw_after"), -(1 << 31), (1 << 31) - 1
            ):
                raise ValueError("hook witness game hour is malformed")
            if witness.get("game_hour_raw_before") != witness.get("game_hour_raw_after"):
                raise ValueError("hook witness changed game hour inside update")
            model = simulate_daily_transform(witness)
            models.append((witness, model))
            if witness.get("owner_tag_db_id") != owner_id:
                continue
            before_rows, after_rows = model["before"], model["after"]
            permanent_before, permanent_after = before_rows.get(permanent_id), after_rows.get(permanent_id)
            if (
                permanent_before is None
                or permanent_after is None
                or permanent_before.get("remaining_days") is not None
                or permanent_after.get("remaining_days") is not None
                or not same_witness_row(permanent_before, permanent_after)
            ):
                continue
            timed_before, timed_after = before_rows.get(timed_id), after_rows.get(timed_id)
            if (
                timed_before is not None
                and timed_after is not None
                and timed_before.get("remaining_days") == before_timed["remaining_days"]
                and timed_after.get("remaining_days") == after_timed["remaining_days"]
                and timed_after["remaining_days"] == timed_before["remaining_days"] - 1
                and witness.get("game_hour_raw_after") == phase_hours[1]
                and same_exact_instance(timed_before, timed_after)
            ):
                decrement_candidates.append(len(models) - 1)
        if not decrement_candidates:
            raise ValueError("hook receipt lacks exact timed N-to-N-minus-one witness")
        chain = None
        decrement_index = None
        for candidate in decrement_candidates:
            candidate_chain = exact_target_chain(models, candidate, owner_id, timed_id, permanent_id)
            if candidate_chain is not None:
                decrement_index = candidate
                chain = candidate_chain
                break
        if chain is None or decrement_index is None:
            raise ValueError(
                "hook receipt lacks a continuous daily N-to-N-minus-one-through-one-to-zero-to-removal chain"
            )
        decrement_witness = models[decrement_index][0]
        zero_witness = chain["zero_witness"]
        if decrement_witness["game_hour_raw_after"] != phase_hours[1]:
            raise ValueError("positive-after checkpoint is not bound to the decrement witness hour")
        if by_phase["post_removal"]["save"]["game_hour_raw"] < zero_witness["game_hour_raw_after"]:
            raise ValueError("post-removal save predates the zero/removal witness")
        forbidden = find_forbidden_expiry({"checkpoints": checkpoints, "hook": hook})
        if forbidden:
            raise ValueError("forbidden synthesized expiry field: %s" % forbidden[0])

        details = {
            "owner_tag": owner_tag,
            "owner_tag_db_id": owner_id,
            "permanent_modifier": permanent_id,
            "timed_modifier": timed_id,
            "process_instance": {
                "pid": identities[0][0],
                "create_time_100ns": identities[0][1],
                "executable_sha256": identities[0][2],
            },
            "checkpoint_days": [before_timed["remaining_days"], after_timed["remaining_days"]],
            "decrement_witness_sequence": decrement_witness["sequence"],
            "zero_removal_witness_sequence": zero_witness["sequence"],
            "continuous_daily_steps": chain["daily_steps"],
            "continuous_daily_hour_span": chain["zero_hour"] - chain["first_hour"],
            "ordered_vector_transform_verified": True,
        }
    except (KeyError, TypeError, ValueError) as exc:
        errors.append(str(exc))
    return not errors, errors, details


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkpoint", action="append", required=True)
    parser.add_argument("--hook-receipt", required=True)
    parser.add_argument("--capture-manifest", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    checkpoint_paths = [Path(value).resolve() for value in args.checkpoint]
    hook_path = Path(args.hook_receipt).resolve()
    manifest_path = Path(args.capture_manifest).resolve()
    checkpoints = [json.loads(path.read_text(encoding="utf-8")) for path in checkpoint_paths]
    hook = json.loads(hook_path.read_text(encoding="utf-8"))
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    accepted, errors, details = verify_documents(checkpoints, hook, manifest)
    receipt = {
        "schema": "hoi4.a18.dynamic_modifier_fixture_verification.v1",
        "status": "PASS" if accepted else "FAIL",
        "production_admission_ready": accepted,
        "checks": {
            "same_process_exact_a729": accepted,
            "same_hour_save_live_fields": accepted,
            "permanent_minus_one": accepted,
            "timed_n_to_n_minus_one": accepted,
            "synchronous_zero_then_removal": accepted,
            "post_removal_save_live_absence": accepted,
            "no_synthesized_expiry": accepted,
            "pointer_free_promoted_receipt": accepted,
        },
        "details": details,
        "errors": errors,
        "inputs": [
            {"kind": "checkpoint", "sha256": sha256_file(path)} for path in checkpoint_paths
        ] + [
            {"kind": "hook_receipt", "sha256": sha256_file(hook_path)},
            {"kind": "capture_manifest", "sha256": sha256_file(manifest_path)},
        ],
    }
    out = Path(args.out).resolve()
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print("A18 fixture %s: %s" % (receipt["status"], out))
    return 0 if accepted else 1


if __name__ == "__main__":
    raise SystemExit(main())
