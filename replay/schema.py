"""Canonical replay snapshot schema (Python mirror of replay/SCHEMA.md, v1).

Pure stdlib. No game, no network, no I/O on import. Defines:

  * the SCHEMA_VERSION + canonical domain ids,
  * dataclasses for every layer of a SNAPSHOT (§1-§8 of SCHEMA.md),
  * the EVENT record + the event-type enum (§9),
  * FIELD_SOURCES: dotted-path -> source tag (C/M/M*/S/D), the machine-readable
    mirror of the per-field source tables,
  * helpers: date_to_hour_index, empty_snapshot / empty_country / ...,
  * validate_snapshot(): a lightweight structural validator (no jsonschema dep).

Design rule (DESIGN no-fabrication): every container is always present; an
unobtained scalar is ``None`` (JSON null), never guessed. ``0`` means measured zero.
"""
from __future__ import annotations

import math
import re
from dataclasses import dataclass, field, asdict, fields as dataclass_fields
from typing import Any, Dict, List, Optional

SCHEMA_VERSION = "1"
PRODUCTION_VARIANT_ID_MAX = 10_000_000
PRODUCTION_LINE_ID_MAX = 10_000_000
PRODUCTION_FACTORY_COUNT_MAX = 100_000
PRODUCTION_LINE_AMOUNT_MAX = 2_147_483_647
PRODUCTION_FIXED_POINT_ABS_MAX = 1_000_000_000_000.0
FUEL_MEASURE_ABS_MAX = 1_000_000_000.0
DECISION_STATUS_MAX_KEYS = 32
DECISION_STATUS_COUNT_MAX = 4096
DECISION_TIMED_LIVE_MAX_ENTRIES = 4096
DECISION_TIMED_DAYS_MAX = 100_000
DECISION_TIMED_STATE_MAX = 1024
SCRIPT_TOKEN_MAX_LENGTH = 160

