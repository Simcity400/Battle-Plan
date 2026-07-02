"""Lane-local registry/registration anchor gate for air_wings_variants_lifecycle_live.

This does not edit shared collector or recorder entrypoints. It checks that the
current shared files still have the expected integration anchors, rejects stale
provisional ids because they are occupied, and verifies the compact-domain
registry assignment for the final domain id.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
from pathlib import Path
from typing import Any


THIS_DIR = Path(__file__).resolve().parent
REPO_ROOT = THIS_DIR.parents[3]
COLLECTOR = REPO_ROOT / "extractor/native" / "collector" / "src" / "lib.rs"
RECORDER = REPO_ROOT / "extractor/native" / "recorder_host" / "src" / "main.rs"
RECORDER_COMPACT_REGISTRY = REPO_ROOT / "extractor/native" / "recorder_host" / "src" / "compact_domain_registry.rs"
REGISTRY_JSON = REPO_ROOT / "extractor/native" / "domain_registry.json"
LANE_SCHEMA = THIS_DIR / "src" / "domain46_air_variant_archetypes_live.rs"

REJECTED_PROVISIONAL_DOMAIN_IDS = (44, 45)
ASSIGNED_DOMAIN_ID = 46
DOMAIN_NAME = "air_variant_archetypes_live"


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8", errors="replace")


def line_hits(text: str, pattern: str) -> list[dict[str, Any]]:
    rx = re.compile(pattern)
    rows: list[dict[str, Any]] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        if rx.search(line):
            rows.append({"line": line_no, "text": line.strip()})
    return rows


def first_hit(text: str, pattern: str) -> dict[str, Any] | None:
    hits = line_hits(text, pattern)
    return hits[0] if hits else None


def collector_domain_ids(text: str) -> dict[str, int]:
    out: dict[str, int] = {}
    rx = re.compile(r"const\s+(COMPACT_DOMAIN_[A-Z0-9_]+):\s*u16\s*=\s*(\d+)\s*;")
    for name, value in rx.findall(text):
        out[name] = int(value)
    return out


def rust_u16_consts(text: str) -> dict[str, int]:
    out: dict[str, int] = {}
    rx = re.compile(r"const\s+([A-Z0-9_]+):\s*u16\s*=\s*(\d+)\s*;")
    for name, value in rx.findall(text):
        out[name] = int(value)
    return out


def resolve_rust_u16(value: str, consts: dict[str, int]) -> int | None:
    if value.isdigit():
        return int(value)
    return consts.get(value)


def recorder_domain_id_map(text: str) -> dict[str, int]:
    out: dict[str, int] = {}
    consts = rust_u16_consts(text)
    rx = re.compile(r'"([^"]+)"\s*=>\s*(?:\{\s*)?Some\(\s*([A-Z0-9_]+|\d+)\s*\)', re.S)
    for name, value in rx.findall(text):
        resolved = resolve_rust_u16(value, consts)
        if resolved is not None:
            out[name] = resolved
    return out


def recorder_domain_name_map(text: str) -> dict[int, str]:
    out: dict[int, str] = {}
    consts = rust_u16_consts(text)
    rx = re.compile(r"^\s*([A-Z0-9_]+|\d+)\s*=>\s*(?:\{\s*)?\"([^\"]+)\"", re.MULTILINE | re.S)
    for value, name in rx.findall(text):
        resolved = resolve_rust_u16(value, consts)
        if resolved is not None:
            out[resolved] = name
    return out


def compact_registry_domain_map(text: str) -> tuple[dict[str, int], dict[int, str]]:
    ids: dict[str, int] = {}
    names: dict[int, str] = {}
    rx = re.compile(
        r"domain\(\s*(\d+)\s*,\s*\"([^\"]+)\"\s*,\s*\"([^\"]+)\"",
        re.MULTILINE,
    )
    for value, registry_name, wire_name in rx.findall(text):
        domain_id = int(value)
        ids[registry_name] = domain_id
        ids[wire_name] = domain_id
        names[domain_id] = registry_name
    return ids, names


def check_anchor(text: str, name: str, pattern: str) -> dict[str, Any]:
    hit = first_hit(text, pattern)
    return {
        "name": name,
        "ok": hit is not None,
        "pattern": pattern,
        "line": hit["line"] if hit else None,
        "text": hit["text"] if hit else None,
    }


def map_anchor(name: str, ok: bool, evidence: str) -> dict[str, Any]:
    return {
        "name": name,
        "ok": ok,
        "pattern": "parsed_compact_domain_registry_or_legacy_match",
        "line": None,
        "text": evidence if ok else None,
    }


def build_gate() -> dict[str, Any]:
    collector = read_text(COLLECTOR)
    recorder = read_text(RECORDER)
    recorder_compact_registry = read_text(RECORDER_COMPACT_REGISTRY)
    recorder_combined = recorder + "\n" + recorder_compact_registry
    schema = read_text(LANE_SCHEMA)
    registry = json.loads(read_text(REGISTRY_JSON))

    collector_ids = collector_domain_ids(collector)
    recorder_ids = recorder_domain_id_map(recorder)
    compact_registry_ids, compact_registry_names = compact_registry_domain_map(recorder_compact_registry)
    recorder_ids.update(compact_registry_ids)
    recorder_names = recorder_domain_name_map(recorder)
    recorder_names.update(compact_registry_names)
    occupied_ids = sorted(set(collector_ids.values()) | set(recorder_ids.values()) | set(recorder_names.keys()))

    collector_anchors = [
        check_anchor(collector, "raw_air_variant_defs_domain", r'RAW_AIR_VARIANT_DEFS_DOMAIN:\s*&str\s*=\s*"air_variant_defs"'),
        check_anchor(collector, "raw_air_variant_stats_domain", r'RAW_AIR_VARIANT_STATS_DOMAIN:\s*&str\s*=\s*"air_variant_stats"'),
        check_anchor(collector, "raw_air_superiority_inputs_domain", r'RAW_AIR_SUPERIORITY_INPUTS_DOMAIN:\s*&str\s*=\s*"air_superiority_inputs"'),
        check_anchor(collector, "raw_stockpile_variant_defs_domain", r'RAW_STOCKPILE_VARIANT_DEFS_DOMAIN:\s*&str\s*=\s*"stockpile_variant_defs"'),
        check_anchor(collector, "domain_44_supply_owned", r"COMPACT_DOMAIN_SUPPLY_LOGISTICS_ATTRITION_LIVE:\s*u16\s*=\s*44"),
        check_anchor(collector, "domain_45_frd_owned", r"COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE:\s*u16\s*=\s*45"),
        check_anchor(collector, "air_variant_trackers", r"air_variant_stats:\s*CompactAirVariantStatsTracker"),
        check_anchor(collector, "air_superiority_tracker", r"air_superiority_inputs:\s*CompactAirSuperiorityInputTracker"),
        check_anchor(collector, "air_variant_stats_worker", r"read_compact_air_variant_stats\(state,\s*status_flags,\s*air_variant_stats_tracker\)"),
        check_anchor(collector, "air_superiority_worker", r"read_compact_air_superiority_inputs\("),
        check_anchor(collector, "air_variant_stats_append", r"append_compact_air_variant_stats_data_record\("),
        check_anchor(collector, "air_superiority_append", r"append_compact_air_superiority_inputs_data_record\("),
    ]
    recorder_anchors = [
        check_anchor(recorder, "decode_dispatch_domain_44_supply", r"44\s*=>\s*decode_compact_supply_logistics_attrition_data_payload"),
        check_anchor(recorder, "domain_45_frd_const", r"COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE:\s*u16\s*=\s*45"),
        check_anchor(recorder, "decode_air_variant_defs", r"fn\s+decode_compact_air_variant_defs_data_payload\("),
        check_anchor(recorder, "decode_air_variant_stats", r"fn\s+decode_compact_air_variant_stats_data_payload\("),
        check_anchor(recorder, "decode_air_superiority_inputs", r"fn\s+decode_compact_air_superiority_inputs_data_payload\("),
        map_anchor(
            "domain_id_44_supply",
            recorder_ids.get("supply_logistics_attrition_live") == 44,
            "supply_logistics_attrition_live -> 44",
        ),
        map_anchor(
            "domain_name_44_supply",
            recorder_names.get(44) == "supply_logistics_attrition_live",
            '44 -> "supply_logistics_attrition_live"',
        ),
        map_anchor(
            "domain_id_45_frd",
            recorder_ids.get("focus_research_decisions_depth_live") == 45,
            "focus_research_decisions_depth_live -> 45",
        ),
        map_anchor(
            "domain_name_45_frd",
            recorder_names.get(45) == "focus_research_decisions_depth_live",
            '45 -> "focus_research_decisions_depth_live"',
        ),
    ]
    schema_anchors = [
        check_anchor(schema, "lane_schema_assigned_domain_id", r"DOMAIN_ID:\s*u16\s*=\s*46"),
        check_anchor(schema, "source_live_token_dictionary_bit", r"SOURCE_LIVE_TOKEN_DICTIONARY:\s*u32\s*=\s*1\s*<<\s*5"),
        check_anchor(schema, "source_static_source_enum_bit", r"SOURCE_STATIC_SOURCE_ENUM:\s*u32\s*=\s*1\s*<<\s*6"),
    ]

    rejected_id_owners = {
        str(domain_id): {
            "collector": [name for name, value in collector_ids.items() if value == domain_id],
            "recorder_domain_id": [name for name, value in recorder_ids.items() if value == domain_id],
            "recorder_domain_name": recorder_names.get(domain_id),
        }
        for domain_id in REJECTED_PROVISIONAL_DOMAIN_IDS
    }
    assigned_id_shared_registrations = {
        "collector": [name for name, value in collector_ids.items() if value == ASSIGNED_DOMAIN_ID],
        "recorder_domain_id": [name for name, value in recorder_ids.items() if value == ASSIGNED_DOMAIN_ID],
        "recorder_domain_name": recorder_names.get(ASSIGNED_DOMAIN_ID),
    }
    registry_domain = next(
        (
            row
            for row in registry.get("domains", [])
            if row.get("id") == ASSIGNED_DOMAIN_ID and row.get("name") == DOMAIN_NAME
        ),
        None,
    )
    all_anchors = collector_anchors + recorder_anchors + schema_anchors
    anchors_ok = all(anchor["ok"] for anchor in all_anchors)
    old_ids_rejected = all(
        owner["collector"] or owner["recorder_domain_id"] or owner["recorder_domain_name"]
        for owner in rejected_id_owners.values()
    )
    assigned_id_in_registry = registry_domain is not None

    return {
        "schema": "air_wings_variants_lifecycle_live_registry_registration_gate_v1",
        "generated_utc": dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat(),
        "completion_status": "NOT_COMPLETE",
        "backlog_reduced": True,
        "exact_backlog_item_removed_or_narrowed": (
            "Rejected provisional compact domain id 44 for air_variant_archetypes_live because current shared "
            "collector/recorder reserve it for supply_logistics_attrition_live; rejected stale proposed id 45 "
            "because focus_research_decisions_depth_live now owns it; verified registry-assigned final domain "
            "id 46 for air_variant_archetypes_live."
        ),
        "collector_path": str(COLLECTOR),
        "recorder_path": str(RECORDER),
        "recorder_compact_registry_path": str(RECORDER_COMPACT_REGISTRY),
        "lane_schema_path": str(LANE_SCHEMA),
        "domain_name": DOMAIN_NAME,
        "rejected_provisional_domain_ids": list(REJECTED_PROVISIONAL_DOMAIN_IDS),
        "rejected_provisional_domain_owners": rejected_id_owners,
        "assigned_domain_id": ASSIGNED_DOMAIN_ID,
        "assigned_domain_registry_row": registry_domain,
        "assigned_domain_id_in_registry": assigned_id_in_registry,
        "assigned_domain_id_shared_registrations": assigned_id_shared_registrations,
        "occupied_domain_ids": occupied_ids,
        "anchors_ok": anchors_ok,
        "registration_plan_ready": anchors_ok and old_ids_rejected and assigned_id_in_registry,
        "collector_anchors": collector_anchors,
        "recorder_anchors": recorder_anchors,
        "lane_schema_anchors": schema_anchors,
        "shared_files_modified": False,
        "compact_schema_record_changes": (
            "lane-local schema uses registry-assigned domain id 46; source-mask bits unchanged."
        ),
        "rust_decoder_gate_changes": "lane-local decoder only; shared recorder remains untouched.",
        "dll_only": False,
        "compact_stream": False,
        "rust_decoded": False,
        "hoi4replay_verified": False,
        "speed_five_current_equivalent_hourly_or_lower": False,
        "zero_skipped_hours": False,
        "partial_source_worker_bridge_failures": False,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out-dir", required=True)
    args = parser.parse_args()
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    gate = build_gate()
    out_path = out_dir / "slot_integration_anchor_gate.json"
    out_path.write_text(json.dumps(gate, indent=2, sort_keys=True), encoding="utf-8")
    print(json.dumps({
        "out": str(out_path),
        "completion_status": gate["completion_status"],
        "backlog_reduced": gate["backlog_reduced"],
        "registration_plan_ready": gate["registration_plan_ready"],
        "rejected_provisional_domain_ids": gate["rejected_provisional_domain_ids"],
        "assigned_domain_id": gate["assigned_domain_id"],
        "assigned_domain_id_in_registry": gate["assigned_domain_id_in_registry"],
        "anchors_ok": gate["anchors_ok"],
    }, indent=2, sort_keys=True))
    return 0 if gate["registration_plan_ready"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
