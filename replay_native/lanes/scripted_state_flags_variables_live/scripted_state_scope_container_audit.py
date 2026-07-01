#!/usr/bin/env python3
"""Audit scripted-state scope containers for domain 51."""

from __future__ import annotations

import json
from datetime import datetime, timezone
from pathlib import Path


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

LANE_DIR = Path(__file__).resolve().parent
REPO_ROOT = LANE_DIR.parents[2]
DEFAULT_OUT_DIR = (
    REPO_ROOT
    / "replay_native"
    / "runs"
    / "scripted_state_flags_variables_live_20260701_opaque_roots_full_final"
    / "scope_container_audit"
)


def read_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def write_json(path: Path, payload) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=False) + "\n", encoding="utf-8")


def scripted_manifest_rows():
    manifest = read_json(REPO_ROOT / "coverage_audit" / "memory_manifest.json")
    rows = manifest["rows"]
    hits = [
        row
        for row in rows
        if any(term in row.get("path", "") for term in SCRIPTED_TERMS)
    ]
    return hits


def recursive_country_summary():
    path = REPO_ROOT / "coverage_audit" / "recursive_paths_country.txt"
    roots = {
        "countries.<id>.delayed_event": {"root_line": None, "descendant_count": 0},
        "countries.<id>.flags": {"root_line": None, "descendant_count": 0},
        "countries.<id>.scripted_gui_random": {"root_line": None, "descendant_count": 0},
        "countries.<id>.variables": {"root_line": None, "descendant_count": 0},
    }
    for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        entry = line.split("\t", 1)[0]
        for root in roots:
            if entry == root:
                roots[root]["root_line"] = line_no
                roots[root]["root_text"] = line
            elif entry.startswith(root + "."):
                roots[root]["descendant_count"] += 1
    return {
        "file": "coverage_audit/recursive_paths_country.txt",
        "roots": roots,
    }


def build_audit():
    rows = scripted_manifest_rows()
    paths = [row["path"] for row in rows]
    unexpected = sorted(set(paths) - set(EXPECTED_MANIFEST_PATHS))
    missing = sorted(set(EXPECTED_MANIFEST_PATHS) - set(paths))
    domain_groups: dict[str, list[str]] = {}
    for row in rows:
        domain_groups.setdefault(row["domain"], []).append(row["path"])
    for values in domain_groups.values():
        values.sort()

    return {
        "schema": f"{LANE}_scope_container_audit/v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "lane": LANE,
        "domain_id": DOMAIN_ID,
        "completion_status": "NOT COMPLETE",
        "backlog_reduced": True,
        "exact_backlog_item_removed_or_narrowed": (
            "Missing scripted-state scope containers are bounded to the manifest's "
            "world/country/state flag and variable bags plus the two country opaque "
            "roots. No province, unit, character, combat, or other scripted flag/"
            "variable containers are listed in the current manifest."
        ),
        "manifest_file": "coverage_audit/memory_manifest.json",
        "manifest_scripted_path_count": len(paths),
        "manifest_scripted_paths": paths,
        "expected_manifest_paths": EXPECTED_MANIFEST_PATHS,
        "unexpected_manifest_scripted_paths": unexpected,
        "missing_expected_manifest_paths": missing,
        "scope_groups": domain_groups,
        "manifest_rows": [
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
        "recursive_country_inventory": recursive_country_summary(),
        "classification": {
            "closed_by_domain51_or_existing_world_parser": [
                "flags",
                "variables",
                "countries.*.flags",
                "countries.*.variables",
                "states.*.flags",
                "states.*.variables",
            ],
            "blocked_positive_fixture_required": [
                "countries.*.delayed_event",
                "countries.*.scripted_gui_random",
            ],
            "conclusively_not_in_current_manifest": [
                "provinces.*.flags",
                "provinces.*.variables",
                "units.*.flags",
                "units.*.variables",
                "characters.*.flags",
                "characters.*.variables",
                "combats.*.flags",
                "combats.*.variables",
            ],
        },
    }


def markdown(audit: dict) -> str:
    lines = [
        "# Domain 51 Scope Container Audit",
        "",
        f"Lane: `{audit['lane']}`",
        f"Domain: `{audit['domain_id']}`",
        "",
        f"- Completion status: `{audit['completion_status']}`",
        f"- Backlog reduced: {str(audit['backlog_reduced']).lower()}",
        f"- Manifest scripted path count: `{audit['manifest_scripted_path_count']}`",
        f"- Unexpected scripted paths: `{len(audit['unexpected_manifest_scripted_paths'])}`",
        f"- Missing expected paths: `{len(audit['missing_expected_manifest_paths'])}`",
        "",
        "## Manifest Paths",
        "",
    ]
    lines.extend(f"- `{path}`" for path in audit["manifest_scripted_paths"])
    lines.extend(["", "## Classification", ""])
    for key, values in audit["classification"].items():
        lines.append(f"- `{key}`: {', '.join(f'`{value}`' for value in values)}")
    lines.extend(["", "## Recursive Country Roots", ""])
    for root, details in audit["recursive_country_inventory"]["roots"].items():
        lines.append(
            f"- `{root}`: root line `{details['root_line']}`, descendants `{details['descendant_count']}`"
        )
    lines.append("")
    return "\n".join(lines)


def main() -> int:
    audit = build_audit()
    out_dir = DEFAULT_OUT_DIR
    write_json(out_dir / "scope_container_audit.json", audit)
    (out_dir / "SCOPE_CONTAINER_AUDIT.md").write_text(markdown(audit), encoding="utf-8")
    print(out_dir / "scope_container_audit.json")
    print(out_dir / "SCOPE_CONTAINER_AUDIT.md")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
