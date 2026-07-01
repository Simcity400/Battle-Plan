"""Derived EQUIPMENT-VARIANT stat model for the Battle Plan tool (READ-ONLY).

The save stores a designed equipment VARIANT as its *inputs* -- the parent
archetype/frame, the module filling each slot, and the integer level of each
upgrade slider -- but it does NOT store the variant's DERIVED stat grid (the
final max speed / armor / attack / reliability / build cost the equipment
designer shows). The engine recomputes that grid at runtime from the raw script
defs in ``common/units/equipment/**``. This module reproduces that computation.

It covers all three designer families plus the slider-only upgrade variants:

  * SHIPS  (ship_hull_*) -- max speed, HP (max_strength), armor, attack by
    battery type (lg/hg/secondaries), AA, torpedo, sub detection/visibility,
    range, reliability, build cost.
  * TANKS  (*_tank_chassis_*, armored cars, mech) -- speed, armor, piercing,
    soft/hard attack, breakthrough, defense, reliability, fuel.
  * PLANES (*_plane_airframe_*) -- speed, agility, air attack, air defence,
    range, bombing, naval-strike, reliability.
  * UPGRADE-only variants (no modules) -- a non-modular archetype whose only
    customisation is upgrade sliders (e.g. mechanized_equipment_1, support
    equipment); handled by the same upgrade math with an empty module set.

Inputs (per variant, from ``save_oracle/output*/designs.json``):
  modules : {slot_name: module_name}     # the chosen module in each slot
  upgrades: {upgrade_name: int level}    # the slider level for each upgrade
  archetype                              # the frame/chassis/hull this is built on

Game defs are read from the vanilla install and overlaid by the mod (mod wins by
filename), via the shared ``models.division_stats.GameDefs`` loader. Every
formula carries a ``# source:`` citation to the script def it was derived from.

AGGREGATION (the game designer's order of operations -- see AGGREGATION NOTES at
the bottom, and the Codex refutation log in re_scratch/WIRE_equipment_stats.md):

    flat[S]  = base[S]                                   # archetype, parent chain
             + sum over modules of  module.add_stats[S]  # flat add
             + sum over upgrades of level * upgrade.add_stats[S]   # flat add
             + average_contribution[S]                   # add_average_stats
             + sum over upgrades of level * upgrade.<bare flat stat S>
                                                         # reliability only

    mult[S]  = sum over modules of  module.multiply_stats[S]   # fraction
             + sum over upgrades of level * upgrade.<bare percent stat S>
                                                         # all bare stats EXCEPT reliability

    FINAL[S] = (flat[S]) * (1 + mult[S])

  * Multipliers are SUMMED then applied once: ``(1 + a + b)`` -- not compounded.
  * ``multiply_stats`` apply to ``base + adds`` (the whole flat running total).
  * ``add_average_stats`` (ships; e.g. hg_armor_piercing) are AVERAGED across the
    number of modules that PROVIDE that key -- two batteries each contributing
    36 yield 36, not 72.
  * ``reliability`` upgrade entries are FLAT per-level deltas, never percent
    multipliers (Codex correction F). The other bare upgrade stats are percent.

This module creates NO new schema/store/diff coupling -- it is a pure function
over designs.json + game defs. The wiring contract lives in
``re_scratch/WIRE_equipment_stats.md``.
"""
from __future__ import annotations

import json
import os
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable, Optional

_REPO_ROOT = Path(__file__).resolve().parents[1]
if str(_REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(_REPO_ROOT))

# Reuse the repo's tolerant Clausewitz parser + the existing GameDefs loader
# (file resolution, mod-over-vanilla, archetype/parent inheritance, and module
# add/mult/avg extraction). We extend it only with the UPGRADE defs, which
# division_stats does not need.
from game_assets.paradox_parser import Node, parse, coerce_scalar  # noqa: E402
from models.division_stats import GameDefs, _stat_dict  # noqa: E402

# ---------------------------------------------------------------------------
# Keys that appear inside an upgrade block but are NOT stat deltas.
# source: common/units/equipment/upgrades/*.txt -- every upgrade carries these
# bookkeeping keys; only the remaining bare scalars + the add_stats block are
# stat contributions.
# ---------------------------------------------------------------------------
_UPGRADE_NON_STAT_KEYS = frozenset({
    "max_level", "cost", "abbreviation", "linear_cost", "level_requirements",
    "resource_cost_thresholds", "resource_cost", "icon", "ai_will_do",
    "show_equipment_icon", "sfx",
})

