use collector_primitives::country_slots::MIN_COUNTRY_CELLS;

use crate::extended;
use crate::sampling;
use collector_map_capture::map_capture::MapCapacities;
use collector_primitives::country_slots::{raise, rows};
use engine_native::{Memory, cgame_state, country_memory::Cache};
mod navies;
pub mod source_demand;
pub mod table_demand;

pub(crate) use collector_extended_support::demand::{demand_air_game_state, demand_game_state};

pub(crate) const ENGINE_SLOT_CEILING: usize = 1 << 16;

pub fn observe(memory: &impl Memory, base: usize) -> Option<usize> {
    let slots = cgame_state::read_country_registry_size(memory, base)?.slots();
    (slots <= ENGINE_SLOT_CEILING).then_some(slots)
}

pub fn observe_saved_targets(
    memory: &impl Memory,
    base: usize,
) -> Result<Option<engine_scripts::saved_event_targets::Demand>, &'static str> {
    let Some(state) = demand_game_state(memory, base)? else {
        return Ok(None);
    };
    engine_scripts::saved_event_targets::demand(memory, base, state)
        .map(Some)
        .ok_or("saved target sizing demand unreadable or invalid")
}
pub fn observe_country_intel(
    memory: &impl Memory,
    base: usize,
) -> Result<Option<usize>, &'static str> {
    let Some(state) = demand_game_state(memory, base)? else {
        return Ok(None);
    };
    let rows = engine_native::ccountry_intel::native_rows(memory, base, state)
        .ok_or("country intel sizing census unreadable or invalid")?;
    Ok(Some(rows))
}

pub fn observe_railway_gun_names(
    memory: &impl Memory,
    base: usize,
) -> Result<Option<engine_railways::railway_gun_counter_inputs::NameDemand>, &'static str> {
    let Some(state) = demand_game_state(memory, base)? else {
        return Ok(None);
    };
    engine_railways::railway_gun_counter_inputs::name_demand(memory, base, state).map(Some)
}

pub struct MapDemand {
    pub states: Result<usize, &'static str>,
    pub provinces: Result<usize, &'static str>,
    pub(crate) rail_levels: Result<usize, &'static str>,
    pub carrier: Result<engine_native::cair_base::CarrierDemand, &'static str>,
    pub navies: navies::Demand,
}

impl MapDemand {
    pub fn map_refusals(&self) -> impl Iterator<Item = &'static str> {
        [
            self.states.err(),
            self.provinces.err(),
            self.rail_levels.err(),
            self.carrier.err(),
            self.carrier.as_ref().ok().and_then(|hint| hint.failure),
        ]
        .into_iter()
        .flatten()
    }
}

fn province_slots(memory: &impl Memory, state: usize) -> Result<usize, &'static str> {
    let root = state
        .checked_add(cgame_state::PROVINCES)
        .and_then(|address| engine_native::cpdx_array::Array::read(memory, address, 8))
        .ok_or("map sizing province census unreadable or invalid")?;
    if root.count < 2 {
        return Err("map sizing province census empty");
    }
    Ok(root.count as usize)
}

pub fn observe_map(memory: &impl Memory, base: usize) -> Result<Option<MapDemand>, &'static str> {
    let Some(state) = demand_game_state(memory, base)? else {
        return Ok(None);
    };
    let states = state
        .checked_add(cgame_state::STATES)
        .and_then(|address| engine_native::cpdx_array::Array::read(memory, address, 8))
        .map(|array| array.count as usize)
        .ok_or("map sizing state census unreadable or invalid");
    Ok(Some(MapDemand {
        states,
        provinces: province_slots(memory, state),
        rail_levels: engine_railways::crailway_manager::selected_inputs::demand(
            memory, base, state,
        ),
        carrier: engine_native::cair_base::carrier_demand(memory, base, state),
        navies: navies::observe(memory, base, state),
    }))
}

pub fn observe_presentation_census(
    memory: &impl Memory,
    base: usize,
) -> Result<Option<engine_native::carmy::PresentationDemand>, &'static str> {
    let Some(state) = demand_game_state(memory, base)? else {
        return Ok(None);
    };
    engine_native::carmy::presentation_demand(memory, base, state).map(Some)
}

pub fn observe_production_output(memory: &impl Memory, base: usize) -> Option<usize> {
    let observe = || {
        let Some(state) = demand_game_state(memory, base)? else {
            return Ok(None);
        };
        engine_production::line_output_sites::output_events_per_pass(memory, base, state).map(Some)
    };
    match observe() {
        Ok(events) => events,
        Err(reason) => {
            collector_output::line_output::refuse_sizing(reason);
            None
        }
    }
}
pub fn observe_land_details(
    memory: &impl Memory,
    base: usize,
) -> Result<Option<engine_native::cland_combat::DetailDemand>, &'static str> {
    let Some(state) = demand_game_state(memory, base)? else {
        return Ok(None);
    };
    engine_native::cland_combat::detail_demand(memory, base, state).map(Some)
}
pub fn observe_naval_details(
    memory: &impl Memory,
    base: usize,
) -> Result<Option<engine_naval_history::naval_combat_demand::Demand>, &'static str> {
    let Some(state) = demand_game_state(memory, base)? else {
        return Ok(None);
    };
    engine_naval_history::naval_combat_demand::demand(memory, base, state).map(Some)
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RaidArenas {
    pub type_tokens: usize,
    pub reservation_words: usize,
    pub path_rows: usize,
    pub facility_rows: usize,
    pub air_regions: usize,
    pub counteroperatives: usize,
    pub failures: [Option<&'static str>; 5],
}

pub fn observe_raid_arenas(
    memory: &impl Memory,
    base: usize,
) -> Result<Option<RaidArenas>, &'static str> {
    let Some(state) = demand_game_state(memory, base)? else {
        return Ok(None);
    };
    let parents = engine_native::craid_system::required_type_tokens(memory, base, state)?;
    let player = engine_visibility::cplayer_status_cache::required_tokens(memory, base, state);
    let reservation_words =
        engine_native::raid_launch_inputs::required_reservations(memory, base, state)?;
    let facility_rows = match engine_raid_inputs::raid_facilities::count(memory, base, state) {
        engine_native::walk::Outcome::Rows { len } => Ok(len),
        engine_native::walk::Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("raid facility sizing count overflow"),
        engine_native::walk::Outcome::Unavailable { reason } => Err(reason),
    };
    let air_regions = state
        .checked_add(cgame_state::STRATEGIC_REGIONS)
        .and_then(|at| engine_native::cpdx_array::Array::read(memory, at, 8))
        .map(|array| array.count as usize)
        .ok_or("raid region sizing header unreadable or invalid");
    let path_rows = {
        use engine_raid_inputs::raid_path_memo::{Dispatcher, required_rows};
        required_rows(memory, base, state, Dispatcher::Land)
            .zip(required_rows(memory, base, state, Dispatcher::Naval))
            .map(|(land, naval)| land.max(naval))
            .ok_or("raid path sizing header unreadable or invalid")
    };
    let counteroperatives =
        engine_raid_inputs::raid_intel_inputs::required_operatives(memory, base, state);
    Ok(Some(RaidArenas {
        type_tokens: parents
            .checked_add(player.unwrap_or(0))
            .ok_or("raid token sizing count overflow")?,
        reservation_words,
        path_rows: path_rows.unwrap_or(0),
        facility_rows: facility_rows.unwrap_or(0),
        air_regions: air_regions.unwrap_or(0),
        counteroperatives: counteroperatives.unwrap_or(0),
        failures: [
            player.err(),
            path_rows.err(),
            facility_rows.err(),
            air_regions.err(),
            counteroperatives.err(),
        ],
    }))
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CharacterDemand {
    pub characters: engine_native::ccharacter::Demand,
    pub unit_leader_bytes: usize,
    pub flags: usize,
    pub variables: collector_intel::intel::VariableDemand,
}

pub fn observe_characters(
    memory: &impl Memory,
    base: usize,
) -> Result<Option<CharacterDemand>, &'static str> {
    let Some(state) = demand_game_state(memory, base)? else {
        return Ok(None);
    };
    Ok(Some(CharacterDemand {
        characters: engine_native::ccharacter::demand(memory, base, state)?,
        unit_leader_bytes: engine_native::cunit_leader::text_demand(memory, base, state)?,
        flags: collector_intel::intel::script_flag_demand(memory, base, state)
            .ok_or("script flag demand unreadable")?,
        variables: collector_intel::intel::script_variable_demand(memory, base, state)
            .ok_or("script variable demand unreadable")?,
    }))
}

pub fn observe_reserved_visibility(
    memory: &impl Memory,
    base: usize,
) -> Result<Option<usize>, &'static str> {
    let Some(state) = demand_game_state(memory, base)? else {
        return Ok(None);
    };
    engine_native::crelation_status::reserved_visibility_demand(memory, base, state).map(Some)
}

pub fn observe_politics_demand(
    memory: &impl Memory,
    base: usize,
    slots: usize,
    frames: usize,
) -> Option<usize> {
    (|| {
        let mut records = slots
            .checked_mul(engine_events::politics_sites::DAILY_RECORDS)
            .ok_or("politics staging record demand overflow")?;
        if let Some(state) = demand_game_state(memory, base)? {
            records = records.max(engine_events::politics_sites::daily_record_demand(
                memory, base, state,
            )?);
        }
        records
            .checked_mul(frames.max(sampling::BUFFERS))
            .ok_or("politics staging backlog demand overflow")
    })()
    .ok()
}

pub(crate) fn observe_advisor_modifiers(
    memory: &impl Memory,
    base: usize,
) -> Result<Option<engine_native::cadvisor::ModifierDemand>, &'static str> {
    demand_game_state(memory, base).and_then(|state| {
        state
            .map(|state| engine_native::cadvisor::modifier_demand(memory, base, state))
            .transpose()
    })
}

pub(crate) fn observe_production_modifiers(
    memory: &impl Memory,
    base: usize,
) -> Result<Option<engine_production::production_modifiers::Demand>, &'static str> {
    demand_game_state(memory, base).and_then(|state| {
        state
            .map(|state| engine_production::production_modifiers::demand(memory, base, state))
            .transpose()
    })
}

pub(crate) fn observe_land_event_words(
    memory: &impl Memory,
    base: usize,
) -> Result<Option<usize>, &'static str> {
    demand_game_state(memory, base).and_then(|state| {
        state
            .map(|state| engine_land_inputs::land_events::word_demand(memory, base, state))
            .transpose()
    })
}

