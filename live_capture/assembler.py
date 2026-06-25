"""assembler.py -- fold reader-plug results into a schema-conformant snapshot.

Takes the ``{domain: DomainResult}`` map from the registry plus a date and a
game_id, and produces a snapshot dict that ``replay.schema.validate_snapshot``
accepts. The cardinal rule is no fabrication:

  * Start from ``schema.empty_snapshot`` (fully null-but-present).
  * Only a domain whose plug ran to completion (``domain_captured``) is folded
    AND listed in ``provenance.memory.domains``. A domain that did not run leaves
    every one of its fields at the schema null.
  * A value is null iff not measured; ``0``/``0.0`` is a measured zero. We never
    coerce a missing read into 0.
  * tag-db-ids are already resolved to TAGs by the plugs (the registry's
    ``resolve_tag`` returns None, never a numeric id, for an unresolvable id), so
    no numeric id ever lands in a ``*_tag``/``owner``/``controller`` field.

The naval/air superiority and supply domains have no first-class home in the v1
snapshot schema, so they are surfaced under provenance-tracked sidecar fields
(``snap["_live_naval_air"]`` / ``snap["_live_supply"]``) -- present only when
captured, absent otherwise. The schema validator ignores unknown top-level keys,
so this keeps the canonical contract intact while not throwing the data away.
"""
from __future__ import annotations

import copy
import os
import sys
from typing import Any, Dict, Optional

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir))
if _REPO not in sys.path:
    sys.path.insert(0, _REPO)

from replay import schema  # noqa: E402


def _redact_path(value: Any) -> Any:
    """Defensive basename-only redaction at the assembler boundary (HARD rule,
    SCHEMA.md §2): never let an absolute / user-directory path reach tracked
    output, even if a caller bypassed the readers' own redaction. Handles
    os.PathLike via os.fspath. A value with no separator is already a basename and
    is returned unchanged."""
    if hasattr(value, "__fspath__"):
        try:
            value = os.fspath(value)
        except TypeError:
            return value
    if not isinstance(value, str) or value == "":
        return value
    return os.path.basename(value.replace("\\", "/").rstrip("/"))


# Any key in a save-backstop payload whose VALUE is a filesystem path. These are
# scrubbed to a basename before the payload enters the persisted snapshot, and the
# raw *_path aliases are dropped entirely (the *_file basename is the kept form).
_SAVE_PATH_KEYS = ("melt_file", "state_detail_file", "melt_path",
                   "state_detail_path", "path", "file")


def _redact_save_payload(payload: Any) -> Any:
    """Return a shallow copy of a save-backstop payload with every path-valued key
    basename-redacted (HARD rule, FIX3). Drops raw ``*_path`` aliases so only the
    redacted ``*_file`` basename survives. Non-dict payloads pass through."""
    if not isinstance(payload, dict):
        return payload
    out = dict(payload)
    for k in list(out.keys()):
        if k in _SAVE_PATH_KEYS:
            if k.endswith("_path"):
                # normalize a raw *_path into its *_file basename, drop the alias.
                base_key = k[:-5] + "_file"
                out.setdefault(base_key, _redact_path(out[k]))
                del out[k]
            else:
                out[k] = _redact_path(out[k])
    return out


def _ensure_country(snap: Dict[str, Any], tag: str) -> Dict[str, Any]:
    c = snap["countries"].get(tag)
    if c is None:
        c = schema.empty_country()
        snap["countries"][tag] = c
    return c


def _ensure_state(snap: Dict[str, Any], sid: str) -> Dict[str, Any]:
    st = snap["states"].get(sid)
    if st is None:
        st = schema.empty_state()
        snap["states"][sid] = st
    return st


