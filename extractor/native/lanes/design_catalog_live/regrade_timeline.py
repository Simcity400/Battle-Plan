#!/usr/bin/env python3
"""Independent offline grader for repeated A27 whole-sweep timelines."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path

SCHEMA = "extractor.a27.live_heap_catalog_timeline.a729.v1"
PROGRAM_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
PROGRAM = "hoi4.exe 1.19.2.0.a729"
DATE_EPOCH_RAW = 43_799_999
AUTHENTICATED_D04_TARGET_STABLE_ID = "371:52"


def canonical_bytes(value: object) -> bytes:
    return json.dumps(
        value, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")


def write_create_new_json(path: Path, value: object) -> None:
    """Durably publish an immutable report without replacing prior evidence."""
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        raise FileExistsError(f"refusing to replace A27 regrade receipt: {path}")
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    encoded = json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2).encode(
        "utf-8"
    )
    try:
        with temporary.open("xb") as stream:
            stream.write(encoded)
            stream.flush()
            os.fsync(stream.fileno())
        os.rename(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def stable_rows(snapshot: dict) -> dict[str, dict]:
    rows = {}
    for key, runtime_type in (("division_templates", 52), ("variants", 70)):
        source = snapshot.get(key)
        if not isinstance(source, list):
            raise ValueError(f"{key} is not an array")
        for ordinal, row in enumerate(source):
            if row.get("type") != runtime_type or row.get("catalog_scan_ordinal") != ordinal:
                raise ValueError(f"{key}/{ordinal} type or catalog ordinal is invalid")
            stable_id = f"{row.get('id')}:{runtime_type}"
            if stable_id in rows:
                raise ValueError(f"duplicate stable design identity {stable_id}")
            rows[stable_id] = row
    return rows


def pointer_free(value):
    if isinstance(value, list):
        return [pointer_free(item) for item in value]
    if isinstance(value, dict):
        return {
            key: pointer_free(item)
            for key, item in value.items()
            if key != "pointer_identity"
            and not key.endswith("_pointer")
            and key not in {"pid", "module_base", "game_state"}
        }
    return value


def semantic_row(row: dict) -> dict:
    """Native row identity without relocation or scan-order bookkeeping."""
    return {
        key: value
        for key, value in pointer_free(row).items()
        if key != "catalog_scan_ordinal"
    }


def changed_paths(before: object, after: object, prefix: str = "") -> list[str]:
    """Return deterministic leaf paths changed by one native-row upsert."""
    if isinstance(before, dict) and isinstance(after, dict):
        paths = []
        for key in sorted(set(before) | set(after)):
            path = f"{prefix}.{key}" if prefix else key
            if key not in before or key not in after:
                paths.append(path)
            else:
                paths.extend(changed_paths(before[key], after[key], path))
        return paths
    if before != after:
        return [prefix]
    return []


def classify_mutation(before: dict, after: dict) -> dict:
    paths = changed_paths(semantic_row(before), semantic_row(after))
    before_obsolete = before.get("obsolete")
    after_obsolete = after.get("obsolete")
    before_definition = before.get("definition", {})
    after_definition = after.get("definition", {})
    name_paths = {
        "definition.name",
        "definition.name_utf8_hex",
        "definition.display_name",
        "definition.display_name_utf8_hex",
        "definition.variant_name",
        "definition.variant_name_utf8_hex",
    }
    classes = []
    if any(path in name_paths for path in paths):
        classes.append("rename")
    if before_obsolete is not True and after_obsolete is True:
        classes.append("obsolete")
    if before_obsolete is True and after_obsolete is not True:
        classes.append("reactivate")
    if any(path not in name_paths | {"obsolete", "definition.obsolete"} for path in paths):
        classes.append("edit")
    return {
        "stable_id": f"{after['id']}:{after['type']}",
        "classes": classes,
        "changed_paths": paths,
        "before_name": before_definition.get("name")
        or before_definition.get("display_name")
        or before_definition.get("variant_name"),
        "after_name": after_definition.get("name")
        or after_definition.get("display_name")
        or after_definition.get("variant_name"),
        "before_obsolete": before_obsolete,
        "after_obsolete": after_obsolete,
    }


def apply_delta(previous: dict, delta: dict) -> dict:
    rows = {
        f"{row['id']}:{row['type']}": row
        for row in previous["division_templates"] + previous["variants"]
    }
    tombstones = delta.get("variant_tombstones", []) + delta.get(
        "template_tombstones", []
    )
    if len(tombstones) != len(set(tombstones)):
        raise ValueError("delta contains duplicate tombstones")
    for stable_id in tombstones:
        if rows.pop(stable_id, None) is None:
            raise ValueError(f"delta tombstones absent design {stable_id}")
    upserts = delta.get("variant_upserts", []) + delta.get("template_upserts", [])
    upsert_ids = [f"{row['id']}:{row['type']}" for row in upserts]
    if len(upsert_ids) != len(set(upsert_ids)) or set(upsert_ids) & set(tombstones):
        raise ValueError("delta contains duplicate/conflicting upserts")
    rows.update(zip(upsert_ids, upserts))
    frame = dict(delta["frame_metadata"])
    templates = []
    variants = []
    for stable_id in delta["ordered_design_ids"]:
        row = dict(rows.pop(stable_id))
        if row["type"] == 52:
            row["catalog_scan_ordinal"] = len(templates)
            templates.append(row)
        elif row["type"] == 70:
            row["catalog_scan_ordinal"] = len(variants)
            variants.append(row)
        else:
            raise ValueError(f"delta order has wrong-typed design {stable_id}")
    if rows:
        raise ValueError("delta order omits reconstructed designs")
    frame["division_templates"] = templates
    frame["variants"] = variants
    return frame


def grade(
    timeline: dict,
    first_hour: int | None,
    last_hour: int | None,
    require_mutation: bool = False,
    minimum_captures: int = 1,
    action_receipt: dict | None = None,
    required_transactions: tuple[str, ...] = (),
) -> dict:
    errors = []
    transitions = []
    if timeline.get("schema") != SCHEMA:
        errors.append("timeline schema mismatch")
    authority = timeline.get("authority", {})
    if authority.get("program") != PROGRAM or authority.get("program_sha256") != PROGRAM_SHA256:
        errors.append("program identity mismatch")
    if timeline.get("complete_sequence") is not True:
        errors.append("timeline was not sealed complete")
    if timeline.get("transport") != "first_full_then_native_row_deltas":
        errors.append("timeline transport mismatch")
    if timeline.get("fatal") is not None:
        errors.append(f"timeline producer reported fatal state: {timeline.get('fatal')}")
    captures = timeline.get("captures")
    if not isinstance(captures, list) or not captures:
        errors.append("timeline has no captures")
        captures = []
    if len(captures) < minimum_captures:
        errors.append(
            f"capture count {len(captures)} is below required {minimum_captures}"
        )
    previous_hour = None
    previous_rows = None
    previous_snapshot = None
    for sequence, capture in enumerate(captures):
        if capture.get("sequence") != sequence:
            errors.append(f"capture {sequence}: sequence invalid")
            continue
        if not isinstance(capture.get("same_hour_rechecks"), int) or capture[
            "same_hour_rechecks"
        ] < 1:
            errors.append(f"capture {sequence}: same-hour mutation recheck missing")
        recheck_hashes = capture.get("same_hour_recheck_sha256")
        if (
            not isinstance(recheck_hashes, list)
            or len(recheck_hashes) != capture.get("same_hour_rechecks")
            or any(value != capture.get("snapshot_sha256") for value in recheck_hashes)
        ):
            errors.append(f"capture {sequence}: same-hour recheck hashes are invalid")
        try:
            if sequence == 0:
                snapshot = capture.get("snapshot")
                if not isinstance(snapshot, dict) or capture.get("delta") is not None:
                    raise ValueError("baseline is not one full snapshot")
            else:
                if capture.get("snapshot") is not None or not isinstance(
                    capture.get("delta"), dict
                ):
                    raise ValueError("non-baseline capture is not one delta")
                snapshot = apply_delta(previous_snapshot, capture["delta"])
        except (KeyError, TypeError, ValueError) as error:
            errors.append(f"capture {sequence}: {error}")
            continue
        digest = hashlib.sha256(canonical_bytes(snapshot)).hexdigest()
        if capture.get("snapshot_sha256") != digest:
            errors.append(f"capture {sequence}: snapshot hash mismatch")
        hour = capture.get("capture_hour_index")
        snapshot_authority = snapshot.get("authority", {})
        if (
            not isinstance(hour, int)
            or snapshot_authority.get("program") != PROGRAM
            or snapshot_authority.get("program_sha256") != PROGRAM_SHA256
            or snapshot_authority.get("capture_hour_index") != hour
            or snapshot_authority.get("current_date_raw") != DATE_EPOCH_RAW + hour
            or snapshot_authority.get("coherence")
            != "double_read_all_roots_fields_and_membership"
            or snapshot.get("full_replacement") is not True
            or snapshot.get("completeness")
            != {
                "definitions": True,
                "lifecycle": True,
                "membership": True,
                "owner": True,
                "ui_occurrences": True,
            }
            or not isinstance(snapshot.get("countries"), list)
            or snapshot.get("country_count") != len(snapshot["countries"])
        ):
            errors.append(f"capture {sequence}: authority/hour/completeness identity invalid")
            continue
        if previous_hour is not None and hour != previous_hour + 1:
            errors.append(f"capture {sequence}: replay-hour gap {previous_hour}->{hour}")
        try:
            rows = stable_rows(snapshot)
        except ValueError as error:
            errors.append(f"capture {sequence}: {error}")
            continue
        if previous_rows is not None:
            created = sorted(set(rows) - set(previous_rows))
            tombstones = sorted(set(previous_rows) - set(rows))
            mutated = sorted(
                stable_id
                for stable_id in set(rows) & set(previous_rows)
                if semantic_row(rows[stable_id])
                != semantic_row(previous_rows[stable_id])
            )
            mutation_details = [
                classify_mutation(previous_rows[stable_id], rows[stable_id])
                for stable_id in mutated
            ]
            transitions.append(
                {
                    "sequence": sequence,
                    "from_hour": previous_hour,
                    "to_hour": hour,
                    "created": created,
                    "tombstones": tombstones,
                    "mutated": mutated,
                    "mutation_details": mutation_details,
                    "ordered_design_ids": list(rows),
                }
            )
        previous_hour = hour
        previous_rows = rows
        previous_snapshot = snapshot
    if captures:
        observed_first = captures[0].get("capture_hour_index")
        observed_last = captures[-1].get("capture_hour_index")
        if (
            timeline.get("capture_count") != len(captures)
            or timeline.get("first_capture_hour_index") != observed_first
            or timeline.get("last_capture_hour_index") != observed_last
        ):
            errors.append("timeline summary disagrees with capture sequence")
        if timeline.get("same_hour_recheck_count") != sum(
            capture.get("same_hour_rechecks", 0)
            for capture in captures
            if isinstance(capture.get("same_hour_rechecks"), int)
        ):
            errors.append("timeline recheck summary disagrees with capture sequence")
        if first_hour is not None and observed_first != first_hour:
            errors.append(f"first capture hour {observed_first} != required {first_hour}")
        if last_hour is not None and observed_last != last_hour:
            errors.append(f"last capture hour {observed_last} != required {last_hour}")
    if require_mutation and not any(
        transition["created"]
        or transition["tombstones"]
        or transition["mutated"]
        for transition in transitions
    ):
        errors.append("required design mutation was not observed")
    action_windows = timeline.get("action_windows", [])
    if (
        (action_windows or required_transactions or "action_window_count" in timeline)
        and timeline.get("action_window_count") != len(action_windows)
    ):
        errors.append("timeline action-window summary disagrees with retained windows")
    transaction_proofs = []
    if required_transactions:
        if not isinstance(action_receipt, dict):
            errors.append("required authenticated action receipt is missing")
        elif (
            action_receipt.get("schema")
            != "extractor.a27.authenticated_designer_transaction_receipt.v1"
            or action_receipt.get("status") != "complete"
            or action_receipt.get("executable_sha256") != PROGRAM_SHA256
            or action_receipt.get("expected_target_stable_id")
            != AUTHENTICATED_D04_TARGET_STABLE_ID
            or action_receipt.get("process_id") != authority.get("pid")
        ):
            errors.append("authenticated action receipt identity/status is invalid")
        else:
            receipt_transactions = action_receipt.get("transactions", [])
            if not isinstance(receipt_transactions, list):
                errors.append("authenticated action receipt transactions are invalid")
                receipt_transactions = []
            if len(receipt_transactions) != len(action_windows):
                errors.append("action receipt/window count mismatch")
            transition_by_hour = {item["to_hour"]: item for item in transitions}
            for sequence, required in enumerate(required_transactions):
                if sequence >= len(receipt_transactions) or sequence >= len(action_windows):
                    errors.append(f"missing authenticated {required} transaction {sequence}")
                    continue
                receipt_item = receipt_transactions[sequence]
                window = action_windows[sequence]
                transition = transition_by_hour.get(window.get("to_hour"))
                identity_fields = (
                    "sequence",
                    "transaction",
                    "from_hour",
                    "to_hour",
                    "profile_sha256",
                    "request_sha256",
                    "ready_sha256",
                    "release_sha256",
                )
                if (
                    required not in {"rename", "edit", "obsolete", "reactivate"}
                    or receipt_item.get("transaction") != required
                    or window.get("transaction") != required
                    or any(receipt_item.get(key) != window.get(key) for key in identity_fields)
                    or window.get("profile_sha256")
                    != action_receipt.get("profile_sha256")
                    or window.get("censuses_suspended_between_hours") is not True
                ):
                    errors.append(f"authenticated {required} transaction binding is invalid")
                    continue
                matching = [] if transition is None else [
                    detail
                    for detail in transition.get("mutation_details", [])
                    if required in detail.get("classes", [])
                ]
                if not matching:
                    errors.append(
                        f"authenticated {required} action produced no exact same-ID {required} mutation"
                    )
                    continue
                proof = {
                    "sequence": sequence,
                    "transaction": required,
                    "from_hour": window["from_hour"],
                    "to_hour": window["to_hour"],
                    "matching_native_mutations": matching,
                    "request_sha256": window["request_sha256"],
                    "ready_sha256": window["ready_sha256"],
                    "release_sha256": window["release_sha256"],
                }
                transaction_proofs.append(proof)
            observed_kinds = [item.get("transaction") for item in receipt_transactions]
            if observed_kinds != list(required_transactions):
                errors.append("authenticated action order differs from required transaction order")
            if len(transaction_proofs) == len(required_transactions):
                common_stable_ids = set(
                    item["stable_id"]
                    for item in transaction_proofs[0]["matching_native_mutations"]
                )
                for proof in transaction_proofs[1:]:
                    common_stable_ids &= {
                        item["stable_id"]
                        for item in proof["matching_native_mutations"]
                    }
                if not common_stable_ids:
                    errors.append(
                        "rename/edit/obsolete/reactivate were not proven on one common stable design identity"
                    )
                else:
                    expected_target = action_receipt.get("expected_target_stable_id")
                    if expected_target not in common_stable_ids:
                        errors.append(
                            "four transaction classes did not target the profile-pinned stable design identity"
                        )
                    for proof in transaction_proofs:
                        proof["common_stable_design_ids"] = sorted(common_stable_ids)
    return {
        "schema": "extractor.a27.live_heap_catalog_timeline_regrade.v2",
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "capture_count": len(captures),
        "rejected_observations": timeline.get("rejected_observations", []),
        "transitions": transitions,
        "transaction_proofs": transaction_proofs,
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--timeline", required=True, type=Path)
    parser.add_argument("--report-out", required=True, type=Path)
    parser.add_argument("--first-hour", type=int)
    parser.add_argument("--last-hour", type=int)
    parser.add_argument("--require-mutation", action="store_true")
    parser.add_argument("--minimum-captures", type=int, default=1)
    parser.add_argument("--action-receipt", type=Path)
    parser.add_argument(
        "--require-transaction",
        action="append",
        choices=("rename", "edit", "obsolete", "reactivate"),
        default=[],
    )
    args = parser.parse_args(argv)
    if args.minimum_captures < 1:
        parser.error("--minimum-captures must be positive")
    timeline = json.loads(args.timeline.read_text(encoding="utf-8"))
    action_receipt = (
        json.loads(args.action_receipt.read_text(encoding="utf-8"))
        if args.action_receipt is not None
        else None
    )
    report = grade(
        timeline,
        args.first_hour,
        args.last_hour,
        require_mutation=args.require_mutation,
        minimum_captures=args.minimum_captures,
        action_receipt=action_receipt,
        required_transactions=tuple(args.require_transaction),
    )
    report["input_artifacts"] = {
        "timeline_sha256": hashlib.sha256(args.timeline.read_bytes()).hexdigest(),
        "action_receipt_sha256": (
            hashlib.sha256(args.action_receipt.read_bytes()).hexdigest()
            if args.action_receipt is not None
            else None
        ),
    }
    write_create_new_json(args.report_out, report)
    print(
        f"{report['status']}: {report['capture_count']} authenticated hourly A27 censuses"
    )
    return 0 if report["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
