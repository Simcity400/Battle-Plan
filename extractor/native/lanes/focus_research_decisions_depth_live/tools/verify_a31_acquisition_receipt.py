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
MANIFEST = "b319dd1b721ed6a13e47088b7def034111f9882631080bffbf049fbb79265a2f"
JOB = "b1c9d8c0678956f17bea2e7f9e06e7ff4baf31254c11b3c983a9a5ca9ae2d04b"
FRESH = "510353dae716bccaaf37f988b0f7101e31552ecf687ecbd613bce1dd2efb784a"
FRESH_JOB_ID = "20260720T062545Z-w4-a31-owner-runnable-producer-e5090f36"
SENTINEL = "498d09aa4161bedf4087983b47ee97626c4d78df9a4537026aff2ea1b4671429"
SENTINEL_JOB_ID = "20260720T062745Z-w4-a31-hook-entry-sentinels-284611de"
CONTEXT_SENTINEL = "4c85cfe84aee03808798f821e90f8ab5a7e019e990be67ee48b47d34c09d83b4"
CONTEXT_SENTINEL_JOB_ID = "20260720T065515Z-w4-a31-country-context-hook-sentinel-44d091b1"
FRAME_ORDER_MANIFEST = "c3c2a38805153497ad3298893a1b101582aa0651604a742e78214108b7820745"
FRAME_ORDER_JOB = "58ec9118271f47d517ed1b29cb3c900dccc4e0f60e553bbb9bbd47e3de4d082b"
FRAME_ORDER_JOB_ID = "20260720T103726Z-w5-a31-exact-frame-order-boundary-a49db01b"
DETAIL_REFRESH_SENTINEL = "31966be8db16e8aaf55465e901b6d0e8b704f8ecf0af0b83371fc540bc69723b"
DETAIL_REFRESH_SENTINEL_JOB_ID = "20260720T102614Z-w5-a31-detail-refresh-entry-sentinel-7f8e06ea"
INLINE_JOB_ID = "20260720T213713Z-w5final-a31-exact-display-joint-instrume-56d3fc12"
INLINE_MANIFEST = "76cb5caf7dc418b5fbe79dac037440ccb3cecd34140ee9019ad33c96950e0273"
INLINE_JOB_RECEIPT = "5c0dcd6032b2aa747ed02bec309c9c41d0697e445b46b86b91f9fb4132f5059a"
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