def assemble_snapshot(results: Dict[str, Any], *, game_id: str,
                      date: Optional[str],
                      exe_sha256: Optional[str] = None,
                      identity: Any = None,
                      extra_notes: Optional[list] = None) -> Dict[str, Any]:
    """Build a validated snapshot dict from plug results.

    ``results`` is ``{domain_name: DomainResult}``. ``date`` is the Clausewitz
    date string (may be None -> hour_index None). Raises ValueError (via the store
    on write) only later; this function itself does NOT validate -- the writer
    does, so a caller can inspect/derive first.
    """
    snap = schema.empty_snapshot(game_id, date)
    captured_domains = []

    def captured(name: str):
        r = results.get(name)
        if r is not None and getattr(r, "domain_captured", False):
            return r.data or {}
        return None

    # --- states (owner is M/S; we only have the live controller crack here) ---
    sc = captured("state_control")
    if sc is not None:
        for sid, tag in (sc.get("controllers") or {}).items():
            st = _ensure_state(snap, sid)
            # measured controller (may be None = un-nameable id, still measured-
            # present as a state). owner stays null: we did not measure it here.
            # Defense in depth (Codex code-review #6): never let a non-TAG (e.g. a
            # raw numeric id from a misbehaving plug) reach a *_tag field.
            st["controller"] = tag if isinstance(tag, str) else None
        captured_domains.append("state_control")

    # --- provinces ---
    pc = captured("province_control")
    if pc is not None:
        for pid, tag in (pc.get("controllers") or {}).items():
            prv = schema.empty_province()
            prv["controller"] = tag if isinstance(tag, str) else None
            snap["provinces"][pid] = prv
        captured_domains.append("province_control")

    # --- divisions ---
    dv = captured("divisions")
    if dv is not None:
        out = []
        counts: Dict[str, int] = {}
        for d in (dv.get("divisions") or []):
            rec = schema.empty_division()
            rec["div_id"] = d.get("div_id")
            owner = d.get("owner")
            rec["owner"] = owner if isinstance(owner, str) else None  # Codex #6
            rec["location_province"] = d.get("location_province")
            rec["organisation"] = d.get("organisation")
            rec["strength"] = d.get("strength")
            rec["experience"] = d.get("experience")
            rec["moving"] = d.get("moving")
            rec["move_target_province"] = d.get("move_target_province")
            rec["move_progress"] = d.get("move_progress")
            rec["in_combat"] = d.get("in_combat")
            rec["supply_status"] = d.get("supply_status")
            # only count a division toward a country when its owner resolved.
            if isinstance(rec["owner"], str):
                counts[rec["owner"]] = counts.get(rec["owner"], 0) + 1
            out.append(rec)
        # Drop entries lacking an int div_id (schema requires int div_id + unique
        # uid). This is identity-invalid, not fabricated absence.
        snap["divisions"] = [r for r in out if isinstance(r.get("div_id"), int)]
        for tag, n in counts.items():
            _ensure_country(snap, tag)["military"]["division_count"] = n
        captured_domains.append("divisions")

    # --- country scalars (politics + num_ships) ---
    cs = captured("country_scalars")
    if cs is not None:
        for tag, rec in (cs.get("countries") or {}).items():
            c = _ensure_country(snap, tag)
            pol = c["politics"]
            for k in ("stability", "war_support", "political_power",
                      "command_power"):
                if k in rec:
                    pol[k] = rec[k]
            mil = c["military"]
            if "num_ships" in rec:
                mil["num_ships"] = rec["num_ships"]
            if "command_power" in rec:
                mil["command_power"] = rec["command_power"]
        captured_domains.append("country_scalars")

    # When the divisions domain WAS captured, a known country with no division is
    # a MEASURED zero, not unmeasured -> division_count = 0 (Codex code-review #4).
    # Only a country we actually know about (it appears in the snapshot) gets the
    # measured zero; we do not invent countries just to zero them.
    if dv is not None:
        for tag, c in snap["countries"].items():
            mil = c["military"]
            if mil.get("division_count") is None:
                mil["division_count"] = 0

    # --- map mirrors (derived-from-domains; only mirror what we measured) ---
    _fill_map_mirrors(snap)

    # --- naval region control + air superiority -> FIRST-CLASS schema field -----
    # (backlog #3) These are PROXY / region-level LIVE reads, NOT per-province or
    # per-ship ground truth. We promote them into the typed snap["naval_air"] field
    # WITH a source/confidence badge so the UI never renders region control / air
    # superiority as authoritative. The raw plug payload is also kept in the
    # _live_naval_air sidecar for back-compat / the lifecycle sampler.
    na = captured("naval_air")
    if na is not None:
        snap["_live_naval_air"] = na
        snap["naval_air"] = {
            # the plug emits sea_zone_control; surface it under the schema's
            # region_control name (strategic-region granularity).
            "region_control": na.get("sea_zone_control"),
            "air_superiority": na.get("air_superiority"),
            "source": "M",
            "confidence": "region_proxy",
        }
        captured_domains.append("naval_air")
    # --- live supply PROXY -> FIRST-CLASS schema field (badged) -----------------
    # The exact per-province supply NUMBER is a documented wall (DESIGN §B2); this
    # is the validated division-province proxy LAYER, badged as such.
    sup = captured("supply")
    if sup is not None:
        snap["_live_supply"] = sup
        snap["supply"] = {
            "by_province": sup.get("by_province"),
            "source": "M",
            "confidence": "division_province_proxy",
        }
        captured_domains.append("supply")
    cb = captured("combats")
    if cb is not None:
        # The NAVAL/AIR combat crack is in flux; keep its raw plug data in a
        # sidecar and do NOT fold it into the typed combats[] (no fabrication into
        # the canonical list from an unstable source).
        snap["_live_combats"] = cb
        # Provenance honesty (adversarial review #1, refined): this naval/air data
        # is sidecar-ONLY -- it is NOT folded into the canonical typed combats[].
        # So we must NOT claim the canonical "combats" domain at all (that would
        # imply combats[] was populated). We use a DISTINCT sidecar domain id
        # ("naval_air_combats") and, even then, only when the detector actually
        # found something (an empty detector run is an unproven absence, not a
        # measured zero -> sidecar-flagged, not claimed).
        n_found = len(cb.get("naval") or []) + len(cb.get("air") or [])
        if n_found > 0:
            captured_domains.append("naval_air_combats")
        else:
            snap["_live_combats"]["_detector_only_unproven_absence"] = True

    # --- LAND combat: the SOLVED, save-validated 13/13 reader. This one IS stable
    # and its shape maps cleanly onto the typed Combat schema, so we promote it
    # into the canonical combats[] (kind="land"). We also keep the raw rows in a
    # sidecar for the lifecycle sampler. div_ids / province ids are raw ints (NOT
    # tags) and pass through unresolved (Codex Q1/Q4).
    lc = captured("land_combats")
    if lc is not None:
        snap["_live_land_combats"] = lc
        for r in (lc.get("land") or []):
            # A side the reader could not validate is None (fail-closed) -- that is
            # "not measured", NOT "measured empty". We must NOT coerce None -> []
            # (adversarial review #2: that fabricates "no participants"). Preserve
            # the distinction: a measured side ([...] incl. measured empty []) is
            # copied through; an unmeasured side (None) stays None in the typed
            # Combat. The raw sidecar already preserves it too.
            atk = r.get("attacker_units")
            dfn = r.get("defender_units")
            combat = schema.asdict(schema.Combat(
                combat_id=r.get("combat_id"),
                kind="land",
                location_province=r.get("location"),
                attackers=list(atk) if isinstance(atk, list) else None,
                defenders=list(dfn) if isinstance(dfn, list) else None,
            ))
            snap["combats"].append(combat)
        captured_domains.append("land_combats")

    # --- SAVE-anchored backstops (opt-in; present only when a save path was given)
    # Named _save_* (not _live_*) so a consumer never mistakes them for a live read
    # (Codex Q2). These are NOT memory-sourced, so they are tracked in a SEPARATE
    # provenance bucket (save_backstops) and are NOT listed under
    # provenance.memory.domains -- mislabeling save data as a memory read would be
    # dishonest provenance (adversarial review #5).
    save_backstops = []
    bld = captured("buildings")
    if bld is not None:
        # Scrub the WHOLE payload before it enters the persisted snapshot
        # (adversarial review FIX3): the full snap is written to disk, so a raw
        # path on ANY *_path/*_file key in the sidecar itself would leak even though
        # provenance.source_filenames is redacted. Redact the payload at the source.
        snap["_save_buildings"] = _redact_save_payload(bld)
        save_backstops.append("buildings")
    intel = captured("intel")
    if intel is not None:
        snap["_save_intel"] = _redact_save_payload(intel)
        save_backstops.append("intel")

    # --- provenance: memory channel, honest domain list --------------------
    notes = list(extra_notes or [])
    unresolved = _collect_unresolved(results)
    if unresolved:
        notes.append("unresolved tag-db-ids kept as null (no fabrication): "
                     + ", ".join("%s=%d" % (k, v) for k, v in unresolved.items()))
    snap["provenance"]["memory"] = {
        "captured": bool(captured_domains),
        "exe_sha256": exe_sha256,
        "domains": captured_domains,
        "save_date_validated_against": None,
        "process_instance_id": (identity.instance_id if identity is not None
                                else None),
        "pid": (identity.pid if identity is not None else None),
    }
    # Save-backstop provenance lives in the SAVE channel, never the memory channel
    # (adversarial review #5). Only populated when a backstop actually ran; a pure
    # live capture leaves it None (no save backed it).
    if save_backstops:
        # Source identifiers are BASENAMES ONLY. The plugs already redact to *_file
        # keys, but we DEFENSIVELY re-basename here too (adversarial review #3): if a
        # caller ever fed raw data past the readers, the assembler boundary must
        # still never emit a user path into tracked provenance.
        srcs = {}
        if "buildings" in save_backstops:
            b = snap.get("_save_buildings") or {}
            srcs["buildings"] = {k: _redact_path(b.get(k))
                                 for k in ("melt_file", "state_detail_file")
                                 if b.get(k)}
        if "intel" in save_backstops:
            srcs["intel"] = {"melt_file": _redact_path(
                (snap.get("_save_intel") or {}).get("melt_file"))}
        # NOTE: NO "domains" key here (adversarial review #2). The save backstops do
        # NOT fill any CANONICAL domain (e.g. countries.*.intel stays null), so
        # claiming a canonical domain id would let the replay diff engine fabricate
        # domain-presence events. They are tracked ONLY as "backstops".
        snap["provenance"]["save"] = {
            "captured": True,
            "backstops": save_backstops,
            "source": "melted_save_on_disk",
            "is_live": False,
            "fills_canonical_domains": False,
            "source_filenames": srcs,
        }
    else:
        snap["provenance"]["save"] = None
    snap["provenance"]["notes"] = notes
    return snap


