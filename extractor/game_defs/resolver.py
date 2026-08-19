#!/usr/bin/env python3
"""Mod-over-vanilla resolution + localisation loader for the static-def parsers.

This is the shared spine of every ``extractor/game_defs/`` parser (bucket-(c) of the
binding-sweep gap ledger). It does ONE job: hand each parser the *effective*
mod-over-vanilla file set for a ``common/`` (or ``events/``) subdir, plus a
localisation lookup, REUSING the already-audited resolver in
``replay_interface/game_assets/extract.py`` rather than re-implementing Clausewitz load order.

Why reuse, not reinvent:
  * ``extract.py`` already encodes the two Clausewitz override rules correctly
    (file-level override by case-folded relpath, later layer wins; ``replace_path``
    replaces ONE exact dir non-recursively) and is exercised by the existing GUI/
    sprite/flag/loc extraction. The focus-tree / GUI parser (S3.1) resolves the
    same way. We bind to it so the static-def parsers can never drift from it.

Two resolution models, deliberately distinct (verified vs the engine):
  * **FILE set** (``effective_files``): for ``common/*`` and ``events/*`` the unit
    of override is the FILE (a mod file with the same relpath/stem replaces the
    vanilla one; a mod file with a new stem just adds). This is exactly
    ``build_file_map_layers``.
  * **LOC keys** (``LocDB``): localisation override is by *KEY across ALL .yml*,
    not by file. A key defined in any mod yml wins over the same key in any
    vanilla yml regardless of filename. So we load EVERY effective loc file in
    load order and let later-loaded keys win. (``resolve_loc`` already returns the
    effective file set; we add the key-merge on top.)

Vanilla path comes from ``BATTLE_PLAN_VANILLA`` (env) or ``--vanilla``; the active
mod-set from ``replay_interface/game_assets/active_modset.json`` (the committed, path-redacted
manifest) re-resolved against the live user-data dir, OR explicit ``--mods``.
No personal/home path is hardcoded here.

stdlib only (+ the in-repo ``replay_interface/game_assets`` modules).
"""

from __future__ import annotations

import os
import re
import sys
from pathlib import Path

_HERE = Path(__file__).resolve().parent
_EXTRACTOR_ROOT = _HERE.parent
_REPO_ROOT = _EXTRACTOR_ROOT.parent
_GAME_ASSETS = _REPO_ROOT / "replay_interface" / "game_assets"
for _p in (str(_GAME_ASSETS), str(_HERE)):
    if _p not in sys.path:
        sys.path.insert(0, _p)

from paradox_parser import parse, Node
import extract as _ex


DEFAULT_USERDATA = _REPO_ROOT.parent.parent.resolve()
_DEFAULT_MODSET = _GAME_ASSETS / "active_modset.json"


def vanilla_root(explicit: str | None = None) -> Path:
    """Resolve the vanilla the game install dir. Priority: explicit arg >
    BATTLE_PLAN_VANILLA env > extract.py's documented default. Never a baked home path."""
    cand = explicit or os.environ.get("BATTLE_PLAN_VANILLA", os.environ.get("HOI4_VANILLA")) or _ex.DEFAULT_VANILLA
    p = Path(cand)
    if not p.is_dir():
        raise FileNotFoundError(
            f"vanilla the game install not found: {p}\n"
            f"Set BATTLE_PLAN_VANILLA or pass vanilla=... (no home path is baked in)."
        )
    return p.resolve()


def _modfiles_from_manifest(manifest_path: Path, userdata: Path) -> list[Path]:
    """Recover the ordered list of .mod descriptor paths from the committed,
    path-redacted active_modset.json (descriptors stored RELATIVE to user-data)."""
    import json
    data = json.loads(Path(manifest_path).read_text(encoding="utf-8"))
    out: list[Path] = []
    for entry in data.get("load_order", []):
        if entry.get("kind") != "mod":
            continue
        rel = entry.get("descriptor")
        if not rel:
            continue
        out.append((userdata / rel).resolve())
    return out


def build_layers(vanilla: str | None = None,
                 mods: list[str] | None = None,
                 modset_json: str | Path | None = None,
                 userdata: str | Path | None = None,
                 strict_mods: bool = False) -> list:
    """Return the ordered Layer list [vanilla, mod1, ...] for the active set.

    * ``mods``  : explicit ordered list of .mod descriptor paths/tokens (wins).
    * else read ``modset_json`` (default ``replay_interface/game_assets/active_modset.json``) to
      recover the launcher-resolved mod set (path-redacted -> re-resolved here).
    * ``userdata`` overrides the game user-data dir for descriptor resolution.

    Reuses ``extract.build_modset_layers`` so replace_path / load order match the
    rest of the app exactly.
    """
    v = vanilla_root(vanilla)
    ud = Path(userdata).resolve() if userdata else DEFAULT_USERDATA

    mod_files: list[Path] = []
    if mods is not None:
        for tok in mods:
            tok = tok.strip().replace("\\", "/")
            p = Path(tok)
            if not p.is_absolute():
                p = ud / p
            mod_files.append(p)
    else:
        mj = Path(modset_json) if modset_json else _DEFAULT_MODSET
        if mj.is_file():
            mod_files = _modfiles_from_manifest(mj, ud)

    layers, warnings = _ex.build_modset_layers(v, mod_files, ud)
    if strict_mods and (warnings or len(layers) != 1 + len(mod_files)):
        detail = "; ".join(str(warning) for warning in warnings) or "layer count mismatch"
        raise RuntimeError(f"strict mod resolution failed: {detail}")
    for w in warnings:
        print(f"[resolver] WARN: {w}", file=sys.stderr)
    return layers


