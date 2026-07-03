#!/usr/bin/env python3
"""Build-gated live probe for occupation garrison residual candidates.

This script is discovery-only. It uses the already validated Domain 52
occupation roots, dumps reachable CStateGarrisonData structures for occupied
states, compares them to a save-built oracle, and classifies candidate scalar
or detail offsets. It never promotes a root by itself.
"""

from __future__ import annotations

import argparse
import csv
import json
import math
import struct
import sys
import time
from pathlib import Path


HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parents[2]
LANE_DIR = REPO_ROOT / "extractor" / "native" / "lanes" / "occupation_resistance_compliance_live"
MEMORY_READER = REPO_ROOT / "extractor" / "memory_reader"
LIVE_LAYERS = MEMORY_READER / "live_layers"
for path in (str(LANE_DIR), str(MEMORY_READER), str(LIVE_LAYERS)):
    if path not in sys.path:
        sys.path.insert(0, path)

import garrison_scalar_probe as gsp  # noqa: E402
import live_probe  # noqa: E402
import remaining_fields_probe as rem  # noqa: E402
import tag_table  # noqa: E402
import winmem  # noqa: E402


EXPECTED_SHA256 = live_probe.EXPECTED_SHA256
SCHEMA_VERSION = 1


def hx(value):
    return "0x%X" % value if isinstance(value, int) else None


def now_utc():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True), encoding="utf-8")


def fail_closed_report(args, failure, process=None, detail=None):
    return {
        "schema": "occupation_garrison_candidate_probe",
        "schema_version": SCHEMA_VERSION,
        "lane": "occupation_resistance_compliance_live",
        "snapshot_label": args.snapshot_label,
        "expected_sha256": EXPECTED_SHA256,
        "process": process,
        "fail_closed": True,
        "failure": failure,
        "detail": detail,
        "started_utc": now_utc(),
        "completed_utc": now_utc(),
    }


def load_oracle(path):
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    if data.get("schema") != "occupation_garrison_oracle":
        raise ValueError("unsupported oracle schema: %s" % data.get("schema"))
    rows = {}
    for key, row in data.get("rows_by_key", {}).items():
        controller, occupied, state_id = key.split("|")
        rows[(controller, occupied, int(state_id))] = row
    return data, rows


def read_ui_oracle_csv(path):
    if not path:
        return None
    out = {}
    with Path(path).open("r", encoding="utf-8-sig", newline="") as handle:
        reader = csv.DictReader(handle)
        for row in reader:
            state_raw = (row.get("state_id") or "").strip()
            if not state_raw:
                continue
            state_id = int(state_raw)
            parsed = {"state_id": state_id}
            for key in (
                "garrison_manpower_demand",
                "garrison_equipment_demand",
                "suppression_ratio",
            ):
                raw = (row.get(key) or "").strip()
                if raw:
                    parsed[key] = float(raw)
            parsed["notes"] = row.get("notes") or ""
            out[state_id] = parsed
    return out


def numeric_at(blob, encoding, offset):
    try:
        if encoding == "i32":
            return struct.unpack_from("<i", blob, offset)[0]
        if encoding == "u32":
            return struct.unpack_from("<I", blob, offset)[0]
        if encoding == "fixed1000_i32":
            return struct.unpack_from("<i", blob, offset)[0] / 1000.0
        if encoding == "fixed10000_i32":
            return struct.unpack_from("<i", blob, offset)[0] / 10000.0
        if encoding == "fixed100000_i32":
            return struct.unpack_from("<i", blob, offset)[0] / 100000.0
        if encoding == "i64":
            return struct.unpack_from("<q", blob, offset)[0]
        if encoding == "u64":
            return struct.unpack_from("<Q", blob, offset)[0]
        if encoding == "fixed1000_i64":
            return struct.unpack_from("<q", blob, offset)[0] / 1000.0
        if encoding == "fixed10000_i64":
            return struct.unpack_from("<q", blob, offset)[0] / 10000.0
        if encoding == "fixed100000_i64":
            return struct.unpack_from("<q", blob, offset)[0] / 100000.0
        if encoding == "f32":
            value = struct.unpack_from("<f", blob, offset)[0]
            return value if math.isfinite(value) else None
        if encoding == "f64":
            value = struct.unpack_from("<d", blob, offset)[0]
            return value if math.isfinite(value) else None
    except struct.error:
        return None
    return None


def pinned_scalar_values(blob):
    out = {}
    for field, candidates in gsp.PINNED_SCALAR_CANDIDATES.items():
        rows = []
        for encoding, offset in candidates:
            rows.append(
                {
                    "encoding": encoding,
                    "offset": hx(offset),
                    "value": numeric_at(blob, encoding, offset),
                }
            )
        out[field] = rows
    return out


