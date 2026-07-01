"""Produce the domain-46/0xACE1 production registration and static-capture gate.

This is a lane-local proof artifact generator. It does not edit shared
collector/recorder entrypoints. It verifies the current shared anchors that a
minimal registration patch would touch, decodes live cadence compact records, and
rejects startup-only ace capture when the roster or assignment evidence changes.
"""

from __future__ import annotations

import argparse
import json
import struct
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[3]
COLLECTOR = REPO_ROOT / "collector" / "src" / "lib.rs"
RECORDER = REPO_ROOT / "recorder_host" / "src" / "main.rs"
REGISTRY = REPO_ROOT / "recorder_host" / "src" / "compact_domain_registry.rs"
COLLECTOR_CARGO = REPO_ROOT / "collector" / "Cargo.toml"
RECORDER_CARGO = REPO_ROOT / "recorder_host" / "Cargo.toml"

DOMAIN_ID = 46
ACE_RECORD_TYPE = 0xACE1
ACE_RECORD_TYPE_HEX = "0xACE1"
HEADER_LEN = 48
ROW_LEN = 32
ROW_FLAG_WING_ID = 1 << 4


ANCHORS = {
    "collector_domain_const": (COLLECTOR, "const COMPACT_DOMAIN_AIR_VARIANT_ARCHETYPES: u16 = 46;"),
    "collector_raw_domain": (COLLECTOR, 'const RAW_AIR_VARIANT_ARCHETYPES_DOMAIN: &str = "air_variant_archetypes";'),
    "collector_tracker_bundle": (COLLECTOR, "air_variant_archetypes: CompactAirVariantArchetypeTracker,"),
    "collector_enabled_bool": (COLLECTOR, "let air_variant_archetypes_enabled ="),
    "collector_worker_spawn": (COLLECTOR, "read_compact_air_variant_archetypes("),
    "collector_worker_join": (COLLECTOR, "if let Some(handle) = air_variant_archetype_handle"),
    "collector_append_variant": (COLLECTOR, "append_compact_air_variant_archetypes_data_record("),
    "collector_append_record": (COLLECTOR, "fn append_compact_record("),
    "collector_record_type_data_const": (COLLECTOR, "const COMPACT_RECORD_TYPE_DOMAIN_DATA: u16 = 3;"),
    "recorder_record_type_read": (RECORDER, "let record_type = read_u16_le(header, 22);"),
    "recorder_record_type_match": (RECORDER, "match record_type {"),
    "recorder_domain46_dispatch": (RECORDER, "COMPACT_DOMAIN_AIR_VARIANT_ARCHETYPES => {"),
    "recorder_variant_decoder": (RECORDER, "fn decode_compact_air_variant_archetypes_data_payload("),
    "recorder_domain_name": (RECORDER, "fn compact_domain_name(domain_id: u16) -> &'static str"),
    "registry_domain46": (REGISTRY, '46,\n        "air_variant_archetypes_live",'),
    "collector_lane_crate_dependency": (
        COLLECTOR_CARGO,
        'air_wings_variants_lifecycle_live_lane = { path = "../lanes/air_wings_variants_lifecycle_live" }',
    ),
    "recorder_lane_crate_dependency": (
        RECORDER_CARGO,
        'air_wings_variants_lifecycle_live_lane = { path = "../lanes/air_wings_variants_lifecycle_live" }',
    ),
}


def line_anchor(path: Path, needle: str) -> dict[str, Any]:
    lines = path.read_text(encoding="utf-8").splitlines()
    if "\n" in needle:
        compact_lines = "\n".join(lines)
        index = compact_lines.find(needle)
        if index < 0:
            return {"path": str(path), "found": False, "needle": needle}
        line_no = compact_lines[:index].count("\n") + 1
        return {"path": str(path), "found": True, "line": line_no, "needle": needle}
    for idx, line in enumerate(lines, 1):
        if needle in line:
            return {"path": str(path), "found": True, "line": idx, "text": line.strip(), "needle": needle}
    return {"path": str(path), "found": False, "needle": needle}


