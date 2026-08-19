#!/usr/bin/env python3
"""Shared scaffolding for the "deep layer" save-oracle parsers.

The deep-layer modules (parse_focus / parse_tech / parse_production /
parse_economy / parse_diplo / parse_state_detail / parse_combats) all need the
same things:

  - the tolerant Clausewitz tokenizer + melt/discovery helpers from
    ``parse_save`` (REUSED, never reimplemented),
  - a couple of small typed readers ((id,type) block, scalar coercions),
  - identical CLI boilerplate (resolve save, find rakaly, melt, write JSON).

This module collects those so each domain module stays focused on its own
section parsing. It adds nothing to the tokenizer itself.

stdlib only.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import parse_save as ps
from parse_save import (
    Tokenizer,
    LBRACE,
    RBRACE,
    EQUALS,
    atom_value,
    iter_block_entries,
    skip_value,
    _skip_block_body,
    _read_atom_list,
)

__all__ = [
    "ps", "Tokenizer", "LBRACE", "RBRACE", "EQUALS", "atom_value",
    "iter_block_entries", "skip_value", "_skip_block_body", "_read_atom_list",
    "to_int", "to_float", "read_id_block", "read_scalar_list", "collect_block",
    "top_level_pass", "run_cli",
]



def to_float(v):
    try:
        return float(v)
    except (TypeError, ValueError):
        return None


def to_int(v):
    f = to_float(v)
    return int(f) if f is not None else None


def read_id_block(tz: Tokenizer):
    """Read an ``{ id=<n> type=<n> }`` block (LBRACE already consumed).

    Returns ``(id_int, type_int)`` (either may be None). Tolerant of extra keys.
    """
    _id = _type = None
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        val = atom_value(opener)
        if val is None:
            continue
        if key == "id":
            _id = to_int(val)
        elif key == "type":
            _type = to_int(val)
    return _id, _type


def read_scalar_list(tz: Tokenizer):
    """Read a ``{a b c}`` list of bare atoms (LBRACE consumed). Skips nested {}."""
    out = []
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        if opener is None:
            out.append(key)
        else:
            v = atom_value(opener)
            if v is not None:
                out.append(v)
    return out


def collect_block(tz: Tokenizer):
    """Read a flat ``{ k=v k=v ... }`` scalar block into a dict (LBRACE consumed).

    Repeated keys keep the LAST value. Nested blocks are skipped. Bare atoms are
    ignored. Intended for small homogeneous scalar maps (e.g. resource amounts).
    """
    out = {}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        if opener is None:
            continue
        v = atom_value(opener)
        if v is not None:
            out[key] = v
    return out



def top_level_pass(text: str, block_handlers: dict, scalar_keys=None):
    """Stream the implicit top-level block, dispatching depth-0 sections.

    ``block_handlers``: ``{section_key: fn(tz)}`` — called with the tokenizer
    positioned just AFTER the section's ``{``. The handler MUST consume the
    whole block (parse it or ``_skip_block_body``). Sections not in the map are
    skipped.

    ``scalar_keys``: optional iterable of top-level scalar keys to capture; their
    values are returned in the ``meta`` dict.

    Returns ``meta`` = ``{captured scalar key: value, "player_countries": [...]}``.
    """
    scalar_keys = set(scalar_keys or ())
    tz = Tokenizer(text)
    meta = {"player_countries": []}
    while True:
        tok = tz.next()
        if tok is None:
            break
        if tok is RBRACE:
            continue
        if tok is LBRACE:
            _skip_block_body(tz)
            continue
        key = atom_value(tok)
        if key is None:
            continue
        nxt = tz.peek()
        if nxt is not EQUALS:
            continue
        tz.next()
        val = tz.next()
        if val is LBRACE:
            handler = block_handlers.get(key)
            if handler is not None:
                handler(tz)
            elif key == "player_countries":
                meta["player_countries"] = _read_atom_list(tz)
            else:
                _skip_block_body(tz)
        else:
            sval = atom_value(val)
            if sval is not None and key in scalar_keys:
                meta[key] = sval
    return meta



def run_cli(tag: str, description: str, build_fn, out_files):
    """Standard CLI: resolve save -> melt -> build_fn(text, save_path, melted).

    ``build_fn`` returns ``(outputs_dict, report_str)`` where ``outputs_dict`` maps
    a filename (relative to out-dir) -> JSON-serializable object. ``out_files`` is
    only used for the friendly "wrote:" lines; build_fn decides what to emit.

    Returns a process exit code.
    """
    ap = argparse.ArgumentParser(description=description)
    ap.add_argument("save", nargs="?", default=None,
                    help="Path to a .hoi4 save or melted plaintext. "
                         "If omitted, the most recent candidate is used.")
    ap.add_argument("--rakaly", default=None,
                    help="Path to rakaly executable (for binary saves).")
    ap.add_argument("--out-dir", default=None,
                    help="Output directory (default: <script>/output).")
    args = ap.parse_args()

    script_dir = Path(__file__).resolve().parent
    out_dir = Path(args.out_dir) if args.out_dir else script_dir / "output"
    out_dir.mkdir(parents=True, exist_ok=True)

    if args.save:
        save_path = Path(args.save)
        if not save_path.exists():
            print(f"ERROR: save not found: {save_path}", file=sys.stderr)
            return 2
    else:
        save_path = ps.find_latest_save()
        if save_path is None:
            print("ERROR: no candidate save found in known locations.",
                  file=sys.stderr)
            return 2

    print(f"[{tag}] resolved save : {ps._scrub(str(save_path))}")
    rakaly = ps.find_rakaly(args.rakaly)
    print(f"[{tag}] rakaly        : "
          f"{ps._scrub(str(rakaly)) if rakaly else 'NOT FOUND'}")

    try:
        text, was_melted = ps.melt_if_needed(save_path, rakaly, out_dir)
    except RuntimeError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 3

    print(f"[{tag}] melted        : {was_melted}  "
          f"({len(text):,} chars to parse)")

    outputs, report = build_fn(text, save_path, was_melted)

    for fname, obj in outputs.items():
        (out_dir / fname).write_text(
            json.dumps(obj, indent=1, ensure_ascii=False), encoding="utf-8")

    print()
    print(report)
    print()
    for fname in outputs:
        print(f"[{tag}] wrote: {ps._scrub(str(out_dir / fname))}")
    return 0
