"""Derived-scalar formulas for the Battle Plan tool (read-only, offline).

Each public function is a *pure* formula over parsed-save JSON produced by
``save_oracle/`` (``country_snapshot``, ``economy``, ``tech``, ``intel``,
``state_detail``, ``divisions``, ``designs``, ``diplomacy``). They compute
"D"-source (derived) scalars the replay schema needs but the save does not store
in a directly-consumable form:

  * :func:`capitulation_progress` -- surrender / capitulation % (0..100)
  * :func:`research_speed`        -- overall research-speed delta (%)
  * :func:`special_forces`        -- special-forces cap & used (battalions)
  * :func:`intel_ledger`          -- per-category intel level (0..100)

Honesty rules (DESIGN no-fabrication; mirrors ``derive/layers.py`` and
``replay/schema.py``):
  * A scalar whose inputs are absent is ``None`` (JSON null), never a guessed
    number.  ``0`` / ``0.0`` means a *measured* zero.
  * Proxies are labelled as proxies in their return dict (``is_proxy`` /
    ``granularity`` / ``method``) and never laundered into an exact field.
  * Functions do no I/O and never mutate their inputs.

----------------------------------------------------------------------------
RAW-READ vs DERIVE (post-Codex-review correctness pass)
----------------------------------------------------------------------------
Two of these scalars were rebuilding from heuristics data the save ALREADY
stores, which the parse layer then dropped or sentinel-flagged.  Corrected:

  * ``intel_ledger`` now reads the STORED per-observer/per-target/per-category
    intel rows as its PRIMARY source (the save persists the live 0..100 level at
    ``countries.<OBS>.intel.intel`` -- a positional array decoded to a tag-keyed
    dict by the parse/live layer; see ``re_scratch/WIRE_scalars_v2.md``).  The
    old decryption+leak heuristic is now a documented FALLBACK only, used when
    the stored rows are not supplied.
  * ``research_speed`` and ``special_forces`` now EXCLUDE in-progress techs.  An
    active research-slot tech also appears in ``technologies`` with
    ``date == "1.1.1.1"`` and ``level == None`` (parse_tech REALITY NOTE);
    summing its ``research_speed_factor`` / ``special_forces_cap`` before it is
    completed over-counts.  See :func:`_completed_techs`.

----------------------------------------------------------------------------
MODIFIER INDEX
----------------------------------------------------------------------------
the game country modifiers (``research_speed_factor``, ``surrender_limit``,
``special_forces_cap``, ``*_intel_to_others``) are defined in the *static*
installed content (``common/ideas``, ``common/technologies``), not in the save.  The
save only tells us *which* ideas/spirits/laws/techs a country HAS.  So the
scalars take an optional ``modifier_index``: ``{source_name -> {modifier_key ->
summed_value}}`` built by :func:`build_modifier_index` from the mod's static
files.  When no index is supplied the modifier contribution is treated as 0
(documented degradation, not a crash) and ``modifier_index_present=False`` is
flagged in the result.

The mod (this repo's sibling ``../test``) OVERRIDES two special-forces defines:

    NDefines.NCountry.SPECIAL_FORCES_CAP_BASE = 0.00   (vanilla 0.05)
    NDefines.NCountry.SPECIAL_FORCES_CAP_MIN  = 9999    (vanilla 12)

=> in THIS mod the special-forces cap is effectively UNLIMITED.  See
:func:`special_forces` for how that is reported.
"""

from __future__ import annotations

import glob
import os
import re
from typing import Any, Dict, List, Optional, Set, Tuple

# ===========================================================================
# Modifier keys we resolve from the static installed content.
# ===========================================================================
#: global research-speed multiplier source key
RESEARCH_SPEED_KEY = "research_speed_factor"
#: surrender-threshold modifier source key (percent-of-VP offset)
SURRENDER_LIMIT_KEY = "surrender_limit"
#: vanilla base surrender threshold: a country capitulates near this fraction of
#: its VP held by the enemy.  source: the game NDefines.NCountry.SURRENDER_LIMIT_BASE
#: (~0.80; the mod's msb_defines.lua does NOT override it).  Confirmed via Codex
#: review: surrender progress = VP_lost_fraction / (SURRENDER_LIMIT_BASE +
#: surrender_limit), NOT / (1 + surrender_limit).
SURRENDER_LIMIT_BASE = 0.80
#: special-forces battalion-cap modifier source keys (factor + flat)
SPECIAL_FORCES_CAP_KEY = "special_forces_cap"
SPECIAL_FORCES_CAP_FLAT_KEY = "special_forces_cap_flat"
#: the four "intel leaked to others" modifier source keys, by intel category
INTEL_TO_OTHERS_KEYS = {
    "civilian": "civilian_intel_to_others",
    "army": "army_intel_to_others",
    "navy": "navy_intel_to_others",
    "airforce": "airforce_intel_to_others",
}

#: the four "intel I gather on others" modifier keys, by category (observer side)
INTEL_FACTOR_KEYS = {
    "civilian": "civilian_intel_factor",
    "army": "army_intel_factor",
    "navy": "navy_intel_factor",
    "airforce": "airforce_intel_factor",
}

#: per-category intel granted by a CRACKED cipher's decryption modifier, from
#: the mod's static modifiers.  source:
#: ../test/common/modifiers/00_static_modifiers.txt
#:   passive_decryption_modifier: *_intel_decryption_bonus = 10  (post-crack)
#:   active_decryption_modifier:  *_intel_decryption_bonus = 50  (post-crack)
#: NOTE (Codex review): both apply only AFTER a cipher is fully decrypted; a
#: cipher merely being WORKED (decryption=false, amount = progress) grants 0.
#: The fallback heuristic uses only the ACTIVE bonus, gated on decryption=true.
DECRYPTION_PASSIVE_BONUS = 10.0
DECRYPTION_ACTIVE_BONUS = 50.0

#: every modifier key the index extractor collects
MODIFIER_KEYS: Set[str] = {
    RESEARCH_SPEED_KEY,
    SURRENDER_LIMIT_KEY,
    SPECIAL_FORCES_CAP_KEY,
    SPECIAL_FORCES_CAP_FLAT_KEY,
    *INTEL_TO_OTHERS_KEYS.values(),
    *INTEL_FACTOR_KEYS.values(),
}

#: the game intel is tracked in exactly these four categories.
INTEL_CATEGORIES: Tuple[str, ...] = ("civilian", "army", "navy", "airforce")

