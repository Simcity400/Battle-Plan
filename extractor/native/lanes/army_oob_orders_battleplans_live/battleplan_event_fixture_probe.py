"""Live before/after observer for battleplan redraw/toggle fixtures.

This is lane-local, read-only fixture infrastructure. It reuses the current
build CArmy/order validators from the candidate probe, records stable order
identity before and after operator actions, and emits the observation JSON that
`battleplan_fixture_observer.py` converts into gateable fixture rows.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from datetime import datetime, timezone
import json
from pathlib import Path
import sys
from typing import Any


REPO = Path(__file__).resolve().parents[4]
MEMORY_READER = REPO / "extractor" / "memory_reader"
LANE_DIR = Path(__file__).resolve().parent
for path in (str(REPO), str(MEMORY_READER), str(LANE_DIR)):
    if path not in sys.path:
        sys.path.insert(0, path)

import battleplan_candidate_probe as candidate_probe  # type: ignore  # noqa: E402
import battleplan_fixture_observer as fixture_observer  # type: ignore  # noqa: E402
import direct_oob_probe  # type: ignore  # noqa: E402
import winmem  # type: ignore  # noqa: E402


OBSERVATION_SCHEMA = "army_oob_orders_battleplans_observation_v1"
READINESS_SCHEMA = "army_oob_orders_battleplans_event_readiness_v1"

DEFAULT_EXEC_BEGIN = 0x40
DEFAULT_EXEC_END = 0x220
EXCLUDED_EXECUTION_OFFSETS = {0x150}


@dataclass(frozen=True)
class WatchOrder:
    army_id: int
    army_addr: int
    order_addr: int
    slot_index: int
    order_type: int
    order_generation: int = 1


def main() -> int:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)

    prepare = sub.add_parser("prepare", help="scan the live process and write fixture readiness files")
    prepare.add_argument("--pid", type=int, default=45880)
    prepare.add_argument("--out-dir", type=Path, required=True)
    prepare.add_argument("--exec-begin", type=parse_int, default=DEFAULT_EXEC_BEGIN)
    prepare.add_argument("--exec-end", type=parse_int, default=DEFAULT_EXEC_END)

    observe = sub.add_parser("observe", help="wait for redraw/toggle actions and write observation/fixture JSON")
    observe.add_argument("--pid", type=int, default=45880)
    observe.add_argument("--out-dir", type=Path, required=True)
    observe.add_argument("--readiness", type=Path)
    observe.add_argument("--capture-manifest", type=Path)
    observe.add_argument("--capture-verified", action="store_true")
    observe.add_argument("--allow-rejection", action="store_true")
    observe.add_argument("--exec-begin", type=parse_int, default=DEFAULT_EXEC_BEGIN)
    observe.add_argument("--exec-end", type=parse_int, default=DEFAULT_EXEC_END)
    observe.add_argument("--max-execution-candidates", type=int, default=16)
    observe.add_argument(
        "--no-wait",
        action="store_true",
        help="take all three snapshots immediately; intended only for harness checks",
    )

    finalize = sub.add_parser("finalize", help="attach compact capture proof to an observation and build fixture JSON")
    finalize.add_argument("--observation", type=Path, required=True)
    finalize.add_argument("--out-dir", type=Path, required=True)
    finalize.add_argument("--capture-manifest", type=Path, required=True)
    finalize.add_argument("--capture-verified", action="store_true")
    finalize.add_argument("--allow-rejection", action="store_true")

    args = ap.parse_args()
    if args.cmd == "prepare":
        payload = prepare_readiness(args.pid, args.out_dir, exec_begin=args.exec_begin, exec_end=args.exec_end)
        print(json.dumps(compact_readiness(payload), indent=2, sort_keys=True))
        return 0 if payload["ready"] else 1
    if args.cmd == "observe":
        result = observe_actions(
            args.pid,
            args.out_dir,
            readiness_path=args.readiness,
            capture_manifest=args.capture_manifest,
            capture_verified=args.capture_verified,
            allow_rejection=args.allow_rejection,
            exec_begin=args.exec_begin,
            exec_end=args.exec_end,
            max_execution_candidates=args.max_execution_candidates,
            no_wait=args.no_wait,
        )
        print(json.dumps(compact_observe_result(result), indent=2, sort_keys=True))
        return 0 if result["ready_for_fixture_gate"] else 1
    if args.cmd == "finalize":
        result = finalize_observation(
            args.observation,
            args.out_dir,
            capture_manifest=args.capture_manifest,
            capture_verified=args.capture_verified,
            allow_rejection=args.allow_rejection,
        )
        print(json.dumps(compact_observe_result(result), indent=2, sort_keys=True))
        return 0 if result["ready_for_fixture_gate"] else 1
    raise AssertionError(args.cmd)


def prepare_readiness(pid: int, out_dir: Path, *, exec_begin: int, exec_end: int) -> dict[str, Any]:
    out_dir.mkdir(parents=True, exist_ok=True)
    with open_process(pid) as proc:
        roots, scan_stats = direct_oob_probe.enum_current_oob_roots(proc)
        orders = candidate_probe.enumerate_validated_orders(proc, roots["army"])
        geometry_targets = choose_geometry_targets(proc, orders)
        execution_orders = choose_execution_orders(orders, geometry_targets)
        payload = {
            "schema": READINESS_SCHEMA,
            "created_utc": utc_now(),
            "pid": pid,
            "out_dir": str(out_dir),
            "exe_sha256": checked_sha(proc),
            "module_base": proc.module_base,
            "scan_stats": scan_stats,
            "order_summary": candidate_probe.summarize_orders(orders),
            "ready": bool(geometry_targets and execution_orders),
            "geometry_targets": geometry_targets,
            "execution_scan": {
                "orders": [watch_order_to_json(order) for order in execution_orders],
                "offset_begin": f"+0x{exec_begin:X}",
                "offset_end_exclusive": f"+0x{exec_end:X}",
                "excluded_offsets": [f"+0x{offset:X}" for offset in sorted(EXCLUDED_EXECUTION_OFFSETS)],
                "excluded_reason": "+0x150 is already rejected for current build as is_executing u8",
            },
            "operator_steps": [
                "Start the simultaneous compact collector proof if a gate-ready fixture is required.",
                "Run the observe command and wait for the first prompt.",
                "Redraw or resize the listed non-type-5 order while preserving the selected order identity.",
                "Press Enter in the observer.",
                "Toggle battleplan execution for the listed army.",
                "Press Enter in the observer again.",
            ],
            "observe_command": observe_command(out_dir, pid),
            "finalize_command": finalize_command(out_dir),
            "compact_capture_command": compact_capture_command(out_dir, pid),
        }
    write_json(out_dir / "fixture_readiness.json", payload)
    write_text(out_dir / "run_observer.ps1", payload["observe_command"] + "\n")
    write_text(out_dir / "RUNBOOK.md", readiness_runbook(payload))
    return payload


def observe_actions(
    pid: int,
    out_dir: Path,
    *,
    readiness_path: Path | None,
    capture_manifest: Path | None,
    capture_verified: bool,
    allow_rejection: bool,
    exec_begin: int,
    exec_end: int,
    max_execution_candidates: int,
    no_wait: bool,
) -> dict[str, Any]:
    out_dir.mkdir(parents=True, exist_ok=True)
    readiness = load_or_prepare_readiness(pid, out_dir, readiness_path, exec_begin=exec_begin, exec_end=exec_end)
    geometry_orders = watch_orders_from_geometry_targets(readiness.get("geometry_targets", []))
    execution_orders = [watch_order_from_json(row) for row in readiness.get("execution_scan", {}).get("orders", [])]
    if not geometry_orders or not execution_orders:
        raise RuntimeError("readiness has no geometry target or execution scan order")

    with open_process(pid) as proc:
        sha = checked_sha(proc)
        initial = snapshot(proc, geometry_orders, execution_orders, exec_begin=exec_begin, exec_end=exec_end)
        write_json(out_dir / "before_snapshot.json", initial)
        if not no_wait:
            input("Redraw/resize the listed non-type-5 order, then press Enter...")
        after_geometry = snapshot(proc, geometry_orders, execution_orders, exec_begin=exec_begin, exec_end=exec_end)
        write_json(out_dir / "after_geometry_snapshot.json", after_geometry)
        if not no_wait:
            input("Toggle battleplan execution for the listed army, then press Enter...")
        after_toggle = snapshot(proc, geometry_orders, execution_orders, exec_begin=exec_begin, exec_end=exec_end)
        write_json(out_dir / "after_toggle_snapshot.json", after_toggle)

    capture = capture_from_manifest(capture_manifest, capture_verified) if capture_manifest else observer_only_capture()
    observation = build_observation(
        sha,
        capture,
        initial,
        after_geometry,
        after_toggle,
        max_execution_candidates=max_execution_candidates,
    )
    write_json(out_dir / "battleplan_observation.json", observation)
    fixture, errors = fixture_observer.build_fixture(observation, allow_rejection=allow_rejection)
    if fixture is not None:
        write_json(out_dir / "battleplan_fixture.json", fixture)
    gate_result = {
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "allow_rejection": allow_rejection,
        "observation_json": str(out_dir / "battleplan_observation.json"),
        "fixture_json": str(out_dir / "battleplan_fixture.json"),
    }
    write_json(out_dir / "fixture_gate_result.json", gate_result)
    return {
        "ready_for_fixture_gate": not errors,
        "errors": errors,
        "observation": observation,
        "fixture": fixture,
        "gate_result": gate_result,
    }


def finalize_observation(
    observation_path: Path,
    out_dir: Path,
    *,
    capture_manifest: Path,
    capture_verified: bool,
    allow_rejection: bool,
) -> dict[str, Any]:
    out_dir.mkdir(parents=True, exist_ok=True)
    observation = json.loads(observation_path.read_text(encoding="utf-8"))
    observation["capture"] = capture_from_manifest(capture_manifest, capture_verified)
    write_json(out_dir / "battleplan_observation_with_capture.json", observation)
    fixture, errors = fixture_observer.build_fixture(observation, allow_rejection=allow_rejection)
    if fixture is not None:
        write_json(out_dir / "battleplan_fixture.json", fixture)
    gate_result = {
        "status": "PASS" if not errors else "FAIL",
        "errors": errors,
        "allow_rejection": allow_rejection,
        "observation_json": str(out_dir / "battleplan_observation_with_capture.json"),
        "fixture_json": str(out_dir / "battleplan_fixture.json"),
    }
    write_json(out_dir / "fixture_gate_result.json", gate_result)
    return {
        "ready_for_fixture_gate": not errors,
        "errors": errors,
        "observation": observation,
        "fixture": fixture,
        "gate_result": gate_result,
    }


def choose_geometry_targets(proc: Any, orders: list[Any]) -> list[dict[str, Any]]:
    targets: list[dict[str, Any]] = []
    for candidate in candidate_probe.GEOMETRY_CANDIDATES:
        best = None
        for order in orders:
            if order.order_type not in set(candidate["applies_to"]):
                continue
            decoded = candidate_probe.read_vector(
                proc,
                order.order_addr,
                candidate["begin_off"],
                candidate["size_off"],
                candidate_probe.MAX_PROVINCE_ID,
            )
            if decoded.get("valid") and decoded.get("values"):
                best = (order, decoded)
                break
        if best is None:
            continue
        order, decoded = best
        targets.append(
            {
                "candidate_name": candidate["name"],
                "kind": "geometry",
                "field_name": field_name_for_candidate(candidate),
                "field_type": candidate["field_type"],
                "begin_offset": f"+0x{candidate['begin_off']:X}",
                "size_offset": f"+0x{candidate['size_off']:X}",
                "watch_order": watch_order_to_json(
                    WatchOrder(
                        army_id=order.army_id,
                        army_addr=order.army_addr,
                        order_addr=order.order_addr,
                        slot_index=order.slot_index,
                        order_type=order.order_type,
                    )
                ),
                "current_value": decoded.get("values", []),
                "current_count": decoded.get("count"),
                "operator_action": "redraw_non_type5_order",
            }
        )
    return targets


def choose_execution_orders(orders: list[Any], geometry_targets: list[dict[str, Any]]) -> list[WatchOrder]:
    preferred_army_ids = {
        int(target["watch_order"]["army_id"])
        for target in geometry_targets
        if isinstance(target.get("watch_order"), dict)
    }
    selected: list[WatchOrder] = []
    for order in orders:
        if preferred_army_ids and order.army_id not in preferred_army_ids:
            continue
        selected.append(
            WatchOrder(
                army_id=order.army_id,
                army_addr=order.army_addr,
                order_addr=order.order_addr,
                slot_index=order.slot_index,
                order_type=order.order_type,
            )
        )
    if selected:
        return selected[:16]
    return [
        WatchOrder(
            army_id=order.army_id,
            army_addr=order.army_addr,
            order_addr=order.order_addr,
            slot_index=order.slot_index,
            order_type=order.order_type,
        )
        for order in orders[:16]
    ]


def snapshot(
    proc: Any,
    geometry_orders: list[WatchOrder],
    execution_orders: list[WatchOrder],
    *,
    exec_begin: int,
    exec_end: int,
) -> dict[str, Any]:
    geometry = []
    for watch in geometry_orders:
        if not revalidate_order(proc, watch):
            geometry.append({"watch_order": watch_order_to_json(watch), "valid": False, "reason": "identity_mismatch"})
            continue
        geometry.append({"watch_order": watch_order_to_json(watch), "valid": True, "candidates": read_geometry(proc, watch)})
    execution = []
    for watch in execution_orders:
        if not revalidate_order(proc, watch):
            execution.append({"watch_order": watch_order_to_json(watch), "valid": False, "reason": "identity_mismatch"})
            continue
        execution.append(
            {
                "watch_order": watch_order_to_json(watch),
                "valid": True,
                "u8_window": read_u8_window(proc, watch.order_addr, exec_begin, exec_end),
            }
        )
    return {
        "captured_utc": utc_now(),
        "geometry": geometry,
        "execution": execution,
    }


def read_geometry(proc: Any, watch: WatchOrder) -> list[dict[str, Any]]:
    out = []
    for candidate in candidate_probe.GEOMETRY_CANDIDATES:
        if watch.order_type not in set(candidate["applies_to"]):
            continue
        decoded = candidate_probe.read_vector(
            proc,
            watch.order_addr,
            candidate["begin_off"],
            candidate["size_off"],
            candidate_probe.MAX_PROVINCE_ID,
        )
        out.append(
            {
                "candidate_name": candidate["name"],
                "field_name": field_name_for_candidate(candidate),
                "field_type": candidate["field_type"],
                "begin_offset": f"+0x{candidate['begin_off']:X}",
                "size_offset": f"+0x{candidate['size_off']:X}",
                "valid": bool(decoded.get("valid")),
                "reason": decoded.get("reason"),
                "value": decoded.get("values", []),
                "count": decoded.get("count"),
            }
        )
    return out


def read_u8_window(proc: Any, order_addr: int, begin: int, end: int) -> dict[str, int]:
    values: dict[str, int] = {}
    for offset in range(begin, end):
        if offset in EXCLUDED_EXECUTION_OFFSETS:
            continue
        try:
            values[f"+0x{offset:X}"] = int(proc.u8(order_addr + offset))
        except OSError:
            continue
    return values


def build_observation(
    exe_sha256: str,
    capture: dict[str, Any],
    initial: dict[str, Any],
    after_geometry: dict[str, Any],
    after_toggle: dict[str, Any],
    *,
    max_execution_candidates: int,
) -> dict[str, Any]:
    rows: list[dict[str, Any]] = []
    rows.extend(geometry_observation_rows(initial, after_geometry))
    rows.extend(execution_observation_rows(initial, after_geometry, after_toggle, max_rows=max_execution_candidates))
    return {
        "schema": OBSERVATION_SCHEMA,
        "exe_sha256": exe_sha256,
        "created_utc": utc_now(),
        "capture": capture,
        "operator_actions": ["redraw_non_type5_order", "toggle_battleplan_execution"],
        "observations": rows,
        "observer_notes": {
            "execution_scan_excluded_offsets": [f"+0x{offset:X}" for offset in sorted(EXCLUDED_EXECUTION_OFFSETS)],
            "execution_scan_rule": "exact candidates must stay unchanged across geometry redraw and change across execution toggle",
        },
    }


def geometry_observation_rows(initial: dict[str, Any], after_geometry: dict[str, Any]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    after_by_order = geometry_by_order(after_geometry)
    for before_entry in initial.get("geometry", []):
        watch = before_entry.get("watch_order", {})
        after_entry = after_by_order.get(order_key(watch))
        if not before_entry.get("valid") or not after_entry or not after_entry.get("valid"):
            continue
        after_candidates = {row.get("candidate_name"): row for row in after_entry.get("candidates", [])}
        for before_candidate in before_entry.get("candidates", []):
            if not before_candidate.get("valid"):
                continue
            after_candidate = after_candidates.get(before_candidate.get("candidate_name"))
            if not after_candidate or not after_candidate.get("valid"):
                continue
            rows.append(
                {
                    "kind": "geometry",
                    "operator_action": "redraw_non_type5_order",
                    "source_kind": "live_fixture",
                    "army_id": int(watch["army_id"]),
                    "slot_index": int(watch["slot_index"]),
                    "order_type": int(watch["order_type"]),
                    "order_generation": int(watch.get("order_generation", 1)),
                    "candidate": before_candidate["begin_offset"],
                    "field_name": before_candidate["field_name"],
                    "field_type": before_candidate["field_type"],
                    "validated_offsets": [
                        {
                            "name": before_candidate["field_name"],
                            "offset": before_candidate["begin_offset"],
                            "type": before_candidate["field_type"],
                            "evidence": "redraw fixture sampled vector begin pointer before and after stable order identity",
                        },
                        {
                            "name": before_candidate["field_name"] + "_count",
                            "offset": before_candidate["size_offset"],
                            "type": "u32",
                            "evidence": "redraw fixture sampled vector count before and after stable order identity",
                        },
                    ],
                    "before": {"value": before_candidate.get("value", [])},
                    "after": {"value": after_candidate.get("value", [])},
                    "live_identity_evidence": identity_evidence(watch),
                }
            )
    return rows


def execution_observation_rows(
    initial: dict[str, Any],
    after_geometry: dict[str, Any],
    after_toggle: dict[str, Any],
    *,
    max_rows: int,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    geom_by_order = execution_by_order(after_geometry)
    toggle_by_order = execution_by_order(after_toggle)
    for before_entry in initial.get("execution", []):
        watch = before_entry.get("watch_order", {})
        after_geom = geom_by_order.get(order_key(watch))
        after_tog = toggle_by_order.get(order_key(watch))
        if not before_entry.get("valid") or not after_geom or not after_tog:
            continue
        before_values = before_entry.get("u8_window", {})
        geom_values = after_geom.get("u8_window", {})
        toggle_values = after_tog.get("u8_window", {})
        for offset in sorted(set(before_values) & set(geom_values) & set(toggle_values), key=parse_offset):
            before = before_values[offset]
            after_redraw = geom_values[offset]
            after_execution = toggle_values[offset]
            if before != after_redraw:
                continue
            if after_redraw == after_execution:
                continue
            rows.append(
                {
                    "kind": "execution",
                    "operator_action": "toggle_battleplan_execution",
                    "source_kind": "live_fixture",
                    "army_id": int(watch["army_id"]),
                    "slot_index": int(watch["slot_index"]),
                    "order_type": int(watch["order_type"]),
                    "order_generation": int(watch.get("order_generation", 1)),
                    "candidate": offset,
                    "field_name": "execution_u8_" + offset[1:].lower(),
                    "field_type": "u8",
                    "validated_offsets": [
                        {
                            "name": "execution_u8_" + offset[1:].lower(),
                            "offset": offset,
                            "type": "u8",
                            "evidence": "unchanged across redraw, changed after battleplan execution toggle on stable order identity",
                        }
                    ],
                    "before": {"value": after_redraw},
                    "after": {"value": after_execution},
                    "live_identity_evidence": identity_evidence(watch),
                }
            )
            if len(rows) >= max_rows:
                return rows
    return rows


def capture_from_manifest(path: Path, capture_verified: bool) -> dict[str, Any]:
    manifest = json.loads(path.read_text(encoding="utf-8"))
    compact = manifest.get("compact_stream") or {}
    failures = {
        "source_unavailable": compact.get("source_unavailable_seen") is True,
        "partial_data": compact.get("partial_data_seen") is True,
        "bridge_unavailable": compact.get("bridge_unavailable_seen") is True,
        "worker_panic": compact.get("worker_panic_seen") is True,
    }
    return {
        "dll_only": manifest.get("division_root_source") == "dll_native",
        "compact_stream": manifest.get("stream_format") == "compact" and compact.get("ordered") is True,
        "rust_decoded": compact.get("decodable") is True,
        "hoi4replay_verified": bool(capture_verified),
        "zero_skipped_hours": manifest.get("skipped_game_hours") == 0,
        "no_failures": not any(failures.values()),
        **failures,
        "game_hours_advanced": int(manifest.get("game_hours_advanced", 0)),
        "max_frame_hour_gap": 1 if (manifest.get("hourly_requirement") or {}).get("met") is True else 9999,
        "capture_manifest": str(path),
    }


def observer_only_capture() -> dict[str, Any]:
    return {
        "dll_only": False,
        "compact_stream": False,
        "rust_decoded": False,
        "hoi4replay_verified": False,
        "zero_skipped_hours": False,
        "no_failures": False,
        "source_unavailable": None,
        "partial_data": None,
        "bridge_unavailable": None,
        "worker_panic": None,
        "game_hours_advanced": 0,
        "max_frame_hour_gap": 9999,
        "capture_source": "read_only_live_observer_without_compact_capture_manifest",
    }


def open_process(pid: int) -> Any:
    proc = winmem.Process(pid, candidate_probe.PROC_NAME)
    try:
        checked_sha(proc)
    except BaseException:
        proc.close()
        raise
    return proc


def checked_sha(proc: Any) -> str:
    sha = str((proc.fingerprint().get("sha256") or "")).lower()
    if sha != candidate_probe.EXPECTED_SHA256:
        raise RuntimeError(f"build gate failed: {sha} != {candidate_probe.EXPECTED_SHA256}")
    return sha


def revalidate_order(proc: Any, watch: WatchOrder) -> bool:
    try:
        order_vtable = proc.module_base + direct_oob_probe.ORDER_VTABLE_RVA
        return (
            proc.u64(watch.order_addr) == order_vtable
            and proc.u64(watch.order_addr + direct_oob_probe.ORDER_OFF_ARMY_BACKREF) == watch.army_addr
            and proc.i32(watch.order_addr + direct_oob_probe.ORDER_OFF_TYPE) == watch.order_type
        )
    except OSError:
        return False


def load_or_prepare_readiness(
    pid: int,
    out_dir: Path,
    readiness_path: Path | None,
    *,
    exec_begin: int,
    exec_end: int,
) -> dict[str, Any]:
    if readiness_path is not None:
        return json.loads(readiness_path.read_text(encoding="utf-8"))
    return prepare_readiness(pid, out_dir, exec_begin=exec_begin, exec_end=exec_end)


def watch_orders_from_geometry_targets(targets: list[dict[str, Any]]) -> list[WatchOrder]:
    orders = []
    seen = set()
    for target in targets:
        watch = watch_order_from_json(target["watch_order"])
        key = (watch.army_id, watch.slot_index, watch.order_addr)
        if key not in seen:
            seen.add(key)
            orders.append(watch)
    return orders


def geometry_by_order(snapshot_payload: dict[str, Any]) -> dict[tuple[int, int, int], dict[str, Any]]:
    return {order_key(row.get("watch_order", {})): row for row in snapshot_payload.get("geometry", [])}


def execution_by_order(snapshot_payload: dict[str, Any]) -> dict[tuple[int, int, int], dict[str, Any]]:
    return {order_key(row.get("watch_order", {})): row for row in snapshot_payload.get("execution", [])}


def order_key(watch: dict[str, Any]) -> tuple[int, int, int]:
    return (int(watch.get("army_id", 0)), int(watch.get("slot_index", -1)), parse_offset(str(watch.get("order_addr", "0"))))


def watch_order_to_json(order: WatchOrder) -> dict[str, Any]:
    return {
        "army_id": order.army_id,
        "army_addr": f"0x{order.army_addr:X}",
        "order_addr": f"0x{order.order_addr:X}",
        "slot_index": order.slot_index,
        "order_type": order.order_type,
        "order_generation": order.order_generation,
    }


def watch_order_from_json(row: dict[str, Any]) -> WatchOrder:
    return WatchOrder(
        army_id=int(row["army_id"]),
        army_addr=parse_offset(str(row["army_addr"])),
        order_addr=parse_offset(str(row["order_addr"])),
        slot_index=int(row["slot_index"]),
        order_type=int(row["order_type"]),
        order_generation=int(row.get("order_generation", 1)),
    )


def field_name_for_candidate(candidate: dict[str, Any]) -> str:
    if candidate["name"] == "old_type3_secondary_vector_plus_0xC0":
        return "type3_secondary_province_path"
    return "province_path"


def identity_evidence(watch: dict[str, Any]) -> dict[str, Any]:
    return {
        "army_addr": watch.get("army_addr"),
        "order_addr": watch.get("order_addr"),
        "identity_rule": "order vtable, army backref, slot index, order type, and pointer stayed stable across snapshots",
    }


def observe_command(out_dir: Path, pid: int) -> str:
    readiness = out_dir / "fixture_readiness.json"
    return (
        "python extractor\native\\lanes\\army_oob_orders_battleplans_live\\battleplan_event_fixture_probe.py observe "
        f"--pid {pid} --readiness {quote_path(readiness)} --out-dir {quote_path(out_dir)} --allow-rejection"
    )


def finalize_command(out_dir: Path) -> str:
    return (
        "python extractor\native\\lanes\\army_oob_orders_battleplans_live\\battleplan_event_fixture_probe.py finalize "
        f"--observation {quote_path(out_dir / 'battleplan_observation.json')} "
        f"--out-dir {quote_path(out_dir)} "
        f"--capture-manifest {quote_path(out_dir / 'compact_capture' / 'native_session_manifest.json')} "
        "--capture-verified --allow-rejection"
    )


def compact_capture_command(out_dir: Path, pid: int) -> str:
    capture_dir = out_dir / "compact_capture"
    return (
        "extractor\native\\target\\release\\collector_record.exe "
        f"--pid {pid} "
        "--dll extractor\native\\target\\release\\collector.dll "
        f"--out-dir {quote_path(capture_dir)} "
        "--duration-s 90 --sample-ms 0 --min-game-hours 1 "
        "--scheduled-domain army_orders --scheduled-domain oob_command "
        "--stream-format compact --extraction-perf-gate --lightweight-root-frame"
    )


def readiness_runbook(payload: dict[str, Any]) -> str:
    return "\n".join(
        [
            "# battleplan event fixture readiness",
            "",
            f"PID: `{payload['pid']}`",
            f"Executable SHA-256: `{payload['exe_sha256']}`",
            "",
            "## Live targets",
            "",
            f"- geometry targets: `{len(payload.get('geometry_targets', []))}`",
            f"- execution scan orders: `{len((payload.get('execution_scan') or {}).get('orders', []))}`",
            "- execution scan excludes `+0x150`; that offset is already rejected as `is_executing: u8`.",
            "",
            "## Terminal A: compact capture",
            "",
            "```powershell",
            payload["compact_capture_command"],
            "```",
            "",
            "## Terminal B: action observer",
            "",
            "Run this while Terminal A is capturing, then follow the two prompts:",
            "",
            "```powershell",
            payload["observe_command"],
            "```",
            "",
            "## After capture finishes",
            "",
            "Verify the compact replay, then attach the capture manifest to the observation:",
            "",
            "```powershell",
            "$capture = '" + str(Path(payload["out_dir"]) / "compact_capture") + "'",
            "$manifest = Get-Content \"$capture\\native_session_manifest.json\" | ConvertFrom-Json",
            "extractor\native\\target\\release\\pack_replay.exe verify \"$capture\\native_session.battleplan\" $capture $manifest.replay_game_id",
            payload["finalize_command"],
            "```",
            "",
        ]
    )


def compact_readiness(payload: dict[str, Any]) -> dict[str, Any]:
    return {
        "status": "READY" if payload.get("ready") else "NOT_READY",
        "pid": payload.get("pid"),
        "geometry_target_count": len(payload.get("geometry_targets", [])),
        "execution_scan_order_count": len((payload.get("execution_scan") or {}).get("orders", [])),
        "readiness": "fixture_readiness.json",
        "observe_command": payload.get("observe_command"),
        "finalize_command": payload.get("finalize_command"),
    }


def compact_observe_result(result: dict[str, Any]) -> dict[str, Any]:
    return {
        "status": "PASS" if result.get("ready_for_fixture_gate") else "FAIL",
        "errors": result.get("errors", []),
        "observation_rows": len((result.get("observation") or {}).get("observations", [])),
        "ready_for_fixture_gate": bool(result.get("ready_for_fixture_gate")),
    }


def parse_int(value: str) -> int:
    return int(value, 0)


def parse_offset(value: str) -> int:
    return int(value, 16) if value.lower().startswith("0x") or value.lower().startswith("+0x") else int(value, 0)


def quote_path(path: Path) -> str:
    return "'" + str(path).replace("'", "''") + "'"


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


if __name__ == "__main__":
    raise SystemExit(main())
