#!/usr/bin/env python3
"""the game save -> OCCUPATION / RESISTANCE GOVERNANCE oracle (``occupation.json``).

Deep-layer extension of the save oracle. This is the OCCUPIER'S-SIDE view of
occupied territory, which lives per-country under
``countries.<TAG>.occupation_status`` — a DIFFERENT section from the
per-state ``states.<id>.resistance`` already covered by state_detail.json.

Per occupier TAG we extract:

  - ``default_law``          : the country's default occupation law
                               (occupation_status.default_law).
  - ``priority``             : occupation_status.priority.
  - ``garrison_template_id`` : occupation_status.division_template_id (id,type),
                               the template the engine auto-deploys as garrison.
  - ``occupied``             : list, one entry per OCCUPIED country tag the
                               occupier currently administers, each:
        - ``occupied_tag``        : the conquered nation.
        - ``states``              : the state ids held from that nation.
        - ``resistance``          : current resistance level (occupier-side).
        - ``compliance``          : current compliance level.
        - ``compliance_modifiers``: active compliance modifier ids.
        - ``occupation_law_by_state`` : {state_id: occupation_law_id} — the law
                                    actually assigned per held state (e.g.
                                    "military_governor_occupation"). This is the
                                    real per-territory law assignment; the
                                    country default_law is only the fallback.

WHY this is a rehost backstop (DESIGN 11.3): the occupation graph (who controls
whose land and under what occupation law / resistance regime) is a slow-moving,
save-persisted governance layer. After a host crash + rehost it is not cheaply
reconstructable from the live daily layer, which carries raw province/state
control but not the occupier-side resistance/compliance/law state. state_detail
only has the STATE's own resistance scalar; it does NOT have the occupier-keyed
per-state law assignment or the occupier->occupied grouping.

NOTE on collaboration governments: ``countries.<TAG>.collaboration`` is EMPTY
({}) for every country in this save, so there is nothing to extract; the key is
documented in meta.notes and will populate (and can be added here) only when a
collaboration government exists.

stdlib only. Reuses the parse_save tokenizer via deep_common.

Verified against a real 1.19 melt (paused 1946.5.18, DDR world) and refuted with
an adversarial Codex pass.
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, atom_value, iter_block_entries, _skip_block_body,
    read_id_block, to_int, to_float, read_scalar_list, run_cli, top_level_pass,
)


def _parse_manpower_rows(tz):
    rows = []
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key != "value":
            _skip_block_body(tz)
            continue
        tag = None
        value = None
        for subkey, subopener in iter_block_entries(tz):
            if subopener is LBRACE:
                _skip_block_body(tz)
                continue
            raw = atom_value(subopener)
            if raw is None:
                continue
            if subkey == "tag":
                tag = raw.strip('"')
            elif subkey == "value":
                value = to_float(raw)
        rows.append({"source_ordinal": len(rows), "tag": tag, "value": value})
    return rows


def _parse_equipment_map(tz):
    rows = []
    allow_zero_entries = None
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key != "equipment":
                _skip_block_body(tz)
                continue
            equipment_id = equipment_type = amount = None
            for subkey, subopener in iter_block_entries(tz):
                if subopener is LBRACE:
                    if subkey == "id":
                        equipment_id, equipment_type = read_id_block(tz)
                    else:
                        _skip_block_body(tz)
                    continue
                raw = atom_value(subopener)
                if subkey == "amount" and raw is not None:
                    amount = to_float(raw)
            rows.append({
                "source_ordinal": len(rows),
                "equipment_id": equipment_id,
                "equipment_type": equipment_type,
                "amount": amount,
            })
            continue
        raw = atom_value(opener)
        if key == "allow_zero_entries" and raw in ("yes", "no"):
            allow_zero_entries = raw == "yes"
    complete = all(
        row["equipment_id"] is not None
        and row["equipment_type"] is not None
        and row["amount"] is not None
        for row in rows
    )
    return rows, {
        "present": True,
        "rows_complete": complete,
        "allow_zero_entries": allow_zero_entries,
    }


def _parse_numeric_map(tz):
    out = {}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        raw = atom_value(opener)
        value = to_float(raw) if raw is not None else None
        if value is not None:
            out[key] = value
    return out


def _parse_reinforcement_requests(tz):
    rows = []
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key != "request":
            _skip_block_body(tz)
            continue
        row = {"source_ordinal": len(rows), "total_progress": None, "need": {}, "date": None}
        for subkey, subopener in iter_block_entries(tz):
            if subopener is LBRACE:
                if subkey == "need":
                    row["need"] = _parse_numeric_map(tz)
                else:
                    _skip_block_body(tz)
                continue
            raw = atom_value(subopener)
            if raw is None:
                continue
            if subkey == "total_progress":
                row["total_progress"] = to_float(raw)
            elif subkey == "date":
                row["date"] = raw
        rows.append(row)
    return rows


def _parse_state_garrison_entry(tz):
    row = {
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
                row["manpower_pool"] = _parse_manpower_rows(tz)
            elif key == "equipment":
                row["equipment"], row["equipment_meta"] = _parse_equipment_map(tz)
            elif key == "equipment_need":
                row["equipment_need"] = _parse_numeric_map(tz)
            elif key == "garrison_reinforcement_requests":
                row["reinforcement_requests"] = _parse_reinforcement_requests(tz)
            else:
                _skip_block_body(tz)
            continue
        raw = atom_value(opener)
        if raw is None:
            continue
        if key == "strength_ratio":
            row["strength_ratio"] = to_float(raw)
        elif key == "garrison_required":
            row["garrison_required"] = to_float(raw)
        elif key == "army_manpower_need":
            row["army_manpower_need"] = to_int(raw)
    return row


def _parse_state_garrison_data(tz):
    rows = {}
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        state_id = to_int(key)
        row = _parse_state_garrison_entry(tz)
        if state_id is not None:
            rows[state_id] = row
    return rows


def _parse_attack_log(tz, controller_tag, source_ordinal):
    row = {
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
                row["manpower_loss_rows"] = _parse_manpower_rows(tz)
            elif key == "equipment":
                row["equipment_loss_rows"], row["equipment_loss_meta"] = _parse_equipment_map(tz)
            else:
                _skip_block_body(tz)
            continue
        raw = atom_value(opener)
        if raw is None:
            continue
        if key == "state":
            row["state_id"] = to_int(raw)
        elif key == "country":
            row["occupied_country_tag"] = raw.strip('"')
        elif key == "date":
            row["event_hour_or_engine_date"] = raw
        elif key == "garrison" and raw in ("yes", "no"):
            row["garrison"] = raw == "yes"
            row["attack_blocked"] = raw == "yes"
        elif key == "resistance_activity":
            row["resistance_activity"] = raw.strip('"')
    values = [entry.get("value") for entry in row["manpower_loss_rows"]]
    if all(isinstance(value, (int, float)) for value in values):
        total = sum(values)
        row["lost_manpower"] = int(total) if float(total).is_integer() else total
    row["stable_event_id"] = "%s|%s|%s|%d" % (
        controller_tag, row["event_hour_or_engine_date"], row["state_id"], source_ordinal
    )
    row["row_complete"] = all(value is not None for value in (
        row["state_id"], row["occupied_country_tag"], row["event_hour_or_engine_date"],
        row["attack_blocked"], row["lost_manpower"], row["equipment_loss_rows"],
    )) and row["equipment_loss_meta"].get("rows_complete") is True
    return row


def _parse_occupied_entry(tz):
    """One occupied-country block:
       <OCC>={ states={..} compliance_modifiers={..} resistance compliance
               occupation_law_list={ <state>=<law> } state_garrison_data={..} }.
    """
    rec = {
        "states": [], "resistance": None, "compliance": None,
        "compliance_modifiers": [], "occupation_law_by_state": {},
        "state_garrison_data": {},
    }
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "states":
                rec["states"] = [to_int(x) for x in read_scalar_list(tz)
                                 if to_int(x) is not None]
            elif key == "compliance_modifiers":
                rec["compliance_modifiers"] = read_scalar_list(tz)
            elif key == "occupation_law_list":
                for sk, so in iter_block_entries(tz):
                    if so is LBRACE:
                        _skip_block_body(tz)
                        continue
                    v = atom_value(so)
                    sid = to_int(sk)
                    if sid is not None and v is not None:
                        rec["occupation_law_by_state"][sid] = v
            elif key == "state_garrison_data":
                rec["state_garrison_data"] = _parse_state_garrison_data(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "resistance":
            rec["resistance"] = to_float(v)
        elif key == "compliance":
            rec["compliance"] = to_float(v)
    return rec


def _parse_occupation_block(tz, occupied):
    """occupation={ <OCC>={...} <OCC>={...} }."""
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        entry = _parse_occupied_entry(tz)
        entry["occupied_tag"] = tag
        occupied.append(entry)


def _parse_occupation_status(tz, rec, controller_tag):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "division_template_id":
                rec["garrison_template_id"] = read_id_block(tz)
            elif key == "occupation":
                _parse_occupation_block(tz, rec["occupied"])
            elif key == "resistance_attack_log":
                rec["resistance_attack_log"].append(
                    _parse_attack_log(tz, controller_tag, len(rec["resistance_attack_log"]))
                )
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "priority":
            rec["priority"] = to_int(v)
        elif key == "default_law":
            rec["default_law"] = v


def _parse_collaboration(tz, rec):
    """collaboration is empty in this save; capture any scalar that appears."""
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is not None:
            rec.setdefault("collaboration", {})[key] = v


def _parse_countries(tz, by_tag):
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        rec = {
            "default_law": None, "priority": None,
            "garrison_template_id": None, "occupied": [], "resistance_attack_log": [],
        }
        for key, op in iter_block_entries(tz):
            if op is LBRACE:
                if key == "occupation_status":
                    _parse_occupation_status(tz, rec, tag)
                elif key == "collaboration":
                    _parse_collaboration(tz, rec)
                else:
                    _skip_block_body(tz)
        # Keep a country if it administers ANY occupied territory, OR if it has a
        # non-default occupation law set (so the backstop preserves law choices).
        if rec["occupied"] or rec["default_law"] or rec["resistance_attack_log"]:
            by_tag[tag] = rec


def build(text, source, was_melted):
    by_tag: dict = {}
    meta = top_level_pass(
        text,
        {"countries": lambda tz: _parse_countries(tz, by_tag)},
        scalar_keys=("date", "version", "player"),
    )

    countries = {}
    n_occupiers = 0
    n_pairs = 0
    n_state_laws = 0
    n_garrison_rows = 0
    n_equipment_rows = 0
    n_history_rows = 0
    n_history_equipment_rows = 0
    for tag, rec in sorted(by_tag.items()):
        occ = rec["occupied"]
        if occ:
            n_occupiers += 1
            n_pairs += len(occ)
            n_state_laws += sum(len(e["occupation_law_by_state"]) for e in occ)
            n_garrison_rows += sum(len(e["state_garrison_data"]) for e in occ)
            n_equipment_rows += sum(
                len(row.get("equipment") or [])
                for e in occ for row in e["state_garrison_data"].values()
            )
        n_history_rows += len(rec["resistance_attack_log"])
        n_history_equipment_rows += sum(
            len(row.get("equipment_loss_rows") or []) for row in rec["resistance_attack_log"]
        )
        countries[tag] = {
            "default_law": rec["default_law"],
            "priority": rec["priority"],
            "garrison_template_id": rec["garrison_template_id"],
            "occupied_count": len(occ),
            "occupied": occ,
            "resistance_attack_log": rec["resistance_attack_log"],
            "resistance_attack_log_complete": all(
                row.get("row_complete") is True for row in rec["resistance_attack_log"]
            ),
        }

    out = {
        "meta": {
            "source_filename": source.name,
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "notes": {
                "occupier_side": "This is the OCCUPIER-side governance view "
                "(countries.<TAG>.occupation_status). The per-STATE own resistance "
                "scalar is in state_detail.json; here we have the occupier->occupied "
                "grouping, occupier-side resistance/compliance, and the per-state "
                "occupation_law assignment.",
                "collaboration": "countries.<TAG>.collaboration is EMPTY ({}) for "
                "every country in this save; nothing to extract.",
            },
            "stats": {
                "countries_with_occupation_data": len(countries),
                "active_occupiers": n_occupiers,
                "occupier_occupied_pairs": n_pairs,
                "per_state_law_assignments": n_state_laws,
                "state_garrison_rows": n_garrison_rows,
                "state_garrison_equipment_rows": n_equipment_rows,
                "resistance_attack_history_rows": n_history_rows,
                "resistance_attack_equipment_loss_rows": n_history_equipment_rows,
            },
        },
        "countries": countries,
    }

    lines = ["=== OCCUPATION / RESISTANCE GOVERNANCE ORACLE VALIDATION ==="]
    lines.append(f"source             : {source.name} (melted={was_melted})")
    lines.append(f"save date          : {meta.get('date')}")
    lines.append(f"countries w/ data  : {len(countries)}")
    lines.append(f"active occupiers   : {n_occupiers}  "
                 f"pairs={n_pairs}  per-state laws={n_state_laws}")
    shown = 0
    for tag, rec in countries.items():
        for e in rec["occupied"]:
            laws = set(e["occupation_law_by_state"].values())
            lines.append(f"   {tag} occupies {e['occupied_tag']}: "
                         f"{len(e['states'])} states, resistance="
                         f"{e['resistance']}, compliance={e['compliance']}, "
                         f"laws={sorted(laws)}")
            shown += 1
            if shown >= 6:
                break
        if shown >= 6:
            break
    return {"occupation.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(run_cli(
        "occupation", "the game save -> occupation / resistance governance oracle.",
        build, ["occupation.json"]))
