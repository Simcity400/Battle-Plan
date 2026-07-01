"""CSupplySystem root-path probe for domain 44 production hardening.

The shared collector currently finds the single `CSupplySystem` object by a
heap-wide vtable scan, then caches it. That works functionally but is too slow
inside a recorded speed-five frame. This lane-local probe looks for a bounded
root path that can replace in-frame scanning:

* direct module static pointer,
* direct `CGameState` pointer,
* one-hop `CGameState` pointer-region reference,
* country-object references,
* current-build `CCountrySupplySystem` object references back to `CSupplySystem`.

It is read-only and writes proof artifacts only.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import struct
import sys
import time
from collections import Counter
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Set, Tuple

LANE_DIR = Path(__file__).resolve().parent
REPO_ROOT = LANE_DIR.parents[2]
sys.path.insert(0, str(LANE_DIR))
sys.path.insert(0, str(REPO_ROOT / "memory_reader"))

import ccountry_supply_layout_probe as layout_probe  # noqa: E402
import live_probe  # noqa: E402
import winmem  # noqa: E402

DOMAIN_NAME = live_probe.DOMAIN_NAME
EXPECTED_SHA256 = live_probe.EXPECTED_SHA256
HEAP_LO = live_probe.HEAP_LO
HEAP_HI = live_probe.HEAP_HI
MAX_HEAP_REGION_BYTES = 64 << 20
MAX_MODULE_REGION_BYTES = 64 << 20
MAX_REPORTED_MATCHES = 48
MAX_COUNTRY_SCAN_BYTES = 0x5000
DEFAULT_GAMESTATE_SCAN_BYTES = 0x3000
DEFAULT_ONE_HOP_REGION_BYTES = 0x10000
CSUPPLY_OBJECT_SCAN_BYTES = 0x400
CCSS_SCAN_BYTES = 0x2A0
CCSS_CSUPPLY_BACKPTR = 0x70
CCSS_COUNTRY_BACKPTR = 0x78


def hx(value: Optional[int]) -> Optional[str]:
    return f"0x{value:X}" if isinstance(value, int) and value else None


def is_heap(value: Optional[int]) -> bool:
    return isinstance(value, int) and HEAP_LO <= value < HEAP_HI and value % 8 == 0


def safe_u64(p: Any, addr: int) -> Optional[int]:
    try:
        return p.u64(addr)
    except OSError:
        return None


def safe_u32(p: Any, addr: int) -> Optional[int]:
    try:
        return p.u32(addr)
    except OSError:
        return None


def scan_qword_in_blob(blob: bytes, value: int) -> List[int]:
    out: List[int] = []
    needle = struct.pack("<Q", value)
    search = 0
    while True:
        found = blob.find(needle, search)
        if found < 0:
            break
        search = found + 1
        if found % 8 == 0:
            out.append(found)
    return out


def validate_csupply_object(p: Any, object_ptr: Optional[int]) -> Dict[str, Any]:
    vtable0 = p.module_base + live_probe.CSUPPLYSYSTEM_VTABLE0_RVA
    vtable1 = p.module_base + live_probe.CSUPPLYSYSTEM_VTABLE1_RVA
    if not is_heap(object_ptr):
        return {"ok": False, "reason": "object_not_heap_aligned", "object": hx(object_ptr)}
    first = safe_u64(p, int(object_ptr))
    second = safe_u64(p, int(object_ptr) + 8)
    if first != vtable0 or second != vtable1:
        return {
            "ok": False,
            "reason": "vtable_mismatch",
            "object": hx(object_ptr),
            "vtable0": hx(first),
            "vtable1": hx(second),
            "expected_vtable0": f"module+0x{live_probe.CSUPPLYSYSTEM_VTABLE0_RVA:X}",
            "expected_vtable1": f"module+0x{live_probe.CSUPPLYSYSTEM_VTABLE1_RVA:X}",
        }
    array = safe_u64(p, int(object_ptr) + live_probe.CSUPPLYSYSTEM_PROVINCE_INPUT_A)
    count = safe_u32(p, int(object_ptr) + live_probe.CSUPPLYSYSTEM_PROVINCE_INPUT_A + 8)
    if count != live_probe.MAX_PROVINCE_ID + 1 or not is_heap(array):
        return {
            "ok": False,
            "reason": "province_input_a_invalid",
            "object": hx(object_ptr),
            "array": hx(array),
            "count": count,
        }
    probe = p.try_read(int(array), min(256, int(count) * 8))
    if len(probe) < 16:
        return {
            "ok": False,
            "reason": "province_input_a_unreadable",
            "object": hx(object_ptr),
            "array": hx(array),
            "count": count,
            "probe_len": len(probe),
        }
    return {
        "ok": True,
        "object": hx(object_ptr),
        "vtable0": f"module+0x{first - p.module_base:X}",
        "vtable1": f"module+0x{second - p.module_base:X}",
        "province_input_a": hx(array),
        "province_input_a_count": count,
    }


def find_csupply_by_vtable(p: Any) -> Tuple[Optional[int], float]:
    started = time.perf_counter()
    obj = layout_probe.find_csupply_object(p)
    return obj, time.perf_counter() - started


def module_static_pointer_scan(p: Any, object_ptr: int) -> Dict[str, Any]:
    matches: List[Dict[str, Any]] = []
    module_start = p.module_base
    module_end = p.module_base + p.module_size
    for base, size in p.iter_regions(module_start, module_end, want_exec=False):
        if size > MAX_MODULE_REGION_BYTES:
            continue
        blob = p.try_read(base, size)
        if not blob:
            continue
        for off in scan_qword_in_blob(blob, object_ptr):
            addr = base + off
            matches.append({
                "address": hx(addr),
                "rva": f"module+0x{addr - module_start:X}",
                "path": f"u64[module+0x{addr - module_start:X}]",
            })
            if len(matches) >= MAX_REPORTED_MATCHES:
                break
        if len(matches) >= MAX_REPORTED_MATCHES:
            break
    return {
        "matches": matches,
        "match_count_reported": len(matches),
        "verdict": "module_static_pointer_found" if matches else "rejected_no_module_static_pointer",
    }


def game_state_direct_scan(p: Any, game_state: int, object_ptr: int, scan_bytes: int) -> Dict[str, Any]:
    blob = p.try_read(game_state, scan_bytes)
    matches = [
        {
            "offset": f"+0x{off:X}",
            "address": hx(game_state + off),
            "path": f"u64[CGameState+0x{off:X}]",
        }
        for off in scan_qword_in_blob(blob, object_ptr)
    ]
    return {
        "scan_bytes": scan_bytes,
        "matches": matches[:MAX_REPORTED_MATCHES],
        "match_count": len(matches),
        "verdict": "cgame_state_direct_pointer_found" if matches else "rejected_no_cgame_state_direct_pointer",
    }


def game_state_one_hop_scan(
    p: Any,
    game_state: int,
    object_ptr: int,
    scan_bytes: int,
    region_bytes: int,
) -> Dict[str, Any]:
    body = p.try_read(game_state, scan_bytes)
    matches: List[Dict[str, Any]] = []
    pointer_fields = 0
    regions_read = 0
    seen_regions: Set[int] = set()
    for field_off in range(0, max(0, len(body) - 7), 8):
        ptr = struct.unpack_from("<Q", body, field_off)[0]
        if not is_heap(ptr) or ptr in seen_regions:
            continue
        seen_regions.add(ptr)
        pointer_fields += 1
        blob = p.try_read(ptr, region_bytes)
        if len(blob) < 8:
            continue
        regions_read += 1
        for inner_off in scan_qword_in_blob(blob, object_ptr):
            matches.append({
                "field_offset": f"+0x{field_off:X}",
                "region": hx(ptr),
                "region_offset": f"+0x{inner_off:X}",
                "path": f"u64[u64[CGameState+0x{field_off:X}]+0x{inner_off:X}]",
            })
            if len(matches) >= MAX_REPORTED_MATCHES:
                break
        if len(matches) >= MAX_REPORTED_MATCHES:
            break
    return {
        "game_state_scan_bytes": scan_bytes,
        "region_scan_bytes": region_bytes,
        "pointer_fields_considered": pointer_fields,
        "regions_read": regions_read,
        "matches": matches,
        "verdict": "cgame_state_one_hop_pointer_found" if matches else "rejected_no_cgame_state_one_hop_pointer",
    }


def country_direct_scan(
    p: Any,
    countries: Dict[int, int],
    id_to_tag: Dict[int, str],
    object_ptr: int,
) -> Dict[str, Any]:
    offset_counts: Counter[int] = Counter()
    matches: List[Dict[str, Any]] = []
    for tag_id, country in sorted(countries.items()):
        blob = p.try_read(country, MAX_COUNTRY_SCAN_BYTES)
        if len(blob) < 8:
            continue
        for off in scan_qword_in_blob(blob, object_ptr):
            offset_counts[off] += 1
            if len(matches) < MAX_REPORTED_MATCHES:
                matches.append({
                    "tag_db_id": tag_id,
                    "tag": id_to_tag.get(tag_id),
                    "country": hx(country),
                    "offset": f"+0x{off:X}",
                    "path": f"u64[CCountry({id_to_tag.get(tag_id)})+0x{off:X}]",
                })
    return {
        "scan_bytes_per_country": MAX_COUNTRY_SCAN_BYTES,
        "countries_scanned": len(countries),
        "offset_counts": [
            {"offset": f"+0x{off:X}", "matches": count}
            for off, count in offset_counts.most_common(MAX_REPORTED_MATCHES)
        ],
        "matches": matches,
        "verdict": "country_direct_csupply_pointer_found" if offset_counts else "rejected_no_country_direct_csupply_pointer",
    }


def ccss_backptr_validation(
    p: Any,
    object_ptr: int,
    ccss_objects: Sequence[int],
    country_to_tag: Dict[int, Tuple[int, str]],
) -> Dict[str, Any]:
    matches: List[Dict[str, Any]] = []
    offset_counts: Counter[int] = Counter()
    joined = 0
    for obj in ccss_objects:
        body = p.try_read(obj, CCSS_SCAN_BYTES)
        if len(body) < 8:
            continue
        country_ptr = safe_u64(p, obj + CCSS_COUNTRY_BACKPTR)
        if country_ptr in country_to_tag:
            joined += 1
        for off in scan_qword_in_blob(body, object_ptr):
            offset_counts[off] += 1
            if len(matches) < MAX_REPORTED_MATCHES:
                tag = None
                tag_id = None
                if country_ptr in country_to_tag:
                    tag_id, tag = country_to_tag[country_ptr]
                matches.append({
                    "ccountry_supply_system": hx(obj),
                    "offset": f"+0x{off:X}",
                    "tag_db_id": tag_id,
                    "tag": tag,
                    "path": f"u64[CCountrySupplySystem+0x{off:X}]",
                })
    return {
        "objects_scanned": len(ccss_objects),
        "country_joined_objects": joined,
        "offset_counts": [
            {"offset": f"+0x{off:X}", "matches": count}
            for off, count in offset_counts.most_common(MAX_REPORTED_MATCHES)
        ],
        "matches": matches,
        "verdict": "ccountry_supply_backptr_found" if offset_counts else "rejected_no_ccountry_supply_backptr",
    }


def measure_candidate_path(p: Any, candidate: Dict[str, Any], iterations: int = 500) -> Dict[str, Any]:
    path = candidate.get("path", "")
    started = time.perf_counter()
    ok = 0
    for _ in range(iterations):
        obj = None
        if path.startswith("u64[module+0x"):
            rva = int(path.split("+0x", 1)[1].split("]", 1)[0], 16)
            obj = safe_u64(p, p.module_base + rva)
        elif path.startswith("u64[CGameState+0x"):
            game_state = safe_u64(p, p.module_base + live_probe.CGAMESTATE_RVA)
            rva = int(path.split("+0x", 1)[1].split("]", 1)[0], 16)
            obj = safe_u64(p, int(game_state or 0) + rva)
        elif path.startswith("u64[u64[CGameState+0x"):
            pieces = path.split("+0x")
            outer = int(pieces[1].split("]", 1)[0], 16)
            inner = int(pieces[2].split("]", 1)[0], 16)
            game_state = safe_u64(p, p.module_base + live_probe.CGAMESTATE_RVA)
            region = safe_u64(p, int(game_state or 0) + outer)
            obj = safe_u64(p, int(region or 0) + inner)
        if validate_csupply_object(p, obj).get("ok"):
            ok += 1
    elapsed = time.perf_counter() - started
    return {
        "path": path,
        "iterations": iterations,
        "ok": ok,
        "elapsed_ms": round(elapsed * 1000, 3),
        "avg_us": round((elapsed * 1_000_000) / max(1, iterations), 3),
        "verdict": "fast_path_validated" if ok == iterations else "fast_path_unstable",
    }


def _write_outputs(out_dir: Path, report: Dict[str, Any]) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "csupply_root_path_probe.json").write_text(
        json.dumps(report, indent=2),
        encoding="utf-8",
    )
    summary = report.get("summary", {})
    lines = [
        "# supply_logistics_attrition_live CSupply root-path probe",
        "",
        f"- status: `{report.get('status')}`",
        f"- exe_sha256: `{report.get('process', {}).get('sha256')}`",
        f"- csupply_object: `{summary.get('csupply_object')}`",
        f"- slow_vtable_scan_ms: `{summary.get('slow_vtable_scan_ms')}`",
        f"- game_state: `{summary.get('game_state')}`",
        f"- live_date: `{summary.get('live_date')}`",
        "",
        "## Root Path Scans",
        "",
        f"- Module static pointer: `{report.get('module_static_scan', {}).get('verdict')}`",
        f"- CGameState direct pointer: `{report.get('game_state_direct_scan', {}).get('verdict')}`",
        f"- CGameState one-hop pointer: `{report.get('game_state_one_hop_scan', {}).get('verdict')}`",
        f"- CCountry direct pointer: `{report.get('country_direct_scan', {}).get('verdict')}`",
        f"- CCountrySupplySystem backpointer: `{report.get('ccss_backptr_validation', {}).get('verdict')}`",
        "",
        "## Validated Fast Paths",
        "",
    ]
    fast_paths = report.get("fast_path_measurements") or []
    if fast_paths:
        for item in fast_paths:
            lines.append(
                f"- `{item.get('path')}`: `{item.get('verdict')}`, "
                f"avg `{item.get('avg_us')}` us over `{item.get('iterations')}` reads."
            )
    else:
        lines.append("- none")
    lines.extend([
        "",
        "## Production Verdict",
        "",
    ])
    if report.get("status") == "csupply_root_path_validated":
        lines.append(
            "- A bounded root path to `CSupplySystem` was validated. Shared collector should use this path before falling back to any heap-wide vtable scan."
        )
    elif report.get("status") == "csupply_backptr_only_preflight_required":
        lines.append(
            "- No module/CGameState direct production path was found. `CCountrySupplySystem+0x70` remains a valid backpointer, but discovering `CCountrySupplySystem` still requires separate preflight or a root path."
        )
    else:
        lines.append(
            "- No bounded in-frame root path was validated; the safe production contract is to run heap discovery only as preflight/warm-cache before scheduled recording, then fail closed if the cache is absent or invalid."
        )
    (out_dir / "CSUPPLY_ROOT_PATH_PROBE.md").write_text(
        "\n".join(lines) + "\n",
        encoding="utf-8",
    )


def run(pid: Optional[int], out_dir: Path, game_state_scan_bytes: int, one_hop_region_bytes: int) -> Dict[str, Any]:
    started = time.time()
    p = winmem.Process(pid, "hoi4.exe") if pid is not None else winmem.open_process("hoi4.exe")
    report: Dict[str, Any] = {
        "lane": DOMAIN_NAME,
        "status": "fail_closed",
        "read_only": True,
        "writes_to_game_memory": False,
    }
    if p is None:
        report["reason"] = "hoi4.exe not running or cannot be opened"
        _write_outputs(out_dir, report)
        return report
    p.__enter__()
    try:
        fp = layout_probe.build_gate(p)
        report["process"] = fp
        if not fp.get("build_ok"):
            report["reason"] = "build mismatch; offsets are build-specific"
            _write_outputs(out_dir, report)
            return report
        countries = layout_probe.read_country_maps(p)
        game_state = countries.get("game_state")
        live_date = layout_probe.read_current_date(p, game_state)
        object_ptr, slow_scan_s = find_csupply_by_vtable(p)
        validation = validate_csupply_object(p, object_ptr)
        if not validation.get("ok"):
            report.update({
                "status": "csupply_control_object_not_found",
                "csupply_validation": validation,
            })
            _write_outputs(out_dir, report)
            return report

        module_scan = module_static_pointer_scan(p, int(object_ptr))
        direct_scan = game_state_direct_scan(p, int(game_state), int(object_ptr), game_state_scan_bytes) if is_heap(game_state) else {
            "verdict": "rejected_no_cgame_state",
            "matches": [],
        }
        one_hop = game_state_one_hop_scan(
            p,
            int(game_state),
            int(object_ptr),
            game_state_scan_bytes,
            one_hop_region_bytes,
        ) if is_heap(game_state) else {"verdict": "rejected_no_cgame_state", "matches": []}
        country_scan = country_direct_scan(
            p,
            countries["id_to_country"],
            countries["id_to_tag"],
            int(object_ptr),
        )
        ccss_objects = layout_probe.scan_ccountry_supply_objects(p)
        country_to_tag = {
            country: (tag_id, countries["id_to_tag"].get(tag_id, ""))
            for tag_id, country in countries["id_to_country"].items()
        }
        ccss_backptr = ccss_backptr_validation(p, int(object_ptr), ccss_objects, country_to_tag)

        fast_candidates: List[Dict[str, Any]] = []
        for source in (module_scan, direct_scan, one_hop):
            fast_candidates.extend(source.get("matches") or [])
        fast_measurements = [
            measure_candidate_path(p, candidate)
            for candidate in fast_candidates[:3]
        ]
        validated_fast_path = any(item.get("verdict") == "fast_path_validated" for item in fast_measurements)
        backptr_only = (
            not validated_fast_path
            and ccss_backptr.get("verdict") == "ccountry_supply_backptr_found"
        )
        report.update({
            "status": (
                "csupply_root_path_validated"
                if validated_fast_path
                else "csupply_backptr_only_preflight_required"
                if backptr_only
                else "csupply_root_path_blocked_preflight_required"
            ),
            "summary": {
                "csupply_object": hx(object_ptr),
                "slow_vtable_scan_ms": round(slow_scan_s * 1000, 3),
                "game_state": hx(game_state),
                "live_date": live_date.get("decoded"),
                "live_date_raw": live_date.get("raw"),
                "ccountry_supply_system_objects": len(ccss_objects),
                "validated_fast_path": validated_fast_path,
            },
            "csupply_validation": validation,
            "module_static_scan": module_scan,
            "game_state_direct_scan": direct_scan,
            "game_state_one_hop_scan": one_hop,
            "country_direct_scan": country_scan,
            "ccss_backptr_validation": ccss_backptr,
            "fast_path_measurements": fast_measurements,
        })
    except Exception as exc:  # noqa: BLE001
        report["status"] = "probe_failed"
        report["reason"] = str(exc)
    finally:
        p.__exit__()
    report["elapsed_s"] = round(time.time() - started, 3)
    _write_outputs(out_dir, report)
    return report


def default_out_dir() -> Path:
    stamp = _dt.datetime.now().strftime("%Y%m%d_%H%M%S")
    return REPO_ROOT / "replay_native" / "runs" / f"supply_logistics_attrition_live_csupply_root_{stamp}"


def main(argv: Optional[Iterable[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, default=None, help="hoi4.exe pid; defaults to process name lookup")
    parser.add_argument("--out-dir", type=Path, default=None, help="proof output directory")
    parser.add_argument("--game-state-scan-bytes", type=lambda value: int(value, 0), default=DEFAULT_GAMESTATE_SCAN_BYTES)
    parser.add_argument("--one-hop-region-bytes", type=lambda value: int(value, 0), default=DEFAULT_ONE_HOP_REGION_BYTES)
    args = parser.parse_args(list(argv) if argv is not None else None)
    out_dir = args.out_dir or default_out_dir()
    report = run(args.pid, out_dir, args.game_state_scan_bytes, args.one_hop_region_bytes)
    print(json.dumps({
        "status": report.get("status"),
        "out_dir": str(out_dir),
        "csupply_object": report.get("summary", {}).get("csupply_object"),
        "slow_vtable_scan_ms": report.get("summary", {}).get("slow_vtable_scan_ms"),
        "module_static": report.get("module_static_scan", {}).get("verdict"),
        "cgame_direct": report.get("game_state_direct_scan", {}).get("verdict"),
        "cgame_one_hop": report.get("game_state_one_hop_scan", {}).get("verdict"),
        "country_direct": report.get("country_direct_scan", {}).get("verdict"),
        "ccss_backptr": report.get("ccss_backptr_validation", {}).get("verdict"),
        "fast_paths": report.get("fast_path_measurements"),
        "elapsed_s": report.get("elapsed_s"),
    }, indent=2))
    return 0 if report.get("status") in (
        "csupply_root_path_validated",
        "csupply_backptr_only_preflight_required",
        "csupply_root_path_blocked_preflight_required",
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
