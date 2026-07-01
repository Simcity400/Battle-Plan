#!/usr/bin/env python3
"""the game save -> per-country NATIONAL FOCUS oracle (``focus.json``).

Deep-layer extension of the save oracle. Per country it extracts:
  - ``focus_tree``                : the active focus tree id (e.g. "german_focus")
  - ``continuous_focus_palette``  : continuous-focus palette id (if any)
  - ``completed``                 : list of completed focus ids, IN SAVE ORDER
                                    (the engine appends each as it completes, so
                                    list order == completion order)
  - ``completed_count``           : len(completed)
  - ``current``                   : focus currently being worked on (or null)
  - ``progress``                  : whole-focus progress counter on the current
                                    focus (engine stores an int "days" style
                                    counter, NOT a fraction; reported as-is)
  - ``paused``                    : bool

Key paths (verified against a real 1.19 melt):
  countries.<TAG>.focus_tree                = "german_focus"
  countries.<TAG>.continuous_focus_palette  = "generic_focus"
  countries.<TAG>.focus = {
      activate_shine_on_focus = { "..." }   # ignored
      completed = "GER_..."                 # REPEATED, order preserved
      progress  = 12
      current   = "GER_the_swiss_gold"
      paused    = no
  }

Reality note: the save stores the per-focus progress only for the CURRENT focus
(as the ``progress`` scalar inside ``focus``). There is no per-completed-focus
timestamp in the save, so completion DATES cannot be recovered — only the order.

stdlib only. Reuses the shared tokenizer via ``deep_common``.
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, atom_value, iter_block_entries, _skip_block_body, to_int,
)


def _parse_focus_inner(tz, rec):
    """Parse one ``focus={ ... }`` block (LBRACE consumed)."""
    completed = []
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            # activate_shine_on_focus={...} and any other nested list -> skip
            _skip_block_body(tz)
            continue
        val = atom_value(opener)
        if val is None:
            continue
        if key == "completed":
            completed.append(val)
        elif key == "current":
            rec["current"] = val
        elif key == "progress":
            rec["progress"] = to_int(val)
        elif key == "paused":
            rec["paused"] = (val == "yes")
    rec["completed"] = completed
    rec["completed_count"] = len(completed)


def _parse_countries(tz, by_tag):
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        rec = {
            "focus_tree": None,
            "continuous_focus_palette": None,
            "completed": [],
            "completed_count": 0,
            "current": None,
            "progress": None,
            "paused": None,
        }
        for key, op in iter_block_entries(tz):
            if op is LBRACE:
                if key == "focus":
                    _parse_focus_inner(tz, rec)
                else:
                    _skip_block_body(tz)
                continue
            val = atom_value(op)
            if val is None:
                continue
            if key == "focus_tree":
                rec["focus_tree"] = val
            elif key == "continuous_focus_palette":
                rec["continuous_focus_palette"] = val
        # Only keep countries that actually have a focus tree or completed any.
        if rec["focus_tree"] or rec["completed"] or rec["current"]:
            by_tag[tag] = rec


def parse_focus_text(text: str):
    by_tag: dict = {}
    meta = dc.top_level_pass(
        text,
        {"countries": lambda tz: _parse_countries(tz, by_tag)},
        scalar_keys=("date", "version", "player"),
    )
    return by_tag, meta


def build(text, source, was_melted):
    by_tag, meta = parse_focus_text(text)

    total_completed = sum(r["completed_count"] for r in by_tag.values())
    with_current = sum(1 for r in by_tag.values() if r["current"])
    out = {
        "meta": {
            "source_filename": source.name,
            "source_path_redacted": dc.ps._redact(source),
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "stats": {
                "countries_with_focus": len(by_tag),
                "total_completed_focuses": total_completed,
                "countries_currently_focusing": with_current,
            },
        },
        "countries": dict(sorted(by_tag.items())),
    }

    ger = by_tag.get("GER", {})
    lines = ["=== FOCUS ORACLE VALIDATION ==="]
    lines.append(f"source        : {source.name} (melted={was_melted})")
    lines.append(f"save date     : {meta.get('date')}  "
                 f"version={meta.get('version')}")
    lines.append(f"countries w/ focus : {len(by_tag)}")
    lines.append(f"total completed    : {total_completed}")
    lines.append(f"currently focusing : {with_current}")
    lines.append(f"GER focus_tree     : {ger.get('focus_tree')}")
    lines.append(f"GER completed      : {ger.get('completed_count')} "
                 f"(current={ger.get('current')!r} progress={ger.get('progress')})")
    if ger.get("completed"):
        lines.append(f"GER first completed: {ger['completed'][0]}")
        lines.append(f"GER last completed : {ger['completed'][-1]}")
    return {"focus.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(dc.run_cli(
        "focus", "the game save -> per-country national focus oracle.",
        build, ["focus.json"]))
