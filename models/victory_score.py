"""victory_score.py -- end-game VICTORY-SCORE / RANK-LADDER model.

Computes the per-country score ladder analogous to what the game's statistics /
"Scoreboard"-style menubar shows: army strength, navy strength, air strength,
industry, territory, plus an overall composite rank. READ-ONLY: consumes only
the parsed save_oracle JSON; never touches the game or the network.

------------------------------------------------------------------------------
WHY A *DOCUMENTED* FORMULA, NOT AN ORACLE
------------------------------------------------------------------------------
the game's base game ships no global ranking ledger. The familiar 8-branch ladder
(population, territory, politics, industry, technology, army, navy, air -> summed
into a "National Score / Power Points", then ordered strong->weak) comes from the
community "Scoreboard" mod (Steam Workshop id 2522217485). That mod's author
states only that "many variables are used" and explicitly that "all scores are
not actually representative because they are weighted differently" -- the exact
weights are NOT published. There is therefore NO public closed-form display
formula to match byte-for-byte (the schema would tag such a field W6 = display).

Consequence (and the standard we hold ourselves to): this module implements a
TRANSPARENT, documented proxy. Every component is defined from quantities that
genuinely drive in-game strength, every weight is named and justified inline,
and validation is "plausible relative ordering of the major powers" -- NOT an
exact value match. Treat absolute scores as ordinal, not cardinal.

------------------------------------------------------------------------------
COMPONENT DEFINITIONS  (cited; see CITATIONS dict at bottom)
------------------------------------------------------------------------------
ARMY  -- sum over the country's divisions of
            template_combat_weight(role) * (strength_frac) * (org_frac)
         where strength/org fractions are the division's current readiness vs.
         its template max. We approximate the per-division max strength/org from
         the observed fleet/army maxima (the game caps division strength at template
         and org likewise); we use raw `strength` (men present) scaled by a role
         combat weight, then nudge by org readiness. This mirrors how the game's own
         "army strength" estimate counts *combat-ready* manpower, weighting hard
         units (armor) above soft (infantry/garrison). See CITATIONS["army"].

NAVY  -- sum over ships of ship_class_weight(definition) * (strength_frac),
         where strength_frac = current hull strength / observed class-max. Capital
         ships (carrier/battleship/battle_cruiser) dominate; screens (destroyer)
         and subs are light. This mirrors the game naval-strength estimates that
         weight by hull class. See CITATIONS["navy"].

AIR   -- sum over air wings of plane_class_weight(plane_definition) *
         effective_planes (falling back to `count` when effective is null).
         effective_planes already discounts grounded/unsupplied planes, so it is
         the better readiness proxy. See CITATIONS["air"].

INDUSTRY -- weighted factory count:
            MIL_W*mil + DOCK_W*dockyard + CIV_W*civ  (+ small output bonus).
         Military + naval capacity weighted above civilian, matching the
         "industry score weights war production" intuition. Factories come from
         economy.json (authoritative, incl. offmap). A light bonus for actual
         daily output (production.json) rewards *used* not just *owned* capacity.
         See CITATIONS["industry"].

TERRITORY -- sum of victory_points the country *controls* (controller, falling
         back to owner) from state_detail.json. VP is the game's own territorial
         value metric. See CITATIONS["territory"].

OVERALL -- each component is min-max normalized to 0..100 across the live-country
         field, then combined with branch weights (army/navy/air/industry/
         territory). Countries are ranked by this composite descending. Branch
         weights are explicit module constants; absolute composite is ordinal.

All weights live in named constants below; tweak in one place. Every container
is always present; an unobtainable scalar is None, never guessed (DESIGN
no-fabrication). 0 means measured zero.
"""
from __future__ import annotations

import json
import os
from dataclasses import dataclass, field, asdict
from typing import Any, Dict, List, Optional, Tuple

# ---------------------------------------------------------------------------
# Optional import of a richer per-division strength model (parallel agent may
# add models/division_stats.py). IMPORT-GUARDED so we never hard-depend on it.
# ---------------------------------------------------------------------------
try:  # pragma: no cover - presence depends on a parallel build
    from models import division_stats as _division_stats  # type: ignore
