#!/usr/bin/env python3
"""Read-only polling probe for air residual candidates on the game build 870ebf.

This probe intentionally does not hook or mutate hoi4.exe. It rediscoveres
current-build air RTTI/vtables, snapshots validated CAirWing rows, and reports
field deltas that correlate with visible air-war loss/reinforcement activity.
It emits candidates only; it never promotes a sortie, kill, cause, or
reinforcement-reason offset without a later live proof.
"""

from __future__ import annotations

import argparse
import collections
import datetime as dt
import hashlib
import json
import struct
import sys
import time
from pathlib import Path
from typing import Any

import winmem
from market_trade_live_probe import derive_rtti_vtables, read_module_chunks
from naval_air.air_combat_losses_probe_870ebf import (
    AIR_WING_OFF_COUNT,
    AIR_WING_OFF_EFFECTIVE_PLANES,
    AIR_WING_OFF_EQUIPMENT_BANK,
    AIR_WING_OFF_ID,
    AIR_WING_OFF_MISSION_MASK,
    AIR_WING_OFF_MISSION_REGION,
    AIR_WING_OFF_POOL,
    AIR_WING_OFF_TYPE,
    EQUIPMENT_AMOUNT_PER_PLANE,
    EXPECTED_SHA256,
    Probe,
    hx,
)


PROC_NAME = "hoi4.exe"
HEAP_LO = 0x10000000000
HEAP_HI = 0x7FF000000000
SCAN_CHUNK = 64 << 20

AIR_RTTI_NAMES = [
    b".?AVCStrategicAirManager@@",
    b".?AVCStrategicAir@@",
    b".?AVCAirBase@@",
    b".?AVCAirWing@@",
    b".?AVCAirWingPool@@",
    b".?AVCAirMission@@",
    b".?AVCAirRegionCombatData@@",
    b".?AVCStrategicRegion@@",
    b".?AVCAirCombat@@",
    b".?AVCAirCombatData@@",
    b".?AVCAirCombatManager@@",
    b".?AVCFEXAir@@",
    b".?AVCNavalCombat@@",
]

KNOWN_U32_OFFSETS = {
    AIR_WING_OFF_TYPE: "known_type",
    AIR_WING_OFF_ID: "known_wing_id",
    AIR_WING_OFF_POOL: "known_pool_low32",
    AIR_WING_OFF_COUNT: "known_plane_count",
    AIR_WING_OFF_MISSION_MASK: "known_mission_mask",
    AIR_WING_OFF_MISSION_REGION: "known_mission_region_ptr_low32",
    AIR_WING_OFF_EFFECTIVE_PLANES: "known_effective_planes",
    AIR_WING_OFF_EQUIPMENT_BANK: "known_equipment_bank_ptr_low32",
}


def utc_now() -> str:
    return dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat()


def find_pid(process_name: str) -> int | None:
    return winmem.find_pid(process_name)


def read_fixture(path: Path | None) -> dict[str, Any]:
    if path is None:
        return {"available": False, "reason": "fixture_json_not_provided"}
    with path.open("r", encoding="utf-8") as handle:
        data = json.load(handle)
    if not isinstance(data, dict):
        raise ValueError(f"{path}: expected JSON object")
    data.setdefault("source", str(path))
    data["available"] = True
    return data


