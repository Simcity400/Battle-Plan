"""Audit the delayed-event fixture surface for the scripted-state lane.

This lane-local proof deliberately does not claim a live root. It checks the
current save-oracle/keytree/parser evidence before any live root can be accepted
for queued delayed events. The recursive save inventory proves the raw
`countries.*.delayed_event` shape exists, but without parsed owner/value rows a
live heap candidate cannot be validated against an oracle and must stay
fail-closed.
"""

from __future__ import annotations

import argparse
import json
from datetime import datetime, timezone
from pathlib import Path


LANE = "scripted_state_flags_variables_live"
EXPECTED_SHA_870EBF = (
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
)

LANE_DIR = Path(__file__).resolve().parent
REPO_ROOT = LANE_DIR.parents[3]
DEFAULT_OUT_DIR = (
    REPO_ROOT
    / "extractor/native"
    / "runs"
    / "scripted_state_flags_variables_live_870ebf_20260701_delayed_event_blocker_v1"
)


def read_json(path: Path):
    with path.open("r", encoding="utf-8") as f:
        return json.load(f)


def write_json(path: Path, payload) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as f:
        json.dump(payload, f, indent=2, sort_keys=False)
        f.write("\n")


def walk_json(value, path, key_hits, scalar_hits, target):
    if isinstance(value, dict):
        for key, child in value.items():
            next_path = path + [str(key)]
            if key == target:
                key_hits.append(".".join(next_path))
            if child == target:
                scalar_hits.append(".".join(next_path))
            walk_json(child, next_path, key_hits, scalar_hits, target)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            walk_json(child, path + [str(index)], key_hits, scalar_hits, target)


def scan_json_files(paths, target):
    files = []
    total_key_hits = []
    total_scalar_hits = []
    unreadable = []
    for path in sorted(paths):
        try:
            data = read_json(path)
        except Exception as exc:  # pragma: no cover - proof diagnostics
            unreadable.append({"file": str(path), "error": str(exc)})
            continue
        key_hits = []
        scalar_hits = []
        walk_json(data, [path.name], key_hits, scalar_hits, target)
        if key_hits or scalar_hits:
            files.append(
                {
                    "file": str(path.relative_to(REPO_ROOT)),
                    "key_hits": key_hits[:32],
                    "scalar_hits": scalar_hits[:32],
                    "key_hit_count": len(key_hits),
                    "scalar_hit_count": len(scalar_hits),
                }
            )
        total_key_hits.extend(f"{path.name}:{hit}" for hit in key_hits)
        total_scalar_hits.extend(f"{path.name}:{hit}" for hit in scalar_hits)
    return {
        "files_scanned": len(list(paths)),
        "files_with_hits": files,
        "key_hit_count": len(total_key_hits),
        "scalar_hit_count": len(total_scalar_hits),
        "unreadable": unreadable,
    }


def find_manifest_row(manifest, wanted_path):
    rows = []

    def walk(value):
        if isinstance(value, dict):
            if value.get("path") == wanted_path:
                rows.append(value)
            for child in value.values():
                walk(child)
        elif isinstance(value, list):
            for child in value:
                walk(child)

    walk(manifest)
    return rows


def scan_parser_lines(files, target):
    hits = []
    for path in sorted(files):
        text = path.read_text(encoding="utf-8", errors="replace")
        for line_no, line in enumerate(text.splitlines(), 1):
            if target in line:
                hits.append(
                    {
                        "file": str(path.relative_to(REPO_ROOT)),
                        "line": line_no,
                        "text": line.strip(),
                    }
                )
    return hits


def scan_recursive_path_shape(path: Path, root: str):
    hits = []
    for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip() or "\t" not in line:
            continue
        row_path = line.split("\t", 1)[0]
        if row_path == root or row_path.startswith(root + "."):
            fields = line.split("\t")
            hits.append(
                {
                    "line": line_no,
                    "path": row_path,
                    "kind": fields[1] if len(fields) > 1 else "",
                    "count": fields[2] if len(fields) > 2 else "",
                }
            )
    return {
        "file": str(path.relative_to(REPO_ROOT)),
        "root": root,
        "rows": hits,
        "row_count": len(hits),
        "has_root": any(row["path"] == root for row in hits),
        "descendant_count": sum(1 for row in hits if row["path"].startswith(root + ".")),
    }


def summarize_events_json(path: Path):
    data = read_json(path)
    pending = data.get("pending_events") if isinstance(data, dict) else None
    fired = data.get("fired_event_names") if isinstance(data, dict) else None
    return {
        "file": str(path.relative_to(REPO_ROOT)),
        "top_level_keys": list(data.keys()) if isinstance(data, dict) else [],
        "pending_events_count": len(pending) if isinstance(pending, list) else None,
        "fired_event_names_count": len(fired) if isinstance(fired, list) else None,
        "pending_events_is_empty": pending == [],
    }


