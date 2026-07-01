"""Current-build probe for global/state scripted-state root candidates.

This lane-local READ-ONLY probe looks for current 870ebf CVariables and
CFlagManager instances outside the validated country roots. It can validate a
payload instance and reject exact root candidates, but it does not claim a
production root unless a CGameState-reachable path is proven.
"""

from __future__ import annotations

import argparse
import json
import os
import struct
import time
from collections import Counter
from datetime import datetime, timezone

import country_flags_payload_probe as flag_payload
import current_build_live_probe as roots

WORLD_ORACLE = os.path.join(roots.ROOT, "save_oracle", "output", "world.json")

SCAN_MAX_REGION = 64 * 1024 * 1024
MAX_VTABLE_HITS = 200000
MAX_GS_SCAN = 0x40000
MAX_REVERSE_HITS = 64


def read_world_oracle():
    with open(WORLD_ORACLE, "r", encoding="utf-8") as f:
        data = json.load(f)
    return data.get("variables") or {}, data.get("flags") or {}


def scan_vtable_objects(p, vtable, exclude=None):
    exclude = set(exclude or ())
    needle = struct.pack("<Q", vtable)
    hits = []
    regions = 0
    total_vtable_hits = 0
    started = time.perf_counter()
    for region_base, region_size in p.iter_regions(roots.HEAP_LO, roots.HEAP_HI, want_exec=False):
        if region_size > SCAN_MAX_REGION:
            continue
        regions += 1
        blob = roots.try_read(p, region_base, region_size)
        if not blob:
            continue
        pos = 0
        while True:
            idx = blob.find(needle, pos)
            if idx < 0:
                break
            pos = idx + 8
            total_vtable_hits += 1
            addr = region_base + idx
            if addr not in exclude and addr % 8 == 0:
                hits.append(addr)
            if total_vtable_hits >= MAX_VTABLE_HITS:
                return {
                    "objects": hits,
                    "regions_scanned": regions,
                    "vtable_hits": total_vtable_hits,
                    "truncated": True,
                    "elapsed_ms": int((time.perf_counter() - started) * 1000),
                }
    return {
        "objects": hits,
        "regions_scanned": regions,
        "vtable_hits": total_vtable_hits,
        "truncated": False,
        "elapsed_ms": int((time.perf_counter() - started) * 1000),
    }


def enumerate_variables_container(p, container):
    if roots.rtti_name(p, container) != roots.RTTI_VARIABLES:
        return None, "rtti_mismatch"
    hdr = roots.stable_variables_header(p, container)
    if hdr is None:
        return None, "header_unstable"
    base, size, mask = hdr
    if size == 0:
        return {}, "measured_empty"
    bucket_count = mask + 1
    if (
        not roots.heap_ok(base)
        or size > roots.MAX_SIZE
        or bucket_count < 1
        or bucket_count > roots.MAX_BUCKETS
        or (mask & bucket_count) != 0
        or size > bucket_count
    ):
        return None, "header_invalid"
    end = base + (bucket_count + 4) * roots.NODE
    exact = None
    for phase in range(0, roots.NODE, 8):
        tmp = {}
        dup = False
        addr = base + phase
        while addr + roots.NODE <= end:
            key = roots.read_msvc_string(p, addr)
            if key is not None:
                if key in tmp:
                    dup = True
                raw = roots.try_i64(p, addr + roots.NODE_VALUE)
                if raw is not None:
                    tmp[key] = raw
            addr += roots.NODE
        if not dup and len(tmp) == size:
            if exact is not None and exact != tmp:
                return None, "phase_ambiguous"
            exact = tmp
    if exact is None:
        return None, "size_gate_failed"
    return exact, "measured"


