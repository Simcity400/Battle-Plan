#!/usr/bin/env python3
"""Probe CStateGarrisonData scalar semantics against save garrison blocks.

This is lane-local proof code. It deliberately does not feed production rows:
it only classifies or rejects candidate scalar offsets when a live row can be
matched to a save-oracle row by controller tag, occupied tag, state id, and law.
"""

from __future__ import annotations

import argparse
import json
import math
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parents[2]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(REPO_ROOT / "memory_reader"))
sys.path.insert(0, str(REPO_ROOT / "memory_reader" / "live_layers"))
sys.path.insert(0, str(REPO_ROOT / "save_oracle"))

import deep_common as dc  # noqa: E402
import live_probe  # noqa: E402
import remaining_fields_probe as rem  # noqa: E402
import tag_table  # noqa: E402
import winmem  # noqa: E402
from deep_common import LBRACE, atom_value, iter_block_entries, to_float, to_int  # noqa: E402

STATE_GARRISON_READ_SIZE = 0x180
FLOAT_TOLERANCE = 0.0007
MIN_VALIDATE_ROWS = 20
MIN_VALIDATE_RATIO = 0.95
MIN_REJECT_ROWS = 12
MIN_NONDEFAULT_REJECT_ROWS = 2
VALUE_COUNT_CAP = 32

FIELD_DEFAULTS = {
    "strength_ratio": 1.0,
    "garrison_required": 0.0,
    "army_manpower_need": 0.0,
}

PINNED_SCALAR_CANDIDATES = {
    "strength_ratio": (
        ("i32", 0x10),
        ("u32", 0x10),
        ("i32", 0x78),
        ("u32", 0x78),
        ("i32", 0x7C),
        ("u32", 0x7C),
        ("fixed100000_i32", 0xF0),
        ("fixed100000_i64", 0xF0),
    ),
    "garrison_required": (
        ("fixed100000_i32", 0xF8),
        ("fixed100000_i64", 0xF8),
    ),
    "army_manpower_need": (
        ("i32", 0x88),
        ("u32", 0x88),
    ),
}


def hx(value):
    return "0x%X" % value if isinstance(value, int) else None


def parse_manpower_pool(tz):
    values = []
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "value":
                tag = None
                value = None
                for subkey, subopener in iter_block_entries(tz):
                    if subopener is LBRACE:
                        dc._skip_block_body(tz)
                        continue
                    raw = atom_value(subopener)
                    if raw is None:
                        continue
                    if subkey == "tag":
                        tag = raw.strip('"')
                    elif subkey == "value":
                        value = to_float(raw)
                values.append({"tag": tag, "value": value})
            else:
                dc._skip_block_body(tz)
    return values


def parse_equipment_need(tz):
    needs = {}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            dc._skip_block_body(tz)
            continue
        raw = atom_value(opener)
        value = to_float(raw)
        if value is not None:
            needs[key] = value
    return needs


def parse_reinforcement_requests(tz):
    out = []
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key != "request":
                dc._skip_block_body(tz)
                continue
            rec = {"status": None, "progress": None, "date": None}
            for subkey, subopener in iter_block_entries(tz):
                if subopener is LBRACE:
                    dc._skip_block_body(tz)
                    continue
                raw = atom_value(subopener)
                if raw is None:
                    continue
                if subkey == "status":
                    rec["status"] = to_int(raw)
                elif subkey == "progress":
                    rec["progress"] = to_float(raw)
                elif subkey == "date":
                    rec["date"] = raw
            out.append(rec)
    return out


def parse_state_garrison_entry(tz):
    rec = {
        "strength_ratio": None,
        "garrison_required": None,
        "army_manpower_need": None,
        "manpower_pool": [],
        "equipment_need": {},
        "reinforcement_requests": [],
    }
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "manpower_pool":
                rec["manpower_pool"] = parse_manpower_pool(tz)
            elif key == "equipment_need":
                rec["equipment_need"] = parse_equipment_need(tz)
            elif key == "garrison_reinforcement_requests":
                rec["reinforcement_requests"] = parse_reinforcement_requests(tz)
            else:
                dc._skip_block_body(tz)
            continue
        raw = atom_value(opener)
        if raw is None:
            continue
        if key == "strength_ratio":
            rec["strength_ratio"] = to_float(raw)
        elif key == "garrison_required":
            rec["garrison_required"] = to_float(raw)
        elif key == "army_manpower_need":
            rec["army_manpower_need"] = to_int(raw)
    return rec


