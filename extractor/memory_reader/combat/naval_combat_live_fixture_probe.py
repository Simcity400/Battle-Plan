"""Bounded fixture verifier for current-build live naval combat roots.

This probe is read-only. It is not a production hot-loop scanner. Its job is to
turn the remaining naval_combat_live blocker into an exact fixture contract:
when the user causes an active naval battle, this verifies the
CNavalCombat -> CNavalCombatant -> CFEXGroup -> CFEXMember chain and emits a
bridge row file that can drive the existing DLL bridge.

Member chain (CRACKED 2026-07-03 on the paused D04 fixture; replaces the
REJECTED side+0x2A0/+0x2A8/+0x2B0 inline group-vector walk that resolved a
308-element non-CFEXMember allocation):

- Members live in CFEXGroup objects (vtable module+0x2A42D18, RTTI
  ``.?AVCFEXGroup@@``; the RTTI names of ALL pinned vtables are re-verified at
  attach and the probe fails closed on any mismatch).
- AUTHORITATIVE walk: heap-scan for the CFEXGroup vtable and bucket every group
  by its owning-side back-ref at group+0x38. A group belongs to the
  CNavalCombatant its +0x38 equals; groups pointing at non-naval objects belong
  to other combat types and are ignored.
- ADVISORY cross-check: side+0x20 sometimes leads to a child object holding
  CFEXGroup pointers whose +0x20 next-chain is CIRCULAR and spans BOTH sides.
  When reachable, every forward-walked group must already be in the back-ref
  buckets (a forward-only group means the back-ref scan missed live data =>
  hard error). The child is absent/unreadable for some live battles, so forward
  reachability itself is NOT required.
- Group layout: +0x08 member ptr array begin, +0x10 u32 capacity, +0x14 u32
  member count (count <= capacity), +0x20 next group, +0x38 owning side.
  Empty groups (count == 0, begin == null) are valid.
- Null member slots are skipped (recorded, never counted); a non-null slot that
  is not a CFEXMember is a hard error. A CFEXMember with packed handle 0 is a
  CONVOY member (matches save ``convoy={...}`` rows).

Save-oracle validation (D04 1944-02-11, 4 battles): 4/4 combat ids, 4/4
front+0xA4 locations, 8/8 sides' ship ids, and member state (+0xE0) /
slot (+0x120) multisets all matched the same-tick melted save
(extractor/native/runs/nav49_50_port_validation/).
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

# Vtable RVAs refreshed via RTTI complete-object-locator rediscovery against the
# live Bay-of-Biscay battle on 870ebf (2026-07-03; rtti_vtable_rediscover.py):
# CNavalCombat 0x29c6f18 (6 live bodies), CFEXMember 0x2a0a228 (64 bodies),
# CShip 0x2941748 (unchanged, still valid). CNavalCombatant 0x29cd298 (17 bodies)
# is the side/participant object; the prior 0x29B6660/0x29BC908/0x29B6588/
# 0x29F9608 constants predated 870ebf and matched zero live bodies.
# CFEXGroup 0x2a42d18 rediscovered by RTTI name on the paused D04 fixture
# (2026-07-03, runs/nav49_50_port_validation/live_backref_precheck.json); ALL
# pinned RVAs below are re-verified by RTTI name at attach (fail closed).
NAVAL_COMBAT_VTABLE_RVA = 0x29C6F18
NAVAL_COMBAT_SIDE_VTABLE_RVA = 0x29CD298
NAVAL_COMBAT_REGISTRY_VTABLE_RVA = None
NAVAL_COMBAT_MEMBER_VTABLE_RVA = 0x2A0A228
NAVAL_COMBAT_GROUP_VTABLE_RVA = 0x2A42D18
# body+0x38 points at the battle-location CProvince object (RTTI-confirmed on
# all 4 D04 battles); CProvince+0xA4 is its province id.
CPROVINCE_VTABLE_RVA = 0x295AF68
CURRENT_870EBF_CSHIP_VTABLE_RVA = 0x2941748

EXPECTED_VTABLE_RTTI = {
    "body": (NAVAL_COMBAT_VTABLE_RVA, ".?AVCNavalCombat@@"),
    "side": (NAVAL_COMBAT_SIDE_VTABLE_RVA, ".?AVCNavalCombatant@@"),
    "member": (NAVAL_COMBAT_MEMBER_VTABLE_RVA, ".?AVCFEXMember@@"),
    "group": (NAVAL_COMBAT_GROUP_VTABLE_RVA, ".?AVCFEXGroup@@"),
    "province": (CPROVINCE_VTABLE_RVA, ".?AVCProvince@@"),
    "cship": (CURRENT_870EBF_CSHIP_VTABLE_RVA, ".?AVCShip@@"),
}

NAVAL_COMBAT_TYPE = 0x1269
CSHIP_TYPE = 51

OFF_BODY_HANDLE = 0x18
OFF_BODY_ATTACKER_SIDE = 0x28
OFF_BODY_DEFENDER_SIDE = 0x30
OFF_BODY_FRONT = 0x38

OFF_SIDE_PARENT_BODY = 0x18
# side+0x68/+0x74 "country vector" is UNVALIDATED on 870ebf (side+0x68 is also
# one of the mirrored forward-child pointers); it is reported as a diagnostic
# candidate only and never gates validation or feeds fixture rows.
OFF_SIDE_COUNTRY_DATA = 0x68
OFF_SIDE_COUNTRY_SIZE = 0x74
# Advisory forward route: side child object that may hold CFEXGroup pointers.
OFF_SIDE_FORWARD_CHILD = 0x20
OFF_SIDE_LEADER = 0x110
# Exact producer outputs written by FUN_14160BE90. The former +0x288 f32
# diagnostic is not the persisted positioning authority.
OFF_SIDE_POSITIONING_RAW = 0x138
OFF_SIDE_ANTI_AIR_RAW = 0x150

# body+0x38 -> CProvince (vtable-checked); CProvince+0xA4 u32 = province id.
# VALIDATED 4/4 against the same-tick D04 save (locations 6434/2804/1032/8819)
# and PROMOTED to the compact location_id.
OFF_FRONT_LOCATION = 0xA4

OFF_GROUP_MEMBERS_DATA = 0x08
OFF_GROUP_MEMBERS_CAP = 0x10
OFF_GROUP_MEMBERS_SIZE = 0x14
OFF_GROUP_NEXT = 0x20
OFF_GROUP_OWNING_SIDE = 0x38
FORWARD_CHILD_SCAN_LEN = 0x400
MAX_GROUP_CHAIN_HOPS = 128

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
MAX_GROUP_HITS = 16384
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


def safe_i64(proc: winmem.Process, addr: int) -> Optional[int]:
    raw = safe_u64(proc, addr)
    if raw is None:
        return None
    return raw - (1 << 64) if raw >= (1 << 63) else raw


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
        self.module_end = self.base + self.proc.module_size
        self.vt_body = self.base + NAVAL_COMBAT_VTABLE_RVA
        self.vt_side = self.base + NAVAL_COMBAT_SIDE_VTABLE_RVA
        self.vt_registry = None
        self.vt_member = self.base + NAVAL_COMBAT_MEMBER_VTABLE_RVA
        self.vt_group = self.base + NAVAL_COMBAT_GROUP_VTABLE_RVA
        self.vt_province = self.base + CPROVINCE_VTABLE_RVA
        self.vt_cship = self.base + CURRENT_870EBF_CSHIP_VTABLE_RVA
        self._rtti_cache: Dict[int, Optional[str]] = {}

    def close(self) -> None:
        self.proc.close()

    def rtti_name(self, vtable: int) -> Optional[str]:
        """MSVC RTTI: vtable[-1] -> complete object locator -> type descriptor name."""
        if vtable in self._rtti_cache:
            return self._rtti_cache[vtable]
        name: Optional[str] = None
        try:
            col = self.proc.u64(vtable - 8)
            if self.base <= col < self.module_end:
                td_rva = self.proc.u32(col + 0x0C)
                raw = self.proc.read(self.base + td_rva + 0x10, 64)
                text = raw.split(b"\x00", 1)[0]
                if text.startswith(b".?AV") or text.startswith(b".?AU"):
                    name = text.decode("ascii", "replace")
        except OSError:
            name = None
        self._rtti_cache[vtable] = name
        return name

    def verify_vtable_rtti(self) -> Dict[str, Any]:
        """Re-verify every pinned vtable RVA by RTTI class name (fail closed)."""
        checks: Dict[str, Any] = {}
        ok = True
        for label, (rva, expected) in EXPECTED_VTABLE_RTTI.items():
            got = self.rtti_name(self.base + rva)
            row_ok = got == expected
            ok = ok and row_ok
            checks[label] = {
                "rva": hex(rva),
                "expected": expected,
                "got": got,
                "ok": row_ok,
            }
        return {"ok": ok, "checks": checks}

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

    def _scan_vtable(self, name: str, rva: int, vtable: int, max_hits: int) -> Dict[str, Any]:
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

        spec = fastscan.make_qword_eq_matcher(vtable)
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
                "name": name,
                "rva": hex(rva),
                "vtable": hx(vtable),
            },
            "elapsed_s": round(elapsed, 3),
            "hit_count": len(hits),
            "hits": hits[:max_hits],
            "truncated": len(hits) > max_hits,
        }

    def scan_body_vtable(self) -> Dict[str, Any]:
        return self._scan_vtable(
            "CNavalCombat_vtable", NAVAL_COMBAT_VTABLE_RVA, self.vt_body, MAX_BODY_HITS
        )

    def scan_group_vtable(self) -> Dict[str, Any]:
        return self._scan_vtable(
            "CFEXGroup_vtable", NAVAL_COMBAT_GROUP_VTABLE_RVA, self.vt_group, MAX_GROUP_HITS
        )

    def bucket_groups_by_owning_side(self, group_hits: List[int]) -> Dict[int, List[int]]:
        """AUTHORITATIVE side attribution: bucket every CFEXGroup by group+0x38.

        Groups whose back-ref is not one of this battle set's validated
        CNavalCombatant sides simply never get looked up (they belong to other
        combat types that share the CFEXGroup container).
        """
        buckets: Dict[int, List[int]] = {}
        for group in group_hits:
            side_ref = safe_u64(self.proc, group + OFF_GROUP_OWNING_SIDE)
            if side_ref is None:
                continue
            buckets.setdefault(side_ref, []).append(group)
        for side_ref in buckets:
            buckets[side_ref].sort()
        return buckets

    def forward_group_set(self, side: Optional[int]) -> Tuple[set, str]:
        """Advisory forward route: scan the side+0x20 child for CFEXGroup ptrs
        and walk the circular +0x20 next-chain (cycle-guarded). The child is
        absent for some live battles; that alone is not an error."""
        found: set = set()
        if not is_heap(side):
            return found, "no_side"
        child = safe_u64(self.proc, side + OFF_SIDE_FORWARD_CHILD)
        if not is_heap(child):
            return found, "no_child"
        raw = self.proc.try_read(child, FORWARD_CHILD_SCAN_LEN)
        if not raw:
            return found, "child_unreadable"
        seeds = []
        for off in range(0, len(raw) - 7, 8):
            value = int.from_bytes(raw[off:off + 8], "little")
            if is_heap(value) and safe_u64(self.proc, value) == self.vt_group:
                seeds.append(value)
        for seed in seeds:
            cursor = seed
            hops = 0
            while is_heap(cursor) and cursor not in found and hops < MAX_GROUP_CHAIN_HOPS:
                if safe_u64(self.proc, cursor) != self.vt_group:
                    break
                found.add(cursor)
                cursor = safe_u64(self.proc, cursor + OFF_GROUP_NEXT) or 0
                hops += 1
        return found, "ok"

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

    def validate_body(self, body: int, groups_by_side: Dict[int, List[int]]) -> Dict[str, Any]:
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
            elif safe_u64(self.proc, (side or 0) + OFF_SIDE_PARENT_BODY) != body:
                errors.append("%s_side_parent_mismatch" % label)

        attacker_stats = self.validate_side(attacker, body, groups_by_side)
        defender_stats = self.validate_side(defender, body, groups_by_side)
        if attacker_stats.get("errors"):
            errors.append("attacker_side_invalid")
        if defender_stats.get("errors"):
            errors.append("defender_side_invalid")

        # Advisory forward cross-check: any group reachable via the side child
        # chain that the back-ref buckets did NOT attribute to this battle
        # means the authoritative scan missed live data -> fail closed.
        bucket_union = set()
        for side in (attacker, defender):
            if is_heap(side):
                bucket_union.update(groups_by_side.get(side, []))
        forward_union = set()
        forward_status = {}
        for label, side in (("attacker", attacker), ("defender", defender)):
            found, status = self.forward_group_set(side)
            forward_union.update(found)
            forward_status[label] = {"status": status, "group_count": len(found)}
        forward_only = sorted(forward_union - bucket_union)
        if forward_only:
            errors.append("forward_walk_found_group_missing_from_backref_scan")

        # Stale-group guard: the same ship as a live member of one combat twice
        # (within or across sides/groups) means the back-ref scan bucketed a
        # stale/duplicated CFEXGroup -> fail closed.
        seen_ship_ids: set = set()
        duplicate_ship_ids: List[int] = []
        for stats in (attacker_stats, defender_stats):
            for group_row in stats.get("groups") or []:
                for member_row in group_row.get("member_rows") or []:
                    ship_id = int(member_row.get("ship_instance_id") or 0)
                    if ship_id == 0:
                        continue
                    if ship_id in seen_ship_ids:
                        duplicate_ship_ids.append(ship_id)
                    seen_ship_ids.add(ship_id)
        if duplicate_ship_ids:
            errors.append("duplicate_ship_instance_id_across_member_groups")

        # body+0x38 must be the battle-location CProvince (vtable-checked); a
        # wrong/stale front object fails the combat closed instead of emitting
        # a plausible-but-unverified province id.
        location = None
        if not is_heap(front) or safe_u64(self.proc, front or 0) != self.vt_province:
            errors.append("bad_front_province_vtable")
        else:
            location = safe_u32(self.proc, front + OFF_FRONT_LOCATION)
            if not location:
                errors.append("missing_front_location_province")
                location = None
        two_sided_members = (
            attacker_stats.get("member_count", 0) > 0
            and defender_stats.get("member_count", 0) > 0
        )

        return {
            "status": "valid" if not errors else "invalid",
            "combat_id": combat_id,
            "combat_type": combat_type,
            "body_ptr": body,
            "front_ptr": front,
            "front_ptr_hex": hx(front),
            # PROMOTED: validated 4/4 against the same-tick D04 save oracle.
            "location_front_0xA4": location,
            "location_front_0xA4_validated": True,
            "forward_cross_check": {
                "sides": forward_status,
                "forward_group_count": len(forward_union),
                "backref_group_count": len(bucket_union),
                "forward_only_groups": [hx(g) for g in forward_only],
            },
            "duplicate_ship_instance_ids": sorted(set(duplicate_ship_ids)),
            "attacker": attacker_stats,
            "defender": defender_stats,
            "two_sided_members": two_sided_members,
            "errors": errors,
        }

    def validate_side(
        self,
        side: Optional[int],
        parent_body: int,
        groups_by_side: Dict[int, List[int]],
    ) -> Dict[str, Any]:
        errors: List[str] = []
        if not is_heap(side) or safe_u64(self.proc, side or 0) != self.vt_side:
            return {"status": "invalid", "member_count": 0, "errors": ["bad_side"]}
        if safe_u64(self.proc, side + OFF_SIDE_PARENT_BODY) != parent_body:
            errors.append("parent_body_mismatch")

        # Diagnostic only: the side+0x68/+0x74 "country vector" interpretation
        # is UNVALIDATED on 870ebf (side+0x68 doubles as a forward-child
        # mirror). Report the candidate values, never gate on them, never feed
        # them into fixture rows.
        country_data = safe_u64(self.proc, side + OFF_SIDE_COUNTRY_DATA)
        country_count = safe_u32(self.proc, side + OFF_SIDE_COUNTRY_SIZE)
        countries: List[int] = []
        if country_count is not None and country_count <= MAX_COUNTRIES_PER_SIDE:
            handles = read_ptr_array(self.proc, country_data or 0, country_count, MAX_COUNTRIES_PER_SIDE)
            if handles is not None:
                countries = [handle & 0xFFFFFFFF for handle in handles]

        # AUTHORITATIVE member walk: CFEXGroups bucketed by group+0x38 back-ref.
        groups = list(groups_by_side.get(side, []))
        member_count = 0
        null_slot_count = 0
        group_rows = []
        if len(groups) > MAX_GROUPS_PER_SIDE:
            errors.append("bad_group_count")
            groups = []
        for group_index, group in enumerate(groups):
            row = self.validate_group(group, group_index, side)
            group_rows.append(row)
            member_count += len(row.get("member_rows") or [])
            null_slot_count += int(row.get("null_slot_count") or 0)
            if row.get("errors"):
                errors.append("bad_group_%d" % group_index)

        leader_present = is_heap(safe_u64(self.proc, side + OFF_SIDE_LEADER))
        positioning_first = safe_i64(self.proc, side + OFF_SIDE_POSITIONING_RAW)
        positioning_second = safe_i64(self.proc, side + OFF_SIDE_POSITIONING_RAW)
        anti_air_first = safe_i64(self.proc, side + OFF_SIDE_ANTI_AIR_RAW)
        anti_air_second = safe_i64(self.proc, side + OFF_SIDE_ANTI_AIR_RAW)
        if positioning_first is None or positioning_first != positioning_second:
            errors.append("positioning_raw_unreadable_or_unstable")
        elif not 0 <= positioning_first <= 100_000:
            errors.append("positioning_raw_out_of_range")
        if anti_air_first is None or anti_air_first != anti_air_second:
            errors.append("anti_air_raw_unreadable_or_unstable")
        return {
            "status": "valid" if not errors else "invalid",
            "country_tag_db_ids_candidate_unvalidated": countries,
            "group_route": "CFEXGroup heap scan bucketed by group+0x38 owning-side back-ref",
            "group_count": len(groups),
            "member_count": member_count,
            "null_slot_count": null_slot_count,
            "leader_present": leader_present,
            "positioning_raw": positioning_first,
            "anti_air_raw": anti_air_first,
            "groups": group_rows,
            "errors": errors,
        }

    def validate_group(self, group: Optional[int], group_index: int, owning_side: int) -> Dict[str, Any]:
        errors: List[str] = []
        if not is_heap(group):
            return {
                "group_index": group_index,
                "member_count": 0,
                "errors": ["bad_group_ptr"],
            }
        if safe_u64(self.proc, group) != self.vt_group:
            return {
                "group_index": group_index,
                "member_count": 0,
                "errors": ["bad_group_vtable"],
            }
        side_backref = safe_u64(self.proc, group + OFF_GROUP_OWNING_SIDE)
        if side_backref != owning_side:
            # The bucket lookup already keyed on +0x38; a mismatch here means
            # the group mutated between reads -> fail closed.
            errors.append("group_owning_side_backref_mismatch")
        member_data = safe_u64(self.proc, group + OFF_GROUP_MEMBERS_DATA)
        member_cap = safe_u32(self.proc, group + OFF_GROUP_MEMBERS_CAP)
        member_count = safe_u32(self.proc, group + OFF_GROUP_MEMBERS_SIZE)
        next_group = safe_u64(self.proc, group + OFF_GROUP_NEXT)
        if (
            member_count is None
            or member_cap is None
            or member_count > member_cap
            or member_cap > MAX_MEMBERS_PER_GROUP
            or member_count > MAX_MEMBERS_PER_GROUP
        ):
            return {
                "group_index": group_index,
                "member_count": 0,
                "errors": errors + ["bad_member_count"],
            }
        if member_count == 0:
            # Empty CFEXGroups (count == 0, begin == null) are live-observed
            # and valid (D04 battle 151343 defender).
            members: List[int] = []
        else:
            members_read = read_ptr_array(self.proc, member_data or 0, member_count, MAX_MEMBERS_PER_GROUP)
            if members_read is None:
                return {
                    "group_index": group_index,
                    "member_count": 0,
                    "errors": errors + ["bad_member_vector"],
                }
            members = members_read
        member_samples = []
        member_rows = []
        null_slot_count = 0
        for member_index, member in enumerate(members):
            if member == 0:
                # Null slots inside count are skipped, never emitted, and never
                # silently mistaken for convoy/ship members.
                null_slot_count += 1
                continue
            if not is_heap(member) or safe_u64(self.proc, member) != self.vt_member:
                errors.append("bad_member_vtable")
                continue
            packed = safe_u64(self.proc, member + OFF_MEMBER_HANDLE)
            if packed is None:
                # An unreadable handle must fail closed; `or 0` would silently
                # classify a torn/freed member as a convoy row.
                errors.append("unreadable_member_handle")
                continue
            owner_tag = safe_u32(self.proc, member + OFF_MEMBER_OWNER_TAG)
            unit_type_tag = packed & 0xFFFFFFFF
            ship_instance_id = (packed >> 32) & 0xFFFFFFFF
            # Packed handle 0 == convoy member (matches save convoy={...} rows
            # on the D04 oracle; ship members carry (ship_id<<32)|51).
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
            "group_ptr_hex": hx(group),
            "owning_side_backref_hex": hx(side_backref),
            "next_group_hex": hx(next_group),
            "member_count": member_count,
            "member_capacity": member_cap,
            "null_slot_count": null_slot_count,
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
            "vtable_rtti.ok == true (every pinned vtable RVA re-verified by RTTI class name at attach)",
            "scan.status == scanned and group_scan.status == scanned",
            "status == validated_active_fixture",
            "bridge_row_count >= 1",
            "member_bridge_row_count >= 2 with attacker and defender CFEXMember rows",
            "non_convoy_member_count >= 1 and cship_unresolved_member_count == 0 when --require-cship-join is active",
            "each bridge row validates CNavalCombat body vtable, packed type/id, both CNavalCombatant side vtables, side parent back-refs, and the CFEXGroup back-ref member walk (group+0x38 owning side, group+0x08/+0x10/+0x14 member array)",
            "no forward-walked CFEXGroup (side+0x20 child, circular +0x20 chain) is missing from the back-ref buckets",
            "location_front_0xA4 is the PROMOTED battle location province (validated 4/4 vs the same-tick D04 save oracle)",
            "attacker.member_count > 0 and defender.member_count > 0 when --require-two-sided is used",
            "each non-convoy member resolves CFEXMember+0x08 high u32 ship_instance_id through current CShip module+0x2941748",
        ],
        "recorder_followup": [
            "Use bridge.txt as the candidate naval_combat_body_bridge payload for the existing DLL bridge path.",
            "A production acceptance run must emit raw naval_air_combats and naval_combat_members, replay-verify, advance hourly-or-better at current-equivalent speed, skip zero hours, and show naval_combat_ended if the fixture is the ending fixture.",
        ],
        "rejection_criteria": [
            "any pinned vtable RVA fails the attach-time RTTI class-name check (status vtable_rtti_mismatch)",
            "body vtable hits exist but no candidate passes the side/member validation chain",
            "candidate passes body/side checks but cannot produce member rows for either side under the fleet-vs-fleet fixture",
            "a forward-walked CFEXGroup is missing from the back-ref buckets (back-ref scan dropped live data)",
            "candidate produces non-convoy CFEXMember rows whose ship_instance_id cannot resolve to current CShip module+0x2941748",
            "validated bridge rows produce DLL gap rows or fail replay verification in the recorder followup",
        ],
    }


def run(args: argparse.Namespace) -> Dict[str, Any]:
    probe = NavalCombatFixtureProbe(args.pid, args.workers)
    try:
        root = probe.root()
        report: Dict[str, Any] = {
            "probe": "naval_combat_live_fixture_probe_v2_cfexgroup_backref",
            "created_utc": dt.datetime.utcnow().replace(microsecond=0).isoformat() + "Z",
            "read_only": True,
            "root": root,
            "candidate_route": {
                "body_vtable": "module+0x29C6F18",
                "side_vtable": "module+0x29CD298",
                "registry_vtable": None,
                "member_vtable": "module+0x2A0A228",
                "group_vtable": "module+0x2A42D18 (CFEXGroup, RTTI-verified at attach)",
                "cship_vtable": "module+0x2941748",
                "body_chain": "body+0x28/+0x30 sides; body+0x38 front; side+0x18 parent CNavalCombat back-ref",
                "location": "body+0x38 front +0xA4 u32 battle location province; PROMOTED (validated 4/4 vs same-tick D04 save oracle)",
                "member_chain": "heap-scan CFEXGroup vtable; bucket by group+0x38 owning-side back-ref (AUTHORITATIVE); group+0x08 member ptr array; +0x10 cap; +0x14 count; +0x20 circular next-chain used only as an advisory forward cross-check from side+0x20",
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
        rtti = probe.verify_vtable_rtti()
        report["vtable_rtti"] = rtti
        if not rtti["ok"]:
            report.update({
                "status": "vtable_rtti_mismatch",
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
        group_scan = probe.scan_group_vtable()
        report["group_scan"] = {
            k: v for k, v in group_scan.items()
            if k != "hits"
        }
        group_hits = list(group_scan.get("hits") or [])
        groups_by_side = probe.bucket_groups_by_owning_side(group_hits)
        if scan.get("status") == "scanned" and group_scan.get("status") != "scanned":
            report.update({
                "status": "scan_failed",
                "body_hit_count": len(hits),
                "validated_combat_count": 0,
                "bridge_row_count": 0,
                "bridge": "",
                "validated_combats": [],
            })
            return report
        if scan.get("truncated") or group_scan.get("truncated"):
            # A truncated hit list can silently drop live bodies/groups; that
            # violates fail-closed completeness, so it is terminal.
            report.update({
                "status": "scan_truncated",
                "body_hit_count": len(hits),
                "validated_combat_count": 0,
                "bridge_row_count": 0,
                "bridge": "",
                "validated_combats": [],
            })
            return report
        validated = [probe.validate_body(hit, groups_by_side) for hit in hits]
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
        rejected_bridge = ""
        rejected_member_rows: List[Dict[str, Any]] = []
        rejected_bridge_row_count = 0
        rejected_member_bridge_row_count = 0
        if status in {
            "candidate_rejected_cship_member_join",
            "validated_member_rows_missing_cship_fixture",
        }:
            rejected_bridge = encode_bridge(rows)
            rejected_member_rows = list(member_rows)
            rejected_bridge_row_count = len(rows)
            rejected_member_bridge_row_count = member_summary["member_bridge_row_count"]
            rows = []
            member_rows = []
            member_summary = summarize_member_bridge_rows(member_rows)
        report.update({
            "status": status,
            "body_hit_count": len(hits),
            "validated_combat_count": 0
            if status in {
                "candidate_rejected_cship_member_join",
                "validated_member_rows_missing_cship_fixture",
            }
            else sum(1 for row in validated if row.get("status") == "valid"),
            "bridge_row_count": len(rows),
            "bridge": encode_bridge(rows),
            "rejected_bridge_row_count": rejected_bridge_row_count,
            "rejected_bridge": rejected_bridge,
            "member_bridge_row_count": member_summary["member_bridge_row_count"],
            "rejected_member_bridge_row_count": rejected_member_bridge_row_count,
            "rejected_member_bridge_rows": rejected_member_rows,
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
            "address_manifest_policy": "bridge.txt and member_bridge.jsonl are written only for status=validated_active_fixture; required CShip join failures may write bridge.REJECTED.txt/member_bridge.REJECTED.jsonl diagnostics only",
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
        # Bridge artifacts must reflect THIS run only: a stale bridge.txt from
        # an earlier validated poll must never survive a failed rerun in the
        # same out-dir (it would be reusable as if the required join passed).
        for stale in (
            "bridge.txt",
            "member_bridge.jsonl",
            "bridge.REJECTED.txt",
            "member_bridge.REJECTED.jsonl",
        ):
            (out_dir / stale).unlink(missing_ok=True)
        out_path = out_dir / "naval_combat_live_fixture_probe.json"
        out_path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
        if report.get("status") == "validated_active_fixture":
            bridge = report.get("bridge") or ""
            (out_dir / "bridge.txt").write_text(bridge, encoding="ascii")
            member_bridge = report.get("member_bridge_rows") or []
            (out_dir / "member_bridge.jsonl").write_text(
                encode_member_bridge_jsonl(member_bridge),
                encoding="utf-8",
            )
        elif report.get("status") in {
            "candidate_rejected_cship_member_join",
            "validated_member_rows_missing_cship_fixture",
        }:
            rejected_bridge = report.get("rejected_bridge") or ""
            if rejected_bridge:
                (out_dir / "bridge.REJECTED.txt").write_text(
                    rejected_bridge,
                    encoding="ascii",
                )
            rejected_member_bridge = report.get("rejected_member_bridge_rows") or []
            if rejected_member_bridge:
                (out_dir / "member_bridge.REJECTED.jsonl").write_text(
                    encode_member_bridge_jsonl(rejected_member_bridge),
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
        "candidate_rejected",
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
