#!/usr/bin/env python3
"""Materialize one CID1 matrix stream-boundary receipt from durable recorder truth.

The input is the recorder-owned ``logical-stream-set.json`` plus its exact raw
member.  No caller-supplied CID1/CFA1 hash or boundary classification is
accepted.  The complete H4CR stream, CID1 payload, descriptor member, and
rollover transition are independently decoded and joined before an immutable
receipt is written.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import struct
import tempfile
from pathlib import Path


SCHEMA = "battle-plan.campaign_identity_stream_boundary.v2"
SET_SCHEMA = "cid1-rollover-set-v1"
HEX64 = re.compile(r"^[0-9a-f]{64}$")
UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
HEADER = struct.Struct("<4sHHQiHHII")
CID_PREFIX = b"hoi4-campaign-id-v1\0"


class BoundaryError(ValueError):
    pass


def load_json(path: Path, label: str):
    def pairs(values):
        out = {}
        for key, value in values:
            if key in out:
                raise BoundaryError(f"duplicate key in {label}: {key}")
            out[key] = value
        return out

    try:
        return json.loads(path.read_text(encoding="utf-8-sig"), object_pairs_hook=pairs)
    except (OSError, json.JSONDecodeError) as exc:
        raise BoundaryError(f"cannot read {label}: {exc}") from exc


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def require_hex(value, label: str) -> str:
    if not isinstance(value, str) or HEX64.fullmatch(value) is None:
        raise BoundaryError(f"{label} is not lowercase SHA-256")
    return value


def parse_cid1(payload: bytes, record_hour: int) -> dict:
    if len(payload) < 96 or payload[:4] != b"CID1":
        raise BoundaryError("sequence-zero payload is not CID1")
    version, fixed_len = struct.unpack_from("<HH", payload, 4)
    total_len, flags = struct.unpack_from("<II", payload, 8)
    source = struct.unpack_from("<H", payload, 16)[0]
    seed, payload_hour = struct.unpack_from("<ii", payload, 20)
    uuid_len, profile_len, scenario_len, roster_count, roster_len, modset_len = struct.unpack_from("<HHHHHH", payload, 28)
    if version != 1 or fixed_len != 96 or total_len != len(payload) or len(payload) > 964:
        raise BoundaryError("CID1 fixed header or total length is invalid")
    if flags & ~0xFF or flags & 0xE0 != 0xE0 or source != 1:
        raise BoundaryError("CID1 flags/source do not prove two equal live observations")
    if payload[18:20] != b"\0\0" or any(payload[80:96]):
        raise BoundaryError("CID1 reserved bytes are nonzero")
    if payload_hour != record_hour or uuid_len != 36 or not 1 <= profile_len <= 128:
        raise BoundaryError("CID1 hour or primary variable lengths are invalid")
    if scenario_len > 256 or roster_count > 64 or roster_len != roster_count * 3 or modset_len > 256:
        raise BoundaryError("CID1 witness lengths are invalid")
    if 96 + uuid_len + profile_len + scenario_len + roster_len + modset_len != len(payload):
        raise BoundaryError("CID1 variable fields do not exactly consume payload")
    uuid = payload[96:132].decode("ascii", errors="strict")
    if UUID.fullmatch(uuid) is None:
        raise BoundaryError("CID1 UUID is not canonical")
    campaign_id = hashlib.sha256(CID_PREFIX + uuid.encode("ascii") + b"\0" + str(seed).encode("ascii")).digest()
    if payload[48:80] != campaign_id:
        raise BoundaryError("CID1 campaign digest does not recompute")
    return {
        "game_unique_id": uuid,
        "game_unique_seed": seed,
        "game_hour_raw": payload_hour,
        "campaign_id_v1": campaign_id.hex(),
        "payload_sha256": hashlib.sha256(payload).hexdigest(),
    }


def parse_stream(path: Path) -> dict:
    data = path.read_bytes()
    offset = 0
    records = []
    while offset < len(data):
        if len(data) - offset < HEADER.size:
            raise BoundaryError("raw member ends with a truncated H4CR header")
        magic, version, header_len, sequence, hour, domain, record_type, payload_len, flags = HEADER.unpack_from(data, offset)
        if magic != b"H4CR" or version != 1 or header_len != HEADER.size or payload_len > (1 << 28):
            raise BoundaryError(f"invalid H4CR framing at byte {offset}")
        end = offset + HEADER.size + payload_len
        if end > len(data):
            raise BoundaryError(f"truncated H4CR payload at byte {offset}")
        if sequence != len(records):
            raise BoundaryError(f"non-contiguous H4CR sequence {sequence}, expected {len(records)}")
        records.append((sequence, hour, domain, record_type, flags, data[offset + HEADER.size:end]))
        offset = end
    if len(records) < 2:
        raise BoundaryError("logical stream has no post-CID1 world record")
    sequence, hour, domain, record_type, flags, payload = records[0]
    if (sequence, domain, record_type, flags) != (0, 0, 4, 0):
        raise BoundaryError("first H4CR is not placement-valid sequence-zero CID1")
    cid1 = parse_cid1(payload, hour)
    if records[1][0] != 1 or records[1][2] == 0:
        raise BoundaryError("first post-CID1 world record is not sequence one")
    if any(record[2] == 0 for record in records[1:]):
        raise BoundaryError("post-header stream reuses reserved compact domain zero")
    cid1["last_complete_sequence"] = records[-1][0]
    cid1["record_count"] = len(records)
    return cid1


def atomic_json(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        raise BoundaryError(f"refusing to replace immutable receipt: {path}")
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as handle:
            json.dump(value, handle, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def materialize(args) -> dict:
    descriptor_path = args.rollover_set.resolve()
    descriptor = load_json(descriptor_path, "rollover set descriptor")
    if not isinstance(descriptor, dict) or set(descriptor) != {"schema", "ordered_streams", "transitions"} or descriptor["schema"] != SET_SCHEMA:
        raise BoundaryError("rollover set descriptor shape/schema is invalid")
    streams = descriptor["ordered_streams"]
    if not isinstance(streams, list) or not streams:
        raise BoundaryError("rollover set has no ordered streams")
    for index, stream in enumerate(streams):
        if not isinstance(stream, dict) or stream.get("logical_stream_id") != index:
            raise BoundaryError("rollover streams are not ordered exactly 0..N")
    if args.logical_stream_id < 0 or args.logical_stream_id >= len(streams):
        raise BoundaryError("requested logical stream is absent")
    member = streams[args.logical_stream_id]
    member_path = (descriptor_path.parent / member["relative_path"]).resolve()
    if not member_path.is_file() or sha256_file(member_path) != require_hex(member.get("file_sha256_hex"), "member hash"):
        raise BoundaryError("raw member path/hash disagrees with descriptor")
    if member_path.stat().st_size != member.get("file_bytes"):
        raise BoundaryError("raw member byte count disagrees with descriptor")
    parsed = parse_stream(member_path)
    if parsed["last_complete_sequence"] != member.get("last_complete_sequence"):
        raise BoundaryError("raw member frontier disagrees with descriptor")
    if parsed["campaign_id_v1"] != require_hex(member.get("campaign_id_hex"), "member campaign id"):
        raise BoundaryError("raw CID1 campaign id disagrees with descriptor")
    if parsed["payload_sha256"] != require_hex(member.get("cid1_payload_sha256_hex"), "member CID1 hash"):
        raise BoundaryError("raw CID1 payload hash disagrees with descriptor")

    if args.logical_stream_id == 0:
        boundary_kind, previous, cra1 = "initial", "", False
    else:
        matches = [t for t in descriptor["transitions"] if isinstance(t, dict) and t.get("proposed_logical_stream_id") == args.logical_stream_id]
        if len(matches) != 1:
            raise BoundaryError("rollover member has no unique durable transition")
        transition = matches[0]
        if (transition.get("current_logical_stream_id") != args.logical_stream_id - 1
                or transition.get("proposed_campaign_id_hex") != parsed["campaign_id_v1"]
                or transition.get("proposed_cid1_payload_sha256_hex") != parsed["payload_sha256"]
                or transition.get("boundary_game_hour_raw") != parsed["game_hour_raw"]):
            raise BoundaryError("durable rollover transition disagrees with proposed CID1")
        previous = require_hex(transition.get("current_campaign_id_hex"), "previous campaign id")
        boundary_kind, cra1 = "rollover", True

    return {
        "schema": SCHEMA,
        "status": "passed",
        "transcript_sha256": require_hex(args.transcript_sha256, "transcript hash"),
        "campaign_id_v1": parsed["campaign_id_v1"],
        "boundary_kind": boundary_kind,
        "previous_campaign_id_v1": previous,
        "cra1_acknowledged": cra1,
        "logical_stream_id": args.logical_stream_id,
        "cid1_sequence": 0,
        "cid1_domain": 0,
        "cid1_type": 4,
        "cid1_payload_sha256": parsed["payload_sha256"],
        "cfa1_payload_sha256": parsed["payload_sha256"],
        "first_world_sequence": 1,
        "no_uncertain_frames": True,
        "rollover_set_descriptor_path": str(descriptor_path),
        "rollover_set_descriptor_sha256": sha256_file(descriptor_path),
        "raw_member_path": str(member_path),
        "raw_member_sha256": sha256_file(member_path),
        "raw_member_bytes": member_path.stat().st_size,
        "last_complete_sequence": parsed["last_complete_sequence"],
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rollover-set", type=Path, required=True)
    parser.add_argument("--logical-stream-id", type=int, required=True)
    parser.add_argument("--transcript-sha256", required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        atomic_json(args.out.resolve(), materialize(args))
    except (BoundaryError, OSError, UnicodeError, struct.error) as exc:
        print(f"CID1 stream boundary: FAIL: {exc}")
        return 1
    print(args.out.resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
