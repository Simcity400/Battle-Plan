"""Read-only exact-a729 A23 CProgram owner-equality admission probe.

The production d65 reader deliberately drops a CProgram reached from more than
one CCountry program-status record.  This probe is the bounded observation that
can retire that fallback: it walks only the validated CCountry+0xFA8 route,
reads one coherent CProgram window, and compares the duplicated owner/province
fields with an exact-hour projects.json save oracle.  It never scans the heap,
writes game memory, or infers an owner from traversal reachability.
"""

import argparse
import hashlib
import json
import os
import struct
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[4]
MEMORY_READER = REPO_ROOT / "extractor" / "memory_reader"
sys.path.insert(0, str(MEMORY_READER))

import projects_progress_probe as pp  # noqa: E402
import winmem  # noqa: E402


PROGRAM_STATUS_RECORD_STRIDE = 0x160
MAX_PROGRAM_STATUS_RECORDS = 3
PROGRAM_VECTOR_BEGIN_OFF = 0x20
MAX_PROGRAMS_PER_RECORD = 4096
PROGRAM_WINDOW_LEN = 0x180
GAME_STATE_CURRENT_DATE_OFF = 0x468
DATE_EPOCH_CONST = 43_799_999
MONTH_LENGTHS = (31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31)

FIELD_OFFSETS = {
    "program_id": 0x0C,
    "control_0x10": 0x10,
    "control_0x14": 0x14,
    "owner_primary": 0x18,
    "province_primary": 0x1C,
    "control_0x20": 0x20,
    "control_0x168": 0x168,
    "control_0x16c": 0x16C,
    "owner_mirror": 0x170,
    "province_mirror": 0x174,
    "unread_prototype_rewards": 0x178,
    "control_0x17c": 0x17C,
}
NEGATIVE_CONTROL_FIELDS = tuple(
    name for name in FIELD_OFFSETS if name.startswith("control_")
)
REQUIRED_D04_MULTIPLY_REACHABLE_PROGRAM_IDS = frozenset((23, 24, 25, 27, 28))


def hx(value):
    return pp.hx(value)


def u32_at(blob, offset):
    return struct.unpack_from("<I", blob, offset)[0]


def encode_save_date(date_text):
    if not isinstance(date_text, str):
        return None
    try:
        year, month, day, hour = (int(part) for part in date_text.split("."))
    except (TypeError, ValueError):
        return None
    if (
        year < 0
        or month < 1
        or month > len(MONTH_LENGTHS)
        or day < 1
        or day > MONTH_LENGTHS[month - 1]
        or hour < 0
        or hour > 23
    ):
        return None
    day_of_year = sum(MONTH_LENGTHS[: month - 1]) + day - 1
    return ((year * 365 + day_of_year) * 24 + hour) + DATE_EPOCH_CONST


def load_save_facilities(path):
    with open(path, "r", encoding="utf-8") as fh:
        data = json.load(fh)
    rows = []
    duplicate_program_ids = []
    by_program_id = {}
    for tag, country in sorted((data.get("countries") or {}).items()):
        for facility in country.get("facilities") or []:
            row = {
                "country": tag,
                "program_id": int(facility.get("id") or 0),
                "province": int(facility.get("province") or 0),
                "building": facility.get("building"),
                "unread_prototype_rewards": int(
                    facility.get("unread_prototype_rewards") or 0
                ),
            }
            rows.append(row)
            prior = by_program_id.get(row["program_id"])
            if prior is not None:
                duplicate_program_ids.append(row["program_id"])
            else:
                by_program_id[row["program_id"]] = row
    meta = data.get("meta") or {}
    save_date = meta.get("save_date") or meta.get("date")
    return {
        "path": str(path),
        "meta": meta,
        "save_date": save_date,
        "game_hour_raw": encode_save_date(save_date),
        "rows": rows,
        "by_program_id": by_program_id,
        "duplicate_program_ids": sorted(set(duplicate_program_ids)),
    }


