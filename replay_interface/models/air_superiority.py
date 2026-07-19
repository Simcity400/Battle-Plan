"""air_superiority.py -- DISPLAYED air-superiority PERCENT per strategic region.

Reproduces the number the game shows in the "Air" map mode / region tooltip
(e.g. "Enemy air superiority is at 42%"). READ-ONLY: consumes only the parsed
save_oracle JSON (and, optionally, the game's equipment defs for exact per-plane
weights); never touches the live game or the network.

SOURCE BADGE: this is a MODEL estimate (schema source "W6" = a recomputed display
formula, not a stored oracle value). The per-region percentage is a transient
contest the engine recomputes each tick from the air wings present; it is never
persisted as an object, so we reproduce the formula rather than read a value.
Every output dict carries `"source": "MODEL"`.

==============================================================================
THE FORMULA  (and how each piece was established)
==============================================================================
For each deployed air wing on a region-bearing mission the engine tracks an
`effective_air_superiority` (EAS) value. The region's per-side score is the SUM
of its wings' EAS; the DISPLAYED percent for a given viewing side is

      pct(side) = 100 * score(side) / ( score(side) + score(opposing) )

clamped to [0, 100]. (Friendly-only region -> ~100%; both-zero -> undefined,
returned as None, never a divide-by-zero.)

PER-WING SCORE.  Empirically (melted 1946 save, `effective_air_superiority`
field present on 322 region-mission wings), per wing:

      EAS_wing = effective_planes * air_superiority_stat(plane)

The measured ratio EAS / effective_planes_count is EXACTLY the airframe's
`air_superiority` equipment stat, by plane class:

      fighter        1.000      cv_fighter      1.000
      cas            1.000      cv_cas          1.000
      tac_bomber     1.000      cv_nav_bomber   1.000
      nav_bomber     1.000      heavy_fighter   1.250
      strat_bomber   0.010

which matches `common/units/equipment` air_superiority stats: fighter airframe
= 1, twin-engine / heavy fighter = 1.25, strategic / intercontinental bomber =
0.01, quad-engine bomber = 0. So the per-plane weight is the PLANE/EQUIPMENT
air_superiority STAT -- NOT a per-mission-type weight. (A fighter flown on a CAS
mission still contributes ratio 1.0; the mission KIND only gates whether the
wing is in the regional pool, it does not rescale the value.) "Fighters count
fully, other types partially" is therefore a statement about PLANE TYPE.

`effective_planes` (the operational plane count after efficiency / range / fuel
discounting) is the count side of the product, NOT raw `count` (total strength).
This is the same `effective_planes` the live memory reader pins at wing+0x160 and
the proxy `naval_air.enumerate_air_superiority()` sums.

AGILITY / AIR_ATTACK / AIR_DEFENCE do NOT enter the displayed superiority number
(they drive air-COMBAT plane-loss resolution, a separate system). The save oracle
shows EAS = planes * stat with no agility/attack term; Codex concurred.

CONSTANTS (cited inline; see CITATIONS at bottom):
  - air_superiority equipment stat: common/units/equipment/*airframe*.txt.
  - AIR_REGION_SUPERIORITY_PIXEL_SCALE = 0.04 -- map TEXTURE intensity only
    (superiority/(pixels*scale)); does NOT affect the tooltip percent.
  - AIR_SUPERIORITY_FACTOR = 2.5 -- an AI mission-scoring factor, not the display.

==============================================================================
WHAT THIS MODEL IMPROVES OVER THE PROXY
==============================================================================
The existing proxy (`memory_reader/naval_air` + the `naval_air.air_superiority`
sidecar) sums raw `effective_planes` per (region, owner) -- correct ONLY while
every plane's air_superiority stat is 1. This model applies the per-plane stat,
so a region defended by strat bombers (stat 0.01) or contested by heavy fighters
(stat 1.25) gets the correct weighting, and it computes the actual contested
PERCENT (own/(own+enemy)) rather than just the leader's plane sum.

VALIDATION: see models/tests/test_air_superiority.py. There is no exact whole-
region oracle (the percent is a display formula, schema W6); validation is:
(1) per-wing EAS reproduced vs the save's `effective_air_superiority` where that
field is recoverable, (2) ordering vs the proxy (effective_planes sum) on
single-stat regions must match, (3) a friendly-only region -> ~100%, a contested
region -> strictly between 0 and 100, (4) Codex refutation (incorporated).
"""
from __future__ import annotations

