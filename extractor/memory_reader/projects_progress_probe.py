"""Bounded live probe for special-project progress/state/breakthrough roots.

This is intentionally a read-only probe. It follows the current DLL production
roots from ``CGameState`` to ``CCountry`` to ``CProgramStatus`` and samples only
anchored project objects. It validates or rejects candidate paths before the DLL
schema is extended.
"""

import argparse
import json
import os
import struct
import sys
import time
from collections import Counter
from datetime import datetime, timezone

_THIS = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _THIS)

import winmem  # noqa: E402

EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

CG_STATE_DATE_OFFSET = 0x3316E90
COUNTRY_DATABASE_OFFSET = 0x33189C8
CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C
COUNTRY_VTABLE_RVA = 0x27D0E90
COUNTRY_OFF_PROGRAM_STATUS = 0xFA8
OBJ_TAG_DB_ID = 0x08

PROJECT_PROGRAM_STATUS_A_VTABLE_RVA = 0x295AC88
PROJECT_PROGRAM_STATUS_B_VTABLE_RVA = 0x295ACA8
PROJECT_PROGRAM_STATUS_OFF_POOL = 0x18
PROJECT_PROGRAM_STATUS_OFF_PROGRAMS = 0x20
PROJECT_POOL_VTABLE_RVA = 0x29B0238
PROJECT_POOL_OFF_FIRST_PROJECT = 0x10
PROJECT_POOL_OFF_COUNT = 0x1C
PROJECT_COUNTRY_STRIDE = 0x1C0

PROJECT_FACILITY_VTABLE_RVA = 0x295AC30
PROJECT_CPROJECT_VTABLE_RVA = 0x296C270
PROJECT_PROJECT_TYPE = 86
PROJECT_PROJECT_OFF_TYPE = 0x08
PROJECT_PROJECT_OFF_ID = 0x0C
PROJECT_PROJECT_OFF_DEF_HANDLE = 0x18
PROJECT_PROJECT_OFF_STATE_MACHINE_PTR = 0x28
PROJECT_PROJECT_OFF_OWNER_TAG = 0x60
PROJECT_PROJECT_OFF_OWNER_FACILITY = 0x64
PROJECT_PROJECT_OFF_ITERATIONS = 0x74
PROJECT_PROJECT_OFF_STATE_MACHINE = 0x170

PROJECT_STATE_MACHINE_VTABLE_RVA = 0x2A19358
PROJECT_PROTOTYPE_STATE_VTABLE_RVA = 0x2A19300
PROJECT_SIMPLE_STATE_VTABLE_RVA = 0x2A192A8
PROJECT_BREAKTHROUGH_PROGRESS_VTABLE_RVA = 0x2A152E0
PROJECT_STATE_MACHINE_OFF_CURRENT_STATE_PTR = 0x08
PROJECT_STATE_MACHINE_OFF_PROTOTYPE_STATE = 0x18
PROJECT_STATE_MACHINE_OFF_SIMPLE_STATES = (0xE8, 0x168, 0x1E8)
PROJECT_STATE_OFF_CURRENT_STATE_ENUM = 0x78
PROJECT_PROTOTYPE_STATE_OFF_PHASE_PROGRESS_RAW = 0x48
PROJECT_PROTOTYPE_STATE_OFF_PHASE_TARGET = 0x50
PROJECT_PROTOTYPE_STATE_OFF_ITERATIONS = 0x8C

MAX_TAG_ROWS = 1024
MAX_COUNTRY_PROJECTS = 256
MAX_SCAN_WINDOW = 0xC0


def hx(value):
    return "0x%X" % value if isinstance(value, int) and value else None


def is_heap_ptr(value, align=8):
    return HEAP_LO <= value < HEAP_HI and value % align == 0


def try_u32(p, addr):
    try:
        return p.u32(addr)
    except OSError:
        return None


def try_u8(p, addr):
    try:
        return p.u8(addr)
    except OSError:
        return None


def try_u64(p, addr):
    try:
        return p.u64(addr)
    except OSError:
        return None


def read_small_inline_string(p, addr):
    data = p.try_read(addr, 16)
    if len(data) < 16:
        return None
    end = data.find(b"\0")
    if end == -1:
        end = 16
    try:
        return data[:end].decode("latin-1")
    except UnicodeDecodeError:
        return None


