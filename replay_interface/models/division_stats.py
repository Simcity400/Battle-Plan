"""Derived division combat-stat model for the Battle Plan tool (READ-ONLY).

The save stores a division's *grid layout* (which sub_units / support companies
sit in the template) and, for deployed divisions, the equipment it actually
holds -- but it does NOT store the derived combat stats the game shows in the
division designer (soft/hard attack, defense, breakthrough, org, HP, width,
armor, piercing, hardness, speed, supply use, ...). Those are recomputed at
runtime by the engine from the regiment + support composition. This module
reproduces that computation from the raw script defs in ``common/units/**``.

Inputs
------
- A division TEMPLATE: list of regiments (line battalions) + support companies,
  each a sub_unit key (``infantry``, ``medium_armor``, ``engineer``, ...).
- The equipment VARIANT chosen for each ``need`` archetype (from the save's
  ``equipment_variants`` catalog or a deployed division's ``equipment`` list).

Game defs are read from the vanilla install and overlaid by the mod (mod wins by
filename). Every formula carries a ``# source: <file>`` citation to the script
def it was derived from.

Aggregation summary (Codex-verified against the game wiki; see AGGREGATION NOTES
at the bottom for the full rationale + sources):

  battalion stat (from equipment) = the equipment stat itself (NO need-scaling).
      # The `need = { archetype = N }` amount is the equipment QUANTITY (stockpile
      # / IC cost / reinforcement), NOT a stat multiplier. Line artillery with
      # need=24 and artillery soft_attack=25 contributes 25, not 6.
  sub_unit inline stat (e.g. support `soft_attack = -0.4`, mech `hard_attack=4`)
      = a MULTIPLIER on the equipment-derived stat: final = derived * (1 + inline)
      # support "nerfs" of -0.4 mean 60% of the equipment stat; mech `hard_attack
      #  = 4.0` means +400% of mech_equipment hard attack.
  division ATTACK/DEFENSE/BREAKTHROUGH = SUM over all battalions (incl. support).
  division MAX ORG = simple arithmetic mean of max_organisation over ALL placed
      sub-units INCLUDING support companies (engineer org=20 and line-arty org=0
      both count); NOT manpower-weighted, NOT filtered to org>0.
  division MAX HP (strength) = SUM of every battalion's max_strength (incl support).
  division COMBAT WIDTH = SUM of combat_width over all sub-units (support = 0).
  division HARDNESS = simple average of per-battalion hardness.
  division ARMOR    = 0.4*max_armor   + 0.6*avg_armor   (ARMOR_VS_AVERAGE = 0.4).
  division PIERCING = 0.4*max_piercing + 0.6*avg_piercing (PEN_VS_AVERAGE = 0.4).
  division SPEED = the slowest sub-unit speed (affects_speed=no excluded);
      sub_unit maximum_speed is a MULTIPLIER on the equipment speed.

NOT modeled (would need country state, not just the template): tech/doctrine/
idea/designer/MIO modifiers, terrain modifiers, and battalion_mult is only a
coarse approximation. This yields the BASE designer stats for a template, the
same way the designer screen shows them before country bonuses.
"""
from __future__ import annotations

import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Optional

# Reuse the repo's tolerant Clausewitz parser (game_assets/paradox_parser.py).
_REPO_ROOT = Path(__file__).resolve().parents[1]
if str(_REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(_REPO_ROOT))
from game_assets.paradox_parser import Node, parse, coerce_scalar  # noqa: E402

# ---------------------------------------------------------------------------
# Game directories. Repo-relative config -- NO absolute user paths in source.
# Override via env or the GameDefs(...) constructor for a different install.
# ---------------------------------------------------------------------------
import os

#: Default vanilla install. Set the BATTLE_PLAN_VANILLA env var to your install dir
#: (or pass vanilla_dir to GameDefs(...)); empty by default so no machine-specific
#: path lives in source -- the loader degrades gracefully when it's unset.
_DEFAULT_VANILLA = os.environ.get("BATTLE_PLAN_VANILLA", os.environ.get("HOI4_VANILLA", ""))
#: Default mod overlay (the AMSB test mod). Relative to this repo's parent dir.
_DEFAULT_MOD = os.environ.get(
    "BATTLE_PLAN_MOD",
    str(_REPO_ROOT.parent.parent / "test"),
)

# --- engine defines for the armor/piercing max-vs-average blend ---
# source: common/defines/00_defines.lua NDefines.NMilitary:
#   ARMOR_VS_AVERAGE = 0.4,   -- "how to weight in highest armor & pen vs the
#   PEN_VS_AVERAGE   = 0.4,       division average"  (VERIFIED in the real file)
# A division's effective armor/piercing is a weighted blend of the BEST sub-unit
# value and the AVERAGE across all sub-units (so one tank battalion does not give
# the whole division its full armor). These module-level constants are the vanilla
# defaults; GameDefs.armor_vs_average / pen_vs_average parse the live defines (incl.
# a mod override) and are what compute_division_stats actually uses.
ARMOR_VS_AVERAGE = float(os.environ.get("BATTLE_PLAN_ARMOR_VS_AVERAGE", os.environ.get("HOI4_ARMOR_VS_AVERAGE", "0.4")))
PEN_VS_AVERAGE = float(os.environ.get("BATTLE_PLAN_PEN_VS_AVERAGE", os.environ.get("HOI4_PEN_VS_AVERAGE", "0.4")))