# Stats whose bare upgrade entries are FLAT per-level deltas, not percent.
# source: Codex refutation (re_scratch/WIRE_equipment_stats.md, answers F + the
# carrier_size flag). reliability = -0.005 / 0.1 are point deltas on a 0..1
# fraction; carrier_size = 1 (ship_deckspace_upgrade, naval_upgrades.txt) is an
# additive deck-slot count, not a +100%% multiplier. Both are flat per level.
_FLAT_UPGRADE_STATS = frozenset({"reliability", "carrier_size"})


# ===========================================================================
# UPGRADE DEFS  (the slider definitions -- not parsed by division_stats)
# ===========================================================================
@dataclass
class Upgrade:
    """A single upgrade slider definition.

    flat  : {stat: per-level flat delta}      # from the upgrade's add_stats {} block
                                              # + bare flat stats (reliability)
    pct   : {stat: per-level fraction}        # bare percent stats (e.g. hg_attack=0.04)
    max_level : int
    """
    name: str
    flat: dict[str, float] = field(default_factory=dict)
    pct: dict[str, float] = field(default_factory=dict)
    max_level: int = 0

    @classmethod
    def from_node(cls, name: str, node: Node) -> "Upgrade":
        flat: dict[str, float] = {}
        pct: dict[str, float] = {}
        max_level = 0
        for k, v in node.entries:
            kl = k.lower()
            if kl == "max_level":
                c = coerce_scalar(v)
                if isinstance(c, (int, float)):
                    max_level = int(c)
                continue
            if kl == "add_stats":
                # source: upgrades/land_upgrades.txt tank_nsb_*_upgrade --
                # add_stats inside an upgrade is a FLAT per-level delta.
                if isinstance(v, Node):
                    for sk, sv in _stat_dict(v).items():
                        flat[sk] = flat.get(sk, 0.0) + sv
                continue
            if kl in _UPGRADE_NON_STAT_KEYS:
                continue
            if isinstance(v, Node):
                # any other nested block (multiply/limit/etc.) is not a per-level
                # bare stat; ignore (none observed in vanilla/mod upgrade defs).
                continue
            c = coerce_scalar(v)
            if not isinstance(c, (int, float)):
                continue
            # source: upgrades/*.txt -- a bare numeric stat under an upgrade is a
            # per-level PERCENT factor (e.g. ship_gun_upgrade hg_attack=0.04),
            # EXCEPT reliability which is a flat per-level point delta.
            if kl in _FLAT_UPGRADE_STATS:
                flat[kl] = flat.get(kl, 0.0) + float(c)
            else:
                pct[kl] = pct.get(kl, 0.0) + float(c)
        return cls(name=name, flat=flat, pct=pct, max_level=max_level)


class EquipmentDefs:
    """GameDefs (equipments + modules) + the upgrade slider defs.

    Thin wrapper that reuses models.division_stats.GameDefs for the heavy
    lifting (mod-over-vanilla file resolution, archetype/parent inheritance,
    module add/mult/avg parsing) and adds the ``upgrades/`` defs.
    """

    def __init__(self, vanilla_dir: Optional[str | Path] = None,
                 mod_dir: Optional[str | Path] = None):
        # Empty default keeps machine-specific paths out of source; set the
        # BATTLE_PLAN_VANILLA env var (or pass vanilla_dir) to point at your install.
        van = Path(vanilla_dir) if vanilla_dir is not None else Path(
            os.environ.get("BATTLE_PLAN_VANILLA", os.environ.get("HOI4_VANILLA", ""))
        )
        if mod_dir is not None:
            mod = Path(mod_dir)
        else:
            env_mod = os.environ.get("BATTLE_PLAN_MOD", os.environ.get("HOI4_MOD"))
            mod = Path(env_mod) if env_mod else (_REPO_ROOT.parent.parent / "test")
        self.defs = GameDefs(vanilla_dir=van, mod_dir=mod)
        self.upgrades: dict[str, Upgrade] = {}
        self._load_upgrades(van, mod)

    # --- upgrade file resolution (mod wins by filename, mirrors GameDefs) ----
    def _upgrade_files(self, van: Path, mod: Optional[Path]) -> list[Path]:
        files: dict[str, Path] = {}
        vdir = van / "common" / "units" / "equipment" / "upgrades"
        if vdir.is_dir():
            for p in sorted(vdir.glob("*.txt")):
                files[p.name] = p
        if mod:
            mdir = mod / "common" / "units" / "equipment" / "upgrades"
            if mdir.is_dir():
                for p in sorted(mdir.glob("*.txt")):
                    files[p.name] = p  # mod wins
        return list(files.values())

    def _load_upgrades(self, van: Path, mod: Optional[Path]) -> None:
        for p in self._upgrade_files(van, mod):
            raw = p.read_bytes().decode("cp1252", errors="replace")
            root = parse(raw)
            # source: upgrades/*.txt -- top-level `upgrades = { <name> = {..} }`
            for _, blk in root.children_named("upgrades"):
                if not isinstance(blk, Node):
                    continue
                for name, body in blk.entries:
                    if isinstance(body, Node) and name:
                        self.upgrades[name] = Upgrade.from_node(name, body)

    # --- convenience pass-throughs -----------------------------------------
    @property
    def equipments(self):
        return self.defs.equipments

    @property
    def modules(self):
        return self.defs.modules

    def base_stats(self, archetype: str) -> dict[str, float]:
        """Resolved archetype/frame base stats (parent chain, child wins)."""
        # source: equipment/_documentation.md -- `parent`+`archetype` chain; a
        # variant lists only the stats it changes, inheriting the rest.
        return self.defs._inherited_stats(archetype)