def parse_occupation_law_list(tz):
    laws = {}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            dc._skip_block_body(tz)
            continue
        raw = atom_value(opener)
        state_id = to_int(key)
        if state_id is not None and raw is not None:
            laws[state_id] = raw.strip('"')
    return laws


def parse_occupied_country(tz, controller_tag, occupied_tag, out):
    laws_by_state = {}
    pending_garrison = []
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "occupation_law_list":
                laws_by_state = parse_occupation_law_list(tz)
            elif key == "state_garrison_data":
                for state_key, state_opener in iter_block_entries(tz):
                    if state_opener is not LBRACE:
                        continue
                    state_id = to_int(state_key)
                    rec = parse_state_garrison_entry(tz)
                    if state_id is not None:
                        pending_garrison.append((state_id, rec))
            else:
                dc._skip_block_body(tz)
    for state_id, rec in pending_garrison:
        rec = dict(rec)
        rec["controller_tag"] = controller_tag
        rec["occupied_tag"] = occupied_tag
        rec["state_id"] = state_id
        rec["law_name"] = laws_by_state.get(state_id)
        out[(controller_tag, occupied_tag, state_id)] = rec


def parse_occupation_status(tz, controller_tag, out):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "occupation":
                for occupied_tag, occupied_opener in iter_block_entries(tz):
                    if occupied_opener is LBRACE:
                        parse_occupied_country(tz, controller_tag, occupied_tag, out)
            else:
                dc._skip_block_body(tz)


def parse_save_garrison_rows(text):
    out = {}

    def parse_countries(tz):
        for tag, opener in iter_block_entries(tz):
            if opener is not LBRACE:
                continue
            for key, op in iter_block_entries(tz):
                if op is LBRACE:
                    if key == "occupation_status":
                        parse_occupation_status(tz, tag, out)
                    else:
                        dc._skip_block_body(tz)

    meta = dc.top_level_pass(
        text,
        {"countries": parse_countries},
        scalar_keys=("date", "version", "player"),
    )
    return out, meta


