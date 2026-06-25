"""registry.py -- the modular READER-PLUG registry.

Each cracked memory domain is an optional *plug*. A plug owns:

  * a stable ``name`` (the provenance domain id it contributes),
  * an import-guarded ``available`` flag (the underlying reader module imported
    cleanly -> True; an ImportError -> the plug is permanently unavailable this
    run, NO fabrication),
  * a ``read(ctx) -> dict`` that returns a small, plain-data result for the
    assembler to fold into a snapshot.

The capture loop (capture.py) calls ``read_all(ctx)``; the assembler
(assembler.py) consumes the returned ``DomainResult``s. The contract is strict
and honest:

  * A plug whose module did not import -> NOT run; its domain is absent from
    ``provenance.memory.domains``; every field it would have filled stays null.
  * A plug that RAISES at read time -> its domain is absent from provenance and
    its (partial) data is DISCARDED (transactional: the assembler only ever sees
    a result that was produced without raising). The error is recorded in the
    result for diagnostics but never reaches the snapshot as data.
  * A plug that returns successfully -> its domain IS listed in provenance, even
    if the data is empty (measured-absent: e.g. "no active combats" is a real
    observation, distinct from "combats not measured").

This is the single choke point that enforces DESIGN no-fabrication: a domain is
"captured" iff a plug ran to completion without raising.

The naval/air COMBAT reader and the weather reader are being cracked by other
agents right now; their plugs import-guard their modules and simply report
unavailable until those land -- no edits to memory_reader/ are required here.

Pure orchestration: this module reads nothing from memory itself. It imports the
reader modules lazily inside each factory so a missing/broken reader can never
break the import of live_capture.
"""
from __future__ import annotations

import os
import sys
import traceback
from dataclasses import dataclass, field
from typing import Any, Callable, Dict, List, Optional

# Make the repo root importable so `import winmem`, `from live_layers import ...`,
# etc. resolve the same way the readers expect when run in-tree.
_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir))
_MR = os.path.join(_REPO, "memory_reader")
for _p in (_REPO, _MR):
    if _p not in sys.path:
        sys.path.insert(0, _p)


@dataclass
class CaptureContext:
    """Everything a plug needs to read once. Built fresh per capture by the loop.

    ``proc`` is an open winmem.Process (shared by the winmem-based plugs).
    ``tag_table`` / ``bridge`` / ``province_control`` are the already-resolved
    live-layer objects (resolved once per capture and reused). Any may be None if
    its resolve() failed -- a plug that needs one and finds it None must raise (so
    its domain is honestly omitted), NOT fabricate.
    """

    proc: Any = None
    identity: Any = None              # ProcessIdentity
    tag_table: Any = None
    bridge: Any = None
    province_control: Any = None
    extras: Dict[str, Any] = field(default_factory=dict)

    def resolve_tag(self, tag_db_id: Optional[int]) -> Optional[str]:
        """tag_db_id -> TAG via the live tag table, or None. NEVER returns a
        numeric id (schema forbids a numeric id in a *_tag field)."""
        if self.tag_table is None or tag_db_id is None:
            return None
        try:
            t = self.tag_table.tag(int(tag_db_id))
        except Exception:
            return None
        if not t or t == "---":
            return None
        return t


@dataclass
class DomainResult:
    """The outcome of running one plug for one capture."""

    name: str
    available: bool                 # the plug's module imported
    captured: bool                  # read() completed without raising
    data: Optional[Dict[str, Any]] = None
    error: Optional[str] = None     # diagnostic only; never enters the snapshot

    @property
    def domain_captured(self) -> bool:
        return self.available and self.captured


class ReaderPlug:
    """One optional domain reader.

    ``factory`` lazily imports + returns the underlying reader callable (or raises
    ImportError). ``reader`` is ``(ctx, impl) -> dict`` doing the actual read.
    """

    def __init__(self, name: str,
                 factory: Callable[[], Any],
                 reader: Callable[[CaptureContext, Any], Dict[str, Any]]):
        self.name = name
        self._factory = factory
        self._reader = reader
        self._impl: Any = None
        self._import_error: Optional[str] = None
        self._available: Optional[bool] = None

    @property
    def available(self) -> bool:
        """True iff the underlying reader module imported. Cached; import is tried
        once. An ImportError (or any import-time error) -> permanently False."""
        if self._available is None:
            try:
                self._impl = self._factory()
                self._available = True
            except Exception as exc:  # ImportError + anything raised at import
                self._impl = None
                self._available = False
                self._import_error = "%s: %s" % (type(exc).__name__, exc)
        return self._available

    def read(self, ctx: CaptureContext) -> DomainResult:
        """Run the plug transactionally: build the domain's data in a LOCAL dict
        and only return it on success. Any exception -> captured=False, data
        discarded (the assembler never folds a partially-built domain)."""
        if not self.available:
            return DomainResult(self.name, available=False, captured=False,
                                error=self._import_error)
        try:
            data = self._reader(ctx, self._impl)
            if data is None:
                data = {}
            return DomainResult(self.name, available=True, captured=True,
                                data=data)
        except Exception:
            return DomainResult(self.name, available=True, captured=False,
                                data=None, error=traceback.format_exc(limit=6))