pub fn observe_fuel_unit_demand(
    memory: &impl Memory,
    base: usize,
) -> Result<Option<[usize; 12]>, &'static str> {
    demand_air_game_state(memory, base).and_then(|state| {
        state
            .map(|state| engine_fuel::fuel_demand::demand(memory, base, state))
            .transpose()
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoliticsDemand {
    pub war_overview: engine_native::cwar_relation::overview::Demand,
    pub(crate) active_characters: usize,
    pub(crate) retired_characters: usize,
    pub(crate) party_leaders: usize,
    pub(crate) civil_wars: usize,
    pub(crate) advisor_tokens: usize,
    pub(crate) advisor_token_bytes: usize,
    pub(crate) faction_advisors: usize,
    pub(crate) faction_advisor_bytes: usize,
    pub(crate) agency_name_bytes: usize,
    pub(crate) agency_upgrade_rows: usize,
    pub(crate) agency_upgrade_definitions: usize,
    pub(crate) operation_rows: usize,
    pub(crate) operation_assignments: usize,
    pub(crate) operation_description_bytes: usize,
    pub(crate) operation_parts: usize,
    pub(crate) operative_memberships: usize,
    pub(crate) operative_name_bytes: usize,
    pub(crate) operative_missions: usize,
    pub(crate) operative_mission_bytes: usize,
    pub(crate) strategic_operatives: usize,
    pub(crate) strategic_operative_entries: usize,
    pub(crate) available_advisors: usize,
    pub(crate) available_advisor_bytes: usize,
    pub(crate) advisor_modifiers: engine_native::cadvisor::ModifierDemand,
    pub(crate) country_rosters: engine_native::ccountry::RosterDemand,
    pub pride_name_bytes: usize,
    pub(crate) pride_name_failure: Option<&'static str>,
    optional_failures: [Option<&'static str>; 4],
    pub(crate) timed_ideas: engine_native::cpolitical_status::IdeaDemand,
    pub(crate) decision_random_items: engine_decisions::cdecision_status::RandomItemDemand,
    pub(crate) effect_rosters: engine_native::ccountry::EffectRosterDemand,
    pub(crate) captured_operatives: usize,
    pub(crate) occupied_countries: usize,
    pub(crate) reserved_naming_bytes: usize,
    pub(crate) original_tag_lists: engine_stability::original_tag_lists::Demand,
    pub(crate) country_array: usize,
    pub(crate) co_belligerents: usize,
    pub character_inputs: collector_characters::extended::character_inputs::Demand,
}

impl PoliticsDemand {
    pub fn failures(&self) -> impl Iterator<Item = &'static str> {
        [self.pride_name_failure]
            .into_iter()
            .chain(self.optional_failures)
            .flatten()
    }

    pub fn exceeds(&self, seen: &Self) -> bool {
        self.agency_name_bytes > seen.agency_name_bytes
            || self.agency_upgrade_rows > seen.agency_upgrade_rows
            || self.agency_upgrade_definitions > seen.agency_upgrade_definitions
            || self.operation_rows > seen.operation_rows
            || self.operation_assignments > seen.operation_assignments
            || self.operation_description_bytes > seen.operation_description_bytes
            || self.operation_parts > seen.operation_parts
            || self.operative_memberships > seen.operative_memberships
            || self.operative_name_bytes > seen.operative_name_bytes
            || self.operative_missions > seen.operative_missions
            || self.operative_mission_bytes > seen.operative_mission_bytes
            || self.strategic_operatives > seen.strategic_operatives
            || self.strategic_operative_entries > seen.strategic_operative_entries
            || self.war_overview.relations > seen.war_overview.relations
            || self.war_overview.owned > seen.war_overview.owned
            || self.war_overview.goals > seen.war_overview.goals
            || self.available_advisors > seen.available_advisors
            || self.available_advisor_bytes > seen.available_advisor_bytes
            || self.advisor_modifiers.rows > seen.advisor_modifiers.rows
            || self.advisor_modifiers.bytes > seen.advisor_modifiers.bytes
            || self.advisor_modifiers.tooltip_bytes > seen.advisor_modifiers.tooltip_bytes
            || self.timed_ideas.rows > seen.timed_ideas.rows
            || self.timed_ideas.bytes > seen.timed_ideas.bytes
            || self.decision_random_items.rows > seen.decision_random_items.rows
            || self.decision_random_items.bytes > seen.decision_random_items.bytes
            || self.effect_rosters.neighbors > seen.effect_rosters.neighbors
            || self.effect_rosters.policies > seen.effect_rosters.policies
            || self.captured_operatives > seen.captured_operatives
            || self.occupied_countries > seen.occupied_countries
            || self.reserved_naming_bytes > seen.reserved_naming_bytes
            || self.original_tag_lists.buckets > seen.original_tag_lists.buckets
            || self.original_tag_lists.members > seen.original_tag_lists.members
            || self.country_array > seen.country_array
            || self.co_belligerents > seen.co_belligerents
            || self.country_rosters.states > seen.country_rosters.states
            || self.country_rosters.leaders > seen.country_rosters.leaders
            || self.pride_name_bytes > seen.pride_name_bytes
            || self.active_characters > seen.active_characters
            || self.retired_characters > seen.retired_characters
            || self.party_leaders > seen.party_leaders
            || self.civil_wars > seen.civil_wars
            || self.advisor_tokens > seen.advisor_tokens
            || self.advisor_token_bytes > seen.advisor_token_bytes
            || self.faction_advisors > seen.faction_advisors
            || self.faction_advisor_bytes > seen.faction_advisor_bytes
            || self.character_inputs.exceeds(&seen.character_inputs)
    }

    pub fn max(self, other: Self) -> Self {
        Self {
            war_overview: engine_native::cwar_relation::overview::Demand {
                relations: self
                    .war_overview
                    .relations
                    .max(other.war_overview.relations),
                owned: self.war_overview.owned.max(other.war_overview.owned),
                goals: self.war_overview.goals.max(other.war_overview.goals),
            },
            agency_name_bytes: self.agency_name_bytes.max(other.agency_name_bytes),
            agency_upgrade_rows: self.agency_upgrade_rows.max(other.agency_upgrade_rows),
            agency_upgrade_definitions: self
                .agency_upgrade_definitions
                .max(other.agency_upgrade_definitions),
            operation_rows: self.operation_rows.max(other.operation_rows),
            operation_assignments: self.operation_assignments.max(other.operation_assignments),
            operation_description_bytes: self
                .operation_description_bytes
                .max(other.operation_description_bytes),
            operation_parts: self.operation_parts.max(other.operation_parts),
            operative_memberships: self.operative_memberships.max(other.operative_memberships),
            operative_name_bytes: self.operative_name_bytes.max(other.operative_name_bytes),
            operative_missions: self.operative_missions.max(other.operative_missions),
            operative_mission_bytes: self
                .operative_mission_bytes
                .max(other.operative_mission_bytes),
            strategic_operatives: self.strategic_operatives.max(other.strategic_operatives),
            strategic_operative_entries: self
                .strategic_operative_entries
                .max(other.strategic_operative_entries),
            available_advisors: self.available_advisors.max(other.available_advisors),
            available_advisor_bytes: self
                .available_advisor_bytes
                .max(other.available_advisor_bytes),
            advisor_modifiers: engine_native::cadvisor::ModifierDemand {
                rows: self
                    .advisor_modifiers
                    .rows
                    .max(other.advisor_modifiers.rows),
                bytes: self
                    .advisor_modifiers
                    .bytes
                    .max(other.advisor_modifiers.bytes),
                tooltip_bytes: self
                    .advisor_modifiers
                    .tooltip_bytes
                    .max(other.advisor_modifiers.tooltip_bytes),
            },
            timed_ideas: engine_native::cpolitical_status::IdeaDemand {
                rows: self.timed_ideas.rows.max(other.timed_ideas.rows),
                bytes: self.timed_ideas.bytes.max(other.timed_ideas.bytes),
            },
            decision_random_items: engine_decisions::cdecision_status::RandomItemDemand {
                rows: self
                    .decision_random_items
                    .rows
                    .max(other.decision_random_items.rows),
                bytes: self
                    .decision_random_items
                    .bytes
                    .max(other.decision_random_items.bytes),
            },
            effect_rosters: engine_native::ccountry::EffectRosterDemand {
                neighbors: self
                    .effect_rosters
                    .neighbors
                    .max(other.effect_rosters.neighbors),
                policies: self
                    .effect_rosters
                    .policies
                    .max(other.effect_rosters.policies),
            },
            captured_operatives: self.captured_operatives.max(other.captured_operatives),
            occupied_countries: self.occupied_countries.max(other.occupied_countries),
            reserved_naming_bytes: self.reserved_naming_bytes.max(other.reserved_naming_bytes),
            original_tag_lists: self.original_tag_lists.max(other.original_tag_lists),
            country_array: self.country_array.max(other.country_array),
            co_belligerents: self.co_belligerents.max(other.co_belligerents),
            country_rosters: engine_native::ccountry::RosterDemand {
                states: self
                    .country_rosters
                    .states
                    .max(other.country_rosters.states),
                leaders: self
                    .country_rosters
                    .leaders
                    .max(other.country_rosters.leaders),
            },
            pride_name_bytes: self.pride_name_bytes.max(other.pride_name_bytes),
            pride_name_failure: other.pride_name_failure,
            optional_failures: other.optional_failures,
            active_characters: self.active_characters.max(other.active_characters),
            retired_characters: self.retired_characters.max(other.retired_characters),
            party_leaders: self.party_leaders.max(other.party_leaders),
            civil_wars: self.civil_wars.max(other.civil_wars),
            advisor_tokens: self.advisor_tokens.max(other.advisor_tokens),
            advisor_token_bytes: self.advisor_token_bytes.max(other.advisor_token_bytes),
            faction_advisors: self.faction_advisors.max(other.faction_advisors),
            faction_advisor_bytes: self.faction_advisor_bytes.max(other.faction_advisor_bytes),
            character_inputs: self.character_inputs.max(other.character_inputs),
        }
    }
}

pub fn observe_politics(
    memory: &impl Memory,
    base: usize,
) -> Result<Option<PoliticsDemand>, &'static str> {
    let Some(state) = demand_game_state(memory, base)? else {
        return Ok(None);
    };
    let demand = |outcome: engine_native::walk::Outcome| match outcome {
        engine_native::walk::Outcome::Rows { len } => Ok(len),
        engine_native::walk::Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("politics demand row count overflow"),
        engine_native::walk::Outcome::Unavailable { reason } => Err(reason),
    };
    let tokens = engine_characters::advisor_tokens::count(memory, base, state);
    let advisor_tokens = demand(tokens.outcome)?;
    let factions = engine_native::cfaction::count_advisors(memory, base, state);
    let faction_advisors = demand(factions.outcome)?;
    let [active, retired] = engine_native::ccountry_characters::count_records(memory, base, state);
    let active_characters = demand(active)?;
    let retired_characters = demand(retired)?;
    let agencies = engine_native::cintelligence_agency::count(memory, base, state);
    demand(agencies.outcome)?;
    let operatives = engine_native::coperative_leader::count(memory, base, state);
    let operative_memberships = demand(operatives.outcome)?
        .checked_sub(operatives.refused_rows as usize)
        .ok_or("operative refusal count exceeds demand")?;
    let missions = collector_intel::intel::operative_mission_demand(memory, base, state);
    let operative_missions = demand(missions.outcome)?;
    let strategic = engine_intelligence::strategic_operatives::count(memory, base, state);
    let strategic_operatives = demand(strategic.outcome)?;
    let (available_advisors, available_advisor_bytes) =
        engine_native::ccountry_characters::demand_available(memory, base, state)?;
    let advisor_modifiers =
        collector_characters::extended::characters::advisors::live_modifier_demand();
    let mut pride_name_bytes = 0_usize;
    let mut pride_name_failure = None;
    let mut references = engine_native::creference_registry::ScopedResolver::new(base);
    let country_rosters = engine_native::ccountry::counter_roster_demand_with_pride_observer(
        memory,
        base,
        state,
        &mut |reference| {
            let observed = reference
                .and_then(|handle| pride_name_demand(memory, base, handle, &mut references))
                .and_then(|bytes| {
                    pride_name_bytes
                        .checked_add(bytes)
                        .ok_or("pride ship name sizing byte overflow")
                });
            match observed {
                Ok(bytes) => pride_name_bytes = bytes,
                Err(reason) => {
                    pride_name_failure.get_or_insert(reason);
                }
            }
        },
    )?;
    let upgrades = engine_native::agency_upgrade_levels::native_demand(memory, base, state)?;
    let operations = engine_intelligence::ccountry_operation_manager::demand(memory, base, state)?;
    let reserved_naming_bytes =
        engine_native::ccountry::read_country_naming_demand(memory, base, state);
    let original_tag_lists = engine_stability::original_tag_lists::demand(memory, state);
    let country_array = engine_stability::country_roster::demand(memory, base, state);
    let (co_belligerents, co_belligerent_failure) =
        collector_country_counters::country_counters::co_belligerent_demand(memory, base, state);
    Ok(Some(PoliticsDemand {
        war_overview: engine_native::cwar_relation::overview::demand(memory, base, state)?,
        agency_name_bytes: agencies.required_bytes,
        agency_upgrade_rows: upgrades.rows,
        agency_upgrade_definitions: upgrades.catalog,
        operation_rows: operations.rows,
        operation_assignments: operations.assignments,
        operation_description_bytes: operations.description_bytes,
        operation_parts: operations.parts,
        operative_memberships,
        operative_name_bytes: operatives.required_bytes,
        operative_missions,
        operative_mission_bytes: missions.required_bytes,
        strategic_operatives,
        strategic_operative_entries: strategic.required_entries,
        available_advisors,
        available_advisor_bytes,
        advisor_modifiers,
        country_rosters,
        pride_name_bytes,
        pride_name_failure,
        timed_ideas: engine_native::cpolitical_status::timed_idea_demand(memory, base, state)?,
        decision_random_items: engine_decisions::cdecision_status::random_item_demand(
            memory, base, state,
        )?,
        effect_rosters: engine_native::ccountry::counter_effect_roster_demand(memory, base, state)?,
        captured_operatives: engine_native::cintelligence_agency::captured_operative_demand(
            memory, base, state,
        )?,
        occupied_countries: engine_occupation::ccountry_occupation_status::occupied_demand(
            memory, base, state,
        )?,
        reserved_naming_bytes: reserved_naming_bytes.unwrap_or_default(),
        original_tag_lists: original_tag_lists.unwrap_or_default(),
        country_array: country_array.unwrap_or_default(),
        optional_failures: [
            reserved_naming_bytes.err(),
            original_tag_lists.err(),
            country_array.err(),
            co_belligerent_failure,
        ],
        co_belligerents,
        active_characters,
        retired_characters,
        party_leaders: engine_native::cpolitical_status::party_leader_demand(memory, base, state)?,
        civil_wars: demand(engine_native::ccountry::count_civil_wars(
            memory, base, state,
        ))?,
        advisor_tokens,
        advisor_token_bytes: tokens.required_bytes,
        faction_advisors,
        faction_advisor_bytes: factions.required_bytes,
        character_inputs: collector_characters::extended::character_inputs::Demand::observe(
            memory, base, state,
        )?,
    }))
}

fn pride_name_demand(
    memory: &impl Memory,
    base: usize,
    handle: u64,
    references: &mut engine_native::creference_registry::ScopedResolver,
) -> Result<usize, &'static str> {
    if handle == 0 {
        return Ok(0);
    }
    let Some(ship) = references.resolve(memory, handle)? else {
        return Ok(0);
    };
    engine_native::cship::cached_name_demand(memory, base, ship, handle)
}