def _blend(values, max_weight: float) -> float:
    """blend = max_weight*max + (1-max_weight)*average. Empty -> 0."""
    if not values:
        return 0.0
    mx = max(values)
    avg = sum(values) / len(values)
    return max_weight * mx + (1.0 - max_weight) * avg


# ===========================================================================
# 1. GAME DEFS: parse common/units/** with mod-over-vanilla by filename.
# ===========================================================================
@dataclass
class GameDefs:
    """Parsed sub_unit, equipment and module defs with mod overrides applied."""
    vanilla_dir: Path
    mod_dir: Optional[Path] = None

    sub_units: dict[str, "SubUnit"] = field(default_factory=dict)
    equipments: dict[str, "Equip"] = field(default_factory=dict)   # raw equip nodes by name
    modules: dict[str, dict] = field(default_factory=dict)          # module name -> {add, mult, avg}
    armor_vs_average: float = ARMOR_VS_AVERAGE   # live define (mod-overridable)
    pen_vs_average: float = PEN_VS_AVERAGE

    def __post_init__(self):
        self.vanilla_dir = Path(self.vanilla_dir)
        if self.mod_dir is not None:
            self.mod_dir = Path(self.mod_dir)
        self._load()
        self._load_defines()

    # --- file resolution ---------------------------------------------------
    def _unit_files(self) -> list[Path]:
        """common/units/*.txt with mod files shadowing vanilla by filename.

        # source: the game override model -- a mod file at the same relative path
        # fully replaces the vanilla file (replace_path-equivalent for content).
        """
        van = self.vanilla_dir / "common" / "units"
        files: dict[str, Path] = {}
        for p in sorted(van.glob("*.txt")):
            files[p.name] = p
        if self.mod_dir:
            mod = self.mod_dir / "common" / "units"
            if mod.is_dir():
                for p in sorted(mod.glob("*.txt")):
                    files[p.name] = p  # mod wins
        return list(files.values())

    def _equip_files(self) -> list[Path]:
        van = self.vanilla_dir / "common" / "units" / "equipment"
        files: dict[str, Path] = {}
        for p in sorted(van.glob("*.txt")):
            files[p.name] = p
        if self.mod_dir:
            mod = self.mod_dir / "common" / "units" / "equipment"
            if mod.is_dir():
                for p in sorted(mod.glob("*.txt")):
                    files[p.name] = p
        return list(files.values())

    def _module_files(self) -> list[Path]:
        van = self.vanilla_dir / "common" / "units" / "equipment" / "modules"
        files: dict[str, Path] = {}
        if van.is_dir():
            for p in sorted(van.glob("*.txt")):
                files[p.name] = p
        if self.mod_dir:
            mod = self.mod_dir / "common" / "units" / "equipment" / "modules"
            if mod.is_dir():
                for p in sorted(mod.glob("*.txt")):
                    files[p.name] = p
        return list(files.values())

    @staticmethod
    def _read(p: Path) -> Node:
        # AMSB / the game .txt are Windows-1252 / mixed; decode permissively.
        raw = p.read_bytes().decode("cp1252", errors="replace")
        return parse(raw)

    def _load_defines(self):
        """Read ARMOR_VS_AVERAGE / PEN_VS_AVERAGE from common/defines (mod wins).

        # source: common/defines/00_defines.lua NDefines.NMilitary.
        # These are Lua assignments, not Clausewitz script, so we regex them.
        """
        import re
        pat = re.compile(r"(ARMOR_VS_AVERAGE|PEN_VS_AVERAGE)\s*=\s*([0-9.]+)")
        vals: dict[str, float] = {}
        roots = [self.vanilla_dir / "common" / "defines"]
        if self.mod_dir:
            roots.append(self.mod_dir / "common" / "defines")  # mod last -> wins
        for root in roots:
            if not root.is_dir():
                continue
            for p in sorted(root.glob("*.lua")):
                try:
                    txt = p.read_bytes().decode("utf-8", errors="replace")
                except OSError:
                    continue
                for m in pat.finditer(txt):
                    vals[m.group(1)] = float(m.group(2))
        if "ARMOR_VS_AVERAGE" in vals:
            self.armor_vs_average = vals["ARMOR_VS_AVERAGE"]
        if "PEN_VS_AVERAGE" in vals:
            self.pen_vs_average = vals["PEN_VS_AVERAGE"]

    def _load(self):
        # --- sub_units ---
        for p in self._unit_files():
            root = self._read(p)
            for _, blk in root.children_named("sub_units"):
                if not isinstance(blk, Node):
                    continue
                for name, body in blk.entries:
                    if isinstance(body, Node) and name:
                        self.sub_units[name] = SubUnit.from_node(name, body)
        # --- equipments (archetypes + variants) ---
        for p in self._equip_files():
            root = self._read(p)
            for _, blk in root.children_named("equipments"):
                if not isinstance(blk, Node):
                    continue
                for name, body in blk.entries:
                    if isinstance(body, Node) and name:
                        self.equipments[name] = Equip.from_node(name, body)
        # --- modules (tank/plane/ship designer modules) ---
        for p in self._module_files():
            root = self._read(p)
            self._collect_modules(root)

    def _collect_modules(self, node: Node):
        """Recurse; any block with add_stats/multiply_stats is a module.

        # source: common/units/equipment/modules/*.txt -- modules live under
        # `equipment_modules = { limit = {..} <name> = { add_stats multiply_stats } }`
        # and the limit blocks vary, so we walk generically.
        """
        for key, body in node.entries:
            if not isinstance(body, Node):
                continue
            add = body.get("add_stats")
            mult = body.get("multiply_stats")
            avg = body.get("add_average_stats")
            if isinstance(add, Node) or isinstance(mult, Node) or isinstance(avg, Node):
                if key:  # named module
                    self.modules[key] = {
                        "add": _stat_dict(add),
                        "mult": _stat_dict(mult),
                        "avg": _stat_dict(avg),
                    }
            # recurse into container blocks (equipment_modules, limit groups)
            self._collect_modules(body)

    # --- equipment stat resolution ----------------------------------------
    def resolve_equip_stats(self, equip_name: str) -> dict[str, float]:
        """Final per-unit stats for an equipment variant name.

        Non-modular: walk parent/archetype chain, child overrides win.
        # source: equipment/_documentation.md -- `parent` + `archetype` chain;
        # a variant only lists the stats it changes, inheriting the rest.
        Modular (tank chassis with modules): chassis base + sum(add_stats),
        then apply multiply_stats, then average modules.
        """
        eq = self.equipments.get(equip_name)
        if eq is None:
            return {}
        base = self._inherited_stats(equip_name)
        # if a save-side variant carries modules, fold them in
        return base

    def _inherited_stats(self, name: str, _seen=None) -> dict[str, float]:
        _seen = _seen or set()
        if name in _seen:
            return {}
        _seen.add(name)
        eq = self.equipments.get(name)
        if eq is None:
            return {}
        stats: dict[str, float] = {}
        # archetype provides the base; then parent chain; then this variant.
        if eq.archetype and eq.archetype != name:
            stats.update(self._inherited_stats(eq.archetype, _seen))
        if eq.parent and eq.parent != name:
            stats.update(self._inherited_stats(eq.parent, _seen))
        stats.update(eq.stats)
        return stats

    def apply_modules(self, base_stats: dict[str, float],
                      module_names: list[str]) -> dict[str, float]:
        """Modular designer math for tanks/planes/ships.

        # source: modules/*.txt -- each module has add_stats (flat add) and
        # multiply_stats (fractional bonus on the running total). Engine applies
        # ALL adds first, then all multiplies. add_average_stats are summed then
        # divided by the module count (averaged) -- modeled as a post add.
        """
        out = dict(base_stats)
        add_total: dict[str, float] = {}
        mult_total: dict[str, float] = {}
        avg_total: dict[str, float] = {}
        n_avg_modules = 0
        for m in module_names:
            mod = self.modules.get(m)
            if not mod:
                continue
            for k, v in mod["add"].items():
                add_total[k] = add_total.get(k, 0.0) + v
            for k, v in mod["mult"].items():
                mult_total[k] = mult_total.get(k, 0.0) + v
            if mod["avg"]:
                n_avg_modules += 1
                for k, v in mod["avg"].items():
                    avg_total[k] = avg_total.get(k, 0.0) + v
        # 1) flat adds
        for k, v in add_total.items():
            out[k] = out.get(k, 0.0) + v
        # 2) averaged adds (divide by number of avg-contributing modules)
        if n_avg_modules:
            for k, v in avg_total.items():
                out[k] = out.get(k, 0.0) + v / n_avg_modules
        # 3) multiplies on the running total
        for k, v in mult_total.items():
            out[k] = out.get(k, 0.0) * (1.0 + v)
        return out