except Exception:  # noqa: BLE001 - any failure -> fall back to the proxy below
    _division_stats = None

try:  # pragma: no cover
    from models import equipment_stats as _equipment_stats  # type: ignore
except Exception:  # noqa: BLE001
    _equipment_stats = None


# ===========================================================================
# WEIGHT TABLES  (the only knobs)
# ===========================================================================

# --- ARMY: per-division-role combat weight. Hard/mobile units count for more
#     combat power per man than soft/static ones. Anchored at infantry = 1.0.
#     Rationale (CITATIONS["army"]): armor/mech bring high hard-attack, breakthrough
#     and armor; garrisons/suppression bring almost no field-combat value.
ROLE_COMBAT_WEIGHT: Dict[Optional[str], float] = {
    "infantry": 1.0,
    "marines": 1.1,
    "mountaineers": 1.1,
    "motorized": 1.3,
    "mobile": 1.4,
    "paratroopers": 1.2,
    "armor": 2.2,
    "heavy_armor": 2.6,
    "garrison": 0.3,
    "suppression": 0.2,
    None: 0.8,  # untyped template -> conservative soft value
}
DEFAULT_ROLE_WEIGHT = 0.8

# `strength` = men present in a division; caps at the template max. Empirically the
# observed ceiling in these saves is exactly 625 (no division exceeds it), so 625 is
# a real full-template reference, not an invented one. We use RAW men as the army
# size term (so a full division ~> its men count) and modulate by org readiness.
DIV_STRENGTH_REF = 625.0
# ORG is NOT a 0..100 percentage in the parsed data: it is the absolute org value,
# whose template MAX varies (infantry max org ~50-70, tanks ~10-40). Observed range
# here is 0..~96, mean ~37. We therefore do NOT treat org as a clean 0..1 readiness;
# instead org_mult stays in a narrow [ORG_MULT_FLOOR..1.0] band keyed off org/ORG_REF
# so disorganized troops count somewhat less WITHOUT this absolute-scale mismatch
# dominating the score (Codex caveat: prefer current_org/max_org if ever available).
ORG_REF = 90.0          # a high-but-attainable org; division near this counts ~full
ORG_MULT_FLOOR = 0.6    # org=0 division still counts at 60% (it exists, can defend)

# --- NAVY: per-ship-class weight. Capitals dwarf screens; subs are cheap raiders.
#     Anchored at destroyer = 1.0. (CITATIONS["navy"])
#     NOTE on carriers (Codex caveat, double-count): a carrier's embarked aircraft
#     are ALSO counted in the AIR component via cv_* plane definitions. So the carrier
#     HULL weight here represents deck/positioning/hull value ONLY, deliberately kept
#     below where it would be if it bundled the air group -> 10.0, not ~16.
SHIP_CLASS_WEIGHT: Dict[str, float] = {
    "destroyer": 1.0,
    "submarine": 1.2,
    "light_cruiser": 3.0,
    "heavy_cruiser": 5.0,
    "battle_cruiser": 9.0,
    "battleship": 11.0,
    "carrier": 10.0,  # hull/deck only; CV air group counted separately in AIR
}
DEFAULT_SHIP_WEIGHT = 1.0
# Ship `strength` is ABSOLUTE hull HP (observed 0..~518), NOT a 0..100 %. Bigger hulls
# have more HP, but that capital-vs-screen size gap is ALREADY captured by the class
# weight above -> double-counting it would inflate capitals twice. We therefore use
# strength only as a *damage* readiness fraction vs the ship's own class-max hull,
# learned per save (a half-sunk battleship counts ~half). See _ship_hull_refs().