def live_garrison_rows(p, rows, status_by_tag, tbl):
    target_vtables = {
        p.module_base + rem.COUNTRY_OCCUPATION_DATA_VTABLE_RVA: "country_occupation_data",
        p.module_base + rem.STATE_GARRISON_DATA_VTABLE_RVA: "state_garrison_data",
        p.module_base + live_probe.OCCUPATION_LAW_VTABLE_RVA: "occupation_law",
        p.module_base + live_probe.STATE_VTABLE_RVA: "state",
    }
    state_ptr_to_id = {
        row["state_ptr"]: row["state_id"]
        for row in rows
        if isinstance(row.get("state_ptr"), int)
    }
    out = {}
    for controller_id, country in status_by_tag.items():
        controller_tag = country.get("country_tag")
        status_ptr = country.get("occupation_status_ptr")
        if not controller_tag or not status_ptr:
            continue
        status_container = rem.read_u64(
            p, status_ptr + rem.COUNTRY_OCCUPATION_STATUS_OCCUPATION_DATA_CONTAINER_OFF
        )
        occupation_data_by_offset = rem.collect_container_ptrs_by_kind(
            p, status_container, target_vtables
        ).get("country_occupation_data", {})
        for occupation_data_ptr in sorted(set(occupation_data_by_offset.values())):
            if (
                rem.read_u64(
                    p,
                    occupation_data_ptr + rem.COUNTRY_OCCUPATION_DATA_PARENT_STATUS_OFF,
                )
                != status_ptr
            ):
                continue
            occupied_tag_id = rem.read_u32(
                p, occupation_data_ptr + rem.COUNTRY_OCCUPATION_DATA_OCCUPIED_TAG_OFF
            )
            if not isinstance(occupied_tag_id, int) or occupied_tag_id >= tbl.count:
                continue
            occupied_tag = tbl.tag(occupied_tag_id)
            law_container = rem.read_u64(
                p, occupation_data_ptr + rem.COUNTRY_OCCUPATION_DATA_LAW_LIST_CONTAINER_OFF
            )
            garrison_container = rem.read_u64(
                p, occupation_data_ptr + rem.COUNTRY_OCCUPATION_DATA_GARRISON_LIST_CONTAINER_OFF
            )
            law_by_offset = rem.collect_container_ptrs_by_kind(
                p, law_container, target_vtables
            ).get("occupation_law", {})
            garrison_by_offset = rem.collect_container_ptrs_by_kind(
                p, garrison_container, target_vtables
            ).get("state_garrison_data", {})
            for entry_offset, garrison_ptr in garrison_by_offset.items():
                state_ptr = rem.read_u64(p, garrison_ptr + rem.STATE_GARRISON_DATA_STATE_OFF)
                occupation_parent = rem.read_u64(
                    p, garrison_ptr + rem.STATE_GARRISON_DATA_OCCUPATION_DATA_OFF
                )
                state_id = state_ptr_to_id.get(state_ptr)
                if state_id is None or occupation_parent != occupation_data_ptr:
                    continue
                law_ptr = law_by_offset.get(entry_offset)
                law_name = None
                if law_ptr:
                    law_name = live_probe.read_pdx_string(
                        p, law_ptr + live_probe.OCCUPATION_LAW_NAME_OFF
                    )
                blob = p.try_read(garrison_ptr, STATE_GARRISON_READ_SIZE)
                if not blob or len(blob) < STATE_GARRISON_READ_SIZE:
                    continue
                out[(controller_tag, occupied_tag, state_id)] = {
                    "controller_tag": controller_tag,
                    "controller_tag_db_id": controller_id,
                    "occupied_tag": occupied_tag,
                    "occupied_tag_db_id": occupied_tag_id,
                    "state_id": state_id,
                    "law_name": law_name,
                    "entry_offset": entry_offset,
                    "garrison_ptr": garrison_ptr,
                    "blob": blob,
                }
    return out


def iter_candidate_values(blob):
    limit = len(blob)
    for offset in range(0, limit - 4 + 1, 4):
        chunk4 = blob[offset : offset + 4]
        i32 = struct.unpack("<i", chunk4)[0]
        u32 = struct.unpack("<I", chunk4)[0]
        f32 = struct.unpack("<f", chunk4)[0]
        yield "i32", offset, i32
        yield "u32", offset, u32
        yield "fixed100000_i32", offset, i32 / 100000.0
        if math.isfinite(f32) and abs(f32) < 1e9:
            yield "f32", offset, f32
    for offset in range(0, limit - 8 + 1, 8):
        chunk8 = blob[offset : offset + 8]
        i64 = struct.unpack("<q", chunk8)[0]
        u64 = struct.unpack("<Q", chunk8)[0]
        f64 = struct.unpack("<d", chunk8)[0]
        yield "i64", offset, i64
        yield "u64", offset, u64
        yield "fixed100000_i64", offset, i64 / 100000.0
        if math.isfinite(f64) and abs(f64) < 1e12:
            yield "f64", offset, f64


def matches_value(field, expected, actual):
    if expected is None:
        return False
    if field in ("strength_ratio", "garrison_required"):
        return isinstance(actual, (int, float)) and abs(float(actual) - float(expected)) <= FLOAT_TOLERANCE
    if field == "army_manpower_need":
        return isinstance(actual, (int, float)) and abs(float(actual) - float(expected)) <= 0.01
    return False


def rounded_key(value):
    if isinstance(value, float):
        return f"{value:.6f}"
    return str(value)


def add_count_capped(counts, value, cap=VALUE_COUNT_CAP):
    key = rounded_key(value)
    if key in counts:
        counts[key] += 1
    elif len(counts) < cap:
        counts[key] = 1
    else:
        counts["<more>"] = counts.get("<more>", 0) + 1


def is_nondefault(field, value):
    if value is None:
        return False
    default = FIELD_DEFAULTS.get(field)
    if default is None:
        return True
    tolerance = FLOAT_TOLERANCE if isinstance(default, float) else 0.01
    return abs(float(value) - float(default)) > tolerance


