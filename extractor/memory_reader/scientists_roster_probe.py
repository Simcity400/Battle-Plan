"""Bounded live probe for the Ghidra-backed CScientistRoster route.

Read-only: uses winmem.Process, which opens PROCESS_VM_READ |
PROCESS_QUERY_INFORMATION only. This is intentionally isolated from the
production collector while projects_live hardening is in flight.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import dataclass
from typing import Iterable

if __package__ in (None, ""):
    sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from memory_reader.winmem import Process, find_pid  # noqa: E402
import memory_reader.projects_live as projects_live  # noqa: E402


CG_STATE_DATE_OFFSET = 0x3316E90
CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C
CGAMESTATE_SCIENTIST_ROSTER_PRIMARY = 0x520
CGAMESTATE_SCIENTIST_ROSTER_FALLBACK = 0x524

COUNTRY_VTABLE_RVA = 0x27D0E90
COUNTRY_OFF_TAG_DB_ID = 0x08
COUNTRY_OFF_CHARACTER_STATUS = 0xF88
COUNTRY_OFF_PROGRAM_STATUS = 0xFA8
COUNTRY_OFF_SCIENTIST_ROSTER_ROOT = 0xFF0

PROJECT_PROGRAM_STATUS_A_VTABLE_RVA = 0x295AC88
PROJECT_PROGRAM_STATUS_B_VTABLE_RVA = 0x295ACA8
PROJECT_PROGRAM_STATUS_OFF_FACILITIES = 0x20
SCIENTIST_ROSTER_VECTOR_OFF = 0xA0

FACILITY_VTABLE_RVA = 0x295AC30
FACILITY_OFF_TYPE = 0x08
FACILITY_OFF_ID = 0x0C
FACILITY_OFF_SCI_TAG = 0x58
FACILITY_OFF_SCI_ID = 0x5C
FACILITY_TYPE = 83

CHARACTER_TYPE = 73
CHARACTER_OFF_TYPE = 0x08
CHARACTER_OFF_ID = 0x0C
CHARACTER_OFF_SCIENTIST_PTR = 0xC0

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

MAX_COUNTRIES = 1024
MAX_FACILITIES_PER_COUNTRY = 64
MAX_VECTOR_COUNT = 512
ROOT_SCAN_SPAN = 0x800


@dataclass(frozen=True)
class CountryStatus:
    tag_db_id: int
    country_ptr: int
    status_ptr: int


@dataclass(frozen=True)
class FacilityScientist:
    tag_db_id: int
    facility_id: int
    facility_ptr: int
    scientist_type: int
    scientist_char_id: int


@dataclass(frozen=True)
class ScientistCandidate:
    char_ptr: int
    char_id: int
    mode: str


def heap_ptr(value: int, align: int = 8) -> bool:
    return value >= HEAP_LO and value < HEAP_HI and value % align == 0


def safe_u32(p: Process, addr: int) -> int | None:
    try:
        return p.u32(addr)
    except OSError:
        return None


def safe_u64(p: Process, addr: int) -> int | None:
    try:
        return p.u64(addr)
    except OSError:
        return None


def valid_scientist_character(
    p: Process, char_ptr: int, character_vtable: int, scientist_vtable: int
) -> ScientistCandidate | None:
    if not heap_ptr(char_ptr):
        return None
    if safe_u64(p, char_ptr) != character_vtable:
        return None
    if safe_u32(p, char_ptr + CHARACTER_OFF_TYPE) != CHARACTER_TYPE:
        return None
    char_id = safe_u32(p, char_ptr + CHARACTER_OFF_ID)
    if not char_id:
        return None
    scientist_ptr = safe_u64(p, char_ptr + CHARACTER_OFF_SCIENTIST_PTR)
    if not scientist_ptr or not heap_ptr(scientist_ptr):
        return None
    if safe_u64(p, scientist_ptr) != scientist_vtable:
        return None
    return ScientistCandidate(char_ptr=char_ptr, char_id=char_id, mode="direct")


def candidate_from_slot(
    p: Process, slot_value: int, character_vtable: int, scientist_vtable: int
) -> ScientistCandidate | None:
    direct = valid_scientist_character(p, slot_value, character_vtable, scientist_vtable)
    if direct is not None:
        return direct

    if heap_ptr(slot_value):
        nested = safe_u64(p, slot_value + 8)
        if nested is not None:
            via_field = valid_scientist_character(
                p, nested, character_vtable, scientist_vtable
            )
            if via_field is not None:
                return ScientistCandidate(via_field.char_ptr, via_field.char_id, "element+8")

    plus_8 = valid_scientist_character(
        p, slot_value + 8, character_vtable, scientist_vtable
    )
    if plus_8 is not None:
        return ScientistCandidate(plus_8.char_ptr, plus_8.char_id, "element_addr_plus_8")
    return None


def vector_candidates_from_header(
    p: Process, header_addr: int, character_vtable: int, scientist_vtable: int
) -> list[dict]:
    out: list[dict] = []
    begin = safe_u64(p, header_addr)
    if begin is None or not heap_ptr(begin):
        return out

    for count_offset in (8, 12, 16, 20):
        count = safe_u32(p, header_addr + count_offset)
        if count is None or count <= 0 or count > MAX_VECTOR_COUNT:
            continue
        rows: list[ScientistCandidate] = []
        failed_slots = 0
        for index in range(count):
            slot = safe_u64(p, begin + index * 8)
            if slot is None:
                failed_slots += 1
                break
            candidate = candidate_from_slot(p, slot, character_vtable, scientist_vtable)
            if candidate is None:
                failed_slots += 1
                continue
            rows.append(candidate)
        if not rows:
            continue
        modes = sorted({row.mode for row in rows})
        out.append(
            {
                "header_addr": header_addr,
                "begin": begin,
                "count_offset": count_offset,
                "declared_count": count,
                "valid_rows": len(rows),
                "failed_slots": failed_slots,
                "modes": modes,
                "char_ids": sorted({row.char_id for row in rows}),
                "char_ptrs": sorted({row.char_ptr for row in rows}),
            }
        )
    return out


def country_statuses(p: Process, base: int, gamestate: int) -> tuple[list[CountryStatus], dict]:
    country_vtable = base + COUNTRY_VTABLE_RVA
    status_a_vtable = base + PROJECT_PROGRAM_STATUS_A_VTABLE_RVA
    status_b_vtable = base + PROJECT_PROGRAM_STATUS_B_VTABLE_RVA
    table = safe_u64(p, gamestate + CGAMESTATE_COUNTRY_TABLE)
    count = safe_u32(p, gamestate + CGAMESTATE_COUNTRY_COUNT)
    diag = {
        "country_table": table,
        "country_count": count,
        "country_vtable": country_vtable,
        "status_a_vtable": status_a_vtable,
        "status_b_vtable": status_b_vtable,
        "invalid_country_rows": 0,
        "invalid_status_rows": 0,
    }
    if table is None or count is None or not heap_ptr(table):
        return [], diag
    statuses: list[CountryStatus] = []
    for tag_db_id in range(1, min(count, MAX_COUNTRIES)):
        country_ptr = safe_u64(p, table + tag_db_id * 8)
        if not country_ptr:
            continue
        if not heap_ptr(country_ptr) or safe_u64(p, country_ptr) != country_vtable:
            diag["invalid_country_rows"] += 1
            continue
        status_ptr = safe_u64(p, country_ptr + COUNTRY_OFF_PROGRAM_STATUS)
        if not status_ptr:
            continue
        if (
            not heap_ptr(status_ptr)
            or safe_u64(p, status_ptr) != status_a_vtable
            or safe_u64(p, status_ptr + 8) != status_b_vtable
        ):
            diag["invalid_status_rows"] += 1
            continue
        statuses.append(CountryStatus(tag_db_id, country_ptr, status_ptr))
    return statuses, diag


def facility_scientists(
    p: Process, base: int, statuses: Iterable[CountryStatus]
) -> list[FacilityScientist]:
    facility_vtable = base + FACILITY_VTABLE_RVA
    rows: list[FacilityScientist] = []
    for status in statuses:
        ptrs = safe_u64(p, status.status_ptr + PROJECT_PROGRAM_STATUS_OFF_FACILITIES)
        if not ptrs or not heap_ptr(ptrs):
            continue
        for index in range(MAX_FACILITIES_PER_COUNTRY):
            facility_ptr = safe_u64(p, ptrs + index * 8)
            if not facility_ptr:
                break
            if not heap_ptr(facility_ptr) or safe_u64(p, facility_ptr) != facility_vtable:
                break
            if safe_u32(p, facility_ptr + FACILITY_OFF_TYPE) != FACILITY_TYPE:
                break
            facility_id = safe_u32(p, facility_ptr + FACILITY_OFF_ID)
            scientist_type = safe_u32(p, facility_ptr + FACILITY_OFF_SCI_TAG) or 0
            scientist_char_id = safe_u32(p, facility_ptr + FACILITY_OFF_SCI_ID) or 0
            rows.append(
                FacilityScientist(
                    status.tag_db_id,
                    int(facility_id or 0),
                    facility_ptr,
                    int(scientist_type),
                    int(scientist_char_id),
                )
            )
    return rows


def scan_root_vectors(
    p: Process,
    root_addr: int,
    root_source: str,
    tag_db_id: int,
    status_ptr: int,
    status_offset_base: int,
    character_vtable: int,
    scientist_vtable: int,
    seen: set[tuple[int, int, int]],
) -> list[dict]:
    out: list[dict] = []
    if not heap_ptr(root_addr):
        return out
    for offset in range(0, ROOT_SCAN_SPAN, 4):
        direct_header = root_addr + offset
        for candidate in vector_candidates_from_header(
            p, direct_header, character_vtable, scientist_vtable
        ):
            key = (candidate["header_addr"], candidate["begin"], candidate["count_offset"])
            if key in seen:
                continue
            seen.add(key)
            candidate.update(
                {
                    "source": f"{root_source}_direct",
                    "tag_db_id": tag_db_id,
                    "status_ptr": status_ptr,
                    "status_offset": status_offset_base + offset,
                    "root_addr": root_addr,
                    "root_offset": offset,
                }
            )
            out.append(candidate)

        if offset % 8 != 0:
            continue
        indirect_header = safe_u64(p, root_addr + offset)
        if indirect_header is None or not heap_ptr(indirect_header):
            continue
        for candidate in vector_candidates_from_header(
            p, indirect_header, character_vtable, scientist_vtable
        ):
            key = (candidate["header_addr"], candidate["begin"], candidate["count_offset"])
            if key in seen:
                continue
            seen.add(key)
            candidate.update(
                {
                    "source": f"{root_source}_ptr",
                    "tag_db_id": tag_db_id,
                    "status_ptr": status_ptr,
                    "status_offset": status_offset_base + offset,
                    "root_addr": root_addr,
                    "root_offset": offset,
                }
            )
            out.append(candidate)
    return out


def scan_status_vectors(
    p: Process,
    base: int,
    statuses: Iterable[CountryStatus],
    country_count: int | None,
    ui_ref_value: int | None,
    scan_fallbacks: bool,
) -> list[dict]:
    character_vtable = base + projects_live.CHAR_VT_RVA
    scientist_vtable = base + projects_live.SCI_VT_RVA
    out: list[dict] = []
    seen: set[tuple[int, int, int]] = set()
    scanned_roots: set[int] = set()
    for status in statuses:
        roster_root = safe_u64(p, status.country_ptr + COUNTRY_OFF_SCIENTIST_ROSTER_ROOT)
        if roster_root and heap_ptr(roster_root):
            header = roster_root + SCIENTIST_ROSTER_VECTOR_OFF
            for candidate in vector_candidates_from_header(
                p, header, character_vtable, scientist_vtable
            ):
                key = (candidate["header_addr"], candidate["begin"], candidate["count_offset"])
                if key in seen:
                    continue
                seen.add(key)
                candidate.update(
                    {
                        "source": "ghidra_exact_country_ff0_plus_a0",
                        "tag_db_id": status.tag_db_id,
                        "status_ptr": status.status_ptr,
                        "status_offset": COUNTRY_OFF_SCIENTIST_ROSTER_ROOT,
                        "root_addr": roster_root,
                        "root_offset": SCIENTIST_ROSTER_VECTOR_OFF,
                    }
                )
                out.append(candidate)

        if not scan_fallbacks:
            continue

        if status.status_ptr not in scanned_roots:
            scanned_roots.add(status.status_ptr)
            out.extend(
                scan_root_vectors(
                    p,
                    status.status_ptr,
                    "program_status",
                    status.tag_db_id,
                    status.status_ptr,
                    0,
                    character_vtable,
                    scientist_vtable,
                    seen,
                )
            )

        character_status = safe_u64(p, status.country_ptr + COUNTRY_OFF_CHARACTER_STATUS)
        if character_status and heap_ptr(character_status) and character_status not in scanned_roots:
            scanned_roots.add(character_status)
            out.extend(
                scan_root_vectors(
                    p,
                    character_status,
                    "country_f88",
                    status.tag_db_id,
                    status.status_ptr,
                    COUNTRY_OFF_CHARACTER_STATUS,
                    character_vtable,
                    scientist_vtable,
                    seen,
                )
            )
        if character_status and heap_ptr(character_status):
            table = safe_u64(p, character_status + 8)
            if table and heap_ptr(table):
                max_rows = min(int(country_count or 0), MAX_COUNTRIES)
                table_indexes = {status.tag_db_id}
                if ui_ref_value is not None and 0 <= ui_ref_value < max_rows:
                    table_indexes.add(ui_ref_value)
                for table_index in sorted(i for i in table_indexes if 0 <= i < max_rows):
                    child = safe_u64(p, table + table_index * 8)
                    if not child or not heap_ptr(child):
                        continue
                    if child in scanned_roots:
                        continue
                    scanned_roots.add(child)
                    out.extend(
                        scan_root_vectors(
                            p,
                            child,
                            f"country_f88_table_{table_index}",
                            status.tag_db_id,
                            status.status_ptr,
                            COUNTRY_OFF_CHARACTER_STATUS,
                            character_vtable,
                            scientist_vtable,
                            seen,
                        )
                    )
    return out


def summarize(pid: int, include_cold_scan: bool, scan_fallbacks: bool) -> dict:
    with Process(pid, "hoi4.exe") as p:
        fp = p.fingerprint()
        if not projects_live.apply_build_profile(fp.get("sha256")):
            raise SystemExit(f"unsupported hoi4.exe sha256: {fp.get('sha256')}")
        base = p.module_base
        gamestate = safe_u64(p, base + CG_STATE_DATE_OFFSET)
        if not gamestate or not heap_ptr(gamestate):
            raise SystemExit("CGameState global did not resolve to a heap pointer")

        branch_offset = (
            CGAMESTATE_SCIENTIST_ROSTER_PRIMARY
            if (safe_u32(p, gamestate + CGAMESTATE_SCIENTIST_ROSTER_PRIMARY) or 0) > 0
            else CGAMESTATE_SCIENTIST_ROSTER_FALLBACK
        )
        branch_value = safe_u32(p, gamestate + branch_offset)

        statuses, country_diag = country_statuses(p, base, gamestate)
        facilities = facility_scientists(p, base, statuses)
        assigned_ids = sorted(
            {
                row.scientist_char_id
                for row in facilities
                if row.scientist_type == CHARACTER_TYPE and row.scientist_char_id > 0
            }
        )

        candidates = scan_status_vectors(
            p,
            base,
            statuses,
            country_diag.get("country_count"),
            branch_value,
            scan_fallbacks,
        )
        assigned = set(assigned_ids)
        for candidate in candidates:
            char_ids = set(candidate["char_ids"])
            candidate["assigned_intersection"] = sorted(char_ids & assigned)
            candidate["assigned_missing"] = sorted(assigned - char_ids)
            candidate["assigned_extra"] = sorted(char_ids - assigned)

        candidates.sort(
            key=lambda c: (
                -len(c["assigned_intersection"]),
                c["count_offset"] != 12,
                len(c["assigned_missing"]),
                c["failed_slots"],
                -c["valid_rows"],
                c["source"],
                c["tag_db_id"],
                c["status_offset"],
            )
        )

        exact_vectors = [
            c
            for c in candidates
            if c["source"] == "ghidra_exact_country_ff0_plus_a0"
            and c["count_offset"] == 12
            and c["failed_slots"] == 0
        ]
        exact_by_tag: dict[int, set[int]] = {}
        for candidate in exact_vectors:
            exact_by_tag.setdefault(int(candidate["tag_db_id"]), set()).update(
                int(char_id) for char_id in candidate["char_ids"]
            )
        exact_union = set().union(*exact_by_tag.values()) if exact_by_tag else set()
        facility_misses = []
        facility_match_count = 0
        facility_checked = 0
        for row in facilities:
            if row.scientist_type != CHARACTER_TYPE or row.scientist_char_id <= 0:
                continue
            facility_checked += 1
            country_ids = exact_by_tag.get(row.tag_db_id, set())
            if row.scientist_char_id in country_ids:
                facility_match_count += 1
            else:
                facility_misses.append(
                    {
                        "tag_db_id": row.tag_db_id,
                        "facility_id": row.facility_id,
                        "facility_ptr": row.facility_ptr,
                        "scientist_char_id": row.scientist_char_id,
                        "country_vector_char_ids": sorted(country_ids),
                    }
                )

        exact_aggregate = {
            "valid_country_vectors": len(exact_vectors),
            "char_ids_union_count": len(exact_union),
            "assigned_intersection": sorted(exact_union & assigned),
            "assigned_missing": sorted(assigned - exact_union),
            "assigned_extra": sorted(exact_union - assigned),
            "facility_assignments_checked": facility_checked,
            "facility_assignment_matches": facility_match_count,
            "facility_assignment_misses": facility_misses,
            "ui_ref_vector": next(
                (
                    c
                    for c in exact_vectors
                    if branch_value is not None and c["tag_db_id"] == int(branch_value)
                ),
                None,
            ),
        }

        cold = None
        if include_cold_scan:
            cold_addrs = projects_live.enumerate_scientist_addrs(p)
            cold = {
                "count": len(cold_addrs),
                "char_ids": sorted({fp[0] for _addr, fp in cold_addrs}),
            }

        return {
            "pid": pid,
            "module": fp,
            "base": base,
            "gamestate_global": base + CG_STATE_DATE_OFFSET,
            "gamestate": gamestate,
            "ui_roster_ref_offset": branch_offset,
            "ui_roster_ref_value": branch_value,
            "country_diag": country_diag,
            "program_status_count": len(statuses),
            "facility_rows": len(facilities),
            "assigned_scientist_char_ids": assigned_ids,
            "candidate_count": len(candidates),
            "top_candidates": candidates[:10],
            "exact_aggregate": exact_aggregate,
            "cold_scientist_scan": cold,
            "scan_fallbacks": scan_fallbacks,
        }


def compact_print(result: dict) -> None:
    print(f"pid={result['pid']}")
    print(f"sha256={result['module'].get('sha256')}")
    print(f"base=0x{result['base']:x}")
    print(f"gamestate=0x{result['gamestate']:x}")
    print(
        "ui_roster_ref="
        f"+0x{result['ui_roster_ref_offset']:x}:{result['ui_roster_ref_value']}"
    )
    print(f"program_status_count={result['program_status_count']}")
    print(f"facility_rows={result['facility_rows']}")
    print(f"assigned_scientist_char_ids={result['assigned_scientist_char_ids']}")
    aggregate = result["exact_aggregate"]
    print(f"exact_valid_country_vectors={aggregate['valid_country_vectors']}")
    print(f"exact_char_ids_union_count={aggregate['char_ids_union_count']}")
    print(f"exact_assigned_intersection={aggregate['assigned_intersection']}")
    print(f"exact_assigned_missing={aggregate['assigned_missing']}")
    print(
        "facility_assignment_matches="
        f"{aggregate['facility_assignment_matches']}/"
        f"{aggregate['facility_assignments_checked']}"
    )
    if aggregate["facility_assignment_misses"]:
        print(f"facility_assignment_misses={aggregate['facility_assignment_misses']}")
    ui_vector = aggregate.get("ui_ref_vector")
    if ui_vector is not None:
        print(
            "ui_ref_vector "
            "tag_db_id={tag} header=0x{header:x} begin=0x{begin:x} "
            "declared={declared} valid={valid} modes={modes} char_ids={ids}".format(
                tag=ui_vector["tag_db_id"],
                header=ui_vector["header_addr"],
                begin=ui_vector["begin"],
                declared=ui_vector["declared_count"],
                valid=ui_vector["valid_rows"],
                modes=",".join(ui_vector["modes"]),
                ids=ui_vector["char_ids"],
            )
        )
    print(f"candidate_count={result['candidate_count']}")
    print(f"scan_fallbacks={result['scan_fallbacks']}")
    for idx, candidate in enumerate(result["top_candidates"], 1):
        print(
            "candidate[{idx}] source={source} tag_db_id={tag} "
            "status_ptr=0x{status:x} status_offset=0x{off:x} "
            "header=0x{header:x} begin=0x{begin:x} count_off=0x{coff:x} "
            "declared={declared} valid={valid} failed={failed} modes={modes} "
            "char_ids={ids} assigned_intersection={hit} missing={missing}".format(
                idx=idx,
                source=candidate["source"],
                tag=candidate["tag_db_id"],
                status=candidate["status_ptr"],
                off=candidate["status_offset"],
                header=candidate["header_addr"],
                begin=candidate["begin"],
                coff=candidate["count_offset"],
                declared=candidate["declared_count"],
                valid=candidate["valid_rows"],
                failed=candidate["failed_slots"],
                modes=",".join(candidate["modes"]),
                ids=candidate["char_ids"],
                hit=candidate["assigned_intersection"],
                missing=candidate["assigned_missing"],
            )
        )
    cold = result.get("cold_scientist_scan")
    if cold is not None:
        print(f"cold_scientist_count={cold['count']}")
        print(f"cold_scientist_char_ids={cold['char_ids']}")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int, default=None)
    ap.add_argument("--cold-scan", action="store_true")
    ap.add_argument("--scan-fallbacks", action="store_true")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    pid = args.pid or find_pid("hoi4.exe")
    if pid is None:
        raise SystemExit("hoi4.exe is not running")
    result = summarize(
        pid, include_cold_scan=args.cold_scan, scan_fallbacks=args.scan_fallbacks
    )
    if args.json:
        print(json.dumps(result, indent=2, sort_keys=True))
    else:
        compact_print(result)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