import json
import os
import re
from dataclasses import dataclass, field, asdict
from typing import Any, Dict, List, Optional, Tuple

# ===========================================================================
# PER-PLANE air_superiority STAT  (the weight table -- the only real knob)
# ===========================================================================
# Keyed by the oob.json wing `plane_definition`. Values are the vanilla airframe
# `air_superiority` equipment stat, CONFIRMED save-exact (the EAS/effective_planes
# ratio measured over 322 region-mission wings == these numbers). For modded /
# upgraded variants prefer the exact stat resolved from the equipment defs
# (`load_air_superiority_stats`), which overrides this table when available.
#
# Sources: common/units/equipment/single_engine_airframe.txt (fighter/cas = 1),
#          twin_engine_airframe.txt (heavy/twin fighter = 1.25, cas/nav = 1),
#          quad_engine_airframe.txt (strat bomber chassis = 0),
#          intercontinental_bomber.txt (= 0.01),
#          x_plane_airframes.txt (jets/rockets: fighter set 1, others set 0).
PLANE_AIR_SUP_STAT: Dict[str, float] = {
    # --- full-value air-superiority planes (fighters) ---
    "fighter": 1.0,
    "cv_fighter": 1.0,
    "interceptor": 1.0,
    "heavy_fighter": 1.25,   # twin-engine / heavy fighters carry stat 1.25
    "jet_fighter": 1.0,
    "rocket_interceptor": 1.0,
    # --- planes whose airframe still carries the fighter stat of 1 in vanilla ---
    #     (close-air-support / tactical / naval / carrier strike: measured 1.000)
    "cas": 1.0,
    "cv_cas": 1.0,
    "tac_bomber": 1.0,
    "nav_bomber": 1.0,
    "cv_nav_bomber": 1.0,
    "suicide": 1.0,
    "scout_plane": 1.0,      # recon airframe carries the light-plane stat (1)
    "maritime_patrol_plane": 1.0,
    # --- near-zero air-superiority contributors (strategic / heavy bombers) ---
    "strat_bomber": 0.01,    # intercontinental/strategic bomber stat 0.01
    "jet_strat_bomber": 0.01,
    "rocket": 0.0,           # missiles contribute no air superiority
}
# Fallback for an unseen plane_definition. Conservative: most modded planes that
# carry a real air-superiority value are fighter-class, so 1.0 mirrors the
# dominant vanilla case -- but a resolved equipment stat (when available) wins.
DEFAULT_AIR_SUP_STAT = 1.0

# ===========================================================================
# CITED DEFINE CONSTANTS  (for transparency; not all are used by the formula)
# ===========================================================================
# These describe the surrounding system. The tooltip PERCENT uses none of them
# (it is the bare contest ratio); they are documented so a reader can see what
# was checked and ruled out.
AIR_REGION_SUPERIORITY_PIXEL_SCALE = 0.04   # map texture intensity ONLY
AIR_SUPERIORITY_FACTOR = 2.5                 # AI mission-scoring factor
PARADROP_AIR_SUPERIORITY_RATIO = 0.7         # paradrop gate (friendly/enemy ratio)


# ===========================================================================
# OPTIONAL: resolve EXACT air_superiority stats from the game equipment defs
# ===========================================================================
# When a the game / mod equipment dir is available we parse archetype-level
# `air_superiority = X` lines so non-vanilla balance is honoured. We map the
# oob `plane_definition` (a class label) to the matching archetype stat. This is
# best-effort: if parsing yields nothing we fall back to PLANE_AIR_SUP_STAT.

