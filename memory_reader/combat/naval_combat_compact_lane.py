"""Lane-local compact schema and gate for live naval combat extraction.

This module intentionally does not wire into shared collector or recorder
entrypoints. It mirrors the existing compact stream envelope (``H4CR`` records)
and defines merge-ready payloads for the naval combat lifecycle/member route:

    CNavalCombat -> CNavalCombatant -> registry -> CFEXMember
"""

from __future__ import annotations

from dataclasses import dataclass, field
import json
import struct
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Tuple

MAGIC = b"H4CR"
VERSION = 1
HEADER_LEN = 32
RECORD_TYPE_DOMAIN_PERF = 1
RECORD_TYPE_FRAME_END = 2
RECORD_TYPE_DOMAIN_DATA = 3

DOMAIN_NAVAL_COMBAT_LIFECYCLE = 49
DOMAIN_NAVAL_COMBAT_MEMBERS = 50
DOMAIN_NAMES = {
    DOMAIN_NAVAL_COMBAT_LIFECYCLE: "naval_combat_lifecycle_live",
    DOMAIN_NAVAL_COMBAT_MEMBERS: "naval_combat_members_live",
}

SCHEMA_VERSION = 1
COMBAT_ROW_STRUCT = struct.Struct("<IQQIIIIiiIQQQ")
MEMBER_ROW_STRUCT = struct.Struct("<IQIIIIIIIIIIiIQI")
PAYLOAD_HEADER_STRUCT = struct.Struct("<HHQIIII")
RECORD_HEADER_STRUCT = struct.Struct("<4sHHQiHHII")

STATUS_SOURCE_UNAVAILABLE = 1 << 8
STATUS_PARTIAL_DATA = 1 << 9
STATUS_DELTA_DATA = 1 << 10

COMBAT_ROW_DELETED = 1 << 0
COMBAT_LOCATION_PRESENT = 1 << 1
COMBAT_ATTACKER_SHIP_COUNT_PRESENT = 1 << 2
COMBAT_DEFENDER_SHIP_COUNT_PRESENT = 1 << 3
COMBAT_ATTACKER_POSITIONING_PRESENT = 1 << 4
COMBAT_DEFENDER_POSITIONING_PRESENT = 1 << 5

MEMBER_ROW_DELETED = 1 << 0
MEMBER_SHIP_INSTANCE_PRESENT = 1 << 1
MEMBER_OWNER_PRESENT = 1 << 2
MEMBER_DESIGNED_MAX_STRENGTH_PRESENT = 1 << 3
MEMBER_STATE_BITMASK_PRESENT = 1 << 4
MEMBER_ENGAGEMENT_WEIGHT_PRESENT = 1 << 5
MEMBER_CSHIP_JOIN_VALIDATED = 1 << 6
MEMBER_IS_CONVOY = 1 << 7

SIDE_ATTACKER = 1
SIDE_DEFENDER = 2
SIDE_NAMES = {
    SIDE_ATTACKER: "attacker",
    SIDE_DEFENDER: "defender",
}

MAX_COMBAT_ID = 0x7FFF_FFFF
MAX_SHIP_INSTANCE_ID = 4_000_000
MAX_OWNER_TAG_DB_ID = 2_048
MAX_GROUP_INDEX = 64
MAX_MEMBER_INDEX = 1_000
MAX_BATTLE_SLOT_INDEX = 4_096
MAX_LOCATION_ID = 1_000_000
MAX_DESIGNED_STRENGTH_RAW = 1_000_000_000
CSHIP_TYPE_TAG = 51
MAX_STRING_BYTES = 512
MAX_STRINGS = 4096
MAX_ROWS = 100_000


class CompactDecodeError(ValueError):
    pass


@dataclass(frozen=True)
class BridgeRow:
    combat_id: int
    body_ptr: int


@dataclass(frozen=True)
class NavalCombatLifecycleRow:
    combat_id: int
    generation: int
    body_ptr: int
    deleted: bool = False
    location: Optional[int] = None
    attacker_tags: Optional[Tuple[str, ...]] = None
    defender_tags: Optional[Tuple[str, ...]] = None
    attacker_ship_count: Optional[int] = None
    defender_ship_count: Optional[int] = None
    attacker_commander: Optional[str] = None
    defender_commander: Optional[str] = None
    attacker_positioning: Optional[float] = None
    defender_positioning: Optional[float] = None
    attacker_status: str = "active"
    defender_status: str = "active"