#[derive(Clone, Copy, PartialEq)]
pub struct Sizing {
    pub core: [usize; 12],
    pub map: MapCapacities,
    pub extended: extended::Capacities,
    pub country_cells: usize,
    pub content: engine_dlc::cdlc_manager::Counts,
}

impl Sizing {
    pub fn for_slots(slots: usize) -> Self {
        let mut sizing = Self {
            core: sampling::INITIAL_CAPACITIES,
            map: MapCapacities::default(),
            extended: extended::Capacities::default(),
            country_cells: MIN_COUNTRY_CELLS,
            content: engine_dlc::cdlc_manager::Counts::default(),
        };
        sizing.raise_for_slots(slots);
        sizing.extended.raise_for_states(sizing.core[3]);
        sizing
    }

    pub fn raise_for_slots(&mut self, slots: usize) {
        sampling::raise_for_countries(&mut self.core, slots);
        self.map.raise_for_countries(slots);
        self.extended.raise_for_countries(slots);
        raise(
            &mut self.country_cells,
            Cache::cells_for_countries(slots, MIN_COUNTRY_CELLS),
        );
    }

    pub fn raise_for_map(&mut self, demand: &MapDemand) {
        if let Ok(levels) = demand.rail_levels {
            raise(&mut self.extended.naval_history.rail_levels, levels);
        }
        if let Ok(states) = demand.states {
            raise(&mut self.core[3], states);
        }
        self.extended.raise_for_states(self.core[3]);
        if let Ok(provinces) = demand.provinces {
            raise(&mut self.core[1], provinces.saturating_sub(1));
        }
        self.extended
            .raise_for_provinces(demand.provinces.unwrap_or(0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    struct ProvinceHeaderMemory {
        count: u32,
        reads: std::cell::RefCell<Vec<(usize, usize)>>,
    }

    impl Memory for ProvinceHeaderMemory {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            if address == STATE + cgame_state::PROVINCES && output.len() == 16 {
                output[..8].copy_from_slice(&(TABLE as u64).to_le_bytes());
                output[8..12].copy_from_slice(&self.count.to_le_bytes());
                output[12..16].copy_from_slice(&self.count.to_le_bytes());
                return true;
            }
            false
        }
    }

    #[test]
    fn province_slot_hint_reads_only_the_census_header() {
        let memory = ProvinceHeaderMemory {
            count: 5000,
            reads: Default::default(),
        };
        assert_eq!(province_slots(&memory, STATE), Ok(5000));
        assert_eq!(
            *memory.reads.borrow(),
            [(STATE + cgame_state::PROVINCES, 16)]
        );
        let empty = ProvinceHeaderMemory {
            count: 1,
            reads: Default::default(),
        };
        assert_eq!(
            province_slots(&empty, STATE),
            Err("map sizing province census empty")
        );
    }

    #[test]
    fn optional_political_hint_failures_preserve_floors_and_clear_after_recovery() {
        let seen = PoliticsDemand {
            reserved_naming_bytes: 300,
            country_array: 5000,
            ..Default::default()
        };
        let failed = PoliticsDemand {
            optional_failures: [
                Some("country naming sizing header changed"),
                None,
                None,
                None,
            ],
            ..Default::default()
        };
        let retained = seen.max(failed);
        assert_eq!(retained.reserved_naming_bytes, 300);
        assert_eq!(retained.country_array, 5000);
        assert_eq!(
            retained.failures().collect::<Vec<_>>(),
            ["country naming sizing header changed"]
        );
        assert!(
            retained
                .max(PoliticsDemand::default())
                .failures()
                .next()
                .is_none()
        );
    }

    #[test]
    fn failed_co_belligerent_hint_keeps_prior_floor_and_requests_diagnostics() {
        let seen = PoliticsDemand {
            co_belligerents: 5_000,
            ..Default::default()
        };
        let failed = PoliticsDemand {
            co_belligerents: 3,
            optional_failures: [None, None, None, Some("diplomacy vtable mismatch")],
            ..Default::default()
        };
        let retained = seen.max(failed);
        assert_eq!(retained.co_belligerents, 5_000);
        assert_eq!(
            retained.failures().collect::<Vec<_>>(),
            ["diplomacy vtable mismatch"]
        );
        assert!(
            retained
                .max(PoliticsDemand::default())
                .failures()
                .next()
                .is_none()
        );
    }

    #[test]
    fn politics_staging_demand_follows_the_frames_holding_undrained_hours() {
        let memory = engine_native::memory::SliceMemory {
            base: cgame_state::SINGLETON_RVA.0,
            bytes: &[0; 8],
        };
        for slots in [1_501, 5_001] {
            assert_eq!(
                observe_politics_demand(&memory, 0, slots, sampling::BUFFERS),
                Some(slots * 5 * sampling::BUFFERS)
            );
            assert_eq!(
                observe_politics_demand(&memory, 0, slots, 1),
                Some(slots * 5 * sampling::BUFFERS)
            );
            assert_eq!(
                observe_politics_demand(&memory, 0, slots, 13),
                Some(slots * 5 * 13)
            );
        }
        let unreadable = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        assert_eq!(observe_politics_demand(&unreadable, 0, 5_001, 13), None);
    }

    #[test]
    fn observed_raid_demand_presizes_raid_arenas() {
        let demand = RaidArenas {
            type_tokens: 5_400,
            reservation_words: 1_900,
            path_rows: 5_000,
            facility_rows: 10_000,
            counteroperatives: 5000 * 11,
            air_regions: 700,
            ..Default::default()
        };
        let mut sizing = Sizing::for_slots(440);
        let default_tokens = sizing.extended.projects.raid_type_tokens;
        assert!(default_tokens < demand.type_tokens);
        assert!(sizing.extended.projects.rows[12] < demand.path_rows);
        sizing.extended.raise_for_raid_arenas(demand);
        assert!(sizing.extended.projects.raid_type_tokens >= demand.type_tokens);
        assert!(sizing.extended.projects.rows[12] >= demand.path_rows);
        assert!(sizing.extended.projects.rows[16] >= demand.facility_rows);
        assert!(sizing.extended.air.scores.regions >= demand.air_regions);
        assert!(
            sizing
                .extended
                .raid_launch_inputs
                .holds_reservations(demand.reservation_words)
        );
        let before = sizing.extended;
        sizing.extended.raise_for_raid_arenas(RaidArenas::default());
        assert!(sizing.extended == before);
    }

    #[test]
    fn observed_politics_demand_reads_the_registry_and_only_raises() {
        assert_eq!(
            observe_politics(&world(), BASE),
            Err("character manager unreadable or null")
        );
        let demand = PoliticsDemand::default();
        let mut sizing = Sizing::for_slots(COUNTRIES + 1);
        let before = sizing.extended;
        // SAFETY: Attach, session sizing or the isolated fixture owns the advisor demand update.
        unsafe { sizing.extended.raise_for_politics(demand) };
        assert!(sizing.extended == before);
        let intel = PoliticsDemand {
            agency_upgrade_rows: 5_000 * 17,
            agency_upgrade_definitions: 9_001,
            operation_rows: 5_000 * 3,
            operation_assignments: 5_000 * 7,
            operation_description_bytes: 5_000 * 400,
            operative_missions: 5_000 * 4,
            operative_mission_bytes: 5_000 * 4 * 24,
            operation_parts: 5_000 * 3 * 12,
            strategic_operatives: 5_000,
            strategic_operative_entries: 5_000 * 40,
            ..Default::default()
        };
        assert!(intel.exceeds(&PoliticsDemand::default()));
        assert_eq!(intel.max(PoliticsDemand::default()), intel);
        // SAFETY: Attach, session sizing or the isolated fixture owns the advisor demand update.
        unsafe { sizing.extended.raise_for_politics(intel) };
        assert!(sizing.extended.intel.rows[17] >= intel.agency_upgrade_rows);
        assert!(
            sizing.extended.intel.agency_upgrade_definitions >= intel.agency_upgrade_definitions
        );
        assert!(sizing.extended.intel.rows[9] >= intel.operation_rows);
        assert!(sizing.extended.intel.rows[10] >= intel.operation_assignments);
        assert!(sizing.extended.intel.operation_descriptions >= intel.operation_description_bytes);
        assert!(sizing.extended.intel.rows[18] >= intel.operative_missions);
        assert!(sizing.extended.intel.operative_mission_bytes >= intel.operative_mission_bytes);
        assert!(sizing.extended.intel.operation_parts >= intel.operation_parts);
        assert!(sizing.extended.intel.rows[19] >= intel.strategic_operatives);
        assert!(
            sizing.extended.intel.strategic_operative_entries >= intel.strategic_operative_entries
        );
        let before = sizing.extended;
        // SAFETY: Attach, session sizing or the isolated fixture owns the advisor demand update.
        unsafe {
            sizing
                .extended
                .raise_for_politics(PoliticsDemand::default())
        };
        assert!(sizing.extended == before);
        // SAFETY: Attach, session sizing or the isolated fixture owns the advisor demand update.
        unsafe {
            sizing.extended.raise_for_politics(PoliticsDemand {
                active_characters: 1 << 20,
                ..Default::default()
            })
        };
        assert!(sizing.extended != before);
        let before = sizing.extended;
        // SAFETY: Attach, session sizing or the isolated fixture owns the advisor demand update.
        unsafe {
            sizing.extended.raise_for_politics(PoliticsDemand {
                party_leaders: 1 << 20,
                ..Default::default()
            })
        };
        assert!(sizing.extended != before);
        let before = sizing.extended;
        // SAFETY: Attach, session sizing or the isolated fixture owns the advisor demand update.
        unsafe {
            sizing.extended.raise_for_politics(PoliticsDemand {
                civil_wars: 1 << 20,
                ..Default::default()
            })
        };
        assert!(sizing.extended != before);
        let before = sizing.extended;
        // SAFETY: Attach, session sizing or the isolated fixture owns the advisor demand update.
        unsafe {
            sizing.extended.raise_for_politics(PoliticsDemand {
                retired_characters: 1 << 20,
                ..Default::default()
            })
        };
        assert!(sizing.extended != before);
        let before = sizing.extended;
        // SAFETY: Attach, session sizing or the isolated fixture owns the advisor demand update.
        unsafe {
            sizing.extended.raise_for_politics(PoliticsDemand {
                character_inputs: collector_characters::extended::character_inputs::Demand {
                    traits: 1 << 20,
                    ..Default::default()
                },
                ..Default::default()
            })
        };
        assert!(sizing.extended != before);
        let seen = PoliticsDemand {
            war_overview: Default::default(),
            agency_name_bytes: 0,
            agency_upgrade_rows: 0,
            agency_upgrade_definitions: 0,
            operation_rows: 0,
            operation_assignments: 0,
            operation_description_bytes: 0,
            operative_memberships: 0,
            operative_name_bytes: 0,
            active_characters: 5,
            retired_characters: 1,
            party_leaders: 3,
            civil_wars: 1,
            advisor_tokens: 4,
            advisor_token_bytes: 40,
            faction_advisors: 2,
            faction_advisor_bytes: 20,
            character_inputs: Default::default(),
            ..Default::default()
        };
        let before = sizing.extended;
        // SAFETY: Attach, session sizing or the isolated fixture owns the advisor demand update.
        unsafe {
            sizing.extended.raise_for_politics(PoliticsDemand {
                faction_advisors: 1 << 16,
                faction_advisor_bytes: 1 << 20,
                ..Default::default()
            })
        };
        assert!(sizing.extended != before);
        let before = sizing.extended;
        // SAFETY: Attach, session sizing or the isolated fixture owns the advisor demand update.
        unsafe {
            sizing.extended.raise_for_politics(PoliticsDemand {
                advisor_tokens: 1 << 16,
                advisor_token_bytes: 1 << 20,
                ..Default::default()
            })
        };
        assert!(sizing.extended != before);
        assert!(!seen.exceeds(&seen) && seen.max(demand) == seen);
        let modifiers = PoliticsDemand {
            advisor_modifiers: engine_native::cadvisor::ModifierDemand {
                rows: 15_000,
                bytes: 1_000_000,
                tooltip_bytes: 2_000_000,
            },
            ..Default::default()
        };
        assert!(modifiers.exceeds(&seen));
        assert_eq!(
            seen.max(modifiers).advisor_modifiers,
            modifiers.advisor_modifiers
        );
    }

    #[test]
    fn demand_observers_distinguish_a_menu_from_an_unreadable_game_state() {
        let missing = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        assert_eq!(
            observe_politics(&missing, 0),
            Err("input demand game state unreadable or address overflow")
        );
        assert_eq!(
            observe_characters(&missing, 0),
            Err("input demand game state unreadable or address overflow")
        );
        let null = [0; 8];
        let menu = engine_native::memory::SliceMemory {
            base: cgame_state::SINGLETON_RVA.0,
            bytes: &null,
        };
        assert_eq!(observe_politics(&menu, 0), Ok(None));
        assert_eq!(observe_characters(&menu, 0), Ok(None));
        assert!(observe_characters(&world(), BASE).is_err());
    }

    #[test]
    fn zero_pride_hint_skips_registry_and_positive_unreadable_ref_is_not_empty() {
        struct Unreadable(std::cell::Cell<usize>);
        impl Memory for Unreadable {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                self.0.set(self.0.get() + 1);
                false
            }
        }
        let memory = Unreadable(std::cell::Cell::new(0));
        let mut references = engine_native::creference_registry::ScopedResolver::new(0);
        assert_eq!(pride_name_demand(&memory, 0, 0, &mut references), Ok(0));
        assert_eq!(memory.0.get(), 0);
        assert_eq!(
            pride_name_demand(&memory, 0, (3 << 32) | 51, &mut references),
            Err("reference root unreadable")
        );
        assert_eq!(memory.0.get(), 1);
    }

    #[test]
    fn failed_pride_hint_keeps_known_floor_and_independent_roster_growth() {
        let seen = PoliticsDemand {
            pride_name_bytes: 50000,
            country_rosters: engine_native::ccountry::RosterDemand {
                states: 30,
                leaders: 20,
            },
            ..Default::default()
        };
        let failed = PoliticsDemand {
            pride_name_failure: Some("ship cached name header invalid"),
            country_rosters: engine_native::ccountry::RosterDemand {
                states: 60,
                leaders: 20,
            },
            ..Default::default()
        };
        assert!(failed.exceeds(&seen));
        let combined = seen.max(failed);
        assert_eq!(combined.pride_name_bytes, 50000);
        assert_eq!(combined.pride_name_failure, failed.pride_name_failure);
        assert_eq!(combined.country_rosters.states, 60);
        let recovered = PoliticsDemand {
            pride_name_bytes: 3,
            ..Default::default()
        };
        assert!(!recovered.exceeds(&combined));
        let retained = combined.max(recovered);
        assert_eq!(retained.pride_name_bytes, 50000);
        assert_eq!(retained.pride_name_failure, None);
        assert_eq!(retained.country_rosters.states, 60);
        let mut capacity = collector_country_counters::country_counters::Capacities::default();
        let initial = capacity;
        capacity.raise_for_pride_names(retained.pride_name_bytes);
        assert!(capacity != initial);
        let grown = capacity;
        capacity.raise_for_pride_names(0);
        assert!(capacity == grown);
    }

    #[test]
    fn fuel_sizing_waits_for_the_late_air_manager_and_its_country_slots() {
        struct Loading {
            manager: usize,
            slots: u32,
        }
        impl Memory for Loading {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let pointer = match address {
                    address if address == cgame_state::SINGLETON_RVA.0 => Some(0x1000_u64),
                    address if address == 0x1000 + cgame_state::STRATEGIC_AIR_MANAGER => {
                        Some(self.manager as u64)
                    }
                    _ => None,
                };
                if let Some(pointer) = pointer {
                    output.copy_from_slice(&pointer.to_le_bytes());
                    return true;
                }
                let count = if address == 0x1000 + cgame_state::COUNTRIES {
                    5_001
                } else if self.manager != 0
                    && address == self.manager + engine_native::cstrategic_air_manager::COUNTRY_AIR
                {
                    self.slots
                } else {
                    return false;
                };
                let mut header = [0; 16];
                header[..8].copy_from_slice(&0x9000_u64.to_le_bytes());
                header[8..12].copy_from_slice(&count.to_le_bytes());
                header[12..].copy_from_slice(&count.to_le_bytes());
                output.copy_from_slice(&header);
                true
            }
        }
        for loading in [
            Loading {
                manager: 0,
                slots: 0,
            },
            Loading {
                manager: 0x8000,
                slots: 0,
            },
            Loading {
                manager: 0x8000,
                slots: 2,
            },
        ] {
            assert_eq!(demand_air_game_state(&loading, 0), Ok(None));
            assert_eq!(observe_fuel_unit_demand(&loading, 0), Ok(None));
        }
        let loaded = Loading {
            manager: 0x8000,
            slots: 5_001,
        };
        assert_eq!(demand_air_game_state(&loaded, 0), Ok(Some(0x1000)));
    }

