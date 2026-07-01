#!/usr/bin/env python3
"""Bounded focus pause fixture watcher.

This is a lane-local fixture contract, not production capture. It proves the
current build can be watched for paused-focus evidence without shared collector
edits and records the exact oracle needed to promote a pause root.
"""

from __future__ import annotations

import argparse
import json
import time
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import probe_current_roots as roots
import probe_focus_pause_candidates as focus_gate


GS_CURRENT_DATE_OFFSET = 0x468
EXPECTED_SHA256 = roots.EXPECTED_SHA256


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def load_optional_json(path: Path | None) -> dict[str, Any] | None:
    if path is None:
        return None
    return json.loads(path.read_text(encoding="utf-8"))


def read_game_hour_raw(proc: roots.winmem.Process, module_base: int) -> int | None:
    game_state = roots.safe(lambda: proc.u64(module_base + roots.C_GAME_STATE_GLOBAL))
    if not game_state:
        return None
    return roots.safe(lambda: proc.i32(game_state + GS_CURRENT_DATE_OFFSET))


def live_sample() -> dict[str, Any]:
    proc = roots.winmem.open_process("hoi4.exe", "hoi4.exe")
    if proc is None:
        return {
            "ok": False,
            "error": "hoi4.exe_not_running",
            "build_gate": {
                "expected_sha256": EXPECTED_SHA256,
                "actual_sha256": None,
                "ok": False,
            },
        }
    with proc:
        fp = proc.fingerprint()
        module_base = proc.module_base
        rows, process, build_gate = focus_gate.live_rows()
        return {
            "ok": bool(build_gate.get("ok")) and bool(rows),
            "game_hour_raw": read_game_hour_raw(proc, module_base),
            "process": process,
            "build_gate": build_gate,
            "focus_status_rows": len(rows),
            "focus_context_rows": sum(1 for row in rows if focus_gate.has_focus_context(row)),
            "active_tokens": sorted({
                str(row.get("current") or row.get("continuous"))
                for row in rows
                if row.get("current") or row.get("continuous")
            })[:32],
            "progress_by_key": {
                f"{row.get('tag_db_id')}:{row.get('current') or row.get('continuous') or ''}": row.get("progress_raw")
                for row in rows
                if focus_gate.has_focus_context(row)
            },
            "window_by_key": {
                f"{row.get('tag_db_id')}:{row.get('current') or row.get('continuous') or ''}": (row.get("focus") or {}).get("pause_candidate_window_0x20_0x3f_hex")
                for row in rows
                if focus_gate.has_focus_context(row)
            },
            "fingerprint_sha256": fp.get("sha256"),
        }


def summarize_samples(samples: list[dict[str, Any]]) -> dict[str, Any]:
    hours = [
        sample.get("game_hour_raw")
        for sample in samples
        if isinstance(sample.get("game_hour_raw"), int)
    ]
    unique_hours = sorted(set(hours))
    progress_changes = 0
    window_changes = 0
    by_key_progress: dict[str, set[Any]] = defaultdict(set)
    by_key_window: dict[str, set[Any]] = defaultdict(set)
    token_counts: Counter[str] = Counter()
    for sample in samples:
        for key, value in (sample.get("progress_by_key") or {}).items():
            by_key_progress[str(key)].add(value)
        for key, value in (sample.get("window_by_key") or {}).items():
            by_key_window[str(key)].add(value)
        for token in sample.get("active_tokens") or []:
            token_counts[str(token)] += 1
    for values in by_key_progress.values():
        if len(values) > 1:
            progress_changes += 1
    for values in by_key_window.values():
        if len(values) > 1:
            window_changes += 1
    return {
        "sample_count": len(samples),
        "unique_game_hour_count": len(unique_hours),
        "first_game_hour_raw": unique_hours[0] if unique_hours else None,
        "last_game_hour_raw": unique_hours[-1] if unique_hours else None,
        "focus_status_rows_min": min((sample.get("focus_status_rows", 0) for sample in samples), default=0),
        "focus_status_rows_max": max((sample.get("focus_status_rows", 0) for sample in samples), default=0),
        "focus_context_rows_max": max((sample.get("focus_context_rows", 0) for sample in samples), default=0),
        "active_token_examples": [token for token, _count in token_counts.most_common(20)],
        "tracked_focus_keys": len(by_key_progress),
        "focus_keys_with_progress_changes": progress_changes,
        "focus_keys_with_candidate_window_changes": window_changes,
    }


