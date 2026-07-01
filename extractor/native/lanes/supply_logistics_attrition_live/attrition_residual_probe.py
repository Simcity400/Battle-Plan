"""Division attrition residual probe for domain 44.

This is a lane-local, read-only negative/triage probe. It scans current-build
division bodies for simple scalar fields that could plausibly be attrition flags
or reasons, using low/zero supply as a weak grouping signal only. It never emits
compact rows: without a same-frame visible attrition reason oracle, correlation
against supply pressure is not enough to label a field as attrition.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
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
FIXED_SCALE_RAW = live_probe.FIXED_SCALE_RAW
MAX_SUPPLY_RAW = live_probe.MAX_SUPPLY_RAW
SCAN_BYTES = 0x780
MAX_SAMPLE_DIVISIONS = 4096

KNOWN_DIVISION_OFFSETS = {
    live_probe.DIVISION_OFF_ID_TYPE,
    live_probe.DIVISION_OFF_ID_ID,
    live_probe.DIVISION_OFF_LOCATION,
    live_probe.DIVISION_OFF_OWNER_TAG_DB_ID,
    live_probe.DIVISION_OFF_TEMPLATE,
    live_probe.DIVISION_OFF_MAX_SUPPLY,
    live_probe.DIVISION_OFF_SUPPLY_GAIN,
    live_probe.DIVISION_OFF_CURRENT_SUPPLY,
    live_probe.DIVISION_OFF_FUEL,
}


def _is_heap(value: Optional[int]) -> bool:
    return live_probe.HEAP_LO <= value < live_probe.HEAP_HI and value % 8 == 0 if value is not None else False


def _division_bodies(p: Any, module_base: int, game_state: int) -> Tuple[List[Dict[str, Any]], Dict[str, Any]]:
    country_table = p.u64(game_state + live_probe.CGAMESTATE_COUNTRY_TABLE)
    country_count = p.u32(game_state + live_probe.CGAMESTATE_COUNTRY_COUNT)
    if country_count == 0 or country_count > live_probe.MAX_CGAMESTATE_COUNTRIES:
        raise RuntimeError(f"invalid country_count {country_count}")
    if not _is_heap(country_table):
        raise RuntimeError(f"invalid country table 0x{country_table:X}")
    country_blob = p.try_read(country_table, country_count * 8)
    if not country_blob or len(country_blob) < country_count * 8:
        raise RuntimeError("could not read country table")

    country_vtable = module_base + live_probe.COUNTRY_VTABLE_RVA
    wrapper_vtable = module_base + live_probe.DIVISION_WRAPPER_VTABLE_RVA
    division_vtable = module_base + live_probe.DIVISION_VTABLE_RVA
    bodies: Dict[int, Dict[str, Any]] = {}
    rejected = 0
    country_rows = 0
    wrapper_rows = 0

    for index in range(country_count):
        country_ptr = struct.unpack_from("<Q", country_blob, index * 8)[0]
        if not _is_heap(country_ptr):
            continue
        try:
            if p.u64(country_ptr) != country_vtable:
                continue
            vector_data = p.u64(country_ptr + live_probe.COUNTRY_DIVISION_WRAPPER_DATA)
            capacity = p.u32(country_ptr + live_probe.COUNTRY_DIVISION_WRAPPER_CAPACITY)
            count = p.u32(country_ptr + live_probe.COUNTRY_DIVISION_WRAPPER_COUNT)
        except OSError:
            continue
        country_rows += 1
        if count == 0:
            continue
        if count > capacity or count > live_probe.MAX_DIVISION_ROWS or not _is_heap(vector_data):
            rejected += int(count)
            continue
        wrapper_blob = p.try_read(vector_data, count * 8)
        if not wrapper_blob or len(wrapper_blob) < count * 8:
            rejected += int(count)
            continue
        for item in range(count):
            wrapper_ptr = struct.unpack_from("<Q", wrapper_blob, item * 8)[0]
            wrapper_rows += 1
            if not _is_heap(wrapper_ptr):
                rejected += 1
                continue
            try:
                if p.u64(wrapper_ptr) != wrapper_vtable:
                    rejected += 1
                    continue
                body = wrapper_ptr + live_probe.DIVISION_WRAPPER_TO_BODY
                if p.u64(body) != division_vtable:
                    rejected += 1
                    continue
                unit_type = p.u32(body + live_probe.DIVISION_OFF_ID_TYPE)
                if unit_type not in (live_probe.DIVISION_TYPE, live_probe.DIVISION_ALT_UNIT_TYPE):
                    rejected += 1
                    continue
                div_id = p.u32(body + live_probe.DIVISION_OFF_ID_ID)
                province_id = p.u32(body + live_probe.DIVISION_OFF_LOCATION)
                owner = p.u32(body + live_probe.DIVISION_OFF_OWNER_TAG_DB_ID)
                current = live_probe._clamp_i32(
                    p.u32(body + live_probe.DIVISION_OFF_CURRENT_SUPPLY),
                    live_probe.MAX_SUPPLY_RAW,
                )
                max_supply = live_probe._clamp_i32(
                    p.u32(body + live_probe.DIVISION_OFF_MAX_SUPPLY),
                    live_probe.MAX_SUPPLY_RAW,
                )
                gain = live_probe._clamp_i32(
                    p.u32(body + live_probe.DIVISION_OFF_SUPPLY_GAIN),
                    live_probe.MAX_SUPPLY_GAIN_RAW,
                )
                fuel = live_probe._clamp_i32(
                    p.u32(body + live_probe.DIVISION_OFF_FUEL),
                    live_probe.MAX_DIVISION_FUEL_RAW,
                )
            except OSError:
                rejected += 1
                continue
            if not div_id or not province_id or province_id > live_probe.MAX_PROVINCE_ID:
                rejected += 1
                continue
            if div_id not in bodies:
                bodies[div_id] = {
                    "division_id": div_id,
                    "body": body,
                    "province_id": province_id,
                    "owner_tag_db_id": owner,
                    "current_supply_raw": current,
                    "max_supply_raw": max_supply,
                    "supply_gain_raw": gain,
                    "supply_deficit_raw": max(0, max_supply - current),
                    "fuel_raw": fuel,
                }
    rows = [bodies[key] for key in sorted(bodies)][:MAX_SAMPLE_DIVISIONS]
    summary = {
        "country_rows_seen": country_rows,
        "wrapper_rows_seen": wrapper_rows,
        "rejected_wrappers": rejected,
        "division_bodies": len(rows),
        "zero_supply": sum(1 for row in rows if row["current_supply_raw"] == 0),
        "low_supply": sum(1 for row in rows if 0 < row["current_supply_raw"] < FIXED_SCALE_RAW),
        "deficit": sum(1 for row in rows if row["supply_deficit_raw"] > 0),
        "fuel": sum(1 for row in rows if row["fuel_raw"] > 0),
    }
    return rows, summary


def _scan_scalar_offsets(p: Any, divisions: Sequence[Dict[str, Any]]) -> List[Dict[str, Any]]:
    if not divisions:
        return []
    blobs: List[Tuple[Dict[str, Any], bytes]] = []
    for row in divisions:
        blob = p.try_read(int(row["body"]), SCAN_BYTES)
        if blob and len(blob) >= SCAN_BYTES:
            blobs.append((row, blob))
    if not blobs:
        return []

    zero_or_low_ids = {
        row["division_id"] for row, _blob in blobs
        if row["current_supply_raw"] < FIXED_SCALE_RAW
    }
    deficit_ids = {
        row["division_id"] for row, _blob in blobs
        if row["supply_deficit_raw"] > 0
    }
    normal_ids = {
        row["division_id"] for row, _blob in blobs
        if row["current_supply_raw"] >= FIXED_SCALE_RAW and row["supply_deficit_raw"] == 0
    }
    candidates: List[Dict[str, Any]] = []
    for offset in range(0, SCAN_BYTES, 4):
        if offset in KNOWN_DIVISION_OFFSETS:
            continue
        values: Dict[int, int] = {}
        for row, blob in blobs:
            values[int(row["division_id"])] = struct.unpack_from("<I", blob, offset)[0]
        distinct = sorted(set(values.values()))
        if len(distinct) < 2 or len(distinct) > 16:
            continue
        max_value = max(distinct)
        if max_value > 4096:
            continue
        nonzero_ids = {div_id for div_id, value in values.items() if value != 0}
        if not nonzero_ids:
            continue
        nonzero_low = len(nonzero_ids & zero_or_low_ids)
        nonzero_deficit = len(nonzero_ids & deficit_ids)
        nonzero_normal = len(nonzero_ids & normal_ids)
        if nonzero_low == 0 and nonzero_deficit == 0:
            continue
        # Keep only offsets that are at least somewhat enriched in pressure groups.
        pressure_count = max(len(zero_or_low_ids), 1)
        deficit_count = max(len(deficit_ids), 1)
        normal_count = max(len(normal_ids), 1)
        pressure_rate = nonzero_low / pressure_count
        deficit_rate = nonzero_deficit / deficit_count
        normal_rate = nonzero_normal / normal_count
        if max(pressure_rate, deficit_rate) <= normal_rate:
            continue
        samples = []
        for row, _blob in blobs:
            value = values[int(row["division_id"])]
            if value == 0:
                continue
            samples.append({
                "division_id": row["division_id"],
                "province_id": row["province_id"],
                "value": value,
                "current_supply": round(row["current_supply_raw"] / FIXED_SCALE_RAW, 5),
                "supply_deficit": round(row["supply_deficit_raw"] / FIXED_SCALE_RAW, 5),
                "fuel": round(row["fuel_raw"] / FIXED_SCALE_RAW, 5),
            })
            if len(samples) >= 8:
                break
        candidates.append({
            "offset": f"+0x{offset:X}",
            "offset_int": offset,
            "encoding": "u32",
            "distinct_values": distinct[:16],
            "nonzero": len(nonzero_ids),
            "nonzero_low_supply": nonzero_low,
            "nonzero_deficit": nonzero_deficit,
            "nonzero_normal_no_deficit": nonzero_normal,
            "pressure_rate": round(pressure_rate, 6),
            "deficit_rate": round(deficit_rate, 6),
            "normal_rate": round(normal_rate, 6),
            "samples": samples,
            "verdict": "candidate_requires_visible_attrition_oracle",
        })
    candidates.sort(
        key=lambda item: (
            max(item["pressure_rate"], item["deficit_rate"]) - item["normal_rate"],
            item["nonzero_deficit"],
            item["nonzero_low_supply"],
        ),
        reverse=True,
    )
    return candidates[:24]


def _write_outputs(out_dir: Path, report: Dict[str, Any]) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "attrition_residual_probe.json").write_text(
        json.dumps(report, indent=2),
        encoding="utf-8",
    )
    candidate_lines: List[str] = []
    for cand in report.get("candidate_offsets", [])[:12]:
        candidate_lines.append(
            f"- `{cand.get('offset')}`: nonzero `{cand.get('nonzero')}`, "
            f"low `{cand.get('nonzero_low_supply')}`, deficit `{cand.get('nonzero_deficit')}`, "
            f"normal `{cand.get('nonzero_normal_no_deficit')}`, verdict `{cand.get('verdict')}`."
        )
    if not candidate_lines:
        candidate_lines.append("- No simple scalar candidate was enriched in low-supply or deficit groups.")
    summary = report.get("division_summary", {})
    lines = [
        "# supply_logistics_attrition_live attrition residual probe",
        "",
        f"- status: `{report.get('status')}`",
        f"- exe_sha256: `{report.get('process', {}).get('sha256')}`",
        f"- division_bodies: `{summary.get('division_bodies', 0)}`",
        f"- zero_supply: `{summary.get('zero_supply', 0)}`",
        f"- low_supply: `{summary.get('low_supply', 0)}`",
        f"- deficit: `{summary.get('deficit', 0)}`",
        f"- candidate_offsets: `{len(report.get('candidate_offsets', []))}`",
        f"- production_safe_attrition_fields: `{report.get('production_safe_attrition_fields', 0)}`",
        "",
        "## Candidate Scalar Offsets",
        "",
        *candidate_lines,
        "",
        "## Verdict",
        "",
        "- No attrition flag/reason was emitted. Low supply and supply deficit are pressure signals, not a same-frame attrition-reason oracle.",
        "- Candidate scalar offsets remain unusable until a controlled division has a visible/saved attrition reason and negative controls prove the offset is not aliasing supply, fuel, movement, combat, template, or cached UI state.",
    ]
    (out_dir / "ATTRITION_RESIDUAL_PROBE.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def run(pid: Optional[int], out_dir: Path) -> Dict[str, Any]:
    started = time.time()
    p = winmem.Process(pid, "hoi4.exe") if pid is not None else winmem.open_process("hoi4.exe")
    report: Dict[str, Any] = {
        "lane": DOMAIN_NAME,
        "status": "fail_closed",
        "read_only": True,
        "writes_to_game_memory": False,
        "hypothesis": "division-body simple scalar attrition flag/reason",
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
        game_state = p.u64(p.module_base + live_probe.CGAMESTATE_RVA)
        if not _is_heap(game_state):
            raise RuntimeError(f"invalid CGameState pointer 0x{game_state:X}")
        divisions, division_summary = _division_bodies(p, p.module_base, game_state)
        candidates = _scan_scalar_offsets(p, divisions)
        report.update({
            "status": "attrition_oracle_required",
            "roots_validated": {
                "cgame_state": "module+0x3316E90 -> CCurrentGameState",
                "division": "CGameState+0x310 country table -> country+0x290 wrapper vector -> wrapper+0x10 body",
                "division_vtable": "module+0x2943CB0",
            },
            "division_summary": division_summary,
            "candidate_offsets": candidates,
            "production_safe_attrition_fields": 0,
            "verdict": (
                "No compact attrition row emitted. A visible/saved same-frame "
                "attrition reason oracle is required before any scalar candidate "
                "can replace the attrition badge."
            ),
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
    return REPO_ROOT / "extractor/native" / "runs" / f"supply_logistics_attrition_live_attrition_{stamp}"


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
        "division_bodies": report.get("division_summary", {}).get("division_bodies", 0),
        "candidate_offsets": len(report.get("candidate_offsets", [])),
        "production_safe_attrition_fields": report.get("production_safe_attrition_fields", 0),
        "reason": report.get("reason"),
    }, indent=2))
    return 0 if report.get("status") == "attrition_oracle_required" else 1


if __name__ == "__main__":
    raise SystemExit(main())