@dataclass(frozen=True)
class NavalCombatMemberRow:
    combat_id: int
    generation: int
    side: str
    group_index: int
    member_index: int
    unit_type_tag: int
    ship_instance_id: Optional[int] = None
    cship_join_validated: bool = False
    is_convoy: bool = False
    owner_tag_db_id: Optional[int] = None
    owner: Optional[str] = None
    name: Optional[str] = None
    type_display: Optional[str] = None
    archetype: Optional[str] = None
    designed_max_strength_raw: Optional[int] = None
    battle_slot_index_raw: int = 0
    state_bitmask: Optional[int] = None
    engagement_weight_raw: Optional[int] = None
    deleted: bool = False


@dataclass
class CompactRecord:
    sequence: int
    game_hour_raw: int
    domain_id: int
    record_type: int
    payload: bytes
    offset: int = 0


@dataclass
class DecodedPayload:
    schema_version: int
    elapsed_wall_ms: int
    status_flags: int
    strings: List[str]
    combat_rows: List[NavalCombatLifecycleRow] = field(default_factory=list)
    member_rows: List[NavalCombatMemberRow] = field(default_factory=list)


@dataclass
class GateResult:
    ok: bool
    errors: List[str]
    summary: Dict[str, object]


class _StringTable:
    def __init__(self) -> None:
        self.values = [""]
        self.index = {"": 0}

    def put_optional(self, value: Optional[str]) -> int:
        if value is None:
            return 0
        if not isinstance(value, str):
            raise TypeError("string table value must be str or None")
        encoded = value.encode("utf-8")
        if len(encoded) > MAX_STRING_BYTES:
            raise ValueError("string table value exceeds MAX_STRING_BYTES")
        if value in self.index:
            return self.index[value]
        if len(self.values) >= MAX_STRINGS:
            raise ValueError("string table exceeds MAX_STRINGS")
        idx = len(self.values)
        self.values.append(value)
        self.index[value] = idx
        return idx

    def put_tags(self, tags: Optional[Tuple[str, ...]]) -> int:
        if tags is None:
            return 0
        return self.put_optional(",".join(tags))


def parse_bridge_rows(text: str) -> List[BridgeRow]:
    rows: List[BridgeRow] = []
    seen = set()
    for part in text.strip().split(";"):
        part = part.strip()
        if not part:
            continue
        if ":" not in part:
            raise ValueError("bridge row missing ':'")
        combat_text, ptr_text = part.split(":", 1)
        combat_id = int(combat_text, 10)
        body_ptr = int(ptr_text, 0)
        _require_u32(combat_id, "combat_id")
        if combat_id == 0 or combat_id > MAX_COMBAT_ID:
            raise ValueError("combat_id out of range")
        if body_ptr < 0x10000000000 or body_ptr >= 0x7FF000000000 or body_ptr % 8 != 0:
            raise ValueError("body_ptr is not a valid heap-aligned bridge pointer")
        if combat_id in seen:
            raise ValueError("duplicate combat_id in bridge rows")
        seen.add(combat_id)
        rows.append(BridgeRow(combat_id=combat_id, body_ptr=body_ptr))
    return rows


def encode_records(
    *,
    sequence_start: int,
    game_hour_raw: int,
    elapsed_wall_ms: int,
    combat_rows: Iterable[NavalCombatLifecycleRow],
    member_rows: Iterable[NavalCombatMemberRow],
    status_flags: int = STATUS_DELTA_DATA,
) -> bytes:
    sequence = sequence_start
    out = bytearray()
    combat_payload = encode_combat_payload(
        list(combat_rows),
        elapsed_wall_ms=elapsed_wall_ms,
        status_flags=status_flags,
    )
    out.extend(encode_record(
        sequence=sequence,
        game_hour_raw=game_hour_raw,
        domain_id=DOMAIN_NAVAL_COMBAT_LIFECYCLE,
        record_type=RECORD_TYPE_DOMAIN_DATA,
        payload=combat_payload,
    ))
    sequence += 1
    member_payload = encode_member_payload(
        list(member_rows),
        elapsed_wall_ms=elapsed_wall_ms,
        status_flags=status_flags,
    )
    out.extend(encode_record(
        sequence=sequence,
        game_hour_raw=game_hour_raw,
        domain_id=DOMAIN_NAVAL_COMBAT_MEMBERS,
        record_type=RECORD_TYPE_DOMAIN_DATA,
        payload=member_payload,
    ))
    return bytes(out)


