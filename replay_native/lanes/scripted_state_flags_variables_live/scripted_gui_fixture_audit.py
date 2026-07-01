"""Audit scripted-GUI/open-state fixture evidence for this lane.

This proof is intentionally fixture/root gating, not a shared integration hook.
It distinguishes static scripted-GUI binding definitions from live per-country
open/opaque causal state. Static definitions plus country variables are useful
inputs, but they cannot validate a live root for `scripted_gui_random` or GUI
open state without a positive save/live fixture.
"""

from __future__ import annotations

import argparse
import json
from datetime import datetime, timezone
from pathlib import Path


LANE = "scripted_state_flags_variables_live"
DOMAIN_ID = 51
EXPECTED_SHA_870EBF = (
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
)

LANE_DIR = Path(__file__).resolve().parent
REPO_ROOT = LANE_DIR.parents[2]
DEFAULT_OUT_DIR = (
    REPO_ROOT
    / "replay_native"
    / "runs"
    / "scripted_state_flags_variables_live_870ebf_20260701_scripted_gui_blocker_v1"
)


def read_json(path: Path):
    with path.open("r", encoding="utf-8") as f:
        return json.load(f)


def write_json(path: Path, payload) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as f:
        json.dump(payload, f, indent=2, sort_keys=False)
        f.write("\n")


def find_manifest_rows(manifest, needle):
    rows = []

    def walk(value):
        if isinstance(value, dict):
            path = str(value.get("path", ""))
            note = str(value.get("note", ""))
            closed_by = str(value.get("closed_by", ""))
            if needle.lower() in path.lower() or needle.lower() in note.lower() or needle.lower() in closed_by.lower():
                rows.append(value)
            for child in value.values():
                walk(child)
        elif isinstance(value, list):
            for child in value:
                walk(child)

    walk(manifest)
    return rows


def walk_json(value, path, targets, hits):
    if isinstance(value, dict):
        for key, child in value.items():
            next_path = path + [str(key)]
            key_lower = str(key).lower()
            for target in targets:
                if target in key_lower:
                    hits[target].append(
                        {"path": ".".join(next_path), "hit_kind": "key"}
                    )
            if isinstance(child, str):
                child_lower = child.lower()
                for target in targets:
                    if target in child_lower:
                        hits[target].append(
                            {"path": ".".join(next_path), "hit_kind": "string_value"}
                        )
            walk_json(child, next_path, targets, hits)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            walk_json(child, path + [str(index)], targets, hits)


def scan_json_dirs(dirs, targets):
    out = {}
    for root in dirs:
        files = sorted(root.glob("*.json")) if root.exists() else []
        hits = {target: [] for target in targets}
        unreadable = []
        for path in files:
            try:
                data = read_json(path)
            except Exception as exc:  # pragma: no cover - proof diagnostics
                unreadable.append({"file": str(path), "error": str(exc)})
                continue
            walk_json(data, [path.name], targets, hits)
        out[str(root.relative_to(REPO_ROOT))] = {
            "files_scanned": len(files),
            "hit_counts": {target: len(values) for target, values in hits.items()},
            "sample_hits": {
                target: values[:24] for target, values in hits.items() if values
            },
            "unreadable": unreadable,
        }
    return out


def scan_json_files(files, targets):
    hits = {target: [] for target in targets}
    unreadable = []
    file_count = 0
    for path in sorted(files):
        file_count += 1
        try:
            data = read_json(path)
        except Exception as exc:  # pragma: no cover - proof diagnostics
            unreadable.append({"file": str(path), "error": str(exc)})
            continue
        walk_json(data, [path.name], targets, hits)
    return {
        "files_scanned": file_count,
        "hit_counts": {target: len(values) for target, values in hits.items()},
        "sample_hits": {target: values[:24] for target, values in hits.items() if values},
        "unreadable": unreadable,
    }


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


