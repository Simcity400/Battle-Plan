"""Live root proof for the characters/leaders/cabinet lane.

This is a development/proof harness, not a runtime capture path. It reuses the
existing read-only memory_reader cracks to validate the production roots on the
currently running hoi4.exe and writes a lane-local proof artifact under
replay_native/runs/.
"""

from __future__ import annotations

import argparse
import collections
import datetime as _dt
import json
import os
import struct
import sys
import time
from pathlib import Path


REPO = Path(__file__).resolve().parents[3]
MEMORY_READER = REPO / "memory_reader"
for path in (str(REPO), str(MEMORY_READER)):
    if path not in sys.path:
        sys.path.insert(0, path)

import characters_live as CL  # noqa: E402
import country_static_live as CSTAT  # noqa: E402
import defnames_live as DN  # noqa: E402
import leadership_deep_live as LD  # noqa: E402
import leader_skill_traits_live as LST  # noqa: E402
import locate as LOC  # noqa: E402
import politics_live as POL  # noqa: E402
import politics_residuals_live as PRES  # noqa: E402
import winmem  # noqa: E402
from live_layers import tag_table  # noqa: E402


def _utc_stamp() -> str:
    return _dt.datetime.now(_dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def _write_json(path: Path, obj: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def _sample(seq, limit: int):
    if isinstance(seq, dict):
        return dict(list(seq.items())[:limit])
    return list(seq)[:limit]


def _tag_inverse(p) -> dict[str, int]:
    try:
        tbl = tag_table.TagTable.locate(p)
        if tbl is None:
            return {}
        return {tag: tag_id for tag_id, tag in (tbl.as_map() or {}).items() if tag and tag != "---"}
    except Exception:
        return {}


def _heap_ok(value: int | None) -> bool:
    return value is not None and POL.HEAP_LO <= int(value) < POL.HEAP_HI


class _CountryRecordBridge:
    def __init__(self, rec0: int, count: int, tbl: tag_table.TagTable, base: int, source: str):
        self.rec0 = rec0
        self.count = count
        self.tbl = tbl
        self.base = base
        self.source = source

    def revalidate(self) -> bool:
        return bool(POL._verify_rec0(self.tbl.p, self.rec0) and self.tbl.revalidate())


def _bridge_info(bridge, idx_to_tag: dict[int, str], tag_to_id: dict[str, int], source: str, stale_note: str) -> dict:
    return {
        "source": source,
        "rec0": f"0x{bridge.rec0:X}",
        "count": int(bridge.count),
        "bridge_base": f"0x{int(getattr(bridge, 'base', 0)):X}",
        "resolved_countries": len(idx_to_tag),
        "revalidate": bool(bridge.revalidate()),
        "sample": _sample(
            {
                tag: {
                    "tag_db_id": tag_to_id[tag],
                    "running_index": running_index,
                    "record": f"0x{bridge.rec0 + running_index * POL.REC:X}",
                }
                for running_index, tag in sorted(idx_to_tag.items())
            },
            8,
        ),
        "stale_route_rejected": stale_note,
    }


def _resolve_live_country_bridge(p) -> tuple[object, dict[int, str], dict[str, int], dict]:
    """Resolve a save-free country record bridge and expose the mapping politics readers need."""
    tbl = tag_table.TagTable.locate(p)
    if tbl is None:
        raise RuntimeError("TagTable.locate returned None")
    located = LOC.locate(p)
    if located is None:
        raise RuntimeError("locate.locate returned None")
    rec0, begin_field, count = located
    idx_to_tag = CSTAT.bridge_via_tag_table(p, rec0, count, tbl)
    tag_to_id = {tag: tag_id for tag_id, tag in tbl.as_map().items() if tag and tag != tag_table.SENTINEL}
    tag_to_id = {tag: int(tag_id) for tag, tag_id in tag_to_id.items() if tag in set(idx_to_tag.values())}
    if len(idx_to_tag) < 40:
        raise RuntimeError(
            f"fallback locate.locate + CCountry+0x08 tag bridge resolved only {len(idx_to_tag)} countries"
        )
    fallback = _CountryRecordBridge(
        rec0=int(rec0),
        count=int(count),
        tbl=tbl,
        base=int(begin_field),
        source="locate.locate + country_static_live.bridge_via_tag_table",
    )
    info = _bridge_info(
        fallback,
        idx_to_tag,
        tag_to_id,
        fallback.source,
        "CountryIndexBridge tag-db-id object-array fingerprint failed; fallback uses live country records and CCountry+0x08 tag_db_id, not save PP",
    )
    info["country_index_bridge_result"] = "bypassed; prior v1 proof rejected it with CountryIndexBridge.resolve returned None"
    return fallback, idx_to_tag, tag_to_id, info


def _trait_membership_fingerprint(cl: CL.CharactersLiveReader, leader_addr: int, trait_vtable_abs: int | None) -> tuple[bool, dict]:
    """Validate the assigned-trait vector this lane consumes, not the stale +0x288 XP bag."""
    try:
        data1 = cl.u64(leader_addr + 0xDC8)
        cap1 = cl.u32(leader_addr + 0xDD0)
        size1 = cl.u32(leader_addr + 0xDD4)
        data2 = cl.u64(leader_addr + 0xDC8)
        cap2 = cl.u32(leader_addr + 0xDD0)
        size2 = cl.u32(leader_addr + 0xDD4)
    except Exception:
        return False, {"error": "trait vector header unreadable"}
    if (data1, cap1, size1) != (data2, cap2, size2):
        return False, {"error": "trait vector header changed between reads"}
    if cap1 is None or size1 is None or size1 > cap1 or size1 > 64:
        return False, {"error": f"trait vector size/cap invalid size={size1} cap={cap1}"}
    if size1 and not _heap_ok(data1):
        return False, {"error": f"trait vector data pointer invalid data=0x{int(data1 or 0):X}"}
    trait_ptrs = []
    for ordinal in range(int(size1 or 0)):
        ptr = cl.u64(int(data1) + ordinal * 8)
        if not _heap_ok(ptr):
            return False, {"error": f"trait pointer {ordinal} invalid ptr=0x{int(ptr or 0):X}"}
        if trait_vtable_abs is not None and cl.u64(int(ptr)) != trait_vtable_abs:
            return False, {"error": f"trait pointer {ordinal} vtable mismatch ptr=0x{int(ptr):X}"}
        trait_ptrs.append(int(ptr))
    return True, {
        "data": f"0x{int(data1 or 0):X}",
        "capacity": int(cap1 or 0),
        "size": int(size1 or 0),
        "trait_ptr_sample": [f"0x{ptr:X}" for ptr in trait_ptrs[:8]],
    }


def _filter_lane_addr_sets(cl: CL.CharactersLiveReader, addr_sets: dict, sample_limit: int) -> tuple[dict, dict]:
    """Drop aligned vtable occurrences that are not object starts before pinning.

    The upstream scanner returns every heap qword equal to a vtable. On current
    build that includes a small number of interior/reference hits for CCharacter.
    Production pins must start from validated object bodies only.
    """
    filtered = {"army": [], "navy": [], "char": [], "country_leader": [], "_ids": {}}
    rejected = collections.Counter()
    samples = []

    def reject(kind: str, addr: int, reason: str) -> None:
        rejected[kind] += 1
        if len(samples) < sample_limit:
            samples.append({"kind": kind, "addr": f"0x{addr:X}", "reason": reason})

    for key, vt_name, want_type in (
        ("army", "CArmyLeader", CL.UNIT_LEADER_TYPE),
        ("navy", "CNavyLeader", CL.UNIT_LEADER_TYPE),
    ):
        for addr in addr_sets.get(key, ()):
            if cl.u64(addr + CL.OFF_VT) != cl.vt[vt_name]:
                reject(key, addr, "vtable mismatch")
                continue
            if cl.u32(addr + CL.OFF_TYPE) != want_type:
                reject(key, addr, "type guard mismatch")
                continue
            unit_id = cl.u32(addr + CL.OFF_ID)
            if unit_id is None:
                reject(key, addr, "unit_leader_id unreadable")
                continue
            filtered[key].append(addr)
            filtered["_ids"][addr] = unit_id

    for addr in addr_sets.get("char", ()):
        if cl.u64(addr + CL.OFF_VT) != cl.vt["CCharacter"]:
            reject("char", addr, "vtable mismatch")
            continue
        if cl.u32(addr + CL.OFF_TYPE) != CL.CHARACTER_TYPE:
            reject("char", addr, "type guard mismatch")
            continue
        char_id = cl.u32(addr + CL.OFF_ID)
        if char_id is None:
            reject("char", addr, "char_id unreadable")
            continue
        filtered["char"].append(addr)
        filtered["_ids"][addr] = char_id

    for addr in addr_sets.get("country_leader", ()):
        if cl.u64(addr + CL.OFF_VT) != cl.vt["CCountryLeader"]:
            reject("country_leader", addr, "vtable mismatch")
            continue
        char_ptr = cl.u64(addr + CL.OFF_CL_CHAR_PTR)
        if not _heap_ok(char_ptr):
            reject("country_leader", addr, "character pointer invalid")
            continue
        if cl.u64(int(char_ptr) + CL.OFF_VT) != cl.vt["CCharacter"]:
            reject("country_leader", addr, "role character vtable mismatch")
            continue
        if cl.u32(int(char_ptr) + CL.OFF_TYPE) != CL.CHARACTER_TYPE:
            reject("country_leader", addr, "role character type mismatch")
            continue
        filtered["country_leader"].append(addr)
        filtered["_ids"][addr] = char_ptr

    info = {
        "source_counts": {
            "army": len(addr_sets.get("army") or []),
            "navy": len(addr_sets.get("navy") or []),
            "characters": len(addr_sets.get("char") or []),
            "country_leader_roles": len(addr_sets.get("country_leader") or []),
        },
        "filtered_counts": {
            "army": len(filtered["army"]),
            "navy": len(filtered["navy"]),
            "characters": len(filtered["char"]),
            "country_leader_roles": len(filtered["country_leader"]),
        },
        "rejected_counts": dict(rejected),
        "rejected_sample": samples,
    }
    return filtered, info


def _validate_lane_warm_pins(
    cl: CL.CharactersLiveReader,
    addr_sets: dict,
    trait_vtable_abs: int | None,
    sample_limit: int,
) -> dict:
    """Lane-local warm-pin gate using current RTTI roots and production-consumed fields.

    This deliberately does not call characters_live.validate_addrs as the authority:
    that legacy helper fingerprints leader+0x288, which prior work rejected as the
    assigned-trait membership source. The lane gate validates addresses plus the
    current assigned-trait vector at +0xDC8.
    """
    ids = addr_sets.get("_ids") or {}
    failures = []
    checked = collections.Counter()
    trait_vectors = collections.Counter()

    def fail(kind: str, addr: int, reason: str) -> None:
        if len(failures) < 20:
            failures.append({"kind": kind, "addr": f"0x{addr:X}", "reason": reason})

    for key, want_vt, want_type in (
        ("army", cl.vt["CArmyLeader"], CL.UNIT_LEADER_TYPE),
        ("navy", cl.vt["CNavyLeader"], CL.UNIT_LEADER_TYPE),
    ):
        for addr in addr_sets.get(key, ()):
            checked[key] += 1
            if cl.u64(addr + CL.OFF_VT) != want_vt:
                fail(key, addr, "vtable mismatch")
                continue
            if cl.u32(addr + CL.OFF_TYPE) != want_type:
                fail(key, addr, "type guard mismatch")
                continue
            unit_id = cl.u32(addr + CL.OFF_ID)
            if unit_id is None or (addr in ids and unit_id != ids[addr]):
                fail(key, addr, "unit_leader_id ABA guard mismatch")
                continue
            if cl.u32(addr + 0xF48) != CL.CHARACTER_TYPE or cl.u32(addr + CL.OFF_CHARID_IN_L) is None:
                fail(key, addr, "embedded character header mismatch")
                continue
            skill_box = cl.u64(addr + 0xE60)
            if not _heap_ok(skill_box):
                fail(key, addr, "skill box pointer invalid")
                continue
            skill = cl.u32(int(skill_box) + 0x1B8)
            if skill is None or skill > 15:
                fail(key, addr, f"skill value invalid {skill}")
                continue
            ok, fp = _trait_membership_fingerprint(cl, addr, trait_vtable_abs)
            if not ok:
                fail(key, addr, fp.get("error", "trait membership validation failed"))
                continue
            trait_vectors["leaders"] += 1
            trait_vectors["trait_rows"] += int(fp.get("size") or 0)

    for addr in addr_sets.get("char", ()):
        checked["char"] += 1
        if cl.u64(addr + CL.OFF_VT) != cl.vt["CCharacter"]:
            fail("char", addr, "vtable mismatch")
            continue
        if cl.u32(addr + CL.OFF_TYPE) != CL.CHARACTER_TYPE:
            fail("char", addr, "type guard mismatch")
            continue
        char_id = cl.u32(addr + CL.OFF_ID)
        if char_id is None or (addr in ids and char_id != ids[addr]):
            fail("char", addr, "char_id ABA guard mismatch")
            continue

    for addr in addr_sets.get("country_leader", ()):
        checked["country_leader"] += 1
        if cl.u64(addr + CL.OFF_VT) != cl.vt["CCountryLeader"]:
            fail("country_leader", addr, "vtable mismatch")
            continue
        char_ptr = cl.u64(addr + CL.OFF_CL_CHAR_PTR)
        if not _heap_ok(char_ptr):
            fail("country_leader", addr, "character pointer invalid")
            continue
        if addr in ids and char_ptr != ids[addr]:
            fail("country_leader", addr, "role-character pointer ABA guard mismatch")
            continue
        if cl.u64(int(char_ptr) + CL.OFF_VT) != cl.vt["CCharacter"]:
            fail("country_leader", addr, "role character vtable mismatch")
            continue
        if cl.u32(int(char_ptr) + CL.OFF_TYPE) != CL.CHARACTER_TYPE:
            fail("country_leader", addr, "role character type mismatch")
            continue

    return {
        "ok": not failures,
        "checked_counts": dict(checked),
        "trait_membership_vectors_checked": int(trait_vectors["leaders"]),
        "trait_membership_rows_seen": int(trait_vectors["trait_rows"]),
        "validator": "lane-local current RTTI vtable/type/id/backlink/+0xDC8 assigned-trait gate",
        "legacy_dependency_bypassed": "characters_live.validate_addrs fingerprints leader+0x288; this lane validates the assigned-trait vector at +0xDC8 instead",
        "failures": _sample(failures, sample_limit),
    }


def _read_skill_traits_from_located_addresses(
    cl: CL.CharactersLiveReader,
    p,
    addr_sets: dict,
    trait_vtable_abs: int | None = None,
    include_rows: bool = False,
) -> dict:
    """Read skill + assigned trait set without rescanning the leader pool."""
    reader = LST.LeaderSkillTraitsReader.__new__(LST.LeaderSkillTraitsReader)
    reader._cl = cl
    reader._p = p
    reader.module_base = cl.module_base
    if trait_vtable_abs is None:
        def_addr_tok = reader.build_trait_defaddr_map()
    else:
        def_addr_tok = {}
        for addr in DN._enum_vtable(p, trait_vtable_abs):
            tok = DN._read_msvc_str(p, addr + DN.UL_TRAIT_TOKEN_OFF)
            if tok:
                def_addr_tok[addr] = tok
    rows = []
    none_skill = 0
    none_traits = 0
    trait_tokens = 0
    for key, kind in (("army", "army"), ("navy", "navy")):
        for addr in addr_sets.get(key, []):
            rec = reader.read_leader(addr, kind, def_addr_tok)
            if not rec:
                continue
            rows.append(rec)
            if rec.get("skill") is None:
                none_skill += 1
            traits = rec.get("traits")
            if traits is None:
                none_traits += 1
            else:
                trait_tokens += len(traits)
    out = {
        "counts": {
            "leaders": len(rows),
            "army": sum(1 for row in rows if row.get("kind") == "army"),
            "navy": sum(1 for row in rows if row.get("kind") == "navy"),
            "skill_none": none_skill,
            "traits_none": none_traits,
            "trait_tokens": trait_tokens,
            "unit_leader_trait_defs": len(def_addr_tok),
        },
        "sample": _sample(
            [
                {
                    "unit_leader_id": row.get("unit_leader_id"),
                    "kind": row.get("kind"),
                    "skill": row.get("skill"),
                    "traits": _sample(row.get("traits") or [], 8),
                }
                for row in rows
            ],
            8,
        ),
    }
    if include_rows:
        out["_rows_for_compact"] = rows
    return out


def _apply_rtti_vtable_overrides(cl: CL.CharactersLiveReader, class_names: list[str]) -> dict:
    overrides = {}
    for name in class_names:
        try:
            got = cl._resolve_vtable_rtti(name)
        except Exception as exc:
            overrides[name] = {"error": f"{type(exc).__name__}: {exc}"}
            continue
        if got:
            overrides[name] = {
                "absolute": f"0x{got:X}",
                "rva": f"0x{got - cl.module_base:X}",
            }
            if name in cl.vt:
                cl.vt[name] = got
        else:
            overrides[name] = {"absolute": None, "rva": None}
    return overrides


DOMAIN_ID = 47
COMPACT_RECORD_MAGIC = b"H4CR"
COMPACT_RECORD_HEADER_LEN = 32
COMPACT_RECORD_VERSION = 1
COMPACT_RECORD_TYPE_DOMAIN_DATA = 3
GAMESTATE_RVA_CANDIDATES = (0x3316E90, 0x33048C0)
GAMESTATE_CURRENT_DATE_OFF = 0x468

HEADER_LEN = 64
STRING_NONE = 0xFFFFFFFF
SKILL_NONE = 0xFFFFFFFF
BYTE_NONE = 0xFF
EXPERIENCE_RAW_NONE = -0x80000000
MAX_STRING_BYTES = 160

SCHEMA_ROOTS_VALIDATED = 1
SCHEMA_STATIC_NAME_CLOSURE = 1 << 1
SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS = 1 << 2

ROW_CHARACTER = 1
ROW_UNIT_LEADER = 2
ROW_UNIT_LEADER_TRAIT = 3
ROW_COUNTRY_LEADER_ROLE = 4
ROW_ACTIVE_RULER = 5
ROW_ADVISOR_ROSTER = 6
ROW_APPOINTED_ADVISOR = 7
ROW_CHARACTER_PORTRAIT = 8

UNIT_LEADER_ARMY = 1
UNIT_LEADER_NAVY = 2

ROW_FLAG_AMBIGUOUS = 1
ROW_FLAG_SKILL_PRESENT = 1 << 1
ROW_FLAG_EXPERIENCE_PRESENT = 1 << 2

ADVISOR_CABINET_BUCKETS = (
    "political_advisor",
    "army_chief",
    "navy_chief",
    "air_chief",
    "high_command",
    "theorist",
    "unknown",
)
ADVISOR_SLOT_TO_BUCKET = {bucket: bucket for bucket in ADVISOR_CABINET_BUCKETS}


def read_current_game_hour_sample(cl: CL.CharactersLiveReader) -> dict:
    for rva in GAMESTATE_RVA_CANDIDATES:
        gamestate = cl.u64(cl.module_base + rva)
        if not _heap_ok(gamestate):
            continue
        raw = cl.u32(int(gamestate) + GAMESTATE_CURRENT_DATE_OFF)
        decoded = PRES.decode_datehour(raw)
        if decoded:
            return {
                "ok": True,
                "game_hour_raw": int(raw),
                "decoded": decoded,
                "gamestate": f"0x{int(gamestate):X}",
                "root_rva": f"0x{rva:X}",
                "offset": f"+0x{GAMESTATE_CURRENT_DATE_OFF:X}",
            }
    return {
        "ok": False,
        "game_hour_raw": None,
        "decoded": None,
        "gamestate": None,
        "root_rva": None,
        "offset": f"+0x{GAMESTATE_CURRENT_DATE_OFF:X}",
    }


def _ascii_optional(value, max_len: int = MAX_STRING_BYTES):
    if value is None:
        return None
    value = str(value)
    raw = value.encode("ascii", errors="ignore")
    if not raw or len(raw) != len(value) or len(raw) > max_len:
        return None
    if any(not (b == 0x20 or 0x21 <= b <= 0x7E) for b in raw):
        return None
    return value


def _advisor_cabinet_bucket(slot) -> str:
    slot = _ascii_optional(slot)
    return ADVISOR_SLOT_TO_BUCKET.get(slot, "unknown")


class _StringTable:
    def __init__(self):
        self.values: list[str] = []
        self.index: dict[str, int] = {}

    def intern(self, value) -> int:
        value = _ascii_optional(value)
        if value is None:
            return STRING_NONE
        if value in self.index:
            return self.index[value]
        idx = len(self.values)
        self.index[value] = idx
        self.values.append(value)
        return idx

    def require(self, value) -> int | None:
        value = _ascii_optional(value)
        if value is None:
            return None
        return self.intern(value)

    def encoded(self) -> bytes:
        out = bytearray()
        for value in self.values:
            raw = value.encode("ascii")
            out += struct.pack("<H", len(raw))
            out += raw
        return bytes(out)


def _push_row_header(out: bytearray, kind: int, row_len: int, flags: int = 0) -> None:
    out += struct.pack("<BBH", kind, row_len, flags)


def _u8_or_none(value) -> int:
    if value is None:
        return BYTE_NONE
    value = int(value)
    return value if 0 <= value <= 254 else BYTE_NONE


def _compact_tag_or_zero(value) -> int:
    try:
        value = int(value or 0)
    except (TypeError, ValueError):
        return 0
    return value if 0 <= value <= 2048 else 0


def _encode_domain47_payload(
    elapsed_ms: int,
    status_flags: int,
    schema_flags: int,
    rows: list[dict],
) -> tuple[bytes, dict]:
    strings = _StringTable()
    row_bytes = bytearray()
    counts = collections.Counter()
    skipped = collections.Counter()

    for row in rows:
        kind = row["kind"]
        if kind == ROW_CHARACTER:
            counts["characters"] += 1
            _push_row_header(row_bytes, ROW_CHARACTER, 20)
            row_bytes += struct.pack(
                "<IIII",
                int(row.get("country_tag_db_id") or 0),
                int(row["char_id"]),
                strings.intern(row.get("name")),
                strings.intern(row.get("token")),
            )
        elif kind == ROW_UNIT_LEADER:
            counts["unit_leaders"] += 1
            skill = row.get("skill")
            exp = row.get("experience_raw")
            flags = 0
            if skill is not None:
                flags |= ROW_FLAG_SKILL_PRESENT
            if exp is not None:
                flags |= ROW_FLAG_EXPERIENCE_PRESENT
            _push_row_header(row_bytes, ROW_UNIT_LEADER, 42, flags)
            row_bytes += struct.pack(
                "<IIIIiIIHBBBBBBBB",
                int(row["country_tag_db_id"]),
                int(row["unit_leader_id"]),
                int(row["char_id"]),
                int(skill) if skill is not None else SKILL_NONE,
                int(exp) if exp is not None else EXPERIENCE_RAW_NONE,
                strings.intern(row.get("token")),
                strings.intern(row.get("name")),
                int(row.get("trait_count") or 0),
                _u8_or_none(row.get("attack")),
                _u8_or_none(row.get("defense")),
                _u8_or_none(row.get("planning")),
                _u8_or_none(row.get("logistics")),
                _u8_or_none(row.get("maneuvering")),
                _u8_or_none(row.get("coordination")),
                int(row["leader_kind"]),
                0,
            )
        elif kind == ROW_UNIT_LEADER_TRAIT:
            trait_sid = strings.require(row.get("trait_token"))
            if trait_sid is None:
                skipped["unit_leader_trait_non_ascii_token"] += 1
                continue
            counts["traits"] += 1
            _push_row_header(row_bytes, ROW_UNIT_LEADER_TRAIT, 16)
            row_bytes += struct.pack(
                "<IHHI",
                int(row["unit_leader_id"]),
                int(row["ordinal"]),
                0,
                trait_sid,
            )
        elif kind == ROW_COUNTRY_LEADER_ROLE:
            counts["country_leader_roles"] += 1
            _push_row_header(row_bytes, ROW_COUNTRY_LEADER_ROLE, 20)
            row_bytes += struct.pack(
                "<IIII",
                int(row.get("country_tag_db_id") or 0),
                int(row["char_id"]),
                strings.intern(row.get("name")),
                strings.intern(row.get("token")),
            )
        elif kind == ROW_ACTIVE_RULER:
            counts["active_rulers"] += 1
            flags = ROW_FLAG_AMBIGUOUS if row.get("ambiguous") else 0
            _push_row_header(row_bytes, ROW_ACTIVE_RULER, 28, flags)
            row_bytes += struct.pack(
                "<IIIIIHH",
                int(row["country_tag_db_id"]),
                int(row["char_id"]),
                strings.intern(row.get("ruling_party")),
                strings.intern(row.get("name")),
                strings.intern(row.get("token")),
                int(row.get("leader_vec_size") or 1),
                0,
            )
        elif kind in (ROW_ADVISOR_ROSTER, ROW_APPOINTED_ADVISOR):
            idea_sid = strings.require(row.get("idea_token"))
            if idea_sid is None:
                skipped["advisor_non_ascii_or_missing_idea_token"] += 1
                continue
            bucket_sid = strings.require(row.get("ledger"))
            if bucket_sid is None:
                skipped["advisor_non_ascii_or_missing_cabinet_bucket"] += 1
                continue
            if kind == ROW_ADVISOR_ROSTER:
                counts["advisor_roster"] += 1
            else:
                counts["appointed_advisors"] += 1
            _push_row_header(row_bytes, kind, 24)
            row_bytes += struct.pack(
                "<IIIII",
                int(row["country_tag_db_id"]),
                int(row["char_id"]),
                strings.intern(row.get("slot")),
                bucket_sid,
                idea_sid,
            )
        elif kind == ROW_CHARACTER_PORTRAIT:
            role_sid = strings.require(row.get("role"))
            size_sid = strings.require(row.get("size"))
            gfx_sid = strings.require(row.get("gfx"))
            if role_sid is None or size_sid is None or gfx_sid is None:
                skipped["character_portrait_missing_required_string"] += 1
                continue
            counts["character_portraits"] += 1
            _push_row_header(row_bytes, kind, 24)
            row_bytes += struct.pack(
                "<IIIII",
                int(row["country_tag_db_id"]),
                int(row["char_id"]),
                role_sid,
                size_sid,
                gfx_sid,
            )
        else:
            skipped["unknown_kind"] += 1

    strings_blob = strings.encoded()
    total = sum(counts.values())
    header = struct.pack(
        "<QIIIIIIIIIIIII",
        int(elapsed_ms),
        int(status_flags),
        int(total),
        len(strings.values),
        len(strings_blob),
        counts["characters"],
        counts["unit_leaders"],
        counts["country_leader_roles"],
        counts["active_rulers"],
        counts["advisor_roster"],
        counts["appointed_advisors"],
        counts["traits"],
        len(row_bytes),
        int(schema_flags),
    ) + struct.pack("<I", 0)
    if len(header) != HEADER_LEN:
        raise AssertionError(f"domain47 header length drifted: {len(header)}")
    payload = header + strings_blob + bytes(row_bytes)
    return payload, {
        "row_count_total": int(total),
        "character_rows": int(counts["characters"]),
        "unit_leader_rows": int(counts["unit_leaders"]),
        "unit_leader_trait_rows": int(counts["traits"]),
        "country_leader_role_rows": int(counts["country_leader_roles"]),
        "active_ruler_rows": int(counts["active_rulers"]),
        "advisor_roster_rows": int(counts["advisor_roster"]),
        "appointed_advisor_rows": int(counts["appointed_advisors"]),
        "character_portrait_rows": int(counts["character_portraits"]),
        "string_count": len(strings.values),
        "string_bytes": len(strings_blob),
        "payload_bytes": len(payload),
        "status_flags": int(status_flags),
        "schema_flags": int(schema_flags),
        "skipped": dict(skipped),
    }


def _encode_compact_record(sequence: int, game_hour_raw: int, payload: bytes) -> bytes:
    return (
        COMPACT_RECORD_MAGIC
        + struct.pack(
            "<HHQiHHII",
            COMPACT_RECORD_VERSION,
            COMPACT_RECORD_HEADER_LEN,
            int(sequence),
            int(game_hour_raw),
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            len(payload),
            0,
        )
        + payload
    )


def _compact_rows_from_live(
    cl: CL.CharactersLiveReader,
    addr_sets: dict,
    commander_data: dict,
    skill_trait_rows: list[dict],
    tag_to_id: dict[str, int],
    rulers: dict,
    roster: dict,
    appointed: dict,
) -> tuple[list[dict], dict]:
    rows: list[dict] = []
    skipped = collections.Counter()
    advisor_bucket_counts = {
        "advisor_roster": collections.Counter(),
        "appointed_advisor": collections.Counter(),
    }
    chars = commander_data.get("characters") or {}

    for char_id, char in sorted(chars.items()):
        rows.append(
            {
                "kind": ROW_CHARACTER,
                "country_tag_db_id": _compact_tag_or_zero(char.get("country_tag_db_id")),
                "char_id": int(char_id),
                "name": char.get("name"),
                "token": None,
            }
        )

    skills_by_unit = {int(row["unit_leader_id"]): row for row in skill_trait_rows if row.get("unit_leader_id")}
    for leader_kind, source_rows in (
        (UNIT_LEADER_ARMY, commander_data.get("army_leaders") or []),
        (UNIT_LEADER_NAVY, commander_data.get("navy_leaders") or []),
    ):
        for leader in source_rows:
            unit_id = leader.get("unit_leader_id")
            char_id = leader.get("char_id")
            country_id = leader.get("country_tag_db_id")
            if not unit_id or not char_id or not country_id:
                skipped["unit_leader_missing_identity"] += 1
                continue
            compact_country_id = _compact_tag_or_zero(country_id)
            if compact_country_id == 0:
                skipped["unit_leader_dynamic_or_invalid_country_id"] += 1
                continue
            skill_row = skills_by_unit.get(int(unit_id), {})
            traits = skill_row.get("traits") or []
            exp = leader.get("experience")
            rows.append(
                {
                    "kind": ROW_UNIT_LEADER,
                    "country_tag_db_id": compact_country_id,
                    "unit_leader_id": int(unit_id),
                    "char_id": int(char_id),
                    "leader_kind": leader_kind,
                    "skill": skill_row.get("skill"),
                    "experience_raw": int(round(float(exp) * 100000)) if exp is not None else None,
                    "attack": leader.get("attack"),
                    "defense": leader.get("defense"),
                    "planning": leader.get("planning"),
                    "logistics": leader.get("logistics"),
                    "maneuvering": leader.get("maneuvering"),
                    "coordination": leader.get("coordination"),
                    "token": leader.get("token"),
                    "name": leader.get("name") or leader.get("char_name"),
                    "trait_count": len(traits),
                }
            )
            for ordinal, trait_token in enumerate(traits):
                rows.append(
                    {
                        "kind": ROW_UNIT_LEADER_TRAIT,
                        "unit_leader_id": int(unit_id),
                        "ordinal": ordinal,
                        "trait_token": trait_token,
                    }
                )

    seen_role_chars = set()
    for addr in addr_sets.get("country_leader", ()):
        char_ptr = cl.u64(addr + CL.OFF_CL_CHAR_PTR)
        if not _heap_ok(char_ptr):
            skipped["country_leader_bad_char_ptr"] += 1
            continue
        char_id = cl.u32(int(char_ptr) + CL.OFF_ID)
        if not char_id or char_id in seen_role_chars:
            skipped["country_leader_duplicate_or_missing_char_id"] += 1
            continue
        seen_role_chars.add(int(char_id))
        country_id = cl.u32(int(char_ptr) + CL.OFF_CHAR_COUNTRY) or 0
        rows.append(
            {
                "kind": ROW_COUNTRY_LEADER_ROLE,
                "country_tag_db_id": _compact_tag_or_zero(country_id),
                "char_id": int(char_id),
                "name": cl.read_msvc_str(int(char_ptr) + LD.CHAR_NAME, maxlen=80),
                "token": cl.read_msvc_str(int(char_ptr) + LD.CHAR_TOKEN, maxlen=80),
            }
        )

    for tag, ruler in sorted(rulers.items()):
        tag_id = tag_to_id.get(tag)
        if not tag_id:
            skipped["active_ruler_missing_tag_id"] += 1
            continue
        rows.append(
            {
                "kind": ROW_ACTIVE_RULER,
                "country_tag_db_id": int(tag_id),
                "char_id": int(ruler.get("ruler_char_id") or 0),
                "ruling_party": ruler.get("ruling_party"),
                "name": ruler.get("ruler_name"),
                "token": ruler.get("ruler_token"),
                "leader_vec_size": 2 if ruler.get("ambiguous") else 1,
                "ambiguous": bool(ruler.get("ambiguous")),
            }
        )

    seen_advisors = set()
    for tag, advisor_rows in sorted(roster.items()):
        tag_id = tag_to_id.get(tag)
        if not tag_id:
            skipped["advisor_roster_missing_or_zero_tag_id"] += len(advisor_rows or [])
            continue
        for advisor in advisor_rows or []:
            if not advisor.get("char_id") or not advisor.get("idea_token"):
                skipped["advisor_roster_missing_identity"] += 1
                continue
            dedupe_key = (ROW_ADVISOR_ROSTER, int(tag_id), int(advisor["char_id"]), str(advisor.get("idea_token")))
            if dedupe_key in seen_advisors:
                skipped["advisor_roster_duplicate"] += 1
                continue
            seen_advisors.add(dedupe_key)
            bucket = _advisor_cabinet_bucket(advisor.get("slot"))
            advisor_bucket_counts["advisor_roster"][bucket] += 1
            rows.append(
                {
                    "kind": ROW_ADVISOR_ROSTER,
                    "country_tag_db_id": int(tag_id),
                    "char_id": int(advisor["char_id"]),
                    "slot": advisor.get("slot"),
                    "ledger": bucket,
                    "idea_token": advisor.get("idea_token"),
                }
            )

    for tag, bucket in sorted(appointed.items()):
        tag_id = tag_to_id.get(tag)
        if not tag_id:
            skipped["appointed_missing_or_zero_tag_id"] += len(bucket.get("appointed") or [])
            continue
        for advisor in bucket.get("appointed") or []:
            if not advisor.get("char_id") or not advisor.get("idea_token"):
                skipped["appointed_missing_identity"] += 1
                continue
            dedupe_key = (ROW_APPOINTED_ADVISOR, int(tag_id), int(advisor["char_id"]), str(advisor.get("idea_token")))
            if dedupe_key in seen_advisors:
                skipped["appointed_duplicate"] += 1
                continue
            seen_advisors.add(dedupe_key)
            bucket = _advisor_cabinet_bucket(advisor.get("slot"))
            advisor_bucket_counts["appointed_advisor"][bucket] += 1
            rows.append(
                {
                    "kind": ROW_APPOINTED_ADVISOR,
                    "country_tag_db_id": int(tag_id),
                    "char_id": int(advisor["char_id"]),
                    "slot": advisor.get("slot"),
                    "ledger": bucket,
                    "idea_token": advisor.get("idea_token"),
                }
            )

    return rows, {
        "skipped_source_rows": dict(skipped),
        "advisor_cabinet_bucket_counts": {
            key: dict(sorted(counter.items()))
            for key, counter in advisor_bucket_counts.items()
        },
    }


def _write_compact_artifacts(
    out_dir: Path,
    elapsed_ms: int,
    rows: list[dict],
    game_hour_raw: int,
) -> dict:
    payload, summary = _encode_domain47_payload(
        elapsed_ms,
        0,
        SCHEMA_ROOTS_VALIDATED | SCHEMA_STATIC_NAME_CLOSURE | SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS,
        rows,
    )
    record = _encode_compact_record(1, game_hour_raw, payload)
    payload_path = out_dir / "domain47_payload.bin"
    record_path = out_dir / "domain47_record.bin"
    payload_path.write_bytes(payload)
    record_path.write_bytes(record)
    payload_rel = payload_path.resolve().relative_to(REPO)
    record_rel = record_path.resolve().relative_to(REPO)
    summary.update(
        {
            "payload_path": str(payload_rel),
            "record_path": str(record_rel),
            "record_bytes": len(record),
            "record_domain_id": DOMAIN_ID,
            "record_game_hour_raw": int(game_hour_raw),
        }
    )
    return summary


def run(args: argparse.Namespace) -> dict:
    started = time.perf_counter()
    p = winmem.open_process(args.process)
    if p is None:
        raise SystemExit(f"{args.process} not running")

    proof: dict = {
        "lane": "characters_leaders_cabinet_live",
        "proof_kind": "live_readonly_root_validation",
        "process": args.process,
        "started_utc": _utc_stamp(),
        "runtime_dependency": "no; this script is a dev proof harness only",
        "production_target": "hoi4.exe -> DLL collector -> typed compact domain rows -> Rust recorder -> .battleplan",
        "build": {},
        "roots_offsets_validated": {
            "CCharacter": {
                "vtable_rva": "0x2957D40",
                "guards": ["vtable", "type@+0x08 == 73", "char_id@+0x0C"],
                "fields": {
                    "country_tag_db_id": "+0x68",
                    "unit_leader_ptr": "+0xA8",
                },
            },
            "CArmyLeader": {
                "vtable_rva": "0x292F788",
                "guards": ["vtable", "type@+0x08 == 4713", "unit_leader_id@+0x0C"],
                "fields": {
                    "experience_raw": "+0xE68 i32 fixed x100000",
                    "skill": "*(u32*)(*(leader+0xE60)+0x1B8)",
                    "trait_membership": "data@+0xDC8 capacity@+0xDD0 size@+0xDD4 elements=CUnitLeaderTrait*",
                    "attack_defense_planning_logistics": "+0xF58/+0xF68/+0xF78/+0xF88 u8",
                },
            },
            "CNavyLeader": {
                "vtable_rva": "0x292F8A0",
                "guards": ["vtable", "type@+0x08 == 4713", "unit_leader_id@+0x0C"],
                "fields": {
                    "experience_raw": "+0xE68 i32 fixed x100000",
                    "skill": "*(u32*)(*(leader+0xE60)+0x1B8)",
                    "trait_membership": "data@+0xDC8 capacity@+0xDD0 size@+0xDD4 elements=CUnitLeaderTrait*",
                    "attack_defense_maneuvering_coordination": "+0xF68/+0xF78/+0xF88/+0xF98 u8",
                },
            },
            "CCountryLeader": {
                "vtable_rva": "0x295A630",
                "fields": {"role_character_ptr": "+0x08 -> CCharacter"},
            },
            "active_ruler": {
                "chain": [
                    "CCountry+0xF90 -> CPoliticalStatus",
                    "CPoliticalStatus+0xD0 -> ruling CPoliticalParty",
                    "CPoliticalParty+0x70 -> vector<CCountryLeader*>",
                    "vector[0]+0x08 -> active ruler CCharacter",
                ],
                "ambiguity_guard": "leader vector size must be 1; otherwise row is ambiguous",
            },
            "advisor_roster": {
                "CAdvisor_vtable_rva": "0x2980560",
                "fields": {
                    "character_ptr": "+0x10 -> CCharacter",
                    "idea_token": "+0x30 std::string",
                    "slot": "+0x80 std::string",
                },
            },
            "appointed_cabinet": {
                "source": "active politics.ideas intersect advisor token/static metadata",
                "dll_note": "collector already supports advisor_token_filter; do not sweep CAdvisor in the hourly hot path",
            },
            "unit_leader_trait_defs": {
                "CUnitLeaderTrait_vtable_rva": "0x291F3C0",
                "token": "+0x10 std::string",
            },
        },
        "candidate_rejections": [
            "CCharacter/CCountryLeader active-ruler flag rejected; active pick is politics-side via ruling party leader vector.",
            "leader+0x288 flat_map rejected as assigned trait membership; it is not the persistent trait set.",
            "CAdvisor hired bool rejected for this lane; appointed state is active-idea intersection and static advisor-token metadata.",
        ],
        "compact_schema": {
            "domain_id": 47,
            "domain_id_note": "proposed production id; prior candidates 44, 45, and 46 are rejected for supply, focus/research/decisions depth, and air_variant_archetypes respectively",
            "rows": [
                "character",
                "unit_leader",
                "unit_leader_trait",
                "country_leader_role",
                "active_ruler",
                "advisor_roster",
                "appointed_advisor",
                "character_portrait",
            ],
            "codec": "replay_native/lanes/characters_leaders_cabinet_live/domain47_compact.rs",
            "status_flags": "shared compact health flags only",
            "schema_flags": [
                "roots_validated",
                "static_name_closure",
                "appointed_from_active_ideas",
            ],
        },
        "checks": {},
        "errors": [],
    }

    try:
        cl = CL.CharactersLiveReader(args.process)
        ok, digest = cl.verify_build()
        proof["build"] = {
            "pid": getattr(cl, "pid", None),
            "module_base": f"0x{cl.module_base:X}",
            "exe_path": getattr(cl, "exe_path", None),
            "sha256": digest,
            "sha256_matches_expected": bool(ok),
            "expected_sha256": CL.EXPECTED_SHA256,
        }
        if not ok and not args.force:
            raise SystemExit(f"build gate failed: {digest} != {CL.EXPECTED_SHA256}")

        proof["game_hour_start"] = read_current_game_hour_sample(cl)

        rtti = cl.rtti_self_check()
        rtti_overrides = {}
        if args.use_rtti_vtables:
            rtti_overrides = _apply_rtti_vtable_overrides(
                cl,
                [
                    "CCharacter",
                    "CArmyLeader",
                    "CNavyLeader",
                    "CCountryLeader",
                    "CAdvisor",
                    "CUnitLeaderTrait",
                ],
            )
            # Patch the helper module constants for the optional politics/advisor proof path.
            # This affects only this proof process.
            for attr, name in (
                ("VT_CHAR_RVA", "CCharacter"),
                ("VT_COUNTRYLEADER_RVA", "CCountryLeader"),
                ("VT_ADVISOR_RVA", "CAdvisor"),
            ):
                value = rtti_overrides.get(name, {}).get("rva")
                if value:
                    setattr(LD, attr, int(value, 16))

        trait_vtable_abs = (
            int(rtti_overrides.get("CUnitLeaderTrait", {}).get("absolute"), 16)
            if rtti_overrides.get("CUnitLeaderTrait", {}).get("absolute")
            else None
        )

        raw_addr_sets = cl.enumerate_addrs()
        addr_sets, filter_info = _filter_lane_addr_sets(cl, raw_addr_sets, args.sample_limit)
        legacy_addr_valid = cl.validate_addrs(raw_addr_sets)
        lane_pin_valid = _validate_lane_warm_pins(
            cl,
            addr_sets,
            trait_vtable_abs,
            args.sample_limit,
        )
        commander_data = cl.enumerate_from_addrs(
            addr_sets,
            resolve_country=True,
            read_traits=False,
        )
        proof["checks"]["characters_live"] = {
            "rtti_self_check": rtti,
            "rtti_overrides": rtti_overrides,
            "rtti_overrides_applied": bool(args.use_rtti_vtables),
            "address_sets_validate": bool(lane_pin_valid.get("ok")),
            "legacy_characters_live_validate_addrs": bool(legacy_addr_valid),
            "lane_warm_pin_validate": lane_pin_valid,
            "lane_pin_filter": filter_info,
            "located_counts": {
                "army": len(raw_addr_sets.get("army") or []),
                "navy": len(raw_addr_sets.get("navy") or []),
                "characters": len(raw_addr_sets.get("char") or []),
                "country_leader_roles": len(raw_addr_sets.get("country_leader") or []),
            },
            "decoded_counts": commander_data.get("counts", {}),
            "sample_army": _sample(
                [
                    {
                        "unit_leader_id": row.get("unit_leader_id"),
                        "char_id": row.get("char_id"),
                        "country": row.get("country"),
                        "token": row.get("token"),
                        "skill_stats": {
                            "attack": row.get("attack"),
                            "defense": row.get("defense"),
                            "planning": row.get("planning"),
                            "logistics": row.get("logistics"),
                        },
                    }
                    for row in commander_data.get("army_leaders", [])
                ],
                args.sample_limit,
            ),
            "sample_navy": _sample(
                [
                    {
                        "unit_leader_id": row.get("unit_leader_id"),
                        "char_id": row.get("char_id"),
                        "country": row.get("country"),
                        "token": row.get("token"),
                        "skill_stats": {
                            "attack": row.get("attack"),
                            "defense": row.get("defense"),
                            "maneuvering": row.get("maneuvering"),
                            "coordination": row.get("coordination"),
                        },
                    }
                    for row in commander_data.get("navy_leaders", [])
            ],
                args.sample_limit,
            ),
        }
        if not lane_pin_valid.get("ok"):
            proof["errors"].append(
                "lane-local current-build warm-pin validation failed"
            )
        elif not legacy_addr_valid:
            proof["candidate_rejections"].append(
                "legacy characters_live.validate_addrs rejected for this lane on current build: it fingerprints leader+0x288, while assigned traits are validated at leader+0xDC8"
            )

        skill_traits = _read_skill_traits_from_located_addresses(
            cl,
            p,
            addr_sets,
            trait_vtable_abs,
            include_rows=True,
        )
        skill_trait_rows = skill_traits.pop("_rows_for_compact", [])
        proof["checks"]["leader_skill_traits"] = skill_traits

        live_country_bridge = None
        idx_to_tag = {}
        tag_to_id = _tag_inverse(p)
        if args.skip_politics:
            proof["checks"]["active_rulers"] = {
                "skipped": True,
                "reason": "--skip-politics supplied",
            }
            proof["checks"]["appointed_advisors"] = {
                "skipped": True,
                "reason": "--skip-politics supplied",
            }
            proof["checks"]["advisor_roster"] = {
                "skipped": True,
                "reason": "--skip-politics supplied",
            }
            proof["checks"]["unit_leader_trait_registry"] = {
                "def_objects": proof["checks"]["leader_skill_traits"]["counts"].get("unit_leader_trait_defs"),
                "index_entries": None,
                "source": "RTTI override" if args.use_rtti_vtables else "defnames_live",
            }
            proof["result"] = "validated" if not proof["errors"] else "partial"
            proof["elapsed_wall_seconds"] = round(time.perf_counter() - started, 3)
            proof["finished_utc"] = _utc_stamp()
            return proof

        live_country_bridge, idx_to_tag, bridge_tag_to_id, bridge_info = _resolve_live_country_bridge(p)
        tag_to_id.update(bridge_tag_to_id)
        proof["checks"]["country_index_bridge"] = bridge_info
        if not bridge_info.get("revalidate"):
            proof["errors"].append("CountryIndexBridge resolved but revalidate() returned false")

        rulers = LD.read_active_rulers(
            p,
            bridge=idx_to_tag,
            rec0=live_country_bridge.rec0,
            count=live_country_bridge.count,
        )
        ambiguous_rulers = {
            tag: row
            for tag, row in rulers.items()
            if isinstance(row, dict) and row.get("ambiguous")
        }
        proof["checks"]["active_rulers"] = {
            "countries": len(rulers),
            "ambiguous_count": len(ambiguous_rulers),
            "sample": _sample(
                {
                    tag: {
                        "tag_db_id": tag_to_id.get(tag),
                        "ruler_char_id": row.get("ruler_char_id"),
                        "ruling_party": row.get("ruling_party"),
                        "ruler_name": row.get("ruler_name"),
                        "ruler_token": row.get("ruler_token"),
                        "ambiguous": row.get("ambiguous"),
                    }
                    for tag, row in sorted(rulers.items())
                },
                args.sample_limit,
            ),
            "path": "CountryIndexBridge rec0/count + CCountry+0xF90 -> CPoliticalStatus+0xD0 -> CPoliticalParty+0x70 -> CCountryLeader[0]+0x08",
            "save_pp_route_used": False,
        }
        if not rulers:
            proof["errors"].append("active_rulers read zero rows through the live country bridge")

        roster = {}
        if not args.skip_advisor_roster:
            advisor_addrs, advisor_snapshot = LD.enumerate_advisor_addrs(p)
            advisor_pin_valid = LD.validate_advisor_addrs(p, advisor_addrs, advisor_snapshot)
            tagmap = live_country_bridge.tbl.as_map()
            roster = LD.read_advisor_roster_from_addrs(p, advisor_addrs, tagmap)
            slots = collections.Counter()
            for rows in roster.values():
                for row in rows:
                    slots[row.get("slot") or "unknown"] += 1
            proof["checks"]["advisor_roster"] = {
                "address_count": len(advisor_addrs),
                "warm_pin_validate": bool(advisor_pin_valid),
                "countries": len(roster),
                "rows": sum(len(rows) for rows in roster.values()),
                "slots": dict(sorted(slots.items())),
                "sample": _sample(
                    {
                        tag: _sample(rows, args.sample_limit)
                        for tag, rows in sorted(roster.items())
                        if rows
                    },
                    args.sample_limit,
                ),
                "path": "RTTI CAdvisor pool -> CAdvisor+0x10 CCharacter -> CCharacter+0x68 tag_db_id -> live tag table",
            }
            if not advisor_pin_valid:
                proof["errors"].append("CAdvisor lane warm-pin validation failed")
            if not roster:
                proof["errors"].append("advisor_roster read zero rows with current CAdvisor RTTI root")
        else:
            proof["checks"]["advisor_roster"] = {
                "skipped": True,
                "reason": "--skip-advisor-roster supplied",
            }

        politics = POL.read_country_politics(
            p,
            bridge=idx_to_tag,
            rec0=live_country_bridge.rec0,
            count=live_country_bridge.count,
        )
        appointed = PRES.read_appointed_advisors(
            p,
            politics=politics,
            roster=roster or None,
        )
        nonempty_appointed = {
            tag: row
            for tag, row in appointed.items()
            if row.get("appointed_tokens")
        }
        proof["checks"]["appointed_advisors"] = {
            "countries": len(appointed),
            "appointed_rows": sum(len(row.get("appointed") or []) for row in appointed.values()),
            "nonempty_countries": sorted(nonempty_appointed),
            "sample_nonempty": _sample(nonempty_appointed, args.sample_limit),
            "measured_empty_countries": sum(
                1 for row in appointed.values() if not row.get("appointed_tokens")
            ),
            "politics_countries": len(politics),
            "path": "active politics.ideas intersect current-build CAdvisor roster idea_token",
            "save_pp_route_used": False,
        }
        if not politics:
            proof["errors"].append("politics read zero countries through the live country bridge")
        if not appointed:
            proof["errors"].append("appointed advisor intersection produced zero country buckets")

        proof["checks"]["unit_leader_trait_registry"] = {
            "def_objects": proof["checks"]["leader_skill_traits"]["counts"].get("unit_leader_trait_defs"),
            "index_entries": (
                len([addr for addr in DN._enum_vtable(p, trait_vtable_abs)])
                if trait_vtable_abs is not None
                else len(DN.build_unit_leader_trait_index(p))
            ),
            "source": "RTTI override" if trait_vtable_abs is not None else "defnames_live",
        }
        if not proof["errors"]:
            out_dir = Path(args.out_dir)
            compact_rows, compact_source_summary = _compact_rows_from_live(
                cl,
                addr_sets,
                commander_data,
                skill_trait_rows,
                tag_to_id,
                rulers,
                roster,
                appointed,
            )
            game_hour_end = read_current_game_hour_sample(cl)
            proof["game_hour_end"] = game_hour_end
            record_game_hour_raw = (
                game_hour_end.get("game_hour_raw")
                or proof["game_hour_start"].get("game_hour_raw")
                or 0
            )
            compact_summary = _write_compact_artifacts(
                out_dir,
                int((time.perf_counter() - started) * 1000),
                compact_rows,
                int(record_game_hour_raw),
            )
            compact_summary.update(compact_source_summary)
            compact_summary["game_hour_start"] = proof.get("game_hour_start")
            compact_summary["game_hour_end"] = game_hour_end
            proof["checks"]["compact_artifacts"] = compact_summary
        proof["result"] = "validated" if not proof["errors"] else "partial"
    except BaseException as exc:
        proof["errors"].append(f"{type(exc).__name__}: {exc}")
        proof["result"] = "failed"
        if not args.keep_failed:
            raise
    finally:
        closer = getattr(p, "close", None)
        if callable(closer):
            closer()

    proof["elapsed_wall_seconds"] = round(time.perf_counter() - started, 3)
    proof["finished_utc"] = _utc_stamp()
    return proof


def write_markdown(path: Path, proof: dict) -> None:
    checks = proof.get("checks", {})
    chars = checks.get("characters_live", {})
    bridge = checks.get("country_index_bridge", {})
    skills = checks.get("leader_skill_traits", {}).get("counts", {})
    rulers = checks.get("active_rulers", {})
    roster = checks.get("advisor_roster", {})
    appointed = checks.get("appointed_advisors", {})
    compact = checks.get("compact_artifacts", {})
    lines = [
        "# Characters / Leaders / Cabinet Live Proof",
        "",
        f"- result: `{proof.get('result')}`",
        f"- pid: `{proof.get('build', {}).get('pid')}`",
        f"- exe sha256: `{proof.get('build', {}).get('sha256')}`",
        f"- module base: `{proof.get('build', {}).get('module_base')}`",
        f"- elapsed wall seconds: `{proof.get('elapsed_wall_seconds')}`",
        "",
        "## Root Counts",
        "",
        f"- character vtable counts: `{chars.get('located_counts')}`",
        f"- address validation: `{chars.get('address_sets_validate')}`",
        f"- legacy characters_live.validate_addrs: `{chars.get('legacy_characters_live_validate_addrs')}`",
        f"- country index bridge: `{bridge.get('resolved_countries')}` countries, revalidate `{bridge.get('revalidate')}`",
        f"- leader skill/trait counts: `{skills}`",
        f"- active rulers: `{rulers.get('countries')}` countries, ambiguous `{rulers.get('ambiguous_count')}`",
        f"- advisor roster rows: `{roster.get('rows')}` across `{roster.get('countries')}` countries",
        f"- appointed advisor rows: `{appointed.get('appointed_rows')}`; nonempty countries `{appointed.get('nonempty_countries')}`",
        f"- politics countries for appointed intersection: `{appointed.get('politics_countries')}`",
        f"- compact record: `{compact.get('record_path')}` bytes `{compact.get('record_bytes')}` rows `{compact.get('row_count_total')}`",
        f"- compact record game hour: `{compact.get('record_game_hour_raw')}` start `{proof.get('game_hour_start')}` end `{proof.get('game_hour_end')}`",
        "",
        "## Completion Interpretation",
        "",
        "This proof validates current-build roots for at least one politics/cabinet sub-slice and the lane-local compact row contract. It does not claim DLL-only extraction or shared Rust recorder integration for this lane. Replay container verification is performed by the separate replay_record_verify gate when present.",
    ]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--process", default="hoi4.exe")
    ap.add_argument("--force", action="store_true", help="run despite build-gate mismatch")
    ap.add_argument("--use-rtti-vtables", action="store_true", help="use RTTI-discovered vtables when forcing an unpinned build")
    ap.add_argument("--keep-failed", action="store_true", help="write failed proof JSON instead of raising")
    ap.add_argument("--skip-advisor-roster", action="store_true", help="skip the expensive CAdvisor roster scan")
    ap.add_argument("--skip-politics", action="store_true", help="skip active-ruler and appointed-advisor politics proof")
    ap.add_argument("--sample-limit", type=int, default=6)
    ap.add_argument(
        "--out-dir",
        default=None,
        help="proof output directory; default replay_native/runs/characters_leaders_cabinet_live_<UTC>",
    )
    args = ap.parse_args()

    out_dir = Path(args.out_dir) if args.out_dir else REPO / "replay_native" / "runs" / f"characters_leaders_cabinet_live_{_utc_stamp()}"
    out_dir.mkdir(parents=True, exist_ok=True)
    args.out_dir = str(out_dir)
    proof = run(args)
    _write_json(out_dir / "live_roots_proof.json", proof)
    write_markdown(out_dir / "PROOF.md", proof)
    print(str(out_dir))
    print(json.dumps({
        "result": proof.get("result"),
        "elapsed_wall_seconds": proof.get("elapsed_wall_seconds"),
        "errors": proof.get("errors"),
    }, indent=2))


if __name__ == "__main__":
    main()