def encode_record(
    *,
    sequence: int,
    game_hour_raw: int,
    domain_id: int,
    record_type: int,
    payload: bytes,
) -> bytes:
    header = RECORD_HEADER_STRUCT.pack(
        MAGIC,
        VERSION,
        HEADER_LEN,
        sequence,
        game_hour_raw,
        domain_id,
        record_type,
        len(payload),
        0,
    )
    return header + payload


def encode_combat_payload(
    rows: List[NavalCombatLifecycleRow],
    *,
    elapsed_wall_ms: int,
    status_flags: int,
) -> bytes:
    if len(rows) > MAX_ROWS:
        raise ValueError("too many combat rows")
    encoded_rows = bytearray()
    for row in rows:
        _validate_combat_row(row)
        flags = 0
        if row.deleted:
            flags |= COMBAT_ROW_DELETED
        flags |= _present(row.location, COMBAT_LOCATION_PRESENT)
        flags |= _present(row.attacker_ship_count, COMBAT_ATTACKER_SHIP_COUNT_PRESENT)
        flags |= _present(row.defender_ship_count, COMBAT_DEFENDER_SHIP_COUNT_PRESENT)
        flags |= _present(row.attacker_positioning, COMBAT_ATTACKER_POSITIONING_PRESENT)
        flags |= _present(row.defender_positioning, COMBAT_DEFENDER_POSITIONING_PRESENT)
        encoded_rows.extend(COMBAT_ROW_STRUCT.pack(
            row.combat_id,
            row.generation,
            row.body_ptr,
            flags,
            row.location or 0,
            row.attacker_ship_count or 0,
            row.defender_ship_count or 0,
            _milli(row.attacker_positioning),
            _milli(row.defender_positioning),
            0,
            0,
            0,
            0,
        ))
    return _payload_header(
        row_len=COMBAT_ROW_STRUCT.size,
        elapsed_wall_ms=elapsed_wall_ms,
        status_flags=status_flags,
        row_count=len(rows),
        string_count=1,
    ) + bytes(encoded_rows) + _encode_string_table([""])


def encode_member_payload(
    rows: List[NavalCombatMemberRow],
    *,
    elapsed_wall_ms: int,
    status_flags: int,
) -> bytes:
    if len(rows) > MAX_ROWS:
        raise ValueError("too many member rows")
    encoded_rows = bytearray()
    for row in rows:
        _validate_member_row(row)
        side = _side_id(row.side)
        flags = 0
        if row.deleted:
            flags |= MEMBER_ROW_DELETED
        if row.is_convoy:
            flags |= MEMBER_IS_CONVOY
        if row.cship_join_validated:
            flags |= MEMBER_CSHIP_JOIN_VALIDATED
        flags |= _present(row.ship_instance_id, MEMBER_SHIP_INSTANCE_PRESENT)
        flags |= _present(row.owner_tag_db_id, MEMBER_OWNER_PRESENT)
        flags |= _present(row.designed_max_strength_raw, MEMBER_DESIGNED_MAX_STRENGTH_PRESENT)
        flags |= _present(row.state_bitmask, MEMBER_STATE_BITMASK_PRESENT)
        flags |= _present(row.engagement_weight_raw, MEMBER_ENGAGEMENT_WEIGHT_PRESENT)
        encoded_rows.extend(MEMBER_ROW_STRUCT.pack(
            row.combat_id,
            row.generation,
            flags,
            side,
            row.group_index,
            row.member_index,
            row.unit_type_tag,
            row.ship_instance_id or 0,
            row.owner_tag_db_id or 0,
            row.designed_max_strength_raw or 0,
            row.battle_slot_index_raw,
            row.state_bitmask or 0,
            row.engagement_weight_raw or 0,
            0,
            0,
            0,
        ))
    return _payload_header(
        row_len=MEMBER_ROW_STRUCT.size,
        elapsed_wall_ms=elapsed_wall_ms,
        status_flags=status_flags,
        row_count=len(rows),
        string_count=1,
    ) + bytes(encoded_rows) + _encode_string_table([""])