def _stat_dict(node) -> dict[str, float]:
    """Flatten a stat block (add_stats/multiply_stats/base equip body) to floats."""
    out: dict[str, float] = {}
    if not isinstance(node, Node):
        return out
    for k, v in node.entries:
        if isinstance(v, Node):
            continue
        c = coerce_scalar(v)
        if isinstance(c, (int, float)):
            out[k.lower()] = float(c)
    return out


@dataclass
class SubUnit:
    name: str
    # size / misc
    combat_width: float = 0.0
    max_strength: float = 0.0
    max_organisation: float = 0.0
    default_morale: float = 0.0
    manpower: float = 0.0
    training_time: float = 0.0
    suppression: float = 0.0
    weight: float = 0.0
    supply_consumption: float = 0.0
    maximum_speed: Optional[float] = None
    entrenchment: float = 0.0
    recon: float = 0.0
    initiative: float = 0.0
    hardness: float = 0.0
    armor_value: float = 0.0
    affects_speed: bool = True
    is_support: bool = False           # group == support / regimental == no
    # inline combat-stat MULTIPLIERS (applied to equipment-derived stats)
    inline: dict[str, float] = field(default_factory=dict)
    need: dict[str, float] = field(default_factory=dict)   # archetype -> amount
    transport: Optional[str] = None    # archetype that sets speed (motorized/mech)
    battalion_mult: list[dict] = field(default_factory=list)

    # NOTE: hardness and armor_value are NOT inline multipliers -- they are
    # flat sub_unit fields (su.hardness / su.armor_value) applied additively in
    # battalion_contribution; listing them here would double-count them.
    _INLINE_KEYS = ("soft_attack", "hard_attack", "air_attack", "ap_attack",
                    "defense", "breakthrough")

    @classmethod
    def from_node(cls, name: str, node: Node) -> "SubUnit":
        su = cls(name=name)

        def num(key, default=0.0):
            v = node.get(key)
            c = coerce_scalar(v) if v is not None else None
            return float(c) if isinstance(c, (int, float)) else default

        su.combat_width = num("combat_width")
        su.max_strength = num("max_strength")
        su.max_organisation = num("max_organisation")
        su.default_morale = num("default_morale")
        su.manpower = num("manpower")
        su.training_time = num("training_time")
        su.suppression = num("suppression")
        su.weight = num("weight")
        su.supply_consumption = num("supply_consumption")
        su.entrenchment = num("entrenchment")
        su.recon = num("recon")
        su.initiative = num("initiative")
        su.hardness = num("hardness")
        su.armor_value = num("armor_value")
        sp = node.get("maximum_speed")
        if sp is not None:
            c = coerce_scalar(sp)
            su.maximum_speed = float(c) if isinstance(c, (int, float)) else None
        af = node.get("affects_speed")
        su.affects_speed = (str(af).lower() != "no") if af is not None else True
        group = (node.get("group") or "").lower() if isinstance(node.get("group"), str) else ""
        regimental = node.get("regimental")
        su.is_support = (group == "support") or (str(regimental).lower() == "no")
        tr = node.get("transport")
        su.transport = tr if isinstance(tr, str) else None
        # inline combat-stat multipliers
        for k in cls._INLINE_KEYS:
            v = node.get(k)
            if v is not None:
                c = coerce_scalar(v)
                if isinstance(c, (int, float)):
                    su.inline[k] = float(c)
        # need block
        nb = node.get("need")
        if isinstance(nb, Node):
            for k, v in nb.entries:
                c = coerce_scalar(v)
                if isinstance(c, (int, float)):
                    su.need[k] = float(c)
        # battalion_mult blocks (bonuses pushed onto OTHER battalions)
        for _, bm in node.children_named("battalion_mult"):
            if isinstance(bm, Node):
                d = {"category": None, "add": False, "stats": {}}
                for k, v in bm.entries:
                    if k == "category" and isinstance(v, str):
                        d["category"] = v
                    elif k == "add":
                        d["add"] = (str(v).lower() == "yes")
                    else:
                        c = coerce_scalar(v)
                        if isinstance(c, (int, float)):
                            d["stats"][k.lower()] = float(c)
                su.battalion_mult.append(d)
        return su


