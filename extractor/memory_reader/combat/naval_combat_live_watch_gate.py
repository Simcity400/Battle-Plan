"""Bounded watcher/gate for current-build live naval combat bridge proof.

This is a read-only native discovery gate until an active naval battle is
detected. It reuses ``naval_combat_live_fixture_probe`` for fail-closed
validation, emits the bridge rows for the existing DLL path, and writes the
exact recorder/proof command script needed to finish the lane once a two-sided
fixture appears.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional

THIS_DIR = Path(__file__).resolve().parent
MEMORY_READER = THIS_DIR.parent
REPO_ROOT = MEMORY_READER.parent
for _path in (str(THIS_DIR), str(MEMORY_READER), str(REPO_ROOT)):
    if _path not in sys.path:
        sys.path.insert(0, _path)

from naval_combat_live_fixture_probe import (  # noqa: E402
    encode_member_bridge_jsonl,
    run as run_fixture_probe,
)
from naval_combat_compact_lane import proof_from_bridge_text  # noqa: E402


def utc_now() -> str:
    return dt.datetime.utcnow().replace(microsecond=0).isoformat() + "Z"


def write_json(path: Path, payload: Dict[str, Any]) -> None:
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def ps_quote(value: Path) -> str:
    return "'" + str(value).replace("'", "''") + "'"


def build_proof_args(require_ended: bool, min_game_hours: int) -> List[str]:
    args = [
        "--min-game-hours",
        str(min_game_hours),
        "--require-raw-domain",
        "naval_air_combats",
        "--require-raw-domain",
        "naval_combat_members",
        "--require-raw-event-type",
        "naval_combat_started",
        "--require-raw-event-type",
        "naval_combat_updated",
    ]
    if require_ended:
        args.extend(["--require-raw-event-type", "naval_combat_ended"])
    return args


def write_followup_script(
    out_dir: Path,
    *,
    pid: int,
    collector_dll: Optional[Path],
    lane_compact_out_root: Path,
    record_out_root: Path,
    game_id: str,
    record_duration_s: float,
    sample_ms: int,
    until_game_hours: int,
    max_wall_s: float,
    require_ended: bool,
) -> Path:
    script_path = out_dir / "run_detected_compact_proof.ps1"
    bridge_path = out_dir / "bridge.txt"
    manifest_path = "$runDir\\session_manifest.json"
    proof_args = " ".join(build_proof_args(require_ended, until_game_hours))
    dll_value = str(collector_dll) if collector_dll is not None else "<path-to-collector-dll>"
    lane_compact_out = lane_compact_out_root / out_dir.name
    lines = [
        "$ErrorActionPreference = 'Stop'",
        "$repo = " + ps_quote(REPO_ROOT),
        "Set-Location $repo",
        "$bridgePath = " + ps_quote(bridge_path),
        "$bridge = (Get-Content -Raw $bridgePath).Trim()",
        "if (-not $bridge) { throw 'bridge.txt is empty; the watcher has not detected a validated fixture yet.' }",
        "python memory_reader\\combat\\naval_combat_compact_proof.py --bridge-file $bridgePath --out-dir "
        + ps_quote(lane_compact_out),
        "if ($LASTEXITCODE -ne 0) { throw \"lane-local compact proof failed with exit code $LASTEXITCODE\" }",
        "$env:NAVAL_COMBAT_BODY_BRIDGE = $bridge",
        "$collectorDll = '" + dll_value.replace("'", "''") + "'",
        "if ($collectorDll -eq '<path-to-collector-dll>' -or -not (Test-Path $collectorDll)) { throw 'Set --collector-dll or edit $collectorDll before running.' }",
        "$runOutput = python -c \"import os; from pathlib import Path; from live_capture.collector_session import record_collector_session; "
        "print(record_collector_session("
        f"{pid}, Path(r'$collectorDll'), Path(r'{record_out_root}'), "
        f"game_id='{game_id}', duration_s={record_duration_s!r}, sample_ms={sample_ms}, "
        f"until_game_hours={until_game_hours}, max_wall_s={max_wall_s!r}, "
        "require_hourly_cadence=True, naval_combat_body_bridge=os.environ['NAVAL_COMBAT_BODY_BRIDGE']))\"",
        "if ($LASTEXITCODE -ne 0) { throw \"collector session failed with exit code $LASTEXITCODE\" }",
        "$runDir = ($runOutput | Select-Object -Last 1).Trim()",
        "if (-not (Test-Path \"$runDir\\session_manifest.json\")) { throw \"recorder did not produce session_manifest.json under $runDir\" }",
        f"python -m live_capture.collector_proof_gate \"{manifest_path}\" {proof_args}",
        "if ($LASTEXITCODE -ne 0) { throw \"collector proof gate failed with exit code $LASTEXITCODE\" }",
    ]
    script_path.write_text("\n".join(lines) + "\n", encoding="ascii")
    return script_path


def run_lane_compact_followup(
    args: argparse.Namespace,
    bridge: str,
) -> Dict[str, Any]:
    if args.skip_lane_compact_proof:
        return {
            "status": "skipped",
            "reason": "--skip-lane-compact-proof",
        }
    lane_out_dir = args.lane_compact_out_root / args.out_dir.name
    try:
        report = proof_from_bridge_text(
            bridge,
            out_dir=lane_out_dir,
            game_hour_raw=0,
        )
    except Exception as exc:
        return {
            "status": "failed",
            "out_dir": str(lane_out_dir),
            "error": str(exc),
        }
    return {
        "status": report.get("status"),
        "out_dir": str(lane_out_dir),
        "bridge_row_count": report.get("bridge_row_count"),
        "compact_path": report.get("compact_path"),
        "gate_ok": report.get("gate", {}).get("ok"),
        "gate_errors": report.get("gate", {}).get("errors"),
        "gate_summary": report.get("gate", {}).get("summary"),
    }


def run_recorder_followup(
    args: argparse.Namespace,
    bridge: str,
) -> Dict[str, Any]:
    if not args.collector_dll:
        return {
            "status": "not_run",
            "reason": "--run-recorder requires --collector-dll",
        }

    from live_capture.collector_session import record_collector_session  # noqa: WPS433

    run_dir = record_collector_session(
        args.pid,
        Path(args.collector_dll),
        args.record_out_root,
        game_id=args.game_id,
        duration_s=args.record_duration_s,
        sample_ms=args.sample_ms,
        until_game_hours=args.until_game_hours,
        max_wall_s=args.max_wall_s,
        require_hourly_cadence=True,
        naval_combat_body_bridge=bridge,
    )
    proof_cmd = [
        sys.executable,
        "-m",
        "live_capture.collector_proof_gate",
        str(run_dir / "session_manifest.json"),
        *build_proof_args(args.require_ended, args.until_game_hours),
    ]
    completed = subprocess.run(
        proof_cmd,
        cwd=str(REPO_ROOT),
        text=True,
        capture_output=True,
        check=False,
    )
    return {
        "status": "ran",
        "run_dir": str(run_dir),
        "proof_command": proof_cmd,
        "proof_returncode": completed.returncode,
        "proof_stdout": completed.stdout,
        "proof_stderr": completed.stderr,
    }


def compact_poll_summary(report: Dict[str, Any]) -> Dict[str, Any]:
    scan = report.get("scan") or {}
    return {
        "created_utc": report.get("created_utc"),
        "status": report.get("status"),
        "sha256_ok": (report.get("root") or {}).get("sha256_ok"),
        "body_hit_count": report.get("body_hit_count", 0),
        "validated_combat_count": report.get("validated_combat_count", 0),
        "bridge_row_count": report.get("bridge_row_count", 0),
        "member_bridge_row_count": report.get("member_bridge_row_count", 0),
        "cship_resolved_member_count": report.get("cship_resolved_member_count", 0),
        "cship_unresolved_member_count": report.get("cship_unresolved_member_count", 0),
        "scan_status": scan.get("status"),
        "scan_elapsed_s": scan.get("elapsed_s"),
    }


def run_poll(args: argparse.Namespace) -> Dict[str, Any]:
    probe_args = argparse.Namespace(
        pid=args.pid,
        workers=args.workers,
        require_two_sided=args.require_two_sided,
        require_cship_join=args.require_cship_join,
        out_dir=None,
    )
    return run_fixture_probe(probe_args)


def terminal_statuses() -> Iterable[str]:
    return (
        "build_mismatch",
        "scan_failed",
        "candidate_rejected",
        "candidate_rejected_cship_member_join",
        "validated_member_rows_missing_cship_fixture",
        "validated_active_fixture",
    )


def run(args: argparse.Namespace) -> Dict[str, Any]:
    out_dir = args.out_dir
    out_dir.mkdir(parents=True, exist_ok=True)
    script_path = write_followup_script(
        out_dir,
        pid=args.pid,
        collector_dll=Path(args.collector_dll) if args.collector_dll else None,
        lane_compact_out_root=args.lane_compact_out_root,
        record_out_root=args.record_out_root,
        game_id=args.game_id,
        record_duration_s=args.record_duration_s,
        sample_ms=args.sample_ms,
        until_game_hours=args.until_game_hours,
        max_wall_s=args.max_wall_s,
        require_ended=args.require_ended,
    )

    deadline = time.monotonic() + args.timeout_s
    polls: List[Dict[str, Any]] = []
    final_report: Optional[Dict[str, Any]] = None
    poll_index = 0
    while True:
        poll_started = time.monotonic()
        report = run_poll(args)
        poll_path = out_dir / ("poll_%04d.json" % poll_index)
        write_json(poll_path, report)
        summary = compact_poll_summary(report)
        summary["path"] = str(poll_path)
        polls.append(summary)
        final_report = report

        bridge = report.get("bridge") or ""
        if bridge:
            (out_dir / "bridge.txt").write_text(bridge, encoding="ascii")
        member_bridge_rows = report.get("member_bridge_rows") or []
        if member_bridge_rows:
            (out_dir / "member_bridge.jsonl").write_text(
                encode_member_bridge_jsonl(member_bridge_rows),
                encoding="utf-8",
            )

        status = str(report.get("status") or "unknown")
        if status == "validated_active_fixture":
            lane_compact_followup = run_lane_compact_followup(args, bridge)
            followup = (
                run_recorder_followup(args, bridge)
                if args.run_recorder and lane_compact_followup.get("status") == "passed" else
                {"status": "scripted", "script": str(script_path)}
            )
            if args.run_recorder and lane_compact_followup.get("status") != "passed":
                followup = {
                    "status": "not_run",
                    "reason": "lane-local compact proof did not pass",
                }
            return {
                "watcher": "naval_combat_live_watch_gate_v1",
                "created_utc": utc_now(),
                "status": "validated_fixture_detected",
                "pid": args.pid,
                "timeout_s": args.timeout_s,
                "poll_s": args.poll_s,
                "require_two_sided": args.require_two_sided,
                "require_ended": args.require_ended,
                "poll_count": len(polls),
                "polls": polls,
                "final_fixture_report": report,
                "bridge_path": str(out_dir / "bridge.txt"),
                "member_bridge_path": str(out_dir / "member_bridge.jsonl"),
                "bridge_row_count": report.get("bridge_row_count", 0),
                "member_bridge_row_count": report.get("member_bridge_row_count", 0),
                "cship_resolved_member_count": report.get("cship_resolved_member_count", 0),
                "cship_unresolved_member_count": report.get("cship_unresolved_member_count", 0),
                "followup_script": str(script_path),
                "lane_compact_followup": lane_compact_followup,
                "recorder_followup": followup,
            }

        if status in terminal_statuses() and status != "validated_active_fixture":
            break

        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        sleep_s = min(args.poll_s, remaining)
        elapsed = time.monotonic() - poll_started
        if sleep_s > 0 and elapsed < args.timeout_s:
            time.sleep(sleep_s)
        poll_index += 1

    final_status = str(final_report.get("status") if final_report else "no_poll")
    return {
        "watcher": "naval_combat_live_watch_gate_v1",
        "created_utc": utc_now(),
        "status": "timeout_no_validated_fixture" if final_status == "fixture_absent" else "stopped_without_fixture",
        "final_fixture_status": final_status,
        "pid": args.pid,
        "timeout_s": args.timeout_s,
        "poll_s": args.poll_s,
        "require_two_sided": args.require_two_sided,
        "require_ended": args.require_ended,
        "poll_count": len(polls),
        "polls": polls,
        "bridge_path": str(out_dir / "bridge.txt"),
        "member_bridge_path": str(out_dir / "member_bridge.jsonl"),
        "bridge_row_count": int(final_report.get("bridge_row_count", 0)) if final_report else 0,
        "member_bridge_row_count": int(final_report.get("member_bridge_row_count", 0)) if final_report else 0,
        "cship_resolved_member_count": int(final_report.get("cship_resolved_member_count", 0)) if final_report else 0,
        "cship_unresolved_member_count": int(final_report.get("cship_unresolved_member_count", 0)) if final_report else 0,
        "followup_script": str(script_path),
        "lane_compact_followup": {
            "status": "not_run",
            "reason": "no validated two-sided fixture bridge rows detected",
        },
        "recorder_followup": {
            "status": "not_run",
            "reason": "no validated two-sided fixture bridge rows detected",
        },
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, required=True)
    ap.add_argument("--workers", type=int, default=4)
    ap.add_argument("--timeout-s", type=float, default=900.0)
    ap.add_argument("--poll-s", type=float, default=10.0)
    ap.add_argument("--require-two-sided", action="store_true", default=True)
    ap.add_argument("--allow-one-sided", dest="require_two_sided", action="store_false")
    ap.add_argument("--require-cship-join", action=argparse.BooleanOptionalAction, default=True)
    ap.add_argument("--require-ended", action="store_true")
    ap.add_argument("--out-dir", type=Path, required=True)
    ap.add_argument("--collector-dll", type=Path, default=None)
    ap.add_argument("--run-recorder", action="store_true")
    ap.add_argument("--skip-lane-compact-proof", action="store_true")
    ap.add_argument(
        "--lane-compact-out-root",
        type=Path,
        default=Path("extractor/native/runs/naval_combat_live_compact_lane_detected"),
    )
    ap.add_argument("--record-out-root", type=Path, default=Path("replay/snapshots/_collector_session_naval_combat_live_watch_gate"))
    ap.add_argument("--game-id", default="naval_combat_live_watch_gate")
    ap.add_argument("--record-duration-s", type=float, default=180.0)
    ap.add_argument("--sample-ms", type=int, default=10)
    ap.add_argument("--until-game-hours", type=int, default=12)
    ap.add_argument("--max-wall-s", type=float, default=360.0)
    args = ap.parse_args()

    args.record_out_root = args.record_out_root.resolve()
    args.lane_compact_out_root = args.lane_compact_out_root.resolve()
    if args.timeout_s <= 0:
        raise SystemExit("--timeout-s must be > 0")
    if args.poll_s <= 0:
        raise SystemExit("--poll-s must be > 0")
    if args.run_recorder and not args.collector_dll:
        raise SystemExit("--run-recorder requires --collector-dll")

    report = run(args)
    out_path = args.out_dir / "watch_gate_summary.json"
    write_json(out_path, report)
    if not (args.out_dir / "bridge.txt").exists():
        (args.out_dir / "bridge.txt").write_text("", encoding="ascii")
    if not (args.out_dir / "member_bridge.jsonl").exists():
        (args.out_dir / "member_bridge.jsonl").write_text("", encoding="utf-8")

    print(json.dumps({
        "out_path": str(out_path),
        "status": report.get("status"),
        "final_fixture_status": report.get("final_fixture_status"),
        "poll_count": report.get("poll_count"),
        "bridge_row_count": report.get("bridge_row_count"),
        "member_bridge_row_count": report.get("member_bridge_row_count"),
        "cship_resolved_member_count": report.get("cship_resolved_member_count"),
        "cship_unresolved_member_count": report.get("cship_unresolved_member_count"),
        "followup_script": report.get("followup_script"),
        "lane_compact_followup": report.get("lane_compact_followup", {}).get("status"),
        "recorder_followup": report.get("recorder_followup", {}).get("status"),
    }, indent=2, sort_keys=True))
    return 0 if report.get("status") in (
        "timeout_no_validated_fixture",
        "validated_fixture_detected",
        "stopped_without_fixture",
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