def decode_stream(data: bytes) -> List[CompactRecord]:
    records: List[CompactRecord] = []
    cursor = 0
    while cursor < len(data):
        if len(data) - cursor < HEADER_LEN:
            raise CompactDecodeError("trailing bytes shorter than compact header")
        magic, version, header_len, sequence, game_hour_raw, domain_id, record_type, payload_len, reserved = (
            RECORD_HEADER_STRUCT.unpack_from(data, cursor)
        )
        if magic != MAGIC:
            raise CompactDecodeError("bad compact magic at offset %d" % cursor)
        if version != VERSION:
            raise CompactDecodeError("bad compact version at offset %d" % cursor)
        if header_len != HEADER_LEN:
            raise CompactDecodeError("bad compact header length at offset %d" % cursor)
        if reserved != 0:
            raise CompactDecodeError("nonzero compact header reserved at offset %d" % cursor)
        payload_start = cursor + HEADER_LEN
        payload_end = payload_start + payload_len
        if payload_end > len(data):
            raise CompactDecodeError("payload overruns stream at offset %d" % cursor)
        records.append(CompactRecord(
            sequence=sequence,
            game_hour_raw=game_hour_raw,
            domain_id=domain_id,
            record_type=record_type,
            payload=data[payload_start:payload_end],
            offset=cursor,
        ))
        cursor = payload_end
    return records


def decode_payload(record: CompactRecord) -> DecodedPayload:
    if record.record_type != RECORD_TYPE_DOMAIN_DATA:
        raise CompactDecodeError("unsupported naval lane record type")
    if record.domain_id == DOMAIN_NAVAL_COMBAT_LIFECYCLE:
        return decode_combat_payload(record.payload)
    if record.domain_id == DOMAIN_NAVAL_COMBAT_MEMBERS:
        return decode_member_payload(record.payload)
    raise CompactDecodeError("unsupported naval lane domain_id %d" % record.domain_id)


def decode_combat_payload(payload: bytes) -> DecodedPayload:
    version, row_len, elapsed, flags, row_count, string_count, cursor = _decode_payload_header(
        payload, COMBAT_ROW_STRUCT.size
    )
    rows: List[Tuple[int, ...]] = []
    for _ in range(row_count):
        if cursor + COMBAT_ROW_STRUCT.size > len(payload):
            raise CompactDecodeError("combat row overruns payload")
        rows.append(COMBAT_ROW_STRUCT.unpack_from(payload, cursor))
        cursor += COMBAT_ROW_STRUCT.size
    strings = _decode_string_table(payload, cursor, string_count)
    decoded = []
    for raw in rows:
        (
            combat_id, generation, body_ptr, row_flags, location, attacker_ship_count,
            defender_ship_count, attacker_positioning_milli, defender_positioning_milli,
            reserved, reserved_q1, reserved_q2, reserved_q3,
        ) = raw
        if reserved != 0 or reserved_q1 != 0 or reserved_q2 != 0 or reserved_q3 != 0:
            raise CompactDecodeError("combat row reserved field is nonzero")
        decoded.append(NavalCombatLifecycleRow(
            combat_id=combat_id,
            generation=generation,
            body_ptr=body_ptr,
            deleted=bool(row_flags & COMBAT_ROW_DELETED),
            location=location if row_flags & COMBAT_LOCATION_PRESENT else None,
            attacker_ship_count=attacker_ship_count if row_flags & COMBAT_ATTACKER_SHIP_COUNT_PRESENT else None,
            defender_ship_count=defender_ship_count if row_flags & COMBAT_DEFENDER_SHIP_COUNT_PRESENT else None,
            attacker_positioning=_from_milli(attacker_positioning_milli) if row_flags & COMBAT_ATTACKER_POSITIONING_PRESENT else None,
            defender_positioning=_from_milli(defender_positioning_milli) if row_flags & COMBAT_DEFENDER_POSITIONING_PRESENT else None,
        ))
    return DecodedPayload(version, elapsed, flags, strings, combat_rows=decoded)


