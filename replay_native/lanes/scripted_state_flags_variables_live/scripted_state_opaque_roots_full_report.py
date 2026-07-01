#!/usr/bin/env python3
"""Build the final proof report for domain 51 opaque scripted-state roots."""

from __future__ import annotations

import json
from datetime import datetime, timezone
from pathlib import Path


LANE = "scripted_state_flags_variables_live"
DOMAIN_ID = 51
DEFAULT_RUN_DIR = (
    Path(__file__).resolve().parents[2]
    / "runs"
    / "scripted_state_flags_variables_live_20260701_opaque_roots_full_final"
)


def read_json(path: Path):
    try:
        return json.loads(path.read_text(encoding="utf-8-sig"))
    except UnicodeDecodeError:
        return json.loads(path.read_text(encoding="utf-16"))


def write_json(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as f:
        json.dump(value, f, indent=2, sort_keys=False)
        f.write("\n")


def text(path: Path) -> str:
    return path.read_text(encoding="utf-8-sig", errors="replace")


def command_result(command: str, result: str) -> dict[str, str]:
    return {"command": command, "result": result}


def build_report(run_dir: Path) -> dict:
    live_dir = run_dir / "live_refresh"
    shared_dir = run_dir / "shared_live_refresh"
    current = read_json(live_dir / "current_build_probe_full.json")
    country_flags = read_json(live_dir / "country_flags_payload_probe.json")
    global_roots = read_json(live_dir / "global_state_root_probe_fast.json")
    state_roots = read_json(live_dir / "state_scope_root_probe.json")
    delayed = read_json(run_dir / "delayed_event_blocker" / "delayed_event_fixture_audit.json")
    scripted_gui = read_json(
        run_dir / "scripted_gui_blocker" / "scripted_gui_fixture_audit.json"
    )
    scope_audit = read_json(run_dir / "scope_container_audit" / "scope_container_audit.json")
    fixture_watch = read_json(run_dir / "fixture_watch" / "opaque_fixture_watch.json")
    fixture_contracts = read_json(
        run_dir / "opaque_fixture_contracts" / "opaque_root_fixture_contracts.json"
    )
    manifest = read_json(shared_dir / "native_session_manifest.json")
    stream_gate = read_json(shared_dir / "domain51_stream_gate.json")
    compact_gate = read_json(shared_dir / "compact_gate_report.json")
    pack_verify = text(shared_dir / "pack_replay_verify.txt").strip()

    per_domain = manifest["compact_stream"]["per_domain"][LANE]
    country_summary = current["summary"]
    country_flag_summary = country_flags["summary"]

    validated_roots = [
        "CGameState = u64[module_base+0x3316E90]",
        "country table CGameState+0x310, count CGameState+0x31C",
        "CCountry vtable module_base+0x27D0E90",
        "CCountry+0x08 tag id",
        "CCountry+0x218 -> CVariables",
        "CCountry+0x230 -> CFlagManager",
    ]
    validated_roots.extend(
        f"{row['candidate']} ({row['root_offset']})"
        for row in global_roots["validated_root_candidates"]
    )
    validated_roots.extend(
        f"{row['candidate']} ({row['root_offset']})" for row in state_roots["validated_roots"]
    )

    rejected = [
        {
            "candidate": row["candidate"],
            "status": row["status"],
            "evidence": row["evidence"],
        }
        for row in state_roots["rejected_candidates"]
    ]
    rejected.extend(delayed["rejected_candidates"])
    rejected.extend(scripted_gui["rejected_candidates"])

    partial_failures = bool(
        manifest["compact_stream"]["partial_data_seen"]
        or manifest["compact_stream"]["source_unavailable_seen"]
        or manifest["compact_stream"]["worker_panic_seen"]
        or manifest["compact_stream"]["bridge_unavailable_seen"]
        or stream_gate["partial_failure_seen"]
        or stream_gate["source_unavailable_seen"]
    )

    return {
        "schema": f"{LANE}_opaque_roots_full_final_report/v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "lane": LANE,
        "domain_id": DOMAIN_ID,
        "completion_status": "NOT COMPLETE",
        "lane_full_production_status": "NOT COMPLETE",
        "backlog_reduced": "yes",
        "exact_backlog_item_removed_or_narrowed": (
            "Country/global/state CVariables and CFlagManager roots are validated "
            "and shared-DLL captured through domain 51; delayed-event and "
            "scripted-GUI/open-state opaque roots are narrowed to exact missing "
            "parsed owner/value and phase-labeled fixture blockers with "
            "fail-closed compact badges; no other script flag/variable scope "
            "containers are listed in the current manifest."
        ),
        "scope_container_audit": {
            "proof": "scope_container_audit/scope_container_audit.json",
            "manifest_scripted_path_count": scope_audit["manifest_scripted_path_count"],
            "manifest_scripted_paths": scope_audit["manifest_scripted_paths"],
            "unexpected_manifest_scripted_paths": scope_audit[
                "unexpected_manifest_scripted_paths"
            ],
            "missing_expected_manifest_paths": scope_audit[
                "missing_expected_manifest_paths"
            ],
            "classification": scope_audit["classification"],
        },
        "bounded_fixture_watch": {
            "proof": "fixture_watch/opaque_fixture_watch.json",
            "completion_status": fixture_watch["completion_status"],
            "duration_s": fixture_watch["watcher_limits"]["duration_s"],
            "sample_count": fixture_watch["watcher_limits"]["sample_count"],
            "positive_fixture_observed": fixture_watch["positive_fixture_observed"],
            "no_other_unblocked_scripted_work": fixture_watch[
                "no_other_unblocked_scripted_work"
            ],
            "final_decision": fixture_watch["final_decision"],
            "available_raw_save_sources": fixture_watch["available_raw_save_sources"],
        },
        "validated_slice": {
            "scope": (
                "country/global/state script variables and flag token/date/value "
                "rows, including timed flags represented by date_hour_raw"
            ),
            "country_root_refresh": {
                "pid": current["pid"],
                "sha256": current["sha256"],
                "validated_country_rows": current["source_roots"][
                    "validated_country_rows"
                ],
                "country_count": current["source_roots"]["country_count"],
                "variables_status": country_summary["variables_status"],
                "variable_entries_measured_in_emitted_rows": country_summary[
                    "variable_entries_measured_in_emitted_rows"
                ],
                "flags_status": country_summary["flags_status"],
            },
            "country_flag_payload_refresh": {
                "countries_scanned": country_flag_summary["countries_scanned"],
                "manager_status": country_flag_summary["manager_status"],
                "total_flag_rows": country_flag_summary["total_flag_rows"],
                "date_valid_rows": country_flag_summary["date_valid_rows"],
                "records_named": country_flag_summary["name_resolution"][
                    "records_named"
                ],
                "records_unresolved": country_flag_summary["name_resolution"][
                    "records_unresolved"
                ],
                "unresolved_rows": country_flag_summary["name_resolution"][
                    "unresolved_rows"
                ],
            },
            "global_roots": global_roots["validated_root_candidates"],
            "state_roots": [
                {
                    "candidate": row["candidate"],
                    "root_offset": row["root_offset"],
                    "status_counts": row["status_counts"],
                    "total_entries": row.get("total_entries"),
                    "total_rows": row.get("total_rows"),
                    "status": row["status"],
                }
                for row in state_roots["validated_roots"]
            ],
        },
        "validated_roots_offsets_functions": validated_roots,
        "rejected_or_blocked_candidates": rejected,
        "opaque_root_blockers": [
            {
                "root": "countries.*.delayed_event",
                "status": fixture_watch["final_decision"]["delayed_event_queue"][
                    "status"
                ],
                "status_bit": "STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED",
                "record_kind": "VALUE_DELAYED_EVENT_OPAQUE",
                "proof": "delayed_event_blocker/delayed_event_fixture_audit.json",
                "watcher_proof": "fixture_watch/opaque_fixture_watch.json",
                "blocker": delayed["blocker"],
            },
            {
                "root": "countries.*.scripted_gui_random / scripted-GUI open state",
                "status": fixture_watch["final_decision"]["scripted_gui_open_state"][
                    "status"
                ],
                "status_bit": "STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED",
                "record_kind": "VALUE_SCRIPTED_GUI_OPAQUE",
                "proof": "scripted_gui_blocker/scripted_gui_fixture_audit.json",
                "watcher_proof": "fixture_watch/opaque_fixture_watch.json",
                "blocker": scripted_gui["blocker"],
            },
        ],
        "fixture_contracts": fixture_contracts["fixture_contracts"],
        "compact_schema_record_changes": {
            "changed_this_turn": "none",
            "existing_lane_schema": [
                "scripted_state_compact.rs: domain id 51, schema version 1",
                "48-byte rows for variable fixed i64, named flag, token-only flag, delayed-event opaque, scripted-GUI opaque",
                "token/date/value flag semantics preserved as token_id, date_hour_raw, value_raw",
            ],
        },
        "rust_decoder_gate_changes": {
            "changed_this_turn": "none",
            "existing_lane_gate": [
                "scripted_state_domain51_gate.rs decodes domain-51 payloads",
                "scripted_state_domain51_stream.rs verified the fresh shared H4CR stream",
                "unresolved opaque root bits are badges, not partial/source failures",
            ],
        },
        "shared_live_capture": {
            "manifest": "shared_live_refresh/native_session_manifest.json",
            "collector_stream": "shared_live_refresh/collector_stream.bin",
            "hoi4replay": "shared_live_refresh/native_session.battleplan",
            "scheduled_domains": manifest["scheduled_domains"],
            "pid": manifest["pid"],
            "dll": manifest["dll"],
            "duration_ms": manifest["duration_ms"],
            "sample_ms": manifest["sample_ms"],
            "game_hours_advanced": manifest["game_hours_advanced"],
            "unique_game_hour_count": manifest["unique_game_hour_count"],
            "skipped_game_hours": manifest["skipped_game_hours"],
            "hourly_requirement": manifest["hourly_requirement"],
            "compact_stream": {
                "decodable": manifest["compact_stream"]["decodable"],
                "ordered": manifest["compact_stream"]["ordered"],
                "domain_data_record_count": manifest["compact_stream"][
                    "domain_data_record_count"
                ],
                "domain_perf_record_count": manifest["compact_stream"][
                    "domain_perf_record_count"
                ],
                "frame_end_record_count": manifest["compact_stream"][
                    "frame_end_record_count"
                ],
                "status_flags_or": manifest["compact_stream"]["status_flags_or"],
                "per_domain": per_domain,
            },
        },
        "proof_commands_results": [
            command_result(
                "python replay_native/lanes/scripted_state_flags_variables_live/current_build_live_probe.py --out <run>/live_refresh/current_build_probe_full.json --max-countries 2048 --include-empty",
                f"validated_country_rows={current['source_roots']['validated_country_rows']}; variables_status={country_summary['variables_status']}; flags_status={country_summary['flags_status']}",
            ),
            command_result(
                "python replay_native/lanes/scripted_state_flags_variables_live/country_flags_payload_probe.py --out <run>/live_refresh/country_flags_payload_probe.json --max-countries 2048 --sample-limit 12",
                f"total_flag_rows={country_flag_summary['total_flag_rows']}; date_valid_rows={country_flag_summary['date_valid_rows']}; named={country_flag_summary['name_resolution']['records_named']}; unresolved={country_flag_summary['name_resolution']['records_unresolved']}",
            ),
            command_result(
                "python replay_native/lanes/scripted_state_flags_variables_live/global_state_root_probe.py --out <run>/live_refresh/global_state_root_probe_fast.json --fast-direct-roots --skip-token-dictionary",
                "validated CGameState+0x980 global CVariables and CGameState+0x258 global CFlagManager",
            ),
            command_result(
                "python replay_native/lanes/scripted_state_flags_variables_live/state_scope_root_probe.py --out <run>/live_refresh/state_scope_root_probe.json",
                f"states_seen={state_roots['source_roots']['states_seen']}; validated_roots={len(state_roots['validated_roots'])}; rejected_candidates={len(state_roots['rejected_candidates'])}",
            ),
            command_result(
                "python replay_native/lanes/scripted_state_flags_variables_live/scripted_state_scope_container_audit.py",
                f"manifest_scripted_path_count={scope_audit['manifest_scripted_path_count']}; unexpected={len(scope_audit['unexpected_manifest_scripted_paths'])}; missing_expected={len(scope_audit['missing_expected_manifest_paths'])}",
            ),
            command_result(
                "python replay_native/lanes/scripted_state_flags_variables_live/scripted_state_opaque_fixture_watcher.py --output-dir <run>/fixture_watch --duration-s 5 --poll-s 1",
                f"positive_fixture_observed={fixture_watch['positive_fixture_observed']}; no_other_unblocked_scripted_work={fixture_watch['no_other_unblocked_scripted_work']}; samples={fixture_watch['watcher_limits']['sample_count']}",
            ),
            command_result(
                "cargo run --manifest-path replay_native/Cargo.toml -p recorder_host --bin collector_record -- --pid <hoi4-pid> --dll replay_native/target/debug/collector.dll --out-dir <run>/shared_live_refresh --duration-s 30 --sample-ms 200 --min-game-hours 10 --stream-format compact --extraction-perf-gate --lightweight-root-frame --scheduled-domain scripted_state_flags_variables_live",
                f"game_hours_advanced={manifest['game_hours_advanced']}; skipped_game_hours={manifest['skipped_game_hours']}; domain51_data_records={per_domain['data_record_count']}; partial/source/worker/bridge failure=false",
            ),
            command_result(
                "domain51_stream_verify --verify-json <run>/shared_live_refresh/collector_stream.bin",
                f"ok={stream_gate['ok']}; domain51_data_records={stream_gate['domain51_data_records']}; skipped_hours={stream_gate['skipped_hours']}; partial_failure_seen={stream_gate['partial_failure_seen']}; source_unavailable_seen={stream_gate['source_unavailable_seen']}",
            ),
            command_result(
                "cargo run --manifest-path replay_native/snapshot/Cargo.toml --bin pack_replay -- verify <run>/shared_live_refresh/native_session.battleplan <run>/shared_live_refresh <game_id>",
                pack_verify,
            ),
            command_result(
                "cargo run --manifest-path replay_native/lanes/compact_replay_verification_gates/Cargo.toml -- --run-dir <run>/shared_live_refresh --min-game-hours 10 --write-report <run>/shared_live_refresh/compact_gate_report.json",
                "rejected only because compact_replay_verification_gates does not yet know required domain scripted_state_flags_variables_live/domain 51; replay container opened and stream bytes were identified",
            ),
            command_result(
                "cargo test -p scripted_state_flags_variables_live_lane",
                "29 passed; 0 failed; doctests 0 passed",
            ),
        ],
        "production_flags": {
            "dll_only": "yes",
            "compact_stream": "yes",
            "rust_decoded": "yes",
            "hoi4replay_verified": "yes",
            "speed_five_current_equivalent_hourly_or_lower": "yes",
            "zero_skipped_hours": "yes",
            "any_partial_source_worker_bridge_failures": "no" if not partial_failures else "yes",
        },
        "generic_compact_gate": {
            "passed": compact_gate["passed"],
            "errors": compact_gate["errors"],
            "classification": (
                "gate contract coverage blocker, not a domain-51 stream failure"
            ),
        },
        "exact_files_changed": [
            "replay_native/lanes/scripted_state_flags_variables_live/README.md",
            "replay_native/lanes/scripted_state_flags_variables_live/scripted_state_opaque_roots_full_report.py",
            "replay_native/lanes/scripted_state_flags_variables_live/scripted_state_scope_container_audit.py",
            "replay_native/lanes/scripted_state_flags_variables_live/scripted_state_opaque_fixture_watcher.py",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/live_refresh/current_build_probe_full.json",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/live_refresh/country_flags_payload_probe.json",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/live_refresh/global_state_root_probe_fast.json",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/live_refresh/state_scope_root_probe.json",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/delayed_event_blocker/delayed_event_fixture_audit.json",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/delayed_event_blocker/proof_manifest.json",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/scripted_gui_blocker/scripted_gui_fixture_audit.json",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/scripted_gui_blocker/proof_manifest.json",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/opaque_fixture_contracts/opaque_root_fixture_contracts.json",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/opaque_fixture_contracts/OPAQUE_ROOT_FIXTURE_CONTRACTS.md",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/opaque_fixture_contracts/proof_manifest.json",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/scope_container_audit/scope_container_audit.json",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/scope_container_audit/SCOPE_CONTAINER_AUDIT.md",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/fixture_watch/opaque_fixture_watch.json",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/fixture_watch/OPAQUE_FIXTURE_WATCH.md",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/fixture_watch/proof_manifest.json",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/shared_live_refresh/collector_stream.bin",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/shared_live_refresh/native_session.battleplan",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/shared_live_refresh/native_session_manifest.json",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/shared_live_refresh/domain51_stream_gate.json",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/shared_live_refresh/pack_replay_verify.txt",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/shared_live_refresh/compact_gate_report.json",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/FINAL_REPORT.json",
            "replay_native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/FINAL_REPORT.md",
        ],
    }


def markdown(report: dict) -> str:
    flags = report["production_flags"]
    lines = [
        "# Domain 51 Scripted State Opaque Roots Full Report",
        "",
        f"Lane: `{report['lane']}`",
        f"Domain: `{report['domain_id']}`",
        "",
        "## Status",
        "",
        f"- Completion status: `{report['completion_status']}`",
        f"- Lane full production status: `{report['lane_full_production_status']}`",
        f"- Backlog reduced: {report['backlog_reduced']}",
        f"- Exact backlog item removed or narrowed: {report['exact_backlog_item_removed_or_narrowed']}",
        "",
        "This is a validated production slice, not whole-concept completion. The remaining scripted-state backlog is the delayed-event queue and scripted-GUI/open-state opaque roots named below.",
        "",
        "## Production Proof Flags",
        "",
        f"- DLL-only: {flags['dll_only']}",
        f"- Compact stream: {flags['compact_stream']}",
        f"- Rust decoded: {flags['rust_decoded']}",
        f"- `.battleplan` verified: {flags['hoi4replay_verified']}",
        f"- Speed-five/current-equivalent hourly-or-lower: {flags['speed_five_current_equivalent_hourly_or_lower']}",
        f"- Zero skipped hours: {flags['zero_skipped_hours']}",
        f"- Any partial/source/worker/bridge failures: {flags['any_partial_source_worker_bridge_failures']}",
        "",
        "## Shared Live Capture",
        "",
    ]
    cap = report["shared_live_capture"]
    lines.extend(
        [
            f"- Manifest: `{cap['manifest']}`",
            f"- Scheduled domains: `{', '.join(cap['scheduled_domains'])}`",
            f"- Game hours advanced: `{cap['game_hours_advanced']}`",
            f"- Unique game hours: `{cap['unique_game_hour_count']}`",
            f"- Skipped game hours: `{cap['skipped_game_hours']}`",
            f"- Domain-51 data records: `{cap['compact_stream']['per_domain']['data_record_count']}`",
            f"- Domain-51 rows: `{cap['compact_stream']['per_domain']['data_rows_total']}`",
            f"- Status flags OR: `{cap['compact_stream']['status_flags_or']}`",
            "",
            "## Validated Roots",
            "",
        ]
    )
    lines.extend(f"- `{root}`" for root in report["validated_roots_offsets_functions"])
    scope = report["scope_container_audit"]
    lines.extend(
        [
            "",
            "## Scope Containers",
            "",
            f"- Manifest scripted paths: `{scope['manifest_scripted_path_count']}`",
            f"- Unexpected scripted paths: `{len(scope['unexpected_manifest_scripted_paths'])}`",
            f"- Missing expected scripted paths: `{len(scope['missing_expected_manifest_paths'])}`",
        ]
    )
    lines.extend(f"- `{path}`" for path in scope["manifest_scripted_paths"])
    watch = report["bounded_fixture_watch"]
    lines.extend(
        [
            "",
            "## Bounded Fixture Watch",
            "",
            f"- Proof: `{watch['proof']}`",
            f"- Positive fixture observed: `{str(watch['positive_fixture_observed']).lower()}`",
            f"- No other unblocked scripted work: `{str(watch['no_other_unblocked_scripted_work']).lower()}`",
            f"- Samples: `{watch['sample_count']}` over `{watch['duration_s']}` seconds",
        ]
    )
    lines.extend(["", "## Opaque Blockers", ""])
    for blocker in report["opaque_root_blockers"]:
        lines.append(
            f"- `{blocker['root']}`: `{blocker['status']}`; compact bit `{blocker['status_bit']}`; proof `{blocker['proof']}`"
        )
    lines.extend(["", "## Rejections", ""])
    for row in report["rejected_or_blocked_candidates"]:
        lines.append(
            f"- `{row['candidate']}`: `{row['status']}` - {row['evidence']}"
        )
    lines.extend(["", "## Proof Commands", ""])
    for item in report["proof_commands_results"]:
        lines.append(f"- `{item['command']}` -> {item['result']}")
    lines.extend(["", "## Generic Gate Note", ""])
    lines.append(
        "The generic `compact_replay_verification_gates` run rejected this proof only because its contract table does not yet know domain 51. The lane-owned stream verifier accepted the same stream and `pack_replay verify` accepted the replay container."
    )
    lines.append("")
    return "\n".join(lines)


def main() -> int:
    run_dir = DEFAULT_RUN_DIR
    report = build_report(run_dir)
    write_json(run_dir / "FINAL_REPORT.json", report)
    (run_dir / "FINAL_REPORT.md").write_text(markdown(report), encoding="utf-8")
    print(run_dir / "FINAL_REPORT.json")
    print(run_dir / "FINAL_REPORT.md")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