def candidate_rejection(field, item):
    if item["compared"] < MIN_REJECT_ROWS:
        return None
    if (
        item["nondefault_expected_rows"] >= MIN_NONDEFAULT_REJECT_ROWS
        and item["nondefault_expected_misses"] == item["nondefault_expected_rows"]
    ):
        return "rejected_misses_all_nondefault_fixture_rows"
    if item["match_ratio"] < MIN_VALIDATE_RATIO:
        return "rejected_low_match_ratio_current_fixture"
    return None


def classify_candidates(matches):
    fields = ("strength_ratio", "garrison_required", "army_manpower_need")
    stats = {field: {} for field in fields}
    for live, oracle in matches:
        for encoding, offset, actual in iter_candidate_values(live["blob"]):
            key = (encoding, offset)
            for field in fields:
                expected = oracle.get(field)
                if expected is None:
                    continue
                rec = stats[field].setdefault(
                    key,
                    {
                        "encoding": encoding,
                        "offset": offset,
                        "compared": 0,
                        "matches": 0,
                        "samples": [],
                        "miss_samples": [],
                        "actual_value_counts": {},
                        "expected_value_counts": {},
                        "nondefault_expected_rows": 0,
                        "nondefault_expected_misses": 0,
                    },
                )
                rec["compared"] += 1
                add_count_capped(rec["actual_value_counts"], actual)
                add_count_capped(rec["expected_value_counts"], expected)
                ok = matches_value(field, expected, actual)
                nondefault = is_nondefault(field, expected)
                if nondefault:
                    rec["nondefault_expected_rows"] += 1
                    if not ok:
                        rec["nondefault_expected_misses"] += 1
                        if len(rec["miss_samples"]) < 8:
                            rec["miss_samples"].append(
                                {
                                    "state_id": live["state_id"],
                                    "controller_tag": live["controller_tag"],
                                    "occupied_tag": live["occupied_tag"],
                                    "law_name": live["law_name"],
                                    "expected": expected,
                                    "actual": actual,
                                    "garrison_ptr": hx(live["garrison_ptr"]),
                                }
                            )
                if ok:
                    rec["matches"] += 1
                    if len(rec["samples"]) < 8:
                        rec["samples"].append(
                            {
                                "state_id": live["state_id"],
                                "controller_tag": live["controller_tag"],
                                "occupied_tag": live["occupied_tag"],
                                "law_name": live["law_name"],
                                "expected": expected,
                                "actual": actual,
                                "garrison_ptr": hx(live["garrison_ptr"]),
                            }
                        )
    out = {}
    for field, by_key in stats.items():
        ranked = sorted(
            by_key.values(),
            key=lambda rec: (rec["matches"], rec["matches"] / max(1, rec["compared"])),
            reverse=True,
        )
        compact = []
        selected = list(ranked[:12])
        selected_keys = {(rec["encoding"], rec["offset"]) for rec in selected}
        for pinned in PINNED_SCALAR_CANDIDATES.get(field, ()):
            rec = by_key.get(pinned)
            if rec and pinned not in selected_keys:
                selected.append(rec)
                selected_keys.add(pinned)
        for rec in selected:
            ratio = rec["matches"] / max(1, rec["compared"])
            item = {
                "encoding": rec["encoding"],
                "offset": hx(rec["offset"]),
                "compared": rec["compared"],
                "matches": rec["matches"],
                "match_ratio": ratio,
                "actual_value_counts": rec["actual_value_counts"],
                "expected_value_counts": rec["expected_value_counts"],
                "nondefault_expected_rows": rec["nondefault_expected_rows"],
                "nondefault_expected_misses": rec["nondefault_expected_misses"],
                "samples": rec["samples"],
                "miss_samples": rec["miss_samples"],
            }
            rejection = candidate_rejection(field, item)
            if rejection:
                item["candidate_verdict"] = rejection
            compact.append(item)
        best = compact[0] if compact else None
        threshold_candidates = [
            item
            for item in compact
            if item["compared"] >= MIN_VALIDATE_ROWS
            and item["match_ratio"] >= MIN_VALIDATE_RATIO
        ]
        threshold_offsets = sorted({item["offset"] for item in threshold_candidates})
        rejected_top_candidates = [
            {
                "encoding": item["encoding"],
                "offset": item["offset"],
                "matches": item["matches"],
                "compared": item["compared"],
                "match_ratio": item["match_ratio"],
                "candidate_verdict": item["candidate_verdict"],
                "nondefault_expected_rows": item["nondefault_expected_rows"],
                "nondefault_expected_misses": item["nondefault_expected_misses"],
            }
            for item in compact
            if item.get("candidate_verdict")
        ]
        verdict = "fixture_insufficient"
        if len(threshold_offsets) == 1:
            verdict = "validated_candidate"
        elif len(threshold_offsets) > 1:
            verdict = "ambiguous_candidate_fixture_required"
        if compact and rejected_top_candidates and not any(
            item.get("candidate_verdict") is None
            and item["compared"] >= MIN_VALIDATE_ROWS
            and item["match_ratio"] >= MIN_VALIDATE_RATIO
            for item in compact
        ):
            verdict = "inline_candidates_rejected_current_fixture"
        elif best and best["matches"] == 0:
            verdict = "no_matching_candidate"
        out[field] = {
            "verdict": verdict,
            "top_candidates": compact,
            "threshold_offsets": threshold_offsets,
            "threshold_candidate_count": len(threshold_candidates),
            "rejected_top_candidates": rejected_top_candidates,
        }
    return out