def decode_member_payload(payload: bytes) -> DecodedPayload:
    version, row_len, elapsed, flags, row_count, string_count, cursor = _decode_payload_header(
        payload, MEMBER_ROW_STRUCT.size
    )
    rows: List[Tuple[int, ...]] = []
    for _ in range(row_count):
        if cursor + MEMBER_ROW_STRUCT.size > len(payload):
            raise CompactDecodeError("member row overruns payload")
        rows.append(MEMBER_ROW_STRUCT.unpack_from(payload, cursor))
        cursor += MEMBER_ROW_STRUCT.size
    strings = _decode_string_table(payload, cursor, string_count)
    decoded = []
    for raw in rows:
        (
            combat_id, generation, row_flags, side_id, group_index, member_index,
            unit_type_tag, ship_instance_id, owner_tag_db_id, designed_max_strength_raw,
            battle_slot_index_raw, state_bitmask, engagement_weight_raw, reserved,
            reserved_q1, reserved_i1,
        ) = raw
        if reserved != 0 or reserved_q1 != 0 or reserved_i1 != 0:
            raise CompactDecodeError("member row reserved field is nonzero")
        decoded.append(NavalCombatMemberRow(
            combat_id=combat_id,
            generation=generation,
            side=SIDE_NAMES.get(side_id, "unknown"),
            group_index=group_index,
            member_index=member_index,
            unit_type_tag=unit_type_tag,
            ship_instance_id=ship_instance_id if row_flags & MEMBER_SHIP_INSTANCE_PRESENT else None,
            cship_join_validated=bool(row_flags & MEMBER_CSHIP_JOIN_VALIDATED),
            is_convoy=bool(row_flags & MEMBER_IS_CONVOY),
            owner_tag_db_id=owner_tag_db_id if row_flags & MEMBER_OWNER_PRESENT else None,
            designed_max_strength_raw=designed_max_strength_raw if row_flags & MEMBER_DESIGNED_MAX_STRENGTH_PRESENT else None,
            battle_slot_index_raw=battle_slot_index_raw,
            state_bitmask=state_bitmask if row_flags & MEMBER_STATE_BITMASK_PRESENT else None,
            engagement_weight_raw=engagement_weight_raw if row_flags & MEMBER_ENGAGEMENT_WEIGHT_PRESENT else None,
            deleted=bool(row_flags & MEMBER_ROW_DELETED),
        ))
    return DecodedPayload(version, elapsed, flags, strings, member_rows=decoded)


def gate_stream(data: bytes, *, require_two_sided: bool = True) -> GateResult:
    errors: List[str] = []
    try:
        records = decode_stream(data)
    except CompactDecodeError as exc:
        return GateResult(False, [str(exc)], {"record_count": 0})

    last_sequence = -1
    combat_rows: List[NavalCombatLifecycleRow] = []
    member_rows: List[NavalCombatMemberRow] = []
    domains = []
    status_flags = 0
    for record in records:
        if record.sequence <= last_sequence:
            errors.append("record sequence is not strictly increasing")
        last_sequence = record.sequence
        domains.append(record.domain_id)
        try:
            payload = decode_payload(record)
        except CompactDecodeError as exc:
            errors.append("offset %d: %s" % (record.offset, exc))
            continue
        status_flags |= payload.status_flags
        combat_rows.extend(payload.combat_rows)
        member_rows.extend(payload.member_rows)

    if DOMAIN_NAVAL_COMBAT_LIFECYCLE not in domains:
        errors.append("missing naval combat lifecycle domain")
    if DOMAIN_NAVAL_COMBAT_MEMBERS not in domains:
        errors.append("missing naval combat member domain")
    if status_flags & STATUS_SOURCE_UNAVAILABLE:
        errors.append("source unavailable status flag set")
    if status_flags & STATUS_PARTIAL_DATA:
        errors.append("partial data status flag set")

    active_combat_keys = {
        (row.combat_id, row.generation)
        for row in combat_rows
        if not row.deleted
    }
    if not active_combat_keys:
        errors.append("no active lifecycle rows")
    member_sides: Dict[Tuple[int, int], set] = {}
    cship_validated_member_count = 0
    convoy_member_count = 0
    for member in member_rows:
        if member.deleted:
            continue
        if member.is_convoy:
            convoy_member_count += 1
        if member.cship_join_validated:
            cship_validated_member_count += 1
        key = (member.combat_id, member.generation)
        member_sides.setdefault(key, set()).add(member.side)
    if require_two_sided:
        for key in sorted(active_combat_keys):
            sides = member_sides.get(key, set())
            if not {"attacker", "defender"}.issubset(sides):
                errors.append("combat %s:%s lacks two-sided member rows" % key)
    if member_rows and cship_validated_member_count == 0:
        errors.append("member rows exist but no non-convoy CShip join was validated")

    summary = {
        "record_count": len(records),
        "domains": [DOMAIN_NAMES.get(domain, str(domain)) for domain in domains],
        "combat_row_count": len(combat_rows),
        "member_row_count": len(member_rows),
        "active_combat_count": len(active_combat_keys),
        "two_sided_combat_count": sum(
            1 for key in active_combat_keys
            if {"attacker", "defender"}.issubset(member_sides.get(key, set()))
        ),
        "cship_validated_member_count": cship_validated_member_count,
        "convoy_member_count": convoy_member_count,
        "status_flags": status_flags,
    }
    return GateResult(not errors, errors, summary)