@dataclass
class Equip:
    name: str
    archetype: Optional[str]
    parent: Optional[str]
    stats: dict[str, float]

    @classmethod
    def from_node(cls, name: str, node: Node) -> "Equip":
        arche = node.get("archetype")
        par = node.get("parent")
        stats = _stat_dict(node)
        return cls(
            name=name,
            archetype=arche if isinstance(arche, str) else None,
            parent=par if isinstance(par, str) else None,
            stats=stats,
        )


# ===========================================================================
# 2. PER-BATTALION CONTRIBUTION
# ===========================================================================
def battalion_contribution(su: SubUnit,
                           equip_resolver,
                           variant_for_archetype: dict[str, str]) -> dict[str, float]:
    """Compute one battalion's combat-stat contribution.

    equip_resolver(name) -> dict of per-unit equipment stats.
    variant_for_archetype maps an archetype like 'infantry_equipment' to the
    chosen variant name (e.g. 'infantry_equipment_2'). If absent, the archetype
    name itself is used.

    # source: common/units/<file>.txt sub_unit `need` + inline modifiers;
    # equipment/<file>.txt per-unit stats.
    """
    # Equipment-derived stats. The `need` AMOUNT is the equipment QUANTITY
    # (stockpile / IC cost / reinforcement) and does NOT scale combat stats:
    # the displayed equipment stat IS the full per-battalion contribution.
    # (Codex-verified vs the game wiki Land_units / Unit_modding pages: there is
    #  no /100 divisor -- line artillery with need=24 and artillery soft_attack=25
    #  contributes 25, not 6.) A battalion that needs MULTIPLE archetypes adds
    #  each one's relevant stats (e.g. motorized = infantry_equipment combat stats
    #  + motorized_equipment hardness/armor/speed).
    # source: the game Land_units + equipment/_documentation.md; need = equipment count.
    derived: dict[str, float] = {}
    for archetype, amount in su.need.items():
        variant = variant_for_archetype.get(archetype, archetype)
        estats = equip_resolver(variant) or {}
        for k, v in estats.items():
            if k in ("soft_attack", "hard_attack", "air_attack", "ap_attack",
                     "defense", "breakthrough"):
                derived[k] = derived.get(k, 0.0) + v
            elif k in ("armor_value", "hardness"):
                # armor/hardness come from the heaviest equipment the battalion
                # uses (a motorized inf gets 0 armor from infantry_equipment and
                # 0 from motorized_equipment; a tank gets the chassis armor).
                derived[k] = max(derived.get(k, 0.0), v)
            elif k == "maximum_speed":
                # slowest equipment caps the battalion before the sub_unit mult
                derived[k] = v if "maximum_speed" not in derived else min(derived["maximum_speed"], v)
            elif k == "reliability":
                # lowest-reliability equipment dominates (multiplicative in-engine;
                # we keep the min as a conservative per-battalion proxy)
                derived[k] = v if "reliability" not in derived else min(derived["reliability"], v)
            elif k in ("fuel_consumption", "supply_consumption"):
                derived[k] = derived.get(k, 0.0) + v

    # Apply inline sub_unit MULTIPLIERS to the equipment-derived combat stats.
    # source: support companies (e.g. artillery support `soft_attack = -0.4`) and
    # mechanized (`hard_attack = 4.0`) -- final = derived * (1 + inline).
    out: dict[str, float] = {}
    for k in ("soft_attack", "hard_attack", "air_attack", "ap_attack",
              "defense", "breakthrough"):
        base = derived.get(k, 0.0)
        mult = su.inline.get(k, 0.0)
        out[k] = base * (1.0 + mult)

    # hardness: equipment hardness (max over equipment) + sub_unit flat
    # hardness (additive).
    # source: mechanized.txt etc. carry hardness on the sub_unit (#hardness = 0.2).
    out["hardness"] = derived.get("hardness", 0.0) + su.hardness
    # armor: flat-additive -- equipment armor_value (max over the battalion's
    # equipment) + sub_unit flat armor_value; not need-scaled; kept for the
    # division-level 0.4*max + 0.6*avg blend.
    out["armor_value"] = derived.get("armor_value", 0.0) + su.armor_value

    # size / org / hp / width come from the sub_unit itself (not equipment)
    out["max_strength"] = su.max_strength
    out["max_organisation"] = su.max_organisation
    out["default_morale"] = su.default_morale
    out["combat_width"] = su.combat_width
    out["manpower"] = su.manpower
    out["suppression"] = su.suppression
    out["weight"] = su.weight
    out["entrenchment"] = su.entrenchment
    out["recon"] = su.recon
    out["initiative"] = su.initiative
    out["supply_consumption"] = su.supply_consumption + derived.get("supply_consumption", 0.0)
    out["fuel_consumption"] = derived.get("fuel_consumption", 0.0)
    out["reliability"] = derived.get("reliability", 1.0)
    out["is_support"] = su.is_support
    out["affects_speed"] = su.affects_speed

    # speed: the battalion's speed comes from its transport / propulsion
    # equipment (motorized / mechanized / tank chassis). The sub_unit's own
    # `maximum_speed` is a MULTIPLIER on the equipment speed:
    #   final = equip_speed * (1 + sub_unit.maximum_speed)
    # (Codex-verified). Foot units have no transport; their speed comes from the
    # infantry_equipment maximum_speed (4). cavalry/recon use maximum_speed<1 to
    # slow down; motorized uses motorized_equipment (faster) with no multiplier.
    # source: the game Land_units (speed = slowest sub-unit speed); recon.txt
    # maximum_speed = 0.6 multiplier.
    equip_speed = None
    if su.transport:
        tvar = variant_for_archetype.get(su.transport, su.transport)
        tstats = equip_resolver(tvar) or {}
        equip_speed = tstats.get("maximum_speed")
    if equip_speed is None:
        equip_speed = derived.get("maximum_speed", 4.0)  # foot units: inf eq speed
    mult = su.maximum_speed if su.maximum_speed is not None else 0.0
    out["maximum_speed"] = equip_speed * (1.0 + mult)
    return out


