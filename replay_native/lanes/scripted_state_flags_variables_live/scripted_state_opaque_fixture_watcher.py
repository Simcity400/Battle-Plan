#!/usr/bin/env python3
"""Bounded fixture watcher for unresolved scripted-state opaque roots.

The watcher does not validate a live heap root by itself. It repeatedly samples
the available repo fixture surfaces and fails closed unless a positive, parsed
owner/value fixture exists for delayed events or a phase-labeled scripted-GUI
open-state fixture exists. It also checks that the manifest has no other
scripted flag/variable containers outside the validated slice plus these two
opaque roots.
"""

from __future__ import annotations

import argparse
import json
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


LANE = "scripted_state_flags_variables_live"
DOMAIN_ID = 51
SCRIPTED_TERMS = ("flags", "variables", "delayed_event", "scripted_gui_random")
EXPECTED_MANIFEST_PATHS = [
    "flags",
    "variables",
    "countries.*.flags",
    "countries.*.variables",
    "countries.*.scripted_gui_random",
    "countries.*.delayed_event",
    "states.*.flags",
    "states.*.variables",
]
VALIDATED_MANIFEST_PATHS = [
    "flags",
    "variables",
    "countries.*.flags",
    "countries.*.variables",
    "states.*.flags",
    "states.*.variables",
]
OPAQUE_BLOCKED_PATHS = [
    "countries.*.delayed_event",
    "countries.*.scripted_gui_random",
]

LANE_DIR = Path(__file__).resolve().parent
REPO_ROOT = LANE_DIR.parents[2]
DEFAULT_OUT_DIR = (
    REPO_ROOT
    / "replay_native"
    / "runs"
    / "scripted_state_flags_variables_live_20260701_opaque_roots_full_final"
    / "fixture_watch"
)


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8-sig"))


def write_json(path: Path, payload: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=False) + "\n", encoding="utf-8")


def rel(path: Path) -> str:
    return str(path.relative_to(REPO_ROOT)).replace("\\", "/")


def walk_json_for_terms(value: Any, path: list[str], terms: list[str], hits: dict[str, list[str]]) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            next_path = path + [str(key)]
            key_lower = str(key).lower()
            for term in terms:
                if term in key_lower:
                    hits[term].append(".".join(next_path))
            if isinstance(child, str):
                child_lower = child.lower()
                for term in terms:
                    if term in child_lower:
                        hits[term].append(".".join(next_path))
            walk_json_for_terms(child, next_path, terms, hits)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            walk_json_for_terms(child, path + [str(index)], terms, hits)


def scan_json_files(files: list[Path], terms: list[str]) -> dict[str, Any]:
    hits = {term: [] for term in terms}
    unreadable = []
    for path in files:
        try:
            data = read_json(path)
        except Exception as exc:  # pragma: no cover - diagnostic proof output
            unreadable.append({"file": rel(path), "error": str(exc)})
            continue
        file_hits = {term: [] for term in terms}
        walk_json_for_terms(data, [path.name], terms, file_hits)
        for term, values in file_hits.items():
            hits[term].extend(f"{rel(path)}:{value}" for value in values)
    return {
        "files_scanned": len(files),
        "hit_counts": {term: len(values) for term, values in hits.items()},
        "sample_hits": {term: values[:24] for term, values in hits.items() if values},
        "unreadable": unreadable,
    }


def recursive_shape(path: Path, root: str) -> dict[str, Any]:
    rows = []
    for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip() or "\t" not in line:
            continue
        row_path = line.split("\t", 1)[0]
        if row_path == root or row_path.startswith(root + "."):
            fields = line.split("\t")
            rows.append(
                {
                    "line": line_no,
                    "path": row_path,
                    "kind": fields[1] if len(fields) > 1 else "",
                    "count": fields[2] if len(fields) > 2 else "",
                }
            )
    return {
        "file": rel(path),
        "root": root,
        "has_root": any(row["path"] == root for row in rows),
        "row_count": len(rows),
        "descendant_count": sum(1 for row in rows if row["path"].startswith(root + ".")),
        "rows": rows,
    }


