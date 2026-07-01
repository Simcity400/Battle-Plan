"""Residual live probe for supply/logistics/attrition production gaps.

This lane-local scanner is deliberately narrower than a general heap search.
It tests one new structure hypothesis: a CSupplySystem-shaped object may expose
province-sized pointer/count fields near the known vtable pair. Every candidate
field is classified, but none is emitted as production data unless it satisfies
strict province-indexed bounds. The script is read-only and writes only proof
artifacts under extractor/native/runs.
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
from typing import Any, Dict, Iterable, List, Optional, Tuple

LANE_DIR = Path(__file__).resolve().parent
REPO_ROOT = LANE_DIR.parents[3]
sys.path.insert(0, str(LANE_DIR))
sys.path.insert(0, str(REPO_ROOT / "extractor" / "memory_reader"))

import live_probe  # noqa: E402
import winmem  # noqa: E402

EXPECTED_SHA256 = live_probe.EXPECTED_SHA256
DOMAIN_NAME = live_probe.DOMAIN_NAME
MAX_PROVINCE_ID = live_probe.MAX_PROVINCE_ID
PROVINCE_COUNT = MAX_PROVINCE_ID + 1
HEAP_LO = live_probe.HEAP_LO
HEAP_HI = live_probe.HEAP_HI
FIXED_SCALE_RAW = live_probe.FIXED_SCALE_RAW
MAX_SUPPLY_RAW = live_probe.MAX_SUPPLY_RAW

CSUPPLYSYSTEM_VTABLE0_RVA = live_probe.CSUPPLYSYSTEM_VTABLE0_RVA
CSUPPLYSYSTEM_VTABLE1_RVA = live_probe.CSUPPLYSYSTEM_VTABLE1_RVA
KNOWN_DIRECT_ARRAY_OFFSETS = {
    0x10: "+0x10 filtered efficiency rows emitted by live_probe; raw full-array semantics still require oracle",
    0x138: "+0x138 previously rejected direct array",
    0x150: "+0x150 previously rejected direct array",
}

STRUCTURAL_SCAN_BYTES = 0x400
MAX_HEAP_REGION_BYTES = 0x10000000
SAMPLE_PROVINCES = (1, 2, 10, 100, 500, 1000, 2000, 3324, 3544, 6421, 9244, 12000, 13413)


def _is_heap(value: Optional[int]) -> bool:
    return value is not None and HEAP_LO <= value < HEAP_HI and value % 8 == 0


def _read_csupplysystem_objects(p: Any, module_base: int) -> List[int]:
    vtable0 = module_base + CSUPPLYSYSTEM_VTABLE0_RVA
    vtable1 = module_base + CSUPPLYSYSTEM_VTABLE1_RVA
    needle = struct.pack("<QQ", vtable0, vtable1)
    out: List[int] = []
    for base, size in p.iter_regions(HEAP_LO, HEAP_HI, want_exec=False):
        if size > MAX_HEAP_REGION_BYTES:
            continue
        blob = p.try_read(base, size)
        if not blob:
            continue
        search = 0
        while True:
            found = blob.find(needle, search)
            if found < 0:
                break
            obj = base + found
            out.append(obj)
            search = found + 8
    return sorted(set(out))


def _safe_read_object_blob(p: Any, obj: int, size: int) -> bytes:
    blob = p.try_read(obj, size)
    if len(blob) < 16:
        raise RuntimeError(f"could not read CSupplySystem object at 0x{obj:X}")
    return blob


def _u32(blob: bytes, offset: int) -> Optional[int]:
    if offset < 0 or offset + 4 > len(blob):
        return None
    return struct.unpack_from("<I", blob, offset)[0]


def _u64(blob: bytes, offset: int) -> Optional[int]:
    if offset < 0 or offset + 8 > len(blob):
        return None
    return struct.unpack_from("<Q", blob, offset)[0]


def _count_forms(blob: bytes, offset: int) -> List[Tuple[str, int]]:
    forms: List[Tuple[str, int]] = []
    raw_u32 = _u32(blob, offset + 8)
    if raw_u32 is not None:
        forms.append(("u32_at_plus_8", raw_u32))
    packed = _u64(blob, offset + 8)
    if packed is not None:
        forms.append(("packed_high_u32_at_plus_8", (packed >> 32) & 0xFFFFFFFF))
        forms.append(("packed_low_u32_at_plus_8", packed & 0xFFFFFFFF))
    raw_u32_plus_12 = _u32(blob, offset + 12)
    if raw_u32_plus_12 is not None:
        forms.append(("u32_at_plus_12", raw_u32_plus_12))
    dedup: Dict[str, int] = {}
    for label, value in forms:
        dedup.setdefault(label, int(value))
    return list(dedup.items())


def _series_values(p: Any, array: int, stride: int, width: int, fmt: str) -> Tuple[List[Any], int]:
    want = PROVINCE_COUNT * stride
    blob = p.try_read(array, want)
    if len(blob) < want:
        return [], want - len(blob)
    values: List[Any] = []
    for province_id in range(1, PROVINCE_COUNT):
        pos = province_id * stride
        try:
            values.append(struct.unpack_from(fmt, blob, pos)[0])
        except struct.error:
            return values, want - pos
    return values, 0


def _numeric_stats(values: List[Any], kind: str) -> Dict[str, Any]:
    if not values:
        return {"kind": kind, "values": 0, "verdict": "unreadable"}
    finite_values = [
        value for value in values if not isinstance(value, float) or math.isfinite(value)
    ]
    nonzero = sum(1 for value in finite_values if value != 0)
    negative = sum(1 for value in finite_values if value < 0)
    gt_ratio = sum(1 for value in finite_values if value > FIXED_SCALE_RAW)
    gt_supply = sum(1 for value in finite_values if value > MAX_SUPPLY_RAW)
    min_value = min(finite_values) if finite_values else None
    max_value = max(finite_values) if finite_values else None
    max_abs = max((abs(value) for value in finite_values), default=0)
    nan_or_inf = len(values) - len(finite_values)
    samples: Dict[str, Any] = {}
    for province_id in SAMPLE_PROVINCES:
        if 1 <= province_id < PROVINCE_COUNT:
            value = values[province_id - 1]
            if isinstance(value, float):
                samples[str(province_id)] = round(value, 6) if math.isfinite(value) else str(value)
            else:
                samples[str(province_id)] = value
    if nan_or_inf:
        verdict = "rejected_nan_or_inf"
    elif kind.startswith("f") and max_abs < 1e-6:
        verdict = "rejected_degenerate_float_encoding"
    elif negative:
        verdict = "rejected_negative_values"
    elif gt_supply:
        verdict = "rejected_gt_max_supply_raw"
    elif nonzero == 0:
        verdict = "rejected_all_zero"
    elif gt_ratio:
        verdict = "bounded_supply_raw_candidate_unproven"
    else:
        verdict = "bounded_ratio_raw_candidate_unproven"
    return {
        "kind": kind,
        "values": len(values),
        "nonzero": nonzero,
        "zero": len(values) - nonzero,
        "negative": negative,
        "gt_100000": gt_ratio,
        "gt_max_supply_raw": gt_supply,
        "nan_or_inf": nan_or_inf,
        "min": min_value,
        "max": max_value,
        "samples": samples,
        "verdict": verdict,
    }


def _classify_array(p: Any, array: int) -> List[Dict[str, Any]]:
    specs = [
        ("i32_stride4", 4, 4, "<i"),
        ("u32_stride4", 4, 4, "<I"),
        ("f32_stride4", 4, 4, "<f"),
        ("i32_stride8_low", 8, 4, "<i"),
        ("u32_stride8_low", 8, 4, "<I"),
        ("f32_stride8_low", 8, 4, "<f"),
        ("f64_stride8", 8, 8, "<d"),
    ]
    out: List[Dict[str, Any]] = []
    for kind, stride, width, fmt in specs:
        values, missing = _series_values(p, array, stride, width, fmt)
        stats = _numeric_stats(values, kind)
        stats["stride"] = stride
        stats["width"] = width
        stats["missing_bytes"] = missing
        if missing:
            stats["verdict"] = "rejected_short_read"
        out.append(stats)
    return out


def _candidate_fields(p: Any, obj: int, blob: bytes) -> List[Dict[str, Any]]:
    fields: List[Dict[str, Any]] = []
    for offset in range(0, min(len(blob), STRUCTURAL_SCAN_BYTES) - 16 + 1, 8):
        ptr = _u64(blob, offset)
        if not _is_heap(ptr):
            continue
        matched_count_forms = [
            {"form": label, "value": value}
            for label, value in _count_forms(blob, offset)
            if value == PROVINCE_COUNT
        ]
        if not matched_count_forms:
            continue
        fields.append({
            "offset": f"+0x{offset:X}",
            "offset_int": offset,
            "array": f"0x{ptr:X}",
            "count_forms": matched_count_forms,
            "known_direct_array_note": KNOWN_DIRECT_ARRAY_OFFSETS.get(offset),
            "classifications": _classify_array(p, ptr),
        })
    return fields


def _field_has_emit_candidate(field: Dict[str, Any]) -> bool:
    for item in field.get("classifications", []):
        if item.get("verdict") in (
            "bounded_ratio_raw_candidate_unproven",
            "bounded_supply_raw_candidate_unproven",
        ):
            return True
    return False


def _summarize_candidate_fields(fields: List[Dict[str, Any]]) -> Dict[str, Any]:
    candidate_fields = [field for field in fields if _field_has_emit_candidate(field)]
    production_safe_fields = []
    for field in candidate_fields:
        clean = [
            item for item in field["classifications"]
            if item.get("verdict") == "bounded_supply_raw_candidate_unproven"
            and item.get("negative") == 0
            and item.get("gt_max_supply_raw") == 0
        ]
        if clean:
            production_safe_fields.append({"offset": field["offset"], "array": field["array"]})
    return {
        "province_sized_fields": len(fields),
        "bounded_numeric_candidate_fields": len(candidate_fields),
        "production_safe_fields": 0,
        "candidate_fields_requiring_oracle": len(candidate_fields),
        "bounded_supply_layouts_requiring_oracle": production_safe_fields,
        "verdict": (
            "blocked_oracle_required"
            if candidate_fields
            else "rejected_no_province_sized_numeric_candidate"
        ),
    }


def _write_outputs(out_dir: Path, report: Dict[str, Any]) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "residual_probe.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    objects = report.get("csupplysystem_objects", [])
    first = objects[0] if objects else {}
    field_lines: List[str] = []
    for field in first.get("province_sized_fields", []):
        verdicts = sorted({item.get("verdict", "unknown") for item in field.get("classifications", [])})
        note = field.get("known_direct_array_note") or "new structural field"
        field_lines.append(
            f"- `{field.get('offset')}` `{field.get('array')}`: {note}; verdicts `{', '.join(verdicts)}`."
        )
    if not field_lines:
        field_lines.append("- No province-sized pointer/count fields found in the scanned object span.")
    summary = report.get("summary", {})
    md_lines = [
        "# supply_logistics_attrition_live residual probe",
        "",
        f"- status: `{report.get('status')}`",
        f"- exe_sha256: `{report.get('process', {}).get('sha256')}`",
        f"- csupplysystem_objects: `{len(objects)}`",
        f"- province_sized_fields: `{summary.get('province_sized_fields', 0)}`",
        f"- bounded_numeric_candidate_fields: `{summary.get('bounded_numeric_candidate_fields', 0)}`",
        f"- candidate_fields_requiring_oracle: `{summary.get('candidate_fields_requiring_oracle', 0)}`",
        f"- production_safe_fields: `{summary.get('production_safe_fields', 0)}`",
        f"- verdict: `{summary.get('verdict', 'unknown')}`",
        "",
        "## CSupplySystem Structural Scan",
        "",
        *field_lines,
        "",
        "## Residual Field Verdicts",
        "",
        "- Exact per-province supply / remaining supply amount: not emitted. The live scan found no exact displayed remaining field that is both province-sized and production-safe without a same-frame tooltip/save oracle.",
        "- Attrition flags and reason: not emitted. Current validated division roots expose supply/fuel pressure, but no pinned attrition reason field or function output.",
        "- Hub motorization: not emitted. Hub/building roots are validated, but no current-build motorization setting root is pinned.",
        "- Convoy/logistics pressure: not emitted. Existing convoy/fuel data in other domains is not an exact supply-pressure root for this lane.",
        "- CSupplySystem province-input path: `+0x10` is emitted by `live_probe.py` as filtered bounded efficiency/fulfilment rows; raw full-array semantics still require anchors for rejected/packed entries. `+0x138` and `+0x150` remain un-emitted under current guards.",
        "",
        "## Required Fixture To Unblock",
        "",
        "- Same-frame tooltip or save oracle for a known province supply value while the probe samples candidate arrays.",
        "- Controlled hub motorization toggle fixture with before/after samples for the same hub/province.",
        "- Controlled attrition fixture with a known division attrition reason visible in UI/save state.",
        "- Controlled overseas/convoy supply-pressure fixture with known convoy/logistics shortage state.",
        "- See `RESIDUAL_FIXTURE_CONTRACTS.md` for exact anchor and differential requirements.",
    ]
    (out_dir / "RESIDUAL_PROBE.md").write_text("\n".join(md_lines) + "\n", encoding="utf-8")


def run(pid: Optional[int], out_dir: Path) -> Dict[str, Any]:
    started = time.time()
    p = winmem.Process(pid, "hoi4.exe") if pid is not None else winmem.open_process("hoi4.exe")
    report: Dict[str, Any] = {
        "lane": DOMAIN_NAME,
        "status": "fail_closed",
        "read_only": True,
        "writes_to_game_memory": False,
        "hypothesis": "CSupplySystem object-local province-sized pointer/count fields",
        "scan_bytes": STRUCTURAL_SCAN_BYTES,
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
        objects = _read_csupplysystem_objects(p, p.module_base)
        object_reports: List[Dict[str, Any]] = []
        all_fields: List[Dict[str, Any]] = []
        for obj in objects:
            blob = _safe_read_object_blob(p, obj, STRUCTURAL_SCAN_BYTES)
            fields = _candidate_fields(p, obj, blob)
            all_fields.extend(fields)
            object_reports.append({
                "object": f"0x{obj:X}",
                "vtable0": f"module+0x{CSUPPLYSYSTEM_VTABLE0_RVA:X}",
                "vtable1": f"module+0x{CSUPPLYSYSTEM_VTABLE1_RVA:X}",
                "province_sized_fields": fields,
            })
        report["csupplysystem_objects"] = object_reports
        report["summary"] = _summarize_candidate_fields(all_fields)
        report["status"] = "residuals_hard_blocked" if objects else "csupplysystem_not_found"
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
    return REPO_ROOT / "extractor/native" / "runs" / f"supply_logistics_attrition_live_residual_{stamp}"


def main(argv: Optional[Iterable[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, default=None, help="hoi4.exe pid; defaults to process name lookup")
    parser.add_argument("--out-dir", type=Path, default=None, help="proof output directory")
    args = parser.parse_args(list(argv) if argv is not None else None)
    out_dir = args.out_dir or default_out_dir()
    report = run(args.pid, out_dir)
    print(json.dumps({
        "status": report.get("status"),
        "out_dir": str(out_dir),
        "csupplysystem_objects": len(report.get("csupplysystem_objects", [])),
        "province_sized_fields": report.get("summary", {}).get("province_sized_fields", 0),
        "bounded_numeric_candidate_fields": report.get("summary", {}).get("bounded_numeric_candidate_fields", 0),
        "candidate_fields_requiring_oracle": report.get("summary", {}).get("candidate_fields_requiring_oracle", 0),
        "production_safe_fields": report.get("summary", {}).get("production_safe_fields", 0),
        "reason": report.get("reason"),
    }, indent=2))
    return 0 if report.get("status") == "residuals_hard_blocked" else 1


if __name__ == "__main__":
    raise SystemExit(main())
