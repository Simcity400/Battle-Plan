#!/usr/bin/env python3
"""Fail-closed regrade for one A31 selected-Present evidence receipt."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import struct
from pathlib import Path

EXE = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
MANIFEST = "303351ed65468aa7655b904998819c96ca1c325f906b5a3bf32c4f18decc7969"
JOB = "c6c3fb002bc05f1e092d7eb9b436aa720ca2cf19ad08f055747216dfa5871b4f"
FRESH = "510353dae716bccaaf37f988b0f7101e31552ecf687ecbd613bce1dd2efb784a"
SENTINEL = "498d09aa4161bedf4087983b47ee97626c4d78df9a4537026aff2ea1b4671429"
CONTEXT_SENTINEL = "4c85cfe84aee03808798f821e90f8ab5a7e019e990be67ee48b47d34c09d83b4"
FOCUS_ID = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_.@-]{0,127}$")
TAG = re.compile(r"^[A-Z0-9]{3}$")


class Reader:
    def __init__(self, payload: bytes) -> None:
        self.payload = payload
        self.at = 0

    def take(self, count: int) -> bytes:
        end = self.at + count
        if count < 0 or end > len(self.payload):
            raise ValueError("truncated NFP1 payload")
        value = self.payload[self.at:end]
        self.at = end
        return value

    def u8(self) -> int:
        return self.take(1)[0]

    def u16(self) -> int:
        return struct.unpack("<H", self.take(2))[0]

    def u32(self) -> int:
        return struct.unpack("<I", self.take(4))[0]

    def u64(self) -> int:
        return struct.unpack("<Q", self.take(8))[0]

    def text(self) -> str:
        count = self.u32()
        if count > 65_536:
            raise ValueError("NFP1 text exceeds bound")
        return self.take(count).decode("utf-8")

    def opt_text(self) -> str | None:
        present = self.u8()
        if present == 0:
            return None
        if present != 1:
            raise ValueError("NFP1 optional text marker invalid")
        return self.text()


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def parse_uint(value: object, name: str) -> int:
    require(isinstance(value, str) and value.isdigit(), f"{name} is not a decimal string")
    parsed = int(value)
    require(0 <= parsed <= (1 << 64) - 1, f"{name} exceeds u64")
    return parsed


def valid_focus_id(value: str) -> bool:
    return bool(FOCUS_ID.fullmatch(value)) and ".." not in value and not (
        value[:2].lower() == "0x"
        and len(value[2:]) >= 8
        and all(character in "0123456789abcdefABCDEF" for character in value[2:])
    )


def parse_nfp1(payload: bytes) -> dict[str, object]:
    reader = Reader(payload)
    require(reader.u32() == int.from_bytes(b"NFP1", "little"), "NFP1 magic mismatch")
    require(reader.u32() == 1, "NFP1 schema mismatch")
    require(reader.u32() == 1 and reader.u32() == 0, "NFP1 is not exact authority")
    _elapsed = reader.u64()
    frame_hour = reader.u64()
    require(reader.take(32).hex() == EXE, "NFP1 executable hash mismatch")
    require(reader.take(32).hex() == MANIFEST, "NFP1 manifest hash mismatch")
    require(reader.take(32).hex() == JOB, "NFP1 job receipt hash mismatch")
    require(reader.u8() == 1 and reader.take(3) == b"\0\0\0", "NFP1 snapshot marker invalid")
    source_sequence = reader.u64()
    country_tag = reader.text()
    view_id = reader.text()
    record_identity = reader.text()
    expected = reader.u32()
    count = reader.u32()
    require(0 < source_sequence and count <= 131_072 and expected == count, "NFP1 count/sequence invalid")
    require(bool(TAG.fullmatch(country_tag)), "NFP1 country tag invalid")
    require(view_id == f"nationalfocusview:{country_tag}", "NFP1 view identity invalid")
    require(
        record_identity == f"a729:a31:{frame_hour}:{country_tag}:{source_sequence}",
        "NFP1 producer identity invalid",
    )
    seen: set[str] = set()
    for ordinal in range(count):
        require(reader.u32() == ordinal, "NFP1 source ordinal is not contiguous")
        state = reader.u8()
        percent = reader.u8()
        flags = reader.u16()
        focus_id = reader.text()
        joint = reader.opt_text()
        require(state <= 3 and percent <= 100 and flags & ~0x7 == 0, "NFP1 item domain invalid")
        require(valid_focus_id(focus_id) and focus_id not in seen, "NFP1 focus identity invalid")
        seen.add(focus_id)
        require(((flags & 0x4) != 0) == (joint is not None), "NFP1 joint-country flag mismatch")
        require(joint is None or bool(TAG.fullmatch(joint)), "NFP1 joint-country tag invalid")
        require(not (state in (0, 2) and percent != 0), "NFP1 not-started progress invalid")
        require(not (state == 1 and percent != 100), "NFP1 completed progress invalid")
    require(reader.at == len(payload), "NFP1 payload has trailing bytes")
    return {
        "frame_hour": frame_hour,
        "source_sequence": source_sequence,
        "country_tag": country_tag,
        "visible_item_count": count,
    }


def verify(receipt_path: Path) -> dict[str, object]:
    receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
    require(receipt.get("schema") == "hoi4.a31.selected_present_acquisition_receipt.v1", "receipt schema mismatch")
    require(receipt.get("binary_sha256") == EXE, "receipt executable hash mismatch")
    require(receipt.get("exact20_function_manifest_sha256") == MANIFEST, "receipt manifest mismatch")
    require(receipt.get("exact20_job_receipt_sha256") == JOB, "receipt job hash mismatch")
    require(receipt.get("fresh_acquisition_manifest_sha256") == FRESH, "fresh acquisition hash mismatch")
    require(receipt.get("sentinel_dump_sha256") == SENTINEL, "sentinel dump hash mismatch")
    require(receipt.get("country_context_sentinel_dump_sha256") == CONTEXT_SENTINEL, "country-context sentinel hash mismatch")
    require(receipt.get("production_scheduling_enabled") is False, "d89 unexpectedly scheduled")
    require(receipt.get("live_fixture_accepted_in_binary") is False, "fixture latch unexpectedly active")
    body_hash = receipt.pop("receipt_body_sha256", None)
    canonical = json.dumps(receipt, sort_keys=True, separators=(",", ":")).encode("utf-8")
    require(body_hash == sha256(canonical), "receipt body SHA-256 mismatch")
    record = receipt.get("nfp1")
    require(isinstance(record, dict), "receipt NFP1 descriptor missing")
    record_path = (receipt_path.parent / str(record.get("path"))).resolve()
    require(record_path.parent == receipt_path.parent.resolve(), "NFP1 record escapes receipt directory")
    payload = record_path.read_bytes()
    require(str(record.get("byte_len")) == str(len(payload)), "NFP1 byte length mismatch")
    require(record.get("sha256") == sha256(payload), "NFP1 SHA-256 mismatch")
    decoded = parse_nfp1(payload)
    require(parse_uint(receipt.get("frame_hour"), "frame_hour") == decoded["frame_hour"], "frame hour mismatch")
    require(parse_uint(receipt.get("source_sequence"), "source_sequence") == decoded["source_sequence"], "source sequence mismatch")
    require(receipt.get("country_tag") == decoded["country_tag"], "country tag mismatch")
    require(receipt.get("visible_item_count") == decoded["visible_item_count"], "item count mismatch")
    completion = parse_uint(receipt.get("completion_qpc_ticks"), "completion_qpc_ticks")
    selected = parse_uint(receipt.get("selected_present_qpc_ticks"), "selected_present_qpc_ticks")
    require(completion <= selected, "focus refresh completed after selected Present")
    require(int(receipt.get("selected_present_global_order", 0)) > 0, "selected Present order missing")
    require(parse_uint(receipt.get("qpc_frequency_hz"), "qpc_frequency_hz") > 0, "QPC frequency missing")
    hook_counts = receipt.get("hook_counts")
    require(isinstance(hook_counts, dict), "hook counts missing")
    for name in ("vector_publish", "dynamic_refresh", "state_writer", "state_producer", "highlight"):
        require(parse_uint(hook_counts.get(name), f"hook_counts.{name}") > 0, f"{name} hook was not invoked")
    return {"status": "PASS", "receipt_body_sha256": body_hash, **decoded}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("receipt", type=Path)
    args = parser.parse_args()
    try:
        result = verify(args.receipt.resolve())
    except Exception as error:  # fail-closed CLI boundary
        print(json.dumps({"status": "FAIL", "error": str(error)}, sort_keys=True))
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
