#!/usr/bin/env python3
"""Read-only current-build root probe for focus/research/decisions depth.

This is lane-local development validation. It does not participate in hot-loop
capture and does not edit the shared collector or recorder entrypoints.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(REPO_ROOT / "extractor" / "memory_reader"))

import winmem  # noqa: E402


EXPECTED_SHA256 = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

C_GAME_STATE_GLOBAL = 0x3316E90
COUNTRY_DATABASE_GLOBAL = 0x33189C8
COUNTRY_VTABLE_RVA = 0x27D0E90
DECISION_STATUS_VTABLE_RVA = 0x27D50F0
TIMED_DECISION_VTABLE_RVA = 0x27D4FD0

CGAMESTATE_COUNTRY_TABLE = 0x310
CGAMESTATE_COUNTRY_COUNT = 0x31C
MAX_CGAMESTATE_COUNTRIES = 4096

HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000

OBJ_TAG_DB_ID = 0x08
OBJ_FOCUS_TREE = 0x1370
OBJ_FOCUS_STATUS = 0x1380
OBJ_DECISION_STATUS = 0x0FA0

FOCUS_TREE_ID = 0x08
FOCUS_STATUS_CURRENT = 0x10
FOCUS_STATUS_CONTINUOUS = 0x18
FOCUS_STATUS_PROGRESS = 0x38
FOCUS_STATUS_COMPLETED_DATA = 0x40
FOCUS_STATUS_COMPLETED_CAP = 0x48
FOCUS_STATUS_COMPLETED_SIZE = 0x4C
FOCUS_OBJECT_ID = 0x18

DECISION_STATUS_OWNER = 0x08
DECISION_STATUS_LISTS = [
    ("decisions_taken_bucket", 0x40),
    ("active_targeted_decision", 0x88),
    ("active_timed_decision", 0xA0),
    ("active_targeted_timed_decision", 0xE8),
    ("decision_to_re_enable", 0x130),
]
TIMED_DECISION_DEF = 0x10
TIMED_DECISION_DAYS = 0x18
TIMED_DECISION_STATE = 0x1C
DECISION_DEF_NAME = 0x120

MAX_FOCUS_COUNT = 4000
MAX_DECISION_STATUS_LIST = 4096
MAX_TIMED_DECISIONS = 256
MAX_DECISION_DAYS = 100000
MAX_TOKEN_LEN = 128


def hex0(value: int | None) -> str | None:
    return None if value is None else f"0x{value:X}"


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def is_heap_ptr(value: int, align: int = 8) -> bool:
    return value >= HEAP_LO and value < HEAP_HI and value % align == 0


def safe(call, default=None):
    try:
        return call()
    except OSError:
        return default
    except ValueError:
        return default


def read_i32_stable(proc: winmem.Process, addr: int) -> int | None:
    first = safe(lambda: proc.i32(addr))
    second = safe(lambda: proc.i32(addr))
    return first if first is not None and first == second else None


def read_u32_stable(proc: winmem.Process, addr: int) -> int | None:
    first = safe(lambda: proc.u32(addr))
    second = safe(lambda: proc.u32(addr))
    return first if first is not None and first == second else None


def read_msvc_string(proc: winmem.Process, addr: int, max_len: int = MAX_TOKEN_LEN) -> str | None:
    size = safe(lambda: proc.u64(addr + 0x10))
    cap = safe(lambda: proc.u64(addr + 0x18))
    if size is None or cap is None:
        return None
    if size == 0 or size > max_len or cap < size or cap > (1 << 20):
        return None
    chars_addr = addr if cap < 16 else safe(lambda: proc.u64(addr))
    if chars_addr is None:
        return None
    raw = safe(lambda: proc.read(chars_addr, size + 1))
    if raw is None or len(raw) != size + 1 or raw[-1] != 0:
        return None
    body = raw[:-1]
    if not all(32 <= byte < 127 for byte in body):
        return None
    try:
        value = body.decode("utf-8")
    except UnicodeDecodeError:
        return None
    return value if is_script_token(value) else None


def read_small_inline_string(proc: winmem.Process, addr: int) -> str | None:
    raw = safe(lambda: proc.read(addr, 32))
    if raw is None or len(raw) != 32:
        return None
    size = struct.unpack("<I", raw[16:20])[0]
    if size == 0 or size > 7:
        return None
    body = raw[:size]
    if not all(byte in b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-" for byte in body):
        return None
    try:
        value = body.decode("ascii")
    except UnicodeDecodeError:
        return None
    if len(value) != 3 or (value != "---" and not any(ch.isalpha() for ch in value)):
        return None
    return value


def country_tag_table(proc: winmem.Process, country_database: int | None) -> tuple[int | None, dict[int, str], dict]:
    meta = {
        "country_database": hex0(country_database),
        "tag_table": None,
        "tag_count": None,
        "active_count": None,
        "row_count": None,
        "prefix_ok": False,
        "valid": False,
    }
    if not country_database:
        return None, {}, meta
    tag_table = safe(lambda: proc.u64(country_database))
    tag_count = safe(lambda: proc.u32(country_database + 8))
    active_count = safe(lambda: proc.u32(country_database + 12))
    meta.update({
        "tag_table": hex0(tag_table),
        "tag_count": tag_count,
        "active_count": active_count,
    })
    if not tag_table or not tag_count or tag_count < 40 or tag_count > 1024:
        return None, {}, meta
    prefix = {0: "---", 1: "GER", 2: "ENG", 3: "SOV"}
    prefix_ok = all(read_small_inline_string(proc, tag_table + idx * 32) == tag for idx, tag in prefix.items())
    meta["prefix_ok"] = prefix_ok
    if not prefix_ok or not active_count or active_count > tag_count:
        return None, {}, meta
    row_count = min(tag_count, active_count, 1024)
    meta["row_count"] = row_count
    tags = {}
    for tag_db_id in range(row_count):
        tag = read_small_inline_string(proc, tag_table + tag_db_id * 32)
        if tag:
            tags[tag_db_id] = tag
    meta["valid"] = len(tags) >= 40
    return tag_table, tags, meta


def is_script_token(value: str) -> bool:
    return bool(value) and len(value) <= MAX_TOKEN_LEN and all(
        ch.isalnum() or ch in "_-.:"
        for ch in value
    )


def vector_header(proc: winmem.Process, header_addr: int, max_size: int = MAX_DECISION_STATUS_LIST):
    data = safe(lambda: proc.u64(header_addr))
    cap = read_u32_stable(proc, header_addr + 0x08)
    size = read_u32_stable(proc, header_addr + 0x0C)
    valid = (
        data is not None
        and cap is not None
        and size is not None
        and size <= cap
        and cap <= max_size
        and (size == 0 or is_heap_ptr(data))
    )
    return {
        "data": hex0(data),
        "size": size,
        "cap": cap,
        "valid": bool(valid),
    }


def read_focus_token(proc: winmem.Process, ptr: int, id_offset: int) -> str | None:
    if ptr == 0:
        return None
    return read_msvc_string(proc, ptr + id_offset, MAX_TOKEN_LEN)


def focus_status_probe(proc: winmem.Process, country_ptr: int) -> dict | None:
    submgr = safe(lambda: proc.u64(country_ptr + OBJ_FOCUS_STATUS))
    if not submgr or not is_heap_ptr(submgr):
        return None
    tree_ptr = safe(lambda: proc.u64(country_ptr + OBJ_FOCUS_TREE)) or 0
    current_ptr = safe(lambda: proc.u64(submgr + FOCUS_STATUS_CURRENT)) or 0
    continuous_ptr = safe(lambda: proc.u64(submgr + FOCUS_STATUS_CONTINUOUS)) or 0
    progress = read_i32_stable(proc, submgr + FOCUS_STATUS_PROGRESS)
    completed_size = read_u32_stable(proc, submgr + FOCUS_STATUS_COMPLETED_SIZE)
    completed_cap = read_u32_stable(proc, submgr + FOCUS_STATUS_COMPLETED_CAP)
    completed_data = safe(lambda: proc.u64(submgr + FOCUS_STATUS_COMPLETED_DATA)) or 0
    completed_valid = (
        completed_size is not None
        and completed_cap is not None
        and completed_size <= completed_cap
        and completed_cap <= MAX_FOCUS_COUNT
        and (completed_size == 0 or is_heap_ptr(completed_data))
    )
    candidate_window = safe(lambda: proc.read(submgr + 0x20, 0x20), b"")
    return {
        "submgr": hex0(submgr),
        "focus_tree": read_focus_token(proc, tree_ptr, FOCUS_TREE_ID),
        "current": read_focus_token(proc, current_ptr, FOCUS_OBJECT_ID),
        "current_ptr": hex0(current_ptr),
        "continuous": read_focus_token(proc, continuous_ptr, FOCUS_OBJECT_ID),
        "continuous_ptr": hex0(continuous_ptr),
        "progress_raw": progress,
        "completed_size": completed_size,
        "completed_cap": completed_cap,
        "completed_data": hex0(completed_data),
        "completed_vector_valid": bool(completed_valid),
        "pause_candidate_window_0x20_0x3f_hex": candidate_window.hex(),
    }


def decision_def_token(proc: winmem.Process, def_ptr: int) -> str | None:
    if not is_heap_ptr(def_ptr):
        return None
    return read_msvc_string(proc, def_ptr + DECISION_DEF_NAME, MAX_TOKEN_LEN)


def module_rva(module_base: int, ptr: int) -> str | None:
    if module_base <= ptr < module_base + 0x4000000:
        return f"0x{ptr - module_base:X}"
    return None


def inspect_decision_record(
    proc: winmem.Process,
    module_base: int,
    rec_ptr: int,
    country_ptr_to_tag: dict[int, int],
    tag_by_id: dict[int, str],
) -> dict:
    out = {
        "record_ptr": hex0(rec_ptr),
        "heap_ptr": is_heap_ptr(rec_ptr),
        "record_vtable": None,
        "record_vtable_rva": None,
        "def_ptr": None,
        "decision": None,
        "days": None,
        "state": None,
        "timed_body_contract": False,
        "target_country_pointer_candidates": [],
        "tag_id_field_candidates": [],
        "small_i32_candidates_0x20_0x7c": [],
        "raw_0x00_0x7f_hex": None,
    }
    if not is_heap_ptr(rec_ptr):
        return out
    rec_vtable = safe(lambda: proc.u64(rec_ptr))
    out["record_vtable"] = hex0(rec_vtable)
    out["record_vtable_rva"] = module_rva(module_base, rec_vtable or 0)
    raw = safe(lambda: proc.read(rec_ptr, 0x80), b"")
    out["raw_0x00_0x7f_hex"] = raw.hex()
    def_ptr = safe(lambda: proc.u64(rec_ptr + TIMED_DECISION_DEF)) or 0
    out["def_ptr"] = hex0(def_ptr)
    out["decision"] = decision_def_token(proc, def_ptr)
    out["days"] = read_i32_stable(proc, rec_ptr + TIMED_DECISION_DAYS)
    out["state"] = read_i32_stable(proc, rec_ptr + TIMED_DECISION_STATE)
    out["timed_body_contract"] = bool(
        rec_vtable == module_base + TIMED_DECISION_VTABLE_RVA
        and out["decision"]
        and out["days"] is not None
        and 0 <= out["days"] <= MAX_DECISION_DAYS
        and out["state"] is not None
        and 0 <= out["state"] <= 16
    )
    for offset in range(0x20, 0x80, 8):
        value = safe(lambda off=offset: proc.u64(rec_ptr + off))
        if value in country_ptr_to_tag:
            out["target_country_pointer_candidates"].append({
                "offset": f"0x{offset:X}",
                "tag_db_id": country_ptr_to_tag[value],
                "country_ptr": hex0(value),
            })
    for offset in range(0x20, 0x80, 4):
        value = read_i32_stable(proc, rec_ptr + offset)
        if value is not None and -1 <= value <= 100000:
            out["small_i32_candidates_0x20_0x7c"].append({
                "offset": f"0x{offset:X}",
                "value": value,
            })
            if value and value in tag_by_id:
                out["tag_id_field_candidates"].append({
                    "offset": f"0x{offset:X}",
                    "tag_db_id": value,
                    "tag": tag_by_id[value],
                })
    return out


def country_rows(
    proc: winmem.Process,
    module_base: int,
    game_state: int,
    tag_by_id: dict[int, str],
    tag_row_count: int | None,
) -> tuple[list[dict], dict]:
    country_table = safe(lambda: proc.u64(game_state + CGAMESTATE_COUNTRY_TABLE))
    country_count = read_u32_stable(proc, game_state + CGAMESTATE_COUNTRY_COUNT)
    meta = {
        "country_table": hex0(country_table),
        "country_count": country_count,
        "valid_table": bool(
            country_table and is_heap_ptr(country_table) and country_count and country_count <= MAX_CGAMESTATE_COUNTRIES
        ),
    }
    if not meta["valid_table"]:
        return [], meta
    country_vtable = module_base + COUNTRY_VTABLE_RVA
    rows = []
    if tag_row_count and tag_row_count > 1:
        indices = range(1, min(tag_row_count, MAX_CGAMESTATE_COUNTRIES))
    else:
        indices = range(1, country_count)
    for index in indices:
        ptr = safe(lambda idx=index: proc.u64(country_table + idx * 8))
        if not ptr or not is_heap_ptr(ptr):
            continue
        vtable = safe(lambda p=ptr: proc.u64(p))
        if vtable != country_vtable:
            continue
        tag_db_id = read_u32_stable(proc, ptr + OBJ_TAG_DB_ID)
        rows.append({
            "index": index,
            "tag_db_id": tag_db_id,
            "tag": tag_by_id.get(index),
            "object_ptr": ptr,
            "tag_matches_index": tag_db_id == index,
        })
    return rows, meta


def summarize_focus(rows: list[dict], focus_samples: list[dict]) -> dict:
    active = [
        sample for sample in focus_samples
        if sample.get("current") or sample.get("continuous") or (sample.get("progress_raw") or 0) > 0
    ]
    byte_values = {f"0x{offset:X}": Counter() for offset in range(0x20, 0x38)}
    for sample in focus_samples:
        raw = bytes.fromhex(sample["focus"]["pause_candidate_window_0x20_0x3f_hex"])
        for rel, byte in enumerate(raw[:0x18]):
            byte_values[f"0x{0x20 + rel:X}"][byte] += 1
    nonzero_offsets = []
    for offset, counter in byte_values.items():
        nonzero = sum(count for value, count in counter.items() if value != 0)
        if nonzero:
            nonzero_offsets.append({"offset": offset, "nonzero_rows": nonzero, "values": dict(counter)})
    return {
        "country_rows": len(rows),
        "focus_status_valid_rows": len(focus_samples),
        "active_or_progress_rows": len(active),
        "national_current_rows": sum(1 for sample in focus_samples if sample.get("current")),
        "continuous_current_rows": sum(1 for sample in focus_samples if sample.get("continuous")),
        "completed_vector_valid_rows": sum(1 for sample in focus_samples if sample["focus"].get("completed_vector_valid")),
        "pause_candidate_window": "CFocusStatus+0x20..+0x37 sampled; +0x38 is known progress",
        "pause_candidate_nonzero_offsets_0x20_0x37": nonzero_offsets,
        "paused_flag_verdict": (
            "not_validated_no_paused_true_ground_truth"
            if focus_samples
            else "not_validated_no_focus_status_rows"
        ),
    }


def summarize_decisions(decision_list_samples: list[dict], status_rows: int) -> dict:
    per_list = {}
    for row in decision_list_samples:
        summary = per_list.setdefault(row["list_name"], {
            "offset": row["offset"],
            "countries_with_nonzero": 0,
            "total_entries": 0,
            "records_sampled": 0,
            "def_name_valid_records": 0,
            "timed_body_contract_records": 0,
            "target_country_pointer_candidate_records": 0,
            "tag_id_field_candidate_records": 0,
            "tag_id_field_candidate_offsets": Counter(),
            "record_vtable_rvas": Counter(),
        })
        summary["countries_with_nonzero"] += 1
        summary["total_entries"] += row["size"]
        summary["records_sampled"] += len(row["records"])
        for record in row["records"]:
            if record.get("decision"):
                summary["def_name_valid_records"] += 1
            if record.get("timed_body_contract"):
                summary["timed_body_contract_records"] += 1
            if record.get("target_country_pointer_candidates"):
                summary["target_country_pointer_candidate_records"] += 1
            if record.get("tag_id_field_candidates"):
                summary["tag_id_field_candidate_records"] += 1
                for candidate in record["tag_id_field_candidates"]:
                    summary["tag_id_field_candidate_offsets"][candidate["offset"]] += 1
            rva = record.get("record_vtable_rva") or record.get("record_vtable") or "none"
            summary["record_vtable_rvas"][rva] += 1
    for summary in per_list.values():
        summary["record_vtable_rvas"] = dict(summary["record_vtable_rvas"])
        summary["tag_id_field_candidate_offsets"] = dict(summary["tag_id_field_candidate_offsets"])
        if summary["records_sampled"] == 0:
            summary["body_verdict"] = "not_observed_nonzero_count_only"
        elif summary["timed_body_contract_records"] == summary["records_sampled"]:
            summary["body_verdict"] = "validated_timed_decision_body_contract"
        elif summary["timed_body_contract_records"] > 0:
            summary["body_verdict"] = "partially_validated_timed_body_some_rows_failed_gate"
        elif summary["def_name_valid_records"] == summary["records_sampled"]:
            summary["body_verdict"] = "validated_definition_pointer_target_details_unproven"
        elif summary["def_name_valid_records"] == 0:
            summary["body_verdict"] = "rejected_timed_definition_layout"
        else:
            summary["body_verdict"] = "not_validated_raw_candidates_only"
    return {
        "decision_status_valid_rows": status_rows,
        "lists": per_list,
    }


def run_probe(out_dir: Path, sample_limit_per_list: int) -> int:
    out_dir.mkdir(parents=True, exist_ok=True)
    proc = winmem.open_process("hoi4.exe", "hoi4.exe")
    if proc is None:
        manifest = {
            "lane": "focus_research_decisions_depth_live",
            "created_utc": utc_now(),
            "ok": False,
            "error": "hoi4.exe_not_running",
        }
        (out_dir / "root_probe_manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
        return 2

    with proc:
        fp = proc.fingerprint()
        module_base = proc.module_base
        game_state = safe(lambda: proc.u64(module_base + C_GAME_STATE_GLOBAL))
        country_database = safe(lambda: proc.u64(module_base + COUNTRY_DATABASE_GLOBAL))
        tag_table, tag_by_id, tag_meta = country_tag_table(proc, country_database)
        sha_ok = (fp.get("sha256") or "").lower() == EXPECTED_SHA256
        build_gate = {
            "expected_sha256": EXPECTED_SHA256,
            "actual_sha256": fp.get("sha256"),
            "ok": sha_ok,
        }
        countries, table_meta = country_rows(
            proc,
            module_base,
            game_state or 0,
            tag_by_id,
            tag_meta.get("row_count"),
        )
        country_ptr_to_tag = {
            row["object_ptr"]: row["tag_db_id"]
            for row in countries
            if row["tag_db_id"] is not None
        }

        focus_samples = []
        for row in countries:
            focus = focus_status_probe(proc, row["object_ptr"])
            if focus is None:
                continue
            focus_samples.append({
                "index": row["index"],
                "tag_db_id": row["tag_db_id"],
                "object_ptr": hex0(row["object_ptr"]),
                "current": focus["current"],
                "continuous": focus["continuous"],
                "progress_raw": focus["progress_raw"],
                "focus": focus,
            })

        decision_status_rows = 0
        decision_list_samples = []
        status_vtable = module_base + DECISION_STATUS_VTABLE_RVA
        timed_vtable = module_base + TIMED_DECISION_VTABLE_RVA
        for row in countries:
            object_ptr = row["object_ptr"]
            status_ptr = safe(lambda p=object_ptr: proc.u64(p + OBJ_DECISION_STATUS))
            if not status_ptr or not is_heap_ptr(status_ptr):
                continue
            if safe(lambda p=status_ptr: proc.u64(p)) != status_vtable:
                continue
            if safe(lambda p=status_ptr: proc.u64(p + DECISION_STATUS_OWNER)) != object_ptr:
                continue
            decision_status_rows += 1
            for name, offset in DECISION_STATUS_LISTS:
                header = vector_header(proc, status_ptr + offset)
                if not header["valid"] or not header["size"]:
                    continue
                records = []
                data = int(header["data"], 16)
                max_records = min(header["size"], sample_limit_per_list)
                if name == "active_timed_decision":
                    max_records = min(max_records, MAX_TIMED_DECISIONS)
                for index in range(max_records):
                    rec_ptr = safe(lambda i=index: proc.u64(data + i * 8))
                    if rec_ptr:
                        records.append(inspect_decision_record(
                            proc,
                            module_base,
                            rec_ptr,
                            country_ptr_to_tag,
                            tag_by_id,
                        ))
                decision_list_samples.append({
                    "index": row["index"],
                    "tag_db_id": row["tag_db_id"],
                    "object_ptr": hex0(object_ptr),
                    "status_ptr": hex0(status_ptr),
                    "list_name": name,
                    "offset": f"0x{offset:X}",
                    "data": header["data"],
                    "size": header["size"],
                    "cap": header["cap"],
                    "records": records,
                })

        manifest = {
            "lane": "focus_research_decisions_depth_live",
            "created_utc": utc_now(),
            "ok": bool(sha_ok and table_meta["valid_table"] and countries),
            "process": {
                "pid": fp.get("pid"),
                "module": fp.get("module"),
                "base": hex0(module_base),
                "size": fp.get("size"),
                "path": fp.get("path"),
            },
            "build_gate": build_gate,
            "roots": {
                "CGameState_global": hex0(module_base + C_GAME_STATE_GLOBAL),
                "CGameState": hex0(game_state),
                "country_database_global": hex0(module_base + COUNTRY_DATABASE_GLOBAL),
                "country_database": hex0(country_database),
                "country_table": table_meta["country_table"],
                "country_count": table_meta["country_count"],
                "country_vtable": hex0(module_base + COUNTRY_VTABLE_RVA),
                "decision_status_vtable": hex0(status_vtable),
                "timed_decision_vtable": hex0(timed_vtable),
                "country_tag_table": hex0(tag_table),
            },
            "tag_table": tag_meta,
            "country_scan": {
                "valid_country_rows": len(countries),
                "tag_index_match_rows": sum(1 for row in countries if row["tag_matches_index"]),
                "tag_index_mismatch_rows": sum(1 for row in countries if not row["tag_matches_index"]),
                "valid_country_table": table_meta["valid_table"],
            },
            "focus": summarize_focus(countries, focus_samples),
            "decisions": summarize_decisions(decision_list_samples, decision_status_rows),
            "verdicts": {
                "focus_paused_flag": "not_validated_badge_fail_closed",
                "decision_targeted_cooldown_bodies": "see_per_list_body_verdicts",
                "direct_transition_event_markers": "no_engine_event_function_validated_use_deterministic_compact_derivation",
            },
        }

        (out_dir / "root_probe_manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
        with (out_dir / "focus_pause_samples.jsonl").open("w", encoding="utf-8") as fh:
            for sample in focus_samples[:100]:
                fh.write(json.dumps(sample, sort_keys=True) + "\n")
        with (out_dir / "decision_body_samples.jsonl").open("w", encoding="utf-8") as fh:
            for sample in decision_list_samples:
                fh.write(json.dumps(sample, sort_keys=True) + "\n")

    print(json.dumps({
        "ok": manifest["ok"],
        "valid_country_rows": manifest["country_scan"]["valid_country_rows"],
        "focus_status_valid_rows": manifest["focus"]["focus_status_valid_rows"],
        "focus_paused_flag": manifest["verdicts"]["focus_paused_flag"],
        "decision_lists": {
            name: {
                "total_entries": data["total_entries"],
                "records_sampled": data["records_sampled"],
                "body_verdict": data["body_verdict"],
            }
            for name, data in manifest["decisions"]["lists"].items()
        },
        "out_dir": str(out_dir),
    }, indent=2))
    return 0 if manifest["ok"] else 1


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--sample-limit-per-list", type=int, default=12)
    args = parser.parse_args()
    return run_probe(args.out_dir, max(1, args.sample_limit_per_list))


if __name__ == "__main__":
    raise SystemExit(main())
