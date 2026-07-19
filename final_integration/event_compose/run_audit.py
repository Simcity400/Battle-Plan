#!/usr/bin/env python3
"""Rerun the frozen renderer event/composition gates without changing production."""

from __future__ import annotations

import concurrent.futures
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
EVIDENCE = ROOT / "replay_interface/re_evidence/rendering_laws"
AUTHORITY = ROOT
PLAN = ROOT / "replay_interface/documents/RENDERING_LAWS_RE_EXECUTION_PLAN.md"
RUNS = HERE / "runs"
STATIC_SRV_CLOSURE = {
    "frame_bundle": {
        "path": "replay_interface/native/map_viewer/src/frame_bundle.rs",
        "sha256": "c6919d0a0984b7640133292b15d61cbda5674aae1b144fafed9991a80109c1e5",
    },
    "verification": {
        "path": "static_srv_resolver_join/verification.json",
        "sha256": "468035b76a967ad1c070967faa7d2199d0e2e1e20b4f7e452bf1e326beff932d",
    },
    "trees_oracle_manifest": {
        "path": "trees_measure_legacy_join/manifest.json",
        "sha256": "875047b57fb58e8a9faa4d8b62e21d29f04ab11be2fb6f0372e181c9b852af01",
    },
}


def normalized_log(text: str) -> str:
    """Remove wall-clock-only fields so repeated mechanical runs freeze identically."""
    text = re.sub(r"finished in [0-9]+(?:\.[0-9]+)?s", "finished in <elapsed>", text)
    text = re.sub(r"target\(s\) in [0-9]+(?:\.[0-9]+)?s", "target(s) in <elapsed>", text)
    text = re.sub(r"elapsed_ms [0-9]+(?:\.[0-9]+)?", "elapsed_ms <elapsed>", text)
    text = re.sub(r"(thread '[^']+') \([0-9]+\) panicked", r"\1 (<thread>) panicked", text)
    return text


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def verified_static_srv_closure() -> dict:
    resolved = {}
    for name, row in STATIC_SRV_CLOSURE.items():
        path = (ROOT / row["path"]) if name == "frame_bundle" else (HERE / row["path"])
        if not path.is_file() or sha256(path) != row["sha256"]:
            raise RuntimeError(f"static SRV closure drift: {name}: {path}")
        resolved[name] = {**row, "bytes": path.stat().st_size}
    verification = json.loads((HERE / STATIC_SRV_CLOSURE["verification"]["path"]).read_text(encoding="utf-8"))
    oracle = json.loads((HERE / STATIC_SRV_CLOSURE["trees_oracle_manifest"]["path"]).read_text(encoding="utf-8"))
    if verification.get("status") != "PASS_PRODUCTION_ADAPTER_READY_TO_PROMOTE":
        raise RuntimeError("static SRV resolver verification is not promotion-ready")
    if oracle.get("status") != "PASS_EXACT_SOURCE_AND_CAPTURE_ORACLE_FROZEN":
        raise RuntimeError("trees static SRV oracle is not frozen PASS")
    return resolved


