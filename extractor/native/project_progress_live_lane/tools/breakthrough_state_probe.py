"""Validate project breakthrough state rows against the save oracle.

Read-only development probe. This does not feed the production hot loop. It
checks whether the existing domain-41 breakthrough compact row
(`tag_db_id, specialization_handle, points_raw`) has exact save semantics for
the current build and fixture.
"""

import argparse
import itertools
import json
import os
import sys
import time
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[4]
MEMORY_READER = REPO_ROOT / "extractor" / "memory_reader"
sys.path.insert(0, str(MEMORY_READER))

import projects_progress_probe as pp  # noqa: E402
import winmem  # noqa: E402


SPEC_KEYS = (
    "specialization_nuclear",
    "specialization_naval",
    "specialization_air",
    "specialization_land",
)


def hx(value):
    return pp.hx(value)


def load_save_breakthroughs(path):
    with open(path, "r", encoding="utf-8") as fh:
        data = json.load(fh)
    countries = {}
    for tag, country in (data.get("countries") or {}).items():
        breakthrough = country.get("breakthrough") or {}
        countries[tag] = {
            key: int(breakthrough.get(key) or 0)
            for key in SPEC_KEYS
        }
    return {
        "path": path,
        "meta": data.get("meta") or {},
        "countries": countries,
    }


def collect_live_breakthroughs(p):
    module_base = p.module_base
    country_vtable = module_base + pp.COUNTRY_VTABLE_RVA
    status_a_vtable = module_base + pp.PROJECT_PROGRAM_STATUS_A_VTABLE_RVA
    status_b_vtable = module_base + pp.PROJECT_PROGRAM_STATUS_B_VTABLE_RVA
    breakthrough_vtable = module_base + pp.PROJECT_BREAKTHROUGH_PROGRESS_VTABLE_RVA

    game_state = pp.try_u64(p, module_base + pp.CG_STATE_DATE_OFFSET)
    country_database = pp.try_u64(p, module_base + pp.COUNTRY_DATABASE_OFFSET)
    if not pp.is_heap_ptr(game_state) or not pp.is_heap_ptr(country_database):
        raise RuntimeError("failed to resolve CGameState/country database")
    country_table = pp.try_u64(p, game_state + pp.CGAMESTATE_COUNTRY_TABLE)
    tag_table = pp.try_u64(p, country_database)
    row_count = pp.tag_row_count(p, country_database)
    if not row_count or not pp.is_heap_ptr(country_table) or not pp.is_heap_ptr(tag_table):
        raise RuntimeError("failed to validate country table/tag table")

    status_rows = 0
    parse_failures = []
    live = {}
    all_handles = Counter()
    examples = []
    for tag_db_id in range(1, row_count):
        country_ptr = pp.try_u64(p, country_table + tag_db_id * 8)
        if not country_ptr:
            continue
        if not pp.is_heap_ptr(country_ptr) or pp.try_u64(p, country_ptr) != country_vtable:
            continue
        if pp.try_u32(p, country_ptr + pp.OBJ_TAG_DB_ID) != tag_db_id:
            continue
        tag = pp.read_small_inline_string(p, tag_table + tag_db_id * 32)
        status_ptr = pp.try_u64(p, country_ptr + pp.COUNTRY_OFF_PROGRAM_STATUS)
        if (
            not pp.is_heap_ptr(status_ptr)
            or pp.try_u64(p, status_ptr) != status_a_vtable
            or pp.try_u64(p, status_ptr + 8) != status_b_vtable
        ):
            continue
        status_rows += 1
        rows = pp.read_breakthrough_rows(
            p,
            status_ptr + 0x50,
            breakthrough_vtable,
            max_rows=16,
        )
        if rows is None:
            parse_failures.append(
                {
                    "tag": tag,
                    "tag_db_id": tag_db_id,
                    "status_ptr": hx(status_ptr),
                }
            )
            continue
        if not rows:
            continue
        by_handle = {}
        for row in rows:
            handle = int(row["specialization_handle"])
            points = int(row["points_raw"])
            by_handle[handle] = points
            all_handles[handle] += 1
        live[tag] = {
            "tag_db_id": tag_db_id,
            "status_ptr": status_ptr,
            "rows": by_handle,
        }
        if len(examples) < 16:
            examples.append(
                {
                    "tag": tag,
                    "tag_db_id": tag_db_id,
                    "status_ptr": hx(status_ptr),
                    "rows": [
                        {
                            "specialization_handle": handle,
                            "points_raw": points,
                        }
                        for handle, points in sorted(by_handle.items())
                    ],
                }
            )
    return {
        "roots": {
            "game_state": game_state,
            "country_database": country_database,
            "country_table": country_table,
            "tag_row_count": row_count,
        },
        "expected_vtables": {
            "CBreakthroughProgress": breakthrough_vtable,
        },
        "status_rows": status_rows,
        "parse_failures": parse_failures,
        "live": live,
        "handle_counts": dict(sorted(all_handles.items())),
        "examples": examples,
    }


