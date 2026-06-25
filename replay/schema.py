"""Canonical replay snapshot schema (Python mirror of replay/SCHEMA.md, v1).

Pure stdlib. No game, no network, no I/O on import. Defines:

  * the SCHEMA_VERSION + canonical domain ids,
  * dataclasses for every layer of a SNAPSHOT (§1-§8 of SCHEMA.md),
  * the EVENT record + the event-type enum (§9),
  * FIELD_SOURCES: dotted-path -> source tag (M/M*/S/D), the machine-readable
    mirror of the per-field source tables,
  * helpers: date_to_hour_index, empty_snapshot / empty_country / ...,
  * validate_snapshot(): a lightweight structural validator (no jsonschema dep).

Design rule (DESIGN no-fabrication): every container is always present; an
unobtained scalar is ``None`` (JSON null), never guessed. ``0`` means measured zero.
"""
from __future__ import annotations

import re
from dataclasses import dataclass, field, asdict, fields as dataclass_fields
from typing import Any, Dict, List, Optional

SCHEMA_VERSION = "1"

# Canonical domain ids referenced by provenance.{memory,save}.domains (SCHEMA §2).
DOMAINS = (
    # country domains
    "country_scalars",   # politics scalars (stability/ws/pp/cp) — memory's bread & butter
    "economy",
    "production",
    "tech",
    "focus",
    "decisions",
    "diplomacy",
    "intel",
    "market",            # equipment-market / trade contracts + lend-lease (parse_market)
    "commanders",        # country leader + generals/FM/admirals + advisors (parse_characters)
    # world domains
    "state_ownership",
    "state_control",
    "state_detail",      # buildings/vp/resources/etc.
    "province_control",
    "divisions",
    "combats",
    "country_snapshot",  # the oracle's merged per-country scalar file
    "designs",           # division templates + equipment-variant catalog + MIOs (parse_designs)
    "order_of_battle",   # armies/army-groups/fleets/task-forces/ships/air-wings/plans (parse_oob)
    "occupation",        # occupier-side governance: per-state occupation law (parse_occupation)
    # PROXY/region-level live sidecars promoted to first-class fields (badged below)
    "naval_air",         # naval region control + air superiority (live, proxy/region)
    "supply",            # live supply proxy (where troops are; confidence-badged)
    "weather",           # live per-region temperature + phenomenon (M; no save oracle)
    # NEW WAVE-2 save deep domains (all save-sourced; folded under provenance.save)
    "abilities",         # aces + commander cooldowns + officer corps (parse_abilities)
    "projects",          # special-projects/facilities/scientists deep (parse_projects)
    "market_extra",      # equipment-market price bands + subsidies (parse_market_extra)
    "deployment",        # deploying-division conveyor + expeditionary (parse_deployment)
    # --- Tier-1 EXTRACT save domains (parse_supply/strategic/events/world/country_extra/
    #     players + divisions_full). All save-sourced (S); folded under provenance.save.
    "supply_logistics",  # supply_system_2 per-country logistics (nodes/trucks/lost_*)
    "strategic_air",     # world air-base capacity table + air theatre/group indices
    "strategic_operatives",  # per-operative subversive-activity + intel-network graph
    "raids",             # strategic-bombing/naval-raid target table + per-country priority
    "world_events",      # fired events, threat ledger, sunk-convoy history (events.json)
    "world",             # scenario meta, global vars/flags, tech-sharing, power balance
    "country_extra",     # nukes / dynamic_modifiers / aces / special projects / misc
    "players",           # human player persona names (player.user); MP-only
)

# Resource keys used across economy.resources.* sub-maps (SCHEMA §3.1).
RESOURCE_KEYS = ("steel", "oil", "aluminium", "tungsten", "chromium", "rubber", "coal")

# Building keys for State.buildings (SCHEMA §4). Extensible; unknown keys allowed.
BUILDING_KEYS = (
    "infrastructure", "arms_factory", "industrial_complex", "dockyard",
    "land_fort", "coastal_fort", "anti_air_building", "radar_station",
    "naval_base", "rocket_site", "nuclear_reactor", "synthetic_refinery",
    "fuel_silo", "supply_node", "rail_way",
)

# ---------------------------------------------------------------------------
# Source-tag map: dotted field path -> source tag. Mirrors SCHEMA.md tables.
# Paths use "*" for the per-entity key (country TAG / state id / list element).
# Looked up by source_of() with prefix-fallback so list-element subfields resolve.
# ---------------------------------------------------------------------------
M, MS, S, D = "M", "M*", "S", "D"  # M* spelled "M*" in values below