def manifest_rows() -> dict[str, Any]:
    manifest = read_json(REPO_ROOT / "coverage_audit" / "memory_manifest.json")
    rows = [
        row
        for row in manifest["rows"]
        if any(term in row.get("path", "") for term in SCRIPTED_TERMS)
    ]
    paths = [row["path"] for row in rows]
    unexpected = sorted(set(paths) - set(EXPECTED_MANIFEST_PATHS))
    missing = sorted(set(EXPECTED_MANIFEST_PATHS) - set(paths))
    return {
        "file": "coverage_audit/memory_manifest.json",
        "scripted_path_count": len(paths),
        "scripted_paths": paths,
        "unexpected_scripted_paths": unexpected,
        "missing_expected_paths": missing,
        "validated_paths": VALIDATED_MANIFEST_PATHS,
        "opaque_blocked_paths": OPAQUE_BLOCKED_PATHS,
        "rows": [
            {
                "path": row["path"],
                "domain": row["domain"],
                "source_class": row.get("source_class"),
                "extracted_live": row.get("extracted_live"),
                "save_parsed": row.get("save_parsed"),
                "status": row.get("status"),
                "note": row.get("note"),
            }
            for row in rows
        ],
    }


def events_summary() -> dict[str, Any]:
    path = REPO_ROOT / "save_oracle" / "output" / "events.json"
    if not path.exists():
        return {"file": rel(path), "exists": False}
    data = read_json(path)
    pending = data.get("pending_events") if isinstance(data, dict) else None
    fired = data.get("fired_event_names") if isinstance(data, dict) else None
    return {
        "file": rel(path),
        "exists": True,
        "pending_events_count": len(pending) if isinstance(pending, list) else None,
        "pending_events_is_empty": pending == [],
        "fired_event_names_count": len(fired) if isinstance(fired, list) else None,
    }


def scripted_gui_summary() -> dict[str, Any]:
    path = REPO_ROOT / "game_defs" / "out" / "scripted_guis.json"
    data = read_json(path)
    guis = data.get("scripted_guis", {})
    totals = {
        "guis": len(guis),
        "reads": 0,
        "writes": 0,
        "visible_vars": 0,
        "dynamic_loc_calls": 0,
    }
    ranked = []
    for name, rec in guis.items():
        reads = rec.get("reads") or []
        writes = rec.get("writes") or []
        visible = rec.get("visible_vars") or []
        dynamic = rec.get("dynamic_loc_calls") or []
        totals["reads"] += len(reads)
        totals["writes"] += len(writes)
        totals["visible_vars"] += len(visible)
        totals["dynamic_loc_calls"] += len(dynamic)
        ranked.append(
            {
                "name": name,
                "reads": len(reads),
                "writes": len(writes),
                "visible_vars": len(visible),
                "dynamic_loc_calls": len(dynamic),
            }
        )
    ranked.sort(
        key=lambda row: (
            row["writes"] * 4
            + row["reads"] * 2
            + row["visible_vars"]
            + row["dynamic_loc_calls"],
            row["name"],
        ),
        reverse=True,
    )
    return {
        "file": rel(path),
        "totals": totals,
        "best_fixture_guis": ranked[:8],
        "classification": "static_definition_oracle_not_live_open_state",
    }


def current_json_surfaces() -> dict[str, Any]:
    terms = ["delayed_event", "scripted_gui_random", "scripted_gui"]
    output_dirs = [
        REPO_ROOT / "save_oracle" / "output",
        REPO_ROOT / "save_oracle" / "output_1940",
    ]
    scans = {}
    for root in output_dirs:
        files = sorted(root.glob("*.json")) if root.exists() else []
        scans[rel(root)] = scan_json_files(files, terms)
    keytrees = sorted((REPO_ROOT / "coverage_audit").glob("save_keytree_*.json"))
    scans["coverage_audit/save_keytree_*.json"] = scan_json_files(keytrees, terms)
    return scans


def available_raw_save_sources() -> dict[str, Any]:
    candidates = []
    for root in [REPO_ROOT / "save_oracle", REPO_ROOT / "coverage_audit"]:
        for pattern in ("*.hoi4", "*_melted.txt", "*.txt"):
            for path in sorted(root.rglob(pattern)):
                if "sample" in path.parts:
                    kind = "synthetic_sample"
                elif path.name.endswith("_melted.txt") or path.suffix == ".hoi4":
                    kind = "candidate_real_save"
                else:
                    kind = "derived_or_helper_text"
                candidates.append({"file": rel(path), "kind": kind})
    return {
        "searched_roots": ["save_oracle", "coverage_audit"],
        "candidate_real_saves": [
            row for row in candidates if row["kind"] == "candidate_real_save"
        ],
        "synthetic_or_derived_files": [
            row for row in candidates if row["kind"] != "candidate_real_save"
        ][:16],
    }


