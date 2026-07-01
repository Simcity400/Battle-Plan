"""CProvince residual probe for exact supply candidates on domain 44.

This is lane-local, read-only, and current-build gated. It is not a production
emitter. Its job is to prevent stale supply-offset claims from leaking into the
compact stream and to produce an anchor-gated candidate inventory for the exact
per-province supply tooltip value.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import math
import struct
import sys
import time
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

LANE_DIR = Path(__file__).resolve().parent
REPO_ROOT = LANE_DIR.parents[3]
sys.path.insert(0, str(LANE_DIR))
sys.path.insert(0, str(REPO_ROOT / "extractor" / "memory_reader"))

import live_probe  # noqa: E402
import winmem  # noqa: E402

EXPECTED_SHA256 = live_probe.EXPECTED_SHA256
DOMAIN_NAME = live_probe.DOMAIN_NAME
MAX_PROVINCE_ID = live_probe.MAX_PROVINCE_ID
MAX_SUPPLY_RAW = live_probe.MAX_SUPPLY_RAW
FIXED_SCALE_RAW = live_probe.FIXED_SCALE_RAW
HEAP_LO = live_probe.HEAP_LO
HEAP_HI = live_probe.HEAP_HI

STALE_16022851_CHAIN = {
    "name": "stale_16022851_remaining_body_0x190_0x348_0x108",
    "first_offset": 0x190,
    "second_offset": 0x348,
    "value_offset": 0x108,
    "old_build": "16022851eaa571728aa3a0b5fd5f990a56da57e2249a4c1b9cfb30336f59b120",
}

CURRENT_STRUCTURE_CHAINS = [
    ("cprovince_2c8_218", 0x2C8, 0x218),
    ("cprovince_2c8_230", 0x2C8, 0x230),
    ("cprovince_2c8_2f8", 0x2C8, 0x2F8),
    ("cprovince_2c8_310", 0x2C8, 0x310),
    ("cprovince_2c8_348", 0x2C8, 0x348),
    ("cprovince_2c8_3d8", 0x2C8, 0x3D8),
    ("cprovince_2c8_428", 0x2C8, 0x428),
]

DEFAULT_SAMPLE_PROVINCES = [
    1,
    100,
    500,
    1000,
    2000,
    3324,
    3544,
    6421,
    9244,
    12000,
    13413,
]

LEAF_SCAN_BYTES = 0x400
MIN_CANDIDATE_DISTINCT_VALUES = 5


def _is_heap(value: Optional[int]) -> bool:
    return value is not None and HEAP_LO <= value < HEAP_HI and value % 8 == 0


def _load_anchors(path: Optional[Path]) -> List[Dict[str, Any]]:
    if path is None:
        return []
    data = json.loads(path.read_text(encoding="utf-8"))
    anchors = data.get("anchors", data if isinstance(data, list) else [])
    out: List[Dict[str, Any]] = []
    for item in anchors:
        province_id = int(item["province_id"])
        if province_id <= 0 or province_id > MAX_PROVINCE_ID:
            raise ValueError(f"invalid anchor province_id {province_id}")
        rec = dict(item)
        rec["province_id"] = province_id
        out.append(rec)
    return out


def _province_sample_ids(anchors: Sequence[Dict[str, Any]], extra: Sequence[int]) -> List[int]:
    ids = list(DEFAULT_SAMPLE_PROVINCES)
    ids.extend(int(value) for value in extra)
    ids.extend(int(anchor["province_id"]) for anchor in anchors)
    out = []
    seen = set()
    for province_id in ids:
        if province_id <= 0 or province_id > MAX_PROVINCE_ID or province_id in seen:
            continue
        seen.add(province_id)
        out.append(province_id)
    return out


def _resolve_provinces(
    p: Any,
    module_base: int,
    game_state: int,
    province_ids: Sequence[int],
) -> Tuple[List[Dict[str, Any]], Dict[str, Any]]:
    province_array = p.u64(game_state + live_probe.CGAMESTATE_CPROVINCE_ARRAY)
    province_count = p.u32(game_state + live_probe.CGAMESTATE_CPROVINCE_COUNT)
    if province_count == 0 or province_count > MAX_PROVINCE_ID + 1:
        raise RuntimeError(f"invalid province_count {province_count}")
    if not _is_heap(province_array):
        raise RuntimeError(f"invalid CProvince array 0x{province_array:X}")

    cprovince_vtable = module_base + live_probe.CPROVINCE_VTABLE_RVA
    resolved: List[Dict[str, Any]] = []
    rejected: List[Dict[str, Any]] = []
    for province_id in province_ids:
        try:
            province_ptr = p.u64(province_array + province_id * 8)
            if not _is_heap(province_ptr):
                rejected.append({"province_id": province_id, "reason": "not_heap"})
                continue
            if p.u64(province_ptr) != cprovince_vtable:
                rejected.append({"province_id": province_id, "reason": "bad_vtable"})
                continue
            if p.u32(province_ptr + live_probe.CPROVINCE_OFF_ID) != province_id:
                rejected.append({"province_id": province_id, "reason": "id_mismatch"})
                continue
        except OSError as exc:
            rejected.append({"province_id": province_id, "reason": f"read_error:{exc}"})
            continue
        resolved.append({"province_id": province_id, "address": province_ptr})
    summary = {
        "requested": len(province_ids),
        "resolved": len(resolved),
        "rejected": rejected,
        "province_array": f"0x{province_array:X}",
        "province_count": province_count,
    }
    return resolved, summary


def _read_chain_leaf(
    p: Any,
    province: Dict[str, Any],
    first_offset: int,
    second_offset: int,
) -> Dict[str, Any]:
    province_id = int(province["province_id"])
    base = int(province["address"])
    try:
        first = p.u64(base + first_offset)
    except OSError as exc:
        return {"province_id": province_id, "status": "first_read_error", "reason": str(exc)}
    if not _is_heap(first):
        return {
            "province_id": province_id,
            "status": "first_not_heap",
            "first": f"0x{first:X}" if first else None,
        }
    try:
        second = p.u64(first + second_offset)
    except OSError as exc:
        return {
            "province_id": province_id,
            "status": "second_read_error",
            "first": f"0x{first:X}",
            "reason": str(exc),
        }
    if not _is_heap(second):
        return {
            "province_id": province_id,
            "status": "second_not_heap",
            "first": f"0x{first:X}",
            "second": f"0x{second:X}" if second else None,
        }
    return {
        "province_id": province_id,
        "status": "leaf",
        "first": f"0x{first:X}",
        "second": f"0x{second:X}",
        "leaf": second,
    }


def _read_i32_at(p: Any, leaf: int, value_offset: int) -> Optional[int]:
    try:
        return int(p.i32(leaf + value_offset))
    except OSError:
        return None


def _classify_i32_series(values: Sequence[Optional[int]]) -> Dict[str, Any]:
    present = [value for value in values if value is not None]
    if not present:
        return {"status": "unreadable", "values": 0, "verdict": "rejected_no_reads"}
    finite = [
        value for value in present
        if isinstance(value, int) and -2_147_483_648 <= value <= 2_147_483_647
    ]
    negative = sum(1 for value in finite if value < 0)
    gt_supply = sum(1 for value in finite if value > MAX_SUPPLY_RAW)
    nonzero = sum(1 for value in finite if value != 0)
    distinct = len(set(finite))
    min_value = min(finite) if finite else None
    max_value = max(finite) if finite else None
    if len(finite) != len(present):
        verdict = "rejected_non_i32"
    elif negative:
        verdict = "rejected_negative_values"
    elif gt_supply:
        verdict = "rejected_gt_max_supply_raw"
    elif nonzero == 0:
        verdict = "rejected_all_zero"
    elif distinct < MIN_CANDIDATE_DISTINCT_VALUES:
        verdict = "rejected_low_variance"
    else:
        verdict = "i32_fixed_1e5_candidate_requires_anchor"
    return {
        "status": "classified",
        "values": len(present),
        "nonzero": nonzero,
        "zero": len(present) - nonzero,
        "negative": negative,
        "gt_max_supply_raw": gt_supply,
        "distinct": distinct,
        "min_raw": min_value,
        "max_raw": max_value,
        "verdict": verdict,
    }


def _anchor_match_summary(
    anchors: Sequence[Dict[str, Any]],
    values_by_province: Dict[int, Optional[int]],
    tolerance: float,
) -> Dict[str, Any]:
    if not anchors:
        return {"anchors": 0, "verdict": "no_anchors_supplied"}
    results: List[Dict[str, Any]] = []
    remaining_matches = 0
    other_matches = 0
    for anchor in anchors:
        province_id = int(anchor["province_id"])
        raw = values_by_province.get(province_id)
        value = None if raw is None else raw / FIXED_SCALE_RAW
        rec: Dict[str, Any] = {
            "province_id": province_id,
            "raw": raw,
            "value": value,
            "remaining": anchor.get("remaining"),
            "matches_remaining": False,
            "matches_other_tooltip_component": False,
        }
        if value is not None and anchor.get("remaining") is not None:
            if abs(value - float(anchor["remaining"])) <= tolerance:
                remaining_matches += 1
                rec["matches_remaining"] = True
        for key in ("hubs", "state", "sum"):
            target = anchor.get(key)
            if target is not None and value is not None and abs(value - float(target)) <= tolerance:
                other_matches += 1
                rec["matches_other_tooltip_component"] = True
                rec.setdefault("other_matches", []).append(key)
        results.append(rec)
    if remaining_matches == len(anchors) and len(anchors) >= 3 and other_matches == 0:
        verdict = "anchor_value_match_requires_temporal_differential"
    elif remaining_matches:
        verdict = "partial_anchor_match_unusable"
    else:
        verdict = "anchor_rejected"
    return {
        "anchors": len(anchors),
        "remaining_matches": remaining_matches,
        "other_component_matches": other_matches,
        "tolerance": tolerance,
        "verdict": verdict,
        "results": results,
    }


def _stale_chain_report(p: Any, provinces: Sequence[Dict[str, Any]]) -> Dict[str, Any]:
    first_offset = int(STALE_16022851_CHAIN["first_offset"])
    second_offset = int(STALE_16022851_CHAIN["second_offset"])
    value_offset = int(STALE_16022851_CHAIN["value_offset"])
    samples: List[Dict[str, Any]] = []
    usable_values = 0
    first_not_heap = 0
    second_not_heap = 0
    for province in provinces:
        rec = _read_chain_leaf(p, province, first_offset, second_offset)
        if rec["status"] == "first_not_heap":
            first_not_heap += 1
        elif rec["status"] == "second_not_heap":
            second_not_heap += 1
        if rec["status"] == "leaf":
            raw = _read_i32_at(p, int(rec["leaf"]), value_offset)
            rec["raw"] = raw
            rec["value"] = None if raw is None else raw / FIXED_SCALE_RAW
            if raw is not None and 0 <= raw <= MAX_SUPPLY_RAW:
                usable_values += 1
            rec.pop("leaf", None)
        samples.append(rec)
    if usable_values == 0:
        verdict = "rejected_current_build"
    else:
        verdict = "not_rejected_requires_manual_review"
    return {
        **STALE_16022851_CHAIN,
        "first_offset": f"+0x{first_offset:X}",
        "second_offset": f"+0x{second_offset:X}",
        "value_offset": f"+0x{value_offset:X}",
        "provinces_sampled": len(provinces),
        "first_not_heap": first_not_heap,
        "second_not_heap": second_not_heap,
        "usable_values": usable_values,
        "verdict": verdict,
        "samples": samples,
    }


def _scan_chain(
    p: Any,
    name: str,
    first_offset: int,
    second_offset: int,
    provinces: Sequence[Dict[str, Any]],
    anchors: Sequence[Dict[str, Any]],
    anchor_tolerance: float,
) -> Dict[str, Any]:
    leaves = [_read_chain_leaf(p, province, first_offset, second_offset) for province in provinces]
    leaf_records = [rec for rec in leaves if rec["status"] == "leaf"]
    slots: List[Dict[str, Any]] = []
    for value_offset in range(0, LEAF_SCAN_BYTES, 4):
        values_by_province: Dict[int, Optional[int]] = {}
        values: List[Optional[int]] = []
        for rec in leaf_records:
            raw = _read_i32_at(p, int(rec["leaf"]), value_offset)
            province_id = int(rec["province_id"])
            values_by_province[province_id] = raw
            values.append(raw)
        stats = _classify_i32_series(values)
        if stats["verdict"] != "i32_fixed_1e5_candidate_requires_anchor":
            continue
        stats["offset"] = f"+0x{value_offset:X}"
        stats["offset_int"] = value_offset
        stats["anchor_check"] = _anchor_match_summary(
            anchors,
            values_by_province,
            anchor_tolerance,
        )
        stats["samples"] = {
            str(province_id): raw
            for province_id, raw in sorted(values_by_province.items())
            if raw is not None
        }
        slots.append(stats)
    slots.sort(
        key=lambda item: (
            item.get("anchor_check", {}).get("remaining_matches", 0),
            item.get("distinct", 0),
            item.get("nonzero", 0),
        ),
        reverse=True,
    )
    candidate_slots = slots[:12]
    leaf_samples = []
    for rec in leaves:
        out = dict(rec)
        out.pop("leaf", None)
        leaf_samples.append(out)
    return {
        "name": name,
        "first_offset": f"+0x{first_offset:X}",
        "second_offset": f"+0x{second_offset:X}",
        "provinces_sampled": len(provinces),
        "leaf_count": len(leaf_records),
        "candidate_slot_count": len(slots),
        "candidate_slots": candidate_slots,
        "leaf_samples": leaf_samples,
        "verdict": (
            "bounded_i32_candidates_require_anchor"
            if slots
            else "rejected_no_bounded_i32_candidate"
        ),
    }


def _write_outputs(out_dir: Path, report: Dict[str, Any]) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "cprovince_residual_probe.json").write_text(
        json.dumps(report, indent=2),
        encoding="utf-8",
    )
    stale = report.get("stale_chain", {})
    chains = report.get("current_structure_chains", [])
    top_lines: List[str] = []
    for chain in chains:
        first = chain.get("first_offset")
        second = chain.get("second_offset")
        verdict = chain.get("verdict")
        count = chain.get("candidate_slot_count", 0)
        top_lines.append(
            f"- `{chain.get('name')}` `{first}->{second}`: `{verdict}`, "
            f"candidate_slots `{count}`."
        )
        for slot in chain.get("candidate_slots", [])[:3]:
            top_lines.append(
                f"  - slot `{slot.get('offset')}` raw range "
                f"`{slot.get('min_raw')}`..`{slot.get('max_raw')}`, "
                f"distinct `{slot.get('distinct')}`, anchor verdict "
                f"`{slot.get('anchor_check', {}).get('verdict')}`."
            )
    if not top_lines:
        top_lines.append("- No current CProvince structure chains were scanned.")
    md_lines = [
        "# supply_logistics_attrition_live CProvince residual probe",
        "",
        f"- status: `{report.get('status')}`",
        f"- exe_sha256: `{report.get('process', {}).get('sha256')}`",
        f"- province_samples_resolved: `{report.get('province_resolution', {}).get('resolved', 0)}`",
        f"- anchors_supplied: `{report.get('anchors_supplied', 0)}`",
        f"- stale_chain_verdict: `{stale.get('verdict')}`",
        f"- production_safe_exact_supply_fields: `{report.get('summary', {}).get('production_safe_exact_supply_fields', 0)}`",
        f"- current_candidate_chains: `{report.get('summary', {}).get('current_candidate_chains', 0)}`",
        "",
        "## Stale Current-Build Rejection",
        "",
        (
            f"- `{stale.get('name')}` from build `{stale.get('old_build')}` "
            f"uses `{stale.get('first_offset')}->{stale.get('second_offset')}->{stale.get('value_offset')}`."
        ),
        (
            f"- Current 870ebf sample: first_not_heap `{stale.get('first_not_heap')}`, "
            f"second_not_heap `{stale.get('second_not_heap')}`, usable_values `{stale.get('usable_values')}`."
        ),
        "- Verdict: do not port this chain into production on the current build.",
        "",
        "## Current CProvince Candidate Inventory",
        "",
        *top_lines,
        "",
        "## Residual Verdict",
        "",
        "- Exact per-province supply remains fail-closed. Candidate slots are supply-shaped fixed-point i32 values, not semantics.",
        "- A candidate can replace the mandatory exact-supply badge only after same-frame tooltip anchors validate the same chain/slot/encoding across several provinces and a temporal/network differential rejects sibling quantities.",
    ]
    (out_dir / "CPROVINCE_RESIDUAL_PROBE.md").write_text(
        "\n".join(md_lines) + "\n",
        encoding="utf-8",
    )


def run(
    pid: Optional[int],
    out_dir: Path,
    anchors_path: Optional[Path],
    extra_provinces: Sequence[int],
    anchor_tolerance: float,
) -> Dict[str, Any]:
    started = time.time()
    anchors = _load_anchors(anchors_path)
    p = winmem.Process(pid, "hoi4.exe") if pid is not None else winmem.open_process("hoi4.exe")
    report: Dict[str, Any] = {
        "lane": DOMAIN_NAME,
        "status": "fail_closed",
        "read_only": True,
        "writes_to_game_memory": False,
        "hypothesis": "current-build CProvince two-deref supply-band leaf records",
        "anchors_supplied": len(anchors),
    }
    if p is None:
        report["reason"] = "hoi4.exe not running or cannot be opened"
        _write_outputs(out_dir, report)
        return report
    p.__enter__()
    try:
        fp = p.fingerprint()
        report["process"] = fp
        if (fp.get("sha256") or "").lower() != EXPECTED_SHA256:
            report["reason"] = "build mismatch; offsets are build-specific"
            _write_outputs(out_dir, report)
            return report
        module_base = p.module_base
        game_state = p.u64(module_base + live_probe.CGAMESTATE_RVA)
        if not _is_heap(game_state):
            raise RuntimeError(f"invalid CGameState pointer 0x{game_state:X}")
        sample_ids = _province_sample_ids(anchors, extra_provinces)
        provinces, province_resolution = _resolve_provinces(
            p,
            module_base,
            game_state,
            sample_ids,
        )
        stale = _stale_chain_report(p, provinces)
        chains = [
            _scan_chain(
                p,
                name,
                first_offset,
                second_offset,
                provinces,
                anchors,
                anchor_tolerance,
            )
            for name, first_offset, second_offset in CURRENT_STRUCTURE_CHAINS
        ]
        current_candidate_chains = sum(
            1 for chain in chains
            if chain.get("verdict") == "bounded_i32_candidates_require_anchor"
        )
        anchor_matches = sum(
            1
            for chain in chains
            for slot in chain.get("candidate_slots", [])
            if slot.get("anchor_check", {}).get("verdict")
            == "anchor_value_match_requires_temporal_differential"
        )
        report.update({
            "status": "cprovince_residuals_narrowed",
            "roots_validated": {
                "cgame_state": "module+0x3316E90 -> CCurrentGameState",
                "province_array": "CGameState+0x2B0 ptr, CGameState+0x2BC count",
                "cprovince": "vtable module+0x295AF68, province_id u32@+0xA4",
            },
            "province_resolution": province_resolution,
            "stale_chain": stale,
            "current_structure_chains": chains,
            "summary": {
                "stale_chain_rejected_current_build": stale.get("verdict")
                == "rejected_current_build",
                "current_candidate_chains": current_candidate_chains,
                "anchor_value_matches_requiring_differential": anchor_matches,
                "production_safe_exact_supply_fields": 0,
                "verdict": "oracle_required",
            },
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
    return REPO_ROOT / "extractor/native" / "runs" / f"supply_logistics_attrition_live_cprovince_{stamp}"


def main(argv: Optional[Iterable[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, default=None, help="hoi4.exe pid; defaults to process name lookup")
    parser.add_argument("--out-dir", type=Path, default=None, help="proof output directory")
    parser.add_argument("--anchors", type=Path, default=None, help="optional tooltip anchor JSON")
    parser.add_argument(
        "--province",
        type=int,
        action="append",
        default=[],
        help="extra province id to sample; may be passed more than once",
    )
    parser.add_argument("--anchor-tolerance", type=float, default=0.055)
    args = parser.parse_args(list(argv) if argv is not None else None)
    out_dir = args.out_dir or default_out_dir()
    report = run(args.pid, out_dir, args.anchors, args.province, args.anchor_tolerance)
    print(json.dumps({
        "status": report.get("status"),
        "out_dir": str(out_dir),
        "stale_chain_verdict": report.get("stale_chain", {}).get("verdict"),
        "current_candidate_chains": report.get("summary", {}).get("current_candidate_chains", 0),
        "production_safe_exact_supply_fields": report.get("summary", {}).get("production_safe_exact_supply_fields", 0),
        "reason": report.get("reason"),
    }, indent=2))
    return 0 if report.get("status") == "cprovince_residuals_narrowed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
