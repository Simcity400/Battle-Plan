"""Probe targeted direct-OOB root refresh-on-mismatch candidates.

This is lane-local discovery/proof code. It starts with the current-build cold
direct-root discovery, then validates cached addresses across warm iterations.
When cached roots go stale it tries one targeted handle scan for only those
stale `(type,id)` handles. The goal is to validate or reject this candidate
family as a bounded DLL-native refresh-on-mismatch strategy.
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
import json
from pathlib import Path
import sys
import time
from typing import Any

import numpy as np


REPO = Path(__file__).resolve().parents[4]
MEMORY_READER = REPO / "extractor" / "memory_reader"
LANE_DIR = Path(__file__).resolve().parent
for path in (str(REPO), str(MEMORY_READER), str(LANE_DIR)):
    if path not in sys.path:
        sys.path.insert(0, path)

import direct_oob_probe  # type: ignore  # noqa: E402
import fastscan  # type: ignore  # noqa: E402
import winmem  # type: ignore  # noqa: E402
from direct_oob_warm_cache_probe import (  # type: ignore  # noqa: E402
    ROOT_RVAS,
    serialize_roots,
    validate_cached_roots,
)


SCHEMA = "army_oob_orders_battleplans_direct_oob_targeted_refresh_v1"
PROC_NAME = direct_oob_probe.PROC_NAME
EXPECTED_SHA256 = direct_oob_probe.EXPECTED_SHA256
DIRECT_KINDS = ("army", "army_group", "theatre")
ALL_KINDS = ("army", "army_group", "theatre", "division")
DEFAULT_CHUNK = 32 << 20
DEFAULT_MAX_BOUNDED_SCAN_BYTES = 512 << 20


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int)
    ap.add_argument("--out-dir", type=Path, required=True)
    ap.add_argument("--iterations", type=int, default=3)
    ap.add_argument("--sleep-s", type=float, default=0.25)
    ap.add_argument("--chunk-mb", type=int, default=32)
    ap.add_argument("--max-refresh-ids", type=int, default=256)
    ap.add_argument("--max-bounded-scan-mb", type=int, default=512)
    ap.add_argument(
        "--synthetic-stale-per-kind",
        type=int,
        default=0,
        help="deliberately invalidate this many cached direct roots per kind to fixture refresh-on-mismatch",
    )
    args = ap.parse_args()

    payload = run(
        args.pid,
        args.out_dir,
        iterations=args.iterations,
        sleep_s=args.sleep_s,
        chunk_bytes=max(1, args.chunk_mb) << 20,
        max_refresh_ids=args.max_refresh_ids,
        max_bounded_scan_bytes=max(1, args.max_bounded_scan_mb) << 20,
        synthetic_stale_per_kind=max(0, args.synthetic_stale_per_kind),
    )
    view = {
        "status": "PASS" if not payload["errors"] else "FAIL",
        "candidate_status": payload["candidate_status"],
        "errors": payload["errors"],
        "preflight_counts": payload["preflight"]["summary"]["counts"],
        "warm_iterations": len(payload["warm_iterations"]),
        "totals": payload["totals"],
        "rejection": payload["rejected_candidate_family"],
    }
    print(json.dumps(view, indent=2, sort_keys=True))
    return 0 if not payload["errors"] else 1


def run(
    pid: int | None,
    out_dir: Path,
    *,
    iterations: int,
    sleep_s: float,
    chunk_bytes: int = DEFAULT_CHUNK,
    max_refresh_ids: int = 256,
    max_bounded_scan_bytes: int = DEFAULT_MAX_BOUNDED_SCAN_BYTES,
    synthetic_stale_per_kind: int = 0,
) -> dict[str, Any]:
    if iterations <= 0:
        raise ValueError("iterations must be positive")
    if pid is None:
        pid = winmem.find_pid(PROC_NAME)
    if pid is None:
        raise RuntimeError("hoi4.exe not running")

    started = time.monotonic()
    proc = winmem.Process(pid, PROC_NAME)
    try:
        fingerprint = proc.fingerprint()
        sha = str(fingerprint.get("sha256") or "").lower()
        if sha != EXPECTED_SHA256:
            raise RuntimeError(f"build gate failed: {sha} != {EXPECTED_SHA256}")

        preflight_started = time.monotonic()
        roots, scan_stats = direct_oob_probe.enum_current_oob_roots(proc)
        preflight_summary = direct_oob_probe.summarize_direct_land_oob(proc, roots)
        preflight_elapsed_s = time.monotonic() - preflight_started
        cached_roots = serialize_roots(roots)
        synthetic_fixture = inject_synthetic_stale_roots(cached_roots, synthetic_stale_per_kind)

        warm_iterations: list[dict[str, Any]] = []
        for iteration in range(iterations):
            iter_started = time.monotonic()
            valid_roots, invalid_roots = validate_cached_roots(proc, cached_roots)
            refresh = refresh_invalid_roots(
                proc,
                invalid_roots,
                chunk_bytes=chunk_bytes,
                max_refresh_ids=max_refresh_ids,
            )
            refreshed_root_objects = refresh.pop("_refreshed_root_objects")
            refresh_cached_roots(cached_roots, refreshed_root_objects)
            current_roots = merge_roots(valid_roots, refreshed_root_objects)
            summary = direct_oob_probe.summarize_direct_land_oob(proc, current_roots)
            warm_iterations.append(
                {
                    "iteration": iteration,
                    "elapsed_s": time.monotonic() - iter_started,
                    "validation_mode": "cached_address_then_targeted_handle_refresh",
                    "warm_heap_sweep": False,
                    "targeted_handle_refresh": True,
                    "invalid_cached_roots": invalid_roots,
                    "invalid_cached_root_count": len(invalid_roots),
                    "refresh": refresh,
                    "summary": summary,
                }
            )
            if iteration + 1 < iterations and sleep_s > 0:
                time.sleep(sleep_s)

        candidate_status, rejection, production_position = classify_candidate(
            preflight_summary,
            warm_iterations,
            max_bounded_scan_bytes=max_bounded_scan_bytes,
        )
        payload = {
            "schema": SCHEMA,
            "created_utc": datetime.now(timezone.utc).isoformat(),
            "pid": pid,
            "exe_sha256": sha,
            "module_base": proc.module_base,
            "module_path": proc.module_path,
            "elapsed_s": time.monotonic() - started,
            "preflight": {
                "elapsed_s": preflight_elapsed_s,
                "scan_stats": scan_stats,
                "summary": preflight_summary,
                "root_cache": root_cache_summary(cached_roots),
                "synthetic_stale_fixture": synthetic_fixture,
            },
            "warm_iterations": warm_iterations,
            "totals": totals(warm_iterations),
            "thresholds": {
                "max_bounded_scan_bytes": max_bounded_scan_bytes,
                "max_refresh_ids": max_refresh_ids,
                "synthetic_stale_per_kind": synthetic_stale_per_kind,
            },
            "candidate_status": candidate_status,
            "rejected_candidate_family": rejection,
            "offsets": {
                "handle": direct_oob_probe.OFF_HANDLE,
                "army_vtable_rva": direct_oob_probe.ARMY_VTABLE_RVA,
                "army_group_vtable_rva": direct_oob_probe.AG_VTABLE_RVA,
                "theatre_vtable_rva": direct_oob_probe.THEATRE_VTABLE_RVA,
                "division_vtable_rva": direct_oob_probe.DIVISION_VTABLE_RVA,
                "army_type": direct_oob_probe.ARMY_TYPE,
                "theatre_type": direct_oob_probe.THEATRE_TYPE,
                "division_type": direct_oob_probe.DIVISION_TYPE,
                "division_alt_type": direct_oob_probe.DIVISION_ALT_TYPE,
            },
            "production_position": production_position,
            "errors": [],
        }
        payload["errors"] = validate_payload(payload)
        out_dir.mkdir(parents=True, exist_ok=True)
        (out_dir / "direct_oob_targeted_refresh_probe.json").write_text(
            json.dumps(payload, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        return payload
    finally:
        proc.close()


def refresh_invalid_roots(
    proc: Any,
    invalid_roots: list[dict[str, Any]],
    *,
    chunk_bytes: int,
    max_refresh_ids: int,
) -> dict[str, Any]:
    targets = [
        {
            "kind": str(root["kind"]),
            "id": int(root["id"]),
            "old_addr": str(root["addr"]),
            "reason": str(root["reason"]),
        }
        for root in invalid_roots
        if str(root.get("kind")) in ALL_KINDS and int(root.get("id", 0)) > 0
    ]
    limited_targets = targets[:max_refresh_ids]
    skipped_targets = targets[max_refresh_ids:]
    target_by_handle: dict[int, list[dict[str, Any]]] = defaultdict(list)
    for target in limited_targets:
        target_by_handle[handle_value(target["kind"], target["id"])].append(target)

    if not target_by_handle:
        return empty_refresh(targets, skipped_targets)

    arena = fastscan.discover_arena(proc.pid)
    if arena is None:
        raise RuntimeError("could not discover heap arena")
    regions = fastscan.enumerate_regions(proc.handle, arena[0], arena[1])
    handles = np.array(sorted(target_by_handle), dtype=np.uint64)

    started = time.monotonic()
    bytes_scanned = 0
    membership_false_positive_hits = 0
    raw_handle_hits = 0
    validated: dict[tuple[str, int], dict[str, Any]] = {}
    rejected_hits: Counter[str] = Counter()
    hit_samples: list[dict[str, Any]] = []

    for base, size in regions:
        pos = 0
        while pos < size:
            want = min(chunk_bytes, size - pos)
            blob = proc.try_read(base + pos, want)
            got = len(blob)
            if not blob:
                pos += max(want, 0x1000)
                continue

            aligned_len = got - (got % 8)
            if aligned_len > 0:
                arr = np.frombuffer(blob[:aligned_len], dtype=np.uint64)
                mask = np.isin(arr, handles)
                if bool(mask.any()):
                    for idx in np.nonzero(mask)[0]:
                        handle = int(arr[int(idx)])
                        candidates = target_by_handle.get(handle)
                        if not candidates:
                            membership_false_positive_hits += 1
                            continue
                        raw_handle_hits += 1
                        handle_addr = base + pos + int(idx) * 8
                        for candidate in candidates:
                            validated_hit, reject_reason = validate_handle_hit(
                                proc,
                                candidate["kind"],
                                candidate["id"],
                                handle_addr,
                            )
                            if validated_hit is None:
                                rejected_hits[reject_reason] += 1
                                continue
                            key = (candidate["kind"], candidate["id"])
                            current = validated.get(key)
                            if current is None or int(validated_hit["addr"], 16) != int(candidate["old_addr"], 16):
                                validated[key] = validated_hit
                            if len(hit_samples) < 16:
                                hit_samples.append(validated_hit)
            bytes_scanned += got
            if pos + got >= size or got < want:
                pos += max(got, 0x1000)
            else:
                pos += got

    refreshed_roots = {kind: [] for kind in ALL_KINDS}
    refreshed_rows = []
    unresolved = []
    for target in limited_targets:
        key = (target["kind"], target["id"])
        hit = validated.get(key)
        if hit is None:
            unresolved.append(target)
            continue
        addr = int(hit["addr"], 16)
        refreshed_roots[target["kind"]].append(direct_oob_probe.ObjRoot(target["id"], addr))
        refreshed_rows.append(
            {
                **target,
                "new_addr": hit["addr"],
                "handle_addr": hit["handle_addr"],
                "refreshed": True,
            }
        )
    unresolved.extend({**target, "reason": "refresh_id_cap_exceeded"} for target in skipped_targets)

    return {
        "arena": [arena[0], arena[1]],
        "regions": len(regions),
        "elapsed_s": time.monotonic() - started,
        "bytes_scanned": bytes_scanned,
        "target_count": len(targets),
        "searched_target_count": len(limited_targets),
        "skipped_target_count": len(skipped_targets),
        "target_handles": len(target_by_handle),
        "raw_handle_hits": raw_handle_hits,
        "membership_false_positive_hits": membership_false_positive_hits,
        "rejected_handle_hits": dict(rejected_hits),
        "refreshed_count": len(refreshed_rows),
        "unresolved_count": len(unresolved),
        "refreshed_by_kind": dict(Counter(row["kind"] for row in refreshed_rows)),
        "unresolved_by_kind": dict(Counter(row["kind"] for row in unresolved)),
        "refreshed_roots": serialize_roots(refreshed_roots),
        "_refreshed_root_objects": refreshed_roots,
        "refreshed_rows": refreshed_rows,
        "unresolved": unresolved,
        "validated_hit_samples": hit_samples,
    }


def empty_refresh(targets: list[dict[str, Any]], skipped_targets: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "arena": None,
        "regions": 0,
        "elapsed_s": 0.0,
        "bytes_scanned": 0,
        "target_count": len(targets),
        "searched_target_count": 0,
        "skipped_target_count": len(skipped_targets),
        "target_handles": 0,
        "raw_handle_hits": 0,
        "membership_false_positive_hits": 0,
        "rejected_handle_hits": {},
        "refreshed_count": 0,
        "unresolved_count": len(targets),
        "refreshed_by_kind": {},
        "unresolved_by_kind": dict(Counter(row["kind"] for row in targets)),
        "refreshed_roots": {kind: [] for kind in ALL_KINDS},
        "_refreshed_root_objects": {kind: [] for kind in ALL_KINDS},
        "refreshed_rows": [],
        "unresolved": targets,
        "validated_hit_samples": [],
    }


def handle_value(kind: str, object_id: int) -> int:
    if kind == "theatre":
        type_tag = direct_oob_probe.THEATRE_TYPE
    elif kind == "division":
        type_tag = direct_oob_probe.DIVISION_TYPE
    else:
        type_tag = direct_oob_probe.ARMY_TYPE
    return (int(object_id) << 32) | int(type_tag)


def validate_handle_hit(proc: Any, kind: str, expected_id: int, handle_addr: int) -> tuple[dict[str, Any] | None, str]:
    object_addr = handle_addr - direct_oob_probe.OFF_HANDLE
    try:
        expected_vtable = proc.module_base + ROOT_RVAS[kind]
        actual_vtable = proc.u64(object_addr)
    except OSError:
        return None, "unreadable_candidate_object"
    if actual_vtable != expected_vtable:
        return None, "vtable_mismatch"
    actual_id = direct_oob_probe.validate_root_hit(proc, kind, object_addr)
    if actual_id is None:
        return None, "handle_or_type_validation_failed"
    if int(actual_id) != int(expected_id):
        return None, "object_id_mismatch"
    return {
        "kind": kind,
        "id": int(expected_id),
        "addr": f"0x{object_addr:X}",
        "handle_addr": f"0x{handle_addr:X}",
    }, ""


def refresh_cached_roots(
    cached_roots: dict[str, list[dict[str, Any]]],
    refreshed_roots: dict[str, list[Any]],
) -> None:
    by_key = {
        (kind, int(root.id)): int(root.addr)
        for kind, roots in refreshed_roots.items()
        for root in roots
    }
    if not by_key:
        return
    for kind, rows in cached_roots.items():
        for row in rows:
            key = (kind, int(row["id"]))
            if key in by_key:
                row["addr"] = by_key[key]


def merge_roots(
    valid_roots: dict[str, list[Any]],
    refreshed_roots: dict[str, list[Any]],
) -> dict[str, list[Any]]:
    merged: dict[str, dict[int, Any]] = {kind: {} for kind in ALL_KINDS}
    for kind in ALL_KINDS:
        for root in valid_roots.get(kind, []):
            merged[kind][int(root.id)] = root
        for root in refreshed_roots.get(kind, []):
            merged[kind][int(root.id)] = root
    return {
        kind: [merged[kind][object_id] for object_id in sorted(merged[kind])]
        for kind in ALL_KINDS
    }


def root_cache_summary(cached_roots: dict[str, list[dict[str, Any]]]) -> dict[str, Any]:
    return {
        key: {
            "rows": len(rows),
            "sample": [{"id": row["id"], "addr": f"0x{int(row['addr']):X}"} for row in rows[:8]],
        }
        for key, rows in cached_roots.items()
    }


def inject_synthetic_stale_roots(
    cached_roots: dict[str, list[dict[str, Any]]],
    per_kind: int,
) -> dict[str, Any]:
    if per_kind <= 0:
        return {
            "enabled": False,
            "fixture_mode": "none",
            "rows": [],
        }
    rows = []
    for kind in DIRECT_KINDS:
        for row in cached_roots.get(kind, [])[:per_kind]:
            original = int(row["addr"])
            row["addr"] = original + 0x10
            rows.append(
                {
                    "kind": kind,
                    "id": int(row["id"]),
                    "original_addr": f"0x{original:X}",
                    "synthetic_stale_addr": f"0x{int(row['addr']):X}",
                    "expected_invalid_reason": "vtable_mismatch",
                }
            )
    return {
        "enabled": bool(rows),
        "fixture_mode": "cache_address_plus_0x10_for_first_direct_roots",
        "per_kind": per_kind,
        "rows": rows,
    }


def totals(warm_iterations: list[dict[str, Any]]) -> dict[str, Any]:
    refreshes = [row["refresh"] for row in warm_iterations]
    return {
        "warm_iterations": len(warm_iterations),
        "warm_invalid_cached_roots": sum(row["invalid_cached_root_count"] for row in warm_iterations),
        "targeted_refresh_scans": sum(1 for refresh in refreshes if refresh.get("target_count", 0) > 0),
        "targeted_refresh_bytes_scanned": sum(refresh.get("bytes_scanned", 0) for refresh in refreshes),
        "targeted_refresh_elapsed_s": sum(refresh.get("elapsed_s", 0.0) for refresh in refreshes),
        "targeted_refresh_unresolved_roots": sum(refresh.get("unresolved_count", 0) for refresh in refreshes),
        "targeted_refresh_refreshed_roots": sum(refresh.get("refreshed_count", 0) for refresh in refreshes),
        "warm_heap_sweeps": 0,
    }


def classify_candidate(
    preflight_summary: dict[str, Any],
    warm_iterations: list[dict[str, Any]],
    *,
    max_bounded_scan_bytes: int,
) -> tuple[str, dict[str, Any] | None, str]:
    reasons: list[str] = []
    preflight_counts = preflight_summary.get("counts", {})
    saw_mismatch = any(row["invalid_cached_root_count"] > 0 for row in warm_iterations)
    saw_unresolved = any(row["refresh"].get("unresolved_count", 0) > 0 for row in warm_iterations)
    saw_arena_sized_scan = any(
        row["refresh"].get("bytes_scanned", 0) > max_bounded_scan_bytes
        for row in warm_iterations
        if row["refresh"].get("target_count", 0) > 0
    )
    direct_count_drift = []
    lost_empty_coverage = False
    for row in warm_iterations:
        counts = row.get("summary", {}).get("counts", {})
        for key in ("direct_armies", "army_groups", "theatres"):
            if counts.get(key) != preflight_counts.get(key):
                direct_count_drift.append({"iteration": row["iteration"], "count": key})
        if (
            preflight_counts.get("empty_or_inactive_direct_armies", 0) > 0
            and counts.get("empty_or_inactive_direct_armies", 0) <= 0
        ):
            lost_empty_coverage = True
    if not saw_mismatch:
        reasons.append("no stale cached roots occurred; refresh-on-mismatch was not exercised")
    if saw_unresolved:
        reasons.append("one or more stale cached roots were not found by targeted handle refresh")
    if saw_arena_sized_scan:
        reasons.append("targeted handle refresh scanned more bytes than the bounded hot-loop threshold")
    if direct_count_drift:
        reasons.append("direct army/group/theatre counts drifted after refresh")
    if lost_empty_coverage:
        reasons.append("empty/inactive direct-army coverage was lost after refresh")

    if reasons:
        return (
            "REJECTED_TARGETED_HANDLE_REFRESH_ON_MISMATCH",
            {
                "candidate_family": "cached_direct_roots_plus_targeted_handle_scan_refresh_on_mismatch",
                "reasons": reasons,
                "max_bounded_scan_bytes": max_bounded_scan_bytes,
            },
            (
                "current-build cold direct roots remain validated, but cached roots plus targeted "
                "handle refresh is rejected as a bounded hot-loop production path for the reasons "
                "captured in rejected_candidate_family. Remaining direct-root paths are a real "
                "manager/container root, another bounded DLL-native root enumerator, or fail-closed "
                "badging/omission for direct roots that cannot be produced."
            ),
        )
    return (
        "VALIDATED_BOUNDED_TARGETED_HANDLE_REFRESH",
        None,
        (
            "cached direct roots plus targeted handle refresh repaired all observed mismatches inside "
            "the configured bounded scan threshold. This validates the refresh-on-mismatch candidate "
            "family for a lane-local DLL implementation, pending shared collector registration."
        ),
    )


def validate_payload(payload: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    counts = payload.get("preflight", {}).get("summary", {}).get("counts", {})
    candidate_status = payload.get("candidate_status")
    if counts.get("direct_armies", 0) <= 0:
        errors.append("preflight proved no direct armies")
    if candidate_status == "VALIDATED_BOUNDED_TARGETED_HANDLE_REFRESH":
        if counts.get("direct_armies", 0) <= counts.get("active_armies_from_division_parents", 0):
            errors.append("preflight direct armies did not exceed active division-parent armies")
        if counts.get("empty_or_inactive_direct_armies", 0) <= 0:
            errors.append("preflight proved no empty/inactive direct armies")
    if counts.get("army_groups", 0) <= 0:
        errors.append("preflight proved no army groups")
    if counts.get("theatres", 0) <= 0:
        errors.append("preflight proved no theatres")
    if not payload.get("warm_iterations"):
        errors.append("no warm iterations recorded")
    if candidate_status not in {
        "VALIDATED_BOUNDED_TARGETED_HANDLE_REFRESH",
        "REJECTED_TARGETED_HANDLE_REFRESH_ON_MISMATCH",
    }:
        errors.append("unexpected candidate status")
    for row in payload.get("warm_iterations", []):
        if row.get("validation_mode") != "cached_address_then_targeted_handle_refresh":
            errors.append(f"iteration {row.get('iteration')} has unexpected validation mode")
        refresh = row.get("refresh", {})
        if refresh.get("membership_false_positive_hits", 0) != 0:
            errors.append(f"iteration {row.get('iteration')} had membership false positives")
        if refresh.get("target_count", 0) > 0 and refresh.get("bytes_scanned", 0) <= 0:
            errors.append(f"iteration {row.get('iteration')} refresh scanned no bytes")
    if candidate_status == "REJECTED_TARGETED_HANDLE_REFRESH_ON_MISMATCH":
        rejection = payload.get("rejected_candidate_family")
        if not isinstance(rejection, dict):
            errors.append("missing rejected candidate family")
    return errors


if __name__ == "__main__":
    raise SystemExit(main())
