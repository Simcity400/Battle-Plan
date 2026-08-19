#!/usr/bin/env python3
"""the game save -> INTELLIGENCE oracle (``intel.json``).

Deep-layer extension of the save oracle. Reconstructs the full the game 1.19
espionage picture per country: the intelligence agency (name, icon, upgrade
branches, capacity), every operative (status / current mission or operation,
traits, captured?), the agency's cryptology / decryption progress against each
target, the recruitable-operative pool, and each country's active + finished
intelligence OPERATIONS (target, timeline, assigned operatives & their
missions). Plus the global peace-conference state.

Key paths (verified against a real 1.19 melt + adversarial Codex pass):

  countries.<TAG>.intelligence_agency = {
      name icon is_created upgrade_progress upgrade
      upgrades = { upgrade_passive_defense=N upgrade_decryption_boost=N ... }
      cryptology = { targets = { { target decryption=yes amount } ... }
                     intel_source = { country pool id discriminant } }
      recruitment = { recruitable_operatives = { <operative> ... } }
      defense max_operative_count usable_operative_slots
              elapsed_days_for_next_slot building
      operative = { id={id type=4713} name gfx female skill experience
          traits in_progress legacy_id nationalities codename
          operation={id type}  |  mission={ <mission_type>={ target state } }
          state=on_operation|on_mission|... captured(=yes) captured_by } ... }

  countries.<TAG>.operations = {              (SIBLING of the agency, not inside)
      finished = { <op_type> = { "TAG" "count" } ... }
      priority
      running = { <op_type> = {
          id={id type} date duration target operative_slots = {
              { operative={id type} mission={ mission target_country
                                              target_state } } ... }
          phases = { "phase_a" "phase_b" ... } resources } ... } }

  countries.<TAG>.tokens = {
      intel_source = { country pool id discriminant }
      tokens = { <operation_token> = { "TARGET_TAG" ... } ... } }

  peace_conference = { ... }                  (TOP-LEVEL; empty in this save)

REALITY NOTES:
  - Decryption/cryptology lives INSIDE the agency (``cryptology.targets``), NOT
    in a top-level cipher table. ``amount`` is the accumulated decryption points
    toward cracking that target; ``decryption=yes`` flags an actively-decrypted
    target.
  - Operation ``progress`` has no explicit key; it is derivable from the save
    ``date`` vs the operation ``date`` + ``duration`` (days). We surface
    ``date``, ``duration`` and the save date so the replay can compute it.
  - Strategic intel-network graphs (top-level ``strategic_operatives``) are a
    separate large graph structure; parse_diplo already summarizes operative
    coverage, so this module does not re-walk them.
  - ``captured``/``captured_by`` are rare; supported generically on operatives.
  - ``tokens.tokens`` is persistent operation-token ownership, not a generic
    engine token dictionary.  The outer ``intel_source`` is retained only as
    an oracle validation reference; portable replay uses the exact
    owner/token/target membership and the modset-bound static dictionary.

stdlib only. Reuses the shared tokenizer via ``deep_common`` (read-only).
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, RBRACE, atom_value, iter_block_entries, _skip_block_body,
    to_int, to_float, read_scalar_list, collect_block,
)


def _read_id(tz):
    _id = _type = None
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "id":
            _id = to_int(v)
        elif k == "type":
            _type = to_int(v)
    return _id, _type



def _parse_mission(tz):
    """mission={ <mission_type>={ target/target_country state/target_state } }."""
    rec = {"mission": None, "target": None, "state": None}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            rec["mission"] = k
            for kk, oo in iter_block_entries(tz):
                if oo is LBRACE:
                    _skip_block_body(tz)
                    continue
                v = atom_value(oo)
                if v is None:
                    continue
                if kk in ("target", "target_country"):
                    rec["target"] = v
                elif kk in ("state", "target_state"):
                    rec["state"] = to_int(v)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "mission":
            rec["mission"] = v
        elif k in ("target", "target_country"):
            rec["target"] = v
        elif k in ("state", "target_state"):
            rec["state"] = to_int(v)
    return rec


def _parse_operative(tz):
    rec = {
        "op_id": None, "op_type": None,
        "name": None, "gfx": None, "female": None,
        "skill": None, "experience": None,
        "traits": [], "nationalities": [],
        "state": None,
        "operation_id": None,
        "mission": None,
        "captured": False, "captured_by": None,
        "legacy_id": None, "script_id": None,
    }
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                rec["op_id"], rec["op_type"] = _read_id(tz)
            elif k == "traits":
                rec["traits"] = read_scalar_list(tz)
            elif k == "nationalities":
                rec["nationalities"] = read_scalar_list(tz)
            elif k == "operation":
                oid, _ot = _read_id(tz)
                rec["operation_id"] = oid
            elif k == "mission":
                rec["mission"] = _parse_mission(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "name":
            rec["name"] = v
        elif k == "gfx":
            rec["gfx"] = v
        elif k == "female":
            rec["female"] = (v == "yes")
        elif k == "skill":
            rec["skill"] = to_int(v)
        elif k == "experience":
            rec["experience"] = to_float(v)
        elif k == "state":
            rec["state"] = v
        elif k == "captured":
            rec["captured"] = (v == "yes")
        elif k == "captured_by":
            rec["captured_by"] = v
        elif k == "legacy_id":
            rec["legacy_id"] = to_int(v)
        elif k == "script_id":
            rec["script_id"] = to_int(v)
    return rec



def _parse_cryptology(tz):
    """cryptology={ targets={ { target decryption amount } ... } intel_source }."""
    rec = {"targets": [], "intel_source": None}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "targets":
                while True:
                    tok = tz.next()
                    if tok is None or tok is RBRACE:
                        break
                    if tok is LBRACE:
                        t = {"target": None, "decryption": False,
                             "amount": None}
                        for kk, oo in iter_block_entries(tz):
                            if oo is LBRACE:
                                _skip_block_body(tz)
                                continue
                            v = atom_value(oo)
                            if v is None:
                                continue
                            if kk == "target":
                                t["target"] = v
                            elif kk == "decryption":
                                t["decryption"] = (v == "yes")
                            elif kk == "amount":
                                t["amount"] = to_float(v)
                        rec["targets"].append(t)
            elif k == "intel_source":
                rec["intel_source"] = collect_block(tz)
            else:
                _skip_block_body(tz)
    return rec


def _parse_recruitment(tz):
    """recruitment={ recruitable_operatives={ <operative> ... } }."""
    out = []
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "recruitable_operatives":
                while True:
                    tok = tz.next()
                    if tok is None or tok is RBRACE:
                        break
                    if tok is LBRACE:
                        out.append(_parse_operative(tz))
            else:
                _skip_block_body(tz)
    return out


def _parse_intelligence_agency(tz, rec):
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "operative":
                rec["operatives"].append(_parse_operative(tz))
            elif k == "upgrades":
                ups = collect_block(tz)
                rec["upgrades"] = {kk: to_int(vv) for kk, vv in ups.items()}
            elif k == "cryptology":
                rec["cryptology"] = _parse_cryptology(tz)
            elif k == "recruitment":
                rec["recruitable_operatives"] = _parse_recruitment(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "name":
            rec["agency_name"] = v
        elif k == "icon":
            rec["icon"] = v
        elif k == "is_created":
            rec["is_created"] = (v == "yes")
        elif k == "upgrade_progress":
            rec["upgrade_progress"] = to_float(v)
        elif k == "upgrade":
            rec["current_upgrade"] = v
        elif k == "defense":
            rec["defense"] = to_float(v)
        elif k == "max_operative_count":
            rec["max_operative_count"] = to_int(v)
        elif k == "usable_operative_slots":
            rec["usable_operative_slots"] = to_int(v)



def _parse_running_operation(tz, op_type):
    rec = {
        "type": op_type, "op_id": None, "date": None, "duration": None,
        "target": None, "phases": [], "operatives": [],
    }
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                rec["op_id"], _ot = _read_id(tz)
            elif k == "phases":
                rec["phases"] = read_scalar_list(tz)
            elif k == "operative_slots":
                while True:
                    tok = tz.next()
                    if tok is None or tok is RBRACE:
                        break
                    if tok is LBRACE:
                        slot = {"operative_id": None, "mission": None}
                        for kk, oo in iter_block_entries(tz):
                            if oo is LBRACE:
                                if kk == "operative":
                                    oid, _t = _read_id(tz)
                                    slot["operative_id"] = oid
                                elif kk == "mission":
                                    slot["mission"] = _parse_mission(tz)
                                else:
                                    _skip_block_body(tz)
                        rec["operatives"].append(slot)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "date":
            rec["date"] = v
        elif k == "duration":
            rec["duration"] = to_int(v)
        elif k == "target":
            rec["target"] = v
    return rec


def _parse_operations(tz, rec):
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "running":
                for otype, oop in iter_block_entries(tz):
                    if oop is LBRACE:
                        rec["operations_running"].append(
                            _parse_running_operation(tz, otype))
            elif k == "finished":
                for otype, oop in iter_block_entries(tz):
                    if oop is LBRACE:
                        vals = read_scalar_list(tz)
                        rec["operations_finished"].append(
                            {"type": otype, "values": vals})
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "priority":
            rec["operations_priority"] = to_int(v)


def _parse_operation_tokens(tz, rec):
    """Parse the country-owned operation-token target sets.

    The raw ``intel_source`` tuple is serializer/reference identity.  It is
    useful for proving that the block belongs to this country, but is not a
    portable replay identity and is therefore kept separate from ``owned``.
    """
    for k, op in iter_block_entries(tz):
        if op is not LBRACE:
            continue
        if k == "intel_source":
            rec["operation_token_source"] = collect_block(tz)
        elif k == "tokens":
            for token, token_op in iter_block_entries(tz):
                if token_op is not LBRACE or not token:
                    continue
                targets = []
                seen = set()
                for target in read_scalar_list(tz):
                    if target not in seen:
                        seen.add(target)
                        targets.append(target)
                rec["operation_tokens"][token] = targets
        else:
            _skip_block_body(tz)



def _new_country_rec():
    return {
        "agency_name": None, "icon": None, "is_created": False,
        "upgrade_progress": None, "current_upgrade": None,
        "upgrades": {}, "cryptology": {"targets": [], "intel_source": None},
        "defense": None, "max_operative_count": None,
        "usable_operative_slots": None,
        "operatives": [], "recruitable_operatives": [],
        "operations_running": [], "operations_finished": [],
        "operations_priority": None,
        "operation_tokens": {}, "operation_token_source": None,
    }


def _parse_one_country(tz, tag, by_tag):
    rec = _new_country_rec()
    has_intel = False
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "intelligence_agency":
                _parse_intelligence_agency(tz, rec)
                has_intel = True
            elif k == "operations":
                _parse_operations(tz, rec)
                has_intel = True
            elif k == "tokens":
                _parse_operation_tokens(tz, rec)
                has_intel = True
            else:
                _skip_block_body(tz)
    if (has_intel and (rec["agency_name"] or rec["operatives"]
                       or rec["operations_running"]
                       or rec["cryptology"]["targets"]
                       or rec["operation_tokens"])):
        by_tag[tag] = rec


def _parse_countries(tz, by_tag):
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        _parse_one_country(tz, tag, by_tag)


def _parse_peace_conference(tz, pc):
    """peace_conference is empty here; capture whatever scalars exist."""
    pc["active"] = False
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            pc["active"] = True
            _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is not None:
            pc["active"] = True
            pc[k] = v



def parse_intel_text(text: str):
    by_tag: dict = {}
    pc: dict = {"active": False}
    meta = dc.top_level_pass(
        text,
        {
            "countries": lambda tz: _parse_countries(tz, by_tag),
            "peace_conference": lambda tz: _parse_peace_conference(tz, pc),
        },
        scalar_keys=("date", "version", "player"),
    )
    return by_tag, pc, meta


def build(text, source, was_melted):
    by_tag, peace, meta = parse_intel_text(text)

    total_ops = total_running = total_finished = 0
    total_crypto_targets = total_decrypting = 0
    total_captured = 0
    agencies_named = 0
    op_assign_total = op_assign_resolved = 0
    operation_token_owner_count = 0
    operation_token_membership_count = 0
    operation_token_source_mismatch_count = 0
    operation_token_source_shape_rejected_count = 0

    countries = {}
    for tag, rec in sorted(by_tag.items()):
        if rec["agency_name"]:
            agencies_named += 1
        n_ops = len(rec["operatives"])
        total_ops += n_ops
        total_running += len(rec["operations_running"])
        total_finished += len(rec["operations_finished"])
        total_crypto_targets += len(rec["cryptology"]["targets"])
        total_decrypting += sum(
            1 for t in rec["cryptology"]["targets"] if t["decryption"])
        total_captured += sum(1 for o in rec["operatives"] if o["captured"])

        operation_tokens = {
            token: list(targets)
            for token, targets in sorted(rec["operation_tokens"].items())
        }
        if operation_tokens:
            operation_token_owner_count += 1
            operation_token_membership_count += sum(
                len(targets) for targets in operation_tokens.values())
        source_ref = rec["operation_token_source"]
        if source_ref is not None:
            if source_ref.get("country") != tag:
                operation_token_source_mismatch_count += 1
            if any(source_ref.get(key) != expected for key, expected in (
                    ("pool", "1"), ("id", "1"), ("discriminant", "0"))):
                operation_token_source_shape_rejected_count += 1

        own_op_ids = {o["op_id"] for o in rec["operatives"] if o["op_id"]}
        for runop in rec["operations_running"]:
            for slot in runop["operatives"]:
                if slot["operative_id"] is not None:
                    op_assign_total += 1
                    if slot["operative_id"] in own_op_ids:
                        op_assign_resolved += 1

        countries[tag] = {
            "agency_name": rec["agency_name"],
            "icon": rec["icon"],
            "is_created": rec["is_created"],
            "upgrade_progress": rec["upgrade_progress"],
            "current_upgrade": rec["current_upgrade"],
            "upgrades": rec["upgrades"],
            "capacity": {
                "max_operatives": rec["max_operative_count"],
                "usable_slots": rec["usable_operative_slots"],
                "defense": rec["defense"],
            },
            "cryptology": rec["cryptology"],
            "operatives": rec["operatives"],
            "recruitable_operatives": rec["recruitable_operatives"],
            "operations": {
                "priority": rec["operations_priority"],
                "running": rec["operations_running"],
                "finished": rec["operations_finished"],
            },
            "operation_tokens": {
                "owned": operation_tokens,
            },
            "counts": {
                "operatives": n_ops,
                "recruitable": len(rec["recruitable_operatives"]),
                "running_operations": len(rec["operations_running"]),
                "finished_operations": len(rec["operations_finished"]),
                "cryptology_targets": len(rec["cryptology"]["targets"]),
                "operation_token_memberships": sum(
                    len(targets) for targets in operation_tokens.values()),
            },
        }

    out = {
        "meta": {
            "source_filename": source.name,
            "source_path_redacted": dc.ps._redact(source),
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "notes": {
                "agency": "intelligence_agency holds operatives, upgrade "
                "branches, cryptology/decryption and the recruitable pool. "
                "operative.state in {on_operation,on_mission,...}; operation_id "
                "links to operations.running; mission is an in-field task.",
                "operations": "operations is a SIBLING of the agency (per "
                "country), with running + finished. Operation progress has no "
                "explicit key: derive from save_date vs op date+duration(days).",
                "cryptology": "cryptology.targets[].amount = accumulated "
                "decryption points vs that target; decryption=true flags an "
                "actively-decrypted target.",
                "operation_tokens": "countries.<TAG>.tokens.tokens is the "
                "persistent owner/token/target operation-token set. The raw "
                "intel_source {country,pool,id,discriminant} tuple is an "
                "owner-reference validation field and is deliberately not "
                "projected into replay identity.",
                "peace_conference": "top-level peace_conference is empty in this "
                "save (no active conference).",
                "portraits": "operative.gfx is a GFX sprite NAME, not image "
                "data. Rendering operative portrait art is a SEPARATE "
                "asset-extraction step (FLAGGED as an asset need).",
            },
            "peace_conference": peace,
            "stats": {
                "countries_with_intel": len(countries),
                "agencies_named": agencies_named,
                "total_operatives": total_ops,
                "total_captured_operatives": total_captured,
                "total_running_operations": total_running,
                "total_finished_operations": total_finished,
                "total_cryptology_targets": total_crypto_targets,
                "targets_being_decrypted": total_decrypting,
                "operation_slots_total": op_assign_total,
                "operation_slots_operative_resolved": op_assign_resolved,
                "operation_token_owners": operation_token_owner_count,
                "operation_token_memberships": operation_token_membership_count,
                "operation_token_source_mismatches":
                    operation_token_source_mismatch_count,
                "operation_token_source_shape_rejections":
                    operation_token_source_shape_rejected_count,
            },
        },
        "countries": countries,
    }

    st = out["meta"]["stats"]
    ger = countries.get("GER", {})
    lines = ["=== INTELLIGENCE ORACLE VALIDATION ==="]
    lines.append(f"source        : {source.name} (melted={was_melted})")
    lines.append(f"save date     : {meta.get('date')}")
    lines.append(f"countries     : {st['countries_with_intel']} with intel "
                 f"({st['agencies_named']} named agencies)")
    lines.append(f"operatives    : {st['total_operatives']} "
                 f"({st['total_captured_operatives']} captured)")
    lines.append(f"operations    : {st['total_running_operations']} running, "
                 f"{st['total_finished_operations']} finished")
    lines.append(f"cryptology    : {st['total_cryptology_targets']} targets, "
                 f"{st['targets_being_decrypted']} actively decrypting")
    lines.append(f"op slots      : {st['operation_slots_operative_resolved']}/"
                 f"{st['operation_slots_total']} operative ids resolved to an "
                 f"own operative")
    lines.append(f"op tokens     : {st['operation_token_memberships']} memberships "
                 f"across {st['operation_token_owners']} owners; "
                 f"source mismatches={st['operation_token_source_mismatches']} "
                 f"shape rejects={st['operation_token_source_shape_rejections']}")
    lines.append(f"peace conf    : active={peace.get('active')}")
    if ger:
        c = ger
        lines.append("-- GER spot-check --")
        lines.append(f"   agency={c['agency_name']!r} icon={c['icon']!r} "
                     f"upgrade_progress={c['upgrade_progress']}")
        lines.append(f"   {c['counts']['operatives']} operatives, "
                     f"{c['counts']['recruitable']} recruitable, "
                     f"{c['counts']['running_operations']} running ops, "
                     f"{c['counts']['cryptology_targets']} crypto targets")
        for o in c["operatives"][:3]:
            lines.append(f"   operative {o['name']!r} skill={o['skill']} "
                         f"state={o['state']} op_id={o['operation_id']} "
                         f"mission={(o['mission'] or {}).get('mission')} "
                         f"nat={o['nationalities']} captured={o['captured']}")
        for t in c["cryptology"]["targets"][:4]:
            lines.append(f"   crypto vs {t['target']}: amount={t['amount']} "
                         f"decrypting={t['decryption']}")
        for ro in c["operations"]["running"][:3]:
            lines.append(f"   RUNNING {ro['type']} vs {ro['target']} "
                         f"date={ro['date']} dur={ro['duration']} "
                         f"slots={len(ro['operatives'])} "
                         f"phases={len(ro['phases'])}")
    return {"intel.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(dc.run_cli(
        "intel", "the game save -> intelligence oracle.",
        build, ["intel.json"]))