# --- AIR: per-plane-class weight. Anchored at fighter = 1.0. Multirole/strategic
#     planes are individually heavier; CV variants tie up carrier capacity.
#     (CITATIONS["air"])
PLANE_CLASS_WEIGHT: Dict[str, float] = {
    "fighter": 1.0,
    "heavy_fighter": 1.3,
    "cas": 1.1,
    "tac_bomber": 1.5,
    "nav_bomber": 1.2,
    "strat_bomber": 2.2,
    "scout_plane": 0.4,
    "cv_fighter": 1.2,
    "cv_nav_bomber": 1.3,
    "cv_cas": 1.2,
}
DEFAULT_PLANE_WEIGHT = 1.0

# --- INDUSTRY: factory weights. War capacity (mil + dock) above civilian.
#     (CITATIONS["industry"])  Factory COUNT from economy.json is the authoritative,
#     clean industrial-base metric (incl. offmap). We deliberately do NOT add a daily-
#     output term: production.json's `produced` is per-item BUILD-PROGRESS (resets per
#     item, ~0..N, NOT a daily rate or a lifetime total) and `speed` is correlated with
#     the mil-factory count already weighted here -> any output bonus would be either
#     mis-derived or double-counting (Codex caveat). OUTPUT_W kept at 0 by default.
MIL_FACTORY_W = 1.6
DOCKYARD_W = 1.3
CIV_FACTORY_W = 1.0
OUTPUT_W = 0.0  # disabled: see comment above. Use `speed` (units/day) if ever re-enabled.

# --- OVERALL composite: branch weights for the normalized 0..100 components.
#     Military weighted heavily, industry/territory as the economic base.
BRANCH_WEIGHTS: Dict[str, float] = {
    "army": 0.30,
    "navy": 0.15,
    "air": 0.15,
    "industry": 0.25,
    "territory": 0.15,
}


# ===========================================================================
# DATA LOADING
# ===========================================================================

_REQUIRED = ("divisions", "oob", "economy", "state_detail")
_OPTIONAL = ("production", "country_snapshot")


def _load_json(output_dir: str, name: str) -> Optional[Any]:
    path = os.path.join(output_dir, name + ".json")
    if not os.path.exists(path):
        return None
    with open(path, "r", encoding="utf-8") as fh:
        return json.load(fh)


def load_inputs(output_dir: str) -> Dict[str, Any]:
    """Load the JSON inputs from an output directory. Missing optional files -> None.

    Raises FileNotFoundError if a *required* input is absent.
    """
    data: Dict[str, Any] = {}
    for name in _REQUIRED:
        d = _load_json(output_dir, name)
        if d is None:
            raise FileNotFoundError(
                f"required input '{name}.json' not found in {output_dir!r}"
            )
        data[name] = d
    for name in _OPTIONAL:
        data[name] = _load_json(output_dir, name)
    return data


# ===========================================================================
# COMPONENT SCORERS  (each returns {tag: float})
# ===========================================================================

def _clamp01(x: float) -> float:
    if x < 0.0:
        return 0.0
    if x > 1.0:
        return 1.0
    return x


def army_strength(divisions_json: Dict[str, Any]) -> Dict[str, float]:
    """Per-country army strength score from divisions.json.

    If a richer models.division_stats is importable, defer to it; otherwise use
    the role-weighted, readiness-scaled proxy described in the module docstring.
    """
    out: Dict[str, float] = {}
    divs = divisions_json.get("divisions", []) or []

    # Optional richer model -----------------------------------------------------
    if _division_stats is not None:
        fn = getattr(_division_stats, "division_combat_value", None)
        if callable(fn):
            for d in divs:
                owner = d.get("owner")
                if not owner:
                    continue
                try:
                    val = float(fn(d))
                except Exception:  # noqa: BLE001 - never let the optional model break us
                    val = None  # type: ignore
                if val is None:
                    break  # bail to proxy for ALL (consistency)
                out[owner] = out.get(owner, 0.0) + val
            else:
                return out  # completed cleanly with the richer model
            out.clear()  # fell through -> recompute with proxy

    # Proxy ---------------------------------------------------------------------
    for d in divs:
        owner = d.get("owner")
        if not owner:
            continue
        role = d.get("template_role")
        w = ROLE_COMBAT_WEIGHT.get(role, DEFAULT_ROLE_WEIGHT)
        strength = d.get("strength")
        org = d.get("organisation")
        if strength is None:
            continue
        # men-equivalent combat value: RAW men present, scaled by role, modulated by a
        # NARROW org-readiness band [ORG_MULT_FLOOR..1.0] (org is an absolute, not %,
        # scale -> we keep its influence mild so it can't dominate the army size term).
        org_frac = _clamp01(float(org) / ORG_REF) if org is not None else 0.8
        org_mult = ORG_MULT_FLOOR + (1.0 - ORG_MULT_FLOOR) * org_frac
        out[owner] = out.get(owner, 0.0) + w * float(strength) * org_mult
    return out