def count_live_bodies(
    proc: winmem.Process,
    vtable: int,
    *,
    limit: int,
    max_seconds: float,
) -> dict[str, Any]:
    started = time.monotonic()
    needle = struct.pack("<Q", vtable)
    hits: list[int] = []
    regions_scanned = 0
    timed_out = False
    for base, size in proc.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        if time.monotonic() - started > max_seconds:
            timed_out = True
            break
        regions_scanned += 1
        pos = 0
        while pos < size:
            if time.monotonic() - started > max_seconds:
                timed_out = True
                break
            want = min(SCAN_CHUNK, size - pos)
            blob = proc.try_read(base + pos, want)
            if not blob:
                pos += max(want, 0x1000)
                continue
            start = 0
            while True:
                idx = blob.find(needle, start)
                if idx < 0:
                    break
                addr = base + pos + idx
                if addr % 8 == 0:
                    hits.append(addr)
                    if len(hits) >= limit:
                        return {
                            "hit_count_capped": len(hits),
                            "sample_hits": [hx(hit) for hit in hits[:16]],
                            "limit_reached": True,
                            "regions_scanned": regions_scanned,
                            "timed_out": timed_out,
                            "elapsed_s": round(time.monotonic() - started, 3),
                        }
                start = idx + 1
            got = len(blob)
            if pos + got >= size or got < want:
                pos += max(got, 0x1000)
            else:
                pos += max(got - 16, 0x1000)
        if timed_out:
            break
    return {
        "hit_count_capped": len(hits),
        "sample_hits": [hx(hit) for hit in hits[:16]],
        "limit_reached": False,
        "regions_scanned": regions_scanned,
        "timed_out": timed_out,
        "elapsed_s": round(time.monotonic() - started, 3),
    }


def rediscover_air_rtti(
    proc: winmem.Process,
    *,
    count_bodies: bool,
    body_limit: int,
    body_count_seconds: float,
) -> dict[str, Any]:
    chunks = read_module_chunks(proc)
    classes: dict[str, Any] = {}
    for name in AIR_RTTI_NAMES:
        key = name.decode("ascii", errors="replace")
        try:
            string_addrs, col_hits, vtables = derive_rtti_vtables(proc, chunks, name)
            row: dict[str, Any] = {
                "string_hits": len(string_addrs),
                "complete_object_locator_hits": len(col_hits),
                "vtables": [hx(value) for value in vtables],
                "vtable_rvas": [hx(value - proc.module_base) for value in vtables],
            }
            if count_bodies:
                row["live_body_counts"] = {
                    hx(vtable - proc.module_base): count_live_bodies(
                        proc,
                        vtable,
                        limit=body_limit,
                        max_seconds=body_count_seconds,
                    )
                    for vtable in vtables
                }
            classes[key] = row
        except Exception as exc:  # noqa: BLE001 - proof script records all failures.
            classes[key] = {"error": repr(exc)}
    return {
        "method": "MSVC RTTI complete-object-locator rediscovery from live module",
        "class_count": len(classes),
        "classes": classes,
    }


def row_key(row: dict[str, Any]) -> str:
    return f"{row['owner_tag_db_id']}:{row['wing_id']}"


def selected_rows(
    rows: list[dict[str, Any]],
    fixture: dict[str, Any],
    max_wings: int,
) -> list[dict[str, Any]]:
    expected_regions = {
        int(value)
        for value in fixture.get("expected_regions", [])
        if isinstance(value, int) or str(value).isdigit()
    }
    active = [row for row in rows if row.get("mission_region") is not None]
    if expected_regions:
        active = [row for row in active if row.get("mission_region") in expected_regions]
    if not active:
        active = [row for row in rows if row.get("mission_region") is not None] or rows
    return active[:max_wings]


def read_body_u32(proc: winmem.Process, body: int, span: int) -> dict[int, int]:
    blob = proc.try_read(body, span)
    out: dict[int, int] = {}
    for off in range(0, len(blob) - 3, 4):
        out[off] = struct.unpack_from("<I", blob, off)[0]
    return out