def compare_variables(bag, world_vars):
    world_keys = set(world_vars)
    keys = set(bag)
    overlap = sorted(keys & world_keys)
    exact_values = 0
    samples = []
    for key in overlap[:24]:
        raw = bag[key]
        decoded = raw / roots.VALUE_SCALE_RAW
        expected = world_vars.get(key)
        exact = isinstance(expected, (int, float)) and abs(decoded - float(expected)) < 1e-5
        exact_values += 1 if exact else 0
        samples.append(
            {
                "key": key,
                "raw": raw,
                "decoded": decoded,
                "oracle": expected,
                "exact_vs_oracle": exact,
            }
        )
    return {
        "entry_count": len(bag),
        "overlap_count": len(overlap),
        "world_key_count": len(world_keys),
        "exact_value_matches_in_sample": exact_values,
        "sample": samples,
    }


def read_flag_manager_rows(p, manager, expected_vtables):
    header = flag_payload.stable_flag_header(p, manager)
    if header is None:
        return None, "header_unstable"
    base, slot_count, entry_count = header
    if slot_count == 0 and entry_count == 0:
        return [], "measured_empty"
    if (
        not roots.heap_ok(base)
        or entry_count > slot_count
        or slot_count > 100000
        or entry_count > 100000
    ):
        return None, "header_invalid"
    rows = []
    for index in range(entry_count):
        addr = base + index * flag_payload.FLAG_ROW_STRIDE
        if (
            roots.try_u64(p, addr + flag_payload.FLAG_ROW_TOKEN_VTABLE)
            != expected_vtables["token_vtable"]
            or roots.try_u64(p, addr + flag_payload.FLAG_ROW_DATE_VTABLE)
            != expected_vtables["date_vtable"]
            or roots.try_u64(p, addr + flag_payload.FLAG_ROW_VALUE_VTABLE)
            != expected_vtables["value_vtable"]
        ):
            return None, "row_vtable_mismatch"
        token_id = roots.try_u32(p, addr + flag_payload.FLAG_ROW_TOKEN_U32)
        date_raw = roots.try_u32(p, addr + flag_payload.FLAG_ROW_DATE_U32)
        value = roots.try_u32(p, addr + flag_payload.FLAG_ROW_VALUE_U32)
        date = flag_payload.decode_datehour(date_raw)
        if token_id is None or token_id == 0 or date is None or value is None:
            return None, "row_scalar_invalid"
        rows.append(
            {
                "token_id": token_id,
                "date_hour_raw": date_raw,
                "date": date,
                "value": value,
            }
        )
    return rows, "measured"


def compare_flag_rows(rows, token_names, world_flags):
    world_keys = set(world_flags)
    resolved = []
    unresolved = 0
    for row in rows:
        name = token_names.get(row["token_id"])
        if name is None:
            unresolved += 1
            continue
        expected = world_flags.get(name)
        resolved.append(
            {
                **row,
                "name": name,
                "in_world_oracle": name in world_keys,
                "oracle": expected,
                "exact_vs_oracle": bool(
                    expected
                    and expected.get("value") == row["value"]
                    and expected.get("date") == row["date"]
                ),
            }
        )
    overlap = [row for row in resolved if row["in_world_oracle"]]
    exact = [row for row in overlap if row["exact_vs_oracle"]]
    return {
        "entry_count": len(rows),
        "resolved_names": len(resolved),
        "unresolved_names": unresolved,
        "world_overlap_count": len(overlap),
        "world_exact_count": len(exact),
        "sample": sorted(overlap, key=lambda row: row["name"])[:24],
    }


def direct_gamestate_refs(p, gamestate, target):
    needle = struct.pack("<Q", target)
    out = []
    offset = 0
    chunk_size = 0x4000
    while offset < MAX_GS_SCAN:
        want = min(chunk_size, MAX_GS_SCAN - offset)
        blob = roots.try_read(p, gamestate + offset, want)
        if blob:
            pos = 0
            while True:
                idx = blob.find(needle, pos)
                if idx < 0:
                    break
                out.append(
                    {
                        "offset": roots.hx(offset + idx),
                        "address": roots.hx(gamestate + offset + idx),
                    }
                )
                pos = idx + 8
        next_offset = offset + want
        if next_offset >= MAX_GS_SCAN:
            break
        offset = next_offset - 7
    return out