def _ship_hull_refs(oob_json: Dict[str, Any]) -> Dict[str, float]:
    """Per-class reference hull strength = max observed `strength` for that class.

    Lets us convert a ship's absolute hull HP into a 0..1 *damage* readiness frac vs.
    a healthy ship of its own class, instead of mis-treating absolute HP as a percent
    (which would double-count the capital-vs-screen size gap already in the weights).
    """
    refs: Dict[str, float] = {}
    for fleet in oob_json.get("fleets", []) or []:
        for tf in fleet.get("task_forces", []) or []:
            for ship in tf.get("ships", []) or []:
                definition = ship.get("definition")
                s = ship.get("strength")
                if definition is None or s is None:
                    continue
                if s > refs.get(definition, 0.0):
                    refs[definition] = float(s)
    return refs


def navy_strength(oob_json: Dict[str, Any]) -> Dict[str, float]:
    """Per-country navy strength score from oob.json fleets/task_forces/ships."""
    out: Dict[str, float] = {}
    hull_refs = _ship_hull_refs(oob_json)
    for fleet in oob_json.get("fleets", []) or []:
        for tf in fleet.get("task_forces", []) or []:
            tf_owner = tf.get("owner") or fleet.get("owner")
            for ship in tf.get("ships", []) or []:
                owner = ship.get("owner") or tf_owner
                if not owner:
                    continue
                definition = ship.get("definition")
                w = SHIP_CLASS_WEIGHT.get(definition, DEFAULT_SHIP_WEIGHT)
                strength = ship.get("strength")
                # damage readiness vs a healthy ship of the SAME class (not absolute %).
                ref = hull_refs.get(definition, 0.0)
                if strength is None or ref <= 0.0:
                    frac = 1.0  # unknown -> count as intact
                else:
                    frac = _clamp01(float(strength) / ref)
                out[owner] = out.get(owner, 0.0) + w * frac
    return out


def air_strength(oob_json: Dict[str, Any]) -> Dict[str, float]:
    """Per-country air strength score from oob.json air_wings."""
    out: Dict[str, float] = {}
    for wing in oob_json.get("air_wings", []) or []:
        owner = wing.get("owner")
        if not owner:
            continue
        definition = wing.get("plane_definition")
        w = PLANE_CLASS_WEIGHT.get(definition, DEFAULT_PLANE_WEIGHT)
        eff = wing.get("effective_planes")
        if eff is None:
            eff = wing.get("count")
        if eff is None:
            continue
        out[owner] = out.get(owner, 0.0) + w * float(eff)
    return out


