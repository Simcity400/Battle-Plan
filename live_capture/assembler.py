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
import json
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

# Keys whose mere PRESENCE (key name OR value) the path-leak guard rejects — derived
# models echo a `source_path_redacted` (a ~/AppData/... path) in their meta. These are
# DROPPED outright (key+value) before any model meta enters the persisted snapshot;
# the redacted basename already survives as `source_filename`.
_DROP_META_KEYS = ("source_path_redacted", "source_path")


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


def _load_doctrine_save(output_dir: str) -> Optional[Dict[str, Any]]:
    """Load save_oracle doctrine.json if present.

    Doctrine is currently an additive save-cadence surface, not a replay/schema.py
    canonical domain. This helper keeps the fold local to live_capture so the
    parser can expose truthful save state without claiming a replay-domain enum
    that does not exist yet.
    """
    path = os.path.join(output_dir, "doctrine.json")
    if not os.path.isfile(path):
        return None
    try:
        with open(path, "r", encoding="utf-8") as fh:
            doc = json.load(fh)
    except Exception:
        return None
    return doc if isinstance(doc, dict) else None


def _fold_doctrine_save(snap: Dict[str, Any], doc: Optional[Dict[str, Any]]) -> Dict[str, int]:
    """Fold doctrine.json into additive snapshot homes, fail-closed.

    TAG-keyed rows land under countries.*.research.doctrine. Non-TAG save keys are
    kept only in snap["doctrine_save"].countries_by_key; no country-id bridge is
    inferred here.
    """
    stats = {"tag_rows": 0, "non_tag_rows": 0}
    if not isinstance(doc, dict):
        return stats

    for tag, rec in (doc.get("countries") or {}).items():
        if not schema.valid_tag(tag) or not isinstance(rec, dict):
            continue
        payload = {}
        for key in ("raw", "completed", "completed_count", "in_progress"):
            if key in rec:
                payload[key] = copy.deepcopy(rec[key])
        if not payload:
            continue
        payload["source"] = "S"
        _ensure_country(snap, tag)["research"]["doctrine"] = payload
        stats["tag_rows"] += 1

    by_key = doc.get("countries_by_key")
    if isinstance(by_key, dict) and by_key:
        side = snap.setdefault("doctrine_save", {})
        side["countries_by_key"] = copy.deepcopy(by_key)
        side["source"] = "S"
        stats["non_tag_rows"] = len(by_key)

    return stats


