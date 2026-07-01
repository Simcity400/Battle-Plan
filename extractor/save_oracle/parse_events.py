#!/usr/bin/env python3
"""the game save -> EVENT LEDGERS + THREAT + CONVOY-LOSS history (``events.json``).

Closes four top-level domains the older oracle treated as a renderer-only WALL
(W5). Two of them ARE persisted ledgers and refute the blanket "history not
persisted" claim:

  - ``fired_event_names``     : the persisted list of every scripted-event id
      fired this game (``id="france.10"`` ...). Recovers *which* events fired
      (not the exact intra-sample timing). 109 in the sample.
  - ``pending_events``        : the queued-but-not-yet-fired event list (empty in
      the sample save — parsed anyway, future-proof).
  - ``threat``                : the world-tension *contributor* ledger
      (``threat={ threat final_threat daily tag date label }`` per entry) — the
      individual threat-generating actions (Anschluss, Danzig-or-War, war
      declarations). The aggregate world_tension VALUE proper is not a stored
      top-level key in this save; the scaling inputs (``average_major_ic``,
      ``tension_scaling_base_country``) are captured in scenario/world.json.
  - ``sunk_convoys_history``  : the persisted per-month convoy-loss ledger
      (``sunk_convoy={ month convoys killer_country owner }``). 199 in the
      sample. (Warship sunk-ship history is NOT in the save — that wall holds.)

stdlib only. Reuses the shared tokenizer via ``deep_common``.
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, atom_value, iter_block_entries, _skip_block_body, to_int, to_float,
)


def _parse_fired_events(tz, out):
    """fired_event_names={ id="x" id="y" ... }."""
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        if key == "id":
            v = atom_value(opener)
            if v is not None:
                out.append(v)


def _parse_pending_events(tz, out):
    """pending_events={ ... } — shape varies; capture event ids if present."""
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            # an event-instance block; pull an id/name if present
            rec = {}
            for k, op in iter_block_entries(tz):
                if op is LBRACE:
                    _skip_block_body(tz)
                    continue
                v = atom_value(op)
                if v is None:
                    continue
                if k in ("id", "name", "event", "days", "random"):
                    rec[k] = v
            if rec:
                out.append(rec)
        else:
            v = atom_value(opener)
            if v is not None and key == "id":
                out.append({"id": v})


def _parse_threat(tz, out):
    """threat={ threat={ threat final_threat daily tag date label } ... }."""
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key != "threat":
            _skip_block_body(tz)
            continue
        rec = {"threat": None, "final_threat": None, "daily": None,
               "tag": None, "date": None, "label": None}
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                _skip_block_body(tz)
                continue
            v = atom_value(op)
            if v is None:
                continue
            if k == "threat":
                rec["threat"] = to_float(v)
            elif k == "final_threat":
                rec["final_threat"] = to_float(v)
            elif k == "daily":
                rec["daily"] = to_float(v)
            elif k == "tag":
                rec["tag"] = v
            elif k == "date":
                rec["date"] = v
            elif k == "label":
                rec["label"] = v
        out.append(rec)


def _parse_sunk_convoys(tz, out):
    """sunk_convoys_history={ sunk_convoy={ month convoys killer_country owner } ...}."""
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key != "sunk_convoy":
            _skip_block_body(tz)
            continue
        rec = {"month": None, "convoys": None,
               "killer_country": None, "owner": None}
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                _skip_block_body(tz)
                continue
            v = atom_value(op)
            if v is None:
                continue
            if k == "month":
                rec["month"] = to_int(v)
            elif k == "convoys":
                rec["convoys"] = to_int(v)
            elif k == "killer_country":
                rec["killer_country"] = v
            elif k == "owner":
                rec["owner"] = v
        out.append(rec)


def build(text, source, was_melted):
    fired = []
    pending = []
    threat = []
    sunk = []
    meta = dc.top_level_pass(
        text,
        {
            "fired_event_names": lambda tz: _parse_fired_events(tz, fired),
            "pending_events": lambda tz: _parse_pending_events(tz, pending),
            "threat": lambda tz: _parse_threat(tz, threat),
            "sunk_convoys_history": lambda tz: _parse_sunk_convoys(tz, sunk),
        },
        scalar_keys=("date", "version", "player"),
    )

    # Aggregate convoy losses by killer and by owner (victim).
    by_killer = {}
    by_owner = {}
    total_convoys = 0
    for s in sunk:
        c = s["convoys"] or 0
        total_convoys += c
        if s["killer_country"]:
            by_killer[s["killer_country"]] = by_killer.get(
                s["killer_country"], 0) + c
        if s["owner"]:
            by_owner[s["owner"]] = by_owner.get(s["owner"], 0) + c

    out = {
        "meta": {
            "source_filename": source.name,
            "source_path_redacted": dc.ps._redact(source),
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "notes": {
                "fired_event_names": "persisted list of scripted-event ids "
                "fired this game; recovers WHICH fired, not intra-sample timing.",
                "threat": "world-tension CONTRIBUTOR ledger (per threat-action). "
                "The aggregate world_tension value is not a top-level key here; "
                "scaling inputs are in world.json.",
                "sunk_convoys_history": "persisted per-month convoy-loss ledger "
                "(month is hours-since-epoch/month index as stored).",
            },
            "stats": {
                "fired_events": len(fired),
                "pending_events": len(pending),
                "threat_entries": len(threat),
                "sunk_convoy_entries": len(sunk),
                "total_convoys_sunk": total_convoys,
            },
        },
        "fired_event_names": fired,
        "pending_events": pending,
        "threat": threat,
        "sunk_convoys_history": sunk,
        "convoy_losses_by_killer": dict(
            sorted(by_killer.items(), key=lambda kv: -kv[1])),
        "convoy_losses_by_owner": dict(
            sorted(by_owner.items(), key=lambda kv: -kv[1])),
    }

    lines = ["=== EVENTS / THREAT / CONVOY-LOSS ORACLE VALIDATION ==="]
    lines.append(f"source        : {source.name} (melted={was_melted})")
    lines.append(f"save date     : {meta.get('date')}")
    lines.append(f"fired events  : {len(fired)} (e.g. {fired[:3]})")
    lines.append(f"pending events: {len(pending)}")
    lines.append(f"threat entries: {len(threat)}")
    if threat:
        lines.append(f"  threat[0]   : {threat[0]['label']!r} "
                     f"threat={threat[0]['threat']} tag={threat[0]['tag']}")
    lines.append(f"sunk convoys  : {len(sunk)} entries, "
                 f"{total_convoys} convoys total")
    lines.append(f"  top killer  : {next(iter(out['convoy_losses_by_killer'].items()), None)}")
    lines.append(f"  top victim  : {next(iter(out['convoy_losses_by_owner'].items()), None)}")
    return {"events.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(dc.run_cli(
        "events",
        "the game save -> fired/pending events, threat, sunk-convoy ledgers.",
        build, ["events.json"]))
