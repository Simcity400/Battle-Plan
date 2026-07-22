#!/usr/bin/env python3
"""Fail-closed admission for the autonomous A25/A26 live acquisition.

The action driver is not semantic authority.  This verifier independently
regrades the A26 matrix, validates the owner-scoped A25 raw glyph/icon join,
binds every raw artifact to the authenticated action chronology, and writes a
create-new portable receipt containing content hashes rather than host paths.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

EXE_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
A25_MANIFEST_SHA256 = "0d08554c66e82e188e3dec267bb9c03a2f0718aa71c3c648fc4aedf29ed27edf"
A26_MANIFEST_SHA256 = "a23b63ccefc021eeced9a1a918588d3e2fbba07e5d1afb4d53f83cc7c1640177"
PROFILE_SCHEMA = "hoi4.a25-a26.authenticated-tooltip-glyph-actions.v1"
ACTION_SCHEMA = "hoi4.a25-a26.authenticated-action-receipt.v1"
A25_SCHEMA = "hoi4.a25.owner-glyph.selected-present-receipt.v1"
FINAL_SCHEMA = "hoi4.a25-a26.autonomous-acquisition-admission.v1"
HEX64 = re.compile(r"^[0-9a-f]{64}$")
UINT = re.compile(r"^(0|[1-9][0-9]*)$")
REQUIRED_A26_CASES = {
    "hidden_baseline_1920x1080",
    "capture_start_visible_1920x1080",
    "show_1920x1080",
    "hide_1920x1080",
    "destroyed_1920x1080",
    "owner_change_1920x1080",
    "multiline_dynamic_1920x1080",
    "edge_geometry_1920x1080",
    "edge_geometry_2560x1440",
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def canonical_sha256(value: object) -> str:
    raw = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()


def load_object(path: Path, label: str) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    require(isinstance(value, dict), f"{label} must be a JSON object")
    return value


def contained(root: Path, candidate: Path, label: str) -> Path:
    resolved = candidate.resolve()
    try:
        resolved.relative_to(root.resolve())
    except ValueError as error:
        raise ValueError(f"{label} escapes acquisition directory") from error
    return resolved


def lower_hash(value: object, label: str, *, nonzero: bool = False) -> str:
    require(isinstance(value, str) and HEX64.fullmatch(value) is not None, f"{label} is not lowercase SHA-256")
    if nonzero:
        require(value != "0" * 64, f"{label} is zero")
    return value


def positive_uint(value: object, label: str) -> int:
    require(isinstance(value, str) and UINT.fullmatch(value) is not None, f"{label} is not canonical unsigned text")
    parsed = int(value)
    require(parsed > 0, f"{label} must be positive")
    return parsed


def validate_profile(profile: dict) -> list[dict]:
    require(profile.get("schema") == PROFILE_SCHEMA, "action profile schema mismatch")
    authority = profile.get("authority")
    require(isinstance(authority, dict), "action profile authority absent")
    require(authority.get("executable_sha256") == EXE_SHA256, "profile executable mismatch")
    require(authority.get("a25_re_manifest_sha256") == A25_MANIFEST_SHA256, "profile A25 RE mismatch")
    require(authority.get("a26_re_manifest_sha256") == A26_MANIFEST_SHA256, "profile A26 RE mismatch")
    actions = profile.get("actions")
    require(isinstance(actions, list) and actions, "profile actions absent")
    requests = [row for row in actions if isinstance(row, dict) and row.get("kind") == "capture_case"]
    a26 = [row.get("case_id") for row in requests if row.get("lane") == "A26"]
    a25 = [row.get("case_id") for row in requests if row.get("lane") == "A25"]
    require(len(a26) == len(REQUIRED_A26_CASES) and set(a26) == REQUIRED_A26_CASES, "profile A26 case set mismatch")
    require(a25 == ["mixed_glyph_inline_icon_1920x1080"], "profile A25 case set mismatch")
    for request in requests:
        width, height = int(request.get("width", 0)), int(request.get("height", 0))
        expected = (2560, 1440) if request.get("case_id") == "edge_geometry_2560x1440" else (1920, 1080)
        require((width, height) == expected, f"{request.get('case_id')}: profile viewport mismatch")
        require(isinstance(request.get("harness_command"), str) and request["harness_command"], "harness command absent")
    return requests


def validate_action_receipt(action: dict, profile: dict, profile_sha: str, out_dir: Path) -> tuple[dict, dict[str, str]]:
    require(action.get("schema") == ACTION_SCHEMA and action.get("status") == "PASS", "action receipt did not pass")
    require(action.get("promotion_authorized") is False, "action driver claimed semantic authority")
    process = action.get("process")
    require(isinstance(process, dict), "action process receipt absent")
    require(process.get("executable_sha256") == EXE_SHA256, "action process executable mismatch")
    lower_hash(process.get("collector_sha256"), "collector SHA-256", nonzero=True)
    profile_receipt = action.get("profile")
    require(isinstance(profile_receipt, dict) and profile_receipt.get("sha256") == profile_sha, "action profile hash mismatch")
    require(profile_receipt.get("profile_id") == profile.get("profile_id"), "action profile identity mismatch")
    steps = action.get("steps")
    require(isinstance(steps, list) and len(steps) == len(profile["actions"]), "action chronology length mismatch")
    capture_hashes: dict[str, str] = {}
    for ordinal, (expected, actual) in enumerate(zip(profile["actions"], steps)):
        require(isinstance(actual, dict), f"action {ordinal}: receipt is not an object")
        require(actual.get("ordinal") == ordinal, f"action {ordinal}: ordinal mismatch")
        require(actual.get("label") == expected.get("label") and actual.get("kind") == expected.get("kind"), f"action {ordinal}: profile mismatch")
        before, after = actual.get("client_size_before"), actual.get("client_size_after")
        require(isinstance(before, list) and len(before) == 2 and isinstance(after, list) and len(after) == 2, f"action {ordinal}: client geometry absent")
        if expected.get("kind") != "capture_case":
            continue
        capture = actual.get("capture")
        require(isinstance(capture, dict), f"action {ordinal}: capture result absent")
        case_id = str(expected["case_id"])
        require(capture.get("lane") == expected.get("lane") and capture.get("case_id") == case_id, f"{case_id}: action capture mismatch")
        require(capture.get("harness_command") == expected.get("harness_command"), f"{case_id}: harness command mismatch")
        require(after == [expected["width"], expected["height"]], f"{case_id}: capture viewport was not exact")
        signal = contained(out_dir, out_dir / str(capture.get("signal_file", "")), f"{case_id} signal")
        raw = contained(out_dir, out_dir / str(capture.get("raw_receipt_file", "")), f"{case_id} raw receipt")
        require(signal.is_file() and raw.is_file(), f"{case_id}: action artifacts absent")
        require(sha256(signal) == capture.get("signal_sha256"), f"{case_id}: action signal hash mismatch")
        raw_sha = sha256(raw)
        require(raw_sha == capture.get("raw_receipt_sha256"), f"{case_id}: raw receipt hash mismatch")
        require(case_id not in capture_hashes, f"{case_id}: duplicate action capture")
        capture_hashes[case_id] = raw_sha
    require(set(capture_hashes) == REQUIRED_A26_CASES | {"mixed_glyph_inline_icon_1920x1080"}, "action capture set incomplete")
    return process, capture_hashes


def validate_a25(receipt: dict, process: dict) -> dict:
    require(receipt.get("schema") == A25_SCHEMA, "A25 raw receipt schema mismatch")
    require(receipt.get("case_id") == "mixed_glyph_inline_icon_1920x1080", "A25 case identity mismatch")
    require(receipt.get("viewport") == [1920, 1080], "A25 viewport mismatch")
    authority = receipt.get("authority")
    require(isinstance(authority, dict), "A25 authority absent")
    require(authority.get("executable_sha256") == EXE_SHA256, "A25 executable mismatch")
    require(authority.get("re_manifest_sha256") == A25_MANIFEST_SHA256, "A25 RE manifest mismatch")
    require(receipt.get("process_id") == process.get("process_id"), "A25 process ID mismatch")
    require(receipt.get("process_start_utc") == process.get("process_start_utc"), "A25 process epoch mismatch")
    require(isinstance(receipt.get("hook_epoch"), int) and receipt["hook_epoch"] > 0, "A25 hook epoch absent")
    selected = receipt.get("selected_present")
    require(isinstance(selected, dict), "A25 selected Present absent")
    positive_uint(selected.get("selected_candidate_id"), "A25 selected candidate")
    positive_uint(selected.get("selected_qpc_ticks"), "A25 selected QPC")
    lower_hash(selected.get("candidate_set_sha256"), "A25 candidate set", nonzero=True)
    lower_hash(selected.get("receipt_sha256"), "A25 selected Present receipt", nonzero=True)
    captures = receipt.get("captures")
    require(isinstance(captures, list) and captures, "A25 owner captures absent")
    total_glyphs = total_icons = 0
    for index, capture in enumerate(captures):
        require(isinstance(capture, dict), f"A25 capture {index} is not an object")
        for field in ("context_id", "view_id", "element_id", "selected_final_text_sha256"):
            require(isinstance(capture.get(field), str) and capture[field], f"A25 capture {index}: {field} absent")
        lower_hash(capture["selected_final_text_sha256"], f"A25 capture {index} text digest", nonzero=True)
        require(isinstance(capture.get("gui_engine_instance_token"), str), f"A25 capture {index}: engine token must be pointer-free text")
        positive_uint(capture["gui_engine_instance_token"], f"A25 capture {index} engine token")
        require(isinstance(capture.get("paint_ordinal"), int) and capture["paint_ordinal"] >= 0, f"A25 capture {index}: paint ordinal invalid")
        require(isinstance(capture.get("render_invocation_ordinal"), int) and capture["render_invocation_ordinal"] >= 0, f"A25 capture {index}: invocation ordinal invalid")
        destination = capture.get("destination_rect")
        require(isinstance(destination, list) and len(destination) == 4 and all(isinstance(v, int) for v in destination), f"A25 capture {index}: destination absent")
        require(destination[2] > destination[0] and destination[3] > destination[1], f"A25 capture {index}: destination empty")
        health = capture.get("health")
        require(isinstance(health, dict) and health, f"A25 capture {index}: health absent")
        require(all(isinstance(value, int) and value == 0 for value in health.values()), f"A25 capture {index}: loss/error telemetry nonzero")
        glyphs = capture.get("glyphs")
        icons = capture.get("inline_icons")
        fonts = capture.get("fonts")
        icon_assets = capture.get("icon_assets")
        require(all(isinstance(value, list) for value in (glyphs, icons, fonts, icon_assets)), f"A25 capture {index}: run/assets absent")
        font_ids = set()
        for asset in fonts:
            require(isinstance(asset, dict) and isinstance(asset.get("identity"), str) and asset["identity"], f"A25 capture {index}: font identity absent")
            require(asset["identity"] not in font_ids, f"A25 capture {index}: duplicate font identity")
            font_ids.add(asset["identity"])
            lower_hash(asset.get("descriptor_sha256"), f"A25 capture {index} font descriptor", nonzero=True)
            atlases = asset.get("atlas_sha256")
            require(isinstance(atlases, list) and atlases, f"A25 capture {index}: font atlas absent")
            for digest in atlases:
                lower_hash(digest, f"A25 capture {index} font atlas", nonzero=True)
        icon_ids = set()
        for asset in icon_assets:
            require(isinstance(asset, dict) and isinstance(asset.get("identity"), str) and asset["identity"], f"A25 capture {index}: icon identity absent")
            require(asset["identity"] not in icon_ids, f"A25 capture {index}: duplicate icon identity")
            icon_ids.add(asset["identity"])
            lower_hash(asset.get("definition_sha256"), f"A25 capture {index} icon definition", nonzero=True)
            textures = asset.get("texture_sha256")
            require(isinstance(textures, list) and textures, f"A25 capture {index}: icon texture absent")
            for digest in textures:
                lower_hash(digest, f"A25 capture {index} icon texture", nonzero=True)
        ordinals = []
        for glyph in glyphs:
            require(isinstance(glyph, dict) and glyph.get("font_identity") in font_ids, f"A25 capture {index}: glyph font is unauthenticated")
            require(isinstance(glyph.get("run_ordinal"), int) and glyph["run_ordinal"] >= 0, f"A25 capture {index}: glyph ordinal invalid")
            require(isinstance(glyph.get("glyph_id"), int) and glyph["glyph_id"] >= 0, f"A25 capture {index}: glyph ID invalid")
            vertices = glyph.get("generated_vertices_hex")
            require(isinstance(vertices, str) and re.fullmatch(r"[0-9a-f]{336}", vertices) is not None, f"A25 capture {index}: glyph is not six 0x1c-byte vertices")
            ordinals.append(glyph["run_ordinal"])
        for icon in icons:
            require(isinstance(icon, dict) and icon.get("sprite_identity") in icon_ids, f"A25 capture {index}: inline icon asset is unauthenticated")
            require(isinstance(icon.get("run_ordinal"), int) and icon["run_ordinal"] >= 0, f"A25 capture {index}: icon ordinal invalid")
            lower_hash(icon.get("draw_receipt_sha256"), f"A25 capture {index} icon draw", nonzero=True)
            ordinals.append(icon["run_ordinal"])
        require(sorted(ordinals) == list(range(len(ordinals))), f"A25 capture {index}: mixed run ordinals are not exact")
        require(capture.get("glyph_decision_count") == len(glyphs), f"A25 capture {index}: glyph count mismatch")
        require(capture.get("inline_icon_decision_count") == len(icons), f"A25 capture {index}: icon count mismatch")
        total_glyphs += len(glyphs)
        total_icons += len(icons)
    require(total_glyphs > 0, "A25 fixture contains no ordinary glyph observation")
    require(total_icons > 0, "A25 fixture contains no inline-icon observation")
    telemetry = receipt.get("telemetry")
    require(isinstance(telemetry, dict) and telemetry, "A25 epoch telemetry absent")
    for key in ("completed_count", "discarded_non_selected_count", "rejected_count", "reentrancy_count", "queue_overflow_count"):
        require(isinstance(telemetry.get(key), int) and telemetry[key] >= 0, f"A25 telemetry {key} invalid")
    require(
        telemetry["rejected_count"] == 0
        and telemetry["reentrancy_count"] == 0
        and telemetry["queue_overflow_count"] == 0,
        "A25 epoch telemetry reports failure/loss",
    )
    require(telemetry["completed_count"] >= len(captures), "A25 completed telemetry is smaller than the selected set")
    return {"capture_count": len(captures), "glyph_count": total_glyphs, "inline_icon_count": total_icons}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out-dir", required=True, type=Path)
    parser.add_argument("--profile", required=True, type=Path)
    parser.add_argument("--launch-receipt", required=True, type=Path)
    parser.add_argument("--action-receipt", required=True, type=Path)
    parser.add_argument("--a25-receipt", required=True, type=Path)
    parser.add_argument("--a26-matrix", required=True, type=Path)
    parser.add_argument("--collector-dll", required=True, type=Path)
    parser.add_argument("--harness-exe", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    out_dir = args.out_dir.resolve()
    require(out_dir.is_dir(), "acquisition directory absent")
    profile_path = args.profile.resolve()
    launch_path = contained(out_dir, args.launch_receipt, "launch receipt")
    action_path = contained(out_dir, args.action_receipt, "action receipt")
    a25_path = contained(out_dir, args.a25_receipt, "A25 receipt")
    matrix_path = contained(out_dir, args.a26_matrix, "A26 matrix")
    output_path = contained(out_dir, args.output, "admission output")
    require(not output_path.exists(), "admission output must be create-new")
    for path, label in ((profile_path, "profile"), (launch_path, "launch receipt"), (action_path, "action receipt"), (a25_path, "A25 receipt"), (matrix_path, "A26 matrix"), (args.collector_dll, "collector"), (args.harness_exe, "harness")):
        require(path.is_file(), f"{label} absent")

    profile = load_object(profile_path, "profile")
    validate_profile(profile)
    profile_sha = sha256(profile_path)
    launch = load_object(launch_path, "launch receipt")
    require(launch.get("schema") == "hoi4.a25-a26.harness-launch.v1" and launch.get("status") == "ARMED", "harness launch receipt invalid")
    require(launch.get("executable_sha256") == EXE_SHA256, "harness launch executable mismatch")
    require(launch.get("harness_sha256") == sha256(args.harness_exe), "harness launch binary hash mismatch")
    require(launch.get("collector_sha256") == sha256(args.collector_dll), "harness launch collector hash mismatch")
    require(launch.get("profile_sha256") == profile_sha, "harness launch profile hash mismatch")
    action = load_object(action_path, "action receipt")
    process, action_hashes = validate_action_receipt(action, profile, profile_sha, out_dir)
    require(process.get("collector_sha256") == sha256(args.collector_dll), "action process collector hash mismatch")
    a25 = load_object(a25_path, "A25 receipt")
    a25_summary = validate_a25(a25, process)
    require(sha256(a25_path) == action_hashes["mixed_glyph_inline_icon_1920x1080"], "A25 action/raw hash mismatch")

    a26_verifier = Path(__file__).with_name("verify_a26_runtime_receipt.py")
    require(a26_verifier.is_file(), "A26 independent verifier absent")
    regrade = subprocess.run(
        [sys.executable, "-B", str(a26_verifier), str(matrix_path)],
        check=False,
        capture_output=True,
        text=True,
    )
    require(regrade.returncode == 0, f"A26 independent regrade failed: {(regrade.stderr or regrade.stdout).strip()}")
    matrix = load_object(matrix_path, "A26 matrix")
    cases = matrix.get("cases")
    require(isinstance(cases, list) and {row.get("case_id") for row in cases if isinstance(row, dict)} == REQUIRED_A26_CASES, "A26 matrix case set mismatch")
    for row in cases:
        case_id = row["case_id"]
        require(row.get("case_receipt_file_sha256") == action_hashes[case_id], f"{case_id}: matrix/action raw hash mismatch")

    artifacts = {
        "harness_launch.json": sha256(launch_path),
        "action_receipt.json": sha256(action_path),
        "a25_owner_glyph_capture.json": sha256(a25_path),
        "a26_same_present_matrix.json": sha256(matrix_path),
    }
    for case_id in sorted(REQUIRED_A26_CASES):
        artifacts[f"a26_cases/{case_id}.json"] = action_hashes[case_id]
    for step in action["steps"]:
        capture = step.get("capture") if isinstance(step, dict) else None
        if isinstance(capture, dict):
            signal_rel = str(capture["signal_file"])
            artifacts[signal_rel] = sha256(contained(out_dir, out_dir / signal_rel, "action signal"))
    source_set_sha = canonical_sha256(artifacts)
    result = {
        "schema": FINAL_SCHEMA,
        "status": "PASS",
        "assembled_utc": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "evidence_admission": "authenticated_exact_a729_owner_observation",
        "production_promotion_authorized": False,
        "authority": {
            "executable_sha256": EXE_SHA256,
            "a25_re_manifest_sha256": A25_MANIFEST_SHA256,
            "a26_re_manifest_sha256": A26_MANIFEST_SHA256,
        },
        "process_epoch": {
            "process_id": process["process_id"],
            "process_start_utc": process["process_start_utc"],
        },
        "tooling": {
            "collector_file": args.collector_dll.name,
            "collector_sha256": sha256(args.collector_dll),
            "harness_file": args.harness_exe.name,
            "harness_sha256": sha256(args.harness_exe),
            "profile_file": profile_path.name,
            "profile_sha256": profile_sha,
            "a26_verifier_file": a26_verifier.name,
            "a26_verifier_sha256": sha256(a26_verifier),
        },
        "a25_summary": a25_summary,
        "a26_case_count": len(REQUIRED_A26_CASES),
        "artifacts": [{"path": path, "sha256": digest} for path, digest in sorted(artifacts.items())],
        "source_set_sha256": source_set_sha,
        "promotion_note": "Evidence is admitted. Production activation remains a separate source change followed by central validation.",
    }
    output_path.parent.mkdir(parents=True, exist_ok=True)
    with output_path.open("x", encoding="utf-8", newline="\n") as handle:
        json.dump(result, handle, indent=2, sort_keys=True, ensure_ascii=False)
        handle.write("\n")
    print(f"PASS A25/A26 autonomous acquisition: {a25_summary['glyph_count']} glyphs, {a25_summary['inline_icon_count']} icons, 9 tooltip cases")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        raise SystemExit(f"FAIL A25/A26 autonomous acquisition: {error}")