def oracle_detail_summary(row):
    if not row:
        return None
    manpower_values = [
        item.get("value")
        for item in row.get("manpower_pool") or []
        if isinstance(item.get("value"), (int, float))
    ]
    return {
        "law_name": row.get("law_name"),
        "strength_ratio": row.get("strength_ratio"),
        "garrison_required": row.get("garrison_required"),
        "army_manpower_need": row.get("army_manpower_need"),
        "manpower_pool_total": sum(manpower_values) if manpower_values else None,
        "manpower_pool_tags": [
            item.get("tag") for item in row.get("manpower_pool") or [] if item.get("tag")
        ],
        "equipment_need": row.get("equipment_need") or {},
        "reinforcement_request_count": len(row.get("reinforcement_requests") or []),
        "controller_garrison_template_id": row.get("controller_garrison_template_id"),
    }


def scrub_child_roots(children):
    rows = []
    for child in children or []:
        rows.append(
            {
                "parent_offset": hx(child.get("parent_offset")),
                "ptr": hx(child.get("ptr")),
                "vtable": hx(child.get("vtable")),
                "kind": child.get("kind"),
            }
        )
    return rows


def make_live_row_dump(live, oracle, ui_oracle):
    state_id = live["state_id"]
    return {
        "key": "%s|%s|%s" % (live["controller_tag"], live["occupied_tag"], state_id),
        "controller_tag": live["controller_tag"],
        "controller_tag_db_id": live["controller_tag_db_id"],
        "occupied_tag": live["occupied_tag"],
        "occupied_tag_db_id": live["occupied_tag_db_id"],
        "state_id": state_id,
        "law_name": live.get("law_name"),
        "entry_offset": hx(live.get("entry_offset")),
        "garrison_ptr": hx(live.get("garrison_ptr")),
        "selected_pinned_values": pinned_scalar_values(live["blob"]),
        "direct_child_roots": scrub_child_roots(live.get("direct_child_roots")),
        "oracle": oracle_detail_summary(oracle),
        "ui_oracle": ui_oracle.get(state_id) if ui_oracle else None,
    }


def compare_live_to_oracle(live_rows, oracle_rows):
    matched = []
    key_only = 0
    law_mismatches = []
    missing_oracle = []
    for key, live in sorted(live_rows.items()):
        oracle = oracle_rows.get(key)
        if not oracle:
            if len(missing_oracle) < 20:
                missing_oracle.append(
                    {
                        "controller_tag": key[0],
                        "occupied_tag": key[1],
                        "state_id": key[2],
                        "live_law": live.get("law_name"),
                    }
                )
            continue
        key_only += 1
        live_law = live.get("law_name")
        oracle_law = oracle.get("law_name")
        if live_law and oracle_law and live_law != oracle_law:
            if len(law_mismatches) < 20:
                law_mismatches.append(
                    {
                        "controller_tag": key[0],
                        "occupied_tag": key[1],
                        "state_id": key[2],
                        "live_law": live_law,
                        "oracle_law": oracle_law,
                    }
                )
            continue
        matched.append((live, oracle))
    return {
        "matched": matched,
        "key_only_matches": key_only,
        "same_law_matches": len(matched),
        "law_mismatches_sample": law_mismatches,
        "missing_oracle_sample": missing_oracle,
    }


def classify_ui_candidates(matched, ui_oracle):
    if not ui_oracle:
        return None
    field_names = (
        "garrison_manpower_demand",
        "garrison_equipment_demand",
        "suppression_ratio",
    )
    stats = {field: {} for field in field_names}
    compared_rows = {field: 0 for field in field_names}
    for live, _oracle in matched:
        ui_row = ui_oracle.get(live["state_id"])
        if not ui_row:
            continue
        for field in field_names:
            expected = ui_row.get(field)
            if expected is None:
                continue
            compared_rows[field] += 1
            for encoding, offset, actual in gsp.iter_candidate_values(live["blob"]):
                if not isinstance(actual, (int, float)):
                    continue
                if abs(float(actual)) > 100000000:
                    continue
                key = (encoding, offset)
                rec = stats[field].setdefault(
                    key,
                    {
                        "encoding": encoding,
                        "offset": offset,
                        "compared": 0,
                        "matches_exactish": 0,
                        "samples": [],
                    },
                )
                rec["compared"] += 1
                if abs(float(actual) - float(expected)) <= 0.01:
                    rec["matches_exactish"] += 1
                    if len(rec["samples"]) < 8:
                        rec["samples"].append(
                            {
                                "state_id": live["state_id"],
                                "controller_tag": live["controller_tag"],
                                "occupied_tag": live["occupied_tag"],
                                "expected": expected,
                                "actual": actual,
                                "garrison_ptr": hx(live["garrison_ptr"]),
                            }
                        )
    out = {}
    for field in field_names:
        ranked = sorted(
            stats[field].values(),
            key=lambda rec: (rec["matches_exactish"], rec["compared"]),
            reverse=True,
        )
        out[field] = {
            "ui_rows_compared": compared_rows[field],
            "top_inline_candidates": [
                {
                    "encoding": rec["encoding"],
                    "offset": hx(rec["offset"]),
                    "compared": rec["compared"],
                    "matches_exactish": rec["matches_exactish"],
                    "match_ratio": rec["matches_exactish"] / max(1, rec["compared"]),
                    "samples": rec["samples"],
                }
                for rec in ranked[:12]
            ],
            "verdict": (
                "ui_fixture_missing"
                if compared_rows[field] == 0
                else "ui_candidates_reported_no_promotion"
            ),
        }
    return {
        "contract": (
            "UI values are manual fallback evidence from the occupied-territories "
            "screen. Exactish candidate matches are reported only as candidates; "
            "a unique offset still needs same-build live and preferably before/after "
            "support before production wiring."
        ),
        "fields": out,
    }