FIELD_SOURCES: Dict[str, str] = {
    # --- snapshot scalars ---
    "date": S,
    "hour_index": D,
    "save_version": S,
    # --- country.economy (§3.1) ---
    "countries.*.economy.factories.civ": "S",
    "countries.*.economy.factories.mil": "S",
    "countries.*.economy.factories.dockyard": "S",
    "countries.*.economy.factories.under_construction": "S",
    "countries.*.economy.construction_queue": "S",
    "countries.*.economy.resources.produced": "S",
    "countries.*.economy.resources.exported": "S",
    "countries.*.economy.resources.to_export": "S",
    "countries.*.economy.resources.available_derived": "S/D",
    "countries.*.economy.resources.imported": "S",
    "countries.*.economy.fuel.current": "S/M*",
    "countries.*.economy.fuel.max": "S/M*",
    "countries.*.economy.gross_ic_by_equipment": "S/D",
    "countries.*.economy.market_stockpile": "S",
    "countries.*.economy.market_automation": "S",
    "countries.*.economy.logistics": "S",            # supply.json (§3.1)
    # --- country extras + player name (Tier-1 EXTRACT, §3.11/§3.12) ---
    "countries.*.extra": "S",                        # country_extra.json
    "countries.*.human_name": "S",                   # players.json (persona name)
    # --- country.military (§3.2) ---
    "countries.*.military.num_ships": "M/S",
    "countries.*.military.division_count": "M/D",
    "countries.*.military.manpower.available": "S/M*",
    "countries.*.military.manpower.deployed": "S/D",
    "countries.*.military.xp.army": "S",
    "countries.*.military.equipment_stockpile": "S",
    "countries.*.military.convoys": "S",
    "countries.*.military.ships_by_hull": "S/D",
    "countries.*.military.casualties.killed": "S/D",
    "countries.*.military.casualties.captured": "S/D",
    "countries.*.military.command_power": "M",
    # --- country.politics (§3.3) ---
    "countries.*.politics.stability": "M",
    "countries.*.politics.war_support": "M",
    "countries.*.politics.political_power": "M",
    "countries.*.politics.command_power": "M",
    "countries.*.politics.ruling_party": "S",
    "countries.*.politics.party_popularities": "S",
    "countries.*.politics.laws.economy": "S",
    "countries.*.politics.laws.trade": "S",
    "countries.*.politics.laws.conscription": "S",
    "countries.*.politics.ideas_all": "S",
    "countries.*.politics.ideas_other": "S",
    "countries.*.politics.cabinet": "S",
    "countries.*.politics.is_human": "S/M",
    "countries.*.politics.is_major": "S",
    "countries.*.politics.capital_state": "S",
    "countries.*.politics.autonomy": "S",
    "countries.*.politics.faction": "S",
    # --- country.research (§3.4) ---
    "countries.*.research.tech_count": "S",
    "countries.*.research.technologies": "S/D",
    "countries.*.research.in_progress": "S/M*",
    "countries.*.research.research_slots": "S",
    "countries.*.research.original_research_slots": "S",
    "countries.*.research.research_bonuses": "S",
    # --- country.focus (§3.5) ---
    "countries.*.focus.focus_tree": "S",
    "countries.*.focus.continuous_focus_palette": "S",
    "countries.*.focus.completed": "S/D",
    "countries.*.focus.completed_count": "S",
    "countries.*.focus.current": "S/M*",
    "countries.*.focus.progress": "S/M*",
    "countries.*.focus.paused": "S",
    # --- country.decisions / diplomacy / intel (§3.6-3.8) ---
    "countries.*.decisions": "S",
    "countries.*.diplomacy.wars": "S",
    "countries.*.diplomacy.opinion": "S",
    # equipment market / lend-lease (parse_market). Trade contracts attached to
    # both parties under diplomacy.lend_lease; canonical world list in §7b.
    "countries.*.diplomacy.lend_lease": "S",
    "countries.*.diplomacy.expeditionary": "S",
    "countries.*.diplomacy.volunteers": "S",
    "countries.*.diplomacy.attaches": "S",
    # --- country.intel (§3.8, deepened from intel.json) ---
    "countries.*.intel.agency": "S",
    "countries.*.intel.operatives": "S",
    "countries.*.intel.recruitable_operatives": "S",
    "countries.*.intel.operations": "S",
    "countries.*.intel.decryption": "S",
    "countries.*.intel.peace_conference": "S",
    "countries.*.intel.intel_networks": "S",
    # --- country.commanders (§3.9, from characters.json) ---
    "countries.*.commanders.leader": "S",
    "countries.*.commanders.party_leaders": "S",
    "countries.*.commanders.ruling_party": "S",
    "countries.*.commanders.generals": "S",
    "countries.*.commanders.field_marshals": "S",
    "countries.*.commanders.admirals": "S",
    "countries.*.commanders.advisors": "S",
    "countries.*.commanders.counts": "S/D",
    # --- country.scalars (§3.13, derived; models/scalars.py). All D. ---
    "countries.*.scalars.capitulation.cap_pct": "D",
    "countries.*.scalars.capitulation.vp_lost_fraction": "D",
    "countries.*.scalars.capitulation.surrender_threshold": "D",
    "countries.*.scalars.capitulation.surrender_limit_modifier": "D",
    "countries.*.scalars.research_speed.pct": "D",
    "countries.*.scalars.research_speed.multiplier": "D",
    "countries.*.scalars.research_speed.factor_total": "D",
    "countries.*.scalars.special_forces.used_divisions": "D",
    "countries.*.scalars.special_forces.cap": "D",
    "countries.*.scalars.special_forces.special_forces_cap_modifier": "D",
    # --- country.score (§3.14, derived victory-score ladder; models/victory_score) ---
    "countries.*.score.army": "D",
    "countries.*.score.navy": "D",
    "countries.*.score.air": "D",
    "countries.*.score.industry": "D",
    "countries.*.score.territory": "D",
    "countries.*.score.army_n": "D",
    "countries.*.score.navy_n": "D",
    "countries.*.score.air_n": "D",
    "countries.*.score.industry_n": "D",
    "countries.*.score.territory_n": "D",
    "countries.*.score.overall": "D",
    "countries.*.score.rank": "D",
    # --- country.abilities (§3.15, from abilities.json). S, two capture tallies D. ---
    "countries.*.abilities.aces": "S",
    "countries.*.abilities.commander_cooldowns": "S",
    "countries.*.abilities.officer_corps": "S",
    "countries.*.abilities.officer_corps.captured_enemy_generals": "D",
    "countries.*.abilities.officer_corps.own_generals_captured": "D",
    # --- country.projects + scientists (§3.16, from projects.json). S, two joins D. ---
    "countries.*.projects": "S",
    "countries.*.projects.facilities.*.scientist_token": "D",
    "countries.*.projects.active_projects.*.in_facility": "D",
    "countries.*.scientists": "S",
    # --- country.economy market-extra (§3.1, from market_extra.json). S. ---
    "countries.*.economy.market_price_levels": "S",
    "countries.*.economy.market_subsidies": "S",
    # --- country.military deployment (§3.2, from deployment.json). S. ---
    "countries.*.military.deployment_queue": "S",
    "countries.*.military.reinforcement_priority": "S",
    # --- country.mios (§3.10, from designs.json) ---
    "countries.*.mios": "S",
    "countries.*.scheduled_variants": "S",
    "countries.*.mio_policies": "S",
    # --- top-level designs (§11, from designs.json) ---
    "designs.division_templates": "S",
    "designs.equipment_variants": "S",
    # derived design stats (WAVE-2; models/division_stats + models/equipment_stats).
    # Single prefix entry covers every nested stat (source_of longest-prefix match).
    "designs.template_stats": "D",            # {template_id: DivisionStats}
    "designs.equipment_variant_stats": "D",   # {"id:type": VariantStats}
    "designs.equipment_variant_stats.*.archetype": "S",  # echoed straight from variant
    "designs.equipment_variant_stats.*.family": "S",
    # --- top-level order_of_battle (§12, from oob.json) ---
    "order_of_battle.army_groups": "S",
    "order_of_battle.armies": "S",
    "order_of_battle.fleets": "S",
    "order_of_battle.navy_theaters": "S",
    "order_of_battle.air_wings": "S",
    "order_of_battle.theatres": "S",
    "order_of_battle.by_country": "S/D",
    # --- per-country OOB view (§12, derived rollup) ---
    "countries.*.order_of_battle": "S/D",
    # --- state (§4) ---
    "states.*.owner": "M/S",
    "states.*.controller": "M/S",
    "states.*.is_core_of": "S",
    "states.*.buildings": "S/M*",
    "states.*.province_buildings": "S",
    "states.*.victory_points_total": "S",
    "states.*.victory_points_by_province": "S",
    "states.*.resources": "S",
    "states.*.manpower_pool": "S",
    "states.*.category": "M/S",
    "states.*.occupation": "S",
    "states.*.occupation_law": "S",
    "states.*.demilitarized": "S",
    "states.*.supply": "S/M*",
    # --- province (§5) ---
    "provinces.*.controller": "S/D",
    "provinces.*.state_id": "D",
    "provinces.*.supply": "S",
    # --- division (§6) ---
    "divisions.*.div_id": "M/S",
    "divisions.*.div_type": "M/S",
    "divisions.*.owner": "M/S",
    "divisions.*.location_province": "M/S",
    "divisions.*.state_id": "M/D",
    "divisions.*.template_id": "M/S",
    "divisions.*.template_type": "M/S",
    "divisions.*.template_name": "M/S",
    "divisions.*.template_role": "S",
    "divisions.*.organisation": "M",
    "divisions.*.strength": "M/S",
    "divisions.*.equipment": "M/S",
    "divisions.*.experience": "M*",
    "divisions.*.moving": "M*",
    "divisions.*.in_combat": "M*",
    # divisions_full.json additive body (Tier-1 EXTRACT). LITERAL save fields (S).
    "divisions.*.script_id": "S",
    "divisions.*.division_name": "S",
    "divisions.*.max_supply": "S",
    "divisions.*.army_current_supply_ratio": "S",
    "divisions.*.supply_gain": "S",
    "divisions.*.motorization_level": "S",
    "divisions.*.move_priority": "S",
    "divisions.*.last_combat_date": "S",
    "divisions.*.dig_in": "S",
    "divisions.*.dig_in_cap": "S",
    "divisions.*.acclimatization": "S",
    "divisions.*.army_manpower": "S",
    "divisions.*.killed": "S",
    "divisions.*.killer_definition": "S",
    "divisions.*.equipment_archetypes": "S",
    "divisions.*.has_officer": "S",
    # --- combat (§7) ---
    "combats.*": "M*",
    # --- world-scoped trade contracts (§7b, from market.json) ---
    "diplomacy_world.trade_contracts": "S",
    # world-scoped market-extra + deployment (WAVE-2). All S.
    "diplomacy_world.market_price_levels": "S",   # per-contract price band
    "diplomacy_world.market_requests": "S",        # pending request id list
    "diplomacy_world.expeditionary": "S",          # owner->controller lent divisions
    # --- live region/proxy sidecars promoted to first-class fields (§13) ---
    # These are PROXY / region-level / confidence-badged LIVE reads, NOT
    # authoritative per-province/per-ship values. The source is M (live memory) but
    # each carries its own confidence/level badge so a consumer never renders region
    # control or air superiority as if it were per-province ground truth.
    "naval_air.region_control": "M",
    "naval_air.air_superiority": "M",
    "supply.by_province": "M",
    # MODEL air-superiority estimate (WAVE-2, models/air_superiority). Derived
    # (planes x stat); the per-region percent is a W6 display estimate -> D.
    "naval_air.air_superiority_model.*.by_owner.*.score": "D",
    "naval_air.air_superiority_model.*.total_score": "D",
    "naval_air.air_superiority_model.*.leader": "D",
    "naval_air.air_superiority_model.*.percent": "D",
    "naval_air.air_superiority_model_meta": "D",
    # --- live weather domain (§19, WAVE-2). M live; phenom_name + rollup are D. ---
    "weather.regions.*.temp_c": "M",          # SWeatherPerRegion+0x138 i64/1e5
    "weather.regions.*.phenom": "M",          # +0x78 enum (already pinned)
    "weather.regions.*.phenom_name": "D",     # derived from phenom via file-order map
    "weather.regions.*.active": "M",          # +0x5D intensity flag
    "weather.global_rollup": "D",             # rolled up from the M rows
    # --- naval combat-side stats (§7c, WAVE-2, naval_combat_stats). Live M / M*. ---
    "combats.naval.*.attacker.positioning": "M",       # validated live
    "combats.naval.*.attacker.damage_map_size": "M",   # validated live
    "combats.naval.*.attacker.damage_history": "M*",   # layout M; values W1 transient
    "combats.naval.*.attacker.ships.*.state_bitmask": "M",
    "combats.naval.*.attacker.ships.*.engagement_weight": "M",
    "combats.naval.*.attacker.ships.*.damage_scalar": "M*",  # W1 tick-transient
    "combats.naval.*.attacker.ships.*.hit_records": "M*",
    "combats.naval.*.defender.positioning": "M",
    "combats.naval.*.defender.damage_map_size": "M",
    "combats.naval.*.defender.damage_history": "M*",
    "combats.naval.*.defender.ships.*.state_bitmask": "M",
    "combats.naval.*.defender.ships.*.engagement_weight": "M",
    "combats.naval.*.defender.ships.*.damage_scalar": "M*",
    "combats.naval.*.defender.ships.*.hit_records": "M*",
    # --- derived-MODEL supply estimate (§20, WAVE-2, models/supply_number). D. ---
    "models.supply_estimate.provinces.*.supply": "D",     # rows self-badge true tier
    "models.supply_estimate.provinces.*.provenance": "D",
    "models.supply_estimate.provinces.*.confidence": "D",
    "models.supply_estimate.provinces.*.value_kind": "D",
    "models.supply_estimate.states.*.estimated_supply": "D",
    "models.supply_estimate.global_rollup": "D",
    # --- pairwise intel ledger (§3.13b, WAVE-2, models/scalars). All D. ---
    "intel_ledger.*.*.categories.civilian.confidence": "D",
    "intel_ledger.*.*.categories.army.confidence": "D",
    "intel_ledger.*.*.categories.navy.confidence": "D",
    "intel_ledger.*.*.categories.airforce.confidence": "D",
    "intel_ledger.*.*.cipher_progress": "D",
    "intel_ledger.*.*.actively_decrypting": "D",
    # --- Tier-1 EXTRACT world-scoped save domains (§14-§18). All save-sourced (S). ---
    "strategic_air.air_bases": "S",
    "strategic_air.indices": "S",
    "strategic_air.per_country_region_blocks": "S",
    "strategic_operatives.pools": "S",
    "raids.counters": "S",
    "raids.targets": "S",
    "raids.countries": "S",
    "world_events.fired_events": "S",
    "world_events.pending_events": "S",
    "world_events.threat": "S",
    "world_events.sunk_convoys": "S",
    "world_events.convoy_losses_by_killer": "S",
    "world_events.convoy_losses_by_owner": "S",
    "world.scenario": "S",
    "world.variables": "S",
    "world.flags": "S",
    "world.tech_sharing_groups": "S",
    "world.power_balances": "S",
    "world.faction_influence": "S",
    "world.world_tension_inputs": "S",
    "world.saved_event_targets": "S",
    # --- map (§8) ---
    "map.owner_by_state": "M/D",
    "map.control_by_province": "S/D",
    "map.vp_by_state": "S/D",
    "map.controlled_industry_share": "D",
    "map.front_lines": "D",
    # --- events (§9) ---
    "events.*": "D",
}