    use engine_native::{
        ccountry, ccountry_intel, cgame_state as gs, country_intel_row, msvc_string, walk::Outcome,
    };

    const COUNTRIES: usize = 5_000;
    const BASE: usize = 0x1_0000_0000;
    const STATE: usize = 0x10_0000;
    const TABLE: usize = 0x20_0000;
    const LINKS: usize = 0x30_0000;
    const TAGS: usize = 0x40_0000;
    const DATABASE: usize = 0x50_0000;
    const DIPLOMACY: usize = 0x60_0000;
    const OBJECTS: usize = 0x100_0000;
    const OBJECT: usize = 0x2000;
    const INTEL_TABLES: usize = 0x5000_0000;
    const INTEL_RECORDS: usize = 0x6000_0000;
    const BLOC_LINKS: usize = 0x7000_0000;
    const AIR_MANAGER: usize = 0x80_0000;
    const AIR_ROOTS: usize = 0x90_0000;
    const AIR_OBJECTS: usize = 0x8000_0000;
    const AIR_OBJECT: usize = 0x200;
    const AIR_TABLES: usize = 0x9000_0000;
    const HISTORIES: usize = 0xa000_0000;
    const WINDOWS: usize = 0xb000_0000;
    const RECORD_SLOTS: usize = 0xc000_0000;
    const RECORDS: usize = 0x20_0000_0000;
    const VALUES: usize = 0x30_0000_0000;
    const REGION_SLOTS: usize = 256;
    const HISTORY: usize = 0x40;
    const RECORD: usize = 0x20;
    const VALUE_FIELDS: usize = 0x300;
    const TACTIC_DATABASE: usize = 0x2_0000_0000;
    const TACTIC_ENTRIES: usize = 0x2_0000_1000;
    const TACTIC_OBJECTS: usize = 0x2_0001_0000;
    const TACTIC_OBJECT: usize = 0x100;
    const DOCTRINE_SYSTEM: usize = 0x2_0100_0000;
    const DOCTRINE_ROWS: usize = 0x2_0200_0000;
    const UNLOCK_LISTS: usize = 0x2_0400_0000;
    const TACTICS: usize = 64;
    const UNLOCKS: usize = 8;

    struct World {
        slots: usize,
        intel: bool,
        bloc: Option<(u32, u32)>,
        air: Option<AirWorld>,
        relocated: Option<usize>,
    }

    struct AirWorld {
        every: usize,
        regions: Vec<u32>,
        ring: usize,
        current: u64,
    }

    impl AirWorld {
        fn holds(&self, slot: usize) -> bool {
            slot % self.every == slot / self.every % self.every
        }

        fn has(&self, slot: usize, region: usize) -> bool {
            self.holds(slot) && self.regions.contains(&(region as u32))
        }
    }