def _scrub_model_meta(meta: Any) -> Any:
    """Recursively drop path-leaking keys (_DROP_META_KEYS) from a derived-model meta
    block and basename any remaining path-valued key (_SAVE_PATH_KEYS). The path-leak
    guard rejects even the key NAME `source_path_redacted`, so we delete it outright;
    the basename survives as `source_filename`. Non-dict input passes through."""
    if isinstance(meta, list):
        return [_scrub_model_meta(x) for x in meta]
    if not isinstance(meta, dict):
        return meta
    out = {}
    for k, v in meta.items():
        if k in _DROP_META_KEYS:
            continue  # drop key + value entirely
        if k in _SAVE_PATH_KEYS:
            out[k] = _redact_path(v)
        else:
            out[k] = _scrub_model_meta(v)
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
                      extra_notes: Optional[list] = None,
                      modset: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
    """Build a validated snapshot dict from plug results.

    ``results`` is ``{domain_name: DomainResult}``. ``date`` is the Clausewitz
    date string (may be None -> hour_index None). Raises ValueError (via the store
    on write) only later; this function itself does NOT validate -- the writer
    does, so a caller can inspect/derive first.

    ``modset`` (additive, §K MOD AWARENESS) is the active MOD-SET read from the
    host's launcher config (``live_capture.mod_identity.read_modset``); it lands
    verbatim in ``provenance.mod``. ``None`` (the default, and the value when no
    launcher config was found) records ``provenance.mod = None`` -- NO FABRICATION,
    never a guessed mod.
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
            # Defense in depth (schema review #6): never let a non-TAG (e.g. a
            # raw numeric id from a misbehaving plug) reach a *_tag field.
            st["controller"] = tag if schema.valid_tag(tag) else None
        captured_domains.append("state_control")

    # --- provinces ---
    pc = captured("province_control")
    if pc is not None:
        for pid, tag in (pc.get("controllers") or {}).items():
            prv = schema.empty_province()
            prv["controller"] = tag if schema.valid_tag(tag) else None
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
            rec["owner"] = owner if schema.valid_tag(owner) else None  # schema #6/#5
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

    # --- country scalars (politics + num_ships + manpower_pool + convoys) ---
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
            # manpower_pool -> military.manpower.available (deployable pool, M).
            # manpower is a dict {available, ...}; only set the measured leaf, do
            # NOT invent total/locked here (no fabrication).
            if "manpower_pool" in rec:
                mp = mil.get("manpower")
                if not isinstance(mp, dict):
                    mp = {}
                    mil["manpower"] = mp
                mp["available"] = int(rec["manpower_pool"])
            # convoys -> military.convoys_count (integer stockpile count, M). The
            # raw read is i32/1e5 -> a float count; coerce to int (it is a count).
            if "convoys" in rec:
                cv = rec["convoys"]
                mil["convoys_count"] = int(round(cv)) if cv is not None else None
        captured_domains.append("country_scalars")

    # --- country misc (live convoys_destroyed / accidents_score / ace_count /
    # cosmetic_tag) -> country.misc (M). Only set the measured keys; a measured 0
    # is informative and kept. cosmetic_tag is a str or None (None == unset). ---
    cm = captured("country_misc")
    if cm is not None:
        for tag, rec in (cm.get("countries") or {}).items():
            c = _ensure_country(snap, tag)
            misc = c.get("misc")
            if not isinstance(misc, dict):
                misc = {}
                c["misc"] = misc
            # numeric scalars: a None here means the reader's 2-read sentinel
            # FAILED (unmeasured) -> omit. A measured 0 is informative -> kept.
            for k in ("convoys_destroyed", "accidents_score", "ace_count"):
                if k in rec and rec[k] is not None:
                    misc[k] = rec[k]
            # cosmetic_tag: the reader MEASURES this; None == "no cosmetic tag set"
            # (a measured absence), a str == the tag. Write it through unconditionally
            # so the measured value (incl. the measured None) is recorded (schema
            # review #2). country.misc is a live-only home, so no save fold reads it.
            if "cosmetic_tag" in rec:
                misc["cosmetic_tag"] = rec["cosmetic_tag"]
        captured_domains.append("country_misc")

    # --- country static identity scalars (live country_static_live) ->
    # country.identity.{original_capital, original_tag, reserved_dynamic_country,
    # templates_locked} (M, distinct live-only leaves). Every key the reader
    # MEASURES is written through (incl. a measured None == "absent", e.g.
    # original_tag None for a non-dynamic country) so the measured value is
    # recorded. country.identity is a live-only home, so no save fold reads it. ---
    cst = captured("country_static")
    if cst is not None:
        for tag, rec in (cst.get("countries") or {}).items():
            if not schema.valid_tag(tag):
                continue
            c = _ensure_country(snap, tag)
            ident = c.get("identity")
            if not isinstance(ident, dict):
                ident = {}
                c["identity"] = ident
            for k in ("original_capital", "original_tag",
                      "reserved_dynamic_country", "templates_locked"):
                if k in rec:
                    ident[k] = rec[k]
        captured_domains.append("country_static")

    # --- country script-variables bag (live variables_live) ->
    # country.script_variables (M, live-only home). The reader is fail-closed
    # (a bag only when key-count == size header), so a {} here is a real
    # measured-empty container, written through unconditionally. ---
    cvar = captured("country_variables")
    if cvar is not None:
        for tag, bag in (cvar.get("countries") or {}).items():
            if not schema.valid_tag(tag):
                continue
            if bag is None:
                continue
            c = _ensure_country(snap, tag)
            c["script_variables"] = dict(bag)
        captured_domains.append("country_variables")

    # --- country container fields (live country_containers_live) ->
    # country.identity.{cores, claims, naval_headquarter_status} +
    # country.decisions.status_present + country.country_reports (M). cores/claims are
    # measured lists (a country with none reads a real []); decision/naval-HQ presence
    # and the report scalars are measured (None == not measured for that country). ---
    cco = captured("country_containers")
    if cco is not None:
        for tag, rec in (cco.get("countries") or {}).items():
            if not schema.valid_tag(tag):
                continue
            c = _ensure_country(snap, tag)
            ident = c.get("identity")
            if not isinstance(ident, dict):
                ident = {}
                c["identity"] = ident
            cores = rec.get("cores")
            if cores is not None:
                ident["cores"] = list(cores)
            claims = rec.get("claims")
            if claims is not None:
                ident["claims"] = list(claims)
            nhq = rec.get("naval_headquarter_status")
            if nhq is not None:                         # measured child count
                ident["naval_headquarter_status"] = nhq
            dsp = rec.get("decision_status_present")
            if dsp is not None:                         # measured presence dict
                dec = c.get("decisions")
                if isinstance(dec, dict):
                    dec["status_present"] = dsp
                else:
                    c["decisions"] = {"status_present": dsp}
            cr = rec.get("country_reports")
            if cr is not None:                          # measured report scalars
                c["country_reports"] = dict(cr)
        captured_domains.append("country_containers")

    # --- tech_live: LIVE research counts + active_research -> country.research (M)
    # tech_count (= completed_tech_count) + research_slots (= research_slot_count)
    # fold onto the SAME save leaves (live-first; protected from save overwrite).
    # active_research goes to a DISTINCT leaf research.in_progress_live (raw-handle
    # shape) so it never mixes with the save-shaped research.in_progress (schema).
    # A measured 0 is informative and kept; tech NAMES + progress SCALE stay null. ---
    tl = captured("tech_live")
    if tl is not None:
        for tag, rec in (tl.get("countries") or {}).items():
            if not schema.valid_tag(tag):
                continue
            c = _ensure_country(snap, tag)
            res = c["research"]
            tc = rec.get("completed_tech_count")
            if tc is not None:
                res["tech_count"] = tc
            rs = rec.get("research_slot_count")
            if rs is not None:
                res["research_slots"] = rs
            # active_research is MEASURED by the reader (incl. a measured-empty []);
            # write it through to the live-only leaf so a measured zero is recorded.
            ar = rec.get("active_research")
            if ar is not None:
                res["in_progress_live"] = list(ar)
        captured_domains.append("tech_live")

    # --- commanders_live: LIVE unit-leaders -> country.commanders (M). The live
    # reader CANNOT split generals vs field-marshals (RESISTED level), so it folds
    # the UNSPLIT army leaders into commanders.army_leaders + admirals into
    # commanders.navy_leaders -- NEVER onto the role-split save leaves. Live counts
    # are written as individual sub-keys of commanders.counts so the save's richer
    # counts dict is preserved (schema). Measured-empty lists are kept (measured 0). ---
    cl = captured("commanders_live")
    if cl is not None:
        for tag, leaders in (cl.get("army_by_tag") or {}).items():
            if not schema.valid_tag(tag):
                continue
            _ensure_country(snap, tag)["commanders"]["army_leaders"] = list(leaders)
        for tag, leaders in (cl.get("navy_by_tag") or {}).items():
            if not schema.valid_tag(tag):
                continue
            _ensure_country(snap, tag)["commanders"]["navy_leaders"] = list(leaders)
        for tag, cnts in (cl.get("counts_by_tag") or {}).items():
            if not schema.valid_tag(tag) or not isinstance(cnts, dict):
                continue
            cmd = _ensure_country(snap, tag)["commanders"]
            cur = cmd.get("counts")
            if not isinstance(cur, dict):
                cur = {}
                cmd["counts"] = cur
            for k in ("army_leaders", "navy_leaders", "characters"):
                if cnts.get(k) is not None:
                    cur[k] = cnts[k]
        captured_domains.append("commanders_live")
        # leader_detail: the OPT-IN skill/traits enrichment joins ONTO the army/navy leader
        # records IN the commanders_live plug (it has ctx.proc + the raw unit_leader_id). The
        # enriched skill/traits keys are therefore ALREADY on the leader dicts folded above; we
        # only need to CLAIM the distinct "leader_detail" provenance id when the enrichment
        # actually ran to completion (the plug sets leader_detail_enriched=True). A disabled/
        # failed enrichment leaves the flag False -> the domain is NOT claimed (the leaders are
        # structurally valid with no skill/traits leaves; no fabrication, schema).
        if cl.get("leader_detail_enriched"):
            captured_domains.append("leader_detail")

    # --- intel_live: LIVE agency scalars -> country.intel.agency.* (M). Folds onto
    # the SAME nested save shape (name/icon/upgrade_progress + capacity.{max_operatives
    # ,usable_slots,defense}) writing ONLY the measured sub-keys, so save-only agency
    # sub-keys (is_created/current_upgrade/upgrades/...) are preserved -- never clobber
    # the whole agency dict (schema). A measured 0 is kept; per-operative data RESISTED. ---
    il = captured("intel_live")
    if il is not None:
        for tag, rec in (il.get("countries") or {}).items():
            if not schema.valid_tag(tag):
                continue
            c = _ensure_country(snap, tag)
            intel = c["intel"]
            ag = intel.get("agency")
            if not isinstance(ag, dict):
                ag = {}
                intel["agency"] = ag
            for src, dst in (("name", "name"), ("icon", "icon"),
                             ("upgrade_progress", "upgrade_progress")):
                v = rec.get(src)
                if v is not None:
                    ag[dst] = v
            cap = ag.get("capacity")
            if not isinstance(cap, dict):
                cap = {}
                ag["capacity"] = cap
            for src, dst in (("max_operative_count", "max_operatives"),
                             ("usable_operative_slots", "usable_slots"),
                             ("defense", "defense")):
                v = rec.get(src)
                if v is not None:
                    cap[dst] = v
        captured_domains.append("intel_live")

    # --- focus_live: LIVE national-focus state -> country.focus (M). Folds onto the
    # SAME save leaves (focus_tree/completed/completed_count/current/progress) under the
    # DISTINCT "focus_live" provenance id, plus the NEW live-only leaf current_continuous.
    # Live WINS (protected via _LIVE_COUNTRY_DOMAIN_LEAVES). progress_days is the same
    # days-elapsed unit as the save `progress`, so it folds onto focus.progress. A
    # measured value (incl. completed_count 0 / a measured-empty completed []) is written
    # through; an UNMEASURED None is left so the save backstop can fill it. paused is
    # RESISTED (the reader's None) and is NEVER written (the save keeps focus.paused). ---
    fl = captured("focus_live")
    if fl is not None:
        # Record the exact TAGs focus_live MEASURED, so the save backstop can protect the
        # measured-empty country.focus.completed [] for these countries WITHOUT also
        # protecting the skeleton [] on a country that was created by another live domain
        # but NOT measured by focus_live (the cross-domain masquerade, schema review). The
        # snapshot keeps this as a non-canonical sidecar consumed by fold_save_bundle.
        _measured = snap.setdefault("_live_measured_leaf_tags", {})
        _focus_completed_tags = set()
        for tag, rec in (fl.get("countries") or {}).items():
            if not schema.valid_tag(tag):
                continue
            c = _ensure_country(snap, tag)
            fc = c["focus"]
            _focus_completed_tags.add(tag)
            # focus_tree / current / current_continuous: a measured str OR a measured
            # None (no active focus) are both real. The reader returns None only when the
            # field is genuinely absent (no tree / no active focus), so write str through;
            # leave a None unwritten so the save backstop can fill focus_tree if it has it.
            ft = rec.get("focus_tree")
            if ft is not None:
                fc["focus_tree"] = ft
            # completed list: a measured [] is a real "zero completed" and must win, so
            # write it through whenever the reader MEASURED it (not None). completed_count
            # likewise (a measured 0 is informative).
            comp = rec.get("completed")
            if comp is not None:
                fc["completed"] = list(comp)
            cc_count = rec.get("completed_count")
            if cc_count is not None:
                fc["completed_count"] = cc_count
            # current national focus: write a measured id; a None is "no active national
            # focus this frame" -> leave the save value (it may name a focus the live read
            # saw as null due to the XOR with current_continuous). Same for continuous.
            cur = rec.get("current")
            if cur is not None:
                fc["current"] = cur
            curc = rec.get("current_continuous")
            if curc is not None:
                fc["current_continuous"] = curc
            # progress in days (same unit as save `progress`); only meaningful when a
            # focus is active, so the reader returns None when neither is set. Write a
            # measured value (incl. a measured 0.0 days = just started) through.
            pd = rec.get("progress_days")
            if pd is not None:
                fc["progress"] = pd
            # paused stays save-sourced (RESISTED live): never written here.
        # Persist the measured-TAG set (sorted list -> JSON-able) for the masquerade
        # guard in fold_save_bundle. Only focus.completed needs it today (the sole
        # protected save-shared leaf with a non-None empty-container skeleton default).
        _measured["focus.completed"] = sorted(_focus_completed_tags)
        captured_domains.append("focus_live")

    # --- production_live: LIVE stockpile + production lines -> country.production (M).
    # A NEW live-only home (the save "production" domain folds into economy/military), so
    # there is no collision. ALWAYS assign each MEASURED leaf -- incl. a measured-empty {}
    # stockpile / [] line list / 0 factory count -- so the live channel wins and a measured
    # zero is recorded as a measured zero, not a gap. Per-line priority / requested /
    # variant-index + construction queue are RESISTED (absent, never fabricated). ---
    pl = captured("production_live")
    if pl is not None:
        for tag, rec in (pl.get("countries") or {}).items():
            if not schema.valid_tag(tag):
                continue
            c = _ensure_country(snap, tag)
            prod = c["production"]
            for src in ("stockpile", "stockpile_total", "military_lines",
                        "naval_lines", "military_factories", "naval_factories"):
                v = rec.get(src)
                # the reader MEASURES all six (the reader returns a concrete value for
                # each, incl. measured-empty {}/[] and measured 0); write each measured
                # value through. An unmeasured None (a key absent) stays the skeleton null.
                if v is not None:
                    if src in ("military_lines", "naval_lines"):
                        prod[src] = list(v)
                    elif src == "stockpile":
                        prod[src] = dict(v)
                    else:
                        prod[src] = v
        captured_domains.append("production_live")

    # --- diplomacy_live: LIVE per-country diplomacy -> country.diplomacy (M) + the world
    # sidecar snap["diplomacy_live"]. Per-country at_war/casualties/faction_group fold onto
    # the country (DISTINCT live-only leaves from the save-derived diplomacy.wars). at_war is
    # MEASURED (incl. a measured-empty [] = at peace) and written through; casualties (a
    # measured 0 = no casualties) + faction_group (an OPAQUE engine group id, an int NOT a
    # TAG) likewise. The world-scoped war_relations + faction_members go to the NEW top-level
    # sidecar -- NOT diplomacy_world (a SAVE-shaped catalog with a different identity model;
    # schema). EVERY enemy/member TAG was already valid_tag-filtered by the reader. ---
    dl = captured("diplomacy_live")
    if dl is not None:
        for tag, rec in (dl.get("countries") or {}).items():
            if not schema.valid_tag(tag):
                continue
            c = _ensure_country(snap, tag)
            dip = c["diplomacy"]
            # at_war: a MEASURED list (incl. measured-empty []); only write when the reader
            # MEASURED it (the key present), else leave the skeleton None (unmeasured). Defense
            # in depth: re-filter each enemy through valid_tag so no non-TAG can land here.
            aw = rec.get("at_war")
            if isinstance(aw, list):
                dip["at_war"] = [e for e in aw if schema.valid_tag(e)]
            cv = rec.get("casualties")
            if cv is not None:
                dip["casualties"] = cv
            fg = rec.get("faction_group")
            if fg is not None:
                dip["faction_group"] = fg
        # world sidecar: war_relations list + faction_members (group_id -> [TAGs]) + the
        # completeness advisory. ALWAYS assign the measured lists/dicts (incl. measured-empty)
        # so the live channel wins and a measured zero is recorded as a measured zero (the
        # domain_captured gate already proves the reader ran to completion -> the reader
        # always yields concrete containers here). DEFENSE IN DEPTH (schema final review):
        # re-validate EVERY TAG at the assembler boundary too -- a war_relation with a
        # non-TAG end is dropped, and a faction member that is not a TAG is filtered out, so
        # a misbehaving reader can never let a numeric/None id masquerade as identity in the
        # persisted sidecar (mirrors the state_control / naval_air boundary sanitization).
        dl_side = snap.get("diplomacy_live")
        if not isinstance(dl_side, dict):
            dl_side = schema.empty_diplomacy_live()
            snap["diplomacy_live"] = dl_side
        clean_wr = []
        for w in (dl.get("war_relations") or []):
            if not isinstance(w, dict):
                continue
            if not (schema.valid_tag(w.get("first")) and schema.valid_tag(w.get("second"))):
                continue
            clean_wr.append({
                "first": w.get("first"), "second": w.get("second"),
                "first_casualties": w.get("first_casualties"),
                "second_casualties": w.get("second_casualties"),
            })
        clean_fm = {}
        for gid, members in (dl.get("faction_members") or {}).items():
            if not isinstance(members, (list, tuple)):
                continue
            # key is the OPAQUE numeric group id (stringified); each member must be a TAG.
            clean_fm[str(gid)] = sorted(m for m in members if schema.valid_tag(m))
        dl_side["war_relations"] = clean_wr
        dl_side["faction_members"] = clean_fm
        dl_side["subtype_warning"] = dict(dl.get("subtype_warning") or {})
        dl_side["source"] = "M"
        captured_domains.append("diplomacy_live")

    # --- diplo_relations: LIVE bilateral relations -> country.diplomacy (M). PER-COUNTRY
    # (schema-approved; the bilateral matrix is the SAME {TGT: value} per-country shape the save
    # opinion leaf uses, so no top-level-sidecar identity duplication). `opinion` folds LIVE-FIRST
    # onto the save-shared diplomacy.opinion leaf ({TGT: net_opinion}); `relation_flags`
    # ({TGT: [flag,...]}) + overlord/autonomy_state/autonomy_ratio are DISTINCT live-only leaves.
    # ALL of opinion/relation_flags/overlord/autonomy_* are protected via _LIVE_COUNTRY_DOMAIN_
    # LEAVES. A measured-empty {} opinion/relation_flags is NOT emitted by the plug (it only keys
    # a country that carries a measured leaf), so an unmeasured country keeps the None skeleton
    # and the save backstop fills it. EVERY TAG was already valid_tag-filtered by the reader. ---
    dr = captured("diplo_relations")
    if dr is not None:
        for tag, rec in (dr.get("countries") or {}).items():
            if not schema.valid_tag(tag):
                continue
            c = _ensure_country(snap, tag)
            dip = c["diplomacy"]
            # opinion: a MEASURED {TGT: net_opinion} dict. Defense in depth -- re-filter each
            # target through valid_tag so no non-TAG key can land here. Only write when the
            # reader MEASURED it (the key present), else leave the skeleton None (save fills).
            op = rec.get("opinion")
            if isinstance(op, dict):
                dip["opinion"] = {t: v for t, v in op.items() if schema.valid_tag(t)}
            rf = rec.get("relation_flags")
            if isinstance(rf, dict):
                dip["relation_flags"] = {t: list(v) for t, v in rf.items()
                                         if schema.valid_tag(t) and isinstance(v, list)}
            # subject/autonomy: overlord is a TAG (or None); autonomy_state a str; ratio a float.
            ov = rec.get("overlord")
            if ov is not None and schema.valid_tag(ov):
                dip["overlord"] = ov
            ast = rec.get("autonomy_state")
            if ast is not None:
                dip["autonomy_state"] = ast
            ratio = rec.get("autonomy_ratio")
            if ratio is not None:
                dip["autonomy_ratio"] = ratio
        captured_domains.append("diplo_relations")

    # --- politics_live: LIVE per-country politics -> country.politics (M). Folds onto the
    # SAME save leaves (party_popularities/ruling_party) under the DISTINCT "politics_live"
    # provenance id, plus the NEW live leaves ideas/ideas_other/national_spirits/laws/
    # idea_count/election_frequency/party_count/low_confidence. political_power is RESISTED here
    # (country_scalars owns it; we never double-write the protected PP leaf). A measured
    # value (incl. a measured-empty ideas [] / ideas_other [] / a measured 0 popularity)
    # is written through; an UNMEASURED None is left so the save backstop can fill it
    # (ruling_party/party_popularities/laws/ideas_other are save-shared). ---
    polv = captured("politics_live")
    if polv is not None:
        _ideas_other_tags = set()
        _national_spirit_tags = set()
        for tag, rec in (polv.get("countries") or {}).items():
            if not schema.valid_tag(tag):
                continue
            c = _ensure_country(snap, tag)
            pol = c["politics"]
            # party_popularities + ruling_party fold onto the save-shared leaves: a measured
            # dict / a measured str wins; an unmeasured None is left for the save backstop.
            pops = rec.get("popularities")
            if pops is not None:
                pol["party_popularities"] = dict(pops)
            rp = rec.get("ruling_party")
            if rp is not None:
                pol["ruling_party"] = rp
            # ideas: a MEASURED ordered list (incl. measured-empty []) -> live-only leaf.
            ideas = rec.get("ideas")
            if ideas is not None:
                pol["ideas"] = list(ideas)
            # ideas_other: live-classified non-law active ideas. This leaf's schema default
            # is [], so measured-empty [] needs a measured-TAG sidecar before save backstop
            # can safely protect it from a stale save list.
            ideas_other = rec.get("ideas_other")
            if isinstance(ideas_other, list):
                pol["ideas_other"] = list(ideas_other)
                _ideas_other_tags.add(tag)
            national_spirits = rec.get("national_spirits")
            if isinstance(national_spirits, list):
                pol["national_spirits"] = list(national_spirits)
                _national_spirit_tags.add(tag)
            # the remaining live-only scalars: a measured value (incl. 0 / False) is written.
            for src, dst in (("idea_count", "idea_count"),
                             ("election_frequency", "election_frequency"),
                             ("party_count", "party_count"),
                             ("low_confidence", "low_confidence")):
                v = rec.get(src)
                if v is not None:
                    pol[dst] = v
            # laws: the typed {economy/trade_laws/mobilization_laws} triple classified from the
            # MEASURED idea tokens by defnames (M, additive enrichment; present only when the
            # name-resolution opt-in ran). Deliberately NOT protected as a whole leaf: laws
            # defaults to {}, so a live empty dict must never overwrite save-derived laws.
            laws = rec.get("laws")
            if isinstance(laws, dict):
                pol["laws"] = dict(laws)
            idea_groups = rec.get("idea_groups")
            if isinstance(idea_groups, dict):
                pol["idea_groups"] = {
                    group: list(tokens)
                    for group, tokens in idea_groups.items()
                    if isinstance(group, str) and isinstance(tokens, list)
                }
            advisor_ideas = rec.get("advisor_ideas")
            if isinstance(advisor_ideas, dict):
                pol["advisor_ideas"] = {
                    group: list(tokens)
                    for group, tokens in advisor_ideas.items()
                    if isinstance(group, str) and isinstance(tokens, list)
                }
            # political_power: NOT written here (country_scalars owns the protected leaf).
        if _ideas_other_tags:
            _measured = snap.setdefault("_live_measured_leaf_tags", {})
            _measured["politics.ideas_other"] = sorted(_ideas_other_tags)
        if _national_spirit_tags:
            _measured = snap.setdefault("_live_measured_leaf_tags", {})
            _measured["politics.national_spirits"] = sorted(_national_spirit_tags)
        captured_domains.append("politics_live")

    # --- fuel_live: LIVE per-country national FUEL -> country.economy.fuel.{current,max} (M).
    # Folds onto the SAME save-shared home (economy.fuel is a dict defaulting to {}) under the
    # DISTINCT "fuel_live" provenance id (live-first; protected via _LIVE_COUNTRY_DOMAIN_LEAVES).
    # We write the INDIVIDUAL scalar leaves current/max -- whose own skeleton default is absent
    # (the dict is {}), so the `lv is not None` guard in _merge_country suffices and no
    # measured-empty container guard is needed. A measured value (incl. a
    # measured 0.0 = empty tank) is written through with a `is not None` guard, NEVER truthiness;
    # an unmeasured None is left so the save backstop can fill it. NO save-relative range gate on
    # current fuel (it legitimately reads below the save: live post-clamp + live!=save frame). ---
    flv = captured("fuel_live")
    if flv is not None:
        for tag, rec in (flv.get("countries") or {}).items():
            if not schema.valid_tag(tag):
                continue
            c = _ensure_country(snap, tag)
            econ = c["economy"]
            fuel = econ.get("fuel")
            if not isinstance(fuel, dict):
                fuel = {}
                econ["fuel"] = fuel
            cur = rec.get("fuel")
            if cur is not None:
                fuel["current"] = cur
            mx = rec.get("max_fuel")
            if mx is not None:
                fuel["max"] = mx
        captured_domains.append("fuel_live")

    # --- state detail (live manpower pool / occupation / category) -> states ---
    # Folds manpower_pool {available, locked, total}, occupation {resistance,
    # compliance}, and category onto each measured state. owner/controller are NOT
    # touched here (state_control owns controller; owner is M/S). A measured 0 in a
    # manpower/occupation leaf is informative and kept (no fabrication).
    sd = captured("state_detail")
    if sd is not None:
        for sid, rec in (sd.get("states") or {}).items():
            st = _ensure_state(snap, sid)
            mp = {}
            for src, dst in (("manpower_available", "available"),
                             ("manpower_locked", "locked"),
                             ("manpower_total", "total")):
                v = rec.get(src)
                if v is not None:
                    mp[dst] = v
            if mp:
                st["manpower_pool"] = mp
            occ = {}
            for k in ("resistance", "compliance"):
                v = rec.get(k)
                if v is not None:
                    occ[k] = v
            if occ:
                # merge into any existing occupation object rather than clobber it.
                cur = st.get("occupation")
                if isinstance(cur, dict):
                    cur.update(occ)
                else:
                    st["occupation"] = occ
            cat = rec.get("state_category")
            if cat is not None:
                st["category"] = cat
        captured_domains.append("state_detail")

    # --- state VP (live per-state victory points) -> states (M). Folds onto the
    # SAME canonical save leaves states.*.victory_points_total + .victory_points_by_province
    # under the DISTINCT "state_vp" provenance id (live wins; protected via
    # _LIVE_STATE_DOMAIN_LEAVES in fold_save_bundle). The plug enumerates the FULL state
    # asset, so EVERY measured state carries a victory_points_total (a measured 0 for a
    # zero-VP state, which must beat a stale save nonzero) + a victory_points_by_province
    # map (a measured-empty {} = no VP-bearing province; it REPLACES the save map, never
    # key-merges it). Record the measured state set so fold_save_bundle re-asserts these
    # leaves ONLY on the states this plug actually measured (no cross-domain skeleton
    # masquerade). vp/by_province are raw ints (NOT tags). ---
    vp = captured("state_vp")
    if vp is not None:
        _measured = snap.setdefault("_live_measured_leaf_tags", {})
        _vp_states = set()
        for sid, rec in (vp.get("states") or {}).items():
            st = _ensure_state(snap, sid)
            _vp_states.add(str(sid))
            total = rec.get("victory_points_total")
            if total is not None:                       # measured (incl. 0) -> write
                st["victory_points_total"] = total
            byp = rec.get("victory_points_by_province")
            if byp is not None:                         # measured (incl. {}) -> write
                st["victory_points_by_province"] = dict(byp)
        # the VP plug measures BOTH leaves on every emitted state, so a single measured
        # state set covers both protected leaves.
        _vp_sorted = sorted(_vp_states)
        _measured["states.victory_points_total"] = _vp_sorted
        _measured["states.victory_points_by_province"] = _vp_sorted
        captured_domains.append("state_vp")

    # --- state extra fields (live state_fields_live) -> states.*.{resources,
    # dynamic_modifier, extra_shared_slots} (M). resources shares the save home
    # (live-first; protected via _LIVE_STATE_DOMAIN_LEAVES, re-asserted only on the
    # states this plug measured); dynamic_modifier + extra_shared_slots are distinct
    # live-only leaves. Each value is a REAL read of the located container (a
    # measured-empty {} / [] = no resources / no modifiers; it REPLACES, never
    # key-merges, the save value). ---
    stf = captured("state_fields")
    if stf is not None:
        _measured = snap.setdefault("_live_measured_leaf_tags", {})
        _sf_states = set()
        for sid, rec in (stf.get("states") or {}).items():
            st = _ensure_state(snap, sid)
            _sf_states.add(str(sid))
            res = rec.get("resources")
            if res is not None:                         # measured (incl. {}) -> write
                st["resources"] = dict(res)
            dm = rec.get("dynamic_modifier")
            if dm is not None:                          # measured (incl. []) -> write
                st["dynamic_modifier"] = list(dm)
            ess = rec.get("extra_shared_slots")
            if ess is not None:                         # measured int (incl. 0) -> write
                st["extra_shared_slots"] = ess
        _sf_sorted = sorted(_sf_states)
        # only the SAVE-shared leaf needs re-assertion protection; the live-only
        # leaves never collide with a save value.
        _measured["states.resources"] = _sf_sorted
        captured_domains.append("state_fields")

    # When the divisions domain WAS captured, a known country with no division is
    # a MEASURED zero, not unmeasured -> division_count = 0 (schema review #4).
    # Only a country we actually know about (it appears in the snapshot) gets the
    # measured zero; we do not invent countries just to zero them.
    if dv is not None:
        for tag, c in snap["countries"].items():
            mil = c["military"]
            if mil.get("division_count") is None:
                mil["division_count"] = 0

    # --- designs: LIVE division templates -> designs.division_templates (M).
    # LIVE wins over the save backstop (the save fold only fills when live left it
    # empty). Equipment variants are NOT exposed by the live reader yet, so we leave
    # designs.equipment_variants untouched (the save backstop fills it). ---
    dsg = captured("designs")
    if dsg is not None:
        # ALWAYS assign the LIVE list (even a measured-empty []), so the save
        # backstop can never overlay templates while we badge them "M" (schema
        # review #1: an empty live read is a measured zero, not a gap to fill).
        snap["designs"]["division_templates"] = list(
            dsg.get("division_templates") or [])
        snap["designs"]["division_templates_source"] = "M"
        # honesty: record that the live variant catalog is not yet available, so a
        # consumer knows equipment_variants (if any) came from the save backstop.
        snap["designs"]["live_variants_available"] = bool(
            dsg.get("variants_available"))
        captured_domains.append("designs")

    # --- live MIO catalog -> top-level snap["mio_live"] (M). Distinct from the
    # save-sourced per-country Country.mios, so no collision with the save fold. ---
    mio = captured("mio")
    if mio is not None:
        ml = snap.get("mio_live")
        if not isinstance(ml, dict):
            ml = schema.empty_mio_live()
            snap["mio_live"] = ml
        ml["by_tag"] = mio.get("by_tag")
        ml["by_id"] = mio.get("by_id")
        ml["count"] = mio.get("count")
        ml["source"] = "M"
        captured_domains.append("mio")

    # --- live special-projects / facilities / scientists -> top-level
    # snap["special_projects"] (M). Distinct from the SAVE-sourced per-country
    # Country.projects / Country.scientists (different shape + provenance), so no
    # collision with the save fold. The reader exposes NO owner TAG, so the pools are
    # world-scoped GLOBAL lists -- not keyed by TAG (per-country attribution is an
    # open ticket, never fabricated). ALWAYS assign the LIVE lists (incl. a measured-
    # empty []), so the save backstop can never overlay them while badged M and so a
    # measured-empty pool is recorded as a measured zero, not a gap. ---
    proj = captured("projects_live")
    if proj is not None:
        sp = snap.get("special_projects")
        if not isinstance(sp, dict):
            sp = schema.empty_special_projects()
            snap["special_projects"] = sp
        sp["facilities"] = list(proj.get("facilities") or [])
        sp["projects"] = list(proj.get("projects") or [])
        sp["scientists"] = list(proj.get("scientists") or [])
        sp["count"] = proj.get("count")
        sp["source"] = "M"
        captured_domains.append("projects_live")

    # --- live per-ship roster -> order_of_battle.ships (M). owner is absent live
    # (it comes from the save OOB join), so each ship carries no owner here. ---
    ships = captured("ships")
    if ships is not None:
        oob = snap.get("order_of_battle")
        if not isinstance(oob, dict):
            oob = schema.empty_order_of_battle()
            snap["order_of_battle"] = oob
        oob["ships"] = list(ships.get("ships") or [])
        oob["ships_source"] = "M"
        captured_domains.append("ships")

    # --- live OOB command tree -> order_of_battle.{armies,army_groups,fleets,theatres}
    # (M). World-scoped command structure (schema-approved top-level home, NOT by_country);
    # each army/AG/fleet element carries its owner-CONSENSUS TAG, membership ids are raw
    # engine ints, task forces nest under each fleet. The per-SHIP order_of_battle.ships
    # leaf is owned by the "ships" domain and is NEVER touched here -- we add the command-
    # group structure ALONGSIDE it on the SAME order_of_battle dict. ALWAYS assign each
    # MEASURED list (incl. a measured-empty []) so the live channel wins and a measured zero
    # is recorded as a measured zero; the matching <list>_source = "M" flag is set so the
    # save backstop (M/S) is known to have been overridden. navy_theaters/air_wings stay
    # save-sourced (not measured live). ---
    oc = captured("oob_command")
    if oc is not None:
        oob = snap.get("order_of_battle")
        if not isinstance(oob, dict):
            oob = schema.empty_order_of_battle()
            snap["order_of_battle"] = oob
        # FAIL-CLOSED (schema final review): a CAPTURED oob_command must carry a concrete
        # list for every command leaf. A missing / non-list leaf is a contract violation,
        # NOT a measured-empty -> fold NOTHING and do not claim the domain rather than
        # fabricating a measured-empty [] from a malformed read. The reader always builds
        # these lists, so this only trips on a genuinely broken result.
        _leaves = ("armies", "army_groups", "fleets", "theatres")
        if all(isinstance(oc.get(src), list) for src in _leaves):
            for src in _leaves:
                oob[src] = list(oc[src])
                oob[src + "_source"] = "M"
            # orphan task forces (live TF whose fleet parent missed this frame) are a
            # MEASURED observation, not an error; keep them in a sidecar on the OOB dict so
            # they are not silently discarded (schema finding #1). Only set when measured
            # (a list); absent -> leave it unset (validator-inert extra key).
            otf = oc.get("orphan_task_forces")
            if isinstance(otf, list):
                oob["orphan_task_forces"] = list(otf)
            captured_domains.append("oob_command")

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
    # --- live supply SOLVER INPUTS -> ADDITIONAL badged fields on the supply block ----------
    # The exact per-province "supply available" number is provably NOT stored on this build
    # (supply_solver_live.province_supply_exact()->{} honestly), so we DO NOT claim it. What
    # IS stored are the per-province solver INPUTS -- efficiency/fulfilment + draw/demand --
    # which we surface as DISTINCTLY-NAMED, SEPARATELY-BADGED fields ALONGSIDE the existing
    # division-province proxy WITHOUT touching it (schema: badge harder, clearer names, don't
    # let the proxy's confidence apply to these). Each field carries its OWN confidence so a
    # consumer never reads efficiency/draw as "supply available". The supply block is added
    # even if the proxy ("supply") domain did NOT capture (the inputs stand on their own).
    si = captured("supply_inputs")
    if si is not None:
        snap["_live_supply_inputs"] = si
        sblk = snap.get("supply")
        if not isinstance(sblk, dict):
            sblk = schema.empty_supply()
            snap["supply"] = sblk
        # ALWAYS assign each measured map (incl. a measured-sparse {} -- the reader returns
        # only nonzero entries) so the live channel wins; an unmeasured None stays None. Copy
        # the maps so the typed supply block never ALIASES the _live_supply_inputs sidecar's
        # dict objects (a later normalization of one must not mutate the other -- schema #4).
        eff = si.get("efficiency_by_province")
        draw = si.get("draw_by_province")
        if eff is not None:
            sblk["solver_efficiency_by_province"] = dict(eff) if isinstance(eff, dict) else eff
        if draw is not None:
            sblk["solver_draw_by_province"] = dict(draw) if isinstance(draw, dict) else draw
        # per-field source + confidence so the solver-input signal is never mistaken for the
        # exact per-province supply (NOT the block-level proxy badge -- schema).
        sblk["solver_inputs_source"] = "M"
        sblk["solver_inputs_confidence"] = "live_solver_input_not_exact_supply"
        captured_domains.append("supply_inputs")

    # --- world_meta: LIVE scenario DATES -> world.scenario.{current_date_live,start_date_live}
    # (M). DISTINCT _live keys under the save-shaped scenario block (source S), so the live read
    # never overwrites a save scenario field. Only write a MEASURED date (a str); a None means
    # the date did not decode -> leave the key absent (no fabrication). world_tension is
    # COMPUTED-not-stored and is NEVER written here. ---
    wm = captured("world_meta")
    if wm is not None:
        world = snap.get("world")
        if not isinstance(world, dict):
            world = schema.empty_world()
            snap["world"] = world
        scen = world.get("scenario")
        if not isinstance(scen, dict):
            scen = {}
            world["scenario"] = scen
        cur = wm.get("current_date_live")
        if cur is not None:
            scen["current_date_live"] = cur
        start = wm.get("start_date_live")
        if start is not None:
            scen["start_date_live"] = start
        captured_domains.append("world_meta")

    # --- construction_orders: LIVE construction queue + battle-plan orders + opinion-modifier
    # decomposition (M). Three DISTINCT live-only homes:
    #   * construction -> country.production.construction_queue (only TAGs the reader MEASURED;
    #     a measured-empty queue is NOT emitted by the plug, so we never write a skeleton []).
    #   * orders -> order_of_battle.battle_plan_orders (a world list; obj heap addr already
    #     stripped by the plug). ALWAYS assign the measured list (incl. measured-empty []).
    #   * opinion_modifiers -> country.diplomacy.opinion_modifiers ({TGT: [{token_id, value}]}).
    # Record the measured country sets so fold_save_bundle protects the live-only leaves. ---
    co = captured("construction_orders")
    if co is not None:
        _constr_tags = set()
        for tag, lines in (co.get("construction") or {}).items():
            if not schema.valid_tag(tag) or not isinstance(lines, list):
                continue
            c = _ensure_country(snap, tag)
            c["production"]["construction_queue"] = list(lines)
            _constr_tags.add(tag)
        _om_tags = set()
        for tag, mods in (co.get("opinion_modifiers") or {}).items():
            if not schema.valid_tag(tag) or not isinstance(mods, dict):
                continue
            c = _ensure_country(snap, tag)
            # re-validate each target TAG at the assembler boundary (defense in depth).
            c["diplomacy"]["opinion_modifiers"] = {
                t: list(v) for t, v in mods.items()
                if schema.valid_tag(t) and isinstance(v, list)}
            _om_tags.add(tag)
        orders = co.get("orders")
        if isinstance(orders, list):
            oob = snap.get("order_of_battle")
            if not isinstance(oob, dict):
                oob = schema.empty_order_of_battle()
                snap["order_of_battle"] = oob
            # each order: {army_id(int), order_type, target_states(list|None)}; target_states is
            # a measured type-5 set or RESISTED None (never fabricated []).
            oob["battle_plan_orders"] = [
                {"army_id": o.get("army_id"), "order_type": o.get("order_type"),
                 "target_states": (list(o["target_states"])
                                   if isinstance(o.get("target_states"), list) else None)}
                for o in orders if isinstance(o, dict)]
            oob["battle_plan_orders_source"] = "M"
        if _constr_tags or _om_tags:
            _measured = snap.setdefault("_live_measured_leaf_tags", {})
            if _constr_tags:
                _measured["production.construction_queue"] = sorted(_constr_tags)
            if _om_tags:
                _measured["diplomacy.opinion_modifiers"] = sorted(_om_tags)
        captured_domains.append("construction_orders")

    # --- decisions_market: LIVE active timed DECISIONS -> country.decisions.timed_live (M, a
    # DISTINCT live-only leaf) + the global equipment-MARKET contracts -> the NEW top-level
    # snap["market_live"] sidecar (M). The plug emits only TAGs it MEASURED for decisions (a
    # measured-empty timed_live [] = no active timed decision is a real value and is written +
    # protected). market contracts already have raw ids/ptrs stripped + seller/buyer resolved. ---
    dmk = captured("decisions_market")
    if dmk is not None:
        _dec_tags = set()
        for tag, timed in (dmk.get("decisions") or {}).items():
            if not schema.valid_tag(tag) or not isinstance(timed, list):
                continue
            _ensure_country(snap, tag)["decisions"]["timed_live"] = list(timed)
            _dec_tags.add(tag)
        if _dec_tags:
            _measured = snap.setdefault("_live_measured_leaf_tags", {})
            _measured["decisions.timed_live"] = sorted(_dec_tags)
        market = dmk.get("market")
        mkl = snap.get("market_live")
        if not isinstance(mkl, dict):
            mkl = schema.empty_market_live()
            snap["market_live"] = mkl
        # ALWAYS assign the measured list (incl. measured-empty []); re-validate seller/buyer.
        clean_contracts = []
        for ctr in (market or []):
            if not isinstance(ctr, dict):
                continue
            s = ctr.get("seller")
            b = ctr.get("buyer")
            clean_contracts.append({
                "contract_id": ctr.get("contract_id"),
                "seller": s if schema.valid_tag(s) else None,
                "buyer": b if schema.valid_tag(b) else None,
                "equip_variant_id": ctr.get("equip_variant_id"),
                "amount": ctr.get("amount"),
                "price": ctr.get("price"),
            })
        mkl["contracts"] = clean_contracts
        mkl["count"] = len(clean_contracts)
        mkl["source"] = "M"
        captured_domains.append("decisions_market")

    # --- operative_xp: LIVE operative SKILL + XP -> country.intel.operative_skill_xp (M, a
    # DISTINCT live-only leaf {op_id: {name, skill, experience}}) + tech in-progress UNITS MERGED
    # onto the existing research.in_progress_live slots (by slot_index) -- progress_units is added
    # ALONGSIDE progress_raw, NEVER replacing the slot list (so tech_live's tech_handle_raw/
    # tech_def_index_raw/tech_name survive). The completion FRACTION is RESISTED (never computed).
    # A skill/experience that read None per operative stays None (a guarded miss, not 0). ---
    oxp = captured("operative_xp")
    if oxp is not None:
        _osx_tags = set()
        for tag, ops in (oxp.get("operative_skill_xp") or {}).items():
            if not schema.valid_tag(tag) or not isinstance(ops, dict):
                continue
            _ensure_country(snap, tag)["intel"]["operative_skill_xp"] = dict(ops)
            _osx_tags.add(tag)
        if _osx_tags:
            _measured = snap.setdefault("_live_measured_leaf_tags", {})
            _measured["intel.operative_skill_xp"] = sorted(_osx_tags)
        # MERGE progress_units onto research.in_progress_live by slot_index (do NOT replace the
        # list; the slots came from tech_live and carry the raw handles + tech_name).
        for tag, slot_units in (oxp.get("tech_progress_units") or {}).items():
            if not schema.valid_tag(tag) or not isinstance(slot_units, dict):
                continue
            c = snap["countries"].get(tag)
            if c is None:
                continue  # only enrich a country tech_live already created (no fabrication)
            slots = c["research"].get("in_progress_live")
            if not isinstance(slots, list):
                continue
            for slot in slots:
                if not isinstance(slot, dict):
                    continue
                si = slot.get("slot_index")
                if si is not None and int(si) in slot_units:
                    slot["progress_units"] = slot_units[int(si)]
        captured_domains.append("operative_xp")

    # --- politics_residuals: LIVE election residuals + appointed advisors -> country.politics.
    # {last_election,elections_allowed,appointed_advisors} (M, DISTINCT live-only leaves). Only
    # the MEASURED leaves are written (the plug already drops a None); appointed_advisors is a
    # list (a measured-empty [] = no advisor hired is real). 46/46 EXACT vs save. ---
    pr = captured("politics_residuals")
    if pr is not None:
        _pr_tags = set()
        for tag, rec in (pr.get("countries") or {}).items():
            if not schema.valid_tag(tag) or not isinstance(rec, dict):
                continue
            pol = _ensure_country(snap, tag)["politics"]
            if rec.get("last_election") is not None:
                pol["last_election"] = rec["last_election"]
            if rec.get("elections_allowed") is not None:
                pol["elections_allowed"] = rec["elections_allowed"]
            if isinstance(rec.get("appointed_advisors"), list):
                pol["appointed_advisors"] = list(rec["appointed_advisors"])
            if isinstance(rec.get("appointed_advisor_details"), list):
                pol["appointed_advisor_details"] = [
                    dict(row) for row in rec["appointed_advisor_details"]
                    if isinstance(row, dict)
                ]
            if isinstance(rec.get("appointed_advisor_slots"), dict):
                pol["appointed_advisor_slots"] = {
                    str(slot): list(tokens)
                    for slot, tokens in rec["appointed_advisor_slots"].items()
                    if isinstance(tokens, list)
                }
            if isinstance(rec.get("appointed_advisor_ledgers"), dict):
                pol["appointed_advisor_ledgers"] = {
                    str(ledger): list(tokens)
                    for ledger, tokens in rec["appointed_advisor_ledgers"].items()
                    if isinstance(tokens, list)
                }
            _pr_tags.add(tag)
        if _pr_tags:
            _measured = snap.setdefault("_live_measured_leaf_tags", {})
            # appointed_advisors is the only one whose live-only default could read as a
            # skeleton container; but the leaf default is None (Optional), so the None-guard
            # suffices. We still record the measured set for fold-bundle protection symmetry.
            _measured["politics.appointed_advisors"] = sorted(_pr_tags)
            _measured["politics.appointed_advisor_details"] = sorted(_pr_tags)
            _measured["politics.appointed_advisor_slots"] = sorted(_pr_tags)
            _measured["politics.appointed_advisor_ledgers"] = sorted(_pr_tags)
        captured_domains.append("politics_residuals")

    # --- intel_deep: LIVE operative roster + cryptology -> country.intel.{operatives_live,
    # cryptology_live} (M, DISTINCT live-only leaves from the save-shaped intel.operatives). The
    # plug already grouped by owner TAG + stripped the diagnostic addrs/raw ids. A country with
    # measured operatives/crypto gets the list/dict; an unmeasured one keeps None. ---
    idp = captured("intel_deep")
    if idp is not None:
        _id_tags = set()
        for tag, ops in (idp.get("operatives_by_tag") or {}).items():
            if not schema.valid_tag(tag) or not isinstance(ops, list):
                continue
            _ensure_country(snap, tag)["intel"]["operatives_live"] = list(ops)
            _id_tags.add(tag)
        for tag, crypto in (idp.get("cryptology_by_tag") or {}).items():
            if not schema.valid_tag(tag) or not isinstance(crypto, dict):
                continue
            # re-validate each target TAG key at the boundary (defense in depth).
            _ensure_country(snap, tag)["intel"]["cryptology_live"] = {
                t: v for t, v in crypto.items() if schema.valid_tag(t)}
            _id_tags.add(tag)
        if _id_tags:
            _measured = snap.setdefault("_live_measured_leaf_tags", {})
            _measured["intel.operatives_live"] = sorted(
                t for t in (idp.get("operatives_by_tag") or {}) if schema.valid_tag(t))
        captured_domains.append("intel_deep")

    # --- buildings_live: LIVE per-state + per-province BUILDING levels -> states.*.buildings +
    # provinces.*.buildings (M; SHARES the save home, live-first). The state home is save-shared
    # (its default is None), so a live measured {} wins via the _backstop_merge None-guard; we
    # also record the measured state set so fold_save_bundle protects it. Province buildings is a
    # live-only leaf (None default). Keys are raw ids (NOT tags). ---
    bl = captured("buildings_live")
    if bl is not None:
        _bld_states = set()
        for sid, b in (bl.get("states") or {}).items():
            if not isinstance(b, dict):
                continue
            _ensure_state(snap, sid)["buildings"] = dict(b)
            _bld_states.add(str(sid))
        for pid, b in (bl.get("provinces") or {}).items():
            if not isinstance(b, dict):
                continue
            prv = snap["provinces"].get(pid)
            if prv is None:
                prv = schema.empty_province()
                snap["provinces"][pid] = prv
            prv["buildings"] = dict(b)
        if _bld_states:
            _measured = snap.setdefault("_live_measured_leaf_tags", {})
            _measured["states.buildings"] = sorted(_bld_states)
        captured_domains.append("buildings_live")

    # --- designs_variants: LIVE equipment-VARIANT module catalog -> designs.equipment_variants_live
    # (M, a DISTINCT live-only leaf keyed by the bare live variant id). Kept SEPARATE from the save
    # designs.equipment_variants (keyed "id:type") so the two key spaces never collide and the
    # save catalog is never clobbered/discarded (schema review #3). ALWAYS assign the measured
    # catalog (the plug raises on an empty read, so a captured result is a real non-empty
    # catalog); badge equipment_variants_live_source = "M". ---
    dvr = captured("designs_variants")
    if dvr is not None:
        ev = dvr.get("equipment_variants")
        if isinstance(ev, dict):
            snap["designs"]["equipment_variants_live"] = dict(ev)
            snap["designs"]["equipment_variants_live_source"] = "M"
        captured_domains.append("designs_variants")

    # --- division_equipment: LIVE per-division held-equipment inventory -> divisions[].
    # equipment_live (M, a DISTINCT live-only leaf, NOT the save-owned `equipment` list -- so the
    # live join can NEVER clobber a richer save equipment shape, schema review). The plug keyed by
    # "TAG|div_id"; we join onto the snapshot divisions list by (owner, div_id). A division with
    # no live equipment match keeps equipment_live None. Variant ids are raw ints (NOT tags). ---
    de = captured("division_equipment")
    if de is not None:
        by_div = de.get("by_div") or {}
        if by_div:
            for d in (snap.get("divisions") or []):
                if not isinstance(d, dict):
                    continue
                owner = d.get("owner")
                did = d.get("div_id")
                if not (schema.valid_tag(owner) and isinstance(did, int)):
                    continue
                rec = by_div.get("%s|%d" % (owner, did))
                if isinstance(rec, dict) and isinstance(rec.get("equipment"), dict):
                    # live {variant_id(str): count} -> the DISTINCT equipment_live leaf (dict),
                    # leaving the save-owned `equipment` list untouched (no clobber).
                    d["equipment_live"] = dict(rec["equipment"])
        captured_domains.append("division_equipment")

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
    # tags) and pass through unresolved (schema Q1/Q4).
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
    # (schema Q2). These are NOT memory-sourced, so they are tracked in a SEPARATE
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

    # --- WAVE-2 live readers (weather temperature + naval combat stats) ---------
    # Each is guarded so the offline snapshot-build path (no running game) NEVER
    # crashes: a missing pid / import / live read returns None/empty and the domain
    # is simply not claimed (no fabrication). pid is the resolved live process id.
    pid = (identity.pid if identity is not None else None)
    if _wire_weather(snap, pid):
        captured_domains.append("weather")
    # extended per-region weather + per-province mud/custom-modifiers (validated read).
    # Runs AFTER _wire_weather so it merges onto the province rows it created and fills
    # the region block _wire_weather left null.
    if _wire_weather_fields(snap, pid):
        if "weather" not in captured_domains:
            captured_domains.append("weather")
        captured_domains.append("weather_fields")
    if _wire_naval_combat_stats(snap, pid):
        captured_domains.append("naval_air_combat_stats")

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
    # --- provenance.mod: the per-replay MOD IDENTITY (§K) --------------------
    # Read from the host launcher config ON DISK at capture time (NOT memory/save).
    # `None` when no launcher config was found (no fabrication); a dict otherwise
    # (incl. a measured-vanilla empty mod-set). The skeleton already seeds None via
    # asdict(Provenance()); only overwrite when we actually have a mod-set so an
    # offline/test path that passes modset=None never disturbs the skeleton.
    if modset is not None:
        snap["provenance"]["mod"] = modset
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
    only ever fills a SKELETON-default empty, never a measured one (schema P1)."""
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