def collect_live_programs(p):
    module_base = p.module_base
    country_vtable = module_base + pp.COUNTRY_VTABLE_RVA
    status_primary_vtable = module_base + pp.PROJECT_PROGRAM_STATUS_A_VTABLE_RVA
    status_secondary_vtable = module_base + pp.PROJECT_PROGRAM_STATUS_B_VTABLE_RVA
    program_vtable = module_base + pp.PROJECT_FACILITY_VTABLE_RVA

    game_state = pp.try_u64(p, module_base + pp.CG_STATE_DATE_OFFSET)
    country_database = pp.try_u64(p, module_base + pp.COUNTRY_DATABASE_OFFSET)
    if not pp.is_heap_ptr(game_state) or not pp.is_heap_ptr(country_database):
        raise RuntimeError("failed to resolve CGameState/country database")
    game_hour_raw = pp.try_u32(p, game_state + GAME_STATE_CURRENT_DATE_OFF)
    country_table = pp.try_u64(p, game_state + pp.CGAMESTATE_COUNTRY_TABLE)
    tag_table = pp.try_u64(p, country_database)
    tag_row_count = pp.tag_row_count(p, country_database)
    if (
        game_hour_raw is None
        or not tag_row_count
        or not pp.is_heap_ptr(country_table)
        or not pp.is_heap_ptr(tag_table)
    ):
        raise RuntimeError("failed to validate date/country/tag roots")

    tag_by_id = {}
    country_rows = []
    programs_by_ptr = {}
    invalid_records = []
    invalid_programs = []
    for tag_db_id in range(1, tag_row_count):
        country_ptr = pp.try_u64(p, country_table + tag_db_id * 8)
        if not country_ptr:
            continue
        if (
            not pp.is_heap_ptr(country_ptr)
            or pp.try_u64(p, country_ptr) != country_vtable
            or pp.try_u32(p, country_ptr + pp.OBJ_TAG_DB_ID) != tag_db_id
        ):
            continue
        tag = pp.read_small_inline_string(p, tag_table + tag_db_id * 32)
        if not tag:
            continue
        tag_by_id[tag_db_id] = tag
        status_ptr = pp.try_u64(p, country_ptr + pp.COUNTRY_OFF_PROGRAM_STATUS)
        if not status_ptr:
            continue
        if (
            not pp.is_heap_ptr(status_ptr)
            or pp.try_u64(p, status_ptr) != status_primary_vtable
            or pp.try_u64(p, status_ptr + 8) != status_secondary_vtable
        ):
            invalid_records.append({"country": tag, "reason": "invalid_status_root"})
            continue
        country_rows.append({"tag": tag, "tag_db_id": tag_db_id})
        for record_index in range(MAX_PROGRAM_STATUS_RECORDS):
            record_ptr = status_ptr + record_index * PROGRAM_STATUS_RECORD_STRIDE
            record_vtable = pp.try_u64(p, record_ptr)
            if record_vtable != status_primary_vtable:
                invalid_records.append(
                    {
                        "country": tag,
                        "record_index": record_index,
                        "reason": "invalid_program_status_record_vtable",
                    }
                )
                break
            header = p.try_read(record_ptr + PROGRAM_VECTOR_BEGIN_OFF, 16)
            if len(header) != 16:
                invalid_records.append(
                    {"country": tag, "record_index": record_index, "reason": "torn_header"}
                )
                continue
            begin, packed = struct.unpack_from("<QQ", header, 0)
            count = packed & 0xFFFFFFFF
            capacity = packed >> 32
            if count == 0:
                continue
            if (
                count > capacity
                or count > MAX_PROGRAMS_PER_RECORD
                or not pp.is_heap_ptr(begin)
            ):
                invalid_records.append(
                    {
                        "country": tag,
                        "record_index": record_index,
                        "reason": "invalid_vector_shape",
                        "count": count,
                        "capacity": capacity,
                    }
                )
                continue
            for slot_index in range(count):
                program_ptr = pp.try_u64(p, begin + slot_index * 8)
                if not program_ptr or not pp.is_heap_ptr(program_ptr):
                    invalid_programs.append(
                        {
                            "country": tag,
                            "record_index": record_index,
                            "slot_index": slot_index,
                            "reason": "invalid_program_pointer",
                        }
                    )
                    break
                blob = p.try_read(program_ptr, PROGRAM_WINDOW_LEN)
                if len(blob) != PROGRAM_WINDOW_LEN or struct.unpack_from("<Q", blob, 0)[0] != program_vtable:
                    invalid_programs.append(
                        {
                            "country": tag,
                            "record_index": record_index,
                            "slot_index": slot_index,
                            "reason": "invalid_or_torn_program_window",
                        }
                    )
                    break
                fields = {name: u32_at(blob, off) for name, off in FIELD_OFFSETS.items()}
                row = programs_by_ptr.get(program_ptr)
                if row is None:
                    row = {
                        "program_ptr": program_ptr,
                        "window_sha256": hashlib.sha256(blob).hexdigest(),
                        "fields": fields,
                        "traversal_owner_tags": set(),
                        "traversal_records": set(),
                    }
                    programs_by_ptr[program_ptr] = row
                elif row["fields"] != fields or row["window_sha256"] != hashlib.sha256(blob).hexdigest():
                    invalid_programs.append(
                        {
                            "country": tag,
                            "record_index": record_index,
                            "slot_index": slot_index,
                            "reason": "program_changed_between_alias_reads",
                        }
                    )
                    continue
                row["traversal_owner_tags"].add(tag)
                row["traversal_records"].add((tag, record_index))

    programs = []
    for ptr, row in programs_by_ptr.items():
        fields = row["fields"]
        owner_tag = tag_by_id.get(fields["owner_mirror"])
        programs.append(
            {
                "program_ptr": ptr,
                "window_sha256": row["window_sha256"],
                "fields": fields,
                "resolved_owner_tag": owner_tag,
                "traversal_owner_tags": sorted(row["traversal_owner_tags"]),
                "traversal_records": [
                    {"country": tag, "record_index": index}
                    for tag, index in sorted(row["traversal_records"])
                ],
            }
        )
    programs.sort(
        key=lambda row: (
            row["fields"]["program_id"],
            row["fields"]["province_mirror"],
            row["program_ptr"],
        )
    )
    game_hour_raw_end = pp.try_u32(p, game_state + GAME_STATE_CURRENT_DATE_OFF)
    return {
        "game_hour_raw": game_hour_raw,
        "game_hour_raw_end": game_hour_raw_end,
        "tag_row_count": tag_row_count,
        "country_rows": country_rows,
        "tag_by_id": tag_by_id,
        "programs": programs,
        "invalid_records": invalid_records,
        "invalid_programs": invalid_programs,
        "expected_vtable_rvas": {
            "CCountry": pp.COUNTRY_VTABLE_RVA,
            "CProgramStatus_primary": pp.PROJECT_PROGRAM_STATUS_A_VTABLE_RVA,
            "CProgramStatus_secondary": pp.PROJECT_PROGRAM_STATUS_B_VTABLE_RVA,
            "CProgram": pp.PROJECT_FACILITY_VTABLE_RVA,
        },
    }