# ===========================================================================
# 3. DIVISION AGGREGATION
# ===========================================================================
@dataclass
class DivisionStats:
    soft_attack: float = 0.0
    hard_attack: float = 0.0
    air_attack: float = 0.0
    defense: float = 0.0
    breakthrough: float = 0.0
    armor: float = 0.0
    piercing: float = 0.0
    hardness: float = 0.0
    max_organisation: float = 0.0
    max_strength: float = 0.0          # division HP
    combat_width: float = 0.0
    max_speed: float = 0.0
    supply_use: float = 0.0
    fuel_use: float = 0.0
    reliability: float = 1.0
    initiative: float = 0.0
    entrenchment: float = 0.0
    recon: float = 0.0
    suppression: float = 0.0
    default_morale: float = 0.0        # org recovery proxy
    manpower: float = 0.0
    n_battalions: int = 0
    n_support: int = 0

    def as_dict(self) -> dict:
        return {k: round(v, 4) if isinstance(v, float) else v
                for k, v in self.__dict__.items()}


def compute_division_stats(template: dict,
                           defs: GameDefs,
                           variant_for_archetype: Optional[dict[str, str]] = None,
                           equip_resolver=None
                           ) -> DivisionStats:
    """Aggregate a full template's derived stats.

    template: a save designs.json division_template dict with `regiments` and
    `support` lists of {type: <sub_unit key>}.
    variant_for_archetype: optional map archetype->variant name (else archetype
    base stats are used, i.e. the un-upgraded baseline).
    equip_resolver: optional callable name->stats (e.g. from
    make_modular_resolver to include tank modules). Defaults to plain
    parent/archetype inheritance.
    """
    variant_for_archetype = variant_for_archetype or {}
    resolver = equip_resolver or defs.resolve_equip_stats

    regiments = [r["type"] for r in template.get("regiments", [])]
    support = [s["type"] for s in template.get("support", [])]

    line_contribs = []
    support_contribs = []
    for t in regiments:
        su = defs.sub_units.get(t)
        if su is None:
            continue
        line_contribs.append((su, battalion_contribution(su, resolver, variant_for_archetype)))
    for t in support:
        su = defs.sub_units.get(t)
        if su is None:
            continue
        support_contribs.append((su, battalion_contribution(su, resolver, variant_for_archetype)))

    all_contribs = line_contribs + support_contribs
    ds = DivisionStats()
    ds.n_battalions = len(line_contribs)
    ds.n_support = len(support_contribs)

    # --- battalion_mult: support companies (and some line units) push bonuses
    # onto OTHER battalions in the division by category. We approximate by
    # applying multiplicative `stats` bonuses to matching line battalions.
    # source: sub_unit `battalion_mult = { category=.. <stat>=.. }`.
    _apply_battalion_mults(defs, line_contribs, all_contribs)

    # --- SUM stats over all battalions (incl. support) ---
    for _, c in all_contribs:
        ds.soft_attack += c.get("soft_attack", 0.0)
        ds.hard_attack += c.get("hard_attack", 0.0)
        ds.air_attack += c.get("air_attack", 0.0)
        ds.defense += c.get("defense", 0.0)
        ds.breakthrough += c.get("breakthrough", 0.0)
        ds.max_strength += c.get("max_strength", 0.0)
        ds.suppression += c.get("suppression", 0.0)
        ds.supply_use += c.get("supply_consumption", 0.0)
        ds.fuel_use += c.get("fuel_consumption", 0.0)
        ds.entrenchment += c.get("entrenchment", 0.0)
        ds.recon = max(ds.recon, c.get("recon", 0.0))   # recon does not stack additively
        ds.manpower += c.get("manpower", 0.0)

    # combat width = sum over LINE battalions (support combat_width = 0 anyway)
    ds.combat_width = sum(c.get("combat_width", 0.0) for _, c in line_contribs)

    # --- MAX ORG: simple arithmetic mean over ALL placed sub-units INCLUDING
    # support companies (and including 0-org line artillery / support arty).
    # (Codex-verified vs the game Land_units: NOT manpower-weighted, NOT filtered to
    # org>0 -- engineer org=20 and line-arty org=0 both count.) Empty template
    # slots are simply absent from the lists, so they are already ignored.
    # source: the game Land_units "Organization is the average of all battalions".
    org_vals = [c.get("max_organisation", 0.0) for _, c in all_contribs]
    ds.max_organisation = (sum(org_vals) / len(org_vals)) if org_vals else 0.0

    # default morale (org recovery rate) -- averaged the same way (all sub-units).
    mor_vals = [c.get("default_morale", 0.0) for _, c in all_contribs]
    ds.default_morale = (sum(mor_vals) / len(mor_vals)) if mor_vals else 0.0

    # --- HARDNESS: simple average of per-sub-unit hardness over all sub-units.
    # source: the game Land_units -- hardness is the average across battalions.
    hd_vals = [c.get("hardness", 0.0) for _, c in all_contribs]
    ds.hardness = (sum(hd_vals) / len(hd_vals)) if hd_vals else 0.0

    # --- ARMOR & PIERCING: defines blend of max + average over all sub-units.
    #   armor    = ARMOR_VS_AVERAGE * max_armor + (1-ARMOR_VS_AVERAGE) * avg_armor
    #   piercing = PEN_VS_AVERAGE   * max_pierce + (1-PEN_VS_AVERAGE)   * avg_pierce
    # vanilla defines: ARMOR_VS_AVERAGE = 0.4, PEN_VS_AVERAGE = 0.4.
    # source: 00_defines.lua NDefines.NMilitary; the game Land_units. (Codex-verified.)
    armor_vals = [c.get("armor_value", 0.0) for _, c in all_contribs]
    pierce_vals = [c.get("ap_attack", 0.0) for _, c in all_contribs]
    ds.armor = _blend(armor_vals, defs.armor_vs_average)
    ds.piercing = _blend(pierce_vals, defs.pen_vs_average)

    # --- SPEED: slowest sub-unit speed among those that affect speed.
    speeds = [c.get("maximum_speed", 4.0) for su, c in all_contribs
              if su.affects_speed]
    ds.max_speed = min(speeds) if speeds else 0.0

    # --- RELIABILITY: HP-weighted average of equipment reliability ---
    tot_hp = sum(c.get("max_strength", 0.0) for _, c in all_contribs) or 1.0
    rel = sum(c.get("reliability", 1.0) * c.get("max_strength", 0.0)
              for _, c in all_contribs)
    ds.reliability = rel / tot_hp

    # initiative: sum of sub_unit initiative contributions
    ds.initiative = sum(c.get("initiative", 0.0) for _, c in all_contribs)

    return ds