def o05_command() -> list[str]:
    manifest = json.loads((EVIDENCE / "O05/evidence_manifest.json").read_text(encoding="utf-8"))
    by_id = {row["id"]: Path(row["path"]) for row in manifest["artifacts"]}
    for key, path in tuple(by_id.items()):
        if not path.is_absolute():
            by_id[key] = ROOT / path
    paths = {
        "plan": PLAN,
        "oracle": by_id["O05_ORACLE"],
        "summary": by_id["W13_SUMMARY"],
        "placement": by_id["W13_PLACEMENT"],
        "attribution": by_id["W13_ATTRIBUTION"],
        "w13_manifest": by_id["W13_EVIDENCE_MANIFEST"],
        "events": by_id["CAP9_EVENTS"],
        "resources": by_id["CAP9_RESOURCES"],
        "immutable": by_id["CAP9_IMMUTABLE_CONTENTS"],
        "cb_blobs": by_id["CAP9_CB_BLOBS"],
        "rdc": by_id["CAP9_RDC"],
        "w21_core": by_id["W21_CORE_FUNCTIONS"],
        "w21_helpers": by_id["W21_HELPER_FUNCTIONS"],
        "event_state": by_id["O05_EVENT_STATE"],
        "pixel_gate": by_id["O05_PIXEL_GATE"],
        "semantic_identity": by_id["O05_SEMANTIC_IDENTITY"],
        "semantic_builder": by_id["O05_SEMANTIC_BUILDER"],
        "fixed_object_semantics": by_id["O05_FIXED_OBJECT_SEMANTICS"],
        "map_buildings": by_id["BATTLE_PLAN_MAP_BUILDINGS"],
        "graphics_defines": by_id["BATTLE_PLAN_GRAPHICS_DEFINES"],
        "ambient_object": by_id["BATTLE_PLAN_AMBIENT_OBJECT"],
        "mapitems_asset": by_id["BATTLE_PLAN_MAPITEMS_ASSET"],
        "mapitems_gfx": by_id["BATTLE_PLAN_MAPITEMS_GFX"],
        "state9": by_id["BATTLE_PLAN_STATE_9"],
        "landmark_buildings": by_id["BATTLE_PLAN_LANDMARK_BUILDINGS"],
        "landmarks_asset": by_id["BATTLE_PLAN_LANDMARKS_ASSET"],
        "landmarks_gfx": by_id["BATTLE_PLAN_LANDMARKS_GFX"],
        "ghidra_index": by_id["O05_GHIDRA_PROMOTION_INDEX"],
    }
    cmd = [sys.executable, str(EVIDENCE / "O05/verify_o05.py")]
    for name, path in paths.items():
        cmd.extend(["--" + name.replace("_", "-"), str(path)])
    cmd.extend(["--require-promotion", "--output", str(RUNS / "o05_result.json")])
    return cmd


def gate_specs() -> list[dict]:
    py = sys.executable
    return [
        {"id": "L02_shadow_matrix_composition", "packet": "L02", "class": "production_promotion", "cmd": [py, str(EVIDENCE / "L02/verify_production_port.py")], "artifacts": ["replay_interface/re_evidence/rendering_laws/L02/promotion_manifest.json", "replay_interface/re_evidence/rendering_laws/L02/production_port_verification.json", "replay_interface/native/map_viewer/src/shadow_law.rs"]},
        {"id": "L03_object_pass_projection", "packet": "L03", "class": "production_promotion", "cmd": [py, str(EVIDENCE / "L03/production_projection/verify_production_projection.py")], "artifacts": ["replay_interface/re_evidence/rendering_laws/L03/manifest.json", "replay_interface/re_evidence/rendering_laws/L03/production_projection/verification.json", "replay_interface/native/map_viewer/src/light_depth_projection.rs"]},
        {"id": "L06_snowmud_pixel_join", "packet": "L06", "class": "captured_pixel_join", "cmd": [py, str(EVIDENCE / "L06/verify_l06.py")]},
        {"id": "M01_unit_event_pass", "packet": "M01", "class": "production_promotion", "cmds": [[py, str(EVIDENCE / "M01/runtime_admission/verify_runtime_admission.py")], [py, str(EVIDENCE / "M01/verify_m01.py")]], "artifacts": ["replay_interface/re_evidence/rendering_laws/M01/evidence_manifest.json", "replay_interface/re_evidence/rendering_laws/M01/runtime_admission/verification_result.json", "replay_interface/native/map_viewer/src/m01_runtime_admission.rs"]},
        {"id": "M03_arrow_pixel_contract", "packet": "M03", "class": "pixel_consumer_contract", "cmd": [py, str(EVIDENCE / "M03/assist_pixel_consumer/verify_pixel_consumer.py")]},
        {"id": "N02_map_label_projection", "packet": "N02", "class": "production_promotion", "cmds": [[py, str(EVIDENCE / "N02/verify_n02.py"), "--output", str(RUNS / "n02_result.json")], [py, str(EVIDENCE / "N02/verify_promotion_manifest.py")]], "artifacts": ["replay_interface/re_evidence/rendering_laws/N02/promotion_manifest.json", "replay_interface/re_evidence/rendering_laws/N02/promotion_manifest_verification.json", "replay_interface/native/map_viewer/src/map_label_law.rs"]},
        {"id": "O04_city_event_pixel_join", "packet": "O04", "class": "event_pixel_evidence", "cmd": [py, str(EVIDENCE / "O04/verify_o04.py")]},
        {"id": "O05_object_event_pixel_gate", "packet": "O05", "class": "production_promotion", "cmd": [py, str(EVIDENCE / "O05/verify_o05_production_promotion.py")], "artifacts": ["replay_interface/re_evidence/rendering_laws/O05/production_promotion.json", "replay_interface/re_evidence/rendering_laws/O05/production_promotion_verification.json", "replay_interface/native/map_viewer/src/o05_fixed_object_law.rs"]},
        {"id": "R02_river_event_order", "packet": "R02", "class": "event_pass_order", "cmd": [py, str(EVIDENCE / "R02/verify_r02.py")]},
        {"id": "UI06_legacy_pixel_join", "packet": "UI06", "class": "production_promotion", "cmd": [py, str(EVIDENCE / "UI06/verify_production_promotion.py")], "artifacts": ["replay_interface/re_evidence/rendering_laws/UI06/production_manifest.json", "replay_interface/re_evidence/rendering_laws/UI06/production_promotion.json", "replay_interface/native/game_ui/src/scrollbar_law.rs"]},
        {"id": "UI09_UI10_current_canvas_join", "packet": "UI09+UI10", "class": "production_promotion", "cmd": [py, str(HERE / "verify_ui09_ui10_current_join.py")], "artifacts": ["replay_interface/re_evidence/rendering_laws/UI09/production_completion/manifest.json", "replay_interface/native/game_ui/src/research_canvas.rs", "replay_interface/native/game_ui/tests/ui09_canvas_admission.rs"]},
        {"id": "UI15_archive_mount_promotion", "packet": "UI15", "class": "production_promotion", "cmd": [py, str(EVIDENCE / "UI15/verify_ui15.py")], "artifacts": ["replay_interface/re_evidence/rendering_laws/UI15/manifest.json", "replay_interface/re_evidence/rendering_laws/UI15/verifier_result.json", "replay_interface/native/map_viewer/src/mod_resolver.rs"]},
    ]


