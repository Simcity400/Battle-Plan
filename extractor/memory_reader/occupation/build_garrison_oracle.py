#!/usr/bin/env python3
"""Build a save oracle for occupation garrison residual fields.

Input is a melted the game save. The output is intentionally a development oracle:
it records the save-visible garrison template, per-state occupation law, and
state_garrison_data fields so live probes can compare current memory candidates
without promoting offsets from static save evidence alone.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parents[2]
SAVE_ORACLE = REPO_ROOT / "extractor" / "save_oracle"
if str(SAVE_ORACLE) not in sys.path:
    sys.path.insert(0, str(SAVE_ORACLE))

import deep_common as dc
from deep_common import (
    EQUALS,
    LBRACE,
    RBRACE,
    atom_value,
    iter_block_entries,
    read_id_block,
    read_scalar_list,
    to_float,
    to_int,
    top_level_pass,
)


SCHEMA_VERSION = 2


class OracleParseError(RuntimeError):
    pass


_EOF_TRUNCATED = object()


def iter_block_entries_checked(tz):
    """Like iter_block_entries, but yields a final ``(_EOF_TRUNCATED, None)`` if
    the block ended because the stream hit EOF rather than a matching '}'.

    Mirrors deep_common.iter_block_entries token handling exactly so a truncated
    ``state_garrison_data`` block is detectable and can fail closed.
    """
    while True:
        tok = tz.next()
        if tok is None:
            yield _EOF_TRUNCATED, None
            return
        if tok is RBRACE:
            return
        if tok is LBRACE:
            dc._skip_block_body(tz)
            continue
        key = atom_value(tok)
        if key is None:
            continue
        nxt = tz.peek()
        if nxt is EQUALS:
            tz.next()
            vpeek = tz.peek()
            if vpeek is RBRACE:
                tz.next()
                return
            val = tz.next()
            if val is None:
                yield _EOF_TRUNCATED, None
                return
            if val is LBRACE:
                yield key, LBRACE
            else:
                yield key, val
        else:
            yield key, None


def parse_manpower_pool(tz):
    rows = []
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key != "value":
            dc._skip_block_body(tz)
            continue
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
        rows.append({"tag": tag, "value": value})
    return rows


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


def parse_equipment_rows(tz, issues, context):
    """Parse a save equipment collection without losing repeated rows or order.

    The save shape is ``equipment={ equipment={ id={id type} amount } ...
    allow_zero_entries=no }``.  A measured-empty collection is represented by
    an empty row list plus the explicit ``allow_zero_entries`` value; it is not
    confused with an absent collection.
    """
    rows = []
    allow_zero_entries = None
    truncated = False
    malformed = 0
    for key, opener in iter_block_entries_checked(tz):
        if key is _EOF_TRUNCATED:
            truncated = True
            malformed += 1
            issues.append("%s: equipment block truncated" % context)
            break
        if opener is LBRACE:
            if key != "equipment":
                dc._skip_block_body(tz)
                continue
            equipment_id = None
            equipment_type = None
            amount = None
            for subkey, subopener in iter_block_entries(tz):
                if subopener is LBRACE:
                    if subkey == "id":
                        equipment_id, equipment_type = read_id_block(tz)
                    else:
                        dc._skip_block_body(tz)
                    continue
                raw = atom_value(subopener)
                if subkey == "amount" and raw is not None:
                    amount = to_float(raw)
            if equipment_id is None or equipment_type is None or amount is None:
                malformed += 1
                issues.append("%s: malformed equipment row %d" % (context, len(rows)))
                continue
            rows.append({
                "source_ordinal": len(rows),
                "equipment_id": equipment_id,
                "equipment_type": equipment_type,
                "amount": amount,
            })
            continue
        raw = atom_value(opener)
        if key == "allow_zero_entries" and raw is not None:
            if raw == "yes":
                allow_zero_entries = True
            elif raw == "no":
                allow_zero_entries = False
            else:
                malformed += 1
                issues.append("%s: invalid allow_zero_entries %r" % (context, raw))
    return rows, {
        "present": True,
        "allow_zero_entries": allow_zero_entries,
        "rows_complete": not truncated and malformed == 0,
        "truncated": truncated,
        "malformed_rows": malformed,
    }


def parse_reinforcement_requests(tz):
    requests = []
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key != "request":
            dc._skip_block_body(tz)
            continue
        rec = {
            "source_ordinal": len(requests),
            "status": None,
            "progress": None,
            "total_progress": None,
            "need": {},
            "date": None,
        }
        for subkey, subopener in iter_block_entries(tz):
            if subopener is LBRACE:
                if subkey == "need":
                    rec["need"] = parse_equipment_need(tz)
                else:
                    dc._skip_block_body(tz)
                continue
            raw = atom_value(subopener)
            if raw is None:
                continue
            if subkey == "status":
                rec["status"] = to_int(raw)
            elif subkey == "progress":
                rec["progress"] = to_float(raw)
            elif subkey == "total_progress":
                rec["total_progress"] = to_float(raw)
            elif subkey == "date":
                rec["date"] = raw
        requests.append(rec)
    return requests


def parse_state_garrison_entry(tz, issues, context):
    rec = {
        "strength_ratio": None,
        "garrison_required": None,
        "army_manpower_need": None,
        "manpower_pool": [],
        "equipment": None,
        "equipment_meta": {"present": False, "rows_complete": False},
        "equipment_need": {},
        "reinforcement_requests": [],
    }
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "manpower_pool":
                rec["manpower_pool"] = parse_manpower_pool(tz)
            elif key == "equipment":
                rec["equipment"], rec["equipment_meta"] = parse_equipment_rows(
                    tz, issues, context + ".equipment"
                )
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


def parse_law_list(tz):
    laws = {}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            dc._skip_block_body(tz)
            continue
        state_id = to_int(key)
        raw = atom_value(opener)
        if state_id is not None and raw is not None:
            laws[state_id] = raw.strip('"')
    return laws


def parse_state_garrison_data(tz, issues, context):
    rows = {}
    entry_count = 0
    malformed_count = 0
    truncated = False
    for key, opener in iter_block_entries_checked(tz):
        if key is _EOF_TRUNCATED:
            truncated = True
            malformed_count += 1
            issues.append(
                "%s: state_garrison_data block truncated (EOF before closing brace)" % context
            )
            break
        entry_count += 1
        if opener is not LBRACE:
            malformed_count += 1
            issues.append(
                "%s: state_garrison_data entry %r is not a block" % (context, key)
            )
            continue
        state_id = to_int(key)
        rec = parse_state_garrison_entry(tz, issues, "%s state %r" % (context, key))
        if state_id is None:
            malformed_count += 1
            issues.append("%s: state_garrison_data key %r is not a state id" % (context, key))
            continue
        rows[state_id] = rec
    return rows, {
        "entry_count": entry_count,
        "malformed_count": malformed_count,
        "truncated": truncated,
    }


def parse_occupied_country(tz, controller_tag, occupied_tag, issues):
    rec = {
        "occupied_tag": occupied_tag,
        "states": [],
        "resistance": None,
        "compliance": None,
        "compliance_modifiers": [],
        "occupation_law_by_state": {},
        "state_garrison_data": {},
        "state_garrison_data_block_count": 0,
        "state_garrison_data_entry_count": 0,
        "state_garrison_data_malformed_entries": 0,
        "state_garrison_data_truncated": False,
    }
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "states":
                rec["states"] = [
                    value for value in (to_int(item) for item in read_scalar_list(tz))
                    if value is not None
                ]
            elif key == "compliance_modifiers":
                rec["compliance_modifiers"] = read_scalar_list(tz)
            elif key == "occupation_law_list":
                rec["occupation_law_by_state"] = parse_law_list(tz)
            elif key == "state_garrison_data":
                rec["state_garrison_data_block_count"] += 1
                context = "%s occupies %s" % (controller_tag, occupied_tag)
                rows, stats = parse_state_garrison_data(tz, issues, context)
                rec["state_garrison_data"].update(rows)
                rec["state_garrison_data_entry_count"] += stats["entry_count"]
                rec["state_garrison_data_malformed_entries"] += stats["malformed_count"]
                if stats["truncated"]:
                    rec["state_garrison_data_truncated"] = True
            else:
                dc._skip_block_body(tz)
            continue
        raw = atom_value(opener)
        if raw is None:
            continue
        if key == "resistance":
            rec["resistance"] = to_float(raw)
        elif key == "compliance":
            rec["compliance"] = to_float(raw)

    if (
        rec["state_garrison_data_block_count"]
        and rec["states"]
        and not rec["state_garrison_data"]
    ):
        issues.append(
            "%s occupies %s: state_garrison_data block parsed zero rows for %d occupied states"
            % (controller_tag, occupied_tag, len(rec["states"]))
        )

    for state_id, row in rec["state_garrison_data"].items():
        row["controller_tag"] = controller_tag
        row["occupied_tag"] = occupied_tag
        row["state_id"] = state_id
        row["law_name"] = rec["occupation_law_by_state"].get(state_id)
    return rec


def parse_resistance_attack_log(tz, controller_tag, source_ordinal, issues):
    context = "%s resistance_attack_log[%d]" % (controller_tag, source_ordinal)
    rec = {
        "stable_event_id": None,
        "source_ordinal": source_ordinal,
        "controller_tag": controller_tag,
        "state_id": None,
        "occupied_country_tag": None,
        "event_hour_or_engine_date": None,
        "garrison": None,
        "attack_blocked": None,
        "resistance_activity": None,
        "manpower_loss_rows": [],
        "lost_manpower": None,
        "equipment_loss_rows": None,
        "equipment_loss_meta": {"present": False, "rows_complete": False},
    }
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "manpower":
                rec["manpower_loss_rows"] = parse_manpower_pool(tz)
            elif key == "equipment":
                rec["equipment_loss_rows"], rec["equipment_loss_meta"] = parse_equipment_rows(
                    tz, issues, context + ".equipment"
                )
            else:
                dc._skip_block_body(tz)
            continue
        raw = atom_value(opener)
        if raw is None:
            continue
        if key == "state":
            rec["state_id"] = to_int(raw)
        elif key == "country":
            rec["occupied_country_tag"] = raw.strip('"')
        elif key == "date":
            rec["event_hour_or_engine_date"] = raw
        elif key == "garrison":
            if raw == "yes":
                rec["garrison"] = True
                rec["attack_blocked"] = True
            elif raw == "no":
                rec["garrison"] = False
                rec["attack_blocked"] = False
            else:
                issues.append("%s: invalid garrison flag %r" % (context, raw))
        elif key == "resistance_activity":
            rec["resistance_activity"] = raw.strip('"')
    manpower = [row.get("value") for row in rec["manpower_loss_rows"]]
    if all(isinstance(value, (int, float)) for value in manpower):
        total = sum(manpower)
        rec["lost_manpower"] = int(total) if float(total).is_integer() else total
    rec["stable_event_id"] = "%s|%s|%s|%d" % (
        controller_tag,
        rec["event_hour_or_engine_date"],
        rec["state_id"],
        source_ordinal,
    )
    required = [
        rec["state_id"], rec["occupied_country_tag"],
        rec["event_hour_or_engine_date"], rec["attack_blocked"],
        rec["lost_manpower"], rec["equipment_loss_rows"],
    ]
    rec["row_complete"] = all(value is not None for value in required) and rec[
        "equipment_loss_meta"
    ].get("rows_complete") is True
    if not rec["row_complete"]:
        issues.append("%s: incomplete resistance attack row" % context)
    return rec


def parse_occupation_status(tz, controller_tag, issues):
    rec = {
        "default_law": None,
        "priority": None,
        "garrison_template_id": None,
        "garrison_template_type": None,
        "occupied": [],
        "resistance_attack_log": [],
        "resistance_attack_log_complete": True,
    }
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "division_template_id":
                template_id, template_type = read_id_block(tz)
                rec["garrison_template_id"] = template_id
                rec["garrison_template_type"] = template_type
            elif key == "occupation":
                for occupied_tag, occupied_opener in iter_block_entries(tz):
                    if occupied_opener is LBRACE:
                        rec["occupied"].append(
                            parse_occupied_country(tz, controller_tag, occupied_tag, issues)
                        )
            elif key == "resistance_attack_log":
                rec["resistance_attack_log"].append(
                    parse_resistance_attack_log(
                        tz, controller_tag, len(rec["resistance_attack_log"]), issues
                    )
                )
            else:
                dc._skip_block_body(tz)
            continue
        raw = atom_value(opener)
        if raw is None:
            continue
        if key == "default_law":
            rec["default_law"] = raw.strip('"')
        elif key == "priority":
            rec["priority"] = to_int(raw)
    return rec


def parse_collaboration_block(tz):
    scalars = {}
    nested_keys = {}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            nested_keys[key] = nested_keys.get(key, 0) + 1
            dc._skip_block_body(tz)
            continue
        raw = atom_value(opener)
        if raw is not None:
            scalars[key] = raw
    return {"scalar_keys": scalars, "nested_key_counts": nested_keys}


def parse_countries(tz, by_tag, collaboration_by_tag, issues):
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        occupation_status = None
        collaboration = None
        for key, subopener in iter_block_entries(tz):
            if subopener is not LBRACE:
                continue
            if key == "occupation_status":
                occupation_status = parse_occupation_status(tz, tag, issues)
            elif key == "collaboration":
                collaboration = parse_collaboration_block(tz)
            else:
                dc._skip_block_body(tz)
        if occupation_status and (occupation_status["occupied"] or occupation_status["default_law"]):
            by_tag[tag] = occupation_status
        if collaboration and (collaboration["scalar_keys"] or collaboration["nested_key_counts"]):
            collaboration_by_tag[tag] = collaboration


def flatten_rows(countries):
    rows = []
    for controller_tag, rec in sorted(countries.items()):
        for occupied in rec["occupied"]:
            for state_id, row in sorted(occupied["state_garrison_data"].items()):
                copy = dict(row)
                copy["controller_default_law"] = rec["default_law"]
                copy["controller_priority"] = rec["priority"]
                copy["controller_garrison_template_id"] = rec["garrison_template_id"]
                copy["controller_garrison_template_type"] = rec["garrison_template_type"]
                rows.append(copy)
    return rows


def row_key(row):
    return "%s|%s|%s" % (
        row.get("controller_tag"),
        row.get("occupied_tag"),
        row.get("state_id"),
    )


def build_oracle(text, source_name):
    countries = {}
    collaboration_by_tag = {}
    parse_issues = []
    meta = top_level_pass(
        text,
        {
            "countries": lambda tz: parse_countries(
                tz, countries, collaboration_by_tag, parse_issues
            )
        },
        scalar_keys=("date", "version", "player"),
    )
    rows = flatten_rows(countries)
    history_rows = [
        row
        for _tag, country in sorted(countries.items())
        for row in country.get("resistance_attack_log", [])
    ]
    rows_by_key = {row_key(row): row for row in rows}
    equipment_type_counts = {}
    reinforcement_status_counts = {}
    for row in rows:
        for equipment_type in row.get("equipment_need") or {}:
            equipment_type_counts[equipment_type] = equipment_type_counts.get(equipment_type, 0) + 1
        for request in row.get("reinforcement_requests") or []:
            status = request.get("status")
            key = "none" if status is None else str(status)
            reinforcement_status_counts[key] = reinforcement_status_counts.get(key, 0) + 1

    active_occupiers = sum(1 for rec in countries.values() if rec["occupied"])
    occupation_pairs = sum(len(rec["occupied"]) for rec in countries.values())
    occupied_state_count = sum(
        len(occupied["states"])
        for rec in countries.values()
        for occupied in rec["occupied"]
    )
    garrison_block_count = sum(
        occupied["state_garrison_data_block_count"]
        for rec in countries.values()
        for occupied in rec["occupied"]
    )
    garrison_entry_count = sum(
        occupied["state_garrison_data_entry_count"]
        for rec in countries.values()
        for occupied in rec["occupied"]
    )
    malformed_garrison_entries = sum(
        occupied["state_garrison_data_malformed_entries"]
        for rec in countries.values()
        for occupied in rec["occupied"]
    )
    per_state_laws = sum(
        len(occupied["occupation_law_by_state"])
        for rec in countries.values()
        for occupied in rec["occupied"]
    )
    if parse_issues:
        raise OracleParseError(
            "malformed garrison oracle input: %s" % "; ".join(parse_issues[:6])
        )
    if occupied_state_count and not rows:
        if garrison_block_count:
            reason = (
                "parse failure: %d occupied states and %d state_garrison_data "
                "block(s) produced zero rows"
            ) % (occupied_state_count, garrison_block_count)
        else:
            reason = (
                "parse failure: %d occupied states are present but no "
                "state_garrison_data blocks were found"
            ) % occupied_state_count
        raise OracleParseError(reason)

    out = {
        "schema": "occupation_garrison_oracle",
        "schema_version": SCHEMA_VERSION,
        "source": {
            "filename": source_name,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
        },
        "counts": {
            "countries_with_occupation_data": len(countries),
            "active_occupiers": active_occupiers,
            "occupier_occupied_pairs": occupation_pairs,
            "occupied_state_count": occupied_state_count,
            "per_state_law_assignments": per_state_laws,
            "state_garrison_data_blocks": garrison_block_count,
            "state_garrison_data_entries_seen": garrison_entry_count,
            "state_garrison_data_malformed_entries": malformed_garrison_entries,
            "state_garrison_rows": len(rows),
            "rows_with_strength_ratio": sum(
                1 for row in rows if row.get("strength_ratio") is not None
            ),
            "rows_with_garrison_required": sum(
                1 for row in rows if row.get("garrison_required") is not None
            ),
            "rows_with_army_manpower_need": sum(
                1 for row in rows if row.get("army_manpower_need") is not None
            ),
            "rows_with_manpower_pool": sum(1 for row in rows if row.get("manpower_pool")),
            "rows_with_equipment_need": sum(1 for row in rows if row.get("equipment_need")),
            "rows_with_equipment_collection": sum(
                1 for row in rows if row.get("equipment") is not None
            ),
            "equipment_rows": sum(len(row.get("equipment") or []) for row in rows),
            "rows_with_reinforcement_requests": sum(
                1 for row in rows if row.get("reinforcement_requests")
            ),
            "countries_with_nonempty_collaboration": len(collaboration_by_tag),
            "resistance_attack_history_rows": len(history_rows),
            "resistance_attack_equipment_loss_rows": sum(
                len(row.get("equipment_loss_rows") or []) for row in history_rows
            ),
        },
        "equipment_type_counts": dict(sorted(equipment_type_counts.items())),
        "reinforcement_status_counts": dict(sorted(reinforcement_status_counts.items())),
        "countries": countries,
        "collaboration_by_tag": collaboration_by_tag,
        "garrison_rows": rows,
        "rows_by_key": rows_by_key,
        "resistance_attack_history": history_rows,
        "resistance_attack_history_complete": all(
            row.get("row_complete") is True for row in history_rows
        ),
        "field_contract": {
            "save_visible_only": (
                "These values are persisted save-oracle facts. They can reject or "
                "classify live candidates, but cannot by themselves promote a live "
                "offset without same-build live evidence."
            ),
            "join_key": "controller_tag + occupied_tag + state_id, with law_name used as an extra guard.",
            "candidate_fields": [
                "strength_ratio",
                "garrison_required",
                "army_manpower_need",
                "manpower_pool",
                "equipment",
                "equipment_need",
                "reinforcement_requests",
                "resistance_attack_history",
            ],
            "attack_blocked_mapping": (
                "save resistance_attack_log.garrison yes/no maps directly to the "
                "GARRISON_LOG_ACTIVITY_STOPPED versus GARRISON_LOG_ACTIVITY_SUCCES presentation; "
                "the raw garrison value is retained alongside attack_blocked"
            ),
        },
    }
    return out


def write_summary(path, oracle):
    counts = oracle["counts"]
    lines = [
        "# Garrison Save Oracle Summary",
        "",
        "- Source: `%s`" % oracle["source"]["filename"],
        "- Save date: `%s`" % oracle["source"].get("save_date"),
        "- Player: `%s`" % oracle["source"].get("player"),
        "- Countries with occupation data: `%s`" % counts["countries_with_occupation_data"],
        "- Active occupiers: `%s`" % counts["active_occupiers"],
        "- Occupier/occupied pairs: `%s`" % counts["occupier_occupied_pairs"],
        "- Per-state law assignments: `%s`" % counts["per_state_law_assignments"],
        "- State garrison rows: `%s`" % counts["state_garrison_rows"],
        "- Rows with manpower pool: `%s`" % counts["rows_with_manpower_pool"],
        "- Rows with equipment need: `%s`" % counts["rows_with_equipment_need"],
        "- Rows with reinforcement requests: `%s`"
        % counts["rows_with_reinforcement_requests"],
        "",
        "## Equipment Need Tokens",
        "",
    ]
    for key, value in oracle["equipment_type_counts"].items():
        lines.append("- `%s`: `%s`" % (key, value))
    lines.extend(["", "## Reinforcement Request Statuses", ""])
    for key, value in oracle["reinforcement_status_counts"].items():
        lines.append("- `%s`: `%s`" % (key, value))
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def remove_outputs(out_path, summary_path):
    for path in (out_path, summary_path):
        if not path:
            continue
        try:
            path.unlink()
        except FileNotFoundError:
            pass


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--save-melt", required=True, help="Melted plaintext save path.")
    parser.add_argument("--out", required=True, help="Output oracle JSON path.")
    parser.add_argument("--summary", default=None, help="Optional markdown summary path.")
    parser.add_argument(
        "--expect-date",
        default=None,
        help="Optional exact save date gate, for example 1943.5.1.1.",
    )
    args = parser.parse_args(argv)

    out_path = Path(args.out)
    summary_path = Path(args.summary) if args.summary else None
    save_path = Path(args.save_melt)
    text = save_path.read_text(encoding="utf-8-sig", errors="replace")
    try:
        oracle = build_oracle(text, save_path.name)
    except OracleParseError as exc:
        remove_outputs(out_path, summary_path)
        raise SystemExit(str(exc))
    if args.expect_date and oracle["source"].get("save_date") != args.expect_date:
        remove_outputs(out_path, summary_path)
        raise SystemExit(
            "save date gate failed: %s != %s"
            % (oracle["source"].get("save_date"), args.expect_date)
        )

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(oracle, indent=2, sort_keys=True), encoding="utf-8")
    if summary_path:
        summary_path.parent.mkdir(parents=True, exist_ok=True)
        write_summary(summary_path, oracle)

    print(
        json.dumps(
            {
                "out": str(out_path),
                "summary": args.summary,
                "save_date": oracle["source"].get("save_date"),
                "state_garrison_rows": oracle["counts"]["state_garrison_rows"],
                "active_occupiers": oracle["counts"]["active_occupiers"],
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
