#!/usr/bin/env python3
"""Mine d29/d49/d50 lifecycle transitions from a retained compact stream.

The proof is deliberately semantic-conservative: a domain-50 tombstone means
"member no longer belongs to this combat generation". It is never called a
sinking. Domain-29 object deletion is reported separately because deletion
does not encode the cause of deletion.
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import json
import struct
from pathlib import Path

import zstandard


H4CR_HEADER_LEN = 32
DOMAIN_SHIPS = 29
DOMAIN_COMBAT = 49
DOMAIN_MEMBERS = 50
DOMAIN_DATA = 3
SHIP_DELETED = 1 << 0
COMBAT_DELETED = 1 << 0
MEMBER_DELETED = 1 << 0
STATUS_FAILURE_MASK = (
    (1 << 0)
    | (1 << 1)
    | (1 << 2)
    | (1 << 3)
    | (1 << 6)
    | (1 << 7)
    | (1 << 8)
    | (1 << 9)
    | (1 << 12)
)


def u16(data: bytes | memoryview, offset: int) -> int:
    return struct.unpack_from("<H", data, offset)[0]


def u32(data: bytes | memoryview, offset: int) -> int:
    return struct.unpack_from("<I", data, offset)[0]


def i32(data: bytes | memoryview, offset: int) -> int:
    return struct.unpack_from("<i", data, offset)[0]


def u64(data: bytes | memoryview, offset: int) -> int:
    return struct.unpack_from("<Q", data, offset)[0]


def load_stream(path: Path) -> tuple[bytes, str]:
    stored = path.read_bytes()
    if stored[:4] == b"H4CR":
        return stored, "raw"
    with zstandard.ZstdDecompressor().stream_reader(memoryview(stored)) as reader:
        decoded = reader.read()
    if decoded[:4] != b"H4CR":
        raise ValueError("decoded stream does not begin with H4CR")
    return decoded, "zstd"


def records_by_hour(data: bytes) -> tuple[dict[int, dict[int, memoryview]], int]:
    selected: dict[int, dict[int, memoryview]] = collections.defaultdict(dict)
    position = 0
    total = 0
    while position < len(data):
        if data[position : position + 4] != b"H4CR":
            raise ValueError(f"bad compact magic at byte {position}")
        if u16(data, position + 4) != 1 or u16(data, position + 6) != H4CR_HEADER_LEN:
            raise ValueError(f"unsupported compact header at byte {position}")
        payload_len = u32(data, position + 24)
        end = position + H4CR_HEADER_LEN + payload_len
        if end > len(data):
            raise ValueError(f"record at byte {position} overruns stream")
        game_hour = i32(data, position + 16)
        domain = u16(data, position + 20)
        record_type = u16(data, position + 22)
        if record_type == DOMAIN_DATA and domain in (DOMAIN_SHIPS, DOMAIN_COMBAT, DOMAIN_MEMBERS):
            if domain in selected[game_hour]:
                raise ValueError(f"duplicate domain {domain} DATA record at hour {game_hour}")
            selected[game_hour][domain] = memoryview(data)[position + 32 : end]
        total += 1
        position = end
    if position != len(data):
        raise ValueError("stream has trailing bytes")
    return dict(selected), total


def ship_rows(payload: memoryview):
    count = u32(payload, 8)
    cursor = 40
    for _ in range(count):
        if cursor + 42 > len(payload):
            raise ValueError("truncated domain-29 row")
        ship_id = u32(payload, cursor)
        flags = u32(payload, cursor + 4)
        cursor += 40
        text_len = u16(payload, cursor)
        cursor += 2 + text_len
        if cursor > len(payload):
            raise ValueError("truncated domain-29 definition string")
        yield ship_id, flags
    if cursor != len(payload):
        raise ValueError("domain-29 payload has trailing bytes")


def lifecycle_rows(payload: memoryview):
    count = u32(payload, 16)
    cursor = 28
    for _ in range(count):
        if cursor + 72 > len(payload):
            raise ValueError("truncated domain-49 row")
        yield (u32(payload, cursor), u64(payload, cursor + 4)), u32(payload, cursor + 20)
        cursor += 72


def member_rows(payload: memoryview):
    count = u32(payload, 16)
    cursor = 28
    for _ in range(count):
        if cursor + 88 > len(payload):
            raise ValueError("truncated domain-50 row")
        parent = (u32(payload, cursor), u64(payload, cursor + 4))
        key = parent + (
            u32(payload, cursor + 16),
            u32(payload, cursor + 20),
            u32(payload, cursor + 24),
        )
        yield key, u32(payload, cursor + 12), u32(payload, cursor + 32)
        cursor += 88


def payload_status(domain: int, payload: memoryview) -> int:
    """Return the domain DATA status flags from its versioned header."""
    if domain == DOMAIN_SHIPS:
        if len(payload) < 40:
            raise ValueError("truncated domain-29 header")
        return u32(payload, 16)
    if domain in (DOMAIN_COMBAT, DOMAIN_MEMBERS):
        if len(payload) < 28:
            raise ValueError(f"truncated domain-{domain} header")
        return u32(payload, 12)
    raise ValueError(f"no status decoder for domain {domain}")


def analyze(data: bytes) -> dict[str, object]:
    hours, total_records = records_by_hour(data)
    active_combats: set[tuple[int, int]] = set()
    active_members: dict[tuple[int, int, int, int, int], int] = {}
    last_combat_hour_by_ship: dict[int, int] = {}
    combat_removal_hour_by_ship: dict[int, int] = {}
    counters: collections.Counter[str] = collections.Counter()
    status_counts: dict[int, collections.Counter[int]] = {
        domain: collections.Counter() for domain in (DOMAIN_SHIPS, DOMAIN_COMBAT, DOMAIN_MEMBERS)
    }
    deleted_ships: list[dict[str, int | None]] = []

    for game_hour in sorted(hours):
        domains = hours[game_hour]
        missing = {DOMAIN_SHIPS, DOMAIN_COMBAT, DOMAIN_MEMBERS} - set(domains)
        if missing:
            raise ValueError(f"hour {game_hour} is missing required domains {sorted(missing)}")
        statuses = {
            domain: payload_status(domain, domains[domain])
            for domain in (DOMAIN_SHIPS, DOMAIN_COMBAT, DOMAIN_MEMBERS)
        }
        for domain, status in statuses.items():
            status_counts[domain][status] += 1
        naval_pair_admissible = not (
            (statuses[DOMAIN_COMBAT] | statuses[DOMAIN_MEMBERS]) & STATUS_FAILURE_MASK
        )
        if naval_pair_admissible:
            counters["naval_pair_admitted_frames"] += 1
        else:
            counters["naval_pair_masked_frames"] += 1
        ship_deletes = {
            ship_id
            for ship_id, flags in ship_rows(domains[DOMAIN_SHIPS])
            if flags & SHIP_DELETED
        }
        started: set[tuple[int, int]] = set()
        ended: set[tuple[int, int]] = set()

        lifecycle = lifecycle_rows(domains[DOMAIN_COMBAT]) if naval_pair_admissible else ()
        for combat, flags in lifecycle:
            if flags & COMBAT_DELETED:
                counters["combat_ends"] += 1
                ended.add(combat)
                active_combats.discard(combat)
            elif combat in active_combats:
                counters["combat_updates"] += 1
            else:
                counters["combat_starts"] += 1
                started.add(combat)
                active_combats.add(combat)

        members = member_rows(domains[DOMAIN_MEMBERS]) if naval_pair_admissible else ()
        for key, flags, row_ship_id in members:
            parent = key[:2]
            if flags & MEMBER_DELETED:
                counters["member_removals"] += 1
                old_ship_id = active_members.pop(key, 0)
                ship_id = row_ship_id or old_ship_id
                if ship_id:
                    combat_removal_hour_by_ship[ship_id] = game_hour
                if parent in ended:
                    counters["member_removals_at_combat_end"] += 1
                else:
                    counters["member_removals_while_combat_survives"] += 1
                if ship_id and ship_id in ship_deletes:
                    counters["member_removals_with_same_frame_ship_delete"] += 1
            else:
                is_join = key not in active_members
                active_members[key] = row_ship_id
                if row_ship_id:
                    last_combat_hour_by_ship[row_ship_id] = game_hour
                if is_join:
                    counters["member_joins"] += 1
                    if parent in started:
                        counters["member_joins_at_combat_start"] += 1
                    else:
                        counters["member_joins_after_combat_start"] += 1

        parentless_active = sum(key[:2] not in active_combats for key in active_members)
        counters["max_parentless_active_members"] = max(
            counters["max_parentless_active_members"], parentless_active
        )
        if parentless_active:
            counters["frames_with_parentless_active_members"] += 1

        for ship_id in sorted(ship_deletes):
            last_hour = last_combat_hour_by_ship.get(ship_id)
            removal_hour = combat_removal_hour_by_ship.get(ship_id)
            still_active = any(value == ship_id for value in active_members.values())
            deleted_ships.append(
                {
                    "ship_instance_id": ship_id,
                    "delete_game_hour": game_hour,
                    "last_combat_member_game_hour": last_hour,
                    "hours_after_last_combat_observation": (
                        None if last_hour is None else game_hour - last_hour
                    ),
                    "combat_membership_removal_game_hour": removal_hour,
                    "hours_after_combat_membership_removal": (
                        None if removal_hour is None else game_hour - removal_hour
                    ),
                    "still_active_combat_member_at_delete": still_active,
                }
            )

    return {
        "h4cr_record_count": total_records,
        "selected_hour_count": len(hours),
        "selected_domain_data_records": sum(len(v) for v in hours.values()),
        "status_flags_by_domain": {
            str(domain): {str(status): count for status, count in sorted(counts.items())}
            for domain, counts in status_counts.items()
        },
        "naval_pair_admission": {
            "admitted_frames": counters["naval_pair_admitted_frames"],
            "masked_failure_frames": counters["naval_pair_masked_frames"],
            "failure_mask": STATUS_FAILURE_MASK,
            "rule": "d49 and d50 transitions are decoded only when neither DATA status intersects the compact failure mask",
        },
        "lifecycle": {
            "combat_starts": counters["combat_starts"],
            "combat_updates": counters["combat_updates"],
            "combat_ends": counters["combat_ends"],
            "active_combats_at_stream_end": len(active_combats),
        },
        "membership": {
            "member_joins": counters["member_joins"],
            "member_joins_at_combat_start": counters["member_joins_at_combat_start"],
            "member_joins_after_combat_start": counters["member_joins_after_combat_start"],
            "member_removals": counters["member_removals"],
            "member_removals_at_combat_end": counters["member_removals_at_combat_end"],
            "member_removals_while_combat_survives": counters[
                "member_removals_while_combat_survives"
            ],
            "member_removals_with_same_frame_ship_delete": counters[
                "member_removals_with_same_frame_ship_delete"
            ],
            "active_members_at_stream_end": len(active_members),
            "parentless_active_members_at_stream_end": sum(
                key[:2] not in active_combats for key in active_members
            ),
            "frames_with_parentless_active_members": counters[
                "frames_with_parentless_active_members"
            ],
            "max_parentless_active_members": counters["max_parentless_active_members"],
        },
        "domain29_ship_deletions": deleted_ships,
        "domain29_ship_deletion_summary": {
            "total": len(deleted_ships),
            "while_last_certified_membership_active": sum(
                bool(row["still_active_combat_member_at_delete"])
                for row in deleted_ships
            ),
            "after_observed_membership_removal": sum(
                row["combat_membership_removal_game_hour"] is not None
                and not bool(row["still_active_combat_member_at_delete"])
                for row in deleted_ships
            ),
            "without_captured_combat_membership": sum(
                row["last_combat_member_game_hour"] is None
                for row in deleted_ships
            ),
        },
        "conclusions": {
            "combat_start_update_end": "PROVEN_RETAINED_STREAM",
            "member_join_after_start": "PROVEN_RETAINED_STREAM",
            "member_leave_while_parent_survives": (
                "PROVEN_RETAINED_STREAM"
                if counters["member_removals_while_combat_survives"]
                else "NOT_OBSERVED"
            ),
            "member_removal_means_sinking": "REJECTED_UNSAFE_INFERENCE",
            "domain29_delete_cause": "NOT_ENCODED",
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("stream", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    stored = args.stream.read_bytes()
    decoded, compression = load_stream(args.stream)
    result = {
        "schema": "naval_combat_retained_lifecycle_proof_v1",
        "source_stream": str(args.stream).replace("\\", "/"),
        "stream_storage": compression,
        "stored_bytes": len(stored),
        "stored_sha256": hashlib.sha256(stored).hexdigest(),
        "decoded_bytes": len(decoded),
        "decoded_sha256": hashlib.sha256(decoded).hexdigest(),
        **analyze(decoded),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