def evaluate_owner_gate(live, save, expected_game_hour_raw):
    problems = []
    if save["game_hour_raw"] is None:
        problems.append("save oracle has no valid save_date raw-hour anchor")
    elif save["game_hour_raw"] != expected_game_hour_raw:
        problems.append(
            "save-derived raw hour %s != explicitly expected raw hour %s"
            % (save["game_hour_raw"], expected_game_hour_raw)
        )
    if live["game_hour_raw_end"] != live["game_hour_raw"]:
        problems.append("live raw hour changed during the CProgram census")
    if (
        live["game_hour_raw"] != expected_game_hour_raw
        or live["game_hour_raw_end"] != expected_game_hour_raw
    ):
        problems.append(
            "live raw-hour bracket %s..%s != expected save raw hour %s"
            % (
                live["game_hour_raw"],
                live["game_hour_raw_end"],
                expected_game_hour_raw,
            )
        )
    if save["duplicate_program_ids"]:
        problems.append("save program ids are not unique")
    if len(save["rows"]) != 26:
        problems.append("save facility count is %d, expected 26" % len(save["rows"]))
    if len(live["programs"]) != 26:
        problems.append("live distinct CProgram count is %d, expected 26" % len(live["programs"]))
    if live["invalid_records"] or live["invalid_programs"]:
        problems.append("live walk contains invalid/torn records")

    compared = []
    seen_program_ids = set()
    missing_save_ids = []
    primary_mirror_mismatches = []
    owner_mismatches = []
    province_mismatches = []
    payload_mismatches = []
    unresolved_owner_handles = []
    ambiguous_owner_mismatches = []
    multiply_reachable_program_ids = set()
    control_match_counts = {name: 0 for name in NEGATIVE_CONTROL_FIELDS}
    for live_row in live["programs"]:
        fields = live_row["fields"]
        program_id = fields["program_id"]
        if program_id in seen_program_ids:
            problems.append("live program ids are not unique")
        seen_program_ids.add(program_id)
        oracle = save["by_program_id"].get(program_id)
        if oracle is None:
            missing_save_ids.append(program_id)
            continue
        owner_equal = fields["owner_primary"] == fields["owner_mirror"]
        province_equal = fields["province_primary"] == fields["province_mirror"]
        if not owner_equal or not province_equal:
            primary_mirror_mismatches.append(program_id)
        if live_row["resolved_owner_tag"] is None:
            unresolved_owner_handles.append(program_id)
        if live_row["resolved_owner_tag"] != oracle["country"]:
            owner_mismatches.append(program_id)
        if fields["province_mirror"] != oracle["province"]:
            province_mismatches.append(program_id)
        if fields["unread_prototype_rewards"] != oracle["unread_prototype_rewards"]:
            payload_mismatches.append(program_id)
        for name in NEGATIVE_CONTROL_FIELDS:
            if fields[name] == fields["owner_mirror"]:
                control_match_counts[name] += 1
        ambiguous = len(live_row["traversal_owner_tags"]) > 1
        if ambiguous:
            multiply_reachable_program_ids.add(program_id)
        if ambiguous and live_row["resolved_owner_tag"] != oracle["country"]:
            ambiguous_owner_mismatches.append(program_id)
        compared.append(
            {
                "program_id": program_id,
                "oracle_country": oracle["country"],
                "owner_tag_db_id": fields["owner_mirror"],
                "resolved_owner_tag": live_row["resolved_owner_tag"],
                "province": fields["province_mirror"],
                "unread_prototype_rewards": fields["unread_prototype_rewards"],
                "owner_fields_equal": owner_equal,
                "province_fields_equal": province_equal,
                "traversal_owner_tags": live_row["traversal_owner_tags"],
                "multiply_reachable": ambiguous,
                "negative_controls": {
                    name: {"matches_owner": fields[name] == fields["owner_mirror"]}
                    for name in NEGATIVE_CONTROL_FIELDS
                },
            }
        )

    save_ids = set(save["by_program_id"])
    extra_save_ids = sorted(save_ids - seen_program_ids)
    if missing_save_ids or extra_save_ids:
        problems.append("live/save program-id sets differ")
    if primary_mirror_mismatches:
        problems.append("duplicated owner/province fields disagree")
    if owner_mismatches or unresolved_owner_handles:
        problems.append("owner handle does not resolve to exact serialized owner")
    if province_mismatches or payload_mismatches:
        problems.append("live program payload does not match exact serialized row")
    if ambiguous_owner_mismatches:
        problems.append("multiply-reachable program did not select serialized owner")
    missing_required_ambiguous = sorted(
        REQUIRED_D04_MULTIPLY_REACHABLE_PROGRAM_IDS - multiply_reachable_program_ids
    )
    unexpected_ambiguous = sorted(
        multiply_reachable_program_ids - REQUIRED_D04_MULTIPLY_REACHABLE_PROGRAM_IDS
    )
    if missing_required_ambiguous or unexpected_ambiguous:
        problems.append("live alias program-id set is not the exact retained D04 discriminator")
    control_failures = [
        name for name, count in control_match_counts.items() if count == len(compared) and compared
    ]
    if control_failures:
        problems.append("adjacent negative control aliases the owner field for every row")

    accepted = not problems and len(compared) == 26
    return {
        "accepted": accepted,
        "problems": problems,
        "counts": {
            "save_facilities": len(save["rows"]),
            "live_distinct_programs": len(live["programs"]),
            "compared_rows": len(compared),
            "multiply_reachable_rows": sum(
                1 for row in compared if row["multiply_reachable"]
            ),
            "owner_primary_mirror_mismatches": len(primary_mirror_mismatches),
            "owner_save_mismatches": len(owner_mismatches),
            "province_save_mismatches": len(province_mismatches),
            "payload_save_mismatches": len(payload_mismatches),
            "unresolved_owner_handles": len(unresolved_owner_handles),
            "invalid_records": len(live["invalid_records"]),
            "invalid_programs": len(live["invalid_programs"]),
        },
        "negative_control_owner_match_counts": control_match_counts,
        "missing_live_save_program_ids": sorted(missing_save_ids),
        "missing_save_live_program_ids": extra_save_ids,
        "primary_mirror_mismatch_program_ids": sorted(primary_mirror_mismatches),
        "owner_mismatch_program_ids": sorted(owner_mismatches),
        "province_mismatch_program_ids": sorted(province_mismatches),
        "payload_mismatch_program_ids": sorted(payload_mismatches),
        "unresolved_owner_program_ids": sorted(unresolved_owner_handles),
        "ambiguous_owner_mismatch_program_ids": sorted(ambiguous_owner_mismatches),
        "required_multiply_reachable_program_ids": sorted(
            REQUIRED_D04_MULTIPLY_REACHABLE_PROGRAM_IDS
        ),
        "observed_multiply_reachable_program_ids": sorted(
            multiply_reachable_program_ids
        ),
        "missing_required_multiply_reachable_program_ids": missing_required_ambiguous,
        "unexpected_multiply_reachable_program_ids": unexpected_ambiguous,
        "rows": compared,
    }