# ---------------------------------------------------------------------------
# Special-forces battalions.
# ---------------------------------------------------------------------------
# source: the game ``common/units`` -- the engine counts SF by BATTALIONS whose
# sub_unit is flagged ``special_forces = yes``, NOT by division role.  The mod's
# SF roster (vanilla + ../test/common/units, mod wins) is the marine, marine
# commando, ranger, mountaineer, paratrooper and amphibious armor/mech families.
# ``designs.json`` division_templates expose each template's ``regiments`` (line
# battalions) by sub_unit ``type``; we count regiments whose type is an SF
# sub_unit, times the number of divisions on that template.  This is the
# BATTALION metric the engine uses (it picks up rangers / marine commandos /
# amphibious armor that the old division-``template_role`` proxy missed, and that
# can sit inside infantry/armor-role templates).
#
# Default roster (used when the live game-files roster is not supplied); built
# from a vanilla+mod scan of every ``common/units`` sub_unit flagged
# ``special_forces = yes`` -- see :func:`build_sf_subunit_set`.  Hardcoding the
# resolved set keeps the formula pure & game-file-free for the common case while
# :func:`build_sf_subunit_set` lets a caller re-derive it from the actual files.
SF_SUBUNIT_TYPES: Set[str] = {
    "marine", "marine_commando", "ranger_battalion", "mountaineers",
    "paratrooper", "hq_paratrooper",
    "amphibious_armor", "amphibious_light_armor", "amphibious_medium_armor",
    "amphibious_heavy_armor", "amphibious_mechanized",
}

# Legacy DIVISION-role set, kept for the documented division-granularity proxy
# that :func:`special_forces` falls back to when no template data is available.
SF_DIVISION_ROLES: Set[str] = {"marines", "mountaineers", "paratroopers"}

# ---------------------------------------------------------------------------
# Mod special-forces define overrides (see module docstring).
# source: ../test/common/defines/msb_defines.lua
#   SPECIAL_FORCES_CAP_BASE = 0.00 ; SPECIAL_FORCES_CAP_MIN = 9999
# ---------------------------------------------------------------------------
MOD_SF_CAP_BASE = 0.00
MOD_SF_CAP_MIN_BATTALIONS = 9999  # effectively unlimited
# vanilla reference values, for documentation / non-mod callers
VANILLA_SF_CAP_BASE = 0.05
VANILLA_SF_CAP_MIN_BATTALIONS = 12


# ===========================================================================
# Modifier-index builder (static installed content -> {source: {key: value}})
# ===========================================================================
_TOKEN_RE = re.compile(r'#[^\n]*|"[^"]*"|[{}]|[^\s{}#"]+')

# block-opener names that are STRUCTURAL, not a modifier-bearing definition
# (an idea/tech name); used to attribute a modifier to its owning definition.
_STRUCTURAL_BLOCKS = {
    "modifier", "limit", "if", "else", "else_if", "trigger", "available",
    "allowed", "allowed_to_remove", "removal_cost", "on_add", "on_remove",
    "and", "or", "not", "custom_trigger_tooltip", "hidden_trigger",
    "cancel_if_invalid", "targeted_modifier", "equipment_bonus",
    "research_bonus", "ai_will_do", "picture", "rule", "do_effect", "ideas",
    "add_idea", "country", "state", "path", "focus", "complete_tooltip",
    "show_equipment_icon", "dependencies", "categories", "country_event",
    "every_country", "any_country", "modifiers",
}


def _read_cp1252(path: str) -> str:
    """Read an AMSB game file as Windows-1252 (the mod's byte encoding)."""
    with open(path, "rb") as fh:
        return fh.read().decode("cp1252", "replace")


def _tokenize(text: str):
    for m in _TOKEN_RE.finditer(text):
        tok = m.group(0)
        if tok.startswith("#"):
            continue
        yield tok


def _extract_file(text: str) -> Dict[str, Dict[str, float]]:
    """Walk the brace tree of one game file.

    Returns ``{definition_name -> {modifier_key -> summed_value}}`` for every
    ``MODIFIER_KEYS`` numeric assignment, attributed to the nearest enclosing
    NON-structural block name (the idea/tech/spirit/law id).
    """
    toks = list(_tokenize(text))
    out: Dict[str, Dict[str, float]] = {}
    stack: List[Optional[str]] = []
    pending: Optional[str] = None
    i, n = 0, len(toks)
    while i < n:
        tok = toks[i]
        if tok == "{":
            stack.append(pending)
            pending = None
        elif tok == "}":
            if stack:
                stack.pop()
            pending = None
        else:
            if i + 1 < n and toks[i + 1] == "=":
                if i + 2 < n and toks[i + 2] == "{":
                    pending = tok  # this token opens a named block
                    i += 2
                    continue
                # flat 'key = value'
                key = tok
                val = toks[i + 2] if i + 2 < n else None
                if key in MODIFIER_KEYS and val is not None:
                    try:
                        fv = float(val)
                    except ValueError:
                        fv = None
                    if fv is not None:
                        owner = None
                        for name in reversed(stack):
                            if name and name.lower() not in _STRUCTURAL_BLOCKS:
                                owner = name
                                break
                        if owner:
                            out.setdefault(owner, {})
                            out[owner][key] = out[owner].get(key, 0.0) + fv
                i += 2
                continue
            pending = None
        i += 1
    return out


def build_modifier_index(
    common_dirs: Optional[List[str]] = None,
    subdirs: Tuple[str, ...] = ("ideas", "technologies", "country_leader",
                                "dynamic_modifiers", "modifiers"),
) -> Dict[str, Dict[str, float]]:
    """Build ``{source_name -> {modifier_key -> value}}`` from static installed content.

    ``common_dirs`` are paths to ``common`` directories (mod overrides FIRST so
    they win on duplicate names).  Defaults to this repo's sibling mod at
    ``../test/common`` resolved relative to this file -- repo-relative, never an
    absolute user path baked into the source.

    A later definition with the same name (e.g. a mod override of a vanilla
    idea) REPLACES the earlier one's per-key values, so list the mod dir first
    and any vanilla dir second.

    source: the game modifier model -- a country idea / national spirit / law /
    technology carries a ``modifier = { key = value ... }`` block; the country
    gets the SUM of every key over the ideas/techs it currently holds.
    """
    if common_dirs is None:
        here = os.path.dirname(os.path.abspath(__file__))
        common_dirs = [os.path.normpath(os.path.join(here, "..", "..", "test", "common"))]

    index: Dict[str, Dict[str, float]] = {}
    # process in reverse so the FIRST listed dir overrides later ones
    for common in reversed(common_dirs):
        for sub in subdirs:
            pattern = os.path.join(common, sub, "*.txt")
            for path in sorted(glob.glob(pattern)):
                try:
                    text = _read_cp1252(path)
                except OSError:
                    continue
                for name, kv in _extract_file(text).items():
                    index[name] = dict(kv)  # override (mod wins)
    return index


