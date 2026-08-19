#!/usr/bin/env python3
"""the game save -> special projects / facilities / scientists (``projects.json``).

The 1.16+ Special-Projects mechanic (research facilities, prototype projects,
scientist characters) is only *thinly* surfaced by ``parse_country_extra.py``:
that module COUNTS started/completed projects, lists facilities as bare
``{building, province, country}``, and reads the per-specialization breakthrough
points. It deliberately does NOT extract project phases / iterations / progress /
rewards, facility scientist assignment / dismantle status / supply, or the
scientist roster at all. This module is the deep reach for that section.

Three deep sub-domains, all per country:

  - **special projects**  : ``countries.<TAG>.program_status.project_pool.project``
    -> per project token: ``id`` ref, ``current_state`` (0=locked/idle,
    1=prototyping, 2+=in-production/complete), prototype ``iterations`` +
    ``phase_progress`` + ``project_progress``, ``start`` / ``completed`` flags,
    fired prototype-reward flags (``flags``), and the prototype-reward HISTORY
    (each fired ``option`` + ``prototype_reward`` + date). Rolled up to
    started/completed counts and an active list.

  - **facilities**        : ``countries.<TAG>.program_status.program`` -> per
    facility: ``id`` ref, location ``province``, ``building`` type
    (land/naval/air/nuclear_facility), the ACTIVE ``project`` token being worked
    (absent => idle), the assigned ``scientist`` character ref, the
    ``delayed_dismantle_facility`` countdown (target/current/timed => dismantle
    status), ``delayed_unassigned_scientist`` countdown, the facility ``supply``
    block (tag / province / motorization_level), and ``unread_prototype_rewards``.

  - **scientists**        : top-level ``characters`` (historical+dynamic) whose
    ``character`` carries a ``scientist={ traits skills }`` block -> name / token /
    country / gender, scientist ``traits`` (specializations + perks), and the
    four ``specialization_*`` skill levels (nuclear / naval / air / land).
    Cross-referenced to the facility that has them assigned (by char id).

NOT in the program block (documented, never faked): facility *research-speed
bonus* and scientist *injured* state + injury *cooldown* are computed at runtime
and absent from the save. Facility *health* is likewise absent here, but is a
``damage_building`` target of prototype rewards, so a damaged facility's health
would live in the STATE/province building-damage data (parse_state_detail
territory), not in this section. The scientist *reassignment* cooldown IS stored
(``facilities[].unassigned_scientist``).

stdlib only. Reuses the shared tokenizer via ``deep_common``.
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, RBRACE, EQUALS, atom_value, iter_block_entries, _skip_block_body,
    read_id_block, read_scalar_list, to_int, to_float,
)



def _parse_progress(tz):
    """progress={ current_state=N prototype_state={ project_progress iterations
       phase_progress } stopping_state={ phase_progress } }."""
    rec = {"current_state": None, "project_progress": None,
           "iterations": None, "phase_progress": None,
           "stopping_phase_progress": None}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "prototype_state":
                for kk, oo in iter_block_entries(tz):
                    if oo is LBRACE:
                        _skip_block_body(tz)
                        continue
                    v = atom_value(oo)
                    if v is None:
                        continue
                    if kk == "project_progress":
                        rec["project_progress"] = to_float(v)
                    elif kk == "iterations":
                        rec["iterations"] = to_int(v)
                    elif kk == "phase_progress":
                        rec["phase_progress"] = to_float(v)
            elif k == "stopping_state":
                for kk, oo in iter_block_entries(tz):
                    if oo is LBRACE:
                        _skip_block_body(tz)
                        continue
                    v = atom_value(oo)
                    if v is not None and kk == "phase_progress":
                        rec["stopping_phase_progress"] = to_float(v)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is not None and k == "current_state":
            rec["current_state"] = to_int(v)
    return rec


def _parse_history_context(tz):
    """context={ province character={id type} progress country scientist state }.

    A snapshot of where/who fired the reward. We keep the stable analytic
    scalars (province, progress) and the scientist character id; the bare
    ``country`` / ``scientist`` / ``state`` numerics here are packed run-time
    ref handles, not tags — kept verbatim for replay/debug, not dereferenced."""
    ctx = {"province": None, "progress": None, "scientist_char_id": None}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "character":
                cid, _ct = read_id_block(tz)
                ctx["scientist_char_id"] = cid
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "province":
            ctx["province"] = to_int(v)
        elif k == "progress":
            ctx["progress"] = to_float(v)
    return ctx


def _parse_reward_history(tz):
    """history={ { context={...} option=X prototype_reward=Y date=D } ... }.

    Returns a list of {option, prototype_reward, date, context}. The context is
    a slimmed fire-time snapshot (province / progress / scientist char id)."""
    out = []
    depth = 1
    while depth > 0:
        tok = tz.next()
        if tok is None:
            break
        if tok is RBRACE:
            depth -= 1
            continue
        if tok is LBRACE:
            ev = {"option": None, "prototype_reward": None, "date": None,
                  "context": None}
            for k, op in iter_block_entries(tz):
                if op is LBRACE:
                    if k == "context":
                        ev["context"] = _parse_history_context(tz)
                    else:
                        _skip_block_body(tz)
                    continue
                v = atom_value(op)
                if v is None:
                    continue
                if k == "option":
                    ev["option"] = v
                elif k == "prototype_reward":
                    ev["prototype_reward"] = v
                elif k == "date":
                    ev["date"] = v
            out.append(ev)
    return out


def _parse_project(tz, token):
    """One project: <token>={ id={..} progress={..} variables flags history
       start=yes/no completed=yes/no }."""
    rec = {"token": token, "id": None, "type": None,
           "current_state": None, "project_progress": None,
           "iterations": None, "phase_progress": None,
           "stopping_phase_progress": None,
           "project_flags": [], "reward_history": [],
           "start": False, "completed": False}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                rec["id"], rec["type"] = read_id_block(tz)
            elif k == "progress":
                pr = _parse_progress(tz)
                rec["current_state"] = pr["current_state"]
                rec["project_progress"] = pr["project_progress"]
                rec["iterations"] = pr["iterations"]
                rec["phase_progress"] = pr["phase_progress"]
                rec["stopping_phase_progress"] = pr["stopping_phase_progress"]
            elif k == "flags":
                flags = []
                for fk, fo in iter_block_entries(tz):
                    if fo is LBRACE:
                        fval = fdate = None
                        for ik, io in iter_block_entries(tz):
                            if io is LBRACE:
                                _skip_block_body(tz)
                                continue
                            iv = atom_value(io)
                            if iv is None:
                                continue
                            if ik == "value":
                                fval = to_int(iv)
                            elif ik == "date":
                                fdate = iv
                        flags.append({"flag": fk, "value": fval, "date": fdate})
                rec["project_flags"] = flags
            elif k == "history":
                rec["reward_history"] = _parse_reward_history(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "start":
            rec["start"] = (v == "yes")
        elif k == "completed":
            rec["completed"] = (v == "yes")
    return rec


def _parse_project_pool(tz):
    """project_pool={ project={ <token>={..} <token>={..} } resources={..} }."""
    projects = []
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key != "project":
            _skip_block_body(tz)
            continue
        for ptoken, op in iter_block_entries(tz):
            if op is not LBRACE:
                continue
            projects.append(_parse_project(tz, ptoken))
    return projects



def _parse_delayed(tz):
    """delayed_dismantle_facility / delayed_unassigned_scientist =
       { target current timed }."""
    rec = {"target": None, "current": None, "timed": None}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "target":
            rec["target"] = to_int(v)
        elif k == "current":
            rec["current"] = to_int(v)
        elif k == "timed":
            rec["timed"] = (v == "yes")
    return rec


def _parse_supply(tz):
    """supply={ tag province_id id={..} motorization_level }."""
    rec = {"tag": None, "province_id": None, "motorization_level": None}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "tag":
            rec["tag"] = v
        elif k == "province_id":
            rec["province_id"] = to_int(v)
        elif k == "motorization_level":
            rec["motorization_level"] = to_int(v)
    return rec


def _parse_program(tz):
    """One facility: program={ id country province project scientist={..}
       delayed_dismantle_facility delayed_unassigned_scientist supply building
       unread_prototype_rewards }."""
    rec = {"id": None, "type": None, "country": None, "province": None,
           "active_project": None, "scientist_id": None, "scientist_type": None,
           "dismantle": None, "unassigned_scientist": None, "supply": None,
           "building": None, "unread_prototype_rewards": None}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                rec["id"], rec["type"] = read_id_block(tz)
            elif k == "scientist":
                for sk, so in iter_block_entries(tz):
                    if so is not LBRACE:
                        continue
                    if sk == "scientist":
                        rec["scientist_id"], rec["scientist_type"] = \
                            read_id_block(tz)
                    elif sk == "delayed_unassigned_scientist":
                        rec["unassigned_scientist"] = _parse_delayed(tz)
                    else:
                        _skip_block_body(tz)
            elif k == "delayed_dismantle_facility":
                rec["dismantle"] = _parse_delayed(tz)
            elif k == "delayed_unassigned_scientist":
                rec["unassigned_scientist"] = _parse_delayed(tz)
            elif k == "supply":
                rec["supply"] = _parse_supply(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "country":
            rec["country"] = v
        elif k == "province":
            rec["province"] = to_int(v)
        elif k == "project":
            rec["active_project"] = v
        elif k == "building":
            rec["building"] = v
        elif k == "unread_prototype_rewards":
            rec["unread_prototype_rewards"] = to_int(v)
    return rec



def _parse_program_status(tz):
    projects = []
    facilities = []
    breakthrough = {}
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key == "project_pool":
            projects.extend(_parse_project_pool(tz))
        elif key == "program":
            facilities.append(_parse_program(tz))
        elif key == "breakthrough":
            for k, op in iter_block_entries(tz):
                if op is LBRACE:
                    _skip_block_body(tz)
                    continue
                v = atom_value(op)
                if v is not None:
                    breakthrough[k] = to_int(v)
        else:
            _skip_block_body(tz)
    return projects, facilities, breakthrough


def _parse_countries(tz, by_tag):
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        ps_found = False
        for key, op in iter_block_entries(tz):
            if op is LBRACE:
                if key == "program_status":
                    projects, facilities, breakthrough = \
                        _parse_program_status(tz)
                    by_tag[tag] = {
                        "projects": projects,
                        "facilities": facilities,
                        "breakthrough": breakthrough,
                    }
                    ps_found = True
                else:
                    _skip_block_body(tz)
        if not ps_found:
            pass



def _parse_scientist_block(tz):
    """scientist={ traits={..} skills={ specialization_*={ level } ... } }."""
    traits = []
    skills = {}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "traits":
                traits = read_scalar_list(tz)
            elif k == "skills":
                for sk, so in iter_block_entries(tz):
                    if so is not LBRACE:
                        continue
                    lvl = 0
                    for lk, lo in iter_block_entries(tz):
                        if lo is LBRACE:
                            _skip_block_body(tz)
                            continue
                        v = atom_value(lo)
                        if v is not None and lk == "level":
                            lvl = to_int(v)
                    skills[sk] = lvl
            else:
                _skip_block_body(tz)
            continue
    return traits, skills


def _parse_character_for_scientist(tz):
    """One characters.*.character block. Returns a scientist record IF it carries
    a scientist={traits,skills} sub-block, else None (but always consumes block).
    """
    rec = {"char_id": None, "char_type": None, "token": None, "name": None,
           "country": None, "gender": None, "traits": [], "skills": {}}
    is_scientist = False
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                rec["char_id"], rec["char_type"] = read_id_block(tz)
            elif k == "scientist":
                t, s = _parse_scientist_block(tz)
                rec["traits"] = t
                rec["skills"] = s
                is_scientist = True
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "token":
            rec["token"] = v
        elif k == "name":
            rec["name"] = v
        elif k == "country":
            rec["country"] = v
        elif k == "gender":
            rec["gender"] = v
    return rec if is_scientist else None


def _parse_characters(tz, scientists):
    """characters={ historical={ character={..} ... } dynamic={..} }."""
    for k, op in iter_block_entries(tz):
        if op is not LBRACE:
            continue
        if k in ("historical", "dynamic"):
            for kk, oo in iter_block_entries(tz):
                if oo is not LBRACE:
                    continue
                if kk == "character":
                    rec = _parse_character_for_scientist(tz)
                    if rec is not None and rec["char_id"] is not None:
                        scientists.append(rec)
                else:
                    _skip_block_body(tz)
        else:
            _skip_block_body(tz)



_SPECS = ("specialization_nuclear", "specialization_naval",
          "specialization_air", "specialization_land")


def build(text, source, was_melted):
    by_tag = {}
    scientists = []
    meta = dc.top_level_pass(
        text,
        {
            "countries": lambda tz: _parse_countries(tz, by_tag),
            "character_manager": lambda tz: _parse_characters(tz, scientists),
        },
        scalar_keys=("date", "version", "player"),
    )

    sci_by_id = {}
    for s in scientists:
        if s["char_id"] is not None:
            sci_by_id[s["char_id"]] = s

    countries = {}
    total_projects = total_started = total_completed = total_facilities = 0
    for tag, rec in sorted(by_tag.items()):
        projects = rec["projects"]
        facilities = rec["facilities"]
        started = [p for p in projects if p["start"]]
        completed = [p for p in projects if p["completed"]]
        active = [p for p in started if not p["completed"]]

        worked = {f.get("active_project") for f in facilities
                  if f.get("active_project")}

        for f in facilities:
            sid = f.get("scientist_id")
            s = sci_by_id.get(sid) if sid is not None else None
            f["scientist_token"] = s["token"] if s else None
            d = f.get("dismantle") or {}
            f["dismantle_in_progress"] = bool(d.get("current"))

        total_projects += len(projects)
        total_started += len(started)
        total_completed += len(completed)
        total_facilities += len(facilities)

        countries[tag] = {
            "projects_total": len(projects),
            "projects_started": len(started),
            "projects_completed": len(completed),
            "projects_active": len(active),
            "active_projects": [
                {"token": p["token"], "current_state": p["current_state"],
                 "iterations": p["iterations"],
                 "project_progress": p["project_progress"],
                 "phase_progress": p["phase_progress"],
                 "in_facility": p["token"] in worked,
                 "project_flags": [fl["flag"] for fl in p["project_flags"]]}
                for p in active],
            "completed_projects": [p["token"] for p in completed],
            "projects": projects,
            "facilities": facilities,
            "breakthrough": rec["breakthrough"],
        }

    sci_by_country = {}
    for s in scientists:
        c = s["country"]
        sci_by_country.setdefault(c, []).append({
            "char_id": s["char_id"],
            "char_type": s["char_type"],
            "country": s["country"],
            "token": s["token"],
            "name_token": s["name"],
            "gender": s["gender"],
            "traits": s["traits"],
            "skills": {k: s["skills"].get(k, 0) for k in _SPECS},
        })

    base_meta = {
        "source_filename": source.name,
        "source_path_redacted": dc.ps._redact(source),
        "was_melted": was_melted,
        "save_date": meta.get("date"),
        "save_version": meta.get("version"),
        "player": meta.get("player"),
    }

    out = {
        "meta": {**base_meta, "stats": {
            "countries_with_program_status": len(countries),
            "projects_total": total_projects,
            "projects_started": total_started,
            "projects_completed": total_completed,
            "facilities_total": total_facilities,
            "scientists_total": len(scientists),
            "scientist_countries": len(sci_by_country),
        }, "notes": {
            "current_state": "an internal project state-machine ENUM, NOT a "
            "clean lifecycle field — a started-but-unfinished project can read "
            "0. Use start / completed / facility-assignment for status, not "
            "current_state alone.",
            "active_vs_worked": "active_projects = started & not completed. "
            "in_facility=true means a facility's program.project points at that "
            "token (i.e. currently being worked); active without in_facility = "
            "started but no facility currently assigned.",
            "project_flags": "PROJECT flags set by fired prototype rewards; "
            "later rewards test has_project_flag to bias weights. value/date kept "
            "in countries.<TAG>.projects[].project_flags.",
            "scientist_name": "name_token / scientist_token are localisation "
            "keys, not resolved human display names (no loc table joined here).",
            "not_stored": "facility research-speed bonus, scientist injured-state "
            "and injury cooldown are COMPUTED at runtime, absent from the save "
            "(never faked). NOTE: facility HEALTH is also absent from the program "
            "block, but prototype rewards can damage_building the facility, so a "
            "damaged facility's health would live in the STATE/province building "
            "damage data (parse_state_detail territory), not here. The scientist "
            "REASSIGNMENT cooldown IS stored: facilities[].unassigned_scientist "
            "(delayed_unassigned_scientist target/current/timed).",
            "reward_history": "each fired prototype-reward {option, "
            "prototype_reward, date} plus a slimmed fire-time context "
            "(province / progress / scientist char id).",
        }},
        "countries": countries,
        "scientists": sci_by_country,
    }

    ger = countries.get("GER", {})
    ger_sci = sci_by_country.get("GER", [])
    lines = ["=== SPECIAL-PROJECTS / FACILITIES / SCIENTISTS ORACLE VALIDATION ==="]
    lines.append(f"source            : {source.name} (melted={was_melted})")
    lines.append(f"save date         : {meta.get('date')}")
    lines.append(f"countries w/ progs: {len(countries)}")
    lines.append(f"projects total    : {total_projects} "
                 f"(started={total_started} completed={total_completed})")
    lines.append(f"facilities total  : {total_facilities}")
    lines.append(f"scientists total  : {len(scientists)} "
                 f"across {len(sci_by_country)} countries")
    lines.append("")
    lines.append(f"GER projects      : total={ger.get('projects_total')} "
                 f"started={ger.get('projects_started')} "
                 f"completed={ger.get('projects_completed')} "
                 f"active={ger.get('projects_active')}")
    lines.append(f"GER breakthrough  : {ger.get('breakthrough')}")
    facs = ger.get("facilities", [])
    lines.append(f"GER facilities    : {len(facs)}")
    for f in facs[:6]:
        lines.append(f"  facility id={f['id']} {f['building']} "
                     f"prov={f['province']} project={f['active_project']} "
                     f"scientist={f['scientist_token']} "
                     f"unread_rewards={f['unread_prototype_rewards']} "
                     f"dismantle_in_progress={f['dismantle_in_progress']}")
    ap = ger.get("active_projects", [])
    lines.append(f"GER active projs  : {len(ap)}")
    for p in ap[:5]:
        lines.append(f"  {p['token']} state={p['current_state']} "
                     f"in_facility={p['in_facility']} "
                     f"iters={p['iterations']} "
                     f"proj_progress={p['project_progress']} "
                     f"phase={p['phase_progress']} "
                     f"flags_set={len(p['project_flags'])}")
    lines.append(f"GER scientists    : {len(ger_sci)}")
    for s in ger_sci[:5]:
        top = max(s["skills"].items(), key=lambda kv: kv[1]) if s["skills"] else (None, 0)
        lines.append(f"  {s['name_token']} traits={s['traits']} "
                     f"top_skill={top[0]}={top[1]}")
    return {"projects.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(dc.run_cli(
        "projects",
        "the game save -> special projects / facilities / scientists.",
        build, ["projects.json"]))