def build_audit():
    output_jsons = sorted((REPO_ROOT / "extractor" / "save_oracle" / "output").glob("*.json"))
    keytrees = sorted((REPO_ROOT / "extractor" / "coverage_audit").glob("save_keytree_*.json"))
    parser_files = sorted((REPO_ROOT / "extractor" / "save_oracle").glob("*.py"))
    manifest = read_json(REPO_ROOT / "extractor" / "coverage_audit" / "memory_manifest.json")

    delayed_manifest = find_manifest_row(manifest, "countries.*.delayed_event")
    saved_target_manifest = find_manifest_row(manifest, "saved_event_target")
    delayed_output_scan = scan_json_files(output_jsons, "delayed_event")
    delayed_keytree_scan = scan_json_files(keytrees, "delayed_event")
    saved_target_keytree_scan = scan_json_files(keytrees, "saved_event_target")
    delayed_parser_hits = scan_parser_lines(parser_files, "delayed_event")
    saved_target_parser_hits = scan_parser_lines(parser_files, "saved_event_target")
    events_summary = summarize_events_json(REPO_ROOT / "extractor" / "save_oracle" / "output" / "events.json")
    recursive_shape = scan_recursive_path_shape(
        REPO_ROOT / "extractor" / "coverage_audit" / "recursive_paths_country.txt",
        "countries.<id>.delayed_event",
    )

    no_parsed_owner_value_fixture = (
        delayed_output_scan["key_hit_count"] == 0
        and delayed_output_scan["scalar_hit_count"] == 0
        and delayed_keytree_scan["key_hit_count"] == 0
        and delayed_keytree_scan["scalar_hit_count"] == 0
        and bool(delayed_manifest)
        and delayed_manifest[0].get("save_parsed") == "no"
        and recursive_shape["has_root"]
    )

    return {
        "schema": f"{LANE}_delayed_event_fixture_blocker/v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "lane": LANE,
        "target_build": {
            "sha256": EXPECTED_SHA_870EBF,
            "process": "hoi4.exe",
            "note": "Fixture-side blocker audit; no live process read is required.",
        },
        "shared_entrypoint_policy": {
            "collector_entrypoints_edited": False,
            "recorder_entrypoints_edited": False,
            "integration_slot_required": True,
        },
        "completion_status": "NOT COMPLETE",
        "lane_full_production_status": "NOT COMPLETE",
        "backlog_reduced": True,
        "exact_backlog_item_removed_or_narrowed": (
            "Delayed-event live root validation narrowed to a missing parsed "
            "owner/value fixture blocker. The raw recursive save inventory proves "
            "countries.*.delayed_event shape exists, while saved_event_target and "
            "pending_events are rejected as validation proxies."
        ),
        "delayed_event_manifest_rows": delayed_manifest,
        "saved_event_target_manifest_rows": saved_target_manifest,
        "recursive_delayed_event_shape": recursive_shape,
        "save_oracle_output_scan": delayed_output_scan,
        "save_keytree_delayed_event_scan": delayed_keytree_scan,
        "save_keytree_saved_event_target_scan": saved_target_keytree_scan,
        "parser_delayed_event_hits": delayed_parser_hits,
        "parser_saved_event_target_hits": saved_target_parser_hits,
        "events_json_summary": events_summary,
        "rejected_candidates": [
            {
                "candidate": "saved_event_target as delayed_event validation proxy",
                "status": "rejected",
                "evidence": (
                    "coverage_audit/memory_manifest.json classifies saved_event_target "
                    "as top_level:world, save_parsed=yes, closed-by-reader, while "
                    "countries.*.delayed_event is domain=country, save_parsed=no, "
                    "status=excluded."
                ),
            },
            {
                "candidate": "save_oracle/output/events.json pending_events as queued country delayed_event fixture",
                "status": "rejected_current_fixture",
                "evidence": "pending_events is present but empty in the current oracle output.",
            },
        ],
        "blocker": {
            "kind": "missing_parsed_owner_value_fixture",
            "is_blocking_current_live_root_acceptance": no_parsed_owner_value_fixture,
            "blocked_work": (
                "Do not accept a delayed-event heap/root candidate from the current "
                "870ebf process without parsed owner/value rows for a non-empty "
                "countries.*.delayed_event fixture or an equivalent engine-side "
                "invariant."
            ),
            "unblocker": (
                "Use the raw recursive save shape already observed here to extend a "
                "fixture parser, or capture/synthesize a development save with at "
                "least one non-empty countries.*.delayed_event row, then correlate "
                "owner, event, originator, scope, and time fields against validated "
                "CCountry rows."
            ),
        },
        "compact_packet_effect": {
            "domain_id": 51,
            "status_bit": "STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED",
            "record_kind": "VALUE_DELAYED_EVENT_OPAQUE",
            "required_behavior": (
                "Keep delayed-event rows fail-closed; emit no guessed event payload and "
                "only an unresolved badge/status bit until a root is validated."
            ),
        },
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", default=str(DEFAULT_OUT_DIR))
    args = parser.parse_args()

    out_dir = Path(args.output_dir)
    audit = build_audit()
    write_json(out_dir / "delayed_event_fixture_audit.json", audit)
    write_json(
        out_dir / "proof_manifest.json",
        {
            "schema": f"{LANE}_delayed_event_fixture_blocker_proof_manifest/v1",
            "created_utc": audit["created_utc"],
            "completion_status": audit["completion_status"],
            "lane_full_production_status": audit["lane_full_production_status"],
            "backlog_reduced": audit["backlog_reduced"],
            "exact_backlog_item_removed_or_narrowed": audit[
                "exact_backlog_item_removed_or_narrowed"
            ],
            "files": [
                "delayed_event_fixture_audit.json",
                "proof_manifest.json",
            ],
            "validated_roots": [],
            "rejected_candidates": audit["rejected_candidates"],
            "blocker": audit["blocker"],
            "compact_packet_effect": audit["compact_packet_effect"],
            "production_flags": {
                "dll_only": False,
                "compact_stream": False,
                "rust_decoded": False,
                "hoi4replay_verified": False,
                "speed_five_current_equivalent_hourly_or_lower": False,
                "zero_skipped_hours": False,
                "partial_source_worker_bridge_failures": True,
            },
        },
    )
    print(f"wrote {out_dir / 'delayed_event_fixture_audit.json'}")
    print(f"wrote {out_dir / 'proof_manifest.json'}")


if __name__ == "__main__":
    main()