def source_of(dotted_path: str) -> Optional[str]:
    """Return the canonical source tag for a dotted field path (SCHEMA tables).

    Concrete keys are normalized to '*' so e.g.
    'countries.GER.politics.stability' resolves via 'countries.*.politics.stability'.
    Falls back to the longest matching prefix (so a list element's subfield inherits
    its list's source). Returns None if no rule matches.
    """
    norm = _normalize_path(dotted_path)
    if norm in FIELD_SOURCES:
        return FIELD_SOURCES[norm]
    parts = norm.split(".")
    for cut in range(len(parts) - 1, 0, -1):
        prefix = ".".join(parts[:cut])
        if prefix in FIELD_SOURCES:
            return FIELD_SOURCES[prefix]
    return None


# A path-segment that is an entity KEY (country TAG / numeric id) -> normalize to '*'.
# TAGs are 2-3 uppercase alphanumerics starting with a letter, which covers both
# static tags (GER, SOV) AND dynamic/released tags (D01, D02, ...). Field/domain names
# are lowercase, so they never match (Codex integ-review finding 2).
_KEY_SEG = re.compile(r"^[A-Z][A-Z0-9]{1,2}$|^\d+$")


def _normalize_path(dotted_path: str) -> str:
    out = []
    for i, seg in enumerate(dotted_path.split(".")):
        # The segment right after a known container name is an entity key -> '*'.
        if i > 0 and (_KEY_SEG.match(seg) or seg == "*"):
            out.append("*")
        else:
            out.append(seg)
    return ".".join(out)


# ---------------------------------------------------------------------------
# Date <-> hour_index (SCHEMA §0.3). Clausewitz 365-day calendar, no leap.
# ---------------------------------------------------------------------------
_DAYS_BEFORE_MONTH = (0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334)  # 365-day
_DAYS_IN_MONTH = (31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31)  # no leap (Clausewitz)
_DATE_RE = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:\.(\d+))?$")


def date_to_hour_index(date_str: Optional[str]) -> Optional[int]:
    """Clausewitz date string 'Y.M.D[.H]' -> integer hours-since-epoch, or None.

    day_of_year_index is 0-based; matches the memory_reader epoch note
    (1940.9.2 hour21 -> 17000277). 365-day no-leap calendar. Validates per-month
    day max and 0<=hour<=23 so out-of-range dates (e.g. '1936.2.31.0', '...24')
    are rejected rather than silently aliasing onto another day (Codex P3 #14).
    Returns None on malformed input.
    """
    if not date_str or not isinstance(date_str, str):
        return None
    m = _DATE_RE.match(date_str.strip())
    if not m:
        return None
    year, month, day = int(m.group(1)), int(m.group(2)), int(m.group(3))
    hour = int(m.group(4)) if m.group(4) is not None else 0
    if not (1 <= month <= 12):
        return None
    if not (1 <= day <= _DAYS_IN_MONTH[month - 1]):
        return None
    if not (0 <= hour <= 23):
        return None
    day_of_year_index = _DAYS_BEFORE_MONTH[month - 1] + (day - 1)
    return (year * 365 + day_of_year_index) * 24 + hour