# Map a LIVE memory/collector domain to the EXACT country leaf paths it MEASURES (dotted,
# relative to the country object), so the fold protects a measured value/empty there
# from save overwrite WITHOUT over-protecting sibling leaves the domain did NOT
# measure (schema P1). country_scalars measures only these specific scalars; a future
# live reader that measures e.g. a list field registers its leaf path here and the
# protection then covers a measured-empty []/{}.
_LIVE_COUNTRY_DOMAIN_LEAVES = {
    "country_scalars": (
        "politics.stability", "politics.war_support", "politics.political_power",
        "politics.command_power", "politics.ruling_party",
        "military.num_ships", "military.command_power",
        # live manpower pool + convoy stockpile count (pinned offsets, M). Protect
        # the measured leaves so the save backstop cannot overwrite a measured 0.
        "military.manpower.available", "military.convoys_count",
    ),
    # per-country live MISC scalars (country_misc_live): convoys_destroyed,
    # accidents_score, ace_count, cosmetic_tag — folded under country.misc.
    "country_misc": (
        "misc.convoys_destroyed", "misc.accidents_score", "misc.ace_count",
        "misc.cosmetic_tag",
    ),
    # LIVE research (tech_live): completed-tech count + slot capacity fold onto the
    # save leaves; active_research lands in its own live-only leaf. Protect all three
    # so the save backstop cannot overwrite a measured value (incl. a measured-empty
    # in_progress_live []). NOT research.in_progress (that stays save-shaped).
    "tech_live": (
        "research.tech_count", "research.research_slots", "research.in_progress_live",
    ),
    # LIVE unit-leaders (commanders_live): the unsplit army/navy leader lists + the
    # individual live count sub-keys. Protect the deep count sub-leaves (NOT the whole
    # commanders.counts dict) so the save's richer counts are preserved (schema).
    "commanders_live": (
        "commanders.army_leaders", "commanders.navy_leaders",
        "commanders.counts.army_leaders", "commanders.counts.navy_leaders",
        "commanders.counts.characters",
    ),
    # LIVE agency scalars (intel_live): protect the DEEP measured sub-leaves of
    # intel.agency (NOT the whole agency dict) so save-only agency sub-keys survive
    # the merge while live wins on the measured scalars (schema).
    "intel_live": (
        "intel.agency.name", "intel.agency.icon", "intel.agency.upgrade_progress",
        "intel.agency.capacity.max_operatives", "intel.agency.capacity.usable_slots",
        "intel.agency.capacity.defense",
    ),
    # LIVE national-focus (focus_live): country.focus is a SAVE-SHARED home, so we
    # protect ONLY the exact leaves the live read measured (incl. a measured-empty
    # completed [] = truly zero, which must beat a stale save list). focus.paused is
    # NOT protected (RESISTED live -> the save keeps it); current_continuous is a
    # live-only leaf. A live None leaf is unmeasured and left for the save (schema).
    "focus_live": (
        "focus.focus_tree", "focus.completed", "focus.completed_count",
        "focus.current", "focus.current_continuous", "focus.progress",
    ),
    # LIVE production (production_live): country.production is a LIVE-ONLY home (no save
    # writes there), so protecting all six measured leaves cannot mask a richer save
    # value. Protects the measured-empty {}/[] + measured 0 from a save backstop (there
    # is none today, but the protection is the invariant the convention requires).
    "production_live": (
        "production.stockpile", "production.stockpile_total",
        "production.military_lines", "production.naval_lines",
        "production.military_factories", "production.naval_factories",
    ),
    # LIVE diplomacy (diplomacy_live): country.diplomacy is a SAVE-SHARED home (the save
    # writes diplomacy.wars/opinion/...), but at_war/casualties/faction_group are DISTINCT
    # live-only leaves whose skeleton default is None (NOT []), so the `lv is not None` guard
    # in _merge_country already skips an unmeasured country -- no _SKELETON_EMPTY_CONTAINER_
    # LEAVES entry is needed (a measured-empty at_war [] = at peace is protected here; schema).
    "diplomacy_live": (
        "diplomacy.at_war", "diplomacy.casualties", "diplomacy.faction_group",
    ),
    # LIVE bilateral relations (diplo_relations_live): opinion folds onto the SAVE-SHARED
    # diplomacy.opinion leaf (live wins on the measured matrix); relation_flags + overlord/
    # autonomy_state/autonomy_ratio are live-only leaves. ALL default None (NOT {}), so the
    # `lv is not None` guard in _merge_country already skips an unmeasured country -- no
    # measured-empty container guard is needed (a measured-empty {} is never emitted by
    # the plug; it only keys a country that carries a measured leaf). Protecting opinion keeps a
    # live measured matrix from being clobbered by a stale save opinion map (schema).
    "diplo_relations": (
        "diplomacy.opinion", "diplomacy.relation_flags", "diplomacy.overlord",
        "diplomacy.autonomy_state", "diplomacy.autonomy_ratio",
    ),
    # LIVE politics (politics_live): country.politics is a SAVE-SHARED home. party_popularities
    # + ruling_party fold onto the save leaves (live wins on the measured value); ideas/
    # idea_count/election_frequency/party_count/low_confidence are live leaves (ideas default
    # None, NOT [], so the None-guard suffices). ideas_other and national_spirits are
    # save-shared and default [], so measured-empty [] is protected only for TAGs recorded
    # in _live_measured_leaf_tags.
    # laws is deliberately NOT protected because an empty live {} must not erase save laws.
    # political_power is DELIBERATELY NOT here (country_scalars owns + protects it; a single
    # live owner).
    "politics_live": (
        "politics.party_popularities", "politics.ruling_party", "politics.ideas",
        "politics.idea_count", "politics.election_frequency", "politics.party_count",
        "politics.low_confidence", "politics.ideas_other", "politics.idea_groups",
        "politics.advisor_ideas",
        "politics.national_spirits",
    ),
    # LIVE national fuel (fuel_live): country.economy.fuel is a SAVE-SHARED dict home. We
    # protect ONLY the two scalar leaves the live read measures (current/max). Their own
    # skeleton default is ABSENT (economy.fuel defaults to {}), so a country fuel_live did
    # NOT measure carries None at fuel.current/fuel.max and the None-guard in _merge_country
    # already skips it -- no measured-empty container guard is needed (the {} parent
    # is itself never protected as a measurement). A measured 0.0 fuel is protected here so a
    # stale save value cannot overwrite a live empty tank (schema).
    "fuel_live": (
        "economy.fuel.current", "economy.fuel.max",
    ),
    # LIVE construction queue + opinion-modifier decomposition (construction_orders_live):
    # both are live-only leaves (default None), so the None-guard suffices for an unmeasured
    # country; the per-tag measured set (recorded in _live_measured_leaf_tags) protects a
    # measured-empty construction_queue [] on a measured country. opinion_modifiers default
    # None (never a skeleton {}), so the None-guard suffices.
    "construction_orders": (
        "production.construction_queue", "diplomacy.opinion_modifiers",
    ),
    # LIVE active timed decisions (decisions_market_live): a live-only leaf (default None); the
    # per-tag measured set protects a measured-empty timed_live [] on a measured country.
    "decisions_market": (
        "decisions.timed_live",
    ),
    # Scheduled in-process collector decision rows: status-vector presence counts and
    # active timed decisions. timed_live=[] is a measured "no active timed decision"
    # only for TAGs recorded by the collector fold.
    "decisions_live": (
        "decisions.status_present", "decisions.timed_live",
    ),
    # LIVE operative skill/xp (final_residuals_live): a live-only leaf (default None), so the
    # None-guard suffices for an unmeasured country.
    "operative_xp": (
        "intel.operative_skill_xp",
    ),
    # LIVE election residuals (politics_residuals_live): all three are live-only leaves whose
    # default is None (incl. appointed_advisors -> Optional list, default None, NOT []), so the
    # None-guard in _merge_country already skips an unmeasured country.
    "politics_residuals": (
        "politics.last_election", "politics.elections_allowed",
        "politics.appointed_advisors", "politics.appointed_advisor_details",
        "politics.appointed_advisor_slots", "politics.appointed_advisor_ledgers",
    ),
    # LIVE operative roster + cryptology (intel_deep_live): both are live-only leaves (default
    # None), so the None-guard suffices for an unmeasured country.
    "intel_deep": (
        "intel.operatives_live", "intel.cryptology_live",
    ),
}


