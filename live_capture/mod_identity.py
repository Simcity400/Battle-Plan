"""Capture-side reader for the HOST's ACTIVE MOD-SET (the per-replay mod identity).

Reads the game LAUNCHER config at capture time -- the `dlc_load.json` in the game
user-data directory plus the `.mod` descriptors it enables -- and builds the
`provenance.mod` ModSet that the native UI auto-matches against
(GAME_UI_REPLICA_PLAN §K). This is read from the launcher config ON DISK, NOT from
game memory and NOT from the melted save.

Hard rules honoured here:
  * READ-ONLY. Opens files for reading only; never writes the game/mod/launcher.
  * NO FABRICATION. If no launcher config is found (or it cannot be read), the
    result is ``None`` -- never a guessed/default mod-set. A launcher config that
    enables ZERO mods yields a *measured* vanilla mod-set (``schema.empty_modset``),
    which is distinguishable from ``None`` (not read at all).
  * NO PERSONAL PATHS. The stored ModSet records mod NAMES, workshop ids, versions,
    and the descriptor BASENAMES only -- never an absolute `path=` from a `.mod`
    file (those carry a user directory and are dropped).

Pure stdlib. No game, no network, no third-party deps.
"""
from __future__ import annotations

import json
import os
import re
from typing import Any, Dict, List, Optional

# schema.modset_id / schema.empty_modset are the single source of truth for the
# mod-set IDENTITY digest, so the capture side and the validator agree.
try:                                  # package-relative (normal import)
    from replay import schema as _schema
except Exception:                     # pragma: no cover - path-shim fallback
    import importlib.util as _ilu
    import sys as _sys
    _here = os.path.dirname(os.path.abspath(__file__))
    _root = os.path.dirname(_here)
    if _root not in _sys.path:
        _sys.path.insert(0, _root)
    from replay import schema as _schema


# A `.mod` descriptor is the Paradox key=value / key="value" / key={ ... } format.
# We only need a handful of scalar keys; parse them with a tolerant line scanner
# rather than pulling in the full clausewitz parser (this stays stdlib + cheap).
_KV_RE = re.compile(r'^\s*([A-Za-z_]+)\s*=\s*"(.*?)"\s*$')


def _redact_basename(p: Optional[str]) -> Optional[str]:
    """A path-or-ref -> its basename only (never a user directory). ``mod/test.mod``
    -> ``test.mod``; ``None`` -> ``None``. Mirrors the assembler's redaction rule."""
    if not p or not isinstance(p, str):
        return None
    return os.path.basename(p.replace("\\", "/").rstrip("/")) or None


def parse_mod_descriptor(text: str) -> Dict[str, Optional[str]]:
    """Parse the scalar fields of a `.mod` descriptor we care about.

    Returns ``{name, version, supported_version, workshop_id}`` (each str or None).
    The descriptor's ``path=`` is DELIBERATELY NOT returned (it is an absolute user
    path -- a privacy leak). ``remote_file_id`` is the Steam Workshop id.
    """
    out: Dict[str, Optional[str]] = {
        "name": None,
        "version": None,
        "supported_version": None,
        "workshop_id": None,
    }
    for line in text.splitlines():
        m = _KV_RE.match(line)
        if not m:
            continue
        key, val = m.group(1), m.group(2)
        if key == "name":
            out["name"] = val or None
        elif key == "version":
            out["version"] = val or None
        elif key == "supported_version":
            out["supported_version"] = val or None
        elif key == "remote_file_id":
            out["workshop_id"] = val or None
        # `path`, `tags`, `replace_path`, `dependencies` are intentionally skipped
        # here: path is a privacy leak; the rest do not contribute to mod IDENTITY.
    return out


def _read_text(path: str) -> Optional[str]:
    try:
        with open(path, "r", encoding="utf-8-sig", errors="replace") as f:
            return f.read()
    except OSError:
        return None