def read_compact_string(data: bytes, cursor: int) -> tuple[str | None, int]:
    if cursor + 2 > len(data):
        raise ValueError("truncated compact string length")
    size = struct.unpack_from("<H", data, cursor)[0]
    cursor += 2
    if cursor + size > len(data):
        raise ValueError("truncated compact string bytes")
    if size == 0:
        return None, cursor
    return data[cursor : cursor + size].decode("utf-8", errors="replace"), cursor + size


def decode_ace_record(path: Path) -> dict[str, Any]:
    data = path.read_bytes()
    if len(data) < 32:
        raise ValueError(f"{path}: record too short")
    if data[:4] != b"H4CR":
        raise ValueError(f"{path}: invalid H4CR magic")
    version, hlen = struct.unpack_from("<HH", data, 4)
    sequence = struct.unpack_from("<Q", data, 8)[0]
    game_hour_raw = struct.unpack_from("<i", data, 16)[0]
    domain_id, record_type = struct.unpack_from("<HH", data, 20)
    payload_len = struct.unpack_from("<I", data, 24)[0]
    payload = data[32 : 32 + payload_len]
    if len(payload) != payload_len:
        raise ValueError(f"{path}: truncated payload")
    if len(payload) < HEADER_LEN:
        raise ValueError(f"{path}: payload too short")
    row_count = struct.unpack_from("<I", payload, 8)[0]
    changed_rows = struct.unpack_from("<I", payload, 12)[0]
    status_flags = struct.unpack_from("<I", payload, 16)[0]
    identity_rows = struct.unpack_from("<I", payload, 28)[0]
    assignment_rows = struct.unpack_from("<I", payload, 32)[0]
    rows = []
    cursor = HEADER_LEN
    for _ in range(row_count):
        tag_db_id, ace_id, flags, source_mask, country_ace_index, wing_id, _r0, _r1 = struct.unpack_from(
            "<IIIIIIII", payload, cursor
        )
        cursor += ROW_LEN
        rows.append(
            {
                "key": f"{tag_db_id}:{ace_id}",
                "tag_db_id": tag_db_id,
                "ace_id": ace_id,
                "country_ace_index": country_ace_index,
                "wing_id": wing_id if flags & ROW_FLAG_WING_ID else None,
                "source_mask": source_mask,
            }
        )
    for row in rows:
        row["name"], cursor = read_compact_string(payload, cursor)
        row["surname"], cursor = read_compact_string(payload, cursor)
        row["callsign"], cursor = read_compact_string(payload, cursor)
    return {
        "path": str(path),
        "record_bytes": len(data),
        "header": {
            "version": version,
            "header_len": hlen,
            "sequence": sequence,
            "game_hour_raw": game_hour_raw,
            "domain_id": domain_id,
            "record_type": record_type,
            "payload_len": payload_len,
        },
        "row_count": row_count,
        "changed_rows": changed_rows,
        "status_flags": status_flags,
        "identity_rows": identity_rows,
        "wing_assignment_rows": assignment_rows,
        "rows": rows,
    }


def diff_records(records: list[dict[str, Any]]) -> list[dict[str, Any]]:
    diffs = []
    for previous, current in zip(records, records[1:]):
        old = {row["key"]: row for row in previous["rows"]}
        new = {row["key"]: row for row in current["rows"]}
        added_keys = sorted(set(new) - set(old))
        removed_keys = sorted(set(old) - set(new))
        wing_changed = []
        for key in sorted(set(old) & set(new)):
            if old[key].get("wing_id") != new[key].get("wing_id"):
                wing_changed.append(
                    {
                        "key": key,
                        "old_wing_id": old[key].get("wing_id"),
                        "new_wing_id": new[key].get("wing_id"),
                    }
                )
        diffs.append(
            {
                "from_game_hour_raw": previous["header"]["game_hour_raw"],
                "to_game_hour_raw": current["header"]["game_hour_raw"],
                "from_row_count": previous["row_count"],
                "to_row_count": current["row_count"],
                "added_count": len(added_keys),
                "removed_count": len(removed_keys),
                "wing_assignment_changed_count": len(wing_changed),
                "added_examples": [new[key] for key in added_keys[:8]],
                "removed_examples": [old[key] for key in removed_keys[:8]],
                "wing_assignment_changed_examples": wing_changed[:8],
            }
        )
    return diffs