def _sum_modifier(
    sources: List[str],
    modifier_index: Optional[Dict[str, Dict[str, float]]],
    key: str,
) -> Tuple[float, List[Tuple[str, float]]]:
    """Sum ``key`` over ``sources`` using ``modifier_index``.

    Returns ``(total, [(source, value), ...])``.  Missing index -> ``(0.0, [])``.
    """
    if not modifier_index:
        return 0.0, []
    total = 0.0
    contribs: List[Tuple[str, float]] = []
    for src in sources:
        v = modifier_index.get(src, {}).get(key)
        if v:
            total += v
            contribs.append((src, v))
    return total, contribs


# ===========================================================================
# Completed-vs-in-progress tech filter (Codex finding: in-progress techs were
# being summed as completed).
# ===========================================================================
#: sentinel "date" the melt writes for a tech that is sitting in an active
#: research SLOT but is NOT yet completed.  source: parse_tech REALITY NOTE +
#: a real 1940 melt -- in-progress slot techs appear in ``technologies`` with
#: ``date == "1.1.1.1"`` and ``level == None``; completed techs carry a real
#: research date AND a level >= 1.
IN_PROGRESS_TECH_DATE = "1.1.1.1"


def _completed_techs(tdata: Dict[str, Any]) -> List[Dict[str, Any]]:
    """Return only the COMPLETED technologies from a parsed ``tech`` country rec.

    A tech is completed iff it has a real research date (not the
    ``IN_PROGRESS_TECH_DATE`` sentinel) AND a ``level`` is set.  In-progress
    research-slot techs (which the melt also lists in ``technologies`` with
    ``date == "1.1.1.1"`` / ``level == None``) are EXCLUDED so their modifiers
    are not counted before the tech is finished.
    """
    out: List[Dict[str, Any]] = []
    for t in tdata.get("technologies", []) or []:
        if not isinstance(t, dict):
            continue
        if t.get("level") is None:
            continue
        if t.get("date") == IN_PROGRESS_TECH_DATE:
            continue
        out.append(t)
    return out


def _completed_tech_names(tdata: Dict[str, Any]) -> List[str]:
    """Names of the completed technologies (see :func:`_completed_techs`)."""
    return [t.get("name") for t in _completed_techs(tdata) if t.get("name")]


# ===========================================================================
# Special-forces sub_unit roster builder (static common/units scan).
# ===========================================================================
def build_sf_subunit_set(
    common_dirs: Optional[List[str]] = None,
) -> Set[str]:
    """Scan ``common/units`` and return the sub_unit names flagged SF.

    Returns the set of sub_unit ids whose definition contains
    ``special_forces = yes`` across the supplied ``common`` dirs (mod files win
    over vanilla on duplicate names -- list the mod dir first).  Defaults to the
    repo's sibling mod at ``../test/common`` (repo-relative, never an absolute
    user path).  Falls back to :data:`SF_SUBUNIT_TYPES` semantics: if no files
    are found the caller may use that hardcoded resolved set.
    """
    if common_dirs is None:
        here = os.path.dirname(os.path.abspath(__file__))
        common_dirs = [os.path.normpath(os.path.join(here, "..", "..", "test", "common"))]

    found: Dict[str, bool] = {}
    # process in reverse so the FIRST listed dir wins on name collision
    for common in reversed(common_dirs):
        for path in sorted(glob.glob(os.path.join(common, "units", "*.txt"))):
            try:
                text = _read_cp1252(path)
            except OSError:
                continue
            for name, is_sf in _scan_subunits(text).items():
                found[name] = is_sf
    return {n for n, sf in found.items() if sf}