# ---------------------------------------------------------------------------
# Plug factories + readers. Each factory imports its module lazily; each reader
# returns plain JSON-able data keyed exactly as the assembler expects.
# ---------------------------------------------------------------------------
def _f_state_control():
    from live_layers import province_control as pc  # noqa
    return pc


def _r_state_control(ctx: CaptureContext, pc) -> Dict[str, Any]:
    """{state_id: TAG} controller map from the live state-controller crack.

    Folds tag-db-id -> TAG via the tag table. An id that won't resolve is kept as
    a state with controller=None (measured, un-nameable) rather than dropped --
    the state still exists (Codex D). The raw count of unresolved ids is reported
    so provenance/diagnostics can see it without fabricating a TAG.
    """
    if ctx.tag_table is None:
        raise RuntimeError("state_control needs the tag table (unavailable)")
    raw = pc.enumerate_state_controllers(ctx.proc)   # {state_id: ctrl_tag_db_id}
    controllers: Dict[str, Optional[str]] = {}
    unresolved = 0
    for sid, ctrl_id in raw.items():
        tag = ctx.resolve_tag(ctrl_id) if ctrl_id else None
        if ctrl_id and tag is None:
            unresolved += 1
        controllers[str(int(sid))] = tag
    return {"controllers": controllers, "count": len(controllers),
            "unresolved_ids": unresolved}


def _f_province_control():
    from live_layers import province_control as pc  # noqa
    return pc


def _r_province_control(ctx: CaptureContext, pc) -> Dict[str, Any]:
    """{province_id: TAG|None} from the resolved ProvinceControl layer."""
    layer = ctx.province_control
    if layer is None:
        raise RuntimeError("province control layer not resolved")
    pmap = layer.as_map()    # {province_id: TAG_or_None}
    out = {str(int(pid)): tag for pid, tag in pmap.items()}
    nn = sum(1 for v in out.values() if v)
    return {"controllers": out, "count": len(out), "with_controller": nn}


def _f_divisions():
    from division_live.division_live_reader import DivisionLiveReader  # noqa
    return DivisionLiveReader


def _r_divisions(ctx: CaptureContext, DivisionLiveReader) -> Dict[str, Any]:
    """List of division dicts. Owner resolved to TAG via the tag table; an
    unresolved owner keeps the division with owner=None (Codex D), never dropped.
    Opens + closes its own reader handle (the division reader has its own handle
    class)."""
    if ctx.tag_table is None:
        raise RuntimeError("divisions need the tag table for owner resolution")
    reader = DivisionLiveReader()
    try:
        # verify it attached to OUR process instance (Codex G: same-process check)
        _verify_same_process(ctx, reader)
        rows = reader.enumerate_divisions(resolve_owner_tag=ctx.tag_table.tag)
    finally:
        try:
            reader.close()
        except Exception:
            pass
    out = []
    unresolved_owner = 0
    for r in rows:
        owner = r.get("owner")
        if owner == "---" or not owner:
            owner = None
        if owner is None and r.get("owner_tag_db_id"):
            unresolved_owner += 1
        out.append({
            "div_id": r.get("id"),
            "owner": owner,
            "location_province": r.get("location"),
            "moving": r.get("moving"),
            "move_target_province": r.get("move_target_province"),
            "move_progress": r.get("movement_progress"),
            "in_combat": r.get("in_combat"),
            "organisation": r.get("organisation"),
            "strength": r.get("strength"),
            "experience": r.get("experience"),
            "supply_status": r.get("army_current_supply_ratio"),
        })
    return {"divisions": out, "count": len(out),
            "unresolved_owner": unresolved_owner}