def industry_score(
    economy_json: Dict[str, Any],
    production_json: Optional[Dict[str, Any]] = None,
) -> Dict[str, float]:
    """Per-country industry score: weighted factory count (the clean base metric).

    An optional output term is wired but DISABLED (OUTPUT_W=0) -- see the constant's
    comment: production.json `produced` is per-item build-progress, not a daily rate,
    and `speed` correlates with the factory count already weighted here. Pass a
    production_json only matters if OUTPUT_W is set > 0 and you switch to `speed`.
    """
    out: Dict[str, float] = {}
    countries = economy_json.get("countries", {}) or {}

    # Optional used-capacity term (off by default). Uses line `speed` = units/day,
    # the only genuine daily-rate field; gated on OUTPUT_W so it adds nothing at 0.
    output_by: Dict[str, float] = {}
    if production_json and OUTPUT_W:
        for tag, pc in (production_json.get("countries", {}) or {}).items():
            tot = 0.0
            for line in pc.get("equipment_lines", []) or []:
                sp = line.get("speed")
                if sp:
                    tot += float(sp)
            output_by[tag] = tot

    for tag, ec in countries.items():
        fac = ec.get("factories", {}) or {}
        civ = float(fac.get("civ", 0) or 0)
        mil = float(fac.get("mil", 0) or 0)
        dock = float(fac.get("dockyard", 0) or 0)
        score = MIL_FACTORY_W * mil + DOCKYARD_W * dock + CIV_FACTORY_W * civ
        if OUTPUT_W:
            score += OUTPUT_W * output_by.get(tag, 0.0)
        out[tag] = score
    return out


def territory_score(state_detail_json: Dict[str, Any]) -> Dict[str, float]:
    """Per-country territory score: victory points the country CONTROLS."""
    out: Dict[str, float] = {}
    states = state_detail_json.get("states", {}) or {}
    iterable = states.values() if isinstance(states, dict) else states
    for st in iterable:
        holder = st.get("controller") or st.get("owner")
        if not holder:
            continue
        vp = st.get("victory_points") or 0
        out[holder] = out.get(holder, 0.0) + float(vp)
    return out


# ===========================================================================
# NORMALIZATION + COMPOSITE
# ===========================================================================

def _minmax_normalize(
    values: Dict[str, float], scale: float = 100.0, *, log: bool = True
) -> Dict[str, float]:
    """Normalize a component to 0..scale across the country field.

    Empty / all-equal fields -> all zeros (no spurious spread).

    log=True (default) applies log1p BEFORE min-max. Codex caveat: a single dominant
    power (e.g. Soviet army, US industry) under raw min-max compresses every other
    country toward 0, so the branch weights end up acting over a near-degenerate
    spread. log1p tames that outlier so the mid-field stays distinguishable while the
    leader still tops the branch. This makes scores RELATIVE-TO-THIS-SAVE ordinal
    measures (documented), not cardinal. Set log=False for plain linear min-max.
    """
    if not values:
        return {}
    if log:
        import math

        vals = {k: math.log1p(max(0.0, v)) for k, v in values.items()}
    else:
        vals = values
    lo = min(vals.values())
    hi = max(vals.values())
    span = hi - lo
    if span <= 0:
        return {k: 0.0 for k in vals}
    return {k: scale * (v - lo) / span for k, v in vals.items()}


@dataclass
class CountryScore:
    tag: str
    army: float = 0.0
    navy: float = 0.0
    air: float = 0.0
    industry: float = 0.0
    territory: float = 0.0
    # normalized 0..100 components
    army_n: float = 0.0
    navy_n: float = 0.0
    air_n: float = 0.0
    industry_n: float = 0.0
    territory_n: float = 0.0
    overall: float = 0.0
    rank: Optional[int] = None

    def as_dict(self) -> Dict[str, Any]:
        return asdict(self)


# Synthetic / non-playable tags we exclude from the ladder by default.
#   WST = wasteland holder; D01..D75 = dormant releasable pool.
def _is_synthetic(tag: str) -> bool:
    if tag == "WST":
        return True
    if len(tag) == 3 and tag[0] == "D" and tag[1:].isdigit():
        return True
    return False


@dataclass
class ScoreLadder:
    countries: Dict[str, CountryScore]
    ranking: List[str]  # tags, best -> worst
    branch_weights: Dict[str, float]
    meta: Dict[str, Any]

    def as_dict(self) -> Dict[str, Any]:
        return {
            "meta": self.meta,
            "branch_weights": self.branch_weights,
            "ranking": self.ranking,
            "countries": {k: v.as_dict() for k, v in self.countries.items()},
        }

    def top(self, n: int = 12) -> List[CountryScore]:
        return [self.countries[t] for t in self.ranking[:n]]