# Map a LIVE memory domain to the EXACT STATE leaf paths it MEASURES (dotted, relative
# to the state object), so the save backstop cannot overwrite a value the live channel
# measured -- including a measured 0 (a zero-VP state) or a measured-empty {} (a state
# with no VP-bearing province). Mirrors _LIVE_COUNTRY_DOMAIN_LEAVES for state entities
# (states have no per-leaf protection in the plain _backstop_merge path; this adds it).
# state_vp is the only live state domain that folds onto a SAVE-SHARED leaf today;
# state_detail's manpower_pool/category are M/S but their skeleton default is None and
# _backstop_merge already lets a live value win, so they need no re-assertion here.
_LIVE_STATE_DOMAIN_LEAVES = {
    "state_vp": (
        "victory_points_total", "victory_points_by_province",
    ),
    # LIVE per-state building levels (buildings_live): states.*.buildings is a SAVE-SHARED leaf
    # (default None). Protect the measured map so the save backstop cannot overwrite a live
    # building read (incl. a measured-empty {} for a state with no buildings) on a measured
    # state. The per-state measured set is recorded under "states.buildings" in
    # _live_measured_leaf_tags so it re-asserts ONLY on the states buildings_live measured.
    "buildings_live": (
        "buildings",
    ),
    # LIVE per-state extra fields (state_fields_live): states.*.resources is a SAVE-SHARED leaf
    # (default None); protect the measured table (incl. a measured-empty {} for a state with no
    # resources) so the save backstop cannot overwrite the live read on a measured state. The
    # measured set is recorded under "states.resources". dynamic_modifier + extra_shared_slots
    # are live-only leaves (default None) and need no re-assertion (the None-guard suffices).
    "state_fields": (
        "resources",
    ),
}