def synthetic_rows_from_bridge(rows: List[BridgeRow]) -> Tuple[List[NavalCombatLifecycleRow], List[NavalCombatMemberRow]]:
    combat_rows = []
    member_rows = []
    for index, bridge in enumerate(rows):
        generation = 1
        combat_rows.append(NavalCombatLifecycleRow(
            combat_id=bridge.combat_id,
            generation=generation,
            body_ptr=bridge.body_ptr,
            location=10_000 + index,
            attacker_tags=("ENG",),
            defender_tags=("ITA",),
            attacker_ship_count=1,
            defender_ship_count=1,
            attacker_positioning=0.75,
            defender_positioning=0.62,
            attacker_status="active",
            defender_status="active",
        ))
        member_rows.extend([
            NavalCombatMemberRow(
                combat_id=bridge.combat_id,
                generation=generation,
                side="attacker",
                group_index=0,
                member_index=0,
                unit_type_tag=51,
                ship_instance_id=100_000 + index,
                cship_join_validated=True,
                owner_tag_db_id=17,
                owner="ENG",
                name="Synthetic Attacker",
                type_display="Destroyer",
                archetype="screen",
                designed_max_strength_raw=100_000,
                battle_slot_index_raw=1,
                state_bitmask=1,
                engagement_weight_raw=10_000,
            ),
            NavalCombatMemberRow(
                combat_id=bridge.combat_id,
                generation=generation,
                side="defender",
                group_index=0,
                member_index=0,
                unit_type_tag=0,
                is_convoy=True,
                owner_tag_db_id=18,
                owner="ITA",
                type_display="Convoy",
                archetype="convoy",
                designed_max_strength_raw=120_000,
                battle_slot_index_raw=2,
                state_bitmask=1,
                engagement_weight_raw=11_000,
            ),
        ])
    return combat_rows, member_rows


def proof_from_bridge_text(
    bridge_text: str,
    *,
    out_dir: Path,
    game_hour_raw: int = 0,
) -> Dict[str, object]:
    rows = parse_bridge_rows(bridge_text)
    if not rows:
        raise ValueError("bridge rows are empty")
    out_dir.mkdir(parents=True, exist_ok=True)
    combat_rows, member_rows = synthetic_rows_from_bridge(rows)
    stream = encode_records(
        sequence_start=1,
        game_hour_raw=game_hour_raw,
        elapsed_wall_ms=1_000,
        combat_rows=combat_rows,
        member_rows=member_rows,
    )
    compact_path = out_dir / "naval_combat_lane_local_compact.bin"
    compact_path.write_bytes(stream)
    gate = gate_stream(stream, require_two_sided=True)
    report = {
        "status": "passed" if gate.ok else "failed",
        "bridge_row_count": len(rows),
        "compact_path": str(compact_path),
        "gate": {
            "ok": gate.ok,
            "errors": gate.errors,
            "summary": gate.summary,
        },
        "schema": schema_manifest(),
    }
    (out_dir / "naval_combat_lane_local_compact_proof.json").write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return report