def compute_victory_score(
    output_dir: str,
    *,
    include_synthetic: bool = False,
    branch_weights: Optional[Dict[str, float]] = None,
    log_normalize: bool = True,
) -> ScoreLadder:
    """Compute the full per-country score ladder from a save_oracle output dir.

    log_normalize (default True): log1p-then-min-max per component, taming dominant
    outliers so the mid-field stays distinguishable (see _minmax_normalize).
    """
    data = load_inputs(output_dir)
    weights = dict(branch_weights or BRANCH_WEIGHTS)

    army = army_strength(data["divisions"])
    navy = navy_strength(data["oob"])
    air = air_strength(data["oob"])
    industry = industry_score(data["economy"], data.get("production"))
    territory = territory_score(data["state_detail"])

    # Universe of tags = anything appearing in any component, filtered.
    all_tags = set()
    for comp in (army, navy, air, industry, territory):
        all_tags.update(comp.keys())
    if not include_synthetic:
        all_tags = {t for t in all_tags if not _is_synthetic(t)}

    raw_army = {t: army.get(t, 0.0) for t in all_tags}
    raw_navy = {t: navy.get(t, 0.0) for t in all_tags}
    raw_air = {t: air.get(t, 0.0) for t in all_tags}
    raw_ind = {t: industry.get(t, 0.0) for t in all_tags}
    raw_ter = {t: territory.get(t, 0.0) for t in all_tags}

    n_army = _minmax_normalize(raw_army, log=log_normalize)
    n_navy = _minmax_normalize(raw_navy, log=log_normalize)
    n_air = _minmax_normalize(raw_air, log=log_normalize)
    n_ind = _minmax_normalize(raw_ind, log=log_normalize)
    n_ter = _minmax_normalize(raw_ter, log=log_normalize)

    countries: Dict[str, CountryScore] = {}
    for t in all_tags:
        cs = CountryScore(
            tag=t,
            army=round(raw_army[t], 3),
            navy=round(raw_navy[t], 3),
            air=round(raw_air[t], 3),
            industry=round(raw_ind[t], 3),
            territory=round(raw_ter[t], 3),
            army_n=round(n_army.get(t, 0.0), 3),
            navy_n=round(n_navy.get(t, 0.0), 3),
            air_n=round(n_air.get(t, 0.0), 3),
            industry_n=round(n_ind.get(t, 0.0), 3),
            territory_n=round(n_ter.get(t, 0.0), 3),
        )
        cs.overall = round(
            weights["army"] * cs.army_n
            + weights["navy"] * cs.navy_n
            + weights["air"] * cs.air_n
            + weights["industry"] * cs.industry_n
            + weights["territory"] * cs.territory_n,
            3,
        )
        countries[t] = cs

    ranking = sorted(countries, key=lambda t: countries[t].overall, reverse=True)
    for i, t in enumerate(ranking, start=1):
        countries[t].rank = i

    meta_src = data["divisions"].get("meta", {}) if isinstance(data["divisions"], dict) else {}
    meta = {
        "model": "victory_score",
        "model_version": "1",
        "save_date": meta_src.get("save_date"),
        "save_version": meta_src.get("save_version"),
        "n_countries": len(countries),
        "include_synthetic": include_synthetic,
        "log_normalize": log_normalize,
        "note": (
            "Documented PROXY ladder. No public the game display formula exists; "
            "absolute scores are ordinal, not cardinal. See module docstring + CITATIONS."
        ),
    }
    return ScoreLadder(
        countries=countries, ranking=ranking, branch_weights=weights, meta=meta
    )