def portable_selected_present_sha256(value: object) -> str:
    require(isinstance(value, dict), "portable selected-Present receipt missing")
    expected = {
        "candidate_count",
        "candidate_set_sha256",
        "selected_candidate_id",
        "selected_ordinal",
        "selected_game_hour_raw",
        "selected_qpc_ticks",
        "selection_cutoff_qpc_ticks",
    }
    require(set(value) == expected, "portable selected-Present receipt shape mismatch")
    ordered = {
        "candidate_count": value.get("candidate_count"),
        "candidate_set_sha256": value.get("candidate_set_sha256"),
        "selected_candidate_id": value.get("selected_candidate_id"),
        "selected_ordinal": value.get("selected_ordinal"),
        "selected_game_hour_raw": value.get("selected_game_hour_raw"),
        "selected_qpc_ticks": value.get("selected_qpc_ticks"),
        "selection_cutoff_qpc_ticks": value.get("selection_cutoff_qpc_ticks"),
    }
    candidate_count = ordered["candidate_count"]
    require(isinstance(candidate_count, int) and 0 < candidate_count <= (1 << 32) - 1,
            "portable selected-Present candidate count invalid")
    require(
        isinstance(ordered["candidate_set_sha256"], str)
        and bool(re.fullmatch(r"[0-9a-f]{64}", ordered["candidate_set_sha256"])),
        "portable selected-Present candidate-set hash invalid",
    )
    selected_candidate_id = parse_uint(
        ordered["selected_candidate_id"], "selected_present.selected_candidate_id"
    )
    selected_ordinal = ordered["selected_ordinal"]
    require(selected_candidate_id > 0, "portable selected-Present candidate id invalid")
    require(
        isinstance(selected_ordinal, int)
        and 0 <= selected_ordinal <= (1 << 32) - 1
        and selected_ordinal + 1 == candidate_count,
        "portable selected-Present ordinal/count mismatch",
    )
    require(
        isinstance(ordered["selected_game_hour_raw"], int)
        and -(1 << 31) <= ordered["selected_game_hour_raw"] < (1 << 31),
        "portable selected-Present raw game hour invalid",
    )
    selected_qpc = parse_uint(
        ordered["selected_qpc_ticks"], "selected_present.selected_qpc_ticks"
    )
    cutoff_qpc = parse_uint(
        ordered["selection_cutoff_qpc_ticks"],
        "selected_present.selection_cutoff_qpc_ticks",
    )
    require(0 < selected_qpc <= cutoff_qpc, "portable selected-Present QPC order invalid")
    encoded = json.dumps(ordered, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    return sha256(encoded)


def parse_nfp1(payload: bytes) -> dict[str, object]:
    reader = Reader(payload)
    require(reader.u32() == int.from_bytes(b"NFP1", "little"), "NFP1 magic mismatch")
    require(reader.u32() == 2, "NFP1 schema mismatch")
    require(reader.u32() == 1 and reader.u32() == 0, "NFP1 is not exact authority")
    _elapsed = reader.u64()
    frame_hour = reader.u64()
    require(reader.take(32).hex() == EXE, "NFP1 executable hash mismatch")
    require(reader.take(32).hex() == MANIFEST, "NFP1 manifest hash mismatch")
    require(reader.take(32).hex() == JOB, "NFP1 job receipt hash mismatch")
    require(
        reader.u8() == 1 and reader.take(3) == b"\0\0\0",
        "NFP1 selected-Present marker invalid",
    )
    materialization_qpc_ticks = reader.u64()
    selected_present_global_order = reader.u32()
    require(reader.u32() == 0, "NFP1 selected-Present reserved word invalid")
    selected_present_qpc_ticks = reader.u64()
    qpc_frequency_hz = reader.u64()
    selected_present_receipt_sha256 = reader.take(32).hex()
    require(
        0 < materialization_qpc_ticks <= selected_present_qpc_ticks
        and selected_present_global_order > 0
        and qpc_frequency_hz > 0,
        "NFP1 selected-Present order/frequency invalid",
    )
    require(reader.u8() == 1 and reader.take(3) == b"\0\0\0", "NFP1 snapshot marker invalid")
    source_sequence = reader.u64()
    country_tag = reader.text()
    view_id = reader.text()
    record_identity = reader.text()
    expected = reader.u32()
    count = reader.u32()
    require(0 < source_sequence and 0 < count <= 131_072 and expected == count, "NFP1 count/sequence invalid")
    require(bool(TAG.fullmatch(country_tag)), "NFP1 country tag invalid")
    require(view_id == f"nationalfocusview:{country_tag}", "NFP1 view identity invalid")
    require(
        record_identity == f"a729:a31:{frame_hour}:{country_tag}:{source_sequence}",
        "NFP1 producer identity invalid",
    )
    seen: set[str] = set()
    items: list[dict[str, object]] = []
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
        items.append({
            "focus_id": focus_id,
            "source_ordinal": ordinal,
            "presentation_state_code": state,
            "display_progress_percent": percent,
            "presentation_flags": flags,
            "joint_viewing_country_tag": joint,
        })
    require(reader.at == len(payload), "NFP1 payload has trailing bytes")
    return {
        "frame_hour": frame_hour,
        "source_sequence": source_sequence,
        "country_tag": country_tag,
        "visible_item_count": count,
        "materialization_qpc_ticks": materialization_qpc_ticks,
        "selected_present_global_order": selected_present_global_order,
        "selected_present_qpc_ticks": selected_present_qpc_ticks,
        "qpc_frequency_hz": qpc_frequency_hz,
        "selected_present_receipt_sha256": selected_present_receipt_sha256,
        "items": items,
    }


def verify(receipt_path: Path) -> dict[str, object]:
    receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
    require(receipt.get("schema") == "hoi4.a31.selected_present_acquisition_receipt.v2", "receipt schema mismatch")
    require(receipt.get("binary_sha256") == EXE, "receipt executable hash mismatch")
    require(receipt.get("exact20_function_manifest_sha256") == MANIFEST, "receipt manifest mismatch")
    require(receipt.get("exact20_job_receipt_sha256") == JOB, "receipt job hash mismatch")
    require(receipt.get("fresh_acquisition_job_id") == FRESH_JOB_ID, "fresh acquisition job id mismatch")
    require(receipt.get("fresh_acquisition_manifest_sha256") == FRESH, "fresh acquisition hash mismatch")
    require(receipt.get("sentinel_dump_job_id") == SENTINEL_JOB_ID, "sentinel dump job id mismatch")
    require(receipt.get("sentinel_dump_sha256") == SENTINEL, "sentinel dump hash mismatch")
    require(receipt.get("country_context_sentinel_job_id") == CONTEXT_SENTINEL_JOB_ID, "country-context sentinel job id mismatch")
    require(receipt.get("country_context_sentinel_dump_sha256") == CONTEXT_SENTINEL, "country-context sentinel hash mismatch")
    require(receipt.get("frame_order_job_id") == FRAME_ORDER_JOB_ID, "frame-order job id mismatch")
    require(receipt.get("frame_order_manifest_sha256") == FRAME_ORDER_MANIFEST, "frame-order manifest mismatch")
    require(receipt.get("frame_order_job_receipt_sha256") == FRAME_ORDER_JOB, "frame-order job hash mismatch")
    require(receipt.get("detail_refresh_sentinel_job_id") == DETAIL_REFRESH_SENTINEL_JOB_ID, "detail-refresh sentinel job id mismatch")
    require(receipt.get("detail_refresh_sentinel_dump_sha256") == DETAIL_REFRESH_SENTINEL, "detail-refresh sentinel mismatch")
    require(receipt.get("inline_instrumentation_job_id") == INLINE_JOB_ID, "inline instrumentation job id mismatch")
    require(receipt.get("inline_instrumentation_manifest_sha256") == INLINE_MANIFEST, "inline instrumentation manifest mismatch")
    require(receipt.get("inline_instrumentation_job_receipt_sha256") == INLINE_JOB_RECEIPT, "inline instrumentation job receipt mismatch")
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
    publish_seen = receipt.get("same_epoch_vector_publish_seen")
    require(isinstance(publish_seen, bool), "same-epoch vector-publish witness missing")
    armed = parse_uint(receipt.get("arm_qpc_ticks"), "arm_qpc_ticks")
    completion = parse_uint(receipt.get("completion_qpc_ticks"), "completion_qpc_ticks")
    selected = parse_uint(receipt.get("selected_present_qpc_ticks"), "selected_present_qpc_ticks")
    require(armed <= completion <= selected, "focus refresh is outside the arm-to-selected-Present bracket")
    require(completion == decoded["materialization_qpc_ticks"], "NFP1 materialization QPC mismatch")
    require(int(receipt.get("selected_present_global_order", 0)) > 0, "selected Present order missing")
    require(
        receipt.get("selected_present_global_order") == decoded["selected_present_global_order"],
        "NFP1 selected-Present global order mismatch",
    )
    require(
        isinstance(receipt.get("selected_present_receipt_sha256"), str)
        and bool(re.fullmatch(r"[0-9a-f]{64}", receipt["selected_present_receipt_sha256"])),
        "selected Present receipt SHA-256 is invalid",
    )
    portable_selected = receipt.get("selected_present")
    portable_hash = portable_selected_present_sha256(portable_selected)
    require(
        receipt.get("selected_present_portable_receipt_sha256") == portable_hash,
        "portable selected-Present receipt SHA-256 mismatch",
    )
    require(
        decoded["selected_present_receipt_sha256"] == portable_hash,
        "NFP1 portable selected-Present receipt SHA-256 mismatch",
    )
    require(
        parse_uint(receipt.get("selected_present_qpc_ticks"), "selected_present_qpc_ticks")
        == decoded["selected_present_qpc_ticks"],
        "NFP1 selected-Present QPC mismatch",
    )
    require(
        parse_uint(receipt.get("qpc_frequency_hz"), "qpc_frequency_hz")
        == decoded["qpc_frequency_hz"]
        and decoded["qpc_frequency_hz"] > 0,
        "NFP1 QPC frequency mismatch",
    )
    hook_counts = receipt.get("hook_counts")
    require(isinstance(hook_counts, dict), "hook counts missing")
    vector_publish_count = parse_uint(hook_counts.get("vector_publish"), "hook_counts.vector_publish")
    require((vector_publish_count > 0) == publish_seen, "vector-publish count/witness mismatch")
    for name in (
        "dynamic_refresh",
        "detail_refresh",
        "state_writer",
        "state_producer",
        "highlight",
        "display_percent_callsite",
        "joint_country_boundary",
    ):
        require(parse_uint(hook_counts.get(name), f"hook_counts.{name}") > 0, f"{name} hook was not invoked")
    public_decoded = {key: value for key, value in decoded.items() if key != "items"}
    return {"status": "PASS", "receipt_body_sha256": body_hash, **public_decoded}


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