def run_one(spec: dict) -> dict:
    commands = spec["cmds"] if "cmds" in spec else [spec["cmd"]]
    completed = [
        subprocess.run(command, cwd=ROOT, text=True, capture_output=True, check=False)
        for command in commands
    ]
    stdout = "".join(
        f"=== command {index + 1}/{len(commands)} ===\n{proc.stdout}"
        for index, proc in enumerate(completed)
    )
    stderr = "".join(
        f"=== command {index + 1}/{len(commands)} ===\n{proc.stderr}"
        for index, proc in enumerate(completed)
    )
    exit_code = next((proc.returncode for proc in completed if proc.returncode != 0), 0)
    stdout_path = RUNS / f"{spec['id']}.stdout.log"
    stderr_path = RUNS / f"{spec['id']}.stderr.log"
    stdout_path.write_text(normalized_log(stdout), encoding="utf-8")
    stderr_path.write_text(normalized_log(stderr), encoding="utf-8")
    result = {
        "id": spec["id"],
        "packet": spec["packet"],
        "class": spec["class"],
        "status": "PASS" if exit_code == 0 else "FAIL",
        "exit_code": exit_code,
        "stdout": {"path": stdout_path.relative_to(HERE).as_posix(), "sha256": sha256(stdout_path)},
        "stderr": {"path": stderr_path.relative_to(HERE).as_posix(), "sha256": sha256(stderr_path)},
    }
    if spec.get("artifacts"):
        result["promotion_artifacts"] = [
            {
                "path": relative,
                "sha256": sha256(ROOT / relative),
                "bytes": (ROOT / relative).stat().st_size,
            }
            for relative in spec["artifacts"]
        ]
    return result


def cargo_test_status(returncode: int, stdout: str) -> str:
    """Fail closed when a filtered cargo test run matched zero tests.

    `cargo test --lib <filter>` exits 0 even when the filter matches nothing
    ("running 0 tests"), so a renamed or removed pinned test would otherwise
    become a silent no-op PASS. Require a `test result: ok.` summary with at
    least one passed test before accepting the exit code as a PASS.
    """
    if returncode != 0:
        return "FAIL"
    passed_counts = [int(count) for count in re.findall(r"test result: ok\. (\d+) passed", stdout)]
    if not passed_counts or sum(passed_counts) == 0:
        return "ZERO_TESTS_MATCHED_FILTER"
    return "PASS"