# ===========================================================================
# CITATIONS  (component-definition provenance)
# ===========================================================================
CITATIONS: Dict[str, str] = {
    "model": (
        "Community 'Scoreboard' mod (Steam Workshop id 2522217485) defines the "
        "8-branch National-Score ladder (population/territory/politics/industry/"
        "technology/army/navy/air summed, then ranked strong->weak). Its author "
        "states the exact branch weights are undisclosed and 'not actually "
        "representative because they are weighted differently' -> no public display "
        "formula to match; we implement a transparent proxy instead."
    ),
    "army": (
        "the game land-unit combat is driven by battalion type: armor/mechanized bring "
        "high hard-attack/breakthrough/armor while infantry is soft and garrisons/"
        "suppression bring ~no field-combat value (the game wiki: Land units). Division "
        "'strength' is current men present (caps at template max). We weight men by "
        "role and modulate by organisation readiness."
    ),
    "navy": (
        "the game naval power scales by hull class: capital ships (carrier/battleship/"
        "battlecruiser) vastly outweigh screens (destroyer/light cruiser) and "
        "submarines are cheap raiders (the game wiki: Naval warfare / ship classes). "
        "Ship 'strength' is ABSOLUTE hull HP (observed 0..~518, varies by class), so "
        "we convert it to a 0..1 DAMAGE readiness vs the per-class healthy-hull max "
        "(carrier hull weight excludes its air group, which AIR counts separately). "
        "Caveat (Codex): per-class max-as-ref is self-referential -- if an entire "
        "class is uniformly damaged the ref drops with it; this only nudges the minor "
        "readiness factor, the dominant capital-vs-screen gap stays in the class weight."
    ),
    "air": (
        "Air strength scales with effective plane count per wing weighted by plane "
        "type; 'effective_planes' already discounts grounded/unsupplied airframes "
        "(parsed live OOB), making it the better readiness proxy than raw 'count'."
    ),
    "industry": (
        "the game industry = factories: military + naval (dockyard) capacity drives war "
        "output and is weighted above civilian capacity, which builds/trades (the game "
        "wiki: Production). A DELIBERATELY SMALL bonus (OUTPUT_W=0.15) rewards actual "
        "daily output. Caveat (Codex): output is correlated with factory count "
        "(produced BY the mil factories already counted), so it is only a light "
        "used-capacity tiebreaker, not an independent term -- kept tiny on purpose."
    ),
    "territory": (
        "Victory points are the game's own per-state territorial-value metric (used for "
        "capitulation %). Summed over states the country CONTROLS (controller, "
        "falling back to owner) from state_detail.json."
    ),
}


# ===========================================================================
# CLI  (manual inspection; no side effects on import)
# ===========================================================================
def _format_ladder(ladder: ScoreLadder, n: int = 15) -> str:
    lines = []
    m = ladder.meta
    lines.append(
        f"victory_score ladder  (save {m.get('save_date')}, "
        f"v{m.get('save_version')}, {m['n_countries']} countries)"
    )
    lines.append(
        f"{'rk':>2}  {'tag':<4} {'OVERALL':>8} | "
        f"{'army':>7} {'navy':>6} {'air':>7} {'indus':>6} {'terr':>6}"
    )
    lines.append("-" * 64)
    for cs in ladder.top(n):
        lines.append(
            f"{cs.rank:>2}  {cs.tag:<4} {cs.overall:>8.2f} | "
            f"{cs.army:>7.0f} {cs.navy:>6.1f} {cs.air:>7.0f} "
            f"{cs.industry:>6.0f} {cs.territory:>6.0f}"
        )
    return "\n".join(lines)


def main(argv: Optional[List[str]] = None) -> int:
    import argparse

    ap = argparse.ArgumentParser(description="Compute the game victory-score ladder.")
    ap.add_argument(
        "output_dir",
        nargs="?",
        default=os.path.join("save_oracle", "output"),
        help="save_oracle output dir containing the parsed JSON (default: save_oracle/output)",
    )
    ap.add_argument("--top", type=int, default=15, help="rows to print")
    ap.add_argument(
        "--include-synthetic",
        action="store_true",
        help="include WST / D01..D75 dormant tags",
    )
    ap.add_argument("--json", action="store_true", help="dump full ladder as JSON")
    args = ap.parse_args(argv)

    ladder = compute_victory_score(
        args.output_dir, include_synthetic=args.include_synthetic
    )
    if args.json:
        print(json.dumps(ladder.as_dict(), indent=1))
    else:
        print(_format_ladder(ladder, n=args.top))
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