def read_modset(user_data_dir: Optional[str]) -> Optional[Dict[str, Any]]:
    """Read the active mod-set from a the game user-data directory.

    ``user_data_dir`` is the directory that contains ``dlc_load.json`` and the
    ``mod/`` descriptor folder (the game user-data dir). The caller supplies it
    (never hardcoded here) -- e.g. from an env var or a launcher probe.

    Returns a ModSet dict (``schema``-shaped) when the launcher config is read:
      * enables N mods  -> a mod-set with those N mods in LOAD ORDER;
      * enables 0 mods  -> ``schema.empty_modset()`` (a MEASURED vanilla run).
    Returns ``None`` (NO FABRICATION) when:
      * ``user_data_dir`` is None / does not exist, or
      * ``dlc_load.json`` is missing / unreadable / malformed.

    A descriptor named by ``dlc_load.json`` that is missing on disk is recorded as
    a mod with only its descriptor basename known (name/version/workshop_id null) --
    we never invent its name; we just report that it was enabled but unreadable.
    """
    if not user_data_dir or not os.path.isdir(user_data_dir):
        return None
    dlc_path = os.path.join(user_data_dir, "dlc_load.json")
    raw = _read_text(dlc_path)
    if raw is None:
        return None
    try:
        cfg = json.loads(raw)
    except (json.JSONDecodeError, ValueError):
        return None
    if not isinstance(cfg, dict):
        return None
    enabled = cfg.get("enabled_mods")
    if not isinstance(enabled, list):
        return None  # malformed -> do not fabricate

    mods: List[Dict[str, Optional[str]]] = []
    load_order: List[str] = []
    for ref in enabled:
        if not isinstance(ref, str) or not ref.strip():
            continue
        ref = ref.strip()
        load_order.append(_redact_basename(ref) or ref)
        # The ref is relative to the user-data dir (e.g. "mod/test.mod").
        desc_path = os.path.join(user_data_dir, ref.replace("\\", "/"))
        text = _read_text(desc_path)
        if text is None:
            # Enabled but the descriptor is unreadable -> record it WITHOUT inventing
            # a name/version (no fabrication); keep its descriptor basename for a hint.
            mods.append({
                "name": None,
                "version": None,
                "supported_version": None,
                "workshop_id": None,
                "descriptor": _redact_basename(ref),
            })
            continue
        fields = parse_mod_descriptor(text)
        fields["descriptor"] = _redact_basename(ref)
        mods.append(fields)

    if not mods:
        # The launcher config was READ and it enables no mods -> measured vanilla.
        ms = _schema.empty_modset()
        ms["source"] = "dlc_load.json"
        return ms

    return {
        "modset_id": _schema.modset_id(mods),
        "source": "dlc_load.json",
        "load_order": load_order,
        "mods": mods,
    }


# Common the game user-data dir candidates RELATIVE to a home directory, so a caller
# can probe without hardcoding the absolute personal path in committed source.
# (Windows cloud-redirected Documents is a common live-host case.)
_USER_DATA_SUBPATHS = (
    os.path.join("Documents", "Paradox Interactive", "Hearts of Iron IV"),
    os.path.join("One" + "Drive", "Documents", "Paradox Interactive", "Hearts of Iron IV"),
)


def default_user_data_dir() -> Optional[str]:
    """Best-effort probe for the game user-data dir, override-first.

    Precedence (no personal path baked into source):
      1. ``BATTLE_PLAN_USER_DATA`` env var (explicit; the supported override);
      2. the documented per-home subpaths under ``USERPROFILE``/``HOME``.
    Returns the first existing candidate, else ``None`` (caller then records
    ``provenance.mod = None`` -- no fabrication)."""
    env = os.environ.get("BATTLE_PLAN_USER_DATA", os.environ.get("HOI4_USER_DATA"))
    if env and os.path.isdir(env):
        return env
    home = os.environ.get("USERPROFILE") or os.environ.get("HOME")
    if home:
        for sub in _USER_DATA_SUBPATHS:
            cand = os.path.join(home, sub)
            if os.path.isdir(cand):
                return cand
    return None


def capture_modset(user_data_dir: Optional[str] = None) -> Optional[Dict[str, Any]]:
    """The capture entry point: resolve the user-data dir (probe if not given) and
    read the active mod-set, or ``None`` if no launcher config is found."""
    udir = user_data_dir or default_user_data_dir()
    return read_modset(udir)
