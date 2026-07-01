"""Read-only current-build probe for direct ace roster roots.

This is a lane-local RE/proof tool. It does not edit or invoke shared collector
entrypoints. The save oracle is used only as a current-fixture validation anchor;
runtime production must still come from live `hoi4.exe` memory.
"""

from __future__ import annotations

import argparse
import collections
import datetime as dt
import json
import struct
import sys
from pathlib import Path
from typing import Any


THIS_DIR = Path(__file__).resolve().parent
REPO_ROOT = THIS_DIR.parents[3]
sys.path.insert(0, str(REPO_ROOT / "extractor" / "memory_reader"))
sys.path.insert(0, str(REPO_ROOT / "extractor" / "save_oracle"))

try:
    import winmem  # type: ignore  # noqa: E402
except Exception:  # pragma: no cover - non-Windows/unit-less environments
    winmem = None

try:
    import proof_live_token_lifecycle_gates as air_proof  # type: ignore  # noqa: E402
except Exception:  # pragma: no cover - tests can exercise this module without live probe imports
    air_proof = None

try:
    import deep_common as save_dc  # type: ignore  # noqa: E402
except Exception:  # pragma: no cover - live proof can run without save-oracle helpers
    save_dc = None


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

C_GAME_STATE_GLOBAL = 0x3316E90
COUNTRY_DATABASE_GLOBAL = 0x33189C8
COUNTRY_VTABLE_RVA = 0x27D0E90

CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C
MAX_CGAMESTATE_COUNTRIES = 4096

OBJ_TAG_DB_ID = 0x08
HISTORICAL_ACE_VECTOR_OFF = 0x12D8
HISTORICAL_ACE_COUNT_OFF = 0x12E4
ACE_OBJECT_ID_OFF = 0x0C
ACE_OBJECT_NAME_OFF = 0xF0
ACE_OBJECT_SURNAME_OFF = 0x110
ACE_OBJECT_CALLSIGN_OFF = 0x130
WING_BODY_ASSIGNED_ACE_ID_OFF = 0x2DC

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
MAX_ACE_ID = 0x7FFFFFFF
MAX_ACE_COUNT = 1024
MAX_ACE_STRING_LEN = 96
MAX_VECTOR_CAP = 4096
MAX_COUNTRY_OBJECT_SCAN = 0x2200
MAX_VECTOR_BYTES = 128 * 1024
ACE_OBJECT_ASSIGNMENT_SCAN_BYTES = 0x1000
WING_ASSIGNMENT_SCAN_BYTES = 0x1000
INLINE_VECTOR_STRIDES = (8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 256)
COMPACT_RECORD_MAGIC = b"H4CR"
COMPACT_RECORD_VERSION = 1
COMPACT_RECORD_HEADER_LEN = 32
COMPACT_RECORD_TYPE_ACE_ROSTER_DATA = 0xACE1
DOMAIN_ID = 46
ACE_STATUS_DELTA_DATA = 1 << 10
ROW_FLAG_NAME = 1 << 1
ROW_FLAG_SURNAME = 1 << 2
ROW_FLAG_CALLSIGN = 1 << 3
ROW_FLAG_WING_ID = 1 << 4
SOURCE_LIVE_COUNTRY_ACE_VECTOR = 1
SOURCE_LIVE_ACE_OBJECT_LAYOUT = 1 << 1
SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT = 1 << 4


def hx(value: int | None) -> str | None:
    return None if value is None else f"0x{value:X}"


def utc_now() -> str:
    return dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat()


def safe(fn):
    try:
        return fn()
    except Exception:
        return None


def is_heap_ptr(value: int | None) -> bool:
    return isinstance(value, int) and HEAP_LO <= value < HEAP_HI


def read_u32_stable(proc: Any, addr: int) -> int | None:
    first = safe(lambda: proc.u32(addr))
    second = safe(lambda: proc.u32(addr))
    return first if first == second else None


def read_u64_stable(proc: Any, addr: int) -> int | None:
    first = safe(lambda: proc.u64(addr))
    second = safe(lambda: proc.u64(addr))
    return first if first == second else None