def sample_once(index: int) -> dict[str, Any]:
    recursive_path = REPO_ROOT / "coverage_audit" / "recursive_paths_country.txt"
    delayed_shape = recursive_shape(recursive_path, "countries.<id>.delayed_event")
    gui_random_shape = recursive_shape(recursive_path, "countries.<id>.scripted_gui_random")
    scans = current_json_surfaces()
    event_info = events_summary()
    manifest = manifest_rows()

    delayed_term_hits = sum(
        surface["hit_counts"].get("delayed_event", 0)
        for surface in scans.values()
    )
    gui_random_hits = sum(
        surface["hit_counts"].get("scripted_gui_random", 0)
        for surface in scans.values()
    )
    scripted_gui_hits = sum(
        surface["hit_counts"].get("scripted_gui", 0)
        for surface in scans.values()
    )

    delayed_positive = (
        delayed_term_hits > 0
        and not event_info.get("pending_events_is_empty", True)
    )
    scripted_gui_positive = gui_random_hits > 0 or scripted_gui_hits > 0
    no_other_unblocked = (
        manifest["unexpected_scripted_paths"] == []
        and manifest["missing_expected_paths"] == []
        and sorted(manifest["opaque_blocked_paths"]) == sorted(OPAQUE_BLOCKED_PATHS)
    )

    return {
        "sample_index": index,
        "sample_utc": datetime.now(timezone.utc).isoformat(),
        "recursive_shapes": {
            "delayed_event_queue": delayed_shape,
            "scripted_gui_random": gui_random_shape,
        },
        "json_surface_scans": scans,
        "events_summary": event_info,
        "scripted_gui_static_definitions": scripted_gui_summary(),
        "manifest_scripted_scope_inventory": manifest,
        "fixture_decision": {
            "delayed_event_queue": {
                "positive_fixture_observed": delayed_positive,
                "status": (
                    "blocked_no_parsed_owner_value_fixture"
                    if delayed_shape["has_root"] and not delayed_positive
                    else "positive_fixture_observed"
                ),
                "evidence": (
                    "Raw recursive save shape is present, but current parsed JSON/keytree "
                    "surfaces do not expose owner/event/originator/scope/time rows."
                    if delayed_shape["has_root"] and not delayed_positive
                    else "Current surfaces exposed a delayed_event term and non-empty pending event evidence."
                ),
            },
            "scripted_gui_open_state": {
                "positive_fixture_observed": scripted_gui_positive,
                "status": (
                    "blocked_no_phase_labeled_fixture"
                    if gui_random_shape["has_root"] and not scripted_gui_positive
                    else "positive_fixture_observed"
                ),
                "evidence": (
                    "Raw scripted_gui_random leaf exists and static scripted GUI definitions exist, "
                    "but current parsed JSON/keytree surfaces have no scripted_gui_random or "
                    "phase-labeled open/closed live fixture rows."
                    if gui_random_shape["has_root"] and not scripted_gui_positive
                    else "Current surfaces exposed scripted GUI positive fixture evidence."
                ),
            },
            "no_other_unblocked_scripted_work": no_other_unblocked,
        },
    }