def _apply_battalion_mults(defs: GameDefs, line_contribs, all_contribs):
    """Apply support/line battalion_mult bonuses to matching line battalions.

    # source: sub_unit `battalion_mult = { category = <cat> <stat> = <x> }`.
    # The bonus is multiplicative on the target battalion's stat unless add=yes
    # (then it's an additive bonus, typically for entrenchment/recon).
    cat membership is read from the sub_unit `categories` set -- here we resolve
    via the GameDefs sub_unit registry's recorded categories.
    """
    # Build category membership for line sub_units.
    for su, _c in all_contribs:
        for bm in su.battalion_mult:
            cat = bm.get("category")
            if not cat:
                continue
            for tsu, tc in line_contribs:
                if _sub_in_category(defs, tsu.name, cat):
                    for stat, val in bm["stats"].items():
                        if bm.get("add"):
                            tc[stat] = tc.get(stat, 0.0) + val
                        else:
                            tc[stat] = tc.get(stat, 0.0) * (1.0 + val)


def _sub_in_category(defs: GameDefs, sub_name: str, cat: str) -> bool:
    # Category membership is cached lazily on the GameDefs instance itself
    # (getattr because _category_cache is not a declared dataclass field), so
    # the cache's lifetime is tied to the instance -- no stale-id reuse, no
    # module-level leak.
    cache = getattr(defs, "_category_cache", None)
    if cache is None:
        cache = _build_category_cache(defs)
        defs._category_cache = cache
    return cat in cache.get(sub_name, set())