# archetype-name substring -> plane_definition class (so a parsed archetype stat
# can override the right class entry).
_ARCHETYPE_TO_CLASS: List[Tuple[str, str]] = [
    ("small_plane_airframe", "fighter"),
    ("fighter_equipment", "fighter"),
    ("cv_small_airframe", "cv_fighter"),
    ("cv_fighter", "cv_fighter"),
    ("medium_plane_airframe", "heavy_fighter"),
    ("heavy_fighter", "heavy_fighter"),
    ("cas_equipment", "cas"),
    ("cv_cas", "cv_cas"),
    ("tac_bomber", "tac_bomber"),
    ("medium_plane_scout_airframe", "scout_plane"),
    ("scout_plane", "scout_plane"),
    ("naval_bomber", "nav_bomber"),
    ("nav_bomber", "nav_bomber"),
    ("cv_naval_bomber", "cv_nav_bomber"),
    ("large_plane_airframe", "strat_bomber"),
    ("strat_bomber", "strat_bomber"),
    ("intercontinental", "strat_bomber"),
    ("missile", "rocket"),
]

_AIR_SUP_LINE = re.compile(r"^\s*air_superiority\s*=\s*([0-9]*\.?[0-9]+)", re.M)
_ARCHETYPE_HDR = re.compile(r"^\s*([A-Za-z0-9_]+)\s*=\s*\{", re.M)


def load_air_superiority_stats(equipment_dir: str) -> Dict[str, float]:
    """Best-effort parse of per-class air_superiority stats from a HOI4/mod
    equipment dir (e.g. <install>/common/units/equipment). Returns
    {plane_definition_class: air_superiority_stat}. Honours mod overrides if the
    given dir is the mod's. Skips commented (`#`) lines. Returns {} if the dir is
    absent or carries no usable stats -> caller falls back to PLANE_AIR_SUP_STAT.

    NOTE: equipment_dir is taken from the CALLER (CLI flag / config). This module
    NEVER hardcodes an absolute machine path.
    """
    out: Dict[str, float] = {}
    if not equipment_dir or not os.path.isdir(equipment_dir):
        return out
    for fname in sorted(os.listdir(equipment_dir)):
        if not fname.endswith(".txt"):
            continue
        path = os.path.join(equipment_dir, fname)
        try:
            with open(path, "r", encoding="latin-1", errors="replace") as fh:
                text = fh.read()
        except OSError:
            continue
        # Drop fully-commented lines so a disabled archetype's stat is ignored.
        live = "\n".join(
            ln for ln in text.splitlines() if not ln.lstrip().startswith("#")
        )
        # Walk archetype headers; for each, grab the FIRST air_superiority within.
        headers = list(_ARCHETYPE_HDR.finditer(live))
        for i, hdr in enumerate(headers):
            name = hdr.group(1).lower()
            seg_end = headers[i + 1].start() if i + 1 < len(headers) else len(live)
            seg = live[hdr.end():seg_end]
            mm = _AIR_SUP_LINE.search(seg)
            if not mm:
                continue
            stat = float(mm.group(1))
            for substr, cls in _ARCHETYPE_TO_CLASS:
                if substr in name:
                    # keep the lowest-version / first stat seen per class
                    out.setdefault(cls, stat)
                    break
    return out


def plane_air_sup_stat(
    plane_definition: Optional[str],
    overrides: Optional[Dict[str, float]] = None,
) -> float:
    """Resolve the per-plane air_superiority weight for a plane_definition class.

    Priority: caller-provided defs override -> calibrated vanilla table ->
    DEFAULT_AIR_SUP_STAT. (Codex caveat: do not blindly hardcode "others=1.0";
    a resolved equipment stat wins when present.)
    """
    if plane_definition is None:
        return DEFAULT_AIR_SUP_STAT
    if overrides and plane_definition in overrides:
        return overrides[plane_definition]
    return PLANE_AIR_SUP_STAT.get(plane_definition, DEFAULT_AIR_SUP_STAT)