# ---------------------------------------------------------------------------
# SAVE-BUNDLE FOLD (backlog #1): wire the melted-save oracle in as the BACKSTOP
# layer so a live snapshot's national tabs (politics scalars, ruling party, laws,
# ideas, research, focus, production, economy, diplomacy/wars/factions, designs,
# OOB, intel, commanders, market, state_detail/occupation) populate from the save
# when the LIVE memory channel did not measure them.
#
# Provenance honesty (the cardinal rule Track A established):
#   * save-derived canonical domains land under provenance.save with
#     fills_canonical_domains=True and an explicit `domains` list — they are NOT
#     relabeled as provenance.memory.domains. The memory channel is left untouched.
#   * the overlay is a BACKSTOP: a value the LIVE channel measured WINS; the save
#     only fills where live is null/empty (the no-fabrication skeleton default).
#     0/0.0 from live is a measured zero and is preserved over a save value.
#   * a field stays null iff NEITHER channel measured it.
# ---------------------------------------------------------------------------
def _is_empty(v: Any) -> bool:
    """No-information value: None or the empty list/dict skeleton default. A value
    like 0 / 0.0 / False is INFORMATIVE (a measured zero) and is NOT empty."""
    return v is None or v == [] or v == {}


def _backstop_merge(live: Any, save: Any) -> Any:
    """Overlay save UNDER live: live wins wherever it carries information; save fills
    only the gaps. Dicts merge key-wise recursively; for a scalar/list, keep live
    when informative else take save. A measured zero/False on the live side is kept
    over a save value (it is informative).

    Measured-empty []/{} containers at the SPECIFIC leaves the live channel measured
    are protected separately by ``_merge_country`` (re-asserting those leaves AFTER
    this merge), so this function's generic "empty -> take save" rule is safe: it
    only ever fills a SKELETON-default empty, never a measured one (Codex P1)."""
    if isinstance(live, dict) and isinstance(save, dict):
        out = dict(live)
        for k, sv in save.items():
            if k in out:
                out[k] = _backstop_merge(out[k], sv)
            else:
                out[k] = sv
        return out
    # live wins when it carries info; else fall back to the save value.
    return live if not _is_empty(live) else save