def probe(args):
    save = load_save_facilities(args.save_projects_json)
    with winmem.Process(args.pid, "hoi4.exe") as p:
        fp = p.fingerprint()
        sha = str(fp.get("sha256") or "").lower()
        if sha != pp.EXPECTED_SHA256 and not args.force:
            raise SystemExit(
                "live hoi4.exe sha256 %s != expected %s; pass --force to probe anyway"
                % (sha, pp.EXPECTED_SHA256)
            )
        live = collect_live_programs(p)
        confirmation = collect_live_programs(p)
        gate = evaluate_owner_gate(live, save, args.expected_game_hour_raw)
        double_walk_equal = live == confirmation
        if not double_walk_equal:
            gate["accepted"] = False
            gate["problems"].append(
                "two complete rooted observations at the boundary were not identical"
            )
        if sha != pp.EXPECTED_SHA256:
            # --force is diagnostic only.  It must never turn an unsupported
            # executable into production evidence.
            gate["accepted"] = False
            gate["problems"].append("live executable is not exact supported a729")
        pointer_free_rows = gate.pop("rows")
        return {
            "schema": "a23-program-owner-equality-probe-v1",
            "utc": datetime.now(timezone.utc).isoformat(),
            "supported_binary": {
                "binary_id": "hoi4.exe 1.19.2.0.a729",
                "image_base": "0x140000000",
                "sha256": pp.EXPECTED_SHA256,
            },
            "process": {
                "pid": args.pid,
                "module_path": fp.get("path"),
                "observed_sha256": sha,
            },
            "boundary": {
                "live_game_hour_raw": live["game_hour_raw"],
                "live_game_hour_raw_end": live["game_hour_raw_end"],
                "expected_save_game_hour_raw": args.expected_game_hour_raw,
                "save_date": save["save_date"],
                "save_derived_game_hour_raw": save["game_hour_raw"],
                "same_raw_hour": live["game_hour_raw"]
                == live["game_hour_raw_end"]
                == args.expected_game_hour_raw
                == save["game_hour_raw"],
                "double_rooted_walk_equal": double_walk_equal,
            },
            "root": "CCountry+0xFA8 -> contiguous CProgramStatus[0..3) -> +0x20 CProgram pointer vector",
            "field_contract": {
                "owner_primary": "CProgram+0x18 u32",
                "owner_mirror": "CProgram+0x170 u32",
                "province_primary": "CProgram+0x1C u32",
                "province_mirror": "CProgram+0x174 u32",
                "window": "CProgram+[0x00,0x180) one ReadProcessMemory call per observation",
                "negative_controls": list(NEGATIVE_CONTROL_FIELDS),
            },
            "save_projects_json": save["path"],
            "expected_vtable_rvas": {
                name: hx(value) for name, value in live["expected_vtable_rvas"].items()
            },
            "walk": {
                "tag_row_count": live["tag_row_count"],
                "validated_country_rows": len(live["country_rows"]),
                "invalid_records": live["invalid_records"][:64],
                "invalid_programs": live["invalid_programs"][:64],
            },
            "owner_gate": gate,
            "pointer_free_rows": pointer_free_rows,
            "production_admission": gate["accepted"],
            "on_failure": "retain d65 PARTIAL_DATA and drop multiply-reachable rows",
        }


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Validate exact-a729 CProgram owner fields against an exact-hour save"
    )
    parser.add_argument("--pid", type=int, default=None)
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--save-projects-json", required=True)
    parser.add_argument("--expected-game-hour-raw", required=True, type=int)
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