# Canonical domain ids referenced by provenance.{collector,memory,save}.domains (SCHEMA §2).
DOMAINS = (
    # collector/runtime session domains
    "clock",              # trusted live date/hour emitted by the in-process collector
    "country_identity",   # live country tag/identity rows emitted by the collector
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
    "projects_live",     # LIVE special-projects/facilities/scientists pools (C/M; distinct
                         # from the SAVE-sourced per-country "projects" domain below)
    "fuel_live",         # LIVE per-country national fuel -> countries.*.economy.fuel.{current,
                         # max,gain,cost} (C/M; shares the save economy.fuel home, tracked under
                         # a DISTINCT live id). Portable Ghidra pointer chain; max_fuel 71/71 EXACT.
    "supply_inputs",     # LIVE supply solver INPUTS (per-province efficiency/draw) -> badged
                         # signals on snap["supply"] (M; NOT the exact per-province supply,
                         # which is provably not stored — distinct from the "supply" proxy).
    # LIVE provenance ids the assembler emits for per-entity live domains (schema
    # review #7: these are appended to provenance.memory.domains, so they must be
    # canonical here too):
    "country_misc",          # convoys_destroyed/accidents/ace_count/cosmetic_tag (M)
    "mio",                   # LIVE MIO pool -> top-level mio_live (M)
    "ships",                 # LIVE per-ship OOB/CShip telemetry -> order_of_battle.ships (C/M);
                             # raw collector deltas use stable ship:<instance_id> keys and carry
                             # CShip strength/org/experience/category/sunk-convoy stats.
    # LIVE per-country domains that fold into the SAME canonical save HOMES
    # (research/commanders/intel) but are tracked under DISTINCT *_live provenance
    # ids so the live memory channel never collides with the save channel's
    # canonical "tech"/"commanders"/"intel" ids (schema: distinct-live-id strategy).
    "tech_live",             # LIVE research counts/slots -> countries.*.research (C/M);
                             # DLL collector emits raw active_research rows from the bounded
                             # CTechnologyStatus+0xA0 CResearchSlot* vector on scheduled frames.
    "commanders_live",       # LIVE unit-leaders -> countries.*.commanders.{army,navy}_leaders (M)
    "intel_live",            # LIVE agency scalars -> countries.*.intel.agency.* (M)
    "focus_live",            # LIVE national-focus state -> countries.*.focus.* (M; shares the
                             # save "focus" home, tracked under a DISTINCT live id)
    "production_live",       # LIVE stockpile + production lines -> countries.*.production
                             # (C/M; a NEW live-only home, distinct from the save "production"
                             # domain which folds into economy/military)
    "diplomacy_live",        # LIVE wars/at-war/casualties/faction-group -> countries.*.diplomacy
                             # (at_war/casualties M, faction_group C/M) + a top-level live sidecar
                             # snap["diplomacy_live"] (war_relations M + faction_members C/M).
                             # Distinct from the SAVE "diplomacy" id (which folds diplomacy.wars +
                             # diplomacy_world).
    "diplo_relations",       # LIVE bilateral RELATIONS (diplo_relations_live, M) -> countries.*.
                             # diplomacy.{opinion (live-first onto the save-shared leaf),
                             # relation_flags, overlord, autonomy_state, autonomy_ratio}.
                             # Distinct from the diplomacy_live id (wars/factions) -- this is the
                             # opinion/flags/subjects matrix the save's opinion leaf mirrors.
    "leader_detail",         # LIVE unit-leader SKILL + assigned-TRAIT set (leader_skill_traits_
                             # live, M) -- an OPT-IN enrichment that joins skill/traits onto the
                             # commanders_live army/navy leader records by unit_leader_id. NO new
                             # home; it adds `skill`/`traits` keys to existing leader dicts.
    "politics_live",         # LIVE per-country politics -> countries.*.politics (party_popularities
                             # /ruling_party/ideas/idea_count/election_frequency/party_count/
                             # low_confidence, M; shares the save politics home, tracked under a
                             # DISTINCT live id). political_power stays country_scalars-owned.
    "oob_command",           # LIVE OOB command hierarchy -> order_of_battle.{armies,army_groups,
                             # fleets,theatres} + membership/leader/owner (M; shares the save
                             # "order_of_battle" home, tracked under a DISTINCT live id). The
                             # raw DLL stream also emits fleet/task-force/ship membership deltas.
                             # The per-SHIP order_of_battle.ships leaf is owned by the "ships" id.
    # --- LIVE EXTRACT-residual domains wired from the deep-crack push (all M; each shares a
    # save home under a DISTINCT live id OR a NEW live-only home/leaf). ---
    "world_meta",            # LIVE scenario dates (world_extract_live.read_scenario_meta) ->
                             # world.scenario.{current_date_live,start_date_live} (M; the save
                             # "world" domain owns scenario, so these are DISTINCT _live keys).
                             # world_tension is COMPUTED-not-stored -> NEVER folded.
    "construction_orders",   # LIVE construction queue + battle-plan orders + opinion-modifier
                             # decomposition (construction_orders_live, M) -> countries.*.
                             # production.construction_queue (NEW live-only leaf) + order_of_
                             # battle.battle_plan_orders (NEW live-only list) + countries.*.
                             # diplomacy.opinion_modifiers (NEW live-only leaf). Order/modifier
                             # target geometry + token NAMES stay RESISTED (null), never invented.
    "army_orders",           # Raw in-process collector battle-plan order stream. Emits
                             # save-free per-army order deltas/events before later folding into
                             # order_of_battle.battle_plan_orders.
    "decisions_market",      # LIVE active timed DECISIONS + global equipment-MARKET contracts
                             # (decisions_market_live, M) -> countries.*.decisions.timed_live
                             # (NEW live-only leaf) + the NEW top-level snap["market_live"]
                             # sidecar (distinct from the SAVE "market" domain).
    "decisions_live",        # scheduled in-process collector decision-status/timed-decision
                             # rows -> countries.*.decisions.{status_present,timed_live}
                             # (C; distinct from the market sidecar domain).
    "operative_xp",          # LIVE operative SKILL + XP + tech in-progress UNITS (final_residuals_
                             # live, M) -> countries.*.intel.operative_skill_xp (NEW live-only leaf)
                             # + merged onto research.in_progress_live slots (progress_units). The
                             # tech-completion FRACTION + operations detail stay RESISTED (null).
    "politics_residuals",    # LIVE election residuals + appointed advisors (politics_residuals_
                             # live, M) -> countries.*.politics.{last_election,elections_allowed,
                             # appointed_advisors,appointed_advisor_details,
                             # appointed_advisor_slots,appointed_advisor_ledgers}
                             # (NEW live-only leaves).
    "intel_deep",            # LIVE operative roster + cryptology (intel_deep_live, M) ->
                             # countries.*.intel.{operatives_live,cryptology_live} (NEW live-only
                             # leaves, distinct from the save-shaped intel.operatives).
    "buildings_live",        # LIVE per-state + per-province BUILDING levels (buildings_live, M) ->
                             # states.*.buildings + provinces.*.buildings (M; shares the save home,
                             # tracked under a DISTINCT live id).
    "designs_variants",      # LIVE equipment-VARIANT module catalog (designs_mio_deep_live, M) ->
                             # designs.equipment_variants_live (M; a DISTINCT live-only leaf, kept
                             # separate from the save "id:type"-keyed equipment_variants catalog).
    "division_equipment",    # LIVE per-division held-equipment inventory (division_equipment_live,
                             # M) -> divisions[].equipment_live (M; a DISTINCT live-only leaf, kept
                             # separate from the save-owned equipment list -> no clobber).
    "land_combats",          # SOLVED land-combat reader -> combats[] kind=land (M)
    "naval_air_combats",     # naval/air combat detector sidecar (M)
    "naval_combat_members",  # Raw active-naval-combat CFEXMember rows: ship/convoy
                             # identity, side membership, designed max strength, and
                             # raw engagement state. Pointer-free; rooted from the
                             # live CNavalCombat bridge, not save OOB.
    "naval_air_combat_stats",  # per-side naval combat stat rows (M/M*)
    "air_wings",             # Raw in-process collector wing telemetry stream. Emits save-free
                             # per-wing count/mission/effective-plane deltas/events before folding
                             # into order_of_battle.air_wings / air-map inputs.
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

# Replay-owned collector-domain contracts. These are schema/validation metadata,
# not native offsets or live adapter enablement.
COLLECTOR_DOMAIN_CONTRACTS: Dict[str, Dict[str, Any]] = {
    "divisions": {
        "cadence_class": "scheduled_hourly_snapshot",
        "snapshot_path": "divisions[]",
        "shape": "active_division_identity_status_rows",
        "source_marker": (
            "C only when provenance.collector.domains includes divisions after "
            "a successful fold; no save fallback may fill collector gaps"
        ),
        "event_shape": (
            "snapshot diffs emit division_created/division_destroyed/"
            "division_moved/division_template_changed at snapshot cadence; "
            "sub-hour movement stays in a later raw delta/event stream"
        ),
        "required_fields": ("div_id", "div_type"),
        "first_slice_fields": (
            "div_id", "div_type", "owner", "location_province", "state_id",
            "template_id", "template_type", "template_name", "organisation",
            "strength", "experience", "moving", "move_target_province",
            "move_progress", "in_combat", "supply_status",
        ),
    },
    "land_combats": {
        "cadence_class": "high_frequency_raw_stream",
        "snapshot_path": None,
        "shape": "raw_land_combat_lifecycle_and_participant_samples",
        "root_contract_status": "pinned_live_raw_stream",
        "source_marker": (
            "C only in collector raw domain_delta/event/gap frames; do not "
            "promote to typed combats[] snapshots until replay ingestion for "
            "the lifecycle stream is implemented"
        ),
        "required_raw_fields": (
            "combat_key", "combat_id", "generation", "location",
            "attacker_divisions", "defender_divisions",
        ),
        "nullable_side_fields": (
            "attacker_divisions", "defender_divisions",
        ),
        "identity_rule": "land_combat:<combat_id>:<generation>",
    },
}

def valid_tag(v: Any) -> bool:
    """True iff ``v`` is a real the game country TAG: exactly 3 chars, uppercase
    alphanumeric, not all-digits (GER/ENG/SOV/D01 pass; '---', '?<id>', '42',
    '123', lowercase, and any non-TAG string fail). The single source of truth
    for "a 3-letter TAG or null" so a raw numeric id can never masquerade as
    identity in an owner/controller/*_tag field (schema review #5)."""
    return (isinstance(v, str) and len(v) == 3 and v.isalnum()
            and v.isupper() and not v.isdigit())


def valid_script_token(v: Any) -> bool:
    return (
        isinstance(v, str)
        and 0 < len(v) <= SCRIPT_TOKEN_MAX_LENGTH
        and all(
            ("A" <= ch <= "Z")
            or ("a" <= ch <= "z")
            or ("0" <= ch <= "9")
            or ch in "_-.:"
            for ch in v
        )
    )


# ---------------------------------------------------------------------------
# Source-tag map: dotted field path -> source tag. Mirrors SCHEMA.md tables.
# Paths use "*" for the per-entity key (country TAG / state id / list element).
# Looked up by source_of() with prefix-fallback so list-element subfields resolve.
# ---------------------------------------------------------------------------
C, M, MS, S, D = "C", "M", "M*", "S", "D"  # M* spelled "M*" in values below

FIELD_SOURCES: Dict[str, str] = {
    # --- snapshot scalars ---
    "date": S,
    "hour_index": D,
    "save_version": S,
    # --- provenance.mod (§2.1, additive MOD IDENTITY). Read from the host LAUNCHER
    # config on disk at capture time (dlc_load.json + .mod descriptors) -- a disk
    # oracle, not memory and not the melted save; tagged S (on-disk source). None
    # when no launcher config was found (no fabrication). ---
    "provenance.mod": "S",
    "provenance.collector": "C",
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
    # fuel is LIVE-first via scheduled collector frames (fuel_live, C/M) with the save
    # as the backstop: the collector reads the canonical i64 fixed-point store from the
    # validated CCountry object (CCountry+0x1580 fuel_status; /32768). Worker polls stay
    # light and must not carry fuel payloads. gain/cost are collector-only in the current
    # parser wiring.
    "countries.*.economy.fuel.current": "C/M/S",
    "countries.*.economy.fuel.max": "C/M/S",
    "countries.*.economy.fuel.gain": "C",
    "countries.*.economy.fuel.cost": "C",
    "countries.*.economy.gross_ic_by_equipment": "S/D",
    "countries.*.economy.market_stockpile": "S",
    "countries.*.economy.market_automation": "S",
    "countries.*.economy.logistics": "S",            # supply.json (§3.1)
    # --- country extras + player name (Tier-1 EXTRACT, §3.11/§3.12) ---
    "countries.*.extra": "S",                        # country_extra.json
    "countries.*.extra.script_flags": "S",            # country_extra flags bag
    "countries.*.extra.strategic_navy": "S",          # country_extra strategic_navy block
    "countries.*.human_name": "S",                   # players.json (persona name)
    # --- country.military (§3.2) ---
    "countries.*.military.num_ships": "M/S",
    "countries.*.military.division_count": "M/D",
    "countries.*.military.manpower.available": "S/M*",
    "countries.*.military.manpower.deployed": "S/D",
    "countries.*.military.xp.army": "S",
    "countries.*.military.equipment_stockpile": "S",
    "countries.*.military.convoys": "S",
    "countries.*.military.convoys_count": "M",       # live country-obj stockpile count
    "countries.*.military.ships_by_hull": "S/D",
    "countries.*.military.casualties.killed": "S/D",
    "countries.*.military.casualties.captured": "S/D",
    "countries.*.military.command_power": "M",
    # --- country.politics (§3.3) ---
    "countries.*.politics.stability": "M",
    "countries.*.politics.war_support": "M",
    "countries.*.politics.political_power": "M",
    "countries.*.politics.command_power": "M",
    # ruling_party + party_popularities are now LIVE-first (politics_live, M) with the save
    # as the backstop: the live reader measures the ruling ideology-group id (46/46 EXACT)
    # and the 4-ideology popularity vector (46/46 EXACT vs save). political_power stays
    # country_scalars-owned (M) — politics_live does NOT re-fold it (single live owner).
    "countries.*.politics.ruling_party": "M/S",
    "countries.*.politics.party_popularities": "M/S",
    "countries.*.politics.laws.economy": "S",
    "countries.*.politics.laws.trade": "S",
    "countries.*.politics.laws.conscription": "S",
    "countries.*.politics.ideas_all": "S",
    "countries.*.politics.ideas_other": "M/S",
    "countries.*.politics.national_spirits": "M/S",
    "countries.*.politics.idea_groups": "C/M",
    "countries.*.politics.advisor_ideas": "C/M",
    # LIVE active-ideas list + cadence (politics_live, M). `ideas` is the ordered full
    # active-ideas/spirits set (incl. law ideas by id; set AND order EXACT vs save);
    # ideas_other is the ordered broad active non-law subset when live classification is
    # available; national_spirits is the narrower `common/ideas` category=country subset.
    # idea_groups is the complete active-token grouping by `common/ideas` group;
    # advisor_ideas is the cabinet/designer/advisor subset keyed by group.
    # election_frequency (months, EXACT) + party_count + low_confidence are live-only
    # read-quality/cadence leaves.
    "countries.*.politics.ideas": "M",
    "countries.*.politics.idea_count": "M",
    "countries.*.politics.election_frequency": "M",
    "countries.*.politics.party_count": "M",
    "countries.*.politics.low_confidence": "M",
    # LIVE typed law triple (politics_live + defnames_live, M). DERIVED from the MEASURED idea
    # tokens by classifying each law idea into its group (defnames idea_group_map). Keyed by the
    # defnames law-group names (economy / trade_laws / mobilization_laws); each value is the idea
    # token. DISTINCT from the save-derived laws.{economy,trade,conscription} above (different
    # group naming + source). Absent for a country whose law group could not be classified
    # (no fabrication). Only present when name resolution is enabled.
    "countries.*.politics.laws": "M/S",
    # LIVE election residuals + appointed advisors (politics_residuals_live, M). DISTINCT
    # live-only leaves: last_election (decoded "Y.M.D.H", 46/46 EXACT vs save), elections_allowed
    # (u8 bool, 46/46 EXACT), appointed_advisors (the idea-token list of HIRED cabinet advisors,
    # 46/46 EXACT), plus static-contract detail/slot/ledger maps. All M; absent until a
    # live read folds them (no fabrication).
    "countries.*.politics.last_election": "M",
    "countries.*.politics.elections_allowed": "M",
    "countries.*.politics.appointed_advisors": "M",
    "countries.*.politics.appointed_advisor_details": "M",
    "countries.*.politics.appointed_advisor_slots": "M",
    "countries.*.politics.appointed_advisor_ledgers": "M",
    "countries.*.politics.cabinet": "S",
    "countries.*.politics.is_human": "S/M",
    "countries.*.politics.is_major": "S",
    "countries.*.politics.capital_state": "S",
    "countries.*.politics.autonomy": "S",
    "countries.*.politics.faction": "S",
    # --- country.research (§3.4) ---
    # tech_count + research_slots + active-research rows are now LIVE-first
    # (tech_live, C/M) with the save as the backstop. The collector reads the
    # CTechnologyStatus completed-tech vector size, slot vector size, and bounded
    # CResearchSlot* vector through CCountry+0xF60 on scheduled frames. Rows carry
    # raw handles, progress_raw, progress_units, and optionally tech_name when a
    # validated tech_def_filter is supplied.
    "countries.*.research.tech_count": "C/M/S",
    "countries.*.research.technologies": "S/D",
    "countries.*.research.in_progress": "S/M*",
    # Save-backed doctrine manager fold (parse_doctrine -> doctrine.json, D/S): TAG-keyed
    # rows are folded additively here. Numeric/non-TAG save keys stay in the top-level
    # doctrine_save sidecar because there is no safe country bridge.
    "countries.*.research.doctrine": "S/D",
    # Derived save active-research progress/ETA (models.research_progress, D): computed by
    # joining in_progress[].name to the active technologies[] row whose date is 1.1.1.1 and
    # then to static game_defs research_cost. Absent/reasoned when the active progress row,
    # static cost, current-year context, or positive points_factor is unavailable.
    "countries.*.research.in_progress.*.progress_points": "D",
    "countries.*.research.in_progress.*.required_points_base": "D",
    "countries.*.research.in_progress.*.remaining_points_base": "D",
    "countries.*.research.in_progress.*.progress_pct": "D",
    "countries.*.research.in_progress.*.eta_days": "D",
    "countries.*.research.in_progress.*.progress_basis": "D",
    "countries.*.research.in_progress.*.progress_unavailable_reason": "D",
    "countries.*.research.in_progress.*.eta_unavailable_reason": "D",
    # LIVE active-research rows live in a SEPARATE leaf (raw-handle shape, distinct
    # from the save's {name, points_factor} in_progress) so the two shapes never mix.
    # Carries {slot_index, tech_handle_raw, tech_def_index_raw, progress_raw,
    # progress_units, tech_name?};
    # scheduled DLL frames now emit the raw rows from the bounded CResearchSlot vector,
    # while legacy memory remains a fallback. Completion fraction/ETA remain derived
    # until the static cost and live speed inputs are safe.
    "countries.*.research.in_progress_live": "C/M",
    # resolved tech TOKEN per in-progress slot. The scheduled DLL emits this only when
    # tech_def_filter supplies a validated live tech-definition handle map; absent for
    # an unknown handle (no fabrication).
    "countries.*.research.in_progress_live.*.tech_name": "C/M",
    # accumulated research UNITS per in-progress slot. The scheduled DLL now emits
    # progress_units directly from the validated CResearchSlot row.
    # = progress_raw/1000 = accumulated research-points (NOT a completion fraction; the fraction
    # needs the per-tech cost def + has no save GT this frame -> RESISTED, never fabricated).
    # ADDED ALONGSIDE the raw progress_raw on each slot dict (merged by slot_index).
    "countries.*.research.in_progress_live.*.progress_units": "C/M",
    # Derived active-research progress/ETA (models.research_progress, D): computed only when a
    # live slot has progress_units + resolved tech_name, generated static tech defs provide a
    # positive research_cost, and the current year/bonus context is safe. Otherwise the value
    # stays absent and an *_unavailable_reason explains the blocker.
    "countries.*.research.in_progress_live.*.required_points_base": "D",
    "countries.*.research.in_progress_live.*.remaining_points_base": "D",
    "countries.*.research.in_progress_live.*.progress_pct": "D",
    "countries.*.research.in_progress_live.*.eta_days": "D",
    "countries.*.research.in_progress_live.*.progress_basis": "D",
    "countries.*.research.in_progress_live.*.progress_unavailable_reason": "D",
    "countries.*.research.in_progress_live.*.eta_unavailable_reason": "D",
    "countries.*.research.research_slots": "C/M/S",
    "countries.*.research.original_research_slots": "S",
    "countries.*.research.research_bonuses": "S",
    # --- country.focus (§3.5) ---
    # focus_tree / completed / completed_count / current / progress are LIVE-first via
    # scheduled collector focus_live (C) and legacy memory focus_live (M), with the save
    # as the backstop. The collector reads focus_tree from CCountry+0x1370 and the focus
    # submanager from CCountry+0x1380. current_continuous is a live-only leaf. paused
    # stays save-only until a live byte is pinned.
    "countries.*.focus.focus_tree": "C/M/S",
    "countries.*.focus.continuous_focus_palette": "S",
    "countries.*.focus.completed": "C/M/S",
    "countries.*.focus.completed_count": "C/M/S",
    "countries.*.focus.current": "C/M/S",
    "countries.*.focus.current_continuous": "C/M",  # active CONTINUOUS focus (live-only leaf)
    "countries.*.focus.progress": "C/M/S",          # days into the active focus (same unit as save)
    "countries.*.focus.paused": "S",
    # --- country.decisions / diplomacy / intel (§3.6-3.8) ---
    "countries.*.decisions": "S",
    # LIVE decision status/timed decisions (decisions_live, C; decisions_market_live, M).
    # DISTINCT live-only leaf from the save-shaped decisions.{active,timed,missions}:
    # each timed row is {decision, days, state, state_name}.
    "countries.*.decisions.timed_live": "C/M",
    "countries.*.diplomacy.wars": "S",
    # opinion is now LIVE-first (diplo_relations_live, M) with the save as the backstop: the
    # live reader measures the bilateral cached_sum net opinion (1113/1113 real ordered pairs
    # EXACT vs save) as a {TGT: value} map, the SAME shape as the save leaf.
    "countries.*.diplomacy.opinion": "M/S",
    # LIVE bilateral relation FLAGS (diplo_relations_live, M): {TGT: [flag,...]} of the 12
    # directed relation-flag types (NAP/military_access/embargo/.../puppet) this country has
    # toward each target. DISTINCT live-only leaf (no save-shaped home).
    "countries.*.diplomacy.relation_flags": "M",
    # LIVE subject/autonomy (diplo_relations_live, M): when THIS country is a subject, its
    # overlord TAG + autonomy-state name + ratio (read from the CSubjectRelation sub-object,
    # 17/17 EXACT vs save). DISTINCT live-only leaves (the save models this under politics.
    # autonomy / diplomacy_world.subjects -- a different shape, so no collision).
    "countries.*.diplomacy.overlord": "M",
    "countries.*.diplomacy.autonomy_state": "M",
    "countries.*.diplomacy.autonomy_ratio": "M",
    # LIVE opinion-modifier decomposition (construction_orders_live, M). DISTINCT live-only
    # leaf {TGT: [{token_id, value}, ...]} decomposing the cached_sum net opinion above; the
    # modifier-token id->NAME registry is RESISTED, so only the raw token_id + value are carried.
    "countries.*.diplomacy.opinion_modifiers": "M",
    # LIVE per-country diplomacy (diplomacy_live, C/M). DISTINCT live-only leaves from the
    # save-derived `wars` above: `at_war` (live enemy-TAG list from the global war_relation
    # pool, at-war set 30/30 EXACT vs save), `casualties` (cumulative war casualties, per-
    # country total 30/30 EXACT), `faction_group` (OPAQUE engine diplomatic-group id, NOT
    # a TAG; DLL reads the per-country group id directly from CCountry+0x598).
    "countries.*.diplomacy.at_war": "M",
    "countries.*.diplomacy.casualties": "M",
    "countries.*.diplomacy.faction_group": "C/M",
    # equipment market / lend-lease (parse_market). Trade contracts attached to
    # both parties under diplomacy.lend_lease; canonical world list in §7b.
    "countries.*.diplomacy.lend_lease": "S",
    "countries.*.diplomacy.expeditionary": "S",
    "countries.*.diplomacy.volunteers": "S",
    "countries.*.diplomacy.attaches": "S",
    # --- country.intel (§3.8, deepened from intel.json) ---
    "countries.*.intel.agency": "S",
    # LIVE-measured agency scalars (intel_live, M): folded onto the SAME nested save
    # shape (name/icon/upgrade_progress + capacity.{max_operatives,usable_slots,
    # defense}) so live wins on the measured scalars while save-only sub-keys
    # (is_created/current_upgrade/upgrades/operatives capacity extras) are preserved.
    "countries.*.intel.agency.name": "M/S",
    "countries.*.intel.agency.icon": "M/S",
    "countries.*.intel.agency.upgrade_progress": "M/S",
    "countries.*.intel.agency.capacity.max_operatives": "M/S",
    "countries.*.intel.agency.capacity.usable_slots": "M/S",
    "countries.*.intel.agency.capacity.defense": "M/S",
    "countries.*.intel.operatives": "S",
    # LIVE operative roster (intel_deep_live, M): the agency's active-owned operatives, each
    # {op_id, name, detected}. DISTINCT live-only leaf from the save-shaped intel.operatives.
    "countries.*.intel.operatives_live": "M",
    # LIVE cryptology (intel_deep_live, M): {TGT: {amount, decrypting}} decryption progress
    # toward each target's cipher. DISTINCT live-only leaf.
    "countries.*.intel.cryptology_live": "M",
    # LIVE operative SKILL + XP (final_residuals_live, M): {op_id: {name, skill, experience}}.
    # DISTINCT live-only leaf; the operative body base is L=ident-0x8 (corrected).
    "countries.*.intel.operative_skill_xp": "M",
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
    # LIVE unit-leaders (commanders_live, M): the live reader CANNOT split generals
    # vs field-marshals (the role split needs the RESISTED overall level), so it
    # folds into DISTINCT live leaves rather than clobbering the role-split save
    # leaves above. Each leader carries char_id/name/skills/experience/trait_xp +
    # country TAG; overall LEVEL + trait NAMES stay RESISTED (null/absent).
    "countries.*.commanders.army_leaders": "M",     # generals + field marshals (unsplit)
    "countries.*.commanders.navy_leaders": "M",     # admirals
    # OPT-IN leader_detail enrichment (leader_skill_traits_live, M): overall SKILL/level + the
    # assigned-TRAIT token set joined onto each commanders_live leader record by unit_leader_id
    # (953/953 EXACT vs save). ADDED ALONGSIDE the existing leader fields; absent when the
    # enrichment is off or a leader was not measured (overall level was otherwise RESISTED).
    "countries.*.commanders.army_leaders.*.skill": "M",
    "countries.*.commanders.army_leaders.*.traits": "M",
    "countries.*.commanders.navy_leaders.*.skill": "M",
    "countries.*.commanders.navy_leaders.*.traits": "M",
    # live count sub-leaves only (the save's richer counts dict is preserved; we
    # write just these measured sub-keys so save counts are never erased — schema).
    "countries.*.commanders.counts.army_leaders": "M/S",
    "countries.*.commanders.counts.navy_leaders": "M/S",
    "countries.*.commanders.counts.characters": "M/S",
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
    # --- country.misc (§3.17, live country_misc_live.py). All M. ---
    "countries.*.misc.convoys_destroyed": "M",
    "countries.*.misc.accidents_score": "M",
    "countries.*.misc.ace_count": "M",
    "countries.*.misc.cosmetic_tag": "M",
    # --- country.identity (live country_static_live.py). Per-country static scalars
    # cracked + validated 140/140 vs the content-matched melt; all M. original_tag is
    # a resolved TAG (None when absent); reserved_dynamic_country / templates_locked are
    # bools; original_capital is a state id (None when 0/none). ---
    "countries.*.identity.tag_db_id": "C",
    "countries.*.identity.original_capital": "M",
    "countries.*.identity.original_tag": "M",
    "countries.*.identity.reserved_dynamic_country": "M",
    "countries.*.identity.templates_locked": "M",
    # country.identity container fields (live country_containers_live). cores/claims are
    # inverted from the per-state vectors (63/63 + 9/9 EXACT); naval_headquarter_status is
    # a customizable-building child count (5-country set EXACT). All M.
    "countries.*.identity.cores": "M",
    "countries.*.identity.claims": "M",
    "countries.*.identity.naval_headquarter_status": "M",
    # country.decisions.status_present (live country_containers_live / collector decisions_live):
    # which decision-status keys are non-empty + their counts. C/M (presence only).
    "countries.*.decisions.status_present": "C/M",
    # country.country_reports (live country_containers_live): latest-report scalar header
    # {days, index, date_raw} (140/140 EXACT). M.
    "countries.*.country_reports": "M",
    # --- country.script_variables (live variables_live.py). The per-country script-
    # variable bag {name: value} (fixed-point /1e5; array vars are name^N keys). M; the
    # mass closure for the nation scripted-GUI overlays. A measured-empty {} is a real
    # read of the located, empty container. ---
    "countries.*.script_variables": "M",
    # --- country.production (§3.18, live production_live.py). LIVE-only home, distinct
    # from the save "production" domain (which folds into economy/military). The live
    # reader measures the equipment STOCKPILE bank ({variant_id: amount}, 459/461 ids
    # EXACT fixed-point) + the military/naval PRODUCTION LINES ({id, active_factories,
    # cost, speed, produced, amount}, line count + SUM(active) EXACT) + the active-
    # factory totals. Collector DLL and legacy memory paths can both measure this;
    # stockpile_total is summed FROM the measured stockpile -> derived-from-live (C/M/D). ---
    "countries.*.production.stockpile": "C/M",
    "countries.*.production.stockpile_total": "C/M/D",   # summed from measured stockpile
    "countries.*.production.military_lines": "C/M",
    "countries.*.production.naval_lines": "C/M",
    "countries.*.production.military_factories": "C/M",
    "countries.*.production.naval_factories": "C/M",
    # LIVE construction queue (construction_orders_live, M). DISTINCT live-only leaf: the
    # per-country civil/military BUILDING queue in priority order, each {line_id, template_id,
    # active_factories, cost, produced, priority}. The build target STATE/location is RESISTED
    # (province handle behind the template def; needs a province->state map) -> never fabricated.
    "countries.*.production.construction_queue": "M",
    # --- top-level designs (§11, from designs.json + live designs_live.py) ---
    # division_templates is now LIVE-first (designs_live, M) with the save as the
    # backstop; equipment_variants remains save-only until the live variant crack
    # is exposed.
    "designs.division_templates": "M/S",
    # the save equipment_variants catalog stays save-owned (keyed "id:type"); the LIVE variant
    # module catalog (designs_mio_deep_live) lands in the DISTINCT equipment_variants_live leaf
    # (keyed by the bare live variant id) so the two key spaces never collide (schema review #3).
    "designs.equipment_variants": "S",
    "designs.equipment_variants_live": "M",
    # derived design stats (WAVE-2; models/division_stats + models/equipment_stats).
    # Single prefix entry covers every nested stat (source_of longest-prefix match).
    "designs.template_stats": "D",            # {template_id: DivisionStats}
    "designs.equipment_variant_stats": "D",   # {"id:type": VariantStats}
    "designs.equipment_variant_stats.*.archetype": "S",  # echoed straight from variant
    "designs.equipment_variant_stats.*.family": "S",
    # --- top-level order_of_battle (§12, from oob.json) ---
    # The LIVE oob_command reader (§12c) folds the command TREE into armies/army_groups/
    # fleets/theatres (membership + leader + owner-consensus), so those are live-first with
    # the save oob.json as the backstop (M/S). navy_theaters and the save-shaped
    # order_of_battle.air_wings list are not filled by oob_command -> save-only (S).
    # The separate raw `air_wings` domain carries live wing telemetry. by_country
    # is the per-tag count rollup (S/D). Per-list *_source flags are "M" when the
    # live read filled the list, else absent (save-sourced).
    "order_of_battle.army_groups": "M/S",
    "order_of_battle.armies": "M/S",
    "order_of_battle.fleets": "M/S",
    "order_of_battle.theatres": "M/S",
    "order_of_battle.navy_theaters": "S",
    "order_of_battle.air_wings": "S",
    "order_of_battle.by_country": "S/D",
    # live per-ship roster from the global CShip registry (§12b). All M.
    "order_of_battle.ships": "M",
    # LIVE battle-plan orders (construction_orders_live, M). DISTINCT live-only list: per-army
    # offensive/front orders, each {army_id, order_type, target_states}. target_states is the
    # exact state-id set for type-5 (offensive) orders ONLY; non-type-5 geometry is RESISTED
    # (null target_states), never fabricated. army_id + target_states are RAW engine ints.
    "order_of_battle.battle_plan_orders": "M",
    # --- per-country OOB view (§12, derived rollup) ---
    "countries.*.order_of_battle": "S/D",
    # --- top-level live MIO catalog (§11b, mio_live.py). All M. ---
    "mio_live.by_tag": "M",
    "mio_live.by_id": "M",
    # resolved allowed-policy ICON tokens per MIO (mio + defnames_live, M-derived). ADDED
    # ALONGSIDE allowed_policies_count; absent when resolution is off or a handle is unknown
    # (no fabrication). Icon token (GFX_mio_policy_<suffix>), not the unique script name.
    "mio_live.by_id.*.policies": "M",
    "mio_live.by_tag.*.policies": "M",
    # --- top-level live special-projects catalog (§11c, projects_live.py / DLL
    # projects_live). Catalog rows are C/M; resolved traits remain memory-derived.
    # Raw static-def db-id HANDLES (active_project_handle / static_def_handle) are
    # measured ids, not resolved names. project_progress/phase_progress/
    # current_state are serialize-COMPUTED and intentionally NOT carried. ---
    "special_projects.facilities": "C/M",
    "special_projects.projects": "C/M",
    "special_projects.scientists": "C/M",
    # resolved scientist TRAIT tokens (projects_live + defnames_live, M-derived). ADDED
    # ALONGSIDE trait_count via the direct trait def-pointer walk; absent when resolution is
    # off or the walk fails closed (no fabrication). A measured-empty [] = a traitless scientist.
    "special_projects.scientists.*.traits": "M",
    # --- top-level LIVE diplomacy sidecar (§13b, diplomacy_live.py / DLL). World-scoped
    # data that has NO save-shaped home: the pairwise war_relations list (each TAG pair +
    # per-side casualties) + faction_members keyed by the OPAQUE engine diplomatic-group id
    # (stringified for JSON; member TAGs valid_tag-checked). Distinct from the SAVE
    # diplomacy_world (war-id-keyed wars{} + save-faction-id-keyed factions{}) — a DIFFERENT
    # identity model, so it is NOT folded into diplomacy_world. subtype_warning is the
    # reader's completeness advisory (nonempty => a possible missed war_relation subtype). ---
    "diplomacy_live.war_relations": "M",
    "diplomacy_live.faction_members": "C/M",
    # --- top-level LIVE equipment-MARKET sidecar (§7c, decisions_market_live.py). All M. The
    # global purchase contracts (28/28 EXACT vs save), each {contract_id, seller(TAG), buyer(TAG),
    # equip_variant_id, amount, price}. Distinct from the SAVE "market" domain (a per-country
    # diplomacy.lend_lease + §7b world list) — a different identity model + provenance. ---
    "market_live.contracts": "M",
    # --- state (§4) ---
    "states.*.owner": "C/M/S",
    "states.*.controller": "C/M/S",
    "states.*.is_core_of": "S",
    # building levels are now LIVE-first (buildings_live, M) with the save as the backstop:
    # the live reader resolves every per-state building level exactly (validated vs the melt).
    "states.*.buildings": "M/S",
    "states.*.province_buildings": "S",
    "states.*.victory_points_total": "M/S",       # live state_vp (state_vp_live, measured 0 kept)
    "states.*.victory_points_by_province": "M/S",  # live state_vp (per-province int VP map)
    "states.*.resources": "M/S",          # live state_fields_live (CState def table; live-first)
    "states.*.dynamic_modifier": "M",      # live state_fields_live (per-state modifier name list)
    "states.*.extra_shared_slots": "M",    # live state_fields_live (i32 shared-building slot count)
    "states.*.manpower_pool": "M/S",
    "states.*.category": "M/S",
    "states.*.occupation": "S",
    # live state_detail measures resistance/compliance; the rest of occupation
    # (occupied_country_tag, *_speed, …) stays save-sourced (schema review #6).
    "states.*.occupation.resistance": "M/S",
    "states.*.occupation.compliance": "M/S",
    "states.*.occupation_law": "S",
    "states.*.demilitarized": "S",
    "states.*.supply": "S/M*",
    # --- province (§5) ---
    "provinces.*.controller": "C/M/S/D",
    "provinces.*.state_id": "D",
    "provinces.*.supply": "S",
    # LIVE per-province building levels (buildings_live, M): {building_name: level} for the
    # province-scoped buildings (naval_base/rail_way/supply_node/forts/...). DISTINCT live-only
    # leaf, validated vs the melt. null until a live read folds it.
    "provinces.*.buildings": "M",
    # --- division (§6) ---
    "divisions.*.div_id": "C/M/S",
    "divisions.*.div_type": "C/M/S",
    "divisions.*.owner": "C/M/S",
    "divisions.*.location_province": "C/M/S",
    "divisions.*.state_id": "C/M/S/D",
    "divisions.*.template_id": "C/M/S",
    "divisions.*.template_type": "C/M/S",
    "divisions.*.template_name": "C/M/S",
    "divisions.*.template_role": "S",
    "divisions.*.organisation": "C/M",
    "divisions.*.strength": "C/M/S",
    "divisions.*.equipment": "M/S",
    # LIVE per-division held-equipment inventory (division_equipment_live, M): {variant_id: count}.
    # DISTINCT live-only leaf from the save-shaped `equipment` above (no clobber).
    "divisions.*.equipment_live": "M",
    "divisions.*.experience": "C/M*",
    "divisions.*.moving": "C/M*",
    "divisions.*.move_target_province": "C/M*",
    "divisions.*.move_progress": "C/M*",
    "divisions.*.in_combat": "C/M*",
    "divisions.*.supply_status": "C/M*",
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
    # LIVE supply solver INPUTS (supply_inputs, M). DISTINCTLY-NAMED + SEPARATELY-BADGED from
    # the division-province proxy above: per-province efficiency/fulfilment + draw/demand are
    # the PERSISTENT solver inputs that ARE stored, NOT the exact per-province "supply
    # available" number (which is provably not stored — transient routing-solver output). The
    # solver_inputs_confidence field self-badges "live_solver_input_not_exact_supply" so a
    # consumer never reads efficiency/draw as supply available.
    "supply.solver_efficiency_by_province": "M",
    "supply.solver_draw_by_province": "M",
    # MODEL air-superiority estimate (WAVE-2, models/air_superiority). Derived
    # (planes x stat); the per-region percent is a W6 display estimate -> D.
    "naval_air.air_superiority_model.*.by_owner.*.score": "D",
    "naval_air.air_superiority_model.*.total_score": "D",
    "naval_air.air_superiority_model.*.leader": "D",
    "naval_air.air_superiority_model.*.percent": "D",
    "naval_air.air_superiority_model_meta": "D",
    # --- live weather domain (§19, WAVE-2). M live; phenom_name + rollup are D. ---
    "weather.regions.*.temp_c": "M",          # SWeatherPerRegion+0x138 i64/1e5 (retracted reader; null)
    "weather.regions.*.phenom": "M",          # +0x78 enum (already pinned)
    "weather.regions.*.phenom_name": "D",     # derived from phenom via file-order map
    "weather.regions.*.active": "M",          # +0x5D intensity flag
    # re-cracked per-region fields (weather_fields_live, validated 305/305 vs the melt):
    "weather.regions.*.temperature": "M",                # +0x138 i32/1e5 deg C (region temp)
    "weather.regions.*.phenomenon": "M",                 # phenom-flag band (u8 names list)
    "weather.regions.*.next_weather_change_hours": "M",  # +0x150 (abs hours; None == no change)
    "weather.regions.*.active_modifier_count": "M",      # +0x30 PdxVector size
    "weather.regions.*.region_id": "M",
    # live per-PROVINCE weather (weather_temp_model, M). The realized displayed temperature
    # + precipitation (`water`) + snow share the SAME per-province struct at province-id index.
    # temp/precip/snow are i32/1e5; None == not realized / zero-or-unmeasured (never a fab 0).
    "weather.provinces.*.temp_c": "M",        # SWeatherPerProvince+0x118 i32/1e5
    "weather.provinces.*.temp_change_c": "M", # +0x128 i32/1e5 (signed)
    "weather.provinces.*.precip": "M",        # +0x38 i32/1e5 `water` amount (save-persisted)
    "weather.provinces.*.snow": "M",          # +0x40 i32/1e5 snow amount (save-persisted)
    # re-cracked per-province fields (weather_fields_live, full-array TP/FP/FN validated):
    "weather.provinces.*.mud": "M",                  # +0x130 u8 bool (146 mud provinces)
    "weather.provinces.*.custom_modifier_count": "M",  # +0x148 PdxVector size (52 provinces)
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
    "world.scenario.difficulty_settings_empty": "S",
    # LIVE scenario DATES (world_extract_live, M). DISTINCT live-only keys under the save-shaped
    # scenario block: current_date_live (the live in-game date, EXACT vs save) + start_date_live
    # (the scenario start date, EXACT). EXPLICIT M entries so source_of() does NOT fall back to
    # the parent "world.scenario"=S (which would mislabel a live read as save — schema review #1).
    "world.scenario.current_date_live": "M",
    "world.scenario.start_date_live": "M",
    "world.variables": "S",
    "world.flags": "S",
    "world.tech_sharing_groups": "S",
    "world.power_balances": "S",
    "world.faction_influence": "S",
    "world.world_tension_inputs": "S",
    "world.saved_event_targets": "S",
    # --- map (§8) ---
    "map.owner_by_state": "C/M/S/D",
    "map.control_by_province": "C/M/S/D",
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
# are lowercase, so they never match (schema integration review finding 2).
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
    are rejected rather than silently aliasing onto another day (schema P3 #14).
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
    collector: Optional[Dict[str, Any]] = None
    memory: Optional[Dict[str, Any]] = None
    save: Optional[Dict[str, Any]] = None
    # The ACTIVE MOD-SET this snapshot was captured under, read at CAPTURE time from
    # the host's the game launcher config (`dlc_load.json` + the `.mod` descriptors it
    # names) — NOT from game memory and NOT from the melted save. This is the
    # per-replay MOD IDENTITY the native UI auto-matches against, so vanilla chrome
    # is never rendered over a mod's replay (GAME_UI_REPLICA_PLAN §K).
    #
    # NO FABRICATION (the cardinal rule): `mod` is `None` whenever no launcher
    # config was found (a SP/vanilla run, or capture without launcher-dir access) —
    # it is NEVER a guessed or default mod-set. A genuinely-vanilla run that DID read
    # the launcher config records `mod = {modset_id:"vanilla", mods:[], ...}` (a
    # measured empty mod-set), which is distinguishable from `None` (not read).
    #
    # Shape when present (a ModSet, see `empty_modset`):
    #   { "modset_id":  str,                # stable digest of (id|version|order)
    #     "source":     str,                # e.g. "dlc_load.json"
    #     "load_order": [str, ...],         # ordered enabled-mod descriptor refs
    #     "mods": [ { "name": str|None, "workshop_id": str|None,
    #                 "version": str|None, "supported_version": str|None,
    #                 "descriptor": str|None }, ... ] }   # in LOAD ORDER
    mod: Optional[Dict[str, Any]] = None
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
    # LIVE convoy STOCKPILE count (country_obj +0x1280, M). Distinct from the
    # save-side `convoys` LIST (per-equipment rows): this is the single integer
    # total the country object caches. null until a live read folds it; a measured
    # 0 is a real "no convoys". See economy_offsets.draft.json `convoys`.
    convoys_count: Optional[int] = None
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
    national_spirits: List[str] = field(default_factory=list)
    idea_groups: Optional[Dict[str, List[str]]] = None
    advisor_ideas: Optional[Dict[str, List[str]]] = None
    # LIVE active-ideas list (politics_live, M). DISTINCT live-only leaf, separate from the
    # save-derived ideas_all / ideas_other/national_spirits above: the ORDERED full
    # active-ideas/spirits set (incl. the economy/trade/mobilisation LAW ideas, by id) read straight from the live
    # CPoliticalStatus ideas vector (set AND order EXACT vs save). Defaults None (Optional)
    # NOT [] so an unmeasured country reads None (skipped by the backstop's `lv is not None`
    # guard); a measured-empty [] is a real "no active ideas" and is written + protected, so
    # no _SKELETON_EMPTY_CONTAINER_LEAVES entry is needed (live-only leaf — schema).
    ideas: Optional[List[str]] = None
    idea_count: Optional[int] = None
    # LIVE election cadence (politics_live, M): months between elections (ps+0xE8, EXACT).
    election_frequency: Optional[int] = None
    # LIVE party count (politics_live, M): the size of the parties vector (== 4 normally).
    party_count: Optional[int] = None
    # LIVE confidence flag (politics_live): True when the ruling-party-in-vector / party_count
    # discriminator was not fully satisfied this frame (a read-quality flag, never a value).
    low_confidence: Optional[bool] = None
    # LIVE election residuals + appointed advisors (politics_residuals_live, M). DISTINCT
    # live-only leaves: `last_election` is the decoded "Y.M.D.H" date of the last election
    # (46/46 EXACT vs save); `elections_allowed` is the u8 election-enabled bool (46/46 EXACT);
    # `appointed_advisors` is the idea-token list of HIRED cabinet advisors (the advisor roster
    # idea tokens present in the active ideas; 46/46 EXACT); details/slots/ledgers add static
    # `common/characters` metadata. All default None (unmeasured); a
    # measured-empty appointed_advisors [] (no advisor hired) is a real value (live-only leaf,
    # so no _SKELETON_EMPTY_CONTAINER_LEAVES entry needed — schema).
    last_election: Optional[str] = None
    elections_allowed: Optional[bool] = None
    appointed_advisors: Optional[List[str]] = None
    appointed_advisor_details: Optional[List[Dict[str, Any]]] = None
    appointed_advisor_slots: Optional[Dict[str, List[str]]] = None
    appointed_advisor_ledgers: Optional[Dict[str, List[str]]] = None
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
    # LIVE active-research rows (tech_live, C/M). DISTINCT leaf from the save-shaped
    # `in_progress` (whose elements are {name, points_factor}); these carry the raw
    # engine shape {slot_index, tech_handle_raw, tech_def_index_raw, progress_raw}.
    # null-but-present until a live read folds it (None != a measured-empty []).
    in_progress_live: Optional[List[Dict[str, Any]]] = None
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
    # LIVE active CONTINUOUS focus (focus_live, M). DISTINCT live-only leaf: the save
    # Focus has no such field (it carries only the continuous_focus_palette id). National
    # `current` and `current_continuous` are mutually exclusive (the live XOR invariant,
    # matching the save). null-but-present until a live read folds it.
    current_continuous: Optional[str] = None
    progress: Optional[float] = None
    paused: Optional[bool] = None


@dataclass
class Production:
    """LIVE per-country production picture (§3.18, production_live.py). C/M = collector
    DLL or legacy process memory. NEW live-only home, distinct from the SAVE-sourced
    "production" domain
    (which folds into military.equipment_stockpile / military.convoys /
    economy._production_lines — a different shape AND home), so the save backstop never
    writes here and the two channels stay independently auditable.

    ``stockpile`` maps a save equipment-VARIANT id (str/int key) to the banked amount
    (float, i64 fixed-point /100000). ``stockpile_total`` is the sum of those amounts
    (derived-from-live, C/M/D). ``military_lines`` / ``naval_lines`` each carry the
    measured per-line {id, active_factories, cost, speed, produced, amount}. The factory
    totals are SUM(active_factories) over the respective line list.

    RESISTED (absent, never fabricated — see production_offsets.draft.json open tickets):
    per-line priority / requested_factories / equipment_variant_index (not offset-pinned,
    no distinguishing variance this frame); construction queue (state-derived, owned by
    the state domain). null-but-present until a live read folds it.
    """
    stockpile: Optional[Dict[str, Any]] = None
    stockpile_total: Optional[float] = None
    military_lines: Optional[List[Dict[str, Any]]] = None
    naval_lines: Optional[List[Dict[str, Any]]] = None
    military_factories: Optional[int] = None
    naval_factories: Optional[int] = None
    # LIVE construction queue (construction_orders_live, M). DISTINCT live-only leaf: the
    # per-country civil/military BUILDING queue in PRIORITY order, each {line_id, template_id,
    # active_factories, cost, produced, priority}. Defaults None (Optional) NOT [] so an
    # unmeasured country (created by another live domain) reads None and the backstop's
    # `lv is not None` guard skips it; a measured-empty [] (empty queue) is written + protected.
    # The build target STATE/location is RESISTED (province handle; needs a province->state map)
    # -> never fabricated. null-but-present until a live read folds it.
    construction_queue: Optional[List[Dict[str, Any]]] = None


@dataclass
class Decisions:
    active: List[Dict[str, Any]] = field(default_factory=list)
    timed: List[Dict[str, Any]] = field(default_factory=list)
    missions: List[Dict[str, Any]] = field(default_factory=list)
    # LIVE active timed decisions (decisions_market_live, M). DISTINCT live-only leaf, separate
    # from the save-shaped `timed` above: each {decision, days, state, state_name} for the
    # country's currently-active timed decisions. Defaults None (Optional) NOT [] so an
    # unmeasured country reads None (skipped by the backstop's `lv is not None` guard); a
    # measured-empty [] (no active timed decision) is a real value (live-only leaf, so no
    # _SKELETON_EMPTY_CONTAINER_LEAVES entry is needed — schema). null until a live read folds it.
    timed_live: Optional[List[Dict[str, Any]]] = None


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
    # LIVE diplomacy (diplomacy_live, M). DISTINCT live-only leaves, separate from the
    # save-derived `wars` ([{against}]) above. `at_war` is the live enemy-TAG list from
    # the global war_relation pool; `casualties` is this country's cumulative war
    # casualties; `faction_group` is the OPAQUE engine diplomatic-group id (NOT a TAG —
    # an int that is meaningful only via diplomacy_live.faction_members). Each defaults
    # to None (Optional) NOT [] so an UNMEASURED country (created by another live domain)
    # reads None and the backstop merge's `lv is not None` guard skips it; a measured-empty
    # at_war [] (a country genuinely at peace) is written + protected (no skeleton-[]
    # masquerade, so no _SKELETON_EMPTY_CONTAINER_LEAVES entry is needed — schema).
    at_war: Optional[List[str]] = None
    casualties: Optional[int] = None
    faction_group: Optional[int] = None
    # LIVE bilateral RELATIONS (diplo_relations_live, M). DISTINCT live-only leaves that
    # share the per-country diplomacy home. `opinion` (the save-shared {TGT: net_opinion}
    # leaf above) is folded LIVE-FIRST here (cached_sum, 1113/1113 real ordered pairs EXACT
    # vs save); `relation_flags` is {TGT: [flag,...]} of the 12 directed relation-flag types
    # (NAP/embargo/military_access/.../puppet) the country has toward each target. Both
    # default None (Optional) NOT {} so an UNMEASURED country (created by another live domain)
    # reads None and the backstop's `lv is not None` guard skips it; a measured-empty {} is a
    # real "no relations" and is written + protected (no skeleton-{} masquerade -> no
    # _SKELETON_EMPTY_CONTAINER_LEAVES entry needed -- schema).
    relation_flags: Optional[Dict[str, Any]] = None
    # SUBJECT/AUTONOMY (diplo_relations_live, M). This country's overlord-or-subject role,
    # read from the CSubjectRelation sub-object (the save puppet={} block). `overlord` is the
    # overlord TAG when THIS country is a subject (else None); `autonomy_state` is the named
    # autonomy level (e.g. "autonomy_puppet"); `autonomy_ratio` the i32/1e5 ratio. All default
    # None (unmeasured); a country with no subject relation simply carries None on all three.
    overlord: Optional[str] = None
    autonomy_state: Optional[str] = None
    autonomy_ratio: Optional[float] = None
    # LIVE opinion-modifier decomposition (construction_orders_live, M). DISTINCT live-only leaf
    # {TGT: [{token_id, value}, ...]} decomposing the cached_sum net `opinion` above into its
    # individual modifiers (the modifier-vector SIZE + value-SET 178/178 pairs EXACT vs save).
    # The modifier-token id->NAME registry is RESISTED -> only the raw token_id (int) + value
    # (float) are carried, never a fabricated name. Defaults None (Optional) NOT {} so an
    # unmeasured country reads None (skipped by the backstop's `lv is not None` guard); a
    # measured-empty {} is never emitted by the plug (it only keys a country with measured pairs).
    opinion_modifiers: Optional[Dict[str, Any]] = None


@dataclass
class Intel:
    # agency = {name, icon, is_created, upgrade_progress, current_upgrade,
    #           upgrades{branch:level}, capacity{max_operatives,usable_slots,defense}}
    agency: Optional[Dict[str, Any]] = None
    operatives: List[Dict[str, Any]] = field(default_factory=list)
    # LIVE operative roster (intel_deep_live, M). DISTINCT live-only leaf, separate from the
    # save-shaped `operatives` above (a richer save list): the agency's ACTIVE-OWNED operatives,
    # each {op_id, name, detected}. operatives 56/56 + per-country owner counts EXACT vs save.
    # null until a live read folds it (no fabrication; a measured-empty [] is a real "no ops").
    operatives_live: Optional[List[Dict[str, Any]]] = None
    # LIVE cryptology (intel_deep_live, M). DISTINCT live-only leaf {TGT: {amount, decrypting}}
    # of this agency's decryption progress toward each target's cipher (crypto targets 37/37
    # EXACT vs save). null until a live read folds it.
    cryptology_live: Optional[Dict[str, Any]] = None
    # LIVE operative SKILL + XP (final_residuals_live, M). DISTINCT live-only leaf {op_id: {name,
    # skill, experience}} — the per-operative overall skill (u32) + accumulated experience (i64/
    # 1e5), read from the CORRECTED operative body base L=ident-0x8 (skill+experience 56/56 EXACT
    # vs save). null until a live read folds it.
    operative_skill_xp: Optional[Dict[str, Any]] = None
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
    # LIVE unit-leaders (commanders_live, M). DISTINCT from the role-split save
    # leaves above: the live reader cannot split generals vs field-marshals (needs
    # the RESISTED overall level), so it carries the UNSPLIT army leaders here +
    # admirals under navy_leaders. Each element: {char_id, unit_leader_id, name,
    # country(TAG), skills{attack/defense/...}, experience(float), trait_xp[]}.
    # When the OPT-IN leader_detail enrichment runs (ctx.extras["resolve_leader_detail"]),
    # each record additionally carries `skill` (overall level, int, leader_skill_traits_live,
    # M) + `traits` (assigned-trait token list, M) joined by unit_leader_id; absent when the
    # enrichment is off or that leader was not measured (overall LEVEL was otherwise RESISTED;
    # never fabricated). null-but-present until a live read.
    army_leaders: Optional[List[Dict[str, Any]]] = None   # generals + field marshals (unsplit)
    navy_leaders: Optional[List[Dict[str, Any]]] = None   # admirals


@dataclass
class Country:
    economy: Dict[str, Any] = field(default_factory=lambda: asdict(Economy()))
    military: Dict[str, Any] = field(default_factory=lambda: asdict(Military()))
    politics: Dict[str, Any] = field(default_factory=lambda: asdict(Politics()))
    research: Dict[str, Any] = field(default_factory=lambda: asdict(Research()))
    focus: Dict[str, Any] = field(default_factory=lambda: asdict(Focus()))
    # LIVE per-country production (§3.18, production_live.py, C/M). NEW live-only home,
    # distinct from the SAVE "production" domain (which folds into economy/military).
    # Always present; every leaf null until a live read folds it (no fabrication).
    production: Dict[str, Any] = field(default_factory=lambda: asdict(Production()))
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
    # LIVE per-country MISC scalars (country_misc_live.py, §3.17, M). null until a
    # live read folds it: {convoys_destroyed, accidents_score, ace_count,
    # cosmetic_tag}. A measured 0 is a real value; cosmetic_tag is None when unset.
    misc: Optional[Dict[str, Any]] = None
    # LIVE per-country STATIC identity scalars (country_static_live.py, M). null until a
    # live read folds it: {original_capital, original_tag, reserved_dynamic_country,
    # templates_locked}. original_tag is a resolved TAG or None; the bools are real reads.
    identity: Optional[Dict[str, Any]] = None
    # LIVE per-country SCRIPT-VARIABLES bag (variables_live.py, M). null until a live read
    # folds it: {name: value} (fixed-point; array vars are name^N keys). A measured-empty
    # {} = the located container is empty. The mass closure for nation scripted-GUI overlays.
    script_variables: Optional[Dict[str, Any]] = None
    # LIVE country_reports latest-record scalars (country_containers_live.py, M). null until
    # folded: {days, index, date_raw}.
    country_reports: Optional[Dict[str, Any]] = None
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
    # VP: total scalar AND per-province map are distinct things (schema P1 #6).
    victory_points_total: Optional[int] = None
    victory_points_by_province: Optional[Dict[str, int]] = None
    resources: Optional[Dict[str, float]] = None
    # LIVE per-state extra fields (state_fields_live.py, M). dynamic_modifier = the list of
    # active state modifier names (real read; [] = none). extra_shared_slots = i32 shared
    # building-slot count (real read; incl. a measured 0). null until a live read folds them.
    dynamic_modifier: Optional[List[str]] = None
    extra_shared_slots: Optional[int] = None
    # manpower is a pool object {available, locked, total}, not a scalar (schema P1 #6).
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
    # LIVE per-province building levels (buildings_live, M): {building_name: level} for the
    # province-scoped buildings (naval_base/rail_way/supply_node/land_fort/coastal_fort/...).
    # null-but-present until a live read folds it (validated vs the melt; no fabrication).
    buildings: Optional[Dict[str, int]] = None
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
    # LIVE per-division held-equipment inventory (division_equipment_live, M). DISTINCT live-only
    # leaf, separate from the save-shaped `equipment` above (which may carry a richer per-item
    # shape): {variant_id(str): count_held}. null-but-present until a live read folds it; a
    # measured count is the held amount (validated vs the melt). Kept DISTINCT so the live join
    # never clobbers the save-owned equipment list (schema review).
    equipment_live: Optional[Dict[str, Any]] = None
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
    # LIVE equipment-VARIANT module catalog (designs_mio_deep_live, M). DISTINCT live-only leaf,
    # keyed by the bare live variant id (str) -> {modules{slot_name: module_name}, upgrade_levels[]}.
    # Kept SEPARATE from the save `equipment_variants` above (which is keyed "id:type" and carries
    # the save's richer per-variant record) so the live join never collides the two key spaces or
    # discards save metadata (schema review #3). upgrade NAMES are RESISTED (levels only). null
    # until a live read folds it; `equipment_variants_live_source` self-badges "M".
    equipment_variants_live: Optional[Dict[str, Any]] = None
    equipment_variants_live_source: Optional[str] = None
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
class WeatherProvince:
    """One per-PROVINCE weather row (§19, live). M = process memory.

    The realized DISPLAYED temperature + precipitation (`water`) + snow all live in the
    SAME per-province weather struct at province-id index (the per-region temperature read
    was retracted -- it was a climatology base, not the displayed value). ``temp_c`` is None
    when the temp slot reads exactly 0 (sea/inactive province -- NOT a literal 0 C).
    ``precip`` / ``snow`` are the save-persisted i32/1e5 amounts; the reader collapses a
    zero/absent slot to None (a measured 0.0 is indistinguishable from unmeasured), so None
    here means zero-or-unmeasured and is NEVER fabricated to 0.0. Always source-badged.
    """
    province_id: Optional[int] = None
    temp_c: Optional[float] = None
    temp_change_c: Optional[float] = None
    precip: Optional[float] = None       # `water` amount (None == zero-or-unmeasured)
    snow: Optional[float] = None         # snow amount  (None == zero-or-unmeasured)


@dataclass
class Weather:
    """Live weather domain (§19, from weather_temp_model + weather_state_reader).

    NEW first-class domain. There is NO save oracle for weather (never persisted),
    so every row is live-memory (M) except ``phenom_name`` + the rollup (D). Always
    present; ``regions`` / ``provinces`` null until a live read folds them. ``source``
    self-badges.

    GRANULARITY: the live temperature/precip/snow read is PER-PROVINCE (``provinces``);
    the per-REGION temperature field was retracted (climatology base, not displayed value),
    so ``regions`` has no working live reader and stays null. A consumer reads ``provinces``
    for live temp/precip/snow.
    """
    regions: Optional[Dict[str, Any]] = None    # {region_id(str): WeatherRegion dict}
    provinces: Optional[Dict[str, Any]] = None  # {province_id(str): WeatherProvince dict}
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
    # LIVE supply solver INPUTS (supply_inputs, M). DISTINCTLY-NAMED + SEPARATELY-BADGED from
    # the division-province proxy above: per-province efficiency/fulfilment + draw/demand are
    # the PERSISTENT solver inputs (NOT the exact per-province "supply available" number,
    # which is provably not stored). Keyed by raw province id (str). null until a live read
    # folds them; solver_inputs_confidence self-badges "live_solver_input_not_exact_supply"
    # so a consumer never reads efficiency/draw as supply available.
    solver_efficiency_by_province: Optional[Dict[str, Any]] = None
    solver_draw_by_province: Optional[Dict[str, Any]] = None
    solver_inputs_source: Optional[str] = None       # "M" when a live read filled them
    solver_inputs_confidence: Optional[str] = None   # "live_solver_input_not_exact_supply"


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
class MioLive:
    """Top-level LIVE Military-Industrial-Organisation catalog (§11b, mio_live.py).

    NEW first-class LIVE container, distinct from the save-sourced per-country
    Country.mios (different shape + provenance). ``by_tag`` maps a TAG to its list
    of live org records; ``by_id`` maps an org id (str) to the single record.
    Each record: {id, owner_tag_db_id, owner(TAG|None), level, points, funds,
    research_bonus, unlocked_traits_count, allowed_policies_count}. null until a
    live read folds it; ``source`` self-badges. Always present in the skeleton.
    """
    by_tag: Optional[Dict[str, Any]] = None
    by_id: Optional[Dict[str, Any]] = None
    count: Optional[int] = None
    source: Optional[str] = None        # "M" when a live read filled it, else null


@dataclass
class MarketLive:
    """Top-level LIVE equipment-MARKET sidecar (§7c, decisions_market_live.py). M = process
    memory.

    NEW first-class LIVE container for the global equipment-market purchase contracts, distinct
    from the SAVE "market" domain (which attaches trade contracts under each country's
    diplomacy.lend_lease + a §7b world list — a DIFFERENT identity model + provenance, so the
    save backstop never writes here). ``contracts`` is the world list of CPurchaseContract
    records (28/28 EXACT vs save), each:
        {contract_id(int), seller(TAG|None), buyer(TAG|None), equip_variant_id(int|None),
         amount(float|None), price(float|None)}.
    The raw seller/buyer tag-db-ids + the buyer country-object pointer are diagnostic and are
    STRIPPED before folding; seller/buyer are resolved TAGs (re-validated via valid_tag at the
    assembler boundary). null-but-present until a live read folds it; ``source`` self-badges "M".
    """
    contracts: Optional[List[Dict[str, Any]]] = None
    count: Optional[int] = None
    source: Optional[str] = None        # "M" when a live read filled it, else null


@dataclass
class SpecialProjects:
    """Top-level LIVE special-projects / facilities / scientists catalog (§11c,
    projects_live.py / scheduled collector projects_live). C/M = in-process collector
    or legacy process-memory reader.

    NEW first-class LIVE container, distinct from the SAVE-sourced per-country
    Country.projects / Country.scientists (§3.16, S) — different shape AND
    provenance, so the save backstop never writes here. The live reader exposes
    GLOBAL pools with NO resolvable owner TAG (facilities carry only a province id;
    scientists carry no country), so the lists are world-scoped and are NOT keyed by
    TAG — per-country attribution is an open ticket, never fabricated.

    ``facilities`` each: {id, province, building(str|None), active_project_handle
    (raw static-def db-id handle, NOT a token name), active(bool),
    scientist_char_id(int|None), unread_prototype_rewards(int)}.
    ``projects`` each: {id, static_def_handle (raw handle, NOT a token name),
    iterations(int), worked_at_facility(facility_id|None)}. project_progress /
    phase_progress / current_state are serialize-time COMPUTED (not stored live) and
    stay absent — never fabricated.
    ``scientists`` each: {char_id, name_token(str|None), display_name(str|None),
    skills{specialization_nuclear/_naval/_air/_land: int}, trait_count(int|None)}.
    Trait NAMES are unresolved registry handles -> only the count is carried.

    null-but-present until a live read folds it; ``source`` self-badges "C" or "M".
    """
    facilities: Optional[List[Dict[str, Any]]] = None
    projects: Optional[List[Dict[str, Any]]] = None
    scientists: Optional[List[Dict[str, Any]]] = None
    count: Optional[Dict[str, int]] = None   # {facilities, projects, scientists}
    source: Optional[str] = None             # "C"/"M" when a live read filled it


@dataclass
class DiplomacyLive:
    """Top-level LIVE diplomacy sidecar (§13b, diplomacy_live.py / DLL). C/M = live.

    NEW first-class LIVE container for the world-scoped diplomacy data that has NO
    save-shaped home. Distinct from the SAVE-sourced snap["diplomacy_world"] (whose
    `wars` is war-id-keyed and `factions` is save-faction-id-keyed with rich
    {id,template,members,leader} records) — the live data is a DIFFERENT identity model:

    ``war_relations`` is the pairwise list straight from the global war_relation pool,
    each {first(TAG), second(TAG), first_casualties(int), second_casualties(int)}; both
    TAGs are valid_tag-checked. ``faction_members`` maps the OPAQUE engine diplomatic-
    group id (stringified for JSON; NOT the save faction id) to its member TAG list (each
    valid_tag-checked). ``subtype_warning`` is the reader's completeness advisory: a
    nonempty dict means a record that LOOKED like a war_relation carried a foreign type
    pointer (a possible missed subtype) — surfaced, never silently dropped.

    Forcing this into diplomacy_world would collide the two key spaces and fabricate a
    unified identity model that does not exist, so it lives here. null-but-present until
    a live read folds it; ``source`` self-badges "C" for the DLL collector or "M" for the
    external process-memory reader.
    """
    war_relations: Optional[List[Dict[str, Any]]] = None
    faction_members: Optional[Dict[str, Any]] = None
    subtype_warning: Optional[Dict[str, Any]] = None
    source: Optional[str] = None        # "C"/"M" when a live read filled it, else null


@dataclass
class OrderOfBattle:
    """Top-level world order-of-battle (from oob.json, §12). Always present.

    Command structure is world-scoped (cross-tag expeditionary/lend-lease divisions can
    sit in another tag's army), so the canonical lists live here; each element carries
    its `owner` TAG. `by_country` is the per-tag count rollup (also mirrored onto
    Country.order_of_battle for convenience).

    LIVE command TREE (§12c, oob_command_live.py, M/S): the raw "oob_command"
    domain currently carries army-group member army ids, army member division ids,
    army parent group ids, army leader ids, fleet member task-force ids, and
    task-force member ship instance ids. The save oob.json remains the BACKSTOP
    for owner TAGs, command names, theatres, missions, locations, naval leaders,
    and save-shaped nesting. The separate raw `air_wings` domain carries live
    wing telemetry.
    """
    army_groups: List[Dict[str, Any]] = field(default_factory=list)
    armies: List[Dict[str, Any]] = field(default_factory=list)
    fleets: List[Dict[str, Any]] = field(default_factory=list)
    navy_theaters: List[Dict[str, Any]] = field(default_factory=list)
    air_wings: List[Dict[str, Any]] = field(default_factory=list)
    theatres: List[Dict[str, Any]] = field(default_factory=list)
    by_country: Dict[str, Any] = field(default_factory=_none_dict)
    # LIVE per-SHIP roster from the global CShip registry (oob_full_live.py, §12b,
    # M). Each item: {instance_id, definition, category, strength_hp, organisation,
    # experience, sunk_convoys}. owner is NOT carried live (it comes from the save
    # OOB join), so live ships have no owner. Empty-but-present until a live read
    # folds it. Validator-inert (a new list key alongside the canonical sub-lists).
    ships: List[Dict[str, Any]] = field(default_factory=list)
    # LIVE battle-plan ORDERS (construction_orders_live, §12d, M). DISTINCT live-only list:
    # per-army offensive/front orders, each {army_id, order_type, target_states}. target_states
    # is the EXACT state-id set for type-5 (offensive) orders ONLY (107/108 EXACT vs save); non-
    # type-5 order geometry is RESISTED (null target_states), never fabricated. army_id +
    # target_states are RAW engine ints (NOT TAGs). Defaults None (Optional) NOT [] so an
    # unmeasured frame reads None; a measured-empty [] (no battle plans) is written + protected.
    battle_plan_orders: Optional[List[Dict[str, Any]]] = None


# --- Event (§9) ---
EVENT_TYPES = (
    # territory / control
    "state_owner_change", "state_controller_change", "province_controller_change",
    # country lifecycle (schema P1 #2, P3 #13)
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
EVENT_SOURCES = ("C", "M", "M*", "S", "D", "marker")


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
    """Canonical division identity = full Clausewitz id '(id:type)' (schema P1 #3).

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


def empty_production() -> Dict[str, Any]:
    return asdict(Production())


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


def empty_mio_live() -> Dict[str, Any]:
    return asdict(MioLive())


def empty_market_live() -> Dict[str, Any]:
    return asdict(MarketLive())


def empty_diplomacy_live() -> Dict[str, Any]:
    return asdict(DiplomacyLive())


def empty_special_projects() -> Dict[str, Any]:
    return asdict(SpecialProjects())


def empty_naval_air() -> Dict[str, Any]:
    return asdict(NavalAir())


def empty_supply() -> Dict[str, Any]:
    return asdict(Supply())


def empty_weather() -> Dict[str, Any]:
    return asdict(Weather())


def empty_models() -> Dict[str, Any]:
    return asdict(ModelsBlock())


def modset_id(mods: List[Dict[str, Any]]) -> str:
    """A stable, order-SENSITIVE digest identifying a mod-set (provenance.mod).

    The identity that two mod-sets must agree on to be "the same skin" is the
    ORDERED list of (workshop_id-or-name, version) pairs — a different load order,
    a different version, or a different mod composition all change the digest, so
    the native auto-matcher catches every one of those as a mismatch.

    `mods` is the per-mod list already in LOAD ORDER. Each mod contributes
    `workshop_id` (falling back to `name` for a local mod with no workshop id) and
    `version`; missing parts become the empty string (so a mod with no version is
    still identified, just by id alone). Returns a short hex digest. The vanilla
    (empty) mod-set digests to a fixed sentinel so it is stable + recognisable.

    The identity token is NAMESPACED ``workshop:<id>`` vs ``name:<name>`` (schema
    review A): so a LOCAL mod named ``12345`` can never collide with a WORKSHOP mod
    whose ``remote_file_id`` is ``12345``. A mod with neither id nor name tokenises
    to ``name:`` (empty) -- still order-positioned, but the native matcher diffs the
    full list to surface such under-identified entries.

    The digest is taken over a CANONICAL JSON encoding of the ordered
    ``[[ident, version], ...]`` pairs: a JSON list cannot be spoofed by an ``@``/``|``
    inside a name/version the way a delimiter-joined string could, so two distinct
    mod-sets can never collide through delimiter ambiguity. NOTE (documented
    limitation, not a bug): identity is still metadata-only (id + version + order),
    so two LOCAL mods with the SAME name + version are indistinguishable here; the
    native matcher therefore also diffs the FULL mod list, not just this digest.
    """
    import hashlib
    import json
    if not mods:
        return "vanilla"
    pairs = []
    for m in mods:
        wid = (m.get("workshop_id") or "").strip()
        if wid:
            ident = "workshop:" + wid
        else:
            ident = "name:" + (m.get("name") or "").strip()
        ver = (m.get("version") or "").strip()
        pairs.append([ident, ver])
    raw = json.dumps(pairs, separators=(",", ":"), ensure_ascii=True)
    return hashlib.sha1(raw.encode("utf-8")).hexdigest()[:16]


def empty_modset() -> Dict[str, Any]:
    """A measured VANILLA mod-set (launcher config read, found zero enabled mods).

    Distinct from `provenance.mod = None` (launcher config NOT read): this is the
    honest "I looked and there are no mods" record. `mods` is empty; `modset_id`
    is the fixed `"vanilla"` sentinel.
    """
    return {
        "modset_id": "vanilla",
        "source": None,
        "load_order": [],
        "mods": [],
    }


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
        # top-level LIVE MIO catalog (§11b, M). NEW first-class live container,
        # distinct from the save-sourced per-country Country.mios. Always present;
        # null-but-present until a live read folds it.
        "mio_live": empty_mio_live(),
        # top-level LIVE equipment-MARKET sidecar (§7c, M). NEW first-class live container for
        # the global purchase contracts; distinct from the SAVE "market" domain (per-country
        # lend_lease + world list, a different identity model). null-but-present until a live read.
        "market_live": empty_market_live(),
        # top-level LIVE diplomacy sidecar (§13b, M). NEW first-class live container for the
        # world-scoped war_relations list + faction_members map; distinct from the SAVE
        # diplomacy_world (different identity model). null-but-present until a live read.
        "diplomacy_live": empty_diplomacy_live(),
        # top-level LIVE special-projects / facilities / scientists catalog (§11c, M).
        # NEW first-class live container, distinct from the save-sourced per-country
        # Country.projects / Country.scientists. null-but-present until a live read
        # folds it (the lists stay None offline so they never look measured-empty).
        "special_projects": empty_special_projects(),
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
    # top-level LIVE catalogs (§11b mio_live source M, §11c special_projects source C/M).
    # Always present (null-but-present block); validated as a dict so a malformed
    # payload can't pass as an ignored unknown key (schema: close the validator
    # blind spot — mio_live was missing here from a prior pass).
    "mio_live": dict,
    # top-level LIVE equipment-MARKET sidecar (§7c, market_live). Always present (null-but-
    # present block); validated as a dict so a malformed payload can't pass as an ignored key.
    "market_live": dict,
    "special_projects": dict,
    # top-level LIVE diplomacy sidecar (§13b, diplomacy_live). Always present
    # (null-but-present block); validated as a dict so a malformed payload can't pass
    # as an ignored unknown key.
    "diplomacy_live": dict,
    "naval_air": dict, "supply": dict,
    # Tier-1 EXTRACT world-scoped save domains (§14-§18)
    "strategic_air": dict, "strategic_operatives": dict, "raids": dict,
    "world_events": dict, "world": dict,
    # WAVE-2 first-class domains (§19-§20 + pairwise ledger)
    "weather": dict, "models": dict, "intel_ledger": dict,
}


def _validate_advisor_token_map(
    tag: str,
    politics: Dict[str, Any],
    field: str,
    problems: List[str],
) -> None:
    value = politics.get(field)
    if value is not None and not isinstance(value, dict):
        problems.append(f"country {tag!r}.politics.{field} {value!r} not dict|null")
        return
    if not isinstance(value, dict):
        return
    for bucket, tokens in value.items():
        if not valid_script_token(bucket):
            problems.append(
                f"country {tag!r}.politics.{field} key {bucket!r} not token"
            )
            continue
        if not isinstance(tokens, list):
            problems.append(f"country {tag!r}.politics.{field}.{bucket} not list")
            continue
        for token in tokens:
            if not valid_script_token(token):
                problems.append(
                    f"country {tag!r}.politics.{field}.{bucket} "
                    f"entry {token!r} not token"
                )
                break


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

    # provenance.mod (the additive per-replay MOD IDENTITY, §K): None (launcher
    # config not read — NO FABRICATION) or a ModSet dict. When a dict, the
    # identity fields must be the right kind so the native auto-matcher (which keys
    # off modset_id + the per-mod id/version/order) can trust it. A measured-vanilla
    # mod-set (mods == []) is valid and distinct from None.
    prov = snap.get("provenance")
    if isinstance(prov, dict):
        collector = prov.get("collector")
        if collector is not None:
            if not isinstance(collector, dict):
                problems.append(f"provenance.collector {collector!r} not dict|null")
            else:
                cap = collector.get("captured")
                if cap is not None and not isinstance(cap, bool):
                    problems.append(f"provenance.collector.captured {cap!r} not bool|null")
                domains = collector.get("domains")
                if domains is not None:
                    if not isinstance(domains, list):
                        problems.append(
                            f"provenance.collector.domains {domains!r} not list|null")
                    else:
                        for i, dom in enumerate(domains):
                            if not isinstance(dom, str):
                                problems.append(
                                    f"provenance.collector.domains[{i}] {dom!r} not str")
                            elif dom not in DOMAINS:
                                problems.append(
                                    f"provenance.collector.domains[{i}] {dom!r} not canonical")
                for k in ("backend", "collector_version", "exe_sha256", "mod_checksum",
                          "profile_id", "quality_status", "capture_boundary", "transport",
                          "transport_crc32", "session_id"):
                    v = collector.get(k)
                    if v is not None and not isinstance(v, str):
                        problems.append(f"provenance.collector.{k} {v!r} not str|null")
                seq = collector.get("sequence")
                if seq is not None and not isinstance(seq, int):
                    problems.append(f"provenance.collector.sequence {seq!r} not int|null")
                for k in ("session_sequence", "duration_ms", "sample_ms",
                          "elapsed_wall_ms"):
                    v = collector.get(k)
                    if v is not None and not isinstance(v, int):
                        problems.append(f"provenance.collector.{k} {v!r} not int|null")
                transport_bytes = collector.get("transport_bytes")
                if transport_bytes is not None and not isinstance(transport_bytes, int):
                    problems.append(
                        f"provenance.collector.transport_bytes {transport_bytes!r} "
                        "not int|null")

        mod = prov.get("mod")
        if mod is not None:
            if not isinstance(mod, dict):
                problems.append(f"provenance.mod {mod!r} not dict|null")
            else:
                if not isinstance(mod.get("modset_id"), str) or not mod.get("modset_id"):
                    problems.append("provenance.mod.modset_id missing/empty")
                mods = mod.get("mods")
                if not isinstance(mods, list):
                    problems.append(f"provenance.mod.mods {mods!r} not a list")
                else:
                    for i, m in enumerate(mods):
                        if not isinstance(m, dict):
                            problems.append(f"provenance.mod.mods[{i}] {m!r} not a dict")
                            continue
                        for k in ("name", "workshop_id", "version"):
                            v = m.get(k)
                            if v is not None and not isinstance(v, str):
                                problems.append(
                                    f"provenance.mod.mods[{i}].{k} {v!r} not str|null")
                lo = mod.get("load_order")
                if lo is not None and not isinstance(lo, list):
                    problems.append(f"provenance.mod.load_order {lo!r} not list|null")

    # Countries: keys look like TAGs; each has the 8 domains as dicts.
    for tag, c in (snap.get("countries") or {}).items():
        if not (isinstance(tag, str) and 2 <= len(tag) <= 4):
            problems.append(f"country key {tag!r} not a TAG-like string")
        if not isinstance(c, dict):
            problems.append(f"country {tag!r} not a dict")
            continue
        for dom in ("economy", "military", "politics", "research", "focus",
                    "production", "decisions", "diplomacy", "intel", "commanders",
                    "mios", "scheduled_variants"):
            if dom not in c:
                problems.append(f"country {tag!r} missing domain {dom!r}")
        # the always-present-but-nullable per-country deep fields must be the right kind
        if "order_of_battle" not in c:
            problems.append(f"country {tag!r} missing field 'order_of_battle'")
        if not isinstance(c.get("mios", []), list):
            problems.append(f"country {tag!r}.mios should be a list")
        # newly-wired live leaves: misc dict|null, military.convoys_count int|null
        # (schema review #8 — these were folded but not validated).
        if c.get("misc") is not None and not isinstance(c.get("misc"), dict):
            problems.append(f"country {tag!r}.misc {c.get('misc')!r} not dict|null")
        extra = c.get("extra")
        if extra is not None and not isinstance(extra, dict):
            problems.append(f"country {tag!r}.extra {extra!r} not dict|null")
        elif isinstance(extra, dict):
            sn = extra.get("strategic_navy")
            if sn is not None and not isinstance(sn, dict):
                problems.append(
                    f"country {tag!r}.extra.strategic_navy {sn!r} not dict|null")
            elif isinstance(sn, dict):
                for k in ("max_allowed_dockyards_to_repair",
                          "num_used_dockyards"):
                    v = sn.get(k)
                    if v is not None and not isinstance(v, int):
                        problems.append(
                            f"country {tag!r}.extra.strategic_navy.{k} "
                            f"{v!r} not int|null")
        econ = c.get("economy")
        if isinstance(econ, dict):
            fuel = econ.get("fuel")
            if fuel is not None and not isinstance(fuel, dict):
                problems.append(f"country {tag!r}.economy.fuel {fuel!r} not dict|null")
            elif isinstance(fuel, dict):
                for k in ("current", "max"):
                    v = fuel.get(k)
                    if v is not None:
                        if not _is_finite_number(v):
                            problems.append(
                                f"country {tag!r}.economy.fuel.{k} {v!r} "
                                "not number|null")
                        elif v < 0 or v > FUEL_MEASURE_ABS_MAX:
                            problems.append(
                                f"country {tag!r}.economy.fuel.{k} {v!r} "
                                "out of range")
                for k in ("gain", "cost"):
                    v = fuel.get(k)
                    if v is not None:
                        if not _is_finite_number(v):
                            problems.append(
                                f"country {tag!r}.economy.fuel.{k} {v!r} "
                                "not number|null")
                        elif abs(v) > FUEL_MEASURE_ABS_MAX:
                            problems.append(
                                f"country {tag!r}.economy.fuel.{k} {v!r} "
                                "out of range")
        mil = c.get("military")
        if isinstance(mil, dict):
            cc = mil.get("convoys_count")
            if cc is not None and not isinstance(cc, int):
                problems.append(
                    f"country {tag!r}.military.convoys_count {cc!r} not int|null")
        # newly-wired LIVE leaves (tech_live / commanders_live): list|null shape.
        rsh = c.get("research")
        if isinstance(rsh, dict):
            ipl = rsh.get("in_progress_live")
            if ipl is not None and not isinstance(ipl, list):
                problems.append(
                    f"country {tag!r}.research.in_progress_live {ipl!r} not list|null")
            tcc = rsh.get("tech_count")
            if tcc is not None and not isinstance(tcc, int):
                problems.append(
                    f"country {tag!r}.research.tech_count {tcc!r} not int|null")
            slots = rsh.get("research_slots")
            if slots is not None and not isinstance(slots, int):
                problems.append(
                    f"country {tag!r}.research.research_slots {slots!r} not int|null")
        cmd = c.get("commanders")
        if isinstance(cmd, dict):
            for k in ("army_leaders", "navy_leaders"):
                v = cmd.get(k)
                if v is not None and not isinstance(v, list):
                    problems.append(
                        f"country {tag!r}.commanders.{k} {v!r} not list|null")
        # intel_live agency: a dict|null whose live-measured capacity (if present) is
        # a dict|null (schema review: validate the newly-wired nested leaf).
        intl = c.get("intel")
        if isinstance(intl, dict):
            ag = intl.get("agency")
            if ag is not None and not isinstance(ag, dict):
                problems.append(
                    f"country {tag!r}.intel.agency {ag!r} not dict|null")
            elif isinstance(ag, dict):
                cap = ag.get("capacity")
                if cap is not None and not isinstance(cap, dict):
                    problems.append(
                        f"country {tag!r}.intel.agency.capacity {cap!r} not dict|null")
        # newly-wired LIVE focus leaf: current_continuous str|null (focus_live, M).
        fcs = c.get("focus")
        if isinstance(fcs, dict):
            cc = fcs.get("current_continuous")
            if cc is not None and not isinstance(cc, str):
                problems.append(
                    f"country {tag!r}.focus.current_continuous {cc!r} not str|null")
        # newly-wired LIVE production home (production_live, C/M): a dict whose measured
        # leaves are the right kind (stockpile dict|null, *_lines list|null, factory
        # counts int|null). schema review: validate the new always-present sub-dict so a
        # malformed live payload can't pass as an ignored key.
        prod = c.get("production")
        if "production" in c and not isinstance(prod, dict):
            problems.append(f"country {tag!r}.production {prod!r} not a dict")
        elif isinstance(prod, dict):
            sk = prod.get("stockpile")
            if sk is not None and not isinstance(sk, dict):
                problems.append(
                    f"country {tag!r}.production.stockpile {sk!r} not dict|null")
            elif isinstance(sk, dict):
                if len(sk) > 4096:
                    problems.append(
                        f"country {tag!r}.production.stockpile too many entries")
                for variant_id, amount in sk.items():
                    if not _valid_positive_int_key(
                        variant_id, max_value=PRODUCTION_VARIANT_ID_MAX
                    ):
                        problems.append(
                            f"country {tag!r}.production.stockpile key "
                            f"{variant_id!r} not valid positive id")
                    if not _valid_signed_measure(amount):
                        problems.append(
                            f"country {tag!r}.production.stockpile[{variant_id!r}] "
                            "not finite number in range")
            st_total = prod.get("stockpile_total")
            if st_total is not None and not _valid_signed_measure(st_total):
                problems.append(
                    f"country {tag!r}.production.stockpile_total {st_total!r} "
                    "not number|null")
            for k in ("military_lines", "naval_lines"):
                v = prod.get(k)
                if v is not None and not isinstance(v, list):
                    problems.append(
                        f"country {tag!r}.production.{k} {v!r} not list|null")
                elif isinstance(v, list):
                    if len(v) > 4096:
                        problems.append(
                            f"country {tag!r}.production.{k} too many entries")
                    for i, line in enumerate(v):
                        problems.extend(_validate_production_line(tag, k, i, line))
            for k in ("military_factories", "naval_factories"):
                v = prod.get(k)
                if v is not None and not _valid_bounded_int(
                    v, minimum=0, maximum=PRODUCTION_FACTORY_COUNT_MAX
                ):
                    problems.append(
                        f"country {tag!r}.production.{k} {v!r} "
                        "not nonnegative int|null")
            # newly-wired LIVE construction queue (construction_orders_live, M): list|null.
            cq = prod.get("construction_queue")
            if cq is not None and not isinstance(cq, list):
                problems.append(
                    f"country {tag!r}.production.construction_queue {cq!r} not list|null")
        # newly-wired LIVE diplomacy leaves (diplomacy_live, M): at_war list|null whose
        # elements are TAG strings; casualties int|null; faction_group int|null.
        dip = c.get("diplomacy")
        if isinstance(dip, dict):
            aw = dip.get("at_war")
            if aw is not None and not isinstance(aw, list):
                problems.append(
                    f"country {tag!r}.diplomacy.at_war {aw!r} not list|null")
            elif isinstance(aw, list):
                for e in aw:
                    if not (isinstance(e, str) and 2 <= len(e) <= 4):
                        problems.append(
                            f"country {tag!r}.diplomacy.at_war entry {e!r} not a TAG")
            cas = dip.get("casualties")
            if cas is not None and not isinstance(cas, int):
                problems.append(
                    f"country {tag!r}.diplomacy.casualties {cas!r} not int|null")
            fg = dip.get("faction_group")
            if fg is not None and not isinstance(fg, int):
                problems.append(
                    f"country {tag!r}.diplomacy.faction_group {fg!r} not int|null")
            # newly-wired LIVE bilateral-relations leaves (diplo_relations_live, M):
            # opinion + relation_flags are dict|null; overlord/autonomy_state str|null;
            # autonomy_ratio number|null. A malformed live payload can't slip through.
            for k in ("opinion", "relation_flags"):
                v = dip.get(k)
                if v is not None and not isinstance(v, dict):
                    problems.append(
                        f"country {tag!r}.diplomacy.{k} {v!r} not dict|null")
            for k in ("overlord", "autonomy_state"):
                v = dip.get(k)
                if v is not None and not isinstance(v, str):
                    problems.append(
                        f"country {tag!r}.diplomacy.{k} {v!r} not str|null")
            ar = dip.get("autonomy_ratio")
            if ar is not None and not isinstance(ar, (int, float)):
                problems.append(
                    f"country {tag!r}.diplomacy.autonomy_ratio {ar!r} not number|null")
            # newly-wired LIVE opinion-modifier decomposition (construction_orders_live, M):
            # dict|null.
            om = dip.get("opinion_modifiers")
            if om is not None and not isinstance(om, dict):
                problems.append(
                    f"country {tag!r}.diplomacy.opinion_modifiers {om!r} not dict|null")
        # newly-wired LIVE politics leaves (politics_live, M): party_popularities dict|null,
        # ideas/ideas_other/national_spirits list|null, laws dict|null,
        # idea_count/election_frequency/party_count int|null, low_confidence bool|null.
        # (political_power stays country_scalars-owned;
        # not re-validated here.)
        pol = c.get("politics")
        if isinstance(pol, dict):
            pp = pol.get("party_popularities")
            if pp is not None and not isinstance(pp, dict):
                problems.append(
                    f"country {tag!r}.politics.party_popularities {pp!r} not dict|null")
            laws = pol.get("laws")
            if laws is not None and not isinstance(laws, dict):
                problems.append(
                    f"country {tag!r}.politics.laws {laws!r} not dict|null")
            elif isinstance(laws, dict):
                for group, token in laws.items():
                    if not valid_script_token(group):
                        problems.append(
                            f"country {tag!r}.politics.laws key {group!r} not token")
                        continue
                    if token is not None and not valid_script_token(token):
                        problems.append(
                            f"country {tag!r}.politics.laws.{group} {token!r} not token")
            idl = pol.get("ideas")
            if idl is not None and not isinstance(idl, list):
                problems.append(
                    f"country {tag!r}.politics.ideas {idl!r} not list|null")
            elif isinstance(idl, list):
                for token in idl:
                    if not valid_script_token(token):
                        problems.append(
                            f"country {tag!r}.politics.ideas entry {token!r} not token")
                        break
            io = pol.get("ideas_other")
            if io is not None and not isinstance(io, list):
                problems.append(
                    f"country {tag!r}.politics.ideas_other {io!r} not list|null")
            elif isinstance(io, list):
                for token in io:
                    if not valid_script_token(token):
                        problems.append(
                            f"country {tag!r}.politics.ideas_other entry {token!r} not token")
                        break
            ns = pol.get("national_spirits")
            if ns is not None and not isinstance(ns, list):
                problems.append(
                    f"country {tag!r}.politics.national_spirits {ns!r} not list|null")
            elif isinstance(ns, list):
                for token in ns:
                    if not valid_script_token(token):
                        problems.append(
                            f"country {tag!r}.politics.national_spirits entry {token!r} not token")
                        break
            for field_name in ("idea_groups", "advisor_ideas"):
                problems.extend(_validate_token_list_map(
                    pol.get(field_name),
                    f"country {tag!r}.politics.{field_name}",
                ))
            for k in ("idea_count", "election_frequency", "party_count"):
                v = pol.get(k)
                if v is not None and not isinstance(v, int):
                    problems.append(
                        f"country {tag!r}.politics.{k} {v!r} not int|null")
            lc = pol.get("low_confidence")
            if lc is not None and not isinstance(lc, bool):
                problems.append(
                    f"country {tag!r}.politics.low_confidence {lc!r} not bool|null")
            # newly-wired LIVE election residuals (politics_residuals_live, M): last_election
            # str|null, elections_allowed bool|null, appointed advisor lists/maps|null.
            le = pol.get("last_election")
            if le is not None and not isinstance(le, str):
                problems.append(
                    f"country {tag!r}.politics.last_election {le!r} not str|null")
            ea = pol.get("elections_allowed")
            if ea is not None and not isinstance(ea, bool):
                problems.append(
                    f"country {tag!r}.politics.elections_allowed {ea!r} not bool|null")
            aa = pol.get("appointed_advisors")
            if aa is not None and not isinstance(aa, list):
                problems.append(
                    f"country {tag!r}.politics.appointed_advisors {aa!r} not list|null")
            aad = pol.get("appointed_advisor_details")
            if aad is not None and not isinstance(aad, list):
                problems.append(
                    f"country {tag!r}.politics.appointed_advisor_details {aad!r} "
                    "not list|null")
            elif isinstance(aad, list):
                for idx, row in enumerate(aad):
                    if not isinstance(row, dict):
                        problems.append(
                            f"country {tag!r}.politics.appointed_advisor_details[{idx}] "
                            "not dict")
                        continue
                    token = row.get("idea_token")
                    if not valid_script_token(token):
                        problems.append(
                            f"country {tag!r}.politics.appointed_advisor_details[{idx}]."
                            "idea_token not token")
                    for key in ("slot", "ledger", "character"):
                        value = row.get(key)
                        if value is not None and not valid_script_token(value):
                            problems.append(
                                f"country {tag!r}.politics."
                                f"appointed_advisor_details[{idx}].{key} not token|null")
            _validate_advisor_token_map(
                tag,
                pol,
                "appointed_advisor_slots",
                problems,
            )
            _validate_advisor_token_map(
                tag,
                pol,
                "appointed_advisor_ledgers",
                problems,
            )
        # newly-wired LIVE intel leaves (intel_deep_live + final_residuals_live, M):
        # operatives_live list|null, cryptology_live dict|null, operative_skill_xp dict|null.
        intl2 = c.get("intel")
        if isinstance(intl2, dict):
            ol = intl2.get("operatives_live")
            if ol is not None and not isinstance(ol, list):
                problems.append(
                    f"country {tag!r}.intel.operatives_live {ol!r} not list|null")
            cl2 = intl2.get("cryptology_live")
            if cl2 is not None and not isinstance(cl2, dict):
                problems.append(
                    f"country {tag!r}.intel.cryptology_live {cl2!r} not dict|null")
            osx = intl2.get("operative_skill_xp")
            if osx is not None and not isinstance(osx, dict):
                problems.append(
                    f"country {tag!r}.intel.operative_skill_xp {osx!r} not dict|null")
        # newly-wired LIVE decisions leaves (decisions_live, C; decisions_market_live, M):
        # status_present dict|null and timed_live list|null.
        dec = c.get("decisions")
        if isinstance(dec, dict):
            status_present = dec.get("status_present")
            if status_present is not None and not isinstance(status_present, dict):
                problems.append(
                    f"country {tag!r}.decisions.status_present {status_present!r} "
                    "not dict|null"
                )
            elif isinstance(status_present, dict):
                if len(status_present) > DECISION_STATUS_MAX_KEYS:
                    problems.append(
                        f"country {tag!r}.decisions.status_present too many entries")
                for key, value in status_present.items():
                    if not valid_script_token(key):
                        problems.append(
                            f"country {tag!r}.decisions.status_present key {key!r} "
                            "not token"
                        )
                        continue
                    if not _valid_bounded_int(
                        value,
                        minimum=0,
                        maximum=DECISION_STATUS_COUNT_MAX,
                    ):
                        problems.append(
                            f"country {tag!r}.decisions.status_present.{key} "
                            f"{value!r} not int in range"
                        )
            tlv = dec.get("timed_live")
            if tlv is not None and not isinstance(tlv, list):
                problems.append(
                    f"country {tag!r}.decisions.timed_live {tlv!r} not list|null")
            elif isinstance(tlv, list):
                if len(tlv) > DECISION_TIMED_LIVE_MAX_ENTRIES:
                    problems.append(
                        f"country {tag!r}.decisions.timed_live too many entries")
                for idx, row in enumerate(tlv):
                    prefix = f"country {tag!r}.decisions.timed_live[{idx}]"
                    if not isinstance(row, dict):
                        problems.append(f"{prefix} {row!r} not dict")
                        break
                    decision = row.get("decision")
                    if not valid_script_token(decision):
                        problems.append(f"{prefix}.decision {decision!r} not token")
                    days = row.get("days")
                    if not _valid_bounded_int(
                        days,
                        minimum=0,
                        maximum=DECISION_TIMED_DAYS_MAX,
                    ):
                        problems.append(f"{prefix}.days {days!r} not int in range")
                    state = row.get("state")
                    if not _valid_bounded_int(
                        state,
                        minimum=0,
                        maximum=DECISION_TIMED_STATE_MAX,
                    ):
                        problems.append(f"{prefix}.state {state!r} not int in range")
                    state_name = row.get("state_name")
                    if state_name is not None and not valid_script_token(state_name):
                        problems.append(
                            f"{prefix}.state_name {state_name!r} not token|null")
                    if problems and any(f"timed_live[{idx}]" in p for p in problems):
                        break
        st = c.get("politics", {}).get("stability")
        if st is not None and not (isinstance(st, (int, float)) and -0.01 <= st <= 1.01):
            problems.append(f"country {tag!r} stability {st!r} out of 0..1")

    # States: keys parse as ints; owner/controller TAG|null.
    for sid, stt in (snap.get("states") or {}).items():
        if not _is_intlike(sid):
            problems.append(f"state key {sid!r} not int-like")
        if not isinstance(stt, dict):
            problems.append(f"state {sid!r} not a dict"); continue
        for fld in ("owner", "controller"):
            v = stt.get(fld)
            if v is not None and not valid_tag(v):
                problems.append(f"state {sid!r}.{fld} {v!r} not TAG|null")

    # Provinces: keys int-like; controller TAG|null.
    for pid, prv in (snap.get("provinces") or {}).items():
        if not _is_intlike(pid):
            problems.append(f"province key {pid!r} not int-like")
        if isinstance(prv, dict):
            v = prv.get("controller")
            if v is not None and not valid_tag(v):
                problems.append(f"province {pid!r}.controller {v!r} not TAG|null")
            # newly-wired LIVE per-province buildings (buildings_live, M): dict|null.
            pb = prv.get("buildings")
            if pb is not None and not isinstance(pb, dict):
                problems.append(f"province {pid!r}.buildings {pb!r} not dict|null")
        else:
            problems.append(f"province {pid!r} not a dict")

    # Divisions: each needs an int div_id; identity (uid = id:type) must be unique.
    collector_domains = set(
        ((snap.get("provenance") or {}).get("collector") or {}).get("domains") or []
    )
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
        # newly-wired LIVE per-division equipment inventory (division_equipment_live, M):
        # equipment_live dict|null (DISTINCT from the save-owned equipment list).
        el = dv.get("equipment_live")
        if el is not None and not isinstance(el, dict):
            problems.append(f"division[{i}].equipment_live {el!r} not dict|null")
        if "divisions" in collector_domains:
            problems.extend(_validate_collector_division(dv, i))

    # Designs: division_templates is a list, equipment_variants a dict — both must be
    # present (always-present-container rule), §11.
    dsg = snap.get("designs")
    if isinstance(dsg, dict):
        if not isinstance(dsg.get("division_templates"), list):
            problems.append("designs.division_templates missing/not a list")
        if not isinstance(dsg.get("equipment_variants"), dict):
            problems.append("designs.equipment_variants missing/not a dict")
        # newly-wired LIVE variant module catalog (designs_mio_deep_live, M): dict|null.
        evl = dsg.get("equipment_variants_live")
        if evl is not None and not isinstance(evl, dict):
            problems.append(f"designs.equipment_variants_live {evl!r} not dict|null")

    # Order of battle: the canonical sub-lists are lists, by_country a dict — all
    # present (always-present-container rule), §12.
    oob = snap.get("order_of_battle")
    if isinstance(oob, dict):
        for k in ("army_groups", "armies", "fleets", "navy_theaters",
                  "air_wings", "theatres", "ships"):  # ships = LIVE per-ship list (#8)
            if not isinstance(oob.get(k), list):
                problems.append(f"order_of_battle.{k} missing/not a list")
        if not isinstance(oob.get("by_country"), dict):
            problems.append("order_of_battle.by_country missing/not a dict")
        # newly-wired LIVE battle-plan orders (construction_orders_live, M): list|null.
        bpo = oob.get("battle_plan_orders")
        if bpo is not None and not isinstance(bpo, list):
            problems.append(f"order_of_battle.battle_plan_orders {bpo!r} not list|null")

    # Top-level LIVE catalogs: validate the documented leaves so a malformed
    # payload can't slip through (schema review #8). mio_live.by_tag/by_id are
    # dict|null; special_projects.facilities/projects/scientists are list|null.
    ml = snap.get("mio_live")
    if isinstance(ml, dict):
        for k in ("by_tag", "by_id"):
            v = ml.get(k)
            if v is not None and not isinstance(v, dict):
                problems.append(f"mio_live.{k} {v!r} not dict|null")
    sp = snap.get("special_projects")
    if isinstance(sp, dict):
        for k in ("facilities", "projects", "scientists"):
            v = sp.get(k)
            if v is not None and not isinstance(v, list):
                problems.append(f"special_projects.{k} {v!r} not list|null")
    # top-level LIVE equipment-MARKET sidecar (market_live): contracts list|null, count int|null,
    # source str|null. A malformed payload can't slip through as an ignored key.
    mkl = snap.get("market_live")
    if isinstance(mkl, dict):
        ct = mkl.get("contracts")
        if ct is not None and not isinstance(ct, list):
            problems.append(f"market_live.contracts {ct!r} not list|null")
        cn = mkl.get("count")
        if cn is not None and not isinstance(cn, int):
            problems.append(f"market_live.count {cn!r} not int|null")
    # top-level LIVE diplomacy sidecar (diplomacy_live): war_relations list|null,
    # faction_members/subtype_warning dict|null. A malformed payload can't slip through.
    dl = snap.get("diplomacy_live")
    if isinstance(dl, dict):
        wr = dl.get("war_relations")
        if wr is not None and not isinstance(wr, list):
            problems.append(f"diplomacy_live.war_relations {wr!r} not list|null")
        for k in ("faction_members", "subtype_warning"):
            v = dl.get(k)
            if v is not None and not isinstance(v, dict):
                problems.append(f"diplomacy_live.{k} {v!r} not dict|null")

    # live weather domain (§19): regions/provinces dict|null, source str|null. A malformed
    # payload can't slip through as an ignored key (mirrors the mio_live/diplomacy_live checks).
    wx = snap.get("weather")
    if isinstance(wx, dict):
        for k in ("regions", "provinces", "global_rollup"):
            v = wx.get(k)
            if v is not None and not isinstance(v, dict):
                problems.append(f"weather.{k} {v!r} not dict|null")

    # Events: type/confidence/source from the enums.
    for i, ev in enumerate(snap.get("events") or []):
        problems.extend(_validate_event(ev, f"events[{i}]"))

    return problems


def _validate_token_list_map(value: Any, where: str) -> List[str]:
    if value is None:
        return []
    if not isinstance(value, dict):
        return [f"{where} {value!r} not dict|null"]
    problems: List[str] = []
    for key, tokens in value.items():
        if not valid_script_token(key):
            problems.append(f"{where} key {key!r} not token")
            continue
        if not isinstance(tokens, list):
            problems.append(f"{where}.{key} {tokens!r} not list")
            continue
        for token in tokens:
            if not valid_script_token(token):
                problems.append(f"{where}.{key} entry {token!r} not token")
                break
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


def _validate_collector_division(div: Dict[str, Any], index: int) -> List[str]:
    prefix = f"division[{index}]"
    problems: List[str] = []
    if _contains_forbidden_raw_key(div):
        problems.append(f"{prefix} contains raw pointer/tag-db diagnostic key")
    dtype = div.get("div_type")
    if not _valid_bounded_int(dtype, minimum=0, maximum=2_147_483_647):
        problems.append(f"{prefix}.div_type {dtype!r} not nonnegative int for collector")
    owner = div.get("owner")
    if owner is not None and not valid_tag(owner):
        problems.append(f"{prefix}.owner {owner!r} not TAG|null")
    for field_name in (
        "location_province", "state_id", "template_id", "template_type",
        "move_target_province", "movement_source_province",
    ):
        value = div.get(field_name)
        if value is not None and not _valid_bounded_int(
            value, minimum=0, maximum=2_147_483_647
        ):
            problems.append(f"{prefix}.{field_name} {value!r} not nonnegative int|null")
    for field_name in (
        "organisation", "strength", "experience", "move_progress", "supply_status",
    ):
        value = div.get(field_name)
        if value is not None and not _is_finite_number(value):
            problems.append(f"{prefix}.{field_name} {value!r} not finite number|null")
    for field_name in ("moving", "in_combat"):
        value = div.get(field_name)
        if value is not None and not isinstance(value, bool):
            problems.append(f"{prefix}.{field_name} {value!r} not bool|null")
    template_name = div.get("template_name")
    if template_name is not None and not isinstance(template_name, str):
        problems.append(f"{prefix}.template_name {template_name!r} not str|null")
    return problems


def _contains_forbidden_raw_key(value: Any) -> bool:
    if isinstance(value, dict):
        for key, child in value.items():
            if isinstance(key, str):
                lowered = key.lower()
                if (
                    lowered in {"addr", "address", "ptr", "pointer", "body_addr",
                                "tag_db_id"}
                    or lowered.endswith("_addr")
                    or lowered.endswith("_ptr")
                    or lowered.endswith("_tag_db_id")
                ):
                    return True
            if _contains_forbidden_raw_key(child):
                return True
    elif isinstance(value, list):
        return any(_contains_forbidden_raw_key(child) for child in value)
    return False


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


def _is_finite_number(value: Any) -> bool:
    return (
        not isinstance(value, bool)
        and isinstance(value, (int, float))
        and math.isfinite(float(value))
    )


def _valid_signed_measure(value: Any) -> bool:
    return (
        _is_finite_number(value)
        and abs(float(value)) <= PRODUCTION_FIXED_POINT_ABS_MAX
    )


def _valid_nonnegative_measure(value: Any) -> bool:
    return (
        _is_finite_number(value)
        and 0.0 <= float(value) <= PRODUCTION_FIXED_POINT_ABS_MAX
    )


def _valid_bounded_int(value: Any, *, minimum: int, maximum: int) -> bool:
    return (
        not isinstance(value, bool)
        and isinstance(value, int)
        and minimum <= value <= maximum
    )


def _valid_positive_int_key(value: Any, *, max_value: int) -> bool:
    if isinstance(value, bool):
        return False
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        return False
    return str(parsed) == str(value) and 1 <= parsed <= max_value


def _validate_production_line(
    tag: str,
    key: str,
    index: int,
    line: Any,
) -> List[str]:
    prefix = f"country {tag!r}.production.{key}[{index}]"
    if not isinstance(line, dict):
        return [f"{prefix} not a dict"]
    problems: List[str] = []
    line_id = line.get("id")
    if line_id is None:
        problems.append(f"{prefix}.id missing")
    elif not _valid_bounded_int(line_id, minimum=0, maximum=PRODUCTION_LINE_ID_MAX):
        problems.append(f"{prefix}.id not int in range")
    active = line.get("active_factories")
    if active is None:
        problems.append(f"{prefix}.active_factories missing")
    elif not _valid_bounded_int(
        active, minimum=0, maximum=PRODUCTION_FACTORY_COUNT_MAX
    ):
        problems.append(f"{prefix}.active_factories not int in range")
    for field in ("cost", "speed", "produced"):
        measured = line.get(field)
        if measured is None:
            problems.append(f"{prefix}.{field} missing")
        elif not _valid_nonnegative_measure(measured):
            problems.append(f"{prefix}.{field} not finite nonnegative number in range")
    amount = line.get("amount")
    if amount is None:
        problems.append(f"{prefix}.amount missing")
    elif not (
        amount == -1
        or _valid_bounded_int(
            amount, minimum=0, maximum=PRODUCTION_LINE_AMOUNT_MAX
        )
    ):
        problems.append(f"{prefix}.amount not -1 or nonnegative int in range")
    return problems


__all__ = [
    "SCHEMA_VERSION", "DOMAINS", "RESOURCE_KEYS", "BUILDING_KEYS",
    "COLLECTOR_DOMAIN_CONTRACTS", "FIELD_SOURCES", "source_of", "date_to_hour_index",
    "Economy", "Military", "Politics", "Research", "Focus", "Production",
    "Decisions",
    "Diplomacy", "Intel", "Scalars", "Commanders", "Country", "State", "Province",
    "Division", "Combat", "MapLayers", "Designs", "OrderOfBattle", "MioLive",
    "DiplomacyLive", "SpecialProjects",
    "NavalAir", "Supply", "Weather", "WeatherRegion", "WeatherProvince", "ModelsBlock",
    "StrategicAir", "StrategicOperatives", "Raids", "WorldEvents", "World",
    "Provenance", "Event", "make_event",
    "EVENT_TYPES", "CONFIDENCE_LEVELS", "EVENT_SOURCES",
    "empty_country", "empty_production",
    "empty_state", "empty_province", "empty_division",
    "empty_map", "empty_designs", "empty_order_of_battle", "empty_mio_live",
    "empty_market_live",
    "empty_diplomacy_live", "empty_special_projects",
    "empty_naval_air", "empty_supply", "empty_weather", "empty_models",
    "empty_snapshot",
    "empty_strategic_air", "empty_strategic_operatives", "empty_raids",
    "empty_world_events", "empty_world",
    "validate_snapshot", "validate_event", "div_uid",
]