# ---------------------------------------------------------------------------
# Dataclasses. Defaults give a fully-null-but-present skeleton (no fabrication).
# Lists default to empty (always-present container); scalars default to None.
# We store snapshots as plain dicts on disk; dataclasses are the typed mirror +
# the source of the empty-skeleton builders below.
# ---------------------------------------------------------------------------
def _none_dict() -> Dict[str, Any]:
    return {}


@dataclass
class Provenance:
    memory: Optional[Dict[str, Any]] = None
    save: Optional[Dict[str, Any]] = None
    notes: List[str] = field(default_factory=list)


# --- country sub-domains ---
@dataclass
class Economy:
    factories: Dict[str, Any] = field(default_factory=_none_dict)
    construction_queue: List[Dict[str, Any]] = field(default_factory=list)
    resources: Dict[str, Any] = field(default_factory=_none_dict)
    fuel: Dict[str, Any] = field(default_factory=_none_dict)
    gross_ic_by_equipment: Optional[Dict[str, float]] = None
    # supply_system_2 per-country logistics (from supply.json). null until ingested;
    # an obj when measured (incl. genuine 0/0.0 leaves the verifier confirmed exist).
    logistics: Optional[Dict[str, Any]] = None
    # market_extra.json (WAVE-2) — equipment-market manual price bands + subsidies.
    # null/empty until ingested; auto-price worlds leave price_levels empty (S).
    market_price_levels: Optional[List[Dict[str, Any]]] = None
    market_subsidies: Optional[List[Dict[str, Any]]] = None


@dataclass
class Military:
    num_ships: Optional[int] = None
    division_count: Optional[int] = None
    manpower: Dict[str, Any] = field(default_factory=_none_dict)
    xp: Dict[str, Any] = field(default_factory=_none_dict)
    equipment_stockpile: List[Dict[str, Any]] = field(default_factory=list)
    convoys: List[Dict[str, Any]] = field(default_factory=list)
    ships_by_hull: Optional[Dict[str, Any]] = None
    casualties: Dict[str, Any] = field(default_factory=_none_dict)
    command_power: Optional[float] = None
    # deployment.json (WAVE-2) — the deploying-division conveyor + reinforcement
    # priority. null until ingested; only countries with a non-default value (S).
    deployment_queue: Optional[List[Dict[str, Any]]] = None
    reinforcement_priority: Optional[int] = None


@dataclass
class Politics:
    stability: Optional[float] = None
    war_support: Optional[float] = None
    political_power: Optional[float] = None
    command_power: Optional[float] = None
    ruling_party: Optional[str] = None
    party_popularities: Optional[Dict[str, float]] = None
    laws: Dict[str, Any] = field(default_factory=_none_dict)
    ideas_all: List[str] = field(default_factory=list)
    ideas_other: List[str] = field(default_factory=list)
    cabinet: Optional[Dict[str, Any]] = None
    is_human: Optional[bool] = None
    is_major: Optional[bool] = None
    capital_state: Optional[int] = None
    autonomy: Optional[Dict[str, Any]] = None
    faction: Optional[Dict[str, Any]] = None


@dataclass
class Research:
    tech_count: Optional[int] = None
    technologies: List[Dict[str, Any]] = field(default_factory=list)
    in_progress: List[Dict[str, Any]] = field(default_factory=list)
    research_slots: Optional[int] = None
    original_research_slots: Optional[int] = None
    research_bonuses: List[Dict[str, Any]] = field(default_factory=list)


@dataclass
class Focus:
    focus_tree: Optional[str] = None
    continuous_focus_palette: Optional[str] = None
    completed: List[str] = field(default_factory=list)
    completed_count: Optional[int] = None
    current: Optional[str] = None
    progress: Optional[float] = None
    paused: Optional[bool] = None


@dataclass
class Decisions:
    active: List[Dict[str, Any]] = field(default_factory=list)
    timed: List[Dict[str, Any]] = field(default_factory=list)
    missions: List[Dict[str, Any]] = field(default_factory=list)


@dataclass
class Diplomacy:
    wars: List[Dict[str, Any]] = field(default_factory=list)
    guarantees: List[Dict[str, Any]] = field(default_factory=list)
    non_aggression: List[Dict[str, Any]] = field(default_factory=list)
    embargoes: List[Dict[str, Any]] = field(default_factory=list)
    opinion: Optional[Dict[str, Any]] = None
    lend_lease: List[Dict[str, Any]] = field(default_factory=list)
    volunteers: List[Dict[str, Any]] = field(default_factory=list)
    attaches: List[Dict[str, Any]] = field(default_factory=list)
    expeditionary: List[Dict[str, Any]] = field(default_factory=list)


@dataclass
class Intel:
    # agency = {name, icon, is_created, upgrade_progress, current_upgrade,
    #           upgrades{branch:level}, capacity{max_operatives,usable_slots,defense}}
    agency: Optional[Dict[str, Any]] = None
    operatives: List[Dict[str, Any]] = field(default_factory=list)
    recruitable_operatives: List[Dict[str, Any]] = field(default_factory=list)
    intel_networks: List[Dict[str, Any]] = field(default_factory=list)
    # operations = {priority, running[], finished[]}
    operations: Optional[Dict[str, Any]] = None
    # decryption = cryptology {targets[], intel_source}
    decryption: Optional[Dict[str, Any]] = None
    peace_conference: Optional[Dict[str, Any]] = None


@dataclass
class Scalars:
    """Derived per-country scalar cluster (§3.13, from models/scalars.py). All D.

    Every value nullable; ``None`` == not computed / input absent (never a guessed 0).
    Each sub-dict carries its own ``is_estimate`` / ``is_proxy`` honesty flags.
    """
    capitulation: Optional[Dict[str, Any]] = None
    research_speed: Optional[Dict[str, Any]] = None
    special_forces: Optional[Dict[str, Any]] = None


@dataclass
class Commanders:
    """Country leader + military leaders + advisors (from characters.json, §3.9)."""
    leader: Optional[Dict[str, Any]] = None             # active country leader (ruling party)
    party_leaders: List[Dict[str, Any]] = field(default_factory=list)  # all ideology-group leaders
    ruling_party: Optional[str] = None
    generals: List[Dict[str, Any]] = field(default_factory=list)
    field_marshals: List[Dict[str, Any]] = field(default_factory=list)
    admirals: List[Dict[str, Any]] = field(default_factory=list)
    advisors: List[Dict[str, Any]] = field(default_factory=list)  # cabinet/high-command/theorists
    counts: Optional[Dict[str, int]] = None


