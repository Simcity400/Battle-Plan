#!/usr/bin/env python3
"""the game save -> exact raid-status and raid-instance oracle.

The top-level ``raids.countries`` carrier is an ordered anonymous list.  This
parser deliberately preserves both parent and child producer order; it does
not collapse by tag or raid id.  That makes the output suitable for B7 paused
acceptance of the live ``CCountryRaidStatus`` / ``CRaidInstance`` walk.

stdlib only.  Reuses the shared quote-aware Clausewitz tokenizer.
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    EQUALS,
    LBRACE,
    RBRACE,
    atom_value,
    iter_block_entries,
    read_id_block,
    to_int,
    _skip_block_body,
)


def _parse_raid_source(tz):
    out = {"type": None, "province": None, "tag": None}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        value = atom_value(opener)
        if value is None:
            continue
        if key == "type":
            out["type"] = value
        elif key == "province":
            out["province"] = to_int(value)
        elif key == "tag":
            out["tag"] = value
    return out


def _parse_raid_instance(tz, status_index, raid_index, owner_tag):
    out = {
        "status_index": status_index,
        "raid_index": raid_index,
        "owner_tag": owner_tag,
        "id": None,
        "id_type": None,
        "prep_time": None,
        "phase": None,
        "outcome": None,
        "raid_source": None,
        "end_date": None,
        "victim_country": None,
    }
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "id":
                out["id"], out["id_type"] = read_id_block(tz)
            elif key == "raid_source":
                out["raid_source"] = _parse_raid_source(tz)
            else:
                _skip_block_body(tz)
            continue
        value = atom_value(opener)
        if value is None:
            continue
        if key == "prep_time":
            out["prep_time"] = to_int(value)
        elif key == "phase":
            out["phase"] = value
        elif key == "outcome":
            out["outcome"] = value
        elif key == "end_date":
            out["end_date"] = value
        elif key == "victim_country":
            out["victim_country"] = value
    return out


def _parse_raid_instance_list(tz, status_index, owner_tag):
    rows = []
    while True:
        token = tz.next()
        if token is None or token is RBRACE:
            return rows
        if token is LBRACE:
            rows.append(
                _parse_raid_instance(tz, status_index, len(rows), owner_tag)
            )
            continue
        # Be tolerant of a named entry while preserving synchronization.
        key = atom_value(token)
        if key is None or tz.peek() is not EQUALS:
            continue
        tz.next()
        value = tz.next()
        if value is LBRACE:
            _skip_block_body(tz)


def _parse_status(tz, status_index):
    status = {
        "status_index": status_index,
        "tag": None,
        "classification": None,
        "priority": None,
        "dummy_id": None,
        "dummy_id_type": None,
        "raids": [],
    }
    # Raid rows need the tag, but the save writes tag before raid_instance on
    # current builds.  If a future build reverses that order the final owner
    # fix-up below still keeps the result correct.
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "dummy":
                for child, child_opener in iter_block_entries(tz):
                    if child == "id" and child_opener is LBRACE:
                        status["dummy_id"], status["dummy_id_type"] = read_id_block(tz)
                    elif child_opener is LBRACE:
                        _skip_block_body(tz)
            elif key == "raid_instance":
                status["raids"] = _parse_raid_instance_list(
                    tz, status_index, status["tag"]
                )
            else:
                _skip_block_body(tz)
            continue
        value = atom_value(opener)
        if value is None:
            continue
        if key == "tag":
            status["tag"] = value
        elif key == "priority":
            status["priority"] = to_int(value)

    if status["tag"] is None:
        status["classification"] = "missing_tag_dummy"
    elif status["tag"] == "---":
        status["classification"] = "null_tag_dummy"
    else:
        status["classification"] = "country"
    for raid in status["raids"]:
        raid["owner_tag"] = status["tag"]
    return status


def _parse_status_list(tz, statuses, raids):
    while True:
        token = tz.next()
        if token is None or token is RBRACE:
            return
        if token is LBRACE:
            status = _parse_status(tz, len(statuses))
            statuses.append(status)
            raids.extend(status["raids"])
            continue
        key = atom_value(token)
        if key is None or tz.peek() is not EQUALS:
            continue
        tz.next()
        value = tz.next()
        if value is LBRACE:
            _skip_block_body(tz)


def _parse_raids_root(tz, statuses, raids):
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key == "countries":
            _parse_status_list(tz, statuses, raids)
        else:
            _skip_block_body(tz)


def parse_raids_text(text: str):
    statuses = []
    raids = []
    meta = dc.top_level_pass(
        text,
        {"raids": lambda tz: _parse_raids_root(tz, statuses, raids)},
        scalar_keys=("date", "version", "player"),
    )
    return statuses, raids, meta


def build(text, source, was_melted):
    statuses, raids, meta = parse_raids_text(text)
    classifications = {}
    for status in statuses:
        kind = status["classification"]
        classifications[kind] = classifications.get(kind, 0) + 1
    phase_counts = {}
    outcome_counts = {}
    owner_counts = {}
    raid_id_counts = {}
    for raid in raids:
        phase = raid["phase"]
        outcome = raid["outcome"]
        owner = raid["owner_tag"]
        phase_counts[phase] = phase_counts.get(phase, 0) + 1
        outcome_counts[outcome] = outcome_counts.get(outcome, 0) + 1
        owner_counts[owner] = owner_counts.get(owner, 0) + 1
        raid_id = raid["id"]
        if raid_id is not None:
            raid_id_counts[raid_id] = raid_id_counts.get(raid_id, 0) + 1
    duplicate_raid_ids = sorted(
        raid_id for raid_id, count in raid_id_counts.items() if count > 1
    )
    out = {
        "meta": {
            "source_filename": source.name,
            "source_path_redacted": dc.ps._redact(source),
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "stats": {
                "country_statuses": len(statuses),
                "raid_instances": len(raids),
                "classifications": classifications,
                "phase_counts": phase_counts,
                "outcome_counts": outcome_counts,
                "owner_counts": owner_counts,
                "raid_instances_with_id": sum(
                    raid["id"] is not None for raid in raids
                ),
                "unique_raid_ids": len(raid_id_counts),
                "duplicate_raid_ids": duplicate_raid_ids,
            },
        },
        "country_statuses": statuses,
        "raid_instances": raids,
    }
    lines = ["=== RAID ORACLE VALIDATION ==="]
    lines.append(f"source          : {source.name} (melted={was_melted})")
    lines.append(f"save date       : {meta.get('date')}")
    lines.append(f"country statuses: {len(statuses)} {classifications}")
    lines.append(f"raid instances  : {len(raids)}")
    lines.append(f"phase counts    : {phase_counts}")
    lines.append(f"outcome counts  : {outcome_counts}")
    lines.append(
        "raid ids        : "
        f"{len(raid_id_counts)} unique, {len(duplicate_raid_ids)} duplicated"
    )
    # `parse_strategic.py` owns the replay-consumed `raids.json` contract.
    # Keep the B7 parent/child acceptance oracle in a distinct artifact so the
    # aggregate save-oracle runner can never overwrite either schema.
    return {"raid_instances.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(
        dc.run_cli(
            "raids",
            "the game save -> exact ordered raid status/instance oracle.",
            build,
            ["raid_instances.json"],
        )
    )