def build_snapshot(
    proc: winmem.Process,
    probe: Probe,
    strategic_air: int,
    fixture: dict[str, Any],
    args: argparse.Namespace,
) -> dict[str, Any]:
    rows, topology, _sources, _pools = probe.enumerate_air_wings(strategic_air)
    selected = selected_rows(rows, fixture, args.max_wings)
    field_rows: dict[str, dict[int, int]] = {}
    wing_rows = []
    for row in selected:
        key = row_key(row)
        fields = read_body_u32(proc, row["body"], args.body_span)
        field_rows[key] = fields
        wing_rows.append(
            {
                "key": key,
                "owner_tag_db_id": row["owner_tag_db_id"],
                "wing_id": row["wing_id"],
                "object": hx(row["object"]),
                "body": hx(row["body"]),
                "pool": hx(row["pool"]),
                "count": row["count"],
                "effective_planes": row["effective_planes"],
                "mission_mask": row["mission_mask"],
                "mission_region": row["mission_region"],
                "equipment_amount_sum": row["equipment_amount_sum"],
                "equipment_variant_count": row["equipment_variant_count"],
            }
        )
    region_owner = collections.Counter()
    for row in rows:
        if row.get("mission_region") is not None:
            region_owner[(row["mission_region"], row["owner_tag_db_id"])] += row["count"]
    return {
        "sample_utc": utc_now(),
        "air_wing_rows": len(rows),
        "active_region_wings": sum(1 for row in rows if row.get("mission_region") is not None),
        "topology": topology,
        "selected_wings": wing_rows,
        "body_u32_fields": {
            key: {f"0x{off:X}": value for off, value in fields.items()}
            for key, fields in field_rows.items()
        },
        "region_owner_plane_counts": [
            {
                "mission_region": region,
                "owner_tag_db_id": owner,
                "plane_count_sum": count,
            }
            for (region, owner), count in sorted(region_owner.items())
        ],
    }


def summarize_fixture_crosscheck(
    fixture: dict[str, Any],
    first_snapshot: dict[str, Any],
) -> dict[str, Any]:
    if not fixture.get("available"):
        return fixture
    rows_by_key = {row["key"]: row for row in first_snapshot.get("selected_wings", [])}
    region_counts = {
        (row["mission_region"], row["owner_tag_db_id"]): row["plane_count_sum"]
        for row in first_snapshot.get("region_owner_plane_counts", [])
    }
    missing_wings = []
    matched_wings = 0
    for expected in fixture.get("expected_wings", []):
        if not isinstance(expected, dict):
            continue
        key = f"{expected.get('owner_tag_db_id')}:{expected.get('wing_id')}"
        row = rows_by_key.get(key)
        if row is None:
            missing_wings.append(expected)
            continue
        matched_wings += 1
    missing_region_counts = []
    matched_region_counts = 0
    for expected in fixture.get("expected_region_owner_counts", []):
        if not isinstance(expected, dict):
            continue
        key = (expected.get("mission_region"), expected.get("owner_tag_db_id"))
        actual = region_counts.get(key)
        min_count = expected.get("min_plane_count", 1)
        if isinstance(actual, int) and actual >= min_count:
            matched_region_counts += 1
        else:
            missing_region_counts.append({**expected, "actual_plane_count": actual})
    return {
        "available": True,
        "source": fixture.get("source"),
        "matched_expected_wings": matched_wings,
        "missing_expected_wings": missing_wings,
        "matched_region_owner_counts": matched_region_counts,
        "missing_region_owner_counts": missing_region_counts,
        "status": "passed" if not missing_wings and not missing_region_counts else "failed_or_fixture_stale",
    }