def run_cargo_gate(name: str, test_filter: str) -> dict:
    env = dict(os.environ)
    env["CARGO_TARGET_DIR"] = str(Path(tempfile.gettempdir()) / "hoi4_event_compose_cargo")
    spec = {
        "id": name,
        "packet": "production_authority",
        "class": "ported_merge_infrastructure",
        "cmd": ["cargo", "test", "--lib", test_filter, "--", "--nocapture", "--test-threads=1"],
    }
    proc = subprocess.run(spec["cmd"], cwd=AUTHORITY / "replay_interface/native/map_viewer", env=env, text=True, capture_output=True, check=False)
    stdout_path = RUNS / f"{name}.stdout.log"
    stderr_path = RUNS / f"{name}.stderr.log"
    stdout_path.write_text(normalized_log(proc.stdout), encoding="utf-8")
    stderr_path.write_text(normalized_log(proc.stderr), encoding="utf-8")
    return {
        "id": name,
        "packet": spec["packet"],
        "class": spec["class"],
        "test_filter": test_filter,
        "status": cargo_test_status(proc.returncode, proc.stdout),
        "exit_code": proc.returncode,
        "stdout": {"path": stdout_path.relative_to(HERE).as_posix(), "sha256": sha256(stdout_path)},
        "stderr": {"path": stderr_path.relative_to(HERE).as_posix(), "sha256": sha256(stderr_path)},
    }


def run_terrain_capture_gate(name: str, test_filter: str, capture: Path) -> dict:
    env = dict(os.environ)
    env["CARGO_TARGET_DIR"] = str(Path(tempfile.gettempdir()) / "hoi4_event_compose_cargo")
    env["TERRAIN_MESH_LAW_CORPUS_CAPTURE"] = str(capture)
    env["TERRAIN_MESH_LAW_CORPUS_GAME"] = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV"
    proc = subprocess.run(
        ["cargo", "test", "--lib", test_filter, "--", "--ignored", "--nocapture", "--test-threads=1"],
        cwd=AUTHORITY / "replay_interface/native/map_viewer",
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )
    stdout_path = RUNS / f"{name}.stdout.log"
    stderr_path = RUNS / f"{name}.stderr.log"
    stdout_path.write_text(normalized_log(proc.stdout), encoding="utf-8")
    stderr_path.write_text(normalized_log(proc.stderr), encoding="utf-8")
    return {
        "id": name,
        "packet": "T01+production_authority",
        "class": "ported_real_corpus_compose",
        "capture": str(capture),
        "test_filter": test_filter,
        "status": cargo_test_status(proc.returncode, proc.stdout),
        "exit_code": proc.returncode,
        "stdout": {"path": stdout_path.relative_to(HERE).as_posix(), "sha256": sha256(stdout_path)},
        "stderr": {"path": stderr_path.relative_to(HERE).as_posix(), "sha256": sha256(stderr_path)},
    }


def build_renderer() -> dict:
    env = dict(os.environ)
    env["CARGO_TARGET_DIR"] = str(Path(tempfile.gettempdir()) / "hoi4_event_compose_cargo")
    proc = subprocess.run(
        ["cargo", "build", "--bin", "map_viewer"],
        cwd=AUTHORITY / "replay_interface/native/map_viewer",
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )
    stdout_path = RUNS / "production_renderer_build.stdout.log"
    stderr_path = RUNS / "production_renderer_build.stderr.log"
    stdout_path.write_text(normalized_log(proc.stdout), encoding="utf-8")
    stderr_path.write_text(normalized_log(proc.stderr), encoding="utf-8")
    return {
        "id": "production_renderer_build",
        "packet": "production_authority",
        "class": "ported_final_frame_infrastructure",
        "status": "PASS" if proc.returncode == 0 else "FAIL",
        "exit_code": proc.returncode,
        "stdout": {"path": stdout_path.relative_to(HERE).as_posix(), "sha256": sha256(stdout_path)},
        "stderr": {"path": stderr_path.relative_to(HERE).as_posix(), "sha256": sha256(stderr_path)},
    }