def tag_row_count(p, country_database_ptr):
    tag_table = try_u64(p, country_database_ptr)
    tag_count = try_u32(p, country_database_ptr + 8)
    active_count = try_u32(p, country_database_ptr + 12)
    if not tag_table or not tag_count or not active_count:
        return None
    if tag_count > MAX_TAG_ROWS or active_count > tag_count:
        return None
    for index, want in enumerate(["---", "GER", "ENG", "SOV"]):
        if read_small_inline_string(p, tag_table + index * 32) != want:
            return None
    return min(tag_count, active_count, MAX_TAG_ROWS)


def vtable_name(vtable, expected):
    for name, addr in expected.items():
        if vtable == addr:
            return name
    return None


def scan_direct_and_pointer_vtables(p, base_addr, window, expected):
    direct = Counter()
    pointer = Counter()
    examples = []
    for off in range(0, window, 8):
        value = try_u64(p, base_addr + off)
        if value is None:
            continue
        name = vtable_name(value, expected)
        if name:
            direct[(off, name)] += 1
            if len(examples) < 16:
                examples.append(
                    {
                        "kind": "direct",
                        "offset": hx(off),
                        "vtable": name,
                        "addr": hx(base_addr + off),
                    }
                )
            continue
        if is_heap_ptr(value):
            pointee_vtable = try_u64(p, value)
            name = vtable_name(pointee_vtable, expected)
            if name:
                pointer[(off, name)] += 1
                if len(examples) < 16:
                    examples.append(
                        {
                            "kind": "pointer",
                            "offset": hx(off),
                            "vtable": name,
                            "addr": hx(value),
                        }
                    )
    return direct, pointer, examples


def read_breakthrough_rows(p, obj_addr, breakthrough_vtable, max_rows=16):
    if try_u64(p, obj_addr) != breakthrough_vtable:
        return None
    head = try_u64(p, obj_addr + 8)
    size = try_u64(p, obj_addr + 0x10)
    if not is_heap_ptr(head) or size is None or size > max_rows:
        return None
    root = try_u64(p, head + 8)
    if not root or not is_heap_ptr(root):
        return [] if size == 0 else None
    rows = []
    seen = set()

    def walk(node, depth=0):
        if len(rows) > max_rows or depth > 32:
            return False
        if not is_heap_ptr(node) or node in seen:
            return False
        seen.add(node)
        is_nil = try_u8(p, node + 0x19)
        if is_nil is None:
            return False
        if is_nil != 0:
            return True
        left = try_u64(p, node)
        right = try_u64(p, node + 0x10)
        if left and left != head and not walk(left, depth + 1):
            return False
        key = try_u32(p, node + 0x1C)
        value = try_u32(p, node + 0x20)
        if key is None or value is None:
            return False
        rows.append({"specialization_handle": key, "points_raw": value})
        if right and right != head and not walk(right, depth + 1):
            return False
        return True

    if not walk(root):
        return None
    if len(rows) != size:
        return None
    rows.sort(key=lambda row: row["specialization_handle"])
    return rows


def classify_current_state(p, state_machine_addr, expected):
    current_ptr = try_u64(
        p, state_machine_addr + PROJECT_STATE_MACHINE_OFF_CURRENT_STATE_PTR
    )
    if not is_heap_ptr(current_ptr):
        return {
            "valid": False,
            "current_ptr": current_ptr,
            "offset": None,
            "vtable": None,
            "enum": None,
        }
    offset = current_ptr - state_machine_addr
    current_vtable = try_u64(p, current_ptr)
    name = vtable_name(current_vtable, expected)
    expected_offsets = {
        PROJECT_STATE_MACHINE_OFF_PROTOTYPE_STATE: "CPrototypeProjectState",
        **{
            off: "CSimpleProjectState"
            for off in PROJECT_STATE_MACHINE_OFF_SIMPLE_STATES
        },
    }
    expected_name = expected_offsets.get(offset)
    enum_value = try_u32(p, current_ptr + PROJECT_STATE_OFF_CURRENT_STATE_ENUM)
    return {
        "valid": expected_name is not None and name == expected_name,
        "current_ptr": current_ptr,
        "offset": offset,
        "vtable": name,
        "enum": enum_value,
    }