def _collect_class_fields(validated, ambiguous, class_name, candidate_fields,
                          validated_field_verdict, ambiguous_field_verdict):
    """Surface EVERY per-field verdict of a candidate class into the flat
    validated/ambiguous lists, instead of collapsing the class to a single
    field="*" row. A class with one validated field and one ambiguous field
    must contribute BOTH so the 'exactly one scope crosses threshold' criterion
    counts every threshold-crossing field, not just the class's best one.
    """
    for field, rec in (candidate_fields or {}).items():
        verdict = rec.get("verdict")
        if verdict == validated_field_verdict:
            validated.append({"class": class_name, "field": field, "verdict": verdict})
        elif verdict == ambiguous_field_verdict:
            ambiguous.append({"class": class_name, "field": field, "verdict": verdict})


def summarize_candidate_verdict(candidate_fields, child_root_probe, detail_probe):
    validated = []
    ambiguous = []
    for field, rec in candidate_fields.items():
        verdict = rec.get("verdict")
        if verdict == "validated_candidate":
            validated.append({"class": "inline_scalar", "field": field, "verdict": verdict})
        elif verdict == "ambiguous_candidate_fixture_required":
            ambiguous.append({"class": "inline_scalar", "field": field, "verdict": verdict})

    # Direct-child and detail classes carry their own per-field verdicts in
    # candidate_fields; surface each field so a validated field in a class does
    # not hide a sibling ambiguous field from the top-level summary.
    _collect_class_fields(
        validated, ambiguous, "direct_child",
        child_root_probe.get("candidate_fields"),
        "validated_child_candidate", "ambiguous_child_candidate_fixture_required",
    )
    _collect_class_fields(
        validated, ambiguous, "detail",
        detail_probe.get("candidate_fields"),
        "validated_detail_candidate", "ambiguous_detail_candidate_fixture_required",
    )

    if validated:
        return {
            "verdict": "candidate_threshold_hit_review_required_no_promotion",
            "winning_candidate_classes": sorted({item["class"] for item in validated}),
            "validated_candidates": validated,
            "ambiguous_candidates": ambiguous,
        }
    if ambiguous:
        return {
            "verdict": "ambiguous_candidate_fixture_required_no_promotion",
            "winning_candidate_classes": [],
            "validated_candidates": [],
            "ambiguous_candidates": ambiguous,
        }
    return {
        "verdict": "no_candidate_promoted",
        "winning_candidate_classes": [],
        "validated_candidates": [],
        "ambiguous_candidates": [],
    }


