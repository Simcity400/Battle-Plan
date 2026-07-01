"""Probe current-build state-scope scripted variable/flag roots.

This lane-local validator is read-only. It resolves current 870ebf CState
objects through the already-used CGameState -> CProvince array path, then scans
validated state objects for direct pointer or embedded CVariables/CFlagManager
roots. It intentionally does not edit or call shared collector entrypoints.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from collections import Counter, defaultdict
from datetime import datetime, timezone

LANE_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, LANE_DIR)

import current_build_live_probe as roots  # noqa: E402
import country_flags_payload_probe as flag_payload  # noqa: E402
import global_state_root_probe as global_roots  # noqa: E402


CGAMESTATE_CPROVINCE_ARRAY = 0x2B0
CGAMESTATE_CPROVINCE_COUNT = 0x2BC
CPROVINCE_VTABLE_RVA = 0x295AF68
CPROVINCE_OFF_ID = 0xA4
CPROVINCE_OFF_STATE = 0xC0
STATE_VTABLE_RVA = 0x29206C0
STATE_OFF_SID = 0x58

MAX_PROVINCES = 20000
MIN_USABLE_STATES = 1000
MAX_STATE_SCAN_BYTES = 0x4000
STATE_SCAN_CHUNK = 0x1000
MAX_SAMPLE_HITS = 16


def read_qword_from_blob(blob, off):
    return int.from_bytes(blob[off : off + 8], "little")


def resolve_state_roots(p):
    country_roots = roots.resolve_country_roots(p)
    gamestate = country_roots["gamestate"]
    module_base = country_roots["module_base"]
    cprovince_vtable = module_base + CPROVINCE_VTABLE_RVA
    state_vtable = module_base + STATE_VTABLE_RVA
    province_array = roots.try_u64(p, gamestate + CGAMESTATE_CPROVINCE_ARRAY)
    province_count = roots.try_u32(p, gamestate + CGAMESTATE_CPROVINCE_COUNT)
    if not roots.heap_ok(province_array) or not isinstance(province_count, int):
        raise RuntimeError("CGameState province array/count did not resolve")
    if province_count <= 0 or province_count > MAX_PROVINCES:
        raise RuntimeError(f"province count out of bounds: {province_count}")
    ptrs = roots.try_read(p, province_array, province_count * 8)
    if not ptrs or len(ptrs) < province_count * 8:
        raise RuntimeError("CProvince pointer table unreadable")

    states = {}
    province_rows = 0
    invalid_provinces = Counter()
    state_conflicts = []
    for province_id in range(1, province_count):
        province_ptr = read_qword_from_blob(ptrs, province_id * 8)
        if province_ptr == 0:
            continue
        if not roots.heap_ok(province_ptr):
            invalid_provinces["province_pointer_not_heap"] += 1
            continue
        if roots.try_u64(p, province_ptr) != cprovince_vtable:
            invalid_provinces["province_vtable_mismatch"] += 1
            continue
        if roots.try_u32(p, province_ptr + CPROVINCE_OFF_ID) != province_id:
            invalid_provinces["province_id_mismatch"] += 1
            continue
        state_ptr = roots.try_u64(p, province_ptr + CPROVINCE_OFF_STATE)
        if not roots.heap_ok(state_ptr):
            invalid_provinces["state_pointer_not_heap"] += 1
            continue
        if roots.try_u64(p, state_ptr) != state_vtable:
            invalid_provinces["state_vtable_mismatch"] += 1
            continue
        state_id = roots.try_u32(p, state_ptr + STATE_OFF_SID)
        if not isinstance(state_id, int) or state_id == 0 or state_id > 65535:
            invalid_provinces["state_id_invalid"] += 1
            continue
        previous = states.get(state_id)
        if previous is not None and previous != state_ptr:
            state_conflicts.append(
                {
                    "state_id": state_id,
                    "previous": roots.hx(previous),
                    "current": roots.hx(state_ptr),
                }
            )
            continue
        states[state_id] = state_ptr
        province_rows += 1

    if len(states) < MIN_USABLE_STATES:
        raise RuntimeError(f"only {len(states)} state roots resolved")
    return {
        "country_roots": country_roots,
        "gamestate": gamestate,
        "module_base": module_base,
        "province_array": province_array,
        "province_count": province_count,
        "province_rows": province_rows,
        "invalid_provinces": dict(invalid_provinces),
        "state_conflicts": state_conflicts[:MAX_SAMPLE_HITS],
        "states": states,
        "cprovince_vtable": cprovince_vtable,
        "state_vtable": state_vtable,
    }


def scan_state_objects(p, states, variable_vtable, flag_manager_vtable, scan_bytes):
    pointer_hits = {
        "cvariables": defaultdict(list),
        "cflagmanager": defaultdict(list),
    }
    embedded_hits = {
        "cvariables": defaultdict(list),
        "cflagmanager": defaultdict(list),
    }
    chunks = Counter()

    for state_id, state_ptr in sorted(states.items()):
        offset = 0
        while offset < scan_bytes:
            want = min(STATE_SCAN_CHUNK, scan_bytes - offset)
            blob = roots.try_read(p, state_ptr + offset, want)
            if not blob:
                chunks["failed"] += 1
                offset += want
                continue
            chunks["read"] += 1
            limit = len(blob) - 8
            for local in range(0, limit + 1, 8):
                off = offset + local
                value = read_qword_from_blob(blob, local)
                if value == variable_vtable:
                    embedded_hits["cvariables"][off].append((state_id, state_ptr, state_ptr + off))
                elif value == flag_manager_vtable:
                    embedded_hits["cflagmanager"][off].append(
                        (state_id, state_ptr, state_ptr + off)
                    )
                if roots.heap_ok(value):
                    target_vtable = roots.try_u64(p, value)
                    if target_vtable == variable_vtable:
                        pointer_hits["cvariables"][off].append((state_id, state_ptr, value))
                    elif target_vtable == flag_manager_vtable:
                        pointer_hits["cflagmanager"][off].append((state_id, state_ptr, value))
            offset += len(blob) if len(blob) < want else want
    return pointer_hits, embedded_hits, dict(chunks)


def sample_hits(hit_map):
    out = []
    for off, hits in sorted(hit_map.items(), key=lambda item: (-len(item[1]), item[0])):
        out.append(
            {
                "offset": roots.hx(off),
                "hit_count": len(hits),
                "samples": [
                    {
                        "state_id": state_id,
                        "state_ptr": roots.hx(state_ptr),
                        "target": roots.hx(target),
                    }
                    for state_id, state_ptr, target in hits[:MAX_SAMPLE_HITS]
                ],
            }
        )
    return out


def validate_variable_candidate(p, states, off, mode):
    status = Counter()
    entries = 0
    non_empty = 0
    samples = []
    for state_id, state_ptr in sorted(states.items()):
        container = state_ptr + off if mode == "embedded" else roots.try_u64(p, state_ptr + off)
        if not roots.heap_ok(container):
            status["no_heap_pointer"] += 1
            continue
        bag, state_status = global_roots.enumerate_variables_container(p, container)
        status[state_status] += 1
        if state_status == "measured" and bag is not None:
            entries += len(bag)
            non_empty += 1
            if len(samples) < MAX_SAMPLE_HITS:
                samples.append(
                    {
                        "state_id": state_id,
                        "container": roots.hx(container),
                        "entry_count": len(bag),
                        "sample": [
                            {"key": key, "raw": raw, "decoded": raw / roots.VALUE_SCALE_RAW}
                            for key, raw in list(sorted(bag.items()))[:4]
                        ],
                    }
                )
    return {
        "offset": roots.hx(off),
        "mode": mode,
        "status": dict(status),
        "non_empty_states": non_empty,
        "total_entries": entries,
        "samples": samples,
    }


def validate_flag_candidate(p, states, off, mode, expected_vtables):
    status = Counter()
    rows_total = 0
    non_empty = 0
    samples = []
    for state_id, state_ptr in sorted(states.items()):
        manager = state_ptr + off if mode == "embedded" else roots.try_u64(p, state_ptr + off)
        if not roots.heap_ok(manager):
            status["no_heap_pointer"] += 1
            continue
        rows, state_status = global_roots.read_flag_manager_rows(p, manager, expected_vtables)
        status[state_status] += 1
        if state_status == "measured" and rows is not None:
            rows_total += len(rows)
            if rows:
                non_empty += 1
                if len(samples) < MAX_SAMPLE_HITS:
                    samples.append(
                        {
                            "state_id": state_id,
                            "manager": roots.hx(manager),
                            "row_count": len(rows),
                            "sample": rows[:4],
                        }
                    )
    return {
        "offset": roots.hx(off),
        "mode": mode,
        "status": dict(status),
        "non_empty_states": non_empty,
        "total_rows": rows_total,
        "samples": samples,
    }


def summarize_validations(p, states, pointer_hits, embedded_hits, flag_expected):
    validations = {"variables": [], "flags": []}
    for mode, source in (("pointer", pointer_hits), ("embedded", embedded_hits)):
        for off in source["cvariables"]:
            validations["variables"].append(validate_variable_candidate(p, states, off, mode))
        for off in source["cflagmanager"]:
            validations["flags"].append(
                validate_flag_candidate(p, states, off, mode, flag_expected)
            )
    validations["variables"].sort(
        key=lambda row: (row["non_empty_states"], row["total_entries"]), reverse=True
    )
    validations["flags"].sort(
        key=lambda row: (row["non_empty_states"], row["total_rows"]), reverse=True
    )
    return validations


def fully_validated_roots(validations, states_seen):
    out = []
    allowed_statuses = {"measured", "measured_empty"}
    for row in validations["variables"]:
        total = sum(row["status"].values())
        if (
            row["mode"] == "pointer"
            and total == states_seen
            and set(row["status"]).issubset(allowed_statuses)
        ):
            out.append(
                {
                    "candidate": f"CState+{row['offset']} -> CVariables",
                    "status": "validated",
                    "root_offset": row["offset"],
                    "mode": row["mode"],
                    "rtti": roots.RTTI_VARIABLES,
                    "state_count": states_seen,
                    "status_counts": row["status"],
                    "non_empty_states": row["non_empty_states"],
                    "total_entries": row["total_entries"],
                    "sample": row["samples"][:4],
                }
            )
    for row in validations["flags"]:
        total = sum(row["status"].values())
        if (
            row["mode"] == "pointer"
            and total == states_seen
            and set(row["status"]).issubset(allowed_statuses)
        ):
            out.append(
                {
                    "candidate": f"CState+{row['offset']} -> CFlagManager",
                    "status": "validated",
                    "root_offset": row["offset"],
                    "mode": row["mode"],
                    "rtti": roots.RTTI_FLAG_MANAGER,
                    "state_count": states_seen,
                    "status_counts": row["status"],
                    "non_empty_states": row["non_empty_states"],
                    "total_rows": row["total_rows"],
                    "sample": row["samples"][:4],
                }
            )
    return out


def rejected_partial_offsets(validations, states_seen):
    out = []
    for kind, rows in (("CVariables", validations["variables"]), ("CFlagManager", validations["flags"])):
        for row in rows:
            total = sum(row["status"].values())
            invalid = {
                key: value
                for key, value in row["status"].items()
                if key not in {"measured", "measured_empty"}
            }
            if total != states_seen or invalid:
                out.append(
                    {
                        "candidate": f"CState+{row['offset']} -> {kind}",
                        "status": "rejected_as_production_root",
                        "reason": "candidate did not satisfy all-state measured/measured-empty gate",
                        "mode": row["mode"],
                        "status_counts": row["status"],
                    }
                )
    return out[:32]


def run_probe(scan_bytes=MAX_STATE_SCAN_BYTES):
    p = roots.winmem.open_process("hoi4.exe")
    if p is None:
        raise SystemExit("hoi4.exe not running")
    with p:
        sha = str((p.fingerprint().get("sha256") or "")).lower()
        if sha != roots.EXPECTED_SHA256:
            raise SystemExit(
                "live hoi4.exe sha256 %s != expected %s" % (sha, roots.EXPECTED_SHA256)
            )
        started = time.perf_counter()
        state_roots = resolve_state_roots(p)
        global_var = roots.try_u64(p, state_roots["gamestate"] + 0x980)
        global_flag = roots.try_u64(p, state_roots["gamestate"] + 0x258)
        variable_vtable = roots.try_u64(p, global_var) if roots.heap_ok(global_var) else None
        flag_manager_vtable = roots.try_u64(p, global_flag) if roots.heap_ok(global_flag) else None
        if not isinstance(variable_vtable, int) or not isinstance(flag_manager_vtable, int):
            raise RuntimeError("could not derive scripted-state vtables from global roots")

        flag_expected = flag_payload.read_expected_row_vtables(
            p, state_roots["country_roots"]["rows"]
        )
        pointer_hits, embedded_hits, chunks = scan_state_objects(
            p,
            state_roots["states"],
            variable_vtable,
            flag_manager_vtable,
            scan_bytes,
        )
        validations = summarize_validations(
            p, state_roots["states"], pointer_hits, embedded_hits, flag_expected
        )
        states_seen = len(state_roots["states"])
        accepted_roots = fully_validated_roots(validations, states_seen)

        rejected = []
        if not pointer_hits["cvariables"]:
            rejected.append(
                {
                    "candidate": f"CState[0x0..0x{scan_bytes:X}] direct qword pointer -> CVariables",
                    "status": "rejected",
                    "evidence": "0 qword slots across validated CState objects pointed to the current CVariables vtable",
                }
            )
        if not embedded_hits["cvariables"]:
            rejected.append(
                {
                    "candidate": f"CState[0x0..0x{scan_bytes:X}] embedded CVariables object",
                    "status": "rejected",
                    "evidence": "0 qword slots across validated CState objects equaled the current CVariables vtable",
                }
            )
        if not pointer_hits["cflagmanager"]:
            rejected.append(
                {
                    "candidate": f"CState[0x0..0x{scan_bytes:X}] direct qword pointer -> CFlagManager",
                    "status": "rejected",
                    "evidence": "0 qword slots across validated CState objects pointed to the current CFlagManager vtable",
                }
            )
        if not embedded_hits["cflagmanager"]:
            rejected.append(
                {
                    "candidate": f"CState[0x0..0x{scan_bytes:X}] embedded CFlagManager object",
                    "status": "rejected",
                    "evidence": "0 qword slots across validated CState objects equaled the current CFlagManager vtable",
                }
            )

        elapsed_ms = int((time.perf_counter() - started) * 1000)
        return {
            "schema": "scripted_state_flags_variables_live_state_scope_root_probe/v1",
            "created_utc": datetime.now(timezone.utc).isoformat(),
            "pid": p.pid,
            "sha256": sha,
            "elapsed_ms": elapsed_ms,
            "scan_window": {
                "bytes": scan_bytes,
                "qword_aligned": True,
                "chunks": chunks,
            },
            "source_roots": {
                "module_base": roots.hx(state_roots["module_base"]),
                "gamestate_global": roots.hx(
                    state_roots["module_base"] + roots.CG_STATE_DATE_OFFSET
                ),
                "gamestate": roots.hx(state_roots["gamestate"]),
                "province_array": roots.hx(state_roots["province_array"]),
                "province_count": state_roots["province_count"],
                "province_rows": state_roots["province_rows"],
                "cprovince_vtable": roots.hx(state_roots["cprovince_vtable"]),
                "state_vtable": roots.hx(state_roots["state_vtable"]),
                "states_seen": len(state_roots["states"]),
                "state_id_offset": roots.hx(STATE_OFF_SID),
                "invalid_provinces": state_roots["invalid_provinces"],
                "state_conflicts": state_roots["state_conflicts"],
            },
            "scripted_state_vtables": {
                "cvariables": roots.hx(variable_vtable),
                "cflagmanager": roots.hx(flag_manager_vtable),
                "cvariables_rtti": roots.RTTI_VARIABLES,
                "cflagmanager_rtti": roots.RTTI_FLAG_MANAGER,
                "derived_from": {
                    "global_variables_root": roots.hx(global_var),
                    "global_flags_root": roots.hx(global_flag),
                },
            },
            "hits": {
                "pointer_to_cvariables": sample_hits(pointer_hits["cvariables"]),
                "embedded_cvariables": sample_hits(embedded_hits["cvariables"]),
                "pointer_to_cflagmanager": sample_hits(pointer_hits["cflagmanager"]),
                "embedded_cflagmanager": sample_hits(embedded_hits["cflagmanager"]),
            },
            "validated_candidates": validations,
            "validated_roots": accepted_roots,
            "rejected_partial_offsets": rejected_partial_offsets(validations, states_seen),
            "rejected_candidates": rejected,
        }


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", required=True)
    ap.add_argument("--scan-bytes", type=lambda value: int(value, 0), default=MAX_STATE_SCAN_BYTES)
    args = ap.parse_args()
    if args.scan_bytes <= 0 or args.scan_bytes > 0x10000:
        raise SystemExit("--scan-bytes must be in 1..0x10000")
    result = run_probe(args.scan_bytes)
    os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(result, f, indent=2, sort_keys=True)
        f.write("\n")
    compact = {
        "elapsed_ms": result["elapsed_ms"],
        "states_seen": result["source_roots"]["states_seen"],
        "province_count": result["source_roots"]["province_count"],
        "pointer_to_cvariables_offsets": len(result["hits"]["pointer_to_cvariables"]),
        "embedded_cvariables_offsets": len(result["hits"]["embedded_cvariables"]),
        "pointer_to_cflagmanager_offsets": len(result["hits"]["pointer_to_cflagmanager"]),
        "embedded_cflagmanager_offsets": len(result["hits"]["embedded_cflagmanager"]),
        "variable_validations": result["validated_candidates"]["variables"][:4],
        "flag_validations": result["validated_candidates"]["flags"][:4],
        "validated_roots": result["validated_roots"],
        "rejected_candidates": result["rejected_candidates"],
    }
    print(json.dumps(compact, indent=2, sort_keys=True))
    print(args.out)


if __name__ == "__main__":
    raise SystemExit(main())