def score_mapping(handles, live, save):
    candidates = []
    save_countries = save["countries"]
    for perm in itertools.permutations(SPEC_KEYS, len(handles)):
        mapping = dict(zip(handles, perm))
        mismatches = []
        matched_rows = 0
        exact_rows = 0
        live_tags_missing_from_save = []
        for tag, row in live.items():
            save_row = save_countries.get(tag)
            if save_row is None:
                live_tags_missing_from_save.append(tag)
                continue
            for handle, live_points in sorted(row["rows"].items()):
                key = mapping.get(handle)
                if key is None:
                    continue
                matched_rows += 1
                save_points = save_row.get(key)
                if live_points == save_points:
                    exact_rows += 1
                elif len(mismatches) < 64:
                    mismatches.append(
                        {
                            "tag": tag,
                            "tag_db_id": row["tag_db_id"],
                            "specialization_handle": handle,
                            "mapped_key": key,
                            "live_points_raw": live_points,
                            "save_points_raw": save_points,
                        }
                    )
        candidates.append(
            {
                "mapping": mapping,
                "matched_rows": matched_rows,
                "exact_rows": exact_rows,
                "mismatch_count": matched_rows - exact_rows,
                "mismatch_examples": mismatches,
                "live_tags_missing_from_save": live_tags_missing_from_save[:64],
                "live_tags_missing_from_save_count": len(live_tags_missing_from_save),
            }
        )
    candidates.sort(
        key=lambda row: (
            row["mismatch_count"],
            -row["exact_rows"],
            row["live_tags_missing_from_save_count"],
        )
    )
    return candidates


def compact_sample_rows(live, limit):
    out = []
    for tag, row in sorted(live.items()):
        for handle, points in sorted(row["rows"].items()):
            out.append(
                {
                    "tag": tag,
                    "tag_db_id": row["tag_db_id"],
                    "specialization_handle": handle,
                    "points_raw": points,
                }
            )
            if len(out) >= limit:
                return out
    return out


def probe(args):
    save = load_save_breakthroughs(args.save_projects_json)
    with winmem.Process(args.pid, "hoi4.exe") as p:
        fp = p.fingerprint()
        sha = str(fp.get("sha256") or "").lower()
        if sha != pp.EXPECTED_SHA256 and not args.force:
            raise SystemExit(
                "live hoi4.exe sha256 %s != expected %s; pass --force to probe anyway"
                % (sha, pp.EXPECTED_SHA256)
            )
        live_summary = collect_live_breakthroughs(p)
        live = live_summary["live"]
        handles = sorted(live_summary["handle_counts"])
        mapping_candidates = score_mapping(handles, live, save) if handles else []
        best = mapping_candidates[0] if mapping_candidates else None
        accepted = bool(
            best
            and len(handles) == len(SPEC_KEYS)
            and not live_summary["parse_failures"]
            and best["matched_rows"] > 0
            and best["mismatch_count"] == 0
            and best["live_tags_missing_from_save_count"] == 0
        )
        return {
            "schema": "project_progress_breakthrough_state_probe_v1",
            "utc": datetime.now(timezone.utc).isoformat(),
            "pid": args.pid,
            "module_path": fp.get("path"),
            "module_base": hx(p.module_base),
            "sha256": sha,
            "save_projects_json": save["path"],
            "save_date": save["meta"].get("save_date") or save["meta"].get("date"),
            "root": "CCountry -> CProgramStatus + 0x50 embedded CBreakthroughProgress",
            "row_shape": {
                "specialization_handle_offset": "tree_node+0x1C",
                "points_raw_offset": "tree_node+0x20",
                "compact_row": "tag_db_id:u32, specialization_handle:u32, points_raw:u32, reserved:u32",
            },
            "roots": {key: hx(value) for key, value in live_summary["roots"].items()},
            "expected_vtables": {
                key: hx(value) for key, value in live_summary["expected_vtables"].items()
            },
            "accepted_for_existing_compact_row": accepted,
            "production_schema_change": False,
            "counts": {
                "status_rows": live_summary["status_rows"],
                "live_countries_with_breakthrough_rows": len(live),
                "live_rows": sum(len(row["rows"]) for row in live.values()),
                "parse_failures": len(live_summary["parse_failures"]),
                "save_countries_with_program_status": len(save["countries"]),
                "live_handles": handles,
                "handle_counts": {
                    str(handle): count
                    for handle, count in sorted(live_summary["handle_counts"].items())
                },
            },
            "handle_mapping": {
                "accepted": accepted,
                "best": best,
                "candidate_count": len(mapping_candidates),
            },
            "failure_mode": None
            if accepted
            else (
                "breakthrough live/save mismatch or fixture drift; do not expand semantics from this fixture"
            ),
            "compact_sample_rows": compact_sample_rows(live, args.compact_sample_limit),
            "parse_failure_examples": live_summary["parse_failures"][:32],
            "examples": live_summary["examples"],
        }


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Validate project breakthrough state rows against save oracle"
    )
    parser.add_argument("--pid", type=int, default=None)
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--save-projects-json", required=True)
    parser.add_argument("--compact-sample-limit", type=int, default=128)
    parser.add_argument("--out", default=None)
    args = parser.parse_args(argv)
    if args.pid is None:
        args.pid = winmem.find_pid("hoi4.exe")
    if not args.pid:
        raise SystemExit("hoi4.exe PID not found")
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