def build_report(args: argparse.Namespace) -> dict[str, Any]:
    focus_candidate_gate = load_optional_json(args.focus_gate)
    samples: list[dict[str, Any]] = []
    started = time.monotonic()
    deadline = started + args.duration_s
    while time.monotonic() <= deadline:
        sample = live_sample()
        samples.append(sample)
        if args.min_game_hours > 0:
            summary = summarize_samples(samples)
            if int(summary["unique_game_hour_count"]) >= args.min_game_hours:
                break
        time.sleep(max(args.sample_ms, 50) / 1000.0)

    summary = summarize_samples(samples)
    all_samples_ok = all(sample.get("ok") for sample in samples)
    focus_window_rejected = bool(
        focus_candidate_gate
        and not focus_candidate_gate.get("inconclusive_byte_candidate_offsets")
        and len(focus_candidate_gate.get("rejected_byte_candidate_offsets") or []) == 24
    )
    fixture_status = "BLOCKED_NO_PAUSED_TRUE_ORACLE"
    if args.oracle_jsonl:
        fixture_status = "ORACLE_INPUT_DECLARED_NOT_EVALUATED"

    return {
        "schema": "focus_research_decisions_depth_live_focus_pause_fixture_watch_v1",
        "lane": "focus_research_decisions_depth_live",
        "created_utc": utc_now(),
        "completion_status": "NOT_COMPLETE",
        "backlog_reduced": True,
        "duration_s": args.duration_s,
        "sample_ms": args.sample_ms,
        "requested_min_game_hours": args.min_game_hours,
        "summary": summary,
        "build_gate": samples[-1].get("build_gate") if samples else {},
        "focus_candidate_gate": str(args.focus_gate) if args.focus_gate else None,
        "focus_candidate_window_rejected": focus_window_rejected,
        "fixture_status": fixture_status,
        "oracle_contract": {
            "required_fields": [
                "game_hour_raw",
                "tag_db_id",
                "current_focus_token_or_continuous_focus_token",
                "paused_expected_true",
                "source",
            ],
            "accepted_sources": [
                "same-frame UI observation",
                "save-aligned paused focus state",
                "dynamic hook proof with arguments",
            ],
            "promotion_rule": (
                "A pause root can be promoted only if a watched row has the same "
                "tag/focus/hour as a paused_expected_true oracle and the candidate "
                "field changes consistently while non-paused controls remain false."
            ),
        },
        "remaining_blocker": {
            "field": "focus_paused_state",
            "status": "BLOCKED",
            "reason": (
                "The original CFocusStatus+0x20..+0x37 candidate window is rejected. "
                "No paused=true oracle or wider root is present in this bounded watch."
            ),
            "next_evidence_needed": (
                "Run this watcher with a same-build paused=true UI/save/dynamic oracle, "
                "or extend the root search outside the rejected candidate window."
            ),
        },
        "gate": {
            "ok": bool(samples) and all_samples_ok and focus_window_rejected,
            "errors": [] if (samples and all_samples_ok and focus_window_rejected) else [
                "watch samples failed or focus candidate window was not rejected"
            ],
        },
    }


def write_markdown(path: Path, report: dict[str, Any]) -> None:
    summary = report["summary"]
    lines = [
        "# Focus Pause Fixture Watch",
        "",
        f"Created UTC: `{report['created_utc']}`",
        f"Completion status: `{report['completion_status']}`",
        f"Backlog reduced: `{str(report['backlog_reduced']).lower()}`",
        f"Fixture status: `{report['fixture_status']}`",
        "",
        "## Watch Summary",
        "",
        f"- Samples: `{summary['sample_count']}`.",
        f"- Unique game hours: `{summary['unique_game_hour_count']}`.",
        f"- Focus status rows min/max: `{summary['focus_status_rows_min']}` / `{summary['focus_status_rows_max']}`.",
        f"- Focus context rows max: `{summary['focus_context_rows_max']}`.",
        f"- Focus keys with progress changes: `{summary['focus_keys_with_progress_changes']}`.",
        f"- Focus keys with candidate-window changes: `{summary['focus_keys_with_candidate_window_changes']}`.",
        f"- Candidate window rejected before watch: `{str(report['focus_candidate_window_rejected']).lower()}`.",
        "",
        "## Oracle Contract",
        "",
        f"- Required fields: `{', '.join(report['oracle_contract']['required_fields'])}`.",
        f"- Accepted sources: `{', '.join(report['oracle_contract']['accepted_sources'])}`.",
        f"- Promotion rule: {report['oracle_contract']['promotion_rule']}",
        "",
        "## Remaining Blocker",
        "",
        f"- `{report['remaining_blocker']['field']}`: {report['remaining_blocker']['reason']}",
        f"  Next evidence: {report['remaining_blocker']['next_evidence_needed']}",
    ]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def run() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--duration-s", type=float, default=12.0)
    parser.add_argument("--sample-ms", type=int, default=500)
    parser.add_argument("--min-game-hours", type=int, default=2)
    parser.add_argument("--focus-gate", type=Path, default=None)
    parser.add_argument("--oracle-jsonl", type=Path, default=None)
    args = parser.parse_args()

    report = build_report(args)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "focus_pause_fixture_watch.json").write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    write_markdown(args.out_dir / "FOCUS_PAUSE_FIXTURE_WATCH.md", report)
    print(json.dumps({
        "ok": report["gate"]["ok"],
        "fixture_status": report["fixture_status"],
        "unique_game_hour_count": report["summary"]["unique_game_hour_count"],
        "focus_candidate_window_rejected": report["focus_candidate_window_rejected"],
        "out_dir": str(args.out_dir),
    }, indent=2))
    return 0 if report["gate"]["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(run())
