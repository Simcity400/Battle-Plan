"""Probe current-build direct-OOB root pointer table candidates.

This is lane-local discovery/proof code. It reuses the current-build direct
root scanner, then searches readable the game heap memory for qword references to
validated army, army-group, and theatre roots. The goal is to validate or
reject the simple direct-root manager/table candidate family: a dense pointer
vector or fixed-stride pointer table whose entries are the direct root object
addresses.
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


SCHEMA = "army_oob_orders_battleplans_direct_oob_table_probe_v1"
PROC_NAME = direct_oob_probe.PROC_NAME
EXPECTED_SHA256 = direct_oob_probe.EXPECTED_SHA256
DIRECT_KINDS = ("army", "army_group", "theatre")
SEARCH_STRIDES = (8, 16, 24, 32)
DEFAULT_MIN_RUN = 3
DEFAULT_MIN_KIND_COVERAGE = 0.50
DEFAULT_MIN_TOTAL_DIRECT_COVERAGE = 0.50
DEFAULT_CHUNK = 32 << 20
DEFAULT_MAX_REFERENCE_HITS = 2_000_000


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int)
    ap.add_argument("--out-dir", type=Path, required=True)
    ap.add_argument("--chunk-mb", type=int, default=32)
    ap.add_argument("--min-run", type=int, default=DEFAULT_MIN_RUN)
    ap.add_argument("--max-reference-hits", type=int, default=DEFAULT_MAX_REFERENCE_HITS)
    ap.add_argument("--min-kind-coverage", type=float, default=DEFAULT_MIN_KIND_COVERAGE)
    ap.add_argument("--min-total-direct-coverage", type=float, default=DEFAULT_MIN_TOTAL_DIRECT_COVERAGE)
    args = ap.parse_args()

    payload = run(
        args.pid,
        args.out_dir,
        chunk_bytes=max(1, args.chunk_mb) << 20,
        min_run=args.min_run,
        max_reference_hits=args.max_reference_hits,
        min_kind_coverage=args.min_kind_coverage,
        min_total_direct_coverage=args.min_total_direct_coverage,
    )
    view = {
        "status": "PASS" if not payload["errors"] else "FAIL",
        "candidate_status": payload["candidate_status"],
        "errors": payload["errors"],
        "preflight_counts": payload["preflight"]["summary"]["counts"],
        "reference_hits": payload["table_scan"]["total_reference_hits"],
        "hit_cap_exceeded": payload["table_scan"]["hit_cap_exceeded"],
        "max_coverage": payload["table_scan"]["max_coverage"],
        "top_runs": payload["table_scan"]["top_runs"][:5],
    }
    print(json.dumps(view, indent=2, sort_keys=True))
    return 0 if not payload["errors"] else 1


def run(
    pid: int | None,
    out_dir: Path,
    *,
    chunk_bytes: int = DEFAULT_CHUNK,
    min_run: int = DEFAULT_MIN_RUN,
    max_reference_hits: int = DEFAULT_MAX_REFERENCE_HITS,
    min_kind_coverage: float = DEFAULT_MIN_KIND_COVERAGE,
    min_total_direct_coverage: float = DEFAULT_MIN_TOTAL_DIRECT_COVERAGE,
) -> dict[str, Any]:
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

        special_root_sets = build_special_root_sets(proc, roots)
        table_scan = scan_direct_root_references(
            proc,
            roots,
            special_root_sets=special_root_sets,
            chunk_bytes=chunk_bytes,
            min_run=min_run,
            max_reference_hits=max_reference_hits,
            min_kind_coverage=min_kind_coverage,
            min_total_direct_coverage=min_total_direct_coverage,
        )
        accepted = table_scan["accepted_table_candidates"]
        if accepted:
            has_total_direct_candidate = any(
                row["total_direct_coverage"] >= min_total_direct_coverage
                for row in accepted
            )
            candidate_status = (
                "VALIDATED_DIRECT_ROOT_POINTER_TABLE"
                if has_total_direct_candidate
                else "VALIDATED_PARTIAL_DIRECT_ROOT_POINTER_TABLE"
            )
            rejection = None
            production_position = (
                "current-build direct roots remain validated by cold vtable discovery, and this "
                "probe validates at least one dense/strided direct-root pointer-vector candidate. "
                "The accepted candidate is coverage-bounded by kind; any unaccepted direct root "
                "kind still requires a real manager container, bounded DLL-native refresh on "
                "mismatch, or fail-closed badging/omission."
            )
        else:
            candidate_status = "REJECTED_PLAIN_DIRECT_ROOT_POINTER_TABLE"
            rejection = {
                "candidate_family": "plain_contiguous_or_fixed_stride_direct_root_pointer_table",
                "reason": (
                    "no stride 8/16/24/32 qword-reference run covered enough current-build "
                    "army, army-group, theatre, or total direct-root addresses to be a bounded "
                    "hot-loop root table"
                ),
                "thresholds": {
                    "min_kind_coverage": min_kind_coverage,
                    "min_total_direct_coverage": min_total_direct_coverage,
                },
                "max_coverage": table_scan["max_coverage"],
            }
            production_position = (
                "current-build direct roots remain validated by cold vtable discovery, but this "
                "probe rejects the simple direct manager/vector table candidate family as a "
                "DLL hot-loop source. Remaining paths are a real manager container, a bounded "
                "DLL-native refresh-on-mismatch contract, or fail-closed badging/omission."
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
                "special_root_sets": special_root_sets,
            },
            "table_scan": table_scan,
            "candidate_status": candidate_status,
            "rejected_candidate_family": rejection,
            "offsets": {
                "army_vtable_rva": direct_oob_probe.ARMY_VTABLE_RVA,
                "army_group_vtable_rva": direct_oob_probe.AG_VTABLE_RVA,
                "theatre_vtable_rva": direct_oob_probe.THEATRE_VTABLE_RVA,
                "division_vtable_rva": direct_oob_probe.DIVISION_VTABLE_RVA,
            },
            "production_position": production_position,
            "errors": [],
        }
        payload["errors"] = validate_payload(payload)
        out_dir.mkdir(parents=True, exist_ok=True)
        (out_dir / "direct_oob_table_probe.json").write_text(
            json.dumps(payload, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        return payload
    finally:
        proc.close()


def scan_direct_root_references(
    proc: Any,
    roots: dict[str, list[Any]],
    *,
    special_root_sets: dict[str, list[int]],
    chunk_bytes: int,
    min_run: int,
    max_reference_hits: int,
    min_kind_coverage: float,
    min_total_direct_coverage: float,
) -> dict[str, Any]:
    arena = fastscan.discover_arena(proc.pid)
    if arena is None:
        raise RuntimeError("could not discover heap arena")
    regions = fastscan.enumerate_regions(proc.handle, arena[0], arena[1])

    target_by_addr: dict[int, dict[str, Any]] = {}
    root_counts = {kind: len(roots.get(kind, [])) for kind in DIRECT_KINDS}
    for kind in DIRECT_KINDS:
        for root in roots.get(kind, []):
            target_by_addr[int(root.addr)] = {"kind": kind, "id": int(root.id)}
    root_values = np.array(sorted(target_by_addr), dtype=np.uint64)
    if root_values.size == 0:
        raise RuntimeError("no direct root addresses to search for")

    hits: list[tuple[int, int, str, int]] = []
    ref_hits_by_kind: Counter[str] = Counter()
    unique_by_kind: dict[str, set[int]] = defaultdict(set)
    hit_cap_exceeded = False
    membership_false_positive_hits = 0
    bytes_scanned = 0
    scan_started = time.monotonic()

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
                mask = np.isin(arr, root_values)
                if bool(mask.any()):
                    indexes = np.nonzero(mask)[0]
                    for idx in indexes:
                        ref_addr = base + pos + int(idx) * 8
                        target_addr = int(arr[int(idx)])
                        target = target_by_addr.get(target_addr)
                        if target is None:
                            membership_false_positive_hits += 1
                            continue
                        kind = str(target["kind"])
                        object_id = int(target["id"])
                        ref_hits_by_kind[kind] += 1
                        unique_by_kind[kind].add(object_id)
                        if len(hits) < max_reference_hits:
                            hits.append((ref_addr, target_addr, kind, object_id))
                        else:
                            hit_cap_exceeded = True
            bytes_scanned += got
            if pos + got >= size or got < want:
                pos += max(got, 0x1000)
            else:
                pos += got

    runs = build_candidate_runs(hits, root_counts, min_run=min_run)
    run_union = union_coverage_by_kind(runs, root_counts)
    accepted = [
        run
        for run in runs
        if run["total_direct_coverage"] >= min_total_direct_coverage
        or any(
            coverage >= min_kind_coverage
            for coverage in run["coverage_by_kind"].values()
        )
    ]
    accepted.sort(
        key=lambda row: (
            row["total_direct_coverage"],
            max(row["coverage_by_kind"].values() or [0.0]),
            row["length"],
        ),
        reverse=True,
    )

    return {
        "arena": [arena[0], arena[1]],
        "regions": len(regions),
        "bytes_scanned": bytes_scanned,
        "elapsed_s": time.monotonic() - scan_started,
        "target_root_counts": root_counts,
        "search_strides": list(SEARCH_STRIDES),
        "min_run": min_run,
        "thresholds": {
            "min_kind_coverage": min_kind_coverage,
            "min_total_direct_coverage": min_total_direct_coverage,
        },
        "total_reference_hits": sum(ref_hits_by_kind.values()),
        "stored_reference_hits": len(hits),
        "hit_cap_exceeded": hit_cap_exceeded,
        "membership_false_positive_hits": membership_false_positive_hits,
        "reference_hits_by_kind": dict(ref_hits_by_kind),
        "unique_targets_referenced_by_kind": {
            kind: len(unique_by_kind.get(kind, set())) for kind in DIRECT_KINDS
        },
        "run_union_coverage_by_kind": run_union,
        "special_root_coverage": special_root_coverage(
            hits,
            runs,
            accepted,
            special_root_sets=special_root_sets,
        ),
        "top_runs": runs[:32],
        "max_coverage": max_coverage(runs),
        "accepted_table_candidates": accepted[:16],
    }


def build_special_root_sets(proc: Any, roots: dict[str, list[Any]]) -> dict[str, list[int]]:
    army_by_ptr = {int(root.addr): int(root.id) for root in roots.get("army", [])}
    active_army_ids: set[int] = set()
    for division in roots.get("division", []):
        try:
            parent = proc.u64(int(division.addr) + direct_oob_probe.DIVISION_OFF_ARMY_PARENT)
        except OSError:
            continue
        army_id = army_by_ptr.get(parent)
        if army_id is not None:
            active_army_ids.add(army_id)
    all_army_ids = {int(root.id) for root in roots.get("army", [])}
    empty_or_inactive = sorted(all_army_ids - active_army_ids)
    return {
        "active_armies_from_division_parents": sorted(active_army_ids),
        "empty_or_inactive_armies": empty_or_inactive,
    }


def build_candidate_runs(
    hits: list[tuple[int, int, str, int]],
    root_counts: dict[str, int],
    *,
    min_run: int,
) -> list[dict[str, Any]]:
    if not hits:
        return []
    ordered = sorted(hits, key=lambda row: row[0])
    runs: list[dict[str, Any]] = []
    for stride in SEARCH_STRIDES:
        current: list[tuple[int, int, str, int]] = []
        last_ref = None
        for hit in ordered:
            ref_addr = hit[0]
            if last_ref is not None and ref_addr - last_ref == stride:
                current.append(hit)
            else:
                append_run(runs, current, stride, root_counts, min_run)
                current = [hit]
            last_ref = ref_addr
        append_run(runs, current, stride, root_counts, min_run)
    runs.sort(
        key=lambda row: (
            row["total_direct_coverage"],
            max(row["coverage_by_kind"].values() or [0.0]),
            row["length"],
        ),
        reverse=True,
    )
    return runs


def append_run(
    runs: list[dict[str, Any]],
    current: list[tuple[int, int, str, int]],
    stride: int,
    root_counts: dict[str, int],
    min_run: int,
) -> None:
    if len(current) < min_run:
        return
    unique: dict[str, set[int]] = defaultdict(set)
    for _ref_addr, _target_addr, kind, object_id in current:
        unique[kind].add(object_id)
    unique_counts = {kind: len(unique.get(kind, set())) for kind in DIRECT_KINDS}
    target_ids_by_kind = {kind: sorted(unique.get(kind, set())) for kind in DIRECT_KINDS}
    coverage_by_kind = {
        kind: safe_ratio(unique_counts[kind], root_counts.get(kind, 0))
        for kind in DIRECT_KINDS
    }
    total_direct_roots = sum(root_counts.get(kind, 0) for kind in DIRECT_KINDS)
    total_unique = sum(unique_counts.values())
    runs.append(
        {
            "stride": stride,
            "start_ref_addr": f"0x{current[0][0]:X}",
            "end_ref_addr": f"0x{current[-1][0]:X}",
            "length": len(current),
            "target_ids_by_kind": target_ids_by_kind,
            "unique_targets_by_kind": unique_counts,
            "coverage_by_kind": coverage_by_kind,
            "total_unique_direct_targets": total_unique,
            "total_direct_coverage": safe_ratio(total_unique, total_direct_roots),
            "sample": [
                {
                    "ref_addr": f"0x{ref_addr:X}",
                    "target_addr": f"0x{target_addr:X}",
                    "kind": kind,
                    "id": object_id,
                }
                for ref_addr, target_addr, kind, object_id in current[:8]
            ],
        }
    )


def union_coverage_by_kind(
    runs: list[dict[str, Any]],
    root_counts: dict[str, int],
) -> dict[str, Any]:
    union_ids: dict[str, set[int]] = {kind: set() for kind in DIRECT_KINDS}
    for run in runs:
        target_ids = run.get("target_ids_by_kind", {})
        for kind in DIRECT_KINDS:
            union_ids[kind].update(int(value) for value in target_ids.get(kind, []))
    return {
        kind: {
            "covered": len(union_ids[kind]),
            "total": root_counts.get(kind, 0),
            "coverage": safe_ratio(len(union_ids[kind]), root_counts.get(kind, 0)),
        }
        for kind in DIRECT_KINDS
    }


def special_root_coverage(
    hits: list[tuple[int, int, str, int]],
    runs: list[dict[str, Any]],
    accepted_runs: list[dict[str, Any]],
    *,
    special_root_sets: dict[str, list[int]],
) -> dict[str, Any]:
    referenced_any: dict[str, set[int]] = defaultdict(set)
    for _ref_addr, _target_addr, kind, object_id in hits:
        referenced_any[kind].add(int(object_id))
    run_union: dict[str, set[int]] = defaultdict(set)
    accepted_union: dict[str, set[int]] = defaultdict(set)
    for run in runs:
        for kind, ids in run.get("target_ids_by_kind", {}).items():
            run_union[kind].update(int(value) for value in ids)
    for run in accepted_runs:
        for kind, ids in run.get("target_ids_by_kind", {}).items():
            accepted_union[kind].update(int(value) for value in ids)

    out: dict[str, Any] = {}
    for name, ids in special_root_sets.items():
        kind = "army" if "armies" in name else name
        wanted = set(int(value) for value in ids)
        any_ids = wanted & referenced_any.get(kind, set())
        run_ids = wanted & run_union.get(kind, set())
        accepted_ids = wanted & accepted_union.get(kind, set())
        best_run = best_special_run(wanted, kind, runs)
        out[name] = {
            "kind": kind,
            "total": len(wanted),
            "referenced_anywhere": len(any_ids),
            "referenced_anywhere_ids": sorted(any_ids),
            "covered_by_any_table_run": len(run_ids),
            "covered_by_any_table_run_ids": sorted(run_ids),
            "covered_by_accepted_table_run": len(accepted_ids),
            "covered_by_accepted_table_run_ids": sorted(accepted_ids),
            "best_single_run": best_run,
        }
    return out


def best_special_run(
    wanted: set[int],
    kind: str,
    runs: list[dict[str, Any]],
) -> dict[str, Any] | None:
    if not wanted:
        return None
    best: dict[str, Any] | None = None
    best_count = 0
    for run in runs:
        ids = set(int(value) for value in run.get("target_ids_by_kind", {}).get(kind, []))
        matched = sorted(wanted & ids)
        if len(matched) > best_count:
            best_count = len(matched)
            best = {
                "start_ref_addr": run.get("start_ref_addr"),
                "stride": run.get("stride"),
                "length": run.get("length"),
                "matched": len(matched),
                "total": len(wanted),
                "coverage": safe_ratio(len(matched), len(wanted)),
                "ids": matched,
            }
    return best


def max_coverage(runs: list[dict[str, Any]]) -> dict[str, Any]:
    out = {
        "total_direct_coverage": 0.0,
        "by_kind": {kind: 0.0 for kind in DIRECT_KINDS},
        "best_run": None,
    }
    for run in runs:
        if run["total_direct_coverage"] > out["total_direct_coverage"]:
            out["total_direct_coverage"] = run["total_direct_coverage"]
            out["best_run"] = {
                "stride": run["stride"],
                "start_ref_addr": run["start_ref_addr"],
                "length": run["length"],
            }
        for kind, coverage in run["coverage_by_kind"].items():
            out["by_kind"][kind] = max(out["by_kind"].get(kind, 0.0), coverage)
    return out


def safe_ratio(numerator: int, denominator: int) -> float:
    if denominator <= 0:
        return 0.0
    return float(numerator) / float(denominator)


def validate_payload(payload: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    counts = payload.get("preflight", {}).get("summary", {}).get("counts", {})
    if counts.get("direct_armies", 0) <= counts.get("active_armies_from_division_parents", 0):
        errors.append("preflight direct armies did not exceed active division-parent armies")
    if counts.get("empty_or_inactive_direct_armies", 0) <= 0:
        errors.append("preflight proved no empty/inactive direct armies")
    if counts.get("army_groups", 0) <= 0:
        errors.append("preflight proved no army groups")
    if counts.get("theatres", 0) <= 0:
        errors.append("preflight proved no theatres")
    table_scan = payload.get("table_scan", {})
    if table_scan.get("hit_cap_exceeded"):
        errors.append("reference hit cap exceeded; table rejection would be incomplete")
    if table_scan.get("membership_false_positive_hits", 0) != 0:
        errors.append("reference scan had membership false positives")
    if table_scan.get("bytes_scanned", 0) <= 0:
        errors.append("reference scan read no bytes")
    if table_scan.get("total_reference_hits", 0) <= 0:
        errors.append("reference scan found no direct-root references")
    if payload.get("candidate_status") not in {
        "VALIDATED_DIRECT_ROOT_POINTER_TABLE",
        "VALIDATED_PARTIAL_DIRECT_ROOT_POINTER_TABLE",
        "REJECTED_PLAIN_DIRECT_ROOT_POINTER_TABLE",
    }:
        errors.append("unexpected candidate status")
    if payload.get("candidate_status") == "REJECTED_PLAIN_DIRECT_ROOT_POINTER_TABLE":
        rejection = payload.get("rejected_candidate_family")
        if not isinstance(rejection, dict):
            errors.append("missing rejected candidate family")
    return errors


if __name__ == "__main__":
    raise SystemExit(main())