def schema_manifest() -> Dict[str, object]:
    return {
        "magic": MAGIC.decode("ascii"),
        "version": VERSION,
        "header_len": HEADER_LEN,
        "record_type_domain_data": RECORD_TYPE_DOMAIN_DATA,
        "domains": DOMAIN_NAMES,
        "schema_version": SCHEMA_VERSION,
        "combat_row_len": COMBAT_ROW_STRUCT.size,
        "member_row_len": MEMBER_ROW_STRUCT.size,
        "route": "CNavalCombat -> CNavalCombatant -> registry -> CFEXMember",
        "wire_contract": "Matches replay_native/lanes/naval_combat_live Rust schema: 72-byte lifecycle rows, 72-byte member rows, empty string table, and CShip validation flag bit 6.",
        "merge_plan": [
            "Add COMPACT_DOMAIN_NAVAL_COMBAT_LIFECYCLE_LIVE = 49 and COMPACT_DOMAIN_NAVAL_COMBAT_MEMBERS_LIVE = 50 to collector and recorder domain maps.",
            "Move this payload schema into collector compact builders after the existing raw naval bridge sampler validates body/side/registry/member roots.",
            "Add recorder_host compact decoders for both row types and require one data record per frame hour when scheduled.",
            "Expose a naval-combat bridge file argument only through the orchestrator-approved integration slot.",
        ],
    }


def _payload_header(
    *,
    row_len: int,
    elapsed_wall_ms: int,
    status_flags: int,
    row_count: int,
    string_count: int,
) -> bytes:
    return PAYLOAD_HEADER_STRUCT.pack(
        SCHEMA_VERSION,
        row_len,
        elapsed_wall_ms,
        status_flags,
        row_count,
        string_count,
        0,
    )


def _decode_payload_header(payload: bytes, expected_row_len: int) -> Tuple[int, int, int, int, int, int, int]:
    if len(payload) < PAYLOAD_HEADER_STRUCT.size:
        raise CompactDecodeError("payload shorter than lane header")
    version, row_len, elapsed, flags, row_count, string_count, reserved = PAYLOAD_HEADER_STRUCT.unpack_from(payload, 0)
    if version != SCHEMA_VERSION:
        raise CompactDecodeError("unsupported schema version")
    if row_len != expected_row_len:
        raise CompactDecodeError("unexpected row length")
    if reserved != 0:
        raise CompactDecodeError("payload reserved field is nonzero")
    if row_count > MAX_ROWS:
        raise CompactDecodeError("row count exceeds MAX_ROWS")
    if string_count > MAX_STRINGS:
        raise CompactDecodeError("string count exceeds MAX_STRINGS")
    return version, row_len, elapsed, flags, row_count, string_count, PAYLOAD_HEADER_STRUCT.size


def _encode_string_table(strings: List[str]) -> bytes:
    out = bytearray()
    for value in strings:
        encoded = value.encode("utf-8")
        if len(encoded) > MAX_STRING_BYTES:
            raise ValueError("string table entry exceeds MAX_STRING_BYTES")
        out.extend(struct.pack("<H", len(encoded)))
        out.extend(encoded)
    return bytes(out)


def _decode_string_table(payload: bytes, cursor: int, count: int) -> List[str]:
    strings = []
    for _ in range(count):
        if cursor + 2 > len(payload):
            raise CompactDecodeError("string length overruns payload")
        length = struct.unpack_from("<H", payload, cursor)[0]
        cursor += 2
        if length > MAX_STRING_BYTES or cursor + length > len(payload):
            raise CompactDecodeError("string bytes overrun payload")
        strings.append(payload[cursor:cursor + length].decode("utf-8"))
        cursor += length
    if cursor != len(payload):
        raise CompactDecodeError("trailing bytes after string table")
    if not strings or strings[0] != "":
        raise CompactDecodeError("string table index 0 must be empty")
    return strings