# Map a LIVE memory domain to the EXACT country leaf paths it MEASURES (dotted,
# relative to the country object), so the fold protects a measured value/empty there
# from save overwrite WITHOUT over-protecting sibling leaves the domain did NOT
# measure (Codex P1). country_scalars measures only these specific scalars; a future
# live reader that measures e.g. a list field registers its leaf path here and the
# protection then covers a measured-empty []/{}.
_LIVE_COUNTRY_DOMAIN_LEAVES = {
    "country_scalars": (
        "politics.stability", "politics.war_support", "politics.political_power",
        "politics.command_power", "politics.ruling_party",
        "military.num_ships", "military.command_power",
    ),
}


def _set_leaf(d: Dict[str, Any], path: str, value: Any) -> None:
    """Set a dotted leaf, CREATING missing dict parents so a measured live value is
    never silently dropped (Codex P1c). A non-dict intermediate is the only case we
    cannot recover; that is a malformed snapshot and would have failed validation, so
    we leave it (the leaf simply isn't reasserted rather than corrupting the shape)."""
    parts = path.split(".")
    cur = d
    for p in parts[:-1]:
        nxt = cur.get(p)
        if nxt is None:
            nxt = {}
            cur[p] = nxt
        elif not isinstance(nxt, dict):
            return  # malformed shape; do not clobber a non-dict intermediate
        cur = nxt
    cur[parts[-1]] = value