def read_prototype_progress_snapshot(p, state_machine_addr, expected):
    prototype_addr = state_machine_addr + PROJECT_STATE_MACHINE_OFF_PROTOTYPE_STATE
    if try_u64(p, prototype_addr) != expected["CPrototypeProjectState"]:
        return None
    raw = try_u32(p, prototype_addr + PROJECT_PROTOTYPE_STATE_OFF_PHASE_PROGRESS_RAW)
    target = try_u32(p, prototype_addr + PROJECT_PROTOTYPE_STATE_OFF_PHASE_TARGET)
    iterations = try_u32(p, prototype_addr + PROJECT_PROTOTYPE_STATE_OFF_ITERATIONS)
    enum_value = try_u32(p, prototype_addr + PROJECT_STATE_OFF_CURRENT_STATE_ENUM)
    phase_progress_percent_candidate = None
    if raw is not None and target:
        phase_progress_percent_candidate = round(raw / target / 1000.0, 6)
    return {
        "addr": prototype_addr,
        "phase_progress_raw": raw,
        "phase_target": target,
        "phase_progress_percent_candidate": phase_progress_percent_candidate,
        "iterations": iterations,
        "enum": enum_value,
    }


def summarize_counter(counter):
    return [
        {"offset": hx(off), "vtable": name, "count": count}
        for (off, name), count in sorted(counter.items())
    ]


def sample_project(p, addr, project_vtable):
    if not is_heap_ptr(addr):
        return None
    if try_u64(p, addr) != project_vtable:
        return None
    if try_u32(p, addr + PROJECT_PROJECT_OFF_TYPE) != PROJECT_PROJECT_TYPE:
        return None
    project_id = try_u32(p, addr + PROJECT_PROJECT_OFF_ID)
    static_def_handle = try_u32(p, addr + PROJECT_PROJECT_OFF_DEF_HANDLE)
    if not project_id or not static_def_handle:
        return None
    owner_tag = try_u32(p, addr + PROJECT_PROJECT_OFF_OWNER_TAG) or 0
    owner_facility = try_u32(p, addr + PROJECT_PROJECT_OFF_OWNER_FACILITY) or 0
    worked = owner_tag == 83 and owner_facility != 0
    state_machine_ptr = try_u64(p, addr + PROJECT_PROJECT_OFF_STATE_MACHINE_PTR)
    return {
        "addr": addr,
        "project_id": project_id,
        "static_def_handle": static_def_handle,
        "iterations": try_u32(p, addr + PROJECT_PROJECT_OFF_ITERATIONS),
        "worked_at_facility": owner_facility if worked else None,
        "state_machine_ptr": state_machine_ptr if state_machine_ptr else None,
    }


def sample_projects_from_pool(p, pool_ptr, project_vtable):
    if not is_heap_ptr(pool_ptr):
        return [], "invalid_pool_pointer"
    first_project = try_u64(p, pool_ptr + PROJECT_POOL_OFF_FIRST_PROJECT)
    pool_count = try_u32(p, pool_ptr + PROJECT_POOL_OFF_COUNT)
    if not first_project or not is_heap_ptr(first_project):
        return [], "empty_pool"
    if pool_count is None or pool_count == 0:
        return [], "empty_count"
    if pool_count > MAX_COUNTRY_PROJECTS:
        return [], "count_overflow"
    rows = []
    for index in range(pool_count):
        addr = first_project + index * PROJECT_COUNTRY_STRIDE
        row = sample_project(p, addr, project_vtable)
        if row is None:
            return rows, "partial_project_validation"
        rows.append(row)
    return rows, "valid"