# ===========================================================================
# THE STAT COMPUTATION
# ===========================================================================
@dataclass
class VariantStats:
    """Computed final stat grid for one equipment variant."""
    archetype: str
    family: str
    name: Optional[str]
    stats: dict[str, float]                 # final derived stat grid
    base: dict[str, float]                  # archetype/frame base (pre module/upgrade)
    modules: dict[str, str]                 # slot -> module
    upgrades: dict[str, int]                # upgrade -> level
    unknown_modules: list[str] = field(default_factory=list)
    unknown_upgrades: list[str] = field(default_factory=list)

    def get(self, stat: str, default=None):
        return self.stats.get(stat, default)


def compute_variant_stats(variant: dict[str, Any],
                          defs: EquipmentDefs) -> VariantStats:
    """Compute the final derived stat grid for one designs.json variant.

    `variant` is a designs.json equipment_variant (or a scheduled_variant) dict
    with at least: archetype, family, modules{slot:module}, upgrades{name:level}.

    Returns a VariantStats with the full final grid. Stats never present in any
    contributing def are simply absent (no fabricated zeros) -- consistent with
    the repo's no-fabrication rule.
    """
    archetype = variant.get("archetype") or ""
    family = variant.get("family") or ""
    name = variant.get("name")
    modules: dict[str, str] = dict(variant.get("modules") or {})
    upgrades_in = variant.get("upgrades") or {}

    # --- 1. base: archetype / frame, with parent + archetype inheritance ----
    base = defs.base_stats(archetype)
    flat: dict[str, float] = dict(base)          # running flat bucket
    mult: dict[str, float] = {}                  # running multiplicative fraction
    unknown_modules: list[str] = []
    unknown_upgrades: list[str] = []

    # --- 2. modules: flat adds, multiply fractions, averaged adds -----------
    # source: modules/*.txt -- each module: add_stats (flat), multiply_stats
    # (fraction summed into the mult bucket), add_average_stats (averaged).
    avg_sum: dict[str, float] = {}
    avg_count: dict[str, int] = {}
    for slot, module_name in modules.items():
        mod = defs.modules.get(module_name)
        if mod is None:
            # `empty` slots and unresolved modules contribute nothing.
            if module_name and module_name != "empty":
                unknown_modules.append(module_name)
            continue
        for k, v in mod["add"].items():
            flat[k] = flat.get(k, 0.0) + v
        for k, v in mod["mult"].items():
            mult[k] = mult.get(k, 0.0) + v
        if mod["avg"]:
            for k, v in mod["avg"].items():
                avg_sum[k] = avg_sum.get(k, 0.0) + v
                avg_count[k] = avg_count.get(k, 0) + 1
    # add_average_stats: divide by the number of modules PROVIDING the key.
    # source: the game naval AP-averaging rule (Codex answer E).
    for k, total in avg_sum.items():
        n = avg_count.get(k, 0)
        if n:
            flat[k] = flat.get(k, 0.0) + total / n

    # --- 3. upgrades: flat (add_stats + reliability) and percent buckets ----
    # source: upgrades/*.txt -- level * per-level delta; flat into `flat`,
    # percent into `mult` (same multiplicative bucket as module multiply_stats).
    for up_name, level in upgrades_in.items():
        try:
            lvl = int(level)
        except (TypeError, ValueError):
            lvl = 0
        if lvl == 0:
            continue
        up = defs.upgrades.get(up_name)
        if up is None:
            unknown_upgrades.append(up_name)
            continue
        if up.max_level:
            lvl = min(lvl, up.max_level)
        for k, v in up.flat.items():
            flat[k] = flat.get(k, 0.0) + v * lvl
        for k, v in up.pct.items():
            mult[k] = mult.get(k, 0.0) + v * lvl

    # --- 4. final: (base + adds) * (1 + sum of mults) -----------------------
    # source: Codex order-of-operations (answers A/B/C); mults summed, applied
    # once to the flat running total.
    final: dict[str, float] = {}
    keys: set[str] = set(flat) | set(mult)
    for k in keys:
        f = flat.get(k, 0.0)
        m = mult.get(k, 0.0)
        final[k] = f * (1.0 + m)

    return VariantStats(
        archetype=archetype,
        family=family,
        name=name,
        stats=final,
        base=base,
        modules=modules,
        upgrades={k: int(v) if isinstance(v, (int, float)) else 0
                  for k, v in upgrades_in.items()},
        unknown_modules=unknown_modules,
        unknown_upgrades=unknown_upgrades,
    )


