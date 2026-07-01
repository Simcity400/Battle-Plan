"""Exact-record refinement probe for foreign-homebase logistics pressure.

The broad pressure probe narrowed save-oracle `foreign_homebase_nodes` matches
to `CCountrySupplySystem+0x1E0`, but broad byte-window co-location is not a
production contract. This probe tests concrete record layouts against every
available anchor and reports only offset-specific matches:

* country join: `CCountrySupplySystem+0x78 -> CCountry`;
* candidate root: `CCountrySupplySystem+0x1E0`;
* candidate record: stride `0x70`, province `+0x00`, supply `+0x08` as
  i32/100000, penalty `+0x18` as i32/100000, duplicate province `+0x50`.

The save oracle is not same-frame, so even strong matches remain a validated
candidate/fixture contract rather than an emitted compact row. Competing fields
and competing strides are scored so false positives can be rejected explicitly.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import math
import struct
import sys
import time
from collections import defaultdict
from dataclasses import dataclass
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
PRIMARY_FIELD = 0x1E0
REGION_SCAN_BYTES = 0x10000
FIXED_SCALE_RAW = 100_000
RAW_ROUNDED_TOLERANCE = 500
MAX_REPORTED = 48
MAX_COMPACT_CANDIDATE_ROWS = 4096

PRESSURE_FLAGS = (
    live_probe.ROW_FLAG_FOREIGN_HOMEBASE_PRESSURE
    | live_probe.ROW_FLAG_PRESSURE_CANDIDATE_LAYOUT
)

DEFAULT_SUPPLY_JSON = (
    REPO_ROOT
    / "extractor/native"
    / "runs"
    / "supply_logistics_attrition_live_save_oracle_870ebf_20260701_v3"
    / "supply.json"
)


@dataclass(frozen=True)
class Layout:
    name: str
    field_off: int
    stride: int
    province_off: int
    supply_off: int
    penalty_off: int
    duplicate_province_off: Optional[int] = None


LAYOUTS = [
    Layout("ccss_1e0_stride70_primary", 0x1E0, 0x70, 0x00, 0x08, 0x18, 0x50),
    Layout("ccss_1e0_stride70_duplicate_slot", 0x1E0, 0x70, 0x50, 0x08, 0x18, None),
    Layout("ccss_1e0_stride20_slot0", 0x1E0, 0x20, 0x00, 0x08, 0x18, None),
    Layout("ccss_1e0_stride10_slot0", 0x1E0, 0x10, 0x00, 0x08, 0x18, None),
    Layout("ccss_038_stride70_primary", 0x38, 0x70, 0x00, 0x08, 0x18, 0x50),
    Layout("ccss_0d0_stride70_primary", 0xD0, 0x70, 0x00, 0x08, 0x18, 0x50),
    Layout("ccss_0b0_stride20_slot0", 0xB0, 0x20, 0x00, 0x08, 0x18, None),
    Layout("ccss_020_stride18_slot14", 0x20, 0x18, 0x14, 0x0C, 0x10, None),
    Layout("ccss_120_stride70_primary", 0x120, 0x70, 0x00, 0x08, 0x18, 0x50),
]


def hx(value: Optional[int]) -> Optional[str]:
    return f"0x{value:X}" if isinstance(value, int) and value else None


def is_heap(value: Optional[int]) -> bool:
    return isinstance(value, int) and HEAP_LO <= value < HEAP_HI and value % 8 == 0


def safe_u64(p: Any, addr: int) -> Optional[int]:
    try:
        return p.u64(addr)
    except OSError:
        return None


def _raw(value: Any) -> Optional[int]:
    if not isinstance(value, (int, float)) or not math.isfinite(float(value)):
        return None
    return int(round(float(value) * FIXED_SCALE_RAW))


def load_foreign_anchors(path: Path) -> Dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    by_tag: Dict[str, List[Dict[str, Any]]] = defaultdict(list)
    for tag, rec in (data.get("countries") or {}).items():
        for item in rec.get("foreign_homebase_nodes") or []:
            province = item.get("province")
            if not isinstance(province, int) or not (0 < province <= MAX_PROVINCE_ID):
                continue
            supply_raw = _raw(item.get("supply"))
            penalty_raw = _raw(item.get("penalty"))
            by_tag[tag].append({
                "province": int(province),
                "supply": item.get("supply"),
                "penalty": item.get("penalty"),
                "supply_raw_expected": supply_raw,
                "penalty_raw_expected": penalty_raw,
                "source": item.get("source"),
            })
    return {
        "path": str(path),
        "save_date": data.get("meta", {}).get("save_date"),
        "player": data.get("meta", {}).get("player"),
        "by_tag": dict(sorted(by_tag.items())),
        "entry_count": sum(len(rows) for rows in by_tag.values()),
        "country_count": len(by_tag),
        "disrupted_count": int(data.get("meta", {}).get("stats", {}).get("countries_with_disruption") or 0),
    }


def _read_i32(blob: bytes, off: int) -> int:
    return struct.unpack_from("<i", blob, off)[0]


def _read_u32(blob: bytes, off: int) -> int:
    return struct.unpack_from("<I", blob, off)[0]


def _plausible_pressure_record(province: int, supply_raw: int, penalty_raw: int) -> bool:
    if not (0 < province <= MAX_PROVINCE_ID):
        return False
    if supply_raw < -5_000_000 or supply_raw > 10_000_000:
        return False
    if penalty_raw < -1_000_000 or penalty_raw > 1_000_000:
        return False
    return True


def decode_layout(blob: bytes, layout: Layout) -> List[Dict[str, Any]]:
    max_rel = max(
        layout.province_off + 4,
        layout.supply_off + 4,
        layout.penalty_off + 4,
        (layout.duplicate_province_off or 0) + 4,
    )
    rows: List[Dict[str, Any]] = []
    if len(blob) < max_rel:
        return rows
    for off in range(0, len(blob) - max_rel + 1, layout.stride):
        province = _read_u32(blob, off + layout.province_off)
        supply_raw = _read_i32(blob, off + layout.supply_off)
        penalty_raw = _read_i32(blob, off + layout.penalty_off)
        duplicate = (
            _read_u32(blob, off + layout.duplicate_province_off)
            if layout.duplicate_province_off is not None
            else None
        )
        if not _plausible_pressure_record(province, supply_raw, penalty_raw):
            continue
        rows.append({
            "record_offset": off,
            "province": province,
            "supply_raw": supply_raw,
            "penalty_raw": penalty_raw,
            "duplicate_province": duplicate,
            "duplicate_matches": duplicate == province if duplicate is not None else None,
        })
    return rows


def _index_by_province(rows: Sequence[Dict[str, Any]]) -> Dict[int, List[Dict[str, Any]]]:
    out: Dict[int, List[Dict[str, Any]]] = defaultdict(list)
    for row in rows:
        out[int(row["province"])].append(row)
    return out


def _raw_match(actual: int, expected: Optional[int]) -> Tuple[bool, bool, Optional[int]]:
    if expected is None:
        return False, False, None
    delta = abs(actual - expected)
    return delta == 0, delta <= RAW_ROUNDED_TOLERANCE, delta


def _badge_rows() -> List[Tuple[int, ...]]:
    return [
        (
            live_probe.ROW_KIND_COMPUTED_BADGE,
            0,
            0,
            0,
            live_probe.ROW_FLAG_EXACT_SUPPLY_WALL,
            live_probe.BADGE_EXACT_PROVINCE_SUPPLY_TRANSIENT,
            0,
            0,
            0,
            0,
        ),
        (
            live_probe.ROW_KIND_COMPUTED_BADGE,
            0,
            0,
            0,
            live_probe.ROW_FLAG_ATTRITION_WALL,
            live_probe.BADGE_ATTRITION_RUNTIME_COMPUTED,
            0,
            0,
            0,
            0,
        ),
        (
            live_probe.ROW_KIND_COMPUTED_BADGE,
            0,
            0,
            0,
            live_probe.ROW_FLAG_MOTORIZATION_WALL,
            live_probe.BADGE_MOTORIZATION_SETTINGS_UNPINNED,
            0,
            0,
            0,
            0,
        ),
        (
            live_probe.ROW_KIND_COMPUTED_BADGE,
            0,
            0,
            0,
            live_probe.ROW_FLAG_CONVOY_PRESSURE_WALL,
            live_probe.BADGE_CONVOY_LOGISTICS_PRESSURE_UNPINNED,
            0,
            0,
            0,
            0,
        ),
        (
            live_probe.ROW_KIND_COMPUTED_BADGE,
            0,
            0,
            0,
            live_probe.ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED,
            live_probe.BADGE_CSUPPLY_INPUT_ARRAYS_UNSAFE,
            0,
            0,
            0,
            0,
        ),
    ]


def build_pressure_payload(candidate_rows: Sequence[Dict[str, Any]]) -> bytes:
    pressure_rows: List[Tuple[int, ...]] = []
    seen: Set[Tuple[int, int, int]] = set()
    for row in candidate_rows:
        province = int(row["province"])
        tag_id = int(row["tag_db_id"])
        record_offset = int(row["record_offset_raw"])
        key = (tag_id, province, record_offset)
        if key in seen:
            continue
        seen.add(key)
        pressure_rows.append((
            live_probe.ROW_KIND_FOREIGN_HOMEBASE_PRESSURE_CANDIDATE,
            province,
            0,
            tag_id,
            PRESSURE_FLAGS,
            int(row["supply_raw"]),
            int(row["penalty_raw"]),
            province,
            record_offset,
            0,
        ))
        if len(pressure_rows) >= MAX_COMPACT_CANDIDATE_ROWS:
            break
    rows = pressure_rows + _badge_rows()
    header = live_probe.HEADER_STRUCT.pack(
        0,
        len(rows),
        len(rows),
        live_probe.STATUS_COMPACT_HEALTH_OK,
        live_probe.FIXED_SCALE_RAW,
        0,
        0,
        5,
        0,
        0,
        0,
        len(pressure_rows),
    )
    body = b"".join(live_probe.ROW_STRUCT.pack(*row) for row in rows)
    return header + body


def score_decoded_rows(
    decoded: Sequence[Dict[str, Any]],
    anchors: Sequence[Dict[str, Any]],
) -> Dict[str, Any]:
    by_province = _index_by_province(decoded)
    score = {
        "anchors": len(anchors),
        "decoded_pressure_records": len(decoded),
        "province_matches": 0,
        "supply_exact_raw_matches": 0,
        "supply_rounded_matches": 0,
        "penalty_exact_raw_matches": 0,
        "penalty_rounded_matches": 0,
        "both_exact_raw_matches": 0,
        "both_rounded_matches": 0,
        "duplicate_province_matches": 0,
        "missing": [],
        "mismatches": [],
        "samples": [],
    }
    for anchor in anchors:
        province = int(anchor["province"])
        candidates = by_province.get(province) or []
        if not candidates:
            score["missing"].append(anchor)
            continue
        score["province_matches"] += 1
        expected_supply = anchor.get("supply_raw_expected")
        expected_penalty = anchor.get("penalty_raw_expected")

        ranked: List[Tuple[int, Dict[str, Any], Dict[str, Any]]] = []
        for candidate in candidates:
            supply_exact, supply_round, supply_delta = _raw_match(
                int(candidate["supply_raw"]),
                expected_supply,
            )
            penalty_exact, penalty_round, penalty_delta = _raw_match(
                int(candidate["penalty_raw"]),
                expected_penalty,
            )
            badness = (supply_delta or 9_999_999) + (penalty_delta or 9_999_999)
            ranked.append((badness, candidate, {
                "supply_exact": supply_exact,
                "supply_round": supply_round,
                "supply_delta": supply_delta,
                "penalty_exact": penalty_exact,
                "penalty_round": penalty_round,
                "penalty_delta": penalty_delta,
            }))
        _badness, best, match = sorted(ranked, key=lambda item: item[0])[0]
        if match["supply_exact"]:
            score["supply_exact_raw_matches"] += 1
        if match["supply_round"]:
            score["supply_rounded_matches"] += 1
        if match["penalty_exact"]:
            score["penalty_exact_raw_matches"] += 1
        if match["penalty_round"]:
            score["penalty_rounded_matches"] += 1
        if match["supply_exact"] and match["penalty_exact"]:
            score["both_exact_raw_matches"] += 1
        if match["supply_round"] and match["penalty_round"]:
            score["both_rounded_matches"] += 1
        if best.get("duplicate_matches") is True:
            score["duplicate_province_matches"] += 1

        sample = {
            "province": province,
            "expected_supply_raw": expected_supply,
            "expected_penalty_raw": expected_penalty,
            "record_offset": f"+0x{int(best['record_offset']):X}",
            "supply_raw": int(best["supply_raw"]),
            "penalty_raw": int(best["penalty_raw"]),
            "duplicate_province": best.get("duplicate_province"),
            "match": match,
        }
        if len(score["samples"]) < MAX_REPORTED:
            score["samples"].append(sample)
        if not (match["supply_round"] and match["penalty_round"]) and len(score["mismatches"]) < MAX_REPORTED:
            score["mismatches"].append(sample)
    score["missing"] = score["missing"][:MAX_REPORTED]
    return score


def _score_rank(score: Dict[str, Any]) -> Tuple[int, int, int, int, int]:
    return (
        int(score.get("both_rounded_matches", 0)),
        int(score.get("both_exact_raw_matches", 0)),
        int(score.get("province_matches", 0)),
        int(score.get("duplicate_province_matches", 0)),
        -len(score.get("mismatches", [])),
    )


def scan_layouts(
    p: Any,
    ccss_objects: Sequence[int],
    country_to_tag: Dict[int, Tuple[int, str]],
    anchors: Dict[str, Any],
) -> Dict[str, Any]:
    by_tag = anchors["by_tag"]
    ccss_by_tag: Dict[str, List[Tuple[int, int]]] = defaultdict(list)
    for obj in ccss_objects:
        country = safe_u64(p, obj + CCSS_COUNTRY_BACKPTR)
        if country not in country_to_tag:
            continue
        tag_id, tag = country_to_tag[country]
        if tag in by_tag:
            ccss_by_tag[tag].append((obj, tag_id))

    layout_scores: List[Dict[str, Any]] = []
    for layout in LAYOUTS:
        aggregate = {
            "layout": layout.name,
            "field_offset": f"+0x{layout.field_off:X}",
            "stride": f"0x{layout.stride:X}",
            "province_offset": f"+0x{layout.province_off:X}",
            "supply_offset": f"+0x{layout.supply_off:X}",
            "penalty_offset": f"+0x{layout.penalty_off:X}",
            "duplicate_province_offset": (
                f"+0x{layout.duplicate_province_off:X}"
                if layout.duplicate_province_off is not None
                else None
            ),
            "tags_tested": 0,
            "anchors": 0,
            "decoded_pressure_records": 0,
            "province_matches": 0,
            "supply_exact_raw_matches": 0,
            "supply_rounded_matches": 0,
            "penalty_exact_raw_matches": 0,
            "penalty_rounded_matches": 0,
            "both_exact_raw_matches": 0,
            "both_rounded_matches": 0,
            "duplicate_province_matches": 0,
            "chosen_objects": [],
            "mismatches": [],
            "missing": [],
            "samples": [],
        }
        for tag, rows in by_tag.items():
            object_scores: List[Tuple[Tuple[int, int, int, int, int], Dict[str, Any]]] = []
            for obj, tag_id in ccss_by_tag.get(tag, []):
                ptr = safe_u64(p, obj + layout.field_off)
                if not is_heap(ptr):
                    continue
                blob = p.try_read(ptr, REGION_SCAN_BYTES)
                if len(blob) < layout.stride:
                    continue
                decoded = decode_layout(blob, layout)
                score = score_decoded_rows(decoded, rows)
                score.update({
                    "tag": tag,
                    "tag_db_id": tag_id,
                    "ccountry_supply_system": hx(obj),
                    "region": hx(ptr),
                })
                object_scores.append((_score_rank(score), score))
            if not object_scores:
                aggregate["missing"].extend({"tag": tag, **row} for row in rows[:MAX_REPORTED])
                continue
            _rank, best = sorted(object_scores, key=lambda item: item[0], reverse=True)[0]
            aggregate["tags_tested"] += 1
            for key in (
                "anchors",
                "decoded_pressure_records",
                "province_matches",
                "supply_exact_raw_matches",
                "supply_rounded_matches",
                "penalty_exact_raw_matches",
                "penalty_rounded_matches",
                "both_exact_raw_matches",
                "both_rounded_matches",
                "duplicate_province_matches",
            ):
                aggregate[key] += int(best.get(key, 0))
            aggregate["chosen_objects"].append({
                "tag": tag,
                "tag_db_id": best.get("tag_db_id"),
                "ccountry_supply_system": best.get("ccountry_supply_system"),
                "region": best.get("region"),
                "anchors": best.get("anchors"),
                "province_matches": best.get("province_matches"),
                "both_rounded_matches": best.get("both_rounded_matches"),
                "both_exact_raw_matches": best.get("both_exact_raw_matches"),
                "decoded_pressure_records": best.get("decoded_pressure_records"),
            })
            if layout.name == "ccss_1e0_stride70_primary":
                object_best = next(
                    (
                        score
                        for _rank_item, score in object_scores
                        if score.get("ccountry_supply_system") == best.get("ccountry_supply_system")
                    ),
                    None,
                )
                if object_best:
                    ptr = safe_u64(
                        p,
                        int(best["ccountry_supply_system"], 16) + layout.field_off,
                    )
                    if is_heap(ptr):
                        decoded = decode_layout(p.try_read(ptr, REGION_SCAN_BYTES), layout)
                        for decoded_row in decoded:
                            if decoded_row.get("duplicate_matches") is not True:
                                continue
                            aggregate.setdefault("compact_candidate_rows", []).append({
                                "tag": tag,
                                "tag_db_id": best.get("tag_db_id"),
                                "province": decoded_row["province"],
                                "supply_raw": decoded_row["supply_raw"],
                                "penalty_raw": decoded_row["penalty_raw"],
                                "record_offset": f"+0x{int(decoded_row['record_offset']):X}",
                                "record_offset_raw": int(decoded_row["record_offset"]),
                                "ccountry_supply_system": best.get("ccountry_supply_system"),
                                "region": best.get("region"),
                                "layout": layout.name,
                            })
            aggregate["mismatches"].extend(best.get("mismatches", [])[:MAX_REPORTED])
            aggregate["missing"].extend(best.get("missing", [])[:MAX_REPORTED])
            aggregate["samples"].extend(best.get("samples", [])[:MAX_REPORTED])
        for key in ("chosen_objects", "mismatches", "missing", "samples"):
            aggregate[key] = aggregate[key][:MAX_REPORTED]
        if "compact_candidate_rows" in aggregate:
            aggregate["compact_candidate_row_count_full"] = len(aggregate["compact_candidate_rows"])
            aggregate["compact_candidate_rows"] = aggregate["compact_candidate_rows"][
                :MAX_COMPACT_CANDIDATE_ROWS
            ]
        layout_scores.append(aggregate)

    layout_scores.sort(
        key=lambda item: (
            int(item["both_rounded_matches"]),
            int(item["both_exact_raw_matches"]),
            int(item["province_matches"]),
            int(item["duplicate_province_matches"]),
        ),
        reverse=True,
    )
    best = layout_scores[0] if layout_scores else {}
    if best and best.get("layout") == "ccss_1e0_stride70_primary" and best.get("both_rounded_matches", 0) > 0:
        verdict = "foreign_homebase_pressure_record_layout_validated_candidate"
    elif best and best.get("both_rounded_matches", 0) > 0:
        verdict = "foreign_homebase_pressure_record_layout_shifted"
    elif best and best.get("province_matches", 0) > 0:
        verdict = "foreign_homebase_pressure_record_layout_rejected_values"
    else:
        verdict = "foreign_homebase_pressure_record_layout_rejected"
    return {
        "verdict": verdict,
        "ccss_anchor_tags": sorted(ccss_by_tag),
        "ccss_anchor_tag_count": len(ccss_by_tag),
        "missing_anchor_tags": sorted(set(by_tag) - set(ccss_by_tag)),
        "layouts": layout_scores,
        "best_layout": best,
        "production_safe_pressure_rows": 0,
        "same_frame_required": True,
    }


def _write_outputs(out_dir: Path, report: Dict[str, Any], payload: Optional[bytes] = None) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    if payload is not None:
        payload_path = out_dir / "domain44_pressure_candidate_payload.bin"
        payload_path.write_bytes(payload)
        report["compact_payload"] = {
            "path": str(payload_path),
            "bytes": len(payload),
            "domain_id": live_probe.DOMAIN_ID,
            "domain_name": live_probe.DOMAIN_NAME,
            "pressure_candidate_rows": report.get("record_scan", {})
            .get("best_layout", {})
            .get("compact_candidate_row_count_full", 0),
            "candidate_only": True,
            "mandatory_pressure_wall_still_required": True,
        }
    (out_dir / "foreign_homebase_pressure_record_probe.json").write_text(
        json.dumps(report, indent=2),
        encoding="utf-8",
    )
    scan = report.get("record_scan", {})
    best = scan.get("best_layout", {})
    lines = [
        "# supply_logistics_attrition_live foreign-homebase pressure record probe",
        "",
        f"- status: `{report.get('status')}`",
        f"- exe_sha256: `{report.get('process', {}).get('sha256')}`",
        f"- save_date: `{report.get('save_oracle', {}).get('save_date')}`",
        f"- live_date: `{report.get('live_date', {}).get('decoded')}`",
        f"- foreign_homebase_entries: `{report.get('save_oracle', {}).get('entry_count')}`",
        f"- joined_anchor_tags: `{scan.get('ccss_anchor_tag_count')}`",
        f"- missing_anchor_tags: `{scan.get('missing_anchor_tags')}`",
        f"- disrupted_anchor_countries: `{report.get('save_oracle', {}).get('disrupted_count')}`",
        f"- compact_payload: `{report.get('compact_payload', {}).get('path', 'none')}`",
        f"- pressure_candidate_rows: `{report.get('compact_payload', {}).get('pressure_candidate_rows', 0)}`",
        "",
        "## Best Layout",
        "",
        f"- layout: `{best.get('layout')}`",
        f"- field_offset: `{best.get('field_offset')}`",
        f"- stride: `{best.get('stride')}`",
        f"- province_offset: `{best.get('province_offset')}`",
        f"- supply_offset: `{best.get('supply_offset')}` as `i32/100000`",
        f"- penalty_offset: `{best.get('penalty_offset')}` as `i32/100000`",
        f"- duplicate_province_offset: `{best.get('duplicate_province_offset')}`",
        f"- anchors tested: `{best.get('anchors')}`",
        f"- province matches: `{best.get('province_matches')}`",
        f"- both exact raw matches: `{best.get('both_exact_raw_matches')}`",
        f"- both rounded matches: `{best.get('both_rounded_matches')}`",
        f"- duplicate province matches: `{best.get('duplicate_province_matches')}`",
        f"- compact candidate rows: `{best.get('compact_candidate_row_count_full', 0)}`",
        "",
        "## Competing Layout Scores",
        "",
    ]
    for row in scan.get("layouts", [])[:8]:
        lines.append(
            f"- `{row.get('layout')}` field `{row.get('field_offset')}` stride `{row.get('stride')}`: "
            f"province `{row.get('province_matches')}/{row.get('anchors')}`, "
            f"both-rounded `{row.get('both_rounded_matches')}`, "
            f"both-exact `{row.get('both_exact_raw_matches')}`, "
            f"dup-province `{row.get('duplicate_province_matches')}`."
        )
    lines.extend([
        "",
        "## Samples",
        "",
    ])
    for sample in best.get("samples", [])[:16]:
        lines.append(f"- `{sample}`")
    if best.get("mismatches"):
        lines.extend(["", "## Mismatches", ""])
        for sample in best.get("mismatches", [])[:16]:
            lines.append(f"- `{sample}`")
    if best.get("missing"):
        lines.extend(["", "## Missing Anchors", ""])
        for sample in best.get("missing", [])[:16]:
            lines.append(f"- `{sample}`")
    lines.extend([
        "",
        "## Production Verdict",
        "",
        "- No production pressure compact row is emitted by this probe.",
        "- The lane-local compact payload, when present, carries candidate pressure rows only and keeps the mandatory convoy-pressure wall badge.",
        "- `CCountrySupplySystem+0x1E0` is a concrete record-layout candidate only if the best layout is `ccss_1e0_stride70_primary` with fixed-point value matches.",
        "- Same-frame foreign-homebase anchors are still required before the row is allowed into the DLL/Rust production stream.",
        "- Disrupted-supply pressure cannot be validated from this fixture because the save oracle has zero disrupted-supply countries.",
    ])
    (out_dir / "FOREIGN_HOMEBASE_PRESSURE_RECORD_PROBE.md").write_text(
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
    payload: Optional[bytes] = None
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
        country_to_tag = {
            country: (tag_id, countries["id_to_tag"].get(tag_id, ""))
            for tag_id, country in countries["id_to_country"].items()
        }
        scan = scan_layouts(p, ccss_objects, country_to_tag, anchors)
        best = scan.get("best_layout", {})
        compact_rows = best.get("compact_candidate_rows") or []
        if scan.get("verdict") == "foreign_homebase_pressure_record_layout_validated_candidate" and compact_rows:
            payload = build_pressure_payload(compact_rows)
        report.update({
            "status": scan["verdict"],
            "live_date": live_date,
            "save_oracle": {
                "path": anchors["path"],
                "save_date": anchors["save_date"],
                "player": anchors["player"],
                "entry_count": anchors["entry_count"],
                "country_count": anchors["country_count"],
                "disrupted_count": anchors["disrupted_count"],
            },
            "summary": {
                "ccountry_supply_system_objects": len(ccss_objects),
                "live_countries": len(countries["id_to_country"]),
                "production_safe_pressure_rows": 0,
            },
            "record_scan": scan,
        })
    except Exception as exc:  # noqa: BLE001
        report["status"] = "probe_failed"
        report["reason"] = str(exc)
    finally:
        p.__exit__()
    report["elapsed_s"] = round(time.time() - started, 3)
    _write_outputs(out_dir, report, payload if report.get("status") == "foreign_homebase_pressure_record_layout_validated_candidate" else None)
    return report


def default_out_dir() -> Path:
    stamp = _dt.datetime.now().strftime("%Y%m%d_%H%M%S")
    return REPO_ROOT / "extractor/native" / "runs" / f"supply_logistics_attrition_live_foreign_pressure_records_{stamp}"


def main(argv: Optional[Iterable[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, default=None, help="hoi4.exe pid; defaults to process name lookup")
    parser.add_argument("--supply-json", type=Path, default=DEFAULT_SUPPLY_JSON, help="save-oracle supply.json anchors")
    parser.add_argument("--out-dir", type=Path, default=None, help="proof output directory")
    args = parser.parse_args(list(argv) if argv is not None else None)
    out_dir = args.out_dir or default_out_dir()
    report = run(args.pid, args.supply_json, out_dir)
    best = report.get("record_scan", {}).get("best_layout", {})
    print(json.dumps({
        "status": report.get("status"),
        "out_dir": str(out_dir),
        "live_date": report.get("live_date", {}).get("decoded"),
        "save_date": report.get("save_oracle", {}).get("save_date"),
        "foreign_homebase_entries": report.get("save_oracle", {}).get("entry_count"),
        "joined_anchor_tags": report.get("record_scan", {}).get("ccss_anchor_tag_count"),
        "best_layout": best.get("layout"),
        "field_offset": best.get("field_offset"),
        "stride": best.get("stride"),
        "province_matches": best.get("province_matches"),
        "anchors": best.get("anchors"),
        "both_rounded_matches": best.get("both_rounded_matches"),
        "both_exact_raw_matches": best.get("both_exact_raw_matches"),
        "elapsed_s": report.get("elapsed_s"),
    }, indent=2))
    return 0 if report.get("status") in (
        "foreign_homebase_pressure_record_layout_validated_candidate",
        "foreign_homebase_pressure_record_layout_shifted",
        "foreign_homebase_pressure_record_layout_rejected_values",
        "foreign_homebase_pressure_record_layout_rejected",
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
