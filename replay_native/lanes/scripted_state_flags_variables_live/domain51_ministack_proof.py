"""Emit proof metadata for the domain-51 scripted-state mini stack."""

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
VALID_PRE_INTEGRATION_STATUSES = {
    "reserved",
    "lane_ready_opaque_positive_fixtures_blocked",
    "validated_lane_slice_positive_country_flags_shared_registration_blocked",
}

LANE_DIR = Path(__file__).resolve().parent
REPO_ROOT = LANE_DIR.parents[2]
DEFAULT_OUT_DIR = (
    REPO_ROOT
    / "replay_native"
    / "runs"
    / "scripted_state_flags_variables_live_870ebf_20260701_domain51_ministack_v1"
)


def read_json(path: Path):
    with path.open("r", encoding="utf-8") as f:
        return json.load(f)


def write_json(path: Path, payload) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as f:
        json.dump(payload, f, indent=2, sort_keys=False)
        f.write("\n")


def registry_assignment():
    registry = read_json(REPO_ROOT / "replay_native" / "domain_registry.json")
    matches = [row for row in registry.get("domains", []) if row.get("id") == DOMAIN_ID]
    return {
        "file": "replay_native/domain_registry.json",
        "domain_id": DOMAIN_ID,
        "matches": matches,
        "valid": len(matches) == 1
        and matches[0].get("name") == LANE
        and matches[0].get("status") in VALID_PRE_INTEGRATION_STATUSES,
    }


def maybe_read_json(path: Path):
    if not path.exists():
        return None
    return read_json(path)


def summarize_live_refresh(out_dir: Path):
    live_dir = out_dir / "live_refresh"
    current = maybe_read_json(live_dir / "current_build_probe_full.json")
    country_flags = maybe_read_json(live_dir / "country_flags_payload_probe.json")
    global_roots = maybe_read_json(live_dir / "global_state_root_probe_fast.json")
    state_roots = maybe_read_json(live_dir / "state_scope_root_probe.json")
    if not any((current, country_flags, global_roots, state_roots)):
        return {
            "status": "not_run",
            "note": "No live_refresh proof JSONs were present when the manifest was generated.",
        }

    summary = {"status": "present", "files": []}
    if current:
        summary["files"].append("live_refresh/current_build_probe_full.json")
        roots = current.get("source_roots", {})
        summary["country_root_refresh"] = {
            "sha256": current.get("sha256"),
            "pid": current.get("pid"),
            "elapsed_ms": current.get("elapsed_ms"),
            "country_count": roots.get("country_count"),
            "validated_country_rows": roots.get("validated_country_rows"),
            "summary": current.get("summary"),
        }
    if country_flags:
        summary["files"].append("live_refresh/country_flags_payload_probe.json")
        flag_summary = country_flags.get("summary", {})
        summary["country_flag_payload_refresh"] = {
            "sha256": country_flags.get("sha256"),
            "pid": country_flags.get("pid"),
            "elapsed_ms": country_flags.get("elapsed_ms"),
            "countries_scanned": flag_summary.get("countries_scanned"),
            "manager_status": flag_summary.get("manager_status"),
            "total_flag_rows": flag_summary.get("total_flag_rows"),
            "date_valid_rows": flag_summary.get("date_valid_rows"),
            "name_resolution": flag_summary.get("name_resolution"),
            "dictionary": flag_summary.get("dictionary"),
        }
    if global_roots:
        summary["files"].append("live_refresh/global_state_root_probe_fast.json")
        summary["global_root_refresh"] = {
            "elapsed_ms": global_roots.get("elapsed_ms"),
            "validated_root_candidates": global_roots.get("validated_root_candidates"),
            "rejected_root_candidates": global_roots.get("rejected_root_candidates"),
        }
    if state_roots:
        summary["files"].append("live_refresh/state_scope_root_probe.json")
        summary["state_root_refresh"] = {
            "sha256": state_roots.get("sha256"),
            "pid": state_roots.get("pid"),
            "elapsed_ms": state_roots.get("elapsed_ms"),
            "source_roots": {
                key: state_roots.get("source_roots", {}).get(key)
                for key in (
                    "province_count",
                    "province_rows",
                    "states_seen",
                    "state_id_offset",
                    "cprovince_vtable",
                    "state_vtable",
                )
            },
            "validated_roots": [
                {
                    "candidate": root.get("candidate"),
                    "root_offset": root.get("root_offset"),
                    "state_count": root.get("state_count"),
                    "status_counts": root.get("status_counts"),
                    "total_entries": root.get("total_entries"),
                    "total_rows": root.get("total_rows"),
                    "status": root.get("status"),
                }
                for root in state_roots.get("validated_roots", [])
            ],
            "rejected_candidates": state_roots.get("rejected_candidates"),
            "rejected_partial_offsets": state_roots.get("rejected_partial_offsets"),
        }
    return summary


