#!/usr/bin/env python3
"""Operative lifecycle-group root probe for domain 53.

The accepted active-operative slice uses the identity object around
``COperativeLeader`` records, but full lifecycle replay still needs non-active
operative groups such as recruitable/pool/captured states. This probe validates
the current-build ``COperativeLeader`` identity root and groups every live row
by the already observed ``identity+0x110`` lifecycle flag.

It does not name the non-active flags as recruitable or captured without a
same-tick UI/save fixture. It only proves the current root/path/offsets and
warm-stable cohorts.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


LANE_DIR = Path(__file__).resolve().parent
if str(LANE_DIR) not in sys.path:
    sys.path.insert(0, str(LANE_DIR))

import probe_870ebf_intel as intel  # type: ignore


SCAN_CHUNK = 64 << 20
OVERLAP = 16
MAX_GROUP_SAMPLES = 12


def load_json(path: Path | None) -> dict[str, Any]:
    if path is None:
        return {}
    with path.open("r", encoding="utf-8") as f:
        value = json.load(f)
    if not isinstance(value, dict):
        raise ValueError(f"{path}: expected JSON object")
    return value


def parse_hex(value: Any) -> int | None:
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        try:
            return int(value, 16)
        except ValueError:
            return None
    return None


def group_key(classdesc: int, lifeflag: int | None, owner_valid: bool) -> str:
    return f"{classdesc:016X}:{lifeflag}:{int(owner_valid)}"


def read_identity_row(
    p: Any,
    ident: int,
    tags: dict[int, str],
    operative_vtables: set[int],
) -> dict[str, Any] | None:
    classdesc = intel.u64(p, ident + intel.IDENT_CLASSDESC)
    if classdesc not in operative_vtables:
        return None
    handle_type = intel.i32(p, ident)
    if handle_type != intel.OP_TYPE_ID:
        return None
    op_id = intel.i32(p, ident + 4)
    if op_id is None or op_id <= 0:
        return None
    name = intel.read_msvc_string_utf8(p, ident + intel.IDENT_NAME, maxlen=64)
    name2 = intel.read_msvc_string_utf8(p, ident + intel.IDENT_NAME2, maxlen=64)
    if not name or name2 != name:
        return None
    owner_id = intel.i32(p, ident + intel.IDENT_OWNER)
    owner_valid = owner_id in tags
    lifeflag = intel.i32(p, ident + intel.IDENT_LIFEFLAG)
    detected_raw = intel.i32(p, ident + intel.IDENT_DETECTED)
    skill, xp, xp_raw = intel.decode_skill_xp(p, ident)
    return {
        "identity_addr": ident,
        "identity_addr_hex": intel.hx(ident),
        "leader_addr": intel.hx(ident + intel.LEADER_FROM_IDENT),
        "op_id": op_id,
        "name": name,
        "classdesc": intel.hx(classdesc),
        "classdesc_rva": intel.hx(classdesc - p.module_base),
        "classdesc_rtti": intel.rtti_type_name_for_vtable(p, classdesc)
        if intel.is_module(p, classdesc)
        else None,
        "lifeflag": lifeflag,
        "owner_id": owner_id,
        "owner_tag": tags.get(owner_id) if owner_id is not None else None,
        "owner_valid": owner_valid,
        "detected_raw": detected_raw,
        "skill": skill,
        "experience": xp,
        "experience_raw": xp_raw,
    }


def scan_operative_identities(
    p: Any,
    tags: dict[int, str],
    operative_vtables: set[int],
    *,
    max_seconds: float | None,
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    started = time.time()
    needles = [(vtable, struct.pack("<Q", vtable)) for vtable in sorted(operative_vtables)]
    rows_by_ident: dict[int, dict[str, Any]] = {}
    regions_scanned = 0
    bytes_scanned = 0
    read_failures = 0
    timed_out = False
    raw_vtable_hits = 0
    rejected_not_identity = 0

    for base, size in p.iter_regions(intel.HEAP_LO, intel.HEAP_HI, want_exec=False):
        if max_seconds is not None and time.time() - started > max_seconds:
            timed_out = True
            break
        regions_scanned += 1
        pos = 0
        while pos < size:
            if max_seconds is not None and time.time() - started > max_seconds:
                timed_out = True
                break
            want = min(SCAN_CHUNK, size - pos)
            blob = p.try_read(base + pos, want)
            if not blob:
                read_failures += 1
                pos += max(want, 0x1000)
                continue
            bytes_scanned += len(blob)
            for _vtable, needle in needles:
                search = 0
                while True:
                    idx = blob.find(needle, search)
                    if idx < 0:
                        break
                    search = idx + 1
                    addr = base + pos + idx
                    if addr % 8 != 0:
                        continue
                    raw_vtable_hits += 1
                    ident = addr - intel.IDENT_CLASSDESC
                    row = read_identity_row(p, ident, tags, operative_vtables)
                    if row is None:
                        rejected_not_identity += 1
                        continue
                    rows_by_ident[ident] = row
            got = len(blob)
            if pos + got >= size or got < want:
                pos += max(got, 0x1000)
            else:
                pos += max(got - OVERLAP, 0x1000)
        if timed_out:
            break

    return list(rows_by_ident.values()), {
        "regions_scanned": regions_scanned,
        "bytes_scanned": bytes_scanned,
        "read_failures": read_failures,
        "raw_vtable_hits": raw_vtable_hits,
        "rejected_not_identity": rejected_not_identity,
        "timed_out": timed_out,
        "elapsed_seconds": round(time.time() - started, 3),
    }


def summarize_groups(rows: list[dict[str, Any]]) -> tuple[list[dict[str, Any]], dict[str, int]]:
    groups: dict[str, dict[str, Any]] = {}
    owner_counts: dict[str, int] = {}
    for row in rows:
        owner_tag = str(row.get("owner_tag") or row.get("owner_id"))
        owner_counts[owner_tag] = owner_counts.get(owner_tag, 0) + 1
        classdesc = parse_hex(row.get("classdesc")) or 0
        lifeflag = row.get("lifeflag")
        owner_valid = bool(row.get("owner_valid"))
        key = group_key(classdesc, lifeflag, owner_valid)
        group = groups.setdefault(
            key,
            {
                "classdesc": row.get("classdesc"),
                "classdesc_rva": row.get("classdesc_rva"),
                "classdesc_rtti": row.get("classdesc_rtti"),
                "lifeflag": lifeflag,
                "owner_valid": owner_valid,
                "count": 0,
                "skill_decoded": 0,
                "experience_decoded": 0,
                "detected_raw_distribution": {},
                "samples": [],
            },
        )
        group["count"] += 1
        if row.get("skill") is not None:
            group["skill_decoded"] += 1
        if row.get("experience") is not None:
            group["experience_decoded"] += 1
        detected = str(row.get("detected_raw"))
        group["detected_raw_distribution"][detected] = (
            group["detected_raw_distribution"].get(detected, 0) + 1
        )
        if len(group["samples"]) < MAX_GROUP_SAMPLES:
            group["samples"].append(
                {
                    "identity_addr": row.get("identity_addr_hex"),
                    "op_id": row.get("op_id"),
                    "name": row.get("name"),
                    "owner_id": row.get("owner_id"),
                    "owner_tag": row.get("owner_tag"),
                    "skill": row.get("skill"),
                    "experience": row.get("experience"),
                    "detected_raw": row.get("detected_raw"),
                }
            )
    return (
        sorted(groups.values(), key=lambda row: (-int(row["count"]), row["lifeflag"] or -1)),
        dict(sorted(owner_counts.items())),
    )


def warm_validate(
    p: Any,
    rows: list[dict[str, Any]],
    tags: dict[int, str],
    operative_vtables: set[int],
    delay_seconds: float,
    *,
    max_samples: int,
) -> dict[str, Any]:
    time.sleep(max(0.0, delay_seconds))
    checked = 0
    valid = 0
    invalid_by_reason: dict[str, int] = {}
    invalid_samples: list[dict[str, Any]] = []

    def add_invalid(row: dict[str, Any], reason: str) -> None:
        invalid_by_reason[reason] = invalid_by_reason.get(reason, 0) + 1
        if len(invalid_samples) < max_samples:
            invalid_samples.append(
                {
                    "identity_addr": row.get("identity_addr_hex"),
                    "op_id": row.get("op_id"),
                    "name": row.get("name"),
                    "reason": reason,
                }
            )

    for row in rows:
        ident = parse_hex(row.get("identity_addr_hex"))
        if ident is None:
            add_invalid(row, "missing_identity_addr")
            continue
        checked += 1
        reread = read_identity_row(p, ident, tags, operative_vtables)
        if reread is None:
            add_invalid(row, "identity_no_longer_valid")
            continue
        comparable = [
            "op_id",
            "name",
            "classdesc",
            "lifeflag",
            "owner_id",
            "owner_tag",
            "detected_raw",
        ]
        if any(reread.get(key) != row.get(key) for key in comparable):
            add_invalid(row, "identity_fields_changed")
            continue
        valid += 1
    return {
        "delay_seconds": delay_seconds,
        "checked_rows": checked,
        "valid_rows": valid,
        "invalid_rows": checked - valid,
        "invalid_by_reason": invalid_by_reason,
        "invalid_samples": invalid_samples,
        "status": "validated" if checked > 0 and checked == valid else "changed",
    }


def prior_group_counts(prior: dict[str, Any]) -> dict[str, int]:
    out: dict[str, int] = {}
    groups = prior.get("operatives", {}).get("classdesc_groups", [])
    if not isinstance(groups, list):
        return out
    for row in groups:
        if not isinstance(row, dict):
            continue
        if row.get("classdesc_rtti") != ".?AVCOperativeLeader@@" or not row.get("owner_valid"):
            continue
        lifeflag = row.get("lifeflag")
        count = row.get("count")
        if isinstance(lifeflag, int) and isinstance(count, int):
            out[str(lifeflag)] = count
    return dict(sorted(out.items()))


def build_report(args: argparse.Namespace) -> tuple[dict[str, Any], int]:
    prior = load_json(Path(args.intel_proof) if args.intel_proof else None)
    p = intel.winmem.open_process("hoi4.exe")
    if p is None:
        raise RuntimeError("hoi4.exe not running")
    with p:
        started = time.time()
        process = intel.process_info(p)
        report: dict[str, Any] = {
            "schema": "battle-plan.diplomacy_intel_operatives.operative_lifecycle_groups_probe.v1",
            "lane": "diplomacy_intel_operatives_live",
            "domain_id": 53,
            "probe": "probe_870ebf_operative_lifecycle_groups",
            "generated_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "process": process,
            "expected_sha256": intel.EXPECTED_SHA256,
            "build_gate_ok": process.get("sha256") == intel.EXPECTED_SHA256,
            "prior_intel_proof": {
                "path": args.intel_proof,
                "operative_lifeflag_counts": prior_group_counts(prior),
            },
        }
        if not report["build_gate_ok"]:
            report["status"] = "build_gate_failed"
            return report, 2

        roots = intel.read_country_roots(p)
        tags = intel.load_tags(p, roots)
        _countries, country_report = intel.validate_country_objects(p, roots, tags)
        rtti = intel.derive_rtti_set(p)
        operative_vtables = intel.vtable_set(rtti, "COperativeLeader")
        report["roots"] = roots
        report["countries"] = country_report
        report["rtti"] = {
            "COperativeLeader_vtables": [
                {"addr": intel.hx(v), "rva": intel.hx(v - p.module_base)}
                for v in sorted(operative_vtables)
            ],
            "module_chunk_count": rtti.get("module_chunk_count"),
            "module_bytes_read": rtti.get("module_bytes_read"),
        }
        if not operative_vtables:
            report["status"] = "operative_rtti_unavailable"
            return report, 2

        rows, scan_report = scan_operative_identities(
            p, tags, operative_vtables, max_seconds=args.max_seconds
        )
        group_rows, owner_counts = summarize_groups(rows)
        report["scan"] = scan_report
        report["identity_offsets"] = {
            "classdesc": intel.hx(intel.IDENT_CLASSDESC),
            "type": intel.hx(0),
            "op_id": intel.hx(4),
            "name": intel.hx(intel.IDENT_NAME),
            "name_duplicate": intel.hx(intel.IDENT_NAME2),
            "detected": intel.hx(intel.IDENT_DETECTED),
            "lifeflag": intel.hx(intel.IDENT_LIFEFLAG),
            "owner": intel.hx(intel.IDENT_OWNER),
        }
        report["skill_xp_offsets"] = {
            "leader_from_identity": intel.hx(intel.LEADER_FROM_IDENT),
            "skill_subobject": intel.hx(intel.OFF_SKILL_SUBOBJ),
            "xp_raw": intel.hx(intel.OFF_XP),
            "skill_level_in_subobject": intel.hx(intel.OFF_LEVEL_IN_SUBOBJ),
        }
        report["lifecycle_groups"] = group_rows
        report["owner_counts"] = owner_counts
        report["warm_validation"] = warm_validate(
            p,
            rows,
            tags,
            operative_vtables,
            max(0, args.warm_delay_ms) / 1000.0,
            max_samples=max(0, args.max_samples),
        )

        current_counts = {
            str(row["lifeflag"]): int(row["count"])
            for row in group_rows
            if row.get("classdesc_rtti") == ".?AVCOperativeLeader@@"
            and row.get("owner_valid")
        }
        expected_flags = {"31", "47", "63"}
        nonactive_flags = sorted(flag for flag in current_counts if flag != "47")
        report["summary"] = {
            "COperativeLeader_identity_rows": sum(current_counts.values()),
            "owner_valid_COperativeLeader_lifeflag_counts": dict(sorted(current_counts.items())),
            "active_lifeflag_47_rows": current_counts.get("47", 0),
            "nonactive_lifeflags_observed": nonactive_flags,
            "prior_counts_match_current": current_counts
            == report["prior_intel_proof"]["operative_lifeflag_counts"],
            "warm_validated": report["warm_validation"].get("status") == "validated",
            "semantic_status": (
                "lifeflag cohorts validated; recruitable/captured/mission semantics still fixture-gated"
            ),
            "backlog_item_narrowed": "operative_full_lifecycle",
        }
        if (
            not scan_report.get("timed_out")
            and expected_flags.issubset(set(current_counts))
            and report["warm_validation"].get("status") == "validated"
        ):
            report["status"] = "operative_lifecycle_groups_validated"
            code = 0
        else:
            report["status"] = "operative_lifecycle_groups_unvalidated"
            code = 2
        report["elapsed_seconds"] = round(time.time() - started, 3)
        return report, code


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--intel-proof")
    ap.add_argument("--out", required=True)
    ap.add_argument("--max-seconds", type=float, default=None)
    ap.add_argument("--max-samples", type=int, default=12)
    ap.add_argument("--warm-delay-ms", type=int, default=250)
    args = ap.parse_args(argv)

    report, code = build_report(args)
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", encoding="utf-8") as f:
        json.dump(report, f, indent=2, sort_keys=True)
        f.write("\n")
    print(
        "probe_870ebf_operative_lifecycle_groups: "
        f"status={report['status']} out={out}"
    )
    return code


if __name__ == "__main__":
    raise SystemExit(main())