def summarize_scripted_guis(path: Path):
    data = read_json(path)
    guis = data.get("scripted_guis", {})
    reads = 0
    writes = 0
    visible_vars = 0
    dynamic_loc_calls = 0
    dynamic_loc_resolved = 0
    sample_guis = []
    for name, rec in sorted(guis.items()):
        reads += len(rec.get("reads") or [])
        writes += len(rec.get("writes") or [])
        visible_vars += len(rec.get("visible_vars") or [])
        dynamic_loc_calls += len(rec.get("dynamic_loc_calls") or [])
        dynamic_loc_resolved += len(rec.get("dynamic_loc_calls_resolved") or [])
        if len(sample_guis) < 8:
            sample_guis.append(
                {
                    "name": name,
                    "elements": len(rec.get("elements") or {}),
                    "reads": len(rec.get("reads") or []),
                    "writes": len(rec.get("writes") or []),
                    "visible_vars": len(rec.get("visible_vars") or []),
                    "dynamic_loc_calls": len(rec.get("dynamic_loc_calls") or []),
                }
            )
    return {
        "file": str(path.relative_to(REPO_ROOT)),
        "meta": data.get("meta", {}),
        "gui_count": len(guis),
        "read_count": reads,
        "write_count": writes,
        "visible_var_count": visible_vars,
        "dynamic_loc_call_count": dynamic_loc_calls,
        "dynamic_loc_resolved_count": dynamic_loc_resolved,
        "sample_guis": sample_guis,
    }


def validate_registry(path: Path):
    data = read_json(path)
    matches = [
        domain
        for domain in data.get("domains", [])
        if domain.get("id") == DOMAIN_ID
    ]
    return {
        "file": str(path.relative_to(REPO_ROOT)),
        "domain_id": DOMAIN_ID,
        "matches": matches,
        "valid": len(matches) == 1 and matches[0].get("name") == LANE,
    }