    impl World {
        fn tactic_byte(&self, address: usize) -> Option<u8> {
            use engine_land_inputs::country_tactic_unlocks as doctrine;
            use engine_native::ccombat_tactic as tactic;
            let word = |at: usize, value: u64| (value >> ((address - at) * 8)) as u8;
            let header = |at: usize, pointer: usize, count: usize| match address - at {
                0..8 => word(at, pointer as u64),
                8..12 => word(at + 8, count as u64),
                _ => word(at + 12, count as u64),
            };
            let within = |at: usize, len: usize| (at..at + len).contains(&address);
            let singleton = BASE + tactic::DATABASE_RVA.0;
            if within(singleton, 8) {
                return Some(word(singleton, TACTIC_DATABASE as u64));
            }
            let entries = TACTIC_DATABASE + tactic::DATABASE_ENTRIES;
            if within(entries, 16) {
                return Some(header(entries, TACTIC_ENTRIES, TACTICS + 1));
            }
            if within(TACTIC_ENTRIES, (TACTICS + 1) * 8) {
                let index = (address - TACTIC_ENTRIES) / 8;
                let object = TACTIC_OBJECTS + index * TACTIC_OBJECT;
                return Some(word(TACTIC_ENTRIES + index * 8, object as u64));
            }
            if within(TACTIC_OBJECTS, (TACTICS + 1) * TACTIC_OBJECT) {
                let index = (address - TACTIC_OBJECTS) / TACTIC_OBJECT;
                let object = TACTIC_OBJECTS + index * TACTIC_OBJECT;
                let vtable = match index {
                    0 => engine_native::cnull_combat_tactic::VTABLE_RVA.0,
                    _ => tactic::VTABLE_RVA.0,
                };
                return Some(match address - object {
                    0..8 => word(object, (BASE + vtable) as u64),
                    o if (tactic::ID..tactic::ID + 4).contains(&o) => {
                        word(object + tactic::ID, index as u64)
                    }
                    _ => 0,
                });
            }
            let system = STATE + doctrine::DOCTRINE_SYSTEM;
            if within(system, 8) {
                return Some(word(system, DOCTRINE_SYSTEM as u64));
            }
            let rows = DOCTRINE_SYSTEM + doctrine::DOCTRINE_ROWS;
            if within(rows, 16) {
                return Some(header(rows, DOCTRINE_ROWS, self.slots));
            }
            if within(DOCTRINE_ROWS, self.slots * doctrine::DOCTRINE_STRIDE) {
                let slot = (address - DOCTRINE_ROWS) / doctrine::DOCTRINE_STRIDE;
                let row = DOCTRINE_ROWS + slot * doctrine::DOCTRINE_STRIDE;
                let (tag, list) = (row + doctrine::ROW_TAG, row + doctrine::TACTICS);
                return Some(if within(list, 16) {
                    header(list, UNLOCK_LISTS + slot * UNLOCKS * 8, UNLOCKS)
                } else if within(tag, 4) {
                    word(tag, slot as u64)
                } else {
                    0
                });
            }
            if within(UNLOCK_LISTS, self.slots * UNLOCKS * 8) {
                let entry = (address - UNLOCK_LISTS) / 8;
                let object = TACTIC_OBJECTS + (1 + entry % TACTICS) * TACTIC_OBJECT;
                return Some(word(UNLOCK_LISTS + entry * 8, object as u64));
            }
            None
        }

        fn byte(&self, address: usize) -> u8 {
            if let Some(byte) = self.tactic_byte(address) {
                return byte;
            }
            let slots = self.slots as u64;
            let word = |at: usize, value: u64| (value >> ((address - at) * 8)) as u8;
            let header = |at: usize, pointer: usize| match address - at {
                0..8 => word(at, pointer as u64),
                8..12 => word(at + 8, slots),
                _ => word(at + 12, slots),
            };
            let singleton = BASE + gs::SINGLETON_RVA.0;
            if (singleton..singleton + 8).contains(&address) {
                return word(singleton, STATE as u64);
            }
            let database = BASE + engine_native::ccountry_database::SINGLETON_RVA.0;
            if (database..database + 8).contains(&address) {
                return word(database, DATABASE as u64);
            }
            let boundary = DATABASE + engine_native::ccountry_database::DYNAMIC_TAG_BOUNDARY;
            if (boundary..boundary + 4).contains(&address) {
                return word(boundary, slots);
            }
            if let Some(air) = &self.air
                && let Some(value) = self.air_byte(air, address)
            {
                return value;
            }
            if (DIPLOMACY..DIPLOMACY + 8).contains(&address) {
                return word(
                    DIPLOMACY,
                    (BASE + engine_native::cdiplomacy_status::VTABLE_RVA.0) as u64,
                );
            }
            if let Some((enemies, allies)) = self.bloc {
                for (field, count) in [
                    (engine_native::cdiplomacy_status::ENEMY_COUNTRIES, enemies),
                    (engine_native::cdiplomacy_status::WARTIME_ALLIES, allies),
                ] {
                    let at = DIPLOMACY + field;
                    if (at..at + 16).contains(&address) {
                        return match address - at {
                            0..8 => word(at, BLOC_LINKS as u64),
                            8..12 => word(at + 8, u64::from(count)),
                            _ => word(at + 12, u64::from(count)),
                        };
                    }
                }
                if (BLOC_LINKS..BLOC_LINKS + self.slots * 4).contains(&address) {
                    let index = (address - BLOC_LINKS) / 4;
                    return word(BLOC_LINKS + index * 4, index as u64 + 1);
                }
            }
            for (field, pointer) in [
                (gs::COUNTRIES, TABLE),
                (gs::COUNTRY_LINKS, LINKS),
                (gs::COUNTRY_TAGS, TAGS),
            ] {
                if (STATE + field..STATE + field + 16).contains(&address) {
                    return header(STATE + field, pointer);
                }
            }
            if (TABLE + 8..TABLE + self.slots * 8).contains(&address) {
                let slot = (address - TABLE) / 8;
                let object = if self.relocated == Some(slot) {
                    self.slots
                } else {
                    slot
                };
                return word(TABLE + slot * 8, (OBJECTS + object * OBJECT) as u64);
            }
            if (LINKS + 4..LINKS + self.slots * 4).contains(&address) {
                let id = (address - LINKS) / 4;
                return word(LINKS + id * 4, id as u64);
            }
            let objects = self.slots + usize::from(self.relocated.is_some());
            if (OBJECTS + OBJECT..OBJECTS + objects * OBJECT).contains(&address) {
                let index = (address - OBJECTS) / OBJECT;
                let object = OBJECTS + index * OBJECT;
                let slot = match self.relocated {
                    Some(slot) if index == self.slots => slot,
                    _ => index,
                };
                let offset = address - object;
                let capacity = ccountry::COSMETIC_TAG + msvc_string::CAPACITY;
                let culture = ccountry::GRAPHICAL_CULTURE + msvc_string::CAPACITY;
                let culture_2d = ccountry::GRAPHICAL_CULTURE_2D + msvc_string::CAPACITY;
                return match offset {
                    0..8 => word(object, (BASE + ccountry::VTABLE_RVA.0) as u64),
                    o if (ccountry::ID..ccountry::ID + 4).contains(&o) => {
                        word(object + ccountry::ID, slot as u64)
                    }
                    o if (capacity..capacity + 8).contains(&o) => {
                        word(object + capacity, msvc_string::INLINE_CAPACITY)
                    }
                    o if (culture..culture + 8).contains(&o) => {
                        word(object + culture, msvc_string::INLINE_CAPACITY)
                    }
                    o if (culture_2d..culture_2d + 8).contains(&o) => {
                        word(object + culture_2d, msvc_string::INLINE_CAPACITY)
                    }
                    o if (ccountry::DIPLOMACY_STATUS..ccountry::DIPLOMACY_STATUS + 8)
                        .contains(&o) =>
                    {
                        word(object + ccountry::DIPLOMACY_STATUS, DIPLOMACY as u64)
                    }
                    o if self.intel
                        && (ccountry::INTEL_TABLE..ccountry::INTEL_TABLE + 8).contains(&o) =>
                    {
                        word(
                            object + ccountry::INTEL_TABLE,
                            (INTEL_TABLES + slot * 0x20) as u64,
                        )
                    }
                    _ => 0,
                };
            }
            if self.intel && (INTEL_TABLES..INTEL_TABLES + self.slots * 0x20).contains(&address) {
                let table = INTEL_TABLES + (address - INTEL_TABLES) / 0x20 * 0x20;
                return match address - table {
                    0..8 => word(table, (BASE + ccountry_intel::VTABLE_RVA.0) as u64),
                    0x10..0x18 => word(table + 0x10, INTEL_RECORDS as u64),
                    0x18..0x1c => word(table + 0x18, slots),
                    0x1c..0x20 => word(table + 0x1c, slots),
                    _ => 0,
                };
            }
            if self.intel && self.in_records(address) {
                return 1;
            }
            0
        }

        fn air_byte(&self, air: &AirWorld, address: usize) -> Option<u8> {
            let word = |at: usize, value: u64| (value >> ((address - at) * 8)) as u8;
            let header = |at: usize, data: usize, count: usize| match address - at {
                0..8 => word(at, data as u64),
                8..12 => word(at + 8, count as u64),
                _ => word(at + 12, count as u64),
            };
            let (slots, ring) = (self.slots, air.ring);
            let pair = |at: usize, base: usize, stride: usize| {
                let index = (at - base) / stride;
                (
                    index / REGION_SLOTS,
                    index % REGION_SLOTS,
                    base + index * stride,
                )
            };
            Some(match address {
                a if (STATE + gs::STRATEGIC_AIR_MANAGER..STATE + gs::STRATEGIC_AIR_MANAGER + 8)
                    .contains(&a) =>
                {
                    word(STATE + gs::STRATEGIC_AIR_MANAGER, AIR_MANAGER as u64)
                }
                a if (AIR_MANAGER..AIR_MANAGER + 8).contains(&a) => word(
                    AIR_MANAGER,
                    (BASE + engine_native::cstrategic_air_manager::VTABLE_RVA.0) as u64,
                ),
                a if (AIR_MANAGER + 0x30..AIR_MANAGER + 0x40).contains(&a) => {
                    match a - AIR_MANAGER - 0x30 {
                        0..8 => word(AIR_MANAGER + 0x30, AIR_ROOTS as u64),
                        8..12 => word(AIR_MANAGER + 0x38, slots as u64),
                        _ => word(AIR_MANAGER + 0x3c, slots as u64),
                    }
                }
                a if (AIR_ROOTS + 8..AIR_ROOTS + slots * 8).contains(&a) => {
                    let slot = (a - AIR_ROOTS) / 8;
                    word(
                        AIR_ROOTS + slot * 8,
                        (AIR_OBJECTS + slot * AIR_OBJECT) as u64,
                    )
                }
                a if (AIR_OBJECTS + AIR_OBJECT..AIR_OBJECTS + slots * AIR_OBJECT).contains(&a) => {
                    let slot = (a - AIR_OBJECTS) / AIR_OBJECT;
                    let object = AIR_OBJECTS + slot * AIR_OBJECT;
                    let table = engine_native::cstrategic_air::HISTORY_TABLE;
                    match a - object {
                        0..8 => word(
                            object,
                            (BASE + engine_native::cstrategic_air::VTABLE_RVA.0) as u64,
                        ),
                        o if (0x90..0x94).contains(&o) => word(object + 0x90, slot as u64),
                        o if (table..table + 8).contains(&o) => word(
                            object + table,
                            (AIR_TABLES + slot * REGION_SLOTS * 8) as u64,
                        ),
                        o if (table + 8..table + 16).contains(&o) => {
                            let at = object + table + 8;
                            match a - at {
                                0..4 => word(at, REGION_SLOTS as u64),
                                _ => word(at + 4, REGION_SLOTS as u64),
                            }
                        }
                        _ => 0,
                    }
                }
                a if (AIR_TABLES..AIR_TABLES + slots * REGION_SLOTS * 8).contains(&a) => {
                    let (slot, region, at) = pair(a, AIR_TABLES, 8);
                    let history = HISTORIES + (slot * REGION_SLOTS + region) * HISTORY;
                    word(
                        at,
                        if air.has(slot, region) {
                            history as u64
                        } else {
                            0
                        },
                    )
                }
                a if (HISTORIES..HISTORIES + slots * REGION_SLOTS * HISTORY).contains(&a) => {
                    let (slot, region, at) = pair(a, HISTORIES, HISTORY);
                    let window = WINDOWS + (slot * REGION_SLOTS + region) * 3 * HISTORY;
                    match a - at {
                        0..8 => word(
                            at,
                            (BASE + engine_air_history::cloop_history::VTABLE_RVA.0) as u64,
                        ),
                        0x10..0x28 => {
                            let level = (a - at - 0x10) / 8;
                            word(at + 0x10 + level * 8, (window + level * HISTORY) as u64)
                        }
                        0x28..0x30 => word(at + 0x28, at as u64),
                        0x34..0x38 => word(
                            at + 0x34,
                            u64::from(engine_air_history::cloop_history::air_stats::FIELD_COUNT),
                        ),
                        _ => 0,
                    }
                }
                a if (WINDOWS..WINDOWS + slots * REGION_SLOTS * 3 * HISTORY).contains(&a) => {
                    let index = (a - WINDOWS) / HISTORY;
                    let at = WINDOWS + index * HISTORY;
                    let history = HISTORIES + index / 3 * HISTORY;
                    let level = index % 3;
                    let records = RECORD_SLOTS + index * ring * 8;
                    match a - at {
                        0..8 => word(
                            at,
                            (BASE + engine_air_history::loop_history_window::VTABLE_RVA.0) as u64,
                        ),
                        8..24 => header(at + 8, records, ring),
                        0x20..0x24 => word(at + 0x20, ring as u64),
                        0x24..0x28 => word(at + 0x24, if level == 0 { air.current } else { 0 }),
                        0x28 => u8::from(level == 0),
                        0x38..0x40 => word(at + 0x38, history as u64),
                        _ => 0,
                    }
                }
                a if (RECORD_SLOTS..RECORD_SLOTS + slots * REGION_SLOTS * 3 * ring * 8)
                    .contains(&a) =>
                {
                    let entry = (a - RECORD_SLOTS) / 8;
                    word(RECORD_SLOTS + entry * 8, (RECORDS + entry * RECORD) as u64)
                }
                a if (RECORDS..RECORDS + slots * REGION_SLOTS * 3 * ring * RECORD).contains(&a) => {
                    let entry = (a - RECORDS) / RECORD;
                    let at = RECORDS + entry * RECORD;
                    let level = (entry / ring) % 3;
                    let history = HISTORIES + entry / (ring * 3) * HISTORY;
                    match a - at {
                        0..16 => header(
                            at,
                            VALUES + (level * ring + entry % ring) * VALUE_FIELDS,
                            89,
                        ),
                        0x18..0x20 => word(at + 0x18, history as u64),
                        _ => 0,
                    }
                }
                a if (VALUES..VALUES + 3 * ring * VALUE_FIELDS).contains(&a) => {
                    let entry = (a - VALUES) / VALUE_FIELDS;
                    let at = VALUES + entry * VALUE_FIELDS;
                    let channel = (a - at) / 8;
                    let level = entry / ring;
                    let record = entry % ring;
                    let value = if [3, 43].contains(&channel) {
                        if level == 0 && record.is_multiple_of(3) {
                            100_000 * (record as u64 + 1)
                        } else if level != 0 && record == 0 {
                            (0..ring)
                                .filter(|record| record.is_multiple_of(3))
                                .map(|record| 100_000 * (record as u64 + 1))
                                .sum()
                        } else {
                            0
                        }
                    } else {
                        0
                    };
                    word(at + channel * 8, value)
                }
                _ => return None,
            })
        }