def _f_country_scalars():
    from live_layers import country_index_bridge as cib  # noqa
    import locate  # noqa
    return (cib, locate)


def _r_country_scalars(ctx: CaptureContext, impl) -> Dict[str, Any]:
    """Per-TAG politics scalars (stability, war_support, political_power,
    command_power) + num_ships, read from the country object via the live bridge.

    Offsets within the country object are taken from the economy/owner offset
    drafts. Only fields whose offset is PINNED in the draft are read; anything
    else stays null (no fabrication). A field that reads out of its sane range is
    null, not clamped.
    """
    cib, locate = impl
    bridge = ctx.bridge
    if bridge is None:
        raise RuntimeError("country-index bridge not resolved")
    p = ctx.proc
    offs = ctx.extras.get("economy_offsets") or {}
    fields = offs.get("fields") or {}
    countries: Dict[str, Dict[str, Any]] = {}
    for tag_db_id in bridge.as_map():
        tag = ctx.resolve_tag(tag_db_id)
        if tag is None:
            continue   # un-nameable country: cannot key it by TAG, skip (not fab)
        obj = bridge.country_object(tag_db_id)
        if not obj:
            continue
        rec: Dict[str, Any] = {}
        for fname in ("stability", "war_support", "political_power",
                      "command_power", "num_ships"):
            spec = fields.get(fname)
            val = _read_country_field(p, obj, spec)
            if val is not None:
                rec[fname] = val
        if rec:
            countries[tag] = rec
    return {"countries": countries, "count": len(countries)}


def _f_supply():
    from logistics_intel import supply_live  # noqa
    return supply_live


def _r_supply(ctx: CaptureContext, supply_live) -> Dict[str, Any]:
    """{province_id: {current, capacity, gain, divisions}} live supply proxy."""
    raw = supply_live.enumerate_supply(ctx.proc)
    out = {str(int(pid)): rec for pid, rec in raw.items()}
    return {"by_province": out, "count": len(out)}


def _f_sea_control():
    from naval_air.naval_air_reader import NavalAirReader  # noqa
    return NavalAirReader


def _r_sea_control(ctx: CaptureContext, NavalAirReader) -> Dict[str, Any]:
    """Naval sea-zone dominance + air superiority by strategic region (TAGs).

    REQUIRES the tag table: without it the reader would emit raw tag-db-ids /
    "?<id>" placeholders into controller/owner keys (Codex code-review #5). We
    sanitize any surviving "?<id>" placeholder to None so a numeric id can never
    masquerade as a TAG in the sidecar.
    """
    if ctx.tag_table is None:
        raise RuntimeError("naval_air needs the tag table to name controllers")
    reader = NavalAirReader()
    try:
        _verify_same_process(ctx, reader)
        sea = reader.enumerate_sea_zone_control(tag_table=ctx.tag_table)
        air = reader.enumerate_air_superiority(tag_table=ctx.tag_table)
    finally:
        try:
            reader.close()
        except Exception:
            pass
    sea_out = {str(int(rid)): _sanitize_naval_entries(ents)
               for rid, ents in (sea or {}).items()}
    air_out = {str(int(rid)): _sanitize_air_region(r)
               for rid, r in (air or {}).items()}
    return {"sea_zone_control": sea_out, "air_superiority": air_out}


def _clean_tag(v: Any) -> Optional[str]:
    """Return a real 3-letter TAG or None. The naval/air reader writes "?<id>"
    when it cannot name a controller; that is an un-nameable id, NOT a TAG -> None
    (no numeric id leaks into a tag field, Codex #5)."""
    if isinstance(v, str) and v and not v.startswith("?") and v != "---":
        return v
    return None


def _sanitize_naval_entries(ents):
    out = []
    for e in (ents or []):
        if isinstance(e, dict):
            e = dict(e)
            if "controller" in e:
                e["controller"] = _clean_tag(e.get("controller"))
            # Drop any RAW tag-db-id the reader left in the entry (adversarial
            # review #4): a numeric id must never reach the sidecar masquerading
            # as identity. The named TAG already lives in "controller" (or is
            # None); the raw id is diagnostic-only and is stripped here.
            e.pop("controller_tagid", None)
            e.pop("controller_id", None)
        out.append(e)
    return out