# Protected STATE leaves whose schema SKELETON DEFAULT is a non-None empty container
# (analogous to _MEASURED_EMPTY_CONTAINER_LEAVES for countries). victory_points_by_province
# defaults to None (NOT {}), so it is NOT listed here -- a state the live domain did not
# measure carries None there and the None-guard skips it. This frozenset is the hook for
# any future live state leaf whose default is []/{} (none today).
_SKELETON_EMPTY_CONTAINER_STATE_LEAVES = frozenset()


def _set_leaf(d: Dict[str, Any], path: str, value: Any) -> None:
    """Set a dotted leaf, CREATING missing dict parents so a measured live value is
    never silently dropped (schema P1c). A non-dict intermediate is the only case we
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


# Protected leaves whose empty []/{} value must be proven measured before it can
# be re-asserted over the save. This includes save-shared leaves whose schema
# skeleton default is an empty container, plus live leaves like decisions.timed_live
# where [] is a real "measured empty" and not just a generic no-information value.
_MEASURED_EMPTY_CONTAINER_LEAVES = frozenset((
    "focus.completed",       # Focus.completed defaults [] (save-shared list home)
    "politics.ideas_other",  # Politics.ideas_other defaults [] (save-shared list home)
    "politics.national_spirits",
    # Decisions.timed_live defaults None, but an empty timed_live=[] still needs proof
    # that this TAG was actually measured before it may overwrite a backstop value.
    "decisions.timed_live",
))


def _merge_country(live_c: Dict[str, Any], save_c: Dict[str, Any],
                   protected_leaves, tag: Optional[str] = None,
                   measured_leaf_tags: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
    """Backstop-merge one country (live wins where informative; save fills gaps),
    then RE-ASSERT the exact live-measured leaves so save can never overwrite a
    value the live channel measured -- including a measured-empty []/{} (schema P1).
    A live None leaf is unmeasured and is left for save to have filled.

    ``measured_leaf_tags`` (optional) maps a protected leaf path -> the set of TAGs the
    owning live domain MEASURED. It is consulted ONLY for leaves whose skeleton default
    is or may carry a measured empty container (``_MEASURED_EMPTY_CONTAINER_LEAVES``):
    for those, a
    non-None [] is re-asserted only when ``tag`` is in the leaf's measured set, so a
    cross-domain skeleton [] on an UN-measured country can never masquerade as a measured
    zero and overwrite the save (schema cross-domain review). When the map is absent (the
    legacy 3-arg call), behaviour is unchanged for None-defaulting leaves; an empty-
    container leaf falls back to the value guard (callers that need the cross-domain
    protection pass the map)."""
    merged = _backstop_merge(live_c, save_c)
    for path in protected_leaves:
        lv = _get_leaf(live_c, path)
        if lv is None:
            continue  # unmeasured -> leave whatever the save filled
        # Masquerade guard for an empty-container-default leaf (skeleton default []):
        # re-assert ONLY when this TAG was actually measured by the owning live domain,
        # proven by the per-leaf measured-TAG set the assembler recorded. We FAIL CLOSED:
        # if we cannot prove this TAG was measured (no measured_leaf_tags map at all, no
        # entry for this leaf, or tag absent from the set), we do NOT re-assert -- a
        # skeleton [] must never overwrite the save (schema cross-domain review + legacy-
        # snapshot edge). The normal assembler path always supplies the map + entry, so a
        # genuinely-measured country is still protected; only an un-provable [] is skipped.
        if path in _MEASURED_EMPTY_CONTAINER_LEAVES and lv in ([], {}):
            measured = (measured_leaf_tags or {}).get(path)
            if measured is None or tag is None or tag not in measured:
                continue  # cannot prove this [] was measured -> do NOT protect (fail-closed)
        # measured (incl. 0/0.0/False/[]/{}) -> re-assert it. deep-copy a mutable leaf so
        # the merged snapshot never ALIASES the live snapshot's object (schema P1c).
        _set_leaf(merged, path,
                  copy.deepcopy(lv) if isinstance(lv, (dict, list)) else lv)
    return merged


def _merge_state(live_s: Dict[str, Any], save_s: Dict[str, Any],
                 protected_leaves, sid: Optional[str] = None,
                 measured_leaf_tags: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
    """Backstop-merge one state (live wins where informative; save fills gaps), then
    RE-ASSERT the exact live-measured STATE leaves so the save can never overwrite a
    value the live channel measured -- including a measured 0 (zero-VP state) or a
    measured-empty {} (no VP-bearing province). A live None leaf is unmeasured and is
    left for the save to have filled. Mirrors _merge_country for state entities.

    ``measured_leaf_tags`` maps a protected leaf path -> the set of STATE IDs the owning
    live domain MEASURED. It is consulted for a leaf whose skeleton default is a non-None
    empty container (_SKELETON_EMPTY_CONTAINER_STATE_LEAVES): re-assert only when ``sid``
    is in that set (fail-closed). For a None-defaulting leaf (victory_points_by_province
    defaults to None), the value guard alone is correct: a measured {} is non-None and is
    re-asserted, an unmeasured None is skipped. We additionally fail-closed re-assert a
    None-defaulting leaf only on measured states when the map is supplied, so a state the
    plug did NOT measure (a save-only state id) can never have its save VP clobbered."""
    merged = _backstop_merge(live_s, save_s)
    for path in protected_leaves:
        lv = _get_leaf(live_s, path)
        if lv is None:
            continue  # unmeasured -> leave whatever the save filled
        # Masquerade guard for an empty-container-default leaf (skeleton default []/{}):
        # re-assert ONLY when this STATE was actually measured by the owning live domain.
        if path in _SKELETON_EMPTY_CONTAINER_STATE_LEAVES:
            measured = (measured_leaf_tags or {}).get(path)
            if measured is None or sid is None or sid not in measured:
                continue
        else:
            # For a None-defaulting protected leaf we still fail-closed when a measured set
            # is supplied: only re-assert on a state the plug measured. Without a map (legacy
            # call) the value guard above (lv is not None) preserves the original behaviour.
            measured = (measured_leaf_tags or {}).get(path)
            if measured is not None and (sid is None or sid not in measured):
                continue
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
    doctrine_doc = _load_doctrine_save(output_dir)
    doctrine_has_payload = (
        isinstance(doctrine_doc, dict)
        and (bool(doctrine_doc.get("countries"))
             or bool(doctrine_doc.get("countries_by_key")))
    )
    save_snap = _store.build_snapshot_from_oracle(output_dir, gid)
    save_prov = (save_snap.get("provenance") or {}).get("save") or {}
    save_domains = list(save_prov.get("domains") or [])
    if not save_domains and not doctrine_has_payload:
        return snap  # nothing to fold (empty/absent oracle) -> no fabrication

    # Which exact country LEAVES did the LIVE memory/collector channel measure? Those values
    # (incl. a measured-empty []/{}) must be protected from save overwrite (schema P1).
    live_mem_domains = ((snap.get("provenance") or {}).get("memory") or {}).get(
        "domains") or []
    live_collector_domains = (
        ((snap.get("provenance") or {}).get("collector") or {}).get("domains") or []
    )
    protected_leaves = []
    for dom in list(live_mem_domains) + list(live_collector_domains):
        protected_leaves.extend(_LIVE_COUNTRY_DOMAIN_LEAVES.get(dom, ()))

    # Per-leaf MEASURED-TAG sets for the masquerade guard: a protected save-shared leaf
    # whose skeleton default is a non-None empty container ([]) must be re-asserted ONLY
    # on the TAGs the owning live domain actually measured, never on a cross-domain
    # skeleton [] (schema cross-domain review). The assembler recorded this set per leaf.
    measured_leaf_tags = {}
    for path, tags in (snap.get("_live_measured_leaf_tags") or {}).items():
        measured_leaf_tags[path] = set(tags or ())

    # --- countries: backstop each TAG's domains (live wins) ---
    live_countries = snap.setdefault("countries", {})
    for tag, sc in (save_snap.get("countries") or {}).items():
        lc = live_countries.get(tag)
        if lc is None:
            # a TAG present only in the save: the save IS the only measurement.
            live_countries[tag] = sc
        else:
            live_countries[tag] = _merge_country(
                lc, sc, protected_leaves, tag=tag,
                measured_leaf_tags=measured_leaf_tags)

    # --- states: backstop per entity, RE-ASSERTING the exact live-measured state leaves
    # (state_vp's victory_points_*) so a stale save value can never overwrite a measured
    # live VP -- incl. a measured 0 (zero-VP state) or a measured-empty {} by_province map
    # (which REPLACES, never key-merges, the save map). A state the live domain did NOT
    # measure keeps the save value (the live leaf is None -> not re-asserted). ---
    protected_state_leaves = []
    for dom in live_mem_domains:
        protected_state_leaves.extend(_LIVE_STATE_DOMAIN_LEAVES.get(dom, ()))
    measured_state_leaf_ids = {}
    for path, ids in (snap.get("_live_measured_leaf_tags") or {}).items():
        if path.startswith("states."):
            # the assembler keys these as "states.<leaf>"; the protected paths are
            # state-object-relative ("<leaf>"). Strip the "states." prefix to align.
            measured_state_leaf_ids[path[len("states."):]] = set(ids or ())
    live_states = snap.setdefault("states", {})
    for key, sv in (save_snap.get("states") or {}).items():
        lv = live_states.get(key)
        if lv is None:
            live_states[key] = sv
        else:
            live_states[key] = _merge_state(
                lv, sv, protected_state_leaves, sid=str(key),
                measured_leaf_tags=measured_state_leaf_ids)
    # --- provinces: backstop per entity (no live province-leaf protection today) ---
    live_p = snap.setdefault("provinces", {})
    for key, sv in (save_snap.get("provinces") or {}).items():
        lv = live_p.get(key)
        live_p[key] = sv if lv is None else _backstop_merge(lv, sv)

    # --- doctrine manager (top-level doctrine.json): additive save-cadence state.
    # This is not yet a replay/schema.py canonical domain id, so it is folded as a
    # save backstop under countries.*.research.doctrine for TAG-keyed rows and as a
    # top-level sidecar for non-TAG save keys. No TAG bridge is inferred here.
    doctrine_stats = _fold_doctrine_save(snap, doctrine_doc)

    # provenance domains the LIVE memory channel actually captured. Gate the
    # divisions/combats adoption on THIS, not on emptiness: a captured measured-empty
    # live [] is a measured zero that must WIN over a stale save roster (schema review
    # #1/#2). Computed once here and reused below.
    _live_mem = ((snap.get("provenance") or {}).get("memory") or {}).get("domains") or []

    # --- divisions: live divisions[] are authoritative when the divisions domain was
    # CAPTURED (even as a measured-empty []); only adopt the save roster when LIVE did
    # not capture divisions at all (else mixing two channels' (id,type) rosters risks
    # double-counting). A live read replaces; it is never diluted by stale save rows.
    if "divisions" not in _live_mem and save_snap.get("divisions"):
        snap["divisions"] = save_snap["divisions"]
    # If LIVE captured divisions, that roster is authoritative -> recompute every known
    # country's military.division_count from snap["divisions"] (0 for a tag with no live
    # division). Without this, a country present ONLY in the save (copied wholesale into
    # countries above) keeps its stale SAVE-derived division_count even though the live
    # divisions won (schema re-verify #1: a measured-zero from the live roster must win).
    if "divisions" in _live_mem:
        _dc: Dict[str, int] = {}
        for d in (snap.get("divisions") or []):
            o = d.get("owner")
            if isinstance(o, str):
                _dc[o] = _dc.get(o, 0) + 1
        for tag, c in (snap.get("countries") or {}).items():
            if isinstance(c, dict):
                mil = c.setdefault("military", {})
                if isinstance(mil, dict):
                    mil["division_count"] = _dc.get(tag, 0)

    # --- world-scoped catalogs: backstop only if live left them empty ---
    # Includes the Tier-1 EXTRACT world domains (strategic_air / strategic_operatives
    # / raids / world_events / world). These are save-ONLY today (no live reader emits
    # them), so the backstop simply fills the empty live skeleton; if a future live
    # read ever populates one, the live-wins rule still holds (informative wins).
    # Protect the EXACT top-level leaves the LIVE channel measured (incl. a
    # measured-empty []) from being overlaid by the save's _backstop_merge (schema
    # review #1): a measured-empty live list is a measured zero, not a gap. We
    # snapshot those leaves BEFORE the merge and re-assert them AFTER. Only a leaf
    # whose live domain actually captured is protected. (_live_mem computed above.)
    _protect_top = []
    if "designs" in _live_mem:
        _protect_top.append(("designs", "division_templates"))
    # LIVE equipment-variant catalog (designs_variants): protect the DISTINCT live-only leaf
    # designs.equipment_variants_live from the save backstop (the save designs.equipment_variants
    # is a separate key space and is left save-owned).
    if "designs_variants" in _live_mem:
        _protect_top.append(("designs", "equipment_variants_live"))
    if "ships" in _live_mem:
        _protect_top.append(("order_of_battle", "ships"))
    # LIVE battle-plan orders (construction_orders): protect the live-measured
    # order_of_battle.battle_plan_orders (incl. a measured-empty []) from the save backstop.
    if "construction_orders" in _live_mem:
        _protect_top.append(("order_of_battle", "battle_plan_orders"))
    # LIVE OOB command tree: protect the EXACT leaves the live read filled (incl. a
    # measured-empty []) from the save backstop's _backstop_merge, so the live command
    # structure wins. LEAF-level re-assertion (NOT the whole order_of_battle dict), so it
    # coexists with the ships re-assert above -- each restores an independent sub-list of
    # the same dict (schema-approved). navy_theaters/air_wings/by_country are NOT live-measured
    # and stay save-filled.
    if "oob_command" in _live_mem:
        for _leaf in ("armies", "army_groups", "fleets", "theatres"):
            _protect_top.append(("order_of_battle", _leaf))
    _live_top = {(k, leaf): copy.deepcopy((snap.get(k) or {}).get(leaf))
                 for (k, leaf) in _protect_top}
    for key in ("designs", "order_of_battle", "diplomacy_world",
                "strategic_air", "strategic_operatives", "raids",
                "world_events", "world"):
        sv = save_snap.get(key)
        if sv is not None:
            snap[key] = _backstop_merge(snap.get(key), sv)
    # re-assert the live-measured top-level leaves (save can never overwrite them).
    for (k, leaf), lv in _live_top.items():
        if lv is not None and isinstance(snap.get(k), dict):
            snap[k][leaf] = lv

    # --- combats: adopt the save's active combats only when NO live combat domain
    # captured (land_combats / naval_air_combats / naval_air_combat_stats). A captured
    # live combat domain — even producing an empty combats[] — wins (schema review #2).
    _live_combat = any(d in _live_mem for d in
                       ("land_combats", "naval_air_combats", "naval_air_combat_stats"))
    if not _live_combat and save_snap.get("combats"):
        snap["combats"] = save_snap["combats"]

    # --- map mirrors: refill from the merged state/province data ---
    _fill_map_mirrors(snap)

    # --- WAVE-2 derived-model attachments (designs stats, victory score, scalars,
    # air-superiority model, supply estimate). Computed over the merged snapshot +
    # this save's output dir; each is independently guarded (a missing game install
    # leaves the field None). Provenance = derived (NOT a save/memory channel). ---
    attach_derived_models(snap, output_dir)

    # --- provenance.save: record the save-filled canonical domains HONESTLY -------
    # Preserve any pre-existing non-canonical backstops (buildings/intel sidecars).
    prov = snap.setdefault("provenance", {})
    existing = prov.get("save") or {}
    backstops = list(existing.get("backstops") or [])
    src_filenames = dict(existing.get("source_filenames") or {})
    dmeta = doctrine_doc.get("meta") if isinstance(doctrine_doc, dict) else None
    doctrine_src = None
    doctrine_date = None
    if isinstance(dmeta, dict):
        doctrine_src = _redact_path(dmeta.get("source_filename"))
        doctrine_date = dmeta.get("save_date")
    save_src = _redact_path(save_prov.get("source_filename")) or doctrine_src
    new_save = {
        "captured": True,
        "source": "melted_save_on_disk",
        "is_live": False,
        # the full-bundle fold fills canonical domains only when replay/store.py
        # exposed them. Additive doctrine is tracked below as a backstop.
        "fills_canonical_domains": bool(save_domains),
        "save_date": save_prov.get("save_date") or doctrine_date,
        "source_filename": save_src,
        "source_filenames": src_filenames,
    }
    if save_domains:
        new_save["domains"] = sorted(set(save_domains))
    if backstops:
        new_save["backstops"] = backstops
    if doctrine_stats.get("tag_rows") or doctrine_stats.get("non_tag_rows"):
        new_save.setdefault("backstops", [])
        if "doctrine" not in new_save["backstops"]:
            new_save["backstops"].append("doctrine")
        if doctrine_src:
            src_filenames["doctrine"] = {
                "source_filename": doctrine_src
            }
    prov["save"] = new_save
    notes = list(prov.get("notes") or [])
    if save_domains:
        notes.append("save backstop folded %d canonical domain(s) from the melted save "
                     "(live channel wins; save fills gaps; no fabrication)."
                     % len(new_save["domains"]))
    if doctrine_stats.get("tag_rows") or doctrine_stats.get("non_tag_rows"):
        notes.append("save doctrine backstop folded %d TAG row(s) and preserved %d "
                     "non-TAG row(s) without inferring a country bridge."
                     % (doctrine_stats.get("tag_rows", 0),
                        doctrine_stats.get("non_tag_rows", 0)))
    prov["notes"] = notes
    return snap


# ---------------------------------------------------------------------------
# WAVE-2 LIVE READERS (weather temperature + naval combat per-side stats).
# Every entry point is import-guarded AND value-guarded so the offline/no-live-game
# path returns False (domain not claimed) instead of raising. READ-ONLY memory.
# ---------------------------------------------------------------------------
def _wire_weather(snap: Dict[str, Any], pid: Optional[int]) -> bool:
    """Fold live per-PROVINCE temperature + precipitation + snow into snap["weather"]
    (§19, M).

    GRANULARITY (2026-06-26): the weather temperature read was RETRACTED from per-REGION
    to per-PROVINCE (memory_reader/weather/weather_temp_model.py: the per-region +0x138
    field was a climatology base, not the displayed value). The realized displayed
    temperature + precipitation (`water`) + snow all live in the SAME per-PROVINCE struct
    at province-id index. So the live weather fold is PER-PROVINCE: it populates the
    province-keyed weather.provinces.{pid}.{temp_c,precip,snow} block. weather.regions is
    left at its null skeleton because this live fold has NO working per-region reader
    (its read was retracted). Save-persisted region weather is handled as a separate
    oracle/parser path; this live fold must not backfill it and claim freshness. A null
    region block here means "not measured by the live fold", never a stale merge.

    precip (`water`) and snow are the SAVE-PERSISTED per-province amounts (validated exact
    vs the save), i32/1e5 floats. The reader collapses a zero/absent slot to None (it cannot
    distinguish a measured 0.0 from unmeasured), so we fold None as None and NEVER fabricate
    a 0.0; a positive float is kept as measured (schema B2). temp_c is None for a province
    whose temp slot reads exactly 0 (sea/inactive), also never a fabricated 0 C.

    Guarded: no pid (offline) or any import/read failure -> leave the null-but-present
    weather skeleton and return False (domain not claimed; no fabrication). The domain is
    claimed iff AT LEAST the temperature read produced a per-province array (precip is
    additive; a precip-read failure leaves precip/snow None but still claims weather on the
    temperature evidence)."""
    if pid is None:
        return False
    try:
        from memory_reader.weather import weather_temp_model as _wt  # noqa: E402
    except Exception:
        return False
    try:
        tres = _wt.read_province_temperatures(pid)
    except Exception:
        return False
    if not isinstance(tres, dict):
        return False
    tprov = tres.get("provinces")
    if not isinstance(tprov, dict) or not tprov:
        return False
    # precipitation + snow are ADDITIVE (same per-province array); a precip-read failure
    # must not sink the temperature evidence -> guard it independently and leave precip/
    # snow None when it is unavailable (no fabrication).
    pprov: Dict[Any, Any] = {}
    pres = None
    precip_measured = False   # True only when the precip read produced a province array
    try:
        pres = _wt.read_province_precipitation(pid)
        if isinstance(pres, dict) and isinstance(pres.get("provinces"), dict):
            pprov = pres["provinces"]
            precip_measured = True
    except Exception:
        pprov = {}
        precip_measured = False

    out_provinces: Dict[str, Any] = {}
    temps = []
    n_with_precip = 0
    n_with_snow = 0
    for pidk, row in tprov.items():
        if not isinstance(row, dict):
            continue
        temp_c = row.get("temp_c")          # None when not realized -> NOT a 0 C
        prow = pprov.get(pidk) if isinstance(pprov, dict) else None
        precip = prow.get("water") if isinstance(prow, dict) else None
        snow = prow.get("snow") if isinstance(prow, dict) else None
        if temp_c is not None:
            temps.append(temp_c)
        if precip is not None:
            n_with_precip += 1
        if snow is not None:
            n_with_snow += 1
        out_provinces[str(pidk)] = {
            "province_id": (int(pidk) if str(pidk).lstrip("-").isdigit() else None),
            "temp_c": temp_c,
            "temp_change_c": row.get("temp_change_c"),
            "precip": precip,               # `water` amount (None == zero-or-unmeasured)
            "snow": snow,                   # snow amount  (None == zero-or-unmeasured)
        }
    if not out_provinces:
        return False
    rollup = {
        "n_provinces": len(out_provinces),
        "n_with_temp": tres.get("n_with_temp"),
        "temp_min_c": tres.get("temp_min_c"),
        "temp_max_c": tres.get("temp_max_c"),
        "temp_mean_c": tres.get("temp_mean_c"),
        # honesty: when the precip read did NOT run (additive read failed), the tallies are
        # UNMEASURED -> None, NOT a fabricated 0 that would read as "measured, none found"
        # (schema final review). They are concrete counts only when precip was measured.
        "n_with_precip": (n_with_precip if precip_measured else None),
        "n_with_snow": (n_with_snow if precip_measured else None),
    }
    if precip_measured:
        # carry the precip-read's own rollup scalars when present (measured-additive).
        if "water_max" in (pres or {}):
            rollup["precip_max"] = pres.get("water_max")
        if "water_mean" in (pres or {}):
            rollup["precip_mean"] = pres.get("water_mean")
    # preserve the null-but-present region skeleton (no working region reader); only fill
    # the province block + rollup + source badge.
    wblk = snap.get("weather")
    if not isinstance(wblk, dict):
        wblk = schema.empty_weather()
        snap["weather"] = wblk
    wblk["provinces"] = out_provinces
    wblk["global_rollup"] = rollup
    wblk["source"] = "M"
    return True


def _wire_weather_fields(snap: Dict[str, Any], pid: Optional[int]) -> bool:
    """Fold the VALIDATED per-region weather fields (temperature, phenomenon flags,
    next_weather_change, active_modifier_count) into snap["weather"]["regions"] and the
    per-province mud + custom_modifier_count into the existing province rows (§19, M).

    Cracked + validated read-only vs the content-matched melt (305/305 region temp;
    per-province mud TP146/FP0/FN0; custom_modifiers TP52/FP0/FN0). The region block
    was previously left null (the old per-region temp read was retracted); this fills it
    with the re-cracked SWeatherPerRegion+0x138 temperature + the phenomenon-flag band +
    next_weather_change@0x150 + active_modifiers@0x30 count -- all distinct from the
    per-province displayed temperature.

    Guarded: no pid / import / read failure -> return False (region block stays whatever
    _wire_weather left it; no fabrication). A measured-empty phenomenon [] / active-
    modifier 0 is a real read of the located struct."""
    if pid is None:
        return False
    try:
        from memory_reader.weather import weather_fields_live as _wf  # noqa: E402
    except Exception:
        return False
    # Locate the CWeatherManager ONCE (an arena discover + parallel vtable scan) and thread
    # the open (proc, mgr) into both reads so the locate is not paid twice per capture.
    located = None
    try:
        located = _wf.locate_weather(pid)
    except Exception:
        located = None
    if not located or located[0] is None:
        located = None
    try:
        try:
            rres = _wf.read_region_fields(pid, _located=located)
        except Exception:
            rres = None
        try:
            pres = _wf.read_province_fields(pid, _located=located)
        except Exception:
            pres = None
    finally:
        if located is not None and located[0] is not None:
            try:
                located[0].close()
            except Exception:
                pass
    folded = False
    wblk = snap.get("weather")
    if not isinstance(wblk, dict):
        wblk = schema.empty_weather()
        snap["weather"] = wblk

    # per-region fields
    if isinstance(rres, dict) and isinstance(rres.get("regions"), dict) and rres["regions"]:
        regions: Dict[str, Any] = {}
        for rid, row in rres["regions"].items():
            if not isinstance(row, dict):
                continue
            regions[str(rid)] = {
                "region_id": (int(rid) if str(rid).lstrip("-").isdigit() else None),
                "temperature": row.get("temperature"),
                "phenomenon": list(row.get("phenomenon") or []),
                "next_weather_change_hours": row.get("next_weather_change_hours"),
                "active_modifier_count": row.get("active_modifier_count"),
            }
        if regions:
            wblk["regions"] = regions
            wblk["source"] = "M"
            folded = True

    # per-province mud + custom_modifier_count: MERGE onto the existing province rows
    # (the temp/precip rows from _wire_weather); a province not already present is added.
    if isinstance(pres, dict) and isinstance(pres.get("provinces"), dict) and pres["provinces"]:
        prov = wblk.get("provinces")
        if not isinstance(prov, dict):
            prov = {}
            wblk["provinces"] = prov
        for pidk, row in pres["provinces"].items():
            if not isinstance(row, dict):
                continue
            key = str(pidk)
            cur = prov.get(key)
            if not isinstance(cur, dict):
                cur = {"province_id": (int(pidk) if str(pidk).lstrip("-").isdigit() else None)}
                prov[key] = cur
            cur["mud"] = row.get("mud")
            cur["custom_modifier_count"] = row.get("custom_modifier_count")
        wblk["source"] = "M"
        folded = True

    return folded


def _wire_naval_combat_stats(snap: Dict[str, Any], pid: Optional[int]) -> bool:
    """Fold per-side naval combat damage/stat rows onto naval combat records under
    combats[] (kind=="naval"), keyed by combat_id (§7c, M / M*).

    Guarded: no pid (offline) or any import/read failure -> return False, leaving the
    typed combats[] untouched (no fabrication). Per-side rows self-badge their tier."""
    if pid is None:
        return False
    try:
        from memory_reader.combat import combat_enum as _ce  # noqa: E402
        from memory_reader.combat import naval_combat_stats as _ncs  # noqa: E402
    except Exception:
        return False
    try:
        # CombatReader takes a process NAME (defaults to hoi4.exe), NOT a pid (schema
        # re-verify #4: passing the int pid made name.lower() throw, so the whole domain
        # silently failed closed and never captured). Attach by name, then confirm it is
        # OUR process instance before trusting any read.
        reader = _ce.CombatReader()
        if getattr(reader, "pid", None) != pid:
            return False
        combats = reader.enumerate_naval_combats(with_defenders=True)
    except Exception:
        return False
    if not combats:
        return False
    try:
        statreader = _ncs.NavalCombatStats(reader)
    except Exception:
        return False
    # Index live naval combats by combat_id. This domain is ALL-OR-NOTHING: any
    # enumerated combat we cannot FULLY read (not a dict / no body_addr / non-dict stats
    # / missing int combat_id / read error) -> abort and fold nothing, so we never claim
    # naval_air_combat_stats from a partial read (schema re-verify #2/#3).
    by_id: Dict[str, Any] = {}
    for c in combats:
        if not isinstance(c, dict) or c.get("body_addr") is None:
            return False
        try:
            s = statreader.read_combat_stats(c["body_addr"])
        except Exception:
            return False
        cid = s.get("combat_id") if isinstance(s, dict) else None
        if not isinstance(cid, int):
            return False
        by_id[str(cid)] = {"attacker": s.get("attacker"),
                           "defender": s.get("defender")}
    if not by_id:
        return False
    # merge onto existing combats[] records of kind "naval"; if none matched, append
    # a minimal naval combat record carrying just the stats (still no fabrication of
    # participants — those stay absent).
    merged_any = False
    # Match ONLY existing naval (or as-yet-untyped) combat records. Land combats are
    # already in snap["combats"] (folded earlier); a naval combat whose id collides with
    # a land combat must NOT attach naval_stats to the LAND record -> exclude kind=="land"
    # so a same-id land record is treated as non-matching and we append a naval record
    # instead (schema re-verify #2: combat-id spaces can overlap across kinds).
    existing = {str(c.get("combat_id")): c for c in snap.get("combats") or []
                if isinstance(c, dict) and c.get("kind") in (None, "naval")}
    for cid, stats in by_id.items():
        rec = existing.get(cid)
        if rec is not None:
            rec["naval_stats"] = stats
            if rec.get("kind") is None:
                rec["kind"] = "naval"
            merged_any = True
        else:
            # cid is the stringified map key; Combat.combat_id is typed int, so write
            # back the original int id (validation doesn't catch a str). schema review #4.
            cid_int = int(cid) if isinstance(cid, str) and cid.lstrip("-").isdigit() else cid
            combat = schema.asdict(schema.Combat(combat_id=cid_int, kind="naval"))
            # participants were NOT measured on this stats-only path; schema.Combat
            # defaults them to [] which would FABRICATE "no participants". Force them
            # absent (None) -- measured-absent, not measured-empty (schema re-verify #3).
            combat["attackers"] = None
            combat["defenders"] = None
            combat["naval_stats"] = stats
            snap.setdefault("combats", []).append(combat)
            merged_any = True
    return merged_any


# ---------------------------------------------------------------------------
# WAVE-2 DERIVED-MODEL attachments (division/equipment stats, air superiority,
# victory score, scalars, supply estimate). Computed over the assembled snapshot +
# the save oracle output dir, so they live in the save-fold path (where designs /
# economy / tech / divisions actually populate). All D-source (provenance.derived).
# Every model is guarded: a missing game install / import / read leaves the field
# None (no fabrication) and never aborts the fold.
# ---------------------------------------------------------------------------
def attach_derived_models(snap: Dict[str, Any], output_dir: Optional[str]) -> Dict[str, Any]:
    """Attach all WAVE-2 derived-model layers to ``snap`` in place. Returns snap.

    Builds the expensive GameDefs / EquipmentDefs ONCE and reuses them. Each block is
    independently guarded so one model failing (e.g. no installed content) never blocks the
    others. Safe to call with output_dir=None (only the designs-based models, which
    read the snapshot's own designs block, then run; the output_dir-based ones skip)."""
    _attach_design_stats(snap)                         # division + equipment stats (D)
    if output_dir:
        _attach_victory_score(snap, output_dir)        # per-country score ladder (D)
        _attach_scalars(snap, output_dir)              # capitulation/research/SF (D)
        _attach_air_superiority_model(snap, output_dir)  # stat-weighted % (D)
        _attach_supply_estimate(snap, output_dir)      # priority-laddered supply (D)
    _attach_research_progress(snap)                    # live progress units + static tech cost (D)
    return snap


def _attach_design_stats(snap: Dict[str, Any]) -> None:
    """designs.template_stats (DivisionStats) + designs.equipment_variant_stats
    (VariantStats). Pure-derived from the snapshot's designs block + installed content."""
    designs = snap.get("designs")
    if not isinstance(designs, dict):
        return
    # division template stats
    try:
        from models import division_stats as _ds  # noqa: E402
        # GameDefs needs explicit dirs; the module exposes env-var-resolved defaults
        # (_DEFAULT_VANILLA / _DEFAULT_MOD). A missing install raises -> guarded skip.
        gdefs = _ds.GameDefs(vanilla_dir=_ds._DEFAULT_VANILLA,
                             mod_dir=_ds._DEFAULT_MOD)
        compute_division_stats = _ds.compute_division_stats
        make_modular_resolver = _ds.make_modular_resolver
        resolver = make_modular_resolver(gdefs, designs.get("equipment_variants") or {})
        out: Dict[str, Any] = {}
        for t in (designs.get("division_templates") or []):
            if not isinstance(t, dict) or t.get("id") is None:
                continue
            try:
                ds = compute_division_stats(t, gdefs, equip_resolver=resolver)
                out[str(t["id"])] = ds.as_dict()
            except Exception:
                continue
        if out:
            designs["template_stats"] = out
            designs["template_stats_source"] = "D"
    except Exception:
        pass  # no game install / parse failure -> leave template_stats None
    # equipment variant stats
    try:
        import dataclasses as _dc
        from models.equipment_stats import EquipmentDefs, compute_all  # noqa: E402
        edefs = EquipmentDefs()
        vstats = {k: _dc.asdict(vs) for k, vs in compute_all(designs, edefs).items()}
        if vstats:
            designs["equipment_variant_stats"] = vstats
            designs["equipment_variant_stats_source"] = "D"
    except Exception:
        pass  # leave equipment_variant_stats None


def _attach_victory_score(snap: Dict[str, Any], output_dir: str) -> None:
    """countries.*.score = per-country derived rank-ladder (models/victory_score)."""
    try:
        from models.victory_score import compute_victory_score  # noqa: E402
        ladder = compute_victory_score(output_dir)
    except Exception:
        return
    countries = getattr(ladder, "countries", None)
    if not isinstance(countries, dict):
        return
    for tag, cs in countries.items():
        c = snap["countries"].get(tag)
        if c is None:
            continue
        try:
            c["score"] = cs.as_dict()
        except Exception:
            continue


def _attach_scalars(snap: Dict[str, Any], output_dir: str) -> None:
    """countries.*.scalars (capitulation/research/special_forces) + intel_ledger
    (pairwise). Builds the static modifier index ONCE (models/scalars)."""
    try:
        from models import scalars as S  # noqa: E402
        import os as _os
        import json as _json
    except Exception:
        return

    def _load(name):
        p = _os.path.join(output_dir, name)
        if not _os.path.isfile(p):
            return None
        try:
            with open(p, "r", encoding="utf-8") as fh:
                return _json.load(fh)
        except Exception:
            return None

    economy = _load("economy.json")
    state_detail = _load("state_detail.json")
    tech = _load("tech.json")
    divisions = _load("divisions.json")
    intel = _load("intel.json")
    designs = _load("designs.json")      # exact SF battalion count (WIRE_scalars_v2 B)
    diplomacy = _load("diplomacy.json")  # enemy-only capitulation VP (WIRE_scalars_v2 C)
    try:
        modifier_index = S.build_modifier_index()
    except Exception:
        modifier_index = None

    for tag, c in snap["countries"].items():
        scal = c.get("scalars")
        if not isinstance(scal, dict):
            scal = {}
            c["scalars"] = scal
        try:
            if state_detail is not None and economy is not None:
                scal["capitulation"] = S.capitulation_progress(
                    tag, state_detail, economy, modifier_index, diplomacy=diplomacy)
        except Exception:
            pass
        try:
            if economy is not None and tech is not None:
                scal["research_speed"] = S.research_speed(
                    tag, economy, tech, modifier_index)
        except Exception:
            pass
        try:
            if divisions is not None and economy is not None and tech is not None:
                scal["special_forces"] = S.special_forces(
                    tag, divisions, economy, tech, modifier_index, designs=designs)
        except Exception:
            pass

    # pairwise intel ledger — only meaningful (crypto/leak) pairs, never NxN.
    if intel is not None and economy is not None:
        ledger = snap.setdefault("intel_ledger", {})
        targets_fn = getattr(S, "targets_of_interest", None)
        cdata = intel.get("countries") if isinstance(intel.get("countries"), dict) else {}
        for observer in cdata:
            try:
                tgts = (targets_fn(observer, intel) if callable(targets_fn)
                        else _crypto_targets(cdata.get(observer)))
            except Exception:
                tgts = ()
            for tgt in (tgts or ()):
                try:
                    led = S.intel_ledger(observer, tgt, intel, economy, modifier_index)
                except Exception:
                    led = None
                if led is not None:
                    ledger.setdefault(observer, {})[tgt] = led


def _attach_research_progress(snap: Dict[str, Any]) -> None:
    """Add fail-closed progress/ETA fields to active-research slots.

    Save slots are joined through their active ``technologies[]`` progress row;
    live slots require measured ``progress_units`` plus resolved ``tech_name``.
    Missing inputs leave raw slots intact or add an unavailable reason.
    """
    try:
        from models import research_progress as RP  # noqa: E402
    except Exception:
        return
    try:
        tech_defs = RP.load_static_tech_defs()
    except Exception:
        tech_defs = None
    try:
        RP.enrich_snapshot_research_progress(snap, tech_defs)
    except Exception:
        return


def _crypto_targets(observer_rec: Any):
    """Best-effort: the targets the observer is tracking via cryptology (so the
    ledger stays small). Returns a set of target TAGs."""
    out = set()
    if not isinstance(observer_rec, dict):
        return out
    crypto = observer_rec.get("cryptology")
    if isinstance(crypto, dict):
        for t in (crypto.get("targets") or []):
            if isinstance(t, dict) and isinstance(t.get("target"), str):
                out.add(t["target"])
            elif isinstance(t, str):
                out.add(t)
    return out


def _attach_air_superiority_model(snap: Dict[str, Any], output_dir: str) -> None:
    """naval_air.air_superiority_model = stat-weighted per-region contest
    (models/air_superiority). Reads oob.json from the output dir."""
    try:
        from models.air_superiority import compute_air_superiority  # noqa: E402
        result = compute_air_superiority(output_dir)
    except Exception:
        return
    try:
        d = result.as_dict()
    except Exception:
        return
    regions = d.get("regions")
    if not isinstance(regions, dict) or not regions:
        return
    na = snap.get("naval_air")
    if not isinstance(na, dict):
        na = schema.empty_naval_air()
        snap["naval_air"] = na
    na["air_superiority_model"] = regions
    # scrub path-leaking meta keys (source_path_redacted dropped; others basenamed)
    # before it enters the persisted snapshot (HARD path-leak rule).
    na["air_superiority_model_meta"] = _scrub_model_meta(d.get("meta"))


def _attach_supply_estimate(snap: Dict[str, Any], output_dir: str) -> None:
    """models.supply_estimate = priority-laddered per-province supply estimate
    (models/supply_number). Reuses the live supply proxy if the snapshot has one."""
    try:
        from models.supply_number import compute_supply_number  # noqa: E402
    except Exception:
        return
    # reuse the live division-province proxy already folded into snap["supply"].
    proxy = None
    sup = snap.get("supply")
    if isinstance(sup, dict) and isinstance(sup.get("by_province"), dict):
        proxy = sup["by_province"]
    try:
        est = compute_supply_number(output_dir, proxy_by_province=proxy)
    except Exception:
        return
    if isinstance(est, dict) and est:
        # defensively scrub any path-leaking key in the model's meta (HARD rule).
        if isinstance(est.get("meta"), dict):
            est["meta"] = _scrub_model_meta(est["meta"])
        models_blk = snap.get("models")
        if not isinstance(models_blk, dict):
            models_blk = schema.empty_models()
            snap["models"] = models_blk
        models_blk["supply_estimate"] = est


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


__all__ = ["assemble_snapshot", "fold_save_bundle", "attach_derived_models"]
