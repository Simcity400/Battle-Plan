#!/usr/bin/env python3
"""LOCALISATION static-def parser (R3: token -> localised-name join).

Builds the deterministic ``{key: {name, raw, source_layer, source_file}}`` map
the replay UI needs to render human-readable labels for the TOKEN ids the
compact domains emit (focus ids, idea/law tokens, tech names, decision ids,
national-spirit tokens, equipment/unit ids, state names, character tokens,
country TAGs — all of which are, ultimately, localisation keys or use a loc
key as their display name).

This module is DELIBERATELY separate from ``resolver.LocDB`` even though it
reuses the same override semantics, because the replay UI needs three things
``LocDB`` does not expose:

  * ``source_layer`` per key (did the label come from vanilla or a named mod
    layer?) — the coverage/override reporting R3 asks for.
  * a CLEAN name (color codes + surrounding quotes stripped) alongside the
    RAW value (verbatim, ``$VAR$``/``[Scope.Func]`` kept, per spec: static
    parsing does not resolve interpolation, it only notes it).
  * a stable, gate-able JSON artifact under ``game_defs/out/`` that the
    domain-59-style pack/verify gate can hash and pack byte-for-byte.

Override model: KEY-level merge across ALL effective ``localisation/english``
(and ``localisation/english/replace``) ``*.yml`` files, later-loaded layer
wins — same rule ``resolver.load_loc`` uses, re-derived here so we can also
capture provenance (``source_layer``) per key, which ``LocDB`` intentionally
does not track (it only needs the winning value).

No fabrication: a key absent from every effective file is simply absent from
the output dict — never guessed, never defaulted to the raw token.

stdlib + resolver only.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

import resolver as R

_LOC_LINE = R._LOC_LINE

_COLOR = re.compile(r"§.|\x15.")

_VAR_TOKEN = re.compile(r"\$[A-Za-z0-9_.]+\$")
_SCOPE_TOKEN = re.compile(r"\[[^\[\]]*\]")


def _strip_wrapping_quotes(value: str) -> str:
    """The grammar's capture group already excludes the outer quote pair, but
    a value can itself start/end with an ESCAPED quote (``\\"``) that some
    loc lines leave in place; normalise those too so the clean name never
    carries a stray backslash-quote artifact."""
    v = value
    if v.startswith('\\"'):
        v = v[2:]
    if v.endswith('\\"'):
        v = v[:-2]
    return v


def clean_name(raw: str) -> str:
    """Strip §color codes and normalise stray escaped-quote wrapping, leaving
    ``$VAR$`` / ``[Scope.Func]`` interpolation tokens untouched (resolving
    those is a runtime job per the R3 spec, not a static-parse job)."""
    return _COLOR.sub("", _strip_wrapping_quotes(raw))


def has_interpolation(raw: str) -> bool:
    return bool(_VAR_TOKEN.search(raw) or _SCOPE_TOKEN.search(raw))


def _iter_loc_lines(text: str):
    """Yield (key, raw_value) for every well-formed ``KEY:VER "value"`` line,
    skipping the ``l_<lang>:`` header, blank lines, and full-line comments —
    mirrors ``resolver.load_loc``'s line-admission filter exactly."""
    for line in text.splitlines():
        s = line.strip()
        if not s or s.startswith("#"):
            continue
        if s.endswith(":") and '"' not in s:
            continue
        m = _LOC_LINE.match(line)
        if not m:
            continue
        key, _ver, raw = m.group(1), m.group(2), m.group(3)
        yield key, raw


def parse_localisation(layers, language: str = "english") -> dict:
    """Build the effective token -> {name, raw, source_layer, source_file}
    map across ``localisation/<language>`` (mod-over-vanilla, KEY-level
    override, later-loaded layer wins — matches the engine and
    ``resolver.load_loc``).

    Returns the same shape ``build_all.py``'s other parsers use: a ``meta``
    block (counts + per-layer override tally) alongside the keyed catalog.
    """
    file_map = R._ex.resolve_loc(layers, language)
    prio = R._layer_prio_fn(layers)
    layer_of = {i: ly.label for i, ly in enumerate(layers)}

    ordered = sorted(file_map.items(), key=lambda kv: (prio(kv[1]), kv[0]))

    keys: dict[str, dict] = {}
    files_read = 0
    lines_seen = 0
    overrides = 0
    by_layer_new: dict[str, int] = {}
    by_layer_override: dict[str, int] = {}

    for rel, abspath in ordered:
        label = layer_of.get(prio(abspath), "?")
        try:
            text = R.read_text(abspath)
        except Exception as e:
            print(f"[localisation] WARN {rel}: {e}", file=sys.stderr)
            continue
        added_here = 0
        for key, raw in _iter_loc_lines(text):
            lines_seen += 1
            added_here += 1
            entry = {
                "name": clean_name(raw),
                "raw": raw,
                "has_interpolation": has_interpolation(raw),
                "source_layer": label,
                "source_file": rel,
            }
            if key in keys:
                overrides += 1
                by_layer_override[label] = by_layer_override.get(label, 0) + 1
            else:
                by_layer_new[label] = by_layer_new.get(label, 0) + 1
            keys[key] = entry
        if added_here:
            files_read += 1

    return {
        "meta": {
            "language": language,
            "key_count": len(keys),
            "files_read": files_read,
            "files_total": len(ordered),
            "lines_seen": lines_seen,
            "overrides": overrides,
            "by_layer_new_keys": dict(sorted(by_layer_new.items())),
            "by_layer_override_keys": dict(sorted(by_layer_override.items())),
            "layers": [ly.label for ly in layers],
        },
        "keys": dict(sorted(keys.items())),
    }


def build(vanilla=None, mods=None, language: str = "english") -> dict:
    layers = R.build_layers(vanilla=vanilla, mods=mods)
    return parse_localisation(layers, language=language)


if __name__ == "__main__":
    out = build()
    outp = Path(__file__).resolve().parent / "out" / "localisation.json"
    outp.parent.mkdir(exist_ok=True)
    outp.write_text(json.dumps(out, ensure_ascii=False, indent=1), encoding="utf-8")
    m = out["meta"]
    print(f"localisation: {m['key_count']} keys from {m['files_read']}/{m['files_total']} files "
          f"({m['overrides']} mod-over-vanilla overrides) -> {outp.name}")
    for label, n in m["by_layer_new_keys"].items():
        ov = m["by_layer_override_keys"].get(label, 0)
        print(f"  {label:20s}: {n:6d} new keys, {ov:6d} overrides")