def _sanitize_air_region(r):
    if not isinstance(r, dict):
        return r
    r = dict(r)
    by = r.get("by_owner")
    if isinstance(by, dict):
        # keys that are "?<id>" placeholders are un-nameable owners: relabel the
        # bucket key to None-string is lossy, so we move them under an explicit
        # "_unresolved" bucket instead of pretending they are a TAG.
        clean = {}
        unresolved = {"effective_planes": 0.0, "wings": 0}
        for k, v in by.items():
            if _clean_tag(k) is None:
                if isinstance(v, dict):
                    unresolved["effective_planes"] += v.get("effective_planes", 0) or 0
                    unresolved["wings"] += v.get("wings", 0) or 0
            else:
                clean[k] = v
        if unresolved["wings"] or unresolved["effective_planes"]:
            clean["_unresolved"] = unresolved
        r["by_owner"] = clean
    # sanitize the leader UNCONDITIONALLY (Codex reconcile #5) -- not only when
    # by_owner happens to be a dict; an un-nameable leader becomes None.
    if _clean_tag(r.get("leader")) is None:
        r["leader"] = None
    return r


def _f_land_combats():
    from combat.combat_enum import CombatReader  # noqa
    return CombatReader


def _r_land_combats(ctx: CaptureContext, CombatReader) -> Dict[str, Any]:
    """Active LAND combats (the SOLVED, save-validated 13/13 land reader).

    Each record is {combat_id, location(province id), day, duration,
    attacker_units:[div_id], defender_units:[div_id]}. These carry NO tag-db-ids:
    div_ids and the province id are raw engine ints, NOT tags, so ctx.resolve_tag
    is deliberately NOT applied (Codex Q4 -- applying it would be a category error).
    A side that failed the reader's fail-closed validators is None (distinct from an
    empty []); we pass that through honestly. The assembler promotes these into the
    canonical typed combats[] (kind="land") AND keeps the raw rows in a sidecar."""
    reader = CombatReader()
    try:
        _verify_same_process(ctx, reader)
        rows = reader.enumerate_land_combats(with_defenders=True)
    finally:
        try:
            reader.close()
        except Exception:
            pass
    out = []
    for r in (rows or []):
        out.append({
            "combat_id": r.get("combat_id"),
            "location": r.get("location"),
            "day": r.get("day"),
            "duration": r.get("duration"),
            "attacker_units": r.get("attacker_units"),
            "defender_units": r.get("defender_units"),
        })
    return {"land": out, "count": len(out)}


def _f_combats():
    # The naval/air COMBAT reader is being cracked RIGHT NOW by another agent.
    # Import-guard it: if the methods aren't there yet, the plug is unavailable.
    from naval_air.naval_air_reader import NavalAirReader  # noqa
    if not (hasattr(NavalAirReader, "enumerate_naval_combats")
            and hasattr(NavalAirReader, "enumerate_air_combats")):
        raise ImportError("naval/air combat reader not yet available")
    return NavalAirReader


def _r_combats(ctx: CaptureContext, NavalAirReader) -> Dict[str, Any]:
    reader = NavalAirReader()
    try:
        _verify_same_process(ctx, reader)
        naval = reader.enumerate_naval_combats()
        air = reader.enumerate_air_combats()
    finally:
        try:
            reader.close()
        except Exception:
            pass
    return {"naval": naval or [], "air": air or []}


def _f_buildings_save():
    from logistics_intel import buildings_save as bs  # noqa
    return bs


