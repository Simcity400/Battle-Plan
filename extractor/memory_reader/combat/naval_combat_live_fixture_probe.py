"""Bounded fixture verifier for current-build live naval combat roots.

This probe is read-only. It is not a production hot-loop scanner. Its job is to
turn the remaining naval_combat_live blocker into an exact fixture contract:
when the user causes an active naval battle, this verifies the candidate
CNavalCombat -> CNavalCombatant -> CFEXGroup -> CFEXMember chain and emits a
bridge row file that can drive the existing DLL bridge.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import sys
import time
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

THIS_DIR = Path(__file__).resolve().parent
MEMORY_READER = THIS_DIR.parent
REPO_ROOT = MEMORY_READER.parent
for _path in (str(MEMORY_READER), str(REPO_ROOT)):
    if _path not in sys.path:
        sys.path.insert(0, _path)

import fastscan  # noqa: E402
import winmem  # noqa: E402

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

NAVAL_COMBAT_VTABLE_RVA = 0x29B6660
NAVAL_COMBAT_SIDE_VTABLE_RVA = 0x29BC908
NAVAL_COMBAT_REGISTRY_VTABLE_RVA = 0x29B6588
NAVAL_COMBAT_MEMBER_VTABLE_RVA = 0x29F9608
CURRENT_870EBF_CSHIP_VTABLE_RVA = 0x2941748

NAVAL_COMBAT_TYPE = 4713
CSHIP_TYPE = 51

OFF_BODY_HANDLE = 0x08
OFF_BODY_ATTACKER_SIDE = 0x18
OFF_BODY_DEFENDER_SIDE = 0x20
OFF_BODY_FRONT = 0x28

OFF_SIDE_REGISTRY = 0x18
OFF_SIDE_COUNTRY_DATA = 0x68
OFF_SIDE_COUNTRY_SIZE = 0x74
OFF_SIDE_GROUP_DATA = 0xC8
OFF_SIDE_GROUP_SIZE = 0xD4
OFF_SIDE_LEADER = 0x110
OFF_SIDE_POSITIONING = 0x288

OFF_REG_HANDLE = 0x18
OFF_REG_ATTACKER_SIDE = 0x28
OFF_REG_DEFENDER_SIDE = 0x30
OFF_REG_FRONT = 0x38

OFF_FRONT_LOCATION = 0xA4

OFF_GROUP_MEMBERS_DATA = 0x08
OFF_GROUP_MEMBERS_SIZE = 0x14

OFF_MEMBER_HANDLE = 0x08
OFF_MEMBER_OWNER_TAG = 0x20
OFF_MEMBER_MAX_STRENGTH_RAW = 0x30
OFF_MEMBER_STATE = 0xE0
OFF_MEMBER_WEIGHT = 0xF8
OFF_MEMBER_SLOT = 0x120

OFF_CSHIP_HANDLE = 0x08
OFF_CSHIP_SUNK_CONVOYS = 0x3C
OFF_CSHIP_CATEGORY = 0xE0
OFF_CSHIP_STRENGTH = 0x6F8
OFF_CSHIP_ORGANISATION = 0x700
OFF_CSHIP_EXPERIENCE = 0x708
OFF_CSHIP_REF100 = 0x710
CSHIP_REF100_RAW = 10_000_000

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
MAX_BODY_HITS = 4096
MAX_GROUPS_PER_SIDE = 64
MAX_MEMBERS_PER_GROUP = 1000
MAX_COUNTRIES_PER_SIDE = 64
MAX_SHIP_INSTANCE_ID = 4_000_000
MAX_SHIP_STRENGTH_RAW = 1_000_000_000
MAX_SHIP_ORGANISATION_RAW = 100_000_000
MAX_SHIP_EXPERIENCE_RAW = 100_000_000
MAX_CSHIP_HITS = 50_000


def hx(value: Optional[int]) -> Optional[str]:
    return None if value is None else "0x%X" % value


def is_heap(value: Optional[int]) -> bool:
    return isinstance(value, int) and HEAP_LO <= value < HEAP_HI and value % 8 == 0


def safe_u32(proc: winmem.Process, addr: int) -> Optional[int]:
    try:
        return proc.u32(addr)
    except OSError:
        return None


def safe_u64(proc: winmem.Process, addr: int) -> Optional[int]:
    try:
        return proc.u64(addr)
    except OSError:
        return None


def safe_f32(proc: winmem.Process, addr: int) -> Optional[float]:
    try:
        value = proc.f32(addr)
    except OSError:
        return None
    if value != value or value in (float("inf"), float("-inf")):
        return None
    return float(value)


def read_ptr_array(proc: winmem.Process, data: int, count: int, max_count: int) -> Optional[List[int]]:
    if count < 0 or count > max_count:
        return None
    if count == 0:
        return []
    if not is_heap(data):
        return None
    raw = proc.try_read(data, count * 8)
    if len(raw) != count * 8:
        return None
    return [int.from_bytes(raw[i:i + 8], "little") for i in range(0, len(raw), 8)]


def member_bridge_rows(validated: List[Dict[str, Any]], require_two_sided: bool) -> List[Dict[str, Any]]:
    rows: List[Dict[str, Any]] = []
    for combat in validated:
        if combat.get("status") != "valid":
            continue
        if require_two_sided and not combat.get("two_sided_members"):
            continue
        combat_id = int(combat["combat_id"])
        body_ptr = int(combat["body_ptr"])
        for side_name in ("attacker", "defender"):
            side = combat.get(side_name) or {}
            for group in side.get("groups") or []:
                group_index = int(group.get("group_index") or 0)
                for member in group.get("member_rows") or []:
                    row = {
                        "combat_id": combat_id,
                        "body_ptr": body_ptr,
                        "side": side_name,
                        "group_index": group_index,
                        **member,
                    }
                    rows.append(row)
    return sorted(
        rows,
        key=lambda row: (
            int(row.get("combat_id") or 0),
            str(row.get("side") or ""),
            int(row.get("group_index") or 0),
            int(row.get("member_index") or 0),
        ),
    )


def summarize_member_bridge_rows(rows: List[Dict[str, Any]]) -> Dict[str, int]:
    non_convoy = [row for row in rows if not row.get("is_convoy")]
    resolved = [row for row in non_convoy if row.get("cship_status") == "resolved"]
    unresolved = [row for row in non_convoy if row.get("cship_status") == "missing"]
    return {
        "member_bridge_row_count": len(rows),
        "non_convoy_member_count": len(non_convoy),
        "convoy_member_count": len(rows) - len(non_convoy),
        "cship_resolved_member_count": len(resolved),
        "cship_unresolved_member_count": len(unresolved),
    }


def encode_member_bridge_jsonl(rows: List[Dict[str, Any]]) -> str:
    return "".join(json.dumps(row, sort_keys=True) + "\n" for row in rows)


class NavalCombatFixtureProbe:
    def __init__(self, pid: int, workers: int):
        self.pid = int(pid)
        self.workers = int(workers)
        self.proc = winmem.Process(self.pid, "hoi4.exe")
        self.base = self.proc.module_base
        self.vt_body = self.base + NAVAL_COMBAT_VTABLE_RVA
        self.vt_side = self.base + NAVAL_COMBAT_SIDE_VTABLE_RVA
        self.vt_registry = self.base + NAVAL_COMBAT_REGISTRY_VTABLE_RVA
        self.vt_member = self.base + NAVAL_COMBAT_MEMBER_VTABLE_RVA
        self.vt_cship = self.base + CURRENT_870EBF_CSHIP_VTABLE_RVA

    def close(self) -> None:
        self.proc.close()

    def root(self) -> Dict[str, Any]:
        fp = self.proc.fingerprint()
        return {
            "pid": self.pid,
            "module": self.proc.module_name,
            "module_path": self.proc.module_path,
            "module_base": hx(self.base),
            "module_size": self.proc.module_size,
            "sha256": fp.get("sha256"),
            "sha256_expected": EXPECTED_SHA256,
            "sha256_ok": (fp.get("sha256") or "").lower() == EXPECTED_SHA256,
        }

    def scan_body_vtable(self) -> Dict[str, Any]:
        arena = fastscan.discover_arena(self.pid)
        if arena is None:
            return {"status": "arena_discovery_failed", "hits": []}
        lo, hi = arena
        handle = fastscan._k32.OpenProcess(
            winmem.PROCESS_VM_READ | winmem.PROCESS_QUERY_INFORMATION, False, self.pid
        )
        if not handle:
            return {"status": "open_process_failed", "hits": []}
        try:
            regions = fastscan.enumerate_regions(handle, lo, hi)
        finally:
            fastscan._k32.CloseHandle(handle)

        spec = fastscan.make_qword_eq_matcher(self.vt_body)
        started = time.monotonic()
        hits = fastscan.parallel_scan(
            self.pid, lo, hi, workers=self.workers, regions=regions, **spec
        )
        elapsed = time.monotonic() - started
        return {
            "status": "scanned",
            "arena": {
                "lo": hx(lo),
                "hi": hx(hi),
                "region_count": len(regions),
                "bytes": sum(size for _, size in regions),
            },
            "target": {
                "name": "CNavalCombat_vtable",
                "rva": hex(NAVAL_COMBAT_VTABLE_RVA),
                "vtable": hx(self.vt_body),
            },
            "elapsed_s": round(elapsed, 3),
            "hit_count": len(hits),
            "hits": hits[:MAX_BODY_HITS],
            "truncated": len(hits) > MAX_BODY_HITS,
        }

    def scan_cship_registry(self) -> Dict[str, Any]:
        arena = fastscan.discover_arena(self.pid)
        if arena is None:
            return {"status": "arena_discovery_failed", "hits": [], "rows_by_id": {}}
        lo, hi = arena
        handle = fastscan._k32.OpenProcess(
            winmem.PROCESS_VM_READ | winmem.PROCESS_QUERY_INFORMATION, False, self.pid
        )
        if not handle:
            return {"status": "open_process_failed", "hits": [], "rows_by_id": {}}
        try:
            regions = fastscan.enumerate_regions(handle, lo, hi)
        finally:
            fastscan._k32.CloseHandle(handle)

        spec = fastscan.make_qword_eq_matcher(self.vt_cship)
        started = time.monotonic()
        hits = fastscan.parallel_scan(
            self.pid, lo, hi, workers=self.workers, regions=regions, **spec
        )
        elapsed = time.monotonic() - started
        rows = [self.validate_cship(hit) for hit in hits[:MAX_CSHIP_HITS]]
        valid_rows = [row for row in rows if row["status"] == "valid"]
        rows_by_id = {int(row["instance_id"]): row for row in valid_rows}
        duplicate_count = len(valid_rows) - len(rows_by_id)
        error_counts: Dict[str, int] = {}
        for row in rows:
            for error in row.get("errors") or []:
                error_counts[error] = error_counts.get(error, 0) + 1
        return {
            "status": "scanned",
            "arena": {
                "lo": hx(lo),
                "hi": hx(hi),
                "region_count": len(regions),
                "bytes": sum(size for _, size in regions),
            },
            "target": {
                "name": "CShip_vtable",
                "rva": hex(CURRENT_870EBF_CSHIP_VTABLE_RVA),
                "vtable": hx(self.vt_cship),
            },
            "elapsed_s": round(elapsed, 3),
            "hit_count": len(hits),
            "validated_count": len(valid_rows),
            "validated_unique_instance_ids": len(rows_by_id),
            "duplicate_instance_id_count": duplicate_count,
            "invalid_count": len(rows) - len(valid_rows),
            "error_counts": dict(sorted(error_counts.items())),
            "truncated": len(hits) > MAX_CSHIP_HITS,
            "rows_by_id": rows_by_id,
        }

    def validate_cship(self, body: int) -> Dict[str, Any]:
        errors: List[str] = []
        if not is_heap(body):
            errors.append("bad_cship_ptr")
        if safe_u64(self.proc, body) != self.vt_cship:
            errors.append("bad_cship_vtable")
        packed = safe_u64(self.proc, body + OFF_CSHIP_HANDLE)
        if packed is None:
            errors.append("missing_cship_handle")
            packed = 0
        type_tag = packed & 0xFFFFFFFF
        instance_id = (packed >> 32) & 0xFFFFFFFF
        if type_tag != CSHIP_TYPE:
            errors.append("bad_cship_type_tag")
        if instance_id == 0 or instance_id > MAX_SHIP_INSTANCE_ID:
            errors.append("bad_cship_instance_id")
        if safe_u32(self.proc, body + OFF_CSHIP_HANDLE + 4) != instance_id:
            errors.append("cship_instance_alias_mismatch")
        strength_raw = safe_u32(self.proc, body + OFF_CSHIP_STRENGTH)
        organisation_raw = safe_u32(self.proc, body + OFF_CSHIP_ORGANISATION)
        experience_raw = safe_u32(self.proc, body + OFF_CSHIP_EXPERIENCE)
        ref100_raw = safe_u32(self.proc, body + OFF_CSHIP_REF100)
        if ref100_raw != CSHIP_REF100_RAW:
            errors.append("bad_cship_ref100_sentinel")
        if strength_raw is None or strength_raw > MAX_SHIP_STRENGTH_RAW:
            errors.append("bad_cship_strength_raw")
        if organisation_raw is None or organisation_raw > MAX_SHIP_ORGANISATION_RAW:
            errors.append("bad_cship_organisation_raw")
        if experience_raw is None or experience_raw > MAX_SHIP_EXPERIENCE_RAW:
            errors.append("bad_cship_experience_raw")
        return {
            "status": "valid" if not errors else "invalid",
            "cship_ptr": body,
            "cship_ptr_hex": hx(body),
            "type_tag": type_tag,
            "instance_id": instance_id,
            "strength_raw": strength_raw,
            "organisation_raw": organisation_raw,
            "experience_raw": experience_raw,
            "sunk_convoys": safe_u32(self.proc, body + OFF_CSHIP_SUNK_CONVOYS),
            "category_enum": safe_u32(self.proc, body + OFF_CSHIP_CATEGORY),
            "ref100_raw": ref100_raw,
            "errors": errors,
        }

    def attach_cship_resolution(self, rows: List[Dict[str, Any]]) -> Dict[str, Any]:
        non_convoy_ids = sorted({
            int(row.get("ship_instance_id") or 0)
            for row in rows
            if not row.get("is_convoy") and int(row.get("ship_instance_id") or 0) > 0
        })
        if not rows:
            return {"status": "not_run_no_member_rows"}
        if not non_convoy_ids:
            for row in rows:
                row["cship_status"] = "convoy_no_cship" if row.get("is_convoy") else "missing_instance_id"
            return {"status": "not_run_no_non_convoy_member_ids"}

        registry = self.scan_cship_registry()
        rows_by_id = registry.pop("rows_by_id", {})
        for row in rows:
            ship_instance_id = int(row.get("ship_instance_id") or 0)
            if row.get("is_convoy"):
                row["cship_status"] = "convoy_no_cship"
            elif ship_instance_id in rows_by_id:
                cship = rows_by_id[ship_instance_id]
                row.update({
                    "cship_status": "resolved",
                    "cship_ptr": cship["cship_ptr"],
                    "cship_ptr_hex": cship["cship_ptr_hex"],
                    "cship_strength_raw": cship["strength_raw"],
                    "cship_organisation_raw": cship["organisation_raw"],
                    "cship_experience_raw": cship["experience_raw"],
                    "cship_sunk_convoys": cship["sunk_convoys"],
                    "cship_category_enum": cship["category_enum"],
                })
            else:
                row["cship_status"] = "missing"
        registry["required_instance_ids"] = non_convoy_ids
        registry["resolved_required_count"] = sum(
            1 for instance_id in non_convoy_ids if instance_id in rows_by_id
        )
        registry["missing_required_instance_ids"] = [
            instance_id for instance_id in non_convoy_ids if instance_id not in rows_by_id
        ]
        return registry

    def validate_body(self, body: int) -> Dict[str, Any]:
        errors: List[str] = []
        body_vt = safe_u64(self.proc, body)
        packed = safe_u64(self.proc, body + OFF_BODY_HANDLE)
        if body_vt != self.vt_body:
            errors.append("bad_body_vtable")
        if packed is None:
            errors.append("missing_body_handle")
            packed = 0
        combat_type = packed & 0xFFFFFFFF
        combat_id = (packed >> 32) & 0xFFFFFFFF
        if combat_type != NAVAL_COMBAT_TYPE or combat_id == 0:
            errors.append("bad_body_handle")

        attacker = safe_u64(self.proc, body + OFF_BODY_ATTACKER_SIDE)
        defender = safe_u64(self.proc, body + OFF_BODY_DEFENDER_SIDE)
        front = safe_u64(self.proc, body + OFF_BODY_FRONT)
        for label, side in (("attacker", attacker), ("defender", defender)):
            if not is_heap(side) or safe_u64(self.proc, side or 0) != self.vt_side:
                errors.append("bad_%s_side_vtable" % label)

        registry = safe_u64(self.proc, (attacker or 0) + OFF_SIDE_REGISTRY) if is_heap(attacker) else None
        if not is_heap(registry) or safe_u64(self.proc, registry or 0) != self.vt_registry:
            errors.append("bad_registry_vtable")
        else:
            reg_handle = safe_u64(self.proc, registry + OFF_REG_HANDLE)
            reg_attacker = safe_u64(self.proc, registry + OFF_REG_ATTACKER_SIDE)
            reg_defender = safe_u64(self.proc, registry + OFF_REG_DEFENDER_SIDE)
            reg_front = safe_u64(self.proc, registry + OFF_REG_FRONT)
            if reg_handle != packed:
                errors.append("registry_handle_mismatch")
            if reg_attacker != attacker:
                errors.append("registry_attacker_mismatch")
            if reg_defender != defender:
                errors.append("registry_defender_mismatch")
            if is_heap(front) and reg_front != front:
                errors.append("registry_front_mismatch")
            if not is_heap(front):
                front = reg_front

        attacker_stats = self.validate_side(attacker)
        defender_stats = self.validate_side(defender)
        if attacker_stats.get("errors"):
            errors.append("attacker_side_invalid")
        if defender_stats.get("errors"):
            errors.append("defender_side_invalid")

        location = safe_u32(self.proc, (front or 0) + OFF_FRONT_LOCATION) if is_heap(front) else None
        two_sided_members = (
            attacker_stats.get("member_count", 0) > 0
            and defender_stats.get("member_count", 0) > 0
        )

        return {
            "status": "valid" if not errors else "invalid",
            "combat_id": combat_id,
            "combat_type": combat_type,
            "body_ptr": body,
            "location": location,
            "attacker": attacker_stats,
            "defender": defender_stats,
            "two_sided_members": two_sided_members,
            "errors": errors,
        }

    def validate_side(self, side: Optional[int]) -> Dict[str, Any]:
        errors: List[str] = []
        if not is_heap(side) or safe_u64(self.proc, side or 0) != self.vt_side:
            return {"status": "invalid", "member_count": 0, "errors": ["bad_side"]}

        country_data = safe_u64(self.proc, side + OFF_SIDE_COUNTRY_DATA)
        country_count = safe_u32(self.proc, side + OFF_SIDE_COUNTRY_SIZE)
        countries: List[int] = []
        if country_count is None or country_count > MAX_COUNTRIES_PER_SIDE:
            errors.append("bad_country_count")
        else:
            handles = read_ptr_array(self.proc, country_data or 0, country_count, MAX_COUNTRIES_PER_SIDE)
            if handles is None:
                errors.append("bad_country_vector")
            else:
                countries = [handle & 0xFFFFFFFF for handle in handles]

        group_data = safe_u64(self.proc, side + OFF_SIDE_GROUP_DATA)
        group_count = safe_u32(self.proc, side + OFF_SIDE_GROUP_SIZE)
        member_count = 0
        group_rows = []
        if group_count is None or group_count > MAX_GROUPS_PER_SIDE:
            errors.append("bad_group_count")
        else:
            groups = read_ptr_array(self.proc, group_data or 0, group_count, MAX_GROUPS_PER_SIDE)
            if groups is None:
                errors.append("bad_group_vector")
            else:
                for group_index, group in enumerate(groups):
                    row = self.validate_group(group, group_index)
                    group_rows.append(row)
                    member_count += int(row.get("member_count", 0))
                    if row.get("errors"):
                        errors.append("bad_group_%d" % group_index)

        leader_present = is_heap(safe_u64(self.proc, side + OFF_SIDE_LEADER))
        positioning = safe_f32(self.proc, side + OFF_SIDE_POSITIONING)
        return {
            "status": "valid" if not errors else "invalid",
            "country_tag_db_ids": countries,
            "group_count": group_count,
            "member_count": member_count,
            "leader_present": leader_present,
            "positioning": positioning,
            "groups": group_rows,
            "errors": errors,
        }

    def validate_group(self, group: Optional[int], group_index: int) -> Dict[str, Any]:
        errors: List[str] = []
        if not is_heap(group):
            return {
                "group_index": group_index,
                "member_count": 0,
                "errors": ["bad_group_ptr"],
            }
        member_data = safe_u64(self.proc, group + OFF_GROUP_MEMBERS_DATA)
        member_count = safe_u32(self.proc, group + OFF_GROUP_MEMBERS_SIZE)
        if member_count is None or member_count > MAX_MEMBERS_PER_GROUP:
            return {
                "group_index": group_index,
                "member_count": 0,
                "errors": ["bad_member_count"],
            }
        members = read_ptr_array(self.proc, member_data or 0, member_count, MAX_MEMBERS_PER_GROUP)
        if members is None:
            return {
                "group_index": group_index,
                "member_count": 0,
                "errors": ["bad_member_vector"],
            }
        member_samples = []
        member_rows = []
        for member_index, member in enumerate(members):
            if not is_heap(member) or safe_u64(self.proc, member) != self.vt_member:
                errors.append("bad_member_vtable")
                continue
            packed = safe_u64(self.proc, member + OFF_MEMBER_HANDLE) or 0
            owner_tag = safe_u32(self.proc, member + OFF_MEMBER_OWNER_TAG)
            unit_type_tag = packed & 0xFFFFFFFF
            ship_instance_id = (packed >> 32) & 0xFFFFFFFF
            is_convoy = unit_type_tag == 0 and ship_instance_id == 0
            row = {
                "member_index": member_index,
                "member_ptr": member,
                "member_ptr_hex": hx(member),
                "unit_type_tag": unit_type_tag,
                "ship_instance_id": ship_instance_id,
                "is_convoy": is_convoy,
                "owner_tag_db_id": owner_tag,
                "max_strength_raw": safe_u32(self.proc, member + OFF_MEMBER_MAX_STRENGTH_RAW),
                "slot_index": safe_u32(self.proc, member + OFF_MEMBER_SLOT),
                "state_bitmask": (safe_u32(self.proc, member + OFF_MEMBER_STATE) or 0) & 0xFF,
                "engagement_weight": safe_u32(self.proc, member + OFF_MEMBER_WEIGHT),
            }
            member_rows.append(row)
            member_samples.append({
                key: value for key, value in row.items()
                if key not in ("member_ptr",)
            })
        return {
            "group_index": group_index,
            "member_count": member_count,
            "sample_members": member_samples[:8],
            "member_rows": member_rows,
            "errors": errors,
        }


def bridge_rows(validated: List[Dict[str, Any]], require_two_sided: bool) -> List[Dict[str, int]]:
    rows = []
    for combat in validated:
        if combat.get("status") != "valid":
            continue
        if require_two_sided and not combat.get("two_sided_members"):
            continue
        rows.append({
            "combat_id": int(combat["combat_id"]),
            "body_ptr": int(combat["body_ptr"]),
        })
    return sorted(rows, key=lambda row: row["combat_id"])


def encode_bridge(rows: List[Dict[str, int]]) -> str:
    return ";".join("%d:0x%X" % (row["combat_id"], row["body_ptr"]) for row in rows)


def fixture_contract() -> Dict[str, Any]:
    return {
        "in_game_fixture": [
            "Use the current 870ebf hoi4.exe process.",
            "Create or load a visible active naval battle before starting recorder attach.",
            "Preferred fixture: fleet-vs-fleet surface battle with non-empty attacker and defender ship/member rows.",
            "For lifecycle-end proof, keep the battle active long enough to attach, then let that same battle end during the capture window.",
        ],
        "success_probe": (
            "python memory_reader\\combat\\naval_combat_live_fixture_probe.py --pid <pid> "
            "--require-two-sided --out-dir extractor\native\\runs\\naval_combat_live_<fixture>"
        ),
        "success_criteria": [
            "root.sha256_ok == true",
            "scan.status == scanned",
            "status == validated_active_fixture",
            "bridge_row_count >= 1",
            "member_bridge_row_count >= 2 with attacker and defender CFEXMember rows",
            "non_convoy_member_count >= 1 and cship_unresolved_member_count == 0 when --require-cship-join is active",
            "each bridge row validates CNavalCombat body vtable, packed type/id, both side vtables, registry vtable, registry handle/order/front, and CFEXMember vectors",
            "attacker.member_count > 0 and defender.member_count > 0 when --require-two-sided is used",
            "each non-convoy member resolves CFEXMember+0x08 high u32 ship_instance_id through current CShip module+0x2941748",
        ],
        "recorder_followup": [
            "Use bridge.txt as the candidate naval_combat_body_bridge payload for the existing DLL bridge path.",
            "A production acceptance run must emit raw naval_air_combats and naval_combat_members, replay-verify, advance hourly-or-better at current-equivalent speed, skip zero hours, and show naval_combat_ended if the fixture is the ending fixture.",
        ],
        "rejection_criteria": [
            "body vtable hits exist but no candidate passes the registry/side/member validation chain",
            "candidate passes body/side registry checks but cannot produce member rows for either side under the fleet-vs-fleet fixture",
            "candidate produces non-convoy CFEXMember rows whose ship_instance_id cannot resolve to current CShip module+0x2941748",
            "validated bridge rows produce DLL gap rows or fail replay verification in the recorder followup",
        ],
    }


def run(args: argparse.Namespace) -> Dict[str, Any]:
    probe = NavalCombatFixtureProbe(args.pid, args.workers)
    try:
        root = probe.root()
        report: Dict[str, Any] = {
            "probe": "naval_combat_live_fixture_probe_v1",
            "created_utc": dt.datetime.utcnow().replace(microsecond=0).isoformat() + "Z",
            "read_only": True,
            "root": root,
            "candidate_route": {
                "body_vtable": "module+0x29B6660",
                "side_vtable": "module+0x29BC908",
                "registry_vtable": "module+0x29B6588",
                "member_vtable": "module+0x29F9608",
                "cship_vtable": "module+0x2941748",
                "body_chain": "body+0x18/+0x20 sides; side+0x18 registry; registry+0x28/+0x30 sides; registry+0x38 front",
                "member_chain": "side+0xC8 group vector; group+0x08 member vector; group+0x14 member count",
                "member_cship_join": "CFEXMember+0x08 high u32 ship_instance_id -> CShip+0x08 high u32 instance_id; CShip vtable module+0x2941748",
            },
            "fixture_contract": fixture_contract(),
        }
        if not root["sha256_ok"]:
            report.update({
                "status": "build_mismatch",
                "validated_combat_count": 0,
                "bridge_row_count": 0,
                "bridge": "",
                "validated_combats": [],
            })
            return report
        scan = probe.scan_body_vtable()
        report["scan"] = {
            k: v for k, v in scan.items()
            if k != "hits"
        }
        hits = list(scan.get("hits") or [])
        validated = [probe.validate_body(hit) for hit in hits]
        rows = bridge_rows(validated, args.require_two_sided)
        member_rows = member_bridge_rows(validated, args.require_two_sided)
        cship_registry = probe.attach_cship_resolution(member_rows)
        member_summary = summarize_member_bridge_rows(member_rows)
        if scan.get("status") != "scanned":
            status = "scan_failed"
        elif not hits:
            status = "fixture_absent"
        elif rows:
            status = "validated_active_fixture"
        elif any(row.get("status") == "valid" for row in validated):
            status = "validated_but_not_two_sided" if args.require_two_sided else "validated_without_bridge_rows"
        else:
            status = "candidate_rejected"
        if status == "validated_active_fixture" and args.require_cship_join:
            if member_summary["non_convoy_member_count"] == 0:
                status = "validated_member_rows_missing_cship_fixture"
            elif member_summary["cship_unresolved_member_count"] > 0:
                status = "candidate_rejected_cship_member_join"
        report.update({
            "status": status,
            "body_hit_count": len(hits),
            "validated_combat_count": sum(1 for row in validated if row.get("status") == "valid"),
            "bridge_row_count": len(rows),
            "bridge": encode_bridge(rows),
            "member_bridge_row_count": member_summary["member_bridge_row_count"],
            "non_convoy_member_count": member_summary["non_convoy_member_count"],
            "convoy_member_count": member_summary["convoy_member_count"],
            "cship_resolved_member_count": member_summary["cship_resolved_member_count"],
            "cship_unresolved_member_count": member_summary["cship_unresolved_member_count"],
            "cship_registry": cship_registry,
            "member_bridge_rows": member_rows,
            "member_bridge_rows_sample": member_rows[:32],
            "validated_combats": [
                {
                    key: value for key, value in row.items()
                    if key not in ("body_ptr",)
                }
                for row in validated[:128]
            ],
            "address_manifest_policy": "body addresses are written to bridge.txt; member and CShip addresses are written to member_bridge.jsonl when out-dir is supplied",
        })
        return report
    finally:
        probe.close()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, required=True)
    ap.add_argument("--workers", type=int, default=4)
    ap.add_argument("--require-two-sided", action="store_true")
    ap.add_argument("--require-cship-join", action=argparse.BooleanOptionalAction, default=True)
    ap.add_argument("--out-dir", default=None)
    args = ap.parse_args()

    report = run(args)
    out_path = None
    if args.out_dir:
        out_dir = Path(args.out_dir)
        out_dir.mkdir(parents=True, exist_ok=True)
        out_path = out_dir / "naval_combat_live_fixture_probe.json"
        out_path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
        bridge = report.get("bridge") or ""
        (out_dir / "bridge.txt").write_text(bridge, encoding="ascii")
        member_bridge = report.get("member_bridge_rows") or []
        (out_dir / "member_bridge.jsonl").write_text(
            encode_member_bridge_jsonl(member_bridge),
            encoding="utf-8",
        )

    summary = {
        "out_path": str(out_path) if out_path else None,
        "status": report.get("status"),
        "sha256_ok": report.get("root", {}).get("sha256_ok"),
        "body_hit_count": report.get("body_hit_count", 0),
        "validated_combat_count": report.get("validated_combat_count", 0),
        "bridge_row_count": report.get("bridge_row_count", 0),
        "member_bridge_row_count": report.get("member_bridge_row_count", 0),
        "cship_resolved_member_count": report.get("cship_resolved_member_count", 0),
        "cship_unresolved_member_count": report.get("cship_unresolved_member_count", 0),
    }
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0 if report.get("status") in (
        "fixture_absent",
        "validated_active_fixture",
        "validated_but_not_two_sided",
        "validated_member_rows_missing_cship_fixture",
        "candidate_rejected",
        "candidate_rejected_cship_member_join",
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