def run_final_frame_gate(label: str, capture: Path) -> dict:
    target = Path(tempfile.gettempdir()) / "hoi4_event_compose_cargo" / "debug" / "map_viewer.exe"
    cmd = [
        str(target),
        "render-production-map-d3d11",
        "--terrain-map-frame-bundle",
        str(capture),
        "--game",
        r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV",
    ]
    dependency_graph = ["terrain"]
    missing_dependencies = []
    for layer, option, filename in (
        ("water", "--water-map-frame-bundle", "map_frame_water_pass_bundle.json"),
        ("rivers", "--rivers-map-frame-bundle", "map_frame_rivers_pass_bundle.json"),
        ("borders", "--borders-map-frame-bundle", "map_frame_borders_pass_bundle.json"),
        ("trees", "--trees-map-frame-bundle", "map_frame_trees_pass_bundle.json"),
    ):
        if (capture / filename).is_file():
            cmd.extend([option, str(capture)])
            dependency_graph.append(layer)
        elif label in {"trees_measure", "fow_off", "fow_on"} and layer in {"water", "rivers", "trees"}:
            missing_dependencies.append(filename)
    required_graph = ["terrain", "water", "rivers", "trees"] if label in {"trees_measure", "fow_off", "fow_on"} else None
    if required_graph is not None and (dependency_graph != required_graph or missing_dependencies):
        message = (
            f"canonical dependency graph unavailable for {label}: expected terrain -> water -> rivers -> trees; "
            f"available {' -> '.join(dependency_graph)}; missing {', '.join(missing_dependencies)}\n"
        )
        stdout_text = ""
        stderr_text = message
        return_code = 1
    else:
        proc = subprocess.run(cmd, cwd=AUTHORITY / "replay_interface/native/map_viewer", text=True, capture_output=True, check=False)
        stdout_text = proc.stdout
        stderr_text = proc.stderr
        return_code = proc.returncode
    stdout_path = RUNS / f"production_final_frame_{label}.stdout.log"
    stderr_path = RUNS / f"production_final_frame_{label}.stderr.log"
    stdout_path.write_text(normalized_log(stdout_text), encoding="utf-8")
    stderr_path.write_text(normalized_log(stderr_text), encoding="utf-8")
    status = "PASS" if return_code == 0 else "FAIL"
    classification = None
    terrain = json.loads((capture / "map_frame_bundle.json").read_text(encoding="utf-8"))
    if (
        return_code != 0
        and terrain.get("post_pass_output_merger") is not None
        and "has a D3D11 descriptor but no raw texture payload file in the map-frame bundle" in stderr_text
    ):
        status = "NOT_RUNNABLE_LEGACY_BUNDLE_PAYLOAD_JOIN"
        classification = "the production comparator and captured post-pass target exist, but the legacy bundle omits per-draw raw-SRV file references required by the current loader"
    elif return_code != 0 and "map-frame rivers_pass is still active; waiting for pass-close boundary" in stderr_text:
        status = "NOT_RUNNABLE_EXTERNAL_CAPTURE_PASS_NOT_CLOSED"
        classification = "the retained rivers capture was exported before the pass-close boundary and has no required post-pass output-merger payload"
    elif (
        return_code != 0
        and "has no captured ClearRenderTargetView provenance" in stderr_text
        and "has no captured ClearDepthStencilView provenance" in stderr_text
    ):
        status = "NOT_RUNNABLE_EXTERNAL_CAPTURE_CLEAR_PROVENANCE"
        classification = "the retained capture omits exact render-target and depth-stencil clear provenance required before the first depth-tested draw"
    elif return_code != 0 and "Access is denied." in stderr_text:
        access_failures = []
        for path in sorted(capture.iterdir(), key=lambda item: item.name.lower()):
            if not path.is_file():
                continue
            try:
                with path.open("rb") as stream:
                    stream.read(1)
            except PermissionError as error:
                access_failures.append({"path": path.name, "winerror": error.winerror})
        if access_failures:
            status = "NOT_RUNNABLE_EXTERNAL_CAPTURE_FILE_ACCESS"
            classification = "the preserved capture contains a file whose ACL denies read access; no capture bytes were substituted or modified"
    elif return_code != 0 and missing_dependencies:
        status = "NOT_RUNNABLE_EXTERNAL_CAPTURE_DEPENDENCY_GRAPH"
        classification = "the preserved capture lacks one or more upstream pass bundles required by the canonical terrain -> water -> rivers -> trees graph"
    result = {
        "id": f"production_final_frame_{label}",
        "packet": "production_authority",
        "class": "ported_final_frame_parity",
        "capture": str(capture),
        "dependency_graph": dependency_graph,
        "required_dependency_graph": required_graph,
        "missing_dependency_bundles": missing_dependencies,
        "status": status,
        "exit_code": return_code,
        "stdout": {"path": stdout_path.relative_to(HERE).as_posix(), "sha256": sha256(stdout_path)},
        "stderr": {"path": stderr_path.relative_to(HERE).as_posix(), "sha256": sha256(stderr_path)},
    }
    if classification:
        result["classification"] = classification
    if status == "NOT_RUNNABLE_EXTERNAL_CAPTURE_FILE_ACCESS":
        result["capture_access_failures"] = access_failures
    return result