def _r_buildings_save(ctx: CaptureContext, bs) -> Dict[str, Any]:
    """SAVE-anchored per-province rail/supply_node/naval_base + per-state building
    levels. This is NOT a live memory read (Codex Q2): it parses a melted save on
    disk and is OPT-IN -- it activates ONLY when the caller supplies a path via
    ctx.extras. With no path it RAISES, so the domain is honestly absent by default
    (no fabrication) rather than fabricating a freshness it does not have.

    extras keys (any one enables it):
        ``melt_path``         -> per-province building levels (BuildingsSave.from_melt)
        ``state_detail_path`` -> per-state building levels (StateBuildings.from_state_detail)
    The result is provenance-honest about its source (path + the fact it is a save,
    not a live read). It does NOT touch ctx.proc / PID / exe checks."""
    melt = ctx.extras.get("melt_path")
    state_detail = ctx.extras.get("state_detail_path")
    if not melt and not state_detail:
        raise RuntimeError("buildings (save-backstop) needs a melt_path or "
                           "state_detail_path in ctx.extras; none supplied "
                           "(domain honestly omitted, not fabricated)")
    out: Dict[str, Any] = {"source": "save", "from": "buildings_save"}
    if melt:
        layer = bs.BuildingsSave.from_melt(melt)
        # emit the full per-province set the reader exposes, not just rail
        # (adversarial review #6): rail_way, supply_node, naval_base.
        out["rail_by_province"] = {str(int(pid)): lvl
                                   for pid, lvl in layer.rail_map().items()}
        out["supply_node_by_province"] = {
            str(int(pid)): lvl
            for pid, lvl in (layer.prov.get("supply_node") or {}).items()}
        out["naval_base_by_province"] = {
            str(int(pid)): lvl
            for pid, lvl in (layer.prov.get("naval_base") or {}).items()}
        # HARD redaction rule (SCHEMA.md / global): store the BASENAME only -- never
        # an absolute / user-directory path in tracked snapshot output.
        out["melt_file"] = _basename(melt)
    if state_detail:
        sl = bs.StateBuildings.from_state_detail(state_detail)
        # full per-state building dict, not just infrastructure (review #6).
        out["state_buildings_by_state"] = {str(int(sid)): dict(blds)
                                           for sid, blds in sl.states.items()}
        out["infrastructure_by_state"] = {str(int(sid)): sl.infrastructure(sid)
                                          for sid in sl.states}
        out["state_detail_file"] = _basename(state_detail)
    return out


def _f_intel_save():
    from logistics_intel import intel_save as isv  # noqa
    return isv


def _r_intel_save(ctx: CaptureContext, isv) -> Dict[str, Any]:
    """SAVE-anchored per-(observer TAG, target INDEX) intel rows. NOT a live read
    (the live intel array is an unresolved residual). OPT-IN: activates ONLY when
    ctx.extras['melt_path'] is supplied; raises otherwise (absent by default).

    No-fabrication caveat (Codex Q3): the observer key IS a real save TAG, but the
    target INDEX -> TAG mapping is EXPLICITLY UNCONFIRMED in the reader's own
    docstring. We therefore keep target indices as raw integers and DO NOT relabel
    them as TAGs; a provenance note records that the index->TAG map is unvalidated."""
    melt = ctx.extras.get("melt_path")
    if not melt:
        raise RuntimeError("intel (save-backstop) needs ctx.extras['melt_path']; "
                           "none supplied (domain honestly omitted, not fabricated)")
    layer = isv.IntelSave.from_melt(melt)
    rows: Dict[str, Any] = {}
    for obs in layer.observers():
        # keys are target INDEXES (ints), kept raw -- the index->TAG map is
        # UNCONFIRMED, so we never fabricate a TAG here.
        rows[obs] = {str(int(idx)): rec
                     for idx, rec in layer.observer_map(obs).items()}
    return {"source": "save", "from": "intel_save",
            "melt_file": _basename(melt),  # basename only (HARD redaction rule)
            "by_observer": rows, "stats": layer.stats(),
            "target_index_is_tag": False,
            "note": ("target indices are raw engine indices; the index->TAG map "
                     "is UNCONFIRMED and intentionally NOT resolved")}


# NOTE on weather (Codex code-review #10): the weather reader is being cracked by
# another agent right now. Its only live entry point (read_live_month_value ->
# locate_clock) does the ~150s arena scan AND returns a DATE integer, not weather.
# That is neither cheap nor actually-weather, so there is NO honest weather plug to
# register yet -- registering one would either block every capture or surface clock
# data mislabeled as weather (a fabrication). The weather domain therefore stays
# entirely absent from the default registry until a cheap, true weather read lands;
# it is NOT in default_registry() below. (No stub, no fake "captured" -- absent.)


def _basename(path: Optional[str]) -> Optional[str]:
    """Return the BASENAME of a path (HARD redaction rule, SCHEMA.md §2): tracked
    snapshot output must never contain an absolute / user-directory path. Handles
    both separators so a Windows path is redacted on any OS."""
    if not path:
        return None
    return os.path.basename(str(path).replace("\\", "/").rstrip("/"))


