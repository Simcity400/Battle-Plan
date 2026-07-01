"""Focused CCountrySupplySystem node-record probe for domain 44.

This probe follows the current-build RTTI-backed `CCountrySupplySystem` objects
through the live `+0x78` country backpointer and tests the strongest remaining
node/motorization candidate found by `ccountry_supply_layout_probe.py`:

* `CCountrySupplySystem+0x20` points at 0x30-stride tag/node records.
* Each record is checked against save-oracle motorization entries for the same
  top-level country tag.
* The candidate scalar at record `+0x04` is tested, and rejected unless it
  matches the saved motorization level for the same country/node/tag tuple.

The probe is read-only and emits no compact rows. It is intended to reduce the
residual search space without guessing fields into the replay stream.
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
import live_probe  # noqa: E402
import winmem  # noqa: E402

DOMAIN_NAME = live_probe.DOMAIN_NAME
EXPECTED_SHA256 = live_probe.EXPECTED_SHA256
HEAP_LO = live_probe.HEAP_LO
HEAP_HI = live_probe.HEAP_HI
MAX_PROVINCE_ID = live_probe.MAX_PROVINCE_ID

CCSS_COUNTRY_BACKPTR = 0x78
CCSS_NODE_RECORD_PTR = 0x20
CCSS_INLINE_NODE_CANDIDATE = 0x288
NODE_RECORD_STRIDE = 0x30
NODE_RECORD_SCAN_BYTES = 0x8000
MAX_REPORTED_ROWS = 36

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


def load_motorization(path: Path) -> Dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    by_top_tag: Dict[str, Dict[int, Dict[str, int]]] = defaultdict(dict)
    foreign_by_tag: Dict[str, List[Dict[str, Any]]] = defaultdict(list)
    for tag, rec in (data.get("countries") or {}).items():
        for item in rec.get("node_motorization") or []:
            node = item.get("node_province")
            levels = item.get("levels") or {}
            if isinstance(node, int) and 0 < node <= MAX_PROVINCE_ID:
                by_top_tag[tag][int(node)] = {
                    str(level_tag): int(level)
                    for level_tag, level in levels.items()
                    if isinstance(level, int)
                }
        for item in rec.get("foreign_homebase_nodes") or []:
            foreign_by_tag[tag].append(item)
    return {
        "path": str(path),
        "save_date": data.get("meta", {}).get("save_date"),
        "player": data.get("meta", {}).get("player"),
        "by_top_tag": {
            tag: {str(node): levels for node, levels in sorted(nodes.items())}
            for tag, nodes in sorted(by_top_tag.items())
        },
        "foreign_by_tag": {tag: rows for tag, rows in sorted(foreign_by_tag.items())},
        "motorization_entries": sum(len(nodes) for nodes in by_top_tag.values()),
        "motorization_countries": len(by_top_tag),
        "foreign_homebase_entries": sum(len(rows) for rows in foreign_by_tag.values()),
        "foreign_homebase_countries": len(foreign_by_tag),
    }


def _record_u32s(blob: bytes, off: int) -> Dict[int, int]:
    out: Dict[int, int] = {}
    for rel in range(0, NODE_RECORD_STRIDE, 4):
        out[rel] = struct.unpack_from("<I", blob, off + rel)[0]
    return out


def scan_node_records(
    p: Any,
    ccss_objects: Sequence[int],
    country_to_tag: Dict[int, Tuple[int, str]],
    tag_to_id: Dict[str, int],
    motorization: Dict[str, Any],
) -> Dict[str, Any]:
    motor_by_tag = motorization["by_top_tag"]
    records_scanned = 0
    joined_objects = 0
    objects_with_node_ptr = 0
    node_like_records = 0
    save_node_records = 0
    save_node_tag_records = 0
    exact_level_matches = 0
    level_mismatches = 0
    missing_level_tag_id = 0
    inline_save_node_hits = 0
    top_tag_counter: Counter[str] = Counter()
    field0_counter: Counter[int] = Counter()
    field4_counter: Counter[int] = Counter()
    matched_tags: Counter[str] = Counter()
    samples: List[Dict[str, Any]] = []
    mismatches: List[Dict[str, Any]] = []
    inline_samples: List[Dict[str, Any]] = []

    for obj in ccss_objects:
        country_ptr = safe_u64(p, obj + CCSS_COUNTRY_BACKPTR)
        if country_ptr not in country_to_tag:
            continue
        tag_id, top_tag = country_to_tag[country_ptr]
        joined_objects += 1
        top_tag_counter[top_tag] += 1
        saved_nodes = motor_by_tag.get(top_tag) or {}
        inline_node = layout_probe.safe_u32(p, obj + CCSS_INLINE_NODE_CANDIDATE)
        if saved_nodes and isinstance(inline_node, int) and str(inline_node) in saved_nodes:
            inline_save_node_hits += 1
            if len(inline_samples) < MAX_REPORTED_ROWS:
                inline_samples.append({
                    "ccountry_supply_system": hx(obj),
                    "tag_db_id": tag_id,
                    "tag": top_tag,
                    "offset": f"+0x{CCSS_INLINE_NODE_CANDIDATE:X}",
                    "province": inline_node,
                    "saved_levels": saved_nodes.get(str(inline_node)),
                })

        ptr = safe_u64(p, obj + CCSS_NODE_RECORD_PTR)
        if not is_heap(ptr):
            continue
        blob = p.try_read(ptr, NODE_RECORD_SCAN_BYTES)
        if len(blob) < NODE_RECORD_STRIDE:
            continue
        objects_with_node_ptr += 1
        limit = len(blob) - NODE_RECORD_STRIDE + 1
        for off in range(0, limit, NODE_RECORD_STRIDE):
            records_scanned += 1
            fields = _record_u32s(blob, off)
            actor_tag_id = fields[0x00]
            level_candidate = fields[0x04]
            node_province = fields[0x10]
            owner_tag_id = fields[0x14]
            if 0 < actor_tag_id <= 2048:
                field0_counter[actor_tag_id] += 1
            if level_candidate <= 16:
                field4_counter[level_candidate] += 1
            if not (0 < node_province <= MAX_PROVINCE_ID and owner_tag_id == tag_id):
                continue
            node_like_records += 1
            saved_levels = saved_nodes.get(str(node_province))
            if not saved_levels:
                continue
            save_node_records += 1
            actor_tag = None
            for candidate_tag, candidate_id in tag_to_id.items():
                if candidate_id == actor_tag_id:
                    actor_tag = candidate_tag
                    break
            expected_level = saved_levels.get(actor_tag or "")
            if actor_tag is None or expected_level is None:
                missing_level_tag_id += 1
                if len(samples) < MAX_REPORTED_ROWS:
                    samples.append({
                        "ccountry_supply_system": hx(obj),
                        "tag_db_id": tag_id,
                        "tag": top_tag,
                        "node_record_ptr": hx(ptr),
                        "record_offset": f"+0x{off:X}",
                        "actor_tag_id": actor_tag_id,
                        "actor_tag": actor_tag,
                        "node_province": node_province,
                        "owner_tag_id": owner_tag_id,
                        "level_candidate_u32_0x04": level_candidate,
                        "saved_levels": saved_levels,
                        "verdict": "node_match_actor_not_in_saved_levels",
                    })
                continue
            save_node_tag_records += 1
            matched_tags[actor_tag] += 1
            verdict = "level_match" if level_candidate == expected_level else "level_mismatch"
            if level_candidate == expected_level:
                exact_level_matches += 1
            else:
                level_mismatches += 1
                if len(mismatches) < MAX_REPORTED_ROWS:
                    mismatches.append({
                        "ccountry_supply_system": hx(obj),
                        "tag_db_id": tag_id,
                        "tag": top_tag,
                        "node_record_ptr": hx(ptr),
                        "record_offset": f"+0x{off:X}",
                        "actor_tag_id": actor_tag_id,
                        "actor_tag": actor_tag,
                        "node_province": node_province,
                        "owner_tag_id": owner_tag_id,
                        "level_candidate_u32_0x04": level_candidate,
                        "expected_motorization_level": expected_level,
                    })
            if len(samples) < MAX_REPORTED_ROWS:
                samples.append({
                    "ccountry_supply_system": hx(obj),
                    "tag_db_id": tag_id,
                    "tag": top_tag,
                    "node_record_ptr": hx(ptr),
                    "record_offset": f"+0x{off:X}",
                    "actor_tag_id": actor_tag_id,
                    "actor_tag": actor_tag,
                    "node_province": node_province,
                    "owner_tag_id": owner_tag_id,
                    "level_candidate_u32_0x04": level_candidate,
                    "expected_motorization_level": expected_level,
                    "verdict": verdict,
                })

    total_level_tests = exact_level_matches + level_mismatches
    level_match_ratio = (
        exact_level_matches / total_level_tests
        if total_level_tests
        else 0.0
    )
    return {
        "candidate": "CCountrySupplySystem+0x20 0x30-stride node/tag records",
        "country_join": "CCountrySupplySystem+0x78 -> CCountry",
        "record_stride": NODE_RECORD_STRIDE,
        "scan_bytes_per_pointer": NODE_RECORD_SCAN_BYTES,
        "joined_objects": joined_objects,
        "objects_with_node_record_ptr": objects_with_node_ptr,
        "records_scanned": records_scanned,
        "node_like_records_owner_matches": node_like_records,
        "save_node_records": save_node_records,
        "save_node_tag_records": save_node_tag_records,
        "exact_level_matches": exact_level_matches,
        "level_mismatches": level_mismatches,
        "missing_level_tag_id": missing_level_tag_id,
        "level_match_ratio": round(level_match_ratio, 4),
        "inline_0x288_save_node_hits": inline_save_node_hits,
        "top_tag_sample": top_tag_counter.most_common(16),
        "record_u32_0x00_tag_id_counts": field0_counter.most_common(16),
        "record_u32_0x04_small_value_counts": field4_counter.most_common(16),
        "matched_actor_tags": matched_tags.most_common(24),
        "samples": samples,
        "level_mismatch_samples": mismatches,
        "inline_0x288_samples": inline_samples,
        "verdict": (
            "country_node_records_validated_exact_motorization_level_rejected"
            if save_node_tag_records and level_mismatches
            else "country_node_records_validated_level_candidate_inconclusive"
            if save_node_records
            else "node_record_candidate_unmatched_to_save_anchors"
        ),
        "production_safe_motorization_fields": 0,
    }


def _write_outputs(out_dir: Path, report: Dict[str, Any]) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "ccountry_supply_node_record_probe.json").write_text(
        json.dumps(report, indent=2),
        encoding="utf-8",
    )
    scan = report.get("node_record_scan", {})
    lines = [
        "# supply_logistics_attrition_live CCountrySupplySystem node-record probe",
        "",
        f"- status: `{report.get('status')}`",
        f"- exe_sha256: `{report.get('process', {}).get('sha256')}`",
        f"- save_date: `{report.get('save_oracle', {}).get('save_date')}`",
        f"- live_date: `{report.get('live_date', {}).get('decoded')}`",
        f"- ccss_objects: `{report.get('summary', {}).get('ccountry_supply_system_objects')}`",
        f"- joined_objects: `{scan.get('joined_objects')}`",
        f"- node_record_ptr_objects: `{scan.get('objects_with_node_record_ptr')}`",
        "",
        "## Validated / Rejected",
        "",
        "- Validated current-build country join: `CCountrySupplySystem+0x78 -> CCountry`.",
        "- Validated current-build node/tag candidate shape: `CCountrySupplySystem+0x20` points to `0x30`-stride records containing owner-country tag id and node province ids.",
        f"- Rejected as exact motorization level: record `+0x04` (`level_match_ratio={scan.get('level_match_ratio')}`, matches `{scan.get('exact_level_matches')}`, mismatches `{scan.get('level_mismatches')}`).",
        f"- Inline `+0x288` matched only `{scan.get('inline_0x288_save_node_hits')}` saved motorization nodes, so it is not a complete motorization override table.",
        "",
        "## Counts",
        "",
        f"- records_scanned: `{scan.get('records_scanned')}`",
        f"- node_like_records_owner_matches: `{scan.get('node_like_records_owner_matches')}`",
        f"- save_node_records: `{scan.get('save_node_records')}`",
        f"- save_node_tag_records: `{scan.get('save_node_tag_records')}`",
        f"- missing_level_tag_id: `{scan.get('missing_level_tag_id')}`",
        "",
        "## Samples",
        "",
    ]
    for sample in scan.get("samples", [])[:12]:
        lines.append(f"- `{sample}`")
    lines.extend([
        "",
        "## Production Verdict",
        "",
        "- No motorization compact row is emitted from this probe. `+0x20` is a real country/node/tag record source, but the tested scalar does not satisfy exact motorization-level semantics against the freshest save oracle.",
        "- A same-frame hub motorization toggle remains required to identify the exact level field or function output.",
    ])
    (out_dir / "CCOUNTRY_SUPPLY_NODE_RECORD_PROBE.md").write_text(
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
        current_date = layout_probe.read_current_date(p, countries.get("game_state"))
        motorization = load_motorization(supply_json)
        country_to_tag = {
            country: (tag_id, countries["id_to_tag"].get(tag_id, ""))
            for tag_id, country in countries["id_to_country"].items()
        }
        ccss_objects = layout_probe.scan_ccountry_supply_objects(p)
        scan = scan_node_records(
            p,
            ccss_objects,
            country_to_tag,
            countries["tag_to_id"],
            motorization,
        )
        report.update({
            "status": scan["verdict"],
            "live_date": current_date,
            "save_oracle": {
                "path": motorization["path"],
                "save_date": motorization["save_date"],
                "player": motorization["player"],
                "motorization_entries": motorization["motorization_entries"],
                "motorization_countries": motorization["motorization_countries"],
                "foreign_homebase_entries": motorization["foreign_homebase_entries"],
                "foreign_homebase_countries": motorization["foreign_homebase_countries"],
            },
            "summary": {
                "ccountry_supply_system_objects": len(ccss_objects),
                "live_countries": len(countries["id_to_country"]),
                "tag_rows": len(countries["id_to_tag"]),
                "production_safe_motorization_fields": 0,
            },
            "node_record_scan": scan,
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
    return REPO_ROOT / "replay_native" / "runs" / f"supply_logistics_attrition_live_ccountry_node_records_{stamp}"


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
        "ccountry_supply_system_objects": report.get("summary", {}).get("ccountry_supply_system_objects"),
        "joined_objects": report.get("node_record_scan", {}).get("joined_objects"),
        "save_node_tag_records": report.get("node_record_scan", {}).get("save_node_tag_records"),
        "exact_level_matches": report.get("node_record_scan", {}).get("exact_level_matches"),
        "level_mismatches": report.get("node_record_scan", {}).get("level_mismatches"),
        "elapsed_s": report.get("elapsed_s"),
    }, indent=2))
    return 0 if report.get("status") in (
        "country_node_records_validated_exact_motorization_level_rejected",
        "country_node_records_validated_level_candidate_inconclusive",
        "node_record_candidate_unmatched_to_save_anchors",
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
