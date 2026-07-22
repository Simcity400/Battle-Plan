#!/usr/bin/env python3
"""Fail-closed verifier for the autonomous D44 ARC1/PST1 acquisition."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import secrets
from pathlib import Path


EXPECTED_EXE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
EXPECTED_DOMAINS = {"country_identity", "supply_logistics_attrition_live"}
ARC_AUTHORITY = "exact_a729_transient_attrition_arithmetic_checkpoints"
PST_AUTHORITY = "exact_a729_province_supply_tooltip_formatter_sink"
PST_JOB_ID = "20260721T064706Z-wave6-exact-supply-tooltip-producer-chai-82394c9c"
PST_JOB_JSON_SHA256 = "2fb7d61f19dc76ba05bfb2850b0d43ede16ca476f9aabd721f32dfc3dfd591da"
PST_MANIFEST_SHA256 = "3f346cfcaf37895c347002fcfbd0782bd15672342aa6ba3da56f2396016108fd"
MAX_SNAPSHOT_FILES = 32
MAX_SNAPSHOT_BYTES = 512 * 1024 * 1024


def exact_int(value, minimum: int, maximum: int) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and minimum <= value <= maximum


def positive_decimal_string(value) -> bool:
    return isinstance(value, str) and value.isdecimal() and int(value) > 0


def is_sha256(value) -> bool:
    return isinstance(value, str) and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def source_product_modset_sha256(source_bundle: dict):
    bindings = source_bundle.get("bindings")
    product_modset = bindings.get("product_modset") if isinstance(bindings, dict) else None
    return product_modset.get("product_modset_sha256") if isinstance(product_modset, dict) else None


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def strict_pairs(pairs):
    value = {}
    for key, child in pairs:
        if key in value:
            raise ValueError(f"duplicate JSON member: {key}")
        value[key] = child
    return value


def load_object(path: Path, label: str) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8-sig"), object_pairs_hook=strict_pairs)
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"{label} is not readable strict JSON: {error}") from None
    if not isinstance(value, dict):
        raise ValueError(f"{label} root is not an object")
    return value


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def trunc_div(numerator: int, denominator: int) -> int:
    quotient = abs(numerator) // denominator
    return -quotient if numerator < 0 else quotient


def validate_plan(
    plan: dict,
    manifest: dict,
    source_bundle: dict,
    source_bundle_path: Path,
    collector_dll: Path,
    recorder_exe: Path,
    stream_fold_exe: Path,
    continue_save: Path,
) -> None:
    require(plan.get("schema") == "hoi4.d44.arc1-pst1-acquisition-plan.v1", "acquisition plan schema mismatch")
    require(plan.get("attempt_budget") == 1 and plan.get("clean_launch") is True, "plan is not a one-attempt clean launch")
    require(set(plan.get("scheduled_domains", [])) == EXPECTED_DOMAINS, "plan domain roster mismatch")
    require(plan.get("executable_sha256") == EXPECTED_EXE_SHA256, "plan executable identity mismatch")
    require(is_sha256(plan.get("collector_sha256")) and is_sha256(plan.get("recorder_sha256")), "plan artifact hashes malformed")
    require(is_sha256(plan.get("continue_save_sha256")), "plan save hash malformed")
    require(plan.get("collector_sha256") == sha256_file(collector_dll), "plan/collector DLL hash mismatch")
    require(plan.get("recorder_sha256") == sha256_file(recorder_exe), "plan/recorder hash mismatch")
    require(plan.get("stream_fold_sha256") == sha256_file(stream_fold_exe), "plan/stream_fold hash mismatch")
    require(plan.get("continue_save_sha256") == sha256_file(continue_save), "plan/continue-save hash mismatch")
    require(
        plan.get("source_static_bundle_manifest_sha256") == sha256_file(source_bundle_path),
        "plan/static-bundle manifest hash mismatch",
    )
    require(plan.get("stream_format") == "compact" and plan.get("stream_compression") == "none", "plan stream contract mismatch")
    require(plan.get("d44_emit_mode") == "full" and plan.get("min_game_hours") == 1, "plan D44/hour contract mismatch")
    require(plan.get("hour_latch_poll_milliseconds") == 1, "plan hour latch is not exact 1ms mode")
    require(plan.get("session_id") == manifest.get("session_id"), "plan/session manifest session mismatch")
    require(plan.get("capture_id") == manifest.get("capture_id"), "plan/session manifest capture mismatch")
    source_product = source_product_modset_sha256(source_bundle)
    require(is_sha256(source_product), "source static bundle lacks product_modset_sha256")
    require(plan.get("product_modset_sha256") == source_product, "plan/source product-modset mismatch")


def validate_manifest(manifest: dict, source_bundle: dict) -> int:
    require(manifest.get("schema") == "native_collector_session_v1", "capture manifest schema mismatch")
    scheduled = manifest.get("scheduled_domains")
    require(isinstance(scheduled, list) and len(scheduled) == 2 and set(scheduled) == EXPECTED_DOMAINS, "capture was not exact D18+D44")
    require(manifest.get("stream_format") == "compact", "capture stream is not compact")
    require(manifest.get("collector_control_errors") == [] and manifest.get("parse_errors") == [], "capture reports collector/parse errors")
    require(exact_int(manifest.get("game_hours_advanced"), 1, 8), "capture did not retain a bounded advancing window")
    pid = manifest.get("pid")
    require(exact_int(pid, 1, (1 << 32) - 1), "capture PID is malformed")
    modset = manifest.get("modset")
    require(isinstance(modset, dict) and modset.get("resolved") is True and modset.get("errors") == [], "capture modset is unresolved")
    require(isinstance(modset.get("game"), dict) and modset["game"].get("exe_sha256") == EXPECTED_EXE_SHA256, "capture is not exact a729")
    early = manifest.get("early_render_capture_receipt")
    require(isinstance(early, dict) and early.get("process_id") == pid, "early-launch process receipt is absent or PID-mismatched")
    static = manifest.get("static_bundle")
    require(isinstance(static, dict) and static.get("admission") == "passed", "finalized static-bundle admission missing")
    require(static.get("verification_phase") == "post_capture_finalized" and static.get("token_join_status") == "passed", "static bundle was not post-capture finalized")
    require(static.get("product_modset_sha256") == source_product_modset_sha256(source_bundle), "static product-modset identity drifted")
    compact = manifest.get("compact_stream")
    require(isinstance(compact, dict) and compact.get("decodable") is True, "compact stream is not decodable")
    expected = compact.get("expected_domains")
    require(isinstance(expected, list) and set(expected) == EXPECTED_DOMAINS, "compact expected-domain roster mismatch")
    seen = compact.get("domains_seen")
    require(isinstance(seen, list) and EXPECTED_DOMAINS.issubset(set(seen)), "compact stream did not observe D18 and D44")
    per_domain = compact.get("per_domain")
    require(isinstance(per_domain, dict), "compact per-domain telemetry missing")
    for name in EXPECTED_DOMAINS:
        row = per_domain.get(name)
        require(isinstance(row, dict) and exact_int(row.get("data_record_count"), 2, (1 << 63) - 1), f"{name} has fewer than two data frames")
        require(row.get("problems") in ({}, None), f"{name} reports compact-stream problems")
    return pid


def validate_actions(readiness: dict, hover: dict, pulse: dict, pid: int) -> None:
    for receipt, label in ((readiness, "readiness"), (hover, "hover"), (pulse, "pulse")):
        require(receipt.get("process_id") == pid, f"{label} receipt PID mismatch")
        require(receipt.get("executable_sha256") == EXPECTED_EXE_SHA256, f"{label} receipt executable mismatch")
    require(readiness.get("schema") == "hoi4-rendering-law-paused-save-readiness-v1", "readiness schema mismatch")
    require(readiness.get("action") == "authenticated_read_only_paused_save_ready", "readiness action mismatch")
    require(readiness.get("pause") == 1 and exact_int(readiness.get("stable_samples"), 2, 20), "paused-save readiness was not stable")
    require(hover.get("schema") == "hoi4-rendering-law-window-action-v1", "hover receipt schema mismatch")
    require(hover.get("action") == "authenticated_foreground_strong_key_and_hover", "hover action mismatch")
    require(hover.get("virtual_key") == "0x73", "hover action was not F4")
    require(hover.get("foreground_owner_process_id") == pid, "hover foreground ownership mismatch")
    require(isinstance(hover.get("client_point"), list) and len(hover["client_point"]) == 2, "hover client point missing")
    require(exact_int(hover.get("settle_milliseconds"), 1500, 10000), "hover dwell was too short")
    require(pulse.get("schema") == "hoi4-rendering-law-authenticated-clock-pulse-v2", "pulse receipt schema mismatch")
    require(pulse.get("status") == "PASS", "pulse terminal status is not PASS")
    require(pulse.get("action") == "authenticated_foreground_sendinput_clock_pulse", "pulse action mismatch")
    require(pulse.get("input_api") == "user32!SendInput" and pulse.get("input_mode") == "KEYEVENTF_SCANCODE", "pulse input authority mismatch")
    require(pulse.get("target_hours") == 1 and pulse.get("poll_milliseconds") == 1, "pulse did not request exact one-hour/1ms mode")
    require(pulse.get("pause_before") == 1 and pulse.get("pause_after") == 1, "pulse did not prove repause")
    require(pulse.get("speed_before") == pulse.get("speed_after"), "pulse did not restore the original game speed")
    require(pulse.get("game_hours_advanced") == 1, "pulse did not advance exactly one game hour")
    require(pulse.get("game_hour_after") == pulse.get("game_hour_before") + 1, "pulse hour arithmetic mismatch")
    require(pulse.get("pulse_error") is None and pulse.get("restoration_error") is None, "pulse reported an action/restoration error")
    require(readiness.get("process_start_utc") == pulse.get("process_start_utc") == pulse.get("process_start_after_utc"), "action receipts span different process lifetimes")


def validate_arc(meta: dict) -> int:
    require(meta.get("division_attrition_source_available") is True, "ATR1 source unavailable in candidate snapshot")
    require(meta.get("division_attrition_components_source_available") is True, "ARC1 source unavailable in candidate snapshot")
    observed = meta.get("observed_division_roots")
    rows = meta.get("division_attrition_component_rows")
    attrition_rows = meta.get("division_attrition_rows")
    require(exact_int(observed, 1, 16384), "D44 observed division roster is empty or malformed")
    require(isinstance(rows, list) and len(rows) == observed, "ARC1 is not a complete current D44 division roster")
    require(meta.get("division_attrition_components_row_count") == len(rows), "ARC1 row count metadata mismatch")
    require(isinstance(attrition_rows, list) and len(attrition_rows) == observed, "ATR1 roster does not match D44 roots")
    authority = meta.get("division_attrition_components_authority")
    require(isinstance(authority, dict), "ARC1 authority missing")
    require(authority.get("carrier_magic") == "ARC1" and authority.get("authority") == ARC_AUTHORITY, "ARC1 authority identity mismatch")
    require(authority.get("executable_sha256") == EXPECTED_EXE_SHA256, "ARC1 executable identity mismatch")
    require(authority.get("record_stride") == 176 and authority.get("dropped_rows") == 0, "ARC1 stride/drop telemetry invalid")
    require(authority.get("pointer_free") is True and authority.get("current_frame_only") is True, "ARC1 portability/lifetime flags invalid")
    require(positive_decimal_string(authority.get("hook_epoch")) and positive_decimal_string(authority.get("drain_sequence")), "ARC1 batch identity invalid")
    identities = []
    sequences = set()
    terms = (
        "upkeep_total_raw", "terrain_raw", "equipment_raw", "state_raw", "supply_raw",
        "state_modifiers_raw", "weather_modifiers_raw", "derived_terms_raw", "training_raw", "border_war_raw",
    )
    for index, row in enumerate(rows):
        require(isinstance(row, dict), f"ARC1 row {index} is malformed")
        identity = row.get("raw_division_id")
        require(exact_int(identity, 1, (1 << 32) - 1), f"ARC1 row {index} division identity invalid")
        identities.append(identity)
        require(exact_int(row.get("province_id"), 1, 13413), f"ARC1 row {index} province invalid")
        require(exact_int(row.get("owner_tag_db_id"), 1, 2048), f"ARC1 row {index} owner invalid")
        require(row.get("observation_mask") == "1023" and row.get("refresh_caller_rva") == "0x00C79598", f"ARC1 row {index} checkpoint mask/caller invalid")
        require(row.get("exact") is True, f"ARC1 row {index} is not exact")
        require(all(exact_int(row.get(field), -(1 << 63), (1 << 63) - 1) for field in terms), f"ARC1 row {index} component malformed")
        pre = row.get("pre_multiplier_total_raw")
        multiplier = row.get("final_multiplier_raw")
        final = row.get("final_total_raw")
        cached = row.get("cached_total_raw")
        require(all(exact_int(value, -(1 << 63), (1 << 63) - 1) for value in (pre, multiplier, final, cached)), f"ARC1 row {index} totals malformed")
        require(sum(row[field] for field in terms) == pre, f"ARC1 row {index} component recomposition failed")
        require(trunc_div(pre * multiplier, 100000) == final and final == cached, f"ARC1 row {index} multiplier/cache join failed")
        sequence = row.get("sequence")
        require(positive_decimal_string(sequence) and sequence not in sequences, f"ARC1 row {index} sequence invalid")
        sequences.add(sequence)
    require(identities == sorted(set(identities)), "ARC1 rows are not unique canonical division order")
    atr_identities = [row.get("raw_division_id") for row in attrition_rows if isinstance(row, dict)]
    require(atr_identities == identities, "ARC1/ATR1 division identity order mismatch")
    return len(rows)


def validate_pst(snapshot: dict, supply: dict) -> int:
    tooltip = supply.get("exact_tooltip_observations")
    require(isinstance(tooltip, dict) and tooltip.get("status") == "available", "PST1 is not available")
    require(tooltip.get("dropped_rows") == 0, "PST1 dropped rows")
    require(positive_decimal_string(tooltip.get("hook_epoch")) and positive_decimal_string(tooltip.get("drain_sequence")), "PST1 batch identity invalid")
    authority = tooltip.get("authority")
    require(isinstance(authority, dict) and authority.get("authority_id") == 0xA7290044, "PST1 authority id mismatch")
    require(authority.get("executable_sha256") == EXPECTED_EXE_SHA256 and authority.get("ghidra_job_id") == PST_JOB_ID, "PST1 executable/job identity mismatch")
    require(authority.get("ghidra_job_json_sha256") == PST_JOB_JSON_SHA256 and authority.get("function_manifest_sha256") == PST_MANIFEST_SHA256, "PST1 authority hashes mismatch")
    require(authority.get("producer_rva") == "0x01623070" and authority.get("formatter_return_rva") == "0x016238F9", "PST1 producer/formatter identity mismatch")
    require(authority.get("fixed_scale") == 100000 and authority.get("displayed_decimals") == 2, "PST1 scale/format mismatch")
    measured = snapshot.get("measured_countries")
    tag_index = snapshot.get("tag_db_id_index")
    require(isinstance(measured, dict) and measured.get("known") is True, "PST1 snapshot lacks measured D18 countries")
    require(isinstance(tag_index, dict) and tag_index.get("resolved") is True, "PST1 snapshot lacks resolved D18 tag index")
    measured_ids = measured.get("tag_db_ids")
    require(isinstance(measured_ids, list) and measured.get("count") == len(measured_ids), "D18 measured-country roster is malformed")
    require(
        all(exact_int(value, 1, 2048) for value in measured_ids) and measured_ids == sorted(set(measured_ids)),
        "D18 measured-country identities are not unique canonical order",
    )
    rows = tooltip.get("rows")
    require(isinstance(rows, list) and 1 <= len(rows) <= 16384, "PST1 row set is empty or oversized")
    sequences = []
    for index, row in enumerate(rows):
        require(isinstance(row, dict), f"PST1 row {index} malformed")
        require(exact_int(row.get("province_id"), 1, 13413), f"PST1 row {index} province invalid")
        require(exact_int(row.get("observer_tag_db_id"), 1, 2048), f"PST1 row {index} observer invalid")
        require(row["observer_tag_db_id"] in measured_ids, f"PST1 row {index} observer was not resolved by same-frame D18")
        raw = row.get("remaining_supply_raw")
        scaled = row.get("remaining_supply")
        require(exact_int(raw, -(1 << 63), (1 << 63) - 1) and isinstance(scaled, (int, float)) and not isinstance(scaled, bool), f"PST1 row {index} value malformed")
        require(math.isfinite(float(scaled)) and float(scaled) == raw / 100000.0, f"PST1 row {index} scaled parity failed")
        require(row.get("fixed_scale") == 100000 and row.get("displayed_decimals") == 2, f"PST1 row {index} format mismatch")
        require(exact_int(row.get("tick"), 0, (1 << 63) - 1) and positive_decimal_string(row.get("sequence")), f"PST1 row {index} tick/sequence invalid")
        sequences.append(int(row["sequence"]))
    require(sequences == sorted(set(sequences)), "PST1 rows are not unique producer sequence order")
    return len(rows)


def select_snapshot(fold_dir: Path) -> tuple[Path, dict, int, int]:
    root = fold_dir.resolve()
    files = sorted(fold_dir.glob("*.snapshot.json"))
    require(1 <= len(files) <= MAX_SNAPSHOT_FILES, "folded snapshot count outside bounded acquisition contract")
    failures = []
    for path in files:
        resolved = path.resolve()
        require(resolved.parent == root and path.stat().st_size <= MAX_SNAPSHOT_BYTES, "snapshot path/size escaped acquisition bounds")
        snapshot = load_object(path, f"snapshot {path.name}")
        try:
            supply = snapshot.get("supply")
            require(isinstance(supply, dict), "supply projection absent")
            meta = supply.get("r2_logistics_attrition")
            require(isinstance(meta, dict), "D44 logistics metadata absent")
            arc_rows = validate_arc(meta)
            pst_rows = validate_pst(snapshot, supply)
            return path, snapshot, arc_rows, pst_rows
        except ValueError as error:
            failures.append(f"{path.name}: {error}")
    raise ValueError("no same-frame ARC1+PST1 snapshot passed: " + " | ".join(failures[-4:]))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--capture-manifest", required=True, type=Path)
    parser.add_argument("--acquisition-plan", required=True, type=Path)
    parser.add_argument("--readiness-receipt", required=True, type=Path)
    parser.add_argument("--hover-receipt", required=True, type=Path)
    parser.add_argument("--pulse-receipt", required=True, type=Path)
    parser.add_argument("--source-static-bundle-manifest", required=True, type=Path)
    parser.add_argument("--collector-stream", required=True, type=Path)
    parser.add_argument("--collector-dll", required=True, type=Path)
    parser.add_argument("--recorder-exe", required=True, type=Path)
    parser.add_argument("--stream-fold-exe", required=True, type=Path)
    parser.add_argument("--continue-save", required=True, type=Path)
    parser.add_argument("--fold-dir", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()

    manifest = load_object(args.capture_manifest, "capture manifest")
    plan = load_object(args.acquisition_plan, "acquisition plan")
    readiness = load_object(args.readiness_receipt, "readiness receipt")
    hover = load_object(args.hover_receipt, "hover receipt")
    pulse = load_object(args.pulse_receipt, "pulse receipt")
    source_bundle = load_object(args.source_static_bundle_manifest, "source static bundle manifest")
    validate_plan(
        plan,
        manifest,
        source_bundle,
        args.source_static_bundle_manifest,
        args.collector_dll,
        args.recorder_exe,
        args.stream_fold_exe,
        args.continue_save,
    )
    pid = validate_manifest(manifest, source_bundle)
    validate_actions(readiness, hover, pulse, pid)
    winning_path, winning_snapshot, arc_rows, pst_rows = select_snapshot(args.fold_dir)

    receipt = {
        "schema": "hoi4.d44.arc1-pst1-live-verification.v1",
        "verdict": "PASS_D44_ARC1_PST1_LIVE_ACQUISITION",
        "executable_sha256": EXPECTED_EXE_SHA256,
        "session_id": manifest.get("session_id"),
        "capture_id": manifest.get("capture_id"),
        "process_id": pid,
        "game_hours_advanced": manifest.get("game_hours_advanced"),
        "pulse_game_hours_advanced": pulse.get("game_hours_advanced"),
        "pause_restored": True,
        "winning_snapshot": winning_path.name,
        "winning_snapshot_sha256": sha256_file(winning_path),
        "winning_hour_index": winning_snapshot.get("hour_index"),
        "winning_date": winning_snapshot.get("date"),
        "arc1_row_count": arc_rows,
        "pst1_row_count": pst_rows,
        "capture_manifest_sha256": sha256_file(args.capture_manifest),
        "acquisition_plan_sha256": sha256_file(args.acquisition_plan),
        "readiness_receipt_sha256": sha256_file(args.readiness_receipt),
        "hover_receipt_sha256": sha256_file(args.hover_receipt),
        "pulse_receipt_sha256": sha256_file(args.pulse_receipt),
        "source_static_bundle_manifest_sha256": sha256_file(args.source_static_bundle_manifest),
        "collector_stream_sha256": sha256_file(args.collector_stream),
        "collector_dll_sha256": sha256_file(args.collector_dll),
        "recorder_exe_sha256": sha256_file(args.recorder_exe),
        "stream_fold_exe_sha256": sha256_file(args.stream_fold_exe),
        "continue_save_sha256": sha256_file(args.continue_save),
        "verifier_sha256": sha256_file(Path(__file__).resolve()),
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.out.with_name(args.out.name + "." + secrets.token_hex(8) + ".tmp")
    temporary.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    temporary.replace(args.out)
    print(args.out)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError) as error:
        raise SystemExit(f"verify_d44_arc1_pst1_capture: {error}") from None