def _scan_subunits(text: str) -> Dict[str, bool]:
    """Parse one ``common/units`` file -> {sub_unit_name: has special_forces=yes}.

    Walks the ``sub_units = { NAME = { ... } ... }`` block with a brace counter;
    comments stripped.  Pure string work, no I/O.
    """
    text = re.sub(r"#[^\n]*", "", text)
    m = re.search(r"sub_units\s*=\s*\{", text)
    if not m:
        return {}
    i, depth, body_start = m.end(), 1, m.end()
    while i < len(text) and depth > 0:
        c = text[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
        i += 1
    body = text[body_start:i - 1]
    out: Dict[str, bool] = {}
    name_re = re.compile(r"([A-Za-z_][A-Za-z0-9_]*)\s*=\s*\{")
    j = 0
    while True:
        nm = name_re.search(body, j)
        if not nm:
            break
        name, k, d, s = nm.group(1), nm.end(), 1, nm.end()
        while k < len(body) and d > 0:
            if body[k] == "{":
                d += 1
            elif body[k] == "}":
                d -= 1
            k += 1
        sub = body[s:k - 1]
        out[name] = bool(re.search(r"\bspecial_forces\s*=\s*yes\b", sub))
        j = k
    return out


# ===========================================================================
# War-graph helper (Codex finding: capitulation counted ANY non-self controller
# as enemy; only ENEMY-at-war control should count toward surrender).
# ===========================================================================
def _enemies_of(tag: str, diplomacy: Optional[Dict[str, Any]]) -> Optional[Set[str]]:
    """Set of tags currently AT WAR with ``tag`` from ``diplomacy.json``.

    Reads both signals the diplomacy oracle exposes:
      * the de-duplicated ``wars`` list ({a, b, ...}); and
      * per-country ``relations[other].at_war`` booleans.
    Returns ``None`` when no diplomacy graph is supplied (=> caller falls back to
    the coarse "any non-self controller" rule, flagged as a limitation).  An
    empty set means "diplomacy present, this country is at war with nobody".
    """
    if not isinstance(diplomacy, dict):
        return None
    enemies: Set[str] = set()
    have_signal = False

    wars = diplomacy.get("wars")
    if isinstance(wars, list):
        have_signal = True
        for w in wars:
            if not isinstance(w, dict):
                continue
            a, b = w.get("a"), w.get("b")
            if a == tag and isinstance(b, str):
                enemies.add(b)
            elif b == tag and isinstance(a, str):
                enemies.add(a)

    cdata = diplomacy.get("countries")
    if isinstance(cdata, dict):
        rels = (cdata.get(tag, {}) or {}).get("relations")
        if isinstance(rels, dict):
            have_signal = True
            for other, rel in rels.items():
                if isinstance(rel, dict) and rel.get("at_war"):
                    enemies.add(other)

    return enemies if have_signal else None


# ===========================================================================
# 1. capitulation_progress  --  surrender / capitulation %  (0..100)
# ===========================================================================
def capitulation_progress(
    tag: str,
    state_detail: Dict[str, Any],
    economy: Optional[Dict[str, Any]] = None,
    modifier_index: Optional[Dict[str, Dict[str, float]]] = None,
    diplomacy: Optional[Dict[str, Any]] = None,
) -> Optional[Dict[str, Any]]:
    """Estimate a country's surrender / capitulation progress, 0..100 %.

    source (the game surrender mechanic; threshold per Codex review): a country
    capitulates when the *fraction of its national victory points held by the
    enemy* reaches a threshold ``SURRENDER_LIMIT_BASE (~0.80) + surrender_limit``
    (positive ``surrender_limit`` => harder to capitulate).  Progress toward
    that threshold is therefore:

        vp_lost_fraction = (sum of victory_points of states this country OWNS
                            but an ENEMY currently CONTROLS)
                           / (sum of victory_points of all states it OWNS)

        cap_pct = clamp(vp_lost_fraction / (SURRENDER_LIMIT_BASE
                                            + surrender_limit), 0, 1) * 100

    i.e. divide by ``0.80 + surrender_limit``, NOT by ``1 + surrender_limit``
    (corrected after Codex review).  ``surrender_limit`` is summed over the
    country's ideas/laws/spirits via ``modifier_index`` (key
    ``surrender_limit``).

    ENEMY CONTROL (corrected after Codex review): a lost state counts toward
    ``vp_lost_fraction`` only when its CONTROLLER is actually an ENEMY at war
    with ``tag``.  When ``diplomacy`` (parsed ``diplomacy.json``) is supplied we
    build the war graph (:func:`_enemies_of` over ``wars`` + ``relations.at_war``)
    and count VP only for controllers in it -- a state occupied by an ALLY or a
    CO-BELLIGERENT (military access, joint operations) is NOT counted, fixing the
    old over-count that treated ANY ``controller != owner`` as a loss.  When no
    diplomacy graph is supplied we DOCUMENT the degradation and fall back to the
    coarse "any non-self controller" rule (``enemy_detection='controller_neq_owner'``).

    MODELLING NOTES / what this does and does NOT capture:
      * VP loss is the dominant, save-derivable driver and the only component
        we compute exactly.  Omitted, save-non-derivable terms (=> ``cap_pct``
        is an estimate, flagged ``is_estimate=True`` / ``method='vp_only'``):
          - war casualties / manpower-loss contribution (no trusted per-country
            counter in the save);
          - ``surrender_limit`` from *active* sources NOT visible as country
            ideas in ``modifier_index`` -- country-leader traits, enabled
            dynamic modifiers (with their save ``value[]``), decision/static
            modifiers (e.g. ``war_support_bad_modifier``), timed decisions and
            faction rules.  ``modifier_index`` covers ideas/spirits/laws/techs
            only; the active-modifier surrender_limit is an acknowledged gap.
      * COLLABORATION is NOT surfaced here.  ``add_collaboration`` lowers an
        occupier's required garrison and can flip control of states, but it is
        not a ``surrender_limit`` modifier and the save does not expose it on
        this path, so this function does not model or claim to surface it.
      * Capital loss is not a separate capitulation trigger in modern the game (it
        is just a usually-large VP contributing to ``vp_lost_fraction``), so
        capital is not special-cased.

    Returns ``None`` if the country owns no states in ``state_detail`` (input
    absent -> no fabrication).  ``cap_pct`` is ``0.0`` for a healthy country.
    """
    states = state_detail.get("states") if isinstance(state_detail, dict) else None
    if not states:
        return None

    enemies = _enemies_of(tag, diplomacy)
    enemy_detection = "war_graph" if enemies is not None else "controller_neq_owner"

    owned_vp = 0.0
    enemy_vp = 0.0
    owned_states = 0
    for st in (states.values() if isinstance(states, dict) else states):
        if not isinstance(st, dict):
            continue
        if st.get("owner") != tag:
            continue
        owned_states += 1
        vp = st.get("victory_points") or 0
        owned_vp += vp
        ctrl = st.get("controller")
        if not ctrl or ctrl == tag:
            continue
        if enemies is not None:
            # diplomacy known: count ONLY genuine at-war enemy control.
            if ctrl in enemies:
                enemy_vp += vp
        else:
            # no war graph: coarse fallback (documented).
            enemy_vp += vp

    if owned_states == 0:
        return None

    vp_lost_fraction = (enemy_vp / owned_vp) if owned_vp > 0 else 0.0

    sl_total, sl_contribs = 0.0, []
    if economy is not None:
        ideas = (economy.get("countries", {}).get(tag, {}) or {}).get("ideas_all", []) or []
        sl_total, sl_contribs = _sum_modifier(ideas, modifier_index, SURRENDER_LIMIT_KEY)

    denom = SURRENDER_LIMIT_BASE + sl_total
    cap = vp_lost_fraction / denom if denom > 0 else vp_lost_fraction
    cap_pct = max(0.0, min(1.0, cap)) * 100.0

    return {
        "tag": tag,
        "cap_pct": round(cap_pct, 2),
        "vp_lost_fraction": round(vp_lost_fraction, 4),
        "owned_vp": round(owned_vp, 1),
        "enemy_controlled_vp": round(enemy_vp, 1),
        "owned_states": owned_states,
        "surrender_threshold": round(denom, 3),
        "surrender_limit_modifier": round(sl_total, 3),
        "surrender_limit_sources": sl_contribs,
        "enemy_detection": enemy_detection,
        "diplomacy_present": enemies is not None,
        "method": "vp_only",
        "is_estimate": True,  # casualty + active-modifier surrender_limit omitted
        "modifier_index_present": bool(modifier_index),
        "source_tag": "D",
    }


# ===========================================================================
# 2. research_speed  --  overall research-speed delta (%)
# ===========================================================================
def research_speed(
    tag: str,
    economy: Dict[str, Any],
    tech: Optional[Dict[str, Any]] = None,
    modifier_index: Optional[Dict[str, Dict[str, float]]] = None,
) -> Optional[Dict[str, Any]]:
    """Overall research-speed modifier for a country.

    source (the game research model): a country's research SPEED multiplier is

        speed = 1 + SUM(research_speed_factor over every idea / national spirit
                        / law / technology the country currently holds)

    reported here as a percentage delta ``sum * 100`` (e.g. +0.17 -> +17 %).

    The ``research_speed_factor`` sources are resolved from ``modifier_index``
    keyed on ``economy.countries[tag].ideas_all`` PLUS the COMPLETED technology
    names in ``tech.countries[tag].technologies`` (the computing-machine /
    mechanical-computing tech line is the single biggest research-speed source in
    the game and would be missed if we scanned ideas only).

    COMPLETED-ONLY (corrected after Codex review): a tech sitting in an active
    research SLOT also appears in ``technologies`` with ``date == "1.1.1.1"`` and
    ``level == None``; summing its ``research_speed_factor`` before it is
    finished over-counts.  We filter to completed techs via
    :func:`_completed_techs`.

    DELIBERATELY EXCLUDED (different mechanic, NOT a global speed factor):
      * ``tech.research_bonuses[]`` -- these are per-CATEGORY one-shot bonuses
        toward specific techs (e.g. armor +35 %), applied to a single research
        slot, not the global speed multiplier.  Reported separately as
        ``category_bonuses`` for visibility but never summed into ``pct``.
      * The ahead-of-time penalty and the doctrine/xp cost reductions: those
        change a tech's COST, not the global speed factor, and are not stored
        as a country modifier.
      * Extra research SLOTS (``research_slots``) -- parallelism, not speed;
        surfaced as ``research_slots`` for context only.

    MODIFIER-SOURCE COVERAGE (documented limitation): ``modifier_index`` covers
    ideas / national spirits / laws / technologies.  A ``research_speed_factor``
    coming from a country-leader TRAIT, an enabled DYNAMIC modifier, or a
    decision/static modifier is NOT in the index and is therefore not summed; the
    ideas+techs sum is a lower bound on the true factor for those rare cases.

    Returns ``None`` if the country is absent from ``economy``.
    """
    cdata = (economy.get("countries", {}) or {}).get(tag)
    if cdata is None:
        return None

    ideas = cdata.get("ideas_all", []) or []
    sources = list(ideas)
    slots = None
    category_bonus_count = 0
    in_progress_excluded = 0
    if tech is not None:
        tdata = (tech.get("countries", {}) or {}).get(tag, {}) or {}
        completed = _completed_tech_names(tdata)
        sources += completed
        in_progress_excluded = (
            len(tdata.get("technologies", []) or []) - len(completed))
        slots = tdata.get("research_slots")
        category_bonus_count = len(tdata.get("research_bonuses", []) or [])

    total, contribs = _sum_modifier(sources, modifier_index, RESEARCH_SPEED_KEY)

    return {
        "tag": tag,
        "pct": round(total * 100.0, 2),       # +% delta
        "multiplier": round(1.0 + total, 4),  # 1 + sum
        "factor_total": round(total, 4),
        "sources": sorted(contribs, key=lambda x: -abs(x[1])),
        "research_slots": slots,
        "category_bonuses_excluded": category_bonus_count,
        "in_progress_techs_excluded": in_progress_excluded,
        "modifier_index_present": bool(modifier_index),
        "source_tag": "D",
    }


# ===========================================================================
# 3. special_forces  --  cap & used
# ===========================================================================
def special_forces(
    tag: str,
    divisions: Dict[str, Any],
    economy: Optional[Dict[str, Any]] = None,
    tech: Optional[Dict[str, Any]] = None,
    modifier_index: Optional[Dict[str, Dict[str, float]]] = None,
    mod_overrides: bool = True,
    designs: Optional[Dict[str, Any]] = None,
    sf_subunit_types: Optional[Set[str]] = None,
) -> Optional[Dict[str, Any]]:
    """Special-forces cap and used count for a country, in BATTALIONS.

    source (the game special-forces mechanic; shape per Codex review):

        cap_battalions = max(
            SPECIAL_FORCES_CAP_MIN,
            total_battalions
                * (SPECIAL_FORCES_CAP_BASE + sum(special_forces_cap))
            + sum(special_forces_cap_flat) )

    where ``special_forces_cap`` (a fraction of total battalions) and
    ``special_forces_cap_flat`` (a flat battalion add) modifiers come from
    completed techs (e.g. the ``special_forces`` infantry tech, +0.10) and army
    spirits, summed via ``modifier_index``.

    THIS MOD (``mod_overrides=True``, the default) overrides the defines to
        SPECIAL_FORCES_CAP_BASE = 0.00,  SPECIAL_FORCES_CAP_MIN = 9999
    => the cap is effectively UNLIMITED.  We therefore report
    ``cap='unlimited'`` and ``cap_battalions=None`` for the mod, while still
    surfacing the modifier sum so a vanilla consumer can see it.  Pass
    ``mod_overrides=False`` to compute the vanilla-formula numeric cap instead.

    USED -- BATTALION count (corrected after Codex review): the engine counts
    BATTALIONS whose sub_unit is flagged ``special_forces = yes``, NOT divisions
    by role.  We join each of ``tag``'s divisions to its template in
    ``designs.json`` (``division_templates`` by ``(template_id, template_type)``)
    and sum the template ``regiments`` (line battalions) whose ``type`` is an SF
    sub_unit (:data:`SF_SUBUNIT_TYPES`, or a custom ``sf_subunit_types`` set
    derived from the live installed content via :func:`build_sf_subunit_set`).  This
    correctly picks up rangers, marine commandos and amphibious SF armor/mech
    that the old division-``template_role`` proxy under-counted -- including SF
    battalions inside templates whose role is infantry/armor.  Support companies
    are NOT counted (the SF cap is on line battalions).  Reported as
    ``used_battalions`` with ``granularity='battalion'`` and ``is_proxy=False``.

    FALLBACK (no template data): when ``designs`` is not supplied we cannot do
    the battalion join, so we degrade to the legacy DIVISION-role proxy
    (:data:`SF_DIVISION_ROLES`), reported as ``used_divisions`` /
    ``granularity='division'`` / ``is_proxy=True`` (an UNDER-count floor).  The
    assembler currently does not pass ``designs`` -- see
    ``re_scratch/WIRE_scalars_v2.md`` for the one-line wiring change to enable
    the exact battalion count.

    DATA-COVERAGE CAVEAT: the parsed ``divisions.json`` in this repo omits the
    SAVE PLAYER's own armies (the host tag whose perspective the save was
    melted from).  When a country has zero divisions in the parsed data we
    cannot tell "no army" from "not parsed", so the used/total figures are
    returned as ``None`` (no fabrication) with ``divisions_present=False``.

    MODIFIER-SOURCE COVERAGE (documented limitation): the cap-modifier sum draws
    on ideas + COMPLETED techs only; a ``special_forces_cap`` from a leader trait
    or an enabled dynamic modifier is not in ``modifier_index``.

    Returns ``None`` only if ``divisions`` has no ``divisions`` list at all.
    """
    div_list = divisions.get("divisions") if isinstance(divisions, dict) else None
    if div_list is None:
        return None

    sf_types = sf_subunit_types if sf_subunit_types is not None else SF_SUBUNIT_TYPES

    # --- template index (for the battalion join) -----------------------
    template_sf_bn = _template_sf_battalions(designs, sf_types)  # {(id,type): n}
    have_templates = template_sf_bn is not None

    used_battalions = 0
    used_divs_proxy = 0
    total_divs = 0
    for d in div_list:
        if d.get("owner") != tag:
            continue
        total_divs += 1
        # legacy division-role proxy (fallback / cross-check)
        if d.get("template_role") in SF_DIVISION_ROLES:
            used_divs_proxy += 1
        # battalion count via template join
        if have_templates:
            key = (d.get("template_id"), d.get("template_type"))
            used_battalions += template_sf_bn.get(key, 0)

    divisions_present = total_divs > 0

    # --- modifier sum (special_forces_cap from ideas + COMPLETED techs) -
    sources: List[str] = []
    if economy is not None:
        sources += (economy.get("countries", {}).get(tag, {}) or {}).get("ideas_all", []) or []
    if tech is not None:
        tdata = (tech.get("countries", {}) or {}).get(tag, {}) or {}
        sources += _completed_tech_names(tdata)
    cap_mod_sum, cap_mod_contribs = _sum_modifier(
        sources, modifier_index, SPECIAL_FORCES_CAP_KEY)
    cap_flat_sum, cap_flat_contribs = _sum_modifier(
        sources, modifier_index, SPECIAL_FORCES_CAP_FLAT_KEY)

    result: Dict[str, Any] = {
        "tag": tag,
        "total_divisions": total_divs if divisions_present else None,
        "special_forces_cap_modifier": round(cap_mod_sum, 3),
        "special_forces_cap_sources": cap_mod_contribs,
        "special_forces_cap_flat": round(cap_flat_sum, 2),
        "special_forces_cap_flat_sources": cap_flat_contribs,
        "divisions_present": divisions_present,
        "templates_present": have_templates,
        "modifier_index_present": bool(modifier_index),
        "source_tag": "D",
    }

    if have_templates:
        # exact engine metric: SF battalions
        result["used_battalions"] = used_battalions if divisions_present else None
        result["used"] = result["used_battalions"]
        result["used_divisions"] = used_divs_proxy if divisions_present else None
        result["granularity"] = "battalion"
        result["is_proxy"] = False
    else:
        # no template data -> division-role proxy floor
        result["used_battalions"] = None
        result["used_divisions"] = used_divs_proxy if divisions_present else None
        result["used"] = result["used_divisions"]
        result["granularity"] = "division"
        result["is_proxy"] = True

    if mod_overrides:
        result["cap"] = "unlimited"
        result["cap_battalions"] = None
        result["mod_define_note"] = (
            "SPECIAL_FORCES_CAP_BASE=0.00 / SPECIAL_FORCES_CAP_MIN=9999 "
            "(msb_defines.lua) -> effectively unlimited"
        )
    else:
        # vanilla numeric cap, in BATTALIONS.  total_battalions is the true
        # battalion count when templates are present; otherwise it degrades to a
        # division-count stand-in (flagged via templates_present=False).
        if divisions_present:
            total_battalions = (
                _total_battalions(tag, div_list, template_sf_bn, designs, sf_types)
                if have_templates else total_divs)
            base_cap = (VANILLA_SF_CAP_BASE + cap_mod_sum) * total_battalions + cap_flat_sum
            result["cap_battalions"] = round(
                max(VANILLA_SF_CAP_MIN_BATTALIONS, base_cap), 2)
            result["total_battalions"] = total_battalions if have_templates else None
        else:
            result["cap_battalions"] = None
        result["cap"] = result["cap_battalions"]

    return result


# ---------------------------------------------------------------------------
# special-forces template helpers
# ---------------------------------------------------------------------------
def _template_sf_battalions(
    designs: Optional[Dict[str, Any]],
    sf_types: Set[str],
) -> Optional[Dict[Tuple[Any, Any], int]]:
    """Map ``(template_id, template_type) -> count of SF line battalions``.

    Returns ``None`` when no ``designs`` (division-template) data is supplied so
    the caller can fall back to the division-role proxy.  Counts only
    ``regiments`` (line battalions) whose ``type`` is an SF sub_unit; support
    companies are excluded (SF cap is on line battalions).
    """
    if not isinstance(designs, dict):
        return None
    templates = designs.get("division_templates")
    if not isinstance(templates, list):
        return None
    out: Dict[Tuple[Any, Any], int] = {}
    for t in templates:
        if not isinstance(t, dict):
            continue
        n = 0
        for r in t.get("regiments", []) or []:
            if isinstance(r, dict) and r.get("type") in sf_types:
                n += 1
        out[(t.get("id"), t.get("type"))] = n
    return out


def _template_all_battalions(
    designs: Optional[Dict[str, Any]],
) -> Dict[Tuple[Any, Any], int]:
    """Map ``(template_id, template_type) -> total line battalions (regiments)``."""
    out: Dict[Tuple[Any, Any], int] = {}
    if not isinstance(designs, dict):
        return out
    for t in designs.get("division_templates", []) or []:
        if not isinstance(t, dict):
            continue
        out[(t.get("id"), t.get("type"))] = len(t.get("regiments", []) or [])
    return out


def _total_battalions(
    tag: str,
    div_list: List[Dict[str, Any]],
    template_sf_bn: Optional[Dict[Tuple[Any, Any], int]],
    designs: Optional[Dict[str, Any]],
    sf_types: Set[str],
) -> int:
    """Total line battalions across ``tag``'s divisions (for the vanilla cap)."""
    all_bn = _template_all_battalions(designs)
    total = 0
    for d in div_list:
        if d.get("owner") != tag:
            continue
        total += all_bn.get((d.get("template_id"), d.get("template_type")), 0)
    return total


# ===========================================================================
# 4. intel_ledger  --  per-category intel-estimate confidence
# ===========================================================================
#: decryption points that saturate the cipher-PROGRESS bar.  Per Codex review
#: this ``amount`` is progress toward cracking a cipher, NOT a linear ledger-
#: intel contribution; we surface it as ``cipher_progress`` (0..1) for context
#: and do not treat it as the intel value itself.  Tunable.
CRYPTO_SATURATION_POINTS = 50000.0
#: full-cipher per-category intel a fully-decrypted target grants (passive) and
#: an activated decryption grants (active), normalised to [0,1] by /100.
#: source: ../test/common/modifiers/00_static_modifiers.txt
#:   passive_decryption_modifier *_intel_decryption_bonus = 10 -> 0.10
#:   active_decryption_modifier  *_intel_decryption_bonus = 50 -> 0.50
DECRYPTION_PASSIVE_CONFIDENCE = DECRYPTION_PASSIVE_BONUS / 100.0  # 0.10
DECRYPTION_ACTIVE_CONFIDENCE = DECRYPTION_ACTIVE_BONUS / 100.0    # 0.50


def _stored_intel_row(
    intel: Dict[str, Any],
    observer: str,
    target: str,
    stored_rows: Optional[Dict[str, Any]] = None,
) -> Optional[Dict[str, float]]:
    """Return the STORED per-category intel level ``observer`` holds on ``target``.

    The save PERSISTS the live 0..100 per-category level at
    ``countries.<OBS>.intel.intel`` (a positional array by internal country
    index).  The parse / live layer is responsible for decoding that positional
    array to a TAG-keyed dict (the index->tag join is a cross-section step it
    owns -- see ``re_scratch/WIRE_scalars_v2.md``).  This function reads the
    ALREADY-tag-keyed result from, in priority order:

      1. an explicit ``stored_rows`` arg: ``{observer: {target: {civilian,
         army, navy, airforce}}}`` OR ``{target: {...}}`` (already scoped to the
         observer); OR
      2. the observer's own intel dict, looking for a tag-keyed
         ``intel`` / ``intel_levels`` / ``intel_rows`` mapping
         (``{TARGET: {civilian,...}}``) that a future parser may attach.

    Returns ``{civilian,army,navy,airforce}`` (floats, 0..100) or ``None`` when
    no stored row is available (=> caller falls back to the heuristic).  We never
    accept a raw POSITIONAL array here (a list) -- only a resolved tag-keyed map.
    """
    def _coerce_row(row: Any) -> Optional[Dict[str, float]]:
        if not isinstance(row, dict):
            return None
        out: Dict[str, float] = {}
        for cat in INTEL_CATEGORIES:
            v = row.get(cat)
            if isinstance(v, (int, float)):
                out[cat] = float(v)
        return out if out else None

    # 1) explicit stored_rows arg
    if isinstance(stored_rows, dict):
        scoped = stored_rows.get(observer)
        if isinstance(scoped, dict) and target in scoped:
            r = _coerce_row(scoped.get(target))
            if r is not None:
                return r
        if target in stored_rows:  # already observer-scoped
            r = _coerce_row(stored_rows.get(target))
            if r is not None:
                return r

    # 2) the observer's own intel dict (tag-keyed map a future parser attaches)
    countries = intel.get("countries") if isinstance(intel, dict) else None
    obs = countries.get(observer) if isinstance(countries, dict) else None
    if isinstance(obs, dict):
        for key in ("intel_levels", "intel_rows", "intel"):
            m = obs.get(key)
            if isinstance(m, dict) and target in m:
                r = _coerce_row(m.get(target))
                if r is not None:
                    return r
    return None


def intel_ledger(
    observer: str,
    target: str,
    intel: Dict[str, Any],
    economy: Optional[Dict[str, Any]] = None,
    modifier_index: Optional[Dict[str, Dict[str, float]]] = None,
    stored_rows: Optional[Dict[str, Any]] = None,
) -> Optional[Dict[str, Any]]:
    """How well ``observer`` can read ``target`` in each intel category (0..100).

    Categories (the game intel is tracked in exactly these four):
      * ``civilian`` -- industry / economy / manpower / convoy visibility
      * ``army``     -- land OOB / divisions
      * ``navy``     -- fleet composition / convoys
      * ``airforce`` -- air wings

    PRIMARY SOURCE -- STORED LEVEL (corrected per Codex review).  Contrary to an
    earlier docstring, the save DOES persist the live per-category intel level:
    ``countries.<OBS>.intel.intel`` is a 0..100 value per target per category.
    When that stored row is available (decoded to a tag-keyed map by the parse /
    live layer and supplied via ``stored_rows`` or attached to the observer's
    intel dict -- see :func:`_stored_intel_row` and
    ``re_scratch/WIRE_scalars_v2.md``), we read it DIRECTLY: ``confidence =
    level/100`` per category, ``method='stored_level'``, ``is_estimate=False``.
    This is a raw-read win -- no heuristic reconstruction needed.

    FALLBACK HEURISTIC -- only when no stored row exists (``method='heuristic'``,
    ``is_estimate=True``, a LOWER bound).  The the game intel level is built from:
      1. DECRYPTION -- the agency's static decryption modifier once a target's
         cipher is cracked/activated.  The save exposes
         ``cryptology.targets[] = {target, decryption(bool), amount}``; ``amount``
         is cracking PROGRESS (NOT a linear intel value, so never added as
         intel), and ``decryption=true`` flags an ACTIVATED (cracked) cipher.
         Per Codex review the passive +10 decryption intel applies only AFTER
         full decryption, so:
           - ``decryption=true`` (cracked/activated) -> active bonus +0.50;
           - cipher tracked but ``decryption=false`` (still cracking,
             ``amount`` only progress) -> 0 decryption intel (NOT +0.10);
         category-uniform (the save's ``amount`` is not category-split).
         ``cipher_progress`` is reported for context only.
      2. PASSIVE leakage -- the TARGET leaks intel via ``*_intel_to_others`` on
         its own ideas/spirits (per category) -> ``passive_leak`` (point values
         scaled by /100).  NEGATIVE leaks are KEPT (some ideas REDUCE leakage);
         the old code dropped them with a ``>0`` guard.
      3. OBSERVER gathering ``*_intel_factor`` on the observer's ideas amplifies
         intel gained; modelled as a multiplier ``1 + factor`` (negative factors
         kept too, clamped so the multiplier stays >= 0).
      4. Intel NETWORK / diplomatic / scout-radar intel -- NOT in the save on
         this path; left in ``unmodelled``.

    MODIFIER-SOURCE COVERAGE (documented limitation, fallback path): the leak /
    factor sums draw on ideas/spirits/laws via ``modifier_index``; an
    ``*_intel_to_others`` / ``*_intel_factor`` from a leader trait or enabled
    dynamic modifier is not in the index.

    Returns ``None`` if ``observer`` has no agency entry in ``intel`` AND no
    stored row is available (input absent).
    """
    stored = _stored_intel_row(intel, observer, target, stored_rows)

    countries = intel.get("countries") if isinstance(intel, dict) else None
    obs = countries.get(observer) if isinstance(countries, dict) else None
    if obs is None and stored is None:
        return None

    # --- cipher status vs target (progress + activated flag) -----------
    crypto_amount = 0.0
    decrypting = False
    found_cipher = False
    if isinstance(obs, dict):
        for t in (obs.get("cryptology", {}) or {}).get("targets", []) or []:
            if t.get("target") == target:
                crypto_amount = float(t.get("amount") or 0.0)
                decrypting = bool(t.get("decryption"))
                found_cipher = True
                break
    cipher_progress = (
        min(1.0, crypto_amount / CRYPTO_SATURATION_POINTS) if crypto_amount else 0.0)

    # decryption intel (per-category, category-uniform).  The engine grants a
    # PASSIVE +10 intel once a cipher is fully cracked and a larger ACTIVE +50
    # while decryption is toggled on.  The save exposes only ONE boolean
    # (``decryption``) per cipher target plus an ``amount`` (cracking PROGRESS),
    # with NO separate "cracked but passive" flag (verified on the 1940 melt:
    # every row is either decryption=true or still-in-progress).  So this binary
    # signal collapses to:
    #   * decryption=true  -> ACTIVE bonus (DECRYPTION_ACTIVE_CONFIDENCE, 0.50);
    #   * decryption=false -> 0 (still cracking; amount is progress, not intel).
    # The PASSIVE-only tier (DECRYPTION_PASSIVE_CONFIDENCE, 0.10) is NOT
    # distinguishable from the save's two-state signal and is therefore
    # unreachable on this fallback path -- documented, not silently approximated.
    # This whole branch is fallback-only; when the stored 0..100 level is present
    # it is used instead (method='stored_level').
    decryption_conf = DECRYPTION_ACTIVE_CONFIDENCE if decrypting else 0.0  # 0.50 / 0.0

    # --- agency decryption-boost upgrades (context only) ---------------
    upgrades = (obs.get("upgrades", {}) if isinstance(obs, dict) else {}) or {}
    boost_levels = sum(
        v for k, v in upgrades.items()
        if isinstance(v, (int, float)) and k.startswith("upgrade_decryption_boost")
    )

    # --- per-category sources ------------------------------------------
    target_ideas: List[str] = []
    observer_ideas: List[str] = []
    if economy is not None:
        target_ideas = (economy.get("countries", {}).get(target, {}) or {}).get("ideas_all", []) or []
        observer_ideas = (economy.get("countries", {}).get(observer, {}) or {}).get("ideas_all", []) or []

    used_stored = stored is not None
    per_category: Dict[str, Any] = {}
    for cat in INTEL_CATEGORIES:
        leak_total, leak_contribs = _sum_modifier(
            target_ideas, modifier_index, INTEL_TO_OTHERS_KEYS[cat])
        factor_total, factor_contribs = _sum_modifier(
            observer_ideas, modifier_index, INTEL_FACTOR_KEYS[cat])
        # keep NEGATIVE leaks (some ideas reduce leakage); clamp final to [0,1].
        passive_leak = max(-1.0, min(1.0, leak_total / 100.0))
        # observer factor multiplier (>= 0 so a strong negative cannot flip sign).
        factor_mult = max(0.0, 1.0 + factor_total)

        if used_stored:
            # raw read: the stored 0..100 level IS the confidence.
            confidence = max(0.0, min(1.0, float(stored[cat]) / 100.0)) if cat in stored else None
        else:
            base = decryption_conf + passive_leak
            confidence = max(0.0, min(1.0, base * factor_mult))

        per_category[cat] = {
            "confidence": (round(confidence, 3) if confidence is not None else None),
            "stored_level": (round(float(stored[cat]), 2)
                             if used_stored and cat in stored else None),
            "decryption_intel": round(decryption_conf, 3),
            "passive_leak": round(passive_leak, 3),
            "passive_leak_sources": leak_contribs,
            "observer_factor": round(factor_total, 3),
            "observer_factor_sources": factor_contribs,
        }

    return {
        "observer": observer,
        "target": target,
        "categories": per_category,
        "method": "stored_level" if used_stored else "heuristic",
        "cipher_progress": round(cipher_progress, 3),
        "crypto_amount": round(crypto_amount, 1),
        "cipher_tracked": found_cipher,
        "actively_decrypting": decrypting,
        "decryption_boost_levels": boost_levels,
        "is_estimate": not used_stored,
        "unmodelled": ([] if used_stored else
                       ["operative_network_strength", "infiltrated_assets",
                        "military_attache", "lend_lease_diplomatic_intel",
                        "faction_intel_sharing", "scout_radar_combat_intel",
                        "live_0_100_category_level", "per_category_crypto_split"]),
        "modifier_index_present": bool(modifier_index),
        "source_tag": "D",
    }


# ===========================================================================
# convenience aggregate (optional; used by tests + the wire doc)
# ===========================================================================
def compute_all_scalars(
    tag: str,
    *,
    state_detail: Optional[Dict[str, Any]] = None,
    economy: Optional[Dict[str, Any]] = None,
    tech: Optional[Dict[str, Any]] = None,
    divisions: Optional[Dict[str, Any]] = None,
    intel: Optional[Dict[str, Any]] = None,
    designs: Optional[Dict[str, Any]] = None,
    diplomacy: Optional[Dict[str, Any]] = None,
    modifier_index: Optional[Dict[str, Dict[str, float]]] = None,
) -> Dict[str, Any]:
    """Compute every per-country scalar for ``tag`` from whatever inputs exist.

    Each entry is ``None`` when its required input was not supplied.  Pure;
    does no I/O.  ``designs`` enables the exact SF-battalion count and
    ``diplomacy`` enables the enemy-only VP capitulation filter; both degrade
    gracefully (documented) when absent.
    """
    return {
        "capitulation": (
            capitulation_progress(tag, state_detail, economy, modifier_index,
                                  diplomacy)
            if state_detail is not None else None),
        "research_speed": (
            research_speed(tag, economy, tech, modifier_index)
            if economy is not None else None),
        "special_forces": (
            special_forces(tag, divisions, economy, tech, modifier_index,
                           designs=designs)
            if divisions is not None else None),
    }