@dataclass
class Country:
    economy: Dict[str, Any] = field(default_factory=lambda: asdict(Economy()))
    military: Dict[str, Any] = field(default_factory=lambda: asdict(Military()))
    politics: Dict[str, Any] = field(default_factory=lambda: asdict(Politics()))
    research: Dict[str, Any] = field(default_factory=lambda: asdict(Research()))
    focus: Dict[str, Any] = field(default_factory=lambda: asdict(Focus()))
    decisions: Dict[str, Any] = field(default_factory=lambda: asdict(Decisions()))
    diplomacy: Dict[str, Any] = field(default_factory=lambda: asdict(Diplomacy()))
    intel: Dict[str, Any] = field(default_factory=lambda: asdict(Intel()))
    commanders: Dict[str, Any] = field(default_factory=lambda: asdict(Commanders()))
    # derived scalar cluster (§3.13, models/scalars.py). Always present; each
    # sub-field null until computed (no fabrication). All D-source.
    scalars: Dict[str, Any] = field(default_factory=lambda: asdict(Scalars()))
    # derived victory-score ladder block (§3.14, models/victory_score.py). null until
    # computed; the per-country CountryScore.as_dict() shape. All D-source.
    score: Optional[Dict[str, Any]] = None
    # designs.json per-country military-industrial picture (§3.10)
    mios: List[Dict[str, Any]] = field(default_factory=list)
    scheduled_variants: List[Dict[str, Any]] = field(default_factory=list)
    mio_policies: Optional[Dict[str, Any]] = None
    # oob.json per-country rollup (counts of armies/fleets/wings); null until ingested (§12)
    order_of_battle: Optional[Dict[str, Any]] = None
    # country_extra.json (Tier-1 EXTRACT) — null-but-present until ingested (§3.11)
    extra: Optional[Dict[str, Any]] = None
    # players.json — human player's lobby/persona display name (NOT a Steam id).
    # Populates in MP only; null for AI and for SP/missing players (§3.12).
    human_name: Optional[str] = None
    # abilities.json (WAVE-2) — aces + commander cooldowns + officer corps (§3.15).
    # null until ingested; the deep ace/cooldown/officer-corps reach (S, two D tallies).
    abilities: Optional[Dict[str, Any]] = None
    # projects.json (WAVE-2) — special-projects / facilities / breakthrough deep reach
    # (§3.16). null until ingested. All S except two cross-ref join fields (D).
    projects: Optional[Dict[str, Any]] = None
    # projects.json scientists roster for this TAG (§3.16). null until ingested (S).
    scientists: Optional[List[Dict[str, Any]]] = None


@dataclass
class State:
    owner: Optional[str] = None
    controller: Optional[str] = None
    is_core_of: List[str] = field(default_factory=list)
    is_claimed_by: List[str] = field(default_factory=list)
    buildings: Optional[Dict[str, int]] = None
    province_buildings: Optional[Dict[str, int]] = None  # naval_base etc. (province-scoped)
    # VP: total scalar AND per-province map are distinct things (Codex P1 #6).
    victory_points_total: Optional[int] = None
    victory_points_by_province: Optional[Dict[str, int]] = None
    resources: Optional[Dict[str, float]] = None
    # manpower is a pool object {available, locked, total}, not a scalar (Codex P1 #6).
    manpower_pool: Optional[Dict[str, int]] = None
    category: Optional[str] = None
    # occupation is an object {resistance, compliance, *_speed, occupied_country_tag,...}
    occupation: Optional[Dict[str, Any]] = None
    occupation_law: Optional[str] = None
    demilitarized: Optional[bool] = None
    supply: Optional[float] = None


@dataclass
class Province:
    controller: Optional[str] = None
    state_id: Optional[int] = None
    supply: Optional[float] = None
    rail_usage: Optional[float] = None
    route_usage: Optional[float] = None
    naval_base_usage: Optional[float] = None
    port_usage: Optional[float] = None


@dataclass
class Division:
    div_id: Optional[int] = None
    div_type: Optional[int] = None       # Clausewitz id is (id,type); both needed for identity
    owner: Optional[str] = None
    location_province: Optional[int] = None
    state_id: Optional[int] = None
    template_id: Optional[int] = None
    template_type: Optional[int] = None  # template id is (id,type) too
    template_name: Optional[str] = None
    template_role: Optional[str] = None
    organisation: Optional[float] = None
    strength: Optional[float] = None
    equipment: List[Dict[str, Any]] = field(default_factory=list)
    experience: Optional[float] = None
    moving: Optional[bool] = None
    move_target_province: Optional[int] = None
    move_progress: Optional[float] = None
    in_combat: Optional[bool] = None
    encircled: Optional[bool] = None
    supply_status: Optional[float] = None
    # --- divisions_full.json additive body (Tier-1 EXTRACT). LITERAL save fields,
    # not derived; absent on a division iff that division did not store them (null,
    # not fabricated). The 9 already above are NOT re-declared here.
    script_id: Optional[int] = None
    division_name: Optional[Dict[str, Any]] = None   # {type, name_order}
    max_supply: Optional[float] = None
    army_current_supply_ratio: Optional[float] = None
    supply_gain: Optional[float] = None              # genuine 0.0 exists — keep it
    motorization_level: Optional[int] = None
    move_priority: Optional[str] = None
    last_combat_date: Optional[str] = None
    dig_in: Optional[float] = None
    dig_in_cap: Optional[float] = None
    acclimatization: Optional[Dict[str, Any]] = None  # {hot_climate, cold_climate, max}
    army_manpower: Optional[Dict[str, Any]] = None    # {value, need}
    killed: Optional[int] = None
    killer_definition: Optional[str] = None
    equipment_archetypes: Optional[int] = None
    has_officer: Optional[bool] = None


@dataclass
class Combat:
    combat_id: Optional[int] = None
    kind: Optional[str] = None  # "land" | "naval" | "air"
    location_province: Optional[int] = None
    attackers: List[Any] = field(default_factory=list)
    defenders: List[Any] = field(default_factory=list)
    combat_width: Optional[float] = None
    terrain: Optional[str] = None
    progress: Optional[float] = None
    modifiers: Optional[Dict[str, Any]] = None
    # NAVAL combat-side damage/stat rows (WAVE-2, memory_reader/combat/
    # naval_combat_stats). null unless this is a naval combat with a live per-side
    # read. {attacker:{...}, defender:{...}}; positioning/state/weight are M
    # (validated live), damage_history/scalar/hit_records are M* (W1 tick-transient).
    naval_stats: Optional[Dict[str, Any]] = None


@dataclass
class MapLayers:
    owner_by_state: Optional[Dict[str, str]] = None
    control_by_province: Optional[Dict[str, str]] = None
    vp_by_state: Optional[Dict[str, int]] = None
    controlled_industry_share: Optional[Dict[str, float]] = None
    front_lines: List[Dict[str, Any]] = field(default_factory=list)
    strategic_regions: Optional[Dict[str, Any]] = None


@dataclass
class Designs:
    """Top-level world design catalogs (from designs.json, §11). Always present.

    `division_templates` is a list of template dicts (regiment grids); the per-country
    owning tag lives on each element (`country`). `equipment_variants` is a dict keyed
    by "id:type" — the variant CATALOG that production lines / divisions / ships / wings
    join against. Per-country MIOs / scheduled presets / policies live on each Country.
    """
    division_templates: List[Dict[str, Any]] = field(default_factory=list)
    equipment_variants: Dict[str, Any] = field(default_factory=_none_dict)
    # NEW (WAVE-2, DERIVED) — never from the save. None until the model runs.
    # template_stats: {str(template_id): DivisionStats.as_dict()} (models/division_stats)
    template_stats: Optional[Dict[str, Any]] = None
    template_stats_source: Optional[str] = None        # "D" when populated, else None
    # equipment_variant_stats: {"id:type": VariantStats.as_dict()} (models/equipment_stats)
    equipment_variant_stats: Optional[Dict[str, Any]] = None
    equipment_variant_stats_source: Optional[str] = None  # "D" when populated, else None