# ===========================================================================
# DATA LOADING
# ===========================================================================

def _load_json(output_dir: str, name: str) -> Optional[Any]:
    path = os.path.join(output_dir, name + ".json")
    if not os.path.exists(path):
        return None
    with open(path, "r", encoding="utf-8") as fh:
        return json.load(fh)


def load_inputs(output_dir: str) -> Dict[str, Any]:
    """Load the JSON inputs. `oob.json` is required (carries air_wings); the
    rest are optional context. Raises FileNotFoundError if oob is absent."""
    oob = _load_json(output_dir, "oob")
    if oob is None:
        raise FileNotFoundError(
            f"required input 'oob.json' not found in {output_dir!r}"
        )
    return {
        "oob": oob,
        "diplomacy": _load_json(output_dir, "diplomacy"),
        "players": _load_json(output_dir, "players"),
        "meta": _load_json(output_dir, "meta") or oob.get("meta"),
    }


# ===========================================================================
# CORE: per-wing score + per-region aggregation
# ===========================================================================

def wing_air_sup_score(
    wing: Dict[str, Any],
    overrides: Optional[Dict[str, float]] = None,
) -> Optional[float]:
    """Per-wing air-superiority score = effective_planes * air_sup_stat(plane).

    Returns None if the wing is NOT in a region's superiority pool (idle: no
    strategic_region / no effective_planes). Idle wings are excluded -- the
    mission-region pointer being null is the engine's own gate (Codex-confirmed).
    Falls back to `count` only when effective_planes is missing AND the wing is
    region-assigned (defensive; the save normally fills effective_planes).
    """
    if wing.get("strategic_region") is None:
        return None
    effp = wing.get("effective_planes")
    if effp is None:
        effp = wing.get("count")
    if effp is None:
        return None
    stat = plane_air_sup_stat(wing.get("plane_definition"), overrides)
    return float(effp) * stat


@dataclass
class RegionAirSup:
    """Per-strategic-region air-superiority breakdown (MODEL estimate)."""
    region_id: int
    by_owner: Dict[str, Dict[str, float]] = field(default_factory=dict)
    # by_owner[tag] = {"score": float, "effective_planes": float, "wings": int}
    total_score: float = 0.0
    leader: Optional[str] = None
    source: str = "MODEL"

    def percent_for(self, friendly_tags) -> Optional[float]:
        """Displayed air-superiority percent for a viewing SIDE.

        friendly_tags: iterable of owner tags counted as the viewer's side.
        Returns 100 * own/(own+enemy), clamped [0,100]; None when the region has
        zero total score (no superiority to display -- avoids div-by-zero).
        """
        friendly = set(friendly_tags)
        own = sum(
            v["score"] for t, v in self.by_owner.items() if t in friendly
        )
        enemy = sum(
            v["score"] for t, v in self.by_owner.items() if t not in friendly
        )
        denom = own + enemy
        if denom <= 0.0:
            return None
        pct = 100.0 * own / denom
        if pct < 0.0:
            return 0.0
        if pct > 100.0:
            return 100.0
        return pct

    def as_dict(self) -> Dict[str, Any]:
        return asdict(self)