def build_audit():
    manifest = read_json(REPO_ROOT / "coverage_audit" / "memory_manifest.json")
    registry = validate_registry(REPO_ROOT / "replay_native" / "domain_registry.json")
    targets = ["scripted_gui_random", "scripted_gui", "dynamic_loc_calls"]
    oracle_scan = scan_json_dirs(
        [
            REPO_ROOT / "save_oracle" / "output",
            REPO_ROOT / "save_oracle" / "output_1940",
        ],
        targets,
    )
    keytree_scan = scan_json_files(
        (REPO_ROOT / "coverage_audit").glob("save_keytree_*.json"),
        targets,
    )
    scripted_gui_summary = summarize_scripted_guis(
        REPO_ROOT / "game_defs" / "out" / "scripted_guis.json"
    )
    recursive_scripted_gui_random = scan_recursive_path_shape(
        REPO_ROOT / "coverage_audit" / "recursive_paths_country.txt",
        "countries.<id>.scripted_gui_random",
    )
    scripted_gui_rows = find_manifest_rows(manifest, "scripted_gui")
    variable_rows = [
        row
        for row in find_manifest_rows(manifest, "countries.*.variables")
        if row.get("path") == "countries.*.variables"
    ]

    no_scripted_gui_random_fixture = (
        all(
            scan["hit_counts"]["scripted_gui_random"] == 0
            for scan in oracle_scan.values()
        )
        and keytree_scan["hit_counts"]["scripted_gui_random"] == 0
    )
    no_live_open_fixture = (
        all(scan["hit_counts"]["scripted_gui"] == 0 for scan in oracle_scan.values())
        and keytree_scan["hit_counts"]["scripted_gui"] == 0
    )

    return {
        "schema": f"{LANE}_scripted_gui_fixture_blocker/v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "lane": LANE,
        "target_build": {
            "sha256": EXPECTED_SHA_870EBF,
            "process": "hoi4.exe",
            "note": "Fixture/root blocker audit; no live process read is required.",
        },
        "domain_registry": registry,
        "shared_entrypoint_policy": {
            "collector_entrypoints_edited": False,
            "recorder_entrypoints_edited": False,
            "integration_slot_required": True,
        },
        "completion_status": "NOT COMPLETE",
        "lane_full_production_status": "NOT COMPLETE",
        "backlog_reduced": True,
        "exact_backlog_item_removed_or_narrowed": (
            "Scripted-GUI open/opaque root validation narrowed to a missing "
            "phase-labeled positive fixture: the raw recursive save inventory "
            "proves countries.*.scripted_gui_random exists, while static "
            "scripted_gui definitions and validated country variables are not "
            "sufficient roots for scripted_gui_random or open GUI causal state."
        ),
        "manifest_scripted_gui_rows": scripted_gui_rows,
        "manifest_country_variable_rows": variable_rows,
        "recursive_scripted_gui_random_shape": recursive_scripted_gui_random,
        "scripted_gui_static_definition_summary": scripted_gui_summary,
        "save_oracle_output_scan": oracle_scan,
        "save_keytree_scan": keytree_scan,
        "rejected_candidates": [
            {
                "candidate": "game_defs/out/scripted_guis.json as live scripted-GUI open-state root",
                "status": "rejected",
                "evidence": (
                    "It is a static structural definition map: 40 GUI definitions, "
                    "47 reads, 54 writes, 12 visible vars, and 31 dynamic_loc call "
                    "names. It contains no per-country open-state or RNG payload."
                ),
            },
            {
                "candidate": "countries.*.variables as complete scripted-GUI causal/open state",
                "status": "rejected_as_complete_root",
                "evidence": (
                    "Manifest credits countries.*.variables as closing direct "
                    "scripted-GUI variable values, but separately lists "
                    "countries.*.scripted_gui_random as engine-internal with no "
                    "save parser or live reader."
                ),
            },
            {
                "candidate": "countries.*.scripted_gui_random live root from current fixtures",
                "status": "blocked_missing_positive_fixture",
                "evidence": (
                    "save_oracle/output, save_oracle/output_1940, and all "
                    "coverage_audit/save_keytree_*.json files have zero "
                    "scripted_gui_random hits. coverage_audit/recursive_paths_country.txt "
                    "does prove a raw countries.<id>.scripted_gui_random leaf exists, "
                    "but it does not provide phase-labeled owner/value rows."
                ),
            },
        ],
        "blocker": {
            "kind": "missing_phase_labeled_scripted_gui_open_fixture",
            "is_blocking_current_live_root_acceptance": (
                registry["valid"]
                and no_scripted_gui_random_fixture
                and no_live_open_fixture
                and recursive_scripted_gui_random["has_root"]
            ),
            "blocked_work": (
                "Do not accept a scripted-GUI/open-state heap/root candidate from "
                "current 870ebf without a positive save-derived "
                "countries.*.scripted_gui_random fixture, a visible open-GUI live "
                "mutation fixture, or an equivalent engine-side invariant."
            ),
            "unblocker": (
                "Create a development fixture that mutates scripted-GUI RNG/open "
                "state under a known country, parse or otherwise observe that "
                "payload, then correlate it against validated CCountry roots and "
                "the static scripted_guis map."
            ),
        },
        "compact_packet_effect": {
            "domain_id": DOMAIN_ID,
            "status_bit": "STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED",
            "record_kind": "VALUE_SCRIPTED_GUI_OPAQUE",
            "required_behavior": (
                "Keep scripted-GUI opaque rows fail-closed; emit no guessed "
                "open-state payload and only an unresolved badge/status bit until "
                "a root is validated."
            ),
        },
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", default=str(DEFAULT_OUT_DIR))
    args = parser.parse_args()

    out_dir = Path(args.output_dir)
    audit = build_audit()
    write_json(out_dir / "scripted_gui_fixture_audit.json", audit)
    write_json(
        out_dir / "proof_manifest.json",
        {
            "schema": f"{LANE}_scripted_gui_fixture_blocker_proof_manifest/v1",
            "created_utc": audit["created_utc"],
            "completion_status": audit["completion_status"],
            "lane_full_production_status": audit["lane_full_production_status"],
            "backlog_reduced": audit["backlog_reduced"],
            "exact_backlog_item_removed_or_narrowed": audit[
                "exact_backlog_item_removed_or_narrowed"
            ],
            "files": [
                "scripted_gui_fixture_audit.json",
                "proof_manifest.json",
            ],
            "domain_registry": audit["domain_registry"],
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
    print(f"wrote {out_dir / 'scripted_gui_fixture_audit.json'}")
    print(f"wrote {out_dir / 'proof_manifest.json'}")


if __name__ == "__main__":
    main()