def summarize_deltas(samples: list[dict[str, Any]]) -> dict[str, Any]:
    field_changes: collections.Counter[int] = collections.Counter()
    field_examples: dict[int, list[dict[str, Any]]] = collections.defaultdict(list)
    loss_events = []
    reinforcement_events = []
    loss_correlated_fields: collections.Counter[int] = collections.Counter()

    for before, after in zip(samples, samples[1:]):
        before_rows = {row["key"]: row for row in before.get("selected_wings", [])}
        after_rows = {row["key"]: row for row in after.get("selected_wings", [])}
        before_fields = {
            key: {int(off, 16): value for off, value in fields.items()}
            for key, fields in before.get("body_u32_fields", {}).items()
        }
        after_fields = {
            key: {int(off, 16): value for off, value in fields.items()}
            for key, fields in after.get("body_u32_fields", {}).items()
        }
        interval_loss_keys = set()
        for key, old in before_rows.items():
            new = after_rows.get(key)
            if new is None:
                continue
            count_delta = new["count"] - old["count"]
            equipment_delta = new["equipment_amount_sum"] - old["equipment_amount_sum"]
            if count_delta < 0 or equipment_delta < 0:
                interval_loss_keys.add(key)
                loss_events.append(
                    {
                        "from_utc": before["sample_utc"],
                        "to_utc": after["sample_utc"],
                        "key": key,
                        "owner_tag_db_id": new["owner_tag_db_id"],
                        "wing_id": new["wing_id"],
                        "mission_region": new["mission_region"],
                        "mission_mask": new["mission_mask"],
                        "count_delta": count_delta,
                        "equipment_amount_delta": equipment_delta,
                        "equipment_delta_scale_ok": (
                            count_delta == 0
                            or equipment_delta == count_delta * EQUIPMENT_AMOUNT_PER_PLANE
                        ),
                    }
                )
            elif count_delta > 0 or equipment_delta > 0:
                reinforcement_events.append(
                    {
                        "from_utc": before["sample_utc"],
                        "to_utc": after["sample_utc"],
                        "key": key,
                        "owner_tag_db_id": new["owner_tag_db_id"],
                        "wing_id": new["wing_id"],
                        "mission_region": new["mission_region"],
                        "mission_mask": new["mission_mask"],
                        "count_delta": count_delta,
                        "equipment_amount_delta": equipment_delta,
                        "equipment_delta_scale_ok": (
                            count_delta == 0
                            or equipment_delta == count_delta * EQUIPMENT_AMOUNT_PER_PLANE
                        ),
                    }
                )
        for key, old_fields in before_fields.items():
            new_fields = after_fields.get(key)
            if not new_fields:
                continue
            for off, old_value in old_fields.items():
                if off in KNOWN_U32_OFFSETS:
                    continue
                new_value = new_fields.get(off)
                if new_value is None or new_value == old_value:
                    continue
                field_changes[off] += 1
                if key in interval_loss_keys:
                    loss_correlated_fields[off] += 1
                if len(field_examples[off]) < 8:
                    field_examples[off].append(
                        {
                            "from_utc": before["sample_utc"],
                            "to_utc": after["sample_utc"],
                            "key": key,
                            "old": old_value,
                            "new": new_value,
                            "known_offset_note": KNOWN_U32_OFFSETS.get(off),
                        }
                    )

    field_candidates = [
        {
            "offset": f"0x{off:X}",
            "change_count": count,
            "loss_correlated_change_count": loss_correlated_fields.get(off, 0),
            "examples": field_examples[off],
            "status": "candidate_requires_save_or_ui_semantic_fixture",
        }
        for off, count in field_changes.most_common(80)
    ]
    return {
        "loss_like_delta_count": len(loss_events),
        "reinforcement_like_delta_count": len(reinforcement_events),
        "loss_like_examples": loss_events[:40],
        "reinforcement_like_examples": reinforcement_events[:40],
        "unknown_u32_field_candidates": field_candidates,
        "sortie_lifecycle_polling_status": (
            "candidate_fields_seen_requires_fixture"
            if field_candidates
            else "no_unknown_candidate_field_deltas_seen"
        ),
        "kill_attribution_polling_status": (
            "not_validated_by_polling"
            if loss_events
            else "not_observed_no_loss_fixture"
        ),
        "interpretation": (
            "Polling can observe wing-local deltas and region/mission context, but a "
            "killer/cause row requires either a stable opposed participant field in these "
            "candidate deltas or an air-resolution hook. No candidate is promoted here."
        ),
    }