def build_report(args, proc):
    started = time.time()
    oracle_data, oracle_rows = load_oracle(args.oracle)
    ui_oracle = read_ui_oracle_csv(args.ui_oracle_csv)
    fp = proc.fingerprint()
    process = {
        "pid": fp.get("pid"),
        "module": fp.get("module"),
        "base": hx(fp.get("base")),
        "size": fp.get("size"),
        "path": fp.get("path"),
        "sha256": fp.get("sha256"),
    }
    if fp.get("sha256") != EXPECTED_SHA256:
        return fail_closed_report(
            args,
            "build_sha_mismatch",
            process=process,
            detail={"actual": fp.get("sha256"), "expected": EXPECTED_SHA256},
        ), 2

    tbl = tag_table.TagTable.locate(proc)
    if tbl is None:
        raise RuntimeError("tag table not found")
    roots = live_probe.resolve_state_roots(proc)
    status_by_tag, status_summary = live_probe.read_country_occupation_status(
        proc, roots["game_state"], tbl
    )
    state_rows, invalid_states = live_probe.read_state_rows(
        proc, roots["state_roots"], tbl, status_by_tag
    )
    live_rows = gsp.live_garrison_rows(proc, state_rows, status_by_tag, tbl)
    compare = compare_live_to_oracle(live_rows, oracle_rows)
    matched = compare["matched"]
    tag_to_id = {tag: tag_id for tag_id, tag in tbl.as_map().items()}

    candidate_fields = gsp.classify_candidates(matched)
    child_root_probe = gsp.classify_child_candidates(matched)
    detail_probe = gsp.classify_detail_candidates(matched, oracle_rows, tag_to_id)
    ui_probe = classify_ui_candidates(matched, ui_oracle)
    object_graph = rem.probe_occupation_object_graph(
        proc, state_rows, status_by_tag, tbl, args.graph_countries
    )

    live_dumps = []
    for key, live in sorted(live_rows.items(), key=lambda item: item[0]):
        oracle = oracle_rows.get(key)
        live_dumps.append(make_live_row_dump(live, oracle, ui_oracle))

    candidate_verdict = summarize_candidate_verdict(
        candidate_fields, child_root_probe, detail_probe
    )
    if compare["same_law_matches"] == 0:
        verdict = "fixture_unmatched_no_promotion"
    else:
        verdict = candidate_verdict["verdict"]

    report = {
        "schema": "occupation_garrison_candidate_probe",
        "schema_version": SCHEMA_VERSION,
        "lane": "occupation_resistance_compliance_live",
        "snapshot_label": args.snapshot_label,
        "expected_sha256": EXPECTED_SHA256,
        "started_utc": now_utc(),
        "completed_utc": now_utc(),
        "elapsed_seconds": round(time.time() - started, 3),
        "fail_closed": False,
        "process": process,
        "oracle": {
            "path": str(args.oracle),
            "source": oracle_data.get("source"),
            "counts": oracle_data.get("counts"),
        },
        "roots": {
            "game_state": hx(roots["game_state"]),
            "province_array": hx(roots["province_array"]),
            "province_count": roots["province_count"],
            "province_rows": roots["province_rows"],
            "invalid_province_rows": roots["invalid_province_rows"],
            "invalid_state_links": roots["invalid_state_links"],
            "state_roots": len(roots["state_roots"]),
        },
        "status_summary": status_summary,
        "live_counts": {
            "state_rows": len(state_rows),
            "invalid_state_rows": invalid_states,
            "live_garrison_rows": len(live_rows),
        },
        "oracle_compare": {
            "oracle_rows": len(oracle_rows),
            "key_only_matches": compare["key_only_matches"],
            "same_law_matches": compare["same_law_matches"],
            "law_mismatches_sample": compare["law_mismatches_sample"],
            "missing_oracle_sample": compare["missing_oracle_sample"],
        },
        "object_graph": object_graph,
        "candidate_classification": {
            "inline_scalar_candidates": candidate_fields,
            "direct_child_candidates": child_root_probe,
            "detail_candidates": detail_probe,
            "ui_candidates": ui_probe,
        },
        "candidate_verdict_summary": candidate_verdict,
        "live_garrison_rows": live_dumps,
        "candidate_structure_sample": live_dumps[: args.max_rows],
        "verdict": verdict,
        "field_contract": (
            "This probe reports candidates only. Promotion requires same-build "
            "live evidence, exact save/UI oracle agreement, one unique threshold "
            "scope for the field, and no contradictory rejected-candidate proof. "
            "SHA mismatch, missing roots, or missing oracle matches fail closed."
        ),
    }
    return report, 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--oracle", required=True, help="garrison oracle JSON from build_garrison_oracle.py")
    parser.add_argument("--out", required=True, help="Output JSON proof path")
    parser.add_argument("--snapshot-label", default="single_live_snapshot")
    parser.add_argument("--max-rows", type=int, default=80)
    parser.add_argument("--graph-countries", type=int, default=40)
    parser.add_argument(
        "--ui-oracle-csv",
        default=None,
        help=(
            "Optional manual occupied-territories transcription CSV with columns "
            "state_id,garrison_manpower_demand,garrison_equipment_demand,"
            "suppression_ratio,notes."
        ),
    )
    args = parser.parse_args(argv)
    out_path = Path(args.out)

    proc = winmem.open_process("hoi4.exe")
    if proc is None:
        report = fail_closed_report(args, "process_not_running")
        write_json(out_path, report)
        return 2

    try:
        with proc:
            try:
                report, code = build_report(args, proc)
            except Exception as exc:
                report = fail_closed_report(
                    args,
                    "probe_exception",
                    detail={"exception": "%s: %s" % (type(exc).__name__, exc)},
                )
                code = 1
    finally:
        try:
            proc.close()
        except Exception:
            pass

    write_json(out_path, report)
    print(json.dumps({"out": str(out_path), "verdict": report.get("verdict"), "fail_closed": report.get("fail_closed")}, indent=2, sort_keys=True))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