@dataclass
class WeatherRegion:
    """One strategic-region weather row (§19, live). M = process memory.

    ``temp_c`` is the realized region-aggregate temperature (None when raw==0, i.e.
    not realized this tick — NOT a literal 0 C). ``phenom`` is the +0x78 phenomenon
    enum; ``phenom_name`` its file-order label (D, derived). Always source-badged.
    """
    region_id: Optional[int] = None
    phenom: Optional[int] = None
    phenom_name: Optional[str] = None
    temp_c: Optional[float] = None
    active: Optional[bool] = None


@dataclass
class Weather:
    """Live weather domain (§19, from weather_temp_model + weather_state_reader).

    NEW first-class domain. There is NO save oracle for weather (never persisted),
    so every row is live-memory (M) except ``phenom_name`` + the rollup (D). Always
    present; ``regions`` null until a live read folds it. ``source`` self-badges.
    """
    regions: Optional[Dict[str, Any]] = None   # {region_id(str): WeatherRegion dict}
    global_rollup: Optional[Dict[str, Any]] = None
    source: Optional[str] = None        # "M" when a live read filled it, else null


@dataclass
class NavalAir:
    """Live naval region control + air superiority (§13). PROXY / region-level.

    This is a confidence-badged LIVE read at strategic-region granularity — NOT a
    per-province or per-ship authoritative value. ``region_control`` maps a
    strategic-region id to the controlling-TAG entries the reader resolved;
    ``air_superiority`` maps a region id to the air-dominance record. ``source`` and
    ``confidence`` travel WITH the data so a downstream consumer never renders it as
    if it were ground truth. Always present; null/empty until a live read folds it.
    """
    region_control: Optional[Dict[str, Any]] = None   # {region_id: [entries]}
    air_superiority: Optional[Dict[str, Any]] = None   # {region_id: record}
    source: Optional[str] = None        # "M" when a live read filled it, else null
    confidence: Optional[str] = None    # e.g. "region_proxy" (never "exact")
    # MODEL estimate (WAVE-2, models/air_superiority): stat-weighted per-region
    # contest. Distinct from the raw effective_planes proxy above. Each record
    # self-badges source:"MODEL"; the percent is a W6 display estimate (D), never
    # tagged exact. null until the model runs.
    air_superiority_model: Optional[Dict[str, Any]] = None  # {region_id: record}
    air_superiority_model_meta: Optional[Dict[str, Any]] = None


@dataclass
class Supply:
    """Live supply PROXY (§13). Where troops are, NOT the exact tooltip number.

    ``by_province`` maps a province id to the proxy supply record
    (``{current, capacity, gain, divisions}``). The exact per-province supply NUMBER
    is a documented wall (DESIGN §B2); this is the validated division-province proxy
    LAYER, badged so the UI shows it as an estimate, never authoritative. Always
    present; null until a live read folds it.
    """
    by_province: Optional[Dict[str, Any]] = None
    source: Optional[str] = None        # "M" when a live read filled it, else null
    confidence: Optional[str] = None    # e.g. "division_province_proxy"


@dataclass
class ModelsBlock:
    """Derived-MODEL sidecar block (§20, from models/*). Provenance = derived.

    NEW first-class container for confidence-badged compute layers that have no
    single save/memory home. ``supply_estimate`` (models/supply_number) is the
    priority-laddered per-province supply estimate: every row self-badges its tier
    (solver/proxy/structural) so a consumer never reads a structural estimate as an
    exact reading. null until the model runs. All D-tier (rows carry true per-tier S/M).
    """
    supply_estimate: Optional[Dict[str, Any]] = None


@dataclass
class StrategicAir:
    """World strategic-air picture (from strategic_air.json, §14). Always present.

    ``air_bases`` is the world list of air-base capacity records (each carries its
    state + the per-country capacity-penalty list). ``indices`` holds the engine's
    air_theatre/air_group running indices. ``per_country_region_blocks`` is the
    per-TAG count of strategic-air region sub-blocks (a coverage indicator).
    null/empty-but-present until the parser feeds it.
    """
    air_bases: List[Dict[str, Any]] = field(default_factory=list)
    indices: Optional[Dict[str, Any]] = None
    per_country_region_blocks: Optional[Dict[str, int]] = None


@dataclass
class StrategicOperatives:
    """World operative pools + intel-network graph (from strategic_operatives.json,
    §15). Always present. ``pools`` maps an operative index (string) to its
    ``{subversive_activity_level, danger_level, networks[]}`` record; each network is
    ``{target, vertices}``. null/empty until the parser feeds it.
    """
    pools: Optional[Dict[str, Any]] = None


@dataclass
class Raids:
    """World strategic-bombing / naval-raid target table (from raids.json, §16).

    ``counters`` = the engine's next_state / target counters; ``targets`` = the raid
    target list (province/building/location); ``countries`` = per-TAG raid priority.
    Always present; empty/null until the parser feeds it.
    """
    counters: Optional[Dict[str, Any]] = None
    targets: List[Dict[str, Any]] = field(default_factory=list)
    countries: List[Dict[str, Any]] = field(default_factory=list)


@dataclass
class WorldEvents:
    """Persisted event ledgers (from events.json, §17). Always present.

    ``fired_events`` = list of scripted-event ids that fired this game (persisted);
    ``pending_events`` = scheduled-but-unfired (empty in most saves; future-proofed);
    ``threat`` = the world-tension CONTRIBUTOR ledger (per threat-action entry, NOT
    the aggregate tension scalar); ``sunk_convoys`` = per-month convoy-loss ledger;
    ``convoy_losses_by_killer`` / ``convoy_losses_by_owner`` = rolled-up tallies.
    Lists default empty; rollup dicts null until measured.
    """
    fired_events: List[str] = field(default_factory=list)
    pending_events: List[Dict[str, Any]] = field(default_factory=list)
    threat: List[Dict[str, Any]] = field(default_factory=list)
    sunk_convoys: List[Dict[str, Any]] = field(default_factory=list)
    convoy_losses_by_killer: Optional[Dict[str, int]] = None
    convoy_losses_by_owner: Optional[Dict[str, int]] = None


@dataclass
class World:
    """World/scenario meta (from world.json, §18). Always present.

    ``scenario`` = start-date/dlcs/mods/game-rules/speed/ideology; ``variables`` =
    global script variables; ``flags`` = global flags (each ``{value, date}``);
    ``tech_sharing_groups`` = research-sharing groups; ``power_balances`` = balance-
    of-power tracks; ``faction_influence`` = per-faction initiative/contribution/
    war-score internals; ``world_tension_inputs`` = the tension-scaling inputs (NOT
    the aggregate tension value, which is not stored); ``saved_event_targets`` =
    named saved scopes. null/empty-but-present until the parser feeds it.
    """
    scenario: Optional[Dict[str, Any]] = None
    variables: Optional[Dict[str, Any]] = None
    flags: Optional[Dict[str, Any]] = None
    tech_sharing_groups: List[Dict[str, Any]] = field(default_factory=list)
    power_balances: List[Dict[str, Any]] = field(default_factory=list)
    faction_influence: List[Dict[str, Any]] = field(default_factory=list)
    world_tension_inputs: Optional[Dict[str, Any]] = None
    saved_event_targets: List[Dict[str, Any]] = field(default_factory=list)


@dataclass
class OrderOfBattle:
    """Top-level world order-of-battle (from oob.json, §12). Always present.

    Command structure is world-scoped (cross-tag expeditionary/lend-lease divisions can
    sit in another tag's army), so the canonical lists live here; each element carries
    its `owner` TAG. `by_country` is the per-tag count rollup (also mirrored onto
    Country.order_of_battle for convenience).
    """
    army_groups: List[Dict[str, Any]] = field(default_factory=list)
    armies: List[Dict[str, Any]] = field(default_factory=list)
    fleets: List[Dict[str, Any]] = field(default_factory=list)
    navy_theaters: List[Dict[str, Any]] = field(default_factory=list)
    air_wings: List[Dict[str, Any]] = field(default_factory=list)
    theatres: List[Dict[str, Any]] = field(default_factory=list)
    by_country: Dict[str, Any] = field(default_factory=_none_dict)