def _build_category_cache(defs: GameDefs) -> dict[str, set]:
    """Re-read categories from the unit files (not stored on SubUnit by default)."""
    cache: dict[str, set] = {}
    for p in defs._unit_files():
        root = defs._read(p)
        for _, blk in root.children_named("sub_units"):
            if not isinstance(blk, Node):
                continue
            for name, body in blk.entries:
                if isinstance(body, Node) and name:
                    cats = body.get("categories")
                    s = set()
                    if isinstance(cats, Node):
                        for k, v in cats.entries:
                            if k == "" and isinstance(v, str):
                                s.add(v)
                    cache[name] = s
    return cache


# ===========================================================================
# 4. CONVENIENCE: build a variant map from a deployed division's equipment list
# ===========================================================================
def variant_map_from_division(division: dict) -> dict[str, str]:
    """A deployed division stores the actual equipment archetype+variant it holds.

    divisions.json entries carry `equipment = [{archetype, equip_id, ...}]`.
    The archetype field is the *variant* name (e.g. 'infantry_equipment_2');
    map its base archetype to that variant.
    """
    out: dict[str, str] = {}
    for e in division.get("equipment", []):
        variant = e.get("archetype")
        if not variant:
            continue
        # base archetype = strip trailing _<n> if present, else itself
        base = variant
        if "_" in variant and variant.rsplit("_", 1)[-1].isdigit():
            base = variant.rsplit("_", 1)[0]
        out[base] = variant
    return out