def write_md(out_dir, proof):
    lines = [
        "# Garrison Scalar Probe",
        "",
        "- Live SHA-256: `%s`" % proof["process"]["sha256"],
        "- Save fixture: `%s`, date `%s`" % (
            proof["save_fixture"]["path"],
            proof["save_fixture"]["date"],
        ),
        "- Save garrison rows: `%d`" % proof["save_fixture"]["garrison_rows"],
        "- Live garrison rows: `%d`" % proof["live"]["garrison_rows"],
        "- Matched key rows: `%d`" % proof["compare"]["matched_key_rows"],
        "- Matched same-law rows: `%d`" % proof["compare"]["matched_same_law_rows"],
        "- Verdict: `%s`" % proof["verdict"],
        "",
        "## Candidate Fields",
        "",
    ]
    for field, rec in proof["candidate_fields"].items():
        best = rec["top_candidates"][0] if rec["top_candidates"] else None
        if best:
            rejected = rec.get("rejected_top_candidates") or []
            lines.append(
                "- `%s`: `%s`, best `%s @ %s`, `%d/%d` matches, "
                "threshold offsets `%s`, rejected top candidates `%d`"
                % (
                    field,
                    rec["verdict"],
                    best["encoding"],
                    best["offset"],
                    best["matches"],
                    best["compared"],
                    ", ".join(rec.get("threshold_offsets") or []),
                    len(rejected),
                )
            )
            for item in rec["top_candidates"]:
                if item.get("candidate_verdict") and item["offset"] in {
                    "0x10",
                    "0x78",
                    "0x7C",
                    "0xF0",
                    "0xF8",
                    "0x88",
                }:
                    lines.append(
                        "  - rejected `%s @ %s`: `%s`, `%d/%d` matches, "
                        "non-default misses `%d/%d`, actual values `%s`"
                        % (
                            item["encoding"],
                            item["offset"],
                            item["candidate_verdict"],
                            item["matches"],
                            item["compared"],
                            item["nondefault_expected_misses"],
                            item["nondefault_expected_rows"],
                            item["actual_value_counts"],
                        )
                    )
        else:
            lines.append("- `%s`: `%s`, no candidates" % (field, rec["verdict"]))
    lines.extend(
        [
            "",
            "## Contract",
            "",
            proof["field_contract"],
            "",
            proof.get("rejection_contract", ""),
        ]
    )
    (out_dir / "garrison_scalar_PROOF.md").write_text(
        "\n".join(lines) + "\n", encoding="utf-8"
    )