# --- Event (§9) ---
EVENT_TYPES = (
    # territory / control
    "state_owner_change", "state_controller_change", "province_controller_change",
    # country lifecycle (Codex P1 #2, P3 #13)
    "country_created", "country_removed",
    # divisions
    "division_created", "division_destroyed", "division_moved",
    "division_template_changed",
    # politics / focus / research
    "focus_completed", "law_changed", "idea_gained", "idea_lost",
    "government_changed", "tech_researched", "tech_lost", "research_slot_gained",
    # diplomacy
    "war_declared", "peace", "faction_joined", "faction_left", "capitulated",
    "subject_created", "subject_freed", "autonomy_changed",
    # economy / production
    "production_milestone", "factory_milestone",
    # commanders (from characters.json + oob assignment join)
    "commander_assigned",
    # order of battle (from oob.json)
    "army_formed", "army_disbanded",
    # designs (from designs.json)
    "design_created", "mio_leveled",
    # intel / operations (from intel.json)
    "operation_started", "operation_completed",
    # trade / equipment market / lend-lease (from market.json)
    "lend_lease_started", "lend_lease_ended",
    # occupation governance (from state_detail + occupation.json)
    "resistance_changed", "compliance_changed", "occupation_law_changed",
    # Tier-1 EXTRACT save domains
    # events.json
    "event_fired", "convoy_sunk",
    # country_extra.json
    "nuke_count_changed", "ace_gained", "project_completed",
    # combat
    "combat_started", "combat_ended",
    # --- WAVE-2 new events ---
    # abilities.json (commander cooldowns + officer corps + ace lifecycle)
    "ace_lost", "commander_cooldown_started", "commander_freed",
    "general_captured", "general_freed", "officer_spirit_changed",
    # projects.json (special projects / facilities / scientists)
    "project_finished", "project_started", "facility_built", "facility_dismantled",
    "scientist_specialization_up",
    # market_extra.json + deployment.json
    "unit_deployment_started", "unit_deployment_completed",
    "expeditionary_force_sent", "expeditionary_force_recalled",
    "market_subsidy_changed", "market_price_level_changed",
)
CONFIDENCE_LEVELS = ("exact", "interval", "inferred")
EVENT_SOURCES = ("M", "M*", "S", "D", "marker")


@dataclass
class Event:
    type: str = ""
    hour_index: Optional[int] = None
    date: Optional[str] = None
    subject: Optional[str] = None
    object: Optional[str] = None
    key: Optional[str] = None
    from_: Optional[Any] = None   # serialized as "from" (reserved word workaround)
    to: Optional[Any] = None
    source: str = "D"
    confidence: str = "interval"
    detail: Optional[Dict[str, Any]] = None

    def to_dict(self) -> Dict[str, Any]:
        d = asdict(self)
        d["from"] = d.pop("from_")
        return d


def div_uid(div: Dict[str, Any]) -> Optional[str]:
    """Canonical division identity = full Clausewitz id '(id:type)' (Codex P1 #3).

    Using bare div_id alone mis-classifies id-reuse-after-destruction as a move +
    retemplate. Falls back to bare id when div_type is absent.
    """
    did = div.get("div_id")
    if did is None:
        return None
    dtype = div.get("div_type")
    return f"{did}:{dtype}" if dtype is not None else f"{did}"


def make_event(type: str, **kw: Any) -> Dict[str, Any]:
    """Build an Event dict, mapping the 'from' kwarg around the reserved word."""
    frm = kw.pop("from", None)
    if "from_" in kw:
        frm = kw.pop("from_")
    ev = Event(type=type, from_=frm, **kw)
    return ev.to_dict()


# ---------------------------------------------------------------------------
# Empty-skeleton builders (the no-fabrication baseline).
# ---------------------------------------------------------------------------
def empty_country() -> Dict[str, Any]:
    return asdict(Country())


def empty_state() -> Dict[str, Any]:
    return asdict(State())


def empty_province() -> Dict[str, Any]:
    return asdict(Province())


def empty_division() -> Dict[str, Any]:
    return asdict(Division())


def empty_map() -> Dict[str, Any]:
    return asdict(MapLayers())


def empty_designs() -> Dict[str, Any]:
    return asdict(Designs())


def empty_order_of_battle() -> Dict[str, Any]:
    return asdict(OrderOfBattle())


def empty_naval_air() -> Dict[str, Any]:
    return asdict(NavalAir())


def empty_supply() -> Dict[str, Any]:
    return asdict(Supply())


def empty_weather() -> Dict[str, Any]:
    return asdict(Weather())


def empty_models() -> Dict[str, Any]:
    return asdict(ModelsBlock())


def empty_strategic_air() -> Dict[str, Any]:
    return asdict(StrategicAir())


def empty_strategic_operatives() -> Dict[str, Any]:
    return asdict(StrategicOperatives())


def empty_raids() -> Dict[str, Any]:
    return asdict(Raids())


def empty_world_events() -> Dict[str, Any]:
    return asdict(WorldEvents())


def empty_world() -> Dict[str, Any]:
    return asdict(World())


def empty_snapshot(game_id: str, date: Optional[str] = None) -> Dict[str, Any]:
    """A fully-present, all-null SNAPSHOT skeleton (SCHEMA §1)."""
    return {
        "schema_version": SCHEMA_VERSION,
        "game_id": game_id,
        "date": date,
        "hour_index": date_to_hour_index(date),
        "save_version": None,
        "provenance": asdict(Provenance()),
        "countries": {},
        "states": {},
        "provinces": {},
        "divisions": [],
        "combats": [],
        # world-scoped diplomacy (war-id-keyed wars + factions + subjects); the
        # per-country view lives under countries.*.diplomacy. Always present.
        "diplomacy_world": {"wars": {}, "factions": {}, "subjects": []},
        "map": empty_map(),
        # world-scoped design catalogs + order-of-battle (§11, §12). Always present;
        # null-but-present lists/dicts until their parser feeds them.
        "designs": empty_designs(),
        "order_of_battle": empty_order_of_battle(),
        # live PROXY/region sidecars promoted to first-class fields (§13). Always
        # present; null-but-present until a live read folds them. Confidence-badged.
        "naval_air": empty_naval_air(),
        "supply": empty_supply(),
        # live weather domain (§19, M). NEW first-class domain; no save oracle.
        "weather": empty_weather(),
        # derived-MODEL sidecar block (§20, D). Confidence-badged compute layers.
        "models": empty_models(),
        # pairwise intel ledger (§3.13b, observer->target, D). Always present;
        # populated only for meaningful pairs (crypto/leak), never the full NxN matrix.
        "intel_ledger": {},
        # Tier-1 EXTRACT save-side world domains (§14-§18). Always present;
        # null/empty-but-present until their parser feeds them. Save-sourced (S).
        "strategic_air": empty_strategic_air(),
        "strategic_operatives": empty_strategic_operatives(),
        "raids": empty_raids(),
        "world_events": empty_world_events(),
        "world": empty_world(),
        "events": [],
    }