def read_small_inline_string(proc: Any, addr: int) -> str | None:
    raw = safe(lambda: proc.try_read(addr, 32))
    if raw is None or len(raw) != 32:
        return None
    size = struct.unpack("<I", raw[16:20])[0]
    if size == 0 or size > 7:
        return None
    body = raw[:size]
    if not all(byte in b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-" for byte in body):
        return None
    try:
        value = body.decode("ascii")
    except UnicodeDecodeError:
        return None
    if len(value) != 3 or (value != "---" and not any(ch.isalpha() for ch in value)):
        return None
    return value


def read_msvc_string_stable(proc: Any, addr: int, max_len: int = MAX_ACE_STRING_LEN) -> str | None:
    def read_once() -> str | None:
        raw = proc.try_read(addr, 0x20)
        if len(raw) < 0x20:
            return None
        size = struct.unpack_from("<Q", raw, 0x10)[0]
        cap = struct.unpack_from("<Q", raw, 0x18)[0]
        if size == 0:
            return None
        if size > max_len or cap < size or cap > (1 << 20):
            return None
        if cap < 16:
            body = raw[:size]
        else:
            ptr = struct.unpack_from("<Q", raw, 0)[0]
            if not is_heap_ptr(ptr):
                return None
            heap = proc.try_read(ptr, size + 1)
            if len(heap) < size + 1 or heap[size] != 0:
                return None
            body = heap[:size]
        if not all(32 <= byte < 127 for byte in body):
            return None
        try:
            return body.decode("utf-8")
        except UnicodeDecodeError:
            return body.decode("latin-1", errors="replace")

    first = safe(read_once)
    second = safe(read_once)
    return first if first == second else None


def country_tag_table(proc: Any, country_database: int | None) -> tuple[int | None, dict[int, str], dict[str, Any]]:
    meta: dict[str, Any] = {
        "country_database": hx(country_database),
        "tag_table": None,
        "tag_count": None,
        "active_count": None,
        "prefix_ok": False,
        "valid": False,
    }
    if not country_database:
        return None, {}, meta
    tag_table = read_u64_stable(proc, country_database)
    tag_count = read_u32_stable(proc, country_database + 8)
    active_count = read_u32_stable(proc, country_database + 12)
    meta.update({"tag_table": hx(tag_table), "tag_count": tag_count, "active_count": active_count})
    if not tag_table or not tag_count or tag_count < 40 or tag_count > 1024:
        return None, {}, meta
    prefix = {0: "---", 1: "GER", 2: "ENG", 3: "SOV"}
    prefix_ok = all(read_small_inline_string(proc, tag_table + idx * 32) == tag for idx, tag in prefix.items())
    meta["prefix_ok"] = prefix_ok
    if not prefix_ok or not active_count or active_count > tag_count:
        return None, {}, meta
    row_count = min(tag_count, active_count, 1024)
    tags = {}
    for tag_db_id in range(row_count):
        tag = read_small_inline_string(proc, tag_table + tag_db_id * 32)
        if tag:
            tags[tag_db_id] = tag
    meta["row_count"] = row_count
    meta["valid"] = len(tags) >= 40
    return tag_table, tags, meta


def live_country_rows(proc: Any, module_base: int, game_state: int, tag_by_id: dict[int, str]) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    country_table = read_u64_stable(proc, game_state + CGAMESTATE_COUNTRY_TABLE)
    country_count = read_u32_stable(proc, game_state + CGAMESTATE_COUNTRY_COUNT)
    meta = {
        "country_table": hx(country_table),
        "country_count": country_count,
        "valid": bool(country_table and is_heap_ptr(country_table) and country_count and country_count <= MAX_CGAMESTATE_COUNTRIES),
    }
    if not meta["valid"]:
        return [], meta
    rows: list[dict[str, Any]] = []
    country_vtable = module_base + COUNTRY_VTABLE_RVA
    for index in range(1, min(country_count or 0, len(tag_by_id), MAX_CGAMESTATE_COUNTRIES)):
        ptr = read_u64_stable(proc, country_table + index * 8)
        if not is_heap_ptr(ptr):
            continue
        if read_u64_stable(proc, ptr) != country_vtable:
            continue
        tag_db_id = read_u32_stable(proc, ptr + OBJ_TAG_DB_ID)
        rows.append(
            {
                "index": index,
                "tag_db_id": tag_db_id,
                "tag": tag_by_id.get(index),
                "object_ptr": ptr,
                "tag_matches_index": tag_db_id == index,
            }
        )
    return rows, meta


def load_ace_oracle(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    aces_by_tag = data.get("aces", {})
    tags: dict[str, dict[str, Any]] = {}
    for tag, payload in aces_by_tag.items():
        aces = payload.get("aces", []) if isinstance(payload, dict) else []
        ace_ids = [row.get("ace_id") for row in aces if isinstance(row.get("ace_id"), int)]
        names = []
        for row in aces:
            for key in ("name", "surname", "callsign", "modifier"):
                value = row.get(key)
                if isinstance(value, str) and value and len(value) >= 3:
                    names.append(value)
        tags[tag] = {
            "count": int(payload.get("count", len(aces))),
            "aces": aces,
            "ace_ids": ace_ids,
            "string_anchors": sorted(set(names))[:32],
        }
    return {
        "source": str(path),
        "save_date": data.get("meta", {}).get("save_date"),
        "stats": data.get("meta", {}).get("stats", {}),
        "tags": tags,
    }


def load_save_air_wing_ace_assignments(path: Path | None, tag_to_id: dict[str, int]) -> dict[str, Any]:
    if path is None:
        return {"available": False, "reason": "melted_save_not_provided"}
    if save_dc is None:
        return {"available": False, "reason": "save_oracle_deep_common_unavailable", "source": str(path)}
    if not path.exists():
        return {"available": False, "reason": "melted_save_not_found", "source": str(path)}

    text = path.read_text(encoding="utf-8", errors="replace")
    assignments: list[dict[str, Any]] = []
    totals = {"air_wing_rows": 0, "air_wing_pool_rows": 0, "tag_blocks": 0}

    def id_key(id_pair: tuple[int | None, int | None]) -> str | None:
        object_id, object_type = id_pair
        if object_id is None:
            return None
        return f"{object_id}:{object_type}"

    def parse_air_wing(tz: Any, pool_tag: str) -> None:
        totals["air_wing_rows"] += 1
        wing_id = None
        wing_type = None
        ace_id = None
        ace_type = None
        owner = pool_tag
        name = None
        for key, opener in save_dc.iter_block_entries(tz):
            if opener is save_dc.LBRACE:
                if key == "id":
                    wing_id, wing_type = save_dc.read_id_block(tz)
                elif key == "ace":
                    ace_id, ace_type = save_dc.read_id_block(tz)
                else:
                    save_dc._skip_block_body(tz)
                continue
            value = save_dc.atom_value(opener)
            if value is None:
                continue
            if key == "tag":
                owner = value
            elif key == "name":
                name = value
        if wing_id is not None and ace_id is not None:
            assignments.append(
                {
                    "owner": owner,
                    "owner_tag_db_id": tag_to_id.get(owner),
                    "wing_id": int(wing_id),
                    "wing_key": id_key((wing_id, wing_type)),
                    "ace_id": int(ace_id),
                    "ace_key": id_key((ace_id, ace_type)),
                    "name": name,
                }
            )

    def parse_air_wing_pool(tz: Any, tag: str) -> None:
        totals["air_wing_pool_rows"] += 1
        for key, opener in save_dc.iter_block_entries(tz):
            if opener is not save_dc.LBRACE:
                continue
            if key == "air_wings":
                parse_air_wing(tz, tag)
            else:
                save_dc._skip_block_body(tz)

    def parse_strategic_air(tz: Any) -> None:
        for key, opener in save_dc.iter_block_entries(tz):
            if opener is not save_dc.LBRACE:
                continue
            if len(key) <= 4 and key.isupper():
                totals["tag_blocks"] += 1
                for inner_key, inner_opener in save_dc.iter_block_entries(tz):
                    if inner_opener is not save_dc.LBRACE:
                        continue
                    if inner_key == "air_wing_pool":
                        parse_air_wing_pool(tz, key)
                    else:
                        save_dc._skip_block_body(tz)
            else:
                save_dc._skip_block_body(tz)

    meta = save_dc.top_level_pass(
        text,
        {"strategic_air": parse_strategic_air},
        scalar_keys=("date", "version"),
    )
    unmapped_tags = sorted({row["owner"] for row in assignments if row.get("owner_tag_db_id") is None})
    return {
        "available": True,
        "source": str(path),
        "meta": meta,
        "air_wing_rows": totals["air_wing_rows"],
        "air_wing_pool_rows": totals["air_wing_pool_rows"],
        "tag_blocks": totals["tag_blocks"],
        "assignment_rows": len(assignments),
        "unmapped_owner_tags": unmapped_tags[:16],
        "assignments": assignments,
        "sample": assignments[:16],
    }


def score_ace_count_offsets(proc: Any, rows: list[dict[str, Any]], oracle: dict[str, Any]) -> list[dict[str, Any]]:
    by_tag = oracle["tags"]
    scored = []
    for off in range(0, MAX_COUNTRY_OBJECT_SCAN + 1, 4):
        matched = 0
        nonzero_matched = 0
        total = 0
        nonzero_total = 0
        examples = []
        bad_examples = []
        for row in rows:
            tag = row.get("tag")
            if tag not in by_tag:
                continue
            want = by_tag[tag]["count"]
            if want > MAX_ACE_COUNT:
                continue
            got = read_u32_stable(proc, row["object_ptr"] + off)
            if got is None or got > MAX_ACE_COUNT:
                continue
            total += 1
            if want:
                nonzero_total += 1
            if got == want:
                matched += 1
                if want:
                    nonzero_matched += 1
                if len(examples) < 12 and want:
                    examples.append({"tag": tag, "want": want, "got": got})
            elif want and len(bad_examples) < 8:
                bad_examples.append({"tag": tag, "want": want, "got": got})
        if nonzero_matched:
            scored.append(
                {
                    "offset": hx(off),
                    "matches": matched,
                    "total_checked": total,
                    "nonzero_matches": nonzero_matched,
                    "nonzero_total": nonzero_total,
                    "examples": examples,
                    "bad_examples": bad_examples,
                    "historical_16022851_offset": off == HISTORICAL_ACE_COUNT_OFF,
                }
            )
    return sorted(scored, key=lambda row: (row["nonzero_matches"], row["matches"]), reverse=True)[:24]


def custom_vector_header(proc: Any, obj: int, off: int) -> tuple[int, int, int] | None:
    data = read_u64_stable(proc, obj + off)
    cap = read_u32_stable(proc, obj + off + 8)
    size = read_u32_stable(proc, obj + off + 0x0C)
    if data is None or cap is None or size is None:
        return None
    if size > cap or cap > MAX_VECTOR_CAP:
        return None
    if size and not is_heap_ptr(data):
        return None
    return data, cap, size


def pointer_vector_header(proc: Any, obj: int, off: int, stride: int) -> tuple[int, int, int] | None:
    begin = read_u64_stable(proc, obj + off)
    end = read_u64_stable(proc, obj + off + 8)
    cap = read_u64_stable(proc, obj + off + 16)
    if begin is None or end is None or cap is None:
        return None
    if begin == 0 and end == 0 and cap == 0:
        return 0, 0, 0
    if not (is_heap_ptr(begin) and is_heap_ptr(end) and is_heap_ptr(cap)):
        return None
    if end < begin or cap < end:
        return None
    span = end - begin
    if span % stride != 0:
        return None
    size = span // stride
    capacity = (cap - begin) // stride if cap >= begin else 0
    if size > capacity or capacity > MAX_VECTOR_CAP:
        return None
    return begin, capacity, size


def payload_evidence(proc: Any, data: int, size: int, oracle_tag: dict[str, Any], stride: int | None = None) -> dict[str, Any]:
    if not data or not size or not is_heap_ptr(data):
        return {"available": False}
    if stride is None:
        want = min(MAX_VECTOR_BYTES, max(0x400, size * 256))
    else:
        want = min(MAX_VECTOR_BYTES, size * stride)
    raw = proc.try_read(data, want)
    ace_id_hits = []
    for ace_id in oracle_tag.get("ace_ids", [])[:64]:
        if struct.pack("<I", ace_id) in raw:
            ace_id_hits.append(ace_id)
    string_hits = []
    for text in oracle_tag.get("string_anchors", [])[:32]:
        needle = text.encode("latin1", errors="ignore")
        if len(needle) >= 3 and needle in raw:
            string_hits.append(text)
    pointer_targets_checked = 0
    pointer_target_ace_id_hits = []
    pointer_target_string_hits = []
    layout_votes: dict[str, collections.Counter[int]] = {
        "ace_id": collections.Counter(),
        "name": collections.Counter(),
        "surname": collections.Counter(),
        "callsign": collections.Counter(),
        "modifier": collections.Counter(),
    }
    layout_samples = []
    if len(raw) >= 8:
        qwords = [
            struct.unpack_from("<Q", raw, pos)[0]
            for pos in range(0, min(len(raw), size * 8), 8)
        ]
        oracle_aces = oracle_tag.get("aces", [])
        for index, ptr in enumerate(qwords[: min(len(qwords), 96)]):
            if not is_heap_ptr(ptr):
                continue
            body = proc.try_read(ptr, 0x500)
            if not body:
                continue
            pointer_targets_checked += 1
            expected = oracle_aces[index] if index < len(oracle_aces) else {}
            sample: dict[str, Any] = {"index": index, "ptr": hx(ptr), "matched_offsets": {}}
            ace_id = expected.get("ace_id")
            if isinstance(ace_id, int):
                needle = struct.pack("<I", ace_id)
                at = body.find(needle)
                if at >= 0:
                    layout_votes["ace_id"][at] += 1
                    sample["matched_offsets"]["ace_id"] = hx(at)
            for ace_id in oracle_tag.get("ace_ids", [])[:64]:
                if ace_id not in pointer_target_ace_id_hits and struct.pack("<I", ace_id) in body:
                    pointer_target_ace_id_hits.append(ace_id)
            for field in ("name", "surname", "callsign", "modifier"):
                value = expected.get(field)
                if not isinstance(value, str) or len(value) < 3:
                    continue
                needle = value.encode("latin1", errors="ignore")
                if not needle:
                    continue
                at = body.find(needle)
                if at >= 0:
                    layout_votes[field][at] += 1
                    sample["matched_offsets"][field] = hx(at)
            for text in oracle_tag.get("string_anchors", [])[:32]:
                needle = text.encode("latin1", errors="ignore")
                if len(needle) >= 3 and text not in pointer_target_string_hits and needle in body:
                    pointer_target_string_hits.append(text)
            if sample["matched_offsets"] and len(layout_samples) < 12:
                layout_samples.append(sample)
    layout_summary = {
        key: [
            {"offset": hx(offset), "votes": votes}
            for offset, votes in counter.most_common(8)
        ]
        for key, counter in layout_votes.items()
        if counter
    }
    return {
        "available": True,
        "bytes_read": len(raw),
        "ace_id_hits": ace_id_hits[:12],
        "string_hits": string_hits[:12],
        "pointer_targets_checked": pointer_targets_checked,
        "pointer_target_ace_id_hits": pointer_target_ace_id_hits[:12],
        "pointer_target_string_hits": pointer_target_string_hits[:12],
        "pointer_target_layout_votes": layout_summary,
        "pointer_target_layout_samples": layout_samples,
    }


def scan_vector_candidates(proc: Any, rows: list[dict[str, Any]], oracle: dict[str, Any]) -> list[dict[str, Any]]:
    rows_by_tag = {row["tag"]: row for row in rows if row.get("tag")}
    ace_tags = {tag: payload for tag, payload in oracle["tags"].items() if payload["count"] > 0 and tag in rows_by_tag}
    zero_controls = [
        tag
        for tag, row in rows_by_tag.items()
        if tag not in oracle["tags"] or oracle["tags"].get(tag, {}).get("count", 0) == 0
    ][:64]
    candidates: list[dict[str, Any]] = []

    for off in range(0, MAX_COUNTRY_OBJECT_SCAN + 1, 8):
        matched = []
        bad = []
        headers = {}
        for tag, expected in ace_tags.items():
            header = custom_vector_header(proc, rows_by_tag[tag]["object_ptr"], off)
            if header is None:
                bad.append({"tag": tag, "want": expected["count"], "reason": "invalid_header"})
                continue
            data, cap, size = header
            headers[tag] = {"data": data, "cap": cap, "size": size}
            if size == expected["count"]:
                matched.append(tag)
            elif len(bad) < 8:
                bad.append({"tag": tag, "want": expected["count"], "got": size})
        if len(matched) >= max(4, len(ace_tags) - 1):
            zero_false = 0
            for tag in zero_controls:
                header = custom_vector_header(proc, rows_by_tag[tag]["object_ptr"], off)
                if header and header[2] != 0:
                    zero_false += 1
            evidence_tag = max(matched, key=lambda tag: ace_tags[tag]["count"])
            evidence_header = headers[evidence_tag]
            candidates.append(
                {
                    "kind": "custom_ptr_cap_size",
                    "offset": hx(off),
                    "matched_ace_tags": len(matched),
                    "ace_tags_total": len(ace_tags),
                    "zero_control_nonzero": zero_false,
                    "matched_examples": matched[:16],
                    "bad_examples": bad,
                    "sample_header": {
                        "tag": evidence_tag,
                        "data": hx(evidence_header["data"]),
                        "cap": evidence_header["cap"],
                        "size": evidence_header["size"],
                    },
                    "payload_evidence": payload_evidence(proc, evidence_header["data"], evidence_header["size"], ace_tags[evidence_tag]),
                }
            )

        for stride in INLINE_VECTOR_STRIDES:
            matched = []
            bad = []
            headers = {}
            for tag, expected in ace_tags.items():
                header = pointer_vector_header(proc, rows_by_tag[tag]["object_ptr"], off, stride)
                if header is None:
                    if len(bad) < 8:
                        bad.append({"tag": tag, "want": expected["count"], "reason": "invalid_header"})
                    continue
                begin, cap, size = header
                headers[tag] = {"begin": begin, "cap": cap, "size": size}
                if size == expected["count"]:
                    matched.append(tag)
                elif len(bad) < 8:
                    bad.append({"tag": tag, "want": expected["count"], "got": size})
            if len(matched) >= max(4, len(ace_tags) - 1):
                zero_false = 0
                for tag in zero_controls:
                    header = pointer_vector_header(proc, rows_by_tag[tag]["object_ptr"], off, stride)
                    if header and header[2] != 0:
                        zero_false += 1
                evidence_tag = max(matched, key=lambda tag: ace_tags[tag]["count"])
                evidence_header = headers[evidence_tag]
                candidates.append(
                    {
                        "kind": "begin_end_cap",
                        "offset": hx(off),
                        "stride": stride,
                        "matched_ace_tags": len(matched),
                        "ace_tags_total": len(ace_tags),
                        "zero_control_nonzero": zero_false,
                        "matched_examples": matched[:16],
                        "bad_examples": bad,
                        "sample_header": {
                            "tag": evidence_tag,
                            "begin": hx(evidence_header["begin"]),
                            "cap": evidence_header["cap"],
                            "size": evidence_header["size"],
                        },
                        "payload_evidence": payload_evidence(
                            proc,
                            evidence_header["begin"],
                            evidence_header["size"],
                            ace_tags[evidence_tag],
                            stride,
                        ),
                    }
                )

    return sorted(
        candidates,
        key=lambda row: (
            row["matched_ace_tags"],
            -row["zero_control_nonzero"],
            len(row["payload_evidence"].get("ace_id_hits", []))
            + len(row["payload_evidence"].get("pointer_target_ace_id_hits", [])),
        ),
        reverse=True,
    )[:24]


def historical_roster_header_probe(proc: Any, rows: list[dict[str, Any]], oracle: dict[str, Any]) -> dict[str, Any]:
    """Probe the known-adjacent CCountry ace roster header without strict save count gating."""

    rows_by_tag = {row["tag"]: row for row in rows if row.get("tag")}
    oracle_tags = oracle["tags"]
    row_evidence = []
    live_nonzero_tags = 0
    live_count_matched_tags = 0
    oracle_nonzero_tags = 0
    oracle_count_matched_tags = 0
    zero_control_nonzero = 0
    invalid_headers = 0
    evidence_choice: tuple[str, dict[str, Any], tuple[int, int, int]] | None = None

    for tag, row in sorted(rows_by_tag.items()):
        live_count = read_u32_stable(proc, row["object_ptr"] + HISTORICAL_ACE_COUNT_OFF)
        oracle_payload = oracle_tags.get(tag, {"count": 0, "ace_ids": [], "string_anchors": [], "aces": []})
        oracle_count = int(oracle_payload.get("count", 0))
        header = custom_vector_header(proc, row["object_ptr"], HISTORICAL_ACE_VECTOR_OFF)
        entry: dict[str, Any] = {
            "tag": tag,
            "object_ptr": hx(row.get("object_ptr")),
            "live_ace_count": live_count,
            "oracle_ace_count": oracle_count,
            "header_valid": header is not None,
        }
        if live_count and live_count <= MAX_ACE_COUNT:
            live_nonzero_tags += 1
        if oracle_count:
            oracle_nonzero_tags += 1
        if header is None:
            if live_count or oracle_count:
                invalid_headers += 1
            entry["reason"] = "invalid_header"
        else:
            data, cap, size = header
            entry.update(
                {
                    "data": hx(data),
                    "cap": cap,
                    "size": size,
                    "matches_live_count": size == live_count,
                    "matches_oracle_count": size == oracle_count,
                }
            )
            if live_count == 0 and size != 0:
                zero_control_nonzero += 1
            if live_count and live_count <= MAX_ACE_COUNT and size == live_count:
                live_count_matched_tags += 1
            if oracle_count and size == oracle_count:
                oracle_count_matched_tags += 1
            if oracle_count and size and data and is_heap_ptr(data):
                if evidence_choice is None or oracle_count > evidence_choice[1].get("count", 0):
                    evidence_choice = (tag, oracle_payload, header)
        if live_count or oracle_count or (header is not None and header[2] != 0):
            row_evidence.append(entry)

    payload: dict[str, Any] = {"available": False}
    candidate: dict[str, Any] | None = None
    if evidence_choice:
        evidence_tag, oracle_payload, header = evidence_choice
        data, cap, size = header
        payload = payload_evidence(proc, data, size, oracle_payload)
        candidate = {
            "kind": "historical_custom_ptr_cap_size",
            "offset": hx(HISTORICAL_ACE_VECTOR_OFF),
            "matched_ace_tags": oracle_count_matched_tags,
            "ace_tags_total": oracle_nonzero_tags,
            "live_count_matched_tags": live_count_matched_tags,
            "live_count_nonzero_tags": live_nonzero_tags,
            "zero_control_nonzero": zero_control_nonzero,
            "matched_examples": [
                row["tag"]
                for row in row_evidence
                if row.get("matches_live_count") and row.get("live_ace_count")
            ][:16],
            "bad_examples": [
                row
                for row in row_evidence
                if (row.get("live_ace_count") or row.get("oracle_ace_count"))
                and not row.get("matches_live_count")
            ][:8],
            "sample_header": {
                "tag": evidence_tag,
                "data": hx(data),
                "cap": cap,
                "size": size,
            },
            "payload_evidence": payload,
        }

    payload_validated = bool(
        payload.get("pointer_target_ace_id_hits") or payload.get("ace_id_hits")
    )
    live_threshold = max(4, live_nonzero_tags - 1)
    live_size_validated = live_nonzero_tags >= 4 and live_count_matched_tags >= live_threshold
    status = (
        "validated_live_roster_header_candidate"
        if live_size_validated and payload_validated
        else "blocked_no_payload_identity_evidence"
        if live_size_validated
        else "blocked_header_size_does_not_match_live_ace_count"
        if live_nonzero_tags
        else "blocked_no_live_ace_count_rows"
    )

    return {
        "offset": hx(HISTORICAL_ACE_VECTOR_OFF),
        "count_offset": hx(HISTORICAL_ACE_COUNT_OFF),
        "status": status,
        "live_count_nonzero_tags": live_nonzero_tags,
        "live_count_matched_tags": live_count_matched_tags,
        "oracle_nonzero_tags": oracle_nonzero_tags,
        "oracle_count_matched_tags": oracle_count_matched_tags,
        "zero_control_nonzero": zero_control_nonzero,
        "invalid_headers": invalid_headers,
        "payload_evidence": payload,
        "row_evidence": row_evidence[:64],
        "candidate": candidate,
        "validation_rule": (
            "CCountry+0x12D8 custom ptr/cap/size header is accepted only when header size "
            "matches CCountry+0x12E4 live ace_count for nearly all nonzero live countries and "
            "the sampled vector payload resolves save-oracle ace ids or identity strings."
        ),
    }


def collect_live_ace_pointer_index(proc: Any, rows: list[dict[str, Any]]) -> dict[str, Any]:
    entries = []
    invalid_headers = 0
    truncated_vectors = 0
    for row in rows:
        tag = row.get("tag")
        live_count = read_u32_stable(proc, row["object_ptr"] + HISTORICAL_ACE_COUNT_OFF)
        if not live_count or live_count > MAX_ACE_COUNT:
            continue
        header = custom_vector_header(proc, row["object_ptr"], HISTORICAL_ACE_VECTOR_OFF)
        if header is None:
            invalid_headers += 1
            continue
        data, _cap, size = header
        if size != live_count or not data:
            invalid_headers += 1
            continue
        raw = proc.try_read(data, min(MAX_VECTOR_BYTES, size * 8))
        if len(raw) < size * 8:
            truncated_vectors += 1
            continue
        for index in range(size):
            ptr = struct.unpack_from("<Q", raw, index * 8)[0]
            if not is_heap_ptr(ptr):
                continue
            ace_id = read_u32_stable(proc, ptr + ACE_OBJECT_ID_OFF)
            if not ace_id or ace_id > MAX_ACE_ID:
                continue
            entries.append(
                {
                    "tag": tag,
                    "tag_db_id": row.get("tag_db_id"),
                    "country_ace_index": index,
                    "ace_ptr": hx(ptr),
                    "ace_id": ace_id,
                    "name": read_msvc_string_stable(proc, ptr + ACE_OBJECT_NAME_OFF),
                    "surname": read_msvc_string_stable(proc, ptr + ACE_OBJECT_SURNAME_OFF),
                    "callsign": read_msvc_string_stable(proc, ptr + ACE_OBJECT_CALLSIGN_OFF),
                }
            )
    duplicate_ids = [
        ace_id
        for ace_id, count in collections.Counter(row["ace_id"] for row in entries).items()
        if count > 1
    ]
    return {
        "available": bool(entries),
        "entry_count": len(entries),
        "country_rows": len({row.get("tag_db_id") for row in entries}),
        "invalid_headers": invalid_headers,
        "truncated_vectors": truncated_vectors,
        "duplicate_ace_ids": duplicate_ids[:16],
        "entries": entries,
        "sample": entries[:16],
    }


def enumerate_air_wings(pid: int) -> dict[str, Any]:
    if air_proof is None:
        return {"available": False, "reason": "air_proof_import_unavailable"}
    probe = air_proof.LiveAirProbe(pid)
    try:
        root = probe.root_probe()
        strategic_air = probe.require_root(root)
        enumeration = probe.enumerate_air_wings(strategic_air)
        return {
            "available": True,
            "root": root,
            "topology": enumeration.get("topology", {}),
            "air_wings": enumeration.get("air_wings", []),
        }
    except Exception as exc:
        return {"available": False, "reason": str(exc)}
    finally:
        probe.close()


def scan_wing_assignment_candidate(
    proc: Any,
    candidate: dict[str, Any] | None,
    air_wings: dict[str, Any],
    ace_pointer_index: dict[str, Any] | None = None,
    save_assignments: dict[str, Any] | None = None,
) -> dict[str, Any]:
    if not candidate:
        return {"available": False, "reason": "no_ace_vector_candidate"}
    if not air_wings.get("available"):
        return {"available": False, "reason": "air_wings_unavailable", "air_wings": air_wings}
    indexed_entries = []
    if ace_pointer_index and ace_pointer_index.get("available"):
        indexed_entries = [
            row
            for row in ace_pointer_index.get("entries", [])
            if isinstance(row.get("ace_ptr"), str)
            and row["ace_ptr"].startswith("0x")
            and isinstance(row.get("ace_id"), int)
        ]
    if indexed_entries:
        ace_ptrs = [int(row["ace_ptr"], 16) for row in indexed_entries]
    else:
        header = candidate.get("sample_header", {})
        size = header.get("size")
        if not isinstance(size, int) or size <= 0:
            return {"available": False, "reason": "candidate_sample_size_missing"}
        data_text = header.get("data") or header.get("begin")
        if not isinstance(data_text, str) or not data_text.startswith("0x"):
            return {"available": False, "reason": "candidate_sample_data_missing"}
        data = int(data_text, 16)
        raw = proc.try_read(data, min(MAX_VECTOR_BYTES, size * 8))
        if len(raw) < size * 8:
            return {"available": False, "reason": "ace_pointer_vector_read_truncated", "bytes_read": len(raw)}
        ace_ptrs = [
            struct.unpack_from("<Q", raw, pos)[0]
            for pos in range(0, min(len(raw), size * 8), 8)
        ]
    wing_rows = air_wings.get("air_wings", [])
    wing_ids = {row.get("wing_id") for row in wing_rows if isinstance(row.get("wing_id"), int)}
    wing_objects = {
        int(row["object"], 16)
        for row in wing_rows
        if isinstance(row.get("object"), str) and row["object"].startswith("0x")
    }
    wing_bodies = {
        int(row["body"], 16)
        for row in wing_rows
        if isinstance(row.get("body"), str) and row["body"].startswith("0x")
    }
    ace_to_wing_object_offsets: collections.Counter[int] = collections.Counter()
    ace_to_wing_body_offsets: collections.Counter[int] = collections.Counter()
    u32_wing_id_offsets: collections.Counter[int] = collections.Counter()
    examples = []
    bodies_checked = 0
    for index, ace_ptr in enumerate(ace_ptrs[:128]):
        if not is_heap_ptr(ace_ptr):
            continue
        body = proc.try_read(ace_ptr, ACE_OBJECT_ASSIGNMENT_SCAN_BYTES)
        if not body:
            continue
        bodies_checked += 1
        sample: dict[str, Any] = {"ace_index": index, "ace_ptr": hx(ace_ptr), "matches": []}
        for off in range(0, len(body) - 8, 8):
            value = struct.unpack_from("<Q", body, off)[0]
            if value in wing_objects:
                ace_to_wing_object_offsets[off] += 1
                if len(sample["matches"]) < 4:
                    sample["matches"].append({"kind": "wing_object_ptr", "offset": hx(off), "value": hx(value)})
            if value in wing_bodies:
                ace_to_wing_body_offsets[off] += 1
                if len(sample["matches"]) < 4:
                    sample["matches"].append({"kind": "wing_body_ptr", "offset": hx(off), "value": hx(value)})
        for off in range(0, len(body) - 4, 4):
            value = struct.unpack_from("<I", body, off)[0]
            if value in wing_ids:
                u32_wing_id_offsets[off] += 1
                if len(sample["matches"]) < 4:
                    sample["matches"].append({"kind": "wing_id", "offset": hx(off), "value": value})
        if sample["matches"] and len(examples) < 16:
            examples.append(sample)

    wing_to_ace_pointer_offsets: collections.Counter[str] = collections.Counter()
    wing_to_ace_id_offsets: collections.Counter[str] = collections.Counter()
    reverse_examples = []
    ace_ptr_set = {ptr for ptr in ace_ptrs if is_heap_ptr(ptr)}
    ace_ids = {row["ace_id"] for row in indexed_entries}
    if not ace_ids:
        for ace_ptr in list(ace_ptr_set)[:512]:
            ace_id = read_u32_stable(proc, ace_ptr + 0x0C)
            if ace_id:
                ace_ids.add(ace_id)
    save_by_wing: dict[tuple[int, int], int] = {}
    if save_assignments and save_assignments.get("available"):
        for row in save_assignments.get("assignments", []):
            owner = row.get("owner_tag_db_id")
            wing_id = row.get("wing_id")
            ace_id = row.get("ace_id")
            if isinstance(owner, int) and isinstance(wing_id, int) and isinstance(ace_id, int):
                save_by_wing[(owner, wing_id)] = ace_id
    save_assignment_offset_votes: collections.Counter[str] = collections.Counter()
    save_assignment_examples = []
    save_assignment_rows_checked = 0
    for wing_index, row in enumerate(wing_rows[:2048]):
        ptr_sources = []
        if isinstance(row.get("object"), str) and row["object"].startswith("0x"):
            ptr_sources.append(("wing_object", int(row["object"], 16)))
        if isinstance(row.get("body"), str) and row["body"].startswith("0x"):
            ptr_sources.append(("wing_body", int(row["body"], 16)))
        sample = {"wing_index": wing_index, "wing_id": row.get("wing_id"), "matches": []}
        wanted_save_ace_id = save_by_wing.get((row.get("owner_tag_db_id"), row.get("wing_id")))
        if wanted_save_ace_id:
            save_assignment_rows_checked += 1
        for label, base in ptr_sources:
            blob = proc.try_read(base, WING_ASSIGNMENT_SCAN_BYTES)
            if not blob:
                continue
            for off in range(0, len(blob) - 8, 8):
                value = struct.unpack_from("<Q", blob, off)[0]
                if value in ace_ptr_set:
                    key = f"{label}+{hx(off)}"
                    wing_to_ace_pointer_offsets[key] += 1
                    if len(sample["matches"]) < 4:
                        sample["matches"].append(
                            {"kind": "ace_ptr", "source": label, "offset": hx(off), "value": hx(value)}
                        )
            for off in range(0, len(blob) - 4, 4):
                value = struct.unpack_from("<I", blob, off)[0]
                if value in ace_ids:
                    key = f"{label}+{hx(off)}"
                    wing_to_ace_id_offsets[key] += 1
                    if len(sample["matches"]) < 4:
                        sample["matches"].append(
                            {"kind": "ace_id", "source": label, "offset": hx(off), "value": value}
                        )
                if wanted_save_ace_id and value == wanted_save_ace_id:
                    key = f"{label}+{hx(off)}"
                    save_assignment_offset_votes[key] += 1
                    if len(save_assignment_examples) < 16:
                        save_assignment_examples.append(
                            {
                                "wing_index": wing_index,
                                "owner_tag_db_id": row.get("owner_tag_db_id"),
                                "wing_id": row.get("wing_id"),
                                "source": label,
                                "offset": hx(off),
                                "ace_id": value,
                            }
                        )
        if sample["matches"] and len(reverse_examples) < 16:
            reverse_examples.append(sample)

    pointer_votes = (
        sum(ace_to_wing_object_offsets.values())
        + sum(ace_to_wing_body_offsets.values())
        + sum(wing_to_ace_pointer_offsets.values())
    )
    id_votes = sum(u32_wing_id_offsets.values())
    save_assignment_votes = sum(save_assignment_offset_votes.values())
    save_assignment_top_votes = save_assignment_offset_votes.most_common(1)[0][1] if save_assignment_offset_votes else 0
    save_assignment_validated = bool(
        save_assignment_top_votes >= max(4, min(save_assignment_rows_checked, 8))
        and save_assignment_top_votes >= max(1, save_assignment_rows_checked // 2)
    )
    return {
        "available": True,
        "air_wing_rows": len(wing_rows),
        "ace_pointer_rows": len(ace_ptrs),
        "ace_pointer_index_rows": len(indexed_entries),
        "ace_bodies_checked": bodies_checked,
        "ace_to_wing_object_ptr_offset_votes": [
            {"offset": hx(offset), "votes": votes}
            for offset, votes in ace_to_wing_object_offsets.most_common(12)
        ],
        "ace_to_wing_body_ptr_offset_votes": [
            {"offset": hx(offset), "votes": votes}
            for offset, votes in ace_to_wing_body_offsets.most_common(12)
        ],
        "ace_to_wing_id_offset_votes": [
            {"offset": hx(offset), "votes": votes}
            for offset, votes in u32_wing_id_offsets.most_common(12)
        ],
        "examples": examples,
        "wing_to_ace_pointer_offset_votes": [
            {"offset": offset, "votes": votes}
            for offset, votes in wing_to_ace_pointer_offsets.most_common(12)
        ],
        "wing_to_ace_id_offset_votes": [
            {"offset": offset, "votes": votes}
            for offset, votes in wing_to_ace_id_offsets.most_common(12)
        ],
        "save_assignment_validation": {
            "available": bool(save_by_wing),
            "save_assignment_rows": len(save_by_wing),
            "save_assignment_rows_checked": save_assignment_rows_checked,
            "offset_votes": [
                {"offset": offset, "votes": votes}
                for offset, votes in save_assignment_offset_votes.most_common(12)
            ],
            "examples": save_assignment_examples,
            "validated": save_assignment_validated,
        },
        "reverse_examples": reverse_examples,
        "status": (
            "validated_pointer_candidate"
            if pointer_votes
            else "validated_save_assignment_ace_id_candidate"
            if save_assignment_validated
            else "blocked_no_direct_pointer_candidate_id_only_low_entropy"
            if id_votes or save_assignment_votes
            else "blocked_no_bidirectional_assignment_candidate"
        ),
        "rejection_scope": (
            f"ace object pointer targets first 0x{ACE_OBJECT_ASSIGNMENT_SCAN_BYTES:X} bytes and "
            f"air wing object/body first 0x{WING_ASSIGNMENT_SCAN_BYTES:X} bytes; pointer joins required, "
            "u32 ace-id joins are accepted only when they match the persisted save ace assignment for "
            "the same live owner_tag_db_id/wing_id"
        ),
    }


def compact_string(value: str | None) -> bytes:
    raw = (value or "").encode("utf-8")
    if len(raw) > 0xFFFF:
        raw = raw[:0xFFFF]
    return struct.pack("<H", len(raw)) + raw


def encode_ace_roster_compact_record(
    proc: Any,
    ace_pointer_index: dict[str, Any],
    air_wings: dict[str, Any],
    wing_assignment: dict[str, Any],
    *,
    sequence: int = 1,
    game_hour_raw: int = 1942110101,
) -> tuple[bytes, dict[str, Any]]:
    entries = [
        row
        for row in ace_pointer_index.get("entries", [])
        if isinstance(row.get("tag_db_id"), int)
        and isinstance(row.get("ace_id"), int)
        and isinstance(row.get("country_ace_index"), int)
    ]
    ace_ids = {row["ace_id"] for row in entries}
    wing_by_ace: dict[int, int] = {}
    assignment_validated = wing_assignment.get("status") in {
        "validated_pointer_candidate",
        "validated_save_assignment_ace_id_candidate",
    }
    if assignment_validated and air_wings.get("available"):
        for wing in air_wings.get("air_wings", []):
            body_text = wing.get("body")
            wing_id = wing.get("wing_id")
            if not (isinstance(body_text, str) and body_text.startswith("0x") and isinstance(wing_id, int)):
                continue
            body = int(body_text, 16)
            ace_id = read_u32_stable(proc, body + WING_BODY_ASSIGNED_ACE_ID_OFF)
            if ace_id in ace_ids:
                wing_by_ace[ace_id] = wing_id

    row_blobs = []
    string_blobs = []
    identity_rows = 0
    assignment_rows = 0
    for row in sorted(entries, key=lambda item: (item["tag_db_id"], item["country_ace_index"], item["ace_id"])):
        name = row.get("name") if isinstance(row.get("name"), str) else None
        surname = row.get("surname") if isinstance(row.get("surname"), str) else None
        callsign = row.get("callsign") if isinstance(row.get("callsign"), str) else None
        wing_id = wing_by_ace.get(row["ace_id"])
        flags = 0
        if name:
            flags |= ROW_FLAG_NAME
        if surname:
            flags |= ROW_FLAG_SURNAME
        if callsign:
            flags |= ROW_FLAG_CALLSIGN
        if wing_id:
            flags |= ROW_FLAG_WING_ID
        source_mask = SOURCE_LIVE_COUNTRY_ACE_VECTOR | SOURCE_LIVE_ACE_OBJECT_LAYOUT
        if wing_id:
            source_mask |= SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT
            assignment_rows += 1
        if name or surname or callsign:
            identity_rows += 1
        row_blobs.append(
            struct.pack(
                "<IIIIIIII",
                row["tag_db_id"],
                row["ace_id"],
                flags,
                source_mask,
                row["country_ace_index"],
                wing_id or 0,
                0,
                0,
            )
        )
        string_blobs.append(compact_string(name) + compact_string(surname) + compact_string(callsign))

    row_count = len(row_blobs)
    payload = bytearray()
    payload += struct.pack(
        "<QIIIIIIIII",
        0,
        row_count,
        row_count,
        ACE_STATUS_DELTA_DATA,
        air_wings.get("topology", {}).get("containers_seen", 0) if air_wings.get("available") else 0,
        len(entries),
        identity_rows,
        assignment_rows,
        0,
        0,
    )
    payload += struct.pack("<I", 0)
    for blob in row_blobs:
        payload += blob
    for blob in string_blobs:
        payload += blob

    record = bytearray()
    record += COMPACT_RECORD_MAGIC
    record += struct.pack("<H", COMPACT_RECORD_VERSION)
    record += struct.pack("<H", COMPACT_RECORD_HEADER_LEN)
    record += struct.pack("<Q", sequence)
    record += struct.pack("<i", game_hour_raw)
    record += struct.pack("<H", DOMAIN_ID)
    record += struct.pack("<H", COMPACT_RECORD_TYPE_ACE_ROSTER_DATA)
    record += struct.pack("<I", len(payload))
    record += struct.pack("<I", 0)
    record += payload

    return bytes(record), {
        "available": True,
        "record_bytes": len(record),
        "payload_bytes": len(payload),
        "row_count": row_count,
        "identity_rows": identity_rows,
        "wing_assignment_rows": assignment_rows,
        "observed_ace_rows": len(entries),
        "assignment_validated": assignment_validated,
        "assignment_source": "wing_body+0x2DC",
        "sample": [
            {
                "tag_db_id": row["tag_db_id"],
                "ace_id": row["ace_id"],
                "country_ace_index": row["country_ace_index"],
                "wing_id": wing_by_ace.get(row["ace_id"]),
                "name": row.get("name"),
                "surname": row.get("surname"),
                "callsign": row.get("callsign"),
            }
            for row in entries[:16]
        ],
    }


def build_probe(args: argparse.Namespace) -> dict[str, Any]:
    if winmem is None:
        return {"completion_status": "NOT_COMPLETE", "available": False, "error": "winmem_unavailable"}
    pid = args.pid or winmem.find_pid("hoi4.exe")
    if not pid:
        return {"completion_status": "NOT_COMPLETE", "available": False, "error": "hoi4_process_not_found"}
    oracle = load_ace_oracle(Path(args.abilities))
    proc = winmem.Process(pid, "hoi4.exe")
    try:
        base = proc.module_base
        fp = proc.fingerprint()
        game_state = read_u64_stable(proc, base + C_GAME_STATE_GLOBAL)
        country_database = read_u64_stable(proc, base + COUNTRY_DATABASE_GLOBAL)
        _, tag_by_id, tag_meta = country_tag_table(proc, country_database)
        tag_to_id = {tag: tag_db_id for tag_db_id, tag in tag_by_id.items()}
        rows, country_meta = live_country_rows(proc, base, game_state or 0, tag_by_id)
        count_candidates = score_ace_count_offsets(proc, rows, oracle)
        vector_candidates = scan_vector_candidates(proc, rows, oracle)
        historical_vector_probe = historical_roster_header_probe(proc, rows, oracle)
        ace_pointer_index = collect_live_ace_pointer_index(proc, rows)
        save_assignment_oracle = load_save_air_wing_ace_assignments(
            Path(args.melted_save) if args.melted_save else None,
            tag_to_id,
        )
        best_count = count_candidates[0] if count_candidates else None
        best_strict_vector = vector_candidates[0] if vector_candidates else None
        best_vector = best_strict_vector or historical_vector_probe.get("candidate")
        air_wings = enumerate_air_wings(pid)
        live_clock: dict[str, Any] = {}
        if isinstance(air_wings.get("root"), dict):
            live_clock = {
                "current_game_date_raw": air_wings["root"].get("current_game_date_raw"),
                "start_date_raw": air_wings["root"].get("start_date_raw"),
            }
        wing_assignment = scan_wing_assignment_candidate(
            proc,
            best_vector,
            air_wings,
            ace_pointer_index,
            save_assignment_oracle,
        )
        compact_record_artifact: dict[str, Any] = {"available": False, "reason": "compact_record_out_not_requested"}
        if args.compact_record_out:
            game_hour_raw = live_clock.get("current_game_date_raw")
            record, compact_summary = encode_ace_roster_compact_record(
                proc,
                ace_pointer_index,
                air_wings,
                wing_assignment,
                game_hour_raw=game_hour_raw if isinstance(game_hour_raw, int) else 1942110101,
            )
            compact_path = Path(args.compact_record_out)
            compact_path.parent.mkdir(parents=True, exist_ok=True)
            compact_path.write_bytes(record)
            compact_record_artifact = {
                "available": True,
                "path": str(compact_path),
                **compact_summary,
            }
        ace_tags_total = sum(1 for payload in oracle["tags"].values() if payload["count"] > 0)
        vector_payload = best_vector.get("payload_evidence", {}) if isinstance(best_vector, dict) else {}
        strict_match_ok = bool(
            best_strict_vector
            and best_strict_vector["matched_ace_tags"] >= max(4, ace_tags_total - 1)
        )
        live_match_ok = bool(
            isinstance(best_vector, dict)
            and best_vector.get("live_count_nonzero_tags", 0) >= 4
            and best_vector.get("live_count_matched_tags", 0)
            >= max(4, best_vector.get("live_count_nonzero_tags", 0) - 1)
        )
        live_index_identity_rows = sum(
            1
            for row in ace_pointer_index.get("entries", [])
            if row.get("name") or row.get("surname") or row.get("callsign")
        )
        live_index_validated = bool(
            ace_pointer_index.get("available")
            and ace_pointer_index.get("country_rows", 0) >= max(4, ace_tags_total - 1)
            and ace_pointer_index.get("entry_count", 0) >= live_index_identity_rows
            and live_index_identity_rows > 0
            and not ace_pointer_index.get("duplicate_ace_ids")
        )
        ace_route_validated = bool(
            (
                best_vector
                and (strict_match_ok or live_match_ok)
                and (
                    vector_payload.get("ace_id_hits")
                    or vector_payload.get("pointer_target_ace_id_hits")
                )
            )
            or live_index_validated
        )
        ace_assignment_validated = wing_assignment.get("status") in {
            "validated_pointer_candidate",
            "validated_save_assignment_ace_id_candidate",
        }
        route_rejected = bool(best_count and not vector_candidates and not ace_route_validated)
        conclusion = (
            "ace_roster_identity_and_wing_assignment_roots_validated_needs_compact_integration"
            if ace_route_validated and ace_assignment_validated
            else
            "ace_roster_vector_identity_root_validated_assignment_pending"
            if ace_route_validated
            else "country_object_ace_count_validated_but_roster_vector_not_found"
            if route_rejected
            else "ace_count_anchor_not_validated"
            if not best_count
            else "ace_roster_vector_candidates_inconclusive"
        )
        return {
            "schema": "air_variant_archetypes_live_ace_root_candidate_probe_v2",
            "generated_utc": utc_now(),
            "completion_status": "NOT_COMPLETE",
            "backlog_reduced": bool(best_count or route_rejected or ace_route_validated),
            "exact_backlog_item_removed_or_narrowed": (
                "Validated current-build country ace_count offset and scanned bounded CCountry object "
                "vector-header candidates for direct ace roster roots. The dedicated historical-header "
                "probe validates CCountry+0x12D8 against live CCountry+0x12E4 ace_count before using "
                "save-oracle identity anchors."
            ),
            "available": True,
            "pid": pid,
            "module": {
                "base": hx(base),
                "path": proc.module_path,
                "sha256": fp.get("sha256"),
                "sha256_expected": EXPECTED_SHA256,
                "sha256_ok": (fp.get("sha256") or "").lower() == EXPECTED_SHA256,
            },
            "roots": {
                "game_state_global": hx(base + C_GAME_STATE_GLOBAL),
                "game_state": hx(game_state),
                "country_database_global": hx(base + COUNTRY_DATABASE_GLOBAL),
                "country_database": hx(country_database),
                "country_vtable": hx(base + COUNTRY_VTABLE_RVA),
            },
            "tag_table": tag_meta,
            "country_table": country_meta,
            "country_rows": {
                "validated_rows": len(rows),
                "tag_matches_index_rows": sum(1 for row in rows if row.get("tag_matches_index")),
                "sample": [
                    {
                        "tag": row.get("tag"),
                        "tag_db_id": row.get("tag_db_id"),
                        "object_ptr": hx(row.get("object_ptr")),
                    }
                    for row in rows[:12]
                ],
            },
            "ace_oracle": {
                "source": oracle["source"],
                "save_date": oracle["save_date"],
                "stats": oracle["stats"],
                "ace_tags": {
                    tag: {"count": payload["count"], "ace_id_sample": payload["ace_ids"][:8]}
                    for tag, payload in sorted(oracle["tags"].items())
                    if payload["count"] > 0
                },
            },
            "ace_count_offset_candidates": count_candidates,
            "best_ace_count_offset": best_count,
            "ace_roster_vector_candidates": vector_candidates,
            "historical_ace_roster_vector_probe": historical_vector_probe,
            "live_ace_pointer_index": {
                key: value
                for key, value in ace_pointer_index.items()
                if key != "entries"
            },
            "save_air_wing_ace_assignment_oracle": {
                key: value
                for key, value in save_assignment_oracle.items()
                if key != "assignments"
            },
            "ace_roster_compact_record": compact_record_artifact,
            "best_ace_roster_vector_candidate": best_vector,
            "air_wing_enumeration": {
                "available": air_wings.get("available"),
                "topology": air_wings.get("topology"),
                "air_wing_rows": len(air_wings.get("air_wings", []) or []),
            },
            "live_clock": live_clock,
            "ace_wing_assignment_probe": wing_assignment,
            "conclusion": conclusion,
            "candidate_rejections_or_blockers": {
                "historical_16022851_ace_count_offset": (
                    "validated_current_build"
                    if best_count and best_count.get("offset") == hx(HISTORICAL_ACE_COUNT_OFF)
                    else "not_top_current_build_candidate"
                ),
                "country_object_adjacent_roster_vector_route": (
                    "validated_candidate"
                    if ace_route_validated
                    else "rejected_no_count_matching_vector_headers"
                    if route_rejected
                    else "blocked_inconclusive_vector_headers"
                ),
                "ace_wing_assignment_from_ace_object": wing_assignment.get("status"),
            },
            "compact_schema_record_changes": (
                "lane-local ace roster H4CR record emitted with domain 46 and record_type 0xACE1"
                if compact_record_artifact.get("available")
                else "none; probe only"
            ),
            "rust_decoder_gate_changes": "lane-local Rust ace roster gate decodes record_type 0xACE1",
            "dll_only": False,
            "compact_stream": bool(compact_record_artifact.get("available")),
            "rust_decoded": False,
            "hoi4replay_verified": False,
            "speed_five_current_equivalent_hourly_or_lower": False,
            "zero_skipped_hours": False,
            "partial_source_worker_bridge_failures": False,
        }
    finally:
        proc.close()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--abilities", required=True)
    parser.add_argument("--out-dir", required=True)
    parser.add_argument("--melted-save")
    parser.add_argument("--compact-record-out")
    parser.add_argument("--pid", type=int)
    args = parser.parse_args()
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    proof = build_probe(args)
    out_path = out_dir / "ace_root_candidate_probe.json"
    out_path.write_text(json.dumps(proof, indent=2, sort_keys=True), encoding="utf-8")
    print(
        json.dumps(
            {
                "out": str(out_path),
                "completion_status": proof.get("completion_status"),
                "backlog_reduced": proof.get("backlog_reduced"),
                "conclusion": proof.get("conclusion"),
                "best_ace_count_offset": proof.get("best_ace_count_offset", {}).get("offset")
                if isinstance(proof.get("best_ace_count_offset"), dict)
                else None,
                "vector_candidates": len(proof.get("ace_roster_vector_candidates", []) or []),
                "historical_vector_status": proof.get("historical_ace_roster_vector_probe", {}).get("status")
                if isinstance(proof.get("historical_ace_roster_vector_probe"), dict)
                else None,
                "assignment_status": proof.get("ace_wing_assignment_probe", {}).get("status")
                if isinstance(proof.get("ace_wing_assignment_probe"), dict)
                else None,
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0 if proof.get("available") else 1


if __name__ == "__main__":
    raise SystemExit(main())