def run(args: argparse.Namespace) -> tuple[int, dict[str, Any]]:
    pid = args.pid or find_pid(args.process)
    if pid is None:
        return 2, {
            "schema": "air_rtti_polling_probe_870ebf_v1",
            "created_utc": utc_now(),
            "status": "failed_closed_process_not_found",
            "process": args.process,
        }

    fixture = read_fixture(args.fixture_json)
    probe = Probe(pid)
    try:
        root = probe.root_probe()
        if not root.get("sha256_ok"):
            return 3, {
                "schema": "air_rtti_polling_probe_870ebf_v1",
                "created_utc": utc_now(),
                "status": "failed_closed_sha_mismatch",
                "root": root,
            }
        game_state, strategic_air = probe.require_root(root)
        rtti = rediscover_air_rtti(
            probe.proc,
            count_bodies=not args.skip_body_counts,
            body_limit=args.body_count_limit,
            body_count_seconds=args.body_count_seconds,
        )
        samples = []
        deadline = time.monotonic() + args.duration_s
        while True:
            samples.append(build_snapshot(probe.proc, probe, strategic_air, fixture, args))
            if time.monotonic() + args.interval_s > deadline:
                break
            time.sleep(args.interval_s)
        deltas = summarize_deltas(samples)
        fixture_crosscheck = summarize_fixture_crosscheck(fixture, samples[0] if samples else {})
        report = {
            "schema": "air_rtti_polling_probe_870ebf_v1",
            "created_utc": utc_now(),
            "read_only": True,
            "process": args.process,
            "pid": pid,
            "module_sha256_expected": EXPECTED_SHA256,
            "root": root,
            "game_state": hx(game_state),
            "strategic_air": hx(strategic_air),
            "rtti_rediscovery": rtti,
            "fixture_crosscheck": fixture_crosscheck,
            "sampling": {
                "duration_s": args.duration_s,
                "interval_s": args.interval_s,
                "sample_count": len(samples),
                "max_wings": args.max_wings,
                "body_span": f"0x{args.body_span:X}",
                "first_snapshot": samples[0] if samples else None,
                "last_snapshot": samples[-1] if samples else None,
            },
            "delta_summary": deltas,
            "candidate_promotion_policy": (
                "Fail closed: unknown fields are only candidates. Promotion requires a "
                "same-session save/UI oracle match and either a stable polling field with "
                "specific semantics or a separately approved resolution hook."
            ),
            "status": "completed_candidates_only",
        }
        if args.include_all_samples:
            report["sampling"]["all_samples"] = samples
        return 0, report
    except Exception as exc:  # noqa: BLE001
        return 4, {
            "schema": "air_rtti_polling_probe_870ebf_v1",
            "created_utc": utc_now(),
            "status": "failed_closed_exception",
            "error": repr(exc),
        }
    finally:
        probe.close()


def write_report(path: Path | None, report: dict[str, Any]) -> None:
    if path is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int)
    parser.add_argument("--process", default=PROC_NAME)
    parser.add_argument("--duration-s", type=float, default=30.0)
    parser.add_argument("--interval-s", type=float, default=0.25)
    parser.add_argument("--max-wings", type=int, default=96)
    parser.add_argument("--body-span", type=lambda value: int(value, 0), default=0x380)
    parser.add_argument("--fixture-json", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--include-all-samples", action="store_true")
    parser.add_argument("--skip-body-counts", action="store_true")
    parser.add_argument("--body-count-limit", type=int, default=128)
    parser.add_argument("--body-count-seconds", type=float, default=1.5)
    args = parser.parse_args()

    code, report = run(args)
    write_report(args.out, report)
    summary = {
        "out": str(args.out),
        "status": report.get("status"),
        "sha_ok": report.get("root", {}).get("sha256_ok"),
        "loss_like_delta_count": report.get("delta_summary", {}).get("loss_like_delta_count"),
        "reinforcement_like_delta_count": report.get("delta_summary", {}).get("reinforcement_like_delta_count"),
        "unknown_candidate_count": len(
            report.get("delta_summary", {}).get("unknown_u32_field_candidates", [])
        ),
        "report_sha256": hashlib.sha256(
            json.dumps(report, sort_keys=True).encode("utf-8")
        ).hexdigest(),
    }
    print(json.dumps(summary, indent=2, sort_keys=True))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