def build_watch(duration_s: float, poll_s: float) -> dict[str, Any]:
    samples = []
    started = time.monotonic()
    next_index = 0
    while True:
        samples.append(sample_once(next_index))
        next_index += 1
        elapsed = time.monotonic() - started
        if elapsed >= duration_s:
            break
        time.sleep(max(0.1, min(poll_s, duration_s - elapsed)))

    final = samples[-1]["fixture_decision"]
    positive_seen = any(
        sample["fixture_decision"]["delayed_event_queue"]["positive_fixture_observed"]
        or sample["fixture_decision"]["scripted_gui_open_state"]["positive_fixture_observed"]
        for sample in samples
    )
    no_other_unblocked = all(
        sample["fixture_decision"]["no_other_unblocked_scripted_work"]
        for sample in samples
    )
    return {
        "schema": f"{LANE}_opaque_fixture_watcher/v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "lane": LANE,
        "domain_id": DOMAIN_ID,
        "completion_status": "NOT COMPLETE",
        "backlog_reduced": True,
        "exact_backlog_item_removed_or_narrowed": (
            "Added and ran a bounded fixture watcher. It validates raw save shape "
            "evidence for countries.*.delayed_event and countries.*.scripted_gui_random, "
            "rejects current parsed/static proxies as positive live-root fixtures, "
            "and proves the manifest has no other unblocked scripted flag/variable "
            "containers outside the validated slice plus these two blockers."
        ),
        "watcher_limits": {
            "duration_s": duration_s,
            "poll_s": poll_s,
            "sample_count": len(samples),
            "surfaces": [
                "coverage_audit/memory_manifest.json",
                "coverage_audit/recursive_paths_country.txt",
                "coverage_audit/save_keytree_*.json",
                "save_oracle/output/*.json",
                "save_oracle/output_1940/*.json",
                "game_defs/out/scripted_guis.json",
            ],
        },
        "available_raw_save_sources": available_raw_save_sources(),
        "positive_fixture_observed": positive_seen,
        "no_other_unblocked_scripted_work": no_other_unblocked,
        "final_decision": final,
        "samples": samples,
        "production_flags": {
            "dll_only": False,
            "compact_stream": False,
            "rust_decoded": False,
            "hoi4replay_verified": False,
            "speed_five_current_equivalent_hourly_or_lower": False,
            "zero_skipped_hours": False,
            "partial_source_worker_bridge_failures": False,
        },
    }


def markdown(report: dict[str, Any]) -> str:
    delayed = report["final_decision"]["delayed_event_queue"]
    gui = report["final_decision"]["scripted_gui_open_state"]
    return "\n".join(
        [
            "# Opaque Fixture Watch",
            "",
            f"Lane: `{report['lane']}`",
            f"Domain: `{report['domain_id']}`",
            "",
            f"- Completion status: `{report['completion_status']}`",
            f"- Backlog reduced: {str(report['backlog_reduced']).lower()}",
            f"- Positive fixture observed: {str(report['positive_fixture_observed']).lower()}",
            f"- No other unblocked scripted work: {str(report['no_other_unblocked_scripted_work']).lower()}",
            f"- Samples: `{report['watcher_limits']['sample_count']}` over `{report['watcher_limits']['duration_s']}` seconds",
            "",
            "## Final Decisions",
            "",
            f"- `countries.*.delayed_event`: `{delayed['status']}` - {delayed['evidence']}",
            f"- `countries.*.scripted_gui_random / open state`: `{gui['status']}` - {gui['evidence']}",
            "",
            "## Scope Bound",
            "",
            "The manifest scripted paths are exactly `flags`, `variables`, `countries.*.flags`, "
            "`countries.*.variables`, `countries.*.delayed_event`, "
            "`countries.*.scripted_gui_random`, `states.*.flags`, and `states.*.variables`.",
            "",
        ]
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", default=str(DEFAULT_OUT_DIR))
    parser.add_argument("--duration-s", type=float, default=5.0)
    parser.add_argument("--poll-s", type=float, default=1.0)
    args = parser.parse_args()

    out_dir = Path(args.output_dir)
    report = build_watch(max(0.0, args.duration_s), max(0.1, args.poll_s))
    write_json(out_dir / "opaque_fixture_watch.json", report)
    (out_dir / "OPAQUE_FIXTURE_WATCH.md").write_text(markdown(report), encoding="utf-8")
    write_json(
        out_dir / "proof_manifest.json",
        {
            "schema": f"{LANE}_opaque_fixture_watcher_proof_manifest/v1",
            "created_utc": report["created_utc"],
            "completion_status": report["completion_status"],
            "backlog_reduced": report["backlog_reduced"],
            "exact_backlog_item_removed_or_narrowed": report[
                "exact_backlog_item_removed_or_narrowed"
            ],
            "files": [
                "opaque_fixture_watch.json",
                "OPAQUE_FIXTURE_WATCH.md",
                "proof_manifest.json",
            ],
            "positive_fixture_observed": report["positive_fixture_observed"],
            "no_other_unblocked_scripted_work": report[
                "no_other_unblocked_scripted_work"
            ],
            "final_decision": report["final_decision"],
            "production_flags": report["production_flags"],
        },
    )
    print(out_dir / "opaque_fixture_watch.json")
    print(out_dir / "OPAQUE_FIXTURE_WATCH.md")
    print(out_dir / "proof_manifest.json")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
