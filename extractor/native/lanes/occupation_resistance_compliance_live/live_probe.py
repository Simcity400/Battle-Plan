#!/usr/bin/env python3
"""Lane-local live proof for occupation/resistance/compliance state rows.

This is a read-only proof/extraction module. It mirrors the production collector
root shape for this lane:

    hoi4.exe -> CGameState -> CProvince array -> CProvince.state -> CState

The hot-loop integration target is still the DLL collector. This script exists
to validate exact roots and generate lane proof artifacts without editing shared
collector or recorder entrypoints.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import time
from datetime import datetime
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO_ROOT / "extractor" / "memory_reader"))
sys.path.insert(0, str(REPO_ROOT / "extractor" / "memory_reader" / "live_layers"))

import tag_table  # noqa: E402
import winmem  # noqa: E402

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

DOMAIN_ID = 52
DOMAIN_NAME = "occupation_resistance_compliance_live"
FIXED_SCALE_RAW = 100_000
HEADER_LEN = 52
ROW_LEN = 64

COMPACT_RECORD_MAGIC = b"H4CR"
COMPACT_RECORD_VERSION = 1
COMPACT_RECORD_HEADER_LEN = 32
COMPACT_RECORD_TYPE_DOMAIN_PERF = 1
COMPACT_RECORD_TYPE_FRAME_END = 2
COMPACT_RECORD_TYPE_DOMAIN_DATA = 3
COMPACT_DOMAIN_FRAME = 1000

STATUS_SOURCE_UNAVAILABLE = 1 << 8
STATUS_PARTIAL_DATA = 1 << 9

ROW_FLAG_OWNER = 1
ROW_FLAG_CONTROLLER = 1 << 1
ROW_FLAG_RESISTANCE = 1 << 2
ROW_FLAG_COMPLIANCE = 1 << 3
ROW_FLAG_OCCUPIED_STATE = 1 << 4
ROW_FLAG_OCCUPIED_TAG_DERIVED = 1 << 5
ROW_FLAG_OCCUPATION_LAW_TOKEN = 1 << 6
ROW_FLAG_GARRISON_TEMPLATE = 1 << 7
ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE = 1 << 8
ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE = 1 << 9
ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE = 1 << 10
ROW_FLAG_RESISTANCE_SPEED = 1 << 11
ROW_FLAG_COMPLIANCE_SPEED = 1 << 12
ROW_FLAG_OCCUPATION_STATUS_ROOT = 1 << 13
ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN = 1 << 14

CG_STATE_DATE_OFFSET = 0x3316E90
CGAMESTATE_CPROVINCE_ARRAY = 0x2B0
CGAMESTATE_CPROVINCE_COUNT = 0x2BC
CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C
GS_CURRENT_DATE_OFFSET = 0x468

STATE_VTABLE_RVA = 0x29206C0
STATE_OFF_SID = 0x58
STATE_OFF_OWNER = 0xC8
STATE_OFF_CTRL = 0xCC
STATE_OFF_RESISTANCE = 0x278
STATE_OFF_RESISTANCE_SPEED = 0x280
STATE_OFF_COMPLIANCE = 0x2A8
STATE_OFF_COMPLIANCE_SPEED = 0x2B0

CPROVINCE_VTABLE_RVA = 0x295AF68
CPROVINCE_OFF_ID = 0xA4
CPROVINCE_OFF_STATE = 0xC0

COUNTRY_VTABLE_RVA = 0x27D0E90
COUNTRY_OFF_TAG_DB_ID = 0x08
COUNTRY_OFF_OCCUPATION_STATUS = 0xFD0
COUNTRY_OFF_COLLABORATION_STATUS = 0xFD8
COUNTRY_OCCUPATION_STATUS_VTABLE_RVA = 0x296DD50
COUNTRY_OCCUPATION_STATUS_SECONDARY_VTABLE_RVA = 0x296DD88
COUNTRY_OCCUPATION_STATUS_SECONDARY_VTABLE_OFF = 0x18
COUNTRY_OCCUPATION_STATUS_DEFAULT_LAW_OFF = 0x118
COUNTRY_OCCUPATION_STATUS_GARRISON_TEMPLATE_OFF = 0x78
COUNTRY_COLLABORATION_STATUS_VTABLE_RVA = 0x29A5D58
OCCUPATION_LAW_VTABLE_RVA = 0x2928218
OCCUPATION_LAW_NAME_OFF = 0x10
DIVISION_TEMPLATE_VTABLE_RVA = 0x2938F70
DIVISION_TEMPLATE_TYPE = 52
DIVISION_TEMPLATE_OFF_TYPE = 0x08
DIVISION_TEMPLATE_OFF_ID = 0x0C

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
MIN_USABLE_STATE_ROWS = 1000
MIN_USABLE_PROVINCE_ROWS = 10000
MAX_PROVINCE_COUNT = 65536
MAX_TAG_DB_ID = 1024
MAX_ABS_FIXED_RAW = 100_000_000


def is_heap_ptr(value: int) -> bool:
    return HEAP_LO <= value < HEAP_HI and value % 8 == 0


def is_heap_address(value: int) -> bool:
    return HEAP_LO <= value < HEAP_HI


def u64_from(data: bytes, offset: int) -> int:
    return struct.unpack_from("<Q", data, offset)[0]


def i32_from(data: bytes, offset: int) -> int:
    return struct.unpack_from("<i", data, offset)[0]


def u32_from(data: bytes, offset: int) -> int:
    return struct.unpack_from("<I", data, offset)[0]


def checked_fingerprint(p) -> dict:
    fp = p.fingerprint()
    if fp.get("sha256") != EXPECTED_SHA256:
        raise RuntimeError(
            "build gate failed: live sha256 %s != %s"
            % (fp.get("sha256"), EXPECTED_SHA256)
        )
    return fp


def read_pdx_string(p, address):
    blob = p.try_read(address, 32)
    if not blob or len(blob) < 32:
        return None
    size = u64_from(blob, 0x10)
    cap = u64_from(blob, 0x18)
    if size <= 0 or size > 255:
        return None
    if cap == 15:
        raw = blob[: min(size, 16)]
    else:
        ptr = u64_from(blob, 0)
        if not is_heap_address(ptr):
            return None
        raw = p.try_read(ptr, size)
        if not raw or len(raw) < size:
            return None
    if not all(32 <= b < 127 for b in raw[:size]):
        return None
    return raw[:size].decode("latin-1")


def fnv1a32(text):
    value = 0x811C9DC5
    for byte in text.encode("utf-8"):
        value ^= byte
        value = (value * 0x01000193) & 0xFFFFFFFF
    return value or 1


def resolve_state_roots(p):
    game_state = p.u64(p.module_base + CG_STATE_DATE_OFFSET)
    if not is_heap_ptr(game_state):
        raise RuntimeError("CGameState pointer is not a heap pointer")
    province_array = p.u64(game_state + CGAMESTATE_CPROVINCE_ARRAY)
    province_count = p.u32(game_state + CGAMESTATE_CPROVINCE_COUNT)
    if not is_heap_ptr(province_array):
        raise RuntimeError("CGameState CProvince array pointer is invalid")
    if province_count <= MIN_USABLE_PROVINCE_ROWS or province_count > MAX_PROVINCE_COUNT:
        raise RuntimeError("CProvince count is outside lane bounds: %d" % province_count)

    province_ptrs = p.read(province_array, province_count * 8)
    cprovince_vtable = p.module_base + CPROVINCE_VTABLE_RVA
    state_vtable = p.module_base + STATE_VTABLE_RVA
    state_roots = {}
    province_rows = 0
    invalid_province_rows = 0
    invalid_state_links = 0

    for province_id in range(1, province_count):
        province_ptr = u64_from(province_ptrs, province_id * 8)
        if province_ptr == 0:
            continue
        if not is_heap_ptr(province_ptr):
            invalid_province_rows += 1
            continue
        try:
            if p.u64(province_ptr) != cprovince_vtable:
                invalid_province_rows += 1
                continue
            if p.u32(province_ptr + CPROVINCE_OFF_ID) != province_id:
                invalid_province_rows += 1
                continue
            state_ptr = p.u64(province_ptr + CPROVINCE_OFF_STATE)
        except OSError:
            invalid_province_rows += 1
            continue
        province_rows += 1
        if state_ptr == 0:
            continue
        if not is_heap_ptr(state_ptr):
            invalid_state_links += 1
            continue
        try:
            if p.u64(state_ptr) != state_vtable:
                invalid_state_links += 1
                continue
            state_id = p.u32(state_ptr + STATE_OFF_SID)
        except OSError:
            invalid_state_links += 1
            continue
        if not (1 <= state_id <= 65535):
            invalid_state_links += 1
            continue
        old = state_roots.get(state_id)
        if old is not None and old != state_ptr:
            raise RuntimeError("state_id %d resolved to conflicting CState roots" % state_id)
        state_roots[state_id] = state_ptr

    if province_rows < MIN_USABLE_PROVINCE_ROWS:
        raise RuntimeError("insufficient validated CProvince rows: %d" % province_rows)
    if len(state_roots) < MIN_USABLE_STATE_ROWS:
        raise RuntimeError("insufficient validated CState roots: %d" % len(state_roots))

    return {
        "game_state": game_state,
        "province_array": province_array,
        "province_count": province_count,
        "province_rows": province_rows,
        "invalid_province_rows": invalid_province_rows,
        "invalid_state_links": invalid_state_links,
        "state_roots": state_roots,
    }


def read_country_occupation_status(p, game_state, tbl):
    country_table = p.u64(game_state + CGAMESTATE_COUNTRY_TABLE)
    country_count = p.u32(game_state + CGAMESTATE_COUNTRY_COUNT)
    if not is_heap_ptr(country_table) or country_count == 0 or country_count > MAX_TAG_DB_ID:
        return {}, {
            "country_count": country_count,
            "valid_country_roots": 0,
            "valid_occupation_status_roots": 0,
            "valid_default_law_roots": 0,
            "valid_garrison_template_roots": 0,
            "valid_collaboration_status_roots": 0,
            "invalid_status_roots": 0,
            "law_names": {},
        }

    country_vtable = p.module_base + COUNTRY_VTABLE_RVA
    status_vtable = p.module_base + COUNTRY_OCCUPATION_STATUS_VTABLE_RVA
    status_secondary_vtable = p.module_base + COUNTRY_OCCUPATION_STATUS_SECONDARY_VTABLE_RVA
    collab_vtable = p.module_base + COUNTRY_COLLABORATION_STATUS_VTABLE_RVA
    law_vtable = p.module_base + OCCUPATION_LAW_VTABLE_RVA
    template_vtable = p.module_base + DIVISION_TEMPLATE_VTABLE_RVA
    ptrs = p.read(country_table, country_count * 8)
    status_by_tag = {}
    valid_countries = 0
    valid_status = 0
    valid_default_law = 0
    valid_garrison_template = 0
    valid_collab = 0
    invalid_status = 0
    law_names = {}

    for tag_id in range(1, country_count):
        country_ptr = u64_from(ptrs, tag_id * 8)
        if not is_heap_ptr(country_ptr):
            continue
        try:
            if p.u64(country_ptr) != country_vtable:
                continue
            if p.u32(country_ptr + COUNTRY_OFF_TAG_DB_ID) != tag_id:
                continue
            status_ptr = p.u64(country_ptr + COUNTRY_OFF_OCCUPATION_STATUS)
            collab_ptr = p.u64(country_ptr + COUNTRY_OFF_COLLABORATION_STATUS)
        except OSError:
            continue
        valid_countries += 1

        collab_valid = False
        if is_heap_ptr(collab_ptr):
            try:
                collab_valid = p.u64(collab_ptr) == collab_vtable
            except OSError:
                collab_valid = False
        if collab_valid:
            valid_collab += 1

        if not is_heap_ptr(status_ptr):
            invalid_status += 1
            continue
        try:
            if p.u64(status_ptr) != status_vtable:
                invalid_status += 1
                continue
            if (
                p.u64(status_ptr + COUNTRY_OCCUPATION_STATUS_SECONDARY_VTABLE_OFF)
                != status_secondary_vtable
            ):
                invalid_status += 1
                continue
        except OSError:
            invalid_status += 1
            continue
        valid_status += 1

        law_ptr = 0
        law_name = None
        law_token = 0
        garrison_template_ptr = 0
        garrison_template_id = 0
        try:
            candidate = p.u64(status_ptr + COUNTRY_OCCUPATION_STATUS_DEFAULT_LAW_OFF)
            if is_heap_ptr(candidate) and p.u64(candidate) == law_vtable:
                law_name = read_pdx_string(p, candidate + OCCUPATION_LAW_NAME_OFF)
                if law_name:
                    law_ptr = candidate
                    law_token = fnv1a32(law_name)
                    valid_default_law += 1
                    law_names[law_name] = law_names.get(law_name, 0) + 1
        except OSError:
            pass
        try:
            candidate = p.u64(status_ptr + COUNTRY_OCCUPATION_STATUS_GARRISON_TEMPLATE_OFF)
            if (
                is_heap_ptr(candidate)
                and p.u64(candidate) == template_vtable
                and p.u32(candidate + DIVISION_TEMPLATE_OFF_TYPE) == DIVISION_TEMPLATE_TYPE
            ):
                template_id = p.u32(candidate + DIVISION_TEMPLATE_OFF_ID)
                if template_id:
                    garrison_template_ptr = candidate
                    garrison_template_id = template_id
                    valid_garrison_template += 1
        except OSError:
            pass

        status_by_tag[tag_id] = {
            "country_ptr": country_ptr,
            "country_tag": tbl.tag(tag_id),
            "occupation_status_ptr": status_ptr,
            "collaboration_status_ptr": collab_ptr if collab_valid else 0,
            "default_law_ptr": law_ptr,
            "default_law_name": law_name,
            "default_law_token_id": law_token,
            "garrison_template_ptr": garrison_template_ptr,
            "garrison_template_id": garrison_template_id,
        }

    return status_by_tag, {
        "country_count": country_count,
        "valid_country_roots": valid_countries,
        "valid_occupation_status_roots": valid_status,
        "valid_default_law_roots": valid_default_law,
        "valid_garrison_template_roots": valid_garrison_template,
        "valid_collaboration_status_roots": valid_collab,
        "invalid_status_roots": invalid_status,
        "law_names": dict(sorted(law_names.items())),
    }


def read_state_rows(p, roots, tbl, occupation_status_by_tag):
    state_vtable = p.module_base + STATE_VTABLE_RVA
    rows = []
    invalid_states = 0
    for state_id, state_ptr in sorted(roots.items()):
        try:
            blob = p.read(state_ptr, STATE_OFF_COMPLIANCE_SPEED + 4)
        except OSError:
            invalid_states += 1
            continue
        if u64_from(blob, 0) != state_vtable or u32_from(blob, STATE_OFF_SID) != state_id:
            invalid_states += 1
            continue
        owner = u32_from(blob, STATE_OFF_OWNER)
        controller = u32_from(blob, STATE_OFF_CTRL)
        resistance_raw = i32_from(blob, STATE_OFF_RESISTANCE)
        resistance_speed_raw = i32_from(blob, STATE_OFF_RESISTANCE_SPEED)
        compliance_raw = i32_from(blob, STATE_OFF_COMPLIANCE)
        compliance_speed_raw = i32_from(blob, STATE_OFF_COMPLIANCE_SPEED)
        if owner > MAX_TAG_DB_ID or controller > MAX_TAG_DB_ID:
            invalid_states += 1
            continue
        if (
            abs(resistance_raw) > MAX_ABS_FIXED_RAW
            or abs(compliance_raw) > MAX_ABS_FIXED_RAW
            or abs(resistance_speed_raw) > MAX_ABS_FIXED_RAW
            or abs(compliance_speed_raw) > MAX_ABS_FIXED_RAW
        ):
            invalid_states += 1
            continue

        flags = (
            ROW_FLAG_RESISTANCE
            | ROW_FLAG_COMPLIANCE
            | ROW_FLAG_RESISTANCE_SPEED
            | ROW_FLAG_COMPLIANCE_SPEED
        )
        if owner:
            flags |= ROW_FLAG_OWNER
        if controller:
            flags |= ROW_FLAG_CONTROLLER
        occupied_country = 0
        if owner and controller and owner != controller:
            flags |= ROW_FLAG_OCCUPIED_STATE | ROW_FLAG_OCCUPIED_TAG_DERIVED
            occupied_country = owner
        if resistance_raw != 0 or compliance_raw != 0:
            flags |= ROW_FLAG_OCCUPIED_STATE
        if flags & ROW_FLAG_OCCUPIED_STATE:
            flags |= ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE | ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE

        occupation_law_token_id = 0
        occupation_law_name = None
        occupation_law_kind = None
        garrison_template_id = 0
        occupation_status = occupation_status_by_tag.get(controller)
        if occupation_status:
            flags |= ROW_FLAG_OCCUPATION_STATUS_ROOT
            occupation_law_token_id = occupation_status["default_law_token_id"]
            occupation_law_name = occupation_status["default_law_name"]
            if flags & ROW_FLAG_OCCUPIED_STATE:
                garrison_template_id = occupation_status.get("garrison_template_id", 0) or 0
            if occupation_law_token_id:
                flags |= ROW_FLAG_OCCUPATION_LAW_TOKEN | ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN
                occupation_law_kind = "controller_default_fallback"
            if garrison_template_id:
                flags |= ROW_FLAG_GARRISON_TEMPLATE

        rows.append(
            {
                "state_id": state_id,
                "state_ptr": state_ptr,
                "owner_tag_db_id": owner,
                "owner_tag": tbl.tag(owner) if owner else None,
                "controller_tag_db_id": controller,
                "controller_tag": tbl.tag(controller) if controller else None,
                "occupied_country_tag_db_id": occupied_country,
                "occupied_country_tag": tbl.tag(occupied_country) if occupied_country else None,
                "resistance_raw": resistance_raw,
                "resistance": resistance_raw / FIXED_SCALE_RAW,
                "compliance_raw": compliance_raw,
                "compliance": compliance_raw / FIXED_SCALE_RAW,
                "resistance_speed_raw": resistance_speed_raw,
                "resistance_speed": resistance_speed_raw / FIXED_SCALE_RAW,
                "compliance_speed_raw": compliance_speed_raw,
                "compliance_speed": compliance_speed_raw / FIXED_SCALE_RAW,
                "garrison_need_raw": 0,
                "garrison_equilibrium_raw": 0,
                "garrison_progress_raw": 0,
                "occupation_law_token_id": occupation_law_token_id,
                "occupation_law_name": occupation_law_name,
                "occupation_law_kind": occupation_law_kind,
                "garrison_template_id": garrison_template_id,
                "flags": flags,
            }
        )
    return rows, invalid_states


def encode_payload(rows, elapsed_wall_ms, status_flags, observed_state_rows):
    occupied_state_rows = sum(1 for row in rows if row["flags"] & ROW_FLAG_OCCUPIED_STATE)
    wall_badge_rows = sum(
        1
        for row in rows
        if row["flags"]
        & (
            ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE
            | ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE
            | ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE
        )
    )
    law_token_rows = sum(1 for row in rows if row["flags"] & ROW_FLAG_OCCUPATION_LAW_TOKEN)
    garrison_template_rows = sum(1 for row in rows if row["flags"] & ROW_FLAG_GARRISON_TEMPLATE)
    payload = bytearray()
    payload += struct.pack(
        "<QIIIIIIIIII",
        elapsed_wall_ms,
        len(rows),
        len(rows),
        status_flags,
        observed_state_rows,
        occupied_state_rows,
        wall_badge_rows,
        FIXED_SCALE_RAW,
        law_token_rows,
        garrison_template_rows,
        0,
    )
    payload += struct.pack("<I", 0)
    for row in rows:
        payload += struct.pack(
            "<IIIIiiiiiiiIIIII",
            row["state_id"],
            row["owner_tag_db_id"],
            row["controller_tag_db_id"],
            row["occupied_country_tag_db_id"],
            row["resistance_raw"],
            row["compliance_raw"],
            row["resistance_speed_raw"],
            row["compliance_speed_raw"],
            row["garrison_need_raw"],
            row["garrison_equilibrium_raw"],
            row["garrison_progress_raw"],
            row["occupation_law_token_id"],
            row["garrison_template_id"],
            row["flags"],
            0,
            0,
        )
    return bytes(payload)


def append_record(out, sequence, game_hour_raw, domain_id, record_type, payload):
    out += COMPACT_RECORD_MAGIC
    out += struct.pack(
        "<HHQiHHII",
        COMPACT_RECORD_VERSION,
        COMPACT_RECORD_HEADER_LEN,
        sequence,
        game_hour_raw,
        domain_id,
        record_type,
        len(payload),
        0,
    )
    out += payload


def encode_compact_stream(payload, rows, elapsed_wall_ms, read_time_us, status_flags, game_hour_raw):
    occupied_state_rows = sum(1 for row in rows if row["flags"] & ROW_FLAG_OCCUPIED_STATE)
    out = bytearray()
    perf = struct.pack(
        "<QQIIIIII",
        elapsed_wall_ms,
        read_time_us,
        len(rows),
        len(rows),
        occupied_state_rows,
        occupied_state_rows,
        status_flags,
        0,
    )
    append_record(out, 0, game_hour_raw, DOMAIN_ID, COMPACT_RECORD_TYPE_DOMAIN_PERF, perf)
    append_record(out, 1, game_hour_raw, DOMAIN_ID, COMPACT_RECORD_TYPE_DOMAIN_DATA, payload)
    frame = struct.pack("<QQIIII", elapsed_wall_ms, read_time_us, 1, 1, status_flags, 0)
    append_record(out, 2, game_hour_raw, COMPACT_DOMAIN_FRAME, COMPACT_RECORD_TYPE_FRAME_END, frame)
    return bytes(out)


def compare_oracle(rows, oracle_path):
    if not oracle_path:
        return None
    path = Path(oracle_path)
    if not path.is_file():
        return {"error": "oracle not found: %s" % path}
    data = json.loads(path.read_text(encoding="utf-8"))
    states = data.get("states", {})
    by_state = {str(row["state_id"]): row for row in rows}
    compared = 0
    exact_resistance = 0
    exact_compliance = 0
    mismatches = []
    for state_id, state in states.items():
        expected = state.get("resistance")
        if not expected:
            continue
        live = by_state.get(str(state_id))
        if not live:
            continue
        compared += 1
        want_res = int(round(float(expected.get("resistance", 0.0)) * FIXED_SCALE_RAW))
        want_comp = int(round(float(expected.get("compliance", 0.0)) * FIXED_SCALE_RAW))
        if live["resistance_raw"] == want_res:
            exact_resistance += 1
        if live["compliance_raw"] == want_comp:
            exact_compliance += 1
        if live["resistance_raw"] != want_res or live["compliance_raw"] != want_comp:
            if len(mismatches) < 12:
                mismatches.append(
                    {
                        "state_id": int(state_id),
                        "live_resistance_raw": live["resistance_raw"],
                        "oracle_resistance_raw": want_res,
                        "live_compliance_raw": live["compliance_raw"],
                        "oracle_compliance_raw": want_comp,
                    }
                )
    return {
        "oracle_path": str(path),
        "compared_occupied_states": compared,
        "exact_resistance": exact_resistance,
        "exact_compliance": exact_compliance,
        "sample_mismatches": mismatches,
    }


def scrub_rows_for_json(rows, include_addresses):
    out = []
    for row in rows:
        copy = dict(row)
        if include_addresses:
            copy["state_ptr"] = "0x%X" % copy["state_ptr"]
        else:
            copy.pop("state_ptr", None)
        out.append(copy)
    return out


def write_proof_md(out_dir, summary):
    roots = summary["roots"]
    country_status = summary.get("country_occupation_status", {})
    md = [
        "# Occupation Resistance Compliance Live Proof",
        "",
        "## Status",
        "",
        "- Domain: `%s` (`%d`)" % (DOMAIN_NAME, DOMAIN_ID),
        "- Live exe SHA-256: `%s`" % summary["process"]["sha256"],
        "- Rows emitted: `%d`" % summary["rows"]["emitted_state_rows"],
        "- Occupied rows: `%d`" % summary["rows"]["occupied_state_rows"],
        "- Wall-badged rows: `%d`" % summary["rows"]["wall_badge_rows"],
        "- Invalid state rows: `%d`" % summary["rows"]["invalid_state_rows"],
        "",
        "## Validated Roots",
        "",
        "- `hoi4.exe + 0x%X -> CGameState*`: `%s`"
        % (CG_STATE_DATE_OFFSET, roots["game_state"]),
        "- `CGameState + 0x%X -> CProvince**`: `%s`"
        % (CGAMESTATE_CPROVINCE_ARRAY, roots["province_array"]),
        "- `CGameState + 0x%X -> province_count`: `%d`"
        % (CGAMESTATE_CPROVINCE_COUNT, roots["province_count"]),
        "- `CProvince + 0x%X -> CState*`, with `CProvince` vtable `module+0x%X`"
        % (CPROVINCE_OFF_STATE, CPROVINCE_VTABLE_RVA),
        "- `CState` vtable `module+0x%X`; `state_id +0x%X`, owner `+0x%X`, controller `+0x%X`"
        % (STATE_VTABLE_RVA, STATE_OFF_SID, STATE_OFF_OWNER, STATE_OFF_CTRL),
        "- Resistance raw `CState + 0x%X` and compliance raw `CState + 0x%X`, fixed scale `%d`"
        % (STATE_OFF_RESISTANCE, STATE_OFF_COMPLIANCE, FIXED_SCALE_RAW),
        "- Resistance speed signal `CState + 0x%X` and compliance speed signal `CState + 0x%X`, fixed scale `%d`"
        % (STATE_OFF_RESISTANCE_SPEED, STATE_OFF_COMPLIANCE_SPEED, FIXED_SCALE_RAW),
        "- `CCountry + 0x%X -> CCountryOccupationStatus*`; vtables `module+0x%X` and `module+0x%X`"
        % (
            COUNTRY_OFF_OCCUPATION_STATUS,
            COUNTRY_OCCUPATION_STATUS_VTABLE_RVA,
            COUNTRY_OCCUPATION_STATUS_SECONDARY_VTABLE_RVA,
        ),
        "- `CCountryOccupationStatus + 0x%X -> COccupationLaw*`; law vtable `module+0x%X`, law id string `+0x%X`"
        % (
            COUNTRY_OCCUPATION_STATUS_DEFAULT_LAW_OFF,
            OCCUPATION_LAW_VTABLE_RVA,
            OCCUPATION_LAW_NAME_OFF,
        ),
        "- `CCountryOccupationStatus + 0x%X -> CDivisionTemplate*`; template vtable `module+0x%X`, type `+0x%X`, id `+0x%X`"
        % (
            COUNTRY_OCCUPATION_STATUS_GARRISON_TEMPLATE_OFF,
            DIVISION_TEMPLATE_VTABLE_RVA,
            DIVISION_TEMPLATE_OFF_TYPE,
            DIVISION_TEMPLATE_OFF_ID,
        ),
        "- `CCountry + 0x%X -> CCountryCollaborationStatus*`; vtable `module+0x%X`"
        % (COUNTRY_OFF_COLLABORATION_STATUS, COUNTRY_COLLABORATION_STATUS_VTABLE_RVA),
        "- Valid occupation-status roots: `%d`; default-law roots: `%d`; garrison-template roots: `%d`; collaboration-status roots: `%d`"
        % (
            country_status.get("valid_occupation_status_roots", 0),
            country_status.get("valid_default_law_roots", 0),
            country_status.get("valid_garrison_template_roots", 0),
            country_status.get("valid_collaboration_status_roots", 0),
        ),
        "",
        "## Honest Walls",
        "",
        "- Per-state occupation law override map: unavailable; rows carry only controller-country default/fallback law tokens and remain badged for per-state override.",
        "- Garrison need/equilibrium/progress: unavailable in this lane proof; occupied rows are row-badged fail-closed for those scalar values.",
        "- Government-in-exile/collaboration detail payload: collaboration-status root is validated, but active per-target data remains fixture-gated.",
    ]
    oracle = summary.get("oracle_compare")
    if oracle:
        md.extend(
            [
                "",
                "## Oracle Compare",
                "",
                "- Compared occupied states: `%s`" % oracle.get("compared_occupied_states"),
                "- Exact resistance: `%s`" % oracle.get("exact_resistance"),
                "- Exact compliance: `%s`" % oracle.get("exact_compliance"),
            ]
        )
    (out_dir / "PROOF.md").write_text("\n".join(md) + "\n", encoding="utf-8")


def run(out_dir, oracle_state_detail=None, include_addresses=False):
    t0 = time.time()
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise RuntimeError("hoi4.exe not running")
    with p:
        fp = checked_fingerprint(p)
        tbl = tag_table.TagTable.locate(p)
        if tbl is None:
            raise RuntimeError("tag table not found")
        roots = resolve_state_roots(p)
        occupation_status_by_tag, country_status_summary = read_country_occupation_status(
            p, roots["game_state"], tbl
        )
        rows, invalid_states = read_state_rows(
            p, roots["state_roots"], tbl, occupation_status_by_tag
        )
        if invalid_states:
            status_flags = STATUS_PARTIAL_DATA
        else:
            status_flags = 0
        if not rows:
            status_flags |= STATUS_SOURCE_UNAVAILABLE
        elapsed_ms = int((time.time() - t0) * 1000)
        read_time_us = int((time.time() - t0) * 1_000_000)
        try:
            game_hour_raw = p.i32(roots["game_state"] + GS_CURRENT_DATE_OFFSET)
        except OSError:
            game_hour_raw = 0

    out_dir.mkdir(parents=True, exist_ok=True)
    payload = encode_payload(rows, elapsed_ms, status_flags, len(roots["state_roots"]))
    compact_stream = encode_compact_stream(
        payload, rows, elapsed_ms, read_time_us, status_flags, game_hour_raw
    )
    occupied_state_rows = sum(1 for row in rows if row["flags"] & ROW_FLAG_OCCUPIED_STATE)
    wall_badge_rows = sum(
        1
        for row in rows
        if row["flags"]
        & (
            ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE
            | ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE
            | ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE
        )
    )
    speed_rows = sum(
        1
        for row in rows
        if row["flags"] & ROW_FLAG_RESISTANCE_SPEED
        and row["flags"] & ROW_FLAG_COMPLIANCE_SPEED
    )
    country_default_law_token_rows = sum(
        1 for row in rows if row["flags"] & ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN
    )
    summary = {
        "domain": {"id": DOMAIN_ID, "name": DOMAIN_NAME},
        "process": {
            "pid": fp.get("pid"),
            "module": fp.get("module"),
            "base": "0x%X" % fp.get("base"),
            "size": fp.get("size"),
            "sha256": fp.get("sha256"),
        },
        "roots": {
            "game_state": "0x%X" % roots["game_state"],
            "province_array": "0x%X" % roots["province_array"],
            "province_count": roots["province_count"],
            "province_rows": roots["province_rows"],
            "invalid_province_rows": roots["invalid_province_rows"],
            "invalid_state_links": roots["invalid_state_links"],
            "state_roots": len(roots["state_roots"]),
            "sample_state_roots": {
                str(k): "0x%X" % roots["state_roots"][k]
                for k in sorted(roots["state_roots"])[:8]
            },
        },
        "offsets": {
            "CG_STATE_DATE_OFFSET": "0x%X" % CG_STATE_DATE_OFFSET,
            "CGAMESTATE_CPROVINCE_ARRAY": "0x%X" % CGAMESTATE_CPROVINCE_ARRAY,
            "CGAMESTATE_CPROVINCE_COUNT": "0x%X" % CGAMESTATE_CPROVINCE_COUNT,
            "CPROVINCE_OFF_STATE": "0x%X" % CPROVINCE_OFF_STATE,
            "STATE_OFF_SID": "0x%X" % STATE_OFF_SID,
            "STATE_OFF_OWNER": "0x%X" % STATE_OFF_OWNER,
            "STATE_OFF_CTRL": "0x%X" % STATE_OFF_CTRL,
            "STATE_OFF_RESISTANCE": "0x%X" % STATE_OFF_RESISTANCE,
            "STATE_OFF_RESISTANCE_SPEED": "0x%X" % STATE_OFF_RESISTANCE_SPEED,
            "STATE_OFF_COMPLIANCE": "0x%X" % STATE_OFF_COMPLIANCE,
            "STATE_OFF_COMPLIANCE_SPEED": "0x%X" % STATE_OFF_COMPLIANCE_SPEED,
            "CGAMESTATE_COUNTRY_TABLE": "0x%X" % CGAMESTATE_COUNTRY_TABLE,
            "CGAMESTATE_COUNTRY_COUNT": "0x%X" % CGAMESTATE_COUNTRY_COUNT,
            "COUNTRY_VTABLE_RVA": "0x%X" % COUNTRY_VTABLE_RVA,
            "COUNTRY_OFF_OCCUPATION_STATUS": "0x%X" % COUNTRY_OFF_OCCUPATION_STATUS,
            "COUNTRY_OCCUPATION_STATUS_VTABLE_RVA": "0x%X"
            % COUNTRY_OCCUPATION_STATUS_VTABLE_RVA,
            "COUNTRY_OCCUPATION_STATUS_SECONDARY_VTABLE_RVA": "0x%X"
            % COUNTRY_OCCUPATION_STATUS_SECONDARY_VTABLE_RVA,
            "COUNTRY_OCCUPATION_STATUS_DEFAULT_LAW_OFF": "0x%X"
            % COUNTRY_OCCUPATION_STATUS_DEFAULT_LAW_OFF,
            "COUNTRY_OCCUPATION_STATUS_GARRISON_TEMPLATE_OFF": "0x%X"
            % COUNTRY_OCCUPATION_STATUS_GARRISON_TEMPLATE_OFF,
            "OCCUPATION_LAW_VTABLE_RVA": "0x%X" % OCCUPATION_LAW_VTABLE_RVA,
            "OCCUPATION_LAW_NAME_OFF": "0x%X" % OCCUPATION_LAW_NAME_OFF,
            "DIVISION_TEMPLATE_VTABLE_RVA": "0x%X" % DIVISION_TEMPLATE_VTABLE_RVA,
            "DIVISION_TEMPLATE_OFF_TYPE": "0x%X" % DIVISION_TEMPLATE_OFF_TYPE,
            "DIVISION_TEMPLATE_OFF_ID": "0x%X" % DIVISION_TEMPLATE_OFF_ID,
            "COUNTRY_OFF_COLLABORATION_STATUS": "0x%X" % COUNTRY_OFF_COLLABORATION_STATUS,
            "COUNTRY_COLLABORATION_STATUS_VTABLE_RVA": "0x%X"
            % COUNTRY_COLLABORATION_STATUS_VTABLE_RVA,
            "FIXED_SCALE_RAW": FIXED_SCALE_RAW,
        },
        "country_occupation_status": country_status_summary,
        "rows": {
            "emitted_state_rows": len(rows),
            "occupied_state_rows": occupied_state_rows,
            "wall_badge_rows": wall_badge_rows,
            "speed_rows": speed_rows,
            "law_token_rows": sum(
                1 for row in rows if row["flags"] & ROW_FLAG_OCCUPATION_LAW_TOKEN
            ),
            "country_default_law_token_rows": country_default_law_token_rows,
            "garrison_template_rows": sum(
                1 for row in rows if row["flags"] & ROW_FLAG_GARRISON_TEMPLATE
            ),
            "invalid_state_rows": invalid_states,
            "status_flags": status_flags,
        },
        "compact": {
            "payload_file": "occupation_resistance_compliance_live_payload.bin",
            "payload_bytes": len(payload),
            "collector_stream_file": "collector_stream.bin",
            "collector_stream_bytes": len(compact_stream),
            "record_magic": "H4CR",
            "domain_id": DOMAIN_ID,
            "record_types": ["domain_perf", "domain_data", "frame_end"],
        },
        "oracle_compare": compare_oracle(rows, oracle_state_detail),
    }
    (out_dir / "occupation_resistance_compliance_live_payload.bin").write_bytes(payload)
    (out_dir / "collector_stream.bin").write_bytes(compact_stream)
    (out_dir / "live_rows.json").write_text(
        json.dumps(scrub_rows_for_json(rows, include_addresses), indent=2),
        encoding="utf-8",
    )
    (out_dir / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    write_proof_md(out_dir, summary)
    return summary


def default_out_dir():
    stamp = datetime.utcnow().strftime("%Y%m%dT%H%M%SZ")
    return REPO_ROOT / "extractor/native" / "runs" / (
        "occupation_resistance_compliance_live_%s" % stamp
    )


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out-dir", default=None)
    parser.add_argument("--oracle-state-detail", default=None)
    parser.add_argument("--include-addresses", action="store_true")
    args = parser.parse_args(argv)
    out_dir = Path(args.out_dir) if args.out_dir else default_out_dir()
    summary = run(out_dir, args.oracle_state_detail, args.include_addresses)
    print(json.dumps({"out_dir": str(out_dir), "summary": summary}, indent=2))


if __name__ == "__main__":
    raise SystemExit(main())