def build_manifest(out_dir: Path):
    registry = registry_assignment()
    live_refresh = summarize_live_refresh(out_dir)
    return {
        "schema": f"{LANE}_domain51_ministack_proof/v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "lane": LANE,
        "target_build": {
            "sha256": EXPECTED_SHA_870EBF,
            "process": "hoi4.exe",
        },
        "domain_registry": registry,
        "completion_status": "COMPLETE",
        "lane_full_production_status": "NOT_COMPLETE",
        "backlog_reduced": True,
        "exact_backlog_item_removed_or_narrowed": (
            "Built a lane-owned domain-51 mini stack for validated "
            "country/global/state variables and flags; delayed-event and "
            "scripted-GUI roots remain fail-closed fixture blockers only."
        ),
        "lane_owned_modules": [
            {
                "file": "replay_native/lanes/scripted_state_flags_variables_live/scripted_state_domain51_collector.rs",
                "kind": "collector/extractor/encoder module",
                "entrypoints": [
                    "collect_domain51_frame",
                    "registration_descriptor",
                ],
                "coverage": [
                    "validated country/global/state variable rows",
                    "validated country/global/state flag rows",
                    "token-only flag rows when dictionary text is unavailable",
                    "opaque unresolved delayed-event badge rows",
                    "opaque unresolved scripted-GUI badge rows",
                ],
            },
            {
                "file": "replay_native/lanes/scripted_state_flags_variables_live/scripted_state_compact.rs",
                "kind": "compact schema encoder/decoder",
                "entrypoints": ["encode_payload", "decode_payload"],
                "domain_id": DOMAIN_ID,
                "schema_version": 1,
                "header_bytes": 64,
                "row_bytes": 48,
            },
            {
                "file": "replay_native/lanes/scripted_state_flags_variables_live/scripted_state_domain51_gate.rs",
                "kind": "lane-owned Rust decoder/gate module",
                "entrypoints": ["gate_domain51_payload"],
                "policy": (
                    "parse errors, STATUS_PARTIAL_DATA, or STATUS_SOURCE_UNAVAILABLE "
                    "are failures; delayed/scripted-GUI unresolved bits are badges only"
                ),
            },
        ],
        "validated_roots": [
            "CGameState = u64[module_base+0x3316E90]",
            "country table CGameState+0x310, count CGameState+0x31C",
            "CCountry vtable module_base+0x27D0E90",
            "CCountry+0x08 tag id",
            "CCountry+0x218 -> CVariables",
            "CCountry+0x230 -> CFlagManager",
            "CGameState+0x980 -> CVariables",
            "CGameState+0x258 -> CFlagManager",
            "CGameState+0x2B0 CProvince array, CGameState+0x2BC count",
            "CProvince vtable module_base+0x295AF68",
            "CProvince+0xC0 -> CState",
            "CState vtable module_base+0x29206C0",
            "CState+0x58 state id",
            "CState+0x7F8 -> CVariables",
            "CState+0x540 -> CFlagManager",
        ],
        "validated_layouts": {
            "CVariables": {
                "node_base": "CVariables+0x18",
                "packed_size_mask": "CVariables+0x20",
                "node_stride": "0x30",
                "key": "MSVC std::string at node+0x00",
                "value_raw": "i64 at node+0x20, scale 100000",
                "gate": "non-empty scope accepted only when one unambiguous phase has distinct decoded keys == declared size",
            },
            "CFlagManager": {
                "rows": "CFlagManager+0x08",
                "entry_count": "high u32 of CFlagManager+0x10",
                "slot_or_capacity_count": "low u32 of CFlagManager+0x10",
                "row_stride": "0x30",
                "token_id": "low u32 at row+0x08",
                "date_hour_raw": "low u32 at row+0x18",
                "value": "low u32 at row+0x28",
                "gate": "emit named flag when text resolves; otherwise token-only with STATUS_FLAG_NAMES_PARTIAL",
            },
        },
        "fail_closed_blockers": [
            {
                "root": "delayed_events",
                "status_bit": "STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED",
                "proof": "replay_native/runs/scripted_state_flags_variables_live_870ebf_20260701_delayed_event_blocker_v1/delayed_event_fixture_audit.json",
                "blocker": "missing_positive_save_fixture",
            },
            {
                "root": "scripted_gui_open_state",
                "status_bit": "STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED",
                "proof": "replay_native/runs/scripted_state_flags_variables_live_870ebf_20260701_scripted_gui_blocker_v1/scripted_gui_fixture_audit.json",
                "blocker": "missing_positive_scripted_gui_open_fixture",
            },
        ],
        "registration_hook_plan": {
            "file": "replay_native/lanes/scripted_state_flags_variables_live/REGISTRATION_HOOK_PLAN.md",
            "collector": (
                "register domain 51 through the new CollectorDomainModule-style "
                "module hook; keep shared collector changes registration-only"
            ),
            "recorder": (
                "map domain 51 to the lane-owned compact gate; treat unresolved "
                "opaque bits as badges rather than partial failures"
            ),
        },
        "live_refresh": live_refresh,
        "production_flags": {
            "dll_only": False,
            "compact_stream": False,
            "rust_decoded": True,
            "hoi4replay_verified": False,
            "speed_five_current_equivalent_hourly_or_lower": False,
            "zero_skipped_hours": False,
            "partial_source_worker_bridge_failures": True,
        },
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", default=str(DEFAULT_OUT_DIR))
    args = parser.parse_args()
    out_dir = Path(args.output_dir)
    manifest = build_manifest(out_dir)
    write_json(out_dir / "domain51_ministack_proof.json", manifest)
    write_json(
        out_dir / "proof_manifest.json",
        {
            "schema": f"{LANE}_domain51_ministack_proof_manifest/v1",
            "created_utc": manifest["created_utc"],
            "completion_status": manifest["completion_status"],
            "lane_full_production_status": manifest["lane_full_production_status"],
            "backlog_reduced": manifest["backlog_reduced"],
            "exact_backlog_item_removed_or_narrowed": manifest[
                "exact_backlog_item_removed_or_narrowed"
            ],
            "domain_registry": manifest["domain_registry"],
            "lane_owned_modules": manifest["lane_owned_modules"],
            "validated_roots": manifest["validated_roots"],
            "fail_closed_blockers": manifest["fail_closed_blockers"],
            "registration_hook_plan": manifest["registration_hook_plan"],
            "live_refresh": manifest["live_refresh"],
            "production_flags": manifest["production_flags"],
        },
    )
    print(f"wrote {out_dir / 'domain51_ministack_proof.json'}")
    print(f"wrote {out_dir / 'proof_manifest.json'}")


if __name__ == "__main__":
    main()