def _str(strings: List[str], index: int) -> str:
    if index >= len(strings):
        raise CompactDecodeError("string index out of range")
    return strings[index]


def _split_tags(value: str) -> Tuple[str, ...]:
    return tuple(part for part in value.split(",") if part)


def _present(value: object, flag: int) -> int:
    return flag if value is not None else 0


def _milli(value: Optional[float]) -> int:
    if value is None:
        return 0
    return int(round(value * 1000.0))


def _from_milli(value: int) -> float:
    return value / 1000.0


def _side_id(value: str) -> int:
    if value == "attacker":
        return SIDE_ATTACKER
    if value == "defender":
        return SIDE_DEFENDER
    raise ValueError("unknown side %r" % value)


def _require_u32(value: int, label: str) -> None:
    if not isinstance(value, int) or value < 0 or value > 0xFFFF_FFFF:
        raise ValueError("%s must be u32" % label)


def _validate_combat_row(row: NavalCombatLifecycleRow) -> None:
    _require_u32(row.combat_id, "combat_id")
    if row.combat_id == 0 or row.combat_id > MAX_COMBAT_ID:
        raise ValueError("combat_id out of range")
    if row.generation < 0 or row.generation > 0xFFFF_FFFF_FFFF_FFFF:
        raise ValueError("generation out of range")
    if row.body_ptr < 0 or row.body_ptr > 0xFFFF_FFFF_FFFF_FFFF:
        raise ValueError("body_ptr out of range")
    if row.location is not None and (row.location == 0 or row.location > MAX_LOCATION_ID):
        raise ValueError("location out of range")


def _validate_member_row(row: NavalCombatMemberRow) -> None:
    _require_u32(row.combat_id, "combat_id")
    if row.combat_id == 0 or row.combat_id > MAX_COMBAT_ID:
        raise ValueError("combat_id out of range")
    if row.side not in ("attacker", "defender"):
        raise ValueError("member side must be attacker or defender")
    for label in ("group_index", "member_index", "unit_type_tag", "battle_slot_index_raw"):
        _require_u32(getattr(row, label), label)
    if row.group_index > MAX_GROUP_INDEX:
        raise ValueError("group_index out of range")
    if row.member_index > MAX_MEMBER_INDEX:
        raise ValueError("member_index out of range")
    if row.battle_slot_index_raw > MAX_BATTLE_SLOT_INDEX:
        raise ValueError("battle_slot_index_raw out of range")
    if row.ship_instance_id is not None:
        _require_u32(row.ship_instance_id, "ship_instance_id")
        if row.ship_instance_id == 0 or row.ship_instance_id > MAX_SHIP_INSTANCE_ID:
            raise ValueError("ship_instance_id out of range")
    if row.owner_tag_db_id is not None:
        _require_u32(row.owner_tag_db_id, "owner_tag_db_id")
        if row.owner_tag_db_id > MAX_OWNER_TAG_DB_ID:
            raise ValueError("owner_tag_db_id out of range")
    if row.designed_max_strength_raw is not None:
        _require_u32(row.designed_max_strength_raw, "designed_max_strength_raw")
        if row.designed_max_strength_raw > MAX_DESIGNED_STRENGTH_RAW:
            raise ValueError("designed_max_strength_raw out of range")
    if row.state_bitmask is not None:
        _require_u32(row.state_bitmask, "state_bitmask")
    if row.engagement_weight_raw is not None:
        if not isinstance(row.engagement_weight_raw, int) or not -(2 ** 31) <= row.engagement_weight_raw < 2 ** 31:
            raise ValueError("engagement_weight_raw must be i32")
    if row.is_convoy and row.cship_join_validated:
        raise ValueError("convoy member cannot carry CShip join validation")
    if row.is_convoy and row.ship_instance_id is not None:
        raise ValueError("convoy member cannot carry ship_instance_id")
    if not row.is_convoy and row.ship_instance_id is not None and not row.cship_join_validated:
        raise ValueError("non-convoy ship member requires CShip join validation")
    if not row.is_convoy and row.ship_instance_id is not None and row.unit_type_tag != CSHIP_TYPE_TAG:
        raise ValueError("non-convoy ship member has unexpected unit_type_tag")