# ===========================================================================
# Bulk helpers over a whole designs.json
# ===========================================================================
def iter_designs_variants(designs: dict[str, Any]) -> Iterable[tuple[str, dict]]:
    """Yield (key, variant) for every top-level equipment_variant."""
    ev = designs.get("equipment_variants") or {}
    for k, v in ev.items():
        if isinstance(v, dict):
            yield k, v


def compute_all(designs: dict[str, Any], defs: EquipmentDefs,
                families: Optional[Iterable[str]] = None) -> dict[str, VariantStats]:
    """Compute final stats for every (or selected-family) variant in designs.json.

    Variants with no modules and no upgrades (e.g. plain stockpile archetypes
    with family ``land_other``) still resolve to their inherited base grid.
    """
    fam_filter = set(families) if families else None
    out: dict[str, VariantStats] = {}
    for key, variant in iter_designs_variants(designs):
        if fam_filter and (variant.get("family") not in fam_filter):
            continue
        out[key] = compute_variant_stats(variant, defs)
    return out


def load_designs(path: str | Path) -> dict[str, Any]:
    with open(path, "r", encoding="utf-8") as fh:
        return json.load(fh)


# ===========================================================================
# CLI: spot-check a handful of variants
# ===========================================================================
def _fmt(stats: dict[str, float], keys: Iterable[str]) -> str:
    return ", ".join(
        f"{k}={stats[k]:.3g}" for k in keys if k in stats
    )


_FAMILY_REPORT_KEYS = {
    "ship": ("naval_speed", "max_strength", "armor_value", "hg_attack",
             "hg_armor_piercing", "lg_attack", "anti_air_attack",
             "torpedo_attack", "sub_attack", "sub_detection", "sub_visibility",
             "naval_range", "reliability", "build_cost_ic"),
    "land_vehicle": ("maximum_speed", "armor_value", "ap_attack", "soft_attack",
                     "hard_attack", "breakthrough", "defense", "reliability",
                     "fuel_consumption", "build_cost_ic"),
    "plane": ("maximum_speed", "air_agility", "air_attack", "air_defence",
              "air_range", "air_bombing", "naval_strike_attack", "reliability",
              "build_cost_ic"),
    "land_other": ("maximum_speed", "soft_attack", "hard_attack", "armor_value",
                   "reliability", "build_cost_ic"),
}


def _cli(argv: list[str]) -> int:
    designs_path = argv[1] if len(argv) > 1 else str(
        _REPO_ROOT.parent / "extractor" / "save_oracle" / "output" / "designs.json")
    designs = load_designs(designs_path)
    defs = EquipmentDefs()
    print(f"# designs: {designs_path}")
    print(f"# upgrades loaded: {len(defs.upgrades)}  modules: {len(defs.modules)}"
          f"  equipments: {len(defs.equipments)}")
    # pick the richest named variant per family
    best: dict[str, tuple[int, str, dict]] = {}
    for key, v in iter_designs_variants(designs):
        fam = v.get("family") or ""
        score = len(v.get("modules") or {}) + sum(
            1 for x in (v.get("upgrades") or {}).values() if x)
        if v.get("name") and (fam not in best or score > best[fam][0]):
            best[fam] = (score, key, v)
    for fam, (_score, key, v) in sorted(best.items()):
        vs = compute_variant_stats(v, defs)
        rep_keys = _FAMILY_REPORT_KEYS.get(fam, sorted(vs.stats))
        print(f"\n## {fam}  {key}  {v.get('name')!r}  arch={vs.archetype}")
        print(f"   modules={len(vs.modules)} upgrades={vs.upgrades}")
        print(f"   FINAL: {_fmt(vs.stats, rep_keys)}")
        print(f"   base : {_fmt(vs.base, rep_keys)}")
        if vs.unknown_modules:
            print(f"   UNKNOWN MODULES: {vs.unknown_modules}")
        if vs.unknown_upgrades:
            print(f"   UNKNOWN UPGRADES: {vs.unknown_upgrades}")
    return 0