def reverse_pointer_hits(p, target):
    needle = struct.pack("<Q", target)
    out = []
    regions = 0
    for region_base, region_size in p.iter_regions(roots.HEAP_LO, roots.HEAP_HI, want_exec=False):
        if region_size > SCAN_MAX_REGION:
            continue
        regions += 1
        blob = roots.try_read(p, region_base, region_size)
        if not blob:
            continue
        pos = 0
        while True:
            idx = blob.find(needle, pos)
            if idx < 0:
                break
            out.append(roots.hx(region_base + idx))
            pos = idx + 8
            if len(out) >= MAX_REVERSE_HITS:
                return out, regions, True
    return out, regions, False


def run_probe(fast_direct_roots=False, skip_token_dictionary=False):
    world_vars, world_flags = read_world_oracle()
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
        country_roots = roots.resolve_country_roots(p)
        country_variable_containers = set()
        country_flag_managers = set()
        for row in country_roots["rows"]:
            var = roots.try_u64(p, row["country_ptr"] + roots.OBJ_VARIABLES)
            if roots.heap_ok(var):
                country_variable_containers.add(var)
            mgr = roots.try_u64(p, row["country_ptr"] + roots.OBJ_FLAGS)
            if roots.heap_ok(mgr):
                country_flag_managers.add(mgr)

        variable_vtable = roots.try_u64(p, next(iter(country_variable_containers)))
        flag_manager_vtable = roots.try_u64(p, next(iter(country_flag_managers)))
        if not isinstance(variable_vtable, int) or not isinstance(flag_manager_vtable, int):
            raise RuntimeError("could not derive country scripted-state vtables")

        flag_expected = flag_payload.read_expected_row_vtables(p, country_roots["rows"])
        token_record_vtable = roots.try_u64(
            p, flag_expected["source_manager"] + flag_payload.TOKEN_RECORD_VTABLE_OFF
        )

        if fast_direct_roots:
            global_var_container = roots.try_u64(p, country_roots["gamestate"] + 0x980)
            global_flag_manager = roots.try_u64(p, country_roots["gamestate"] + 0x258)
            variable_scan = {
                "objects": [global_var_container]
                if roots.heap_ok(global_var_container)
                else [],
                "regions_scanned": 0,
                "vtable_hits": 0,
                "truncated": False,
                "elapsed_ms": 0,
                "mode": "direct_cgamestate_offsets",
            }
            flag_scan = {
                "objects": [global_flag_manager]
                if roots.heap_ok(global_flag_manager)
                else [],
                "regions_scanned": 0,
                "vtable_hits": 0,
                "truncated": False,
                "elapsed_ms": 0,
                "mode": "direct_cgamestate_offsets",
            }
        else:
            variable_scan = scan_vtable_objects(p, variable_vtable, country_variable_containers)

        variable_candidates = []
        variable_status = Counter()
        direct_var_root = roots.try_u64(p, country_roots["gamestate"] + 0x980)
        direct_flag_root = roots.try_u64(p, country_roots["gamestate"] + 0x258)
        for container in variable_scan["objects"]:
            bag, status = enumerate_variables_container(p, container)
            variable_status[status] += 1
            if status != "measured" or bag is None:
                continue
            cmp = compare_variables(bag, world_vars)
            if cmp["overlap_count"] > 0:
                direct_refs = direct_gamestate_refs(p, country_roots["gamestate"], container)
                if direct_refs:
                    reverse_hits, reverse_regions, reverse_truncated = [], 0, False
                else:
                    reverse_hits, reverse_regions, reverse_truncated = reverse_pointer_hits(
                        p, container
                    )
                variable_candidates.append(
                    {
                        "container": roots.hx(container),
                        "rtti": roots.RTTI_VARIABLES,
                        "comparison": cmp,
                        "direct_gamestate_refs_0x0_0x40000": direct_refs,
                        "reverse_pointer_hits": reverse_hits,
                        "reverse_regions_scanned": reverse_regions,
                        "reverse_truncated": reverse_truncated,
                    }
                )

        if not fast_direct_roots:
            flag_scan = scan_vtable_objects(p, flag_manager_vtable, country_flag_managers)
        raw_flag_candidates = []
        flag_status = Counter()
        wanted_tokens = set()
        for manager in flag_scan["objects"]:
            rows, status = read_flag_manager_rows(p, manager, flag_expected)
            flag_status[status] += 1
            if status != "measured" or rows is None:
                continue
            if rows:
                raw_flag_candidates.append((manager, rows))
                wanted_tokens.update(row["token_id"] for row in rows)

        dictionary = {"_resolved_by_int": {}, "wanted_resolved": 0, "wanted_tokens": 0}
        if wanted_tokens and isinstance(token_record_vtable, int) and not skip_token_dictionary:
            dictionary = flag_payload.scan_token_dictionary(
                p,
                token_record_vtable,
                wanted_tokens,
                wanted_only=fast_direct_roots,
            )
        resolved = dictionary.get("_resolved_by_int", {})
        token_names = {
            int(token): row["name"]
            for token, row in resolved.items()
            if isinstance(row, dict) and row.get("name")
        }

        flag_candidates = []
        for manager, rows in raw_flag_candidates:
            cmp = compare_flag_rows(rows, token_names, world_flags)
            if cmp["world_overlap_count"] > 0 or (fast_direct_roots and skip_token_dictionary):
                direct_refs = direct_gamestate_refs(p, country_roots["gamestate"], manager)
                if direct_refs:
                    reverse_hits, reverse_regions, reverse_truncated = [], 0, False
                else:
                    reverse_hits, reverse_regions, reverse_truncated = reverse_pointer_hits(
                        p, manager
                    )
                flag_candidates.append(
                    {
                        "manager": roots.hx(manager),
                        "rtti": roots.RTTI_FLAG_MANAGER,
                        "comparison": cmp,
                        "sample_raw_rows": rows[:16],
                        "direct_gamestate_refs_0x0_0x40000": direct_refs,
                        "reverse_pointer_hits": reverse_hits,
                        "reverse_regions_scanned": reverse_regions,
                        "reverse_truncated": reverse_truncated,
                    }
                )

        variable_candidates.sort(
            key=lambda item: (
                item["comparison"]["overlap_count"],
                item["comparison"]["entry_count"],
            ),
            reverse=True,
        )
        flag_candidates.sort(
            key=lambda item: (
                item["comparison"]["world_exact_count"],
                item["comparison"]["world_overlap_count"],
            ),
            reverse=True,
        )

        elapsed_ms = int((time.perf_counter() - started) * 1000)
        return {
            "schema": "scripted_state_flags_variables_live_global_state_root_probe/v1",
            "created_utc": datetime.now(timezone.utc).isoformat(),
            "pid": p.pid,
            "sha256": sha,
            "elapsed_ms": elapsed_ms,
            "source_roots": {
                "module_base": roots.hx(country_roots["module_base"]),
                "gamestate_global": roots.hx(p.module_base + roots.CG_STATE_DATE_OFFSET),
                "gamestate": roots.hx(country_roots["gamestate"]),
                "country_table": roots.hx(country_roots["country_table"]),
                "country_count": country_roots["country_count"],
                "validated_country_rows": len(country_roots["rows"]),
            },
            "oracle": {
                "world_variables": len(world_vars),
                "world_flags": len(world_flags),
                "path": WORLD_ORACLE,
            },
            "scans": {
                "cvariables_vtable": roots.hx(variable_vtable),
                "cvariables_scan": {
                    k: v for k, v in variable_scan.items() if k != "objects"
                },
                "cvariables_status": dict(variable_status),
                "country_variable_containers_excluded": len(country_variable_containers),
                "cflagmanager_vtable": roots.hx(flag_manager_vtable),
                "cflagmanager_scan": {
                    k: v for k, v in flag_scan.items() if k != "objects"
                },
                "cflagmanager_status": dict(flag_status),
                "country_flag_managers_excluded": len(country_flag_managers),
                "token_dictionary": {
                    "token_record_vtable": roots.hx(token_record_vtable),
                    "wanted_tokens": dictionary.get("wanted_tokens"),
                    "wanted_resolved": dictionary.get("wanted_resolved"),
                    "wanted_unresolved": dictionary.get("wanted_unresolved"),
                    "candidate_records": dictionary.get("candidate_records"),
                    "unique_tokens": dictionary.get("unique_tokens"),
                    "elapsed_ms": dictionary.get("elapsed_ms"),
                },
            },
            "validated_instances": {
                "global_variable_like_candidates": variable_candidates[:8],
                "global_flag_like_candidates": flag_candidates[:8],
            },
            "rejected_root_candidates": [],
            "overturned_prior_probe_rejections": [
                {
                    "candidate": "direct CGameState[0x0..0x40000] -> global CVariables container",
                    "reason": "prior broad artifact used a direct-reference scanner that failed when a chunk read returned None; the chunked scanner now validates the qword root.",
                },
                {
                    "candidate": "direct CGameState[0x0..0x40000] -> global CFlagManager",
                    "reason": "prior broad artifact used a direct-reference scanner that failed when a chunk read returned None; the chunked scanner now validates the qword root.",
                },
            ],
            "validated_root_candidates": [
                {
                    "candidate": "CGameState+0x980 -> global CVariables",
                    "root_offset": "0x980",
                    "root_address": roots.hx(country_roots["gamestate"] + 0x980),
                    "root_value": roots.hx(direct_var_root),
                    "expected_rtti": roots.RTTI_VARIABLES,
                    "status": "validated"
                    if variable_candidates
                    and variable_candidates[0]["direct_gamestate_refs_0x0_0x40000"]
                    else "not_validated",
                    "evidence": variable_candidates[0]["direct_gamestate_refs_0x0_0x40000"]
                    if variable_candidates
                    else [],
                },
                {
                    "candidate": "CGameState+0x258 -> global CFlagManager",
                    "root_offset": "0x258",
                    "root_address": roots.hx(country_roots["gamestate"] + 0x258),
                    "root_value": roots.hx(direct_flag_root),
                    "expected_rtti": roots.RTTI_FLAG_MANAGER,
                    "status": "validated"
                    if flag_candidates
                    and flag_candidates[0]["direct_gamestate_refs_0x0_0x40000"]
                    else "not_validated",
                    "evidence": flag_candidates[0]["direct_gamestate_refs_0x0_0x40000"]
                    if flag_candidates
                    else [],
                },
            ],
        }


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", required=True)
    ap.add_argument(
        "--fast-direct-roots",
        action="store_true",
        help="validate known direct CGameState root candidates without full vtable sweeps",
    )
    ap.add_argument(
        "--skip-token-dictionary",
        action="store_true",
        help="skip heap-wide token-name dictionary scan; validates raw CFlagManager payload only",
    )
    args = ap.parse_args()
    result = run_probe(
        fast_direct_roots=args.fast_direct_roots,
        skip_token_dictionary=args.skip_token_dictionary,
    )
    os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(result, f, indent=2, sort_keys=True)
        f.write("\n")
    compact = {
        "elapsed_ms": result["elapsed_ms"],
        "cvariables_status": result["scans"]["cvariables_status"],
        "global_variable_like_candidates": len(
            result["validated_instances"]["global_variable_like_candidates"]
        ),
        "cflagmanager_status": result["scans"]["cflagmanager_status"],
        "global_flag_like_candidates": len(
            result["validated_instances"]["global_flag_like_candidates"]
        ),
        "rejected_root_candidates": result["rejected_root_candidates"],
        "validated_root_candidates": result["validated_root_candidates"],
    }
    print(json.dumps(compact, indent=2, sort_keys=True))
    print(args.out)


if __name__ == "__main__":
    raise SystemExit(main())