def effective_files(layers: list, subdir: str, exts=(".txt",)) -> dict:
    """{relpath: abs Path} effective mod-over-vanilla set for ``subdir``.

    Thin pass-through to ``extract.build_file_map_layers`` (the audited rule:
    file-level override by relpath, later wins; replace_path replaces one exact
    dir). ``exts`` are lowercase suffixes.
    """
    return _ex.build_file_map_layers(layers, subdir, tuple(e.lower() for e in exts))


def read_text(path: Path) -> str:
    """Read a Clausewitz/loc file tolerantly. the game ships UTF-8 (often with BOM)
    but some legacy files are latin-1; decode utf-8-sig first, fall back to
    latin-1 so we never crash on a stray byte."""
    raw = Path(path).read_bytes()
    try:
        return raw.decode("utf-8-sig")
    except UnicodeDecodeError:
        return raw.decode("latin-1")


def parse_file(path: Path) -> Node:
    """Parse one Clausewitz file into a root Node (shared tolerant parser)."""
    return parse(read_text(path))


def _layer_prio_fn(layers):
    """Return prio(abs_path)->int using PATH-BOUNDARY-SAFE containment (Codex #6:
    ``startswith`` is wrong if a mod root is nested under / shares a prefix with
    the vanilla root). We test ``relative_to`` and pick the DEEPEST matching root
    (longest), so a mod under vanilla still maps to the mod layer, not vanilla."""
    roots = [(Path(ly.root).resolve(), i) for i, ly in enumerate(layers)]

    def prio(abs_path):
        ap = Path(abs_path).resolve()
        best_i, best_len = 0, -1
        for root, i in roots:
            try:
                ap.relative_to(root)
            except ValueError:
                continue
            depth = len(root.parts)
            if depth > best_len:
                best_len, best_i = depth, i
        return best_i
    return prio


def ordered_files(layers, subdir, exts=(".txt",)):
    """Effective {relpath: abs Path} for ``subdir``, returned as a LIST of
    (relpath, abs Path) sorted vanilla-first then mod (by path-boundary-safe layer
    priority) then relpath — the deterministic load order every parser needs."""
    fm = effective_files(layers, subdir, exts)
    prio = _layer_prio_fn(layers)
    return sorted(fm.items(), key=lambda kv: (prio(kv[1]), kv[0]))


_LOC_LINE = re.compile(
    r'^\s*'
    r'([^\s:#"]+)'
    r':\s*'
    r'(\d+)?'
    r'\s*'
    r'"(.*)"'
    r'\s*(?:#.*)?$'
)


class LocDB:
    """Effective localisation key->value table (mod-over-vanilla, key-level).

    ``get(key)`` returns the raw loc string (color codes / $VAR$ / [scope] kept
    verbatim — resolving those is a runtime job, not a static-parse job).
    ``resolve(key)`` additionally strips §color codes and yields plain text for a
    quick human-readable sample; both are provided so the app can choose."""

    __slots__ = ("by_key", "source_count", "files")

    def __init__(self):
        self.by_key: dict[str, str] = {}
        self.source_count = 0
        self.files: list[str] = []

    def __len__(self):
        return len(self.by_key)

    def __contains__(self, key):
        return key in self.by_key

    def get(self, key: str, default=None):
        """Raw loc value (verbatim, markup preserved) or default."""
        if key is None:
            return default
        return self.by_key.get(key, default)

    _COLOR = re.compile(r"§.|\x15.")

    def resolve(self, key: str, default=None):
        v = self.by_key.get(key)
        if v is None:
            return default
        return self._COLOR.sub("", v)

    def add_line(self, line: str):
        m = _LOC_LINE.match(line)
        if not m:
            return False
        key, _ver, val = m.group(1), m.group(2), m.group(3)
        self.by_key[key] = val
        return True


def load_loc(layers: list, language: str = "english") -> LocDB:
    """Build a LocDB by loading EVERY effective loc file IN LOAD ORDER and letting
    later-loaded keys win (KEY-level override, not file-level).

    ``resolve_loc`` returns the effective FILE set already mod-over-vanilla at the
    file granularity; but the engine also overrides at KEY granularity across
    files, so within that file set we still apply last-write-wins per key. Because
    ``build_file_map_layers`` keeps mod files (loaded after vanilla) and the dict
    preserves insertion order roughly by layer, we additionally sort so vanilla
    files are processed before mod files, guaranteeing the mod key wins on a tie.
    """
    file_map = _ex.resolve_loc(layers, language)
    prio = _layer_prio_fn(layers)

    db = LocDB()
    for rel, abspath in sorted(file_map.items(), key=lambda kv: (prio(kv[1]), kv[0])):
        text = read_text(abspath)
        added = 0
        for line in text.splitlines():
            s = line.strip()
            if not s or s.startswith("#") or s.endswith(":") and '"' not in s:
                continue
            if db.add_line(line):
                added += 1
        if added:
            db.source_count += 1
            db.files.append(rel)
    return db


if __name__ == "__main__":
    layers = build_layers()
    print("layers:", [ly.label for ly in layers])
    loc = load_loc(layers)
    print(f"loc keys: {len(loc)} from {loc.source_count} files")
    for k in ("LOADING_TIP_0",):
        print(f"  {k} = {loc.resolve(k)!r}")