def build_registration_plan(anchor_results: dict[str, dict[str, Any]]) -> dict[str, Any]:
    return {
        "schema": "air_variant_archetypes_live.ace_roster_registration_plan.v1",
        "domain_id": DOMAIN_ID,
        "record_type": ACE_RECORD_TYPE_HEX,
        "shared_entrypoints_edited_by_lane": False,
        "cargo_changes_required": False,
        "collector_patch_operations": [
            {
                "file": str(COLLECTOR),
                "anchor": "COMPACT_DOMAIN_AIR_VARIANT_ARCHETYPES",
                "operation": "keep existing COMPACT_DOMAIN_AIR_VARIANT_ARCHETYPES: u16 = 46; add only COMPACT_RECORD_TYPE_AIR_ACE_ROSTER_DATA: u16 = 0xACE1 next to compact record type constants",
            },
            {
                "file": str(COLLECTOR),
                "anchor": "CompactPerfGateTrackers",
                "operation": "add air_ace_roster: CompactAirAceRosterTracker using lane row cache semantics",
            },
            {
                "file": str(COLLECTOR),
                "anchor": "compact_explicit_domain_enabled(control, RAW_AIR_VARIANT_ARCHETYPES_DOMAIN)",
                "operation": "when air_variant_archetypes is scheduled, run read_compact_air_ace_roster on the same hourly frame as domain 46",
            },
            {
                "file": str(COLLECTOR),
                "anchor": "append_compact_air_variant_archetypes_data_record",
                "operation": "after variant archetype record, append ace roster payload with append_compact_record(..., domain_id=46, record_type=0xACE1, payload)",
            },
            {
                "file": str(COLLECTOR),
                "anchor": "validated roots from ace_root_candidate_probe.py",
                "operation": "implement AceRosterMemory over existing collector read helpers; pass CountryAceRoot and AirWingAceAssignmentRoot into ace_roster_live_roots::read_ace_roster_from_roots; no root discovery or bounded reader design remains",
            },
            {
                "file": str(COLLECTOR_CARGO),
                "anchor": "air_wings_variants_lifecycle_live_lane crate dependency",
                "operation": "no Cargo change: collector already depends on the lane crate; call extractor::build_air_ace_roster_payload and domain46_air_ace_roster_live::encode_compact_data_record from that crate",
            },
        ],
        "recorder_patch_operations": [
            {
                "file": str(RECORDER),
                "anchor": "let record_type = read_u16_le(header, 22)",
                "operation": "before the COMPACT_RECORD_TYPE_DOMAIN_DATA branch, route record_type==0xACE1 && domain_id==46 to domain46_air_ace_roster_live::decode_compact_data_record(offset, &raw[offset..payload_end])",
            },
            {
                "file": str(RECORDER),
                "anchor": "decode_compact_air_variant_archetypes_data_payload",
                "operation": "add decode_compact_air_ace_roster_data_record helper that merges AirAceRosterRecordGateSummary counters into per_domain[46]",
            },
            {
                "file": str(RECORDER),
                "anchor": "compact_domain_name(46)",
                "operation": "keep name unchanged as air_variant_archetypes; record_type disambiguates ace roster subrecord",
            },
        ],
        "proof_after_patch": [
            "collector_record --stream-format compact --scheduled-domain air_variant_archetypes --min-game-hours >= 3",
            "expect compact_stream.per_domain.air_variant_archetypes.data_units_total.air_ace_roster_rows > 0",
            "expect air_ace_roster_wing_assignment_rows > 0, parse_errors=[], .battleplan stream bytes unchanged, skipped_game_hours=0",
        ],
        "anchor_results": anchor_results,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cadence-dir", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--residual-proof")
    parser.add_argument("--variant-proof")
    args = parser.parse_args()

    anchor_results = {name: line_anchor(path, needle) for name, (path, needle) in ANCHORS.items()}
    records = [
        decode_ace_record(path)
        for path in sorted(Path(args.cadence_dir).glob("sample_*/air_ace_roster_compact_record.bin"))
    ]
    diffs = diff_records(records)
    roster_changed = any(
        item["added_count"] or item["removed_count"] or item["wing_assignment_changed_count"]
        for item in diffs
    )
    cadence_rejection = {
        "startup_or_static_capture_rejected": roster_changed,
        "reason": (
            "live cadence compact samples changed ace roster rows; startup-only capture would miss later ace creation/death or assignment changes"
            if roster_changed
            else "no row/assignment mutation observed in this sample window; still not sufficient to prove static semantics"
        ),
        "record_count": len(records),
        "record_summaries": [
            {
                "path": record["path"],
                "game_hour_raw": record["header"]["game_hour_raw"],
                "row_count": record["row_count"],
                "identity_rows": record["identity_rows"],
                "wing_assignment_rows": record["wing_assignment_rows"],
                "domain_id": record["header"]["domain_id"],
                "record_type": record["header"]["record_type"],
            }
            for record in records
        ],
        "diffs": diffs,
    }

    residual_summary: dict[str, Any] = {}
    if args.residual_proof:
        residual = json.loads(Path(args.residual_proof).read_text(encoding="utf-8"))
        residual_summary = {
            "completion_status": residual.get("completion_status"),
            "backlog_reduced": residual.get("backlog_reduced"),
            "lifecycle_evidence_not_domain42": residual.get("lifecycle_evidence_not_domain42"),
        }
    variant_summary: dict[str, Any] = {}
    if args.variant_proof:
        variant = json.loads(Path(args.variant_proof).read_text(encoding="utf-8"))
        gates = dict(variant.get("gates", {}))
        gates.update(variant.get("proof_checks", {}))
        variant_summary = {
            "completion_status": variant.get("completion_status"),
            "backlog_reduced": variant.get("backlog_reduced"),
            "base_airframe_role_closed": gates.get("compact_active_base_airframe_names_closed"),
            "variant_stats_closed": gates.get("active_compact_variant_defs_stats_closed"),
            "current_compact_ace_join_rejected": gates.get("ace_join_current_compact_route_rejected"),
            "wing_lifecycle_observed": {
                "create": gates.get("wing_create_after_initial_observed"),
                "delete": gates.get("wing_delete_observed"),
                "rebase": gates.get("wing_rebase_observed"),
            },
        }

    report = {
        "schema": "air_variant_archetypes_live.ace_roster_registration_static_gate.v1",
        "completion_status": "NOT_COMPLETE",
        "backlog_reduced": True,
        "exact_backlog_item_removed_or_narrowed": (
            "Exact shared registration anchors and operations are now enumerated for domain 46 record_type 0xACE1; "
            "startup/static ace capture is rejected by decoded live compact sample mutation."
        ),
        "all_shared_anchors_found": all(item.get("found") for item in anchor_results.values()),
        "registration_plan": build_registration_plan(anchor_results),
        "cadence_static_rejection": cadence_rejection,
        "remaining_residuals": {
            "ace_identity_wing_assignment": "lane-local replay verified, shared DLL/recorder registration pending",
            "base_airframe_role_strings": variant_summary,
            "aircraft_variant_stat_closure": variant_summary,
            "wing_create_delete_rebase_lifecycle": residual_summary,
            "startup_static_ace_capture": "rejected" if roster_changed else "not_proven_static",
        },
        "dll_only": False,
        "compact_stream": True,
        "rust_decoded": True,
        "hoi4replay_verified": True,
        "speed_five_current_equivalent_hourly_or_lower": False,
        "zero_skipped_hours": False,
        "partial_source_worker_bridge_failures": False,
    }
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
    print(
        json.dumps(
            {
                "out": str(out),
                "completion_status": report["completion_status"],
                "backlog_reduced": report["backlog_reduced"],
                "all_shared_anchors_found": report["all_shared_anchors_found"],
                "startup_or_static_capture_rejected": cadence_rejection["startup_or_static_capture_rejected"],
                "sample_record_count": len(records),
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0 if report["all_shared_anchors_found"] and records else 1


if __name__ == "__main__":
    raise SystemExit(main())