def _get_leaf(d: Dict[str, Any], path: str) -> Any:
    cur: Any = d
    for p in path.split("."):
        if not isinstance(cur, dict):
            return None
        cur = cur.get(p)
    return cur


def _merge_country(live_c: Dict[str, Any], save_c: Dict[str, Any],
                   protected_leaves) -> Dict[str, Any]:
    """Backstop-merge one country (live wins where informative; save fills gaps),
    then RE-ASSERT the exact live-measured leaves so save can never overwrite a
    value the live channel measured -- including a measured-empty []/{} (Codex P1).
    A live None leaf is unmeasured and is left for save to have filled."""
    merged = _backstop_merge(live_c, save_c)
    for path in protected_leaves:
        lv = _get_leaf(live_c, path)
        if lv is not None:  # measured (incl. 0/0.0/False/[]/{}) -> re-assert it
            # deep-copy a mutable leaf so the merged snapshot never ALIASES the live
            # snapshot's object (Codex P1c: later mutation of one must not affect the
            # other). Scalars are immutable -> copied trivially.
            _set_leaf(merged, path,
                      copy.deepcopy(lv) if isinstance(lv, (dict, list)) else lv)
    return merged


def fold_save_bundle(snap: Dict[str, Any], output_dir: str, *,
                     game_id: Optional[str] = None) -> Dict[str, Any]:
    """Fold a save_oracle output directory into an already-assembled LIVE snapshot
    as the BACKSTOP layer, in place. Returns the same snap for chaining.

    READ-ONLY on ``output_dir``. Live-measured fields win; the save fills gaps.
    Records the save-filled canonical domains under ``provenance.save`` honestly
    (fills_canonical_domains=True), without touching ``provenance.memory``.
    """
    from replay import store as _store  # local import: keeps assembler import-light

    gid = game_id or snap.get("game_id") or "fold"
    # Build the canonical save snapshot (its own empty skeleton + oracle ingest).
    save_snap = _store.build_snapshot_from_oracle(output_dir, gid)
    save_prov = (save_snap.get("provenance") or {}).get("save") or {}
    save_domains = list(save_prov.get("domains") or [])
    if not save_domains:
        return snap  # nothing to fold (empty/absent oracle) -> no fabrication

    # Which exact country LEAVES did the LIVE memory channel measure? Those values
    # (incl. a measured-empty []/{}) must be protected from save overwrite (Codex P1).
    live_mem_domains = ((snap.get("provenance") or {}).get("memory") or {}).get(
        "domains") or []
    protected_leaves = []
    for dom in live_mem_domains:
        protected_leaves.extend(_LIVE_COUNTRY_DOMAIN_LEAVES.get(dom, ()))

    # --- countries: backstop each TAG's domains (live wins) ---
    live_countries = snap.setdefault("countries", {})
    for tag, sc in (save_snap.get("countries") or {}).items():
        lc = live_countries.get(tag)
        if lc is None:
            # a TAG present only in the save: the save IS the only measurement.
            live_countries[tag] = sc
        else:
            live_countries[tag] = _merge_country(lc, sc, protected_leaves)

    # --- states / provinces: backstop per entity ---
    for container in ("states", "provinces"):
        live_c = snap.setdefault(container, {})
        for key, sv in (save_snap.get(container) or {}).items():
            lv = live_c.get(key)
            live_c[key] = sv if lv is None else _backstop_merge(lv, sv)

    # --- divisions: live divisions[] are authoritative when present; only adopt the
    # save roster wholesale when LIVE captured no divisions (else mixing two channels'
    # (id,type) rosters risks double-counting). This mirrors the no-fabrication rule:
    # a live divisions read replaces, it does not get diluted by stale save rows.
    if not snap.get("divisions") and save_snap.get("divisions"):
        snap["divisions"] = save_snap["divisions"]

    # --- world-scoped catalogs: backstop only if live left them empty ---
    for key in ("designs", "order_of_battle", "diplomacy_world"):
        snap[key] = _backstop_merge(snap.get(key), save_snap.get(key))

    # --- combats: only adopt the save's active combats when live captured none ---
    if not snap.get("combats") and save_snap.get("combats"):
        snap["combats"] = save_snap["combats"]

    # --- map mirrors: refill from the merged state/province data ---
    _fill_map_mirrors(snap)

    # --- provenance.save: record the save-filled canonical domains HONESTLY -------
    # Preserve any pre-existing non-canonical backstops (buildings/intel sidecars).
    prov = snap.setdefault("provenance", {})
    existing = prov.get("save") or {}
    backstops = list(existing.get("backstops") or [])
    src_filenames = dict(existing.get("source_filenames") or {})
    save_src = _redact_path(save_prov.get("source_filename"))
    new_save = {
        "captured": True,
        "source": "melted_save_on_disk",
        "is_live": False,
        # the full-bundle fold DOES fill canonical domains (distinct from the
        # buildings/intel sidecar backstops, which fill none).
        "fills_canonical_domains": True,
        "domains": sorted(set(save_domains)),
        "save_date": save_prov.get("save_date"),
        "source_filename": save_src,
        "source_filenames": src_filenames,
    }
    if backstops:
        new_save["backstops"] = backstops
    prov["save"] = new_save
    notes = list(prov.get("notes") or [])
    notes.append("save backstop folded %d canonical domain(s) from the melted save "
                 "(live channel wins; save fills gaps; no fabrication)."
                 % len(new_save["domains"]))
    prov["notes"] = notes
    return snap


def _fill_map_mirrors(snap: Dict[str, Any]) -> None:
    """Mirror measured owner/controller into map.* (D convenience). Only emit a
    mirror when at least one measured value exists, else leave null (the empty
    skeleton already has them null)."""
    owner_by_state = {sid: st.get("owner") for sid, st in snap["states"].items()
                      if st.get("owner") is not None}
    control_by_province = {pid: prv.get("controller")
                           for pid, prv in snap["provinces"].items()
                           if prv.get("controller") is not None}
    snap["map"]["owner_by_state"] = owner_by_state or None
    snap["map"]["control_by_province"] = control_by_province or None


def _collect_unresolved(results: Dict[str, Any]) -> Dict[str, int]:
    out: Dict[str, int] = {}
    for name, r in results.items():
        data = getattr(r, "data", None)
        if not isinstance(data, dict):
            continue
        for k in ("unresolved_ids", "unresolved_owner"):
            v = data.get(k)
            if isinstance(v, int) and v > 0:
                out["%s.%s" % (name, k)] = v
    return out


__all__ = ["assemble_snapshot", "fold_save_bundle"]
