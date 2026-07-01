"""Hub motorization level oracle/field scan for domain 44.

The node-record probe validated `CCountrySupplySystem+0x20` as a country/node
record source, but rejected record `+0x04` as the exact motorization level. This
probe checks whether the current save oracle can identify an exact level field
at all:

* it reports the saved motorization level distribution;
* it scans each byte/u16/u32 field in matched `0x30` node records;
* it fails closed if the fixture has no level diversity, because a constant
  field matching level `2` would not prove motorization semantics.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import struct
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

LANE_DIR = Path(__file__).resolve().parent
REPO_ROOT = LANE_DIR.parents[2]
sys.path.insert(0, str(LANE_DIR))
sys.path.insert(0, str(REPO_ROOT / "memory_reader"))

import ccountry_supply_layout_probe as layout_probe  # noqa: E402
import ccountry_supply_node_record_probe as node_probe  # noqa: E402
import live_probe  # noqa: E402
import winmem  # noqa: E402

DOMAIN_NAME = live_probe.DOMAIN_NAME
HEAP_LO = live_probe.HEAP_LO
HEAP_HI = live_probe.HEAP_HI
MAX_PROVINCE_ID = live_probe.MAX_PROVINCE_ID

CCSS_COUNTRY_BACKPTR = 0x78
CCSS_NODE_RECORD_PTR = 0x20
NODE_RECORD_STRIDE = 0x30
NODE_RECORD_SCAN_BYTES = 0x8000
MAX_REPORTED = 48

DEFAULT_SUPPLY_JSON = (
    REPO_ROOT
    / "replay_native"
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


def level_distribution(motorization: Dict[str, Any]) -> Dict[str, Any]:
    counts: Counter[int] = Counter()
    pairs = 0
    for nodes in motorization["by_top_tag"].values():
        for levels in nodes.values():
            for level in levels.values():
                counts[int(level)] += 1
                pairs += 1
    return {
        "pairs": pairs,
        "levels": dict(sorted(counts.items())),
        "distinct_levels": len(counts),
        "has_discriminating_levels": len(counts) >= 2,
    }


def _field_values(record: bytes) -> Iterable[Tuple[str, int, int]]:
    for off in range(0, len(record)):
        value = record[off]
        if value <= 8:
            yield ("u8", off, value)
    for off in range(0, len(record) - 1, 2):
        value = struct.unpack_from("<H", record, off)[0]
        if value <= 8:
            yield ("u16", off, value)
    for off in range(0, len(record) - 3, 4):
        value = struct.unpack_from("<I", record, off)[0]
        if value <= 8:
            yield ("u32", off, value)


def scan_level_fields(
    p: Any,
    ccss_objects: Sequence[int],
    country_to_tag: Dict[int, Tuple[int, str]],
    tag_to_id: Dict[str, int],
    motorization: Dict[str, Any],
) -> Dict[str, Any]:
    id_to_tag = {tag_id: tag for tag, tag_id in tag_to_id.items()}
    saved_by_tag = motorization["by_top_tag"]
    candidate_scores: Dict[Tuple[str, int], Dict[str, Any]] = {}
    matched_records = 0
    node_owner_records = 0
    missing_actor_level = 0
    object_count = 0
    samples: List[Dict[str, Any]] = []

    for obj in ccss_objects:
        country = safe_u64(p, obj + CCSS_COUNTRY_BACKPTR)
        if country not in country_to_tag:
            continue
        tag_id, top_tag = country_to_tag[country]
        saved_nodes = saved_by_tag.get(top_tag) or {}
        if not saved_nodes:
            continue
        ptr = safe_u64(p, obj + CCSS_NODE_RECORD_PTR)
        if not is_heap(ptr):
            continue
        blob = p.try_read(ptr, NODE_RECORD_SCAN_BYTES)
        if len(blob) < NODE_RECORD_STRIDE:
            continue
        object_count += 1
        for off in range(0, len(blob) - NODE_RECORD_STRIDE + 1, NODE_RECORD_STRIDE):
            record = blob[off : off + NODE_RECORD_STRIDE]
            actor_tag_id = struct.unpack_from("<I", record, 0x00)[0]
            node_province = struct.unpack_from("<I", record, 0x10)[0]
            owner_tag_id = struct.unpack_from("<I", record, 0x14)[0]
            if not (0 < node_province <= MAX_PROVINCE_ID and owner_tag_id == tag_id):
                continue
            saved_levels = saved_nodes.get(str(node_province))
            if not saved_levels:
                continue
            node_owner_records += 1
            actor_tag = id_to_tag.get(actor_tag_id)
            expected = saved_levels.get(actor_tag or "")
            if expected is None:
                missing_actor_level += 1
                continue
            matched_records += 1
            expected = int(expected)
            if len(samples) < MAX_REPORTED:
                samples.append({
                    "ccountry_supply_system": hx(obj),
                    "tag": top_tag,
                    "tag_db_id": tag_id,
                    "node_record_ptr": hx(ptr),
                    "record_offset": f"+0x{off:X}",
                    "actor_tag_id": actor_tag_id,
                    "actor_tag": actor_tag,
                    "node_province": node_province,
                    "expected_level": expected,
                    "u32_0x04": struct.unpack_from("<I", record, 0x04)[0],
                })
            for encoding, field_off, value in _field_values(record):
                key = (encoding, field_off)
                row = candidate_scores.setdefault(key, {
                    "encoding": encoding,
                    "offset": field_off,
                    "tests": 0,
                    "exact_matches": 0,
                    "mismatches": 0,
                    "values": Counter(),
                    "expected_levels": Counter(),
                    "mismatch_samples": [],
                })
                row["tests"] += 1
                row["values"][value] += 1
                row["expected_levels"][expected] += 1
                if value == expected:
                    row["exact_matches"] += 1
                else:
                    row["mismatches"] += 1
                    if len(row["mismatch_samples"]) < 8:
                        row["mismatch_samples"].append({
                            "tag": top_tag,
                            "actor_tag": actor_tag,
                            "node_province": node_province,
                            "expected_level": expected,
                            "value": value,
                            "record_offset": f"+0x{off:X}",
                        })

    scored: List[Dict[str, Any]] = []
    for row in candidate_scores.values():
        tests = max(1, int(row["tests"]))
        scored.append({
            "encoding": row["encoding"],
            "offset": f"+0x{row['offset']:X}",
            "tests": row["tests"],
            "exact_matches": row["exact_matches"],
            "mismatches": row["mismatches"],
            "exact_ratio": round(row["exact_matches"] / tests, 4),
            "values": dict(row["values"].most_common(12)),
            "expected_levels": dict(row["expected_levels"].most_common(12)),
            "mismatch_samples": row["mismatch_samples"],
        })
    scored.sort(
        key=lambda item: (
            int(item["exact_matches"]),
            -int(item["mismatches"]),
            int(item["tests"]),
        ),
        reverse=True,
    )
    dist = level_distribution(motorization)
    all_match = [
        row for row in scored
        if row["tests"] == matched_records and row["mismatches"] == 0
    ]
    if not dist["has_discriminating_levels"]:
        verdict = "hub_motorization_level_fixture_blocked_single_level_oracle"
    elif all_match:
        verdict = "hub_motorization_level_candidate_found"
    elif matched_records:
        verdict = "hub_motorization_level_record_fields_rejected"
    else:
        verdict = "hub_motorization_level_no_matched_records"
    return {
        "verdict": verdict,
        "level_distribution": dist,
        "objects_with_node_record_ptr": object_count,
        "node_owner_records": node_owner_records,
        "matched_actor_level_records": matched_records,
        "missing_actor_level_records": missing_actor_level,
        "top_candidates": scored[:32],
        "ambiguous_all_match_candidates": all_match[:16],
        "samples": samples,
        "production_safe_motorization_fields": 0,
    }


def _write_outputs(out_dir: Path, report: Dict[str, Any]) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "hub_motorization_level_probe.json").write_text(
        json.dumps(report, indent=2),
        encoding="utf-8",
    )
    scan = report.get("level_scan", {})
    dist = scan.get("level_distribution", {})
    lines = [
        "# supply_logistics_attrition_live hub motorization level probe",
        "",
        f"- status: `{report.get('status')}`",
        f"- exe_sha256: `{report.get('process', {}).get('sha256')}`",
        f"- save_date: `{report.get('save_oracle', {}).get('save_date')}`",
        f"- live_date: `{report.get('live_date', {}).get('decoded')}`",
        f"- motorization_pairs: `{dist.get('pairs')}`",
        f"- saved_level_distribution: `{dist.get('levels')}`",
        f"- matched_actor_level_records: `{scan.get('matched_actor_level_records')}`",
        f"- missing_actor_level_records: `{scan.get('missing_actor_level_records')}`",
        "",
        "## Top Field Scores",
        "",
    ]
    for row in scan.get("top_candidates", [])[:16]:
        lines.append(
            f"- `{row.get('encoding')}@{row.get('offset')}`: "
            f"matches `{row.get('exact_matches')}/{row.get('tests')}` "
            f"ratio `{row.get('exact_ratio')}`, values `{row.get('values')}`."
        )
    lines.extend(["", "## Ambiguous All-Match Candidates", ""])
    for row in scan.get("ambiguous_all_match_candidates", [])[:16]:
        lines.append(
            f"- `{row.get('encoding')}@{row.get('offset')}` matched every tested record, "
            f"but the fixture only contains saved level distribution `{dist.get('levels')}`."
        )
    lines.extend(["", "## Samples", ""])
    for sample in scan.get("samples", [])[:16]:
        lines.append(f"- `{sample}`")
    lines.extend([
        "",
        "## Production Verdict",
        "",
        "- No motorization compact row is emitted by this probe.",
        "- The current save oracle cannot validate an exact level field because every saved motorization level is `2`.",
        "- A same-frame hub motorization fixture must include at least two known levels, ideally one node at level `1` and one at level `3`, before a constant-looking record field can be accepted.",
    ])
    (out_dir / "HUB_MOTORIZATION_LEVEL_PROBE.md").write_text(
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
        motorization = node_probe.load_motorization(supply_json)
        ccss_objects = layout_probe.scan_ccountry_supply_objects(p)
        country_to_tag = {
            country: (tag_id, countries["id_to_tag"].get(tag_id, ""))
            for tag_id, country in countries["id_to_country"].items()
        }
        scan = scan_level_fields(
            p,
            ccss_objects,
            country_to_tag,
            countries["tag_to_id"],
            motorization,
        )
        report.update({
            "status": scan["verdict"],
            "live_date": live_date,
            "save_oracle": {
                "path": motorization["path"],
                "save_date": motorization["save_date"],
                "player": motorization["player"],
                "motorization_entries": motorization["motorization_entries"],
                "motorization_countries": motorization["motorization_countries"],
            },
            "summary": {
                "ccountry_supply_system_objects": len(ccss_objects),
                "live_countries": len(countries["id_to_country"]),
                "production_safe_motorization_fields": 0,
            },
            "level_scan": scan,
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
    return REPO_ROOT / "replay_native" / "runs" / f"supply_logistics_attrition_live_hub_motorization_level_{stamp}"


def main(argv: Optional[Iterable[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, default=None, help="hoi4.exe pid; defaults to process name lookup")
    parser.add_argument("--supply-json", type=Path, default=DEFAULT_SUPPLY_JSON, help="save-oracle supply.json anchors")
    parser.add_argument("--out-dir", type=Path, default=None, help="proof output directory")
    args = parser.parse_args(list(argv) if argv is not None else None)
    out_dir = args.out_dir or default_out_dir()
    report = run(args.pid, args.supply_json, out_dir)
    scan = report.get("level_scan", {})
    print(json.dumps({
        "status": report.get("status"),
        "out_dir": str(out_dir),
        "live_date": report.get("live_date", {}).get("decoded"),
        "save_date": report.get("save_oracle", {}).get("save_date"),
        "level_distribution": scan.get("level_distribution"),
        "matched_actor_level_records": scan.get("matched_actor_level_records"),
        "top_candidates": scan.get("top_candidates", [])[:5],
        "elapsed_s": report.get("elapsed_s"),
    }, indent=2))
    return 0 if report.get("status") in (
        "hub_motorization_level_fixture_blocked_single_level_oracle",
        "hub_motorization_level_candidate_found",
        "hub_motorization_level_record_fields_rejected",
        "hub_motorization_level_no_matched_records",
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