# ---------------------------------------------------------------------------
# small helpers shared by readers
# ---------------------------------------------------------------------------
def _verify_same_process(ctx: CaptureContext, reader) -> None:
    """A reader that opened its OWN handle must point at the SAME process instance
    as the capture context, else a domain could silently read a different hoi4.exe.

    Compare PID *and* process creation time (Codex code-review #9): PID alone can
    be reused. We also confirm the reader's exe sha matches the context's so a
    reader can't read an unverified build. Raise on any mismatch (fail-closed ->
    the domain is honestly omitted rather than cross-wired)."""
    ident = ctx.identity
    rpid = getattr(reader, "pid", None)
    if ident is None or rpid is None:
        return
    from live_capture import process_identity as _pi
    rid = _pi.identify(reader)
    if int(rid.pid) != int(ident.pid):
        raise RuntimeError("reader attached to pid %s != capture pid %s"
                           % (rid.pid, ident.pid))
    # creation-time check (defeats PID reuse) when both are known.
    ct_r = getattr(rid, "create_time_100ns", None)
    ct_c = getattr(ident, "create_time_100ns", None)
    if ct_r is not None and ct_c is not None and ct_r != ct_c:
        raise RuntimeError("reader pid %s creation-time mismatch (PID reuse)"
                           % rid.pid)
    sha_r = getattr(rid, "exe_sha256", None)
    sha_c = getattr(ident, "exe_sha256", None)
    if sha_r and sha_c and sha_r != sha_c:
        raise RuntimeError("reader exe sha mismatch (different build)")


def _read_country_field(p, obj_addr, spec) -> Optional[Any]:
    """Read one scalar from a country object given an offset spec from the draft.

    spec is expected to look like {"offset": "0x...", "type": "f32"|"u32"|...,
    "scale": <float>} (or a subset). Returns None if the spec is missing/unpinned
    or the read fails -- NEVER a guessed value. A read that lands out of a declared
    sane range returns None (garbage, not a measured value)."""
    if not isinstance(spec, dict):
        return None
    off = spec.get("offset")
    if off is None:
        return None
    try:
        off = int(off, 16) if isinstance(off, str) else int(off)
    except (TypeError, ValueError):
        return None
    typ = (spec.get("type") or "u32").lower()
    try:
        addr = obj_addr + off
        if typ in ("f32", "float"):
            val = p.f32(addr)
        elif typ in ("f64", "double"):
            val = p.f64(addr)
        elif typ in ("i32", "int"):
            val = p.i32(addr)
        elif typ in ("u16",):
            val = p.u16(addr)
        else:
            val = p.u32(addr)
    except Exception:
        return None
    scale = spec.get("scale")
    if scale:
        try:
            val = val / float(scale)
        except (TypeError, ZeroDivisionError):
            return None
    lo = spec.get("sane_min")
    hi = spec.get("sane_max")
    if lo is not None and val < lo:
        return None
    if hi is not None and val > hi:
        return None
    return val


# ---------------------------------------------------------------------------
# The default registry.
# ---------------------------------------------------------------------------
def default_registry() -> List[ReaderPlug]:
    """The built-in plugs, in a sensible read order. Combat + (future) weather
    are import-guarded and report unavailable until cracked."""
    return [
        ReaderPlug("state_control", _f_state_control, _r_state_control),
        ReaderPlug("province_control", _f_province_control, _r_province_control),
        ReaderPlug("divisions", _f_divisions, _r_divisions),
        ReaderPlug("country_scalars", _f_country_scalars, _r_country_scalars),
        ReaderPlug("supply", _f_supply, _r_supply),
        ReaderPlug("naval_air", _f_sea_control, _r_sea_control),
        ReaderPlug("land_combats", _f_land_combats, _r_land_combats),
        ReaderPlug("combats", _f_combats, _r_combats),
        # Save-backstop plugs: OPT-IN only (they raise unless a save path is in
        # ctx.extras), so they are absent by default in a pure-live capture. They
        # are registered so a caller that DOES supply a melt path gets them, but
        # they never fabricate freshness (Codex Q2).
        ReaderPlug("buildings", _f_buildings_save, _r_buildings_save),
        ReaderPlug("intel", _f_intel_save, _r_intel_save),
        # weather: intentionally NOT registered -- no cheap, true weather read
        # exists yet (see the weather note above). Absent != fabricated.
    ]


def read_all(ctx: CaptureContext,
             plugs: Optional[List[ReaderPlug]] = None) -> Dict[str, DomainResult]:
    """Run every plug once; return {domain_name: DomainResult}. Never raises for a
    plug failure (each plug's read() is already transactional)."""
    if plugs is None:
        plugs = default_registry()
    out: Dict[str, DomainResult] = {}
    for plug in plugs:
        out[plug.name] = plug.read(ctx)
    return out


__all__ = [
    "CaptureContext", "DomainResult", "ReaderPlug",
    "default_registry", "read_all",
]