# ---------------------------------------------------------------------------
# Lightweight structural validator (no jsonschema dependency).
# Verifies required containers exist with the right *kind* (dict/list), the
# canonical scalar types, and the cross-field invariants the diff engine relies
# on (hour_index consistency, division.div_id presence, event type/confidence).
# Returns a list of human-readable problem strings; empty == valid.
# ---------------------------------------------------------------------------
_TOP_CONTAINERS = {
    "countries": dict, "states": dict, "provinces": dict,
    "divisions": list, "combats": list, "events": list, "map": dict,
    "provenance": dict, "diplomacy_world": dict,
    "designs": dict, "order_of_battle": dict,
    "naval_air": dict, "supply": dict,
    # Tier-1 EXTRACT world-scoped save domains (§14-§18)
    "strategic_air": dict, "strategic_operatives": dict, "raids": dict,
    "world_events": dict, "world": dict,
    # WAVE-2 first-class domains (§19-§20 + pairwise ledger)
    "weather": dict, "models": dict, "intel_ledger": dict,
}


def validate_snapshot(snap: Dict[str, Any]) -> List[str]:
    problems: List[str] = []

    if not isinstance(snap, dict):
        return ["snapshot is not a dict"]

    if snap.get("schema_version") != SCHEMA_VERSION:
        problems.append(
            f"schema_version {snap.get('schema_version')!r} != {SCHEMA_VERSION!r}"
        )
    if not snap.get("game_id"):
        problems.append("game_id missing/empty")

    # hour_index must agree with date (the canonical diff key).
    date = snap.get("date")
    expect_hi = date_to_hour_index(date)
    if snap.get("hour_index") != expect_hi:
        problems.append(
            f"hour_index {snap.get('hour_index')!r} != date_to_hour_index({date!r})"
            f"={expect_hi!r}"
        )

    # Containers present and of the right kind.
    for key, kind in _TOP_CONTAINERS.items():
        if key not in snap:
            problems.append(f"missing top-level container {key!r}")
        elif not isinstance(snap[key], kind):
            problems.append(f"{key!r} should be {kind.__name__}, got {type(snap[key]).__name__}")

    # Countries: keys look like TAGs; each has the 8 domains as dicts.
    for tag, c in (snap.get("countries") or {}).items():
        if not (isinstance(tag, str) and 2 <= len(tag) <= 4):
            problems.append(f"country key {tag!r} not a TAG-like string")
        if not isinstance(c, dict):
            problems.append(f"country {tag!r} not a dict")
            continue
        for dom in ("economy", "military", "politics", "research", "focus",
                    "decisions", "diplomacy", "intel", "commanders",
                    "mios", "scheduled_variants"):
            if dom not in c:
                problems.append(f"country {tag!r} missing domain {dom!r}")
        # the always-present-but-nullable per-country deep fields must be the right kind
        if "order_of_battle" not in c:
            problems.append(f"country {tag!r} missing field 'order_of_battle'")
        if not isinstance(c.get("mios", []), list):
            problems.append(f"country {tag!r}.mios should be a list")
        st = c.get("politics", {}).get("stability")
        if st is not None and not (isinstance(st, (int, float)) and -0.01 <= st <= 1.01):
            problems.append(f"country {tag!r} stability {st!r} out of 0..1")

    # States: keys parse as ints; owner/controller str|null.
    for sid, stt in (snap.get("states") or {}).items():
        if not _is_intlike(sid):
            problems.append(f"state key {sid!r} not int-like")
        if not isinstance(stt, dict):
            problems.append(f"state {sid!r} not a dict"); continue
        for fld in ("owner", "controller"):
            v = stt.get(fld)
            if v is not None and not isinstance(v, str):
                problems.append(f"state {sid!r}.{fld} {v!r} not str|null")

    # Provinces: keys int-like; controller str|null.
    for pid, prv in (snap.get("provinces") or {}).items():
        if not _is_intlike(pid):
            problems.append(f"province key {pid!r} not int-like")
        if isinstance(prv, dict):
            v = prv.get("controller")
            if v is not None and not isinstance(v, str):
                problems.append(f"province {pid!r}.controller {v!r} not str|null")
        else:
            problems.append(f"province {pid!r} not a dict")

    # Divisions: each needs an int div_id; identity (uid = id:type) must be unique.
    seen_uids = set()
    for i, dv in enumerate(snap.get("divisions") or []):
        if not isinstance(dv, dict):
            problems.append(f"division[{i}] not a dict"); continue
        did = dv.get("div_id")
        if not isinstance(did, int):
            problems.append(f"division[{i}].div_id {did!r} not int")
            continue
        uid = div_uid(dv)
        if uid in seen_uids:
            problems.append(f"duplicate division uid {uid!r}")
        else:
            seen_uids.add(uid)

    # Designs: division_templates is a list, equipment_variants a dict — both must be
    # present (always-present-container rule), §11.
    dsg = snap.get("designs")
    if isinstance(dsg, dict):
        if not isinstance(dsg.get("division_templates"), list):
            problems.append("designs.division_templates missing/not a list")
        if not isinstance(dsg.get("equipment_variants"), dict):
            problems.append("designs.equipment_variants missing/not a dict")

    # Order of battle: the canonical sub-lists are lists, by_country a dict — all
    # present (always-present-container rule), §12.
    oob = snap.get("order_of_battle")
    if isinstance(oob, dict):
        for k in ("army_groups", "armies", "fleets", "navy_theaters",
                  "air_wings", "theatres"):
            if not isinstance(oob.get(k), list):
                problems.append(f"order_of_battle.{k} missing/not a list")
        if not isinstance(oob.get("by_country"), dict):
            problems.append("order_of_battle.by_country missing/not a dict")

    # Events: type/confidence/source from the enums.
    for i, ev in enumerate(snap.get("events") or []):
        problems.extend(_validate_event(ev, f"events[{i}]"))

    return problems


def _validate_event(ev: Any, where: str) -> List[str]:
    out: List[str] = []
    if not isinstance(ev, dict):
        return [f"{where} not a dict"]
    if ev.get("type") not in EVENT_TYPES:
        out.append(f"{where}.type {ev.get('type')!r} not a known event type")
    if ev.get("confidence") not in CONFIDENCE_LEVELS:
        out.append(f"{where}.confidence {ev.get('confidence')!r} invalid")
    if ev.get("source") not in EVENT_SOURCES:
        out.append(f"{where}.source {ev.get('source')!r} invalid")
    return out


def validate_event(ev: Dict[str, Any]) -> List[str]:
    """Public single-event validator (used by the diff engine's tests)."""
    return _validate_event(ev, "event")


def _is_intlike(x: Any) -> bool:
    if isinstance(x, int):
        return True
    if isinstance(x, str):
        try:
            int(x)
            return True
        except ValueError:
            return False
    return False


__all__ = [
    "SCHEMA_VERSION", "DOMAINS", "RESOURCE_KEYS", "BUILDING_KEYS",
    "FIELD_SOURCES", "source_of", "date_to_hour_index",
    "Economy", "Military", "Politics", "Research", "Focus", "Decisions",
    "Diplomacy", "Intel", "Scalars", "Commanders", "Country", "State", "Province",
    "Division", "Combat", "MapLayers", "Designs", "OrderOfBattle",
    "NavalAir", "Supply", "Weather", "WeatherRegion", "ModelsBlock",
    "StrategicAir", "StrategicOperatives", "Raids", "WorldEvents", "World",
    "Provenance", "Event", "make_event",
    "EVENT_TYPES", "CONFIDENCE_LEVELS", "EVENT_SOURCES",
    "empty_country", "empty_state", "empty_province", "empty_division",
    "empty_map", "empty_designs", "empty_order_of_battle",
    "empty_naval_air", "empty_supply", "empty_weather", "empty_models",
    "empty_snapshot",
    "empty_strategic_air", "empty_strategic_operatives", "empty_raids",
    "empty_world_events", "empty_world",
    "validate_snapshot", "validate_event", "div_uid",
]