def run(out_dir, save_melt):
    text = Path(save_melt).read_text(encoding="utf-8-sig", errors="replace")
    save_rows, save_meta = parse_save_garrison_rows(text)
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise RuntimeError("hoi4.exe not running")
    with p:
        fp = live_probe.checked_fingerprint(p)
        tbl = tag_table.TagTable.locate(p)
        roots = live_probe.resolve_state_roots(p)
        status_by_tag, status_summary = live_probe.read_country_occupation_status(
            p, roots["game_state"], tbl
        )
        rows, invalid_states = live_probe.read_state_rows(
            p, roots["state_roots"], tbl, status_by_tag
        )
        live_rows = live_garrison_rows(p, rows, status_by_tag, tbl)

    matched = []
    key_only = 0
    law_mismatches = []
    for key, live in live_rows.items():
        oracle = save_rows.get(key)
        if not oracle:
            continue
        key_only += 1
        if live.get("law_name") and oracle.get("law_name") and live["law_name"] != oracle["law_name"]:
            if len(law_mismatches) < 12:
                law_mismatches.append(
                    {
                        "controller_tag": key[0],
                        "occupied_tag": key[1],
                        "state_id": key[2],
                        "live_law": live.get("law_name"),
                        "save_law": oracle.get("law_name"),
                    }
                )
            continue
        matched.append((live, oracle))

    candidate_fields = classify_candidates(matched)
    validated = [
        field
        for field, rec in candidate_fields.items()
        if rec["verdict"] == "validated_candidate"
    ]
    ambiguous = [
        field
        for field, rec in candidate_fields.items()
        if rec["verdict"] == "ambiguous_candidate_fixture_required"
    ]
    if validated:
        verdict = "validated_scalar_candidates"
    elif ambiguous:
        verdict = "ambiguous_scalar_candidates_fixture_required"
    else:
        verdict = "fixture_insufficient"
    proof = {
        "process": {
            "pid": fp.get("pid"),
            "base": hx(fp.get("base")),
            "sha256": fp.get("sha256"),
        },
        "save_fixture": {
            "path": str(save_melt),
            "date": save_meta.get("date"),
            "version": save_meta.get("version"),
            "player": save_meta.get("player"),
            "garrison_rows": len(save_rows),
        },
        "live": {
            "state_roots": len(roots["state_roots"]),
            "invalid_state_rows": invalid_states,
            "occupation_status": status_summary,
            "garrison_rows": len(live_rows),
        },
        "compare": {
            "matched_key_rows": key_only,
            "matched_same_law_rows": len(matched),
            "law_mismatches_sample": law_mismatches,
        },
        "candidate_fields": candidate_fields,
        "verdict": verdict,
        "field_contract": (
            "Only fields with verdict validated_candidate are safe to wire. "
            "ambiguous_candidate_fixture_required means more than one live offset "
            "passes the threshold, so a controlled fixture with a non-default "
            "value is required before wiring. fixture_insufficient means the "
            "current live process and save fixture do not provide enough "
            "same-key/same-law rows to classify the scalar; keep garrison "
            "need/equilibrium/progress row-badged unavailable until a same-hour "
            "save or controlled UI flip is captured."
        ),
        "rejection_contract": (
            "Candidate offsets are rejected for this fixture when they have at "
            f"least {MIN_REJECT_ROWS} compared same-key/same-law rows and either "
            "miss every non-default save row or remain below the validation "
            f"ratio {MIN_VALIDATE_RATIO:.2f}. Rejection is scoped to the scanned "
            f"CStateGarrisonData inline range 0x0..0x{STATE_GARRISON_READ_SIZE - 1:X} "
            "and the listed primitive encodings."
        ),
    }

    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "garrison_scalar_probe.json").write_text(
        json.dumps(proof, indent=2), encoding="utf-8"
    )
    write_md(out_dir, proof)
    return proof


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out-dir", required=True)
    parser.add_argument(
        "--save-melt",
        default=str(Path.home() / "AppData" / "Local" / "Temp" / "ger1946_melt.txt"),
    )
    args = parser.parse_args(argv)
    proof = run(Path(args.out_dir), Path(args.save_melt))
    print(json.dumps({"out_dir": args.out_dir, "proof": proof}, indent=2))


if __name__ == "__main__":
    raise SystemExit(main())