def make_modular_resolver(defs: GameDefs,
                          equipment_variants: dict[str, dict]):
    """Build an equip_resolver that folds modular designer modules into stats.

    equipment_variants is the save designs.json 'equipment_variants' map
    (key 'id:type' -> {archetype, modules, upgrades, ...}). For a modular
    variant (tank chassis / plane frame / ship hull) we look up its module
    list and apply add/multiply/average stats. For non-modular equipment we
    fall back to plain parent/archetype inheritance.

    The returned resolver accepts EITHER an equipment archetype/variant name
    (e.g. 'medium_tank_chassis_2') and uses the catalog entry whose archetype
    matches, OR a catalog key. It returns the final per-unit stat dict.

    # source: designs.json equipment_variants.modules + modules/*.txt math.
    """
    # index catalog entries by archetype (last one wins -- newest design)
    by_arche: dict[str, dict] = {}
    for _key, v in equipment_variants.items():
        arche = v.get("archetype")
        if arche:
            by_arche[arche] = v

    _cache: dict[str, dict] = {}

    def resolver(name: str) -> dict[str, float]:
        if name in _cache:
            return _cache[name]
        base = defs.resolve_equip_stats(name)
        cat = by_arche.get(name)
        if cat and cat.get("modules"):
            module_names = [m for m in cat["modules"].values()
                            if isinstance(m, str) and m and m != "empty"]
            base = defs.apply_modules(base, module_names)
        _cache[name] = base
        return base

    return resolver


# ===========================================================================
# AGGREGATION NOTES (rationale, with sources)
# ===========================================================================
__doc_aggregation__ = """
All rules below were adversarially reviewed by Codex (two independent runs that
web-searched the game wiki Land_units / Unit_modding pages); the first draft of
this model had three errors (A, D, G) which Codex caught and which are corrected
here.

A) Equipment-stat scaling -- there is NO need-amount divisor. *** CORRECTED ***
   # source: the game Land_units + equipment/_documentation.md.
   The `need = { archetype = N }` amount is the equipment QUANTITY (stockpile /
   IC cost / reinforcement), NOT a stat multiplier. The equipment stat IS the
   per-battalion contribution. Line artillery (need { artillery_equipment = 24 },
   artillery soft_attack = 25) contributes the FULL 25, not 6. Infantry happens
   to need 100 so the old buggy "/100" gave the right answer for infantry only --
   which is why infantry validated but artillery/marine(150)/support(10..40) did
   not. A battalion needing multiple archetypes adds each archetype's stats
   (combat stats from the weapon equipment; armor/hardness/speed from the heaviest
   / propulsion equipment).

B) Sub_unit inline stat = MULTIPLIER on equipment-derived stat
   # source: artillery.txt support `artillery` soft_attack = -0.4 (=> *0.6);
   #         infantry.txt mechanized hard_attack = 4.0 (=> *5.0 of mech eq hard atk);
   #         engineer.txt engineer defense = 0.1, breakthrough = 0.5 (=> *1.1, *1.5)
   final = equip_derived * (1 + inline).

C) Division attack/defense/breakthrough = SUM over ALL battalions incl. support.
   # source: the game Land_units -- the unit view sums each battalion's contribution.

D) MAX ORG = simple arithmetic mean over ALL placed sub-units. *** CORRECTED ***
   # source: the game Land_units -- org is the average across ALL battalions AND
   # support companies, including 0-org line artillery and 0-org support arty.
   # NOT manpower-weighted, NOT filtered to org>0, support NOT excluded. So
   # 6 infantry(60) + 1 engineer(20) => org = (6*60 + 20)/7 = 54.29, not 60.
   default_morale (org recovery rate) is averaged the same way.

E) MAX HP (strength) = SUM of every battalion's max_strength (incl. support ~0.2).
   # source: infantry.txt max_strength = 25; engineer.txt max_strength = 2.
   # This is the one derived quantity the SAVE also stores (division.strength),
   # so it pins the aggregation: model HP == save strength for a full division.

F) COMBAT WIDTH = SUM of combat_width over all sub-units (support width = 0).
   # source: infantry.txt combat_width = 2; engineer.txt combat_width = 0.

G) HARDNESS / ARMOR / PIERCING -- max-vs-average blend. *** CORRECTED ***
   # source: the game Land_units + 00_defines.lua NDefines.NMilitary.
   hardness  = simple average of per-sub-unit hardness.
   armor     = ARMOR_VS_AVERAGE * max + (1-ARMOR_VS_AVERAGE) * avg  (0.4 vanilla)
   piercing  = PEN_VS_AVERAGE   * max + (1-PEN_VS_AVERAGE)   * avg  (0.4 vanilla)
   (NOT HP-weighted, NOT max-only.) One tank battalion therefore does NOT give
   the whole division its full armor.

H) SPEED = slowest sub-unit speed (affects_speed=no excluded). sub_unit
   maximum_speed is a MULTIPLIER on equipment speed: equip_speed * (1 + ms).
   # source: the game Land_units; recon.txt maximum_speed = 0.6 (cavalry-speed mult).

NOT modeled (need country state, not the template): tech / doctrine / idea /
designer / MIO modifiers, terrain modifiers; battalion_mult is a coarse
approximation. Output = the BASE designer stats, pre-country-bonus.
"""
