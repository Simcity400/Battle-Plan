#!/usr/bin/env python3
"""Emit exact positive-fixture contracts for unresolved opaque roots.

The delayed-event and scripted-GUI/open-state roots cannot be accepted from a
zero-data heap scan. This proof pins what a future fixture must contain before a
candidate live root can be validated or rejected.
"""

from __future__ import annotations

import argparse
import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


LANE = "scripted_state_flags_variables_live"
DOMAIN_ID = 51
EXPECTED_SHA_870EBF = (
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
)

LANE_DIR = Path(__file__).resolve().parent
REPO_ROOT = LANE_DIR.parents[3]
DEFAULT_OUT_DIR = (
    REPO_ROOT
    / "extractor/native"
    / "runs"
    / "scripted_state_flags_variables_live_870ebf_20260701_opaque_fixture_contracts_v1"
)


def read_json(path: Path) -> Any:
    with path.open("r", encoding="utf-8-sig") as handle:
        return json.load(handle)


def write_json(path: Path, payload: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        json.dump(payload, handle, indent=2, sort_keys=False)
        handle.write("\n")


def rel(path: Path) -> str:
    return str(path.relative_to(REPO_ROOT)).replace("\\", "/")


def find_manifest_rows(manifest: Any, wanted_path: str) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []

    def walk(value: Any) -> None:
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


def parse_recursive_paths(path: Path, needles: list[str]) -> dict[str, Any]:
    hits = {needle: [] for needle in needles}
    for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip() or "\t" not in line:
            continue
        fields = line.split("\t")
        row_path = fields[0]
        for needle in needles:
            if needle in row_path:
                hits[needle].append(
                    {
                        "line": line_no,
                        "path": row_path,
                        "kind": fields[1] if len(fields) > 1 else "",
                        "count": fields[2] if len(fields) > 2 else "",
                    }
                )
    return {
        "file": rel(path),
        "needles": hits,
    }


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


def scan_current_json_outputs() -> dict[str, Any]:
    terms = ["delayed_event", "scripted_gui_random", "scripted_gui"]
    roots = [
        REPO_ROOT / "extractor" / "save_oracle" / "output",
        REPO_ROOT / "extractor" / "save_oracle" / "output_1940",
        REPO_ROOT / "extractor" / "coverage_audit",
    ]
    out: dict[str, Any] = {}
    for root in roots:
        files = (
            sorted(root.glob("*.json"))
            if root.name != "coverage_audit"
            else sorted(root.glob("save_keytree_*.json"))
        )
        hits = {term: [] for term in terms}
        for file in files:
            try:
                data = read_json(file)
            except Exception:
                continue
            file_hits = {term: [] for term in terms}
            walk_json_for_terms(data, [file.name], terms, file_hits)
            for term, values in file_hits.items():
                hits[term].extend(f"{file.name}:{value}" for value in values)
        out[rel(root)] = {
            "files_scanned": len(files),
            "hit_counts": {term: len(values) for term, values in hits.items()},
            "sample_hits": {term: values[:16] for term, values in hits.items() if values},
        }
    return out


def scripted_gui_summary(path: Path) -> dict[str, Any]:
    data = read_json(path)
    guis = data.get("scripted_guis", {})
    ranked = []
    totals = {
        "guis": len(guis),
        "reads": 0,
        "writes": 0,
        "visible_vars": 0,
        "dynamic_loc_calls": 0,
    }
    for name, rec in guis.items():
        reads = rec.get("reads") or []
        writes = rec.get("writes") or []
        visible_vars = rec.get("visible_vars") or []
        dynamic_loc_calls = rec.get("dynamic_loc_calls") or []
        totals["reads"] += len(reads)
        totals["writes"] += len(writes)
        totals["visible_vars"] += len(visible_vars)
        totals["dynamic_loc_calls"] += len(dynamic_loc_calls)
        ranked.append(
            {
                "name": name,
                "reads": reads,
                "writes": writes,
                "visible_vars": visible_vars,
                "dynamic_loc_calls": dynamic_loc_calls,
                "score": len(writes) * 4
                + len(reads) * 2
                + len(visible_vars)
                + len(dynamic_loc_calls),
            }
        )
    ranked.sort(key=lambda item: (item["score"], item["name"]), reverse=True)
    return {
        "file": rel(path),
        "totals": totals,
        "best_fixture_guis": ranked[:8],
    }


def delayed_event_contract(recursive_paths: dict[str, Any]) -> dict[str, Any]:
    shape_rows = recursive_paths["needles"]["countries.<id>.delayed_event"]
    return {
        "contract_id": "ssfv_delayed_event_queue_positive_fixture_v1",
        "root": "delayed_event_queue",
        "current_status": "fixture_blocked",
        "raw_save_shape_evidence": shape_rows,
        "required_player_game_action": [
            "Enable a dev-only fixture event namespace `ssfv_delayed_probe` in a disposable local mod or branch.",
            "Start a single-player current-build session as a stable country such as GER, pause on 1936-01-01 01:00, and record the country table tag-db id for that country from the accepted CCountry path.",
            "Fire `event ssfv_delayed_probe.1 GER`; that event must schedule exactly one country-scoped delayed event, `ssfv_delayed_probe.2`, at least 72 hours in the future using the engine's normal delayed country_event effect.",
            "Save immediately before unpausing, then optionally advance until the delayed event fires and save/log again.",
        ],
        "required_save_evidence": {
            "source": "raw save walker and save oracle fixture parser",
            "required_rows": [
                "countries.<target_id>.delayed_event",
                "countries.<target_id>.delayed_event.event",
                "countries.<target_id>.delayed_event.hours or .days or .months",
                "countries.<target_id>.delayed_event.originator",
                "countries.<target_id>.delayed_event.scope",
            ],
            "required_values": {
                "event": "must identify `ssfv_delayed_probe.2` as name/token/id",
                "countdown": "must be positive in hours/days/months in the immediate save",
                "originator": "must equal the target or explicitly recorded origin country id",
                "scope": "must contain a country scope matching the target country",
            },
            "current_repo_gap": "current save_oracle outputs/keytrees do not expose positive values for this row; only recursive path inventory preserves the raw shape.",
        },
        "required_live_evidence": {
            "accepted_roots_to_reuse": [
                "CGameState = qword(module_base + 0x3316E90)",
                "country table CGameState + 0x310, count +0x31C",
                "target CCountry row validated by vtable module+0x27D0E90 and CCountry+0x08 tag-db id",
            ],
            "bounded_candidate_search": [
                "Prefer qword-aligned CCountry-owned candidates under target CCountry[0x0..0x4000].",
                "A CGameState-owned queue candidate is allowed only if each row owner/scope joins back to the validated target CCountry id.",
                "Do not accept zero-row, all-empty, or ownerless candidates.",
            ],
            "fields_to_decode": [
                "owner country tag-db id",
                "event name/token/id",
                "remaining hours/days/months or raw absolute due date",
                "originator country id",
                "scope kind and id",
                "row count",
            ],
        },
        "required_ui_or_runtime_evidence": [
            "After advancing to the due time, `ssfv_delayed_probe.2` must fire for the target country, visible as an event popup, log line, or `fired_event_names`/equivalent save evidence.",
            "The event firing evidence is a runtime sanity check; it does not replace the immediate-save and live-heap row match.",
        ],
        "pass_rule": [
            "Exactly one positive save row for `ssfv_delayed_probe.2` exists under the target country immediately after scheduling.",
            "The live candidate has the same owner, event id/name/token, originator, scope, and countdown/due-time within one game hour of the save evidence.",
            "The same candidate becomes empty or the probe row disappears after the delayed event fires.",
        ],
        "fail_rule": [
            "No positive save row is present.",
            "The live candidate cannot identify event, owner, originator, scope, and time fields.",
            "The candidate matches `saved_event_target` or `events.pending_events` only; those are rejected proxies.",
            "The candidate requires setting `STATUS_PARTIAL_DATA` or `STATUS_SOURCE_UNAVAILABLE` for the already validated country/global/state variable/flag subset.",
        ],
    }


def scripted_gui_contract(recursive_paths: dict[str, Any], gui_summary: dict[str, Any]) -> dict[str, Any]:
    random_rows = recursive_paths["needles"]["countries.<id>.scripted_gui_random"]
    best = gui_summary["best_fixture_guis"][0]
    return {
        "contract_id": "ssfv_scripted_gui_open_state_positive_fixture_v1",
        "root": "scripted_gui_open_state_and_scripted_gui_random",
        "current_status": "fixture_blocked",
        "raw_save_shape_evidence": random_rows,
        "static_definition_evidence": {
            "scripted_guis_file": gui_summary["file"],
            "totals": gui_summary["totals"],
            "recommended_existing_fixture_gui": {
                "name": best["name"],
                "reads": best["reads"][:12],
                "writes": best["writes"][:12],
                "visible_vars": best["visible_vars"][:12],
            },
        },
        "required_player_game_action": [
            "Use a disposable current-build session with a country that can open a parsed scripted GUI. Prefer an existing high-signal GUI from `game_defs/out/scripted_guis.json`, such as `st_country_powers`, because it has many parsed writes and visible guards.",
            "Record a closed-baseline sample for the target country, including validated country variables/flags and the raw `countries.<id>.scripted_gui_random` leaf if the save walker exposes it.",
            "Open the chosen scripted GUI, perform one deterministic UI click/effect that writes a known parsed variable/flag, and keep the GUI open while taking a live sample and a save or UI screenshot.",
            "Close the GUI and take a second live sample at the same country as a negative/control phase.",
        ],
        "required_save_evidence": {
            "source": "raw save walker plus parsed country variable/flag outputs",
            "required_rows": [
                "countries.<target_id>.scripted_gui_random",
                "countries.<target_id>.variables or flags changed by the selected scripted GUI action",
            ],
            "required_values": {
                "scripted_gui_random": "raw leaf value for the target country; record before/open/after phases if it changes",
                "gui_written_variable_or_flag": "must match one parsed write from the selected GUI definition",
            },
            "current_repo_gap": "current save_oracle outputs/keytrees have zero scripted_gui_random hits; recursive path inventory only proves the leaf path exists in raw country saves.",
        },
        "required_live_evidence": {
            "accepted_roots_to_reuse": [
                "validated CCountry identity path and country variables/flags roots",
                "static `game_defs/out/scripted_guis.json` binding map only as a definition oracle, not as live state",
            ],
            "bounded_candidate_search": [
                "For scripted_gui_random, scan qword-aligned candidates under CCountry[0x0..0x4000] and CGameState[0x0..0x2000] only if values can be joined per country.",
                "For open-state, require a live candidate that changes between closed/open/closed phases in lockstep with the visible GUI state.",
                "Reject candidates that only mirror ordinary country variables/flags already covered by validated roots.",
            ],
            "fields_to_decode": [
                "owner country tag-db id",
                "scripted GUI name or stable token when available",
                "open/closed state or selected element/state token if present",
                "scripted_gui_random raw value",
                "linked variable/flag write evidence from the selected GUI action",
            ],
        },
        "required_ui_or_runtime_evidence": [
            "Screenshot or UI log with the selected scripted GUI visibly open for the open phase.",
            "Definition evidence that the clicked element belongs to the selected scripted GUI and writes/reads the recorded variable/flag.",
            "Closed/open/closed timing labels so live samples can be compared against the UI state transition.",
        ],
        "pass_rule": [
            "The candidate value set joins to the target country and changes or persists exactly as the raw `scripted_gui_random`/open-state fixture predicts.",
            "The candidate distinguishes GUI open state from ordinary scripted variable/flag values already emitted by domain 51.",
            "The same candidate returns to the closed/control value after the GUI closes, or is proven persistent RNG state by matching raw save leaf values across countries.",
        ],
        "fail_rule": [
            "Only static `scripted_guis.json` definition data is available.",
            "Only country variable/flag rows change; those are already covered by the validated subset and are not a separate open-state root.",
            "The candidate has no country join or does not toggle/match the UI phase labels.",
            "The candidate requires setting `STATUS_PARTIAL_DATA` or `STATUS_SOURCE_UNAVAILABLE` for the validated country/global/state variable/flag subset.",
        ],
    }


def build_contracts() -> dict[str, Any]:
    manifest = read_json(REPO_ROOT / "extractor" / "coverage_audit" / "memory_manifest.json")
    recursive = parse_recursive_paths(
        REPO_ROOT / "extractor" / "coverage_audit" / "recursive_paths_country.txt",
        ["countries.<id>.delayed_event", "countries.<id>.scripted_gui_random"],
    )
    current_scans = scan_current_json_outputs()
    gui = scripted_gui_summary(REPO_ROOT / "game_defs" / "out" / "scripted_guis.json")
    delayed_manifest = find_manifest_rows(manifest, "countries.*.delayed_event")
    scripted_random_manifest = find_manifest_rows(
        manifest, "countries.*.scripted_gui_random"
    )
    scripted_defs_manifest = find_manifest_rows(
        manifest, "def.scripted_gui_element_var_map"
    )
    variables_manifest = find_manifest_rows(manifest, "countries.*.variables")

    return {
        "schema": f"{LANE}_opaque_root_fixture_contracts/v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "lane": LANE,
        "domain_id": DOMAIN_ID,
        "target_build_sha256": EXPECTED_SHA_870EBF,
        "completion_status": "NOT COMPLETE",
        "backlog_reduced": True,
        "exact_backlog_item_removed_or_narrowed": (
            "Delayed-event and scripted-GUI/open-state root work narrowed to exact "
            "positive-fixture contracts. Current parsed fixtures remain insufficient; "
            "raw recursive path inventory supplies required save fields."
        ),
        "shared_entrypoint_policy": {
            "collector_entrypoints_edited": False,
            "recorder_entrypoints_edited": False,
        },
        "source_evidence": {
            "manifest_rows": {
                "countries.*.delayed_event": delayed_manifest,
                "countries.*.scripted_gui_random": scripted_random_manifest,
                "def.scripted_gui_element_var_map": scripted_defs_manifest,
                "countries.*.variables": variables_manifest,
            },
            "recursive_paths": recursive,
            "current_json_output_scans": current_scans,
            "scripted_gui_definitions": gui,
        },
        "rejected_candidates_kept_rejected": [
            {
                "candidate": "saved_event_target as delayed_event queue root/proxy",
                "status": "rejected",
                "reason": "top-level world saved scopes are not per-country delayed_event queue rows.",
            },
            {
                "candidate": "events.pending_events as current delayed_event fixture",
                "status": "rejected_current_fixture",
                "reason": "current save_oracle/output/events.json pending_events is empty.",
            },
            {
                "candidate": "game_defs/out/scripted_guis.json as live open-state root",
                "status": "rejected",
                "reason": "static definition oracle only; contains no live per-country open-state payload.",
            },
            {
                "candidate": "countries.*.variables as complete scripted-GUI open-state root",
                "status": "rejected_as_complete_root",
                "reason": "variables cover GUI-bound values but not scripted_gui_random or UI open/selection state.",
            },
        ],
        "fixture_contracts": {
            "delayed_event_queue": delayed_event_contract(recursive),
            "scripted_gui_open_state": scripted_gui_contract(recursive, gui),
        },
        "compact_gate_policy": {
            "delayed_event_unresolved_status": "STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED",
            "scripted_gui_unresolved_status": "STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED",
            "must_not_set": ["STATUS_PARTIAL_DATA", "STATUS_SOURCE_UNAVAILABLE"],
            "validated_subset": "country/global/state variables and flags stay valid when these opaque roots are fixture-blocked.",
        },
    }


def markdown_for_contracts(proof: dict[str, Any]) -> str:
    delayed = proof["fixture_contracts"]["delayed_event_queue"]
    gui = proof["fixture_contracts"]["scripted_gui_open_state"]
    return "\n".join(
        [
            "# Opaque Root Fixture Contracts",
            "",
            "Lane: `scripted_state_flags_variables_live`",
            "Domain: `51`",
            "",
            "These contracts are the acceptance gate for delayed-event and scripted-GUI/open-state root probes. A zero-data heap scan is not enough.",
            "",
            "## Delayed Event Queue",
            "",
            f"Contract: `{delayed['contract_id']}`",
            "",
            "Required action:",
            *[f"- {item}" for item in delayed["required_player_game_action"]],
            "",
            "Required save rows:",
            *[f"- `{item}`" for item in delayed["required_save_evidence"]["required_rows"]],
            "",
            "Live pass rule:",
            *[f"- {item}" for item in delayed["pass_rule"]],
            "",
            "Reject if:",
            *[f"- {item}" for item in delayed["fail_rule"]],
            "",
            "## Scripted GUI Open State",
            "",
            f"Contract: `{gui['contract_id']}`",
            "",
            "Required action:",
            *[f"- {item}" for item in gui["required_player_game_action"]],
            "",
            "Required save/live/UI rows:",
            *[f"- `{item}`" for item in gui["required_save_evidence"]["required_rows"]],
            *[f"- `{item}`" for item in gui["required_live_evidence"]["fields_to_decode"]],
            "",
            "Live pass rule:",
            *[f"- {item}" for item in gui["pass_rule"]],
            "",
            "Reject if:",
            *[f"- {item}" for item in gui["fail_rule"]],
            "",
            "## Compact Gate Policy",
            "",
            "- Unresolved delayed-event root emits only `STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED` plus an opaque badge row.",
            "- Unresolved scripted-GUI root emits only `STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED` plus an opaque badge row.",
            "- Neither blocker may set `STATUS_PARTIAL_DATA` or `STATUS_SOURCE_UNAVAILABLE` for the validated country/global/state variable and flag subset.",
            "",
        ]
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", default=str(DEFAULT_OUT_DIR))
    args = parser.parse_args()

    out_dir = Path(args.output_dir)
    proof = build_contracts()
    write_json(out_dir / "opaque_root_fixture_contracts.json", proof)
    (out_dir / "OPAQUE_ROOT_FIXTURE_CONTRACTS.md").write_text(
        markdown_for_contracts(proof), encoding="utf-8"
    )
    write_json(
        out_dir / "proof_manifest.json",
        {
            "schema": f"{LANE}_opaque_root_fixture_contracts_proof_manifest/v1",
            "created_utc": proof["created_utc"],
            "completion_status": proof["completion_status"],
            "backlog_reduced": proof["backlog_reduced"],
            "exact_backlog_item_removed_or_narrowed": proof[
                "exact_backlog_item_removed_or_narrowed"
            ],
            "files": [
                "opaque_root_fixture_contracts.json",
                "OPAQUE_ROOT_FIXTURE_CONTRACTS.md",
                "proof_manifest.json",
            ],
            "validated_roots": [],
            "rejected_candidates": proof["rejected_candidates_kept_rejected"],
            "compact_gate_policy": proof["compact_gate_policy"],
            "production_flags": {
                "dll_only": False,
                "compact_stream": False,
                "rust_decoded": False,
                "hoi4replay_verified": False,
                "speed_five_current_equivalent_hourly_or_lower": False,
                "zero_skipped_hours": False,
                "partial_source_worker_bridge_failures": False,
            },
        },
    )
    print(out_dir / "opaque_root_fixture_contracts.json")
    print(out_dir / "OPAQUE_ROOT_FIXTURE_CONTRACTS.md")
    print(out_dir / "proof_manifest.json")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