        fn in_records(&self, address: usize) -> bool {
            (INTEL_RECORDS..INTEL_RECORDS + self.slots * country_intel_row::STRIDE)
                .contains(&address)
        }
    }

    impl Memory for World {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self.in_records(address) && self.in_records(address + output.len() - 1) {
                output.fill(u8::from(self.intel));
                return true;
            }
            if self.bloc.is_some()
                && (BLOC_LINKS..BLOC_LINKS + self.slots * 4).contains(&address)
                && address + output.len() <= BLOC_LINKS + self.slots * 4
                && (address - BLOC_LINKS).is_multiple_of(4)
                && output.len().is_multiple_of(4)
            {
                let first = (address - BLOC_LINKS) / 4;
                for (index, entry) in output.chunks_exact_mut(4).enumerate() {
                    entry.copy_from_slice(&(first as u32 + index as u32 + 1).to_le_bytes());
                }
                return true;
            }
            for (index, byte) in output.iter_mut().enumerate() {
                *byte = self.byte(address + index);
            }
            true
        }
    }

    fn world() -> World {
        World {
            slots: COUNTRIES + 1,
            intel: false,
            bloc: None,
            air: None,
            relocated: None,
        }
    }

    #[test]
    fn country_intel_sizing_uses_native_headers_without_a_registry_ceiling() {
        const RECORD_COUNT: usize = 8;
        struct LongerVectors(World);
        impl Memory for LongerVectors {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                for (index, byte) in output.iter_mut().enumerate() {
                    let at = address + index;
                    *byte = if (INTEL_TABLES..INTEL_TABLES + self.0.slots * 0x20).contains(&at)
                        && (0x18..0x20).contains(&((at - INTEL_TABLES) % 0x20))
                    {
                        (RECORD_COUNT as u32).to_le_bytes()[(at - INTEL_TABLES) % 4]
                    } else if (INTEL_RECORDS
                        ..INTEL_RECORDS + RECORD_COUNT * country_intel_row::STRIDE)
                        .contains(&at)
                    {
                        1
                    } else {
                        self.0.byte(at)
                    };
                }
                true
            }
        }
        let memory = LongerVectors(World {
            slots: 3,
            intel: true,
            ..world()
        });
        let native = observe_country_intel(&memory, BASE).unwrap().unwrap();
        assert_eq!(native, 2 * RECORD_COUNT);
        let mut capacity = collector_intel::intel::Capacities::default();
        capacity.rows[15] = 0;
        capacity.raise_for_country_intel(native);
        let mut rows = vec![country_intel_row::Row::default(); capacity.rows[15]];
        let mut scratch = vec![0; 2 * country_intel_row::STRIDE];
        assert_eq!(
            ccountry_intel::walk(&memory, BASE, STATE, &mut rows, &mut scratch),
            Outcome::Rows { len: native }
        );
        assert_eq!(rows[native - 1].target_index, RECORD_COUNT as u32 - 1);
    }

    struct Notified {
        revision: std::sync::atomic::AtomicU64,
        countries: Vec<std::sync::atomic::AtomicU64>,
        refused: std::sync::atomic::AtomicUsize,
    }

    const REFUSE_ALL: usize = usize::MAX;

    impl collector_air_relations::relation_changes::Changes for Notified {
        fn quiet(&self) -> Option<(u64, u64)> {
            Some((1, self.revision.load(Ordering::Relaxed)))
        }
        fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
            if receiver == DIPLOMACY {
                return Some((1, 0));
            }
            let slot = receiver.checked_sub(OBJECTS)? / OBJECT;
            let refused = self.refused.load(Ordering::Relaxed);
            if refused == REFUSE_ALL || refused == slot + 1 {
                return None;
            }
            let sequence = self.countries.get(slot)?.load(Ordering::Relaxed);
            (receiver != 0).then_some((1, sequence))
        }
    }

    impl Notified {
        fn new(slots: usize) -> Self {
            Self {
                revision: Default::default(),
                countries: (0..slots).map(|_| Default::default()).collect(),
                refused: Default::default(),
            }
        }

        fn touch(&self, slots: impl IntoIterator<Item = usize>) {
            self.revision.fetch_add(1, Ordering::Relaxed);
            for slot in slots {
                self.countries[slot].fetch_add(2, Ordering::Relaxed);
            }
        }
    }

    struct RelationsHour {
        calls: usize,
        wall: std::time::Duration,
        copied: usize,
        held: usize,
        outcome: Outcome,
        decode: std::time::Duration,
    }

    struct Counting<'a>(&'a World, std::sync::atomic::AtomicUsize);

    impl Memory for Counting<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.1.fetch_add(1, Ordering::Relaxed);
            self.0.copy(address, output)
        }
        fn copy_records(
            &self,
            sources: engine_native::memory::RecordSources<'_>,
            layout: &engine_native::memory::RecordLayout<'_>,
            destination: &mut [u8],
        ) -> usize {
            self.1.fetch_add(1, Ordering::Relaxed);
            self.0.copy_records(sources, layout, destination)
        }
        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            destinations: &mut [&mut [u8]; N],
        ) -> usize {
            self.1.fetch_add(1, Ordering::Relaxed);
            for (done, (address, destination)) in addresses
                .into_iter()
                .zip(destinations.iter_mut())
                .enumerate()
            {
                if !destination.is_empty() && !self.0.copy(address, destination) {
                    return done;
                }
            }
            N
        }
    }

    fn relations_hour(
        world: &World,
        changes: &Notified,
        hour: &mut collector_air_relations::air_relations::hour::Hour,
        at: i32,
        threads: bool,
    ) -> RelationsHour {
        let memory = Counting(world, Default::default());
        let started = std::time::Instant::now();
        let wall = hour.run(&memory, BASE, STATE, changes, at, threads);
        let decode = started.elapsed() - wall;
        RelationsHour {
            calls: memory.1.load(Ordering::Relaxed),
            wall,
            copied: hour.staged_countries(),
            held: hour.held.rows(),
            outcome: hour.emitted(),
            decode,
        }
    }

    fn assert_bloc_rows(
        emitted: &[engine_air_history::air_participant_vectors::Row],
        slots: &[u32],
        (enemies, allies): (u32, u32),
    ) {
        use engine_air_history::{
            air_participant_vectors::ALLY, air_participant_vectors::ENEMY,
            air_participant_vectors::Row,
        };
        let expected = slots.iter().flat_map(|&country| {
            [(ALLY, allies), (ENEMY, enemies)]
                .into_iter()
                .flat_map(move |(kind, count)| {
                    (0..count).map(move |ordinal| Row {
                        country,
                        kind,
                        ordinal,
                        other: ordinal + 1,
                    })
                })
        });
        let mut len = 0;
        for (index, row) in expected.enumerate() {
            assert_eq!(emitted.get(index), Some(&row), "row {index}");
            len += 1;
        }
        assert_eq!(emitted.len(), len);
    }

    fn bloc_war_relations(countries: usize) {
        use collector_air_relations::air_relations::{PARTS, hour::Hour};
        let enemies = countries as u32 / 2;
        let mut world = World {
            slots: countries + 1,
            bloc: Some((0, enemies - 1)),
            ..world()
        };
        let changes = Notified::new(countries + 1);
        let rows = collector_primitives::country_slots::rows(countries + 1, 8);
        let mut hour = Hour::new(rows, rows * 4 / PARTS);
        let mut serial = Hour::new(rows, rows * 4 / PARTS);
        let serial_changes = Notified::new(countries + 1);
        let started = std::time::Instant::now();
        std::thread::scope(|scope| {
            for _ in 0..PARTS {
                scope.spawn(|| ());
            }
        });
        let spawn = started.elapsed();
        for (at, (label, touched, copied)) in [
            ("first hour", vec![], countries),
            ("bloc declaration", (1..=countries).collect(), countries),
            ("quiet hour", vec![], 0),
            ("single declaration", vec![1, 2], 2),
            ("held table", (1..=countries).collect(), countries),
            ("held table, quiet again", vec![], countries),
        ]
        .into_iter()
        .enumerate()
        {
            if label == "bloc declaration" {
                world.bloc = Some((enemies, enemies - 1));
            }
            if label == "held table" {
                changes.refused.store(REFUSE_ALL, Ordering::Relaxed);
                serial_changes.refused.store(REFUSE_ALL, Ordering::Relaxed);
            }
            if !touched.is_empty() {
                changes.touch(touched.iter().copied());
                serial_changes.touch(touched);
            }
            let result = relations_hour(&world, &changes, &mut hour, at as i32, true);
            let one = relations_hour(&world, &serial_changes, &mut serial, at as i32, false);
            let emitted = match result.outcome {
                Outcome::Rows { len } => len.to_string(),
                other => format!("{other:?}"),
            };
            eprintln!(
                "relations, {countries} countries, {label}: {} countries copied in {} guarded calls, {:?} wall over {PARTS} parts ({spawn:?} of it spawning), {:?} on one thread; decode {:?}, {emitted} of {} rows emitted",
                result.copied, result.calls, result.wall, one.wall, result.decode, result.held
            );
            let slice: Vec<u32> = (1..=countries as u32)
                .filter(|&slot| {
                    at < 2 || slot % replay_data::cadence::STAGGER_PERIOD as u32 == at as u32
                })
                .collect();
            let bloc = world.bloc.unwrap();
            let per_country = (bloc.0 + bloc.1) as usize;
            for (run, pack) in [(&result, &hour), (&one, &serial)] {
                assert_eq!(run.copied, copied, "{label}");
                assert_eq!(run.held, countries * per_country, "{label}");
                assert_eq!(
                    run.outcome,
                    Outcome::Rows {
                        len: slice.len() * per_country
                    },
                    "{label}"
                );
                assert_bloc_rows(pack.emitted_rows(), &slice, bloc);
            }
            hour.grow();
            serial.grow();
        }
    }

    #[test]
    fn relations_are_copied_only_for_touched_countries_and_emitted_by_slice() {
        use collector_air_relations::air_relations::hour::Hour;
        let mut world = World {
            slots: 49,
            bloc: Some((24, 23)),
            ..world()
        };
        let changes = Notified::new(49);
        let mut hour = Hour::new(4096, 4096);
        let first = relations_hour(&world, &changes, &mut hour, 0, true);
        assert_eq!(first.copied, 48);
        assert_eq!(first.outcome, Outcome::Rows { len: 48 * 47 });
        let quiet = relations_hour(&world, &changes, &mut hour, 1, true);
        assert!(quiet.calls <= 8 * 48, "{}", quiet.calls);
        assert_eq!(quiet.copied, 0);
        assert_eq!(quiet.outcome, Outcome::Rows { len: 2 * 47 });
        changes.touch([3]);
        let unchanged = relations_hour(&world, &changes, &mut hour, 2, true);
        assert_eq!(unchanged.copied, 1);
        assert_eq!(unchanged.outcome, Outcome::Rows { len: 2 * 47 });
        world.bloc = Some((24, 22));
        changes.touch([5]);
        let changed = relations_hour(&world, &changes, &mut hour, 3, true);
        assert_eq!(changed.copied, 1);
        assert_eq!(changed.outcome, Outcome::Rows { len: 2 * 47 + 46 });
        world.relocated = Some(7);
        changes.touch([3]);
        let replaced = relations_hour(&world, &changes, &mut hour, 4, true);
        assert_eq!(replaced.copied, 2);
        world.bloc = Some((24, 23));
        let mut small = Hour::new(4096, 64);
        let grown = relations_hour(&world, &Notified::new(49), &mut small, 0, false);
        assert_eq!(grown.outcome, Outcome::Rows { len: 48 * 47 });
    }

    #[test]
    fn a_refused_receiver_is_copied_every_hour() {
        use collector_air_relations::air_relations::hour::Hour;
        let world = World {
            slots: 49,
            bloc: Some((24, 23)),
            ..world()
        };
        let changes = Notified::new(49);
        changes.refused.store(3 + 1, Ordering::Relaxed);
        let mut hour = Hour::new(4096, 4096);
        assert_eq!(
            relations_hour(&world, &changes, &mut hour, 0, true).copied,
            48
        );
        for at in 1..3 {
            assert_eq!(
                relations_hour(&world, &changes, &mut hour, at, true).copied,
                1
            );
        }
        changes.touch([5]);
        assert_eq!(
            relations_hour(&world, &changes, &mut hour, 3, true).copied,
            2
        );
    }

    #[test]
    fn a_forced_full_hour_re_emits_an_unpublished_hours_changes_exactly() {
        use collector_air_relations::air_relations::hour::Hour;
        let mut world = World {
            slots: 49,
            bloc: Some((24, 23)),
            ..world()
        };
        let changes = Notified::new(49);
        let mut hour = Hour::new(4096, 4096);
        relations_hour(&world, &changes, &mut hour, 0, true);
        world.bloc = Some((24, 22));
        changes.touch(1..=48);
        let withheld = relations_hour(&world, &changes, &mut hour, 1, true);
        assert_eq!(withheld.outcome, Outcome::Rows { len: 48 * 46 });
        let between = relations_hour(&world, &changes, &mut hour, 2, true);
        assert_eq!(between.outcome, Outcome::Rows { len: 2 * 46 });
        hour.force_full = true;
        let recovered = relations_hour(&world, &changes, &mut hour, 3, true);
        assert_eq!(recovered.copied, 0);
        assert_eq!(recovered.outcome, Outcome::Rows { len: 48 * 46 });
        let mut fresh = Hour::new(4096, 4096);
        relations_hour(&world, &Notified::new(49), &mut fresh, 0, true);
        assert_eq!(hour.emitted_rows(), fresh.emitted_rows());
    }

    #[test]
    fn a_relation_vector_longer_than_the_country_links_fails_the_hour() {
        use collector_air_relations::air_relations::hour::Hour;
        let world = World {
            slots: 49,
            bloc: Some((0, 50)),
            ..world()
        };
        let mut hour = Hour::new(4096, 4096);
        let result = relations_hour(&world, &Notified::new(49), &mut hour, 0, false);
        assert_eq!(
            result.outcome,
            Outcome::Unavailable {
                reason: "air participant vector longer than the country links"
            }
        );
    }

    #[test]
    fn bloc_war_relations_at_200_countries() {
        bloc_war_relations(200);
    }

    fn touch(
        watches: &collector_air_histories::history_changes::Watches,
        site: usize,
        receivers: impl IntoIterator<Item = usize>,
        calls: u64,
    ) {
        for receiver in receivers {
            for _ in 0..calls {
                drop(watches.notify(receiver, site));
            }
        }
    }

    struct Blinded<'a> {
        watches: &'a collector_air_histories::history_changes::Watches,
        blind: std::collections::BTreeSet<usize>,
    }

    impl collector_air_histories::history_changes::Changes for Blinded<'_> {
        fn quiet(&self) -> Option<(u64, u64)> {
            self.watches.quiet()
        }
        fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
            (!self.blind.contains(&receiver))
                .then(|| self.watches.sequence(receiver))
                .flatten()
        }
        fn site(&self, index: usize) -> Option<(u64, u64)> {
            self.watches.site(index)
        }
    }

    struct HistoriesHour {
        calls: usize,
        wall: std::time::Duration,
        held: usize,
        outcome: Outcome,
        decode: std::time::Duration,
    }

    #[allow(clippy::too_many_arguments)]
    fn histories_hour(
        world: &World,
        regions: &[u64],
        changes: &(impl collector_air_histories::history_changes::Changes + Sync),
        hour: &mut collector_air_histories::air_histories::hour::Hour,
        frame: &mut collector_air_histories::air_histories::Captured,
        at: i32,
        threads: bool,
    ) -> HistoriesHour {
        let mut cache = engine_allocation::shared(
            Cache::try_with_capacity(Cache::cells_for_countries(world.slots, MIN_COUNTRY_CELLS))
                .unwrap(),
        )
        .unwrap();
        let snapshot = Cache::prepare_shared(&mut cache, world, BASE, STATE).unwrap();
        let counting = Counting(world, Default::default());
        let memory = snapshot.roster_view(&counting);
        let (wall, decode, outcome) =
            hour.run(frame, &memory, BASE, STATE, regions, changes, at, threads);
        HistoriesHour {
            calls: counting.1.load(Ordering::Relaxed),
            wall,
            held: hour.held_rows(),
            outcome,
            decode,
        }
    }

    fn history(slot: usize, region: u32) -> usize {
        HISTORIES + (slot * REGION_SLOTS + region as usize) * HISTORY
    }

    const HELD: usize = 12;

    fn naval_patrol_histories(countries: usize, every: usize, patrolled: u32) -> Vec<usize> {
        use collector_air_histories::air_histories::{
            PARTS,
            hour::{self, Hour},
        };
        use collector_air_histories::history_changes::Watches;
        let mut regions: Vec<u64> = (0..patrolled)
            .map(|region| u64::from(3 + region * 7))
            .collect();
        let mut world = World {
            slots: countries + 1,
            air: Some(AirWorld {
                every,
                regions: (0..=patrolled).map(|region| 3 + region * 7).collect(),
                ring: 30,
                current: 3,
            }),
            ..world()
        };
        let air = world.air.as_ref().unwrap();
        let held: Vec<usize> = (1..=countries).filter(|&slot| air.holds(slot)).collect();
        let (watches, serial_watches) = (
            Watches::enabled(countries + 1, air.regions.len()),
            Watches::enabled(countries + 1, air.regions.len()),
        );
        let installed = watches.capacity();
        let mut capacity = hour::capacities(countries + 1);
        let mut serial_capacity = capacity;
        let (mut frame, mut serial_frame) = (hour::frame(&capacity), hour::frame(&capacity));
        let (mut hour, mut serial) = (Hour::new(), Hour::new());
        let started = std::time::Instant::now();
        std::thread::scope(|scope| {
            for _ in 0..PARTS {
                scope.spawn(|| ());
            }
        });
        let spawn = started.elapsed();
        let mut calls = Vec::new();
        for (at, label) in [
            "first hour",
            "quiet hour",
            "one country's combat",
            "daily advance",
            "one more region",
        ]
        .into_iter()
        .enumerate()
        {
            match label {
                "one country's combat" => {
                    let air = [AIR_OBJECTS + held[0] * AIR_OBJECT];
                    touch(&watches, 0, air, 3);
                    touch(&serial_watches, 0, air, 3);
                }
                "daily advance" => {
                    let air = world.air.as_mut().unwrap();
                    air.current = 4;
                    let histories: Vec<_> = held
                        .iter()
                        .flat_map(|&slot| {
                            air.regions.iter().map(move |&region| history(slot, region))
                        })
                        .collect();
                    touch(&watches, 1, histories.iter().copied(), 1);
                    touch(&serial_watches, 1, histories, 1);
                }
                "one more region" => regions.push(u64::from(3 + patrolled * 7)),
                _ => {}
            }
            let allocated = hour.allocated(&frame);
            let result = histories_hour(
                &world, &regions, &watches, &mut hour, &mut frame, at as i32, true,
            );
            let one = histories_hour(
                &world,
                &regions,
                &serial_watches,
                &mut serial,
                &mut serial_frame,
                at as i32,
                false,
            );
            assert_eq!(
                result.held,
                held.len() * world.air.as_ref().unwrap().regions.len() * HELD
            );
            assert_eq!(one.held, result.held);
            if (2..=3).contains(&at) {
                assert_eq!(hour.allocated(&frame), allocated, "{label} grew a buffer");
            }
            let counts = Hour::counts(&frame);
            assert_eq!(
                counts.menu_refusals,
                [0; engine_air_history::menu_history::REFUSALS.len()]
            );
            let emitted = match result.outcome {
                Outcome::Rows { len } => len.to_string(),
                other => format!("{other:?}"),
            };
            eprintln!(
                "naval patrol, {countries} countries, {patrolled} regions, 1/{every} with history, {label}: {} guarded calls; parts wall {:?} on {PARTS} threads ({spawn:?} of it spawning); serial {:?}, pool of 3 {:?}, pool of 8 {:?}; decode {:?}; {emitted} of {} rows emitted; {} read, {} advanced, {} refreshed",
                result.calls,
                result.wall,
                one.wall,
                one.wall / 3,
                one.wall / PARTS as u32,
                result.decode,
                result.held,
                counts.read,
                counts.advanced,
                counts.refreshed,
            );
            watches.grow(Hour::grow(&mut frame, &mut capacity));
            serial_watches.grow(Hour::grow(&mut serial_frame, &mut serial_capacity));
            calls.push(result.calls);
        }
        for watches in [&watches, &serial_watches] {
            assert_eq!(watches.capacity(), installed);
        }
        let mut fresh = Hour::new();
        let unobserved = collector_air_histories::history_changes::Unobserved;
        histories_hour(
            &world,
            &regions,
            &unobserved,
            &mut fresh,
            &mut hour::frame(&capacity),
            0,
            false,
        );
        assert!(fresh.same(&hour) && fresh.same(&serial));
        calls
    }

    #[test]
    fn naval_patrol_histories_are_read_where_written() {
        use collector_air_histories::air_histories::PARTS;
        let calls = naval_patrol_histories(100, 5, 2);
        assert_eq!(calls[0], 200, "{calls:?}");
        assert_eq!(calls[1], 3 * PARTS);
        assert!(calls[2] < calls[0], "{calls:?}");
        assert_eq!(calls[3], 3 * PARTS);
        assert_eq!(calls[4], 3 * PARTS);
    }

    #[test]
    fn an_unobserved_advance_rereads_only_that_history() {
        use collector_air_histories::air_histories::hour::{self, Hour};
        let world = |current| World {
            slots: 3,
            air: Some(AirWorld {
                every: 1,
                regions: vec![3],
                ring: 30,
                current,
            }),
            ..world()
        };
        let watches = collector_air_histories::history_changes::Watches::enabled(3, 1);
        let mut changes = Blinded {
            watches: &watches,
            blind: Default::default(),
        };
        let (mut hour, mut frame) = (Hour::new(), hour::frame(&hour::capacities(3)));
        histories_hour(&world(3), &[3], &changes, &mut hour, &mut frame, 0, false);
        changes.blind.insert(history(1, 3));
        touch(&watches, 1, [history(1, 3), history(2, 3)], 1);
        let rows = histories_hour(&world(4), &[3], &changes, &mut hour, &mut frame, 1, true).held;
        let counts = Hour::counts(&frame);
        assert_eq!((counts.advanced, counts.refreshed, rows), (1, 1, 2 * HELD));
        let mut fresh = Hour::new();
        let unobserved = collector_air_histories::history_changes::Unobserved;
        histories_hour(
            &world(4),
            &[3],
            &unobserved,
            &mut fresh,
            &mut frame,
            0,
            false,
        );
        assert!(fresh.same(&hour));
    }

    #[test]
    fn a_quiet_hour_rereads_every_unwatched_receiver() {
        use collector_air_histories::air_histories::{
            Counts,
            hour::{self, Hour},
        };
        let world = World {
            slots: 3,
            air: Some(AirWorld {
                every: 1,
                regions: vec![3],
                ring: 30,
                current: 3,
            }),
            ..world()
        };
        let unobserved = collector_air_histories::history_changes::Unobserved;
        let mut fresh = Hour::new();
        histories_hour(
            &world,
            &[3],
            &unobserved,
            &mut fresh,
            &mut hour::frame(&hour::capacities(3)),
            0,
            false,
        );
        for (blind, counts) in [
            (
                None,
                Counts {
                    reused: 2,
                    quiet: true,
                    ..Counts::default()
                },
            ),
            (
                Some(history(1, 3)),
                Counts {
                    reused: 2,
                    refreshed: 1,
                    ..Counts::default()
                },
            ),
            (
                Some(AIR_OBJECTS + AIR_OBJECT),
                Counts {
                    reused: 1,
                    read: 1,
                    ..Counts::default()
                },
            ),
        ] {
            let watches = collector_air_histories::history_changes::Watches::enabled(3, 1);
            let changes = Blinded {
                watches: &watches,
                blind: blind.into_iter().collect(),
            };
            let (mut hour, mut frame) = (Hour::new(), hour::frame(&hour::capacities(3)));
            histories_hour(&world, &[3], &changes, &mut hour, &mut frame, 0, false);
            for at in 1..3 {
                let quiet = histories_hour(&world, &[3], &changes, &mut hour, &mut frame, at, true);
                assert_eq!(Hour::counts(&frame), counts, "{blind:x?} at {at}");
                assert_eq!(quiet.held, 2 * HELD);
                assert!(fresh.same(&hour));
            }
        }
    }

    #[test]
    fn a_failed_part_withdraws_the_hour_and_the_next_copies_afresh() {
        use collector_air_histories::air_histories::{
            PARTS,
            hour::{self, Hour},
        };
        let slots = 2 * PARTS + 1;
        let world = |current| World {
            slots,
            air: Some(AirWorld {
                every: 1,
                regions: vec![3],
                ring: 30,
                current,
            }),
            ..world()
        };
        let histories: Vec<_> = (1..slots).map(|slot| history(slot, 3)).collect();
        let capacity = hour::capacities(slots);
        for part in 0..PARTS {
            let watches = collector_air_histories::history_changes::Watches::enabled(slots, 1);
            let memory = world(4);
            let (mut hour, mut frame) = (Hour::new(), hour::frame(&capacity));
            histories_hour(&world(3), &[3], &watches, &mut hour, &mut frame, 0, true);
            touch(&watches, 1, histories.iter().copied(), 1);
            let (mut failed, mut next) = (hour::frame(&capacity), hour::frame(&capacity));
            hour.supply(&mut failed, &[3], &watches);
            hour.stage(&mut failed, &memory, BASE, STATE, &watches, true);
            hour.fail(&mut failed, part);
            hour.settle(&failed);
            hour.supply(&mut next, &[3], &watches);
            hour.stage(&mut next, &memory, BASE, STATE, &watches, true);
            hour.settle(&next);
            assert_eq!(Hour::copied(&next), (slots - 1) as u64, "part {part}");
            assert_eq!(
                hour.decode(&mut failed, 1),
                Outcome::Unavailable {
                    reason: "naval-patrol history acquisition failed"
                },
                "part {part}"
            );
            assert_eq!(
                hour.decode(&mut next, 2),
                Outcome::Rows {
                    len: (slots - 1) * 16
                }
            );
            assert_eq!(
                next.selection().read_owners().collect::<Vec<_>>(),
                (1..slots as u32).collect::<Vec<_>>(),
                "part {part}"
            );
            let mut fresh = Hour::new();
            let unobserved = collector_air_histories::history_changes::Unobserved;
            histories_hour(&memory, &[3], &unobserved, &mut fresh, &mut next, 0, false);
            assert!(fresh.same(&hour), "part {part}");
        }
    }

    #[test]
    fn a_refused_part_fails_the_hour_and_the_next_copies_afresh() {
        use collector_air_histories::air_histories::{
            PARTS,
            hour::{self, Hour},
        };
        let slots = 2 * PARTS + 1;
        let world = World {
            slots,
            air: Some(AirWorld {
                every: 1,
                regions: vec![3],
                ring: 30,
                current: 3,
            }),
            ..world()
        };
        let capacity = hour::capacities(slots);
        let watches = collector_air_histories::history_changes::Watches::enabled(slots, 1);
        let (mut hour, mut frame) = (Hour::new(), hour::frame(&capacity));
        histories_hour(&world, &[3], &watches, &mut hour, &mut frame, 0, true);
        let mut refused = hour::frame(&capacity);
        hour.supply(&mut refused, &[3], &watches);
        hour.stage_refusing(&mut refused, &world, BASE, STATE, &watches, 1);
        hour.settle(&refused);
        assert_eq!(
            hour.decode(&mut refused, 1),
            Outcome::Unavailable {
                reason: "naval-patrol history worker admission failed"
            }
        );
        let next = histories_hour(&world, &[3], &watches, &mut hour, &mut frame, 2, true);
        assert_eq!(Hour::copied(&frame), (slots - 1) as u64);
        assert_eq!(
            next.outcome,
            Outcome::Rows {
                len: (slots - 1) * 16
            }
        );
        assert_eq!(
            frame.selection().read_owners().collect::<Vec<_>>(),
            (1..slots as u32).collect::<Vec<_>>()
        );
    }

    #[test]
    fn growing_a_menu_sized_frame_reaches_the_registry_sizing() {
        let menu = Sizing::for_slots(0);
        let mut loaded = menu;
        loaded.raise_for_slots(COUNTRIES + 1);
        let mut frame =
            // SAFETY: The session or isolated fixture owns this frame's observer allocations and
            // excludes teardown.
            unsafe { sampling::Captured::with_capacities(&menu.core, &menu.map, &menu.extended) }.unwrap();
        let (mut core, mut map, mut extended) = (loaded.core, loaded.map, loaded.extended);
        frame.grow(&mut core);
        // SAFETY: This path owns the frame capacity changes and excludes teardown.
        unsafe { frame.map.grow(&mut map) };
        // SAFETY: This path owns the frame capacity changes and excludes teardown.
        unsafe { frame.extended.grow(&mut extended) };
        assert_eq!(core, loaded.core);
        assert!(map == loaded.map);
        let built =
            // SAFETY: The session or isolated fixture owns this frame's observer allocations and
            // excludes teardown.
            unsafe { sampling::Captured::with_capacities(&loaded.core, &loaded.map, &loaded.extended) }
                .unwrap();
        assert!(frame.countries.len() >= built.countries.len());
        assert!(frame.links.len() >= built.links.len());
        for ((name, grown), (_, expected)) in frame
            .extended
            .allocated_by_pack()
            .unwrap()
            .into_iter()
            .zip(built.extended.allocated_by_pack().unwrap())
        {
            assert!(
                grown >= expected,
                "{name}: grew to {grown}, built {expected}"
            );
        }
    }

    #[test]
    fn a_larger_country_cache_is_handed_to_the_hook_without_game_thread_allocation() {
        use std::sync::atomic::Ordering;
        let shared =
        // SAFETY: The caller owns the session or isolated fixture state required by this
        // constructor.
        unsafe { sampling::Shared::new(BASE, 0, 1) };
        let installed = shared.country_cache_cells.load(Ordering::Acquire);
        let cells = Cache::cells_for_countries(COUNTRIES + 1, MIN_COUNTRY_CELLS);
        assert!(cells > installed);
        // SAFETY: This is the sizing pass for the active session's cache.
        unsafe { shared.offer_country_cache(cells) };
        let pool = shared.pool_bytes.load(Ordering::Relaxed);
        // SAFETY: This is the sizing pass for the active session's cache.
        unsafe { shared.offer_country_cache(cells) };
        assert_eq!(shared.pool_bytes.load(Ordering::Relaxed), pool);
        shared.install_offered_country_cache();
        assert_eq!(shared.country_cache_cells.load(Ordering::Acquire), cells);
        assert!(shared.country_cache_swap.lock().unwrap().retired.is_some());
        // SAFETY: This is the sizing pass for the active session's cache.
        unsafe { shared.offer_country_cache(0) };
        assert!(shared.country_cache_swap.lock().unwrap().retired.is_none());
    }

    #[test]
    fn sizing_only_rises() {
        let small = Sizing::for_slots(0);
        let mut large = Sizing::for_slots(COUNTRIES + 1);
        assert!(large.core[0] > COUNTRIES && large.core[2] > COUNTRIES);
        assert!(large.country_cells >= COUNTRIES * 6 + 2);
        let before = large;
        large.raise_for_slots(10);
        assert!(large == before);
        assert!(small.core[0] <= large.core[0]);
        assert_eq!(small.country_cells, MIN_COUNTRY_CELLS);
    }
}
