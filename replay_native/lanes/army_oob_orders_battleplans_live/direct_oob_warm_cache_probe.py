"""Validate a bounded direct-OOB root warm-cache contract.

This is lane-local proof code. It performs one direct-OOB root discovery pass,
then reuses the discovered addresses for repeated warm reads with vtable,
handle/type, and object-id liveness checks. The warm phase intentionally does
not run another heap/vtable sweep.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import sys
import time
from typing import Any


REPO = Path(__file__).resolve().parents[3]
MEMORY_READER = REPO / "memory_reader"
LANE_DIR = Path(__file__).resolve().parent
for path in (str(REPO), str(MEMORY_READER), str(LANE_DIR)):
    if path not in sys.path:
        sys.path.insert(0, path)

import direct_oob_probe  # type: ignore  # noqa: E402
import winmem  # type: ignore  # noqa: E402


SCHEMA = "army_oob_orders_battleplans_direct_oob_warm_cache_v1"
PROC_NAME = direct_oob_probe.PROC_NAME
EXPECTED_SHA256 = direct_oob_probe.EXPECTED_SHA256

ROOT_RVAS = {
    "army": direct_oob_probe.ARMY_VTABLE_RVA,
    "army_group": direct_oob_probe.AG_VTABLE_RVA,
    "theatre": direct_oob_probe.THEATRE_VTABLE_RVA,
    "division": direct_oob_probe.DIVISION_VTABLE_RVA,
}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int)
    ap.add_argument("--out-dir", type=Path, required=True)
    ap.add_argument("--iterations", type=int, default=5)
    ap.add_argument("--sleep-s", type=float, default=0.25)
    args = ap.parse_args()

    payload = run(args.pid, args.out_dir, iterations=args.iterations, sleep_s=args.sleep_s)
    view = {
        "status": "PASS" if not payload["errors"] else "FAIL",
        "errors": payload["errors"],
        "preflight_counts": payload["preflight"]["summary"]["counts"],
        "warm_iterations": len(payload["warm_iterations"]),
        "warm_invalid_cached_roots": payload["totals"]["warm_invalid_cached_roots"],
        "warm_heap_sweeps": payload["totals"]["warm_heap_sweeps"],
    }
    print(json.dumps(view, indent=2, sort_keys=True))
    return 0 if not payload["errors"] else 1


def run(pid: int | None, out_dir: Path, *, iterations: int, sleep_s: float) -> dict[str, Any]:
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

        warm_iterations = []
        for iteration in range(iterations):
            iter_started = time.monotonic()
            valid_roots, invalid_roots = validate_cached_roots(proc, cached_roots)
            summary = direct_oob_probe.summarize_direct_land_oob(proc, valid_roots)
            warm_iterations.append(
                {
                    "iteration": iteration,
                    "elapsed_s": time.monotonic() - iter_started,
                    "warm_heap_sweep": False,
                    "validation_mode": "cached_address_vtable_handle_id_liveness",
                    "invalid_cached_roots": invalid_roots,
                    "invalid_cached_root_count": len(invalid_roots),
                    "summary": summary,
                }
            )
            if iteration + 1 < iterations and sleep_s > 0:
                time.sleep(sleep_s)

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
            },
            "warm_iterations": warm_iterations,
            "totals": {
                "warm_iterations": len(warm_iterations),
                "warm_heap_sweeps": sum(1 for item in warm_iterations if item["warm_heap_sweep"]),
                "warm_invalid_cached_roots": sum(item["invalid_cached_root_count"] for item in warm_iterations),
            },
            "offsets": {
                "army_vtable_rva": direct_oob_probe.ARMY_VTABLE_RVA,
                "army_group_vtable_rva": direct_oob_probe.AG_VTABLE_RVA,
                "theatre_vtable_rva": direct_oob_probe.THEATRE_VTABLE_RVA,
                "division_vtable_rva": direct_oob_probe.DIVISION_VTABLE_RVA,
                "army_member_count": direct_oob_probe.ARMY_OFF_MEMBER_COUNT,
                "army_orders_vector": direct_oob_probe.ARMY_OFF_ORDERS_VEC,
                "army_group_parent": direct_oob_probe.ARMY_OFF_AG_PARENT,
                "division_army_parent": direct_oob_probe.DIVISION_OFF_ARMY_PARENT,
                "order_type": direct_oob_probe.ORDER_OFF_TYPE,
                "order_army_backref": direct_oob_probe.ORDER_OFF_ARMY_BACKREF,
                "order_type5_states_begin": direct_oob_probe.ORDER_OFF_TYPE5_STATES_BEGIN,
                "order_type5_states_size": direct_oob_probe.ORDER_OFF_TYPE5_STATES_SIZE,
            },
            "production_position": (
                "validated bounded preflight plus cached-address warm-read contract; the warm phase "
                "does not sweep the heap, but it is not a DLL-only production stream until the helper "
                "is moved behind the shared collector registration slot"
            ),
            "errors": [],
        }
        payload["errors"] = validate_payload(payload)
        out_dir.mkdir(parents=True, exist_ok=True)
        (out_dir / "direct_oob_warm_cache_probe.json").write_text(
            json.dumps(payload, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        return payload
    finally:
        proc.close()


def serialize_roots(roots: dict[str, list[Any]]) -> dict[str, list[dict[str, Any]]]:
    return {
        key: [{"id": int(root.id), "addr": int(root.addr)} for root in rows]
        for key, rows in roots.items()
    }


def root_cache_summary(cached_roots: dict[str, list[dict[str, Any]]]) -> dict[str, Any]:
    return {
        key: {
            "rows": len(rows),
            "sample": [{"id": row["id"], "addr": f"0x{row['addr']:X}"} for row in rows[:8]],
        }
        for key, rows in cached_roots.items()
    }


def validate_cached_roots(
    proc: Any,
    cached_roots: dict[str, list[dict[str, Any]]],
) -> tuple[dict[str, list[Any]], list[dict[str, Any]]]:
    valid: dict[str, list[Any]] = {key: [] for key in cached_roots}
    invalid: list[dict[str, Any]] = []
    for key, rows in cached_roots.items():
        for row in rows:
            object_id = int(row["id"])
            addr = int(row["addr"])
            reason = validate_cached_root(proc, key, object_id, addr)
            if reason is None:
                valid[key].append(direct_oob_probe.ObjRoot(object_id, addr))
            else:
                invalid.append(
                    {
                        "kind": key,
                        "id": object_id,
                        "addr": f"0x{addr:X}",
                        "reason": reason,
                    }
                )
    return valid, invalid


def validate_cached_root(proc: Any, key: str, expected_id: int, addr: int) -> str | None:
    try:
        expected_vtable = proc.module_base + ROOT_RVAS[key]
        actual_vtable = proc.u64(addr)
    except OSError:
        return "unreadable_vtable"
    if actual_vtable != expected_vtable:
        return "vtable_mismatch"
    actual_id = direct_oob_probe.validate_root_hit(proc, key, addr)
    if actual_id is None:
        return "handle_or_type_validation_failed"
    if int(actual_id) != expected_id:
        return "object_id_mismatch"
    return None


def validate_payload(payload: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    preflight_counts = payload.get("preflight", {}).get("summary", {}).get("counts", {})
    if preflight_counts.get("direct_armies", 0) <= preflight_counts.get("active_armies_from_division_parents", 0):
        errors.append("preflight direct armies did not exceed active division-parent armies")
    if preflight_counts.get("empty_or_inactive_direct_armies", 0) <= 0:
        errors.append("preflight proved no empty/inactive direct armies")
    if preflight_counts.get("theatres", 0) <= 0:
        errors.append("preflight proved no theatres")
    if payload.get("totals", {}).get("warm_heap_sweeps") != 0:
        errors.append("warm phase performed a heap sweep")
    if payload.get("totals", {}).get("warm_invalid_cached_roots") != 0:
        errors.append("warm phase had invalid cached roots")
    if not payload.get("warm_iterations"):
        errors.append("no warm iterations recorded")
    for item in payload.get("warm_iterations", []):
        counts = item.get("summary", {}).get("counts", {})
        for key in ("direct_armies", "army_groups", "theatres"):
            if counts.get(key) != preflight_counts.get(key):
                errors.append(f"iteration {item.get('iteration')} {key} count drifted")
        if counts.get("empty_or_inactive_direct_armies", 0) <= 0:
            errors.append(f"iteration {item.get('iteration')} lost empty/inactive direct-army coverage")
        if item.get("validation_mode") != "cached_address_vtable_handle_id_liveness":
            errors.append(f"iteration {item.get('iteration')} used unexpected validation mode")
    return errors


if __name__ == "__main__":
    raise SystemExit(main())