if __name__ == "__main__":
    raise SystemExit(_cli(sys.argv))


# ===========================================================================
# AGGREGATION NOTES (rationale + citations)
# ===========================================================================
# 1. BASE / ARCHETYPE CHAIN
#    A buildable variant (e.g. medium_tank_chassis_1) carries `archetype` and
#    optionally `parent`; it lists only the stats it changes. Resolution walks
#    archetype first then the parent chain, child keys overriding.
#    # source: equipment/_documentation.md ("Base values on various unit stats
#    # before upgrades and modules ..."); tank_chassis.txt light_tank_chassis_1
#    # overrides maximum_speed/armor_value while inheriting hardness/defense.
#    Reused verbatim from models.division_stats.GameDefs._inherited_stats.
#
# 2. MODULES  (modules/00_{tank,ship,plane}_modules.txt)
#    add_stats{}          -> flat additive to the running total.
#    multiply_stats{}     -> fraction summed into the mult bucket; applied ONCE
#                            as (1 + sum) to (base + all adds).
#    add_average_stats{}  -> contributions AVERAGED across the modules providing
#                            the key (ship armour-piercing). divisor = count of
#                            providing modules, not total slots.
#    # source: 00_ship_modules.txt ship_heavy_battery_2 has add_stats{hg_attack=18}
#    # + add_average_stats{hg_armor_piercing=36} + multiply_stats{naval_speed=-0.06}.
#    NOTE: module `parent=` chains (e.g. heavy_ship_engine_3 parent=heavy_ship_
#    engine_2) carry their OWN full add/mult/avg blocks in the def -- they do NOT
#    inherit a parent module's stats (verified: heavy_ship_engine_3 restates its
#    multiply_stats). So summing the module's own block is correct.
#    NOTE: mission_type_stats{} (planes) are conditional on the wing's MISSION and
#    are intentionally EXCLUDED from the base designer grid; the unconditional
#    add_stats are included. The designer shows mission-independent stats here.
#
# 3. UPGRADES  (upgrades/{land,naval,air}_upgrades.txt)
#    Two shapes coexist:
#      (a) bare numeric stats under the upgrade (ship_gun_upgrade hg_attack=0.04)
#          -> per-level PERCENT factor: contributes level*0.04 to the mult bucket.
#      (b) an add_stats{} block (tank_nsb_engine_upgrade) -> per-level FLAT add:
#          contributes level*delta to the flat bucket.
#    reliability is the ONE bare stat that is FLAT not percent (Codex answer F);
#    e.g. ship_gun_upgrade reliability=-0.005 is -0.005*level points.
#    level is clamped to the upgrade's max_level.
#    # source: naval_upgrades.txt ship_gun_upgrade (percent) vs land_upgrades.txt
#    # tank_nsb_engine_upgrade add_stats (flat).
#
# 4. FINAL  = (base + sum_adds + avg) * (1 + sum_mults)
#    Multipliers SUMMED (1+a+b), not compounded; applied to base+adds, not base
#    alone. Confirmed by Codex (answers A/B/C). See re_scratch/WIRE_equipment_stats.md
#    for the full Codex transcript + the worked Panzer III / heavy-ship examples.
#
# WHAT THIS DOES NOT MODEL (out of scope; the designer also folds these in, but
# they are country/tech-state dependent and not part of the variant def):
#    * research/tech stat bonuses, MIO design_team_bonus (present in designs.json
#      as `design_team_bonus` -- a separate %% layer that could be applied on top),
#    * doctrine / national-spirit modifiers,
#    * the thrust/weight -> speed & agility refinement for planes (the airframe's
#      own maximum_speed/air_agility base already encodes the intended value; the
#      engine multiply_stats{maximum_speed=..} captures the headline change).
#    These are deliberately layered ON TOP elsewhere, keeping this module a pure
#    function of (archetype, modules, upgrades).