def probe(args):
    with winmem.Process(args.pid, "hoi4.exe") as p:
        fp = p.fingerprint()
        sha = str(fp.get("sha256") or "").lower()
        if sha != EXPECTED_SHA256 and not args.force:
            raise SystemExit(
                "live hoi4.exe sha256 %s != expected %s; pass --force to probe anyway"
                % (sha, EXPECTED_SHA256)
            )

        module_base = p.module_base
        expected = {
            "CProjectStateMachine": module_base + PROJECT_STATE_MACHINE_VTABLE_RVA,
            "CPrototypeProjectState": module_base + PROJECT_PROTOTYPE_STATE_VTABLE_RVA,
            "CSimpleProjectState": module_base + PROJECT_SIMPLE_STATE_VTABLE_RVA,
            "CBreakthroughProgress": module_base + PROJECT_BREAKTHROUGH_PROGRESS_VTABLE_RVA,
        }
        country_vtable = module_base + COUNTRY_VTABLE_RVA
        status_a_vtable = module_base + PROJECT_PROGRAM_STATUS_A_VTABLE_RVA
        status_b_vtable = module_base + PROJECT_PROGRAM_STATUS_B_VTABLE_RVA
        pool_vtable = module_base + PROJECT_POOL_VTABLE_RVA
        project_vtable = module_base + PROJECT_CPROJECT_VTABLE_RVA

        game_state = try_u64(p, module_base + CG_STATE_DATE_OFFSET)
        country_database = try_u64(p, module_base + COUNTRY_DATABASE_OFFSET)
        if not is_heap_ptr(game_state) or not is_heap_ptr(country_database):
            raise SystemExit("failed to resolve CGameState or country database root")
        country_table = try_u64(p, game_state + CGAMESTATE_COUNTRY_TABLE)
        gamestate_country_count = try_u32(p, game_state + CGAMESTATE_COUNTRY_COUNT)
        rows = tag_row_count(p, country_database)
        if not rows or not is_heap_ptr(country_table):
            raise SystemExit("failed to validate country table/tag table")

        summary = {
            "schema": "project_progress_live_probe_v2",
            "pid": args.pid,
            "utc": datetime.now(timezone.utc).isoformat(),
            "module_path": fp.get("path"),
            "module_base": hx(module_base),
            "sha256": sha,
            "roots": {
                "game_state": hx(game_state),
                "country_database": hx(country_database),
                "country_table": hx(country_table),
                "tag_row_count": rows,
                "gamestate_country_count": gamestate_country_count,
            },
            "expected_vtables": {key: hx(value) for key, value in expected.items()},
            "ghidra_evidence": [
                {
                    "file": "symbol_14296c270__entry_2__140fd0950__FUN_140fd0950.c",
                    "function": "FUN_140fd0950",
                    "evidence": "CProject serializer emits token 0x29c9 from param_1 + 0x170",
                },
                {
                    "file": "symbol_14296c270__entry_0__140fcb610__FUN_140fcb610.c",
                    "function": "FUN_140fcb610",
                    "evidence": "CProject destructor tears down param_1 + 0x170 before deleting 0x1C0 object",
                },
                {
                    "file": "symbol_14295ac88__entry_6__140e63f30__FUN_140e63f30.c",
                    "function": "FUN_140e63f30",
                    "evidence": "CProgramStatus serializer emits token 0x2eb4 from param_1 + 0x48",
                },
                {
                    "file": "symbol_14295ac88__entry_1__140e61210__FUN_140e61210.c",
                    "function": "FUN_140e61210",
                    "evidence": "CProgramStatus destructor shows +0x20/+0x38 vectors and +0x58 tree; +0x48 is not accepted without live vtable proof",
                },
            ],
        }

        valid_countries = 0
        duplicate_country_objects = 0
        seen_country_objects = set()
        status_valid = 0
        status_absent = 0
        status_invalid = 0
        pool_valid = 0
        pool_invalid = 0
        project_pool_status = Counter()
        countries_with_worked_projects = 0
        projects = []
        status_direct_hits = Counter()
        status_pointer_hits = Counter()
        status_examples = []
        project_direct_hits = Counter()
        project_pointer_hits = Counter()
        project_0x170_direct_hits = Counter()
        project_0x170_pointer_hits = Counter()
        state_machine_direct_hits = Counter()
        state_machine_pointer_hits = Counter()
        state_machine_ptr_valid = 0
        state_machine_ptr_invalid = 0
        current_state_selector_hits = Counter()
        current_state_selector_worked_hits = Counter()
        current_state_selector_invalid = 0
        current_state_enum_hits = Counter()
        simple_state_enum_hits = Counter()
        prototype_state_enum_hits = Counter()
        prototype_phase_target_hits = Counter()
        prototype_phase_nonzero = 0
        prototype_iterations_match_project = 0
        prototype_iterations_mismatch_project = 0
        breakthrough_countries = 0
        breakthrough_rows_total = 0
        breakthrough_parse_failures = 0
        breakthrough_key_counts = Counter()
        breakthrough_examples = []
        project_examples = []
        project_0x170_examples = []
        state_machine_examples = []
        current_state_examples = []
        progress_examples = []

        for tag_db_id in range(1, rows):
            country_ptr = try_u64(p, country_table + tag_db_id * 8)
            if not country_ptr:
                continue
            if not is_heap_ptr(country_ptr) or try_u64(p, country_ptr) != country_vtable:
                continue
            if try_u32(p, country_ptr + OBJ_TAG_DB_ID) != tag_db_id:
                continue
            valid_countries += 1
            if country_ptr in seen_country_objects:
                duplicate_country_objects += 1
                continue
            seen_country_objects.add(country_ptr)

            status_ptr = try_u64(p, country_ptr + COUNTRY_OFF_PROGRAM_STATUS)
            if not status_ptr:
                status_absent += 1
                continue
            if (
                not is_heap_ptr(status_ptr)
                or try_u64(p, status_ptr) != status_a_vtable
                or try_u64(p, status_ptr + 8) != status_b_vtable
            ):
                status_invalid += 1
                continue
            status_valid += 1

            direct, pointer, examples = scan_direct_and_pointer_vtables(
                p, status_ptr, args.status_scan_window, expected
            )
            status_direct_hits.update(direct)
            status_pointer_hits.update(pointer)
            if len(status_examples) < 16:
                status_examples.extend(examples[: 16 - len(status_examples)])
            breakthrough_rows = read_breakthrough_rows(
                p,
                status_ptr + 0x50,
                module_base + PROJECT_BREAKTHROUGH_PROGRESS_VTABLE_RVA,
                max_rows=16,
            )
            if breakthrough_rows is None:
                breakthrough_parse_failures += 1
            else:
                if breakthrough_rows:
                    breakthrough_countries += 1
                    breakthrough_rows_total += len(breakthrough_rows)
                    for row in breakthrough_rows:
                        breakthrough_key_counts[row["specialization_handle"]] += 1
                    if len(breakthrough_examples) < 16:
                        breakthrough_examples.append(
                            {
                                "tag_db_id": tag_db_id,
                                "status_ptr": hx(status_ptr),
                                "rows": breakthrough_rows,
                            }
                        )

            pool_ptr = try_u64(p, status_ptr + PROJECT_PROGRAM_STATUS_OFF_POOL)
            if not pool_ptr or not is_heap_ptr(pool_ptr) or try_u64(p, pool_ptr) != pool_vtable:
                pool_invalid += 1
                continue
            pool_valid += 1
            project_rows, pool_status = sample_projects_from_pool(p, pool_ptr, project_vtable)
            project_pool_status[pool_status] += 1
            if pool_status != "valid":
                continue
            worked_in_country = 0
            for row in project_rows:
                if len(projects) >= args.max_projects:
                    break
                projects.append(row)
                if row["worked_at_facility"] is not None:
                    worked_in_country += 1
                direct, pointer, examples = scan_direct_and_pointer_vtables(
                    p, row["addr"], PROJECT_COUNTRY_STRIDE, expected
                )
                project_direct_hits.update(direct)
                project_pointer_hits.update(pointer)
                if len(project_examples) < 16:
                    project_examples.extend(examples[: 16 - len(project_examples)])

                state_machine_ptr = row.get("state_machine_ptr")
                if (
                    is_heap_ptr(state_machine_ptr)
                    and try_u64(p, state_machine_ptr)
                    == module_base + PROJECT_STATE_MACHINE_VTABLE_RVA
                ):
                    state_machine_ptr_valid += 1
                    current_state = classify_current_state(
                        p, state_machine_ptr, expected
                    )
                    if current_state["valid"]:
                        key = (
                            current_state["offset"],
                            current_state["vtable"],
                            current_state["enum"],
                        )
                        current_state_selector_hits[key] += 1
                        current_state_enum_hits[current_state["enum"]] += 1
                        if row["worked_at_facility"] is not None:
                            current_state_selector_worked_hits[key] += 1
                    else:
                        current_state_selector_invalid += 1

                    prototype_progress = read_prototype_progress_snapshot(
                        p, state_machine_ptr, expected
                    )
                    if prototype_progress is not None:
                        prototype_state_enum_hits[prototype_progress["enum"]] += 1
                        if prototype_progress["phase_target"] is not None:
                            prototype_phase_target_hits[
                                prototype_progress["phase_target"]
                            ] += 1
                        if prototype_progress["phase_progress_raw"]:
                            prototype_phase_nonzero += 1
                        if prototype_progress["iterations"] == row["iterations"]:
                            prototype_iterations_match_project += 1
                        else:
                            prototype_iterations_mismatch_project += 1
                        if (
                            row["worked_at_facility"] is not None
                            and len(progress_examples) < 16
                        ):
                            progress_examples.append(
                                {
                                    "project_id": row["project_id"],
                                    "static_def_handle": row["static_def_handle"],
                                    "worked_at_facility": row["worked_at_facility"],
                                    "project_iterations": row["iterations"],
                                    "prototype_state_addr": hx(
                                        prototype_progress["addr"]
                                    ),
                                    "phase_progress_raw": prototype_progress[
                                        "phase_progress_raw"
                                    ],
                                    "phase_target": prototype_progress[
                                        "phase_target"
                                    ],
                                    "phase_progress_percent_candidate": (
                                        prototype_progress[
                                            "phase_progress_percent_candidate"
                                        ]
                                    ),
                                    "prototype_iterations": prototype_progress[
                                        "iterations"
                                    ],
                                    "current_state_offset": hx(
                                        current_state["offset"]
                                    ),
                                    "current_state_enum": current_state["enum"],
                                    "current_state_vtable": current_state["vtable"],
                                }
                            )
                    for simple_off in PROJECT_STATE_MACHINE_OFF_SIMPLE_STATES:
                        simple_addr = state_machine_ptr + simple_off
                        if (
                            try_u64(p, simple_addr)
                            == expected["CSimpleProjectState"]
                        ):
                            simple_state_enum_hits[
                                (simple_off, try_u32(
                                    p,
                                    simple_addr
                                    + PROJECT_STATE_OFF_CURRENT_STATE_ENUM,
                                ))
                            ] += 1
                    if len(current_state_examples) < 16:
                        current_state_examples.append(
                            {
                                "project_id": row["project_id"],
                                "static_def_handle": row["static_def_handle"],
                                "worked_at_facility": row["worked_at_facility"],
                                "state_machine_ptr": hx(state_machine_ptr),
                                "current_state_ptr": hx(
                                    current_state["current_ptr"]
                                ),
                                "current_state_offset": hx(
                                    current_state["offset"]
                                ),
                                "current_state_vtable": current_state["vtable"],
                                "current_state_enum": current_state["enum"],
                                "valid": current_state["valid"],
                            }
                        )
                    direct, pointer, examples = scan_direct_and_pointer_vtables(
                        p, state_machine_ptr, args.state_scan_window, expected
                    )
                    state_machine_direct_hits.update(direct)
                    state_machine_pointer_hits.update(pointer)
                    if len(state_machine_examples) < 16:
                        state_machine_examples.extend(
                            examples[: 16 - len(state_machine_examples)]
                        )
                else:
                    state_machine_ptr_invalid += 1

                state_addr = row["addr"] + PROJECT_PROJECT_OFF_STATE_MACHINE
                direct, pointer, examples = scan_direct_and_pointer_vtables(
                    p, state_addr, 0x50, expected
                )
                project_0x170_direct_hits.update(direct)
                project_0x170_pointer_hits.update(pointer)
                if len(project_0x170_examples) < 16:
                    project_0x170_examples.extend(
                        examples[: 16 - len(project_0x170_examples)]
                    )
            if worked_in_country:
                countries_with_worked_projects += 1

        worked_projects = [row for row in projects if row["worked_at_facility"] is not None]
        project_0x170_direct_count = project_0x170_direct_hits.get(
            (0, "CProjectStateMachine"), 0
        )
        project_0x28_pointer_count = project_pointer_hits.get((0x28, "CProjectStateMachine"), 0)
        current_state_selector_total = sum(current_state_selector_hits.values())
        status_breakthrough_direct = [
            item
            for item in summarize_counter(status_direct_hits)
            if item["vtable"] == "CBreakthroughProgress"
        ]
        status_breakthrough_pointer = [
            item
            for item in summarize_counter(status_pointer_hits)
            if item["vtable"] == "CBreakthroughProgress"
        ]

        summary.update(
            {
                "counts": {
                    "valid_country_rows": valid_countries,
                    "duplicate_country_objects": duplicate_country_objects,
                    "program_status_valid": status_valid,
                    "program_status_absent": status_absent,
                    "program_status_invalid": status_invalid,
                    "project_pool_valid": pool_valid,
                    "project_pool_invalid": pool_invalid,
                    "sampled_projects": len(projects),
                    "sampled_worked_projects": len(worked_projects),
                    "countries_with_worked_projects_in_sample": countries_with_worked_projects,
                    "state_machine_ptr_valid": state_machine_ptr_valid,
                    "state_machine_ptr_invalid": state_machine_ptr_invalid,
                    "current_state_selector_valid": current_state_selector_total,
                    "current_state_selector_invalid": current_state_selector_invalid,
                    "prototype_phase_nonzero": prototype_phase_nonzero,
                    "prototype_iterations_match_project": (
                        prototype_iterations_match_project
                    ),
                    "prototype_iterations_mismatch_project": (
                        prototype_iterations_mismatch_project
                    ),
                    "breakthrough_countries_with_points": breakthrough_countries,
                    "breakthrough_rows_total": breakthrough_rows_total,
                    "breakthrough_parse_failures": breakthrough_parse_failures,
                },
                "project_pool_status": dict(project_pool_status),
                "validated": {
                    "CProject_state_machine_direct_at_0x170": (
                        project_0x170_direct_count == len(projects) and len(projects) > 0
                    ),
                    "CProject_state_machine_pointer_at_0x28": (
                        project_0x28_pointer_count == len(projects) and len(projects) > 0
                    ),
                    "CProjectStateMachine_current_state_selector_at_0x8": (
                        current_state_selector_total == len(projects)
                        and current_state_selector_invalid == 0
                        and len(projects) > 0
                    ),
                    "CProjectStateMachine_embedded_state_enums": (
                        prototype_state_enum_hits.get(0, 0) == len(projects)
                        and all(
                            simple_state_enum_hits.get((off, index), 0)
                            == len(projects)
                            for index, off in enumerate(
                                PROJECT_STATE_MACHINE_OFF_SIMPLE_STATES,
                                start=1,
                            )
                        )
                        and len(projects) > 0
                    ),
                    "CPrototypeProjectState_progress_raw_fields_candidate": (
                        prototype_phase_nonzero > 0
                        and bool(prototype_phase_target_hits)
                    ),
                    "CProgramStatus_breakthrough_direct_or_pointer": bool(
                        status_breakthrough_direct or status_breakthrough_pointer
                    ),
                    "CProgramStatus_plus_0x50_breakthrough_map_decoded": (
                        breakthrough_parse_failures == 0 and breakthrough_rows_total > 0
                    ),
                },
                "candidate_results": {
                    "CProject_plus_0x170_direct": {
                        "expected": "CProjectStateMachine",
                        "hits": project_0x170_direct_count,
                        "sampled_projects": len(projects),
                    },
                    "CProject_plus_0x28_pointer": {
                        "expected": "CProjectStateMachine",
                        "hits": project_0x28_pointer_count,
                        "sampled_projects": len(projects),
                    },
                    "CProjectStateMachine_plus_0x8_current_state_selector": {
                        "expected": (
                            "pointer to embedded CPrototypeProjectState at +0x18 "
                            "or embedded CSimpleProjectState at +0xE8/+0x168/+0x1E8"
                        ),
                        "hits": [
                            {
                                "offset": hx(off),
                                "vtable": name,
                                "current_state_enum": enum_value,
                                "count": count,
                            }
                            for (off, name, enum_value), count in sorted(
                                current_state_selector_hits.items()
                            )
                        ],
                        "worked_hits": [
                            {
                                "offset": hx(off),
                                "vtable": name,
                                "current_state_enum": enum_value,
                                "count": count,
                            }
                            for (off, name, enum_value), count in sorted(
                                current_state_selector_worked_hits.items()
                            )
                        ],
                        "invalid": current_state_selector_invalid,
                        "sampled_projects": len(projects),
                    },
                    "CProjectStateMachine_embedded_state_enum_offsets": {
                        "prototype": {
                            "offset": hx(PROJECT_STATE_MACHINE_OFF_PROTOTYPE_STATE),
                            "enum_offset": hx(PROJECT_STATE_OFF_CURRENT_STATE_ENUM),
                            "enum_counts": dict(
                                sorted(prototype_state_enum_hits.items())
                            ),
                        },
                        "simple": [
                            {
                                "offset": hx(off),
                                "enum_offset": hx(
                                    PROJECT_STATE_OFF_CURRENT_STATE_ENUM
                                ),
                                "enum_value": enum_value,
                                "count": count,
                            }
                            for (off, enum_value), count in sorted(
                                simple_state_enum_hits.items()
                            )
                        ],
                    },
                    "CPrototypeProjectState_progress_candidate_fields": {
                        "path": "CProject+0x28 -> CProjectStateMachine+0x18",
                        "phase_progress_raw_offset": hx(
                            PROJECT_PROTOTYPE_STATE_OFF_PHASE_PROGRESS_RAW
                        ),
                        "phase_target_offset": hx(
                            PROJECT_PROTOTYPE_STATE_OFF_PHASE_TARGET
                        ),
                        "iterations_offset": hx(
                            PROJECT_PROTOTYPE_STATE_OFF_ITERATIONS
                        ),
                        "phase_target_counts": dict(
                            sorted(prototype_phase_target_hits.items())
                        ),
                        "nonzero_phase_raw_projects": prototype_phase_nonzero,
                        "iterations_match_CProject_plus_0x74": (
                            prototype_iterations_match_project
                        ),
                        "iterations_mismatch_CProject_plus_0x74": (
                            prototype_iterations_mismatch_project
                        ),
                        "semantic_note": (
                            "raw accumulator fields, not direct saved "
                            "project_progress/phase_progress percent scalars"
                        ),
                    },
                    "CProgramStatus_breakthrough_hits_direct": status_breakthrough_direct,
                    "CProgramStatus_breakthrough_hits_pointer": status_breakthrough_pointer,
                    "CProgramStatus_plus_0x50_breakthrough_rows": {
                        "countries_with_points": breakthrough_countries,
                        "rows_total": breakthrough_rows_total,
                        "parse_failures": breakthrough_parse_failures,
                        "specialization_handle_counts": dict(
                            sorted(breakthrough_key_counts.items())
                        ),
                    },
                },
                "offset_scan_results": {
                    "status_direct": summarize_counter(status_direct_hits),
                    "status_pointer": summarize_counter(status_pointer_hits),
                    "project_direct": summarize_counter(project_direct_hits),
                    "project_pointer": summarize_counter(project_pointer_hits),
                    "project_plus_0x170_direct": summarize_counter(project_0x170_direct_hits),
                    "project_plus_0x170_pointer": summarize_counter(project_0x170_pointer_hits),
                    "state_machine_direct": summarize_counter(state_machine_direct_hits),
                    "state_machine_pointer": summarize_counter(state_machine_pointer_hits),
                },
                "examples": {
                    "projects": [
                        {
                            "project_id": row["project_id"],
                            "static_def_handle": row["static_def_handle"],
                            "iterations": row["iterations"],
                            "worked_at_facility": row["worked_at_facility"],
                            "state_machine_ptr": hx(row["state_machine_ptr"]),
                            "addr": hx(row["addr"]),
                        }
                        for row in projects[: args.example_projects]
                    ],
                    "worked_projects": [
                        {
                            "project_id": row["project_id"],
                            "static_def_handle": row["static_def_handle"],
                            "iterations": row["iterations"],
                            "worked_at_facility": row["worked_at_facility"],
                            "state_machine_ptr": hx(row["state_machine_ptr"]),
                            "addr": hx(row["addr"]),
                        }
                        for row in worked_projects[: args.example_projects]
                    ],
                    "status_vtable_hits": status_examples,
                    "project_vtable_hits": project_examples,
                    "project_plus_0x170_vtable_hits": project_0x170_examples,
                    "state_machine_vtable_hits": state_machine_examples,
                    "current_state_selector": current_state_examples,
                    "prototype_progress_candidates": progress_examples,
                    "breakthrough_rows": breakthrough_examples,
                },
            }
        )
        return summary


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Read-only project progress/state/breakthrough root probe"
    )
    parser.add_argument("--pid", type=int, default=None, help="live hoi4.exe PID")
    parser.add_argument("--force", action="store_true", help="probe despite SHA mismatch")
    parser.add_argument("--max-projects", type=int, default=2048)
    parser.add_argument("--example-projects", type=int, default=12)
    parser.add_argument("--status-scan-window", type=lambda x: int(x, 0), default=MAX_SCAN_WINDOW)
    parser.add_argument("--state-scan-window", type=lambda x: int(x, 0), default=0x280)
    parser.add_argument("--out", default=None, help="write JSON proof to this file")
    args = parser.parse_args(argv)
    if args.pid is None:
        args.pid = winmem.find_pid("hoi4.exe")
    if not args.pid:
        raise SystemExit("hoi4.exe PID not found")
    if args.max_projects <= 0:
        raise SystemExit("--max-projects must be positive")

    started = time.perf_counter()
    summary = probe(args)
    summary["elapsed_ms"] = round((time.perf_counter() - started) * 1000, 3)

    if args.out:
        os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
        with open(args.out, "w", encoding="utf-8") as fh:
            json.dump(summary, fh, indent=2, sort_keys=True)
            fh.write("\n")
    print(json.dumps(summary, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