def aggregate_regions(
    air_wings: List[Dict[str, Any]],
    overrides: Optional[Dict[str, float]] = None,
) -> Dict[int, RegionAirSup]:
    """Bucket region-mission wings by (strategic_region, owner) and sum the
    stat-weighted score (and effective_planes, for cross-check vs the proxy)."""
    regions: Dict[int, RegionAirSup] = {}
    for w in air_wings:
        score = wing_air_sup_score(w, overrides)
        if score is None:
            continue
        rid = int(w["strategic_region"])
        owner = w.get("owner") or "?"
        effp = float(w.get("effective_planes") or w.get("count") or 0)
        r = regions.get(rid)
        if r is None:
            r = regions[rid] = RegionAirSup(region_id=rid)
        b = r.by_owner.setdefault(
            owner, {"score": 0.0, "effective_planes": 0.0, "wings": 0}
        )
        b["score"] += score
        b["effective_planes"] += effp
        b["wings"] += 1
        r.total_score += score
    for r in regions.values():
        if r.by_owner:
            r.leader = max(r.by_owner, key=lambda t: r.by_owner[t]["score"])
    return regions


# ===========================================================================
# SIDE RESOLUTION  (viewer faction / war side)
# ===========================================================================
# The displayed percent is viewer-relative. We do not always know a "faction"
# cleanly, so we expose a flexible resolver: explicit friendly tag set, OR a
# best-effort grouping from diplomacy.json factions when present. (Codex caveat:
# the in-game grouping is by the viewer's war/diplomacy side, which can differ
# from formal faction membership in edge cases -- callers pass an explicit set
# when they need exactness.)

def faction_members(diplomacy_json: Optional[Dict[str, Any]]) -> Dict[str, List[str]]:
    """Extract {faction_name: [member_tags]} from diplomacy.json if it carries
    factions. Returns {} when unavailable (caller then supplies tags directly)."""
    out: Dict[str, List[str]] = {}
    if not isinstance(diplomacy_json, dict):
        return out
    fac = diplomacy_json.get("factions")
    if isinstance(fac, dict):
        for name, info in fac.items():
            members = None
            if isinstance(info, dict):
                members = info.get("members") or info.get("countries")
            elif isinstance(info, list):
                members = info
            if isinstance(members, list):
                out[name] = [str(m) for m in members]
    elif isinstance(fac, list):
        for info in fac:
            if isinstance(info, dict):
                name = info.get("name") or info.get("faction") or info.get("id")
                members = info.get("members") or info.get("countries")
                if name and isinstance(members, list):
                    out[str(name)] = [str(m) for m in members]
    return out


# ===========================================================================
# TOP-LEVEL COMPUTE
# ===========================================================================

@dataclass
class AirSupResult:
    meta: Dict[str, Any]
    regions: Dict[int, RegionAirSup]
    stats_overrides: Dict[str, float]
    source: str = "MODEL"

    def as_dict(self) -> Dict[str, Any]:
        return {
            "meta": self.meta,
            "source": self.source,
            "stat_overrides_used": self.stats_overrides,
            "regions": {
                str(rid): r.as_dict() for rid, r in sorted(self.regions.items())
            },
        }

    def region(self, region_id: int) -> Optional[RegionAirSup]:
        return self.regions.get(int(region_id))


def compute_air_superiority(
    output_dir: str,
    equipment_dir: Optional[str] = None,
) -> AirSupResult:
    """Compute per-region air-superiority scores from a save_oracle output dir.

    output_dir   : dir with oob.json (and optional diplomacy/players/meta).
    equipment_dir: optional HOI4/mod `common/units/equipment` dir to resolve
                   EXACT per-plane air_superiority stats (honours mod balance).
                   Pass None to use the calibrated vanilla table.
    """
    data = load_inputs(output_dir)
    overrides = (
        load_air_superiority_stats(equipment_dir) if equipment_dir else {}
    )
    air_wings = data["oob"].get("air_wings") or []
    regions = aggregate_regions(air_wings, overrides)
    meta = dict(data.get("meta") or {})
    meta.update(
        {
            "n_regions": len(regions),
            "n_wings_total": len(air_wings),
            "n_wings_scored": sum(
                b["wings"] for r in regions.values() for b in r.by_owner.values()
            ),
            "model": "air_superiority",
            "formula": "pct(side)=100*own/(own+enemy); EAS=effective_planes*air_sup_stat(plane)",
        }
    )
    return AirSupResult(meta=meta, regions=regions, stats_overrides=overrides)


