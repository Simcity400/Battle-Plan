"""Lane-local proof for live air variant archetype closure.

This script intentionally consumes an existing DLL compact stream instead of
reading saves as a runtime source. Its job is to verify the live air compact
records needed for variant closure and to reject the stale save-oracle shortcut
of joining current live variant ids directly to saved production archetype ids.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import struct
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


RECORD_HEADER = struct.Struct("<4sHHQiHHII")
RECORD_MAGIC = b"H4CR"
RECORD_HEADER_LEN = 32
RECORD_VERSION = 1
RECORD_TYPE_DOMAIN_DATA = 3

DOMAIN_AIR_VARIANT_DEFS = 33
DOMAIN_AIR_VARIANT_STATS = 35
DOMAIN_STOCKPILE_VARIANT_DEFS = 37
DOMAIN_AIR_SUPERIORITY_INPUTS = 38

VARIANT_DEF_HEADER_LEN = 44
VARIANT_DEF_ROW = struct.Struct("<IIIIII")
VARIANT_STATS_HEADER_LEN = 36
VARIANT_STATS_ROW = struct.Struct("<IIIIIiiiiiiiiiiI")
SUPERIORITY_HEADER_LEN = 52
SUPERIORITY_ROW = struct.Struct("<IIIIIIIIIIq")

ROW_FLAG_DELETED = 1

STATIC_AIR_NAME_MARKERS = (
    "airframe",
    "plane",
    "fighter",
    "bomber",
    "cas",
    "scout_plane",
)
STATIC_AIR_MISSION_MARKERS = {
    "air_superiority",
    "interception",
    "cas",
    "strategic_bomber",
    "naval_bomber",
    "port_strike",
    "attack_logistics",
    "naval_mines_planting",
    "naval_mines_sweeping",
    "recon",
}


@dataclass
class VariantDef:
    variant_id: int
    domain_id: int
    display_name: str
    variant_name: str
    modules: tuple[str, ...]


@dataclass
class VariantStats:
    variant_id: int
    flags: int
    equipment_token_raw: int
    equipment_type_token_raw: int
    interface_type_raw: int
    speed_cache_raw: int
    range_raw: int
    air_attack_raw: int
    agility_raw: int
    max_speed_raw: int
    reliability_raw: int
    air_defence_raw: int
    thrust_raw: int
    fuel_consumption_raw: int
    build_cost_ic_raw: int


@dataclass
class SuperiorityRow:
    region_id: int
    owner_tag_db_id: int
    mission_mask: int
    variant_id: int
    flags: int
    equipment_token_raw: int
    equipment_type_token_raw: int
    interface_type_raw: int
    wing_count: int
    amount_raw: int

    @property
    def deleted(self) -> bool:
        return bool(self.flags & ROW_FLAG_DELETED)


def read_u32(payload: bytes, offset: int) -> int:
    return struct.unpack_from("<I", payload, offset)[0]


def read_compact_string(payload: bytes, cursor: int) -> tuple[str, int]:
    if len(payload) - cursor < 2:
        raise ValueError("truncated compact string length")
    size = struct.unpack_from("<H", payload, cursor)[0]
    cursor += 2
    if len(payload) - cursor < size:
        raise ValueError(f"compact string length {size} exceeds remaining payload")
    value = payload[cursor : cursor + size].decode("utf-8", errors="replace")
    return value, cursor + size


def iter_compact_records(blob: bytes) -> tuple[list[tuple[int, int, int, int, bytes]], list[str]]:
    records: list[tuple[int, int, int, int, bytes]] = []
    errors: list[str] = []
    cursor = 0
    previous_sequence: int | None = None
    while cursor < len(blob):
        if len(blob) - cursor < RECORD_HEADER_LEN:
            errors.append(f"offset {cursor}: truncated compact record header")
            break
        (
            magic,
            version,
            header_len,
            sequence,
            game_hour_raw,
            domain_id,
            record_type,
            payload_len,
            _record_flags,
        ) = RECORD_HEADER.unpack_from(blob, cursor)
        if magic != RECORD_MAGIC:
            errors.append(f"offset {cursor}: bad compact magic {magic!r}")
            break
        if version != RECORD_VERSION:
            errors.append(f"offset {cursor}: unsupported compact version {version}")
            break
        if header_len != RECORD_HEADER_LEN:
            errors.append(f"offset {cursor}: unsupported compact header length {header_len}")
            break
        if previous_sequence is not None and sequence <= previous_sequence:
            errors.append(
                f"offset {cursor}: nonmonotonic compact sequence {sequence} <= {previous_sequence}"
            )
        previous_sequence = sequence
        payload_start = cursor + RECORD_HEADER_LEN
        payload_end = payload_start + payload_len
        if payload_end > len(blob):
            errors.append(f"offset {cursor}: payload length {payload_len} exceeds stream")
            break
        records.append(
            (cursor, game_hour_raw, domain_id, record_type, blob[payload_start:payload_end])
        )
        cursor = payload_end
    return records, errors


def decode_variant_defs(payload: bytes, domain_id: int) -> tuple[dict[int, VariantDef], list[str]]:
    errors: list[str] = []
    if len(payload) < VARIANT_DEF_HEADER_LEN:
        return {}, [f"variant defs payload too short: {len(payload)}"]
    row_count = read_u32(payload, 8)
    emitted_module_rows = read_u32(payload, 28)
    rows_end = VARIANT_DEF_HEADER_LEN + row_count * VARIANT_DEF_ROW.size
    if len(payload) < rows_end:
        return {}, [f"variant defs rows truncated: {len(payload)} < {rows_end}"]

    cursor = VARIANT_DEF_HEADER_LEN
    row_headers: list[tuple[int, int, int, int]] = []
    for row_index in range(row_count):
        variant_id, flags, module_first, module_count, reserved0, reserved1 = (
            VARIANT_DEF_ROW.unpack_from(payload, cursor)
        )
        cursor += VARIANT_DEF_ROW.size
        if reserved0 or reserved1:
            errors.append(f"variant defs row {row_index}: reserved fields are nonzero")
        row_headers.append((variant_id, flags, module_first, module_count))

    names: list[tuple[str, str]] = []
    for row_index in range(row_count):
        try:
            display_name, cursor = read_compact_string(payload, cursor)
            variant_name, cursor = read_compact_string(payload, cursor)
        except ValueError as exc:
            return {}, [*errors, f"variant defs row {row_index}: {exc}"]
        names.append((display_name, variant_name))

    module_tokens: list[str] = []
    for module_index in range(emitted_module_rows):
        if len(payload) - cursor < 8:
            return {}, [*errors, f"variant defs module row {module_index}: truncated fixed fields"]
        _slot_handle, reserved = struct.unpack_from("<II", payload, cursor)
        cursor += 8
        if reserved:
            errors.append(f"variant defs module row {module_index}: reserved field is nonzero")
        try:
            token, cursor = read_compact_string(payload, cursor)
        except ValueError as exc:
            return {}, [*errors, f"variant defs module row {module_index}: {exc}"]
        module_tokens.append(token)

    if cursor != len(payload):
        errors.append(f"variant defs payload has {len(payload) - cursor} trailing bytes")

    out: dict[int, VariantDef] = {}
    for row_index, (variant_id, _flags, module_first, module_count) in enumerate(row_headers):
        module_end = module_first + module_count
        if module_end > len(module_tokens):
            errors.append(
                f"variant defs row {row_index}: module range {module_first}+{module_count} exceeds {len(module_tokens)}"
            )
            modules = ()
        else:
            modules = tuple(module_tokens[module_first:module_end])
        display_name, variant_name = names[row_index]
        out[variant_id] = VariantDef(
            variant_id=variant_id,
            domain_id=domain_id,
            display_name=display_name,
            variant_name=variant_name,
            modules=modules,
        )
    return out, errors


def decode_variant_stats(payload: bytes) -> tuple[dict[int, VariantStats], list[str]]:
    if len(payload) < VARIANT_STATS_HEADER_LEN:
        return {}, [f"variant stats payload too short: {len(payload)}"]
    row_count = read_u32(payload, 8)
    expected_len = VARIANT_STATS_HEADER_LEN + row_count * VARIANT_STATS_ROW.size
    if len(payload) != expected_len:
        return {}, [f"variant stats payload length {len(payload)} != expected {expected_len}"]
    cursor = VARIANT_STATS_HEADER_LEN
    out: dict[int, VariantStats] = {}
    for _row_index in range(row_count):
        fields = VARIANT_STATS_ROW.unpack_from(payload, cursor)
        cursor += VARIANT_STATS_ROW.size
        out[fields[0]] = VariantStats(*fields[:-1])
    return out, []


def decode_superiority_inputs(payload: bytes) -> tuple[list[SuperiorityRow], list[str]]:
    if len(payload) < SUPERIORITY_HEADER_LEN:
        return [], [f"air superiority payload too short: {len(payload)}"]
    row_count = read_u32(payload, 8)
    expected_len = SUPERIORITY_HEADER_LEN + row_count * SUPERIORITY_ROW.size
    if len(payload) != expected_len:
        return [], [f"air superiority payload length {len(payload)} != expected {expected_len}"]
    cursor = SUPERIORITY_HEADER_LEN
    rows: list[SuperiorityRow] = []
    for _row_index in range(row_count):
        (
            region_id,
            owner_tag_db_id,
            mission_mask,
            variant_id,
            flags,
            equipment_token_raw,
            equipment_type_token_raw,
            interface_type_raw,
            wing_count,
            _reserved,
            amount_raw,
        ) = SUPERIORITY_ROW.unpack_from(payload, cursor)
        cursor += SUPERIORITY_ROW.size
        rows.append(
            SuperiorityRow(
                region_id=region_id,
                owner_tag_db_id=owner_tag_db_id,
                mission_mask=mission_mask,
                variant_id=variant_id,
                flags=flags,
                equipment_token_raw=equipment_token_raw,
                equipment_type_token_raw=equipment_type_token_raw,
                interface_type_raw=interface_type_raw,
                wing_count=wing_count,
                amount_raw=amount_raw,
            )
        )
    return rows, []


def load_live_stream(stream_path: Path) -> dict[str, Any]:
    records, stream_errors = iter_compact_records(stream_path.read_bytes())
    air_defs: dict[int, VariantDef] = {}
    stockpile_defs: dict[int, VariantDef] = {}
    stats: dict[int, VariantStats] = {}
    superiority_rows: list[SuperiorityRow] = []
    decode_errors = list(stream_errors)
    domain_data_counts: dict[int, int] = {}
    frame_hours: set[int] = set()

    for offset, game_hour_raw, domain_id, record_type, payload in records:
        if record_type != RECORD_TYPE_DOMAIN_DATA:
            continue
        domain_data_counts[domain_id] = domain_data_counts.get(domain_id, 0) + 1
        frame_hours.add(game_hour_raw)
        if domain_id in (DOMAIN_AIR_VARIANT_DEFS, DOMAIN_STOCKPILE_VARIANT_DEFS):
            decoded, errors = decode_variant_defs(payload, domain_id)
            for error in errors:
                decode_errors.append(f"offset {offset} domain {domain_id}: {error}")
            if domain_id == DOMAIN_AIR_VARIANT_DEFS:
                air_defs.update(decoded)
            else:
                stockpile_defs.update(decoded)
        elif domain_id == DOMAIN_AIR_VARIANT_STATS:
            decoded, errors = decode_variant_stats(payload)
            for error in errors:
                decode_errors.append(f"offset {offset} domain {domain_id}: {error}")
            stats.update(decoded)
        elif domain_id == DOMAIN_AIR_SUPERIORITY_INPUTS:
            decoded, errors = decode_superiority_inputs(payload)
            for error in errors:
                decode_errors.append(f"offset {offset} domain {domain_id}: {error}")
            superiority_rows.extend(decoded)

    combined_defs = dict(stockpile_defs)
    combined_defs.update(air_defs)
    active_superiority_variant_ids = sorted(
        {row.variant_id for row in superiority_rows if not row.deleted}
    )
    return {
        "decode_errors": decode_errors,
        "domain_data_counts": dict(sorted(domain_data_counts.items())),
        "unique_game_hour_count": len(frame_hours),
        "air_defs": air_defs,
        "stockpile_defs": stockpile_defs,
        "combined_defs": combined_defs,
        "stats": stats,
        "superiority_rows": superiority_rows,
        "active_superiority_variant_ids": active_superiority_variant_ids,
    }


def load_catalog(catalog_path: Path) -> tuple[dict[str, Any], str]:
    raw = catalog_path.read_bytes()
    digest = hashlib.sha256(raw).hexdigest()
    data = json.loads(raw.decode("utf-8"))
    return data.get("catalogs", {}).get("equipment_archetypes", {}), digest


def is_static_air_equipment(name: str, record: Any) -> bool:
    lowered_name = name.lower()
    if "anti_air" in lowered_name:
        return False
    if any(marker in lowered_name for marker in STATIC_AIR_NAME_MARKERS):
        return True
    if not isinstance(record, dict):
        return False
    source_file = str(record.get("source_file") or "").replace("\\", "/").lower()
    archetype = str(record.get("archetype") or "").lower()
    if "anti_air" in archetype:
        return False
    if "common/units/equipment/x_plane" in source_file:
        return True
    if any(marker in archetype for marker in STATIC_AIR_NAME_MARKERS):
        return True
    stats = record.get("stats")
    if isinstance(stats, dict):
        missions = stats.get("allow_mission_type")
        if isinstance(missions, str):
            mission_values = {missions}
        elif isinstance(missions, list):
            mission_values = {str(item) for item in missions}
        else:
            mission_values = set()
        if mission_values & STATIC_AIR_MISSION_MARKERS:
            return True
        if "air_superiority" in stats:
            return True
    return False


def load_save_archetype_rows(path: Path) -> dict[int, list[tuple[str, dict[str, Any]]]]:
    data = json.loads(path.read_text(encoding="utf-8"))
    rows: dict[int, list[tuple[str, dict[str, Any]]]] = {}
    for key, value in data.get("equipment_archetypes", {}).items():
        if not isinstance(value, dict):
            continue
        variant_text = key.split(":", 1)[0]
        if not variant_text.isdigit():
            continue
        rows.setdefault(int(variant_text), []).append((key, value))
    return rows


def live_variant_example(
    variant_id: int,
    combined_defs: dict[int, VariantDef],
    stats: dict[int, VariantStats],
    rows: list[SuperiorityRow],
) -> dict[str, Any]:
    definition = combined_defs.get(variant_id)
    stat = stats.get(variant_id)
    row = next((candidate for candidate in rows if candidate.variant_id == variant_id), None)
    return {
        "variant_id": variant_id,
        "display_name": definition.display_name if definition else None,
        "variant_name": definition.variant_name if definition else None,
        "module_tokens": list(definition.modules[:8]) if definition else [],
        "stats": {
            "equipment_token_raw": stat.equipment_token_raw if stat else None,
            "equipment_type_token_raw": stat.equipment_type_token_raw if stat else None,
            "interface_type_raw": stat.interface_type_raw if stat else None,
            "air_attack_raw": stat.air_attack_raw if stat else None,
            "agility_raw": stat.agility_raw if stat else None,
            "air_defence_raw": stat.air_defence_raw if stat else None,
            "build_cost_ic_raw": stat.build_cost_ic_raw if stat else None,
        },
        "superiority_input": {
            "region_id": row.region_id if row else None,
            "owner_tag_db_id": row.owner_tag_db_id if row else None,
            "mission_mask": row.mission_mask if row else None,
            "wing_count": row.wing_count if row else None,
            "amount_raw": row.amount_raw if row else None,
        },
    }


def reject_save_variant_id_joins(
    save_paths: list[Path],
    active_variant_ids: list[int],
    combined_defs: dict[int, VariantDef],
    stats: dict[int, VariantStats],
    superiority_rows: list[SuperiorityRow],
    equipment_catalog: dict[str, Any],
) -> list[dict[str, Any]]:
    rejections: list[dict[str, Any]] = []
    for save_path in save_paths:
        if not save_path.exists():
            rejections.append(
                {
                    "save_production_path": str(save_path),
                    "status": "not_checked_missing_file",
                    "rejected": False,
                }
            )
            continue
        save_rows = load_save_archetype_rows(save_path)
        examples: list[dict[str, Any]] = []
        matched_live_active_ids = 0
        contradicted_live_active_ids = 0
        for variant_id in active_variant_ids:
            matches = save_rows.get(variant_id, [])
            if not matches:
                continue
            matched_live_active_ids += 1
            preferred = [item for item in matches if item[0].endswith(":70")] or matches
            key, save_record = preferred[0]
            save_name = str(save_record.get("name") or "")
            static_record = equipment_catalog.get(save_name)
            save_name_is_air = is_static_air_equipment(save_name, static_record)
            if save_name_is_air:
                continue
            contradicted_live_active_ids += 1
            if len(examples) < 8:
                example = live_variant_example(
                    variant_id, combined_defs, stats, superiority_rows
                )
                example.update(
                    {
                        "save_key": key,
                        "save_name": save_name,
                        "save_static_catalog_class": "non_air_or_unknown",
                    }
                )
                examples.append(example)
        rejections.append(
            {
                "save_production_path": str(save_path),
                "status": "rejected" if examples else "no_contradiction_found",
                "rejected": bool(examples),
                "active_live_variant_ids_with_save_id_match": matched_live_active_ids,
                "active_live_variant_ids_contradicted_by_save_name": contradicted_live_active_ids,
                "reason": (
                    "Save production equipment_archetypes variant ids are save/session-local; "
                    "joining them directly to the current live compact variant ids maps live air "
                    "variants to non-air equipment names."
                ),
                "examples": examples,
            }
        )
    return rejections


def summarize_manifest(manifest_path: Path | None) -> dict[str, Any]:
    if not manifest_path:
        return {}
    data = json.loads(manifest_path.read_text(encoding="utf-8"))
    compact = data.get("compact_stream", {})
    replay_file = data.get("replay_file")
    replay_exists = False
    if isinstance(replay_file, str):
        replay_path = Path(replay_file)
        if not replay_path.is_absolute():
            replay_path = manifest_path.parent / replay_path
        replay_exists = replay_path.exists()
    return {
        "manifest_path": str(manifest_path),
        "game_hours_advanced": data.get("game_hours_advanced"),
        "unique_game_hour_count": data.get("unique_game_hour_count"),
        "skipped_game_hours": data.get("skipped_game_hours"),
        "duration_s": data.get("duration_s"),
        "sample_no_sleep": data.get("sample_no_sleep"),
        "replay_file": replay_file,
        "replay_file_exists": replay_exists,
        "compact_stream": {
            "decodable": compact.get("decodable"),
            "ordered": compact.get("ordered"),
            "domain_coverage_ok": compact.get("domain_coverage", {}).get("ok"),
            "domain_data_record_count": compact.get("domain_data_record_count"),
            "domain_perf_record_count": compact.get("domain_perf_record_count"),
            "source_unavailable_seen": compact.get("source_unavailable_seen"),
            "partial_data_seen": compact.get("partial_data_seen"),
            "bridge_unavailable_seen": compact.get("bridge_unavailable_seen"),
            "worker_panic_seen": compact.get("worker_panic_seen"),
        },
    }


def build_proof(args: argparse.Namespace) -> dict[str, Any]:
    stream_path = Path(args.stream)
    catalog_path = Path(args.catalogs)
    manifest_path = Path(args.manifest) if args.manifest else None
    save_paths = [Path(path) for path in args.save_production]

    stream = load_live_stream(stream_path)
    equipment_catalog, catalog_sha256 = load_catalog(catalog_path)
    active_variant_ids = stream["active_superiority_variant_ids"]
    combined_defs = stream["combined_defs"]
    stats = stream["stats"]
    superiority_rows = stream["superiority_rows"]
    save_join_rejections = reject_save_variant_id_joins(
        save_paths,
        active_variant_ids,
        combined_defs,
        stats,
        superiority_rows,
        equipment_catalog,
    )
    rejected_save_paths = [row for row in save_join_rejections if row.get("rejected")]
    active_with_defs = [variant_id for variant_id in active_variant_ids if variant_id in combined_defs]
    active_with_stats = [variant_id for variant_id in active_variant_ids if variant_id in stats]
    active_with_names_or_modules = [
        variant_id
        for variant_id in active_variant_ids
        if (
            variant_id in combined_defs
            and (
                combined_defs[variant_id].display_name
                or combined_defs[variant_id].variant_name
                or combined_defs[variant_id].modules
            )
        )
    ]
    unresolved_active = [
        variant_id
        for variant_id in active_variant_ids
        if variant_id not in combined_defs
        or (
            not combined_defs[variant_id].display_name
            and not combined_defs[variant_id].variant_name
            and not combined_defs[variant_id].modules
        )
    ]
    static_air_catalog_names = [
        name
        for name, record in equipment_catalog.items()
        if is_static_air_equipment(str(name), record)
    ]

    problems: list[str] = []
    if stream["decode_errors"]:
        problems.append("compact_stream_decode_errors")
    if not active_variant_ids:
        problems.append("no_active_air_superiority_variants")
    if len(active_with_defs) != len(active_variant_ids):
        problems.append("active_variants_missing_live_defs")
    if len(active_with_stats) != len(active_variant_ids):
        problems.append("active_variants_missing_live_stats")
    if not rejected_save_paths:
        problems.append("no_stale_save_join_rejection")

    proof = {
        "schema": "air_wings_variants_lifecycle_live_variant_archetype_join_proof_v1",
        "utc": datetime.now(timezone.utc).isoformat(),
        "stream_path": str(stream_path),
        "catalog_path": str(catalog_path),
        "catalog_sha256": catalog_sha256,
        "manifest": summarize_manifest(manifest_path),
        "lane_scope": {
            "lane": "air_wings_variants_lifecycle_live",
            "does_not_cover": "air_combat_losses_live loss/reinforcement deltas",
            "shared_entrypoints_touched": False,
        },
        "live_compact_domains": {
            "decoded_domain_data_counts": stream["domain_data_counts"],
            "unique_game_hour_count_from_stream": stream["unique_game_hour_count"],
            "air_variant_defs_unique_ids": len(stream["air_defs"]),
            "stockpile_variant_defs_unique_ids": len(stream["stockpile_defs"]),
            "combined_variant_defs_unique_ids": len(combined_defs),
            "air_variant_stats_unique_ids": len(stats),
            "air_superiority_input_rows_total": len(superiority_rows),
            "active_superiority_variant_count": len(active_variant_ids),
            "active_superiority_variants_with_live_defs": len(active_with_defs),
            "active_superiority_variants_with_live_stats": len(active_with_stats),
            "active_superiority_variants_with_names_or_modules": len(active_with_names_or_modules),
            "unresolved_active_variant_ids": unresolved_active,
        },
        "static_catalog": {
            "equipment_archetype_count": len(equipment_catalog),
            "air_equipment_name_count_by_static_rules": len(static_air_catalog_names),
            "air_equipment_name_examples": static_air_catalog_names[:12],
        },
        "candidate_rejections": {
            "save_variant_id_direct_join": save_join_rejections,
            "production_impact": (
                "The current live stream must use live compact variant definitions/stats plus "
                "a same-session equipment-token/name root or bounded static catalog validation; "
                "old save production variant ids cannot be used as production archetype closure."
            ),
        },
        "lane_local_compact_schema": {
            "registry_domain_id": 46,
            "registry_domain_name": "air_variant_archetypes_live",
            "wire_domain_name": "air_variant_archetypes",
            "rust_module": "extractor/native/lanes/air_wings_variants_lifecycle_live/src/domain46_air_variant_archetypes_live.rs",
            "integration_status": "validated slice only; shared entrypoints are orchestrator-owned registration hooks",
        },
        "gate": {
            "ok": not problems,
            "problems": problems,
            "decode_error_count": len(stream["decode_errors"]),
            "decode_errors_sample": stream["decode_errors"][:12],
            "rejected_save_join_paths": [row["save_production_path"] for row in rejected_save_paths],
        },
    }
    return proof


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--stream", required=True)
    parser.add_argument("--catalogs", required=True)
    parser.add_argument("--manifest")
    parser.add_argument("--save-production", action="append", default=[])
    parser.add_argument("--out-json")
    args = parser.parse_args()
    proof = build_proof(args)
    if args.out_json:
        out_path = Path(args.out_json)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(json.dumps(proof, indent=2, sort_keys=True), encoding="utf-8")
    print(json.dumps(proof["gate"], indent=2, sort_keys=True))
    return 0 if proof["gate"]["ok"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