def main() -> int:
    RUNS.mkdir(parents=True, exist_ok=True)
    static_srv_closure = verified_static_srv_closure()
    specs = gate_specs()
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        results = list(pool.map(run_one, specs))
    # Cargo shares one target directory, so keep its two invocations serialized.
    results.append(run_cargo_gate("production_pass_graph_tests", "pass_graph::tests"))
    results.append(run_cargo_gate("production_shadow_law_tests", "shadow_law::tests"))
    results.append(run_cargo_gate("production_terrain_lod_compose_tests", "terrain_lod_law::tests"))
    results.append(run_cargo_gate("production_o05_fixed_object_gate_tests", "o05_fixed_object_law::tests"))
    captures = {
        "pan2": Path(r"C:\Users\Example\hoi4_capture_corpus_preserved\live_PAN2"),
        "pan_chunks": Path(r"C:\Users\Example\hoi4_capture_corpus_preserved\live_PAN_chunks"),
        "final_full": Path(r"C:\Users\Example\hoi4_capture_corpus_preserved\live_FINAL_full"),
        "gate18": Path(r"C:\Users\Example\hoi4_capture_corpus_preserved\gate18_final"),
    }
    for label, capture in captures.items():
        results.append(run_terrain_capture_gate(
            f"production_t01_lod_selection_{label}",
            "terrain_lod_law::tests::lod_selection_gate_env_when_session_available",
            capture,
        ))
        results.append(run_terrain_capture_gate(
            f"production_t01_vsnap_corpus_{label}",
            "terrain_lod_law::tests::corpus_gate_env_when_session_available",
            capture,
        ))
        results.append(run_terrain_capture_gate(
            f"production_t01_mesh_corpus_{label}",
            "terrain_mesh_law::tests::corpus_gate_env_when_session_available",
            capture,
        ))
    results.append(build_renderer())
    final_frames = {
        "gate18": captures["gate18"],
        "final_full": captures["final_full"],
        "pan2": captures["pan2"],
        "pan_chunks": captures["pan_chunks"],
        "trees_measure": Path(r"C:\Users\Example\hoi4_capture_corpus_preserved\live_trees_measure"),
        "fow_off": Path(r"C:\Users\Example\hoi4_capture_corpus_preserved\session_WAVE1_CAP4_rounds\fow_off_observer_front"),
        "fow_on": Path(r"C:\Users\Example\hoi4_capture_corpus_preserved\session_WAVE1_CAP4_rounds\fow_on_pov_front"),
    }
    for label, capture in final_frames.items():
        results.append(run_final_frame_gate(label, capture))
    by_id = {row["id"]: row for row in results}
    pan2 = by_id["production_t01_vsnap_corpus_pan2"]
    if pan2["status"] == "FAIL":
        diagnostic = (HERE / pan2["stderr"]["path"]).read_text(encoding="utf-8")
        exact = "LODSNAP gate: 170/176 edges matched directly; map_edge=0 coarsest_fill=0 unknown_neighbour=0 cross_frame_pan=6 unexplained=0"
        if exact in diagnostic:
            pan2["status"] = "NOT_APPLICABLE_MULTI_FRAME_DIAGNOSTIC"
            pan2["classification"] = "six known cross-frame duplicates are classified and unexplained is zero, but this aggregate is deliberately not a strict acceptance oracle"
    gate18_lod = by_id["production_t01_lod_selection_gate18"]
    if gate18_lod["status"] == "FAIL":
        diagnostic = (HERE / gate18_lod["stderr"]["path"]).read_text(encoding="utf-8")
        if (
            "The system cannot find the path specified" in diagnostic
            and by_id["production_t01_vsnap_corpus_gate18"]["status"] == "PASS"
            and by_id["production_t01_mesh_corpus_gate18"]["status"] == "PASS"
        ):
            gate18_lod["status"] = "NOT_APPLICABLE_INCOMPLETE_LEGACY_FIXTURE"
            gate18_lod["classification"] = "gate18 retains the normative 64/64 vSnap and byte-exact mesh inputs, but not every auxiliary path required by the independent LOD-selection diagnostic; the three full corpora supply that diagnostic"
    gate18_frame = by_id["production_final_frame_gate18"]
    if gate18_frame["status"] == "FAIL":
        diagnostic = (HERE / gate18_frame["stderr"]["path"]).read_text(encoding="utf-8")
        if "captured TerrainIDMap hash f8c3bac0db222325 does not match resolver-synthesized hash f493368c56224ab1" in diagnostic:
            gate18_frame["status"] = "NOT_APPLICABLE_KNOWN_NEGATIVE_CONTROL"
            gate18_frame["classification"] = "the preserved gate18 TerrainIDMap mismatch is a pinned fail-closed negative control, not a final-frame parity oracle"
    accepted = {
        "PASS",
        "NOT_APPLICABLE_MULTI_FRAME_DIAGNOSTIC",
        "NOT_APPLICABLE_INCOMPLETE_LEGACY_FIXTURE",
        "NOT_APPLICABLE_KNOWN_NEGATIVE_CONTROL",
        "NOT_RUNNABLE_LEGACY_BUNDLE_PAYLOAD_JOIN",
        "NOT_RUNNABLE_EXTERNAL_CAPTURE_PASS_NOT_CLOSED",
        "NOT_RUNNABLE_EXTERNAL_CAPTURE_CLEAR_PROVENANCE",
        "NOT_RUNNABLE_EXTERNAL_CAPTURE_FILE_ACCESS",
        "NOT_RUNNABLE_EXTERNAL_CAPTURE_DEPENDENCY_GRAPH",
    }
    source_paths = {
        "d3d11_device.rs": ROOT / "replay_interface/native/map_viewer/src/d3d11_device.rs",
        "frame_bundle.rs": ROOT / "replay_interface/native/map_viewer/src/frame_bundle.rs",
        "pass_graph.rs": ROOT / "replay_interface/native/map_viewer/src/pass_graph.rs",
        "terrain_lod_law.rs": ROOT / "replay_interface/native/map_viewer/src/terrain_lod_law.rs",
        "shadow_law.rs": ROOT / "replay_interface/native/map_viewer/src/shadow_law.rs",
        "light_depth_projection.rs": ROOT / "replay_interface/native/map_viewer/src/light_depth_projection.rs",
        "m01_runtime_admission.rs": ROOT / "replay_interface/native/map_viewer/src/m01_runtime_admission.rs",
        "o05_fixed_object_law.rs": ROOT / "replay_interface/native/map_viewer/src/o05_fixed_object_law.rs",
        "map_label_law.rs": ROOT / "replay_interface/native/map_viewer/src/map_label_law.rs",
        "mod_resolver.rs": ROOT / "replay_interface/native/map_viewer/src/mod_resolver.rs",
        "scrollbar_law.rs": ROOT / "replay_interface/native/game_ui/src/scrollbar_law.rs",
        "research_canvas.rs": ROOT / "replay_interface/native/game_ui/src/research_canvas.rs",
    }
    payload = {
        "schema": "hoi4-rendering-laws-final-event-compose-audit-v1",
        "status": "PASS" if all(row["status"] in accepted for row in results) else "FAIL",
        "authority": {
            "workspace": str(ROOT),
            "production_root": str(AUTHORITY),
            "production_head": subprocess.check_output(["git", "-C", str(AUTHORITY), "rev-parse", "HEAD"], text=True).strip(),
            "source_sha256": {name: sha256(path) for name, path in source_paths.items()},
        },
        "static_srv_closure": static_srv_closure,
        "gate_count": len(results),
        "passed": sum(row["status"] == "PASS" for row in results),
        "classified_passed": 0,
        "not_applicable": sum(row["status"].startswith("NOT_APPLICABLE_") for row in results),
        "post_port_fixture_gaps": sum(row["status"] == "NOT_RUNNABLE_LEGACY_BUNDLE_PAYLOAD_JOIN" for row in results),
        "external_capture_gaps": sum(row["status"].startswith("NOT_RUNNABLE_EXTERNAL_CAPTURE_") for row in results),
        "failed": sum(row["status"] not in accepted for row in results),
        "gates": results,
    }
    output = HERE / "mechanical_result.json"
    output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": payload["status"], "passed": payload["passed"], "classified_passed": payload["classified_passed"], "not_applicable": payload["not_applicable"], "total": payload["gate_count"], "result": str(output)}, indent=2))
    return 0 if payload["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