# ===========================================================================
# PRESENTATION
# ===========================================================================

def _format(result: AirSupResult, n: int = 30) -> str:
    lines: List[str] = []
    m = result.meta
    lines.append(
        f"air_superiority  (MODEL)  regions={m.get('n_regions')} "
        f"wings_scored={m.get('n_wings_scored')}/{m.get('n_wings_total')}"
    )
    lines.append(f"  formula: {m.get('formula')}")
    lines.append("-" * 70)
    rows = sorted(
        result.regions.values(), key=lambda r: r.total_score, reverse=True
    )
    for r in rows[:n]:
        owners = sorted(
            r.by_owner.items(), key=lambda kv: kv[1]["score"], reverse=True
        )
        parts = " ".join(
            f"{t}:{v['score']:.0f}({v['wings']}w)" for t, v in owners
        )
        lead_pct = r.percent_for([r.leader]) if r.leader else None
        lp = f"{lead_pct:5.1f}%" if lead_pct is not None else "  n/a"
        lines.append(
            f"region {r.region_id:>4}  leader {r.leader or '-':<4} {lp}  | {parts}"
        )
    return "\n".join(lines)


def main(argv: Optional[List[str]] = None) -> int:  # pragma: no cover
    import argparse

    ap = argparse.ArgumentParser(
        description="MODEL: displayed air-superiority percent per strategic region."
    )
    ap.add_argument(
        "output_dir",
        nargs="?",
        default=os.path.join("extractor", "save_oracle", "output"),
        help="save_oracle output dir with oob.json (default: extractor/save_oracle/output)",
    )
    ap.add_argument(
        "--equipment-dir",
        default=None,
        help="optional HOI4/mod common/units/equipment dir for exact plane stats",
    )
    ap.add_argument("--top", type=int, default=30, help="rows to print")
    ap.add_argument("--json", action="store_true", help="dump full result as JSON")
    args = ap.parse_args(argv)

    result = compute_air_superiority(args.output_dir, args.equipment_dir)
    if args.json:
        print(json.dumps(result.as_dict(), indent=1))
    else:
        print(_format(result, n=args.top))
    return 0


# ===========================================================================
# CITATIONS  (where each constant / fact comes from)
# ===========================================================================
CITATIONS: Dict[str, str] = {
    "air_superiority_stat": (
        "common/units/equipment/{single,twin,quad}_engine_airframe.txt, "
        "intercontinental_bomber.txt, x_plane_airframes.txt -- air_superiority = "
        "1 (fighter), 1.25 (twin/heavy fighter), 0.01 (strat/intercont. bomber), "
        "0 (quad bomber chassis)."
    ),
    "eas_is_planes_times_stat": (
        "Melted 1946 save: per-wing mission.effective_air_superiority / "
        "effective_planes_count == the airframe air_superiority stat, exactly "
        "(fighter 1.000, heavy_fighter 1.250, strat_bomber 0.010 over 322 wings)."
    ),
    "contest_percent": (
        "Display is own/(own+enemy)*100 (e.g. air_l_english.yml "
        "AIR_CAN_NOT_PERFORM_PORT_STRIKE '$CURRENT|H2%$'); Codex-confirmed no "
        "fixed-cap / logistic / pixel-scale normalization for the tooltip percent."
    ),
    "pixel_scale": (
        "00_defines.lua AIR_REGION_SUPERIORITY_PIXEL_SCALE = 0.04 -- map TEXTURE "
        "intensity (superiority/(pixels*scale)) only; not the tooltip percent."
    ),
    "no_agility_term": (
        "Save EAS = effective_planes * stat with no agility/air_attack/air_defence "
        "term; those drive air-COMBAT loss resolution, not the displayed percent."
    ),
    "idle_excluded": (
        "Only wings with a region-bearing mission carry effective_air_superiority "
        "in the save; idle wings (null mission region) are excluded from the pool."
    ),
}


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
