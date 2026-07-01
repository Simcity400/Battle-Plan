"""Foreign-homebase / convoy pressure probe for domain 44.

This lane-local probe follows the validated current-build join
`CCountrySupplySystem+0x78 -> CCountry` and tests pointer fields on the joined
object for save-oracle `foreign_homebase_nodes` anchors.

The goal is bounded residual reduction:

* validate/reject pointer fields as tag-matched foreign-homebase province roots;
* infer simple record-stride hypotheses for surviving province roots;
* test whether saved `supply` / `penalty` values are co-located as f32/f64 or
  common fixed-point encodings.

No compact rows are emitted. Stale save anchors are treated as candidate
evidence only unless value co-location is exact enough for a same-frame proof.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import math
import struct
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Set, Tuple

LANE_DIR = Path(__file__).resolve().parent
REPO_ROOT = LANE_DIR.parents[3]
sys.path.insert(0, str(LANE_DIR))
sys.path.insert(0, str(REPO_ROOT / "extractor" / "memory_reader"))

import ccountry_supply_layout_probe as layout_probe  # noqa: E402
import live_probe  # noqa: E402
import winmem  # noqa: E402

DOMAIN_NAME = live_probe.DOMAIN_NAME
HEAP_LO = live_probe.HEAP_LO
HEAP_HI = live_probe.HEAP_HI
MAX_PROVINCE_ID = live_probe.MAX_PROVINCE_ID

CCSS_COUNTRY_BACKPTR = 0x78
CCSS_SCAN_BYTES = 0x300
REGION_SCAN_BYTES = 0x8000
MAX_REPORTED = 48
BACKREF_POINTER_OFFSETS = {0x70, 0x78, 0x100, 0x310, 0x318, 0x3A0}
COMMON_STRIDES = (0x10, 0x18, 0x20, 0x28, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90, 0xA0)
VALUE_WINDOW = 0x90
F32_TOL = 0.0025
F64_TOL = 0.00001
FIXED_SCALES = (100, 1000, 10000, 100000)

DEFAULT_SUPPLY_JSON = (
    REPO_ROOT
    / "extractor/native"
    / "runs"
    / "supply_logistics_attrition_live_save_oracle_870ebf_20260701_v3"
    / "supply.json"
)


def hx(value: Optional[int]) -> Optional[str]:
    return f"0x{value:X}" if isinstance(value, int) and value else None


def is_heap(value: Optional[int]) -> bool:
    return isinstance(value, int) and HEAP_LO <= value < HEAP_HI and value % 8 == 0


def safe_u64(p: Any, addr: int) -> Optional[int]:
    try:
        return p.u64(addr)
    except OSError:
        return None


def load_foreign_anchors(path: Path) -> Dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    by_tag: Dict[str, List[Dict[str, Any]]] = defaultdict(list)
    for tag, rec in (data.get("countries") or {}).items():
        for item in rec.get("foreign_homebase_nodes") or []:
            province = item.get("province")
            supply = item.get("supply")
            penalty = item.get("penalty")
            if not isinstance(province, int) or not (0 < province <= MAX_PROVINCE_ID):
                continue
            by_tag[tag].append({
                "province": int(province),
                "supply": float(supply) if isinstance(supply, (int, float)) else None,
                "penalty": float(penalty) if isinstance(penalty, (int, float)) else None,
                "source": item.get("source"),
            })
    return {
        "path": str(path),
        "save_date": data.get("meta", {}).get("save_date"),
        "player": data.get("meta", {}).get("player"),
        "by_tag": dict(sorted(by_tag.items())),
        "entry_count": sum(len(rows) for rows in by_tag.values()),
        "country_count": len(by_tag),
        "unique_provinces": sorted({
            row["province"]
            for rows in by_tag.values()
            for row in rows
        }),
    }


def scan_u32_values(blob: bytes, values: Set[int]) -> List[Tuple[int, int]]:
    out: List[Tuple[int, int]] = []
    for off in range(0, max(0, len(blob) - 3), 4):
        value = struct.unpack_from("<I", blob, off)[0]
        if value in values:
            out.append((off, value))
    return out


def _value_targets(rows: Sequence[Dict[str, Any]], province: int) -> List[Tuple[str, float]]:
    out: List[Tuple[str, float]] = []
    for row in rows:
        if row.get("province") != province:
            continue
        for key in ("supply", "penalty"):
            value = row.get(key)
            if isinstance(value, (int, float)) and math.isfinite(float(value)):
                out.append((key, float(value)))
    return out


def scan_value_colocation(
    blob: bytes,
    match_off: int,
    rows: Sequence[Dict[str, Any]],
    province: int,
) -> List[Dict[str, Any]]:
    targets = _value_targets(rows, province)
    if not targets:
        return []
    lo = max(0, match_off - VALUE_WINDOW)
    hi = min(len(blob), match_off + VALUE_WINDOW)
    hits: List[Dict[str, Any]] = []
    for off in range(lo, max(lo, hi - 3), 4):
        u32 = struct.unpack_from("<I", blob, off)[0]
        i32 = struct.unpack_from("<i", blob, off)[0]
        f32 = struct.unpack_from("<f", blob, off)[0]
        for key, target in targets:
            if math.isfinite(f32) and abs(f32 - target) <= F32_TOL:
                hits.append({
                    "encoding": "f32",
                    "field": key,
                    "offset": f"+0x{off:X}",
                    "value": round(f32, 6),
                    "target": target,
                })
            for scale in FIXED_SCALES:
                fixed = i32 / scale
                if abs(fixed - target) <= max(F32_TOL, 0.5 / scale):
                    hits.append({
                        "encoding": f"i32/{scale}",
                        "field": key,
                        "offset": f"+0x{off:X}",
                        "raw": i32,
                        "value": round(fixed, 6),
                        "target": target,
                    })
                if u32 <= 10_000_000:
                    ufixed = u32 / scale
                    if abs(ufixed - target) <= max(F32_TOL, 0.5 / scale):
                        hits.append({
                            "encoding": f"u32/{scale}",
                            "field": key,
                            "offset": f"+0x{off:X}",
                            "raw": u32,
                            "value": round(ufixed, 6),
                            "target": target,
                        })
    for off in range(lo, max(lo, hi - 7), 8):
        f64 = struct.unpack_from("<d", blob, off)[0]
        for key, target in targets:
            if math.isfinite(f64) and abs(f64 - target) <= F64_TOL:
                hits.append({
                    "encoding": "f64",
                    "field": key,
                    "offset": f"+0x{off:X}",
                    "value": round(f64, 8),
                    "target": target,
                })
    return hits


def stride_hypotheses(matches: Sequence[Tuple[int, int]]) -> List[Dict[str, Any]]:
    out: List[Dict[str, Any]] = []
    if not matches:
        return out
    offsets = [off for off, _ in matches]
    for stride in COMMON_STRIDES:
        slots = Counter(off % stride for off in offsets)
        top_slots = slots.most_common(4)
        out.append({
            "stride": f"0x{stride:X}",
            "top_slots": [
                {"slot": f"+0x{slot:X}", "matches": count}
                for slot, count in top_slots
            ],
            "covered_matches": sum(count for _, count in top_slots[:2]),
        })
    return sorted(out, key=lambda item: item["covered_matches"], reverse=True)[:6]


def scan_pressure_candidates(
    p: Any,
    ccss_objects: Sequence[int],
    country_to_tag: Dict[int, Tuple[int, str]],
    anchors: Dict[str, Any],
    csupply_object: Optional[int],
) -> Dict[str, Any]:
    country_vtable = p.module_base + live_probe.COUNTRY_VTABLE_RVA
    by_tag = anchors["by_tag"]
    field_counts: Counter[int] = Counter()
    field_value_hits: Counter[int] = Counter()
    field_tags: Dict[int, Set[str]] = defaultdict(set)
    candidates: List[Dict[str, Any]] = []
    object_count = 0
    regions_read = 0
    regions_with_matches = 0
    joined_anchor_tags: Set[str] = set()

    for obj in ccss_objects:
        country_ptr = safe_u64(p, obj + CCSS_COUNTRY_BACKPTR)
        if country_ptr not in country_to_tag:
            continue
        tag_id, tag = country_to_tag[country_ptr]
        tag_rows = by_tag.get(tag) or []
        if not tag_rows:
            continue
        joined_anchor_tags.add(tag)
        object_count += 1
        provinces = {int(row["province"]) for row in tag_rows}
        body = p.try_read(obj, CCSS_SCAN_BYTES)
        if len(body) < 8:
            continue
        for field_off in range(0x10, min(len(body) - 7, CCSS_SCAN_BYTES), 8):
            if field_off in BACKREF_POINTER_OFFSETS:
                continue
            ptr = struct.unpack_from("<Q", body, field_off)[0]
            if not is_heap(ptr) or ptr == csupply_object or ptr == country_ptr:
                continue
            if safe_u64(p, ptr) == country_vtable:
                continue
            blob = p.try_read(ptr, REGION_SCAN_BYTES)
            if len(blob) < 4:
                continue
            regions_read += 1
            matches = scan_u32_values(blob, provinces)
            if not matches:
                continue
            regions_with_matches += 1
            field_counts[field_off] += len(matches)
            field_tags[field_off].add(tag)
            value_hits: List[Dict[str, Any]] = []
            samples: List[Dict[str, Any]] = []
            for match_off, province in matches[:MAX_REPORTED]:
                hits = scan_value_colocation(blob, match_off, tag_rows, province)
                value_hits.extend(hits)
                if len(samples) < 12:
                    samples.append({
                        "region_offset": f"+0x{match_off:X}",
                        "province": province,
                        "value_hits": hits[:6],
                    })
            if value_hits:
                field_value_hits[field_off] += len(value_hits)
            if len(candidates) < MAX_REPORTED:
                candidates.append({
                    "ccountry_supply_system": hx(obj),
                    "tag_db_id": tag_id,
                    "tag": tag,
                    "field_offset": f"+0x{field_off:X}",
                    "region": hx(ptr),
                    "province_matches": len(matches),
                    "value_colocation_hits": len(value_hits),
                    "stride_hypotheses": stride_hypotheses(matches),
                    "samples": samples,
                })

    return {
        "joined_anchor_tags": sorted(joined_anchor_tags),
        "joined_anchor_tag_count": len(joined_anchor_tags),
        "objects_with_foreign_anchors": object_count,
        "regions_read": regions_read,
        "regions_with_province_matches": regions_with_matches,
        "field_offset_counts": [
            {
                "field_offset": f"+0x{off:X}",
                "province_matches": count,
                "tags": sorted(field_tags[off]),
                "value_colocation_hits": field_value_hits[off],
            }
            for off, count in field_counts.most_common(24)
        ],
        "candidates": candidates,
        "verdict": (
            "foreign_homebase_pressure_values_candidate_found"
            if field_value_hits
            else "foreign_homebase_pressure_province_roots_narrowed"
            if field_counts
            else "foreign_homebase_pressure_roots_rejected"
        ),
        "production_safe_pressure_fields": 0,
    }


def _write_outputs(out_dir: Path, report: Dict[str, Any]) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "foreign_homebase_pressure_probe.json").write_text(
        json.dumps(report, indent=2),
        encoding="utf-8",
    )
    scan = report.get("pressure_scan", {})
    lines = [
        "# supply_logistics_attrition_live foreign-homebase pressure probe",
        "",
        f"- status: `{report.get('status')}`",
        f"- exe_sha256: `{report.get('process', {}).get('sha256')}`",
        f"- save_date: `{report.get('save_oracle', {}).get('save_date')}`",
        f"- live_date: `{report.get('live_date', {}).get('decoded')}`",
        f"- foreign_homebase_entries: `{report.get('save_oracle', {}).get('entry_count')}`",
        f"- joined_anchor_tags: `{scan.get('joined_anchor_tag_count')}`",
        f"- regions_with_province_matches: `{scan.get('regions_with_province_matches')}`",
        "",
        "## Field Counts",
        "",
    ]
    for row in scan.get("field_offset_counts", [])[:16]:
        lines.append(
            f"- `{row.get('field_offset')}`: province matches `{row.get('province_matches')}`, "
            f"value hits `{row.get('value_colocation_hits')}`, tags `{row.get('tags')}`."
        )
    lines.extend(["", "## Candidate Samples", ""])
    for candidate in scan.get("candidates", [])[:10]:
        lines.append(f"- `{candidate}`")
    lines.extend([
        "",
        "## Production Verdict",
        "",
        "- No pressure compact row is emitted from this probe.",
        "- Province roots are candidate evidence only unless supply/penalty value co-location and same-frame anchors are proven.",
    ])
    (out_dir / "FOREIGN_HOMEBASE_PRESSURE_PROBE.md").write_text(
        "\n".join(lines) + "\n",
        encoding="utf-8",
    )


def run(pid: Optional[int], supply_json: Path, out_dir: Path) -> Dict[str, Any]:
    started = time.time()
    p = winmem.Process(pid, "hoi4.exe") if pid is not None else winmem.open_process("hoi4.exe")
    report: Dict[str, Any] = {
        "lane": DOMAIN_NAME,
        "status": "fail_closed",
        "read_only": True,
        "writes_to_game_memory": False,
        "supply_json": str(supply_json),
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
        live_date = layout_probe.read_current_date(p, countries.get("game_state"))
        anchors = load_foreign_anchors(supply_json)
        ccss_objects = layout_probe.scan_ccountry_supply_objects(p)
        csupply_object = layout_probe.find_csupply_object(p)
        country_to_tag = {
            country: (tag_id, countries["id_to_tag"].get(tag_id, ""))
            for tag_id, country in countries["id_to_country"].items()
        }
        scan = scan_pressure_candidates(
            p,
            ccss_objects,
            country_to_tag,
            anchors,
            csupply_object,
        )
        report.update({
            "status": scan["verdict"],
            "live_date": live_date,
            "save_oracle": {
                "path": anchors["path"],
                "save_date": anchors["save_date"],
                "player": anchors["player"],
                "entry_count": anchors["entry_count"],
                "country_count": anchors["country_count"],
                "unique_provinces": len(anchors["unique_provinces"]),
            },
            "summary": {
                "ccountry_supply_system_objects": len(ccss_objects),
                "live_countries": len(countries["id_to_country"]),
                "csupply_object": hx(csupply_object),
                "production_safe_pressure_fields": 0,
            },
            "pressure_scan": scan,
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
    return REPO_ROOT / "extractor/native" / "runs" / f"supply_logistics_attrition_live_foreign_pressure_{stamp}"


def main(argv: Optional[Iterable[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, default=None, help="hoi4.exe pid; defaults to process name lookup")
    parser.add_argument("--supply-json", type=Path, default=DEFAULT_SUPPLY_JSON, help="save-oracle supply.json anchors")
    parser.add_argument("--out-dir", type=Path, default=None, help="proof output directory")
    args = parser.parse_args(list(argv) if argv is not None else None)
    out_dir = args.out_dir or default_out_dir()
    report = run(args.pid, args.supply_json, out_dir)
    print(json.dumps({
        "status": report.get("status"),
        "out_dir": str(out_dir),
        "live_date": report.get("live_date", {}).get("decoded"),
        "save_date": report.get("save_oracle", {}).get("save_date"),
        "foreign_homebase_entries": report.get("save_oracle", {}).get("entry_count"),
        "joined_anchor_tags": report.get("pressure_scan", {}).get("joined_anchor_tag_count"),
        "regions_with_province_matches": report.get("pressure_scan", {}).get("regions_with_province_matches"),
        "top_fields": report.get("pressure_scan", {}).get("field_offset_counts", [])[:8],
        "elapsed_s": report.get("elapsed_s"),
    }, indent=2))
    return 0 if report.get("status") in (
        "foreign_homebase_pressure_values_candidate_found",
        "foreign_homebase_pressure_province_roots_narrowed",
        "foreign_homebase_pressure_roots_rejected",
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
